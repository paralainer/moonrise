//! Fix Match's candidate posters, served by the server (capability `match-posters`), so
//! the apps talk to nothing else ([ADR 0013](../../../docs/adr/0013-the-server-is-the-only-integration-point.md)).
//!
//! Candidates are TMDB search results, not titles of the library, so their art isn't in `images`:
//! registering them would keep a master of every result anybody ever looked at. A poster is
//! fetched when a Fix Match row asks for it and kept in the segment cache (under
//! `match-poster/`), which evicts it with everything else and so bounds it.
//!
//! This is not a proxy: `GET /v1/admin/match/poster` takes a TMDB file path of the one form TMDB
//! gives (`/name.jpg`) and one of [`SIZES`], and the address is built here from a constant. No
//! redirect is followed, the answer is capped at [`MAX_BYTES`], and only image bytes are passed on.

#[cfg(test)]
use std::sync::Mutex;
use std::{sync::Arc, time::Duration};

use anyhow::{bail, Context, Result};
use bytes::{Bytes, BytesMut};
use serde_json::{json, Value};
use tokio::sync::Semaphore;

use crate::storage::cache::Cache;

const TMDB_IMAGES: &str = "https://image.tmdb.org/t/p";
/// The size of the full URLs in candidates (`identify.rs`), stored ones included.
const CANDIDATE_SIZE: &str = "w342";
/// The sizes that can be asked for.
pub const SIZES: [&str; 2] = ["w185", CANDIDATE_SIZE];
/// More than any poster at these sizes.
const MAX_BYTES: usize = 4 << 20;
const MAX_NAME: usize = 64;

/// A file path as TMDB gives them: `/`, a name of letters, digits, `-` and `_`, an image extension.
pub fn valid_path(path: &str) -> bool {
    let Some((name, ext)) = path.strip_prefix('/').and_then(|p| p.rsplit_once('.')) else {
        return false;
    };
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && matches!(ext, "jpg" | "jpeg" | "png" | "webp")
}

/// The file path in a candidate's `poster` URL (None: not a TMDB poster of the expected form).
fn path_of_url(url: &str) -> Option<&str> {
    url.strip_prefix(TMDB_IMAGES)?
        .strip_prefix('/')?
        .strip_prefix(CANDIDATE_SIZE)
        .filter(|p| valid_path(p))
}

/// Candidates for an app that asked for `server_posters=1`: `poster` (a TMDB URL) gives way to
/// `poster_path`, for `GET /v1/admin/match/poster`. No address of a third party is left in them.
pub fn to_server_posters(candidates: &mut Value) {
    for c in candidates.as_array_mut().into_iter().flatten() {
        let Some(c) = c.as_object_mut() else { continue };
        let path = c
            .remove("poster")
            .and_then(|p| p.as_str().and_then(path_of_url).map(str::to_string));
        if let Some(path) = path {
            c.insert("poster_path".into(), json!(path));
        }
    }
}

/// What the bytes are, by their first bytes (None: not an image this route serves).
fn image_type(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

pub struct MatchPosters {
    http: reqwest::Client,
    cache: Arc<Cache>,
    /// Polite to TMDB: a page of candidates asks for a few at once.
    slots: Semaphore,
    #[cfg(test)]
    test_base: Mutex<Option<String>>,
}

impl MatchPosters {
    pub fn new(cache: Arc<Cache>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("http client");
        MatchPosters {
            http,
            cache,
            slots: Semaphore::new(3),
            #[cfg(test)]
            test_base: Mutex::new(None),
        }
    }

    /// The one address posters come from (tests stand a local server in for it).
    #[cfg_attr(not(test), allow(clippy::unused_self, reason = "only tests read self here"))]
    fn base(&self) -> String {
        #[cfg(test)]
        if let Some(b) = self.test_base.lock().unwrap().clone() {
            return b;
        }
        TMDB_IMAGES.to_string()
    }

    /// The poster's bytes and type. `path` and `size` are checked here too: nothing else builds
    /// the address.
    pub async fn get(&self, path: &str, size: &str) -> Result<(Bytes, &'static str)> {
        if !valid_path(path) || !SIZES.contains(&size) {
            bail!("not a TMDB poster");
        }
        let key = format!("match-poster/{size}{path}");
        if let Some(hit) = self.cached(&key).await {
            return Ok(hit);
        }
        let _slot = self.slots.acquire().await.context("shutting down")?;
        // It may have arrived while this request waited for its turn.
        if let Some(hit) = self.cached(&key).await {
            return Ok(hit);
        }
        let bytes = self.fetch(&format!("{}/{size}{path}", self.base())).await?;
        let ctype = image_type(&bytes).context("not an image")?;
        self.cache.put(&key, &bytes).await;
        Ok((bytes, ctype))
    }

    async fn cached(&self, key: &str) -> Option<(Bytes, &'static str)> {
        let b = self.cache.get(key).await?;
        let t = image_type(&b)?;
        Some((b, t))
    }

    async fn fetch(&self, url: &str) -> Result<Bytes> {
        let mut resp = self.http.get(url).send().await.context("poster request")?;
        if !resp.status().is_success() {
            bail!("HTTP {}", resp.status());
        }
        if resp.content_length().is_some_and(|n| n > MAX_BYTES as u64) {
            bail!("poster too large");
        }
        let mut out = BytesMut::new();
        while let Some(chunk) = resp.chunk().await.context("poster body")? {
            if out.len() + chunk.len() > MAX_BYTES {
                bail!("poster too large");
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out.freeze())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        convert::Infallible,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use axum::{
        body::{to_bytes, Body},
        extract::{Query, State},
        http::{header, HeaderMap, StatusCode},
        response::{IntoResponse, Redirect, Response},
        routing::get,
        Json, Router,
    };

    use super::*;
    use crate::{
        api::AppState,
        auth,
        library::api::{admin_review, match_poster},
    };

    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];
    /// The 4 MiB cap, written out: a test sized from [`MAX_BYTES`] would follow it wherever it went.
    const CAP: usize = 4 << 20;

    /// `len` bytes that pass for a JPEG, so only their size can get them refused.
    fn jpeg_of(len: usize) -> Vec<u8> {
        let mut b = JPEG.to_vec();
        b.resize(len, 0);
        b
    }

    /// The same answer with no Content-Length: only counting while reading can stop it.
    fn without_length(len: usize) -> Response {
        let chunks: Vec<_> = jpeg_of(len)
            .chunks(64 << 10)
            .map(|c| Ok::<_, Infallible>(Bytes::copy_from_slice(c)))
            .collect();
        Body::from_stream(futures_util::stream::iter(chunks)).into_response()
    }

    #[test]
    fn only_tmdb_file_paths_of_the_expected_form() {
        for good in ["/abc.jpg", "/qJ2tW6WMUDux911r6m7haRef0WH.jpg", "/a-b_C.png", "/x.webp", "/x.jpeg"] {
            assert!(valid_path(good), "{good}");
        }
        let long = format!("/{}.jpg", "a".repeat(MAX_NAME + 1));
        for bad in [
            "",
            "/",
            "abc.jpg",
            "/.jpg",
            "/abc",
            "/abc.svg",
            "/abc.JPG",
            "/abc.jpg?x=1",
            "/abc.jpg#f",
            "/a/b.jpg",
            "/../secret.jpg",
            "/..%2fsecret.jpg",
            "/a.b.jpg",
            "//evil.example/x.jpg",
            "https://evil.example/x.jpg",
            "/abc.jpg\n",
            "/abç.jpg",
            long.as_str(),
        ] {
            assert!(!valid_path(bad), "{bad:?}");
        }
    }

    #[test]
    fn only_a_tmdb_url_gives_a_path() {
        let mut v = json!([
            {"tmdb_id": 1, "poster": "https://image.tmdb.org/t/p/w342/abc.jpg"},
            {"tmdb_id": 2, "poster": null},
            {"tmdb_id": 3},
            {"tmdb_id": 4, "poster": "https://evil.example/t/p/w342/abc.jpg"},
            {"tmdb_id": 5, "poster": "https://image.tmdb.org/t/p/original/abc.jpg"},
            {"tmdb_id": 6, "poster": "https://image.tmdb.org/t/p/w342/a/../b.jpg"},
        ]);
        to_server_posters(&mut v);
        assert_eq!(v[0], json!({"tmdb_id": 1, "poster_path": "/abc.jpg"}));
        for (i, c) in v.as_array().unwrap().iter().enumerate().skip(1) {
            assert_eq!(*c, json!({"tmdb_id": i + 1}), "no poster and no third-party address");
        }
    }

    #[test]
    fn image_types_by_their_bytes() {
        assert_eq!(image_type(JPEG), Some("image/jpeg"));
        assert_eq!(image_type(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(image_type(b"RIFF\0\0\0\0WEBPrest"), Some("image/webp"));
        assert_eq!(image_type(b"<html>"), None);
        assert_eq!(image_type(b"<svg xmlns='x'/>"), None);
        assert_eq!(image_type(b""), None);
    }

    struct Upstream {
        hits: Arc<AtomicUsize>,
        base: String,
    }

    /// Stands in for TMDB's image host; `app` is told every request's path.
    async fn upstream(app: Router) -> Upstream {
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        let app = app.layer(axum::middleware::from_fn(
            move |r: axum::extract::Request, n: axum::middleware::Next| {
                let h = h.clone();
                async move {
                    h.fetch_add(1, Ordering::SeqCst);
                    n.run(r).await
                }
            },
        ));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        Upstream { hits, base }
    }

    struct Fixture {
        st: Arc<AppState>,
        admin: HeaderMap,
        user: HeaderMap,
    }

    async fn fixture(name: &str) -> Fixture {
        let t = std::env::temp_dir().join(format!("moonrise-match-posters-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        let env = crate::admin::config::Env::from_pairs([
            ("MOONRISE_DATA_DIR".to_string(), t.join("data").to_string_lossy().to_string()),
            ("MOONRISE_CACHE_DIR".to_string(), t.join("cache").to_string_lossy().to_string()),
            ("MOONRISE_DISABLE".to_string(), "mdns".to_string()),
            ("TMDB_API_KEY".to_string(), "not-a-real-key".to_string()),
        ]);
        let st = Arc::new(AppState::new(env).await.unwrap());
        let (a, u) = st
            .db
            .clone()
            .unwrap()
            .call(|c| {
                let a = auth::add_user(c, "boss", "password-1", true)?;
                let u = auth::add_user(c, "kid", "password-2", false)?;
                Ok((
                    auth::issue_token(c, &a.id, Some("p"), Some("Phone"))?,
                    auth::issue_token(c, &u.id, Some("i"), Some("Tablet"))?,
                ))
            })
            .await
            .unwrap();
        let bearer = |t: &str| {
            let mut h = HeaderMap::new();
            h.insert("authorization", format!("Bearer {t}").parse().unwrap());
            h
        };
        Fixture {
            admin: bearer(&a),
            user: bearer(&u),
            st,
        }
    }

    fn q(pairs: &[(&str, &str)]) -> Query<HashMap<String, String>> {
        Query(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
    }

    fn point_at(f: &Fixture, up: &Upstream) {
        *f.st.library.as_ref().unwrap().match_posters.test_base.lock().unwrap() = Some(up.base.clone());
    }

    async fn poster(f: &Fixture, h: &HeaderMap, query: &[(&str, &str)]) -> Result<Response, StatusCode> {
        match_poster(State(f.st.clone()), h.clone(), q(query)).await.map_err(|e| e.0)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn refuses_non_admins_and_anything_but_a_tmdb_path() {
        let f = fixture("refuses").await;
        let up = upstream(Router::new().fallback(|| async { JPEG })).await;
        point_at(&f, &up);
        let good: &[(&str, &str)] = &[("path", "/abc.jpg")];
        assert_eq!(poster(&f, &HeaderMap::new(), good).await.unwrap_err(), StatusCode::UNAUTHORIZED);
        assert_eq!(poster(&f, &f.user, good).await.unwrap_err(), StatusCode::FORBIDDEN);
        for bad in [
            "https://evil.example/x.jpg",
            "//evil.example/x.jpg",
            "/../../etc/passwd",
            "/a/b.jpg",
            "/abc.svg",
            "abc.jpg",
            "",
        ] {
            assert_eq!(
                poster(&f, &f.admin, &[("path", bad)]).await.unwrap_err(),
                StatusCode::BAD_REQUEST,
                "{bad}"
            );
        }
        assert_eq!(poster(&f, &f.admin, &[]).await.unwrap_err(), StatusCode::BAD_REQUEST);
        for bad in ["original", "w9999", "w342/../x", ""] {
            assert_eq!(
                poster(&f, &f.admin, &[("path", "/abc.jpg"), ("size", bad)]).await.unwrap_err(),
                StatusCode::BAD_REQUEST,
                "{bad}"
            );
        }
        // A caller-supplied address has no parameter to go in.
        let r = poster(
            &f,
            &f.admin,
            &[
                ("path", "/abc.jpg"),
                ("url", "http://evil.example/"),
                ("base", "http://evil.example/"),
            ],
        )
        .await
        .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(up.hits.load(Ordering::SeqCst), 1, "only the one good request reached the host");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn serves_the_poster_and_asks_tmdb_once() {
        let f = fixture("serves").await;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        let up = upstream(Router::new().fallback(move |uri: axum::http::Uri| {
            s.lock().unwrap().push(uri.to_string());
            async { JPEG }
        }))
        .await;
        point_at(&f, &up);
        let query: &[(&str, &str)] = &[("path", "/abc.jpg"), ("size", "w185")];
        for _ in 0..2 {
            let r = poster(&f, &f.admin, query).await.unwrap();
            assert_eq!(r.status(), StatusCode::OK);
            assert_eq!(r.headers()[header::CONTENT_TYPE], "image/jpeg");
            assert_eq!(r.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
            assert!(r.headers()[header::CACHE_CONTROL].to_str().unwrap().starts_with("private"));
            assert_eq!(to_bytes(r.into_body(), 1 << 20).await.unwrap(), JPEG);
        }
        assert_eq!(*seen.lock().unwrap(), ["/w185/abc.jpg"], "the second answer came from the cache");
        // The other size is another file; the default is the size candidates are in.
        poster(&f, &f.admin, &[("path", "/abc.jpg")]).await.unwrap();
        assert_eq!(seen.lock().unwrap().last().unwrap(), "/w342/abc.jpg");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_bad_answer_is_a_failure_and_is_not_kept() {
        let f = fixture("bad-answer").await;
        let far = upstream(Router::new().fallback(|| async { JPEG })).await;
        let to = format!("{}/x.jpg", far.base);
        let up = upstream(
            Router::new()
                .route("/w342/html.jpg", get(|| async { "<html>not an image</html>" }))
                .route("/w342/gone.jpg", get(|| async { StatusCode::NOT_FOUND }))
                .route(
                    "/w342/moved.jpg",
                    get(move || {
                        let to = to.clone();
                        async move { Redirect::temporary(&to).into_response() }
                    }),
                )
                .route("/w342/big.jpg", get(|| async { jpeg_of(CAP + 1) }))
                .route("/w342/big-unsized.jpg", get(|| async { without_length(CAP + 1) }))
                .route("/w342/at-cap.jpg", get(|| async { jpeg_of(CAP) }))
                .route("/w342/at-cap-unsized.jpg", get(|| async { without_length(CAP) })),
        )
        .await;
        point_at(&f, &up);
        for name in ["/html.jpg", "/gone.jpg", "/moved.jpg", "/big.jpg", "/big-unsized.jpg"] {
            assert_eq!(
                poster(&f, &f.admin, &[("path", name)]).await.unwrap_err(),
                StatusCode::BAD_GATEWAY,
                "{name}"
            );
            let cached =
                f.st.library
                    .as_ref()
                    .unwrap()
                    .match_posters
                    .cached(&format!("match-poster/w342{name}"))
                    .await;
            assert!(cached.is_none(), "{name}");
        }
        assert_eq!(far.hits.load(Ordering::SeqCst), 0, "no redirect is followed");
        // One byte less is a good answer: it was the size that got those two refused.
        for name in ["/at-cap.jpg", "/at-cap-unsized.jpg"] {
            let r = poster(&f, &f.admin, &[("path", name)]).await.unwrap();
            assert_eq!(to_bytes(r.into_body(), CAP + 1).await.unwrap().len(), CAP, "{name}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_answer_changes_only_when_asked() {
        let f = fixture("flag").await;
        let stored = r#"[{"tmdb_id":7,"title":"A","poster":"https://image.tmdb.org/t/p/w342/abc.jpg"}]"#.to_string();
        f.st.db
            .clone()
            .unwrap()
            .call(move |c| {
                c.execute_batch("PRAGMA foreign_keys = OFF;")?;
                c.execute(
                    "INSERT INTO items(id, kind, library_id, key, title, sort_title, identify_state, added_at, updated_at, candidates)
                     VALUES ('m1', 'movie', 'lib', 'k1', 'A', 'A', 'review', 0, 0, ?1)",
                    [stored],
                )?;
                c.execute(
                    "INSERT INTO files(id, library_id, path, size, mtime, role, item_id, first_seen, last_seen, stable_since, duration)
                     VALUES ('m1', 'lib', '/m/m1', 1, 1, 'main', 'm1', 0, 0, 0, 2700)",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let Json(old) = admin_review(State(f.st.clone()), f.admin.clone(), q(&[])).await.unwrap();
        assert_eq!(
            old["items"][0]["candidates"],
            json!([{"tmdb_id": 7, "title": "A", "poster": "https://image.tmdb.org/t/p/w342/abc.jpg"}]),
            "without the flag, the answer older apps get"
        );
        let Json(new) = admin_review(State(f.st.clone()), f.admin.clone(), q(&[("server_posters", "1")]))
            .await
            .unwrap();
        assert_eq!(
            new["items"][0]["candidates"],
            json!([{"tmdb_id": 7, "title": "A", "poster_path": "/abc.jpg"}]),
        );
        assert!(!new.to_string().contains("image.tmdb.org"));
    }
}
