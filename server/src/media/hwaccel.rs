//! What video is transcoded with: found once at start, used by every transcode (`transcode.rs`).
//!
//! **Backends**, in the order `auto` tries them:
//!
//! - `nvenc`: an NVIDIA GPU (NVDEC, `scale_cuda`/`tonemap_cuda`/`overlay_cuda`, NVENC). Found when
//!   the NVIDIA runtime gives the container the GPU (`nvidia-smi` answers, or `/dev/nvidiactl`
//!   is there) and ffmpeg has `h264_nvenc`. Taken without a test run (a CUDA start takes
//!   seconds; the warm-up after start pays it, as before).
//! - `qsv`: Intel Quick Sync, through the render node (`/dev/dri/renderD*`, vendor 0x8086):
//!   decoded with VA-API, mapped to QSV, `scale_qsv`/`vpp_qsv`, `h264_qsv`/`hevc_qsv`.
//! - `vaapi`: VA-API on Intel or AMD (vendor 0x1002) render nodes: `scale_vaapi`,
//!   `tonemap_vaapi` (Intel), `h264_vaapi`/`hevc_vaapi`.
//! - `videotoolbox` (macOS only, and the only GPU backend there): Apple's VideoToolbox on Apple
//!   silicon and on Intel Macs (Quick Sync underneath). Decoded with `-hwaccel videotoolbox` into
//!   GPU frames, scaled and tone-mapped by `scale_vt` (VideoToolbox's pixel transfer, which maps
//!   PQ/HLG to SDR), `h264_videotoolbox`/`hevc_videotoolbox`.
//! - `v4l2`: the V4L2 memory-to-memory encoder of a Raspberry Pi 4 (`h264_v4l2m2m`, at most
//!   1080p; pictures scaled on the CPU). Sources it can't keep up with (4K, 10-bit HEVC) aren't
//!   transcoded: they play as they are, or the app is told why not.
//! - `cpu`: libx264 (`veryfast`), at most 1080p, threads shared out between the transcode slots.
//!
//! QSV, VA-API, VideoToolbox and V4L2 are only taken after a short test encode (5 frames) works;
//! HEVC Main 10 (HDR kept for HDR screens) and GPU tone-mapping are tested apart, and fall back to
//! tone-mapping to H.264 and to tone-mapping on the CPU (`zscale`; an ffmpeg without it, like
//! Homebrew's, can't tone-map on the CPU: HDR titles then play only as they are). The setting
//! `transcode.hardware` (`MOONRISE_HWACCEL`: auto, nvidia, qsv, vaapi, videotoolbox, v4l2, cpu) picks
//! one; `transcode.device` (`MOONRISE_HWACCEL_DEVICE`) a render node. A backend asked for that doesn't
//! work falls back to `auto` (the reason is shown in Manage Server → Playback & Hardware).

use std::{
    path::Path,
    process::{Command, Stdio},
    sync::OnceLock,
    time::{Duration, Instant},
};

use serde::Serialize;
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Nvenc,
    Qsv,
    Vaapi,
    #[serde(rename = "videotoolbox")]
    VideoToolbox,
    V4l2,
    Cpu,
}

impl Backend {
    pub fn key(self) -> &'static str {
        match self {
            Backend::Nvenc => "nvenc",
            Backend::Qsv => "qsv",
            Backend::Vaapi => "vaapi",
            Backend::VideoToolbox => "videotoolbox",
            Backend::V4l2 => "v4l2",
            Backend::Cpu => "cpu",
        }
    }
}

/// The backends `auto` tries on this system, in order (`transcode.hardware` values).
pub fn platform_order() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["videotoolbox", "cpu"]
    } else {
        &["nvidia", "qsv", "vaapi", "v4l2", "cpu"]
    }
}

/// What `transcode.hardware` may be set to here, with a name for people (the apps' picker).
pub fn choices() -> Vec<(&'static str, &'static str)> {
    let mut out = vec![("auto", "Automatic")];
    for k in platform_order() {
        out.push(match *k {
            "nvidia" => ("nvidia", "NVIDIA"),
            "qsv" => ("qsv", "Intel Quick Sync"),
            "vaapi" => ("vaapi", "VA-API (Intel, AMD)"),
            "videotoolbox" => ("videotoolbox", "VideoToolbox"),
            "v4l2" => ("v4l2", "V4L2 (Raspberry Pi)"),
            _ => ("cpu", "CPU"),
        });
    }
    out
}

/// The transcoding hardware in use.
#[derive(Clone, Debug, Serialize)]
pub struct Hw {
    pub backend: Backend,
    /// The render node (QSV, VA-API).
    pub device: Option<String>,
    /// "NVIDIA", "Intel", "AMD" (the GPU), none for the CPU.
    pub vendor: Option<String>,
    /// HEVC Main 10 can be encoded: HDR is kept for HDR screens. Otherwise it is tone-mapped to
    /// H.264 (the original still plays as it is).
    pub hevc10: bool,
    /// Tone-mapping runs on the GPU (otherwise on the CPU between download and upload).
    pub gpu_tonemap: bool,
    /// ffmpeg can tone-map on the CPU (`zscale`).
    pub cpu_tonemap: bool,
    /// Threads per transcode run (CPU decoding and the CPU encoder).
    pub threads: usize,
    /// The largest picture height made (the CPU makes at most 1080p).
    pub max_height: i64,
    /// The narrowest picture the encoder takes (VideoToolbox on Intel Macs: 640); narrower ones
    /// are encoded with libx264.
    pub min_width: i64,
    /// What `transcode.hardware` asked for.
    pub requested: String,
    /// Why another backend than the one asked for is used (or why a feature is off).
    pub notes: Vec<String>,
    /// False: nothing transcodes here (no encoder at all).
    pub available: bool,
}

impl Hw {
    /// The NVIDIA backend (the default before detection, and in tests).
    pub fn nvenc() -> Hw {
        Hw {
            backend: Backend::Nvenc,
            device: None,
            vendor: Some("NVIDIA".into()),
            hevc10: true,
            gpu_tonemap: true,
            cpu_tonemap: true,
            threads: 3,
            max_height: 2160,
            min_width: 0,
            requested: "auto".into(),
            notes: vec![],
            available: true,
        }
    }

    pub fn cpu(threads: usize) -> Hw {
        Hw {
            backend: Backend::Cpu,
            device: None,
            vendor: None,
            hevc10: false,
            gpu_tonemap: false,
            cpu_tonemap: true,
            threads,
            max_height: 1080,
            min_width: 0,
            requested: "auto".into(),
            notes: vec![],
            available: true,
        }
    }

    /// VideoToolbox (macOS), with everything the test encodes may switch off.
    pub fn videotoolbox(threads: usize) -> Hw {
        let vendor = if cfg!(target_arch = "aarch64") { "Apple" } else { "Intel" };
        Hw {
            backend: Backend::VideoToolbox,
            device: None,
            vendor: Some(vendor.into()),
            hevc10: true,
            gpu_tonemap: true,
            cpu_tonemap: true,
            threads,
            max_height: 2160,
            min_width: 0,
            requested: "auto".into(),
            notes: vec![],
            available: true,
        }
    }

    /// A V4L2 M2M encoder (Raspberry Pi 4): H.264 up to 1080p.
    pub fn v4l2(threads: usize) -> Hw {
        Hw {
            backend: Backend::V4l2,
            device: None,
            vendor: board_model(),
            hevc10: false,
            gpu_tonemap: false,
            cpu_tonemap: true,
            threads,
            max_height: 1080,
            min_width: 0,
            requested: "auto".into(),
            notes: vec![],
            available: true,
        }
    }

    pub fn label(&self) -> String {
        match self.backend {
            Backend::Nvenc => "NVIDIA NVENC".into(),
            Backend::Qsv => "Intel Quick Sync".into(),
            Backend::Vaapi => format!("{} VA-API", self.vendor.as_deref().unwrap_or("GPU")),
            Backend::VideoToolbox => "Apple VideoToolbox".into(),
            Backend::V4l2 => match self.vendor.as_deref() {
                Some(m) if m.starts_with("Raspberry Pi") => "Raspberry Pi V4L2".into(),
                _ => "V4L2 M2M".into(),
            },
            Backend::Cpu => "CPU (libx264)".into(),
        }
    }

    /// HDR can be tone-mapped to SDR by something here.
    pub fn can_tonemap(&self) -> bool {
        self.backend == Backend::Nvenc || self.gpu_tonemap || self.cpu_tonemap
    }

    /// Why this backend shouldn't transcode the track at all (too heavy to keep up with in real
    /// time): it then plays only as it is. None but on a single-board computer (a Raspberry Pi's
    /// V4L2 encoder, or its CPU).
    pub fn too_heavy(&self, v: &crate::media::VideoTrack) -> Option<String> {
        let board = self.backend == Backend::V4l2 || (self.backend == Backend::Cpu && self.vendor.is_some());
        if !board {
            return None;
        }
        let ten_bit = v.pix_fmt.contains("10") || v.pix_fmt.contains("12");
        let what = self.vendor.clone().unwrap_or_else(|| self.label());
        if v.height > 1088 || v.width > 2048 {
            Some(format!(
                "{}p video is too much to transcode on this server's hardware ({what}): it plays only as it is",
                v.height
            ))
        } else if v.codec == "hevc" && ten_bit {
            Some(format!(
                "10-bit HEVC is too much to transcode on this server's hardware ({what}): it plays only as it is"
            ))
        } else {
            None
        }
    }

    /// For the admin API: what decodes, scales, tone-maps and encodes.
    pub fn describe(&self) -> Value {
        let (decode, scale, encode) = match self.backend {
            Backend::Nvenc => (
                "NVDEC (CPU for MPEG-4 Part 2, 10-bit H.264)",
                "scale_cuda",
                "h264_nvenc, hevc_nvenc",
            ),
            Backend::Qsv => (
                "VA-API for H.264/HEVC, else CPU",
                "scale_qsv",
                if self.hevc10 { "h264_qsv, hevc_qsv" } else { "h264_qsv" },
            ),
            Backend::Vaapi => (
                "VA-API for H.264/HEVC, else CPU",
                "scale_vaapi",
                if self.hevc10 { "h264_vaapi, hevc_vaapi" } else { "h264_vaapi" },
            ),
            Backend::VideoToolbox => (
                "VideoToolbox for H.264/HEVC, else CPU",
                "scale_vt",
                if self.hevc10 {
                    "h264_videotoolbox, hevc_videotoolbox"
                } else {
                    "h264_videotoolbox"
                },
            ),
            Backend::V4l2 => ("V4L2 for H.264 up to 1080p, else CPU", "scale (CPU)", "h264_v4l2m2m"),
            Backend::Cpu => ("CPU", "scale (CPU)", "libx264"),
        };
        let tonemap = match (self.backend, self.gpu_tonemap) {
            (Backend::Nvenc, _) => "tonemap_cuda",
            (Backend::Qsv, true) => "vpp_qsv",
            (Backend::Vaapi, true) => "tonemap_vaapi",
            (Backend::VideoToolbox, true) => "scale_vt (VideoToolbox)",
            _ if !self.cpu_tonemap => "Not available (HDR plays as it is)",
            _ => "CPU (zscale)",
        };
        json!({
            "backend": self.backend.key(),
            "label": self.label(),
            "vendor": self.vendor,
            "device": self.device,
            "decode": decode,
            "scale": scale,
            "tonemap": tonemap,
            "encode": encode,
            "hdr_kept": self.hevc10,
            "max_height": self.max_height,
            "min_width": self.min_width,
            "threads": self.threads,
            "requested": self.requested,
            "notes": self.notes,
            "available": self.available,
            "choices": choices().into_iter().map(|(k, l)| json!({"key": k, "label": l})).collect::<Vec<_>>(),
        })
    }
}

static CURRENT: OnceLock<Hw> = OnceLock::new();

/// The backend in use (NVIDIA until `set` is called: the CLI commands and the tests).
pub fn current() -> &'static Hw {
    static DEFAULT: OnceLock<Hw> = OnceLock::new();
    CURRENT.get().unwrap_or_else(|| DEFAULT.get_or_init(Hw::nvenc))
}

pub fn set(hw: Hw) {
    let _ = CURRENT.set(hw);
}

/// A render node and its GPU's vendor.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderNode {
    pub device: String,
    pub vendor: &'static str,
    /// The node exists in this container (passed with `devices: /dev/dri`).
    pub usable: bool,
}

/// Render nodes the kernel lists (`/sys/class/drm/renderD*`), NVIDIA's left out.
pub fn render_nodes() -> Vec<RenderNode> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir("/sys/class/drm") else { return out };
    let mut names: Vec<String> = rd
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("renderD"))
        .collect();
    names.sort();
    for n in names {
        let vendor = std::fs::read_to_string(format!("/sys/class/drm/{n}/device/vendor")).unwrap_or_default();
        let vendor = match vendor.trim() {
            "0x8086" => "Intel",
            "0x1002" => "AMD",
            "0x10de" => continue,
            _ => "other",
        };
        let device = format!("/dev/dri/{n}");
        out.push(RenderNode {
            usable: Path::new(&device).exists(),
            device,
            vendor,
        });
    }
    // A node passed in without /sys showing it (some NAS kernels): vendor unknown.
    if out.is_empty() {
        if let Ok(rd) = std::fs::read_dir("/dev/dri") {
            for e in rd.filter_map(std::result::Result::ok) {
                let n = e.file_name().to_string_lossy().to_string();
                if n.starts_with("renderD") {
                    out.push(RenderNode {
                        device: format!("/dev/dri/{n}"),
                        vendor: "other",
                        usable: true,
                    });
                }
            }
        }
    }
    out
}

/// NVIDIA GPUs as `nvidia-smi` lists them (name, driver, memory in MB).
pub fn nvidia_gpus() -> Vec<Value> {
    run("nvidia-smi", &["--query-gpu=name,driver_version,memory.total", "--format=csv,noheader,nounits"])
        .map(|s| {
            s.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| {
                    let p: Vec<&str> = l.split(',').map(str::trim).collect();
                    json!({"vendor": "NVIDIA", "name": p.first(), "driver": p.get(1), "memory_mb": p.get(2).and_then(|m| m.parse::<u64>().ok())})
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The board's model on single-board computers ("Raspberry Pi 4 Model B Rev 1.4"): the device
/// tree, else `/proc/cpuinfo`'s `Model` line (containers don't see the device tree).
pub fn board_model() -> Option<String> {
    let tree = std::fs::read_to_string("/proc/device-tree/model")
        .ok()
        .map(|m| m.trim_end_matches('\0').trim().to_string())
        .filter(|m| !m.is_empty());
    tree.or_else(|| {
        let info = std::fs::read_to_string("/proc/cpuinfo").ok()?;
        info.lines()
            .find(|l| l.split(':').next().is_some_and(|k| k.trim() == "Model"))
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().to_string())
            .filter(|m| !m.is_empty())
    })
}

/// The CPU's name: `/proc/cpuinfo` (x86), the board (ARM boards list no model name), or
/// `sysctl` (macOS).
pub fn cpu_model() -> Option<String> {
    if cfg!(target_os = "macos") {
        return run("sysctl", &["-n", "machdep.cpu.brand_string"])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
    }
    let info = std::fs::read_to_string("/proc/cpuinfo").ok().unwrap_or_default();
    info.lines()
        .find(|l| l.starts_with("model name"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_string())
        .or_else(board_model)
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).args(args).stdin(Stdio::null()).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).to_string())
}

/// ffmpeg's encoders (or filters): the names in the second column.
pub fn ffmpeg_list(ffmpeg: &str, what: &str) -> Vec<String> {
    run(ffmpeg, &["-hide_banner", what])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1).map(String::from))
        .collect()
}

/// Threads for one CPU transcode run: the cores but one, shared between the slots, 2 to 16.
pub fn cpu_threads(jobs: usize) -> usize {
    let cores = std::thread::available_parallelism().map_or(2, std::num::NonZero::get);
    (cores.saturating_sub(1) / jobs.max(1)).clamp(2, 16)
}

/// Find the backend. `requested`: auto, nvidia (nvenc), qsv, vaapi, cpu; `device`: a render node
/// (empty: the first one of a fitting vendor).
pub fn detect(ffmpeg: &str, requested: &str, device: &str, jobs: usize) -> Hw {
    let t0 = Instant::now();
    let requested = match requested.trim().to_ascii_lowercase().as_str() {
        "" => "auto".to_string(),
        "nvenc" => "nvidia".to_string(),
        "vt" => "videotoolbox".to_string(),
        "v4l2m2m" => "v4l2".to_string(),
        r => r.to_string(),
    };
    let here = Found {
        ffmpeg,
        device,
        encoders: ffmpeg_list(ffmpeg, "-encoders"),
        filters: ffmpeg_list(ffmpeg, "-filters"),
        nodes: render_nodes()
            .into_iter()
            .filter(|n| n.usable)
            .filter(|n| device.is_empty() || n.device == device)
            .collect(),
        threads: cpu_threads(jobs),
    };
    let mut notes = Vec::new();
    let threads = here.threads;

    let auto = platform_order();
    let order: Vec<&str> = match requested.as_str() {
        "auto" => auto.to_vec(),
        r @ ("nvidia" | "qsv" | "vaapi" | "videotoolbox" | "v4l2" | "cpu") => {
            let mut v = vec![r];
            v.extend(auto.iter().copied().filter(|x| *x != r));
            v
        }
        other => {
            notes.push(format!("Unknown transcode.hardware \"{other}\": automatic"));
            auto.to_vec()
        }
    };
    let mut found = None;
    for (i, b) in order.iter().enumerate() {
        let mut why = Vec::new();
        let hw = match *b {
            "nvidia" => here.try_nvidia(&mut why),
            "qsv" => here.try_qsv(&mut why),
            "vaapi" => here.try_vaapi(&mut why),
            "videotoolbox" => here.try_videotoolbox(&mut why),
            "v4l2" => here.try_v4l2(&mut why),
            _ => here.try_cpu(&mut why),
        };
        // What `auto` passed over is only worth saying when a GPU was there to be had.
        if i == 0 && requested != "auto" {
            notes.extend(why.iter().map(|w| format!("{w} (asked for {b})")));
        } else if hw.is_none() && why.iter().any(|w| w.contains("didn't work")) {
            notes.extend(why);
        }
        if let Some(h) = hw {
            found = Some(h);
            break;
        }
    }
    let mut hw = found.unwrap_or_else(|| {
        notes.push("Nothing to transcode with: titles stream as they are".into());
        Hw {
            available: false,
            ..Hw::cpu(threads)
        }
    });
    hw.requested = requested;
    hw_notes(&hw, &mut notes);
    hw.notes = notes;
    tracing::info!(backend = hw.backend.key(), device = ?hw.device, hevc10 = hw.hevc10, gpu_tonemap = hw.gpu_tonemap, ms = t0.elapsed().as_millis() as u64, notes = ?hw.notes, "transcoding with {}", hw.label());
    hw
}

/// The notes on what the backend chosen can't do (HDR, small pictures, boards).
fn hw_notes(hw: &Hw, notes: &mut Vec<String>) {
    if !hw.can_tonemap() {
        notes.push("HDR titles aren't transcoded (this ffmpeg can't tone-map: it has no zscale filter); they play as they are".into());
    } else if hw.backend != Backend::Nvenc && !hw.hevc10 {
        notes.push("HDR is tone-mapped to SDR when transcoding (no HEVC Main 10 encoder)".into());
    }
    if matches!(hw.backend, Backend::Qsv | Backend::Vaapi | Backend::VideoToolbox) && !hw.gpu_tonemap && hw.cpu_tonemap {
        notes.push("HDR is tone-mapped on the CPU".into());
    }
    if hw.min_width > 0 {
        notes.push(format!(
            "Pictures narrower than {} are encoded on the CPU (the hardware encoder doesn't take them)",
            hw.min_width
        ));
    }
    if hw.backend == Backend::V4l2 {
        notes.push("The V4L2 encoder makes H.264 up to 1080p; 4K and 10-bit HEVC titles aren't transcoded (they play as they are)".into());
    } else if hw.backend == Backend::Cpu && hw.vendor.is_some() {
        notes.push(
            "On this board's CPU, transcodes are made up to 720p; 4K and 10-bit HEVC titles aren't transcoded (they play as they are)"
                .into(),
        );
    }
}

/// What `detect` has to choose from: ffmpeg's encoders and filters, the render nodes passed in
/// (the one asked for, `device`, if any), and the threads for a CPU run.
struct Found<'a> {
    ffmpeg: &'a str,
    device: &'a str,
    encoders: Vec<String>,
    filters: Vec<String>,
    nodes: Vec<RenderNode>,
    threads: usize,
}

impl Found<'_> {
    fn has(&self, e: &str) -> bool {
        self.encoders.iter().any(|x| x == e)
    }

    fn has_filter(&self, f: &str) -> bool {
        self.filters.iter().any(|x| x == f)
    }

    fn try_nvidia(&self, notes: &mut Vec<String>) -> Option<Hw> {
        let gpu = Path::new("/dev/nvidiactl").exists() || !nvidia_gpus().is_empty();
        if !gpu {
            notes.push("No NVIDIA GPU is passed to the server".into());
            return None;
        }
        if !self.has("h264_nvenc") {
            notes.push("ffmpeg has no NVENC encoder".into());
            return None;
        }
        Some(Hw { threads: 3, ..Hw::nvenc() })
    }

    fn try_videotoolbox(&self, notes: &mut Vec<String>) -> Option<Hw> {
        if !cfg!(target_os = "macos") {
            notes.push("VideoToolbox is only on macOS".into());
            return None;
        }
        if !self.has("h264_videotoolbox") {
            notes.push("ffmpeg has no VideoToolbox encoder".into());
            return None;
        }
        if !self.has_filter("scale_vt") {
            notes.push("ffmpeg is too old for VideoToolbox (scale_vt needs ffmpeg 7 or newer)".into());
            return None;
        }
        let mut hw = Hw {
            cpu_tonemap: self.has_filter("zscale"),
            ..Hw::videotoolbox(self.threads)
        };
        if let Err(e) = probe(self.ffmpeg, &hw, Probe::H264) {
            notes.push(format!("VideoToolbox didn't work: {e}"));
            return None;
        }
        hw.hevc10 = self.has("hevc_videotoolbox") && probe(self.ffmpeg, &hw, Probe::Hevc10).is_ok();
        hw.gpu_tonemap = probe(self.ffmpeg, &hw, Probe::Tonemap).is_ok();
        // Intel Macs' encoder (Quick Sync) takes nothing narrower than 640.
        if probe(self.ffmpeg, &hw, Probe::Small).is_err() {
            hw.min_width = 640;
        }
        Some(hw)
    }

    fn try_v4l2(&self, notes: &mut Vec<String>) -> Option<Hw> {
        if !self.has("h264_v4l2m2m") {
            notes.push("ffmpeg has no V4L2 encoder".into());
            return None;
        }
        // Only worth a test run where there are video devices (passed in with `devices:`).
        let videos: Vec<std::path::PathBuf> = std::fs::read_dir("/dev")
            .map(|rd| {
                rd.filter_map(std::result::Result::ok)
                    .filter(|e| e.file_name().to_string_lossy().starts_with("video"))
                    .map(|e| e.path())
                    .collect()
            })
            .unwrap_or_default();
        if videos.is_empty() {
            notes.push("No V4L2 video device (/dev/video*) is passed to the server".into());
            return None;
        }
        let denied = |p: &std::path::PathBuf| matches!(std::fs::OpenOptions::new().read(true).write(true).open(p), Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied);
        if videos.iter().all(denied) {
            notes.push("The V4L2 encoder didn't work: /dev/video* can't be opened (permission denied: the server's user needs the video group; rootless Docker can't pass them in)".into());
            return None;
        }
        let hw = Hw {
            cpu_tonemap: self.has_filter("zscale"),
            ..Hw::v4l2(self.threads)
        };
        if let Err(e) = probe(self.ffmpeg, &hw, Probe::H264) {
            notes.push(format!("The V4L2 encoder didn't work: {e}"));
            return None;
        }
        Some(hw)
    }

    fn try_qsv(&self, notes: &mut Vec<String>) -> Option<Hw> {
        if !self.has("h264_qsv") {
            notes.push("ffmpeg has no Quick Sync encoder".into());
            return None;
        }
        let node = self
            .nodes
            .iter()
            .find(|n| n.vendor == "Intel" || (n.vendor == "other" && !self.device.is_empty()));
        let Some(node) = node else {
            notes.push("No Intel GPU (/dev/dri) is passed to the server".into());
            return None;
        };
        let mut hw = Hw {
            backend: Backend::Qsv,
            device: Some(node.device.clone()),
            vendor: Some("Intel".into()),
            hevc10: false,
            gpu_tonemap: false,
            cpu_tonemap: self.has_filter("zscale"),
            threads: self.threads,
            max_height: 2160,
            min_width: 0,
            requested: String::new(),
            notes: vec![],
            available: true,
        };
        if let Err(e) = probe(self.ffmpeg, &hw, Probe::H264) {
            notes.push(format!("Quick Sync on {} didn't work: {e}", node.device));
            return None;
        }
        hw.hevc10 = self.has("hevc_qsv") && probe(self.ffmpeg, &hw, Probe::Hevc10).is_ok();
        hw.gpu_tonemap = self.has_filter("vpp_qsv") && probe(self.ffmpeg, &hw, Probe::Tonemap).is_ok();
        Some(hw)
    }

    fn try_vaapi(&self, notes: &mut Vec<String>) -> Option<Hw> {
        if !self.has("h264_vaapi") {
            notes.push("ffmpeg has no VA-API encoder".into());
            return None;
        }
        let node = self
            .nodes
            .iter()
            .find(|n| n.vendor == "Intel" || n.vendor == "AMD")
            .or_else(|| self.nodes.first());
        let Some(node) = node else {
            notes.push("No Intel or AMD GPU (/dev/dri) is passed to the server".into());
            return None;
        };
        let vendor = if node.vendor == "other" { "GPU" } else { node.vendor };
        let mut hw = Hw {
            backend: Backend::Vaapi,
            device: Some(node.device.clone()),
            vendor: Some(vendor.into()),
            hevc10: false,
            gpu_tonemap: false,
            cpu_tonemap: self.has_filter("zscale"),
            threads: self.threads,
            max_height: 2160,
            min_width: 0,
            requested: String::new(),
            notes: vec![],
            available: true,
        };
        if let Err(e) = probe(self.ffmpeg, &hw, Probe::H264) {
            notes.push(format!("VA-API on {} didn't work: {e}", node.device));
            return None;
        }
        hw.hevc10 = self.has("hevc_vaapi") && probe(self.ffmpeg, &hw, Probe::Hevc10).is_ok();
        hw.gpu_tonemap = self.has_filter("tonemap_vaapi") && probe(self.ffmpeg, &hw, Probe::Tonemap).is_ok();
        Some(hw)
    }

    fn try_cpu(&self, notes: &mut Vec<String>) -> Option<Hw> {
        if !self.has("libx264") {
            notes.push("ffmpeg has no libx264".into());
            return None;
        }
        // On a single-board computer (a device-tree model) the board is named: such a CPU
        // leaves 4K and 10-bit HEVC alone (`too_heavy`).
        let board = board_model().filter(|_| cfg!(target_arch = "aarch64") || cfg!(target_arch = "arm"));
        // A board's CPU (a Raspberry Pi 4 makes 720p with x264's superfast preset in about real
        // time): 720p at most.
        let max_height = if board.is_some() { 720 } else { 1080 };
        Some(Hw {
            cpu_tonemap: self.has_filter("zscale"),
            vendor: board,
            max_height,
            ..Hw::cpu(self.threads)
        })
    }
}

#[derive(Clone, Copy, Debug)]
enum Probe {
    H264,
    Hevc10,
    Tonemap,
    /// A 320×180 picture (VideoToolbox).
    Small,
}

/// A 5-frame test encode through the backend's chain (`-f null`), at most 20 s.
fn probe(ffmpeg: &str, hw: &Hw, what: Probe) -> Result<(), String> {
    let mut a: Vec<String> = ["-nostdin", "-hide_banner", "-v", "error"].map(String::from).to_vec();
    a.extend(init_args(hw));
    a.extend(["-f", "lavfi", "-i", "testsrc2=size=640x360:rate=24"].map(String::from));
    let up = upload(hw);
    let (vf, enc): (String, &str) = match (hw.backend, what) {
        (Backend::Qsv, Probe::H264) => (format!("format=nv12,{up},scale_qsv=w=320:h=180:format=nv12"), "h264_qsv"),
        (Backend::Qsv, Probe::Hevc10) => (format!("format=p010le,{up},scale_qsv=w=320:h=180:format=p010le"), "hevc_qsv"),
        (Backend::Qsv, Probe::Tonemap) => (
            format!("format=p010le,setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc,{up},vpp_qsv=tonemap=1:format=nv12:w=320:h=180"),
            "h264_qsv",
        ),
        (Backend::Vaapi, Probe::H264) => (format!("format=nv12,{up},scale_vaapi=w=320:h=180:format=nv12"), "h264_vaapi"),
        (Backend::Vaapi, Probe::Hevc10) => (format!("format=p010le,{up},scale_vaapi=w=320:h=180:format=p010"), "hevc_vaapi"),
        (Backend::Vaapi, Probe::Tonemap) => (
            format!("format=p010le,setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc,{up},tonemap_vaapi=format=nv12:p=bt709:t=bt709:m=bt709"),
            "h264_vaapi",
        ),
        // 640×360 as it comes: Intel Macs' encoder takes nothing narrower than 640 (`Small`).
        (Backend::VideoToolbox, Probe::H264) => (format!("format=nv12,{up},scale_vt=w=640:h=360"), "h264_videotoolbox"),
        (Backend::VideoToolbox, Probe::Small) => (format!("format=nv12,{up},scale_vt=w=320:h=180"), "h264_videotoolbox"),
        (Backend::VideoToolbox, Probe::Hevc10) => (format!("format=p010le,{up},scale_vt=w=640:h=360"), "hevc_videotoolbox"),
        (Backend::VideoToolbox, Probe::Tonemap) => (
            format!("format=p010le,setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc,{up},scale_vt=w=640:h=360:{VT_TO_SDR}"),
            "h264_videotoolbox",
        ),
        // The encoder takes what its device takes: 640x360 as it comes.
        (Backend::V4l2, Probe::H264) => ("format=yuv420p".to_string(), "h264_v4l2m2m"),
        _ => return Ok(()),
    };
    a.extend(["-vf".into(), vf, "-frames:v".into(), "5".into(), "-c:v".into(), enc.into()]);
    if matches!(what, Probe::Hevc10) {
        a.extend(["-profile:v", "main10"].map(String::from));
    }
    a.extend(["-f", "null", "-"].map(String::from));
    let mut child = Command::new(ffmpeg)
        .args(&a)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let t0 = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) if st.success() => return Ok(()),
            Ok(Some(_)) => {
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    use std::io::Read;
                    let _ = e.read_to_string(&mut err);
                }
                tracing::debug!(?what, backend = hw.backend.key(), "probe failed: {err}");
                return Err(probe_error(&err));
            }
            Ok(None) if t0.elapsed() > Duration::from_secs(20) => {
                let _ = child.kill();
                return Err("timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// `scale_vt`'s options that convert HDR (PQ/HLG, BT.2020) to SDR BT.709: VideoToolbox's pixel
/// transfer tone-maps on the way.
pub const VT_TO_SDR: &str = "color_matrix=bt709:color_primaries=bt709:color_transfer=bt709";

/// The line of a failed test run that says why: the first one that isn't ffmpeg's generic
/// aftermath, without its "[context @ 0x…]" prefix.
fn probe_error(stderr: &str) -> String {
    const AFTERMATH: [&str; 6] = [
        "Nothing was written",
        "Terminating thread",
        "Task finished",
        "before EOF",
        "received EOF",
        "Conversion failed",
    ];
    let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let line = lines
        .iter()
        .find(|l| !AFTERMATH.iter().any(|a| l.contains(a)))
        .or(lines.last())
        .copied()
        .unwrap_or("failed");
    let line = match line.strip_prefix('[').and_then(|r| r.split_once("] ")) {
        Some((_, rest)) => rest,
        None => line,
    };
    crate::admin::logbuf::scrub(line).chars().take(160).collect()
}

/// The hardware device options before the inputs.
pub fn init_args(hw: &Hw) -> Vec<String> {
    let dev = hw.device.clone().unwrap_or_else(|| "/dev/dri/renderD128".into());
    match hw.backend {
        Backend::Nvenc => ["-init_hw_device", "cuda=cu", "-filter_hw_device", "cu"].map(String::from).to_vec(),
        Backend::Qsv => vec![
            "-init_hw_device".into(),
            format!("vaapi=va:{dev}"),
            "-init_hw_device".into(),
            "qsv=qs@va".into(),
            "-filter_hw_device".into(),
            "qs".into(),
        ],
        Backend::Vaapi => vec![
            "-init_hw_device".into(),
            format!("vaapi=va:{dev}"),
            "-filter_hw_device".into(),
            "va".into(),
        ],
        Backend::VideoToolbox => ["-init_hw_device", "videotoolbox=vt", "-filter_hw_device", "vt"]
            .map(String::from)
            .to_vec(),
        Backend::V4l2 | Backend::Cpu => vec![],
    }
}

/// Uploads CPU frames to the backend's device.
pub fn upload(hw: &Hw) -> &'static str {
    match hw.backend {
        Backend::Nvenc => "hwupload_cuda",
        Backend::Qsv => "hwupload=extra_hw_frames=64",
        Backend::Vaapi | Backend::VideoToolbox | Backend::V4l2 | Backend::Cpu => "hwupload",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_args() {
        let mut hw = Hw::nvenc();
        assert_eq!(init_args(&hw).join(" "), "-init_hw_device cuda=cu -filter_hw_device cu");
        hw.backend = Backend::Qsv;
        hw.device = Some("/dev/dri/renderD129".into());
        assert_eq!(
            init_args(&hw).join(" "),
            "-init_hw_device vaapi=va:/dev/dri/renderD129 -init_hw_device qsv=qs@va -filter_hw_device qs"
        );
        hw.backend = Backend::Vaapi;
        assert_eq!(
            init_args(&hw).join(" "),
            "-init_hw_device vaapi=va:/dev/dri/renderD129 -filter_hw_device va"
        );
        assert!(init_args(&Hw::cpu(4)).is_empty());
        assert_eq!(
            init_args(&Hw::videotoolbox(4)).join(" "),
            "-init_hw_device videotoolbox=vt -filter_hw_device vt"
        );
        assert!(init_args(&Hw::v4l2(3)).is_empty());
        assert!(cpu_threads(1) >= 2 && cpu_threads(64) == 2);
    }

    #[test]
    fn falls_back_to_the_cpu() {
        // No GPU in a test environment, and no such ffmpeg: nothing found, CPU-shaped answer.
        let hw = detect("/nonexistent/ffmpeg", "nvidia", "", 2);
        assert_eq!(hw.backend, Backend::Cpu);
        assert!(!hw.available);
        assert_eq!(hw.requested, "nvidia");
        assert!(hw.notes.iter().any(|n| n.contains("asked for nvidia")), "{:?}", hw.notes);
        assert_eq!(hw.describe()["backend"], "cpu");
        // VideoToolbox asked for: only on macOS.
        let hw = detect("/nonexistent/ffmpeg", "vt", "", 2);
        assert_eq!(hw.requested, "videotoolbox");
        assert!(hw.notes.iter().any(|n| n.contains("asked for videotoolbox")), "{:?}", hw.notes);
    }

    #[test]
    fn probe_errors_say_why() {
        let err = "[h264_v4l2m2m @ 0x55a39bbb30] Could not find a valid device\n[h264_v4l2m2m @ 0x55a39bbb30] can't configure encoder\n[vost#0:0/h264_v4l2m2m @ 0x5] Task finished with error code: -22 (Invalid argument)\n[out#0/null @ 0x5] Nothing was written into output file\n";
        assert_eq!(probe_error(err), "Could not find a valid device");
        assert_eq!(
            probe_error("[out#0/null @ 0x5] Nothing was written into output file\n"),
            "Nothing was written into output file"
        );
        assert_eq!(probe_error(""), "failed");
    }

    #[test]
    fn choices_fit_the_platform() {
        let keys: Vec<&str> = choices().iter().map(|c| c.0).collect();
        assert_eq!(keys.first(), Some(&"auto"));
        assert_eq!(keys.last(), Some(&"cpu"));
        assert_eq!(keys.contains(&"videotoolbox"), cfg!(target_os = "macos"));
        assert_eq!(keys.contains(&"nvidia"), !cfg!(target_os = "macos"));
        assert_eq!(Hw::nvenc().describe()["choices"][0]["key"], "auto");
    }
}
