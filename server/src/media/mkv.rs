//! A Matroska file's index (Cues) and track list, read with a handful of small range reads
//! instead of scanning the file: keyframe times (segment boundaries) and where subtitle blocks
//! are (`subx.rs`).

use anyhow::{bail, Context, Result};

use crate::storage::source::Source;

const SEGMENT: u32 = 0x18538067;
const SEEK_HEAD: u32 = 0x114D9B74;
const SEEK: u32 = 0x4DBB;
const SEEK_ID: u32 = 0x53AB;
const SEEK_POSITION: u32 = 0x53AC;
const INFO: u32 = 0x1549A966;
const TIMESTAMP_SCALE: u32 = 0x2AD7B1;
const TRACKS: u32 = 0x1654AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_TYPE: u32 = 0x83;
const CUES: u32 = 0x1C53BB6B;
const CUE_POINT: u32 = 0xBB;
const CUE_TIME: u32 = 0xB3;
const CUE_TRACK_POSITIONS: u32 = 0xB7;
const CUE_TRACK: u32 = 0xF7;
const CLUSTER: u32 = 0x1F43B675;

/// A Matroska file's headers and index: what the keyframe index and subtitle extraction need.
#[derive(Debug, Clone)]
pub struct Index {
    /// Where the Segment's data starts (positions in the file are relative to it).
    pub segment_data: u64,
    /// Nanoseconds per timestamp unit.
    pub timescale: u64,
    /// In the file's order (ffmpeg numbers its streams the same way).
    pub tracks: Vec<TrackInfo>,
    pub cues: Vec<CuePoint>,
    /// Where the Cues element is (None: no index).
    pub cues_at: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub number: u64,
    /// 1 video, 2 audio, 17 subtitle.
    pub kind: u64,
    pub codec: String,
    /// The track's ContentEncodings (compression, encryption): blocks aren't plain.
    pub encoded: bool,
    /// Nanoseconds (0: none).
    pub default_duration: u64,
    /// How its blocks are stored, when `encoded` (picture subtitles read it; text tracks don't).
    pub encoding: Encoding,
    /// CodecPrivate (a VobSub track's palette and size, as `.idx` text).
    pub codec_private: Vec<u8>,
}

/// A track's ContentEncodings, as far as reading its blocks goes.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Encoding {
    /// Plain blocks.
    #[default]
    None,
    /// zlib-compressed blocks (mkvmerge's default for picture subtitles).
    Zlib,
    /// Header stripping: these bytes come off the front of every block.
    Stripped(Vec<u8>),
    /// Anything else (encrypted, another compression): unreadable here.
    Other,
}

impl Encoding {
    /// The ContentEncodings element's body.
    fn parse(body: &[u8]) -> Encoding {
        let Ok(encodings) = children(body) else { return Encoding::Other };
        let mut found = Encoding::None;
        for (id, enc) in encodings {
            if id != CONTENT_ENCODING || !matches!(found, Encoding::None) {
                return Encoding::Other;
            }
            let Ok(parts) = children(enc) else { return Encoding::Other };
            for (pid, pbody) in parts {
                match pid {
                    CONTENT_COMPRESSION => {
                        let Ok(c) = children(pbody) else { return Encoding::Other };
                        let algo = c.iter().find(|(i, _)| *i == CONTENT_COMP_ALGO).map_or(0, |(_, b)| uint(b));
                        let settings = c.iter().find(|(i, _)| *i == CONTENT_COMP_SETTINGS).map(|(_, b)| b.to_vec());
                        found = match (algo, settings) {
                            (0, _) => Encoding::Zlib,
                            (3, Some(b)) => Encoding::Stripped(b),
                            _ => return Encoding::Other,
                        };
                    }
                    CONTENT_ENCRYPTION => return Encoding::Other,
                    // Only every frame (1, the default) is supported.
                    CONTENT_ENCODING_SCOPE if uint(pbody) & 1 == 0 => return Encoding::Other,
                    _ => {}
                }
            }
        }
        found
    }

    /// A block's payload as stored by the muxer.
    pub fn decode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        match self {
            Encoding::None => Ok(data),
            Encoding::Zlib => {
                use std::io::Read as _;
                let mut out = Vec::with_capacity(data.len() * 4);
                flate2::read::ZlibDecoder::new(&data[..])
                    .take(64 * 1024 * 1024)
                    .read_to_end(&mut out)?;
                Ok(out)
            }
            Encoding::Stripped(head) => Ok([head.as_slice(), &data].concat()),
            Encoding::Other => bail!("the track's blocks are encrypted or compressed in an unknown way"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CuePoint {
    /// In timestamp units.
    pub time: u64,
    pub track: u64,
    /// The Cluster's position, relative to the Segment's data.
    pub cluster: u64,
    /// The block's position inside the Cluster's data (mkvmerge writes it).
    pub relative: Option<u64>,
    /// In timestamp units.
    pub duration: Option<u64>,
}

impl Index {
    pub fn secs(&self, units: i64) -> f64 {
        units as f64 * self.timescale as f64 / 1e9
    }
}

const TRACK_CODEC: u32 = 0x86;
const CONTENT_ENCODINGS: u32 = 0x6D80;
const DEFAULT_DURATION: u32 = 0x23E383;
const CODEC_PRIVATE: u32 = 0x63A2;
const CONTENT_ENCODING: u32 = 0x6240;
const CONTENT_ENCODING_SCOPE: u32 = 0x5032;
const CONTENT_COMPRESSION: u32 = 0x5034;
const CONTENT_COMP_ALGO: u32 = 0x4254;
const CONTENT_COMP_SETTINGS: u32 = 0x4255;
const CONTENT_ENCRYPTION: u32 = 0x5035;
const CUE_CLUSTER_POSITION: u32 = 0xF1;
const CUE_RELATIVE_POSITION: u32 = 0xF0;
const CUE_DURATION: u32 = 0xB2;

/// Range reads of a file: through the source (a new open per read), or one open handle.
pub trait RangeRead: Sync {
    fn read_range(&self, offset: u64, len: u64) -> futures_util::future::BoxFuture<'_, Result<Vec<u8>>>;
}

/// A source's range reads (each opens the file).
pub struct SourceRead<'a>(pub &'a Source, pub &'a reqwest::Client);

impl RangeRead for SourceRead<'_> {
    fn read_range(&self, offset: u64, len: u64) -> futures_util::future::BoxFuture<'_, Result<Vec<u8>>> {
        Box::pin(async move { Ok(self.0.read_at(self.1, offset, len).await?.to_vec()) })
    }
}

/// A read of the file failed (the share, not what is in the file): worth trying again later.
#[derive(Debug)]
pub struct ReadFailed;

impl std::fmt::Display for ReadFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("read failed")
    }
}

/// A well-formed file without usable Cues.
#[derive(Debug)]
pub struct NoCues(String);

impl std::fmt::Display for NoCues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NoCues {}

/// What a failed index read says about the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// A read failed (a share error, a timeout): the file may be fine, so try again later.
    Read,
    /// The file is well formed but has no Cues (or too few to cut on).
    NoCues,
    /// What should be Matroska isn't (bad element ids or sizes, zeros, cut short, an element
    /// other than the one the SeekHead names): the file is damaged or incomplete.
    Damaged,
}

/// Classifies an error of `index`, `index_with` or `keyframes`.
pub fn failure(e: &anyhow::Error) -> Failure {
    if e.downcast_ref::<ReadFailed>().is_some() {
        Failure::Read
    } else if e.downcast_ref::<NoCues>().is_some() {
        Failure::NoCues
    } else {
        Failure::Damaged
    }
}

/// A range read, its error marked as a read failure (`failure`).
async fn read(rd: &impl RangeRead, at: u64, len: u64) -> Result<Vec<u8>> {
    rd.read_range(at, len).await.context(ReadFailed)
}

/// The headers (SeekHead, Info, Tracks) and the Cues, in a handful of range reads.
pub async fn index(source: &Source, http: &reqwest::Client) -> Result<Index> {
    index_with(&SourceRead(source, http)).await
}

/// The same through any reader (one open handle: each open over CIFS is a round trip, and slow
/// while the share is busy).
pub async fn index_with(rd: &impl RangeRead) -> Result<Index> {
    let head = read(rd, 0, 64 * 1024).await?;
    let mut r = Reader::new(&head);
    let (id, size) = r.element()?;
    if id != 0x1A45DFA3 {
        bail!("not EBML");
    }
    r.skip(size)?;
    let (id, _) = r.element()?;
    if id != SEGMENT {
        bail!("no Segment");
    }
    let segment_data = r.pos as u64;

    // Top-level element positions, from the SeekHead at the start and the ones it points to.
    let mut positions: Vec<(u32, u64)> = Vec::new();
    let mut seek_heads = vec![segment_data];
    let mut seen = Vec::new();
    while let Some(at) = seek_heads.pop() {
        if seen.contains(&at) || seen.len() > 4 {
            continue;
        }
        seen.push(at);
        let first = at == segment_data;
        // mkvmerge puts a second SeekHead at the end that lists every Cluster (hundreds of KiB
        // for a long film). It is only needed for what the first one doesn't list, so a second
        // one that can't be read (or isn't there: a damaged file) is passed over.
        let entries = match read_element(rd, at, if first { 256 * 1024 } else { SEEK_HEAD_MAX }).await {
            Ok((SEEK_HEAD, data)) => seek_entries(&data),
            Ok(_) if first => continue,
            Ok((id, _)) => Err(anyhow::anyhow!("found {id:x}")),
            Err(e) => Err(e),
        };
        let entries = match entries {
            Ok(e) => e,
            // (Not a read failure: then the file may be fine and the index is tried again later.)
            Err(e) if !first && failure(&e) != Failure::Read => {
                tracing::debug!("second SeekHead at {at} not read: {e:#}");
                continue;
            }
            Err(e) => return Err(e),
        };
        for (target, p) in entries {
            let abs = segment_data + p;
            if target == SEEK_HEAD {
                seek_heads.push(abs);
            } else {
                positions.push((target, abs));
            }
        }
    }
    // No SeekHead, or one that doesn't list them: the top-level elements before the first
    // Cluster (Cues written ahead of the Clusters are found this way; ones after them aren't).
    if [INFO, TRACKS, CUES].iter().any(|w| !positions.iter().any(|(id, _)| id == w)) {
        for (id, at) in top_level(rd, segment_data).await? {
            if !positions.iter().any(|(i, _)| *i == id) {
                positions.push((id, at));
            }
        }
    }
    let find = |want: u32| positions.iter().find(|(id, _)| *id == want).map(|(_, p)| *p);

    let mut timescale = 1_000_000u64;
    if let Some(at) = find(INFO) {
        let (_, data) = read_element(rd, at, 64 * 1024).await?;
        for (id, body) in children(&data)? {
            if id == TIMESTAMP_SCALE {
                timescale = uint(body);
            }
        }
    }

    let mut tracks = Vec::new();
    if let Some(at) = find(TRACKS) {
        let (_, data) = read_element(rd, at, 1024 * 1024).await?;
        for (id, body) in children(&data)? {
            if id != TRACK_ENTRY {
                continue;
            }
            let mut t = TrackInfo {
                number: 0,
                kind: 0,
                codec: String::new(),
                encoded: false,
                default_duration: 0,
                encoding: Encoding::None,
                codec_private: Vec::new(),
            };
            for (tid, tbody) in children(body)? {
                match tid {
                    TRACK_NUMBER => t.number = uint(tbody),
                    TRACK_TYPE => t.kind = uint(tbody),
                    TRACK_CODEC => t.codec = String::from_utf8_lossy(tbody).trim_end_matches('\0').to_string(),
                    CONTENT_ENCODINGS => {
                        t.encoded = true;
                        t.encoding = Encoding::parse(tbody);
                    }
                    CODEC_PRIVATE => t.codec_private = tbody.to_vec(),
                    DEFAULT_DURATION => t.default_duration = uint(tbody),
                    _ => {}
                }
            }
            tracks.push(t);
        }
    }

    let cues_at = find(CUES);
    let mut cues = Vec::new();
    if let Some(at) = cues_at {
        let (id, data) = read_element(rd, at, 64 * 1024 * 1024).await?;
        if id != CUES {
            bail!("expected Cues at {at}, found {id:x}");
        }
        for (id, body) in children(&data)? {
            if id != CUE_POINT {
                continue;
            }
            let mut time = None;
            let mut points = Vec::new();
            for (cid, cbody) in children(body)? {
                match cid {
                    CUE_TIME => time = Some(uint(cbody)),
                    CUE_TRACK_POSITIONS => {
                        let mut p = CuePoint {
                            time: 0,
                            track: 0,
                            cluster: u64::MAX,
                            relative: None,
                            duration: None,
                        };
                        for (pid, pbody) in children(cbody)? {
                            match pid {
                                CUE_TRACK => p.track = uint(pbody),
                                CUE_CLUSTER_POSITION => p.cluster = uint(pbody),
                                CUE_RELATIVE_POSITION => p.relative = Some(uint(pbody)),
                                CUE_DURATION => p.duration = Some(uint(pbody)),
                                _ => {}
                            }
                        }
                        points.push(p);
                    }
                    _ => {}
                }
            }
            if let Some(t) = time {
                cues.extend(points.into_iter().map(|p| CuePoint { time: t, ..p }));
            }
        }
    }
    Ok(Index {
        segment_data,
        timescale,
        tracks,
        cues,
        cues_at,
    })
}

/// Video keyframe times in seconds, ascending, from the first video track's cue points.
pub async fn keyframes(source: &Source, http: &reqwest::Client) -> Result<Vec<f64>> {
    keyframes_with(&SourceRead(source, http)).await
}

async fn keyframes_with(rd: &impl RangeRead) -> Result<Vec<f64>> {
    let index = index_with(rd).await?;
    if index.cues_at.is_none() {
        return Err(NoCues("no Cues found".into()).into());
    }
    keyframes_of(&index)
}

fn keyframes_of(index: &Index) -> Result<Vec<f64>> {
    let video_track = index.tracks.iter().find(|t| t.kind == 1).map(|t| t.number);
    let mut times: Vec<f64> = Vec::new();
    // A cue point counts if one of its positions is the video track's (or any, without video).
    for c in &index.cues {
        if video_track.is_none_or(|v| c.track == v) {
            times.push(c.time as f64 * index.timescale as f64 / 1e9);
        }
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    times.dedup();
    if times.len() < 2 {
        return Err(NoCues(format!("too few cue points ({})", times.len())).into());
    }
    Ok(times)
}

/// The top-level elements from `at` up to the first Cluster (or one of unknown size, or what
/// can't be parsed): (id, position). At most 64. A read that fails is an error.
async fn top_level(rd: &impl RangeRead, mut at: u64) -> Result<Vec<(u32, u64)>> {
    let mut out = Vec::new();
    while out.len() < 64 {
        let head = read(rd, at, 16).await?;
        let mut r = Reader::new(&head);
        let Ok((id, Some(size))) = r.element() else { break };
        if id == CLUSTER {
            break;
        }
        out.push((id, at));
        at += r.pos as u64 + size;
    }
    Ok(out)
}

/// The largest SeekHead read after the first: one listing every Cluster of a long film
/// (about 15 bytes each) is well under this.
const SEEK_HEAD_MAX: u64 = 16 * 1024 * 1024;

/// A SeekHead's entries: (element id, position relative to the Segment's data).
fn seek_entries(data: &[u8]) -> Result<Vec<(u32, u64)>> {
    let mut out = Vec::new();
    for (cid, body) in children(data)? {
        if cid != SEEK {
            continue;
        }
        let mut target = 0u32;
        let mut pos = None;
        for (sid, sbody) in children(body)? {
            match sid {
                SEEK_ID => target = sbody.iter().fold(0u32, |a, b| (a << 8) | *b as u32),
                SEEK_POSITION => pos = Some(uint(sbody)),
                _ => {}
            }
        }
        if let Some(p) = pos {
            out.push((target, p));
        }
    }
    Ok(out)
}

/// The element at `at` (whole body, up to `max` bytes).
pub(crate) async fn read_element(rd: &impl RangeRead, at: u64, max: u64) -> Result<(u32, Vec<u8>)> {
    let head = read(rd, at, 16).await?;
    let mut r = Reader::new(&head);
    let (id, size) = r.element()?;
    if id == CLUSTER {
        bail!("expected index element, found Cluster");
    }
    let size = size.context("element of unknown size")?;
    if size > max {
        bail!("element {id:x} too large ({size})");
    }
    let body = read(rd, at + r.pos as u64, size).await?;
    Ok((id, body))
}

pub(crate) fn children(data: &[u8]) -> Result<Vec<(u32, &[u8])>> {
    let mut r = Reader::new(data);
    let mut out = Vec::new();
    while r.pos < data.len() {
        let (id, size) = r.element()?;
        let size = size.unwrap_or((data.len() - r.pos) as u64) as usize;
        let end = (r.pos + size).min(data.len());
        out.push((id, &data[r.pos..end]));
        r.pos = end;
    }
    Ok(out)
}

pub(crate) fn uint(b: &[u8]) -> u64 {
    b.iter().take(8).fold(0u64, |a, x| (a << 8) | *x as u64)
}

pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pub(crate) pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    fn byte(&mut self) -> Result<u8> {
        let b = *self.data.get(self.pos).context("truncated")?;
        self.pos += 1;
        Ok(b)
    }

    /// Element ID (marker bit kept) and data size (None: unknown size).
    pub(crate) fn element(&mut self) -> Result<(u32, Option<u64>)> {
        let first = self.byte()?;
        let len = first.leading_zeros() as usize + 1;
        if len > 4 {
            bail!("bad element id");
        }
        let mut id = first as u32;
        for _ in 1..len {
            id = (id << 8) | self.byte()? as u32;
        }
        let first = self.byte()?;
        let len = first.leading_zeros() as usize + 1;
        if len > 8 {
            bail!("bad size");
        }
        let mut size = (first as u64) & (0xFF >> len);
        let mut all_ones = size == (0xFF >> len) as u64;
        for _ in 1..len {
            let b = self.byte()?;
            all_ones &= b == 0xFF;
            size = (size << 8) | b as u64;
        }
        Ok((id, if all_ones { None } else { Some(size) }))
    }

    fn skip(&mut self, size: Option<u64>) -> Result<()> {
        let size = size.context("unknown size")? as usize;
        self.pos += size;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_encodings_of_picture_tracks() {
        // mkvmerge's default for picture subtitles: zlib (algorithm 0, or no algorithm given).
        let zlib = el(CONTENT_ENCODING, &el(CONTENT_COMPRESSION, &uel(CONTENT_COMP_ALGO, 0)));
        let e = Encoding::parse(&zlib);
        assert_eq!(e, Encoding::Zlib);
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut z, b"picture data").unwrap();
        assert_eq!(e.decode(z.finish().unwrap()).unwrap(), b"picture data");
        assert_eq!(
            Encoding::parse(&el(CONTENT_ENCODING, &el(CONTENT_COMPRESSION, &[]))),
            Encoding::Zlib
        );
        // Header stripping: the bytes go back on the front.
        let strip = el(
            CONTENT_ENCODING,
            &el(
                CONTENT_COMPRESSION,
                &[uel(CONTENT_COMP_ALGO, 3), el(CONTENT_COMP_SETTINGS, &[0x0F, 0x10])].concat(),
            ),
        );
        let e = Encoding::parse(&strip);
        assert_eq!(e, Encoding::Stripped(vec![0x0F, 0x10]));
        assert_eq!(e.decode(vec![1, 2]).unwrap(), [0x0F, 0x10, 1, 2]);
        // Encrypted: not readable here.
        let enc = el(CONTENT_ENCODING, &el(CONTENT_ENCRYPTION, &[]));
        assert_eq!(Encoding::parse(&enc), Encoding::Other);
        assert!(Encoding::Other.decode(vec![1]).is_err());
    }

    /// A file in memory; reads past the end come back short, as from a file.
    struct Mem(Vec<u8>);

    impl RangeRead for Mem {
        fn read_range(&self, offset: u64, len: u64) -> futures_util::future::BoxFuture<'_, Result<Vec<u8>>> {
            let start = (offset as usize).min(self.0.len());
            let end = (offset.saturating_add(len) as usize).min(self.0.len());
            Box::pin(async move { Ok(self.0[start..end].to_vec()) })
        }
    }

    /// `Mem` whose read number `fail` (from 0) fails, as a share that drops out briefly does.
    struct Flaky(Vec<u8>, usize, std::sync::atomic::AtomicUsize);

    impl RangeRead for Flaky {
        fn read_range(&self, offset: u64, len: u64) -> futures_util::future::BoxFuture<'_, Result<Vec<u8>>> {
            if self.2.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == self.1 {
                return Box::pin(async { Err(anyhow::anyhow!("STATUS_IO_TIMEOUT")) });
            }
            let start = (offset as usize).min(self.0.len());
            let end = (offset.saturating_add(len) as usize).min(self.0.len());
            Box::pin(async move { Ok(self.0[start..end].to_vec()) })
        }
    }

    /// Fails each read of `f` in turn: every failure is a read failure, never damage. Returns
    /// how many reads there were.
    fn each_read_fails(f: &[u8]) -> usize {
        for n in 0.. {
            match block_on(keyframes_with(&Flaky(f.to_vec(), n, std::sync::atomic::AtomicUsize::default()))) {
                Ok(_) => return n,
                Err(e) => {
                    assert_eq!(failure(&e), Failure::Read, "read {n}: {e:#}");
                    assert!(format!("{e:#}").contains("STATUS_IO_TIMEOUT"), "{e:#}");
                }
            }
        }
        unreachable!()
    }

    fn id_bytes(id: u32) -> Vec<u8> {
        let b = id.to_be_bytes();
        let skip = b.iter().take_while(|x| **x == 0).count();
        b[skip..].to_vec()
    }

    /// An element with a 1-byte size where it fits (and `wide` isn't asked), else an 8-byte one.
    fn el_with(id: u32, body: &[u8], wide: bool) -> Vec<u8> {
        let mut out = id_bytes(id);
        if body.len() < 127 && !wide {
            out.push(0x80 | body.len() as u8);
        } else {
            out.push(0x01);
            out.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
        }
        out.extend_from_slice(body);
        out
    }

    fn el(id: u32, body: &[u8]) -> Vec<u8> {
        el_with(id, body, false)
    }

    fn uel(id: u32, v: u64) -> Vec<u8> {
        let b = v.to_be_bytes();
        let skip = b.iter().take_while(|x| **x == 0).count().min(7);
        el(id, &b[skip..])
    }

    const VOID: u32 = 0xEC;
    const CRC32: u32 = 0xBF;

    fn seek(id: u32, pos: u64) -> Vec<u8> {
        // Fixed-width position (8 bytes), so a SeekHead's size doesn't depend on where things land.
        el(SEEK, &[el(SEEK_ID, &id_bytes(id)), el(SEEK_POSITION, &pos.to_be_bytes())].concat())
    }

    fn info() -> Vec<u8> {
        el(INFO, &uel(TIMESTAMP_SCALE, 1_000_000))
    }

    fn tracks() -> Vec<u8> {
        let video = el(
            TRACK_ENTRY,
            &[uel(TRACK_NUMBER, 1), uel(TRACK_TYPE, 1), el(TRACK_CODEC, b"V_MPEG4/ISO/AVC")].concat(),
        );
        let audio = el(
            TRACK_ENTRY,
            &[uel(TRACK_NUMBER, 2), uel(TRACK_TYPE, 2), el(TRACK_CODEC, b"A_AC3")].concat(),
        );
        el(TRACKS, &[video, audio].concat())
    }

    /// Cue points at `times` (ms) for track 1, each also with a position for track 2, a
    /// CueRelativePosition, and Void/CRC-32 elements in between.
    fn cues(times: &[u64]) -> Vec<u8> {
        let mut body = el(CRC32, &[0, 0, 0, 0]);
        for (i, t) in times.iter().enumerate() {
            let pos = |track: u64| {
                el(
                    CUE_TRACK_POSITIONS,
                    &[
                        uel(CUE_TRACK, track),
                        uel(CUE_CLUSTER_POSITION, 1000 + i as u64),
                        uel(CUE_RELATIVE_POSITION, 5),
                    ]
                    .concat(),
                )
            };
            body.extend(el(CUE_POINT, &[uel(CUE_TIME, *t), el(VOID, &[0; 3]), pos(1), pos(2)].concat()));
        }
        body.extend(el(VOID, &[0; 10]));
        el(CUES, &body)
    }

    fn cluster() -> Vec<u8> {
        el(CLUSTER, &[uel(0xE7, 0), el(0xA3, &[0x81, 0, 0, 0x80, 1, 2, 3])].concat())
    }

    fn ebml_header() -> Vec<u8> {
        el(0x1A45DFA3, &el(0x4282, b"matroska"))
    }

    /// A file: EBML header, then a Segment of unknown size holding `parts`.
    fn file(parts: &[Vec<u8>]) -> (Vec<u8>, u64) {
        let mut out = ebml_header();
        out.extend(id_bytes(SEGMENT));
        out.push(0x01);
        out.extend([0xFF; 7]);
        let data = out.len() as u64;
        for p in parts {
            out.extend_from_slice(p);
        }
        (out, data)
    }

    /// The usual mkvmerge layout: SeekHead, Void, Info, Tracks, Clusters, Cues, then a second
    /// SeekHead of `second_len` bytes (Void padding standing in for its Cluster entries).
    /// `cues_in_first`: the first SeekHead lists the Cues (else the second does).
    fn mkvmerge_layout(times: &[u64], second_len: usize, cues_in_first: bool) -> Vec<u8> {
        let first_len = el(
            SEEK_HEAD,
            &[seek(INFO, 0), seek(TRACKS, 0), seek(CUES, 0), seek(SEEK_HEAD, 0)].concat(),
        )
        .len();
        let void = el(VOID, &[0; 40]);
        let info = info();
        let tracks = tracks();
        let clusters = [cluster(), cluster()].concat();
        let cues = cues(times);
        let at_info = (first_len + void.len()) as u64;
        let at_tracks = at_info + info.len() as u64;
        let at_cues = at_tracks + tracks.len() as u64 + clusters.len() as u64;
        let at_second = at_cues + cues.len() as u64;
        let mut first = vec![seek(INFO, at_info), seek(TRACKS, at_tracks), seek(SEEK_HEAD, at_second)];
        let mut second = vec![seek(CLUSTER, at_tracks + tracks.len() as u64)];
        if cues_in_first {
            first.insert(2, seek(CUES, at_cues));
        } else {
            first.insert(2, el(VOID, &[0; 19])); // as long as a Seek
            second.push(seek(CUES, at_cues));
        }
        let first = el(SEEK_HEAD, &first.concat());
        assert_eq!(first.len(), first_len);
        let used: usize = second.iter().map(Vec::len).sum();
        second.push(el_with(VOID, &vec![0; second_len.saturating_sub(used + 9 + 4 + 9)], true));
        let second = el_with(SEEK_HEAD, &second.concat(), true);
        file(&[first, void, info, tracks, clusters, cues, second]).0
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(f)
    }

    const TIMES: [u64; 4] = [0, 2002, 4004, 6006];

    #[test]
    fn reads_cues_with_void_crc_relative_and_second_track() {
        let f = mkvmerge_layout(&TIMES, 100, true);
        let index = block_on(index_with(&Mem(f.clone()))).unwrap();
        assert_eq!(index.tracks.len(), 2);
        assert_eq!(index.cues.len(), 8);
        assert!(index.cues.iter().all(|c| c.relative == Some(5)));
        assert_eq!(block_on(keyframes_with(&Mem(f))).unwrap(), vec![0.0, 2.002, 4.004, 6.006]);
    }

    /// The Fifth Element, Avatar Extended: the second SeekHead (every Cluster) is over 256 KiB.
    #[test]
    fn large_second_seek_head() {
        let f = mkvmerge_layout(&TIMES, 400 * 1024, true);
        assert_eq!(block_on(keyframes_with(&Mem(f))).unwrap(), vec![0.0, 2.002, 4.004, 6.006]);
        // Only that one lists the Cues: it is read too.
        let f = mkvmerge_layout(&TIMES, 400 * 1024, false);
        assert_eq!(block_on(keyframes_with(&Mem(f))).unwrap(), vec![0.0, 2.002, 4.004, 6.006]);
    }

    /// Star Trek 2009, the Pirates films, Nemesis, Terminator 3: the file's end is zeros, where
    /// the Cues and the second SeekHead should be. The file is damaged (the server says so).
    #[test]
    fn zeroed_tail_is_an_error_not_garbage() {
        let mut f = mkvmerge_layout(&TIMES, 100 * 1024, true);
        // From the Cues on (the last place their id appears; the first SeekHead names it too).
        let id = id_bytes(CUES);
        let at = f.windows(4).rposition(|w| w == id.as_slice()).unwrap();
        f[at..].fill(0);
        let e = block_on(keyframes_with(&Mem(f.clone()))).unwrap_err();
        assert!(format!("{e:#}").contains("bad element id"), "{e:#}");
        assert_eq!(failure(&e), Failure::Damaged);
        // Cut short instead of zeroed: damaged too.
        f.truncate(at);
        let e = block_on(keyframes_with(&Mem(f))).unwrap_err();
        assert_eq!(failure(&e), Failure::Damaged, "{e:#}");
        // Zeros from the start (Avatar: The Way of Water 720p).
        let e = block_on(keyframes_with(&Mem(vec![0; 200_000]))).unwrap_err();
        assert_eq!(failure(&e), Failure::Damaged, "{e:#}");
    }

    /// A read that fails (the share, not the file) is never taken for damage: the index is
    /// tried again later. Wherever it fails: the head, the first SeekHead, the Tracks, the
    /// second SeekHead (which a damaged one would be passed over for), the Cues.
    #[test]
    fn read_failures_are_not_damage() {
        // Head, first SeekHead, second SeekHead, Info, Tracks, Cues: 2 reads each but the head.
        assert!(each_read_fails(&mkvmerge_layout(&TIMES, 100 * 1024, false)) >= 9);
        assert!(each_read_fails(&mkvmerge_layout(&TIMES, 400 * 1024, true)) >= 9);
        // The walk of the top-level elements (no SeekHead).
        let (f, _) = file(&[el(VOID, &[0; 8]), info(), tracks(), cues(&TIMES), cluster()]);
        assert!(each_read_fails(&f) >= 8);
        // An error wrapped further up still says so.
        let e = block_on(keyframes_with(&Flaky(f, 0, std::sync::atomic::AtomicUsize::default())))
            .unwrap_err()
            .context("index");
        assert_eq!(failure(&e), Failure::Read);
    }

    #[test]
    fn seek_position_pointing_elsewhere_is_an_error() {
        let info = info();
        let tracks = tracks();
        let first_len = el(SEEK_HEAD, &[seek(INFO, 0), seek(TRACKS, 0), seek(CUES, 0)].concat()).len() as u64;
        let at_info = first_len;
        let at_tracks = at_info + info.len() as u64;
        // The Cues "position" is the Tracks.
        let head = el(
            SEEK_HEAD,
            &[seek(INFO, at_info), seek(TRACKS, at_tracks), seek(CUES, at_tracks)].concat(),
        );
        let (f, _) = file(&[head, info, tracks, cluster()]);
        let e = block_on(keyframes_with(&Mem(f))).unwrap_err();
        assert!(format!("{e:#}").contains("expected Cues"), "{e:#}");
        assert_eq!(failure(&e), Failure::Damaged);
    }

    /// No SeekHead at all, Cues ahead of the Clusters (and 8-byte sizes): found by walking the
    /// top-level elements.
    #[test]
    fn no_seek_head_cues_before_clusters() {
        let wide_info = el_with(INFO, &uel(TIMESTAMP_SCALE, 1_000_000), true);
        let (f, _) = file(&[el(VOID, &[0; 8]), wide_info, tracks(), cues(&TIMES), cluster(), cluster()]);
        assert_eq!(block_on(keyframes_with(&Mem(f))).unwrap(), vec![0.0, 2.002, 4.004, 6.006]);
        // Cues after the Clusters, and no SeekHead: not found (not served, but not damaged).
        let (f, _) = file(&[info(), tracks(), cluster(), cues(&TIMES)]);
        let e = block_on(keyframes_with(&Mem(f))).unwrap_err();
        assert!(format!("{e:#}").contains("no Cues"), "{e:#}");
        assert_eq!(failure(&e), Failure::NoCues);
        // One cue point: too few to cut on.
        let (f, _) = file(&[info(), tracks(), cues(&[0]), cluster()]);
        assert_eq!(failure(&block_on(keyframes_with(&Mem(f))).unwrap_err()), Failure::NoCues);
    }

    /// A SeekHead that leaves the Tracks out: they are found by the walk; what it lists wins.
    #[test]
    fn walk_fills_in_only_what_the_seek_head_leaves_out() {
        let info = info();
        let tracks = tracks();
        let first_len = el(SEEK_HEAD, &[seek(INFO, 0), seek(CUES, 0)].concat()).len() as u64;
        let at_cues = first_len + info.len() as u64 + tracks.len() as u64;
        let head = el(SEEK_HEAD, &[seek(INFO, first_len), seek(CUES, at_cues)].concat());
        let (f, data) = file(&[head, info, tracks, cues(&TIMES), cluster()]);
        let index = block_on(index_with(&Mem(f))).unwrap();
        assert_eq!(index.tracks.len(), 2);
        assert_eq!(index.cues_at, Some(data + at_cues));
    }

    #[test]
    fn ids_and_sizes() {
        // 4-byte id, 8-byte size; unknown size (all ones) in 1 and 8 bytes.
        let mut r = Reader::new(&[0x1A, 0x45, 0xDF, 0xA3, 0x01, 0, 0, 0, 0, 0, 0, 0x05]);
        assert_eq!(r.element().unwrap(), (0x1A45DFA3, Some(5)));
        assert_eq!(Reader::new(&[0xEC, 0xFF]).element().unwrap(), (0xEC, None));
        assert_eq!(
            Reader::new(&[0xEC, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])
                .element()
                .unwrap(),
            (0xEC, None)
        );
        assert!(Reader::new(&[0x00, 0x81]).element().is_err());
    }
}
