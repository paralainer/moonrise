//! Watch-state endpoints (Phase 3). Only Moonrise tokens have watch state.
//!
//! - `POST /v1/sessions/{sid}/start|progress|stop {position, paused, audio, subtitle, credits, tracks}`:
//!   `sid` is the playback session (`session` in the playback answer, the id in the stream URLs).
//!   `credits`: where the end credits the app offers start (seconds), if it has any. `tracks`:
//!   the audio and subtitle tracks playing, kept for the file (`tracks.rs`).
//!   Answers `{recorded, outcome, user}`.
//! - `POST /v1/sessions/local {reports: [{viewing, item, event, position, at, duration, credits, tracks, file}]}`:
//!   playback of a copy on the device (a download), reported when the server can be reached,
//!   possibly long after (queued while offline). `viewing` is the device's id for one playback
//!   (it counts as played once), `event` start|progress|stop, `at` when it happened (Unix
//!   seconds). A report older than the title's last change (another device, a mark) is skipped.
//!   Answers `{results: [{item, recorded, outcome|reason}]}`.
//! - `PUT /v1/items/{id}/played`, `DELETE /v1/items/{id}/played`: mark played or unplayed (a
//!   series or season: all its episodes). Answers `{changed, user}`.
//! - `PUT /v1/items/{id}/version {file_id}`, `DELETE /v1/items/{id}/version`: the version (one
//!   of the title's files) this user plays, remembered for every device (`user.version` in the
//!   item's answer; playback without a `version` plays it). Answers `{version}`.
//! - `GET /v1/shows/{id}/continue`: what the show's Play button plays: `{item, reason:
//!   "resume"|"next"|"first"}`, or `{item: null}` when everything has been watched.
//! - `DELETE /v1/continue/{id}`: Remove from Continue Watching (`/v1/home` `continue`) for every
//!   device of the user: the item's show, film collection or film stays out of the row until it
//!   is watched again. Answers `{hidden: true, thread}`.
//! - `GET /v1/search/start` (capability `search-start`): what Search offers before
//!   anything is typed, `{recent, suggestions}` (`search_start.rs`). `PUT /v1/search/recent/{id}`:
//!   a title was opened from Search (`{recorded: true}`). `DELETE /v1/search/recent`: forget them
//!   (`{cleared}`, how many). All of it for the signed-in user, the same on every device.

use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::store;
use crate::{
    api::{ApiError, AppState},
    auth::{self, Caller},
    library::{api as lapi, util},
};

type Q = Query<HashMap<String, String>>;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/sessions/local", post(report_local))
        .route("/v1/sessions/{sid}/{event}", post(report))
        .route("/v1/items/{id}/played", put(mark_played).delete(mark_unplayed))
        .route("/v1/items/{id}/version", put(choose_version).delete(forget_version))
        .route("/v1/shows/{id}/continue", get(continue_show))
        .route("/v1/continue/{id}", axum::routing::delete(hide_continue))
        .route("/v1/search/start", get(search_start))
        .route("/v1/search/recent", axum::routing::delete(clear_search_recent))
        .route("/v1/search/recent/{id}", put(record_search_recent))
}

fn internal(e: anyhow::Error) -> ApiError {
    ApiError::internal(e)
}

#[derive(Deserialize, Default)]
struct ReportBody {
    /// Seconds into the title.
    position: Option<f64>,
    #[serde(default)]
    paused: bool,
    /// Where the app's end credits start (seconds): the credits it offers. With it a film is
    /// played from there, not at 90 % (`super::report_credits`).
    credits: Option<f64>,
    #[allow(dead_code, reason = "sent by the apps; accepted, not used (`tracks` says more)")]
    audio: Option<i64>,
    #[allow(dead_code, reason = "sent by the apps (a subtitle track); accepted, not used (`tracks` says more)")]
    subtitle: Option<i64>,
    /// The audio and subtitle tracks playing (`super::tracks`): kept for the file.
    tracks: Option<super::tracks::Choice>,
}

async fn report(
    State(st): State<Arc<AppState>>,
    Path((sid, event)): Path<(String, String)>,
    headers: HeaderMap,
    Query(q): Q,
    body: axum::body::Bytes,
) -> Result<Json<Value>, ApiError> {
    if !matches!(event.as_str(), "start" | "progress" | "stop") {
        return Err(ApiError(StatusCode::NOT_FOUND, "start, progress or stop".into()));
    }
    let who = auth::caller(&st, &headers, &q).await?;
    // The body is optional (a start needs none); parsed after the credentials are checked.
    let b: ReportBody = if body.iter().all(u8::is_ascii_whitespace) {
        ReportBody::default()
    } else {
        serde_json::from_slice(&body).map_err(|e| ApiError(StatusCode::BAD_REQUEST, format!("body: {e}")))?
    };
    let Caller::Moonrise { user, .. } = &who else {
        // The admin token: accepted, not recorded.
        return Ok(Json(
            json!({"recorded": false, "reason": "only Moonrise accounts have watch state"}),
        ));
    };
    let Some(db) = st.db.clone().filter(|_| st.auth.is_some()) else {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Moonrise accounts are switched off".into(),
        ));
    };
    let position = b.position.filter(|p| p.is_finite() && *p >= 0.0);
    let uid = user.id.clone();
    let live = st.session_for_report(&sid);
    let report = store::SessionReport {
        session: sid.clone(),
        event: event.clone(),
        position,
        credits: b.credits,
        tracks: b.tracks,
        now: util::now(),
    };
    let r = db
        .call(move |c| store::session_report(c, &uid, live, &report))
        .await
        .map_err(internal)?;
    match r {
        Ok((item, outcome, state)) => {
            if event != "progress" {
                tracing::info!(session = %sid.chars().take(6).collect::<String>(), item = %item, event, position = ?position, paused = b.paused, outcome = ?outcome, user = %user.name, "playback report");
            }
            let outcome = outcome.map(store::outcome_name);
            Ok(Json(json!({"recorded": true, "item": item, "outcome": outcome, "user": state})))
        }
        Err(store::ReportRefused::UnknownSession) => Err(ApiError(StatusCode::NOT_FOUND, "unknown session".into())),
        Err(store::ReportRefused::NotYours) => Err(ApiError(StatusCode::FORBIDDEN, "not your session".into())),
        Err(store::ReportRefused::NotInLibrary) => Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "the title isn't in the Moonrise library".into(),
        )),
    }
}

#[derive(Deserialize)]
struct LocalBody {
    reports: Vec<store::LocalReport>,
}

/// Most reports one request may carry.
const LOCAL_MAX: usize = 500;

async fn report_local(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Q,
    body: axum::body::Bytes,
) -> Result<Json<Value>, ApiError> {
    let who = auth::caller(&st, &headers, &q).await?;
    let b: LocalBody = serde_json::from_slice(&body).map_err(|e| ApiError(StatusCode::BAD_REQUEST, format!("body: {e}")))?;
    let Caller::Moonrise { user, .. } = &who else {
        return Ok(Json(json!({"results": [], "reason": "only Moonrise accounts have watch state"})));
    };
    let Some(db) = st.db.clone().filter(|_| st.auth.is_some()) else {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Moonrise accounts are switched off".into(),
        ));
    };
    if b.reports.len() > LOCAL_MAX {
        return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, format!("at most {LOCAL_MAX} reports")));
    }
    // The current id of each title (ids sent long ago may have been merged since).
    let mut resolved = Vec::with_capacity(b.reports.len());
    for r in &b.reports {
        resolved.push(moonrise_item(&st, &r.item).await.ok());
    }
    let now = util::now();
    let (uid, name) = (user.id.clone(), user.name.clone());
    let reports = b.reports.into_iter().zip(resolved).collect();
    let results = db
        .call(move |c| store::local_reports(c, &uid, reports, now))
        .await
        .map_err(internal)?;
    let recorded = results.iter().filter(|r| r["recorded"] == true).count();
    tracing::info!(user = %name, reports = results.len(), recorded, "local playback reports");
    Ok(Json(json!({"results": results})))
}

async fn moonrise_item(st: &AppState, id: &str) -> Result<String, ApiError> {
    let l = lapi::lib(st)?;
    lapi::resolve_id(l, id)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such item".into()))
}

async fn set_played(
    st: Arc<AppState>,
    id: String,
    headers: HeaderMap,
    q: HashMap<String, String>,
    played: bool,
) -> Result<Json<Value>, ApiError> {
    let (user, _) = auth::moonrise_caller(&st, &headers, &q).await?;
    let rid = moonrise_item(&st, &id).await?;
    let l = lapi::lib(&st)?;
    let (uid, r2) = (user.id.clone(), rid.clone());
    let changed =
        l.db.call(move |c| super::set_played(c, &uid, &r2, played, util::now()))
            .await
            .map_err(internal)?;
    let Some(changed) = changed else {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such item".into()));
    };
    tracing::info!(item = %rid, played, changed, user = %user.name, "marked");
    let item = lapi::items_by_ids(l, Some(user.id.clone()), vec![rid]).await?;
    Ok(Json(json!({"changed": changed, "user": item.first().map(|v| v["user"].clone())})))
}

async fn mark_played(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    set_played(st, id, headers, q, true).await
}

async fn mark_unplayed(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    set_played(st, id, headers, q, false).await
}

async fn continue_show(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    let (user, _) = auth::moonrise_caller(&st, &headers, &q).await?;
    let sid = moonrise_item(&st, &id).await?;
    let l = lapi::lib(&st)?;
    let uid = user.id.clone();
    let found = l.db.call(move |c| super::continue_show(c, &uid, &sid)).await.map_err(internal)?;
    match found {
        Some((ep, reason)) => {
            let item = lapi::items_by_ids(l, Some(user.id.clone()), vec![ep]).await?;
            Ok(Json(json!({"item": item.into_iter().next(), "reason": reason})))
        }
        None => Ok(Json(json!({"item": null, "reason": null}))),
    }
}

async fn hide_continue(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    let (user, _) = auth::moonrise_caller(&st, &headers, &q).await?;
    let rid = moonrise_item(&st, &id).await?;
    let l = lapi::lib(&st)?;
    let (uid, r2) = (user.id.clone(), rid.clone());
    let thread =
        l.db.call(move |c| super::continuing::hide(c, &uid, &r2, util::now()))
            .await
            .map_err(internal)?;
    let Some(thread) = thread else {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such item".into()));
    };
    tracing::info!(item = %rid, thread = %thread, user = %user.name, "removed from continue watching");
    Ok(Json(json!({"hidden": true, "thread": thread})))
}

async fn search_start(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    let who = auth::caller(&st, &headers, &q).await?;
    let user = lapi::viewer(&st, &who);
    let l = lapi::lib(&st)?;
    let (recent, suggestions) =
        l.db.call(move |c| {
            let recent = super::search_start::recent(c, user.as_deref())?;
            let suggestions = super::search_start::suggestions(c, user.as_deref(), &recent)?;
            Ok((recent, suggestions))
        })
        .await
        .map_err(internal)?;
    Ok(Json(json!({"recent": recent, "suggestions": suggestions})))
}

async fn record_search_recent(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    let (user, _) = auth::moonrise_caller(&st, &headers, &q).await?;
    let rid = moonrise_item(&st, id.split('~').next().unwrap_or(&id)).await?;
    let l = lapi::lib(&st)?;
    let uid = user.id.clone();
    let recorded =
        l.db.call(move |c| super::search_start::record(c, &uid, &rid, util::now()))
            .await
            .map_err(internal)?;
    if !recorded {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such item".into()));
    }
    Ok(Json(json!({"recorded": true})))
}

async fn clear_search_recent(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    let (user, _) = auth::moonrise_caller(&st, &headers, &q).await?;
    let l = lapi::lib(&st)?;
    let uid = user.id.clone();
    let cleared = l.db.call(move |c| super::search_start::clear(c, &uid)).await.map_err(internal)?;
    Ok(Json(json!({"cleared": cleared})))
}

#[derive(Deserialize)]
struct VersionBody {
    file_id: String,
}

async fn choose_version(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
    Json(b): Json<VersionBody>,
) -> Result<Json<Value>, ApiError> {
    let (user, _) = auth::moonrise_caller(&st, &headers, &q).await?;
    let rid = moonrise_item(&st, id.split('~').next().unwrap_or(&id)).await?;
    let l = lapi::lib(&st)?;
    let (uid, r2, fid) = (user.id.clone(), rid.clone(), b.file_id.clone());
    let ok =
        l.db.call(move |c| store::choose_version(c, &uid, &r2, &fid))
            .await
            .map_err(internal)?;
    if !ok {
        return Err(ApiError(StatusCode::NOT_FOUND, "not a version of this title".into()));
    }
    tracing::info!(item = %rid, file = %b.file_id, user = %user.name, "version chosen");
    Ok(Json(json!({"version": b.file_id})))
}

async fn forget_version(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    let (user, _) = auth::moonrise_caller(&st, &headers, &q).await?;
    let rid = moonrise_item(&st, id.split('~').next().unwrap_or(&id)).await?;
    let l = lapi::lib(&st)?;
    let (uid, r2) = (user.id.clone(), rid.clone());
    l.db.call(move |c| store::forget_version(c, &uid, &r2)).await.map_err(internal)?;
    Ok(Json(json!({"version": null})))
}
