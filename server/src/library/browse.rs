//! Phase 7, a proper Library: the filters of `GET /v1/items`, `GET /v1/libraries/{id}/facets`,
//! the video summary per title (`item_video`: resolution, HDR, length of the best file), and
//! TMDB collections (stored at metadata time; a background backfill reads the movies that
//! predate it).
//!
//! Filters (all optional; lists are comma-separated and match any of their values):
//! - `genre=Action,Comedy` (any of them; with `genre_match=all`, all of them),
//!   `rating=M,MA 15+` (spaces and case ignored; `none`: no rating),
//!   `studio=<name>` (studios, networks for shows), `collection=<TMDB collection id>`
//! - `year_from=`, `year_to=` (inclusive)
//! - `resolution=4k,1080,720,sd`, `hdr=1` (a show: one of its episodes is)
//! - `unplayed=1` (movies: not watched; shows: an episode isn't), `played=1` (all watched),
//!   `in_progress=1` (movies: a resume point; shows: started and not finished) — a Moonrise token's
//!   watch state
//! - `added_days=N`: added in the last N days (a show: one of its episodes)
//! - `only=id,id`: within these titles (the app's downloads)

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde_json::{json, Value};

use super::{
    images,
    store::{self, visible_sql},
    streams,
    util::now,
    Library, TITLE_ROLES,
};
use crate::userdata;

/// One condition of a query and the facet it belongs to (facet counts leave out their own).
#[derive(Debug, Clone)]
pub struct Cond {
    pub facet: Option<&'static str>,
    pub sql: String,
    pub args: Vec<String>,
}

impl Cond {
    fn new(facet: Option<&'static str>, sql: impl Into<String>, args: Vec<String>) -> Self {
        Cond {
            facet,
            sql: sql.into(),
            args,
        }
    }
}

/// (WHERE, args) of the conditions, leaving out `except`'s.
pub fn join(conds: &[Cond], except: Option<&str>) -> (String, Vec<String>) {
    let mut sql = Vec::new();
    let mut args = Vec::new();
    for c in conds.iter().filter(|c| except.is_none() || c.facet != except) {
        sql.push(c.sql.clone());
        args.extend(c.args.iter().cloned());
    }
    (if sql.is_empty() { "1".into() } else { sql.join(" AND ") }, args)
}

fn list(q: &HashMap<String, String>, key: &str) -> Vec<String> {
    q.get(key)
        .map(|v| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

fn flag(q: &HashMap<String, String>, key: &str) -> bool {
    q.get(key).is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

fn marks(n: usize) -> String {
    vec!["?"; n].join(",")
}

/// A rating as compared: upper case, no spaces ("MA 15+" = "MA15+").
pub fn rating_key(r: &str) -> String {
    r.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase()
}

/// The resolution bucket of a video (as the stream labels: 576p and below are SD).
pub fn res_bucket(width: i64, height: i64) -> &'static str {
    if width <= 1024 && height <= 576 {
        "sd"
    } else if width <= 1280 && height <= 962 {
        "720"
    } else if width <= 2560 && height <= 1440 {
        "1080"
    } else {
        "4k"
    }
}

/// `EXISTS` over a title's video rows: a movie's own, a show's (or season's) visible episodes'.
fn video_exists(alias: &str, cond: &str) -> String {
    format!(
        "(CASE WHEN {alias}.kind IN ('movie', 'episode') THEN EXISTS (SELECT 1 FROM item_video v WHERE v.item_id = {alias}.id AND {cond})
          ELSE EXISTS (SELECT 1 FROM items ve2 JOIN item_video v ON v.item_id = ve2.id WHERE ve2.kind = 'episode'
                       AND (ve2.series_id = {alias}.id OR ve2.parent_id = {alias}.id) AND {} AND {cond}) END)",
        visible_sql("ve2")
    )
}

/// Watched with a resume point and not finished (movies), or started and not finished (shows).
pub fn in_progress_sql(uid: &str) -> String {
    format!(
        "(CASE WHEN i.kind IN ('movie', 'episode') THEN EXISTS (SELECT 1 FROM user_data u WHERE u.user_id = '{uid}' AND u.item_id = i.id AND u.played = 0 AND u.position > 0)
          ELSE EXISTS (SELECT 1 FROM items pe JOIN user_data u ON u.item_id = pe.id AND u.user_id = '{uid}' WHERE pe.kind = 'episode'
                       AND (pe.series_id = i.id OR pe.parent_id = i.id) AND {} AND (u.played = 1 OR u.position > 0))
               AND {} END)",
        visible_sql("pe"),
        userdata::unplayed_sql(uid)
    )
}

fn added_sql(cutoff: i64) -> String {
    format!(
        "(CASE WHEN i.kind IN ('movie', 'episode') THEN {} > {cutoff}
          ELSE EXISTS (SELECT 1 FROM items ae WHERE ae.kind = 'episode' AND (ae.series_id = i.id OR ae.parent_id = i.id) AND {} > {cutoff} AND {}) END)",
        store::added_sql("i"),
        store::added_sql("ae"),
        visible_sql("ae")
    )
}

/// The filter conditions of a request (alias `i`), and notes about ones not applied.
pub fn filters(q: &HashMap<String, String>, user: Option<&str>) -> Result<(Vec<Cond>, Vec<&'static str>), String> {
    let mut out = Vec::new();
    let mut notes = Vec::new();
    // One library's titles (`library=<id>`; libraries have no type).
    let libraries = list(q, "library");
    if !libraries.is_empty() {
        out.push(Cond::new(
            Some("library"),
            format!("i.library_id IN ({})", marks(libraries.len())),
            libraries,
        ));
    }
    let genres = list(q, "genre");
    if !genres.is_empty() {
        let has = |list: &str| format!("EXISTS (SELECT 1 FROM json_each(i.genres) jg WHERE jg.value COLLATE NOCASE IN ({list}))");
        let sql = if q.get("genre_match").is_some_and(|m| m.eq_ignore_ascii_case("all")) {
            // Every one of them.
            genres.iter().map(|_| has("?")).collect::<Vec<_>>().join(" AND ")
        } else {
            has(&marks(genres.len()))
        };
        out.push(Cond::new(Some("genre"), format!("({sql})"), genres));
    }
    for (key, op) in [("year_from", ">="), ("year_to", "<=")] {
        if let Some(v) = q.get(key).filter(|v| !v.is_empty()) {
            let y: i64 = v.parse().map_err(|_| format!("{key}: a year"))?;
            out.push(Cond::new(Some("year"), format!("i.year {op} {y}"), vec![]));
        }
    }
    let ratings = list(q, "rating");
    if !ratings.is_empty() {
        let none = ratings.iter().any(|r| r.eq_ignore_ascii_case("none"));
        let keys: Vec<String> = ratings
            .iter()
            .filter(|r| !r.eq_ignore_ascii_case("none"))
            .map(|r| rating_key(r))
            .collect();
        let mut parts = Vec::new();
        if !keys.is_empty() {
            parts.push(format!("UPPER(REPLACE(i.official_rating, ' ', '')) IN ({})", marks(keys.len())));
        }
        if none {
            parts.push("COALESCE(i.official_rating, '') = ''".into());
        }
        out.push(Cond::new(Some("rating"), format!("({})", parts.join(" OR ")), keys));
    }
    if let Some(studio) = q.get("studio").filter(|s| !s.is_empty()) {
        out.push(Cond::new(
            Some("studio"),
            "EXISTS (SELECT 1 FROM json_each(i.studios) js WHERE js.value = ?)",
            vec![studio.clone()],
        ));
    }
    if let Some(c) = q.get("collection").filter(|s| !s.is_empty()) {
        let id: i64 = c.parse().map_err(|_| "collection: a TMDB collection id".to_string())?;
        out.push(Cond::new(Some("collection"), format!("i.collection_id = {id}"), vec![]));
    }
    let res: Vec<String> = list(q, "resolution")
        .into_iter()
        .map(|r| r.to_lowercase().trim_end_matches('p').to_string())
        .collect();
    if !res.is_empty() {
        if let Some(bad) = res.iter().find(|r| !["4k", "1080", "720", "sd"].contains(&r.as_str())) {
            return Err(format!("resolution: 4k, 1080, 720 or sd (not {bad})"));
        }
        let inlist = res.iter().map(|r| format!("'{r}'")).collect::<Vec<_>>().join(",");
        out.push(Cond::new(
            Some("resolution"),
            video_exists("i", &format!("v.res IN ({inlist})")),
            vec![],
        ));
    }
    if flag(q, "hdr") {
        out.push(Cond::new(Some("hdr"), video_exists("i", "v.hdr = 1"), vec![]));
    }
    let watched: Vec<(&str, fn(&str) -> String)> = vec![
        ("unplayed", userdata::unplayed_sql),
        ("played", |u| format!("NOT {}", userdata::unplayed_sql(u))),
        ("in_progress", in_progress_sql),
    ];
    for (key, sql) in watched {
        if flag(q, key) {
            match user {
                Some(u) => out.push(Cond::new(Some("watched"), sql(u), vec![])),
                None => notes.push("watch-state filters need a Moonrise account token; not applied"),
            }
        }
    }
    if let Some(d) = q.get("added_days").filter(|s| !s.is_empty()) {
        let days: i64 = d.parse().map_err(|_| "added_days: a number of days".to_string())?;
        out.push(Cond::new(Some("added"), added_sql(now() - days.max(0) * 86_400), vec![]));
    }
    let only = list(q, "only");
    if q.contains_key("only") {
        // An empty list is nothing (no downloads), not everything.
        out.push(Cond::new(
            None,
            if only.is_empty() {
                "0".to_string()
            } else {
                format!("i.id IN ({})", marks(only.len()))
            },
            only,
        ));
    }
    notes.dedup();
    Ok((out, notes))
}

/// `ORDER BY` for a sort (and whether it is descending).
pub fn order(sort: &str, desc: bool, user: Option<&str>, notes: &mut Vec<&'static str>) -> String {
    let dir = if desc { "DESC" } else { "ASC" };
    let title = format!("COALESCE(i.sort_title, i.title) COLLATE NOCASE {dir}, i.id {dir}");
    match sort {
        // A show (or season) by its latest episode's arrival, as Home's Latest and the
        // "added in the last N days" filter see it.
        "added" | "date_added" => format!(
            "(CASE WHEN i.kind IN ('series', 'season') THEN COALESCE((SELECT MAX({}) FROM items ae WHERE ae.kind = 'episode'
                AND (ae.series_id = i.id OR ae.parent_id = i.id) AND {}), i.added_at) ELSE {} END) {dir}, {title}",
            store::added_sql("ae"),
            visible_sql("ae"),
            store::added_sql("i")
        ),
        // Ties by title in the same direction, as the usual "ProductionYear,SortName" and
        // "CommunityRating,SortName" sorts do.
        "year" => format!("i.year {dir} NULLS LAST, {title}"),
        "rating" => format!("i.community_rating {dir} NULLS LAST, {title}"),
        "runtime" => format!("COALESCE((SELECT v.duration FROM item_video v WHERE v.item_id = i.id), i.runtime) {dir} NULLS LAST, {title}"),
        "played" | "last_played" | "date_played" => match user {
            Some(u) => format!("{} {dir} NULLS LAST, {title}", userdata::last_played_sql(u)),
            None => {
                notes.push("played sort: watch state needs a Moonrise account token; sorted by title");
                title
            }
        },
        "airing" => format!("i.season_no = 0, i.season_no {dir}, i.episode_no {dir}"),
        _ => title,
    }
}

/// The first letter of a sort title as the index shows it: A-Z, else "#".
pub const LETTER_SQL: &str = "(CASE WHEN UPPER(SUBSTR(COALESCE(i.sort_title, i.title), 1, 1)) BETWEEN 'A' AND 'Z' THEN UPPER(SUBSTR(COALESCE(i.sort_title, i.title), 1, 1)) ELSE '#' END)";

/// The alphabet index of a title-sorted list: (letter, offset of its first title).
pub fn letter_index(c: &Connection, from: &str, sql_where: &str, args: &[String], order: &str) -> rusqlite::Result<Vec<(String, i64)>> {
    let mut st = c.prepare(&format!(
        "SELECT letter, MIN(rn) FROM (SELECT {LETTER_SQL} letter, ROW_NUMBER() OVER (ORDER BY {order}) - 1 rn FROM {from} WHERE {sql_where}) GROUP BY letter ORDER BY 2"
    ))?;
    let rows = st
        .query_map(params_from_iter(args.iter()), |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect();
    rows
}

/// One title as the facets see it.
struct FacetRow {
    id: String,
    genres: Vec<String>,
    studios: Vec<String>,
    year: Option<i64>,
    rating: Option<String>,
    collection: Option<i64>,
    /// How good a title is as a tile's artwork ([`art_quality`]).
    quality: f64,
    votes: i64,
    /// All its backdrops, the main one first.
    backdrops: Vec<String>,
    res: Vec<String>,
    hdr: bool,
    unplayed: bool,
    in_progress: bool,
    recent: bool,
    library: String,
}

/// TMDB's vote counts by title id, read from the TMDB answers the metadata pass cached (no
/// library data changes). Kept for an hour: reading them parses every details answer.
fn tmdb_votes(c: &Connection) -> rusqlite::Result<Arc<HashMap<String, i64>>> {
    static CACHE: Mutex<Option<(String, i64, Arc<HashMap<String, i64>>)>> = Mutex::new(None);
    let path = c.path().unwrap_or("").to_string();
    if !path.is_empty() {
        if let Some((p, at, v)) = CACHE.lock().unwrap().as_ref() {
            if *p == path && now() - at < 3600 {
                return Ok(v.clone());
            }
        }
    }
    // `url` is the key: a range on '/movie/<id>?' finds the details answer whatever its query.
    let mut st = c.prepare(
        "SELECT i.id, (SELECT max(json_extract(t.body, '$.vote_count')) FROM tmdb_cache t
                       WHERE t.status = 200 AND t.url > x.p || '?' AND t.url < x.p || '?~')
         FROM items i JOIN (SELECT id, '/' || (CASE kind WHEN 'movie' THEN 'movie' ELSE 'tv' END) || '/' || json_extract(provider_ids, '$.Tmdb') AS p
                            FROM items WHERE kind IN ('movie', 'series') AND json_extract(provider_ids, '$.Tmdb') IS NOT NULL) x ON x.id = i.id",
    )?;
    let v: HashMap<String, i64> = st
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?)))?
        .filter_map(std::result::Result::ok)
        .filter_map(|(id, n)| n.map(|n| (id, n)))
        .collect();
    let v = Arc::new(v);
    if !path.is_empty() {
        *CACHE.lock().unwrap() = Some((path, now(), v.clone()));
    }
    Ok(v)
}

/// A tile's artwork should come from a well-known, well-rated title: the rating weighed by how
/// many voted (a title with one 10/10 vote ranks like an average one), plus a little for fame.
pub fn art_quality(rating: Option<f64>, votes: i64) -> f64 {
    const M: f64 = 500.0; // votes before a rating counts in full
    const MEAN: f64 = 6.0;
    let v = votes.max(0) as f64;
    let r = rating.filter(|r| *r > 0.0).unwrap_or(MEAN);
    (v * r + M * MEAN) / (v + M) + 0.25 * (v + 1.0).log10()
}

fn facet_rows(
    c: &Connection,
    base: &str,
    args: &[String],
    user: Option<&str>,
    recent_cutoff: i64,
    votes: &HashMap<String, i64>,
) -> rusqlite::Result<Vec<FacetRow>> {
    let (unplayed, inprog) = match user {
        Some(u) => (userdata::unplayed_sql(u), in_progress_sql(u)),
        None => ("0".into(), "0".into()),
    };
    let sql = format!(
        "SELECT i.genres, i.studios, i.year, i.official_rating, i.collection_id, i.community_rating, i.images,
                (CASE WHEN i.kind IN ('movie', 'episode') THEN (SELECT v.res || ':' || v.hdr FROM item_video v WHERE v.item_id = i.id)
                 ELSE (SELECT group_concat(DISTINCT v.res || ':' || v.hdr) FROM items ve2 JOIN item_video v ON v.item_id = ve2.id
                       WHERE ve2.kind = 'episode' AND (ve2.series_id = i.id OR ve2.parent_id = i.id) AND {}) END),
                {unplayed}, {inprog}, {}, i.id, i.library_id
         FROM items i WHERE {base}",
        visible_sql("ve2"),
        added_sql(recent_cutoff)
    );
    let mut st = c.prepare(&sql)?;
    let strs = |s: Option<String>| -> Vec<String> { s.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok()).unwrap_or_default() };
    let rows = st
        .query_map(params_from_iter(args.iter()), |r| {
            let video: Option<String> = r.get(7)?;
            let mut res = Vec::new();
            let mut hdr = false;
            for part in video.as_deref().unwrap_or("").split(',').filter(|p| !p.is_empty()) {
                let (rv, h) = part.split_once(':').unwrap_or((part, "0"));
                if !rv.is_empty() && !res.iter().any(|x| x == rv) {
                    res.push(rv.to_string());
                }
                hdr |= h == "1";
            }
            let images: Option<String> = r.get(6)?;
            let id: String = r.get(11)?;
            let n = votes.get(&id).copied().unwrap_or(0);
            let backdrops = images
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .and_then(|v| {
                    v["backdrop"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|b| b.as_str().map(String::from)).collect())
                })
                .unwrap_or_default();
            Ok(FacetRow {
                quality: art_quality(r.get(5)?, n),
                votes: n,
                id,
                genres: strs(r.get(0)?),
                studios: strs(r.get(1)?),
                year: r.get(2)?,
                rating: r.get::<_, Option<String>>(3)?.filter(|s| !s.trim().is_empty()),
                collection: r.get(4)?,
                backdrops,
                res,
                hdr,
                unplayed: r.get(8)?,
                in_progress: r.get(9)?,
                recent: r.get(10)?,
                library: r.get(12)?,
            })
        })?
        .collect();
    rows
}

/// A title that can lend a tile its artwork.
#[derive(Clone)]
struct ArtCandidate {
    item: String,
    quality: f64,
    votes: i64,
    backdrops: Vec<String>,
}

/// name → (count, the titles with artwork).
#[derive(Default)]
struct Tally(BTreeMap<String, (i64, Vec<ArtCandidate>)>);

impl Tally {
    /// Counts a title under each of its values once (its lists can repeat a value).
    fn add_all(&mut self, keys: &[String], row: &FacetRow) {
        let mut seen = HashSet::new();
        for k in keys {
            if seen.insert(k.as_str()) {
                self.add(k, row);
            }
        }
    }

    fn add(&mut self, key: &str, row: &FacetRow) {
        let e = self.0.entry(key.to_string()).or_insert((0, Vec::new()));
        e.0 += 1;
        if !row.backdrops.is_empty() {
            e.1.push(ArtCandidate {
                item: row.id.clone(),
                quality: row.quality,
                votes: row.votes,
                backdrops: row.backdrops.clone(),
            });
        }
    }

    /// By count, then name (the order the app shows them in).
    fn sorted(self) -> Vec<(String, i64, Vec<ArtCandidate>)> {
        let mut v: Vec<_> = self.0.into_iter().map(|(k, (n, arts))| (k, n, arts)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        v
    }
}

/// Hands out a page's tile artwork so no two tiles show the same picture: in tile order, each
/// tile takes the main backdrop of its best title not used yet; failing that, another backdrop
/// of its titles not used yet; failing that, none (the tile draws its icon).
#[derive(Default)]
struct ArtPicker {
    images: HashSet<String>,
    items: HashSet<String>,
}

impl ArtPicker {
    fn pick(&mut self, mut cands: Vec<ArtCandidate>) -> Option<String> {
        cands.sort_by(|a, b| {
            b.quality
                .total_cmp(&a.quality)
                .then(b.votes.cmp(&a.votes))
                .then_with(|| a.item.cmp(&b.item))
        });
        let fresh = cands
            .iter()
            .find(|c| !self.items.contains(&c.item) && !self.images.contains(&c.backdrops[0]))
            .map(|c| (&c.item, &c.backdrops[0]));
        let (item, art) = fresh.or_else(|| {
            cands
                .iter()
                .find_map(|c| c.backdrops.iter().find(|b| !self.images.contains(*b)).map(|b| (&c.item, b)))
        })?;
        self.items.insert(item.clone());
        self.images.insert(art.clone());
        Some(art.clone())
    }

    /// A picture of the tile's own (a collection's): kept when no earlier tile has it.
    fn claim(&mut self, art: &str) -> bool {
        self.images.insert(art.to_string())
    }

    /// Ordered tiles with their candidates → their artwork.
    fn assign<T>(list: Vec<(T, i64, Vec<ArtCandidate>)>) -> Vec<(T, i64, Option<String>)> {
        let mut p = ArtPicker::default();
        list.into_iter().map(|(k, n, c)| (k, n, p.pick(c))).collect()
    }
}

/// The facets of a library under the request's filters: each facet's counts leave out its own
/// filter (so picking a second genre still shows the others' counts).
pub fn facets(c: &Connection, base: &[Cond], filters: &[Cond], user: Option<&str>, recent_days: i64) -> Result<Value> {
    let all: Vec<Cond> = base.iter().chain(filters.iter()).cloned().collect();
    let cutoff = now() - recent_days * 86_400;
    let votes = tmdb_votes(c)?;
    // One query per distinct condition set (with no filters, one in all).
    let mut cache: HashMap<String, std::rc::Rc<Vec<FacetRow>>> = HashMap::new();
    let mut rows_for = |except: Option<&str>| -> rusqlite::Result<std::rc::Rc<Vec<FacetRow>>> {
        let (w, a) = join(&all, except);
        let key = format!("{w}\u{1}{}", a.join("\u{2}"));
        if let Some(r) = cache.get(&key) {
            return Ok(r.clone());
        }
        let r = std::rc::Rc::new(facet_rows(c, &w, &a, user, cutoff, &votes)?);
        cache.insert(key, r.clone());
        Ok(r)
    };
    let total = rows_for(Some("\u{0}"))?.len();

    let mut genres = Tally::default();
    for r in rows_for(Some("genre"))?.iter() {
        genres.add_all(&r.genres, r);
    }
    let mut studios = Tally::default();
    for r in rows_for(Some("studio"))?.iter() {
        studios.add_all(&r.studios, r);
    }
    let mut years: BTreeMap<i64, i64> = BTreeMap::new();
    let mut decades = Tally::default();
    for r in rows_for(Some("year"))?.iter() {
        if let Some(y) = r.year {
            *years.entry(y).or_default() += 1;
            decades.add(&format!("{:04}", y - y.rem_euclid(10)), r);
        }
    }
    // Ratings: grouped without spaces, shown as their most common spelling.
    let mut ratings: BTreeMap<String, (i64, HashMap<String, i64>)> = BTreeMap::new();
    for r in rows_for(Some("rating"))?.iter() {
        let (key, shown) = match &r.rating {
            Some(s) => (rating_key(s), s.trim().to_string()),
            None => ("NONE".to_string(), String::new()),
        };
        let e = ratings.entry(key).or_default();
        e.0 += 1;
        *e.1.entry(shown).or_default() += 1;
    }
    let mut collections = Tally::default();
    for r in rows_for(Some("collection"))?.iter() {
        if let Some(id) = r.collection {
            collections.add(&id.to_string(), r);
        }
    }
    let mut res: BTreeMap<String, i64> = BTreeMap::new();
    for r in rows_for(Some("resolution"))?.iter() {
        for x in &r.res {
            *res.entry(x.clone()).or_default() += 1;
        }
    }
    let hdr = rows_for(Some("hdr"))?.iter().filter(|r| r.hdr).count();
    let w = rows_for(Some("watched"))?;
    let watched = user.map(|_| {
        json!({
            "all": w.len(),
            "unplayed": w.iter().filter(|r| r.unplayed).count(),
            "in_progress": w.iter().filter(|r| r.in_progress).count(),
            "played": w.iter().filter(|r| !r.unplayed).count(),
        })
    });
    let recent = rows_for(Some("added"))?.iter().filter(|r| r.recent).count();
    // Libraries (no type: each holds any titles), in their order.
    let mut by_library: HashMap<String, i64> = HashMap::new();
    for r in rows_for(Some("library"))?.iter() {
        *by_library.entry(r.library.clone()).or_default() += 1;
    }
    let mut libraries = Vec::new();
    {
        let mut st = c.prepare("SELECT id, name FROM libraries ORDER BY sort")?;
        for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, name) = row?;
            libraries.push(json!({"id": id, "name": name, "count": by_library.get(&id).copied().unwrap_or(0)}));
        }
    }

    let named = |v: Vec<(String, i64, Option<String>)>| -> Vec<Value> {
        v.into_iter()
            .map(|(name, count, art)| json!({"name": name, "count": count, "backdrop": art}))
            .collect()
    };
    // Decades show newest first: their artwork is handed out in that order.
    let mut decade_tiles: Vec<(i64, i64, Vec<ArtCandidate>)> = decades
        .sorted()
        .into_iter()
        .map(|(d, n, a)| (d.parse::<i64>().unwrap_or(0), n, a))
        .collect();
    decade_tiles.sort_by_key(|d| std::cmp::Reverse(d.0));
    let decade_list: Vec<Value> = ArtPicker::assign(decade_tiles)
        .into_iter()
        .map(|(d, n, art)| json!({"decade": d, "count": n, "backdrop": art}))
        .collect();
    let mut rating_list: Vec<(String, i64, String)> = ratings
        .into_iter()
        .map(|(key, (n, spellings))| {
            let shown = spellings
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
                .map(|s| s.0)
                .unwrap_or_default();
            (if key == "NONE" { "none".into() } else { shown }, n, key)
        })
        .collect();
    // By count, "none" last.
    rating_list.sort_by(|a, b| {
        (a.0 == "none")
            .cmp(&(b.0 == "none"))
            .then(b.1.cmp(&a.1))
            .then_with(|| a.0.cmp(&b.0))
    });
    // Collections: by count, then name; each keeps its own backdrop unless an earlier one has
    // that picture, else takes one of its titles'.
    let mut colls = Vec::new();
    {
        let mut st = c.prepare("SELECT name, poster, backdrop FROM collections WHERE id = ?1")?;
        for (id, n, cands) in collections.0.into_iter().map(|(k, (n, a))| (k.parse::<i64>().unwrap_or(0), n, a)) {
            if let Some((name, poster, backdrop)) = st
                .query_row([id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })
                .optional()?
            {
                colls.push((id, name, n, poster, backdrop, cands));
            }
        }
    }
    colls.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.cmp(&b.1)));
    let mut picker = ArtPicker::default();
    let coll_list: Vec<Value> = colls
        .into_iter()
        .map(|(id, name, n, poster, own, cands)| {
            let backdrop = match own {
                Some(b) if picker.claim(&b) => Some(b),
                _ => picker.pick(cands),
            };
            json!({"id": id, "name": name, "count": n, "poster": poster, "backdrop": backdrop})
        })
        .collect();
    let order = ["4k", "1080", "720", "sd"];
    Ok(json!({
        "total": total,
        "genres": named(ArtPicker::assign(genres.sorted())),
        "studios": named(ArtPicker::assign(studios.sorted())),
        "years": years.into_iter().rev().map(|(y, n)| json!({"year": y, "count": n})).collect::<Vec<_>>(),
        "decades": decade_list,
        "ratings": rating_list.into_iter().map(|(name, n, _)| json!({"name": name, "count": n})).collect::<Vec<_>>(),
        "collections": coll_list,
        "resolutions": order.iter().filter_map(|k| res.get(*k).map(|n| json!({"name": k, "count": n}))).collect::<Vec<_>>(),
        "hdr": hdr,
        "watched": watched,
        "recently_added": {"days": recent_days, "count": recent},
        "libraries": libraries,
    }))
}

// MARK: - item_video

/// Brings `item_video` up to date with the best files' probes; returns how many rows changed.
pub fn refresh_item_video(c: &mut Connection) -> Result<usize> {
    let best: Vec<(String, String, String, i64, i64, Option<f64>)> = {
        let mut st = c.prepare(&format!(
            "SELECT item_id, id, path, size, mtime, duration FROM (
                 SELECT f.item_id, f.id, f.path, f.size, f.mtime, f.duration,
                        ROW_NUMBER() OVER (PARTITION BY f.item_id ORDER BY f.size DESC, f.id) rn
                 FROM files f JOIN items it ON it.id = f.item_id AND it.kind IN ('movie', 'episode')
                 WHERE f.role IN {TITLE_ROLES} AND f.missing_since IS NULL)
             WHERE rn = 1"
        ))?;
        let rows = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let known: HashMap<String, String> = {
        let mut st = c.prepare("SELECT item_id, stamp FROM item_video")?;
        let rows = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<HashMap<_, _>, _>>()?;
        rows
    };
    let tx = c.transaction()?;
    let mut changed = 0;
    {
        let mut probe = tx.prepare("SELECT probe FROM files_probe WHERE path = ?1 AND size = ?2 AND mtime = ?3")?;
        let mut put = tx.prepare(
            "INSERT OR REPLACE INTO item_video(item_id, file_id, stamp, width, height, res, hdr, range, duration) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        for (item, file, path, size, mtime, duration) in &best {
            let stamp = format!("{path}|{size}|{mtime}");
            if known.get(item) == Some(&stamp) {
                continue;
            }
            let Some(p) = probe.query_row(params![path, size, mtime], |r| r.get::<_, String>(0)).optional()? else {
                continue;
            };
            let Ok(pv) = serde_json::from_str::<Value>(&p) else { continue };
            let (w, h, res, hdr, range) = video_of(&pv);
            put.execute(params![item, file, stamp, w, h, res, hdr as i64, range, duration])?;
            changed += 1;
        }
    }
    // Titles that have no file any more.
    let gone = tx.execute(
        &format!("DELETE FROM item_video WHERE NOT EXISTS (SELECT 1 FROM files f WHERE f.item_id = item_video.item_id AND f.role IN {TITLE_ROLES} AND f.missing_since IS NULL)"),
        [],
    )?;
    tx.commit()?;
    Ok(changed + gone)
}

/// (width, height, resolution bucket, HDR, range) of a probe's main video stream.
fn video_of(probe: &Value) -> (Option<i64>, Option<i64>, Option<&'static str>, bool, Option<String>) {
    let s = streams::derive(std::path::Path::new(""), probe, &[]);
    match s.iter().find(|x| x.kind == "Video") {
        Some(v) => {
            let (w, h) = (v.width.unwrap_or(0), v.height.unwrap_or(0));
            let res = (w > 0 || h > 0).then(|| res_bucket(w, h));
            let range = v.video_range_type.clone();
            let hdr = range.as_deref().is_some_and(|r| r != "SDR" && r != "Unknown");
            (v.width, v.height, res, hdr, range)
        }
        None => (None, None, None, false, None),
    }
}

// MARK: - Collections

/// Stores a movie's collection from TMDB's movie answer (`belongs_to_collection`).
pub fn store_collection(c: &Connection, item: &str, movie: &Value) -> Result<()> {
    let col = &movie["belongs_to_collection"];
    let id = col["id"].as_i64();
    if let (Some(id), Some(name)) = (id, col["name"].as_str()) {
        let poster = images::register(c, col["poster_path"].as_str(), "poster")?;
        let backdrop = images::register(c, col["backdrop_path"].as_str(), "backdrop")?;
        c.execute(
            "INSERT INTO collections(id, name, poster, backdrop, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, poster = COALESCE(excluded.poster, collections.poster),
                                           backdrop = COALESCE(excluded.backdrop, collections.backdrop), updated_at = excluded.updated_at",
            params![id, name, poster, backdrop, now()],
        )?;
    }
    c.execute(
        "UPDATE items SET collection_id = ?2, collection_at = ?3 WHERE id = ?1",
        params![item, id.filter(|_| col["name"].is_string()), now()],
    )?;
    Ok(())
}

impl Library {
    /// `item_video` after probing or a pass (on the blocking pool).
    pub async fn refresh_video_summary(&self) {
        match self.db.call(refresh_item_video).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(rows = n, "video summary updated"),
            Err(e) => tracing::warn!("video summary: {e:#}"),
        }
    }

    /// Reads the collection of every movie that predates collections, one TMDB answer at a
    /// time (the stored answer when it's under a year old, else a request), gently: at most
    /// ~3 a second on top of the client's own spacing.
    pub async fn backfill_collections(&self) -> Result<usize> {
        let Some(tmdb) = self.tmdb() else { return Ok(0) };
        let mut done = 0;
        loop {
            let todo: Vec<(String, String)> = self
                .db
                .call(|c| {
                    let mut st = c.prepare(
                        "SELECT id, key FROM items WHERE kind = 'movie' AND key LIKE 'tmdb:movie:%' AND collection_at IS NULL LIMIT 25",
                    )?;
                    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
                    Ok(rows)
                })
                .await?;
            if todo.is_empty() {
                break;
            }
            for (id, key) in todo {
                let Some(tid) = key.rsplit(':').next().and_then(|s| s.parse::<i64>().ok()) else {
                    continue;
                };
                let d = match tmdb.movie(tid, 365 * super::tmdb::DAY).await {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::warn!(item = id, "collection backfill: {e:#}");
                        // Try again on the next start.
                        return Ok(done);
                    }
                };
                self.db.call(move |c| store_collection(c, &id, &d)).await?;
                done += 1;
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
        }
        if done > 0 {
            tracing::info!(movies = done, "collections backfilled");
        }
        Ok(done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets() {
        assert_eq!(res_bucket(3840, 2160), "4k");
        assert_eq!(res_bucket(3840, 1600), "4k");
        assert_eq!(res_bucket(1920, 800), "1080");
        assert_eq!(res_bucket(1440, 1080), "1080");
        assert_eq!(res_bucket(1280, 720), "720");
        assert_eq!(res_bucket(720, 576), "sd");
        assert_eq!(res_bucket(640, 480), "sd");
    }

    #[test]
    fn ratings() {
        assert_eq!(rating_key("MA 15+"), rating_key("ma15+"));
    }

    #[test]
    fn joins_leave_out_a_facet() {
        let conds = vec![
            Cond::new(None, "a = ?", vec!["1".into()]),
            Cond::new(Some("genre"), "g IN (?)", vec!["x".into()]),
            Cond::new(Some("year"), "y > 1", vec![]),
        ];
        assert_eq!(join(&conds, Some("genre")), ("a = ? AND y > 1".to_string(), vec!["1".to_string()]));
        assert_eq!(join(&conds, None).1, vec!["1".to_string(), "x".to_string()]);
        assert_eq!(join(&[], None).0, "1");
    }

    #[test]
    fn bad_filters() {
        let q = |k: &str, v: &str| HashMap::from([(k.to_string(), v.to_string())]);
        assert!(filters(&q("resolution", "8k"), None).is_err());
        assert!(filters(&q("year_from", "x"), None).is_err());
        let (c, notes) = filters(&q("played", "1"), None).unwrap();
        assert!(c.is_empty() && !notes.is_empty());
        let (c, _) = filters(&q("resolution", "4K,1080p"), None).unwrap();
        assert!(c[0].sql.contains("'4k','1080'"));
    }

    fn probe(w: i64, h: i64, transfer: &str) -> String {
        json!({"streams": [{"index": 0, "codec_type": "video", "codec_name": "hevc", "width": w, "height": h, "color_transfer": transfer}], "format": {}}).to_string()
    }

    /// Movies m1..m4 and a show with two episodes; files, probes, watch state.
    fn fixture(c: &mut Connection) {
        c.execute("INSERT INTO libraries(id, name, kind, roots, sort) VALUES ('movies', 'Movies', 'movies', '[]', 0), ('shows', 'Shows', 'shows', '[]', 1)", []).unwrap();
        let t = now();
        let item = |id: &str,
                    kind: &str,
                    lib: &str,
                    title: &str,
                    year: i64,
                    rating: Option<&str>,
                    genres: &str,
                    studios: &str,
                    series: Option<&str>,
                    added: i64| {
            c.execute(
                "INSERT INTO items(id, kind, library_id, key, title, sort_title, year, official_rating, genres, studios, series_id, parent_id, community_rating, identify_state, added_at, updated_at, images)
                 VALUES (?1, ?2, ?3, ?1, ?4, ?4, ?5, ?6, ?7, ?8, ?9, ?9, 7.0, 'matched', ?10, 0, '{\"backdrop\":[\"bd\"]}')",
                params![id, kind, lib, title, year, rating, genres, studios, series, added],
            )
            .unwrap();
        };
        item(
            "m1",
            "movie",
            "movies",
            "Alien",
            1979,
            Some("MA 15+"),
            r#"["Horror","Science Fiction"]"#,
            r#"["Fox"]"#,
            None,
            t - 400 * 86_400,
        );
        item(
            "m2",
            "movie",
            "movies",
            "Aliens",
            1986,
            Some("MA15+"),
            r#"["Action","Science Fiction"]"#,
            r#"["Fox"]"#,
            None,
            t - 400 * 86_400,
        );
        // TMDB listed its studio twice: one title all the same.
        item(
            "m3",
            "movie",
            "movies",
            "Brave",
            2012,
            Some("PG"),
            r#"["Animation"]"#,
            r#"["Pixar","Pixar"]"#,
            None,
            t - 5 * 86_400,
        );
        item(
            "m4",
            "movie",
            "movies",
            "Zodiac",
            2007,
            None,
            r#"["Crime"]"#,
            r#"["Paramount"]"#,
            None,
            t - 400 * 86_400,
        );
        item(
            "sh",
            "series",
            "shows",
            "Severance",
            2022,
            Some("MA 15+"),
            r#"["Drama"]"#,
            r#"["Apple TV+"]"#,
            None,
            t - 400 * 86_400,
        );
        item(
            "e1",
            "episode",
            "shows",
            "Good News",
            2022,
            None,
            "[]",
            "[]",
            Some("sh"),
            t - 400 * 86_400,
        );
        item(
            "e2",
            "episode",
            "shows",
            "Half Loop",
            2022,
            None,
            "[]",
            "[]",
            Some("sh"),
            t - 2 * 86_400,
        );
        // Added to the library after Severance, but its latest episode is older.
        item(
            "sh2",
            "series",
            "shows",
            "Andor",
            2022,
            Some("M"),
            r#"["Drama"]"#,
            r#"["Disney+"]"#,
            None,
            t - 86_400,
        );
        item(
            "e3",
            "episode",
            "shows",
            "Kassa",
            2022,
            None,
            "[]",
            "[]",
            Some("sh2"),
            t - 300 * 86_400,
        );
        let file = |id: &str, item: &str, size: i64, w: i64, h: i64, transfer: &str| {
            let path = format!("/media/{id}.mkv");
            c.execute(
                "INSERT INTO files(id, library_id, path, size, mtime, duration, role, item_id, first_seen, last_seen, stable_since) VALUES (?1, 'movies', ?2, ?3, 1, 6000, 'main', ?4, 0, 0, 0)",
                params![id, path, size, item],
            )
            .unwrap();
            c.execute(
                "INSERT INTO files_probe(path, size, mtime, probe, probed_at) VALUES (?1, ?2, 1, ?3, 0)",
                params![path, size, probe(w, h, transfer)],
            )
            .unwrap();
        };
        file("f1", "m1", 10, 3840, 1600, "smpte2084");
        file("f1b", "m1", 5, 1920, 800, "bt709"); // a smaller second version: not the one counted
        file("f2", "m2", 10, 1920, 1040, "bt709");
        file("f3", "m3", 10, 1280, 720, "bt709");
        file("f4", "m4", 10, 720, 576, "bt709");
        file("fe1", "e1", 10, 3840, 2160, "arib-std-b67");
        file("fe2", "e2", 10, 1920, 1080, "bt709");
        file("fe3", "e3", 10, 1280, 720, "bt709");
        c.execute(
            "INSERT INTO users(id, name, password_hash, created_at) VALUES ('u1', 'sam', 'x', 0)",
            [],
        )
        .unwrap();
        // m1 watched, m2 half-way, e1 watched (the show: in progress).
        c.execute("INSERT INTO user_data(user_id, item_id, position, played, updated_at) VALUES ('u1', 'm1', 0, 1, 0), ('u1', 'm2', 900, 0, 0), ('u1', 'e1', 0, 1, 0)", []).unwrap();
        store_collection(c, "m1", &json!({"belongs_to_collection": {"id": 8091, "name": "Alien Collection"}})).unwrap();
        store_collection(c, "m2", &json!({"belongs_to_collection": {"id": 8091, "name": "Alien Collection"}})).unwrap();
        store_collection(c, "m3", &json!({"belongs_to_collection": null})).unwrap();
    }

    fn ids(c: &Connection, lib: &str, kind: &str, q: &[(&str, &str)], sort: &str) -> Vec<String> {
        let q: HashMap<String, String> = q.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let (conds, _) = filters(&q, Some("u1")).unwrap();
        let (w, a) = join(&conds, None);
        let sql = format!(
            "SELECT i.id FROM items i WHERE i.library_id = '{lib}' AND i.kind = '{kind}' AND {w} ORDER BY {}",
            order(sort, false, Some("u1"), &mut vec![])
        );
        let mut st = c.prepare(&sql).unwrap();
        let rows = st
            .query_map(params_from_iter(a.iter()), |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows
    }

    #[test]
    fn art_quality_wants_votes() {
        // One 10/10 vote (a fitness DVD) ranks below a famous, well-rated film and an average one.
        let dvd = art_quality(Some(10.0), 1);
        assert!(art_quality(Some(8.5), 37_000) > art_quality(Some(7.0), 900));
        assert!(art_quality(Some(6.4), 3_000) > dvd);
        assert!(art_quality(None, 0) < art_quality(Some(7.5), 200));
    }

    #[test]
    fn art_picker_never_repeats() {
        let cand = |item: &str, q: f64, b: &[&str]| ArtCandidate {
            item: item.into(),
            quality: q,
            votes: 0,
            backdrops: b.iter().map(ToString::to_string).collect(),
        };
        let big = vec![cand("a", 9.0, &["a1", "a2"]), cand("b", 8.0, &["b1"]), cand("c", 7.0, &["c1"])];
        let tiles = vec![
            ("Adventure", 3, big.clone()),
            ("Action", 3, big.clone()),
            ("Fantasy", 3, big.clone()),
            ("Small", 1, vec![cand("a", 9.0, &["a1", "a2"])]),
            ("Tiny", 1, vec![cand("b", 8.0, &["b1"])]),
            ("Bare", 1, vec![]),
        ];
        let got: Vec<_> = ArtPicker::assign(tiles).into_iter().map(|(k, _, a)| (k, a)).collect();
        let s = |x: &str| Some(x.to_string());
        assert_eq!(
            got,
            vec![
                ("Adventure", s("a1")),
                ("Action", s("b1")),
                ("Fantasy", s("c1")),
                ("Small", s("a2")),
                ("Tiny", None),
                ("Bare", None)
            ]
        );
    }

    #[tokio::test]
    async fn filters_facets_and_index() {
        let db = crate::db::Db::memory().unwrap();
        db.call(|c| {
            fixture(c);
            assert_eq!(refresh_item_video(c)?, 7);
            assert_eq!(refresh_item_video(c)?, 0, "unchanged files aren't read again");
            let m = |q: &[(&str, &str)]| ids(c, "movies", "movie", q, "title");
            assert_eq!(m(&[]), ["m1", "m2", "m3", "m4"]);
            assert_eq!(m(&[("genre", "science fiction,Animation")]), ["m1", "m2", "m3"]);
            assert_eq!(
                m(&[("genre", "science fiction,Horror"), ("genre_match", "all")]),
                ["m1"],
                "all of them"
            );
            assert_eq!(m(&[("genre", "science fiction,Horror"), ("genre_match", "any")]), ["m1", "m2"]);
            assert_eq!(m(&[("year_from", "1980"), ("year_to", "2009")]), ["m2", "m4"]);
            assert_eq!(m(&[("rating", "MA 15+")]), ["m1", "m2"], "spellings with and without the space");
            assert_eq!(m(&[("rating", "none,pg")]), ["m3", "m4"]);
            assert_eq!(m(&[("resolution", "4k")]), ["m1"], "the largest file counts");
            assert_eq!(m(&[("resolution", "1080,720")]), ["m2", "m3"]);
            assert_eq!(m(&[("resolution", "sd")]), ["m4"]);
            assert_eq!(m(&[("hdr", "1")]), ["m1"]);
            assert_eq!(m(&[("played", "1")]), ["m1"]);
            assert_eq!(m(&[("unplayed", "1")]), ["m2", "m3", "m4"]);
            assert_eq!(m(&[("in_progress", "1")]), ["m2"]);
            assert_eq!(m(&[("collection", "8091")]), ["m1", "m2"]);
            assert_eq!(m(&[("studio", "Fox")]), ["m1", "m2"]);
            assert_eq!(m(&[("added_days", "30")]), ["m3"]);
            assert_eq!(m(&[("only", "m4,m2")]), ["m2", "m4"]);
            assert!(m(&[("only", "")]).is_empty());
            assert_eq!(ids(c, "movies", "movie", &[], "year"), ["m1", "m2", "m4", "m3"]);
            // Shows: through their episodes.
            let s = |q: &[(&str, &str)]| ids(c, "shows", "series", q, "title");
            assert_eq!(s(&[("hdr", "1")]), ["sh"]);
            assert_eq!(s(&[("resolution", "1080")]), ["sh"]);
            assert!(s(&[("resolution", "sd")]).is_empty());
            assert_eq!(s(&[("in_progress", "1")]), ["sh"]);
            assert!(s(&[("played", "1")]).is_empty());
            assert_eq!(s(&[("added_days", "7")]), ["sh"]);
            // Date Added: a show by its latest episode (Severance's came two days ago).
            assert_eq!(ids(c, "shows", "series", &[], "added"), ["sh2", "sh"]);
            assert_eq!(m(&[("studio", "Pixar")]), ["m3"]);
            // Facets: genre counts ignore the genre filter, the rest follow it.
            let base = vec![Cond::new(None, "i.kind = 'movie' AND i.library_id = 'movies'", vec![])];
            let q = HashMap::from([("genre".to_string(), "Science Fiction".to_string())]);
            let (f, _) = filters(&q, Some("u1")).unwrap();
            let v = facets(c, &base, &f, Some("u1"), 30)?;
            assert_eq!(v["total"], 2);
            assert_eq!(v["genres"].as_array().unwrap().len(), 5);
            assert_eq!(v["genres"][0], json!({"name": "Science Fiction", "count": 2, "backdrop": "bd"}));
            assert_eq!(v["ratings"], json!([{"name": "MA 15+", "count": 2}]));
            assert_eq!(v["collections"][0]["name"], "Alien Collection");
            assert_eq!(v["collections"][0]["count"], 2);
            assert_eq!(v["resolutions"], json!([{"name": "4k", "count": 1}, {"name": "1080", "count": 1}]));
            assert_eq!(v["hdr"], 1);
            assert_eq!(v["watched"], json!({"all": 2, "unplayed": 1, "in_progress": 1, "played": 1}));
            assert_eq!(v["decades"][0]["decade"], 1980);
            let v = facets(c, &base, &[], Some("u1"), 30)?;
            assert_eq!(v["total"], 4);
            assert_eq!(
                v["studios"].as_array().unwrap().iter().find(|s| s["name"] == "Pixar").unwrap()["count"],
                1,
                "a repeated studio counts once"
            );
            assert_eq!(
                v["ratings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| r["name"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["MA 15+", "PG", "none"]
            );
            assert_eq!(v["recently_added"]["count"], 1);
            // The alphabet index.
            let o = order("title", false, None, &mut vec![]);
            let idx = letter_index(c, "items i", "i.kind = 'movie'", &[], &o)?;
            assert_eq!(idx, vec![("A".to_string(), 0), ("B".to_string(), 2), ("Z".to_string(), 3)]);
            // A bigger file of m2 replaces its row.
            c.execute("UPDATE files SET size = 99 WHERE id = 'f2'", [])?;
            c.execute(
                "UPDATE files_probe SET size = 99, probe = ?1 WHERE path = '/media/f2.mkv'",
                [probe(3840, 2160, "bt709")],
            )?;
            assert_eq!(refresh_item_video(c)?, 1);
            assert_eq!(ids(c, "movies", "movie", &[("resolution", "4k")], "title"), ["m1", "m2"]);
            Ok(())
        })
        .await
        .unwrap();
    }
}
