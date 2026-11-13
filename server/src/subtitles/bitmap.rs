//! Picture subtitles (Blu-ray PGS, DVD VobSub, DVB) decoded into timed images, which the apps
//! draw over the video (`pictures.rs` serves them in windows).
//!
//! The decoders take the track's packets in time order (Matroska blocks, or the packets ffprobe
//! reads from other containers) and answer what shows when: each picture with its place on the
//! track's canvas (the video's frame the subtitles were made for), its RGBA pixels, and its
//! start and end on the file's own clock. A picture with no end in the packets given has
//! `end: None` (the caller decides).
//!
//! - **PGS**: display sets (composition, windows, palettes, objects, end), objects run-length
//!   coded with up to 256 colours from a YCbCr palette with alpha. A set without objects clears
//!   the screen.
//! - **VobSub**: one packet per picture, 2-bit run-length coded in two fields, 4 of the track's 16
//!   colours (`palette:` in its `.idx` text, the Matroska CodecPrivate) with their own alpha, and
//!   start and stop delays.
//! - **DVB**: pages of regions holding objects, 2/4/8-bit run-length coded, colours from CLUTs;
//!   a page shows until the next one or its timeout.

use std::collections::HashMap;

use anyhow::{bail, Result};

use crate::subtitles::subx::Block;

/// A picture subtitle codec, by any of its names (ffprobe's, the library's, Matroska's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Pgs,
    VobSub,
    Dvb,
}

impl Codec {
    pub fn from_name(name: &str) -> Option<Codec> {
        match name.to_ascii_lowercase().as_str() {
            "hdmv_pgs_subtitle" | "pgssub" | "s_hdmv/pgs" => Some(Codec::Pgs),
            "dvd_subtitle" | "dvdsub" | "s_vobsub" => Some(Codec::VobSub),
            "dvb_subtitle" | "dvbsub" | "s_dvbsub" => Some(Codec::Dvb),
            _ => None,
        }
    }
}

/// One picture: its place on the canvas, RGBA pixels (not premultiplied), and when it shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    pub start: f64,
    pub end: Option<f64>,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// What a run of packets shows: the canvas the pictures are placed on (None: the packets don't
/// say; the video's size then), and the pictures in time order.
#[derive(Debug, Default)]
pub struct Decoded {
    pub canvas: Option<(u32, u32)>,
    pub pictures: Vec<Picture>,
}

/// Decodes `packets` (in time order). `private`: the track's codec private data (VobSub's
/// `.idx` text). Damaged packets are skipped.
pub fn decode(codec: Codec, private: &[u8], packets: &[Block]) -> Decoded {
    match codec {
        Codec::Pgs => pgs::decode(packets),
        Codec::VobSub => vobsub::decode(private, packets),
        Codec::Dvb => dvb::decode(packets),
    }
}

/// Screens in time order (each a time and the pictures then showing, empty when cleared) as
/// pictures that end when the next screen comes, or at `time + timeout` when that is sooner.
fn screens_to_pictures(screens: Vec<(f64, Option<f64>, Vec<Picture>)>) -> Vec<Picture> {
    let mut out = Vec::new();
    for (i, (time, timeout, pics)) in screens.iter().enumerate() {
        let next = screens.get(i + 1).map(|s| s.0);
        let end = match (next, timeout) {
            (Some(n), Some(t)) => Some(n.min(time + t)),
            (Some(n), None) => Some(n),
            (None, t) => t.map(|t| time + t),
        };
        if end.is_some_and(|e| e <= *time) {
            continue;
        }
        for p in pics {
            out.push(Picture {
                start: *time,
                end,
                ..p.clone()
            });
        }
    }
    out
}

/// BT.601 or BT.709 limited-range YCbCr to RGB.
fn ycbcr_to_rgb(y: u8, cb: u8, cr: u8, bt709: bool) -> [u8; 3] {
    let y = (f32::from(y) - 16.0) * 255.0 / 219.0;
    let cb = (f32::from(cb) - 128.0) * 255.0 / 224.0;
    let cr = (f32::from(cr) - 128.0) * 255.0 / 224.0;
    let (r, g, b) = if bt709 {
        (y + 1.5748 * cr, y - 0.1873 * cb - 0.4681 * cr, y + 1.8556 * cb)
    } else {
        (y + 1.402 * cr, y - 0.344_136 * cb - 0.714_136 * cr, y + 1.772 * cb)
    };
    let c = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    [c(r), c(g), c(b)]
}

/// The smallest rectangle holding every pixel that isn't fully transparent, as (x, y, w, h);
/// None when there is none.
fn opaque_bounds(rgba: &[u8], width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = (width as usize, height as usize);
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if rgba[(y * w + x) * 4 + 3] != 0 {
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    (x0 != usize::MAX).then(|| (x0 as u32, y0 as u32, (x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32))
}

/// A picture cut down to what isn't transparent (None: nothing shows).
fn trimmed(x: u32, y: u32, width: u32, height: u32, rgba: Vec<u8>) -> Option<Picture> {
    let (bx, by, bw, bh) = opaque_bounds(&rgba, width, height)?;
    let rgba = if (bw, bh) == (width, height) {
        rgba
    } else {
        let mut out = Vec::with_capacity((bw * bh * 4) as usize);
        for row in by..by + bh {
            let at = ((row * width + bx) * 4) as usize;
            out.extend_from_slice(&rgba[at..at + (bw * 4) as usize]);
        }
        out
    };
    Some(Picture {
        start: 0.0,
        end: None,
        x: x + bx,
        y: y + by,
        width: bw,
        height: bh,
        rgba,
    })
}

/// A picture as PNG.
pub fn png(p: &Picture) -> Result<Vec<u8>> {
    use image::ImageEncoder as _;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut out,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Sub,
    )
    .write_image(&p.rgba, p.width, p.height, image::ExtendedColorType::Rgba8)?;
    Ok(out)
}

/// Pictures larger than this (either side) are damaged data.
const MAX_SIDE: u32 = 8192;

mod pgs {
    use super::{screens_to_pictures, trimmed, ycbcr_to_rgb, Block, Decoded, HashMap, Picture, MAX_SIDE};

    const PDS: u8 = 0x14;
    const ODS: u8 = 0x15;
    const PCS: u8 = 0x16;
    const WDS: u8 = 0x17;
    const END: u8 = 0x80;

    struct Object {
        width: u32,
        height: u32,
        rle: Vec<u8>,
    }

    struct Placement {
        object: u16,
        x: u32,
        y: u32,
        crop: Option<(u32, u32, u32, u32)>,
    }

    struct Composition {
        palette: u8,
        placements: Vec<Placement>,
    }

    /// A packet's segments: (type, body). A `.sup` file's own "PG" headers are skipped.
    fn segments(data: &[u8]) -> Vec<(u8, &[u8])> {
        let mut out = Vec::new();
        let mut p = 0;
        while p < data.len() {
            if data[p..].starts_with(b"PG") {
                p += 10;
            }
            if p + 3 > data.len() {
                break;
            }
            let kind = data[p];
            let len = u16::from_be_bytes([data[p + 1], data[p + 2]]) as usize;
            let Some(body) = data.get(p + 3..p + 3 + len) else { break };
            out.push((kind, body));
            p += 3 + len;
        }
        out
    }

    pub fn decode(packets: &[Block]) -> Decoded {
        let mut palettes: HashMap<u8, Vec<[u8; 4]>> = HashMap::new();
        let mut objects: HashMap<u16, Object> = HashMap::new();
        let mut composition: Option<(f64, Composition)> = None;
        let mut screens: Vec<(f64, Option<f64>, Vec<Picture>)> = Vec::new();
        let mut canvas = None;
        for packet in packets {
            for (kind, b) in segments(&packet.data) {
                match kind {
                    PCS if b.len() >= 11 => {
                        let width = u32::from(u16::from_be_bytes([b[0], b[1]]));
                        let height = u32::from(u16::from_be_bytes([b[2], b[3]]));
                        let state = b[7];
                        if state & 0x80 != 0 {
                            // An epoch starts: nothing from before is used again.
                            objects.clear();
                            palettes.clear();
                        }
                        let mut placements = Vec::new();
                        let mut p = 11;
                        for _ in 0..b[10] {
                            let Some(o) = b.get(p..p + 8) else { break };
                            let cropped = o[3] & 0x40 != 0;
                            let crop = if cropped {
                                b.get(p + 8..p + 16).map(|c| {
                                    let v = |i: usize| u32::from(u16::from_be_bytes([c[i], c[i + 1]]));
                                    (v(0), v(2), v(4), v(6))
                                })
                            } else {
                                None
                            };
                            placements.push(Placement {
                                object: u16::from_be_bytes([o[0], o[1]]),
                                x: u32::from(u16::from_be_bytes([o[4], o[5]])),
                                y: u32::from(u16::from_be_bytes([o[6], o[7]])),
                                crop,
                            });
                            p += if cropped { 16 } else { 8 };
                        }
                        canvas = Some((width, height));
                        composition = Some((packet.time, Composition { palette: b[9], placements }));
                    }
                    PDS if b.len() >= 2 => {
                        let hd = canvas.is_none_or(|c: (u32, u32)| c.1 > 576);
                        let table = palettes.entry(b[0]).or_insert_with(|| vec![[0; 4]; 256]);
                        for e in b[2..].as_chunks::<5>().0 {
                            let [r, g, bl] = ycbcr_to_rgb(e[1], e[3], e[2], hd);
                            table[e[0] as usize] = [r, g, bl, e[4]];
                        }
                    }
                    ODS if b.len() >= 4 => {
                        let id = u16::from_be_bytes([b[0], b[1]]);
                        let first = b[3] & 0x80 != 0;
                        if first {
                            if b.len() < 11 {
                                continue;
                            }
                            objects.insert(
                                id,
                                Object {
                                    width: u32::from(u16::from_be_bytes([b[7], b[8]])),
                                    height: u32::from(u16::from_be_bytes([b[9], b[10]])),
                                    rle: b[11..].to_vec(),
                                },
                            );
                        } else if let Some(o) = objects.get_mut(&id) {
                            o.rle.extend_from_slice(&b[4..]);
                        }
                    }
                    WDS => {}
                    END => {
                        if let Some((time, c)) = composition.take() {
                            let palette = palettes.get(&c.palette);
                            let pics = c
                                .placements
                                .iter()
                                .filter_map(|pl| {
                                    let o = objects.get(&pl.object)?;
                                    render(o, pl, palette?)
                                })
                                .collect();
                            screens.push((time, packet.duration, pics));
                        }
                    }
                    _ => {}
                }
            }
        }
        Decoded {
            canvas,
            pictures: screens_to_pictures(screens),
        }
    }

    fn render(o: &Object, pl: &Placement, palette: &[[u8; 4]]) -> Option<Picture> {
        let (w, h) = (o.width, o.height);
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
            return None;
        }
        let idx = rle(&o.rle, w as usize, h as usize);
        let mut rgba = Vec::with_capacity(idx.len() * 4);
        for i in idx {
            rgba.extend_from_slice(&palette[i as usize]);
        }
        let (mut x, mut y, mut w, mut h) = (pl.x, pl.y, w, h);
        if let Some((cx, cy, cw, ch)) = pl.crop {
            let (cw, ch) = (cw.min(w.saturating_sub(cx)), ch.min(h.saturating_sub(cy)));
            if cw == 0 || ch == 0 {
                return None;
            }
            let mut out = Vec::with_capacity((cw * ch * 4) as usize);
            for row in cy..cy + ch {
                let at = ((row * w + cx) * 4) as usize;
                out.extend_from_slice(&rgba[at..at + (cw * 4) as usize]);
            }
            rgba = out;
            (w, h) = (cw, ch);
            (x, y) = (pl.x, pl.y);
        }
        trimmed(x, y, w, h, rgba)
    }

    /// PGS run-length data as palette indices, `w` × `h` (short data leaves the rest 0).
    pub(super) fn rle(data: &[u8], w: usize, h: usize) -> Vec<u8> {
        let mut out = vec![0u8; w * h];
        let (mut x, mut y, mut p) = (0usize, 0usize, 0usize);
        let mut next = || {
            let b = data.get(p).copied();
            p += 1;
            b
        };
        while y < h {
            let Some(b) = next() else { break };
            let (color, run) = if b != 0 {
                (b, 1)
            } else {
                let Some(f) = next() else { break };
                if f == 0 {
                    x = 0;
                    y += 1;
                    continue;
                }
                let mut run = usize::from(f & 0x3F);
                if f & 0x40 != 0 {
                    run = (run << 8) | usize::from(next().unwrap_or(0));
                }
                let color = if f & 0x80 != 0 { next().unwrap_or(0) } else { 0 };
                (color, run)
            };
            let n = run.min(w.saturating_sub(x));
            if color != 0 {
                out[y * w + x..y * w + x + n].fill(color);
            }
            x += n;
        }
        out
    }
}

mod vobsub {
    use super::{screens_to_pictures, trimmed, Block, Decoded, Picture, MAX_SIDE};

    /// The `.idx` text's size and 16-colour palette (RGB); a grey ramp when it has none.
    pub(super) fn idx(private: &[u8]) -> (Option<(u32, u32)>, [[u8; 3]; 16]) {
        let text = String::from_utf8_lossy(private);
        let mut size = None;
        let mut palette = [[0u8; 3]; 16];
        for (i, p) in palette.iter_mut().enumerate() {
            let v = (i * 17) as u8;
            *p = [v, v, v];
        }
        for line in text.lines() {
            let line = line.trim();
            if let Some(v) = line.strip_prefix("size:") {
                let mut it = v.trim().split('x').map(|n| n.trim().parse::<u32>().ok());
                if let (Some(Some(w)), Some(Some(h))) = (it.next(), it.next()) {
                    size = Some((w, h));
                }
            } else if let Some(v) = line.strip_prefix("palette:") {
                for (i, c) in v.split(',').take(16).enumerate() {
                    if let Ok(n) = u32::from_str_radix(c.trim(), 16) {
                        palette[i] = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
                    }
                }
            }
        }
        (size, palette)
    }

    /// Delays in control sequences are in units of 1024 / 90 kHz.
    const DELAY: f64 = 1024.0 / 90_000.0;

    pub fn decode(private: &[u8], packets: &[Block]) -> Decoded {
        let (canvas, palette) = idx(private);
        let mut screens: Vec<(f64, Option<f64>, Vec<Picture>)> = Vec::new();
        for packet in packets {
            let Some((start, stop, pic)) = spu(&packet.data, &palette) else {
                continue;
            };
            let at = packet.time + start;
            let shown = stop.map(|s| s - start).or(packet.duration).filter(|d| *d > 0.0);
            screens.push((at, shown, pic.into_iter().collect()));
        }
        Decoded {
            canvas,
            pictures: screens_to_pictures(screens),
        }
    }

    /// One subpicture unit: (start delay, stop delay, the picture).
    pub(super) fn spu(d: &[u8], palette: &[[u8; 3]; 16]) -> Option<(f64, Option<f64>, Option<Picture>)> {
        let be = |p: usize| d.get(p..p + 2).map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])));
        let size = be(0)?.min(d.len());
        let mut ctrl = be(2)?;
        let (mut start, mut stop) = (None, None);
        let mut colors = [0u8; 4];
        let mut alpha = [0u8; 4];
        let mut area = None;
        let mut fields = None;
        for _ in 0..64 {
            if ctrl + 4 > size {
                break;
            }
            let delay = be(ctrl)? as f64 * DELAY;
            let next = be(ctrl + 2)?;
            let mut p = ctrl + 4;
            while p < size {
                let cmd = d[p];
                p += 1;
                match cmd {
                    0x00 | 0x01 => start = Some(delay),
                    0x02 => stop = Some(delay),
                    0x03 => {
                        let b = d.get(p..p + 2)?;
                        colors = [b[1] & 0x0F, b[1] >> 4, b[0] & 0x0F, b[0] >> 4];
                        p += 2;
                    }
                    0x04 => {
                        let b = d.get(p..p + 2)?;
                        alpha = [b[1] & 0x0F, b[1] >> 4, b[0] & 0x0F, b[0] >> 4];
                        p += 2;
                    }
                    0x05 => {
                        let b = d.get(p..p + 6)?;
                        let x1 = (usize::from(b[0]) << 4) | usize::from(b[1] >> 4);
                        let x2 = (usize::from(b[1] & 0x0F) << 8) | usize::from(b[2]);
                        let y1 = (usize::from(b[3]) << 4) | usize::from(b[4] >> 4);
                        let y2 = (usize::from(b[4] & 0x0F) << 8) | usize::from(b[5]);
                        area = Some((x1, y1, x2, y2));
                        p += 6;
                    }
                    0x06 => {
                        fields = Some((be(p)?, be(p + 2)?));
                        p += 4;
                    }
                    // Colour and contrast changes within the picture: the length follows.
                    0x07 => {
                        p += be(p)?;
                    }
                    _ => break,
                }
            }
            if next == ctrl {
                break;
            }
            ctrl = next;
        }
        let pic = match (area, fields) {
            (Some((x1, y1, x2, y2)), Some((top, bottom))) if x2 >= x1 && y2 >= y1 => {
                let (w, h) = (x2 - x1 + 1, y2 - y1 + 1);
                if w as u32 > MAX_SIDE || h as u32 > MAX_SIDE {
                    return None;
                }
                let idx = rle(&d[..size], top, bottom, w, h);
                let mut rgba = Vec::with_capacity(w * h * 4);
                for v in idx {
                    let c = palette[colors[v as usize] as usize];
                    rgba.extend_from_slice(&[c[0], c[1], c[2], alpha[v as usize] * 17]);
                }
                trimmed(x1 as u32, y1 as u32, w as u32, h as u32, rgba)
            }
            _ => None,
        };
        Some((start.unwrap_or(0.0), stop, pic))
    }

    /// The two interlaced fields' 2-bit run-length data as pixel values 0..=3.
    pub(super) fn rle(d: &[u8], top: usize, bottom: usize, w: usize, h: usize) -> Vec<u8> {
        let nibble = |n: usize| {
            let b = d.get(n / 2).copied().unwrap_or(0);
            usize::from(if n.is_multiple_of(2) { b >> 4 } else { b & 0x0F })
        };
        let mut out = vec![0u8; w * h];
        for (field, start) in [(0, top), (1, bottom)] {
            let mut nib = start * 2;
            let mut y = field;
            while y < h {
                let mut x = 0;
                while x < w && nib / 2 < d.len() {
                    let mut v = nibble(nib);
                    nib += 1;
                    for limit in [0x4, 0x10, 0x40] {
                        if v >= limit {
                            break;
                        }
                        v = (v << 4) | nibble(nib);
                        nib += 1;
                    }
                    let color = (v & 3) as u8;
                    let run = if v >> 2 == 0 { w - x } else { (v >> 2).min(w - x) };
                    out[y * w + x..y * w + x + run].fill(color);
                    x += run;
                }
                // Lines start on a byte.
                nib = nib.div_ceil(2) * 2;
                y += 2;
            }
        }
        out
    }
}

mod dvb {
    use super::{screens_to_pictures, trimmed, ycbcr_to_rgb, Block, Decoded, HashMap, Picture, MAX_SIDE};

    const PAGE: u8 = 0x10;
    const REGION: u8 = 0x11;
    const CLUT: u8 = 0x12;
    const OBJECT: u8 = 0x13;
    const DISPLAY: u8 = 0x14;

    #[derive(Clone)]
    struct Clut {
        c4: [[u8; 4]; 4],
        c16: [[u8; 4]; 16],
        c256: Vec<[u8; 4]>,
    }

    impl Default for Clut {
        fn default() -> Self {
            let mut c4 = [[0u8; 4]; 4];
            c4[1] = [255, 255, 255, 255];
            c4[2] = [0, 0, 0, 255];
            c4[3] = [127, 127, 127, 255];
            let mut c16 = [[0u8; 4]; 16];
            for (i, c) in c16.iter_mut().enumerate().skip(1) {
                let v = if i < 8 { 255 } else { 127 };
                *c = [
                    if i & 1 != 0 { v } else { 0 },
                    if i & 2 != 0 { v } else { 0 },
                    if i & 4 != 0 { v } else { 0 },
                    255,
                ];
            }
            let mut c256 = vec![[0u8; 4]; 256];
            for (i, c) in c256.iter_mut().enumerate().skip(1) {
                let bit = |m: usize, v: u8| if i & m != 0 { v } else { 0 };
                *c = if i < 8 {
                    [bit(1, 255), bit(2, 255), bit(4, 255), 63]
                } else {
                    match i & 0x88 {
                        0x00 => [
                            bit(1, 85) + bit(0x10, 170),
                            bit(2, 85) + bit(0x20, 170),
                            bit(4, 85) + bit(0x40, 170),
                            255,
                        ],
                        0x08 => [
                            bit(1, 85) + bit(0x10, 170),
                            bit(2, 85) + bit(0x20, 170),
                            bit(4, 85) + bit(0x40, 170),
                            127,
                        ],
                        0x80 => [
                            127 + bit(1, 43) + bit(0x10, 85),
                            127 + bit(2, 43) + bit(0x20, 85),
                            127 + bit(4, 43) + bit(0x40, 85),
                            255,
                        ],
                        _ => [
                            bit(1, 43) + bit(0x10, 85),
                            bit(2, 43) + bit(0x20, 85),
                            bit(4, 43) + bit(0x40, 85),
                            255,
                        ],
                    }
                };
            }
            Clut { c4, c16, c256 }
        }
    }

    struct Region {
        width: usize,
        height: usize,
        /// 2, 4 or 8 bits a pixel.
        depth: u8,
        clut: u8,
        pixels: Vec<u8>,
        /// Objects in it: id → (x, y) inside the region.
        objects: Vec<(u16, usize, usize)>,
    }

    struct Page {
        timeout: f64,
        /// Regions shown, with their place on the display.
        regions: Vec<(u8, u32, u32)>,
    }

    pub fn decode(packets: &[Block]) -> Decoded {
        let mut cluts: HashMap<u8, Clut> = HashMap::new();
        let mut regions: HashMap<u8, Region> = HashMap::new();
        let mut display: Option<(u32, u32, u32, u32)> = None;
        let mut screens: Vec<(f64, Option<f64>, Vec<Picture>)> = Vec::new();
        for packet in packets {
            let mut d: &[u8] = &packet.data;
            if d.starts_with(&[0x20, 0x00]) {
                d = &d[2..];
            }
            let mut page: Option<Page> = None;
            let mut p = 0;
            while p + 6 <= d.len() && d[p] == 0x0F {
                let kind = d[p + 1];
                let len = usize::from(u16::from_be_bytes([d[p + 4], d[p + 5]]));
                let Some(b) = d.get(p + 6..p + 6 + len) else { break };
                p += 6 + len;
                match kind {
                    PAGE if b.len() >= 2 => {
                        if (b[1] >> 2) & 3 == 2 {
                            // A mode change: everything starts over.
                            regions.clear();
                            cluts.clear();
                        }
                        let list = b[2..]
                            .as_chunks::<6>()
                            .0
                            .iter()
                            .map(|r| {
                                (
                                    r[0],
                                    u32::from(u16::from_be_bytes([r[2], r[3]])),
                                    u32::from(u16::from_be_bytes([r[4], r[5]])),
                                )
                            })
                            .collect();
                        page = Some(Page {
                            timeout: f64::from(b[0]),
                            regions: list,
                        });
                    }
                    REGION if b.len() >= 10 => region(&mut regions, b),
                    CLUT if b.len() >= 2 => clut(cluts.entry(b[0]).or_default(), &b[2..]),
                    OBJECT if b.len() >= 3 => object(&mut regions, b),
                    DISPLAY if b.len() >= 5 => {
                        let w = u32::from(u16::from_be_bytes([b[1], b[2]])) + 1;
                        let h = u32::from(u16::from_be_bytes([b[3], b[4]])) + 1;
                        let (ox, oy) = if b[0] & 0x08 != 0 && b.len() >= 13 {
                            (
                                u32::from(u16::from_be_bytes([b[5], b[6]])),
                                u32::from(u16::from_be_bytes([b[9], b[10]])),
                            )
                        } else {
                            (0, 0)
                        };
                        display = Some((w, h, ox, oy));
                    }
                    _ => {}
                }
            }
            let Some(page) = page else { continue };
            let (ox, oy) = display.map_or((0, 0), |d| (d.2, d.3));
            let mut pics = Vec::new();
            for (id, x, y) in &page.regions {
                let Some(r) = regions.get(id) else { continue };
                let clut = cluts.get(&r.clut).cloned().unwrap_or_default();
                let mut rgba = Vec::with_capacity(r.pixels.len() * 4);
                for &v in &r.pixels {
                    rgba.extend_from_slice(&match r.depth {
                        2 => clut.c4[usize::from(v & 3)],
                        4 => clut.c16[usize::from(v & 15)],
                        _ => clut.c256[usize::from(v)],
                    });
                }
                if let Some(p) = trimmed(x + ox, y + oy, r.width as u32, r.height as u32, rgba) {
                    pics.push(p);
                }
            }
            let timeout = (page.timeout > 0.0).then_some(page.timeout);
            screens.push((packet.time, timeout.or(packet.duration), pics));
        }
        Decoded {
            canvas: Some(display.map_or((720, 576), |d| (d.0, d.1))),
            pictures: screens_to_pictures(screens),
        }
    }

    fn region(regions: &mut HashMap<u8, Region>, b: &[u8]) {
        let id = b[0];
        let fill = b[1] & 0x08 != 0;
        let width = usize::from(u16::from_be_bytes([b[2], b[3]]));
        let height = usize::from(u16::from_be_bytes([b[4], b[5]]));
        if width == 0 || height == 0 || width as u32 > MAX_SIDE || height as u32 > MAX_SIDE {
            return;
        }
        let depth = match (b[6] >> 2) & 7 {
            1 => 2,
            2 => 4,
            _ => 8,
        };
        let clut = b[7];
        let background = match depth {
            8 => b[8],
            4 => b[9] >> 4,
            _ => (b[9] >> 2) & 3,
        };
        let mut objects = Vec::new();
        let mut p = 10;
        while p + 6 <= b.len() {
            let oid = u16::from_be_bytes([b[p], b[p + 1]]);
            let kind = b[p + 2] >> 6;
            let x = usize::from(u16::from_be_bytes([b[p + 2], b[p + 3]]) & 0x0FFF);
            let y = usize::from(u16::from_be_bytes([b[p + 4], b[p + 5]]) & 0x0FFF);
            objects.push((oid, x, y));
            p += if kind == 1 || kind == 2 { 8 } else { 6 };
        }
        let r = regions.entry(id).or_insert_with(|| Region {
            width,
            height,
            depth,
            clut,
            pixels: vec![background; width * height],
            objects: Vec::new(),
        });
        if r.width != width || r.height != height || r.depth != depth {
            r.width = width;
            r.height = height;
            r.depth = depth;
            r.pixels = vec![background; width * height];
        } else if fill {
            r.pixels.fill(background);
        }
        r.clut = clut;
        r.objects = objects;
    }

    fn clut(c: &mut Clut, mut b: &[u8]) {
        while b.len() >= 2 {
            let id = usize::from(b[0]);
            let flags = b[1];
            let (y, cr, cb, t) = if flags & 1 != 0 {
                let Some(e) = b.get(2..6) else { return };
                b = &b[6..];
                (e[0], e[1], e[2], e[3])
            } else {
                let Some(e) = b.get(2..4) else { return };
                b = &b[4..];
                (
                    e[0] & 0xFC,
                    ((e[0] & 3) << 6) | ((e[1] >> 2) & 0x30),
                    (e[1] << 2) & 0xF0,
                    (e[1] & 3) * 0x55,
                )
            };
            let entry = if y == 0 {
                [0, 0, 0, 0]
            } else {
                let [r, g, bl] = ycbcr_to_rgb(y, cb, cr, false);
                [r, g, bl, 255 - t]
            };
            if flags & 0x80 != 0 && id < 4 {
                c.c4[id] = entry;
            }
            if flags & 0x40 != 0 && id < 16 {
                c.c16[id] = entry;
            }
            if flags & 0x20 != 0 {
                c.c256[id] = entry;
            }
        }
    }

    fn object(regions: &mut HashMap<u8, Region>, b: &[u8]) {
        let id = u16::from_be_bytes([b[0], b[1]]);
        if (b[2] >> 2) & 3 != 0 || b.len() < 7 {
            // Characters, not pixels: not drawn.
            return;
        }
        let top_len = usize::from(u16::from_be_bytes([b[3], b[4]]));
        let bottom_len = usize::from(u16::from_be_bytes([b[5], b[6]]));
        let Some(top) = b.get(7..7 + top_len) else { return };
        let bottom = if bottom_len == 0 {
            top
        } else {
            match b.get(7 + top_len..7 + top_len + bottom_len) {
                Some(x) => x,
                None => return,
            }
        };
        for r in regions.values_mut() {
            let places: Vec<(usize, usize)> = r.objects.iter().filter(|o| o.0 == id).map(|o| (o.1, o.2)).collect();
            for (x, y) in places {
                draw_field(r, top, x, y);
                draw_field(r, bottom, x, y + 1);
            }
        }
    }

    /// Bits read most significant first.
    struct Bits<'a> {
        d: &'a [u8],
        bit: usize,
    }

    impl Bits<'_> {
        fn get(&mut self, n: usize) -> usize {
            let mut v = 0;
            for _ in 0..n {
                let byte = self.d.get(self.bit / 8).copied().unwrap_or(0);
                v = (v << 1) | usize::from((byte >> (7 - self.bit % 8)) & 1);
                self.bit += 1;
            }
            v
        }
        fn align(&mut self) {
            self.bit = self.bit.div_ceil(8) * 8;
        }
        fn done(&self) -> bool {
            self.bit / 8 >= self.d.len()
        }
    }

    /// One field of an object's pixel data, drawn every other line from (x, y) of the region.
    fn draw_field(r: &mut Region, data: &[u8], x0: usize, y0: usize) {
        const MAP_2_4: [u8; 4] = [0x0, 0x7, 0x8, 0xF];
        const MAP_2_8: [u8; 4] = [0x00, 0x77, 0x88, 0xFF];
        let mut map_2_4 = MAP_2_4;
        let mut map_2_8 = MAP_2_8;
        let mut map_4_8: [u8; 16] = std::array::from_fn(|i| (i * 0x11) as u8);
        let mut bits = Bits { d: data, bit: 0 };
        let (mut x, mut y) = (x0, y0);
        let depth = r.depth;
        let put = |x: &mut usize, y: usize, run: usize, v: u8, r: &mut Region| {
            if y < r.height {
                for _ in 0..run {
                    if *x < r.width {
                        r.pixels[y * r.width + *x] = v;
                    }
                    *x += 1;
                }
            } else {
                *x += run;
            }
        };
        while !bits.done() {
            match bits.get(8) {
                0x10 => {
                    // 2-bit codes.
                    loop {
                        let (run, c) = match bits.get(2) {
                            0 => {
                                let switch_1 = bits.get(1);
                                let switch_2 = if switch_1 == 1 { 0 } else { bits.get(1) };
                                if switch_1 == 1 {
                                    (bits.get(3) + 3, bits.get(2))
                                } else if switch_2 == 1 {
                                    (1, 0)
                                } else {
                                    match bits.get(2) {
                                        0 => break,
                                        1 => (2, 0),
                                        2 => (bits.get(4) + 12, bits.get(2)),
                                        _ => (bits.get(8) + 29, bits.get(2)),
                                    }
                                }
                            }
                            c => (1, c),
                        };
                        let v = match depth {
                            2 => c as u8,
                            4 => map_2_4[c],
                            _ => map_2_8[c],
                        };
                        put(&mut x, y, run, v, r);
                        if bits.done() {
                            break;
                        }
                    }
                    bits.align();
                }
                0x11 => {
                    // 4-bit codes.
                    loop {
                        let (run, c) = match bits.get(4) {
                            0 => {
                                let switch_1 = bits.get(1);
                                let switch_2 = if switch_1 == 0 { 0 } else { bits.get(1) };
                                if switch_1 == 0 {
                                    match bits.get(3) {
                                        0 => break,
                                        n => (n + 2, 0),
                                    }
                                } else if switch_2 == 0 {
                                    (bits.get(2) + 4, bits.get(4))
                                } else {
                                    match bits.get(2) {
                                        0 => (1, 0),
                                        1 => (2, 0),
                                        2 => (bits.get(4) + 9, bits.get(4)),
                                        _ => (bits.get(8) + 25, bits.get(4)),
                                    }
                                }
                            }
                            c => (1, c),
                        };
                        let v = match depth {
                            8 => map_4_8[c],
                            2 => (c & 3) as u8,
                            _ => c as u8,
                        };
                        put(&mut x, y, run, v, r);
                        if bits.done() {
                            break;
                        }
                    }
                    bits.align();
                }
                0x12 => {
                    // 8-bit codes.
                    loop {
                        let (run, c) = match bits.get(8) {
                            0 => {
                                if bits.get(1) == 0 {
                                    match bits.get(7) {
                                        0 => break,
                                        n => (n, 0),
                                    }
                                } else {
                                    (bits.get(7), bits.get(8))
                                }
                            }
                            c => (1, c),
                        };
                        let v = match depth {
                            8 => c as u8,
                            4 => (c & 15) as u8,
                            _ => (c & 3) as u8,
                        };
                        put(&mut x, y, run, v, r);
                        if bits.done() {
                            break;
                        }
                    }
                }
                0x20 => {
                    for m in &mut map_2_4 {
                        *m = bits.get(4) as u8;
                    }
                }
                0x21 => {
                    for m in &mut map_2_8 {
                        *m = bits.get(8) as u8;
                    }
                }
                0x22 => {
                    for m in &mut map_4_8 {
                        *m = bits.get(8) as u8;
                    }
                }
                0xF0 => {
                    x = x0;
                    y += 2;
                }
                _ => return,
            }
        }
    }
}

/// ffprobe's hex dump of packet data (`-show_data`: "00000000: 1600 0000 ...  ascii") as bytes.
pub fn parse_hexdump(dump: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for line in dump.lines() {
        let Some((_, rest)) = line.split_once(": ") else { continue };
        // The hex part is the first 40 characters (8 groups of 4 digits and the spaces).
        let hex: String = rest.chars().take(40).filter(char::is_ascii_hexdigit).collect();
        if !hex.len().is_multiple_of(2) {
            bail!("odd hex dump line");
        }
        for i in (0..hex.len()).step_by(2) {
            out.push(u8::from_str_radix(&hex[i..i + 2], 16)?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(time: f64, data: Vec<u8>) -> Block {
        Block {
            time,
            duration: None,
            data,
        }
    }

    /// PGS run-length coding of rows of palette indices (as an encoder writes it).
    fn pgs_rle(rows: &[Vec<u8>]) -> Vec<u8> {
        let mut b = Vec::new();
        for row in rows {
            let mut i = 0;
            while i < row.len() {
                let c = row[i];
                let mut j = i;
                while j < row.len() && row[j] == c && j - i < 16383 {
                    j += 1;
                }
                let n = j - i;
                if c != 0 && n < 3 {
                    b.extend(std::iter::repeat_n(c, n));
                } else if c == 0 {
                    if n < 64 {
                        b.extend([0, n as u8]);
                    } else {
                        b.extend([0, 0x40 | (n >> 8) as u8, n as u8]);
                    }
                } else if n < 64 {
                    b.extend([0, 0x80 | n as u8, c]);
                } else {
                    b.extend([0, 0xC0 | (n >> 8) as u8, n as u8, c]);
                }
                i = j;
            }
            b.extend([0, 0]);
        }
        b
    }

    fn seg(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![kind];
        v.extend((body.len() as u16).to_be_bytes());
        v.extend(body);
        v
    }

    /// A display set showing `rows` at (x, y) on 1920×1080, or clearing it (`rows` empty).
    fn pgs_set(x: u16, y: u16, rows: &[Vec<u8>]) -> Vec<u8> {
        let mut pcs = vec![0x07, 0x80, 0x04, 0x38, 0x10, 0, 1];
        if rows.is_empty() {
            pcs.extend([0x00, 0, 0, 0]);
            return [seg(0x16, &pcs), seg(0x80, &[])].concat();
        }
        pcs.extend([0x80, 0, 0, 1, 0, 0, 0, 0]);
        pcs.extend(x.to_be_bytes());
        pcs.extend(y.to_be_bytes());
        let (w, h) = (rows[0].len() as u16, rows.len() as u16);
        let mut wds = vec![1, 0];
        for v in [x, y, w, h] {
            wds.extend(v.to_be_bytes());
        }
        // 1: opaque white, 2: half-transparent black.
        let pds = [0, 0, 1, 235, 128, 128, 255, 2, 16, 128, 128, 128];
        let data = pgs_rle(rows);
        let mut ods = vec![0, 0, 0, 0xC0];
        ods.extend(&((data.len() + 4) as u32).to_be_bytes()[1..]);
        ods.extend(w.to_be_bytes());
        ods.extend(h.to_be_bytes());
        ods.extend(&data);
        [seg(0x16, &pcs), seg(0x17, &wds), seg(0x14, &pds), seg(0x15, &ods), seg(0x80, &[])].concat()
    }

    fn rows() -> Vec<Vec<u8>> {
        let mut rows = vec![vec![0u8; 100]; 4];
        rows[1][10..90].fill(1);
        rows[2][10..12].fill(2);
        rows[2][12..90].fill(1);
        rows
    }

    #[test]
    fn pgs_shows_until_the_clearing_set() {
        let d = decode(
            Codec::Pgs,
            &[],
            &[block(10.0, pgs_set(200, 900, &rows())), block(13.5, pgs_set(0, 0, &[]))],
        );
        assert_eq!(d.canvas, Some((1920, 1080)));
        assert_eq!(d.pictures.len(), 1);
        let p = &d.pictures[0];
        assert_eq!((p.start, p.end), (10.0, Some(13.5)));
        // Trimmed to what isn't transparent: rows 1-2, columns 10-89.
        assert_eq!((p.x, p.y, p.width, p.height), (210, 901, 80, 2));
        assert_eq!(&p.rgba[0..4], &[255, 255, 255, 255]);
        let second_row = (80 * 4) as usize;
        assert_eq!(&p.rgba[second_row..second_row + 4], &[0, 0, 0, 128]);
    }

    #[test]
    fn pgs_runs_long_and_short() {
        let mut row = vec![0u8; 400];
        row[1] = 5;
        row[2..300].fill(7);
        let back = pgs::rle(&pgs_rle(&[row.clone(), row.clone()]), 400, 2);
        assert_eq!(&back[..400], &row[..]);
        assert_eq!(&back[400..], &row[..]);
    }

    #[test]
    fn pgs_without_an_end_is_open_and_a_new_set_ends_the_last() {
        let d = decode(
            Codec::Pgs,
            &[],
            &[block(1.0, pgs_set(0, 0, &rows())), block(2.0, pgs_set(0, 500, &rows()))],
        );
        assert_eq!(d.pictures.len(), 2);
        assert_eq!(d.pictures[0].end, Some(2.0));
        assert_eq!(d.pictures[1].end, None);
        assert_eq!(d.pictures[1].y, 501);
    }

    #[test]
    fn pgs_sup_headers_are_skipped() {
        let mut sup = Vec::new();
        let set = pgs_set(5, 5, &rows());
        // Each segment with a "PG" header in front, as in a .sup file.
        let mut p = 0;
        while p < set.len() {
            let len = u16::from_be_bytes([set[p + 1], set[p + 2]]) as usize;
            sup.extend(b"PG");
            sup.extend([0u8; 8]);
            sup.extend(&set[p..p + 3 + len]);
            p += 3 + len;
        }
        let d = decode(Codec::Pgs, &[], &[block(0.0, sup)]);
        assert_eq!(d.pictures.len(), 1);
    }

    /// VobSub nibble run-length coding of one line of values 0..=3.
    fn vob_line(row: &[u8], nibbles: &mut Vec<u8>) {
        let mut i = 0;
        while i < row.len() {
            let c = row[i];
            let mut j = i;
            while j < row.len() && row[j] == c && j - i < 255 {
                j += 1;
            }
            let n = j - i;
            let code = (n << 2) | c as usize;
            let digits = if n < 4 {
                1
            } else if n < 16 {
                2
            } else if n < 64 {
                3
            } else {
                4
            };
            for k in (0..digits).rev() {
                nibbles.push(((code >> (4 * k)) & 0xF) as u8);
            }
            i = j;
        }
        if nibbles.len() % 2 == 1 {
            nibbles.push(0);
        }
    }

    fn pack(nibbles: &[u8]) -> Vec<u8> {
        nibbles.chunks(2).map(|c| (c[0] << 4) | c.get(1).copied().unwrap_or(0)).collect()
    }

    /// A subpicture unit with `rows` at (x, y), shown from 0 for `stop` units of 1024/90 kHz.
    fn spu(x: u16, y: u16, rows: &[Vec<u8>], stop: u16) -> Vec<u8> {
        let (mut top, mut bottom) = (Vec::new(), Vec::new());
        for (i, r) in rows.iter().enumerate() {
            vob_line(r, if i % 2 == 0 { &mut top } else { &mut bottom });
        }
        let (top, bottom) = (pack(&top), pack(&bottom));
        let top_at = 4u16;
        let bottom_at = top_at + top.len() as u16;
        let ctrl = bottom_at + bottom.len() as u16;
        let (x2, y2) = (x + rows[0].len() as u16 - 1, y + rows.len() as u16 - 1);
        let mut c = Vec::new();
        let second = ctrl + 4 + 1 + 3 + 3 + 7 + 5 + 1;
        c.extend(0u16.to_be_bytes());
        c.extend(second.to_be_bytes());
        c.push(0x01);
        // Value 3 → colour 1, 2 → 2, 1 → 3, 0 → 0; values 1-3 opaque, 0 transparent.
        c.extend([0x03, 0x12, 0x30, 0x04, 0xFF, 0xF0, 0x05]);
        c.extend([(x >> 4) as u8, (((x & 0xF) << 4) | (x2 >> 8)) as u8, x2 as u8]);
        c.extend([(y >> 4) as u8, (((y & 0xF) << 4) | (y2 >> 8)) as u8, y2 as u8]);
        c.push(0x06);
        c.extend(top_at.to_be_bytes());
        c.extend(bottom_at.to_be_bytes());
        c.push(0xFF);
        c.extend(stop.to_be_bytes());
        c.extend(second.to_be_bytes());
        c.extend([0x02, 0xFF]);
        let mut out = Vec::new();
        let size = 4 + top.len() + bottom.len() + c.len();
        out.extend((size as u16).to_be_bytes());
        out.extend(ctrl.to_be_bytes());
        out.extend(top);
        out.extend(bottom);
        out.extend(c);
        out
    }

    const IDX: &str = "# VobSub index file\nsize: 720x480\npalette: 000000, ffffff, 808080, 0000ff, 000000, 000000, 000000, 000000, 000000, 000000, 000000, 000000, 000000, 000000, 000000, 000000\n";

    #[test]
    fn vobsub_picture_colours_and_stop_delay() {
        let mut rows = vec![vec![0u8; 40]; 6];
        for r in rows.iter_mut().skip(1).take(4) {
            r[4..36].fill(3);
            r[4] = 1;
        }
        let pkt = spu(100, 400, &rows, 264); // 264 × 1024 / 90000 ≈ 3.004 s
        let d = decode(Codec::VobSub, IDX.as_bytes(), &[block(20.0, pkt)]);
        assert_eq!(d.canvas, Some((720, 480)));
        assert_eq!(d.pictures.len(), 1);
        let p = &d.pictures[0];
        assert_eq!(p.start, 20.0);
        assert!((p.end.unwrap() - 23.004).abs() < 0.001, "{:?}", p.end);
        assert_eq!((p.x, p.y, p.width, p.height), (104, 401, 32, 4));
        // Value 1 → colour 3 (blue), value 3 → colour 1 (white).
        assert_eq!(&p.rgba[0..4], &[0, 0, 255, 255]);
        assert_eq!(&p.rgba[4..8], &[255, 255, 255, 255]);
    }

    #[test]
    fn vobsub_palette_from_idx() {
        let (size, palette) = vobsub::idx(IDX.as_bytes());
        assert_eq!(size, Some((720, 480)));
        assert_eq!(palette[1], [255, 255, 255]);
        assert_eq!(palette[3], [0, 0, 255]);
    }

    /// A DVB display set: a page with one 4-bit region of `w`×`h` at (x, y) holding one object
    /// (a filled box of colour 1), or an empty page.
    fn dvb_set(region: Option<(u16, u16, u16, u16)>, timeout: u8) -> Vec<u8> {
        let segment = |kind: u8, body: &[u8]| {
            let mut v = vec![0x0F, kind, 0, 1];
            v.extend((body.len() as u16).to_be_bytes());
            v.extend(body);
            v
        };
        let mut out = vec![0x20, 0x00];
        let Some((x, y, w, h)) = region else {
            out.extend(segment(0x10, &[timeout, 0x14]));
            out.push(0xFF);
            return out;
        };
        let mut page = vec![timeout, 0x04, 0, 0];
        page.extend(x.to_be_bytes());
        page.extend(y.to_be_bytes());
        out.extend(segment(0x10, &page));
        let mut reg = vec![0, 0x08];
        reg.extend(w.to_be_bytes());
        reg.extend(h.to_be_bytes());
        reg.extend([2 << 2 | 2 << 5, 0, 0, 0]);
        reg.extend([0, 1, 0, 0, 0, 0]);
        out.extend(segment(0x11, &reg));
        // CLUT 0: entry 1 white (full range), 4-bit.
        out.extend(segment(0x12, &[0, 0, 1, 0x41, 235, 128, 128, 0]));
        // Object 1: every line is w pixels of colour 1, then the end of the line.
        let mut field = Vec::new();
        for _ in 0..h.div_ceil(2) {
            field.push(0x11);
            // 4-bit run of colour 1: 0000 1 1 11 (8-bit length + 25) … use runs of one pixel.
            let mut nib = vec![1u8; usize::from(w)];
            nib.extend([0, 0]);
            if nib.len() % 2 == 1 {
                nib.push(0);
            }
            field.extend(nib.chunks(2).map(|c| (c[0] << 4) | c[1]));
            field.push(0xF0);
        }
        let mut obj = vec![0, 1, 0];
        obj.extend((field.len() as u16).to_be_bytes());
        obj.extend(0u16.to_be_bytes());
        obj.extend(&field);
        out.extend(segment(0x13, &obj));
        out.extend(segment(0x80, &[]));
        out.push(0xFF);
        out
    }

    #[test]
    fn dvb_page_shows_until_the_empty_page() {
        let d = decode(
            Codec::Dvb,
            &[],
            &[block(5.0, dvb_set(Some((100, 500, 8, 4)), 10)), block(7.0, dvb_set(None, 10))],
        );
        assert_eq!(d.canvas, Some((720, 576)));
        assert_eq!(d.pictures.len(), 1);
        let p = &d.pictures[0];
        assert_eq!((p.start, p.end), (5.0, Some(7.0)));
        assert_eq!((p.x, p.y, p.width, p.height), (100, 500, 8, 4));
        assert_eq!(&p.rgba[0..4], &[255, 255, 255, 255]);
    }

    #[test]
    fn dvb_page_times_out() {
        let d = decode(
            Codec::Dvb,
            &[],
            &[block(5.0, dvb_set(Some((0, 0, 4, 2)), 3)), block(30.0, dvb_set(None, 3))],
        );
        assert_eq!(d.pictures[0].end, Some(8.0));
    }

    #[test]
    fn hexdump() {
        let dump =
            "\n00000000: 1600 0000 0102 0304 0506 0708 090a 0b0c  ................\n00000010: ff                                       .\n";
        let b = parse_hexdump(dump).unwrap();
        assert_eq!(b.len(), 17);
        assert_eq!(b[0], 0x16);
        assert_eq!(b[16], 0xFF);
    }

    #[test]
    fn png_round_trip() {
        let p = Picture {
            start: 0.0,
            end: None,
            x: 0,
            y: 0,
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 0, 0, 0],
        };
        let png = png(&p).unwrap();
        let img = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(img.as_raw(), &p.rgba);
    }

    #[test]
    fn codec_names() {
        assert_eq!(Codec::from_name("PGSSUB"), Some(Codec::Pgs));
        assert_eq!(Codec::from_name("dvd_subtitle"), Some(Codec::VobSub));
        assert_eq!(Codec::from_name("S_DVBSUB"), Some(Codec::Dvb));
        assert_eq!(Codec::from_name("subrip"), None);
    }
}
