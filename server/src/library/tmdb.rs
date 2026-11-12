//! TMDB v3: search, details, seasons, episode ids. Answers are kept in `tmdb_cache` (by the
//! URL without the key) and reused while fresh enough for the caller; requests are spaced
//! (at most ~8 a second, well under TMDB's limit) and retried on 429 with its Retry-After.
//!
//! The key (`TMDB_API_KEY`) is never logged: URLs are logged and cached without it. A key that
//! looks like a JWT (`eyJ…`, a v4 read token) is sent as a Bearer token, any other as
//! `api_key`, as popcorn-tracker does.

use std::fmt::Write as _;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::Value;

use super::util::now;
use crate::{admin::config::Secret, db::Db};

const SPACING: Duration = Duration::from_millis(125);
pub const DAY: i64 = 86_400;

pub struct Tmdb {
    key: Secret,
    http: reqwest::Client,
    db: Db,
    pub language: String,
    pub country: String,
    last: tokio::sync::Mutex<std::time::Instant>,
}

impl Tmdb {
    pub fn new(key: Secret, db: Db, language: String, country: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(super::util::same_origin_redirects())
            .build()
            .expect("http client");
        Tmdb {
            key,
            http,
            db,
            language,
            country,
            last: tokio::sync::Mutex::new(super::util::interval_ago(std::time::Instant::now(), SPACING)),
        }
    }

    /// `path` with `query` (sorted, without the key): the cached answer if younger than
    /// `max_age` seconds, else a fresh one. 404 is answered as `Value::Null`.
    pub async fn get(&self, path: &str, query: &[(&str, String)], max_age: i64) -> Result<Value> {
        let mut q: Vec<(&str, String)> = query.to_vec();
        q.sort();
        let qs: String = q.iter().map(|(k, v)| format!("{k}={}", urlencode(v))).collect::<Vec<_>>().join("&");
        let url = if qs.is_empty() { path.to_string() } else { format!("{path}?{qs}") };
        let cached: Option<(i64, Option<String>, i64)> = {
            let u = url.clone();
            self.db
                .call(move |c| {
                    Ok(
                        c.query_row("SELECT status, body, fetched_at FROM tmdb_cache WHERE url = ?1", [u], |r| {
                            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                        })
                        .optional()?,
                    )
                })
                .await?
        };
        if let Some((status, body, at)) = &cached {
            if now() - at < max_age {
                return Ok(if *status == 200 {
                    serde_json::from_str(body.as_deref().unwrap_or("null"))?
                } else {
                    Value::Null
                });
            }
        }
        let (status, body) = match self.fetch(&url).await {
            Ok(r) => r,
            Err(e) => {
                // Stale beats nothing.
                if let Some((200, Some(body), _)) = cached {
                    tracing::warn!(url, "tmdb: {e:#}; using the cached answer");
                    return Ok(serde_json::from_str(&body)?);
                }
                return Err(e);
            }
        };
        let u = url.clone();
        let b = body.clone();
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT OR REPLACE INTO tmdb_cache(url, status, body, fetched_at) VALUES (?1, ?2, ?3, ?4)",
                    params![u, status, b, now()],
                )?;
                Ok(())
            })
            .await?;
        Ok(if status == 200 {
            serde_json::from_str(body.as_deref().unwrap_or("null"))?
        } else {
            Value::Null
        })
    }

    async fn fetch(&self, url: &str) -> Result<(i64, Option<String>)> {
        let key = self.key.expose();
        if key.is_empty() {
            bail!("no TMDB key");
        }
        // Tests are offline: only answers cached in `tmdb_cache` are seen.
        if cfg!(test) {
            bail!("tmdb {url}: not cached (tests are offline)");
        }
        for attempt in 0..4 {
            {
                let mut last = self.last.lock().await;
                let wait = SPACING.saturating_sub(last.elapsed());
                tokio::time::sleep(wait).await;
                *last = std::time::Instant::now();
            }
            let full = format!("https://api.themoviedb.org/3{url}");
            let mut req = self.http.get(&full);
            if key.starts_with("eyJ") {
                req = req.bearer_auth(key);
            } else {
                req = req.query(&[("api_key", key)]);
            }
            crate::library::count_usage(&self.db, "tmdb", 0, 0).await;
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    // reqwest's error text carries the URL (with the key): don't show it.
                    let kind = if e.is_timeout() {
                        "timeout"
                    } else if e.is_connect() {
                        "connect"
                    } else {
                        "request"
                    };
                    if attempt < 3 {
                        tokio::time::sleep(Duration::from_secs(2 << attempt)).await;
                        continue;
                    }
                    bail!("tmdb {url}: {kind} error");
                }
            };
            let status = resp.status().as_u16();
            match status {
                200 => return Ok((200, Some(resp.text().await.map_err(|_| anyhow::anyhow!("tmdb {url}: read error"))?))),
                404 => return Ok((404, None)),
                429 | 500..=599 if attempt < 3 => {
                    let after = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(2 << attempt);
                    tracing::info!(url, status, after, "tmdb: retrying");
                    tokio::time::sleep(Duration::from_secs(after.min(30))).await;
                }
                _ => bail!("tmdb {url}: HTTP {status}"),
            }
        }
        bail!("tmdb {url}: gave up")
    }

    /// Search results (`results[]`) for a movie or tv title.
    pub async fn search(&self, kind: &str, title: &str, year: Option<i32>) -> Result<Vec<Value>> {
        let mut q = vec![
            ("query", title.to_string()),
            ("language", self.language.clone()),
            ("include_adult", "false".to_string()),
        ];
        if let Some(y) = year {
            q.push((if kind == "movie" { "year" } else { "first_air_date_year" }, y.to_string()));
        }
        let v = self.get(&format!("/search/{kind}"), &q, 30 * DAY).await.context("tmdb search")?;
        Ok(v["results"].as_array().cloned().unwrap_or_default())
    }

    pub async fn movie(&self, id: i64, max_age: i64) -> Result<Value> {
        self.get(
            &format!("/movie/{id}"),
            &[
                ("language", self.language.clone()),
                (
                    "append_to_response",
                    "credits,release_dates,external_ids,images,alternative_titles,translations".into(),
                ),
                ("include_image_language", format!("{},null", self.language)),
            ],
            max_age,
        )
        .await
    }

    pub async fn tv(&self, id: i64, max_age: i64) -> Result<Value> {
        self.get(
            &format!("/tv/{id}"),
            &[
                ("language", self.language.clone()),
                (
                    "append_to_response",
                    "aggregate_credits,content_ratings,external_ids,images,alternative_titles,translations".into(),
                ),
                ("include_image_language", format!("{},null", self.language)),
            ],
            max_age,
        )
        .await
    }

    pub async fn season(&self, tv: i64, season: i64, max_age: i64) -> Result<Value> {
        self.get(
            &format!("/tv/{tv}/season/{season}"),
            &[("language", self.language.clone())],
            max_age,
        )
        .await
    }

    pub async fn episode_ids(&self, tv: i64, season: i64, episode: i64, max_age: i64) -> Result<Value> {
        self.get(&format!("/tv/{tv}/season/{season}/episode/{episode}/external_ids"), &[], max_age)
            .await
    }

    /// TMDB ids for an IMDb/TVDB id (`/find`).
    pub async fn find(&self, external: &str, source: &str) -> Result<Value> {
        self.get(&format!("/find/{external}"), &[("external_source", source.to_string())], 30 * DAY)
            .await
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~,".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn encoding() {
        assert_eq!(super::urlencode("Thor: Love & Thunder"), "Thor%3A%20Love%20%26%20Thunder");
        assert_eq!(super::urlencode("en,null"), "en,null");
    }
}
