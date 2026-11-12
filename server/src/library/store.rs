//! The library's queries for the browse and Fix Match API (`api.rs`): items as the API shapes
//! them (`query_items`, `row_json`, `decorate`), the lists built on them (seasons, episodes,
//! latest, by id), one title in full (`item_detail`), and what Fix Match reads and writes. The
//! handlers parse the request, call one of these inside `Db::call`, and serialise the answer.

use std::collections::HashMap;
use std::fmt::Write as _;

use anyhow::Result;
use rusqlite::{params_from_iter, Connection, OptionalExtension, Row};
use serde_json::{json, Value};

use super::{browse, ids_map, rfc3339, streams, util, TITLE_ROLES};
use crate::userdata;

/// A title is shown while one of its files is there (or went missing less than a day ago).
pub(crate) fn visible_sql(alias: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM files vf WHERE vf.item_id = {alias}.id AND vf.role IN {TITLE_ROLES} AND (vf.missing_since IS NULL OR vf.missing_since > {}))",
        util::now() - 86_400
    )
}

/// When a movie or episode was added, as Recently Added and the Date Added sort see it: its own
/// `added_at`, or later, when a title file arrived after the title's first file (a new version
/// or a replacement). Files found by one scan share its `first_seen`, so a library's
/// first scan moves nothing; a renamed, moved or overwritten file keeps its row and
/// `first_seen`, so it moves nothing either. Series and seasons have no files: `added_at`.
pub(crate) fn added_sql(alias: &str) -> String {
    format!(
        "MAX({alias}.added_at, COALESCE((SELECT MAX(nf.first_seen) FROM files nf WHERE nf.item_id = {alias}.id AND nf.role = 'main'
           AND nf.missing_since IS NULL AND nf.first_seen > (SELECT MIN(ff.first_seen) FROM files ff WHERE ff.item_id = {alias}.id)), 0))"
    )
}

/// Series and seasons are shown while one of their episodes is.
pub(crate) fn visible_any(alias: &str) -> String {
    format!(
        "(CASE WHEN {alias}.kind IN ('movie', 'episode') THEN {} ELSE EXISTS (SELECT 1 FROM items ve WHERE ve.kind = 'episode' AND (ve.series_id = {alias}.id OR ve.parent_id = {alias}.id) AND {}) END)",
        visible_sql(alias),
        visible_sql("ve")
    )
}

/// Leaves out what a Moonrise user has watched (nothing for other callers).
fn unwatched_sql(user: Option<&str>, alias: &str) -> String {
    match user {
        Some(u) => {
            format!(" AND NOT EXISTS (SELECT 1 FROM user_data u WHERE u.user_id = '{u}' AND u.item_id = {alias}.id AND u.played = 1)")
        }
        None => String::new(),
    }
}

/// `GET /v1/items?parent=`: a series' seasons or a season's episodes (bound twice).
pub(crate) const CHILDREN_OF: &str = "(i.parent_id = ? OR (i.series_id = ? AND i.kind = 'season'))";

/// One library (older apps' `parent_id`).
pub(crate) const IN_LIBRARY: &str = "i.library_id = ?";

/// The order of lists by id (re-sorted into the ids' order after).
pub(crate) const BY_SORT_TITLE: &str = "i.sort_title";

/// Items of these kinds (each one of movie, series, season, episode).
pub(crate) fn kind_in(kinds: &[&str]) -> String {
    format!(
        "i.kind IN ({})",
        kinds.iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(",")
    )
}

/// Items of one kind (`movie` or `series`).
pub(crate) fn kind_is(top: &str) -> String {
    format!("i.kind = '{top}'")
}

/// Items with one of `n` ids (bound in order).
pub(crate) fn ids_in(n: usize) -> String {
    let marks = vec!["?"; n].join(",");
    format!("i.id IN ({marks})")
}

/// The columns `row_json` reads (`added_at` as `added_sql` has it).
fn cols() -> String {
    format!(
        "i.id, i.kind, i.library_id, i.title, i.sort_title, i.original_title, i.year, i.premiere_date, i.end_date, i.status,
    i.overview, i.tagline, i.official_rating, i.community_rating, i.critic_rating, i.genres, i.studios, i.provider_ids, i.runtime,
    i.images, i.series_id, i.parent_id, i.season_no, i.episode_no, i.episode_end, i.identify_state, i.identify_source, i.confidence,
    i.identify_note, {}, i.updated_at,
    s.title, s.images, p.title, p.images, i.collection_id, col.name, i.part_no,
    CASE WHEN i.part_no IS NOT NULL THEN (SELECT COUNT(*) FROM items pp WHERE pp.kind = 'episode' AND pp.parent_id = i.parent_id AND pp.episode_no = i.episode_no AND pp.part_no IS NOT NULL) END",
        added_sql("i")
    )
}

const FROM: &str = "items i LEFT JOIN items s ON s.id = i.series_id LEFT JOIN items p ON p.id = i.parent_id LEFT JOIN collections col ON col.id = i.collection_id";

fn json_col(r: &Row, i: usize) -> rusqlite::Result<Value> {
    Ok(r.get::<_, Option<String>>(i)?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null))
}

/// The neutral image set: poster, backdrop[], logo, thumb (16:9: an episode's still, else the
/// first backdrop).
fn image_set(own: &Value) -> Value {
    let backdrop = own["backdrop"].as_array().cloned().unwrap_or_default();
    let thumb = own["still"]
        .as_str()
        .map(String::from)
        .or_else(|| backdrop.first().and_then(|b| b.as_str()).map(String::from));
    json!({"poster": own["poster"], "backdrop": backdrop, "logo": own["logo"], "thumb": thumb})
}

fn row_json(r: &Row) -> rusqlite::Result<Value> {
    let kind: String = r.get(1)?;
    let own = json_col(r, 19)?;
    let series_images = json_col(r, 32)?;
    let parent_images = json_col(r, 34)?;
    let series_id: Option<String> = r.get(20)?;
    let parent_id: Option<String> = r.get(21)?;
    let mut v = json!({
        "id": r.get::<_, String>(0)?,
        "kind": kind,
        "library_id": r.get::<_, String>(2)?,
        "title": r.get::<_, String>(3)?,
        "sort_title": r.get::<_, Option<String>>(4)?,
        "original_title": r.get::<_, Option<String>>(5)?,
        "year": r.get::<_, Option<i64>>(6)?,
        "premiere_date": r.get::<_, Option<String>>(7)?,
        "end_date": r.get::<_, Option<String>>(8)?,
        "status": r.get::<_, Option<String>>(9)?,
        "overview": r.get::<_, Option<String>>(10)?,
        "tagline": r.get::<_, Option<String>>(11)?,
        "official_rating": r.get::<_, Option<String>>(12)?,
        "community_rating": r.get::<_, Option<f64>>(13)?,
        "critic_rating": r.get::<_, Option<f64>>(14)?,
        "genres": json_col(r, 15)?.as_array().cloned().unwrap_or_default(),
        "studios": json_col(r, 16)?.as_array().cloned().unwrap_or_default(),
        "provider_ids": ids_map(r.get::<_, Option<String>>(17)?.as_deref()),
        "runtime": r.get::<_, Option<f64>>(18)?,
        "images": image_set(&own),
        "series_id": series_id,
        "series_title": r.get::<_, Option<String>>(31)?,
        "season_id": if kind == "episode" { parent_id.clone() } else { None },
        "season_title": if kind == "episode" { r.get::<_, Option<String>>(33)? } else { None },
        "season_no": r.get::<_, Option<i64>>(22)?,
        "episode_no": r.get::<_, Option<i64>>(23)?,
        "episode_end": r.get::<_, Option<i64>>(24)?,
        "identify": {"state": r.get::<_, String>(25)?, "source": r.get::<_, Option<String>>(26)?, "confidence": r.get::<_, Option<f64>>(27)?, "note": r.get::<_, Option<String>>(28)?},
        "added_at": rfc3339(r.get::<_, i64>(29)?),
        "updated_at": rfc3339(r.get::<_, i64>(30)?),
        "user": Value::Null,
    });
    // One part of an episode split over several files: Part 1 of 2…
    if let Some(part) = r.get::<_, Option<i64>>(37)? {
        v["part"] = json!(part);
        v["parts"] = json!(r.get::<_, Option<i64>>(38)?);
    }
    if let Some(cid) = r.get::<_, Option<i64>>(35)? {
        v["collection"] = json!({"id": cid, "name": r.get::<_, Option<String>>(36)?});
    }
    if kind == "episode" || kind == "season" {
        // The series' art (and, for an episode, its season's poster).
        let mut pi = image_set(&series_images);
        if kind == "episode" {
            if let Some(p) = parent_images["poster"].as_str() {
                pi["season_poster"] = json!(p);
            }
        }
        v["parent_images"] = pi;
        if v["images"]["poster"].is_null() {
            v["images"]["poster"] = v["parent_images"]["poster"].clone();
        }
    }
    Ok(v)
}

/// Child counts and a track summary (badges) for a page of items (the largest healthy file's;
/// a `broken` one only when the title has nothing else).
fn decorate(c: &Connection, items: &mut [Value]) -> rusqlite::Result<()> {
    let now_vis = visible_sql("e");
    for v in items.iter_mut() {
        let id = v["id"].as_str().unwrap_or("").to_string();
        match v["kind"].as_str().unwrap_or("") {
            "series" => {
                let n: i64 = c.query_row(
                    &format!("SELECT COUNT(DISTINCT e.parent_id) FROM items e WHERE e.kind = 'episode' AND e.series_id = ?1 AND {now_vis}"),
                    [&id],
                    |r| r.get(0),
                )?;
                v["child_count"] = json!(n);
                let n: i64 = c.query_row(
                    &format!(
                        "SELECT COUNT(DISTINCT {}) FROM items e WHERE e.kind = 'episode' AND e.series_id = ?1 AND {now_vis}",
                        crate::userdata::EPISODE_OF
                    ),
                    [&id],
                    |r| r.get(0),
                )?;
                v["episode_count"] = json!(n);
            }
            "season" => {
                let n: i64 = c.query_row(
                    &format!(
                        "SELECT COUNT(DISTINCT {}) FROM items e WHERE e.kind = 'episode' AND e.parent_id = ?1 AND {now_vis}",
                        crate::userdata::EPISODE_OF
                    ),
                    [&id],
                    |r| r.get(0),
                )?;
                v["child_count"] = json!(n);
            }
            _ => {
                let probe: Option<(String, Option<f64>, i64)> = c
                    .query_row(
                        &format!(
                            "SELECT fp.probe, f.duration, (SELECT COUNT(*) FROM files f2 WHERE f2.item_id = f.item_id AND f2.role IN {TITLE_ROLES} AND f2.missing_since IS NULL)
                             FROM files f JOIN files_probe fp ON fp.path = f.path AND fp.size = f.size AND fp.mtime = f.mtime
                             WHERE f.item_id = ?1 AND f.role IN {TITLE_ROLES} AND f.missing_since IS NULL ORDER BY f.role = 'broken', f.size DESC LIMIT 1"
                        ),
                        [&id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                if let Some((p, duration, versions)) = probe {
                    if let Ok(pv) = serde_json::from_str::<Value>(&p) {
                        v["media"] = media_summary(&pv);
                    }
                    // The file's own length, as the probe reports it (TMDB's can be another cut).
                    if let Some(d) = duration {
                        v["tmdb_runtime"] = v["runtime"].clone();
                        v["runtime"] = json!(d);
                    }
                    v["versions"] = json!(versions);
                }
            }
        }
    }
    Ok(())
}

/// What the tech badges need (from the probe; sidecar files aren't counted here).
fn media_summary(probe: &Value) -> Value {
    let s = streams::derive(std::path::Path::new(""), probe, &[]);
    let video = s.iter().find(|x| x.kind == "Video");
    json!({
        "video": video.map(|v| json!({"codec": v.codec, "width": v.width, "height": v.height, "range": v.video_range_type, "title": v.display_title})),
        "audio": s.iter().filter(|x| x.kind == "Audio").map(|a| json!({"codec": a.codec, "channels": a.channels, "profile": a.profile, "language": a.language, "title": a.display_title, "default": a.is_default})).collect::<Vec<_>>(),
        "subtitles": s.iter().filter(|x| x.kind == "Subtitle").map(|a| json!({"codec": a.codec, "language": a.language, "text": a.is_text, "forced": a.is_forced})).collect::<Vec<_>>(),
    })
}

/// A page of items as the API shapes them (with the viewer's watch state), and how many match.
pub(crate) fn query_items(
    c: &Connection,
    user: Option<&str>,
    sql_where: &str,
    order: &str,
    args: &[String],
    offset: i64,
    limit: i64,
) -> Result<(Vec<Value>, i64)> {
    let total: i64 = c.query_row(
        &format!("SELECT COUNT(*) FROM {FROM} WHERE {sql_where}"),
        params_from_iter(args.iter()),
        |r| r.get(0),
    )?;
    let sql = format!(
        "SELECT {} FROM {FROM} WHERE {sql_where} ORDER BY {order} LIMIT {limit} OFFSET {offset}",
        cols()
    );
    let mut st = c.prepare(&sql)?;
    let mut rows = st
        .query_map(params_from_iter(args.iter()), row_json)?
        .collect::<Result<Vec<_>, _>>()?;
    decorate(c, &mut rows)?;
    if let Some(u) = user {
        userdata::attach(c, u, &mut rows)?;
    }
    Ok((rows, total))
}

/// The alphabet index of a title sort (offsets into the whole filtered list).
pub(crate) fn letter_index(c: &Connection, sql_where: &str, args: &[String], order: &str) -> Result<Vec<(String, i64)>> {
    Ok(browse::letter_index(c, FROM, sql_where, args, order)?)
}

/// Items by id, in that order (at most `limit`).
pub(crate) fn items_by_ids(c: &Connection, user: Option<&str>, ids: Vec<String>, limit: i64) -> Result<Vec<Value>> {
    let (mut items, _) = query_items(c, user, &ids_in(ids.len()), BY_SORT_TITLE, &ids, 0, limit)?;
    items.sort_by_key(|v| ids.iter().position(|x| v["id"] == *x));
    Ok(items)
}

/// A series' seasons, specials last.
pub(crate) fn seasons(c: &Connection, user: Option<&str>, series: String) -> Result<(Vec<Value>, i64)> {
    query_items(
        c,
        user,
        &format!("i.kind = 'season' AND i.parent_id = ? AND {}", visible_any("i")),
        "i.season_no = 0, i.season_no",
        &[series],
        0,
        500,
    )
}

/// Which of a series' episodes: one season's, by number or by the season's id.
pub(crate) enum Season {
    Number(i64),
    Id(String),
}

/// A series' episodes (or one season's) in airing order, specials last.
pub(crate) fn episodes(c: &Connection, user: Option<&str>, series: String, season: Option<Season>) -> Result<(Vec<Value>, i64)> {
    let mut cond = format!("i.kind = 'episode' AND i.series_id = ? AND {}", visible_sql("i"));
    let mut args = vec![series];
    match season {
        Some(Season::Number(n)) => {
            let _ = write!(cond, " AND i.season_no = {n}");
        }
        Some(Season::Id(id)) => {
            cond += " AND i.parent_id = ?";
            args.push(id);
        }
        None => {}
    }
    query_items(
        c,
        user,
        &cond,
        "i.season_no = 0, i.season_no, i.episode_no, COALESCE(i.part_no, 1), i.title",
        &args,
        0,
        2000,
    )
}

/// The libraries as tiles: counts of movies and shows, and the newest title's backdrop.
pub(crate) fn libraries(c: &Connection) -> Result<Value> {
    let vis = visible_any("i");
    let mut st = c.prepare("SELECT id, name, kind FROM libraries ORDER BY sort")?;
    let libs = st
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut items = Vec::new();
    for (id, name, kind) in libs {
        let count = |top: &str| -> rusqlite::Result<i64> {
            c.query_row(
                &format!("SELECT COUNT(*) FROM items i WHERE i.library_id = ?1 AND i.kind = ?2 AND {vis}"),
                [&id, top],
                |r| r.get(0),
            )
        };
        let (movies, shows) = (count("movie")?, count("series")?);
        let tops = match kind.as_str() {
            "movies" => "'movie'",
            "shows" => "'series'",
            _ => "'movie', 'series'",
        };
        // Tile art: the newest title's backdrop.
        let art: Option<String> = c
            .query_row(
                &format!("SELECT i.images FROM items i WHERE i.library_id = ?1 AND i.kind IN ({tops}) AND i.images LIKE '%\"backdrop\":[\"%' AND {vis} ORDER BY {} DESC LIMIT 1", added_sql("i")),
                [&id],
                |r| r.get(0),
            )
            .optional()?;
        let backdrop = art
            .and_then(|a| serde_json::from_str::<Value>(&a).ok())
            .map_or(Value::Null, |v| v["backdrop"][0].clone());
        let (collection_type, n) = match kind.as_str() {
            "movies" => ("movies", movies),
            "shows" => ("tvshows", shows),
            _ => ("mixed", movies + shows),
        };
        items.push(json!({"id": id, "name": name, "kind": kind, "collection_type": collection_type, "item_count": n, "movies": movies, "shows": shows, "backdrop": backdrop}));
    }
    let total = items.len();
    Ok(json!({"items": items, "total": total}))
}

/// What a library was made as (`movies`, `shows`, `mixed`), or None: no such library.
pub(crate) fn library_kind(c: &Connection, id: &str) -> Result<Option<String>> {
    Ok(c.query_row("SELECT kind FROM libraries WHERE id = ?1", [id], |r| r.get(0))
        .optional()?)
}

/// The newest movies (of one library, or all), without what the user has watched.
pub(crate) fn latest_movies(c: &Connection, user: Option<&str>, library: Option<&str>, limit: i64) -> Result<Vec<Value>> {
    let (lib_sql, args) = match library {
        Some(id) => (" AND i.library_id = ?", vec![id.to_string()]),
        None => ("", vec![]),
    };
    Ok(query_items(
        c,
        user,
        &format!("i.kind = 'movie'{lib_sql} AND {}{}", visible_sql("i"), unwatched_sql(user, "i")),
        &format!("{} DESC", added_sql("i")),
        &args,
        0,
        limit,
    )?
    .0)
}

/// Each series' latest arrivals (of one library, or all; the `limit` series with the newest),
/// as (episode, series, added_at), newest first, without what the user has watched.
pub(crate) fn latest_episodes(c: &Connection, user: Option<&str>, library: Option<&str>, limit: i64) -> Result<Vec<(String, String, i64)>> {
    let vis = format!("{}{}", visible_sql("e"), unwatched_sql(user, "e"));
    let lib_sql = if library.is_some() { "AND e.library_id = ?1" } else { "" };
    let added = added_sql("e");
    let mut st = c.prepare(&format!(
        "WITH a AS (SELECT e.id, e.series_id, e.season_no, e.episode_no, {added} at FROM items e
                    WHERE e.kind = 'episode' {lib_sql} AND e.series_id IS NOT NULL AND {vis}),
              s AS (SELECT series_id sid, MAX(at) m FROM a GROUP BY series_id ORDER BY m DESC LIMIT {limit})
         SELECT a.id, a.series_id, a.at FROM a JOIN s ON s.sid = a.series_id
         WHERE a.at > s.m - 259200
         ORDER BY s.m DESC, a.at DESC, a.season_no DESC, a.episode_no DESC"
    ))?;
    let rows = st
        .query_map(params_from_iter(library.iter()), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The ids of the titles (40) and episodes (20) a search finds, best first.
pub(crate) fn search_ids(c: &Connection, titles: &str, episodes: &str) -> Result<(Vec<String>, Vec<String>)> {
    let find = |q: &str, kinds: &str, n: i64| -> rusqlite::Result<Vec<String>> {
        let mut st = c.prepare(&format!(
            "SELECT f.item_id FROM items_fts f JOIN items i ON i.id = f.item_id WHERE items_fts MATCH ?1 AND i.kind IN ({kinds}) AND {} ORDER BY f.rank LIMIT {n}",
            visible_any("i")
        ))?;
        let rows = st.query_map([q], |r| r.get(0))?.collect();
        rows
    };
    Ok((find(titles, "'movie', 'series'", 40)?, find(episodes, "'episode'", 20)?))
}

/// A title's file as `item_detail` reads it.
pub(crate) struct FileRowOut {
    pub(crate) id: String,
    pub(crate) path: String,
    pub(crate) size: i64,
    pub(crate) mtime: i64,
    pub(crate) container: Option<String>,
    pub(crate) duration: Option<f64>,
    pub(crate) bitrate: Option<i64>,
    pub(crate) label: Option<String>,
    pub(crate) missing: bool,
    pub(crate) chapters: Option<String>,
    pub(crate) manual: bool,
    pub(crate) nfo_edition: Option<String>,
    pub(crate) probe: Option<String>,
    pub(crate) broken: bool,
    pub(crate) broken_reason: Option<String>,
}

/// A viewer's version choice for a title and resume points: the title's (and the file it was
/// reached in), and each version's own.
#[derive(Default)]
pub(crate) struct UserVersions {
    pub(crate) choice: Option<String>,
    pub(crate) resume: Option<(f64, Option<String>)>,
    pub(crate) positions: Vec<(String, f64)>,
}

/// One title in full: the item (with the viewer's watch state, and ratings if asked for) and
/// its people, its files, and the viewer's versions. None: no such item.
pub(crate) fn item_detail(
    c: &Connection,
    id: &str,
    user: Option<&str>,
    with_ratings: bool,
) -> Result<Option<(Value, Vec<FileRowOut>, UserVersions)>> {
    let Some(mut v) = c
        .query_row(&format!("SELECT {} FROM {FROM} WHERE i.id = ?1", cols()), [id], row_json)
        .optional()?
    else {
        return Ok(None);
    };
    decorate(c, std::slice::from_mut(&mut v))?;
    if let Some(u) = user {
        userdata::attach(c, u, std::slice::from_mut(&mut v))?;
    }
    if with_ratings {
        super::ratings::attach(c, std::slice::from_mut(&mut v))?;
    }
    let mut st = c.prepare("SELECT p.id, p.name, ip.kind, ip.role, p.image FROM item_people ip JOIN people p ON p.id = ip.person_id WHERE ip.item_id = ?1 ORDER BY ip.ord")?;
    let mut people_of = |id: &str| -> rusqlite::Result<Vec<Value>> {
        st.query_map([id], |r| Ok(json!({"id": r.get::<_, String>(0)?, "name": r.get::<_, String>(1)?, "kind": r.get::<_, String>(2)?, "role": r.get::<_, Option<String>>(3)?, "image": r.get::<_, Option<String>>(4)?})))?
            .collect::<Result<Vec<_>, _>>()
    };
    let mut people = people_of(id)?;
    // Episodes and seasons: the series' cast.
    if people.is_empty() {
        if let Some(sid) = v["series_id"].as_str() {
            people = people_of(sid)?;
        }
    }
    v["people"] = json!(people);
    let mut st = c.prepare(&format!(
        "SELECT f.id, f.path, f.size, f.mtime, f.container, f.duration, f.bitrate, f.version_label, f.missing_since, fp.chapters,
                        f.manual_key, f.edition, fp.probe, f.role = 'broken', f.probe_error
                 FROM files f LEFT JOIN files_probe fp ON fp.path = f.path AND fp.size = f.size AND fp.mtime = f.mtime
                 WHERE f.item_id = ?1 AND f.role IN {TITLE_ROLES} ORDER BY f.missing_since IS NOT NULL, f.role = 'broken', f.size DESC"
    ))?;
    let files = st
        .query_map([id], |r| {
            Ok(FileRowOut {
                id: r.get(0)?,
                path: r.get(1)?,
                size: r.get(2)?,
                mtime: r.get(3)?,
                container: r.get(4)?,
                duration: r.get(5)?,
                bitrate: r.get(6)?,
                label: r.get(7)?,
                missing: r.get::<_, Option<i64>>(8)?.is_some(),
                chapters: r.get(9)?,
                manual: r.get::<_, Option<String>>(10)?.is_some(),
                nfo_edition: r.get(11)?,
                probe: r.get(12)?,
                broken: r.get(13)?,
                broken_reason: r.get(14)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    // The viewer's version and where they stopped in each (a cut of another length keeps its own).
    let mut mine = UserVersions::default();
    if let Some(u) = user {
        mine.choice = c
            .query_row(
                "SELECT file_id FROM user_versions WHERE user_id = ?1 AND item_id = ?2",
                rusqlite::params![u, id],
                |r| r.get(0),
            )
            .optional()?;
        mine.resume = c
            .query_row(
                "SELECT position, position_file FROM user_data WHERE user_id = ?1 AND item_id = ?2",
                rusqlite::params![u, id],
                |r| Ok((r.get::<_, f64>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        let mut q = c.prepare(&format!(
            "SELECT p.file_id, p.position FROM user_file_positions p JOIN files f ON f.id = p.file_id WHERE p.user_id = ?1 AND f.item_id = ?2 AND f.role IN {TITLE_ROLES}"
        ))?;
        mine.positions = q
            .query_map(rusqlite::params![u, id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
    }
    Ok(Some((v, files, mine)))
}

/// The file a title's segments come from: (chapters, duration, path, size, mtime, probe,
/// credits, the item's kind).
pub(crate) type SegRow = (
    Option<String>,
    Option<f64>,
    String,
    i64,
    i64,
    Option<String>,
    Option<String>,
    String,
);

/// The file played for a title's segments: the version asked for (a file id, or empty), else
/// the largest healthy one.
pub(crate) fn segment_file(c: &Connection, item: String, version: String) -> Result<Option<SegRow>> {
    Ok(c.query_row(
        &format!(
            "SELECT fp.chapters, f.duration, f.path, f.size, f.mtime, fp.probe, fp.credits, i.kind
                     FROM files f JOIN items i ON i.id = f.item_id
                     LEFT JOIN files_probe fp ON fp.path = f.path AND fp.size = f.size AND fp.mtime = f.mtime
                     WHERE f.item_id = ?1 AND f.role IN {TITLE_ROLES} AND f.missing_since IS NULL ORDER BY f.role = 'broken', f.id = ?2 DESC, f.size DESC LIMIT 1"
        ),
        [item, version],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
    )
    .optional()?)
}

/// Every file is identified again by the next pass.
pub(crate) fn forget_identified(c: &Connection) -> Result<usize> {
    Ok(c.execute("UPDATE files SET identified_at = NULL", [])?)
}

/// Fix Match's list: the titles in review (or, `llm`, those the LLM matched, with its latest
/// verdicts).
pub(crate) fn review(c: &Connection, llm: bool) -> Result<Value> {
    let cond = if llm {
        "i.identify_state = 'llm'"
    } else {
        "i.identify_state = 'review'"
    };
    let mut st = c.prepare(&format!(
        "SELECT i.id, i.kind, i.title, i.key, i.identify_state, i.identify_source, i.confidence, i.identify_note,
                        (SELECT group_concat(f.path, char(10)) FROM files f WHERE f.item_id = i.id AND f.missing_since IS NULL), i.candidates, i.year, i.library_id,
                        s.id, s.title, s.key, i.season_no, i.episode_no, i.episode_end
                 FROM items i LEFT JOIN items s ON s.id = i.series_id
                 WHERE {cond} AND EXISTS (SELECT 1 FROM files f WHERE f.missing_since IS NULL AND (f.item_id = i.id OR f.item_id IN (SELECT e.id FROM items e WHERE e.series_id = i.id)))
                 ORDER BY i.library_id, COALESCE(s.sort_title, i.sort_title), i.season_no, i.episode_no"
    ))?;
    let rows = st
        .query_map([], |r| {
            let series_key = r.get::<_, Option<String>>(14)?;
            Ok(json!({"id": r.get::<_, String>(0)?, "kind": r.get::<_, String>(1)?, "title": r.get::<_, String>(2)?, "key": r.get::<_, String>(3)?,
                      "state": r.get::<_, String>(4)?, "source": r.get::<_, Option<String>>(5)?, "confidence": r.get::<_, Option<f64>>(6)?,
                      "note": r.get::<_, Option<String>>(7)?, "files": r.get::<_, Option<String>>(8)?.map(|f| f.split('\n').map(String::from).collect::<Vec<_>>()),
                      "candidates": r.get::<_, Option<String>>(9)?.and_then(|c| serde_json::from_str::<Value>(&c).ok()).unwrap_or(json!([])),
                      "year": r.get::<_, Option<i64>>(10)?, "library": r.get::<_, String>(11)?,
                      "series_id": r.get::<_, Option<String>>(12)?, "series_title": r.get::<_, Option<String>>(13)?,
                      "series_tmdb_id": series_key.as_deref().and_then(super::util::tmdb_tv_id),
                      "season": r.get::<_, Option<i64>>(15)?, "episode": r.get::<_, Option<i64>>(16)?, "episode_end": r.get::<_, Option<i64>>(17)?}))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut verdicts = Vec::new();
    if llm {
        let mut st = c.prepare("SELECT key, model, verdict, accepted, reason, input_tokens, output_tokens, datetime(created_at, 'unixepoch') FROM llm_verdicts ORDER BY created_at DESC LIMIT 200")?;
        verdicts = st
            .query_map([], |r| {
                Ok(json!({"key": r.get::<_, String>(0)?, "model": r.get::<_, String>(1)?, "verdict": r.get::<_, Option<String>>(2)?.and_then(|v| serde_json::from_str::<Value>(&v).ok()),
                          "accepted": r.get::<_, bool>(3)?, "note": r.get::<_, Option<String>>(4)?, "input_tokens": r.get::<_, Option<i64>>(5)?, "output_tokens": r.get::<_, Option<i64>>(6)?, "at": r.get::<_, String>(7)?}))
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }
    let total = rows.len();
    Ok(json!({"items": rows, "total": total, "verdicts": verdicts}))
}

/// A Fix Match choice, checked (`POST /v1/admin/items/{id}/identify`).
pub(crate) struct MatchChoice {
    /// Only this one of the title's files.
    pub(crate) file: Option<String>,
    pub(crate) automatic: bool,
    pub(crate) split: bool,
    /// Make it that type: `movie` or `episode`.
    pub(crate) want: Option<&'static str>,
    pub(crate) tmdb_id: Option<i64>,
    pub(crate) season: Option<i64>,
    pub(crate) episode: Option<i64>,
    /// A part: a number, or None: after the parts the episode has.
    pub(crate) part: Option<Option<i64>>,
}

/// `apply_match`'s answers that aren't a count of files: the file isn't the title's (or the
/// title is a show or season), an episode needs season and episode, an episode needs a show.
pub(crate) const NOT_ITS_FILE: usize = usize::MAX - 2;
pub(crate) const NEEDS_EPISODE: usize = usize::MAX - 1;
pub(crate) const NEEDS_SHOW: usize = usize::MAX;

/// Records a Fix Match choice on the title's files (for the next pass to apply): how many files
/// it was set on (or one of the answers above), and the title's kind (empty: no such item).
pub(crate) fn apply_match(c: &mut Connection, item: &str, m: &MatchChoice) -> Result<(usize, String)> {
    let r2 = item;
    let fid = &m.file;
    let (automatic, split, want, tmdb_id) = (m.automatic, m.split, m.want, m.tmdb_id);
    let (want_season, want_episode, part) = (m.season, m.episode, m.part);
    let row: Option<(String, Option<i64>, Option<i64>)> = c
        .query_row("SELECT kind, season_no, episode_no FROM items WHERE id = ?1", [&r2], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    let Some((kind, season, episode)) = row else {
        return Ok((0, String::new()));
    };
    if let Some(f) = fid {
        let mine: bool = c.query_row(
            "SELECT EXISTS (SELECT 1 FROM files WHERE id = ?1 AND item_id = ?2)",
            rusqlite::params![f, r2],
            |r| r.get(0),
        )?;
        if !mine || !matches!(kind.as_str(), "movie" | "episode") {
            return Ok((NOT_ITS_FILE, kind));
        }
    }
    // The files a choice is for: the title's own (or the one asked for); a show's or
    // season's: its episodes'.
    // (Each names ?3, the file, so the statements take the same parameters.)
    let scope = match (kind.as_str(), fid) {
        (_, Some(_)) => "id = ?3 AND item_id = ?1",
        ("movie" | "episode", None) => "?3 IS NULL AND item_id = ?1",
        _ => "?3 IS NULL AND item_id IN (SELECT id FROM items WHERE series_id = ?1 OR parent_id = ?1)",
    };
    if automatic {
        return Ok((back_to_automatic(c, r2, scope, fid.as_deref())?, kind));
    }
    let set = |key: String, scope: &str| {
        c.execute(
            &format!("UPDATE files SET manual_key = ?2, identified_at = NULL WHERE {scope}"),
            rusqlite::params![r2, key, fid],
        )
    };
    let own = if fid.is_some() {
        "id = ?3 AND item_id = ?1"
    } else {
        "?3 IS NULL AND item_id = ?1"
    };
    let all = if fid.is_some() {
        own
    } else {
        "?3 IS NULL AND (item_id = ?1 OR item_id IN (SELECT id FROM items WHERE series_id = ?1 OR parent_id = ?1))"
    };
    if split {
        return Ok((set("split".into(), own)?, kind));
    }
    let (s, e) = (want_season.or(season).unwrap_or(0), want_episode.or(episode).unwrap_or(0));
    let ep_key = |t: i64| -> rusqlite::Result<String> {
        let base = format!("tmdb:tv:{t}:s{s}e{e}");
        let Some(p) = part else { return Ok(base) };
        let p = match p {
            Some(p) => p,
            // After the parts (or the file) the episode has now, but this title's own.
            None => {
                let (parts, files): (Option<i64>, i64) = c.query_row(
                    &format!(
                        "SELECT MAX(i.part_no), COUNT(DISTINCT i.id) FROM items i JOIN items sr ON sr.id = i.series_id
                                 WHERE i.kind = 'episode' AND sr.key = ?1 AND i.season_no = ?2 AND i.episode_no = ?3 AND i.id != ?4
                                   AND EXISTS (SELECT 1 FROM files f WHERE f.item_id = i.id AND f.role IN {TITLE_ROLES} AND f.missing_since IS NULL)"
                    ),
                    rusqlite::params![format!("tmdb:tv:{t}"), s, e, r2],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                parts.unwrap_or(0).max(files) + 1
            }
        };
        Ok(if p >= 1 { format!("{base}:p{p}") } else { base })
    };
    let n: usize = match (kind.as_str(), want, tmdb_id) {
        // Another type: the files are read as that from now on.
        ("episode" | "series" | "season", Some("movie"), t) => set(t.map_or("local:movie".into(), |t| format!("tmdb:movie:{t}")), all)?,
        ("movie", Some("episode"), None) => set("local:episode".into(), own)?,
        ("movie", Some("episode"), Some(t)) if want_season.is_some() && want_episode.is_some() => set(ep_key(t)?, own)?,
        ("movie", Some("episode"), Some(_)) => return Ok((NEEDS_EPISODE, kind)),
        // An episode keeps its show: it is matched to one of the show's episodes.
        ("episode", _, None) => return Ok((NEEDS_SHOW, kind)),
        ("movie", _, None) => set("local".into(), own)?,
        ("movie", _, Some(t)) => set(format!("tmdb:movie:{t}"), own)?,
        ("episode", _, Some(t)) => set(ep_key(t)?, own)?,
        ("series" | "season", _, t) => c.execute(
            "UPDATE files SET manual_key = ?2, identified_at = NULL WHERE item_id IN (SELECT id FROM items WHERE series_id = ?1 OR parent_id = ?1)",
            rusqlite::params![r2, t.map_or("local".to_string(), |t| format!("tmdb:tv:{t}"))],
        )?,
        _ => 0,
    };
    Ok((n, kind))
}

/// Forgets the Fix Match choices for the files in `scope` (SQL on `files` with ?1 the item, ?3
/// the file) and has them identified again; titles set by hand there (and their seasons and
/// shows, when nothing under them is any more) are no longer kept as they are. Answers how many
/// files had a choice (or, with none, how many titles were still kept by hand).
fn back_to_automatic(c: &mut Connection, item: &str, scope: &str, file: Option<&str>) -> rusqlite::Result<usize> {
    let tx = c.transaction()?;
    let p = rusqlite::params![item, rusqlite::types::Null, file];
    // The titles involved: this one, its files' titles, their seasons and shows.
    let mut items: Vec<String> = vec![item.to_string()];
    {
        let mut st = tx.prepare(&format!(
            "SELECT DISTINCT i.id, i.parent_id, i.series_id FROM items i WHERE i.id IN (SELECT item_id FROM files WHERE {scope})"
        ))?;
        let rows = st.query_map(p, |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        for r in rows {
            let (a, b, c) = r?;
            for x in [Some(a), b, c].into_iter().flatten() {
                if !items.contains(&x) {
                    items.push(x);
                }
            }
        }
    }
    let n = tx.execute(
        &format!("UPDATE files SET manual_key = NULL, identified_at = NULL WHERE ({scope}) AND manual_key IS NOT NULL"),
        p,
    )?;
    // Everything in scope is looked at again (a title's state comes back from its files).
    tx.execute(&format!("UPDATE files SET identified_at = NULL WHERE {scope}"), p)?;
    let mut reset = 0;
    for i in &items {
        reset += tx.execute(
            "UPDATE items SET identify_state = 'matched', identify_source = CASE identify_source WHEN 'admin' THEN 'tmdb' ELSE identify_source END
             WHERE id = ?1 AND identify_state = 'manual'
               AND NOT EXISTS (SELECT 1 FROM files f JOIN items e ON e.id = f.item_id
                               WHERE f.manual_key IS NOT NULL AND (e.id = ?1 OR e.parent_id = ?1 OR e.series_id = ?1))",
            [i],
        )?;
    }
    tx.commit()?;
    Ok(if n > 0 { n } else { reset })
}

/// The title a file is on now (None: no such file, or on none).
pub(crate) fn file_item(c: &Connection, file: String) -> Result<Option<String>> {
    Ok(
        c.query_row("SELECT item_id FROM files WHERE id = ?1", [file], |r| r.get::<_, Option<String>>(0))
            .optional()?
            .flatten(),
    )
}

/// The titles the library has under these keys (`tmdb:movie:<id>`), with how many files each.
pub(crate) fn titles_by_key(c: &Connection, keys: Vec<String>) -> Result<HashMap<String, (String, i64)>> {
    let mut out = HashMap::new();
    let mut st = c.prepare(&format!(
        "SELECT i.id, COUNT(f.id) FROM items i JOIN files f ON f.item_id = i.id AND f.role IN {TITLE_ROLES} AND f.missing_since IS NULL WHERE i.key = ?1 GROUP BY i.id"
    ))?;
    for k in keys {
        if let Some(r) = st
            .query_row([&k], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .optional()?
        {
            out.insert(k, r);
        }
    }
    Ok(out)
}

/// What the library has of each episode of a show's season (`key` the show's): episode number →
/// (its titles, their files).
pub(crate) fn season_files(c: &Connection, key: String, season: i64) -> Result<HashMap<i64, (i64, i64)>> {
    let mut st = c.prepare(&format!(
        "SELECT i.episode_no, COUNT(DISTINCT i.id), COUNT(f.id) FROM items i JOIN items sr ON sr.id = i.series_id
                 JOIN files f ON f.item_id = i.id AND f.role IN {TITLE_ROLES} AND f.missing_since IS NULL
                 WHERE i.kind = 'episode' AND sr.key = ?1 AND i.season_no = ?2 AND i.episode_no IS NOT NULL GROUP BY i.episode_no"
    ))?;
    let rows = st.query_map(rusqlite::params![key, season], |r| {
        Ok((r.get::<_, i64>(0)?, (r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))
    })?;
    Ok(rows.collect::<Result<HashMap<_, _>, _>>()?)
}

/// A title as Fix Match shows it (None: no such item).
pub(crate) fn match_item(c: &Connection, id: &str) -> Result<Option<Value>> {
    Ok(c.query_row(
        "SELECT i.id, i.kind, i.title, i.year, i.key, i.identify_state, i.identify_source, i.identify_note, i.candidates, i.season_no, i.episode_no,
                        s.id, s.title, s.key, (SELECT group_concat(f.path, char(10)) FROM files f WHERE f.missing_since IS NULL AND (f.item_id = i.id OR f.item_id IN (SELECT e.id FROM items e WHERE e.series_id = i.id))),
                        i.part_no
                 FROM items i LEFT JOIN items s ON s.id = i.series_id WHERE i.id = ?1",
        [id],
        |r| {
            let key: String = r.get(4)?;
            let series_key: Option<String> = r.get(13)?;
            Ok(json!({"id": r.get::<_, String>(0)?, "kind": r.get::<_, String>(1)?, "title": r.get::<_, String>(2)?, "year": r.get::<_, Option<i64>>(3)?,
                "state": r.get::<_, String>(5)?, "source": r.get::<_, Option<String>>(6)?, "note": r.get::<_, Option<String>>(7)?,
                "candidates": r.get::<_, Option<String>>(8)?.and_then(|c| serde_json::from_str::<Value>(&c).ok()).unwrap_or(json!([])),
                "season": r.get::<_, Option<i64>>(9)?, "episode": r.get::<_, Option<i64>>(10)?,
                "series_id": r.get::<_, Option<String>>(11)?, "series_title": r.get::<_, Option<String>>(12)?,
                "series_tmdb_id": series_key.as_deref().and_then(super::util::tmdb_tv_id),
                "tmdb_id": super::util::tmdb_movie_id(&key).or_else(|| super::util::tmdb_tv_id(&key)),
                "local": key.starts_with("path:"), "part": r.get::<_, Option<i64>>(15)?,
                "files": r.get::<_, Option<String>>(14)?.map(|f| f.split('\n').take(20).map(String::from).collect::<Vec<_>>()).unwrap_or_default()}))
        },
    )
    .optional()?)
}

/// The choices made by hand on a title's files (and its episodes'), and a movie's or episode's
/// files one by one, largest first.
pub(crate) fn match_files(c: &Connection, id: &str) -> Result<(i64, Vec<Value>)> {
    let manual: i64 = c.query_row(
        "SELECT COUNT(*) FROM files f WHERE f.manual_key IS NOT NULL AND f.missing_since IS NULL
                   AND (f.item_id = ?1 OR f.item_id IN (SELECT id FROM items WHERE series_id = ?1 OR parent_id = ?1))",
        [id],
        |r| r.get(0),
    )?;
    let mut st = c.prepare(&format!(
        "SELECT id, path, size, duration, manual_key FROM files WHERE item_id = ?1 AND role IN {TITLE_ROLES} AND missing_since IS NULL ORDER BY size DESC"
    ))?;
    let files = st
        .query_map([id], |r| {
            let path: String = r.get(1)?;
            let mk: Option<String> = r.get(4)?;
            Ok(json!({"id": r.get::<_, String>(0)?, "name": path.rsplit('/').next(), "size": r.get::<_, i64>(2)?, "duration": r.get::<_, Option<f64>>(3)?,
                      "manual": mk.is_some(), "split": mk.as_deref() == Some("split")}))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok((manual, files))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn badges_come_from_a_healthy_file() {
        let db = crate::db::Db::memory().unwrap();
        let v = db
            .call(|c| {
                let probe = |h: i64| json!({"format": {"format_name": "matroska,webm"}, "streams": [{"index": 0, "codec_type": "video", "codec_name": "h264", "width": h * 16 / 9, "height": h}]}).to_string();
                c.execute_batch(
                    "INSERT INTO files(id, library_id, path, size, mtime, role, first_seen, last_seen, stable_since, item_id)
                       VALUES ('remux', 'm', '/m/remux.mkv', 30, 5, 'broken', 0, 0, 0, 't3'),
                              ('ok', 'm', '/m/ok.mkv', 10, 5, 'main', 0, 0, 0, 't3');",
                )?;
                c.execute("INSERT INTO files_probe(path, size, mtime, probe, probed_at, prober) VALUES ('/m/remux.mkv', 30, 5, ?1, 0, 'p'), ('/m/ok.mkv', 10, 5, ?2, 0, 'p')", rusqlite::params![probe(1080), probe(720)])?;
                let mut items = vec![json!({"id": "t3", "kind": "movie"})];
                decorate(c, &mut items)?;
                Ok(items.remove(0))
            })
            .await
            .unwrap();
        assert_eq!(v["media"]["video"]["height"], 720);
        assert_eq!(v["versions"], 2);
    }

    /// A title that gets a new file later (a new version, a replacement) comes back to
    /// Recently Added and the Date Added sort; the first scan (every file at once), a later extra,
    /// and a later file that is gone again move nothing.
    #[tokio::test]
    async fn a_later_file_brings_a_title_back_to_recently_added() {
        let db = crate::db::Db::memory().unwrap();
        let (movies, ep_rows, sorted, json_added) = db
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO items(id, kind, library_id, key, title, identify_state, added_at, updated_at) VALUES
                       ('m_old', 'movie', 'm', 'k1', 'Old', 'matched', 100, 0),
                       ('m_new', 'movie', 'm', 'k2', 'New', 'matched', 500, 0),
                       ('m_boot', 'movie', 'm', 'k3', 'Two versions at the first scan', 'matched', 50, 0),
                       ('m_ver', 'movie', 'm', 'k4', 'Got a new version', 'matched', 10, 0),
                       ('m_extra', 'movie', 'm', 'k5', 'Got an extra', 'matched', 20, 0),
                       ('m_gone', 'movie', 'm', 'k6', 'New version gone again', 'matched', 30, 0),
                       ('s1', 'series', 't', 'k7', 'Show One', 'matched', 0, 0),
                       ('s2', 'series', 't', 'k8', 'Show Two', 'matched', 0, 0),
                       ('s3', 'series', 't', 'k9', 'Show Three', 'matched', 0, 0);
                     INSERT INTO items(id, kind, library_id, key, series_id, parent_id, season_no, episode_no, title, identify_state, added_at, updated_at) VALUES
                       ('e1', 'episode', 't', 'k10', 's1', 's1', 1, 1, 'Gets a new version', 'matched', 100, 0),
                       ('e2', 'episode', 't', 'k11', 's2', 's2', 1, 1, 'Newer', 'matched', 300, 0),
                       ('e3', 'episode', 't', 'k12', 's3', 's3', 1, 1, 'Two versions at the first scan', 'matched', 200, 0);
                     INSERT INTO files(id, library_id, path, size, mtime, role, first_seen, last_seen, stable_since, item_id, missing_since) VALUES
                       ('f1', 'm', '/m/1', 1, 1, 'main', 1000, 0, 0, 'm_old', NULL),
                       ('f2', 'm', '/m/2', 1, 1, 'main', 1000, 0, 0, 'm_new', NULL),
                       ('f3a', 'm', '/m/3a', 1, 1, 'main', 1000, 0, 0, 'm_boot', NULL),
                       ('f3b', 'm', '/m/3b', 1, 1, 'main', 1000, 0, 0, 'm_boot', NULL),
                       ('f4a', 'm', '/m/4a', 1, 1, 'main', 1000, 0, 0, 'm_ver', NULL),
                       ('f4b', 'm', '/m/4b', 1, 1, 'main', 2000, 0, 0, 'm_ver', NULL),
                       ('f5a', 'm', '/m/5a', 1, 1, 'main', 1000, 0, 0, 'm_extra', NULL),
                       ('f5b', 'm', '/m/5b', 1, 1, 'extra', 3000, 0, 0, 'm_extra', NULL),
                       ('f6a', 'm', '/m/6a', 1, 1, 'main', 1000, 0, 0, 'm_gone', NULL),
                       ('f6b', 'm', '/m/6b', 1, 1, 'main', 3000, 0, 0, 'm_gone', 3500),
                       ('g1a', 't', '/t/1a', 1, 1, 'main', 1000, 0, 0, 'e1', NULL),
                       ('g1b', 't', '/t/1b', 1, 1, 'main', 2500, 0, 0, 'e1', NULL),
                       ('g2', 't', '/t/2', 1, 1, 'main', 1000, 0, 0, 'e2', NULL),
                       ('g3a', 't', '/t/3a', 1, 1, 'main', 1000, 0, 0, 'e3', NULL),
                       ('g3b', 't', '/t/3b', 1, 1, 'main', 1000, 0, 0, 'e3', NULL);",
                )?;
                let movies: Vec<String> = latest_movies(c, None, None, 10)?
                    .iter()
                    .map(|v| v["id"].as_str().unwrap().to_string())
                    .collect();
                let ep_rows = latest_episodes(c, None, None, 10)?;
                let order = browse::order("date_added", true, None, &mut Vec::new());
                let mut st = c.prepare(&format!("SELECT i.id FROM items i WHERE i.kind IN ('movie', 'series') ORDER BY {order}"))?;
                let sorted: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
                let one = query_items(c, None, "i.id = ?", "i.id", &["m_ver".to_string()], 0, 1)?.0;
                Ok((movies, ep_rows, sorted, one[0]["added_at"].clone()))
            })
            .await
            .unwrap();
        // m_ver's new file (2000) puts it first; the rest keep their own order.
        assert_eq!(movies, ["m_ver", "m_new", "m_old", "m_boot", "m_gone", "m_extra"]);
        let ep = |e: &str, s: &str, at: i64| (e.to_string(), s.to_string(), at);
        assert_eq!(ep_rows, [ep("e1", "s1", 2500), ep("e2", "s2", 300), ep("e3", "s3", 200)]);
        assert_eq!(sorted, ["s1", "m_ver", "m_new", "s2", "s3", "m_old", "m_boot", "m_gone", "m_extra"]);
        assert_eq!(json_added, rfc3339(2000));
    }
}
