//! The apps' "Manage Server" screens (admins only; the server has no web admin): accounts and
//! their devices, invitations, the server itself (version, uptime, disk use, transcoding
//! hardware), what it is doing (playback, jobs, scans, recent errors), its log, a restart, and
//! checking an OpenSubtitles account before it is saved.
//!
//! - `GET /v1/admin/users` `{items: [{id, name, is_admin, created_at, devices, last_used, me}]}`;
//!   `POST /v1/admin/users {name, password, admin?}`; `PATCH /v1/admin/users/{id} {password?,
//!   admin?}`; `DELETE /v1/admin/users/{id}` (not yourself, not the last admin: 409). Watch state
//!   and tokens go with a deleted account.
//! - `GET /v1/admin/users/{id}/devices` the signed-in devices (tokens not revoked): `{items:
//!   [{id, device_name, device_id, created_at, last_used, current}]}` (`id`: 16 hex characters of
//!   the token's hash, enough to name it, nothing to sign in with); `DELETE
//!   /v1/admin/users/{id}/devices/{device}` signs that device out (not the one asking: 409);
//!   `DELETE /v1/admin/users/{id}/devices` every device but the one asking.
//! - Invitations: `POST /v1/admin/invites {admin?, days?}` → `{id, code, expires_at, admin}` (the
//!   code is shown once: 8 characters, `XXXX-XXXX`), `GET /v1/admin/invites` (pending ones,
//!   without codes), `DELETE /v1/admin/invites/{id}`. A new user enters it on the sign-in screen:
//!   `POST /v1/auth/invite {code, username, password, device_*}` → `{token, user}` (wrong codes are
//!   rate-limited like passwords).
//! - `GET /v1/admin/system`: version, start time and uptime, disk use (database, extracted
//!   subtitles, artwork, cache and its limit, prepared downloads), transcoding hardware (NVIDIA
//!   GPU, other GPUs, CPU) and what transcoding uses, whether the server can restart itself.
//! - `GET /v1/admin/activity`: playback sessions now (who, what, copied or transcoded, how far),
//!   transcode runs, subtitle extraction and backfill, keyframe indexes made ahead of need,
//!   prepared downloads, the library's pass,
//!   and the last warnings and errors from the log.
//! - `GET /v1/admin/logs?lines=200&level=warn|error`: the log's last lines (kept in memory since
//!   the start), secrets redacted (`logbuf.rs`).
//! - `POST /v1/admin/restart`: only when `MOONRISE_RESTART=exit` says something restarts the server
//!   when it exits (Docker's `restart: unless-stopped`); otherwise 409. The server answers, then
//!   exits a moment later.
//! - `POST /v1/admin/services/opensubtitles/check {username, password}`: signs in to
//!   OpenSubtitles with them (and the key in use) without saving anything: `{ok, allowed_downloads,
//!   vip}` or 422 `{error}`.

use std::{
    collections::HashMap,
    net::SocketAddr,
    path::Path,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use axum::{
    extract::{ConnectInfo, Path as UrlPath, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get, post},
    Json, Router,
};
use serde_json::{json, Value};

use super::store;
use crate::{
    api::{ApiError, AppState},
    auth::{self, Caller},
    library::util,
};

type Q = Query<HashMap<String, String>>;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/admin/users", get(list_users).post(create_user))
        .route("/v1/admin/users/{id}", axum::routing::patch(update_user).delete(delete_user))
        .route("/v1/admin/users/{id}/devices", get(list_devices).delete(revoke_all_devices))
        .route("/v1/admin/users/{id}/devices/{device}", delete(revoke_device))
        .route("/v1/admin/invites", get(list_invites).post(create_invite))
        .route("/v1/admin/invites/{id}", delete(delete_invite))
        .route("/v1/auth/invite", post(redeem_invite))
        .route("/v1/admin/system", get(system))
        .route("/v1/admin/activity", get(activity))
        .route("/v1/admin/logs", get(logs))
        .route("/v1/admin/restart", post(restart))
        .route("/v1/admin/services/opensubtitles/check", post(check_opensubtitles))
}

static STARTED: OnceLock<(Instant, i64)> = OnceLock::new();

/// Called once as the server starts serving (uptime).
pub fn mark_started() {
    let _ = STARTED.set((Instant::now(), util::now()));
}

fn internal(e: anyhow::Error) -> ApiError {
    ApiError::internal(e)
}

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

fn conflict(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::CONFLICT, msg.into())
}

fn accounts(st: &AppState) -> Result<&Arc<auth::Auth>, ApiError> {
    st.auth
        .as_ref()
        .ok_or_else(|| ApiError(StatusCode::SERVICE_UNAVAILABLE, "Moonrise accounts are switched off".into()))
}

/// The admin asking (a Moonrise admin or the CLI's token), and the accounts.
async fn admin(st: &AppState, headers: &HeaderMap, q: &HashMap<String, String>) -> Result<(Caller, Arc<auth::Auth>), ApiError> {
    let c = auth::admin_caller(st, headers, q).await?;
    Ok((c, accounts(st)?.clone()))
}

// ---------------------------------------------------------------- users

async fn list_users(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    let (caller, a) = admin(&st, &headers, &q).await?;
    let me = caller.user_id();
    let items = a.db.call(move |c| store::list_users(c, me.as_deref())).await.map_err(internal)?;
    Ok(Json(json!({"items": items, "total": items.len()})))
}

async fn create_user(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Q,
    Json(b): Json<Value>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let (_, a) = admin(&st, &headers, &q).await?;
    let name = b["name"].as_str().unwrap_or("").trim().to_string();
    let password = b["password"].as_str().unwrap_or("").to_string();
    let is_admin = b["admin"].as_bool().unwrap_or(false);
    let n2 = name.clone();
    let exists = a.db.call(move |c| auth::user_by_name(c, &n2)).await.map_err(internal)?.is_some();
    if exists {
        return Err(conflict(format!("there is already an account called {name}")));
    }
    let u =
        a.db.call(move |c| auth::add_user(c, &name, &password, is_admin))
            .await
            .map_err(|e| bad(format!("{e:#}")))?;
    tracing::info!(user = %u.name, admin = u.is_admin, "account added");
    Ok((StatusCode::CREATED, Json(json!(u))))
}

/// The accounts: (id, name, is_admin).
async fn users(a: &auth::Auth) -> Result<Vec<(String, String, bool)>, ApiError> {
    a.db.call(|c| store::users(c)).await.map_err(internal)
}

async fn update_user(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    Query(q): Q,
    Json(b): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let (caller, a) = admin(&st, &headers, &q).await?;
    let all = users(&a).await?;
    let (_, name, was_admin) = all
        .iter()
        .find(|u| u.0 == id)
        .cloned()
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such account".into()))?;
    if let Some(make_admin) = b["admin"].as_bool() {
        if !make_admin && was_admin {
            if caller.user_id().as_deref() == Some(id.as_str()) {
                return Err(conflict("you can't take away your own admin rights"));
            }
            if all.iter().filter(|u| u.2).count() <= 1 {
                return Err(conflict("the server needs at least one admin"));
            }
        }
        let i2 = id.clone();
        a.db.call(move |c| store::set_admin(c, &i2, make_admin)).await.map_err(internal)?;
        tracing::info!(user = %name, admin = make_admin, "admin rights changed");
    }
    if let Some(pw) = b["password"].as_str() {
        let (n2, pw) = (name.clone(), pw.to_string());
        a.db.call(move |c| auth::set_password(c, &n2, &pw))
            .await
            .map_err(|e| bad(format!("{e:#}")))?;
        tracing::info!(user = %name, "password changed");
    }
    a.forget_user(&id);
    let i2 = id.clone();
    let u = a.db.call(move |c| store::user(c, &i2)).await.map_err(internal)?;
    Ok(Json(json!(u)))
}

async fn delete_user(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    let (caller, a) = admin(&st, &headers, &q).await?;
    let all = users(&a).await?;
    let (_, name, is_admin) = all
        .iter()
        .find(|u| u.0 == id)
        .cloned()
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such account".into()))?;
    if caller.user_id().as_deref() == Some(id.as_str()) {
        return Err(conflict("you can't delete your own account"));
    }
    if is_admin && all.iter().filter(|u| u.2).count() <= 1 {
        return Err(conflict("the server needs at least one admin"));
    }
    let i2 = id.clone();
    a.db.call(move |c| store::delete_user(c, &i2)).await.map_err(internal)?;
    a.forget_user(&id);
    tracing::info!(user = %name, "account deleted");
    Ok(Json(json!({"deleted": id})))
}

// ---------------------------------------------------------------- devices

async fn list_devices(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    let (caller, a) = admin(&st, &headers, &q).await?;
    let mine = match &caller {
        Caller::Moonrise { token_hash, .. } => Some(token_hash.clone()),
        Caller::Admin => None,
    };
    let items =
        a.db.call(move |c| store::devices(c, &id, mine.as_deref()))
            .await
            .map_err(internal)?;
    Ok(Json(json!({"items": items, "total": items.len()})))
}

/// Revokes a user's tokens whose hash starts with `prefix` (all: None), never `keep`.
async fn revoke(a: &auth::Auth, user: &str, prefix: Option<String>, keep: Option<String>) -> Result<usize, ApiError> {
    let (u, now) = (user.to_string(), util::now());
    let hashes: Vec<String> =
        a.db.call(move |c| store::revoke(c, &u, prefix.as_deref(), keep.as_deref(), now))
            .await
            .map_err(internal)?;
    a.forget_user(user);
    Ok(hashes.len())
}

async fn revoke_device(
    State(st): State<Arc<AppState>>,
    UrlPath((id, device)): UrlPath<(String, String)>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    let (caller, a) = admin(&st, &headers, &q).await?;
    if device.len() != 16 || !device.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such device".into()));
    }
    if let Caller::Moonrise { token_hash, .. } = &caller {
        if token_hash.starts_with(&device) {
            return Err(conflict("that's this device: use Sign Out instead"));
        }
    }
    let n = revoke(&a, &id, Some(device.clone()), None).await?;
    if n == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such device".into()));
    }
    tracing::info!(user = %id, device, "device signed out by an admin");
    Ok(Json(json!({"revoked": n})))
}

async fn revoke_all_devices(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    let (caller, a) = admin(&st, &headers, &q).await?;
    let keep = match &caller {
        Caller::Moonrise { token_hash, .. } => Some(token_hash.clone()),
        Caller::Admin => None,
    };
    let n = revoke(&a, &id, None, keep).await?;
    tracing::info!(user = %id, devices = n, "devices signed out by an admin");
    Ok(Json(json!({"revoked": n})))
}

// ---------------------------------------------------------------- invitations

/// Invitation codes: 8 characters without look-alikes, shown as XXXX-XXXX.
const CODE_CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

fn new_code() -> String {
    use rand::Rng;
    let mut r = rand::thread_rng();
    (0..8).map(|_| CODE_CHARS[r.gen_range(0..CODE_CHARS.len())] as char).collect()
}

/// What a typed code is compared as: letters and digits, upper case.
fn code_norm(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

async fn create_invite(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Q,
    Json(b): Json<Value>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let (caller, a) = admin(&st, &headers, &q).await?;
    let is_admin = b["admin"].as_bool().unwrap_or(false);
    let days = b["days"].as_i64().unwrap_or(7).clamp(1, 30);
    let code = new_code();
    let (id, now) = (util::ulid().to_lowercase(), util::now());
    let expires = now + days * 86_400;
    let (i2, h, by) = (id.clone(), auth::sha256_hex(&code), caller.user_id());
    a.db.call(move |c| store::add_invite(c, &i2, &h, by.as_deref(), now, expires, is_admin))
        .await
        .map_err(internal)?;
    tracing::info!(id, admin = is_admin, days, "invitation made");
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"id": id, "code": format!("{}-{}", &code[..4], &code[4..]), "admin": is_admin, "expires_at": crate::library::rfc3339(expires)}),
        ),
    ))
}

async fn list_invites(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    let (_, a) = admin(&st, &headers, &q).await?;
    let now = util::now();
    let items = a.db.call(move |c| store::invites(c, now)).await.map_err(internal)?;
    Ok(Json(json!({"items": items, "total": items.len()})))
}

async fn delete_invite(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    let (_, a) = admin(&st, &headers, &q).await?;
    let i2 = id.clone();
    let n = a.db.call(move |c| store::delete_invite(c, &i2)).await.map_err(internal)?;
    if n == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such invitation".into()));
    }
    Ok(Json(json!({"deleted": id})))
}

async fn redeem_invite(
    State(st): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(b): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let a = accounts(&st)?.clone();
    let ip = addr.ip().to_string();
    let key = format!("invite:{ip}");
    if !a.limiter.allowed(&key, auth::LOGIN_MAX_FAILURES, auth::LOGIN_WINDOW) {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "too many wrong codes: try again in a few minutes".into(),
        ));
    }
    let code = code_norm(b["code"].as_str().unwrap_or(""));
    let name = b["username"].as_str().unwrap_or("").trim().to_string();
    let password = b["password"].as_str().unwrap_or("").to_string();
    let s = |k: &str| b[k].as_str().map(String::from);
    let (device_id, device_name) = (s("device_id"), s("device_name"));
    auth::validate_password(&password).map_err(|e| bad(format!("{e:#}")))?;
    let h = auth::sha256_hex(&code);
    let now = util::now();
    let result =
        a.db.call(move |c| store::redeem_invite(c, &h, now, &name, &password, (device_id.as_deref(), device_name.as_deref())))
            .await;
    match result {
        Ok(Ok((u, token))) => {
            tracing::info!(user = %u.name, admin = u.is_admin, "account made with an invitation");
            Ok(Json(json!({"token": token, "user": u})))
        }
        Ok(Err(store::InviteRefused::NameTaken)) => Err(conflict("that user name is taken: pick another one")),
        Ok(Err(store::InviteRefused::Invalid)) => {
            a.limiter.hit(&key);
            Err(ApiError(
                StatusCode::FORBIDDEN,
                "that invitation code isn't valid (or was used, or has expired)".into(),
            ))
        }
        Err(e) => Err(bad(format!("{e:#}"))),
    }
}

// ---------------------------------------------------------------- the server

/// Bytes in a folder (recursively; symlinks not followed).
fn dir_size(p: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else { return 0 };
    rd.filter_map(std::result::Result::ok)
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}

fn file_size(p: &Path) -> u64 {
    std::fs::metadata(p).map_or(0, |m| m.len())
}

/// The transcoding hardware, looked at once.
fn hardware(ffmpeg: &str) -> Value {
    static HW: OnceLock<Value> = OnceLock::new();
    HW.get_or_init(|| {
        let run = |cmd: &str, args: &[&str]| -> Option<String> {
            let o = std::process::Command::new(cmd).args(args).output().ok()?;
            o.status.success().then(|| String::from_utf8_lossy(&o.stdout).to_string())
        };
        let nvidia = crate::media::hwaccel::nvidia_gpus();
        // Render nodes (Intel/AMD VA-API and Quick Sync): /dev/dri/renderD*.
        let others: Vec<Value> = crate::media::hwaccel::render_nodes().into_iter().map(|n| json!({"vendor": n.vendor, "device": n.device, "usable": n.usable})).collect();
        let cpu = crate::media::hwaccel::cpu_model();
        let encoders = run(ffmpeg, &["-hide_banner", "-encoders"]).unwrap_or_default();
        let has = |e: &str| encoders.lines().any(|l| l.split_whitespace().nth(1) == Some(e));
        json!({
            "nvidia": nvidia,
            "other_gpus": others,
            "cpu": {"model": cpu, "threads": std::thread::available_parallelism().map_or(1, std::num::NonZero::get)},
            "ffmpeg": run(ffmpeg, &["-hide_banner", "-version"]).and_then(|s| s.lines().next().map(|l| l.split(" Copyright").next().unwrap_or(l).trim().to_string())),
            "encoders": {"h264_nvenc": has("h264_nvenc"), "hevc_nvenc": has("hevc_nvenc"), "h264_vaapi": has("h264_vaapi"), "hevc_vaapi": has("hevc_vaapi"), "h264_qsv": has("h264_qsv"), "hevc_qsv": has("hevc_qsv"), "h264_videotoolbox": has("h264_videotoolbox"), "hevc_videotoolbox": has("hevc_videotoolbox"), "h264_v4l2m2m": has("h264_v4l2m2m"), "libx264": has("libx264")},
        })
    })
    .clone()
}

/// Whether something restarts the server when it exits (`MOONRISE_RESTART=exit`).
fn can_restart(st: &AppState) -> bool {
    st.settings
        .env()
        .get("MOONRISE_RESTART")
        .is_some_and(|v| v.eq_ignore_ascii_case("exit"))
}

async fn system(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    auth::admin_caller(&st, &headers, &q).await?;
    let c = st.config.clone();
    let cur = st.settings.current();
    let (data, cache) = (c.data_dir.clone(), c.cache_dir.clone());
    let ffmpeg = c.ffmpeg.clone();
    let (sizes, hw) = tokio::task::spawn_blocking(move || {
        let db = ["moonrise.db", "moonrise.db-wal", "moonrise.db-shm"]
            .iter()
            .map(|f| file_size(&data.join(f)))
            .sum::<u64>();
        let sizes = json!({
            "database": db,
            "subtitles": dir_size(&data.join("subs")),
            "artwork": dir_size(&data.join("images")),
            "data": dir_size(&data),
            "cache": dir_size(&cache),
        });
        (sizes, hardware(&ffmpeg))
    })
    .await
    .map_err(|e| ApiError::internal(e.into()))?;
    let (started, at) = STARTED.get().copied().unwrap_or((Instant::now(), util::now()));
    let transcode_on = cur.enabled(crate::admin::config::Feature::Transcode);
    let backend = crate::media::hwaccel::current();
    // nvenc | qsv | vaapi | videotoolbox | v4l2 | cpu (what transcodes), off (switched off),
    // unavailable (nothing can).
    let transcoding = match (transcode_on, backend.available) {
        (false, _) => "off",
        (true, true) => backend.backend.key(),
        (true, false) => "unavailable",
    };
    let (dl_used, _) = st.downloads.usage();
    Ok(Json(json!({
        "name": st.name(),
        "version": env!("CARGO_PKG_VERSION"),
        "server_id": st.server_id,
        "started_at": crate::library::rfc3339(at),
        "uptime_secs": started.elapsed().as_secs(),
        "sizes": sizes,
        "cache_max_bytes": cur.cache_max_bytes,
        "downloads_bytes": dl_used,
        "download_max_bytes": cur.download_max_bytes,
        "hardware": hw,
        "transcoding": transcoding,
        "transcode": backend.describe(),
        "can_restart": can_restart(&st),
        "restart_required": st.settings.document()["restart_required"],
    })))
}

/// The secrets the log must never show.
fn secrets(st: &AppState) -> Vec<String> {
    let c = st.settings.current();
    let mut v = vec![
        c.tmdb_key.expose().to_string(),
        c.opensubtitles.api_key.expose().to_string(),
        c.opensubtitles.password.expose().to_string(),
        c.llm.api_key.expose().to_string(),
    ];
    v.extend(st.settings.stored().smb_credentials().values().map(|c| c.password.clone()));
    v.extend(st.admin_token.clone());
    v.retain(|s| s.len() >= 4);
    v
}

async fn logs(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    auth::admin_caller(&st, &headers, &q).await?;
    let n = q
        .get("lines")
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(200)
        .clamp(1, crate::admin::logbuf::MAX_LINES);
    let level = q.get("level").map(String::as_str).filter(|l| *l == "warn" || *l == "error");
    let lines = crate::admin::logbuf::tail(n, level, &secrets(&st));
    Ok(Json(json!({"lines": lines, "total": lines.len()})))
}

async fn activity(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    auth::admin_caller(&st, &headers, &q).await?;
    let mut sessions = st.active_sessions(Duration::from_secs(90));
    // Who and what, by name.
    if let Some(db) = st.db.clone() {
        let ids: Vec<(Option<String>, Option<String>, String)> = sessions
            .iter()
            .map(|s| {
                (
                    s["user_id"].as_str().map(String::from),
                    s["item_id"].as_str().map(String::from),
                    s["sid"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect();
        let names = db.call(move |c| store::session_names(c, ids)).await.unwrap_or_default();
        for (s, (user, item, pos)) in sessions.iter_mut().zip(names) {
            s["user"] = json!(user);
            if let Some((title, series, season, episode, kind)) = item {
                s["title"] = json!(title);
                s["series_title"] = json!(series);
                s["kind"] = json!(kind);
                if kind == "episode" {
                    s["episode_label"] = json!(format!("S{}E{}", season.unwrap_or(0), episode.unwrap_or(0)));
                }
            }
            if let Some((p, at)) = pos {
                s["position"] = json!(p);
                s["reported_at"] = json!(at.map(crate::library::rfc3339));
            }
        }
    }
    let scan = match &st.library {
        Some(l) => {
            let p = l.progress().await.unwrap_or(Value::Null);
            json!({"phase": p["phase"], "full": p["full"], "started_at": p["started_at"].as_i64().map(crate::library::rfc3339), "walked": p["walked"], "identify": p["identify"], "last_pass": p["last_pass"].clone().as_object().map(|o| {
                let mut o = o.clone();
                if let Some(at) = o.get("at").and_then(Value::as_i64) { o.insert("at".into(), json!(crate::library::rfc3339(at))); }
                Value::Object(o)
            })})
        }
        None => Value::Null,
    };
    let downloads: Vec<Value> = st
        .downloads
        .list()
        .into_iter()
        .take(20)
        .map(|j| json!({"id": j.id, "item_id": j.item_id, "label": j.label, "method": j.method, "state": j.state, "progress": j.progress, "size": j.size, "error": j.error, "created_at": j.created_at}))
        .collect();
    let errors = crate::admin::logbuf::tail(30, Some("warn"), &secrets(&st));
    Ok(Json(json!({
        "sessions": sessions,
        "transcodes": st.transcoder.running(),
        "transcode_slots": st.config.transcode_jobs,
        "subtitles": st.subs.status(),
        "index_backfill": st.index_backfill.status(),
        "trickplay": st.trickplay.status(),
        "downloads": downloads,
        "scan": scan,
        "errors": errors,
    })))
}

async fn restart(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    let caller = auth::admin_caller(&st, &headers, &q).await?;
    if !can_restart(&st) {
        return Err(conflict("this server can't restart itself (nothing would start it again): restart it where it runs, e.g. docker restart moonrise-server"));
    }
    tracing::warn!(
        by = caller.user().map_or("admin token", |u| u.name.as_str()),
        "restart asked for: exiting (the supervisor starts the server again)"
    );
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(600)).await;
        // EX_TEMPFAIL: an `on-failure` restart policy restarts it too.
        std::process::exit(75);
    });
    Ok(Json(json!({"restarting": true})))
}

async fn check_opensubtitles(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Q,
    Json(b): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    auth::admin_caller(&st, &headers, &q).await?;
    let user = b["username"].as_str().unwrap_or("").trim().to_string();
    let pass = b["password"].as_str().unwrap_or("").to_string();
    if user.is_empty() || pass.is_empty() {
        return Err(bad("username and password"));
    }
    let mut creds = st.settings.current().opensubtitles.clone();
    if !creds.usable() {
        return Err(conflict("there is no OpenSubtitles key: subtitle search is off"));
    }
    creds.username = crate::admin::config::Secret::new(&user);
    creds.password = crate::admin::config::Secret::new(&pass);
    match crate::subtitles::opensubtitles::OpenSubtitles::new(creds).check_login().await {
        Ok(v) => {
            tracing::info!("OpenSubtitles account checked: signed in");
            Ok(Json(v))
        }
        Err(e) => {
            let msg = match e.downcast_ref::<crate::subtitles::opensubtitles::Refused>() {
                Some(r) if r.status == 401 || r.status == 403 => "OpenSubtitles didn't accept that user name and password.".to_string(),
                Some(r) => format!("OpenSubtitles: {} ({})", r.message, r.status),
                None => format!("OpenSubtitles couldn't be reached: {e:#}"),
            };
            tracing::info!("OpenSubtitles account check failed");
            Err(ApiError(StatusCode::UNPROCESSABLE_ENTITY, msg))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes() {
        let c = new_code();
        assert_eq!(c.len(), 8);
        assert!(c.bytes().all(|b| CODE_CHARS.contains(&b)));
        assert_eq!(code_norm(" abcd-efgh "), "ABCDEFGH");
        assert_eq!(store::device_id("0123456789abcdef0123"), "0123456789abcdef");
    }

    fn bearer(t: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("authorization", format!("Bearer {t}").parse().unwrap());
        h
    }

    fn code_of<T>(r: Result<T, ApiError>) -> StatusCode {
        match r {
            Ok(_) => StatusCode::OK,
            Err(e) => e.0,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn users_devices_and_invites() {
        let t = std::env::temp_dir().join(format!("moonrise-manage-{}", std::process::id()));
        std::fs::create_dir_all(&t).unwrap();
        let env = crate::admin::config::Env::from_pairs([
            ("MOONRISE_DATA_DIR".to_string(), t.join("data").to_string_lossy().to_string()),
            ("MOONRISE_CACHE_DIR".to_string(), t.join("cache").to_string_lossy().to_string()),
            ("MOONRISE_DISABLE".to_string(), "mdns,library".to_string()),
        ]);
        let st = Arc::new(AppState::new(env).await.unwrap());
        let db = st.db.clone().unwrap();
        let (boss, boss_tv, kid) = db
            .call(|c| {
                let a = auth::add_user(c, "boss", "password-1", true)?;
                let u = auth::add_user(c, "kid", "password-2", false)?;
                Ok((
                    auth::issue_token(c, &a.id, Some("p"), Some("Phone"))?,
                    auth::issue_token(c, &a.id, Some("t"), Some("TV"))?,
                    auth::issue_token(c, &u.id, Some("i"), Some("iPad"))?,
                ))
            })
            .await
            .unwrap();
        let q = || Query(HashMap::new());
        let a = st.auth.clone().unwrap();
        // Admins only.
        assert_eq!(
            code_of(list_users(State(st.clone()), bearer(&kid), q()).await),
            StatusCode::FORBIDDEN
        );
        let Json(v) = list_users(State(st.clone()), bearer(&boss), q()).await.unwrap();
        assert_eq!(v["total"], 2);
        let boss_id = v["items"][0]["id"].as_str().unwrap().to_string();
        let kid_id = v["items"][1]["id"].as_str().unwrap().to_string();
        assert_eq!(
            (v["items"][0]["me"].as_bool(), v["items"][0]["devices"].as_i64()),
            (Some(true), Some(2))
        );
        // Can't delete yourself or demote the last admin.
        assert_eq!(
            code_of(delete_user(State(st.clone()), UrlPath(boss_id.clone()), bearer(&boss), q()).await),
            StatusCode::CONFLICT
        );
        assert_eq!(
            code_of(
                update_user(
                    State(st.clone()),
                    UrlPath(boss_id.clone()),
                    bearer(&boss),
                    q(),
                    Json(json!({"admin": false}))
                )
                .await
            ),
            StatusCode::CONFLICT
        );
        // New account, duplicate name, short password.
        let r = create_user(
            State(st.clone()),
            bearer(&boss),
            q(),
            Json(json!({"name": "guest", "password": "password-3"})),
        )
        .await
        .unwrap();
        assert_eq!(r.0, StatusCode::CREATED);
        assert_eq!(
            code_of(
                create_user(
                    State(st.clone()),
                    bearer(&boss),
                    q(),
                    Json(json!({"name": "Guest", "password": "password-3"}))
                )
                .await
            ),
            StatusCode::CONFLICT
        );
        assert_eq!(
            code_of(
                create_user(
                    State(st.clone()),
                    bearer(&boss),
                    q(),
                    Json(json!({"name": "x", "password": "short"}))
                )
                .await
            ),
            StatusCode::BAD_REQUEST
        );
        // Devices: list, revoke one (not this one), the token stops working at once.
        let Json(d) = list_devices(State(st.clone()), UrlPath(boss_id.clone()), bearer(&boss), q())
            .await
            .unwrap();
        assert_eq!(d["total"], 2);
        let this = d["items"].as_array().unwrap().iter().find(|x| x["current"] == true).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let tv = d["items"].as_array().unwrap().iter().find(|x| x["device_name"] == "TV").unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(!d.to_string().contains(&boss_tv[5..]), "no token in the answer");
        assert_eq!(
            code_of(revoke_device(State(st.clone()), UrlPath((boss_id.clone(), this)), bearer(&boss), q()).await),
            StatusCode::CONFLICT
        );
        assert!(a.user_for_token(&boss_tv).await.unwrap().is_some());
        let _ = revoke_device(State(st.clone()), UrlPath((boss_id.clone(), tv)), bearer(&boss), q())
            .await
            .unwrap();
        assert!(
            a.user_for_token(&boss_tv).await.unwrap().is_none(),
            "revoked at once (the cache too)"
        );
        assert!(a.user_for_token(&boss).await.unwrap().is_some());
        // Password reset, then delete: the kid's token is gone with the account.
        let _ = update_user(
            State(st.clone()),
            UrlPath(kid_id.clone()),
            bearer(&boss),
            q(),
            Json(json!({"password": "new-password-9"})),
        )
        .await
        .unwrap();
        assert!(a.check_password("kid".into(), "new-password-9".into()).await.unwrap().is_some());
        assert!(a.user_for_token(&kid).await.unwrap().is_some());
        let _ = delete_user(State(st.clone()), UrlPath(kid_id.clone()), bearer(&boss), q())
            .await
            .unwrap();
        assert!(a.user_for_token(&kid).await.unwrap().is_none());
        // Invitations: made, redeemed once, not twice.
        let (_, Json(inv)) = create_invite(State(st.clone()), bearer(&boss), q(), Json(json!({}))).await.unwrap();
        let code = inv["code"].as_str().unwrap().to_lowercase();
        let Json(l) = list_invites(State(st.clone()), bearer(&boss), q()).await.unwrap();
        assert_eq!(l["total"], 1);
        assert!(!l.to_string().contains(&code.to_uppercase()), "codes aren't listed");
        let addr: SocketAddr = "192.168.1.9:1234".parse().unwrap();
        let body = json!({"code": code, "username": "newbie", "password": "password-4", "device_name": "Mac"});
        assert_eq!(
            code_of(
                redeem_invite(
                    State(st.clone()),
                    ConnectInfo(addr),
                    Json(json!({"code": "AAAA-AAAA", "username": "x", "password": "password-4"}))
                )
                .await
            ),
            StatusCode::FORBIDDEN
        );
        let Json(r) = redeem_invite(State(st.clone()), ConnectInfo(addr), Json(body.clone()))
            .await
            .unwrap();
        assert_eq!(r["user"]["name"], "newbie");
        assert!(a.user_for_token(r["token"].as_str().unwrap()).await.unwrap().is_some());
        assert_eq!(
            code_of(
                redeem_invite(
                    State(st.clone()),
                    ConnectInfo(addr),
                    Json(json!({"code": code, "username": "other", "password": "password-4"}))
                )
                .await
            ),
            StatusCode::FORBIDDEN
        );
        // The log keeps lines, the restart is refused without a supervisor.
        assert_eq!(code_of(restart(State(st.clone()), bearer(&boss), q()).await), StatusCode::CONFLICT);
        let Json(sys) = system(State(st.clone()), bearer(&boss), q()).await.unwrap();
        assert_eq!(sys["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(sys["can_restart"], false);
        let Json(act) = activity(State(st.clone()), bearer(&boss), q()).await.unwrap();
        assert!(act["sessions"].as_array().unwrap().is_empty());
        std::fs::remove_dir_all(&t).ok();
    }
}
