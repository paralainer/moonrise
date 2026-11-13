//! The admin API for the server's settings (admins only: a Moonrise admin's token, or the CLI's
//! local admin token). Settings live in the database (`settings.rs`); environment variables
//! still win over them (`config.rs`).
//!
//! - `GET /v1/admin/settings`: `{settings, fields, libraries, restart_required}`. `settings` is
//!   nested (`server.name`, `metadata.language|country`, `tmdb.api_key`, `opensubtitles.*`,
//!   `llm.*`, `performance.*`, `features.*`); a secret is shown as `{set, last4?}` (the last 4
//!   characters of an API key, nothing of a password; a built-in key `{set, builtin}`). `fields["<key>"]`: `source` (`env`,
//!   `file`, `store`, `builtin` or `default`), `env` (the variable that sets it), `apply` (`live`,
//!   `restart` or `env-only`), `restart_pending` (stored, but the server still runs with the
//!   old value).
//! - `PATCH /v1/admin/settings`: the same shape, partial, nested or with dotted keys
//!   (`{"metadata.country": "GB"}`); `null` or `""` removes a value (back to the default). A
//!   secret's masked form given back is ignored. Answers the document plus `changed`,
//!   `overridden_by_env` (stored, but an environment variable wins) and `restart_required`.
//!   Applied at once where possible: TMDB/OpenSubtitles/LLM clients take new keys, scan
//!   intervals, the server's name (also re-announced on Bonjour), most feature switches.
//! - `GET|POST /v1/admin/libraries`, `GET|PATCH|DELETE /v1/admin/libraries/{id}`:
//!   `{name, kind: movies|shows, roots: [...]}`, a root being `"/path"`, `{source: "local", path}`
//!   or `{source: "smb", host, share, path, credential_id}` (instead of `credential_id`:
//!   `username`, `password`, `domain` — stored as a credential, reused when identical; neither:
//!   guest; or `url: "smb://host/share/path"`). Every root is checked before saving (a share is
//!   signed in to and its folder listed: it must exist and hold something). A change rewrites
//!   the library tables and starts a full scan; removing a library removes its titles (and
//!   watch state for them stays unused). Roots answer `available`, and after a scan pass
//!   `state` (`online`|`offline`), `error`, `since`. 409 while `MOONRISE_LIBRARIES` is set.
//! - SMB (the app's "Network share" flow; passwords only in request bodies, never answered):
//!   `GET /v1/admin/smb/discover[?wait_ms=]` file servers on the LAN (Bonjour `_smb._tcp`,
//!   NetBIOS); `POST /v1/admin/smb/shares {host, credential_id | username, password, domain}`
//!   its shares; `POST /v1/admin/smb/browse {host, share, path?, …credentials}` a folder's
//!   sub-folders and video count; `POST /v1/admin/smb/test {host, share, path, …}` a root as it
//!   would be saved (time, entries). Failures answer `{error, problem, detail}` with `problem`
//!   one of `unreachable` (502), `auth_required`, `wrong_password`, `access_denied` (422),
//!   `not_found` (404), `other` (502). `GET|POST /v1/admin/smb/credentials`,
//!   `PATCH|DELETE /v1/admin/smb/credentials/{id}` (`{host?, username, password, domain?}`; the
//!   password shown as `{set}`; a credential in use can't be deleted: 409).
//! - The LLM's provider (presets, models, a test, Sign in with ChatGPT): `/v1/admin/llm…`, see
//!   `llm/admin.rs`.
//! - `GET /v1/admin/fs/browse[?path=]`: the server's folders, for picking a library root:
//!   without `path`, the roots that may be browsed; with one, its sub-folders and how many video
//!   files it holds directly. Only below the browsable roots (`MOONRISE_BROWSE_ROOTS`, comma-separated;
//!   default: every top-level folder but the system's and the server's own data and cache; on macOS
//!   the user's Movies and home folders, /Users/Shared and every volume in /Volumes);
//!   symlinks are resolved first. Names only, never file contents.

pub mod claim;
pub mod config;
pub mod logbuf;
pub mod manage;
pub mod services;
pub mod settings;
pub(crate) mod store;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{
    extract::{Path as UrlPath, Query, State},
    http::{HeaderMap, StatusCode},
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};

use std::collections::BTreeMap;

use axum::response::{IntoResponse, Response};

use crate::{
    admin::config::{library_id, Config, Feature},
    admin::settings::{LibrarySetting, Root, SmbCredential},
    api::{ApiError, AppState},
    storage::smb::{self, Creds, SmbPath},
};

type Q = Query<HashMap<String, String>>;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/admin/settings", get(get_settings).patch(patch_settings))
        .route("/v1/admin/libraries", get(list_libraries).post(create_library))
        .route(
            "/v1/admin/libraries/{id}",
            get(get_library).patch(update_library).delete(delete_library),
        )
        .route("/v1/admin/fs/browse", get(browse))
        .route("/v1/admin/smb/discover", get(smb_discover))
        .route("/v1/admin/smb/shares", axum::routing::post(smb_shares))
        .route("/v1/admin/smb/browse", axum::routing::post(smb_browse))
        .route("/v1/admin/smb/test", axum::routing::post(smb_test))
        .route("/v1/admin/smb/credentials", get(list_credentials).post(create_credential))
        .route(
            "/v1/admin/smb/credentials/{id}",
            axum::routing::patch(update_credential).delete(delete_credential),
        )
        .merge(crate::llm::admin::routes())
}

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

async fn get_settings(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    crate::auth::admin_caller(&st, &headers, &q).await?;
    Ok(Json(st.settings.document()))
}

async fn patch_settings(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Q,
    raw: axum::body::Bytes,
) -> Result<Json<Value>, ApiError> {
    crate::auth::admin_caller(&st, &headers, &q).await?;
    // Read strictly: a key named twice is refused, not resolved by order.
    let body = settings::parse_strict(&raw).map_err(bad)?;
    if !body.is_object() {
        return Err(bad("a JSON object of settings"));
    }
    let mut changes: Vec<(String, Option<Value>)> = Vec::new();
    let mut errors = Vec::new();
    for (k, v) in settings::flatten(&body) {
        if k == "libraries" || k.starts_with("libraries.") {
            errors.push("libraries: use /v1/admin/libraries".to_string());
            continue;
        }
        // One value per setting: `{"llm": {"base_url": …}, "llm.base_url": …}` is refused, not
        // resolved by order.
        if changes.iter().any(|(c, _)| c == &k) {
            errors.push(format!("{k}: given more than once"));
            continue;
        }
        match settings::validate(&k, &v) {
            Ok(v) => changes.push((k, v)),
            Err(e) => errors.push(e),
        }
    }
    if !errors.is_empty() {
        return Err(bad(errors.join("; ")));
    }
    if changes.is_empty() {
        return Err(bad("nothing to change"));
    }
    let keys: Vec<String> = changes.iter().map(|(k, _)| k.clone()).collect();
    // A provider's model and address stay with it, and a key given is bound to the provider and
    // address in effect after the change (`llm/admin.rs`, `llm/keys.rs`): decided on the store
    // as it is when written.
    let env = st.settings.env().clone();
    let (before, after) = st
        .settings
        .update_with(move |stored| {
            let mut changes = changes;
            crate::llm::admin::provider_changes(&env, stored, &mut changes)?;
            Ok(changes)
        })
        .await
        .map_err(|e| match e {
            settings::UpdateError::Refused(m) => bad(m),
            settings::UpdateError::Failed(e) => ApiError(StatusCode::SERVICE_UNAVAILABLE, format!("{e:#}")),
        })?;
    apply(&st, &before, &after).await;
    let stored = st.settings.stored();
    let env_won: Vec<&String> = keys
        .iter()
        .filter(|k| matches!(settings::source(st.settings.env(), &stored, k).0, "env" | "file"))
        .collect();
    let mut doc = st.settings.document();
    doc["changed"] = json!(settings::changed_keys(&before, &after));
    doc["overridden_by_env"] = json!(env_won);
    tracing::info!(keys = ?keys, changed = ?doc["changed"], restart_required = ?doc["restart_required"], "settings changed");
    Ok(Json(doc))
}

/// Puts a new configuration into effect where that can be done while running.
pub async fn apply(st: &Arc<AppState>, before: &Config, after: &Config) {
    // Switches that apply live follow the settings; the others keep what the server started with.
    let mut off = st.config.disabled.get();
    for f in Feature::ALL.into_iter().filter(|f| f.live() && !f.env_only()) {
        if after.enabled(f) {
            off.remove(f.key());
        } else {
            off.insert(f.key().to_string());
        }
    }
    st.config.disabled.set(off);
    if before.opensubtitles != after.opensubtitles {
        st.opensubtitles.set_creds(after.opensubtitles.clone()).await;
        tracing::info!(configured = st.opensubtitles.configured(), "OpenSubtitles credentials applied");
    }
    crate::storage::smb::client().configure(&after.smb_shares, after.smb_signing == "always");
    if let Some(l) = &st.library {
        l.reconfigure(after);
    }
    if before.name != after.name || before.enabled(Feature::Mdns) != after.enabled(Feature::Mdns) {
        let st = st.clone();
        let _ = tokio::task::spawn_blocking(move || st.announce()).await;
    }
}

// ---------------------------------------------------------------- libraries

fn env_managed(st: &AppState) -> Result<(), ApiError> {
    if st.settings.libraries_from_env() {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "the libraries are set by MOONRISE_LIBRARIES in the server's environment: remove it there to manage them here".into(),
        ));
    }
    Ok(())
}

fn root_states(st: &AppState) -> Option<HashMap<String, crate::library::RootState>> {
    st.library.as_ref().map(|l| l.root_states())
}

fn libraries_doc(st: &AppState) -> Value {
    let stored = st.settings.stored();
    let states = root_states(st);
    let items: Vec<Value> = st
        .settings
        .current()
        .libraries
        .iter()
        .map(|l| settings::library_json(l, &stored, states.as_ref()))
        .collect();
    json!({"items": items, "total": items.len(), "managed_by": if st.settings.libraries_from_env() { "env" } else { "store" }})
}

async fn list_libraries(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    crate::auth::admin_caller(&st, &headers, &q).await?;
    Ok(Json(libraries_doc(&st)))
}

async fn get_library(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    crate::auth::admin_caller(&st, &headers, &q).await?;
    let stored = st.settings.stored();
    let l = st
        .settings
        .current()
        .libraries
        .into_iter()
        .find(|l| l.id == id)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such library".into()))?;
    Ok(Json(settings::library_json(&l, &stored, root_states(&st).as_ref())))
}

/// The libraries as stored now (what a change starts from).
fn stored_libraries(st: &AppState) -> Vec<LibrarySetting> {
    let stored = st.settings.stored();
    if stored.has_libraries() {
        stored.libraries()
    } else {
        st.settings.current().libraries.iter().map(LibrarySetting::from_runtime).collect()
    }
}

fn parse_name(v: &Value) -> Result<String, ApiError> {
    let name = v.as_str().map_or("", str::trim);
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(bad("name: 1 to 64 characters"));
    }
    Ok(name.to_string())
}

/// Credentials named in a request: `credential_id` (stored), else `username`/`password`/`domain`
/// (no user name: guest). Answers the credential's id when it is a stored one.
fn creds_in(o: &serde_json::Map<String, Value>, stored: &BTreeMap<String, SmbCredential>) -> Result<(Creds, Option<String>), ApiError> {
    if let Some(id) = o.get("credential_id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        let c = stored.get(id).ok_or_else(|| bad(format!("no credential {id}")))?;
        return Ok((c.creds(), Some(id.to_string())));
    }
    let s = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let creds = Creds {
        username: s("username").trim().to_string(),
        password: s("password"),
        domain: s("domain").trim().to_string(),
    };
    if creds.username.chars().any(char::is_control) || creds.username.len() > 256 || creds.password.len() > 1024 || creds.domain.len() > 256
    {
        return Err(bad("username, password, domain: too long or control characters"));
    }
    Ok((creds, None))
}

/// The share a request names (`host`, `share`, `path`, or `url`).
fn smb_target(o: &serde_json::Map<String, Value>) -> Result<SmbPath, ApiError> {
    if let Some(u) = o.get("url").and_then(Value::as_str) {
        return SmbPath::parse(u.trim()).ok_or_else(|| bad(format!("{u}: an smb://host/share/folder URL")));
    }
    let host = smb::check_host(o.get("host").and_then(Value::as_str).unwrap_or("")).map_err(|e| bad(e.to_string()))?;
    let share = smb::check_share(o.get("share").and_then(Value::as_str).unwrap_or("")).map_err(|e| bad(e.to_string()))?;
    let path = o.get("path").and_then(Value::as_str).unwrap_or("");
    SmbPath::new(&host, &share, path).ok_or_else(|| bad(format!("{path}: a folder in the share")))
}

/// Stores `creds` as a credential (or finds the identical one); answers its id.
fn remember_creds(creds: &Creds, host: &str, stored: &mut BTreeMap<String, SmbCredential>) -> String {
    if let Some((id, _)) = stored
        .iter()
        .find(|(_, c)| c.username == creds.username && c.password == creds.password && c.domain == creds.domain)
    {
        return id.clone();
    }
    let id = format!("cred-{}", crate::library::util::ulid().to_lowercase());
    stored.insert(
        id.clone(),
        SmbCredential {
            host: Some(host.to_string()),
            username: creds.username.clone(),
            password: creds.password.clone(),
            domain: creds.domain.clone(),
        },
    );
    id
}

fn same_root(a: &str, b: &str) -> bool {
    if smb::is_smb(a) || smb::is_smb(b) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

fn overlaps(a: &str, b: &str) -> bool {
    let under = |x: &str, y: &str| {
        let y = format!("{y}/");
        x.len() > y.len() && same_root(&x[..y.len()], &y)
    };
    same_root(a, b) || under(a, b) || under(b, a)
}

/// `["/path", {source, path, …}]` → roots, each checked: a local one an existing, non-empty
/// folder the server may browse; an SMB one signed in to and listed (non-empty). None may be
/// inside or around another library's root. New SMB credentials land in `creds`.
async fn parse_roots(
    st: &AppState,
    v: &Value,
    others: &[LibrarySetting],
    creds: &mut BTreeMap<String, SmbCredential>,
) -> Result<Vec<Root>, ApiError> {
    let list = v
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or_else(|| bad("roots: a list of folders"))?;
    let mut roots: Vec<Root> = Vec::new();
    for r in list {
        let root = match r {
            Value::String(p) if smb::is_smb(p) => {
                let p = SmbPath::parse(p).ok_or_else(|| bad(format!("{p}: an smb://host/share/folder URL")))?;
                smb_root(st, &p, Creds::guest(), None, creds).await?
            }
            Value::String(p) => local_root(st, p)?,
            Value::Object(o) => {
                match o
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or(if o.contains_key("share") || o.contains_key("url") {
                        "smb"
                    } else {
                        "local"
                    }) {
                    "local" => local_root(
                        st,
                        o.get("path")
                            .and_then(Value::as_str)
                            .ok_or_else(|| bad("roots: each needs a path"))?,
                    )?,
                    "smb" => {
                        let p = smb_target(o)?;
                        let (c, id) = creds_in(o, creds)?;
                        smb_root(st, &p, c, id, creds).await?
                    }
                    other => return Err(bad(format!("roots: source {other} isn't supported (local or smb)"))),
                }
            }
            _ => return Err(bad("roots: folder paths")),
        };
        let p = root.runtime_path().unwrap_or_default();
        if let Some(o) = others
            .iter()
            .find(|o| o.roots.iter().filter_map(Root::runtime_path).any(|x| overlaps(&p, &x)))
        {
            return Err(bad(format!("{p} overlaps a folder of the library {}", o.name)));
        }
        if roots.iter().filter_map(Root::runtime_path).any(|x| overlaps(&p, &x)) {
            return Err(bad(format!("{p} is given twice (or inside another root)")));
        }
        roots.push(root);
    }
    Ok(roots)
}

fn local_root(st: &AppState, path: &str) -> Result<Root, ApiError> {
    let canon = check_dir(&st.config, path).map_err(bad)?;
    if std::fs::read_dir(&canon).map_or(true, |mut d| d.next().is_none()) {
        return Err(bad(format!("{path}: the folder is empty or unreadable")));
    }
    Ok(Root::local(&canon.to_string_lossy()))
}

async fn smb_root(
    st: &AppState,
    p: &SmbPath,
    c: Creds,
    id: Option<String>,
    stored: &mut BTreeMap<String, SmbCredential>,
) -> Result<Root, ApiError> {
    let sign = st.settings.current().smb_signing == "always";
    let (entries, _) = smb::probe_root(p, &c, sign)
        .await
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, format!("{}: {e:#}", p.url())))?;
    if entries.is_empty() {
        return Err(bad(format!("{}: the folder is empty", p.url())));
    }
    let id = match id {
        Some(id) => Some(id),
        None if c.is_guest() => None,
        None => Some(remember_creds(&c, &p.host, stored)),
    };
    Ok(Root::smb(p, id))
}

async fn save_libraries(
    st: &Arc<AppState>,
    libs: &[LibrarySetting],
    creds: Option<&BTreeMap<String, SmbCredential>>,
) -> Result<(), ApiError> {
    let v = serde_json::to_value(libs).map_err(|e| ApiError::internal(e.into()))?;
    let mut changes = vec![("libraries".to_string(), Some(v))];
    if let Some(c) = creds {
        if *c != st.settings.stored().smb_credentials() {
            changes.push((
                "smb.credentials".into(),
                Some(serde_json::to_value(c).map_err(|e| ApiError::internal(e.into()))?),
            ));
        }
    }
    let (before, after) = st
        .settings
        .update(changes)
        .await
        .map_err(|e| ApiError(StatusCode::SERVICE_UNAVAILABLE, format!("{e:#}")))?;
    apply(st, &before, &after).await;
    Ok(())
}

fn scan_note(st: &AppState) -> &'static str {
    if st.library.is_some() {
        "started"
    } else {
        "the library is switched off: applied when it is on"
    }
}

async fn create_library(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Q,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    crate::auth::admin_caller(&st, &headers, &q).await?;
    env_managed(&st)?;
    let name = parse_name(&body["name"])?;
    // Libraries have no type; a kind sent by older apps is kept for them.
    let kind = match body["kind"].as_str().map(str::to_ascii_lowercase) {
        None => "mixed".to_string(),
        Some(k) if ["movies", "shows", "mixed"].contains(&k.as_str()) => k,
        Some(_) => return Err(bad("kind: movies, shows or mixed (or none: libraries have no type)")),
    };
    let mut libs = stored_libraries(&st);
    let mut creds = st.settings.stored().smb_credentials();
    let roots = parse_roots(&st, &body["roots"], &libs, &mut creds).await?;
    let base = Some(library_id(&name))
        .filter(|i| i.chars().any(char::is_alphanumeric))
        .unwrap_or_else(|| "library".into());
    let mut id = base.clone();
    let mut n = 2;
    while libs.iter().any(|l| l.id == id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    let lib = LibrarySetting {
        id: id.clone(),
        name,
        kind,
        roots,
    };
    libs.push(lib.clone());
    save_libraries(&st, &libs, Some(&creds)).await?;
    tracing::info!(id, "library added");
    let mut v = settings::library_json(&lib.runtime(), &st.settings.stored(), None);
    v["scan"] = json!(scan_note(&st));
    Ok((StatusCode::CREATED, Json(v)))
}

async fn update_library(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    Query(q): Q,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    crate::auth::admin_caller(&st, &headers, &q).await?;
    env_managed(&st)?;
    let mut libs = stored_libraries(&st);
    let i = libs
        .iter()
        .position(|l| l.id == id)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such library".into()))?;
    // A kind (older apps send the one it has) is accepted and kept; it changes nothing else.
    if let Some(k) = body.get("kind").and_then(Value::as_str).map(str::to_ascii_lowercase) {
        if !["movies", "shows", "mixed"].contains(&k.as_str()) {
            return Err(bad("kind: movies, shows or mixed"));
        }
        libs[i].kind = k;
    }
    if let Some(n) = body.get("name") {
        libs[i].name = parse_name(n)?;
    }
    let mut creds = st.settings.stored().smb_credentials();
    if let Some(r) = body.get("roots") {
        let others: Vec<LibrarySetting> = libs.iter().filter(|l| l.id != id).cloned().collect();
        libs[i].roots = parse_roots(&st, r, &others, &mut creds).await?;
    }
    let lib = libs[i].clone();
    save_libraries(&st, &libs, Some(&creds)).await?;
    tracing::info!(id, "library changed");
    let mut v = settings::library_json(&lib.runtime(), &st.settings.stored(), None);
    v["scan"] = json!(scan_note(&st));
    Ok(Json(v))
}

async fn delete_library(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    crate::auth::admin_caller(&st, &headers, &q).await?;
    env_managed(&st)?;
    let mut libs = stored_libraries(&st);
    let n = libs.len();
    libs.retain(|l| l.id != id);
    if libs.len() == n {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such library".into()));
    }
    save_libraries(&st, &libs, None).await?;
    tracing::info!(id, "library removed");
    Ok(Json(json!({"deleted": id, "libraries": libraries_doc(&st)})))
}

// ---------------------------------------------------------------- folders

/// Top-level folders never browsed (the system's).
const SYSTEM_DIRS: &[&str] = &[
    "bin",
    "boot",
    "dev",
    "etc",
    "lib",
    "lib32",
    "lib64",
    "libx32",
    "lost+found",
    "opt",
    "proc",
    "root",
    "run",
    "sbin",
    "snap",
    "sys",
    "tmp",
    "usr",
    "var",
];

/// Folder names never listed (hidden ones, NAS housekeeping).
fn skipped(name: &str) -> bool {
    name.starts_with('.')
        || name.starts_with('@')
        || name.starts_with('#')
        || name.starts_with('$')
        || name == "System Volume Information"
        || name == "lost+found"
}

fn canonical(p: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(p).ok()
}

/// The server's own folders (its data, with the secrets, and its cache).
fn own_dirs(c: &Config) -> Vec<PathBuf> {
    [&c.data_dir, &c.cache_dir].iter().filter_map(|d| canonical(d)).collect()
}

/// The folders that may be browsed (canonical).
pub fn browse_roots(c: &Config) -> Vec<PathBuf> {
    let own = own_dirs(c);
    let mut roots: Vec<PathBuf> = if c.browse_roots.is_empty() && cfg!(target_os = "macos") {
        mac_roots()
    } else if c.browse_roots.is_empty() {
        std::fs::read_dir("/")
            .map(|rd| {
                rd.filter_map(std::result::Result::ok)
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() || t.is_symlink()))
                    .filter(|e| {
                        let n = e.file_name().to_string_lossy().to_string();
                        !SYSTEM_DIRS.contains(&n.as_str()) && !skipped(&n)
                    })
                    .filter_map(|e| canonical(&e.path()))
                    // Empty ones (a container's /home, /mnt…) aren't worth showing.
                    .filter(|p| std::fs::read_dir(p).is_ok_and(|mut d| d.next().is_some()))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        c.browse_roots.iter().filter_map(|r| canonical(Path::new(r))).collect()
    };
    roots.retain(|r| r.is_dir() && r != Path::new("/") && !own.iter().any(|o| r.starts_with(o) || o == r));
    roots.sort();
    roots.dedup();
    roots
}

/// macOS: the user's Movies and home folders and every mounted volume (`/Volumes/*`, the
/// startup disk's link to `/` left out).
fn mac_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_dir()) {
        out.extend([home.join("Movies"), home].into_iter().filter_map(|p| canonical(&p)));
    }
    if let Ok(rd) = std::fs::read_dir("/Volumes") {
        out.extend(
            rd.filter_map(std::result::Result::ok)
                .filter(|e| !skipped(&e.file_name().to_string_lossy()))
                .filter_map(|e| canonical(&e.path())),
        );
    }
    out.push(PathBuf::from("/Users/Shared"));
    out.into_iter().filter(|p| p.is_dir()).collect()
}

/// `path` resolved, if it is a folder that may be browsed.
pub fn check_dir(c: &Config, path: &str) -> Result<PathBuf, String> {
    if !path.starts_with('/') {
        return Err(format!("{path}: an absolute path"));
    }
    let p = canonical(Path::new(path)).ok_or_else(|| format!("{path}: no such folder"))?;
    if !p.is_dir() {
        return Err(format!("{path}: not a folder"));
    }
    if own_dirs(c).iter().any(|o| p.starts_with(o)) {
        return Err(format!("{path}: the server's own data"));
    }
    if !browse_roots(c).iter().any(|r| p.starts_with(r)) {
        return Err(format!("{path}: outside the folders the server may browse"));
    }
    Ok(p)
}

async fn browse(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    crate::auth::admin_caller(&st, &headers, &q).await?;
    let config = st.config.clone();
    let path = q.get("path").map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    let v = tokio::task::spawn_blocking(move || list(&config, path.as_deref()))
        .await
        .map_err(|e| ApiError::internal(e.into()))?;
    v.map(Json)
}

const MAX_ENTRIES: usize = 5000;

fn list(c: &Config, path: Option<&str>) -> Result<Value, ApiError> {
    let roots = browse_roots(c);
    let Some(path) = path else {
        let dirs: Vec<Value> = roots
            .iter()
            .map(|r| json!({"name": r.to_string_lossy(), "path": r.to_string_lossy()}))
            .collect();
        return Ok(json!({"path": null, "parent": null, "dirs": dirs, "video_files": 0, "truncated": false}));
    };
    let dir = check_dir(c, path).map_err(|e| ApiError(StatusCode::FORBIDDEN, e))?;
    let rd = std::fs::read_dir(&dir).map_err(|e| ApiError(StatusCode::FORBIDDEN, format!("{}: {e}", dir.display())))?;
    let mut dirs = Vec::new();
    let mut videos = 0usize;
    let mut truncated = false;
    for e in rd.filter_map(std::result::Result::ok) {
        if dirs.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if skipped(&name) {
            continue;
        }
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir() || (t.is_symlink() && e.path().is_dir()));
        if is_dir {
            dirs.push(json!({"name": name, "path": dir.join(&name).to_string_lossy()}));
        } else if crate::library::parse::is_video(&name) {
            videos += 1;
        }
    }
    dirs.sort_by_key(|d| d["name"].as_str().unwrap_or("").to_lowercase());
    let parent = dir
        .parent()
        .filter(|p| roots.iter().any(|r| p.starts_with(r)))
        .map(|p| p.to_string_lossy().to_string());
    Ok(json!({"path": dir.to_string_lossy(), "parent": parent, "dirs": dirs, "video_files": videos, "truncated": truncated}))
}

// ---------------------------------------------------------------- SMB

/// An SMB failure as the API answers it: `{error, problem, detail}`.
fn smb_failure(e: anyhow::Error) -> Response {
    let problem = smb::problem_of(&e);
    let status = match problem {
        smb::Problem::AuthRequired | smb::Problem::WrongPassword | smb::Problem::AccessDenied => StatusCode::UNPROCESSABLE_ENTITY,
        smb::Problem::NotFound => StatusCode::NOT_FOUND,
        smb::Problem::Unreachable | smb::Problem::Other => StatusCode::BAD_GATEWAY,
    };
    tracing::info!(?problem, "SMB request failed: {e:#}");
    (
        status,
        Json(json!({"error": problem.message(), "problem": problem, "detail": format!("{e:#}")})),
    )
        .into_response()
}

type SmbResult = Result<Json<Value>, Response>;

async fn admin_only(st: &AppState, headers: &HeaderMap, q: &HashMap<String, String>) -> Result<(), Response> {
    crate::auth::admin_caller(st, headers, q)
        .await
        .map(|_| ())
        .map_err(IntoResponse::into_response)
}

fn object(body: &Value) -> Result<&serde_json::Map<String, Value>, Response> {
    body.as_object().ok_or_else(|| bad("a JSON object").into_response())
}

fn sign_always(st: &AppState) -> bool {
    st.settings.current().smb_signing == "always"
}

async fn smb_discover(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> SmbResult {
    admin_only(&st, &headers, &q).await?;
    let wait = q
        .get("wait_ms")
        .and_then(|w| w.parse::<u64>().ok())
        .unwrap_or(2000)
        .clamp(300, 10_000);
    let found = smb::discover(std::time::Duration::from_millis(wait)).await;
    Ok(Json(json!({"servers": found})))
}

async fn smb_shares(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q, Json(body): Json<Value>) -> SmbResult {
    admin_only(&st, &headers, &q).await?;
    let o = object(&body)?;
    let host = smb::check_host(o.get("host").and_then(Value::as_str).unwrap_or("")).map_err(|e| bad(e.to_string()).into_response())?;
    let (creds, _) = creds_in(o, &st.settings.stored().smb_credentials()).map_err(IntoResponse::into_response)?;
    let shares = smb::list_shares(&host, &creds, sign_always(&st)).await.map_err(smb_failure)?;
    Ok(Json(json!({"host": host, "guest": creds.is_guest(), "shares": shares})))
}

async fn smb_browse(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q, Json(body): Json<Value>) -> SmbResult {
    admin_only(&st, &headers, &q).await?;
    let o = object(&body)?;
    let p = smb_target(o).map_err(IntoResponse::into_response)?;
    let (creds, _) = creds_in(o, &st.settings.stored().smb_credentials()).map_err(IntoResponse::into_response)?;
    let share = smb::Share::connect(&p.host, &p.share, &creds, sign_always(&st))
        .await
        .map_err(smb_failure)?;
    let entries = share.list(&p.path).await.map_err(smb_failure)?;
    let mut dirs = Vec::new();
    let mut videos = 0usize;
    let mut truncated = false;
    for e in entries {
        if skipped(&e.name) {
            continue;
        }
        if dirs.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        if e.is_dir && !e.is_link {
            dirs.push(json!({"name": e.name, "path": p.join(&e.name).path}));
        } else if !e.is_dir && crate::library::parse::is_video(&e.name) {
            videos += 1;
        }
    }
    dirs.sort_by_key(|d| d["name"].as_str().unwrap_or("").to_lowercase());
    let parent = (!p.path.is_empty()).then(|| p.path.rsplit_once('/').map_or(String::new(), |(a, _)| a.to_string()));
    Ok(Json(
        json!({"host": p.host, "share": p.share, "path": p.path, "url": p.url(), "parent": parent, "dirs": dirs, "video_files": videos, "truncated": truncated}),
    ))
}

async fn smb_test(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q, Json(body): Json<Value>) -> SmbResult {
    admin_only(&st, &headers, &q).await?;
    let o = object(&body)?;
    let p = smb_target(o).map_err(IntoResponse::into_response)?;
    let (creds, _) = creds_in(o, &st.settings.stored().smb_credentials()).map_err(IntoResponse::into_response)?;
    let (entries, took) = smb::probe_root(&p, &creds, sign_always(&st)).await.map_err(smb_failure)?;
    let dirs = entries.iter().filter(|e| e.is_dir).count();
    let videos = entries
        .iter()
        .filter(|e| !e.is_dir && crate::library::parse::is_video(&e.name))
        .count();
    let mut v = json!({"ok": !entries.is_empty(), "url": p.url(), "ms": took.as_millis() as u64, "entries": entries.len(), "dirs": dirs, "video_files": videos, "guest": creds.is_guest()});
    if entries.is_empty() {
        v["error"] = json!("the folder is empty");
    }
    Ok(Json(v))
}

/// Which roots use each credential: id → library ids.
fn credential_use(st: &AppState) -> HashMap<String, Vec<String>> {
    let mut m: HashMap<String, Vec<String>> = HashMap::new();
    for l in stored_libraries(st) {
        for r in &l.roots {
            if let Some(id) = &r.credential_id {
                let v = m.entry(id.clone()).or_default();
                if !v.contains(&l.id) {
                    v.push(l.id.clone());
                }
            }
        }
    }
    m
}

fn credentials_doc(st: &AppState) -> Value {
    let used = credential_use(st);
    let items: Vec<Value> = st
        .settings
        .stored()
        .smb_credentials()
        .iter()
        .map(|(id, c)| c.json(id, used.get(id).map_or(&[][..], Vec::as_slice)))
        .collect();
    json!({"items": items, "total": items.len()})
}

async fn save_credentials(st: &Arc<AppState>, creds: &BTreeMap<String, SmbCredential>) -> Result<(), Response> {
    let v = serde_json::to_value(creds).map_err(|e| ApiError::internal(e.into()).into_response())?;
    let (before, after) = st
        .settings
        .update(vec![("smb.credentials".into(), Some(v))])
        .await
        .map_err(|e| ApiError(StatusCode::SERVICE_UNAVAILABLE, format!("{e:#}")).into_response())?;
    apply(st, &before, &after).await;
    Ok(())
}

async fn list_credentials(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> SmbResult {
    admin_only(&st, &headers, &q).await?;
    Ok(Json(credentials_doc(&st)))
}

fn credential_from(o: &serde_json::Map<String, Value>, base: Option<&SmbCredential>) -> Result<SmbCredential, Response> {
    let mut c = base.cloned().unwrap_or_default();
    if let Some(h) = o.get("host").and_then(Value::as_str) {
        c.host = Some(smb::check_host(h).map_err(|e| bad(e.to_string()).into_response())?);
    }
    if let Some(u) = o.get("username").and_then(Value::as_str) {
        c.username = u.trim().to_string();
    }
    if let Some(p) = o.get("password").and_then(Value::as_str) {
        c.password = p.to_string();
    }
    if let Some(d) = o.get("domain").and_then(Value::as_str) {
        c.domain = d.trim().to_string();
    }
    if c.username.is_empty()
        || c.username.len() > 256
        || c.username.chars().any(char::is_control)
        || c.password.len() > 1024
        || c.domain.len() > 256
    {
        return Err(bad("username: 1 to 256 characters; password: up to 1024").into_response());
    }
    Ok(c)
}

async fn create_credential(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Q,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), Response> {
    admin_only(&st, &headers, &q).await?;
    let c = credential_from(object(&body)?, None)?;
    let mut all = st.settings.stored().smb_credentials();
    let id = remember_creds(&c.creds(), c.host.as_deref().unwrap_or(""), &mut all);
    if let Some(x) = all.get_mut(&id) {
        x.host.clone_from(&c.host);
    }
    save_credentials(&st, &all).await?;
    tracing::info!(id, "SMB credential stored");
    Ok((
        StatusCode::CREATED,
        Json(all[&id].json(&id, credential_use(&st).get(&id).map_or(&[][..], Vec::as_slice))),
    ))
}

async fn update_credential(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    Query(q): Q,
    Json(body): Json<Value>,
) -> SmbResult {
    admin_only(&st, &headers, &q).await?;
    let mut all = st.settings.stored().smb_credentials();
    let old = all
        .get(&id)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such credential".into()).into_response())?;
    let c = credential_from(object(&body)?, Some(old))?;
    all.insert(id.clone(), c);
    save_credentials(&st, &all).await?;
    tracing::info!(id, "SMB credential changed");
    Ok(Json(
        all[&id].json(&id, credential_use(&st).get(&id).map_or(&[][..], Vec::as_slice)),
    ))
}

async fn delete_credential(State(st): State<Arc<AppState>>, UrlPath(id): UrlPath<String>, headers: HeaderMap, Query(q): Q) -> SmbResult {
    admin_only(&st, &headers, &q).await?;
    let mut all = st.settings.stored().smb_credentials();
    if all.remove(&id).is_none() {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such credential".into()).into_response());
    }
    if let Some(libs) = credential_use(&st).get(&id) {
        return Err(ApiError(StatusCode::CONFLICT, format!("used by the libraries {}", libs.join(", "))).into_response());
    }
    save_credentials(&st, &all).await?;
    tracing::info!(id, "SMB credential removed");
    Ok(Json(json!({"deleted": id, "credentials": credentials_doc(&st)})))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn config(data: &Path, cache: &Path, roots: &[&Path]) -> Config {
        let mut c = Config::from_lookup(|_| None);
        c.data_dir = data.into();
        c.cache_dir = cache.into();
        c.browse_roots = roots.iter().map(|r| r.to_string_lossy().to_string()).collect();
        c
    }

    #[test]
    fn browsing_stays_inside_the_roots() {
        let t = std::env::temp_dir().join(format!("moonrise-browse-{}", std::process::id()));
        let media = t.join("media");
        std::fs::create_dir_all(media.join("Movies/Alien (1979)")).unwrap();
        std::fs::create_dir_all(media.join(".hidden")).unwrap();
        std::fs::write(media.join("Movies/a.mkv"), b"x").unwrap();
        std::fs::write(media.join("Movies/notes.txt"), b"x").unwrap();
        std::fs::create_dir_all(t.join("data/secrets")).unwrap();
        std::fs::create_dir_all(t.join("cache")).unwrap();
        std::fs::create_dir_all(t.join("elsewhere")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(t.join("data"), media.join("sneaky")).unwrap();
        let c = config(&t.join("data"), &t.join("cache"), &[&media, &t.join("data")]);
        let roots = browse_roots(&c);
        assert_eq!(roots, vec![canonical(&media).unwrap()], "the data folder is never a root");
        let v = list(&c, Some(media.join("Movies").to_str().unwrap())).unwrap();
        assert_eq!(v["video_files"], json!(1));
        assert_eq!(v["dirs"][0]["name"], json!("Alien (1979)"));
        assert!(!v.to_string().contains("notes.txt"), "file names aren't listed");
        let v = list(&c, Some(media.to_str().unwrap())).unwrap();
        let names: Vec<&str> = v["dirs"].as_array().unwrap().iter().map(|d| d["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"Movies") && !names.contains(&".hidden"));
        assert_eq!(v["parent"], Value::Null, "no way up from a root");
        for bad in [
            t.join("elsewhere"),
            t.join("data/secrets"),
            media.join("sneaky"),
            media.join("Movies/../../data"),
            PathBuf::from("/etc"),
        ] {
            assert!(list(&c, Some(bad.to_str().unwrap())).is_err(), "{}", bad.display());
        }
        assert!(check_dir(&c, "relative/path").is_err());
        let top = list(&c, None).unwrap();
        assert_eq!(top["dirs"].as_array().unwrap().len(), 1);
        std::fs::remove_dir_all(&t).ok();
    }

    /// A server on temporary folders (no library, no Bonjour), an admin and a plain user.
    pub(crate) async fn server(t: &Path, extra: &[(&str, &str)]) -> (Arc<AppState>, String, String) {
        std::fs::create_dir_all(t.join("media/Films/x")).unwrap();
        std::fs::write(t.join("media/Films/x/a.mkv"), b"x").unwrap();
        std::fs::create_dir_all(t.join("media/Series/s")).unwrap();
        std::fs::create_dir_all(t.join("media/Everything/x")).unwrap();
        std::fs::write(t.join("media/Series/s/e.mkv"), b"x").unwrap();
        let (data, cache, media) = (t.join("data"), t.join("cache"), t.join("media"));
        let mut vars: Vec<(String, String)> = vec![
            ("MOONRISE_DATA_DIR".into(), data.to_string_lossy().into()),
            ("MOONRISE_CACHE_DIR".into(), cache.to_string_lossy().into()),
            ("MOONRISE_BROWSE_ROOTS".into(), media.to_string_lossy().into()),
            ("MOONRISE_DISABLE".into(), "mdns".into()),
            ("TMDB_API_KEY".into(), "env-tmdb-key-0123456789".into()),
        ];
        vars.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        let st = Arc::new(AppState::new(crate::admin::config::Env::from_pairs(vars)).await.unwrap());
        let db = st.db.clone().unwrap();
        let (admin, user) = db
            .call(|c| {
                let a = crate::auth::add_user(c, "boss", "password-1", true)?;
                let u = crate::auth::add_user(c, "kid", "password-2", false)?;
                Ok((
                    crate::auth::issue_token(c, &a.id, None, None)?,
                    crate::auth::issue_token(c, &u.id, None, None)?,
                ))
            })
            .await
            .unwrap();
        (st, admin, user)
    }

    pub(crate) fn bearer(t: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("authorization", format!("Bearer {t}").parse().unwrap());
        h
    }

    /// A settings change by `admin` that must work.
    pub(crate) async fn patch(st: &Arc<AppState>, admin: &str, body: Value) -> Value {
        patch_settings(State(st.clone()), bearer(admin), Query(HashMap::new()), body.to_string().into())
            .await
            .unwrap()
            .0
    }

    /// A settings change by `admin`: its status.
    pub(crate) async fn try_patch(st: &Arc<AppState>, admin: &str, body: Value) -> StatusCode {
        status(patch_settings(State(st.clone()), bearer(admin), Query(HashMap::new()), body.to_string().into()).await)
    }

    /// A settings change by `admin` sent as it is (JSON text): its status.
    pub(crate) async fn try_patch_raw(st: &Arc<AppState>, admin: &str, body: &'static str) -> StatusCode {
        status(patch_settings(State(st.clone()), bearer(admin), Query(HashMap::new()), body.into()).await)
    }

    fn status<T>(r: Result<T, ApiError>) -> StatusCode {
        match r {
            Ok(_) => StatusCode::OK,
            Err(e) => e.0,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn admins_only_and_settings_apply_live() {
        let t = std::env::temp_dir().join(format!("moonrise-admin-{}", std::process::id()));
        let (st, admin, user) = server(&t, &[]).await;
        let q = || Query(HashMap::new());
        // Authorization: no token 401, a plain user 403, an admin 200.
        assert_eq!(
            status(get_settings(State(st.clone()), HeaderMap::new(), q()).await),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status(get_settings(State(st.clone()), bearer(&user), q()).await),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status(
                patch_settings(
                    State(st.clone()),
                    bearer(&user),
                    q(),
                    json!({"server.name": "Mine"}).to_string().into()
                )
                .await
            ),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status(list_libraries(State(st.clone()), bearer(&user), q()).await),
            StatusCode::FORBIDDEN
        );
        assert_eq!(status(browse(State(st.clone()), bearer(&user), q()).await), StatusCode::FORBIDDEN);
        assert_eq!(
            status(
                create_library(
                    State(st.clone()),
                    bearer(&user),
                    q(),
                    Json(json!({"name": "X", "kind": "movies", "roots": ["/"]}))
                )
                .await
            ),
            StatusCode::FORBIDDEN
        );
        let Json(doc) = get_settings(State(st.clone()), bearer(&admin), q()).await.unwrap();
        assert!(!doc.to_string().contains("env-tmdb-key"), "secrets never answered");
        assert_eq!(doc["settings"]["tmdb"]["api_key"]["last4"], json!("6789"));
        assert_eq!(doc["fields"]["tmdb.api_key"]["source"], json!("env"));

        // Changes apply at once: the name, OpenSubtitles credentials, a live switch.
        assert!(!st.opensubtitles.configured());
        let Json(doc) = patch_settings(
            State(st.clone()),
            bearer(&admin),
            q(),
            json!({"server": {"name": "Living Room"}, "opensubtitles": {"api_key": "k", "username": "u", "password": "p-secret"},
                        "features": {"convert": false, "transcode": false}, "performance.jobs": 6, "tmdb.api_key": "store-key-abcdefghijk"})
            .to_string()
            .into(),
        )
        .await
        .unwrap();
        assert_eq!(st.name(), "Living Room");
        assert!(st.opensubtitles.configured());
        assert!(!st.config.enabled(Feature::Convert), "a live switch");
        assert!(st.config.enabled(Feature::Transcode), "a restart-only switch waits for the restart");
        assert_eq!(doc["restart_required"], json!(["performance.jobs", "features.transcode"]));
        assert_eq!(
            doc["overridden_by_env"],
            json!(["tmdb.api_key"]),
            "TMDB_API_KEY in the environment still wins"
        );
        assert!(!doc.to_string().contains("p-secret") && !doc.to_string().contains("store-key"));
        assert_eq!(
            status(
                patch_settings(
                    State(st.clone()),
                    bearer(&admin),
                    q(),
                    json!({"features.moonrise_auth": false}).to_string().into()
                )
                .await
            ),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status(
                patch_settings(
                    State(st.clone()),
                    bearer(&admin),
                    q(),
                    json!({"performance.jobs": 0}).to_string().into()
                )
                .await
            ),
            StatusCode::BAD_REQUEST
        );

        // Libraries: added (checked against the browsable folders), renamed, removed.
        let media = canonical(&t.join("media")).unwrap();
        let films = media.join("Films").to_string_lossy().to_string();
        let r = create_library(
            State(st.clone()),
            bearer(&admin),
            q(),
            Json(json!({"name": "Films", "kind": "movies", "roots": [films.clone()]})),
        )
        .await
        .unwrap();
        assert_eq!((r.0, &r.1 .0["id"]), (StatusCode::CREATED, &json!("films")));
        assert_eq!(st.settings.current().libraries[0].roots, vec![films.clone()]);
        for (body, why) in [
            (json!({"name": "Again", "kind": "movies", "roots": [films.clone()]}), "same folder"),
            (
                json!({"name": "All", "kind": "movies", "roots": [media.to_string_lossy()]}),
                "around another root",
            ),
            (
                json!({"name": "Secrets", "kind": "movies", "roots": [t.join("data").to_string_lossy()]}),
                "the server's data",
            ),
            (
                json!({"name": "Nas", "kind": "movies", "roots": [{"source": "smb", "host": "nas; x", "share": "x"}]}),
                "bad host",
            ),
            (
                json!({"name": "Nfs", "kind": "movies", "roots": [{"source": "nfs", "path": "/x"}]}),
                "unknown source",
            ),
            (json!({"name": "Bad", "kind": "music", "roots": [films.clone()]}), "kind"),
        ] {
            assert_eq!(
                status(create_library(State(st.clone()), bearer(&admin), q(), Json(body)).await),
                StatusCode::BAD_REQUEST,
                "{why}"
            );
        }
        let series = media.join("Series").to_string_lossy().to_string();
        let _ = create_library(
            State(st.clone()),
            bearer(&admin),
            q(),
            Json(json!({"name": "Series", "kind": "shows", "roots": [series]})),
        )
        .await
        .unwrap();
        let _ = update_library(
            State(st.clone()),
            UrlPath("films".into()),
            bearer(&admin),
            q(),
            Json(json!({"name": "Movies"})),
        )
        .await
        .unwrap();
        assert_eq!(
            status(
                update_library(
                    State(st.clone()),
                    UrlPath("films".into()),
                    bearer(&admin),
                    q(),
                    Json(json!({"kind": "cartoons"}))
                )
                .await
            ),
            StatusCode::BAD_REQUEST
        );
        let _ = update_library(
            State(st.clone()),
            UrlPath("films".into()),
            bearer(&admin),
            q(),
            Json(json!({"kind": "mixed"})),
        )
        .await
        .unwrap();
        // No kind: a library with no type ("mixed").
        let everything = media.join("Everything").to_string_lossy().to_string();
        let r = create_library(
            State(st.clone()),
            bearer(&admin),
            q(),
            Json(json!({"name": "Media", "roots": [everything]})),
        )
        .await
        .unwrap();
        assert_eq!((&r.1 .0["id"], &r.1 .0["kind"]), (&json!("media"), &json!("mixed")));
        let _ = delete_library(State(st.clone()), UrlPath("media".into()), bearer(&admin), q())
            .await
            .unwrap();
        let names: Vec<String> = st
            .settings
            .current()
            .libraries
            .iter()
            .map(|l| format!("{}={}={}", l.id, l.name, l.kind))
            .collect();
        assert_eq!(names, vec!["films=Movies=mixed", "series=Series=shows"]);
        let _ = delete_library(State(st.clone()), UrlPath("films".into()), bearer(&admin), q())
            .await
            .unwrap();
        assert_eq!(st.settings.current().libraries.len(), 1);
        // The library tables follow (in the background, between passes).
        let lib = st.library.clone().unwrap();
        let mut ids = Vec::new();
        for _ in 0..50 {
            ids = lib
                .db
                .call(|c| {
                    let mut s = c.prepare("SELECT id FROM libraries ORDER BY sort")?;
                    let r = s.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
                    Ok(r)
                })
                .await
                .unwrap();
            if ids == ["series"] {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert_eq!(ids, ["series"]);

        // The store survives a restart; MOONRISE_LIBRARIES in the environment makes them read-only.
        drop(st);
        let (st, admin, ()) = {
            let vars = [("MOONRISE_LIBRARIES", "Movies=movies:/media/movies")];
            let (data, cache) = (t.join("data"), t.join("cache"));
            let env = crate::admin::config::Env::from_pairs(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).chain([
                ("MOONRISE_DATA_DIR".to_string(), data.to_string_lossy().to_string()),
                ("MOONRISE_CACHE_DIR".to_string(), cache.to_string_lossy().to_string()),
                ("MOONRISE_DISABLE".to_string(), "mdns,library".to_string()),
            ]));
            let st = Arc::new(AppState::new(env).await.unwrap());
            let token = st
                .db
                .clone()
                .unwrap()
                .call(|c| {
                    let (u, _) = crate::auth::user_by_name(c, "boss")?.unwrap();
                    crate::auth::issue_token(c, &u.id, None, None)
                })
                .await
                .unwrap();
            (st, token, ())
        };
        assert_eq!(st.name(), "Living Room");
        assert_eq!(st.config.jobs, 6, "restart-only settings apply after the restart");
        assert!(!st.config.enabled(Feature::Transcode));
        assert_eq!(st.config.libraries[0].id, "movies", "the environment's libraries win");
        assert_eq!(
            status(
                create_library(
                    State(st.clone()),
                    bearer(&admin),
                    q(),
                    Json(json!({"name": "X", "kind": "movies", "roots": ["/"]}))
                )
                .await
            ),
            StatusCode::CONFLICT
        );
        std::fs::remove_dir_all(&t).ok();
    }
}
