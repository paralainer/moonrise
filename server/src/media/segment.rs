//! Cutting HLS segments. Each segment is made on its own by a short ffmpeg run that copies
//! (never re-encodes) one track from the file, so any segment can be made at any time:
//!
//! - Video: ffmpeg seeks to the segment's keyframe (it lands up to a GOP early) and writes
//!   fragmented MP4 with a fragment per keyframe and the file's own timestamps (frag_discont);
//!   only the fragments inside the segment are kept. Splitting at keyframes in decode order
//!   keeps every frame with the pictures it depends on. A fragment belongs to the segment its
//!   keyframe's presentation time falls in: boundaries are presentation times, and in an open
//!   GOP the keyframe is decoded well before it is shown, by half a second or more in x265
//!   files. Every fragment of a file goes to exactly one segment, whichever run it came from.
//! - Audio: every packet belongs to exactly one segment: ffmpeg trims at packet precision with
//!   an output-side -ss/-to, which also rebases timestamps to the segment start; they are moved
//!   back into the file's time here.
//!
//! The init section (ftyp+moov) of a track is kept from one run. For video that is always the
//! run from the title's start, and every other run's fragments are moved onto its timing: ffmpeg
//! starts a run's edit list and decode times at its first keyframe's composition offset, which
//! differs from keyframe to keyframe when a seek lands exactly on them (MP4 files).

use std::{
    collections::HashMap,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use tokio::{
    io::AsyncReadExt,
    sync::{OnceCell, Semaphore},
};

use crate::{media::mp4, media::Media, storage::cache::Cache};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Track {
    Video,
    /// A copied audio track, by its stream index in the file (ffprobe's, not the track index).
    Audio(i64),
}

impl Track {
    fn dir(&self) -> String {
        match self {
            Track::Video => "v".into(),
            Track::Audio(i) => format!("a{i}"),
        }
    }
}

/// Bump when the way segments are cut changes (or the ffmpeg that cuts them: 3 = the patched ffmpeg build;
/// 4 = fragments assigned by presentation time, on the timing of a run from the start), so cached ones aren't mixed with new ones.
const CUT_VERSION: u32 = 4;

/// An input seek up to this reads from the file's start instead. For a video stream with B-frames
/// ffmpeg asks the demuxer for 3/23 s before the time given, so a seek near 0 asks for a time
/// before the file's start. AVI, FLV and ASF can't seek there: ffmpeg says "could not seek", and
/// the run starts where the failed seek left off, at the next keyframe.
const SEEK_FROM_START: f64 = 3.0 / 23.0 + 0.006;

/// How far a keyframe may be before a boundary and still count as on it (index and stream can
/// differ by a few ms). Less than the 3/23 s ffmpeg seeks ahead of the time asked for when frames
/// are reordered: otherwise the run could start at a kept fragment, where ffmpeg can't know the
/// decode times yet and squeezes the frames shown before the keyframe.
const BOUNDARY_SLACK: f64 = 0.1;

/// A run that makes no progress for this long is stopped, so it can't keep its job slot for good
/// (and with a video init section, every title's: `making_init`). Not a limit on the run as a
/// whole: when all else failed a cut reads the file to its end, minutes on a large file. Progress
/// is output or input: a video run writes a fragment at every keyframe, at most some 10 s of video
/// apart, but an audio run writes nothing until it ends (one fragment), however long the reading
/// takes: about 15 s of the file's data for a segment, a lot on a high-bitrate file and a slow share.
const STALL_AFTER: Duration = Duration::from_secs(90);

/// How often a run's input is looked at.
const PROGRESS_EVERY: Duration = Duration::from_secs(1);

/// How far a run has read its input.
enum Input {
    /// A file on a share: what the loopback listener has sent it.
    Loopback(crate::storage::vfs::ReadCount),
    /// A local file: what the process has read (`/proc/<pid>/io`, Linux; elsewhere output only).
    Proc,
}

impl Input {
    fn read(&self, pid: Option<u32>) -> Option<u64> {
        match self {
            Input::Loopback(c) => Some(c.get()),
            Input::Proc => {
                let io = std::fs::read_to_string(format!("/proc/{}/io", pid?)).ok()?;
                io.lines()
                    .find_map(|l| l.strip_prefix("rchar:"))
                    .and_then(|v| v.trim().parse().ok())
            }
        }
    }
}

/// What a command writes to stdout; an error if it fails, or neither writes nor reads for `stall`
/// (killed then).
async fn output_unless_stalled(mut command: tokio::process::Command, stall: Duration, input: Input) -> Result<Vec<u8>> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("ffmpeg")?;
    let pid = child.id();
    let mut stdout = child.stdout.take().context("no stdout")?;
    let mut stderr = child.stderr.take().context("no stderr")?;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let reading = async {
        let mut moved = tokio::time::Instant::now();
        let mut read = input.read(pid);
        let mut tick = tokio::time::interval(PROGRESS_EVERY.min(stall / 4));
        loop {
            out.reserve(256 << 10);
            tokio::select! {
                r = stdout.read_buf(&mut out) => {
                    if r.context("ffmpeg's output")? == 0 {
                        return Ok(());
                    }
                    moved = tokio::time::Instant::now();
                }
                _ = tick.tick() => {
                    let now = input.read(pid);
                    if now != read {
                        read = now;
                        moved = tokio::time::Instant::now();
                    } else if moved.elapsed() >= stall {
                        bail!("ffmpeg neither wrote nor read anything for {} s", stall.as_secs_f64());
                    }
                }
            }
        }
    };
    // Stderr has to be drained for the run to go on, and says nothing of how it is doing.
    tokio::select! {
        r = reading => r?,
        () = async { let _ = stderr.read_to_end(&mut err).await; std::future::pending::<()>().await } => {}
    }
    let status = tokio::time::timeout(stall, child.wait())
        .await
        .context("ffmpeg doesn't exit")?
        .context("ffmpeg")?;
    if !status.success() {
        let _ = tokio::time::timeout(Duration::from_secs(1), stderr.read_to_end(&mut err)).await;
        bail!("ffmpeg: {}", crate::admin::logbuf::scrub(String::from_utf8_lossy(&err).trim()));
    }
    Ok(out)
}

/// Which of a run's fragments (by the file time of their keyframes) segment `start..end` keeps.
fn in_segment(t: f64, start: f64, end: f64, first: bool, last: bool) -> bool {
    (first || t >= start - BOUNDARY_SLACK) && (last || t < end - BOUNDARY_SLACK)
}

/// What a run has to look like before it can be trusted with a segment.
#[derive(Debug, PartialEq)]
enum RunCheck {
    /// It started after the segment's start, so it may have skipped a fragment that belongs here.
    StartedLate,
    /// It stopped before showing where the segment ends, so its last fragment may be cut short.
    StoppedEarly,
    Complete,
}

fn check_run(times: &[f64], start: f64, end: f64, first: bool, last: bool) -> RunCheck {
    if !first && times.first().is_none_or(|t| *t > start + BOUNDARY_SLACK) {
        return RunCheck::StartedLate;
    }
    if !last && times.last().is_none_or(|t| *t < end - BOUNDARY_SLACK) {
        return RunCheck::StoppedEarly;
    }
    RunCheck::Complete
}

/// The next seek point when a run started late: further back each time, down to the file's start.
fn seek_earlier(start: f64, seek: f64) -> f64 {
    (start - ((start - seek) * 4.0).max(30.0)).max(0.0)
}

/// The next stop when a run stopped early: further on each time, in the end the file's end.
fn read_further(end: f64, to: f64) -> Option<f64> {
    let ahead = (to - end) * 4.0;
    (ahead <= 480.0).then_some(end + ahead.max(30.0))
}

/// A segment with no keyframe of its own (a boundary that isn't one in the stream): its frames are
/// in the fragment before it, which the previous segment holds. A player can't take an empty
/// segment, and an overlap costs nothing, so it gets that fragment too.
fn fallback_fragment(times: &[f64], start: f64) -> Option<usize> {
    times
        .iter()
        .rposition(|t| *t < start)
        .or(if times.is_empty() { None } else { Some(0) })
}

/// A cut in progress; everyone asking for the same segment waits on it.
type Inflight = Arc<OnceCell<Result<(), String>>>;

pub struct Segmenter {
    ffmpeg: String,
    cache: Arc<Cache>,
    jobs: Semaphore,
    inflight: Mutex<HashMap<String, Inflight>>,
    /// Held while a video init section is made, so a title gets one.
    making_init: tokio::sync::Mutex<()>,
}

impl Segmenter {
    pub fn new(ffmpeg: String, cache: Arc<Cache>, jobs: usize) -> Self {
        Segmenter {
            ffmpeg,
            cache,
            jobs: Semaphore::new(jobs),
            inflight: Mutex::new(HashMap::new()),
            making_init: tokio::sync::Mutex::new(()),
        }
    }

    fn key(media: &Media, track: Track, name: &str) -> String {
        // (Video with short NAL lengths: now rewritten, so not what was cached before.)
        let nal = if media.video.as_ref().is_some_and(|v| v.short_nal_lengths) {
            "n4"
        } else {
            ""
        };
        // (A run from near the start of these used to begin late.)
        let from_start = if seek_failed_near_start(&media.container) { "s0" } else { "" };
        format!(
            "{}/{}/c{CUT_VERSION}{nal}{from_start}/{}/{}",
            media.item_id,
            media.version,
            track.dir(),
            name
        )
    }

    pub async fn init(&self, media: &Media, track: Track) -> Result<Bytes> {
        let key = Self::key(media, track, "init.mp4");
        if track == Track::Video {
            return self.video_init(media).await;
        }
        if let Some(b) = self.cache.get(&key).await {
            return Ok(b);
        }
        // Any cut of the track writes it: wait for one in progress (the session's first
        // segments, started when it opened) rather than cut the title's start as well.
        let prefix = Self::key(media, track, "");
        let running: Vec<Inflight> = self
            .inflight
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(_, c)| c.clone())
            .collect();
        for cell in running {
            let t0 = std::time::Instant::now();
            while !cell.initialized() && t0.elapsed() < std::time::Duration::from_secs(30) {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            if let Some(b) = self.cache.get(&key).await {
                return Ok(b);
            }
        }
        // Else the first segment, the cheapest to make.
        self.ensure(media, track, 0).await?;
        self.cache.get(&key).await.context("init missing after cut")
    }

    pub async fn segment(&self, media: &Media, track: Track, n: usize) -> Result<Bytes> {
        let key = Self::key(media, track, &format!("{n}.m4s"));
        if let Some(b) = self.cache.get(&key).await {
            return Ok(b);
        }
        self.ensure(media, track, n).await?;
        self.cache.get(&key).await.context("segment missing after cut")
    }

    /// Cuts segment `n` unless it's cached or already being cut (then waits for that).
    pub async fn ensure(&self, media: &Media, track: Track, n: usize) -> Result<()> {
        let key = Self::key(media, track, &format!("{n}.m4s"));
        if tokio::fs::metadata(self.cache_path_hint(&key)).await.is_ok() {
            return Ok(());
        }
        let cell = self.inflight.lock().unwrap().entry(key.clone()).or_default().clone();
        let result = cell
            .get_or_init(|| async {
                let r = self.cut(media, track, n).await.map_err(|e| format!("{e:#}"));
                if let Err(e) = &r {
                    tracing::warn!(item = %media.item_id, ?track, n, "cut failed: {e}");
                }
                r
            })
            .await
            .clone();
        self.inflight.lock().unwrap().remove(&key);
        result.map_err(anyhow::Error::msg)
    }

    fn cache_path_hint(&self, key: &str) -> std::path::PathBuf {
        self.cache.path_of(key)
    }

    /// The video init section: the one of a run from the title's start, the same whichever
    /// segment is asked for first and whenever it is made again.
    async fn video_init(&self, media: &Media) -> Result<Bytes> {
        let key = Self::key(media, Track::Video, "init.mp4");
        if let Some(b) = self.cache.get(&key).await {
            return Ok(b);
        }
        let _making = self.making_init.lock().await;
        if let Some(b) = self.cache.get(&key).await {
            return Ok(b);
        }
        let (start, end) = media.segment(0).context("no segments")?;
        let to = (!media.is_last(0)).then_some(end + 1.0);
        let _permit = self.jobs.acquire().await?;
        let _reading = crate::subtitles::subs::StreamRead::start();
        let frag = self
            .run(media, Track::Video, start, start, to)
            .await?
            .context("no video at the title's start")?;
        self.cache.put(&key, &frag.init).await;
        Ok(Bytes::from(frag.init))
    }

    /// One ffmpeg run: the track's fragmented MP4 (None: no packets at all).
    async fn run(&self, media: &Media, track: Track, seek: f64, start: f64, to: Option<f64>) -> Result<Option<mp4::Fragmented>> {
        let mut args: Vec<String> = vec!["-nostdin".into(), "-v".into(), "error".into()];
        let (mut input_args, count) = media.source.input_args_counted();
        let input = count.map_or(Input::Proc, Input::Loopback);
        // `seek` and `start` are on the file's own clock, and ffmpeg adds the file's start time to
        // an input's `-ss`. A start time below 0 is left as before 0.31.8: the run is checked.
        let input_seek = media.copy_seek(seek);
        let mut seek_args: Vec<String> = Vec::new();
        if input_seek > SEEK_FROM_START {
            seek_args.extend(["-ss".into(), format!("{input_seek:.3}")]);
        }
        match track {
            Track::Video => {
                let v = media.video.as_ref().context("no video")?;
                args.append(&mut seek_args);
                args.append(&mut input_args);
                args.extend(["-copyts".into(), "-map".into(), format!("0:{}", v.stream)]);
                args.extend(video_copy_args(&v.codec, v.short_nal_lengths));
            }
            Track::Audio(index) => {
                args.append(&mut seek_args);
                args.append(&mut input_args);
                args.extend(["-copyts".into(), "-ss".into(), format!("{start:.3}")]);
                args.extend(["-map".into(), format!("0:{index}")]);
                args.extend(audio_copy_args(adts_framed(media, index)));
            }
        }
        if let Some(to) = to {
            args.extend(["-to".into(), format!("{to:.3}")]);
        }
        // MKV chapters would come along as a QuickTime chapter track, which AVPlayer rejects.
        args.extend(["-map_chapters".into(), "-1".into(), "-map_metadata".into(), "-1".into()]);
        args.extend([
            "-f".into(),
            "mp4".into(),
            "-movflags".into(),
            "+frag_keyframe+empty_moov+default_base_moof+delay_moov+frag_discont".into(),
            "pipe:1".into(),
        ]);
        let mut ffmpeg = tokio::process::Command::new(&self.ffmpeg);
        ffmpeg.args(&args);
        let stdout = output_unless_stalled(ffmpeg, STALL_AFTER, input).await?;
        if stdout.is_empty() {
            return Ok(None);
        }
        mp4::parse(&stdout).map(Some)
    }

    /// The video run for a segment: a little past its end (the next keyframe's fragment is dropped,
    /// but shows that the last one kept is whole), checked and run again wider if needed.
    async fn video_run(&self, media: &Media, n: usize, start: f64, end: f64) -> Result<Option<mp4::Fragmented>> {
        let (first, last) = (n == 0, media.is_last(n));
        let (mut seek, mut to) = (start, (!last).then_some(end + 1.0));
        loop {
            let frag = self.run(media, Track::Video, seek, start, to).await?;
            let times: Vec<f64> = frag.iter().flat_map(|f| f.fragments.iter().map(|x| f.file_time(x))).collect();
            let check = check_run(&times, start, end, first, last);
            match (&check, to) {
                (RunCheck::StartedLate, _) if seek > 0.0 => seek = seek_earlier(start, seek),
                (RunCheck::StoppedEarly, Some(t)) => to = read_further(end, t),
                _ => return Ok(frag),
            }
            tracing::info!(item = %media.item_id, n, ?check, seek, ?to, "cutting again");
        }
    }

    async fn cut(&self, media: &Media, track: Track, n: usize) -> Result<()> {
        let (start, end) = media.segment(n).context("no such segment")?;
        let last = media.is_last(n);
        // The timing this segment has to match; made first, as that takes a slot of its own.
        let init = match track {
            Track::Video => Some(self.video_init(media).await?),
            Track::Audio(_) => None,
        };
        let _permit = self.jobs.acquire().await?;
        let _reading = crate::subtitles::subs::StreamRead::start();
        let t0 = std::time::Instant::now();
        let frag = match track {
            Track::Video => self.video_run(media, n, start, end).await?,
            Track::Audio(_) => {
                self.run(media, track, (start - 5.0).max(0.0), start, (!last).then_some(end))
                    .await?
            }
        };
        let Some(mut frag) = frag else {
            bail!("no media in {start:.3}..{end:.3}")
        };
        let ts = frag.timescale as f64;
        let mut body = Vec::new();
        match track {
            Track::Video => {
                let times: Vec<f64> = frag.fragments.iter().map(|f| frag.file_time(f)).collect();
                let target = mp4::parse(init.as_deref().unwrap_or_default())?.media_start;
                let delta = target - frag.media_start;
                for f in &mut frag.fragments {
                    f.shift(delta)?;
                }
                for (f, t) in frag.fragments.iter().zip(&times) {
                    if in_segment(*t, start, end, n == 0, last) {
                        body.extend_from_slice(&f.bytes);
                    }
                }
                if body.is_empty() {
                    if let Some(i) = fallback_fragment(&times, start) {
                        tracing::warn!(item = %media.item_id, n, start, end, "no keyframe in this segment: repeating the one before");
                        body.extend_from_slice(&frag.fragments[i].bytes);
                    }
                }
            }
            Track::Audio(_) => {
                let shift = (start * ts).round() as i64;
                for f in &mut frag.fragments {
                    f.shift(shift)?;
                    body.extend_from_slice(&f.bytes);
                }
            }
        }
        if body.is_empty() {
            bail!("no media in {start:.3}..{end:.3}");
        }
        if track != Track::Video {
            let init_key = Self::key(media, track, "init.mp4");
            if self.cache.get(&init_key).await.is_none() {
                self.cache.put(&init_key, &frag.init).await;
            }
        }
        self.cache.put(&Self::key(media, track, &format!("{n}.m4s")), &body).await;
        tracing::debug!(item = %media.item_id, ?track, n, ms = t0.elapsed().as_millis() as u64, bytes = body.len(), "cut");
        Ok(())
    }
}

/// Whether a copied audio track's AAC may come in ADTS frames: from these containers (in AVI only
/// sometimes; the filter passes packets without the framing through unchanged). MP4 wants the raw frames. ffmpeg's muxer removes the framing by itself, but not with
/// `empty_moov`, which turns that off, so the run fails with "Malformed AAC bitstream".
fn adts_framed(media: &Media, stream: i64) -> bool {
    matches!(media.container.as_str(), "mpegts" | "aac" | "avi") && media.audio.iter().any(|a| a.stream == stream && a.codec == "aac")
}

/// Containers (ffprobe `format_name`) where an input seek near 0 failed and the run began at the
/// next keyframe, measured with this ffmpeg; Matroska, MP4, MOV and MPEG-TS weren't affected.
fn seek_failed_near_start(container: &str) -> bool {
    matches!(container, "avi" | "flv" | "asf")
}

/// A copied audio track's codec arguments.
fn audio_copy_args(adts: bool) -> Vec<String> {
    let mut a: Vec<String> = vec!["-c".into(), "copy".into()];
    if adts {
        a.extend(["-bsf:a".into(), "aac_adtstoasc".into()]);
    }
    a
}

/// A streamed video segment's codec arguments: copied, HEVC tagged `hvc1` and H.264 with 1–3 byte
/// NAL lengths rewritten through Annex B (Apple's players refuse short lengths).
fn video_copy_args(codec: &str, short_nal_lengths: bool) -> Vec<String> {
    let mut a: Vec<String> = vec!["-c".into(), "copy".into()];
    if codec == "hevc" {
        a.extend(["-tag:v".into(), "hvc1".into()]);
    }
    if short_nal_lengths {
        a.extend(["-bsf:v".into(), "h264_mp4toannexb".into()]);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A streamed segment of H.264 with short NAL lengths goes through h264_mp4toannexb;
    /// other video is copied as it is (HEVC tagged hvc1).
    #[test]
    fn short_nal_lengths_are_rewritten_in_streamed_segments() {
        assert_eq!(video_copy_args("h264", false).join(" "), "-c copy");
        assert_eq!(video_copy_args("h264", true).join(" "), "-c copy -bsf:v h264_mp4toannexb");
        assert_eq!(video_copy_args("hevc", false).join(" "), "-c copy -tag:v hvc1");
    }

    /// What a run of an audio track is told, from the file's container and the track's codec.
    #[tokio::test]
    async fn adts_framed_aac_goes_through_the_filter_when_copied() {
        let dir = std::env::temp_dir().join(format!("moonrise-adts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ffmpeg = dir.join("ffmpeg");
        crate::testutil::write_executable(&ffmpeg, &format!("#!/bin/sh\necho \"$*\" > {}/args\n", dir.display()));
        let cache = Arc::new(Cache::new(dir.join("cache"), 1 << 30));
        let seg = Segmenter::new(ffmpeg.display().to_string(), cache, 1);
        let args = async |container: &str, codec: &str| {
            let mut m = crate::media::transcode::tests::media(crate::media::transcode::tests::video("h264", 1920, 1080, 8_000_000, "SDR"));
            m.container = container.into();
            m.audio = vec![crate::media::AudioTrack {
                index: 1,
                stream: 1,
                codec: codec.into(),
                codecs_attr: "mp4a.40.2".into(),
                name: "a".into(),
                language: None,
                channels: 2,
                default: true,
                converted: false,
                conversion: None,
                lossless_source: false,
                source_channels: 2,
                sample_rate: 48000,
                time_base: 1.0 / 90000.0,
            }];
            seg.run(&m, Track::Audio(1), 0.0, 0.0, Some(6.0)).await.unwrap();
            std::fs::read_to_string(dir.join("args")).unwrap()
        };
        for container in ["mpegts", "aac", "avi"] {
            let a = args(container, "aac").await;
            assert!(a.contains("-map 0:1 -c copy -bsf:a aac_adtstoasc -to 6.000"), "{container}: {a}");
        }
        for (container, codec) in [
            ("matroska,webm", "aac"),
            ("mov,mp4,m4a,3gp,3g2,mj2", "aac"),
            ("mpegts", "ac3"),
            ("mpegts", "eac3"),
        ] {
            let a = args(container, codec).await;
            assert!(
                a.contains("-map 0:1 -c copy -to 6.000") && !a.contains("-bsf"),
                "{container} {codec}: {a}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A run from near the start reads from the file's start. With `-ss 0.000`, ffmpeg asks
    /// an AVI with B-frames for 3/23 s before 0, the seek fails, and the run began at the next
    /// keyframe (H.264 + AC-3 or AAC: the first 1.5 s of audio lost, and of video where copied). Only the
    /// files where that seek failed get new cache keys.
    #[tokio::test]
    async fn a_run_from_near_the_start_reads_from_the_files_start() {
        let dir = std::env::temp_dir().join(format!("moonrise-seek0-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ffmpeg = dir.join("ffmpeg");
        crate::testutil::write_executable(&ffmpeg, &format!("#!/bin/sh\necho \"$*\" > {}/args\n", dir.display()));
        let cache = Arc::new(Cache::new(dir.join("cache"), 1 << 30));
        let seg = Segmenter::new(ffmpeg.display().to_string(), cache, 1);
        let mut m = crate::media::transcode::tests::media(crate::media::transcode::tests::video("h264", 1920, 1080, 8_000_000, "SDR"));
        m.container = "avi".into();
        m.source = crate::storage::source::Source::Local("/m/a.avi".into());
        m.audio = vec![crate::media::AudioTrack {
            index: 1,
            stream: 1,
            codec: "ac3".into(),
            codecs_attr: "ac-3".into(),
            name: "a".into(),
            language: None,
            channels: 6,
            default: true,
            converted: false,
            conversion: None,
            lossless_source: false,
            source_channels: 6,
            sample_rate: 48000,
            time_base: 1.0 / 48000.0,
        }];
        let args = async |m: &Media, track: Track, seek: f64, start: f64| {
            seg.run(m, track, seek, start, Some(start + 6.0)).await.unwrap();
            std::fs::read_to_string(dir.join("args")).unwrap()
        };
        for (track, seek, start) in [(Track::Audio(1), 0.0, 0.0), (Track::Audio(1), 0.1, 5.1), (Track::Video, 0.0, 0.0)] {
            let a = args(&m, track, seek, start).await;
            assert!(a.starts_with("-nostdin -v error -i "), "{track:?} {seek}: {a}");
        }
        let a = args(&m, Track::Audio(1), 6.783, 11.783).await;
        assert!(a.starts_with("-nostdin -v error -ss 6.783 -i "), "{a}");
        assert!(a.contains("-copyts -ss 11.783 -map 0:1 -c copy -to 17.783"), "{a}");
        let a = args(&m, Track::Video, 11.783, 11.783).await;
        assert!(a.starts_with("-nostdin -v error -ss 11.783 -i "), "{a}");

        // Cached cuts: new for the containers where the seek failed, the same for the others.
        for (container, key) in [
            ("avi", "i/v/c4s0/a1/0.m4s"),
            ("flv", "i/v/c4s0/a1/0.m4s"),
            ("asf", "i/v/c4s0/a1/0.m4s"),
            ("matroska,webm", "i/v/c4/a1/0.m4s"),
            ("mov,mp4,m4a,3gp,3g2,mj2", "i/v/c4/a1/0.m4s"),
            ("mpegts", "i/v/c4/a1/0.m4s"),
        ] {
            m.container = container.into();
            assert_eq!(Segmenter::key(&m, Track::Audio(1), "0.m4s"), key);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The keyframes of "28 Days Later" around the failure: the one at 76.410 is decoded at
    /// 76.117 (in the cut's own timing), 0.293 s early, and the old check by decode time filed its
    /// fragment under the segment before.
    const KEYFRAMES: [f64; 8] = [51.802, 62.229, 63.397, 64.940, 65.983, 76.410, 86.295, 90.132];
    const BOUNDARIES: [f64; 5] = [62.229, 70.0, 76.410, 86.295, 96.0];

    fn owners(keyframes: &[f64], boundaries: &[f64]) -> Vec<Vec<f64>> {
        let last = boundaries.len() - 2;
        (0..=last)
            .map(|i| {
                let (s, e) = (boundaries[i], boundaries[i + 1]);
                keyframes
                    .iter()
                    .copied()
                    .filter(|t| in_segment(*t, s, e, i == 0, i == last))
                    .collect()
            })
            .collect()
    }

    fn shell(script: &str) -> tokio::process::Command {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", script]);
        c
    }

    #[tokio::test]
    async fn a_silent_run_is_stopped_and_a_slow_one_is_not() {
        let stall = Duration::from_millis(400);
        // Writes, then stops writing. `exec` so the process killed is the one that sleeps.
        let t0 = std::time::Instant::now();
        let e = output_unless_stalled(shell("echo some; exec sleep 30"), stall, Input::Proc)
            .await
            .unwrap_err();
        assert!(format!("{e:#}").contains("neither wrote nor read"), "{e:#}");
        assert!(t0.elapsed() < Duration::from_secs(5), "{:?}", t0.elapsed());
        // Longer in all than the limit, but it keeps writing.
        let out = output_unless_stalled(shell("for i in 1 2 3 4 5 6; do echo $i; sleep 0.15; done"), stall, Input::Proc)
            .await
            .unwrap();
        assert_eq!(out, b"1\n2\n3\n4\n5\n6\n");
        // A failure says what ffmpeg said, whatever it wrote before.
        let e = output_unless_stalled(shell("echo part; echo broken >&2; exit 3"), stall, Input::Proc)
            .await
            .unwrap_err();
        assert!(format!("{e:#}").contains("broken"), "{e:#}");
        // A lot of stderr doesn't stop it.
        let out = output_unless_stalled(shell("head -c 300000 /dev/zero >&2; echo done"), stall, Input::Proc)
            .await
            .unwrap();
        assert_eq!(out, b"done\n");
    }

    /// An audio run writes nothing until it ends: reading its input is progress too.
    #[tokio::test]
    async fn a_run_that_only_reads_is_not_stopped() {
        let stall = Duration::from_millis(400);
        // Input from a share: what the loopback sent it.
        let feed = |n: usize| {
            let count = crate::storage::vfs::ReadCount::new();
            let c = count.counter();
            let feeding = async move {
                for _ in 0..n {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    c.fetch_add(1 << 20, std::sync::atomic::Ordering::Relaxed);
                }
            };
            (Input::Loopback(count), feeding)
        };
        let (input, feeding) = feed(10);
        let (out, ()) = tokio::join!(output_unless_stalled(shell("sleep 1.4; echo done"), stall, input), feeding);
        assert_eq!(out.unwrap(), b"done\n");
        // Reads that stop count as silence again.
        let t0 = std::time::Instant::now();
        let (input, feeding) = feed(3);
        let (out, ()) = tokio::join!(output_unless_stalled(shell("exec sleep 30"), stall, input), feeding);
        assert!(format!("{:#}", out.unwrap_err()).contains("neither wrote nor read"));
        assert!(t0.elapsed() < Duration::from_secs(5), "{:?}", t0.elapsed());
        // A local file: what the process read (Linux).
        if std::path::Path::new("/proc/self/io").exists() {
            let script = "i=0; while [ $i -lt 10 ]; do read -r x < /proc/self/status; sleep 0.15; i=$((i+1)); done; echo done";
            let out = output_unless_stalled(shell(script), stall, Input::Proc).await.unwrap();
            assert_eq!(out, b"done\n");
        }
    }

    #[test]
    fn a_keyframe_decoded_early_stays_in_its_segment() {
        // Decode times would put 76.410's fragment (76.117) before 76.410 - 0.25.
        assert!(in_segment(76.410, 76.410, 86.295, false, false));
        assert!(!in_segment(76.410, 65.983, 76.410, false, false));
        assert!(in_segment(86.295, 86.295, 96.0, false, false));
    }

    #[test]
    fn every_keyframe_has_exactly_one_segment() {
        // Also keyframes just either side of a boundary, the title's start and its end.
        let mut keyframes = KEYFRAMES.to_vec();
        keyframes.extend([0.0, 62.2, 69.95, 70.0, 70.05, 86.2, 95.95, 500.0]);
        keyframes.sort_by(f64::total_cmp);
        let mut boundaries = vec![0.0];
        boundaries.extend(BOUNDARIES);
        let owned = owners(&keyframes, &boundaries);
        assert_eq!(owned.concat(), keyframes);
        // The first segment takes what comes before its boundary, the last what comes after.
        assert!(owned[0].contains(&0.0));
        assert!(owned.last().unwrap().contains(&500.0));
    }

    #[test]
    fn a_run_is_checked_before_it_is_trusted() {
        // Landed a GOP early, and went past the end: the normal case.
        assert_eq!(
            check_run(&[65.983, 76.410, 86.295], 76.410, 86.295, false, false),
            RunCheck::Complete
        );
        // Landed on the keyframe itself (MP4 files seek exactly).
        assert_eq!(check_run(&[76.410, 86.295], 76.410, 86.295, false, false), RunCheck::Complete);
        // Landed after the start: could have skipped a keyframe.
        assert_eq!(check_run(&[86.295], 76.410, 86.295, false, false), RunCheck::StartedLate);
        assert_eq!(check_run(&[], 76.410, 86.295, false, false), RunCheck::StartedLate);
        // The last fragment read is one this segment keeps: it may be cut short by -to.
        assert_eq!(check_run(&[65.983, 76.410], 76.410, 86.295, false, false), RunCheck::StoppedEarly);
        // The title's first and last segments don't have those limits.
        assert_eq!(check_run(&[2.0], 0.0, 6.0, true, false), RunCheck::StoppedEarly);
        assert_eq!(check_run(&[8.0], 0.0, 6.0, true, false), RunCheck::Complete);
        assert_eq!(check_run(&[100.0], 100.0, 106.0, false, true), RunCheck::Complete);
    }

    #[test]
    fn retries_widen_and_end() {
        let mut seek = 1000.0;
        let mut seeks = vec![];
        while seek > 0.0 {
            seek = seek_earlier(1000.0, seek);
            seeks.push(seek);
        }
        assert_eq!(seeks, [970.0, 880.0, 520.0, 0.0]);
        let mut to = Some(1001.0);
        let mut tos = vec![];
        while let Some(t) = to {
            to = read_further(1000.0, t);
            tos.push(to);
        }
        assert_eq!(tos, [Some(1030.0), Some(1120.0), Some(1480.0), None]);
    }

    #[test]
    fn a_boundary_without_a_keyframe_repeats_the_fragment_holding_it() {
        // The segment 70..76.410 has no keyframe: 65.983's fragment covers it.
        assert_eq!(owners(&KEYFRAMES, &BOUNDARIES)[1], Vec::<f64>::new());
        assert_eq!(fallback_fragment(&KEYFRAMES, 70.0), Some(4));
        // Nothing before it at all: the first there is.
        assert_eq!(fallback_fragment(&[10.0, 20.0], 5.0), Some(0));
        assert_eq!(fallback_fragment(&[], 5.0), None);
    }
}
