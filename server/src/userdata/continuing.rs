//! Continue Watching: one row of what to watch next, the same on every device of a user, latest
//! activity first. Built from the server's watch state, so playback on one device shows
//! up on all the others.
//!
//! The rules:
//! - Watching is grouped into **threads**: a show, a TMDB collection of films, or a single film
//!   outside any collection. Each thread gives at most one card, decided by its latest activity
//!   (a progress report, a stop, or a mark as watched) in the last [`MAX_AGE_DAYS`] days.
//! - **Part-way through** (a resume point): that title, reason `resume`.
//! - **A finished episode** (watched, or stopped in its credits): the episode right after it in
//!   airing order, across seasons, never into specials, whether or not it was watched before
//!   (the same rule as Next Episode in the player, so a rewatch carries on), reason
//!   `next_episode`. None yet: the show drops out, and comes back when the next one is added.
//!   A finished special doesn't count; the episode before it does.
//! - **A finished film** in a collection: the film of that collection released right after it
//!   that the library has, whether or not it was watched, reason `collection_next`. None: the
//!   thread drops out.
//! - A title started but stopped before 5 % has no resume point and isn't counted.
//! - **Removed** (Remove from Continue Watching, on any device): the thread stays out until it
//!   is watched again.
//! - Cards are ordered by the time of the activity that made them (for a next episode: when the
//!   previous one was finished).

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::library::store::visible_sql;

/// Activity older than this many days ages out of the row.
pub const MAX_AGE_DAYS: i64 = 60;

/// Why a card is in the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Part-way through: play from the resume point.
    Resume,
    /// The episode after the last one finished.
    NextEpisode,
    /// The next film of the collection of the last one finished.
    CollectionNext,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Resume => "resume",
            Reason::NextEpisode => "next_episode",
            Reason::CollectionNext => "collection_next",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    /// The movie or episode to play.
    pub item: String,
    pub reason: Reason,
    /// When the activity that made the card happened (Unix seconds).
    pub at: i64,
    /// The title that activity was on (the card's own item for `resume`).
    pub from: String,
}

/// One watched title (a `user_data` row).
struct Activity {
    id: String,
    kind: String,
    series: Option<String>,
    season: i64,
    episode: i64,
    part: i64,
    collection: Option<i64>,
    played: bool,
    position: f64,
    at: i64,
    visible: bool,
}

/// The thread a title belongs to: its show, its collection, or itself.
fn thread_key(kind: &str, id: &str, series: Option<&str>, collection: Option<i64>) -> String {
    match (kind, series, collection) {
        ("episode", Some(s), _) => s.to_string(),
        ("movie", _, Some(c)) => format!("collection:{c}"),
        _ => id.to_string(),
    }
}

/// The thread of an item (a series: itself), for Remove from Continue Watching. None: no such item.
pub fn thread_of(c: &Connection, item: &str) -> Result<Option<String>> {
    Ok(
        c.query_row("SELECT kind, series_id, collection_id FROM items WHERE id = ?1", [item], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<i64>>(2)?))
        })
        .optional()?
        .map(|(kind, series, col)| match kind.as_str() {
            "series" => item.to_string(),
            "season" => series.unwrap_or_else(|| item.to_string()),
            _ => thread_key(&kind, item, series.as_deref(), col),
        }),
    )
}

/// Remove from Continue Watching: the item's thread stays out of the row until it is watched
/// again (activity after `now`). Returns the thread, or None for no such item.
pub fn hide(c: &Connection, user: &str, item: &str, now: i64) -> Result<Option<String>> {
    let Some(key) = thread_of(c, item)? else { return Ok(None) };
    c.execute(
        "INSERT INTO continue_hidden(user_id, thread, hidden_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(user_id, thread) DO UPDATE SET hidden_at = excluded.hidden_at",
        params![user, key, now],
    )?;
    Ok(Some(key))
}

/// The episode after (season, episode, part) of `series` in airing order, specials left out:
/// `(id, resume position)`. With `unwatched_only`, the first one not yet watched (Next Up for
/// older apps); without, the very next one (Continue Watching, as Next Episode in the player).
pub fn next_episode_after(
    c: &Connection,
    user: &str,
    series: &str,
    season: i64,
    episode: i64,
    part: i64,
    unwatched_only: bool,
) -> Result<Option<(String, f64)>> {
    let mut st = c.prepare_cached(&format!(
        "SELECT e.id, COALESCE(u.position, 0) FROM items e LEFT JOIN user_data u ON u.item_id = e.id AND u.user_id = ?1
         WHERE e.kind = 'episode' AND e.series_id = ?2 AND COALESCE(e.season_no, 1) > 0 AND {}
           AND (COALESCE(e.season_no, 1) > ?3 OR (COALESCE(e.season_no, 1) = ?3 AND COALESCE(e.episode_no, 0) > ?4)
                OR (COALESCE(e.season_no, 1) = ?3 AND COALESCE(e.episode_no, 0) = ?4 AND COALESCE(e.part_no, 1) > ?5))
           AND (?6 = 0 OR COALESCE(u.played, 0) = 0)
         ORDER BY COALESCE(e.season_no, 1), e.episode_no, COALESCE(e.part_no, 1), e.id LIMIT 1",
        visible_sql("e")
    ))?;
    Ok(st
        .query_row(params![user, series, season, episode, part, unwatched_only], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?)
}

/// The film of `collection` that comes right after `after` in release order (release date or
/// year, then title, then id, so films released on the same day follow each other too), visible
/// in the library, watched or not: `(id, resume position)`.
fn next_in_collection(c: &Connection, user: &str, collection: i64, after: &str) -> Result<Option<(String, f64)>> {
    let mut st = c.prepare_cached(&format!(
        "SELECT m.id, COALESCE(u.position, 0) FROM items m LEFT JOIN user_data u ON u.item_id = m.id AND u.user_id = ?1
         WHERE m.kind = 'movie' AND m.collection_id = ?2 AND {}
           AND (COALESCE(m.premiere_date, CAST(m.year AS TEXT)), COALESCE(m.sort_title, ''), m.id)
             > (SELECT COALESCE(premiere_date, CAST(year AS TEXT)), COALESCE(sort_title, ''), id FROM items WHERE id = ?3)
         ORDER BY COALESCE(m.premiere_date, CAST(m.year AS TEXT)), COALESCE(m.sort_title, ''), m.id LIMIT 1",
        visible_sql("m")
    ))?;
    Ok(st
        .query_row(params![user, collection, after], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?)
}

/// The row for `user` at `now`: at most `limit` cards, latest activity first.
pub fn continue_watching(c: &Connection, user: &str, limit: usize, now: i64) -> Result<Vec<Card>> {
    let since = now - MAX_AGE_DAYS * 86_400;
    // Several titles marked at the same moment (a season, a whole collection): the last of them
    // in airing or release order speaks for its thread.
    let mut st = c.prepare(&format!(
        "SELECT i.id, i.kind, i.series_id, COALESCE(i.season_no, 1), COALESCE(i.episode_end, i.episode_no, 0), COALESCE(i.part_no, 1),
                i.collection_id, u.played, u.position, u.last_played_at, {}
         FROM user_data u JOIN items i ON i.id = u.item_id
         WHERE u.user_id = ?1 AND i.kind IN ('movie', 'episode') AND u.last_played_at >= ?2
         ORDER BY u.last_played_at DESC, COALESCE(i.season_no, 1) DESC, i.episode_no DESC, COALESCE(i.part_no, 1) DESC,
                  COALESCE(i.premiere_date, CAST(i.year AS TEXT)) DESC, COALESCE(i.sort_title, '') DESC, i.id DESC",
        visible_sql("i")
    ))?;
    let rows = st
        .query_map(params![user, since], |r| {
            Ok(Activity {
                id: r.get(0)?,
                kind: r.get(1)?,
                series: r.get(2)?,
                season: r.get(3)?,
                episode: r.get(4)?,
                part: r.get(5)?,
                collection: r.get(6)?,
                played: r.get(7)?,
                position: r.get(8)?,
                at: r.get(9)?,
                visible: r.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let hidden: HashMap<String, i64> = {
        let mut st = c.prepare("SELECT thread, hidden_at FROM continue_hidden WHERE user_id = ?1")?;
        let rows = st.query_map([user], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        rows
    };
    let mut decided = HashSet::new();
    let mut items = HashSet::new();
    let mut out = Vec::new();
    for a in rows {
        if out.len() >= limit {
            break;
        }
        let key = thread_key(&a.kind, &a.id, a.series.as_deref(), a.collection);
        if decided.contains(&key) {
            continue;
        }
        let in_progress = a.position > 0.0;
        let finished = a.played && !in_progress;
        // Started but stopped before 5 % (or marked unwatched): nothing to go on.
        if !in_progress && !finished {
            continue;
        }
        let episode = a.kind == "episode";
        // A finished special says nothing about where the show is: the episode before it does.
        if finished && episode && a.season == 0 {
            continue;
        }
        if in_progress && !a.visible {
            continue;
        }
        decided.insert(key.clone());
        if hidden.get(&key).is_some_and(|h| a.at <= *h) {
            continue;
        }
        let card = if in_progress {
            Some(Card {
                item: a.id.clone(),
                reason: Reason::Resume,
                at: a.at,
                from: a.id,
            })
        } else if episode {
            match a.series.as_deref() {
                Some(s) => next_episode_after(c, user, s, a.season, a.episode, a.part, false)?.map(|(id, pos)| Card {
                    item: id,
                    reason: if pos > 0.0 { Reason::Resume } else { Reason::NextEpisode },
                    at: a.at,
                    from: a.id.clone(),
                }),
                None => None,
            }
        } else {
            match a.collection {
                Some(col) => next_in_collection(c, user, col, &a.id)?.map(|(id, pos)| Card {
                    item: id,
                    reason: if pos > 0.0 { Reason::Resume } else { Reason::CollectionNext },
                    at: a.at,
                    from: a.id.clone(),
                }),
                _ => None,
            }
        };
        if let Some(card) = card {
            if items.insert(card.item.clone()) {
                out.push(card);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::Db, userdata::record_position};

    const DAY: i64 = 86_400;
    const T0: i64 = 1_700_000_000;

    /// A show (S00E01 special, S01E01-E03, S02E01-E02, S03E01 coming later), a collection of three
    /// films (1999, 2003, 2005-05-01 as "2005-05-01"), a lone film.
    fn fixture(c: &Connection) {
        let item = |id: &str, kind: &str, parent: Option<&str>, series: Option<&str>, s: Option<i64>, e: Option<i64>| {
            c.execute(
                "INSERT INTO items(id, kind, library_id, key, parent_id, series_id, season_no, episode_no, title, sort_title, identify_state, added_at, updated_at)
                 VALUES (?1, ?2, 'lib', ?1, ?3, ?4, ?5, ?6, ?1, ?1, 'matched', 0, 0)",
                params![id, kind, parent, series, s, e],
            )
            .unwrap();
            if kind == "movie" || kind == "episode" {
                c.execute(
                    "INSERT INTO files(id, library_id, path, size, mtime, role, item_id, first_seen, last_seen, stable_since, duration)
                     VALUES (?1, 'lib', '/m/' || ?1, 1, 1, 'main', ?1, 0, 0, 0, 2700)",
                    [id],
                )
                .unwrap();
            }
        };
        item("show", "series", None, Some("show"), None, None);
        item("s0", "season", Some("show"), Some("show"), Some(0), None);
        item("s1", "season", Some("show"), Some("show"), Some(1), None);
        item("s2", "season", Some("show"), Some("show"), Some(2), None);
        item("sp1", "episode", Some("s0"), Some("show"), Some(0), Some(1));
        for (id, s, e) in [("e11", 1, 1), ("e12", 1, 2), ("e13", 1, 3), ("e21", 2, 1), ("e22", 2, 2)] {
            item(
                id,
                "episode",
                Some(if s == 1 { "s1" } else { "s2" }),
                Some("show"),
                Some(s),
                Some(e),
            );
        }
        for (id, date) in [("m1", "1999-03-31"), ("m2", "2003-05-15"), ("m3", "2005-05-01")] {
            item(id, "movie", None, None, None, None);
            c.execute(
                "UPDATE items SET collection_id = 7, premiere_date = ?2 WHERE id = ?1",
                params![id, date],
            )
            .unwrap();
        }
        item("lone", "movie", None, None, None, None);
        c.execute(
            "INSERT INTO users(id, name, password_hash, created_at) VALUES ('u1', 'sam', 'x', 0)",
            [],
        )
        .unwrap();
    }

    fn row(c: &Connection, now: i64) -> Vec<(String, &'static str)> {
        continue_watching(c, "u1", 20, now)
            .unwrap()
            .into_iter()
            .map(|c| (c.item, c.reason.as_str()))
            .collect()
    }

    fn watch(c: &Connection, id: &str, at: i64) {
        record_position(c, "u1", id, 2650.0, Some(2700.0), None, None, at).unwrap();
    }

    fn part(c: &Connection, id: &str, at: i64) {
        record_position(c, "u1", id, 1000.0, Some(2700.0), None, None, at).unwrap();
    }

    #[tokio::test]
    async fn episodes() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            fixture(c);
            assert!(row(c, T0).is_empty());
            // Part-way through S01E01: resume it.
            part(c, "e11", T0);
            assert_eq!(row(c, T0), vec![("e11".into(), "resume")]);
            // Finished: the next episode, one card for the show.
            watch(c, "e11", T0 + 10);
            assert_eq!(row(c, T0 + 10), vec![("e12".into(), "next_episode")]);
            // A special watched later doesn't move the show: still S01E02.
            watch(c, "sp1", T0 + 20);
            assert_eq!(row(c, T0 + 20), vec![("e12".into(), "next_episode")]);
            // The last of a season: the next season's first.
            watch(c, "e12", T0 + 30);
            watch(c, "e13", T0 + 40);
            assert_eq!(row(c, T0 + 40), vec![("e21".into(), "next_episode")]);
            // The last episode there is: the show drops out...
            watch(c, "e21", T0 + 50);
            watch(c, "e22", T0 + 60);
            assert!(row(c, T0 + 60).is_empty());
            // ...and comes back when the next one is added.
            c.execute("INSERT INTO items(id, kind, library_id, key, parent_id, series_id, season_no, episode_no, title, identify_state, added_at, updated_at) VALUES ('e31', 'episode', 'lib', 'e31', 'show', 'show', 3, 1, 'e31', 'matched', 0, 0)", [])?;
            c.execute("INSERT INTO files(id, library_id, path, size, mtime, role, item_id, first_seen, last_seen, stable_since, duration) VALUES ('e31', 'lib', '/m/e31', 1, 1, 'main', 'e31', 0, 0, 0, 2700)", [])?;
            assert_eq!(row(c, T0 + 70), vec![("e31".into(), "next_episode")]);
            // Aged out after 60 days.
            assert!(row(c, T0 + 60 + MAX_AGE_DAYS * DAY + 1).is_empty());
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn films_order_and_removal() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            fixture(c);
            // A lone film finished: nothing.
            watch(c, "lone", T0);
            assert!(row(c, T0).is_empty());
            // Part-way through: resume.
            part(c, "lone", T0 + 5);
            assert_eq!(row(c, T0 + 5), vec![("lone".into(), "resume")]);
            // The first film of the collection finished: the second.
            watch(c, "m1", T0 + 10);
            assert_eq!(row(c, T0 + 10), vec![("m2".into(), "collection_next"), ("lone".into(), "resume")]);
            // The second watched too: the third. Then all: the collection drops out.
            watch(c, "m2", T0 + 20);
            assert_eq!(row(c, T0 + 20)[0], ("m3".into(), "collection_next"));
            watch(c, "m3", T0 + 30);
            assert_eq!(row(c, T0 + 30), vec![("lone".into(), "resume")]);
            // A show in between, ordered by activity.
            part(c, "e11", T0 + 40);
            assert_eq!(row(c, T0 + 40), vec![("e11".into(), "resume"), ("lone".into(), "resume")]);
            // Removed (from any device): out until watched again.
            assert_eq!(hide(c, "u1", "e11", T0 + 41)?, Some("show".into()));
            assert_eq!(row(c, T0 + 41), vec![("lone".into(), "resume")]);
            watch(c, "e11", T0 + 50);
            assert_eq!(row(c, T0 + 50), vec![("e12".into(), "next_episode"), ("lone".into(), "resume")]);
            // Removing a film's card hides its collection's thread; a lone film its own.
            hide(c, "u1", "lone", T0 + 51)?;
            assert_eq!(row(c, T0 + 51), vec![("e12".into(), "next_episode")]);
            assert_eq!(hide(c, "u1", "nope", T0)?, None);
            assert_eq!(thread_of(c, "m2")?, Some("collection:7".into()));
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn marks_and_rewatches() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            fixture(c);
            // Marked watched from a card's menu: the next episode.
            crate::userdata::set_played(c, "u1", "e12", true, T0)?;
            assert_eq!(row(c, T0), vec![("e13".into(), "next_episode")]);
            // A season marked watched: after its last episode.
            crate::userdata::set_played(c, "u1", "s1", true, T0 + 10)?;
            assert_eq!(row(c, T0 + 10), vec![("e21".into(), "next_episode")]);
            // Started and stopped under 5 %: doesn't count; the show stays on S02E01.
            record_position(c, "u1", "e22", 20.0, Some(2700.0), None, None, T0 + 20)?;
            assert_eq!(row(c, T0 + 20), vec![("e21".into(), "next_episode")]);
            // Rewatching a finished film part-way: resume it.
            watch(c, "m2", T0 + 30);
            part(c, "m2", T0 + 40);
            assert_eq!(row(c, T0 + 40)[0], ("m2".into(), "resume"));
            // The next film of a collection part-way through: resumed, not suggested afresh.
            watch(c, "m1", T0 + 60);
            assert_eq!(
                row(c, T0 + 60)[0],
                ("m2".into(), "resume"),
                "after m1 comes m2, part-way through a rewatch"
            );
            Ok(())
        })
        .await
        .unwrap();
    }

    /// Rewatching: the card is the very next episode or film, watched before or not, as Next
    /// Episode in the player. Next Up for older apps stays unwatched-only.
    #[tokio::test]
    async fn rewatches_carry_on() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            fixture(c);
            // Everything watched once: the show and the collection are done, nothing to offer.
            crate::userdata::set_played(c, "u1", "show", true, T0)?;
            for m in ["m1", "m2", "m3"] {
                crate::userdata::set_played(c, "u1", m, true, T0)?;
            }
            assert!(row(c, T0).is_empty());
            // A rewatch of S01E01 finished: S01E02, though it was watched before.
            watch(c, "e11", T0 + 10);
            assert_eq!(row(c, T0 + 10), vec![("e12".into(), "next_episode")]);
            assert!(
                crate::userdata::next_up(c, "u1", 20, None, true)?.is_empty(),
                "Next Up stays unwatched-only"
            );
            // Across seasons the same way, and the last episode drops the show again.
            watch(c, "e13", T0 + 20);
            assert_eq!(row(c, T0 + 20), vec![("e21".into(), "next_episode")]);
            watch(c, "e22", T0 + 30);
            assert!(row(c, T0 + 30).is_empty());
            // Films: a rewatch of the first gives the second; the last drops the collection.
            watch(c, "m1", T0 + 40);
            assert_eq!(row(c, T0 + 40), vec![("m2".into(), "collection_next")]);
            watch(c, "m3", T0 + 50);
            assert!(row(c, T0 + 50).is_empty());
            Ok(())
        })
        .await
        .unwrap();
    }

    /// Only the episode after it was watched before: that one, not the first unwatched one.
    #[tokio::test]
    async fn rewatch_does_not_skip_a_watched_episode() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            fixture(c);
            watch(c, "e12", T0);
            watch(c, "e11", T0 + 10);
            assert_eq!(row(c, T0 + 10), vec![("e12".into(), "next_episode")]);
            assert_eq!(
                crate::userdata::next_up(c, "u1", 20, None, false)?,
                vec!["e13"],
                "Next Up skips what was watched"
            );
            // Films released the same day follow each other too.
            c.execute(
                "UPDATE items SET premiere_date = '2003-05-15', sort_title = 'b' WHERE id = 'm3'",
                [],
            )?;
            c.execute("UPDATE items SET sort_title = 'a' WHERE id = 'm2'", [])?;
            watch(c, "m3", T0 + 15);
            watch(c, "m2", T0 + 20);
            assert_eq!(row(c, T0 + 20)[0], ("m3".into(), "collection_next"));
            Ok(())
        })
        .await
        .unwrap();
    }
}
