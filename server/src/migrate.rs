//! `moonrise-server migrate-root <from> <to> [--credential <id>] [--apply]`: moves a library root to
//! another place that holds the same files (a local mount → the same folder on an SMB share, or
//! back), keeping everything known about them. Run with the server stopped (it edits the
//! database directly); without `--apply` it only says what it would do.
//!
//! What follows the files: their rows (`files.path`, so ids, titles and watch state stay), the
//! probe and keyframe cache (`files_probe`), the quick scan's directory mtimes (`scan_state`),
//! `item_video.stamp`, the library's root in the `libraries` setting (an SMB root naming the
//! credential `--credential`), and the subtitle folders keyed by the file (`subs/v1/<key>`, the
//! key being a hash of path, size and mtime; `subs/p<hash of path>` of old fetched tracks).
//! Segments in the cache are keyed by path too: they are cut again (the cache is disposable).
//! The reverse move undoes it.

use std::path::Path;

use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::Value;

use crate::{
    admin::settings::{LibrarySetting, Root},
    media::fnv,
};

#[derive(Debug, Default, serde::Serialize)]
pub struct Report {
    pub files: usize,
    pub probes: usize,
    pub scan_dirs: usize,
    pub item_video: usize,
    pub roots: usize,
    pub subs_dirs: usize,
    pub extras_dirs: usize,
    pub applied: bool,
}

fn norm(p: &str) -> String {
    p.trim().trim_end_matches('/').to_string()
}

/// `path` with the root `from` replaced by `to` (None: not under `from`).
pub fn moved(path: &str, from: &str, to: &str) -> Option<String> {
    if path == from {
        return Some(to.to_string());
    }
    path.strip_prefix(from).filter(|r| r.starts_with('/')).map(|r| format!("{to}{r}"))
}

/// The subtitle folder key of a file (as `media/mod.rs` makes it).
fn file_key(path: &str, size: i64, mtime: i64) -> String {
    format!("{:016x}", fnv(&format!("{path}|{size}|{mtime}")))
}

pub fn migrate(db_path: &Path, subs_dir: &Path, from: &str, to: &str, credential: Option<&str>, apply: bool) -> Result<Report> {
    let (from, to) = (norm(from), norm(to));
    if from.is_empty() || to.is_empty() || from == to {
        bail!("give two different roots");
    }
    if moved(&to, &from, &to).is_some() || moved(&from, &to, &from).is_some() {
        bail!("one root is inside the other");
    }
    let mut c = rusqlite::Connection::open(db_path).with_context(|| format!("open {}", db_path.display()))?;
    c.busy_timeout(std::time::Duration::from_secs(5))?;
    let tx = c.transaction()?;
    let mut r = Report {
        applied: apply,
        ..Default::default()
    };

    // Files (path is unique: the target must be free).
    let files: Vec<(String, String, i64, i64)> = {
        let mut st = tx.prepare("SELECT id, path, size, mtime FROM files")?;
        let rows = st
            .query_map([], |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?, x.get(3)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .filter(|f: &(String, String, i64, i64)| moved(&f.1, &from, &to).is_some())
            .collect()
    };
    for (id, path, _, _) in &files {
        let new = moved(path, &from, &to).unwrap();
        let taken: Option<String> = tx
            .query_row("SELECT id FROM files WHERE path = ?1", [&new], |x| x.get(0))
            .optional()?;
        if let Some(other) = taken {
            bail!("{new} is already a file of the library ({other}); nothing changed");
        }
        tx.execute("UPDATE files SET path = ?2 WHERE id = ?1", params![id, new])?;
    }
    r.files = files.len();

    // Path-keyed tables.
    let rows = |tx: &rusqlite::Transaction, sql: &str| -> Result<Vec<String>> {
        let mut st = tx.prepare(sql)?;
        let v = st.query_map([], |x| x.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    };
    for p in rows(&tx, "SELECT path FROM files_probe")? {
        if let Some(new) = moved(&p, &from, &to) {
            tx.execute("DELETE FROM files_probe WHERE path = ?1", [&new])?;
            r.probes += tx.execute("UPDATE files_probe SET path = ?2 WHERE path = ?1", params![p, new])?;
        }
    }
    for p in rows(&tx, "SELECT path FROM scan_state")? {
        if let Some(new) = moved(&p, &from, &to) {
            tx.execute("DELETE FROM scan_state WHERE path = ?1", [&new])?;
            r.scan_dirs += tx.execute("UPDATE scan_state SET path = ?2 WHERE path = ?1", params![p, new])?;
        }
    }
    for s in rows(&tx, "SELECT stamp FROM item_video")? {
        let (path, rest) = s.split_once('|').unwrap_or((&s, ""));
        if let Some(new) = moved(path, &from, &to) {
            r.item_video += tx.execute(
                "UPDATE item_video SET stamp = ?2 WHERE stamp = ?1",
                params![s, format!("{new}|{rest}")],
            )?;
        }
    }

    // The library root in the settings.
    let libs: Option<String> = tx
        .query_row("SELECT value FROM settings WHERE key = 'libraries'", [], |x| x.get(0))
        .optional()?;
    if let Some(libs) = libs {
        let mut list: Vec<LibrarySetting> = serde_json::from_str(&libs).context("libraries setting")?;
        for l in &mut list {
            for root in &mut l.roots {
                if root.runtime_path().is_some_and(|p| p == from) {
                    let mut new = Root::from_runtime(&to);
                    if new.source == "smb" {
                        new.credential_id = credential.map(String::from);
                    }
                    *root = new;
                    r.roots += 1;
                }
            }
        }
        if let Some(id) = credential {
            let creds: Option<String> = tx
                .query_row("SELECT value FROM settings WHERE key = 'smb.credentials'", [], |x| x.get(0))
                .optional()?;
            let known = creds
                .and_then(|c| serde_json::from_str::<Value>(&c).ok())
                .is_some_and(|v| v.get(id).is_some());
            if !known {
                bail!("no SMB credential {id} in the settings");
            }
        }
        tx.execute(
            "UPDATE settings SET value = ?1 WHERE key = 'libraries'",
            [serde_json::to_string(&list)?],
        )?;
    }

    // Subtitle folders (renamed only when applying, after the database commits).
    let mut renames: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new();
    for (_, path, size, mtime) in &files {
        let new = moved(path, &from, &to).unwrap();
        let (a, b) = (
            subs_dir.join("v1").join(file_key(path, *size, *mtime)),
            subs_dir.join("v1").join(file_key(&new, *size, *mtime)),
        );
        if a.is_dir() && !b.exists() {
            renames.push((a, b));
            r.subs_dirs += 1;
        }
        let (a, b) = (
            subs_dir.join(format!("p{:016x}", fnv(path))),
            subs_dir.join(format!("p{:016x}", fnv(&new))),
        );
        if a.is_dir() && !b.exists() {
            renames.push((a, b));
            r.extras_dirs += 1;
        }
    }
    if apply {
        tx.commit()?;
        for (a, b) in &renames {
            std::fs::rename(a, b).with_context(|| format!("rename {} → {}", a.display(), b.display()))?;
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_move() {
        assert_eq!(
            moved("/media/movies/a.mkv", "/media/movies", "smb://nas/Media/movies").as_deref(),
            Some("smb://nas/Media/movies/a.mkv")
        );
        assert_eq!(
            moved("/media/movies", "/media/movies", "smb://nas/Media/movies").as_deref(),
            Some("smb://nas/Media/movies")
        );
        assert_eq!(moved("/media/movies2/a.mkv", "/media/movies", "x"), None);
        assert_eq!(
            moved("smb://nas/Media/movies/a.mkv", "smb://nas/Media/movies", "/media/movies").as_deref(),
            Some("/media/movies/a.mkv")
        );
    }

    #[test]
    fn migrates_and_back() {
        let t = std::env::temp_dir().join(format!("moonrise-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        let dbp = t.join("moonrise.db");
        drop(crate::db::Db::open(&dbp).unwrap());
        let mut db = rusqlite::Connection::open(&dbp).unwrap();
        let subs = t.join("subs");
        let from = "/media/movies";
        let to = "smb://nas/Media/movies";
        let file = "/media/movies/A (2000)/a.mkv";
        (|c: &mut rusqlite::Connection| -> rusqlite::Result<()> {
            c.execute("INSERT INTO libraries(id, name, kind, roots, sort) VALUES ('movies', 'Movies', 'movies', '[]', 0)", [])?;
            c.execute(
                "INSERT INTO files(id, library_id, path, size, mtime, role, first_seen, last_seen, stable_since) VALUES ('f1', 'movies', ?1, 10, 5, 'main', 0, 0, 0)",
                [file],
            )?;
            c.execute("INSERT INTO files_probe(path, size, mtime, probe, probed_at) VALUES (?1, 10, 5, '{}', 0)", [file])?;
            c.execute("INSERT INTO scan_state(path, mtime, scanned_at) VALUES ('/media/movies/A (2000)', 1, 0)", [])?;
            c.execute("INSERT INTO settings(key, value, updated_at) VALUES ('libraries', ?1, 0)", [r#"[{"id":"movies","name":"Movies","kind":"movies","roots":[{"source":"local","path":"/media/movies"}]}]"#])?;
            c.execute("INSERT INTO settings(key, value, updated_at) VALUES ('smb.credentials', '{\"c1\":{\"username\":\"u\",\"password\":\"p\"}}', 0)", [])?;
            Ok(())
        })(&mut db)
        .unwrap();
        drop(db);
        let key_dir = subs.join("v1").join(file_key(file, 10, 5));
        std::fs::create_dir_all(&key_dir).unwrap();
        std::fs::write(key_dir.join("s2.srt"), "1\n").unwrap();

        // A dry run changes nothing.
        let r = migrate(&dbp, &subs, from, to, Some("c1"), false).unwrap();
        assert_eq!((r.files, r.probes, r.scan_dirs, r.roots, r.subs_dirs), (1, 1, 1, 1, 1));
        assert!(key_dir.is_dir());
        assert!(migrate(&dbp, &subs, from, to, Some("nope"), true).is_err(), "unknown credential");
        let r = migrate(&dbp, &subs, from, to, Some("c1"), true).unwrap();
        assert!(r.applied && !key_dir.exists());
        let new_file = "smb://nas/Media/movies/A (2000)/a.mkv";
        assert!(subs.join("v1").join(file_key(new_file, 10, 5)).join("s2.srt").is_file());
        let c = rusqlite::Connection::open(&dbp).unwrap();
        let p: String = c.query_row("SELECT path FROM files WHERE id = 'f1'", [], |x| x.get(0)).unwrap();
        assert_eq!(p, new_file);
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM files_probe WHERE path = ?1", [new_file], |x| x.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let libs: String = c
            .query_row("SELECT value FROM settings WHERE key = 'libraries'", [], |x| x.get(0))
            .unwrap();
        let l: Vec<LibrarySetting> = serde_json::from_str(&libs).unwrap();
        assert_eq!(l[0].roots[0].runtime_path().as_deref(), Some(to));
        assert_eq!(l[0].roots[0].credential_id.as_deref(), Some("c1"));
        drop(c);
        // And back.
        let r = migrate(&dbp, &subs, to, from, None, true).unwrap();
        assert_eq!((r.files, r.roots, r.subs_dirs), (1, 1, 1));
        assert!(key_dir.join("s2.srt").is_file());
        let _ = std::fs::remove_dir_all(&t);
    }
}
