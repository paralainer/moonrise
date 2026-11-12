//! Where an LLM key may be sent.
//!
//! Every key the settings keep is **bound**, when it is saved, to a provider and an origin
//! (scheme, host and port, as the HTTP client connects to them). Keys live in `llm.keys` (a list
//! of `{provider, origin, key}`; not a [`crate::admin::settings::KEYS`] setting: never answered, never
//! written by a settings PATCH directly). The configuration in effect takes the key bound to its
//! own provider and address, if there is one ([`crate::admin::config::Config::resolve`]); a key given
//! with a settings change is bound to the provider and address in effect *after* that change.
//!
//! The request layer ([`crate::llm::Llm`]) checks every outbound request against the key's
//! binding and attaches the key only when the request's provider and origin are the binding's.
//! So changing the address sets a key aside (it stays bound to the old address) rather than
//! moving it, and it comes back if the address does. A key from the environment is bound the
//! same way, to the environment's provider and address (`config.rs`).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    admin::config::{Config, Env, Secret},
    admin::settings::Stored,
};

pub const VAULT_KEY: &str = "llm.keys";
/// The setting a key used to be stored under (before keys were bound): migrated into the vault.
const LEGACY_KEY: &str = "llm.api_key";
const SAVED_KEY: &str = "llm.saved";

/// A provider and an origin a key may be sent to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub provider: String,
    pub origin: String,
}

/// `scheme://host:port` of an http(s) URL, as the HTTP client will connect to it (the port
/// filled in). None for anything else, and for a URL with a user name or password in it.
pub fn origin_of(url: &str) -> Option<String> {
    origin_of_url(&reqwest::Url::parse(url.trim()).ok()?)
}

pub fn origin_of_url(u: &reqwest::Url) -> Option<String> {
    if !matches!(u.scheme(), "http" | "https") || !u.username().is_empty() || u.password().is_some() {
        return None;
    }
    let host = u.host_str().filter(|h| !h.is_empty())?;
    Some(format!(
        "{}://{}:{}",
        u.scheme(),
        host.to_ascii_lowercase(),
        u.port_or_known_default()?
    ))
}

impl Binding {
    /// `provider` at the origin of `url` (None: not an http(s) address a key can be bound to).
    pub fn new(provider: &str, url: &str) -> Option<Binding> {
        Some(Binding {
            provider: provider.to_string(),
            origin: origin_of(url)?,
        })
    }

    /// Whether a request of `provider` to `url` may carry what is bound here.
    pub fn allows(&self, provider: &str, url: &reqwest::Url) -> bool {
        self.provider == provider && origin_of_url(url).as_deref() == Some(self.origin.as_str())
    }

    /// The same, for an address not yet parsed.
    pub fn allows_str(&self, provider: &str, url: &str) -> bool {
        reqwest::Url::parse(url.trim()).is_ok_and(|u| self.allows(provider, &u))
    }
}

/// A kept key. `Debug` shows the binding only.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(flatten)]
    pub binding: Binding,
    pub key: String,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry").field("binding", &self.binding).finish_non_exhaustive()
    }
}

/// The keys kept.
pub fn entries(stored: &Stored) -> Vec<Entry> {
    stored
        .get(VAULT_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

/// The key bound to `provider` at the address `base_url`, with its binding.
pub fn find(stored: &Stored, provider: &str, base_url: &str) -> Option<(Secret, Binding)> {
    let want = Binding::new(provider, base_url)?;
    entries(stored)
        .into_iter()
        .find(|e| e.binding == want && !e.key.is_empty())
        .map(|e| (Secret::new(e.key), e.binding))
}

/// The keys kept for `provider`, any address.
pub fn for_provider(stored: &Stored, provider: &str) -> Vec<Entry> {
    entries(stored).into_iter().filter(|e| e.binding.provider == provider).collect()
}

/// The change that keeps `key` for `binding` (None: forgets the key kept for it).
pub fn change(mut list: Vec<Entry>, binding: &Binding, key: Option<String>) -> (String, Option<Value>) {
    list.retain(|e| &e.binding != binding);
    if let Some(k) = key.filter(|k| !k.is_empty()) {
        list.push(Entry {
            binding: binding.clone(),
            key: k,
        });
    }
    (
        VAULT_KEY.into(),
        (!list.is_empty()).then(|| serde_json::to_value(&list).expect("keys")),
    )
}

/// The changes that move keys stored before they were bound into the vault: the key of
/// `llm.api_key` is bound to the provider and address the server used it with (the
/// configuration in effect), a key put aside per provider in `llm.saved` to that provider at the
/// address kept with it (else the preset's). Empty when there is nothing to move.
pub fn migration(env: &Env, stored: &Stored) -> Vec<(String, Option<Value>)> {
    let mut list = entries(stored);
    let mut out = Vec::new();
    let add = |list: &mut Vec<Entry>, b: Option<Binding>, key: &str| {
        if let Some(b) = b {
            if !key.is_empty() && !list.iter().any(|e| e.binding == b) {
                list.push(Entry {
                    binding: b,
                    key: key.to_string(),
                });
            }
        }
    };
    let mut moved = false;
    if let Some(v) = stored.get(LEGACY_KEY) {
        let c = Config::resolve(env, stored).llm;
        add(&mut list, Binding::new(&c.provider, &c.base_url), v.as_str().unwrap_or("").trim());
        out.push((LEGACY_KEY.to_string(), None));
        moved = true;
    }
    if let Some(saved) = stored.get(SAVED_KEY).and_then(Value::as_object) {
        let mut kept: Map<String, Value> = saved.clone();
        for (p, s) in saved {
            let Some(key) = s.get(LEGACY_KEY).and_then(Value::as_str) else {
                continue;
            };
            let base = s
                .get("llm.base_url")
                .and_then(Value::as_str)
                .map_or_else(|| crate::llm::preset_or_custom(p).base_url.to_string(), String::from);
            add(&mut list, Binding::new(p, &base), key.trim());
            if let Some(o) = kept.get_mut(p).and_then(Value::as_object_mut) {
                o.remove(LEGACY_KEY);
                if o.is_empty() {
                    kept.remove(p);
                }
            }
            moved = true;
        }
        if moved {
            out.push((SAVED_KEY.to_string(), (!kept.is_empty()).then_some(Value::Object(kept))));
        }
    }
    if moved {
        out.push((
            VAULT_KEY.to_string(),
            (!list.is_empty()).then(|| serde_json::to_value(&list).expect("keys")),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn origins() {
        assert_eq!(
            origin_of("https://API.Anthropic.com/v1/").as_deref(),
            Some("https://api.anthropic.com:443")
        );
        assert_eq!(origin_of("https://api.anthropic.com:443"), origin_of("https://api.anthropic.com"));
        assert_eq!(origin_of("http://192.168.1.5:8080/v1").as_deref(), Some("http://192.168.1.5:8080"));
        // As the client connects: a decimal IP is that address.
        assert_eq!(origin_of("http://3232235777:80/").as_deref(), Some("http://192.168.1.1:80"));
        for bad in [
            "nope",
            "ftp://x/",
            "https://api.anthropic.com@catcher/",
            "https://u:p@api.anthropic.com/",
            "file:///etc",
        ] {
            assert_eq!(origin_of(bad), None, "{bad}");
        }
        assert_ne!(
            origin_of("https://api.anthropic.com./"),
            origin_of("https://api.anthropic.com/"),
            "strict: no key rather than a guess"
        );
        let b = Binding::new("anthropic", "https://api.anthropic.com").unwrap();
        assert!(b.allows_str("anthropic", "https://api.anthropic.com/v1/messages"));
        assert!(!b.allows_str("custom", "https://api.anthropic.com/v1/messages"));
        assert!(!b.allows_str("anthropic", "http://api.anthropic.com/v1/messages"));
        assert!(!b.allows_str("anthropic", "https://api.anthropic.com:8443/v1/messages"));
        assert!(!b.allows_str("anthropic", "https://api.anthropic.com@catcher/v1/messages"));
        assert!(!b.allows_str("anthropic", "https://api.anthropic.com.catcher/v1/messages"));
    }

    #[test]
    fn keys_stored_before_binding_are_bound_where_they_were_used() {
        let store = Stored::from_pairs([
            ("llm.api_key", json!("sk-ant")),
            (
                SAVED_KEY,
                json!({"custom": {"llm.api_key": "kc", "llm.base_url": "http://gw:1/v1", "llm.model": "m"}, "openrouter": {"llm.api_key": "ko"}}),
            ),
        ]);
        let changes = migration(&Env::default(), &store);
        let after = store.with(&changes);
        assert!(after.get(LEGACY_KEY).is_none());
        assert_eq!(
            after.get(SAVED_KEY),
            Some(&json!({"custom": {"llm.base_url": "http://gw:1/v1", "llm.model": "m"}}))
        );
        assert_eq!(find(&after, "anthropic", "https://api.anthropic.com").unwrap().0.expose(), "sk-ant");
        assert_eq!(find(&after, "custom", "http://gw:1/v2").unwrap().0.expose(), "kc");
        assert_eq!(find(&after, "openrouter", "https://openrouter.ai/api/v1").unwrap().0.expose(), "ko");
        assert!(find(&after, "anthropic", "http://catcher:1").is_none());
        assert!(migration(&Env::default(), &after).is_empty(), "once");
        // Stored with an address: bound to it.
        let store = Stored::from_pairs([
            ("llm.api_key", json!("k")),
            ("llm.provider", json!("custom")),
            ("llm.base_url", json!("http://gw:1/v1")),
        ]);
        let after = store.with(&migration(&Env::default(), &store));
        assert_eq!(
            entries(&after)[0].binding,
            Binding {
                provider: "custom".into(),
                origin: "http://gw:1".into()
            }
        );
        assert!(!format!("{:?}", entries(&after)).contains("\"k\""), "Debug shows no key");
    }

    #[test]
    fn changes() {
        let b = Binding::new("custom", "http://a:1/v1").unwrap();
        let (k, v) = change(Vec::new(), &b, Some("k1".into()));
        assert_eq!(k, VAULT_KEY);
        let s = Stored::from_pairs([(VAULT_KEY, v.unwrap())]);
        assert_eq!(find(&s, "custom", "http://a:1/x").unwrap().0.expose(), "k1");
        let (_, v) = change(entries(&s), &b, None);
        assert_eq!(v, None);
    }
}
