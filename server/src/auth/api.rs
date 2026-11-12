//! Sign-in endpoints (Phase 3). JSON bodies; tokens only ever in bodies and the
//! `Authorization` header, never in URLs.
//!
//! - `POST /v1/auth/login {username, password, device_id, device_name}` → `{token, user}`
//!   (401 wrong name or password; 429 after 10 failures in 15 min from the address or for the name)
//! - `POST /v1/auth/logout` (Bearer): revokes the token → 204
//! - `GET /v1/me` (Bearer): `{id, name, is_admin, device: {…}}`
//! - `POST /v1/auth/device {device_id, device_name}` → `{device_code, user_code, verify_url,
//!   verify_url_complete, interval, expires_in}`
//! - `POST /v1/auth/device/poll {device_code}` → `{status: "pending"|"slow_down"|"expired"}` or
//!   `{status: "approved", token, user}` (once)
//! - `POST /v1/auth/device/approve {user_code}` (Bearer) → `{approved: true, device_name}`
//! - `GET /link[?code=]`, `POST /link`: a plain form (user name, password, code) that approves a
//!   code from any browser. CSRF: a SameSite=Strict cookie that must match a hidden field, and
//!   the Origin must be this host. Same rate limits as login.

use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    extract::{ConnectInfo, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Form, Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{device, Auth, LOGIN_MAX_FAILURES, LOGIN_WINDOW};
use crate::{
    api::{ApiError, AppState},
    library::util,
};

type Q = Query<HashMap<String, String>>;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/auth/login", post(login))
        .route("/v1/auth/logout", post(logout))
        .route("/v1/me", get(me))
        .route("/v1/auth/device", post(device_start))
        .route("/v1/auth/device/poll", post(device_poll))
        .route("/v1/auth/device/approve", post(device_approve))
        .route("/link", get(link_form).post(link_submit))
}

fn auth(st: &AppState) -> Result<&Arc<Auth>, ApiError> {
    st.auth
        .as_ref()
        .ok_or_else(|| ApiError(StatusCode::SERVICE_UNAVAILABLE, "Moonrise accounts are switched off".into()))
}

fn internal(e: anyhow::Error) -> ApiError {
    ApiError::internal(e)
}

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
    device_id: Option<String>,
    device_name: Option<String>,
}

/// Why a password attempt is refused before checking it.
fn limited(a: &Auth, ip: &str, name: &str) -> bool {
    !a.limiter.allowed(&format!("ip:{ip}"), LOGIN_MAX_FAILURES, LOGIN_WINDOW)
        || !a
            .limiter
            .allowed(&format!("user:{}", name.to_lowercase()), LOGIN_MAX_FAILURES, LOGIN_WINDOW)
}

fn failed(a: &Auth, ip: &str, name: &str) {
    a.limiter.hit(&format!("ip:{ip}"));
    a.limiter.hit(&format!("user:{}", name.to_lowercase()));
    tracing::warn!(%ip, user = ?name.chars().take(64).collect::<String>(), "sign-in failed");
}

async fn password_check(a: &Auth, ip: &str, name: &str, password: &str) -> Result<super::User, ApiError> {
    if limited(a, ip, name) {
        tracing::warn!(%ip, "sign-in rate-limited");
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "too many failed attempts; try again later".into(),
        ));
    }
    if name.len() > 64 || password.len() > 1024 {
        failed(a, ip, name);
        return Err(ApiError(StatusCode::UNAUTHORIZED, "wrong user name or password".into()));
    }
    match a.check_password(name.to_string(), password.to_string()).await.map_err(internal)? {
        Some(u) => Ok(u),
        None => {
            failed(a, ip, name);
            Err(ApiError(StatusCode::UNAUTHORIZED, "wrong user name or password".into()))
        }
    }
}

async fn login(
    State(st): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(b): Json<LoginBody>,
) -> Result<Json<Value>, ApiError> {
    let a = auth(&st)?;
    let ip = addr.ip().to_string();
    let user = password_check(a, &ip, &b.username, &b.password).await?;
    let token = a
        .issue(user.id.clone(), b.device_id.clone(), b.device_name.clone())
        .await
        .map_err(internal)?;
    tracing::info!(user = %user.name, device = ?b.device_name, %ip, "signed in");
    Ok(Json(json!({"token": token, "user": user})))
}

async fn logout(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<StatusCode, ApiError> {
    let (user, hash) = super::moonrise_caller(&st, &headers, &q).await?;
    auth(&st)?.revoke(hash).await.map_err(internal)?;
    tracing::info!(user = %user.name, "signed out");
    Ok(StatusCode::NO_CONTENT)
}

async fn me(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    let (user, hash) = super::moonrise_caller(&st, &headers, &q).await?;
    let device = auth(&st)?.token_info(hash).await.map_err(internal)?;
    let mut v = json!(*user);
    v["device"] = device;
    Ok(Json(v))
}

#[derive(Deserialize, Default)]
struct DeviceBody {
    device_id: Option<String>,
    device_name: Option<String>,
}

/// This server's address as the client sees it (for the `/link` URL shown on the TV).
fn base_url(headers: &HeaderMap, st: &AppState) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .filter(|h| h.chars().all(|c| c.is_ascii_alphanumeric() || ".-:[]".contains(c)))
        .map_or_else(|| st.config.listen.to_string(), String::from);
    format!("http://{host}")
}

async fn device_start(
    State(st): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Option<Json<DeviceBody>>,
) -> Result<Json<Value>, ApiError> {
    let a = auth(&st)?;
    let key = format!("device:{}", addr.ip());
    if !a.limiter.allowed(&key, 30, Duration::from_secs(600)) {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "too many codes requested; try again later".into(),
        ));
    }
    a.limiter.hit(&key);
    let b = body.map(|b| b.0).unwrap_or_default();
    let s =
        a.db.call(move |c| device::start(c, b.device_id.as_deref(), b.device_name.as_deref(), util::now()))
            .await
            .map_err(internal)?;
    let base = base_url(&headers, &st);
    tracing::info!(ip = %addr.ip(), "device code started");
    Ok(Json(json!({
        "device_code": s.device_code,
        "user_code": s.user_code,
        "verify_url": format!("{base}/link"),
        "verify_url_complete": format!("{base}/link?code={}", s.user_code),
        "interval": device::INTERVAL_SECS,
        "expires_in": device::EXPIRES_SECS,
    })))
}

#[derive(Deserialize)]
struct PollBody {
    device_code: String,
}

async fn device_poll(State(st): State<Arc<AppState>>, Json(b): Json<PollBody>) -> Result<Json<Value>, ApiError> {
    let a = auth(&st)?;
    let code = b.device_code;
    if code.len() > 100 {
        return Ok(Json(json!({"status": "expired"})));
    }
    let r = a.db.call(move |c| device::poll(c, &code, util::now())).await.map_err(internal)?;
    Ok(Json(match r {
        device::Poll::Pending => json!({"status": "pending"}),
        device::Poll::SlowDown => json!({"status": "slow_down", "interval": device::INTERVAL_SECS}),
        device::Poll::Expired => json!({"status": "expired"}),
        device::Poll::Approved { token, user } => {
            tracing::info!(user = %user.name, "device signed in with a code");
            json!({"status": "approved", "token": token, "user": user})
        }
    }))
}

#[derive(Deserialize)]
struct ApproveBody {
    user_code: String,
}

async fn device_approve(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Q,
    Json(b): Json<ApproveBody>,
) -> Result<Json<Value>, ApiError> {
    let (user, _) = super::moonrise_caller(&st, &headers, &q).await?;
    let a = auth(&st)?;
    let key = format!("approve:{}", user.id);
    if !a.limiter.allowed(&key, 20, Duration::from_secs(600)) {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "too many wrong codes; try again later".into(),
        ));
    }
    let uid = user.id.clone();
    let code = b.user_code;
    match a
        .db
        .call(move |c| device::approve(c, &code, &uid, util::now()))
        .await
        .map_err(internal)?
    {
        Some(device_name) => {
            tracing::info!(user = %user.name, device = ?device_name, "device code approved");
            Ok(Json(json!({"approved": true, "device_name": device_name})))
        }
        None => {
            a.limiter.hit(&key);
            Err(ApiError(StatusCode::NOT_FOUND, "no such code, or it has expired".into()))
        }
    }
}

// ---------------------------------------------------------------- /link

const CSRF_COOKIE: &str = "moonrise_link_csrf";

fn esc(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&#39;".into(),
            c => c.to_string(),
        })
        .collect()
}

fn page(status: StatusCode, body: &str, set_cookie: Option<String>) -> Response {
    let html = format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Moonrise: sign in a device</title>
<style>body{{font:17px -apple-system,system-ui,sans-serif;max-width:26rem;margin:2rem auto;padding:0 1rem;color:#111;background:#fafafa}}
h1{{font-size:1.4rem}}label{{display:block;margin:.9rem 0 .3rem;font-weight:600}}input{{width:100%;box-sizing:border-box;font-size:1.1rem;padding:.6rem;border:1px solid #bbb;border-radius:.5rem}}
input[name=code]{{text-transform:uppercase;letter-spacing:.15em}}button{{margin-top:1.3rem;width:100%;font-size:1.1rem;padding:.7rem;border:0;border-radius:.5rem;background:#c0392b;color:#fff}}
.msg{{padding:.8rem;border-radius:.5rem;background:#eee}}.err{{background:#fde2e0}}.ok{{background:#e0f5e4}}
@media (prefers-color-scheme:dark){{body{{background:#111;color:#eee}}input{{background:#222;color:#eee;border-color:#444}}.msg{{background:#222}}.err{{background:#4a1f1b}}.ok{{background:#1c3d24}}}}</style></head>
<body><h1>Sign in a device to Moonrise</h1>{body}</body></html>"#
    );
    let mut resp = (status, Html(html)).into_response();
    let h = resp.headers_mut();
    h.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'"
            .parse()
            .unwrap(),
    );
    h.insert(header::X_FRAME_OPTIONS, "DENY".parse().unwrap());
    // Not "no-referrer": with it browsers send `Origin: null` on the form's POST, which the
    // same-origin check refuses ("The form expired" in Chrome). "same-origin" still sends nothing
    // to other sites.
    h.insert(header::REFERRER_POLICY, "same-origin".parse().unwrap());
    h.insert("x-content-type-options", "nosniff".parse().unwrap());
    if let Some(c) = set_cookie.and_then(|c| c.parse().ok()) {
        h.insert(header::SET_COOKIE, c);
    }
    resp
}

fn form_html(code: &str, csrf: &str, message: Option<(&str, &str)>) -> String {
    let msg = message
        .map(|(class, m)| format!(r#"<p class="msg {class}">{}</p>"#, esc(m)))
        .unwrap_or_default();
    format!(
        r#"{msg}<p>Enter the code shown on the TV, and your Moonrise user name and password.</p>
<form method="post" action="/link" autocomplete="on">
<input type="hidden" name="csrf" value="{}">
<label for="code">Code</label><input id="code" name="code" value="{}" placeholder="ABCD-1234" autocomplete="off" autocapitalize="characters" spellcheck="false" required maxlength="12">
<label for="u">User name</label><input id="u" name="username" autocomplete="username" autocapitalize="none" spellcheck="false" required maxlength="64">
<label for="p">Password</label><input id="p" name="password" type="password" autocomplete="current-password" required maxlength="1024">
<button type="submit">Sign in the device</button></form>"#,
        esc(csrf),
        esc(code)
    )
}

fn new_csrf() -> (String, String) {
    let v: [u8; 16] = rand::random();
    let v: String = v.iter().map(|b| format!("{b:02x}")).collect();
    let cookie = format!("{CSRF_COOKIE}={v}; Path=/link; HttpOnly; SameSite=Strict; Max-Age=1800");
    (v, cookie)
}

async fn link_form(State(st): State<Arc<AppState>>, Query(q): Q) -> Response {
    if st.auth.is_none() {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            r#"<p class="msg err">Moonrise accounts are switched off on this server.</p>"#,
            None,
        );
    }
    let code = q.get("code").and_then(|c| device::normalize_user_code(c)).unwrap_or_default();
    let (csrf, cookie) = new_csrf();
    page(StatusCode::OK, &form_html(&code, &csrf, None), Some(cookie))
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|kv| {
            let (k, v) = kv.trim().split_once('=')?;
            (k == name).then_some(v)
        })
}

/// The request comes from this site's own form: same Origin (when the browser sends one) and
/// the CSRF cookie matches the form field.
fn same_site(headers: &HeaderMap, form_csrf: &str) -> bool {
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|o| o.to_str().ok()) {
        if origin != format!("http://{host}") && origin != format!("https://{host}") {
            return false;
        }
    }
    if let Some(site) = headers.get("sec-fetch-site").and_then(|o| o.to_str().ok()) {
        if site != "same-origin" && site != "none" {
            return false;
        }
    }
    cookie_value(headers, CSRF_COOKIE).is_some_and(|c| !c.is_empty() && super::ct_eq(c.as_bytes(), form_csrf.as_bytes()))
}

async fn link_submit(
    State(st): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(f): Form<HashMap<String, String>>,
) -> Response {
    let Some(a) = st.auth.as_ref() else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            r#"<p class="msg err">Moonrise accounts are switched off on this server.</p>"#,
            None,
        );
    };
    let get = |k: &str| f.get(k).map_or("", String::as_str);
    let raw_code = get("code");
    let shown_code = device::normalize_user_code(raw_code).unwrap_or_else(|| raw_code.chars().take(12).collect());
    let (csrf, cookie) = new_csrf();
    let again = |status: StatusCode, m: &str| page(status, &form_html(&shown_code, &csrf, Some(("err", m))), Some(cookie.clone()));
    if !same_site(&headers, get("csrf")) {
        tracing::warn!(ip = %addr.ip(), "link form: CSRF check failed");
        return again(StatusCode::FORBIDDEN, "The form expired. Please try again.");
    }
    let ip = addr.ip().to_string();
    let user = match password_check(a, &ip, get("username"), get("password")).await {
        Ok(u) => u,
        Err(ApiError(StatusCode::TOO_MANY_REQUESTS, _)) => {
            return again(StatusCode::TOO_MANY_REQUESTS, "Too many failed attempts. Try again in 15 minutes.")
        }
        Err(ApiError(StatusCode::UNAUTHORIZED, _)) => return again(StatusCode::UNAUTHORIZED, "Wrong user name or password."),
        Err(e) => return again(e.0, "Something went wrong. Please try again."),
    };
    let uid = user.id.clone();
    let code = raw_code.to_string();
    match a.db.call(move |c| device::approve(c, &code, &uid, util::now())).await {
        Ok(Some(device_name)) => {
            tracing::info!(user = %user.name, device = ?device_name, ip = %addr.ip(), "device code approved on /link");
            let what = device_name.map_or_else(|| "The device".into(), |d| format!("“{d}”"));
            page(
                StatusCode::OK,
                &format!(
                    r#"<p class="msg ok">{} is now signed in as <b>{}</b>. You can close this page.</p>"#,
                    esc(&what),
                    esc(&user.name)
                ),
                Some(format!("{CSRF_COOKIE}=; Path=/link; HttpOnly; SameSite=Strict; Max-Age=0")),
            )
        }
        Ok(None) => {
            // A wrong code counts as a failed attempt (codes can't be guessed through here).
            failed(a, &ip, get("username"));
            again(
                StatusCode::NOT_FOUND,
                "That code isn't valid or has expired. Check the code on the TV.",
            )
        }
        Err(e) => {
            tracing::warn!("link approve: {e:#}");
            again(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong. Please try again.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csrf_and_origin() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, "192.168.1.10:8097".parse().unwrap());
        h.insert(header::COOKIE, "a=b; moonrise_link_csrf=abc123".parse().unwrap());
        assert!(same_site(&h, "abc123"));
        assert!(!same_site(&h, "abc124"));
        assert!(!same_site(&h, ""));
        h.insert(header::ORIGIN, "http://192.168.1.10:8097".parse().unwrap());
        assert!(same_site(&h, "abc123"));
        h.insert(header::ORIGIN, "http://evil.example".parse().unwrap());
        assert!(!same_site(&h, "abc123"));
        h.remove(header::ORIGIN);
        h.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert!(!same_site(&h, "abc123"));
        let mut h2 = HeaderMap::new();
        h2.insert(header::HOST, "x".parse().unwrap());
        assert!(!same_site(&h2, "abc123"), "no cookie");
    }

    #[test]
    fn escaping() {
        assert_eq!(esc("<b>\"x\"&'"), "&lt;b&gt;&quot;x&quot;&amp;&#39;");
        assert!(form_html("<x>", "c", Some(("err", "<script>"))).contains("&lt;script&gt;"));
    }
}
