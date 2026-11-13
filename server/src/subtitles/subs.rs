//! Text subtitles, served as SRT (`/v1/items/{id}/subtitles/{index}.srt`) and as WebVTT
//! renditions of the HLS stream.
//!
//! - **Embedded tracks** are extracted here, every text track of a file at once, with the
//!   file's own timestamps, and kept in `/data/subs/v1/<file key>/` (persistent, never evicted,
//!   a few tens of KB per track; `done.json` marks a file finished, `failed.json` one that
//!   couldn't be read). Matroska files are read directly (`subx.rs`): by the index, only the
//!   subtitle blocks (seconds for a film), or by scanning the clusters; other containers by
//!   ffmpeg (one pass mapping every text track).
//! - **One file is read at a time**, the most urgent first: a title being played (from
//!   `play` until its session closes) or a client waiting for a track; among those the latest
//!   asked for wins and the others pause. A job nobody waits for any more goes on as background
//!   work. Background work (prepared downloads, the next episodes, the library backfill: every
//!   file once, newest first) is paced down while anything streams.
//! - **Sidecar files** (`.srt` and friends next to the media) are read from the media share at
//!   the path the scan found, decoded (UTF-8, UTF-16, else Windows-1251/1252 by a guess) and
//!   converted to SRT.
//! - **Fetched tracks** (OpenSubtitles, `opensubtitles.rs`) live in `/data/subs/<media key>/`
//!   with an `extras.json` listing them; they are listed as external tracks (index 10000 + n)
//!   on every device. Nothing is written to the media share.
//!
//! Cue times are on the title's timeline, which players start at the file's start time
//! (`Media::origin`): an embedded track's cues (the file's own timestamps) are served that much
//! earlier. The video's timestamps in the HLS segments (tfdt) are the file's own, so the WebVTT
//! (made from the SRT) maps 00:00 to MPEGTS `origin`: 0 for most files.
//!
//! Fetched and uploaded tracks are served matched to the file's timing, when that could be done
//! with confidence (`subsync.rs`); their originals stay.
//!
//! With `subs_extract` switched off, embedded and sidecar text tracks aren't served.

use std::fmt::Write as _;
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::{watch, Notify};

use crate::{
    media::mkv,
    media::moov,
    media::{fnv, Media, SubtitleTrack},
    storage::source::Source,
    subtitles::subx,
};

/// Bump when extraction output changes (a new directory; the old one can be deleted).
const EXTRACT_VERSION: u32 = 1;
/// Fetched tracks are numbered from here (a file's own track indices are small).
pub const EXTRA_BASE: i64 = 10_000;
/// An extraction taking longer than this is stopped (a 35 GB file read whole over CIFS takes
/// ~8 min; paused time counts too).
const EXTRACT_TIMEOUT: Duration = Duration::from_hours(4);

/// Stream segments being cut right now (`segment.rs`): extraction holds its next read
/// meanwhile, so a played title's own segments never queue behind its subtitles on the share.
pub static STREAM_READS: AtomicUsize = AtomicUsize::new(0);

/// Counts a stream read for as long as it lives.
pub struct StreamRead;

impl StreamRead {
    pub fn start() -> StreamRead {
        STREAM_READS.fetch_add(1, Ordering::SeqCst);
        StreamRead
    }
}

impl Drop for StreamRead {
    fn drop(&mut self) {
        STREAM_READS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Subtitle windows being read for a client (`Subs::window`): extraction holds its next read
/// meanwhile, as for stream cuts.
pub static WINDOW_READS: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct WindowRead;

impl WindowRead {
    pub(crate) fn start() -> WindowRead {
        WINDOW_READS.fetch_add(1, Ordering::SeqCst);
        WindowRead
    }
}

impl Drop for WindowRead {
    fn drop(&mut self) {
        WINDOW_READS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A subtitle window's length: window `k` is [k·WINDOW, (k+1)·WINDOW) of the file's time (the
/// HLS rendition's segments are the same windows).
pub const WINDOW: f64 = 30.0;
/// Blocks without a cue duration starting this long before a window may still show in it.
const WINDOW_LEAD: f64 = 10.0;
/// The same when scanning (every second of lead is read whole).
const SCAN_LEAD: f64 = 6.0;
/// Windows kept in memory (a few KB each).
const WINDOWS_KEPT: usize = 4000;
/// Parsed indexes kept in memory (by file).
const INDEXES_KEPT: usize = 32;

/// A window's cues, and where they came from.
#[derive(Clone, Debug, Serialize)]
pub struct Window {
    pub index: i64,
    pub window: usize,
    pub from: f64,
    pub to: f64,
    pub span: f64,
    /// The whole track is ready (its SRT answers at once): fetch that instead of windows.
    pub complete: bool,
    /// "file" (the extracted or sidecar track), "memory", "index", "scan", "mp4", "ffmpeg".
    pub source: &'static str,
    pub cues: Arc<Vec<subx::Cue>>,
}

/// A scanned window's pace: each read waits while a stream segment is cut (so the video of what
/// plays doesn't wait for its subtitles), at most 1.5 s in all for the window.
pub(crate) fn yield_to_streams() -> impl Fn() -> futures_util::future::BoxFuture<'static, Result<subx::Step>> + Send + Sync {
    let waited = Arc::new(Mutex::new(Duration::ZERO));
    move || {
        let waited = waited.clone();
        Box::pin(async move {
            let t0 = Instant::now();
            let budget = Duration::from_millis(1500).saturating_sub(*waited.lock().unwrap());
            while STREAM_READS.load(Ordering::SeqCst) > 0 && t0.elapsed() < budget {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            *waited.lock().unwrap() += t0.elapsed();
            Ok(subx::Step {
                parallel: 2,
                rest: Duration::ZERO,
                rate: 0,
            })
        })
    }
}

/// A file's index as read for windows.
pub(crate) enum FileIndex {
    Mkv {
        index: Arc<mkv::Index>,
        file: subx::File,
        header: Option<u64>,
    },
    Mp4 {
        tracks: Arc<Vec<moov::TextTrack>>,
        file: subx::File,
    },
}

/// How long a title counts as being played after `play` (unless its session closes first).
const HOT_FOR: Duration = Duration::from_secs(600);
/// Streaming counts as going on this long after the last stream request.
const STREAMING_FOR: Duration = Duration::from_secs(90);

/// Why a file's tracks are wanted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Demand {
    /// A client is waiting for a track now.
    Wait,
    /// Ahead of need (a download being prepared, the next episode, the backfill).
    Background,
}

/// An extraction (one file, every text track), queued or running.
struct Job {
    key: String,
    media: Arc<Media>,
    done: watch::Receiver<Option<Result<(), String>>>,
    /// Requests waiting for it now.
    waiters: AtomicUsize,
    /// Last asked for (a request or a play): the latest urgent job goes first.
    touched: Mutex<Instant>,
    /// Queue order among background jobs.
    seq: u64,
    running: AtomicBool,
    /// How the file is read: 0 not known yet, 1 by the index (seconds), 2 whole (a scan or
    /// ffmpeg: minutes). A played title is urgent only while it is cheap: the app shows
    /// windows meanwhile (`window`).
    cost: std::sync::atomic::AtomicU8,
}

pub struct Subs {
    /// `/data/subs`
    dir: PathBuf,
    pub(crate) ffmpeg: String,
    /// `subs_extract` is on.
    extract: bool,
    jobs: Mutex<HashMap<String, Arc<Job>>>,
    /// A job came or went, or its priority changed.
    changed: Notify,
    seq: AtomicU64,
    /// Titles being played: session → (file key, since).
    hot: Mutex<HashMap<String, (String, Instant)>>,
    /// The last stream request (segments, playlists): background work slows down meanwhile.
    last_stream: Mutex<Option<Instant>>,
    /// Item ids to extract before the rest of the backfill (the next episodes of what plays).
    hints: Mutex<VecDeque<String>>,
    pub backfill_wake: Notify,
    pub backfill: Mutex<BackfillStatus>,
    /// Serialises writes of `extras.json`.
    extras_lock: tokio::sync::Mutex<()>,
    /// Windows made (by file key, stream, window number).
    windows: Mutex<HashMap<(String, i64, usize), (Arc<Vec<subx::Cue>>, Instant)>>,
    /// Windows being made: everyone asking for the same one waits on it.
    window_inflight: Mutex<HashMap<String, Arc<tokio::sync::OnceCell<Result<&'static str, String>>>>>,
    /// Indexes read for windows and extraction (by file key), with an open handle of the file.
    indexes: Mutex<HashMap<String, (Arc<FileIndex>, Instant)>>,
    index_inflight: Mutex<HashMap<String, Arc<tokio::sync::OnceCell<Result<(), String>>>>>,
    /// Scanned windows read one at a time (the next one waits for the one at the playhead).
    pub(crate) scan_gate: tokio::sync::Semaphore,
    /// ffprobe, for picture subtitle packets outside Matroska (`pictures.rs`).
    pub(crate) ffprobe: String,
    /// Picture subtitle windows made, and those being made (`pictures.rs`).
    pub(crate) pictures: crate::subtitles::pictures::Cache,
    /// Matching fetched and uploaded tracks to the file (`subsync.rs`).
    pub(crate) sync: crate::subtitles::subsync::Jobs,
    /// The switches: `subs_sync` can change while the server runs.
    switches: crate::admin::config::Switches,
}

/// The library backfill's progress (`GET /v1/jobs`).
#[derive(Clone, Debug, Default, Serialize)]
pub struct BackfillStatus {
    pub files: usize,
    pub done: usize,
    pub failed: usize,
    pub extracted_this_run: usize,
    pub secs_this_run: f64,
    pub current: Option<String>,
    pub pass_started: Option<u64>,
}

/// Marks a file's tracks all extracted (or none to extract).
#[derive(Serialize, Deserialize)]
struct Done {
    streams: Vec<i64>,
    method: String,
    secs: f64,
    at: u64,
}

#[derive(Serialize, Deserialize)]
struct Failed {
    error: String,
    at: u64,
}

/// Background work is retried after this long.
const RETRY_FAILED: u64 = 7 * 86_400;

/// `Extra::source` of a file a device sent (`POST …/subtitles/upload`).
pub const UPLOAD: &str = "upload";

/// A fetched (or uploaded) track, as listed in `extras.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Extra {
    /// "opensubtitles", or "upload"
    pub source: String,
    /// The provider's file id (an upload: its text's hash).
    pub remote_id: String,
    /// ISO 639-1 (as OpenSubtitles gives it, e.g. "en", "pt-BR").
    pub language: Option<String>,
    pub release: Option<String>,
    /// File name in the same directory.
    pub file: String,
    #[serde(default)]
    pub hash_match: bool,
    #[serde(default)]
    pub hearing_impaired: bool,
    pub created_at: u64,
}

impl Subs {
    pub fn new(dir: PathBuf, ffmpeg: String, ffprobe: String, extract: bool, switches: crate::admin::config::Switches) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Subs {
            dir,
            ffmpeg,
            extract,
            jobs: Mutex::new(HashMap::new()),
            changed: Notify::new(),
            seq: AtomicU64::new(0),
            hot: Mutex::new(HashMap::new()),
            last_stream: Mutex::new(None),
            hints: Mutex::new(VecDeque::new()),
            backfill_wake: Notify::new(),
            backfill: Mutex::new(BackfillStatus::default()),
            extras_lock: tokio::sync::Mutex::new(()),
            windows: Mutex::new(HashMap::new()),
            window_inflight: Mutex::new(HashMap::new()),
            indexes: Mutex::new(HashMap::new()),
            index_inflight: Mutex::new(HashMap::new()),
            scan_gate: tokio::sync::Semaphore::new(1),
            ffprobe,
            pictures: crate::subtitles::pictures::Cache::default(),
            sync: crate::subtitles::subsync::Jobs::new(),
            switches,
        }
    }

    pub fn enabled(&self) -> bool {
        self.extract
    }

    /// Fetched and uploaded tracks are matched to the file's timing.
    pub(crate) fn sync_enabled(&self) -> bool {
        !self.switches.contains(crate::admin::config::Feature::SubsSync.key())
    }

    pub(crate) fn extract_dir(&self, media: &Media) -> PathBuf {
        self.dir_for_key(&media.file_key)
    }

    fn dir_for_key(&self, file_key: &str) -> PathBuf {
        self.dir.join(format!("v{EXTRACT_VERSION}")).join(file_key)
    }

    /// The backfill has nothing to do for this file: finished, or failed not long ago.
    pub fn settled(&self, file_key: &str) -> bool {
        let dir = self.dir_for_key(file_key);
        if dir.join("done.json").is_file() {
            return true;
        }
        std::fs::read(dir.join("failed.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Failed>(&b).ok())
            .is_some_and(|f| now_secs().saturating_sub(f.at) < RETRY_FAILED)
    }

    /// A stream request: background extraction slows down while anything plays.
    pub fn streaming(&self) {
        *self.last_stream.lock().unwrap() = Some(Instant::now());
    }

    fn is_streaming(&self) -> bool {
        self.streaming_within(STREAMING_FOR)
    }

    /// A stream request came in the last `d`.
    pub fn streaming_within(&self, d: Duration) -> bool {
        self.last_stream.lock().unwrap().is_some_and(|t| t.elapsed() < d)
    }

    /// Item ids worth extracting before the rest of the backfill (the next episodes of what
    /// plays), most wanted first.
    pub fn hint(&self, next: Vec<String>) {
        if !self.extract || next.is_empty() {
            return;
        }
        let mut h = self.hints.lock().unwrap();
        for id in next.into_iter().rev() {
            h.retain(|x| *x != id);
            h.push_front(id);
        }
        h.truncate(20);
        drop(h);
        self.backfill_wake.notify_one();
    }

    /// A title starts playing (a new session): its tracks are extracted first, ahead of
    /// anything else, until the session closes.
    pub fn play(self: &Arc<Self>, media: &Arc<Media>, sid: &str) {
        if !self.extract {
            return;
        }
        if !has_embedded_text(media) {
            return;
        }
        {
            let mut hot = self.hot.lock().unwrap();
            hot.retain(|_, (_, at)| at.elapsed() < HOT_FOR);
            hot.insert(sid.to_string(), (media.file_key.clone(), Instant::now()));
        }
        let this = self.clone();
        let media = media.clone();
        tokio::spawn(async move {
            let _ = this.ensure_extracted(&media, Demand::Background).await;
        });
        self.changed.notify_waiters();
    }

    /// The session ended: its title is no longer urgent (its extraction goes on in the
    /// background).
    pub fn session_closed(&self, sid: &str) {
        if self.hot.lock().unwrap().remove(sid).is_some() {
            self.changed.notify_waiters();
        }
    }

    /// The next item id the backfill should look at before its list.
    pub fn next_hint(&self) -> Option<String> {
        self.hints.lock().unwrap().pop_front()
    }

    /// Where a title's fetched tracks are kept: by the library's file id (it survives
    /// renames), else by the file's path.
    /// A path-keyed directory is moved to the file id the first time the id is known.
    pub fn extras_dir(&self, media: &Media) -> PathBuf {
        match &media.source {
            Source::Local(_) | Source::Smb(_) => {
                let legacy = self.dir.join(format!("p{:016x}", fnv(&media.source.path())));
                let Some(id) = &media.file_id else { return legacy };
                let dir = self.dir.join(format!("f-{id}"));
                if !dir.exists() && legacy.is_dir() {
                    if let Err(e) = std::fs::rename(&legacy, &dir) {
                        tracing::warn!("move fetched subtitles to the file id: {e}");
                        return legacy;
                    }
                }
                dir
            }
        }
    }

    /// The fetched tracks of a title, as subtitle tracks (appended to the media's).
    pub fn extras(&self, media: &Media) -> Vec<SubtitleTrack> {
        let dir = self.extras_dir(media);
        read_extras(&dir)
            .into_iter()
            .enumerate()
            .filter(|(_, e)| dir.join(&e.file).is_file())
            .map(|(i, e)| {
                let index = EXTRA_BASE + i as i64;
                let lang = e.language.clone().unwrap_or_default();
                let uploaded = e.source == UPLOAD;
                let origin = if uploaded { "Added" } else { "OpenSubtitles" };
                let mut name = format!("{} - {origin}", language_name(&lang));
                if e.hearing_impaired {
                    name = format!("{} (SDH) - {origin}", language_name(&lang));
                }
                SubtitleTrack {
                    index,
                    stream: None,
                    text: true,
                    codec: "subrip".into(),
                    name,
                    language: e.language.as_deref().map(|l| l.split('-').next().unwrap_or(l).to_ascii_lowercase()),
                    forced: false,
                    default: false,
                    external: true,
                    source: if uploaded { UPLOAD } else { "opensubtitles" },
                    url: Some(SubtitleTrack::url_for(&media.item_id, index)),
                    path: None,
                    file: Some(dir.join(&e.file)),
                    timing: None,
                }
            })
            .collect()
    }

    /// Stores a fetched track for a title; returns its index. The same remote file is stored
    /// once per title.
    pub async fn add_extra(&self, media: &Media, mut extra: Extra, srt: &str) -> Result<i64> {
        let _g = self.extras_lock.lock().await;
        let dir = self.extras_dir(media);
        tokio::fs::create_dir_all(&dir).await?;
        let mut list = read_extras(&dir);
        if let Some(i) = list.iter().position(|e| e.source == extra.source && e.remote_id == extra.remote_id) {
            return Ok(EXTRA_BASE + i as i64);
        }
        let prefix = if extra.source == UPLOAD { "up" } else { "os" };
        extra.file = format!("{prefix}-{}.srt", sanitize(&extra.remote_id));
        write_atomic(&dir.join(&extra.file), srt.as_bytes()).await?;
        list.push(extra);
        write_atomic(&dir.join("extras.json"), &serde_json::to_vec_pretty(&list)?).await?;
        Ok(EXTRA_BASE + list.len() as i64 - 1)
    }

    /// A remote file fetched before, for any title (so it isn't downloaded again).
    pub async fn find_fetched(&self, source: &str, remote_id: &str) -> Option<String> {
        let name = format!("os-{}.srt", sanitize(remote_id));
        let dir = self.dir.clone();
        let source = source.to_string();
        let remote_id = remote_id.to_string();
        tokio::task::spawn_blocking(move || {
            for e in std::fs::read_dir(&dir).ok()?.flatten() {
                let d = e.path();
                if !d.is_dir() || !d.join(&name).is_file() {
                    continue;
                }
                if read_extras(&d).iter().any(|x| x.source == source && x.remote_id == remote_id) {
                    return std::fs::read_to_string(d.join(&name)).ok();
                }
            }
            None
        })
        .await
        .ok()
        .flatten()
    }

    /// Track `index` of a title as SRT (UTF-8), matched to the file's timing if it came from
    /// elsewhere (`subsync.rs`). A client is waiting: extraction goes first.
    pub async fn srt(self: &Arc<Self>, media: &Arc<Media>, index: i64) -> Result<String> {
        let track = media
            .subtitles
            .iter()
            .find(|s| s.index == index && s.text)
            .context("no such text subtitle track")?;
        let original = self.original_srt(media, track).await?;
        if crate::subtitles::subsync::matchable(track) {
            return Ok(self.timed(media, track, original).await);
        }
        Ok(on_timeline(media, track, original))
    }

    /// A text track as it is, before any matching.
    pub(crate) async fn original_srt(self: &Arc<Self>, media: &Arc<Media>, track: &SubtitleTrack) -> Result<String> {
        if let Some(f) = &track.file {
            return Ok(normalize_srt(&decode_text(&tokio::fs::read(f).await?, track.language.as_deref())));
        }
        anyhow::ensure!(self.extract, "subtitle extraction is switched off");
        let r = match (track.stream, &track.path) {
            (Some(stream), _) => self.embedded(media, stream).await,
            (None, Some(path)) if crate::storage::vfs::is_file(path).await => self.sidecar(media, track, path).await,
            _ => Err(anyhow::anyhow!("subtitle {} isn't readable here", track.index)),
        };
        if let Err(e) = &r {
            tracing::warn!(item = %media.item_id, index = track.index, "subtitle extraction failed: {e:#}");
        }
        r
    }

    /// Track `index` as WebVTT for the HLS rendition, with the viewer's nudge (its player can't
    /// delay it, as the app's overlay does).
    pub async fn vtt(self: &Arc<Self>, media: &Arc<Media>, index: i64) -> Result<String> {
        let srt = self.srt(media, index).await?;
        let nudge = self.nudge(media, index);
        if nudge == 0.0 {
            return Ok(srt_to_vtt(&srt, media.origin));
        }
        Ok(cues_to_vtt(&nudged(parse_srt(&srt), nudge), media.origin))
    }

    /// The cues of a rendition's segment `k` (a subtitle window of the video's time), with the
    /// viewer's nudge: so from the windows that many seconds away.
    pub async fn rendition_window(self: &Arc<Self>, media: &Arc<Media>, index: i64, k: usize) -> Result<Arc<Vec<subx::Cue>>> {
        let nudge = self.nudge(media, index);
        if nudge == 0.0 {
            return Ok(self.window(media, index, k).await?.cues);
        }
        let (from, to) = (k as f64 * WINDOW, (k + 1) as f64 * WINDOW);
        let first = ((from - nudge) / WINDOW).floor();
        let last = ((to - nudge) / WINDOW).floor();
        let mut cues: Vec<subx::Cue> = Vec::new();
        if last >= 0.0 {
            for w in (first.max(0.0) as usize)..=(last as usize) {
                for c in self.window(media, index, w).await?.cues.iter() {
                    if !cues.contains(c) {
                        cues.push(c.clone());
                    }
                }
            }
        }
        Ok(Arc::new(subx::overlapping(nudged(cues, nudge), from, to)))
    }

    /// Extract a title's embedded text tracks (if not done) in the background, without
    /// waiting.
    pub fn prefetch(self: &Arc<Self>, media: &Arc<Media>) {
        if !self.extract || !has_embedded_text(media) {
            return;
        }
        let (this, media) = (self.clone(), media.clone());
        tokio::spawn(async move {
            let _ = this.ensure_extracted(&media, Demand::Background).await;
        });
    }

    /// The extracted file of an embedded track, if it is there already.
    pub fn ready(&self, media: &Media, index: i64) -> bool {
        match media.subtitles.iter().find(|s| s.index == index) {
            Some(SubtitleTrack { stream: Some(st), .. }) => self.extract_dir(media).join(format!("s{st}.srt")).is_file(),
            _ => true,
        }
    }

    async fn embedded(self: &Arc<Self>, media: &Arc<Media>, stream: i64) -> Result<String> {
        let path = self.extract_dir(media).join(format!("s{stream}.srt"));
        if !path.is_file() {
            self.ensure_extracted(media, Demand::Wait).await?;
        }
        let bytes = tokio::fs::read(&path)
            .await
            .with_context(|| format!("stream {stream} not extracted"))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn streams_of(media: &Media) -> Vec<i64> {
        let mut v: Vec<i64> = media.subtitles.iter().filter(|s| s.text).filter_map(|s| s.stream).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Every embedded text track of the file (shared by concurrent callers; it runs to the end
    /// even when they go away, as background work then).
    pub async fn ensure_extracted(self: &Arc<Self>, media: &Arc<Media>, demand: Demand) -> Result<()> {
        let dir = self.extract_dir(media);
        let streams = Self::streams_of(media);
        if streams.iter().all(|s| dir.join(format!("s{s}.srt")).is_file()) {
            return Ok(());
        }
        let job = {
            let mut jobs = self.jobs.lock().unwrap();
            match jobs.get(&media.file_key) {
                Some(j) => j.clone(),
                None => {
                    let (tx, rx) = watch::channel(None);
                    let job = Arc::new(Job {
                        key: media.file_key.clone(),
                        media: media.clone(),
                        done: rx,
                        waiters: AtomicUsize::new(0),
                        touched: Mutex::new(Instant::now()),
                        seq: self.seq.fetch_add(1, Ordering::Relaxed),
                        running: AtomicBool::new(false),
                        cost: std::sync::atomic::AtomicU8::new(0),
                    });
                    jobs.insert(media.file_key.clone(), job.clone());
                    let this = self.clone();
                    let j = job.clone();
                    tokio::spawn(async move {
                        let r = this.run(&j).await;
                        this.jobs.lock().unwrap().remove(&j.key);
                        let _ = tx.send(Some(r));
                        this.changed.notify_waiters();
                    });
                    job
                }
            }
        };
        // A waiting client makes it urgent for as long as it waits.
        let _guard = (demand == Demand::Wait).then(|| {
            job.waiters.fetch_add(1, Ordering::SeqCst);
            *job.touched.lock().unwrap() = Instant::now();
            self.changed.notify_waiters();
            WaitGuard {
                job: job.clone(),
                subs: self,
            }
        });
        let mut rx = job.done.clone();
        let result = loop {
            if let Some(r) = rx.borrow_and_update().clone() {
                break r;
            }
            if rx.changed().await.is_err() {
                break Err("extraction ended".into());
            }
        };
        result.map_err(|e| anyhow::anyhow!(e))
    }

    /// A job is urgent while a client waits for it, or its title plays and the file is read
    /// by its index (seconds). A played title read whole (minutes) goes first among the
    /// background work instead: the app shows windows of it meanwhile.
    fn urgent(&self, job: &Job) -> bool {
        if job.waiters.load(Ordering::SeqCst) > 0 {
            return true;
        }
        job.cost.load(Ordering::SeqCst) != 2 && self.hot(job)
    }

    fn hot(&self, job: &Job) -> bool {
        let hot = self.hot.lock().unwrap();
        hot.values().any(|(k, at)| *k == job.key && at.elapsed() < HOT_FOR)
    }

    /// The latest time a play of this job's title started (for ordering urgent jobs).
    fn last_asked(&self, job: &Job) -> Instant {
        let t = *job.touched.lock().unwrap();
        let hot = self.hot.lock().unwrap();
        hot.values().filter(|(k, _)| *k == job.key).map(|(_, at)| *at).fold(t, Instant::max)
    }

    /// The job that may read now: urgent before played before background; among urgent and
    /// played ones the latest asked for, among background ones the oldest.
    fn top(&self) -> Option<String> {
        let jobs: Vec<Arc<Job>> = self.jobs.lock().unwrap().values().cloned().collect();
        jobs.iter()
            .map(|j| {
                let recent = -(self.last_asked(j).elapsed().as_millis() as i128);
                let key = if self.urgent(j) {
                    (2, recent, 0)
                } else if self.hot(j) {
                    (1, recent, 0)
                } else {
                    (0, 0, -(j.seq as i128))
                };
                (key, j)
            })
            .max_by_key(|(k, _)| *k)
            .map(|(_, j)| j.key.clone())
    }

    /// Waits until `job` may read, and says how fast.
    async fn turn(&self, job: &Job) -> subx::Step {
        loop {
            // Streams first (a cut takes a fraction of a second).
            let t0 = Instant::now();
            while (STREAM_READS.load(Ordering::SeqCst) > 0 || WINDOW_READS.load(Ordering::SeqCst) > 0)
                && t0.elapsed() < Duration::from_secs(10)
            {
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
            let notified = self.changed.notified();
            if self.top().as_deref() == Some(job.key.as_str()) {
                return if self.urgent(job) {
                    subx::Step {
                        parallel: 8,
                        rest: Duration::ZERO,
                        rate: 0,
                    }
                } else if self.is_streaming() {
                    subx::Step {
                        parallel: 1,
                        rest: Duration::from_millis(150),
                        rate: 12_000_000,
                    }
                } else {
                    subx::Step {
                        parallel: 3,
                        rest: Duration::from_millis(20),
                        rate: 50_000_000,
                    }
                };
            }
            let _ = tokio::time::timeout(Duration::from_secs(1), notified).await;
        }
    }

    async fn run(self: &Arc<Self>, job: &Arc<Job>) -> Result<(), String> {
        let media = &job.media;
        let dir = self.extract_dir(media);
        let streams = Self::streams_of(media);
        let queued = Instant::now();
        let first = self.turn(job).await;
        job.running.store(true, Ordering::SeqCst);
        if queued.elapsed() > Duration::from_secs(2) {
            tracing::info!(item = %media.item_id, waited = queued.elapsed().as_secs(), "subtitle extraction starting");
        }
        let _ = first;
        let t0 = Instant::now();
        let r = tokio::time::timeout(EXTRACT_TIMEOUT, self.extract_all(job, &streams, &dir))
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("timed out")));
        let secs = t0.elapsed().as_secs_f64();
        match &r {
            Ok((n, method)) => {
                tracing::info!(item = %media.item_id, tracks = n, method, urgent = self.urgent(job), secs, "subtitles extracted");
                let done = Done {
                    streams: streams.clone(),
                    method: method.to_string(),
                    secs,
                    at: now_secs(),
                };
                let _ = tokio::fs::create_dir_all(&dir).await;
                let _ = write_atomic(&dir.join("done.json"), &serde_json::to_vec(&done).unwrap_or_default()).await;
                let _ = tokio::fs::remove_file(dir.join("failed.json")).await;
            }
            Err(e) => {
                tracing::warn!(item = %media.item_id, "subtitle extraction failed: {e:#}");
                let f = Failed {
                    error: format!("{e:#}"),
                    at: now_secs(),
                };
                let _ = tokio::fs::create_dir_all(&dir).await;
                let _ = write_atomic(&dir.join("failed.json"), &serde_json::to_vec(&f).unwrap_or_default()).await;
            }
        }
        r.map(|_| ()).map_err(|e| format!("{e:#}"))
    }

    /// Writes every listed stream to `dir/s<stream>.srt`: Matroska directly, else (or if that
    /// fails) ffmpeg. Returns how many were written and how.
    async fn extract_all(self: &Arc<Self>, job: &Arc<Job>, streams: &[i64], dir: &Path) -> Result<(usize, &'static str)> {
        tokio::fs::create_dir_all(dir).await?;
        let todo: Vec<i64> = streams
            .iter()
            .copied()
            .filter(|s| !dir.join(format!("s{s}.srt")).is_file())
            .collect();
        if todo.is_empty() {
            return Ok((0, "none"));
        }
        let media = &job.media;
        if media.container.contains("matroska") || media.container.contains("webm") {
            match self.extract_mkv(job, &todo, dir).await {
                Ok(m) => return Ok((todo.len(), m)),
                Err(e) => tracing::info!(item = %media.item_id, "direct subtitle read failed ({e:#}): ffmpeg instead"),
            }
        }
        let n = self.extract_ffmpeg(job, &todo, dir).await?;
        Ok((n, "ffmpeg"))
    }

    async fn extract_mkv(self: &Arc<Self>, job: &Arc<Job>, todo: &[i64], dir: &Path) -> Result<&'static str> {
        let media = &job.media;
        // The index and open handle windows use too (one open of the file, one index read).
        let FileIndex::Mkv { index, file, .. } = self.file_index(media).await? else {
            bail!("not Matroska")
        };
        let wants = subx::wants(&index, todo).context("tracks don't match the file's")?;
        let by_index = subx::method(&index, &wants) == subx::Method::Cues;
        job.cost.store(if by_index { 1 } else { 2 }, Ordering::SeqCst);
        self.changed.notify_waiters();
        let this = self.clone();
        let j = job.clone();
        let pace = move || -> futures_util::future::BoxFuture<'static, Result<subx::Step>> {
            let (this, j) = (this.clone(), j.clone());
            Box::pin(async move { Ok(this.turn(&j).await) })
        };
        let (method, cues) = subx::extract(&file, &index, &wants, &pace).await?;
        for w in &wants {
            let srt = normalize_srt(&subx::to_srt(cues.get(&w.stream).map_or(&[][..], Vec::as_slice)));
            write_atomic(&dir.join(format!("s{}.srt", w.stream)), srt.as_bytes()).await?;
        }
        Ok(match method {
            subx::Method::Cues => "index",
            subx::Method::Scan => "scan",
        })
    }

    /// One ffmpeg run writing every listed stream; if it fails, each stream on its own (so one
    /// odd track doesn't cost the others). Paused (SIGSTOP) while a more urgent job reads: its
    /// reads through the loopback listener hold nothing others need then (`Smb::read_at`).
    async fn extract_ffmpeg(&self, job: &Arc<Job>, todo: &[i64], dir: &Path) -> Result<usize> {
        // A whole read of the file.
        job.cost.store(2, Ordering::SeqCst);
        self.changed.notify_waiters();
        match self.run_ffmpeg(job, todo, dir).await {
            Ok(()) => Ok(todo.len()),
            Err(e) if todo.len() > 1 => {
                tracing::info!(item = %job.media.item_id, "one-pass extraction failed ({e:#}): one track at a time");
                let mut n = 0;
                let mut last = None;
                for s in todo {
                    match self.run_ffmpeg(job, &[*s], dir).await {
                        Ok(()) => n += 1,
                        Err(e) => last = Some(e),
                    }
                }
                match (n, last) {
                    (0, Some(e)) => Err(e),
                    _ => Ok(n),
                }
            }
            Err(e) => Err(e),
        }
    }

    async fn run_ffmpeg(&self, job: &Arc<Job>, streams: &[i64], dir: &Path) -> Result<()> {
        let tag = format!("{}-{}", std::process::id(), rand::random::<u32>());
        let part = |s: i64| dir.join(format!("s{s}.srt.{tag}.part"));
        let mut args: Vec<String> = ["-nostdin", "-hide_banner", "-v", "error"].map(String::from).to_vec();
        args.extend(job.media.source.input_args());
        args.push("-copyts".into());
        for &s in streams {
            args.extend([
                "-map".into(),
                format!("0:{s}"),
                "-c:s".into(),
                "srt".into(),
                "-f".into(),
                "srt".into(),
            ]);
            args.push(part(s).to_string_lossy().into_owned());
        }
        let child = tokio::process::Command::new("nice")
            .args(["-n", "15", &self.ffmpeg])
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("ffmpeg")?;
        let pid = child.id();
        let wait = child.wait_with_output();
        tokio::pin!(wait);
        let mut stopped = false;
        let out = loop {
            tokio::select! {
                o = &mut wait => break o.context("ffmpeg")?,
                () = tokio::time::sleep(Duration::from_millis(100)) => {
                    // Paused while another job's turn, and while a stream segment or a
                    // subtitle window is read.
                    let mine = self.top().as_deref() == Some(job.key.as_str())
                        && STREAM_READS.load(Ordering::SeqCst) == 0
                        && WINDOW_READS.load(Ordering::SeqCst) == 0;
                    if let Some(pid) = pid {
                        if mine == stopped {
                            // SAFETY: kill only takes integers. `pid` is our ffmpeg child, which isn't reaped before
                            // `wait` completes (and that ends this loop), so the id is still its.
                            unsafe { libc::kill(pid as i32, if mine { libc::SIGCONT } else { libc::SIGSTOP }); }
                            stopped = !mine;
                        }
                    }
                }
            }
        };
        let result = if out.status.success() {
            let mut r = Ok(());
            for &s in streams {
                let p = part(s);
                match tokio::fs::read(&p).await {
                    Ok(b) => {
                        let srt = normalize_srt(&String::from_utf8_lossy(&b));
                        write_atomic(&dir.join(format!("s{s}.srt")), srt.as_bytes()).await?;
                    }
                    Err(e) => r = Err(anyhow::anyhow!("stream {s}: no output ({e})")),
                }
            }
            r
        } else {
            Err(anyhow::anyhow!(
                "ffmpeg: {}",
                crate::admin::logbuf::scrub(&String::from_utf8_lossy(&out.stderr).lines().take(3).collect::<Vec<_>>().join(" / "))
            ))
        };
        for &s in streams {
            let _ = tokio::fs::remove_file(part(s)).await;
        }
        result
    }

    /// The backfill's step for one title: its embedded text tracks extracted (in the
    /// background) and the file marked done. Ok(true): something was read.
    pub async fn backfill_one(self: &Arc<Self>, media: &Arc<Media>) -> Result<bool> {
        let dir = self.extract_dir(media);
        let streams = Self::streams_of(media);
        let missing = streams.iter().any(|s| !dir.join(format!("s{s}.srt")).is_file());
        if missing {
            self.ensure_extracted(media, Demand::Background).await?;
        }
        if !dir.join("done.json").is_file() {
            tokio::fs::create_dir_all(&dir).await?;
            let done = Done {
                streams,
                method: "none".into(),
                secs: 0.0,
                at: now_secs(),
            };
            write_atomic(&dir.join("done.json"), &serde_json::to_vec(&done)?).await?;
        }
        Ok(missing)
    }

    /// Remembers a failure for the backfill (it tries again after `RETRY_FAILED`).
    pub async fn mark_failed(&self, file_key: &str, error: &str) {
        let dir = self.dir_for_key(file_key);
        let _ = tokio::fs::create_dir_all(&dir).await;
        let f = Failed {
            error: error.to_string(),
            at: now_secs(),
        };
        let _ = write_atomic(&dir.join("failed.json"), &serde_json::to_vec(&f).unwrap_or_default()).await;
    }

    /// Window `k` of track `index`: the cues showing during [k·WINDOW, (k+1)·WINDOW). From the
    /// extracted track when it is there (`complete`), else read for this window alone: by the
    /// index (a few small reads), by scanning that stretch of the file, from an MP4's sample
    /// tables, or by ffmpeg seeking there. Extraction waits while windows are read.
    pub async fn window(self: &Arc<Self>, media: &Arc<Media>, index: i64, k: usize) -> Result<Window> {
        let track = media
            .subtitles
            .iter()
            .find(|s| s.index == index && s.text)
            .context("no such text subtitle track")?;
        let (from, to) = (k as f64 * WINDOW, (k + 1) as f64 * WINDOW);
        let done = |cues: Vec<subx::Cue>, complete: bool, source: &'static str| Window {
            index,
            window: k,
            from,
            to,
            span: WINDOW,
            complete,
            source,
            cues: Arc::new(cues),
        };
        let stream = match track.stream {
            Some(s) if !self.ready(media, index) => s,
            // Extracted, a sidecar or a fetched file: the whole track is at hand.
            _ => {
                let srt = self.srt(media, index).await?;
                return Ok(done(subx::overlapping(parse_srt(&srt), from, to), true, "file"));
            }
        };
        anyhow::ensure!(self.extract, "subtitle extraction is switched off");
        if media.origin <= 0.0 {
            let (cues, source) = self.file_window(media, index, stream, k).await?;
            return Ok(Window {
                cues,
                ..done(Vec::new(), false, source)
            });
        }
        // The track's cues are on the file's own clock, `origin` past the timeline: the file's
        // windows this one falls in, moved back by it.
        let first = ((from + media.origin) / WINDOW).floor() as usize;
        let last = ((to + media.origin) / WINDOW).floor() as usize;
        let mut cues: Vec<subx::Cue> = Vec::new();
        let mut source = "memory";
        for w in first..=last {
            let (got, s) = self.file_window(media, index, stream, w).await?;
            if s != "memory" {
                source = s;
            }
            for c in got.iter() {
                if !cues.contains(c) {
                    cues.push(c.clone());
                }
            }
        }
        Ok(done(subx::overlapping(nudged(cues, -media.origin), from, to), false, source))
    }

    /// Window `k` of an embedded track that isn't extracted yet, on the file's own clock, and
    /// where it came from (`Window::source`).
    async fn file_window(
        self: &Arc<Self>,
        media: &Arc<Media>,
        index: i64,
        stream: i64,
        k: usize,
    ) -> Result<(Arc<Vec<subx::Cue>>, &'static str)> {
        if let Some(c) = self.cached_window(&media.file_key, stream, k) {
            return Ok((c, "memory"));
        }
        // Read it (or wait for the same read by someone else). Extraction holds its reads
        // meanwhile.
        let _reading = WindowRead::start();
        let t0 = Instant::now();
        let fi = self.file_index(media).await;
        if t0.elapsed() > Duration::from_millis(500) {
            tracing::info!(item = %media.item_id, ms = t0.elapsed().as_millis() as u64, ok = fi.is_ok(), "subtitle window: index read slowly");
        }
        let scan = matches!(&fi, Ok(FileIndex::Mkv { index: ix, .. }) if subx::wants(ix, &[stream]).is_some_and(|w| subx::method(ix, &w) == subx::Method::Scan));
        let key = if scan {
            format!("{}|*|{k}", media.file_key)
        } else {
            format!("{}|{stream}|{k}", media.file_key)
        };
        let cell = self.window_inflight.lock().unwrap().entry(key.clone()).or_default().clone();
        let source = cell
            .get_or_init(|| async {
                let t0 = Instant::now();
                let r = self.read_window(media, stream, k, fi).await;
                match &r {
                    Ok((source, bytes)) => tracing::info!(item = %media.item_id, index, window = k, source, bytes, ms = t0.elapsed().as_millis() as u64, "subtitle window read"),
                    Err(e) => tracing::warn!(item = %media.item_id, index, window = k, "subtitle window: {e:#}"),
                }
                r.map(|(s, _)| s).map_err(|e| format!("{e:#}"))
            })
            .await
            .clone();
        self.window_inflight.lock().unwrap().remove(&key);
        let source = source.map_err(anyhow::Error::msg)?;
        let cues = self
            .cached_window(&media.file_key, stream, k)
            .context("window missing after the read")?;
        Ok((cues, source))
    }

    fn cached_window(&self, file_key: &str, stream: i64, k: usize) -> Option<Arc<Vec<subx::Cue>>> {
        self.windows
            .lock()
            .unwrap()
            .get(&(file_key.to_string(), stream, k))
            .map(|(c, _)| c.clone())
    }

    fn keep_window(&self, file_key: &str, stream: i64, k: usize, cues: Vec<subx::Cue>) {
        let mut w = self.windows.lock().unwrap();
        if w.len() >= WINDOWS_KEPT {
            // Drop the oldest quarter.
            let mut ages: Vec<Instant> = w.values().map(|(_, t)| *t).collect();
            ages.sort_unstable();
            let cut = ages[ages.len() / 4];
            w.retain(|_, (_, t)| *t > cut);
        }
        w.insert((file_key.to_string(), stream, k), (Arc::new(cues), Instant::now()));
    }

    /// The file's index for windows (read once, kept): Matroska's Cues and tracks, or an MP4's
    /// timed-text sample tables.
    pub(crate) async fn file_index(&self, media: &Media) -> Result<FileIndex> {
        let cached = || {
            self.indexes.lock().unwrap().get(&media.file_key).map(|(fi, _)| match fi.as_ref() {
                FileIndex::Mkv { index, file, header } => FileIndex::Mkv {
                    index: index.clone(),
                    file: file.clone(),
                    header: *header,
                },
                FileIndex::Mp4 { tracks, file } => FileIndex::Mp4 {
                    tracks: tracks.clone(),
                    file: file.clone(),
                },
            })
        };
        if let Some(fi) = cached() {
            return Ok(fi);
        }
        // One read for everyone asking at once (a window and the extraction, at a play's start).
        let cell = self
            .index_inflight
            .lock()
            .unwrap()
            .entry(media.file_key.clone())
            .or_default()
            .clone();
        let r = cell
            .get_or_init(|| async { self.read_index(media).await.map_err(|e| format!("{e:#}")) })
            .await
            .clone();
        self.index_inflight.lock().unwrap().remove(&media.file_key);
        r.map_err(anyhow::Error::msg)?;
        cached().context("index missing after the read")
    }

    async fn read_index(&self, media: &Media) -> Result<()> {
        let path = media.source.path();
        let path = &path;
        let fi = if media.container.contains("matroska") || media.container.contains("webm") {
            let file = subx::File::open_async(path, true).await?;
            let index = mkv::index_with(&file).await?;
            FileIndex::Mkv {
                index: Arc::new(index),
                file,
                header: None,
            }
        } else if media.container.contains("mp4") || media.container.contains("mov") {
            let file = subx::File::open_async(path, true).await?;
            let moov = moov::read_moov_with(&file).await?;
            let (handlers, tracks) = moov::text_tracks(&moov)?;
            // ffmpeg numbers the streams in `trak` order: check that against what it said.
            let kind_ok = |stream: i64, want: &[&str]| handlers.get(stream as usize).is_some_and(|h| want.contains(&h.as_str()));
            let ok = media.video.as_ref().is_none_or(|v| kind_ok(v.stream, &["vide"]))
                && media.audio.iter().all(|a| kind_ok(a.stream, &["soun"]))
                && media
                    .subtitles
                    .iter()
                    .filter_map(|s| s.stream.filter(|_| s.text))
                    .all(|s| kind_ok(s, &["text", "sbtl", "subt"]));
            anyhow::ensure!(ok, "the MP4's tracks don't line up with ffmpeg's streams");
            FileIndex::Mp4 {
                tracks: Arc::new(tracks),
                file,
            }
        } else {
            bail!("no index for {}", media.container);
        };
        self.store_index(&media.file_key, &fi);
        Ok(())
    }

    pub(crate) fn store_index(&self, file_key: &str, fi: &FileIndex) {
        let copy = match fi {
            FileIndex::Mkv { index, file, header } => FileIndex::Mkv {
                index: index.clone(),
                file: file.clone(),
                header: *header,
            },
            FileIndex::Mp4 { tracks, file } => FileIndex::Mp4 {
                tracks: tracks.clone(),
                file: file.clone(),
            },
        };
        let mut m = self.indexes.lock().unwrap();
        if m.len() >= INDEXES_KEPT {
            if let Some(oldest) = m.iter().min_by_key(|(_, (_, t))| *t).map(|(k, _)| k.clone()) {
                m.remove(&oldest);
            }
        }
        m.insert(file_key.to_string(), (Arc::new(copy), Instant::now()));
    }

    /// Reads window `k` of `stream` (and, scanning, of every text track of the file) into the
    /// window cache. Returns how, and how many bytes were read.
    async fn read_window(&self, media: &Media, stream: i64, k: usize, fi: Result<FileIndex>) -> Result<(&'static str, u64)> {
        let (from, to) = (k as f64 * WINDOW, (k + 1) as f64 * WINDOW);
        match fi {
            Ok(FileIndex::Mkv { index, file, header }) => {
                if let Some(wants) = subx::wants(&index, &[stream]) {
                    if subx::method(&index, &wants) == subx::Method::Cues {
                        let (cues, h) = subx::window_by_cues(&file, &index, &wants[0], from, to, WINDOW_LEAD, header).await?;
                        if header.is_none() {
                            self.store_index(
                                &media.file_key,
                                &FileIndex::Mkv {
                                    index: index.clone(),
                                    file,
                                    header: Some(h),
                                },
                            );
                        }
                        self.keep_window(&media.file_key, stream, k, cues);
                        return Ok(("index", 0));
                    }
                    // Every text track at once: the bytes are read either way.
                    let all = Self::streams_of(media);
                    let wants = subx::wants(&index, &all).unwrap_or(wants);
                    let _turn = self.scan_gate.acquire().await?;
                    // The window before is known (playing on): the cues still showing from it
                    // come from there, so no lead is read.
                    let before: Option<Vec<Arc<Vec<subx::Cue>>>> = if k == 0 {
                        None
                    } else {
                        wants.iter().map(|w| self.cached_window(&media.file_key, w.stream, k - 1)).collect()
                    };
                    let lead = if before.is_some() { 0.0 } else { SCAN_LEAD };
                    let (cues, bytes) = subx::window_by_scan(&file, &index, &wants, from, to, lead, 2, &yield_to_streams()).await?;
                    for (i, w) in wants.iter().enumerate() {
                        let mut c = cues.get(&w.stream).cloned().unwrap_or_default();
                        if let Some(b) = &before {
                            for x in b[i].iter().filter(|x| x.end > from) {
                                if !c.contains(x) {
                                    c.push(x.clone());
                                }
                            }
                            c.sort_by(|a, b| a.start.total_cmp(&b.start));
                        }
                        self.keep_window(&media.file_key, w.stream, k, c);
                    }
                    return Ok(("scan", bytes));
                }
                tracing::info!(item = %media.item_id, stream, "subtitle window: track not readable directly, ffmpeg instead");
            }
            Ok(FileIndex::Mp4 { tracks, file }) => {
                if let Some(t) = tracks.iter().find(|t| t.order as i64 == stream) {
                    let samples: Vec<&moov::TextSample> = t
                        .samples
                        .iter()
                        .filter(|s| s.size > 2 && s.time < to && s.time + s.duration > from)
                        .collect();
                    let spans: Vec<(u64, u64)> = samples.iter().map(|s| (s.offset, s.size as u64)).collect();
                    let bytes: u64 = spans.iter().map(|s| s.1).sum();
                    let data = subx::read_spans(&file, &spans, 8).await?;
                    let cues = samples
                        .iter()
                        .zip(data)
                        .filter_map(|(s, d)| {
                            moov::tx3g_text(&d).map(|text| subx::Cue {
                                start: s.time,
                                end: s.time + s.duration,
                                text,
                            })
                        })
                        .collect();
                    self.keep_window(&media.file_key, stream, k, cues);
                    return Ok(("mp4", bytes));
                }
                tracing::info!(item = %media.item_id, stream, "subtitle window: not a timed-text track, ffmpeg instead");
            }
            Err(e) => tracing::info!(item = %media.item_id, "subtitle window: no index ({e:#}), ffmpeg instead"),
        }
        let cues = self.window_ffmpeg(media, stream, from, to).await?;
        self.keep_window(&media.file_key, stream, k, cues);
        Ok(("ffmpeg", 0))
    }

    /// One window by ffmpeg: an input seek to a little before it, read for its length.
    async fn window_ffmpeg(&self, media: &Media, stream: i64, from: f64, to: f64) -> Result<Vec<subx::Cue>> {
        let start = (from - WINDOW_LEAD).max(0.0);
        let mut args: Vec<String> = [
            "-nostdin",
            "-hide_banner",
            "-v",
            "error",
            "-ss",
            &format!("{start:.3}"),
            "-t",
            &format!("{:.3}", to - start + 2.0),
        ]
        .map(String::from)
        .to_vec();
        args.extend(media.source.input_args());
        args.extend([
            "-copyts".into(),
            "-map".into(),
            format!("0:{stream}"),
            "-c:s".into(),
            "srt".into(),
            "-f".into(),
            "srt".into(),
            "pipe:1".into(),
        ]);
        let out = tokio::process::Command::new(&self.ffmpeg)
            .args(&args)
            .kill_on_drop(true)
            .output()
            .await
            .context("ffmpeg")?;
        if !out.status.success() {
            bail!(
                "ffmpeg: {}",
                crate::admin::logbuf::scrub(&String::from_utf8_lossy(&out.stderr).lines().take(3).collect::<Vec<_>>().join(" / "))
            );
        }
        Ok(subx::overlapping(parse_srt(&String::from_utf8_lossy(&out.stdout)), from, to))
    }

    /// What `GET /v1/jobs` shows: the extractions queued or running.
    pub fn status(&self) -> serde_json::Value {
        let top = self.top();
        let jobs: Vec<serde_json::Value> = self
            .jobs
            .lock()
            .unwrap()
            .values()
            .map(|j| {
                serde_json::json!({
                    "item": j.media.item_id,
                    "urgent": self.urgent(j),
                    "waiters": j.waiters.load(Ordering::SeqCst),
                    "started": j.running.load(Ordering::SeqCst),
                    "reading": top.as_deref() == Some(j.key.as_str()),
                })
            })
            .collect();
        serde_json::json!({ "extractions": jobs, "backfill": *self.backfill.lock().unwrap(), "timing_matches": self.sync.running_count() })
    }

    async fn sidecar(&self, media: &Media, track: &SubtitleTrack, path: &str) -> Result<String> {
        let meta = crate::storage::vfs::stat(path).await?;
        let mtime = meta.mtime / 1_000_000_000;
        let cached = self.extract_dir(media).join(format!(
            "x{}-{:016x}.srt",
            track.index,
            fnv(&format!("{path}|{}|{mtime}", meta.size))
        ));
        if let Ok(b) = tokio::fs::read(&cached).await {
            return Ok(String::from_utf8_lossy(&b).into_owned());
        }
        let raw = crate::storage::vfs::read(path).await.with_context(|| format!("read {path}"))?;
        let text = decode_text(&raw, track.language.as_deref());
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let srt = if ext == "srt" {
            normalize_srt(&text)
        } else {
            // ASS/SSA/WebVTT/SUB…: ffmpeg converts the decoded (UTF-8) text.
            let tmp = std::env::temp_dir().join(format!("moonrise-sub-{}-{}.{ext}", std::process::id(), rand::random::<u32>()));
            tokio::fs::write(&tmp, text.as_bytes()).await?;
            let out = tokio::process::Command::new(&self.ffmpeg)
                .args(["-nostdin", "-hide_banner", "-v", "error", "-i"])
                .arg(&tmp)
                .args(["-map", "0:s:0", "-c:s", "srt", "-f", "srt", "pipe:1"])
                .kill_on_drop(true)
                .output()
                .await;
            let _ = tokio::fs::remove_file(&tmp).await;
            let out = out.context("ffmpeg")?;
            if !out.status.success() {
                bail!(
                    "convert {ext}: {}",
                    crate::admin::logbuf::scrub(String::from_utf8_lossy(&out.stderr).trim())
                );
            }
            normalize_srt(&String::from_utf8_lossy(&out.stdout))
        };
        if let Some(d) = cached.parent() {
            tokio::fs::create_dir_all(d).await?;
        }
        write_atomic(&cached, srt.as_bytes()).await?;
        Ok(srt)
    }
}

struct WaitGuard<'a> {
    job: Arc<Job>,
    subs: &'a Subs,
}

impl Drop for WaitGuard<'_> {
    fn drop(&mut self) {
        self.job.waiters.fetch_sub(1, Ordering::SeqCst);
        self.subs.changed.notify_waiters();
    }
}

/// The title has embedded text tracks (extracted here).
pub fn has_embedded_text(media: &Media) -> bool {
    media.subtitles.iter().any(|s| s.text && s.stream.is_some())
}

/// The fetched tracks listed for a title.
pub fn read_extras_for(subs: &Subs, media: &Media) -> Vec<Extra> {
    read_extras(&subs.extras_dir(media))
}

fn read_extras(dir: &Path) -> Vec<Extra> {
    std::fs::read(dir.join("extras.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub(crate) async fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("tmp{}", rand::random::<u32>()));
    tokio::fs::write(&tmp, data)
        .await
        .with_context(|| format!("write {}", tmp.display()))?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

fn sanitize(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect()
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Subtitle bytes as text: UTF-8 (with or without BOM), UTF-16 with a BOM, else a legacy code
/// page: Windows-1251 for Cyrillic languages or text that looks Cyrillic, Windows-1252 otherwise.
pub fn decode_text(b: &[u8], language: Option<&str>) -> String {
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if b.starts_with(&[0xFF, 0xFE]) {
        return encoding_rs::UTF_16LE.decode_without_bom_handling(&b[2..]).0.into_owned();
    }
    if b.starts_with(&[0xFE, 0xFF]) {
        return encoding_rs::UTF_16BE.decode_without_bom_handling(&b[2..]).0.into_owned();
    }
    if let Ok(s) = std::str::from_utf8(b) {
        return s.to_string();
    }
    let cyrillic_lang = language.is_some_and(|l| {
        let l = l.to_ascii_lowercase();
        [
            "ru", "rus", "uk", "ukr", "be", "bel", "bg", "bul", "sr", "srp", "mk", "mac", "mkd", "kk", "kaz",
        ]
        .contains(&l.as_str())
    });
    // In Windows-1251 Cyrillic text most bytes ≥ 0x80 are letters (0xC0-0xFF); Western text has
    // few high bytes at all.
    let high = b.iter().filter(|&&c| c >= 0x80).count();
    let letters = b.iter().filter(|&&c| c >= 0xC0).count();
    let looks_cyrillic = high > 0 && letters * 10 >= high * 7 && high * 5 >= b.iter().filter(|c| c.is_ascii_alphabetic()).count();
    let enc = if cyrillic_lang || looks_cyrillic {
        encoding_rs::WINDOWS_1251
    } else {
        encoding_rs::WINDOWS_1252
    };
    enc.decode_without_bom_handling(b).0.into_owned()
}

/// Cues moved by the viewer's nudge (those it takes before the start are gone).
/// An embedded track's cues are the file's own timestamps; the title's timeline starts at the file's
/// start time (`Media::origin`), so they are served that much earlier. Sidecar and fetched tracks
/// are timed from the title's start already.
fn on_timeline(media: &Media, track: &SubtitleTrack, srt: String) -> String {
    if track.stream.is_none() || media.origin <= 0.0 {
        return srt;
    }
    subx::to_srt(&nudged(parse_srt(&srt), -media.origin))
}

fn nudged(mut cues: Vec<subx::Cue>, nudge: f64) -> Vec<subx::Cue> {
    for c in &mut cues {
        c.start = (c.start + nudge).max(0.0);
        c.end += nudge;
    }
    cues.retain(|c| c.end > 0.0);
    cues
}

/// SRT with `\n` line ends, no BOM, cues renumbered, and blank or malformed blocks dropped.
pub fn normalize_srt(text: &str) -> String {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(text.len());
    let mut n = 0;
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
        let Some(ti) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let body: Vec<&str> = lines[ti + 1..].to_vec();
        if body.is_empty() || parse_timing(lines[ti]).is_none() {
            continue;
        }
        n += 1;
        let _ = write!(out, "{n}\n{}\n{}\n\n", lines[ti].trim(), body.join("\n"));
    }
    out
}

/// SRT as cues (text lines as they are).
pub fn parse_srt(srt: &str) -> Vec<subx::Cue> {
    let text = srt.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        let Some(ti) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let Some((a, b)) = parse_timing(lines[ti]) else { continue };
        let body = lines[ti + 1..]
            .iter()
            .filter(|l| !l.trim().is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("\n");
        if !body.is_empty() {
            out.push(subx::Cue {
                start: a,
                end: b,
                text: body,
            });
        }
    }
    out.sort_by(|x, y| x.start.total_cmp(&y.start));
    out
}

/// `00:01:02,345 --> 00:01:04,000` (or with dots, or without hours) → seconds.
pub fn parse_timing(line: &str) -> Option<(f64, f64)> {
    let (a, b) = line.split_once("-->")?;
    let b = b.split_whitespace().next()?;
    Some((parse_ts(a.trim())?, parse_ts(b.trim())?))
}

fn parse_ts(s: &str) -> Option<f64> {
    let s = s.replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    let (h, m, sec) = match parts.as_slice() {
        [h, m, s] => (h.parse::<f64>().ok()?, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
        [m, s] => (0.0, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
        _ => return None,
    };
    Some(h * 3600.0 + m * 60.0 + sec)
}

fn vtt_ts(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// The start of a WebVTT file for an HLS rendition: cue times are on the title's timeline, whose 0
/// is the video's timestamp `origin` (`Media::origin`; see the module docs).
fn vtt_header(origin: f64) -> String {
    format!(
        "WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:{},LOCAL:00:00:00.000\n\n",
        (origin.max(0.0) * 90_000.0).round() as u64
    )
}

/// WebVTT for an HLS rendition: cue times as they are (on the title's timeline, which starts at
/// the video's timestamp `origin`), ASS override tags and `<font>` tags removed.
pub fn srt_to_vtt(srt: &str, origin: f64) -> String {
    let mut out = vtt_header(origin);
    for block in srt.split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        let Some(ti) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let Some((a, b)) = parse_timing(lines[ti]) else { continue };
        let text: Vec<String> = lines[ti + 1..]
            .iter()
            .map(|l| clean_cue_line(l))
            .filter(|l| !l.trim().is_empty())
            .collect();
        if text.is_empty() {
            continue;
        }
        let _ = write!(out, "{} --> {}\n{}\n\n", vtt_ts(a), vtt_ts(b), text.join("\n"));
    }
    out
}

/// Cues as WebVTT (one segment of a rendition).
pub fn cues_to_vtt(cues: &[subx::Cue], origin: f64) -> String {
    let mut out = vtt_header(origin);
    for c in cues {
        let text: Vec<String> = c.text.lines().map(clean_cue_line).filter(|l| !l.trim().is_empty()).collect();
        if text.is_empty() {
            continue;
        }
        let _ = write!(out, "{} --> {}\n{}\n\n", vtt_ts(c.start), vtt_ts(c.end), text.join("\n"));
    }
    out
}

fn clean_cue_line(l: &str) -> String {
    let mut s = String::with_capacity(l.len());
    let mut chars = l.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' && chars.peek() == Some(&'\\') {
            for d in chars.by_ref() {
                if d == '}' {
                    break;
                }
            }
            continue;
        }
        s.push(c);
    }
    // <font …> and </font>: not WebVTT.
    while let Some(i) = s.to_ascii_lowercase().find("<font") {
        match s[i..].find('>') {
            Some(j) => s.replace_range(i..=(i + j), ""),
            None => break,
        }
    }
    s.replace("</font>", "").replace("</FONT>", "").replace("-->", "→")
}

/// English name of a language code (ISO 639-1 or -2), for track names.
pub fn language_name(code: &str) -> String {
    let c = code.split(['-', '_']).next().unwrap_or(code).to_ascii_lowercase();
    let names = [
        (["en", "eng"], "English"),
        (["ru", "rus"], "Russian"),
        (["uk", "ukr"], "Ukrainian"),
        (["es", "spa"], "Spanish"),
        (["fr", "fre"], "French"),
        (["de", "ger"], "German"),
        (["it", "ita"], "Italian"),
        (["pt", "por"], "Portuguese"),
        (["nl", "dut"], "Dutch"),
        (["pl", "pol"], "Polish"),
        (["sv", "swe"], "Swedish"),
        (["ja", "jpn"], "Japanese"),
        (["ko", "kor"], "Korean"),
        (["zh", "chi"], "Chinese"),
        (["ar", "ara"], "Arabic"),
        (["he", "heb"], "Hebrew"),
        (["tr", "tur"], "Turkish"),
        (["el", "gre"], "Greek"),
        (["cs", "cze"], "Czech"),
        (["fi", "fin"], "Finnish"),
        (["da", "dan"], "Danish"),
        (["no", "nor"], "Norwegian"),
        (["hu", "hun"], "Hungarian"),
        (["ro", "rum"], "Romanian"),
        (["bg", "bul"], "Bulgarian"),
        (["hr", "hrv"], "Croatian"),
        (["sr", "srp"], "Serbian"),
        (["hi", "hin"], "Hindi"),
    ];
    names
        .iter()
        .find(|(k, _)| k.contains(&c.as_str()))
        .map_or_else(|| if c.is_empty() { "Unknown".into() } else { c }, |(_, n)| n.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_cues_are_served_on_the_titles_timeline() {
        // A file whose clock starts at 1001: players count that as 0, so an embedded cue at
        // the file's 1011 shows at 0:10. A sidecar is timed from the title's start already.
        let mut m = crate::media::transcode::tests::media(crate::media::transcode::tests::video("h264", 1920, 1080, 4_000_000, "SDR"));
        m.origin = 1001.0;
        let track = |stream: Option<i64>| SubtitleTrack {
            index: 2,
            stream,
            text: true,
            codec: "subrip".into(),
            name: String::new(),
            language: None,
            forced: false,
            default: false,
            external: stream.is_none(),
            source: "",
            url: None,
            path: None,
            file: None,
            timing: None,
        };
        let srt = "1\n00:16:51,000 --> 00:16:53,000\nCue at ten\n\n".to_string();
        let embedded = on_timeline(&m, &track(Some(2)), srt.clone());
        assert_eq!(parse_srt(&embedded)[0].start, 10.0, "{embedded}");
        assert_eq!(on_timeline(&m, &track(None), srt.clone()), srt);
        // The rendition maps 0:00 to the video's timestamp 1001 (the segments keep the file's own).
        assert!(srt_to_vtt(&embedded, m.origin).starts_with("WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:90090000,LOCAL:00:00:00.000\n"));
        // A file starting at 0: as before.
        m.origin = 0.0;
        assert_eq!(on_timeline(&m, &track(Some(2)), srt.clone()), srt);
        assert!(srt_to_vtt(&srt, 0.0).starts_with("WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:0,LOCAL:00:00:00.000\n"));
    }

    #[test]
    fn srt_normalised_and_converted() {
        let srt = "\u{feff}1\r\n00:01:41,519 --> 00:01:43,479\r\n{\\an8}<font color=\"#ff0\">Anybody</font> home?\r\n\r\n\r\n7\r\n00:01:44,688 --> 00:01:46,398 X1:0\r\n<i>Hello?</i>\r\n\r\nbroken\r\n\r\n3\r\n00:02:00,000 --> 00:02:01,000\r\n\r\n";
        let n = normalize_srt(srt);
        assert_eq!(n, "1\n00:01:41,519 --> 00:01:43,479\n{\\an8}<font color=\"#ff0\">Anybody</font> home?\n\n2\n00:01:44,688 --> 00:01:46,398 X1:0\n<i>Hello?</i>\n\n");
        let v = srt_to_vtt(&n, 0.0);
        assert_eq!(
            v,
            "WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:0,LOCAL:00:00:00.000\n\n00:01:41.519 --> 00:01:43.479\nAnybody home?\n\n00:01:44.688 --> 00:01:46.398\n<i>Hello?</i>\n\n"
        );
        assert_eq!(parse_timing("1:02:03.5 --> 1:02:04,25"), Some((3723.5, 3724.25)));
        // A segment [100, 102): only the first cue (it ends at 103.479), and one that started
        // before it but still shows.
        let range = |from, to| cues_to_vtt(&subx::overlapping(parse_srt(&n), from, to), 0.0);
        let r = range(100.0, 102.0);
        assert!(r.contains("00:01:41.519") && !r.contains("00:01:44.688"));
        assert!(r.contains("\nAnybody home?\n"), "{r}");
        assert!(range(103.0, 104.0).contains("00:01:41.519"));
        assert!(range(110.0, 140.0).ends_with("LOCAL:00:00:00.000\n\n"));
        assert_eq!(parse_srt(&n).len(), 2);
        assert_eq!(parse_srt(&n)[1].text, "<i>Hello?</i>");
        assert_eq!(vtt_ts(3723.5), "01:02:03.500");
    }

    #[test]
    fn nudges_move_cues_and_drop_those_before_the_start() {
        let cue = |s: f64, e: f64| subx::Cue {
            start: s,
            end: e,
            text: "x".into(),
        };
        let out = nudged(vec![cue(1.0, 2.0), cue(10.0, 12.0)], -1.5);
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].start, out[0].end), (0.0, 0.5));
        assert_eq!((out[1].start, out[1].end), (8.5, 10.5));
        assert_eq!(nudged(vec![cue(1.0, 2.0)], -3.0).len(), 0);
    }

    #[test]
    fn charsets() {
        assert_eq!(decode_text("Привет".as_bytes(), None), "Привет");
        let (cp1251, _, _) = encoding_rs::WINDOWS_1251.encode("Привет, как дела?");
        assert_eq!(decode_text(&cp1251, None), "Привет, как дела?");
        assert_eq!(decode_text(&cp1251, Some("rus")), "Привет, как дела?");
        let (cp1252, _, _) = encoding_rs::WINDOWS_1252.encode("Ça va très bien, merci beaucoup");
        assert_eq!(decode_text(&cp1252, Some("fre")), "Ça va très bien, merci beaucoup");
        let mut u16 = vec![0xFF, 0xFE];
        u16.extend("Hi".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(decode_text(&u16, None), "Hi");
    }

    #[test]
    fn names() {
        assert_eq!(language_name("en"), "English");
        assert_eq!(language_name("pt-BR"), "Portuguese");
        assert_eq!(language_name("rus"), "Russian");
        assert_eq!(language_name("xx"), "xx");
    }
}
