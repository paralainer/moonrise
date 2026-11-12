//! What a file's path says, with no library type: a movie (title, year, alternative titles) or
//! an episode (its show, season, episode numbers), and whether it is a title at all (extras,
//! samples). Any folder may hold movies, show folders and loose episodes side by side, at any
//! depth. `classify` decides from the path and the names of the other videos in its folder:
//!
//! - **Episode** (strong): `SxxEyy` (`S01E01E02`, `S04E18-E19`), `1x02`, a season folder
//!   (`Season 2`, `Series 2`, `S02`, `Specials`, `Сезон 2`) and a number in the name (`Episode
//!   3`, `03 - Title`, `E03`), `Episode N` in a name without a year, an air date
//!   (`Show.2024.03.15`), a fansub name (`[Group] Show - 12`, absolute numbering).
//! - **Episode** (weak, `weak`): three or more videos in one folder named alike but for a number
//!   (`Show - 01`, `Chernobyl.Part.1`, `01 - Pilot`). Identification checks these against TMDB:
//!   a movie of that name that fits wins.
//! - **Movie**: everything else (a title and year, a lone video in its own folder, edition words).
//!
//! NFOs (`<episodedetails>`, `<movie>`, a `tvshow.nfo` above) and TMDB refine this in
//! `identify.rs`. The show of an episode is the folder above its season folder; else the nearest
//! folder (above per-episode and season-pack release folders) named like the show in the file
//! name; else the show's name in the file name (`series_folder` is then `<folder>/<name>`, so
//! the loose episodes of one show group together).
//!
//! The layouts seen on real shares (popcorn-tracker keeps release names): loose
//! `Title.Year.tags.ext`, `Title (Year) [tags]/…`, release folders, collection folders
//! (`Harry Potter/2. Harry Potter and the Chamber of.Secrets/HP2.….mkv`), `<Show (Year)>/Season
//! N/<…SxxEyy…>` (also a folder per episode), extras in `Featurettes`.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Video file extensions (lower case).
pub const VIDEO_EXTS: [&str; 11] = ["mkv", "mp4", "m4v", "mov", "avi", "mpg", "mpeg", "ts", "m2ts", "webm", "wmv"];

/// Folders whose videos are extras, not titles (the usual media-server list), compared lower-case.
const EXTRA_DIRS: [&str; 12] = [
    "extras",
    "extra",
    "featurettes",
    "featurette",
    "behind the scenes",
    "deleted scenes",
    "interviews",
    "trailers",
    "trailer",
    "backdrops",
    "theme-music",
    "specials features",
];
/// Extras folders with everyday names: only below a title's folder, not at the top of a root
/// (where "Shorts" or "Other" may be a collection of titles).
const EXTRA_DIRS_NESTED: [&str; 4] = ["scenes", "shorts", "other", "clips"];
/// File name endings that make a video an extra (`Title-trailer.mkv`, as media servers name them).
const EXTRA_SUFFIXES: [&str; 11] = [
    "-trailer",
    "-featurette",
    "-behindthescenes",
    "-deleted",
    "-deletedscene",
    "-interview",
    "-scene",
    "-short",
    "-other",
    "-extra",
    "-clip",
];
/// Folders that only sort titles ("Movies", "TV Shows"): never a title's or a show's name.
const CATEGORY_DIRS: [&str; 22] = [
    "movies",
    "movie",
    "films",
    "film",
    "tv",
    "tv shows",
    "tvshows",
    "tv series",
    "shows",
    "series",
    "media",
    "video",
    "videos",
    "kids",
    "anime",
    "cartoons",
    "documentaries",
    "downloads",
    "complete",
    "new",
    "фильмы",
    "сериалы",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Parsed {
    /// "movie" or "episode"; "extra" and "sample" aren't titles.
    pub kind: String,
    /// Movies: the best guess; episodes: the episode title in the name, if any.
    pub title: String,
    pub year: Option<i32>,
    /// Other readings of the title (the folder's, a second script's).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alt_titles: Vec<(String, Option<i32>)>,
    /// Episodes: the show's folder (relative to the library root; for loose episodes
    /// `<folder>/<show name>`), its title and year.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_folder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_end: Option<i32>,
    /// A part (`Pt.1`, `CD2`), when the name says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<i32>,
    /// Why it is what it is: `sxxeyy`, `nxnn`, `season-folder`, `episode-word`, `date`,
    /// `fansub`, `numbered`, `nfo`, `manual`, or `movie`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signal: String,
    /// An episode by a weak signal only (names numbered alike): TMDB may make it a movie.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub weak: bool,
    /// An episode named by its air date (`YYYY-MM-DD`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub air_date: Option<String>,
}

static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[(\[]?((?:19|20)\d\d)[)\]]?$").unwrap());
static SXXEYY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[^a-z0-9])s(\d{1,2})[ ._-]?e(\d{1,3})(-\d{1,3}(?:[^0-9]|$)|(?:[ ._]?-?[ ._]?e\d{1,3})*)").unwrap()
});
static NXNN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|[^a-z0-9])(\d{1,2})x(\d{2,3})(?:-(\d{2,3}))?(?:[^a-z0-9]|$)").unwrap());
static EPISODE_N: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:episode|ep)[ ._]?(\d{1,3})\b").unwrap());
static E_N: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:^|[ ._-])e(\d{1,3})(?:[ ._-]|$)").unwrap());
static LEADING_N: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{1,3})(?:[ ._-]|$)").unwrap());
static SEASON_DIR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:season|series|сезон|staffel|saison|temporada)[ ._-]*(\d{1,3})(?:\s*[(\[].*)?$|^s(\d{1,2})$").unwrap()
});
/// A season pack's release folder (`Show.S02.1080p.WEB`, `Show Season 2 Complete`).
static SEASON_PACK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[ ._-])(?:s\d{1,2}(?:-s?\d{1,2})?|season[ ._-]?\d{1,2}|complete[ ._-]series)(?:[ ._-]|$)").unwrap()
});
static AIR_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[^0-9])((?:19|20)\d\d)[ ._-](0[1-9]|1[0-2])[ ._-](0[1-9]|[12]\d|3[01])(?:[^0-9]|$)").unwrap());
static FANSUB: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[[^\]]+\]\s*(.+?)\s+-\s+(\d{1,4})(?:v\d)?(?:[ .\[(]|$)").unwrap());
static EXTRA_E: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)[e-](\d{1,3})").unwrap());
static PART: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:pt|part|cd|disc)[ ._]?(\d)\b").unwrap());
static SITE_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^www\.[a-z0-9.-]+\s*-\s*").unwrap());
static COLLECTION_NO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{1,2}(?:\.\d)?\.?\s+").unwrap());
static SAMPLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:^|[^a-z])sample(?:[^a-z]|$)").unwrap());

/// Words that end a title in a release name (compared lower-case, after splitting on
/// separators).
const TAGS: &[&str] = &[
    "1080p",
    "720p",
    "2160p",
    "480p",
    "576p",
    "1080i",
    "4k",
    "uhd",
    "bdrip",
    "bdremux",
    "bluray",
    "blu-ray",
    "brrip",
    "web-dl",
    "webdl",
    "webrip",
    "web-dlrip",
    "web",
    "hdrip",
    "dvdrip",
    "dvd",
    "dvd9",
    "dvd5",
    "hdtv",
    "hdtvrip",
    "remux",
    "x264",
    "x265",
    "h264",
    "h265",
    "h.264",
    "h.265",
    "hevc",
    "avc",
    "xvid",
    "divx",
    "10bit",
    "aac",
    "ac3",
    "dts",
    "ddp5",
    "dd5",
    "atmos",
    "hdr",
    "sdr",
    "dv",
    "imax",
    "extended",
    "unrated",
    "directors",
    "remastered",
    "open",
    "proper",
    "repack",
    "dual",
    "rus",
    "eng",
    "ukr",
    "multi",
    "amzn",
    "dsnp",
    "hulu",
    "nf",
    "itunes",
    "ita",
    "rm4k",
    "rm",
    "mvo",
    "dub",
    "iptvrip",
    "satrip",
    "tvrip",
    "camrip",
];

/// What a path (relative to its library root) says on its own; see `classify`.
pub fn parse(rel: &str) -> Parsed {
    classify(rel, &[])
}

/// A path relative to its library root, as a movie or an episode (or not a title), with the
/// file names of the other title videos in its folder (`siblings`; numbered names).
pub fn classify(rel: &str, siblings: &[&str]) -> Parsed {
    let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
    let file = parts.last().copied().unwrap_or("");
    let dirs = &parts[..parts.len().saturating_sub(1)];
    let stem = stem_of(file);
    if let Some(p) = not_a_title(dirs, stem) {
        return p;
    }
    let season_at = dirs.iter().rposition(|d| season_dir(d).is_some());
    let folder_season = season_at.and_then(|i| season_dir(dirs[i]));
    // Numbers in the name, strongest first.
    if let Some(c) = SXXEYY.captures(stem) {
        let mut p = Parsed {
            season: c[1].parse().ok(),
            episode: c[2].parse().ok(),
            signal: "sxxeyy".into(),
            ..Default::default()
        };
        let more: Vec<i32> = EXTRA_E.captures_iter(&c[3]).filter_map(|m| m[1].parse().ok()).collect();
        if let Some(&end) = more.last() {
            if p.episode.is_some_and(|e| end > e && end - e < 10) {
                p.episode_end = Some(end);
            }
        }
        let m = c.get(0).unwrap();
        p.title = episode_title(&stem[m.end()..]);
        return episode(p, dirs, season_at, &stem[..m.start()], stem);
    }
    if let Some(c) = NXNN.captures(stem).filter(|c| !matches!(&c[2], "264" | "265")) {
        let mut p = Parsed {
            season: c[1].parse().ok(),
            episode: c[2].parse().ok(),
            signal: "nxnn".into(),
            ..Default::default()
        };
        p.episode_end = c
            .get(3)
            .and_then(|m| m.as_str().parse().ok())
            .filter(|e| p.episode.is_some_and(|x| *e > x));
        let m = c.get(0).unwrap();
        p.title = episode_title(&stem[m.end()..]);
        return episode(p, dirs, season_at, &stem[..m.start()], stem);
    }
    let spaced = stem.replace('_', " ");
    if let Some(c) = FANSUB.captures(&spaced) {
        let p = Parsed {
            season: folder_season,
            episode: c[2].parse().ok(),
            signal: "fansub".into(),
            ..Default::default()
        };
        let show = c[1].to_string();
        return episode(p, dirs, season_at, &show, stem);
    }
    if let Some(c) = AIR_DATE.captures(stem) {
        let m = c.get(1).unwrap();
        let p = Parsed {
            air_date: Some(format!("{}-{}-{}", &c[1], &c[2], &c[3])),
            signal: "date".into(),
            ..Default::default()
        };
        let rest = stem[c.get(0).unwrap().end()..].to_string();
        let mut p = episode(p, dirs, season_at, &stem[..m.start()], stem);
        p.title = episode_title(&rest);
        return p;
    }
    let (clean, year) = clean_title(stem);
    if let Some(c) = EPISODE_N.captures(stem).filter(|_| season_at.is_some() || year.is_none()) {
        let m = c.get(0).unwrap();
        let p = Parsed {
            season: folder_season.or(Some(1)),
            episode: c[1].parse().ok(),
            signal: "episode-word".into(),
            ..Default::default()
        };
        let mut p = episode(p, dirs, season_at, &stem[..m.start()], stem);
        p.title = episode_title(&stem[m.end()..]);
        return p;
    }
    if season_at.is_some() {
        // In a season folder: a number to start the name (`03 - Title`), or `E03`.
        let n = LEADING_N
            .captures(stem)
            .or_else(|| E_N.captures(stem))
            .and_then(|c| c[1].parse::<i32>().ok());
        if let Some(n) = n {
            let p = Parsed {
                season: folder_season,
                episode: Some(n),
                signal: "season-folder".into(),
                title: clean.clone(),
                ..Default::default()
            };
            return episode(p, dirs, season_at, "", stem);
        }
        // A video in a season folder without a number: an episode Fix Match numbers.
        let p = Parsed {
            season: folder_season,
            title: clean.clone(),
            signal: "season-folder".into(),
            ..Default::default()
        };
        return episode(p, dirs, season_at, "", stem);
    }
    // Named alike but for a number, with two or more others in the folder.
    if let Some((prefix, n, show)) = numbered(stem) {
        let alike: std::collections::BTreeSet<i32> = siblings
            .iter()
            .filter_map(|s| numbered(stem_of(s)))
            .filter(|(p, _, _)| *p == prefix)
            .map(|(_, n, _)| n)
            .chain([n])
            .collect();
        if alike.len() >= 3 {
            let p = Parsed {
                season: Some(1),
                episode: Some(n),
                signal: "numbered".into(),
                weak: true,
                ..Default::default()
            };
            let mut p = episode(p, dirs, None, &show, stem);
            p.title = String::new();
            return p;
        }
    }
    parse_movie(dirs, stem)
}

/// A path read as an episode whatever its name says (Fix Match "This is an episode", an NFO, the
/// old import): its show from its folders.
pub fn as_episode(rel: &str) -> Parsed {
    let p = parse(rel);
    if p.kind == "episode" {
        return p;
    }
    let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
    let dirs = &parts[..parts.len().saturating_sub(1)];
    let stem = stem_of(parts.last().copied().unwrap_or(""));
    let season_at = dirs.iter().rposition(|d| season_dir(d).is_some());
    let mut e = episode(
        Parsed {
            signal: "manual".into(),
            ..Default::default()
        },
        dirs,
        season_at,
        "",
        stem,
    );
    e.title = clean_title(stem).0;
    if e.series_title.as_deref().is_none_or(str::is_empty) {
        e.series_title = Some(e.title.clone());
        e.series_folder = Some(dirs.iter().copied().chain([e.title.as_str()]).collect::<Vec<_>>().join("/"));
    }
    e
}

/// A path read as a movie whatever its name says.
pub fn as_movie(rel: &str) -> Parsed {
    let p = parse(rel);
    if p.kind == "movie" {
        return p;
    }
    let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
    let dirs = &parts[..parts.len().saturating_sub(1)];
    let mut m = parse_movie(dirs, stem_of(parts.last().copied().unwrap_or("")));
    m.signal = "manual".into();
    m
}

/// The file name without its extension.
pub fn stem_of(file: &str) -> &str {
    file.rsplit_once('.')
        .map_or(file, |(s, e)| if e.len() <= 4 && !e.contains(' ') { s } else { file })
}

/// An extra or a sample, by its folders or its name.
fn not_a_title(dirs: &[&str], stem: &str) -> Option<Parsed> {
    let lower = stem.to_lowercase();
    let extra_dir = dirs.iter().enumerate().any(|(i, d)| {
        let d = d.to_lowercase();
        EXTRA_DIRS.contains(&d.as_str()) || (i > 0 && EXTRA_DIRS_NESTED.contains(&d.as_str()))
    });
    if extra_dir || EXTRA_SUFFIXES.iter().any(|s| lower.ends_with(s)) || lower == "trailer" {
        return Some(Parsed {
            kind: "extra".into(),
            title: clean_title(stem).0,
            ..Default::default()
        });
    }
    if dirs
        .iter()
        .any(|d| d.eq_ignore_ascii_case("sample") || d.eq_ignore_ascii_case("samples"))
        || SAMPLE.is_match(stem)
    {
        return Some(Parsed {
            kind: "sample".into(),
            title: clean_title(stem).0,
            ..Default::default()
        });
    }
    None
}

/// A season folder's number (`Specials` = 0).
pub fn season_dir(d: &str) -> Option<i32> {
    if d.eq_ignore_ascii_case("specials") || d.eq_ignore_ascii_case("special") {
        return Some(0);
    }
    SEASON_DIR
        .captures(d.trim())
        .and_then(|c| c.get(1).or(c.get(2)))
        .and_then(|m| m.as_str().parse().ok())
}

/// A folder that only sorts titles ("TV Shows").
pub fn is_category_dir(d: &str) -> bool {
    CATEGORY_DIRS.contains(&d.trim().to_lowercase().as_str())
}

/// A release folder of one episode or one season (`Show.S01E02.1080p`, `Show.S02.WEB`).
fn release_dir(d: &str) -> bool {
    SXXEYY.is_match(d) || NXNN.captures(d).is_some_and(|c| !matches!(&c[2], "264" | "265")) || SEASON_PACK.is_match(d)
}

/// A name numbered within a set (`Show - 03`, `Part 3`, `03 - Title`): (the rest, normalised;
/// the number; the rest as written). Not for names with a year (sequels are named so).
fn numbered(stem: &str) -> Option<(String, i32, String)> {
    let (title, year) = clean_title(stem);
    if year.is_some() {
        return None;
    }
    let words: Vec<&str> = title.split_whitespace().collect();
    let num = |w: &str| -> Option<i32> {
        let w = w.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        (!w.is_empty() && w.len() <= 4 && w.chars().all(|c| c.is_ascii_digit()))
            .then(|| w.parse().ok())
            .flatten()
    };
    let at = if words.first().is_some_and(|w| num(w).is_some()) {
        0
    } else {
        words.iter().rposition(|w| num(w).is_some())?
    };
    let n = num(words[at])?;
    let mut rest: Vec<&str> = words[..at].iter().copied().filter(|w| !w.chars().all(|c| c == '-')).collect();
    // "Part 3", "Episode 3": the word goes with the number, not the show's name.
    if rest
        .last()
        .is_some_and(|w| ["part", "pt", "chapter", "vol", "volume", "episode", "ep", "e"].contains(&w.to_lowercase().trim_end_matches('.')))
    {
        rest.pop();
    }
    let show = rest.join(" ");
    Some((super::util::normalize(&show), n, show))
}

/// An episode's show: its season folder's parent, else the nearest folder (above release
/// folders) named like the show in the file name (`named`: the name's text before the
/// numbers), else that name.
fn episode(mut p: Parsed, dirs: &[&str], season_at: Option<usize>, named: &str, stem: &str) -> Parsed {
    p.kind = "episode".into();
    if p.season.is_none() {
        p.season = season_at.and_then(|i| season_dir(dirs[i]));
    }
    p.part = PART.captures(stem).and_then(|c| c[1].parse().ok());
    let (file_show, file_year) = clean_title(named.trim_matches(|c: char| c == '-' || c == '.' || c == '_' || c.is_whitespace()));
    let file_show = file_show.trim_end_matches(|c: char| c == '-' || c.is_whitespace()).to_string();
    // The folders that can be the show's: above the season folder, else all, without release folders.
    let mut end = season_at.unwrap_or(dirs.len());
    while end > 0 && season_at.is_none() && release_dir(dirs[end - 1]) {
        end -= 1;
    }
    let candidate = (end > 0).then(|| dirs[end - 1]).filter(|d| !is_category_dir(d));
    let take_folder = match candidate {
        Some(d) => season_at.is_some() || file_show.is_empty() || similar_show(&clean_title(d).0, &file_show),
        None => false,
    };
    if take_folder {
        let d = candidate.unwrap();
        let (t, y) = clean_title(d);
        p.series_folder = Some(dirs[..end].join("/"));
        p.series_title = Some(if t.is_empty() { d.to_string() } else { t });
        p.series_year = y.or(file_year);
    } else {
        let show = if file_show.is_empty() {
            candidate.or(dirs.last().copied()).map(|d| clean_title(d).0).unwrap_or_default()
        } else {
            file_show
        };
        let base = dirs[..end].join("/");
        p.series_folder = Some(if base.is_empty() { show.clone() } else { format!("{base}/{show}") });
        p.series_title = Some(show);
        p.series_year = file_year;
    }
    p
}

/// Whether a folder's title is the show named in a file.
fn similar_show(folder: &str, file_show: &str) -> bool {
    let bare = |s: &str| {
        let n = super::util::normalize(s);
        ["the ", "a ", "an "]
            .iter()
            .find_map(|a| n.strip_prefix(a))
            .map(String::from)
            .unwrap_or(n)
    };
    let (a, b) = (bare(folder), bare(file_show));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    a == b || a.starts_with(&format!("{b} ")) || b.starts_with(&format!("{a} ")) || super::util::similarity(folder, file_show) >= 0.75
}

/// The text after SxxEyy: an episode title, up to the first release tag.
fn episode_title(rest: &str) -> String {
    let (t, _) = clean_title(rest.trim_start_matches(|c: char| !c.is_alphanumeric()));
    t
}

fn parse_movie(dirs: &[&str], stem: &str) -> Parsed {
    let mut p = Parsed {
        kind: "movie".into(),
        signal: "movie".into(),
        ..Default::default()
    };
    // Readings, best first: one with a year from the file, then from the nearest folders.
    let mut readings: Vec<(String, Option<i32>)> = Vec::new();
    let push = |s: &str, readings: &mut Vec<(String, Option<i32>)>| {
        let (t, y) = clean_title(s);
        for alt in split_scripts(&t) {
            if !alt.is_empty() && !readings.iter().any(|(r, ry)| r.eq_ignore_ascii_case(&alt) && *ry == y) {
                readings.push((alt, y));
            }
        }
    };
    push(stem, &mut readings);
    for d in dirs.iter().rev().take(2).filter(|d| !is_category_dir(d)) {
        push(d, &mut readings);
    }
    // Prefer a reading with a year (and, among those, the first); a bare collection folder
    // ("Batman") only as a last resort.
    readings.sort_by_key(|(t, y)| (y.is_none(), t.split_whitespace().count() < 2 && y.is_none()));
    let mut it = readings.into_iter();
    if let Some((t, y)) = it.next() {
        p.title = t;
        p.year = y;
    }
    p.alt_titles = it.collect();
    p.part = PART.captures(stem).and_then(|c| c[1].parse().ok());
    p
}

/// A title in Cyrillic and Latin ("Апокалипсис сегодня Apocalypse Now", "Иллюзия обмана 2
/// (Now You See Me 2)"): each script's part on its own, the whole first.
fn split_scripts(t: &str) -> Vec<String> {
    let mut out = vec![t.to_string()];
    let inner: Vec<&str> = t.split(['(', ')']).map(str::trim).filter(|s| !s.is_empty()).collect();
    if inner.len() > 1 {
        out.extend(inner.iter().map(ToString::to_string));
    }
    let cyr = |w: &str| w.chars().any(|c| ('\u{400}'..='\u{4ff}').contains(&c));
    let words: Vec<&str> = t.split_whitespace().collect();
    if words.iter().any(|w| cyr(w)) && words.iter().any(|w| w.chars().any(|c| c.is_ascii_alphabetic())) {
        let c: Vec<&str> = words.iter().copied().filter(|w| cyr(w)).collect();
        let l: Vec<&str> = words
            .iter()
            .copied()
            .filter(|w| !cyr(w) && w.chars().any(|c| c.is_ascii_alphabetic()))
            .collect();
        out.push(c.join(" "));
        out.push(l.join(" ").trim_matches(['(', ')']).to_string());
    }
    out.retain(|s| !s.trim().is_empty());
    out.dedup();
    out
}

/// A release-style name → (title, year). The year is the last 19xx/20xx word that isn't the
/// first word; the title is what comes before it, or before the first release tag.
pub fn clean_title(name: &str) -> (String, Option<i32>) {
    let name = SITE_PREFIX.replace(name.trim(), "");
    let name = COLLECTION_NO.replace(&name, "");
    // Separators: dots and underscores between words; keep "Mr." style dots inside words rare.
    let spaced: String = name.chars().map(|c| if c == '.' || c == '_' { ' ' } else { c }).collect();
    // Brackets start a tag block unless they hold the year.
    let mut words: Vec<String> = Vec::new();
    for raw in spaced.split_whitespace() {
        words.push(raw.to_string());
    }
    let year_at = words
        .iter()
        .enumerate()
        .rev()
        .find(|(i, w)| *i > 0 && YEAR.is_match(w) && !(words[i - 1] == "-" && *i == 1))
        .map(|(i, w)| (i, YEAR.captures(w).unwrap()[1].parse::<i32>().unwrap()));
    let end = match year_at {
        Some((i, _)) => i,
        None => words.iter().position(|w| is_tag(w)).unwrap_or(words.len()),
    };
    let mut title_words: Vec<&str> = words[..end].iter().map(String::as_str).collect();
    // A year range "(1977-2015)" or a tag before the year ends the title too.
    if let Some(i) = title_words.iter().position(|w| is_tag(w) || w.starts_with('[')) {
        if i > 0 {
            title_words.truncate(i);
        }
    }
    let mut title = title_words.join(" ");
    title = title
        .trim_matches(|c: char| c == '-' || c == ' ' || c == '[' || c == '(' || c == ',')
        .to_string();
    // "Thor - The Dark World" style separators are kept; a trailing " -" isn't.
    while title.ends_with(" -") {
        title.truncate(title.len() - 2);
    }
    if title.ends_with('(') {
        title.pop();
    }
    (title.trim().to_string(), year_at.map(|(_, y)| y))
}

fn is_tag(w: &str) -> bool {
    let l = w
        .trim_matches(|c: char| c == '[' || c == ']' || c == '(' || c == ')')
        .to_lowercase();
    TAGS.contains(&l.as_str()) || w.starts_with('[') || l.starts_with("bdrip") || l.ends_with("rip") && l.len() > 5
}

pub fn is_video(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, e)| VIDEO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie(rel: &str) -> (String, Option<i32>) {
        let p = parse(rel);
        assert_eq!(p.kind, "movie", "{rel}");
        (p.title, p.year)
    }

    fn ep(rel: &str) -> (Option<i32>, Option<i32>, Option<i32>) {
        let p = parse(rel);
        assert_eq!(p.kind, "episode", "{rel}");
        (p.season, p.episode, p.episode_end)
    }

    #[test]
    fn movies() {
        let cases = [
            ("Puss in Boots 720p.mkv", "Puss in Boots", None),
            ("Kong.Skull.Island.Bdrip.1080p.by.AKTEP.mkv", "Kong Skull Island", None),
            ("Operation.Fortune.Ruse.de.guerre.2023.1080p.WEB.H264-NAISU[TGx]/operation.fortune.ruse.de.guerre.2023.1080p.web.h264-naisu.mkv", "operation fortune ruse de guerre", Some(2023)),
            ("The Electric State (2025) [1080p] [WEBRip] [x265] [10bit] [5.1] [YTS.MX]/The.Electric.State.2025.1080p.WEBRip.x265.10bit.AAC5.1-[YTS.MX].mp4", "The Electric State", Some(2025)),
            ("Thor - The Dark World (2013) (1080p BluRay x265 HEVC 10bit AAC 7.1 Tigole)/Thor - The Dark World (2013) (1080p BluRay x265 10bit Tigole).mkv", "Thor - The Dark World", Some(2013)),
            ("Marvel/Avengers.Age.of.Ultron.2015.BDRip.1080p.BLUEBIRD.mkv", "Avengers Age of Ultron", Some(2015)),
            ("Zootopia (2016).mkv", "Zootopia", Some(2016)),
            ("Constantine.mkv", "Constantine", None),
            ("Home Alone 2 1992.mkv", "Home Alone 2", Some(1992)),
            ("Taxi.2.2000.BDRip.1080p-rutracker.mkv", "Taxi 2", Some(2000)),
            ("www.UIndex.org    -    Mafia 1998 1080p BluRay Remux AVC DTS-HD MA 5 1-PmP/Mafia 1998 1080p BluRay Remux AVC DTS-HD MA 5 1-PmP.mkv", "Mafia", Some(1998)),
            ("Moana_2016_BDRip_iT_by_Dalemake.avi", "Moana", Some(2016)),
            ("Babylon.2022.WEB-DL.1080p_от New-Team_JNS82.mkv", "Babylon", Some(2022)),
            ("Django Unchained (2012)  [1080p x265 q18 FS100 Joy].mkv", "Django Unchained", Some(2012)),
            ("The.Croods.WEB-DL.KION.1080p-SOFCJ.mkv", "The Croods", None),
            ("Star Wars (1977-2015) BDRip/Star Wars Episode IV - A New Hope (1977) BDRip.mkv", "Star Wars Episode IV - A New Hope", Some(1977)),
            ("Битва за битвой (2025) WEB-DLRip-AVC_ivanes.mkv", "Битва за битвой", Some(2025)),
            ("Harry Potter/7.2 Harry Potter and the Deathly Hallows. Part II/Harry.Potter.and.the.Deathly.Hallows.Part.2.2011.D.BDRip.720p.mkv", "Harry Potter and the Deathly Hallows Part 2", Some(2011)),
            ("Сибирский цирюльник DVD-9_Custom", "Сибирский цирюльник DVD-9 Custom", None),
        ];
        for (rel, t, y) in cases {
            assert_eq!(movie(rel), (t.to_string(), y), "{rel}");
        }
    }

    #[test]
    fn movie_alternatives() {
        let p = parse("Harry Potter/2. Harry Potter and the Chamber of.Secrets/HP2.BDRip.720p.DVD9.mkv");
        assert_eq!(p.title, "Harry Potter and the Chamber of Secrets", "{p:?}");
        let p = parse("Иллюзия обмана 2 (Now You See Me 2) 2016.mkv");
        assert_eq!(p.year, Some(2016));
        assert!(p.alt_titles.iter().any(|(t, _)| t == "Now You See Me 2"), "{p:?}");
        let p = parse("Апокалипсис.сегодня.Apocalypse.Now.1979.Redux.BDRip-HEVC.1080p.mkv");
        assert!(p.alt_titles.iter().any(|(t, _)| t == "Apocalypse Now"), "{p:?}");
        // A bare collection folder doesn't win over the file's own name.
        let p = parse("Batman/The.Dark.Knight.2008.720p.BluRay.DD5.1.x264.6xRus.Eng-CtrlHD.mkv");
        assert_eq!((p.title.as_str(), p.year), ("The Dark Knight", Some(2008)));
    }

    #[test]
    fn episodes() {
        assert_eq!(
            ep("The Office (2005)/Season 4/The Office (US) (2005) - S04E18-E19 - Goodbye, Toby (1080p BluRay x265 Silence).mkv"),
            (Some(4), Some(18), Some(19))
        );
        assert_eq!(
            ep("Avatar - The Last Airbender (2005)/Season 3/Avatar.The.Last.Airbender.S03E18E19E20E21.Sozins.Comet.1080p.mkv"),
            (Some(3), Some(18), Some(21))
        );
        assert_eq!(
            ep("Battlestar Galactica (2004)/Season 1/BSG S01E01 33 (1080p x265 10bit Joy).mkv"),
            (Some(1), Some(1), None)
        );
        assert_eq!(
            ep("The X-Files (1993)/Season 3/The X-Files S03E10 731 (2).mkv"),
            (Some(3), Some(10), None)
        );
        assert_eq!(
            ep("The X-Files (1993)/Season 9/The X-Files S09E04 4-D.mkv"),
            (Some(9), Some(4), None)
        );
        assert_eq!(
            ep("Agatha All Along (2024)/Season 1/Agatha All Along S01E01 1080p x265-ELiTE[EZTVx.to].mkv"),
            (Some(1), Some(1), None)
        );
        assert_eq!(
            ep("The A-Team (1983)/Season 1/The A-Team.1x14.A.Nice.Place.To.Visit.DVDRip.avi"),
            (Some(1), Some(14), None)
        );
        assert_eq!(
            ep("The A-Team (1983)/Season 4/The A-Team.4x01-02.Judgment.Day.DVDRip.avi"),
            (Some(4), Some(1), Some(2))
        );
        assert_eq!(
            ep("ALF (1986)/Season 1/Alf.s01e16-17. Try to Remember.mkv"),
            (Some(1), Some(16), Some(17))
        );
        assert_eq!(ep("Sherlock (2010)/Season 0/Sherlock.S00E09.mkv"), (Some(0), Some(9), None));
        assert_eq!(
            ep("Family Guy (1999)/Season 24/Family Guy - S24E17 - 2160p HDR Ai Upscale -Mesc/Family Guy - S24E17 - 2160p HDR Ai Upscale -Mesc.mkv"),
            (Some(24), Some(17), None)
        );
        assert_eq!(ep("Long Way Round (2004)/Season 1/Episode 3.mkv"), (Some(1), Some(3), None));
        let p = parse("The Office (2005)/Season 4/The Office (US) (2005) - S04E15 - Night Out (1080p BluRay x265 Silence).mkv");
        assert_eq!(p.series_title.as_deref(), Some("The Office"));
        assert_eq!(p.series_year, Some(2005));
        assert_eq!(p.title, "Night Out");
        let p = parse("Caprica (2009)/Season 1/Caprica S01E01 Pilot Pt.2 (1080p Web 1080p Joy).mkv");
        assert_eq!(p.part, Some(2));
    }

    /// One folder with movies, show folders and loose episodes side by side.
    #[test]
    fn mixed_folder() {
        let all = [
            "Inception (2010)/Inception.2010.1080p.mkv",
            "Fargo (1996)/Fargo.1996.mkv",
            "Fargo (2014)/Season 1/Fargo.S01E01.mkv",
            "Breaking.Bad.S01E01.720p.mkv",
            "TV/The Wire/The.Wire.S01E02.mkv",
            "TV/Show.Name.S02E03.Title.mkv",
            "Chernobyl (2019)/Chernobyl.Part.1.mkv",
            "Chernobyl (2019)/Chernobyl.Part.2.mkv",
            "Chernobyl (2019)/Chernobyl.Part.3.mkv",
            "1917 (2019)/1917.2019.1080p.mkv",
            "2012.mkv",
            "[SubsPlease] Sousou no Frieren - 12 (1080p) [ABCD1234].mkv",
            "One Piece/One Piece - 1071 [1080p].mkv",
            "One Piece/One Piece - 1072 [1080p].mkv",
            "One Piece/One Piece - 1073 [1080p].mkv",
            "Daily Show/The.Daily.Show.2024.03.15.Jon.Stewart.mkv",
            "Show.Name.S01.1080p.WEB/Show.Name.S01E01.1080p.mkv",
            "Star.Wars.Episode.1.The.Phantom.Menace.1999.mkv",
            "Kill Bill/Kill.Bill.Vol.1.2003.mkv",
            "Kill Bill/Kill.Bill.Vol.2.2004.mkv",
            "Doctor Who/Specials/Doctor.Who.The.Day.of.the.Doctor.mkv",
            "The Expanse/S02/03 - Static.mkv",
            "Planet Earth (2006)/Planet Earth - 01 - From Pole to Pole.mkv",
            "Planet Earth (2006)/Planet Earth - 02 - Mountains.mkv",
            "Planet Earth (2006)/Planet Earth - 03 - Fresh Water.mkv",
        ];
        let at = |rel: &str| {
            let (d, name) = rel.rsplit_once('/').unwrap_or(("", rel));
            let sib: Vec<&str> = all
                .iter()
                .filter(|p| p.rsplit_once('/').map_or("", |x| x.0) == d)
                .map(|p| p.rsplit('/').next().unwrap())
                .filter(|n| *n != name)
                .collect();
            classify(rel, &sib)
        };
        let show = |rel: &str| {
            let p = at(rel);
            assert_eq!(p.kind, "episode", "{rel}: {p:?}");
            (
                p.series_folder.unwrap_or_default(),
                p.series_title.unwrap_or_default(),
                p.season,
                p.episode,
            )
        };
        let movie = |rel: &str| {
            let p = at(rel);
            assert_eq!(p.kind, "movie", "{rel}: {p:?}");
            (p.title, p.year)
        };
        assert_eq!(movie(all[0]), ("Inception".into(), Some(2010)));
        assert_eq!(movie(all[1]), ("Fargo".into(), Some(1996)));
        assert_eq!(show(all[2]), ("Fargo (2014)".into(), "Fargo".into(), Some(1), Some(1)));
        assert_eq!(show(all[3]), ("Breaking Bad".into(), "Breaking Bad".into(), Some(1), Some(1)));
        assert_eq!(show(all[4]), ("TV/The Wire".into(), "The Wire".into(), Some(1), Some(2)));
        assert_eq!(show(all[5]), ("TV/Show Name".into(), "Show Name".into(), Some(2), Some(3)));
        assert_eq!(show(all[7]), ("Chernobyl (2019)".into(), "Chernobyl".into(), Some(1), Some(2)));
        assert!(at(all[7]).weak, "numbered names are a weak signal");
        assert_eq!(movie(all[9]), ("1917".into(), Some(2019)));
        assert_eq!(movie(all[10]), ("2012".into(), None));
        assert_eq!(
            show(all[11]),
            ("Sousou no Frieren".into(), "Sousou no Frieren".into(), None, Some(12))
        );
        assert_eq!(show(all[13]), ("One Piece".into(), "One Piece".into(), Some(1), Some(1072)));
        let d = at(all[15]);
        assert_eq!(
            (d.kind.as_str(), d.air_date.as_deref(), d.series_folder.as_deref()),
            ("episode", Some("2024-03-15"), Some("Daily Show"))
        );
        assert_eq!(show(all[16]), ("Show Name".into(), "Show Name".into(), Some(1), Some(1)));
        assert_eq!(movie(all[17]).1, Some(1999));
        assert_eq!(movie(all[18]), ("Kill Bill Vol 1".into(), Some(2003)));
        assert_eq!(show(all[20]), ("Doctor Who".into(), "Doctor Who".into(), Some(0), None));
        assert_eq!(show(all[21]), ("The Expanse".into(), "The Expanse".into(), Some(2), Some(3)));
        assert_eq!(
            show(all[23]),
            ("Planet Earth (2006)".into(), "Planet Earth".into(), Some(1), Some(2))
        );
        // Extras by folder or name; "Other" at the top is a collection, below a title extras.
        assert_eq!(parse("Movie (2020)/Movie-trailer.mkv").kind, "extra");
        assert_eq!(parse("Other/Some.Movie.2001.mkv").kind, "movie");
        assert_eq!(parse("Movie (2020)/Other/Bloopers.mkv").kind, "extra");
        // Not an episode: a codec after an audio layout ("DD5.1x264").
        assert_eq!(
            parse("Guardians.of.the.Galaxy.2014.IMAX.Edition.720p.BluRay.DD5.1x264-Talian-.mkv").kind,
            "movie"
        );
    }

    #[test]
    fn not_titles() {
        assert_eq!(parse("Friends (1994)/Season 1/Featurettes/Making of.mkv").kind, "extra");
        assert_eq!(
            parse("The X-Files (1998)/Sample/The X-Files - Fight the Future Sample.mp4").kind,
            "sample"
        );
        assert_eq!(
            parse("Prospect.2018.1080p.BluRay.x264-VETO[EtHD]/Sample/prospect.2018.1080p.bluray.x264-veto-sample.mkv").kind,
            "sample"
        );
        assert!(is_video("a/b.MKV") && !is_video("a/b.srt"));
    }
}

#[cfg(test)]
mod real_paths {
    /// `MOONRISE_LISTING=<file>`: a library's files, one per line `library\trel\trole\tkind\tkey\tseason\tepisode\tend`
    /// (role main|extra|sample|broken, kind movie|episode as identified). Every path is classified
    /// type-blind (as one mixed library: `library/rel`, with the other videos of its folder) and
    /// compared: kind, and an episode's numbers.
    #[test]
    #[ignore = "needs a listing file in MOONRISE_LISTING"]
    fn against_listing() {
        let Ok(path) = std::env::var("MOONRISE_LISTING") else { return };
        let text = std::fs::read_to_string(path).unwrap();
        let rows: Vec<Vec<&str>> = text
            .lines()
            .map(|l| l.split('\t').collect())
            .filter(|f: &Vec<&str>| f.len() >= 8)
            .collect();
        let prefix = |lib: &str| {
            if lib == "movies" {
                "movies".to_string()
            } else {
                "tv shows".to_string()
            }
        };
        let full: Vec<String> = rows.iter().map(|f| format!("{}/{}", prefix(f[0]), f[1])).collect();
        let mut by_dir: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
        for (i, p) in full.iter().enumerate() {
            if rows[i][2] == "main" || rows[i][2] == "broken" {
                let (d, f) = p.rsplit_once('/').unwrap_or(("", p));
                by_dir.entry(d).or_default().push(f);
            }
        }
        let (mut n, mut bad) = (0, 0);
        let mut counts: std::collections::BTreeMap<String, i32> = std::collections::BTreeMap::new();
        for (i, f) in rows.iter().enumerate() {
            let rel = full[i].as_str();
            let (d, name) = rel.rsplit_once('/').unwrap_or(("", rel));
            let sib: Vec<&str> = by_dir
                .get(d)
                .map(|v| v.iter().copied().filter(|s| *s != name).collect())
                .unwrap_or_default();
            let p = super::classify(rel, &sib);
            n += 1;
            let num = |s: &str| s.parse::<i32>().ok();
            let ok = match (f[2], f[3]) {
                ("main" | "broken", "movie") => p.kind == "movie",
                ("main" | "broken", "episode") => {
                    p.kind == "episode" && ((p.season, p.episode) == (num(f[5]), num(f[6])) || f[4].contains(":s"))
                }
                ("main" | "broken", _) => p.kind == "movie" || p.kind == "episode",
                (role, _) => p.kind == role,
            };
            *counts
                .entry(format!(
                    "{}->{}{}",
                    if f[3].is_empty() { f[2] } else { f[3] },
                    p.kind,
                    if ok { "" } else { " (BAD)" }
                ))
                .or_default() += 1;
            if !ok {
                bad += 1;
                println!(
                    "MISMATCH {rel} {:?} -> {} S{:?}E{:?} [{}] series={:?}",
                    &f[2..8],
                    p.kind,
                    p.season,
                    p.episode,
                    p.signal,
                    p.series_folder
                );
            }
        }
        // Shows: one folder per show, and the reverse (column 9: the show's key).
        let mut folders: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> = std::collections::BTreeMap::new();
        let mut keys: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> = std::collections::BTreeMap::new();
        for (i, f) in rows.iter().enumerate() {
            if f[3] == "episode" && f.len() > 8 {
                let rel = full[i].as_str();
                let (d, name) = rel.rsplit_once('/').unwrap_or(("", rel));
                let sib: Vec<&str> = by_dir
                    .get(d)
                    .map(|v| v.iter().copied().filter(|s| *s != name).collect())
                    .unwrap_or_default();
                let p = super::classify(rel, &sib);
                let folder = p.series_folder.unwrap_or_default();
                folders.entry(folder.clone()).or_default().insert(f[8].to_string());
                keys.entry(f[8].to_string()).or_default().insert(folder);
            }
        }
        println!("{} show folders, {} shows", folders.len(), keys.len());
        for (k, v) in folders.iter().filter(|(_, v)| v.len() > 1) {
            println!("FOLDER {k} holds {v:?}");
        }
        for (k, v) in keys.iter().filter(|(_, v)| v.len() > 1) {
            println!("SHOW {k} in {v:?}");
        }
        println!("{counts:#?}");
        println!("{n} paths, {bad} mismatches");
    }
}
