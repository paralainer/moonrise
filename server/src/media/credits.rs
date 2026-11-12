//! End credits found in an episode's file itself, for the apps' Skip Credits and Next Episode.
//!
//! The community databases the apps ask (TheIntroDB, IntroDB) time the credits on whatever copy
//! their contributor had. On another cut those times land in the story: for The Sopranos one of
//! them is two minutes early on most episodes. So the server looks at the file once: the
//! keyframes of the last `WINDOW` seconds are decoded (only those: a few dozen pictures, cheap)
//! and measured for how dark they are. End credits in most shows are text on black that runs to
//! the end, perhaps with a few seconds of studio logos after it (`find`).
//!
//! It runs the first time an episode's segments are asked for (when it starts playing), in the
//! background at a slow pace from a share, and the result is kept with the file's probe
//! (`files_probe.credits`) while the file is unchanged. The apps ask again near the end if the
//! answer was still pending. Credits not on a dark background (much animation, art cards) are
//! simply not found: the apps then go by the databases, checked against the file's length, or
//! offer Next Episode only in the last half minute.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::{api::AppState, db::Db, storage::source::Source, storage::vfs};

/// How much of the end is looked at (seconds).
pub const WINDOW: f64 = 300.0;
/// A keyframe is dark when at least this percentage of its pixels are (luma below 32 of 255).
/// Credits text covers a few percent; letterboxed or dimly lit story stays below.
const DARK: u32 = 85;
/// The dark stretch must reach this close to the end (studio logos may follow it).
const TAIL: f64 = 25.0;
/// Light pictures inside the credits (a logo, a card) may last this long between dark ones.
const GAP: f64 = 6.0;
/// The dark stretch must last this long: shorter ones are fades and title cards.
const MIN_LEN: f64 = 20.0;
/// Bump when `find` or what it is given changes, so stored results are made again (2: the
/// video's own end, not the file's).
pub const VERSION: u64 = 2;
/// How fast a file on a share is read for this (bytes a second): it runs while the episode plays.
const RATE: u64 = 4 << 20;
/// Wait this long after the request before reading, so the episode's own start comes first.
const DELAY: Duration = Duration::from_secs(20);
/// A file whose analysis failed is tried again after this long (or after a restart).
const RETRY_FAILED: Duration = Duration::from_hours(6);
/// An analysis taking longer than this is given up (a large file read at `RATE`, or a stall).
const TIMEOUT: Duration = Duration::from_mins(15);

/// Where the end credits start, from the darkness of the keyframes (time, percentage of dark
/// pixels) of the last `WINDOW` seconds of a file of `duration`: the earliest start of a dark
/// stretch that lasts `MIN_LEN`, has no light gap over `GAP`, and reaches into the last `TAIL`
/// seconds of the video. None when there's none: a later answer is never story, an earlier one
/// could be.
#[allow(clippy::float_cmp, reason = "dedup of identical timestamps, copied from the same index")]
pub fn find(frames: &[(f64, u32)], duration: f64) -> Option<f64> {
    let mut kf: Vec<(f64, u32)> = frames
        .iter()
        .copied()
        .filter(|(t, _)| *t >= duration - WINDOW && *t <= duration)
        .collect();
    kf.sort_by(|a, b| a.0.total_cmp(&b.0));
    kf.dedup_by(|a, b| a.0 == b.0);
    if kf.len() < 3 {
        return None;
    }
    let video_end = video_end(&kf, duration);
    // Each keyframe stands for the time until the next one.
    let spans: Vec<(f64, f64, bool)> = kf
        .iter()
        .enumerate()
        .map(|(i, &(t, p))| (t, kf.get(i + 1).map_or(video_end, |n| n.0), p >= DARK))
        .collect();
    let mut best: Option<f64> = None;
    for (j, &(_, end, dark)) in spans.iter().enumerate() {
        if !dark || end < video_end - TAIL {
            continue;
        }
        let (mut first, mut gap) = (j, 0.0);
        for i in (0..j).rev() {
            let (s, e, d) = spans[i];
            if d {
                first = i;
                gap = 0.0;
            } else {
                gap += e - s;
                if gap > GAP {
                    break;
                }
            }
        }
        let start = spans[first].0;
        if end - start >= MIN_LEN && best.is_none_or(|b| start < b) {
            best = Some(start);
        }
    }
    best
}

/// Where the video ends, from its keyframes (sorted, at least two): a usual keyframe interval
/// after the last one, at most the file's `duration`. The video can stop well before the file
/// does (audio or subtitles running on, 12–31 s on some Sopranos files): its last keyframe's
/// picture doesn't last until the file's end.
fn video_end(kf: &[(f64, u32)], duration: f64) -> f64 {
    let mut steps: Vec<f64> = kf.windows(2).map(|w| w[1].0 - w[0].0).collect();
    steps.sort_by(f64::total_cmp);
    let usual = steps[steps.len() / 2].clamp(1.0, 10.0);
    duration.min(kf[kf.len() - 1].0 + usual)
}

/// The keyframes ffmpeg's `blackframe` filter reported: (time, percentage of dark pixels).
pub fn parse(stderr: &str) -> Vec<(f64, u32)> {
    stderr
        .lines()
        .filter_map(|l| {
            let field = |name: &str| l.split_whitespace().find_map(|w| w.strip_prefix(name));
            Some((field("t:")?.parse().ok()?, field("pblack:")?.parse().ok()?))
        })
        .collect()
}

/// The keyframes of the last `WINDOW` seconds of video stream `stream` and how dark they are,
/// timed from the file's start (`start`: its first timestamp, which players count as 0). One
/// ffmpeg run that decodes only the keyframes, `nice`d, a share read at `RATE`.
pub async fn measure(ffmpeg: &str, source: &Source, stream: i64, duration: f64, start: f64) -> Result<Vec<(f64, u32)>> {
    let pace = vfs::Pace::new(RATE);
    let input = match source {
        Source::Smb(p) => vfs::ffmpeg_input_args_paced(p, &pace),
        Source::Local(_) => source.input_args(),
    };
    let from = (duration - WINDOW).max(0.0);
    let out = tokio::process::Command::new("nice")
        .args([
            "-n",
            "19",
            ffmpeg,
            "-hide_banner",
            "-nostats",
            "-nostdin",
            "-skip_frame",
            "nokey",
            "-copyts",
            "-ss",
            &format!("{from:.3}"),
        ])
        .args(&input)
        .args([
            "-map",
            &format!("0:{stream}"),
            "-an",
            "-sn",
            "-dn",
            "-vf",
            "scale=160:-2,blackframe=amount=0:threshold=32",
            "-f",
            "null",
            "-",
        ])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(TIMEOUT, out).await.context("timed out")?.context("ffmpeg")?;
    let err = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        bail!(
            "ffmpeg: {}",
            crate::admin::logbuf::scrub(err.lines().rev().take(3).collect::<Vec<_>>().join(" / ").trim())
        );
    }
    // `-copyts` keeps the file's own timestamps (`-ss` counts from its start).
    let frames: Vec<(f64, u32)> = parse(&err).into_iter().map(|(t, p)| (t - start, p)).collect();
    if frames.is_empty() {
        bail!("no keyframes in the last {WINDOW} s");
    }
    Ok(frames)
}

/// What is stored per file: the version of `find` and its answer.
pub fn stored(start: Option<f64>) -> Value {
    json!({"v": VERSION, "start": start})
}

/// A stored answer of the current version: Some(start or None), or None if there's none.
pub fn read(stored: Option<&str>) -> Option<Option<f64>> {
    let v: Value = serde_json::from_str(stored?).ok()?;
    (v["v"].as_u64() == Some(VERSION)).then(|| v["start"].as_f64())
}

/// The file to look at for an episode.
#[derive(Clone, Debug)]
pub struct Job {
    pub path: String,
    pub size: i64,
    pub mtime: i64,
    pub stream: i64,
    pub duration: f64,
    /// The file's first timestamp (`format.start_time`), which players count as 0.
    pub start: f64,
}

/// How many files are looked at at once: each reads from the share, and a season's download
/// asks for all its episodes together. The rest wait their turn (and are "pending").
const AT_ONCE: usize = 1;

/// The analyses asked for (waiting or running), and the ones that failed recently.
pub struct Credits {
    queued: Mutex<HashSet<String>>,
    failed: Mutex<HashMap<String, Instant>>,
    turns: tokio::sync::Semaphore,
    /// How long a request waits before it queues (`DELAY`).
    delay: Duration,
}

impl Default for Credits {
    fn default() -> Self {
        Self {
            queued: Mutex::default(),
            failed: Mutex::default(),
            turns: tokio::sync::Semaphore::new(AT_ONCE),
            delay: DELAY,
        }
    }
}

impl Credits {
    /// Starts looking at a file in the background (after the ones asked for before it, `AT_ONCE`
    /// at a time), unless it is asked for already or failed recently. Returns whether an answer
    /// may still come (the caller says "pending" then).
    pub fn request(self: &Arc<Self>, st: &Arc<AppState>, job: Job) -> bool {
        let st = st.clone();
        self.request_with(job, move |job| async move { run(&st, &job).await })
    }

    /// `request`, with `look` doing the looking (the tests' stand-in for ffmpeg and the database).
    fn request_with<F, Fut>(self: &Arc<Self>, job: Job, look: F) -> bool
    where
        F: FnOnce(Job) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<Option<f64>>> + Send + 'static,
    {
        if self
            .failed
            .lock()
            .unwrap()
            .get(&job.path)
            .is_some_and(|at| at.elapsed() < RETRY_FAILED)
        {
            return false;
        }
        if !self.queued.lock().unwrap().insert(job.path.clone()) {
            return true;
        }
        let me = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(me.delay).await;
            let Ok(_turn) = me.turns.acquire().await else { return };
            let t0 = Instant::now();
            let r = look(job.clone()).await;
            let name = vfs::file_name(&job.path);
            match &r {
                Ok(start) => {
                    tracing::info!(file = %name, start = ?start.map(|s| (s * 10.0).round() / 10.0), duration = job.duration, ms = t0.elapsed().as_millis() as u64, "end credits looked for");
                }
                Err(e) => {
                    tracing::warn!(file = %name, "end credits: {e:#}");
                    me.failed.lock().unwrap().insert(job.path.clone(), Instant::now());
                }
            }
            me.queued.lock().unwrap().remove(&job.path);
        });
        true
    }
}

async fn run(st: &AppState, job: &Job) -> Result<Option<f64>> {
    let db = st.db.clone().context("no database")?;
    let frames = measure(&st.config.ffmpeg, &Source::of(&job.path), job.stream, job.duration, job.start).await?;
    let start = find(&frames, job.duration);
    put(&db, job, start).await?;
    Ok(start)
}

/// Keeps the answer with the file's probe (if the file is unchanged).
async fn put(db: &Db, job: &Job, start: Option<f64>) -> Result<()> {
    let (path, size, mtime, value) = (job.path.clone(), job.size, job.mtime, stored(start).to_string());
    db.call(move |c| {
        c.execute(
            "UPDATE files_probe SET credits = ?4 WHERE path = ?1 AND size = ?2 AND mtime = ?3",
            rusqlite::params![path, size, mtime, value],
        )?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn job(path: &str) -> Job {
        Job {
            path: path.into(),
            size: 1,
            mtime: 1,
            stream: 0,
            duration: 2400.0,
            start: 0.0,
        }
    }

    fn credits_now() -> Arc<Credits> {
        Arc::new(Credits {
            delay: Duration::ZERO,
            ..Credits::default()
        })
    }

    /// Waits until nothing is queued (every run has finished).
    async fn idle(c: &Credits) {
        for _ in 0..500 {
            if c.queued.lock().unwrap().is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("runs did not finish");
    }

    /// A season's download once asked for every episode at once and 14 analyses ran
    /// together. Now: one run per file however often it is asked for, one file at a time.
    #[tokio::test]
    async fn one_analysis_at_a_time_and_one_per_file() {
        let c = credits_now();
        let runs: Arc<Mutex<Vec<String>>> = Arc::default();
        let (running, most) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let look = |runs: &Arc<Mutex<Vec<String>>>, running: &Arc<AtomicUsize>, most: &Arc<AtomicUsize>| {
            let (runs, running, most) = (runs.clone(), running.clone(), most.clone());
            move |job: Job| async move {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                most.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(30)).await;
                runs.lock().unwrap().push(job.path);
                running.fetch_sub(1, Ordering::SeqCst);
                Ok(Some(2300.0))
            }
        };
        for p in ["e1", "e1", "e2", "e3", "e2", "e4"] {
            assert!(c.request_with(job(p), look(&runs, &running, &most)), "{p}: an answer may come");
        }
        idle(&c).await;
        let mut done = runs.lock().unwrap().clone();
        done.sort();
        assert_eq!(done, ["e1", "e2", "e3", "e4"], "one run per file");
        assert_eq!(most.load(Ordering::SeqCst), 1, "runs never overlap");
        // Finished: asked again, it runs again (the answer is kept with the file's probe).
        assert!(c.request_with(job("e1"), look(&runs, &running, &most)));
        idle(&c).await;
        assert_eq!(runs.lock().unwrap().len(), 5);
    }

    /// A file that failed isn't looked at again for a while; the caller isn't told "pending".
    #[tokio::test]
    async fn a_failed_file_waits() {
        let c = credits_now();
        let tries = Arc::new(AtomicUsize::new(0));
        let failing = |tries: &Arc<AtomicUsize>| {
            let tries = tries.clone();
            move |_job: Job| async move {
                tries.fetch_add(1, Ordering::SeqCst);
                Err::<Option<f64>, _>(anyhow::anyhow!("unreadable"))
            }
        };
        assert!(c.request_with(job("bad"), failing(&tries)));
        idle(&c).await;
        assert!(!c.request_with(job("bad"), failing(&tries)), "no answer will come");
        idle(&c).await;
        assert_eq!(tries.load(Ordering::SeqCst), 1);
    }

    /// Keyframes as measured on real episodes: (seconds before the end, dark percentage).
    fn frames(duration: f64, from_end: &[(f64, u32)]) -> Vec<(f64, u32)> {
        from_end.iter().map(|&(t, p)| (duration + t, p)).collect()
    }

    fn found(duration: f64, from_end: &[(f64, u32)]) -> Option<f64> {
        find(&frames(duration, from_end), duration).map(|s| ((s - duration) * 10.0).round() / 10.0)
    }

    #[test]
    fn credits_on_black_with_logos_after() {
        // Text on black from 53 s before the end, a production logo at the very end.
        let ep = [
            (-77.0, 22),
            (-67.0, 19),
            (-58.0, 30),
            (-53.0, 99),
            (-43.0, 90),
            (-39.0, 100),
            (-35.0, 100),
            (-31.0, 100),
            (-26.0, 100),
            (-22.0, 100),
            (-18.0, 100),
            (-13.0, 100),
            (-10.0, 62),
            (-1.0, 100),
        ];
        assert_eq!(found(3601.5, &ep), Some(-53.0));
        // Credits, then 10 s of white studio logos.
        let ep = [
            (-59.0, 72),
            (-53.0, 75),
            (-51.0, 90),
            (-48.0, 100),
            (-39.0, 95),
            (-33.0, 94),
            (-27.0, 93),
            (-24.0, 92),
            (-21.0, 91),
            (-19.0, 84),
            (-18.0, 0),
            (-12.0, 0),
            (-9.0, 81),
            (-6.0, 56),
            (-3.0, 55),
        ];
        assert_eq!(found(2613.7, &ep), Some(-51.0));
    }

    #[test]
    fn dark_story_scenes_are_not_credits() {
        // A night scene (dark, but not that dark) right before the credits.
        let ep = [
            (-134.0, 61),
            (-132.0, 100),
            (-122.0, 94),
            (-115.0, 91),
            (-107.0, 81),
            (-97.0, 81),
            (-91.0, 70),
            (-79.0, 71),
            (-67.0, 78),
            (-59.0, 72),
            (-51.0, 90),
            (-48.0, 100),
            (-39.0, 95),
            (-27.0, 93),
            (-21.0, 91),
            (-19.0, 84),
            (-18.0, 0),
            (-9.0, 81),
        ];
        assert_eq!(found(2613.7, &ep), Some(-51.0));
        // Dark shots in the last scene, 15 s of light before the credits.
        let ep = [
            (-135.0, 91),
            (-131.0, 91),
            (-127.0, 50),
            (-119.0, 85),
            (-115.0, 63),
            (-111.0, 53),
            (-107.0, 38),
            (-103.0, 41),
            (-100.0, 99),
            (-91.0, 99),
            (-84.0, 98),
            (-72.0, 98),
            (-60.0, 94),
            (-47.0, 90),
            (-35.0, 92),
            (-27.0, 92),
            (-21.0, 85),
            (-15.0, 87),
            (-12.0, 78),
            (-10.0, 11),
            (-5.0, 30),
            (-3.0, 92),
        ];
        assert_eq!(found(3270.5, &ep), Some(-100.0));
    }

    #[test]
    fn short_or_early_dark_stretches_are_not_credits() {
        // A post-credits scene: the dark credits end half a minute before the end.
        let ep = [
            (-62.0, 0),
            (-61.0, 97),
            (-50.0, 93),
            (-45.0, 85),
            (-39.0, 86),
            (-36.0, 90),
            (-30.0, 9),
            (-17.0, 0),
            (-7.0, 0),
            (-3.0, 0),
        ];
        assert_eq!(found(1349.5, &ep), None);
        // Credits over pictures, then only the last 18 s on black.
        let ep = [
            (-30.0, 89),
            (-26.0, 79),
            (-22.0, 76),
            (-18.0, 94),
            (-14.0, 93),
            (-10.0, 85),
            (-5.0, 100),
            (-1.0, 100),
        ];
        assert_eq!(found(3212.7, &ep), None);
        // Nothing dark at all (credits over a picture).
        let ep: Vec<(f64, u32)> = (0..40).map(|i| (-200.0 + 5.0 * i as f64, 13)).collect();
        assert_eq!(found(1316.9, &ep), None);
        assert_eq!(found(100.0, &[(-5.0, 100)]), None);
    }

    #[test]
    fn the_video_may_end_before_the_file() {
        // The video stops 31 s before the file (audio and subtitles run on): credits on black,
        // then one keyframe of a logo, the last picture.
        let ep = [
            (-214.9, 59),
            (-205.3, 29),
            (-195.8, 33),
            (-191.7, 31),
            (-188.4, 21),
            (-178.8, 24),
            (-169.2, 23),
            (-159.6, 23),
            (-150.0, 30),
            (-144.6, 99),
            (-142.6, 99),
            (-139.6, 99),
            (-136.6, 99),
            (-133.1, 99),
            (-129.6, 99),
            (-126.1, 99),
            (-122.6, 99),
            (-119.1, 99),
            (-115.5, 99),
            (-112.0, 99),
            (-106.6, 97),
            (-103.6, 98),
            (-100.6, 98),
            (-97.6, 97),
            (-94.6, 98),
            (-91.6, 98),
            (-88.6, 97),
            (-85.6, 97),
            (-82.6, 98),
            (-79.6, 98),
            (-76.6, 98),
            (-73.5, 97),
            (-70.4, 97),
            (-67.2, 98),
            (-64.2, 98),
            (-61.2, 98),
            (-58.2, 98),
            (-55.2, 97),
            (-52.0, 98),
            (-49.0, 97),
            (-45.9, 99),
            (-42.9, 99),
            (-40.8, 99),
            (-31.3, 3),
        ];
        assert_eq!(found(3324.672, &ep), Some(-144.6));
        // 17 s short, and a 10 s logo between the credits and a last black picture.
        let ep = [
            (-211.8, 26),
            (-202.2, 21),
            (-192.7, 49),
            (-183.1, 32),
            (-173.7, 50),
            (-170.1, 49),
            (-168.0, 34),
            (-165.3, 52),
            (-160.2, 49),
            (-158.6, 43),
            (-153.3, 47),
            (-151.8, 44),
            (-148.4, 47),
            (-142.7, 46),
            (-135.2, 48),
            (-132.2, 70),
            (-130.3, 38),
            (-125.0, 48),
            (-119.9, 100),
            (-114.8, 100),
            (-112.8, 100),
            (-110.3, 100),
            (-107.8, 100),
            (-105.3, 100),
            (-99.3, 100),
            (-96.3, 100),
            (-93.3, 100),
            (-90.3, 100),
            (-87.3, 100),
            (-84.3, 100),
            (-78.4, 97),
            (-76.1, 98),
            (-73.7, 98),
            (-71.3, 97),
            (-68.9, 98),
            (-66.5, 98),
            (-64.2, 97),
            (-61.6, 97),
            (-59.0, 97),
            (-56.6, 97),
            (-54.2, 98),
            (-51.9, 97),
            (-49.5, 98),
            (-47.1, 98),
            (-44.7, 97),
            (-42.1, 98),
            (-39.8, 97),
            (-37.4, 98),
            (-35.0, 97),
            (-32.7, 100),
            (-30.2, 100),
            (-28.2, 99),
            (-26.7, 61),
            (-17.1, 100),
        ];
        assert_eq!(found(3255.52, &ep), Some(-119.9));
        // Credits over pictures (a montage): nothing found, as before.
        let ep = [
            (-211.9, 30),
            (-210.0, 30),
            (-207.5, 22),
            (-204.0, 30),
            (-202.6, 22),
            (-201.2, 26),
            (-197.6, 22),
            (-196.0, 30),
            (-194.4, 23),
            (-190.3, 30),
            (-187.7, 18),
            (-186.4, 27),
            (-184.2, 19),
            (-174.6, 5),
            (-169.5, 31),
            (-160.0, 33),
            (-150.4, 36),
            (-147.7, 25),
            (-143.1, 23),
            (-133.6, 18),
            (-132.1, 33),
            (-127.1, 20),
            (-122.3, 23),
            (-116.5, 27),
            (-106.9, 3),
            (-97.3, 2),
            (-87.7, 2),
            (-78.1, 2),
            (-68.5, 1),
            (-58.9, 2),
            (-49.3, 1),
            (-39.7, 1),
            (-30.1, 1),
            (-20.5, 1),
            (-10.9, 44),
            (-1.3, 100),
        ];
        assert_eq!(found(3467.552, &ep), None);
        // Video to the file's end: the last keyframe's picture lasts until then.
        assert_eq!(video_end(&[(0.0, 0), (4.0, 0), (8.0, 0)], 12.0), 12.0);
        assert_eq!(video_end(&[(0.0, 0), (4.0, 0), (8.0, 0)], 40.0), 12.0);
    }

    #[test]
    fn parses_blackframe_lines() {
        let err = "Input #0, matroska,webm, from 'x':\n[Parsed_blackframe_1 @ 0x7a] frame:0 pblack:8 pts:3243240 t:3243.240000 type:I last_keyframe:0\n\
                   [Parsed_blackframe_1 @ 0x7a] frame:1 pblack:100 pts:3244950 t:3244.950000 type:I last_keyframe:1\n[out] frame: 2\n";
        assert_eq!(parse(err), vec![(3243.24, 8), (3244.95, 100)]);
    }

    #[test]
    fn stored_answers_carry_their_version() {
        assert_eq!(read(Some(&stored(Some(12.5)).to_string())), Some(Some(12.5)));
        assert_eq!(read(Some(&stored(None).to_string())), Some(None));
        assert_eq!(read(Some(r#"{"v":0,"start":3}"#)), None);
        assert_eq!(read(None), None);
    }
}
