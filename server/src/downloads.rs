//! Downloads for offline playback.
//!
//! - **The original file** (`GET /v1/items/{id}/download`), straight from the media share with
//!   `Range`, `ETag`, `Last-Modified` and `If-Range`, so a background `URLSession` resumes it.
//! - **Prepared MP4s** (`POST /v1/items/{id}/download-jobs`): a progressive MP4 (`faststart`)
//!   that AVPlayer plays offline, made in the cache: the copied video (a remux, for the
//!   original of an MKV) or a GPU transcode at a quality of the title's ladder, with the audio
//!   tracks asked for (copied, or converted to AAC/E-AC-3 as for streaming). Jobs are queued:
//!   at most `MOONRISE_DOWNLOAD_JOBS` transcodes (default 1) on the GPU, on top of the playback
//!   runs, and two remuxes at once. A finished file is kept ("pinned") until it has been
//!   downloaded (then for another hour, for retries) or for 48 h; the space they take is capped
//!   (`MOONRISE_DOWNLOAD_MAX_GB`) and reported (`GET /v1/download-jobs`). A job's state is kept
//!   next to its file (`/cache/dl/<id>.json`), so finished files survive a restart.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use axum::{
    body::Body,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::{
    io::AsyncBufReadExt,
    sync::{watch, Semaphore},
};

use crate::{
    media::transcode::{self, Variant},
    media::{AudioTrack, Media},
    subtitles::subs::now_secs,
};

/// A finished file is kept this long at most.
const PIN_SECS: u64 = 48 * 3600;
/// After it has been downloaded, this long (the client may retry or resume).
const AFTER_DOWNLOAD_SECS: u64 = 3600;
/// Failed jobs are listed this long.
const FAILED_SECS: u64 = 3600;
/// Space left free on the disk beyond a job's expected size.
const DISK_MARGIN: u64 = 3 << 30;

/// What the client asks for.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct JobParams {
    /// Bits/s (None: the original).
    #[serde(default)]
    pub quality: Option<i64>,
    /// Track indices of the audio tracks to include, the first one the default (None: one
    /// per language, the default track first, at most 4).
    #[serde(default)]
    pub audio: Option<Vec<i64>>,
    /// The device shows HDR (default true): HDR is kept; false: tone-mapped to SDR.
    #[serde(default)]
    pub hdr: Option<bool>,
    /// Only "mp4".
    #[serde(default)]
    pub container: Option<String>,
}

/// A job as listed (and as stored next to its file).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobInfo {
    pub id: String,
    pub item_id: String,
    #[serde(skip_serializing)]
    pub file_key: String,
    pub params: JobParams,
    /// The variant's label ("Original", "720p · 4 Mbps"…).
    pub label: String,
    /// "remux" or "transcode".
    pub method: String,
    /// The track indices of the audio tracks in the file, in order.
    pub audio: Vec<i64>,
    /// "queued", "running", "done", "failed"
    pub state: String,
    pub progress: f64,
    /// Bytes written so far (the final size when done).
    pub size: u64,
    /// The estimate the job was admitted with.
    pub expected: u64,
    pub error: Option<String>,
    pub created_at: u64,
    pub finished_at: Option<u64>,
    pub downloaded_at: Option<u64>,
    /// When the file is deleted (done jobs).
    pub expires_at: Option<u64>,
    /// Where to get it (done jobs).
    pub file: Option<String>,
}

struct Job {
    info: Mutex<JobInfo>,
    cancel: watch::Sender<bool>,
}

pub struct Downloads {
    dir: PathBuf,
    ffmpeg: String,
    max_bytes: u64,
    transcode_enabled: bool,
    jobs: Mutex<HashMap<String, Arc<Job>>>,
    gpu: Arc<Semaphore>,
    remux: Arc<Semaphore>,
}

/// Why a job can't be made: the client should use another path.
#[derive(Debug)]
pub struct Refused(pub StatusCode, pub String);

impl Downloads {
    pub fn new(cache_dir: &Path, ffmpeg: String, max_bytes: u64, gpu_jobs: usize, transcode_enabled: bool) -> Self {
        let dir = cache_dir.join("dl");
        let _ = std::fs::create_dir_all(&dir);
        let d = Downloads {
            dir,
            ffmpeg,
            max_bytes,
            transcode_enabled,
            jobs: Mutex::new(HashMap::new()),
            gpu: Arc::new(Semaphore::new(gpu_jobs.max(1))),
            remux: Arc::new(Semaphore::new(2)),
        };
        d.restore();
        d
    }

    /// Finished jobs from before a restart; unfinished ones are dropped (their client asks
    /// again).
    fn restore(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        let mut jobs = self.jobs.lock().unwrap();
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "part") {
                let _ = std::fs::remove_file(&p);
                continue;
            }
            if p.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let info: Option<JobInfo> = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok());
            match info {
                Some(i) if i.state == "done" && self.file_of(&i.id).is_file() => {
                    jobs.insert(
                        i.id.clone(),
                        Arc::new(Job {
                            info: Mutex::new(i),
                            cancel: watch::channel(false).0,
                        }),
                    );
                }
                Some(i) => {
                    let _ = std::fs::remove_file(self.file_of(&i.id));
                    let _ = std::fs::remove_file(&p);
                }
                None => {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
        tracing::info!(jobs = jobs.len(), "prepared downloads restored");
    }

    pub fn file_of(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.mp4"))
    }

    fn persist(&self, info: &JobInfo) {
        if let Ok(b) = serde_json::to_vec_pretty(info) {
            let tmp = self.dir.join(format!("{}.json.tmp", info.id));
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(tmp, self.dir.join(format!("{}.json", info.id)));
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<JobInfo> {
        self.jobs.lock().unwrap().get(id).map(|j| j.info.lock().unwrap().clone())
    }

    pub fn list(&self) -> Vec<JobInfo> {
        let mut l: Vec<JobInfo> = self.jobs.lock().unwrap().values().map(|j| j.info.lock().unwrap().clone()).collect();
        l.sort_by_key(|j| std::cmp::Reverse(j.created_at));
        l
    }

    /// Bytes taken by finished files, and promised to running and queued ones.
    pub fn usage(&self) -> (u64, u64) {
        tally(&self.jobs.lock().unwrap())
    }

    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// The audio tracks for a job: those asked for (that the stream carries), else one per
    /// language, the default first, at most 4.
    pub fn pick_audio<'a>(media: &'a Media, asked: Option<&[i64]>) -> Vec<&'a AudioTrack> {
        match asked {
            Some(list) if !list.is_empty() => {
                let mut out: Vec<&AudioTrack> = Vec::new();
                for i in list {
                    if let Some(a) = media.audio_track(*i) {
                        if !out.iter().any(|x| x.index == a.index) {
                            out.push(a);
                        }
                    }
                }
                out
            }
            _ => {
                let mut out: Vec<&AudioTrack> = media.audio.iter().filter(|a| a.default).take(1).collect();
                for a in &media.audio {
                    if out.len() >= 4 {
                        break;
                    }
                    if !out.iter().any(|x| x.index == a.index || x.language == a.language) {
                        out.push(a);
                    }
                }
                out
            }
        }
    }

    /// A job for these options: an existing one for the same file and options (not failed),
    /// or a new one (queued). `Refused` when it can't be made here.
    pub fn create(self: &Arc<Self>, media: &Arc<Media>, params: JobParams) -> Result<(JobInfo, bool), Refused> {
        let bad = |m: String| Refused(StatusCode::UNPROCESSABLE_ENTITY, m);
        if params.container.as_deref().is_some_and(|c| c != "mp4") {
            return Err(bad("only mp4".into()));
        }
        if !media.supported {
            return Err(bad(format!("can't be served: {}", media.reasons.join("; "))));
        }
        let v = media.video.as_ref().ok_or_else(|| bad("no video".into()))?;
        if media.boundaries.is_empty() {
            return Err(bad("keyframe index pending".into()));
        }
        let ladder = transcode::ladder(v, params.hdr.unwrap_or(true), self.transcode_enabled);
        if ladder.is_empty() {
            return Err(bad(v.copy_reason.clone().unwrap_or_else(|| "not transcoded".into())));
        }
        let variant = ladder[transcode::select(&ladder, params.quality)].clone();
        if let Some(q) = params.quality.filter(|q| variant.bitrate > *q && !self.transcode_enabled) {
            return Err(bad(format!("quality {q} needs transcoding (switched off)")));
        }
        let audio = Self::pick_audio(media, params.audio.as_deref());
        // A title with no audio track is downloaded as video alone.
        if audio.is_empty() && !media.audio.is_empty() {
            return Err(bad("no audio track to include".into()));
        }
        let audio_bits: i64 = audio
            .iter()
            .map(|a| a.conversion.as_ref().map_or(640_000, |c| c.bitrate as i64))
            .sum();
        let expected = if variant.transcode {
            ((variant.bitrate + audio_bits) as f64 * media.duration / 8.0) as u64
        } else {
            media
                .size
                .unwrap_or(((v.bitrate + audio_bits) as f64 * media.duration / 8.0) as u64)
        };

        let mut jobs = self.jobs.lock().unwrap();
        let audio_ids: Vec<i64> = audio.iter().map(|a| a.index).collect();
        for j in jobs.values() {
            let i = j.info.lock().unwrap();
            if i.file_key == media.file_key
                && i.label == variant.label
                && i.audio == audio_ids
                && i.params.hdr == params.hdr
                && i.state != "failed"
            {
                return Ok((i.clone(), false));
            }
        }
        let (used, reserved) = tally(&jobs);
        if used + reserved + expected > self.max_bytes {
            return Err(Refused(
                StatusCode::INSUFFICIENT_STORAGE,
                format!(
                    "download space full: {} GB in use or promised, {} GB more needed, {} GB allowed",
                    gb(used + reserved),
                    gb(expected),
                    gb(self.max_bytes)
                ),
            ));
        }
        if let Some(free) = free_bytes(&self.dir) {
            if free < expected + reserved + DISK_MARGIN {
                return Err(Refused(
                    StatusCode::INSUFFICIENT_STORAGE,
                    format!("not enough disk space ({} GB free)", gb(free)),
                ));
            }
        }
        let id: String = rand::thread_rng()
            .sample_iter(rand::distributions::Alphanumeric)
            .take(16)
            .map(char::from)
            .collect();
        let info = JobInfo {
            id: id.clone(),
            item_id: media.item_id.clone(),
            file_key: media.file_key.clone(),
            params,
            label: variant.label.clone(),
            method: if variant.transcode { "transcode" } else { "remux" }.into(),
            audio: audio_ids,
            state: "queued".into(),
            progress: 0.0,
            size: 0,
            expected,
            error: None,
            created_at: now_secs(),
            finished_at: None,
            downloaded_at: None,
            expires_at: None,
            file: None,
        };
        let job = Arc::new(Job {
            info: Mutex::new(info.clone()),
            cancel: watch::channel(false).0,
        });
        jobs.insert(id, job.clone());
        drop(jobs);
        let (this, media) = (self.clone(), media.clone());
        tokio::spawn(async move { this.run(job, media, variant).await });
        Ok((info, true))
    }

    async fn run(self: Arc<Self>, job: Arc<Job>, media: Arc<Media>, variant: Variant) {
        let id = job.info.lock().unwrap().id.clone();
        // A delete sent before this task ran still stops it.
        let mut cancel = crate::media::transcode::stop_signal(&job.cancel);
        let sem = if variant.transcode { self.gpu.clone() } else { self.remux.clone() };
        let permit = tokio::select! {
            p = sem.acquire_owned() => p,
            _ = cancel.changed() => return,
        };
        let Ok(_permit) = permit else { return };
        job.info.lock().unwrap().state = "running".into();
        let t0 = std::time::Instant::now();
        tracing::info!(job = %id, item = %media.item_id, variant = variant.id, method = if variant.transcode { "transcode" } else { "remux" }, "download job started");
        let result = self.encode(&job, &media, &variant, &mut cancel).await;
        let mut info = job.info.lock().unwrap();
        match result {
            Ok(size) => {
                let now = now_secs();
                info.state = "done".into();
                info.progress = 1.0;
                info.size = size;
                info.finished_at = Some(now);
                info.expires_at = Some(now + PIN_SECS);
                info.file = Some(format!("/v1/download-jobs/{id}/file"));
                tracing::info!(job = %id, item = %media.item_id, mb = size >> 20, secs = t0.elapsed().as_secs(), "download job done");
            }
            Err(e) => {
                info.state = "failed".into();
                info.error = Some(format!("{e:#}"));
                info.finished_at = Some(now_secs());
                tracing::warn!(job = %id, item = %media.item_id, "download job failed: {e:#}");
                let _ = std::fs::remove_file(self.file_of(&id));
            }
        }
        self.persist(&info);
    }

    async fn encode(&self, job: &Job, media: &Media, variant: &Variant, cancel: &mut watch::Receiver<bool>) -> Result<u64> {
        let (id, audio_ids) = {
            let i = job.info.lock().unwrap();
            (i.id.clone(), i.audio.clone())
        };
        let part = self.dir.join(format!("{id}.mp4.part"));
        let args = job_args(media, variant, &audio_ids, &part)?;
        let mut child = tokio::process::Command::new("nice")
            .args(["-n", "10", self.ffmpeg.as_str()])
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("ffmpeg")?;
        let stderr = child.stderr.take().context("no stderr")?;
        let err_log = tokio::spawn(async move {
            let mut s = String::new();
            let mut r = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(l)) = r.next_line().await {
                if s.len() < 4000 {
                    s += &l;
                    s.push('\n');
                }
            }
            s
        });
        let mut lines = tokio::io::BufReader::new(child.stdout.take().context("no stdout")?).lines();
        let duration = media.duration.max(1.0);
        loop {
            tokio::select! {
                l = lines.next_line() => match l {
                    Ok(Some(l)) => {
                        if let Some(us) = l.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<i64>().ok()) {
                            let p = (us as f64 / 1e6 / duration).clamp(0.0, 0.999);
                            let size = std::fs::metadata(&part).map_or(0, |m| m.len());
                            let mut i = job.info.lock().unwrap();
                            i.progress = i.progress.max(p);
                            i.size = size;
                        }
                    }
                    _ => break,
                },
                _ = cancel.changed() => {
                    let _ = child.kill().await;
                    let _ = tokio::fs::remove_file(&part).await;
                    bail!("cancelled");
                }
            }
        }
        let status = child.wait().await?;
        if !status.success() {
            let _ = tokio::fs::remove_file(&part).await;
            bail!(
                "ffmpeg: {}",
                crate::admin::logbuf::scrub(&err_log.await.unwrap_or_default().lines().take(4).collect::<Vec<_>>().join(" / "))
            );
        }
        let fin = self.file_of(&id);
        tokio::fs::rename(&part, &fin).await?;
        Ok(tokio::fs::metadata(&fin).await?.len())
    }

    /// Stop a job and delete its file.
    pub fn delete(&self, id: &str) -> bool {
        let Some(job) = self.jobs.lock().unwrap().remove(id) else {
            return false;
        };
        job.cancel.send_replace(true);
        let _ = std::fs::remove_file(self.file_of(id));
        let _ = std::fs::remove_file(self.dir.join(format!("{id}.json")));
        true
    }

    /// The whole file was sent once: it can go (after a grace period).
    pub fn mark_downloaded(&self, id: &str) {
        let jobs = self.jobs.lock().unwrap();
        if let Some(j) = jobs.get(id) {
            let mut i = j.info.lock().unwrap();
            if i.downloaded_at.is_none() {
                let now = now_secs();
                i.downloaded_at = Some(now);
                i.expires_at = Some(i.expires_at.unwrap_or(u64::MAX).min(now + AFTER_DOWNLOAD_SECS));
                tracing::info!(job = %id, "prepared download fetched");
                self.persist(&i);
            }
        }
    }

    /// Every few minutes: delete expired files and old failures.
    pub async fn janitor(this: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(300)).await;
            let now = now_secs();
            let expired: Vec<String> = {
                let jobs = this.jobs.lock().unwrap();
                jobs.values()
                    .filter_map(|j| {
                        let i = j.info.lock().unwrap();
                        let gone = match i.state.as_str() {
                            "done" => i.expires_at.is_some_and(|e| e <= now),
                            "failed" => i.finished_at.is_some_and(|f| f + FAILED_SECS <= now),
                            _ => false,
                        };
                        gone.then(|| i.id.clone())
                    })
                    .collect()
            };
            for id in expired {
                tracing::info!(job = %id, "prepared download expired");
                this.delete(&id);
            }
        }
    }
}

fn tally(jobs: &HashMap<String, Arc<Job>>) -> (u64, u64) {
    let mut used = 0;
    let mut reserved = 0;
    for j in jobs.values() {
        let i = j.info.lock().unwrap();
        match i.state.as_str() {
            "done" => used += i.size,
            "queued" | "running" => reserved += i.expected.max(i.size),
            _ => {}
        }
    }
    (used, reserved)
}

fn gb(b: u64) -> String {
    format!("{:.1}", b as f64 / (1u64 << 30) as f64)
}

fn free_bytes(p: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs is a C struct of plain integers, for which all-zero bytes are a valid value.
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a valid NUL-terminated path and a zeroed out-parameter.
    if unsafe { libc::statvfs(c.as_ptr(), &raw mut s) } != 0 {
        return None;
    }
    Some(s.f_bavail as u64 * s.f_frsize as u64)
}

/// ISO 639-2 language of a track (the MP4 `mdhd` wants three letters).
fn stream_language(media: &Media, index: i64) -> Option<String> {
    let ms = media.item.media_sources.first()?;
    ms.media_streams
        .iter()
        .find(|s| s.index == index)?
        .language
        .clone()
        .filter(|l| l.len() == 3)
}

/// The ffmpeg command line for a prepared download: the video (copied, or the variant's
/// transcode), the audio tracks (copied or converted), a progressive MP4.
pub fn job_args(media: &Media, variant: &Variant, audio: &[i64], out: &Path) -> Result<Vec<String>> {
    let v = media.video.as_ref().context("no video")?;
    let mut a: Vec<String> = vec!["-progress".into(), "pipe:1".into()];
    if variant.transcode {
        a.extend(transcode::video_args(media, variant, 0, true)?);
    } else {
        a.extend(["-nostdin", "-hide_banner", "-nostats", "-v", "error"].map(String::from));
        a.extend(media.source.input_args());
        a.extend(["-map".into(), format!("0:{}", v.stream), "-c:v".into(), "copy".into()]);
        match v.codec.as_str() {
            "hevc" => a.extend(["-tag:v", "hvc1"].map(String::from)),
            "h264" => a.extend(["-tag:v", "avc1"].map(String::from)),
            _ => {}
        }
        if v.short_nal_lengths {
            a.extend(["-bsf:v", "h264_mp4toannexb"].map(String::from));
        }
    }
    for (k, idx) in audio.iter().enumerate() {
        let t = media.audio_track(*idx).with_context(|| format!("no audio track {idx}"))?;
        a.extend(["-map".into(), format!("0:{}", t.stream)]);
        match &t.conversion {
            None => a.extend([format!("-c:a:{k}"), "copy".into()]),
            Some(c) => {
                let codec = if c.codecs_attr() == "ec-3" { "eac3" } else { "aac" };
                a.extend([
                    format!("-c:a:{k}"),
                    codec.into(),
                    format!("-b:a:{k}"),
                    c.bitrate.to_string(),
                    format!("-ac:a:{k}"),
                    c.channels.to_string(),
                ]);
            }
        }
        if let Some(l) = stream_language(media, *idx) {
            a.extend([format!("-metadata:s:a:{k}"), format!("language={l}")]);
        }
        a.extend([format!("-metadata:s:a:{k}"), format!("title={}", t.name)]);
        a.extend([format!("-disposition:a:{k}"), if k == 0 { "default" } else { "0" }.into()]);
    }
    a.extend(
        [
            "-map_chapters",
            "-1",
            "-map_metadata",
            "-1",
            "-sn",
            "-dn",
            "-max_muxing_queue_size",
            "4096",
        ]
        .map(String::from),
    );
    a.extend(["-movflags", "+faststart", "-f", "mp4", "-y"].map(String::from));
    a.push(out.to_string_lossy().into_owned());
    Ok(a)
}

// ---------------------------------------------------------------- serving files

/// A byte range of a file of `size` bytes from a `Range` header (one range only; anything
/// else is answered with the whole file).
pub fn parse_range(h: &str, size: u64) -> Option<Result<(u64, u64), ()>> {
    let spec = h.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let r = if a.trim().is_empty() {
        let n: u64 = b.trim().parse().ok()?;
        if n == 0 {
            return Some(Err(()));
        }
        (size.saturating_sub(n), size.saturating_sub(1))
    } else {
        let start: u64 = a.trim().parse().ok()?;
        let end: u64 = if b.trim().is_empty() {
            size.saturating_sub(1)
        } else {
            b.trim().parse::<u64>().ok()?.min(size.saturating_sub(1))
        };
        (start, end)
    };
    if r.0 >= size || r.0 > r.1 {
        return Some(Err(()));
    }
    Some(Ok(r))
}

fn http_date(t: std::time::SystemTime) -> String {
    httpdate::fmt_http_date(t)
}

/// Serves a file with `Range`/`If-Range`, `ETag` and `Last-Modified`. `done` runs once the
/// last byte has been handed to the connection.
pub async fn serve_file(
    path: &str,
    req: &HeaderMap,
    content_type: &str,
    filename: &str,
    done: Option<Box<dyn FnOnce() + Send>>,
) -> Response {
    use futures_util::StreamExt;
    let meta = match crate::storage::vfs::stat_for_read(path).await {
        Ok(m) if m.is_file => m,
        _ => return (StatusCode::NOT_FOUND, "no such file").into_response(),
    };
    let size = meta.size;
    let mtime = meta.mtime.max(0) as u128;
    let modified = UNIX_EPOCH + std::time::Duration::from_nanos(meta.mtime.max(0) as u64);
    let etag = format!("\"{size:x}-{mtime:x}\"");
    let last_modified = http_date(modified);

    // If-Range: the range only applies to the same file.
    let range_ok = match req.get(header::IF_RANGE).and_then(|v| v.to_str().ok()) {
        None => true,
        Some(v) => v == etag || v == last_modified,
    };
    let range = req
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .filter(|_| range_ok)
        .and_then(|h| parse_range(h, size));
    let (status, start, end) = match range {
        None => (StatusCode::OK, 0, size.saturating_sub(1)),
        Some(Ok((s, e))) => (StatusCode::PARTIAL_CONTENT, s, e),
        Some(Err(())) => {
            return (
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{size}"))],
            )
                .into_response();
        }
    };
    let len = if size == 0 { 0 } else { end - start + 1 };
    let file = match crate::storage::vfs::stream(path, start, len).await {
        Ok(f) => f,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    // The connection stops asking for data once Content-Length bytes are out: count them.
    let mut done = done.filter(|_| size == 0 || end + 1 == size);
    let mut sent = 0u64;
    let body_stream = file.map(move |chunk: std::io::Result<Bytes>| {
        if let Ok(b) = &chunk {
            sent += b.len() as u64;
            if sent >= len {
                if let Some(f) = done.take() {
                    f();
                }
            }
        }
        chunk
    });
    let mut resp = Response::new(Body::from_stream(body_stream));
    *resp.status_mut() = status;
    let h = resp.headers_mut();
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Ok(v) = HeaderValue::from_str(&etag) {
        h.insert(header::ETAG, v);
    }
    if let Ok(v) = HeaderValue::from_str(&last_modified) {
        h.insert(header::LAST_MODIFIED, v);
    }
    if let Ok(v) = HeaderValue::from_str(content_type) {
        h.insert(header::CONTENT_TYPE, v);
    }
    let safe: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{safe}\"")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    if status == StatusCode::PARTIAL_CONTENT {
        if let Ok(v) = HeaderValue::from_str(&format!("bytes {start}-{end}/{size}")) {
            h.insert(header::CONTENT_RANGE, v);
        }
    }
    resp
}

/// MIME type by file extension.
pub fn content_type(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mp4" | "m4v") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        Some("avi") => "video/x-msvideo",
        Some("ts" | "m2ts") => "video/mp2t",
        Some("mpg" | "mpeg") => "video/mpeg",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_title_with_no_audio_track_downloads_as_video_alone() {
        // Refused before with "no audio track to include".
        use crate::media::transcode::tests;
        let dir = std::env::temp_dir().join(format!("moonrise-dl-silent-{}", std::process::id()));
        let d = Arc::new(Downloads::new(&dir, "false".into(), 1 << 40, 1, false));
        let v = tests::video("h264", 1920, 1080, 4_000_000, "SDR");
        let mut m = tests::media(v.clone());
        m.size = Some(15_000_000);
        let m = Arc::new(m);
        let (info, new) = d.create(&m, JobParams::default()).expect("a silent title downloads");
        assert!(new && info.audio.is_empty());
        let variant = transcode::ladder(&v, true, false).remove(0);
        let args = job_args(&m, &variant, &[], Path::new("/o/a.mp4")).unwrap().join(" ");
        assert!(args.contains("-map 0:0 -c:v copy") && !args.contains("-c:a"), "{args}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some(Ok((0, 99))));
        assert_eq!(parse_range("bytes=900-", 1000), Some(Ok((900, 999))));
        assert_eq!(parse_range("bytes=-100", 1000), Some(Ok((900, 999))));
        assert_eq!(parse_range("bytes=990-5000", 1000), Some(Ok((990, 999))));
        assert_eq!(parse_range("bytes=1000-", 1000), Some(Err(())));
        assert_eq!(parse_range("bytes=0-1,5-6", 1000), None);
        assert_eq!(parse_range("items=0-1", 1000), None);
    }

    #[tokio::test]
    async fn serves_ranges_and_if_range() {
        let dir = std::env::temp_dir().join(format!("moonrise-dl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.mp4");
        std::fs::write(&p, b"0123456789").unwrap();
        let body = |r: Response| async { axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap() };

        let r = serve_file(&p.to_string_lossy(), &HeaderMap::new(), "video/mp4", "a.mp4", None).await;
        assert_eq!(r.status(), StatusCode::OK);
        let etag = r.headers()[header::ETAG].to_str().unwrap().to_string();
        assert_eq!(&body(r).await[..], b"0123456789");

        let fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut h = HeaderMap::new();
        h.insert(header::RANGE, "bytes=4-".parse().unwrap());
        h.insert(header::IF_RANGE, etag.parse().unwrap());
        let f = fired.clone();
        let r = serve_file(
            &p.to_string_lossy(),
            &h,
            "video/mp4",
            "a.mp4",
            Some(Box::new(move || f.store(true, std::sync::atomic::Ordering::SeqCst))),
        )
        .await;
        assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(r.headers()[header::CONTENT_RANGE], "bytes 4-9/10");
        assert_eq!(&body(r).await[..], b"456789");
        assert!(fired.load(std::sync::atomic::Ordering::SeqCst), "the end was reached");

        // Another file version: the whole file.
        h.insert(header::IF_RANGE, "\"other\"".parse().unwrap());
        let r = serve_file(&p.to_string_lossy(), &h, "video/mp4", "a.mp4", None).await;
        assert_eq!(r.status(), StatusCode::OK);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
