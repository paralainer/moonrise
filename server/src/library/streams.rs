//! A file's tracks, numbered and labelled for the app, made from our own ffprobe output and
//! the sidecar files next to it, so playback needs nothing but the file and the
//! labels (which the app matches HLS renditions by) are the same.
//!
//! The numbering rules (as media servers do it): sidecar files are numbered first (sorted by name, ignoring case),
//! then the file's streams keep ffprobe's index shifted by that count (attachments are left
//! out, cover art is `EmbeddedImage`). `DisplayTitle` is built from attributes (language,
//! codec, layout, flags); a stream with a title gets the title plus the attributes it doesn't
//! already contain, joined with " - ".

use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::media::item;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Stream {
    pub index: i64,
    /// Video, Audio, Subtitle, EmbeddedImage, Data (the usual media-server names).
    pub kind: String,
    pub codec: String,
    pub language: Option<String>,
    pub title: Option<String>,
    pub display_title: String,
    pub is_default: bool,
    pub is_forced: bool,
    pub is_hearing_impaired: bool,
    pub is_external: bool,
    pub is_text: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_layout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<i64>,
    /// SDR, HDR10, HLG, DOVI, DOVIWithHDR10… (video)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_range_type: Option<String>,
}

impl Stream {
    pub fn to_item_stream(&self) -> item::MediaStream {
        item::MediaStream {
            index: self.index,
            kind: self.kind.clone(),
            codec: Some(self.codec.clone()),
            language: self.language.clone(),
            title: self.title.clone(),
            display_title: Some(self.display_title.clone()),
            channels: self.channels,
            is_default: self.is_default,
            is_forced: self.is_forced,
            is_external: self.is_external,
            is_text_subtitle_stream: self.is_text,
            video_range_type: self.video_range_type.clone(),
            path: self.path.clone(),
        }
    }
}

/// (ISO 639-2/T, ISO 639-1, the English name). B codes are mapped to T first.
const LANGUAGES: &[(&str, &str, &str)] = &[
    ("eng", "en", "English"),
    ("rus", "ru", "Russian"),
    ("ukr", "uk", "Ukrainian"),
    ("spa", "es", "Spanish; Castilian"),
    ("fra", "fr", "French"),
    ("deu", "de", "German"),
    ("ita", "it", "Italian"),
    ("por", "pt", "Portuguese"),
    ("swe", "sv", "Swedish"),
    ("zho", "zh", "Chinese"),
    ("fin", "fi", "Finnish"),
    ("dan", "da", "Danish"),
    ("nld", "nl", "Dutch; Flemish"),
    ("nob", "nb", "Norwegian (Bokmal)"),
    ("nor", "no", "Norwegian"),
    ("nno", "nn", "Norwegian Nynorsk"),
    ("pol", "pl", "Polish"),
    ("ell", "el", "Greek"),
    ("ron", "ro", "Romanian; Moldavian; Moldovan"),
    ("jpn", "ja", "Japanese"),
    ("hun", "hu", "Hungarian"),
    ("ces", "cs", "Czech"),
    ("kor", "ko", "Korean"),
    ("tur", "tr", "Turkish"),
    ("slk", "sk", "Slovak"),
    ("heb", "he", "Hebrew"),
    ("tha", "th", "Thai"),
    ("ind", "id", "Indonesian"),
    ("ara", "ar", "Arabic"),
    ("bul", "bg", "Bulgarian"),
    ("msa", "ms", "Malay"),
    ("slv", "sl", "Slovenian"),
    ("lit", "lt", "Lithuanian"),
    ("est", "et", "Estonian"),
    ("lav", "lv", "Latvian"),
    ("hin", "hi", "Hindi"),
    ("vie", "vi", "Vietnamese"),
    ("tam", "ta", "Tamil"),
    ("tel", "te", "Telugu"),
    ("hrv", "hr", "Croatian"),
    ("mkd", "mk", "Macedonian"),
    ("srp", "sr", "Serbian"),
    ("isl", "is", "Icelandic"),
    ("mal", "ml", "Malayalam"),
    ("eus", "eu", "Basque"),
    ("cat", "ca", "Catalan; Valencian"),
    ("glg", "gl", "Galician"),
    ("kan", "kn", "Kannada"),
    ("kaz", "kk", "Kazakh"),
    ("fas", "fa", "Persian"),
    ("fil", "", "Filipino"),
    ("sin", "si", "Sinhala"),
    ("ben", "bn", "Bengali"),
    ("bos", "bs", "Bosnian"),
    ("urd", "ur", "Urdu"),
    ("aze", "az", "Azerbaijani"),
    ("kir", "ky", "Kirghiz"),
    ("bel", "be", "Belarusian"),
    ("kat", "ka", "Georgian"),
    ("hye", "hy", "Armenian"),
    ("sqi", "sq", "Albanian"),
    ("cym", "cy", "Welsh"),
    ("gle", "ga", "Irish"),
    ("lat", "la", "Latin"),
    ("afr", "af", "Afrikaans"),
    ("swa", "sw", "Swahili"),
    ("tgl", "tl", "Tagalog"),
    ("mon", "mn", "Mongolian"),
    ("uzb", "uz", "Uzbek"),
    ("mar", "mr", "Marathi"),
];

/// ISO 639-2 B codes → T codes.
fn t_code(code: &str) -> String {
    let c = code.to_ascii_lowercase();
    let b = [
        ("fre", "fra"),
        ("ger", "deu"),
        ("chi", "zho"),
        ("dut", "nld"),
        ("cze", "ces"),
        ("rum", "ron"),
        ("gre", "ell"),
        ("slo", "slk"),
        ("per", "fas"),
        ("may", "msa"),
        ("ice", "isl"),
        ("mac", "mkd"),
        ("baq", "eus"),
        ("alb", "sqi"),
        ("arm", "hye"),
        ("geo", "kat"),
        ("wel", "cym"),
        ("scc", "srp"),
        ("scr", "hrv"),
    ];
    b.iter().find(|(k, _)| *k == c).map(|(_, v)| v.to_string()).unwrap_or(c)
}

/// A language code (2 or 3 letters) or English name → the 3-letter code.
fn language_code(token: &str) -> Option<String> {
    let t = token.to_ascii_lowercase();
    if t.len() == 3 {
        let t = t_code(&t);
        return LANGUAGES.iter().find(|(c, _, _)| *c == t).map(|(c, _, _)| c.to_string());
    }
    if t.len() == 2 {
        return LANGUAGES.iter().find(|(_, c, _)| *c == t).map(|(c, _, _)| c.to_string());
    }
    LANGUAGES
        .iter()
        .find(|(_, _, n)| n.eq_ignore_ascii_case(&t))
        .map(|(c, _, _)| c.to_string())
}

/// The language shown in labels ("English", "Norwegian (Bokmal)"; the first of a list).
fn language_name(code: &str) -> String {
    match LANGUAGES.iter().find(|(c, _, _)| *c == code) {
        Some((_, _, n)) => n.split(';').next().unwrap_or(n).to_string(),
        None => first_upper(code),
    }
}

fn first_upper(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

const SPECIAL_LANGUAGES: [&str; 4] = ["mis", "mul", "und", "zxx"];

fn audio_codec_name(codec: &str) -> String {
    match codec {
        "ac3" => "Dolby Digital".into(),
        "eac3" => "Dolby Digital+".into(),
        "dts" => "DTS".into(),
        c => c.to_uppercase(),
    }
}

fn subtitle_codec(ffprobe: &str) -> String {
    match ffprobe {
        "dvd_subtitle" => "DVDSUB".into(),
        "hdmv_pgs_subtitle" => "PGSSUB".into(),
        "dvb_subtitle" => "DVBSUB".into(),
        c => c.to_string(),
    }
}

const TEXT_SUBTITLES: [&str; 9] = ["subrip", "srt", "ass", "ssa", "webvtt", "mov_text", "text", "microdvd", "subviewer"];

/// The resolution label.
fn resolution(width: i64, height: i64, interlaced: bool) -> Option<String> {
    let p = if interlaced { "i" } else { "p" };
    Some(match width {
        w if w <= 256 && height <= 144 => format!("144{p}"),
        w if w <= 426 && height <= 240 => format!("240{p}"),
        w if w <= 640 && height <= 360 => format!("360{p}"),
        w if w <= 682 && height <= 384 => format!("384{p}"),
        w if w <= 720 && height <= 404 => format!("404{p}"),
        w if w <= 854 && height <= 480 => format!("480{p}"),
        w if w <= 960 && height <= 544 => format!("540{p}"),
        w if w <= 1024 && height <= 576 => format!("576{p}"),
        w if w <= 1280 && height <= 962 => format!("720{p}"),
        w if w <= 2560 && height <= 1440 => format!("1080{p}"),
        w if w <= 4096 && height <= 3072 => "4K".into(),
        w if w <= 8192 && height <= 6144 => "8K".into(),
        _ => return None,
    })
}

/// (VideoRangeType, the Dolby Vision title) from ffprobe's stream.
fn video_range(p: &Value) -> (String, Option<String>) {
    let transfer = p["color_transfer"].as_str().unwrap_or("");
    let dovi = p["side_data_list"]
        .as_array()
        .and_then(|l| l.iter().find(|s| s["side_data_type"].as_str().is_some_and(|t| t.contains("DOVI"))));
    if let Some(d) = dovi {
        let profile = d["dv_profile"].as_i64().unwrap_or(0);
        let compat = d["dv_bl_signal_compatibility_id"].as_i64().unwrap_or(0);
        let (kind, base) = match compat {
            1 => ("DOVIWithHDR10", " (HDR10)"),
            2 => ("DOVIWithSDR", " (SDR)"),
            4 => ("DOVIWithHLG", " (HLG)"),
            _ => ("DOVI", ""),
        };
        let title = if compat > 0 {
            format!("Dolby Vision Profile {profile}.{compat}{base}")
        } else {
            format!("Dolby Vision Profile {profile}")
        };
        return (kind.into(), Some(title));
    }
    let kind = match transfer {
        "smpte2084" => "HDR10",
        "arib-std-b67" => "HLG",
        _ => "SDR",
    };
    (kind.into(), None)
}

fn tag<'a>(p: &'a Value, name: &str) -> Option<&'a str> {
    p["tags"]
        .as_object()?
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .and_then(|(_, v)| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn flag(p: &Value, name: &str) -> bool {
    p["disposition"][name].as_i64() == Some(1)
}

/// The label from attributes: the title plus each attribute it doesn't contain (" - "), or the
/// attributes alone joined by `sep`.
fn label(title: Option<&str>, attrs: &[String], sep: &str) -> String {
    match title {
        Some(t) => {
            let lower = t.to_lowercase();
            let mut out = t.to_string();
            for a in attrs {
                if !lower.contains(&a.to_lowercase()) {
                    out += " - ";
                    out += a;
                }
            }
            out
        }
        None => attrs.join(sep),
    }
}

fn language_attr(lang: Option<&str>) -> Option<String> {
    lang.filter(|l| !SPECIAL_LANGUAGES.contains(l)).map(language_name)
}

/// One of the file's own streams (`n_external` sidecars come before it).
fn embedded(p: &Value, n_external: i64) -> Option<Stream> {
    let index = p["index"].as_i64()? + n_external;
    let codec_type = p["codec_type"].as_str().unwrap_or("");
    let codec_name = p["codec_name"].as_str().unwrap_or("").to_string();
    let language = tag(p, "language").map(t_code);
    // The title: the title tag, else an MP4 track name, else a handler name that says
    // something.
    let handler = tag(p, "handler_name").filter(|h| !["SoundHandler", "VideoHandler", "SubtitleHandler", "TextHandler"].contains(h));
    let title = if codec_type == "video" {
        tag(p, "title")
    } else {
        tag(p, "title").or_else(|| tag(p, "name")).or(handler)
    }
    .map(String::from);
    let (is_default, is_forced, hi) = (flag(p, "default"), flag(p, "forced"), flag(p, "hearing_impaired"));
    let mut s = Stream {
        index,
        kind: String::new(),
        codec: codec_name.clone(),
        language: language.clone(),
        title: title.clone(),
        display_title: String::new(),
        is_default,
        is_forced,
        is_hearing_impaired: hi,
        is_external: false,
        is_text: false,
        path: None,
        width: p["width"].as_i64(),
        height: p["height"].as_i64(),
        channels: p["channels"].as_i64(),
        channel_layout: None,
        profile: p["profile"].as_str().map(String::from),
        bitrate: p["bit_rate"].as_str().and_then(|b| b.parse().ok()),
        video_range_type: None,
    };
    match codec_type {
        "video" if flag(p, "attached_pic") => {
            s.kind = "EmbeddedImage".into();
            s.display_title = String::new();
        }
        "video" => {
            s.kind = "Video".into();
            let (range, dovi) = video_range(p);
            let interlaced = p["field_order"].as_str().is_some_and(|f| f != "progressive" && f != "unknown");
            let mut attrs = Vec::new();
            if let Some(r) = resolution(s.width.unwrap_or(0), s.height.unwrap_or(0), interlaced) {
                attrs.push(r);
            }
            if !codec_name.is_empty() {
                attrs.push(codec_name.to_uppercase());
            }
            attrs.push(dovi.unwrap_or_else(|| if range == "SDR" { "SDR".into() } else { "HDR".into() }));
            s.display_title = label(title.as_deref(), &attrs, " ");
            s.video_range_type = Some(range);
        }
        "audio" => {
            s.kind = "Audio".into();
            let layout = p["channel_layout"]
                .as_str()
                .map(|l| l.replace("(side)", "").trim().to_string())
                .filter(|l| !l.is_empty())
                .or_else(|| match s.channels {
                    Some(1) => Some("mono".into()),
                    Some(2) => Some("stereo".into()),
                    Some(6) => Some("5.1".into()),
                    Some(8) => Some("7.1".into()),
                    _ => None,
                });
            s.channel_layout.clone_from(&layout);
            let mut attrs = Vec::new();
            attrs.extend(language_attr(language.as_deref()));
            match s.profile.as_deref() {
                Some(pr) if !pr.eq_ignore_ascii_case("lc") => attrs.push(pr.to_string()),
                _ if !codec_name.is_empty() => attrs.push(audio_codec_name(&codec_name)),
                _ => {}
            }
            match (&layout, s.channels) {
                (Some(l), _) => attrs.push(first_upper(l)),
                (None, Some(c)) => attrs.push(format!("{c} ch")),
                _ => {}
            }
            if is_default {
                attrs.push("Default".into());
            }
            if flag(p, "original") {
                attrs.push("Original".into());
            }
            s.display_title = label(title.as_deref(), &attrs, " - ");
        }
        "subtitle" => {
            s.kind = "Subtitle".into();
            s.codec = subtitle_codec(&codec_name);
            s.is_text = TEXT_SUBTITLES.contains(&codec_name.as_str());
            s.profile = None;
            s.display_title = subtitle_label(&s);
        }
        "data" => {
            s.kind = "Data".into();
        }
        _ => return None,
    }
    Some(s)
}

fn subtitle_label(s: &Stream) -> String {
    let mut attrs = Vec::new();
    match language_attr(s.language.as_deref()) {
        Some(l) => attrs.push(l),
        None => attrs.push("Undefined".into()),
    }
    if s.is_hearing_impaired {
        attrs.push("Hearing Impaired".into());
    }
    if s.is_default {
        attrs.push("Default".into());
    }
    if s.is_forced {
        attrs.push("Forced".into());
    }
    if !s.codec.is_empty() {
        attrs.push(s.codec.to_uppercase());
    }
    if s.is_external {
        attrs.push("External".into());
    }
    label(s.title.as_deref(), &attrs, " - ")
}

const SIDECAR_SUBS: [&str; 8] = ["srt", "ass", "ssa", "vtt", "sub", "idx", "sup", "smi"];
const SIDECAR_AUDIO: [&str; 7] = ["ac3", "eac3", "dts", "mka", "aac", "flac", "mp3"];

/// The sidecar files of a video among the names in its folder: name starting with the video's
/// (without its extension) and a dot, subtitle or audio extension. Sorted by name, ignoring case.
pub fn sidecars_in(video: &Path, names: &[String]) -> Vec<String> {
    let (Some(dir), Some(stem)) = (video.parent(), video.file_stem().and_then(|s| s.to_str())) else {
        return Vec::new();
    };
    let prefix = format!("{stem}.").to_lowercase();
    let mut out: Vec<String> = names
        .iter()
        .filter(|n| n.to_lowercase().starts_with(&prefix))
        .filter(|n| {
            n.rsplit_once('.').is_some_and(|(_, e)| {
                SIDECAR_SUBS.contains(&e.to_lowercase().as_str()) || SIDECAR_AUDIO.contains(&e.to_lowercase().as_str())
            })
        })
        .map(|n| dir.join(n).to_string_lossy().into_owned())
        .collect();
    out.sort_by_key(|p| p.to_lowercase());
    out
}

/// A sidecar as a stream: language, title and flags from the name after the video's.
fn sidecar(video_stem: &str, path: &str, index: i64) -> Stream {
    let name = Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("");
    let (rest, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let ext = ext.to_lowercase();
    let suffix = rest.get(video_stem.len()..).unwrap_or("").trim_start_matches('.');
    let (mut language, mut forced, mut default, mut hi) = (None, false, false, false);
    let mut title_parts = Vec::new();
    for tok in suffix.split('.').filter(|t| !t.is_empty()) {
        let l = tok.to_ascii_lowercase();
        if language.is_none() && title_parts.is_empty() {
            if let Some(code) = language_code(tok) {
                language = Some(code);
                continue;
            }
        }
        match l.as_str() {
            "forced" | "foreign" => forced = true,
            "default" => default = true,
            "sdh" | "hi" | "cc" => hi = true,
            _ => title_parts.push(tok),
        }
    }
    let audio = SIDECAR_AUDIO.contains(&ext.as_str());
    let codec = match ext.as_str() {
        "srt" => "subrip",
        "vtt" => "webvtt",
        "idx" | "sub" => "DVDSUB",
        "sup" => "PGSSUB",
        "smi" => "smi",
        e => e,
    }
    .to_string();
    let mut s = Stream {
        index,
        kind: if audio { "Audio".into() } else { "Subtitle".into() },
        codec,
        language,
        title: (!title_parts.is_empty()).then(|| title_parts.join(".")),
        display_title: String::new(),
        is_default: default,
        is_forced: forced,
        is_hearing_impaired: hi,
        is_external: true,
        is_text: !audio && TEXT_SUBTITLES.contains(&ext.as_str()) || ext == "srt" || ext == "vtt",
        path: Some(path.to_string()),
        width: None,
        height: None,
        channels: None,
        channel_layout: None,
        profile: None,
        bitrate: None,
        video_range_type: None,
    };
    s.display_title = if audio {
        let mut attrs: Vec<String> = language_attr(s.language.as_deref()).into_iter().collect();
        attrs.push(audio_codec_name(&s.codec));
        attrs.push("External".into());
        label(s.title.as_deref(), &attrs, " - ")
    } else {
        subtitle_label(&s)
    };
    s
}

/// Every track of a file, numbered and labelled for the app.
pub fn derive(video: &Path, probe: &Value, sidecars: &[String]) -> Vec<Stream> {
    let stem = video.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let mut out: Vec<Stream> = sidecars.iter().enumerate().map(|(i, p)| sidecar(stem, p, i as i64)).collect();
    let n = out.len() as i64;
    if let Some(streams) = probe["streams"].as_array() {
        out.extend(streams.iter().filter_map(|p| embedded(p, n)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn video_labels() {
        let p = json!({"index": 0, "codec_type": "video", "codec_name": "h264", "width": 1280, "height": 536, "profile": "High"});
        assert_eq!(embedded(&p, 0).unwrap().display_title, "720p H264 SDR");
        let p = json!({"index": 0, "codec_type": "video", "codec_name": "h264", "width": 1920, "height": 800, "tags": {"handler_name": "video.264#trackID=1:fps=23.976 - Imported with GPAC 0.5.0-rev"}});
        assert_eq!(embedded(&p, 0).unwrap().display_title, "1080p H264 SDR");
        let p = json!({"index": 0, "codec_type": "video", "codec_name": "h264", "width": 704, "height": 528, "tags": {"title": "Alf.s01e24. Weird Science"}});
        assert_eq!(
            embedded(&p, 0).unwrap().display_title,
            "Alf.s01e24. Weird Science - 540p - H264 - SDR"
        );
        let p =
            json!({"index": 0, "codec_type": "video", "codec_name": "hevc", "width": 3840, "height": 2076, "color_transfer": "smpte2084"});
        let s = embedded(&p, 0).unwrap();
        assert_eq!(
            (s.display_title.as_str(), s.video_range_type.as_deref()),
            ("4K HEVC HDR", Some("HDR10"))
        );
        let p = json!({"index": 0, "codec_type": "video", "codec_name": "hevc", "width": 3840, "height": 1606, "color_transfer": "smpte2084",
            "side_data_list": [{"side_data_type": "DOVI configuration record", "dv_profile": 8, "dv_bl_signal_compatibility_id": 1}]});
        assert_eq!(embedded(&p, 0).unwrap().display_title, "4K HEVC Dolby Vision Profile 8.1 (HDR10)");
    }

    #[test]
    fn audio_labels() {
        let a = |v: Value| embedded(&v, 0).unwrap().display_title;
        assert_eq!(
            a(
                json!({"index": 1, "codec_type": "audio", "codec_name": "aac", "profile": "LC", "channels": 6, "channel_layout": "5.1", "tags": {"language": "eng"}, "disposition": {"default": 1}})
            ),
            "English - AAC - 5.1 - Default"
        );
        assert_eq!(
            a(json!({"index": 1, "codec_type": "audio", "codec_name": "mp3", "channels": 2, "channel_layout": "stereo"})),
            "MP3 - Stereo"
        );
        assert_eq!(
            a(
                json!({"index": 1, "codec_type": "audio", "codec_name": "aac", "profile": "LC", "channels": 2, "channel_layout": "stereo", "tags": {"language": "rus", "title": "Russian AAC"}})
            ),
            "Russian AAC - Stereo"
        );
        assert_eq!(
            a(
                json!({"index": 1, "codec_type": "audio", "codec_name": "ac3", "channels": 6, "channel_layout": "5.1(side)", "tags": {"language": "rus", "title": "DD 5.1 @ 448 kbps (AVO Goblin)"}, "disposition": {"default": 1}})
            ),
            "DD 5.1 @ 448 kbps (AVO Goblin) - Russian - Dolby Digital - Default"
        );
        assert_eq!(
            a(
                json!({"index": 1, "codec_type": "audio", "codec_name": "eac3", "profile": "Dolby Digital Plus + Dolby Atmos", "channels": 6, "channel_layout": "5.1(side)", "tags": {"language": "eng", "title": "English"}})
            ),
            "English - Dolby Digital Plus + Dolby Atmos - 5.1"
        );
        assert_eq!(
            a(
                json!({"index": 1, "codec_type": "audio", "codec_name": "eac3", "channels": 6, "channel_layout": "5.1(side)", "tags": {"language": "eng"}, "disposition": {"default": 1, "original": 1}})
            ),
            "English - Dolby Digital+ - 5.1 - Default - Original"
        );
        assert_eq!(
            a(
                json!({"index": 1, "codec_type": "audio", "codec_name": "ac3", "channels": 6, "channel_layout": "5.1(side)", "tags": {"language": "eng", "handler_name": "SoundHandler", "name": "English"}, "disposition": {"default": 1}})
            ),
            "English - Dolby Digital - 5.1 - Default"
        );
        assert_eq!(
            a(
                json!({"index": 1, "codec_type": "audio", "codec_name": "aac", "channels": 2, "channel_layout": "stereo", "tags": {"language": "eng", "handler_name": "GPAC ISO Audio Handler"}, "disposition": {"default": 1}})
            ),
            "GPAC ISO Audio Handler - English - AAC - Stereo - Default"
        );
        assert_eq!(
            a(
                json!({"index": 1, "codec_type": "audio", "codec_name": "truehd", "channels": 8, "channel_layout": "7.1", "tags": {"language": "eng", "title": "Original"}})
            ),
            "Original - English - TRUEHD - 7.1"
        );
    }

    #[test]
    fn subtitles_and_sidecars() {
        let s = embedded(&json!({"index": 3, "codec_type": "subtitle", "codec_name": "subrip", "tags": {"language": "eng", "title": "SDH"}, "disposition": {"hearing_impaired": 1}}), 2).unwrap();
        assert_eq!(
            (s.index, s.display_title.as_str(), s.is_text),
            (5, "SDH - English - Hearing Impaired - SUBRIP", true)
        );
        let s = embedded(
            &json!({"index": 4, "codec_type": "subtitle", "codec_name": "hdmv_pgs_subtitle", "tags": {"language": "spa"}}),
            0,
        )
        .unwrap();
        assert_eq!((s.display_title.as_str(), s.is_text), ("Spanish - PGSSUB", false));
        let stem = "The.Rehearsal.S01E01.1080p.WEBRip.x265-RARBG[eztv.re]";
        let side = |n: &str, i| sidecar(stem, &format!("/m/{stem}.{n}"), i);
        assert_eq!(side("English.2.srt", 0).display_title, "2 - English - SUBRIP - External");
        assert_eq!(side("Dutch.8.srt", 0).display_title, "Dutch.8 - Undefined - SUBRIP - External");
        assert_eq!(side("mac.13.srt", 0).display_title, "13 - Macedonian - SUBRIP - External");
        assert_eq!(sidecar("X", "/m/X.srt", 0).display_title, "Undefined - SUBRIP - External");
        assert_eq!(
            sidecar("X", "/m/X.ukr.ac3", 1).display_title,
            "Ukrainian - Dolby Digital - External"
        );
    }
}
