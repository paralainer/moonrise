//! Keyframe index for containers without a usable index of their own here (AVI, MPEG-PS/TS,
//! WMV…): a one-time ffprobe packet scan, which reads the whole file. Until it is done the title
//! is reported as not served yet ("pending"). The result goes to the probe cache in the
//! database, so it is never redone for an unchanged file.
//!
//! Two kinds of scan:
//! - **For playback** (`Scanner::request`): someone wants to play the title now. One at a time,
//!   as fast as the file reads.
//! - **Ahead of need** (`Scanner::background`, from the backfill in `index_backfill.rs`): at a
//!   pace (`BACKGROUND_RATE` from a share), `nice`d, and stopped (SIGSTOP) while anything plays
//!   or a scan for playback runs. A title played while its scan runs in the background takes it
//!   over: the scan goes on at full speed and is never stopped again.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use tokio::sync::Semaphore;

use crate::{db::Db, storage::source::Source, storage::vfs};

/// A scan that failed is retried after this long.
const RETRY_AFTER: Duration = Duration::from_secs(600);
/// A scan taking longer than this is given up (a 40 GB file over the share is a few minutes).
const SCAN_TIMEOUT: Duration = Duration::from_mins(30);
/// How fast a background scan reads a file on a share (bytes a second): a 350 MB episode in
/// about 35 s, a 3 GB film in 5 minutes.
pub const BACKGROUND_RATE: u64 = 10 << 20;

#[derive(Clone)]
enum State {
    Pending,
    Done(Arc<Vec<f64>>),
    Failed(String, Instant),
}

pub enum Status {
    Ready(Vec<f64>),
    Pending,
    Failed(String),
}

/// Where a finished scan is stored: the probe cache row of (path, size, mtime).
#[derive(Clone)]
pub struct Stamp {
    pub path: String,
    pub size: u64,
    pub mtime: i64,
}

impl Stamp {
    /// The scan's key: the file as it is now, and the stream scanned.
    pub fn key(&self, stream: i64) -> String {
        format!("{}|{}|{}|{stream}", self.path, self.size, self.mtime)
    }
}

/// A background scan running now.
struct Background {
    /// Someone wants to play it: full speed, never stopped.
    urgent: AtomicBool,
    pace: vfs::Pace,
    /// The file's size.
    size: u64,
}

/// Something is playing or being read for playback: background scans wait.
pub type Busy = Arc<dyn Fn() -> bool + Send + Sync>;

pub struct Scanner {
    ffprobe: String,
    db: Option<Db>,
    state: Mutex<HashMap<String, State>>,
    /// One scan for playback at a time: they read whole files from the shared NAS.
    slot: Semaphore,
    /// Scans for playback waiting or running: background scans stop meanwhile.
    urgent: AtomicUsize,
    /// Background scans running, by key.
    background: Mutex<HashMap<String, Arc<Background>>>,
}

impl Scanner {
    pub fn new(ffprobe: String, db: Option<Db>) -> Self {
        Scanner {
            ffprobe,
            db,
            state: Mutex::new(HashMap::new()),
            slot: Semaphore::new(1),
            urgent: AtomicUsize::new(0),
            background: Mutex::new(HashMap::new()),
        }
    }

    /// A scan for playback is waiting or running.
    pub fn scanning_for_playback(&self) -> bool {
        self.urgent.load(Ordering::SeqCst) > 0
    }

    /// The index for `key` (a file and version), starting a scan of `stream` if there is none.
    /// `timing`: how the file times its pictures (see `parse`). A background scan of the same
    /// key is hurried instead.
    pub fn request(self: &Arc<Self>, key: String, source: &Source, stream: i64, timing: Timing, stamp: Option<Stamp>) -> Status {
        let mut state = self.state.lock().unwrap();
        match state.get(&key) {
            Some(State::Done(kf)) => return Status::Ready(kf.to_vec()),
            Some(State::Pending) => {
                if let Some(b) = self.background.lock().unwrap().get(&key) {
                    if !b.urgent.swap(true, Ordering::SeqCst) {
                        b.pace.set_rate(0);
                        tracing::info!(key, "keyframe scan: wanted for playback, no longer in the background");
                    }
                }
                return Status::Pending;
            }
            Some(State::Failed(e, at)) if at.elapsed() < RETRY_AFTER => return Status::Failed(e.clone()),
            _ => {}
        }
        state.insert(key.clone(), State::Pending);
        let (this, source) = (self.clone(), source.clone());
        self.urgent.fetch_add(1, Ordering::SeqCst);
        tokio::spawn(async move {
            let t0 = Instant::now();
            let result = match this.slot.acquire().await {
                Ok(_permit) => tokio::time::timeout(SCAN_TIMEOUT, scan(&this.ffprobe, &source, stream, timing))
                    .await
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("scan timed out"))),
                Err(e) => Err(e.into()),
            };
            this.urgent.fetch_sub(1, Ordering::SeqCst);
            let next = match result {
                Ok(kf) => {
                    tracing::info!(
                        key,
                        keyframes = kf.len(),
                        ms = t0.elapsed().as_millis() as u64,
                        "keyframe scan done"
                    );
                    this.store(stamp, &kf).await;
                    State::Done(Arc::new(kf))
                }
                Err(e) => {
                    tracing::warn!(key, "keyframe scan failed: {e:#}");
                    State::Failed(format!("{e:#}"), Instant::now())
                }
            };
            this.state.lock().unwrap().insert(key, next);
        });
        Status::Pending
    }

    async fn store(&self, stamp: Option<Stamp>, kf: &[f64]) {
        if let (Some(db), Some(s)) = (&self.db, stamp) {
            if let Err(e) = db.keyframes_put(s.path, s.size, s.mtime, kf.to_vec()).await {
                tracing::warn!("keyframe cache write: {e:#}");
            }
        }
    }

    /// Scans `stream` of the file of `stamp` ahead of need and stores the index, at a pace and
    /// stopped while `busy` says so; given up as soon as `wanted` says it no longer is (the
    /// switch was turned off). Ok(None): nothing done (a scan of it is already running, or
    /// it was given up). Runs in the caller's task.
    pub async fn background(
        &self,
        source: &Source,
        stream: i64,
        timing: Timing,
        stamp: Stamp,
        busy: Busy,
        wanted: impl Fn() -> bool,
    ) -> Result<Option<usize>> {
        let key = stamp.key(stream);
        {
            let mut state = self.state.lock().unwrap();
            match state.get(&key) {
                Some(State::Pending) => return Ok(None),
                Some(State::Done(kf)) => return Ok(Some(kf.len())),
                _ => {}
            }
            state.insert(key.clone(), State::Pending);
        }
        let job = Arc::new(Background {
            urgent: AtomicBool::new(false),
            pace: vfs::Pace::new(BACKGROUND_RATE),
            size: stamp.size,
        });
        self.background.lock().unwrap().insert(key.clone(), job.clone());
        let t0 = Instant::now();
        let result = scan_paced(&self.ffprobe, source, stream, timing, &job, &busy, &wanted).await;
        self.background.lock().unwrap().remove(&key);
        let urgent = job.urgent.load(Ordering::SeqCst);
        let mut state = self.state.lock().unwrap();
        match result {
            Ok(Some(kf)) => {
                tracing::info!(
                    key,
                    keyframes = kf.len(),
                    ms = t0.elapsed().as_millis() as u64,
                    read = job.pace.sent(),
                    urgent,
                    "keyframe scan done (ahead of need)"
                );
                let n = kf.len();
                drop(state);
                self.store(Some(stamp), &kf).await;
                let mut state = self.state.lock().unwrap();
                if urgent || self.db.is_none() {
                    // Someone is waiting for it (the next request finds it in the database
                    // too, but the media may not be cached there).
                    state.insert(key, State::Done(Arc::new(kf)));
                } else {
                    // In the database now: nothing to keep in memory.
                    state.remove(&key);
                }
                Ok(Some(n))
            }
            Ok(None) => {
                state.remove(&key);
                Ok(None)
            }
            Err(e) => {
                tracing::warn!(key, "keyframe scan failed (ahead of need): {e:#}");
                // Not remembered here: a play tries again at once (the backfill keeps its own
                // list of failures).
                state.remove(&key);
                Err(e)
            }
        }
    }
}

/// Presentation times of the keyframes of `stream`, from every packet's timestamps, size and flags.
async fn scan(ffprobe: &str, source: &Source, stream: i64, timing: Timing) -> Result<Vec<f64>> {
    let mut cmd = tokio::process::Command::new("nice");
    cmd.args(["-n", "10", ffprobe, "-v", "error", "-select_streams", &stream.to_string()])
        .args(["-show_entries", "packet=pts_time,dts_time,size,flags", "-of", "compact=p=0"])
        .args(source.input_args());
    let out = cmd.kill_on_drop(true).output().await.context("ffprobe")?;
    if !out.status.success() {
        bail!(
            "ffprobe: {}",
            crate::admin::logbuf::scrub(String::from_utf8_lossy(&out.stderr).trim())
        );
    }
    parse(&String::from_utf8_lossy(&out.stdout), timing)
}

/// `scan` in the background: at the job's pace, `nice`d further, stopped (SIGSTOP) while `busy`
/// (unless the job became urgent). Ok(None): given up because it is no longer `wanted`. Given up
/// with an error after `SCAN_TIMEOUT`, or the time the file takes at the pace if longer, of not
/// being stopped.
async fn scan_paced(
    ffprobe: &str,
    source: &Source,
    stream: i64,
    timing: Timing,
    job: &Background,
    busy: &Busy,
    wanted: &impl Fn() -> bool,
) -> Result<Option<Vec<f64>>> {
    const TICK: Duration = Duration::from_millis(200);
    let input = vfs::ffmpeg_input_args_paced(&source.path(), &job.pace);
    let child = tokio::process::Command::new("nice")
        .args(["-n", "19", ffprobe, "-v", "error", "-select_streams", &stream.to_string()])
        .args(["-show_entries", "packet=pts_time,dts_time,size,flags", "-of", "compact=p=0"])
        .args(&input)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("ffprobe")?;
    let pid = child.id();
    let wait = child.wait_with_output();
    tokio::pin!(wait);
    let limit = SCAN_TIMEOUT.max(Duration::from_secs_f64(3.0 * job.size as f64 / BACKGROUND_RATE as f64));
    let mut running = Duration::ZERO;
    let mut stopped = false;
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let out = loop {
        tokio::select! {
            o = &mut wait => break o.context("ffprobe")?,
            _ = tick.tick() => {
                if !wanted() {
                    // Dropping `wait` kills ffprobe (a stopped one too).
                    if stopped {
                        if let Some(pid) = pid {
                            // SAFETY: kill only takes integers. `pid` is our ffprobe child, which isn't reaped before
                            // `wait` completes, so the id is still its.
                            unsafe { libc::kill(pid as i32, libc::SIGCONT); }
                        }
                    }
                    return Ok(None);
                }
                let go = job.urgent.load(Ordering::SeqCst) || !busy();
                if let Some(pid) = pid {
                    if go == stopped {
                        // SAFETY: as above: our unreaped ffprobe child.
                        unsafe { libc::kill(pid as i32, if go { libc::SIGCONT } else { libc::SIGSTOP }); }
                        stopped = !go;
                    }
                }
                if go {
                    running += TICK;
                    anyhow::ensure!(running < limit, "scan timed out");
                }
            }
        }
    };
    if !out.status.success() {
        bail!(
            "ffprobe: {}",
            crate::admin::logbuf::scrub(String::from_utf8_lossy(&out.stderr).trim())
        );
    }
    parse(&String::from_utf8_lossy(&out.stdout), timing).map(Some)
}

/// Packets this small hold no picture: the placeholders ("N-VOPs") of MPEG-4 Part 2 streams
/// with packed B-frames (DivX 5, Xvid), one after each packed pair. A real picture is far bigger
/// (a P-frame with nothing coded is over 100 bytes at SD sizes).
const PLACEHOLDER_MAX: u64 = 15;

struct Packet {
    pts: Option<f64>,
    dts: Option<f64>,
    size: Option<u64>,
    key: bool,
}

impl Packet {
    fn placeholder(&self) -> bool {
        self.size.is_some_and(|s| s <= PLACEHOLDER_MAX)
    }
}

/// How a stream's pictures are timed, for `parse`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timing {
    /// The container stores presentation times (or the decode time is the one there is).
    Pts,
    /// AVI.
    Avi,
    /// AVI, MPEG-4 Part 2 whose decoder holds a picture back (`has_b_frames` > 0: the VOL says
    /// the stream may have B-frames, whether or not any are coded).
    AviDelayed,
}

/// `pts_time=…|dts_time=…|size=…|flags=K__` lines; each keyframe's presentation time: its pts, or
/// where it has none, its dts, except in AVI.
///
/// AVI stores no presentation times. Where the decoder holds a picture back (MPEG-4 Part 2 that
/// may have B-frames), a reference picture (I or P) is shown only when the next one is decoded,
/// and is stamped with that packet's decode time. ffmpeg gives the packets it can time a pts:
/// the B-frames it sees, shown as soon as they are decoded, at their own decode time. The I- and
/// P-frames, and every packet of a packed stream whose B-frames hide inside P packets (Xvid), get
/// none. So a keyframe without a pts is shown at the decode time of the next picture without a
/// pts (`Timing::AviDelayed`, or any AVI stream where some packets have a pts and some don't).
/// Its own decode time is a frame or more earlier, and the pictures in between (the last picture
/// of the previous GOP, and B-frames that refer to it) can't be decoded after a seek to the
/// keyframe: a segment starting there would start that much late. Elsewhere (H.264 in AVI, no
/// pts on any packet) the decode time is all there is.
fn parse(text: &str, timing: Timing) -> Result<Vec<f64>> {
    let avi = timing != Timing::Pts;
    let mut packets = Vec::new();
    for line in text.lines() {
        let mut p = Packet {
            pts: None,
            dts: None,
            size: None,
            key: false,
        };
        for field in line.split('|') {
            match field.split_once('=') {
                Some(("pts_time", v)) => p.pts = v.parse::<f64>().ok(),
                Some(("dts_time", v)) => p.dts = v.parse::<f64>().ok(),
                Some(("size", v)) => p.size = v.parse::<u64>().ok(),
                Some(("flags", v)) => p.key = v.starts_with('K'),
                _ => {}
            }
        }
        packets.push(p);
    }
    let mixed = avi && packets.iter().any(|p| p.pts.is_some()) && packets.iter().any(|p| p.pts.is_none() && p.dts.is_some());
    let delayed = timing == Timing::AviDelayed || mixed;
    let mut kf = Vec::new();
    for (i, p) in packets.iter().enumerate() {
        // A placeholder can be flagged as a keyframe (the one after a packed I-frame).
        if !p.key || (avi && p.placeholder()) {
            continue;
        }
        let shown = match p.pts {
            Some(t) => Some(t),
            None if delayed => packets[i + 1..]
                .iter()
                .find(|q| q.pts.is_none() && q.dts.is_some() && !q.placeholder())
                .and_then(|q| q.dts)
                .or(p.dts),
            None => p.dts,
        };
        if let Some(t) = shown {
            kf.push(t);
        }
    }
    kf.sort_by(|a, b| a.partial_cmp(b).unwrap());
    kf.dedup();
    if kf.len() < 2 {
        bail!("too few keyframes ({})", kf.len());
    }
    Ok(kf)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for ffprobe: two keyframes, half a second apart.
    fn fake_ffprobe(dir: &std::path::Path) -> String {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("ffprobe");
        std::fs::write(&p, "#!/bin/sh\necho 'pts_time=0.000000|dts_time=0.000000|size=900|flags=K__'\nsleep 0.5\necho 'pts_time=2.000000|dts_time=2.000000|size=900|flags=K__'\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.to_string_lossy().into_owned()
    }

    fn setup(name: &str) -> (std::path::PathBuf, Arc<Scanner>, Db, Stamp, Source) {
        let dir = std::env::temp_dir().join(format!("moonrise-scan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::memory().unwrap();
        let scanner = Arc::new(Scanner::new(fake_ffprobe(&dir), Some(db.clone())));
        let path = dir.join("a.avi").to_string_lossy().into_owned();
        std::fs::write(&path, b"x").unwrap();
        let stamp = Stamp {
            path: path.clone(),
            size: 1,
            mtime: 5,
        };
        (dir, scanner, db, stamp, Source::of(&path))
    }

    #[tokio::test]
    async fn background_scan_is_stored_and_waits_while_busy() {
        let (dir, scanner, db, stamp, source) = setup("bg");
        db.probe_put(stamp.path.clone(), 1, 5, serde_json::json!({})).await.unwrap();
        // Busy for the first 1.5 s: stopped meanwhile.
        let t0 = Instant::now();
        let busy: Busy = Arc::new(move || t0.elapsed() < Duration::from_millis(1500));
        let n = scanner
            .background(&source, 0, Timing::Pts, stamp.clone(), busy, || true)
            .await
            .unwrap();
        assert_eq!(n, Some(2));
        assert!(
            t0.elapsed() >= Duration::from_millis(1500),
            "it waited while busy: {:?}",
            t0.elapsed()
        );
        assert_eq!(
            db.probe_get(stamp.path.clone(), 1, 5).await.unwrap().unwrap().keyframes.unwrap(),
            vec![0.0, 2.0]
        );
        // Stored: nothing kept in memory.
        assert!(scanner.state.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn background_scan_given_up_when_switched_off() {
        let (dir, scanner, _db, stamp, source) = setup("off");
        let t0 = Instant::now();
        let busy: Busy = Arc::new(|| false);
        let n = scanner
            .background(&source, 0, Timing::Pts, stamp, busy, move || {
                t0.elapsed() < Duration::from_millis(100)
            })
            .await
            .unwrap();
        assert_eq!(n, None);
        assert!(scanner.state.lock().unwrap().is_empty(), "a later play scans it");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn playback_takes_a_background_scan_over() {
        let (dir, scanner, db, stamp, source) = setup("urgent");
        db.probe_put(stamp.path.clone(), 1, 5, serde_json::json!({})).await.unwrap();
        // Always busy: only a request for playback lets it go on.
        let busy: Busy = Arc::new(|| true);
        let (s2, src2, st2) = (scanner.clone(), source.clone(), stamp.clone());
        let bg = tokio::spawn(async move { s2.background(&src2, 0, Timing::Pts, st2, busy, || true).await.unwrap() });
        tokio::time::sleep(Duration::from_millis(400)).await;
        let key = stamp.key(0);
        assert!(matches!(
            scanner.request(key.clone(), &source, 0, Timing::Pts, Some(stamp.clone())),
            Status::Pending
        ));
        assert!(!scanner.scanning_for_playback(), "no second scan started");
        assert_eq!(tokio::time::timeout(Duration::from_secs(5), bg).await.unwrap().unwrap(), Some(2));
        // The one waiting finds it.
        assert!(matches!(scanner.request(key, &source, 0, Timing::Pts, Some(stamp)), Status::Ready(k) if k == vec![0.0, 2.0]));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn parses_packets() {
        let out = "pts_time=0.000000|dts_time=0.000000|flags=K__\npts_time=N/A|dts_time=0.040000|flags=___\n\
                   pts_time=N/A|dts_time=10.000000|flags=K__\npts_time=5.005000|dts_time=4.963000|flags=K_D\n";
        assert_eq!(parse(out, Timing::Pts).unwrap(), vec![0.0, 5.005, 10.0]);
        assert!(parse("pts_time=1|flags=K__\n", Timing::Pts).is_err());
    }

    fn packets(rows: &[(&str, f64, u64, bool)]) -> String {
        rows.iter()
            .map(|(pts, dts, size, key)| {
                format!(
                    "pts_time={pts}|dts_time={dts:.6}|size={size}|flags={}\n",
                    if *key { "K__" } else { "___" }
                )
            })
            .collect()
    }

    #[test]
    fn avi_keyframes_are_shown_when_the_next_picture_is_decoded() {
        // Xvid, two B-frames, open GOPs (25 fps): the I-frame decoded at 10.00 is shown at 10.12,
        // after the previous GOP's last P (10.00) and two B-frames that refer to it.
        let out = packets(&[
            ("N/A", 0.0, 23395, true),
            ("N/A", 0.04, 18117, false),
            ("0.08", 0.08, 10085, false),
            ("N/A", 9.88, 16567, false),
            ("9.92", 9.92, 10269, false),
            ("9.96", 9.96, 11098, false),
            ("N/A", 10.0, 25648, true),
            ("10.04", 10.04, 9675, false),
            ("10.08", 10.08, 11481, false),
            ("N/A", 10.12, 18203, false),
            ("10.16", 10.16, 10860, false),
            // The last keyframe, with no picture after it: its decode time.
            ("N/A", 12.0, 20000, true),
        ]);
        assert_eq!(parse(&out, Timing::Avi).unwrap(), vec![0.04, 10.12, 12.0]);
        // Not AVI: the decode time where there's no pts, as before.
        assert_eq!(parse(&out, Timing::Pts).unwrap(), vec![0.0, 10.0, 12.0]);
    }

    #[test]
    fn avi_packed_b_frames() {
        // Packed bitstream (DivX 5): the 7-byte placeholders hold no picture, and the one after the
        // I-frame is flagged as a keyframe too.
        let out = packets(&[
            ("N/A", 0.0, 9000, true),
            ("N/A", 0.0417, 2800, false),
            ("N/A", 4.921583, 2868, false),
            ("4.963292", 4.963292, 547, false),
            ("N/A", 5.005, 7, false),
            ("N/A", 5.046708, 7916, true),
            ("5.088417", 5.088417, 670, false),
            ("N/A", 5.130125, 7, true),
            ("N/A", 5.171833, 2751, false),
        ]);
        assert_eq!(parse(&out, Timing::Avi).unwrap(), vec![0.0417, 5.171833]);
    }

    #[test]
    fn avi_without_b_frames_or_without_any_pts() {
        // No B-frames: every packet has its pts.
        let plain = packets(&[
            ("0", 0.0, 9000, true),
            ("0.04", 0.04, 900, false),
            ("2", 2.0, 9000, true),
            ("2.04", 2.04, 900, false),
        ]);
        assert_eq!(parse(&plain, Timing::Avi).unwrap(), vec![0.0, 2.0]);
        // No pts at all (H.264 in AVI): the decode times.
        let none = packets(&[
            ("N/A", 0.0, 9000, true),
            ("N/A", 0.04, 900, false),
            ("N/A", 2.0, 9000, true),
            ("N/A", 2.04, 900, false),
        ]);
        assert_eq!(parse(&none, Timing::Avi).unwrap(), vec![0.0, 2.0]);
    }

    #[test]
    fn avi_delayed_without_any_pts() {
        // Packed Xvid where ffmpeg times no packet (The A-Team, 25 fps): the B-frames hide in the P
        // packets, each followed by a 6-byte placeholder. The I-frame decoded at 8.56 is shown when
        // the next picture is decoded, at 8.60; the one at 11.48 is followed by a placeholder.
        let out = packets(&[
            ("N/A", 0.0, 2332, true),
            ("N/A", 0.04, 176, false),
            ("N/A", 0.08, 6, false),
            ("N/A", 8.52, 6, false),
            ("N/A", 8.56, 9800, true),
            ("N/A", 8.6, 455, false),
            ("N/A", 8.64, 6, false),
            ("N/A", 11.48, 9100, true),
            ("N/A", 11.52, 6, true),
            ("N/A", 11.56, 400, false),
        ]);
        assert_eq!(parse(&out, Timing::AviDelayed).unwrap(), vec![0.04, 8.6, 11.56]);
        // Xvid with B-frames allowed but none coded: the decoder still holds each picture back.
        let no_b = packets(&[
            ("N/A", 0.0, 9000, true),
            ("N/A", 0.08, 900, false),
            ("N/A", 2.04, 9000, true),
            ("N/A", 2.08, 900, false),
        ]);
        assert_eq!(parse(&no_b, Timing::AviDelayed).unwrap(), vec![0.08, 2.08]);
    }
}
