//! Versions: one title, several files — other cuts (Director's Cut, Extended…), qualities (4K HDR
//! and 1080p), dubs and translations, sources (BluRay, WEB). Files are one title when they are
//! identified as the same one (`items.key`); this says how each differs from the others, so a
//! player can offer the choice.
//!
//! What a file is, from (in order) the NFO next to it (`<edition>`, read once per file stamp:
//! `files.edition`), its own tags (Matroska/MP4 `EDITION`, a title naming a cut), the Plex-style
//! `{edition-…}` in its name, and edition words in its name or its folder's; resolution, HDR and
//! codec from its video track; the audio languages from its tracks and dub words in its name
//! (`Dub`, `Дубляж`, `MVO`…); the source from its name.
//!
//! `label`: a short text that tells a title's versions apart (only what differs between them:
//! "4K HDR · Director's Cut", "Russian Dub"); `details`: everything ("2160p · HEVC · HDR10 ·
//! English 5.1, Russian · BluRay · 18.2 GB").

use std::fmt::Write as _;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{json, Value};

use super::streams::Stream;

/// Edition words in names and tags, with the name shown for them (first match wins: longer
/// phrases first).
static EDITIONS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"director'?s[\s._-]*cut|dc[\s._-]*edition|\bdc\b", "Director's Cut"),
        (r"extended[\s._-]*(?:edition|cut|version)?", "Extended"),
        (r"theatrical(?:[\s._-]*(?:cut|edition|version))?", "Theatrical"),
        (r"unrated", "Unrated"),
        (r"uncut", "Uncut"),
        (r"imax", "IMAX"),
        (r"remaster(?:ed)?", "Remastered"),
        (r"final[\s._-]*cut", "Final Cut"),
        (r"ultimate[\s._-]*(?:cut|edition)", "Ultimate Edition"),
        (r"special[\s._-]*edition", "Special Edition"),
        (r"collector'?s[\s._-]*(?:cut|edition)", "Collector's Edition"),
        (r"anniversary[\s._-]*edition", "Anniversary Edition"),
        (r"criterion", "Criterion"),
        (r"open[\s._-]*matte", "Open Matte"),
        (r"redux", "Redux"),
        (r"alt(?:ernative|ernate)?[\s._-]*cut", "Alternative Cut"),
        (r"rough[\s._-]*cut", "Rough Cut"),
        (r"(?:режисс[её]рская|режиссерская)[\s._-]*версия", "Director's Cut"),
        (r"расширенная[\s._-]*версия", "Extended"),
        (r"театральная[\s._-]*версия", "Theatrical"),
    ]
    .into_iter()
    .map(|(p, n)| (Regex::new(&format!(r"(?i)(?:^|\W)(?:{p})(?:$|\W)")).unwrap(), n))
    .collect()
});

/// Plex's `{edition-Director's Cut}` in a file or folder name.
static PLEX_EDITION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\{edition-([^}]+)\}").unwrap());

/// Dub and voice-over words: the file's audio is a translation.
static DUB: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:^|[^\w]|_)(dub(?:bed)?|дубляж|дублированный|mvo|dvo|avo|многоголосый|двухголосый|одноголосый|закадровый)(?:$|[^\w]|_)",
    )
    .unwrap()
});

/// The source a release name gives.
static SOURCES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"remux", "Remux"),
        (r"blu-?ray|bdrip|brrip|bdremux|\bbd\b", "BluRay"),
        (r"web-?dl", "WEB-DL"),
        (r"web-?rip", "WEBRip"),
        (r"\bweb\b", "WEB"),
        (r"hdtv|hdtvrip", "HDTV"),
        (r"dvd-?rip|dvd5|dvd9|\bdvd\b", "DVD"),
        (r"\buhd\b", "UHD"),
    ]
    .into_iter()
    .map(|(p, n)| (Regex::new(&format!(r"(?i){p}")).unwrap(), n))
    .collect()
});

/// One file of a title, as a version.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Version {
    pub file_id: String,
    pub edition: Option<String>,
    /// "4K", "1080p", "720p", "SD".
    pub resolution: Option<String>,
    /// "HDR10", "HDR10+", "Dolby Vision", "HLG" (None: SDR).
    pub hdr: Option<String>,
    /// "HEVC", "H.264", "AV1"…
    pub codec: Option<String>,
    /// ISO 639-2 codes of the audio tracks (the default first), without repeats.
    pub audio: Vec<String>,
    /// "Dub", "MVO"… when the name says the audio is a translation.
    pub dub: Option<String>,
    pub source: Option<String>,
    pub size: i64,
    pub duration: Option<f64>,
    pub channels: Option<i64>,
    /// The file's name (versions alike in everything else are told apart by it).
    pub name: String,
}

/// The edition a name (file or folder) gives.
pub fn edition_from_name(name: &str) -> Option<String> {
    if let Some(c) = PLEX_EDITION.captures(name) {
        let e = c[1].trim();
        if !e.is_empty() {
            return Some(canonical_edition(e).unwrap_or_else(|| e.to_string()));
        }
    }
    canonical_edition(name)
}

fn canonical_edition(text: &str) -> Option<String> {
    let t = text.replace(['.', '_'], " ");
    EDITIONS.iter().find(|(re, _)| re.is_match(&t)).map(|(_, n)| n.to_string())
}

/// The edition a file's own tags give (Matroska/MP4 `EDITION`, or a title that names a cut).
pub fn edition_from_tags(probe: &Value) -> Option<String> {
    let tags = probe["format"]["tags"].as_object()?;
    for (k, v) in tags {
        if k.eq_ignore_ascii_case("edition") || k.eq_ignore_ascii_case("edition_name") {
            if let Some(s) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                return Some(canonical_edition(s).unwrap_or_else(|| s.to_string()));
            }
        }
    }
    tags.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("title"))
        .and_then(|(_, v)| v.as_str())
        .and_then(canonical_edition)
}

fn resolution(width: Option<i64>, height: Option<i64>) -> Option<String> {
    let (w, h) = (width.unwrap_or(0), height.unwrap_or(0));
    if w == 0 && h == 0 {
        return None;
    }
    // By width too: a 1920×800 scope film is 1080p.
    Some(
        if w >= 3200 || h >= 1800 {
            "4K"
        } else if w >= 1700 || h >= 1000 {
            "1080p"
        } else if w >= 1200 || h >= 700 {
            "720p"
        } else {
            "SD"
        }
        .to_string(),
    )
}

fn hdr_name(range: Option<&str>) -> Option<String> {
    let r = range?;
    Some(
        if r.starts_with("DOVI") {
            "Dolby Vision"
        } else if r == "HDR10Plus" {
            "HDR10+"
        } else if r == "HDR10" || r == "HDR" {
            "HDR10"
        } else if r == "HLG" {
            "HLG"
        } else {
            return None;
        }
        .to_string(),
    )
}

fn codec_name(c: &str) -> String {
    match c.to_ascii_lowercase().as_str() {
        "hevc" | "h265" => "HEVC".into(),
        "h264" | "avc" => "H.264".into(),
        "av1" => "AV1".into(),
        "vp9" => "VP9".into(),
        "mpeg4" | "msmpeg4v3" | "xvid" | "divx" => "MPEG-4".into(),
        "mpeg2video" => "MPEG-2".into(),
        "vc1" => "VC-1".into(),
        other => other.to_uppercase(),
    }
}

/// A file as a version: `name` its path (file and folder names), `tracks` its streams, `probe`
/// its stored probe (tags), `nfo_edition` what an NFO next to it says.
pub fn describe(
    file_id: &str,
    name: &str,
    size: i64,
    duration: Option<f64>,
    tracks: &[Stream],
    probe: Option<&Value>,
    nfo_edition: Option<&str>,
) -> Version {
    let file = name.rsplit('/').next().unwrap_or(name);
    let folder = name.rsplit('/').nth(1).unwrap_or("");
    let edition = nfo_edition
        .map(|e| canonical_edition(e).unwrap_or_else(|| e.trim().to_string()))
        .filter(|e| !e.is_empty())
        .or_else(|| probe.and_then(edition_from_tags))
        .or_else(|| edition_from_name(file))
        .or_else(|| edition_from_name(folder));
    let video = tracks.iter().find(|t| t.kind == "Video");
    let mut audio: Vec<String> = Vec::new();
    let mut audio_tracks: Vec<&Stream> = tracks.iter().filter(|t| t.kind == "Audio").collect();
    audio_tracks.sort_by_key(|t| !t.is_default);
    for t in &audio_tracks {
        if let Some(l) = t
            .language
            .as_deref()
            .map(str::to_ascii_lowercase)
            .filter(|l| !l.is_empty() && l != "und")
        {
            if !audio.contains(&l) {
                audio.push(l);
            }
        }
    }
    let dub = DUB.captures(file).map(|c| {
        let w = c[1].to_lowercase();
        match w.as_str() {
            "mvo" | "многоголосый" => "MVO".to_string(),
            "dvo" | "двухголосый" => "DVO".to_string(),
            "avo" | "одноголосый" => "AVO".to_string(),
            "закадровый" => "Voice-Over".to_string(),
            _ => "Dub".to_string(),
        }
    });
    let source = SOURCES.iter().find(|(re, _)| re.is_match(file)).map(|(_, n)| n.to_string());
    Version {
        file_id: file_id.to_string(),
        edition,
        resolution: video.and_then(|v| resolution(v.width, v.height)),
        hdr: video.and_then(|v| hdr_name(v.video_range_type.as_deref())),
        codec: video.map(|v| codec_name(&v.codec)),
        audio,
        dub,
        source,
        size,
        duration,
        channels: audio_tracks.first().and_then(|t| t.channels),
        name: file.to_string(),
    }
}

/// "English", "Russian" for an ISO 639 code.
pub fn language_name(code: &str) -> String {
    // ISO 639-2/T codes as their /B forms (the names are listed by those).
    let c = match code.to_ascii_lowercase().as_str() {
        "fra" => "fre".to_string(),
        "deu" => "ger".to_string(),
        "zho" => "chi".to_string(),
        "ces" => "cze".to_string(),
        "nld" => "dut".to_string(),
        "ell" => "gre".to_string(),
        "ron" => "rum".to_string(),
        "nob" | "nno" => "nor".to_string(),
        other => other.to_string(),
    };
    let n = crate::subtitles::subs::language_name(&c);
    if n.is_empty() || n == c {
        code.to_uppercase()
    } else {
        n
    }
}

/// Decimal units, as Apple's apps (and Moonrise's) show file sizes.
fn size_text(bytes: i64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{:.0} MB", bytes as f64 / 1e6)
    }
}

fn runtime_text(secs: f64) -> String {
    let m = (secs / 60.0).round() as i64;
    if m >= 60 {
        format!("{} h {} min", m / 60, m % 60)
    } else {
        format!("{m} min")
    }
}

fn quality(v: &Version) -> String {
    [v.resolution.clone(), v.hdr.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
}

fn languages(v: &Version) -> String {
    let mut s = v.audio.iter().take(2).map(|l| language_name(l)).collect::<Vec<_>>().join(", ");
    if v.audio.len() > 2 {
        let _ = write!(s, " +{}", v.audio.len() - 2);
    }
    s
}

/// Short labels that tell the versions apart, one per version (same order): only the qualities
/// in which they differ; files alike in all of them get their size (then their length) to tell
/// them apart. A single version gets its quality and edition.
pub fn labels(vs: &[Version]) -> Vec<String> {
    let differs = |f: &dyn Fn(&Version) -> String| vs.iter().map(f).collect::<std::collections::BTreeSet<_>>().len() > 1;
    let q = differs(&quality);
    let e = vs.iter().any(|v| v.edition.is_some());
    let l = differs(&languages);
    let d = differs(&|v: &Version| v.dub.clone().unwrap_or_default());
    let s = differs(&|v: &Version| v.source.clone().unwrap_or_default());
    let c = differs(&|v: &Version| v.codec.clone().unwrap_or_default());
    let mut out: Vec<String> = vs
        .iter()
        .map(|v| {
            let mut parts: Vec<String> = Vec::new();
            if q || vs.len() == 1 {
                parts.push(quality(v));
            }
            if e {
                parts.push(v.edition.clone().unwrap_or_default());
            }
            if l || d {
                let lang = v.audio.first().map(|a| language_name(a)).unwrap_or_default();
                parts.push(match (&v.dub, d) {
                    (Some(dub), _) if !lang.is_empty() => format!("{lang} {dub}"),
                    (Some(dub), _) => dub.clone(),
                    _ if l => languages(v),
                    _ => String::new(),
                });
            }
            if s && !q {
                parts.push(v.source.clone().unwrap_or_default());
            }
            if c && !q && !s {
                parts.push(v.codec.clone().unwrap_or_default());
            }
            parts.retain(|p| !p.is_empty());
            parts.join(" · ")
        })
        .collect();
    // Still alike (or empty): what their names say that the others' don't, else their sizes,
    // else their lengths, else "Version N".
    for i in 0..out.len() {
        let same: Vec<usize> = (0..out.len()).filter(|&j| out[j] == out[i]).collect();
        if same.len() > 1 || out[i].is_empty() {
            let names: Vec<&str> = same.iter().map(|&j| vs[j].name.as_str()).collect();
            if let Some(own) = name_differences(&names) {
                for (k, &j) in same.iter().enumerate() {
                    out[j] = [out[j].clone(), own[k].clone()]
                        .into_iter()
                        .filter(|p| !p.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · ");
                }
                continue;
            }
            for &j in &same {
                let alike = |f: &dyn Fn(&Version) -> String| same.iter().filter(|&&k| f(&vs[k]) == f(&vs[j])).count() == 1;
                let extra = if alike(&|w: &Version| size_text(w.size)) {
                    size_text(vs[j].size)
                } else if alike(&|w: &Version| w.duration.map(runtime_text).unwrap_or_default()) {
                    vs[j].duration.map(runtime_text).unwrap_or_default()
                } else {
                    String::new()
                };
                out[j] = [out[j].clone(), extra]
                    .into_iter()
                    .filter(|p| !p.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ");
            }
        }
    }
    let mut n = 0;
    for i in 0..out.len() {
        if out[i].is_empty() || out.iter().filter(|o| **o == out[i]).count() > 1 {
            n += 1;
            out[i] = format!("Version {n}");
        }
    }
    out
}

/// What each name has that none of the others do (words, in its order), when that tells them
/// all apart briefly ("S04E23 1" / "S04E24 2", "WEB H264" / "x265 ELiTE").
fn name_differences(names: &[&str]) -> Option<Vec<String>> {
    let stem = |n: &str| n.rsplit_once('.').map_or(n, |(a, _)| a).to_string();
    let words: Vec<Vec<String>> = names
        .iter()
        .map(|n| {
            stem(n)
                .split(|c: char| " ._-[](){}".contains(c))
                .filter(|w| !w.is_empty())
                .map(String::from)
                .collect()
        })
        .collect();
    let out: Vec<String> = words
        .iter()
        .enumerate()
        .map(|(i, ws)| {
            ws.iter()
                .filter(|w| {
                    words
                        .iter()
                        .enumerate()
                        .filter(|(j, _)| *j != i)
                        .all(|(_, o)| !o.iter().any(|x| x.eq_ignore_ascii_case(w)))
                })
                .cloned()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    let distinct = out.iter().collect::<std::collections::BTreeSet<_>>().len() == out.len();
    (distinct && out.iter().all(|o| !o.is_empty() && o.chars().count() <= 30)).then_some(out)
}

/// Everything about a version, for a second line.
pub fn details(v: &Version) -> String {
    let mut parts: Vec<String> = Vec::new();
    let res = match v.resolution.as_deref() {
        Some("4K") => Some("2160p".to_string()),
        r => r.map(String::from),
    };
    parts.extend(res);
    parts.extend(v.codec.clone());
    parts.extend(v.hdr.clone());
    if !v.audio.is_empty() {
        let ch = match v.channels {
            Some(c) if c >= 8 => " 7.1",
            Some(c) if c >= 6 => " 5.1",
            Some(2) => " Stereo",
            _ => "",
        };
        let mut s = format!("{}{ch}", language_name(&v.audio[0]));
        if let Some(d) = &v.dub {
            s = format!("{s} ({d})");
        }
        for a in v.audio.iter().skip(1).take(3) {
            let _ = write!(s, ", {}", language_name(a));
        }
        parts.push(s);
    }
    parts.extend(v.source.clone());
    if let Some(d) = v.duration {
        parts.push(runtime_text(d));
    }
    parts.push(size_text(v.size));
    parts.join(" · ")
}

/// The JSON a file's `version` carries.
pub fn json_of(v: &Version, label: &str) -> Value {
    json!({"label": label, "details": details(v), "edition": v.edition, "resolution": v.resolution, "hdr": v.hdr, "codec": v.codec,
           "audio_languages": v.audio, "dub": v.dub, "source": v.source})
}

/// Whether two versions are the same cut (a position in one is the same moment in the other):
/// lengths within 1 % (at least 3 s).
pub fn same_cut(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() <= (a.max(b) * 0.01).max(3.0),
        _ => true,
    }
}

impl super::Library {
    /// The editions NFOs give for the files of titles with several (once per file stamp; a
    /// batch per pass): `files.edition`.
    pub async fn refresh_editions(&self, limit: usize) -> anyhow::Result<usize> {
        let cond = format!(
            "COALESCE(edition_stamp, '') != size || '|' || mtime AND item_id IN
               (SELECT item_id FROM files WHERE role IN {r} AND missing_since IS NULL AND item_id IS NOT NULL GROUP BY item_id HAVING COUNT(*) > 1)",
            r = super::TITLE_ROLES
        );
        let rows: Vec<_> = self.file_rows(&cond).await?.into_iter().take(limit).collect();
        let n = rows.len();
        for (f, _, _) in rows {
            let edition = self
                .nfo_for(&f.path, &f.rel, "movie", false)
                .await
                .and_then(|n| n.edition)
                .filter(|e| !e.trim().is_empty());
            let id = f.id.clone();
            self.db
                .call(move |c| {
                    c.execute(
                        "UPDATE files SET edition = ?2, edition_stamp = size || '|' || mtime WHERE id = ?1",
                        rusqlite::params![id, edition],
                    )?;
                    Ok(())
                })
                .await?;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(id: &str, res: &str, hdr: Option<&str>, edition: Option<&str>, audio: &[&str], size_gb: f64) -> Version {
        Version {
            file_id: id.into(),
            edition: edition.map(String::from),
            resolution: Some(res.into()),
            hdr: hdr.map(String::from),
            codec: Some("HEVC".into()),
            audio: audio.iter().map(ToString::to_string).collect(),
            size: (size_gb * 1e9) as i64,
            duration: Some(7200.0),
            ..Default::default()
        }
    }

    #[test]
    fn editions_from_names() {
        assert_eq!(
            edition_from_name("Blade.Runner.1982.Final.Cut.1080p.BluRay.mkv").as_deref(),
            Some("Final Cut")
        );
        assert_eq!(
            edition_from_name("Kingdom of Heaven (2005) Director's Cut.mkv").as_deref(),
            Some("Director's Cut")
        );
        assert_eq!(
            edition_from_name("Aliens.1986.Special.Edition.mkv").as_deref(),
            Some("Special Edition")
        );
        assert_eq!(
            edition_from_name("Avatar (2009) {edition-Extended Collector's Edition}.mkv").as_deref(),
            Some("Extended")
        );
        assert_eq!(edition_from_name("Movie (2001) {edition-Fan Cut}.mkv").as_deref(), Some("Fan Cut"));
        assert_eq!(edition_from_name("The.Dark.Knight.2008.IMAX.2160p.mkv").as_deref(), Some("IMAX"));
        assert_eq!(edition_from_name("Troy.2004.DC.1080p.mkv").as_deref(), Some("Director's Cut"));
        assert_eq!(
            edition_from_name("Апокалипсис сегодня. Режиссёрская версия.mkv").as_deref(),
            Some("Director's Cut")
        );
        assert_eq!(edition_from_name("The.Matrix.1999.1080p.BluRay.x264.mkv"), None);
        // Words inside other words don't count.
        assert_eq!(edition_from_name("Dcuniverse.mkv"), None);
        assert_eq!(edition_from_name("The Uncutable.mkv"), None);
    }

    #[test]
    fn dub_and_source() {
        let x = describe("f", "/m/Movie (2001)/Movie.2001.1080p.WEB-DL.Dub.mkv", 1, None, &[], None, None);
        assert_eq!((x.dub.as_deref(), x.source.as_deref()), (Some("Dub"), Some("WEB-DL")));
        let x = describe("f", "/m/Фильм.2001.BDRip.MVO.mkv", 1, None, &[], None, None);
        assert_eq!((x.dub.as_deref(), x.source.as_deref()), (Some("MVO"), Some("BluRay")));
        let x = describe("f", "/m/Movie.2001.Remux.mkv", 1, None, &[], None, Some("Theatrical Cut"));
        assert_eq!((x.edition.as_deref(), x.source.as_deref()), (Some("Theatrical"), Some("Remux")));
        let x = describe("f", "/m/Movie Director's Cut (2001)/movie.mkv", 1, None, &[], None, None);
        assert_eq!(x.edition.as_deref(), Some("Director's Cut"), "the folder's name");
        let probe = json!({"format": {"tags": {"EDITION": "Extended Edition"}}});
        let x = describe("f", "/m/movie.mkv", 1, None, &[], Some(&probe), None);
        assert_eq!(x.edition.as_deref(), Some("Extended"));
    }

    #[test]
    fn labels_say_what_differs() {
        // Two cuts, same quality.
        let vs = [
            v("a", "1080p", None, Some("Theatrical"), &["eng"], 10.0),
            v("b", "1080p", None, Some("Director's Cut"), &["eng"], 12.0),
        ];
        assert_eq!(labels(&vs), ["Theatrical", "Director's Cut"]);
        // Two qualities.
        let vs = [
            v("a", "4K", Some("HDR10"), None, &["eng"], 40.0),
            v("b", "1080p", None, None, &["eng"], 10.0),
        ];
        assert_eq!(labels(&vs), ["4K HDR10", "1080p"]);
        // Quality and edition.
        let vs = [
            v("a", "4K", Some("Dolby Vision"), Some("Director's Cut"), &["eng"], 40.0),
            v("b", "1080p", None, None, &["eng"], 10.0),
        ];
        assert_eq!(labels(&vs), ["4K Dolby Vision · Director's Cut", "1080p"]);
        // A dub.
        let mut b = v("b", "1080p", None, None, &["rus"], 8.0);
        b.dub = Some("Dub".into());
        let vs = [v("a", "1080p", None, None, &["eng"], 10.0), b];
        assert_eq!(labels(&vs), ["English", "Russian Dub"]);
        // Alike in everything but size.
        let vs = [
            v("a", "1080p", None, None, &["eng"], 10.0),
            v("b", "1080p", None, None, &["eng"], 4.4),
        ];
        assert_eq!(labels(&vs), ["10.0 GB", "4.4 GB"]);
        // Alike but in name: what the names say.
        let mut a = v("a", "1080p", None, None, &["eng"], 0.56);
        let mut b = v("b", "1080p", None, None, &["eng"], 0.53);
        a.name = "Friends (1994) - S04E23 - The One With Ross's Wedding (1) (1080p BluRay x265 Silence).mkv".into();
        b.name = "Friends (1994) - S04E24 - The One With Ross's Wedding (2) (1080p BluRay x265 Silence).mkv".into();
        assert_eq!(labels(&[a, b]), ["S04E23 1", "S04E24 2"]);
        // Exactly alike.
        let vs = [
            v("a", "1080p", None, None, &["eng"], 10.0),
            v("b", "1080p", None, None, &["eng"], 10.0),
        ];
        assert_eq!(labels(&vs), ["Version 1", "Version 2"]);
        // One: its quality.
        assert_eq!(labels(&[v("a", "4K", Some("HDR10"), None, &["eng"], 40.0)]), ["4K HDR10"]);
    }

    #[test]
    fn cuts_of_another_length() {
        assert!(same_cut(Some(7200.0), Some(7230.0)));
        assert!(!same_cut(Some(7200.0), Some(8100.0)));
        assert!(same_cut(Some(1335.4), Some(1340.4)), "within 3 s");
        assert!(same_cut(None, Some(1.0)));
    }
}
