//! The SQLite database (`/data/moonrise.db`, WAL mode). Its schema is managed by the numbered
//! migrations in `server/migrations/`, applied in order at startup; `PRAGMA user_version` (and
//! `meta.schema_version`) records the last one applied.
//!
//! One connection behind a mutex: the queries are small, and blocking work runs on the blocking
//! thread pool (`Db::call`).

use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

/// (version, name, SQL). Append only; never edit one that has shipped.
const MIGRATIONS: &[(u32, &str, &str)] = &[
    (1, "001_init", include_str!("../../migrations/001_init.sql")),
    (2, "002_library", include_str!("../../migrations/002_library.sql")),
    (3, "003_prober", include_str!("../../migrations/003_prober.sql")),
    (4, "004_users", include_str!("../../migrations/004_users.sql")),
    (5, "005_library_filters", include_str!("../../migrations/005_library_filters.sql")),
    (6, "006_dedupe_names", include_str!("../../migrations/006_dedupe_names.sql")),
    (7, "007_settings", include_str!("../../migrations/007_settings.sql")),
    (8, "008_fallbacks", include_str!("../../migrations/008_fallbacks.sql")),
    (9, "009_invites", include_str!("../../migrations/009_invites.sql")),
    (10, "010_mixed_libraries", include_str!("../../migrations/010_mixed_libraries.sql")),
    (11, "011_parts", include_str!("../../migrations/011_parts.sql")),
    (12, "012_versions", include_str!("../../migrations/012_versions.sql")),
    (13, "013_avi_keyframes", include_str!("../../migrations/013_avi_keyframes.sql")),
    (14, "014_credits", include_str!("../../migrations/014_credits.sql")),
    (
        15,
        "015_continue_watching",
        include_str!("../../migrations/015_continue_watching.sql"),
    ),
    (16, "016_ratings", include_str!("../../migrations/016_ratings.sql")),
    (17, "017_match_hints", include_str!("../../migrations/017_match_hints.sql")),
    (
        18,
        "018_community_segments",
        include_str!("../../migrations/018_community_segments.sql"),
    ),
    (19, "019_track_memory", include_str!("../../migrations/019_track_memory.sql")),
    (20, "020_search_recent", include_str!("../../migrations/020_search_recent.sql")),
    (21, "021_drop_jf_tables", include_str!("../../migrations/021_drop_jf_tables.sql")),
];

/// The ffprobe in use (`ffprobe -version`'s first line), recorded with each probe.
pub static PROBER: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Asks ffprobe for its version (once, at start).
pub fn init_prober(ffprobe: &str) {
    let v = std::process::Command::new(ffprobe)
        .arg("-version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| {
            s.lines()
                .next()
                .map(|l| l.split(" Copyright").next().unwrap_or(l).trim().to_string())
        })
        .unwrap_or_default();
    let _ = PROBER.set(v);
}

fn prober() -> Option<String> {
    PROBER.get().filter(|v| !v.is_empty()).cloned()
}

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

/// A cached probe of one file.
#[derive(Debug, Clone)]
pub struct Probe {
    pub probe: Value,
    pub keyframes: Option<Vec<f64>>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        let db = Self::setup(conn)?;
        // It holds the settings' secrets (API keys, passwords): readable by its owner only.
        #[cfg(unix)]
        for suffix in ["", "-wal", "-shm"] {
            use std::os::unix::fs::PermissionsExt;
            let p = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
            if p.exists() {
                if let Err(e) = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)) {
                    tracing::warn!("chmod 600 {}: {e}", p.display());
                }
            }
        }
        Ok(db)
    }

    #[cfg(test)]
    pub fn memory() -> Result<Db> {
        Self::setup(Connection::open_in_memory()?)
    }

    fn setup(mut conn: Connection) -> Result<Db> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&mut conn)?;
        Ok(Db {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Runs `f` with the connection on the blocking pool.
    pub async fn call<R: Send + 'static>(&self, f: impl FnOnce(&mut Connection) -> Result<R> + Send + 'static) -> Result<R> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || f(&mut lock(&conn))).await?
    }

    fn sync<R>(&self, f: impl FnOnce(&mut Connection) -> Result<R>) -> Result<R> {
        f(&mut lock(&self.conn))
    }

    pub fn schema_version(&self) -> Result<u32> {
        self.sync(|c| Ok(c.pragma_query_value(None, "user_version", |r| r.get(0))?))
    }

    /// This server's id: a random UUID made on first start and kept.
    pub fn server_id(&self) -> Result<String> {
        self.sync(|c| {
            let tx = c.transaction()?;
            let id: Option<String> = tx
                .query_row("SELECT value FROM meta WHERE key = 'server_id'", [], |r| r.get(0))
                .optional()?;
            let id = match id {
                Some(id) => id,
                None => {
                    let id = uuid_v4();
                    tx.execute("INSERT INTO meta(key, value) VALUES ('server_id', ?1)", [&id])?;
                    id
                }
            };
            tx.commit()?;
            Ok(id)
        })
    }

    /// The cached probe of `path`, if the file still has this size and mtime.
    pub async fn probe_get(&self, path: String, size: u64, mtime: i64) -> Result<Option<Probe>> {
        self.call(move |c| {
            let row = c
                .query_row(
                    "SELECT probe, keyframes FROM files_probe WHERE path = ?1 AND size = ?2 AND mtime = ?3",
                    params![path, size as i64, mtime],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<Vec<u8>>>(1)?)),
                )
                .optional()?;
            let Some((probe, kf)) = row else { return Ok(None) };
            Ok(Some(Probe {
                probe: serde_json::from_str(&probe)?,
                keyframes: kf.map(|b| decode_f64s(&b)),
            }))
        })
        .await
    }

    /// Stores a fresh probe (replacing any older one for the path; its keyframes and end credits
    /// go with it).
    pub async fn probe_put(&self, path: String, size: u64, mtime: i64, mut probe: Value) -> Result<()> {
        let chapters = probe
            .as_object_mut()
            .and_then(|o| o.remove("chapters"))
            .filter(|c| c.as_array().is_some_and(|a| !a.is_empty()));
        self.call(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO files_probe(path, size, mtime, probe, keyframes, chapters, probed_at, prober, credits)
                 VALUES (?1, ?2, ?3, ?4, (SELECT keyframes FROM files_probe WHERE path = ?1 AND size = ?2 AND mtime = ?3), ?5, ?6, ?7,
                         (SELECT credits FROM files_probe WHERE path = ?1 AND size = ?2 AND mtime = ?3))",
                params![
                    path,
                    size as i64,
                    mtime,
                    probe.to_string(),
                    chapters.map(|c| c.to_string()),
                    now(),
                    prober()
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// Whether the stored probe of this file was made by the ffprobe in use.
    pub async fn probe_current(&self, path: String, size: u64, mtime: i64) -> Result<bool> {
        let want = prober();
        self.call(move |c| {
            let got: Option<Option<String>> = c
                .query_row(
                    "SELECT prober FROM files_probe WHERE path = ?1 AND size = ?2 AND mtime = ?3",
                    params![path, size as i64, mtime],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(matches!(got, Some(g) if want.is_none() || g == want))
        })
        .await
    }

    /// Adds the keyframe index to a stored probe (if the file is unchanged).
    pub async fn keyframes_put(&self, path: String, size: u64, mtime: i64, keyframes: Vec<f64>) -> Result<()> {
        self.call(move |c| {
            c.execute(
                "UPDATE files_probe SET keyframes = ?4 WHERE path = ?1 AND size = ?2 AND mtime = ?3",
                params![path, size as i64, mtime, encode_f64s(&keyframes)],
            )?;
            Ok(())
        })
        .await
    }
}

/// The connection, even after a panic while it was held (a panic must not take the database
/// down for everyone; an open transaction is rolled back when it is dropped).
fn lock(conn: &Mutex<Connection>) -> std::sync::MutexGuard<'_, Connection> {
    conn.lock().unwrap_or_else(|poisoned| {
        conn.clear_poison();
        poisoned.into_inner()
    })
}

fn migrate(conn: &mut Connection) -> Result<()> {
    migrate_to(conn, u32::MAX)
}

/// Applies the migrations after the database's version, up to `last`. A database a newer server
/// migrated further is left as it is.
fn migrate_to(conn: &mut Connection, last: u32) -> Result<()> {
    let current: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for &(version, name, sql) in MIGRATIONS.iter().filter(|(v, _, _)| *v > current && *v <= last) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql).with_context(|| format!("migration {name}"))?;
        tx.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', ?1)",
            [version.to_string()],
        )?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
        tracing::info!(migration = name, "database migrated");
    }
    Ok(())
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn encode_f64s(v: &[f64]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn decode_f64s(b: &[u8]) -> Vec<f64> {
    b.as_chunks::<8>().0.iter().map(|c| f64::from_le_bytes(*c)).collect()
}

pub fn uuid_v4() -> String {
    let mut b: [u8; 16] = rand::random();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_once() {
        let dir = std::env::temp_dir().join(format!("moonrise-db-test-{}", std::process::id()));
        let path = dir.join("moonrise.db");
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.last().unwrap().0);
        // Libraries of no type ("mixed"), and the old kinds.
        for kind in ["mixed", "movies", "shows"] {
            db.sync(|c| {
                Ok(c.execute(
                    "INSERT INTO libraries(id, name, kind, roots, sort) VALUES (?1, ?1, ?1, '[]', 0)",
                    [kind],
                )?)
            })
            .unwrap();
        }
        assert!(db
            .sync(|c| Ok(c.execute(
                "INSERT INTO libraries(id, name, kind, roots, sort) VALUES ('x', 'x', 'music', '[]', 0)",
                []
            )?))
            .is_err());
        let id = db.server_id().unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        drop(db);
        // Reopening keeps the schema and the id.
        let db = Db::open(&path).unwrap();
        assert_eq!(db.server_id().unwrap(), id);
        let mode: String = db.sync(|c| Ok(c.pragma_query_value(None, "journal_mode", |r| r.get(0))?)).unwrap();
        assert_eq!(mode, "wal");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What identification read from the old import's provider ids before migration 017: the
    /// first key (serde_json's map is ordered) equal to the name ignoring case, a non-empty string
    /// other than "0"; Tmdb as a number.
    fn old_provider_id(ids: Option<&str>, name: &str) -> Option<String> {
        let v: Value = ids.and_then(|i| serde_json::from_str(i).ok()).unwrap_or(Value::Null);
        v.as_object()
            .and_then(|o| o.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)))
            .and_then(|(_, v)| v.as_str())
            .filter(|s| !s.is_empty() && *s != "0")
            .map(String::from)
    }

    #[test]
    fn match_hints_migration() {
        let mut c = Connection::open_in_memory().unwrap();
        migrate_to(&mut c, 16).unwrap();
        let ids = [
            Some(r#"{"Tmdb":"603","Imdb":"tt0133093","TvRage":"0"}"#),
            Some(r#"{"tmdb":"7","IMDB":"tt1"}"#),
            Some(r#"{"Tmdb":"0","Imdb":""}"#),
            Some(r#"{"Tmdb":603,"Imdb":null}"#),
            Some(r#"{"Tmdb":"abc"}"#),
            Some(r#"{"Tmdb":" 12"}"#),
            Some(r#"{"Tmdb":"+12"}"#),
            Some(r#"{"Tmdb":"-12"}"#),
            Some(r#"{"Tmdb":"99999999999999999999"}"#),
            Some(r#"{"TMDB":"5","Tmdb":"6","imdb":"0","Imdb":"tt2"}"#),
            Some(r#"{"TMDB":"","Tmdb":"6"}"#),
            Some(r#"{"Tvdb":"81189"}"#),
            Some("not json"),
            Some("[1, 2]"),
            Some("{}"),
            None,
        ];
        for (n, i) in ids.iter().enumerate() {
            for kind in ["Movie", "Episode", "Series", "Season"] {
                c.execute(
                    "INSERT INTO jf_map(jf_id, kind, jf_path, name, season_no, episode_no, episode_end, provider_ids, date_created, file_id, imported_at)
                     VALUES (?1, ?2, ?3, 'Name', 1, 2, 3, ?4, 1400000000, ?5, 0)",
                    params![
                        format!("{kind}{n}"),
                        kind,
                        format!("/media/{kind}/{n}"),
                        i,
                        (kind != "Series").then(|| format!("file{n}"))
                    ],
                )
                .unwrap();
            }
        }
        // Without a path it was never found by one, nor linked to a file.
        c.execute(
            "INSERT INTO jf_map(jf_id, kind, provider_ids, imported_at) VALUES ('nopath', 'Movie', '{\"Tmdb\":\"1\"}', 0)",
            [],
        )
        .unwrap();
        // Up to the release that still kept the import's table (021 drops it).
        migrate_to(&mut c, 20).unwrap();
        assert_eq!(c.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0)).unwrap(), 20);
        let count: i64 = c.query_row("SELECT COUNT(*) FROM match_hints", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 3 * ids.len() as i64, "movies, episodes and series; no seasons");
        for (n, i) in ids.iter().enumerate() {
            let tmdb = old_provider_id(*i, "Tmdb").and_then(|t| t.parse::<i64>().ok());
            let imdb = old_provider_id(*i, "Imdb");
            for kind in ["Movie", "Episode", "Series"] {
                type Row = (
                    String,
                    Option<String>,
                    Option<i64>,
                    Option<String>,
                    Option<i64>,
                    Option<i64>,
                    Option<i64>,
                    Option<String>,
                    Option<i64>,
                );
                let row: Row = c
                    .query_row(
                        "SELECT kind, file_id, tmdb_id, imdb_id, season_no, episode_no, episode_end, title, added_at FROM match_hints WHERE path = ?1",
                        [format!("/media/{kind}/{n}")],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?)),
                    )
                    .unwrap();
                let file = kind != "Series";
                let expected: Row = (
                    kind.to_lowercase(),
                    file.then(|| format!("file{n}")),
                    tmdb,
                    imdb.clone(),
                    file.then_some(1),
                    file.then_some(2),
                    file.then_some(3),
                    file.then(|| "Name".to_string()),
                    Some(1_400_000_000),
                );
                assert_eq!(row, expected, "{kind} {i:?}");
            }
        }
        // The import's table stays, as it was, for an older server on this database.
        let left: i64 = c.query_row("SELECT COUNT(*) FROM jf_map", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 4 * ids.len() as i64 + 1);
    }

    /// Every table and index but the old import's, with its SQL, and every row of each table.
    fn schema_and_rows(c: &Connection) -> Vec<String> {
        let mut out = Vec::new();
        let objects: Vec<(String, String, Option<String>)> = c
            .prepare("SELECT type, name, sql FROM sqlite_master WHERE tbl_name NOT IN ('jf_map', 'jf_subtitles') ORDER BY type, name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for (kind, name, sql) in objects {
            out.push(format!("{kind} {name}: {sql:?}"));
            if kind == "table" {
                // The schema version is the one thing a migration changes in `meta`.
                let only = if name == "meta" { " WHERE key <> 'schema_version'" } else { "" };
                let mut q = c.prepare(&format!("SELECT * FROM \"{name}\"{only}")).unwrap();
                let n = q.column_count();
                let mut rows: Vec<String> = q
                    .query_map([], |r| {
                        Ok((0..n).map(|i| format!("{:?}", r.get_ref(i).unwrap())).collect::<Vec<_>>().join("|"))
                    })
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                rows.sort();
                out.extend(rows.into_iter().map(|row| format!("  {name}: {row}")));
            }
        }
        out
    }

    #[test]
    fn drop_jf_tables_migration() {
        let mut c = Connection::open_in_memory().unwrap();
        migrate_to(&mut c, 16).unwrap();
        c.execute_batch(
            "INSERT INTO jf_map(jf_id, kind, jf_path, provider_ids, file_id, imported_at)
                 VALUES ('a', 'Movie', '/media/a.mkv', '{\"Tmdb\":\"603\",\"Imdb\":\"tt0133093\"}', 'fa', 0);
             INSERT INTO jf_subtitles(jf_path, jf_item, media_path, imported_at) VALUES ('/jf/a.srt', 'a', '/media/a.mkv', 0);",
        )
        .unwrap();
        migrate_to(&mut c, 20).unwrap();
        let hint: i64 = c
            .query_row("SELECT tmdb_id FROM match_hints WHERE path = '/media/a.mkv'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(hint, 603);
        let before = schema_and_rows(&c);
        let jf = "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('jf_map', 'jf_subtitles', 'jf_map_file', 'jf_map_path', 'jf_subtitles_media')";
        assert_eq!(c.query_row(jf, [], |r| r.get::<_, i64>(0)).unwrap(), 5);

        migrate(&mut c).unwrap();
        assert_eq!(
            c.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0)).unwrap(),
            MIGRATIONS.last().unwrap().0,
            "every migration applied"
        );
        // The tables and their indexes are gone; nothing else changed (match_hints included).
        assert_eq!(c.query_row(jf, [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        let left: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE tbl_name IN ('jf_map', 'jf_subtitles') OR name LIKE '%jf%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
        assert_eq!(schema_and_rows(&c), before);
        let meta: String = c
            .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(meta, "21");
        assert!(c
            .query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
            .optional()
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn probe_cache_roundtrip() {
        let db = Db::memory().unwrap();
        let p = "/media/x.mkv".to_string();
        assert!(db.probe_get(p.clone(), 10, 5).await.unwrap().is_none());
        let probe = serde_json::json!({"format": {"duration": "60.0"}, "streams": [], "chapters": [{"id": 1}]});
        db.probe_put(p.clone(), 10, 5, probe).await.unwrap();
        let got = db.probe_get(p.clone(), 10, 5).await.unwrap().unwrap();
        assert_eq!(got.probe["format"]["duration"], "60.0");
        assert!(got.probe.get("chapters").is_none(), "chapters are stored in their own column");
        assert!(got.keyframes.is_none());
        db.keyframes_put(p.clone(), 10, 5, vec![0.0, 2.5, 5.005]).await.unwrap();
        let got = db.probe_get(p.clone(), 10, 5).await.unwrap().unwrap();
        assert_eq!(got.keyframes.unwrap(), vec![0.0, 2.5, 5.005]);
        // A changed file misses, and its keyframes aren't attached to the old probe.
        assert!(db.probe_get(p.clone(), 11, 5).await.unwrap().is_none());
        assert!(db.probe_get(p.clone(), 10, 6).await.unwrap().is_none());
        db.keyframes_put(p.clone(), 10, 6, vec![1.0]).await.unwrap();
        assert_eq!(db.probe_get(p.clone(), 10, 5).await.unwrap().unwrap().keyframes.unwrap().len(), 3);
        // Probing the same file again keeps its keyframe index.
        db.probe_put(p.clone(), 10, 5, serde_json::json!({"format": {}, "streams": []}))
            .await
            .unwrap();
        assert_eq!(db.probe_get(p.clone(), 10, 5).await.unwrap().unwrap().keyframes.unwrap().len(), 3);
        assert!(db.probe_current(p, 10, 5).await.unwrap());
    }
}
