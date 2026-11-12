//! The API contract: every route of the server's router, the admin API included, is
//! described in `api/openapi.yaml` (at the repository's root), and the spec names no route the
//! router lacks.
//!
//! The routes are read from the router itself: axum keeps no public list, but its `Debug` output
//! has the path of every route (`RouteId(n): "/v1/…"`) and, per route, which methods it has a
//! handler for (`MethodRouter { get: …, post: None, … }`). If a new axum prints them otherwise,
//! this test fails saying so (it never passes on an empty list).
//!
//! The spec is read line by line, in the block style it is written in: a path is a line
//! `  /v1/…:` under `paths:`, its operations the lines `    get:`, `    post:`… under it. The
//! spec's validity as OpenAPI is checked apart (`scripts/api-lint.sh`, in `make check`).

use std::collections::BTreeSet;

use super::*;

const METHODS: [&str; 9] = ["get", "head", "delete", "options", "patch", "post", "put", "trace", "connect"];

/// `(method, path)` of every route in the router's `Debug` output (methods in lower case; HEAD
/// is left out: axum answers it from GET).
fn router_routes(debug: &str) -> BTreeSet<(String, String)> {
    // The routes proper; the fallback router after them has routes of its own, numbered apart.
    let main = debug
        .split_once(", fallback_router: ")
        .map(|(a, _)| a)
        .expect("axum's Router Debug output has changed (no fallback_router field)");
    // RouteId(n): "/path"
    let mut paths = HashMap::new();
    for part in main.split("RouteId(").skip(1) {
        let Some((id, rest)) = part.split_once("): ") else { continue };
        if let Some(path) = rest.strip_prefix('"').and_then(|r| r.split_once('"')).map(|(p, _)| p) {
            paths.insert(id.to_string(), path.to_string());
        }
    }
    // RouteId(n): MethodRouter(MethodRouter { get: Route, head: None, … })
    let mut out = BTreeSet::new();
    for part in main.split("RouteId(").skip(1) {
        let Some((id, rest)) = part.split_once("): MethodRouter(MethodRouter { ") else {
            continue;
        };
        let path = paths
            .get(id)
            .unwrap_or_else(|| panic!("route {id} has methods but no path in the router's Debug output"));
        if path.contains("__private__axum") {
            continue;
        }
        let fields = rest.split(", fallback: ").next().unwrap_or("");
        for field in fields.split(", ") {
            let (name, value) = field.split_once(": ").unwrap_or((field, ""));
            assert!(
                METHODS.contains(&name),
                "axum's MethodRouter Debug output has changed: {name:?} where a method was expected"
            );
            if value != "None" && name != "head" {
                out.insert((name.to_string(), path.clone()));
            }
        }
    }
    out
}

/// `(method, path)` of every operation in the spec.
fn spec_routes(yaml: &str) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    let mut in_paths = false;
    let mut path: Option<String> = None;
    for line in yaml.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if !line.starts_with(' ') {
            in_paths = line.trim_end() == "paths:";
            path = None;
            continue;
        }
        if !in_paths {
            continue;
        }
        if let Some(p) = line.strip_prefix("  /").and_then(|p| p.trim_end().strip_suffix(':')) {
            path = Some(format!("/{}", p.trim_matches(|c| c == '"' || c == '\'')));
        } else if let Some(m) = line.strip_prefix("    ").and_then(|m| m.trim_end().strip_suffix(':')) {
            if let (Some(p), true) = (&path, METHODS.contains(&m)) {
                out.insert((m.to_string(), p.clone()));
            }
        }
    }
    out
}

fn show(routes: &BTreeSet<(String, String)>) -> String {
    routes
        .iter()
        .map(|(m, p)| format!("  {} {p}", m.to_uppercase()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn spec_parsing() {
    let yaml = "openapi: 3.1.0\npaths:\n  /health:\n    get:\n      summary: x\n  /v1/items/{id}/played:\n    \
                parameters: []\n    put:\n      responses: {}\n    delete:\n      responses: {}\ncomponents:\n  /not-a-path:\n    get:\n";
    let got = spec_routes(yaml);
    let want: BTreeSet<_> = [
        ("get", "/health"),
        ("put", "/v1/items/{id}/played"),
        ("delete", "/v1/items/{id}/played"),
    ]
    .into_iter()
    .map(|(m, p)| (m.to_string(), p.to_string()))
    .collect();
    assert_eq!(got, want);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_route_is_in_the_spec() {
    let t = std::env::temp_dir().join(format!("moonrise-contract-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    std::fs::create_dir_all(&t).unwrap();
    let env = crate::admin::config::Env::from_pairs([
        ("MOONRISE_DATA_DIR".to_string(), t.join("data").to_string_lossy().to_string()),
        ("MOONRISE_CACHE_DIR".to_string(), t.join("cache").to_string_lossy().to_string()),
        ("MOONRISE_DISABLE".to_string(), "mdns".to_string()),
    ]);
    let st = Arc::new(AppState::new(env).await.unwrap());
    let routes = router_routes(&format!("{:?}", router(st)));
    let _ = std::fs::remove_dir_all(&t);
    assert!(
        routes.len() > 100
            && routes.contains(&("get".into(), "/v1/items/{id}/playback".into()))
            && routes.contains(&("get".into(), "/".into()))
            && routes.contains(&("patch".into(), "/v1/admin/settings".into())),
        "couldn't read the routes from the router's Debug output (axum changed it?): {} found",
        routes.len()
    );

    let spec_file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../api/openapi.yaml");
    let yaml = std::fs::read_to_string(&spec_file).unwrap_or_else(|e| panic!("{}: {e}", spec_file.display()));
    let spec = spec_routes(&yaml);

    let missing: BTreeSet<_> = routes.difference(&spec).cloned().collect();
    assert!(
        missing.is_empty(),
        "routes missing from api/openapi.yaml (describe them there):\n{}",
        show(&missing)
    );
    let unknown: BTreeSet<_> = spec.difference(&routes).cloned().collect();
    assert!(
        unknown.is_empty(),
        "api/openapi.yaml describes routes the server doesn't have:\n{}",
        show(&unknown)
    );
}
