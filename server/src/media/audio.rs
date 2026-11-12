//! Audio that can't be copied into HLS (DTS, TrueHD, MP3, Vorbis, Opus, FLAC, PCM…) as AAC
//! (up to 2 channels) or E-AC-3 5.1 (more), with no gaps or clicks at segment boundaries.
//! For players that ask for it (`lossless=1`), lossless multichannel sources (TrueHD, DTS-HD MA,
//! FLAC, PCM…) become FLAC instead, keeping every sample and every channel up to 7.1
//! ([`Conversion::lossless`]).
//!
//! **Why not one ffmpeg run per segment** (as the copy path does): every fresh encoder starts
//! with priming and an empty bit reservoir, so independently encoded segments don't join
//! seamlessly. Instead, a *run* encodes continuously from a segment boundary onwards, and the
//! segments are cut from its output here:
//!
//! - **A global frame grid.** Output frame `k` of a track always covers the same samples, in
//!   every run: it decodes to `[k*F - P, (k+1)*F - P)` (F = frame length, P = the encoder's
//!   priming). A run feeds its encoder from exactly sample `k_first*F` (`aresample` with
//!   `first_pts`), so its frames land on that grid, and its timestamps are computed here, not
//!   taken from ffmpeg. Segment `n` holds the frames whose time is in `[b_n, b_n+1)`, so
//!   segments from different runs fit together without overlap or gap.
//! - **Warm-up.** A run starts half a second of frames before the first segment it delivers and
//!   drops them, so the delivered frames come from an encoder in its steady state.
//! - **Sample-exact input.** ffmpeg times decoded audio by the first packet after the seek; MKV
//!   timestamps are rounded to the millisecond, which would put runs started at different
//!   places up to ±0.5 ms apart. Decoded frames are snapped to the source's own frame grid
//!   (when its frames have a fixed length clearly above the timestamp precision), which makes
//!   every run decode the same samples at the same times.
//! - **One run, many segments.** A run keeps going while the player asks for segments within
//!   its reach (`REACH`), staying at most `LEAD` of media ahead of the last request (pipe
//!   back-pressure pauses ffmpeg), and stops when it reaches a segment that is already cached
//!   or another run's start, or after `IDLE` without requests. A request outside every run's
//!   reach (a seek) starts a new run there; the first segment of a run is ready in about a
//!   second. Finished segments go to the disk cache, so seeking back costs nothing.
//!
//! The encoder output is raw ADTS, E-AC-3 or FLAC (no container), parsed into access units and
//! wrapped in fMP4 fragments by `mp4::write_fragment`. The init section comes from a short
//! ffmpeg run on silence with the same settings.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use serde::Serialize;
use tokio::{
    io::AsyncReadExt,
    sync::{watch, OwnedSemaphorePermit, Semaphore},
};

use crate::{
    media::mp4,
    media::{AudioTrack, Media},
    storage::cache::Cache,
};

/// Bump when converted output changes, so cached segments aren't mixed with new ones.
pub const CONV_VERSION: u32 = 1;
/// Frames of warm-up dropped at the start of a run, in seconds of audio.
const WARMUP_SECS: f64 = 0.5;
/// Input is read from this long before a run's first sample (the seek lands at a keyframe at
/// or before it).
const PREROLL_SECS: f64 = 3.0;
/// A request this far (in media time) past a run's progress waits for that run.
const REACH_SECS: f64 = 20.0;
/// How far a run encodes ahead of the last segment requested from it.
const LEAD_SECS: f64 = 60.0;
/// A run with nothing requested for this long stops.
const IDLE: Duration = Duration::from_secs(180);
/// How long a request waits for its segment.
const WAIT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    Aac,
    Eac3,
    Flac,
}

/// FLAC frames made by the converter: a fixed length, so they sit on a grid like AAC's. Short
/// enough that even a frame stored uncompressed (8 channels × 24 bits) fits the 32 KiB input
/// buffers Android's FLAC decoders get (Media3 asks for no more): 2048 overflowed them on loud
/// 7.1 passages.
const FLAC_FRAME: u64 = 1024;
/// The largest FLAC frame a decoder must take (see `FLAC_FRAME`).
const FLAC_MAX_FRAME_BYTES: u64 = 32 * 1024;
// Even an uncompressed 7.1 24-bit frame (verbatim subframes, plus headers) fits.
const _: () = assert!(FLAC_FRAME * 8 * 3 + 64 <= FLAC_MAX_FRAME_BYTES);

/// What a source track is converted to.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Conversion {
    pub codec: Codec,
    pub channels: u32,
    pub sample_rate: u32,
    pub bitrate: u32,
}

impl Conversion {
    /// Up to 2 channels: AAC-LC (every Apple device and AirPlay receiver plays it). More:
    /// E-AC-3 5.1 at 640 kb/s (decoded and downmixed by every Apple device, passed through to
    /// a receiver by the Apple TV; ffmpeg's encoder stops at 5.1, so 6.1/7.1 are downmixed).
    pub fn for_source(channels: i64, sample_rate: i64) -> Conversion {
        let sample_rate = if matches!(sample_rate, 44100 | 48000) {
            sample_rate as u32
        } else {
            48000
        };
        if channels <= 2 {
            let channels = channels.clamp(1, 2) as u32;
            Conversion {
                codec: Codec::Aac,
                channels,
                sample_rate,
                bitrate: if channels == 1 { 96_000 } else { 192_000 },
            }
        } else {
            Conversion {
                codec: Codec::Eac3,
                channels: 6,
                sample_rate,
                bitrate: 640_000,
            }
        }
    }

    /// FLAC, for a lossless source (`lossless_source`) of 6 to 8 channels (5.1, 6.1, 7.1),
    /// served to players that decode FLAC in fMP4 and output more than 2 channels (`lossless=1`).
    /// 24 bits (16-bit sources cost little more: FLAC drops the unused low bits), every channel
    /// kept; 44.1 and 48 kHz kept, other rates 48 kHz (as the lossy conversions). None: not such
    /// a source (it keeps `for_source`'s conversion).
    pub fn lossless(source_lossless: bool, channels: i64, sample_rate: i64) -> Option<Conversion> {
        if !source_lossless || !(6..=8).contains(&channels) {
            return None;
        }
        let sample_rate = if matches!(sample_rate, 44100 | 48000) {
            sample_rate as u32
        } else {
            48000
        };
        // An estimate (FLAC's rate varies): about 60% of 24-bit PCM. For BANDWIDTH and downloads'
        // size estimates; part of the cache tag, so keep it a pure function of the rest.
        let bitrate = (channels as u32 * sample_rate * 24 / 1000 * 6 / 10) * 1000;
        Some(Conversion {
            codec: Codec::Flac,
            channels: channels as u32,
            sample_rate,
            bitrate,
        })
    }

    /// Samples per access unit.
    pub fn frame_len(&self) -> u64 {
        match self.codec {
            Codec::Aac => 1024,
            Codec::Eac3 => 1536,
            Codec::Flac => FLAC_FRAME,
        }
    }

    /// The encoder's delay (ffmpeg's `initial_padding`): the first packet decodes to the
    /// samples this far before the first input sample.
    pub fn priming(&self) -> u64 {
        match self.codec {
            Codec::Aac => 1024,
            Codec::Eac3 => 256,
            Codec::Flac => 0,
        }
    }

    pub fn codecs_attr(&self) -> &'static str {
        match self.codec {
            Codec::Aac => "mp4a.40.2",
            Codec::Eac3 => "ec-3",
            Codec::Flac => "fLaC",
        }
    }

    /// The channel layout (ffmpeg's default for the count, which the decoder's `-ac` makes).
    fn layout(&self) -> &'static str {
        match self.channels {
            1 => "mono",
            2 => "stereo",
            7 => "6.1",
            8 => "7.1",
            _ => "5.1",
        }
    }

    /// The encoder's options (codec, rate, channels…), without the output format.
    fn encoder_args(&self) -> Vec<String> {
        let (ch, sr) = (self.channels.to_string(), self.sample_rate.to_string());
        match self.codec {
            Codec::Aac | Codec::Eac3 => {
                let codec = if self.codec == Codec::Aac { "aac" } else { "eac3" };
                ["-c:a", codec, "-b:a", &self.bitrate.to_string(), "-ac", &ch, "-ar", &sr]
                    .map(String::from)
                    .to_vec()
            }
            // s32: 24-bit FLAC. A fixed frame length keeps the output frames on a grid.
            Codec::Flac => [
                "-c:a",
                "flac",
                "-sample_fmt",
                "s32",
                "-frame_size",
                &FLAC_FRAME.to_string(),
                "-ac",
                &ch,
                "-ar",
                &sr,
            ]
            .map(String::from)
            .to_vec(),
        }
    }

    /// The raw format the encoder writes (parsed by [`Framer`]).
    fn raw_format(&self) -> &'static str {
        match self.codec {
            Codec::Aac => "adts",
            Codec::Eac3 => "eac3",
            Codec::Flac => "flac",
        }
    }

    /// Part of cache keys: everything that changes the output.
    pub fn tag(&self) -> String {
        let c = match self.codec {
            Codec::Aac => "aac",
            Codec::Eac3 => "eac3",
            Codec::Flac => "flac",
        };
        let frames = if self.codec == Codec::Flac {
            format!("-f{}", self.frame_len())
        } else {
            String::new()
        };
        format!("{c}-{}ch-{}k-{}{frames}", self.channels, self.bitrate / 1000, self.sample_rate)
    }

    /// The first output frame whose time is at or after `t` seconds.
    fn frame_at(&self, t: f64) -> u64 {
        let k = (t * self.sample_rate as f64 + self.priming() as f64) / self.frame_len() as f64;
        (k - 1e-6).ceil().max(0.0) as u64
    }
}

/// The source's frame grid in its own samples: frames of `len` samples, one starting at `origin`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Grid {
    origin: i64,
    len: i64,
}

#[derive(Clone, Debug)]
struct Progress {
    /// The segment being made.
    next: usize,
    /// Set when the run has ended.
    done: Option<Result<(), String>>,
}

struct Run {
    id: u64,
    start: usize,
    /// The session that started it (`DELETE /v1/s/{sid}` stops it).
    owner: String,
    progress: watch::Sender<Progress>,
    /// The latest segment requested from this run.
    demand: watch::Sender<usize>,
    last_request: Mutex<Instant>,
    cancel: watch::Sender<bool>,
}

impl Run {
    fn next(&self) -> usize {
        self.progress.borrow().next
    }

    fn ended(&self) -> bool {
        self.progress.borrow().done.is_some()
    }

    fn want(&self, n: usize) {
        self.demand.send_if_modified(|d| {
            let more = n > *d;
            *d = (*d).max(n);
            more
        });
        *self.last_request.lock().unwrap() = Instant::now();
    }
}

pub struct Converter {
    ffmpeg: String,
    ffprobe: String,
    cache: Arc<Cache>,
    runs: Mutex<HashMap<String, Vec<Arc<Run>>>>,
    slots: Arc<Semaphore>,
    grids: Mutex<HashMap<String, Option<Grid>>>,
    next_id: AtomicU64,
    /// Runs started (for logs and tests).
    pub started: AtomicUsize,
}

impl Converter {
    pub fn new(ffmpeg: String, ffprobe: String, cache: Arc<Cache>, jobs: usize) -> Self {
        Converter {
            ffmpeg,
            ffprobe,
            cache,
            runs: Mutex::new(HashMap::new()),
            slots: Arc::new(Semaphore::new(jobs.max(1))),
            grids: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            started: AtomicUsize::new(0),
        }
    }

    fn track_key(media: &Media, a: &AudioTrack, conv: &Conversion) -> String {
        // Since 0.31.8 segment 0 of a file with a start time begins at it (`run`), not at 0.
        let timeline = if media.origin > 0.0 { "-tl2" } else { "" };
        format!(
            "{}/{}/conv{CONV_VERSION}/a{}-{}{timeline}",
            media.item_id,
            media.version,
            a.stream,
            conv.tag()
        )
    }

    fn segment_key(track: &str, n: usize) -> String {
        format!("{track}/{n}.m4s")
    }

    pub async fn init(&self, conv: &Conversion) -> Result<Bytes> {
        let key = format!("conv-init/v{CONV_VERSION}/{}.mp4", conv.tag());
        if let Some(b) = self.cache.get(&key).await {
            return Ok(b);
        }
        let mut args: Vec<String> = ["-nostdin", "-v", "error", "-f", "lavfi", "-i"].map(String::from).to_vec();
        args.push(format!("anullsrc=r={}:cl={}", conv.sample_rate, conv.layout()));
        args.extend(["-t".into(), "0.5".into()]);
        args.extend(conv.encoder_args());
        args.extend(
            [
                "-f",
                "mp4",
                "-movflags",
                "+frag_keyframe+empty_moov+default_base_moof+delay_moov",
                "pipe:1",
            ]
            .map(String::from),
        );
        let out = tokio::process::Command::new(&self.ffmpeg)
            .args(&args)
            .kill_on_drop(true)
            .output()
            .await
            .context("ffmpeg")?;
        if !out.status.success() {
            bail!(
                "ffmpeg (init): {}",
                crate::admin::logbuf::scrub(String::from_utf8_lossy(&out.stderr).trim())
            );
        }
        let init = mp4::strip_edts(&mp4::parse(&out.stdout)?.init)?;
        self.cache.put(&key, &init).await;
        Ok(Bytes::from(init))
    }

    /// Segment `n` of converted track `a`.
    pub async fn segment(self: &Arc<Self>, media: &Arc<Media>, a: &AudioTrack, n: usize, owner: &str) -> Result<Bytes> {
        self.segment_tries(media, a, n, owner, 3).await
    }

    /// Segment `n`, from up to `tries` runs: when the run it waits on ends (or is stopped)
    /// without it, another is started.
    async fn segment_tries(self: &Arc<Self>, media: &Arc<Media>, a: &AudioTrack, n: usize, owner: &str, tries: usize) -> Result<Bytes> {
        let conv = a.conversion.as_ref().context("track isn't converted")?;
        let track = Self::track_key(media, a, conv);
        let key = Self::segment_key(&track, n);
        if let Some(b) = self.cache.get(&key).await {
            return Ok(b);
        }
        let deadline = tokio::time::Instant::now() + WAIT;
        let mut unreadable = None;
        for _ in 0..tries {
            let Some(run) = self.run_for(media, a, &track, n, owner, unreadable).await? else {
                // Made since the look above. Gone again (the janitor), or there but unreadable:
                // another run makes it. Noted before reading, so that only a file this read
                // really failed on counts as unreadable, not one that arrives just after it.
                let seen = file_id(&self.cache.path_of(&key));
                if let Some(b) = self.cache.get(&key).await {
                    return Ok(b);
                }
                unreadable = seen;
                continue;
            };
            let mut rx = run.progress.subscribe();
            loop {
                let p = rx.borrow_and_update().clone();
                if p.next > n || p.done.is_some() {
                    if let Some(b) = self.cache.get(&key).await {
                        return Ok(b);
                    }
                    if let Some(Err(e)) = p.done {
                        bail!("conversion run: {e}");
                    }
                    break; // ended (or was stopped) without it: try another run
                }
                match tokio::time::timeout_at(deadline, rx.changed()).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => break,
                    Err(_) => bail!("timed out waiting for converted segment {n}"),
                }
            }
        }
        bail!("converted segment {n} not produced")
    }

    /// Make sure segment `n` is being made (prefetch), without waiting for it. Once: a run
    /// started again after its run was stopped would take the slot of the run a request waits on.
    pub fn prefetch(self: &Arc<Self>, media: &Arc<Media>, a: &AudioTrack, n: usize, owner: &str) {
        let (this, media, a, owner) = (self.clone(), media.clone(), a.clone(), owner.to_string());
        tokio::spawn(async move {
            if let Err(e) = this.segment_tries(&media, &a, n, &owner, 1).await {
                tracing::debug!(item = %media.item_id, n, "prefetch: {e:#}");
            }
        });
    }

    /// Stop every run a session started.
    pub fn stop_owner(&self, owner: &str) -> usize {
        let runs = self.runs.lock().unwrap();
        let mut n = 0;
        for r in runs.values().flatten().filter(|r| r.owner == owner && !r.ended()) {
            r.cancel.send_replace(true);
            n += 1;
        }
        n
    }

    /// A run that will deliver segment `n` soon, or a new one starting there. None: segment `n`
    /// is in the cache after all, so no run is needed. A run writes a segment before it moves past
    /// it and before it leaves `runs`, so when no run is on its way to `n` (checked under the
    /// lock) the file is there if it was made at all, even though the caller's look missed it.
    /// `unreadable`: the file the caller couldn't read, which doesn't count as made (see [`file_id`]).
    async fn run_for(
        self: &Arc<Self>,
        media: &Arc<Media>,
        a: &AudioTrack,
        track: &str,
        n: usize,
        owner: &str,
        unreadable: Option<u64>,
    ) -> Result<Option<Arc<Run>>> {
        let segment = self.cache.path_of(&Self::segment_key(track, n));
        let made = || file_id(&segment).is_some_and(|id| Some(id) != unreadable);
        let reachable = |run: &Run| {
            let next = run.next();
            run.start <= n && n >= next && !run.ended() && media.boundaries[n] - media.boundaries[next] <= REACH_SECS
        };
        {
            let mut runs = self.runs.lock().unwrap();
            let list = runs.entry(track.to_string()).or_default();
            list.retain(|r| !r.ended());
            if let Some(r) = list.iter().find(|r| reachable(r)) {
                r.want(n);
                return Ok(Some(r.clone()));
            }
            // Before taking a slot: that could stop someone else's run.
            if made() {
                return Ok(None);
            }
        }
        let waited = Instant::now();
        let permit = self.slot().await?;
        if waited.elapsed() > Duration::from_millis(50) {
            tracing::info!(item = %media.item_id, ms = waited.elapsed().as_millis() as u64, "waited for a conversion slot");
        }
        let mut runs = self.runs.lock().unwrap();
        let list = runs.entry(track.to_string()).or_default();
        // Another request may have started one meanwhile.
        if let Some(r) = list.iter().find(|r| reachable(r)) {
            r.want(n);
            return Ok(Some(r.clone()));
        }
        if made() {
            return Ok(None);
        }
        let run = Arc::new(Run {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            start: n,
            owner: owner.to_string(),
            progress: watch::channel(Progress { next: n, done: None }).0,
            demand: watch::channel(n).0,
            last_request: Mutex::new(Instant::now()),
            cancel: watch::channel(false).0,
        });
        list.push(run.clone());
        self.started.fetch_add(1, Ordering::Relaxed);
        let (this, media, a, track, r) = (self.clone(), media.clone(), a.clone(), track.to_string(), run.clone());
        tokio::spawn(async move {
            let t0 = Instant::now();
            let result = this.run(&r, &media, &a, &track).await;
            match &result {
                Ok(()) => tracing::info!(item = %media.item_id, stream = a.stream, run = r.id, from = r.start, to = r.next(),
                    secs = t0.elapsed().as_secs_f32(), "conversion run ended"),
                Err(e) => {
                    tracing::warn!(item = %media.item_id, stream = a.stream, run = r.id, from = r.start, "conversion run failed: {e:#}");
                }
            }
            r.progress.send_modify(|p| p.done = Some(result.map_err(|e| format!("{e:#}"))));
            drop(permit);
            let mut runs = this.runs.lock().unwrap();
            if let Some(list) = runs.get_mut(&track) {
                list.retain(|x| x.id != r.id);
                if list.is_empty() {
                    runs.remove(&track);
                }
            }
        });
        Ok(Some(run))
    }

    /// A run slot; when all are taken, the run asked for least recently is stopped for it.
    async fn slot(&self) -> Result<OwnedSemaphorePermit> {
        if let Ok(p) = self.slots.clone().try_acquire_owned() {
            return Ok(p);
        }
        let victim = {
            let runs = self.runs.lock().unwrap();
            runs.values()
                .flatten()
                .filter(|r| !r.ended())
                .min_by_key(|r| *r.last_request.lock().unwrap())
                .cloned()
        };
        if let Some(v) = victim {
            tracing::info!(run = v.id, "conversion slots full: stopping the least recently used run");
            v.cancel.send_replace(true);
        }
        tokio::time::timeout(Duration::from_secs(10), self.slots.clone().acquire_owned())
            .await
            .context("no conversion slot free")?
            .map_err(Into::into)
    }

    /// The source track's frame grid (fixed-length frames), probed once from its first seconds.
    /// Only for codecs whose frames all have one length by design: Vorbis, Opus, AAC and PCM
    /// packets vary (a Vorbis track can open with seconds of long blocks only).
    async fn grid(&self, media: &Media, a: &AudioTrack) -> Option<Grid> {
        if !matches!(a.codec.as_str(), "dts" | "mp1" | "mp2" | "mp3" | "flac" | "truehd" | "mlp") {
            return None;
        }
        let key = format!("{}/{}/{}", media.item_id, media.version, a.stream);
        if let Some(g) = self.grids.lock().unwrap().get(&key) {
            return *g;
        }
        let mut cmd = tokio::process::Command::new(&self.ffprobe);
        cmd.args(["-v", "error", "-select_streams", &a.stream.to_string(), "-read_intervals", "%+3"])
            .args(["-show_entries", "frame=pts_time,nb_samples", "-of", "compact=p=0"])
            .args(media.source.input_args());
        let out = tokio::time::timeout(Duration::from_secs(15), cmd.kill_on_drop(true).output()).await;
        let grid = match out {
            Ok(Ok(o)) if o.status.success() => grid_from_frames(&String::from_utf8_lossy(&o.stdout), a.sample_rate, a.time_base),
            _ => None,
        };
        tracing::info!(item = %media.item_id, stream = a.stream, ?grid, "source frame grid");
        self.grids.lock().unwrap().insert(key, grid);
        grid
    }

    async fn run(&self, run: &Run, media: &Media, a: &AudioTrack, track: &str) -> Result<()> {
        // Listen for a stop before anything can wait: one sent since the run was made (the slot
        // taken for another run) must still end it.
        let mut cancel = crate::media::transcode::stop_signal(&run.cancel);
        if *cancel.borrow() {
            tracing::info!(item = %media.item_id, stream = a.stream, run = run.id, "conversion run stopped before it started");
            return Ok(());
        }
        let conv = a.conversion.as_ref().context("track isn't converted")?;
        let (timescale, track_id, frame_dur) = self.track_init(conv, track).await?;
        let (fl, sr) = (conv.frame_len(), conv.sample_rate as u64);
        let to_ts = |samples: u64| samples * timescale / sr;

        let t_run = Instant::now();
        let grid = self.grid(media, a).await;
        // Segment 0 is cut from the file's beginning, but its audio starts with the timeline
        // (`Media::origin`): no silence for the time before the file's start.
        let k0 = conv.frame_at(media.boundaries[run.start].max(media.origin));
        let k_first = k0.saturating_sub((WARMUP_SECS * sr as f64 / fl as f64).ceil() as u64);
        let first_sample = k_first * fl;
        let seek = first_sample as f64 / sr as f64 - PREROLL_SECS;
        let dec_args = decoder_args(media, a, conv, seek);
        let enc_args = encoder_args(conv);

        let (dec, mut enc, dec_log, enc_log, pump) = self.pipeline(
            &dec_args,
            &enc_args,
            PumpPlan {
                frame_bytes: 4 * conv.channels as usize,
                source_rate: a.sample_rate,
                rate: sr,
                grid,
                first_sample,
                started: t_run,
            },
            format!("{} stream {} run {}", media.item_id, a.stream, run.id),
        )?;
        let mut stdout = enc.stdout.take().context("no stdout")?;

        let mut demand = run.demand.subscribe();
        let segments = media.boundaries.len();
        let seg_end = |n: usize| {
            if n + 1 >= segments {
                u64::MAX
            } else {
                conv.frame_at(media.boundaries[n + 1])
            }
        };
        let mut seg = run.start;
        let mut seg_first = k0; // frame index the current segment starts at
        let mut end = seg_end(seg);
        let mut frames: Vec<Bytes> = Vec::new();
        let mut framer = Framer::new(conv.codec);
        let mut k = k_first;
        let mut buf: Vec<u8> = Vec::with_capacity(1 << 16);
        let mut chunk = vec![0u8; 1 << 16];
        let t0 = t_run;

        let finish = |seg: usize, seg_first: u64, frames: &[Bytes]| -> Vec<u8> {
            let refs: Vec<&[u8]> = frames.iter().map(AsRef::as_ref).collect();
            let base = to_ts(seg_first * fl - conv.priming());
            mp4::write_fragment(seg as u32 + 1, track_id, base, frame_dur, &refs)
        };

        'read: loop {
            if held_back(run, media, &mut demand, &mut cancel, seg).await {
                break 'read;
            }
            let got = tokio::select! {
                r = stdout.read(&mut chunk) => Some(r),
                _ = cancel.changed() => None,
            };
            let Some(got) = got else { break 'read };
            let got = got.context("read encoder output")?;
            // At the end of the track, whatever is left is the last segment.
            let eof = got == 0;
            buf.extend_from_slice(&chunk[..got]);
            let mut used = 0;
            while let Some((len, payload)) = framer.next(&buf[used..], eof)? {
                let Some(payload) = payload else {
                    used += len; // a stream header
                    continue;
                };
                let frame = Bytes::copy_from_slice(&buf[used + payload.0..used + payload.1]);
                used += len;
                if k >= k0 {
                    while k >= end {
                        let body = finish(seg, seg_first, &frames);
                        self.cache.put(&Self::segment_key(track, seg), &body).await;
                        if seg == run.start {
                            tracing::info!(item = %media.item_id, stream = a.stream, run = run.id, seg,
                                ms = t0.elapsed().as_millis() as u64, "first converted segment");
                        }
                        frames.clear();
                        seg += 1;
                        seg_first = end;
                        run.progress.send_modify(|p| p.next = seg);
                        if self.covered(track, run.id, seg).await {
                            return Ok(());
                        }
                        end = seg_end(seg);
                    }
                    frames.push(frame);
                }
                k += 1;
            }
            buf.drain(..used);
            if eof {
                ended_well(enc, dec, enc_log, dec_log, pump).await?;
                if !buf.is_empty() {
                    tracing::warn!(item = %media.item_id, bytes = buf.len(), "trailing bytes in converted audio");
                }
                if !frames.is_empty() {
                    let body = finish(seg, seg_first, &frames);
                    self.cache.put(&Self::segment_key(track, seg), &body).await;
                    run.progress.send_modify(|p| p.next = seg + 1);
                } else if seg == run.start {
                    bail!("no audio at segment {seg}");
                }
                return Ok(());
            }
        }
        // Stopped (cancelled, idle, or met covered ground): both ffmpegs are killed on drop.
        Ok(())
    }

    /// The track's init section, cached under it: its timescale, track id, and a frame's duration
    /// in its timescale.
    async fn track_init(&self, conv: &Conversion, track: &str) -> Result<(u64, u32, u32)> {
        let init = self.init(conv).await?;
        let timescale = mp4::parse(&init)?.timescale as u64;
        let track_id = mp4::track_id(&init)?;
        let (fl, sr) = (conv.frame_len(), conv.sample_rate as u64);
        if !(fl * timescale).is_multiple_of(sr) {
            bail!("timescale {timescale} doesn't fit {sr} Hz frames");
        }
        let frame_dur = (fl * timescale / sr) as u32;
        let init_key = format!("{track}/init.mp4");
        if self.cache.get(&init_key).await.is_none() {
            self.cache.put(&init_key, &init).await;
        }
        Ok((timescale, track_id, frame_dur))
    }

    /// The decoder and the encoder, started; the tasks reading their logs, and the pump moving
    /// PCM from one to the other.
    fn pipeline(
        &self,
        dec_args: &[String],
        enc_args: &[String],
        plan: PumpPlan,
        what: String,
    ) -> Result<(
        tokio::process::Child,
        tokio::process::Child,
        tokio::task::JoinHandle<String>,
        tokio::task::JoinHandle<String>,
        tokio::task::JoinHandle<Result<()>>,
    )> {
        let spawn = |args: &[String], stdin: std::process::Stdio| {
            tokio::process::Command::new("nice")
                .args(["-n", "5", self.ffmpeg.as_str()])
                .args(args)
                .stdin(stdin)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .context("ffmpeg")
        };
        let mut dec = spawn(dec_args, std::process::Stdio::null())?;
        let mut enc = spawn(enc_args, std::process::Stdio::piped())?;
        let (frames_tx, frames_rx) = tokio::sync::mpsc::unbounded_channel();
        let dec_log = tokio::spawn(decoder_log(dec.stderr.take().context("no stderr")?, frames_tx));
        let mut enc_err = enc.stderr.take().context("no stderr")?;
        let enc_log = tokio::spawn(async move {
            let mut s = String::new();
            let _ = enc_err.read_to_string(&mut s).await;
            s
        });
        let pump = tokio::spawn(pump(
            dec.stdout.take().context("no stdout")?,
            enc.stdin.take().context("no stdin")?,
            frames_rx,
            plan,
            what,
        ));
        Ok((dec, enc, dec_log, enc_log, pump))
    }

    /// Whether segment `n` is cached, or another live run of the track has started at or
    /// before it and got that far.
    async fn covered(&self, track: &str, me: u64, n: usize) -> bool {
        let other = {
            let runs = self.runs.lock().unwrap();
            runs.get(track)
                .is_some_and(|l| l.iter().any(|r| r.id != me && !r.ended() && r.start <= n && r.next() >= n))
        };
        other || tokio::fs::metadata(self.cache.path_of(&Self::segment_key(track, n))).await.is_ok()
    }
}

/// Which file is at `path` now, if any. A remade segment is a new file (`Cache::put` renames one
/// into place), so a segment that was unreadable and has been remade since no longer matches.
fn file_id(path: &std::path::Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| m.ino())
}

/// Back-pressure: waits while segment `seg` is more than LEAD ahead of what has been asked
/// for. True: stop the run (stopped, nothing asked for any more, or idle too long).
async fn held_back(run: &Run, media: &Media, demand: &mut watch::Receiver<usize>, cancel: &mut watch::Receiver<bool>, seg: usize) -> bool {
    let segments = media.boundaries.len();
    loop {
        let d = (*demand.borrow_and_update()).min(segments - 1);
        if seg >= segments || media.boundaries[seg.min(segments - 1)] - media.boundaries[d] <= LEAD_SECS {
            break;
        }
        let stop = tokio::select! {
            r = demand.changed() => r.is_err(),
            _ = cancel.changed() => true,
            () = tokio::time::sleep(IDLE) => run.last_request.lock().unwrap().elapsed() >= IDLE,
        };
        if stop {
            return true;
        }
    }
    false
}

/// At the end of the track: the encoder, the pump and the decoder all ended well (else why not,
/// from their logs).
async fn ended_well(
    mut enc: tokio::process::Child,
    mut dec: tokio::process::Child,
    enc_log: tokio::task::JoinHandle<String>,
    dec_log: tokio::task::JoinHandle<String>,
    pump: tokio::task::JoinHandle<Result<()>>,
) -> Result<()> {
    let status = enc.wait().await?;
    if !status.success() {
        bail!("encoder: {}", crate::admin::logbuf::scrub(enc_log.await.unwrap_or_default().trim()));
    }
    match pump.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => bail!("{e:#}: {}", crate::admin::logbuf::scrub(dec_log.await.unwrap_or_default().trim())),
        Err(e) => bail!("pump: {e}"),
    }
    if !dec.wait().await?.success() {
        bail!("decoder: {}", crate::admin::logbuf::scrub(dec_log.await.unwrap_or_default().trim()));
    }
    Ok(())
}

/// The decoder's command line: PCM at the output rate and layout from `seek` on.
fn decoder_args(media: &Media, a: &AudioTrack, conv: &Conversion, seek: f64) -> Vec<String> {
    let sr = conv.sample_rate as u64;
    let channels = conv.channels.to_string();

    // Decoder: PCM at the output rate and layout. `ashowinfo` reports each decoded frame's
    // own timestamp (before resampling) on stderr; the pump works out from them where the
    // PCM starts. `aresample=async=1` keeps sync over real gaps in the source.
    let mut dec_args: Vec<String> = ["-nostdin", "-hide_banner", "-nostats", "-v", "info", "-noaccurate_seek"]
        .map(String::from)
        .to_vec();
    // `seek` is on the file's own clock; ffmpeg adds the file's start time to `-ss`.
    let seek = media.copy_seek(seek);
    if seek > 0.0 {
        dec_args.extend(["-ss".into(), format!("{seek:.3}")]);
    }
    dec_args.extend(media.source.input_args());
    dec_args.extend([
        "-copyts".into(),
        "-map".into(),
        format!("0:{}", a.stream),
        "-vn".into(),
        "-sn".into(),
        "-dn".into(),
    ]);
    dec_args.extend([
        "-af".into(),
        format!("ashowinfo,aresample={sr}:async=1"),
        "-ac".into(),
        channels.clone(),
    ]);
    dec_args.extend(["-f", "f32le", "pipe:1"].map(String::from));
    dec_args
}

/// The encoder's command line: raw PCM in, the conversion's codec out.
fn encoder_args(conv: &Conversion) -> Vec<String> {
    let sr = conv.sample_rate as u64;
    // Encoder: from exactly the run's first sample, so its frames are on the grid.
    let mut enc_args: Vec<String> = ["-nostdin", "-v", "error", "-f", "f32le", "-ar"].map(String::from).to_vec();
    enc_args.extend([
        sr.to_string(),
        "-ch_layout".into(),
        conv.layout().into(),
        "-i".into(),
        "pipe:0".into(),
    ]);
    enc_args.extend(conv.encoder_args());
    enc_args.extend(["-f".into(), conv.raw_format().into(), "pipe:1".into()]);

    enc_args
}

/// What the pump needs to place the decoded PCM on the output grid.
struct PumpPlan {
    /// Bytes per PCM sample frame (f32 × channels).
    frame_bytes: usize,
    source_rate: i64,
    rate: u64,
    grid: Option<Grid>,
    /// The output sample the encoder must start at.
    first_sample: u64,
    started: Instant,
}

/// Decoded frames used to place the PCM (about a second of the pre-roll).
const ANCHOR_SECS: f64 = 1.0;

/// Moves PCM from the decoder to the encoder, starting at exactly `first_sample`.
///
/// Where the decoder's PCM starts is worked out from the decoded frames' own timestamps:
/// `pts - samples before it` for each, which is the same value for every frame of a correctly
/// stamped stream. Their median ignores per-packet jitter (MKV's millisecond rounding, muxers
/// that stamp 10 ms steps for 10.67 ms DTS frames…), and on a fixed source frame grid it is
/// snapped to a whole frame. Every run of a track thus places the same samples at the same time.
async fn pump(
    mut from: tokio::process::ChildStdout,
    mut to: tokio::process::ChildStdin,
    mut frames: tokio::sync::mpsc::UnboundedReceiver<(i64, i64)>,
    plan: PumpPlan,
    what: String,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut buf: Vec<u8> = Vec::with_capacity(1 << 20);
    let (mut eof, mut frames_done) = (false, false);
    let mut offsets: Vec<i64> = Vec::new();
    let mut before = 0i64; // source samples decoded before the frame
    let enough = (ANCHOR_SECS * plan.source_rate as f64) as i64;
    while !((before >= enough && offsets.len() >= 16) || (eof && frames_done)) {
        tokio::select! {
            r = from.read_buf(&mut buf), if !eof => if r? == 0 { eof = true },
            f = frames.recv(), if !frames_done => match f {
                Some((pts, n)) => {
                    offsets.push(pts - before);
                    before += n;
                }
                None => frames_done = true,
            },
        }
        if buf.len() > 256 << 20 {
            bail!("no frame timestamps from the decoder");
        }
    }
    if offsets.is_empty() {
        bail!("no audio decoded");
    }
    let (lo, hi) = (*offsets.iter().min().unwrap(), *offsets.iter().max().unwrap());
    offsets.sort_unstable();
    let median = offsets[offsets.len() / 2];
    let start = match plan.grid {
        Some(g) => g.origin + ((median - g.origin) as f64 / g.len as f64).round() as i64 * g.len,
        None => median,
    };
    let start_out = (start as f64 * plan.rate as f64 / plan.source_rate as f64).round() as i64;
    let skip = plan.first_sample as i64 - start_out;
    tracing::info!(
        run = what,
        frames = offsets.len(),
        jitter = hi - lo,
        snap = start - median,
        skip,
        ms = plan.started.elapsed().as_millis() as u64,
        "conversion input placed"
    );
    if skip < 0 {
        // The source starts later than the run (audio delayed against the video): silence.
        to.write_all(&vec![0u8; (-skip) as usize * plan.frame_bytes]).await?;
    }
    let mut drop_bytes = skip.max(0) as usize * plan.frame_bytes;
    loop {
        let d = drop_bytes.min(buf.len());
        buf.drain(..d);
        drop_bytes -= d;
        if !buf.is_empty() {
            to.write_all(&buf).await?;
            buf.clear();
        }
        if eof {
            break;
        }
        if from.read_buf(&mut buf).await? == 0 {
            eof = true;
        }
    }
    to.shutdown().await?;
    Ok(())
}

/// Reads the decoder's stderr: `ashowinfo` lines become (pts, samples) of each decoded frame;
/// the rest is returned (the tail) for error messages.
async fn decoder_log(stderr: tokio::process::ChildStderr, frames: tokio::sync::mpsc::UnboundedSender<(i64, i64)>) -> String {
    use tokio::io::AsyncBufReadExt;
    let mut lines = tokio::io::BufReader::new(stderr).lines();
    let mut tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.contains("Parsed_ashowinfo") {
            if let Some(f) = showinfo_frame(&line) {
                let _ = frames.send(f);
            }
            continue;
        }
        if tail.len() == 20 {
            tail.pop_front();
        }
        tail.push_back(line);
    }
    tail.into_iter().collect::<Vec<_>>().join("\n")
}

/// `… n:12 pts:1271664 pts_time:26.493 … nb_samples:512 …` → (1271664, 512).
fn showinfo_frame(line: &str) -> Option<(i64, i64)> {
    let field = |name: &str| {
        line.split_whitespace()
            .find_map(|w| w.strip_prefix(name))
            .and_then(|v| v.parse::<i64>().ok())
    };
    Some((field("pts:")?, field("nb_samples:")?))
}

/// The fixed frame length of a source track from its first frames (`nb_samples`, ignoring the
/// first and last), if it is long enough to snap to: well above the container's timestamp
/// precision in samples (MKV's millisecond is 48 samples at 48 kHz; TrueHD's 40-sample frames
/// are below it, Vorbis's vary).
fn grid_from_frames(text: &str, sample_rate: i64, time_base: f64) -> Option<Grid> {
    let mut frames = Vec::new();
    for line in text.lines() {
        let (mut pts, mut n) = (None, None);
        for field in line.split('|') {
            match field.split_once('=') {
                Some(("pts_time", v)) => pts = v.parse::<f64>().ok(),
                Some(("nb_samples", v)) => n = v.parse::<i64>().ok(),
                _ => {}
            }
        }
        frames.push((pts?, n?));
    }
    if frames.len() < 6 || sample_rate <= 0 {
        return None;
    }
    let len = frames[1].1;
    if len <= 0 || !frames[1..frames.len() - 1].iter().all(|f| f.1 == len) {
        return None;
    }
    let precision = time_base * sample_rate as f64; // one timestamp tick, in samples
    if (len as f64) < 4.0 * precision {
        return None;
    }
    Some(Grid {
        origin: (frames[0].0 * sample_rate as f64).round() as i64,
        len,
    })
}

/// Splits the encoder's raw output into access units.
struct Framer {
    codec: Codec,
    /// FLAC: the stream header (`fLaC` and its metadata blocks) has been passed.
    header_done: bool,
}

impl Framer {
    fn new(codec: Codec) -> Self {
        Framer { codec, header_done: false }
    }

    /// The next piece at the start of `buf`: (bytes to consume, the access unit's range, or None
    /// for a stream header to skip), or None when more data is needed. `eof`: `buf` is all that
    /// is left.
    fn next(&mut self, buf: &[u8], eof: bool) -> Result<Option<(usize, Option<(usize, usize)>)>> {
        if self.codec != Codec::Flac {
            return Ok(next_frame(self.codec, buf)?.map(|(len, payload)| (len, Some(payload))));
        }
        if !self.header_done {
            let Some(len) = flac_stream_header(buf)? else {
                if eof && !buf.is_empty() {
                    bail!("FLAC stream header cut short");
                }
                return Ok(None);
            };
            self.header_done = true;
            return Ok(Some((len, None)));
        }
        Ok(next_flac_frame(buf, eof)?.map(|len| (len, Some((0, len)))))
    }
}

/// The length of a FLAC stream's header (`fLaC` and the metadata blocks), or None when more
/// data is needed.
fn flac_stream_header(buf: &[u8]) -> Result<Option<usize>> {
    if buf.len() < 4 {
        return Ok(None);
    }
    if &buf[..4] != b"fLaC" {
        bail!("no FLAC stream marker");
    }
    let mut pos = 4;
    loop {
        if buf.len() < pos + 4 {
            return Ok(None);
        }
        let last = buf[pos] & 0x80 != 0;
        let len = ((buf[pos + 1] as usize) << 16) | ((buf[pos + 2] as usize) << 8) | buf[pos + 3] as usize;
        pos += 4 + len;
        if buf.len() < pos {
            return Ok(None);
        }
        if last {
            return Ok(Some(pos));
        }
    }
}

fn crc8(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |mut c, b| {
        c ^= b;
        for _ in 0..8 {
            c = if c & 0x80 != 0 { (c << 1) ^ 0x07 } else { c << 1 };
        }
        c
    })
}

fn crc16_step(mut c: u16, b: u8) -> u16 {
    c ^= (b as u16) << 8;
    for _ in 0..8 {
        c = if c & 0x8000 != 0 { (c << 1) ^ 0x8005 } else { c << 1 };
    }
    c
}

/// A FLAC frame header (fixed block size) at the start of `buf` with a good CRC-8: its frame
/// number. None: not one (or cut short).
fn flac_frame_header(buf: &[u8]) -> Option<u64> {
    if buf.len() < 6 || buf[0] != 0xFF || buf[1] != 0xF8 {
        return None;
    }
    let (bs, rate) = (buf[2] >> 4, buf[2] & 0x0F);
    let (chans, bits) = (buf[3] >> 4, (buf[3] >> 1) & 7);
    if bs == 0 || rate == 15 || chans > 10 || bits == 3 || buf[3] & 1 != 0 {
        return None;
    }
    // The frame number, UTF-8 style.
    let first = buf[4];
    let extra = match first.leading_ones() {
        0 => 0,
        n @ 2..=7 => n as usize - 1,
        _ => return None,
    };
    let mut number = if extra == 0 {
        (first & 0x7F) as u64
    } else {
        (0xFFu16 >> (extra + 2)) as u64 & first as u64
    };
    let mut pos = 5;
    for _ in 0..extra {
        let b = *buf.get(pos)?;
        if b & 0xC0 != 0x80 {
            return None;
        }
        number = (number << 6) | (b & 0x3F) as u64;
        pos += 1;
    }
    pos += match bs {
        6 => 1,
        7 => 2,
        _ => 0,
    };
    pos += match rate {
        12 => 1,
        13 | 14 => 2,
        _ => 0,
    };
    let crc = *buf.get(pos)?;
    (crc8(&buf[..pos]) == crc).then_some(number)
}

/// The length of the FLAC frame at the start of `buf`: up to the next frame header (the next
/// frame number, a good CRC-8) where the frame's CRC-16 checks out, or all of `buf` at the end
/// of the stream. None: more data is needed.
fn next_flac_frame(buf: &[u8], eof: bool) -> Result<Option<usize>> {
    if buf.is_empty() {
        return Ok(None);
    }
    if buf.len() < 16 && !eof {
        return Ok(None);
    }
    let Some(number) = flac_frame_header(buf) else {
        bail!("lost FLAC frame sync")
    };
    let mut crc = 0u16;
    for p in 0..buf.len() {
        if p >= 6 && crc == 0 && buf[p] == 0xFF && flac_frame_header(&buf[p..]) == Some(number + 1) {
            return Ok(Some(p));
        }
        crc = crc16_step(crc, buf[p]);
    }
    if eof {
        if crc != 0 {
            bail!("FLAC frame {number} fails its CRC");
        }
        return Ok(Some(buf.len()));
    }
    Ok(None)
}

/// The next access unit at the start of `buf`: (bytes to consume, payload range), or None when
/// more data is needed. ADTS headers are dropped (MP4 carries raw AAC); E-AC-3 sync frames are
/// kept whole.
fn next_frame(codec: Codec, buf: &[u8]) -> Result<Option<(usize, (usize, usize))>> {
    match codec {
        Codec::Aac => {
            if buf.len() < 7 {
                return Ok(None);
            }
            if buf[0] != 0xFF || buf[1] & 0xF6 != 0xF0 {
                bail!("lost ADTS sync");
            }
            let len = (((buf[3] & 3) as usize) << 11) | ((buf[4] as usize) << 3) | ((buf[5] >> 5) as usize);
            let header = if buf[1] & 1 == 1 { 7 } else { 9 };
            if buf[6] & 3 != 0 {
                bail!("ADTS frame with several raw blocks");
            }
            if len <= header {
                bail!("bad ADTS frame length {len}");
            }
            Ok((buf.len() >= len).then_some((len, (header, len))))
        }
        Codec::Eac3 => {
            if buf.len() < 6 {
                return Ok(None);
            }
            if buf[0] != 0x0B || buf[1] != 0x77 {
                bail!("lost E-AC-3 sync");
            }
            if buf[5] >> 3 <= 10 {
                bail!("AC-3 frame where E-AC-3 was expected");
            }
            let len = (((((buf[2] & 7) as usize) << 8) | buf[3] as usize) + 1) * 2;
            let blocks = if buf[4] >> 6 == 3 {
                6
            } else {
                [1, 2, 3, 6][((buf[4] >> 4) & 3) as usize]
            };
            if blocks != 6 {
                bail!("E-AC-3 frame of {blocks} blocks");
            }
            Ok((buf.len() >= len).then_some((len, (0, len))))
        }
        Codec::Flac => bail!("FLAC is split by the framer"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A segment finished between a request's look in the cache and its search for a run
    /// used to start a second run there, whose different warm-up rewrote the finished segment.
    #[tokio::test]
    async fn a_segment_made_meanwhile_starts_no_run() {
        let dir = std::env::temp_dir().join(format!("moonrise-audio-made-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = Arc::new(Cache::new(dir.clone(), 1 << 30));
        let conv = Conversion::for_source(2, 48000);
        let (media, track) = crate::media::audio_run_golden_tests::film_and_track(conv.clone());
        let media = Arc::new(media);
        let c = Arc::new(Converter::new(
            "/nonexistent/ffmpeg".into(),
            "/nonexistent/ffprobe".into(),
            cache.clone(),
            2,
        ));
        let key = Converter::track_key(&media, &track, &conv);
        cache.put(&Converter::segment_key(&key, 1), b"made").await;

        assert!(c.run_for(&media, &track, &key, 1, "s", None).await.unwrap().is_none());
        assert_eq!(c.started.load(Ordering::Relaxed), 0);
        // A segment that really is missing still gets its run.
        assert!(c.run_for(&media, &track, &key, 2, "s", None).await.unwrap().is_some());
        assert_eq!(c.started.load(Ordering::Relaxed), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A cached segment that is there but can't be read used to use up every try and fail
    /// the request ("not produced"); a new run makes it again. The tests can run as root, which
    /// `chmod 000` doesn't stop, so the file is a link to a folder: it exists, reading it fails,
    /// and the run's rename replaces it, as with an unreadable file.
    #[tokio::test]
    async fn an_unreadable_segment_is_made_again() {
        use crate::media::audio_run_golden_tests::{fake_ffmpeg, init};
        let dir = std::env::temp_dir().join(format!("moonrise-audio-unreadable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ffmpeg = fake_ffmpeg(&dir, 0);
        let cache = Arc::new(Cache::new(dir.join("cache"), 1 << 30));
        let conv = Conversion::for_source(2, 48000);
        cache.put(&format!("conv-init/v{CONV_VERSION}/{}.mp4", conv.tag()), &init()).await;
        let (media, track) = crate::media::audio_run_golden_tests::film_and_track(conv.clone());
        let media = Arc::new(media);
        let c = Arc::new(Converter::new(ffmpeg, "/nonexistent/ffprobe".into(), cache.clone(), 2));
        let key = Converter::segment_key(&Converter::track_key(&media, &track, &conv), 0);
        let path = cache.path_of(&key);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&dir, &path).unwrap();
        assert!(cache.get(&key).await.is_none() && path.exists());

        let got = c.segment(&media, &track, 0, "s").await.unwrap();

        assert!(!got.is_empty());
        assert_eq!(c.started.load(Ordering::Relaxed), 1);
        assert_eq!(cache.get(&key).await, Some(got));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Remaking a segment keeps the rule that no run rewrites a finished one: only the very file
    /// that failed is remade. If it has been
    /// replaced since (another request's run), that is a finished segment and starts no run.
    #[tokio::test]
    async fn only_the_file_that_failed_counts_as_unreadable() {
        let dir = std::env::temp_dir().join(format!("moonrise-audio-replaced-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = Arc::new(Cache::new(dir.clone(), 1 << 30));
        let conv = Conversion::for_source(2, 48000);
        let (media, track) = crate::media::audio_run_golden_tests::film_and_track(conv.clone());
        let media = Arc::new(media);
        let c = Arc::new(Converter::new(
            "/nonexistent/ffmpeg".into(),
            "/nonexistent/ffprobe".into(),
            cache.clone(),
            2,
        ));
        let key = Converter::track_key(&media, &track, &conv);
        let segment = Converter::segment_key(&key, 1);
        cache.put(&segment, b"bad").await;
        let failed = file_id(&cache.path_of(&segment));
        cache.put(&segment, b"made again").await;
        assert_ne!(failed, file_id(&cache.path_of(&segment)));

        assert!(c.run_for(&media, &track, &key, 1, "s", failed).await.unwrap().is_none());
        assert_eq!(c.started.load(Ordering::Relaxed), 0);
        // The file that failed is still there: it gets its run.
        let failed = file_id(&cache.path_of(&segment));
        assert!(c.run_for(&media, &track, &key, 1, "s", failed).await.unwrap().is_some());
        assert_eq!(c.started.load(Ordering::Relaxed), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn choices() {
        let c = Conversion::for_source(8, 48000);
        assert_eq!((c.codec, c.channels, c.bitrate, c.codecs_attr()), (Codec::Eac3, 6, 640_000, "ec-3"));
        let c = Conversion::for_source(2, 44100);
        assert_eq!(
            (c.codec, c.channels, c.sample_rate, c.codecs_attr()),
            (Codec::Aac, 2, 44100, "mp4a.40.2")
        );
        assert_eq!(Conversion::for_source(1, 96000).sample_rate, 48000);
        assert_eq!(Conversion::for_source(1, 96000).bitrate, 96_000);
        assert_eq!(Conversion::for_source(6, 48000).tag(), "eac3-6ch-640k-48000");
    }

    #[test]
    fn lossless_choices() {
        // TrueHD / DTS-HD MA 7.1 at 48 kHz: FLAC, all 8 channels.
        let c = Conversion::lossless(true, 8, 48000).unwrap();
        assert_eq!(
            (c.codec, c.channels, c.sample_rate, c.codecs_attr(), c.layout()),
            (Codec::Flac, 8, 48000, "fLaC", "7.1")
        );
        assert_eq!(c.tag(), "flac-8ch-5529k-48000-f1024");
        assert_eq!((c.frame_len(), c.priming()), (1024, 0));
        let a = c.encoder_args().join(" ");
        assert!(
            a.contains("-c:a flac -sample_fmt s32 -frame_size 1024 -ac 8 -ar 48000") && !a.contains("-b:a"),
            "{a}"
        );
        assert_eq!(c.raw_format(), "flac");
        assert_eq!(
            Conversion::lossless(true, 6, 44100).map(|c| (c.channels, c.sample_rate, c.layout())),
            Some((6, 44100, "5.1"))
        );
        assert_eq!(
            Conversion::lossless(true, 7, 96000).map(|c| (c.channels, c.sample_rate, c.layout())),
            Some((7, 48000, "6.1"))
        );
        // Stereo and odd channel counts, and lossy sources, keep the usual conversion.
        assert!(Conversion::lossless(true, 2, 48000).is_none());
        assert!(Conversion::lossless(true, 4, 48000).is_none());
        assert!(Conversion::lossless(false, 8, 48000).is_none());
        // The lossy conversions are unchanged.
        let e = Conversion::for_source(8, 48000);
        assert_eq!(
            e.encoder_args().join(" ") + " -f " + e.raw_format(),
            "-c:a eac3 -b:a 640000 -ac 6 -ar 48000 -f eac3"
        );
        let aac = Conversion::for_source(2, 48000);
        assert_eq!(
            aac.encoder_args().join(" ") + " -f " + aac.raw_format(),
            "-c:a aac -b:a 192000 -ac 2 -ar 48000 -f adts"
        );
        // Frame k of FLAC starts at exactly k*1024 samples (no priming).
        assert_eq!(c.frame_at(0.0), 0);
        assert_eq!(c.frame_at(1024.0 / 48000.0), 1);
        assert_eq!(c.frame_at(1000.0 / 48000.0), 1);
    }

    /// A FLAC frame (fixed block size) numbered `number`, `payload` bytes after the header,
    /// with correct CRCs.
    fn flac_frame(number: u64, payload: &[u8]) -> Vec<u8> {
        // Block size code 10 (1024), rate code 10 (48 kHz); 8 channels (code 7), 24 bits (code 6).
        let mut f = vec![0xFF, 0xF8, 0xAA, (7 << 4) | (6 << 1)];
        if number < 0x80 {
            f.push(number as u8);
        } else {
            assert!(number < 0x800);
            f.extend([0xC0 | (number >> 6) as u8, 0x80 | (number & 0x3F) as u8]);
        }
        f.push(crc8(&f));
        f.extend_from_slice(payload);
        let crc = f.iter().fold(0u16, |c, b| crc16_step(c, *b));
        f.extend(crc.to_be_bytes());
        f
    }

    #[test]
    fn flac_framing() {
        assert_eq!(flac_frame_header(&flac_frame(5, &[1, 2, 3])), Some(5));
        assert_eq!(flac_frame_header(&flac_frame(300, &[1, 2, 3])), Some(300));
        // A bad CRC-8 isn't a header.
        let mut bad = flac_frame(5, &[1, 2, 3]);
        bad[5] ^= 1;
        assert_eq!(flac_frame_header(&bad), None);

        // A stream: header (STREAMINFO, then the last block: PADDING), frames 0..3. Payloads hold
        // bytes that look like a sync code (0xFF 0xF8) to show they aren't taken for a frame start.
        let mut stream = b"fLaC".to_vec();
        stream.extend([0x00, 0, 0, 34]);
        stream.extend([0u8; 34]);
        stream.extend([0x81, 0, 0, 4, 0, 0, 0, 0]);
        let header_len = stream.len();
        let frames: Vec<Vec<u8>> = (0..4)
            .map(|n| flac_frame(n, &[n as u8, 0xFF, 0xF8, 0xAA, 0x7C, n as u8 + 1, 9, 9, 9]))
            .collect();
        for f in &frames {
            stream.extend(f);
        }
        let mut framer = Framer::new(Codec::Flac);
        // Fed a byte at a time: nothing until a whole piece (and the next frame's header) is there.
        let mut buf = Vec::new();
        let mut got: Vec<(usize, Option<(usize, usize)>)> = Vec::new();
        for (i, b) in stream.iter().enumerate() {
            buf.push(*b);
            let eof = i + 1 == stream.len();
            while let Some((len, payload)) = framer.next(&buf, eof).unwrap() {
                got.push((len, payload));
                buf.drain(..len);
            }
        }
        assert!(buf.is_empty());
        assert_eq!(got[0], (header_len, None));
        let lens: Vec<usize> = got[1..]
            .iter()
            .map(|(l, p)| {
                assert_eq!(*p, Some((0, *l)));
                *l
            })
            .collect();
        assert_eq!(lens, frames.iter().map(Vec::len).collect::<Vec<_>>());

        // The last frame, damaged: an error at the end, not a frame.
        let mut cut = frames[0].clone();
        let n = cut.len();
        cut[n - 1] ^= 0x55;
        assert!(next_flac_frame(&cut, true).is_err());
        assert_eq!(next_flac_frame(&cut, false).unwrap(), None);
        // Not a frame header at all.
        assert!(next_flac_frame(&[0u8; 32], false).is_err());
        assert!(Framer::new(Codec::Flac).next(b"RIFF....", false).is_err());
    }

    #[test]
    fn frame_grid_and_segments() {
        let aac = Conversion::for_source(2, 48000);
        // Frame k is at (k*1024 - 1024)/48000 s: frame 1 is at 0.
        assert_eq!(aac.frame_at(0.0), 1);
        assert_eq!(aac.frame_at(1024.0 / 48000.0), 2);
        assert_eq!(aac.frame_at(1000.0 / 48000.0), 2);
        let eac3 = Conversion::for_source(6, 48000);
        // (k*1536 - 256)/48000: frame 0 is at -5.3 ms, frame 1 at 26.7 ms.
        assert_eq!(eac3.frame_at(0.0), 1);
        assert_eq!(eac3.frame_at(6.0), (6.0f64 * 48000.0 + 256.0).div_euclid(1536.0) as u64 + 1);
        // Consecutive segments partition the frames: segment n ends where n+1 starts.
        let b = [0.0, 6.006, 12.012, 18.5];
        let starts: Vec<u64> = b.iter().map(|t| eac3.frame_at(*t)).collect();
        assert!(starts.windows(2).all(|w| w[0] < w[1]));
        for (t, k) in b.iter().zip(&starts) {
            let time = (*k as f64 * 1536.0 - 256.0) / 48000.0;
            assert!(time >= *t - 1e-9 && time - 1536.0 / 48000.0 < *t, "{t} {k}");
        }
    }

    #[test]
    fn source_grid() {
        let dts: String = (0..20)
            .map(|i| format!("pts_time={:.6}|nb_samples=512\n", 0.05 + i as f64 * 512.0 / 48000.0))
            .collect();
        assert_eq!(grid_from_frames(&dts, 48000, 0.001), Some(Grid { origin: 2400, len: 512 }));
        // Vorbis: frame lengths vary.
        let vorbis = "pts_time=0.1|nb_samples=1024\npts_time=0.12|nb_samples=1024\npts_time=0.14|nb_samples=576\n\
                      pts_time=0.15|nb_samples=128\npts_time=0.16|nb_samples=128\npts_time=0.17|nb_samples=128\npts_time=0.18|nb_samples=128\n";
        assert_eq!(grid_from_frames(vorbis, 48000, 0.001), None);
        // TrueHD: 40-sample frames are below MKV's timestamp precision; fine in MP4's.
        let thd: String = (0..20)
            .map(|i| format!("pts_time={}|nb_samples=40\n", i as f64 * 40.0 / 48000.0))
            .collect();
        assert_eq!(grid_from_frames(&thd, 48000, 0.001), None);
        assert!(grid_from_frames(&thd, 48000, 1.0 / 48000.0).is_some());
    }

    #[test]
    fn showinfo_lines() {
        let l = "[Parsed_ashowinfo_0 @ 0x7898a8003240] n:1 pts:1298280 pts_time:27.0475 fmt:s32p channels:8 chlayout:7.1 rate:48000 nb_samples:512 checksum:5B119717";
        assert_eq!(showinfo_frame(l), Some((1298280, 512)));
        assert_eq!(
            showinfo_frame("[Parsed_ashowinfo_0 @ 0x1] n:0 pts:NOPTS pts_time:NOPTS nb_samples:512"),
            None
        );
    }

    #[test]
    fn frame_parsing() {
        // ADTS, no CRC, 10-byte frame: header 7 + 3 payload.
        let len = 10usize;
        let adts = [
            0xFF,
            0xF1,
            0x50,
            0x80 | ((len >> 11) as u8 & 3),
            (len >> 3) as u8,
            ((len & 7) << 5) as u8 | 0x1F,
            0xFC,
            1,
            2,
            3,
        ];
        assert_eq!(next_frame(Codec::Aac, &adts).unwrap(), Some((10, (7, 10))));
        assert_eq!(next_frame(Codec::Aac, &adts[..8]).unwrap(), None);
        assert!(next_frame(Codec::Aac, &[0, 1, 2, 3, 4, 5, 6, 7]).is_err());
        // E-AC-3: frmsiz 3 -> 8 bytes, fscod 0 numblkscod 3, bsid 16.
        let eac3 = [0x0B, 0x77, 0x00, 0x03, 0x30, 16 << 3, 0, 0];
        assert_eq!(next_frame(Codec::Eac3, &eac3).unwrap(), Some((8, (0, 8))));
        let ac3 = [0x0B, 0x77, 0x00, 0x03, 0x30, 8 << 3, 0, 0];
        assert!(next_frame(Codec::Eac3, &ac3).is_err());
    }
}
