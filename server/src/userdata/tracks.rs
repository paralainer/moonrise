//! Track memory: the audio and subtitle tracks each user picked last in a file, so
//! playback resumes with them on every device (`user_file_tracks`, migration 019).
//!
//! - The apps send their choice with their playback reports (`tracks` in
//!   `POST /v1/sessions/{sid}/progress|stop`, and in each of `POST /v1/sessions/local`'s reports):
//!   `{audio: Pick, subtitle: Pick | null}`, `subtitle: null` meaning subtitles off. A missing
//!   key keeps what is stored.
//! - A track is kept as a description, not only its index (`Pick`): the index is right for the
//!   same file as long as it isn't replaced; the language, title and kind find the track in a
//!   file that changed, a download, or the next episode.
//! - The playback answer carries `tracks` (`lookup`): the file's own choice (`same_file: true`),
//!   else the latest in another version of the title, else the latest in the same show
//!   (`same_file: false`: the apps match by language and kind, never by index).
//! - The subtitle Delay is not kept here: it is kept per file and track already (the subtitle
//!   timing's `nudge`), and comes back with the track.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

/// Longest language code and title kept.
const MAX_LANGUAGE: usize = 16;
const MAX_TITLE: usize = 200;

/// One track as the app describes it. Every field is optional; what is sent is kept.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Pick {
    /// The server's track index (subtitles fetched by the server have theirs too). Absent for a
    /// track only the device knows (a download's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The track's title (what tells two tracks of one language apart: a dub, commentary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Subtitles: forced (foreign dialogue only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forced: Option<bool>,
    /// Subtitles: for the deaf and hard of hearing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdh: Option<bool>,
}

impl Pick {
    /// The pick as kept: sizes bounded, empty strings dropped, a negative index dropped.
    fn clean(mut self) -> Pick {
        fn cut(s: Option<String>, max: usize) -> Option<String> {
            s.map(|s| s.trim().chars().take(max).collect::<String>()).filter(|s| !s.is_empty())
        }
        self.index = self.index.filter(|i| (0..=100_000).contains(i));
        self.language = cut(self.language, MAX_LANGUAGE);
        self.title = cut(self.title, MAX_TITLE);
        self
    }
}

/// `tracks` in a report.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Choice {
    /// None: not said (keep what is stored).
    #[serde(default)]
    pub audio: Option<Pick>,
    /// None: not said; Some(None): subtitles off; Some(Some): this track.
    #[serde(default, deserialize_with = "present")]
    pub subtitle: Option<Option<Pick>>,
}

/// A key that is there, even as `null`, is `Some` (`null` is `Some(None)`).
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

/// Keeps `choice` as what `user` picked in `file` (of `item`), at `now`. A choice that says
/// nothing changes nothing.
pub fn record(c: &Connection, user: &str, file: &str, item: &str, choice: &Choice, now: i64) -> Result<()> {
    if choice.audio.is_none() && choice.subtitle.is_none() {
        return Ok(());
    }
    let series: Option<String> = c
        .query_row("SELECT series_id FROM items WHERE id = ?1", [item], |r| r.get(0))
        .optional()?
        .flatten();
    let audio = choice.audio.clone().map(|p| serde_json::to_string(&p.clean())).transpose()?;
    let (said_sub, subtitle, off) = match &choice.subtitle {
        None => (false, None, false),
        Some(None) => (true, None, true),
        Some(Some(p)) => (true, Some(serde_json::to_string(&p.clone().clean())?), false),
    };
    c.execute(
        "INSERT INTO user_file_tracks(user_id, file_id, item_id, series_id, audio, subtitle, subtitle_off, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(user_id, file_id) DO UPDATE SET
           item_id = excluded.item_id, series_id = excluded.series_id,
           audio = COALESCE(excluded.audio, audio),
           subtitle = CASE WHEN ?9 THEN excluded.subtitle ELSE subtitle END,
           subtitle_off = CASE WHEN ?9 THEN excluded.subtitle_off ELSE subtitle_off END,
           updated_at = excluded.updated_at",
        params![user, file, item, series, audio, subtitle, off, now, said_sub],
    )?;
    Ok(())
}

/// A kept row: file, audio, subtitle, subtitles off.
type Row = (String, Option<String>, Option<String>, bool);

/// What the playback answer says about `user`'s tracks in `file` of `item`: the file's own
/// choice, else the latest in another version of the title, else the latest in the same show.
/// None: nothing kept.
pub fn lookup(c: &Connection, user: &str, file: Option<&str>, item: &str) -> Result<Option<Value>> {
    let cols = "file_id, audio, subtitle, subtitle_off";
    let read = |r: &rusqlite::Row| -> rusqlite::Result<Row> { Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)) };
    let mut row: Option<Row> = None;
    if let Some(f) = file {
        row = c
            .query_row(
                &format!("SELECT {cols} FROM user_file_tracks WHERE user_id = ?1 AND file_id = ?2"),
                params![user, f],
                read,
            )
            .optional()?;
    }
    if row.is_none() {
        row = c
            .query_row(
                &format!("SELECT {cols} FROM user_file_tracks WHERE user_id = ?1 AND item_id = ?2 ORDER BY updated_at DESC LIMIT 1"),
                params![user, item],
                read,
            )
            .optional()?;
    }
    if row.is_none() {
        let series: Option<String> = c
            .query_row("SELECT series_id FROM items WHERE id = ?1", [item], |r| r.get(0))
            .optional()?
            .flatten();
        if let Some(s) = series {
            row = c
                .query_row(
                    &format!("SELECT {cols} FROM user_file_tracks WHERE user_id = ?1 AND series_id = ?2 ORDER BY updated_at DESC LIMIT 1"),
                    params![user, s],
                    read,
                )
                .optional()?;
        }
    }
    let Some((f, audio, subtitle, off)) = row else { return Ok(None) };
    let parse = |s: Option<String>| s.and_then(|s| serde_json::from_str::<Value>(&s).ok());
    Ok(Some(json!({
        "same_file": file == Some(f.as_str()),
        "audio": parse(audio),
        "subtitle": parse(subtitle),
        "subtitle_off": off,
    })))
}

/// The file a download's report is about: the one it names, when it is the title's, else the
/// version the user plays, else where the resume point was, else the title's largest file.
pub fn local_file(c: &Connection, user: &str, item: &str, named: Option<&str>) -> Result<Option<String>> {
    let roles = crate::library::TITLE_ROLES;
    if let Some(f) = named {
        let mine: bool = c.query_row(
            &format!("SELECT EXISTS (SELECT 1 FROM files WHERE id = ?1 AND item_id = ?2 AND role IN {roles})"),
            params![f, item],
            |r| r.get(0),
        )?;
        if mine {
            return Ok(Some(f.to_string()));
        }
    }
    let chosen: Option<String> = c
        .query_row(
            "SELECT file_id FROM user_versions WHERE user_id = ?1 AND item_id = ?2",
            params![user, item],
            |r| r.get(0),
        )
        .optional()?;
    if chosen.is_some() {
        return Ok(chosen);
    }
    let resumed: Option<String> = c
        .query_row(
            "SELECT position_file FROM user_data WHERE user_id = ?1 AND item_id = ?2",
            params![user, item],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    if resumed.is_some() {
        return Ok(resumed);
    }
    Ok(c.query_row(
        &format!("SELECT id FROM files WHERE item_id = ?1 AND role IN {roles} AND missing_since IS NULL ORDER BY size DESC LIMIT 1"),
        [item],
        |r| r.get(0),
    )
    .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    /// Users u1, u2; a film m1 (files f1, f2); a show with episodes e1 (file g1), e2 (file g2).
    fn setup(c: &Connection) {
        c.execute_batch(
            "INSERT INTO users(id, name, password_hash, created_at) VALUES ('u1', 'ann', 'x', 0), ('u2', 'bob', 'x', 0);
             INSERT INTO items(id, kind, library_id, title, key, identify_state, added_at, updated_at, runtime)
               VALUES ('m1', 'movie', 'l', 'Film', 'tmdb:movie:1', 'matched', 0, 0, 6000);
             INSERT INTO items(id, kind, library_id, title, key, identify_state, added_at, updated_at, series_id)
               VALUES ('s1', 'series', 'l', 'Show', 'tmdb:tv:1', 'matched', 0, 0, NULL),
                      ('e1', 'episode', 'l', 'One', 'tmdb:tv:1:1:1', 'matched', 0, 0, 's1'),
                      ('e2', 'episode', 'l', 'Two', 'tmdb:tv:1:1:2', 'matched', 0, 0, 's1');
             INSERT INTO files(id, library_id, path, size, mtime, role, first_seen, last_seen, stable_since, item_id, duration)
               VALUES ('f1', 'l', '/m/film.mkv', 10, 5, 'main', 0, 0, 0, 'm1', 6000),
                      ('f2', 'l', '/m/film.2.mkv', 9, 5, 'main', 0, 0, 0, 'm1', 6000),
                      ('g1', 'l', '/s/e1.mkv', 9, 5, 'main', 0, 0, 0, 'e1', 1500),
                      ('g2', 'l', '/s/e2.mkv', 9, 5, 'main', 0, 0, 0, 'e2', 1500);",
        )
        .unwrap();
    }

    fn choice(json: &str) -> Choice {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_missing_subtitle_is_not_off() {
        assert_eq!(choice("{}"), Choice::default());
        assert_eq!(choice(r#"{"subtitle": null}"#).subtitle, Some(None));
        let c = choice(r#"{"audio": {"index": 2, "language": "ru", "title": "Dub"}, "subtitle": {"index": 5, "forced": true}}"#);
        assert_eq!(c.audio.unwrap().index, Some(2));
        assert_eq!(c.subtitle.unwrap().unwrap().forced, Some(true));
    }

    #[tokio::test]
    async fn the_same_file_resumes_with_its_tracks() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            setup(c);
            assert_eq!(lookup(c, "u1", Some("f1"), "m1")?, None);
            record(
                c,
                "u1",
                "f1",
                "m1",
                &choice(
                    r#"{"audio": {"index": 2, "language": "ru", "title": "Dub"}, "subtitle": {"index": 5, "language": "en", "sdh": true}}"#,
                ),
                10,
            )?;
            let t = lookup(c, "u1", Some("f1"), "m1")?.unwrap();
            assert_eq!(t["same_file"], true);
            assert_eq!(t["audio"], json!({"index": 2, "language": "ru", "title": "Dub"}));
            assert_eq!(t["subtitle"], json!({"index": 5, "language": "en", "sdh": true}));
            assert_eq!(t["subtitle_off"], false);
            // Another user has none.
            assert_eq!(lookup(c, "u2", Some("f1"), "m1")?, None);
            // Turning subtitles off keeps the audio; a report without tracks changes nothing.
            record(c, "u1", "f1", "m1", &choice(r#"{"subtitle": null}"#), 20)?;
            record(c, "u1", "f1", "m1", &Choice::default(), 30)?;
            let t = lookup(c, "u1", Some("f1"), "m1")?.unwrap();
            assert_eq!(t["audio"]["index"], 2);
            assert_eq!(t["subtitle"], Value::Null);
            assert_eq!(t["subtitle_off"], true);
            // Subtitles on again.
            record(c, "u1", "f1", "m1", &choice(r#"{"subtitle": {"index": 6}}"#), 40)?;
            let t = lookup(c, "u1", Some("f1"), "m1")?.unwrap();
            assert_eq!(
                (t["subtitle"]["index"].clone(), t["subtitle_off"].clone()),
                (json!(6), json!(false))
            );
            // Another version of the film: the title's choice, by description only.
            let t = lookup(c, "u1", Some("f2"), "m1")?.unwrap();
            assert_eq!(t["same_file"], false);
            assert_eq!(t["audio"]["language"], "ru");
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn the_next_episode_gets_the_shows_latest_choice() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            setup(c);
            record(
                c,
                "u1",
                "g1",
                "e1",
                &choice(r#"{"audio": {"index": 1, "language": "ja"}, "subtitle": {"index": 3, "language": "en"}}"#),
                10,
            )?;
            let t = lookup(c, "u1", Some("g2"), "e2")?.unwrap();
            assert_eq!(t["same_file"], false);
            assert_eq!(t["audio"]["language"], "ja");
            assert_eq!(t["subtitle"]["language"], "en");
            // The film is in no show: nothing.
            assert_eq!(lookup(c, "u1", Some("f1"), "m1")?, None);
            // The episode's own choice wins over the show's later one.
            record(c, "u1", "g2", "e2", &choice(r#"{"subtitle": null}"#), 20)?;
            let t = lookup(c, "u1", Some("g1"), "e1")?.unwrap();
            assert_eq!((t["same_file"].clone(), t["subtitle_off"].clone()), (json!(true), json!(false)));
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn picks_are_kept_bounded() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            setup(c);
            let long = "x".repeat(1000);
            let body = format!(r#"{{"audio": {{"index": -4, "language": " ", "title": "{long}"}}}}"#);
            record(c, "u1", "f1", "m1", &choice(&body), 10)?;
            let t = lookup(c, "u1", Some("f1"), "m1")?.unwrap();
            assert_eq!(t["audio"]["index"], Value::Null);
            assert_eq!(t["audio"]["language"], Value::Null);
            assert_eq!(t["audio"]["title"].as_str().unwrap().len(), MAX_TITLE);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_downloads_file() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            setup(c);
            assert_eq!(local_file(c, "u1", "m1", Some("f2"))?.as_deref(), Some("f2"));
            // Another title's file is not taken: the largest of this one's.
            assert_eq!(local_file(c, "u1", "m1", Some("g1"))?.as_deref(), Some("f1"));
            c.execute(
                "INSERT INTO user_versions(user_id, item_id, file_id, updated_at) VALUES ('u1', 'm1', 'f2', 0)",
                [],
            )?;
            assert_eq!(local_file(c, "u1", "m1", None)?.as_deref(), Some("f2"));
            Ok(())
        })
        .await
        .unwrap();
    }
}
