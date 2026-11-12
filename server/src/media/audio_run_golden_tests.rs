//! Golden test for a conversion run (`Converter::run`): the decoder's and encoder's command
//! lines and the segments it writes, with a stand-in ffmpeg (a decoder that reports its frames
//! and an encoder that answers with fixed ADTS frames).

use std::fmt::Write as _;
use std::{path::Path, sync::Arc};

use super::{
    audio::{Conversion, Converter, CONV_VERSION},
    transcode::tests::{media, video},
    AudioTrack, Media,
};
use crate::storage::{cache::Cache, source::Source};

fn plain_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

fn full_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 4];
    b.extend_from_slice(body);
    plain_box(kind, &b)
}

/// An init section: track 2, timescale 48000.
pub(super) fn init() -> Vec<u8> {
    let mut tkhd = vec![0u8; 8];
    tkhd.extend_from_slice(&2u32.to_be_bytes());
    tkhd.extend_from_slice(&[0u8; 72]);
    let mut mdhd = vec![0u8; 8];
    mdhd.extend_from_slice(&48000u32.to_be_bytes());
    mdhd.extend_from_slice(&[0u8; 8]);
    let mdia = plain_box(b"mdia", &full_box(b"mdhd", &mdhd));
    let trak = plain_box(b"trak", &[full_box(b"tkhd", &tkhd), mdia].concat());
    [plain_box(b"ftyp", b"iso6\0\0\0\0"), plain_box(b"moov", &trak)].concat()
}

/// ADTS frames (AAC-LC, 48 kHz, stereo), each `len` bytes with its number in the payload.
fn adts_frames(count: usize, len: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..count {
        out.extend_from_slice(&[
            0xFF,
            0xF1,
            0x4C,
            0x80 | ((len >> 11) & 3) as u8,
            ((len >> 3) & 0xFF) as u8,
            (((len & 7) << 5) as u8) | 0x1F,
            0xFC,
        ]);
        let mut payload = vec![(i % 251) as u8; len - 7];
        payload[0] = (i >> 8) as u8;
        out.extend_from_slice(&payload);
    }
    out
}

/// The stand-in ffmpeg: logs its arguments; as the decoder (`-af`) it reports 120 frames of
/// 1024 samples from `pts0` on stderr and writes PCM; as the encoder (`pipe:0`) it reads its
/// input and answers with one frame of `frames.adts` per 1024 samples it was given, and one more
/// (the encoder's priming).
pub(super) fn fake_ffmpeg(dir: &Path, pts0: i64) -> String {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(dir.join("frames.adts"), adts_frames(1000, 24)).unwrap();
    let showinfo: String = (0..120)
        .map(|i| {
            format!(
                "[Parsed_ashowinfo_0 @ 0x1] n:{i} pts:{} pts_time:0 nb_samples:1024 checksum:0\n",
                pts0 + i * 1024
            )
        })
        .collect();
    std::fs::write(dir.join("showinfo.txt"), showinfo).unwrap();
    let p = dir.join("ffmpeg");
    let d = dir.display();
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\necho \"$*\" >> '{d}/args.log'\ncase \"$*\" in\n  *pipe:0*) n=$(( $(wc -c) / 8192 + 1 )); head -c $(( n * 24 )) '{d}/frames.adts' ;;\n  *ashowinfo*) cat '{d}/showinfo.txt' >&2; head -c 7680000 /dev/zero ;;\nesac\nexit 0\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p.to_string_lossy().into_owned()
}

/// A film of three segments and its DTS track, converted to `conv`.
pub(super) fn film_and_track(conv: Conversion) -> (Media, AudioTrack) {
    let mut m: Media = media(video("h264", 1920, 1080, 8_000_000, "SDR"));
    m.source = Source::Local("/m/film.mkv".into());
    m.duration = 18.5;
    m.boundaries = vec![0.0, 6.0, 12.0];
    m.seek_offsets = vec![0.0, 0.1, 0.1];
    let a = AudioTrack {
        index: 1,
        stream: 1,
        codec: "dts".into(),
        codecs_attr: conv.codecs_attr().into(),
        name: "DTS".into(),
        language: None,
        channels: 2,
        default: true,
        converted: true,
        conversion: Some(conv),
        lossless_source: false,
        source_channels: 6,
        sample_rate: 48000,
        time_base: 0.001,
    };
    (m, a)
}

async fn run_case(name: &str, first: usize, pts0: i64, root: &Path) -> String {
    run_case_at(name, first, pts0, 0.0, root).await
}

/// `run_case` for a file whose clock starts at `start` (`Media::origin`).
async fn run_case_at(name: &str, first: usize, pts0: i64, start: f64, root: &Path) -> String {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let ffmpeg = fake_ffmpeg(&dir, pts0);
    let cache = Arc::new(Cache::new(dir.join("cache"), 1 << 30));
    let conv = Conversion::for_source(2, 48000);
    cache.put(&format!("conv-init/v{CONV_VERSION}/{}.mp4", conv.tag()), &init()).await;
    let converter = Arc::new(Converter::new(ffmpeg, "/nonexistent/ffprobe".into(), cache.clone(), 2));
    let (mut m, a) = film_and_track(conv);
    if start > 0.0 {
        (m.start_time, m.origin) = (start, start);
        m.boundaries = vec![0.0, start + 6.0, start + 12.0];
    }
    let m = Arc::new(m);
    let mut out = String::new();
    for n in first..m.boundaries.len() {
        match converter.segment(&m, &a, n, "golden").await {
            Ok(b) => {
                let _ = writeln!(out, "segment {n}: {} bytes, fnv {:016x}", b.len(), fnv_bytes(&b));
            }
            Err(e) => {
                let _ = writeln!(out, "segment {n}: Err({e:#})");
            }
        }
    }
    // Every file the run left in the cache.
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    collect(&dir.join("cache"), &dir.join("cache"), &mut files);
    files.sort();
    for (path, bytes) in files {
        let _ = writeln!(out, "cache {path}: {} bytes, fnv {:016x}", bytes.len(), fnv_bytes(&bytes));
    }
    // The decoder and the encoder start together: their order in the log varies.
    let log = std::fs::read_to_string(dir.join("args.log")).unwrap_or_default();
    let mut lines: Vec<&str> = log.lines().collect();
    lines.sort_unstable();
    for line in lines {
        let _ = writeln!(out, "ffmpeg {line}");
    }
    out
}

fn fnv_bytes(b: &[u8]) -> u64 {
    // FNV-1a.
    b.iter()
        .fold(0xcbf29ce484222325u64, |h, x| (h ^ *x as u64).wrapping_mul(0x100000001b3))
}

fn collect(base: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.filter_map(std::result::Result::ok) {
        let p = e.path();
        if p.is_dir() {
            collect(base, &p, out);
        } else {
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
            out.push((rel, std::fs::read(&p).unwrap()));
        }
    }
}

#[tokio::test]
async fn conversion_run_golden() {
    let root = std::env::temp_dir().join(format!("moonrise-audio-golden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    // From the start; from segment 1 (a seek, with pre-roll); and a source starting late.
    let mut out = String::new();
    out.push_str("# from 0\n");
    out.push_str(&run_case("from0", 0, 0, &root).await);
    out.push_str("# from 1\n");
    out.push_str(&run_case("from1", 1, 132_096, &root).await);
    out.push_str("# from 0, the source 0.5 s late\n");
    out.push_str(&run_case("late", 0, 24_000, &root).await);
    // Segment 0 of a file whose clock starts at 1001 s begins there, not with 1001 s of
    // silence from 0; the decoder isn't asked to seek past the file's start.
    out.push_str("# from 0, a file starting at 1001 s\n");
    out.push_str(&run_case_at("origin", 0, 1001 * 48_000, 1001.0, &root).await);
    let _ = std::fs::remove_dir_all(&root);
    crate::golden::check("audio/conversion_run", &out.replace(&root.to_string_lossy().into_owned(), "<dir>"));
}
