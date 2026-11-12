//! Scrubbing thumbnails ("trickplay"), made ahead of need (the `trickplay` switch, on by default).
//!
//! **What is made, per file:** one picture every `INTERVAL` seconds, `WIDTH` pixels wide, kept as
//! JPEG sheets of `COLUMNS` × `ROWS` tiles (`{n}.jpg`, for the apps' own scrubbers) and as a
//! tiny H.264 track of the same pictures, every one a keyframe (`iframes.mp4`), which the master
//! playlist offers as an `EXT-X-I-FRAME-STREAM-INF` stream to a player that asks for it
//! (`trick=1`): the Apple TV's system player shows its scrub preview from it. `index.json` says
//! how the sheets are laid out and where each picture of the track is. They live in
//! `<cache>/trickplay/<file key>/` (the file's path, size and mtime: a changed file gets new
//! ones), apart from the segment cache, within their own budget (`performance.trickplay_max_gb`).
//!
//! **How:** each picture is one keyframe, decoded alone, so a file is read only at its pictures
//! (a keyframe every 10 s), never whole. `nice`d, a share read at `RATE`, one picture after the
//! other.
//! - MP4/MOV with H.264 or HEVC: the keyframes' places come from the file's own index (`moov`,
//!   read once), and each picture's keyframe sample alone is read and decoded (`Keys`). Letting
//!   ffmpeg seek in the file instead read more than the whole file (it reads the index again
//!   for every picture).
//! - Other files: found by an ffmpeg seek (`-ss`), the keyframe's packet copied out and decoded.
//! - AVI's MPEG-4 (packets that can't be copied): one paced pass over the whole file.
//! - Dolby Vision without a compatible base layer (profile 5): skipped. Its pictures only have
//!   the right colours with the Dolby Vision metadata applied, which this ffmpeg can't do.
//!
//! **When:** like `index_backfill.rs`: one file at a time, the titles being watched (and what is
//! opened in a player, `hint`) first, then the newest. Nothing runs while anything plays or is
//! read for playback (`index_backfill::busy`): it waits before every picture, and a longer
//! ffmpeg run (the AVI pass, the sheets, the I-frame track) is stopped (SIGSTOP) until that
//! ends. Sets of files that are gone or changed are deleted. No database: a folder with an
//! `index.json` is a finished set.
//!
//! **Budget:** when the sets fill `performance.trickplay_max_gb`, the backfill stops ("full").
//! A title opened in a player (`hint`) is still made: the sets used longest ago (their
//! `index.json`'s mtime, set when a player asks for them) are deleted to make room.

use std::fmt::Write as _;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Notify;

use crate::{admin::config::Feature, api::AppState, storage::source::Source, storage::vfs};

/// Seconds between pictures.
pub const INTERVAL: f64 = 10.0;
/// Picture width (the height follows the display aspect).
pub const WIDTH: u32 = 320;
pub const COLUMNS: u32 = 10;
pub const ROWS: u32 = 10;
/// Bump when what is made changes: older sets are made again.
pub const FORMAT: u32 = 1;
/// At most this many pictures (a longer file gets them further apart).
const MAX_COUNT: u32 = 2000;
/// How fast a file on a share is read for this (bytes a second).
const RATE: u64 = 8 << 20;
/// One picture taking longer than this is given up (it is copied from the one before).
const PICTURE_TIMEOUT: Duration = Duration::from_secs(60);
/// A file whose set couldn't be made is tried again after this long.
const RETRY_FAILED: Duration = Duration::from_hours(7 * 24);
/// A set's last use is written at most this often (`Trickplay::index`).
const USE_GRANULARITY: Duration = Duration::from_secs(3600);
/// Room made for a set when a title that plays needs it and the budget is full: about 8 KB a
/// picture, with a margin.
const BYTES_PER_PICTURE: u64 = 12 << 10;

/// A finished set (`index.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Index {
    pub format: u32,
    /// Seconds between pictures: picture `i` stands for `i * interval` to `(i + 1) * interval`.
    pub interval: f64,
    /// One picture's size.
    pub width: u32,
    pub height: u32,
    pub columns: u32,
    pub rows: u32,
    /// Pictures, and the sheets they are on (`count` / (`columns` × `rows`), rounded up).
    pub count: u32,
    pub sheets: u32,
    pub duration: f64,
    /// The I-frame track, if it could be made.
    #[serde(default)]
    pub iframes: Option<IFrames>,
    /// Bytes on disk, bytes read from the share for it, and seconds it took (working, not
    /// waiting for playback to end).
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub read: u64,
    #[serde(default)]
    pub secs: f64,
}

/// Where the pictures are in `iframes.mp4`: its init section and one fragment per picture,
/// as (offset, length).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IFrames {
    pub init: (u64, u64),
    pub fragments: Vec<(u64, u64)>,
    /// The `CODECS` attribute (`avc1.…`).
    pub codecs: String,
}

/// Progress (`GET /v1/admin/activity` → `trickplay`).
#[derive(Clone, Debug, Default, Serialize)]
pub struct Status {
    /// "off", "starting", "running", "paused" (something plays), "full" (the budget is used
    /// up), "idle" (nothing to do).
    pub state: &'static str,
    /// Files without a set this pass, and how many are done or failed.
    pub files: usize,
    pub done: usize,
    pub failed: usize,
    /// Files that get no set, and why (e.g. "Dolby Vision profile 5").
    pub skipped: usize,
    pub skipped_why: Vec<String>,
    /// The file being worked on (its name), and how far (pictures).
    pub current: Option<String>,
    pub pictures: u32,
    pub of: u32,
    /// Space the sets take, and the budget.
    pub bytes: u64,
    pub max_bytes: u64,
}

pub struct Trickplay {
    root: PathBuf,
    max_bytes: u64,
    used: AtomicU64,
    status: Mutex<Status>,
    hints: Mutex<VecDeque<String>>,
    failed: Mutex<HashMap<String, Instant>>,
    wake: Notify,
}

impl Trickplay {
    pub fn new(cache_dir: &Path, max_bytes: u64) -> Self {
        Trickplay {
            root: cache_dir.join("trickplay"),
            max_bytes,
            used: AtomicU64::new(0),
            status: Mutex::new(Status {
                state: "starting",
                max_bytes,
                ..Default::default()
            }),
            hints: Mutex::default(),
            failed: Mutex::default(),
            wake: Notify::new(),
        }
    }

    pub fn status(&self) -> Status {
        let mut s = self.status.lock().unwrap().clone();
        s.bytes = self.used.load(Ordering::Relaxed);
        s
    }

    fn set_state(&self, state: &'static str) {
        self.status.lock().unwrap().state = state;
    }

    /// Item ids to do before the rest (a title opened in a player, the next episodes).
    pub fn hint(&self, ids: Vec<String>) {
        if ids.is_empty() {
            return;
        }
        let mut h = self.hints.lock().unwrap();
        for id in ids.into_iter().rev() {
            h.retain(|x| *x != id);
            h.push_front(id);
        }
        h.truncate(20);
        drop(h);
        self.wake.notify_one();
    }

    pub fn dir(&self, key: &str) -> PathBuf {
        self.root.join(key)
    }

    /// A file's finished set (None: not made, or made by an older version). A player asks for
    /// it: the set counts as used now (it is kept over others when the budget is full).
    pub async fn index(&self, key: &str) -> Option<Index> {
        if !valid_key(key) {
            return None;
        }
        let path = self.dir(key).join("index.json");
        let data = tokio::fs::read(&path).await.ok()?;
        let index = serde_json::from_slice::<Index>(&data).ok().filter(|i| i.format == FORMAT)?;
        let _ = tokio::task::spawn_blocking(move || mark_used(&path, std::time::SystemTime::now())).await;
        Some(index)
    }

    /// A sheet or the I-frame track of a set (None: no such file).
    pub fn file(&self, key: &str, name: &str) -> Option<PathBuf> {
        let ok = name == "iframes.mp4"
            || name
                .strip_suffix(".jpg")
                .is_some_and(|n| !n.is_empty() && n.len() < 6 && n.bytes().all(|b| b.is_ascii_digit()));
        (valid_key(key) && ok).then(|| self.dir(key).join(name))
    }
}

/// A file key as `media/mod.rs` makes it (16 hex digits): nothing else names a folder.
fn valid_key(key: &str) -> bool {
    key.len() == 16 && key.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn file_key(path: &str, size: i64, mtime: i64) -> String {
    format!("{:016x}", crate::media::fnv(&format!("{path}|{size}|{mtime}")))
}

fn enabled(st: &AppState) -> bool {
    st.config.enabled(Feature::Trickplay)
}

/// The pictures' times: picture `i` is the keyframe at or before the middle of its interval.
/// (count, interval).
pub fn plan(duration: f64) -> (u32, f64) {
    if duration <= 0.0 {
        return (0, INTERVAL);
    }
    let count = (duration / INTERVAL).ceil() as u32;
    if count <= MAX_COUNT {
        return (count.max(1), INTERVAL);
    }
    let interval = (duration / MAX_COUNT as f64).ceil();
    ((duration / interval).ceil() as u32, interval)
}

/// A picture's height for a display aspect ratio (even, at least 2).
pub fn height_for(dar: f64) -> u32 {
    let dar = if dar.is_finite() && dar > 0.1 { dar } else { 16.0 / 9.0 };
    ((WIDTH as f64 / dar / 2.0).round() as u32 * 2).clamp(2, WIDTH * 2)
}

/// What a file's picture-making needs from its probe.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub stream: i64,
    pub duration: f64,
    pub height: u32,
    /// PQ or HLG: tone-mapped to SDR.
    pub hdr: bool,
    /// The length isn't ffprobe's duration (`plan_of`): a set made before 0.31.11 counted from 0
    /// and is made again (`stale`).
    pub own_clock: bool,
}

/// A finished set (`index.json` in `dir`) that isn't for this plan: one of a file with a start
/// time (`Plan::own_clock`) made before 0.31.11, which ran to the file's last timestamp. Other
/// files' sets are kept as they are.
fn stale(dir: &Path, plan: &Plan) -> bool {
    plan.own_clock
        && std::fs::read(dir.join("index.json"))
            .ok()
            .and_then(|d| serde_json::from_slice::<Index>(&d).ok())
            // As written in JSON, which needn't read back to the last bit.
            .is_some_and(|i| (i.duration - plan.duration).abs() > 0.001)
}

/// Why a file gets no set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Skip {
    /// No video, or no length.
    Nothing,
    /// Dolby Vision with no base layer a player could show without it (profile 5, compatibility
    /// 0): decoded alone, its pictures have wrong colours (green and purple).
    DoviOnly,
}

impl Skip {
    pub fn reason(self) -> &'static str {
        match self {
            Skip::Nothing => "no video",
            Skip::DoviOnly => "Dolby Vision profile 5 (no HDR10 or SDR base for the pictures)",
        }
    }
}

pub fn plan_of(probe: &Value) -> Result<Plan, Skip> {
    let v = probe["streams"]
        .as_array()
        .and_then(|s| {
            s.iter()
                .find(|p| p["codec_type"] == "video" && p["disposition"]["attached_pic"].as_i64() != Some(1))
        })
        .ok_or(Skip::Nothing)?;
    let probed = probe["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .or_else(|| probe["format"]["duration"].as_f64())
        .ok_or(Skip::Nothing)?;
    // The title's length from its start (players count that as 0): a Matroska file with a start
    // time has its last timestamp as its duration (`media::OwnClock`). The pictures' `-ss` counts
    // from the start already.
    let own_clock = crate::media::OwnClock::of(probe)
        .and_then(|c| c.length)
        .filter(|l| (*l - probed).abs() > 0.001);
    let duration = own_clock.unwrap_or(probed);
    if duration.partial_cmp(&1.0) != Some(std::cmp::Ordering::Greater) {
        return Err(Skip::Nothing);
    }
    if crate::media::Dovi::from_probe(v).is_some_and(|d| d.compat == 0) {
        return Err(Skip::DoviOnly);
    }
    let (w, h) = (v["width"].as_f64().unwrap_or(0.0), v["height"].as_f64().unwrap_or(0.0));
    let sar = v["sample_aspect_ratio"]
        .as_str()
        .and_then(|r| r.split_once(':'))
        .and_then(|(n, d)| Some((n.parse::<f64>().ok()?, d.parse::<f64>().ok()?)));
    let sar = match sar {
        Some((n, d)) if n > 0.0 && d > 0.0 => n / d,
        _ => 1.0,
    };
    let dar = if h > 0.0 { w * sar / h } else { 16.0 / 9.0 };
    let hdr = matches!(v["color_transfer"].as_str(), Some("smpte2084" | "arib-std-b67"));
    Ok(Plan {
        stream: v["index"].as_i64().ok_or(Skip::Nothing)?,
        duration,
        height: height_for(dar),
        hdr,
        own_clock: own_clock.is_some(),
    })
}

/// The `-vf` for one picture.
fn filter(height: u32, hdr: bool) -> String {
    let scale = format!("scale={WIDTH}:{height}:flags=bilinear");
    if hdr {
        format!("{scale},zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p,setsar=1")
    } else {
        format!("{scale},format=yuv420p,setsar=1")
    }
}

/// The init section and the fragments (moof + mdat) of a fragmented MP4, as (offset, length).
pub fn fragments(data: &[u8]) -> Option<((u64, u64), Vec<(u64, u64)>)> {
    let mut boxes = Vec::new();
    let mut at = 0usize;
    while at + 8 <= data.len() {
        let size32 = u32::from_be_bytes(data[at..at + 4].try_into().ok()?) as usize;
        let kind = &data[at + 4..at + 8];
        let size = match size32 {
            1 => u64::from_be_bytes(data.get(at + 8..at + 16)?.try_into().ok()?) as usize,
            0 => data.len() - at,
            n => n,
        };
        if size < 8 || at + size > data.len() {
            return None;
        }
        boxes.push((kind.to_vec(), at as u64, size as u64));
        at += size;
    }
    let moov = boxes.iter().position(|b| b.0 == b"moov")?;
    let init = (0, boxes[moov].1 + boxes[moov].2);
    let mut frags = Vec::new();
    let mut i = moov + 1;
    while i < boxes.len() {
        if boxes[i].0 == b"moof" {
            let start = boxes[i].1;
            let mdat = boxes[i + 1..].iter().position(|b| b.0 == b"mdat")? + i + 1;
            frags.push((start, boxes[mdat].1 + boxes[mdat].2 - start));
            i = mdat + 1;
        } else {
            i += 1;
        }
    }
    (!frags.is_empty()).then_some((init, frags))
}

/// The `CODECS` attribute of an H.264 init section (`avc1.PPCCLL` from its `avcC`).
pub fn avc_codecs(init: &[u8]) -> Option<String> {
    let p = init.windows(4).position(|w| w == b"avcC")? + 4;
    let b = init.get(p..p + 4)?;
    Some(format!("avc1.{:02x}{:02x}{:02x}", b[1], b[2], b[3]))
}

/// The I-frame playlist of a set; `mp4` is the track's URL (relative to the playlist).
pub fn iframe_playlist(index: &Index, mp4: &str) -> Option<String> {
    let f = index.iframes.as_ref()?;
    let mut out = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-I-FRAMES-ONLY\n#EXT-X-MAP:URI=\"{mp4}\",BYTERANGE=\"{}@{}\"\n",
        index.interval.ceil() as i64,
        f.init.1,
        f.init.0
    );
    for (i, (off, len)) in f.fragments.iter().enumerate() {
        let start = i as f64 * index.interval;
        let d = (index.duration - start).min(index.interval);
        if d <= 0.0 {
            break;
        }
        let _ = write!(out, "#EXTINF:{d:.3},\n#EXT-X-BYTERANGE:{len}@{off}\n{mp4}\n");
    }
    out += "#EXT-X-ENDLIST\n";
    Some(out)
}

/// The master playlist's line for a set's I-frame stream (`uri`: its playlist).
pub fn iframe_stream_inf(index: &Index, uri: &str) -> Option<String> {
    let f = index.iframes.as_ref()?;
    let peak = f.fragments.iter().map(|x| x.1).max().unwrap_or(0);
    let bandwidth = ((peak * 8) as f64 / index.interval).ceil().max(1000.0) as u64;
    Some(format!(
        "#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH={bandwidth},CODECS=\"{}\",RESOLUTION={}x{},URI=\"{uri}\"\n",
        f.codecs, index.width, index.height
    ))
}

/// A file without a set.
#[derive(Clone, Debug)]
struct Todo {
    item: Option<String>,
    path: String,
    size: i64,
    mtime: i64,
    key: String,
    plan: Plan,
}

/// The making of sets, forever (started with the library).
pub async fn run(st: Arc<AppState>) {
    let (Some(library), Some(_)) = (st.library.clone(), st.db.clone()) else {
        return;
    };
    let tp = st.trickplay.clone();
    let _ = tokio::fs::create_dir_all(&tp.root).await;
    // Let the first scan pass, the probes and the index backfill go first.
    tokio::time::sleep(Duration::from_secs(60)).await;
    loop {
        if !enabled(&st) {
            tp.set_state("off");
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(5)) => {}
                () = tp.wake.notified() => {}
            }
            continue;
        }
        let (todo, keep, skipped) = match list(&st).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("trickplay: {e:#}");
                (Vec::new(), None, Vec::new())
            }
        };
        if let Some(keep) = keep {
            let root = tp.root.clone();
            let removed = tokio::task::spawn_blocking(move || remove_stale(&root, &keep)).await.unwrap_or(0);
            if removed > 0 {
                tracing::info!(removed, "trickplay: sets of changed or removed files deleted");
            }
        }
        let root = tp.root.clone();
        tp.used.store(
            tokio::task::spawn_blocking(move || dir_size(&root)).await.unwrap_or(0),
            Ordering::Relaxed,
        );
        let failed_recently = |key: &str| tp.failed.lock().unwrap().get(key).is_some_and(|at| at.elapsed() < RETRY_FAILED);
        let todo: Vec<Todo> = todo.into_iter().filter(|t| !failed_recently(&t.key)).collect();
        let skipped_why = skipped_reasons(&skipped);
        if !skipped.is_empty() && tp.status.lock().unwrap().skipped != skipped.len() {
            tracing::info!(files = skipped.len(), why = %skipped_why.join("; "), "trickplay: files without thumbnails");
        }
        *tp.status.lock().unwrap() = Status {
            state: if todo.is_empty() { "idle" } else { "running" },
            files: todo.len(),
            skipped: skipped.len(),
            skipped_why,
            max_bytes: tp.max_bytes,
            ..Default::default()
        };
        if !todo.is_empty() {
            tracing::info!(
                files = todo.len(),
                gb_used = tp.used.load(Ordering::Relaxed) as f64 / 1e9,
                "trickplay"
            );
        }
        let mut queue: VecDeque<Todo> = todo.into();
        let mut seen: HashSet<String> = HashSet::new();
        let t0 = Instant::now();
        while enabled(&st) {
            let (next, hinted) = {
                let mut hinted = None;
                while let Some(id) = tp.hints.lock().unwrap().pop_front() {
                    if let Some(i) = queue.iter().position(|t| t.item.as_deref() == Some(id.as_str())) {
                        hinted = queue.remove(i);
                        break;
                    }
                }
                let was = hinted.is_some();
                (hinted.or_else(|| queue.pop_front()), was)
            };
            let Some(t) = next else { break };
            let dir = tp.dir(&t.key);
            if !seen.insert(t.key.clone()) || (dir.join("index.json").exists() && !stale(&dir, &t.plan)) {
                continue;
            }
            let need = plan(t.plan.duration).0 as u64 * BYTES_PER_PICTURE;
            if tp.used.load(Ordering::Relaxed) + need > tp.max_bytes {
                if !hinted {
                    // Hints that come meanwhile wake it again.
                    tp.set_state("full");
                    tracing::info!(
                        gb = tp.max_bytes as f64 / 1e9,
                        "trickplay: the space for thumbnails is full (performance.trickplay_max_gb)"
                    );
                    break;
                }
                // A title that plays: room is made by deleting the sets used longest ago.
                let over = (tp.used.load(Ordering::Relaxed) + need).saturating_sub(tp.max_bytes);
                let root = tp.root.clone();
                let (sets, freed) = tokio::task::spawn_blocking(move || evict(&root, over)).await.unwrap_or((0, 0));
                tp.used.fetch_sub(freed.min(tp.used.load(Ordering::Relaxed)), Ordering::Relaxed);
                tracing::info!(sets, mb = freed as f64 / 1e6, file = %vfs::file_name(&t.path), "trickplay: the space is full: the sets used longest ago deleted for a title that plays");
                if tp.used.load(Ordering::Relaxed) + need > tp.max_bytes {
                    tp.set_state("full");
                    break;
                }
            }
            let r = make(&st, &t).await;
            {
                let mut s = tp.status.lock().unwrap();
                s.current = None;
                match r {
                    Ok(Some(index)) => {
                        s.done += 1;
                        tp.used.fetch_add(index.bytes, Ordering::Relaxed);
                        tracing::info!(file = %vfs::file_name(&t.path), pictures = index.count, kb = index.bytes / 1024, read_mb = index.read as f64 / 1e6, secs = (index.secs * 10.0).round() / 10.0, iframes = index.iframes.is_some(), "thumbnails made");
                    }
                    Ok(None) => {}
                    Err(e) => {
                        s.failed += 1;
                        tracing::warn!(file = %vfs::file_name(&t.path), "thumbnails: {e:#}");
                        tp.failed.lock().unwrap().insert(t.key.clone(), Instant::now());
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let s = tp.status();
        if s.files > 0 {
            tracing::info!(
                files = s.files,
                done = s.done,
                failed = s.failed,
                secs = t0.elapsed().as_secs(),
                "trickplay pass finished"
            );
        }
        if !enabled(&st) {
            continue;
        }
        if s.state != "full" {
            tp.set_state("idle");
        }
        tokio::select! {
            () = library.probed.notified() => {}
            () = tp.wake.notified() => {}
            () = tokio::time::sleep(Duration::from_secs(3600)) => {}
        }
    }
}

/// The files without a set, in the order to make them, and (when the library could be read)
/// every file key in the library, to keep their sets.
async fn list(st: &AppState) -> Result<(Vec<Todo>, Option<HashSet<String>>, Vec<Skip>)> {
    let db = st.db.clone().context("no database")?;
    let root = st.trickplay.root.clone();
    db.call(move |c| {
        let mut q = c.prepare(&format!(
            "SELECT f.item_id, f.path, f.size, f.mtime, fp.probe FROM files f
             JOIN files_probe fp ON fp.path = f.path AND fp.size = f.size AND fp.mtime = f.mtime
             WHERE f.missing_since IS NULL AND f.role IN {} AND f.probe_error IS NULL
             ORDER BY f.first_seen DESC, f.path",
            crate::library::TITLE_ROLES
        ))?;
        let rows = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        // Every file the library has (missing ones too: a share that is away keeps its sets).
        let keep: HashSet<String> = c
            .prepare("SELECT path, size, mtime FROM files")?
            .query_map([], |r| Ok(file_key(&r.get::<_, String>(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?;
        let mut skipped = Vec::new();
        let mut todo: Vec<Todo> = rows
            .into_iter()
            .filter_map(|(item, path, size, mtime, probe)| {
                let key = file_key(&path, size, mtime);
                // A finished set is kept, unless it's a stale one (`stale`).
                let done = root.join(&key).join("index.json").exists();
                match plan_of(&serde_json::from_str(&probe).ok()?) {
                    Ok(plan) if done && !stale(&root.join(&key), &plan) => None,
                    Ok(plan) => Some(Todo {
                        item,
                        path,
                        size,
                        mtime,
                        key,
                        plan,
                    }),
                    Err(why) => {
                        if why != Skip::Nothing {
                            skipped.push(why);
                        }
                        None
                    }
                }
            })
            .collect();
        if !todo.is_empty() {
            let wanted = crate::media::index_backfill::watching(c)?;
            let rank = |t: &Todo| {
                t.item
                    .as_ref()
                    .and_then(|i| wanted.iter().position(|w| w == i))
                    .unwrap_or(usize::MAX)
            };
            todo.sort_by_key(rank);
        }
        Ok((todo, Some(keep), skipped))
    })
    .await
}

/// "3 Dolby Vision profile 5 (…)", per reason.
fn skipped_reasons(skipped: &[Skip]) -> Vec<String> {
    let mut out: Vec<(Skip, usize)> = Vec::new();
    for s in skipped {
        match out.iter_mut().find(|o| o.0 == *s) {
            Some(o) => o.1 += 1,
            None => out.push((*s, 1)),
        }
    }
    out.into_iter().map(|(s, n)| format!("{n} {}", s.reason())).collect()
}

/// Sets a set's last use (its `index.json`'s mtime), at most once an hour.
fn mark_used(index_json: &Path, now: std::time::SystemTime) {
    let Ok(m) = std::fs::metadata(index_json) else { return };
    if m.modified()
        .ok()
        .and_then(|t| now.duration_since(t).ok())
        .is_some_and(|d| d < USE_GRANULARITY)
    {
        return;
    }
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(index_json) {
        let _ = f.set_modified(now);
    }
}

/// Deletes finished sets, the one used longest ago first, until `bytes` are freed. Returns how
/// many sets and bytes. Sets being made (`.part`) are left alone.
fn evict(root: &Path, bytes: u64) -> (usize, u64) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return (0, 0);
    };
    let mut sets: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().ends_with(".part"))
        .filter_map(|e| Some((std::fs::metadata(e.path().join("index.json")).ok()?.modified().ok()?, e.path())))
        .collect();
    sets.sort();
    let (mut n, mut freed) = (0, 0);
    for (_, dir) in sets {
        if freed >= bytes {
            break;
        }
        let size = dir_size(&dir);
        if std::fs::remove_dir_all(&dir).is_ok() {
            n += 1;
            freed += size;
        }
    }
    (n, freed)
}

/// Deletes the sets (and leftovers) of files not in `keep`. Returns how many.
#[allow(clippy::case_sensitive_file_extension_comparisons, reason = "`.part` is our own suffix")]
fn remove_stale(root: &Path, keep: &HashSet<String>) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut n = 0;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let key = name.strip_suffix(".part").unwrap_or(&name);
        // A `.part` is a set being made, or left by a stop: made again from the start.
        if name.ends_with(".part") || !keep.contains(key) {
            let p = e.path();
            let r = if p.is_dir() {
                std::fs::remove_dir_all(&p)
            } else {
                std::fs::remove_file(&p)
            };
            if r.is_ok() {
                n += 1;
            }
        }
    }
    n
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            match e.metadata() {
                Ok(m) if m.is_dir() => stack.push(e.path()),
                Ok(m) => total += m.len(),
                Err(_) => {}
            }
        }
    }
    total
}

/// Makes one file's set. Ok(None): given up (the file changed, or switched off).
async fn make(st: &Arc<AppState>, t: &Todo) -> Result<Option<Index>> {
    let tp = st.trickplay.clone();
    let now = vfs::stat(&t.path).await?;
    if (now.size as i64, now.mtime) != (t.size, t.mtime) {
        return Ok(None);
    }
    let (count, interval) = plan(t.plan.duration);
    {
        let mut s = tp.status.lock().unwrap();
        s.current = Some(vfs::file_name(&t.path));
        s.pictures = 0;
        s.of = count;
    }
    let work = tp.root.join(format!("{}.part", t.key));
    let _ = tokio::fs::remove_dir_all(&work).await;
    tokio::fs::create_dir_all(&work).await?;
    let r = make_in(st, t, &work, count, interval).await;
    let r = match r {
        Err(e) if e.is::<Off>() => Ok(None),
        r => r,
    };
    match r {
        Ok(Some(index)) => {
            let dir = tp.dir(&t.key);
            let _ = tokio::fs::remove_dir_all(&dir).await;
            tokio::fs::rename(&work, &dir).await?;
            Ok(Some(index))
        }
        other => {
            let _ = tokio::fs::remove_dir_all(&work).await;
            other
        }
    }
}

/// Switched off while a set was being made.
#[derive(Debug)]
struct Off;

impl std::fmt::Display for Off {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("switched off")
    }
}

impl std::error::Error for Off {}

/// An MP4/MOV file's keyframes, read one sample at a time (`moov::video_keys`).
struct Keys {
    /// ffmpeg's raw format: `h264` or `hevc`.
    format: &'static str,
    /// The parameter sets, in Annex B, put before every keyframe.
    head: Vec<u8>,
    len_size: usize,
    keys: Vec<crate::media::moov::KeySample>,
    /// The keyframe the picture before was made from.
    last: Option<usize>,
}

/// A range reader over a file that counts what it reads, at a pace on a share.
struct CountedRead<'a> {
    path: &'a str,
    pace: Option<&'a vfs::Pace>,
    read: AtomicU64,
}

impl crate::media::mkv::RangeRead for CountedRead<'_> {
    fn read_range(&self, offset: u64, len: u64) -> futures_util::future::BoxFuture<'_, Result<Vec<u8>>> {
        Box::pin(async move {
            if let Some(p) = self.pace {
                p.take(len).await;
            }
            let b = vfs::read_at(self.path, offset, len).await?;
            self.read.fetch_add(b.len() as u64, Ordering::Relaxed);
            Ok(b.to_vec())
        })
    }
}

fn is_mp4(path: &str) -> bool {
    let name = vfs::file_name(path).to_ascii_lowercase();
    [".mp4", ".m4v", ".mov"].iter().any(|e| name.ends_with(e))
}

/// The keyframes of an MP4/MOV file with H.264 or HEVC video, from its `moov`.
async fn mp4_keys(rd: &CountedRead<'_>) -> Result<Keys> {
    let moov = crate::media::moov::read_moov_with(rd).await?;
    let v = crate::media::moov::video_keys(&moov)?;
    let (sets, len_size) = parameter_sets(&v.codec, &v.config).context("unreadable decoder configuration")?;
    let format = if v.codec.starts_with(b"avc") { "h264" } else { "hevc" };
    let mut head = Vec::new();
    for p in &sets {
        head.extend_from_slice(&[0, 0, 0, 1]);
        head.extend_from_slice(p);
    }
    Ok(Keys {
        format,
        head,
        len_size,
        keys: v.keys,
        last: None,
    })
}

/// The parameter sets (H.264's SPS and PPS, HEVC's VPS, SPS, PPS and SEI) of an `avcC` or
/// `hvcC` body, and the size of the samples' NAL unit lengths (bytes).
pub fn parameter_sets(codec: &[u8; 4], config: &[u8]) -> Option<(Vec<Vec<u8>>, usize)> {
    let mut sets = Vec::new();
    let take = |at: &mut usize, sets: &mut Vec<Vec<u8>>| -> Option<()> {
        let len = u16::from_be_bytes(config.get(*at..*at + 2)?.try_into().ok()?) as usize;
        sets.push(config.get(*at + 2..*at + 2 + len)?.to_vec());
        *at += 2 + len;
        Some(())
    };
    if codec.starts_with(b"avc") {
        let len_size = (*config.get(4)? & 3) as usize + 1;
        let mut at = 6;
        for _ in 0..(*config.get(5)? & 0x1f) {
            take(&mut at, &mut sets)?;
        }
        let pps = *config.get(at)?;
        at += 1;
        for _ in 0..pps {
            take(&mut at, &mut sets)?;
        }
        Some((sets, len_size))
    } else {
        let len_size = (*config.get(21)? & 3) as usize + 1;
        let mut at = 23;
        for _ in 0..*config.get(22)? {
            let n = u16::from_be_bytes(config.get(at + 1..at + 3)?.try_into().ok()?);
            at += 3;
            for _ in 0..n {
                take(&mut at, &mut sets)?;
            }
        }
        Some((sets, len_size))
    }
}

/// A sample's NAL units (each after its length) in Annex B (each after a start code).
pub fn annex_b(sample: &[u8], len_size: usize, out: &mut Vec<u8>) -> Option<()> {
    let mut at = 0;
    while at < sample.len() {
        let mut len = 0usize;
        for b in sample.get(at..at + len_size)? {
            len = len << 8 | *b as usize;
        }
        at += len_size;
        let nal = sample.get(at..at + len)?;
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
        at += len;
    }
    Some(())
}

/// The keyframe for a picture at `at` seconds: the last one at or before it (else the first).
pub fn key_for(keys: &[crate::media::moov::KeySample], at: f64) -> usize {
    keys.partition_point(|k| k.time <= at + 1e-6).saturating_sub(1)
}

/// One picture from the keyframe `n` of an MP4: its sample alone is read and decoded.
async fn picture_from_key(
    ffmpeg: &str,
    rd: &CountedRead<'_>,
    keys: &Keys,
    n: usize,
    height: u32,
    hdr: &mut bool,
    out: &Path,
) -> Result<()> {
    let k = keys.keys[n];
    if k.size == 0 || k.size > 64 << 20 {
        bail!("keyframe of {} bytes", k.size);
    }
    let sample = crate::media::mkv::RangeRead::read_range(rd, k.offset, k.size as u64).await?;
    if sample.len() != k.size as usize {
        bail!("keyframe cut short");
    }
    let mut data = keys.head.clone();
    annex_b(&sample, keys.len_size, &mut data).context("keyframe's NAL units")?;
    let key = out.with_file_name(format!("key.{}", keys.format));
    tokio::fs::write(&key, &data).await?;
    let decode = |tonemap: bool| -> Vec<String> {
        // `showall`: an I-frame that isn't an IDR (an open GOP's) is shown too.
        let mut args: Vec<String> = ["-threads", "1", "-flags2", "+showall", "-f", keys.format, "-i"]
            .iter()
            .map(ToString::to_string)
            .collect();
        args.push(key.to_string_lossy().to_string());
        args.extend(
            [
                "-an",
                "-sn",
                "-dn",
                "-frames:v",
                "1",
                "-vf",
                &filter(height, tonemap),
                "-q:v",
                "5",
                "-f",
                "image2",
                "-update",
                "1",
                "-y",
            ]
            .iter()
            .map(ToString::to_string),
        );
        args.push(out.to_string_lossy().to_string());
        args
    };
    let mut r = nice_ffmpeg(ffmpeg, &decode(*hdr), PICTURE_TIMEOUT).await;
    if r.is_err() && *hdr {
        *hdr = false;
        r = nice_ffmpeg(ffmpeg, &decode(false), PICTURE_TIMEOUT).await;
    }
    let _ = tokio::fs::remove_file(&key).await;
    r?;
    if tokio::fs::metadata(out).await.map_or(0, |m| m.len()) == 0 {
        bail!("no picture from the keyframe at {:.1} s", k.time);
    }
    Ok(())
}

/// Waits while anything plays. Err(Off) when switched off meanwhile.
async fn wait_idle(st: &AppState) -> Result<()> {
    let tp = &st.trickplay;
    while index_busy(st) {
        if !enabled(st) {
            return Err(Off.into());
        }
        tp.set_state("paused");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    if !enabled(st) {
        return Err(Off.into());
    }
    tp.set_state("running");
    Ok(())
}

async fn make_in(st: &Arc<AppState>, t: &Todo, work: &Path, count: u32, interval: f64) -> Result<Option<Index>> {
    let tp = st.trickplay.clone();
    let source = Source::of(&t.path);
    let mut mode = Mode {
        hdr: t.plan.hdr,
        copy: true,
        keys: None,
    };
    let mut failures = 0u32;
    let mut read = 0u64;
    let mut worked = Duration::ZERO;
    let pace = vfs::Pace::new(RATE);
    let rd = CountedRead {
        path: &t.path,
        pace: matches!(source, Source::Smb(_)).then_some(&pace),
        read: AtomicU64::new(0),
    };
    if is_mp4(&t.path) {
        wait_idle(st).await?;
        let started = Instant::now();
        match mp4_keys(&rd).await {
            Ok(k) => mode.keys = Some(k),
            Err(e) => tracing::debug!(file = %vfs::file_name(&t.path), "thumbnails: keyframes not read from the index: {e:#}"),
        }
        worked += started.elapsed();
    }
    for i in 0..count {
        if i == 1 && !mode.copy {
            // The keyframe's packet can't be copied out (AVI's MPEG-4 without timestamps), and
            // decoding a picture from the file reads ~10 MB around it: one pass over the whole
            // file reads less.
            let started = Instant::now();
            let r = whole_pass(st, &source, &t.plan, t.size.max(0) as u64, count, interval, mode.hdr, work).await;
            worked += started.elapsed();
            match r? {
                Some(n) => read += n,
                None => return Ok(None),
            }
            break;
        }
        // Never while something plays.
        wait_idle(st).await?;
        let at = ((i as f64 + 0.5) * interval).min(t.plan.duration - 0.5).max(0.0);
        let out = work.join(format!("{i:05}.jpg"));
        let started = Instant::now();
        let mut r = Err(anyhow::anyhow!("not made"));
        let mut from_keys = false;
        if let Some(keys) = &mut mode.keys {
            let n = key_for(&keys.keys, at);
            if i > 0 && keys.last == Some(n) {
                // Keyframes further apart than the pictures: the same picture again.
                r = tokio::fs::copy(work.join(format!("{:05}.jpg", i - 1)), &out)
                    .await
                    .map(|_| 0)
                    .map_err(Into::into);
                from_keys = r.is_ok();
            } else {
                match picture_from_key(&st.config.ffmpeg, &rd, keys, n, t.plan.height, &mut mode.hdr, &out).await {
                    Ok(()) => {
                        keys.last = Some(n);
                        r = Ok(0);
                        from_keys = true;
                    }
                    Err(e) if keys.last.is_none() => {
                        // The first keyframe alone didn't decode: seeking in the file instead,
                        // for all of it.
                        tracing::debug!(file = %vfs::file_name(&t.path), "thumbnails: the keyframe alone didn't decode, seeking instead: {e:#}");
                        mode.keys = None;
                    }
                    Err(e) => {
                        // One damaged keyframe: the picture before stands in (below).
                        r = Err(e);
                        from_keys = true;
                    }
                }
            }
        }
        if !from_keys {
            r = picture(&st.config.ffmpeg, &source, t.plan.stream, at, t.plan.height, &mut mode, &out).await;
        }
        worked += started.elapsed();
        match r {
            Ok(n) => read += n,
            Err(e) => {
                failures += 1;
                if i == 0 && failures == 1 {
                    tracing::debug!(file = %vfs::file_name(&t.path), "thumbnail at {at:.0} s: {e:#}");
                }
                // The one before stands in (a seek past the last keyframe, a damaged spot).
                let prev = work.join(format!("{:05}.jpg", i.saturating_sub(1)));
                if i == 0 {
                    return Err(e.context("the first picture"));
                }
                tokio::fs::copy(&prev, &out).await.context("the picture before")?;
                if failures > 5 && failures * 5 > i + 1 {
                    bail!("{failures} of {} pictures failed: {e:#}", i + 1);
                }
            }
        }
        tp.status.lock().unwrap().pictures = i + 1;
    }
    read += rd.read.load(Ordering::Relaxed);
    let started = Instant::now();
    let sheets = tile(st, work, count).await?;
    let iframes = match iframes(st, work, interval).await {
        Ok(f) => Some(f),
        Err(e) if e.is::<Off>() => return Err(e),
        Err(e) => {
            tracing::warn!(file = %vfs::file_name(&t.path), "thumbnail track for the Apple TV: {e:#}");
            let _ = tokio::fs::remove_file(work.join("iframes.mp4")).await;
            None
        }
    };
    worked += started.elapsed();
    for i in 0..count {
        let _ = tokio::fs::remove_file(work.join(format!("{i:05}.jpg"))).await;
    }
    let mut index = Index {
        format: FORMAT,
        interval,
        width: WIDTH,
        height: t.plan.height,
        columns: COLUMNS,
        rows: ROWS,
        count,
        sheets,
        duration: t.plan.duration,
        iframes,
        bytes: 0,
        read,
        secs: worked.as_secs_f64(),
    };
    let w = work.to_path_buf();
    index.bytes = tokio::task::spawn_blocking(move || dir_size(&w)).await.unwrap_or(0);
    tokio::fs::write(work.join("index.json"), serde_json::to_vec(&index)?).await?;
    Ok(Some(index))
}

fn index_busy(st: &AppState) -> bool {
    crate::media::index_backfill::busy(st)
}

async fn nice_ffmpeg(ffmpeg: &str, args: &[String], timeout: Duration) -> Result<()> {
    let out = tokio::process::Command::new("nice")
        .args(["-n", "19", ffmpeg, "-hide_banner", "-nostats", "-nostdin", "-loglevel", "error"])
        .args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(timeout, out).await.context("timed out")?.context("ffmpeg")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!(
            "ffmpeg: {}",
            crate::admin::logbuf::scrub(err.lines().rev().take(3).collect::<Vec<_>>().join(" / ").trim())
        );
    }
    Ok(())
}

/// One picture: the keyframe at or before `at` seconds, scaled (tone-mapped while `mode.hdr`).
/// Returns the bytes read from a share (0 for a local file).
///
/// Two runs: the keyframe's packet alone is copied out (`key.mkv`), then decoded. Decoding it
/// straight from the file read several more keyframes first (a decoder holds pictures back to
/// reorder them, ~20 MB of a 4K film for every picture); a file of one packet ends there. The
/// probe reads at most 2 MB, the file's head, which the share's block cache keeps.
async fn picture(ffmpeg: &str, source: &Source, stream: i64, at: f64, height: u32, mode: &mut Mode, out: &Path) -> Result<u64> {
    let pace = vfs::Pace::new(RATE);
    let input = match source {
        Source::Smb(p) => vfs::ffmpeg_input_args_paced(p, &pace),
        Source::Local(_) => source.input_args(),
    };
    let seek: Vec<String> = [
        "-probesize",
        "2000000",
        "-analyzeduration",
        "1000000",
        "-noaccurate_seek",
        "-ss",
        &format!("{at:.3}"),
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    let key = out.with_file_name("key.mkv");
    let decode = |input: &[String], copied: bool, tonemap: bool| {
        let mut args: Vec<String> = vec!["-threads".into(), "1".into()];
        if !copied {
            args.extend(["-skip_frame".to_string(), "nokey".to_string()]);
        }
        args.extend(input.iter().cloned());
        args.extend(
            [
                "-map",
                // `key.mkv` holds the video alone, at 0 whatever its number in the file.
                &if copied { "0:0".to_string() } else { format!("0:{stream}") },
                "-an",
                "-sn",
                "-dn",
                "-frames:v",
                "1",
                "-vf",
                &filter(height, tonemap),
                "-q:v",
                "5",
                "-f",
                "image2",
                "-update",
                "1",
                "-y",
            ]
            .iter()
            .map(ToString::to_string),
        );
        args.push(out.to_string_lossy().to_string());
        args
    };
    // The keyframe's packet alone, to decode from (`Mode::copy`).
    let mut copied = false;
    if mode.copy {
        let mut args = seek.clone();
        args.extend(input.iter().cloned());
        args.extend(
            [
                "-map",
                &format!("0:{stream}"),
                "-c",
                "copy",
                "-frames:v",
                "1",
                "-f",
                "matroska",
                "-y",
            ]
            .iter()
            .map(ToString::to_string),
        );
        args.push(key.to_string_lossy().to_string());
        match nice_ffmpeg(ffmpeg, &args, PICTURE_TIMEOUT).await {
            Ok(()) => copied = true,
            Err(e) => {
                // Packets Matroska can't take as they are (AVI's MPEG-4 without timestamps):
                // decoded from the file for the rest of it.
                tracing::debug!("thumbnails: decoding from the file: {e:#}");
                mode.copy = false;
            }
        }
    }
    let input_args: Vec<String> = if copied {
        vec!["-i".into(), key.to_string_lossy().to_string()]
    } else {
        let mut a = seek.clone();
        a.extend(input.iter().cloned());
        a
    };
    let mut r = nice_ffmpeg(ffmpeg, &decode(&input_args, copied, mode.hdr), PICTURE_TIMEOUT).await;
    if r.is_err() && mode.hdr {
        // No zscale in this ffmpeg (or it can't take this file): without tone mapping.
        mode.hdr = false;
        r = nice_ffmpeg(ffmpeg, &decode(&input_args, copied, false), PICTURE_TIMEOUT).await;
    }
    let _ = tokio::fs::remove_file(&key).await;
    r?;
    if tokio::fs::metadata(out).await.map_or(0, |m| m.len()) == 0 {
        bail!("no picture at {at:.1} s");
    }
    Ok(pace.sent())
}

/// Every picture in one pass over the whole file (keyframes decoded, one every `interval`
/// seconds kept), read at `RATE` from a share and stopped (SIGSTOP) while anything plays.
/// Returns the bytes read, or None when switched off meanwhile.
#[allow(
    clippy::too_many_arguments,
    reason = "the steps' inputs, passed through; a struct for one call would only rename them"
)]
async fn whole_pass(
    st: &Arc<AppState>,
    source: &Source,
    plan: &Plan,
    size: u64,
    count: u32,
    interval: f64,
    hdr: bool,
    work: &Path,
) -> Result<Option<u64>> {
    let tp = st.trickplay.clone();
    let mut tonemap = hdr;
    loop {
        let pace = vfs::Pace::new(RATE);
        let input = match source {
            Source::Smb(p) => vfs::ffmpeg_input_args_paced(p, &pace),
            Source::Local(_) => source.input_args(),
        };
        let mut args: Vec<String> = QUIET
            .iter()
            .chain(&["-threads", "1", "-skip_frame", "nokey"])
            .map(ToString::to_string)
            .collect();
        args.extend(input);
        let vf = format!("fps=fps=1/{interval}:round=down,{}", filter(plan.height, tonemap));
        args.extend(
            [
                "-map",
                &format!("0:{}", plan.stream),
                "-an",
                "-sn",
                "-dn",
                "-vf",
                &vf,
                "-q:v",
                "5",
                "-start_number",
                "0",
                "-f",
                "image2",
                "-y",
            ]
            .iter()
            .map(ToString::to_string),
        );
        args.push(work.join("%05d.jpg").to_string_lossy().to_string());
        let mut progress = || {
            let made = std::fs::read_dir(work).map_or(0, |d| {
                d.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".jpg")).count()
            });
            tp.status.lock().unwrap().pictures = (made as u32).min(count);
        };
        let ctl = Control {
            busy: &|| index_busy(st),
            on: &|| enabled(st),
            paused: &|p| tp.set_state(if p { "paused" } else { "running" }),
        };
        match run_yielding(&st.config.ffmpeg, &args, whole_pass_limit(size), &ctl, &mut progress).await {
            Ok(()) => {}
            Err(e) if e.is::<Off>() => return Ok(None),
            Err(_) if tonemap => {
                // No zscale in this ffmpeg (or it can't take this file): without tone mapping.
                tonemap = false;
                continue;
            }
            Err(e) => return Err(e),
        }
        // As many pictures as planned: the last one stands in for any missing at the end.
        let mut last = None;
        for i in 0..count {
            let p = work.join(format!("{i:05}.jpg"));
            if p.exists() {
                last = Some(p);
            } else if let Some(l) = &last {
                tokio::fs::copy(l, &p).await?;
            } else {
                bail!("no pictures");
            }
        }
        let mut i = count;
        while tokio::fs::remove_file(work.join(format!("{i:05}.jpg"))).await.is_ok() {
            i += 1;
        }
        return Ok(Some(pace.sent()));
    }
}

/// How long the whole-file pass may run (not counting while it is stopped): three times what
/// reading the file at `RATE` takes, and 10 minutes more. A damaged file that keeps ffmpeg busy
/// longer is given up.
pub fn whole_pass_limit(size: u64) -> Duration {
    Duration::from_secs(size / RATE * 3 + 600)
}

/// What a long ffmpeg run checks, every second: is something playing (it is stopped until
/// that ends), is the feature still on (it is ended), and whom to tell it was stopped.
struct Control<'a> {
    busy: &'a (dyn Fn() -> bool + Sync),
    on: &'a (dyn Fn() -> bool + Sync),
    paused: &'a (dyn Fn(bool) + Sync),
}

/// Runs `nice -n 19 <program> <args>`: stopped (SIGSTOP) while `ctl.busy`,
/// ended when `ctl.on` turns false (Err(Off)), given up after `limit` of running (not stopped)
/// time. Its stderr is read as it comes (the end kept for the error), so a run that writes a
/// lot of errors can't fill the pipe and hang. `tick` is called every second.
async fn run_yielding(program: &str, args: &[String], limit: Duration, ctl: &Control<'_>, tick: &mut (dyn FnMut() + Send)) -> Result<()> {
    use tokio::io::AsyncReadExt;
    let mut child = tokio::process::Command::new("nice")
        .args(["-n", "19", program])
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| program.to_string())?;
    let pid = child.id();
    let mut stderr = child.stderr.take().context("stderr")?;
    let drain = tokio::spawn(async move {
        // The last 4 KB.
        let mut tail: Vec<u8> = Vec::new();
        let mut buf = [0u8; 8192];
        while let Ok(n) = stderr.read(&mut buf).await {
            if n == 0 {
                break;
            }
            tail.extend_from_slice(&buf[..n]);
            if tail.len() > 4096 {
                tail.drain(..tail.len() - 4096);
            }
        }
        tail
    });
    let signal = |sig: i32| {
        if let Some(pid) = pid {
            // SAFETY: a signal to our own child, which is still running (not reaped).
            unsafe { libc::kill(pid as i32, sig) };
        }
    };
    let mut stopped = false;
    let mut ran = Duration::ZERO;
    let status = loop {
        let t = Instant::now();
        tokio::select! {
            s = child.wait() => break s?,
            () = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
        if !stopped {
            ran += t.elapsed();
        }
        if !(ctl.on)() {
            signal(libc::SIGCONT);
            let _ = child.kill().await;
            return Err(Off.into());
        }
        if ran > limit {
            signal(libc::SIGCONT);
            let _ = child.kill().await;
            bail!("{} gave up after {} s", vfs::file_name(program), limit.as_secs());
        }
        let busy = (ctl.busy)();
        if busy != stopped {
            signal(if busy { libc::SIGSTOP } else { libc::SIGCONT });
            stopped = busy;
            (ctl.paused)(busy);
        }
        tick();
    };
    if stopped {
        (ctl.paused)(false);
    }
    let tail = drain.await.unwrap_or_default();
    if !status.success() {
        let err = String::from_utf8_lossy(&tail);
        bail!(
            "{}: {}",
            vfs::file_name(program),
            crate::admin::logbuf::scrub(
                err.lines()
                    .rev()
                    .filter(|l| !l.trim().is_empty())
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" / ")
                    .trim()
            )
        );
    }
    Ok(())
}

/// How a file's pictures are made: tone-mapped (`hdr`), and through a copy of the keyframe's
/// packet (`copy`). Each is turned off for the rest of the file once it fails.
struct Mode {
    hdr: bool,
    copy: bool,
    /// MP4/MOV: the keyframes read alone (None: seeking in the file instead).
    keys: Option<Keys>,
}

/// An assembly step's ffmpeg run, after the last picture: stopped while anything plays, like
/// the rest.
async fn assemble(st: &AppState, args: &[String]) -> Result<()> {
    let tp = &st.trickplay;
    wait_idle(st).await?;
    let ctl = Control {
        busy: &|| index_busy(st),
        on: &|| enabled(st),
        paused: &|p| tp.set_state(if p { "paused" } else { "running" }),
    };
    let args: Vec<String> = QUIET.iter().map(ToString::to_string).chain(args.iter().cloned()).collect();
    run_yielding(&st.config.ffmpeg, &args, Duration::from_secs(300), &ctl, &mut || {}).await
}

/// ffmpeg's first arguments here: errors only.
const QUIET: [&str; 5] = ["-hide_banner", "-nostats", "-nostdin", "-loglevel", "error"];

/// The pictures as sheets (`{n}.jpg`). Returns how many.
async fn tile(st: &AppState, work: &Path, count: u32) -> Result<u32> {
    let per = COLUMNS * ROWS;
    let sheets = count.div_ceil(per);
    let args: Vec<String> = [
        "-threads",
        "2",
        "-framerate",
        "1",
        "-i",
        &work.join("%05d.jpg").to_string_lossy(),
        "-vf",
        &format!("tile={COLUMNS}x{ROWS}"),
        "-q:v",
        "5",
        "-start_number",
        "0",
        "-y",
        &work.join("%d.jpg").to_string_lossy(),
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assemble(st, &args).await?;
    for n in 0..sheets {
        if !work.join(format!("{n}.jpg")).exists() {
            bail!("sheet {n} of {sheets} wasn't made");
        }
    }
    Ok(sheets)
}

/// The pictures as an H.264 track of keyframes alone, one fragment each (`iframes.mp4`).
async fn iframes(st: &AppState, work: &Path, interval: f64) -> Result<IFrames> {
    let out = work.join("iframes.mp4");
    let args: Vec<String> = [
        "-threads",
        "2",
        "-framerate",
        &format!("1/{}", interval.round().max(1.0) as u64),
        "-i",
        &work.join("%05d.jpg").to_string_lossy(),
        "-c:v",
        "libx264",
        "-preset",
        "medium",
        "-profile:v",
        "main",
        "-level:v",
        "3.0",
        "-pix_fmt",
        "yuv420p",
        "-crf",
        "30",
        "-g",
        "1",
        "-bf",
        "0",
        "-video_track_timescale",
        "1000",
        "-movflags",
        "+frag_keyframe+empty_moov+default_base_moof",
        "-f",
        "mp4",
        "-y",
        &out.to_string_lossy(),
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assemble(st, &args).await?;
    let data = tokio::fs::read(&out).await?;
    let (init, fragments) = fragments(&data).context("no fragments in the track")?;
    let codecs = avc_codecs(&data[..init.1 as usize]).context("no avcC")?;
    Ok(IFrames { init, fragments, codecs })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    #[allow(clippy::float_cmp, reason = "the interval is a whole number of seconds (ceil)")]
    fn pictures_every_ten_seconds_at_most_2000() {
        assert_eq!(plan(2400.0), (240, 10.0));
        assert_eq!(plan(2405.0), (241, 10.0));
        assert_eq!(plan(3.0), (1, 10.0));
        assert_eq!(plan(0.0).0, 0);
        // Over 5.5 h: further apart.
        let (n, i) = plan(30_000.0);
        assert!(n <= 2000 && i == 15.0, "{n} {i}");
    }

    #[test]
    fn heights_follow_the_display_aspect() {
        assert_eq!(height_for(16.0 / 9.0), 180);
        assert_eq!(height_for(2.39), 134);
        assert_eq!(height_for(4.0 / 3.0), 240);
        assert_eq!(height_for(f64::NAN), 180);
        // Anamorphic DVD: 720×576 with 64:45 samples is 16:9.
        let p = json!({"format": {"duration": "1500.5"}, "streams": [
            {"index": 0, "codec_type": "video", "codec_name": "mjpeg", "disposition": {"attached_pic": 1}},
            {"index": 1, "codec_type": "video", "width": 720, "height": 576, "sample_aspect_ratio": "64:45", "color_transfer": "bt709"}]});
        assert_eq!(
            plan_of(&p),
            Ok(Plan {
                stream: 1,
                duration: 1500.5,
                height: 180,
                hdr: false,
                own_clock: false
            })
        );
        let p = json!({"format": {"duration": "7000"}, "streams": [{"index": 0, "codec_type": "video", "width": 3840, "height": 1600, "color_transfer": "smpte2084"}]});
        assert_eq!(
            plan_of(&p),
            Ok(Plan {
                stream: 0,
                duration: 7000.0,
                height: 134,
                hdr: true,
                own_clock: false
            })
        );
        // Audio only, or no length: nothing to make.
        assert_eq!(
            plan_of(&json!({"format": {"duration": "100"}, "streams": [{"index": 0, "codec_type": "audio"}]})),
            Err(Skip::Nothing)
        );
        assert_eq!(
            plan_of(&json!({"format": {}, "streams": [{"index": 0, "codec_type": "video"}]})),
            Err(Skip::Nothing)
        );
    }

    #[test]
    fn a_file_with_a_start_time_gets_pictures_for_its_length() {
        // A TV recording remuxed into Matroska keeping its broadcast clock: it starts at
        // 1002.597 s and its duration (its last timestamp) is 1182.96, so before 0.31.11 it got
        // 119 pictures for 180 s (and one starting at 30002.6 s, 1887 pictures 16 s apart).
        let probe = |name: &str, start: &str, duration: &str| {
            json!({"format": {"format_name": name, "start_time": start, "duration": duration},
                   "streams": [{"index": 0, "codec_type": "video", "width": 1280, "height": 720}]})
        };
        let mkv = plan_of(&probe("matroska,webm", "1002.597000", "1182.960000")).unwrap();
        assert!((mkv.duration - 180.363).abs() < 1e-6 && mkv.own_clock, "{mkv:?}");
        assert_eq!(plan(mkv.duration), (19, INTERVAL));
        let far = plan_of(&probe("matroska,webm", "30002.597000", "30182.960000")).unwrap();
        assert_eq!(plan(far.duration), (19, INTERVAL));
        // MPEG-TS's duration is its length already; files starting at 0 or a frame later as before.
        for (name, start, duration) in [
            ("mpegts", "1002.597000", "180.362667"),
            ("matroska,webm", "0.000000", "180.363000"),
            ("matroska,webm", "0.080000", "180.443000"),
        ] {
            let p = plan_of(&probe(name, start, duration)).unwrap();
            assert_eq!((p.duration, p.own_clock), (duration.parse().unwrap(), false));
        }
        // A set made before for the file's last timestamp is made again; others are kept.
        let dir = std::env::temp_dir().join(format!("moonrise-trickplay-stale-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let set = |duration: f64| {
            let (count, interval) = plan(duration);
            let index = Index {
                format: FORMAT,
                interval,
                width: WIDTH,
                height: 180,
                columns: COLUMNS,
                rows: ROWS,
                count,
                sheets: 1,
                duration,
                iframes: None,
                bytes: 0,
                read: 0,
                secs: 0.0,
            };
            std::fs::write(dir.join("index.json"), serde_json::to_vec(&index).unwrap()).unwrap();
        };
        set(1182.96);
        assert!(stale(&dir, &mkv));
        set(mkv.duration);
        assert!(!stale(&dir, &mkv));
        let ts = plan_of(&probe("mpegts", "1002.597000", "180.362667")).unwrap();
        set(1182.96);
        assert!(!stale(&dir, &ts), "only sets of files read on their own clock are checked");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn dovi(profile: u8, compat: u8) -> Value {
        json!({"format": {"duration": "7000"}, "streams": [{"index": 0, "codec_type": "video", "width": 3840, "height": 2160, "color_transfer": "smpte2084",
            "side_data_list": [{"side_data_type": "DOVI configuration record", "dv_profile": profile, "dv_level": 6, "rpu_present_flag": 1, "el_present_flag": 0,
                "bl_present_flag": 1, "dv_bl_signal_compatibility_id": compat}]}]})
    }

    #[test]
    fn dolby_vision_without_a_base_is_skipped_with_its_reason() {
        // Profile 5: no HDR10 or SDR base layer, its pictures alone have the wrong colours.
        assert_eq!(plan_of(&dovi(5, 0)), Err(Skip::DoviOnly));
        // 8.1 (HDR10 base) and 7 (HDR10 base, a second layer): made from the base, tone-mapped.
        assert!(plan_of(&dovi(8, 1)).is_ok_and(|p| p.hdr));
        assert!(plan_of(&dovi(7, 6)).is_ok());
        assert_eq!(
            skipped_reasons(&[Skip::DoviOnly, Skip::DoviOnly]),
            vec!["2 Dolby Vision profile 5 (no HDR10 or SDR base for the pictures)".to_string()]
        );
    }

    fn mp4_box(kind: &[u8; 4], size: usize) -> Vec<u8> {
        let mut b = (size as u32).to_be_bytes().to_vec();
        b.extend(kind);
        b.resize(size, 0);
        b
    }

    #[test]
    fn finds_the_fragments_of_the_track() {
        let mut data = Vec::new();
        data.extend(mp4_box(b"ftyp", 16));
        data.extend(mp4_box(b"moov", 100));
        let init_end = data.len() as u64;
        for n in 0..3 {
            data.extend(mp4_box(b"moof", 40));
            data.extend(mp4_box(b"mdat", 1000 + n));
        }
        let (init, frags) = fragments(&data).unwrap();
        assert_eq!(init, (0, init_end));
        assert_eq!(frags, vec![(init_end, 1040), (init_end + 1040, 1041), (init_end + 2081, 1042)]);
        // Cut short: refused.
        assert_eq!(fragments(&data[..data.len() - 1]), None);
        let mut avcc = mp4_box(b"moov", 8);
        avcc.extend(b"avcC\x01\x4d\x40\x1e\xff");
        assert_eq!(avc_codecs(&avcc).as_deref(), Some("avc1.4d401e"));
    }

    fn index(duration: f64, n: usize) -> Index {
        Index {
            format: FORMAT,
            interval: 10.0,
            width: 320,
            height: 180,
            columns: 10,
            rows: 10,
            count: n as u32,
            sheets: 1,
            duration,
            iframes: Some(IFrames {
                init: (0, 700),
                fragments: (0..n).map(|i| (700 + 5000 * i as u64, 5000)).collect(),
                codecs: "avc1.4d401e".into(),
            }),
            bytes: 0,
            read: 0,
            secs: 0.0,
        }
    }

    #[test]
    fn iframe_playlist_follows_the_pictures() {
        let p = iframe_playlist(&index(25.0, 3), "iframes.mp4").unwrap();
        assert!(p.contains("#EXT-X-I-FRAMES-ONLY\n"));
        assert!(p.contains("#EXT-X-MAP:URI=\"iframes.mp4\",BYTERANGE=\"700@0\"\n"));
        assert!(p.contains("#EXTINF:10.000,\n#EXT-X-BYTERANGE:5000@700\niframes.mp4\n"));
        // The last one lasts until the end.
        assert!(
            p.contains("#EXTINF:5.000,\n#EXT-X-BYTERANGE:5000@10700\niframes.mp4\n#EXT-X-ENDLIST\n"),
            "{p}"
        );
        let inf = iframe_stream_inf(&index(25.0, 3), "/v1/s/x/trickplay/iframes.m3u8").unwrap();
        assert_eq!(
            inf,
            "#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=4000,CODECS=\"avc1.4d401e\",RESOLUTION=320x180,URI=\"/v1/s/x/trickplay/iframes.m3u8\"\n"
        );
        // No track: no line.
        let mut i = index(25.0, 3);
        i.iframes = None;
        assert!(iframe_stream_inf(&i, "x").is_none() && iframe_playlist(&i, "x").is_none());
    }

    #[test]
    fn only_set_names_are_served() {
        let tp = Trickplay::new(Path::new("/c"), 1 << 30);
        assert_eq!(
            tp.file("0123456789abcdef", "3.jpg"),
            Some(PathBuf::from("/c/trickplay/0123456789abcdef/3.jpg"))
        );
        assert!(tp.file("0123456789abcdef", "iframes.mp4").is_some());
        for (k, n) in [
            ("../../etc", "3.jpg"),
            ("0123456789abcdef", "../x.jpg"),
            ("0123456789abcdef", "index.json"),
            ("0123456789abcdef", ".jpg"),
            ("0123456789abcdeg", "1.jpg"),
        ] {
            assert!(tp.file(k, n).is_none(), "{k} {n}");
        }
    }

    #[test]
    fn stale_sets_are_deleted() {
        let dir = std::env::temp_dir().join(format!("tp-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let keep = file_key("/m/a.mkv", 1, 2);
        let gone = file_key("/m/a.mkv", 1, 3);
        for d in [keep.clone(), gone.clone(), format!("{keep}.part")] {
            std::fs::create_dir_all(dir.join(&d)).unwrap();
            std::fs::write(dir.join(&d).join("index.json"), b"{}").unwrap();
        }
        assert_eq!(remove_stale(&dir, &[keep.clone()].into()), 2);
        assert!(dir.join(&keep).is_dir() && !dir.join(&gone).exists() && !dir.join(format!("{keep}.part")).exists());
        assert_eq!(dir_size(&dir), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mp4_keyframes_become_annex_b_with_their_parameter_sets() {
        // avcC: 4-byte lengths, one SPS, one PPS.
        let avcc = [&[1u8, 0x64, 0, 0x28, 0xff, 0xe1, 0, 3][..], b"SPS", &[1, 0, 2], b"PP"].concat();
        let (sets, len) = parameter_sets(b"avc1", &avcc).unwrap();
        assert_eq!((sets, len), (vec![b"SPS".to_vec(), b"PP".to_vec()], 4));
        // hvcC: 22 bytes of fields, 2-byte lengths, VPS and SPS arrays.
        let mut hvcc = vec![0u8; 21];
        hvcc.extend([0xfd, 2, 0x20, 0, 1, 0, 1, b'V', 0x21, 0, 2, 0, 1, b'S', 0, 2, b'S', b'2']);
        let (sets, len) = parameter_sets(b"hvc1", &hvcc).unwrap();
        assert_eq!((sets, len), (vec![b"V".to_vec(), b"S".to_vec(), b"S2".to_vec()], 2));
        assert!(parameter_sets(b"avc1", &avcc[..9]).is_none(), "cut short");
        // Two NAL units, each after its 4-byte length.
        let sample = [&[0u8, 0, 0, 2][..], b"ab", &[0, 0, 0, 3], b"cde"].concat();
        let mut out = Vec::new();
        annex_b(&sample, 4, &mut out).unwrap();
        assert_eq!(out, [&[0u8, 0, 0, 1][..], b"ab", &[0, 0, 0, 1], b"cde"].concat());
        assert!(annex_b(&sample[..sample.len() - 1], 4, &mut Vec::new()).is_none());
    }

    #[test]
    fn a_picture_takes_the_keyframe_at_or_before_it() {
        let k = |time| crate::media::moov::KeySample { time, offset: 0, size: 1 };
        let keys = [k(0.5), k(4.0), k(12.0), k(30.0)];
        assert_eq!(key_for(&keys, 0.0), 0);
        assert_eq!(key_for(&keys, 5.0), 1);
        assert_eq!(key_for(&keys, 12.0), 2);
        assert_eq!(key_for(&keys, 25.0), 2);
        assert_eq!(key_for(&keys, 1e6), 3);
    }

    fn quiet() -> Control<'static> {
        Control {
            busy: &|| false,
            on: &|| true,
            paused: &|_| {},
        }
    }

    /// A stand-in ffmpeg: logs its arguments, and writes its output, unless it is asked for a
    /// stream the one-packet `key.mkv` doesn't have (as ffmpeg refuses it), or for the copy when
    /// `copy_fails`.
    fn picture_ffmpeg(dir: &Path, copy_fails: bool) -> String {
        use std::os::unix::fs::PermissionsExt;
        let refuse_copy = if copy_fails { "exit 1" } else { ":" };
        let script = format!(
            "#!/bin/sh\necho \"$*\" >> '{log}'\nfor a; do out=$a; done\n\
             case \" $* \" in *' -c copy '*) {refuse_copy};; esac\n\
             case \" $* \" in *' -i {key} '*) case \" $* \" in *' -map 0:0 '*) ;; *)\n\
             echo \"Failed to set value for option 'map'\" >&2; exit 1;; esac;; esac\n\
             echo picture > \"$out\"\n",
            log = dir.join("log").display(),
            key = dir.join("key.mkv").display(),
        );
        let p = dir.join("ffmpeg");
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.to_string_lossy().to_string()
    }

    async fn one_picture(copy_fails: bool) -> (Result<u64>, Vec<String>) {
        let dir = std::env::temp_dir().join(format!("moonrise-picture-{}-{copy_fails}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ffmpeg = picture_ffmpeg(&dir, copy_fails);
        let mut mode = Mode {
            hdr: false,
            copy: true,
            keys: None,
        };
        // The video after the audio: stream 1 of the file.
        let r = picture(
            &ffmpeg,
            &Source::of("/media/film.mkv"),
            1,
            5.0,
            180,
            &mut mode,
            &dir.join("00000.jpg"),
        )
        .await;
        let log = std::fs::read_to_string(dir.join("log")).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        (r, log.lines().map(ToString::to_string).collect())
    }

    #[tokio::test]
    async fn a_picture_decodes_the_only_stream_of_the_copied_keyframe() {
        let (r, runs) = one_picture(false).await;
        r.unwrap();
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert!(
            runs[0].contains("-map 0:1 -c copy"),
            "the copy takes the file's stream: {}",
            runs[0]
        );
        assert!(runs[1].contains("-map 0:0 "), "key.mkv holds one stream: {}", runs[1]);
    }

    #[tokio::test]
    async fn a_picture_decoded_from_the_file_keeps_the_files_stream() {
        let (r, runs) = one_picture(true).await;
        r.unwrap();
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert!(runs[1].contains("-skip_frame nokey"), "{}", runs[1]);
        assert!(runs[1].contains("-map 0:1 "), "from the file itself: {}", runs[1]);
    }

    #[tokio::test]
    async fn a_run_that_floods_stderr_never_hangs() {
        // 2 MB of errors (far more than a pipe holds), then a failure: read as it comes.
        let args: Vec<String> = [
            "-c",
            "head -c 2000000 /dev/zero | tr '\\0' 'e' >&2; echo the last error >&2; exit 3",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        let r = tokio::time::timeout(
            Duration::from_secs(20),
            run_yielding("sh", &args, Duration::from_secs(20), &quiet(), &mut || {}),
        )
        .await
        .expect("hung");
        let e = format!("{:#}", r.unwrap_err());
        assert!(e.contains("the last error"), "{e}");
        let ok: Vec<String> = ["-c", "head -c 2000000 /dev/zero >&2"].iter().map(ToString::to_string).collect();
        run_yielding("sh", &ok, Duration::from_secs(20), &quiet(), &mut || {})
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_run_is_given_up_after_its_limit_and_ended_when_switched_off() {
        let sleep: Vec<String> = vec!["30".into()];
        let t = Instant::now();
        let e = run_yielding("sleep", &sleep, Duration::from_secs(2), &quiet(), &mut || {})
            .await
            .unwrap_err();
        assert!(format!("{e:#}").contains("gave up"), "{e:#}");
        assert!(t.elapsed() < Duration::from_secs(10));
        let off = Control {
            busy: &|| false,
            on: &|| false,
            paused: &|_| {},
        };
        let e = run_yielding("sleep", &sleep, Duration::from_secs(60), &off, &mut || {})
            .await
            .unwrap_err();
        assert!(e.is::<Off>());
        // Stopped while something plays: that time doesn't count against the limit.
        let started = Instant::now();
        let busy_for_3s = move || started.elapsed() < Duration::from_secs(3);
        let paused = std::sync::Mutex::new(Vec::new());
        let ctl = Control {
            busy: &busy_for_3s,
            on: &|| true,
            paused: &|p| paused.lock().unwrap().push(p),
        };
        // (Counted, the 2 s stopped would pass the 1.5 s limit.)
        let short: Vec<String> = vec!["2".into()];
        run_yielding("sleep", &short, Duration::from_millis(1500), &ctl, &mut || {})
            .await
            .unwrap();
        assert_eq!(*paused.lock().unwrap(), vec![true, false]);
        // An AVI pass: three times the file's reading time at the pace, and 10 minutes.
        assert_eq!(whole_pass_limit(800 << 20), Duration::from_mins(5 + 10));
    }

    #[test]
    fn a_full_budget_deletes_the_sets_used_longest_ago() {
        let dir = std::env::temp_dir().join(format!("tp-evict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let now = std::time::SystemTime::now();
        let day = Duration::from_hours(24);
        for (name, age) in [("a", 3), ("b", 1), ("c", 2)] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            std::fs::write(dir.join(name).join("0.jpg"), vec![0u8; 1000]).unwrap();
            let idx = dir.join(name).join("index.json");
            std::fs::write(&idx, b"{}").unwrap();
            std::fs::File::options()
                .write(true)
                .open(&idx)
                .unwrap()
                .set_modified(now - day * age)
                .unwrap();
        }
        std::fs::create_dir_all(dir.join("d.part")).unwrap();
        // A player asking for "a" makes it the most recently used.
        mark_used(&dir.join("a").join("index.json"), now);
        let (n, freed) = evict(&dir, 1500);
        assert_eq!((n, freed), (2, 2004));
        assert!(dir.join("a").exists() && !dir.join("b").exists() && !dir.join("c").exists() && dir.join("d.part").exists());
        // Used within the hour: not written again.
        let before = std::fs::metadata(dir.join("a").join("index.json")).unwrap().modified().unwrap();
        mark_used(&dir.join("a").join("index.json"), now + Duration::from_secs(60));
        assert_eq!(
            std::fs::metadata(dir.join("a").join("index.json")).unwrap().modified().unwrap(),
            before
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn hints_go_first_newest_hint_first() {
        let tp = Trickplay::new(Path::new("/c"), 1);
        tp.hint(vec!["a".into(), "b".into()]);
        tp.hint(vec!["c".into(), "a".into()]);
        assert_eq!(tp.hints.lock().unwrap().iter().cloned().collect::<Vec<_>>(), ["c", "a", "b"]);
    }
}
