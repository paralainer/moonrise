//! Keyframe times of an MP4/MOV file from its `moov` box, read with a few range reads (the
//! top-level box headers, then the `moov` itself) instead of scanning the file.
//!
//! For the video track: decode times from `stts`, composition offsets from `ctts`, sync samples
//! from `stss` (none: every sample is one), shifted by the edit list (`elst`) the way ffmpeg's
//! demuxer presents them, so the times match the timestamps the segment cuts see.

use anyhow::{bail, Context, Result};
use std::fmt::Write as _;

use crate::{
    media::mp4::{boxes, find},
    storage::source::Source,
};

/// Larger `moov` boxes are refused (a 3-hour 60 fps movie's is a few MB).
const MAX_MOOV: u64 = 256 << 20;

pub async fn keyframes(source: &Source, http: &reqwest::Client) -> Result<Vec<f64>> {
    parse_moov(&read_moov(source, http).await?)
}

/// The `moov` box's body: the top-level box headers walked from the start, then one read.
pub async fn read_moov(source: &Source, http: &reqwest::Client) -> Result<Vec<u8>> {
    read_moov_with(&crate::media::mkv::SourceRead(source, http)).await
}

pub async fn read_moov_with(rd: &impl crate::media::mkv::RangeRead) -> Result<Vec<u8>> {
    let mut pos = 0u64;
    for _ in 0..256 {
        let head = rd.read_range(pos, 16).await?;
        if head.len() < 8 {
            break;
        }
        let size32 = u32::from_be_bytes(head[0..4].try_into().unwrap()) as u64;
        let kind = &head[4..8];
        let (size, header) = match size32 {
            1 if head.len() >= 16 => (u64::from_be_bytes(head[8..16].try_into().unwrap()), 16),
            0 => bail!("box {} runs to the end of the file before any moov", String::from_utf8_lossy(kind)),
            s => (s, 8),
        };
        if size < header {
            bail!("bad box size at {pos}");
        }
        if kind == b"moov" {
            if size > MAX_MOOV {
                bail!("moov too large ({size})");
            }
            let body = rd.read_range(pos + header, size - header).await?;
            if (body.len() as u64) < size - header {
                bail!("truncated moov");
            }
            return Ok(body);
        }
        pos += size;
    }
    bail!("no moov")
}

/// A timed-text sample (`tx3g`): where it is and when it shows (seconds, the file's time as
/// ffmpeg presents it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextSample {
    pub time: f64,
    pub duration: f64,
    pub offset: u64,
    pub size: u32,
}

/// A timed-text track: its position among the `trak`s (ffmpeg's stream index) and samples.
#[derive(Debug, Clone)]
pub struct TextTrack {
    pub order: usize,
    pub samples: Vec<TextSample>,
}

/// Every track's handler (`vide`, `soun`, `text`, `sbtl`, …) in `trak` order, and the timed-text
/// tracks with their sample tables.
pub fn text_tracks(moov: &[u8]) -> Result<(Vec<String>, Vec<TextTrack>)> {
    let mut movie_timescale = 1000u32;
    if let Some((p, _)) = find(moov, 0, moov.len(), &["mvhd"])? {
        let at = p + if moov[p] == 1 { 4 + 16 } else { 4 + 8 };
        movie_timescale = be32(moov, at)?;
    }
    let mut handlers = Vec::new();
    let mut out = Vec::new();
    for (kind, _, payload, end) in boxes(moov, 0, moov.len())? {
        if kind != "trak" {
            continue;
        }
        let order = handlers.len();
        let d = moov;
        let handler = match find(d, payload, end, &["mdia", "hdlr"])? {
            Some((h, _)) => String::from_utf8_lossy(d.get(h + 8..h + 12).unwrap_or(b"????")).into_owned(),
            None => "????".into(),
        };
        handlers.push(handler.clone());
        if !matches!(handler.as_str(), "text" | "sbtl" | "subt") {
            continue;
        }
        let Some((stbl, stbl_end)) = find(d, payload, end, &["mdia", "minf", "stbl"])? else {
            continue;
        };
        // Only 3GPP timed text (what ffmpeg calls mov_text).
        let Some((stsd, _)) = find(d, stbl, stbl_end, &["stsd"])? else {
            continue;
        };
        if d.get(stsd + 12..stsd + 16) != Some(b"tx3g") {
            continue;
        }
        let (mdhd, _) = find(d, payload, end, &["mdia", "mdhd"])?.context("no mdhd")?;
        let timescale = be32(d, mdhd + if d[mdhd] == 1 { 4 + 16 } else { 4 + 8 })? as f64;
        if timescale == 0.0 {
            bail!("mdhd timescale 0");
        }
        // Decode times and durations.
        let (stts, _) = find(d, stbl, stbl_end, &["stts"])?.context("no stts")?;
        let mut times: Vec<(i64, i64)> = Vec::new();
        let mut t = 0i64;
        for i in 0..be32(d, stts + 4)? as usize {
            let count = be32(d, stts + 8 + i * 8)?;
            let delta = be32(d, stts + 12 + i * 8)? as i64;
            if times.len() as u64 + count as u64 > 5_000_000 {
                bail!("implausible sample count");
            }
            for _ in 0..count {
                times.push((t, delta));
                t += delta;
            }
        }
        let (sizes, offsets) = sample_locations(d, stbl, stbl_end)?;
        // Edit list, as for the video (`video_trak`).
        let (mut empty, mut media_time) = (0.0f64, 0i64);
        if let Some((elst, _)) = find(d, payload, end, &["edts", "elst"])? {
            let v1 = d[elst] == 1;
            let entry = if v1 { 20 } else { 12 };
            for e in 0..be32(d, elst + 4)? as usize {
                let at = elst + 8 + e * entry;
                let (seg_dur, mt) = if v1 {
                    (be64(d, at)? as f64, be64(d, at + 8)? as i64)
                } else {
                    (be32(d, at)? as f64, be32(d, at + 4)? as i32 as i64)
                };
                if mt == -1 {
                    empty += seg_dur / movie_timescale.max(1) as f64;
                } else {
                    media_time = mt;
                    break;
                }
            }
        }
        let n = times.len().min(sizes.len()).min(offsets.len());
        let samples = (0..n)
            .map(|i| TextSample {
                time: (times[i].0 - media_time) as f64 / timescale + empty,
                duration: times[i].1 as f64 / timescale,
                offset: offsets[i],
                size: sizes[i],
            })
            .collect();
        out.push(TextTrack { order, samples });
    }
    Ok((handlers, out))
}

/// A `tx3g` sample as SRT text: the text (UTF-8, or UTF-16 with a BOM) with its `styl` runs as
/// `<b>`, `<i>`, `<u>` tags (as ffmpeg writes them). None: an empty sample (a gap).
pub fn tx3g_text(sample: &[u8]) -> Option<String> {
    let len = u16::from_be_bytes(sample.get(0..2)?.try_into().ok()?) as usize;
    let raw = sample.get(2..2 + len)?;
    if raw.is_empty() {
        return None;
    }
    let text: String = if raw.starts_with(&[0xFE, 0xFF]) {
        let u: Vec<u16> = raw[2..].as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        String::from_utf16_lossy(&u)
    } else {
        String::from_utf8_lossy(raw).into_owned()
    };
    // Style runs: (start char, end char, flags).
    let mut runs: Vec<(usize, usize, u8)> = Vec::new();
    let mut pos = 2 + len;
    while pos + 8 <= sample.len() {
        let size = u32::from_be_bytes(sample[pos..pos + 4].try_into().ok()?) as usize;
        if size < 8 || pos + size > sample.len() {
            break;
        }
        if &sample[pos + 4..pos + 8] == b"styl" && size >= 10 {
            let count = u16::from_be_bytes([sample[pos + 8], sample[pos + 9]]) as usize;
            for k in 0..count {
                let e = pos + 10 + k * 12;
                if e + 12 > pos + size {
                    break;
                }
                let s = u16::from_be_bytes([sample[e], sample[e + 1]]) as usize;
                let t = u16::from_be_bytes([sample[e + 2], sample[e + 3]]) as usize;
                let flags = sample[e + 6] & 0x07;
                if flags != 0 && t > s {
                    runs.push((s, t, flags));
                }
            }
        }
        pos += size;
    }
    let mut out = String::with_capacity(text.len() + 16);
    let tags = |flags: u8, open: bool| -> String {
        let mut s = String::new();
        let order: &[(u8, &str)] = if open {
            &[(1, "b"), (2, "i"), (4, "u")]
        } else {
            &[(4, "u"), (2, "i"), (1, "b")]
        };
        for (bit, t) in order {
            if flags & bit != 0 {
                let _ = write!(s, "<{}{t}>", if open { "" } else { "/" });
            }
        }
        s
    };
    let chars: Vec<char> = text.chars().collect();
    for (i, ch) in chars.iter().enumerate() {
        for r in runs.iter().filter(|r| r.0 == i) {
            out += &tags(r.2, true);
        }
        out.push(*ch);
        for r in runs.iter().filter(|r| r.1 == i + 1) {
            out += &tags(r.2, false);
        }
    }
    let out = out.replace("\r\n", "\n").replace('\r', "\n");
    let out = out.trim();
    (!out.is_empty()).then(|| out.to_string())
}

/// Each sample's size and offset in the file, from `stsz`, `stco`/`co64` and `stsc`.
fn sample_locations(d: &[u8], stbl: usize, stbl_end: usize) -> Result<(Vec<u32>, Vec<u64>)> {
    let (stsz, _) = find(d, stbl, stbl_end, &["stsz"])?.context("no stsz")?;
    let fixed = be32(d, stsz + 4)?;
    let count = be32(d, stsz + 8)? as usize;
    if count > 20_000_000 {
        bail!("implausible sample count");
    }
    let sizes: Vec<u32> = if fixed != 0 {
        vec![fixed; count]
    } else {
        (0..count).map(|i| be32(d, stsz + 12 + i * 4)).collect::<Result<_>>()?
    };
    let chunks: Vec<u64> = if let Some((stco, _)) = find(d, stbl, stbl_end, &["stco"])? {
        (0..be32(d, stco + 4)? as usize)
            .map(|i| be32(d, stco + 8 + i * 4).map(u64::from))
            .collect::<Result<_>>()?
    } else if let Some((co64, _)) = find(d, stbl, stbl_end, &["co64"])? {
        (0..be32(d, co64 + 4)? as usize)
            .map(|i| be64(d, co64 + 8 + i * 8))
            .collect::<Result<_>>()?
    } else {
        bail!("no chunk offsets");
    };
    // Samples per chunk (runs from first_chunk, 1-based).
    let (stsc, _) = find(d, stbl, stbl_end, &["stsc"])?.context("no stsc")?;
    let runs: Vec<(usize, usize)> = (0..be32(d, stsc + 4)? as usize)
        .map(|i| Ok((be32(d, stsc + 8 + i * 12)? as usize, be32(d, stsc + 12 + i * 12)? as usize)))
        .collect::<Result<_>>()?;
    let mut offsets = Vec::with_capacity(sizes.len());
    let mut run = 0usize;
    for (ci, &chunk_at) in chunks.iter().enumerate() {
        while run + 1 < runs.len() && runs[run + 1].0 <= ci + 1 {
            run += 1;
        }
        let per = runs.get(run).filter(|r| r.0 <= ci + 1).map_or(1, |r| r.1);
        let mut at = chunk_at;
        for _ in 0..per {
            let Some(&sz) = sizes.get(offsets.len()) else { break };
            offsets.push(at);
            at += sz as u64;
        }
    }
    Ok((sizes, offsets))
}

/// A keyframe of the video track: when it shows (seconds, as `parse_moov` gives them) and where
/// its sample is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeySample {
    pub time: f64,
    pub offset: u64,
    pub size: u32,
}

/// The video track's keyframes with their samples' places, and its sample entry's codec
/// (`avc1`, `hvc1`, …) and decoder configuration (the `avcC`/`hvcC` box's body), so a keyframe
/// can be read alone and decoded without the rest of the file (`trickplay.rs`).
#[derive(Debug, Clone)]
pub struct VideoKeys {
    pub codec: [u8; 4],
    pub config: Vec<u8>,
    pub keys: Vec<KeySample>,
}

pub fn video_keys(moov: &[u8]) -> Result<VideoKeys> {
    let t = best_video(moov)?;
    let d = moov;
    let (stbl, stbl_end) = t.stbl;
    let (stsd, stsd_end) = find(d, stbl, stbl_end, &["stsd"])?.context("no stsd")?;
    // The first entry: after the full box header and the entry count.
    let entry = stsd + 8;
    let entry_size = be32(d, entry)? as usize;
    if entry_size < 8 + 78 || entry + entry_size > stsd_end {
        bail!("bad sample entry");
    }
    let codec: [u8; 4] = d[entry + 4..entry + 8].try_into().unwrap();
    let name = match &codec {
        b"avc1" | b"avc3" => "avcC",
        b"hvc1" | b"hev1" | b"dvh1" | b"dvhe" => "hvcC",
        other => bail!("codec {} isn't read keyframe by keyframe", String::from_utf8_lossy(other)),
    };
    // A visual sample entry's own fields are 78 bytes; its boxes follow.
    let (cfg, cfg_end) = find(d, entry + 8 + 78, entry + entry_size, &[name])?.context("no decoder configuration")?;
    let config = d[cfg..cfg_end].to_vec();
    let (sizes, offsets) = sample_locations(d, stbl, stbl_end)?;
    let mut keys: Vec<KeySample> = t
        .key_samples
        .iter()
        .filter(|(_, i)| *i < sizes.len() && *i < offsets.len())
        .map(|&(time, i)| KeySample {
            time,
            offset: offsets[i],
            size: sizes[i],
        })
        .collect();
    keys.sort_by(|a, b| a.time.total_cmp(&b.time));
    if keys.is_empty() {
        bail!("no keyframes");
    }
    Ok(VideoKeys { codec, config, keys })
}

#[derive(Debug, Default)]
struct Trak {
    sample_count: u64,
    keyframes: Vec<f64>,
    /// The keyframes' times and sample numbers (0-based), and the track's `stbl`.
    key_samples: Vec<(f64, usize)>,
    stbl: (usize, usize),
}

/// Keyframe times (seconds, ascending) of the video track with the most samples (cover art is a
/// one-sample video track).
pub fn parse_moov(moov: &[u8]) -> Result<Vec<f64>> {
    let t = best_video(moov)?;
    if t.keyframes.len() < 2 {
        bail!("too few keyframes ({})", t.keyframes.len());
    }
    Ok(t.keyframes)
}

fn best_video(moov: &[u8]) -> Result<Trak> {
    let mut movie_timescale = 1000u32;
    if let Some((p, _)) = find(moov, 0, moov.len(), &["mvhd"])? {
        let at = p + if moov[p] == 1 { 4 + 16 } else { 4 + 8 };
        movie_timescale = be32(moov, at)?;
    }
    let mut best: Option<Trak> = None;
    for (kind, _, payload, end) in boxes(moov, 0, moov.len())? {
        if kind != "trak" {
            continue;
        }
        let Some(t) = video_trak(moov, payload, end, movie_timescale)? else {
            continue;
        };
        if best.as_ref().is_none_or(|b| t.sample_count > b.sample_count) {
            best = Some(t);
        }
    }
    let t = best.context("no video track")?;
    if t.sample_count == 0 {
        bail!("no samples in moov (fragmented MP4)");
    }
    Ok(t)
}

fn video_trak(d: &[u8], start: usize, end: usize, movie_timescale: u32) -> Result<Option<Trak>> {
    let Some((hdlr, _)) = find(d, start, end, &["mdia", "hdlr"])? else {
        return Ok(None);
    };
    if d.get(hdlr + 8..hdlr + 12) != Some(b"vide") {
        return Ok(None);
    }
    let (mdhd, _) = find(d, start, end, &["mdia", "mdhd"])?.context("no mdhd")?;
    let timescale = be32(d, mdhd + if d[mdhd] == 1 { 4 + 16 } else { 4 + 8 })? as f64;
    if timescale == 0.0 {
        bail!("mdhd timescale 0");
    }
    let (stbl, stbl_end) = find(d, start, end, &["mdia", "minf", "stbl"])?.context("no stbl")?;

    // Decode time of each sample.
    let (stts, _) = find(d, stbl, stbl_end, &["stts"])?.context("no stts")?;
    let mut dts: Vec<i64> = Vec::new();
    let mut t = 0i64;
    for i in 0..be32(d, stts + 4)? as usize {
        let count = be32(d, stts + 8 + i * 8)?;
        let delta = be32(d, stts + 12 + i * 8)? as i64;
        if dts.len() as u64 + count as u64 > 20_000_000 {
            bail!("implausible sample count");
        }
        for _ in 0..count {
            dts.push(t);
            t += delta;
        }
    }
    let n = dts.len();

    // Composition offsets (version 0 is unsigned on paper, but written signed in practice;
    // ffmpeg reads both as signed).
    let mut cto = vec![0i64; n];
    if let Some((ctts, _)) = find(d, stbl, stbl_end, &["ctts"])? {
        let mut i = 0usize;
        for e in 0..be32(d, ctts + 4)? as usize {
            let count = be32(d, ctts + 8 + e * 8)? as usize;
            let off = be32(d, ctts + 12 + e * 8)? as i32 as i64;
            for _ in 0..count {
                if i < n {
                    cto[i] = off;
                }
                i += 1;
            }
        }
    }

    // Sync samples (1-based); without stss every sample is one.
    let sync: Vec<usize> = match find(d, stbl, stbl_end, &["stss"])? {
        Some((stss, _)) => (0..be32(d, stss + 4)? as usize)
            .map(|i| be32(d, stss + 8 + i * 4).map(|s| s as usize))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|s| (1..=n).contains(s))
            .map(|s| s - 1)
            .collect(),
        None => (0..n).collect(),
    };

    // Edit list: leading empty edits delay the track; the first real edit's media time is the
    // media timestamp shown at that point.
    let (mut empty, mut media_time) = (0.0f64, 0i64);
    if let Some((elst, _)) = find(d, start, end, &["edts", "elst"])? {
        let v1 = d[elst] == 1;
        let entry = if v1 { 20 } else { 12 };
        for e in 0..be32(d, elst + 4)? as usize {
            let at = elst + 8 + e * entry;
            let (seg_dur, mt) = if v1 {
                (be64(d, at)? as f64, be64(d, at + 8)? as i64)
            } else {
                (be32(d, at)? as f64, be32(d, at + 4)? as i32 as i64)
            };
            if mt == -1 {
                empty += seg_dur / movie_timescale.max(1) as f64;
            } else {
                media_time = mt;
                break;
            }
        }
    }

    let key_samples: Vec<(f64, usize)> = sync
        .iter()
        .map(|&i| ((dts[i] + cto[i] - media_time) as f64 / timescale + empty, i))
        .collect();
    let mut keyframes: Vec<f64> = key_samples.iter().map(|k| k.0).collect();
    keyframes.sort_by(|a, b| a.partial_cmp(b).unwrap());
    keyframes.dedup();
    Ok(Some(Trak {
        sample_count: n as u64,
        keyframes,
        key_samples,
        stbl: (stbl, stbl_end),
    }))
}

fn be32(d: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(d.get(at..at + 4).context("truncated box")?.try_into().unwrap()))
}

fn be64(d: &[u8], at: usize) -> Result<u64> {
    Ok(u64::from_be_bytes(d.get(at..at + 8).context("truncated box")?.try_into().unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        [((body.len() + 8) as u32).to_be_bytes().as_slice(), kind, body].concat()
    }

    fn full(kind: &[u8; 4], version: u8, body: &[u8]) -> Vec<u8> {
        bx(kind, &[&[version, 0, 0, 0][..], body].concat())
    }

    fn table(entries: &[&[u32]]) -> Vec<u8> {
        let mut b = (entries.len() as u32).to_be_bytes().to_vec();
        for e in entries {
            for v in *e {
                b.extend_from_slice(&v.to_be_bytes());
            }
        }
        b
    }

    /// A video track: 12 frames at 1/24 s (timescale 24000, delta 1000), B-frame offsets,
    /// keyframes at samples 1, 5 and 9, an edit list starting at media time 2000.
    fn trak(handler: &[u8; 4], with_elst: bool, samples: u32) -> Vec<u8> {
        let hdlr = full(b"hdlr", 0, &[&[0u8; 4][..], handler, &[0u8; 12]].concat());
        let mdhd = full(b"mdhd", 0, &[&[0u8; 8][..], &24000u32.to_be_bytes(), &[0u8; 8]].concat());
        let stts = full(b"stts", 0, &table(&[&[samples, 1000]]));
        let ctts = full(b"ctts", 0, &table(&[&[1, 2000], &[(samples - 1), 1000]]));
        let stss = full(b"stss", 0, &table(&[&[1], &[5], &[9]]));
        let stbl = bx(b"stbl", &[stts, ctts, stss].concat());
        let minf = bx(b"minf", &stbl);
        let mdia = bx(b"mdia", &[mdhd, hdlr, minf].concat());
        let mut body = Vec::new();
        if with_elst {
            // One empty edit of 0.5 s (movie timescale 1000), then media time 2000.
            let elst = full(b"elst", 0, &table(&[&[500, u32::MAX, 0x10000], &[12000, 2000, 0x10000]]));
            body.extend(bx(b"edts", &elst));
        }
        body.extend(mdia);
        bx(b"trak", &body)
    }

    fn moov(traks: &[Vec<u8>]) -> Vec<u8> {
        let mvhd = full(b"mvhd", 0, &[&[0u8; 8][..], &1000u32.to_be_bytes(), &[0u8; 84]].concat());
        [mvhd, traks.concat()].concat()
    }

    fn close(got: &[f64], want: &[f64]) {
        assert_eq!(got.len(), want.len(), "{got:?}");
        assert!(got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-9), "{got:?} != {want:?}");
    }

    #[test]
    fn timed_text() {
        // "Hi there" with "there" (chars 3..8) italic.
        let mut sample = vec![0, 8];
        sample.extend_from_slice(b"Hi there");
        let styl = [&[0u8, 1][..], &[0, 3, 0, 8, 0, 1, 2, 18, 255, 255, 255, 255]].concat();
        sample.extend(bx(b"styl", &styl));
        assert_eq!(tx3g_text(&sample).as_deref(), Some("Hi <i>there</i>"));
        assert_eq!(tx3g_text(&[0, 0]), None);
        let mut u16 = vec![0, 6, 0xFE, 0xFF, 0, b'O', 0, b'k'];
        u16.truncate(8);
        assert_eq!(tx3g_text(&u16).as_deref(), Some("Ok"));

        // A text track: 3 samples (1 s, 2 s empty, 1.5 s) at timescale 1000 in two chunks.
        let hdlr = full(b"hdlr", 0, &[&[0u8; 4][..], b"sbtl", &[0u8; 12]].concat());
        let mdhd = full(b"mdhd", 0, &[&[0u8; 8][..], &1000u32.to_be_bytes(), &[0u8; 8]].concat());
        let stsd = full(b"stsd", 0, &[&1u32.to_be_bytes()[..], &bx(b"tx3g", &[0u8; 30])].concat());
        let stts = full(b"stts", 0, &table(&[&[1, 1000], &[1, 2000], &[1, 1500]]));
        let stsz = full(b"stsz", 0, &[&0u32.to_be_bytes()[..], &table(&[&[10], &[2], &[12]])].concat());
        let stco = full(b"stco", 0, &table(&[&[5000], &[9000]]));
        let stsc = full(b"stsc", 0, &table(&[&[1, 2, 1], &[2, 1, 1]]));
        let stbl = bx(b"stbl", &[stsd, stts, stsz, stco, stsc].concat());
        let minf = bx(b"minf", &stbl);
        let mdia = bx(b"mdia", &[mdhd, hdlr, minf].concat());
        let text = bx(b"trak", &mdia);
        let (handlers, tracks) = text_tracks(&moov(&[trak(b"vide", false, 12), text])).unwrap();
        assert_eq!(handlers, vec!["vide", "sbtl"]);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].order, 1);
        let s = &tracks[0].samples;
        assert_eq!(s.len(), 3);
        assert_eq!((s[0].offset, s[1].offset, s[2].offset), (5000, 5010, 9000));
        assert_eq!((s[1].time, s[2].time, s[2].duration), (1.0, 3.0, 1.5));
    }

    #[test]
    fn keyframes_with_edit_list() {
        let kf = parse_moov(&moov(&[trak(b"soun", false, 100), trak(b"vide", true, 12)])).unwrap();
        // Sample 0: dts 0 + cto 2000 - 2000 = 0, plus the 0.5 s empty edit.
        // Sample 4: 4000 + 1000 - 2000 = 3000 -> 0.125 s + 0.5.
        close(&kf, &[0.5, 0.625, 0.5 + 7000.0 / 24000.0]);
    }

    #[test]
    fn keyframe_samples_and_decoder_configuration() {
        // 6 samples at 1 s (timescale 1000), keyframes 1 and 4 (1-based), sizes 10..15, in two
        // chunks of 3 at 1000 and 5000, an avc1 entry with its avcC.
        let hdlr = full(b"hdlr", 0, &[&[0u8; 4][..], b"vide", &[0u8; 12]].concat());
        let mdhd = full(b"mdhd", 0, &[&[0u8; 8][..], &1000u32.to_be_bytes(), &[0u8; 8]].concat());
        let avcc = bx(b"avcC", &[1, 0x64, 0, 0x28, 0xff, 0xe1, 0, 1, 0x67, 1, 0, 1, 0x68]);
        let entry = bx(b"avc1", &[&[0u8; 78][..], &avcc].concat());
        let stsd = full(b"stsd", 0, &[&1u32.to_be_bytes()[..], &entry].concat());
        let stts = full(b"stts", 0, &table(&[&[6, 1000]]));
        let stss = full(b"stss", 0, &table(&[&[1], &[4]]));
        let stsz = full(
            b"stsz",
            0,
            &[&0u32.to_be_bytes()[..], &table(&[&[10], &[11], &[12], &[13], &[14], &[15]])].concat(),
        );
        let stco = full(b"stco", 0, &table(&[&[1000], &[5000]]));
        let stsc = full(b"stsc", 0, &table(&[&[1, 3, 1]]));
        let stbl = bx(b"stbl", &[stsd, stts, stss, stsz, stco, stsc].concat());
        let mdia = bx(b"mdia", &[mdhd, hdlr, bx(b"minf", &stbl)].concat());
        let v = video_keys(&moov(&[bx(b"trak", &mdia)])).unwrap();
        assert_eq!(&v.codec, b"avc1");
        assert_eq!(v.config, vec![1, 0x64, 0, 0x28, 0xff, 0xe1, 0, 1, 0x67, 1, 0, 1, 0x68]);
        assert_eq!(
            v.keys,
            vec![
                KeySample {
                    time: 0.0,
                    offset: 1000,
                    size: 10
                },
                KeySample {
                    time: 3.0,
                    offset: 5000,
                    size: 13
                }
            ]
        );
        // Another codec: refused (its pictures are found by seeking instead).
        let mp4v = moov(&[bx(
            b"trak",
            &bx(
                b"mdia",
                &[
                    full(b"mdhd", 0, &[&[0u8; 8][..], &1000u32.to_be_bytes(), &[0u8; 8]].concat()),
                    full(b"hdlr", 0, &[&[0u8; 4][..], b"vide", &[0u8; 12]].concat()),
                    bx(
                        b"minf",
                        &bx(
                            b"stbl",
                            &[
                                full(b"stsd", 0, &[&1u32.to_be_bytes()[..], &bx(b"mp4v", &[0u8; 78])].concat()),
                                full(b"stts", 0, &table(&[&[6, 1000]])),
                                full(
                                    b"stsz",
                                    0,
                                    &[&0u32.to_be_bytes()[..], &table(&[&[10], &[11], &[12], &[13], &[14], &[15]])].concat(),
                                ),
                                full(b"stco", 0, &table(&[&[1000], &[5000]])),
                                full(b"stsc", 0, &table(&[&[1, 3, 1]])),
                            ]
                            .concat(),
                        ),
                    ),
                ]
                .concat(),
            ),
        )]);
        assert!(format!("{:#}", video_keys(&mp4v).unwrap_err()).contains("mp4v"));
    }

    #[test]
    fn keyframes_without_edit_list_and_cover_art() {
        // The one-sample "video" (cover art) loses to the real track.
        let kf = parse_moov(&moov(&[trak(b"vide", false, 1), trak(b"vide", false, 12)])).unwrap();
        close(&kf, &[2000.0 / 24000.0, 5000.0 / 24000.0, 9000.0 / 24000.0]);
        assert!(parse_moov(&moov(&[trak(b"soun", false, 5)])).is_err());
    }
}
