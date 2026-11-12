//! What Search offers before anything is typed (capability `search-start`).
//!
//! - **Recent**: the titles this user opened from Search, newest first, on every device. A title
//!   opened again moves to the front; only the latest [`RECENT_KEPT`] are kept. Titles merged
//!   since follow the merge; titles gone from the library drop out.
//! - **Suggestions**: from the user's own library, and simple on purpose: movies and shows they
//!   haven't started, those in the genres they watch most first, then best rated first, movies
//!   and shows taking turns so neither fills the row. Without any watch history: best rated.
//!   What's in Recent isn't suggested again.

use std::{collections::HashSet, fmt::Write};

use anyhow::Result;
use rusqlite::{params, Connection};
use serde_json::Value;

use crate::library::store;

/// How many recent titles are kept, and answered, per user.
pub const RECENT_KEPT: usize = 20;
/// How many titles are suggested.
pub const SUGGESTIONS: usize = 20;
/// Best rated first; unrated last, newest first among those.
const BEST_FIRST: &str = "i.community_rating IS NULL, i.community_rating DESC, i.added_at DESC, i.id";
/// How many of the user's top genres count as "the genres they watch most".
const TOP_GENRES: usize = 3;

/// Puts a title at the front of the user's Recent. Only what Search can open counts (false: no
/// such title).
pub fn record(c: &Connection, user: &str, item: &str, now: i64) -> Result<bool> {
    let known = c.query_row(
        "SELECT EXISTS (SELECT 1 FROM items WHERE id = ?1 AND kind IN ('movie', 'series', 'episode'))",
        [item],
        |r| r.get::<_, bool>(0),
    )?;
    if !known {
        return Ok(false);
    }
    c.execute(
        "INSERT OR REPLACE INTO user_search_recent(user_id, item_id, opened_at) VALUES (?1, ?2, ?3)",
        params![user, item, now],
    )?;
    c.execute(
        "DELETE FROM user_search_recent WHERE user_id = ?1 AND rowid NOT IN
         (SELECT rowid FROM user_search_recent WHERE user_id = ?1 ORDER BY opened_at DESC, rowid DESC LIMIT ?2)",
        params![user, RECENT_KEPT as i64],
    )?;
    Ok(true)
}

/// Forgets the user's Recent, on every device. Answers how many titles it held.
pub fn clear(c: &Connection, user: &str) -> Result<usize> {
    Ok(c.execute("DELETE FROM user_search_recent WHERE user_id = ?1", [user])?)
}

/// The ids of the user's Recent, newest first, merged titles under their new ids, once each.
fn recent_ids(c: &Connection, user: &str) -> Result<Vec<String>> {
    let mut st = c.prepare(
        "SELECT COALESCE(ir.new_id, r.item_id) FROM user_search_recent r
         LEFT JOIN item_redirects ir ON ir.old_id = r.item_id
         WHERE r.user_id = ?1 ORDER BY r.opened_at DESC, r.rowid DESC",
    )?;
    let ids = st
        .query_map([user], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut seen = HashSet::new();
    Ok(ids.into_iter().filter(|id| seen.insert(id.clone())).collect())
}

/// The user's Recent as the API shapes items (only what is still in the library).
pub fn recent(c: &Connection, user: Option<&str>) -> Result<Vec<Value>> {
    let Some(u) = user else { return Ok(Vec::new()) };
    let ids = recent_ids(c, u)?;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let items = store::items_by_ids(c, user, ids, RECENT_KEPT as i64)?;
    Ok(items)
}

/// SQL: the item (alias `i`) is one the user has started (a resume point or watched; a show: any
/// of its episodes).
fn started_sql(uid: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM user_data u LEFT JOIN items se ON se.id = u.item_id
                 WHERE u.user_id = '{uid}' AND (u.played = 1 OR u.position > 0)
                   AND (u.item_id = i.id OR se.series_id = i.id OR se.parent_id = i.id))"
    )
}

/// The genres of what the user watched most (a show's episodes count for the show), most first.
fn top_genres(c: &Connection, user: &str) -> Result<Vec<String>> {
    let mut st = c.prepare(
        "SELECT g.value, COUNT(*) AS n
         FROM user_data u JOIN items e ON e.id = u.item_id
         LEFT JOIN items s ON s.id = e.series_id, json_each(COALESCE(s.genres, e.genres, '[]')) g
         WHERE u.user_id = ?1 AND u.played = 1
         GROUP BY g.value ORDER BY n DESC, g.value LIMIT ?2",
    )?;
    let genres = st
        .query_map(params![user, TOP_GENRES as i64], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(genres)
}

/// One kind's suggestions: the top genres' titles first, best rated first.
fn suggestions_of_kind(
    c: &Connection,
    user: Option<&str>,
    kind: &str,
    genres: &[String],
    exclude: &[String],
    limit: usize,
) -> Result<Vec<Value>> {
    let mut base = format!("i.kind = '{kind}' AND {}", store::visible_any("i"));
    if let Some(u) = user {
        let _ = write!(base, " AND NOT {}", started_sql(u));
    }
    let mut args: Vec<String> = Vec::new();
    if !exclude.is_empty() {
        let _ = write!(base, " AND i.id NOT IN ({})", vec!["?"; exclude.len()].join(", "));
        args.extend(exclude.iter().cloned());
    }
    let mut picked = Vec::new();
    if !genres.is_empty() {
        let in_genres = format!(
            "EXISTS (SELECT 1 FROM json_each(COALESCE(i.genres, '[]')) WHERE value IN ({}))",
            vec!["?"; genres.len()].join(", ")
        );
        let mut a = args.clone();
        a.extend(genres.iter().cloned());
        picked = store::query_items(c, user, &format!("{base} AND {in_genres}"), BEST_FIRST, &a, 0, limit as i64)?.0;
    }
    if picked.len() < limit {
        let have: Vec<String> = picked.iter().filter_map(|v| v["id"].as_str().map(String::from)).collect();
        let mut more_base = base.clone();
        let mut a = args;
        if !have.is_empty() {
            let _ = write!(more_base, " AND i.id NOT IN ({})", vec!["?"; have.len()].join(", "));
            a.extend(have);
        }
        picked.extend(store::query_items(c, user, &more_base, BEST_FIRST, &a, 0, (limit - picked.len()) as i64)?.0);
    }
    Ok(picked)
}

/// Takes turns from two lists, up to `limit`; when one runs out the other fills the rest.
fn interleave(a: Vec<Value>, b: Vec<Value>, limit: usize) -> Vec<Value> {
    let (mut a, mut b) = (a.into_iter(), b.into_iter());
    let mut out = Vec::with_capacity(limit);
    loop {
        let (x, y) = (a.next(), b.next());
        if x.is_none() && y.is_none() {
            break;
        }
        out.extend(x);
        out.extend(y);
    }
    out.truncate(limit);
    out
}

/// What to suggest to the user (`user` None: a caller without watch state, best rated).
pub fn suggestions(c: &Connection, user: Option<&str>, recent: &[Value]) -> Result<Vec<Value>> {
    let exclude: Vec<String> = recent.iter().filter_map(|v| v["id"].as_str().map(String::from)).collect();
    let genres = match user {
        Some(u) => top_genres(c, u)?,
        None => Vec::new(),
    };
    let movies = suggestions_of_kind(c, user, "movie", &genres, &exclude, SUGGESTIONS)?;
    let shows = suggestions_of_kind(c, user, "series", &genres, &exclude, SUGGESTIONS)?;
    Ok(interleave(movies, shows, SUGGESTIONS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    const U: &str = "u1";

    fn item(c: &Connection, id: &str, kind: &str, series: Option<&str>, genres: &str, rating: Option<f64>) {
        c.execute(
            "INSERT INTO items(id, kind, library_id, key, parent_id, series_id, title, sort_title, identify_state, added_at, updated_at, genres, community_rating)
             VALUES (?1, ?2, 'lib', ?1, ?3, ?3, ?1, ?1, 'matched', 0, 0, ?4, ?5)",
            params![id, kind, series, genres, rating],
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
    }

    async fn with_db(f: impl FnOnce(&Connection) -> Result<()> + Send + 'static) {
        let db = Db::memory().unwrap();
        db.call(|c| {
            c.execute_batch(
                "INSERT INTO users(id, name, password_hash, is_admin, created_at) VALUES ('u1', 'A', 'x', 1, 0), ('u2', 'B', 'x', 0, 0);",
            )?;
            f(c)
        })
        .await
        .unwrap();
    }

    fn ids(v: &[Value]) -> Vec<&str> {
        v.iter().map(|x| x["id"].as_str().unwrap()).collect()
    }

    #[tokio::test]
    async fn recent_is_newest_first_per_user_and_capped() {
        with_db(|c| {
            for n in 0..25 {
                item(c, &format!("m{n}"), "movie", None, "[]", None);
            }
            for n in 0..25 {
                assert!(record(c, U, &format!("m{n}"), 100 + n)?);
            }
            let r = recent(c, Some(U))?;
            assert_eq!(r.len(), RECENT_KEPT, "only the latest are kept");
            assert_eq!(ids(&r)[..2], ["m24", "m23"]);
            // Opened again: to the front, once.
            assert!(record(c, U, "m10", 500)?);
            let r = recent(c, Some(U))?;
            assert_eq!(ids(&r)[0], "m10");
            assert_eq!(ids(&r).iter().filter(|i| **i == "m10").count(), 1);
            // Another user has their own; clearing is per user.
            assert!(recent(c, Some("u2"))?.is_empty());
            assert!(record(c, "u2", "m0", 600)?);
            assert_eq!(clear(c, U)?, RECENT_KEPT);
            assert!(recent(c, Some(U))?.is_empty());
            assert_eq!(ids(&recent(c, Some("u2"))?), ["m0"]);
            // Nothing recent for a caller without an account.
            assert!(recent(c, None)?.is_empty());
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn recent_only_holds_what_search_can_open_and_follows_the_library() {
        with_db(|c| {
            item(c, "a", "movie", None, "[]", None);
            item(c, "b", "movie", None, "[]", None);
            item(c, "gone", "movie", None, "[]", None);
            item(c, "sn", "season", None, "[]", None);
            assert!(!record(c, U, "nope", 1)?);
            assert!(!record(c, U, "sn", 1)?, "not something Search opens");
            record(c, U, "a", 1)?;
            record(c, U, "b", 2)?;
            record(c, U, "gone", 3)?;
            // Merged into another title: found under the new id (once); removed: dropped.
            c.execute("INSERT INTO item_redirects(old_id, new_id) VALUES ('a', 'b')", [])?;
            c.execute("DELETE FROM files WHERE id = 'gone'", [])?;
            c.execute("DELETE FROM items WHERE id = 'gone'", [])?;
            assert_eq!(ids(&recent(c, Some(U))?), ["b"]);
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn suggestions_prefer_unstarted_titles_in_watched_genres_and_mix_kinds() {
        with_db(|c| {
            item(c, "seen", "movie", None, r#"["Drama"]"#, Some(9.0));
            item(c, "started", "movie", None, r#"["Drama"]"#, Some(9.0));
            item(c, "drama-low", "movie", None, r#"["Drama"]"#, Some(5.0));
            item(c, "comedy-high", "movie", None, r#"["Comedy"]"#, Some(9.5));
            item(c, "unrated", "movie", None, r#"["Comedy"]"#, None);
            item(c, "show-new", "series", None, r#"["Comedy"]"#, Some(9.8));
            item(c, "show-new-e", "episode", Some("show-new"), "[]", None);
            item(c, "show-begun", "series", None, r#"["Drama"]"#, Some(9.9));
            item(c, "show-begun-e", "episode", Some("show-begun"), "[]", None);
            c.execute_batch(
                "INSERT INTO user_data(user_id, item_id, played, position, updated_at) VALUES
                 ('u1', 'seen', 1, 0, 1), ('u1', 'started', 0, 600, 1), ('u1', 'show-begun-e', 1, 0, 1);",
            )?;
            let s = suggestions(c, Some(U), &[])?;
            assert!(
                !ids(&s).contains(&"seen") && !ids(&s).contains(&"started") && !ids(&s).contains(&"show-begun"),
                "started titles left out: {:?}",
                ids(&s)
            );
            // Drama (watched most) first even though it is rated lower, then best rated; a
            // show takes the next turn; unrated last.
            assert_eq!(ids(&s), ["drama-low", "show-new", "comedy-high", "unrated"]);
            // Without history: best rated.
            let s2 = suggestions(c, Some("u2"), &[])?;
            assert_eq!(ids(&s2)[0], "comedy-high");
            // What's in Recent isn't suggested.
            let recent = vec![serde_json::json!({"id": "drama-low"})];
            assert!(!ids(&suggestions(c, Some(U), &recent)?).contains(&"drama-low"));
            Ok(())
        })
        .await;
    }

    #[test]
    fn interleave_fills_from_the_other_list() {
        let v = |s: &[&str]| s.iter().map(|x| serde_json::json!({"id": x})).collect::<Vec<_>>();
        assert_eq!(ids(&interleave(v(&["a", "b", "c"]), v(&["1"]), 3)), ["a", "1", "b"]);
        assert_eq!(ids(&interleave(v(&[]), v(&["1", "2"]), 5)), ["1", "2"]);
    }
}
