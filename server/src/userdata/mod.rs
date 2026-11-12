//! Watch state (Phase 3): Moonrise's own, per user, never synced with any other server.
//!
//! Rules (the usual media-server defaults, as in `UserDataManager.UpdatePlayState`):
//! - under 5 % in: no resume point;
//! - over 90 %, or inside the final credits (a "credits" chapter in the last half, or the
//!   credits start the app reports, the later of the two): played, the resume point cleared. A
//!   movie whose credits start is known is played only from it, not at 90 % before it: a long
//!   film's final scenes can run past 90 %;
//! - a title under 5 minutes long that is stopped between the two: played (no resume point);
//! - otherwise the position is the resume point. A played title being watched again keeps
//!   `played` and gets a resume point.
//!
//! `play_count` counts completed viewings: once per playback session that reaches "played",
//! or a mark as played. Only movies and episodes have rows; a season's or series' state is
//! derived from its episodes (marking one cascades to them).
//!
//! Home rows: `resume` (titles with a resume point, latest first), `next_up` (per series, the
//! first unplayed episode in airing order after the most recently played one, specials left
//! out on both sides; a series whose next episode is in progress is left to `resume`). Both are
//! kept for older apps; newer ones show one Continue Watching row (`continuing`).

pub mod api;
pub mod continuing;
pub mod search_start;
pub(crate) mod store;
pub mod tracks;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::library::{rfc3339, store::visible_sql, TITLE_ROLES};

pub const MIN_RESUME_PCT: f64 = 5.0;
pub const MAX_RESUME_PCT: f64 = 90.0;
pub const MIN_RESUME_SECS: f64 = 300.0;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Outcome {
    /// Keep this resume point.
    Resume(f64),
    /// No resume point (too early).
    Clear,
    /// Watched to the end: played, no resume point.
    Played,
}

/// What a reported position means for a title of `duration` seconds. `movie`: a known credits
/// start replaces the 90 % mark (the story can run past it), instead of only coming before it.
pub fn evaluate(position: f64, duration: Option<f64>, credits_start: Option<f64>, movie: bool) -> Outcome {
    let Some(d) = duration.filter(|d| *d > 0.0) else {
        // Without a runtime, any report means played.
        return if position > 0.0 { Outcome::Played } else { Outcome::Clear };
    };
    if position <= 0.0 {
        return Outcome::Clear;
    }
    let pct = position / d * 100.0;
    if pct < MIN_RESUME_PCT {
        Outcome::Clear
    } else if (pct > MAX_RESUME_PCT && !(movie && credits_start.is_some()))
        || credits_start.is_some_and(|c| position >= c)
        || d < MIN_RESUME_SECS
    {
        // Watched to the end, or a short title (no resume point under 5 minutes).
        Outcome::Played
    } else {
        Outcome::Resume(position)
    }
}

/// User ids are our own ULIDs; checked before they are put into SQL text (ORDER BY and
/// filter expressions shared by a count and a page query).
pub fn sql_id(user_id: &str) -> Option<&str> {
    (!user_id.is_empty() && user_id.len() <= 32 && user_id.bytes().all(|b| b.is_ascii_alphanumeric())).then_some(user_id)
}

/// The length of a title: its largest file's (on the title's timeline: a Matroska file with a
/// start time has its last timestamp as its duration, `media::OwnClock`).
pub fn item_duration(c: &Connection, item_id: &str) -> Result<Option<f64>> {
    let row: Option<(Option<f64>, Option<String>)> = c
        .query_row(
            &format!(
                "SELECT f.duration, fp.probe FROM files f LEFT JOIN files_probe fp ON fp.path = f.path AND fp.size = f.size AND fp.mtime = f.mtime
                 WHERE f.item_id = ?1 AND f.role IN {TITLE_ROLES} AND f.missing_since IS NULL ORDER BY f.size DESC LIMIT 1"
            ),
            [item_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((duration, probe)) = row else { return Ok(None) };
    Ok(own_clock(probe.as_deref()).and_then(|c| c.length).or(duration))
}

fn own_clock(probe: Option<&str>) -> Option<crate::media::OwnClock> {
    crate::media::OwnClock::of(&serde_json::from_str::<Value>(probe?).ok()?)
}

/// Where the final credits start (a "credits" chapter in the second half), if marked; on the
/// title's timeline.
pub fn credits_start(c: &Connection, item_id: &str) -> Result<Option<f64>> {
    let row: Option<(Option<String>, Option<f64>, Option<String>)> = c
        .query_row(
            &format!(
                "SELECT fp.chapters, f.duration, fp.probe FROM files f LEFT JOIN files_probe fp ON fp.path = f.path AND fp.size = f.size AND fp.mtime = f.mtime
                 WHERE f.item_id = ?1 AND f.role IN {TITLE_ROLES} AND f.missing_since IS NULL ORDER BY f.size DESC LIMIT 1"
            ),
            [item_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((Some(ch), Some(d), probe)) = row else { return Ok(None) };
    let ch: Vec<Value> = serde_json::from_str(&ch).unwrap_or_default();
    let clock = own_clock(probe.as_deref());
    Ok(final_credits(&ch, clock.and_then(|c| c.length).unwrap_or(d), clock))
}

/// The credits start a report goes by: the file's credits chapter, or the one the app reported
/// (the credits it offers, never before a community database's start), whichever is later (later
/// is never story), so never before what the app reported. A reported start outside the title's second half is ignored.
pub fn report_credits(reported: Option<f64>, chapter: Option<f64>, duration: Option<f64>) -> Option<f64> {
    let reported = reported.filter(|c| c.is_finite() && *c > 0.0 && duration.filter(|d| *d > 0.0).is_none_or(|d| *c >= d * 0.5 && *c < d));
    match (reported, chapter) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

fn final_credits(chapters: &[Value], duration: f64, clock: Option<crate::media::OwnClock>) -> Option<f64> {
    let starts = crate::library::api::chapter_starts(chapters, clock);
    chapters
        .iter()
        .zip(starts)
        .filter(|(c, _)| c["tags"]["title"].as_str().and_then(crate::library::api::segment_kind) == Some("credits"))
        .map(|(_, s)| s)
        .filter(|s| *s >= duration * 0.5 && *s < duration)
        .reduce(f64::max)
}

fn ensure_row(c: &Connection, user: &str, item: &str, now: i64) -> Result<()> {
    c.execute(
        "INSERT OR IGNORE INTO user_data(user_id, item_id, updated_at) VALUES (?1, ?2, ?3)",
        params![user, item, now],
    )?;
    Ok(())
}

/// Playback started: the title was played now.
pub fn record_start(c: &Connection, user: &str, item: &str, now: i64) -> Result<()> {
    ensure_row(c, user, item, now)?;
    c.execute(
        "UPDATE user_data SET last_played_at = ?3, updated_at = ?3 WHERE user_id = ?1 AND item_id = ?2",
        params![user, item, now],
    )?;
    Ok(())
}

/// A progress or stop report at `position`. `session`: the playback session (a viewing
/// counts once per session).
#[allow(
    clippy::too_many_arguments,
    reason = "the steps' inputs, passed through; a struct for one call would only rename them"
)]
pub fn record_position(
    c: &Connection,
    user: &str,
    item: &str,
    position: f64,
    duration: Option<f64>,
    credits: Option<f64>,
    session: Option<&str>,
    now: i64,
) -> Result<Outcome> {
    let movie = c
        .query_row("SELECT kind = 'movie' FROM items WHERE id = ?1", [item], |r| r.get::<_, bool>(0))
        .optional()?
        .unwrap_or(false);
    let outcome = evaluate(position, duration, credits, movie);
    ensure_row(c, user, item, now)?;
    // The version (file) played: a resume point is kept for it too (another cut keeps its own).
    let file: Option<String> = match session {
        Some(s) => c
            .query_row("SELECT file_id FROM play_sessions WHERE id = ?1", [s], |r| r.get(0))
            .optional()?
            .flatten(),
        None => None,
    };
    match outcome {
        Outcome::Resume(p) => {
            if let Some(f) = &file {
                c.execute(
                    "INSERT INTO user_file_positions(user_id, file_id, position, updated_at) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(user_id, file_id) DO UPDATE SET position = excluded.position, updated_at = excluded.updated_at",
                    params![user, f, p, now],
                )?;
            }
            c.execute(
                "UPDATE user_data SET position_file = ?3 WHERE user_id = ?1 AND item_id = ?2",
                params![user, item, file],
            )?;
        }
        Outcome::Played | Outcome::Clear => {
            c.execute(
                "DELETE FROM user_file_positions WHERE user_id = ?1 AND file_id IN (SELECT id FROM files WHERE item_id = ?2)",
                params![user, item],
            )?;
            c.execute(
                "UPDATE user_data SET position_file = NULL WHERE user_id = ?1 AND item_id = ?2",
                params![user, item],
            )?;
        }
    }
    match outcome {
        Outcome::Played => {
            let was_played: bool = c.query_row(
                "SELECT played FROM user_data WHERE user_id = ?1 AND item_id = ?2",
                params![user, item],
                |r| r.get(0),
            )?;
            let count = match session {
                // Once per viewing, even when it was played before (a rewatch).
                Some(s) => c.execute("UPDATE play_sessions SET counted = 1 WHERE id = ?1 AND counted = 0", [s])? == 1,
                None => !was_played,
            };
            c.execute(
                "UPDATE user_data SET played = 1, position = 0, play_count = play_count + ?3, last_played_at = ?4, updated_at = ?4 WHERE user_id = ?1 AND item_id = ?2",
                params![user, item, i64::from(count), now],
            )?;
        }
        Outcome::Resume(p) => {
            c.execute(
                "UPDATE user_data SET position = ?3, last_played_at = ?4, updated_at = ?4 WHERE user_id = ?1 AND item_id = ?2",
                params![user, item, p, now],
            )?;
        }
        Outcome::Clear => {
            c.execute(
                "UPDATE user_data SET position = 0, last_played_at = ?3, updated_at = ?3 WHERE user_id = ?1 AND item_id = ?2",
                params![user, item, now],
            )?;
        }
    }
    Ok(outcome)
}

/// The movies and episodes an item stands for (a series or season: its episodes).
fn playable_ids(c: &Connection, item: &str) -> Result<Vec<String>> {
    let kind: Option<String> = c
        .query_row("SELECT kind FROM items WHERE id = ?1", [item], |r| r.get(0))
        .optional()?;
    Ok(match kind.as_deref() {
        Some("movie" | "episode") => vec![item.to_string()],
        Some("series") => {
            let mut st = c.prepare("SELECT id FROM items WHERE kind = 'episode' AND series_id = ?1")?;
            let rows = st.query_map([item], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
            rows
        }
        Some("season") => {
            let mut st = c.prepare("SELECT id FROM items WHERE kind = 'episode' AND parent_id = ?1")?;
            let rows = st.query_map([item], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
            rows
        }
        _ => Vec::new(),
    })
}

/// Marks an item (a series or season: all its episodes) played or unplayed. Returns how many
/// titles changed; None: no such item.
pub fn set_played(c: &mut Connection, user: &str, item: &str, played: bool, now: i64) -> Result<Option<usize>> {
    let tx = c.transaction()?;
    let ids = playable_ids(&tx, item)?;
    if ids.is_empty() {
        let exists: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM items WHERE id = ?1)", [item], |r| r.get(0))?;
        return Ok(exists.then_some(0));
    }
    let mut changed = 0;
    for id in &ids {
        ensure_row(&tx, user, id, now)?;
        tx.execute(
            "DELETE FROM user_file_positions WHERE user_id = ?1 AND file_id IN (SELECT id FROM files WHERE item_id = ?2)",
            params![user, id],
        )?;
        changed += if played {
            tx.execute(
                "UPDATE user_data SET play_count = play_count + (played = 0), played = 1, position = 0, last_played_at = ?3, updated_at = ?3
                 WHERE user_id = ?1 AND item_id = ?2 AND (played = 0 OR position > 0)",
                params![user, id, now],
            )?
        } else {
            tx.execute(
                "UPDATE user_data SET played = 0, position = 0, updated_at = ?3 WHERE user_id = ?1 AND item_id = ?2 AND (played = 1 OR position > 0)",
                params![user, id, now],
            )?
        };
    }
    tx.commit()?;
    Ok(Some(changed))
}

/// Adds `user` (watch state) to items as the browse API shapes them (`id`, `kind`, `runtime`).
/// SQL (alias `e`): which episode an episode item is: the parts of a split episode are one.
pub const EPISODE_OF: &str = "CASE WHEN e.part_no IS NOT NULL THEN e.parent_id || ':' || e.episode_no ELSE e.id END";

pub fn attach(c: &Connection, user: &str, items: &mut [Value]) -> Result<()> {
    let vis = visible_sql("e");
    let mut one = c.prepare("SELECT position, played, play_count, last_played_at FROM user_data WHERE user_id = ?1 AND item_id = ?2")?;
    for v in items.iter_mut() {
        let id = v["id"].as_str().unwrap_or("").to_string();
        match v["kind"].as_str().unwrap_or("") {
            "movie" | "episode" => {
                let row: Option<(f64, bool, i64, Option<i64>)> = one
                    .query_row(params![user, id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                    .optional()?;
                let (position, played, count, last) = row.unwrap_or((0.0, false, 0, None));
                let pct = v["runtime"]
                    .as_f64()
                    .filter(|d| *d > 0.0 && position > 0.0)
                    .map(|d| (position / d * 1000.0).round() / 10.0);
                v["user"] = json!({"position": position, "played": played, "play_count": count, "last_played_at": last.map(rfc3339), "played_percentage": pct});
            }
            kind @ ("series" | "season") => {
                let col = if kind == "series" { "series_id" } else { "parent_id" };
                let (total, unplayed, last): (i64, i64, Option<i64>) = c.query_row(
                    &format!(
                        // Episodes (a split one is watched when all its parts are).
                        "SELECT COUNT(DISTINCT {EPISODE_OF}), COUNT(DISTINCT CASE WHEN COALESCE(u.played, 0) = 0 THEN {EPISODE_OF} END), MAX(u.last_played_at)
                         FROM items e LEFT JOIN user_data u ON u.item_id = e.id AND u.user_id = ?1
                         WHERE e.kind = 'episode' AND e.{col} = ?2 AND {vis}"
                    ),
                    params![user, id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;
                v["user"] = json!({"played": total > 0 && unplayed == 0, "unplayed_item_count": unplayed, "last_played_at": last.map(rfc3339), "position": 0.0});
            }
            _ => {}
        }
    }
    Ok(())
}

/// SQL: the item (alias `i`) is unplayed for `uid` (a series or season: has an unplayed episode).
pub fn unplayed_sql(uid: &str) -> String {
    format!(
        "(CASE WHEN i.kind IN ('movie', 'episode') THEN NOT EXISTS (SELECT 1 FROM user_data u WHERE u.user_id = '{uid}' AND u.item_id = i.id AND u.played = 1)
          ELSE EXISTS (SELECT 1 FROM items ue WHERE ue.kind = 'episode' AND (ue.series_id = i.id OR ue.parent_id = i.id) AND {}
                       AND NOT EXISTS (SELECT 1 FROM user_data u WHERE u.user_id = '{uid}' AND u.item_id = ue.id AND u.played = 1)) END)",
        visible_sql("ue")
    )
}

/// SQL: when the item (alias `i`; a series or season: any of its episodes) was last played.
pub fn last_played_sql(uid: &str) -> String {
    format!(
        "(SELECT MAX(u.last_played_at) FROM user_data u JOIN items le ON le.id = u.item_id
          WHERE u.user_id = '{uid}' AND (le.id = i.id OR le.series_id = i.id OR le.parent_id = i.id))"
    )
}

/// Titles with a resume point, latest first (`series`: only its episodes).
pub fn resume(c: &Connection, user: &str, limit: i64, series: Option<&str>) -> Result<Vec<String>> {
    let mut st = c.prepare(&format!(
        "SELECT i.id FROM user_data u JOIN items i ON i.id = u.item_id
         WHERE u.user_id = ?1 AND u.position > 0 AND i.kind IN ('movie', 'episode') AND {} AND (?2 IS NULL OR i.series_id = ?2)
         ORDER BY u.last_played_at DESC LIMIT ?3",
        visible_sql("i")
    ))?;
    let rows = st
        .query_map(params![user, series, limit], |r| r.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Next up: per series with a played episode (not a special), the first unplayed episode
/// after the most recently played one, in airing order (across seasons), specials left out.
/// Series ordered by when they were last played. A series whose next episode has a resume
/// point is left out unless `with_resumable`.
pub fn next_up(c: &Connection, user: &str, limit: usize, series: Option<&str>, with_resumable: bool) -> Result<Vec<String>> {
    let mut st = c.prepare(
        "SELECT e.series_id, COALESCE(e.season_no, 1), COALESCE(e.episode_end, e.episode_no, 0), COALESCE(e.part_no, 1)
         FROM user_data u JOIN items e ON e.id = u.item_id
         WHERE u.user_id = ?1 AND u.played = 1 AND e.kind = 'episode' AND e.series_id IS NOT NULL AND COALESCE(e.season_no, 1) > 0
           AND (?2 IS NULL OR e.series_id = ?2)
         ORDER BY u.last_played_at DESC, COALESCE(e.season_no, 1) DESC, e.episode_no DESC",
    )?;
    let played = st
        .query_map(params![user, series], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (sid, season, episode, part) in played {
        if out.len() >= limit {
            break;
        }
        if !seen.insert(sid.clone()) {
            continue;
        }
        if let Some((id, position)) = continuing::next_episode_after(c, user, &sid, season, episode, part, true)? {
            if position <= 0.0 || with_resumable {
                out.push(id);
            }
        }
    }
    Ok(out)
}

/// What a show's Play button plays: its latest resume point, else the next episode, else
/// (nothing played yet) the first one. `(episode, why)`.
pub fn continue_show(c: &Connection, user: &str, series: &str) -> Result<Option<(String, &'static str)>> {
    if let Some(id) = resume(c, user, 1, Some(series))?.into_iter().next() {
        return Ok(Some((id, "resume")));
    }
    if let Some(id) = next_up(c, user, 1, Some(series), true)?.into_iter().next() {
        return Ok(Some((id, "next")));
    }
    let any_played: bool = c.query_row(
        "SELECT EXISTS (SELECT 1 FROM user_data u JOIN items e ON e.id = u.item_id WHERE u.user_id = ?1 AND u.played = 1 AND e.series_id = ?2 AND COALESCE(e.season_no, 1) > 0)",
        params![user, series],
        |r| r.get(0),
    )?;
    if any_played {
        return Ok(None);
    }
    let first: Option<String> = c
        .query_row(
            &format!(
                "SELECT e.id FROM items e WHERE e.kind = 'episode' AND e.series_id = ?1 AND COALESCE(e.season_no, 1) > 0 AND {}
                 ORDER BY COALESCE(e.season_no, 1), e.episode_no, COALESCE(e.part_no, 1), e.id LIMIT 1",
                visible_sql("e")
            ),
            [series],
            |r| r.get(0),
        )
        .optional()?;
    Ok(first.map(|f| (f, "first")))
}

/// Watch state of items that were merged into others moves with them.
pub fn follow_redirect(c: &Connection, old: &str, new: &str) -> Result<()> {
    c.execute("UPDATE OR IGNORE user_data SET item_id = ?2 WHERE item_id = ?1", params![old, new])?;
    c.execute("UPDATE play_sessions SET item_id = ?2 WHERE item_id = ?1", params![old, new])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    #[test]
    fn thresholds() {
        let d = Some(3000.0);
        assert_eq!(evaluate(0.0, d, None, false), Outcome::Clear);
        assert_eq!(evaluate(149.0, d, None, false), Outcome::Clear, "under 5 %");
        assert_eq!(evaluate(151.0, d, None, false), Outcome::Resume(151.0));
        assert_eq!(
            evaluate(2700.0, d, None, false),
            Outcome::Resume(2700.0),
            "exactly 90 % still resumes"
        );
        assert_eq!(evaluate(2701.0, d, None, false), Outcome::Played, "over 90 %");
        assert_eq!(evaluate(2850.0, d, None, false), Outcome::Played, "95 %");
        // Inside the final credits, before 90 %.
        assert_eq!(evaluate(2400.0, d, Some(2350.0), false), Outcome::Played);
        assert_eq!(evaluate(2300.0, d, Some(2350.0), false), Outcome::Resume(2300.0));
        // Under 5 minutes: no resume point; stopped past 5 %, it's played.
        assert_eq!(evaluate(100.0, Some(240.0), None, false), Outcome::Played);
        assert_eq!(evaluate(5.0, Some(240.0), None, false), Outcome::Clear);
        // No known length.
        assert_eq!(evaluate(10.0, None, None, false), Outcome::Played);
        // A movie with a credits chapter: not played at 90 % while its story runs (Avengers: Infinity
        // War, credits at 8251 of 8979, 91.9 %), played from the credits start.
        let iw = Some(8978.528);
        assert_eq!(evaluate(8200.0, iw, Some(8251.285), true), Outcome::Resume(8200.0));
        assert_eq!(evaluate(8251.285, iw, Some(8251.285), true), Outcome::Played);
        assert_eq!(
            evaluate(8200.0, iw, Some(8251.285), false),
            Outcome::Played,
            "an episode: over 90 % as before"
        );
        assert_eq!(
            evaluate(8200.0, iw, None, true),
            Outcome::Played,
            "no credits chapter: over 90 % as before"
        );
    }

    #[test]
    fn reported_credits() {
        // Thor: Ragnarok, no credits chapter: IntroDB's credits at 7409 of 7831, reported by the
        // app. Played from there, not at 90 % (7048).
        let d = Some(7831.0);
        let c = report_credits(Some(7409.0), None, d);
        assert_eq!(c, Some(7409.0));
        assert_eq!(evaluate(7100.0, d, c, true), Outcome::Resume(7100.0));
        assert_eq!(evaluate(7409.0, d, c, true), Outcome::Played);
        // Older apps send none: over 90 % as before.
        assert_eq!(evaluate(7100.0, d, report_credits(None, None, d), true), Outcome::Played);
        // Guardians of the Galaxy: chapter 6796.6, the app reports IntroDB's 6852 (a chapter never
        // starts before it): not played in between.
        let g = Some(7268.64);
        let c = report_credits(Some(6852.0), Some(6796.622), g);
        assert_eq!(c, Some(6852.0));
        assert_eq!(evaluate(6800.0, g, c, true), Outcome::Resume(6800.0));
        assert_eq!(evaluate(6852.0, g, c, true), Outcome::Played);
        // With a chapter too, the later start.
        assert_eq!(report_credits(Some(7409.0), Some(7300.0), d), Some(7409.0));
        assert_eq!(report_credits(Some(7300.0), Some(7409.0), d), Some(7409.0));
        // Not in the second half, past the end, or not a number: ignored.
        assert_eq!(report_credits(Some(300.0), None, d), None);
        assert_eq!(report_credits(Some(300.0), Some(7409.0), d), Some(7409.0));
        assert_eq!(report_credits(Some(7900.0), None, d), None);
        assert_eq!(report_credits(Some(f64::NAN), None, d), None);
        assert_eq!(report_credits(Some(-1.0), None, None), None);
        assert_eq!(report_credits(Some(50.0), None, None), Some(50.0), "no runtime: taken as it is");
    }

    #[test]
    fn credits_chapter() {
        let ch = serde_json::json!([
            {"start_time": "0.000", "tags": {"title": "Intro"}},
            {"start_time": "60.000", "tags": {"title": "Chapter 2"}},
            {"start_time": "100.000", "tags": {"title": "Credits"}},
            {"start_time": "2500.000", "tags": {"title": "End Credits"}},
        ]);
        assert_eq!(final_credits(ch.as_array().unwrap(), 2700.0, None), Some(2500.0));
        // An opening "Credits" chapter early on isn't the final credits.
        assert_eq!(final_credits(&ch.as_array().unwrap()[..3], 2700.0, None), None);
        // With a language tag (Guardians of the Galaxy).
        let gotg = serde_json::json!([{"start_time": "0.000", "tags": {"title": "en:Chapter 1"}}, {"start_time": "6852.000", "tags": {"title": "en:End Credits"}}]);
        assert_eq!(final_credits(gotg.as_array().unwrap(), 7269.0, None), Some(6852.0));
    }

    #[tokio::test]
    async fn a_recording_with_a_start_time_is_played_at_its_credits_chapter() {
        // A 45-minute TV recording remuxed into Matroska with its broadcast clock (it starts at
        // 1002.5 s; Matroska's duration is its last timestamp) and chapters on that clock, the
        // end credits 2550 s in. Players count the start as 0, so reports come on that
        // timeline. Before 0.31.11 the credits chapter was at 3552.5 and the length 3702.5: the
        // film was never played (a known credits start replaces the 90 % mark).
        let db = Db::memory().unwrap();
        db.call(|c| {
            fixture(c);
            let probe = json!({"format": {"format_name": "matroska,webm", "start_time": "1002.500000", "duration": "3702.500000"}, "streams": []});
            let chapters = json!([
                {"start_time": "1002.500000", "tags": {"title": "Chapter 1"}},
                {"start_time": "3552.500000", "tags": {"title": "End Credits"}},
            ]);
            c.execute("UPDATE files SET duration = 3702.5 WHERE id = 'film'", [])?;
            c.execute(
                "INSERT INTO files_probe(path, size, mtime, probe, chapters, probed_at) VALUES ('/m/film', 1, 1, ?1, ?2, 0)",
                params![probe.to_string(), chapters.to_string()],
            )?;
            assert_eq!(item_duration(c, "film")?, Some(2700.0));
            assert_eq!(credits_start(c, "film")?, Some(2550.0));
            let credits = report_credits(None, credits_start(c, "film")?, item_duration(c, "film")?);
            assert_eq!(record_position(c, "u1", "film", 2560.0, None, credits, None, 100)?, Outcome::Played);
            // A file starting at 0 is read as it is.
            let probe = json!({"format": {"format_name": "matroska,webm", "start_time": "0.000000", "duration": "2700.0"}, "streams": []});
            let chapters = json!([{"start_time": "0.000000", "tags": {"title": "Chapter 1"}}, {"start_time": "2550.000000", "tags": {"title": "End Credits"}}]);
            c.execute("UPDATE files SET duration = 2700 WHERE id = 'e11'", [])?;
            c.execute(
                "INSERT INTO files_probe(path, size, mtime, probe, chapters, probed_at) VALUES ('/m/e11', 1, 1, ?1, ?2, 0)",
                params![probe.to_string(), chapters.to_string()],
            )?;
            assert_eq!((item_duration(c, "e11")?, credits_start(c, "e11")?), (Some(2700.0), Some(2550.0)));
            Ok(())
        })
        .await
        .unwrap();
    }

    /// A series: S00E01 (special), S01E01-E03, S02E01-E02; a movie.
    fn fixture(c: &Connection) {
        let now = 1_700_000_000;
        let item = |id: &str, kind: &str, parent: Option<&str>, series: Option<&str>, s: Option<i64>, e: Option<i64>| {
            c.execute(
                "INSERT INTO items(id, kind, library_id, key, parent_id, series_id, season_no, episode_no, title, identify_state, added_at, updated_at)
                 VALUES (?1, ?2, 'lib', ?1, ?3, ?4, ?5, ?6, ?1, 'matched', ?7, ?7)",
                params![id, kind, parent, series, s, e, now],
            )
            .unwrap();
            if kind == "movie" || kind == "episode" {
                c.execute(
                    "INSERT INTO files(id, library_id, path, size, mtime, role, item_id, first_seen, last_seen, stable_since, duration)
                     VALUES (?1, 'lib', '/m/' || ?1, 1, 1, 'main', ?1, ?2, ?2, ?2, 2700)",
                    params![id, now],
                )
                .unwrap();
            }
        };
        item("show", "series", None, Some("show"), None, None);
        item("s0", "season", Some("show"), Some("show"), Some(0), None);
        item("s1", "season", Some("show"), Some("show"), Some(1), None);
        item("s2", "season", Some("show"), Some("show"), Some(2), None);
        item("sp1", "episode", Some("s0"), Some("show"), Some(0), Some(1));
        item("e11", "episode", Some("s1"), Some("show"), Some(1), Some(1));
        item("e12", "episode", Some("s1"), Some("show"), Some(1), Some(2));
        item("e13", "episode", Some("s1"), Some("show"), Some(1), Some(3));
        item("e21", "episode", Some("s2"), Some("show"), Some(2), Some(1));
        item("e22", "episode", Some("s2"), Some("show"), Some(2), Some(2));
        item("film", "movie", None, None, None, None);
        c.execute(
            "INSERT INTO users(id, name, password_hash, created_at) VALUES ('u1', 'sam', 'x', 0)",
            [],
        )
        .unwrap();
    }

    #[tokio::test]
    async fn parts_of_an_episode() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            fixture(c);
            // S01E02 is split over two files: e12 is Part 1, e12b Part 2.
            c.execute(
                "INSERT INTO items(id, kind, library_id, key, parent_id, series_id, season_no, episode_no, part_no, title, identify_state, added_at, updated_at)
                 VALUES ('e12b', 'episode', 'lib', 'e12:p2', 's1', 'show', 1, 2, 2, 'e12', 'matched', 0, 0)",
                [],
            )?;
            c.execute("INSERT INTO files(id, library_id, path, size, mtime, role, item_id, first_seen, last_seen, stable_since, duration) VALUES ('e12b', 'lib', '/m/e12b', 1, 1, 'main', 'e12b', 0, 0, 0, 1300)", [])?;
            c.execute("UPDATE items SET part_no = 1 WHERE id = 'e12'", [])?;
            let u = "u1";
            let season = |c: &Connection| -> Result<Value> {
                let mut v = vec![json!({"id": "s1", "kind": "season"})];
                attach(c, u, &mut v)?;
                Ok(v[0]["user"].clone())
            };
            assert_eq!(season(c)?["unplayed_item_count"], 3, "three episodes, one of them in two parts");
            record_position(c, u, "e11", 2600.0, Some(2700.0), None, None, 110)?;
            assert_eq!(next_up(c, u, 20, None, false)?, vec!["e12"]);
            // Part 1 → Part 2 (same episode number), then E03.
            record_position(c, u, "e12", 1290.0, Some(1300.0), None, None, 120)?;
            assert_eq!(next_up(c, u, 20, None, false)?, vec!["e12b"]);
            assert_eq!(season(c)?["unplayed_item_count"], 2, "E02 isn't watched until both parts are");
            record_position(c, u, "e12b", 1290.0, Some(1300.0), None, None, 130)?;
            assert_eq!(next_up(c, u, 20, None, false)?, vec!["e13"]);
            assert_eq!(season(c)?["unplayed_item_count"], 1);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn next_up_and_resume() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            fixture(c);
            let u = "u1";
            // Nothing played: no next up; the show's Play button starts at S01E01 (not the special).
            assert!(next_up(c, u, 20, None, false)?.is_empty());
            assert_eq!(continue_show(c, u, "show")?, Some(("e11".into(), "first")));
            // A special played doesn't make next up.
            record_position(c, u, "sp1", 2600.0, Some(2700.0), None, None, 100)?;
            assert!(next_up(c, u, 20, None, false)?.is_empty());
            // S01E01 played → S01E02.
            record_position(c, u, "e11", 2600.0, Some(2700.0), None, None, 110)?;
            assert_eq!(next_up(c, u, 20, None, false)?, vec!["e12"]);
            // S01E03 watched before S01E02 (most recent: E03) → across the season boundary: S02E01.
            record_position(c, u, "e13", 2600.0, Some(2700.0), None, None, 120)?;
            assert_eq!(next_up(c, u, 20, None, false)?, vec!["e21"]);
            // S02E01 in progress: in resume, not in next up (unless resumable is wanted).
            record_position(c, u, "e21", 1000.0, Some(2700.0), None, None, 130)?;
            assert!(next_up(c, u, 20, None, false)?.is_empty());
            assert_eq!(next_up(c, u, 20, None, true)?, vec!["e21"]);
            assert_eq!(resume(c, u, 20, None)?, vec!["e21"]);
            assert_eq!(continue_show(c, u, "show")?, Some(("e21".into(), "resume")));
            // A movie in progress comes first in resume (latest).
            record_position(c, u, "film", 900.0, Some(2700.0), None, None, 140)?;
            assert_eq!(resume(c, u, 20, None)?, vec!["film", "e21"]);
            // Finish S02E01 → S02E02; the last episode → nothing left.
            record_position(c, u, "e21", 2650.0, Some(2700.0), None, None, 150)?;
            assert_eq!(next_up(c, u, 20, None, false)?, vec!["e22"]);
            assert_eq!(continue_show(c, u, "show")?, Some(("e22".into(), "next")));
            record_position(c, u, "e22", 2650.0, Some(2700.0), None, None, 160)?;
            assert!(next_up(c, u, 20, None, false)?.is_empty());
            assert_eq!(continue_show(c, u, "show")?, None);
            // Rewatching S01E01 makes it the anchor again: the next unplayed after it is S01E02.
            record_position(c, u, "e11", 2650.0, Some(2700.0), None, None, 170)?;
            assert_eq!(next_up(c, u, 20, None, false)?, vec!["e12"]);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn cascade_and_counts() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            fixture(c);
            let u = "u1";
            // Season 1 played: its three episodes.
            assert_eq!(set_played(c, u, "s1", true, 10)?, Some(3));
            let mut v = vec![
                json!({"id": "s1", "kind": "season"}),
                json!({"id": "show", "kind": "series"}),
                json!({"id": "e12", "kind": "episode", "runtime": 2700.0}),
            ];
            attach(c, u, &mut v)?;
            assert_eq!(v[0]["user"]["played"], true);
            assert_eq!(v[0]["user"]["unplayed_item_count"], 0);
            assert_eq!(v[1]["user"]["unplayed_item_count"], 3, "special + season 2");
            assert_eq!(v[2]["user"]["played"], true);
            assert_eq!(v[2]["user"]["play_count"], 1);
            // The whole series, then unplayed.
            assert_eq!(set_played(c, u, "show", true, 20)?, Some(3));
            assert_eq!(set_played(c, u, "show", false, 30)?, Some(6));
            assert_eq!(set_played(c, u, "nope", true, 30)?, None);
            // A viewing counts once per session, and a rewatch counts again.
            c.execute(
                "INSERT INTO play_sessions(id, user_id, item_id, started_at) VALUES ('sid1', 'u1', 'film', 0)",
                [],
            )?;
            c.execute(
                "INSERT INTO play_sessions(id, user_id, item_id, started_at) VALUES ('sid2', 'u1', 'film', 0)",
                [],
            )?;
            record_start(c, u, "film", 40)?;
            assert_eq!(
                record_position(c, u, "film", 2600.0, Some(2700.0), None, Some("sid1"), 41)?,
                Outcome::Played
            );
            assert_eq!(
                record_position(c, u, "film", 2650.0, Some(2700.0), None, Some("sid1"), 42)?,
                Outcome::Played
            );
            // Rewatch: played stays, a resume point is kept.
            assert_eq!(
                record_position(c, u, "film", 600.0, Some(2700.0), None, Some("sid2"), 50)?,
                Outcome::Resume(600.0)
            );
            let (pos, played, count): (f64, bool, i64) = c.query_row(
                "SELECT position, played, play_count FROM user_data WHERE item_id = 'film'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            assert_eq!((pos, played, count), (600.0, true, 1));
            record_position(c, u, "film", 2690.0, Some(2700.0), None, Some("sid2"), 60)?;
            let count: i64 = c.query_row("SELECT play_count FROM user_data WHERE item_id = 'film'", [], |r| r.get(0))?;
            assert_eq!(count, 2);
            // Filters for the browse API.
            let n: i64 = c.query_row(
                &format!(
                    "SELECT COUNT(*) FROM items i WHERE i.kind IN ('movie', 'series') AND {}",
                    unplayed_sql(u)
                ),
                [],
                |r| r.get(0),
            )?;
            assert_eq!(n, 1, "the series has unplayed episodes; the film is played");
            let last: Option<i64> = c.query_row(
                &format!("SELECT {} FROM items i WHERE i.id = 'film'", last_played_sql(u)),
                [],
                |r| r.get(0),
            )?;
            assert_eq!(last, Some(60));
            assert!(sql_id("01abc").is_some());
            assert!(sql_id("x' OR 1=1 --").is_none());
            Ok(())
        })
        .await
        .unwrap();
    }
}
