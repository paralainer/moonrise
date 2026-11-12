//! Just enough fragmented-MP4 handling to turn ffmpeg's output into HLS pieces: the init
//! section (ftyp+moov), and moof+mdat fragments whose decode time (tfdt) can be read and moved;
//! and to write fragments of our own (converted audio, whose frames are timed here).

use anyhow::{bail, Context, Result};

pub struct Fragmented {
    pub init: Vec<u8>,
    pub fragments: Vec<Fragment>,
    /// Media timescale of the (single) track, from mdhd.
    pub timescale: u32,
    /// Where the edit list starts the track, in media time (0: no edit list): what is subtracted
    /// from a sample's composition time to get its time in the file.
    pub media_start: i64,
}

pub struct Fragment {
    /// moof followed by its mdat.
    pub bytes: Vec<u8>,
    /// Offset of the tfdt box payload inside `bytes`, and its version.
    tfdt_at: usize,
    tfdt_v1: bool,
    /// The first sample's composition offset (presentation minus decode time).
    first_offset: i64,
}

impl Fragment {
    pub fn decode_time(&self) -> u64 {
        let b = &self.bytes[self.tfdt_at..];
        if self.tfdt_v1 {
            u64::from_be_bytes(b[4..12].try_into().unwrap())
        } else {
            u32::from_be_bytes(b[4..8].try_into().unwrap()) as u64
        }
    }

    /// Composition time of the fragment's first sample: for video, of the keyframe it starts
    /// with. Not the same moment as `decode_time`: with an open GOP the frames shown just before
    /// the keyframe are decoded after it, so it is decoded early by as many frames as that.
    pub fn composition_time(&self) -> i64 {
        self.decode_time() as i64 + self.first_offset
    }

    /// Moves the fragment by `delta` timescale units, not before time 0 (upgrading tfdt to 64-bit
    /// is not needed: ffmpeg writes version 1 when frag_discont is set; version 0 is patched in place).
    pub fn shift(&mut self, delta: i64) -> Result<()> {
        let t = self.decode_time().saturating_add_signed(delta);
        let b = &mut self.bytes[self.tfdt_at..];
        if self.tfdt_v1 {
            b[4..12].copy_from_slice(&t.to_be_bytes());
        } else {
            let t32: u32 = t.try_into().context("tfdt overflow")?;
            b[4..8].copy_from_slice(&t32.to_be_bytes());
        }
        Ok(())
    }
}

impl Fragmented {
    /// The time in the file (seconds) of a fragment's first sample.
    pub fn file_time(&self, f: &Fragment) -> f64 {
        (f.composition_time() - self.media_start) as f64 / self.timescale as f64
    }
}

impl Fragment {
    /// One moof followed by its mdat, as ffmpeg writes them.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Fragment> {
        let (_, payload, end) = match boxes(&bytes, 0, bytes.len())?.first() {
            Some((k, s, p, e)) if k == "moof" => (*s, *p, *e),
            _ => bail!("not a moof"),
        };
        let (t, _) = find(&bytes, payload, end, &["traf", "tfdt"])?.context("no tfdt")?;
        let first_offset = first_sample_offset(&bytes, payload, end)?;
        Ok(Fragment {
            tfdt_at: t,
            tfdt_v1: bytes[t] == 1,
            first_offset,
            bytes,
        })
    }
}

pub(crate) fn boxes(data: &[u8], mut pos: usize, end: usize) -> Result<Vec<(String, usize, usize, usize)>> {
    // (type, start, payload start, end)
    let mut out = Vec::new();
    while pos + 8 <= end {
        let size32 = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let kind = String::from_utf8_lossy(&data[pos + 4..pos + 8]).into_owned();
        let (size, header) = match size32 {
            1 => (u64::from_be_bytes(data[pos + 8..pos + 16].try_into().unwrap()) as usize, 16),
            0 => (end - pos, 8),
            s => (s, 8),
        };
        if size < header || pos + size > end {
            bail!("truncated box {kind}");
        }
        out.push((kind, pos, pos + header, pos + size));
        pos += size;
    }
    Ok(out)
}

pub(crate) fn find(data: &[u8], start: usize, end: usize, path: &[&str]) -> Result<Option<(usize, usize)>> {
    for (kind, _, payload, box_end) in boxes(data, start, end)? {
        if kind == path[0] {
            return if path.len() == 1 {
                Ok(Some((payload, box_end)))
            } else {
                find(data, payload, box_end, &path[1..])
            };
        }
    }
    Ok(None)
}

/// The composition offset of the first sample of a moof's first `trun` (0 when it has none).
fn first_sample_offset(data: &[u8], moof: usize, end: usize) -> Result<i64> {
    let Some((t, t_end)) = find(data, moof, end, &["traf", "trun"])? else {
        return Ok(0);
    };
    let word = |at: usize| -> Result<[u8; 4]> {
        Ok(data
            .get(at..at + 4)
            .filter(|_| at + 4 <= t_end)
            .context("short trun")?
            .try_into()
            .unwrap())
    };
    let head = u32::from_be_bytes(word(t)?);
    let (version, flags) = (head >> 24, head & 0xff_ffff);
    if u32::from_be_bytes(word(t + 4)?) == 0 || flags & 0x800 == 0 {
        return Ok(0);
    }
    // Before the first sample's offset: data offset, first sample flags, its duration, size, flags.
    let skip = [0x1, 0x4, 0x100, 0x200, 0x400].iter().filter(|bit| flags & **bit != 0).count();
    let b = word(t + 8 + 4 * skip)?;
    Ok(if version == 0 {
        u32::from_be_bytes(b) as i64
    } else {
        i32::from_be_bytes(b) as i64
    })
}

/// The media time the (first) track's edit list starts at; 0 without one.
fn edit_list_start(data: &[u8], moov: usize, end: usize) -> Result<i64> {
    let Some((p, p_end)) = find(data, moov, end, &["trak", "edts", "elst"])? else {
        return Ok(0);
    };
    let v1 = data[p] == 1;
    let (size, at) = if v1 { (20, 8) } else { (12, 4) };
    let mut entry = p + 8;
    while entry + size <= p_end {
        // An empty edit (-1) only delays the track; the first real one says where it starts.
        let time = if v1 {
            i64::from_be_bytes(data[entry + at..entry + at + 8].try_into().unwrap())
        } else {
            i32::from_be_bytes(data[entry + at..entry + at + 4].try_into().unwrap()) as i64
        };
        if time >= 0 {
            return Ok(time);
        }
        entry += size;
    }
    Ok(0)
}

pub fn parse(data: &[u8]) -> Result<Fragmented> {
    let top = boxes(data, 0, data.len())?;
    let mut init = Vec::new();
    let mut fragments = Vec::new();
    let mut timescale = 0;
    let mut media_start = 0;
    let mut i = 0;
    while i < top.len() {
        let (kind, start, payload, end) = &top[i];
        match kind.as_str() {
            "ftyp" => init.extend_from_slice(&data[*start..*end]),
            "moov" => {
                init.extend_from_slice(&data[*start..*end]);
                let (p, _) = find(data, *payload, *end, &["trak", "mdia", "mdhd"])?.context("no mdhd")?;
                let v1 = data[p] == 1;
                let at = p + if v1 { 4 + 16 } else { 4 + 8 };
                timescale = u32::from_be_bytes(data[at..at + 4].try_into().unwrap());
                media_start = edit_list_start(data, *payload, *end)?;
            }
            "moof" => {
                let (mdat_start, mdat_end) = match top.get(i + 1) {
                    Some((k, s, _, e)) if k == "mdat" => (*s, *e),
                    _ => bail!("moof without mdat"),
                };
                let (t, _) = find(data, *payload, *end, &["traf", "tfdt"])?.context("no tfdt")?;
                let mut bytes = Vec::with_capacity(mdat_end - start);
                bytes.extend_from_slice(&data[*start..*end]);
                bytes.extend_from_slice(&data[mdat_start..mdat_end]);
                let first_offset = first_sample_offset(data, *payload, *end)?;
                fragments.push(Fragment {
                    tfdt_at: t - start,
                    tfdt_v1: data[t] == 1,
                    first_offset,
                    bytes,
                });
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    if init.is_empty() || timescale == 0 {
        bail!("no init section");
    }
    Ok(Fragmented {
        init,
        fragments,
        timescale,
        media_start,
    })
}

/// The init section without edit lists (`edts`). ffmpeg describes encoder priming with one;
/// converted audio is timed sample-exactly in its fragments instead, as copied audio is.
pub fn strip_edts(init: &[u8]) -> Result<Vec<u8>> {
    fn rebuild(data: &[u8], start: usize, end: usize, drop: &str, descend: &[&str]) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        for (kind, s, payload, e) in boxes(data, start, end)? {
            if kind == drop {
                continue;
            }
            if descend.contains(&kind.as_str()) {
                let body = rebuild(data, payload, e, drop, descend)?;
                out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
                out.extend_from_slice(kind.as_bytes());
                out.extend_from_slice(&body);
            } else {
                out.extend_from_slice(&data[s..e]);
            }
        }
        Ok(out)
    }
    rebuild(init, 0, init.len(), "edts", &["moov", "trak"])
}

/// The init section with a Dolby Vision configuration box (`dvcC`/`dvvC`) in its video sample
/// entry, and the entry renamed to `rename` if given (`dvh1`, for a stream with no base layer).
/// An entry that already has one keeps it.
pub fn with_dovi(init: &[u8], config: &[u8], rename: Option<&[u8; 4]>) -> Result<Vec<u8>> {
    const VISUAL_ENTRY: usize = 78; // VisualSampleEntry's fields before its boxes
    fn rebuild(data: &[u8], start: usize, end: usize, config: &[u8], rename: Option<&[u8; 4]>, done: &mut bool) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        for (kind, s, payload, e) in boxes(data, start, end)? {
            let (head, body) = match kind.as_str() {
                "moov" | "trak" | "mdia" | "minf" | "stbl" => (Vec::new(), rebuild(data, payload, e, config, rename, done)?),
                // Full box: version and flags, entry count, then the entries.
                "stsd" if e >= payload + 8 => (
                    data[payload..payload + 8].to_vec(),
                    rebuild(data, payload + 8, e, config, rename, done)?,
                ),
                "hvc1" | "hev1" | "dvh1" | "dvhe" if e >= payload + VISUAL_ENTRY => {
                    let mut body = data[payload..e].to_vec();
                    let children = boxes(data, payload + VISUAL_ENTRY, e)?;
                    if !children.iter().any(|c| c.0 == "dvcC" || c.0 == "dvvC") {
                        body.extend_from_slice(config);
                    }
                    *done = true;
                    let kind = rename.map_or_else(|| kind.as_bytes().to_vec(), |r| r.to_vec());
                    out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
                    out.extend_from_slice(&kind);
                    out.extend_from_slice(&body);
                    continue;
                }
                _ => {
                    out.extend_from_slice(&data[s..e]);
                    continue;
                }
            };
            out.extend_from_slice(&((head.len() + body.len() + 8) as u32).to_be_bytes());
            out.extend_from_slice(kind.as_bytes());
            out.extend_from_slice(&head);
            out.extend_from_slice(&body);
        }
        Ok(out)
    }
    let mut done = false;
    let out = rebuild(init, 0, init.len(), config, rename, &mut done)?;
    if !done {
        bail!("no HEVC sample entry");
    }
    Ok(out)
}

/// The track id in an init section's (first) `tkhd`.
pub fn track_id(init: &[u8]) -> Result<u32> {
    let (moov, end) = find(init, 0, init.len(), &["moov"])?.context("no moov")?;
    let (p, _) = find(init, moov, end, &["trak", "tkhd"])?.context("no tkhd")?;
    let at = p + if init[p] == 1 { 4 + 16 } else { 4 + 8 };
    Ok(u32::from_be_bytes(init.get(at..at + 4).context("short tkhd")?.try_into().unwrap()))
}

fn full_box(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 12);
    out.extend_from_slice(&((body.len() + 12) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(&(((version as u32) << 24) | flags).to_be_bytes());
    out.extend_from_slice(body);
    out
}

fn plain_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

/// One moof+mdat holding `samples` (each a whole access unit, all sync samples) of
/// `duration` timescale units each, the first decoded at `base`.
pub fn write_fragment(sequence: u32, track_id: u32, base: u64, duration: u32, samples: &[&[u8]]) -> Vec<u8> {
    let mfhd = full_box(b"mfhd", 0, 0, &sequence.to_be_bytes());
    // default-base-is-moof | default-sample-flags: every sample stands alone (audio).
    let mut tfhd_body = track_id.to_be_bytes().to_vec();
    tfhd_body.extend_from_slice(&0x0200_0000u32.to_be_bytes());
    let tfhd = full_box(b"tfhd", 0, 0x02_0000 | 0x20, &tfhd_body);
    let tfdt = full_box(b"tfdt", 1, 0, &base.to_be_bytes());
    // data-offset | sample-duration | sample-size
    let trun_len = 12 + 8 + samples.len() * 8;
    let traf_len = 8 + tfhd.len() + tfdt.len() + trun_len;
    let moof_len = 8 + mfhd.len() + traf_len;
    let mut trun_body = Vec::with_capacity(trun_len - 12);
    trun_body.extend_from_slice(&(samples.len() as u32).to_be_bytes());
    trun_body.extend_from_slice(&((moof_len + 8) as i32).to_be_bytes());
    for s in samples {
        trun_body.extend_from_slice(&duration.to_be_bytes());
        trun_body.extend_from_slice(&(s.len() as u32).to_be_bytes());
    }
    let trun = full_box(b"trun", 0, 0x000001 | 0x000100 | 0x000200, &trun_body);
    let traf = plain_box(b"traf", &[tfhd, tfdt, trun].concat());
    let moof = plain_box(b"moof", &[mfhd, traf].concat());
    debug_assert_eq!(moof.len(), moof_len);
    let mdat_len: usize = samples.iter().map(|s| s.len()).sum();
    let mut out = moof;
    out.reserve(8 + mdat_len);
    out.extend_from_slice(&((mdat_len + 8) as u32).to_be_bytes());
    out.extend_from_slice(b"mdat");
    for s in samples {
        out.extend_from_slice(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal init section: ftyp + moov(trak(tkhd, edts, mdia(mdhd))).
    pub(crate) fn sample_init() -> Vec<u8> {
        let mut tkhd = vec![0u8; 8];
        tkhd.extend_from_slice(&7u32.to_be_bytes());
        tkhd.extend_from_slice(&[0u8; 72]);
        let tkhd = full_box(b"tkhd", 0, 3, &tkhd);
        let edts = plain_box(b"edts", &full_box(b"elst", 0, 0, &[0, 0, 0, 1, 0, 0, 0, 9, 0, 0, 4, 0, 0, 1, 0, 0]));
        let mut mdhd = vec![0u8; 8];
        mdhd.extend_from_slice(&48000u32.to_be_bytes());
        mdhd.extend_from_slice(&[0u8; 8]);
        let mdhd = full_box(b"mdhd", 0, 0, &mdhd);
        let mdia = plain_box(b"mdia", &mdhd);
        let trak = plain_box(b"trak", &[tkhd, edts, mdia].concat());
        let moov = plain_box(b"moov", &trak);
        [plain_box(b"ftyp", b"iso6\0\0\0\0"), moov].concat()
    }

    #[test]
    fn strip_edts_and_track_id() {
        let init = sample_init();
        assert!(find(&init, 0, init.len(), &["moov", "trak", "edts"]).unwrap().is_some());
        let out = strip_edts(&init).unwrap();
        assert!(find(&out, 0, out.len(), &["moov", "trak", "edts"]).unwrap().is_none());
        assert_eq!(out.len(), init.len() - 36);
        assert_eq!(track_id(&out).unwrap(), 7);
        // Still parses as an init section, with its timescale.
        let frag = write_fragment(1, 7, 0, 1024, &[b"ab"]);
        let parsed = parse(&[out.clone(), frag].concat()).unwrap();
        assert_eq!(parsed.timescale, 48000);
        assert_eq!(parsed.init, out);
    }

    /// moof(traf(tfdt, trun)) + mdat: a trun with sample durations, sizes and composition offsets.
    fn reordered_fragment(version: u8, decode: u64, offset: i32) -> Vec<u8> {
        let tfdt = full_box(b"tfdt", 1, 0, &decode.to_be_bytes());
        let mut trun = 1u32.to_be_bytes().to_vec();
        trun.extend_from_slice(&0i32.to_be_bytes()); // data offset
        trun.extend_from_slice(&0x0200_0000u32.to_be_bytes()); // first sample flags
        trun.extend_from_slice(&667u32.to_be_bytes()); // duration
        trun.extend_from_slice(&2u32.to_be_bytes()); // size
        trun.extend_from_slice(&offset.to_be_bytes());
        let trun = full_box(b"trun", version, 0x1 | 0x4 | 0x100 | 0x200 | 0x800, &trun);
        let moof = plain_box(b"moof", &plain_box(b"traf", &[tfdt, trun].concat()));
        [moof, plain_box(b"mdat", b"ab")].concat()
    }

    /// The init section with this edit list start instead of the sample's.
    fn init_with_start(entries: &[i32]) -> Vec<u8> {
        let mut body = (entries.len() as u32).to_be_bytes().to_vec();
        for e in entries {
            body.extend_from_slice(&0u32.to_be_bytes());
            body.extend_from_slice(&e.to_be_bytes());
            body.extend_from_slice(&0x10000u32.to_be_bytes());
        }
        let edts = plain_box(b"edts", &full_box(b"elst", 0, 0, &body));
        let mut mdhd = vec![0u8; 8];
        mdhd.extend_from_slice(&16000u32.to_be_bytes());
        mdhd.extend_from_slice(&[0u8; 8]);
        let mdia = plain_box(b"mdia", &full_box(b"mdhd", 0, 0, &mdhd));
        plain_box(b"moov", &plain_box(b"trak", &[edts, mdia].concat()))
    }

    #[test]
    fn file_time_is_the_keyframes_presentation_time() {
        // As ffmpeg cut 28 Days Later's 76.410 keyframe: decoded at 76.117, offset 0.376, edit
        // list 0.083 (1328 at 16 kHz).
        let (decode, offset) = (1_217_872, 6016);
        for version in [0, 1] {
            let bytes = [init_with_start(&[1328]), reordered_fragment(version, decode, offset)].concat();
            let parsed = parse(&bytes).unwrap();
            assert_eq!(parsed.media_start, 1328);
            let f = &parsed.fragments[0];
            assert_eq!(f.decode_time() as f64 / 16000.0, 76.117);
            assert_eq!(parsed.file_time(f), 76.41);
            assert_eq!(
                Fragment::from_bytes(f.bytes.clone()).unwrap().composition_time(),
                decode as i64 + offset as i64
            );
        }
        // Negative offsets (trun version 1), an empty edit first, no edit list.
        let parsed = parse(&[init_with_start(&[-1, 800]), reordered_fragment(1, 1600, -800)].concat()).unwrap();
        assert_eq!(parsed.media_start, 800);
        assert_eq!(parsed.file_time(&parsed.fragments[0]), 0.0);
        let parsed = parse(&[sample_init(), write_fragment(1, 7, 4800, 1024, &[b"ab"])].concat()).unwrap();
        assert_eq!(parsed.media_start, 1024);
    }

    #[test]
    fn shifting_keeps_the_offset_and_stops_at_zero() {
        let mut f = parse(&[init_with_start(&[0]), reordered_fragment(0, 1000, 500)].concat())
            .unwrap()
            .fragments
            .remove(0);
        f.shift(200).unwrap();
        assert_eq!((f.decode_time(), f.composition_time()), (1200, 1700));
        f.shift(-300).unwrap();
        assert_eq!((f.decode_time(), f.composition_time()), (900, 1400));
        f.shift(-5000).unwrap();
        assert_eq!(f.decode_time(), 0);
    }

    #[test]
    fn fragment_from_bytes() {
        let frag = write_fragment(3, 1, 123_456, 1536, &[b"abc"]);
        let f = Fragment::from_bytes(frag.clone()).unwrap();
        assert_eq!(f.decode_time(), 123_456);
        assert_eq!(f.bytes, frag);
        assert!(Fragment::from_bytes(sample_init()).is_err());
    }

    #[test]
    fn fragment_roundtrip() {
        let frag = write_fragment(3, 1, 1 << 33, 1536, &[b"abc", b"de", b"fghi"]);
        let parsed = parse(&[sample_init(), frag.clone()].concat()).unwrap();
        assert_eq!(parsed.fragments.len(), 1);
        let f = &parsed.fragments[0];
        assert_eq!(f.decode_time(), 1 << 33);
        assert!(frag.ends_with(b"mdatabcdefghi"));
        // trun's data offset points at the first sample.
        let (t, _) = find(&frag, 0, frag.len(), &["moof", "traf", "trun"]).unwrap().unwrap();
        let off = i32::from_be_bytes(frag[t + 8..t + 12].try_into().unwrap()) as usize;
        assert_eq!(&frag[off..off + 3], b"abc");
        assert_eq!(u32::from_be_bytes(frag[t + 4..t + 8].try_into().unwrap()), 3);
    }

    /// A video init section: moov(trak(tkhd, mdia(mdhd, minf(stbl(stsd(hvc1(…, hvcC, colr))))))).
    fn hevc_init(entry: &[u8; 4]) -> Vec<u8> {
        let mut e = vec![0u8; 78];
        e[24..28].copy_from_slice(&[0x0F, 0, 0x06, 0x40]); // width 3840, height 1600
        e.extend(plain_box(b"hvcC", &[1, 2, 3, 4]));
        e.extend(plain_box(b"colr", b"nclx\0\x09\0\x10\0\x09\0"));
        let entry = plain_box(entry, &e);
        let stsd = full_box(b"stsd", 0, 0, &[&1u32.to_be_bytes()[..], &entry].concat());
        let stbl = plain_box(b"stbl", &[stsd, full_box(b"stts", 0, 0, &[0, 0, 0, 0])].concat());
        let minf = plain_box(b"minf", &stbl);
        let mdia = plain_box(b"mdia", &[full_box(b"mdhd", 0, 0, &[0u8; 20]), minf].concat());
        let trak = plain_box(b"trak", &[full_box(b"tkhd", 0, 3, &[0u8; 80]), mdia].concat());
        let moov = plain_box(b"moov", &[trak, plain_box(b"mvex", &[0u8; 4])].concat());
        [plain_box(b"ftyp", b"iso6\0\0\0\0"), moov].concat()
    }

    /// The sample entry of an init: its type and child boxes' types.
    fn entry_of(init: &[u8]) -> (String, Vec<String>) {
        let (stsd, end) = find(init, 0, init.len(), &["moov", "trak", "mdia", "minf", "stbl", "stsd"])
            .unwrap()
            .unwrap();
        let (kind, _, payload, e) = boxes(init, stsd + 8, end).unwrap().remove(0);
        (kind, boxes(init, payload + 78, e).unwrap().into_iter().map(|b| b.0).collect())
    }

    #[test]
    fn dolby_vision_config_in_the_init() {
        let init = hevc_init(b"hvc1");
        let config = plain_box(b"dvvC", &[1u8; 24]);
        let out = with_dovi(&init, &config, None).unwrap();
        assert_eq!(out.len(), init.len() + 32);
        assert_eq!(
            entry_of(&out),
            ("hvc1".to_string(), vec!["hvcC".into(), "colr".into(), "dvvC".into()])
        );
        // Every box around it still parses and ends where its parent does; the rest is untouched.
        let (_, dvvc_end) = find(&out, 0, out.len(), &["moov", "trak", "mdia", "minf", "stbl", "stsd"])
            .unwrap()
            .unwrap();
        assert_eq!(&out[dvvc_end - 24..dvvc_end], &[1u8; 24]);
        assert!(find(&out, 0, out.len(), &["moov", "mvex"]).unwrap().is_some());
        assert!(find(&out, 0, out.len(), &["moov", "trak", "mdia", "minf", "stbl", "stts"])
            .unwrap()
            .is_some());
        assert_eq!(&out[..16], &init[..16]);
        // Again: not a second box.
        assert_eq!(with_dovi(&out, &config, None).unwrap(), out);
        // Profile 5: the entry becomes dvh1.
        let p5 = with_dovi(&init, &plain_box(b"dvcC", &[5u8; 24]), Some(b"dvh1")).unwrap();
        assert_eq!(
            entry_of(&p5),
            ("dvh1".to_string(), vec!["hvcC".into(), "colr".into(), "dvcC".into()])
        );
        // Not HEVC: refused.
        assert!(with_dovi(&sample_init(), &config, None).is_err());
    }
}
