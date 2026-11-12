//! Intro, recap and end-credits times from the community databases: TheIntroDB
//! (`api.theintrodb.org`, by TVDB, TMDB or IMDb id) and IntroDB (`api.introdb.app`, by IMDb id).
//! The apps used to ask them from the device; now only the server does, and
//! `GET /v1/items/{id}/segments?merged=1` answers with the segments the player offers
//! (`media/segment_rules.rs`). Switch: `features.community_segments`.
//!
//! - **What is asked** is what the apps asked: an episode by its show's ids with its season and
//!   episode numbers (and its own TVDB id), a film by its own ids; TheIntroDB with the file's
//!   length (`duration_ms`). Parts of split episodes aren't looked up (the times are the whole
//!   episode's), nor episodes without numbers.
//! - **On demand only**, never the whole library: opening a film or an episode, and asking for
//!   its segments, puts it (and the next episode) at the front of a short queue. One background
//!   worker asks each database at most once every [`SPACING`], at most [`DAILY`] times a day; a
//!   429 pauses that database (its `Retry-After`, else [`PAUSE`]), and a failed request is tried
//!   again after [`RETRY`] (it is not kept as "nothing").
//! - **Kept** in `community_segments` (migration 018), keyed by the query asked, so titles matched
//!   again with the same ids reuse it: found again after [`FOUND_MAX_AGE`], nothing found after
//!   [`EMPTY_MAX_AGE`]. A stale answer is used while it is asked again.
//! - **Never waited for**: `/segments` answers with what is kept, and says `community_pending`
//!   while the title waits in the queue; the apps ask again a few seconds later.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::util::now;
use crate::{
    admin::config::{Config, Feature},
    db::Db,
    media::segment_rules::Segment,
};

pub const THEINTRODB_URL: &str = "https://api.theintrodb.org/v4/media";
pub const INTRODB_URL: &str = "https://api.introdb.app/segments";

const DAY: i64 = 86_400;
/// One request per database at most this often.
pub const SPACING: Duration = Duration::from_secs(2);
/// Requests per database per day, at most.
pub const DAILY: u32 = 2000;
/// A database that answered 429 without a `Retry-After` is left alone this long (seconds).
pub const PAUSE: i64 = 15 * 60;
/// A request that failed (network, 5xx, not JSON) is tried again after this long (seconds).
pub const RETRY: i64 = 3600;
/// An answer with times is asked for again after this long (seconds).
pub const FOUND_MAX_AGE: i64 = 30 * DAY;
/// An answer without any is asked for again after this long (seconds).
pub const EMPTY_MAX_AGE: i64 = 7 * DAY;
const WANTED_MAX: usize = 64;
const TIMEOUT: Duration = Duration::from_secs(10);

/// The databases, in the order their answers fill gaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    TheIntroDb,
    IntroDb,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::TheIntroDb, Provider::IntroDb];

    pub fn name(self) -> &'static str {
        match self {
            Provider::TheIntroDb => "theintrodb",
            Provider::IntroDb => "introdb",
        }
    }

    fn index(self) -> usize {
        match self {
            Provider::TheIntroDb => 0,
            Provider::IntroDb => 1,
        }
    }
}

/// What a title is looked up by: a film's ids, or an episode's show's with its numbers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lookup {
    pub tvdb: Option<String>,
    pub tmdb: Option<String>,
    pub imdb: Option<String>,
    pub tvdb_episode: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    /// The length of the title's file (seconds; 0 when not known).
    pub runtime: f64,
}

/// The query a database is asked with (without the file's length): also the key it is kept by.
/// None: nothing to ask it with.
pub fn query(p: Provider, l: &Lookup) -> Option<Vec<(&'static str, String)>> {
    let mut q = Vec::new();
    match p {
        Provider::TheIntroDb => {
            if let Some(v) = &l.tvdb {
                q.push(("tvdb_id", v.clone()));
            } else if let Some(v) = &l.tmdb {
                q.push(("tmdb_id", v.clone()));
            } else if let Some(v) = &l.imdb {
                q.push(("imdb_id", v.clone()));
            } else {
                return None;
            }
            if let (Some(s), Some(e)) = (l.season, l.episode) {
                q.push(("season", s.to_string()));
                q.push(("episode", e.to_string()));
                if let Some(t) = &l.tvdb_episode {
                    q.push(("tvdb_episode_id", t.clone()));
                }
            }
        }
        Provider::IntroDb => {
            q.push(("imdb_id", l.imdb.clone()?));
            if let (Some(s), Some(e)) = (l.season, l.episode) {
                q.push(("season", s.to_string()));
                q.push(("episode", e.to_string()));
            } else {
                q.push(("is_movie", "true".into()));
            }
        }
    }
    Some(q)
}

/// The key an answer is kept by: the query, `name=value&…`.
pub fn key(q: &[(&str, String)]) -> String {
    q.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")
}

/// A segment as a database gave it. `end` None: to the end of the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Raw {
    pub kind: String,
    pub start: f64,
    pub end: Option<f64>,
}

fn number(v: &Value) -> Option<f64> {
    v.as_f64().filter(|x| x.is_finite())
}

/// TheIntroDB's answer: `{intro|recap|credits|preview: [{start_ms, end_ms}]}` (the first of
/// each); a null start is the start of the file, a null end its end.
pub fn parse_theintrodb(v: &Value) -> Vec<Raw> {
    ["intro", "recap", "credits", "preview"]
        .into_iter()
        .filter_map(|kind| {
            let s = v[kind].as_array()?.first()?;
            if !s.is_object() {
                return None;
            }
            Some(Raw {
                kind: kind.into(),
                start: number(&s["start_ms"]).map_or(0.0, |x| x / 1000.0),
                end: number(&s["end_ms"]).map(|x| x / 1000.0),
            })
        })
        .collect()
}

/// IntroDB's answer: `{intro|recap|outro: {start_sec, end_sec}}`; `outro` is the end credits, a
/// null end the end of the file.
pub fn parse_introdb(v: &Value) -> Vec<Raw> {
    [("intro", "intro"), ("recap", "recap"), ("outro", "credits")]
        .into_iter()
        .filter_map(|(key, kind)| {
            let s = &v[key];
            Some(Raw {
                kind: kind.into(),
                start: number(&s["start_sec"])?,
                end: number(&s["end_sec"]),
            })
        })
        .collect()
}

/// A database's segments for a file of `runtime` seconds (an open end is the file's end; empty
/// ones are left out), as the apps read them.
pub fn resolve(p: Provider, raw: &[Raw], runtime: f64) -> Vec<Segment> {
    raw.iter()
        .filter_map(|r| {
            let end = r.end.unwrap_or(runtime);
            (end > r.start).then(|| Segment::new(&r.kind, r.start, end, Some(p.name())))
        })
        .collect()
}

/// Whether a kept answer should be asked for again.
pub fn stale(now: i64, fetched_at: i64, found: bool) -> bool {
    now - fetched_at >= if found { FOUND_MAX_AGE } else { EMPTY_MAX_AGE }
}

/// The kept answer: (when, segments).
pub fn cached(c: &Connection, p: Provider, key: &str) -> rusqlite::Result<Option<(i64, Vec<Raw>)>> {
    c.query_row(
        "SELECT fetched_at, segments FROM community_segments WHERE provider = ?1 AND lookup = ?2",
        params![p.name(), key],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
    )
    .optional()
    .map(|o| o.map(|(at, s)| (at, serde_json::from_str(&s).unwrap_or_default())))
}

pub fn store(c: &Connection, p: Provider, key: &str, at: i64, raw: &[Raw]) -> rusqlite::Result<()> {
    c.execute(
        "INSERT OR REPLACE INTO community_segments(provider, lookup, fetched_at, segments) VALUES (?1, ?2, ?3, ?4)",
        params![p.name(), key, at, serde_json::to_string(raw).unwrap_or_else(|_| "[]".into())],
    )?;
    Ok(())
}

fn ids(v: Option<String>) -> HashMap<String, String> {
    v.and_then(|s| serde_json::from_str::<HashMap<String, Value>>(&s).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(k, v)| {
            let s = match v {
                Value::String(s) => s,
                Value::Number(n) => n.to_string(),
                _ => return None,
            };
            (!s.is_empty()).then_some((k, s))
        })
        .collect()
}

/// What a film or an episode is looked up by. None: not a film or an episode, a part of a split
/// episode, or an episode without numbers.
pub fn lookup_for(c: &Connection, item: &str) -> rusqlite::Result<Option<Lookup>> {
    let row = c
        .query_row(
            &format!(
                "SELECT i.kind, i.season_no, i.episode_no, i.part_no, i.provider_ids, s.provider_ids, i.runtime,
                        (SELECT f.duration FROM files f WHERE f.item_id = i.id AND f.role IN {} AND f.missing_since IS NULL
                         ORDER BY f.role = 'broken', f.size DESC LIMIT 1)
                 FROM items i LEFT JOIN items s ON s.id = i.series_id WHERE i.id = ?1",
                super::TITLE_ROLES
            ),
            [item],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<f64>>(6)?,
                    r.get::<_, Option<f64>>(7)?,
                ))
            },
        )
        .optional()?;
    let Some((kind, season, episode, part, own, show, runtime, duration)) = row else {
        return Ok(None);
    };
    if part.is_some() {
        return Ok(None);
    }
    let own = ids(own);
    let runtime = duration.or(runtime).filter(|d| d.is_finite() && *d > 0.0).unwrap_or(0.0);
    Ok(match kind.as_str() {
        "movie" => Some(Lookup {
            tvdb: own.get("Tvdb").cloned(),
            tmdb: own.get("Tmdb").cloned(),
            imdb: own.get("Imdb").cloned(),
            runtime,
            ..Lookup::default()
        }),
        "episode" if season.is_some() && episode.is_some() => {
            let show = ids(show);
            Some(Lookup {
                tvdb: show.get("Tvdb").cloned(),
                tmdb: show.get("Tmdb").cloned(),
                imdb: show.get("Imdb").cloned(),
                tvdb_episode: own.get("Tvdb").cloned(),
                season,
                episode,
                runtime,
            })
        }
        _ => None,
    })
}

/// The episode after this one in its show (airing order, specials not after a regular episode).
pub fn next_episode(c: &Connection, item: &str) -> rusqlite::Result<Option<String>> {
    c.query_row(
        "SELECT n.id FROM items e JOIN items n ON n.series_id = e.series_id AND n.kind = 'episode'
         WHERE e.id = ?1 AND e.kind = 'episode' AND n.season_no > 0
           AND (n.season_no > e.season_no OR (n.season_no = e.season_no AND n.episode_no > e.episode_no))
         ORDER BY n.season_no, n.episode_no, n.part_no LIMIT 1",
        [item],
        |r| r.get(0),
    )
    .optional()
}

/// What `/segments?merged=1` gets: one list per database (in [`Provider::ALL`] order), and
/// whether the title is waiting to be looked up.
#[derive(Debug, Default, PartialEq)]
pub struct Answer {
    pub online: Vec<Vec<Segment>>,
    pub pending: bool,
}

#[derive(Default)]
struct State {
    /// Items to look up, first first.
    wanted: VecDeque<String>,
    /// The item being looked up now.
    busy: Option<String>,
    /// Per database: the last request, paused until (Unix seconds), today and its count.
    last: [Option<Instant>; 2],
    paused_until: [i64; 2],
    day: [i64; 2],
    used: [u32; 2],
    /// (database, key) → when a failed request may be tried again.
    failed: HashMap<(Provider, String), i64>,
}

pub struct Community {
    db: Db,
    config: Config,
    http: reqwest::Client,
    bases: [String; 2],
    state: Mutex<State>,
    wake: tokio::sync::Notify,
}

impl Community {
    pub fn new(db: Db, config: &Config) -> Arc<Community> {
        Self::with_bases(db, config, [THEINTRODB_URL.into(), INTRODB_URL.into()])
    }

    /// With other addresses for the databases (tests).
    pub fn with_bases(db: Db, config: &Config, bases: [String; 2]) -> Arc<Community> {
        Arc::new(Community {
            db,
            config: config.clone(),
            http: reqwest::Client::builder()
                .timeout(TIMEOUT)
                .user_agent(concat!("Moonrise/", env!("CARGO_PKG_VERSION")))
                .redirect(super::util::same_origin_redirects())
                .build()
                .expect("http client"),
            bases,
            state: Mutex::new(State::default()),
            wake: tokio::sync::Notify::new(),
        })
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled(Feature::CommunitySegments)
    }

    /// Whether a database may be asked now or soon (not paused, today's count not used up).
    fn open(s: &State, p: Provider, t: i64) -> bool {
        let i = p.index();
        s.paused_until[i] <= t && (s.day[i] != t.div_euclid(DAY) || s.used[i] < DAILY)
    }

    /// Puts titles at the front of the queue (in this order), for those of their answers that
    /// are missing or stale. Answers whether anything was queued.
    pub async fn want(&self, items: Vec<String>) -> bool {
        if !self.enabled() || items.is_empty() {
            return false;
        }
        let t = now();
        let needs = {
            let items = items.clone();
            self.db
                .call(move |c| {
                    let mut out = Vec::new();
                    for item in items {
                        let Some(l) = lookup_for(c, &item)? else { continue };
                        let mut keys = Vec::new();
                        for p in Provider::ALL {
                            let Some(q) = query(p, &l) else { continue };
                            let k = key(&q);
                            let due = match cached(c, p, &k)? {
                                None => true,
                                Some((at, raw)) => stale(t, at, !raw.is_empty()),
                            };
                            if due {
                                keys.push((p, k));
                            }
                        }
                        out.push((item, keys));
                    }
                    Ok(out)
                })
                .await
                .unwrap_or_default()
        };
        let mut s = self.state.lock().unwrap();
        let mut queued = false;
        for (item, keys) in needs.into_iter().rev() {
            let worth = keys
                .iter()
                .any(|(p, k)| Self::open(&s, *p, t) && s.failed.get(&(*p, k.clone())).is_none_or(|at| *at <= t));
            if !worth {
                continue;
            }
            queued = true;
            if s.busy.as_deref() == Some(item.as_str()) {
                continue;
            }
            s.wanted.retain(|w| *w != item);
            s.wanted.push_front(item);
        }
        s.wanted.truncate(WANTED_MAX);
        drop(s);
        if queued {
            self.wake.notify_one();
        }
        queued
    }

    /// Whether a title waits to be looked up (or is being looked up now).
    pub fn waiting(&self, item: &str) -> bool {
        let s = self.state.lock().unwrap();
        s.busy.as_deref() == Some(item) || s.wanted.iter().any(|w| w == item)
    }

    /// The kept answers for a title, its file `runtime` long (open ends end there), with the
    /// title and the next episode queued for what is missing or stale. Never waits for a lookup.
    pub async fn answer(&self, item: &str, runtime: f64) -> Answer {
        if !self.enabled() {
            return Answer::default();
        }
        let id = item.to_string();
        let kept = self
            .db
            .call(move |c| {
                let Some(l) = lookup_for(c, &id)? else { return Ok(None) };
                let mut lists = Vec::new();
                let mut missing = false;
                for p in Provider::ALL {
                    let Some(q) = query(p, &l) else {
                        lists.push(Vec::new());
                        continue;
                    };
                    match cached(c, p, &key(&q))? {
                        Some((_, raw)) => lists.push(resolve(p, &raw, runtime)),
                        None => {
                            missing = true;
                            lists.push(Vec::new());
                        }
                    }
                }
                let next = next_episode(c, &id)?;
                Ok(Some((lists, missing, next)))
            })
            .await
            .ok()
            .flatten();
        let Some((online, missing, next)) = kept else {
            return Answer::default();
        };
        let mut want = vec![item.to_string()];
        want.extend(next);
        let queued = self.want(want).await;
        Answer {
            online,
            pending: missing && queued && self.waiting(item),
        }
    }

    /// Starts the worker.
    pub fn spawn(self: &Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                let next = {
                    let mut s = this.state.lock().unwrap();
                    let n = s.wanted.pop_front();
                    s.busy.clone_from(&n);
                    n
                };
                match next {
                    Some(item) => {
                        this.look_up(&item).await;
                        this.state.lock().unwrap().busy = None;
                    }
                    None => {
                        tokio::select! {
                            () = this.wake.notified() => {}
                            () = tokio::time::sleep(Duration::from_secs(600)) => {}
                        }
                    }
                }
            }
        });
    }

    /// Asks the databases for one title what is missing or stale.
    pub async fn look_up(&self, item: &str) {
        if !self.enabled() {
            return;
        }
        let id = item.to_string();
        let t = now();
        let todo = self
            .db
            .call(move |c| {
                let Some(l) = lookup_for(c, &id)? else { return Ok(Vec::new()) };
                let mut out = Vec::new();
                for p in Provider::ALL {
                    let Some(q) = query(p, &l) else { continue };
                    let k = key(&q);
                    let due = match cached(c, p, &k)? {
                        None => true,
                        Some((at, raw)) => stale(t, at, !raw.is_empty()),
                    };
                    if due {
                        out.push((p, q, k, l.runtime));
                    }
                }
                Ok(out)
            })
            .await
            .unwrap_or_default();
        for (p, q, k, runtime) in todo {
            self.ask(p, q, k, runtime).await;
        }
    }

    /// One request, at the database's pace; the answer kept, or the failure remembered.
    async fn ask(&self, p: Provider, mut q: Vec<(&'static str, String)>, k: String, runtime: f64) {
        let i = p.index();
        let wait = {
            let mut s = self.state.lock().unwrap();
            let t = now();
            if !Self::open(&s, p, t) || s.failed.get(&(p, k.clone())).is_some_and(|at| *at > t) {
                return;
            }
            let day = t.div_euclid(DAY);
            if s.day[i] != day {
                s.day[i] = day;
                s.used[i] = 0;
                s.failed.retain(|_, at| *at > t);
            }
            s.used[i] += 1;
            let wait = s.last[i].map_or(Duration::ZERO, |l| SPACING.saturating_sub(l.elapsed()));
            s.last[i] = Some(Instant::now() + wait);
            wait
        };
        tokio::time::sleep(wait).await;
        if p == Provider::TheIntroDb && runtime > 0.0 {
            q.push(("duration_ms", ((runtime * 1000.0) as i64).to_string()));
        }
        let got = self.http.get(&self.bases[i]).query(&q).send().await;
        let t = now();
        let result: Result<Option<Vec<Raw>>, String> = match got {
            Ok(resp) => {
                let status = resp.status();
                if status == reqwest::StatusCode::NOT_FOUND {
                    Ok(Some(Vec::new()))
                } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    let after = resp
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok()?.trim().parse::<i64>().ok())
                        .filter(|s| *s > 0)
                        .map_or(PAUSE, |s| s.min(DAY));
                    tracing::info!(database = p.name(), secs = after, "community segments: rate limited, paused");
                    self.state.lock().unwrap().paused_until[i] = t + after;
                    Ok(None)
                } else if status.is_success() {
                    match resp.json::<Value>().await {
                        Ok(v) if v.is_object() => Ok(Some(match p {
                            Provider::TheIntroDb => parse_theintrodb(&v),
                            Provider::IntroDb => parse_introdb(&v),
                        })),
                        Ok(_) => Err("not a JSON object".into()),
                        Err(e) => Err(e.without_url().to_string()),
                    }
                } else {
                    Err(format!("HTTP {status}"))
                }
            }
            Err(e) => Err(e.without_url().to_string()),
        };
        match result {
            Ok(Some(raw)) => {
                let n = raw.len();
                let key = k.clone();
                if let Err(e) = self.db.call(move |c| Ok(store(c, p, &key, t, &raw)?)).await {
                    tracing::warn!("community segments: keep {}: {e:#}", p.name());
                }
                tracing::debug!(database = p.name(), lookup = %k, segments = n, "community segments");
            }
            Ok(None) => {}
            Err(why) => {
                tracing::info!(database = p.name(), "community segments: {why}; again in an hour");
                self.state.lock().unwrap().failed.insert((p, k), t + RETRY);
            }
        }
    }
}

/// The answers kept: (database, query).
#[cfg(test)]
fn kept_keys(c: &Connection) -> rusqlite::Result<std::collections::HashSet<(String, String)>> {
    let mut st = c.prepare("SELECT provider, lookup FROM community_segments")?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_config() -> Config {
        let env = crate::admin::config::Env::from_pairs([("MOONRISE_NAME", "x")]);
        Config::resolve(&env, &crate::admin::settings::Stored::default())
    }

    fn episode() -> Lookup {
        Lookup {
            tvdb: Some("75299".into()),
            imdb: Some("tt0141842".into()),
            tvdb_episode: Some("306220".into()),
            season: Some(6),
            episode: Some(6),
            runtime: 3324.672,
            ..Lookup::default()
        }
    }

    #[test]
    fn queries_are_the_apps() {
        let l = episode();
        assert_eq!(
            key(&query(Provider::TheIntroDb, &l).unwrap()),
            "tvdb_id=75299&season=6&episode=6&tvdb_episode_id=306220"
        );
        assert_eq!(key(&query(Provider::IntroDb, &l).unwrap()), "imdb_id=tt0141842&season=6&episode=6");
        let film = Lookup {
            tmdb: Some("118340".into()),
            imdb: Some("tt2015381".into()),
            ..Lookup::default()
        };
        assert_eq!(key(&query(Provider::TheIntroDb, &film).unwrap()), "tmdb_id=118340");
        assert_eq!(key(&query(Provider::IntroDb, &film).unwrap()), "imdb_id=tt2015381&is_movie=true");
        // Nothing to ask with.
        let bare = Lookup {
            tvdb: Some("1".into()),
            ..Lookup::default()
        };
        assert!(query(Provider::IntroDb, &bare).is_none());
        assert!(query(Provider::TheIntroDb, &Lookup::default()).is_none());
    }

    #[test]
    fn answers_are_read_as_the_apps_read_them() {
        let tidb = json!({"intro": [{"start_ms": null, "end_ms": 91000}], "recap": [], "credits": [{"start_ms": 3048000, "end_ms": null}], "preview": null});
        let raw = parse_theintrodb(&tidb);
        assert_eq!(
            resolve(Provider::TheIntroDb, &raw, 3324.672),
            vec![
                Segment::new("intro", 0.0, 91.0, Some("theintrodb")),
                Segment::new("credits", 3048.0, 3324.672, Some("theintrodb"))
            ]
        );
        let idb = json!({"intro": {"start_sec": 5.5, "end_sec": 60}, "outro": {"start_sec": 1250}, "recap": {"start_sec": null}});
        assert_eq!(
            resolve(Provider::IntroDb, &parse_introdb(&idb), 1300.0),
            vec![
                Segment::new("intro", 5.5, 60.0, Some("introdb")),
                Segment::new("credits", 1250.0, 1300.0, Some("introdb"))
            ]
        );
        // An open end without a known length: left out, as the apps did.
        assert!(resolve(Provider::IntroDb, &parse_introdb(&json!({"outro": {"start_sec": 10}})), 0.0).is_empty());
    }

    #[test]
    fn refresh_rules() {
        assert!(!stale(100 * DAY, 100 * DAY - 29 * DAY, true));
        assert!(stale(100 * DAY, 100 * DAY - 30 * DAY, true));
        assert!(!stale(100 * DAY, 100 * DAY - 6 * DAY, false));
        assert!(stale(100 * DAY, 100 * DAY - 7 * DAY, false));
    }

    fn library(c: &Connection) {
        c.execute_batch(
            "INSERT INTO items(id, kind, library_id, key, title, identify_state, added_at, updated_at, provider_ids)
                 VALUES ('show', 'series', 'l', 'tmdb:tv:1398', 'Show', 'matched', 0, 0, '{\"Tmdb\":\"1398\",\"Tvdb\":\"75299\",\"Imdb\":\"tt0141842\"}');
             INSERT INTO items(id, kind, library_id, key, title, identify_state, added_at, updated_at, provider_ids, series_id, season_no, episode_no)
                 VALUES ('e1', 'episode', 'l', 'tmdb:tv:1398:s6e6', 'E', 'matched', 0, 0, '{\"Tvdb\":306220}', 'show', 6, 6),
                        ('e2', 'episode', 'l', 'tmdb:tv:1398:s6e7', 'E', 'matched', 0, 0, NULL, 'show', 6, 7),
                        ('sp', 'episode', 'l', 'tmdb:tv:1398:s0e1', 'E', 'matched', 0, 0, NULL, 'show', 0, 1);
             INSERT INTO items(id, kind, library_id, key, title, identify_state, added_at, updated_at, provider_ids, series_id, season_no, episode_no, part_no)
                 VALUES ('p2', 'episode', 'l', 'tmdb:tv:1398:s6e8:p2', 'E', 'matched', 0, 0, NULL, 'show', 6, 8, 2);
             INSERT INTO items(id, kind, library_id, key, title, identify_state, added_at, updated_at, provider_ids, runtime)
                 VALUES ('m', 'movie', 'l', 'tmdb:movie:118340', 'M', 'matched', 0, 0, '{\"Tmdb\":\"118340\",\"Imdb\":\"tt2015381\"}', 7320.0);",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn lookups_from_the_library() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            library(c);
            let e = lookup_for(c, "e1")?.unwrap();
            assert_eq!(e.tvdb.as_deref(), Some("75299"));
            assert_eq!(e.tvdb_episode.as_deref(), Some("306220"));
            assert_eq!((e.season, e.episode), (Some(6), Some(6)));
            let m = lookup_for(c, "m")?.unwrap();
            assert_eq!((m.tmdb.as_deref(), m.runtime, m.season), (Some("118340"), 7320.0, None));
            // Parts of a split episode and shows aren't looked up.
            assert!(lookup_for(c, "p2")?.is_none());
            assert!(lookup_for(c, "show")?.is_none());
            assert_eq!(next_episode(c, "e1")?.as_deref(), Some("e2"));
            // The part after it (a part is not looked up itself, but it is the next episode).
            assert_eq!(next_episode(c, "e2")?.as_deref(), Some("p2"));
            Ok(())
        })
        .await
        .unwrap();
    }

    /// A stand-in for both databases: counts the requests, answers TheIntroDB with times, IntroDB
    /// 404 for the film and 429 (pause) on `limit`.
    async fn databases(hits: Arc<Mutex<Vec<String>>>) -> String {
        use axum::{extract::RawQuery, http::StatusCode, routing::get, Json, Router};
        let h1 = hits.clone();
        let h2 = hits;
        let app = Router::new()
            .route(
                "/tidb",
                get(move |RawQuery(q): RawQuery| {
                    let h = h1.clone();
                    async move {
                        h.lock().unwrap().push(format!("tidb?{}", q.unwrap_or_default()));
                        Json(json!({"intro": [{"start_ms": 1000, "end_ms": 61000}], "credits": [{"start_ms": 3048000, "end_ms": null}]}))
                    }
                }),
            )
            .route(
                "/idb",
                get(move |RawQuery(q): RawQuery| {
                    let h = h2.clone();
                    async move {
                        let q = q.unwrap_or_default();
                        h.lock().unwrap().push(format!("idb?{q}"));
                        if q.contains("is_movie") {
                            (StatusCode::NOT_FOUND, Json(json!({"error": "not found"})))
                        } else {
                            (StatusCode::TOO_MANY_REQUESTS, Json(json!({})))
                        }
                    }
                }),
            );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn looked_up_once_kept_and_never_waited_for() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            library(c);
            Ok(())
        })
        .await
        .unwrap();
        let hits = Arc::new(Mutex::new(Vec::new()));
        let base = databases(hits.clone()).await;
        let config = test_config();
        let cs = Community::with_bases(db.clone(), &config, [format!("{base}/tidb"), format!("{base}/idb")]);
        // Not looked up yet: nothing, and pending (queued, not waited for).
        let a = cs.answer("e1", 3324.672).await;
        assert_eq!(a.online, vec![Vec::<Segment>::new(), Vec::new()]);
        assert!(a.pending);
        assert!(cs.waiting("e1"));
        // The next episode is queued behind it.
        assert_eq!(
            cs.state.lock().unwrap().wanted.iter().cloned().collect::<Vec<_>>(),
            vec!["e1".to_string(), "e2".to_string()]
        );
        cs.state.lock().unwrap().wanted.clear();
        cs.look_up("e1").await;
        assert_eq!(
            hits.lock().unwrap().clone(),
            vec![
                "tidb?tvdb_id=75299&season=6&episode=6&tvdb_episode_id=306220".to_string(),
                "idb?imdb_id=tt0141842&season=6&episode=6".to_string()
            ]
        );
        // TheIntroDB's kept; IntroDB answered 429: paused, nothing kept, not "nothing found".
        let a = cs.answer("e1", 3324.672).await;
        assert_eq!(
            a.online[0],
            vec![
                Segment::new("intro", 1.0, 61.0, Some("theintrodb")),
                Segment::new("credits", 3048.0, 3324.672, Some("theintrodb"))
            ]
        );
        assert!(a.online[1].is_empty());
        // IntroDB is paused, so it isn't queued for it: not pending.
        assert!(!a.pending);
        assert!(cs.state.lock().unwrap().paused_until[1] > now());
        // The film: TheIntroDB, then IntroDB's 404 kept as nothing found.
        cs.state.lock().unwrap().paused_until[1] = 0;
        cs.look_up("m").await;
        let kept = db.call(|c| Ok(kept_keys(c)?)).await.unwrap();
        assert!(kept.contains(&("introdb".to_string(), "imdb_id=tt2015381&is_movie=true".to_string())));
        assert!(kept.contains(&("theintrodb".to_string(), "tmdb_id=118340".to_string())));
        let n = hits.lock().unwrap().len();
        // Kept: asked again, nothing is requested.
        cs.look_up("m").await;
        assert_eq!(hits.lock().unwrap().len(), n);
        let a = cs.answer("m", 7320.0).await;
        assert!(!a.pending);
        // Requests to one database are at least SPACING apart.
        let s = cs.state.lock().unwrap();
        assert!(s.used[0] >= 2);
    }

    #[tokio::test]
    async fn switched_off_nothing_is_asked() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            library(c);
            Ok(())
        })
        .await
        .unwrap();
        let config = test_config();
        config.disabled.set(["community_segments".to_string()].into());
        let cs = Community::with_bases(db, &config, ["http://127.0.0.1:9/a".into(), "http://127.0.0.1:9/b".into()]);
        assert_eq!(cs.answer("e1", 100.0).await, Answer::default());
        assert!(!cs.waiting("e1"));
    }
}
