//! HTTP API.
//!
//! - `GET /health`: exactly `ok` (the app checks for that string).
//! - `GET /v1/info` (public): `{server_id, name, version, capabilities[], setup}` (`setup`:
//!   `unclaimed` or `claimed`, see `claim.rs`).
//! - `GET /v1/items/{id}/playback?start=&hdr=&quality=&abr=` (token): what can
//!   be served and the master playlist URL. Opening it makes a playback session: the stream
//!   URLs carry its random id instead of the token (AVPlayer and AirPlay receivers fetch them
//!   without headers).
//!   - `start` (seconds): where the playlist starts (`EXT-X-START`).
//!   - `dv=1`: the player shows Dolby Vision: a Dolby Vision title's copy says so
//!     (`SUPPLEMENTAL-CODECS` for profile 8.1/8.4, `dvh1` for profile 5, and the configuration
//!     box in its init section, `Dovi::signal`), so the display switches to Dolby Vision. Without
//!     it, the base layer plays as HDR10/HLG, as before (capability `dolby-vision`).
//!   - `hdr=0`: the display can't show HDR: HDR titles are tone-mapped to SDR (transcoded).
//!   - `quality` (bits/s, or `original`): the variant played, from the title's ladder
//!     (`variants[]`, `variant`): the best one within that bitrate.
//!   - `abr=1`: the master lists the lower variants too, for adaptive switching.
//!   - `lossless=1`: the player decodes FLAC in fMP4 and its audio output takes more than 2
//!     channels: lossless multichannel tracks (TrueHD, DTS-HD MA, PCM…) are converted to FLAC
//!     with every channel (up to 7.1) instead of E-AC-3 5.1 (capability `lossless-audio`).
//!
//!   Track indices (`audio[].index`, `subtitles[].index`, and the `audio/{index}` and
//!   `subs/{index}` URLs) are the library's (sidecar files first); `stream` is the file's own.
//!   `audio[].converted` marks tracks served converted (AAC/E-AC-3). `method` is "copy",
//!   "convert" (some audio converted) or "transcode" (the video variant served is re-encoded);
//!   `video` describes the variant served. `pending: true` means the keyframe index is still
//!   being made: ask again later. `supported: false` with `reasons` when it can't be served as
//!   asked (including when the transcode slots are all in use and there is no copy to offer).
//! - `GET /v1/items/{id}/master.m3u8?…` (token): the same, straight to the playlist.
//! - `GET /v1/s/{sid}/…`: master, video (per variant)/audio/subtitle playlists, init sections,
//!   segments. An unknown session id (expired, closed, or made before the server restarted)
//!   answers `410 Gone`, so a player knows to ask `/playback` for a new session (a `404` means
//!   the session is there but the file isn't).
//! - `DELETE /v1/s/{sid}`: playback ended: stop the session's transcode and conversion runs.
//! - `GET /v1/jobs`: transcode runs in progress (for checks).
//!
//! Scrubbing thumbnails (`trickplay.rs`, capability `trickplay`):
//! - `GET /v1/items/{id}/trickplay?version=&sid=` (token, or `sid=` of a session playing it):
//!   `{state: ready|pending|off, interval, width, height, columns, rows, count, sheets[]}`.
//! - `GET /v1/trickplay/{key}/{n}.jpg` (token, or `sid=`): a sheet of `columns` × `rows`.
//! - `trick=1` on `/playback`: the master lists the thumbnails as an `EXT-X-I-FRAME-STREAM-INF`
//!   stream (`/v1/s/{sid}/trickplay/iframes.m3u8` and `.mp4`) once they are made.
//!
//! Subtitles (`subs.rs`, `opensubtitles.rs`):
//! - `GET /v1/items/{id}/subtitles` (token): the title's subtitle tracks (as in the playback
//!   answer: embedded, sidecar and fetched ones; `url` for text tracks).
//! - `POST /v1/items/{id}/subtitles/upload` (token, JSON `{srt, language?, name?}`): a device's
//!   subtitle file becomes a track of the title (stored once per text), answers the track.
//! - `GET /v1/items/{id}/subtitles/{index}.srt|.vtt` (token, or `sid=` of a session playing
//!   the title): a text track.
//! - `GET /v1/items/{id}/subtitles/{index}/cues?window=<k>|at=<seconds>` (token, or `sid=`):
//!   the cues showing during window `k` (30 s), JSON, read for that window alone while the
//!   track isn't extracted (`subs::Subs::window`). Capability `subtitle-windows`. A picture
//!   track (PGS, DVD, DVB) answers the same way with images: `canvas {width, height}`, and
//!   each cue's `image {x, y, width, height, png}` (base64), which the app draws over the
//!   video (`pictures.rs`). Capability `subtitle-pictures`.
//! - `GET /v1/items/{id}/subtitles/search?lang=&q=` (token): OpenSubtitles results for the
//!   title (by its ids and the file's hash; `q` searches by text instead).
//! - `POST /v1/items/{id}/subtitles/fetch {result_id}` (token): fetch one; it becomes a track
//!   of the title for every device. Answers the track.
//! - `GET /v1/items/{id}/subtitles/{index}/timing` (token): how the track's timing stands: its
//!   match to this file (`subsync.rs`, downloaded tracks only; asking starts it, and `state` is
//!   "pending" while it runs) and the viewer's nudge. `PUT` with `{nudge?, original?}` keeps a nudge, or the original timing, per file and
//!   track for every device. Capability `subtitle-sync`.
//!
//! Downloads (`downloads.rs`):
//! - `GET /v1/items/{id}/download` (token): the original file (`Range`, `ETag`, `If-Range`).
//! - `POST /v1/items/{id}/download-jobs {quality, audio[], hdr, container: "mp4"}` (token): a
//!   prepared progressive MP4; `202` with the job (or `200` with an existing one for the same
//!   file and options), `507` when the download space is full, `422` when it can't be made.
//! - `GET /v1/download-jobs` (token): jobs, and the space they take; `GET /v1/download-jobs/{job}`
//!   (token): state and progress; `GET /v1/download-jobs/{job}/file` (token, or the job id
//!   alone: it is random): the file (`Range`); `DELETE /v1/download-jobs/{job}` (token).
//! - `GET /v1/items/{id}/offline?quality=&hdr=&audio=` (token) and its files: segmented
//!   downloads the app plays while they come in (`offline.rs`).

use std::fmt::Write as _;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Result;
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use bytes::Bytes;
use rand::Rng;
use serde::Serialize;

use crate::{
    admin::config::{Config, Feature},
    auth::Caller,
    db::Db,
    downloads::{self, Downloads, JobParams},
    media::audio::Converter,
    media::scan::Scanner,
    media::segment::{Segmenter, Track},
    media::transcode::{self, Transcoder, Variant},
    media::{DoviSignal, LoadCtx, Media, SubtitleTrack},
    storage::cache::Cache,
    subtitles::opensubtitles::{self, OpenSubtitles},
    subtitles::subs::{self, Subs},
};

pub struct AppState {
    /// As the server started (the feature switches in it follow the settings that apply live).
    pub config: Config,
    /// The settings store and the configuration in effect (`admin/settings.rs`, `admin/mod.rs`).
    pub settings: Arc<crate::admin::settings::Settings>,
    /// The Bonjour announcement (re-made when the server's name changes).
    mdns: Mutex<Option<crate::mdns::Announcement>>,
    /// For the reads that go through `Source::read_at`.
    pub http: reqwest::Client,
    pub cache: Arc<Cache>,
    pub segmenter: Segmenter,
    pub converter: Arc<Converter>,
    pub transcoder: Arc<Transcoder>,
    pub scanner: Arc<Scanner>,
    /// Keyframe indexes made ahead of need (`index_backfill.rs`).
    pub index_backfill: Arc<crate::media::index_backfill::IndexBackfill>,
    /// End credits looked for in episodes' files (`credits.rs`).
    pub credits: Arc<crate::media::credits::Credits>,
    /// Scrubbing thumbnails made ahead of need (`trickplay.rs`).
    pub trickplay: Arc<crate::media::trickplay::Trickplay>,
    pub subs: Arc<Subs>,
    pub opensubtitles: Arc<OpenSubtitles>,
    pub downloads: Arc<Downloads>,
    /// None when the database is switched off or couldn't be opened.
    pub db: Option<Db>,
    /// The library (scanner, identification, metadata, browse API); None when switched off.
    pub library: Option<Arc<crate::library::Library>>,
    /// The CLI's local admin token (`/data/secrets/admin.token`).
    pub admin_token: Option<String>,
    /// Moonrise's own accounts and tokens (None: switched off, or no database).
    pub auth: Option<Arc<crate::auth::Auth>>,
    /// Claiming a new server with its setup code (None: no accounts).
    pub claim: Option<Arc<crate::admin::claim::Claim>>,
    pub server_id: String,
    /// item -> media, loaded at
    media: Arc<Mutex<MediaCache>>,
    /// One load per title at a time (concurrent requests share it).
    loading: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    sessions: Mutex<HashMap<String, Session>>,
}

struct Session {
    view: View,
    used: Instant,
}

/// A session as the stream handlers see it.
#[derive(Clone)]
struct View {
    sid: String,
    /// The Moonrise user who opened it (their reports are recorded).
    owner: Option<String>,
    /// The Moonrise id of the title (for watch state).
    moonrise_item: Option<String>,
    media: Arc<Media>,
    opts: StreamOptions,
    /// The title's ladder for this session's options; `selected` is played.
    variants: Arc<Vec<Variant>>,
    selected: usize,
}

impl View {
    /// How the copy tells this player about Dolby Vision (None: it doesn't).
    fn dovi(&self) -> Option<(crate::media::Dovi, crate::media::DoviSignal)> {
        let d = self.media.video.as_ref()?.dovi.filter(|_| self.opts.dv && self.opts.hdr_display)?;
        Some((d, d.signal()?))
    }

    fn variant(&self, id: &str) -> Result<&Variant, ApiError> {
        self.variants
            .iter()
            .find(|v| v.id == id)
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such variant".into()))
    }

    /// The segment the session starts at (`start` is on the title's timeline, the boundaries on
    /// the file's own clock).
    fn start_segment(&self) -> usize {
        let start = self.media.file_time(self.opts.start.unwrap_or(0.0));
        self.media.boundaries.iter().rposition(|b| *b <= start).unwrap_or(0)
    }
}

/// What the client asked for when opening the stream.
#[derive(Clone, Copy)]
struct StreamOptions {
    /// The player shows Dolby Vision (`Dovi::signal`).
    dv: bool,
    start: Option<f64>,
    /// The display can show HDR (`hdr=0` says it can't). AVPlayer refuses a master playlist
    /// whose only variant is HDR on other displays: HDR titles are tone-mapped for those.
    hdr_display: bool,
    /// Bits/s (None: the original).
    quality: Option<i64>,
    /// List the lower variants in the master too.
    abr: bool,
    /// The player takes lossless multichannel audio as FLAC (`Media::with_lossless`).
    lossless: bool,
    /// The player shows a scrub preview from an I-frame stream: the master lists the title's
    /// thumbnail track when it has one (`trickplay.rs`).
    trick: bool,
}

impl StreamOptions {
    fn from(q: &HashMap<String, String>) -> Self {
        StreamOptions {
            dv: q.get("dv").is_some_and(|v| v == "1"),
            start: q.get("start").and_then(|s| s.parse::<f64>().ok()).filter(|s| *s > 1.0),
            hdr_display: q.get("hdr").is_none_or(|v| v != "0"),
            quality: q.get("quality").and_then(|s| s.parse::<i64>().ok()).filter(|b| *b > 0),
            abr: q.get("abr").is_some_and(|v| v == "1"),
            lossless: q.get("lossless").is_some_and(|v| v == "1"),
            trick: q.get("trick").is_some_and(|v| v == "1"),
        }
    }
}

type MediaCache = HashMap<String, (Arc<Media>, Instant)>;

const MEDIA_TTL: Duration = Duration::from_mins(30);
const SESSION_TTL: Duration = Duration::from_hours(24);
/// How long a WebVTT rendition request waits for its track to be extracted.
const VTT_WAIT: Duration = Duration::from_secs(12);
/// Subtitle rendition segments (seconds), and how long one waits for its track.
const SUBS_SEGMENT: f64 = subs::WINDOW;
const VTT_SEGMENT_WAIT: Duration = Duration::from_secs(6);
/// Segments cut ahead of the one requested.
const LOOKAHEAD: usize = 2;

impl AppState {
    /// `env`: the environment; the settings store in the database is read (and, the first time,
    /// filled from the environment) before anything else is set up.
    pub async fn new(env: crate::admin::config::Env) -> Result<Self> {
        // The environment alone decides where the database is and whether there is one.
        let config = Config::resolve(&env, &crate::admin::settings::Stored::default());
        // The server keeps serving without its database (nothing in it is essential yet).
        let db = if config.enabled(Feature::Db) {
            match Db::open(&config.data_dir.join("moonrise.db")) {
                Ok(db) => Some(db),
                Err(e) => {
                    tracing::error!("database unavailable, running without it: {e:#}");
                    None
                }
            }
        } else {
            None
        };
        if let Some(db) = &db {
            crate::llm::chatgpt::init(db.clone());
        }
        let settings = match crate::admin::settings::Settings::open(env.clone(), db.clone()).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("settings store unavailable, using the environment only: {e:#}");
                crate::admin::settings::Settings::open(env, None).await?
            }
        };
        let settings = Arc::new(settings);
        let config = settings.boot().clone();
        tracing::info!(?config, "configuration");
        let cache = Arc::new(Cache::new(config.cache_dir.clone(), config.cache_max_bytes));
        // SMB shares: their credentials, and the loopback listener ffmpeg reads them through.
        crate::storage::smb::client().configure(&settings.current().smb_shares, settings.current().smb_signing == "always");
        if let Err(e) = crate::storage::vfs::start_loopback().await {
            tracing::error!("SMB loopback listener: {e:#}");
        }
        let server_id = db
            .as_ref()
            .and_then(|db| db.server_id().ok())
            .unwrap_or_else(|| "ephemeral".to_string());
        let library = match &db {
            Some(db) if config.enabled(Feature::Library) => {
                let l = crate::library::Library::new(config.clone(), db.clone(), cache.clone());
                match l.init().await {
                    Ok(()) => Some(Arc::new(l)),
                    Err(e) => {
                        tracing::error!("library unavailable: {e:#}");
                        None
                    }
                }
            }
            _ => None,
        };
        let admin_token = admin_token(&config.data_dir);
        let auth = match &db {
            Some(db) if config.enabled(Feature::MoonriseAuth) => Some(Arc::new(crate::auth::Auth::new(db.clone()))),
            _ => None,
        };
        let claim = match (&auth, &db) {
            (Some(_), Some(db)) => {
                match crate::admin::claim::Claim::open(db.clone(), crate::admin::claim::page_url(config.listen.port())).await {
                    Ok(c) => Some(Arc::new(c)),
                    Err(e) => {
                        tracing::error!("setup state unavailable: {e:#}");
                        None
                    }
                }
            }
            _ => None,
        };
        if let Some(db) = &db {
            // Playback sessions older than 30 days aren't reported on any more.
            let cutoff = crate::library::util::now() - 30 * 86_400;
            if let Err(e) = db
                .call(move |c| Ok(c.execute("DELETE FROM play_sessions WHERE started_at < ?1", [cutoff])?))
                .await
            {
                tracing::warn!("play_sessions cleanup: {e:#}");
            }
        }
        tracing::info!(
            server_id,
            db = db.is_some(),
            schema = db.as_ref().and_then(|d| d.schema_version().ok()),
            "state"
        );
        Ok(AppState {
            converter: Arc::new(Converter::new(
                config.ffmpeg.clone(),
                config.ffprobe.clone(),
                cache.clone(),
                config.convert_jobs,
            )),
            transcoder: Arc::new(Transcoder::new(config.ffmpeg.clone(), cache.clone(), config.transcode_jobs)),
            scanner: Arc::new(Scanner::new(config.ffprobe.clone(), db.clone())),
            index_backfill: Arc::new(crate::media::index_backfill::IndexBackfill::default()),
            credits: Arc::default(),
            trickplay: Arc::new(crate::media::trickplay::Trickplay::new(
                &config.cache_dir,
                config.trickplay_max_bytes,
            )),
            subs: Arc::new(Subs::new(
                config.data_dir.join("subs"),
                config.ffmpeg.clone(),
                config.ffprobe.clone(),
                config.enabled(Feature::SubsExtract),
                config.disabled.clone(),
            )),
            opensubtitles: Arc::new(
                OpenSubtitles::new(config.opensubtitles.clone()).with_state_file(config.data_dir.join("opensubtitles-quota.json")),
            ),
            downloads: Arc::new(Downloads::new(
                &config.cache_dir,
                config.ffmpeg.clone(),
                config.download_max_bytes,
                config.download_jobs,
                config.enabled(Feature::Transcode),
            )),
            db,
            library,
            admin_token,
            auth,
            claim,
            server_id,
            settings,
            mdns: Mutex::new(None),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("http client"),
            segmenter: Segmenter::new(config.ffmpeg.clone(), cache.clone(), config.jobs),
            cache,
            config,
            media: Arc::new(Mutex::new(HashMap::new())),
            loading: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
        })
    }

    /// The server's name (a setting: it can change while running).
    pub fn name(&self) -> String {
        self.settings.current().name
    }

    /// (Re-)announces the server on the local network with Bonjour, or stops announcing when
    /// that is switched off. Blocking (a goodbye is sent first).
    pub fn announce(&self) {
        let mut cur = self.mdns.lock().unwrap();
        if let Some(a) = cur.take() {
            a.stop();
        }
        if !self.config.enabled(Feature::Mdns) {
            return;
        }
        let ifaces = std::env::var("MOONRISE_MDNS_INTERFACES").ok().filter(|s| !s.trim().is_empty());
        let unclaimed = self.claim.as_ref().is_some_and(|c| c.unclaimed());
        match crate::mdns::advertise(
            &self.name(),
            &self.server_id,
            env!("CARGO_PKG_VERSION"),
            self.config.listen.port(),
            ifaces.as_deref(),
            unclaimed,
        ) {
            Ok(a) => *cur = Some(a),
            Err(e) => tracing::warn!("mDNS: not announced: {e:#}"),
        }
    }

    /// A user's remembered version of a title (`PUT /v1/items/{id}/version`), as a media key
    /// (`<item>~<file>`): what playback without a `version` plays for them.
    pub(crate) async fn with_chosen_version(&self, who: &Caller, key: String) -> String {
        if key.contains('~') {
            return key;
        }
        let (Some(user), Some(db)) = (who.user_id(), self.db.clone()) else {
            return key;
        };
        let Some(l) = self.library.as_ref() else { return key };
        let Ok(rid) = l.resolve_id(&key).await else { return key };
        let chosen: Option<String> = db
            .call(move |c| {
                Ok(c.query_row(
                    &format!(
                        "SELECT v.file_id FROM user_versions v JOIN files f ON f.id = v.file_id AND f.item_id = v.item_id AND f.role IN {} AND f.missing_since IS NULL
                         WHERE v.user_id = ?1 AND v.item_id = ?2",
                        crate::library::TITLE_ROLES
                    ),
                    rusqlite::params![user, rid],
                    |r| r.get(0),
                )
                .map(Some)
                .or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e) })?)
            })
            .await
            .ok()
            .flatten();
        match chosen {
            Some(f) => format!("{key}~{f}"),
            None => key,
        }
    }

    /// The title as served: it must be in the library and on a mounted share. `item_id` may
    /// name one of its versions: `<item>~<file>` (`media_key`).
    pub(crate) async fn media(&self, item_id: &str) -> Result<Arc<Media>, ApiError> {
        let key = item_id.to_string();
        if let Some(m) = self.cached_media(&key) {
            return Ok(m);
        }
        // Requests for the same title (the app asks for playback info and the stream at once)
        // share one load.
        let lock = self.loading.lock().unwrap().entry(key.clone()).or_default().clone();
        let guard = lock.lock().await;
        let r = match self.cached_media(&key) {
            Some(m) => Ok(m),
            None => self.load_media(item_id, true).await,
        };
        drop(guard);
        let mut loading = self.loading.lock().unwrap();
        if loading.get(&key).is_some_and(|l| Arc::strong_count(l) <= 2) {
            loading.remove(&key);
        }
        r
    }

    fn cached_media(&self, key: &str) -> Option<Arc<Media>> {
        let cache = self.media.lock().unwrap();
        cache.get(key).filter(|(_, at)| at.elapsed() < MEDIA_TTL).map(|(m, _)| m.clone())
    }

    /// Loads a title (`keep`: into the cache, for the streams; the subtitle backfill doesn't).
    pub(crate) async fn load_media(&self, item_id: &str, keep: bool) -> Result<Arc<Media>, ApiError> {
        let key = item_id.to_string();
        let library = self
            .library
            .as_ref()
            .ok_or_else(|| ApiError(StatusCode::SERVICE_UNAVAILABLE, "the library is switched off".into()))?;
        let t0 = Instant::now();
        let mut item = match library.playback_item(item_id).await {
            Ok(Some(i)) => i,
            Ok(None) => return Err(ApiError(StatusCode::NOT_FOUND, "no such item".into())),
            Err(e) => return Err(ApiError(StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}"))),
        };
        let item_ms = t0.elapsed().as_millis() as u64;
        let t1 = Instant::now();
        let ctx = LoadCtx {
            http: &self.http,
            config: &self.config,
            db: self.db.as_ref().filter(|_| self.config.enabled(Feature::ProbeCache)),
            library_db: self.db.as_ref().filter(|_| self.library.is_some()),
            scanner: &self.scanner,
        };
        let mut media = Media::load(&item, &ctx).await.map_err(ApiError::internal)?;
        if media.damaged {
            // Found damaged just now (and marked `broken`): a healthy version of the title plays
            // instead, from this request on.
            let was = item.media_sources.first().map(|s| s.id.clone());
            if let Ok(Some(other)) = library.playback_item(item_id).await {
                let now = other.media_sources.first().map(|s| s.id.clone());
                if now != was {
                    tracing::info!(item = item_id, damaged = ?was, playing = ?now, "damaged file: playing another version of the title");
                    item = other;
                    media = Media::load(&item, &ctx).await.map_err(ApiError::internal)?;
                }
            }
        }
        // Tracks fetched from OpenSubtitles, as external tracks (names kept unique: the app
        // matches the HLS rendition to its track by name).
        for mut t in self.subs.extras(&media) {
            let base = t.name.clone();
            let mut n = 2;
            while media.subtitles.iter().any(|x| x.text && x.name == t.name) {
                t.name = format!("{base} ({n})");
                n += 1;
            }
            media.subtitles.push(t);
        }
        if keep {
            tracing::info!(item = item_id, name = ?item.name, source = media.source_kind, probe_cache = media.probe_cache, supported = media.supported,
                method = media.method, reasons = ?media.reasons, segments = media.boundaries.len(), item_ms, load_ms = t1.elapsed().as_millis() as u64,
                ms = t0.elapsed().as_millis() as u64, "loaded");
        }
        let media = Arc::new(media);
        if media.pending || media.look_again || !keep {
            // Not kept: the next request looks again (the scan's result lands in the database;
            // a damaged file is `broken` now, and another version of the title may play).
            return Ok(media);
        }
        {
            let mut cache = self.media.lock().unwrap();
            cache.retain(|_, (_, at)| at.elapsed() < MEDIA_TTL);
            cache.insert(key.clone(), (media.clone(), Instant::now()));
        }
        // The stamp came from the library: check the file itself, off the request's path, and
        // forget the load if it has changed since the last scan. A file copied over in place
        // keeps its folder's mtime, so quick passes don't list it: the library takes its new
        // size and mtime from here, and probes it again.
        if let Some(stamp) = item.media_sources.first().and_then(|ms| ms.stamp.zip(ms.path.clone())) {
            let ((size, mtime), path) = stamp;
            let me = self.media.clone();
            let library = library.clone();
            tokio::spawn(async move {
                let now = crate::storage::vfs::stat(&path).await;
                match now {
                    Ok(m) if (m.size, m.mtime) == (size, mtime) => {}
                    other => {
                        tracing::info!(item = %key, "file changed or gone since the last scan: loaded again on the next request ({:?})", other.as_ref().map(|m| (m.size, m.mtime)).map_err(ToString::to_string));
                        me.lock().unwrap().remove(&key);
                        if let Some(m) = other.ok().filter(|m| m.is_file) {
                            if let Err(e) = library.file_changed(path, m.size, m.mtime).await {
                                tracing::warn!("file changed: {e:#}");
                            }
                        }
                    }
                }
            });
        }
        Ok(media)
    }

    /// Authenticates a request (a Moonrise Bearer token, or the CLI's admin token).
    pub(crate) async fn authenticate(&self, headers: &HeaderMap, q: &HashMap<String, String>) -> Result<Caller, ApiError> {
        crate::auth::caller(self, headers, q).await
    }

    /// Forget the loaded media of a title (its tracks changed).
    fn invalidate(&self, item_id: &str) {
        let base = item_id.split('~').next().unwrap_or(item_id);
        self.media.lock().unwrap().retain(|i, _| i.split('~').next() != Some(base));
    }

    fn new_session(
        &self,
        sid: String,
        who: &Caller,
        moonrise_item: Option<String>,
        media: Arc<Media>,
        opts: StreamOptions,
        plan: &Plan,
    ) -> View {
        let view = View {
            sid: sid.clone(),
            owner: who.user_id(),
            moonrise_item,
            media,
            opts,
            variants: Arc::new(plan.variants.clone()),
            selected: plan.selected,
        };
        if let (Some(db), Some(user), Some(item)) = (self.db.clone(), view.owner.clone(), view.moonrise_item.clone()) {
            // Kept, so reports still land after a restart (the stream session itself doesn't).
            let (sid, file, duration, method) = (
                sid.clone(),
                view.media.file_id.clone(),
                view.media.duration,
                plan.variants.get(plan.selected).map(|v| v.id.clone()),
            );
            tokio::spawn(async move {
                let r = db
                    .call(move |c| {
                        Ok(c.execute(
                            "INSERT OR IGNORE INTO play_sessions(id, user_id, item_id, file_id, duration, method, started_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                            rusqlite::params![sid, user, item, file, duration, method, crate::library::util::now()],
                        )?)
                    })
                    .await;
                if let Err(e) = r {
                    tracing::warn!("play session: {e:#}");
                }
            });
        }
        // Its thumbnails first when the server is next idle (then the next episodes', below).
        if let Some(item) = &view.moonrise_item {
            self.trickplay.hint(vec![item.clone()]);
        }
        // Its text subtitles first (if not extracted yet), then the next episodes'.
        self.subs.play(&view.media, &sid);
        self.subs.streaming();
        if let (Some(db), Some(series)) = (self.db.clone(), view.media.item.series_id.clone()) {
            let (subs, ib, tp, item, current) = (
                self.subs.clone(),
                self.index_backfill.clone(),
                self.trickplay.clone(),
                view.media.item.clone(),
                view.moonrise_item.clone(),
            );
            tokio::spawn(async move {
                let (season, episode) = (item.parent_index_number.unwrap_or(0), item.index_number.unwrap_or(0));
                let next = db
                    .call(move |c| {
                        let mut q = c.prepare(
                            "SELECT id FROM items WHERE series_id = ?1 AND kind = 'episode' AND (season_no > ?2 OR (season_no = ?2 AND episode_no > ?3))
                             ORDER BY season_no, episode_no LIMIT 2",
                        )?;
                        let ids = q.query_map(rusqlite::params![series, season, episode], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
                        Ok(ids)
                    })
                    .await
                    .unwrap_or_default();
                ib.hint(next.clone());
                // The title playing stays first.
                tp.hint(current.into_iter().chain(next.iter().cloned()).collect());
                subs.hint(next);
            });
        }
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|_, s| s.used.elapsed() < SESSION_TTL);
        sessions.insert(
            sid,
            Session {
                view: view.clone(),
                used: Instant::now(),
            },
        );
        view
    }

    /// A session's owner, title and length, for playback reports: (owner, Moonrise item, duration).
    /// None: not open in memory (e.g. after a restart; `play_sessions` has it then).
    pub(crate) fn session_for_report(&self, sid: &str) -> Option<crate::userdata::store::LiveSession> {
        let sessions = self.sessions.lock().unwrap();
        sessions.get(sid).map(|s| {
            (
                s.view.owner.clone(),
                s.view.moonrise_item.clone(),
                s.view.media.duration,
                s.view.media.file_id.clone(),
            )
        })
    }

    /// Playback sessions used in the last `within` (the apps' Activity screen): who, what, how.
    pub fn active_sessions(&self, within: Duration) -> Vec<serde_json::Value> {
        let sessions = self.sessions.lock().unwrap();
        let mut v: Vec<(u64, serde_json::Value)> = sessions
            .iter()
            .filter(|(_, s)| s.used.elapsed() < within)
            .map(|(sid, s)| {
                let var = s.view.variants.get(s.view.selected);
                let idle = s.used.elapsed().as_secs();
                (idle, serde_json::json!({
                    "sid": sid.chars().take(8).collect::<String>(),
                    "user_id": s.view.owner, "item_id": s.view.moonrise_item,
                    "method": if var.is_some_and(|v| v.transcode) { "transcode" } else if s.view.media.method == "convert" { "convert" } else { "copy" },
                    "variant": var.map(|v| v.label.clone()), "tonemap": var.is_some_and(|v| v.tonemap),
                    "duration": s.view.media.duration, "idle_secs": idle,
                }))
            })
            .collect();
        v.sort_by_key(|(idle, _)| *idle);
        v.into_iter().map(|(_, s)| s).collect()
    }

    /// The current id of a title (for watch state).
    async fn moonrise_item(&self, id: &str) -> Option<String> {
        let l = self.library.as_ref()?;
        crate::library::api::resolve_id(l, id.split('~').next().unwrap_or(id))
            .await
            .ok()
            .flatten()
    }

    fn session(&self, sid: &str) -> Result<View, ApiError> {
        let view = {
            let mut sessions = self.sessions.lock().unwrap();
            let s = sessions.get_mut(sid).ok_or_else(session_gone)?;
            s.used = Instant::now();
            s.view.clone()
        };
        self.subs.streaming();
        Ok(view)
    }

    /// The media as served to a player with these options (its own copy when they change it).
    fn media_for(media: Arc<Media>, opts: &StreamOptions) -> Arc<Media> {
        if !opts.lossless {
            return media;
        }
        media.with_lossless().map(Arc::new).unwrap_or(media)
    }

    /// What to serve for these options: the ladder and the variant chosen, or why not.
    /// `sid`: the session it will be (a transcode slot is reserved for it).
    fn plan(&self, media: &Media, opts: &StreamOptions, sid: &str) -> Plan {
        let mut plan = Plan {
            variants: Vec::new(),
            selected: 0,
            reasons: media.reasons.clone(),
            notes: Vec::new(),
        };
        let Some(v) = media.video.as_ref().filter(|_| media.supported) else {
            return plan;
        };
        let transcode = self.config.enabled(Feature::Transcode);
        plan.variants = transcode::ladder(v, opts.hdr_display, transcode);
        if plan.variants.is_empty() {
            let why = crate::media::hwaccel::current().too_heavy(v).filter(|_| transcode);
            plan.reasons
                .push(why.or_else(|| v.copy_reason.clone()).unwrap_or_else(|| "not transcoded".into()));
            return plan;
        }
        plan.selected = transcode::select(&plan.variants, opts.quality);
        let chosen = &plan.variants[plan.selected];
        if let Some(q) = opts.quality.filter(|q| !transcode && chosen.bitrate > *q) {
            // Only the original, above the quality asked for.
            plan.reasons.push(format!("quality {q} needs transcoding (switched off)"));
            return plan;
        }
        if chosen.transcode && !self.transcoder.admit(media, chosen, sid) {
            // Every slot is in use by someone else: the copy if there is one.
            match plan.variants.iter().position(Variant::is_copy) {
                Some(c) => {
                    plan.notes.push(format!(
                        "transcode slots busy: serving {} instead of {}",
                        plan.variants[c].label, chosen.label
                    ));
                    plan.selected = c;
                }
                None => plan.reasons.push("transcode slots busy".into()),
            }
        }
        plan
    }
}

/// Extracts every title's embedded text subtitles ahead of need, one file at a time and paced
/// (`subs.rs`): the next episodes of what plays first, then the library newest first. Done
/// files are marked (`done.json`), so a restart doesn't read them again; it looks again when a
/// scan finds new or changed titles.
pub async fn subtitle_backfill(st: Arc<AppState>) {
    let (Some(library), Some(db)) = (st.library.clone(), st.db.clone()) else {
        return;
    };
    if !st.subs.enabled() || !st.config.enabled(Feature::SubsBackfill) {
        return;
    }
    library.warm_folders().await;
    // Let the first scan pass settle.
    tokio::time::sleep(Duration::from_secs(90)).await;
    loop {
        library.warm_folders().await;
        let rows = db
            .call(|c| {
                // Each title's file as playback picks it (the largest, not a broken one first).
                let mut q = c.prepare(&format!(
                    "SELECT item_id, path, size, mtime, first_seen FROM files WHERE item_id IS NOT NULL AND missing_since IS NULL
                     AND role IN {} ORDER BY role = 'broken', size DESC",
                    crate::library::TITLE_ROLES
                ))?;
                let rows = q
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, i64>(2)?,
                            r.get::<_, i64>(3)?,
                            r.get::<_, i64>(4)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        let mut files: Vec<(String, String, i64)> = rows
            .into_iter()
            .filter(|r| seen.insert(r.0.clone()))
            .map(|(item, path, size, mtime, first)| {
                (
                    item,
                    format!("{:016x}", crate::media::fnv(&format!("{path}|{size}|{mtime}"))),
                    first,
                )
            })
            .collect();
        files.sort_by_key(|f| -f.2);
        let total = files.len();
        let todo: Vec<(String, String)> = files.into_iter().filter(|f| !st.subs.settled(&f.1)).map(|f| (f.0, f.1)).collect();
        {
            let mut b = st.subs.backfill.lock().unwrap();
            *b = crate::subtitles::subs::BackfillStatus {
                files: total,
                done: total - todo.len(),
                pass_started: Some(crate::subtitles::subs::now_secs()),
                ..Default::default()
            };
        }
        if !todo.is_empty() {
            tracing::info!(files = total, to_do = todo.len(), "subtitle backfill");
        }
        let t0 = Instant::now();
        let mut queue: std::collections::VecDeque<(String, Option<String>)> = todo.into_iter().map(|(i, k)| (i, Some(k))).collect();
        let mut last_log = Instant::now();
        loop {
            // The next episodes of what plays come first.
            let next = match st.subs.next_hint() {
                Some(id) => (id, None),
                None => match queue.pop_front() {
                    Some(x) => x,
                    None => break,
                },
            };
            let (item, key) = next;
            if key.as_deref().is_some_and(|k| st.subs.settled(k)) {
                continue;
            }
            st.subs.backfill.lock().unwrap().current = Some(item.clone());
            let r = match st.load_media(&item, false).await {
                Ok(media) => {
                    if st.subs.settled(&media.file_key) {
                        Ok(false)
                    } else {
                        let r = st.subs.backfill_one(&media).await;
                        if let Err(e) = &r {
                            st.subs.mark_failed(&media.file_key, &format!("{e:#}")).await;
                        }
                        r
                    }
                }
                Err(e) => {
                    if let Some(k) = &key {
                        st.subs.mark_failed(k, &e.1).await;
                    }
                    Err(anyhow::anyhow!(e.1))
                }
            };
            {
                let mut b = st.subs.backfill.lock().unwrap();
                b.current = None;
                match &r {
                    Ok(read) => {
                        if key.is_some() {
                            b.done += 1;
                        }
                        if *read {
                            b.extracted_this_run += 1;
                        }
                    }
                    Err(_) => b.failed += 1,
                }
                b.secs_this_run = t0.elapsed().as_secs_f64();
            }
            if last_log.elapsed() > Duration::from_secs(600) {
                last_log = Instant::now();
                let b = st.subs.backfill.lock().unwrap().clone();
                tracing::info!(
                    files = b.files,
                    done = b.done,
                    failed = b.failed,
                    extracted = b.extracted_this_run,
                    secs = b.secs_this_run as u64,
                    "subtitle backfill progress"
                );
            }
            // A breath between files.
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let b = st.subs.backfill.lock().unwrap().clone();
        if b.extracted_this_run + b.failed > 0 {
            tracing::info!(
                files = b.files,
                done = b.done,
                failed = b.failed,
                extracted = b.extracted_this_run,
                secs = b.secs_this_run as u64,
                "subtitle backfill pass finished"
            );
        }
        tokio::select! {
            () = library.files_changed.notified() => {}
            () = st.subs.backfill_wake.notified() => {}
            () = tokio::time::sleep(Duration::from_secs(3600)) => {}
        }
    }
}

/// A stream URL of a session the server doesn't have (any more): sessions live in memory, so
/// every restart ends them. `410` tells the app to open a new one (and where it was).
fn session_gone() -> ApiError {
    ApiError(StatusCode::GONE, "unknown or expired session: open a new one".into())
}

/// The CLI's admin token, made once (`/data/secrets/admin.token`, mode 600).
fn admin_token(data_dir: &std::path::Path) -> Option<String> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = data_dir.join("secrets").join("admin.token");
    if let Ok(t) = std::fs::read_to_string(&path) {
        let t = t.trim().to_string();
        if !t.is_empty() {
            return Some(t);
        }
    }
    let t: String = rand::thread_rng()
        .sample_iter(rand::distributions::Alphanumeric)
        .take(40)
        .map(char::from)
        .collect();
    std::fs::create_dir_all(path.parent()?).ok()?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .ok()?;
    std::io::Write::write_all(&mut f, t.as_bytes()).ok()?;
    Some(t)
}

fn new_sid() -> String {
    rand::thread_rng()
        .sample_iter(rand::distributions::Alphanumeric)
        .take(24)
        .map(char::from)
        .collect()
}

struct Plan {
    variants: Vec<Variant>,
    selected: usize,
    /// Why it can't be served (empty: it can).
    reasons: Vec<String>,
    /// Served, but not quite as asked.
    notes: Vec<String>,
}

impl Plan {
    fn ok(&self) -> bool {
        self.reasons.is_empty() && !self.variants.is_empty()
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/info", get(info))
        .route("/v1/items/{id}/playback", get(playback))
        .route("/v1/items/{id}/master.m3u8", get(item_master))
        .route("/v1/jobs", get(jobs))
        .route("/v1/s/{sid}", delete(delete_session))
        .route("/v1/s/{sid}/master.m3u8", get(session_master))
        // The copied video (masters before variants pointed here).
        .route("/v1/s/{sid}/video/index.m3u8", get(copy_playlist))
        .route("/v1/s/{sid}/video/{file}", get(copy_file))
        .route("/v1/s/{sid}/video/{variant}/index.m3u8", get(video_playlist))
        .route("/v1/s/{sid}/video/{variant}/{file}", get(video_file))
        .route("/v1/s/{sid}/audio/{index}/index.m3u8", get(audio_playlist))
        .route("/v1/s/{sid}/audio/{index}/{file}", get(audio_file))
        .route("/v1/s/{sid}/subs/{index}/index.m3u8", get(subs_playlist))
        .route("/v1/s/{sid}/subs/{index}/{file}", get(subs_segment))
        .route("/v1/s/{sid}/trickplay/iframes.m3u8", get(session_iframes_playlist))
        .route("/v1/s/{sid}/trickplay/iframes.mp4", get(session_iframes_mp4))
        .route("/v1/items/{id}/trickplay", get(thumbnails))
        .route("/v1/trickplay/{key}/{file}", get(thumbnail_sheet))
        .route("/v1/items/{id}/subtitles", get(subtitles_list))
        .route("/v1/items/{id}/subtitles/search", get(subtitles_search))
        .route("/v1/items/{id}/subtitles/fetch", post(subtitles_fetch))
        .route("/v1/items/{id}/subtitles/upload", post(subtitles_upload))
        .route("/v1/items/{id}/subtitles/{file}", get(subtitle_file))
        .route("/v1/items/{id}/subtitles/{file}/cues", get(subtitle_window))
        .route(
            "/v1/items/{id}/subtitles/{file}/timing",
            get(subtitle_timing).put(subtitle_timing_change),
        )
        .route("/v1/items/{id}/download", get(download_original))
        .route("/v1/items/{id}/download-jobs", post(download_job_create))
        .route("/v1/download-jobs", get(download_jobs_list))
        .route("/v1/download-jobs/{job}", get(download_job_get).delete(download_job_delete))
        .route("/v1/download-jobs/{job}/file", get(download_job_file))
        .route("/v1/items/{id}/offline", get(crate::offline::describe))
        .route("/v1/items/{id}/offline/{variant}/video/{file}", get(crate::offline::video_file))
        .route(
            "/v1/items/{id}/offline/{variant}/audio/{index}/{file}",
            get(crate::offline::audio_file),
        )
        .route(
            "/v1/items/{id}/offline/{variant}/subs/{index}/cues.vtt",
            get(crate::offline::subs_file),
        )
        .merge(crate::library::api::routes())
        .merge(crate::auth::api::routes())
        .merge(crate::userdata::api::routes())
        .merge(crate::admin::routes())
        .merge(crate::admin::services::routes())
        .merge(crate::admin::manage::routes())
        .merge(crate::admin::claim::routes())
        .layer(axum::middleware::from_fn(auth_span))
        .with_state(state)
}

/// Every request runs in a span saying how it authenticates ("moonrise": a Bearer token, "admin":
/// the CLI's token, else "none").
async fn auth_span(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    use tracing::Instrument;
    let h = req.headers();
    let auth = if crate::auth::bearer(h).is_some() {
        "moonrise"
    } else if h.contains_key("x-moonrise-admin") {
        "admin"
    } else {
        "none"
    };
    let span = tracing::info_span!("req", auth);
    next.run(req).instrument(span).await
}

#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);

impl ApiError {
    pub fn internal(e: anyhow::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if self.0.is_server_error() {
            tracing::warn!("{}: {}", self.0, self.1);
        }
        (self.0, self.1).into_response()
    }
}

type Q = Query<HashMap<String, String>>;

#[derive(Serialize)]
struct Info {
    server_id: String,
    name: String,
    version: &'static str,
    capabilities: Vec<&'static str>,
    /// `unclaimed` (no accounts yet: set up with the setup code) or `claimed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    setup: Option<&'static str>,
}

async fn info(State(st): State<Arc<AppState>>) -> Json<Info> {
    let mut capabilities = vec!["hls-copy", "audio-renditions", "webvtt-subtitles", "session-delete", "dolby-vision"];
    if st.db.is_some() && st.config.enabled(Feature::ProbeCache) {
        capabilities.push("probe-cache");
    }
    for (f, name) in [
        (Feature::Convert, "audio-convert"),
        (Feature::Convert, "lossless-audio"),
        (Feature::Mp4Index, "mp4-index"),
        (Feature::ScanIndex, "scan-index"),
    ] {
        if st.config.enabled(f) {
            capabilities.push(name);
        }
    }
    if st.config.enabled(Feature::SubsExtract) {
        capabilities.push("subtitle-extract");
    }
    capabilities.push("subtitle-files");
    if st.config.enabled(Feature::SubsExtract) && st.config.enabled(Feature::SubsWindows) {
        capabilities.push("subtitle-windows");
    }
    if st.config.enabled(Feature::SubsWindows) {
        capabilities.push("subtitle-pictures");
    }
    capabilities.push("subtitle-timing");
    if st.config.enabled(Feature::SubsSync) {
        capabilities.push("subtitle-sync");
    }
    if st.config.enabled(Feature::Downloads) {
        capabilities.extend(["download-original", "download-jobs", "offline-sets"]);
    }
    if st.config.enabled(Feature::OpenSubtitles) && st.opensubtitles.configured() {
        capabilities.push("subtitle-search");
    }
    if st.library.is_some() {
        capabilities.extend([
            "library",
            "browse",
            "search",
            "images",
            "segments",
            "segments-merged",
            "library-filters",
            "mixed-libraries",
            "versions",
            "fix-match-undo",
        ]);
    }
    if st.auth.is_some() {
        capabilities.extend([
            "moonrise-auth",
            "device-code",
            "user-data",
            "playback-reporting",
            "invites",
            "track-memory",
        ]);
    }
    if st.library.is_some() && st.auth.is_some() {
        capabilities.push("search-start");
    }
    if st.library.is_some() {
        capabilities.push("match-posters");
    }
    if st.config.enabled(Feature::Transcode) && crate::media::hwaccel::current().available {
        capabilities.extend(["video-transcode", "quality-ladder", "hdr-tonemap"]);
    }
    let setup = crate::admin::claim::info_state(crate::admin::claim::refresh(&st).await);
    if setup.is_some() {
        capabilities.push("setup-claim");
    }
    if st.library.is_some() {
        capabilities.push("scan-progress");
    }
    if st.library.is_some() && st.db.is_some() && st.config.enabled(Feature::Trickplay) {
        capabilities.push("trickplay");
    }
    Json(Info {
        server_id: st.server_id.clone(),
        name: st.name(),
        version: env!("CARGO_PKG_VERSION"),
        capabilities,
        setup,
    })
}

/// Authenticates a request (a Moonrise Bearer token, or the CLI's admin token).
async fn auth(st: &AppState, headers: &HeaderMap, q: &HashMap<String, String>) -> Result<Caller, ApiError> {
    crate::auth::caller(st, headers, q).await
}

#[derive(Serialize)]
struct PlaybackInfo {
    /// As served: `supported`, `reasons`, `method` and `video` are for these options.
    #[serde(flatten)]
    media: Media,
    /// Path of the master playlist (only when supported).
    master: Option<String>,
    /// The title's ladder for these options, best first.
    variants: Vec<Variant>,
    /// The one played (an id from `variants`).
    variant: Option<String>,
    /// Served, but not quite as asked (e.g. the original instead of a busy transcode).
    notes: Vec<String>,
    /// The playback session (its id is in `master`): reports go to `/v1/sessions/{session}/…`.
    session: Option<String>,
    /// The audio and subtitle tracks the user picked last in this file, or in the title or the
    /// show (`userdata::tracks::lookup`). Absent when there are none.
    #[serde(skip_serializing_if = "Option::is_none")]
    tracks: Option<serde_json::Value>,
}

/// The playback answer for a plan: the media as served.
fn answer(media: &Media, plan: &Plan, master: Option<String>) -> PlaybackInfo {
    let mut m = media.clone();
    m.supported = plan.ok();
    m.reasons.clone_from(&plan.reasons);
    let chosen = plan.variants.get(plan.selected).filter(|_| plan.ok());
    if let (Some(c), Some(v)) = (chosen, m.video.as_mut()) {
        if c.transcode {
            m.method = "transcode";
            v.codec.clone_from(&c.codec);
            v.codecs_attr = Some(c.codecs_attr.clone());
            v.width = c.width;
            v.height = c.height;
            v.range = c.range;
            v.bitrate = c.bitrate;
        } else if m.method == "transcode" {
            m.method = if m.audio.iter().any(|a| a.converted) { "convert" } else { "copy" };
        }
    }
    PlaybackInfo {
        media: m,
        master,
        variants: plan.variants.clone(),
        variant: chosen.map(|c| c.id.clone()),
        notes: plan.notes.clone(),
        session: None,
        tracks: None,
    }
}

async fn playback(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<PlaybackInfo>, ApiError> {
    let who = auth(&st, &headers, &q).await?;
    let opts = StreamOptions::from(&q);
    let media = AppState::media_for(st.media(&st.with_chosen_version(&who, media_key(&id, &q)).await).await?, &opts);
    let sid = new_sid();
    let plan = st.plan(&media, &opts, &sid);
    let mut master = None;
    let mut session = None;
    let moonrise_item = if who.user().is_some() { st.moonrise_item(&id).await } else { None };
    if plan.ok() {
        let view = st.new_session(sid, &who, moonrise_item.clone(), media.clone(), opts, &plan);
        session = Some(view.sid.clone());
        prewarm(&st, &view);
        master = Some(format!("/v1/s/{}/master.m3u8", view.sid));
    }
    let mut info = answer(&media, &plan, master);
    info.session = session;
    info.tracks = remembered_tracks(&st, &who, moonrise_item, media.file_id.clone()).await;
    tracing::info!(item = %id, auth = who.kind(), supported = info.media.supported, variant = ?info.variant, quality = ?opts.quality, hdr = opts.hdr_display, lossless = opts.lossless,
        reasons = ?info.media.reasons, notes = ?info.notes, "playback");
    Ok(Json(info))
}

/// The tracks the caller picked last in this file (or title, or show), for the playback answer.
async fn remembered_tracks(st: &AppState, who: &Caller, item: Option<String>, file: Option<String>) -> Option<serde_json::Value> {
    let (db, user, item) = (st.db.clone()?, who.user_id()?, item?);
    db.call(move |c| crate::userdata::tracks::lookup(c, &user, file.as_deref(), &item))
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("track memory: {e:#}");
            None
        })
}

async fn item_master(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Response, ApiError> {
    let who = auth(&st, &headers, &q).await?;
    let opts = StreamOptions::from(&q);
    let media = AppState::media_for(st.media(&st.with_chosen_version(&who, media_key(&id, &q)).await).await?, &opts);
    let sid = new_sid();
    let plan = st.plan(&media, &opts, &sid);
    if !plan.ok() {
        return Err(ApiError(StatusCode::UNPROCESSABLE_ENTITY, plan.reasons.join("; ")));
    }
    let moonrise_item = if who.user().is_some() { st.moonrise_item(&id).await } else { None };
    let view = st.new_session(sid, &who, moonrise_item, media, opts, &plan);
    prewarm(&st, &view);
    let iframes = iframe_stream(&st, &view).await;
    Ok(playlist(master_with(&view, iframes)))
}

async fn session_master(State(st): State<Arc<AppState>>, Path(sid): Path<String>) -> Result<Response, ApiError> {
    let view = st.session(&sid)?;
    let iframes = iframe_stream(&st, &view).await;
    Ok(playlist(master_with(&view, iframes)))
}

// ---------------------------------------------------------------- scrubbing thumbnails

/// The thumbnails of the file a session plays, if the player asked for them (`trick=1`) and
/// they are made.
async fn session_thumbnails(st: &AppState, view: &View) -> Option<crate::media::trickplay::Index> {
    if !view.opts.trick || !st.config.enabled(Feature::Trickplay) {
        return None;
    }
    st.trickplay.index(&view.media.file_key).await
}

/// The master playlist's I-frame stream line for a session (`trick=1`).
async fn iframe_stream(st: &AppState, view: &View) -> Option<String> {
    let index = session_thumbnails(st, view).await?;
    crate::media::trickplay::iframe_stream_inf(&index, &format!("/v1/s/{}/trickplay/iframes.m3u8", view.sid))
}

fn no_thumbnails() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "no thumbnails".into())
}

/// `GET /v1/s/{sid}/trickplay/iframes.m3u8`: the session's thumbnails as an I-frame playlist.
async fn session_iframes_playlist(State(st): State<Arc<AppState>>, Path(sid): Path<String>) -> Result<Response, ApiError> {
    let view = st.session(&sid)?;
    let index = session_thumbnails(&st, &view).await.ok_or_else(no_thumbnails)?;
    Ok(playlist(
        crate::media::trickplay::iframe_playlist(&index, "iframes.mp4").ok_or_else(no_thumbnails)?,
    ))
}

/// `GET /v1/s/{sid}/trickplay/iframes.mp4` (`Range`): the session's thumbnail track.
async fn session_iframes_mp4(State(st): State<Arc<AppState>>, Path(sid): Path<String>, headers: HeaderMap) -> Result<Response, ApiError> {
    let view = st.session(&sid)?;
    session_thumbnails(&st, &view).await.ok_or_else(no_thumbnails)?;
    let path = st.trickplay.file(&view.media.file_key, "iframes.mp4").ok_or_else(no_thumbnails)?;
    Ok(downloads::serve_file(&path.to_string_lossy(), &headers, "video/mp4", "iframes.mp4", None).await)
}

#[derive(Serialize)]
struct Thumbnails {
    /// "ready", "pending" (not made yet: the title moves to the front of the queue) or "off".
    state: &'static str,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    layout: Option<ThumbnailLayout>,
}

#[derive(Serialize)]
struct ThumbnailLayout {
    /// Seconds between pictures: picture `i` is shown from `i * interval` to the next one.
    interval: f64,
    width: u32,
    height: u32,
    columns: u32,
    rows: u32,
    count: u32,
    /// The sheets' URLs (token, or `sid=`): picture `i` is on sheet `i / (columns × rows)`, at
    /// column `i % columns` and row `(i / columns) % rows` of it.
    sheets: Vec<String>,
}

/// `GET /v1/items/{id}/trickplay?version=&sid=` (token, or `sid=` of a session playing it): the
/// title's scrubbing thumbnails (with `sid`, those of the file that session plays).
async fn thumbnails(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<Thumbnails>, ApiError> {
    let playing = q
        .get("sid")
        .and_then(|sid| st.session(sid).ok())
        .filter(|v| v.media.item_id.split('~').next() == Some(id.split('~').next().unwrap_or(&id)));
    let media = match playing {
        Some(v) => v.media.clone(),
        None => {
            let who = auth(&st, &headers, &q).await?;
            st.media(&st.with_chosen_version(&who, media_key(&id, &q)).await).await?
        }
    };
    if !st.config.enabled(Feature::Trickplay) {
        return Ok(Json(Thumbnails {
            state: "off",
            layout: None,
        }));
    }
    let Some(index) = st.trickplay.index(&media.file_key).await else {
        if let Some(item) = st.moonrise_item(&id).await {
            st.trickplay.hint(vec![item]);
        }
        return Ok(Json(Thumbnails {
            state: "pending",
            layout: None,
        }));
    };
    let sheets = (0..index.sheets)
        .map(|n| format!("/v1/trickplay/{}/{n}.jpg", media.file_key))
        .collect();
    Ok(Json(Thumbnails {
        state: "ready",
        layout: Some(ThumbnailLayout {
            interval: index.interval,
            width: index.width,
            height: index.height,
            columns: index.columns,
            rows: index.rows,
            count: index.count,
            sheets,
        }),
    }))
}

/// `GET /v1/trickplay/{key}/{n}.jpg` (token, or `sid=` of a session playing that file): a sheet.
async fn thumbnail_sheet(
    State(st): State<Arc<AppState>>,
    Path((key, file)): Path<(String, String)>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Response, ApiError> {
    let playing = q
        .get("sid")
        .and_then(|sid| st.session(sid).ok())
        .is_some_and(|v| v.media.file_key == key);
    if !playing {
        auth(&st, &headers, &q).await?;
    }
    #[allow(
        clippy::case_sensitive_file_extension_comparisons,
        reason = "the sheets are ours, named .jpg; nothing else is served"
    )]
    let path = st
        .trickplay
        .file(&key, &file)
        .filter(|_| file.ends_with(".jpg"))
        .ok_or_else(no_thumbnails)?;
    let data = tokio::fs::read(&path).await.map_err(|_| no_thumbnails())?;
    // A file key names one state of one file: its sheets never change.
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "private, max-age=2592000, immutable"),
        ],
        data,
    )
        .into_response())
}

async fn delete_session(State(st): State<Arc<AppState>>, Path(sid): Path<String>) -> StatusCode {
    let existed = st.sessions.lock().unwrap().remove(&sid).is_some();
    st.subs.session_closed(&sid);
    let video = st.transcoder.stop_owner(&sid);
    let audio = st.converter.stop_owner(&sid);
    tracing::info!(session = %sid.chars().take(6).collect::<String>(), existed, video_runs = video, audio_runs = audio, "session closed");
    if existed {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn jobs(State(st): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let runs = st.transcoder.running();
    Json(
        serde_json::json!({ "transcode_jobs": st.config.transcode_jobs, "running": runs.len(), "runs": runs, "subtitles": st.subs.status() }),
    )
}

/// Start on the first segments at the start (video and the default audio track) while the
/// player is still fetching playlists, so they're ready when it asks.
fn prewarm(st: &Arc<AppState>, view: &View) {
    let media = &view.media;
    let n = view.start_segment();
    let variant = &view.variants[view.selected];
    let mut tracks = Vec::new();
    if variant.transcode {
        // A transcode run starts there and keeps going.
        st.transcoder.prefetch(media, variant, n, &view.sid);
    } else {
        tracks.push(Track::Video);
    }
    match media.audio.iter().find(|a| a.default) {
        // A conversion run starts there and keeps going.
        Some(a) if a.converted => st.converter.prefetch(media, a, n, &view.sid),
        Some(a) => tracks.push(Track::Audio(a.stream)),
        None => {}
    }
    for track in tracks {
        for k in n..(n + 2).min(media.boundaries.len()) {
            let (st, media) = (st.clone(), media.clone());
            tokio::spawn(async move {
                let _ = st.segmenter.ensure(&media, track, k).await;
            });
        }
    }
}

/// The media key of a request: the item, or one of its versions (`<item>~<file>`: the path's
/// own, or `version=<file id>`).
pub(crate) fn media_key(id: &str, q: &HashMap<String, String>) -> String {
    match q
        .get("version")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty() && v.len() <= 64 && v.chars().all(|c| c.is_ascii_alphanumeric()))
    {
        Some(v) if !id.contains('~') => format!("{id}~{v}"),
        _ => id.to_string(),
    }
}

fn playlist(body: String) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/vnd.apple.mpegurl"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}

pub(crate) fn mp4(body: Bytes) -> Response {
    (
        [(header::CONTENT_TYPE, "video/mp4"), (header::CACHE_CONTROL, "max-age=86400")],
        body,
    )
        .into_response()
}

fn quote(s: &str) -> String {
    s.replace('"', "'")
}

#[cfg(test)]
fn master(view: &View) -> String {
    master_with(view, None)
}

/// The master playlist, with the thumbnails' I-frame stream line (`trickplay.rs`) if there is one.
fn master_with(view: &View, iframes: Option<String>) -> String {
    let (m, opts) = (&view.media, &view.opts);
    let base = format!("/v1/s/{}", view.sid);
    let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-INDEPENDENT-SEGMENTS\n");
    if let Some(s) = opts.start {
        let _ = writeln!(out, "#EXT-X-START:TIME-OFFSET={s:.3},PRECISE=YES");
    }
    for a in &m.audio {
        let _ = writeln!(
            out,
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"{}\",{}DEFAULT={},AUTOSELECT=YES,CHANNELS=\"{}\",URI=\"{base}/audio/{}/index.m3u8\"",
            quote(&a.name),
            a.language.as_ref().map(|l| format!("LANGUAGE=\"{l}\",")).unwrap_or_default(),
            if a.default { "YES" } else { "NO" },
            a.channels,
            a.index
        );
    }
    let text: Vec<_> = m.subtitles.iter().filter(|s| s.text).collect();
    for s in &text {
        let _ = writeln!(
            out,
            "#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"{}\",{}DEFAULT=NO,AUTOSELECT={},FORCED={},URI=\"{base}/subs/{}/index.m3u8\"",
            quote(&s.name),
            s.language.as_ref().map(|l| format!("LANGUAGE=\"{l}\",")).unwrap_or_default(),
            if s.forced { "YES" } else { "NO" },
            if s.forced { "YES" } else { "NO" },
            s.index
        );
    }
    let v = m.video.as_ref().expect("supported media has video");
    let mut audio_codecs: Vec<&str> = Vec::new();
    for a in &m.audio {
        if !audio_codecs.contains(&a.codecs_attr.as_str()) {
            audio_codecs.push(&a.codecs_attr);
        }
    }
    // The heaviest audio rendition (copied tracks: up to E-AC-3's 640 kb/s). A title with no
    // audio track has no audio group: its variants carry video alone.
    let silent = m.audio.is_empty();
    let audio_bw: i64 = if silent {
        0
    } else {
        m.audio
            .iter()
            .map(|a| a.conversion.as_ref().map_or(640_000, |c| c.bitrate as i64))
            .max()
            .unwrap_or(0)
            .max(640_000)
    };
    // The variant played first; with `abr`, the lower ones after it.
    let chosen = &view.variants[view.selected];
    let listed = std::iter::once(chosen).chain(
        view.variants
            .iter()
            .filter(|x| opts.abr && x.id != chosen.id && x.bitrate < chosen.bitrate),
    );
    for var in listed {
        // Dolby Vision, for the copy (a transcode carries no Dolby Vision metadata).
        let dovi = view.dovi().map(|d| d.1).filter(|_| var.is_copy());
        let (video_codec, supplemental) = match &dovi {
            Some(DoviSignal::Codec(c)) => (c.as_str(), String::new()),
            Some(DoviSignal::Supplemental(c)) => (var.codecs_attr.as_str(), format!("SUPPLEMENTAL-CODECS=\"{c}\",")),
            None => (var.codecs_attr.as_str(), String::new()),
        };
        let codecs = std::iter::once(video_codec)
            .chain(audio_codecs.iter().copied())
            .collect::<Vec<_>>()
            .join(",");
        let attrs = format!(
            "BANDWIDTH={},AVERAGE-BANDWIDTH={},CODECS=\"{codecs}\",{supplemental}RESOLUTION={}x{},{}{}",
            var.peak + audio_bw,
            var.bitrate + audio_bw,
            var.width,
            var.height,
            v.frame_rate.map(|f| format!("FRAME-RATE={f:.3},")).unwrap_or_default(),
            if opts.hdr_display || var.range == "SDR" {
                format!("VIDEO-RANGE={},", var.range)
            } else {
                String::new()
            },
        );
        let _ = write!(
            out,
            "#EXT-X-STREAM-INF:{}\n{base}/video/{}/index.m3u8\n",
            with_groups(attrs, !silent, !text.is_empty()),
            var.id,
        );
    }
    if let Some(line) = iframes {
        out += &line;
    }
    out
}

/// A variant's attribute list (`attrs`, ending in a comma) with its rendition groups: the audio
/// group, then the subtitles'. A title with no audio track has no audio group.
pub(crate) fn with_groups(mut attrs: String, audio: bool, subs: bool) -> String {
    let groups = match (audio, subs) {
        (true, true) => "AUDIO=\"audio\",SUBTITLES=\"subs\"",
        (true, false) => "AUDIO=\"audio\"",
        (false, true) => "SUBTITLES=\"subs\"",
        (false, false) => {
            if attrs.ends_with(',') {
                attrs.pop();
            }
            return attrs;
        }
    };
    attrs + groups
}

pub(crate) fn media_playlist(m: &Media, init: bool) -> String {
    let mut target = 1.0f64;
    for n in 0..m.boundaries.len() {
        let (s, e) = m.segment_span(n).unwrap();
        target = target.max(e - s);
    }
    let mut out = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-INDEPENDENT-SEGMENTS\n",
        target.ceil() as i64
    );
    if init {
        out += "#EXT-X-MAP:URI=\"init.mp4\"\n";
    }
    for n in 0..m.boundaries.len() {
        let (s, e) = m.segment_span(n).unwrap();
        let _ = write!(out, "#EXTINF:{:.3},\n{n}.m4s\n", e - s);
    }
    out += "#EXT-X-ENDLIST\n";
    out
}

async fn copy_playlist(State(st): State<Arc<AppState>>, Path(sid): Path<String>) -> Result<Response, ApiError> {
    let view = st.session(&sid)?;
    view.variant("copy")?;
    Ok(playlist(media_playlist(&view.media, true)))
}

async fn copy_file(State(st): State<Arc<AppState>>, Path((sid, file)): Path<(String, String)>) -> Result<Response, ApiError> {
    let view = st.session(&sid)?;
    view.variant("copy")?;
    serve_copy_video(&st, view, &file).await
}

/// The copy's video: for a player told about Dolby Vision, its init section with the
/// configuration box (and `dvh1` for profile 5); segments as they are (they carry the RPUs).
async fn serve_copy_video(st: &Arc<AppState>, view: View, file: &str) -> Result<Response, ApiError> {
    if let (Some((d, signal)), "init.mp4") = (view.dovi(), file) {
        let init = st.segmenter.init(&view.media, Track::Video).await.map_err(ApiError::internal)?;
        let rename = matches!(signal, DoviSignal::Codec(_)).then_some(b"dvh1");
        let body = crate::media::mp4::with_dovi(&init, &d.config_box(), rename).map_err(ApiError::internal)?;
        return Ok(mp4(Bytes::from(body)));
    }
    serve_track_file(st, view.media, Track::Video, file).await
}

async fn video_playlist(State(st): State<Arc<AppState>>, Path((sid, variant)): Path<(String, String)>) -> Result<Response, ApiError> {
    let view = st.session(&sid)?;
    view.variant(&variant)?;
    Ok(playlist(media_playlist(&view.media, true)))
}

async fn video_file(
    State(st): State<Arc<AppState>>,
    Path((sid, variant, file)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let view = st.session(&sid)?;
    let var = view.variant(&variant)?.clone();
    if var.is_copy() {
        return serve_copy_video(&st, view, &file).await;
    }
    if file == "init.mp4" {
        let body = st
            .transcoder
            .init(&view.media, &var, view.start_segment(), &sid)
            .await
            .map_err(ApiError::internal)?;
        return Ok(mp4(body));
    }
    let n = segment_number(&view.media, &file)?;
    Ok(mp4(st
        .transcoder
        .segment(&view.media, &var, n, &sid)
        .await
        .map_err(ApiError::internal)?))
}

async fn audio_playlist(State(st): State<Arc<AppState>>, Path((sid, index)): Path<(String, i64)>) -> Result<Response, ApiError> {
    let media = st.session(&sid)?.media;
    if media.audio_track(index).is_none() {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such audio track".into()));
    }
    Ok(playlist(media_playlist(&media, true)))
}

async fn audio_file(State(st): State<Arc<AppState>>, Path((sid, index, file)): Path<(String, i64, String)>) -> Result<Response, ApiError> {
    let media = st.session(&sid)?.media;
    let a = media
        .audio_track(index)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such audio track".into()))?
        .clone();
    if let Some(conv) = &a.conversion {
        if file == "init.mp4" {
            return Ok(mp4(st.converter.init(conv).await.map_err(ApiError::internal)?));
        }
        let n = segment_number(&media, &file)?;
        let body = st.converter.segment(&media, &a, n, &sid).await.map_err(ApiError::internal)?;
        for ahead in n + 1..=n + LOOKAHEAD {
            if media.segment(ahead).is_some() {
                st.converter.prefetch(&media, &a, ahead, &sid);
            }
        }
        return Ok(mp4(body));
    }
    serve_track_file(&st, media, Track::Audio(a.stream), &file).await
}

pub(crate) fn segment_number(media: &Media, file: &str) -> Result<usize, ApiError> {
    let n: usize = file
        .strip_suffix(".m4s")
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "bad segment name".into()))?;
    if media.segment(n).is_none() {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such segment".into()));
    }
    Ok(n)
}

pub(crate) async fn serve_track_file(st: &Arc<AppState>, media: Arc<Media>, track: Track, file: &str) -> Result<Response, ApiError> {
    if file == "init.mp4" {
        return Ok(mp4(st.segmenter.init(&media, track).await.map_err(ApiError::internal)?));
    }
    let n = segment_number(&media, file)?;
    let body = st.segmenter.segment(&media, track, n).await.map_err(ApiError::internal)?;
    // Cut the next ones while this one plays.
    for ahead in n + 1..=n + LOOKAHEAD {
        if media.segment(ahead).is_some() {
            let (st, media) = (st.clone(), media.clone());
            tokio::spawn(async move {
                let _ = st.segmenter.ensure(&media, track, ahead).await;
            });
        }
    }
    Ok(mp4(body))
}

/// A subtitle rendition: WebVTT segments of `SUBS_SEGMENT` seconds. The player fetches them as
/// it plays, so cues not extracted yet when it starts arrive with the next segment.
async fn subs_playlist(State(st): State<Arc<AppState>>, Path((sid, index)): Path<(String, i64)>) -> Result<Response, ApiError> {
    let View { media, .. } = st.session(&sid)?;
    if !media.subtitles.iter().any(|s| s.index == index && s.text) {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such subtitle track".into()));
    }
    // The segments are read window by window (`subs_segment`); the whole track is extracted
    // in the background as the session's title.
    st.subs.prefetch(&media);
    Ok(playlist(subs_media_playlist(media.duration)))
}

fn subs_media_playlist(duration: f64) -> String {
    let n = (duration / SUBS_SEGMENT).ceil().max(1.0) as usize;
    let mut out = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n",
        SUBS_SEGMENT as i64
    );
    for k in 0..n {
        let d = (duration - k as f64 * SUBS_SEGMENT).clamp(0.001, SUBS_SEGMENT);
        let _ = write!(out, "#EXTINF:{d:.3},\n{k}.vtt\n");
    }
    out += "#EXT-X-ENDLIST\n";
    out
}

/// The whole track as one WebVTT file (the rendition before it was segmented).
async fn subs_file(State(st): State<Arc<AppState>>, Path((sid, index)): Path<(String, i64)>) -> Result<Response, ApiError> {
    let View { media, .. } = st.session(&sid)?;
    if !media.subtitles.iter().any(|s| s.index == index && s.text) {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such subtitle track".into()));
    }
    // AVPlayer gives up on the whole stream when a rendition it selected (PiP) doesn't answer
    // in time: while the track is still being extracted, answer with no cues rather than make
    // it wait. The app's own overlay waits for the SRT.
    let made = st.subs.vtt(&media, index);
    let body = if st.subs.ready(&media, index) {
        made.await.map_err(ApiError::internal)?
    } else {
        match tokio::time::timeout(VTT_WAIT, made).await {
            Ok(r) => r.map_err(ApiError::internal)?,
            Err(_) => {
                tracing::info!(item = %media.item_id, index, "subtitle rendition not ready: answered without cues");
                subs::srt_to_vtt("", media.origin)
            }
        }
    };
    Ok((
        [
            (header::CONTENT_TYPE, "text/vtt; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response())
}

/// One segment of a subtitle rendition: the cues that show during it.
async fn subs_segment(
    State(st): State<Arc<AppState>>,
    Path((sid, index, file)): Path<(String, i64, String)>,
) -> Result<Response, ApiError> {
    let View { media, .. } = st.session(&sid)?;
    if file == "cues.vtt" {
        return subs_file(State(st), Path((sid, index))).await;
    }
    let k: usize = file
        .strip_suffix(".vtt")
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "bad segment name".into()))?;
    if !media.subtitles.iter().any(|s| s.index == index && s.text) {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such subtitle track".into()));
    }
    // The segments are the subtitle windows: read for this one alone when the track isn't
    // extracted yet (a fraction of a second). AVPlayer fails the whole stream when a selected
    // rendition stalls: past VTT_SEGMENT_WAIT the segment answers without cues.
    let windows = st.config.enabled(Feature::SubsWindows);
    let made = async {
        if windows {
            return st.subs.rendition_window(&media, index, k).await;
        }
        let (from, to) = (k as f64 * SUBS_SEGMENT, (k + 1) as f64 * SUBS_SEGMENT);
        let srt = st.subs.srt(&media, index).await?;
        let nudge = st.subs.nudge(&media, index);
        let cues = subs::parse_srt(&srt)
            .into_iter()
            .map(|c| crate::subtitles::subx::Cue {
                start: c.start + nudge,
                end: c.end + nudge,
                text: c.text,
            })
            .collect();
        Ok(Arc::new(crate::subtitles::subx::overlapping(cues, from, to)))
    };
    let body = match tokio::time::timeout(VTT_SEGMENT_WAIT, made).await {
        Ok(Ok(cues)) => subs::cues_to_vtt(&cues, media.origin),
        Ok(Err(e)) => {
            tracing::warn!(item = %media.item_id, index, segment = k, "subtitle segment: {e:#}");
            subs::srt_to_vtt("", media.origin)
        }
        Err(_) => {
            tracing::info!(item = %media.item_id, index, segment = k, "subtitle segment not ready: answered without cues");
            subs::srt_to_vtt("", media.origin)
        }
    };
    Ok((
        [
            (header::CONTENT_TYPE, "text/vtt; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response())
}

// ---------------------------------------------------------------- subtitles

/// The request's credentials, or (`sid=`) a session playing this item.
async fn auth_or_session(st: &AppState, headers: &HeaderMap, q: &HashMap<String, String>, item_id: &str) -> Result<(), ApiError> {
    if let Some(view) = q.get("sid").and_then(|sid| st.session(sid).ok()) {
        if view.media.item_id == item_id {
            return Ok(());
        }
    }
    auth(st, headers, q).await.map(|_| ())
}

async fn subtitles_list(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<serde_json::Value>, ApiError> {
    auth(&st, &headers, &q).await?;
    let media = st.media(&media_key(&id, &q)).await?;
    // How each text track's timing stands, read now (the media is kept for a while).
    let tracks: Vec<SubtitleTrack> = media
        .subtitles
        .iter()
        .map(|t| {
            let mut t = t.clone();
            if t.text {
                t.timing = Some(st.subs.timing(&media, &t));
            }
            t
        })
        .collect();
    Ok(Json(serde_json::json!({ "subtitles": tracks })))
}

/// `GET /v1/items/{id}/subtitles/{index}/timing`: see the module docs.
async fn subtitle_timing(
    State(st): State<Arc<AppState>>,
    Path((id, file)): Path<(String, String)>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<crate::subtitles::subsync::Timing>, ApiError> {
    let index: i64 = file.parse().map_err(|_| ApiError(StatusCode::NOT_FOUND, "bad index".into()))?;
    auth(&st, &headers, &q).await?;
    let media = st.media(&media_key(&id, &q)).await?;
    if !media.subtitles.iter().any(|s| s.index == index && s.text) {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such text subtitle track".into()));
    }
    st.subs.timing_now(&media, index).await.map(Json).map_err(ApiError::internal)
}

/// `PUT /v1/items/{id}/subtitles/{index}/timing {nudge?, original?}`: see the module docs.
async fn subtitle_timing_change(
    State(st): State<Arc<AppState>>,
    Path((id, file)): Path<(String, String)>,
    headers: HeaderMap,
    Query(q): Q,
    Json(change): Json<crate::subtitles::subsync::TimingChange>,
) -> Result<Json<crate::subtitles::subsync::Timing>, ApiError> {
    let index: i64 = file.parse().map_err(|_| ApiError(StatusCode::NOT_FOUND, "bad index".into()))?;
    auth(&st, &headers, &q).await?;
    let media = st.media(&media_key(&id, &q)).await?;
    if !media.subtitles.iter().any(|s| s.index == index && s.text) {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such text subtitle track".into()));
    }
    st.subs
        .change_timing(&media, index, change)
        .await
        .map(Json)
        .map_err(ApiError::internal)
}

async fn subtitle_file(
    State(st): State<Arc<AppState>>,
    Path((id, file)): Path<(String, String)>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Response, ApiError> {
    let (index, vtt) = match (file.strip_suffix(".srt"), file.strip_suffix(".vtt")) {
        (Some(i), _) => (i, false),
        (_, Some(i)) => (i, true),
        _ => return Err(ApiError(StatusCode::NOT_FOUND, "want <index>.srt or .vtt".into())),
    };
    let index: i64 = index.parse().map_err(|_| ApiError(StatusCode::NOT_FOUND, "bad index".into()))?;
    auth_or_session(&st, &headers, &q, &id).await?;
    let media = st.media(&media_key(&id, &q)).await?;
    if !media.subtitles.iter().any(|s| s.index == index && s.text) {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such text subtitle track".into()));
    }
    let t0 = Instant::now();
    let srt = st.subs.srt(&media, index).await.map_err(ApiError::internal)?;
    if t0.elapsed() > Duration::from_secs(1) {
        tracing::info!(item = %id, index, secs = t0.elapsed().as_secs_f32(), "subtitles ready");
    }
    let (body, ctype) = if vtt {
        (subs::srt_to_vtt(&srt, media.origin), "text/vtt; charset=utf-8")
    } else {
        (srt, "application/x-subrip; charset=utf-8")
    };
    Ok(([(header::CONTENT_TYPE, ctype), (header::CACHE_CONTROL, "no-cache")], body).into_response())
}

/// `GET /v1/items/{id}/subtitles/{index}/cues?window=<k>` (or `at=<seconds>`): the cues of a
/// text track showing during window `k` (`subs::WINDOW` seconds of the file's time), JSON
/// `{index, window, from, to, span, complete, source, cues: [{start, end, text}]}`. Answered at
/// once from the extracted track, else read for this window alone. `complete`: the whole
/// track is ready (`.srt` answers at once).
async fn subtitle_window(
    State(st): State<Arc<AppState>>,
    Path((id, file)): Path<(String, String)>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Response, ApiError> {
    if !st.config.enabled(Feature::SubsWindows) {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "subtitle windows are switched off".into(),
        ));
    }
    let index: i64 = file.parse().map_err(|_| ApiError(StatusCode::NOT_FOUND, "bad index".into()))?;
    auth_or_session(&st, &headers, &q, &id).await?;
    let k = match (
        q.get("window").and_then(|w| w.parse::<usize>().ok()),
        q.get("at").and_then(|a| a.parse::<f64>().ok()),
    ) {
        (Some(k), _) => k,
        (None, Some(at)) if at >= 0.0 => (at / subs::WINDOW) as usize,
        _ => return Err(ApiError(StatusCode::BAD_REQUEST, "want window=<n> or at=<seconds>".into())),
    };
    let media = st.media(&media_key(&id, &q)).await?;
    let Some(track) = media.subtitles.iter().find(|s| s.index == index) else {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such subtitle track".into()));
    };
    if k as f64 * subs::WINDOW > media.duration + subs::WINDOW {
        return Err(ApiError(StatusCode::NOT_FOUND, "past the end".into()));
    }
    if !track.text {
        // A picture track: its images, with their place on the video.
        let w = st.subs.picture_window(&media, index, k).await.map_err(ApiError::internal)?;
        return Ok(([(header::CACHE_CONTROL, "no-cache")], Json(w)).into_response());
    }
    let w = st.subs.window(&media, index, k).await.map_err(ApiError::internal)?;
    Ok(([(header::CACHE_CONTROL, "no-cache")], Json(w)).into_response())
}

fn os_error(e: anyhow::Error) -> ApiError {
    match e.downcast_ref::<opensubtitles::Refused>() {
        // 406: the daily download quota; 429: too many requests.
        Some(r) if r.status == 406 || r.status == 429 => ApiError(StatusCode::TOO_MANY_REQUESTS, r.to_string()),
        Some(r) => ApiError(StatusCode::BAD_GATEWAY, r.to_string()),
        None => ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    }
}

fn opensubtitles_ready(st: &AppState) -> Result<(), ApiError> {
    if !st.config.enabled(Feature::OpenSubtitles) {
        return Err(ApiError(StatusCode::SERVICE_UNAVAILABLE, "subtitle search is switched off".into()));
    }
    if !st.opensubtitles.configured() {
        return Err(ApiError(StatusCode::SERVICE_UNAVAILABLE, "no OpenSubtitles API key".into()));
    }
    Ok(())
}

async fn subtitles_search(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<serde_json::Value>, ApiError> {
    opensubtitles_ready(&st)?;
    auth(&st, &headers, &q).await?;
    let media = st.media(&media_key(&id, &q)).await?;
    let lang = q
        .get("lang")
        .map(String::as_str)
        .filter(|l| !l.is_empty())
        .unwrap_or("en")
        .to_string();
    let text = q.get("q").cloned().filter(|t| !t.trim().is_empty());
    let local_series = (!media.item.series_provider_ids.is_empty()).then(|| crate::media::item::Item {
        provider_ids: media.item.series_provider_ids.clone(),
        name: media.item.series_name.clone(),
        ..Default::default()
    });
    let series = local_series;
    let hash = opensubtitles::media_hash(&media, &st.http).await;
    let query = opensubtitles::query_for(&media, series.as_ref(), &lang, hash.clone(), text.clone());
    let mut results = st.opensubtitles.search(&query).await.map_err(os_error)?;
    if results.is_empty() && text.is_none() {
        // Nothing for the ids: try the title as text.
        let name = media.item.series_name.clone().or(media.item.name.clone());
        let fallback = opensubtitles::query_for(&media, series.as_ref(), &lang, hash, name);
        if fallback != query {
            results = st.opensubtitles.search(&fallback).await.map_err(os_error)?;
        }
    }
    let fetched: Vec<String> = subs::read_extras_for(&st.subs, &media).into_iter().map(|e| e.remote_id).collect();
    let results: Vec<serde_json::Value> = results
        .into_iter()
        .map(|r| {
            let mut v = serde_json::to_value(&r).unwrap_or_default();
            v["fetched"] = serde_json::Value::Bool(fetched.contains(&r.id));
            v
        })
        .collect();
    tracing::info!(item = %id, lang, hash = query.moviehash.is_some(), results = results.len(), "subtitle search");
    let quota = st.opensubtitles.quota().await;
    Ok(Json(
        serde_json::json!({ "results": results, "remaining_downloads": quota.remaining, "quota": quota }),
    ))
}

#[derive(serde::Deserialize)]
struct FetchBody {
    result_id: String,
    language: Option<String>,
    name: Option<String>,
    hearing_impaired: Option<bool>,
}

async fn subtitles_fetch(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
    Json(body): Json<FetchBody>,
) -> Result<Json<SubtitleTrack>, ApiError> {
    opensubtitles_ready(&st)?;
    auth(&st, &headers, &q).await?;
    let media = st.media(&media_key(&id, &q)).await?;
    let seen = st.opensubtitles.cached_result(&body.result_id);
    let srt = match st.subs.find_fetched("opensubtitles", &body.result_id).await {
        Some(t) => t,
        None => {
            let raw = st.opensubtitles.download(&body.result_id).await.map_err(os_error)?;
            subs::normalize_srt(&subs::decode_text(&raw, body.language.as_deref()))
        }
    };
    if !srt.contains("-->") {
        return Err(ApiError(StatusCode::BAD_GATEWAY, "the subtitle file has no cues".into()));
    }
    let extra = subs::Extra {
        source: "opensubtitles".into(),
        remote_id: body.result_id.clone(),
        language: body.language.clone().or(seen.as_ref().and_then(|r| r.language.clone())),
        release: body.name.clone().or(seen.as_ref().map(|r| r.name.clone())),
        file: String::new(),
        hash_match: seen.as_ref().is_some_and(|r| r.hash_match),
        hearing_impaired: body.hearing_impaired.or(seen.as_ref().map(|r| r.hearing_impaired)).unwrap_or(false),
        created_at: subs::now_secs(),
    };
    let index = st.subs.add_extra(&media, extra, &srt).await.map_err(ApiError::internal)?;
    st.invalidate(&id);
    let media = st.media(&media_key(&id, &q)).await?;
    let track = media
        .subtitles
        .iter()
        .find(|s| s.index == index)
        .cloned()
        .ok_or_else(|| ApiError::internal(anyhow::anyhow!("stored track not listed")))?;
    tracing::info!(item = %id, index, result = %body.result_id, "subtitle fetched");
    st.subs.start_sync(&media, &track, &srt);
    Ok(Json(track))
}

#[derive(serde::Deserialize)]
struct UploadBody {
    /// The file's text (SRT).
    srt: String,
    language: Option<String>,
    name: Option<String>,
}

/// A subtitle file only a device has (an OpenSubtitles download made with the app's own key)
/// becomes a track of the title, like a fetched one: the stream then carries it as a rendition
/// (AirPlay receivers show renditions, not the app's overlay). The same text is stored once.
async fn subtitles_upload(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
    Json(body): Json<UploadBody>,
) -> Result<Json<SubtitleTrack>, ApiError> {
    use sha2::Digest;
    auth(&st, &headers, &q).await?;
    let media = st.media(&media_key(&id, &q)).await?;
    let srt = subs::normalize_srt(&body.srt);
    if !srt.contains("-->") {
        return Err(ApiError(StatusCode::UNPROCESSABLE_ENTITY, "the subtitle file has no cues".into()));
    }
    let digest = sha2::Sha256::digest(srt.as_bytes());
    let hash: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    let extra = subs::Extra {
        source: subs::UPLOAD.into(),
        remote_id: hash.clone(),
        language: body.language.clone().filter(|l| !l.is_empty()),
        release: body.name.clone(),
        file: String::new(),
        hash_match: false,
        hearing_impaired: false,
        created_at: subs::now_secs(),
    };
    let index = st.subs.add_extra(&media, extra, &srt).await.map_err(ApiError::internal)?;
    st.invalidate(&id);
    let media = st.media(&media_key(&id, &q)).await?;
    let track = media
        .subtitles
        .iter()
        .find(|s| s.index == index)
        .cloned()
        .ok_or_else(|| ApiError::internal(anyhow::anyhow!("stored track not listed")))?;
    tracing::info!(item = %id, index, hash = %hash, "subtitle uploaded");
    st.subs.start_sync(&media, &track, &srt);
    Ok(Json(track))
}

// ---------------------------------------------------------------- downloads

fn downloads_ready(st: &AppState) -> Result<(), ApiError> {
    if st.config.enabled(Feature::Downloads) {
        Ok(())
    } else {
        Err(ApiError(StatusCode::SERVICE_UNAVAILABLE, "downloads are switched off".into()))
    }
}

async fn download_original(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Response, ApiError> {
    downloads_ready(&st)?;
    auth(&st, &headers, &q).await?;
    let media = st.media(&media_key(&id, &q)).await?;
    let p = media.source.path();
    let name = Some(crate::storage::vfs::file_name(&p))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "video".into());
    if headers.get(header::RANGE).is_none() {
        tracing::info!(item = %id, file = %name, "original download");
    }
    Ok(downloads::serve_file(&p, &headers, downloads::content_type(&name), &name, None).await)
}

async fn download_job_create(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
    Json(params): Json<JobParams>,
) -> Result<Response, ApiError> {
    downloads_ready(&st)?;
    auth(&st, &headers, &q).await?;
    let media = st.media(&media_key(&id, &q)).await?;
    match st.downloads.create(&media, params) {
        Ok((job, created)) => {
            if created {
                // Its subtitles, so they are ready when the app saves them with the file.
                st.subs.prefetch(&media);
            }
            let status = if created { StatusCode::ACCEPTED } else { StatusCode::OK };
            tracing::info!(item = %id, job = %job.id, label = job.label, method = job.method, created, "download job");
            Ok((status, Json(job)).into_response())
        }
        Err(downloads::Refused(code, msg)) => {
            tracing::info!(item = %id, "download job refused: {msg}");
            Err(ApiError(code, msg))
        }
    }
}

async fn download_jobs_list(State(st): State<Arc<AppState>>, headers: HeaderMap, Query(q): Q) -> Result<Json<serde_json::Value>, ApiError> {
    auth(&st, &headers, &q).await?;
    let (used, reserved) = st.downloads.usage();
    Ok(Json(serde_json::json!({
        "jobs": st.downloads.list(),
        "disk_used": used,
        "disk_reserved": reserved,
        "disk_max": st.downloads.max_bytes(),
        "enabled": st.config.enabled(Feature::Downloads),
    })))
}

async fn download_job_get(
    State(st): State<Arc<AppState>>,
    Path(job): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<downloads::JobInfo>, ApiError> {
    auth(&st, &headers, &q).await?;
    st.downloads
        .get(&job)
        .map(Json)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such job".into()))
}

async fn download_job_delete(
    State(st): State<Arc<AppState>>,
    Path(job): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<StatusCode, ApiError> {
    auth(&st, &headers, &q).await?;
    if st.downloads.delete(&job) {
        tracing::info!(job = %job, "download job deleted");
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError(StatusCode::NOT_FOUND, "no such job".into()))
    }
}

async fn download_job_file(State(st): State<Arc<AppState>>, Path(job): Path<String>, headers: HeaderMap) -> Result<Response, ApiError> {
    downloads_ready(&st)?;
    // The job id is random (like a session id): it is the credential here.
    let info = st
        .downloads
        .get(&job)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such job".into()))?;
    if info.state != "done" {
        return Err(ApiError(StatusCode::CONFLICT, format!("job is {}", info.state)));
    }
    let path = st.downloads.file_of(&job);
    let (st2, job2) = (st.clone(), job.clone());
    let done: Box<dyn FnOnce() + Send> = Box::new(move || st2.downloads.mark_downloaded(&job2));
    Ok(downloads::serve_file(
        &path.to_string_lossy(),
        &headers,
        "video/mp4",
        &format!("{}.mp4", info.item_id),
        Some(done),
    )
    .await)
}

#[cfg(test)]
#[path = "api_dv_tests.rs"]
mod dv_tests;

#[cfg(test)]
#[path = "api_contract_tests.rs"]
mod contract_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::AudioTrack;

    #[test]
    fn lost_sessions_are_gone() {
        // The app recognises a lost session by this status (and recovers with a new one).
        let e = session_gone();
        assert_eq!(e.0, StatusCode::GONE);
        assert_eq!(e.into_response().status(), StatusCode::GONE);
    }

    fn track(index: i64, codec: &str, profile: Option<&str>, channels: i64) -> AudioTrack {
        let conversion = (!matches!(codec, "aac" | "ac3" | "eac3")).then(|| crate::media::audio::Conversion::for_source(channels, 48000));
        AudioTrack {
            index,
            stream: index,
            codec: codec.into(),
            codecs_attr: conversion.as_ref().map_or("ec-3", |c| c.codecs_attr()).into(),
            name: format!("{codec} {channels}"),
            language: None,
            channels: conversion.as_ref().map_or(channels, |c| c.channels as i64),
            default: index == 1,
            converted: conversion.is_some(),
            conversion,
            lossless_source: crate::media::lossless_audio(codec, profile),
            source_channels: channels,
            sample_rate: 48000,
            time_base: 0.001,
        }
    }

    fn view(media: Arc<Media>, opts: StreamOptions) -> View {
        let v = media.video.clone().unwrap();
        View {
            sid: "s".into(),
            owner: None,
            moonrise_item: None,
            media,
            opts,
            variants: Arc::new(crate::media::transcode::ladder(&v, true, false)),
            selected: 0,
        }
    }

    #[test]
    fn a_title_with_a_start_time_plays_on_its_own_timeline() {
        // A file whose clock starts at 1002.39, 130 s long, a boundary every 6 s from its start.
        let mut m = crate::media::transcode::tests::media(crate::media::transcode::tests::video("h264", 1920, 1080, 4_000_000, "SDR"));
        (m.start_time, m.origin, m.duration) = (1002.39, 1002.39, 130.0);
        m.boundaries = std::iter::once(0.0).chain((1..22).map(|i| 1002.39 + 6.0 * i as f64)).collect();
        m.seek_offsets = vec![-1.0; m.boundaries.len()];
        let m = Arc::new(m);
        let pl = media_playlist(&m, true);
        let extinf: Vec<f64> = pl
            .lines()
            .filter_map(|l| l.strip_prefix("#EXTINF:")?.trim_end_matches(',').parse().ok())
            .collect();
        assert_eq!(extinf.len(), 22);
        // The first segment from the file's start, not from its clock's 0; the whole as long as the title.
        assert!((extinf[0] - 6.0).abs() < 0.002, "{pl}");
        assert!((extinf.iter().sum::<f64>() - 130.0).abs() < 0.01, "{pl}");
        assert!(pl.contains("#EXT-X-TARGETDURATION:6\n"), "{pl}");
        // `start` (a position on the timeline, as players give it) begins in the segment holding it.
        let opts = |q: &str| StreamOptions::from(&[("start".to_string(), q.to_string())].into_iter().collect());
        assert_eq!(view(m.clone(), opts("0")).start_segment(), 0);
        assert_eq!(view(m.clone(), opts("13")).start_segment(), 2);
        assert_eq!(view(m.clone(), opts("125")).start_segment(), 20);
    }

    #[test]
    fn a_title_with_no_audio_track_plays_as_video_alone() {
        // A screen recording with no audio track. No audio rendition, no audio group the
        // player would wait for, no audio codec or bandwidth on the variant.
        let m = crate::media::transcode::tests::media(crate::media::transcode::tests::video("h264", 1920, 1080, 4_000_000, "SDR"));
        assert!(m.audio.is_empty());
        let out = master(&view(Arc::new(m.clone()), StreamOptions::from(&HashMap::new())));
        assert!(!out.contains("TYPE=AUDIO") && !out.contains("AUDIO=\"audio\""), "{out}");
        let inf = out.lines().find(|l| l.starts_with("#EXT-X-STREAM-INF:")).unwrap();
        assert_eq!(
            inf,
            "#EXT-X-STREAM-INF:BANDWIDTH=4000000,AVERAGE-BANDWIDTH=4000000,CODECS=\"avc1.640028\",RESOLUTION=1920x1080,FRAME-RATE=23.976,VIDEO-RANGE=SDR"
        );
        // With a text subtitle track: its group alone.
        let mut subbed = m.clone();
        subbed.subtitles = vec![crate::media::SubtitleTrack {
            index: 2,
            stream: Some(2),
            text: true,
            codec: "subrip".into(),
            name: "English".into(),
            language: Some("en".into()),
            forced: false,
            default: false,
            external: false,
            source: "embedded",
            url: None,
            path: None,
            file: None,
            timing: None,
        }];
        let out = master(&view(Arc::new(subbed), StreamOptions::from(&HashMap::new())));
        assert!(out.contains("VIDEO-RANGE=SDR,SUBTITLES=\"subs\"\n"), "{out}");
        assert!(!out.contains("AUDIO"), "{out}");

        // With a track: as before.
        let mut with_audio = m;
        with_audio.audio = vec![track(1, "aac", Some("LC"), 2)];
        let out = master(&view(Arc::new(with_audio), StreamOptions::from(&HashMap::new())));
        assert!(out.contains("VIDEO-RANGE=SDR,AUDIO=\"audio\"\n"), "{out}");
        assert!(out.contains("BANDWIDTH=4640000,AVERAGE-BANDWIDTH=4640000,"), "{out}");
    }

    #[test]
    fn a_picture_subtitle_track_keeps_the_copy() {
        // A PGS track, asked for as old apps did (`burn=`): the stream is the copied original,
        // with the transcodes below it only as lower qualities; the track isn't in the stream
        // (the app draws it from the subtitle windows).
        let mut m = crate::media::transcode::tests::media(crate::media::transcode::tests::video("h264", 1920, 1080, 12_000_000, "SDR"));
        m.subtitles = vec![crate::media::SubtitleTrack {
            index: 2,
            stream: Some(2),
            text: false,
            codec: "PGSSUB".into(),
            name: "English".into(),
            language: Some("en".into()),
            forced: false,
            default: false,
            external: false,
            source: "embedded",
            url: None,
            path: None,
            file: None,
            timing: None,
        }];
        let opts = StreamOptions::from(&[("burn".to_string(), "2".to_string())].into_iter().collect());
        let v = m.video.clone().unwrap();
        let ladder = crate::media::transcode::ladder_with(&crate::media::hwaccel::Hw::nvenc(), &v, opts.hdr_display, true);
        assert_eq!(ladder[0].id, "copy");
        assert!(ladder.iter().skip(1).all(|x| x.transcode && !x.id.contains("-b")), "{ladder:?}");
        let out = master(&view(Arc::new(m), opts));
        assert!(!out.contains("SUBTITLES"), "{out}");
    }

    #[test]
    fn lossless_audio_only_when_asked() {
        let mut m = crate::media::transcode::tests::media(crate::media::transcode::tests::video("hevc", 3840, 2160, 40_000_000, "PQ"));
        m.audio = vec![
            track(1, "truehd", None, 8),
            track(2, "dts", Some("DTS-HD MA"), 6),
            track(3, "dts", Some("DTS"), 6),
            track(4, "eac3", None, 6),
            track(5, "flac", None, 2),
        ];
        let m = Arc::new(m);
        let q = |s: &str| {
            StreamOptions::from(
                &s.split('&')
                    .filter_map(|kv| kv.split_once('='))
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            )
        };

        // Without `lossless=1`: the shared media as it is, E-AC-3 5.1 for every multichannel source.
        let plain = AppState::media_for(m.clone(), &q("hdr=1"));
        assert!(Arc::ptr_eq(&plain, &m));
        let master_plain = master(&view(plain, q("hdr=1")));
        assert!(master_plain.contains(",ec-3,mp4a.40.2\""), "{master_plain}");
        assert!(
            !master_plain.contains("fLaC") && master_plain.contains("BANDWIDTH=40640000,"),
            "{master_plain}"
        );

        // With it: TrueHD 7.1 and DTS-HD MA 5.1 as FLAC with all their channels; DTS core,
        // copied E-AC-3 and stereo FLAC as before.
        let opts = q("hdr=1&lossless=1");
        let ll = AppState::media_for(m.clone(), &opts);
        let got: Vec<(&str, i64)> = ll.audio.iter().map(|a| (a.codecs_attr.as_str(), a.channels)).collect();
        assert_eq!(got, [("fLaC", 8), ("fLaC", 6), ("ec-3", 6), ("ec-3", 6), ("mp4a.40.2", 2)]);
        assert_eq!(ll.audio[0].conversion.as_ref().unwrap().codec, crate::media::audio::Codec::Flac);
        assert_eq!(ll.audio[2].conversion.as_ref().unwrap().codec, crate::media::audio::Codec::Eac3);
        assert!(
            m.audio[0].conversion.as_ref().unwrap().codec == crate::media::audio::Codec::Eac3,
            "the shared media is untouched"
        );
        let out = master(&view(ll, opts));
        assert!(out.contains("CHANNELS=\"8\",URI=\"/v1/s/s/audio/1/index.m3u8\""), "{out}");
        assert!(out.contains(",fLaC,ec-3,mp4a.40.2\""), "{out}");
        // BANDWIDTH counts the heaviest rendition: the 7.1 FLAC estimate.
        assert!(out.contains("BANDWIDTH=45529000,"), "{out}");

        // A title with no lossless multichannel track: the shared media even when asked.
        let mut stereo = (*m).clone();
        stereo.audio = vec![track(1, "dts", Some("DTS"), 6)];
        let stereo = Arc::new(stereo);
        assert!(Arc::ptr_eq(&AppState::media_for(stereo.clone(), &opts), &stereo));
    }

    #[test]
    fn lossless_sources() {
        use crate::media::lossless_audio;
        for (c, p) in [
            ("truehd", None),
            ("mlp", None),
            ("flac", None),
            ("alac", None),
            ("pcm_s24le", None),
            ("dts", Some("DTS-HD MA")),
            ("dts", Some("DTS-HD MA + DTS:X")),
        ] {
            assert!(lossless_audio(c, p), "{c} {p:?}");
        }
        for (c, p) in [
            ("dts", Some("DTS")),
            ("dts", Some("DTS-ES")),
            ("dts", Some("DTS-HD HRA")),
            ("dts", None),
            ("eac3", None),
            ("opus", None),
            ("mp3", None),
        ] {
            assert!(!lossless_audio(c, p), "{c} {p:?}");
        }
    }
}
