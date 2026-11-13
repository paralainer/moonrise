//! The settings store: the server's configuration kept in the database (`settings`, one row
//! per key, the value as JSON), edited through the admin API (`admin/mod.rs`) and applied by
//! [`crate::admin::config::Config::resolve`] under the environment (an environment variable that is
//! set wins; see `config.rs`).
//!
//! Keys (see [`KEYS`]): `server.name`, `metadata.language|country`, `tmdb.api_key`, `omdb.api_key`,
//! `opensubtitles.api_key|username|password`, `llm.enabled|provider|base_url|model|api_key|output|
//! max_calls_per_scan`, `performance.*` (jobs, cache and download space, scan intervals),
//! `features.<switch>` (false: switched off), `smb.signing` (`auto`|`always`), and `libraries` (a
//! list of `{id, name, kind, roots: [...]}`, each root `{source: "local", path}` or
//! `{source: "smb", host, share, path, credential_id?}`). SMB credentials are kept under
//! `smb.credentials` (`{<id>: {host?, username, password, domain?}}`), managed through
//! `/v1/admin/smb/credentials`, never answered (the password is shown as `{set}`).
//!
//! **Migration.** On the first start with this store (no `meta.settings_imported`), every
//! setting the environment provides (including keys that come from an `env_file`, the variable
//! named by `MOONRISE_LLM_API_KEY_ENV`, and the OpenSubtitles credentials file) is copied into the
//! store, and the libraries too (from `MOONRISE_LIBRARIES`, else the ones the database last ran
//! with), so the environment can be dropped later without changing anything. The environment
//! keeps winning while it is set.
//!
//! Secrets are stored as they are (the database file is readable by its owner only) and never
//! answered: the API says whether one is set, and the last 4 characters of an API key.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Mutex, RwLock},
};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    admin::config::{Config, Env, Feature, LibraryConfig},
    db::Db,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Text,
    /// Never answered by the API.
    Secret,
    Int(i64, i64),
    Bool,
    Choice(&'static [&'static str]),
}

/// One setting of the store.
#[derive(Debug)]
pub struct KeyDef {
    pub key: &'static str,
    /// The environment variable that overrides it.
    pub env: &'static str,
    /// Other variables that do (older names).
    pub aliases: &'static [&'static str],
    pub kind: Kind,
    /// Applied at once; otherwise when the server restarts.
    pub live: bool,
}

const fn k(key: &'static str, env: &'static str, kind: Kind, live: bool) -> KeyDef {
    KeyDef {
        key,
        env,
        aliases: &[],
        kind,
        live,
    }
}

pub const KEYS: &[KeyDef] = &[
    k("server.name", "MOONRISE_NAME", Kind::Text, true),
    k("metadata.language", "MOONRISE_TMDB_LANGUAGE", Kind::Text, true),
    k("metadata.country", "MOONRISE_TMDB_COUNTRY", Kind::Text, true),
    k("tmdb.api_key", "TMDB_API_KEY", Kind::Secret, true),
    k("omdb.api_key", "OMDB_API_KEY", Kind::Secret, true),
    k("opensubtitles.api_key", "OPENSUBTITLES_API_KEY", Kind::Secret, true),
    k("opensubtitles.username", "OPENSUBTITLES_USERNAME", Kind::Text, true),
    k("opensubtitles.password", "OPENSUBTITLES_PASSWORD", Kind::Secret, true),
    k("llm.enabled", "MOONRISE_LLM_ENABLED", Kind::Bool, true),
    // The presets of `llm/mod.rs`.
    KeyDef {
        key: "llm.provider",
        env: "MOONRISE_LLM_PROVIDER",
        aliases: &["MOONRISE_LLM_API_STYLE"],
        kind: Kind::Choice(&[
            "anthropic",
            "openai",
            "chatgpt",
            "gemini",
            "openrouter",
            "ollama",
            "lmstudio",
            "llamacpp",
            "custom",
        ]),
        live: true,
    },
    k("llm.base_url", "MOONRISE_LLM_BASE_URL", Kind::Text, true),
    k("llm.model", "MOONRISE_LLM_MODEL", Kind::Text, true),
    // Also overridden by the variable `MOONRISE_LLM_API_KEY_ENV` names (default: the provider's,
    // e.g. ANTHROPIC_API_KEY). Only ever sent to the provider it was given for: see
    // `admin::llm_provider_changes`.
    k("llm.api_key", "MOONRISE_LLM_API_KEY", Kind::Secret, true),
    k(
        "llm.output",
        "MOONRISE_LLM_OUTPUT",
        Kind::Choice(&["auto", "schema", "tool", "prompt"]),
        true,
    ),
    k(
        "llm.max_calls_per_scan",
        "MOONRISE_LLM_MAX_CALLS_PER_SCAN",
        Kind::Int(0, 100_000),
        true,
    ),
    k("performance.jobs", "MOONRISE_JOBS", Kind::Int(1, 64), false),
    k("performance.convert_jobs", "MOONRISE_CONVERT_JOBS", Kind::Int(1, 64), false),
    k("performance.transcode_jobs", "MOONRISE_TRANSCODE_JOBS", Kind::Int(1, 16), false),
    k("performance.download_jobs", "MOONRISE_DOWNLOAD_JOBS", Kind::Int(1, 8), false),
    k(
        "performance.download_max_gb",
        "MOONRISE_DOWNLOAD_MAX_GB",
        Kind::Int(1, 100_000),
        false,
    ),
    k("performance.cache_max_gb", "MOONRISE_CACHE_MAX_GB", Kind::Int(1, 100_000), false),
    k(
        "performance.trickplay_max_gb",
        "MOONRISE_TRICKPLAY_MAX_GB",
        Kind::Int(1, 100_000),
        false,
    ),
    k(
        "performance.scan_quick_secs",
        "MOONRISE_SCAN_QUICK_SECS",
        Kind::Int(10, 86_400),
        true,
    ),
    k("performance.scan_full_hours", "MOONRISE_SCAN_FULL_HOURS", Kind::Int(1, 720), true),
    k("performance.scan_probe_jobs", "MOONRISE_SCAN_PROBE_JOBS", Kind::Int(1, 16), true),
    k("smb.signing", "MOONRISE_SMB_SIGNING", Kind::Choice(&["auto", "always"]), true),
    k(
        "transcode.hardware",
        "MOONRISE_HWACCEL",
        Kind::Choice(&["auto", "nvidia", "qsv", "vaapi", "cpu"]),
        false,
    ),
    k("transcode.device", "MOONRISE_HWACCEL_DEVICE", Kind::Text, false),
];

pub fn key_def(key: &str) -> Option<&'static KeyDef> {
    KEYS.iter().find(|d| d.key == key)
}

/// The variables that override the setting `def` in this environment (with this store: the
/// LLM key's variable depends on the provider).
fn env_vars(def: &KeyDef, env: &Env, stored: &Stored) -> Vec<String> {
    let mut v: Vec<String> = std::iter::once(def.env)
        .chain(def.aliases.iter().copied())
        .map(String::from)
        .collect();
    if def.key == "llm.api_key" {
        let provider = env
            .get("MOONRISE_LLM_PROVIDER")
            .or_else(|| env.get("MOONRISE_LLM_API_STYLE"))
            .or_else(|| stored.get("llm.provider").and_then(Value::as_str))
            .unwrap_or("anthropic")
            .trim()
            .to_ascii_lowercase();
        let var = env
            .get("MOONRISE_LLM_API_KEY_ENV")
            .unwrap_or(crate::llm::preset_or_custom(&provider).key_env);
        if !var.is_empty() {
            v.push(var.to_string());
        }
    }
    v
}

/// The environment variable (of `env`) that sets `def`, if one does.
fn env_setter(def: &KeyDef, env: &Env, stored: &Stored) -> Option<String> {
    env_vars(def, env, stored).into_iter().find(|v| env.get(v).is_some())
}

/// The environment sets the setting whose (primary) variable is `var`: the store is skipped
/// for it (so an alias or the referenced key variable is read instead).
pub fn env_sets(env: &Env, stored: &Stored, var: &str) -> bool {
    KEYS.iter()
        .find(|d| d.env == var)
        .is_some_and(|d| env_setter(d, env, stored).is_some())
}

/// A library as stored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibrarySetting {
    pub id: String,
    pub name: String,
    /// What it was made as: "movies", "shows" or "mixed" (the default). Libraries have no type
    /// (any may hold movies and shows; `parse::classify` decides per file): kept for older apps.
    #[serde(default = "mixed")]
    pub kind: String,
    pub roots: Vec<Root>,
}

/// Where a library's files are: `local` (a directory the server can read: `path`) or `smb` (a
/// folder `path` in `share` on `host`, read by the server itself, signed in with the credential
/// `credential_id`, else as guest).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Root {
    #[serde(default = "local")]
    pub source: String,
    #[serde(default)]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
}

fn mixed() -> String {
    "mixed".into()
}

fn local() -> String {
    "local".into()
}

impl Root {
    pub fn local(path: &str) -> Root {
        Root {
            source: local(),
            path: path.to_string(),
            ..Default::default()
        }
    }

    pub fn smb(p: &crate::storage::smb::SmbPath, credential_id: Option<String>) -> Root {
        Root {
            source: "smb".into(),
            path: p.path.clone(),
            host: Some(p.host.clone()),
            share: Some(p.share.clone()),
            credential_id,
        }
    }

    pub fn smb_path(&self) -> Option<crate::storage::smb::SmbPath> {
        if self.source != "smb" {
            return None;
        }
        crate::storage::smb::SmbPath::new(self.host.as_deref()?, self.share.as_deref()?, &self.path)
    }

    /// The root as the library names its files' paths under it (None: not usable).
    pub fn runtime_path(&self) -> Option<String> {
        match self.source.as_str() {
            "local" => Some(self.path.trim_end_matches('/').to_string()).filter(|p| !p.is_empty()),
            "smb" => self.smb_path().map(|p| p.url()),
            _ => None,
        }
    }

    /// A runtime path (local or `smb://`) as a root.
    pub fn from_runtime(p: &str) -> Root {
        match crate::storage::smb::SmbPath::parse(p) {
            Some(s) => Root::smb(&s, None),
            None => Root::local(p),
        }
    }
}

impl LibrarySetting {
    pub fn runtime(&self) -> LibraryConfig {
        LibraryConfig {
            id: self.id.clone(),
            name: self.name.clone(),
            kind: self.kind.clone(),
            roots: self.roots.iter().filter_map(Root::runtime_path).collect(),
        }
    }

    pub fn from_runtime(l: &LibraryConfig) -> Self {
        LibrarySetting {
            id: l.id.clone(),
            name: l.name.clone(),
            kind: l.kind.clone(),
            roots: l.roots.iter().map(|p| Root::from_runtime(p)).collect(),
        }
    }
}

/// A stored SMB credential.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SmbCredential {
    /// The server it was made for (shown; not enforced).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub domain: String,
}

impl std::fmt::Debug for SmbCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmbCredential")
            .field("host", &self.host)
            .field("username", &self.username)
            .field("domain", &self.domain)
            .finish_non_exhaustive()
    }
}

impl SmbCredential {
    pub fn creds(&self) -> crate::storage::smb::Creds {
        crate::storage::smb::Creds {
            username: self.username.clone(),
            password: self.password.clone(),
            domain: self.domain.clone(),
        }
    }

    /// As the API shows it (never the password).
    pub fn json(&self, id: &str, used_by: &[String]) -> Value {
        json!({"id": id, "host": self.host, "username": self.username, "domain": self.domain, "password": {"set": !self.password.is_empty()}, "used_by": used_by})
    }
}

/// The shares the libraries use, with their credentials (roots of `libraries` that are on SMB;
/// the stored roots name the credential).
pub fn smb_shares(libraries: &[LibraryConfig], store: &Stored) -> Vec<crate::storage::smb::ShareConfig> {
    let creds = store.smb_credentials();
    let stored_roots: Vec<Root> = store.libraries().into_iter().flat_map(|l| l.roots).collect();
    let mut out: Vec<crate::storage::smb::ShareConfig> = Vec::new();
    for root in libraries.iter().flat_map(|l| l.roots.iter()) {
        let Some(p) = crate::storage::smb::SmbPath::parse(root) else {
            continue;
        };
        let cred = stored_roots
            .iter()
            .find(|r| {
                r.smb_path()
                    .is_some_and(|s| s.host.eq_ignore_ascii_case(&p.host) && s.share.eq_ignore_ascii_case(&p.share))
                    && r.credential_id.is_some()
            })
            .and_then(|r| creds.get(r.credential_id.as_deref()?))
            .map(SmbCredential::creds);
        if !out
            .iter()
            .any(|s| s.host.eq_ignore_ascii_case(&p.host) && s.share.eq_ignore_ascii_case(&p.share))
        {
            out.push(crate::storage::smb::ShareConfig {
                host: p.host.clone(),
                share: p.share.clone(),
                creds: cred,
            });
        }
    }
    out
}

/// What the store holds: key → JSON value. `Debug` shows the keys only.
#[derive(Clone, Default, PartialEq)]
pub struct Stored {
    values: BTreeMap<String, Value>,
}

impl std::fmt::Debug for Stored {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.values.keys()).finish()
    }
}

impl Stored {
    pub fn load(c: &Connection) -> Result<Stored> {
        let mut st = c.prepare("SELECT key, value FROM settings")?;
        let values = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .filter_map(std::result::Result::ok)
            .filter_map(|(k, v)| serde_json::from_str(&v).ok().map(|v| (k, v)))
            .collect();
        Ok(Stored { values })
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The stored value of the setting whose variable is `var`, as the variable would hold it.
    pub fn env_value(&self, var: &str) -> Option<String> {
        let def = KEYS.iter().find(|d| d.env == var)?;
        let v = self.values.get(def.key)?;
        Some(match (def.kind, v) {
            (Kind::Bool, Value::Bool(b)) if def.key == "llm.enabled" => (if *b { "auto" } else { "off" }).to_string(),
            (_, Value::String(s)) => s.clone(),
            (_, other) => other.to_string(),
        })
    }

    pub fn libraries(&self) -> Vec<LibrarySetting> {
        self.values
            .get("libraries")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }

    /// `smb.credentials`: id → credential.
    pub fn smb_credentials(&self) -> BTreeMap<String, SmbCredential> {
        self.values
            .get("smb.credentials")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }

    pub fn has_libraries(&self) -> bool {
        self.values.contains_key("libraries")
    }

    /// `features.<key>`: false = switched off.
    pub fn feature(&self, f: Feature) -> Option<bool> {
        self.values.get(&format!("features.{}", f.key())).and_then(Value::as_bool)
    }

    /// This store with `changes` made (None: removed), without writing anything.
    pub fn with(&self, changes: &[(String, Option<Value>)]) -> Stored {
        let mut values = self.values.clone();
        for (k, v) in changes {
            match v {
                Some(v) => values.insert(k.clone(), v.clone()),
                None => values.remove(k),
            };
        }
        Stored { values }
    }

    #[cfg(test)]
    pub fn from_pairs(pairs: impl IntoIterator<Item = (&'static str, Value)>) -> Stored {
        Stored {
            values: pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        }
    }
}

/// Writes changes (None: removed, back to the default) in one transaction.
pub fn write(c: &mut Connection, changes: &[(String, Option<Value>)]) -> Result<()> {
    let tx = c.transaction()?;
    let now = crate::library::util::now();
    for (k, v) in changes {
        match v {
            Some(v) => tx.execute(
                "INSERT INTO settings(key, value, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = ?3",
                params![k, v.to_string(), now],
            )?,
            None => tx.execute("DELETE FROM settings WHERE key = ?1", [k])?,
        };
    }
    tx.commit()?;
    Ok(())
}

/// Checks a value given for `key` (None: remove it). An empty secret or text removes it too.
pub fn validate(key: &str, v: &Value) -> Result<Option<Value>, String> {
    if v.is_null() {
        return Ok(None);
    }
    if let Some(name) = key.strip_prefix("features.") {
        let f = Feature::from_key(name).ok_or_else(|| format!("no feature {name}"))?;
        if f.env_only() {
            return Err(format!("{name} can only be switched in the environment (MOONRISE_DISABLE)"));
        }
        return v.as_bool().map(|b| Some(json!(b))).ok_or_else(|| format!("{key}: true or false"));
    }
    let def = key_def(key).ok_or_else(|| format!("no setting {key}"))?;
    match def.kind {
        Kind::Text | Kind::Secret => {
            let s = v.as_str().ok_or_else(|| format!("{key}: a string"))?.trim();
            if s.is_empty() {
                return Ok(None);
            }
            if s.len() > 4096 || s.chars().any(char::is_control) {
                return Err(format!("{key}: too long or has control characters"));
            }
            let s = match key {
                "server.name" if s.chars().count() > 64 => return Err("server.name: at most 64 characters".into()),
                "metadata.language" => {
                    let ok = s.len() == 2 && s.chars().all(|c| c.is_ascii_lowercase())
                        || (s.len() == 5
                            && s[..2].chars().all(|c| c.is_ascii_lowercase())
                            && &s[2..3] == "-"
                            && s[3..].chars().all(|c| c.is_ascii_uppercase()));
                    if !ok {
                        return Err("metadata.language: an ISO 639-1 code, optionally with a region (en, pt-BR)".into());
                    }
                    s.to_string()
                }
                "metadata.country" => {
                    let u = s.to_ascii_uppercase();
                    if u.len() != 2 || !u.chars().all(|c| c.is_ascii_uppercase()) {
                        return Err("metadata.country: an ISO 3166-1 code (US, GB, DE…)".into());
                    }
                    u
                }
                "llm.base_url" if !(s.starts_with("http://") || s.starts_with("https://")) || crate::llm::keys::origin_of(s).is_none() => {
                    return Err("llm.base_url: an http:// or https:// URL, without a user name or password".into())
                }
                _ => s.to_string(),
            };
            Ok(Some(json!(s)))
        }
        Kind::Int(lo, hi) => {
            let n = v
                .as_i64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
                .ok_or_else(|| format!("{key}: an integer"))?;
            if n < lo || n > hi {
                return Err(format!("{key}: {lo} to {hi}"));
            }
            Ok(Some(json!(n)))
        }
        Kind::Bool => v.as_bool().map(|b| Some(json!(b))).ok_or_else(|| format!("{key}: true or false")),
        Kind::Choice(list) => {
            let s = v
                .as_str()
                .map(str::to_ascii_lowercase)
                .ok_or_else(|| format!("{key}: one of {}", list.join(", ")))?;
            if !list.contains(&s.as_str()) {
                return Err(format!("{key}: one of {}", list.join(", ")));
            }
            Ok(Some(json!(s)))
        }
    }
}

/// A JSON document read strictly: an object that names a key twice (`{"a": 1, "a": 2}`) is an
/// error rather than the last one winning.
pub fn parse_strict(bytes: &[u8]) -> Result<Value, String> {
    use serde::de::{Deserializer, Error, MapAccess, SeqAccess, Visitor};
    struct Strict(Value);
    struct V;
    impl<'de> Deserialize<'de> for Strict {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            d.deserialize_any(V).map(Strict)
        }
    }
    impl<'de> Visitor<'de> for V {
        type Value = Value;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("JSON")
        }
        fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
            Ok(Value::Bool(v))
        }
        fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
            Ok(json!(v))
        }
        fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
            Ok(json!(v))
        }
        fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
            Ok(json!(v))
        }
        fn visit_str<E>(self, v: &str) -> Result<Value, E> {
            Ok(Value::String(v.to_string()))
        }
        fn visit_string<E>(self, v: String) -> Result<Value, E> {
            Ok(Value::String(v))
        }
        fn visit_unit<E>(self) -> Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_none<E>(self) -> Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
            let mut out = Vec::new();
            while let Some(Strict(v)) = seq.next_element()? {
                out.push(v);
            }
            Ok(Value::Array(out))
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
            let mut out = serde_json::Map::new();
            while let Some(k) = map.next_key::<String>()? {
                if out.contains_key(&k) {
                    return Err(A::Error::custom(format!("{k}: given more than once")));
                }
                let Strict(v) = map.next_value()?;
                out.insert(k, v);
            }
            Ok(Value::Object(out))
        }
    }
    serde_json::from_slice::<Strict>(bytes).map(|s| s.0).map_err(|e| e.to_string())
}

/// `{"a": {"b": 1}, "c.d": 2}` → `[("a.b", 1), ("c.d", 2)]`. A secret's masked form
/// (`{"set": …}`) given back unchanged is skipped.
pub fn flatten(v: &Value) -> Vec<(String, Value)> {
    fn go(prefix: &str, v: &Value, out: &mut Vec<(String, Value)>) {
        match v {
            Value::Object(m) if !prefix.is_empty() && m.contains_key("set") => {}
            Value::Object(m) if prefix.is_empty() || key_def(prefix).is_none() => {
                for (k, x) in m {
                    let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                    go(&key, x, out);
                }
            }
            _ => out.push((prefix.to_string(), v.clone())),
        }
    }
    let mut out = Vec::new();
    go("", v, &mut out);
    out
}

/// The value in effect for `key` (secrets in the clear: mask before answering).
pub fn effective(c: &Config, key: &str) -> Value {
    if let Some(f) = key.strip_prefix("features.").and_then(Feature::from_key) {
        return json!(c.enabled(f));
    }
    match key {
        "server.name" => json!(c.name),
        "metadata.language" => json!(c.tmdb_language),
        "metadata.country" => json!(c.tmdb_country),
        "tmdb.api_key" => json!(c.tmdb_key.expose()),
        "omdb.api_key" => json!(c.omdb_key.expose()),
        "opensubtitles.api_key" => json!(c.opensubtitles.api_key.expose()),
        "opensubtitles.username" => json!(c.opensubtitles.username.expose()),
        "opensubtitles.password" => json!(c.opensubtitles.password.expose()),
        "llm.enabled" => json!(c.llm.allowed),
        "llm.provider" => json!(c.llm.provider),
        "llm.base_url" => json!(c.llm.base_url),
        "llm.model" => json!(c.llm.model),
        "llm.api_key" => json!(c.llm.api_key.expose()),
        "llm.output" => json!(c.llm.output),
        "llm.max_calls_per_scan" => json!(c.llm.max_calls_per_scan),
        "performance.jobs" => json!(c.jobs),
        "performance.convert_jobs" => json!(c.convert_jobs),
        "performance.transcode_jobs" => json!(c.transcode_jobs),
        "performance.download_jobs" => json!(c.download_jobs),
        "performance.download_max_gb" => json!(c.download_max_bytes >> 30),
        "performance.cache_max_gb" => json!(c.cache_max_bytes >> 30),
        "performance.trickplay_max_gb" => json!(c.trickplay_max_bytes >> 30),
        "performance.scan_quick_secs" => json!(c.scan_quick_secs),
        "performance.scan_full_hours" => json!(c.scan_full_hours),
        "performance.scan_probe_jobs" => json!(c.scan_probe_jobs),
        "smb.signing" => json!(c.smb_signing),
        "transcode.hardware" => json!(c.hwaccel),
        "transcode.device" => json!(c.hwaccel_device),
        _ => Value::Null,
    }
}

/// A secret as the API shows it: whether it is set, and an API key's last 4 characters.
pub fn mask(key: &str, v: &Value) -> Value {
    let s = v.as_str().unwrap_or("");
    let n = s.chars().count();
    let mut m = json!({"set": !s.is_empty()});
    if key.ends_with("api_key") && n >= 16 {
        m["last4"] = json!(s.chars().skip(n - 4).collect::<String>());
    }
    m
}

/// Every key the API shows: the settings, then the feature switches.
pub fn all_keys() -> Vec<String> {
    KEYS.iter()
        .map(|d| d.key.to_string())
        .chain(Feature::ALL.iter().map(|f| format!("features.{}", f.key())))
        .collect()
}

/// Where a setting's value comes from.
pub fn source(env: &Env, stored: &Stored, key: &str) -> (&'static str, Option<String>) {
    if let Some(f) = key.strip_prefix("features.").and_then(Feature::from_key) {
        let listed = env
            .get("MOONRISE_DISABLE")
            .is_some_and(|d| d.split(',').any(|x| x.trim().eq_ignore_ascii_case(f.key())));
        return if listed {
            ("env", Some("MOONRISE_DISABLE".into()))
        } else if stored.get(key).is_some() {
            ("store", None)
        } else {
            ("default", None)
        };
    }
    let Some(def) = key_def(key) else { return ("default", None) };
    if let Some(var) = env_setter(def, env, stored) {
        return (if env.is_from_file(&var) { "file" } else { "env" }, Some(var));
    }
    if stored.get(key).is_some() || (key == "llm.api_key" && stored.get(crate::llm::keys::VAULT_KEY).is_some()) {
        ("store", None)
    } else if env.builtin.for_key(key).is_some() {
        ("builtin", None)
    } else {
        ("default", None)
    }
}

/// Whether a change of `key` applies at once.
pub fn is_live(key: &str) -> bool {
    match key.strip_prefix("features.").and_then(Feature::from_key) {
        Some(f) => f.live(),
        None => key_def(key).is_some_and(|d| d.live),
    }
}

/// The one-time import of the environment into an empty store (see the module docs). Returns
/// the keys imported (never values).
pub fn import_env(c: &mut Connection, env: &Env) -> Result<Vec<String>> {
    let done: Option<String> = c
        .query_row("SELECT value FROM meta WHERE key = 'settings_imported'", [], |r| r.get(0))
        .optional()?;
    if done.is_some() {
        return Ok(Vec::new());
    }
    let existing = Stored::load(c)?;
    let mut changes: Vec<(String, Option<Value>)> = Vec::new();
    if existing.is_empty() {
        let from_env = Config::resolve(env, &Stored::default());
        for def in KEYS {
            if env_setter(def, env, &existing).is_none() {
                continue;
            }
            let v = effective(&from_env, def.key);
            let v = match validate(def.key, &v) {
                Ok(Some(v)) => v,
                // Kept as given (the environment accepted it); an empty one isn't stored.
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!("settings import: {e}; stored as given");
                    v
                }
            };
            changes.push((def.key.to_string(), Some(v)));
        }
        if let Some(off) = env.get("MOONRISE_DISABLE") {
            for f in off.split(',').filter_map(|x| Feature::from_key(&x.trim().to_ascii_lowercase())) {
                if !f.env_only() {
                    changes.push((format!("features.{}", f.key()), Some(json!(false))));
                }
            }
        }
        // The libraries: the environment's, else the ones the database last ran with.
        let libs: Vec<LibrarySetting> = if env.get("MOONRISE_LIBRARIES").is_some() {
            from_env.libraries.iter().map(LibrarySetting::from_runtime).collect()
        } else {
            let mut st = c.prepare("SELECT id, name, kind, roots FROM libraries ORDER BY sort")?;
            let rows = st
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows.into_iter()
                .map(|(id, name, kind, roots)| {
                    LibrarySetting::from_runtime(&LibraryConfig {
                        id,
                        name,
                        kind,
                        roots: serde_json::from_str(&roots).unwrap_or_default(),
                    })
                })
                .collect()
        };
        if !libs.is_empty() {
            changes.push(("libraries".into(), Some(serde_json::to_value(&libs)?)));
        }
    }
    write(c, &changes)?;
    c.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES ('settings_imported', ?1)",
        [crate::library::util::now().to_string()],
    )?;
    Ok(changes.into_iter().map(|(k, _)| k).collect())
}

/// The store, with keys stored before they were bound to an address moved into the vault
/// (written, once: see `llm::keys::migration`).
pub fn load(c: &mut Connection, env: &Env) -> Result<Stored> {
    let stored = Stored::load(c)?;
    let moved = crate::llm::keys::migration(env, &stored);
    if moved.is_empty() {
        return Ok(stored);
    }
    write(c, &moved)?;
    tracing::info!("LLM keys bound to the provider and address they were used with");
    Stored::load(c)
}

/// Why [`Settings::update_with`] changed nothing.
#[derive(Debug)]
pub enum UpdateError {
    /// `make` refused (a bad request).
    Refused(String),
    Failed(anyhow::Error),
}

/// The settings at run time: the environment, the store, the configuration the server started
/// with and the one in effect now.
pub struct Settings {
    env: Env,
    db: Option<Db>,
    stored: Mutex<Stored>,
    /// As started (what the restart-only settings still run with).
    boot: Config,
    current: RwLock<Config>,
    /// One change at a time.
    write: tokio::sync::Mutex<()>,
}

impl Settings {
    /// Loads the store (importing the environment the first time) and resolves the configuration.
    pub async fn open(env: Env, db: Option<Db>) -> Result<Settings> {
        let stored = match &db {
            Some(db) => {
                let e = env.clone();
                let (imported, stored) = db
                    .call(move |c| {
                        let imported = import_env(c, &e)?;
                        Ok((imported, load(c, &e)?))
                    })
                    .await
                    .context("settings")?;
                if !imported.is_empty() {
                    tracing::info!(keys = ?imported, "settings imported from the environment into the store");
                }
                stored
            }
            None => Stored::default(),
        };
        let boot = Config::resolve(&env, &stored);
        Ok(Settings {
            current: RwLock::new(boot.clone()),
            boot,
            env,
            db,
            stored: Mutex::new(stored),
            write: tokio::sync::Mutex::new(()),
        })
    }

    pub fn env(&self) -> &Env {
        &self.env
    }

    pub fn boot(&self) -> &Config {
        &self.boot
    }

    pub fn current(&self) -> Config {
        self.current.read().unwrap().clone()
    }

    pub fn stored(&self) -> Stored {
        self.stored.lock().unwrap().clone()
    }

    /// `MOONRISE_LIBRARIES` is set: the libraries can't be edited here.
    pub fn libraries_from_env(&self) -> bool {
        self.env.get("MOONRISE_LIBRARIES").is_some()
    }

    /// Stores changes; answers (before, after) configurations.
    pub async fn update(&self, changes: Vec<(String, Option<Value>)>) -> Result<(Config, Config)> {
        self.update_with(move |_| Ok(changes)).await.map_err(|e| match e {
            UpdateError::Refused(m) => anyhow::anyhow!(m),
            UpdateError::Failed(e) => e,
        })
    }

    /// Stores the changes `make` decides from the store as it is now (one change at a time, so
    /// nothing changes between deciding and writing); `make` may refuse.
    pub async fn update_with(
        &self,
        make: impl FnOnce(&Stored) -> Result<Vec<(String, Option<Value>)>, String> + Send + 'static,
    ) -> Result<(Config, Config), UpdateError> {
        let db = self
            .db
            .clone()
            .context("the settings store needs the database")
            .map_err(UpdateError::Failed)?;
        let _g = self.write.lock().await;
        let env = self.env.clone();
        let stored = db
            .call(move |c| {
                // Keys stored before they were bound move first (`llm/keys.rs`).
                let now = load(c, &env)?;
                match make(&now) {
                    Ok(ch) => {
                        write(c, &ch)?;
                        Ok(Ok(Stored::load(c)?))
                    }
                    Err(m) => Ok(Err(m)),
                }
            })
            .await
            .map_err(UpdateError::Failed)?
            .map_err(UpdateError::Refused)?;
        let before = self.current();
        let after = Config::resolve(&self.env, &stored);
        *self.stored.lock().unwrap() = stored;
        *self.current.write().unwrap() = after.clone();
        Ok((before, after))
    }

    /// The document `GET /v1/admin/settings` answers.
    pub fn document(&self) -> Value {
        let cur = self.current();
        let stored = self.stored();
        let mut settings = json!({});
        let mut fields = serde_json::Map::new();
        let mut restart = Vec::new();
        for key in all_keys() {
            let secret = key_def(&key).is_some_and(|d| d.kind == Kind::Secret);
            let v = effective(&cur, &key);
            let (src, var) = source(&self.env, &stored, &key);
            // The project's own key: whether there is one, nothing of it.
            let shown = match (secret, src) {
                (true, "builtin") => json!({"set": true, "builtin": true}),
                (true, _) => mask(&key, &v),
                _ => v.clone(),
            };
            let mut node = &mut settings;
            let parts: Vec<&str> = key.split('.').collect();
            for p in &parts[..parts.len() - 1] {
                node = node.as_object_mut().unwrap().entry(p.to_string()).or_insert_with(|| json!({}));
            }
            node[parts[parts.len() - 1]] = shown;
            let live = is_live(&key);
            let env_only = key
                .strip_prefix("features.")
                .and_then(Feature::from_key)
                .is_some_and(Feature::env_only);
            let pending = !live && effective(&self.boot, &key) != v;
            if pending {
                restart.push(key.clone());
            }
            let mut f = json!({"source": src, "apply": if env_only { "env-only" } else if live { "live" } else { "restart" }});
            if let Some(var) = var {
                f["env"] = json!(var);
            }
            if pending {
                f["restart_pending"] = json!(true);
            }
            fields.insert(key, f);
        }
        json!({
            "settings": settings,
            "fields": fields,
            "libraries": {
                "managed_by": if self.libraries_from_env() { "env" } else { "store" },
                "items": cur.libraries.iter().map(|l| library_json(l, &stored, None)).collect::<Vec<_>>(),
            },
            "restart_required": restart,
        })
    }
}

/// A library as the admin API shows it (with its stored roots, other sources included).
pub fn library_json(
    l: &LibraryConfig,
    stored: &Stored,
    states: Option<&std::collections::HashMap<String, crate::library::RootState>>,
) -> Value {
    let roots: Vec<Root> = stored
        .libraries()
        .into_iter()
        .find(|s| s.id == l.id)
        .map_or_else(|| l.roots.iter().map(|p| Root::from_runtime(p)).collect(), |s| s.roots);
    let creds = stored.smb_credentials();
    json!({
        "id": l.id, "name": l.name, "kind": l.kind,
        "roots": roots.iter().map(|r| {
            let rt = r.runtime_path().unwrap_or_default();
            // From the last scan pass (a share isn't asked here); a local root not scanned yet
            // is looked at.
            let state = states.and_then(|s| s.get(&rt));
            let available = match state {
                Some(s) => s.online,
                None => r.source == "local" && std::fs::read_dir(&r.path).is_ok_and(|mut d| d.next().is_some()),
            };
            let mut v = json!({"source": r.source, "path": r.path, "available": available});
            if r.source == "smb" {
                v["host"] = json!(r.host);
                v["share"] = json!(r.share);
                v["url"] = json!(rt);
                v["credential_id"] = json!(r.credential_id);
                v["username"] = json!(r.credential_id.as_ref().and_then(|c| creds.get(c)).map(|c| c.username.clone()));
            }
            if let Some(s) = state {
                v["state"] = json!(if s.online { "online" } else { "offline" });
                if let Some(e) = &s.error {
                    v["error"] = json!(e);
                }
                v["since"] = json!(s.since);
            }
            v
        }).collect::<Vec<_>>(),
    })
}

/// Keys whose values differ between two configurations (secrets compared, never shown).
pub fn changed_keys(a: &Config, b: &Config) -> BTreeSet<String> {
    all_keys().into_iter().filter(|k| effective(a, k) != effective(b, k)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Env {
        Env::from_pairs(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
    }

    #[test]
    fn precedence_env_over_store_over_default() {
        let store = Stored::from_pairs([
            ("metadata.country", json!("GB")),
            ("metadata.language", json!("de")),
            ("tmdb.api_key", json!("store-key")),
            ("performance.jobs", json!(7)),
            ("llm.enabled", json!(false)),
        ]);
        let c = Config::resolve(&env(&[("MOONRISE_TMDB_COUNTRY", "AU")]), &store);
        assert_eq!(c.tmdb_country, "AU", "the environment wins");
        assert_eq!(c.tmdb_language, "de", "then the store");
        assert_eq!(c.tmdb_key.expose(), "store-key");
        assert_eq!(c.jobs, 7);
        assert!(!c.llm.allowed && !c.llm.enabled);
        assert_eq!(c.scan_quick_secs, 120, "then the default");
        let c = Config::resolve(&env(&[]), &Stored::default());
        assert_eq!((c.tmdb_country.as_str(), c.tmdb_language.as_str()), ("US", "en"));
        let s = Stored::default();
        assert_eq!(
            source(&env(&[("MOONRISE_TMDB_COUNTRY", "AU")]), &store, "metadata.country"),
            ("env", Some("MOONRISE_TMDB_COUNTRY".into()))
        );
        assert_eq!(source(&env(&[]), &store, "metadata.language").0, "store");
        assert_eq!(source(&env(&[]), &s, "metadata.language").0, "default");
    }

    #[test]
    fn strict_json() {
        assert_eq!(
            parse_strict(br#"{"a": {"b": [1, "x", null, true, 1.5]}, "c.d": 2}"#).unwrap(),
            json!({"a": {"b": [1, "x", null, true, 1.5]}, "c.d": 2})
        );
        assert!(parse_strict(br#"{"llm.base_url": "a", "llm.base_url": "b"}"#)
            .unwrap_err()
            .contains("given more than once"));
        assert!(parse_strict(br#"{"llm": {"base_url": "a", "base_url": "b"}}"#).is_err());
        assert!(parse_strict(b"{").is_err());
    }

    #[test]
    fn llm_key_variables_win_over_the_store() {
        let bound = |p: &str, base: &str, key: &str| {
            crate::llm::keys::change(Vec::new(), &crate::llm::keys::Binding::new(p, base).unwrap(), Some(key.into()))
                .1
                .unwrap()
        };
        let store = Stored::from_pairs([
            (
                crate::llm::keys::VAULT_KEY,
                bound("anthropic", "https://api.anthropic.com", "store-llm"),
            ),
            ("llm.provider", json!("anthropic")),
        ]);
        // The variable MOONRISE_LLM_API_KEY_ENV names (default: the provider's) overrides the store.
        let c = Config::resolve(&env(&[("ANTHROPIC_API_KEY", "env-llm")]), &store);
        assert_eq!(c.llm.api_key.expose(), "env-llm");
        let c = Config::resolve(
            &env(&[("MOONRISE_LLM_API_KEY_ENV", "K"), ("K", "k-llm"), ("ANTHROPIC_API_KEY", "ignored")]),
            &store,
        );
        assert_eq!(c.llm.api_key.expose(), "k-llm");
        let c = Config::resolve(&env(&[]), &store);
        assert_eq!(c.llm.api_key.expose(), "store-llm");
        assert_eq!(
            source(&env(&[("ANTHROPIC_API_KEY", "x")]), &store, "llm.api_key"),
            ("env", Some("ANTHROPIC_API_KEY".into()))
        );
        // Another provider in the store: ANTHROPIC_API_KEY is not its key.
        let store = Stored::from_pairs([
            (
                crate::llm::keys::VAULT_KEY,
                bound("openrouter", "https://openrouter.ai/api/v1", "store-or"),
            ),
            ("llm.provider", json!("openrouter")),
        ]);
        let c = Config::resolve(&env(&[("ANTHROPIC_API_KEY", "env-llm")]), &store);
        assert_eq!((c.llm.api_key.expose(), c.llm.style.as_str()), ("store-or", "openai"));
        assert_eq!(source(&env(&[("ANTHROPIC_API_KEY", "x")]), &store, "llm.api_key").0, "store");
        let c = Config::resolve(&env(&[("OPENROUTER_API_KEY", "env-or")]), &store);
        assert_eq!(c.llm.api_key.expose(), "env-or");
        // An alias in the environment beats the store's value of the primary name.
        let c = Config::resolve(&env(&[("MOONRISE_LLM_API_STYLE", "anthropic")]), &store);
        assert_eq!(c.llm.provider, "anthropic");
    }

    #[test]
    fn features_env_only_switches_off() {
        let store = Stored::from_pairs([
            ("features.transcode", json!(false)),
            ("features.llm", json!(true)),
            ("features.db", json!(false)),
        ]);
        let c = Config::resolve(&env(&[("MOONRISE_DISABLE", "llm")]), &store);
        assert!(!c.enabled(Feature::Transcode), "off in the store");
        assert!(!c.enabled(Feature::Llm), "MOONRISE_DISABLE wins over the store's true");
        assert!(c.enabled(Feature::Db), "db is switched in the environment only");
        assert_eq!(source(&env(&[("MOONRISE_DISABLE", "llm")]), &store, "features.llm").0, "env");
        assert!(validate("features.moonrise_auth", &json!(false)).is_err());
        assert!(validate("features.nope", &json!(false)).is_err());
        assert_eq!(validate("features.convert", &json!(false)), Ok(Some(json!(false))));
    }

    #[test]
    fn libraries_from_env_or_store() {
        let libs = vec![LibrarySetting {
            id: "films".into(),
            name: "Films".into(),
            kind: "movies".into(),
            roots: vec![
                Root::local("/m/films/"),
                Root {
                    source: "smb".into(),
                    path: "/movies/".into(),
                    host: Some("nas".into()),
                    share: Some("Media".into()),
                    credential_id: Some("c1".into()),
                },
                Root {
                    source: "nfs".into(),
                    path: "x".into(),
                    ..Default::default()
                },
            ],
        }];
        let creds = json!({"c1": {"username": "media", "password": "pw"}});
        let store = Stored::from_pairs([("libraries", serde_json::to_value(&libs).unwrap()), ("smb.credentials", creds)]);
        let c = Config::resolve(&env(&[]), &store);
        assert_eq!(
            c.libraries,
            vec![LibraryConfig {
                id: "films".into(),
                name: "Films".into(),
                kind: "movies".into(),
                roots: vec!["/m/films".into(), "smb://nas/Media/movies".into()]
            }]
        );
        assert_eq!(c.smb_shares.len(), 1);
        assert_eq!(
            c.smb_shares[0].creds.as_ref().map(|c| (c.username.as_str(), c.password.as_str())),
            Some(("media", "pw"))
        );
        assert!(!format!("{c:?}").contains("\"pw\""), "the password isn't in the Debug output");
        assert_eq!(c.smb_signing, "auto");
        let c = Config::resolve(&env(&[("MOONRISE_LIBRARIES", "Movies=movies:/media/movies")]), &store);
        assert_eq!(c.libraries[0].id, "movies");
        let c = Config::resolve(&env(&[]), &Stored::default());
        assert!(c.libraries.is_empty());
    }

    #[test]
    fn validation() {
        assert_eq!(validate("metadata.country", &json!("gb")), Ok(Some(json!("GB"))));
        assert!(validate("metadata.country", &json!("GBR")).is_err());
        assert_eq!(validate("metadata.language", &json!("pt-BR")), Ok(Some(json!("pt-BR"))));
        assert!(validate("metadata.language", &json!("english")).is_err());
        assert_eq!(validate("performance.jobs", &json!(3)), Ok(Some(json!(3))));
        assert!(validate("performance.jobs", &json!(0)).is_err());
        assert!(validate("performance.jobs", &json!("x")).is_err());
        assert_eq!(validate("tmdb.api_key", &json!("  ")), Ok(None), "empty clears");
        assert_eq!(validate("tmdb.api_key", &Value::Null), Ok(None));
        assert!(validate("llm.provider", &json!("bard")).is_err());
        assert!(validate("llm.base_url", &json!("ftp://x")).is_err());
        assert!(validate("nope", &json!(1)).is_err());
        assert!(validate("server.name", &json!("x".repeat(65))).is_err());
    }

    #[test]
    fn masking() {
        assert_eq!(
            mask("tmdb.api_key", &json!("abcdefghijklmnop1234")),
            json!({"set": true, "last4": "1234"})
        );
        assert_eq!(
            mask("tmdb.api_key", &json!("short")),
            json!({"set": true}),
            "short keys show nothing"
        );
        assert_eq!(
            mask("opensubtitles.password", &json!("a-long-password-1234")),
            json!({"set": true}),
            "passwords never"
        );
        assert_eq!(mask("tmdb.api_key", &json!("")), json!({"set": false}));
    }

    #[test]
    fn flatten_nested_and_dotted() {
        let v = json!({"metadata": {"country": "GB"}, "llm.model": "m", "tmdb": {"api_key": {"set": true, "last4": "1234"}}, "features": {"convert": false}});
        let mut f = flatten(&v);
        f.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            f,
            vec![
                ("features.convert".into(), json!(false)),
                ("llm.model".into(), json!("m")),
                ("metadata.country".into(), json!("GB"))
            ]
        );
    }

    #[tokio::test]
    async fn builtin_keys_are_reported_not_shown() {
        let mut e = env(&[]);
        e.builtin = crate::admin::config::Builtin {
            tmdb: crate::admin::config::Secret::new("0123456789abcdef-built"),
            opensubtitles: crate::admin::config::Secret::default(),
        };
        assert_eq!(source(&e, &Stored::default(), "tmdb.api_key"), ("builtin", None));
        assert_eq!(source(&e, &Stored::default(), "opensubtitles.api_key").0, "default");
        let s = Settings::open(e, Some(Db::memory().unwrap())).await.unwrap();
        let doc = s.document();
        assert_eq!(doc["settings"]["tmdb"]["api_key"], json!({"set": true, "builtin": true}));
        assert_eq!(doc["fields"]["tmdb.api_key"]["source"], "builtin");
        assert!(
            !doc.to_string().contains("cdef-built") && !doc.to_string().contains("0123"),
            "no part of the key"
        );
        // The admin's own key takes over.
        s.update(vec![("tmdb.api_key".into(), Some(json!("my-own-key-1234567890")))])
            .await
            .unwrap();
        assert_eq!(s.current().tmdb_key.expose(), "my-own-key-1234567890");
        assert_eq!(s.document()["fields"]["tmdb.api_key"]["source"], "store");
    }

    #[tokio::test]
    async fn import_once_and_document_masks() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            c.execute(
                "INSERT INTO libraries(id, name, kind, roots, sort) VALUES ('old', 'Old', 'movies', '[\"/media/old\"]', 0)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let mut e = env(&[
            ("TMDB_API_KEY", "tmdb-secret-key-0123456789abcd"),
            ("ANTHROPIC_API_KEY", "sk-ant-secret-wxyz"),
            ("MOONRISE_TMDB_COUNTRY", "AU"),
            ("MOONRISE_JOBS", "3"),
            ("MOONRISE_DISABLE", "convert,db"),
            ("MOONRISE_LIBRARIES", "Movies=movies:/media/movies;Shows=shows:/media/tv shows"),
            ("MOONRISE_DATA_DIR", "/data"),
        ]);
        e.add_file("OPENSUBTITLES_API_KEY=os-key\nOPENSUBTITLES_USERNAME=me\nOPENSUBTITLES_PASSWORD=pw-secret\n");
        let s = Settings::open(e.clone(), Some(db.clone())).await.unwrap();
        let st = s.stored();
        assert_eq!(st.get("tmdb.api_key"), Some(&json!("tmdb-secret-key-0123456789abcd")));
        assert_eq!(st.get("llm.api_key"), None, "keys are kept bound, not as a setting");
        assert_eq!(
            crate::llm::keys::find(&st, "anthropic", "https://api.anthropic.com")
                .unwrap()
                .0
                .expose(),
            "sk-ant-secret-wxyz",
            "the referenced key variable is resolved"
        );
        assert_eq!(
            st.get("opensubtitles.password"),
            Some(&json!("pw-secret")),
            "the credentials file too"
        );
        assert_eq!(st.get("metadata.country"), Some(&json!("AU")));
        assert_eq!(st.get("performance.jobs"), Some(&json!(3)));
        assert_eq!(st.get("features.convert"), Some(&json!(false)));
        assert_eq!(st.get("features.db"), None, "env-only switches aren't imported");
        assert_eq!(st.get("metadata.language"), None, "defaults aren't imported");
        assert_eq!(st.libraries().len(), 2);
        assert_eq!(st.libraries()[1].roots[0], Root::local("/media/tv shows"));
        // Same configuration as from the environment alone.
        assert!(changed_keys(&Config::resolve(&e, &Stored::default()), s.boot()).is_empty());

        // The document never carries a secret.
        let doc = s.document().to_string();
        for secret in ["tmdb-secret-key", "sk-ant-secret", "pw-secret", "os-key"] {
            assert!(!doc.contains(secret), "{secret} leaked");
        }
        let d = s.document();
        assert_eq!(d["settings"]["tmdb"]["api_key"], json!({"set": true, "last4": "abcd"}));
        assert_eq!(d["settings"]["opensubtitles"]["password"], json!({"set": true}));
        assert_eq!(d["settings"]["opensubtitles"]["username"], json!("me"));
        assert_eq!(d["fields"]["opensubtitles.username"]["source"], json!("file"));
        assert_eq!(d["fields"]["metadata.country"]["env"], json!("MOONRISE_TMDB_COUNTRY"));
        assert_eq!(d["fields"]["llm.api_key"]["env"], json!("ANTHROPIC_API_KEY"));
        assert_eq!(d["libraries"]["managed_by"], json!("env"));

        // Not again: a store emptied later stays empty (and the old libraries table isn't read).
        db.call(|c| Ok(c.execute("DELETE FROM settings", [])?)).await.unwrap();
        let s = Settings::open(e, Some(db.clone())).await.unwrap();
        assert!(s.stored().is_empty());

        // Without MOONRISE_LIBRARIES the import takes the libraries the database ran with.
        let db = Db::memory().unwrap();
        db.call(|c| {
            c.execute(
                "INSERT INTO libraries(id, name, kind, roots, sort) VALUES ('old', 'Old', 'movies', '[\"/media/old\"]', 0)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let s = Settings::open(env(&[]), Some(db)).await.unwrap();
        assert_eq!(
            s.current().libraries,
            vec![LibraryConfig {
                id: "old".into(),
                name: "Old".into(),
                kind: "movies".into(),
                roots: vec!["/media/old".into()]
            }]
        );
        assert_eq!(s.document()["libraries"]["managed_by"], json!("store"));
    }

    #[tokio::test]
    async fn update_applies_and_flags_restarts() {
        let db = Db::memory().unwrap();
        let s = Settings::open(env(&[("MOONRISE_TMDB_COUNTRY", "AU")]), Some(db)).await.unwrap();
        let (before, after) = s
            .update(vec![
                ("performance.jobs".into(), Some(json!(9))),
                ("metadata.language".into(), Some(json!("fr"))),
                ("metadata.country".into(), Some(json!("FR"))),
                ("tmdb.api_key".into(), Some(json!("new-key-0123456789"))),
            ])
            .await
            .unwrap();
        assert_eq!(before.jobs, 4);
        assert_eq!(after.jobs, 9);
        assert_eq!(after.tmdb_language, "fr");
        assert_eq!(after.tmdb_country, "AU", "still the environment's");
        let changed = changed_keys(&before, &after);
        assert!(changed.contains("tmdb.api_key") && changed.contains("performance.jobs") && !changed.contains("metadata.country"));
        let d = s.document();
        assert_eq!(d["restart_required"], json!(["performance.jobs"]));
        assert_eq!(d["fields"]["performance.jobs"]["restart_pending"], json!(true));
        assert_eq!(d["fields"]["metadata.country"]["source"], json!("env"));
        // Removing the value goes back to the default (and nothing is pending any more).
        let (_, after) = s.update(vec![("performance.jobs".into(), None)]).await.unwrap();
        assert_eq!(after.jobs, 4);
        assert_eq!(s.document()["restart_required"], json!([]));
    }
}
