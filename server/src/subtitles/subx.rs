//! Text subtitles read straight out of a Matroska file, without ffmpeg and without reading the
//! whole file.
//!
//! - **By the index** (most files: mkvmerge lists every subtitle block in the Cues, with its
//!   position inside the cluster): only those blocks are read, a few KB each, neighbours merged
//!   into one read. The media share answers small reads at ~90 a second (a NAS disk's seeks),
//!   so a film's tracks take seconds instead of a full read of tens of GB.
//! - **By scanning** (the Cues list only the video): the clusters are read in order and only the
//!   subtitle blocks parsed, from up to three places in the file at once (the clusters the video
//!   cues point at split it), which reads the share faster than one stream does.
//!
//! Either way the reading is paced by the caller (`Pace`): it is asked before each read, so a
//! more urgent extraction can pause this one, and background work can be slowed down.

use std::fmt::Write as _;
use std::{collections::HashMap, os::unix::fs::FileExt, path::Path, sync::Arc, time::Duration};

use anyhow::{bail, Context, Result};
use futures_util::future::BoxFuture;

use crate::media::mkv::{self, Index, Reader};

const CLUSTER: u32 = 0x1F43B675;
const CLUSTER_TIMESTAMP: u32 = 0xE7;
const SIMPLE_BLOCK: u32 = 0xA3;
const BLOCK_GROUP: u32 = 0xA0;
const BLOCK: u32 = 0xA1;
const BLOCK_DURATION: u32 = 0x9B;

/// Blocks closer than this are fetched in one read (a read costs about the same up to here).
const MERGE: u64 = 1024 * 1024;
/// Bytes read past a subtitle block's position (the block itself is rarely larger).
const BLOCK_READ: u64 = 4096 + 32;
/// Read size when scanning.
const CHUNK: u64 = 4 * 1024 * 1024;

/// One cue, in seconds of the file's own time.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// A track to extract: ffmpeg's stream index (the output's name) and its Matroska track.
#[derive(Debug, Clone)]
pub struct Want {
    pub stream: i64,
    pub track: u64,
    pub codec: String,
    pub default_duration: u64,
}

/// How the reading may go on: asked before every read. It waits while the caller says so (a
/// more urgent job runs), and answers how many reads may be in flight and how long to rest
/// after each; an error stops the extraction.
pub type Pace<'a> = &'a (dyn Fn() -> BoxFuture<'static, Result<Step>> + Send + Sync);

#[derive(Debug, Clone, Copy)]
pub struct Step {
    pub parallel: usize,
    pub rest: Duration,
    /// Bytes a second when scanning (0: as fast as it goes).
    pub rate: u64,
}

/// The file opened once (each `open` over CIFS is a round trip of its own): a local file, or a
/// file on an SMB share (its handle kept by the SMB client).
#[derive(Clone)]
pub struct File {
    f: Inner,
}

#[derive(Clone)]
enum Inner {
    Local(Arc<std::fs::File>),
    Smb(crate::storage::smb::SmbPath, u64),
}

impl File {
    pub fn open(path: &Path, random: bool) -> Result<File> {
        let f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
        // Small scattered reads: no readahead (it would only add bytes to every seek).
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            let advice = if random {
                libc::POSIX_FADV_RANDOM
            } else {
                libc::POSIX_FADV_SEQUENTIAL
            };
            // SAFETY: posix_fadvise only takes integers; the descriptor is `f`'s, open for the call.
            unsafe {
                libc::posix_fadvise(f.as_raw_fd(), 0, 0, advice);
            }
        }
        // macOS: F_RDAHEAD off for scattered reads (on by default).
        #[cfg(target_os = "macos")]
        if random {
            use std::os::fd::AsRawFd;
            // SAFETY: fcntl(F_RDAHEAD) only takes integers; the descriptor is `f`'s, open for the call.
            unsafe {
                libc::fcntl(f.as_raw_fd(), libc::F_RDAHEAD, 0);
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let _ = random;
        Ok(File {
            f: Inner::Local(Arc::new(f)),
        })
    }

    /// `open` off the async threads (an open over CIFS can take a while when the share is busy).
    pub async fn open_async(path: &str, random: bool) -> Result<File> {
        if let Some(p) = crate::storage::smb::SmbPath::parse(path) {
            let size = crate::storage::smb::client().file_meta(&p).await?.size;
            return Ok(File { f: Inner::Smb(p, size) });
        }
        let path = std::path::PathBuf::from(path);
        tokio::task::spawn_blocking(move || File::open(&path, random)).await?
    }

    pub async fn read(&self, offset: u64, len: u64) -> Result<Vec<u8>> {
        let f = match &self.f {
            Inner::Local(f) => f.clone(),
            Inner::Smb(p, _) => return Ok(crate::storage::smb::client().read_at(p, offset, len).await?.to_vec()),
        };
        tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
            let mut buf = vec![0u8; len as usize];
            let mut got = 0usize;
            while got < buf.len() {
                let n = f.read_at(&mut buf[got..], offset + got as u64)?;
                if n == 0 {
                    break;
                }
                got += n;
            }
            buf.truncate(got);
            Ok(buf)
        })
        .await?
    }

    fn len(&self) -> u64 {
        match &self.f {
            Inner::Local(f) => f.metadata().map_or(u64::MAX, |m| m.len()),
            Inner::Smb(_, size) => *size,
        }
    }
}

impl mkv::RangeRead for File {
    fn read_range(&self, offset: u64, len: u64) -> BoxFuture<'_, Result<Vec<u8>>> {
        Box::pin(self.read(offset, len))
    }
}

/// Which way a file's tracks can be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Every wanted track's blocks are in the index.
    Cues,
    /// Scan the clusters.
    Scan,
}

/// The tracks for the streams asked for (ffmpeg numbers a Matroska file's streams in track
/// order), checked against what the file says; None when they don't line up (ffmpeg then).
pub fn wants(index: &Index, streams: &[i64]) -> Option<Vec<Want>> {
    streams
        .iter()
        .map(|&s| {
            let t = index.tracks.get(usize::try_from(s).ok()?)?;
            let text = matches!(
                t.codec.as_str(),
                "S_TEXT/UTF8" | "S_TEXT/ASCII" | "S_TEXT/ASS" | "S_TEXT/SSA" | "S_TEXT/WEBVTT"
            );
            (t.kind == 17 && text && !t.encoded).then(|| Want {
                stream: s,
                track: t.number,
                codec: t.codec.clone(),
                default_duration: t.default_duration,
            })
        })
        .collect()
}

pub fn method(index: &Index, wants: &[Want]) -> Method {
    let all_indexed = wants.iter().all(|w| {
        let mut n = 0;
        for c in index.cues.iter().filter(|c| c.track == w.track) {
            if c.relative.is_none() || c.cluster == u64::MAX {
                return false;
            }
            n += 1;
        }
        n > 0
    });
    if all_indexed {
        Method::Cues
    } else {
        Method::Scan
    }
}

/// A block as found in the file: its track, start (timestamp units), duration and payload.
struct Found {
    track: u64,
    time: i64,
    duration: Option<u64>,
    data: Vec<u8>,
}

/// Every wanted track's cues, by stream.
pub async fn extract(file: &File, index: &Index, wants: &[Want], pace: Pace<'_>) -> Result<(Method, HashMap<i64, Vec<Cue>>)> {
    extract_with(file, index, wants, pace, method(index, wants)).await
}

pub async fn extract_with(
    file: &File,
    index: &Index,
    wants: &[Want],
    pace: Pace<'_>,
    m: Method,
) -> Result<(Method, HashMap<i64, Vec<Cue>>)> {
    let found = match m {
        Method::Cues => by_cues(file, index, wants, pace).await?,
        Method::Scan => by_scan(file, index, wants, pace).await?,
    };
    Ok((m, to_cues(index, wants, found)))
}

async fn by_cues(file: &File, index: &Index, wants: &[Want], pace: Pace<'_>) -> Result<Vec<Found>> {
    let targets: Vec<mkv::CuePoint> = index
        .cues
        .iter()
        .filter(|c| wants.iter().any(|w| w.track == c.track))
        .copied()
        .collect();
    Ok(read_blocks(file, index, targets, None, pace).await?.0)
}

/// The subtitle blocks the index points at: (blocks, the cluster header length learned).
/// `header`: the length learned before for this file (else one read learns it).
async fn read_blocks(
    file: &File,
    index: &Index,
    targets: Vec<mkv::CuePoint>,
    header: Option<u64>,
    pace: Pace<'_>,
) -> Result<(Vec<Found>, u64)> {
    use futures_util::StreamExt;
    // The cluster's header (ID and size) lies between the cluster's position and the block's
    // relative position: learn its length from the first cluster, check it on every block.
    let mut targets: Vec<(u64, mkv::CuePoint)> = targets
        .into_iter()
        .map(|c| (index.segment_data + c.cluster + c.relative.unwrap_or(0), c))
        .collect();
    targets.sort_by_key(|t| t.0);
    targets.dedup_by_key(|t| t.0);
    let Some(first) = targets.first() else {
        return Ok((Vec::new(), header.unwrap_or(12)));
    };
    let header = match header {
        Some(h) => h,
        None => {
            let at = index.segment_data + first.1.cluster;
            let b = file.read(at, 16).await?;
            let mut r = Reader::new(&b);
            let (id, _) = r.element()?;
            if id != CLUSTER {
                bail!("cue points at no cluster");
            }
            r.pos as u64
        }
    };
    // Reads: runs of targets within MERGE of the run's start.
    let mut windows: Vec<(u64, u64, Vec<(u64, mkv::CuePoint)>)> = Vec::new();
    for t in targets {
        match windows.last_mut() {
            Some(w) if t.0 + BLOCK_READ + 12 <= w.0 + MERGE => {
                w.1 = t.0 + BLOCK_READ + 12;
                w.2.push(t);
            }
            _ => windows.push((t.0, t.0 + BLOCK_READ + 12, vec![t])),
        }
    }
    let mut found = Vec::new();
    let mut clusters: HashMap<u64, i64> = HashMap::new();
    let mut pending = futures_util::stream::FuturesOrdered::new();
    let mut it = windows.into_iter();
    loop {
        let step = pace().await?;
        while pending.len() < step.parallel.max(1) {
            let Some((start, end, items)) = it.next() else { break };
            let f = file.clone();
            pending.push_back(async move { f.read(start, end - start).await.map(|b| (start, b, items)) });
        }
        let Some(r) = pending.next().await else { break };
        let (start, buf, items) = r?;
        for (approx, cue) in items {
            let block = match parse_at(&buf, (approx - start) as usize, header, cue.track) {
                Some(Ok(b)) => b,
                // Larger than what was read: read it whole.
                Some(Err(need)) => {
                    let more = file.read(approx, need as u64 + 16).await?;
                    match parse_at(&more, 0, header, cue.track) {
                        Some(Ok(b)) => b,
                        _ => bail!("subtitle block at {approx} unreadable"),
                    }
                }
                None => bail!("no subtitle block of track {} where the index says (at {approx})", cue.track),
            };
            // The cue time is the block's own (mkvmerge): then every block of a cluster agrees
            // on the cluster's timestamp.
            let cluster_ts = cue.time as i64 - block.2 as i64;
            match clusters.insert(cue.cluster, cluster_ts) {
                Some(other) if other != cluster_ts => bail!("cue times aren't the blocks' own (cluster at {})", cue.cluster),
                _ => {}
            }
            found.push(Found {
                track: cue.track,
                time: cue.time as i64,
                duration: block.1.or(cue.duration),
                data: block.0,
            });
        }
        if !step.rest.is_zero() {
            tokio::time::sleep(step.rest).await;
        }
    }
    Ok((found, header))
}

/// Reads as fast as the share answers (a client is waiting for this window).
fn fast() -> BoxFuture<'static, Result<Step>> {
    Box::pin(async {
        Ok(Step {
            parallel: 8,
            rest: Duration::ZERO,
            rate: 0,
        })
    })
}

/// Cues of `want` showing during [from, to), by the index (`Method::Cues`): only the blocks
/// starting before `to` and still showing at `from` (by their cue duration; without one, those
/// starting at most `lead` seconds before `from`). Returns the cues and the cluster header length.
pub async fn window_by_cues(
    file: &File,
    index: &Index,
    want: &Want,
    from: f64,
    to: f64,
    lead: f64,
    header: Option<u64>,
) -> Result<(Vec<Cue>, u64)> {
    let unit = index.timescale as f64 / 1e9;
    let targets: Vec<mkv::CuePoint> = index
        .cues
        .iter()
        .filter(|c| c.track == want.track)
        .filter(|c| {
            let t = c.time as f64 * unit;
            if t >= to {
                return false;
            }
            match c.duration {
                Some(d) => t + d as f64 * unit > from,
                None => t >= from - lead,
            }
        })
        .copied()
        .collect();
    let (found, header) = read_blocks(file, index, targets, header, &fast).await?;
    let cues = to_cues(index, std::slice::from_ref(want), found)
        .remove(&want.stream)
        .unwrap_or_default();
    Ok((overlapping(cues, from, to), header))
}

/// Cues of every `wants` track showing during [from, to), by scanning the clusters of that
/// stretch of the file (the index has only the video's cue points): from the keyframe `lead`
/// seconds before `from` to the one a little past `to`, in up to `parts` pieces read at once,
/// each read asking `pace` first.
#[allow(
    clippy::too_many_arguments,
    reason = "the steps' inputs, passed through; a struct for one call would only rename them"
)]
pub async fn window_by_scan(
    file: &File,
    index: &Index,
    wants: &[Want],
    from: f64,
    to: f64,
    lead: f64,
    parts: usize,
    pace: Pace<'_>,
) -> Result<(HashMap<i64, Vec<Cue>>, u64)> {
    let (found, bytes) = scan_found(file, index, wants, from, to, lead, parts, pace).await?;
    let cues = to_cues(index, wants, found)
        .into_iter()
        .map(|(s, c)| (s, overlapping(c, from, to)))
        .collect();
    Ok((cues, bytes))
}

/// The blocks of every `wants` track in the clusters from the keyframe `lead` seconds before
/// `from` to the one a little past `to`, read in up to `parts` pieces at once, and the bytes read.
#[allow(
    clippy::too_many_arguments,
    reason = "the steps' inputs, passed through; a struct for one call would only rename them"
)]
async fn scan_found(
    file: &File,
    index: &Index,
    wants: &[Want],
    from: f64,
    to: f64,
    lead: f64,
    parts: usize,
    pace: Pace<'_>,
) -> Result<(Vec<Found>, u64)> {
    let mut points: Vec<(f64, u64)> = index
        .cues
        .iter()
        .filter(|c| c.cluster != u64::MAX)
        .map(|c| (index.secs(c.time as i64), index.segment_data + c.cluster))
        .collect();
    points.sort_by_key(|p| p.1);
    points.dedup_by_key(|p| p.1);
    if points.is_empty() {
        bail!("no cluster positions in the index");
    }
    let len = file.len();
    let start = points.iter().rev().find(|p| p.0 <= from - lead).map_or(points[0].1, |p| p.1);
    // A block shows in the cluster of its time: stop at a cluster that starts clearly later.
    let end = points.iter().find(|p| p.0 >= to + 2.0 && p.1 > start).map_or(len, |p| p.1);
    // Split at cluster positions into parts of about equal size.
    let inside: Vec<u64> = points.iter().map(|p| p.1).filter(|&p| p > start && p < end).collect();
    let n = parts.clamp(1, 4).min(inside.len() + 1);
    let mut cuts = vec![start];
    for k in 1..n {
        let want = start + (end - start) * k as u64 / n as u64;
        if let Some(&p) = inside.iter().find(|&&p| p >= want) {
            if p > *cuts.last().unwrap() {
                cuts.push(p);
            }
        }
    }
    cuts.push(end);
    let tracks: Vec<u64> = wants.iter().map(|w| w.track).collect();
    let results = futures_util::future::join_all(cuts.windows(2).map(|r| scan_range(file, r[0], r[1], len, &tracks, pace, 1))).await;
    let mut found = Vec::new();
    for r in results {
        found.extend(r?);
    }
    Ok((found, end - start))
}

/// Matroska codecs of picture subtitles (`bitmap.rs` decodes them).
pub const PICTURE_CODECS: [&str; 3] = ["S_HDMV/PGS", "S_VOBSUB", "S_DVBSUB"];

/// A picture subtitle block: its time and duration in seconds of the file's own time, and its
/// payload as the codec has it (the track's compression undone).
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub time: f64,
    pub duration: Option<f64>,
    pub data: Vec<u8>,
}

/// The picture subtitle track of ffmpeg's stream `stream`, if the file's track list lines up
/// and its blocks can be read here.
pub fn picture_want(index: &Index, stream: i64) -> Option<Want> {
    let t = index.tracks.get(usize::try_from(stream).ok()?)?;
    (t.kind == 17 && PICTURE_CODECS.contains(&t.codec.as_str()) && t.encoding != mkv::Encoding::Other).then(|| Want {
        stream,
        track: t.number,
        codec: t.codec.clone(),
        default_duration: t.default_duration,
    })
}

/// The blocks of a picture track starting during [from, to), by the index (`Method::Cues`), and
/// the cluster header length learned.
pub async fn picture_blocks_by_cues(
    file: &File,
    index: &Index,
    want: &Want,
    from: f64,
    to: f64,
    header: Option<u64>,
) -> Result<(Vec<Block>, u64)> {
    let targets: Vec<mkv::CuePoint> = index
        .cues
        .iter()
        .filter(|c| c.track == want.track)
        .filter(|c| {
            let t = index.secs(c.time as i64);
            t >= from && t < to
        })
        .copied()
        .collect();
    let (found, header) = read_blocks(file, index, targets, header, &fast).await?;
    Ok((to_blocks(index, want, found)?, header))
}

/// The blocks of a picture track starting during [from, to), by scanning that stretch of the
/// file, and the bytes read.
pub async fn picture_blocks_by_scan(
    file: &File,
    index: &Index,
    want: &Want,
    from: f64,
    to: f64,
    pace: Pace<'_>,
) -> Result<(Vec<Block>, u64)> {
    let (found, bytes) = scan_found(file, index, std::slice::from_ref(want), from, to, 0.0, 2, pace).await?;
    let blocks = to_blocks(index, want, found)?
        .into_iter()
        .filter(|b| b.time >= from && b.time < to)
        .collect();
    Ok((blocks, bytes))
}

/// A picture track's blocks in time order, decompressed.
fn to_blocks(index: &Index, want: &Want, mut found: Vec<Found>) -> Result<Vec<Block>> {
    let encoding = index
        .tracks
        .iter()
        .find(|t| t.number == want.track)
        .map(|t| t.encoding.clone())
        .unwrap_or_default();
    found.retain(|f| f.track == want.track);
    found.sort_by_key(|f| f.time);
    found
        .into_iter()
        .map(|f| {
            Ok(Block {
                time: index.secs(f.time),
                duration: f.duration.map(|d| index.secs(d as i64)).filter(|d| *d > 0.0),
                data: encoding.decode(f.data)?,
            })
        })
        .collect()
}

/// The bytes of each span (offset, length), in the order given: spans near each other are read
/// together, `parallel` reads in flight.
pub async fn read_spans(file: &File, spans: &[(u64, u64)], parallel: usize) -> Result<Vec<Vec<u8>>> {
    use futures_util::StreamExt;
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|&i| spans[i].0);
    let mut reads: Vec<(u64, u64, Vec<usize>)> = Vec::new();
    for i in order {
        let (at, len) = spans[i];
        match reads.last_mut() {
            Some(r) if at + len <= r.0 + MERGE => {
                r.1 = r.1.max(at + len);
                r.2.push(i);
            }
            _ => reads.push((at, at + len, vec![i])),
        }
    }
    let done: Vec<Result<(u64, Vec<u8>, Vec<usize>)>> = futures_util::stream::iter(reads.into_iter().map(|(a, b, items)| {
        let f = file.clone();
        async move { f.read(a, b - a).await.map(|buf| (a, buf, items)) }
    }))
    .buffer_unordered(parallel.max(1))
    .collect()
    .await;
    let mut out = vec![Vec::new(); spans.len()];
    for r in done {
        let (a, buf, items) = r?;
        for i in items {
            let s = (spans[i].0 - a) as usize;
            out[i] = buf.get(s..(s + spans[i].1 as usize).min(buf.len())).unwrap_or_default().to_vec();
        }
    }
    Ok(out)
}

/// The cues showing at some time in [from, to).
pub fn overlapping(cues: Vec<Cue>, from: f64, to: f64) -> Vec<Cue> {
    cues.into_iter().filter(|c| c.end > from && c.start < to).collect()
}

/// The block of `track` at `buf[pos + h..]` for the cluster header length `h` (the learned one
/// first, then any other): (payload, duration). `Err(n)`: it needs `n` bytes from `pos`.
fn parse_at(buf: &[u8], pos: usize, header: u64, track: u64) -> Option<std::result::Result<(Vec<u8>, Option<u64>, i16), usize>> {
    let order = std::iter::once(header as usize).chain((0..=16).filter(|h| *h != header as usize));
    for h in order {
        let at = pos + h;
        let Some(slice) = buf.get(at..) else { continue };
        let mut r = Reader::new(slice);
        let Ok((id, Some(size))) = r.element() else { continue };
        if (id != SIMPLE_BLOCK && id != BLOCK_GROUP) || size > 1 << 20 {
            continue;
        }
        let body_at = r.pos;
        let end = body_at + size as usize;
        // Check the track number before deciding it's too short.
        let peek = &slice[body_at..slice.len().min(end)];
        let tn = if id == SIMPLE_BLOCK {
            block_track(peek)
        } else {
            mkv::children(peek)
                .ok()
                .and_then(|c| c.into_iter().find(|(i, _)| *i == BLOCK).and_then(|(_, b)| block_track(b)))
        };
        if end > slice.len() {
            // A BlockGroup cut short may not show its Block yet: trust the ID.
            if tn.is_none_or(|t| t == track) {
                return Some(Err(h + end));
            }
            continue;
        }
        if tn != Some(track) {
            continue;
        }
        let body = &slice[body_at..end];
        return parse_block(id, body)
            .map(|(t, rel, d, data)| (t == track).then_some(Ok((data, d, rel))))
            .ok()
            .flatten();
    }
    None
}

fn block_track(b: &[u8]) -> Option<u64> {
    let first = *b.first()?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || b.len() < len {
        return None;
    }
    let mut v = (first as u64) & (0xFF >> len);
    for x in &b[1..len] {
        v = (v << 8) | *x as u64;
    }
    Some(v)
}

/// A SimpleBlock's or BlockGroup's body: (track, relative timestamp, duration, payload).
fn parse_block(id: u32, body: &[u8]) -> Result<(u64, i16, Option<u64>, Vec<u8>)> {
    let (block, duration) = if id == SIMPLE_BLOCK {
        (body, None)
    } else {
        let mut block = None;
        let mut duration = None;
        for (cid, cb) in mkv::children(body)? {
            match cid {
                BLOCK => block = Some(cb),
                BLOCK_DURATION => duration = Some(mkv::uint(cb)),
                _ => {}
            }
        }
        (block.context("BlockGroup without Block")?, duration)
    };
    let track = block_track(block).context("block track")?;
    let tl = block[0].leading_zeros() as usize + 1;
    let rest = block.get(tl..).context("short block")?;
    if rest.len() < 3 {
        bail!("short block");
    }
    let rel = i16::from_be_bytes([rest[0], rest[1]]);
    let flags = rest[2];
    if flags & 0x06 != 0 {
        bail!("laced subtitle block");
    }
    Ok((track, rel, duration, rest[3..].to_vec()))
}

/// Reads [start, end) of the file in big chunks, handing out byte ranges.
struct Chunks<'a> {
    file: &'a File,
    buf: Vec<u8>,
    at: u64,
    len: u64,
    pace: Pace<'a>,
    /// Parts reading at once: they share the rate.
    share: u64,
}

impl Chunks<'_> {
    /// `n` bytes at `pos` (fewer at the end of the file).
    async fn get(&mut self, pos: u64, n: u64) -> Result<&[u8]> {
        if pos < self.at || pos + n > self.at + self.buf.len() as u64 {
            let step = (self.pace)().await?;
            let rate = step.rate / self.share.max(1);
            let t0 = std::time::Instant::now();
            let want = CHUNK.max(n).min(self.len.saturating_sub(pos));
            self.buf = self.file.read(pos, want).await?;
            self.at = pos;
            if rate > 0 {
                let min = Duration::from_secs_f64(self.buf.len() as f64 / rate as f64);
                if let Some(left) = min.checked_sub(t0.elapsed()) {
                    tokio::time::sleep(left).await;
                }
            }
            if !step.rest.is_zero() {
                tokio::time::sleep(step.rest).await;
            }
        }
        let s = (pos - self.at) as usize;
        let e = (s + n as usize).min(self.buf.len());
        Ok(&self.buf[s.min(e)..e])
    }
}

/// Element header at `pos`: (id, size, header length).
async fn header_at(c: &mut Chunks<'_>, pos: u64) -> Result<Option<(u32, Option<u64>, u64)>> {
    let b = c.get(pos, 12).await?;
    let short = b.len() < 12;
    let mut r = Reader::new(b);
    match r.element() {
        Ok((id, size)) => Ok(Some((id, size, r.pos as u64))),
        // The end of the file (a truncated last element).
        Err(_) if short => Ok(None),
        Err(e) => Err(e),
    }
}

async fn by_scan(file: &File, index: &Index, wants: &[Want], pace: Pace<'_>) -> Result<Vec<Found>> {
    let mut clusters: Vec<u64> = index
        .cues
        .iter()
        .filter(|c| c.cluster != u64::MAX)
        .map(|c| index.segment_data + c.cluster)
        .collect();
    clusters.sort_unstable();
    clusters.dedup();
    if clusters.is_empty() {
        bail!("no cluster positions in the index");
    }
    let len = file.len();
    // Up to 3 parts of about equal size, each starting at a cluster (together they fill the
    // share's link; one alone reads at about 70 %).
    let parts = 3usize.min(clusters.len());
    let mut starts: Vec<u64> = (0..parts).map(|k| clusters[k * clusters.len() / parts]).collect();
    starts.dedup();
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    for (k, s) in starts.iter().enumerate() {
        ranges.push((*s, starts.get(k + 1).copied().unwrap_or(len)));
    }
    let tracks: Vec<u64> = wants.iter().map(|w| w.track).collect();
    // The parts run together, each waiting for its turn (the pace allows `parallel` of them).
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let share = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let parts_fut = ranges.iter().map(|&(s, e)| {
        let tracks = &tracks;
        let (gate, share) = (gate.clone(), share.clone());
        async move {
            let _permit = gate.acquire().await;
            scan_range(file, s, e, len, tracks, pace, share.load(std::sync::atomic::Ordering::Relaxed)).await
        }
    });
    let opener = async {
        // Let `parallel` parts read at once (decided once: the pace still holds each read).
        let step = pace().await?;
        let n = step.parallel.clamp(1, ranges.len());
        share.store(n as u64, std::sync::atomic::Ordering::Relaxed);
        gate.add_permits(n);
        Ok::<_, anyhow::Error>(())
    };
    let (opened, results) = tokio::join!(opener, futures_util::future::join_all(parts_fut));
    opened?;
    let mut found = Vec::new();
    for r in results {
        found.extend(r?);
    }
    Ok(found)
}

async fn scan_range(file: &File, start: u64, end: u64, len: u64, tracks: &[u64], pace: Pace<'_>, share: u64) -> Result<Vec<Found>> {
    let mut c = Chunks {
        file,
        buf: Vec::new(),
        at: 0,
        len,
        pace,
        share,
    };
    let mut found = Vec::new();
    let mut pos = start;
    while pos < end {
        let Some((id, size, hl)) = header_at(&mut c, pos).await? else {
            break;
        };
        if id != CLUSTER {
            match size {
                Some(s) => {
                    pos += hl + s;
                    continue;
                }
                None => break,
            }
        }
        let data = pos + hl;
        let cluster_end = size.map_or(len, |s| data + s);
        let mut cluster_ts: Option<i64> = None;
        let mut p = data;
        while p < cluster_end {
            let Some((cid, csize, chl)) = header_at(&mut c, p).await? else {
                break;
            };
            if cid == CLUSTER && size.is_none() {
                // The next cluster (this one had an unknown size).
                break;
            }
            let Some(csize) = csize else {
                bail!("element of unknown size in a cluster")
            };
            let body = p + chl;
            match cid {
                CLUSTER_TIMESTAMP => {
                    cluster_ts = Some(mkv::uint(c.get(body, csize).await?) as i64);
                }
                SIMPLE_BLOCK | BLOCK_GROUP => {
                    // The track number is the first thing in a SimpleBlock, and in the Block,
                    // usually the BlockGroup's first child.
                    let head = c.get(body, 16.min(csize)).await?;
                    let tn = if cid == SIMPLE_BLOCK {
                        block_track(head)
                    } else {
                        let mut r = Reader::new(head);
                        match r.element() {
                            Ok((BLOCK, _)) => block_track(&head[r.pos..]),
                            // Not first: look at the whole group.
                            _ => None,
                        }
                    };
                    if tn.is_none_or(|t| tracks.contains(&t)) {
                        let b = c.get(body, csize).await?.to_vec();
                        if let Ok((track, rel, duration, payload)) = parse_block(cid, &b) {
                            if tracks.contains(&track) {
                                let ts = cluster_ts.context("block before the cluster's timestamp")?;
                                found.push(Found {
                                    track,
                                    time: ts + rel as i64,
                                    duration,
                                    data: payload,
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
            p = body + csize;
        }
        pos = if size.is_some() { cluster_end } else { p };
    }
    Ok(found)
}

/// The blocks as cues per stream: times in seconds, text as SRT wants it.
fn to_cues(index: &Index, wants: &[Want], mut found: Vec<Found>) -> HashMap<i64, Vec<Cue>> {
    found.sort_by_key(|f| (f.track, f.time));
    let mut out: HashMap<i64, Vec<Cue>> = HashMap::new();
    for w in wants {
        let blocks: Vec<&Found> = found.iter().filter(|f| f.track == w.track).collect();
        let ass = w.codec == "S_TEXT/ASS" || w.codec == "S_TEXT/SSA";
        let mut cues = Vec::with_capacity(blocks.len());
        for (i, b) in blocks.iter().enumerate() {
            let start = index.secs(b.time);
            let end = match b.duration {
                Some(d) => start + index.secs(d as i64),
                None if w.default_duration > 0 => start + w.default_duration as f64 / 1e9,
                // No duration anywhere: until the next one, at most 5 s.
                None => blocks.get(i + 1).map_or(start + 5.0, |n| index.secs(n.time)).min(start + 5.0),
            };
            let raw = String::from_utf8_lossy(&b.data);
            let text = if ass {
                ass_text(&raw)
            } else {
                raw.replace("\r\n", "\n").trim().to_string()
            };
            if text.trim().is_empty() {
                continue;
            }
            cues.push(Cue {
                start,
                end: end.max(start),
                text,
            });
        }
        out.insert(w.stream, cues);
    }
    out
}

/// A Matroska ASS/SSA block (`ReadOrder,Layer,Style,Name,MarginL,MarginR,MarginV,Effect,Text`)
/// as SRT text: line breaks kept, italic/bold/underline as tags, other overrides dropped.
pub fn ass_text(block: &str) -> String {
    let text = block.splitn(9, ',').nth(8).unwrap_or(block);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' => {
                let mut tag = String::new();
                for d in chars.by_ref() {
                    if d == '}' {
                        break;
                    }
                    tag.push(d);
                }
                for t in tag.split('\\').filter(|t| !t.is_empty()) {
                    match t {
                        "i1" => out.push_str("<i>"),
                        "i0" => out.push_str("</i>"),
                        "b1" => out.push_str("<b>"),
                        "b0" => out.push_str("</b>"),
                        "u1" => out.push_str("<u>"),
                        "u0" => out.push_str("</u>"),
                        _ => {}
                    }
                }
            }
            '\\' => match chars.peek() {
                Some('N' | 'n') => {
                    chars.next();
                    out.push('\n');
                }
                Some('h') => {
                    chars.next();
                    out.push(' ');
                }
                _ => out.push('\\'),
            },
            _ => out.push(ch),
        }
    }
    out.trim().to_string()
}

fn srt_ts(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// Cues as SRT.
pub fn to_srt(cues: &[Cue]) -> String {
    let mut out = String::new();
    for (i, c) in cues.iter().enumerate() {
        let _ = write!(out, "{}\n{} --> {}\n{}\n\n", i + 1, srt_ts(c.start), srt_ts(c.end), c.text);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ass_lines() {
        assert_eq!(
            ass_text("12,0,Default,,0,0,0,,{\\i1}Hello{\\i0}\\Nworld, again"),
            "<i>Hello</i>\nworld, again"
        );
        assert_eq!(ass_text("1,0,Default,,0,0,0,,{\\an8\\pos(10,10)}Top\\hline"), "Top line");
    }

    #[test]
    fn blocks() {
        // SimpleBlock: track 3 (0x83), +1000 units, no lacing, "Hi".
        let b = [0x83, 0x03, 0xE8, 0x80, b'H', b'i'];
        let (t, rel, d, data) = parse_block(SIMPLE_BLOCK, &b).unwrap();
        assert_eq!((t, rel, d, data.as_slice()), (3, 1000, None, &b"Hi"[..]));
        // BlockGroup: Block + BlockDuration 2000.
        let mut g = vec![0xA1, 0x86];
        g.extend_from_slice(&b);
        g.extend_from_slice(&[0x9B, 0x82, 0x07, 0xD0]);
        let (t, rel, d, data) = parse_block(BLOCK_GROUP, &g).unwrap();
        assert_eq!((t, rel, d, data.as_slice()), (3, 1000, Some(2000), &b"Hi"[..]));
        // Found behind a cluster header of 12 bytes (ID + 8-byte size) at a relative position.
        let mut buf = vec![0u8; 12];
        buf.extend_from_slice(&[0xA0, 0x80 | g.len() as u8]);
        buf.extend_from_slice(&g);
        let (data, d, rel) = parse_at(&buf, 0, 12, 3).unwrap().unwrap();
        assert_eq!((data.as_slice(), d, rel), (&b"Hi"[..], Some(2000), 1000));
        // The learned header length is wrong: the others are tried.
        assert!(parse_at(&buf, 0, 9, 3).unwrap().is_ok());
        // Another track's block isn't taken.
        assert!(parse_at(&buf, 0, 12, 4).is_none());
        // Cut short: asks for more.
        assert!(matches!(parse_at(&buf[..16], 0, 12, 3), Some(Err(_))));
    }

    #[test]
    fn srt_out() {
        let cues = vec![
            Cue {
                start: 1.5,
                end: 3.25,
                text: "a\nb".into(),
            },
            Cue {
                start: 3723.0,
                end: 3724.0,
                text: "c".into(),
            },
        ];
        assert_eq!(
            to_srt(&cues),
            "1\n00:00:01,500 --> 00:00:03,250\na\nb\n\n2\n01:02:03,000 --> 01:02:04,000\nc\n\n"
        );
    }
}
