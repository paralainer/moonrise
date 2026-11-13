//! Picture subtitle tracks (PGS, DVD, DVB) served as timed images, a 30-second window at a time
//! (`GET /v1/items/{id}/subtitles/{index}/cues?window=k`, as text tracks are): the apps draw
//! them over the video, which plays as it is (no transcode).
//!
//! A window is read on its own, so the first picture shows as soon as a text window would and a
//! seek costs one more window: the subtitle blocks around it (`LEAD` before, for pictures still
//! showing, and `TAIL` after, for the end of the last one) are read straight out of a Matroska
//! file (by the index, or by scanning that stretch of clusters), or by ffprobe for other
//! containers; decoded (`bitmap.rs`); and answered as PNGs with their place on the track's
//! canvas. Windows are kept in memory (`KEPT_BYTES`), and the same window asked for twice at once
//! is read once.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::{bail, Context, Result};
use base64::Engine as _;
use serde::Serialize;

use crate::{
    media::Media,
    subtitles::{
        bitmap::{self, Codec},
        subs::{self, FileIndex, Subs, WindowRead},
        subx::{self, Block},
    },
};

/// Seconds read before a window, for pictures that started earlier and still show.
const LEAD: f64 = 10.0;
/// Seconds read after a window, for the end of a picture that starts in it.
const TAIL: f64 = 10.0;
/// A picture whose end isn't in what was read shows at most this long.
const LONGEST: f64 = 10.0;
/// Windows kept in memory, in bytes of PNG.
const KEPT_BYTES: usize = 48 * 1024 * 1024;

/// A window of a picture track, as answered.
#[derive(Debug, Serialize)]
pub struct PictureWindow {
    pub index: i64,
    pub window: usize,
    pub from: f64,
    pub to: f64,
    pub span: f64,
    /// Always false: a picture track is only ever served in windows.
    pub complete: bool,
    /// "memory", "index", "scan" or "ffprobe".
    pub source: &'static str,
    /// The frame the pictures are placed on (the video's frame they were made for); the app
    /// scales it onto the picture it shows.
    pub canvas: Canvas,
    pub cues: Arc<Vec<PictureCue>>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
}

/// A picture showing from `start` to `end` (the title's timeline, as text cues).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PictureCue {
    pub start: f64,
    pub end: f64,
    /// Empty: there is no text.
    pub text: &'static str,
    pub image: CueImage,
}

/// Where on the canvas, and the PNG (base64).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CueImage {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub png: String,
}

type Made = (Canvas, Arc<Vec<PictureCue>>);

/// Windows made, and windows being made.
#[derive(Default)]
pub struct Cache {
    kept: Mutex<HashMap<(String, i64, usize), (Made, usize, Instant)>>,
    inflight: Mutex<HashMap<(String, i64, usize), Arc<tokio::sync::OnceCell<Result<&'static str, String>>>>>,
}

impl Cache {
    fn get(&self, key: &(String, i64, usize)) -> Option<Made> {
        let mut kept = self.kept.lock().unwrap();
        let e = kept.get_mut(key)?;
        e.2 = Instant::now();
        Some(e.0.clone())
    }

    fn put(&self, key: (String, i64, usize), made: Made) {
        let bytes: usize = made.1.iter().map(|c| c.image.png.len()).sum::<usize>() + 256;
        let mut kept = self.kept.lock().unwrap();
        let mut total: usize = kept.values().map(|e| e.1).sum();
        while total + bytes > KEPT_BYTES && !kept.is_empty() {
            let oldest = kept.iter().min_by_key(|(_, e)| e.2).map(|(k, _)| k.clone()).unwrap();
            total -= kept.remove(&oldest).map_or(0, |e| e.1);
        }
        kept.insert(key, (made, bytes, Instant::now()));
    }
}

/// Whether a track is a picture track the apps can be sent as images.
pub fn servable(codec: &str) -> bool {
    Codec::from_name(codec).is_some()
}

impl Subs {
    /// Window `k` of picture track `index`: the pictures showing during [k·WINDOW, (k+1)·WINDOW)
    /// of the title's timeline.
    pub async fn picture_window(self: &Arc<Self>, media: &Arc<Media>, index: i64, k: usize) -> Result<PictureWindow> {
        let track = media
            .subtitles
            .iter()
            .find(|s| s.index == index && !s.text)
            .context("no such picture subtitle track")?;
        let codec = Codec::from_name(&track.codec).context("not a picture subtitle codec this server reads")?;
        let stream = track.stream.context("not a stream of the file")?;
        let (from, to) = (k as f64 * subs::WINDOW, (k + 1) as f64 * subs::WINDOW);
        let key = (media.file_key.clone(), stream, k);
        let answer = |(canvas, cues): Made, source| PictureWindow {
            index,
            window: k,
            from,
            to,
            span: subs::WINDOW,
            complete: false,
            source,
            canvas,
            cues,
        };
        if let Some(made) = self.pictures.get(&key) {
            return Ok(answer(made, "memory"));
        }
        let _reading = WindowRead::start();
        let cell = self.pictures.inflight.lock().unwrap().entry(key.clone()).or_default().clone();
        let source = cell
            .get_or_init(|| async {
                let t0 = Instant::now();
                let r = self.read_pictures(media, codec, stream, from, to).await;
                match &r {
                    Ok((source, made)) => {
                        tracing::info!(item = %media.item_id, index, window = k, source, cues = made.1.len(), ms = t0.elapsed().as_millis() as u64, "picture subtitle window read");
                    }
                    Err(e) => tracing::warn!(item = %media.item_id, index, window = k, "picture subtitle window: {e:#}"),
                }
                r.map(|(source, made)| {
                    self.pictures.put(key.clone(), made);
                    source
                })
                .map_err(|e| format!("{e:#}"))
            })
            .await
            .clone();
        self.pictures.inflight.lock().unwrap().remove(&key);
        let source = source.map_err(anyhow::Error::msg)?;
        let made = self.pictures.get(&key).context("window missing after the read")?;
        Ok(answer(made, source))
    }

    /// Reads and decodes the pictures of [from, to) of the title's timeline.
    async fn read_pictures(&self, media: &Media, codec: Codec, stream: i64, from: f64, to: f64) -> Result<(&'static str, Made)> {
        // The file's own clock.
        let (a, b) = ((from + media.origin - LEAD).max(0.0), to + media.origin + TAIL);
        let (blocks, private, source) = match self.file_index(media).await {
            Ok(FileIndex::Mkv { index, file, header }) => match subx::picture_want(&index, stream) {
                Some(want) => {
                    let private = index
                        .tracks
                        .get(stream as usize)
                        .map(|t| t.codec_private.clone())
                        .unwrap_or_default();
                    if subx::method(&index, std::slice::from_ref(&want)) == subx::Method::Cues {
                        let (blocks, h) = subx::picture_blocks_by_cues(&file, &index, &want, a, b, header).await?;
                        if header.is_none() {
                            self.store_index(
                                &media.file_key,
                                &FileIndex::Mkv {
                                    index: index.clone(),
                                    file,
                                    header: Some(h),
                                },
                            );
                        }
                        (blocks, private, "index")
                    } else {
                        let _turn = self.scan_gate.acquire().await?;
                        let (blocks, _) = subx::picture_blocks_by_scan(&file, &index, &want, a, b, &subs::yield_to_streams()).await?;
                        (blocks, private, "scan")
                    }
                }
                None => {
                    let (blocks, private) = self.probe_packets(media, stream, a, b).await?;
                    (blocks, private, "ffprobe")
                }
            },
            _ => {
                let (blocks, private) = self.probe_packets(media, stream, a, b).await?;
                (blocks, private, "ffprobe")
            }
        };
        let decoded = bitmap::decode(codec, &private, &blocks);
        let canvas = decoded
            .canvas
            .or_else(|| media.video.as_ref().map(|v| (v.width as u32, v.height as u32)))
            .unwrap_or((720, 576));
        let origin = media.origin;
        let cues = tokio::task::spawn_blocking(move || cues_in(decoded.pictures, origin, b, from, to)).await??;
        Ok((
            source,
            (
                Canvas {
                    width: canvas.0,
                    height: canvas.1,
                },
                Arc::new(cues),
            ),
        ))
    }

    /// The packets of `stream` in [a, b) of the file's clock, by ffprobe (containers other than
    /// Matroska), and the stream's codec private data.
    async fn probe_packets(&self, media: &Media, stream: i64, a: f64, b: f64) -> Result<(Vec<Block>, Vec<u8>)> {
        let mut args: Vec<String> = [
            "-v",
            "error",
            "-select_streams",
            &stream.to_string(),
            "-read_intervals",
            &format!("{a:.3}%{b:.3}"),
            "-show_packets",
            "-show_streams",
            "-show_data",
            "-of",
            "json",
        ]
        .map(String::from)
        .to_vec();
        args.extend(media.source.input_args());
        let out = tokio::process::Command::new(&self.ffprobe)
            .args(&args)
            .kill_on_drop(true)
            .output()
            .await
            .context("ffprobe")?;
        if !out.status.success() {
            bail!(
                "ffprobe: {}",
                crate::admin::logbuf::scrub(&String::from_utf8_lossy(&out.stderr).lines().take(3).collect::<Vec<_>>().join(" / "))
            );
        }
        probed_packets(&out.stdout, a, b)
    }
}

/// ffprobe's JSON (`-show_packets -show_streams -show_data`) as the packets starting in [a, b)
/// and the stream's extradata.
pub(crate) fn probed_packets(json: &[u8], a: f64, b: f64) -> Result<(Vec<Block>, Vec<u8>)> {
    let v: serde_json::Value = serde_json::from_slice(json).context("ffprobe's output")?;
    let private = match v["streams"][0]["extradata"].as_str() {
        Some(d) => bitmap::parse_hexdump(d)?,
        None => Vec::new(),
    };
    let num = |p: &serde_json::Value, k: &str| p[k].as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| p[k].as_f64());
    let mut blocks = Vec::new();
    for p in v["packets"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let Some(time) = num(p, "pts_time").or_else(|| num(p, "dts_time")) else {
            continue;
        };
        if time < a || time >= b {
            continue;
        }
        let data = bitmap::parse_hexdump(p["data"].as_str().unwrap_or_default())?;
        blocks.push(Block {
            time,
            duration: num(p, "duration_time").filter(|d| *d > 0.0),
            data,
        });
    }
    blocks.sort_by(|x, y| x.time.total_cmp(&y.time));
    Ok((blocks, private))
}

/// The pictures showing during [from, to) of the timeline, as cues: moved back by `origin` (the
/// file's clock to the timeline), an end not in what was read (up to `read_to`, the file's
/// clock) set to at most `LONGEST` after the start, and PNG coded.
pub(crate) fn cues_in(pictures: Vec<bitmap::Picture>, origin: f64, read_to: f64, from: f64, to: f64) -> Result<Vec<PictureCue>> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut cues = Vec::new();
    for p in pictures {
        let end = p.end.unwrap_or_else(|| read_to.min(p.start + LONGEST));
        let (start, end) = (p.start - origin, end - origin);
        if end <= from || start >= to || end <= start {
            continue;
        }
        let png = bitmap::png(&p)?;
        cues.push(PictureCue {
            start: (start * 1000.0).round() / 1000.0,
            end: (end * 1000.0).round() / 1000.0,
            text: "",
            image: CueImage {
                x: p.x,
                y: p.y,
                width: p.width,
                height: p.height,
                png: b64.encode(png),
            },
        });
    }
    Ok(cues)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pic(start: f64, end: Option<f64>) -> bitmap::Picture {
        bitmap::Picture {
            start,
            end,
            x: 10,
            y: 20,
            width: 1,
            height: 1,
            rgba: vec![255, 255, 255, 255],
        }
    }

    #[test]
    fn cues_on_the_timeline_and_in_the_window() {
        // A file whose clock starts at 2 s: a picture at 33-35 s of the file is 31-33 s of the
        // title, in window 1 (30-60 s); one at 10-12 s is in window 0 only.
        let cues = cues_in(vec![pic(12.0, Some(14.0)), pic(33.0, Some(35.0))], 2.0, 72.0, 30.0, 60.0).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!((cues[0].start, cues[0].end), (31.0, 33.0));
        assert_eq!((cues[0].image.x, cues[0].image.y), (10, 20));
        assert!(!cues[0].image.png.is_empty());
    }

    #[test]
    fn a_picture_still_showing_at_the_window_start_is_in_it() {
        let cues = cues_in(vec![pic(28.0, Some(31.0))], 0.0, 70.0, 30.0, 60.0).unwrap();
        assert_eq!(cues.len(), 1);
    }

    #[test]
    fn an_open_end_is_cut_short() {
        // No end in what was read (to 70 s): at most LONGEST.
        let cues = cues_in(vec![pic(40.0, None)], 0.0, 70.0, 30.0, 60.0).unwrap();
        assert_eq!(cues[0].end, 40.0 + LONGEST);
        let cues = cues_in(vec![pic(65.0, None)], 0.0, 70.0, 60.0, 90.0).unwrap();
        assert_eq!(cues[0].end, 70.0);
    }

    /// A Matroska file with one PGS track (made by ffmpeg from a `.sup`): pictures at 0, 5 and
    /// 10 s, each cleared 3 s later.
    const PGS_MKV: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/media/pgs-only.mkv");

    #[tokio::test]
    async fn a_matroska_picture_track_read_and_decoded() {
        let file = subx::File::open_async(PGS_MKV, true).await.unwrap();
        let index = crate::media::mkv::index_with(&file).await.unwrap();
        let want = subx::picture_want(&index, 0).expect("a PGS track");
        // Window [0, 9) with nothing before it: the first two pictures and their clears.
        let (blocks, method) = match subx::method(&index, std::slice::from_ref(&want)) {
            subx::Method::Cues => (
                subx::picture_blocks_by_cues(&file, &index, &want, 0.0, 9.0, None).await.unwrap().0,
                "cues",
            ),
            subx::Method::Scan => (
                subx::picture_blocks_by_scan(&file, &index, &want, 0.0, 9.0, &subs::yield_to_streams())
                    .await
                    .unwrap()
                    .0,
                "scan",
            ),
        };
        assert_eq!(blocks.len(), 4, "{method}");
        let private = index.tracks[0].codec_private.clone();
        let d = bitmap::decode(Codec::Pgs, &private, &blocks);
        assert_eq!(d.canvas, Some((1920, 1080)));
        let times: Vec<(f64, Option<f64>)> = d.pictures.iter().map(|p| (p.start, p.end)).collect();
        assert_eq!(times, [(0.0, Some(3.0)), (5.0, Some(8.0))]);
        // Bottom centre, then the window's cues on the timeline.
        assert!(
            d.pictures[0].y > 900 && d.pictures[0].x > 700,
            "{:?}",
            (d.pictures[0].x, d.pictures[0].y)
        );
        let cues = cues_in(d.pictures, 0.0, 9.0, 0.0, 30.0).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!((cues[1].start, cues[1].end), (5.0, 8.0));
    }

    #[test]
    fn ffprobe_packets_and_extradata() {
        let json = br#"{"packets":[
            {"pts_time":"5.000000","duration_time":"3.000000","data":"\n00000000: 0102 03                                  ...\n"},
            {"pts_time":"1.000000","data":"\n00000000: 09                                       .\n"},
            {"pts_time":"9.500000","data":"\n00000000: 04                                       .\n"}],
            "streams":[{"extradata":"\n00000000: 7369 7a65                                size\n"}]}"#;
        let (blocks, private) = probed_packets(json, 0.0, 9.0).unwrap();
        assert_eq!(private, b"size");
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            blocks[0],
            Block {
                time: 1.0,
                duration: None,
                data: vec![9]
            }
        );
        assert_eq!(
            blocks[1],
            Block {
                time: 5.0,
                duration: Some(3.0),
                data: vec![1, 2, 3]
            }
        );
    }
}
