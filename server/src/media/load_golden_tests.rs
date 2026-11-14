//! Golden tests for `Media::load`: what it makes of representative files (containers, codecs,
//! track lists, cache states, failures), written out in full (`Debug`, skipped fields included).
//! No ffmpeg is needed: probes come from the probe cache or a stand-in ffprobe script.

use std::fmt::Write as _;
use std::{collections::HashMap, path::Path, sync::Arc};

use serde_json::{json, Value};

use super::{item, LoadCtx, Media};
use crate::{admin::config::Config, db::Db, media::scan::Scanner};

const SIZE: u64 = 1_000_000;
const MTIME: i64 = 1_700_000_000_000_000_000;

struct Case {
    name: &'static str,
    /// Environment-style settings (`MOONRISE_DISABLE`).
    env: &'static [(&'static str, &'static str)],
    item: Value,
    /// The file's probe, cached (None: not cached).
    probe: Option<Value>,
    /// Cached keyframes (with a cached probe).
    keyframes: Option<Vec<f64>>,
    /// The file's bytes, written to the path (None: no file on disk).
    file: Option<&'static [u8]>,
    /// The library knows the file (files row, role main).
    library_file: bool,
    /// The library's (size, mtime) stamp is given with the item.
    stamp: bool,
    /// Databases given to the load (probe cache, library).
    dbs: bool,
}

fn v_stream(index: i64, codec: &str, extra: Value) -> Value {
    let mut s = json!({"index": index, "codec_type": "video", "codec_name": codec, "disposition": {"attached_pic": 0}});
    merge(&mut s, extra);
    s
}

fn a_stream(index: i64, codec: &str, extra: Value) -> Value {
    let mut s = json!({"index": index, "codec_type": "audio", "codec_name": codec, "channels": 2, "sample_rate": "48000",
        "time_base": "1/48000", "disposition": {"attached_pic": 0}});
    merge(&mut s, extra);
    s
}

fn s_stream(index: i64, codec: &str) -> Value {
    json!({"index": index, "codec_type": "subtitle", "codec_name": codec, "disposition": {"attached_pic": 0}})
}

fn merge(a: &mut Value, b: Value) {
    if let (Some(a), Value::Object(b)) = (a.as_object_mut(), b) {
        a.extend(b);
    }
}

fn probe_of(format: &str, duration: &str, streams: Vec<Value>) -> Value {
    json!({"format": {"format_name": format, "duration": duration, "size": "999"}, "streams": streams})
}

fn every(step: f64, end: f64) -> Vec<f64> {
    let mut v = Vec::new();
    let mut t = 0.0;
    while t < end {
        v.push(t);
        t += step;
    }
    v
}

fn item_of(id: &str, file: &str, ticks: Option<i64>, streams: Value) -> Value {
    json!({"Id": id, "Name": "A Title", "Type": "Movie", "MediaSources": [
        {"Id": format!("{id}-ms"), "Path": file, "RunTimeTicks": ticks, "Bitrate": 8_000_000, "MediaStreams": streams}
    ]})
}

fn cases() -> Vec<Case> {
    let mp4_streams = json!([
        {"Index": 0, "Type": "Video", "Codec": "h264"},
        {"Index": 1, "Type": "Audio", "Codec": "aac", "Language": "eng", "Title": "Stereo", "IsDefault": true, "Channels": 2},
        {"Index": 2, "Type": "Audio", "Codec": "ac3", "Language": "rus", "DisplayTitle": "Russian AC3 5.1", "Channels": 6},
        {"Index": 3, "Type": "Audio", "Codec": "aac", "Language": "fre", "Title": "Stereo", "Channels": 2},
        {"Index": 4, "Type": "Subtitle", "Codec": "mov_text", "Language": "eng", "DisplayTitle": "English", "IsTextSubtitleStream": true, "IsForced": true}
    ]);
    let mp4_probe = probe_of(
        "mov,mp4,m4a,3gp,3g2,mj2",
        "61.5",
        vec![
            v_stream(
                0,
                "h264",
                json!({"profile": "High", "level": 41, "pix_fmt": "yuv420p", "width": 1920, "height": 800,
                "avg_frame_rate": "24000/1001", "sample_aspect_ratio": "1:1", "has_b_frames": 2, "nal_length_size": "4"}),
            ),
            a_stream(1, "aac", json!({"profile": "LC"})),
            a_stream(2, "ac3", json!({"channels": 6})),
            a_stream(
                3,
                "aac",
                json!({"profile": "HE-AAC", "time_base": "1/44100", "sample_rate": "44100"}),
            ),
            s_stream(4, "mov_text"),
        ],
    );
    let mkv_streams = json!([
        {"Index": 0, "Type": "Subtitle", "Codec": "srt", "Language": "eng", "IsExternal": true, "IsTextSubtitleStream": true, "Path": "/media/films/A Title.en.srt", "DisplayTitle": "English"},
        {"Index": 1, "Type": "Subtitle", "Codec": "pgssub", "IsExternal": true, "Path": "/media/films/A Title.sup"},
        {"Index": 2, "Type": "Video", "Codec": "hevc", "VideoRangeType": "DOVIWithHDR10"},
        {"Index": 3, "Type": "Audio", "Codec": "truehd", "Language": "eng", "Title": "", "DisplayTitle": "TrueHD Atmos 7.1", "Channels": 8},
        {"Index": 4, "Type": "Audio", "Codec": "dts", "Language": "ger", "DisplayTitle": "DTS-HD MA 5.1", "Channels": 6, "IsDefault": true},
        {"Index": 5, "Type": "Audio", "Codec": "flac", "Language": "jpn", "DisplayTitle": "FLAC 2.0"},
        {"Index": 6, "Type": "Audio", "Codec": "eac3", "Language": "xyz", "DisplayTitle": "FLAC 2.0"},
        {"Index": 7, "Type": "Subtitle", "Codec": "subrip", "Language": "eng", "DisplayTitle": "English", "IsTextSubtitleStream": true, "IsDefault": true},
        {"Index": 8, "Type": "Subtitle", "Codec": "PGSSUB", "Language": "eng", "DisplayTitle": "English"},
        {"Index": 9, "Type": "Subtitle", "Codec": "ass", "Language": "ukr", "Title": "Signs", "IsTextSubtitleStream": true},
        {"Index": 10, "Type": "Subtitle", "Codec": "subrip", "Language": "eng", "DisplayTitle": "English", "IsTextSubtitleStream": true},
        {"Index": 11, "Type": "Attachment", "Codec": "ttf"}
    ]);
    let mkv_probe = probe_of(
        "matroska,webm",
        "322.25",
        vec![
            v_stream(
                0,
                "hevc",
                json!({"profile": "Main 10", "level": 153, "pix_fmt": "yuv420p10le", "width": 3840, "height": 1608,
                "r_frame_rate": "24000/1001", "has_b_frames": 2,
                "side_data_list": [{"side_data_type": "DOVI configuration record", "dv_profile": 8, "dv_level": 6,
                    "rpu_present_flag": 1, "el_present_flag": 0, "bl_present_flag": 1, "dv_bl_signal_compatibility_id": 1}]}),
            ),
            a_stream(1, "truehd", json!({"channels": 8})),
            a_stream(2, "dts", json!({"profile": "DTS-HD MA", "channels": 6})),
            a_stream(3, "flac", json!({"sample_rate": "96000", "time_base": "1/1000"})),
            a_stream(4, "eac3", json!({"channels": 6})),
            s_stream(5, "subrip"),
            s_stream(6, "hdmv_pgs_subtitle"),
            s_stream(7, "ass"),
            s_stream(8, "subrip"),
            json!({"index": 9, "codec_type": "attachment", "codec_name": "ttf"}),
        ],
    );
    let avi_streams = json!([
        {"Index": 0, "Type": "Video", "Codec": "mpeg4"},
        {"Index": 1, "Type": "Audio", "Codec": "mp3", "Language": "rus"}
    ]);
    let avi_probe = probe_of(
        "avi",
        "5400.0",
        vec![
            v_stream(
                0,
                "mpeg4",
                json!({"profile": "Advanced Simple Profile", "pix_fmt": "yuv420p", "width": 720, "height": 304,
                "avg_frame_rate": "25/1", "sample_aspect_ratio": "0:1", "has_b_frames": 1}),
            ),
            a_stream(1, "mp3", json!({"sample_rate": "44100", "time_base": "1/56000"})),
        ],
    );
    let vc1_streams = json!([
        {"Index": 0, "Type": "Video", "Codec": "vc1", "VideoRangeType": "SDR"},
        {"Index": 1, "Type": "Audio", "Codec": "dts", "Language": "eng"},
        {"Index": 2, "Type": "Audio", "Codec": "aac", "Language": "eng"}
    ]);
    let vc1_probe = probe_of(
        "matroska,webm",
        "100.0",
        vec![
            v_stream(
                0,
                "vc1",
                json!({"width": 1920, "height": 1080, "avg_frame_rate": "0/0", "r_frame_rate": "30000/1001"}),
            ),
            a_stream(1, "dts", json!({"profile": "DTS", "channels": 6})),
            a_stream(2, "opus", json!({})),
        ],
    );
    let dovi5_streams = json!([
        {"Index": 0, "Type": "Video", "Codec": "hevc", "VideoRangeType": "DOVI"},
        {"Index": 1, "Type": "Audio", "Codec": "eac3", "Language": "eng"}
    ]);
    let dovi5_probe = probe_of(
        "mov,mp4,m4a,3gp,3g2,mj2",
        "30.0",
        vec![
            v_stream(
                0,
                "hevc",
                json!({"profile": "Main 10", "level": 150, "pix_fmt": "yuv420p10le", "width": 3840, "height": 2160,
                "avg_frame_rate": "24/1", "codec_tag_string": "dvh1",
                "side_data_list": [{"dv_profile": 5, "dv_level": 6, "rpu_present_flag": 1, "el_present_flag": 0,
                    "bl_present_flag": 1, "dv_bl_signal_compatibility_id": 0}]}),
            ),
            a_stream(1, "eac3", json!({"channels": 6})),
        ],
    );
    let audio_only = json!([
        {"Index": 0, "Type": "Audio", "Codec": "flac", "Language": "eng", "IsDefault": false},
        {"Index": 1, "Type": "EmbeddedImage", "Codec": "mjpeg"}
    ]);
    let audio_only_probe = probe_of(
        "flac",
        "200.0",
        vec![
            a_stream(0, "flac", json!({"channels": 2})),
            json!({"index": 1, "codec_type": "video", "codec_name": "mjpeg", "disposition": {"attached_pic": 1}}),
        ],
    );
    // The library's numbers don't fit ffprobe's by any offset: paired up in order, kind by kind.
    let by_order = json!([
        {"Index": 5, "Type": "Video", "Codec": "h264"},
        {"Index": 6, "Type": "Audio", "Codec": "aac", "Language": "eng"},
        {"Index": 9, "Type": "Audio", "Codec": "ac3", "Language": "spa"}
    ]);
    let by_order_probe = probe_of(
        "mpegts",
        "50.0",
        vec![
            a_stream(0, "ac3", json!({})),
            v_stream(
                1,
                "h264",
                json!({"profile": "Main", "level": 40, "pix_fmt": "yuv420p", "width": 1280, "height": 720, "avg_frame_rate": "50/1"}),
            ),
            a_stream(2, "aac", json!({"profile": "HE-AACv2"})),
        ],
    );
    let h264_10 = json!([
        {"Index": 0, "Type": "Video", "Codec": "h264", "VideoRangeType": "HLG"},
        {"Index": 1, "Type": "Audio", "Codec": "aac"}
    ]);
    let h264_10_probe = probe_of(
        "matroska,webm",
        "20.0",
        vec![
            v_stream(
                0,
                "h264",
                json!({"profile": "High 10", "level": 51, "pix_fmt": "yuv420p10le", "width": 1920, "height": 1080,
                "avg_frame_rate": "25/1", "nal_length_size": "3"}),
            ),
            a_stream(1, "aac", json!({})),
        ],
    );

    let c = |name, env, item, probe, keyframes, file, library_file, stamp, dbs| Case {
        name,
        env,
        item,
        probe,
        keyframes,
        file,
        library_file,
        stamp,
        dbs,
    };
    vec![
        c(
            "mp4_copy",
            &[],
            item_of("i1", "@/a.mp4", Some(615_000_000), mp4_streams.clone()),
            Some(mp4_probe.clone()),
            Some(every(2.0, 61.0)),
            None,
            true,
            true,
            true,
        ),
        c(
            "mkv_convert",
            &[],
            item_of("i2", "@/b.mkv", None, mkv_streams.clone()),
            Some(mkv_probe.clone()),
            Some(every(4.17, 322.0)),
            None,
            true,
            true,
            true,
        ),
        c(
            "mkv_convert_off",
            &[("MOONRISE_DISABLE", "convert")],
            item_of("i2", "@/b.mkv", None, mkv_streams),
            Some(mkv_probe.clone()),
            Some(every(4.17, 322.0)),
            None,
            false,
            true,
            true,
        ),
        c(
            "avi_pending",
            &[],
            item_of("i3", "@/c.avi", Some(54_000_000_000), avi_streams.clone()),
            Some(avi_probe.clone()),
            None,
            Some(b"RIFF"),
            true,
            true,
            true,
        ),
        c(
            "avi_keyframes",
            &[],
            item_of("i3", "@/c.avi", Some(54_000_000_000), avi_streams.clone()),
            Some(avi_probe.clone()),
            Some(vec![0.0, 0.5, 7.0, 13.04, 20.0, 5399.5]),
            None,
            true,
            true,
            true,
        ),
        c(
            "avi_scan_index_off",
            &[("MOONRISE_DISABLE", "scan_index")],
            item_of("i3", "@/c.avi", None, avi_streams),
            Some(avi_probe),
            None,
            Some(b"RIFF"),
            true,
            true,
            true,
        ),
        c(
            "mkv_damaged",
            &[],
            item_of(
                "i4",
                "@/d.mkv",
                None,
                json!([{"Index": 0, "Type": "Video", "Codec": "hevc"}, {"Index": 1, "Type": "Audio", "Codec": "eac3"}]),
            ),
            Some(probe_of(
                "matroska,webm",
                "600.0",
                vec![
                    v_stream(0, "hevc", json!({"profile": "Main", "level": 120, "pix_fmt": "yuv420p"})),
                    a_stream(1, "eac3", json!({})),
                ],
            )),
            None,
            Some(&[
                0x1A, 0x45, 0xDF, 0xA3, 0x84, 0x42, 0x86, 0x81, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ]),
            true,
            true,
            true,
        ),
        c(
            "mkv_unreadable",
            &[],
            item_of("i4", "@/missing.mkv", None, json!([{"Index": 0, "Type": "Video", "Codec": "hevc"}])),
            Some(probe_of(
                "matroska,webm",
                "600.0",
                vec![v_stream(0, "hevc", json!({"profile": "Main", "level": 120, "pix_fmt": "yuv420p"}))],
            )),
            None,
            None,
            false,
            true,
            true,
        ),
        // A screen recording with no audio track at all plays as video alone.
        c(
            "mp4_no_audio",
            &[],
            item_of(
                "i15",
                "@/screen.mp4",
                Some(1_500_000_000),
                json!([{"Index": 0, "Type": "Video", "Codec": "h264"}]),
            ),
            Some(probe_of(
                "mov,mp4,m4a,3gp,3g2,mj2",
                "150.0",
                vec![v_stream(
                    0,
                    "h264",
                    json!({"profile": "High", "level": 40, "pix_fmt": "yuv420p", "width": 1920, "height": 1080,
                    "avg_frame_rate": "30/1", "has_b_frames": 2, "nal_length_size": "4"}),
                )],
            )),
            Some(every(2.0, 150.0)),
            None,
            true,
            true,
            true,
        ),
        c(
            "mp4_without_moov",
            &[],
            item_of("i5", "@/e.mp4", None, mp4_streams.clone()),
            Some(mp4_probe.clone()),
            None,
            Some(b"\0\0\0\x08free"),
            false,
            true,
            true,
        ),
        c(
            "mp4_index_off",
            &[("MOONRISE_DISABLE", "mp4_index,scan_index")],
            item_of("i5", "@/e.mp4", None, mp4_streams.clone()),
            Some(mp4_probe.clone()),
            None,
            Some(b"\0\0\0\x08free"),
            false,
            true,
            true,
        ),
        c(
            "vc1_transcode",
            &[],
            item_of("i6", "@/f.mkv", Some(1_000_000_000), vc1_streams.clone()),
            Some(vc1_probe.clone()),
            Some(every(1.0, 100.0)),
            None,
            false,
            true,
            true,
        ),
        c(
            "vc1_no_transcode_no_convert",
            &[("MOONRISE_DISABLE", "transcode,convert")],
            item_of("i6", "@/f.mkv", None, vc1_streams),
            Some(vc1_probe),
            Some(every(1.0, 100.0)),
            None,
            false,
            true,
            true,
        ),
        c(
            "dovi_profile_5",
            &[],
            item_of("i7", "@/g.mp4", None, dovi5_streams),
            Some(dovi5_probe),
            Some(every(1.0, 30.0)),
            None,
            false,
            true,
            true,
        ),
        c(
            "audio_only",
            &[],
            item_of("i8", "@/h.flac", None, audio_only),
            Some(audio_only_probe),
            None,
            None,
            false,
            true,
            true,
        ),
        c(
            "streams_by_order",
            &[],
            item_of("i9", "@/i.ts", None, by_order),
            Some(by_order_probe),
            Some(vec![0.0, 3.0, 6.5, 12.0, 49.5]),
            None,
            false,
            true,
            true,
        ),
        c(
            "h264_10bit_hlg",
            &[],
            item_of("i10", "@/j.mkv", None, h264_10),
            Some(h264_10_probe),
            Some(vec![0.0, 6.0, 12.0, 18.0, 19.5]),
            None,
            false,
            true,
            true,
        ),
        // Not cached: the stand-in ffprobe answers (`probe_cache` miss, then stored).
        c(
            "probe_miss_by_stat",
            &[],
            item_of("i11", "@/a.mp4", None, mp4_streams.clone()),
            None,
            None,
            Some(b"0123456789"),
            false,
            false,
            true,
        ),
        c(
            "probe_cache_off",
            &[],
            item_of("i11", "@/a.mp4", Some(1), mp4_streams.clone()),
            None,
            None,
            Some(b"0123456789"),
            false,
            true,
            false,
        ),
        c(
            "not_a_file",
            &[],
            item_of("i12", "@", None, json!([])),
            None,
            None,
            None,
            false,
            false,
            true,
        ),
        c(
            "no_such_file",
            &[],
            item_of("i12", "@/nope.mkv", None, json!([])),
            None,
            None,
            None,
            false,
            false,
            true,
        ),
        c(
            "no_path",
            &[],
            json!({"Id": "i13", "MediaSources": [{"Id": "m", "MediaStreams": []}]}),
            None,
            None,
            None,
            false,
            false,
            true,
        ),
        c("no_media_source", &[], json!({"Id": "i14"}), None, None, None, false, false, true),
    ]
}

/// A stand-in ffprobe: prints `probe.json` from its folder.
fn fake_ffprobe(dir: &Path, probe: &Value) -> String {
    std::fs::write(dir.join("probe.json"), probe.to_string()).unwrap();
    let p = dir.join("ffprobe");
    crate::testutil::write_executable(&p, &format!("#!/bin/sh\ncat '{}'\n", dir.join("probe.json").display()));
    p.to_string_lossy().into_owned()
}

async fn run_case(case: &Case, root: &Path) -> String {
    let (mut result, db, d) = load_case(case, root).await;
    if let Ok(m) = &mut result {
        as_if_in_golden_root(m, &d, &format!("{GOLDEN_ROOT}/{}", case.name)).await;
    }
    describe(result, db, d).await
}

/// The folder the goldens were made in. The file's path is part of the hashes `load` makes
/// (`version`, `file_key`), but each run uses a folder of its own: two runs at once (two
/// checkouts, two containers) must not share one.
const GOLDEN_ROOT: &str = "/tmp/moonrise-load-golden";

/// Makes `m`'s hashes again as if its file were in `golden` instead of `d`: the same functions
/// `load` used, given the same inputs, with only the folder changed. A load that made them some
/// other way fails here.
async fn as_if_in_golden_root(m: &mut Media, d: &str, golden: &str) {
    use super::{scan::Stamp, Container};
    let item = m.item.clone();
    let ms = &item.media_sources[0];
    let mut there = ms.clone();
    there.path = ms.path.as_ref().map(|p| p.replace(d, golden));

    let flags = [Container::Avi, Container::Other]
        .into_iter()
        .flat_map(|c| [(c, false, false), (c, true, false), (c, false, true), (c, true, true)]);
    let (container, late, replanned) = flags
        .clone()
        .find(|&(c, l, r)| Media::version_of(ms, c, l, r) == m.version)
        .unwrap_or_else(|| panic!("version {} wasn't made by version_of", m.version));
    let version = Media::version_of(&there, container, late, replanned);

    // The stamp `load` saw: the library's, else the file's own.
    let stamp = match (&ms.path, ms.stamp) {
        (Some(p), Some((size, mtime))) => Some((p, size, mtime)),
        (Some(p), None) => crate::storage::vfs::stat(p).await.ok().map(|s| (p, s.size, s.mtime)),
        _ => None,
    };
    let key = |path: &str, size, mtime, ms, version: &str| {
        let s = Stamp {
            path: path.to_string(),
            size,
            mtime,
        };
        Media::file_key_of(Some(&s), &item, ms, version)
    };
    m.file_key = match stamp {
        Some((p, size, mtime)) if m.file_key == key(p, size, mtime, ms, &m.version) => {
            key(there.path.as_deref().unwrap(), size, mtime, &there, &version)
        }
        _ if m.file_key == Media::file_key_of(None, &item, ms, &m.version) => Media::file_key_of(None, &item, &there, &version),
        _ => panic!("file_key {} wasn't made by file_key_of", m.file_key),
    };
    m.version = version;
}

/// Loads a case: the media, the database it used, and the case's folder.
async fn load_case(case: &Case, root: &Path) -> (anyhow::Result<Media>, Db, String) {
    let dir = root.join(case.name);
    std::fs::create_dir_all(&dir).unwrap();
    let d = dir.to_string_lossy().into_owned();
    let item_json = case.item.to_string().replace('@', &d);
    let mut item: item::Item = serde_json::from_str(&item_json).unwrap();
    let path = item.media_sources.first().and_then(|m| m.path.clone());
    if let (Some(p), Some(bytes)) = (&path, case.file) {
        std::fs::write(p, bytes).unwrap();
        // A fixed mtime: it is part of the file's key.
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        std::fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
    }
    if case.stamp {
        if let Some(ms) = item.media_sources.first_mut() {
            ms.stamp = Some((SIZE, MTIME));
        }
    }
    // The size and mtime the load will see (the library's stamp, else the file's own).
    let (size, mtime) = match (&path, case.stamp) {
        (_, true) => (SIZE, MTIME),
        (Some(p), false) => match crate::storage::vfs::stat(p).await {
            Ok(m) => (m.size, m.mtime),
            Err(_) => (0, 0),
        },
        _ => (0, 0),
    };
    let db = Db::memory().unwrap();
    if let (Some(p), Some(probe)) = (&path, &case.probe) {
        db.probe_put(p.clone(), size, mtime, probe.clone()).await.unwrap();
        if let Some(kf) = &case.keyframes {
            db.keyframes_put(p.clone(), size, mtime, kf.clone()).await.unwrap();
        }
    }
    if let (Some(p), true) = (&path, case.library_file) {
        let p = p.clone();
        db.call(move |c| {
            c.execute(
                "INSERT INTO files(id, library_id, path, size, mtime, role, first_seen, last_seen, stable_since) VALUES ('F1', 'lib', ?1, ?2, ?3, 'main', 1, 1, 1)",
                rusqlite::params![p, SIZE as i64, MTIME],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }
    let fallback = probe_of(
        "mov,mp4,m4a,3gp,3g2,mj2",
        "12.0",
        vec![
            v_stream(
                0,
                "h264",
                json!({"profile": "High", "level": 40, "pix_fmt": "yuv420p", "width": 640, "height": 360}),
            ),
            a_stream(1, "aac", json!({})),
        ],
    );
    let ffprobe = fake_ffprobe(&dir, &fallback);
    let env: HashMap<&str, &str> = case.env.iter().copied().collect();
    let config = Config::from_lookup(|k| match k {
        "MOONRISE_FFPROBE" => Some(ffprobe.clone()),
        "MOONRISE_FFMPEG" => Some("/nonexistent/ffmpeg".into()),
        _ => env.get(k).map(ToString::to_string),
    });
    // The scanner's ffprobe isn't there: a background scan it starts fails (later).
    let scanner = Arc::new(Scanner::new("/nonexistent/ffprobe".into(), Some(db.clone())));
    let http = reqwest::Client::new();
    let ctx = LoadCtx {
        http: &http,
        config: &config,
        db: case.dbs.then_some(&db),
        library_db: case.dbs.then_some(&db),
        scanner: &scanner,
    };
    let result = Media::load(&item, &ctx).await;
    if let Ok(m) = &result {
        // The item is the input, cloned: left out.
        assert_eq!(m.item.id, item.id);
    }
    (result, db, d)
}

async fn describe(result: anyhow::Result<Media>, db: Db, d: String) -> String {
    let mut out = match &result {
        Ok(m) => {
            let mut m = m.clone();
            m.item = Arc::new(item::Item::default());
            format!("{m:#?}\n")
        }
        Err(e) => format!("Err({e:#})\n"),
    };
    // What the load left in the database.
    let (probe_cached, keyframes_cached, role, probe_error) = db
        .call(|c| {
            let p: Option<(String, Option<Vec<u8>>)> =
                rusqlite::OptionalExtension::optional(c.query_row("SELECT probe, keyframes FROM files_probe LIMIT 1", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                }))?;
            let f: Option<(String, Option<String>)> =
                rusqlite::OptionalExtension::optional(
                    c.query_row("SELECT role, probe_error FROM files LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?))),
                )?;
            Ok((
                p.as_ref().map(|p| p.0.len()),
                p.and_then(|p| p.1).map(|k| k.len() / 8),
                f.as_ref().map(|f| f.0.clone()),
                f.and_then(|f| f.1),
            ))
        })
        .await
        .unwrap();
    let _ = writeln!(
        out,
        "db: probe bytes {probe_cached:?}, keyframes {keyframes_cached:?}, file role {role:?}, probe_error {probe_error:?}"
    );
    out.replace(&d, "<dir>")
}

#[tokio::test]
async fn load_golden() {
    golden_run().await;
}

/// Every case, checked against its golden.
async fn golden_run() {
    // A folder of its own; the hashes are made as if it were `GOLDEN_ROOT` (`run_case`).
    let root = crate::testutil::TempDir::new("load-golden");
    let mut all = String::new();
    for case in cases() {
        let out = run_case(&case, root.path()).await;
        crate::golden::check(&format!("media_load/{}", case.name), &out);
        all.push_str(&out);
    }
    assert!(!all.is_empty());
}

/// Two runs at once (two test processes sharing one temp folder) used the same fixed folder:
/// each removed and rewrote the other's files, and both failed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_golden_runs_at_once_keep_to_their_own_folders() {
    let runs: Vec<_> = (0..4).map(|_| tokio::spawn(golden_run())).collect();
    for run in runs {
        run.await.unwrap();
    }
}

/// A title opened before the scan has probed its file has no bitrate in the library yet:
/// it was planned at the 1 Mb/s floor, its master advertised 1.64 Mb/s for a
/// 12 Mb/s file, and a 3 Mb/s quality limit still got the copy, the only rung.
#[tokio::test]
async fn a_title_opened_before_the_scan_probed_it_has_its_real_bitrate() {
    use crate::media::{hwaccel::Hw, transcode};
    let streams = json!([
        {"Index": 0, "Type": "Video", "Codec": "h264"},
        {"Index": 1, "Type": "Audio", "Codec": "aac", "IsDefault": true, "Channels": 2}
    ]);
    let mut probe = probe_of(
        "matroska,webm",
        "150.021",
        vec![
            v_stream(
                0,
                "h264",
                json!({"profile": "High", "level": 40, "pix_fmt": "yuv420p", "width": 1920, "height": 1080,
                "avg_frame_rate": "24/1", "has_b_frames": 2, "nal_length_size": "4"}),
            ),
            a_stream(1, "aac", json!({"profile": "LC"})),
        ],
    );
    probe["format"]["bit_rate"] = json!("12140071");
    let mut item = item_of("b1", "@/b.mkv", None, streams);
    // Not probed by the scan yet: no bitrate in the library.
    item["MediaSources"][0].as_object_mut().unwrap().remove("Bitrate");
    let case = Case {
        name: "before_the_scan_probe",
        env: &[],
        item,
        probe: Some(probe),
        keyframes: Some(every(2.0, 150.0)),
        file: None,
        library_file: false,
        stamp: true,
        dbs: true,
    };
    let root = crate::testutil::TempDir::new("load-bitrate");
    let (media, _, _) = load_case(&case, root.path()).await;
    let media = media.unwrap();
    let video = media.video.as_ref().unwrap();
    assert_eq!(video.bitrate, 12_140_071);
    let ladder = transcode::ladder_with(&Hw::cpu(4), video, false, true);
    assert!(ladder[0].is_copy() && ladder[0].bitrate == 12_140_071, "{ladder:?}");
    // A 3 Mb/s limit gets a transcode within it, not the copy.
    let chosen = &ladder[transcode::select(&ladder, Some(3_000_000))];
    assert!(chosen.transcode && chosen.bitrate <= 3_000_000, "{chosen:?}");
}
