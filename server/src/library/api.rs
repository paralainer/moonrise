//! The read-only browse API (Phase 2). JSON is snake_case; lists are `{items, total}`; times
//! are seconds, dates RFC 3339. `user`: the Moonrise account's watch state (null for other callers). Auth: a Moonrise
//! token, as for playback (`/v1/images` needs none: the ids are hashes).
//!
//! Libraries have no type: any library holds movies and shows. `kind` (`movies`, `shows` or
//! `mixed`) is what a library was made as, kept for older apps (which show a `movies` or
//! `mixed` library as movies, a `shows` one as shows); browsing by type is `type=` across
//! libraries, a library a filter (`library=`).
//!
//! - `GET /v1/libraries`: `{items: [{id, name, kind, collection_type, item_count, movies, shows, backdrop}]}`
//! - `GET /v1/items?library=&type=movie|series|episode&sort=title|added|year|rating|played|runtime&order=asc|desc&offset=&limit=`
//!   with the filters of `browse.rs` (genre, year_from/year_to, rating, studio, collection,
//!   resolution, hdr, unplayed/played/in_progress, added_days, only); a title sort answers
//!   `index: [{letter, offset}]` too. Or `?ids=a,b,c` (any order) or `?parent=<series|season>`
//! - `GET /v1/facets?type=movie|series&<filters>` (all libraries; `library=` one) and
//!   `GET /v1/libraries/{id}/facets?<filters>`: what the filters can pick, with counts (also
//!   `libraries: [{id, name, count}]`)
//! - `GET /v1/items/{id}`: one item with people, files (and their tracks), chapters. A
//!   merged id answers with the new item. A title with several files has versions: each file's
//!   `version {label, details, edition, resolution, hdr, codec, audio_languages, dub, source}`
//!   (`label` tells it from the others), and for a Moonrise user its `position` (a cut of another
//!   length keeps its own resume point) and `user.version` (the one they chose). Playback,
//!   subtitles, downloads and offline sets take `?version=<file id>` (or `<item>~<file>` as the id).
//!   `ratings` (0.26, when there are any): `{imdb: {value, votes}, rotten_tomatoes: {value},
//!   metacritic: {value}}`, beside TMDB's `community_rating` (`ratings.rs`).
//! - `GET /v1/shows/{id}/seasons`, `GET /v1/shows/{id}/episodes?season=` (airing order,
//!   specials last; each episode's `ratings` too)
//! - `GET /v1/search?q=`: `{titles, episodes}` (40 and 20)
//! - `GET /v1/home`: `{continue, resume, next_up, latest_types: [{type: movie|series, title, items}],
//!   latest: {<library>: [...]}}` (`continue`: the Continue Watching row, each item with
//!   `continue {reason: resume|next_episode|collection_next, at, from}`; `resume` and `next_up`
//!   for older apps; all three a Moonrise token's watch state; `latest_types` across libraries,
//!   new episodes grouped into their shows; `latest` per library for older apps; `?watching=1`
//!   leaves `latest_types` and `latest` empty, for a client that only follows the watching rows);
//!   `GET /v1/libraries/{id}/latest?limit=`
//! - `GET /v1/images/{id}?w=`: JPEG/PNG, `Cache-Control: public, max-age=31536000, immutable`
//! - `GET /v1/items/{id}/segments`: skippable parts from chapter names (the app's rules), and an
//!   episode's end credits found in the file (`source: detected`, `credits.rs`); `pending: true`
//!   while that is still being looked for
//! - Admin (the token, or `X-Moonrise-Admin` with `/data/secrets/admin.token` for the CLI):
//!   `POST /v1/admin/scan?full=1` (waits for the pass; `&wait=0` starts it and answers at once;
//!   `?library=<id>` starts a pass that walks that library's folders in full),
//!   `GET /v1/admin/stats?recent=<hours>`,
//!   `GET /v1/admin/scan/progress` (the pass running now: `phase` idle|queued|scanning|identifying|
//!   metadata|finishing, `walked` files seen by the walk, `identify {done, total}`, `passes`,
//!   `last_pass`, `roots`, per library `files`, `identified`, `movies`, `series`, `episodes`,
//!   `review` (movies and shows waiting in Fix Match), `review_episodes` (episodes of shows that
//!   aren't), `settling`; the app's setup screen polls it),
//!   `GET /v1/admin/review?source=llm` (titles in review with their TMDB `candidates` for Fix
//!   Match; episodes with their series and numbers), `POST /v1/admin/items/{id}/identify
//!   {tmdb_id, season?, episode?, part?, file_id?} | {local: true} | {automatic: true, file_id?} |
//!   {split: true, file_id}` (Fix Match, Back to Automatic, Split Off), `GET /v1/admin/items/{id}/match`,
//!   `GET /v1/admin/match/search?kind=movie|tv&q=&year=`, `GET /v1/admin/match/episodes?tmdb_id=&season=`;
//!   `server_posters=1` on review, match and search gives candidates a `poster_path` in place of a
//!   TMDB `poster` URL, for `GET /v1/admin/match/poster?path=&size=` (capability `match-posters`).

use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};

use super::{browse, community, match_posters, rfc3339, search, store, util, Library};
use crate::{
    api::{ApiError, AppState},
    auth::Caller,
    media::segment_rules,
    userdata,
};

type Q = Query<HashMap<String, String>>;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/libraries", get(libraries))
        .route("/v1/libraries/{id}/latest", get(library_latest))
        .route("/v1/libraries/{id}/facets", get(library_facets))
        .route("/v1/facets", get(type_facets))
        .route("/v1/items", get(items))
        .route("/v1/items/{id}", get(item))
        .route("/v1/items/{id}/segments", get(segments))
        .route("/v1/shows/{id}/seasons", get(seasons))
        .route("/v1/shows/{id}/episodes", get(episodes))
        .route("/v1/search", get(search_handler))
        .route("/v1/home", get(home))
        .route("/v1/images/{id}", get(image))
        .route("/v1/admin/scan", post(admin_scan))
        .route("/v1/admin/stats", get(admin_stats))
        .route("/v1/admin/scan/progress", get(admin_scan_progress))
        .route("/v1/admin/review", get(admin_review))
        .route("/v1/admin/items/{id}/identify", post(admin_identify))
        .route("/v1/admin/items/{id}/match", get(item_match))
        .route("/v1/admin/match/search", get(match_search))
        .route("/v1/admin/match/episodes", get(match_episodes))
        .route("/v1/admin/match/poster", get(match_poster))
}

pub(crate) fn lib(st: &AppState) -> Result<&Arc<Library>, ApiError> {
    st.library
        .as_ref()
        .ok_or_else(|| ApiError(StatusCode::SERVICE_UNAVAILABLE, "the library is switched off".into()))
}

/// Who is asking: a Moonrise token (with watch state) or the CLI's admin token.
async fn authed(st: &AppState, headers: &HeaderMap, q: &HashMap<String, String>) -> Result<Caller, ApiError> {
    crate::auth::caller(st, headers, q).await
}

/// The admin endpoints: the CLI's admin token or a Moonrise admin.
async fn admin_authed(st: &AppState, headers: &HeaderMap, q: &HashMap<String, String>) -> Result<Caller, ApiError> {
    crate::auth::admin_caller(st, headers, q).await
}

/// The Moonrise user whose watch state goes with the answer (None: the admin caller, or the
/// accounts are switched off).
pub(crate) fn viewer(st: &AppState, c: &Caller) -> Option<String> {
    st.auth.as_ref()?;
    c.user_id().filter(|u| userdata::sql_id(u).is_some())
}

fn internal(e: anyhow::Error) -> ApiError {
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
}

/// A page of items as the API shapes them, and how many match (`store::query_items`).
async fn query_items(
    l: &Library,
    user: Option<String>,
    sql_where: String,
    order: String,
    args: Vec<String>,
    offset: i64,
    limit: i64,
) -> Result<(Vec<Value>, i64), ApiError> {
    l.db.call(move |c| store::query_items(c, user.as_deref(), &sql_where, &order, &args, offset, limit))
        .await
        .map_err(internal)
}

async fn libraries(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    authed(&st, &headers, &q).await?;
    let l = lib(&st)?;
    let out = l.db.call(|c| store::libraries(c)).await.map_err(internal)?;
    Ok(Json(out))
}

fn page(q: &HashMap<String, String>) -> (i64, i64) {
    let offset = q
        .get("offset")
        .or(q.get("start"))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0i64)
        .max(0);
    let limit = q.get("limit").and_then(|s| s.parse().ok()).unwrap_or(100i64).clamp(1, 500);
    (offset, limit)
}

async fn items(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    let who = authed(&st, &headers, &q).await?;
    let user = viewer(&st, &who);
    let l = lib(&st)?;
    let (offset, limit) = page(&q);
    let mut notes: Vec<&str> = Vec::new();
    if let Some(ids) = q.get("ids") {
        let mut wanted: Vec<String> = Vec::new();
        for id in ids.split(',').map(str::trim).filter(|s| !s.is_empty()).take(200) {
            wanted.push(resolve_id(l, id).await?.unwrap_or_else(|| id.to_string()));
        }
        let (items, total) = query_items(l, user, store::ids_in(wanted.len()), store::BY_SORT_TITLE.into(), wanted, 0, 200).await?;
        return Ok(Json(json!({"items": items, "total": total})));
    }
    let mut cond = vec![store::visible_any("i")];
    let mut args: Vec<String> = Vec::new();
    if let Some(parent) = q.get("parent") {
        cond.push(store::CHILDREN_OF.into());
        args.push(parent.clone());
        args.push(parent.clone());
    } else {
        // `library` is a filter (browse.rs); `parent_id` the same, as older apps send it.
        let library = q.get("library").or(q.get("parent_id"));
        let kind = match (q.get("type").map(String::as_str), library) {
            (Some(t), _) => t.to_string(),
            (None, Some(lid)) => {
                let lid = lid.clone();
                let k: Option<String> = l.db.call(move |c| store::library_kind(c, &lid)).await.map_err(internal)?;
                if k.as_deref() == Some("shows") {
                    "series".into()
                } else {
                    "movie".into()
                }
            }
            (None, None) => "movie".into(),
        };
        let kinds: Vec<&str> = kind
            .split(',')
            .filter(|k| ["movie", "series", "season", "episode"].contains(k))
            .collect();
        if kinds.is_empty() {
            return Err(ApiError(StatusCode::BAD_REQUEST, "type: movie, series, season or episode".into()));
        }
        cond.push(store::kind_in(&kinds));
        if let (None, Some(lid)) = (q.get("library"), q.get("parent_id")) {
            cond.push(store::IN_LIBRARY.into());
            args.push(lid.clone());
        }
    }
    let (extra, more_notes) = browse::filters(&q, user.as_deref()).map_err(|e| ApiError(StatusCode::BAD_REQUEST, e))?;
    notes.extend(more_notes);
    for c in extra {
        cond.push(c.sql);
        args.extend(c.args);
    }
    let sort = q.get("sort").map_or("title", String::as_str);
    let desc_default = !matches!(sort, "title" | "name");
    let desc = q.get("order").map_or(desc_default, |o| o.eq_ignore_ascii_case("desc"));
    let order = browse::order(sort, desc, user.as_deref(), &mut notes);
    // The alphabet index of a title sort (offsets into the whole filtered list).
    let index = if matches!(sort, "title" | "name") && !q.contains_key("parent") {
        let (w, a, o) = (cond.join(" AND "), args.clone(), order.clone());
        Some(l.db.call(move |c| store::letter_index(c, &w, &a, &o)).await.map_err(internal)?)
    } else {
        None
    };
    let (items, total) = query_items(l, user, cond.join(" AND "), order, args, offset, limit).await?;
    let mut out = json!({"items": items, "total": total, "offset": offset});
    if let Some(index) = index {
        out["index"] = json!(index
            .into_iter()
            .map(|(letter, at)| json!({"letter": letter, "offset": at}))
            .collect::<Vec<_>>());
    }
    if !notes.is_empty() {
        out["notes"] = json!(notes);
    }
    Ok(Json(out))
}

/// The current id of an item (following merges).
pub(crate) async fn resolve_id(l: &Library, id: &str) -> Result<Option<String>, ApiError> {
    Ok(Some(l.resolve_id(id).await.map_err(internal)?))
}

async fn item(State(st): State<Arc<AppState>>, Path(id): Path<String>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    let who = authed(&st, &headers, &q).await?;
    let user = viewer(&st, &who);
    let signed_in = user.is_some();
    let l = lib(&st)?;
    let rid = resolve_id(l, &id)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such item".into()))?;
    let r2 = rid.clone();
    let with_ratings = l.config.enabled(crate::admin::config::Feature::Ratings);
    let row =
        l.db.call(move |c| store::item_detail(c, &r2, user.as_deref(), with_ratings))
            .await
            .map_err(internal)?;
    let Some((mut v, files, mine)) = row else {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such item".into()));
    };
    // A film or show without critics' scores yet is looked up next (this answer doesn't wait).
    if with_ratings
        && matches!(v["kind"].as_str(), Some("movie" | "series"))
        && v["ratings"].get("rotten_tomatoes").is_none()
        && v["ratings"].get("metacritic").is_none()
    {
        if let Some(imdb) = v["provider_ids"]["Imdb"].as_str() {
            l.ratings.want(imdb);
        }
    }
    // A film or an episode: its community segment times are looked up now if they aren't kept,
    // so they are there when it plays (this answer doesn't wait).
    if matches!(v["kind"].as_str(), Some("movie" | "episode")) {
        let (c, id) = (l.community.clone(), rid.clone());
        tokio::spawn(async move { c.want(vec![id]).await });
    }
    let mut out_files = Vec::new();
    let mut chapters = Value::Array(vec![]);
    let mut described = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let tracks = if f.missing {
            Vec::new()
        } else {
            l.streams_for(&f.path, f.size, f.mtime).await.unwrap_or_default()
        };
        let probe = f.probe.as_deref().and_then(|p| serde_json::from_str::<Value>(p).ok());
        if i == 0 {
            chapters = item_chapters(f.chapters.as_deref(), probe.as_ref());
        }
        if !f.missing {
            described.push(super::versions::describe(
                &f.id,
                &f.path,
                f.size,
                f.duration,
                &tracks,
                probe.as_ref(),
                f.nfo_edition.as_deref(),
            ));
        }
        out_files.push(json!({
            "id": f.id, "name": f.path.rsplit('/').next(), "container": f.container, "size": f.size, "duration": f.duration, "bitrate": f.bitrate,
            "version_label": f.label, "missing": f.missing, "tracks": tracks, "manual": f.manual,
            // Damaged or unreadable (role `broken`): playback answers with a healthy version of
            // the title where there is one; why, as the probe or the index read found it.
            "broken": f.broken, "broken_reason": if f.broken { f.broken_reason.clone() } else { None },
        }));
    }
    // Each present file as a version: a label that tells it from the others, its details, and
    // (a Moonrise user) where they stopped in it.
    let labels = super::versions::labels(&described);
    let resume_file = mine.resume.as_ref().and_then(|(_, f)| f.clone());
    let duration_of = |id: &str| files.iter().find(|f| f.id == id).and_then(|f| f.duration);
    for (d, label) in described.iter().zip(&labels) {
        let Some(out) = out_files.iter_mut().find(|o| o["id"] == d.file_id) else {
            continue;
        };
        out["version"] = super::versions::json_of(d, label);
        if signed_in {
            let shared = match (&mine.resume, &resume_file) {
                (Some((p, _)), None) if *p > 0.0 => Some(*p),
                (Some((p, _)), Some(rf)) if *p > 0.0 && (rf == &d.file_id || super::versions::same_cut(duration_of(rf), d.duration)) => {
                    Some(*p)
                }
                _ => None,
            };
            let own = mine
                .positions
                .iter()
                .find(|(f, _)| *f == d.file_id)
                .map(|(_, p)| *p)
                .filter(|p| *p > 0.0);
            out["position"] = json!(shared.or(own).unwrap_or(0.0));
        }
    }
    if let Some(u) = v["user"].as_object_mut() {
        u.insert(
            "version".into(),
            json!(mine.choice.filter(|c| described.iter().any(|d| d.file_id == *c))),
        );
    }
    v["files"] = json!(out_files);
    v["chapters"] = chapters;
    if rid != id {
        v["requested_id"] = json!(id);
    }
    Ok(Json(v))
}

async fn seasons(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    let who = authed(&st, &headers, &q).await?;
    let user = viewer(&st, &who);
    let l = lib(&st)?;
    let sid = resolve_id(l, &id).await?.unwrap_or(id);
    let (items, total) =
        l.db.call(move |c| store::seasons(c, user.as_deref(), sid))
            .await
            .map_err(internal)?;
    Ok(Json(json!({"items": items, "total": total})))
}

async fn episodes(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    let who = authed(&st, &headers, &q).await?;
    let user = viewer(&st, &who);
    let l = lib(&st)?;
    let sid = resolve_id(l, &id).await?.unwrap_or(id);
    let season = match q.get("season").or(q.get("season_id")) {
        Some(season) => Some(match season.parse::<i64>() {
            Ok(n) => store::Season::Number(n),
            Err(_) => store::Season::Id(resolve_id(l, season).await?.unwrap_or(season.clone())),
        }),
        None => None,
    };
    let (mut items, total) =
        l.db.call(move |c| store::episodes(c, user.as_deref(), sid, season))
            .await
            .map_err(internal)?;
    if l.config.enabled(crate::admin::config::Feature::Ratings) {
        items =
            l.db.call(move |c| {
                super::ratings::attach(c, &mut items)?;
                Ok(items)
            })
            .await
            .map_err(internal)?;
    }
    Ok(Json(json!({"items": items, "total": total})))
}

async fn search_handler(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    let who = authed(&st, &headers, &q).await?;
    let user = viewer(&st, &who);
    let l = lib(&st)?;
    let text = q.get("q").or(q.get("query")).cloned().unwrap_or_default();
    let (Some(tq), Some(eq)) = (
        search::fts_query(&text, "title original_title"),
        search::fts_query(&text, "title original_title series_title"),
    ) else {
        return Ok(Json(json!({"titles": [], "episodes": []})));
    };
    let ids = l.db.call(move |c| store::search_ids(c, &tq, &eq)).await.map_err(internal)?;
    let titles = items_in_order(l, user.clone(), ids.0, 100).await?;
    let episodes = items_in_order(l, user, ids.1, 100).await?;
    Ok(Json(json!({"titles": titles, "episodes": episodes})))
}

/// The newest additions of a library, by what it was made as (`latest_movies`,
/// `latest_shows`; a `mixed` one both, newest first). For a Moonrise user, watched titles are left
/// out (as media servers' Latest does).
async fn latest_for(l: &Library, user: Option<String>, library: &str, limit: i64) -> Result<Vec<Value>, ApiError> {
    let lid = library.to_string();
    let kind: Option<String> = l.db.call(move |c| store::library_kind(c, &lid)).await.map_err(internal)?;
    match kind.as_deref() {
        Some("movies") => latest_movies(l, user, Some(library), limit).await,
        Some("shows") => latest_shows(l, user, Some(library), limit).await,
        Some(_) => {
            let mut all = latest_movies(l, user.clone(), Some(library), limit).await?;
            all.extend(latest_shows(l, user, Some(library), limit).await?);
            let at = |v: &Value| v["latest_added_at"].as_str().or(v["added_at"].as_str()).unwrap_or("").to_string();
            all.sort_by_key(|v| std::cmp::Reverse(at(v)));
            all.truncate(limit as usize);
            Ok(all)
        }
        None => Err(ApiError(StatusCode::NOT_FOUND, "no such library".into())),
    }
}

/// The newest movies (of one library, or all).
async fn latest_movies(l: &Library, user: Option<String>, library: Option<&str>, limit: i64) -> Result<Vec<Value>, ApiError> {
    let library = library.map(String::from);
    l.db.call(move |c| store::latest_movies(c, user.as_deref(), library.as_deref(), limit))
        .await
        .map_err(internal)
}

/// New episodes grouped into their series (of one library, or all), newest first, as the usual
/// Latest (`groupItems`) does: a series whose latest arrival is a single episode (nothing else
/// of it came within 3 days before) is shown as that episode, otherwise as the series.
async fn latest_shows(l: &Library, user: Option<String>, library: Option<&str>, limit: i64) -> Result<Vec<Value>, ApiError> {
    let lib = library.map(String::from);
    let u = user.clone();
    let rows: Vec<(String, String, i64)> =
        l.db.call(move |c| store::latest_episodes(c, u.as_deref(), lib.as_deref(), limit))
            .await
            .map_err(internal)?;
    let picks = group_latest(&rows, limit as usize);
    if picks.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = picks.iter().map(|p| p.0.clone()).collect();
    let mut items = items_by_ids(l, user.clone(), ids).await?;
    for v in &mut items {
        if let Some((_, at)) = picks.iter().find(|p| v["id"] == p.0) {
            v["latest_added_at"] = json!(rfc3339(*at));
        }
    }
    Ok(items)
}

async fn library_latest(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    let who = authed(&st, &headers, &q).await?;
    let l = lib(&st)?;
    let limit = q.get("limit").and_then(|s| s.parse().ok()).unwrap_or(20i64).clamp(1, 100);
    let items = latest_for(l, viewer(&st, &who), &id, limit).await?;
    let total = items.len();
    Ok(Json(json!({"items": items, "total": total})))
}

/// `GET /v1/libraries/{id}/facets?<the filters of /v1/items>&recent_days=30`: the genres,
/// years and decades, ratings, studios (networks), collections, resolutions, HDR, watch state
/// and recent additions of a library's titles, with counts. Each facet is counted under the
/// other facets' filters (not its own), so its other values stay pickable.
async fn library_facets(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    let who = authed(&st, &headers, &q).await?;
    let user = viewer(&st, &who);
    let l = lib(&st)?;
    let lid = id.clone();
    let kind: Option<String> = l.db.call(move |c| store::library_kind(c, &lid)).await.map_err(internal)?;
    let Some(kind) = kind else {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such library".into()));
    };
    let top = match q.get("type").map(String::as_str) {
        Some("series") => "series",
        Some(_) => "movie",
        None if kind == "shows" => "series",
        None => "movie",
    };
    let base = vec![
        browse::Cond {
            facet: None,
            sql: store::visible_any("i"),
            args: vec![],
        },
        browse::Cond {
            facet: None,
            sql: store::kind_is(top),
            args: vec![],
        },
        browse::Cond {
            facet: None,
            sql: store::IN_LIBRARY.into(),
            args: vec![id.clone()],
        },
    ];
    let (filters, notes) = browse::filters(&q, user.as_deref()).map_err(|e| ApiError(StatusCode::BAD_REQUEST, e))?;
    let recent_days = q.get("recent_days").and_then(|d| d.parse().ok()).unwrap_or(30i64).clamp(1, 3650);
    let mut out =
        l.db.call(move |c| browse::facets(c, &base, &filters, user.as_deref(), recent_days))
            .await
            .map_err(internal)?;
    out["library"] = json!(id);
    out["kind"] = json!(top);
    if !notes.is_empty() {
        out["notes"] = json!(notes);
    }
    Ok(Json(out))
}

/// `GET /v1/facets?type=movie|series&<the filters of /v1/items>&recent_days=30`: as a
/// library's facets, over all libraries (`library=<id>` is one of the filters).
async fn type_facets(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    let who = authed(&st, &headers, &q).await?;
    let user = viewer(&st, &who);
    let l = lib(&st)?;
    let top = match q.get("type").map(String::as_str) {
        Some("series") => "series",
        Some("movie") | None => "movie",
        Some(_) => return Err(ApiError(StatusCode::BAD_REQUEST, "type: movie or series".into())),
    };
    let base = vec![
        browse::Cond {
            facet: None,
            sql: store::visible_any("i"),
            args: vec![],
        },
        browse::Cond {
            facet: None,
            sql: store::kind_is(top),
            args: vec![],
        },
    ];
    let (filters, notes) = browse::filters(&q, user.as_deref()).map_err(|e| ApiError(StatusCode::BAD_REQUEST, e))?;
    let recent_days = q.get("recent_days").and_then(|d| d.parse().ok()).unwrap_or(30i64).clamp(1, 3650);
    let mut out =
        l.db.call(move |c| browse::facets(c, &base, &filters, user.as_deref(), recent_days))
            .await
            .map_err(internal)?;
    out["kind"] = json!(top);
    if !notes.is_empty() {
        out["notes"] = json!(notes);
    }
    Ok(Json(out))
}

/// Items by id, in that order.
pub(crate) async fn items_by_ids(l: &Library, user: Option<String>, ids: Vec<String>) -> Result<Vec<Value>, ApiError> {
    let n = ids.len() as i64;
    items_in_order(l, user, ids, n).await
}

/// Items by id, in that order (at most `limit`).
async fn items_in_order(l: &Library, user: Option<String>, ids: Vec<String>, limit: i64) -> Result<Vec<Value>, ApiError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    l.db.call(move |c| store::items_by_ids(c, user.as_deref(), ids, limit))
        .await
        .map_err(internal)
}

/// Home's `latest_types` and `latest`; empty (and not looked up) when only the watching rows are wanted.
async fn home_latest(l: &Library, user: Option<String>, limit: i64, watching: bool) -> Result<(Value, Value), ApiError> {
    if watching {
        return Ok((json!([]), json!({})));
    }
    let mut latest = serde_json::Map::new();
    for lc in &l.libs() {
        latest.insert(lc.id.clone(), json!(latest_for(l, user.clone(), &lc.id, limit).await?));
    }
    let latest_types = json!([
        {"type": "movie", "title": "Movies", "items": latest_movies(l, user.clone(), None, limit).await?},
        {"type": "series", "title": "Shows", "items": latest_shows(l, user, None, limit).await?},
    ]);
    Ok((latest_types, Value::Object(latest)))
}

/// The home screen in one round trip: `continue` (Continue Watching, `userdata::continuing`),
/// `resume`, `next_up` (for older apps; all three a Moonrise account's watch state, empty for
/// other callers) and `latest` per library. `watching=1` skips the newest-titles rows.
async fn home(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    let who = authed(&st, &headers, &q).await?;
    let l = lib(&st)?;
    let user = viewer(&st, &who);
    let limit = q.get("limit").and_then(|s| s.parse().ok()).unwrap_or(20i64).clamp(1, 100);
    let (latest_types, latest) = home_latest(l, user.clone(), limit, flag(&q, "watching")).await?;
    let Some(u) = user else {
        return Ok(Json(
            json!({"continue": [], "resume": [], "next_up": [], "latest_types": latest_types, "latest": latest, "notes": ["continue, resume and next_up need a Moonrise account token"]}),
        ));
    };
    let u2 = u.clone();
    let now = util::now();
    let (cards, resume_ids, next_ids) =
        l.db.call(move |c| {
            Ok((
                userdata::continuing::continue_watching(c, &u2, limit as usize, now)?,
                userdata::resume(c, &u2, limit, None)?,
                userdata::next_up(c, &u2, limit as usize, None, false)?,
            ))
        })
        .await
        .map_err(internal)?;
    let mut cont = items_by_ids(l, Some(u.clone()), cards.iter().map(|c| c.item.clone()).collect()).await?;
    for v in &mut cont {
        if let Some(card) = cards.iter().find(|c| v["id"] == *c.item) {
            v["continue"] = json!({"reason": card.reason.as_str(), "at": rfc3339(card.at), "from": card.from});
        }
    }
    let resume = items_by_ids(l, Some(u.clone()), resume_ids).await?;
    let next_up = items_by_ids(l, Some(u), next_ids).await?;
    Ok(Json(
        json!({"continue": cont, "resume": resume, "next_up": next_up, "latest_types": latest_types, "latest": latest}),
    ))
}

async fn image(State(st): State<Arc<AppState>>, Path(id): Path<String>, Query(q): Q) -> Result<Response, ApiError> {
    let l = lib(&st)?;
    let w = q
        .get("w")
        .or(q.get("maxWidth"))
        .or(q.get("width"))
        .and_then(|w| w.parse::<u32>().ok());
    match l.images.serve(&id, w).await {
        Ok(Some((bytes, ctype))) => Ok((
            [
                (header::CONTENT_TYPE, ctype),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            bytes,
        )
            .into_response()),
        Ok(None) => Err(ApiError(StatusCode::NOT_FOUND, "no such image".into())),
        Err(e) => Err(ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}"))),
    }
}

/// The app's chapter-name rules (`MediaSegment.Kind(chapterName:)`).
pub fn segment_kind(name: &str) -> Option<&'static str> {
    let n = without_language(name).to_lowercase();
    let starts = |list: &[&str]| list.iter().any(|p| n == *p || n.starts_with(&format!("{p} ")));
    if starts(&["intro", "opening", "op", "title sequence", "opening credits", "theme song"]) {
        Some("intro")
    } else if n.contains("recap") || n.starts_with("previously") {
        Some("recap")
    } else if starts(&["credits", "end credits", "ending", "ed", "outro", "closing credits"]) {
        Some("credits")
    } else if n.contains("preview") || n.contains("next time") {
        Some("preview")
    } else {
        None
    }
}

/// A file's chapters as `GET /v1/items/{id}` lists them: start (on the title's timeline) and name.
fn item_chapters(chapters: Option<&str>, probe: Option<&Value>) -> Value {
    let ch: Vec<Value> = chapters.and_then(|c| serde_json::from_str(c).ok()).unwrap_or_default();
    let clock = probe.and_then(crate::media::OwnClock::of);
    let offset = crate::media::OwnClock::chapter_offset(clock, &chapter_starts(&ch, None));
    json!(ch
        .iter()
        .map(|c| {
            let start = c["start_time"].as_str().and_then(|s| s.parse::<f64>().ok());
            json!({"start": start.map(|s| on_timeline(s, offset)), "name": c["tags"]["title"]})
        })
        .collect::<Vec<_>>())
}

/// The chapters' starts (ffprobe's `start_time`, 0 if unreadable), on the title's timeline for a
/// file with a start time (`clock`; `media::OwnClock`), else as they are.
pub(crate) fn chapter_starts(chapters: &[Value], clock: Option<crate::media::OwnClock>) -> Vec<f64> {
    let starts: Vec<f64> = chapters
        .iter()
        .map(|c| c["start_time"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0))
        .collect();
    let offset = crate::media::OwnClock::chapter_offset(clock, &starts);
    starts.into_iter().map(|s| on_timeline(s, offset)).collect()
}

/// A time on the file's own clock, `offset` past the title's timeline, on the timeline (rounded
/// to the millisecond ffprobe gives); as it is with no offset.
fn on_timeline(t: f64, offset: f64) -> f64 {
    if offset == 0.0 {
        return t;
    }
    ((t - offset).max(0.0) * 1000.0).round() / 1000.0
}

/// A chapter name without a leading language tag ("en:End Credits", "eng: Credits").
fn without_language(name: &str) -> &str {
    match name.split_once(':') {
        Some((lang, rest)) if (2..=3).contains(&lang.len()) && lang.bytes().all(|b| b.is_ascii_alphabetic()) => rest.trim_start(),
        _ => name,
    }
}

async fn segments(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    let who = authed(&st, &headers, &q).await?;
    let l = lib(&st)?;
    // The version played (`<item>~<file>`, `version=`, else the one the viewer chose): its
    // chapters and its end credits.
    let key = st.with_chosen_version(&who, crate::api::media_key(&id, &q)).await;
    let (base, version) = key.split_once('~').map_or((key.as_str(), ""), |(b, v)| (b, v));
    let version = version.to_string();
    let rid = resolve_id(l, base)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such item".into()))?;
    let row: Option<store::SegRow> = {
        let rid = rid.clone();
        l.db.call(move |c| store::segment_file(c, rid, version))
    }
    .await
    .map_err(internal)?;
    let merged = flag(&q, "merged");
    let Some((chapters, duration, path, size, mtime, probe, credits, kind)) = row else {
        if merged {
            return Ok(Json(
                json!({"items": [], "total": 0, "pending": false, "merged": true, "community_pending": false}),
            ));
        }
        return Ok(Json(json!({"items": [], "total": 0})));
    };
    let ch: Vec<Value> = chapters.and_then(|c| serde_json::from_str(&c).ok()).unwrap_or_default();
    // The chapters and the length on the title's timeline (a file with a start time: `media::OwnClock`).
    let clock = probe
        .as_deref()
        .and_then(|p| serde_json::from_str::<Value>(p).ok())
        .and_then(|p| crate::media::OwnClock::of(&p));
    let duration = clock.and_then(|c| c.length).or(duration);
    let starts = chapter_starts(&ch, clock);
    let mut out = Vec::new();
    if ch.len() > 1 {
        for (i, c) in ch.iter().enumerate() {
            let Some(kind) = c["tags"]["title"].as_str().and_then(segment_kind) else {
                continue;
            };
            let end = starts.get(i + 1).copied().or(duration).unwrap_or(starts[i]);
            if end - starts[i] >= 1.0 && (end - starts[i] < 600.0 || kind == "credits") {
                out.push(json!({"kind": kind, "start": starts[i], "end": end, "source": "chapters"}));
            }
        }
    }
    // An episode's end credits as found in the file (`credits.rs`), unless its chapters say.
    let mut pending = false;
    if kind == "episode" && !out.iter().any(|s| s["kind"] == "credits") {
        if let Some(duration) = duration.filter(|d| *d > crate::media::credits::WINDOW) {
            match crate::media::credits::read(credits.as_deref()) {
                Some(Some(start)) => out.push(json!({"kind": "credits", "start": start, "end": duration, "source": "detected"})),
                Some(None) => {}
                None if st.config.enabled(crate::admin::config::Feature::CreditsDetect) => {
                    let probe = probe.and_then(|p| serde_json::from_str::<Value>(&p).ok());
                    if let Some((_, stream, _)) = probe.as_ref().and_then(|p| crate::media::index_plan(p, &st.config)) {
                        let start = probe
                            .as_ref()
                            .and_then(|p| p["format"]["start_time"].as_str()?.parse::<f64>().ok())
                            .filter(|s| s.is_finite())
                            .unwrap_or(0.0);
                        pending = st.credits.request(
                            &st,
                            crate::media::credits::Job {
                                path,
                                size,
                                mtime,
                                stream,
                                duration,
                                start,
                            },
                        );
                    }
                }
                None => {}
            }
        }
    }
    if !merged {
        return Ok(Json(json!({"items": out, "total": out.len(), "pending": pending})));
    }
    // The segments the player offers: the file's, with the community databases' kept
    // answers, by the rules the apps had (`segment_rules.rs`). `provisional`: a download saved
    // while the file is still looked at uses the databases' credits meanwhile; `community=0`:
    // the device doesn't want the databases'.
    let runtime = duration.filter(|d| d.is_finite() && *d > 0.0).unwrap_or(0.0);
    let community = if q.get("community").is_some_and(|v| v == "0" || v == "false") {
        community::Answer::default()
    } else {
        l.community.answer(&rid, runtime).await
    };
    Ok(Json(merged_answer(
        &out,
        &community,
        runtime,
        pending,
        flag(&q, "provisional"),
        kind == "movie",
    )))
}

/// `segments?merged=1`'s answer: the file's segments (`file`, as the plain answer lists them)
/// merged with the community databases' (`segment_rules::merge`). `pending` is the file's (the
/// end credits still looked for); with `provisional` the databases' credits are used meanwhile.
fn merged_answer(file: &[Value], community: &community::Answer, runtime: f64, pending: bool, provisional: bool, movie: bool) -> Value {
    let file: Vec<segment_rules::Segment> = file.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect();
    let items = segment_rules::merge(&file, &community.online, runtime, pending && !provisional, movie, community.pending);
    json!({
        "items": items,
        "total": items.len(),
        "pending": pending,
        "merged": true,
        "community_pending": community.pending,
    })
}

/// A query flag: `1` or `true`.
fn flag(q: &HashMap<String, String>, name: &str) -> bool {
    q.get(name).is_some_and(|v| v == "1" || v == "true")
}

async fn admin_scan(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    admin_authed(&st, &headers, &q).await?;
    let l = lib(&st)?;
    let full = q.get("full").is_some_and(|v| v == "1" || v == "true");
    // The apps' Scan Now: started, not waited for (`library=`: that library's folders walked in
    // full, the others looked at quickly). Progress: `GET /v1/admin/scan/progress`.
    if let Some(id) = q.get("library").filter(|v| !v.is_empty()) {
        if !l.request_library_scan(id) {
            return Err(ApiError(StatusCode::NOT_FOUND, "no such library".into()));
        }
        tracing::info!(library = %id, "scan asked for");
        return Ok(Json(json!({"started": true, "library": id})));
    }
    if q.get("wait").is_some_and(|v| v == "0" || v == "false") {
        l.request_pass(full);
        tracing::info!(full, "scan asked for");
        return Ok(Json(json!({"started": true, "full": full})));
    }
    if q.get("reidentify").is_some_and(|v| v == "1") {
        l.db.call(|c| store::forget_identified(c)).await.map_err(internal)?;
    }
    let r = l.pass(full).await.map_err(internal)?;
    if q.get("metadata").is_some_and(|v| v == "force") {
        let n = l.refresh_metadata(true).await.map_err(internal)?;
        l.db.call(search::rebuild).await.map_err(internal)?;
        return Ok(Json(json!({"pass": r, "metadata_forced": n})));
    }
    Ok(Json(json!(r)))
}

async fn admin_scan_progress(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    admin_authed(&st, &headers, &q).await?;
    Ok(Json(lib(&st)?.progress().await.map_err(internal)?))
}

async fn admin_stats(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    admin_authed(&st, &headers, &q).await?;
    let l = lib(&st)?;
    let recent = q.get("recent").and_then(|h| h.parse().ok());
    Ok(Json(l.stats(recent).await.map_err(internal)?))
}

pub(super) async fn admin_review(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    admin_authed(&st, &headers, &q).await?;
    let l = lib(&st)?;
    let llm = q.get("source").is_some_and(|s| s == "llm");
    let mut out = l.db.call(move |c| store::review(c, llm)).await.map_err(internal)?;
    if server_posters(&q) {
        for item in out["items"].as_array_mut().into_iter().flatten() {
            match_posters::to_server_posters(&mut item["candidates"]);
        }
    }
    Ok(Json(out))
}

/// `server_posters=1`: an app that loads candidates' posters from this server
/// (`GET /v1/admin/match/poster`). Without it, candidates are as they always were.
fn server_posters(q: &HashMap<String, String>) -> bool {
    q.get("server_posters").is_some_and(|v| v == "1")
}

/// Fix Match: `{tmdb_id}` (a movie's, or a show's: its episodes follow), for an episode also
/// `season`/`episode` (TMDB's numbering; default: the file's), or `{local: true}`: not matched,
/// named from the files and folders (never looked up again). `type: "movie" | "episode"` makes
/// it that (a movie that is an episode: `tmdb_id` of the show with `season` and `episode`, or
/// `local`; an episode or a whole show that is a movie: `tmdb_id` of the movie, or `local`).
/// An episode matched to one that has files already: `part: <n>` (or `true`: the next) makes it
/// that part of the episode; without, it is another version of it.
/// `{automatic: true}`: back to automatic: the choices made here for the title's files (a show's
/// or season's: its episodes') are forgotten and the server identifies them on its own again.
/// `file_id`: only that one of the title's files. `{split: true, file_id}`: that file becomes a
/// title of its own (named from the file; Fix Match it to say what it is).
/// Waits up to 40 s for the pass that applies it (`pending: true` when it is still running);
/// answers the title's new id (`file_item`: the id of the title the file is on now).
async fn admin_identify(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    admin_authed(&st, &headers, &q).await?;
    let l = lib(&st)?.clone();
    let local = body["local"].as_bool() == Some(true);
    let automatic = body["automatic"].as_bool() == Some(true);
    let split = body["split"].as_bool() == Some(true);
    let tmdb_id = body["tmdb_id"].as_i64().filter(|i| *i > 0);
    let file_id = body["file_id"].as_str().map(String::from);
    if file_id
        .as_deref()
        .is_some_and(|f| f.is_empty() || f.len() > 64 || !f.chars().all(|c| c.is_ascii_alphanumeric()))
    {
        return Err(ApiError(StatusCode::BAD_REQUEST, "file_id: one of the title's files".into()));
    }
    if split && file_id.is_none() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "split needs file_id: the file to split off".into(),
        ));
    }
    if !local && !automatic && !split && tmdb_id.is_none() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "tmdb_id (integer), local: true, automatic: true or split: true is required".into(),
        ));
    }
    let want = match body["type"].as_str() {
        None => None,
        Some("movie") => Some("movie"),
        Some("episode" | "series" | "show" | "tv") => Some("episode"),
        Some(_) => return Err(ApiError(StatusCode::BAD_REQUEST, "type: movie or episode".into())),
    };
    let num = |k: &str| -> Result<Option<i64>, ApiError> {
        match &body[k] {
            Value::Null => Ok(None),
            v => v
                .as_i64()
                .filter(|n| (0..10_000).contains(n))
                .map(Some)
                .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, format!("{k}: a number"))),
        }
    };
    let (want_season, want_episode) = (num("season")?, num("episode")?);
    // A part: a number (1…), or `true`: after the parts (or the one file) the episode has.
    let part: Option<Option<i64>> = match &body["part"] {
        Value::Null | Value::Bool(false) => None,
        Value::Bool(true) => Some(None),
        v => Some(Some(v.as_i64().filter(|n| (1..100).contains(n)).ok_or_else(|| {
            ApiError(StatusCode::BAD_REQUEST, "part: a number from 1, or true".into())
        })?)),
    };
    let rid = resolve_id(&l, &id)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such item".into()))?;
    let choice = store::MatchChoice {
        file: file_id.clone(),
        automatic,
        split,
        want,
        tmdb_id,
        season: want_season,
        episode: want_episode,
        part,
    };
    let r2 = rid.clone();
    let (n, kind) = l.db.call(move |c| store::apply_match(c, &r2, &choice)).await.map_err(internal)?;
    if kind.is_empty() {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such item".into()));
    }
    match n {
        store::NOT_ITS_FILE => {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "file_id: not one of this movie's or episode's files".into(),
            ))
        }
        store::NEEDS_EPISODE => {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "an episode of a show needs season and episode".into(),
            ))
        }
        store::NEEDS_SHOW => {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "an episode is matched to an episode of a show (tmdb_id with season and episode)".into(),
            ))
        }
        0 if automatic => {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "nothing was matched by hand here: it is identified automatically already".into(),
            ))
        }
        0 => return Err(ApiError(StatusCode::CONFLICT, "this title has no files to match".into())),
        _ => {}
    }
    tracing::info!(item = %rid, kind, as_type = want, tmdb_id, season = want_season, episode = want_episode, part = ?part, local, automatic, split, file = ?file_id, files = n, "matched by hand");
    let l2 = l.clone();
    let pass = tokio::spawn(async move { l2.pass(false).await });
    let (pending, r) = match tokio::time::timeout(std::time::Duration::from_secs(40), pass).await {
        Ok(Ok(r)) => (false, Some(r.map_err(internal)?)),
        Ok(Err(e)) => return Err(ApiError::internal(e.into())),
        Err(_) => (true, None),
    };
    let new_id = l.resolve_id(&rid).await.map_err(internal)?;
    // Where the file asked about is now (split off, matched apart).
    let file_item = match file_id {
        Some(f) => l.db.call(move |c| store::file_item(c, f)).await.map_err(internal)?,
        None => None,
    };
    Ok(Json(
        json!({"files": n, "item": new_id, "file_item": file_item, "pending": pending, "pass": r}),
    ))
}

/// Fix Match's searches (admins): `GET /v1/admin/match/search?kind=movie|tv&q=&year=` TMDB's
/// results `[{tmdb_id, title, original_title, year, overview, poster}]`;
/// `GET /v1/admin/match/episodes?tmdb_id=&season=` a TMDB season's episodes (`seasons`: the
/// show's season numbers); `GET /v1/admin/items/{id}/match` a title as Fix Match shows it, with
/// its stored candidates, or (`?compute=1`, none stored) candidates found now.
async fn match_search(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    admin_authed(&st, &headers, &q).await?;
    let l = lib(&st)?;
    let tmdb = l
        .tmdb()
        .ok_or_else(|| ApiError(StatusCode::CONFLICT, "no TMDB key: titles can't be looked up".into()))?;
    let kind = match q.get("kind").map(String::as_str) {
        Some("tv" | "series" | "show" | "shows") => "tv",
        _ => "movie",
    };
    let text = q
        .get("q")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "q: what to search for".into()))?;
    let year = q
        .get("year")
        .and_then(|y| y.parse::<i32>().ok())
        .filter(|y| (1870..2100).contains(y));
    let mut results = tmdb.search(kind, text, year).await.map_err(internal)?;
    if results.is_empty() && year.is_some() {
        results = tmdb.search(kind, text, None).await.map_err(internal)?;
    }
    let mut items: Vec<Value> = results.iter().take(20).map(|r| super::identify::candidate_json(kind, r)).collect();
    if kind == "movie" {
        movies_in_library(l, &mut items).await?;
    }
    let mut items = Value::Array(items);
    if server_posters(&q) {
        match_posters::to_server_posters(&mut items);
    }
    let total = items.as_array().map_or(0, Vec::len);
    Ok(Json(json!({"items": items, "total": total})))
}

/// A Fix Match candidate's poster (admins): `GET /v1/admin/match/poster?path=/name.jpg&size=w185|w342`
/// (`poster_path` of a candidate asked for with `server_posters=1`). Not a proxy: see
/// `match_posters.rs`. 400: not a TMDB poster; 502: TMDB didn't give it (the app shows its
/// placeholder).
pub(super) async fn match_poster(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Response, ApiError> {
    admin_authed(&st, &headers, &q).await?;
    let l = lib(&st)?;
    let path = q
        .get("path")
        .map(String::as_str)
        .filter(|p| match_posters::valid_path(p))
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "path: a TMDB poster's file path".into()))?;
    let size = match q.get("size").map(String::as_str) {
        None => "w342",
        Some(s) if match_posters::SIZES.contains(&s) => s,
        Some(_) => return Err(ApiError(StatusCode::BAD_REQUEST, "size: w185 or w342".into())),
    };
    if l.tmdb().is_none() {
        return Err(ApiError(StatusCode::CONFLICT, "no TMDB key: titles can't be looked up".into()));
    }
    match l.match_posters.get(path, size).await {
        Ok((bytes, ctype)) => Ok((
            [
                (header::CONTENT_TYPE, ctype),
                (header::CACHE_CONTROL, "private, max-age=2592000"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            bytes,
        )
            .into_response()),
        Err(e) => Err(ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}"))),
    }
}

/// Marks the TMDB movies (`tmdb_id`) the library has already: `in_library {item, files}`
/// (matching one of them makes another version of it).
async fn movies_in_library(l: &Library, items: &mut [Value]) -> Result<(), ApiError> {
    let keys: Vec<String> = items
        .iter()
        .filter_map(|i| i["tmdb_id"].as_i64())
        .map(|t| format!("tmdb:movie:{t}"))
        .collect();
    let have: HashMap<String, (String, i64)> = l.db.call(move |c| store::titles_by_key(c, keys)).await.map_err(internal)?;
    for i in items.iter_mut() {
        if let Some((item, files)) = i["tmdb_id"].as_i64().and_then(|t| have.get(&format!("tmdb:movie:{t}"))) {
            i["in_library"] = json!({"item": item, "files": files});
        }
    }
    Ok(())
}

async fn match_episodes(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<Value>, ApiError> {
    admin_authed(&st, &headers, &q).await?;
    let l = lib(&st)?;
    let tmdb = l
        .tmdb()
        .ok_or_else(|| ApiError(StatusCode::CONFLICT, "no TMDB key: titles can't be looked up".into()))?;
    let id = q
        .get("tmdb_id")
        .and_then(|v| v.parse::<i64>().ok())
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "tmdb_id: a show's TMDB id".into()))?;
    let show = tmdb.tv(id, super::tmdb::DAY).await.map_err(internal)?;
    let seasons: Vec<i64> = show["seasons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s["season_number"].as_i64())
        .collect();
    let season = q
        .get("season")
        .and_then(|v| v.parse::<i64>().ok())
        .or_else(|| seasons.iter().copied().find(|s| *s > 0))
        .unwrap_or(1);
    let sv = tmdb.season(id, season, super::tmdb::DAY).await.map_err(internal)?;
    // What the library has of each episode: its titles (parts) and their files.
    let key = format!("tmdb:tv:{id}");
    let have: HashMap<i64, (i64, i64)> = l.db.call(move |c| store::season_files(c, key, season)).await.map_err(internal)?;
    let episodes: Vec<Value> = sv["episodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            let n = e["episode_number"].as_i64().unwrap_or(-1);
            let (titles, files) = have.get(&n).copied().unwrap_or((0, 0));
            json!({"episode": e["episode_number"], "season": season, "title": e["name"], "air_date": e["air_date"],
                   "overview": e["overview"].as_str().map(|o| o.chars().take(200).collect::<String>()),
                   "in_library": {"titles": titles, "files": files}})
        })
        .collect();
    Ok(Json(
        json!({"tmdb_id": id, "title": show["name"], "seasons": seasons, "season": season, "episodes": episodes}),
    ))
}

async fn item_match(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Value>, ApiError> {
    admin_authed(&st, &headers, &q).await?;
    let l = lib(&st)?;
    let rid = resolve_id(l, &id)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such item".into()))?;
    let r2 = rid.clone();
    let mut v =
        l.db.call(move |c| store::match_item(c, &r2))
            .await
            .map_err(internal)?
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such item".into()))?;
    // The choices made by hand here (Back to Automatic forgets them), and a movie's or episode's
    // files one by one (each can be split off or matched on its own).
    let r3 = rid.clone();
    let (manual, files): (i64, Vec<Value>) = l.db.call(move |c| store::match_files(c, &r3)).await.map_err(internal)?;
    v["manual"] = json!(manual > 0 || v["state"] == "manual");
    v["file_list"] = json!(files);
    let none_stored = v["candidates"].as_array().is_none_or(Vec::is_empty);
    if none_stored && q.get("compute").is_some_and(|c| c == "1") && matches!(v["kind"].as_str(), Some("movie" | "series")) {
        v["candidates"] = json!(l.match_candidates(&rid).await.map_err(internal)?);
        v["computed"] = json!(true);
    }
    if v["kind"] == "movie" {
        if let Some(c) = v["candidates"].as_array_mut() {
            movies_in_library(l, c).await?;
        }
    }
    if server_posters(&q) {
        match_posters::to_server_posters(&mut v["candidates"]);
    }
    v["tmdb"] = json!(l.tmdb().is_some());
    Ok(Json(v))
}

/// Latest grouping over (episode, series, added_at) rows of each series' latest arrivals, newest
/// first: a series with one episode among them is that episode, with more the series; at most
/// `limit`, in order of each one's newest addition.
fn group_latest(rows: &[(String, String, i64)], limit: usize) -> Vec<(String, i64)> {
    let mut order: Vec<(&str, i64, Vec<&str>)> = Vec::new();
    for (ep, series, at) in rows {
        match order.iter_mut().find(|g| g.0 == series) {
            Some(g) => g.2.push(ep),
            None => order.push((series, *at, vec![ep])),
        }
    }
    order
        .into_iter()
        .take(limit)
        .map(|(series, at, eps)| (if eps.len() == 1 { eps[0].to_string() } else { series.to_string() }, at))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `watching=1` answers without the newest-titles rows, keeping their keys.
    #[tokio::test]
    async fn home_can_skip_the_latest_rows() {
        use crate::{admin::config::Config, db::Db, storage::cache::Cache};
        let dir = std::env::temp_dir().join(format!("moonrise-home-latest-{}", std::process::id()));
        let r = dir.to_string_lossy().into_owned();
        let config = Config::from_lookup(|k| match k {
            "MOONRISE_LIBRARIES" => Some(format!("Films=movies:{r}/films")),
            "MOONRISE_DATA_DIR" => Some(format!("{r}/data")),
            "MOONRISE_CACHE_DIR" => Some(format!("{r}/cache")),
            _ => None,
        });
        let db = Db::memory().unwrap();
        let cache = std::sync::Arc::new(Cache::new(dir.join("cache"), 1 << 30));
        let l = Library::new(config, db.clone(), cache);
        let id = l.libs()[0].id.clone();
        db.call(move |c| {
            c.execute(
                "INSERT INTO libraries(id, name, kind, roots, sort) VALUES (?1, 'Films', 'movies', '[]', 0)",
                [id],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let (types, latest) = home_latest(&l, None, 20, false).await.unwrap();
        assert_eq!(types.as_array().unwrap().len(), 2, "movies and shows");
        assert_eq!(latest.as_object().unwrap().len(), 1, "one row per library");

        let (types, latest) = home_latest(&l, None, 20, true).await.unwrap();
        assert_eq!(types, json!([]));
        assert_eq!(latest, json!({}));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn latest_grouping() {
        let r = |e: &str, s: &str, at: i64| (e.to_string(), s.to_string(), at);
        let rows = vec![r("e1", "a", 9), r("e2", "b", 8), r("e3", "a", 7), r("e4", "c", 6)];
        assert_eq!(
            group_latest(&rows, 10),
            vec![("a".to_string(), 9), ("e2".to_string(), 8), ("e4".to_string(), 6)]
        );
        assert_eq!(group_latest(&rows, 1), vec![("a".to_string(), 9)]);
    }

    #[test]
    fn chapters_of_a_file_with_a_start_time_are_on_its_timeline() {
        // A TV recording remuxed into Matroska keeping its broadcast clock (start 1002.597 s),
        // with chapters on that clock as Matroska has them. Players count the start as 0:
        // before 0.31.11 every chapter was listed 1002.6 s late, past the title's end.
        let chapters = json!([
            {"start_time": "1002.597000", "tags": {"title": "Chapter 1"}},
            {"start_time": "1032.597000", "tags": {"title": "Intro"}},
            {"start_time": "1152.597000", "tags": {"title": "End Credits"}},
        ])
        .to_string();
        let probe = json!({"format": {"format_name": "matroska,webm", "start_time": "1002.597000", "duration": "1182.960000"}});
        assert_eq!(
            item_chapters(Some(&chapters), Some(&probe)),
            json!([{"start": 0.0, "name": "Chapter 1"}, {"start": 30.0, "name": "Intro"}, {"start": 150.0, "name": "End Credits"}])
        );
        let ch: Vec<Value> = serde_json::from_str(&chapters).unwrap();
        assert_eq!(chapter_starts(&ch, crate::media::OwnClock::of(&probe)), vec![0.0, 30.0, 150.0]);
        // The same file starting at 0, or a frame later: as they are.
        for start in ["0.000000", "0.080000"] {
            let probe = json!({"format": {"format_name": "matroska,webm", "start_time": start, "duration": "180.36"}});
            assert_eq!(
                item_chapters(Some(&chapters), Some(&probe)),
                json!([{"start": 1002.597, "name": "Chapter 1"}, {"start": 1032.597, "name": "Intro"}, {"start": 1152.597, "name": "End Credits"}])
            );
        }
        // Chapters written from 0 on a file with a start time are already on its timeline.
        let from_zero =
            json!([{"start_time": "0.000000", "tags": {"title": "Chapter 1"}}, {"start_time": "30.000000", "tags": {"title": "Intro"}}])
                .to_string();
        assert_eq!(
            item_chapters(Some(&from_zero), Some(&probe)),
            json!([{"start": 0.0, "name": "Chapter 1"}, {"start": 30.0, "name": "Intro"}])
        );
        assert_eq!(item_chapters(None, Some(&probe)), json!([]));
    }

    #[test]
    fn segment_names() {
        assert_eq!(segment_kind("Intro"), Some("intro"));
        assert_eq!(segment_kind("Opening Credits"), Some("intro"));
        assert_eq!(segment_kind("Previously on"), Some("recap"));
        assert_eq!(segment_kind("End Credits"), Some("credits"));
        assert_eq!(segment_kind("Chapter 3"), None);
        assert_eq!(segment_kind("Opera"), None);
        // A language tag before the name (Guardians of the Galaxy: "en:End Credits").
        assert_eq!(segment_kind("en:End Credits"), Some("credits"));
        assert_eq!(segment_kind("eng: Intro"), Some("intro"));
        assert_eq!(segment_kind("en:Chapter 12"), None);
        assert_eq!(segment_kind("Part 2: Credits"), None, "only a language tag is dropped");
    }

    /// `segments?merged=1`: the file's segments with the databases', by the apps' rules;
    /// `pending` held, `provisional` uses the databases' credits meanwhile.
    #[test]
    fn merged_segments() {
        use crate::media::segment_rules::Segment;
        let file = vec![
            json!({"kind": "intro", "start": 0.0, "end": 52.0, "source": "chapters"}),
            json!({"kind": "credits", "start": 6796.622, "end": 7210.828, "source": "chapters"}),
        ];
        let online = community::Answer {
            online: vec![
                vec![Segment::new("recap", 0.0, 40.0, Some("theintrodb"))],
                vec![Segment::new("credits", 6852.0, 7210.0, Some("introdb"))],
            ],
            pending: false,
        };
        let a = merged_answer(&file, &online, 7268.64, false, false, true);
        assert_eq!(
            a,
            json!({"items": [
                {"kind": "intro", "start": 0.0, "end": 52.0, "source": "chapters"},
                {"kind": "recap", "start": 0.0, "end": 40.0, "source": "theintrodb"},
                {"kind": "credits", "start": 6852.0, "end": 7210.828, "source": "chapters"},
            ], "total": 3, "pending": false, "merged": true, "community_pending": false})
        );
        // An episode while the file is still looked at: the databases' credits wait, unless provisional.
        let detected = community::Answer {
            online: vec![vec![], vec![Segment::new("credits", 6852.0, 7210.0, Some("introdb"))]],
            pending: true,
        };
        let a = merged_answer(&[], &detected, 7268.64, true, false, false);
        assert_eq!(a["items"], json!([]));
        assert_eq!((a["pending"].clone(), a["community_pending"].clone()), (json!(true), json!(true)));
        // Provisional (a download): the databases' credits meanwhile, once the lookup is done.
        let a = merged_answer(
            &[],
            &community::Answer {
                pending: false,
                ..detected
            },
            7268.64,
            true,
            true,
            false,
        );
        assert_eq!(
            a["items"],
            json!([{"kind": "credits", "start": 6852.0, "end": 7210.0, "source": "introdb"}])
        );
        // While the lookup is pending, the file's credits chapter (6796, before IntroDB's
        // 6852, not known yet) isn't offered; the intro chapter is.
        let a = merged_answer(
            &file,
            &community::Answer {
                online: vec![vec![], vec![]],
                pending: true,
            },
            7268.64,
            false,
            false,
            true,
        );
        assert_eq!(
            a,
            json!({"items": [{"kind": "intro", "start": 0.0, "end": 52.0, "source": "chapters"}],
                   "total": 1, "pending": false, "merged": true, "community_pending": true})
        );
    }
}
