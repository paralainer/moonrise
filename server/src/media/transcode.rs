//! Video that is re-encoded on the GPU (NVENC): the quality ladder, codecs that can't be copied
//! into HLS (MPEG-4 Part 2, MPEG-2, AV1, VC-1, 10-bit H.264…), HDR tone-mapped to SDR for
//! displays without HDR.
//!
//! **One timeline for every variant.** A transcoded variant has exactly the copied original's
//! segments: each segment starts with an IDR frame at the same source keyframe
//! (`-force_key_frames` at the boundaries, `-forced-idr`), timestamps stay the file's own
//! (`-copyts`, `-enc_time_base demux`, `-fps_mode passthrough`), and there are no B-frames, so a
//! fragment's decode time is its presentation time and needs no edit list. Variants can be
//! switched at any segment, and the audio renditions (cut on the same boundaries) fit all.
//!
//! **Rolling runs** (as for converted audio, `audio.rs`). One ffmpeg encodes forward from a
//! segment boundary; ffmpeg writes fragmented MP4 with a fragment per keyframe, and fragments
//! are grouped into segments here by their decode time. A run keeps going while the player asks
//! for segments within `REACH` of its progress, stays at most `LEAD` of media ahead of the last
//! request (pipe back-pressure pauses ffmpeg), and stops when it meets cached ground, after
//! `IDLE` without requests, when its session seeks elsewhere (a new run there replaces it) or
//! ends (`DELETE /v1/s/{sid}`). Finished segments go to the disk cache.
//!
//! **GPU budget.** At most `MOONRISE_TRANSCODE_JOBS` runs at once (the GPU is shared with
//! other services on the host). `admit` tells the playback endpoint when they are all in use by other
//! sessions, so it can answer with the copied variant or `supported: false` instead.
//!
//! Decoding is on the GPU (NVDEC) for 8-bit H.264, HEVC, AV1, VP9, MPEG-2 and VC-1, on the CPU
//! otherwise (MPEG-4 Part 2 with its packed B-frames, 10-bit H.264, 4:2:2/4:4:4).

use std::fmt::Write as _;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
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
    media::hwaccel::{self, Backend, Hw},
    media::mp4,
    media::{Media, VideoTrack},
    storage::cache::Cache,
};

/// Bump when transcoded output changes, so cached segments aren't mixed with new ones.
pub const TRANSCODE_VERSION: u32 = 1;
/// Owners of runs made for downloads (`offline.rs`) start with this. They come after streams:
/// they never stop a stream's run for a slot, a stream may stop theirs, and new streams are
/// admitted as if they weren't there. Their requests jump around (the app fetches where the
/// viewer seeks while it downloads in order), so they don't stop their other runs either.
pub const DOWNLOAD_OWNER: &str = "dl:";

fn is_download(owner: &str) -> bool {
    owner.starts_with(DOWNLOAD_OWNER)
}
/// A request this far (in media time) past a run's progress waits for that run.
const REACH_SECS: f64 = 30.0;
/// How far a run encodes ahead of the last segment requested from it.
const LEAD_SECS: f64 = 40.0;
/// A run with nothing requested for this long stops (and frees its GPU memory).
const IDLE: Duration = Duration::from_secs(60);
/// A run asked for within this long counts as in use when admitting new sessions.
const ACTIVE: Duration = Duration::from_secs(30);
/// How long a request waits for its segment.
const WAIT: Duration = Duration::from_secs(60);
/// A fragment starting this close before a boundary belongs to the segment after it.
const TOL: f64 = 0.02;
/// Keyframes are forced this much before each boundary: the first frame at or after the time
/// becomes the IDR, so tiny differences between the index and the stream don't matter.
const FORCE_EARLY: f64 = 0.005;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    H264,
    Hevc,
}

/// One rung of a title's ladder: the copied original, or a transcode.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Variant {
    /// Stable within a title: part of URLs and cache keys.
    pub id: String,
    pub label: String,
    /// Average bitrate (the source's for the copy).
    pub bitrate: i64,
    /// Peak bitrate (HLS `BANDWIDTH`).
    #[serde(skip)]
    pub peak: i64,
    /// The encoder's VBV buffer, in bits.
    #[serde(skip)]
    bufsize: i64,
    pub width: i64,
    pub height: i64,
    /// The codec served: the source's when copied.
    pub codec: String,
    /// SDR, PQ or HLG.
    pub range: &'static str,
    pub transcode: bool,
    /// HDR mapped to SDR.
    pub tonemap: bool,
    #[serde(skip)]
    pub codecs_attr: String,
    #[serde(skip)]
    enc: Option<Codec>,
    /// `-level` for the encoder.
    #[serde(skip)]
    level: String,
}

impl Variant {
    pub fn is_copy(&self) -> bool {
        !self.transcode
    }
}

/// The quality presets, as the app offers them (`QualityOption`): bitrate, the box the picture
/// is fitted into, label.
const PRESETS: [(i64, i64, i64, &str); 5] = [
    (40_000_000, 3840, 2160, "4K · 40 Mbps"),
    (20_000_000, 1920, 1080, "1080p · 20 Mbps"),
    (10_000_000, 1920, 1080, "1080p · 10 Mbps"),
    (4_000_000, 1280, 720, "720p · 4 Mbps"),
    (1_500_000, 854, 480, "480p · 1.5 Mbps"),
];

/// What a title can be played as, best first. `transcode`: the GPU path is on.
///
/// - The copied original, unless it can't be copied, or it is HDR for
///   a display without HDR (then the original is a tone-mapped transcode; with transcoding off
///   the copy is still offered, unlabelled, as before).
/// - Otherwise "Original" is a transcode at the source's size (at most 4K) and a bitrate that
///   keeps its quality (`original_bitrate`).
/// - Then the presets below 90 % of the source's bitrate (as the app lists them), fitted into
///   their box, never upscaled.
///
/// HDR is kept (HEVC Main 10) for HDR displays, else tone-mapped (H.264). SDR is H.264.
pub fn ladder(v: &VideoTrack, hdr_display: bool, transcode: bool) -> Vec<Variant> {
    ladder_with(hwaccel::current(), v, hdr_display, transcode)
}

/// [`ladder`] for the backend `hw`: without an HEVC Main 10 encoder HDR is tone-mapped in every
/// transcode (the copy keeps it), and the CPU makes at most 1080p.
pub fn ladder_with(hw: &Hw, v: &VideoTrack, hdr_display: bool, transcode: bool) -> Vec<Variant> {
    let hdr = v.hdr();
    // The screen shows HDR (the copy keeps it); the transcodes keep it too when they can.
    let display_hdr = hdr && hdr_display;
    let keep_hdr = display_hdr && hw.hevc10;
    let tonemap = hdr && !keep_hdr;
    // Nothing transcodes what this hardware can't keep up with, or HDR it can't tone-map.
    let can_transcode = transcode
        && hw.available
        && !v.dovi_only
        && v.width > 0
        && v.height > 0
        && hw.too_heavy(v).is_none()
        && (!tonemap || hw.can_tonemap());
    let copy_ok = v.copyable() && !(hdr && !display_hdr && can_transcode);
    let mut out = Vec::new();
    if copy_ok {
        out.push(Variant {
            id: "copy".into(),
            label: "Original".into(),
            bitrate: v.bitrate,
            peak: v.bitrate,
            bufsize: 0,
            width: v.width,
            height: v.height,
            codec: v.codec.clone(),
            range: v.range,
            transcode: false,
            tonemap: false,
            codecs_attr: v.codecs_attr.clone().unwrap_or_default(),
            enc: None,
            level: String::new(),
        });
    }
    if !can_transcode {
        return out;
    }
    let enc = if keep_hdr { Codec::Hevc } else { Codec::H264 };
    let range = if keep_hdr { v.range } else { "SDR" };
    let fps = v.frame_rate.unwrap_or(30.0);
    let make = |bitrate: i64, (w, h): (i64, i64), label: &str| {
        // HEVC at 40 Mb/s stays within Main tier level 5.1 (what Apple devices decode), whose
        // buffer is 40 Mb.
        let (peak, bufsize) = match enc {
            Codec::H264 => (bitrate * 5 / 4, bitrate * 2),
            Codec::Hevc => (
                (bitrate * 5 / 4).min(40_000_000).max(bitrate),
                (bitrate * 2).min(40_000_000).max(bitrate),
            ),
        };
        let (level, codecs_attr) = match enc {
            Codec::H264 => {
                let l = h264_level(w, h, fps, peak, bufsize);
                (format!("{}.{}", l / 10, l % 10), format!("avc1.6400{l:02X}"))
            }
            Codec::Hevc => {
                let l = hevc_level(w, h, fps, peak, bufsize);
                (format!("{}.{}", l / 30, (l % 30) / 3), format!("hvc1.2.4.L{l}.B0"))
            }
        };
        let codec = match enc {
            Codec::H264 => "h264",
            Codec::Hevc => "hevc",
        };
        let mut id = format!("{codec}-{w}x{h}-{}k", bitrate / 1000);
        if tonemap {
            id += "-tm";
        }
        if keep_hdr {
            let _ = write!(id, "-{}", range.to_ascii_lowercase());
        }
        Variant {
            id,
            label: label.to_string(),
            bitrate,
            peak,
            bufsize,
            width: w,
            height: h,
            codec: codec.into(),
            range,
            transcode: true,
            tonemap,
            codecs_attr,
            enc: Some(enc),
            level,
        }
    };
    if !copy_ok {
        let size = if hw.max_height >= 2160 {
            fit(v, 4096, 2160)
        } else {
            fit(v, 1920, hw.max_height)
        };
        out.push(make(original_bitrate(v.bitrate, size.1), size, "Original"));
    }
    let top = out[0].bitrate;
    for (bitrate, bw, bh, label) in PRESETS {
        if bitrate as f64 >= v.bitrate as f64 * 0.9 || bitrate >= top || bh > hw.max_height {
            continue;
        }
        out.push(make(bitrate, fit(v, bw, bh), label));
    }
    out
}

/// The variant for a requested bitrate (None: the original): the best one within it, or the
/// lowest.
pub fn select(ladder: &[Variant], quality: Option<i64>) -> usize {
    match quality {
        None => 0,
        Some(q) => ladder.iter().position(|v| v.bitrate <= q).unwrap_or(ladder.len().saturating_sub(1)),
    }
}

/// The source's display size (square pixels), fitted into `bw`×`bh`, never enlarged; even.
fn fit(v: &VideoTrack, bw: i64, bh: i64) -> (i64, i64) {
    let (dw, dh) = ((v.height as f64 * v.dar).round(), v.height as f64);
    let s = (bw as f64 / dw).min(bh as f64 / dh).min(1.0);
    let even = |x: f64| ((x / 2.0).round() as i64 * 2).max(2);
    (even(dw * s), even(dh * s))
}

/// "Original" when re-encoding: 1.5× the source's bitrate (re-encoding loses some, and
/// AV1/HEVC/DivX at low bitrates need more as H.264), within a floor and a cap for the size.
fn original_bitrate(source: i64, height: i64) -> i64 {
    let (floor, cap) = match height {
        ..=576 => (2_000_000, 6_000_000),
        577..=720 => (4_000_000, 10_000_000),
        721..=1080 => (8_000_000, 20_000_000),
        _ => (16_000_000, 40_000_000),
    };
    (source * 3 / 2).clamp(floor, cap)
}

/// The lowest H.264 level (×10) for the picture size, frame rate, peak bitrate and VBV buffer
/// (High profile: 1.25× the Baseline/Main bitrate and buffer limits; NVENC refuses a level
/// they don't fit).
fn h264_level(w: i64, h: i64, fps: f64, peak: i64, bufsize: i64) -> u32 {
    // level, max macroblocks per second, max frame size in macroblocks, max bitrate kb/s and
    // max CPB kbit (Main)
    const LEVELS: [(u32, f64, i64, i64, i64); 9] = [
        (30, 40_500.0, 1_620, 10_000, 10_000),
        (31, 108_000.0, 3_600, 14_000, 14_000),
        (32, 216_000.0, 5_120, 20_000, 20_000),
        (40, 245_760.0, 8_192, 20_000, 25_000),
        (41, 245_760.0, 8_192, 50_000, 62_500),
        (42, 522_240.0, 8_704, 50_000, 62_500),
        (50, 589_824.0, 22_080, 135_000, 135_000),
        (51, 983_040.0, 36_864, 240_000, 240_000),
        (52, 2_073_600.0, 36_864, 240_000, 240_000),
    ];
    let mbs = ((w + 15) / 16) * ((h + 15) / 16);
    LEVELS
        .iter()
        .find(|(_, mbps, fs, br, cpb)| mbs <= *fs && mbs as f64 * fps <= *mbps && peak <= br * 1250 && bufsize <= cpb * 1250)
        .map_or(52, |l| l.0)
}

/// The lowest HEVC level (×30, as in `hvc1.…L<n>`) for the picture size, frame rate, peak
/// bitrate and VBV buffer (Main tier: the CPB size equals the max bitrate).
fn hevc_level(w: i64, h: i64, fps: f64, peak: i64, bufsize: i64) -> u32 {
    // level ×30, max luma picture size, max luma samples per second, max bitrate kb/s
    const LEVELS: [(u32, i64, f64, i64); 9] = [
        (90, 552_960, 16_588_800.0, 6_000),
        (93, 983_040, 33_177_600.0, 10_000),
        (120, 2_228_224, 66_846_720.0, 12_000),
        (123, 2_228_224, 133_693_440.0, 20_000),
        (150, 8_912_896, 267_386_880.0, 25_000),
        (153, 8_912_896, 534_773_760.0, 40_000),
        (156, 8_912_896, 1_069_547_520.0, 60_000),
        (180, 35_651_584, 1_069_547_520.0, 60_000),
        (183, 35_651_584, 2_139_095_040.0, 120_000),
    ];
    let ps = w * h;
    LEVELS
        .iter()
        .find(|(_, max_ps, sps, br)| ps <= *max_ps && ps as f64 * fps <= *sps && peak.max(bufsize) <= br * 1000)
        .map_or(183, |l| l.0)
}

/// NVDEC decodes it (4:2:0 only; no 10-bit H.264). MPEG-4 Part 2 is decoded on the CPU: packed
/// B-frames (DivX/Xvid in AVI) trip the hardware parser.
fn gpu_decodes(v: &VideoTrack) -> bool {
    let p = v.pix_fmt.as_str();
    let i420 = matches!(p, "yuv420p" | "yuvj420p" | "nv12");
    let i420_10 = matches!(p, "yuv420p10le" | "p010le");
    match v.codec.as_str() {
        "h264" => i420 && !super::h264_beyond_high(&v.profile),
        "hevc" | "av1" | "vp9" => i420 || i420_10,
        "mpeg2video" | "mpeg1video" | "vc1" => i420,
        _ => false,
    }
}

/// The ffmpeg command line for a run of `variant` starting at segment `n` (fragmented MP4 on
/// stdout).
pub fn ffmpeg_args(media: &Media, variant: &Variant, n: usize) -> Result<Vec<String>> {
    let mut a = video_args(media, variant, n, false)?;
    a.extend(["-map_chapters", "-1", "-map_metadata", "-1", "-f", "mp4"].map(String::from));
    a.extend(
        [
            "-movflags",
            "+frag_keyframe+empty_moov+default_base_moof+delay_moov+frag_discont",
            "pipe:1",
        ]
        .map(String::from),
    );
    Ok(a)
}

/// Everything up to the output: inputs, the picture's filters, the video map and encoder.
/// `progressive` (a whole file, for downloads): a keyframe every 4 s instead of exactly at the
/// segment boundaries.
pub fn video_args(media: &Media, variant: &Variant, n: usize, progressive: bool) -> Result<Vec<String>> {
    video_args_with(hwaccel::current(), media, variant, n, progressive)
}

/// [`video_args`] for the backend `hw`.
pub fn video_args_with(hw: &Hw, media: &Media, variant: &Variant, n: usize, progressive: bool) -> Result<Vec<String>> {
    if variant.width < hw.min_width {
        // Narrower than the hardware encoder takes (Intel Macs): libx264, cheap at that size.
        return video_args_with(
            &Hw {
                cpu_tonemap: hw.cpu_tonemap,
                ..Hw::cpu(hw.threads)
            },
            media,
            variant,
            n,
            progressive,
        );
    }
    let v = media.video.as_ref().context("no video")?;
    let enc = variant.enc.context("not a transcode")?;
    let gpu = hw_decodes(hw.backend, v);
    let mut a: Vec<String> = ["-nostdin", "-hide_banner", "-nostats", "-v", "error"].map(String::from).to_vec();
    a.extend(hwaccel::init_args(hw));
    push_decoder(&mut a, hw, v, gpu);
    push_inputs(&mut a, media, n);

    let trc = if v.range == "HLG" { "arib-std-b67" } else { "smpte2084" };
    if hw.backend == Backend::Nvenc {
        push_picture_nvenc(&mut a, v, variant, enc, gpu, trc);
    } else {
        push_picture(&mut a, hw, v, variant, gpu, trc);
    }
    push_encoder(&mut a, hw, variant, enc);
    push_keyframes(&mut a, hw, media, v, n, progressive);
    push_colours(&mut a, hw, variant, enc, trc);
    a.extend(["-fps_mode", "passthrough", "-enc_time_base", "demux"].map(String::from));
    Ok(a)
}

/// The decoder: on the device when it decodes the track (`gpu`), else on the CPU.
fn push_decoder(a: &mut Vec<String>, hw: &Hw, v: &VideoTrack, gpu: bool) {
    if gpu {
        match hw.backend {
            Backend::Nvenc => a.extend(["-hwaccel", "cuda", "-hwaccel_device", "cu", "-hwaccel_output_format", "cuda"].map(String::from)),
            // QSV: decoded with VA-API and mapped (the filters' first step).
            Backend::Qsv | Backend::Vaapi => {
                a.extend(["-hwaccel", "vaapi", "-hwaccel_device", "va", "-hwaccel_output_format", "vaapi"].map(String::from));
            }
            Backend::VideoToolbox => a.extend(
                [
                    "-hwaccel",
                    "videotoolbox",
                    "-hwaccel_device",
                    "vt",
                    "-hwaccel_output_format",
                    "videotoolbox_vld",
                ]
                .map(String::from),
            ),
            // The V4L2 decoder (the frames come back to the CPU).
            Backend::V4l2 => a.extend(["-c:v".to_string(), format!("{}_v4l2m2m", v.codec)]),
            Backend::Cpu => {}
        }
    } else {
        // Leave a core for everything else on the host.
        a.extend(["-threads".into(), hw.threads.to_string()]);
    }
}

/// The input, seeking to segment `n`.
fn push_inputs(a: &mut Vec<String>, media: &Media, n: usize) {
    a.push("-noaccurate_seek".into());
    let start = media.boundaries[n];
    if n > 0 {
        a.extend([
            "-ss".into(),
            format!("{:.3}", media.input_seek(start + media.seek_offsets.get(n).copied().unwrap_or(0.0))),
        ]);
    }
    a.extend(media.source.input_args());
    a.push("-copyts".into());
}

/// NVENC: the picture's filters on the GPU.
fn push_picture_nvenc(a: &mut Vec<String>, v: &VideoTrack, variant: &Variant, enc: Codec, gpu: bool, trc: &str) {
    let (w, h) = (variant.width, variant.height);
    // The picture: scaled (and tone-mapped) to the variant's size and pixel format.
    let out_fmt = match enc {
        Codec::Hevc => "p010le",
        Codec::H264 => "nv12",
    };
    let tonemap = format!(
        "setparams=color_primaries=bt2020:color_trc={trc}:colorspace=bt2020nc,scale_cuda=w={w}:h={h}:format=p010le,\
         tonemap_cuda=format={out_fmt}:p=bt709:t=bt709:m=bt709:r=tv:tonemap=bt2390:peak=100:desat=0"
    );
    let gpu_chain = if variant.tonemap {
        tonemap.clone()
    } else {
        format!("scale_cuda=w={w}:h={h}:format={out_fmt}")
    };
    let main = if gpu {
        gpu_chain
    } else if variant.tonemap || enc == Codec::Hevc {
        // Decoded on the CPU, mapped on the GPU.
        format!(
            "format=p010le,hwupload_cuda,{}",
            if variant.tonemap {
                tonemap
            } else {
                format!("scale_cuda=w={w}:h={h}:format=p010le")
            }
        )
    } else {
        format!("scale=w={w}:h={h}:flags=bicubic,format={out_fmt},hwupload_cuda")
    };
    a.extend(["-map".into(), format!("0:{}", v.stream), "-vf".into(), main]);
}

/// The other backends: the picture's filters.
fn push_picture(a: &mut Vec<String>, hw: &Hw, v: &VideoTrack, variant: &Variant, gpu: bool, trc: &str) {
    // QSV, VA-API, VideoToolbox, V4L2, CPU: the picture on the device (the CPU's for libx264
    // and V4L2).
    a.extend([
        "-map".into(),
        format!("0:{}", v.stream),
        "-vf".into(),
        picture_chain(hw, v, variant, gpu, trc),
    ]);
}

/// The encoder, its level, preset and rate control.
fn push_encoder(a: &mut Vec<String>, hw: &Hw, variant: &Variant, enc: Codec) {
    let (w, h) = (variant.width, variant.height);
    let big = w * h > 1920 * 1080 * 3 / 2;
    match (hw.backend, enc) {
        (Backend::Nvenc, Codec::H264) => a.extend(["-c:v", "h264_nvenc", "-profile:v", "high"].map(String::from)),
        (Backend::Nvenc, Codec::Hevc) => {
            a.extend(["-c:v", "hevc_nvenc", "-profile:v", "main10", "-tier", "main", "-tag:v", "hvc1"].map(String::from));
        }
        (Backend::Qsv, Codec::H264) => a.extend(["-c:v", "h264_qsv", "-profile:v", "high"].map(String::from)),
        (Backend::Qsv, Codec::Hevc) => a.extend(["-c:v", "hevc_qsv", "-profile:v", "main10", "-tag:v", "hvc1"].map(String::from)),
        (Backend::Vaapi, Codec::H264) => a.extend(["-c:v", "h264_vaapi", "-profile:v", "high"].map(String::from)),
        (Backend::Vaapi, Codec::Hevc) => {
            a.extend(["-c:v", "hevc_vaapi", "-profile:v", "main10", "-tier", "main", "-tag:v", "hvc1"].map(String::from));
        }
        (Backend::VideoToolbox, Codec::H264) => a.extend(["-c:v", "h264_videotoolbox", "-profile:v", "high"].map(String::from)),
        (Backend::VideoToolbox, Codec::Hevc) => {
            a.extend(["-c:v", "hevc_videotoolbox", "-profile:v", "main10", "-tag:v", "hvc1"].map(String::from));
        }
        (Backend::V4l2, _) => a.extend(["-c:v", "h264_v4l2m2m", "-profile:v", "high"].map(String::from)),
        (Backend::Cpu, _) => a.extend(["-c:v", "libx264", "-profile:v", "high"].map(String::from)),
    }
    // hevc_qsv and hevc_videotoolbox name their levels differently, the V4L2 encoder takes none:
    // they pick one themselves.
    if !(matches!(hw.backend, Backend::Qsv | Backend::VideoToolbox) && enc == Codec::Hevc) && hw.backend != Backend::V4l2 {
        a.extend(["-level:v".into(), variant.level.clone()]);
    }
    match hw.backend {
        // 4K: the fastest preset (on an entry-level card, an RTX 3050, p4 encodes 4K at 3.4×, p1 at
        // 5.7×; two jobs share it); smaller pictures: p4 with adaptive quantisation.
        Backend::Nvenc if big => a.extend(["-preset", "p1", "-tune", "hq", "-rc", "vbr"].map(String::from)),
        Backend::Nvenc => a.extend(["-preset", "p4", "-tune", "hq", "-rc", "vbr", "-spatial-aq", "1"].map(String::from)),
        Backend::Qsv => a.extend(["-preset".into(), if big { "veryfast" } else { "fast" }.into()]),
        Backend::Vaapi => a.extend(["-rc_mode", "VBR"].map(String::from)),
        // Hardware only (no silent fallback to Apple's software encoder).
        Backend::VideoToolbox => a.extend(["-allow_sw", "0"].map(String::from)),
        Backend::V4l2 => {}
        // A board's CPU (single-board computers): the faster preset.
        Backend::Cpu => a.extend([
            "-preset".into(),
            if hw.vendor.is_some() { "superfast" } else { "veryfast" }.into(),
            "-threads".into(),
            hw.threads.to_string(),
        ]),
    }
    a.extend([
        "-b:v".into(),
        variant.bitrate.to_string(),
        "-maxrate".into(),
        variant.peak.to_string(),
        "-bufsize".into(),
        variant.bufsize.to_string(),
    ]);
}

/// Keyframes: exactly at the segment boundaries from segment `n` on, or every 4 s (`progressive`).
fn push_keyframes(a: &mut Vec<String>, hw: &Hw, media: &Media, v: &VideoTrack, n: usize, progressive: bool) {
    // An IDR exactly at every boundary and nowhere else matters: no B-frames (decode time =
    // presentation time), no scene-cut or periodic keyframes.
    if progressive {
        let gop = (v.frame_rate.unwrap_or(25.0) * 4.0).round().max(1.0) as i64;
        a.extend(["-bf".into(), "0".into(), "-g".into(), gop.to_string()]);
    } else {
        match hw.backend {
            Backend::Nvenc => a.extend(["-bf", "0", "-g", "100000", "-no-scenecut", "1", "-forced-idr", "1"].map(String::from)),
            Backend::Qsv => a.extend(
                [
                    "-bf",
                    "0",
                    "-g",
                    "100000",
                    "-forced_idr",
                    "1",
                    "-adaptive_i",
                    "0",
                    "-adaptive_b",
                    "0",
                ]
                .map(String::from),
            ),
            // A frame forced to be a keyframe is an IDR (VA-API never cuts at scenes).
            // A frame forced to be a keyframe is an IDR (VideoToolbox, V4L2 too).
            Backend::Vaapi | Backend::VideoToolbox | Backend::V4l2 => a.extend(["-bf", "0", "-g", "100000"].map(String::from)),
            Backend::Cpu => a.extend(["-bf", "0", "-g", "100000", "-sc_threshold", "0", "-forced-idr", "1"].map(String::from)),
        }
        let times: Vec<String> = media.boundaries[n..]
            .iter()
            .map(|b| format!("{:.3}", (b - FORCE_EARLY).max(0.0)))
            .collect();
        a.extend(["-force_key_frames".into(), times.join(",")]);
    }
}

/// The colour tags of the output.
fn push_colours(a: &mut Vec<String>, hw: &Hw, variant: &Variant, enc: Codec, trc: &str) {
    // Colour tags. The range stays the source's (asking for another makes ffmpeg insert a
    // conversion it can't do on GPU frames), except after tone-mapping, which outputs limited.
    if enc == Codec::Hevc {
        a.extend(["-color_primaries", "bt2020", "-color_trc", trc, "-colorspace", "bt2020nc"].map(String::from));
    } else if variant.tonemap && hw.backend == Backend::VideoToolbox && hw.gpu_tonemap {
        // scale_vt tags its frames BT.709 itself; asking for a colour matrix too would make ffmpeg
        // insert a CPU conversion it can't do on VideoToolbox frames.
        a.extend(["-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv"].map(String::from));
    } else if variant.tonemap {
        a.extend(
            [
                "-color_primaries",
                "bt709",
                "-color_trc",
                "bt709",
                "-colorspace",
                "bt709",
                "-color_range",
                "tv",
            ]
            .map(String::from),
        );
    }
}

/// The decoder for this backend handles the track (the rest is decoded on the CPU and uploaded).
fn hw_decodes(backend: Backend, v: &VideoTrack) -> bool {
    match backend {
        Backend::Nvenc => gpu_decodes(v),
        // What every VA-API driver of the last decade decodes; the rest goes through the CPU
        // (newer codecs depend on the GPU's generation).
        Backend::Qsv | Backend::Vaapi => {
            let p = v.pix_fmt.as_str();
            let i420 = matches!(p, "yuv420p" | "yuvj420p" | "nv12");
            let i420_10 = matches!(p, "yuv420p10le" | "p010le");
            match v.codec.as_str() {
                "h264" => i420 && !super::h264_beyond_high(&v.profile),
                "hevc" => i420 || i420_10,
                "mpeg2video" => i420,
                _ => false,
            }
        }
        // What VideoToolbox decodes on every Mac that runs a current macOS (Intel from 7th
        // generation: HEVC Main 10 too). The rest goes through the CPU and is uploaded.
        Backend::VideoToolbox => {
            let p = v.pix_fmt.as_str();
            let i420 = matches!(p, "yuv420p" | "yuvj420p" | "nv12");
            let i420_10 = matches!(p, "yuv420p10le" | "p010le");
            match v.codec.as_str() {
                "h264" => i420 && !super::h264_beyond_high(&v.profile),
                "hevc" => i420 || i420_10,
                _ => false,
            }
        }
        // The Pi 4's stateful decoder: H.264 up to 1080p (HEVC there is a stateless decoder
        // upstream ffmpeg doesn't drive).
        Backend::V4l2 => {
            v.codec == "h264"
                && !super::h264_beyond_high(&v.profile)
                && matches!(v.pix_fmt.as_str(), "yuv420p" | "yuvj420p")
                && v.height <= 1088
                && v.width <= 1920
        }
        Backend::Cpu => false,
    }
}

/// HDR (PQ/HLG) to SDR BT.709 on the CPU (ffmpeg's `tonemap`, Hable), ending in yuv420p.
fn cpu_tonemap(trc: &str) -> String {
    format!(
        "setparams=color_primaries=bt2020:color_trc={trc}:colorspace=bt2020nc,zscale=t=linear:npl=100,format=gbrpf32le,\
         zscale=p=bt709,tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p"
    )
}

/// The picture's filters for QSV, VA-API and the CPU: scaled (and tone-mapped) to the variant.
/// The frames end on the device for the hardware encoders, in CPU memory for libx264 or when
/// `on_cpu` (to lay subtitles over them).
fn picture_chain(hw: &Hw, v: &VideoTrack, variant: &Variant, gpu: bool, trc: &str) -> String {
    let (w, h) = (variant.width, variant.height);
    let hevc = variant.enc == Some(Codec::Hevc);
    let ten_bit = v.pix_fmt.contains("10");
    let up = hwaccel::upload(hw);
    let setp = format!("setparams=color_primaries=bt2020:color_trc={trc}:colorspace=bt2020nc");
    if matches!(hw.backend, Backend::Cpu | Backend::V4l2) {
        // V4L2 decodes into CPU memory, and its encoder takes it from there.
        return if variant.tonemap {
            format!("scale=w={w}:h={h}:flags=bicubic,{}", cpu_tonemap(trc))
        } else {
            format!("scale=w={w}:h={h}:flags=bicubic,format=yuv420p")
        };
    }
    if hw.backend == Backend::VideoToolbox {
        return vt_chain(hw, v, variant, gpu, trc);
    }
    let qsv = hw.backend == Backend::Qsv;
    // Frames on the device: decoded there (QSV maps VA-API frames), else uploaded.
    let src = if gpu {
        if qsv {
            "hwmap=derive_device=qsv,format=qsv,".to_string()
        } else {
            String::new()
        }
    } else {
        format!(
            "format={},{up},",
            if ten_bit || hevc || variant.tonemap { "p010le" } else { "nv12" }
        )
    };
    let scale = |fmt: &str| {
        if qsv {
            format!("scale_qsv=w={w}:h={h}:format={fmt}")
        } else {
            format!("scale_vaapi=w={w}:h={h}:format={fmt}")
        }
    };
    let download = |fmt: &str| format!("hwdownload,format={fmt}");
    let chain = if variant.tonemap && hw.gpu_tonemap {
        let tm = if qsv {
            format!("vpp_qsv=tonemap=1:format=nv12:w={w}:h={h}")
        } else {
            format!("{},tonemap_vaapi=format=nv12:p=bt709:t=bt709:m=bt709", scale("p010"))
        };
        format!("{setp},{src}{tm}")
    } else if variant.tonemap {
        // Tone-mapped on the CPU: scaled on the device, down, mapped, up again.
        let c = if gpu {
            format!("{src}{},{},{}", scale("p010"), download("p010le"), cpu_tonemap(trc))
        } else {
            format!("scale=w={w}:h={h}:flags=bicubic,{}", cpu_tonemap(trc))
        };
        format!("{c},format=nv12,{up}")
    } else {
        let fmt = if hevc { "p010" } else { "nv12" };
        format!("{src}{}", scale(fmt))
    };
    chain
}

/// The picture's filters for VideoToolbox: on VideoToolbox frames (decoded there, else uploaded),
/// scaled and tone-mapped by `scale_vt`; the encoder takes them as they are, or from CPU memory
/// when tone-mapping falls back to the CPU.
fn vt_chain(hw: &Hw, v: &VideoTrack, variant: &Variant, gpu: bool, trc: &str) -> String {
    let (w, h) = (variant.width, variant.height);
    let ten_bit = v.pix_fmt.contains("10");
    let hevc = variant.enc == Some(Codec::Hevc);
    // The frames' format on the device: the decoder's, or what is uploaded.
    let dev_fmt = if gpu {
        if ten_bit {
            "p010le"
        } else {
            "nv12"
        }
    } else if ten_bit || hevc || variant.tonemap {
        "p010le"
    } else {
        "nv12"
    };
    if variant.tonemap && !hw.gpu_tonemap {
        // Tone-mapped on the CPU (zscale): down from the device first.
        let down = if gpu {
            format!("hwdownload,format={dev_fmt},")
        } else {
            String::new()
        };
        return format!("{down}scale=w={w}:h={h}:flags=bicubic,{}", cpu_tonemap(trc));
    }
    let src = if gpu {
        String::new()
    } else {
        format!("format={dev_fmt},{},", hwaccel::upload(hw))
    };
    if variant.tonemap {
        // Tagged as what it is first: the pixel transfer maps from the tags.
        let setp = if gpu {
            String::new()
        } else {
            format!("setparams=color_primaries=bt2020:color_trc={trc}:colorspace=bt2020nc,")
        };
        format!("{setp}{src}scale_vt=w={w}:h={h}:{}", hwaccel::VT_TO_SDR)
    } else {
        format!("{src}scale_vt=w={w}:h={h}")
    }
}

#[derive(Clone, Debug)]
struct Progress {
    /// The segment being made.
    next: usize,
    /// The init section is in the cache.
    init: bool,
    /// Set when the run has ended.
    done: Option<Result<(), String>>,
}

/// A receiver for a run's or job's stop signal (`true` once stopped, never reset). Unlike a
/// plain `subscribe()`, which marks the current value as seen, a stop already sent makes
/// `changed()` return at once: a stop sent between the run's creation and this call isn't lost.
/// Stops are sent with `send_replace`: `send` doesn't store the value while nobody listens.
pub(crate) fn stop_signal(stop: &watch::Sender<bool>) -> watch::Receiver<bool> {
    let mut rx = stop.subscribe();
    if *rx.borrow() {
        rx.mark_changed();
    }
    rx
}

struct Run {
    id: u64,
    start: usize,
    /// The session that started it.
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

    fn idle_for(&self) -> Duration {
        self.last_request.lock().unwrap().elapsed()
    }
}

pub struct Transcoder {
    ffmpeg: String,
    cache: Arc<Cache>,
    runs: Mutex<HashMap<String, Vec<Arc<Run>>>>,
    slots: Arc<Semaphore>,
    limit: usize,
    next_id: AtomicU64,
    /// Held while a run is being started: a second request for the same place (the player's
    /// init request racing the playback prewarm) then finds that run instead of starting one.
    starting: tokio::sync::Mutex<()>,
    /// Admitted sessions whose run hasn't started yet (track → when): they count as in use, so
    /// two sessions opened at once can't both take the last slot.
    reserved: Mutex<HashMap<String, (String, Instant)>>,
    /// Sessions closed (`DELETE`): a prewarm still on its way must not start a run for them.
    closed: Mutex<HashMap<String, Instant>>,
}

/// For logs: what is running.
#[derive(Serialize)]
pub struct RunInfo {
    pub track: String,
    pub owner: String,
    pub start: usize,
    pub next: usize,
    pub idle_secs: u64,
}

impl Transcoder {
    pub fn new(ffmpeg: String, cache: Arc<Cache>, jobs: usize) -> Self {
        Transcoder {
            ffmpeg,
            cache,
            runs: Mutex::new(HashMap::new()),
            slots: Arc::new(Semaphore::new(jobs.max(1))),
            limit: jobs.max(1),
            next_id: AtomicU64::new(1),
            starting: tokio::sync::Mutex::new(()),
            reserved: Mutex::new(HashMap::new()),
            closed: Mutex::new(HashMap::new()),
        }
    }

    /// Run the GPU filters and encoders once on a tiny picture, so the first real run doesn't
    /// pay for loading them (the CUDA kernels are compiled on first use in a new container:
    /// seconds).
    pub async fn warm_up(&self) {
        let t0 = Instant::now();
        let graph = "[0]format=p010le,hwupload_cuda,setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc,\
                     scale_cuda=w=320:h=180:format=p010le,tonemap_cuda=format=yuv420p:p=bt709:t=bt709:m=bt709:r=tv[m];\
                     [1]format=yuva420p,hwupload_cuda[s];[m][s]overlay_cuda[out]";
        for (map, enc) in [("[out]", "h264_nvenc"), ("[m2]", "hevc_nvenc")] {
            let mut cmd = tokio::process::Command::new(&self.ffmpeg);
            cmd.args(["-nostdin", "-v", "error", "-init_hw_device", "cuda=cu", "-filter_hw_device", "cu"])
                .args([
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc2=size=320x180:rate=24",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=black@0.5:size=320x180:rate=24",
                ]);
            if enc == "hevc_nvenc" {
                cmd.args([
                    "-filter_complex",
                    "[0]format=p010le,hwupload_cuda,scale_cuda=w=320:h=180:format=p010le[m2]",
                ]);
            } else {
                cmd.args(["-filter_complex", graph]);
            }
            cmd.args(["-map", map, "-frames:v", "5", "-c:v", enc, "-f", "null", "-"]);
            match tokio::time::timeout(Duration::from_secs(60), cmd.kill_on_drop(true).output()).await {
                Ok(Ok(o)) if o.status.success() => {}
                Ok(Ok(o)) => tracing::warn!(enc, "GPU warm-up failed: {}", tail(&String::from_utf8_lossy(&o.stderr))),
                _ => tracing::warn!(enc, "GPU warm-up didn't finish"),
            }
        }
        tracing::info!(ms = t0.elapsed().as_millis() as u64, "GPU warmed up");
    }

    fn track_key(media: &Media, variant: &Variant) -> String {
        format!("{}/{}/tx{TRANSCODE_VERSION}/{}", media.item_id, media.version, variant.id)
    }

    fn segment_key(track: &str, n: usize) -> String {
        format!("{track}/{n}.m4s")
    }

    /// Admit a new session playing `variant`: false when all slots are taken by runs other
    /// sessions are using now (or just admitted ones), and none of them makes this variant (a
    /// new transcode would have to stop someone else's). When admitted, a slot is reserved for
    /// it until its run starts.
    pub fn admit(&self, media: &Media, variant: &Variant, owner: &str) -> bool {
        let track = Self::track_key(media, variant);
        let runs = self.runs.lock().unwrap();
        let mut reserved = self.reserved.lock().unwrap();
        let live = |t: &str| runs.get(t).is_some_and(|l| l.iter().any(|r| !r.ended()));
        reserved.retain(|t, (_, at)| at.elapsed() < Duration::from_secs(10) && !live(t));
        if live(&track) || reserved.contains_key(&track) {
            return true;
        }
        let active: Vec<String> = runs
            .values()
            .flatten()
            .filter(|r| !r.ended() && r.idle_for() < ACTIVE && !is_download(&r.owner))
            .map(|r| format!("{}@{}s", r.id, r.idle_for().as_secs()))
            .collect();
        if active.len() + reserved.len() >= self.limit {
            tracing::info!(item = %media.item_id, variant = variant.id, ?active, reserved = reserved.len(), "transcode slots busy");
            return false;
        }
        reserved.insert(track, (owner.to_string(), Instant::now()));
        true
    }

    pub fn running(&self) -> Vec<RunInfo> {
        let runs = self.runs.lock().unwrap();
        runs.iter()
            .flat_map(|(t, l)| {
                l.iter().filter(|r| !r.ended()).map(|r| RunInfo {
                    track: t.clone(),
                    owner: r.owner.chars().take(6).collect(),
                    start: r.start,
                    next: r.next(),
                    idle_secs: r.idle_for().as_secs(),
                })
            })
            .collect()
    }

    /// Stop every run a session started, and start none for it from now on.
    pub fn stop_owner(&self, owner: &str) -> usize {
        {
            let mut closed = self.closed.lock().unwrap();
            closed.retain(|_, at| at.elapsed() < Duration::from_secs(600));
            closed.insert(owner.to_string(), Instant::now());
        }
        self.reserved.lock().unwrap().retain(|_, (o, _)| o != owner);
        let runs = self.runs.lock().unwrap();
        let mut n = 0;
        for r in runs.values().flatten().filter(|r| r.owner == owner && !r.ended()) {
            r.cancel.send_replace(true);
            n += 1;
        }
        n
    }

    /// The init section; when it isn't cached yet, from a run at segment `n` (where the
    /// session starts).
    pub async fn init(self: &Arc<Self>, media: &Arc<Media>, variant: &Variant, n: usize, owner: &str) -> Result<Bytes> {
        let track = Self::track_key(media, variant);
        let key = format!("{track}/init.mp4");
        if let Some(b) = self.cache.get(&key).await {
            return Ok(b);
        }
        let existing = {
            let runs = self.runs.lock().unwrap();
            runs.get(&track).and_then(|l| l.iter().find(|r| !r.ended()).cloned())
        };
        let run = match existing {
            Some(r) => r,
            None => self.run_for(media, variant, &track, n, owner).await?,
        };
        let mut rx = run.progress.subscribe();
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let p = rx.borrow_and_update().clone();
            if p.init || p.done.is_some() {
                if let Some(b) = self.cache.get(&key).await {
                    return Ok(b);
                }
                if let Some(Err(e)) = p.done {
                    bail!("transcode run: {e}");
                }
                bail!("transcode run ended without an init section");
            }
            match tokio::time::timeout_at(deadline, rx.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => bail!("transcode run gone"),
                Err(_) => bail!("timed out waiting for the init section"),
            }
        }
    }

    /// Segment `n` of `variant`.
    pub async fn segment(self: &Arc<Self>, media: &Arc<Media>, variant: &Variant, n: usize, owner: &str) -> Result<Bytes> {
        self.segment_tries(media, variant, n, owner, 3).await
    }

    /// Segment `n`, from up to `tries` runs: when the run it waits on ends (or is stopped)
    /// without it, another is started.
    async fn segment_tries(self: &Arc<Self>, media: &Arc<Media>, variant: &Variant, n: usize, owner: &str, tries: usize) -> Result<Bytes> {
        let track = Self::track_key(media, variant);
        let key = Self::segment_key(&track, n);
        if let Some(b) = self.cache.get(&key).await {
            // Served from the cache: the slot reserved for the session isn't needed (yet).
            self.reserved.lock().unwrap().retain(|_, (o, _)| o != owner);
            return Ok(b);
        }
        let deadline = tokio::time::Instant::now() + WAIT;
        for _ in 0..tries {
            let run = self.run_for(media, variant, &track, n, owner).await?;
            let mut rx = run.progress.subscribe();
            loop {
                let p = rx.borrow_and_update().clone();
                if p.next > n || p.done.is_some() {
                    if let Some(b) = self.cache.get(&key).await {
                        return Ok(b);
                    }
                    if let Some(Err(e)) = p.done {
                        bail!("transcode run: {e}");
                    }
                    break; // ended (or was stopped) without it: try another run
                }
                match tokio::time::timeout_at(deadline, rx.changed()).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => break,
                    Err(_) => bail!("timed out waiting for transcoded segment {n}"),
                }
            }
        }
        bail!("transcoded segment {n} not produced")
    }

    /// Make sure segment `n` is being made, without waiting for it. Once: when its run is
    /// stopped (the player asked for another place first, or the slot went to another run), it
    /// doesn't start another, which would stop the run the player is waiting on.
    pub fn prefetch(self: &Arc<Self>, media: &Arc<Media>, variant: &Variant, n: usize, owner: &str) {
        let (this, media, variant, owner) = (self.clone(), media.clone(), variant.clone(), owner.to_string());
        tokio::spawn(async move {
            if let Err(e) = this.segment_tries(&media, &variant, n, &owner, 1).await {
                tracing::debug!(item = %media.item_id, variant = variant.id, n, "prefetch: {e:#}");
            }
        });
    }

    /// A run that will deliver segment `n` soon, or a new one starting there (which replaces
    /// the session's other runs of this variant: it has seeked away from them).
    async fn run_for(self: &Arc<Self>, media: &Arc<Media>, variant: &Variant, track: &str, n: usize, owner: &str) -> Result<Arc<Run>> {
        let reachable = |run: &Run| {
            let next = run.next();
            run.start <= n && n >= next && !run.ended() && media.boundaries[n] - media.boundaries[next] <= REACH_SECS
        };
        let find = |this: &Self| {
            let mut runs = this.runs.lock().unwrap();
            let list = runs.entry(track.to_string()).or_default();
            list.retain(|r| !r.ended());
            let r = list.iter().find(|r| reachable(r)).cloned();
            if let Some(r) = &r {
                r.want(n);
            }
            r
        };
        if let Some(r) = find(self) {
            return Ok(r);
        }
        let _starting = self.starting.lock().await;
        if let Some(r) = find(self) {
            return Ok(r);
        }
        if self.closed.lock().unwrap().contains_key(owner) {
            bail!("session closed");
        }
        // The session's runs of this variant elsewhere are of no use now (a download's still are).
        if !is_download(owner) {
            let runs = self.runs.lock().unwrap();
            for r in runs.get(track).into_iter().flatten().filter(|r| r.owner == owner && !r.ended()) {
                tracing::info!(item = %media.item_id, run = r.id, from = r.start, at = r.next(), to = n, "seek: stopping the session's run");
                r.cancel.send_replace(true);
            }
        }
        let waited = Instant::now();
        let permit = self.slot(owner).await?;
        if waited.elapsed() > Duration::from_millis(50) {
            tracing::info!(item = %media.item_id, ms = waited.elapsed().as_millis() as u64, "waited for a transcode slot");
        }
        // Another request may have started one meanwhile.
        if let Some(r) = find(self) {
            return Ok(r);
        }
        let run = Arc::new(Run {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            start: n,
            owner: owner.to_string(),
            progress: watch::channel(Progress {
                next: n,
                init: false,
                done: None,
            })
            .0,
            demand: watch::channel(n).0,
            last_request: Mutex::new(Instant::now()),
            cancel: watch::channel(false).0,
        });
        self.runs.lock().unwrap().entry(track.to_string()).or_default().push(run.clone());
        self.reserved.lock().unwrap().remove(track);
        let (this, media, variant, track, r) = (self.clone(), media.clone(), variant.clone(), track.to_string(), run.clone());
        tokio::spawn(async move {
            let t0 = Instant::now();
            let result = this.run(&r, &media, &variant, &track).await;
            match &result {
                Ok(()) => tracing::info!(item = %media.item_id, variant = variant.id, run = r.id, from = r.start, to = r.next(),
                    secs = t0.elapsed().as_secs_f32(), "transcode run ended"),
                Err(e) => {
                    tracing::warn!(item = %media.item_id, variant = variant.id, run = r.id, from = r.start, "transcode run failed: {e:#}");
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
        Ok(run)
    }

    /// A run slot; when all are taken, a run is stopped for it: a download's first (the one
    /// asked for least recently), then for a stream the stream run asked for least recently. A
    /// download never stops a stream's run: it waits for a slot instead.
    async fn slot(&self, owner: &str) -> Result<OwnedSemaphorePermit> {
        if let Ok(p) = self.slots.clone().try_acquire_owned() {
            return Ok(p);
        }
        let victim = {
            let runs = self.runs.lock().unwrap();
            let live = || runs.values().flatten().filter(|r| !r.ended());
            let lru = |dl: bool| {
                live()
                    .filter(|r| is_download(&r.owner) == dl)
                    .min_by_key(|r| *r.last_request.lock().unwrap())
                    .cloned()
            };
            lru(true).or_else(|| if is_download(owner) { None } else { lru(false) })
        };
        if let Some(v) = victim {
            tracing::info!(
                run = v.id,
                idle_secs = v.idle_for().as_secs(),
                "transcode slots full: stopping the least recently used run"
            );
            v.cancel.send_replace(true);
        }
        tokio::time::timeout(Duration::from_secs(10), self.slots.clone().acquire_owned())
            .await
            .context("no transcode slot free")?
            .map_err(Into::into)
    }

    async fn run(&self, run: &Run, media: &Media, variant: &Variant, track: &str) -> Result<()> {
        // Listen for a stop first: one sent since the run was made (a seek, or the slot taken
        // for another run) must still end it.
        let mut cancel = stop_signal(&run.cancel);
        if *cancel.borrow() {
            tracing::info!(item = %media.item_id, variant = variant.id, run = run.id, "transcode run stopped before it started");
            return Ok(());
        }
        let args = ffmpeg_args(media, variant, run.start)?;
        let t0 = Instant::now();
        if tracing::enabled!(tracing::Level::DEBUG) {
            // Without the source's auth header (a token when read over HTTP).
            let shown: Vec<&str> = args
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    if i > 0 && args[i - 1] == "-headers" {
                        "<redacted>"
                    } else {
                        a.as_str()
                    }
                })
                .collect();
            tracing::debug!(args = %crate::admin::logbuf::scrub(&shown.join(" ")), "ffmpeg");
        }
        tracing::info!(item = %media.item_id, variant = variant.id, run = run.id, from = run.start, at = media.boundaries[run.start], owner = %run.owner.chars().take(6).collect::<String>(), "transcode run");
        let mut child = tokio::process::Command::new("nice")
            .args(["-n", "5", self.ffmpeg.as_str()])
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("ffmpeg")?;
        let mut stderr = child.stderr.take().context("no stderr")?;
        let err_log = tokio::spawn(async move {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s).await;
            s
        });
        let mut stdout = child.stdout.take().context("no stdout")?;

        let segments = media.boundaries.len();
        let mut demand = run.demand.subscribe();
        let init_key = format!("{track}/init.mp4");
        let mut init: Vec<u8> = Vec::new();
        let mut timescale = 0f64;
        let mut moof: Option<Vec<u8>> = None;
        let mut seg = run.start;
        let mut body: Vec<u8> = Vec::new();
        let mut buf: Vec<u8> = Vec::with_capacity(1 << 20);
        let mut chunk = vec![0u8; 1 << 16];
        let mut dropped = 0usize;

        'read: loop {
            // Back-pressure: stay within LEAD of what has been asked for.
            loop {
                let d = (*demand.borrow_and_update()).min(segments - 1);
                if media.boundaries[seg.min(segments - 1)] - media.boundaries[d] <= LEAD_SECS {
                    break;
                }
                let stop = tokio::select! {
                    r = demand.changed() => r.is_err(),
                    _ = cancel.changed() => true,
                    () = tokio::time::sleep(IDLE) => run.idle_for() >= IDLE,
                };
                if stop {
                    break 'read;
                }
            }
            let got = tokio::select! {
                r = stdout.read(&mut chunk) => Some(r),
                _ = cancel.changed() => None,
                // Encoding ahead with nobody asking (the player stalled or went away).
                () = tokio::time::sleep(IDLE) => if run.idle_for() >= IDLE { None } else { continue },
            };
            let Some(got) = got else { break 'read };
            let got = got.context("read ffmpeg output")?;
            if got == 0 {
                let status = child.wait().await?;
                if !status.success() {
                    bail!("ffmpeg: {}", tail(&err_log.await.unwrap_or_default()));
                }
                if !body.is_empty() {
                    if !media.is_last(seg) {
                        tracing::warn!(item = %media.item_id, variant = variant.id, seg, segments, "transcode ended before the last segment");
                    }
                    self.cache.put(&Self::segment_key(track, seg), &body).await;
                    run.progress.send_modify(|p| p.next = seg + 1);
                } else if seg == run.start {
                    bail!("no video at segment {seg}: {}", tail(&err_log.await.unwrap_or_default()));
                }
                return Ok(());
            }
            buf.extend_from_slice(&chunk[..got]);
            let mut used = 0;
            while let Some((kind, len)) = next_box(&buf[used..])? {
                let bytes = &buf[used..used + len];
                used += len;
                match &kind {
                    b"ftyp" | b"moov" => init.extend_from_slice(bytes),
                    b"moof" => moof = Some(bytes.to_vec()),
                    b"mdat" => {
                        let Some(mut frag) = moof.take() else { bail!("mdat without moof") };
                        if timescale == 0.0 {
                            let clean = mp4::strip_edts(&init)?;
                            timescale = mp4::parse(&clean)?.timescale as f64;
                            if self.cache.get(&init_key).await.is_none() {
                                self.cache.put(&init_key, &clean).await;
                            }
                            run.progress.send_modify(|p| p.init = true);
                        }
                        frag.extend_from_slice(bytes);
                        let frag = mp4::Fragment::from_bytes(frag)?;
                        let t = frag.decode_time() as f64 / timescale;
                        // Fragments start at keyframes: the forced ones at the boundaries, and
                        // the first frame (before the run's first boundary when the seek landed
                        // early: dropped).
                        if body.is_empty() && seg == run.start && t < media.boundaries[seg] - TOL {
                            dropped += 1;
                            continue;
                        }
                        while seg + 1 < segments && t >= media.boundaries[seg + 1] - TOL {
                            if body.is_empty() {
                                bail!(
                                    "no keyframe at segment {seg} ({:.3}): next fragment at {t:.3}",
                                    media.boundaries[seg]
                                );
                            }
                            self.cache.put(&Self::segment_key(track, seg), &body).await;
                            if seg == run.start {
                                tracing::info!(item = %media.item_id, variant = variant.id, run = run.id, seg, dropped,
                                    ms = t0.elapsed().as_millis() as u64, "first transcoded segment");
                            }
                            body.clear();
                            seg += 1;
                            run.progress.send_modify(|p| p.next = seg);
                            if self.covered(track, run.id, seg).await {
                                return Ok(());
                            }
                        }
                        // Segment 0 begins with the file's first picture, at its start time.
                        let starts_at = media.boundaries[seg].max(media.origin);
                        if body.is_empty() && (t - starts_at).abs() > 0.25 {
                            tracing::warn!(item = %media.item_id, variant = variant.id, seg, at = t, boundary = starts_at, "segment doesn't start at its boundary");
                        }
                        body.extend_from_slice(&frag.bytes);
                    }
                    _ => {}
                }
            }
            buf.drain(..used);
        }
        // Stopped (cancelled, idle, or met covered ground): ffmpeg is killed on drop.
        Ok(())
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

fn tail(log: &str) -> String {
    let lines: Vec<&str> = log.trim().lines().collect();
    crate::admin::logbuf::scrub(&lines[lines.len().saturating_sub(5)..].join(" | ")).into_owned()
}

/// The next whole top-level box at the start of `buf`: (type, length), or None when more data
/// is needed.
fn next_box(buf: &[u8]) -> Result<Option<([u8; 4], usize)>> {
    if buf.len() < 8 {
        return Ok(None);
    }
    let size32 = u32::from_be_bytes(buf[0..4].try_into().unwrap()) as usize;
    let kind: [u8; 4] = buf[4..8].try_into().unwrap();
    let size = match size32 {
        1 => {
            if buf.len() < 16 {
                return Ok(None);
            }
            u64::from_be_bytes(buf[8..16].try_into().unwrap()) as usize
        }
        0 => bail!("box of unknown size in a stream"),
        s => s,
    };
    if size < 8 {
        bail!("bad box size {size}");
    }
    Ok((buf.len() >= size).then_some((kind, size)))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn video(codec: &str, w: i64, h: i64, bitrate: i64, range: &'static str) -> VideoTrack {
        VideoTrack {
            index: 0,
            stream: 0,
            codec: codec.into(),
            codecs_attr: matches!(codec, "h264" | "hevc").then(|| "avc1.640028".to_string()),
            width: w,
            height: h,
            frame_rate: Some(24000.0 / 1001.0),
            range,
            bitrate,
            dar: w as f64 / h as f64,
            pix_fmt: if range == "SDR" { "yuv420p".into() } else { "yuv420p10le".into() },
            profile: String::new(),
            dovi_only: false,
            dovi: None,
            reorder: 0,
            copy_reason: (!matches!(codec, "h264" | "hevc")).then(|| "no".to_string()),
            short_nal_lengths: false,
        }
    }

    fn ids(l: &[Variant]) -> Vec<&str> {
        l.iter().map(|v| v.id.as_str()).collect()
    }

    #[test]
    fn ladder_for_a_copyable_source() {
        let v = video("h264", 1920, 1080, 12_000_000, "SDR");
        let l = ladder(&v, true, true);
        assert_eq!(
            ids(&l),
            ["copy", "h264-1920x1080-10000k", "h264-1280x720-4000k", "h264-854x480-1500k"]
        );
        assert_eq!(l[1].codecs_attr, "avc1.640028"); // level 4.0
        assert_eq!(l[2].codecs_attr, "avc1.64001F"); // 3.1
        assert_eq!(l[3].codecs_attr, "avc1.64001E"); // 3.0
        assert_eq!(select(&l, None), 0);
        assert_eq!(select(&l, Some(20_000_000)), 0);
        assert_eq!(select(&l, Some(4_000_000)), 2);
        assert_eq!(select(&l, Some(1_000_000)), 3);
        // Transcoding off: the copy only.
        assert_eq!(ids(&ladder(&v, true, false)), ["copy"]);
    }

    #[test]
    fn ladder_for_sources_that_must_be_transcoded() {
        // A small DivX: the original is re-encoded at a floor bitrate; no presets below it.
        let v = video("mpeg4", 656, 368, 1_200_000, "SDR");
        let l = ladder(&v, true, true);
        assert_eq!(ids(&l), ["h264-656x368-2000k"]);
        assert_eq!(l[0].label, "Original");
        assert_eq!(select(&l, Some(1_500_000)), 0);
        assert!(ladder(&v, true, false).is_empty());
        // Anamorphic DVD: square pixels.
        let mut dvd = video("mpeg2video", 720, 576, 6_000_000, "SDR");
        dvd.dar = 16.0 / 9.0;
        assert_eq!(
            ids(&ladder(&dvd, true, true)),
            ["h264-1024x576-6000k", "h264-1024x576-4000k", "h264-854x480-1500k"]
        );
    }

    #[test]
    fn ladder_for_hdr() {
        let v = video("hevc", 3840, 2160, 60_000_000, "PQ");
        // An HDR display: the copy, then HEVC Main 10 keeping HDR.
        let l = ladder(&v, true, true);
        assert_eq!(l[0].id, "copy");
        assert_eq!(l[1].id, "hevc-3840x2160-40000k-pq");
        assert_eq!(l[1].codecs_attr, "hvc1.2.4.L153.B0");
        assert_eq!(l[1].range, "PQ");
        // No HDR: everything tone-mapped to SDR H.264, the original too.
        let l = ladder(&v, false, true);
        assert_eq!(
            ids(&l),
            [
                "h264-3840x2160-40000k-tm",
                "h264-1920x1080-20000k-tm",
                "h264-1920x1080-10000k-tm",
                "h264-1280x720-4000k-tm",
                "h264-854x480-1500k-tm"
            ]
        );
        assert!(l.iter().all(|v| v.tonemap && v.range == "SDR"));
        assert_eq!(l[0].codecs_attr, "avc1.640033"); // 5.1
                                                     // Transcoding off: the copy as before (the app decides).
        assert_eq!(ids(&ladder(&v, false, false)), ["copy"]);
    }

    #[test]
    fn levels() {
        assert_eq!(h264_level(1920, 1080, 23.976, 12_500_000, 20_000_000), 40);
        assert_eq!(h264_level(1920, 1080, 23.976, 25_000_000, 31_250_000), 40);
        assert_eq!(h264_level(1920, 1080, 23.976, 25_000_001, 20_000_000), 41);
        // The 10-bit sample's original: 19.6 Mb/s with a 39 Mb buffer is beyond 4.0's CPB.
        assert_eq!(h264_level(1920, 1080, 23.976, 24_519_982, 39_231_972), 41);
        assert_eq!(h264_level(1920, 1080, 59.94, 12_500_000, 20_000_000), 42);
        assert_eq!(h264_level(1280, 720, 59.94, 5_000_000, 8_000_000), 32);
        assert_eq!(hevc_level(1920, 1080, 23.976, 12_500_000, 20_000_000), 123);
        assert_eq!(hevc_level(1280, 720, 23.976, 5_000_000, 8_000_000), 93);
        assert_eq!(hevc_level(3840, 2160, 23.976, 40_000_000, 40_000_000), 153);
    }

    pub(crate) fn media(v: VideoTrack) -> Media {
        Media {
            file_id: None,
            item_id: "i".into(),
            media_source_id: "i".into(),
            version: "v".into(),
            file_key: "k".into(),
            size: None,
            item: Arc::default(),
            source: crate::storage::source::Source::Local("/m/a.mkv".into()),
            source_kind: "local",
            probe_cache: "off",
            container: "matroska,webm".into(),
            duration: 30.0,
            supported: true,
            reasons: vec![],
            method: "copy",
            pending: false,
            look_again: false,
            damaged: false,
            video: Some(v),
            audio: vec![],
            subtitles: vec![],
            boundaries: vec![0.0, 11.783, 26.506],
            seek_offsets: vec![0.136, 0.136, 0.05],
            start_time: 0.0,
            origin: 0.0,
        }
    }

    #[test]
    fn a_run_in_a_file_with_a_start_time_asks_on_the_files_own_clock() {
        let v = video("mpeg2video", 1920, 1080, 4_000_000, "SDR");
        let mut m = media(v.clone());
        m.source = crate::storage::source::Source::Local("/m/a.ts".into());
        m.boundaries = vec![0.0, 7.44];
        m.seek_offsets = vec![-1.0, -1.0];
        m.start_time = 1.44;
        let l = ladder(&v, true, true);
        let a = ffmpeg_args(&m, &l[0], 1).unwrap().join(" ");
        assert!(a.contains("-noaccurate_seek -ss 5.000 -i /m/a.ts -copyts"), "{a}");
    }

    #[test]
    fn command_lines() {
        let v = video("h264", 1920, 1080, 12_000_000, "SDR");
        let m = media(v.clone());
        let l = ladder(&v, true, true);
        let a = ffmpeg_args(&m, &l[2], 1).unwrap().join(" ");
        assert!(
            a.contains("-hwaccel cuda -hwaccel_device cu -hwaccel_output_format cuda -noaccurate_seek -ss 11.919 -i /m/a.mkv -copyts"),
            "{a}"
        );
        assert!(a.contains("-vf scale_cuda=w=1280:h=720:format=nv12"), "{a}");
        assert!(a.contains("-force_key_frames 11.778,26.501 "), "{a}");
        assert!(
            a.contains("-level:v 3.1 ") && a.contains("-bf 0 ") && a.contains("-forced-idr 1"),
            "{a}"
        );
        let p = video_args(&m, &l[2], 0, true).unwrap().join(" ");
        assert!(
            !p.contains("-force_key_frames") && !p.contains("pipe:1") && p.contains("-bf 0 -g 96 "),
            "{p}"
        );
        // The first segment: no seek.
        assert!(!ffmpeg_args(&m, &l[2], 0).unwrap().contains(&"-ss".to_string()));
        assert!(ffmpeg_args(&m, &l[0], 0).is_err(), "the copy isn't transcoded");

        // DivX: decoded on the CPU, uploaded.
        let d = video("mpeg4", 656, 368, 1_200_000, "SDR");
        let a = ffmpeg_args(&media(d.clone()), &ladder(&d, true, true)[0], 1).unwrap().join(" ");
        assert!(a.contains("-threads 3 -noaccurate_seek") && !a.contains("-hwaccel cuda"), "{a}");
        assert!(a.contains("scale=w=656:h=368:flags=bicubic,format=nv12,hwupload_cuda"), "{a}");

        // HDR kept: HEVC Main 10 with the source's colours; tone-mapped: BT.709.
        let h = video("hevc", 3840, 2160, 60_000_000, "PQ");
        let a = ffmpeg_args(&media(h.clone()), &ladder(&h, true, true)[2], 1).unwrap().join(" ");
        assert!(
            a.contains("hevc_nvenc -profile:v main10") && a.contains("format=p010le") && a.contains("-color_trc smpte2084"),
            "{a}"
        );
        let a = ffmpeg_args(&media(h.clone()), &ladder(&h, false, true)[0], 1).unwrap().join(" ");
        assert!(
            a.contains("tonemap_cuda=format=nv12") && a.contains("-color_trc bt709") && a.contains("-preset p1"),
            "{a}"
        );
    }

    /// H.264 stored with 3-byte NAL lengths is copied through h264_mp4toannexb (Apple
    /// players refuse the short lengths); every other copy is as before.
    #[test]
    fn short_nal_lengths_are_rewritten_on_copy() {
        let out = std::path::Path::new("/tmp/out.mp4");
        let mut v = video("h264", 1920, 1080, 12_000_000, "SDR");
        let l = ladder(&v, true, true);
        assert_eq!(l[0].id, "copy");
        let a = crate::downloads::job_args(&media(v.clone()), &l[0], &[], out).unwrap().join(" ");
        assert!(a.contains("-c:v copy -tag:v avc1") && !a.contains("h264_mp4toannexb"), "{a}");
        v.short_nal_lengths = true;
        let a = crate::downloads::job_args(&media(v.clone()), &l[0], &[], out).unwrap().join(" ");
        assert!(a.contains("-c:v copy -tag:v avc1 -bsf:v h264_mp4toannexb"), "{a}");
    }

    #[test]
    fn h264_above_high_is_decoded_on_the_cpu() {
        // x264's lossless encodes are High 4:4:4 Predictive even at 4:2:0; no hardware
        // decoder here takes them, so the pixel format alone doesn't say.
        let backends = [Backend::Nvenc, Backend::Qsv, Backend::Vaapi, Backend::VideoToolbox, Backend::V4l2];
        let mut v = video("h264", 1280, 720, 5_000_000, "SDR");
        v.profile = "High".into();
        assert!(backends.iter().all(|b| hw_decodes(*b, &v)));
        for p in ["High 4:4:4 Predictive", "High 4:2:2", "High 10", "CAVLC 4:4:4"] {
            v.profile = p.into();
            assert!(!backends.iter().any(|b| hw_decodes(*b, &v)), "{p}");
        }
        let nv = hw(Backend::Nvenc);
        let l = ladder_with(&nv, &v, false, true);
        let t = l.iter().find(|x| x.transcode).unwrap();
        let a = video_args_with(&nv, &media(v.clone()), t, 0, false).unwrap().join(" ");
        assert!(!a.contains("-hwaccel cuda") && a.contains("-c:v h264_nvenc"), "{a}");
    }

    fn hw(backend: Backend) -> Hw {
        let mut h = match backend {
            Backend::Cpu => Hw::cpu(4),
            _ => Hw::nvenc(),
        };
        h.backend = backend;
        if matches!(backend, Backend::Qsv | Backend::Vaapi) {
            h.device = Some("/dev/dri/renderD128".into());
            h.vendor = Some("Intel".into());
            h.threads = 4;
            h.hevc10 = false;
            h.gpu_tonemap = false;
        }
        h
    }

    #[test]
    fn vaapi_command_lines() {
        let va = hw(Backend::Vaapi);
        let v = video("h264", 1920, 1080, 12_000_000, "SDR");
        let m = media(v.clone());
        let l = ladder_with(&va, &v, true, true);
        assert_eq!(
            ids(&l),
            ["copy", "h264-1920x1080-10000k", "h264-1280x720-4000k", "h264-854x480-1500k"]
        );
        let a = video_args_with(&va, &m, &l[2], 1, false).unwrap().join(" ");
        assert!(a.starts_with("-nostdin -hide_banner -nostats -v error -init_hw_device vaapi=va:/dev/dri/renderD128 -filter_hw_device va -hwaccel vaapi -hwaccel_device va -hwaccel_output_format vaapi -noaccurate_seek -ss 11.919 -i /m/a.mkv -copyts"), "{a}");
        assert!(
            a.contains("-vf scale_vaapi=w=1280:h=720:format=nv12 -c:v h264_vaapi -profile:v high -level:v 3.1 -rc_mode VBR -b:v 4000000"),
            "{a}"
        );
        assert!(a.contains("-bf 0 -g 100000 -force_key_frames 11.778,26.501 "), "{a}");
        // DivX: decoded on the CPU, uploaded, scaled on the GPU.
        let d = video("mpeg4", 656, 368, 1_200_000, "SDR");
        let a = video_args_with(&va, &media(d.clone()), &ladder_with(&va, &d, true, true)[0], 1, false)
            .unwrap()
            .join(" ");
        assert!(a.contains("-threads 4 -noaccurate_seek") && !a.contains("-hwaccel vaapi"), "{a}");
        assert!(a.contains("-vf format=nv12,hwupload,scale_vaapi=w=656:h=368:format=nv12"), "{a}");
        // HDR kept (HEVC Main 10) and tone-mapped on the GPU (Intel).
        let mut va10 = va.clone();
        va10.hevc10 = true;
        va10.gpu_tonemap = true;
        let h = video("hevc", 3840, 2160, 60_000_000, "PQ");
        let a = video_args_with(&va10, &media(h.clone()), &ladder_with(&va10, &h, true, true)[2], 1, false)
            .unwrap()
            .join(" ");
        assert!(
            a.contains("scale_vaapi=w=1920:h=1080:format=p010 -c:v hevc_vaapi -profile:v main10 -tier main -tag:v hvc1")
                && a.contains("-color_trc smpte2084"),
            "{a}"
        );
        let a = video_args_with(&va10, &media(h.clone()), &ladder_with(&va10, &h, false, true)[0], 1, false)
            .unwrap()
            .join(" ");
        assert!(a.contains("-vf setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc,scale_vaapi=w=3840:h=2160:format=p010,tonemap_vaapi=format=nv12:p=bt709:t=bt709:m=bt709 -c:v h264_vaapi"), "{a}");
        // No HEVC 10-bit encoder, no GPU tone-mapping (AMD): the transcodes are tone-mapped SDR
        // H.264, mapped on the CPU; the HDR copy stays first for HDR screens.
        let amd = Hw {
            vendor: Some("AMD".into()),
            ..va.clone()
        };
        let l = ladder_with(&amd, &h, true, true);
        assert_eq!(ids(&l)[0], "copy");
        assert!(
            l[1..].iter().all(|x| x.tonemap && x.codec == "h264" && x.range == "SDR"),
            "{:?}",
            ids(&l)
        );
        let a = video_args_with(&amd, &media(h.clone()), &l[1], 1, false).unwrap().join(" ");
        assert!(a.contains("-vf scale_vaapi=w=3840:h=2160:format=p010,hwdownload,format=p010le,setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc,zscale=t=linear:npl=100"), "{a}");
        assert!(a.contains("format=yuv420p,format=nv12,hwupload -c:v h264_vaapi"), "{a}");
    }

    #[test]
    fn qsv_command_lines() {
        let mut qs = hw(Backend::Qsv);
        let v = video("hevc", 1920, 1080, 12_000_000, "SDR");
        let m = media(v.clone());
        let l = ladder_with(&qs, &v, true, true);
        let a = video_args_with(&qs, &m, &l[1], 1, false).unwrap().join(" ");
        assert!(a.contains("-init_hw_device vaapi=va:/dev/dri/renderD128 -init_hw_device qsv=qs@va -filter_hw_device qs -hwaccel vaapi -hwaccel_device va -hwaccel_output_format vaapi"), "{a}");
        assert!(a.contains("-vf hwmap=derive_device=qsv,format=qsv,scale_qsv=w=1920:h=1080:format=nv12 -c:v h264_qsv -profile:v high -level:v 4.0 -preset fast"), "{a}");
        assert!(
            a.contains("-bf 0 -g 100000 -forced_idr 1 -adaptive_i 0 -adaptive_b 0 -force_key_frames"),
            "{a}"
        );
        // GPU tone-mapping through vpp_qsv; HEVC Main 10 without a level.
        qs.gpu_tonemap = true;
        qs.hevc10 = true;
        let h = video("hevc", 3840, 2160, 60_000_000, "HLG");
        let a = video_args_with(&qs, &media(h.clone()), &ladder_with(&qs, &h, false, true)[0], 1, false)
            .unwrap()
            .join(" ");
        assert!(a.contains("-vf setparams=color_primaries=bt2020:color_trc=arib-std-b67:colorspace=bt2020nc,hwmap=derive_device=qsv,format=qsv,vpp_qsv=tonemap=1:format=nv12:w=3840:h=2160 -c:v h264_qsv"), "{a}");
        assert!(a.contains("-preset veryfast"), "{a}");
        let a = video_args_with(&qs, &media(h.clone()), &ladder_with(&qs, &h, true, true)[1], 1, false)
            .unwrap()
            .join(" ");
        assert!(
            a.contains("-c:v hevc_qsv -profile:v main10 -tag:v hvc1 -preset") && !a.contains("-level:v"),
            "{a}"
        );
        // A 10-bit H.264 source: decoded on the CPU, uploaded as P010.
        let mut x = video("h264", 1280, 720, 6_000_000, "SDR");
        x.pix_fmt = "yuv420p10le".into();
        let a = video_args_with(&qs, &media(x.clone()), &ladder_with(&qs, &x, true, true)[1], 0, true)
            .unwrap()
            .join(" ");
        assert!(
            a.contains("-threads 4") && a.contains("-vf format=p010le,hwupload=extra_hw_frames=64,scale_qsv=w=1280:h=720:format=nv12"),
            "{a}"
        );
    }

    #[test]
    fn cpu_command_lines() {
        let cpu = hw(Backend::Cpu);
        // At most 1080p: the 4K preset is left out, "Original" of a 4K AV1 is 1080p.
        let a1 = video("av1", 3840, 2160, 20_000_000, "SDR");
        let l = ladder_with(&cpu, &a1, true, true);
        assert_eq!(
            ids(&l),
            [
                "h264-1920x1080-20000k",
                "h264-1920x1080-10000k",
                "h264-1280x720-4000k",
                "h264-854x480-1500k"
            ]
        );
        let a = video_args_with(&cpu, &media(a1.clone()), &l[2], 1, false).unwrap().join(" ");
        assert!(
            a.starts_with("-nostdin -hide_banner -nostats -v error -threads 4 -noaccurate_seek -ss 11.919"),
            "{a}"
        );
        assert!(
            a.contains(
                "-vf scale=w=1280:h=720:flags=bicubic,format=yuv420p -c:v libx264 -profile:v high -level:v 3.1 -preset veryfast -threads 4"
            ),
            "{a}"
        );
        assert!(
            a.contains("-bf 0 -g 100000 -sc_threshold 0 -forced-idr 1 -force_key_frames 11.778,26.501"),
            "{a}"
        );
        // HDR: the copy for HDR screens, everything else tone-mapped on the CPU.
        let h = video("hevc", 3840, 2160, 60_000_000, "PQ");
        let l = ladder_with(&cpu, &h, true, true);
        assert_eq!(
            ids(&l),
            [
                "copy",
                "h264-1920x1080-20000k-tm",
                "h264-1920x1080-10000k-tm",
                "h264-1280x720-4000k-tm",
                "h264-854x480-1500k-tm"
            ]
        );
        let l = ladder_with(&cpu, &h, false, true);
        assert_eq!(
            ids(&l),
            [
                "h264-1920x1080-20000k-tm",
                "h264-1920x1080-10000k-tm",
                "h264-1280x720-4000k-tm",
                "h264-854x480-1500k-tm"
            ]
        );
        let a = video_args_with(&cpu, &media(h.clone()), &l[0], 1, false).unwrap().join(" ");
        assert!(a.contains("-vf scale=w=1920:h=1080:flags=bicubic,setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc,zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p -c:v libx264"), "{a}");
        // Nothing to transcode with: only the copy.
        let none = Hw { available: false, ..cpu };
        assert_eq!(ids(&ladder_with(&none, &h, true, true)), ["copy"]);
    }

    #[test]
    fn nvidia_unchanged_by_the_backends() {
        // The NVIDIA ladder and command lines are what they were before the backends: the
        // default backend is NVENC, and `ladder_with(Hw::nvenc())` is `ladder`.
        let n = Hw::nvenc();
        for (v, hdr) in [
            (video("hevc", 3840, 2160, 60_000_000, "PQ"), true),
            (video("hevc", 3840, 2160, 60_000_000, "PQ"), false),
            (video("mpeg4", 656, 368, 1_200_000, "SDR"), true),
        ] {
            let l = ladder_with(&n, &v, hdr, true);
            assert_eq!(l, ladder(&v, hdr, true));
            for x in l.iter().filter(|x| x.transcode) {
                assert_eq!(
                    video_args_with(&n, &media(v.clone()), x, 1, false).unwrap(),
                    video_args(&media(v.clone()), x, 1, false).unwrap()
                );
            }
        }
    }

    #[test]
    fn download_runs_yield_to_streams() {
        let cache = Arc::new(Cache::new(std::env::temp_dir().join("moonrise-admit-dl-test"), 1 << 20));
        let t = Transcoder::new("ffmpeg".into(), cache, 1);
        let v = video("h264", 1920, 1080, 12_000_000, "SDR");
        let l = ladder(&v, true, true);
        let a = media(v);
        let run = |owner: &str| {
            Arc::new(Run {
                id: 1,
                start: 0,
                owner: owner.into(),
                progress: watch::channel(Progress {
                    next: 0,
                    init: false,
                    done: None,
                })
                .0,
                demand: watch::channel(0).0,
                last_request: Mutex::new(Instant::now()),
                cancel: watch::channel(false).0,
            })
        };
        // The only slot is a download's: a new stream is still admitted (it may stop it).
        t.runs.lock().unwrap().insert("dl".into(), vec![run("dl:u:i:v")]);
        assert!(t.admit(&a, &l[1], "s1"));
        // A stream's run in it: another stream is not.
        t.runs.lock().unwrap().insert("st".into(), vec![run("s1")]);
        t.reserved.lock().unwrap().clear();
        assert!(!t.admit(&a, &l[2], "s2"));
        assert!(is_download("dl:x") && !is_download("s1"));
    }

    #[test]
    fn admission_reserves_slots() {
        let cache = Arc::new(Cache::new(std::env::temp_dir().join("moonrise-admit-test"), 1 << 20));
        let t = Transcoder::new("ffmpeg".into(), cache, 2);
        let v = video("h264", 1920, 1080, 12_000_000, "SDR");
        let l = ladder(&v, true, true);
        let (a, b) = (media(v.clone()), {
            let mut m = media(v);
            m.item_id = "j".into();
            m
        });
        // Two sessions at once take both slots; a third (another title) is refused, while one
        // more for an already admitted variant shares it.
        assert!(t.admit(&a, &l[1], "s1"));
        assert!(t.admit(&b, &l[1], "s2"));
        assert!(!t.admit(&a, &l[2], "s3"));
        assert!(t.admit(&a, &l[1], "s4"));
        // A closed session starts nothing, and gives its reserved slot back.
        t.stop_owner("s1");
        assert!(t.admit(&a, &l[2], "s5"));
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let (t, a) = (Arc::new(t), Arc::new(a));
        let err = rt.block_on(t.run_for(&a, &l[1], "track", 1, "s1")).err().unwrap();
        assert!(err.to_string().contains("session closed"));
    }

    #[test]
    fn boxes_in_a_stream() {
        let mut b = 16u32.to_be_bytes().to_vec();
        b.extend_from_slice(b"moof");
        b.extend_from_slice(&[0u8; 8]);
        assert_eq!(next_box(&b).unwrap(), Some((*b"moof", 16)));
        assert_eq!(next_box(&b[..12]).unwrap(), None);
        assert!(next_box(&[0, 0, 0, 4, b'f', b'r', b'e', b'e']).is_err());
    }

    #[test]
    fn videotoolbox_command_lines() {
        let vt = Hw::videotoolbox(4);
        // SDR H.264: decoded, scaled and encoded on VideoToolbox frames.
        let v = video("h264", 1920, 1080, 12_000_000, "SDR");
        let l = ladder_with(&vt, &v, true, true);
        assert_eq!(
            ids(&l),
            ["copy", "h264-1920x1080-10000k", "h264-1280x720-4000k", "h264-854x480-1500k"]
        );
        let a = video_args_with(&vt, &media(v.clone()), &l[2], 1, false).unwrap().join(" ");
        assert!(a.starts_with("-nostdin -hide_banner -nostats -v error -init_hw_device videotoolbox=vt -filter_hw_device vt -hwaccel videotoolbox -hwaccel_device vt -hwaccel_output_format videotoolbox_vld -noaccurate_seek -ss 11.919 -i /m/a.mkv -copyts"), "{a}");
        assert!(a.contains("-map 0:0 -vf scale_vt=w=1280:h=720 -c:v h264_videotoolbox -profile:v high -level:v 3.1 -allow_sw 0 -b:v 4000000 -maxrate 5000000 -bufsize 8000000 -bf 0 -g 100000 -force_key_frames 11.778,26.501"), "{a}");
        // HDR for an SDR screen: tone-mapped by scale_vt, no colour matrix asked for.
        let hdr = video("hevc", 3840, 2160, 60_000_000, "PQ");
        let l = ladder_with(&vt, &hdr, false, true);
        assert_eq!(l[0].id, "h264-3840x2160-40000k-tm");
        let a = video_args_with(&vt, &media(hdr.clone()), &l[1], 0, false).unwrap().join(" ");
        assert!(
            a.contains("-vf scale_vt=w=1920:h=1080:color_matrix=bt709:color_primaries=bt709:color_transfer=bt709 -c:v h264_videotoolbox"),
            "{a}"
        );
        assert!(
            a.contains("-color_primaries bt709 -color_trc bt709 -color_range tv") && !a.contains("-colorspace"),
            "{a}"
        );
        // HDR screen: HEVC Main 10, the level left to the encoder.
        let l = ladder_with(&vt, &hdr, true, true);
        let h = l.iter().find(|x| x.codec == "hevc" && x.transcode).unwrap();
        let a = video_args_with(&vt, &media(hdr.clone()), h, 0, false).unwrap().join(" ");
        assert!(
            a.contains("-c:v hevc_videotoolbox -profile:v main10 -tag:v hvc1 -allow_sw 0") && !a.contains("-level:v"),
            "{a}"
        );
        // DivX: decoded on the CPU, uploaded.
        let d = video("mpeg4", 656, 368, 1_200_000, "SDR");
        let l = ladder_with(&vt, &d, true, true);
        let a = video_args_with(&vt, &media(d.clone()), &l[0], 0, false).unwrap().join(" ");
        assert!(
            a.contains("-filter_hw_device vt -threads 4 -noaccurate_seek")
                && a.contains("-vf format=nv12,hwupload,scale_vt=w=656:h=368 -c:v h264_videotoolbox"),
            "{a}"
        );
        // Tone-mapping on the CPU when scale_vt can't, from the device.
        let mut cpu_tm = vt.clone();
        cpu_tm.gpu_tonemap = false;
        let l = ladder_with(&cpu_tm, &hdr, false, true);
        let a = video_args_with(&cpu_tm, &media(hdr.clone()), &l[1], 0, false).unwrap().join(" ");
        assert!(
            a.contains(
                "-vf hwdownload,format=p010le,scale=w=1920:h=1080:flags=bicubic,setparams=color_primaries=bt2020:color_trc=smpte2084"
            ),
            "{a}"
        );
        assert!(a.contains("-colorspace bt709"), "{a}");
        // An Intel Mac: narrower than 640 goes to libx264.
        let intel = Hw {
            min_width: 640,
            ..vt.clone()
        };
        let small = video("mpeg4", 480, 270, 1_000_000, "SDR");
        let l = ladder_with(&intel, &small, true, true);
        let a = video_args_with(&intel, &media(small.clone()), &l[0], 0, false).unwrap().join(" ");
        assert!(a.contains("-c:v libx264") && !a.contains("videotoolbox"), "{a}");
        let a = video_args_with(&intel, &media(v.clone()), &ladder_with(&intel, &v, true, true)[3], 0, false)
            .unwrap()
            .join(" ");
        assert!(a.contains("scale_vt=w=854:h=480 -c:v h264_videotoolbox"), "{a}");
        // Neither (Homebrew's ffmpeg has no zscale): HDR plays only as it is.
        cpu_tm.cpu_tonemap = false;
        assert_eq!(ids(&ladder_with(&cpu_tm, &hdr, false, true)), ["copy"]);
        assert!(ladder_with(&cpu_tm, &hdr, true, true).len() > 1, "kept HDR needs no tone-mapping");
    }

    #[test]
    fn v4l2_command_lines() {
        let pi = Hw::v4l2(3);
        let v = video("h264", 1920, 1080, 12_000_000, "SDR");
        let l = ladder_with(&pi, &v, true, true);
        assert_eq!(
            ids(&l),
            ["copy", "h264-1920x1080-10000k", "h264-1280x720-4000k", "h264-854x480-1500k"]
        );
        let a = video_args_with(&pi, &media(v.clone()), &l[2], 0, false).unwrap().join(" ");
        assert!(a.starts_with("-nostdin -hide_banner -nostats -v error -c:v h264_v4l2m2m -noaccurate_seek -i /m/a.mkv -copyts -map 0:0 -vf scale=w=1280:h=720:flags=bicubic,format=yuv420p -c:v h264_v4l2m2m -profile:v high -b:v 4000000"), "{a}");
        assert!(!a.contains("-level:v") && a.contains("-bf 0 -g 100000 -force_key_frames"), "{a}");
        // DivX: CPU decoding, at most 1080p.
        let d = video("mpeg4", 1920, 1080, 8_000_000, "SDR");
        let a = video_args_with(&pi, &media(d.clone()), &ladder_with(&pi, &d, true, true)[0], 0, false)
            .unwrap()
            .join(" ");
        assert!(
            a.contains("-threads 3 -noaccurate_seek") && !a.contains("_v4l2m2m -noaccurate"),
            "{a}"
        );
        // 4K and 10-bit HEVC: as they are only.
        assert_eq!(
            ids(&ladder_with(&pi, &video("hevc", 3840, 2160, 60_000_000, "PQ"), false, true)),
            ["copy"]
        );
        assert_eq!(
            ids(&ladder_with(&pi, &video("hevc", 1920, 1080, 8_000_000, "PQ"), false, true)),
            ["copy"]
        );
        assert!(ladder_with(&pi, &video("h264", 3840, 2160, 40_000_000, "SDR"), true, true)
            .iter()
            .all(|x| !x.transcode));
        assert!(pi
            .too_heavy(&video("h264", 3840, 2160, 40_000_000, "SDR"))
            .unwrap()
            .contains("2160p"));
        assert!(pi
            .too_heavy(&video("hevc", 1920, 1080, 8_000_000, "PQ"))
            .unwrap()
            .contains("10-bit HEVC"));
        assert!(Hw::nvenc().too_heavy(&video("hevc", 3840, 2160, 60_000_000, "PQ")).is_none());
        assert!(
            Hw::cpu(4).too_heavy(&video("hevc", 3840, 2160, 60_000_000, "PQ")).is_none(),
            "a PC's CPU tries"
        );
        let board_cpu = Hw {
            vendor: Some("Raspberry Pi 5 Model B Rev 1.0".into()),
            max_height: 720,
            ..Hw::cpu(3)
        };
        assert!(
            board_cpu.too_heavy(&video("hevc", 3840, 2160, 60_000_000, "PQ")).is_some(),
            "a board's CPU doesn't"
        );
        let l = ladder_with(&board_cpu, &v, true, true);
        assert_eq!(ids(&l), ["copy", "h264-1280x720-4000k", "h264-854x480-1500k"], "720p at most");
        let a = video_args_with(&board_cpu, &media(v.clone()), &l[1], 0, false).unwrap().join(" ");
        assert!(
            a.contains("-c:v libx264 -profile:v high -level:v 3.1 -preset superfast -threads 3"),
            "{a}"
        );
        // 4K MPEG-4 Part 2 can't be copied: nothing to serve (the playback answer says why).
        assert!(ladder_with(&pi, &video("mpeg4", 3840, 2160, 40_000_000, "SDR"), true, true).is_empty());
    }

    #[test]
    fn a_stop_sent_before_listening_is_seen() {
        // Sent while nobody listens (the run's task hasn't started).
        let (tx, _) = watch::channel(false);
        tx.send_replace(true);
        let mut rx = stop_signal(&tx);
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(1), rx.changed())
                .await
                .expect("seen at once")
                .unwrap();
        });
        // Not stopped yet: waits for the stop.
        let (tx, _) = watch::channel(false);
        let mut rx = stop_signal(&tx);
        rt.block_on(async {
            assert!(tokio::time::timeout(Duration::from_millis(50), rx.changed()).await.is_err());
            tx.send_replace(true);
            rx.changed().await.unwrap();
        });
    }

    /// The playback's prewarm starts a run at segment 0 and the client's first request, for a
    /// later segment, comes before that run's task has started listening for a stop. With one
    /// slot, the request stops the prewarm run and gets the slot (it used to wait 10 s and fail
    /// with "no transcode slot free": the stop was lost).
    #[tokio::test]
    async fn a_run_stopped_before_it_listens_frees_its_slot() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("moonrise-stop-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // An ffmpeg that never writes: only a stop ends its run.
        let ffmpeg = dir.join("ffmpeg");
        std::fs::write(&ffmpeg, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cache = Arc::new(Cache::new(dir.join("cache"), 1 << 20));
        let t = Arc::new(Transcoder::new(ffmpeg.to_string_lossy().into(), cache, 1));
        let v = video("h264", 1920, 1080, 12_000_000, "SDR");
        let l = ladder(&v, true, true);
        let mut m = media(v);
        m.duration = 120.0;
        m.boundaries = (0..20).map(|i| i as f64 * 6.0).collect();
        m.seek_offsets = vec![0.0; 20];
        let m = Arc::new(m);

        let prewarm = t.run_for(&m, &l[1], "track", 0, "s1").await.unwrap();
        // Its task hasn't run yet (one thread, no await since): the request comes first.
        let run = tokio::time::timeout(Duration::from_secs(5), t.run_for(&m, &l[1], "track", 15, "s1"))
            .await
            .expect("a slot well before the 10 s limit")
            .unwrap();
        assert_eq!(run.start, 15);
        let mut done = prewarm.progress.subscribe();
        tokio::time::timeout(Duration::from_secs(5), done.wait_for(|p| p.done.is_some()))
            .await
            .expect("the prewarm run ended")
            .unwrap();
        assert_eq!(prewarm.next(), 0);
        t.stop_owner("s1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The prewarm's prefetch, once its run is stopped by the player's first request for a later
    /// segment, starts no other run: that would stop the run the request waits on (each then
    /// started another, back and forth).
    #[tokio::test]
    async fn a_stopped_prefetch_leaves_the_request_its_run() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("moonrise-prefetch-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ffmpeg = dir.join("ffmpeg");
        std::fs::write(&ffmpeg, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cache = Arc::new(Cache::new(dir.join("cache"), 1 << 20));
        let t = Arc::new(Transcoder::new(ffmpeg.to_string_lossy().into(), cache, 1));
        let v = video("h264", 1920, 1080, 12_000_000, "SDR");
        let l = ladder(&v, true, true);
        let mut m = media(v);
        m.duration = 120.0;
        m.boundaries = (0..20).map(|i| i as f64 * 6.0).collect();
        m.seek_offsets = vec![0.0; 20];
        let m = Arc::new(m);
        let track = Transcoder::track_key(&m, &l[1]);

        t.prefetch(&m, &l[1], 0, "s1");
        while t.next_id.load(Ordering::Relaxed) < 2 {
            tokio::task::yield_now().await;
        }
        let run = tokio::time::timeout(Duration::from_secs(5), t.run_for(&m, &l[1], &track, 15, "s1"))
            .await
            .expect("a slot")
            .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!run.ended(), "the request's run was stopped");
        assert_eq!(t.next_id.load(Ordering::Relaxed), 3, "the prefetch started another run");
        t.stop_owner("s1");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
