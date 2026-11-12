//! OpenSubtitles.com (REST API v1): search by IMDb/TMDb id (plus season and episode for
//! episodes) and by movie hash (computed here from the file), and fetch. Fetched files are kept
//! in `/data/subs` (see `subs.rs`) and listed as tracks of the title on every device.
//!
//! **Tiers.** A consumer key (the admin's own, else the project's built into the release) is all
//! search needs, and downloads work with it too, anonymously: OpenSubtitles allows 5 a day per IP
//! address then. With an account (username and password) downloads count against the account's
//! daily quota instead (20 for a free account, more for VIP). [`OpenSubtitles::quota`] says which
//! tier is in use, how many downloads are left and when the count starts again: for an account
//! from the API (`/infos/user`, asked at most every 5 minutes), anonymously from the last download's
//! answer (kept in `/data/opensubtitles-quota.json` across restarts).
//!
//! Limits respected: the API allows 5 requests/s (calls are spaced 250 ms apart), logins are
//! rare (the token is kept for 20 h and reused), searches are cached for 6 h, and a file
//! already fetched (for any title) is copied rather than downloaded again.

use std::fmt::Write as _;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    admin::config::{OpenSubtitlesCreds, Secret},
    media::Media,
};

const API: &str = "https://api.opensubtitles.com/api/v1";
const MIN_INTERVAL: Duration = Duration::from_millis(250);
const TOKEN_TTL: Duration = Duration::from_hours(20);
const SEARCH_TTL: Duration = Duration::from_hours(6);
/// How long an account's quota from `/infos/user` is trusted.
const QUOTA_TTL: i64 = 300;
/// Downloads a day without an account (per IP address; OpenSubtitles' documented limit).
pub const ANONYMOUS_DOWNLOADS: i64 = 5;
/// Downloads a day with a free account (shown as the reason to add one; the account's own
/// number comes from the API).
pub const ACCOUNT_DOWNLOADS: i64 = 20;

pub struct OpenSubtitles {
    /// Replaced when the settings change (`set_creds`).
    creds: std::sync::RwLock<OpenSubtitlesCreds>,
    http: reqwest::Client,
    /// For subtitle files (links the API answers): no credentials, redirects followed.
    files: reqwest::Client,
    user_agent: String,
    last_call: tokio::sync::Mutex<Instant>,
    /// (token, API base for downloads, obtained at)
    token: tokio::sync::Mutex<Option<(Secret, String, Instant)>>,
    searches: Mutex<HashMap<String, (Instant, Vec<SearchResult>)>>,
    /// The download quota as last known (for the credentials in use).
    quota: Mutex<QuotaState>,
    /// Where the quota is kept across restarts (None: memory only).
    state_file: Option<PathBuf>,
}

/// Which download quota applies and what is left of it, as the API shows it to apps.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Quota {
    /// `anonymous` (the consumer key alone: counted per IP address), `user` or `vip`.
    pub tier: &'static str,
    /// Downloads a day (None: not known yet).
    pub allowed: Option<i64>,
    /// Left now (None: not known yet).
    pub remaining: Option<i64>,
    /// When the count starts again (RFC 3339), when known.
    pub reset_at: Option<String>,
    /// Downloads a day with a (free) account: what adding one would give.
    pub account_allowed: i64,
}

/// What is known of the quota, per set of credentials (`who`: the account name, "" anonymous).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct QuotaState {
    who: String,
    #[serde(default)]
    vip: bool,
    allowed: Option<i64>,
    remaining: Option<i64>,
    /// Unix seconds.
    reset_at: Option<i64>,
    /// When `remaining` was learnt (Unix seconds).
    checked_at: i64,
}

impl QuotaState {
    /// As it stands at `now`: past the reset, the whole allowance is there again.
    fn at(&self, now: i64, account: bool) -> Quota {
        let tier = if !account {
            "anonymous"
        } else if self.vip {
            "vip"
        } else {
            "user"
        };
        let allowed = self.allowed.or((!account).then_some(ANONYMOUS_DOWNLOADS));
        let (remaining, reset_at) = match (self.remaining, self.reset_at) {
            (Some(_), Some(r)) if r <= now => (allowed, None),
            (Some(n), r) => (Some(n.max(0)), r),
            // Anonymous and nothing downloaded since the count was learnt: all of it.
            (None, _) if !account => (allowed, None),
            (None, r) => (None, r),
        };
        Quota {
            tier,
            allowed,
            remaining,
            reset_at: reset_at.map(crate::library::rfc3339),
            account_allowed: ACCOUNT_DOWNLOADS,
        }
    }
}

/// Unix seconds of an RFC 3339 time (`2026-11-04T13:03:16.000Z`; UTC only).
fn parse_utc(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[10] != b'T' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d, hh, mm, ss) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    // Days from civil (Howard Hinnant's algorithm).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

#[derive(Clone, Debug, Serialize)]
pub struct SearchResult {
    /// The file id (what `fetch` takes).
    pub id: String,
    /// The release name.
    pub name: String,
    pub language: Option<String>,
    pub format: &'static str,
    pub downloads: i64,
    pub rating: f64,
    pub hash_match: bool,
    pub hearing_impaired: bool,
    pub machine_translated: bool,
    pub ai_translated: bool,
    pub fps: Option<f64>,
    pub uploader: Option<String>,
    pub upload_date: Option<String>,
}

/// What to search for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    /// ISO 639-1 codes, comma-separated, as OpenSubtitles wants them.
    pub languages: String,
    pub moviehash: Option<String>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub parent_imdb_id: Option<String>,
    pub parent_tmdb_id: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub query: Option<String>,
}

impl Query {
    /// The query string, parameters sorted and lower-case (the API redirects otherwise).
    fn params(&self) -> Vec<(&'static str, String)> {
        let mut p: Vec<(&'static str, String)> = Vec::new();
        let digits = |s: &str| s.trim_start_matches("tt").trim_start_matches('0').to_string();
        if let Some(e) = self.episode {
            p.push(("episode_number", e.to_string()));
        }
        if let Some(v) = &self.imdb_id {
            p.push(("imdb_id", digits(v)));
        }
        p.push(("languages", self.languages.to_ascii_lowercase()));
        if let Some(v) = &self.moviehash {
            p.push(("moviehash", v.to_ascii_lowercase()));
        }
        if let Some(v) = &self.parent_imdb_id {
            p.push(("parent_imdb_id", digits(v)));
        }
        if let Some(v) = &self.parent_tmdb_id {
            p.push(("parent_tmdb_id", v.clone()));
        }
        if let Some(v) = &self.query {
            p.push(("query", v.to_lowercase()));
        }
        if let Some(s) = self.season {
            p.push(("season_number", s.to_string()));
        }
        if let Some(v) = &self.tmdb_id {
            p.push(("tmdb_id", v.clone()));
        }
        p
    }
}

/// The API refused for a reason the client should see (quota, rate limit, credentials).
#[derive(Debug)]
pub struct Refused {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OpenSubtitles: {} ({})", self.message, self.status)
    }
}

impl std::error::Error for Refused {}

impl OpenSubtitles {
    pub fn new(creds: OpenSubtitlesCreds) -> Self {
        OpenSubtitles {
            creds: std::sync::RwLock::new(creds),
            // The API requests carry the key and the account's token: no redirect to another
            // server. The subtitle file itself (a link the API gives, no credentials) may
            // redirect anywhere.
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(crate::library::util::same_origin_redirects())
                .build()
                .expect("http client"),
            files: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("http client"),
            user_agent: format!("MoonrisePlayer v{}", env!("CARGO_PKG_VERSION")),
            last_call: tokio::sync::Mutex::new(crate::library::util::interval_ago(Instant::now(), MIN_INTERVAL)),
            token: tokio::sync::Mutex::new(None),
            searches: Mutex::new(HashMap::new()),
            quota: Mutex::new(QuotaState::default()),
            state_file: None,
        }
    }

    /// Keeps the quota in `path` across restarts (and reads what is there).
    pub fn with_state_file(mut self, path: PathBuf) -> Self {
        if let Some(q) = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<QuotaState>(&b).ok())
        {
            *self.quota.lock().unwrap() = q;
        }
        self.state_file = Some(path);
        self
    }

    /// Search works (a consumer key is there).
    pub fn configured(&self) -> bool {
        self.creds.read().unwrap().usable()
    }

    /// Downloads count against an account (else anonymously, per IP address).
    pub fn has_account(&self) -> bool {
        self.creds.read().unwrap().has_account()
    }

    fn creds(&self) -> OpenSubtitlesCreds {
        self.creds.read().unwrap().clone()
    }

    /// Who the quota is counted for: the account name, "" anonymously.
    fn who(&self) -> String {
        let c = self.creds();
        if c.has_account() {
            c.username.expose().to_string()
        } else {
            String::new()
        }
    }

    /// New credentials: the next call signs in with them (the old login and the cached
    /// searches are dropped).
    pub async fn set_creds(&self, creds: OpenSubtitlesCreds) {
        *self.creds.write().unwrap() = creds;
        *self.token.lock().await = None;
        self.searches.lock().unwrap().clear();
    }

    /// The quota as known now, without asking the API.
    pub fn quota_known(&self) -> Quota {
        let who = self.who();
        let q = self.quota.lock().unwrap().clone();
        let q = if q.who == who {
            q
        } else {
            QuotaState { who, ..Default::default() }
        };
        q.at(crate::library::util::now(), self.has_account())
    }

    /// The quota: for an account asked of the API when the last answer is over 5 minutes old
    /// (a failure answers what is known).
    pub async fn quota(&self) -> Quota {
        if self.has_account() {
            let fresh = {
                let q = self.quota.lock().unwrap();
                q.who == self.who() && crate::library::util::now() - q.checked_at < QUOTA_TTL
            };
            if !fresh {
                if let Err(e) = self.user_info().await {
                    tracing::info!("OpenSubtitles quota: {e:#}");
                }
            }
        }
        self.quota_known()
    }

    /// Records what an answer said of the quota.
    fn note_quota(&self, f: impl FnOnce(&mut QuotaState)) {
        let who = self.who();
        let snapshot = {
            let mut q = self.quota.lock().unwrap();
            if q.who != who {
                *q = QuotaState { who, ..Default::default() };
            }
            f(&mut q);
            q.checked_at = crate::library::util::now();
            q.clone()
        };
        if let Some(path) = &self.state_file {
            if let Ok(b) = serde_json::to_vec(&snapshot) {
                let tmp = path.with_extension("tmp");
                if std::fs::write(&tmp, b).and_then(|()| std::fs::rename(&tmp, path)).is_err() {
                    tracing::debug!("OpenSubtitles quota not saved");
                }
            }
        }
    }

    /// `GET /infos/user`: the account's level and downloads left.
    async fn user_info(&self) -> Result<()> {
        let (token, base) = self.login().await?;
        self.pace().await;
        let resp = self
            .request(reqwest::Method::GET, &format!("{base}/infos/user"))
            .bearer_auth(token.expose())
            .send()
            .await
            .context("OpenSubtitles unreachable")?;
        let status = resp.status().as_u16();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if status != 200 {
            if status == 401 {
                *self.token.lock().await = None;
            }
            return Err(Refused {
                status,
                message: message(&body, "user info failed"),
            }
            .into());
        }
        let d = &body["data"];
        self.note_quota(|q| {
            q.vip = d["vip"].as_bool().unwrap_or(false);
            if let Some(n) = d["allowed_downloads"].as_i64() {
                q.allowed = Some(n);
            }
            if let Some(n) = d["remaining_downloads"].as_i64() {
                q.remaining = Some(n);
            }
        });
        Ok(())
    }

    /// Waits until the next call is allowed (5 per second).
    async fn pace(&self) {
        let mut last = self.last_call.lock().await;
        let wait = MIN_INTERVAL.saturating_sub(last.elapsed());
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        *last = Instant::now();
    }

    fn request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, url)
            .header("Api-Key", self.creds().api_key.expose())
            .header("User-Agent", &self.user_agent)
            .header("Accept", "application/json")
    }

    pub async fn search(&self, q: &Query) -> Result<Vec<SearchResult>> {
        let params = q.params();
        let key = serde_json::to_string(&params)?;
        if let Some((at, r)) = self.searches.lock().unwrap().get(&key) {
            if at.elapsed() < SEARCH_TTL {
                return Ok(r.clone());
            }
        }
        self.pace().await;
        let resp = self
            .request(reqwest::Method::GET, &format!("{API}/subtitles"))
            .query(&params)
            .send()
            .await
            .context("OpenSubtitles unreachable")?;
        let status = resp.status().as_u16();
        if status != 200 {
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            return Err(Refused {
                status,
                message: message(&body, "search failed"),
            }
            .into());
        }
        let body: Value = resp.json().await?;
        let mut results: Vec<SearchResult> = body["data"].as_array().into_iter().flatten().filter_map(parse_result).collect();
        // Hash matches first, then the most downloaded.
        results.sort_by(|a, b| b.hash_match.cmp(&a.hash_match).then(b.downloads.cmp(&a.downloads)));
        let mut cache = self.searches.lock().unwrap();
        cache.retain(|_, (at, _)| at.elapsed() < SEARCH_TTL);
        cache.insert(key, (Instant::now(), results.clone()));
        Ok(results)
    }

    /// A result seen in a recent search.
    pub fn cached_result(&self, id: &str) -> Option<SearchResult> {
        self.searches
            .lock()
            .unwrap()
            .values()
            .flat_map(|(_, r)| r.iter())
            .find(|r| r.id == id)
            .cloned()
    }

    /// Signs in with these credentials (nothing is kept): the account's downloads a day.
    pub async fn check_login(&self) -> Result<Value> {
        self.login().await?;
        let q = self.quota.lock().unwrap().clone();
        Ok(serde_json::json!({"ok": true, "allowed_downloads": q.allowed, "vip": q.vip}))
    }

    async fn login(&self) -> Result<(Secret, String)> {
        let mut token = self.token.lock().await;
        if let Some((t, base, at)) = token.as_ref() {
            if at.elapsed() < TOKEN_TTL {
                return Ok((t.clone(), base.clone()));
            }
        }
        self.pace().await;
        let resp = self
            .request(reqwest::Method::POST, &format!("{API}/login"))
            .json(&{
                let c = self.creds();
                serde_json::json!({"username": c.username.expose(), "password": c.password.expose()})
            })
            .send()
            .await
            .context("OpenSubtitles unreachable")?;
        let status = resp.status().as_u16();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if status != 200 {
            return Err(Refused {
                status,
                message: message(&body, "login failed"),
            }
            .into());
        }
        let t = Secret::new(body["token"].as_str().context("no token in the login answer")?);
        let base = body["base_url"].as_str().filter(|b| !b.is_empty()).map_or_else(
            || API.to_string(),
            |b| format!("https://{}/api/v1", b.trim_start_matches("https://")),
        );
        let user = body["user"].clone();
        if let Some(n) = user["allowed_downloads"].as_i64() {
            tracing::info!(
                allowed_downloads = n,
                vip = user["vip"].as_bool().unwrap_or(false),
                "OpenSubtitles: logged in"
            );
            self.note_quota(|q| {
                q.allowed = Some(n);
                q.vip = user["vip"].as_bool().unwrap_or(false);
            });
        }
        *token = Some((t.clone(), base.clone(), Instant::now()));
        Ok((t, base))
    }

    /// The subtitle file `file_id` (as SRT text, UTF-8 as the API sends it). Uses one of the
    /// day's downloads: the account's, or (no account) the anonymous ones of this IP address.
    pub async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        let id: i64 = file_id.parse().context("bad result id")?;
        let account = self.has_account();
        for attempt in 0..2 {
            let auth = if account { Some(self.login().await?) } else { None };
            let base = auth.as_ref().map_or(API.to_string(), |(_, b)| b.clone());
            self.pace().await;
            let mut req = self
                .request(reqwest::Method::POST, &format!("{base}/download"))
                .json(&serde_json::json!({"file_id": id, "sub_format": "srt"}));
            if let Some((token, _)) = &auth {
                req = req.bearer_auth(token.expose());
            }
            let resp = req.send().await.context("OpenSubtitles unreachable")?;
            let status = resp.status().as_u16();
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            if status == 401 && attempt == 0 && account {
                // The token expired early: log in again.
                *self.token.lock().await = None;
                continue;
            }
            self.note_download(status, &body);
            if status != 200 {
                let mut msg = message(&body, "download refused");
                if let Some(r) = body["reset_time"].as_str() {
                    let _ = write!(msg, " (resets in {r})");
                }
                return Err(Refused { status, message: msg }.into());
            }
            let link = body["link"].as_str().context("no link in the download answer")?;
            tracing::info!(file_id, remaining = ?body["remaining"].as_i64(), anonymous = !account, "OpenSubtitles: downloaded");
            let file = self.files.get(link).header("User-Agent", &self.user_agent).send().await?;
            if !file.status().is_success() {
                bail!("subtitle file: {}", file.status());
            }
            return Ok(file.bytes().await?.to_vec());
        }
        bail!("OpenSubtitles: not logged in")
    }

    /// What a download's answer says of the quota (`remaining`, `requests`, `reset_time_utc`;
    /// 406 = none left).
    fn note_download(&self, status: u16, body: &Value) {
        let remaining = body["remaining"].as_i64().or((status == 406).then_some(0));
        let reset = body["reset_time_utc"].as_str().and_then(parse_utc);
        let used = body["requests"].as_i64();
        if remaining.is_none() && reset.is_none() {
            return;
        }
        let account = self.has_account();
        self.note_quota(|q| {
            if let Some(n) = remaining {
                q.remaining = Some(n.max(0));
                // Anonymous: the allowance is what was used plus what is left.
                if !account {
                    if let Some(u) = used {
                        q.allowed = Some((u + n.max(0)).max(q.allowed.unwrap_or(0)));
                    }
                }
            }
            if reset.is_some() {
                q.reset_at = reset;
            }
        });
    }
}

fn message(body: &Value, default: &str) -> String {
    body["message"]
        .as_str()
        .or_else(|| body["errors"][0].as_str())
        .unwrap_or(default)
        .to_string()
}

fn parse_result(d: &Value) -> Option<SearchResult> {
    let a = &d["attributes"];
    // Multi-part (CD1/CD2) subtitles don't fit one file.
    if a["nb_cd"].as_i64().unwrap_or(1) > 1 {
        return None;
    }
    let file = a["files"].as_array()?.first()?;
    let id = file["file_id"].as_i64()?.to_string();
    let name = a["release"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| file["file_name"].as_str())
        .unwrap_or("Subtitle")
        .to_string();
    Some(SearchResult {
        id,
        name,
        language: a["language"].as_str().map(String::from),
        format: "srt",
        downloads: a["download_count"].as_i64().unwrap_or(0),
        rating: a["ratings"].as_f64().unwrap_or(0.0),
        hash_match: a["moviehash_match"].as_bool().unwrap_or(false),
        hearing_impaired: a["hearing_impaired"].as_bool().unwrap_or(false),
        machine_translated: a["machine_translated"].as_bool().unwrap_or(false),
        ai_translated: a["ai_translated"].as_bool().unwrap_or(false),
        fps: a["fps"].as_f64().filter(|f| *f > 0.0),
        uploader: a["uploader"]["name"].as_str().map(String::from),
        upload_date: a["upload_date"].as_str().map(String::from),
    })
}

/// The OpenSubtitles movie hash: the file size plus the 64-bit little-endian words of its first
/// and last 64 KiB, wrapping.
pub fn movie_hash(size: u64, head: &[u8], tail: &[u8]) -> Option<String> {
    const CHUNK: usize = 65536;
    if size < 2 * CHUNK as u64 || head.len() < CHUNK || tail.len() < CHUNK {
        return None;
    }
    let sum = |b: &[u8]| {
        b[..CHUNK]
            .as_chunks::<8>()
            .0
            .iter()
            .fold(0u64, |h, w| h.wrapping_add(u64::from_le_bytes(*w)))
    };
    Some(format!(
        "{:016x}",
        size.wrapping_add(sum(head)).wrapping_add(sum(&tail[tail.len() - CHUNK..]))
    ))
}

/// The hash of a title's file (two range reads).
pub async fn media_hash(media: &Media, http: &reqwest::Client) -> Option<String> {
    let size = media.size?;
    let head = media.source.read_at(http, 0, 65536).await.ok()?;
    let tail = media.source.read_at(http, size.checked_sub(65536)?, 65536).await.ok()?;
    movie_hash(size, &head, &tail)
}

/// Language codes as OpenSubtitles wants them (ISO 639-1, a few with a region), from what the
/// app sends (ISO 639-1 or -2).
pub fn os_language(code: &str) -> String {
    let c = code.trim().to_ascii_lowercase();
    let map = [
        ("eng", "en"),
        ("rus", "ru"),
        ("ukr", "uk"),
        ("spa", "es"),
        ("fre", "fr"),
        ("fra", "fr"),
        ("ger", "de"),
        ("deu", "de"),
        ("ita", "it"),
        ("por", "pt-pt"),
        ("pt", "pt-pt"),
        ("dut", "nl"),
        ("nld", "nl"),
        ("pol", "pl"),
        ("swe", "sv"),
        ("jpn", "ja"),
        ("kor", "ko"),
        ("chi", "zh-cn"),
        ("zho", "zh-cn"),
        ("zh", "zh-cn"),
        ("ara", "ar"),
        ("heb", "he"),
        ("tur", "tr"),
        ("gre", "el"),
        ("ell", "el"),
        ("cze", "cs"),
        ("ces", "cs"),
        ("fin", "fi"),
        ("dan", "da"),
        ("nor", "no"),
        ("hun", "hu"),
        ("rum", "ro"),
        ("ron", "ro"),
        ("bul", "bg"),
        ("hrv", "hr"),
        ("srp", "sr"),
        ("hin", "hi"),
    ];
    map.iter().find(|(k, _)| *k == c).map(|(_, v)| v.to_string()).unwrap_or(c)
}

/// The search for a title: its ids (an episode's own IMDb id, else the series' ids with season
/// and episode), the movie hash, and the title as text when there are no ids.
pub fn query_for(
    media: &Media,
    series: Option<&crate::media::item::Item>,
    languages: &str,
    hash: Option<String>,
    text: Option<String>,
) -> Query {
    let item = &media.item;
    let mut q = Query {
        languages: languages.split(',').map(os_language).collect::<Vec<_>>().join(","),
        moviehash: hash,
        ..Default::default()
    };
    let episode = item.kind.as_deref() == Some("Episode");
    if let Some(t) = text.filter(|t| !t.trim().is_empty()) {
        q.query = Some(t);
        if episode {
            q.season = item.parent_index_number;
            q.episode = item.index_number;
        }
        return q;
    }
    if episode {
        if let Some(i) = item.provider("Imdb") {
            q.imdb_id = Some(i.to_string());
        } else {
            q.parent_imdb_id = series.and_then(|s| s.provider("Imdb")).map(String::from);
            q.parent_tmdb_id = if q.parent_imdb_id.is_none() {
                series.and_then(|s| s.provider("Tmdb")).map(String::from)
            } else {
                None
            };
            q.season = item.parent_index_number;
            q.episode = item.index_number;
            if q.parent_imdb_id.is_none() && q.parent_tmdb_id.is_none() {
                q.query.clone_from(&item.series_name);
            }
        }
    } else if let Some(i) = item.provider("Imdb") {
        q.imdb_id = Some(i.to_string());
    } else if let Some(t) = item.provider("Tmdb") {
        q.tmdb_id = Some(t.to_string());
    } else {
        q.query.clone_from(&item.name);
    }
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_of_a_known_pattern() {
        // 128 KiB of zeros except the first word (1) and the last word (2): size + 3.
        let mut head = vec![0u8; 65536];
        head[0] = 1;
        let mut tail = vec![0u8; 65536];
        tail[65528] = 2;
        assert_eq!(movie_hash(200_000, &head, &tail).unwrap(), format!("{:016x}", 200_003u64));
        assert!(movie_hash(1000, &head, &tail).is_none());
    }

    #[test]
    fn query_parameters() {
        let q = Query {
            languages: "en".into(),
            parent_imdb_id: Some("tt0141842".into()),
            season: Some(3),
            episode: Some(9),
            moviehash: Some("ABC".into()),
            ..Default::default()
        };
        let p: Vec<String> = q.params().into_iter().map(|(k, v)| format!("{k}={v}")).collect();
        assert_eq!(
            p,
            [
                "episode_number=9",
                "languages=en",
                "moviehash=abc",
                "parent_imdb_id=141842",
                "season_number=3"
            ]
        );
        assert_eq!(os_language("eng"), "en");
        assert_eq!(os_language("pt"), "pt-pt");
        assert_eq!(os_language("ru"), "ru");
    }

    #[test]
    fn quota_tiers_and_reset() {
        assert_eq!(parse_utc("2026-11-04T13:03:16.000Z"), Some(1_793_797_396));
        assert_eq!(crate::library::rfc3339(1_793_797_396), "2026-11-04T13:03:16.000Z");
        assert_eq!(parse_utc("soon"), None);
        let now = 1_793_797_000;
        // Anonymous, nothing known: the documented 5.
        let q = QuotaState::default().at(now, false);
        assert_eq!(
            (q.tier, q.allowed, q.remaining, q.reset_at.as_deref()),
            ("anonymous", Some(5), Some(5), None)
        );
        // After downloads: what the answer said, until the reset.
        let st = QuotaState {
            remaining: Some(3),
            allowed: Some(5),
            reset_at: Some(now + 60),
            ..Default::default()
        };
        assert_eq!(st.at(now, false).remaining, Some(3));
        assert_eq!(st.at(now + 61, false).remaining, Some(5), "past the reset: all of it again");
        // An account: unknown until the API answers; VIP.
        assert_eq!(QuotaState::default().at(now, true).remaining, None);
        let vip = QuotaState {
            vip: true,
            allowed: Some(1000),
            remaining: Some(990),
            ..Default::default()
        }
        .at(now, true);
        assert_eq!((vip.tier, vip.remaining), ("vip", Some(990)));
    }

    #[test]
    fn download_answers_update_the_quota() {
        let os = OpenSubtitles::new(OpenSubtitlesCreds {
            api_key: Secret::new("k"),
            ..Default::default()
        });
        assert!(os.configured() && !os.has_account());
        os.note_download(
            200,
            &serde_json::json!({"requests": 2, "remaining": 3, "reset_time_utc": "2099-01-01T00:00:00.000Z"}),
        );
        let q = os.quota_known();
        assert_eq!((q.tier, q.allowed, q.remaining), ("anonymous", Some(5), Some(3)));
        assert_eq!(q.reset_at.as_deref(), Some("2099-01-01T00:00:00.000Z"));
        os.note_download(
            406,
            &serde_json::json!({"message": "You have downloaded your allowed 5 subtitles for 24h"}),
        );
        assert_eq!(os.quota_known().remaining, Some(0));
        // Another account: its own count.
        *os.creds.write().unwrap() = OpenSubtitlesCreds {
            api_key: Secret::new("k"),
            username: Secret::new("u"),
            password: Secret::new("p"),
        };
        let q = os.quota_known();
        assert_eq!((q.tier, q.remaining), ("user", None));
    }

    #[test]
    fn results_parsed() {
        let d = serde_json::json!({"attributes": {"language": "en", "download_count": 5, "release": "X.S01E01", "moviehash_match": true,
            "files": [{"file_id": 42, "file_name": "x"}], "nb_cd": 1, "uploader": {"name": "u"}}});
        let r = parse_result(&d).unwrap();
        assert_eq!(
            (r.id.as_str(), r.name.as_str(), r.hash_match, r.downloads),
            ("42", "X.S01E01", true, 5)
        );
        let two = serde_json::json!({"attributes": {"nb_cd": 2, "files": [{"file_id": 1}, {"file_id": 2}]}});
        assert!(parse_result(&two).is_none());
    }
}
