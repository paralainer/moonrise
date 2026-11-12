//! Golden test for `hwaccel::detect`: the backend chosen and the notes given, for what is asked
//! for and what a stand-in ffmpeg lists (encoders, filters). Only on a host with no GPU or video
//! devices (the dev container): what the devices are decides the answer too.

use std::fmt::Write as _;
use std::path::Path;

use super::hwaccel::{detect, render_nodes, Hw};

/// A host with nothing to transcode on but its CPU (what the golden file describes).
fn bare_host() -> bool {
    let videos = std::fs::read_dir("/dev").is_ok_and(|rd| {
        rd.filter_map(std::result::Result::ok)
            .any(|e| e.file_name().to_string_lossy().starts_with("video"))
    });
    !cfg!(target_os = "macos")
        && !cfg!(target_arch = "aarch64")
        && !Path::new("/dev/nvidiactl").exists()
        && std::process::Command::new("nvidia-smi").output().is_err()
        && !render_nodes().iter().any(|n| n.usable)
        && !Path::new("/dev/dri").exists()
        && !videos
}

/// A stand-in ffmpeg: `-encoders` and `-filters` list the given names; anything else succeeds.
///
/// Another process writes the executable (`install`). If this one did, a child started at that
/// moment by a test on another thread would inherit the file open for writing, and until that
/// child's own exec, running the file here fails with "Text file busy": `detect` then sees an
/// ffmpeg with no encoders.
fn fake_ffmpeg(dir: &Path, encoders: &[&str], filters: &[&str]) -> String {
    let list = |names: &[&str]| names.iter().map(|n| format!(" V....D {n}  An encoder\n")).collect::<String>();
    std::fs::write(dir.join("encoders.txt"), format!("Encoders:\n ------\n{}", list(encoders))).unwrap();
    std::fs::write(dir.join("filters.txt"), format!("Filters:\n{}", list(filters))).unwrap();
    let script = dir.join("ffmpeg.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$2\" in\n  -encoders) cat '{d}/encoders.txt' ;;\n  -filters) cat '{d}/filters.txt' ;;\nesac\nexit 0\n",
            d = dir.display()
        ),
    )
    .unwrap();
    let p = dir.join("ffmpeg");
    let status = std::process::Command::new("install")
        .args(["-m", "755"])
        .arg(&script)
        .arg(&p)
        .status()
        .unwrap();
    assert!(status.success(), "install: {status}");
    p.to_string_lossy().into_owned()
}

fn show(hw: &Hw) -> String {
    let mut h = hw.clone();
    // Depends on the host's cores.
    h.threads = 0;
    format!("{h:?}")
}

#[test]
fn detect_golden() {
    if !bare_host() {
        eprintln!("detect_golden: skipped (this host has GPU or video devices; run it in the dev container)");
        return;
    }
    let dir = std::env::temp_dir().join(format!("moonrise-detect-golden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let all = [
        "libx264",
        "h264_nvenc",
        "hevc_nvenc",
        "h264_qsv",
        "hevc_qsv",
        "h264_vaapi",
        "hevc_vaapi",
        "h264_videotoolbox",
        "hevc_videotoolbox",
        "h264_v4l2m2m",
    ];
    let encoder_sets: [(&str, &[&str]); 4] = [
        ("all", &all),
        ("x264_only", &["libx264"]),
        ("gpu_only", &["h264_nvenc", "h264_qsv", "h264_vaapi", "h264_v4l2m2m"]),
        ("none", &[]),
    ];
    let filter_sets: [(&str, &[&str]); 2] = [("zscale", &["zscale", "scale_vt", "vpp_qsv", "tonemap_vaapi"]), ("no_zscale", &[])];
    let mut out = String::new();
    for (ename, encoders) in encoder_sets {
        for (fname, filters) in filter_sets {
            let d = dir.join(format!("{ename}-{fname}"));
            std::fs::create_dir_all(&d).unwrap();
            let ffmpeg = fake_ffmpeg(&d, encoders, filters);
            for requested in [
                "",
                "auto",
                " NVENC ",
                "nvidia",
                "qsv",
                "vaapi",
                "vt",
                "videotoolbox",
                "v4l2m2m",
                "cpu",
                "bogus",
            ] {
                for device in ["", "/dev/dri/renderD129"] {
                    let hw = detect(&ffmpeg, requested, device, 2);
                    let _ = writeln!(out, "{ename} {fname} requested={requested:?} device={device:?}: {}", show(&hw));
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    crate::golden::check("hwaccel/detect", &out);
}
