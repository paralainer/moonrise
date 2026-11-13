//! "Sign in with ChatGPT": the LLM step on the admin's ChatGPT plan instead of an API key
//! (provider `chatgpt` in `llm/mod.rs`). OpenAI's flow for open-source and self-hosted apps
//! (developers.openai.com/siwc/token-sharing-open-source): OAuth 2.0 authorization code with
//! PKCE and OpenID Connect, a client registered on the first sign-in, tokens bound to this
//! server's host id.
//!
//! The browser part runs on the admin's device: the callback has to reach `127.0.0.1` on the
//! computer running the browser, and the server usually has none. So:
//! 1. `start(redirect_uri, new_account)`: the app gives its own loopback address
//!    (`http://127.0.0.1:<port>/auth/callback`); the server makes the state, nonce and PKCE
//!    verifier (kept here, 15 minutes) and answers the authorization URL. Signing in again uses
//!    the client issued the first time, with the kept ID token as `id_token_hint` (none after a
//!    sign-out); `new_account` registers a new client instead (only when not signed in).
//! 2. The app opens it, OpenAI signs the user in and redirects to the app's listener with
//!    `code`, `state` and (first sign-in) the issued `client_id`.
//! 3. `finish(…)`: the app passes those on; the server exchanges the code (it holds the
//!    verifier), validates the ID token (RS256 signature against OpenAI's JWKS, issuer,
//!    audience, expiry, nonce), checks that plan usage was granted and, signing in again, that
//!    it is the same account (`sub`) before replacing anything, and keeps the tokens.
//!
//! A new registration is kept as soon as its callback names the issued client, before the code
//! is exchanged ("unfinished": a client id, no account yet), as OpenAI's reference client
//! (`openai/sign-in-with-chatgpt-devkit`) does. OpenAI's sign-in page: when the exchange answers
//! `invalid_grant`, "discard that code and start a fresh authorization with the issued client ID
//! retained for this attempt". So `finish` answers [`CodeRefused`] once per attempt, with a retry
//! handle (the failed attempt's `state`), and only `start(… retry)` with that handle reuses the
//! unfinished registration, for the app's immediate second try. Nothing else does: Sign In and Use
//! Another Account register afresh, as the reference client does for a new profile. An unfinished
//! registration is dropped when its sign-in fails in any other way (`invalid_client`, a second
//! refusal, an error OpenAI sends back to the app, …), is listed in `status` as `unfinished`, and
//! [`ChatGpt::forget`] removes it.
//!
//! `ext_agent_host_id`: OpenAI's sign-in page lists it as a "required host identifier, distinct
//! for each host". OpenAI's reference client leaves it out unless its `sendHostId` option is on
//! ("Defaults to false. Enable only when the authorization provider supports
//! ext_agent_host_id."), and sign-ins without it are reported to complete. Moonrise matches the
//! reference client's default: it is sent only with `MOONRISE_CHATGPT_SEND_HOST_ID` on. This is
//! not a proven cause of the `invalid_grant` seen at the exchange (stale browser sessions are the
//! other candidate, hence the app's second try in a private browser session). The host id is still
//! made and kept, for when it is turned on.
//!
//! Every step is logged without codes, tokens, the verifier or the ID token: the registration
//! kind and port at the start; the callback's age, client id and code shape (length, character
//! classes, dot-separated parts: [`code_shape`]); OpenAI's status, error, `error_reason`,
//! `error_description` and request id for each token request. `llm/admin.rs` adds which app
//! sign-in flow made the request (`app_flow`), so a device running an older app shows as such.
//!
//! **Sign-out** (OpenAI's "profiles and sessions" page): the refresh token is revoked first at
//! the `revocation_endpoint` of OpenAI's OpenID configuration (retried on network errors and
//! 5xx), then only the access, refresh and ID tokens are cleared: the registration (client id,
//! account) and the host id stay for the next sign-in. A revocation that isn't confirmed is
//! reported, so the admin can disconnect Moonrise in ChatGPT's settings.
//!
//! The tokens are only ever sent to OpenAI: the token and revocation endpoints (same origin as
//! [`AUTH_BASE`]) and [`API_BASE`], which `llm/mod.rs` uses for this provider whatever address is
//! configured.
//!
//! Tokens are kept in the settings table under `llm.chatgpt` (not a [`crate::admin::settings::KEYS`]
//! setting: never answered by the API, never logged), the host id under `llm.chatgpt_host`. The
//! access token lasts an hour and is refreshed here, one refresh at a time (the refresh token
//! rotates). A refresh the server refuses for good clears the tokens: the admin signs in again.

use std::fmt::Write as _;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::db::Db;

pub const AUTH_BASE: &str = "https://auth.openai.com/api/accounts";
const ISSUER: &str = "https://auth.openai.com";
/// Where this provider's calls go, always: the access token is never sent anywhere else.
pub const API_BASE: &str = "https://api.openai.com/v1";
const RESOURCE: &str = API_BASE;
const SCOPES: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
const AGENT_NAME: &str = "Moonrise";
pub const TOKENS_KEY: &str = "llm.chatgpt";
pub const HOST_KEY: &str = "llm.chatgpt_host";
pub const CALLBACK_PATH: &str = "/auth/callback";
const PENDING_TTL: Duration = Duration::from_mins(15);
/// How long OpenAI's OpenID configuration and keys are kept.
const DISCOVERY_TTL: Duration = Duration::from_hours(24);
/// Send the host id (`ext_agent_host_id`) in authorization requests (off by default, see above).
pub const SEND_HOST_ID_ENV: &str = "MOONRISE_CHATGPT_SEND_HOST_ID";
const REGISTRATION_ENTRY: &str = "dynamic_agent_client";

/// An answer from OpenAI's token endpoint other than 200. Shows no token: OpenAI's code, reason,
/// description (scrubbed) and request id.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenError {
    pub status: u16,
    pub code: String,
    pub reason: String,
    pub description: String,
    pub request_id: String,
}

impl TokenError {
    /// The refresh token (or code) is no good for good: sign in again.
    pub fn terminal(&self) -> bool {
        matches!(
            self.code.as_str(),
            "invalid_grant"
                | "invalid_refresh_token"
                | "token_expired"
                | "refresh_token_expired"
                | "refresh_token_invalidated"
                | "refresh_token_reused"
        ) || matches!(
            self.reason.as_str(),
            "refresh_token_invalidated" | "refresh_token_reused" | "refresh_token_expired"
        )
    }

    /// What OpenAI said, for a message: `invalid_grant: <description or reason>; request <id>`.
    fn detail(&self) -> String {
        let mut out = if self.code.is_empty() {
            format!("HTTP {}", self.status)
        } else {
            self.code.clone()
        };
        let said = if self.description.is_empty() {
            &self.reason
        } else {
            &self.description
        };
        if !said.is_empty() {
            out.push_str(": ");
            out.push_str(said);
        }
        if !self.request_id.is_empty() {
            out.push_str("; request ");
            out.push_str(&self.request_id);
        }
        out
    }
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ChatGPT sign-in: HTTP {} {}", self.status, self.detail())
    }
}

impl std::error::Error for TokenError {}

/// OpenAI refused the sign-in's code at the exchange (`invalid_grant`), the first time for this
/// attempt. The registration is kept for one more try: `start` with `retry` = this attempt's
/// `state` uses it, as OpenAI asks.
#[derive(Debug)]
pub struct CodeRefused(pub TokenError);

impl std::fmt::Display for CodeRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "OpenAI didn't accept this sign-in ({}); one more try can use the same registration",
            self.0.detail()
        )
    }
}

impl std::error::Error for CodeRefused {}

/// A client id as issued by OpenAI's registration (not the registration entry point).
fn issued_client_id(c: &str) -> bool {
    c != REGISTRATION_ENTRY && (1..=200).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// For the log: a new registration, an unfinished one (its retry) or one signed in before.
fn registration_kind(t: Option<&Tokens>) -> &'static str {
    match t {
        None => "new",
        Some(t) if t.pending() => "unfinished",
        Some(_) => "returning",
    }
}

/// What a code looks like, for the log (never the code, nor any part of it): its length,
/// whether it has OpenAI's `ac_` prefix, which characters it has (letters and digits, and those
/// a URL could have changed: `+`, `/`, `=`, `%`, a space, …), and its dot-separated parts: their
/// count and lengths and, when the first part is a JWT/JWE header, the header's field names and
/// its `alg`/`enc`/`typ` (public metadata, not the code). For example `1903 chars, no ac_
/// prefix, chars [A-Z a-z 0-9 - _ .], 3 parts (36+1520+345), JWT-like header {alg,typ} alg=RS256`.
fn code_shape(code: &str) -> String {
    let mut out = format!(
        "{} chars, {}",
        code.len(),
        if code.starts_with("ac_") { "ac_ prefix" } else { "no ac_ prefix" }
    );
    let mut classes: Vec<&str> = Vec::new();
    for (name, hit) in [
        ("A-Z", code.bytes().any(|b| b.is_ascii_uppercase())),
        ("a-z", code.bytes().any(|b| b.is_ascii_lowercase())),
        ("0-9", code.bytes().any(|b| b.is_ascii_digit())),
    ] {
        if hit {
            classes.push(name);
        }
    }
    for (name, byte) in [
        ("-", b'-'),
        ("_", b'_'),
        (".", b'.'),
        ("~", b'~'),
        ("+", b'+'),
        ("/", b'/'),
        ("=", b'='),
        ("%", b'%'),
        ("space", b' '),
    ] {
        if code.bytes().any(|b| b == byte) {
            classes.push(name);
        }
    }
    if code.bytes().any(|b| !(b.is_ascii_alphanumeric() || b"-_.~+/=% ".contains(&b))) {
        classes.push("other");
    }
    let _ = write!(out, ", chars [{}]", classes.join(" "));
    if code
        .as_bytes()
        .windows(3)
        .any(|w| w[0] == b'%' && w[1].is_ascii_hexdigit() && w[2].is_ascii_hexdigit())
    {
        out.push_str(", has %XX escapes (still URL-encoded?)");
    }
    let parts: Vec<&str> = code.split('.').collect();
    if parts.len() > 1 {
        let lens: Vec<String> = parts.iter().take(6).map(|p| p.len().to_string()).collect();
        let _ = write!(
            out,
            ", {} parts ({}{})",
            parts.len(),
            lens.join("+"),
            if parts.len() > 6 { "+…" } else { "" }
        );
        if let Some(Value::Object(h)) = b64_decode(parts[0]).and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
            let mut keys: Vec<&str> = h
                .keys()
                .map(String::as_str)
                .filter(|k| k.len() <= 20 && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
                .take(10)
                .collect();
            keys.sort_unstable();
            let _ = write!(
                out,
                ", {} header {{{}}}",
                if parts.len() == 5 { "JWE-like" } else { "JWT-like" },
                keys.join(",")
            );
            for k in ["alg", "enc", "typ"] {
                if let Some(v) = h
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|v| v.len() <= 24 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b"+-_".contains(&b)))
                {
                    let _ = write!(out, " {k}={v}");
                }
            }
        }
    }
    out
}

/// One account registration: the client issued for it, its account and its tokens. `Debug`
/// shows the account only.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tokens {
    /// This server's label for the registration (stable).
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub id_token: String,
    /// Unix seconds.
    #[serde(default)]
    pub expires_at: i64,
    #[serde(default)]
    pub email: String,
    /// The account: the validated ID token's `sub`.
    #[serde(default)]
    pub sub: String,
    #[serde(default)]
    pub scope: String,
    /// Signed out by the admin (rather than the sign-in having ended by itself).
    #[serde(default)]
    pub signed_out: bool,
    /// Unix seconds: when an unfinished registration was kept.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub kept_at: i64,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

impl Tokens {
    /// A registration whose sign-in hasn't completed yet: the client OpenAI issued, no account.
    pub fn pending(&self) -> bool {
        self.sub.is_empty()
            && self.email.is_empty()
            && self.id_token.is_empty()
            && self.refresh_token.is_empty()
            && !self.client_id.is_empty()
    }
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("id", &self.id)
            .field("email", &self.email)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// The account registrations kept (OpenAI's "profiles and sessions": each registration's
/// account, issued client id and credentials stay separate, even with the same email), and the
/// one in use.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Accounts {
    #[serde(default)]
    pub active: String,
    #[serde(default)]
    pub accounts: Vec<Tokens>,
}

impl Accounts {
    /// As stored (one registration, as kept before there could be several: label `1`).
    pub fn from_stored(v: Option<&Value>) -> Accounts {
        let Some(v) = v else { return Accounts::default() };
        if v.get("accounts").is_some() {
            let mut a: Accounts = serde_json::from_value(v.clone()).unwrap_or_default();
            // An unfinished registration is never the one in use (kept so by an earlier build).
            if a.active().is_some_and(Tokens::pending) {
                a.active.clear();
            }
            return a;
        }
        match serde_json::from_value::<Tokens>(v.clone()) {
            Ok(mut t) => {
                t.id = "1".into();
                Accounts {
                    active: t.id.clone(),
                    accounts: vec![t],
                }
            }
            Err(_) => Accounts::default(),
        }
    }

    pub fn active(&self) -> Option<&Tokens> {
        self.accounts.iter().find(|t| t.id == self.active)
    }

    fn active_mut(&mut self) -> Option<&mut Tokens> {
        let id = self.active.clone();
        self.accounts.iter_mut().find(|t| t.id == id)
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut Tokens> {
        self.accounts.iter_mut().find(|t| t.id == id)
    }

    /// A label not used yet.
    #[allow(clippy::maybe_infinite_iter, reason = "finds one within accounts.len() + 1 tries")]
    fn new_id(&self) -> String {
        (1..)
            .map(|n: u32| n.to_string())
            .find(|id| !self.accounts.iter().any(|t| &t.id == id))
            .unwrap()
    }

    /// What the app shows to tell registrations apart: the email, numbered when two share it.
    fn label(&self, t: &Tokens) -> String {
        let name = if t.email.is_empty() {
            "ChatGPT account".to_string()
        } else {
            t.email.clone()
        };
        let same: Vec<&Tokens> = self.accounts.iter().filter(|o| o.email == t.email).collect();
        match same.iter().position(|o| o.id == t.id) {
            Some(i) if same.len() > 1 => format!("{name} ({})", i + 1),
            _ => name,
        }
    }
}

/// A stored `llm.chatgpt` whose account in use can be used (a refresh token is there).
pub fn signed_in(stored: Option<&Value>) -> bool {
    Accounts::from_stored(stored).active().is_some_and(|t| !t.refresh_token.is_empty())
}

struct Pending {
    verifier: String,
    nonce: String,
    redirect_uri: String,
    /// Signing in again: the registration (label, client id, account) made before.
    returning: Option<Tokens>,
    /// The second try after a refused code (no third: a refusal now is final).
    retry: bool,
    at: Instant,
}

/// What sign-out managed.
#[derive(Debug, PartialEq)]
pub struct SignedOut {
    /// OpenAI confirmed the refresh token is revoked (or there was none to revoke).
    pub revoked: bool,
}

pub struct ChatGpt {
    auth_base: String,
    /// OpenAI's OpenID configuration and JWKS, with when they were fetched.
    discovery: tokio::sync::Mutex<Option<(Value, Instant)>>,
    jwks: tokio::sync::Mutex<Option<(Value, Instant)>>,
    /// Waits between revocation tries.
    backoff: Duration,
    /// The host id, once read or made.
    host: Mutex<Option<String>>,
    db: Mutex<Option<Db>>,
    /// Loaded on first use; the lock also serializes refreshes.
    tokens: tokio::sync::Mutex<Option<Accounts>>,
    pending: Mutex<HashMap<String, Pending>>,
    /// Attempts whose code OpenAI refused, by their `state`: the registration one more try may
    /// use (once, within [`PENDING_TTL`]).
    retries: Mutex<HashMap<String, (String, Instant)>>,
    /// Tokens to cut out of any text shown or logged.
    secrets: Mutex<Vec<String>>,
    /// Send `ext_agent_host_id` ([`SEND_HOST_ID_ENV`]).
    send_host_id: bool,
    http: reqwest::Client,
}

static CHATGPT: OnceLock<ChatGpt> = OnceLock::new();

/// The server's one instance.
pub fn get() -> &'static ChatGpt {
    CHATGPT.get_or_init(|| ChatGpt::new(AUTH_BASE))
}

/// Where tokens are kept (the server's database).
pub fn init(db: Db) {
    *get().db.lock().unwrap() = Some(db);
}

/// `text` without any token this server holds.
pub fn scrub(text: &str) -> String {
    CHATGPT.get().map_or_else(|| text.to_string(), |c| c.scrub(text))
}

/// An answer that says the plan can't be used now (not worth retrying).
pub fn plan_error(status: u16, body: &str) -> Option<anyhow::Error> {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = v["error"]["code"].as_str().or(v["code"].as_str()).unwrap_or("");
    if let Some(e) = plan_error_code(code) {
        return Some(e);
    }
    (status == 403).then(|| {
        anyhow::anyhow!(
            "ChatGPT refused the request (HTTP 403: {})",
            v["error"]["message"]
                .as_str()
                .unwrap_or("not allowed")
                .chars()
                .take(200)
                .collect::<String>()
        )
    })
}

/// The plan errors that need the admin (OpenAI's `subscription_sharing_*` codes).
pub fn plan_error_code(code: &str) -> Option<anyhow::Error> {
    match code {
        "subscription_sharing_usage_limit_exceeded" => Some(anyhow::anyhow!(
            "ChatGPT plan: Moonrise's usage limit is reached (see ChatGPT settings → Usage); unclear titles wait in Fix Match until then"
        )),
        "subscription_sharing_user_not_eligible" => Some(anyhow::anyhow!("ChatGPT plan: this account can't use its plan in other apps")),
        _ => None,
    }
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn random() -> String {
    b64(&rand::random::<[u8; 32]>())
}

fn now() -> i64 {
    crate::library::util::now()
}

/// `http://127.0.0.1:<port>/auth/callback`, as OpenAI requires for the redirect.
fn valid_redirect(uri: &str) -> bool {
    let Some(rest) = uri.strip_prefix("http://127.0.0.1:") else {
        return false;
    };
    let Some((port, path)) = rest.split_once('/') else { return false };
    port.parse::<u16>().is_ok_and(|p| p > 0) && format!("/{path}") == CALLBACK_PATH
}

fn b64_decode(part: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(part.trim_end_matches('='))
        .ok()
}

/// The claims of a JWT, unverified: only for a token validated when it was kept.
fn claims(jwt: &str) -> Option<Value> {
    serde_json::from_slice(&b64_decode(jwt.split('.').nth(1)?)?).ok()
}

/// Whether `jwt` is signed (RS256) by one of the keys of `jwks`. Err: no key with its `kid`.
fn signed_by(jwt: &str, jwks: &Value) -> Result<bool> {
    let mut parts = jwt.split('.');
    let (Some(h), Some(p), Some(sig), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Ok(false);
    };
    let Some(header) = b64_decode(h).and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else {
        return Ok(false);
    };
    if header["alg"] != "RS256" {
        return Ok(false);
    }
    let kid = header["kid"].as_str();
    let key = jwks["keys"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|k| k["kty"] == "RSA" && k["use"].as_str().is_none_or(|u| u == "sig") && k["alg"].as_str().is_none_or(|a| a == "RS256"))
        .find(|k| kid.is_none() || k["kid"].as_str() == kid)
        .context("no OpenAI signing key for this ID token")?;
    let (Some(n), Some(e), Some(sig)) = (
        key["n"].as_str().and_then(b64_decode),
        key["e"].as_str().and_then(b64_decode),
        b64_decode(sig),
    ) else {
        return Ok(false);
    };
    let public = ring::signature::RsaPublicKeyComponents { n: &n, e: &e };
    Ok(public
        .verify(&ring::signature::RSA_PKCS1_2048_8192_SHA256, format!("{h}.{p}").as_bytes(), &sig)
        .is_ok())
}

impl ChatGpt {
    pub fn new(auth_base: &str) -> Self {
        ChatGpt {
            auth_base: auth_base.trim_end_matches('/').to_string(),
            discovery: tokio::sync::Mutex::new(None),
            jwks: tokio::sync::Mutex::new(None),
            backoff: Duration::from_secs(1),
            host: Mutex::new(None),
            db: Mutex::new(None),
            tokens: tokio::sync::Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            retries: Mutex::new(HashMap::new()),
            secrets: Mutex::new(Vec::new()),
            send_host_id: std::env::var(SEND_HOST_ID_ENV)
                .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")),
            // Redirects are never followed: a form with a code, verifier or refresh token must
            // not be carried to another address. A redirect is an error.
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!("Moonrise/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("http client"),
        }
    }

    fn scrub(&self, text: &str) -> String {
        let mut out = text.to_string();
        for s in self.secrets.lock().unwrap().iter().filter(|s| s.len() >= 8) {
            out = out.replace(s.as_str(), "***");
        }
        out
    }

    fn remember_all(&self, a: &Accounts) {
        for t in &a.accounts {
            self.remember(t);
        }
    }

    fn remember(&self, t: &Tokens) {
        let mut s = self.secrets.lock().unwrap();
        for v in [&t.access_token, &t.refresh_token, &t.id_token] {
            if !v.is_empty() && !s.contains(v) {
                s.push(v.clone());
            }
        }
        // Old tokens stay listed for a while: keep the list short.
        let n = s.len();
        if n > 30 {
            s.drain(..n - 30);
        }
    }

    fn database(&self) -> Option<Db> {
        self.db.lock().unwrap().clone()
    }

    async fn read_key(&self, key: &'static str) -> Result<Option<Value>> {
        let Some(db) = self.database() else { return Ok(None) };
        db.call(move |c| {
            use rusqlite::OptionalExtension;
            let v: Option<String> = c
                .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
                .optional()?;
            Ok(v.and_then(|v| serde_json::from_str(&v).ok()))
        })
        .await
    }

    async fn write_key(&self, key: &'static str, v: Option<Value>) -> Result<()> {
        let Some(db) = self.database() else { return Ok(()) };
        db.call(move |c| crate::admin::settings::write(c, &[(key.to_string(), v)])).await
    }

    async fn persist(&self, a: &Accounts) -> Result<()> {
        self.write_key(
            TOKENS_KEY,
            (!a.accounts.is_empty()).then(|| serde_json::to_value(a).expect("tokens")),
        )
        .await
    }

    /// The registrations kept, loaded once into `g`.
    async fn loaded<'a>(&self, g: &'a mut Option<Accounts>) -> Result<&'a mut Accounts> {
        if g.is_none() {
            let a = Accounts::from_stored(self.read_key(TOKENS_KEY).await?.as_ref());
            self.remember_all(&a);
            *g = Some(a);
        }
        Ok(g.as_mut().unwrap())
    }

    /// The registrations kept (loaded once).
    async fn accounts(&self) -> Result<Accounts> {
        let mut g = self.tokens.lock().await;
        Ok(self.loaded(&mut g).await?.clone())
    }

    /// The registration in use.
    #[cfg(test)]
    async fn current(&self) -> Result<Option<Tokens>> {
        Ok(self.accounts().await?.active().cloned())
    }

    /// For the admin API: `{signed_in, email?, needs_sign_in?, account?, accounts, unfinished}`
    /// (`email`, `account`: the registration in use, signed in or the one a sign-in again returns
    /// to; `needs_sign_in`: its sign-in ended by itself; `accounts`: every registration kept,
    /// `{id, label, email, signed_in, active}`; `unfinished`: registrations OpenAI issued whose
    /// sign-in never completed, `{id, kept_at}`, which no sign-in uses but its own retry).
    pub async fn status(&self) -> Value {
        let a = self.accounts().await.unwrap_or_default();
        // A registration whose sign-in didn't complete isn't an account yet: the next sign-in
        // uses it (no list entry, no "Sign In as").
        let list: Vec<Value> = a
            .accounts
            .iter()
            .filter(|t| !t.pending())
            .map(|t| json!({"id": t.id, "label": a.label(t), "email": t.email, "signed_in": !t.refresh_token.is_empty(), "active": t.id == a.active}))
            .collect();
        let mut v = match a.active().filter(|t| !t.pending()) {
            Some(t) if !t.refresh_token.is_empty() => json!({"signed_in": true, "email": t.email, "account": t.id}),
            Some(t) if t.signed_out => json!({"signed_in": false, "email": t.email, "account": t.id}),
            Some(t) => json!({"signed_in": false, "email": t.email, "account": t.id, "needs_sign_in": true}),
            None => json!({"signed_in": false}),
        };
        v["accounts"] = json!(list);
        v["unfinished"] = json!(a
            .accounts
            .iter()
            .filter(|t| t.pending())
            .map(|t| json!({"id": t.id, "kept_at": t.kept_at}))
            .collect::<Vec<_>>());
        v
    }

    /// This server's host id (made once).
    async fn host_id(&self) -> Result<String> {
        if let Some(id) = self.host.lock().unwrap().clone() {
            return Ok(id);
        }
        let id = match self.read_key(HOST_KEY).await?.and_then(|v| v.as_str().map(String::from)) {
            Some(id) => id,
            None => {
                let id = format!("urn:uuid:{}", crate::db::uuid_v4());
                self.write_key(HOST_KEY, Some(json!(id))).await?;
                id
            }
        };
        *self.host.lock().unwrap() = Some(id.clone());
        Ok(id)
    }

    /// Step 1: the URL to open in a browser on the device whose loopback `redirect_uri` is.
    /// Signs in with the registration `account` (default: the one in use) again, or with
    /// `new_account` registers Moonrise for another ChatGPT account (a new registration; the
    /// others are kept as they are). `retry`: the `state` of an attempt whose code OpenAI refused
    /// ([`CodeRefused`]): one more try with the same registration (the other arguments aside).
    /// An unfinished registration is used only so.
    pub async fn start(&self, redirect_uri: &str, new_account: bool, account: Option<&str>, retry: Option<&str>) -> Result<String> {
        if !valid_redirect(redirect_uri) {
            bail!("redirect_uri: http://127.0.0.1:<port>{CALLBACK_PATH}");
        }
        let host = self.host_id().await?;
        let a = self.accounts().await?;
        let is_retry = retry.is_some_and(|r| !r.is_empty());
        let returning = if let Some(r) = retry.filter(|r| !r.is_empty()) {
            let id = self
                .retries
                .lock()
                .unwrap()
                .remove(r)
                .filter(|(_, at)| at.elapsed() < PENDING_TTL)
                .map(|(id, _)| id);
            let Some(t) = id.and_then(|id| a.accounts.iter().find(|t| t.id == id).cloned()) else {
                tracing::warn!("ChatGPT sign-in retry for an unknown, used or expired attempt");
                bail!("this sign-in can't be tried again any more: tap Sign In to start over");
            };
            Some(t)
        } else {
            match (new_account, account.filter(|s| !s.is_empty())) {
                (true, _) => None,
                (false, Some(id)) => Some(
                    a.accounts
                        .iter()
                        .find(|t| t.id == id && !t.pending())
                        .cloned()
                        .context("no such ChatGPT account kept here")?,
                ),
                (false, None) => a.active().filter(|t| !t.pending()).cloned(),
            }
        }
        .filter(|t| !t.client_id.is_empty());
        let (state, nonce, verifier) = (random(), random(), random());
        let challenge = b64(&Sha256::digest(verifier.as_bytes()));
        let mut params: Vec<(&str, String)> = Vec::new();
        match &returning {
            // Signing in again: the client issued the first time, the account it was for.
            Some(t) => {
                params.push(("client_id", t.client_id.clone()));
                if !t.id_token.is_empty() {
                    params.push(("id_token_hint", t.id_token.clone()));
                }
                if !t.email.is_empty() {
                    params.push(("login_hint", t.email.clone()));
                }
            }
            None => {
                params.push(("client_id", "dynamic_agent_client".into()));
                params.push(("agent_name_hint", AGENT_NAME.into()));
            }
        }
        if self.send_host_id {
            params.push(("ext_agent_host_id", host));
        }
        params.extend([
            ("response_type", "code".into()),
            ("redirect_uri", redirect_uri.to_string()),
            ("scope", SCOPES.into()),
            ("resource", RESOURCE.into()),
            ("state", state.clone()),
            ("nonce", nonce.clone()),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256".into()),
        ]);
        let url = reqwest::Url::parse_with_params(&format!("{}/authorize", self.auth_base), &params)?;
        tracing::info!(
            registration = registration_kind(returning.as_ref()),
            account = returning.as_ref().map_or("", |t| t.id.as_str()),
            client_id = returning.as_ref().map_or(REGISTRATION_ENTRY, |t| t.client_id.as_str()),
            redirect_port = redirect_uri.trim_start_matches("http://127.0.0.1:").split('/').next().unwrap_or(""),
            host_id_sent = self.send_host_id,
            retry = is_retry,
            "ChatGPT sign-in started"
        );
        self.retries.lock().unwrap().retain(|_, (_, at)| at.elapsed() < PENDING_TTL);
        let mut p = self.pending.lock().unwrap();
        p.retain(|_, v| v.at.elapsed() < PENDING_TTL);
        p.insert(
            state,
            Pending {
                verifier,
                nonce,
                redirect_uri: redirect_uri.to_string(),
                returning,
                retry: is_retry,
                at: Instant::now(),
            },
        );
        Ok(url.to_string())
    }

    /// [`Self::finish_from`] for an app that tries again by itself (the tests).
    #[cfg(test)]
    pub async fn finish(&self, state: &str, code: &str, client_id: Option<&str>) -> Result<Tokens> {
        self.finish_from(state, code, client_id, true).await
    }

    /// Step 3: what came back to the app's listener. Keeps the tokens; answers the account.
    /// Nothing kept changes unless the new sign-in is valid, allows plan usage and, signing in
    /// again, is the same account. A new registration is kept before the exchange; if the
    /// sign-in then fails, it is dropped again, except after a first refused code
    /// ([`CodeRefused`]), when it waits for the one retry. `app_retries`: whether the app tries
    /// again by itself after a refused code. Apps from before server 0.22.2 don't (they send no
    /// `app_flow` nor `callback_params`): for them the kept registration is of no use, so it is
    /// dropped at once, and the message says to update the app.
    pub async fn finish_from(&self, state: &str, code: &str, client_id: Option<&str>, app_retries: bool) -> Result<Tokens> {
        let Some(p) = self.pending.lock().unwrap().remove(state).filter(|p| p.at.elapsed() < PENDING_TTL) else {
            tracing::warn!("ChatGPT sign-in callback for an unknown, used or expired attempt");
            bail!("this sign-in expired or was already used: start again");
        };
        let given = client_id.filter(|c| !c.is_empty());
        tracing::info!(
            age_s = p.at.elapsed().as_secs(),
            registration = registration_kind(p.returning.as_ref()),
            retry = p.retry,
            client_id = given.unwrap_or(""),
            code = code_shape(code),
            "ChatGPT sign-in callback"
        );
        let kept = match (p.returning.clone(), given) {
            (None, Some(c)) if issued_client_id(c) => {
                // Registered: kept now, before the one-time code is exchanged, so that the one
                // retry after a refused code can use it rather than register Moonrise again.
                self.keep_registration(c).await?
            }
            (None, Some(_)) => {
                tracing::warn!("ChatGPT sign-in callback named no issued client");
                bail!("ChatGPT didn't finish registering Moonrise (the answer named no issued client): sign in again")
            }
            (None, None) => {
                tracing::warn!("ChatGPT sign-in callback without a client id");
                bail!("ChatGPT didn't register Moonrise (no client_id in the answer): sign in again")
            }
            (Some(kept), _) => kept,
        };
        match self.complete(&p, &kept, given, code).await {
            Ok(t) => Ok(t),
            Err(e) => {
                let refused = e.downcast_ref::<TokenError>().filter(|t| t.code == "invalid_grant").cloned();
                if let (Some(t), false) = (refused, p.retry) {
                    if !app_retries {
                        tracing::warn!("ChatGPT sign-in: this app doesn't try again by itself (an app from before server 0.22.2)");
                        if kept.pending() {
                            self.drop_unfinished(&kept.id).await;
                        }
                        bail!(
                            "OpenAI didn't accept this sign-in ({}). This version of the Moonrise app doesn't try again by itself: update the app on this device, then tap Sign In",
                            t.detail()
                        );
                    }
                    // OpenAI: discard the code, start a fresh authorization with the issued
                    // client retained for this attempt.
                    self.retries
                        .lock()
                        .unwrap()
                        .insert(state.to_string(), (kept.id.clone(), Instant::now()));
                    return Err(CodeRefused(t).into());
                }
                if kept.pending() && self.drop_unfinished(&kept.id).await {
                    return Err(e.context("Moonrise's unfinished registration was dropped, so Sign In starts afresh"));
                }
                Err(e)
            }
        }
    }

    /// The rest of [`Self::finish`], once the registration is known.
    async fn complete(&self, p: &Pending, kept: &Tokens, given: Option<&str>, code: &str) -> Result<Tokens> {
        if given.is_some_and(|c| c != kept.client_id) {
            tracing::warn!(kept = kept.client_id, "ChatGPT sign-in callback for another registration");
            bail!("ChatGPT answered for a different registration; nothing was changed")
        }
        let client_id = kept.client_id.clone();
        let form = [
            ("grant_type", "authorization_code"),
            ("client_id", &client_id),
            ("code", code),
            ("code_verifier", &p.verifier),
            ("redirect_uri", &p.redirect_uri),
            ("resource", RESOURCE),
        ];
        let v = self.token_request(&form).await?;
        let mut t = match self.tokens_from(&v, &client_id, Some(&p.nonce), None).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("ChatGPT sign-in refused after the exchange: {e:#}");
                return Err(e);
            }
        };
        if !t.scope.split_whitespace().any(|s| s == PLAN_SCOPE) {
            tracing::warn!(scope = t.scope, "ChatGPT sign-in without plan usage");
            bail!("signed in, but ChatGPT didn't allow plan usage for Moonrise (your plan may not include it)");
        }
        let kept_sub = if kept.sub.is_empty() {
            claims(&kept.id_token)
                .and_then(|c| c["sub"].as_str().map(String::from))
                .unwrap_or_default()
        } else {
            kept.sub.clone()
        };
        if !kept_sub.is_empty() && kept_sub != t.sub {
            // OpenAI: confirm a returning sign-in is the same account before replacing
            // anything. The tokens just issued aren't kept (nor used).
            tracing::warn!("ChatGPT sign-in for another account refused");
            bail!(
                "that is a different ChatGPT account from the one Moonrise is registered with{}; nothing was changed. To use this account, choose Use Another Account",
                if kept.email.is_empty() { String::new() } else { format!(" ({})", kept.email) }
            );
        }
        let mut g = self.tokens.lock().await;
        let a = self.loaded(&mut g).await?;
        let mut next = a.clone();
        // This registration's tokens only: never another registration's.
        t.id = kept.id.clone();
        let slot = next
            .get_mut(&kept.id)
            .context("that ChatGPT account was removed meanwhile; nothing was changed")?;
        if slot.client_id != t.client_id {
            bail!("ChatGPT answered for a different registration; nothing was changed");
        }
        *slot = t.clone();
        next.active = t.id.clone();
        self.remember(&t);
        self.persist(&next).await?;
        *a = next;
        tracing::info!(account = t.email, registration = t.id, "signed in with ChatGPT");
        Ok(t)
    }

    /// An error OpenAI sent back to the app's listener instead of a code (`error`,
    /// `error_description`): logged (scrubbed), and the attempt is over. An unfinished
    /// registration this attempt was retrying is dropped. Answers the message to show.
    pub async fn callback_error(&self, state: &str, error: &str, description: &str) -> String {
        let error: String = error
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.' || *c == '-')
            .take(64)
            .collect();
        let description: String = self.scrub(description).chars().filter(|c| !c.is_control()).take(300).collect();
        let p = self.pending.lock().unwrap().remove(state).filter(|p| p.at.elapsed() < PENDING_TTL);
        tracing::warn!(
            error,
            description,
            attempt = if p.is_some() { "known" } else { "unknown or expired" },
            registration = registration_kind(p.as_ref().and_then(|p| p.returning.as_ref())),
            retry = p.as_ref().is_some_and(|p| p.retry),
            "ChatGPT sign-in: OpenAI answered with an error"
        );
        if let Some(t) = p.as_ref().and_then(|p| p.returning.as_ref()).filter(|t| t.pending()) {
            self.drop_unfinished(&t.id).await;
        }
        let said = if description.is_empty() { error } else { description };
        format!(
            "ChatGPT didn't sign in: {}",
            if said.is_empty() { "no reason given".to_string() } else { said }
        )
    }

    /// Keeps a new registration (the client OpenAI issued, no account yet), or the one already
    /// kept for that client. It is not the one in use until its sign-in completes.
    async fn keep_registration(&self, client_id: &str) -> Result<Tokens> {
        let mut g = self.tokens.lock().await;
        let a = self.loaded(&mut g).await?;
        if let Some(t) = a.accounts.iter().find(|t| t.client_id == client_id) {
            return Ok(t.clone());
        }
        let mut next = a.clone();
        let t = Tokens {
            id: next.new_id(),
            client_id: client_id.to_string(),
            kept_at: now(),
            ..Default::default()
        };
        next.accounts.push(t.clone());
        self.persist(&next).await?;
        *a = next;
        tracing::info!(registration = t.id, client_id, "ChatGPT registration kept (unfinished)");
        Ok(t)
    }

    /// Drops the registration `id` if it is still unfinished; true if it was.
    async fn drop_unfinished(&self, id: &str) -> bool {
        let mut g = self.tokens.lock().await;
        let Ok(a) = self.loaded(&mut g).await else { return false };
        if !a.accounts.iter().any(|t| t.id == id && t.pending()) {
            return false;
        }
        let mut next = a.clone();
        next.accounts.retain(|t| t.id != id);
        if next.active == id {
            next.active.clear();
        }
        if let Err(e) = self.persist(&next).await {
            tracing::warn!("ChatGPT: unfinished registration not dropped: {e:#}");
            return false;
        }
        *a = next;
        self.retries.lock().unwrap().retain(|_, (r, _)| r != id);
        tracing::info!(registration = id, "ChatGPT unfinished registration dropped");
        true
    }

    /// Removes the registration `account` from this server, if it isn't signed in (an
    /// unfinished one, or one signed out or whose sign-in ended): the next Sign In registers
    /// afresh unless another is in use. Nothing is sent to OpenAI (there is no token to revoke);
    /// the registration may still be listed in ChatGPT's settings, where it can be removed.
    pub async fn forget(&self, account: &str) -> Result<()> {
        let mut g = self.tokens.lock().await;
        let a = self.loaded(&mut g).await?;
        let Some(t) = a.accounts.iter().find(|t| t.id == account) else {
            bail!("no such ChatGPT account kept here")
        };
        if !t.refresh_token.is_empty() {
            bail!("that ChatGPT account is signed in: sign out first");
        }
        let unfinished = t.pending();
        let mut next = a.clone();
        next.accounts.retain(|t| t.id != account);
        if next.active == account {
            next.active.clear();
        }
        self.persist(&next).await?;
        *a = next;
        self.retries.lock().unwrap().retain(|_, (r, _)| r != account);
        tracing::info!(registration = account, unfinished, "ChatGPT registration forgotten");
        Ok(())
    }

    /// Signs out of the registration `account` (default: the one in use): revokes its refresh
    /// token at OpenAI, then clears its tokens, keeping the registration (client id, account)
    /// and the host id for the next sign-in. The other registrations are untouched.
    pub async fn sign_out(&self, account: Option<&str>) -> Result<SignedOut> {
        let mut g = self.tokens.lock().await;
        let a = self.loaded(&mut g).await?;
        let id = account.filter(|s| !s.is_empty()).map_or_else(|| a.active.clone(), String::from);
        let Some(t) = a.get_mut(&id) else {
            if account.is_some_and(|s| !s.is_empty()) {
                bail!("no such ChatGPT account kept here");
            }
            return Ok(SignedOut { revoked: true });
        };
        let revoked = if t.refresh_token.is_empty() {
            true
        } else {
            self.revoke(&t.refresh_token, &t.client_id).await
        };
        t.access_token.clear();
        t.refresh_token.clear();
        t.id_token.clear();
        t.expires_at = 0;
        t.signed_out = true;
        let snapshot = a.clone();
        self.persist(&snapshot).await?;
        if revoked {
            tracing::info!("signed out of ChatGPT (revoked)");
        } else {
            tracing::warn!("signed out of ChatGPT; OpenAI didn't confirm the revocation");
        }
        Ok(SignedOut { revoked })
    }

    /// Revokes a refresh token (RFC 7009, as OpenAI documents it): true once OpenAI confirms.
    /// Network errors and 5xx are retried.
    async fn revoke(&self, refresh_token: &str, client_id: &str) -> bool {
        let mut endpoint: Option<String> = None;
        for attempt in 0..3u32 {
            if attempt > 0 {
                tokio::time::sleep(self.backoff * (1 << (attempt - 1))).await;
            }
            if endpoint.is_none() {
                match self.endpoint("revocation_endpoint").await {
                    Ok(e) => endpoint = Some(e),
                    Err(e) => {
                        tracing::warn!("ChatGPT revocation: {e:#}");
                        continue;
                    }
                }
            }
            let form = [
                ("token", refresh_token),
                ("token_type_hint", "refresh_token"),
                ("client_id", client_id),
            ];
            match self.http.post(endpoint.as_deref().unwrap_or_default()).form(&form).send().await {
                Ok(r) if r.status().is_success() => return true,
                Ok(r) if r.status().is_server_error() => {
                    tracing::warn!(status = r.status().as_u16(), "ChatGPT revocation failed, trying again");
                }
                Ok(r) if r.status().is_redirection() => {
                    tracing::warn!(
                        status = r.status().as_u16(),
                        "ChatGPT revocation answered with a redirect; not followed"
                    );
                    return false;
                }
                Ok(r) => {
                    tracing::warn!(status = r.status().as_u16(), "ChatGPT revocation refused");
                    return false;
                }
                Err(e) => tracing::warn!(
                    "ChatGPT revocation unreachable ({})",
                    if e.is_timeout() { "timeout" } else { "connection" }
                ),
            }
        }
        false
    }

    /// An endpoint from OpenAI's OpenID configuration; only one on the sign-in server's own
    /// origin is used.
    async fn endpoint(&self, name: &str) -> Result<String> {
        let doc = {
            let mut g = self.discovery.lock().await;
            match g.as_ref().filter(|(_, at)| at.elapsed() < DISCOVERY_TTL) {
                Some((d, _)) => d.clone(),
                None => {
                    let url = format!("{}/.well-known/openid-configuration", crate::llm::origin(&self.auth_base));
                    let d = self.get_json(&url).await.context("OpenAI's sign-in configuration")?;
                    *g = Some((d.clone(), Instant::now()));
                    d
                }
            }
        };
        let e = doc[name]
            .as_str()
            .filter(|e| !e.is_empty())
            .with_context(|| format!("OpenAI's sign-in configuration has no {name}"))?;
        if crate::llm::origin(e) != crate::llm::origin(&self.auth_base) {
            bail!("OpenAI's sign-in configuration names {name} elsewhere; not used");
        }
        Ok(e.to_string())
    }

    async fn get_json(&self, url: &str) -> Result<Value> {
        let r = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("unreachable ({})", if e.is_timeout() { "timeout" } else { "connection" }))?;
        if !r.status().is_success() {
            bail!("HTTP {}", r.status().as_u16());
        }
        r.json().await.context("not JSON")
    }

    /// OpenAI's signing keys (fetched again when `fresh`, for a key that was rotated in).
    async fn keys(&self, fresh: bool) -> Result<Value> {
        let mut g = self.jwks.lock().await;
        if let Some((k, at)) = g.as_ref() {
            if !fresh && at.elapsed() < DISCOVERY_TTL {
                return Ok(k.clone());
            }
        }
        let url = self.endpoint("jwks_uri").await?;
        let k = self.get_json(&url).await.context("OpenAI's signing keys")?;
        *g = Some((k.clone(), Instant::now()));
        Ok(k)
    }

    /// The claims of an ID token from OpenAI: signature (OpenAI's JWKS), issuer, audience (the
    /// issued client), expiry and, for a sign-in, the nonce.
    async fn validated(&self, id_token: &str, client_id: &str, nonce: Option<&str>) -> Result<Value> {
        let signed = match signed_by(id_token, &self.keys(false).await?) {
            Ok(ok) => ok,
            // A key OpenAI rotated in since the keys were fetched.
            Err(_) => signed_by(id_token, &self.keys(true).await?)?,
        };
        if !signed {
            bail!("ChatGPT sign-in: the ID token's signature isn't OpenAI's");
        }
        let c = claims(id_token).context("ChatGPT sign-in: unreadable ID token")?;
        let aud_ok = match &c["aud"] {
            Value::String(a) => a == client_id,
            Value::Array(a) => a.iter().any(|x| x == client_id),
            _ => false,
        };
        if c["iss"].as_str().map(|i| i.trim_end_matches('/')) != Some(ISSUER) || !aud_ok {
            bail!("ChatGPT sign-in: the ID token isn't for this server");
        }
        if c["exp"].as_i64().unwrap_or(0) < now() - 300 {
            bail!("ChatGPT sign-in: the ID token has expired");
        }
        if let Some(n) = nonce {
            if c["nonce"].as_str() != Some(n) {
                bail!("ChatGPT sign-in: the ID token doesn't match this sign-in");
            }
        }
        if c["sub"].as_str().is_none_or(str::is_empty) {
            bail!("ChatGPT sign-in: the ID token names no account");
        }
        Ok(c)
    }

    /// The next call refreshes the access token first.
    pub async fn expire_access(&self) {
        if let Some(t) = self.tokens.lock().await.as_mut().and_then(Accounts::active_mut) {
            t.expires_at = 0;
        }
    }

    /// A current access token, refreshed when it is close to expiry. Only for [`API_BASE`].
    pub async fn access_token(&self) -> Result<String> {
        let mut g = self.tokens.lock().await;
        let a = self.loaded(&mut g).await?;
        let mut all = a.clone();
        let Some(t) = all.active_mut() else {
            bail!("not signed in with ChatGPT: sign in in Manage Server → Services → AI Matching")
        };
        if t.refresh_token.is_empty() {
            bail!("the ChatGPT sign-in ended: sign in again in Manage Server → Services → AI Matching");
        }
        if !t.access_token.is_empty() && t.expires_at > now() + 120 {
            return Ok(t.access_token.clone());
        }
        let form = [
            ("grant_type", "refresh_token"),
            ("client_id", t.client_id.as_str()),
            ("refresh_token", t.refresh_token.as_str()),
            ("resource", RESOURCE),
        ];
        match self.token_request(&form).await {
            Ok(v) => {
                let client_id = t.client_id.clone();
                let mut fresh = self.tokens_from(&v, &client_id, None, Some(t)).await?;
                fresh.id = t.id.clone();
                self.remember(&fresh);
                let token = fresh.access_token.clone();
                *t = fresh;
                self.persist(&all).await?;
                *a = all;
                Ok(token)
            }
            Err(e) if e.downcast_ref::<TokenError>().is_some_and(TokenError::terminal) => {
                // The refresh token is no good: keep the account (to sign in again), drop the rest.
                t.access_token.clear();
                t.refresh_token.clear();
                self.persist(&all).await?;
                *a = all;
                tracing::warn!("ChatGPT refresh refused: {e:#}");
                bail!("the ChatGPT sign-in ended: sign in again in Manage Server → Services → AI Matching")
            }
            Err(e) => Err(e),
        }
    }

    async fn token_request(&self, form: &[(&str, &str)]) -> Result<Value> {
        let grant = form.iter().find(|(k, _)| *k == "grant_type").map_or("", |(_, v)| *v).to_string();
        let started = Instant::now();
        let resp = self
            .http
            .post(format!("{}/oauth/token", self.auth_base))
            .header(reqwest::header::ACCEPT, "application/json")
            .form(form)
            .send()
            .await
            .map_err(|e| {
                let why = if e.is_timeout() { "timeout" } else { "connection" };
                tracing::warn!(grant, why, "ChatGPT token request: sign-in server unreachable");
                anyhow::anyhow!("ChatGPT sign-in server unreachable ({why})")
            })?;
        let status = resp.status().as_u16();
        let request_id = resp
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .chars()
            .take(80)
            .collect::<String>();
        let ms = started.elapsed().as_millis() as u64;
        if resp.status().is_redirection() {
            tracing::warn!(
                grant,
                status,
                request_id,
                "ChatGPT token request answered with a redirect; not followed"
            );
            bail!("ChatGPT sign-in: the sign-in server answered with a redirect (HTTP {status}), which Moonrise doesn't follow");
        }
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if status != 200 {
            let text = |k: &str| self.scrub(v[k].as_str().unwrap_or("")).chars().take(300).collect::<String>();
            let e = TokenError {
                status,
                code: text("error"),
                reason: text("error_reason"),
                description: text("error_description"),
                request_id,
            };
            tracing::warn!(
                grant,
                status,
                error = e.code,
                reason = e.reason,
                description = e.description,
                request_id = e.request_id,
                ms,
                "ChatGPT token request refused"
            );
            return Err(e.into());
        }
        tracing::info!(grant, status, request_id, ms, "ChatGPT token request");
        Ok(v)
    }

    /// Tokens from a token-endpoint answer (`nonce`: a sign-in, whose ID token must validate;
    /// `old`: a refresh, whose account and ID token carry over when the answer has none).
    async fn tokens_from(&self, v: &Value, client_id: &str, nonce: Option<&str>, old: Option<&Tokens>) -> Result<Tokens> {
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        let (access, refresh) = (s("access_token"), s("refresh_token"));
        if access.is_empty() {
            bail!("ChatGPT sign-in: no access token in the answer");
        }
        let mut t = Tokens {
            id: old.map(|o| o.id.clone()).unwrap_or_default(),
            client_id: client_id.to_string(),
            access_token: access,
            refresh_token: if refresh.is_empty() {
                old.map(|o| o.refresh_token.clone()).unwrap_or_default()
            } else {
                refresh
            },
            id_token: s("id_token"),
            expires_at: now() + v["expires_in"].as_i64().unwrap_or(3600),
            email: old.map(|o| o.email.clone()).unwrap_or_default(),
            sub: old.map(|o| o.sub.clone()).unwrap_or_default(),
            scope: s("scope"),
            signed_out: false,
            kept_at: 0,
        };
        if t.scope.is_empty() {
            t.scope = old.map(|o| o.scope.clone()).unwrap_or_default();
        }
        if nonce.is_some() && t.id_token.is_empty() {
            bail!("ChatGPT sign-in: no ID token in the answer");
        }
        if t.id_token.is_empty() {
            t.id_token = old.map(|o| o.id_token.clone()).unwrap_or_default();
            return Ok(t);
        }
        let c = match self.validated(&t.id_token, client_id, nonce).await {
            Ok(c) => c,
            Err(e) if nonce.is_some() => return Err(e),
            Err(e) => {
                // A refresh: the tokens are OpenAI's (its token endpoint, over TLS) and the old
                // refresh token is spent, so they are kept; the ID token that didn't validate
                // isn't.
                tracing::warn!("ChatGPT refresh: new ID token not used: {e:#}");
                t.id_token = old.map(|o| o.id_token.clone()).unwrap_or_default();
                return Ok(t);
            }
        };
        let sub = c["sub"].as_str().unwrap_or("").to_string();
        if let Some(o) = old {
            let old_sub = if o.sub.is_empty() {
                claims(&o.id_token)
                    .and_then(|oc| oc["sub"].as_str().map(String::from))
                    .unwrap_or_default()
            } else {
                o.sub.clone()
            };
            if !old_sub.is_empty() && old_sub != sub {
                bail!("ChatGPT answered for a different account");
            }
        }
        t.sub = sub;
        if let Some(e) = c["email"].as_str() {
            t.email = e.to_string();
        }
        Ok(t)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::{
        extract::State,
        routing::{get, post},
        Form, Json, Router,
    };
    use std::sync::Arc;

    /// A signing key made once per test run, as OpenAI's JWKS would publish it.
    pub(crate) struct Signer {
        key: rsa::pkcs1v15::SigningKey<sha2::Sha256>,
        pub jwks: Value,
    }

    pub(crate) fn openai_key() -> &'static Signer {
        static KEY: OnceLock<Signer> = OnceLock::new();
        KEY.get_or_init(|| signer("key-1"))
    }

    fn signer(kid: &str) -> Signer {
        use rsa::traits::PublicKeyParts;
        let k = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
        let jwks = json!({"keys": [{"kty": "RSA", "use": "sig", "alg": "RS256", "kid": kid, "n": b64(&k.n().to_bytes_be()), "e": b64(&k.e().to_bytes_be())}]});
        Signer {
            key: rsa::pkcs1v15::SigningKey::new(k),
            jwks,
        }
    }

    impl Signer {
        pub(crate) fn jwt(&self, claims: Value) -> String {
            use rsa::signature::{SignatureEncoding, Signer as _};
            let kid = self.jwks["keys"][0]["kid"].as_str().unwrap();
            let input = format!(
                "{}.{}",
                b64(json!({"alg": "RS256", "kid": kid}).to_string().as_bytes()),
                b64(claims.to_string().as_bytes())
            );
            let sig = self.key.sign(input.as_bytes()).to_bytes();
            format!("{input}.{}", b64(&sig))
        }
    }

    /// A stand-in for auth.openai.com: OpenID configuration, JWKS, token and revocation
    /// endpoints. What it answers is set through [`Mock`].
    #[derive(Default)]
    pub(crate) struct Mock {
        /// The nonce and account of the next ID token.
        pub nonce: Mutex<String>,
        pub sub: Mutex<String>,
        /// Sign the next ID token with another key (a forgery).
        pub forged: Mutex<bool>,
        pub refreshes: Mutex<u32>,
        /// Statuses the revocation endpoint answers, in turn (then 200).
        pub revoke_statuses: Mutex<Vec<u16>>,
        pub revocations: Mutex<Vec<HashMap<String, String>>>,
        /// The configuration names the revocation endpoint on another origin.
        pub revoke_elsewhere: Mutex<bool>,
        pub addr: Mutex<String>,
        /// The code the next good exchange expects (empty: `the-code`).
        pub code: Mutex<String>,
        /// Every form the token endpoint received.
        pub token_requests: Mutex<Vec<HashMap<String, String>>>,
    }

    async fn openid(State(m): State<Arc<Mock>>) -> Json<Value> {
        let a = m.addr.lock().unwrap().clone();
        let revoke = if *m.revoke_elsewhere.lock().unwrap() {
            "http://127.0.0.1:9/revoke".to_string()
        } else {
            format!("{a}/api/accounts/oauth/revoke")
        };
        Json(json!({"issuer": ISSUER, "revocation_endpoint": revoke, "jwks_uri": format!("{a}/.well-known/jwks.json")}))
    }

    async fn token(
        State(m): State<Arc<Mock>>,
        headers: axum::http::HeaderMap,
        Form(f): Form<HashMap<String, String>>,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        m.token_requests.lock().unwrap().push(f.clone());
        assert_eq!(headers.get("accept").and_then(|v| v.to_str().ok()), Some("application/json"));
        assert!(headers
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|u| u.starts_with("Moonrise/")));
        if f.get("code").is_some_and(|c| c == "bad-code") {
            // What OpenAI answers for a code it won't take.
            let body = json!({"error": "invalid_grant", "error_reason": "code_invalid", "error_description": "The authorization code is invalid or has expired."});
            return (axum::http::StatusCode::BAD_REQUEST, [("x-request-id", "req-123")], Json(body)).into_response();
        }
        if f.get("client_id").is_some_and(|c| c == "gone-client") {
            // A client OpenAI doesn't know (never issued, or removed in ChatGPT's settings).
            return (axum::http::StatusCode::UNAUTHORIZED, Json(json!({"error": "invalid_client"}))).into_response();
        }
        token_answer(&m, f).into_response()
    }

    fn token_answer(m: &Mock, f: HashMap<String, String>) -> (axum::http::StatusCode, Json<Value>) {
        assert_eq!(f["resource"], RESOURCE);
        let client = f["client_id"].clone();
        assert!(client == "issued-1" || client == "issued-2", "{client}");
        match f["grant_type"].as_str() {
            "authorization_code" if client == "issued-2" => {
                let claims = json!({"iss": ISSUER, "aud": "issued-2", "exp": now() + 3600, "nonce": *m.nonce.lock().unwrap(), "sub": *m.sub.lock().unwrap(), "email": format!("{}@example.com", m.sub.lock().unwrap())});
                (
                    axum::http::StatusCode::OK,
                    Json(
                        json!({"access_token": "access-2nd-xxxxxxxx", "refresh_token": "refresh-2nd-xxxxxxx", "id_token": openai_key().jwt(claims), "expires_in": 3600, "scope": SCOPES}),
                    ),
                )
            }
            "authorization_code" => {
                let expected = m.code.lock().unwrap().clone();
                assert_eq!(
                    f["code"],
                    if expected.is_empty() { "the-code" } else { expected.as_str() },
                    "the code reaches OpenAI unchanged"
                );
                assert_eq!(f["redirect_uri"], "http://127.0.0.1:1455/auth/callback");
                assert!(f["code_verifier"].len() >= 43);
                let claims = json!({"iss": ISSUER, "aud": "issued-1", "exp": now() + 3600, "nonce": *m.nonce.lock().unwrap(), "sub": *m.sub.lock().unwrap(), "email": format!("{}@example.com", m.sub.lock().unwrap())});
                let id = if *m.forged.lock().unwrap() {
                    signer("key-1").jwt(claims)
                } else {
                    openai_key().jwt(claims)
                };
                (
                    axum::http::StatusCode::OK,
                    Json(
                        json!({"access_token": "access-one-xxxxxxxx", "refresh_token": "refresh-one-xxxxxxx", "id_token": id, "expires_in": 3600, "scope": SCOPES}),
                    ),
                )
            }
            _ => {
                let mut n = m.refreshes.lock().unwrap();
                *n += 1;
                if *n == 1 {
                    assert_eq!(f["refresh_token"], "refresh-one-xxxxxxx");
                    (
                        axum::http::StatusCode::OK,
                        Json(json!({"access_token": "access-two-xxxxxxxx", "refresh_token": "refresh-two-xxxxxxx", "expires_in": 3600})),
                    )
                } else {
                    (axum::http::StatusCode::BAD_REQUEST, Json(json!({"error": "refresh_token_reused"})))
                }
            }
        }
    }

    async fn revoke(State(m): State<Arc<Mock>>, Form(f): Form<HashMap<String, String>>) -> axum::http::StatusCode {
        m.revocations.lock().unwrap().push(f);
        let mut s = m.revoke_statuses.lock().unwrap();
        let code = if s.is_empty() { 200 } else { s.remove(0) };
        axum::http::StatusCode::from_u16(code).unwrap()
    }

    /// The stand-in, and a [`ChatGpt`] that uses it.
    pub(crate) async fn mock() -> (Arc<Mock>, ChatGpt) {
        let m = Arc::new(Mock::default());
        *m.sub.lock().unwrap() = "u1".into();
        let app = Router::new()
            .route("/.well-known/openid-configuration", get(openid))
            .route("/.well-known/jwks.json", get(|| async { Json(openai_key().jwks.clone()) }))
            .route("/api/accounts/oauth/token", post(token))
            .route("/api/accounts/oauth/revoke", post(revoke))
            .with_state(m.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        *m.addr.lock().unwrap() = format!("http://{addr}");
        let mut g = ChatGpt::new(&format!("http://{addr}/api/accounts"));
        g.backoff = Duration::from_millis(10);
        (m, g)
    }

    const CALLBACK: &str = "http://127.0.0.1:1455/auth/callback";

    /// Starts a sign-in: the authorization URL's parameters (the mock's next nonce is set).
    async fn start(m: &Mock, g: &ChatGpt, new_account: bool) -> HashMap<String, String> {
        start_for(m, g, new_account, None).await
    }

    async fn start_for(m: &Mock, g: &ChatGpt, new_account: bool, account: Option<&str>) -> HashMap<String, String> {
        start_with(m, g, new_account, account, None).await
    }

    /// The one retry after a refused code: `failed` is that attempt's state.
    async fn start_retry(m: &Mock, g: &ChatGpt, failed: &str) -> HashMap<String, String> {
        start_with(m, g, false, None, Some(failed)).await
    }

    async fn start_with(m: &Mock, g: &ChatGpt, new_account: bool, account: Option<&str>, retry: Option<&str>) -> HashMap<String, String> {
        let url = reqwest::Url::parse(&g.start(CALLBACK, new_account, account, retry).await.unwrap()).unwrap();
        assert_eq!(url.path(), "/api/accounts/authorize");
        let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
        *m.nonce.lock().unwrap() = q["nonce"].clone();
        q
    }

    #[test]
    fn redirects_and_errors() {
        assert!(valid_redirect("http://127.0.0.1:1455/auth/callback"));
        assert!(!valid_redirect("http://localhost:1455/auth/callback"));
        assert!(!valid_redirect("http://127.0.0.1:1455/callback"));
        assert!(!valid_redirect("https://127.0.0.1:1455/auth/callback"));
        assert!(format!(
            "{:#}",
            plan_error(429, r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded"}}"#).unwrap()
        )
        .contains("limit"));
        assert!(plan_error(503, r#"{"error":{"code":"subscription_sharing_usage_unavailable"}}"#).is_none());
        assert!(plan_error(403, "{}").is_some());
        assert!(signed_in(Some(&json!({"refresh_token": "r"}))));
        assert!(!signed_in(Some(&json!({"refresh_token": ""}))));
        assert!(format!(
            "{:?}",
            Tokens {
                access_token: "secret-token-value".into(),
                ..Default::default()
            }
        )
        .find("secret")
        .is_none());
    }

    #[test]
    fn id_token_signatures() {
        let k = openai_key();
        let jwt = k.jwt(json!({"sub": "u1"}));
        assert!(signed_by(&jwt, &k.jwks).unwrap());
        // Another key with the same kid: a forgery.
        assert!(!signed_by(&signer("key-1").jwt(json!({"sub": "u1"})), &k.jwks).unwrap());
        // Claims changed after signing.
        let mut parts: Vec<&str> = jwt.split('.').collect();
        let other = b64(json!({"sub": "u2"}).to_string().as_bytes());
        parts[1] = &other;
        assert!(!signed_by(&parts.join("."), &k.jwks).unwrap());
        // Unsigned ("alg": "none"), or a key OpenAI doesn't publish.
        let none = format!("{}.{}.", b64(br#"{"alg":"none","kid":"key-1"}"#), b64(br#"{"sub":"u1"}"#));
        assert!(!signed_by(&none, &k.jwks).unwrap());
        assert!(signed_by(&signer("key-2").jwt(json!({})), &k.jwks).is_err());
    }

    /// The whole flow against a stand-in for OpenAI: register, exchange (the ID token's
    /// signature checked), refresh (rotating), a refused refresh, signing in again.
    #[tokio::test]
    async fn sign_in_and_refresh() {
        let (m, g) = mock().await;
        assert!(g.start("http://localhost:1/auth/callback", false, None, None).await.is_err());
        let q = start(&m, &g, false).await;
        assert_eq!(q["client_id"], "dynamic_agent_client");
        assert_eq!(q["agent_name_hint"], "Moonrise");
        assert!(
            !q.contains_key("ext_agent_host_id"),
            "not sent unless turned on (as OpenAI's reference client)"
        );
        assert_eq!((q["scope"].as_str(), q["code_challenge_method"].as_str()), (SCOPES, "S256"));
        assert!(!q.contains_key("id_token_hint"));
        assert!(g.finish("wrong-state", "the-code", Some("issued-1")).await.is_err());
        // An ID token not signed by OpenAI's key: refused, nothing kept.
        *m.forged.lock().unwrap() = true;
        let e = g.finish(&q["state"], "the-code", Some("issued-1")).await.unwrap_err();
        assert!(format!("{e:#}").contains("signature"), "{e:#}");
        assert_eq!(g.status().await, json!({"signed_in": false, "accounts": [], "unfinished": []}));
        *m.forged.lock().unwrap() = false;
        let q = start(&m, &g, false).await;
        let t = g.finish(&q["state"], "the-code", Some("issued-1")).await.unwrap();
        assert_eq!((t.email.as_str(), t.sub.as_str()), ("u1@example.com", "u1"));
        assert!(
            g.finish(&q["state"], "the-code", Some("issued-1")).await.is_err(),
            "a state is used once"
        );
        assert_eq!(g.status().await["email"], "u1@example.com");
        assert_eq!(g.access_token().await.unwrap(), "access-one-xxxxxxxx");
        assert_eq!(g.scrub("Bearer access-one-xxxxxxxx"), "Bearer ***");
        // Close to expiry: refreshed, the refresh token rotated, the account kept.
        g.expire_access().await;
        assert_eq!(g.access_token().await.unwrap(), "access-two-xxxxxxxx");
        let kept = g.current().await.unwrap().unwrap();
        assert_eq!(
            (kept.refresh_token.as_str(), kept.email.as_str(), kept.sub.as_str()),
            ("refresh-two-xxxxxxx", "u1@example.com", "u1")
        );
        // Refused for good: signed out, the account kept to sign in again with the same client.
        g.expire_access().await;
        assert!(format!("{:#}", g.access_token().await.unwrap_err()).contains("sign in again"));
        assert_eq!(g.status().await["needs_sign_in"], true);
        let q = start(&m, &g, false).await;
        assert_eq!((q["client_id"].as_str(), q["login_hint"].as_str()), ("issued-1", "u1@example.com"));
        assert_eq!(q["id_token_hint"], kept.id_token, "the kept ID token names the account");
        assert!(!q.contains_key("agent_name_hint"));
        // Another account answers: refused before anything is replaced.
        *m.sub.lock().unwrap() = "u2".into();
        let e = g.finish(&q["state"], "the-code", None).await.unwrap_err();
        assert!(format!("{e:#}").contains("different ChatGPT account"), "{e:#}");
        let after = g.current().await.unwrap().unwrap();
        assert_eq!(
            (after.sub.as_str(), after.email.as_str(), after.refresh_token.as_str()),
            ("u1", "u1@example.com", "")
        );
        // Another client id in the answer: refused too.
        *m.sub.lock().unwrap() = "u1".into();
        let q = start(&m, &g, false).await;
        assert!(format!("{:#}", g.finish(&q["state"], "the-code", Some("issued-9")).await.unwrap_err()).contains("different registration"));
        // The same account: signed in again.
        let q = start(&m, &g, false).await;
        let t = g.finish(&q["state"], "the-code", None).await.unwrap();
        assert_eq!((t.client_id.as_str(), t.sub.as_str()), ("issued-1", "u1"));
        assert_eq!(g.status().await["signed_in"], true);
    }

    /// Sign-out revokes the refresh token at the configuration's revocation endpoint (retried
    /// on 5xx), then clears the tokens only: the registration and the host id stay.
    #[tokio::test]
    async fn sign_out_revokes_and_keeps_the_registration() {
        let (m, mut g) = mock().await;
        g.send_host_id = true;
        let q = start(&m, &g, false).await;
        let host = q["ext_agent_host_id"].clone();
        assert!(host.starts_with("urn:uuid:"));
        g.finish(&q["state"], "the-code", Some("issued-1")).await.unwrap();
        *m.revoke_statuses.lock().unwrap() = vec![503];
        assert_eq!(g.sign_out(None).await.unwrap(), SignedOut { revoked: true });
        let r = m.revocations.lock().unwrap().clone();
        assert_eq!(r.len(), 2, "retried after the 503");
        assert_eq!(
            (r[1]["token"].as_str(), r[1]["token_type_hint"].as_str(), r[1]["client_id"].as_str()),
            ("refresh-one-xxxxxxx", "refresh_token", "issued-1")
        );
        let t = g.current().await.unwrap().unwrap();
        assert!(t.access_token.is_empty() && t.refresh_token.is_empty() && t.id_token.is_empty());
        assert_eq!(
            (t.client_id.as_str(), t.sub.as_str(), t.email.as_str()),
            ("issued-1", "u1", "u1@example.com")
        );
        assert_eq!(
            g.status().await,
            json!({"signed_in": false, "email": "u1@example.com", "account": "1", "accounts": [{"id": "1", "label": "u1@example.com", "email": "u1@example.com", "signed_in": false, "active": true}], "unfinished": []})
        );
        assert!(g.access_token().await.is_err());
        // Signing in again: the same registration and host, no ID token hint after a sign-out.
        let q = start(&m, &g, false).await;
        assert_eq!(
            (q["client_id"].as_str(), q["ext_agent_host_id"].as_str()),
            ("issued-1", host.as_str())
        );
        assert!(!q.contains_key("id_token_hint") && !q.contains_key("agent_name_hint"));
        // Or another account: a new registration on the same host.
        let q = start(&m, &g, true).await;
        assert_eq!(
            (q["client_id"].as_str(), q["ext_agent_host_id"].as_str()),
            ("dynamic_agent_client", host.as_str())
        );
        *m.sub.lock().unwrap() = "u2".into();
        let t = g.finish(&q["state"], "the-code", Some("issued-2")).await.unwrap();
        assert_eq!((t.sub.as_str(), t.id.as_str()), ("u2", "2"));
    }

    /// OpenAI's "profiles and sessions": each account registration keeps its own client id and
    /// credentials; another account is a new registration that never overwrites one; switching
    /// back signs in again with that registration's client id (no new client); signing out of
    /// one leaves the others alone.
    #[tokio::test]
    async fn registrations_stay_separate() {
        let (m, g) = mock().await;
        let q = start(&m, &g, false).await;
        g.finish(&q["state"], "the-code", Some("issued-1")).await.unwrap();
        // Another account while signed in: allowed, a new registration.
        let q = start(&m, &g, true).await;
        assert_eq!(q["client_id"], "dynamic_agent_client");
        *m.sub.lock().unwrap() = "u2".into();
        g.finish(&q["state"], "the-code", Some("issued-2")).await.unwrap();
        let a = g.accounts().await.unwrap();
        assert_eq!(a.active, "2");
        let one = a.accounts.iter().find(|t| t.id == "1").unwrap();
        assert_eq!(
            (one.client_id.as_str(), one.sub.as_str(), one.refresh_token.as_str()),
            ("issued-1", "u1", "refresh-one-xxxxxxx"),
            "the first registration untouched"
        );
        assert_eq!(g.access_token().await.unwrap(), "access-2nd-xxxxxxxx", "the account in use");
        let st = g.status().await;
        assert_eq!((st["account"].as_str(), st["accounts"].as_array().unwrap().len()), (Some("2"), 2));
        // Back to the first: its own client id, its account; no new client.
        *m.sub.lock().unwrap() = "u1".into();
        let q = start_for(&m, &g, false, Some("1")).await;
        assert_eq!((q["client_id"].as_str(), q["login_hint"].as_str()), ("issued-1", "u1@example.com"));
        assert!(!q.contains_key("agent_name_hint"));
        // A callback naming the other registration's client: refused.
        assert!(g.finish(&q["state"], "the-code", Some("issued-2")).await.is_err());
        let q = start_for(&m, &g, false, Some("1")).await;
        g.finish(&q["state"], "the-code", None).await.unwrap();
        let a = g.accounts().await.unwrap();
        assert_eq!((a.active.as_str(), a.accounts.len()), ("1", 2));
        assert_eq!(
            a.accounts.iter().find(|t| t.id == "2").unwrap().refresh_token,
            "refresh-2nd-xxxxxxx",
            "the second untouched"
        );
        // The second registration's account must answer for it.
        let q = start_for(&m, &g, false, Some("2")).await;
        assert_eq!(q["client_id"], "issued-2");
        let e = g.finish(&q["state"], "the-code", None).await.unwrap_err();
        assert!(format!("{e:#}").contains("different ChatGPT account"), "{e:#}");
        assert!(g.start(CALLBACK, false, Some("9"), None).await.is_err(), "unknown registration");
        // Signing out of the second: revoked with its own client id; the first stays signed in.
        assert_eq!(g.sign_out(Some("2")).await.unwrap(), SignedOut { revoked: true });
        let r = m.revocations.lock().unwrap().clone();
        assert_eq!(
            (r[0]["token"].as_str(), r[0]["client_id"].as_str()),
            ("refresh-2nd-xxxxxxx", "issued-2")
        );
        assert_eq!(g.status().await["signed_in"], true);
        assert!(g.sign_out(Some("9")).await.is_err());
        // The same email twice is told apart.
        let mut a = g.accounts().await.unwrap();
        a.accounts[1].email = a.accounts[0].email.clone();
        assert_eq!(
            (a.label(&a.accounts[0]), a.label(&a.accounts[1])),
            ("u1@example.com (1)".to_string(), "u1@example.com (2)".to_string())
        );
    }

    /// One registration as stored before there could be several.
    #[test]
    fn earlier_record() {
        let v = json!({"client_id": "c", "refresh_token": "r", "email": "e@x"});
        let a = Accounts::from_stored(Some(&v));
        assert_eq!(
            (a.active.as_str(), a.accounts.len(), a.accounts[0].client_id.as_str()),
            ("1", 1, "c")
        );
        assert!(signed_in(Some(&v)));
        assert!(!signed_in(Some(
            &json!({"active": "2", "accounts": [{"id": "1", "client_id": "c", "refresh_token": "r"}, {"id": "2", "client_id": "d"}]})
        )));
    }

    /// A sign-in server that answers with a redirect to another address: not followed, so the
    /// code, verifier and refresh token never reach it.
    #[tokio::test]
    async fn token_and_revocation_redirects_are_not_followed() {
        let (catcher, caught) = crate::llm::tests::recorder(None).await;
        let to = catcher.clone();
        let app = axum::Router::new().fallback(move |uri: axum::http::Uri| {
            let to = to.clone();
            async move {
                (
                    axum::http::StatusCode::TEMPORARY_REDIRECT,
                    [(axum::http::header::LOCATION, format!("{to}{}", uri.path()))],
                )
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let g = ChatGpt::new(&format!("http://{addr}/api/accounts"));
        let e = g
            .token_request(&[("grant_type", "refresh_token"), ("refresh_token", "refresh-secret-xxxxxxx")])
            .await
            .unwrap_err();
        assert!(format!("{e:#}").contains("redirect"), "{e:#}");
        assert!(!g.revoke("refresh-secret-xxxxxxx", "issued-1").await);
        assert!(g.keys(true).await.is_err());
        assert!(caught.lock().unwrap().is_empty(), "{:?}", caught.lock().unwrap());
    }

    /// A revocation OpenAI doesn't confirm is reported; the tokens are cleared all the same.
    #[tokio::test]
    async fn unconfirmed_revocation() {
        for (statuses, elsewhere) in [(vec![500, 502, 503], false), (vec![400], false), (vec![], true)] {
            let (m, g) = mock().await;
            // (The configuration is read once, at the sign-in.)
            *m.revoke_elsewhere.lock().unwrap() = elsewhere;
            let q = start(&m, &g, false).await;
            g.finish(&q["state"], "the-code", Some("issued-1")).await.unwrap();
            *m.revoke_statuses.lock().unwrap() = statuses;
            assert_eq!(g.sign_out(None).await.unwrap(), SignedOut { revoked: false });
            if elsewhere {
                assert!(m.revocations.lock().unwrap().is_empty(), "the token isn't sent to another origin");
            }
            let t = g.current().await.unwrap().unwrap();
            assert!(t.refresh_token.is_empty() && t.client_id == "issued-1");
        }
    }
    /// OpenAI refuses the code at the exchange (`invalid_grant`): the issued registration is kept
    /// (saved before the exchange), the answer says what OpenAI said and asks to start again, and
    /// starting again uses that registration instead of registering Moonrise once more.
    #[tokio::test]
    async fn refused_code_keeps_the_registration_for_one_retry() {
        let (m, g) = mock().await;
        let q = start(&m, &g, false).await;
        assert_eq!(q["client_id"], "dynamic_agent_client");
        let e = g.finish(&q["state"], "bad-code", Some("issued-1")).await.unwrap_err();
        let refused = e.downcast_ref::<CodeRefused>().expect("a refused code");
        assert_eq!(
            (refused.0.code.as_str(), refused.0.reason.as_str(), refused.0.request_id.as_str()),
            ("invalid_grant", "code_invalid", "req-123")
        );
        let text = format!("{e:#}");
        assert!(
            text.contains("invalid_grant: The authorization code is invalid or has expired.; request req-123"),
            "{text}"
        );
        assert!(!text.contains("terminal") && !text.contains("bad-code"), "{text}");
        // Kept, but not an account yet, and not the one in use.
        let a = g.accounts().await.unwrap();
        assert_eq!(
            (a.active.as_str(), a.accounts.len(), a.accounts[0].client_id.as_str()),
            ("", 1, "issued-1")
        );
        assert!(a.accounts[0].pending() && a.accounts[0].kept_at > 0);
        let st = g.status().await;
        assert_eq!(
            (st["signed_in"].as_bool(), st["accounts"].as_array().unwrap().len()),
            (Some(false), 0)
        );
        assert_eq!(st["unfinished"][0]["id"], "1");
        assert!(format!("{:#}", g.access_token().await.unwrap_err()).starts_with("not signed in with ChatGPT"));
        // Sign In and Use Another Account register afresh; an unfinished registration can't be
        // named either.
        for new_account in [false, true] {
            let q = start(&m, &g, new_account).await;
            assert_eq!(q["client_id"], "dynamic_agent_client");
            assert_eq!(q["agent_name_hint"], "Moonrise");
        }
        assert!(g.start(CALLBACK, false, Some("1"), None).await.is_err());
        // The retry, with the failed attempt's state: the kept registration, no hint to register.
        let r = start_retry(&m, &g, &q["state"]).await;
        assert_eq!(r["client_id"], "issued-1");
        assert!(!r.contains_key("agent_name_hint") && !r.contains_key("id_token_hint") && !r.contains_key("login_hint"));
        // Once only.
        assert!(format!("{:#}", g.start(CALLBACK, false, None, Some(&q["state"])).await.unwrap_err()).contains("start over"));
        // The callback may leave the client id out when signing in again.
        let t = g.finish(&r["state"], "the-code", None).await.unwrap();
        assert_eq!((t.id.as_str(), t.client_id.as_str(), t.sub.as_str()), ("1", "issued-1", "u1"));
        let a = g.accounts().await.unwrap();
        assert_eq!((a.accounts.len(), a.active.as_str()), (1, "1"));
        let st = g.status().await;
        assert_eq!(
            (st["signed_in"].as_bool(), st["unfinished"].as_array().unwrap().len()),
            (Some(true), 0)
        );
        // One exchange per callback: the refused code once, the good one once.
        let codes: Vec<String> = m.token_requests.lock().unwrap().iter().map(|f| f["code"].clone()).collect();
        assert_eq!(codes, ["bad-code", "the-code"]);
    }

    /// A second refusal on the retry is final: the unfinished registration is dropped, no
    /// further retry is offered, and the next Sign In registers afresh.
    #[tokio::test]
    async fn refused_twice_drops_the_unfinished_registration() {
        let (m, g) = mock().await;
        let q = start(&m, &g, false).await;
        assert!(g
            .finish(&q["state"], "bad-code", Some("issued-1"))
            .await
            .unwrap_err()
            .downcast_ref::<CodeRefused>()
            .is_some());
        let r = start_retry(&m, &g, &q["state"]).await;
        let e = g.finish(&r["state"], "bad-code", None).await.unwrap_err();
        assert!(e.downcast_ref::<CodeRefused>().is_none());
        assert!(e.downcast_ref::<TokenError>().is_some_and(|t| t.code == "invalid_grant"));
        assert!(format!("{e:#}").contains("Sign In starts afresh"), "{e:#}");
        assert!(g.accounts().await.unwrap().accounts.is_empty());
        assert!(g.start(CALLBACK, false, None, Some(&r["state"])).await.is_err());
        assert_eq!(start(&m, &g, false).await["client_id"], "dynamic_agent_client");
    }

    /// A client OpenAI doesn't know (`invalid_client`: never issued, or removed in ChatGPT's
    /// settings) is dropped at once, without a retry.
    #[tokio::test]
    async fn unknown_client_is_not_kept() {
        let (m, g) = mock().await;
        let q = start(&m, &g, false).await;
        let e = g.finish(&q["state"], "the-code", Some("gone-client")).await.unwrap_err();
        assert!(e.downcast_ref::<CodeRefused>().is_none());
        assert!(format!("{e:#}").contains("invalid_client"), "{e:#}");
        assert!(g.accounts().await.unwrap().accounts.is_empty());
        assert_eq!(g.status().await["unfinished"], json!([]));
        for new_account in [false, true] {
            assert_eq!(start(&m, &g, new_account).await["client_id"], "dynamic_agent_client");
        }
    }

    /// An error OpenAI sends back to the app on the retry (access_denied, …) ends it and drops
    /// the unfinished registration; on a first attempt nothing is kept anyway.
    #[tokio::test]
    async fn error_back_to_the_app() {
        let (m, g) = mock().await;
        let q = start(&m, &g, false).await;
        assert_eq!(
            g.callback_error(&q["state"], "access_denied", "The user said no.").await,
            "ChatGPT didn't sign in: The user said no."
        );
        // The attempt is over.
        assert!(g.finish(&q["state"], "the-code", Some("issued-1")).await.is_err());
        let q = start(&m, &g, false).await;
        g.finish(&q["state"], "bad-code", Some("issued-1")).await.unwrap_err();
        let r = start_retry(&m, &g, &q["state"]).await;
        assert_eq!(
            g.callback_error(&r["state"], "invalid_scope<script>", "").await,
            "ChatGPT didn't sign in: invalid_scopescript"
        );
        assert!(g.accounts().await.unwrap().accounts.is_empty());
    }

    /// Forget: an unfinished registration, or one that isn't signed in; never one signed in.
    #[tokio::test]
    async fn forget_registrations() {
        let (m, g) = mock().await;
        let q = start(&m, &g, false).await;
        g.finish(&q["state"], "bad-code", Some("issued-2")).await.unwrap_err();
        assert_eq!(g.status().await["unfinished"][0]["id"], "1");
        g.forget("1").await.unwrap();
        assert_eq!(g.status().await["unfinished"], json!([]));
        // Its retry is gone with it.
        assert!(g.start(CALLBACK, false, None, Some(&q["state"])).await.is_err());
        let q = start(&m, &g, false).await;
        g.finish(&q["state"], "the-code", Some("issued-1")).await.unwrap();
        assert!(format!("{:#}", g.forget("1").await.unwrap_err()).contains("sign out first"));
        assert!(g.forget("9").await.is_err());
        g.sign_out(None).await.unwrap();
        g.forget("1").await.unwrap();
        let a = g.accounts().await.unwrap();
        assert!(a.accounts.is_empty() && a.active.is_empty());
        assert_eq!(start(&m, &g, false).await["client_id"], "dynamic_agent_client");
    }

    /// Use Another Account whose code is refused: the account in use stays in use and signed in;
    /// only the retry reuses the registration OpenAI issued for the new one.
    #[tokio::test]
    async fn refused_code_for_another_account() {
        let (m, g) = mock().await;
        let q = start(&m, &g, false).await;
        g.finish(&q["state"], "the-code", Some("issued-1")).await.unwrap();
        let q = start(&m, &g, true).await;
        assert_eq!(q["client_id"], "dynamic_agent_client");
        assert!(g
            .finish(&q["state"], "bad-code", Some("issued-2"))
            .await
            .unwrap_err()
            .downcast_ref::<CodeRefused>()
            .is_some());
        let st = g.status().await;
        assert_eq!(
            (
                st["signed_in"].as_bool(),
                st["account"].as_str(),
                st["accounts"].as_array().unwrap().len()
            ),
            (Some(true), Some("1"), 1)
        );
        assert_eq!(g.access_token().await.unwrap(), "access-one-xxxxxxxx");
        // Sign In stays with the account in use; Use Another Account registers afresh.
        assert_eq!(start(&m, &g, false).await["client_id"], "issued-1");
        assert_eq!(start(&m, &g, true).await["client_id"], "dynamic_agent_client");
        let r = start_retry(&m, &g, &q["state"]).await;
        assert_eq!(r["client_id"], "issued-2");
        *m.sub.lock().unwrap() = "u2".into();
        let t = g.finish(&r["state"], "the-code", Some("issued-2")).await.unwrap();
        assert_eq!((t.id.as_str(), t.sub.as_str()), ("2", "u2"));
        assert_eq!(g.accounts().await.unwrap().accounts.len(), 2);
    }

    /// An unfinished registration kept as the one in use by an earlier build isn't used.
    #[test]
    fn unfinished_is_never_in_use() {
        let a = Accounts::from_stored(Some(&json!({"active": "1", "accounts": [{"id": "1", "client_id": "issued-1"}]})));
        assert!(a.active.is_empty() && a.accounts[0].pending());
        // A signed-out registration with an account is not unfinished.
        let a = Accounts::from_stored(Some(
            &json!({"active": "1", "accounts": [{"id": "1", "client_id": "issued-1", "email": "a@example.com", "signed_out": true}]}),
        ));
        assert_eq!(a.active, "1");
        assert!(!a.accounts[0].pending());
    }

    /// A new registration's callback must name the client OpenAI issued: not the registration
    /// entry point, not an odd value. Nothing is kept or exchanged then.
    #[tokio::test]
    async fn callback_must_name_an_issued_client() {
        let (m, g) = mock().await;
        for bad in ["dynamic_agent_client", "issued 1", "issued-1&x=y", ""] {
            let q = start(&m, &g, false).await;
            let e = g.finish(&q["state"], "the-code", Some(bad)).await.unwrap_err();
            assert!(format!("{e:#}").contains("register"), "{bad}: {e:#}");
        }
        assert!(g.accounts().await.unwrap().accounts.is_empty());
        assert!(m.token_requests.lock().unwrap().is_empty());
    }

    /// Codes like a real one may be: long, JWT-shaped, with every character a URL can carry.
    fn realistic_codes() -> Vec<String> {
        let header = b64(br#"{"alg":"RS256","kid":"k1","typ":"JWT"}"#);
        let body = "aB3-_".repeat(300);
        vec![
            format!("{header}.{body}.{}", "sig_-9Z".repeat(49)),
            format!("ac_{}", "Zz09-_.~".repeat(237)),
            format!("{}+/=", "Ab+C9/Ef=".repeat(211)),
            "a%2Bb%2Fc%3D%25.d-e_f~g h".repeat(70),
        ]
    }

    #[test]
    fn what_is_logged_and_shown() {
        let code = "ac_0123456789abcdefghijklmnop.QRSTUV-wxyz_";
        let shape = code_shape(code);
        assert_eq!(
            shape,
            format!("{} chars, ac_ prefix, chars [A-Z a-z 0-9 - _ .], 2 parts (29+12)", code.len())
        );
        assert!(!shape.contains("0123"));
        assert!(code_shape("x y").contains("space"));
        let [jwt, ac, b64std, escaped] = realistic_codes().try_into().unwrap();
        let shape = code_shape(&jwt);
        assert!(
            shape.starts_with(&format!("{} chars, no ac_ prefix, chars [A-Z a-z 0-9 - _ .], 3 parts (", jwt.len())),
            "{shape}"
        );
        assert!(shape.ends_with("JWT-like header {alg,kid,typ} alg=RS256 typ=JWT"), "{shape}");
        assert!(code_shape(&ac).starts_with(&format!("{} chars, ac_ prefix, chars [A-Z a-z 0-9 - _ . ~]", ac.len())));
        assert!(code_shape(&b64std).contains("chars [A-Z a-z 0-9 + / =]"), "{}", code_shape(&b64std));
        let shape = code_shape(&escaped);
        assert!(shape.contains("% space]") && shape.contains("has %XX escapes"), "{shape}");
        for c in realistic_codes() {
            // No piece of a code is ever in its shape (beyond what is said above).
            let shape = code_shape(&c);
            assert!(
                !shape.contains("aB3-_aB3") && !shape.contains("Zz09") && !shape.contains("Ab+C9") && !shape.contains("a%2B"),
                "{shape}"
            );
        }
        let e = TokenError {
            status: 400,
            code: "invalid_grant".into(),
            reason: String::new(),
            description: String::new(),
            request_id: "r1".into(),
        };
        assert_eq!(e.to_string(), "ChatGPT sign-in: HTTP 400 invalid_grant; request r1");
        assert!(e.terminal());
        assert!(TokenError {
            code: "x".into(),
            reason: "refresh_token_invalidated".into(),
            ..e.clone()
        }
        .terminal());
        assert!(!TokenError {
            code: "invalid_client".into(),
            ..e
        }
        .terminal());
        assert!(issued_client_id("oaiapp_AbC-1_2"));
        assert!(!issued_client_id("dynamic_agent_client") && !issued_client_id("a/b") && !issued_client_id(""));
    }

    /// The code the app passes on reaches OpenAI's token endpoint byte for byte, however long,
    /// and whatever characters it has (`+`, `/`, `=`, `%`, `.`, `-`, `_`, `~`, a space): the
    /// form encoding is undone exactly by the other side.
    #[tokio::test]
    async fn long_codes_reach_openai_unchanged() {
        let (m, g) = mock().await;
        for (i, code) in realistic_codes().into_iter().enumerate() {
            *m.sub.lock().unwrap() = "u1".into();
            *m.code.lock().unwrap() = code.clone();
            let q = start(&m, &g, i == 0).await;
            let t = g.finish(&q["state"], &code, Some("issued-1")).await.unwrap();
            assert_eq!(t.client_id, "issued-1");
            assert_eq!(m.token_requests.lock().unwrap().last().unwrap()["code"], code);
            g.sign_out(None).await.unwrap();
        }
    }

    /// An app that doesn't try again by itself (from before server 0.22.2): a refused code
    /// drops the registration at once, nothing waits for a retry, the message says to update
    /// the app, and the next Sign In registers afresh.
    #[tokio::test]
    async fn refused_code_for_an_app_that_doesnt_retry() {
        let (m, g) = mock().await;
        let q = start(&m, &g, false).await;
        let e = g.finish_from(&q["state"], "bad-code", Some("issued-1"), false).await.unwrap_err();
        assert!(e.downcast_ref::<CodeRefused>().is_none());
        let text = format!("{e:#}");
        assert!(
            text.contains("invalid_grant") && text.contains("request req-123") && text.contains("update the app"),
            "{text}"
        );
        assert!(g.accounts().await.unwrap().accounts.is_empty());
        assert!(g.start(CALLBACK, false, None, Some(&q["state"])).await.is_err());
        assert_eq!(start(&m, &g, false).await["client_id"], "dynamic_agent_client");
    }

    /// What a server holds after a failed sign-in: an unfinished registration whose
    /// retry is gone (expired, or the server restarted). Sign In registers afresh and completes;
    /// the leftover stays listed until Forget.
    #[tokio::test]
    async fn a_leftover_unfinished_registration_doesnt_block_a_fresh_sign_in() {
        let (m, g) = mock().await;
        let q = start(&m, &g, false).await;
        assert!(g
            .finish(&q["state"], "bad-code", Some("issued-2"))
            .await
            .unwrap_err()
            .downcast_ref::<CodeRefused>()
            .is_some());
        g.retries.lock().unwrap().clear();
        assert!(g.start(CALLBACK, false, None, Some(&q["state"])).await.is_err());
        let q = start(&m, &g, false).await;
        assert_eq!(
            (q["client_id"].as_str(), q["agent_name_hint"].as_str()),
            ("dynamic_agent_client", "Moonrise")
        );
        let t = g.finish(&q["state"], "the-code", Some("issued-1")).await.unwrap();
        assert_eq!((t.id.as_str(), t.email.as_str()), ("2", "u1@example.com"));
        let st = g.status().await;
        assert_eq!(
            (st["signed_in"].as_bool(), st["unfinished"][0]["id"].as_str()),
            (Some(true), Some("1"))
        );
        g.forget("1").await.unwrap();
        assert_eq!(g.status().await["unfinished"], json!([]));
        assert_eq!(g.status().await["account"], "2");
    }
}
