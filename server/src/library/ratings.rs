//! Ratings beside TMDB's: IMDb's (with its votes) from IMDb's public dataset, and Rotten
//! Tomatoes (critics) and Metacritic through OMDb when the admin gives a key (`omdb.api_key`).
//! Switch: `features.ratings`.
//!
//! Everything is fetched in the background, at a gentle pace, and kept in the database
//! (`ratings`, keyed by IMDb id); answering a title never waits for it. The item answer
//! carries what is there:
//!
//! ```json
//! "ratings": {"imdb": {"value": 8.1, "votes": 123456}, "rotten_tomatoes": {"value": 94}, "metacritic": {"value": 78}}
//! ```
//!
//! - **IMDb's dataset** (`title.ratings.tsv.gz`, ~8 MB, refreshed daily by IMDb; personal and
//!   non-commercial use, "Information courtesy of IMDb (https://www.imdb.com). Used with
//!   permission."): first fetched a few minutes after start, then again when the last copy is a
//!   week old, or a day old while titles of the library still have no IMDb rating. It is
//!   streamed to the cache folder, read at low priority keeping only the library's titles, and
//!   deleted. Films, shows and episodes alike (whatever has an IMDb id).
//! - **OMDb** (omdbapi.com, CC BY-NC 4.0; a free key allows 1,000 requests a day): films and
//!   shows, one request every [`OMDB_SPACING`], at most [`OMDB_DAILY`] a day, each title again
//!   after 14 days (7 for recent ones), one it doesn't know after 30. A title page opened for a
//!   title not looked up yet moves it to the front ([`Ratings::want`]). A refused key stops it
//!   until the key changes; "limit reached" for [`OMDB_LIMIT_PAUSE`]. Its IMDb rating is kept
//!   only where the dataset has none.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::BufRead,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};

use super::util::now;
use crate::{
    admin::config::{Config, Feature, Secret},
    db::Db,
};

pub const DATASET_URL: &str = "https://datasets.imdbws.com/title.ratings.tsv.gz";
const OMDB_URL: &str = "https://www.omdbapi.com/";
pub const ATTRIBUTION: &str = "Information courtesy of IMDb (https://www.imdb.com). Used with permission. Rotten Tomatoes and Metacritic scores through OMDb (omdbapi.com), CC BY-NC 4.0.";

const DAY: i64 = 86_400;
const START_DELAY: Duration = Duration::from_secs(180);
const DATASET_CHECK: Duration = Duration::from_hours(6);
const DATASET_MAX_AGE: i64 = 7 * DAY;
const DATASET_RETRY: i64 = DAY;
/// The dataset is ~8 MB; anything far larger isn't it.
const DATASET_MAX_BYTES: u64 = 128 << 20;
pub const OMDB_SPACING: Duration = Duration::from_secs(3);
pub const OMDB_DAILY: i64 = 500;
pub const OMDB_LIMIT_PAUSE: i64 = 6 * 3600;
const OMDB_IDLE: Duration = Duration::from_mins(15);
const OMDB_ERROR_PAUSE: i64 = 10 * 60;
const WANTED_MAX: usize = 64;

/// The sources, in the order the apps show them.
pub const SOURCES: [&str; 3] = ["imdb", "rotten_tomatoes", "metacritic"];

/// One title's line of the dataset: (IMDb id, rating, votes).
pub type DatasetRow = (String, f64, i64);

/// Reads `title.ratings.tsv` (header `tconst averageRating numVotes`), keeping the titles in
/// `wanted`.
pub fn parse_dataset(r: impl BufRead, wanted: &HashSet<String>) -> Result<Vec<DatasetRow>> {
    let mut out = Vec::new();
    for (n, line) in r.lines().enumerate() {
        let line = line.context("read the dataset")?;
        if n == 0 {
            if !line.starts_with("tconst") {
                bail!("not IMDb's ratings dataset");
            }
            continue;
        }
        let mut f = line.split('\t');
        let (Some(id), Some(rating), Some(votes)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        if !wanted.contains(id) {
            continue;
        }
        if let (Ok(r), Ok(v)) = (rating.trim().parse::<f64>(), votes.trim().parse::<i64>()) {
            if (0.0..=10.0).contains(&r) && v > 0 {
                out.push((id.to_string(), r, v));
            }
        }
    }
    Ok(out)
}

/// What OMDb answered for one title.
#[derive(Debug, PartialEq)]
pub enum Omdb {
    Found {
        imdb: Option<(f64, Option<i64>)>,
        rotten_tomatoes: Option<i64>,
        metacritic: Option<i64>,
    },
    NotFound,
    BadKey(String),
    Limit,
    Other(String),
}

/// OMDb's answer (`{"Response": "True", "imdbRating": "8.1", "imdbVotes": "1,234", "Metascore":
/// "78", "Ratings": [{"Source": "Rotten Tomatoes", "Value": "94%"}, …]}`; "N/A" where it has none).
pub fn parse_omdb(status: u16, v: &Value) -> Omdb {
    if v["Response"].as_str() != Some("True") {
        let err = v["Error"].as_str().unwrap_or("").to_string();
        let lower = err.to_ascii_lowercase();
        return if lower.contains("limit") {
            Omdb::Limit
        } else if status == 401 || lower.contains("api key") || lower.contains("apikey") {
            Omdb::BadKey(err)
        } else if lower.contains("not found") || lower.contains("incorrect imdb") {
            Omdb::NotFound
        } else {
            Omdb::Other(if err.is_empty() { format!("HTTP {status}") } else { err })
        };
    }
    let num = |s: &str| -> Option<f64> {
        let s = s.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("N/A") {
            return None;
        }
        s.replace(',', "").parse::<f64>().ok()
    };
    let imdb = v["imdbRating"].as_str().and_then(num).filter(|r| (0.0..=10.0).contains(r));
    let votes = v["imdbVotes"].as_str().and_then(num).map(|n| n as i64).filter(|n| *n > 0);
    let mut rt = None;
    let mut mc = None;
    for r in v["Ratings"].as_array().into_iter().flatten() {
        let value = r["Value"].as_str().unwrap_or("");
        match r["Source"].as_str() {
            Some("Rotten Tomatoes") => rt = num(value.trim_end_matches('%')),
            Some("Metacritic") => mc = value.split('/').next().and_then(num),
            _ => {}
        }
    }
    let mc = mc.or_else(|| v["Metascore"].as_str().and_then(num));
    let pct = |x: Option<f64>| x.filter(|x| (0.0..=100.0).contains(x)).map(|x| x.round() as i64);
    Omdb::Found {
        imdb: imdb.map(|r| (r, votes)),
        rotten_tomatoes: pct(rt),
        metacritic: pct(mc),
    }
}

/// Whether IMDb's dataset should be fetched now: never fetched, a week old, or a day since the
/// last try while titles still have no IMDb rating.
pub fn dataset_due(now: i64, last_ok: Option<i64>, last_try: Option<i64>, missing: bool) -> bool {
    if last_try.is_some_and(|t| now - t < DATASET_RETRY) {
        return false;
    }
    match last_ok {
        None => true,
        Some(t) => now - t >= DATASET_MAX_AGE || (missing && now - t >= DATASET_RETRY),
    }
}

/// Whether a title is due for OMDb: never looked up; found, 14 days on (7 for a title from this
/// year or last); not found, 30 days on.
pub fn omdb_due(now: i64, fetched: Option<(i64, bool)>, year: Option<i64>, this_year: i64) -> bool {
    match fetched {
        None => true,
        Some((at, true)) => {
            let recent = year.is_some_and(|y| y >= this_year - 1);
            now - at >= if recent { 7 * DAY } else { 14 * DAY }
        }
        Some((at, false)) => now - at >= 30 * DAY,
    }
}

fn is_imdb_id(s: &str) -> bool {
    s.len() >= 9 && s.len() <= 12 && s.starts_with("tt") && s[2..].bytes().all(|b| b.is_ascii_digit())
}

fn meta_get(c: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    c.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional()
}

fn meta_set(c: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    c.execute("INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)", params![key, value])?;
    Ok(())
}

/// The IMDb ids of the library's titles.
pub fn library_ids(c: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut st = c.prepare("SELECT DISTINCT json_extract(provider_ids, '$.Imdb') FROM items WHERE provider_ids LIKE '%\"Imdb\"%'")?;
    let ids = st
        .query_map([], |r| r.get::<_, Option<String>>(0))?
        .filter_map(|r| r.ok().flatten())
        .filter(|s| is_imdb_id(s))
        .collect();
    Ok(ids)
}

/// A fingerprint of a set of ids (the dataset is asked for "only if changed" while the set is
/// the same; a new title needs the whole file read again).
fn ids_hash(ids: &HashSet<String>) -> String {
    let mut v: Vec<&String> = ids.iter().collect();
    v.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for s in v {
        for b in s.bytes().chain(std::iter::once(b'\n')) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

/// Keeps the dataset's rows (replacing OMDb's IMDb rating where it had one) and forgets titles
/// that left the library.
pub fn store_dataset(c: &mut Connection, rows: &[DatasetRow], library: &HashSet<String>, at: i64) -> Result<usize> {
    let tx = c.transaction()?;
    {
        let mut up = tx.prepare(
            "INSERT INTO ratings(imdb_id, source, value, votes, origin, updated_at) VALUES (?1, 'imdb', ?2, ?3, 'imdb_dataset', ?4)
             ON CONFLICT(imdb_id, source) DO UPDATE SET value = excluded.value, votes = excluded.votes, origin = excluded.origin, updated_at = excluded.updated_at",
        )?;
        for (id, r, v) in rows {
            up.execute(params![id, r, v, at])?;
        }
        let mut known: Vec<String> = Vec::new();
        {
            let mut st = tx.prepare("SELECT DISTINCT imdb_id FROM ratings UNION SELECT DISTINCT imdb_id FROM ratings_fetch")?;
            for id in st.query_map([], |r| r.get::<_, String>(0))? {
                known.push(id?);
            }
        }
        for id in known.iter().filter(|id| !library.contains(*id)) {
            tx.execute("DELETE FROM ratings WHERE imdb_id = ?1", [id])?;
            tx.execute("DELETE FROM ratings_fetch WHERE imdb_id = ?1", [id])?;
        }
    }
    tx.commit()?;
    Ok(rows.len())
}

/// Keeps OMDb's answer for a title.
pub fn store_omdb(c: &mut Connection, imdb_id: &str, answer: &Omdb, at: i64) -> Result<()> {
    let tx = c.transaction()?;
    match answer {
        Omdb::Found {
            imdb,
            rotten_tomatoes,
            metacritic,
        } => {
            let put = |source: &str, value: Option<f64>, votes: Option<i64>| -> rusqlite::Result<()> {
                match value {
                    Some(v) => {
                        tx.execute(
                            "INSERT INTO ratings(imdb_id, source, value, votes, origin, updated_at) VALUES (?1, ?2, ?3, ?4, 'omdb', ?5)
                             ON CONFLICT(imdb_id, source) DO UPDATE SET value = excluded.value, votes = excluded.votes, updated_at = excluded.updated_at
                             WHERE ratings.origin = 'omdb'",
                            params![imdb_id, source, v, votes, at],
                        )?;
                    }
                    None => {
                        tx.execute(
                            "DELETE FROM ratings WHERE imdb_id = ?1 AND source = ?2 AND origin = 'omdb'",
                            params![imdb_id, source],
                        )?;
                    }
                }
                Ok(())
            };
            put("imdb", imdb.map(|(r, _)| r), imdb.and_then(|(_, v)| v))?;
            put("rotten_tomatoes", rotten_tomatoes.map(|v| v as f64), None)?;
            put("metacritic", metacritic.map(|v| v as f64), None)?;
            tx.execute(
                "INSERT OR REPLACE INTO ratings_fetch(imdb_id, provider, fetched_at, ok) VALUES (?1, 'omdb', ?2, 1)",
                params![imdb_id, at],
            )?;
        }
        Omdb::NotFound => {
            tx.execute(
                "INSERT OR REPLACE INTO ratings_fetch(imdb_id, provider, fetched_at, ok) VALUES (?1, 'omdb', ?2, 0)",
                params![imdb_id, at],
            )?;
        }
        _ => {}
    }
    tx.commit()?;
    Ok(())
}

/// The ratings kept for these titles, as the API answers them (titles without any left out).
pub fn lookup(c: &Connection, ids: &[&str]) -> rusqlite::Result<HashMap<String, Value>> {
    let mut out: HashMap<String, Map<String, Value>> = HashMap::new();
    let mut st = c.prepare_cached("SELECT source, value, votes FROM ratings WHERE imdb_id = ?1")?;
    for id in ids.iter().filter(|i| is_imdb_id(i)) {
        if out.contains_key(*id) {
            continue;
        }
        let rows = st.query_map([id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?, r.get::<_, Option<i64>>(2)?))
        })?;
        let mut m = Map::new();
        for row in rows {
            let (source, value, votes) = row?;
            if !SOURCES.contains(&source.as_str()) {
                continue;
            }
            let mut e = json!({"value": if source == "imdb" { json!(value) } else { json!(value.round() as i64) }});
            if let Some(v) = votes {
                e["votes"] = json!(v);
            }
            m.insert(source, e);
        }
        if !m.is_empty() {
            out.insert(id.to_string(), m);
        }
    }
    Ok(out.into_iter().map(|(k, m)| (k, Value::Object(m))).collect())
}

/// Adds `ratings` to the items (from the API's rows) that have some.
pub fn attach(c: &Connection, items: &mut [Value]) -> rusqlite::Result<()> {
    let ids: Vec<String> = items
        .iter()
        .filter_map(|v| v["provider_ids"]["Imdb"].as_str().map(String::from))
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let found = lookup(c, &refs)?;
    for v in items.iter_mut() {
        if let Some(r) = v["provider_ids"]["Imdb"].as_str().and_then(|i| found.get(i)) {
            v["ratings"] = r.clone();
        }
    }
    Ok(())
}

/// What the Services screen says about it.
#[derive(Debug, Clone, Default)]
pub struct Status {
    /// The dataset was last read then (Unix seconds).
    pub dataset_at: Option<i64>,
    /// Titles with an IMDb rating, with Rotten Tomatoes or Metacritic.
    pub imdb_titles: i64,
    pub critic_titles: i64,
    /// OMDb refused the key (its words).
    pub key_refused: Option<String>,
    /// OMDb's daily limit was reached.
    pub limited: bool,
}

#[derive(Default)]
struct OmdbState {
    /// The key OMDb refused (it waits for another).
    refused: Option<(String, String)>,
    paused_until: i64,
    limited: bool,
    /// Films and shows due, oldest first, and when the list was made.
    due: VecDeque<String>,
    due_at: i64,
}

pub struct Ratings {
    db: Db,
    config: Config,
    cache_dir: PathBuf,
    http: reqwest::Client,
    omdb_http: reqwest::Client,
    key: Mutex<Secret>,
    wanted: Mutex<VecDeque<String>>,
    wake: tokio::sync::Notify,
    omdb: Mutex<OmdbState>,
}

impl Ratings {
    pub fn new(db: Db, config: &Config) -> Arc<Ratings> {
        Arc::new(Ratings {
            db,
            cache_dir: config.cache_dir.join("ratings"),
            config: config.clone(),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(20))
                .timeout(Duration::from_secs(300))
                .build()
                .expect("http client"),
            omdb_http: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .redirect(super::util::same_origin_redirects())
                .build()
                .expect("http client"),
            key: Mutex::new(config.omdb_key.clone()),
            wanted: Mutex::new(VecDeque::new()),
            wake: tokio::sync::Notify::new(),
            omdb: Mutex::new(OmdbState::default()),
        })
    }

    fn enabled(&self) -> bool {
        self.config.enabled(Feature::Ratings)
    }

    pub fn omdb_configured(&self) -> bool {
        !self.key.lock().unwrap().expose().is_empty()
    }

    /// The settings changed: a new OMDb key is tried at once.
    pub fn set_key(&self, key: &Secret) {
        let mut k = self.key.lock().unwrap();
        if *k != *key {
            *k = key.clone();
            drop(k);
            let mut o = self.omdb.lock().unwrap();
            o.refused = None;
            o.limited = false;
            o.paused_until = 0;
            drop(o);
            self.wake.notify_one();
        }
    }

    /// A title page was opened: a film or show not looked up yet goes first.
    pub fn want(&self, imdb_id: &str) {
        if !is_imdb_id(imdb_id) || !self.enabled() || !self.omdb_configured() {
            return;
        }
        let mut w = self.wanted.lock().unwrap();
        if !w.iter().any(|x| x == imdb_id) {
            if w.len() >= WANTED_MAX {
                w.pop_back();
            }
            w.push_front(imdb_id.to_string());
            drop(w);
            self.wake.notify_one();
        }
    }

    pub async fn status(&self) -> Status {
        let (refused, limited) = {
            let o = self.omdb.lock().unwrap();
            (o.refused.as_ref().map(|(_, why)| why.clone()), o.limited)
        };
        let counts = self
            .db
            .call(|c| {
                let at = meta_get(c, "ratings.imdb_ok_at")?.and_then(|s| s.parse().ok());
                let imdb: i64 = c.query_row("SELECT COUNT(*) FROM ratings WHERE source = 'imdb'", [], |r| r.get(0))?;
                let critic: i64 = c.query_row(
                    "SELECT COUNT(DISTINCT imdb_id) FROM ratings WHERE source IN ('rotten_tomatoes', 'metacritic')",
                    [],
                    |r| r.get(0),
                )?;
                Ok((at, imdb, critic))
            })
            .await
            .unwrap_or((None, 0, 0));
        Status {
            dataset_at: counts.0,
            imdb_titles: counts.1,
            critic_titles: counts.2,
            key_refused: refused,
            limited,
        }
    }

    /// Starts the two background loops.
    pub fn spawn(self: &Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(START_DELAY).await;
            loop {
                if this.enabled() {
                    if let Err(e) = this.dataset_pass().await {
                        tracing::warn!("ratings: IMDb dataset: {e:#}");
                    }
                }
                tokio::time::sleep(DATASET_CHECK).await;
            }
        });
        let this = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(START_DELAY + Duration::from_secs(30)).await;
            loop {
                let wait = this.omdb_step().await;
                tokio::select! {
                    () = tokio::time::sleep(wait) => {}
                    () = this.wake.notified() => {}
                }
            }
        });
    }

    /// Fetches IMDb's dataset if it is due.
    pub async fn dataset_pass(&self) -> Result<()> {
        let t = now();
        let (ids, last_ok, last_try, missing, same_ids, modified) = self
            .db
            .call(|c| {
                let ids = library_ids(c)?;
                let last_ok: Option<i64> = meta_get(c, "ratings.imdb_ok_at")?.and_then(|s| s.parse().ok());
                let last_try: Option<i64> = meta_get(c, "ratings.imdb_try_at")?.and_then(|s| s.parse().ok());
                let have: HashSet<String> = {
                    let mut st = c.prepare("SELECT imdb_id FROM ratings WHERE source = 'imdb' AND origin = 'imdb_dataset'")?;
                    let v = st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<_, _>>()?;
                    v
                };
                let missing = ids.iter().any(|i| !have.contains(i));
                let same = meta_get(c, "ratings.imdb_ids")?.is_some_and(|h| h == ids_hash(&ids));
                let modified = meta_get(c, "ratings.imdb_modified")?;
                Ok((ids, last_ok, last_try, missing, same, modified))
            })
            .await?;
        if ids.is_empty() || !dataset_due(t, last_ok, last_try, missing) {
            return Ok(());
        }
        self.db
            .call(move |c| Ok(meta_set(c, "ratings.imdb_try_at", &t.to_string())?))
            .await?;
        std::fs::create_dir_all(&self.cache_dir).with_context(|| format!("create {}", self.cache_dir.display()))?;
        let file = self.cache_dir.join("title.ratings.tsv.gz");
        let got = self.download(&file, if same_ids { modified.as_deref() } else { None }).await;
        let modified = match got {
            Ok(Some(m)) => m,
            Ok(None) => {
                tracing::info!("ratings: IMDb dataset unchanged");
                self.db
                    .call(move |c| Ok(meta_set(c, "ratings.imdb_ok_at", &t.to_string())?))
                    .await?;
                return Ok(());
            }
            Err(e) => {
                let _ = std::fs::remove_file(&file);
                return Err(e);
            }
        };
        let wanted = ids.clone();
        let path = file.clone();
        let parsed = read_low_priority(move || {
            let f = std::fs::File::open(&path)?;
            parse_dataset(std::io::BufReader::new(flate2::read::MultiGzDecoder::new(f)), &wanted)
        })
        .await;
        let _ = std::fs::remove_file(&file);
        let rows = parsed?;
        let hash = ids_hash(&ids);
        let n = self
            .db
            .call(move |c| {
                let n = store_dataset(c, &rows, &ids, t)?;
                meta_set(c, "ratings.imdb_ok_at", &t.to_string())?;
                meta_set(c, "ratings.imdb_ids", &hash)?;
                meta_set(c, "ratings.imdb_modified", &modified)?;
                Ok(n)
            })
            .await?;
        tracing::info!(titles = n, "ratings: IMDb dataset read");
        Ok(())
    }

    /// Streams the dataset to `to`. `Ok(None)`: not modified since `since`; else its
    /// Last-Modified (or "").
    async fn download(&self, to: &Path, since: Option<&str>) -> Result<Option<String>> {
        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;
        let mut req = self.http.get(DATASET_URL);
        if let Some(s) = since.filter(|s| !s.is_empty()) {
            req = req.header(reqwest::header::IF_MODIFIED_SINCE, s);
        }
        let resp = req.send().await.context("IMDb dataset")?;
        if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(None);
        }
        if !resp.status().is_success() {
            bail!("IMDb dataset: HTTP {}", resp.status());
        }
        let modified = resp
            .headers()
            .get(reqwest::header::LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let mut out = tokio::fs::File::create(to)
            .await
            .with_context(|| format!("create {}", to.display()))?;
        let mut body = resp.bytes_stream();
        let mut n: u64 = 0;
        while let Some(chunk) = body.next().await {
            let chunk = chunk.context("IMDb dataset")?;
            n += chunk.len() as u64;
            if n > DATASET_MAX_BYTES {
                bail!("IMDb dataset: larger than {} MB", DATASET_MAX_BYTES >> 20);
            }
            out.write_all(&chunk).await?;
        }
        out.flush().await?;
        Ok(Some(modified))
    }

    /// One OMDb request, if one is due; how long to wait before the next.
    async fn omdb_step(&self) -> Duration {
        let key = self.key.lock().unwrap().clone();
        if !self.enabled() || key.expose().is_empty() {
            return OMDB_IDLE;
        }
        let t = now();
        {
            let o = self.omdb.lock().unwrap();
            if o.refused.as_ref().is_some_and(|(k, _)| k == key.expose()) {
                return OMDB_IDLE;
            }
            if o.paused_until > t {
                return Duration::from_secs((o.paused_until - t).clamp(1, OMDB_IDLE.as_secs() as i64) as u64);
            }
        }
        let day = super::chrono_day(t);
        let used: i64 = match self
            .db
            .call(move |c| {
                Ok(meta_get(c, "ratings.omdb_day")?
                    .and_then(|v| v.strip_prefix(&format!("{day}:")).and_then(|n| n.parse().ok()))
                    .unwrap_or(0))
            })
            .await
        {
            Ok(n) => n,
            Err(_) => return OMDB_IDLE,
        };
        if used >= OMDB_DAILY {
            return OMDB_IDLE;
        }
        let Some(id) = self.next_due(t).await else { return OMDB_IDLE };
        let day = super::chrono_day(t);
        let _ = self
            .db
            .call(move |c| Ok(meta_set(c, "ratings.omdb_day", &format!("{day}:{}", used + 1))?))
            .await;
        let answer = match self
            .omdb_http
            .get(OMDB_URL)
            .query(&[("i", id.as_str()), ("apikey", key.expose())])
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status().as_u16();
                match resp.json::<Value>().await {
                    Ok(v) => parse_omdb(status, &v),
                    Err(e) => Omdb::Other(format!("HTTP {status}: {}", e.without_url())),
                }
            }
            // Never with the URL: it holds the key.
            Err(e) => Omdb::Other(e.without_url().to_string()),
        };
        match &answer {
            Omdb::Found { .. } | Omdb::NotFound => {
                let a = answer;
                let i = id.clone();
                if let Err(e) = self.db.call(move |c| store_omdb(c, &i, &a, now())).await {
                    tracing::warn!("ratings: store {id}: {e:#}");
                }
            }
            Omdb::BadKey(why) => {
                tracing::warn!("ratings: OMDb refused the key: {why}");
                self.omdb.lock().unwrap().refused = Some((key.expose().to_string(), why.clone()));
            }
            Omdb::Limit => {
                tracing::info!("ratings: OMDb's daily limit reached");
                let mut o = self.omdb.lock().unwrap();
                o.limited = true;
                o.paused_until = t + OMDB_LIMIT_PAUSE;
            }
            Omdb::Other(why) => {
                tracing::warn!("ratings: OMDb {id}: {why}");
                self.omdb.lock().unwrap().paused_until = t + OMDB_ERROR_PAUSE;
            }
        }
        OMDB_SPACING
    }

    /// The next film or show to look up: one a page asked for (if due), else the oldest due.
    async fn next_due(&self, t: i64) -> Option<String> {
        let this_year = super::chrono_day(t).get(..4).and_then(|y| y.parse::<i64>().ok()).unwrap_or(2000);
        let wanted: Vec<String> = self.wanted.lock().unwrap().drain(..).collect();
        let refresh = { self.omdb.lock().unwrap().due_at + OMDB_IDLE.as_secs() as i64 <= t };
        let (first, due) = self
            .db
            .call(move |c| {
                let fetched = |id: &str| -> rusqlite::Result<Option<(i64, bool)>> {
                    c.query_row(
                        "SELECT fetched_at, ok FROM ratings_fetch WHERE imdb_id = ?1 AND provider = 'omdb'",
                        [id],
                        |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)),
                    )
                    .optional()
                };
                let mut first = None;
                for w in &wanted {
                    if omdb_due(t, fetched(w)?, None, this_year) {
                        first = Some(w.clone());
                        break;
                    }
                }
                if !refresh {
                    return Ok((first, None));
                }
                // Never looked up first (newest added first), then the oldest look-up.
                let mut st = c.prepare(
                    "SELECT json_extract(i.provider_ids, '$.Imdb') AS imdb, MAX(i.year), f.fetched_at, f.ok, MAX(i.added_at)
                     FROM items i LEFT JOIN ratings_fetch f ON f.imdb_id = json_extract(i.provider_ids, '$.Imdb') AND f.provider = 'omdb'
                     WHERE i.kind IN ('movie', 'series') AND i.provider_ids LIKE '%\"Imdb\"%'
                     GROUP BY imdb ORDER BY f.fetched_at IS NOT NULL, f.fetched_at, MAX(i.added_at) DESC",
                )?;
                let rows = st.query_map([], |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<i64>>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                    ))
                })?;
                let mut due = VecDeque::new();
                for row in rows {
                    let (id, year, at, ok) = row?;
                    let Some(id) = id.filter(|i| is_imdb_id(i)) else { continue };
                    if omdb_due(t, at.map(|a| (a, ok.unwrap_or(0) != 0)), year, this_year) {
                        due.push_back(id);
                    }
                }
                Ok((first, Some(due)))
            })
            .await
            .ok()?;
        let mut o = self.omdb.lock().unwrap();
        if let Some(d) = due {
            o.due = d;
            o.due_at = t;
        }
        if let Some(f) = first {
            o.due.retain(|x| *x != f);
            return Some(f);
        }
        o.due.pop_front()
    }
}

/// Runs `f` on a thread of its own, at a low CPU priority where the system allows it per thread.
async fn read_low_priority<R: Send + 'static>(f: impl FnOnce() -> Result<R> + Send + 'static) -> Result<R> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("ratings-read".into())
        .spawn(move || {
            // Linux: a thread's own nice value (elsewhere it would be the whole process's).
            // SAFETY: setpriority only takes integers (0: the calling thread) and touches no memory.
            #[cfg(target_os = "linux")]
            unsafe {
                libc::setpriority(libc::PRIO_PROCESS, 0, 10);
            }
            let _ = tx.send(f());
        })
        .context("spawn the ratings reader")?;
    rx.await.context("the ratings reader stopped")?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn reads_the_dataset() {
        let tsv = "tconst\taverageRating\tnumVotes\ntt0000001\t5.7\t2100\ntt0133093\t8.7\t2112345\ntt0903747\t9.5\t2300000\nttbroken\tx\t1\ntt0000002\t\\N\t5\n";
        let rows = parse_dataset(tsv.as_bytes(), &set(&["tt0133093", "tt0903747", "tt0000002"])).unwrap();
        assert_eq!(
            rows,
            vec![("tt0133093".to_string(), 8.7, 2112345), ("tt0903747".to_string(), 9.5, 2300000)]
        );
        assert!(parse_dataset("<html>".as_bytes(), &set(&[])).is_err(), "not the dataset");
    }

    #[test]
    fn reads_the_dataset_gzipped() {
        use std::io::Write;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(b"tconst\taverageRating\tnumVotes\ntt0133093\t8.7\t2112345\n").unwrap();
        let bytes = gz.finish().unwrap();
        let rows = parse_dataset(
            std::io::BufReader::new(flate2::read::MultiGzDecoder::new(&bytes[..])),
            &set(&["tt0133093"]),
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn reads_omdb() {
        let v = json!({"Response": "True", "imdbRating": "8.7", "imdbVotes": "2,112,345", "Metascore": "73",
            "Ratings": [{"Source": "Internet Movie Database", "Value": "8.7/10"}, {"Source": "Rotten Tomatoes", "Value": "83%"}, {"Source": "Metacritic", "Value": "73/100"}]});
        assert_eq!(
            parse_omdb(200, &v),
            Omdb::Found {
                imdb: Some((8.7, Some(2112345))),
                rotten_tomatoes: Some(83),
                metacritic: Some(73)
            }
        );
        // A show: no RT, the Metascore N/A.
        let v = json!({"Response": "True", "imdbRating": "9.5", "imdbVotes": "N/A", "Metascore": "N/A", "Ratings": [{"Source": "Internet Movie Database", "Value": "9.5/10"}]});
        assert_eq!(
            parse_omdb(200, &v),
            Omdb::Found {
                imdb: Some((9.5, None)),
                rotten_tomatoes: None,
                metacritic: None
            }
        );
        // Metascore without a Ratings entry.
        let v = json!({"Response": "True", "imdbRating": "N/A", "Metascore": "61"});
        assert_eq!(
            parse_omdb(200, &v),
            Omdb::Found {
                imdb: None,
                rotten_tomatoes: None,
                metacritic: Some(61)
            }
        );
        assert_eq!(
            parse_omdb(200, &json!({"Response": "False", "Error": "Incorrect IMDb ID."})),
            Omdb::NotFound
        );
        assert_eq!(
            parse_omdb(200, &json!({"Response": "False", "Error": "Movie not found!"})),
            Omdb::NotFound
        );
        assert_eq!(
            parse_omdb(401, &json!({"Response": "False", "Error": "Invalid API key!"})),
            Omdb::BadKey("Invalid API key!".into())
        );
        assert_eq!(
            parse_omdb(401, &json!({"Response": "False", "Error": "Request limit reached!"})),
            Omdb::Limit
        );
        assert!(matches!(parse_omdb(503, &json!({})), Omdb::Other(_)));
    }

    #[test]
    fn when_things_are_due() {
        let now = 100 * DAY;
        assert!(dataset_due(now, None, None, true), "never fetched");
        assert!(!dataset_due(now, Some(now - 2 * DAY), None, false), "fresh, nothing missing");
        assert!(
            dataset_due(now, Some(now - 2 * DAY), Some(now - 2 * DAY), true),
            "a day on with titles missing"
        );
        assert!(!dataset_due(now, Some(now - 3600), Some(now - 3600), true), "not within a day");
        assert!(dataset_due(now, Some(now - 8 * DAY), None, false), "a week old");
        assert!(!dataset_due(now, None, Some(now - 3600), true), "a failed try waits a day");
        assert!(omdb_due(now, None, Some(1999), 2026));
        assert!(!omdb_due(now, Some((now - 10 * DAY, true)), Some(1999), 2026));
        assert!(omdb_due(now, Some((now - 15 * DAY, true)), Some(1999), 2026));
        assert!(omdb_due(now, Some((now - 8 * DAY, true)), Some(2025), 2026), "recent titles weekly");
        assert!(!omdb_due(now, Some((now - 20 * DAY, false)), None, 2026));
        assert!(omdb_due(now, Some((now - 31 * DAY, false)), None, 2026));
    }

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(include_str!("../../migrations/016_ratings.sql")).unwrap();
        c.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL); CREATE TABLE items (id TEXT, kind TEXT, provider_ids TEXT, year INTEGER, added_at INTEGER);").unwrap();
        c
    }

    #[test]
    fn caches_and_answers() {
        let mut c = db();
        let lib = set(&["tt0133093", "tt0903747"]);
        store_dataset(&mut c, &[("tt0133093".into(), 8.7, 2_000_000)], &lib, 10).unwrap();
        // OMDb adds critics, and its IMDb rating only where the dataset has none.
        let a = Omdb::Found {
            imdb: Some((8.6, Some(1))),
            rotten_tomatoes: Some(83),
            metacritic: Some(73),
        };
        store_omdb(&mut c, "tt0133093", &a, 20).unwrap();
        let b = Omdb::Found {
            imdb: Some((9.4, Some(5))),
            rotten_tomatoes: None,
            metacritic: None,
        };
        store_omdb(&mut c, "tt0903747", &b, 20).unwrap();
        let got = lookup(&c, &["tt0133093", "tt0903747", "tt0000009"]).unwrap();
        assert_eq!(
            got["tt0133093"],
            json!({"imdb": {"value": 8.7, "votes": 2_000_000}, "rotten_tomatoes": {"value": 83}, "metacritic": {"value": 73}})
        );
        assert_eq!(got["tt0903747"], json!({"imdb": {"value": 9.4, "votes": 5}}));
        assert!(!got.contains_key("tt0000009"), "nothing known: no field");
        // The next dataset replaces OMDb's IMDb rating; titles gone from the library are forgotten.
        store_dataset(&mut c, &[("tt0903747".into(), 9.5, 2_300_000)], &set(&["tt0903747"]), 30).unwrap();
        let got = lookup(&c, &["tt0133093", "tt0903747"]).unwrap();
        assert!(!got.contains_key("tt0133093"));
        assert_eq!(got["tt0903747"]["imdb"], json!({"value": 9.5, "votes": 2_300_000}));
        // OMDb doesn't overwrite the dataset's, and a score it no longer has goes.
        store_omdb(
            &mut c,
            "tt0903747",
            &Omdb::Found {
                imdb: Some((1.0, None)),
                rotten_tomatoes: Some(50),
                metacritic: None,
            },
            40,
        )
        .unwrap();
        store_omdb(
            &mut c,
            "tt0903747",
            &Omdb::Found {
                imdb: None,
                rotten_tomatoes: None,
                metacritic: None,
            },
            50,
        )
        .unwrap();
        assert_eq!(
            lookup(&c, &["tt0903747"]).unwrap()["tt0903747"],
            json!({"imdb": {"value": 9.5, "votes": 2_300_000}})
        );
        let f: (i64, i64) = c
            .query_row("SELECT fetched_at, ok FROM ratings_fetch WHERE imdb_id = 'tt0903747'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(f, (50, 1));
        store_omdb(&mut c, "tt0903747", &Omdb::NotFound, 60).unwrap();
        // Attached to the API's rows by their IMDb id.
        let mut items = vec![
            json!({"id": "a", "provider_ids": {"Imdb": "tt0903747"}}),
            json!({"id": "b", "provider_ids": {}}),
        ];
        attach(&c, &mut items).unwrap();
        assert_eq!(items[0]["ratings"]["imdb"]["value"], 9.5);
        assert!(items[1].get("ratings").is_none());
    }

    /// The look-up order: a title a page asked for, then never looked up (newest first), then
    /// the oldest look-up; nothing that isn't due; and a page's wish only with a key.
    #[tokio::test]
    async fn omdb_order() {
        let db = crate::db::Db::memory().unwrap();
        let t = 1_800_000_000;
        db.call(move |c| {
            let lib = "l1";
            for (id, kind, imdb, added) in [("a", "movie", "tt0000001", 1), ("b", "movie", "tt0000002", 5), ("c", "series", "tt0000003", 2), ("d", "episode", "tt0000004", 9), ("e", "movie", "tt0000005", 3)] {
                c.execute(
                    "INSERT INTO items(id, kind, library_id, key, title, identify_state, provider_ids, year, added_at, updated_at) VALUES (?1, ?2, ?3, ?1, ?1, 'matched', ?4, 1999, ?5, ?5)",
                    params![id, kind, lib, format!("{{\"Imdb\":\"{imdb}\"}}"), added],
                )?;
            }
            // c: looked up long ago (due); e: just now (not due).
            c.execute("INSERT INTO ratings_fetch VALUES ('tt0000003', 'omdb', ?1, 1)", [t - 20 * DAY])?;
            c.execute("INSERT INTO ratings_fetch VALUES ('tt0000005', 'omdb', ?1, 1)", [t - DAY])?;
            Ok(())
        })
        .await
        .unwrap();
        let env = crate::admin::config::Env::from_pairs([("MOONRISE_NAME", "x")]);
        let config = Config::resolve(&env, &crate::admin::settings::Stored::default());
        let r = Ratings::new(db, &config);
        r.want("tt0000001");
        assert!(r.wanted.lock().unwrap().is_empty(), "no key: nothing asked for");
        r.set_key(&Secret::new("k"));
        r.want("tt0000001");
        r.want("tt0000005");
        let mut got = Vec::new();
        while let Some(id) = r.next_due(t).await {
            got.push(id);
        }
        assert_eq!(got, ["tt0000001", "tt0000002", "tt0000003"], "episodes and fresh titles left out");
    }

    #[test]
    fn library_ids_and_their_fingerprint() {
        let c = db();
        c.execute_batch(
            "INSERT INTO items VALUES ('1', 'movie', '{\"Tmdb\":\"603\",\"Imdb\":\"tt0133093\"}', 1999, 1);
             INSERT INTO items VALUES ('2', 'episode', '{\"Imdb\":\"tt0959621\"}', 2008, 2);
             INSERT INTO items VALUES ('3', 'movie', '{\"Imdb\":\"nonsense\"}', 2000, 3);
             INSERT INTO items VALUES ('4', 'movie', NULL, 2000, 4);",
        )
        .unwrap();
        let ids = library_ids(&c).unwrap();
        assert_eq!(ids, set(&["tt0133093", "tt0959621"]));
        assert_eq!(ids_hash(&ids), ids_hash(&set(&["tt0959621", "tt0133093"])));
        assert_ne!(ids_hash(&ids), ids_hash(&set(&["tt0133093"])));
    }
}
