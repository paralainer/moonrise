//! `GET /v1/admin/services` (admins): every optional service at a glance, for the apps' server
//! screens: whether it is set up (the admin's own credentials, the project's built-in key, or
//! nothing), and in plain words what Moonrise does with it and without it. Nothing secret is answered.
//!
//! ```json
//! {"services": [{"id": "tmdb", "name": "TMDB", "state": "builtin", "source": "builtin",
//!   "setting": "tmdb.api_key", "summary": "…", "with": "…", "without": "…"}, …]}
//! ```
//!
//! `state`: `configured` (the admin's own credentials, from the settings or the environment),
//! `builtin` (the project's key in this release), `not_set` (the fallback is in use), `off`
//! (switched off by a feature switch). Per service also: `opensubtitles` its `quota` (tier,
//! downloads left, reset) and `account` (whether downloads count against an account);
//! `llm` the `model` and how many titles wait in Fix Match (`review`); `remote_access` the
//! server's Tailscale address when it has one; `ratings` (IMDb, Rotten Tomatoes, Metacritic) how
//! many titles have them and the attribution to show.

use std::{collections::HashMap, net::IpAddr, sync::Arc};

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};

use crate::{
    admin::config::{Config, Env, Feature},
    admin::settings::{self, Stored},
    api::{ApiError, AppState},
    subtitles::opensubtitles::{Quota, ACCOUNT_DOWNLOADS, ANONYMOUS_DOWNLOADS},
};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/v1/admin/services", get(services))
}

async fn services(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    crate::auth::admin_caller(&st, &headers, &q).await?;
    let cur = st.settings.current();
    let quota = if st.opensubtitles.configured() {
        Some(st.opensubtitles.quota().await)
    } else {
        None
    };
    let review = match &st.library {
        Some(l) => l.review_count().await.unwrap_or(0),
        None => 0,
    };
    let stored = st.settings.stored();
    let mut v = describe(
        &cur,
        st.settings.env(),
        &stored,
        quota.as_ref(),
        st.opensubtitles.has_account(),
        review,
        tailnet_address(),
    );
    let status = match &st.library {
        Some(l) => Some(l.ratings.status().await),
        None => None,
    };
    add_ratings(&mut v, ratings(&cur, st.settings.env(), &stored, status.as_ref()));
    Ok(Json(v))
}

/// Puts the ratings entry before remote access (the last).
fn add_ratings(doc: &mut Value, entry: Value) {
    if let Some(list) = doc["services"].as_array_mut() {
        let at = list.iter().position(|s| s["id"] == "remote_access").unwrap_or(list.len());
        list.insert(at, entry);
    }
}

/// Ratings beside TMDB's (`library/ratings.rs`): IMDb's dataset needs nothing; an OMDb key adds
/// Rotten Tomatoes and Metacritic. Also `imdb_titles`, `critic_titles` (titles with those),
/// `updated_at` (IMDb's dataset last read) and the `attribution` the apps show.
pub fn ratings(c: &Config, env: &Env, stored: &Stored, status: Option<&crate::library::ratings::Status>) -> Value {
    let (state, src) = key_state(env, stored, "omdb.api_key", c.omdb_key.expose());
    let state = if !c.enabled(Feature::Ratings) || !c.enabled(Feature::Library) {
        "off"
    } else {
        state
    };
    let refused = status.and_then(|s| s.key_refused.clone());
    let summary = match (state, &refused) {
        ("off", _) => "Switched off: only TMDB's ratings are shown.".to_string(),
        (_, Some(why)) => format!("IMDb ratings from IMDb's dataset. OMDb refused your key ({why}): check it, or enter another."),
        ("configured", None) if status.is_some_and(|s| s.limited) => {
            "IMDb ratings from IMDb's dataset; Rotten Tomatoes and Metacritic through your OMDb key (today's limit reached: more tomorrow)."
                .to_string()
        }
        ("configured", None) => "IMDb ratings from IMDb's dataset; Rotten Tomatoes and Metacritic through your OMDb key.".to_string(),
        _ => "IMDb ratings from IMDb's dataset. Add a free OMDb key for Rotten Tomatoes and Metacritic.".to_string(),
    };
    let mut v = json!({
        "id": "ratings", "name": "Ratings", "state": state, "source": src, "setting": "omdb.api_key",
        "summary": summary,
        "with": "Title pages show IMDb, Rotten Tomatoes and Metacritic beside TMDB. A free OMDb key (omdbapi.com) allows 1,000 look-ups a day; the server uses at most 500, a few seconds apart, and looks each title up again every two weeks.",
        "without": "Title pages show IMDb's rating (from IMDb's dataset, read about once a week) and TMDB's.",
        "attribution": crate::library::ratings::ATTRIBUTION,
    });
    if let Some(s) = status {
        v["imdb_titles"] = json!(s.imdb_titles);
        v["critic_titles"] = json!(s.critic_titles);
        v["updated_at"] = json!(s.dataset_at.map(crate::library::rfc3339));
    }
    v
}

/// The server's address on a tailnet (Tailscale's 100.64.0.0/10), if it has one.
pub fn tailnet_address() -> Option<String> {
    if_addrs::get_if_addrs().ok()?.into_iter().find_map(|i| match i.ip() {
        IpAddr::V4(v4) if v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]) => Some(v4.to_string()),
        _ => None,
    })
}

/// A key setting's state: `configured` (env, file or store), `builtin`, or `not_set`.
fn key_state(env: &Env, stored: &Stored, key: &str, value: &str) -> (&'static str, &'static str) {
    let (src, _) = settings::source(env, stored, key);
    let state = match src {
        _ if value.is_empty() => "not_set",
        "builtin" => "builtin",
        "default" => "not_set",
        _ => "configured",
    };
    (state, src)
}

/// The document (pure: what the server knows is passed in).
pub fn describe(
    c: &Config,
    env: &Env,
    stored: &Stored,
    quota: Option<&Quota>,
    account: bool,
    review: i64,
    tailnet: Option<String>,
) -> Value {
    let mut out = Vec::new();

    // TMDB.
    let (state, src) = key_state(env, stored, "tmdb.api_key", c.tmdb_key.expose());
    let state = if c.enabled(Feature::Library) { state } else { "off" };
    out.push(json!({
        "id": "tmdb", "name": "TMDB", "state": state, "source": src, "setting": "tmdb.api_key",
        "summary": match state {
            "configured" => "Using your TMDB key.",
            "builtin" => "Using Moonrise's built-in TMDB key.",
            "off" => "The library is switched off.",
            _ => "Not set up: titles come from file names and your folders.",
        },
        "with": "Titles are matched on TMDB: posters, backdrops, plots, cast, genres, ratings and episode names.",
        "without": "Titles and years come from file and folder names (and NFO files or the files' own tags); artwork from images in your folders (poster.jpg, fanart.jpg…), cover art inside the files, or a frame of the video.",
    }));

    // OpenSubtitles.
    let (state, src) = key_state(env, stored, "opensubtitles.api_key", c.opensubtitles.api_key.expose());
    let state = if c.enabled(Feature::OpenSubtitles) { state } else { "off" };
    let mut os = json!({
        "id": "opensubtitles", "name": "OpenSubtitles", "state": state, "source": src, "setting": "opensubtitles.api_key",
        "account": account,
        "with": format!("Find Subtitles searches OpenSubtitles.com; a subtitle fetched once plays on every device. Without an account: {ANONYMOUS_DOWNLOADS} downloads a day; with a free account {ACCOUNT_DOWNLOADS}; VIP accounts more."),
        "without": "Find Subtitles is hidden. Subtitles inside your files and next to them still play.",
    });
    os["summary"] = json!(match (state, account) {
        ("off", _) => "Subtitle search is switched off.".to_string(),
        ("not_set", _) => "Not set up: subtitle search is hidden.".to_string(),
        (_, true) => format!("Signed in to {}'s OpenSubtitles account.", c.opensubtitles.username.expose()),
        (s, false) => format!(
            "{} key, no account: {ANONYMOUS_DOWNLOADS} downloads a day. Add an OpenSubtitles account for {ACCOUNT_DOWNLOADS} a day.",
            if s == "builtin" { "Moonrise's built-in" } else { "Your" }
        ),
    });
    if let Some(q) = quota {
        os["quota"] = json!(q);
    }
    out.push(os);

    // LLM.
    let state = if !c.enabled(Feature::Llm) || !c.llm.allowed {
        "off"
    } else if c.llm.enabled {
        "configured"
    } else {
        "not_set"
    };
    let provider = crate::llm::preset_or_custom(&c.llm.provider);
    let model = if c.llm.model.is_empty() {
        "the model it serves".to_string()
    } else {
        c.llm.model.clone()
    };
    out.push(json!({
        "id": "llm", "name": "AI matching", "state": state, "source": settings::source(env, stored, "llm.api_key").0, "setting": "llm.api_key",
        "model": (state == "configured").then(|| c.llm.model.clone()),
        "provider": provider.id, "provider_name": provider.name,
        "review": review,
        "summary": match state {
            "configured" if provider.id == "anthropic" => format!("Unclear file names are matched with {model}."),
            "configured" => format!("Unclear file names are matched with {model} ({}).", provider.name),
            "off" => "Switched off: unclear file names go to Fix Match.".to_string(),
            _ => "Not set up: unclear file names go to Fix Match.".to_string(),
        },
        "with": "A language model reads unclear or transliterated file names and picks among TMDB's candidates; every answer is checked (year, runtime, title).",
        "without": "Titles are matched by name, year and runtime; the ones that stay unclear wait in Fix Match with their likely candidates for you to pick.",
    }));

    // Remote access.
    out.push(json!({
        "id": "remote_access", "name": "Remote access", "state": if tailnet.is_some() { "configured" } else { "not_set" },
        "source": "tailscale", "address": tailnet,
        "summary": if tailnet.is_some() { "On Tailscale: the apps reach the server away from home." } else { "Home network only." },
        "with": "Put the server's computer and your devices on Tailscale (free): the apps reach the server from anywhere, with nothing opened to the internet.",
        "without": "The apps reach the server on your home network only; downloads still play anywhere.",
    }));
    json!({"services": out})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::config::{Builtin, Secret};

    fn find<'a>(v: &'a Value, id: &str) -> &'a Value {
        v["services"].as_array().unwrap().iter().find(|s| s["id"] == id).unwrap()
    }

    #[test]
    fn states_and_fallbacks() {
        // Nothing at all.
        let env = Env::from_pairs([("MOONRISE_NAME", "x")]);
        let c = Config::resolve(&env, &Stored::default());
        let v = describe(&c, &env, &Stored::default(), None, false, 3, None);
        assert_eq!(find(&v, "tmdb")["state"], "not_set");
        assert_eq!(find(&v, "opensubtitles")["state"], "not_set");
        assert_eq!(find(&v, "llm")["state"], "not_set");
        assert_eq!(find(&v, "llm")["review"], 3);
        assert_eq!(find(&v, "remote_access")["state"], "not_set");
        // Built-in keys, an anonymous quota.
        let mut env = Env::from_pairs([("MOONRISE_NAME", "x")]);
        env.builtin = Builtin {
            tmdb: Secret::new("t-built"),
            opensubtitles: Secret::new("o-built"),
        };
        let c = Config::resolve(&env, &Stored::default());
        let q = Quota {
            tier: "anonymous",
            allowed: Some(5),
            remaining: Some(3),
            reset_at: None,
            account_allowed: 20,
        };
        let v = describe(&c, &env, &Stored::default(), Some(&q), false, 0, Some("100.101.1.2".into()));
        assert_eq!(
            (find(&v, "tmdb")["state"].as_str(), find(&v, "tmdb")["source"].as_str()),
            (Some("builtin"), Some("builtin"))
        );
        let os = find(&v, "opensubtitles");
        assert_eq!(os["state"], "builtin");
        assert_eq!(os["quota"]["remaining"], 3);
        assert!(os["summary"].as_str().unwrap().contains("Add an OpenSubtitles account"));
        assert_eq!(find(&v, "remote_access")["address"], "100.101.1.2");
        assert!(!v.to_string().contains("built\""), "no key in the answer");
        // The admin's own key wins and is "configured"; the switch turns it off.
        let store = Stored::from_pairs([("tmdb.api_key", json!("mine")), ("features.opensubtitles", json!(false))]);
        let c = Config::resolve(&env, &store);
        let v = describe(&c, &env, &store, None, false, 0, None);
        assert_eq!(find(&v, "tmdb")["state"], "configured");
        assert_eq!(find(&v, "opensubtitles")["state"], "off");
    }

    #[test]
    fn ratings_entry() {
        use crate::library::ratings::Status;
        let env = Env::from_pairs([("MOONRISE_NAME", "x")]);
        let c = Config::resolve(&env, &Stored::default());
        let mut doc = describe(&c, &env, &Stored::default(), None, false, 0, None);
        let r = ratings(
            &c,
            &env,
            &Stored::default(),
            Some(&Status {
                imdb_titles: 12,
                ..Default::default()
            }),
        );
        assert_eq!(r["state"], "not_set");
        assert!(r["summary"].as_str().unwrap().contains("Add a free OMDb key"));
        assert!(r["attribution"].as_str().unwrap().contains("Information courtesy of IMDb"));
        assert_eq!(r["imdb_titles"], 12);
        add_ratings(&mut doc, r);
        let ids: Vec<&str> = doc["services"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["tmdb", "opensubtitles", "llm", "ratings", "remote_access"]);
        // A key, then one OMDb refused, then the switch off.
        let store = Stored::from_pairs([("omdb.api_key", json!("omdb-key-123456"))]);
        let c = Config::resolve(&env, &store);
        let r = ratings(&c, &env, &store, None);
        assert_eq!((r["state"].as_str(), r["source"].as_str()), (Some("configured"), Some("store")));
        assert!(!r.to_string().contains("omdb-key"), "no key in the answer");
        let refused = Status {
            key_refused: Some("Invalid API key!".into()),
            ..Default::default()
        };
        assert!(ratings(&c, &env, &store, Some(&refused))["summary"]
            .as_str()
            .unwrap()
            .contains("refused"));
        let store = Stored::from_pairs([("features.ratings", json!(false))]);
        let c = Config::resolve(&env, &store);
        assert_eq!(ratings(&c, &env, &store, None)["state"], "off");
    }
}
