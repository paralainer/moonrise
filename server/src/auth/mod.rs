//! Moonrise's own accounts (Phase 3): users with argon2id password hashes, access tokens and the
//! device-code sign-in.
//!
//! - **Tokens** are `moonrise_` + 256 random bits (base64url). Only their SHA-256 is stored
//!   (`tokens.hash`); they don't expire and are revoked by logout or the CLI. Clients send
//!   `Authorization: Bearer <token>`. A token's user is cached in memory for 60 s (a revoke
//!   through the API drops it at once; through the CLI it lasts at most that long).
//! - Password attempts are rate-limited per client address and per user name ([`Limiter`]).

pub mod api;
pub mod device;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::http::{HeaderMap, StatusCode};
use base64::Engine;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    api::{ApiError, AppState},
    db::Db,
    library::util,
};

/// How long a token's user stays cached.
const TOKEN_CACHE: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct User {
    pub id: String,
    pub name: String,
    pub is_admin: bool,
}

/// Who is asking.
#[derive(Clone)]
pub enum Caller {
    /// A Moonrise token: the only kind that has watch state.
    Moonrise { user: Arc<User>, token_hash: String },
    /// The CLI's local admin token (`X-Moonrise-Admin`).
    Admin,
}

impl Caller {
    pub fn user(&self) -> Option<&Arc<User>> {
        match self {
            Caller::Moonrise { user, .. } => Some(user),
            Caller::Admin => None,
        }
    }

    pub fn user_id(&self) -> Option<String> {
        self.user().map(|u| u.id.clone())
    }

    /// May use the admin endpoints: the local admin token or a Moonrise admin.
    pub fn is_admin(&self) -> bool {
        match self {
            Caller::Moonrise { user, .. } => user.is_admin,
            Caller::Admin => true,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Caller::Moonrise { .. } => "moonrise",
            Caller::Admin => "admin",
        }
    }
}

/// The `Authorization: Bearer` value of a request.
pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get("authorization")?.to_str().ok()?;
    let (scheme, rest) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| rest.trim()).filter(|t| !t.is_empty())
}

/// Authenticates a request: the local admin token or a Moonrise Bearer token.
pub async fn caller(st: &AppState, headers: &HeaderMap, _q: &HashMap<String, String>) -> Result<Caller, ApiError> {
    if admin_header(st, headers) {
        return Ok(Caller::Admin);
    }
    let t = bearer(headers).ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "no token (Authorization: Bearer)".into()))?;
    let auth = st
        .auth
        .as_ref()
        .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "Moonrise accounts are switched off".into()))?;
    match auth.user_for_token(t).await.map_err(ApiError::internal)? {
        Some((user, token_hash)) => Ok(Caller::Moonrise { user, token_hash }),
        None => Err(ApiError(StatusCode::UNAUTHORIZED, "invalid or revoked token".into())),
    }
}

/// A Moonrise user is required (watch state, device approval).
pub async fn moonrise_caller(st: &AppState, headers: &HeaderMap, q: &HashMap<String, String>) -> Result<(Arc<User>, String), ApiError> {
    if st.auth.is_none() {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Moonrise accounts are switched off".into(),
        ));
    }
    match caller(st, headers, q).await? {
        Caller::Moonrise { user, token_hash } => Ok((user, token_hash)),
        Caller::Admin => Err(ApiError(
            StatusCode::FORBIDDEN,
            "this needs a Moonrise account token (Authorization: Bearer)".into(),
        )),
    }
}

/// An admin is required: a Moonrise admin's token or the CLI's local admin token (403 otherwise).
pub async fn admin_caller(st: &AppState, headers: &HeaderMap, q: &HashMap<String, String>) -> Result<Caller, ApiError> {
    let c = caller(st, headers, q).await?;
    if !c.is_admin() {
        return Err(ApiError(StatusCode::FORBIDDEN, "admins only".into()));
    }
    Ok(c)
}

/// The CLI's local admin token (`/data/secrets/admin.token`).
pub fn admin_header(st: &AppState, headers: &HeaderMap) -> bool {
    let Some(want) = st.admin_token.as_deref() else { return false };
    headers
        .get("x-moonrise-admin")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| !want.is_empty() && ct_eq(v.as_bytes(), want.as_bytes()))
}

/// Constant-time comparison (for secrets compared in memory).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---------------------------------------------------------------- passwords and tokens

pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("argon2: {e}"))?
        .to_string())
}

pub fn verify_password(hash: &str, password: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
}

/// A hash to verify against when the user doesn't exist, so the answer takes as long.
fn dummy_hash() -> &'static str {
    static H: OnceLock<String> = OnceLock::new();
    H.get_or_init(|| hash_password("moonrise-dummy-password").unwrap_or_default())
}

/// `moonrise_` + 32 random bytes, base64url.
pub fn new_token() -> String {
    let b: [u8; 32] = rand::random();
    format!("moonrise_{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b))
}

/// Hex SHA-256: what is stored for tokens and device codes.
pub fn sha256_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn validate_password(password: &str) -> Result<()> {
    if password.chars().count() < 8 {
        bail!("the password must have at least 8 characters");
    }
    if password.len() > 1024 {
        bail!("the password is too long");
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 || name.chars().any(char::is_control) || name.trim() != name {
        bail!("a user name is 1-64 printable characters, without leading or trailing spaces");
    }
    Ok(())
}

// ---------------------------------------------------------------- users (also used by the CLI)

pub fn add_user(c: &Connection, name: &str, password: &str, admin: bool) -> Result<User> {
    validate_name(name)?;
    validate_password(password)?;
    let id = util::ulid();
    let hash = hash_password(password)?;
    c.execute(
        "INSERT INTO users(id, name, password_hash, is_admin, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, name, hash, admin, util::now()],
    )
    .with_context(|| format!("add user {name} (does it exist already?)"))?;
    Ok(User {
        id,
        name: name.to_string(),
        is_admin: admin,
    })
}

pub fn set_password(c: &mut Connection, name: &str, password: &str) -> Result<()> {
    validate_password(password)?;
    let hash = hash_password(password)?;
    if c.execute("UPDATE users SET password_hash = ?2 WHERE name = ?1", params![name, hash])? == 0 {
        bail!("no user {name}");
    }
    Ok(())
}

pub fn user_by_name(c: &Connection, name: &str) -> Result<Option<(User, String)>> {
    Ok(
        c.query_row("SELECT id, name, is_admin, password_hash FROM users WHERE name = ?1", [name], |r| {
            Ok((
                User {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    is_admin: r.get(2)?,
                },
                r.get::<_, String>(3)?,
            ))
        })
        .optional()?,
    )
}

/// Stores a new token for a user; returns the token (shown once, never stored).
pub fn issue_token(c: &Connection, user_id: &str, device_id: Option<&str>, device_name: Option<&str>) -> Result<String> {
    let token = new_token();
    c.execute(
        "INSERT INTO tokens(hash, user_id, device_id, device_name, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![sha256_hex(&token), user_id, device_id.map(clip), device_name.map(clip), util::now()],
    )?;
    Ok(token)
}

fn clip(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(100).collect()
}

/// Forgets every user's watch state, playback sessions, tokens and pending device codes
/// (`moonrise-server users reset-data`); users and passwords stay. With `user`, only theirs.
pub fn reset_data(c: &mut Connection, user: Option<&str>) -> Result<serde_json::Value> {
    let tx = c.transaction()?;
    let uid: Option<String> = match user {
        Some(n) => Some(
            tx.query_row("SELECT id FROM users WHERE name = ?1", [n], |r| r.get(0))
                .optional()?
                .with_context(|| format!("no user {n}"))?,
        ),
        None => None,
    };
    let (w, p) = match &uid {
        Some(u) => ("WHERE user_id = ?1", vec![u.clone()]),
        None => ("", vec![]),
    };
    let n_data = tx.execute(&format!("DELETE FROM user_data {w}"), rusqlite::params_from_iter(p.iter()))?;
    let n_sessions = tx.execute(&format!("DELETE FROM play_sessions {w}"), rusqlite::params_from_iter(p.iter()))?;
    let n_tokens = tx.execute(&format!("DELETE FROM tokens {w}"), rusqlite::params_from_iter(p.iter()))?;
    let n_codes = match &uid {
        Some(u) => tx.execute("DELETE FROM device_codes WHERE approved_by = ?1", [u])?,
        None => tx.execute("DELETE FROM device_codes", [])?,
    };
    tx.commit()?;
    Ok(serde_json::json!({"user_data": n_data, "play_sessions": n_sessions, "tokens": n_tokens, "device_codes": n_codes}))
}

// ---------------------------------------------------------------- the server side

pub struct Auth {
    pub db: Db,
    /// token hash -> (user, cached at)
    cache: Mutex<HashMap<String, (Arc<User>, Instant)>>,
    pub limiter: Limiter,
}

impl Auth {
    pub fn new(db: Db) -> Self {
        Auth {
            db,
            cache: Mutex::new(HashMap::new()),
            limiter: Limiter::default(),
        }
    }

    /// The user of a token (None: unknown or revoked). Records its last use (at most once a
    /// minute, when the cache is refreshed).
    pub async fn user_for_token(&self, token: &str) -> Result<Option<(Arc<User>, String)>> {
        // Any shape is looked up by its hash (tokens made by earlier versions had another prefix).
        if token.is_empty() || token.len() > 100 {
            return Ok(None);
        }
        let hash = sha256_hex(token);
        if let Some((u, at)) = self.cache.lock().unwrap().get(&hash) {
            if at.elapsed() < TOKEN_CACHE {
                return Ok(Some((u.clone(), hash)));
            }
        }
        let h = hash.clone();
        let user = self
            .db
            .call(move |c| {
                let u = c
                    .query_row(
                        "SELECT u.id, u.name, u.is_admin FROM tokens t JOIN users u ON u.id = t.user_id WHERE t.hash = ?1 AND t.revoked_at IS NULL",
                        [&h],
                        |r| Ok(User { id: r.get(0)?, name: r.get(1)?, is_admin: r.get(2)? }),
                    )
                    .optional()?;
                if u.is_some() {
                    c.execute("UPDATE tokens SET last_used = ?2 WHERE hash = ?1", params![h, util::now()])?;
                }
                Ok(u)
            })
            .await?;
        let mut cache = self.cache.lock().unwrap();
        cache.retain(|_, (_, at)| at.elapsed() < TOKEN_CACHE);
        Ok(user.map(|u| {
            let u = Arc::new(u);
            cache.insert(hash.clone(), (u.clone(), Instant::now()));
            (u, hash)
        }))
    }

    /// Checks a user name and password (argon2 on the blocking pool; an unknown name costs
    /// the same). Rate limits are the caller's job.
    pub async fn check_password(&self, name: String, password: String) -> Result<Option<User>> {
        let found = self.db.call(move |c| user_by_name(c, &name)).await?;
        tokio::task::spawn_blocking(move || match found {
            Some((u, hash)) => verify_password(&hash, &password).then_some(u),
            None => {
                verify_password(dummy_hash(), &password);
                None
            }
        })
        .await
        .context("password check")
    }

    pub async fn issue(&self, user_id: String, device_id: Option<String>, device_name: Option<String>) -> Result<String> {
        self.db
            .call(move |c| issue_token(c, &user_id, device_id.as_deref(), device_name.as_deref()))
            .await
    }

    pub async fn revoke(&self, hash: String) -> Result<bool> {
        self.cache.lock().unwrap().remove(&hash);
        self.db
            .call(move |c| {
                Ok(c.execute(
                    "UPDATE tokens SET revoked_at = ?2 WHERE hash = ?1 AND revoked_at IS NULL",
                    params![hash, util::now()],
                )? > 0)
            })
            .await
    }

    /// Drops a user's cached tokens (their rights or tokens changed: the next request reads
    /// the database).
    pub fn forget_user(&self, user_id: &str) {
        self.cache.lock().unwrap().retain(|_, (u, _)| u.id != user_id);
    }

    /// The device a token was issued to.
    pub async fn token_info(&self, hash: String) -> Result<serde_json::Value> {
        self.db
            .call(move |c| {
                Ok(c.query_row("SELECT device_id, device_name, created_at, last_used FROM tokens WHERE hash = ?1", [hash], |r| {
                    Ok(serde_json::json!({"device_id": r.get::<_, Option<String>>(0)?, "device_name": r.get::<_, Option<String>>(1)?,
                        "created_at": r.get::<_, Option<i64>>(2)?.map(crate::library::rfc3339), "last_used": r.get::<_, Option<i64>>(3)?.map(crate::library::rfc3339)}))
                })
                .optional()?
                .unwrap_or(serde_json::Value::Null))
            })
            .await
    }
}

/// Attempts per key in a sliding window (failed logins per address and per user name,
/// device codes started per address).
#[derive(Default)]
pub struct Limiter {
    hits: Mutex<HashMap<String, Vec<Instant>>>,
}

impl Limiter {
    /// Whether `key` has fewer than `max` hits in the last `window`.
    pub fn allowed(&self, key: &str, max: usize, window: Duration) -> bool {
        let mut hits = self.hits.lock().unwrap();
        match hits.get_mut(key) {
            Some(v) => {
                v.retain(|t| t.elapsed() < window);
                v.len() < max
            }
            None => true,
        }
    }

    pub fn hit(&self, key: &str) {
        let mut hits = self.hits.lock().unwrap();
        if hits.len() > 10_000 {
            // Bounded: drop keys with nothing recent.
            hits.retain(|_, v| v.iter().any(|t| t.elapsed() < Duration::from_secs(3600)));
        }
        hits.entry(key.to_string()).or_default().push(Instant::now());
    }
}

/// Failed password attempts allowed per client address and per user name.
pub const LOGIN_MAX_FAILURES: usize = 10;
pub const LOGIN_WINDOW: Duration = Duration::from_mins(15);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_and_hashed() {
        let a = new_token();
        let b = new_token();
        assert_ne!(a, b);
        assert!(a.starts_with("moonrise_"));
        // 32 bytes → 43 base64url characters.
        assert_eq!(a.len(), "moonrise_".len() + 43);
        let h = sha256_hex(&a);
        assert_eq!(h.len(), 64);
        assert_eq!(h, sha256_hex(&a));
        assert_ne!(h, sha256_hex(&b));
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn passwords() {
        let h = hash_password("correct horse").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password(&h, "correct horse"));
        assert!(!verify_password(&h, "correct horsE"));
        assert!(!verify_password("garbage", "x"));
        assert!(validate_password("short").is_err());
    }

    #[tokio::test]
    async fn token_lifecycle() {
        let db = Db::memory().unwrap();
        let u = db.call(|c| add_user(c, "sam", "password123", true)).await.unwrap();
        assert!(
            db.call(|c| add_user(c, "SAM", "password123", false)).await.is_err(),
            "names are case-insensitive"
        );
        let auth = Auth::new(db.clone());
        assert_eq!(
            auth.check_password("sam".into(), "password123".into()).await.unwrap().unwrap().id,
            u.id
        );
        assert!(auth.check_password("sam".into(), "wrong-password".into()).await.unwrap().is_none());
        assert!(auth.check_password("nobody".into(), "password123".into()).await.unwrap().is_none());
        let t = auth
            .issue(u.id.clone(), Some("tv".into()), Some("Living room".into()))
            .await
            .unwrap();
        // Only the hash is stored.
        let stored: String = db
            .call(|c| Ok(c.query_row("SELECT hash FROM tokens", [], |r| r.get(0))?))
            .await
            .unwrap();
        assert_eq!(stored, sha256_hex(&t));
        assert!(!stored.contains(&t["moonrise_".len()..]));
        let (got, hash) = auth.user_for_token(&t).await.unwrap().unwrap();
        assert_eq!(got.name, "sam");
        assert!(auth.user_for_token("moonrise_nope").await.unwrap().is_none());
        assert!(auth.user_for_token("0123456789abcdef0123456789abcdef").await.unwrap().is_none());
        // A token with another prefix (as earlier versions made them) is found by its hash too.
        let other = "pre_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789abcdefg";
        let (o, uid) = (sha256_hex(other), u.id.clone());
        db.call(move |c| {
            Ok(c.execute(
                "INSERT INTO tokens(hash, user_id, created_at) VALUES (?1, ?2, 0)",
                rusqlite::params![o, uid],
            )?)
        })
        .await
        .unwrap();
        assert_eq!(auth.user_for_token(other).await.unwrap().unwrap().0.name, "sam");
        assert!(auth.revoke(hash).await.unwrap());
        assert!(auth.user_for_token(&t).await.unwrap().is_none(), "revoked at once");
        // reset-data drops tokens, keeps users.
        auth.issue(u.id.clone(), None, None).await.unwrap();
        let r = db.call(|c| reset_data(c, None)).await.unwrap();
        assert_eq!(r["tokens"], 3);
        assert!(db.call(|c| user_by_name(c, "sam")).await.unwrap().is_some());
    }

    #[test]
    fn limiter_window() {
        let l = Limiter::default();
        for _ in 0..3 {
            assert!(l.allowed("ip:1", 3, Duration::from_secs(60)));
            l.hit("ip:1");
        }
        assert!(!l.allowed("ip:1", 3, Duration::from_secs(60)));
        assert!(l.allowed("ip:2", 3, Duration::from_secs(60)));
        assert!(l.allowed("ip:1", 3, Duration::from_nanos(1)), "old hits fall out of the window");
    }

    #[test]
    fn bearer_parsing() {
        let mut h = HeaderMap::new();
        h.insert("authorization", "Bearer moonrise_abc".parse().unwrap());
        assert_eq!(bearer(&h), Some("moonrise_abc"));
        h.insert("authorization", "Basic eDp5".parse().unwrap());
        assert_eq!(bearer(&h), None);
    }
}
