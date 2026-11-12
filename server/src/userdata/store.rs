//! What the watch-state endpoints (`api.rs`) read and write, each in one `Db::call`: a
//! session's report, a batch of reports from a device's local copies, and the version a user
//! plays. Marking played, Continue Watching and the show's Play button are in `mod.rs` and
//! `continuing.rs`.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};

use super::Outcome;
use crate::library::util;

/// A playback report on a stream session (`POST /v1/sessions/{sid}/{event}`), checked.
pub(crate) struct SessionReport {
    /// The session id.
    pub(crate) session: String,
    /// `start`, `progress` or `stop`.
    pub(crate) event: String,
    /// Seconds into the title (finite, not negative).
    pub(crate) position: Option<f64>,
    /// Where the app's end credits start.
    pub(crate) credits: Option<f64>,
    /// The audio and subtitle tracks playing (`tracks::Choice`), when the app says.
    pub(crate) tracks: Option<super::tracks::Choice>,
    pub(crate) now: i64,
}

/// A stream session as it is open in memory: owner, item, duration, file.
pub(crate) type LiveSession = (Option<String>, Option<String>, f64, Option<String>);

/// Why a session's report isn't recorded.
#[derive(Debug, PartialEq)]
pub(crate) enum ReportRefused {
    /// No such session, open or recorded.
    UnknownSession,
    /// Another user's session.
    NotYours,
    /// The session plays something that isn't a library title.
    NotInLibrary,
}

/// Records a session's report for `user`. `live` is the session as it is open in memory (owner,
/// item, duration, file); without it the one recorded when it opened. Answers the item, the outcome
/// of a position, and the user's watch state of the item after it.
pub(crate) fn session_report(
    c: &Connection,
    user: &str,
    live: Option<LiveSession>,
    r: &SessionReport,
) -> Result<Result<(String, Option<Outcome>, Value), ReportRefused>> {
    let (uid, sid2, ev, position, app_credits, now) = (user, &r.session, &r.event, r.position, r.credits, r.now);
    // The session: open in memory, else (after a restart) as recorded when it opened.
    let (owner, item, duration, file) = match live {
        Some((owner, item, d, f)) => (owner, item, Some(d), f),
        None => match c
            .query_row(
                "SELECT user_id, item_id, duration, file_id FROM play_sessions WHERE id = ?1",
                [&sid2],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<f64>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?
        {
            Some((o, i, d, f)) => (Some(o), Some(i), d, f),
            None => return Ok(Err(ReportRefused::UnknownSession)),
        },
    };
    if owner.as_deref() != Some(uid) {
        return Ok(Err(ReportRefused::NotYours));
    }
    let Some(item) = item else {
        return Ok(Err(ReportRefused::NotInLibrary));
    };
    let duration = duration.filter(|d| *d > 0.0).or(super::item_duration(c, &item)?);
    let outcome = match (ev.as_str(), position) {
        ("start", _) => {
            super::record_start(c, uid, &item, now)?;
            None
        }
        (_, Some(p)) => {
            let credits = super::report_credits(app_credits, super::credits_start(c, &item)?, duration);
            Some(super::record_position(c, uid, &item, p, duration, credits, Some(sid2), now)?)
        }
        (_, None) => None,
    };
    if let (Some(t), Some(f)) = (&r.tracks, &file) {
        super::tracks::record(c, uid, f, &item, t, now)?;
    }
    c.execute(
        "UPDATE play_sessions SET last_report_at = ?2, position = COALESCE(?3, position) WHERE id = ?1",
        rusqlite::params![sid2, now, position],
    )?;
    let mut v = vec![
        json!({"id": item, "kind": c.query_row("SELECT kind FROM items WHERE id = ?1", [&item], |r| r.get::<_, String>(0)).optional()?, "runtime": duration}),
    ];
    super::attach(c, uid, &mut v)?;
    Ok(Ok((item, outcome, v.remove(0)["user"].clone())))
}

/// One report of playback of a copy on the device (`POST /v1/sessions/local`).
#[derive(Deserialize)]
pub(crate) struct LocalReport {
    pub(crate) viewing: String,
    pub(crate) item: String,
    pub(crate) event: String,
    pub(crate) position: Option<f64>,
    /// Unix seconds.
    pub(crate) at: i64,
    pub(crate) duration: Option<f64>,
    /// As in a session report.
    pub(crate) credits: Option<f64>,
    /// As in a session report (`tracks::Choice`).
    #[serde(default)]
    pub(crate) tracks: Option<super::tracks::Choice>,
    /// The file (version) the download was made from, when the app knows it.
    #[serde(default)]
    pub(crate) file: Option<String>,
}

/// Records a device's reports for `uid`, each with the current id of its title (None: not
/// found). Answers each one's result: `{item, recorded, outcome|reason}`.
pub(crate) fn local_reports(c: &Connection, uid: &str, reports: Vec<(LocalReport, Option<String>)>, now: i64) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for (r, item) in reports {
        let kind: Option<String> = match &item {
            Some(i) => c.query_row("SELECT kind FROM items WHERE id = ?1", [i], |x| x.get(0)).optional()?,
            None => None,
        };
        let (Some(item), Some("movie" | "episode")) = (item, kind.as_deref()) else {
            out.push(json!({"item": r.item, "recorded": false, "reason": "no such item"}));
            continue;
        };
        if !matches!(r.event.as_str(), "start" | "progress" | "stop") || r.viewing.is_empty() || r.viewing.len() > 64 {
            out.push(json!({"item": item, "recorded": false, "reason": "bad report"}));
            continue;
        }
        let at = r.at.clamp(0, now);
        let sid = format!("local-{}", r.viewing);
        let duration = r.duration.filter(|d| d.is_finite() && *d > 0.0).or(super::item_duration(c, &item)?);
        c.execute(
            "INSERT OR IGNORE INTO play_sessions(id, user_id, item_id, duration, method, started_at) VALUES (?1, ?2, ?3, ?4, 'local', ?5)",
            rusqlite::params![sid, uid, item, duration, at],
        )?;
        let owner: String = c.query_row("SELECT user_id FROM play_sessions WHERE id = ?1", [&sid], |x| x.get(0))?;
        if owner != uid {
            out.push(json!({"item": item, "recorded": false, "reason": "not your viewing"}));
            continue;
        }
        let changed: Option<i64> = c
            .query_row(
                "SELECT updated_at FROM user_data WHERE user_id = ?1 AND item_id = ?2",
                rusqlite::params![uid, item],
                |x| x.get(0),
            )
            .optional()?;
        if changed.is_some_and(|t| t > at) {
            out.push(json!({"item": item, "recorded": false, "reason": "stale"}));
            continue;
        }
        let position = r.position.filter(|p| p.is_finite() && *p >= 0.0);
        let outcome = match (r.event.as_str(), position) {
            ("start", _) => {
                super::record_start(c, uid, &item, at)?;
                None
            }
            (_, Some(p)) => {
                let credits = super::report_credits(r.credits, super::credits_start(c, &item)?, duration);
                Some(super::record_position(c, uid, &item, p, duration, credits, Some(&sid), at)?)
            }
            (_, None) => None,
        };
        if let Some(t) = &r.tracks {
            if let Some(f) = super::tracks::local_file(c, uid, &item, r.file.as_deref())? {
                super::tracks::record(c, uid, &f, &item, t, at)?;
            }
        }
        c.execute(
            "UPDATE play_sessions SET last_report_at = ?2, position = COALESCE(?3, position) WHERE id = ?1",
            rusqlite::params![sid, at, position],
        )?;
        let outcome = outcome.map(outcome_name);
        out.push(json!({"item": item, "recorded": true, "outcome": outcome}));
    }
    Ok(out)
}

/// An outcome as the reports' answers name it.
pub(crate) fn outcome_name(o: Outcome) -> &'static str {
    match o {
        Outcome::Played => "played",
        Outcome::Resume(_) => "resume",
        Outcome::Clear => "cleared",
    }
}

/// Remembers `file` as the version `uid` plays of `item`, if it is one of the title's present
/// files. Answers whether it is.
pub(crate) fn choose_version(c: &Connection, uid: &str, item: &str, file: &str) -> Result<bool> {
    let mine: bool = c.query_row(
        &format!(
            "SELECT EXISTS (SELECT 1 FROM files WHERE id = ?1 AND item_id = ?2 AND role IN {} AND missing_since IS NULL)",
            crate::library::TITLE_ROLES
        ),
        rusqlite::params![file, item],
        |r| r.get(0),
    )?;
    if mine {
        c.execute(
            "INSERT INTO user_versions(user_id, item_id, file_id, updated_at) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(user_id, item_id) DO UPDATE SET file_id = excluded.file_id, updated_at = excluded.updated_at",
            rusqlite::params![uid, item, file, util::now()],
        )?;
    }
    Ok(mine)
}

/// Forgets the version `uid` chose for `item` (they get the default again).
pub(crate) fn forget_version(c: &Connection, uid: &str, item: &str) -> Result<usize> {
    Ok(c.execute(
        "DELETE FROM user_versions WHERE user_id = ?1 AND item_id = ?2",
        rusqlite::params![uid, item],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    /// A user, a film of 6000 s with one file, and a recorded session `s1` of the film.
    fn setup(c: &Connection) {
        c.execute_batch(
            "INSERT INTO users(id, name, password_hash, created_at) VALUES ('u1', 'ann', 'x', 0), ('u2', 'bob', 'x', 0);
             INSERT INTO items(id, kind, library_id, title, key, identify_state, added_at, updated_at, runtime)
               VALUES ('m1', 'movie', 'l', 'Film', 'tmdb:movie:1', 'matched', 0, 0, 6000);
             INSERT INTO files(id, library_id, path, size, mtime, role, first_seen, last_seen, stable_since, item_id, duration)
               VALUES ('f1', 'l', '/m/film.mkv', 10, 5, 'main', 0, 0, 0, 'm1', 6000),
                      ('f2', 'l', '/m/film.2.mkv', 9, 5, 'main', 0, 0, 0, 'm1', 6000);
             INSERT INTO play_sessions(id, user_id, item_id, duration, method, started_at) VALUES ('s1', 'u1', 'm1', 6000, 'copy', 0);",
        )
        .unwrap();
    }

    fn report(session: &str, event: &str, position: Option<f64>) -> SessionReport {
        SessionReport {
            session: session.into(),
            event: event.into(),
            position,
            credits: None,
            tracks: None,
            now: 1000,
        }
    }

    #[tokio::test]
    async fn session_reports() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            setup(c);
            // Unknown, someone else's, and a live session playing something outside the library.
            assert_eq!(
                session_report(c, "u1", None, &report("nope", "start", None))?,
                Err(ReportRefused::UnknownSession)
            );
            assert_eq!(
                session_report(c, "u2", None, &report("s1", "start", None))?,
                Err(ReportRefused::NotYours)
            );
            let live = Some((Some("u1".to_string()), None, 100.0, None));
            assert_eq!(
                session_report(c, "u1", live, &report("live", "start", None))?,
                Err(ReportRefused::NotInLibrary)
            );
            // A position halfway: a resume point, and the session remembers where it was.
            let (item, outcome, user) = session_report(c, "u1", None, &report("s1", "progress", Some(3000.0)))?.unwrap();
            assert_eq!(item, "m1");
            assert_eq!(outcome, Some(Outcome::Resume(3000.0)));
            assert_eq!(user["position"], 3000.0);
            let (at, pos): (i64, f64) = c.query_row("SELECT last_report_at, position FROM play_sessions WHERE id = 's1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
            assert_eq!((at, pos), (1000, 3000.0));
            // Near the end: played.
            let (_, outcome, user) = session_report(c, "u1", None, &report("s1", "stop", Some(5900.0)))?.unwrap();
            assert_eq!(outcome, Some(Outcome::Played));
            assert_eq!(user["played"], true);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn local_reports_check_each_one() {
        let db = Db::memory().unwrap();
        let out = db
            .call(|c| {
                setup(c);
                let r = |viewing: &str, item: &str, event: &str, position: Option<f64>, at: i64| LocalReport {
                    viewing: viewing.into(),
                    item: item.into(),
                    event: event.into(),
                    position,
                    at,
                    duration: None,
                    credits: None,
                    tracks: None,
                    file: None,
                };
                let first = local_reports(
                    c,
                    "u1",
                    vec![
                        (r("v1", "gone", "start", None, 10), None),
                        (r("v1", "m1", "pause", None, 10), Some("m1".into())),
                        (r("v1", "m1", "progress", Some(3000.0), 500), Some("m1".into())),
                    ],
                    1000,
                )?;
                // Another user's viewing id, and a report older than the title's last change.
                let second = local_reports(c, "u2", vec![(r("v1", "m1", "progress", Some(10.0), 600), Some("m1".into()))], 1000)?;
                let third = local_reports(
                    c,
                    "u1",
                    vec![(r("v2", "m1", "progress", Some(100.0), 400), Some("m1".into()))],
                    1000,
                )?;
                Ok((first, second, third))
            })
            .await
            .unwrap();
        let reasons = |v: &[Value]| {
            v.iter()
                .map(|r| r["reason"].as_str().or(r["outcome"].as_str()).unwrap_or("").to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(reasons(&out.0), ["no such item", "bad report", "resume"]);
        assert_eq!(out.0[0]["item"], "gone");
        assert_eq!(reasons(&out.1), ["not your viewing"]);
        assert_eq!(reasons(&out.2), ["stale"]);
    }

    /// The tracks a report names are kept for the session's file (live or recorded),
    /// and a download's for the file it names.
    #[tokio::test]
    async fn reports_keep_the_tracks() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            setup(c);
            c.execute("UPDATE play_sessions SET file_id = 'f2' WHERE id = 's1'", [])?;
            let tracks = |json: &str| Some(serde_json::from_str::<super::super::tracks::Choice>(json).unwrap());
            let mut r = report("s1", "progress", Some(3000.0));
            r.tracks = tracks(r#"{"audio": {"index": 2, "language": "ru"}, "subtitle": null}"#);
            session_report(c, "u1", None, &r)?.unwrap();
            let t = super::super::tracks::lookup(c, "u1", Some("f2"), "m1")?.unwrap();
            assert_eq!((t["audio"]["index"].clone(), t["subtitle_off"].clone()), (json!(2), json!(true)));
            // A live session: its file.
            let mut r = report("live", "progress", Some(3000.0));
            r.tracks = tracks(r#"{"subtitle": {"index": 4}}"#);
            let live = Some((Some("u1".to_string()), Some("m1".to_string()), 6000.0, Some("f1".to_string())));
            session_report(c, "u1", live, &r)?.unwrap();
            let t = super::super::tracks::lookup(c, "u1", Some("f1"), "m1")?.unwrap();
            assert_eq!((t["same_file"].clone(), t["subtitle"]["index"].clone()), (json!(true), json!(4)));
            // A download's report, played offline later.
            let local = LocalReport {
                viewing: "v9".into(),
                item: "m1".into(),
                event: "stop".into(),
                position: Some(3100.0),
                at: 2000,
                duration: None,
                credits: None,
                tracks: tracks(r#"{"audio": {"language": "en", "title": "Commentary"}}"#),
                file: Some("f2".into()),
            };
            let out = local_reports(c, "u1", vec![(local, Some("m1".into()))], 3000)?;
            assert_eq!(out[0]["recorded"], true);
            let t = super::super::tracks::lookup(c, "u1", Some("f2"), "m1")?.unwrap();
            assert_eq!(t["audio"], json!({"language": "en", "title": "Commentary"}));
            assert_eq!(t["subtitle_off"], true, "the download said nothing about subtitles");
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn versions() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            setup(c);
            assert!(!choose_version(c, "u1", "m1", "other")?, "not one of the title's files");
            assert!(choose_version(c, "u1", "m1", "f2")?);
            assert!(choose_version(c, "u1", "m1", "f1")?, "a second choice replaces the first");
            let chosen: String = c.query_row(
                "SELECT file_id FROM user_versions WHERE user_id = 'u1' AND item_id = 'm1'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(chosen, "f1");
            assert_eq!(forget_version(c, "u1", "m1")?, 1);
            assert_eq!(forget_version(c, "u1", "m1")?, 0);
            Ok(())
        })
        .await
        .unwrap();
    }
}
