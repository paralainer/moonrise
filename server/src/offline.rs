//! Segmented downloads ("offline sets"): the same fMP4 HLS segments the server streams, fetched
//! one by one by the app and kept on the device, where a loopback HLS server plays them. A
//! download can be watched while it is still coming in, and segments it doesn't have yet can be
//! fetched out of order (where the viewer seeks).
//!
//! The URLs are stable (no playback session): a set is the title, its file version and one video
//! variant of its ladder; the audio tracks and subtitles are the title's own.
//!
//! - `GET /v1/items/{id}/offline?quality=&hdr=&audio=` (token): what to download: the variant
//!   (the best within `quality` bits/s, the original without), the audio tracks (those asked for,
//!   else one per language, the default first, at most 4), the text subtitles, the master and
//!   media playlists (relative URIs), the size expected, and `base`, where the files are.
//! - `GET {base}/video/{init.mp4|n.m4s}`, `GET {base}/audio/{index}/{init.mp4|n.m4s}`,
//!   `GET {base}/subs/{index}/cues.vtt` (token; `?v=` the file version from the description:
//!   409 when the file changed since). `base` is `/v1/items/{id}/offline/{variant}`.
//!
//! Transcoded variants are made by the same rolling GPU runs as streams. Their owner is the
//! download (`dl:<user>:<item>:<variant>`): a download's runs never stop a stream's (only other
//! downloads'), and a stream may stop a download's run when the slots are full; the app asks
//! again later (503).

use std::fmt::Write as _;
use std::{collections::HashMap, sync::Arc, time::Duration};

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

use crate::{
    admin::config::Feature,
    api::{media_playlist, mp4, segment_number, serve_track_file, ApiError, AppState},
    downloads::Downloads,
    media::segment::Track,
    media::transcode::{self, Variant},
    media::Media,
};

type Q = Query<HashMap<String, String>>;

/// How long a subtitle request waits for its track to be extracted (a big file, read the
/// first time).
const SUBS_WAIT: Duration = Duration::from_secs(600);

#[derive(Serialize)]
pub struct OfflineAudio {
    pub index: i64,
    pub name: String,
    pub language: Option<String>,
    pub channels: i64,
    pub codecs: String,
    pub default: bool,
    pub converted: bool,
}

#[derive(Serialize)]
pub struct OfflineSubtitle {
    pub index: i64,
    pub name: String,
    pub language: Option<String>,
    pub forced: bool,
}

#[derive(Serialize)]
pub struct OfflineSet {
    pub item_id: String,
    /// The file's version: segments are asked for with it (`?v=`).
    pub version: String,
    pub variant: Variant,
    pub duration: f64,
    /// Segments per track (video and every audio track have the same boundaries).
    pub segments: usize,
    pub audio: Vec<OfflineAudio>,
    pub subtitles: Vec<OfflineSubtitle>,
    /// The master playlist, with URIs relative to `base`.
    pub master: String,
    /// The media playlist of every track (video and audio): `init.mp4` and `n.m4s`.
    pub playlist: String,
    /// The playlist of every subtitle rendition: `cues.vtt`.
    pub subs_playlist: String,
    /// Estimated bytes of the whole set.
    pub expected: u64,
    /// Where the files are.
    pub base: String,
}

fn ready(st: &AppState) -> Result<(), ApiError> {
    if st.config.enabled(Feature::Downloads) {
        Ok(())
    } else {
        Err(ApiError(StatusCode::SERVICE_UNAVAILABLE, "downloads are switched off".into()))
    }
}

/// The title's variants for downloads: both ladders (with and without HDR), no burn-in.
fn ladders(st: &AppState, media: &Media) -> Vec<Variant> {
    let Some(v) = media.video.as_ref() else { return Vec::new() };
    let tx = st.config.enabled(Feature::Transcode);
    let mut out = transcode::ladder(v, true, None, tx);
    for x in transcode::ladder(v, false, None, tx) {
        if !out.iter().any(|y| y.id == x.id) {
            out.push(x);
        }
    }
    out
}

fn quote(s: &str) -> String {
    s.replace('"', "'")
}

/// The master playlist of a set, with relative URIs.
pub fn master(media: &Media, variant: &Variant, audio: &[i64], subs: &[i64]) -> String {
    let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-INDEPENDENT-SEGMENTS\n");
    let tracks: Vec<_> = audio.iter().filter_map(|i| media.audio_track(*i)).collect();
    for (k, a) in tracks.iter().enumerate() {
        let _ = writeln!(
            out,
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"{}\",{}DEFAULT={},AUTOSELECT=YES,CHANNELS=\"{}\",URI=\"audio/{}/index.m3u8\"",
            quote(&a.name),
            a.language.as_ref().map(|l| format!("LANGUAGE=\"{l}\",")).unwrap_or_default(),
            if k == 0 { "YES" } else { "NO" },
            a.channels,
            a.index
        );
    }
    let text: Vec<_> = media.subtitles.iter().filter(|s| s.text && subs.contains(&s.index)).collect();
    for s in &text {
        let _ = writeln!(
            out,
            "#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"{}\",{}DEFAULT=NO,AUTOSELECT={},FORCED={},URI=\"subs/{}/index.m3u8\"",
            quote(&s.name),
            s.language.as_ref().map(|l| format!("LANGUAGE=\"{l}\",")).unwrap_or_default(),
            if s.forced { "YES" } else { "NO" },
            if s.forced { "YES" } else { "NO" },
            s.index
        );
    }
    let mut codecs = vec![variant.codecs_attr.as_str()];
    for a in &tracks {
        if !codecs.contains(&a.codecs_attr.as_str()) {
            codecs.push(&a.codecs_attr);
        }
    }
    let frame_rate = media.video.as_ref().and_then(|v| v.frame_rate);
    // A title with no audio track: video alone, no audio group.
    let audio_bw = if tracks.is_empty() { 0 } else { 640_000 };
    let attrs = format!(
        "BANDWIDTH={},AVERAGE-BANDWIDTH={},CODECS=\"{}\",RESOLUTION={}x{},{}VIDEO-RANGE={},",
        variant.peak + audio_bw,
        variant.bitrate + audio_bw,
        codecs.join(","),
        variant.width,
        variant.height,
        frame_rate.map(|f| format!("FRAME-RATE={f:.3},")).unwrap_or_default(),
        variant.range,
    );
    let _ = writeln!(
        out,
        "#EXT-X-STREAM-INF:{}\nvideo/index.m3u8",
        crate::api::with_groups(attrs, !tracks.is_empty(), !text.is_empty())
    );
    out
}

/// The subtitle rendition's playlist: one WebVTT file for the whole title.
pub fn subs_playlist(duration: f64) -> String {
    format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:{duration:.3},\ncues.vtt\n#EXT-X-ENDLIST\n",
        duration.ceil() as i64
    )
}

pub async fn describe(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Json<OfflineSet>, ApiError> {
    ready(&st)?;
    st.authenticate(&headers, &q).await?;
    let media = st.media(&crate::api::media_key(&id, &q)).await?;
    let bad = |m: String| ApiError(StatusCode::UNPROCESSABLE_ENTITY, m);
    if !media.supported {
        return Err(bad(format!("can't be served: {}", media.reasons.join("; "))));
    }
    if media.pending || media.boundaries.is_empty() {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "keyframe index pending: ask again later".into(),
        ));
    }
    let v = media.video.as_ref().ok_or_else(|| bad("no video".into()))?;
    let hdr = q.get("hdr").is_none_or(|h| h != "0");
    let quality = q.get("quality").and_then(|s| s.parse::<i64>().ok()).filter(|b| *b > 0);
    let transcode_on = st.config.enabled(Feature::Transcode);
    let ladder = transcode::ladder(v, hdr, None, transcode_on);
    if ladder.is_empty() {
        return Err(bad(v.copy_reason.clone().unwrap_or_else(|| "not transcoded".into())));
    }
    let variant = ladder[transcode::select(&ladder, quality)].clone();
    if let Some(q) = quality.filter(|q| variant.bitrate > *q && !transcode_on) {
        return Err(bad(format!("quality {q} needs transcoding (switched off)")));
    }
    let asked: Option<Vec<i64>> = q.get("audio").map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect());
    let audio = Downloads::pick_audio(&media, asked.as_deref());
    // A title with no audio track is downloaded as video alone.
    if audio.is_empty() && !media.audio.is_empty() {
        return Err(bad("no audio track to include".into()));
    }
    let audio_ids: Vec<i64> = audio.iter().map(|a| a.index).collect();
    let subs: Vec<_> = media.subtitles.iter().filter(|s| s.text).collect();
    let sub_ids: Vec<i64> = subs.iter().map(|s| s.index).collect();
    let audio_bits: i64 = audio
        .iter()
        .map(|a| a.conversion.as_ref().map_or(256_000, |c| c.bitrate as i64))
        .sum();
    let video_bits = if variant.transcode { variant.bitrate } else { v.bitrate };
    let expected = ((video_bits + audio_bits) as f64 * media.duration / 8.0) as u64;
    // Its subtitles, so they are ready when the app asks.
    st.subs.prefetch(&media);
    let set = OfflineSet {
        item_id: media.item_id.clone(),
        version: media.version.clone(),
        master: master(&media, &variant, &audio_ids, &sub_ids),
        playlist: media_playlist(&media, true),
        subs_playlist: subs_playlist(media.duration),
        duration: media.duration,
        segments: media.boundaries.len(),
        audio: audio
            .iter()
            .map(|a| OfflineAudio {
                index: a.index,
                name: a.name.clone(),
                language: a.language.clone(),
                channels: a.channels,
                codecs: a.codecs_attr.clone(),
                default: a.default,
                converted: a.converted,
            })
            .collect(),
        subtitles: subs
            .iter()
            .map(|s| OfflineSubtitle {
                index: s.index,
                name: s.name.clone(),
                language: s.language.clone(),
                forced: s.forced,
            })
            .collect(),
        expected,
        base: format!("/v1/items/{}/offline/{}", media.item_id, variant.id),
        variant,
    };
    tracing::info!(item = %id, variant = set.variant.id, segments = set.segments, audio = ?audio_ids, subs = sub_ids.len(), "offline set");
    Ok(Json(set))
}

/// The set's media and variant, checked against the version the client has.
async fn open(
    st: &Arc<AppState>,
    headers: &HeaderMap,
    q: &HashMap<String, String>,
    id: &str,
    variant: &str,
) -> Result<(Arc<Media>, Variant, String), ApiError> {
    ready(st)?;
    let who = st.authenticate(headers, q).await?;
    let media = st.media(&crate::api::media_key(id, q)).await?;
    if let Some(v) = q.get("v") {
        if *v != media.version {
            return Err(ApiError(StatusCode::CONFLICT, "the file changed on the server".into()));
        }
    }
    let var = ladders(st, &media)
        .into_iter()
        .find(|x| x.id == variant)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such variant".into()))?;
    let owner = format!(
        "{}{}:{}:{}",
        transcode::DOWNLOAD_OWNER,
        who.user_id().unwrap_or_else(|| "admin".into()),
        media.item_id,
        var.id
    );
    Ok((media, var, owner))
}

fn busy(e: anyhow::Error) -> ApiError {
    // No GPU slot for a download right now (streams come first), or the run was stopped for one.
    ApiError(StatusCode::SERVICE_UNAVAILABLE, format!("{e:#}"))
}

pub async fn video_file(
    State(st): State<Arc<AppState>>,
    Path((id, variant, file)): Path<(String, String, String)>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Response, ApiError> {
    let (media, var, owner) = open(&st, &headers, &q, &id, &variant).await?;
    if var.is_copy() {
        return serve_track_file(&st, media, Track::Video, &file).await;
    }
    if file == "init.mp4" {
        return Ok(mp4(st.transcoder.init(&media, &var, 0, &owner).await.map_err(busy)?));
    }
    let n = segment_number(&media, &file)?;
    Ok(mp4(st.transcoder.segment(&media, &var, n, &owner).await.map_err(busy)?))
}

pub async fn audio_file(
    State(st): State<Arc<AppState>>,
    Path((id, variant, index, file)): Path<(String, String, i64, String)>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Response, ApiError> {
    let (media, _, owner) = open(&st, &headers, &q, &id, &variant).await?;
    let a = media
        .audio_track(index)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such audio track".into()))?
        .clone();
    if let Some(conv) = &a.conversion {
        if file == "init.mp4" {
            return Ok(mp4(st.converter.init(conv).await.map_err(ApiError::internal)?));
        }
        let n = segment_number(&media, &file)?;
        return Ok(mp4(st.converter.segment(&media, &a, n, &owner).await.map_err(busy)?));
    }
    serve_track_file(&st, media, Track::Audio(a.stream), &file).await
}

pub async fn subs_file(
    State(st): State<Arc<AppState>>,
    Path((id, variant, index)): Path<(String, String, i64)>,
    headers: HeaderMap,
    Query(q): Q,
) -> Result<Response, ApiError> {
    let (media, _, _) = open(&st, &headers, &q, &id, &variant).await?;
    if !media.subtitles.iter().any(|s| s.index == index && s.text) {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such subtitle track".into()));
    }
    // A download keeps what it gets: a downloaded track's match first, started if still to do.
    let made = async {
        st.subs.settled_sync(&media, index).await;
        st.subs.vtt(&media, index).await
    };
    let body = match tokio::time::timeout(SUBS_WAIT, made).await {
        Ok(r) => r.map_err(ApiError::internal)?,
        Err(_) => return Err(ApiError(StatusCode::SERVICE_UNAVAILABLE, "subtitle track not extracted yet".into())),
    };
    Ok((
        [
            (header::CONTENT_TYPE, "text/vtt; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subs_playlist_is_one_file() {
        let p = subs_playlist(100.2);
        assert!(p.contains("#EXT-X-TARGETDURATION:101\n"));
        assert!(p.contains("#EXTINF:100.200,\ncues.vtt\n#EXT-X-ENDLIST\n"));
    }

    #[test]
    fn a_title_with_no_audio_track_is_kept_as_video_alone() {
        // No audio rendition and no audio group in the downloaded set's master.
        use crate::media::transcode::{ladder, tests};
        let v = tests::video("h264", 1920, 1080, 4_000_000, "SDR");
        let m = tests::media(v.clone());
        let variant = ladder(&v, true, None, false).remove(0);
        let out = master(&m, &variant, &[], &[]);
        assert!(!out.contains("AUDIO"), "{out}");
        assert!(out.ends_with(",VIDEO-RANGE=SDR\nvideo/index.m3u8\n"), "{out}");
        assert!(out.contains(&format!("BANDWIDTH={},", variant.peak)), "{out}");
    }
}
