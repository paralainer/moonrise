//! The admin API for the LLM step's provider (admins only), next to the `llm.*` settings of
//! `/v1/admin/settings`:
//!
//! - `GET /v1/admin/llm`: `{presets, current, chatgpt}`. `presets`: `llm/mod.rs`'s providers (id,
//!   name, style, base_url, model, output, key: required|optional|sign_in, help, local), each with
//!   `saved` when settings for it are kept from earlier (`{api_key: {set, last4?}, model?,
//!   base_url?, output?}`; `api_key`: the key kept for it at that address). `current`: provider,
//!   base_url, model, output, `output_in_use` (what `auto` settled on), api_key (masked),
//!   `keys_aside` (origins this provider has keys kept for, other than the current address),
//!   enabled. `chatgpt`: `{signed_in, email?, account?, needs_sign_in?, accounts: [{id, label,
//!   email, signed_in, active}], unfinished: [{id, kept_at}]}` (`unfinished`: registrations
//!   OpenAI issued whose sign-in never completed; no sign-in uses them).
//! - `POST /v1/admin/llm/models` `{provider?, base_url?, api_key?}`: the models that endpoint
//!   offers, `{models: [{id, name}]}` (what isn't given is taken from the settings, or what is kept
//!   for that provider).
//! - `POST /v1/admin/llm/test` `{provider?, base_url?, model?, api_key?, output?}`: one real
//!   identification of a sample file with those settings (nothing saved): `{ok, correct,
//!   verdict, output, ms, input_tokens, output_tokens}` or `{ok: false, error, problem}` (problem:
//!   `unreachable`, `auth`, `not_found`, `format`, `limit`, `other`).
//! - `POST /v1/admin/llm/chatgpt/start` `{redirect_uri, new_account?, account?, retry?, app_flow?}` →
//!   `{url}` (`account`: sign in again with that registration; default the one in use;
//!   `new_account`: a new registration; `retry`: the `state` of an attempt `finish` answered 409,
//!   to try once more with its registration). `POST /v1/admin/llm/chatgpt/finish` `{state, code,
//!   client_id?, callback_params?, app_flow?}` (`callback_params`: the names of the callback's
//!   parameters, logged; `app_flow`: the app's sign-in flow, logged; an app that sends neither
//!   doesn't try again by itself, so a refused code drops the registration at once and answers
//!   422 saying to update the app) → `{email, models}` (that registration is in use from then on;
//!   409: OpenAI refused the code, start again with `retry` = this `state`; 422: other
//!   failures), or `{state, error,
//!   error_description?}` when OpenAI sent an error back to the app instead of a code: logged,
//!   the attempt ends, 422 with the message to show. `POST /v1/admin/llm/chatgpt/sign-out`
//!   `{account?}` → `{signed_in: false, revoked, message?}` (`message` when OpenAI didn't confirm
//!   the revocation). `POST /v1/admin/llm/chatgpt/forget` `{account}` → `chatgpt` as above:
//!   removes a registration that isn't signed in (an unfinished one, say). See `chatgpt.rs`.
//!
//! The ChatGPT plan is only ever used at OpenAI's API: any other `base_url` for it is refused,
//! here and in the settings.
//!
//! **Keys go only where they were saved for** (`llm/keys.rs`): each key is bound to a provider
//! and an address, and the request layer sends it only there. Switching `llm.provider` keeps the
//! old provider's model, address and format (`llm.saved`, never answered) and brings back the
//! new one's; its key comes back with its address. Changing the address sets the key aside; it
//! comes back if the address does. A key given in a request is used for that request only. Keys
//! never appear in answers or logs, and are cut out of error messages.

use std::{collections::HashMap, sync::Arc, time::Instant};

use tracing::Instrument;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Map, Value};

use crate::{
    admin::config::{Config, Env, LlmConfig, Secret},
    admin::settings::{self, Stored},
    api::{ApiError, AppState},
    llm::keys::{self, Binding},
    llm::{self, Llm},
};

type Q = Query<HashMap<String, String>>;

/// The settings put aside per provider (keys are kept apart, bound to where they go:
/// `llm/keys.rs`).
const PER_PROVIDER: [&str; 3] = ["llm.model", "llm.base_url", "llm.output"];
pub const SAVED_KEY: &str = "llm.saved";

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/admin/llm", get(overview))
        .route("/v1/admin/llm/models", post(models))
        .route("/v1/admin/llm/test", post(test))
        .route("/v1/admin/llm/chatgpt/start", post(chatgpt_start))
        .route("/v1/admin/llm/chatgpt/finish", post(chatgpt_finish))
        .route("/v1/admin/llm/chatgpt/sign-out", post(chatgpt_sign_out))
        .route("/v1/admin/llm/chatgpt/forget", post(chatgpt_forget))
}

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

fn saved(stored: &Stored) -> Map<String, Value> {
    stored.get(SAVED_KEY).and_then(Value::as_object).cloned().unwrap_or_default()
}

/// The changes a settings PATCH makes to the LLM's settings beyond its own, decided on `stored`
/// as it is when the change is written:
/// - switching provider puts the old one's model, address and format aside and brings the new
///   one's back;
/// - a key given (`llm.api_key`) is never stored as a setting: it is kept in the vault, bound to
///   the provider and address in effect *after* the whole change (`llm/keys.rs`); null or ""
///   forgets the key bound there. Keys bound elsewhere stay where they are, unused until their
///   provider and address are in effect again.
///
/// Err: the change is refused (an address for the ChatGPT plan, which is only ever used at
/// OpenAI's; a key for it; an address no key can be bound to).
pub fn provider_changes(env: &Env, stored: &Stored, changes: &mut Vec<(String, Option<Value>)>) -> Result<(), String> {
    fn given(changes: &[(String, Option<Value>)], k: &str) -> Option<Option<Value>> {
        changes.iter().find(|(c, _)| c == k).map(|(_, v)| v.clone())
    }
    if changes
        .iter()
        .enumerate()
        .any(|(i, (k, _))| changes[..i].iter().any(|(c, _)| c == k))
    {
        return Err("a setting is given more than once".into());
    }
    let typed: Option<Option<String>> = changes
        .iter()
        .position(|(k, _)| k == "llm.api_key")
        .map(|i| changes.remove(i).1.and_then(|v| v.as_str().map(String::from)));
    let current = Config::resolve(env, stored).llm.provider;
    let new_provider =
        given(changes, "llm.provider").map(|v| v.and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "anthropic".into()));
    if let Some(p) = new_provider.filter(|p| *p != current) {
        let mut kept = saved(stored);
        let old: Map<String, Value> = PER_PROVIDER
            .iter()
            .filter_map(|k| stored.get(k).map(|v| (k.to_string(), v.clone())))
            .collect();
        if old.is_empty() {
            kept.remove(&current);
        } else {
            kept.insert(current.clone(), Value::Object(old));
        }
        let back = kept.remove(&p).and_then(|v| v.as_object().cloned()).unwrap_or_default();
        for k in PER_PROVIDER {
            if given(changes, k).is_none() {
                changes.push((k.to_string(), back.get(k).cloned()));
            }
        }
        changes.push((SAVED_KEY.into(), (!kept.is_empty()).then_some(Value::Object(kept))));
    }
    // What will be in effect once this is written.
    let after = stored.with(changes);
    let c = Config::resolve(env, &after).llm;
    let sign_in = llm::preset_or_custom(&c.provider).key == llm::KeyNeed::SignIn;
    if let Some(Some(b)) = given(changes, "llm.base_url") {
        if sign_in && !pinned_address(b.as_str().unwrap_or("")) {
            return Err(chatgpt_address());
        }
    }
    if let Some(key) = typed {
        if sign_in {
            return Err("llm.api_key: the ChatGPT plan uses a sign-in, not a key".into());
        }
        let b = Binding::new(&c.provider, &c.base_url)
            .ok_or("llm.base_url: an http:// or https:// address (without a user name) a key can be kept for")?;
        changes.push(keys::change(keys::entries(&after), &b, key));
    }
    Ok(())
}

/// Whether `base` is the ChatGPT plan's own address (the only one it is used at).
fn pinned_address(base: &str) -> bool {
    base.trim()
        .trim_end_matches('/')
        .eq_ignore_ascii_case(crate::llm::chatgpt::API_BASE)
}

fn chatgpt_address() -> String {
    format!("base_url: the ChatGPT plan is only used at {}", crate::llm::chatgpt::API_BASE)
}

/// The configuration a request asks about: the settings, with what it gives instead.
fn candidate(st: &AppState, body: &Value) -> Result<LlmConfig, ApiError> {
    let cur = st.settings.current().llm;
    let stored = st.settings.stored();
    let s = |k: &str| body[k].as_str().map(str::trim).filter(|v| !v.is_empty()).map(String::from);
    let provider = s("provider").map_or_else(|| cur.provider.clone(), |p| p.to_ascii_lowercase());
    let preset = llm::preset(&provider).ok_or_else(|| {
        bad(format!(
            "provider: one of {}",
            llm::PRESETS.iter().map(|p| p.id).collect::<Vec<_>>().join(", ")
        ))
    })?;
    let same = provider == cur.provider;
    let kept = saved(&stored)
        .get(&provider)
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let kept_s = |k: &str| kept.get(k).and_then(Value::as_str).map(String::from);
    let base_default = if same {
        cur.base_url.clone()
    } else {
        kept_s("llm.base_url").unwrap_or_else(|| preset.base_url.to_string())
    };
    let base_url = s("base_url").unwrap_or_else(|| base_default.clone());
    // The ChatGPT plan's token only ever goes to OpenAI.
    if preset.key == llm::KeyNeed::SignIn {
        if !pinned_address(&base_url) && s("base_url").is_some() {
            return Err(bad(chatgpt_address()));
        }
        return Ok(LlmConfig {
            provider,
            style: preset.style.into(),
            base_url: crate::llm::chatgpt::API_BASE.into(),
            model: s("model").unwrap_or_else(|| {
                if same {
                    cur.model.clone()
                } else {
                    kept_s("llm.model").unwrap_or_default()
                }
            }),
            output: checked_output(s("output").unwrap_or_else(|| {
                if same {
                    cur.output.clone()
                } else {
                    kept_s("llm.output").unwrap_or_else(|| preset.output.to_string())
                }
            }))?,
            api_key: Secret::default(),
            key_for: None,
            enabled: true,
            ..cur
        });
    }
    if !(base_url.starts_with("http://") || base_url.starts_with("https://")) || keys::origin_of(&base_url).is_none() {
        return Err(bad("base_url: an http:// or https:// URL, without a user name or password"));
    }
    let model = s("model").unwrap_or_else(|| {
        if same {
            cur.model.clone()
        } else {
            kept_s("llm.model").unwrap_or_else(|| preset.model.to_string())
        }
    });
    let output = checked_output(s("output").unwrap_or_else(|| {
        if same {
            cur.output.clone()
        } else {
            kept_s("llm.output").unwrap_or_else(|| preset.output.to_string())
        }
    }))?;
    // A key typed here is for this request, bound to its provider and address; otherwise the key
    // bound to this provider at this address, if there is one. The request layer checks the
    // binding again on every request (`Llm`), as for the extra headers.
    let (api_key, key_for) = match s("api_key") {
        Some(k) => (Secret::new(k), Binding::new(&provider, &base_url)),
        None if cur.key_for.as_ref().is_some_and(|b| b.allows_str(&provider, &base_url)) => (cur.api_key.clone(), cur.key_for.clone()),
        None => match keys::find(&stored, &provider, &base_url) {
            Some((k, b)) => (k, Some(b)),
            None => (Secret::default(), None),
        },
    };
    Ok(LlmConfig {
        provider,
        style: preset.style.into(),
        base_url,
        model,
        output,
        api_key,
        key_for,
        enabled: true,
        ..cur
    })
}

fn checked_output(o: String) -> Result<String, ApiError> {
    let o = o.to_ascii_lowercase();
    if llm::OUTPUTS.contains(&o.as_str()) {
        Ok(o)
    } else {
        Err(bad(format!("output: one of {}", llm::OUTPUTS.join(", "))))
    }
}

async fn overview(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    crate::auth::admin_caller(&st, &headers, &q).await?;
    let c = st.settings.current();
    let stored = st.settings.stored();
    let kept = saved(&stored);
    let mut presets = llm::presets_json();
    for p in presets.as_array_mut().unwrap() {
        let id = p["id"].as_str().unwrap_or("").to_string();
        let k = kept.get(&id).and_then(Value::as_object).cloned().unwrap_or_default();
        let mut s = json!({});
        for (from, to) in [("llm.model", "model"), ("llm.base_url", "base_url"), ("llm.output", "output")] {
            if let Some(v) = k.get(from) {
                s[to] = v.clone();
            }
        }
        // The key that comes back with it: the one bound to its address.
        let base = k
            .get("llm.base_url")
            .and_then(Value::as_str)
            .unwrap_or(p["base_url"].as_str().unwrap_or(""));
        if id != c.llm.provider {
            if let Some((key, _)) = keys::find(&stored, &id, base) {
                s["api_key"] = settings::mask("llm.api_key", &json!(key.expose()));
            }
        }
        if s.as_object().is_some_and(|o| !o.is_empty()) {
            p["saved"] = s;
        }
    }
    // Keys of this provider kept for other addresses (they come back with their address).
    let current_origin = keys::origin_of(&c.llm.base_url);
    let aside: Vec<String> = keys::for_provider(&stored, &c.llm.provider)
        .into_iter()
        .map(|e| e.binding.origin)
        .filter(|o| Some(o) != current_origin.as_ref())
        .collect();
    let in_use = st.library.as_ref().and_then(|l| l.llm()).map(|l| l.output());
    Ok(Json(json!({
        "presets": presets,
        "current": {
            "provider": c.llm.provider, "base_url": c.llm.base_url, "model": c.llm.model, "output": c.llm.output,
            "output_in_use": in_use, "api_key": settings::mask("llm.api_key", &json!(c.llm.api_key.expose())),
            "keys_aside": aside,
            "enabled": c.llm.enabled && c.enabled(crate::admin::config::Feature::Llm),
        },
        "chatgpt": crate::llm::chatgpt::get().status().await,
    })))
}

async fn models(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Q,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    crate::auth::admin_caller(&st, &headers, &q).await?;
    let l = Llm::new(candidate(&st, &body)?);
    match l.models().await {
        Ok(m) => Ok(Json(
            json!({"models": m.into_iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>()}),
        )),
        Err(e) => Err(ApiError(StatusCode::BAD_GATEWAY, l.scrub(&format!("{e:#}")))),
    }
}

/// A file whose name needs the LLM (transliterated), and TMDB's candidates for it: the right
/// answer is 331.
pub fn sample() -> (Value, i64) {
    (
        json!({
            "looks_like": "movie", "because": "year",
            "path": "Movies/Park.Yurskogo.Perioda.3.2001.BDRip.1080p.mkv",
            "parsed": {"title": "Park Yurskogo Perioda 3", "year": 2001, "alt_titles": [], "series_folder": null, "season": null, "episode": null},
            "duration_minutes": 92,
            "file": {"video": "h264 1920x1040", "audio": ["ac3 rus"]},
            "candidates": [
                {"id": 331, "title": "Jurassic Park III", "original_title": "Jurassic Park III", "year": 2001, "runtime_minutes": 92,
                 "overview": "In need of funds for research, Dr. Alan Grant accepts a large sum of money to accompany a wealthy couple on an aerial tour of the infamous Isla Sorna."},
                {"id": 329, "title": "Jurassic Park", "original_title": "Jurassic Park", "year": 1993, "runtime_minutes": 127,
                 "overview": "A wealthy entrepreneur secretly creates a theme park featuring living dinosaurs drawn from prehistoric DNA."},
                {"id": 330, "title": "The Lost World: Jurassic Park", "original_title": "The Lost World: Jurassic Park", "year": 1997, "runtime_minutes": 129,
                 "overview": "Four years after Jurassic Park's genetically bred dinosaurs ran amok, multimillionaire John Hammond shocks chaos theorist Ian Malcolm."}
            ]
        }),
        331,
    )
}

/// What kind of failure an error message describes (for the app's wording).
fn problem(e: &anyhow::Error) -> &'static str {
    let text = format!("{e:#}").to_ascii_lowercase();
    if e.downcast_ref::<llm::NoVerdict>().is_some() {
        return "format";
    }
    if let Some(h) = e.downcast_ref::<llm::HttpError>() {
        return match h.status {
            401 | 403 => "auth",
            404 => "not_found",
            400 | 422 if text.contains("api key") || text.contains("api_key") => "auth",
            400 | 422 => "format",
            _ => "other",
        };
    }
    if text.contains("request failed") || text.contains("unreachable") {
        "unreachable"
    } else if text.contains("limit") {
        "limit"
    } else if text.contains("sign in") || text.contains("signed in") {
        "auth"
    } else if text.contains("no answer format worked") {
        "format"
    } else {
        "other"
    }
}

async fn test(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    crate::auth::admin_caller(&st, &headers, &q).await?;
    let config = candidate(&st, &body)?;
    if config.model.is_empty() && config.provider != "llamacpp" {
        return Ok(Json(json!({"ok": false, "problem": "other", "error": "choose a model first"})));
    }
    let l = Llm::new(config);
    let (input, right) = sample();
    let t0 = Instant::now();
    let r = l
        .classify(
            crate::library::identify::SYSTEM,
            &input.to_string(),
            &crate::library::identify::verdict_schema(),
        )
        .await;
    let ms = t0.elapsed().as_millis() as u64;
    Ok(Json(match r {
        Ok(v) => {
            tracing::info!(llm = l.describe(), ms, "llm test");
            json!({"ok": true, "correct": v.value["tmdb_id"].as_i64() == Some(right), "verdict": v.value, "output": v.output, "ms": ms,
                   "input_tokens": v.input_tokens, "output_tokens": v.output_tokens})
        }
        Err(e) => {
            let error = l.scrub(&format!("{e:#}"));
            tracing::info!(llm = l.describe(), ms, error, "llm test failed");
            json!({"ok": false, "problem": problem(&e), "error": error, "ms": ms})
        }
    }))
}

/// For the log: which app's sign-in this is. Apps send `app_flow` (a number, raised when the
/// app's side of the sign-in changes); apps for server 0.22.2 sent `callback_params` instead,
/// older apps neither (they never try again by themselves after a refused code).
fn app_flow(body: &Value) -> String {
    match body["app_flow"].as_u64() {
        Some(n) => format!("flow {n}"),
        // Only the 0.22.2 apps report an error OpenAI sent back, or the callback's names.
        None if body["callback_params"].as_array().is_some_and(|a| !a.is_empty()) || body.get("error").is_some() => "0.22.2 app".into(),
        // `start` bodies: the 0.22.2 apps and older ones look alike here.
        None if body.get("redirect_uri").is_some() => "no app_flow (an app for server 0.22.2 or older)".into(),
        None => "older than 0.22.2 (doesn't try again)".into(),
    }
}

async fn chatgpt_start(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Q,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    crate::auth::admin_caller(&st, &headers, &q).await?;
    let redirect = body["redirect_uri"].as_str().unwrap_or("");
    let new_account = body["new_account"].as_bool().unwrap_or(false);
    let app = app_flow(&body);
    let url = crate::llm::chatgpt::get()
        .start(redirect, new_account, body["account"].as_str(), body["retry"].as_str())
        .instrument(tracing::info_span!("chatgpt", app))
        .await
        .map_err(|e| {
            tracing::warn!(app, "ChatGPT sign-in not started: {e:#}");
            bad(format!("{e:#}"))
        })?;
    Ok(Json(json!({"url": url})))
}

/// Re-reads the store (the sign-in changed) and applies it.
async fn reapply(st: &Arc<AppState>) -> Result<(), ApiError> {
    let (before, after) = st
        .settings
        .update(Vec::new())
        .await
        .map_err(|e| ApiError(StatusCode::SERVICE_UNAVAILABLE, format!("{e:#}")))?;
    crate::admin::apply(st, &before, &after).await;
    Ok(())
}

async fn chatgpt_finish(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Q,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    crate::auth::admin_caller(&st, &headers, &q).await?;
    let (state, code) = (body["state"].as_str().unwrap_or(""), body["code"].as_str().unwrap_or(""));
    // The names of the parameters the browser brought back (never their values), from apps that
    // send them: what OpenAI's answer looked like.
    let names: Vec<&str> = body["callback_params"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|n| n.len() <= 40)
        .take(20)
        .collect();
    if let (Some(error), false) = (body["error"].as_str(), state.is_empty()) {
        // OpenAI sent the browser back with an error (access_denied, invalid_scope, …).
        let message = crate::llm::chatgpt::get()
            .callback_error(state, error, body["error_description"].as_str().unwrap_or(""))
            .instrument(tracing::info_span!("chatgpt", app = app_flow(&body)))
            .await;
        return Err(ApiError(StatusCode::UNPROCESSABLE_ENTITY, message));
    }
    if state.is_empty() || code.is_empty() {
        tracing::warn!(callback_params = ?names, state = !state.is_empty(), code = !code.is_empty(), "ChatGPT sign-in callback incomplete");
        return Err(bad("state and code"));
    }
    if !names.is_empty() {
        tracing::info!(callback_params = ?names, "ChatGPT sign-in callback received by the app");
    }
    // Apps that try again by themselves after a refused code say which sign-in flow they run
    // (`app_flow`); the first ones (server 0.22.2) didn't, but sent `callback_params`.
    let app = app_flow(&body);
    let app_retries = body["app_flow"].as_u64().is_some() || !names.is_empty();
    let t = crate::llm::chatgpt::get()
        .finish_from(state, code, body["client_id"].as_str(), app_retries)
        .instrument(tracing::info_span!("chatgpt", app))
        .await
        .map_err(|e| {
            let text = crate::llm::chatgpt::scrub(&format!("{e:#}"));
            // A refused code: 409, and the app signs in once more (the registration is kept).
            let refused = e.downcast_ref::<crate::llm::chatgpt::CodeRefused>().is_some();
            tracing::warn!(refused, app, "ChatGPT sign-in failed: {text}");
            ApiError(
                if refused {
                    StatusCode::CONFLICT
                } else {
                    StatusCode::UNPROCESSABLE_ENTITY
                },
                text,
            )
        })?;
    reapply(&st).await?;
    // The account's models, for the app to pick from.
    let mut c = candidate(&st, &json!({"provider": "chatgpt"}))?;
    c.model = String::new();
    let models = Llm::new(c).models().await.unwrap_or_default();
    Ok(Json(
        json!({"email": t.email, "models": models.into_iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>()}),
    ))
}

async fn chatgpt_sign_out(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Q,
    body: axum::body::Bytes,
) -> Result<Json<Value>, ApiError> {
    crate::auth::admin_caller(&st, &headers, &q).await?;
    // The body is optional: `{account?}`.
    let account = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|b| b["account"].as_str().map(String::from));
    let out = crate::llm::chatgpt::get()
        .sign_out(account.as_deref())
        .await
        .map_err(|e| ApiError(StatusCode::SERVICE_UNAVAILABLE, format!("{e:#}")))?;
    reapply(&st).await?;
    let mut answer = json!({"signed_in": false, "revoked": out.revoked});
    if !out.revoked {
        answer["message"] = json!(
            "Signed out here, but OpenAI didn't confirm that it ended the sign-in. To be sure, disconnect Moonrise in ChatGPT's settings."
        );
    }
    Ok(Json(answer))
}

async fn chatgpt_forget(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Q,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    crate::auth::admin_caller(&st, &headers, &q).await?;
    let account = body["account"].as_str().unwrap_or("");
    crate::llm::chatgpt::get()
        .forget(account)
        .await
        .map_err(|e| ApiError(StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}")))?;
    reapply(&st).await?;
    Ok(Json(crate::llm::chatgpt::get().status().await))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn which_app_signs_in() {
        assert_eq!(app_flow(&json!({"state": "s", "code": "c", "app_flow": 2})), "flow 2");
        assert_eq!(app_flow(&json!({"redirect_uri": "x", "app_flow": 2})), "flow 2");
        assert_eq!(
            app_flow(&json!({"state": "s", "code": "c", "callback_params": ["code", "state"]})),
            "0.22.2 app"
        );
        assert_eq!(app_flow(&json!({"state": "s", "error": "access_denied"})), "0.22.2 app");
        assert_eq!(
            app_flow(&json!({"state": "s", "code": "c"})),
            "older than 0.22.2 (doesn't try again)"
        );
        assert!(app_flow(&json!({"redirect_uri": "x"})).starts_with("no app_flow"));
    }

    /// The store after a settings PATCH (`patch`, in that order) on `stored`.
    fn apply_env(env: &Env, stored: &Stored, patch: &[(&str, Option<Value>)]) -> Result<Stored, String> {
        let mut changes: Vec<(String, Option<Value>)> = patch.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        provider_changes(env, stored, &mut changes)?;
        Ok(stored.with(&changes))
    }

    fn apply(stored: &Stored, patch: &[(&str, Option<Value>)]) -> Stored {
        apply_env(&Env::default(), stored, patch).unwrap()
    }

    /// The key the configuration in effect would use, and where the request layer sends it.
    fn key(s: &Stored) -> (String, Option<Binding>) {
        let c = Config::resolve(&Env::default(), s).llm;
        (c.api_key.expose().to_string(), c.key_for)
    }

    #[test]
    fn keys_stay_where_they_were_saved_for() {
        let j = |v: &str| Some(json!(v));
        // A key given is bound to the provider and address in effect after the change.
        let s = apply(&Stored::default(), &[("llm.api_key", j("sk-ant")), ("llm.model", j("claude-x"))]);
        assert!(s.get("llm.api_key").is_none(), "never stored as a setting");
        assert_eq!(key(&s), ("sk-ant".into(), Binding::new("anthropic", "https://api.anthropic.com")));
        // Anthropic → OpenRouter: Anthropic's settings are put aside, OpenRouter has no key.
        let s = apply(&s, &[("llm.provider", j("openrouter"))]);
        assert_eq!(key(&s).0, "");
        assert_eq!(s.get(SAVED_KEY), Some(&json!({"anthropic": {"llm.model": "claude-x"}})));
        // …and back: its model, and its key with its address.
        let s = apply(&s, &[("llm.provider", j("anthropic"))]);
        assert_eq!(key(&s).0, "sk-ant");
        assert_eq!(Config::resolve(&Env::default(), &s).llm.model, "claude-x");
        // A key typed with a switch is the new provider's; the old one's stays.
        let s = apply(&s, &[("llm.provider", j("openai")), ("llm.api_key", j("sk-oa"))]);
        assert_eq!(key(&s), ("sk-oa".into(), Binding::new("openai", "https://api.openai.com/v1")));
        assert_eq!(keys::entries(&s).len(), 2);
        // Another address: the key is set aside (bound to the old one), not moved; it comes back
        // with its address; another path at the same origin keeps it.
        let s = apply(
            &s,
            &[
                ("llm.provider", j("custom")),
                ("llm.base_url", j("http://a:1/v1")),
                ("llm.api_key", j("kc")),
                ("llm.model", j("m")),
            ],
        );
        assert_eq!(key(&s).0, "kc");
        let s = apply(&s, &[("llm.base_url", j("http://b:1/v1"))]);
        assert_eq!(key(&s), (String::new(), None));
        let s = apply(&s, &[("llm.base_url", j("http://a:1/v2"))]);
        assert_eq!(key(&s).0, "kc");
        // A key with a new address: bound to that address; the other stays bound to its own.
        let s = apply(&s, &[("llm.api_key", j("kb")), ("llm.base_url", j("http://b:1/v1"))]);
        assert_eq!(key(&s).0, "kb");
        let s = apply(&s, &[("llm.base_url", j("http://a:1/v1"))]);
        assert_eq!(key(&s).0, "kc");
        // The order of the keys in a change doesn't matter: the binding is the result's.
        let s2 = apply(&s, &[("llm.base_url", j("http://c:1/v1")), ("llm.api_key", j("kx"))]);
        let s3 = apply(&s, &[("llm.api_key", j("kx")), ("llm.base_url", j("http://c:1/v1"))]);
        assert_eq!(keys::entries(&s2), keys::entries(&s3));
        assert_eq!(key(&s2), ("kx".into(), Binding::new("custom", "http://c:1")));
        // null forgets the key bound to the address in effect, only that one.
        let s = apply(&s, &[("llm.api_key", None)]);
        assert_eq!(key(&s).0, "");
        assert!(keys::find(&s, "custom", "http://b:1").is_some());
        // A restored provider with a new address in the same change: its key isn't sent there.
        let s = apply(&s, &[("llm.provider", j("anthropic"))]);
        let s = apply(&s, &[("llm.provider", j("custom")), ("llm.base_url", j("http://evil:1/v1"))]);
        assert_eq!(key(&s).0, "");
        // Nothing about keys: no key change.
        let s4 = apply(&s, &[("llm.model", j("m2"))]);
        assert_eq!(s4.get(keys::VAULT_KEY), s.get(keys::VAULT_KEY));
        // A setting given twice is refused (whatever the order would make of it).
        let mut ch = vec![
            ("llm.base_url".to_string(), j("https://api.anthropic.com")),
            ("llm.base_url".to_string(), j("http://catcher:1")),
        ];
        assert!(provider_changes(&Env::default(), &s, &mut ch).is_err());
        // A key needs an address it can be bound to.
        assert!(apply_env(
            &Env::default(),
            &Stored::default(),
            &[
                ("llm.provider", j("custom")),
                ("llm.base_url", j("http://u:p@gw:1/v1")),
                ("llm.api_key", j("k"))
            ]
        )
        .is_err());
    }

    /// With the provider and address set in the environment, a key given in the settings is
    /// bound to those: the configuration in effect, not what the store says.
    #[test]
    fn keys_bind_to_the_configuration_in_effect() {
        let env = Env::from_pairs([
            ("MOONRISE_LLM_PROVIDER", "custom"),
            ("MOONRISE_LLM_BASE_URL", "http://gw:1/v1"),
            ("MOONRISE_LLM_MODEL", "m"),
        ]);
        let s = apply_env(
            &env,
            &Stored::default(),
            &[
                ("llm.base_url", Some(json!("http://catcher:1/v1"))),
                ("llm.api_key", Some(json!("k"))),
            ],
        )
        .unwrap();
        assert_eq!(keys::entries(&s)[0].binding, Binding::new("custom", "http://gw:1").unwrap());
        let c = Config::resolve(&env, &s).llm;
        assert_eq!((c.api_key.expose(), c.base_url.as_str()), ("k", "http://gw:1/v1"));
    }

    /// Through the API: switching providers keeps keys apart, the overview never shows one, and
    /// the test runs a real call against a stand-in OpenAI-compatible server.
    #[tokio::test(flavor = "multi_thread")]
    async fn api_switch_and_test() {
        use crate::admin::tests::{bearer, server};
        let auth_seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let a2 = auth_seen.clone();
        let app = Router::new()
            .route(
                "/v1/chat/completions",
                post(move |h: HeaderMap, Json(b): Json<Value>| {
                    let seen = a2.clone();
                    async move {
                        seen.lock().unwrap().push(h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("").to_string());
                        assert!(b["messages"][1]["content"].as_str().unwrap().contains("Park.Yurskogo"));
                        Json(json!({"choices": [{"finish_reason": "stop", "message": {"content": "{\"kind\":\"movie\",\"tmdb_id\":331,\"season\":null,\"episode\":null,\"episode_end\":null,\"suggested_title\":null,\"suggested_year\":null,\"confidence\":0.9,\"reason\":\"r\"}"}}], "usage": {"prompt_tokens": 10, "completion_tokens": 5}}))
                    }
                }),
            )
            .route("/v1/models", get(|| async { Json(json!({"data": [{"id": "local-model"}]})) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let t = std::env::temp_dir().join(format!("moonrise-llm-admin-{}", std::process::id()));
        let (st, admin, user) = server(&t, &[]).await;
        let q = || Query(HashMap::new());
        assert!(overview(State(st.clone()), bearer(&user), q()).await.is_err(), "admins only");
        // An Anthropic key, then a local server: the key is put aside, never sent there.
        crate::admin::tests::patch(&st, &admin, json!({"llm": {"api_key": "sk-ant-secret-0123456789"}})).await;
        let base = format!("http://{addr}/v1");
        crate::admin::tests::patch(&st, &admin, json!({"llm.provider": "llamacpp", "llm.base_url": base})).await;
        assert_eq!(st.settings.current().llm.api_key.expose(), "");
        let Json(o) = overview(State(st.clone()), bearer(&admin), q()).await.unwrap();
        assert_eq!(o["current"]["provider"], "llamacpp");
        assert_eq!(o["current"]["enabled"], true);
        let anth = o["presets"].as_array().unwrap().iter().find(|p| p["id"] == "anthropic").unwrap();
        assert_eq!(anth["saved"]["api_key"]["last4"], "6789");
        assert!(!o.to_string().contains("sk-ant-secret"), "keys never answered");
        let Json(m) = models(State(st.clone()), bearer(&admin), q(), Json(json!({}))).await.unwrap();
        assert_eq!(m["models"][0]["id"], "local-model");
        let Json(r) = test(State(st.clone()), bearer(&admin), q(), Json(json!({}))).await.unwrap();
        assert_eq!(
            (r["ok"].as_bool(), r["correct"].as_bool(), r["output"].as_str()),
            (Some(true), Some(true), Some("schema"))
        );
        // Testing another provider at this address: Anthropic's saved key isn't sent here.
        let Json(r) = test(
            State(st.clone()),
            bearer(&admin),
            q(),
            Json(json!({"provider": "custom", "base_url": base, "model": "x"})),
        )
        .await
        .unwrap();
        assert_eq!(r["ok"], true);
        assert!(
            auth_seen.lock().unwrap().iter().all(String::is_empty),
            "no key went to the local server"
        );
        // A key typed for the test is used for it only, and kept out of errors.
        let Json(r) = test(
            State(st.clone()),
            bearer(&admin),
            q(),
            Json(json!({"provider": "custom", "base_url": "http://127.0.0.1:9/v1", "model": "x", "api_key": "typed-key-0123456789"})),
        )
        .await
        .unwrap();
        assert_eq!((r["ok"].as_bool(), r["problem"].as_str()), (Some(false), Some("unreachable")));
        // Back to Anthropic: its key returns.
        crate::admin::tests::patch(&st, &admin, json!({"llm.provider": "anthropic"})).await;
        let c = st.settings.current().llm;
        assert_eq!(
            (c.api_key.expose(), c.base_url.as_str()),
            ("sk-ant-secret-0123456789", "https://api.anthropic.com")
        );
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn chatgpt_address_is_fixed() {
        let try_apply = |current: &str, patch: &[(&str, Option<Value>)]| {
            let stored = Stored::from_pairs([("llm.provider", json!(current))]);
            apply_env(&Env::default(), &stored, patch)
        };
        assert!(try_apply("chatgpt", &[("llm.base_url", Some(json!("http://catcher:1/v1")))]).is_err());
        assert!(try_apply(
            "anthropic",
            &[
                ("llm.provider", Some(json!("chatgpt"))),
                ("llm.base_url", Some(json!("http://catcher:1/v1")))
            ]
        )
        .is_err());
        assert!(try_apply("chatgpt", &[("llm.base_url", Some(json!("https://api.openai.com/v1/")))]).is_ok());
        assert!(try_apply("chatgpt", &[("llm.base_url", None)]).is_ok());
        assert!(try_apply(
            "chatgpt",
            &[
                ("llm.provider", Some(json!("custom"))),
                ("llm.base_url", Some(json!("http://catcher:1/v1")))
            ]
        )
        .is_ok());
        assert!(
            try_apply("chatgpt", &[("llm.api_key", Some(json!("k")))]).is_err(),
            "the plan has no key"
        );
    }

    /// The ChatGPT plan's token never goes to another address: the test, the model list and the
    /// settings refuse one, a stored one is ignored, and nothing reaches it.
    #[tokio::test(flavor = "multi_thread")]
    async fn chatgpt_token_stays_with_openai() {
        use crate::admin::tests::{bearer, server, try_patch};
        let hits = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let h2 = hits.clone();
        let catcher = Router::new().fallback(move |h: HeaderMap, uri: axum::http::Uri| {
            let hits = h2.clone();
            async move {
                hits.lock().unwrap().push(format!(
                    "{uri} {}",
                    h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("")
                ));
                "caught"
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, catcher).await.unwrap() });
        let base = format!("http://{addr}/v1");

        let t = std::env::temp_dir().join(format!("moonrise-llm-chatgpt-{}", std::process::id()));
        let (st, admin, _) = server(&t, &[]).await;
        let q = || Query(HashMap::new());
        let body = json!({"provider": "chatgpt", "base_url": base, "model": "m"});
        let e = test(State(st.clone()), bearer(&admin), q(), Json(body.clone())).await.unwrap_err();
        assert_eq!(e.0, StatusCode::BAD_REQUEST);
        assert!(e.1.contains("only used at https://api.openai.com/v1"));
        assert_eq!(
            models(State(st.clone()), bearer(&admin), q(), Json(body)).await.unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            try_patch(&st, &admin, json!({"llm.provider": "chatgpt", "llm.base_url": base})).await,
            StatusCode::BAD_REQUEST
        );
        crate::admin::tests::patch(&st, &admin, json!({"llm.provider": "chatgpt", "llm.model": "m"})).await;
        assert_eq!(try_patch(&st, &admin, json!({"llm.base_url": base})).await, StatusCode::BAD_REQUEST);
        // Even an address stored behind the API's back is ignored.
        st.settings.update(vec![("llm.base_url".into(), Some(json!(base)))]).await.unwrap();
        assert_eq!(st.settings.current().llm.base_url, crate::llm::chatgpt::API_BASE);
        let c = candidate(&st, &json!({})).unwrap();
        assert_eq!(c.base_url, crate::llm::chatgpt::API_BASE);
        let l = Llm::new(LlmConfig {
            base_url: base.clone(),
            ..c
        });
        let Json(r) = test(State(st.clone()), bearer(&admin), q(), Json(json!({}))).await.unwrap();
        assert_eq!(r["ok"], false, "not signed in here");
        assert!(l.models().await.is_err());
        assert!(l.classify("s", "i", &json!({"type": "object"})).await.is_err());
        assert!(
            hits.lock().unwrap().is_empty(),
            "nothing reached the other address: {:?}",
            hits.lock().unwrap()
        );
        std::fs::remove_dir_all(&t).ok();
    }

    /// Every way a stored key could be pointed elsewhere, through the API: settings changes
    /// (nested and flat keys together, either order), test and model-list overrides, the
    /// identification itself, redirects from the key's own address. A catcher must receive no
    /// key or header.
    #[tokio::test(flavor = "multi_thread")]
    async fn no_key_reaches_a_catcher() {
        use crate::admin::tests::{bearer, patch, server, try_patch};
        use crate::llm::tests::recorder;
        let (catcher, caught) = recorder(None).await;
        let (a, seen_a) = recorder(None).await;
        let (r, seen_r) = recorder(Some((307, catcher.clone()))).await;
        let (a1, b1, r1) = (format!("{a}/v1"), format!("{catcher}/v1"), format!("{r}/v1"));
        let t = std::env::temp_dir().join(format!("moonrise-llm-catcher-{}", std::process::id()));
        let (st, admin, _) = server(&t, &[]).await;
        let q = || Query(HashMap::new());
        // Every way a request goes out: the test, the model list, and identification with the
        // configuration in effect.
        let all_paths = |body: Value| {
            let st = st.clone();
            let admin = admin.clone();
            async move {
                let _ = test(State(st.clone()), bearer(&admin), q(), Json(body.clone())).await;
                let _ = models(State(st.clone()), bearer(&admin), q(), Json(body)).await;
                let c = st.settings.current().llm;
                let _ = Llm::new(c).classify("s", "i", &json!({"type": "object"})).await;
            }
        };
        let key_seen = |seen: &std::sync::Arc<std::sync::Mutex<Vec<String>>>, key: &str| {
            seen.lock().unwrap().drain(..).filter(|g| g.contains(key)).count()
        };
        patch(
            &st,
            &admin,
            json!({"llm.provider": "custom", "llm.base_url": a1, "llm.model": "m", "llm.api_key": "stored-key-0123456789"}),
        )
        .await;
        all_paths(json!({})).await;
        assert_eq!(key_seen(&seen_a, "stored-key-0123456789"), 3, "the key goes to its own address");

        // The same setting twice, nested and flat, either order: refused, nothing changed.
        for body in [
            json!({"llm": {"base_url": a1}, "llm.base_url": b1}),
            json!({"llm.base_url": b1, "llm": {"base_url": a1}}),
            json!({"llm": {"base_url": b1, "api_key": "x-0123456789abcdef"}, "llm.base_url": a1}), // gitleaks:allow (a made-up test key)
            json!({"llm": {"provider": "anthropic"}, "llm.provider": "custom", "llm.base_url": b1}),
        ] {
            assert_eq!(try_patch(&st, &admin, body).await, StatusCode::BAD_REQUEST);
        }
        // The same key twice in the JSON text itself.
        for raw in [
            r#"{"llm.base_url": "http://127.0.0.1:9/v1", "llm.base_url": "http://127.0.0.1:8/v1"}"#,
            r#"{"llm": {"base_url": "http://127.0.0.1:9/v1", "base_url": "http://127.0.0.1:8/v1"}}"#,
        ] {
            assert_eq!(crate::admin::tests::try_patch_raw(&st, &admin, raw).await, StatusCode::BAD_REQUEST);
        }
        assert_eq!(st.settings.current().llm.base_url, a1);
        // Another address: the key stays bound to its own.
        patch(&st, &admin, json!({"llm.base_url": b1})).await;
        all_paths(json!({})).await;
        patch(&st, &admin, json!({"llm": {"base_url": b1}})).await;
        all_paths(json!({})).await;
        // Switching away and back with the catcher's address.
        patch(&st, &admin, json!({"llm.provider": "anthropic"})).await;
        patch(&st, &admin, json!({"llm.provider": "custom", "llm.base_url": b1})).await;
        all_paths(json!({})).await;
        patch(&st, &admin, json!({"llm.provider": "llamacpp", "llm": {"base_url": b1}})).await;
        all_paths(json!({})).await;
        // Overrides in the test and the model list.
        patch(&st, &admin, json!({"llm.provider": "custom", "llm.base_url": a1})).await;
        for body in [
            json!({"base_url": b1}),
            json!({"provider": "custom", "base_url": b1}),
            json!({"provider": "openrouter", "base_url": b1}),
            json!({"provider": "anthropic", "base_url": catcher}),
            json!({"base_url": format!("http://user@{}/v1", catcher.trim_start_matches("http://"))}),
            json!({"base_url": format!("{a1}@{}", catcher.trim_start_matches("http://"))}),
        ] {
            all_paths(body).await;
        }
        // Back at its address: the key is back.
        seen_a.lock().unwrap().clear();
        all_paths(json!({})).await;
        assert_eq!(key_seen(&seen_a, "stored-key-0123456789"), 3, "restored with its address");

        // Redirects from the key's own address, both API styles: not followed.
        patch(&st, &admin, json!({"llm.base_url": r1, "llm.api_key": "redirect-key-0123456789"})).await;
        all_paths(json!({})).await;
        assert_eq!(key_seen(&seen_r, "redirect-key-0123456789"), 3);
        let Json(res) = test(State(st.clone()), bearer(&admin), q(), Json(json!({}))).await.unwrap();
        assert!(res["error"].as_str().unwrap().contains("doesn't follow redirects"), "{res}");
        patch(
            &st,
            &admin,
            json!({"llm.provider": "anthropic", "llm.base_url": r, "llm.api_key": "ant-key-0123456789"}),
        )
        .await;
        all_paths(json!({})).await;
        assert_eq!(key_seen(&seen_r, "ant-key-0123456789"), 3);
        patch(&st, &admin, json!({"llm": {"base_url": catcher}})).await;
        all_paths(json!({})).await;

        let caught: Vec<String> = caught.lock().unwrap().clone();
        assert!(!caught.is_empty(), "the catcher was asked");
        assert!(
            caught.iter().all(|g| g.ends_with(' ')),
            "no key or header reached the catcher: {caught:?}"
        );
        std::fs::remove_dir_all(&t).ok();
    }

    /// The same with a key and extra headers from the environment.
    #[tokio::test(flavor = "multi_thread")]
    async fn no_env_credential_reaches_a_catcher() {
        use crate::admin::tests::{bearer, patch, server};
        use crate::llm::tests::recorder;
        let (catcher, caught) = recorder(None).await;
        let (r, seen_r) = recorder(Some((308, catcher.clone()))).await;
        let r1 = format!("{r}/v1");
        let t = std::env::temp_dir().join(format!("moonrise-llm-env-catcher-{}", std::process::id()));
        let (st, admin, _) = server(
            &t,
            &[
                ("MOONRISE_LLM_PROVIDER", "custom"),
                ("MOONRISE_LLM_BASE_URL", &r1),
                ("MOONRISE_LLM_MODEL", "m"),
                ("MOONRISE_LLM_API_KEY", "env-key-0123456789"),
                ("MOONRISE_LLM_EXTRA_HEADERS", "X-Org: org-secret-1"),
            ],
        )
        .await;
        let q = || Query(HashMap::new());
        let b1 = format!("{catcher}/v1");
        for body in [
            json!({}),
            json!({"base_url": b1}),
            json!({"provider": "openrouter", "base_url": b1}),
            json!({"provider": "anthropic", "base_url": catcher}),
        ] {
            let _ = test(State(st.clone()), bearer(&admin), q(), Json(body.clone())).await;
            let _ = models(State(st.clone()), bearer(&admin), q(), Json(body)).await;
        }
        assert!(seen_r
            .lock()
            .unwrap()
            .iter()
            .any(|g| g.contains("env-key-0123456789") && g.contains("org-secret-1")));
        patch(&st, &admin, json!({"llm": {"base_url": b1}, "llm.provider": "llamacpp"})).await;
        let _ = Llm::new(st.settings.current().llm)
            .classify("s", "i", &json!({"type": "object"}))
            .await;
        let caught: Vec<String> = caught.lock().unwrap().clone();
        assert!(!caught.is_empty());
        assert!(caught.iter().all(|g| g.ends_with(' ')), "{caught:?}");
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn problems() {
        assert_eq!(
            problem(
                &llm::HttpError {
                    status: 401,
                    body: String::new()
                }
                .into()
            ),
            "auth"
        );
        assert_eq!(
            problem(
                &llm::HttpError {
                    status: 400,
                    body: "API key not valid".into()
                }
                .into()
            ),
            "auth"
        );
        assert_eq!(
            problem(
                &llm::HttpError {
                    status: 404,
                    body: String::new()
                }
                .into()
            ),
            "not_found"
        );
        assert_eq!(problem(&anyhow::anyhow!("llm request failed (connection)")), "unreachable");
        assert_eq!(problem(&llm::NoVerdict("x".into()).into()), "format");
    }
}
