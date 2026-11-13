//! Everything needed to serve one title: where to read it, its tracks (the library's labels,
//! ffprobe's exact codec details), whether it can be served without re-encoding, and its
//! segment boundaries (from the file's own keyframes).

#[cfg(test)]
mod args_golden_tests;
pub mod audio;
#[cfg(test)]
mod audio_run_golden_tests;
pub mod credits;
#[cfg(test)]
mod detect_golden_tests;
pub mod hwaccel;
pub mod index_backfill;
pub mod item;
#[cfg(test)]
mod load_golden_tests;
pub mod mkv;
pub mod moov;
pub mod mp4;
pub mod scan;
pub mod segment;
pub mod segment_rules;
pub mod transcode;
pub mod trickplay;

use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;

use crate::{
    admin::config::{Config, Feature},
    db::Db,
    media::audio::Conversion,
    media::scan::Scanner,
    storage::source::Source,
};

/// Segments are cut at the first keyframe at least this long after the previous boundary.
const TARGET_SEGMENT: f64 = 6.0;

/// What `Media::load` needs besides the item.
pub struct LoadCtx<'a> {
    pub http: &'a reqwest::Client,
    pub config: &'a Config,
    /// The probe cache (None: off).
    pub db: Option<&'a Db>,
    /// The database, for the library's file ids (None: off).
    pub library_db: Option<&'a Db>,
    pub scanner: &'a Arc<Scanner>,
}

#[derive(Debug, Serialize, Clone)]
pub struct Media {
    pub item_id: String,
    pub media_source_id: String,
    /// Changes when the file does (cache keys include it).
    pub version: String,
    /// The file's identity for persistent data (extracted subtitles): its path, size and mtime
    /// (`version`).
    #[serde(skip)]
    pub file_key: String,
    /// The file's size in bytes, when known.
    #[serde(skip)]
    pub size: Option<u64>,
    /// The library's id of the file (survives renames), when it has been scanned.
    #[serde(skip)]
    pub file_id: Option<String>,
    /// The item (names, provider ids, season/episode: subtitle search).
    #[serde(skip)]
    pub item: Arc<item::Item>,
    #[serde(skip)]
    pub source: Source,
    pub source_kind: &'static str,
    /// Where the probe and keyframe index came from: "hit" (database), "miss" (read now and
    /// stored), "off" (no cache: disabled, or not a local file).
    #[serde(skip)]
    pub probe_cache: &'static str,
    /// ffprobe's format name.
    pub container: String,
    /// How long the title runs, from `origin` (the playlists' timeline).
    pub duration: f64,
    pub supported: bool,
    pub reasons: Vec<String>,
    /// "copy": every track as it is; "convert": video copied, some audio converted;
    /// "transcode": the video is re-encoded (the playback answer says this for the variant
    /// it serves).
    pub method: &'static str,
    /// The keyframe index isn't there yet (a background scan is running): ask again later.
    pub pending: bool,
    /// Not worth keeping in the cache: the file is damaged (`DAMAGED`) and now marked `broken`
    /// (the title's next load may pick another of its versions), or a read of its index failed
    /// (the share; the next request tries again).
    #[serde(skip)]
    pub look_again: bool,
    /// The file was found damaged on this load (and marked `broken`).
    #[serde(skip)]
    pub damaged: bool,
    pub video: Option<VideoTrack>,
    pub audio: Vec<AudioTrack>,
    pub subtitles: Vec<SubtitleTrack>,
    /// Segment start times on the file's own clock; segment i spans boundaries[i]..boundaries[i+1]
    /// (or the end, `origin + duration`). Segment 0 is cut from the file's beginning (0).
    #[serde(skip)]
    pub boundaries: Vec<f64>,
    /// Per boundary: how far from it to ask ffmpeg to seek so it lands on that keyframe
    /// (see `seek_offset`).
    #[serde(skip)]
    pub seek_offsets: Vec<f64>,
    /// The file's first timestamp (`format.start_time`). The boundaries are the file's own
    /// timestamps, and ffmpeg adds this to every `-ss` (see `Media::input_seek`).
    #[serde(skip)]
    pub start_time: f64,
    /// Where the title's timeline starts on the file's own clock (`start_time`, none below 0):
    /// players count it as 0, so the playlists, a session's `start` and the cues of the file's
    /// own subtitle tracks are that much before the file's timestamps (`Timeline`).
    #[serde(skip)]
    pub origin: f64,
}

#[derive(Debug, Serialize, Clone)]
pub struct VideoTrack {
    /// The track index.
    pub index: i64,
    /// ffprobe's stream index in the file (what ffmpeg maps).
    pub stream: i64,
    pub codec: String,
    pub codecs_attr: Option<String>,
    pub width: i64,
    pub height: i64,
    pub frame_rate: Option<f64>,
    pub range: &'static str,
    pub bitrate: i64,
    /// Display aspect ratio (width/height with the sample aspect applied).
    #[serde(skip)]
    pub dar: f64,
    #[serde(skip)]
    pub pix_fmt: String,
    /// ffprobe's profile name ("High", "High 4:4:4 Predictive", "Main 10", "Rext").
    #[serde(skip)]
    pub profile: String,
    /// Dolby Vision with no HDR10/SDR base layer (profile 5): not transcoded.
    #[serde(skip)]
    pub dovi_only: bool,
    /// The Dolby Vision configuration record, when the stream has one.
    #[serde(skip)]
    pub dovi: Option<Dovi>,
    /// Pictures the decoder holds back before showing one (ffprobe's `has_b_frames`).
    #[serde(skip)]
    pub reorder: i64,
    /// Why the video can't be copied into HLS (None: it can).
    #[serde(skip)]
    pub copy_reason: Option<String>,
    /// H.264 whose NAL units carry 1–3 byte lengths (`avcC`): Apple's players refuse it, so a
    /// copy rewrites them as 4-byte lengths (through Annex B: `h264_mp4toannexb`).
    #[serde(skip)]
    pub short_nal_lengths: bool,
}

/// A Dolby Vision configuration record (ffprobe's `DOVI configuration record` side data).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dovi {
    pub profile: u8,
    pub level: u8,
    pub rpu: bool,
    pub el: bool,
    pub bl: bool,
    /// The base layer's compatibility: 1 HDR10, 2 SDR, 4 HLG, 0 none (profile 5).
    pub compat: u8,
}

impl Dovi {
    pub fn from_probe(stream: &Value) -> Option<Dovi> {
        let sd = stream["side_data_list"]
            .as_array()?
            .iter()
            .find(|d| d.get("dv_profile").is_some())?;
        let n = |k: &str| sd[k].as_u64().and_then(|v| u8::try_from(v).ok());
        Some(Dovi {
            profile: n("dv_profile")?,
            level: n("dv_level").unwrap_or(0),
            rpu: n("rpu_present_flag") == Some(1),
            el: n("el_present_flag") == Some(1),
            bl: n("bl_present_flag") == Some(1),
            compat: n("dv_bl_signal_compatibility_id").unwrap_or(0),
        })
    }

    /// How a player that does Dolby Vision is told about it in HLS (None: it isn't; the copy
    /// plays as its base layer, as for every other player). Profile 8.1 (HDR10 base) and 8.4
    /// (HLG base): `SUPPLEMENTAL-CODECS` next to the base layer's `hvc1`; profile 5 (no base
    /// layer): `dvh1` as the codec itself. Profile 7 (a second layer) and 8.2 (SDR base) aren't
    /// played as Dolby Vision by Apple's players: left to their base layer.
    pub fn signal(&self) -> Option<DoviSignal> {
        if !self.rpu || self.el || self.level == 0 {
            return None;
        }
        let codec = format!("dvh1.{:02}.{:02}", self.profile, self.level);
        match (self.profile, self.compat, self.bl) {
            (8, 1, true) => Some(DoviSignal::Supplemental(format!("{codec}/db1p"))),
            (8, 4, true) => Some(DoviSignal::Supplemental(format!("{codec}/db4h"))),
            (5, 0, true) => Some(DoviSignal::Codec(codec)),
            _ => None,
        }
    }

    /// The configuration box for the sample entry: `dvcC` up to profile 7, `dvvC` after (ISO/IEC
    /// 14496-15 Dolby Vision streams, 24 bytes of `DOVIDecoderConfigurationRecord`).
    pub fn config_box(&self) -> Vec<u8> {
        let kind: &[u8; 4] = if self.profile <= 7 { b"dvcC" } else { b"dvvC" };
        let mut body = vec![1u8, 0]; // version 1.0
        let bits: u16 = ((self.profile as u16 & 0x7F) << 9)
            | ((self.level as u16 & 0x3F) << 3)
            | ((self.rpu as u16) << 2)
            | ((self.el as u16) << 1)
            | self.bl as u16;
        body.extend(bits.to_be_bytes());
        body.push((self.compat & 0x0F) << 4); // then 28 reserved bits
        body.extend([0u8; 19]);
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend(body);
        out
    }
}

/// See [`Dovi::signal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoviSignal {
    /// `SUPPLEMENTAL-CODECS` (the copy's sample entry stays `hvc1`).
    Supplemental(String),
    /// `CODECS` for the video (the sample entry becomes `dvh1`).
    Codec(String),
}

impl VideoTrack {
    pub fn copyable(&self) -> bool {
        self.copy_reason.is_none()
    }

    pub fn hdr(&self) -> bool {
        self.range != "SDR"
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct AudioTrack {
    /// The track index: what the app sends and the stream URLs carry.
    pub index: i64,
    /// ffprobe's stream index in the file.
    pub stream: i64,
    /// The source codec.
    pub codec: String,
    /// The codec as served (RFC 6381).
    pub codecs_attr: String,
    pub name: String,
    pub language: Option<String>,
    /// As served.
    pub channels: i64,
    pub default: bool,
    /// Served converted (see `conversion`), not copied.
    pub converted: bool,
    pub conversion: Option<Conversion>,
    /// The source is lossless (TrueHD, DTS-HD MA, FLAC, PCM, ALAC), with this many channels:
    /// what `Media::with_lossless` looks at.
    #[serde(skip)]
    pub lossless_source: bool,
    #[serde(skip)]
    pub source_channels: i64,
    /// The source's sample rate, and its timestamp unit in seconds.
    #[serde(skip)]
    pub sample_rate: i64,
    #[serde(skip)]
    pub time_base: f64,
}

#[derive(Debug, Serialize, Clone)]
pub struct SubtitleTrack {
    /// The track index; tracks fetched from OpenSubtitles have their own (10000 + n).
    pub index: i64,
    /// ffprobe's stream index (embedded tracks).
    pub stream: Option<i64>,
    /// Text (served as a WebVTT rendition) or pictures (PGS, DVD, DVB: served as images in
    /// windows, `subtitles/pictures.rs`, drawn by the app).
    pub text: bool,
    pub codec: String,
    pub name: String,
    pub language: Option<String>,
    pub forced: bool,
    pub default: bool,
    /// A sidecar file next to the media (numbered first), not a stream in it.
    pub external: bool,
    /// "embedded", "sidecar" or "opensubtitles".
    pub source: &'static str,
    /// Text tracks: the file as SRT (`.vtt` for WebVTT), relative to the server.
    pub url: Option<String>,
    /// A sidecar's path.
    #[serde(skip)]
    pub path: Option<String>,
    /// A fetched track's file (OpenSubtitles), in the data directory.
    #[serde(skip)]
    pub file: Option<std::path::PathBuf>,
    /// How its timing stands (`GET /v1/items/{id}/subtitles`; text tracks).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing: Option<crate::subtitles::subsync::Timing>,
}

impl SubtitleTrack {
    pub fn url_for(item_id: &str, index: i64) -> String {
        format!("/v1/items/{item_id}/subtitles/{index}.srt")
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Container {
    Matroska,
    Mp4,
    Avi,
    Other,
}

impl Container {
    fn of(format_name: &str) -> Self {
        if format_name.contains("matroska") || format_name.contains("webm") {
            Container::Matroska
        } else if format_name.contains("mp4") || format_name.contains("mov") {
            Container::Mp4
        } else if format_name == "avi" {
            Container::Avi
        } else {
            Container::Other
        }
    }
}

/// Track index → ffprobe's, for the streams inside the file.
///
/// Track indices are not ffprobe's: the library numbers external subtitle files **first** and
/// shifts every embedded stream by their count (older versions put them last), and it leaves
/// attachments out. So: try the plain offsets (the number of external streams, then none), and
/// accept one only if every embedded stream lands on a stream of the same kind; otherwise pair
/// them up in order, kind by kind.
pub fn map_streams(jf: &[item::MediaStream], probe: &[Value]) -> HashMap<i64, i64> {
    let embedded: Vec<&item::MediaStream> = jf.iter().filter(|s| !s.is_external).collect();
    let by_index = |i: i64| probe.iter().find(|p| p["index"].as_i64() == Some(i));
    let n_external = (jf.len() - embedded.len()) as i64;
    let mut offsets = vec![n_external, 0];
    if let (Some(j), Some(p)) = (
        embedded.iter().map(|s| s.index).min(),
        probe.iter().filter_map(|p| p["index"].as_i64()).min(),
    ) {
        offsets.push(j - p);
    }
    for off in offsets {
        let map: Option<HashMap<i64, i64>> = embedded
            .iter()
            .map(|s| {
                by_index(s.index - off)
                    .filter(|p| same_kind(&s.kind, p))
                    .map(|_| (s.index, s.index - off))
            })
            .collect();
        if let Some(map) = map {
            return map;
        }
    }
    let mut map = HashMap::new();
    for kind in ["Video", "Audio", "Subtitle", "EmbeddedImage", "Data"] {
        let ours = probe.iter().filter(|p| same_kind(kind, p));
        for (s, p) in embedded.iter().filter(|s| s.kind == kind).zip(ours) {
            if let Some(i) = p["index"].as_i64() {
                map.insert(s.index, i);
            }
        }
    }
    tracing::warn!(?map, "stream indices matched by order");
    map
}

fn same_kind(jf_kind: &str, p: &Value) -> bool {
    let cover = p["disposition"]["attached_pic"].as_i64() == Some(1);
    match (jf_kind, p["codec_type"].as_str().unwrap_or("")) {
        ("Video", "video") => !cover,
        ("EmbeddedImage", "video") => true,
        ("Audio", "audio") | ("Subtitle", "subtitle") | ("Data", "data") | ("Attachment", "attachment") => true,
        _ => false,
    }
}

/// The file has no audio track at all: neither the library nor ffprobe lists one (external audio
/// files aside). It plays as video alone, with no audio rendition.
fn silent(ms: &item::MediaSource, probed: &[Value]) -> bool {
    !ms.media_streams.iter().any(|s| s.kind == "Audio" && !s.is_external)
        && !probed.iter().any(|p| p["codec_type"].as_str() == Some("audio"))
}

fn copy_codecs_attr(codec: &str, profile: Option<&str>) -> Option<&'static str> {
    Some(match codec {
        "aac" => match profile {
            Some("HE-AAC") => "mp4a.40.5",
            Some("HE-AACv2") => "mp4a.40.29",
            _ => "mp4a.40.2",
        },
        "ac3" => "ac-3",
        "eac3" => "ec-3",
        _ => return None,
    })
}

/// A lossless audio codec (ffprobe's name and profile). DTS only with its lossless extension
/// (DTS-HD MA, with or without DTS:X); its core alone is lossy.
pub fn lossless_audio(codec: &str, profile: Option<&str>) -> bool {
    match codec {
        "truehd" | "mlp" | "flac" | "alac" => true,
        "dts" => profile.is_some_and(|p| p.starts_with("DTS-HD MA")),
        c => c.starts_with("pcm_"),
    }
}

fn rational(s: Option<&str>) -> Option<f64> {
    let (n, d) = s?.split_once('/')?;
    let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
    (d > 0.0 && n > 0.0).then(|| n / d)
}

impl Media {
    /// Segment `n` on the file's own clock, as it is cut.
    pub fn segment(&self, n: usize) -> Option<(f64, f64)> {
        let start = *self.boundaries.get(n)?;
        let end = self.boundaries.get(n + 1).copied().unwrap_or(self.origin + self.duration);
        Some((start, end))
    }

    /// Segment `n` on the title's timeline (what the playlists say: 0 is `origin`).
    pub fn segment_span(&self, n: usize) -> Option<(f64, f64)> {
        let (start, end) = self.segment(n)?;
        Some((start.max(self.origin) - self.origin, end - self.origin))
    }

    /// A time on the title's timeline (a session's `start`, a player's position) on the file's
    /// own clock.
    pub fn file_time(&self, t: f64) -> f64 {
        t + self.origin
    }

    /// What to give `-ss` before `-i` to reach `at`, a time on the file's own clock: ffmpeg adds
    /// the file's start time to it.
    pub fn input_seek(&self, at: f64) -> f64 {
        (at - self.start_time).max(0.0)
    }

    /// `input_seek` for the copy and the audio conversion, which place what they read by its own
    /// timestamps: for a start time below 0 they ask for `at` as before 0.31.8.
    pub fn copy_seek(&self, at: f64) -> f64 {
        (at - self.origin).max(0.0)
    }

    /// How far the file's own timestamps run ahead of the title's timeline, for what is read from
    /// the file outside the segment plan (subtitle timing's references, `subsync.rs`): `origin`
    /// for a start time past `LATE_START`; 0 for the rest, whose times are taken as they are, as
    /// before 0.31.11 (`OwnClock`).
    pub fn clock_offset(&self) -> f64 {
        if self.origin > LATE_START {
            self.origin
        } else {
            0.0
        }
    }

    pub fn is_last(&self, n: usize) -> bool {
        n + 1 >= self.boundaries.len()
    }

    pub fn audio_track(&self, index: i64) -> Option<&AudioTrack> {
        self.audio.iter().find(|a| a.index == index)
    }

    /// The media for a player that decodes FLAC and outputs more than 2 channels
    /// (`lossless=1`): its converted lossless multichannel tracks become FLAC, keeping every
    /// channel (`Conversion::lossless`). None: no track changes (the shared media serves it).
    pub fn with_lossless(&self) -> Option<Media> {
        let mut m = self.clone();
        let mut changed = false;
        for a in m.audio.iter_mut().filter(|a| a.conversion.is_some()) {
            if let Some(c) = Conversion::lossless(a.lossless_source, a.source_channels, a.sample_rate) {
                a.codecs_attr = c.codecs_attr().to_string();
                a.channels = c.channels as i64;
                a.conversion = Some(c);
                changed = true;
            }
        }
        changed.then_some(m)
    }

    /// The file is a local path or on an SMB share.
    pub async fn load(item: &item::Item, ctx: &LoadCtx<'_>) -> Result<Media> {
        let ms = item.media_sources.first().context("item has no media source")?;
        let (local, stamp) = Self::file_stamp(ms).await?;
        let source = Source::of(local);
        let mut reasons = Vec::new();

        // The probe and keyframe index persist in the database while the file is unchanged.
        let cache_key = stamp.as_ref().zip(ctx.db);
        let (probe, probe_cache, cached_keyframes) = Self::probe_cached(ctx, &source, cache_key).await?;
        let streams = probe["streams"].as_array().cloned().unwrap_or_default();
        let map = map_streams(&ms.media_streams, &streams);
        let ours = |s: &item::MediaStream| {
            map.get(&s.index)
                .and_then(|i| streams.iter().find(|p| p["index"].as_i64() == Some(*i)))
        };
        let probed = probe["format"]["duration"]
            .as_str()
            .and_then(|d| d.parse().ok())
            .or(ms.run_time_ticks.map(|t| t as f64 / 1e7))
            .unwrap_or(0.0);
        let container_name = probe["format"]["format_name"].as_str().unwrap_or("").to_string();
        let container = Container::of(&container_name);
        let start_time = probe["format"]["start_time"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|s| s.is_finite())
            .unwrap_or(0.0);
        let timeline = Timeline::of(container, start_time, probed);

        let bitrate = source_bitrate(ms.bitrate, &probe, stamp.as_ref().map(|s| s.size), probed);
        let video = Self::video_of(ms, &ours, container, bitrate);
        let transcode = ctx.config.enabled(Feature::Transcode);
        match &video {
            None => reasons.push("no video".into()),
            // Transcoded instead (see transcode.rs), unless that is switched off.
            Some(v) if !transcode || v.dovi_only => {
                if let Some(r) = &v.copy_reason {
                    reasons.push(r.clone());
                }
            }
            _ => {}
        }

        let convert = ctx.config.enabled(Feature::Convert);
        let mut audio = Self::audio_tracks(ms, &ours, convert);
        if audio.is_empty() {
            // A file with no audio at all (a screen recording, a camera clip with the sound
            // removed) is served as video alone; one whose audio can't be played isn't.
            if !silent(ms, &streams) {
                reasons.push("no audio track playable as is".into());
            }
        } else if !audio.iter().any(|a| a.default) {
            audio[0].default = true;
        }

        let subtitles = Self::subtitle_tracks(item, ms, &map);

        // The keyframe index is made for every title with video, served or not: later phases
        // (transcoding) cut on the same boundaries.
        let mut damaged = false;
        let (boundaries, seek_offsets, pending, look_again, replanned) = match &video {
            None => (Vec::new(), Vec::new(), false, false, false),
            Some(v) => {
                let keyframes = Self::keyframes(
                    item,
                    ms,
                    ctx,
                    &source,
                    stamp.as_ref(),
                    cache_key,
                    cached_keyframes,
                    v,
                    container,
                    &container_name,
                    &mut damaged,
                )
                .await;
                Self::segments_of(keyframes, damaged, timeline, container, &mut reasons)
            }
        };

        let method = Self::method_of(video.as_ref(), &audio);
        let version = Self::version_of(
            ms,
            container,
            runs_could_begin_late(video.is_some(), container, start_time),
            replanned,
        );
        let file_key = Self::file_key_of(stamp.as_ref(), item, ms, &version);
        let size = stamp
            .as_ref()
            .map(|s| s.size)
            .or_else(|| probe["format"]["size"].as_str().and_then(|s| s.parse().ok()));
        let file_id = Self::library_file_id(stamp.as_ref(), ctx.library_db).await;
        Ok(Media {
            file_id,
            item_id: item.id.clone(),
            media_source_id: ms.id.clone(),
            version,
            file_key,
            size,
            item: Arc::new(item.clone()),
            source_kind: source.describe(),
            probe_cache,
            source,
            container: container_name,
            duration: timeline.length,
            supported: reasons.is_empty(),
            reasons,
            method,
            pending,
            look_again,
            damaged,
            video,
            audio,
            subtitles,
            boundaries,
            seek_offsets,
            start_time,
            origin: timeline.origin,
        })
    }

    /// The file's path and its stamp. The library's stamp (size, mtime) when it has one: nothing
    /// is asked of the share, which answers slowly while it is busy (`AppState::media` checks it
    /// afterwards). Otherwise the file itself.
    async fn file_stamp(ms: &item::MediaSource) -> Result<(&str, Option<scan::Stamp>)> {
        let (local, stamp) = match (ms.path.as_deref(), ms.stamp) {
            (Some(p), Some((size, mtime))) => (
                p,
                Some(scan::Stamp {
                    path: p.to_string(),
                    size,
                    mtime,
                }),
            ),
            (Some(p), None) => match crate::storage::vfs::stat(p).await {
                Ok(m) if m.is_file => (
                    p,
                    Some(scan::Stamp {
                        path: p.to_string(),
                        size: m.size,
                        mtime: m.mtime,
                    }),
                ),
                Ok(_) => anyhow::bail!("{p} isn't a file"),
                Err(e) => anyhow::bail!("the file can't be read: {e}"),
            },
            _ => anyhow::bail!("the item has no file"),
        };
        Ok((local, stamp))
    }

    /// The file's probe (from the cache, else ffprobe's, then cached), where it came from
    /// (`probe_cache`), and the cached keyframe index.
    async fn probe_cached(
        ctx: &LoadCtx<'_>,
        source: &Source,
        cache_key: Option<(&scan::Stamp, &Db)>,
    ) -> Result<(Value, &'static str, Option<Vec<f64>>)> {
        let cached = match cache_key {
            Some((s, db)) => db.probe_get(s.path.clone(), s.size, s.mtime).await.unwrap_or_else(|e| {
                tracing::warn!("probe cache read: {e:#}");
                None
            }),
            None => None,
        };
        let probe_cache = match (&cache_key, &cached) {
            (None, _) => "off",
            (Some(_), Some(_)) => "hit",
            (Some(_), None) => "miss",
        };
        let (probe, cached_keyframes) = match cached {
            Some(c) => (c.probe, c.keyframes),
            None => {
                let p = probe(&ctx.config.ffprobe, source).await?;
                if let Some((s, db)) = cache_key {
                    if let Err(e) = db.probe_put(s.path.clone(), s.size, s.mtime, p.clone()).await {
                        tracing::warn!("probe cache write: {e:#}");
                    }
                }
                (p, None)
            }
        };
        Ok((probe, probe_cache, cached_keyframes))
    }

    /// The video track (the first that isn't cover art) and whether it can be copied.
    fn video_of<'a>(
        ms: &item::MediaSource,
        ours: &impl Fn(&item::MediaStream) -> Option<&'a Value>,
        container: Container,
        bitrate: i64,
    ) -> Option<VideoTrack> {
        let video = ms
            .media_streams
            .iter()
            .filter(|s| s.kind == "Video" && !s.is_external)
            .find_map(|s| ours(s).map(|p| (s, p)))
            .map(|(s, p)| video_track(s, p, bitrate));
        let mut video = video;
        if let Some(v) = video.as_mut() {
            v.copy_reason = if v.codecs_attr.is_none() {
                Some(format!("video codec {} not playable as is", v.codec))
            } else if container == Container::Avi {
                // AVI keeps no presentation times for B-frames (packed bitstreams): copied video
                // would play out of order.
                Some("video in AVI isn't copied".into())
            } else {
                None
            };
        }
        video
    }

    /// The audio tracks served: copied, or converted (`convert`); the rest are left out.
    fn audio_tracks<'a>(ms: &item::MediaSource, ours: &impl Fn(&item::MediaStream) -> Option<&'a Value>, convert: bool) -> Vec<AudioTrack> {
        let mut audio: Vec<AudioTrack> = Vec::new();
        for s in ms.media_streams.iter().filter(|s| s.kind == "Audio" && !s.is_external) {
            let Some(p) = ours(s) else { continue };
            let Some(stream) = p["index"].as_i64() else { continue };
            let codec = p["codec_name"].as_str().unwrap_or("").to_string();
            let source_channels = p["channels"].as_i64().or(s.channels).unwrap_or(2);
            let sample_rate = p["sample_rate"].as_str().and_then(|r| r.parse().ok()).unwrap_or(48000);
            let lossless_source = lossless_audio(&codec, p["profile"].as_str());
            let (codecs_attr, channels, conversion) = match copy_codecs_attr(&codec, p["profile"].as_str()) {
                Some(attr) => (attr.to_string(), source_channels, None),
                None if convert => {
                    let c = Conversion::for_source(source_channels, sample_rate);
                    (c.codecs_attr().to_string(), c.channels as i64, Some(c))
                }
                None => continue,
            };
            let name = s
                .title
                .clone()
                .filter(|t| !t.is_empty())
                .or_else(|| s.display_title.clone())
                .unwrap_or_else(|| format!("Track {}", s.index));
            let name = if audio.iter().any(|a| a.name == name) {
                format!("{name} ({})", s.index)
            } else {
                name
            };
            audio.push(AudioTrack {
                index: s.index,
                stream,
                codec,
                codecs_attr,
                name,
                language: s.language.as_deref().map(bcp47),
                channels,
                default: s.is_default,
                converted: conversion.is_some(),
                conversion,
                lossless_source,
                source_channels,
                sample_rate,
                time_base: rational(p["time_base"].as_str()).unwrap_or(0.001),
            });
        }
        audio
    }

    /// The subtitle tracks: text ones (embedded or sidecar), and embedded picture ones the server
    /// can decode (PGS, DVD, DVB).
    fn subtitle_tracks(item: &item::Item, ms: &item::MediaSource, map: &HashMap<i64, i64>) -> Vec<SubtitleTrack> {
        let mut subtitles: Vec<SubtitleTrack> = Vec::new();
        for s in ms.media_streams.iter().filter(|s| s.kind == "Subtitle") {
            let stream = map.get(&s.index).copied();
            // Picture tracks are listed when they are embedded (sidecar .sup/.sub files aren't
            // read) and in a format the server decodes.
            if !s.is_text_subtitle_stream
                && (s.is_external || stream.is_none() || !crate::subtitles::pictures::servable(s.codec.as_deref().unwrap_or_default()))
            {
                continue;
            }
            // The display title: Moonrise matches the rendition to its overlay track by it.
            let name = s
                .display_title
                .clone()
                .filter(|t| !t.is_empty())
                .or_else(|| s.title.clone())
                .unwrap_or_else(|| format!("Subtitles {}", s.index));
            let name = if subtitles.iter().any(|t| t.text == s.is_text_subtitle_stream && t.name == name) {
                format!("{name} ({})", s.index)
            } else {
                name
            };
            subtitles.push(SubtitleTrack {
                index: s.index,
                stream: stream.filter(|_| !s.is_external),
                text: s.is_text_subtitle_stream,
                codec: s.codec.clone().unwrap_or_default(),
                name,
                language: s.language.as_deref().map(bcp47),
                forced: s.is_forced,
                default: s.is_default,
                external: s.is_external,
                source: if s.is_external { "sidecar" } else { "embedded" },
                url: s.is_text_subtitle_stream.then(|| SubtitleTrack::url_for(&item.id, s.index)),
                path: s.path.clone().filter(|_| s.is_external),
                file: None,
                timing: None,
            });
        }
        subtitles
    }

    /// The video's keyframe times: cached, read from the file's index (Matroska's Cues, MP4's
    /// `moov`; then cached), or from a scan of the file (None while it runs). `damaged`: the file
    /// was found damaged (and is marked `broken` in the library).
    #[allow(
        clippy::too_many_arguments,
        reason = "the steps' inputs, passed through; a struct for one call would only rename them"
    )]
    async fn keyframes(
        item: &item::Item,
        ms: &item::MediaSource,
        ctx: &LoadCtx<'_>,
        source: &Source,
        stamp: Option<&scan::Stamp>,
        cache_key: Option<(&scan::Stamp, &Db)>,
        cached_keyframes: Option<Vec<f64>>,
        v: &VideoTrack,
        container: Container,
        container_name: &str,
        damaged: &mut bool,
    ) -> Result<Option<Vec<f64>>> {
        match cached_keyframes {
            Some(kf) => Ok(Some(kf)),
            None => {
                let read = match container {
                    // Cues that can't be read: not served (a packet scan of a Matroska file
                    // would make every copy segment read the file from its start). A damaged
                    // file is marked `broken` in the library until it changes.
                    Container::Matroska => match mkv::keyframes(source, ctx.http).await {
                        Err(e) if mkv::failure(&e) == mkv::Failure::Damaged => {
                            tracing::warn!(item = %item.id, "damaged Matroska file: {e:#}");
                            *damaged = true;
                            if let (Some(s), Some(db)) = (stamp, ctx.library_db) {
                                let why = format!("damaged or incomplete: {e:#}");
                                if let Err(e) = crate::library::mark_damaged(db, s.path.clone(), s.size, s.mtime, why).await {
                                    tracing::warn!("marking a damaged file: {e:#}");
                                }
                            }
                            Some(Err(e))
                        }
                        r => Some(r),
                    },
                    Container::Mp4 if ctx.config.enabled(Feature::Mp4Index) => match moov::keyframes(source, ctx.http).await {
                        Ok(kf) => Some(Ok(kf)),
                        Err(e) => {
                            tracing::info!(item = %item.id, "no MP4 index ({e:#}): scanning instead");
                            None
                        }
                    },
                    _ => None,
                };
                match read {
                    Some(Ok(kf)) => {
                        if let Some((s, db)) = cache_key {
                            if let Err(e) = db.keyframes_put(s.path.clone(), s.size, s.mtime, kf.clone()).await {
                                tracing::warn!("keyframe cache write: {e:#}");
                            }
                        }
                        Ok(Some(kf))
                    }
                    Some(Err(e)) => Err(e),
                    None if ctx.config.enabled(Feature::ScanIndex) => {
                        // By file (a background scan of it is found: `index_backfill.rs`).
                        let key = match stamp {
                            Some(s) => s.key(v.stream),
                            None => format!("{}|{}|", item.id, ms.id),
                        };
                        let stamp = stamp.cloned().filter(|_| ctx.db.is_some());
                        let timing = scan_timing(container, &v.codec, v.reorder);
                        match ctx.scanner.request(key, source, v.stream, timing, stamp) {
                            scan::Status::Ready(kf) => Ok(Some(kf)),
                            scan::Status::Pending => Ok(None),
                            scan::Status::Failed(e) => Err(anyhow::anyhow!(e)),
                        }
                    }
                    None => Err(anyhow::anyhow!("no index for {container_name} (scan_index off)")),
                }
            }
        }
    }

    /// The segment boundaries and seek offsets from the keyframe index, `pending`, `look_again`,
    /// and whether the boundaries aren't the ones planned before 0.31.8 (`Timeline`); why the
    /// title can't be served without them goes into `reasons`.
    fn segments_of(
        keyframes: Result<Option<Vec<f64>>>,
        damaged: bool,
        timeline: Timeline,
        container: Container,
        reasons: &mut Vec<String>,
    ) -> (Vec<f64>, Vec<f64>, bool, bool, bool) {
        let mut boundaries = Vec::new();
        let mut seek_offsets = Vec::new();
        let mut pending = false;
        let mut look_again = false;
        let mut replanned = false;
        match keyframes {
            Ok(Some(kf)) => {
                boundaries = plan(&kf, timeline.origin, timeline.end());
                replanned = boundaries != plan(&kf, 0.0, timeline.probed);
                let mut sorted = kf.clone();
                sorted.sort_by(f64::total_cmp);
                seek_offsets = boundaries
                    .iter()
                    .map(|b| match container {
                        Container::Other => -BEFORE_KEYFRAME,
                        _ => seek_offset(&sorted, *b),
                    })
                    .collect();
            }
            Ok(None) => {
                pending = true;
                reasons.push("keyframe index pending (scanning the file)".into());
            }
            Err(_) if damaged => {
                look_again = true;
                reasons.push(DAMAGED.into());
            }
            Err(e) => {
                look_again = container == Container::Matroska && mkv::failure(&e) == mkv::Failure::Read;
                reasons.push(format!("no keyframe index: {e:#}"));
            }
        }
        (boundaries, seek_offsets, pending, look_again, replanned)
    }

    /// "transcode", "convert" or "copy" (see `Media::method`).
    fn method_of(video: Option<&VideoTrack>, audio: &[AudioTrack]) -> &'static str {
        if video.is_some_and(|v| !v.copyable()) {
            "transcode"
        } else if audio.iter().any(|a| a.converted) {
            "convert"
        } else {
            "copy"
        }
    }

    /// Changes when the file does (cache keys include it).
    fn version_of(ms: &item::MediaSource, container: Container, could_begin_late: bool, replanned: bool) -> String {
        // AVI keyframe indexes changed meaning in 0.22.1 (shown times, not decode times; see
        // `scan::parse`), and with them the segment boundaries: segments cached before (and
        // downloads started before) must not be mixed with the new ones.
        let plan = if container == Container::Avi { "|kf2" } else { "" };
        // A transcode run in these files could begin past its boundary, and such a segment is
        // cached with its pictures missing (`runs_could_begin_late`).
        let seek = if could_begin_late { "|ss2" } else { "" };
        // Boundaries planned on one clock since 0.31.8 (`Timeline`): only files with a start time
        // can get other ones, and only those get new segments.
        let timeline = if replanned { "|tl2" } else { "" };
        format!(
            "{:x}",
            fnv(&format!(
                "{}|{:?}|{:?}|{}{plan}{seek}{timeline}",
                ms.id,
                ms.path,
                ms.run_time_ticks,
                ms.bitrate.unwrap_or(0)
            ))
        )
    }

    /// The file's identity for persistent data (`Media::file_key`).
    fn file_key_of(stamp: Option<&scan::Stamp>, item: &item::Item, ms: &item::MediaSource, version: &str) -> String {
        match stamp {
            Some(s) => format!("{:016x}", fnv(&format!("{}|{}|{}", s.path, s.size, s.mtime))),
            None => format!("{:016x}", fnv(&format!("{}|{}|{version}", item.id, ms.id))),
        }
    }

    /// The library's id of the file, when it has been scanned.
    async fn library_file_id(stamp: Option<&scan::Stamp>, library_db: Option<&Db>) -> Option<String> {
        match (stamp, library_db) {
            (Some(s), Some(db)) => {
                let p = s.path.clone();
                db.call(move |c| {
                    use rusqlite::OptionalExtension;
                    Ok(c.query_row("SELECT id FROM files WHERE path = ?1", [p], |r| r.get::<_, String>(0))
                        .optional()?)
                })
                .await
                .unwrap_or(None)
            }
            _ => None,
        }
    }
}

/// How a scanned file times its pictures (`scan::parse`).
fn scan_timing(container: Container, codec: &str, reorder: i64) -> scan::Timing {
    match container {
        Container::Avi if codec == "mpeg4" && reorder > 0 => scan::Timing::AviDelayed,
        Container::Avi => scan::Timing::Avi,
        _ => scan::Timing::Pts,
    }
}

/// The reason given for a damaged file (the apps show it after "can't play this title: ").
pub const DAMAGED: &str = "the file is damaged or incomplete (its keyframe index can't be read); re-download it";

/// How the keyframe index of a file is made, as `Media::load` makes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IndexKind {
    /// Matroska's Cues: a few reads (none that can be read: not served).
    Mkv,
    /// MP4's `moov` box: a few reads; a packet scan if it can't be read.
    Mp4,
    /// A packet scan of the whole file.
    Scan,
}

/// What a file needs for its keyframe index, from its probe: how it is made, the video stream
/// (ffprobe's index: the first that isn't cover art) and its timing. None: no video.
pub fn index_plan(probe: &Value, config: &Config) -> Option<(IndexKind, i64, scan::Timing)> {
    let streams = probe["streams"].as_array()?;
    let v = streams
        .iter()
        .find(|p| p["codec_type"] == "video" && p["disposition"]["attached_pic"].as_i64() != Some(1))?;
    let stream = v["index"].as_i64()?;
    let container = Container::of(probe["format"]["format_name"].as_str().unwrap_or(""));
    let timing = scan_timing(
        container,
        v["codec_name"].as_str().unwrap_or(""),
        v["has_b_frames"].as_i64().unwrap_or(0),
    );
    let kind = match container {
        Container::Matroska => IndexKind::Mkv,
        Container::Mp4 if config.enabled(Feature::Mp4Index) => IndexKind::Mp4,
        _ => IndexKind::Scan,
    };
    Some((kind, stream, timing))
}

/// Where a file's timeline is on its own clock. The keyframes are the file's own timestamps, which
/// start at its start time (`origin`; ffprobe's `start_time`, none below 0): players count that as
/// 0. ffprobe's duration is how long the file runs from there, except in Matroska, whose duration
/// is its last timestamp (it counts from the clock's 0). Before 0.31.8 the plan took the duration
/// as the file's end and its first segment from 0: with a large start time the boundaries stopped
/// short (a single segment when the start was past the duration) and the timeline began late.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Timeline {
    origin: f64,
    /// How long the title runs from `origin`.
    length: f64,
    /// ffprobe's duration (as plans took it before 0.31.8).
    probed: f64,
}

impl Timeline {
    fn of(container: Container, start_time: f64, probed: f64) -> Timeline {
        let origin = start_time.max(0.0);
        let length = if container == Container::Matroska && probed > origin {
            probed - origin
        } else {
            probed
        };
        Timeline { origin, length, probed }
    }

    /// The file's end on its own clock.
    fn end(&self) -> f64 {
        self.origin + self.length
    }
}

/// The timeline of a file whose start time is past `LATE_START`, from its probe alone, for what
/// is read from the file outside the segment plan: chapters (`library/api.rs`, `userdata`) and
/// thumbnails (`trickplay.rs`). Their times are on the file's own clock, and Matroska's duration
/// is its last timestamp; players count the start time as 0 (`Timeline`). Files starting at 0 or
/// a little later get None: their times are taken as they are, as before 0.31.11.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OwnClock {
    /// The start time: the timeline's 0 on the file's clock.
    pub origin: f64,
    /// How long the title runs from `origin`, if the probe says.
    pub length: Option<f64>,
}

impl OwnClock {
    pub fn of(probe: &Value) -> Option<OwnClock> {
        let num = |v: &Value| {
            v.as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| v.as_f64())
                .filter(|s| s.is_finite())
        };
        let start = num(&probe["format"]["start_time"])?;
        if start <= LATE_START {
            return None;
        }
        let container = Container::of(probe["format"]["format_name"].as_str().unwrap_or(""));
        // To the microsecond ffprobe gives (a difference of two of its times isn't exact).
        let length = num(&probe["format"]["duration"]).map(|d| (Timeline::of(container, start, d).length * 1e6).round() / 1e6);
        Some(OwnClock { origin: start, length })
    }

    /// What to take off a file's chapter times: `origin` when they are on the file's clock, as
    /// Matroska has them. A list with no chapter at or past the start time can't be on it (the
    /// file has nothing before its start): it was written from 0, and is taken as it is.
    pub fn chapter_offset(clock: Option<OwnClock>, starts: &[f64]) -> f64 {
        match clock {
            Some(c) if starts.iter().any(|s| *s >= c.origin - 0.001) => c.origin,
            _ => 0.0,
        }
    }
}

/// Boundaries at keyframes, about TARGET_SEGMENT apart from the timeline's start (`origin`) to its
/// `end`, all on the file's own clock; no tiny last segment. Segment 0 is cut from the file's
/// beginning (0), whatever its start time.
fn plan(keyframes: &[f64], origin: f64, end: f64) -> Vec<f64> {
    let mut b = vec![origin];
    for &k in keyframes {
        if k - b.last().unwrap() >= TARGET_SEGMENT && end - k >= 1.0 {
            b.push(k);
        }
    }
    b[0] = 0.0;
    b
}

/// How far before a boundary to ask for, in containers without an index (`Container::Other`:
/// MPEG-TS, MPEG-PS...). ffmpeg finds the place by the packets' decode times and starts decoding at
/// the next keyframe it reads, so a seek past the keyframe's packet misses it and the run begins a
/// GOP late. A keyframe's decode time is before its own time by the reordering delay, a fraction of
/// a second. A seek that lands a GOP early is slower, and still right: the pictures before the
/// boundary are dropped (`Transcoder`).
const BEFORE_KEYFRAME: f64 = 1.0;

/// A start time up to this can't move a seek on to another keyframe, short of keyframes this close.
const LATE_START: f64 = 0.1;

/// Whether a transcode run could begin past its boundary before `BEFORE_KEYFRAME` and
/// `Media::input_seek`: those files' cached segments can't be trusted. No video, no such run.
fn runs_could_begin_late(video: bool, container: Container, start_time: f64) -> bool {
    video && (container == Container::Other || start_time > LATE_START)
}

/// ffmpeg seeks (`-ss` before `-i`) to the keyframe at or before the time asked for, but for
/// streams with B-frames in containers that don't seek by presentation time (Matroska, AVI…)
/// it asks for 3/23 s earlier than that, landing a whole GOP early. So ask for a little more
/// than 3/23 s past the boundary, unless the next keyframe is closer than that (then half way
/// to it, and the seek lands early: slower, still right). Containers with a frame-numbered time
/// base (AVI) round the time asked for to the nearest frame, so exactly half way to a keyframe
/// one frame on rounds up and the seek lands on it, a frame late: stop a little short of half way.
fn seek_offset(sorted_keyframes: &[f64], boundary: f64) -> f64 {
    const PAST: f64 = 3.0 / 23.0 + 0.006;
    // More than the 0.5 ms `-ss` is rounded to when it is printed.
    const SHORT_OF_HALF: f64 = 0.002;
    let i = sorted_keyframes.partition_point(|k| *k <= boundary + 0.001);
    let next = sorted_keyframes.get(i).copied().unwrap_or(f64::MAX);
    PAST.min(((next - boundary) / 2.0 - SHORT_OF_HALF).max(0.0))
}

/// The file's bitrate, for the copy's `BANDWIDTH` and the ladder: the library's (the scan's
/// probe), else this load's probe, else its size over its duration. A title opened before the
/// scan has probed its file has no bitrate in the library yet, and was planned at the 1 Mb/s
/// floor: advertised far below its real rate, with no transcodes to pick at a quality limit.
fn source_bitrate(scanned: Option<i64>, probe: &Value, size: Option<u64>, duration: f64) -> i64 {
    let probed = || probe["format"]["bit_rate"].as_str().and_then(|b| b.parse::<i64>().ok());
    let size = size.or_else(|| probe["format"]["size"].as_str().and_then(|s| s.parse().ok()));
    let estimated = || size.filter(|_| duration > 0.0).map(|s| (s as f64 * 8.0 / duration) as i64);
    scanned
        .filter(|b| *b > 0)
        .or_else(|| probed().filter(|b| *b > 0))
        .or_else(|| estimated().filter(|b| *b > 0))
        .unwrap_or(0)
}

fn video_track(s: &item::MediaStream, p: &Value, bitrate: i64) -> VideoTrack {
    let stream = p["index"].as_i64().unwrap_or(s.index);
    let codec = p["codec_name"].as_str().unwrap_or("").to_string();
    let level = p["level"].as_i64().unwrap_or(0);
    let profile = p["profile"].as_str().unwrap_or("");
    let pix = p["pix_fmt"].as_str().unwrap_or("");
    let codecs_attr = match codec.as_str() {
        _ if !players_decode(&codec, profile, pix) => None,
        "h264" => {
            let idc = match profile {
                "Baseline" | "Constrained Baseline" => 0x42,
                "Main" => 0x4D,
                _ => 0x64,
            };
            Some(format!("avc1.{idc:02X}00{level:02X}"))
        }
        "hevc" => Some(if profile.contains("10") || pix.contains("10") {
            format!("hvc1.2.4.L{level}.B0")
        } else {
            format!("hvc1.1.6.L{level}.B0")
        }),
        _ => None,
    };
    let range = match s.video_range_type.as_deref().unwrap_or("SDR") {
        "SDR" => "SDR",
        r if r.contains("HLG") => "HLG",
        _ => "PQ",
    };
    let frame_rate = rational(p["avg_frame_rate"].as_str()).or_else(|| rational(p["r_frame_rate"].as_str()));
    let (w, h) = (p["width"].as_i64().unwrap_or(0), p["height"].as_i64().unwrap_or(0));
    let sar = p["sample_aspect_ratio"]
        .as_str()
        .and_then(|r| r.split_once(':'))
        .and_then(|(n, d)| Some((n.parse::<f64>().ok()?, d.parse::<f64>().ok()?)));
    let sar = match sar {
        Some((n, d)) if n > 0.0 && d > 0.0 => n / d,
        _ => 1.0,
    };
    VideoTrack {
        index: s.index,
        stream,
        codec,
        codecs_attr,
        width: w,
        height: h,
        frame_rate,
        range,
        bitrate: bitrate.max(1_000_000),
        dar: if h > 0 { w as f64 * sar / h as f64 } else { 16.0 / 9.0 },
        pix_fmt: pix.to_string(),
        profile: profile.to_string(),
        dovi_only: s.video_range_type.as_deref() == Some("DOVI"),
        dovi: Dovi::from_probe(p),
        reorder: p["has_b_frames"].as_i64().unwrap_or(0),
        copy_reason: None,
        short_nal_lengths: codec_is_h264_short_nal(p),
    }
}

/// H.264 or HEVC that the Apple and Android apps' players decode as it is, so it can be copied
/// (ffprobe's codec, profile and pixel format): what HLS allows on Apple's devices (H.264 up to
/// High, HEVC up to Main 10); Android requires no more of its decoders (H.264 Main, HEVC Main).
/// VideoToolbox decodes no H.264 above High at all, not even in software (the picture stays
/// black), and HEVC's range extensions only in software on a Mac without a media engine for them.
/// The Windows app decodes in software, but the copy is decided per file, not per player.
/// So these are transcoded:
///
/// - H.264 High 10 and 10-bit pictures (as before);
/// - H.264 4:2:2 and 4:4:4 (cameras, screen recorders), and High 4:4:4 Predictive even at 4:2:0,
///   which is what x264 makes of any lossless encode (`-qp 0`, a recorder's lossless setting);
/// - HEVC range extensions and screen content coding (`Rext`, `SCC`: 4:2:2 and 4:4:4 from
///   cameras, 12-bit), and any HEVC picture that isn't 4:2:0.
///
/// An unknown profile or pixel format leaves the stream copied, as before.
fn players_decode(codec: &str, profile: &str, pix: &str) -> bool {
    let chroma = pix.contains("422") || pix.contains("444") || pix.starts_with("gbr");
    match codec {
        "h264" => !chroma && !pix.contains("10") && !h264_beyond_high(profile),
        "hevc" => !chroma && !pix.contains("12") && !matches!(profile, "Rext" | "SCC"),
        _ => true,
    }
}

/// An H.264 profile above High (High 10, High 4:2:2, High 4:4:4 and their intra and CAVLC
/// forms): no hardware decoder the server uses takes it either, whatever the pixel format.
fn h264_beyond_high(profile: &str) -> bool {
    profile.starts_with("High 10") || profile.contains("4:2:2") || profile.contains("4:4:4")
}

/// An H.264 stream stored with NAL length fields shorter than 4 bytes (ffprobe's `nal_length_size`).
fn codec_is_h264_short_nal(p: &Value) -> bool {
    p["codec_name"] == "h264" && matches!(p["nal_length_size"].as_str(), Some("1" | "2" | "3"))
}

async fn probe(ffprobe: &str, source: &Source) -> Result<Value> {
    let mut cmd = tokio::process::Command::new(ffprobe);
    cmd.args(["-v", "error", "-show_streams", "-show_format", "-show_chapters", "-of", "json"])
        .args(source.input_args());
    run_probe(cmd).await
}

/// ffprobe of a library file (`nice`: at low CPU priority, for the scanner).
pub async fn probe_path(ffprobe: &str, path: &str, nice: bool) -> Result<Value> {
    let mut cmd = if nice {
        let mut c = tokio::process::Command::new("nice");
        c.args(["-n", "10", ffprobe]);
        c
    } else {
        tokio::process::Command::new(ffprobe)
    };
    cmd.args(["-v", "error", "-show_streams", "-show_format", "-show_chapters", "-of", "json"])
        .args(crate::storage::vfs::ffmpeg_input_args(path));
    run_probe(cmd).await
}

async fn run_probe(mut cmd: tokio::process::Command) -> Result<Value> {
    let out = tokio::time::timeout(std::time::Duration::from_secs(120), cmd.kill_on_drop(true).output())
        .await
        .context("ffprobe timed out")?
        .context("ffprobe")?;
    if !out.status.success() {
        anyhow::bail!(
            "ffprobe: {}",
            crate::admin::logbuf::scrub(String::from_utf8_lossy(&out.stderr).trim())
        );
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}

/// ISO 639-2 to the RFC 5646 tags HLS wants, for common languages.
fn bcp47(lang: &str) -> String {
    let map = [
        ("eng", "en"),
        ("rus", "ru"),
        ("ukr", "uk"),
        ("fre", "fr"),
        ("fra", "fr"),
        ("ger", "de"),
        ("deu", "de"),
        ("spa", "es"),
        ("ita", "it"),
        ("por", "pt"),
        ("jpn", "ja"),
        ("chi", "zh"),
        ("zho", "zh"),
        ("kor", "ko"),
        ("dut", "nl"),
        ("nld", "nl"),
        ("swe", "sv"),
        ("nor", "no"),
        ("dan", "da"),
        ("fin", "fi"),
        ("pol", "pl"),
        ("cze", "cs"),
        ("ces", "cs"),
        ("tur", "tr"),
        ("heb", "he"),
        ("ara", "ar"),
        ("hin", "hi"),
        ("gre", "el"),
        ("ell", "el"),
        ("hun", "hu"),
        ("rum", "ro"),
        ("ron", "ro"),
        ("bul", "bg"),
        ("srp", "sr"),
        ("hrv", "hr"),
    ];
    map.iter()
        .find(|(k, _)| *k == lang)
        .map_or_else(|| lang.to_string(), |(_, v)| v.to_string())
}

pub fn fnv(s: &str) -> u64 {
    s.bytes()
        .fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn short_nal_lengths() {
        let p = |codec: &str, n: Option<&str>| json!({"codec_name": codec, "nal_length_size": n});
        assert!(codec_is_h264_short_nal(&p("h264", Some("3"))));
        assert!(!codec_is_h264_short_nal(&p("h264", Some("4"))));
        // Annex B in the file (MPEG-TS, AVI): no length fields.
        assert!(!codec_is_h264_short_nal(&p("h264", None)));
        assert!(!codec_is_h264_short_nal(&p("hevc", Some("3"))));
    }

    fn jf(streams: Value) -> Vec<item::MediaStream> {
        serde_json::from_value(streams).unwrap()
    }

    /// The CODECS of a probed video stream when it is copied, else None.
    fn copied(codec: &str, profile: &str, pix: &str, level: i64) -> Option<String> {
        let p = json!({"index": 0, "codec_name": codec, "profile": profile, "pix_fmt": pix, "level": level,
                       "width": 1280, "height": 720});
        let ms: item::MediaSource = serde_json::from_value(json!({"Id": "m", "MediaStreams": [{"Index": 0, "Type": "Video"}]})).unwrap();
        let v = Media::video_of(&ms, &|_| Some(&p), Container::Mp4, 5_000_000).unwrap();
        assert_eq!(v.copyable(), v.codecs_attr.is_some());
        v.codecs_attr
    }

    #[test]
    fn what_every_player_decodes_is_copied_with_the_codecs_it_had() {
        // ffprobe's names for 4:2:0 at 8 bits (H.264, HEVC) and 10 (HEVC), as x264, x265,
        // phones and cameras make them: the same CODECS as before the check
        // for video Apple devices can't decode.
        let ordinary = [
            ("h264", "High", "yuv420p", 31, "avc1.64001F"),
            ("h264", "High", "yuvj420p", 40, "avc1.640028"),
            ("h264", "Main", "yuv420p", 30, "avc1.4D001E"),
            ("h264", "Constrained Baseline", "yuv420p", 30, "avc1.42001E"),
            ("h264", "", "", 41, "avc1.640029"),
            ("hevc", "Main", "yuv420p", 93, "hvc1.1.6.L93.B0"),
            ("hevc", "Main 10", "yuv420p10le", 150, "hvc1.2.4.L150.B0"),
            ("hevc", "Main 10", "", 153, "hvc1.2.4.L153.B0"),
            ("hevc", "", "", 120, "hvc1.1.6.L120.B0"),
        ];
        for (codec, profile, pix, level, want) in ordinary {
            assert_eq!(copied(codec, profile, pix, level).as_deref(), Some(want), "{codec} {profile} {pix}");
        }
    }

    #[test]
    fn what_apple_devices_cannot_decode_is_transcoded() {
        // Probes of x264/x265 encodes made the way screen recorders, lossless captures and
        // cameras make them. VideoToolbox decodes none of the H.264 ones (no picture at all), and
        // HEVC's range extensions only in software where it has them.
        let refused = [
            ("h264", "High 4:4:4 Predictive", "yuv444p"), // -pix_fmt yuv444p
            ("h264", "High 4:4:4 Predictive", "yuv420p"), // lossless (-qp 0) at 4:2:0
            ("h264", "High 4:4:4 Predictive", "gbrp"),    // libx264rgb
            ("h264", "High 4:4:4 Intra", "yuv444p10le"),
            ("h264", "CAVLC 4:4:4", "yuv444p"),
            ("h264", "High 4:2:2", "yuv422p"),
            ("h264", "High 4:2:2", "yuv422p10le"), // a camera's 4:2:2 10-bit
            ("h264", "High 4:2:2 Intra", "yuv422p10le"),
            ("h264", "High 10", "yuv420p10le"), // as before
            ("h264", "", "yuv444p"),
            ("hevc", "Rext", "yuv422p10le"), // a camera's 4:2:2 10-bit
            ("hevc", "Rext", "yuv444p"),
            ("hevc", "Rext", "yuv444p10le"),
            ("hevc", "Rext", "yuv420p12le"),
            ("hevc", "Rext", "yuv420p"),
            ("hevc", "SCC", "yuv444p"),
            ("hevc", "", "yuv422p10le"),
            ("hevc", "", "gbrp10le"),
        ];
        for (codec, profile, pix) in refused {
            assert_eq!(copied(codec, profile, pix, 31), None, "{codec} {profile} {pix}");
        }
    }

    fn ff(kinds: &[&str]) -> Vec<Value> {
        kinds
            .iter()
            .enumerate()
            .map(|(i, k)| json!({"index": i, "codec_type": k, "disposition": {"attached_pic": 0}}))
            .collect()
    }

    #[test]
    fn external_subtitles_come_first() {
        // The Flash S08E18: the library lists the sidecar .srt as 0 and shifts the file's streams.
        let j = jf(json!([
            {"Index": 0, "Type": "Subtitle", "IsExternal": true},
            {"Index": 1, "Type": "Video"},
            {"Index": 2, "Type": "Audio"},
            {"Index": 3, "Type": "Subtitle"},
        ]));
        let m = map_streams(&j, &ff(&["video", "audio", "subtitle"]));
        assert_eq!(m, HashMap::from([(1, 0), (2, 1), (3, 2)]));
    }

    #[test]
    fn plain_and_external_last() {
        let p = ff(&["video", "audio", "audio", "subtitle", "attachment"]);
        let j = jf(
            json!([{"Index": 0, "Type": "Video"}, {"Index": 1, "Type": "Audio"}, {"Index": 2, "Type": "Audio"},
                          {"Index": 3, "Type": "Subtitle"}, {"Index": 4, "Type": "Subtitle", "IsExternal": true}]),
        );
        assert_eq!(map_streams(&j, &p), HashMap::from([(0, 0), (1, 1), (2, 2), (3, 3)]));
    }

    #[test]
    fn falls_back_to_order_by_kind() {
        // An attachment in the middle of the file's streams: no single offset fits.
        let p = ff(&["video", "attachment", "audio", "subtitle"]);
        let j = jf(json!([{"Index": 0, "Type": "Video"}, {"Index": 1, "Type": "Audio"}, {"Index": 2, "Type": "Subtitle"}]));
        assert_eq!(map_streams(&j, &p), HashMap::from([(0, 0), (1, 2), (2, 3)]));
        // Cover art is not the video.
        let mut p = ff(&["video", "video", "audio"]);
        p[0]["disposition"]["attached_pic"] = json!(1);
        let j = jf(json!([{"Index": 0, "Type": "EmbeddedImage"}, {"Index": 1, "Type": "Video"}, {"Index": 2, "Type": "Audio"}]));
        assert_eq!(map_streams(&j, &p), HashMap::from([(0, 0), (1, 1), (2, 2)]));
    }

    #[test]
    fn seek_offsets() {
        let kf = [0.0, 2.19, 11.783, 11.85, 26.506];
        assert!((seek_offset(&kf, 2.19) - (3.0 / 23.0 + 0.006)).abs() < 1e-9);
        // A keyframe 67 ms later: just short of half way to it.
        assert!((seek_offset(&kf, 11.783) - 0.0315).abs() < 1e-9);
        assert!((seek_offset(&kf, 26.506) - (3.0 / 23.0 + 0.006)).abs() < 1e-9);
    }

    /// AVI seeks by frame number, the time asked for rounded to the nearest frame: with a
    /// keyframe one frame after the boundary, the `-ss` printed must still round to the boundary.
    #[test]
    fn a_seek_between_adjacent_keyframes_stays_on_the_first() {
        let frame_of = |t: f64, fps: f64| (t * fps).round() as i64;
        for fps in [24000.0 / 1001.0, 25.0, 30000.0 / 1001.0, 30.0, 50.0, 60.0, 120.0] {
            for n in [24, 144, 288, 289, 300, 2408, 2409, 86_400] {
                // Printed as the index stores it, and as `-ss` is printed.
                let at = |frame: i64| format!("{:.6}", frame as f64 / fps).parse::<f64>().unwrap();
                let kf = [at(0), at(n), at(n + 1), at(n + 500)];
                let ss = format!("{:.3}", kf[1] + seek_offset(&kf, kf[1])).parse::<f64>().unwrap();
                assert_eq!(frame_of(ss, fps), n, "{fps} fps, keyframes at frames {n} and {}: -ss {ss}", n + 1);
            }
        }
    }

    #[test]
    fn a_seek_is_asked_for_on_the_files_own_clock() {
        // ffmpeg adds the file's start time to `-ss`: boundaries are the file's own timestamps.
        let mut m =
            crate::media::transcode::tests::media(crate::media::transcode::tests::video("mpeg2video", 1920, 1080, 4_000_000, "SDR"));
        m.start_time = 1.44;
        assert!((m.input_seek(7.44) - 6.0).abs() < 1e-9);
        // Nothing before the file's start to ask for.
        assert_eq!(m.input_seek(1.0), 0.0);
    }

    #[test]
    fn a_seek_in_a_file_without_an_index_is_asked_for_before_its_keyframe() {
        let kf = vec![1.44, 7.44, 13.44, 19.44];
        let (boundaries, offsets, ..) = Media::segments_of(
            Ok(Some(kf.clone())),
            false,
            Timeline::of(Container::Other, 1.44, 60.0),
            Container::Other,
            &mut Vec::new(),
        );
        assert!(boundaries.len() > 1);
        // MPEG-TS seeks by decode time and decoding starts at the next keyframe read: past the
        // keyframe's packet the run begins a GOP late.
        assert!(offsets.iter().all(|o| *o == -BEFORE_KEYFRAME), "{offsets:?}");
        // Indexed containers still go past the boundary.
        let (_, offsets, ..) = Media::segments_of(
            Ok(Some(kf)),
            false,
            Timeline::of(Container::Matroska, 1.44, 60.0),
            Container::Matroska,
            &mut Vec::new(),
        );
        assert!(offsets[1] > 0.0, "{offsets:?}");
    }

    #[test]
    fn files_whose_runs_could_begin_late_get_new_segments() {
        let ms = item::MediaSource {
            id: "m".into(),
            path: Some("/m/a".into()),
            run_time_ticks: Some(1),
            bitrate: Some(1),
            media_streams: vec![],
            stamp: None,
        };
        assert_ne!(
            Media::version_of(&ms, Container::Matroska, false, false),
            Media::version_of(&ms, Container::Matroska, true, false)
        );
        assert!(!runs_could_begin_late(true, Container::Matroska, 0.08));
        assert!(runs_could_begin_late(true, Container::Matroska, 1.44));
        assert!(runs_could_begin_late(true, Container::Other, 0.0));
        // Nothing transcoded, nothing cached wrong.
        assert!(!runs_could_begin_late(false, Container::Other, 1.44));
    }

    /// Keyframes every `gop` s from `first` to `end` (exclusive).
    fn keyframes(first: f64, gop: f64, end: f64) -> Vec<f64> {
        let n = ((end - first) / gop).ceil() as usize;
        (0..n).map(|i| first + i as f64 * gop).filter(|k| *k < end).collect()
    }

    /// The media of a file with `keyframes` on its timeline (`Media::load`'s part of it).
    fn timed_media(container: Container, start_time: f64, probed: f64, kf: Vec<f64>) -> (Media, bool) {
        let timeline = Timeline::of(container, start_time, probed);
        let (boundaries, seek_offsets, _, _, replanned) = Media::segments_of(Ok(Some(kf)), false, timeline, container, &mut Vec::new());
        let mut m = crate::media::transcode::tests::media(crate::media::transcode::tests::video("h264", 1920, 1080, 4_000_000, "SDR"));
        (m.start_time, m.origin, m.duration) = (start_time, timeline.origin, timeline.length);
        (m.boundaries, m.seek_offsets) = (boundaries, seek_offsets);
        (m, replanned)
    }

    /// The segments' lengths on the title's timeline (what the playlist says).
    fn spans(m: &Media) -> Vec<f64> {
        (0..m.boundaries.len())
            .map(|n| m.segment_span(n).map(|(s, e)| e - s).unwrap())
            .collect()
    }

    #[test]
    fn a_files_own_clock_from_its_probe() {
        let probe =
            |name: &str, start: &str, duration: &str| json!({"format": {"format_name": name, "start_time": start, "duration": duration}});
        // A TV recording remuxed into Matroska keeping its broadcast clock: Matroska's duration
        // is its last timestamp.
        let mkv = OwnClock::of(&probe("matroska,webm", "1002.500000", "1182.750000")).unwrap();
        assert_eq!((mkv.origin, mkv.length), (1002.5, Some(180.25)));
        // The recording itself: MPEG-TS's duration is its length.
        let ts = OwnClock::of(&probe("mpegts", "1002.500000", "180.250000")).unwrap();
        assert_eq!((ts.origin, ts.length), (1002.5, Some(180.25)));
        // Files starting at 0, or a frame later: their times are taken as they are.
        assert_eq!(OwnClock::of(&probe("matroska,webm", "0.000000", "180.0")), None);
        assert_eq!(OwnClock::of(&probe("matroska,webm", "0.080000", "180.0")), None);
        assert_eq!(OwnClock::of(&probe("mpegts", "-0.020000", "180.0")), None);
        assert_eq!(OwnClock::of(&json!({"format": {}})), None);
        // Chapters on the file's clock lose the start time; a list written from 0 (none at or
        // past the start) is taken as it is.
        assert_eq!(OwnClock::chapter_offset(Some(mkv), &[1002.5, 1032.5]), 1002.5);
        assert_eq!(OwnClock::chapter_offset(Some(mkv), &[0.0, 30.0, 150.0]), 0.0);
        assert_eq!(OwnClock::chapter_offset(None, &[1002.5]), 0.0);
    }

    #[test]
    fn a_file_with_a_large_start_time_is_planned_on_its_own_clock() {
        // An MPEG-TS recording whose clock starts at 1002.39, 130 s long (ffprobe's duration is
        // the length): before 0.31.8 the keyframes were held against 130 as the file's end, and
        // the whole title was one segment.
        let (m, replanned) = timed_media(Container::Other, 1002.39, 130.0, keyframes(1002.39, 6.04, 1132.39));
        assert!(replanned);
        let s = spans(&m);
        assert_eq!(s.len(), 22, "{s:?}");
        assert!((s[0] - 6.04).abs() < 1e-6, "the timeline starts at the file's start: {s:?}");
        assert!((s.iter().sum::<f64>() - 130.0).abs() < 1e-6, "{s:?}");
        assert_eq!(m.boundaries[0], 0.0, "segment 0 is cut from the file's beginning");
        assert!((m.boundaries[1] - 1008.43).abs() < 1e-6);
        // A position on the timeline is on the file's clock past the start time.
        assert!((m.file_time(10.0) - 1012.39).abs() < 1e-6);

        // Matroska's duration is its last timestamp: 1131 for a file starting at 1001. Before,
        // the first segment ran from 0 to 1001 and the timeline was 1131 s long.
        let (m, replanned) = timed_media(Container::Matroska, 1001.0, 1131.0, keyframes(1001.0, 6.04, 1131.0));
        assert!(replanned);
        let s = spans(&m);
        assert!((m.duration - 130.0).abs() < 1e-6);
        assert!((s[0] - 6.04).abs() < 1e-6 && (s.iter().sum::<f64>() - 130.0).abs() < 1e-6, "{s:?}");
        assert!((m.boundaries[1] - 1007.04).abs() < 1e-6, "{:?}", m.boundaries);
    }

    #[test]
    fn a_file_starting_at_zero_keeps_its_plan() {
        // Same boundaries, playlist and version as before 0.31.8: its cached segments stay.
        for container in [Container::Matroska, Container::Mp4, Container::Other] {
            let kf = keyframes(0.0, 5.005, 130.0);
            let (m, replanned) = timed_media(container, 0.0, 130.0, kf.clone());
            assert!(!replanned);
            assert_eq!(m.boundaries, plan(&kf, 0.0, 130.0));
            assert!((spans(&m).iter().sum::<f64>() - 130.0).abs() < 1e-6);
        }
        // A small start time whose boundaries come out the same: no new segments either, only
        // the first segment's length on the timeline is shorter by it.
        let (m, replanned) = timed_media(Container::Other, 1.4, 130.0, keyframes(1.4, 6.006, 131.4));
        assert!(!replanned, "{:?}", m.boundaries);
        assert!((spans(&m)[0] - 6.006).abs() < 1e-6);
    }

    #[test]
    fn a_files_bitrate_comes_from_the_library_else_its_probe_else_its_size() {
        let probe = serde_json::json!({"format": {"bit_rate": "12140071", "size": "227658207"}});
        // The scan's figure first: what every title probed by the scan was planned with.
        assert_eq!(source_bitrate(Some(2_705_478), &probe, None, 150.0), 2_705_478);
        // Opened before the scan has probed it: this load's probe, not the 1 Mb/s floor.
        assert_eq!(source_bitrate(None, &probe, None, 150.0), 12_140_071);
        assert_eq!(source_bitrate(Some(0), &probe, None, 150.0), 12_140_071);
        // No bitrate in the probe: the size over the duration (the library's size first).
        let no_rate = serde_json::json!({"format": {"size": "227658207"}});
        assert_eq!(source_bitrate(None, &no_rate, None, 150.0), 12_141_771);
        assert_eq!(source_bitrate(None, &no_rate, Some(50_727_715), 150.0), 2_705_478);
        // Nothing to go on: 0, which the video track floors at 1 Mb/s as before.
        assert_eq!(source_bitrate(None, &serde_json::json!({}), None, 0.0), 0);
        assert_eq!(source_bitrate(None, &no_rate, None, 0.0), 0);
    }

    #[test]
    fn files_whose_plan_changes_get_new_segments() {
        let ms = item::MediaSource {
            id: "m".into(),
            path: Some("/m/a".into()),
            run_time_ticks: Some(1),
            bitrate: Some(1),
            media_streams: vec![],
            stamp: None,
        };
        assert_ne!(
            Media::version_of(&ms, Container::Other, true, false),
            Media::version_of(&ms, Container::Other, true, true)
        );
    }

    #[test]
    fn a_seek_never_goes_before_its_boundary() {
        // Keyframes closer than the margin: exactly the boundary.
        assert_eq!(seek_offset(&[0.0, 5.0, 5.003], 5.0), 0.0);
    }

    #[test]
    fn dolby_vision_records() {
        let p = |profile: u64, compat: u64, bl: u64, el: u64| {
            json!({"codec_name": "hevc", "side_data_list": [{"side_data_type": "DOVI configuration record", "dv_version_major": 1,
                "dv_version_minor": 0, "dv_profile": profile, "dv_level": 6, "rpu_present_flag": 1, "el_present_flag": el,
                "bl_present_flag": bl, "dv_bl_signal_compatibility_id": compat, "dv_md_compression": "none"}]})
        };
        let d81 = Dovi::from_probe(&p(8, 1, 1, 0)).unwrap();
        assert_eq!(
            d81,
            Dovi {
                profile: 8,
                level: 6,
                rpu: true,
                el: false,
                bl: true,
                compat: 1
            }
        );
        assert_eq!(d81.signal(), Some(DoviSignal::Supplemental("dvh1.08.06/db1p".into())));
        assert_eq!(
            Dovi::from_probe(&p(8, 4, 1, 0)).unwrap().signal(),
            Some(DoviSignal::Supplemental("dvh1.08.06/db4h".into()))
        );
        assert_eq!(
            Dovi::from_probe(&p(5, 0, 1, 0)).unwrap().signal(),
            Some(DoviSignal::Codec("dvh1.05.06".into()))
        );
        // Dual layer (profile 7) and an SDR base (8.2): left to the base layer.
        assert_eq!(Dovi::from_probe(&p(7, 6, 1, 1)).unwrap().signal(), None);
        assert_eq!(Dovi::from_probe(&p(8, 2, 1, 0)).unwrap().signal(), None);
        assert_eq!(Dovi::from_probe(&json!({"codec_name": "hevc"})), None);
        assert_eq!(
            Dovi::from_probe(&json!({"side_data_list": [{"side_data_type": "Mastering display metadata"}]})),
            None
        );

        // The record: version 1.0, profile 8 level 6 rpu+bl (0x10 0x35), compatibility 1.
        let b = d81.config_box();
        assert_eq!(b.len(), 32);
        assert_eq!(&b[..8], &[0, 0, 0, 32, b'd', b'v', b'v', b'C']);
        assert_eq!(&b[8..13], &[1, 0, 0x10, 0x35, 0x10]);
        assert!(b[13..].iter().all(|x| *x == 0));
        let p5 = Dovi::from_probe(&p(5, 0, 1, 0)).unwrap().config_box();
        assert_eq!((&p5[4..8], &p5[8..13]), (&b"dvcC"[..], &[1, 0, 0x0A, 0x35, 0x00][..]));
    }

    #[test]
    fn containers() {
        assert_eq!(Container::of("matroska,webm"), Container::Matroska);
        assert_eq!(Container::of("mov,mp4,m4a,3gp,3g2,mj2"), Container::Mp4);
        assert_eq!(Container::of("avi"), Container::Avi);
        assert_eq!(Container::of("mpeg"), Container::Other);
    }
}
