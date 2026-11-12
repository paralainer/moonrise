//! Golden tests for the library's scan (`Library::scan`), episode identification
//! (`identify_episode`, through `identify_pending`) and season alignment (`align_parts`): a
//! folder tree changed step by step, and episode files named in every way the code tells apart,
//! without and with TMDB (its answers cached in the database: tests are offline). The files and
//! titles they leave in the database are written out in full, ids and times made stable.

use std::fmt::Write as _;
use std::{collections::HashMap, path::Path, sync::Arc};

use serde_json::{json, Value};

use super::Library;
use crate::{admin::config::Config, db::Db, storage::cache::Cache};

/// A fixed, old mtime (files are stable at once).
const MTIME: u64 = 1_500_000_000;

fn write(path: &Path, bytes: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..bytes).map(|i| (i % 251) as u8).collect();
    std::fs::write(path, data).unwrap();
    set_mtime(path, MTIME);
}

fn set_mtime(path: &Path, secs: u64) {
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
    let f = if path.is_dir() {
        std::fs::File::open(path)
    } else {
        std::fs::File::options().write(true).open(path)
    };
    f.unwrap().set_modified(t).unwrap();
}

fn library(root: &Path, tmdb: bool) -> (Library, Db) {
    let r = root.to_string_lossy().into_owned();
    let config = Config::from_lookup(|k| match k {
        "MOONRISE_LIBRARIES" => Some(format!("Films=movies:{r}/films;Shows=shows:{r}/shows")),
        "MOONRISE_DATA_DIR" => Some(format!("{r}/data")),
        "MOONRISE_CACHE_DIR" => Some(format!("{r}/cache")),
        "MOONRISE_FFMPEG" => Some("/nonexistent/ffmpeg".into()),
        "MOONRISE_FFPROBE" => Some("/nonexistent/ffprobe".into()),
        "MOONRISE_DISABLE" => Some("llm".into()),
        "TMDB_API_KEY" => tmdb.then(|| "not-a-real-key".into()),
        _ => None,
    });
    let db = Db::memory().unwrap();
    let cache = Arc::new(Cache::new(root.join("cache"), 1 << 30));
    (Library::new(config, db.clone(), cache), db)
}

/// Every row of `table` (ordered by `order`), with ids replaced by stable labels, times left out
/// (or shown as set or not), and the test's folder as `<root>`.
async fn dump(db: &Db, table: &str, order: &str, labels: &HashMap<String, String>, root: &str) -> String {
    let sql = format!("SELECT * FROM {table} ORDER BY {order}");
    let rows: Vec<Vec<(String, String)>> = db
        .call(move |c| {
            let mut st = c.prepare(&sql)?;
            let names: Vec<String> = st.column_names().iter().map(ToString::to_string).collect();
            let rows = st
                .query_map([], |r| {
                    let mut out = Vec::new();
                    for (i, n) in names.iter().enumerate() {
                        let v: rusqlite::types::Value = r.get(i)?;
                        let s = match v {
                            rusqlite::types::Value::Null => "null".to_string(),
                            rusqlite::types::Value::Integer(x) => x.to_string(),
                            rusqlite::types::Value::Real(x) => format!("{x:?}"),
                            rusqlite::types::Value::Text(t) => format!("{t:?}"),
                            rusqlite::types::Value::Blob(b) => format!("<{} bytes>", b.len()),
                        };
                        out.push((n.clone(), s));
                    }
                    Ok(out)
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap();
    let mut out = String::new();
    for row in rows {
        let mut fields = Vec::new();
        for (name, value) in row {
            let shown = match name.as_str() {
                "updated_at" | "refreshed_at" | "identified_at" | "first_seen" | "last_seen" | "stable_since" | "missing_since"
                | "local_at" | "collection_at" | "scanned_at" => {
                    if value == "null" {
                        "null".to_string()
                    } else {
                        "set".to_string()
                    }
                }
                _ => {
                    let mut v = value.replace(root, "<root>");
                    for (id, label) in labels {
                        v = v.replace(id.as_str(), label);
                    }
                    v
                }
            };
            fields.push(format!("{name}={shown}"));
        }
        out.push_str(&fields.join(" "));
        out.push('\n');
    }
    out
}

/// Labels for the ids of files (by path) and items (by key), stable across runs: ids already
/// labelled keep their label (a renamed file keeps its id).
async fn label(db: &Db, labels: &mut HashMap<String, String>) {
    let ids: Vec<(String, String)> = db
        .call(|c| {
            let mut out = Vec::new();
            for (sql, prefix) in [
                ("SELECT id FROM files ORDER BY path", "F"),
                ("SELECT id FROM items ORDER BY key", "I"),
            ] {
                let mut st = c.prepare(sql)?;
                for id in st.query_map([], |r| r.get::<_, String>(0))? {
                    out.push((id?, prefix.to_string()));
                }
            }
            Ok(out)
        })
        .await
        .unwrap();
    for (id, prefix) in ids {
        if !labels.contains_key(&id) {
            let n = labels.values().filter(|l| l.starts_with(&prefix)).count() + 1;
            labels.insert(id, format!("{prefix}{n}"));
        }
    }
}

fn report(r: &super::scanner::ScanReport, root: &str) -> String {
    let mut v = serde_json::to_value(r).unwrap();
    v["ms"] = json!(0);
    v.to_string().replace(root, "<root>")
}

async fn step(
    lib: &Library,
    db: &Db,
    name: &str,
    r: &super::scanner::ScanReport,
    root: &str,
    labels: &mut HashMap<String, String>,
) -> String {
    label(db, labels).await;
    let mut out = format!("# {name}\n{}\n", report(r, root));
    out.push_str(&dump(db, "files", "path", labels, root).await);
    let mut states: Vec<(String, bool)> = lib
        .root_states()
        .into_iter()
        .map(|(r, s)| (r.replace(root, "<root>"), s.online))
        .collect();
    states.sort();
    let _ = writeln!(out, "roots {states:?}");
    out
}

#[tokio::test]
async fn scan_golden() {
    let dir = std::env::temp_dir().join(format!("moonrise-scan-golden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let root = dir.to_string_lossy().into_owned();
    let (films, shows) = (dir.join("films"), dir.join("shows"));
    write(&films.join("A Film (2001)/A Film (2001).mkv"), 200_000);
    write(&films.join("A Film (2001)/Extras/Making of.mkv"), 10);
    write(&films.join("B.Film.2002.1080p.BluRay.x264.mkv"), 20);
    write(&films.join("sample-b.film.2002.mkv"), 5);
    write(&films.join("C Film/VIDEO_TS/VTS_01_1.VOB"), 30);
    write(&films.join("C Film/VIDEO_TS/VTS_01_2.VOB"), 31);
    write(&films.join("notes.txt"), 3);
    // A DVD folder's mtime is the file's.
    set_mtime(&films.join("C Film"), MTIME);
    write(&shows.join("Show (2000)/Season 1/Show S01E01.mkv"), 40);
    write(&shows.join("Show (2000)/Season 1/Show S01E01.en.srt"), 4);
    write(&shows.join("Show (2000)/Season 1/Show S01E02.mkv"), 41);
    write(&shows.join("Show (2000)/Season 2/Episode 5/Show S02E05.avi"), 42);
    write(&shows.join("Other/S02E05.mp4"), 43);
    let (lib, db) = library(&dir, false);
    lib.init().await.unwrap();
    let mut labels = HashMap::new();
    let mut out = String::new();

    let r = lib.scan(true).await.unwrap();
    out.push_str(&step(&lib, &db, "first, full", &r, &root, &mut labels).await);
    let r = lib.scan(false).await.unwrap();
    out.push_str(&step(&lib, &db, "quick, nothing changed", &r, &root, &mut labels).await);

    // Changed, gone, renamed (same size, mtime and content), new; a new folder.
    write(&shows.join("Show (2000)/Season 1/Show S01E02.mkv"), 99);
    std::fs::remove_file(films.join("B.Film.2002.1080p.BluRay.x264.mkv")).unwrap();
    std::fs::rename(
        films.join("A Film (2001)/A Film (2001).mkv"),
        films.join("A Film (2001)/A Film (2001) 1080p.mkv"),
    )
    .unwrap();
    write(&shows.join("Show (2000)/Season 1/Show S01E03.mkv"), 44);
    write(&shows.join("New Show/Season 1/New Show S01E01.mkv"), 45);
    let r = lib.scan(false).await.unwrap();
    out.push_str(&step(&lib, &db, "quick, after changes", &r, &root, &mut labels).await);
    let r = lib.scan(true).await.unwrap();
    out.push_str(&step(&lib, &db, "full, after changes", &r, &root, &mut labels).await);

    // Renames alone: a small file (no hash: the only candidate), a large one (by its hash).
    std::fs::rename(
        shows.join("Show (2000)/Season 1/Show S01E01.mkv"),
        shows.join("Show (2000)/Season 1/Show - S01E01.mkv"),
    )
    .unwrap();
    std::fs::rename(
        films.join("A Film (2001)/A Film (2001) 1080p.mkv"),
        films.join("A Film (2001)/A Film (2001) Remux.mkv"),
    )
    .unwrap();
    let r = lib.scan(false).await.unwrap();
    out.push_str(&step(&lib, &db, "quick, renames", &r, &root, &mut labels).await);

    // A whole folder gone (deeper than the known ones), and a file inside a known folder.
    std::fs::remove_dir_all(shows.join("Show (2000)/Season 2")).unwrap();
    std::fs::remove_file(shows.join("Show (2000)/Season 1/Show S01E03.mkv")).unwrap();
    let r = lib.scan(false).await.unwrap();
    out.push_str(&step(&lib, &db, "quick, a season folder and a file gone", &r, &root, &mut labels).await);

    // A root offline (missing), then back with a file returned.
    std::fs::rename(&shows, dir.join("shows-away")).unwrap();
    let r = lib.scan(false).await.unwrap();
    out.push_str(&step(&lib, &db, "quick, shows offline", &r, &root, &mut labels).await);
    let r = lib.scan(true).await.unwrap();
    out.push_str(&step(&lib, &db, "full, shows still offline", &r, &root, &mut labels).await);
    std::fs::rename(dir.join("shows-away"), &shows).unwrap();
    write(&shows.join("Show (2000)/Season 1/Show S01E03.mkv"), 44);
    let r = lib.scan(false).await.unwrap();
    out.push_str(&step(&lib, &db, "quick, shows back", &r, &root, &mut labels).await);

    // Every root offline: the pass is aborted.
    std::fs::rename(&shows, dir.join("shows-away")).unwrap();
    std::fs::rename(&films, dir.join("films-away")).unwrap();
    let r = lib.scan(false).await.unwrap();
    out.push_str(&step(&lib, &db, "quick, everything offline", &r, &root, &mut labels).await);
    std::fs::rename(dir.join("shows-away"), &shows).unwrap();
    std::fs::rename(dir.join("films-away"), &films).unwrap();
    let r = lib.scan(false).await.unwrap();
    out.push_str(&step(&lib, &db, "quick, everything back", &r, &root, &mut labels).await);

    let _ = std::fs::remove_dir_all(&dir);
    crate::golden::check("library/scan", &out);
}

/// The episode files of the identification tests (shows library), with a manual key for some.
const EPISODES: &[(&str, Option<&str>)] = &[
    ("Show (2000)/Season 1/Show.S01E01.Pilot.720p.mkv", None),
    ("Show (2000)/Season 1/Show.S01E02E03.mkv", None),
    ("Show (2000)/Season 1/Show S01E04 Finale Pt.1.mkv", None),
    ("Show (2000)/Season 1/Show S01E04 Finale Pt.2.mkv", None),
    ("Show (2000)/Season 1/Show S01E05 (2).mkv", None),
    ("Show (2000)/Season 1/Show S01E05 (1).mkv", None),
    ("Show (2000)/Season 1/Show S01E09.mkv", None),
    ("Show (2000)/Season 1/Show S01E10.mkv", Some("split")),
    ("Show (2000)/Season 2/Show - 2x05 - Something.mkv", None),
    ("Show (2000)/Season 2/Show - 2x06.mkv", Some("tmdb:tv:100:s2e7")),
    ("Show (2000)/Season 2/Show - 2x08.mkv", Some("tmdb:tv:100:s2e7:p2")),
    ("Show (2000)/Season 2/Show - 2x09.mkv", Some("tmdb:tv:100:s1e7:p1")),
    ("Show (2000)/Specials/Show S00E01.mkv", None),
    // The names number a two-part finale's halves 2 and 3; TMDB lists it as one episode, 2.
    ("Show (2000)/Season 3/Show S03E01 Opening.mkv", None),
    ("Show (2000)/Season 3/Show S03E02 The End (1).mkv", None),
    ("Show (2000)/Season 3/Show S03E03 The End (2).mkv", None),
    ("Show (2000)/Season 1/Behind the Scenes.mkv", None),
    ("Other Show/Season 3/Other Show 301.mkv", None),
    ("Other Show/Season 3/Other Show S03E02.mkv", Some("local")),
    ("Third (2010)/Third.S01E01.mkv", Some("tmdb:tv:200")),
    ("Third (2010)/Third.S01E02.mkv", Some("local:episode")),
    ("Imported/Imported S01E01.mkv", None),
    ("Imported/Imported S01E02.mkv", None),
];

/// A library with the episode files scanned and their manual keys and old imports in place.
async fn episodes_library(dir: &Path, tmdb: bool) -> (Library, Db) {
    let shows = dir.join("shows");
    write(&dir.join("films/Some Film (1999)/Some Film (1999).mkv"), 10);
    for (i, (rel, _)) in EPISODES.iter().enumerate() {
        write(&shows.join(rel), 100 + i);
    }
    let (lib, db) = library(dir, tmdb);
    lib.init().await.unwrap();
    lib.scan(true).await.unwrap();
    let root = shows.to_string_lossy().into_owned();
    db.call(move |c| {
        for (rel, key) in EPISODES {
            // Durations (the probe's): parts play as long as their file.
            c.execute(
                "UPDATE files SET duration = ?2 WHERE path = ?1",
                rusqlite::params![format!("{root}/{rel}"), 1500.0 + rel.len() as f64],
            )?;
            if let Some(k) = key {
                c.execute(
                    "UPDATE files SET manual_key = ?2 WHERE path = ?1",
                    rusqlite::params![format!("{root}/{rel}"), k],
                )?;
            }
        }
        // Match hints: another number for one file, a name and a date for another.
        c.execute(
            "INSERT INTO match_hints(kind, path, title, season_no, episode_no) VALUES ('episode', ?1, 'Imported Name', 1, 6)",
            [format!("{root}/Imported/Imported S01E01.mkv")],
        )?;
        c.execute(
            "INSERT INTO match_hints(kind, path, title, added_at) VALUES ('episode', ?1, 'Second', 1400000000)",
            [format!("{root}/Imported/Imported S01E02.mkv")],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    (lib, db)
}

/// TMDB's answers for the show: a search, the show, and its seasons.
async fn seed_tmdb(db: &Db) {
    let show = json!({"id": 100, "name": "Show", "original_name": "Show", "first_air_date": "2000-03-01", "popularity": 50.0,
        "overview": "", "number_of_seasons": 3,
        "seasons": [{"season_number": 0, "episode_count": 1}, {"season_number": 1, "episode_count": 8}, {"season_number": 2, "episode_count": 8}, {"season_number": 3, "episode_count": 2}]});
    let season = |n: i64, eps: &[(i64, &str, f64)]| json!({"season_number": n, "episodes": eps.iter().map(|(e, name, rt)| json!({"episode_number": e, "name": name, "runtime": rt})).collect::<Vec<_>>()});
    let s1 = season(
        1,
        &[
            (1, "Pilot", 50.0),
            (2, "Two", 25.0),
            (3, "Three", 25.0),
            (4, "Finale (1)", 25.0),
            (5, "Finale (2)", 25.0),
            (6, "Six", 25.0),
            (7, "Seven", 25.0),
            (8, "Eight", 25.0),
        ],
    );
    let s2 = season(
        2,
        &[(5, "Something", 45.0), (6, "Six", 45.0), (7, "Seven", 45.0), (8, "Eight", 45.0)],
    );
    let s0 = season(0, &[(1, "Special", 30.0)]);
    let s3 = season(3, &[(1, "Opening", 25.0), (2, "The End", 50.0)]);
    let answers: Vec<(String, Value)> = vec![
        (
            "/search/tv?first_air_date_year=2000&include_adult=false&language=en&query=Show".into(),
            json!({"results": [show.clone()]}),
        ),
        ("/search/tv?include_adult=false&language=en&query=Show".into(), json!({"results": [show.clone()]})),
        (
            "/tv/100?append_to_response=aggregate_credits,content_ratings,external_ids,images,alternative_titles,translations&include_image_language=en,null&language=en"
                .into(),
            show.clone(),
        ),
        ("/tv/100/season/0?language=en".into(), s0),
        ("/tv/100/season/1?language=en".into(), s1),
        ("/tv/100/season/2?language=en".into(), s2),
        ("/tv/100/season/3?language=en".into(), s3),
    ];
    let now = crate::library::util::now();
    db.call(move |c| {
        for (url, body) in answers {
            c.execute(
                "INSERT OR REPLACE INTO tmdb_cache(url, status, body, fetched_at) VALUES (?1, 200, ?2, ?3)",
                rusqlite::params![url, body.to_string(), now],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
}

async fn identified(lib: &Library, db: &Db, dir: &Path, name: &str, labels: &mut HashMap<String, String>) -> String {
    let n = lib.identify_pending().await.unwrap();
    label(db, labels).await;
    let root = dir.to_string_lossy().into_owned();
    format!(
        "# {name}: {n} identified\n## items\n{}## files\n{}",
        dump(db, "items", "key", labels, &root).await,
        dump(db, "files", "path", labels, &root).await
    )
}

#[tokio::test]
async fn identify_golden() {
    for tmdb in [false, true] {
        let dir = std::env::temp_dir().join(format!("moonrise-identify-golden-{tmdb}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (lib, db) = episodes_library(&dir, tmdb).await;
        if tmdb {
            seed_tmdb(&db).await;
        }
        let mut labels = HashMap::new();
        let mut out = identified(&lib, &db, &dir, "first", &mut labels).await;
        // Everything again (as after a Fix Match or a rename): the same titles.
        db.call(|c| Ok(c.execute("UPDATE files SET identified_at = NULL", [])?))
            .await
            .unwrap();
        out.push_str(&identified(&lib, &db, &dir, "again", &mut labels).await);
        if tmdb {
            // Aligned with TMDB's lists, then identified again: the alignment stays.
            let root = dir.to_string_lossy().into_owned();
            let shown = |v: Value, labels: &HashMap<String, String>| {
                let mut v = v.to_string().replace(&root, "<root>");
                for (id, l) in labels {
                    v = v.replace(id.as_str(), l);
                }
                v
            };
            let dry = lib.align_parts(None, false).await.unwrap();
            let _ = write!(out, "# align (dry run)\n{}\n", shown(dry, &labels));
            let applied = lib.align_parts(None, true).await.unwrap();
            label(&db, &mut labels).await;
            let _ = write!(out, "# align (applied)\n{}\n", shown(applied, &labels));
            db.call(|c| Ok(c.execute("UPDATE files SET identified_at = NULL", [])?))
                .await
                .unwrap();
            out.push_str(&identified(&lib, &db, &dir, "after alignment", &mut labels).await);
            let only = lib.align_parts(Some(vec!["nothing".into()]), true).await.unwrap();
            let _ = write!(out, "# align (another show only)\n{}\n", shown(only, &labels));
        }
        let _ = std::fs::remove_dir_all(&dir);
        crate::golden::check(&format!("library/identify_{}", if tmdb { "tmdb" } else { "local" }), &out);
    }
}

#[tokio::test]
async fn align_without_tmdb() {
    let dir = std::env::temp_dir().join(format!("moonrise-align-golden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (lib, _db) = episodes_library(&dir, false).await;
    assert_eq!(lib.align_parts(None, true).await.unwrap(), json!({"skipped": "no TMDB"}));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Match hints (`match_hints`, migration 017) decide a movie's and a show's TMDB id and an
/// episode's numbers, title and "added" date; once linked to its file, a hint follows a rename.
#[tokio::test]
async fn match_hints_identify() {
    let dir = std::env::temp_dir().join(format!("moonrise-match-hints-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(&dir.join("films/Not The Name (1980).mkv"), 10);
    write(&dir.join("shows/Some Folder/Season 1/Some Folder S01E01.mkv"), 20);
    let (lib, db) = library(&dir, false);
    lib.init().await.unwrap();
    lib.scan(true).await.unwrap();
    let root = dir.to_string_lossy().into_owned();
    let movie = format!("{root}/films/Not The Name (1980).mkv");
    let episode = format!("{root}/shows/Some Folder/Season 1/Some Folder S01E01.mkv");
    let (m, e, s) = (movie.clone(), episode.clone(), format!("{root}/shows/Some Folder"));
    db.call(move |c| {
        c.execute("INSERT INTO match_hints(kind, path, tmdb_id, added_at) VALUES ('movie', ?1, 603, 1000000000)", [&m])?;
        c.execute(
            "INSERT INTO match_hints(kind, path, imdb_id, season_no, episode_no, title, added_at) VALUES ('episode', ?1, 'tt1', 2, 7, 'Hinted', 1100000000)",
            [&e],
        )?;
        c.execute("INSERT INTO match_hints(kind, path, tmdb_id) VALUES ('series', ?1, 1398)", [&s])?;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(lib.link_match_hints().await.unwrap(), 2);
    let items = |db: Db| async move {
        db.call(|c| {
            let mut st = c.prepare(
                "SELECT i.kind, i.key, i.identify_state, i.title, i.season_no, i.episode_no, i.added_at
                 FROM files f JOIN items i ON i.id = f.item_id ORDER BY i.key",
            )?;
            let rows = st
                .query_map([], |r| {
                    Ok(format!(
                        "{} {} {} {:?} {:?} {:?} {}",
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<i64>>(4)?,
                        r.get::<_, Option<i64>>(5)?,
                        r.get::<_, i64>(6)?
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
        .unwrap()
    };
    lib.identify_pending().await.unwrap();
    let expected = vec![
        "movie tmdb:movie:603 bootstrap \"tmdb:movie:603\" None None 1000000000".to_string(),
        "episode tmdb:tv:1398:s2e7 bootstrap \"Hinted\" Some(2) Some(7) 1100000000".to_string(),
    ];
    let mut got = items(db.clone()).await;
    got.sort_by_key(|r| !r.starts_with("movie"));
    assert_eq!(got, expected);
    // Renamed: the hints follow the files (by id), not their old paths.
    let (m, e) = (movie.clone(), episode.clone());
    db.call(move |c| {
        c.execute(
            "UPDATE files SET path = replace(path, '.mkv', ' (renamed).mkv'), identified_at = NULL WHERE path IN (?1, ?2)",
            [&m, &e],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    lib.identify_pending().await.unwrap();
    let mut again = items(db.clone()).await;
    again.sort_by_key(|r| !r.starts_with("movie"));
    assert_eq!(again, expected);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A quick pass where a folder and its parent both changed: a file renamed in that folder
/// is recognised as a rename (the parent's listing skips the folder, the folder's own listing must
/// still count as complete), so it keeps its id, its title and the watch state kept against both.
#[tokio::test]
async fn quick_rename_with_parent_changed() {
    let dir = std::env::temp_dir().join(format!("moonrise-quick-rename-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (films, shows) = (dir.join("films"), dir.join("shows"));
    // A large file (renames confirmed by its hash) and a small one (the only candidate).
    let movie = films.join("A Film (2001)/A Film (2001).mkv");
    let episode = shows.join("Show (2000)/Season 1/Show S01E01.mkv");
    write(&movie, 200_000);
    write(&episode, 40);
    write(&films.join("Other Film (2003).mkv"), 30);
    for d in [
        &films,
        &films.join("A Film (2001)"),
        &shows,
        &shows.join("Show (2000)"),
        &shows.join("Show (2000)/Season 1"),
    ] {
        set_mtime(d, MTIME);
    }
    let (lib, db) = library(&dir, false);
    lib.init().await.unwrap();
    lib.scan(true).await.unwrap();
    lib.identify_pending().await.unwrap();
    let ids = |db: Db, path: std::path::PathBuf| async move {
        let p = path.to_string_lossy().into_owned();
        db.call(move |c| {
            Ok(
                c.query_row("SELECT id, item_id, missing_since IS NULL FROM files WHERE path = ?1", [p], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, bool>(2)?))
                })?,
            )
        })
        .await
        .unwrap()
    };
    let (movie_file, movie_item, _) = ids(db.clone(), movie.clone()).await;
    let (ep_file, ep_item, _) = ids(db.clone(), episode.clone()).await;
    let (movie_item, ep_item) = (movie_item.unwrap(), ep_item.unwrap());
    // Watch state: per title (user_data) and per file (user_file_positions).
    let (mi, ei, mf, ef) = (movie_item.clone(), ep_item.clone(), movie_file.clone(), ep_file.clone());
    db.call(move |c| {
        c.execute("INSERT INTO users(id, name, password_hash, created_at) VALUES ('u1', 'viewer', 'x', 0)", [])?;
        for (item, file) in [(&mi, &mf), (&ei, &ef)] {
            c.execute(
                "INSERT INTO user_data(user_id, item_id, position, played, play_count, updated_at, position_file) VALUES ('u1', ?1, 600, 0, 0, 0, ?2)",
                [item, file],
            )?;
            c.execute(
                "INSERT INTO user_file_positions(user_id, file_id, position, updated_at) VALUES ('u1', ?1, 600, 0)",
                [file],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();

    // Renamed inside a folder whose parent changed too (a file gone from the films root, a new
    // file in the show's folder).
    let (movie2, episode2) = (
        films.join("A Film (2001)/A Film (2001) 1080p.mkv"),
        shows.join("Show (2000)/Season 1/Show - S01E01.mkv"),
    );
    std::fs::rename(&movie, &movie2).unwrap();
    std::fs::rename(&episode, &episode2).unwrap();
    std::fs::remove_file(films.join("Other Film (2003).mkv")).unwrap();
    write(&shows.join("Show (2000)/Show S00E01.mkv"), 50);
    let r = lib.scan(false).await.unwrap();
    // (`missing` counts the renamed files too: they went missing before their new paths were matched.)
    assert_eq!((r.renamed, r.new_files, r.missing), (2, 1, 3), "{r:?}");
    lib.identify_pending().await.unwrap();

    // The episode's title is keyed by its show and number: the same item. The movie's (no TMDB
    // here) by its path: a new item, which the old id redirects to and the watch state follows.
    for (old, new, file, item, same_item) in [
        (&movie, &movie2, &movie_file, &movie_item, false),
        (&episode, &episode2, &ep_file, &ep_item, true),
    ] {
        let p = old.to_string_lossy().into_owned();
        let left: i64 = db
            .call(move |c| Ok(c.query_row("SELECT COUNT(*) FROM files WHERE path = ?1", [p], |r| r.get(0))?))
            .await
            .unwrap();
        assert_eq!(left, 0, "no file is left at the old path: {old:?}");
        let (id, item_now, present) = ids(db.clone(), new.clone()).await;
        assert_eq!((&id, present), (file, true), "the file keeps its id: {new:?}");
        let item_now = item_now.unwrap();
        if same_item {
            assert_eq!(&item_now, item, "the title keeps its id: {new:?}");
        } else {
            let (o, n) = (item.clone(), item_now.clone());
            let redirected: bool = db
                .call(move |c| {
                    Ok(c.query_row(
                        "SELECT COUNT(*) FROM item_redirects WHERE old_id = ?1 AND new_id = ?2",
                        [o, n],
                        |r| r.get::<_, i64>(0),
                    )? == 1)
                })
                .await
                .unwrap();
            assert!(redirected, "the old title id leads to the new one: {new:?}");
        }
        let (i, f) = (item_now.clone(), file.clone());
        let state: (f64, f64) = db
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT d.position, p.position FROM user_data d JOIN files f ON f.id = d.position_file
                     JOIN user_file_positions p ON p.file_id = f.id AND p.user_id = d.user_id
                     WHERE d.user_id = 'u1' AND d.item_id = ?1 AND f.id = ?2 AND f.item_id = ?1 AND f.missing_since IS NULL",
                    [i, f],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(state, (600.0, 600.0), "the watch state is the title's and the file's: {new:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file copied over another in place (`cp new old`, a download client or a file manager
/// replacing it) keeps the folder's mtime, so a quick pass doesn't list the folder. A
/// damaged (`broken`) file is looked at again on every pass: replaced, by a file of another size
/// or only another mtime, it takes the new stamp and is probed again. A healthy file replaced
/// that way takes its new stamp when playback finds it changed (`file_changed`). Files that
/// didn't change keep their id, title and stamps.
#[tokio::test]
async fn file_replaced_in_place() {
    let dir = std::env::temp_dir().join(format!("moonrise-replaced-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let season = dir.join("shows/Show (2000)/Season 1");
    let eps: Vec<_> = (1..=4).map(|n| season.join(format!("Show S01E0{n}.mkv"))).collect();
    for (i, e) in eps.iter().enumerate() {
        write(e, 200_000 + i);
    }
    write(&dir.join("films/Some Film (1999)/Some Film (1999).mkv"), 10);
    let (lib, db) = library(&dir, false);
    lib.init().await.unwrap();
    lib.scan(true).await.unwrap();
    lib.identify_pending().await.unwrap();
    let row = |p: &std::path::Path| {
        let (db, p) = (db.clone(), p.to_string_lossy().into_owned());
        async move {
            db.call(move |c| {
                Ok(c.query_row(
                    "SELECT id, item_id, size, mtime, stable_since, role FROM files WHERE path = ?1",
                    [p],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, i64>(2)?,
                            r.get::<_, i64>(3)?,
                            r.get::<_, i64>(4)?,
                            r.get::<_, String>(5)?,
                        ))
                    },
                )?)
            })
            .await
            .unwrap()
        }
    };
    let to_probe = || {
        let db = db.clone();
        async move {
            db.call(|c| {
                let mut st = c.prepare(super::scanner::PROBE_TODO)?;
                let rows = st
                    .query_map([None::<String>], |r| r.get::<_, String>(1))?
                    .collect::<Result<Vec<String>, _>>()?;
                Ok(rows)
            })
            .await
            .unwrap()
        }
    };
    // Every file probed (by this prober); E01 and E02 found damaged on playback.
    db.call(|c| {
        c.execute(
            "UPDATE files SET probed_stamp = size || '|' || mtime, container = 'matroska,webm', duration = 1500, identified_at = 1",
            [],
        )?;
        c.execute(
            "INSERT INTO files_probe(path, size, mtime, probe, probed_at) SELECT path, size, mtime, '{}', 0 FROM files",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    for e in &eps[..2] {
        let (_, _, size, mtime, ..) = row(e).await;
        let p = e.to_string_lossy().into_owned();
        assert!(
            super::mark_damaged(&db, p, size as u64, mtime, "damaged or incomplete: truncated".into())
                .await
                .unwrap()
        );
    }
    let before: Vec<_> = futures_util::future::join_all(eps.iter().map(|e| row(e))).await;
    assert!(to_probe().await.is_empty(), "nothing to probe: {:?}", to_probe().await);
    let r = lib.scan(false).await.unwrap();
    assert!(!r.changed_anything(), "{r:?}");

    // Copied over in place: E01 by a good file of another size, E02 by one of the same size
    // (another mtime), E03 (healthy) by another file. The folder's mtime stays.
    let season_mtime = std::fs::metadata(&season).unwrap().modified().unwrap();
    write(&eps[0], 300_000);
    set_mtime(&eps[0], MTIME + 100);
    write(&eps[1], 200_001);
    set_mtime(&eps[1], MTIME + 200);
    write(&eps[2], 250_000);
    set_mtime(&eps[2], MTIME + 300);
    assert_eq!(
        std::fs::metadata(&season).unwrap().modified().unwrap(),
        season_mtime,
        "an in-place copy keeps the folder's mtime"
    );

    let r = lib.scan(false).await.unwrap();
    assert_eq!((r.dirs_listed, r.changed, r.new_files, r.missing), (0, 2, 0, 0), "{r:?}");
    for (e, size, mtime) in [(&eps[0], 300_000, MTIME + 100), (&eps[1], 200_001, MTIME + 200)] {
        let now = row(e).await;
        let was = &before[eps.iter().position(|x| x == e).unwrap()];
        assert_eq!(
            (&now.0, &now.1),
            (&was.0, &was.1),
            "the replaced file keeps its id and title: {e:?}"
        );
        assert_eq!((now.2, now.3), (size, mtime as i64 * 1_000_000_000), "the new stamp: {e:?}");
    }
    let probe = to_probe().await;
    let names: Vec<&str> = probe.iter().map(|p| p.rsplit('/').next().unwrap()).collect();
    assert_eq!(
        names,
        vec!["Show S01E01.mkv", "Show S01E02.mkv"],
        "the replaced damaged files are probed again"
    );

    // Healthy E03: playback finds it changed; the library takes the new stamp and probes it.
    let p3 = eps[2].to_string_lossy().into_owned();
    let m3 = crate::storage::vfs::stat(&p3).await.unwrap();
    assert!(lib.file_changed(p3.clone(), m3.size, m3.mtime).await.unwrap());
    assert!(!lib.file_changed(p3.clone(), m3.size, m3.mtime).await.unwrap(), "only once");
    let now = row(&eps[2]).await;
    assert_eq!((&now.0, &now.1, now.2), (&before[2].0, &before[2].1, 250_000));
    assert!(to_probe().await.iter().any(|p| p == &p3));

    // E04 untouched: same row, stamps and all; the next pass changes nothing.
    assert_eq!(row(&eps[3]).await, before[3]);
    let r = lib.scan(false).await.unwrap();
    assert_eq!((r.dirs_listed, r.changed), (0, 0), "{r:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A new version of a movie and of an episode, found by a later scan, brings its title back to
/// Recently Added. The first scan finds every file at once and keeps the order the
/// titles' own dates give, two versions or not; a rename moves nothing.
#[tokio::test]
async fn later_version_is_recently_added() {
    let dir = std::env::temp_dir().join(format!("moonrise-later-version-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (films, shows) = (dir.join("films"), dir.join("shows"));
    let files = [
        (films.join("Old Film (1990)/Old Film (1990) 1080p.mkv"), MTIME - 2000),
        (films.join("Old Film (1990)/Old Film (1990) 720p.mkv"), MTIME - 2000),
        (films.join("Oldest Film (1980)/Oldest Film (1980).mkv"), MTIME - 3000),
        (films.join("Newer Film (2005)/Newer Film (2005).mkv"), MTIME),
        (shows.join("Show (2000)/Season 1/Show S01E01.mkv"), MTIME - 2000),
        (shows.join("Other (2001)/Season 1/Other S01E01.mkv"), MTIME),
    ];
    for (i, (p, t)) in files.iter().enumerate() {
        write(p, 100 + i);
        set_mtime(p, *t);
    }
    let (lib, db) = library(&dir, false);
    lib.init().await.unwrap();
    let pass = || async {
        lib.scan(true).await.unwrap();
        db.call(|c| {
            c.execute("UPDATE files SET role = 'main', duration = 6000 WHERE role = 'pending'", [])?;
            // The versions are one title (as TMDB would say).
            Ok(c.execute(
                "UPDATE files SET manual_key = 'tmdb:movie:900' WHERE path LIKE '%/Old Film (1990)/%'",
                [],
            )?)
        })
        .await
        .unwrap();
        lib.identify_pending().await.unwrap();
        db.call(|c| {
            let titles = |ids: Vec<String>| -> anyhow::Result<Vec<String>> {
                ids.iter()
                    .map(|id| Ok(c.query_row("SELECT title FROM items WHERE id = ?1", [id], |r| r.get(0))?))
                    .collect()
            };
            let movies: Vec<String> = super::store::latest_movies(c, None, None, 10)?
                .iter()
                .map(|v| v["title"].as_str().unwrap().to_string())
                .collect();
            let series: Vec<String> = super::store::latest_episodes(c, None, None, 10)?.into_iter().map(|r| r.1).collect();
            Ok((movies, titles(series)?))
        })
        .await
        .unwrap()
    };
    let (movies, shows_now) = pass().await;
    assert_eq!(movies, ["Newer Film", "tmdb:movie:900", "Oldest Film"]);
    assert_eq!(shows_now, ["Other", "Show"]);

    // A later pass (first_seen is in seconds): a new version of each, a rename of the oldest.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    write(&films.join("Old Film (1990)/Old Film (1990) 2160p.mkv"), 300);
    write(&shows.join("Show (2000)/Season 1/Show S01E01 2160p.mkv"), 301);
    std::fs::rename(
        films.join("Oldest Film (1980)/Oldest Film (1980).mkv"),
        films.join("Oldest Film (1980)/Oldest Film (1980) Remux.mkv"),
    )
    .unwrap();
    let (movies, shows_now) = pass().await;
    assert_eq!(movies, ["tmdb:movie:900", "Newer Film", "Oldest Film"]);
    assert_eq!(shows_now, ["Show", "Other"]);
    let titles: Vec<i64> = db
        .call(|c| {
            ["%Old Film (1990)%", "%Show S01E01%"]
                .iter()
                .map(|p| Ok(c.query_row("SELECT COUNT(DISTINCT item_id) FROM files WHERE path LIKE ?1", [p], |r| r.get(0))?))
                .collect()
        })
        .await
        .unwrap();
    assert_eq!(titles, [1, 1], "the new files are versions of the titles there");
    let _ = std::fs::remove_dir_all(&dir);
}
