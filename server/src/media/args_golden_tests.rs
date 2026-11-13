//! Golden test for the transcode command lines (`video_args_with`): every backend and its
//! switches, against sources of each kind, for every variant of their ladders, at the first
//! segment, a later one, and as a whole file.

use super::{
    hwaccel::{Backend, Hw},
    transcode::{ladder_with, tests::media, tests::video, video_args_with},
    VideoTrack,
};
use std::fmt::Write as _;

fn backends() -> Vec<(&'static str, Hw)> {
    let mut out = Vec::new();
    out.push(("nvenc", Hw::nvenc()));
    let gpu = |backend: Backend, vendor: &str| Hw {
        backend,
        device: Some("/dev/dri/renderD128".into()),
        vendor: Some(vendor.into()),
        hevc10: false,
        gpu_tonemap: false,
        threads: 4,
        ..Hw::cpu(4)
    };
    out.push(("qsv", gpu(Backend::Qsv, "Intel")));
    out.push((
        "qsv_hevc10_gpu_tonemap",
        Hw {
            hevc10: true,
            gpu_tonemap: true,
            max_height: 2160,
            ..gpu(Backend::Qsv, "Intel")
        },
    ));
    out.push(("vaapi_amd", gpu(Backend::Vaapi, "AMD")));
    out.push((
        "vaapi_hevc10_gpu_tonemap",
        Hw {
            hevc10: true,
            gpu_tonemap: true,
            max_height: 2160,
            ..gpu(Backend::Vaapi, "Intel")
        },
    ));
    let vt = Hw {
        vendor: Some("Apple".into()),
        ..Hw::videotoolbox(6)
    };
    out.push(("videotoolbox", vt.clone()));
    out.push((
        "videotoolbox_intel_cpu_tonemap",
        Hw {
            vendor: Some("Intel".into()),
            gpu_tonemap: false,
            hevc10: false,
            min_width: 640,
            ..vt
        },
    ));
    out.push((
        "v4l2",
        Hw {
            backend: Backend::V4l2,
            vendor: Some("Raspberry Pi 4 Model B Rev 1.4".into()),
            ..Hw::cpu(4)
        },
    ));
    out.push(("cpu", Hw::cpu(7)));
    out.push((
        "cpu_board",
        Hw {
            vendor: Some("Raspberry Pi 5 Model B".into()),
            max_height: 720,
            ..Hw::cpu(3)
        },
    ));
    out.push((
        "cpu_no_tonemap",
        Hw {
            cpu_tonemap: false,
            ..Hw::cpu(2)
        },
    ));
    out
}

fn sources() -> Vec<(&'static str, VideoTrack)> {
    let mut dvd = video("mpeg2video", 720, 576, 6_000_000, "SDR");
    dvd.dar = 16.0 / 9.0;
    let mut hlg = video("hevc", 3840, 2160, 50_000_000, "HLG");
    hlg.frame_rate = Some(50.0);
    let mut av1 = video("av1", 2560, 1440, 9_000_000, "SDR");
    av1.frame_rate = None;
    let mut h264_10 = video("h264", 1920, 1080, 20_000_000, "SDR");
    h264_10.pix_fmt = "yuv420p10le".into();
    h264_10.copy_reason = Some("10-bit".into());
    vec![
        ("h264_1080p", video("h264", 1920, 1080, 12_000_000, "SDR")),
        ("h264_10bit", h264_10),
        ("hevc_4k_pq", video("hevc", 3840, 2160, 60_000_000, "PQ")),
        ("hevc_4k_hlg", hlg),
        ("hevc_1080p_sdr", video("hevc", 1920, 1080, 8_000_000, "SDR")),
        ("divx", video("mpeg4", 656, 368, 1_200_000, "SDR")),
        ("dvd", dvd),
        ("vc1", video("vc1", 1920, 1080, 15_000_000, "SDR")),
        ("av1", av1),
    ]
}

#[test]
fn video_args_golden() {
    let mut out = String::new();
    for (bname, hw) in backends() {
        let _ = writeln!(out, "# backend {bname}");
        for (sname, v) in sources() {
            let m = media(v.clone());
            // An SDR source's ladder doesn't depend on the screen.
            let displays: &[bool] = if v.hdr() { &[true, false] } else { &[true] };
            for &hdr_display in displays {
                let l = ladder_with(&hw, &v, hdr_display, true);
                let ids: Vec<&str> = l.iter().map(|x| x.id.as_str()).collect();
                let _ = writeln!(out, "## {sname} hdr_display={hdr_display}: {ids:?}");
                // Every variant at a later segment; the first also at the first segment and
                // as a whole file.
                for (i, variant) in l.iter().filter(|x| x.transcode).enumerate() {
                    let runs: &[(usize, bool)] = if i == 0 {
                        &[(2, false), (0, false), (0, true)]
                    } else {
                        &[(2, false)]
                    };
                    for &(n, progressive) in runs {
                        let a = match video_args_with(&hw, &m, variant, n, progressive) {
                            Ok(a) => a.join(" "),
                            Err(e) => format!("Err({e:#})"),
                        };
                        let _ = writeln!(out, "{} n={n} progressive={progressive}: {a}", variant.id);
                    }
                }
            }
        }
    }
    crate::golden::check("transcode/video_args", &out);
}
