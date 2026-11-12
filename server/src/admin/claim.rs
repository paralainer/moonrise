//! Claiming a new server (Phase 4 of the open-source plan): a server with no users is
//! "unclaimed". It makes a 6-digit setup code at start (a new one on every restart), prints it
//! in its log and shows it at `GET /` (to clients on the local network only); the app asks for
//! it, then creates the first account, an admin, with it. After that the server is claimed, the
//! code is gone and `GET /` only says the server is set up.
//!
//! - `GET /v1/info` → `setup: "unclaimed" | "claimed"` (and Bonjour's TXT `setup=unclaimed`).
//! - `POST /v1/setup/verify {code}` → 204 (right code), 403 (wrong), 409 (claimed), 429.
//! - `POST /v1/setup/claim {code, username, password, device_id, device_name}` → `{token, user}`
//!   as a sign-in answers (the user is an admin); 403 wrong code, 409 claimed, 422 a bad name or
//!   password (the code isn't used up), 429.
//!
//! Wrong codes: 5 per client address in 15 minutes, then 429; after 10 wrong codes in all the
//! code is replaced by a new one (logged), so guessing gets nowhere. The CLI (`moonrise-server user
//! add <name> --admin`) still works and claims the server too.

use std::{
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    extract::{ConnectInfo, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rand::Rng;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    api::{ApiError, AppState},
    auth::{self, Limiter, User},
    db::Db,
};

/// Wrong codes allowed per client address in [`WINDOW`].
pub const MAX_FAILURES_PER_ADDRESS: usize = 5;
/// Wrong codes (from anywhere) after which the code is replaced.
pub const MAX_FAILURES_PER_CODE: usize = 10;
pub const WINDOW: Duration = Duration::from_mins(15);

/// Where the apps are (shown on the page at `/`). None until Moonrise is on the App Store.
const APP_STORE_URL: Option<&str> = None;

pub struct Claim {
    db: Db,
    /// The page's address, for the log.
    page: String,
    inner: Mutex<Inner>,
    limiter: Limiter,
    /// One claim at a time.
    claiming: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct Inner {
    claimed: bool,
    code: Option<String>,
    /// Wrong attempts at the current code.
    failures: usize,
}

#[derive(Debug, PartialEq)]
pub enum Refused {
    Claimed,
    TooMany,
    WrongCode,
}

impl Refused {
    fn api(&self) -> ApiError {
        match self {
            Refused::Claimed => ApiError(StatusCode::CONFLICT, "this server is set up already: sign in instead".into()),
            Refused::TooMany => ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "too many wrong codes; try again in a few minutes".into(),
            ),
            Refused::WrongCode => ApiError(StatusCode::FORBIDDEN, "that isn't the setup code".into()),
        }
    }
}

fn new_code() -> String {
    format!("{:06}", rand::thread_rng().gen_range(0..1_000_000u32))
}

/// "123456" → "123 456" (as shown).
pub fn spaced(code: &str) -> String {
    format!("{} {}", &code[..3], &code[3..])
}

/// What was typed, digits only ("123 456", "123-456").
fn digits(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).collect()
}

fn announce_code(code: &str, why: &str, page: &str) {
    let line = "=".repeat(64);
    tracing::warn!("{line}");
    tracing::warn!("  Moonrise server setup code: {}   ({why})", spaced(code));
    tracing::warn!("  Open the Moonrise app on an iPhone, iPad, Mac or Apple TV, pick this");
    tracing::warn!("  server and enter the code. It is also shown on this server's page:");
    tracing::warn!("  {page} (from the local network).");
    tracing::warn!("{line}");
}

/// Where the page with the code is: `http://<LAN address>:<port>/`.
pub fn page_url(port: u16) -> String {
    match crate::mdns::lan_addresses().first() {
        Some(ip) => format!("http://{ip}:{port}/"),
        None => format!("http://<this computer's address>:{port}/"),
    }
}

async fn user_count(db: &Db) -> anyhow::Result<i64> {
    db.call(|c| Ok(c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?)).await
}

impl Claim {
    /// The claim state as the database says (a code is made and logged when unclaimed).
    pub async fn open(db: Db, page: String) -> anyhow::Result<Claim> {
        let users = user_count(&db).await?;
        let claim = Claim {
            db,
            page,
            inner: Mutex::new(Inner::default()),
            limiter: Limiter::default(),
            claiming: tokio::sync::Mutex::new(()),
        };
        if users > 0 {
            claim.inner.lock().unwrap().claimed = true;
        } else {
            let code = new_code();
            announce_code(&code, "no accounts yet", &claim.page);
            claim.inner.lock().unwrap().code = Some(code);
        }
        Ok(claim)
    }

    /// Unclaimed, as last known (no database read; for Bonjour).
    pub fn unclaimed(&self) -> bool {
        !self.inner.lock().unwrap().claimed
    }

    /// Unclaimed, looking at the database again while it is (an account added with the CLI
    /// claims the server too). Answers (unclaimed, changed just now).
    pub async fn refresh(&self) -> (bool, bool) {
        if !self.unclaimed() {
            return (false, false);
        }
        match user_count(&self.db).await {
            Ok(n) if n > 0 => {
                let mut i = self.inner.lock().unwrap();
                let changed = !i.claimed;
                i.claimed = true;
                i.code = None;
                if changed {
                    tracing::info!("an account exists now (added with the CLI): the server is set up, the setup code is gone");
                }
                (false, changed)
            }
            _ => (true, false),
        }
    }

    /// The setup code (while unclaimed).
    pub fn code(&self) -> Option<String> {
        let i = self.inner.lock().unwrap();
        if i.claimed {
            None
        } else {
            i.code.clone()
        }
    }

    /// Checks a code typed at `ip`. Wrong ones count against the address and the code.
    pub fn check(&self, ip: &str, code: &str) -> Result<(), Refused> {
        let key = format!("setup:{ip}");
        let mut i = self.inner.lock().unwrap();
        if i.claimed {
            return Err(Refused::Claimed);
        }
        if !self.limiter.allowed(&key, MAX_FAILURES_PER_ADDRESS, WINDOW) {
            tracing::warn!(%ip, "setup code: too many wrong attempts from this address");
            return Err(Refused::TooMany);
        }
        let typed = digits(code);
        let right = i
            .code
            .as_deref()
            .is_some_and(|c| typed.len() == 6 && auth::ct_eq(typed.as_bytes(), c.as_bytes()));
        if right {
            return Ok(());
        }
        self.limiter.hit(&key);
        i.failures += 1;
        tracing::warn!(%ip, failures = i.failures, "setup code: wrong code");
        if i.failures >= MAX_FAILURES_PER_CODE {
            let code = new_code();
            announce_code(&code, "the old one was guessed at too often", &self.page);
            i.code = Some(code);
            i.failures = 0;
        }
        Err(Refused::WrongCode)
    }

    /// Creates the first account (an admin) with the setup code. The code is gone after.
    pub async fn claim(&self, ip: &str, code: &str, name: &str, password: &str) -> Result<User, ApiError> {
        let _g = self.claiming.lock().await;
        self.check(ip, code).map_err(|r| r.api())?;
        let (name, password) = (name.trim().to_string(), password.to_string());
        let user = self
            .db
            .call(move |c| {
                // One connection per process (and one claim at a time): nothing slips in between.
                let n: i64 = c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
                if n > 0 {
                    return Ok(Err(Refused::Claimed));
                }
                Ok(Ok(auth::add_user(c, &name, &password, true).map_err(|e| e.to_string())))
            })
            .await
            .map_err(ApiError::internal)?;
        let user = match user {
            Err(r) => {
                self.inner.lock().unwrap().claimed = true;
                return Err(r.api());
            }
            Ok(Err(msg)) => return Err(ApiError(StatusCode::UNPROCESSABLE_ENTITY, msg)),
            Ok(Ok(u)) => u,
        };
        {
            let mut i = self.inner.lock().unwrap();
            i.claimed = true;
            i.code = None;
        }
        tracing::info!(user = %user.name, %ip, "server claimed: first account (admin) created with the setup code");
        Ok(user)
    }
}

// ---------------------------------------------------------------- HTTP

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(page))
        .route("/v1/setup/verify", post(verify))
        .route("/v1/setup/claim", post(claim))
}

fn claim_of(st: &AppState) -> Result<&Arc<Claim>, ApiError> {
    st.claim
        .as_ref()
        .ok_or_else(|| ApiError(StatusCode::SERVICE_UNAVAILABLE, "Moonrise accounts are switched off".into()))
}

/// The state changed under us (an account made with the CLI): Bonjour says so too.
pub async fn refresh(st: &Arc<AppState>) -> Option<bool> {
    let c = st.claim.as_ref()?;
    let (unclaimed, changed) = c.refresh().await;
    if changed {
        reannounce(st);
    }
    Some(unclaimed)
}

fn reannounce(st: &Arc<AppState>) {
    let st = st.clone();
    tokio::task::spawn_blocking(move || st.announce());
}

#[derive(Deserialize)]
struct VerifyBody {
    code: String,
}

async fn verify(
    State(st): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(b): Json<VerifyBody>,
) -> Result<StatusCode, ApiError> {
    let c = claim_of(&st)?;
    refresh(&st).await;
    c.check(&addr.ip().to_string(), &b.code).map_err(|r| r.api())?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ClaimBody {
    code: String,
    username: String,
    password: String,
    device_id: Option<String>,
    device_name: Option<String>,
}

async fn claim(
    State(st): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(b): Json<ClaimBody>,
) -> Result<Json<Value>, ApiError> {
    let c = claim_of(&st)?;
    let a = st
        .auth
        .as_ref()
        .ok_or_else(|| ApiError(StatusCode::SERVICE_UNAVAILABLE, "Moonrise accounts are switched off".into()))?;
    refresh(&st).await;
    let ip = addr.ip().to_string();
    let user = c.claim(&ip, &b.code, &b.username, &b.password).await?;
    reannounce(&st);
    let token = a
        .issue(user.id.clone(), b.device_id.clone(), b.device_name.clone())
        .await
        .map_err(ApiError::internal)?;
    tracing::info!(user = %user.name, device = ?b.device_name, %ip, "signed in");
    Ok(Json(json!({"token": token, "user": user})))
}

/// Addresses that may see the code on the page: the local network (private, link-local,
/// loopback) and Tailscale's (100.64/10). Anyone else is told to look in the log.
pub fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private() || v4.is_loopback() || v4.is_link_local() || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_local(IpAddr::V4(v4));
            }
            let s = v6.segments()[0];
            v6.is_loopback() || (s & 0xfe00) == 0xfc00 || (s & 0xffc0) == 0xfe80
        }
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The page at `/`: the setup code while unclaimed, else that the server is set up.
async fn page(State(st): State<Arc<AppState>>, ConnectInfo(addr): ConnectInfo<SocketAddr>) -> Response {
    let unclaimed = refresh(&st).await;
    let name = esc(&st.name());
    let get_app = match APP_STORE_URL {
        Some(u) => format!(
            r#"<p><a href="{}">Get Moonrise on the App Store</a> for iPhone, iPad, Mac and Apple TV.</p>"#,
            esc(u)
        ),
        None => "<p>Moonrise for iPhone, iPad, Mac and Apple TV: App Store link coming soon.</p>".to_string(),
    };
    let body = match (unclaimed, st.claim.as_ref().and_then(|c| c.code())) {
        (Some(true), Some(code)) if is_local(addr.ip()) => format!(
            r#"<p>This Moonrise server isn't set up yet. Open the Moonrise app, pick <b>{name}</b> and enter this setup code:</p>
<p class="code">{}</p>{get_app}<p class="small">The code is also in the server's log. It changes when the server restarts, and it is gone once the server is set up.</p>"#,
            spaced(&code)
        ),
        (Some(true), _) => format!(
            r"<p>This Moonrise server isn't set up yet. Open the Moonrise app and pick <b>{name}</b>; the setup code is in the server's log.</p>{get_app}"
        ),
        _ => format!(r"<p><b>{name}</b> is set up. Sign in from the Moonrise app.</p>{get_app}"),
    };
    let html = format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Moonrise server</title>
<style>body{{font:17px -apple-system,system-ui,sans-serif;max-width:30rem;margin:3rem auto;padding:0 1.2rem;color:#111;background:#fafafa;line-height:1.45}}
h1{{font-size:1.5rem}}.code{{font:700 3rem ui-monospace,Menlo,monospace;letter-spacing:.12em;margin:1.2rem 0;padding:.6rem 1rem;background:#eee;border-radius:.8rem;text-align:center}}
.small{{font-size:.85rem;color:#666}}a{{color:#c0392b}}
@media (prefers-color-scheme:dark){{body{{background:#111;color:#eee}}.code{{background:#222}}.small{{color:#999}}}}</style></head>
<body><h1>Moonrise server</h1>{body}</body></html>"#
    );
    let mut resp = (StatusCode::OK, Html(html)).into_response();
    let h = resp.headers_mut();
    h.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'"
            .parse()
            .unwrap(),
    );
    h.insert(header::X_FRAME_OPTIONS, "DENY".parse().unwrap());
    h.insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
    h.insert("x-content-type-options", "nosniff".parse().unwrap());
    resp
}

/// `{setup}` for `/v1/info`.
pub fn info_state(unclaimed: Option<bool>) -> Option<&'static str> {
    unclaimed.map(|u| if u { "unclaimed" } else { "claimed" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn claim_once_with_the_code() {
        let db = Db::memory().unwrap();
        let c = Claim::open(db.clone(), String::new()).await.unwrap();
        assert!(c.unclaimed());
        let code = c.code().unwrap();
        assert_eq!(code.len(), 6);
        let wrong = if code == "000000" { "111111" } else { "000000" };
        assert_eq!(c.check("10.0.0.2", wrong), Err(Refused::WrongCode));
        assert_eq!(c.check("10.0.0.2", &spaced(&code)), Ok(()), "spaces are fine");
        // A bad password doesn't use the code up.
        let e = c.claim("10.0.0.2", &code, "sam", "short").await.unwrap_err();
        assert_eq!(e.0, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(c.unclaimed());
        let u = c
            .claim("10.0.0.2", &format!("{}-{}", &code[..3], &code[3..]), " sam ", "password123")
            .await
            .unwrap();
        assert_eq!(u.name, "sam");
        assert!(u.is_admin);
        assert!(!c.unclaimed());
        assert!(c.code().is_none());
        assert_eq!(c.check("10.0.0.2", &code), Err(Refused::Claimed));
        assert_eq!(
            c.claim("10.0.0.3", &code, "eve", "password123").await.unwrap_err().0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            db.call(|c| Ok(c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))?))
                .await
                .unwrap(),
            1
        );
        // A server with users starts claimed.
        let c2 = Claim::open(db, String::new()).await.unwrap();
        assert!(!c2.unclaimed());
        assert!(c2.code().is_none());
    }

    #[tokio::test]
    async fn wrong_codes_are_limited_and_rotate_the_code() {
        let c = Claim::open(Db::memory().unwrap(), String::new()).await.unwrap();
        let code = c.code().unwrap();
        let wrong = if code == "000000" { "111111" } else { "000000" };
        for _ in 0..MAX_FAILURES_PER_ADDRESS {
            assert_eq!(c.check("10.0.0.9", wrong), Err(Refused::WrongCode));
        }
        assert_eq!(
            c.check("10.0.0.9", &code),
            Err(Refused::TooMany),
            "locked out, even with the right code"
        );
        assert_eq!(c.check("10.0.0.8", &code), Ok(()), "others aren't");
        for i in 0..MAX_FAILURES_PER_CODE - MAX_FAILURES_PER_ADDRESS {
            let _ = c.check(&format!("10.0.1.{i}"), "12");
        }
        let new = c.code().unwrap();
        // Rotated after 10 wrong codes (it could, rarely, come out the same).
        assert!(new != code || c.inner.lock().unwrap().failures == 0);
        assert_eq!(c.inner.lock().unwrap().failures, 0);
    }

    #[tokio::test]
    async fn cli_account_claims_too() {
        let db = Db::memory().unwrap();
        let c = Claim::open(db.clone(), String::new()).await.unwrap();
        assert_eq!(c.refresh().await, (true, false));
        db.call(|c| auth::add_user(c, "cli", "password123", true)).await.unwrap();
        assert_eq!(c.refresh().await, (false, true));
        assert!(c.code().is_none());
    }

    #[test]
    fn local_addresses() {
        for ip in [
            "192.168.1.5",
            "10.1.2.3",
            "172.16.0.1",
            "127.0.0.1",
            "100.101.1.2",
            "169.254.1.1",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:192.168.1.5",
        ] {
            assert!(is_local(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "100.128.0.1", "2001:db8::1", "::ffff:8.8.8.8"] {
            assert!(!is_local(ip.parse().unwrap()), "{ip}");
        }
    }
}
