//! What each file is: a movie, or an episode of a series, keyed by TMDB id.
//!
//! Libraries have no type: a file is a movie or an episode by `classify` (Fix Match's choice,
//! else the old import's, else its name and folder as `parse::classify` reads them, else an
//! NFO). An episode numbered only weakly (names alike but for a number) becomes a movie when its
//! show isn't on TMDB but a movie of its name is.
//!
//! In order, the first that answers wins (`items.identify_state`):
//! 1. `manual`: set through `POST /v1/admin/items/{id}/identify`; never overridden.
//! 2. `bootstrap`: the match hints stored once for the file's path (or its series folder)
//!    (`match_hints`, from a one-time import in an early version; nothing new is added to it).
//! 3. `matched`: TMDB search by the parsed title and year, scored on title similarity
//!    (transliteration-aware), year and runtime; accepted at ≥ 0.85 with a lead of 0.15.
//! 4. `llm`: an LLM picks among the candidates (Appendix A), checked deterministically
//!    (a candidate offered, year ±1, runtime ±15 %, title similarity ≥ 0.5, the episode exists).
//! 5. `review`: none of these; browsable under its parsed name, listed by
//!    `GET /v1/admin/review`.
//!
//! Items are keyed by identity (`tmdb:movie:603`, `tmdb:tv:1398:s4e3`, or `path:…`), so two
//! files of one title are two versions of one item, and a renamed file keeps its item.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{
    parse::Parsed,
    util::{self, now, similarity},
    Library,
};

/// Bump when the LLM prompt or schema changes (cached verdicts are keyed by it).
const PROMPT_VERSION: i64 = 1;
const ACCEPT_SCORE: f64 = 0.85;
const ACCEPT_LEAD: f64 = 0.15;

#[derive(Debug, Clone)]
pub struct FileRow {
    pub id: String,
    pub library_id: String,
    pub path: String,
    pub rel: String,
    pub duration: Option<f64>,
    pub oshash: Option<String>,
    pub parsed: Parsed,
    pub manual_key: Option<String>,
    pub mtime: i64,
}

/// A TMDB search result, as scored and as shown to the LLM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: i64,
    pub title: String,
    pub original_title: String,
    pub year: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_minutes: Option<i64>,
    pub overview: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seasons: Option<i64>,
    #[serde(skip)]
    pub score: f64,
    #[serde(skip)]
    pub popularity: f64,
    /// Its title is exactly one of the file's own readings, with a year that fits (or, without
    /// a year, a runtime that does: movies).
    #[serde(skip)]
    pub fits: bool,
    /// TMDB's poster path (for Fix Match).
    #[serde(skip)]
    pub poster: Option<String>,
}

/// How a title was identified.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ident {
    /// `tmdb:movie:603`, `tmdb:tv:1398`, or `path:…`
    pub key: String,
    pub state: &'static str,
    pub source: String,
    pub confidence: f64,
    pub note: Option<String>,
    /// The TMDB candidates considered, best first (kept for review: Fix Match offers them).
    pub candidates: Vec<Candidate>,
}

impl Ident {
    pub fn tmdb_id(&self) -> Option<i64> {
        self.key
            .rsplit(':')
            .next()
            .and_then(|s| s.parse().ok())
            .filter(|_| self.key.starts_with("tmdb:"))
    }
}

/// Folder path → the file names of the title videos in it.
pub type Siblings = HashMap<String, Vec<String>>;

/// The stored match hint for a path (`match_hints`).
#[derive(Debug, Clone, Default)]
pub struct Boot {
    /// `Movie` or `Episode` (files).
    pub kind: Option<String>,
    pub tmdb: Option<i64>,
    pub imdb: Option<String>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub episode_end: Option<i32>,
    pub date_created: Option<i64>,
    pub name: Option<String>,
}

fn year_of(s: &Value) -> Option<i32> {
    s.as_str().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok())
}

fn year_score(parsed: Option<i32>, found: Option<i32>) -> f64 {
    match (parsed, found) {
        (Some(p), Some(f)) if p == f => 1.0,
        (Some(p), Some(f)) if (p - f).abs() == 1 => 0.6,
        (Some(_), _) => 0.0,
        (None, _) => 0.5,
    }
}

/// Readings past the first `strong` count less (a collection folder's name, a fragment).
const WEAK_READING: f64 = 0.9;

fn score(readings: &[(String, Option<i32>)], strong: usize, c: &Candidate) -> f64 {
    score_titles(readings, strong, &[c.title.as_str(), c.original_title.as_str()], c.year)
}

fn score_titles(readings: &[(String, Option<i32>)], strong: usize, titles: &[&str], year: Option<i32>) -> f64 {
    readings
        .iter()
        .enumerate()
        .map(|(i, (t, y))| {
            let s = 0.65 * titles.iter().map(|a| similarity(t, a)).fold(0.0, f64::max) + 0.35 * year_score(*y, year);
            if i < strong {
                s
            } else {
                s * WEAK_READING
            }
        })
        .fold(0.0, f64::max)
}

/// Edition words that aren't part of a title ("Troy Director's Cut", "Avatar Extended").
static EDITION: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)\b(?:director'?s\s+cut|extended(?:\s+(?:edition|cut|version))?|alt(?:ernative)?\s+cut|collector'?s\s+(?:cut|edition)|theatrical(?:\s+cut)?|final\s+cut|ultimate\s+(?:cut|edition)|special\s+edition|unrated|uncut|redux|remastered|imax)\b",
    )
    .unwrap()
});
/// A year written as Russian names do ("2001г", "2001 г.").
static YEAR_G: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"[(\[]?\b((?:19|20)\d\d)\s?г\.?").unwrap());

/// The readings of a movie's name, best first, and how many of them are its own (strong): the
/// parsed title; it without edition words, bracketed parts, or before a " - "/": " subtitle
/// only after it ("Star Wars Episode V - The Empire Strikes Back"); then the parser's other
/// readings (those with the title's year are strong too: a second script of the same name).
///
/// `own_folder`: the name of the folder the file is in when that folder holds this title alone
/// (its name is the title's too; a collection's folder isn't).
pub fn movie_readings(p: &super::parse::Parsed, own_folder: Option<&str>) -> (Vec<(String, Option<i32>)>, usize) {
    let mut title = p.title.clone();
    let mut year = p.year;
    if year.is_none() {
        if let Some(c) = YEAR_G.captures(&title) {
            year = c[1].parse().ok();
            title = YEAR_G.replace(&title, " ").to_string();
        }
    }
    let clean = |t: &str| {
        t.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .trim_matches(|c: char| c == '-' || c == ':' || c.is_whitespace())
            .to_string()
    };
    let mut strong: Vec<(String, Option<i32>)> = Vec::new();
    let push = |v: &mut Vec<(String, Option<i32>)>, t: String| {
        let t = clean(&t);
        if !t.is_empty() && !v.iter().any(|(x, _)| util::normalize(x) == util::normalize(&t)) {
            v.push((t, year));
        }
    };
    push(&mut strong, title.clone());
    let no_edition = EDITION.replace_all(&title, " ").to_string();
    push(&mut strong, no_edition.clone());
    let no_brackets: String = {
        let mut out = String::new();
        let mut depth = 0;
        for ch in no_edition.chars() {
            match ch {
                '(' | '[' => depth += 1,
                ')' | ']' if depth > 0 => depth -= 1,
                _ if depth == 0 => out.push(ch),
                _ => {}
            }
        }
        out
    };
    push(&mut strong, no_brackets.clone());
    if let Some((_, tail)) = no_brackets.split_once(" - ").or_else(|| no_brackets.split_once(": ")) {
        if tail.split_whitespace().count() >= 2 {
            push(&mut strong, tail.to_string());
        }
    }
    let folder = own_folder.map(util::normalize).unwrap_or_default();
    for (t, y) in &p.alt_titles {
        let own = !folder.is_empty() && !util::normalize(t).is_empty() && folder.starts_with(&util::normalize(t));
        if (y.is_some() && *y == year) || (own && (y.is_none() || *y == year)) {
            push(&mut strong, t.clone());
        }
    }
    let n = strong.len();
    let mut all = strong;
    for (t, y) in &p.alt_titles {
        let t = clean(t);
        if !t.trim().is_empty() && !all.iter().any(|(x, _)| util::normalize(x) == util::normalize(&t)) {
            all.push((t, *y));
        }
    }
    (all, n)
}

/// Whether `title` is exactly (after normalising) one of the strong readings.
fn exact(readings: &[(String, Option<i32>)], strong: usize, titles: &[String]) -> Option<Option<i32>> {
    readings
        .iter()
        .take(strong)
        .find(|(t, _)| titles.iter().any(|a| util::normalize(a) == util::normalize(t)))
        .map(|(_, y)| *y)
}

/// The JSON schema of a verdict.
pub fn verdict_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "kind": {"type": "string", "enum": ["movie", "episode", "extra", "unknown"]},
            "tmdb_id": {"type": ["integer", "null"], "description": "One of the candidates' ids, or null."},
            "season": {"type": ["integer", "null"]},
            "episode": {"type": ["integer", "null"]},
            "episode_end": {"type": ["integer", "null"]},
            "suggested_title": {"type": ["string", "null"], "description": "When no candidate fits: the title to search TMDB for (in English if you know it)."},
            "suggested_year": {"type": ["integer", "null"]},
            "confidence": {"type": "number"},
            "reason": {"type": "string"}
        },
        "required": ["kind", "tmdb_id", "season", "episode", "episode_end", "suggested_title", "suggested_year", "confidence", "reason"],
        "additionalProperties": false
    })
}

pub const SYSTEM: &str = "You identify video files in a home media library. You get a file's path, what was parsed from \
it, technical details, and candidate titles from TMDB. Decide what the file is: kind \"movie\" (a feature film), \
\"episode\" (a TV episode), \"extra\" (bonus material, trailer, featurette, sample) or \"unknown\". tmdb_id must be one \
of the candidates' ids, or null when none fits; then give suggested_title/suggested_year to search for instead, if you \
can tell what it is (release names are often transliterated Russian or abbreviated). For episodes give season, episode \
and, for a file holding several episodes, episode_end. confidence is 0 to 1. reason is one short sentence.";

/// A manual key for the whole episode (`…:s4e23`, a part of it `…:s4e23:p2`), or for its
/// series; `split`: its own title, apart from the files numbered as it (its show found as usual).
/// Answers (split, the key without the part, the part).
fn manual_episode_key(f: &FileRow) -> (bool, Option<String>, Option<i32>) {
    let split = f.manual_key.as_deref() == Some("split");
    let (manual, manual_part) = match f.manual_key.as_deref() {
        Some("split") => (None, None),
        Some(k) => match k.rsplit_once(":p") {
            Some((b, n)) if b.contains(":s") && n.parse::<i32>().is_ok_and(|n| n >= 1) => (Some(b.to_string()), n.parse::<i32>().ok()),
            _ => (Some(k.to_string()), None),
        },
        None => (None, None),
    };
    (split, manual, manual_part)
}

/// The episode's title until TMDB's arrives: the old import's, the name's, or "Episode n".
fn episode_title(boot: Option<&Boot>, f: &FileRow, episode: Option<i32>) -> String {
    let mut ep_title = boot
        .and_then(|b| b.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| f.parsed.title.clone());
    if ep_title.is_empty() {
        if let Some(e) = episode {
            ep_title = format!("Episode {e}");
        }
    }
    ep_title
}

/// An identified episode, as stored: its series, season and episode items and the file's place.
struct EpisodeWrite {
    lib: String,
    fid: String,
    series: Ident,
    series_title: String,
    series_year: Option<i32>,
    season_key: String,
    season_no: i32,
    ident: Ident,
    episode: Option<i32>,
    episode_end: Option<i32>,
    ep_title: String,
    part_no: Option<i32>,
    added: i64,
    duration: Option<f64>,
}

/// Stores an identified episode: the series, season and episode items (made or updated), parts,
/// and the file assigned to its episode.
fn store_episode(c: &mut rusqlite::Connection, w: EpisodeWrite) -> Result<()> {
    let EpisodeWrite {
        lib,
        fid,
        series: series_ident,
        series_title,
        series_year,
        season_key,
        season_no,
        ident,
        episode,
        episode_end,
        ep_title,
        part_no,
        added,
        duration,
    } = w;
    let tx = c.transaction()?;
    let series_id = ensure_item(
        &tx,
        &ItemSpec {
            key: &series_ident.key,
            kind: "series",
            library_id: &lib,
            title: &series_title,
            year: series_year,
            added_at: added,
            ..Default::default()
        },
        &series_ident,
    )?;
    if series_ident.state != "manual" {
        tx.execute(
            "UPDATE items SET candidates = ?2 WHERE id = ?1",
            params![series_id, review_candidates(&series_ident)],
        )?;
    }
    let season_title = if season_no == 0 {
        "Specials".to_string()
    } else {
        format!("Season {season_no}")
    };
    let season_id = ensure_item(
        &tx,
        &ItemSpec {
            key: &season_key,
            kind: "season",
            library_id: &lib,
            parent_id: Some(&series_id),
            series_id: Some(&series_id),
            season_no: Some(season_no),
            title: &season_title,
            added_at: added,
            ..Default::default()
        },
        &series_ident,
    )?;
    let ep_id = ensure_item(
        &tx,
        &ItemSpec {
            key: &ident.key,
            kind: "episode",
            library_id: &lib,
            parent_id: Some(&season_id),
            series_id: Some(&series_id),
            season_no: Some(season_no),
            episode_no: episode,
            episode_end,
            title: &ep_title,
            added_at: added,
            part_no,
            revive_for: Some(&fid),
            ..Default::default()
        },
        &ident,
    )?;
    if let Some(p) = part_no {
        // A part (chosen by hand, or named so): the episode's other file is Part 1 (its
        // key stays); a part plays as long as its file.
        if let Some(base) = ident.key.rsplit_once(":p").map(|(b, _)| b.to_string()).filter(|_| p >= 2) {
            tx.execute("UPDATE items SET part_no = 1 WHERE key = ?1 AND part_no IS NULL", [&base])?;
        }
        tx.execute(
            "UPDATE items SET runtime = COALESCE(?2, runtime) WHERE id = ?1",
            params![ep_id, duration],
        )?;
    }
    assign(&tx, &fid, &ep_id)?;
    tx.commit()?;
    Ok(())
}

impl Library {
    /// Identifies the stable files that need it; returns how many were assigned.
    pub async fn identify_pending(&self) -> Result<usize> {
        let now = now();
        if self.tmdb().is_some() {
            // Titles named from their files while there was no TMDB key: look them up now.
            let n = self
                .db
                .call(|c| {
                    Ok(c.execute(
                        "UPDATE files SET identified_at = NULL WHERE item_id IN (SELECT id FROM items WHERE identify_state = 'local')",
                        [],
                    )?)
                })
                .await?;
            if n > 0 {
                tracing::info!(files = n, "TMDB is set up: identifying titles named from their files");
            }
        }
        // Once (0.20): episodes with several files are looked at again (parts named as such under
        // one number were taken for versions).
        self.db
            .call(|c| {
                if c.query_row("SELECT 1 FROM meta WHERE key = 'versions_checked'", [], |_| Ok(())).optional()?.is_none() {
                    let n = c.execute(
                        &format!(
                            "UPDATE files SET identified_at = NULL WHERE manual_key IS NULL AND item_id IN
                               (SELECT f.item_id FROM files f JOIN items i ON i.id = f.item_id WHERE i.kind = 'episode' AND f.role IN {r} AND f.missing_since IS NULL
                                GROUP BY f.item_id HAVING COUNT(*) > 1)",
                            r = super::TITLE_ROLES
                        ),
                        [],
                    )?;
                    c.execute("INSERT OR REPLACE INTO meta(key, value) VALUES ('versions_checked', ?1)", [util::now().to_string()])?;
                    if n > 0 {
                        tracing::info!(files = n, "episodes with several files: identifying them again (parts or versions)");
                    }
                }
                Ok(())
            })
            .await?;
        let files: Vec<FileRow> = self
            .file_rows("identified_at IS NULL")
            .await?
            .into_iter()
            .filter(|(r, stable_since, last_seen)| super::scanner::is_stable(now, *stable_since, *last_seen, r.mtime))
            .map(|(r, _, _)| r)
            .collect();
        if files.is_empty() {
            return Ok(0);
        }
        tracing::info!(files = files.len(), "identifying");
        self.identify_total.store(files.len(), std::sync::atomic::Ordering::Relaxed);
        let mut series_cache: HashMap<String, Ident> = HashMap::new();
        let idx = self.sibling_index().await?;
        let mut done = 0;
        for f in &files {
            let r = async {
                let mut f = f.clone();
                f.parsed = self.classify(&f, &idx, true).await;
                if self.prefers_movie(&f, &mut series_cache, false).await? {
                    f.parsed = super::parse::as_movie(&f.rel);
                    f.parsed.signal = "tmdb".into();
                }
                let (id, json) = (f.id.clone(), serde_json::to_string(&f.parsed)?);
                self.db
                    .call(move |c| Ok(c.execute("UPDATE files SET parsed = ?2 WHERE id = ?1", params![id, json])?))
                    .await?;
                if f.parsed.kind == "episode" {
                    self.identify_episode(&f, &mut series_cache).await
                } else {
                    self.identify_movie(&f).await
                }
            }
            .await;
            self.identify_done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            match r {
                Ok(()) => done += 1,
                Err(e) => tracing::warn!(path = f.path, "identify: {e:#}"),
            }
        }
        self.cleanup_items().await?;
        Ok(done)
    }

    /// The file names of the title videos in each folder (for numbered names).
    pub async fn sibling_index(&self) -> Result<Siblings> {
        self.db
            .call(|c| {
                let mut st = c.prepare("SELECT path FROM files WHERE missing_since IS NULL AND role IN ('main', 'broken')")?;
                let mut out: Siblings = HashMap::new();
                for p in st.query_map([], |r| r.get::<_, String>(0))? {
                    let p = p?;
                    if let Some((d, n)) = p.rsplit_once('/') {
                        out.entry(d.to_string()).or_default().push(n.to_string());
                    }
                }
                Ok(out)
            })
            .await
    }

    /// What a file is, with no library type: Fix Match's choice (`stored`), else the old import's
    /// (`stored`), else its name, folder and the other videos there (`parse::classify`), then an
    /// NFO next to it (`<episodedetails>`, `<movie>`) or a `tvshow.nfo` above it.
    pub async fn classify(&self, f: &FileRow, idx: &Siblings, stored: bool) -> Parsed {
        use super::parse::{as_episode, as_movie};
        let forced = |mut p: Parsed, why: &str| {
            p.signal = why.into();
            p.weak = false;
            p
        };
        if stored {
            if let Some(k) = f.manual_key.as_deref() {
                if k.starts_with("tmdb:movie:") || k == "local:movie" {
                    return forced(as_movie(&f.rel), "manual");
                }
                if k.starts_with("tmdb:tv:") || k == "local:episode" {
                    return forced(as_episode(&f.rel), "manual");
                }
            }
            if let Ok(Some(b)) = self.boot_for(&f.id, &f.path).await {
                match b.kind.as_deref() {
                    Some("Movie") => return forced(as_movie(&f.rel), "import"),
                    Some("Episode") => return forced(as_episode(&f.rel), "import"),
                    _ => {}
                }
            }
        }
        let (dir, name) = f.path.rsplit_once('/').unwrap_or(("", f.path.as_str()));
        let sib: Vec<&str> = idx
            .get(dir)
            .map(|v| v.iter().map(String::as_str).filter(|s| *s != name).collect())
            .unwrap_or_default();
        let p = super::parse::classify(&f.rel, &sib);
        if p.kind == "movie" || p.weak {
            match self.nfo_kind(f).await {
                Some(("episode", season, episode)) if p.kind == "movie" || season.is_some() => {
                    let mut e = forced(as_episode(&f.rel), "nfo");
                    if episode.is_some() {
                        (e.season, e.episode) = (season.or(e.season), episode);
                    }
                    return e;
                }
                Some(("movie", _, _)) if p.kind == "episode" => return forced(as_movie(&f.rel), "nfo"),
                _ => {}
            }
        }
        p
    }

    /// What NFOs say a file is: its own (`<stem>.nfo`: movie, or episode with its numbers), else
    /// a `tvshow.nfo` in its folder or the one above (inside the library root): an episode.
    async fn nfo_kind(&self, f: &FileRow) -> Option<(&'static str, Option<i32>, Option<i32>)> {
        let dir = crate::storage::vfs::parent(&f.path)?;
        let names = self.folder_names(dir.clone()).await;
        let stem = super::parse::stem_of(f.rel.rsplit('/').next().unwrap_or(""));
        if let Some(n) = self.read_nfo(&dir, &names, &[format!("{stem}.nfo")]).await {
            match n.root.as_deref() {
                Some("episodedetails") => return Some(("episode", n.season, n.episode)),
                Some("movie") => return Some(("movie", None, None)),
                _ => {}
            }
        }
        let root = f.path[..f.path.len() - f.rel.len()].trim_end_matches('/').to_string();
        let mut d = dir;
        for _ in 0..2 {
            if d.len() <= root.len() {
                break;
            }
            let names = self.folder_names(d.clone()).await;
            if names.iter().any(|n| n.eq_ignore_ascii_case("tvshow.nfo")) {
                return Some(("episode", None, None));
            }
            d = crate::storage::vfs::parent(&d)?;
        }
        None
    }

    /// A weakly numbered episode (names alike but for a number) that is better a movie: its show
    /// isn't found on TMDB, but its own name is a movie that fits (`blind`: no stored ids).
    async fn prefers_movie(&self, f: &FileRow, cache: &mut HashMap<String, Ident>, blind: bool) -> Result<bool> {
        if f.parsed.kind != "episode" || !f.parsed.weak || self.tmdb().is_none() || f.manual_key.is_some() {
            return Ok(false);
        }
        let series = if blind {
            let folder = f.parsed.series_folder.clone().unwrap_or_default();
            match cache.get(&folder) {
                Some(i) => i.clone(),
                None => {
                    let i = self.identify_series_blind(f).await?;
                    cache.insert(folder, i.clone());
                    i
                }
            }
        } else {
            self.identify_series(f, cache).await?
        };
        if series.state != "review" {
            return Ok(false);
        }
        let m = FileRow {
            parsed: super::parse::as_movie(&f.rel),
            ..f.clone()
        };
        Ok(matches!(self.identify_movie_blind(&m).await?.state, "matched" | "llm"))
    }

    /// An episode's numbers in TMDB's order: by its air date (a date in the name), or from an
    /// absolute number (`One Piece - 1071`, fansub names) through the seasons' episode counts.
    async fn tmdb_numbers(&self, p: &Parsed, series: &Ident, season: Option<i32>, episode: Option<i32>) -> (Option<i32>, Option<i32>) {
        let (Some(id), Some(tmdb)) = (series.tmdb_id(), self.tmdb()) else {
            return (season, episode);
        };
        if episode.is_none() {
            let Some(date) = p.air_date.as_deref() else {
                return (season, episode);
            };
            let Ok(show) = tmdb.tv(id, 7 * super::tmdb::DAY).await else {
                return (season, episode);
            };
            let year: i32 = date[..4].parse().unwrap_or(0);
            for s in show["seasons"].as_array().into_iter().flatten().rev() {
                let (Some(n), sy) = (s["season_number"].as_i64(), year_of(&s["air_date"])) else {
                    continue;
                };
                if n == 0 || sy.is_some_and(|y| y > year) {
                    continue;
                }
                let Ok(sv) = tmdb.season(id, n, 7 * super::tmdb::DAY).await else {
                    continue;
                };
                if let Some(e) = sv["episodes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|e| e["air_date"].as_str() == Some(date))
                {
                    return (Some(n as i32), e["episode_number"].as_i64().map(|x| x as i32));
                }
            }
            return (season, episode);
        }
        if !matches!(p.signal.as_str(), "fansub" | "numbered") || season.is_some_and(|s| s > 1) {
            return (season, episode);
        }
        let Ok(show) = tmdb.tv(id, 7 * super::tmdb::DAY).await else {
            return (season, episode);
        };
        let mut counts: Vec<(i32, i32)> = show["seasons"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| Some((s["season_number"].as_i64()? as i32, s["episode_count"].as_i64()? as i32)))
            .filter(|(n, c)| *n > 0 && *c > 0)
            .collect();
        counts.sort_unstable();
        let abs = episode.unwrap_or(0);
        let mut left = abs;
        if counts.first().is_some_and(|(_, c)| left <= *c) {
            return (Some(season.unwrap_or(1)), episode);
        }
        for (n, c) in counts {
            if left <= c {
                // Some shows keep absolute numbers within their seasons on TMDB (One Piece).
                let sv = tmdb.season(id, n as i64, 7 * super::tmdb::DAY).await.unwrap_or(Value::Null);
                let has = |x: i32| {
                    sv["episodes"]
                        .as_array()
                        .is_some_and(|eps| eps.iter().any(|e| e["episode_number"].as_i64() == Some(x as i64)))
                };
                return (Some(n), Some(if !has(left) && has(abs) { abs } else { left }));
            }
            left -= c;
        }
        (season, episode)
    }

    /// Titles waiting in Fix Match (identification state `review`).
    pub async fn review_count(&self) -> Result<i64> {
        self.db
            .call(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM items WHERE identify_state = 'review' AND kind IN ('movie', 'series', 'episode')",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
    }

    /// The library's present title files matching `cond` (SQL on `files`), with their stability
    /// stamps (stable_since, last_seen).
    pub async fn file_rows(&self, cond: &str) -> Result<Vec<(FileRow, i64, i64)>> {
        let libs = self.libs();
        let sql = format!(
            "SELECT id, library_id, path, duration, oshash, parsed, manual_key, stable_since, last_seen, mtime FROM files
             WHERE missing_since IS NULL AND role IN ('main', 'broken') AND ({cond}) ORDER BY path"
        );
        self.db
            .call(move |c| {
                let mut st = c.prepare(&sql)?;
                let rows = st
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, Option<f64>>(3)?,
                            r.get::<_, Option<String>>(4)?,
                            r.get::<_, Option<String>>(5)?,
                            r.get::<_, Option<String>>(6)?,
                            r.get::<_, i64>(7)?,
                            r.get::<_, i64>(8)?,
                            r.get::<_, i64>(9)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows
                    .into_iter()
                    .filter_map(|r| {
                        let lib = libs.iter().find(|l| l.id == r.1)?;
                        let root = lib.roots.iter().find(|root| r.2.starts_with(&format!("{root}/")))?;
                        Some((
                            FileRow {
                                rel: r.2[root.len() + 1..].to_string(),
                                id: r.0,
                                library_id: r.1,
                                path: r.2,
                                duration: r.3,
                                oshash: r.4,
                                parsed: r.5.and_then(|p| serde_json::from_str(&p).ok()).unwrap_or_default(),
                                manual_key: r.6,
                                mtime: r.9,
                            },
                            r.7,
                            r.8,
                        ))
                    })
                    .collect())
            })
            .await
    }

    /// The match hints' paths → our file ids: from then on a hint follows its file's renames.
    pub(crate) async fn link_match_hints(&self) -> Result<usize> {
        self.db
            .call(|c| {
                Ok(c.execute(
                    "UPDATE match_hints SET file_id = (SELECT id FROM files WHERE path = match_hints.path)
                     WHERE file_id IS NULL AND kind IN ('movie', 'episode')",
                    [],
                )?)
            })
            .await
    }

    /// A movie's or episode's match hint: by its file (which follows renames), else its path.
    async fn boot_for(&self, file_id: &str, path: &str) -> Result<Option<Boot>> {
        let (fid, p) = (file_id.to_string(), path.to_string());
        self.db
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT kind, tmdb_id, imdb_id, season_no, episode_no, episode_end, added_at, title FROM match_hints
                     WHERE kind IN ('movie', 'episode') AND (file_id = ?1 OR path = ?2) ORDER BY file_id = ?1 DESC LIMIT 1",
                    params![fid, p],
                    |r| {
                        Ok(Boot {
                            kind: Some(if r.get::<_, String>(0)? == "movie" { "Movie" } else { "Episode" }.to_string()),
                            tmdb: r.get(1)?,
                            imdb: r.get(2)?,
                            season: r.get(3)?,
                            episode: r.get(4)?,
                            episode_end: r.get(5)?,
                            date_created: r.get(6)?,
                            name: r.get(7)?,
                        })
                    },
                )
                .optional()?)
            })
            .await
    }

    /// A series folder's match hint.
    async fn boot_series(&self, folder_path: &str) -> Result<Option<Boot>> {
        let p = folder_path.to_string();
        self.db
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT tmdb_id, imdb_id, added_at FROM match_hints WHERE kind = 'series' AND path = ?1",
                    [p],
                    |r| {
                        Ok(Boot {
                            tmdb: r.get(0)?,
                            imdb: r.get(1)?,
                            date_created: r.get(2)?,
                            ..Boot::default()
                        })
                    },
                )
                .optional()?)
            })
            .await
    }

    async fn identify_movie(&self, f: &FileRow) -> Result<()> {
        let boot = self.boot_for(&f.id, &f.path).await?;
        let ident = if f.manual_key.as_deref().is_some_and(|k| k == "local" || k == "local:movie") {
            // Fix Match's "Don't Match": named from the file and its folder, never looked up.
            Ident {
                key: format!("path:{}/{}", f.library_id, f.rel),
                state: "manual",
                source: "local".into(),
                confidence: 1.0,
                ..Default::default()
            }
        } else if f.manual_key.as_deref() == Some("split") {
            // Split off from the title it was a version of: its own (Fix Match it to say what it is).
            Ident {
                key: format!("path:{}/{}", f.library_id, f.rel),
                state: "manual",
                source: "split".into(),
                confidence: 1.0,
                ..Default::default()
            }
        } else if let Some(k) = f.manual_key.clone() {
            Ident {
                key: k,
                state: "manual",
                source: "admin".into(),
                confidence: 1.0,
                note: None,
                ..Default::default()
            }
        } else if let Some(id) = self.boot_tmdb(boot.as_ref(), "movie").await {
            Ident {
                key: format!("tmdb:movie:{id}"),
                state: "bootstrap",
                source: "bootstrap".into(),
                confidence: 1.0,
                note: None,
                ..Default::default()
            }
        } else if let Some(id) = self.nfo_tmdb(&f.path, &f.rel, "movie", self.own_folder(f).await.is_some()).await {
            Ident {
                key: format!("tmdb:movie:{id}"),
                state: "matched",
                source: "nfo".into(),
                confidence: 1.0,
                ..Default::default()
            }
        } else {
            self.identify_movie_blind(f).await?
        };
        let title = if ident.key.starts_with("path:") {
            match f.manual_key.as_deref() {
                // Apart from the title it was a version of: the file's own name says which.
                Some("split") => f
                    .rel
                    .rsplit('/')
                    .next()
                    .map_or_else(|| f.parsed.title.clone(), |n| n.rsplit_once('.').map_or(n, |(a, _)| a).to_string()),
                _ => f.parsed.title.clone(),
            }
        } else {
            String::new()
        };
        let added = boot
            .as_ref()
            .and_then(|b| b.date_created)
            .unwrap_or(f.mtime / 1_000_000_000)
            .min(now());
        let (lib, fid, path) = (f.library_id.clone(), f.id.clone(), f.path.clone());
        let year = f.parsed.year;
        self.db
            .call(move |c| {
                let tx = c.transaction()?;
                let item = ensure_item(
                    &tx,
                    &ItemSpec {
                        key: &ident.key,
                        kind: "movie",
                        library_id: &lib,
                        title: &title,
                        year,
                        added_at: added,
                        revive_for: Some(&fid),
                        ..Default::default()
                    },
                    &ident,
                )?;
                tx.execute(
                    "UPDATE items SET candidates = ?2 WHERE id = ?1",
                    params![item, review_candidates(&ident)],
                )?;
                assign(&tx, &fid, &item)?;
                tx.commit()?;
                tracing::debug!(path, key = ident.key, state = ident.state, "identified");
                Ok(())
            })
            .await
    }

    /// TMDB id from the stored ids (Tmdb, else Imdb through /find).
    async fn boot_tmdb(&self, boot: Option<&Boot>, kind: &str) -> Option<i64> {
        let b = boot?;
        if let Some(t) = b.tmdb {
            return Some(t);
        }
        let imdb = b.imdb.as_ref()?;
        let tmdb = self.tmdb()?;
        let v = tmdb.find(imdb, "imdb_id").await.ok()?;
        v[if kind == "movie" { "movie_results" } else { "tv_results" }][0]["id"].as_i64()
    }

    /// A movie's identity from its name alone: TMDB scoring, then the LLM, else review.
    pub async fn identify_movie_blind(&self, f: &FileRow) -> Result<Ident> {
        if self.tmdb().is_none() {
            return Ok(Ident {
                key: format!("path:{}/{}", f.library_id, f.rel),
                state: "local",
                source: "parse".into(),
                ..Default::default()
            });
        }
        let own_folder = self.own_folder(f).await;
        let (readings, strong) = movie_readings(&f.parsed, own_folder.as_deref());
        let cands = self.candidates("movie", &readings, strong, f.duration, is_cut(&f.rel)).await?;
        if let Some(i) = accept(&cands, "matched") {
            return Ok(Ident {
                key: format!("tmdb:movie:{}", i.0),
                state: "matched",
                source: "tmdb".into(),
                confidence: i.1,
                candidates: cands,
                ..Default::default()
            });
        }
        if let Some(v) = self.ask_llm(f, "movie", &readings, cands.clone()).await? {
            return Ok(v);
        }
        Ok(Ident {
            candidates: cands,
            ..review(f, "no confident TMDB match")
        })
    }

    /// The TMDB id an NFO file gives (its own, or through its IMDb id), with a TMDB key.
    async fn nfo_tmdb(&self, path: &str, rel: &str, kind: &str, own_folder: bool) -> Option<i64> {
        let tmdb = self.tmdb()?;
        let n = self.nfo_for(path, rel, kind, own_folder).await?;
        if let Some(id) = n.tmdb {
            return Some(id);
        }
        let v = tmdb.find(n.imdb.as_deref()?, "imdb_id").await.ok()?;
        v[if kind == "movie" { "movie_results" } else { "tv_results" }][0]["id"].as_i64()
    }

    /// The name of the folder a movie file is in, when no other title is in it (or below it).
    async fn own_folder(&self, f: &FileRow) -> Option<String> {
        let (dir_rel, _) = f.rel.rsplit_once('/')?;
        let name = dir_rel.rsplit('/').next()?.to_string();
        let dir = f.path[..f.path.len() - f.rel.len()].to_string() + dir_rel;
        let (d, id) = (dir.clone(), f.id.clone());
        let others: i64 = self
            .db
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM files WHERE missing_since IS NULL AND role IN ('main', 'broken') AND id != ?2 AND substr(path, 1, length(?1) + 1) = ?1 || '/'",
                    params![d, id],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap_or(1);
        (others == 0).then_some(name)
    }

    /// TMDB candidates for the readings (the first `strong` the file's own), scored, best first;
    /// for the top ones the runtime is checked against the file's duration (`cut`: the name says
    /// it is a longer cut) and every title TMDB knows compared.
    async fn candidates(
        &self,
        kind: &str,
        readings: &[(String, Option<i32>)],
        strong: usize,
        duration: Option<f64>,
        cut: bool,
    ) -> Result<Vec<Candidate>> {
        let Some(tmdb) = self.tmdb() else { return Ok(Vec::new()) };
        let mut by_id: HashMap<i64, Candidate> = HashMap::new();
        for (title, year) in readings.iter().take(strong.clamp(4, 6)) {
            let mut results = tmdb.search(kind, title, *year).await?;
            if results.is_empty() && year.is_some() {
                results = tmdb.search(kind, title, None).await?;
            }
            for r in results.iter().take(8) {
                let Some(id) = r["id"].as_i64() else { continue };
                let (t, ot, d) = if kind == "movie" {
                    ("title", "original_title", "release_date")
                } else {
                    ("name", "original_name", "first_air_date")
                };
                by_id.entry(id).or_insert_with(|| Candidate {
                    id,
                    title: r[t].as_str().unwrap_or("").to_string(),
                    original_title: r[ot].as_str().unwrap_or("").to_string(),
                    year: year_of(&r[d]),
                    runtime_minutes: None,
                    overview: r["overview"].as_str().unwrap_or("").chars().take(200).collect(),
                    seasons: None,
                    score: 0.0,
                    popularity: r["popularity"].as_f64().unwrap_or(0.0),
                    fits: false,
                    poster: r["poster_path"].as_str().map(String::from),
                });
            }
        }
        let mut cands: Vec<Candidate> = by_id.into_values().collect();
        for c in &mut cands {
            c.score = score(readings, strong, c);
        }
        cands.sort_by(|a, b| b.score.total_cmp(&a.score).then(b.popularity.total_cmp(&a.popularity)));
        cands.truncate(8);
        // A title that is exactly one of the file's own readings fits when the year does (±1);
        // a reading without a year needs the runtime to (movies), shows don't.
        let year_fits = |want: Option<i32>, c: &Candidate, runtime_ok: Option<bool>| match want {
            Some(y) => c.year.is_some_and(|cy| (cy - y).abs() <= 1) && runtime_ok != Some(false),
            None => kind != "movie" || runtime_ok == Some(true),
        };
        let max_ratio = if cut { 1.45 } else { 1.15 };
        for (i, c) in cands.iter_mut().enumerate() {
            let (titles, runtime_ok) = if i < 3 {
                // Runtime and every title TMDB knows: confirm (or sink) the top ones.
                let d = if kind == "movie" {
                    tmdb.movie(c.id, 30 * super::tmdb::DAY).await
                } else {
                    tmdb.tv(c.id, 30 * super::tmdb::DAY).await
                };
                let Ok(d) = d else { continue };
                let alt = all_titles(&d);
                let refs: Vec<&str> = alt.iter().map(String::as_str).collect();
                c.score = c.score.max(score_titles(readings, strong, &refs, c.year));
                let mut ok = None;
                if kind == "movie" {
                    c.runtime_minutes = d["runtime"].as_i64().filter(|r| *r > 0);
                    if let (Some(rt), Some(dur)) = (c.runtime_minutes, duration) {
                        let ratio = dur / 60.0 / rt as f64;
                        if (0.85..=max_ratio).contains(&ratio) {
                            c.score = (c.score + 0.05).min(1.0);
                            ok = Some(true);
                        } else {
                            ok = Some(false);
                            if !(0.7..=max_ratio + 0.15).contains(&ratio) {
                                c.score -= 0.25;
                            }
                        }
                    }
                } else {
                    c.seasons = d["number_of_seasons"].as_i64();
                }
                (alt, ok)
            } else {
                (vec![c.title.clone(), c.original_title.clone()], None)
            };
            c.fits = exact(readings, strong, &titles).is_some_and(|want| year_fits(want, c, runtime_ok));
        }
        cands.sort_by(|a, b| b.score.total_cmp(&a.score).then(b.popularity.total_cmp(&a.popularity)));
        Ok(cands)
    }

    /// The LLM's verdict (cached), accepted only if it passes the checks. None: no usable
    /// verdict (the item goes to review).
    async fn ask_llm(
        &self,
        f: &FileRow,
        kind: &str,
        readings: &[(String, Option<i32>)],
        mut cands: Vec<Candidate>,
    ) -> Result<Option<Ident>> {
        let Some(llm) = self.llm().filter(|_| self.config.enabled(crate::admin::config::Feature::Llm)) else {
            return Ok(None);
        };
        for round in 0..2 {
            let probe = self.probe_summary(f).await;
            let input = json!({
                "looks_like": f.parsed.kind,
                "because": f.parsed.signal,
                "path": f.rel,
                "parsed": {"title": f.parsed.title, "year": f.parsed.year, "alt_titles": readings.iter().skip(1).map(|r| &r.0).collect::<Vec<_>>(),
                           "series_folder": f.parsed.series_folder, "season": f.parsed.season, "episode": f.parsed.episode},
                "duration_minutes": f.duration.map(|d| (d / 60.0).round()),
                "file": probe,
                "candidates": cands,
            });
            let ids: BTreeSet<i64> = cands.iter().map(|c| c.id).collect();
            let key_src = format!(
                "{}|{PROMPT_VERSION}|{}|{:?}|{round}|{kind}",
                f.oshash.clone().unwrap_or_else(|| f.path.clone()),
                llm.config.model,
                ids
            );
            let key: String = Sha256::digest(key_src.as_bytes())
                .iter()
                .take(16)
                .map(|b| format!("{b:02x}"))
                .collect();
            let verdict = match self.cached_verdict(&key).await? {
                Some(v) => v,
                None => {
                    if self.llm_calls.load(std::sync::atomic::Ordering::Relaxed) >= llm.config.max_calls_per_scan {
                        tracing::info!(path = f.path, "LLM call cap reached: review");
                        return Ok(None);
                    }
                    let v = self.llm_vote(&llm, &input.to_string()).await;
                    let (verdict, tin, tout, err) = match v {
                        Ok((val, tin, tout)) => (Some(val), tin, tout, None),
                        Err(e) => (None, 0, 0, Some(format!("{e:#}"))),
                    };
                    let (k, model, req, vs) = (key.clone(), llm.config.model.clone(), input.to_string(), verdict.clone());
                    self.db
                        .call(move |c| {
                            c.execute(
                                "INSERT OR REPLACE INTO llm_verdicts(key, model, prompt_version, request, verdict, accepted, reason, input_tokens, output_tokens, created_at)
                                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8, ?9)",
                                params![k, model, PROMPT_VERSION, req, vs.map(|v| v.to_string()), err, tin, tout, now()],
                            )?;
                            Ok(())
                        })
                        .await?;
                    match verdict {
                        Some(v) => v,
                        None => return Ok(None),
                    }
                }
            };
            let check = self.check_verdict(f, kind, readings, &cands, &verdict).await;
            let accepted = check.is_ok();
            let reason = verdict["reason"].as_str().unwrap_or("").to_string();
            let (k, why) = (key.clone(), check.as_ref().err().cloned());
            self.db
                .call(move |c| {
                    c.execute(
                        "UPDATE llm_verdicts SET accepted = ?2, reason = COALESCE(?3, reason) WHERE key = ?1",
                        params![k, accepted, why],
                    )?;
                    Ok(())
                })
                .await?;
            match check {
                Ok(id) => {
                    let key = if kind == "movie" {
                        format!("tmdb:movie:{id}")
                    } else {
                        format!("tmdb:tv:{id}")
                    };
                    let conf = verdict["confidence"].as_f64().unwrap_or(0.0);
                    tracing::info!(path = f.path, key, conf, reason, "identified by the LLM");
                    return Ok(Some(Ident {
                        key,
                        state: "llm",
                        source: llm.config.model.clone(),
                        confidence: conf,
                        note: Some(reason),
                        ..Default::default()
                    }));
                }
                Err(why) => {
                    // A suggestion to search for: one more round with its candidates.
                    let sug = verdict["suggested_title"]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .map(String::from);
                    if round == 0 && verdict["tmdb_id"].is_null() {
                        if let Some(t) = sug {
                            let more = vec![(t, verdict["suggested_year"].as_i64().map(|y| y as i32))];
                            let mut extra = self.candidates(kind, &more, 1, f.duration, is_cut(&f.rel)).await?;
                            extra.retain(|c| !ids.contains(&c.id));
                            if !extra.is_empty() {
                                cands.extend(extra);
                                cands.truncate(10);
                                continue;
                            }
                        }
                    }
                    tracing::info!(path = f.path, why, reason, "LLM verdict not accepted");
                    return Ok(None);
                }
            }
        }
        Ok(None)
    }

    async fn cached_verdict(&self, key: &str) -> Result<Option<Value>> {
        let k = key.to_string();
        let v: Option<Option<String>> = self
            .db
            .call(move |c| {
                Ok(c.query_row("SELECT verdict FROM llm_verdicts WHERE key = ?1", [k], |r| r.get(0))
                    .optional()?)
            })
            .await?;
        Ok(v.flatten().and_then(|s| serde_json::from_str(&s).ok()))
    }

    /// One verdict, or the 2-of-3 majority when `MOONRISE_LLM_VOTES=3` and it is unsure.
    async fn llm_vote(&self, llm: &crate::llm::Llm, input: &str) -> Result<(Value, i64, i64)> {
        let schema = verdict_schema();
        let call = || async {
            self.llm_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let v = llm.classify(SYSTEM, input, &schema).await;
            if let Ok(v) = &v {
                super::count_usage(&self.db, "llm", v.input_tokens, v.output_tokens).await;
                tracing::info!(input_tokens = v.input_tokens, output_tokens = v.output_tokens, "llm call");
            } else {
                super::count_usage(&self.db, "llm", 0, 0).await;
            }
            v
        };
        let first = call().await?;
        let (mut tin, mut tout) = (first.input_tokens, first.output_tokens);
        if llm.config.votes < 3 || first.value["confidence"].as_f64().unwrap_or(0.0) >= 0.7 {
            return Ok((first.value, tin, tout));
        }
        let mut votes = vec![first.value];
        for _ in 0..2 {
            if let Ok(v) = call().await {
                tin += v.input_tokens;
                tout += v.output_tokens;
                votes.push(v.value);
            }
        }
        let pick = votes
            .iter()
            .find(|v| votes.iter().filter(|w| w["tmdb_id"] == v["tmdb_id"]).count() >= 2)
            .cloned();
        match pick {
            Some(v) => Ok((v, tin, tout)),
            None => Err(crate::llm::NoVerdict("no majority".into()).into()),
        }
    }

    /// Appendix A's checks. Ok(tmdb id) or Err(why not).
    async fn check_verdict(
        &self,
        f: &FileRow,
        kind: &str,
        readings: &[(String, Option<i32>)],
        cands: &[Candidate],
        v: &Value,
    ) -> Result<i64, String> {
        let want = if kind == "movie" { "movie" } else { "episode" };
        if v["kind"].as_str() != Some(want) {
            return Err(format!("kind {}", v["kind"]));
        }
        let id = v["tmdb_id"].as_i64().ok_or("no tmdb_id")?;
        let c = cands.iter().find(|c| c.id == id).ok_or("not a candidate")?;
        let year = if kind == "movie" { f.parsed.year } else { f.parsed.series_year };
        if let (Some(p), Some(y)) = (year, c.year) {
            if (p - y).abs() > 1 {
                return Err(format!("year {y} vs {p}"));
            }
        }
        let tmdb = self.tmdb().ok_or("no tmdb")?;
        let details = if kind == "movie" {
            tmdb.movie(id, 30 * super::tmdb::DAY).await
        } else {
            tmdb.tv(id, 30 * super::tmdb::DAY).await
        }
        .map_err(|e| e.to_string())?;
        let titles = all_titles(&details);
        // Also against the reading cut to the title's length ("Troy Director's Cut" ~ "Troy").
        let sim = readings
            .iter()
            .flat_map(|(t, _)| {
                titles.iter().map(move |a| {
                    let n = a.split_whitespace().count().max(1);
                    let cut: String = t.split_whitespace().take(n).collect::<Vec<_>>().join(" ");
                    similarity(t, a).max(similarity(&cut, a))
                })
            })
            .fold(0.0, f64::max);
        if sim < 0.5 {
            return Err(format!("title similarity {sim:.2}"));
        }
        if kind == "movie" {
            if let (Some(rt), Some(d)) = (details["runtime"].as_i64().filter(|r| *r > 0), f.duration) {
                let ratio = d / 60.0 / rt as f64;
                // Longer cuts are named as such (extended, director's cut, Redux…).
                let max = if is_cut(&f.rel) { 1.45 } else { 1.15 };
                if !(0.85..=max).contains(&ratio) {
                    return Err(format!("runtime {rt} min vs {:.0}", d / 60.0));
                }
            }
        }
        Ok(id)
    }

    async fn probe_summary(&self, f: &FileRow) -> Value {
        let p = f.path.clone();
        let probe: Option<String> = self
            .db
            .call(move |c| {
                Ok(c.query_row("SELECT probe FROM files_probe WHERE path = ?1", [p], |r| r.get(0))
                    .optional()?)
            })
            .await
            .ok()
            .flatten();
        let Some(v) = probe.and_then(|p| serde_json::from_str::<Value>(&p).ok()) else {
            return Value::Null;
        };
        let streams = v["streams"].as_array().cloned().unwrap_or_default();
        json!({
            "embedded_title": v["format"]["tags"]["title"],
            "audio_languages": streams.iter().filter(|s| s["codec_type"] == "audio").filter_map(|s| s["tags"]["language"].as_str()).collect::<Vec<_>>(),
            "video": streams.iter().find(|s| s["codec_type"] == "video").map(|s| format!("{}x{}", s["width"], s["height"])),
        })
    }

    /// A series folder's identity (once per pass).
    async fn identify_series(&self, f: &FileRow, cache: &mut HashMap<String, Ident>) -> Result<Ident> {
        let folder = f.parsed.series_folder.clone().unwrap_or_default();
        if let Some(i) = cache.get(&folder) {
            return Ok(i.clone());
        }
        let root = f.path[..f.path.len() - f.rel.len() - 1].to_string();
        let boot = self.boot_series(&format!("{root}/{folder}")).await?;
        let ident = if let Some(id) = self.boot_tmdb(boot.as_ref(), "tv").await {
            Ident {
                key: format!("tmdb:tv:{id}"),
                state: "bootstrap",
                source: "bootstrap".into(),
                confidence: 1.0,
                note: None,
                ..Default::default()
            }
        } else if let Some(id) = self.nfo_tmdb(&format!("{root}/{folder}/tvshow.nfo"), "", "series", true).await {
            Ident {
                key: format!("tmdb:tv:{id}"),
                state: "matched",
                source: "nfo".into(),
                confidence: 1.0,
                ..Default::default()
            }
        } else {
            self.identify_series_blind(f).await?
        };
        cache.insert(folder, ident.clone());
        Ok(ident)
    }

    pub async fn identify_series_blind(&self, f: &FileRow) -> Result<Ident> {
        let folder = f.parsed.series_folder.clone().unwrap_or_default();
        if self.tmdb().is_none() {
            return Ok(Ident {
                key: format!("path:{}/{folder}", f.library_id),
                state: "local",
                source: "parse".into(),
                ..Default::default()
            });
        }
        let readings = vec![(f.parsed.series_title.clone().unwrap_or_default(), f.parsed.series_year)];
        let cands = self.candidates("tv", &readings, 1, None, false).await?;
        if let Some(i) = accept(&cands, "matched") {
            return Ok(Ident {
                key: format!("tmdb:tv:{}", i.0),
                state: "matched",
                source: "tmdb".into(),
                confidence: i.1,
                candidates: cands,
                ..Default::default()
            });
        }
        if let Some(v) = self.ask_llm(f, "tv", &readings, cands.clone()).await? {
            return Ok(v);
        }
        Ok(Ident {
            key: format!("path:{}/{folder}", f.library_id),
            state: "review",
            source: "parse".into(),
            note: Some("no confident TMDB match".into()),
            candidates: cands,
            ..Default::default()
        })
    }

    /// `moonrise-server identify-eval [--typed]`: every title classified and identified again from
    /// its path alone, with no library type, no stored ids and no LLM (nothing is written but
    /// TMDB's answer cache), as if all libraries' roots were one mixed library (paths below the
    /// roots' common folder; `typed`: below each root, as now); compared with the database: the
    /// kind (movie or episode), the movie, the show and the episode numbers. Answers the counts
    /// and every difference.
    pub async fn eval_blind(&self, typed: bool) -> Result<Value> {
        // What each file is now: (kind, key, state, title, season, episode, series key, series state, series title).
        type Cur = (
            String,
            String,
            String,
            String,
            Option<i32>,
            Option<i32>,
            Option<String>,
            Option<String>,
            Option<String>,
        );
        let mut files: Vec<FileRow> = self.file_rows("item_id IS NOT NULL").await?.into_iter().map(|r| r.0).collect();
        let roots: Vec<String> = self.libs().iter().flat_map(|l| l.roots.clone()).collect();
        let common = common_folder(&roots);
        if !typed {
            for f in &mut files {
                if let Some(r) = f.path.strip_prefix(&format!("{common}/")) {
                    f.rel = r.to_string();
                }
                f.library_id = "mixed".into();
            }
        }
        let ids: Vec<String> = files.iter().map(|f| f.id.clone()).collect();
        let current: HashMap<String, Cur> = self
            .db
            .call(move |c| {
                let mut st = c.prepare(
                    "SELECT i.kind, i.key, i.identify_state, i.title, i.season_no, i.episode_no, s.key, s.identify_state, s.title
                     FROM files f JOIN items i ON i.id = f.item_id LEFT JOIN items s ON s.id = i.series_id WHERE f.id = ?1",
                )?;
                let mut out = HashMap::new();
                for id in ids {
                    if let Some(r) = st
                        .query_row([&id], |r| {
                            Ok((
                                r.get(0)?,
                                r.get(1)?,
                                r.get(2)?,
                                r.get(3)?,
                                r.get(4)?,
                                r.get(5)?,
                                r.get(6)?,
                                r.get(7)?,
                                r.get(8)?,
                            ))
                        })
                        .optional()?
                    {
                        out.insert(id, r);
                    }
                }
                Ok(out)
            })
            .await?;
        let idx = self.sibling_index().await?;
        let mut counts: BTreeMap<String, i64> = BTreeMap::new();
        let mut diffs = Vec::new();
        let mut series_seen: HashMap<String, Ident> = HashMap::new();
        let mut series_counted: BTreeSet<String> = BTreeSet::new();
        let short = |c: &[Candidate]| {
            c.iter()
                .take(3)
                .map(|c| json!({"id": c.id, "title": c.title, "year": c.year, "score": (c.score * 1000.0).round() / 1000.0}))
                .collect::<Vec<_>>()
        };
        for f in &files {
            let Some(cur) = current.get(&f.id) else { continue };
            let mut f = f.clone();
            f.manual_key = None;
            f.parsed = self.classify(&f, &idx, false).await;
            if self.prefers_movie(&f, &mut series_seen, true).await? {
                f.parsed = super::parse::as_movie(&f.rel);
                f.parsed.signal = "tmdb".into();
            }
            *counts.entry(format!("signal.{}", f.parsed.signal)).or_default() += 1;
            if cur.0 != f.parsed.kind {
                *counts.entry(format!("kind.{}_as_{}", cur.0, f.parsed.kind)).or_default() += 1;
                diffs.push(
                    json!({"kind": "type", "verdict": format!("{}_as_{}", cur.0, f.parsed.kind), "path": f.rel,
                    "current": {"key": cur.1, "state": cur.2, "title": cur.3, "series": cur.8}, "parsed": f.parsed}),
                );
                continue;
            }
            *counts.entry("kind.same".into()).or_default() += 1;
            if f.parsed.kind == "movie" {
                let blind = self.identify_movie_blind(&f).await?;
                let verdict = verdict_of(&cur.1, &blind);
                *counts.entry(format!("movies.{verdict}")).or_default() += 1;
                if verdict != "same" {
                    diffs.push(json!({"kind": "movie", "verdict": verdict, "path": f.rel, "current": {"key": cur.1, "state": cur.2, "title": cur.3},
                        "blind": {"key": blind.key, "state": blind.state, "confidence": blind.confidence}, "candidates": short(&blind.candidates)}));
                }
            } else {
                let folder = f.parsed.series_folder.clone().unwrap_or_default();
                if !series_seen.contains_key(&folder) {
                    let b = self.identify_series_blind(&f).await?;
                    series_seen.insert(folder.clone(), b);
                }
                let blind = series_seen[&folder].clone();
                let cur_series = cur.6.clone().unwrap_or_default();
                let sv = verdict_of(&cur_series, &blind);
                if series_counted.insert(folder.clone()) {
                    *counts.entry(format!("series.{sv}")).or_default() += 1;
                    if sv != "same" {
                        diffs.push(json!({"kind": "series", "verdict": sv, "path": folder, "current": {"key": cur_series, "state": cur.7, "title": cur.8},
                            "blind": {"key": blind.key, "state": blind.state, "confidence": blind.confidence}, "candidates": short(&blind.candidates)}));
                    }
                }
                // The episode: its show as above, its numbers from the name (in TMDB's order).
                let (mut s, e) = self.tmdb_numbers(&f.parsed, &blind, f.parsed.season, f.parsed.episode).await;
                if s.is_none() && e.is_some() {
                    s = Some(1);
                }
                let numbers_same = (s, e) == (cur.4, cur.5) && e.is_some();
                let ev = match sv {
                    "same" if numbers_same => "same",
                    "same" => "numbering",
                    other => other,
                };
                *counts.entry(format!("episodes.{ev}")).or_default() += 1;
                if ev == "numbering" {
                    diffs.push(json!({"kind": "episode", "verdict": ev, "path": f.rel, "current": {"key": cur.1, "season": cur.4, "episode": cur.5, "state": cur.2},
                        "parsed": {"season": s, "episode": e, "signal": f.parsed.signal}}));
                }
            }
        }
        let shows: BTreeSet<&String> = current.values().filter_map(|c| c.6.as_ref()).collect();
        Ok(
            json!({"root": if typed { "each library's own".to_string() } else { common }, "files": files.len(), "shows_now": shows.len(),
                  "show_folders_found": series_counted.len(), "counts": counts, "differences": diffs}),
        )
    }

    async fn identify_episode(&self, f: &FileRow, cache: &mut HashMap<String, Ident>) -> Result<()> {
        let boot = self.boot_for(&f.id, &f.path).await?;
        let (split, manual, manual_part) = manual_episode_key(f);
        let series = self.episode_series(f, manual.as_deref(), cache).await?;
        let (season, episode, episode_end, mut note) = self.episode_numbers(f, boot.as_ref(), manual.as_ref(), &series).await;
        let mut state = if manual.is_some() || split { "manual" } else { series.state };
        // The episode's title until TMDB's arrives (TMDB may not list it: other numbering).
        let mut ep_title = episode_title(boot.as_ref(), f, episode);
        let (ep_key, season_key) = match (season, episode) {
            (Some(s), Some(e)) => {
                // Does TMDB know it? (Not a reason to hide it: listed, flagged for review.)
                if !self.tmdb_lists(&series, s, e).await && state != "manual" {
                    state = "review";
                    note = Some(format!("S{s}E{e} isn't in TMDB's season list"));
                }
                let base = format!("{}:s{s}e{e}", series.key);
                let key = match manual_part {
                    Some(p) if p >= 2 => format!("{base}:p{p}"),
                    _ => base,
                };
                (key, format!("{}:s{s}", series.key))
            }
            _ => {
                state = "review";
                note = Some("no season/episode numbers in the name".into());
                if ep_title.is_empty() {
                    ep_title = f.rel.rsplit('/').next().unwrap_or("").to_string();
                }
                (
                    format!("path:{}/{}", f.library_id, f.rel),
                    format!("{}:s{}", series.key, season.unwrap_or(0)),
                )
            }
        };
        // Already placed by the season's alignment with TMDB's list (parts.rs): it stays there.
        let (mut ep_key, mut episode, mut episode_end, mut source) = (ep_key, episode, episode_end, series.source.clone());
        if split {
            // Split off by hand: its own title (Fix Match it to say what it is).
            ep_key = format!("path:{}/{}", f.library_id, f.rel);
            source = "split".into();
            note = None;
        } else if manual.is_none() {
            let prefix = format!("{}:s{}e", series.key, season.unwrap_or(-1));
            let placed = self.aligned_placement(&f.id, prefix).await?;
            if let Some((k, e, end, n)) = placed {
                ep_key = k;
                (episode, episode_end) = (e, end);
                source = "aligned".into();
                if state == "review" {
                    state = "matched";
                }
                note = n;
            }
        }
        let mut part_no = if split { None } else { manual_part };
        // Parts named as such under one number ("Pilot Pt.1", "Pilot Pt.2"; both S01E01): parts of
        // the episode, not versions of it.
        if manual.is_none() && !split && source != "aligned" && f.parsed.episode_end.is_none() && ep_key.starts_with("tmdb:") {
            if let Some(m) = self.part_beside(f).await? {
                part_no = Some(m);
                if m >= 2 {
                    ep_key = format!("{ep_key}:p{m}");
                }
            }
        }
        let ident = Ident {
            key: ep_key,
            state,
            source,
            confidence: series.confidence,
            note,
            ..Default::default()
        };
        let added = boot
            .as_ref()
            .and_then(|b| b.date_created)
            .unwrap_or(f.mtime / 1_000_000_000)
            .min(now());
        let w = EpisodeWrite {
            lib: f.library_id.clone(),
            fid: f.id.clone(),
            series: series.clone(),
            series_title: f.parsed.series_title.clone().unwrap_or_default(),
            series_year: f.parsed.series_year,
            season_key,
            season_no: season.unwrap_or(0),
            ident,
            episode,
            episode_end,
            ep_title,
            part_no,
            added,
            duration: f.duration,
        };
        self.db.call(move |c| store_episode(c, w)).await
    }

    /// The episode's series: by its manual key (`local`, a show, an episode of one), else found.
    async fn episode_series(&self, f: &FileRow, manual: Option<&str>, cache: &mut HashMap<String, Ident>) -> Result<Ident> {
        let series = match manual {
            Some("local" | "local:episode") => {
                let folder = f.parsed.series_folder.clone().unwrap_or_default();
                Ident {
                    key: format!("path:{}/{folder}", f.library_id),
                    state: "manual",
                    source: "local".into(),
                    confidence: 1.0,
                    ..Default::default()
                }
            }
            Some(k) if k.starts_with("tmdb:tv:") && !k.contains(":s") => Ident {
                key: k.to_string(),
                state: "manual",
                source: "admin".into(),
                confidence: 1.0,
                note: None,
                ..Default::default()
            },
            Some(k) if k.contains(":s") => {
                let base = k[..k.rfind(":s").unwrap()].to_string();
                Ident {
                    key: base,
                    state: "manual",
                    source: "admin".into(),
                    confidence: 1.0,
                    note: None,
                    ..Default::default()
                }
            }
            _ => self.identify_series(f, cache).await?,
        };
        Ok(series)
    }

    /// The episode's season, episode and last episode: the name's, the old import's (with a
    /// note when they differ), a manual key's, or TMDB's numbering of the name's.
    async fn episode_numbers(
        &self,
        f: &FileRow,
        boot: Option<&Boot>,
        manual: Option<&String>,
        series: &Ident,
    ) -> (Option<i32>, Option<i32>, Option<i32>, Option<String>) {
        let mut note = None;
        let (mut season, mut episode, mut episode_end) = (f.parsed.season, f.parsed.episode, f.parsed.episode_end);
        if let Some(b) = boot {
            if b.season.is_some() && b.episode.is_some() && (b.season, b.episode) != (season, episode) {
                if season.is_some() && episode.is_some() {
                    note = Some(format!(
                        "The import numbers S{:?}E{:?}; the name says S{season:?}E{episode:?}",
                        b.season, b.episode
                    ));
                }
                season = b.season;
                episode = b.episode;
                episode_end = b.episode_end;
            } else if b.episode_end.is_some() {
                episode_end = b.episode_end;
            }
        }
        if let Some(k) = manual.map(String::as_str).filter(|k| k.contains(":s")) {
            if let Some((s, e)) = k[k.rfind(":s").unwrap() + 2..].split_once('e') {
                season = s.parse().ok();
                episode = e.parse().ok();
            }
        }
        if manual.is_none() && boot.is_none_or(|b| b.episode.is_none()) {
            (season, episode) = self.tmdb_numbers(&f.parsed, series, season, episode).await;
        }
        if season.is_none() && episode.is_some() {
            season = Some(1);
        }
        (season, episode, episode_end, note)
    }

    /// Whether TMDB's list of the season has the episode (true without TMDB).
    async fn tmdb_lists(&self, series: &Ident, s: i32, e: i32) -> bool {
        if let (Some(id), Some(tmdb)) = (series.tmdb_id(), self.tmdb()) {
            let sv = tmdb.season(id, s as i64, 7 * super::tmdb::DAY).await.unwrap_or(Value::Null);
            let known = sv["episodes"]
                .as_array()
                .is_some_and(|eps| eps.iter().any(|x| x["episode_number"].as_i64() == Some(e as i64)));
            return known;
        }
        true
    }

    /// The item an earlier alignment with TMDB's list placed the file in (`parts.rs`), if its
    /// key starts with `prefix`: its key, episode, last episode and note.
    async fn aligned_placement(&self, file_id: &str, prefix: String) -> Result<Option<(String, Option<i32>, Option<i32>, Option<String>)>> {
        let fid = file_id.to_string();
        let placed: Option<(String, Option<i32>, Option<i32>, Option<String>)> = self
            .db
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT i.key, i.episode_no, i.episode_end, i.identify_note FROM files f JOIN items i ON i.id = f.item_id
                         WHERE f.id = ?1 AND i.identify_source = 'aligned' AND i.key LIKE ?2 || '%'",
                    params![fid, prefix],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?)
            })
            .await?;
        Ok(placed)
    }

    /// The part an episode file's name marks (`(2)`, `Pt.2`, `Part Two` at the end of its title)
    /// when another file in its folder has the same numbers and another part marker.
    async fn part_beside(&self, f: &FileRow) -> Result<Option<i32>> {
        let mine = f.parsed.part.or_else(|| super::parts::split_part(&f.parsed.title).1);
        let (Some(m), Some(s), Some(e)) = (mine, f.parsed.season, f.parsed.episode) else {
            return Ok(None);
        };
        let Some((dir, _)) = f.path.rsplit_once('/') else { return Ok(None) };
        let (dir, id) = (dir.to_string(), f.id.clone());
        let others: Vec<String> = self
            .db
            .call(move |c| {
                let mut st = c.prepare(&format!(
                    "SELECT parsed FROM files WHERE substr(path, 1, length(?1) + 1) = ?1 || '/' AND instr(substr(path, length(?1) + 2), '/') = 0
                     AND id != ?2 AND role IN {} AND missing_since IS NULL AND parsed IS NOT NULL",
                    super::TITLE_ROLES
                ))?;
                let rows = st.query_map(params![dir, id], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
                Ok(rows)
            })
            .await?;
        let other_parts = others.iter().filter_map(|p| serde_json::from_str::<Parsed>(p).ok()).any(|p| {
            let marker = p.part.or_else(|| super::parts::split_part(&p.title).1);
            p.season == Some(s) && p.episode == Some(e) && p.episode_end.is_none() && marker.is_some_and(|x| x != m)
        });
        Ok(other_parts.then_some(m))
    }

    /// Items left without files go (with a redirect to where their files went); empty seasons
    /// and series too.
    pub async fn cleanup_items(&self) -> Result<()> {
        self.db
            .call(|c| {
                let tx = c.transaction()?;
                let n1 = tx.execute("DELETE FROM items WHERE kind IN ('movie', 'episode') AND NOT EXISTS (SELECT 1 FROM files f WHERE f.item_id = items.id)", [])?;
                let n2 = tx.execute("DELETE FROM items WHERE kind = 'season' AND NOT EXISTS (SELECT 1 FROM items e WHERE e.parent_id = items.id)", [])?;
                let n3 = tx.execute("DELETE FROM items WHERE kind = 'series' AND NOT EXISTS (SELECT 1 FROM items s WHERE s.parent_id = items.id)", [])?;
                // A part left alone (its other parts matched elsewhere) is the whole episode again.
                tx.execute(
                    "UPDATE items SET part_no = NULL WHERE kind = 'episode' AND part_no IS NOT NULL AND identify_source != 'aligned'
                     AND NOT EXISTS (SELECT 1 FROM items o WHERE o.kind = 'episode' AND o.parent_id = items.parent_id AND o.episode_no = items.episode_no
                                     AND o.id != items.id AND o.part_no IS NOT NULL)",
                    [],
                )?;
                tx.execute("DELETE FROM item_people WHERE NOT EXISTS (SELECT 1 FROM items i WHERE i.id = item_people.item_id)", [])?;
                tx.commit()?;
                if n1 + n2 + n3 > 0 {
                    tracing::info!(titles = n1, seasons = n2, series = n3, "items without files removed");
                }
                Ok(())
            })
            .await
    }
}

/// Every title TMDB knows for a movie or show: its own, the original, translations,
/// alternative titles.
pub fn all_titles(d: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for k in ["title", "name", "original_title", "original_name"] {
        if let Some(s) = d[k].as_str() {
            out.push(s.to_string());
        }
    }
    for t in d["alternative_titles"]["titles"]
        .as_array()
        .into_iter()
        .chain(d["alternative_titles"]["results"].as_array())
        .flatten()
    {
        if let Some(s) = t["title"].as_str() {
            out.push(s.to_string());
        }
    }
    for t in d["translations"]["translations"].as_array().into_iter().flatten() {
        for k in ["title", "name"] {
            if let Some(s) = t["data"][k].as_str().filter(|s| !s.is_empty()) {
                out.push(s.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// A TMDB search result as Fix Match lists it.
pub fn candidate_json(kind: &str, r: &Value) -> Value {
    let (t, ot, d) = if kind == "movie" {
        ("title", "original_title", "release_date")
    } else {
        ("name", "original_name", "first_air_date")
    };
    json!({"tmdb_id": r["id"], "title": r[t], "original_title": r[ot], "year": year_of(&r[d]),
           "overview": r["overview"].as_str().map(|o| o.chars().take(200).collect::<String>()),
           "poster": r["poster_path"].as_str().map(|p| format!("https://image.tmdb.org/t/p/w342{p}"))})
}

impl Library {
    /// Fix Match's candidates for a movie or show without stored ones (titles that went to
    /// review before candidates were kept): TMDB searched now from its first file's name, as
    /// identification would, best first.
    pub async fn match_candidates(&self, item_id: &str) -> Result<Vec<Value>> {
        let id = item_id.to_string();
        let file_id: Option<String> = self
            .db
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT f.id FROM files f WHERE f.missing_since IS NULL AND (f.item_id = ?1 OR f.item_id IN (SELECT id FROM items WHERE series_id = ?1)) ORDER BY f.path LIMIT 1",
                    [&id],
                    |r| r.get(0),
                )
                .optional()?)
            })
            .await?;
        let Some(fid) = file_id.filter(|f| f.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')) else {
            return Ok(Vec::new());
        };
        let Some((f, _, _)) = self.file_rows(&format!("id = '{fid}'")).await?.into_iter().next() else {
            return Ok(Vec::new());
        };
        let cands = if f.parsed.kind == "episode" {
            let readings = vec![(f.parsed.series_title.clone().unwrap_or_default(), f.parsed.series_year)];
            self.candidates("tv", &readings, 1, None, false).await?
        } else {
            let own_folder = self.own_folder(&f).await;
            let (readings, strong) = movie_readings(&f.parsed, own_folder.as_deref());
            self.candidates("movie", &readings, strong, f.duration, is_cut(&f.rel)).await?
        };
        let ident = Ident {
            state: "review",
            candidates: cands,
            ..Default::default()
        };
        Ok(review_candidates(&ident)
            .and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
            .unwrap_or_default())
    }
}

/// The candidates kept for Fix Match (None unless the title went to review).
fn review_candidates(ident: &Ident) -> Option<String> {
    (ident.state == "review" && !ident.candidates.is_empty()).then(|| {
        json!(ident
            .candidates
            .iter()
            .take(6)
            .map(|c| json!({"tmdb_id": c.id, "title": c.title, "original_title": c.original_title, "year": c.year, "overview": c.overview,
                "score": (c.score * 1000.0).round() / 1000.0, "poster": c.poster.as_ref().map(|p| format!("https://image.tmdb.org/t/p/w342{p}"))}))
            .collect::<Vec<_>>())
        .to_string()
    })
}

/// The deepest folder all of these paths are in.
fn common_folder(paths: &[String]) -> String {
    let Some(first) = paths.first() else { return String::new() };
    let mut parts: Vec<&str> = first.split('/').collect();
    for p in &paths[1..] {
        let other: Vec<&str> = p.split('/').collect();
        let n = parts.iter().zip(&other).take_while(|(a, b)| a == b).count();
        parts.truncate(n);
    }
    if paths.len() == 1 {
        parts.pop();
    }
    parts.join("/")
}

/// A longer cut, as its name says (its runtime may run well over TMDB's).
fn is_cut(rel: &str) -> bool {
    let l = rel.to_lowercase();
    ["extended", "redux", "director", "cut", "unrated", "uncut", "special edition"]
        .iter()
        .any(|w| l.contains(w))
}

/// How much more popular the most popular of several exact titles must be to be taken.
const POPULAR_LEAD: f64 = 3.0;

/// The candidate to take, if one is clear: (id, score). Either the best scores high enough and
/// well ahead of the next, or exactly one candidate's title is the file's own with a fitting
/// year (or runtime), or several are and one is far more popular than the others (remakes and
/// namesakes: "Chicago" 2002, the musical, not a 2002 short of the same name).
fn accept(cands: &[Candidate], _state: &str) -> Option<(i64, f64)> {
    let best = cands.first()?;
    let lead = best.score - cands.get(1).map_or(0.0, |c| c.score);
    if best.score >= ACCEPT_SCORE && lead >= ACCEPT_LEAD {
        return Some((best.id, best.score));
    }
    let mut fits: Vec<&Candidate> = cands.iter().filter(|c| c.fits).collect();
    fits.sort_by(|a, b| b.popularity.total_cmp(&a.popularity));
    match fits.as_slice() {
        [one] => Some((one.id, one.score)),
        [first, second, ..] if first.popularity > 0.0 && first.popularity >= POPULAR_LEAD * second.popularity => {
            Some((first.id, first.score))
        }
        _ => None,
    }
}

/// How a blind identification compares with the one in the database: `same`, `review` (no
/// guess: Fix Match), `wrong` (another title), `unknown_now` (the database has no TMDB identity
/// either), `local` (no TMDB key).
fn verdict_of(current: &str, blind: &Ident) -> &'static str {
    if !current.starts_with("tmdb:") {
        return if blind.key.starts_with("tmdb:") {
            "found_new"
        } else {
            "unknown_now"
        };
    }
    match blind.state {
        "local" => "local",
        "review" => "review",
        _ if blind.key == current => "same",
        _ => "wrong",
    }
}

fn review(f: &FileRow, why: &str) -> Ident {
    Ident {
        key: format!("path:{}/{}", f.library_id, f.rel),
        state: "review",
        source: "parse".into(),
        confidence: 0.0,
        note: Some(why.into()),
        ..Default::default()
    }
}

#[derive(Default)]
pub struct ItemSpec<'a> {
    pub key: &'a str,
    pub kind: &'a str,
    pub library_id: &'a str,
    pub parent_id: Option<&'a str>,
    pub series_id: Option<&'a str>,
    pub season_no: Option<i32>,
    pub episode_no: Option<i32>,
    pub episode_end: Option<i32>,
    pub title: &'a str,
    pub year: Option<i32>,
    pub added_at: i64,
    /// A part of an episode (1…), chosen by hand.
    pub part_no: Option<i32>,
    /// The file that will be this item's: a new item takes back the id the file's title had before
    /// it was merged into the one it is on now (undoing a Fix Match keeps ids and watch state).
    pub revive_for: Option<&'a str>,
}

/// The item with this identity (made if new). An existing item keeps its id; its state is
/// updated unless it was set by hand.
pub fn ensure_item(tx: &rusqlite::Transaction, s: &ItemSpec, ident: &Ident) -> Result<String> {
    let now = now();
    let existing: Option<(String, String, i64)> = tx
        .query_row("SELECT id, identify_state, added_at FROM items WHERE key = ?1", [s.key], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    // A season or series isn't "review" because one episode is.
    let state = if (s.kind == "series" || s.kind == "season") && ident.state == "review" && !s.key.starts_with("path:") {
        "matched"
    } else {
        ident.state
    };
    match existing {
        Some((id, old_state, added)) => {
            if old_state != "manual" || state == "manual" {
                tx.execute(
                    "UPDATE items SET identify_state = ?2, identify_source = ?3, confidence = ?4, identify_note = ?5, updated_at = ?6 WHERE id = ?1
                     AND (identify_state != ?2 OR COALESCE(identify_note, '') != COALESCE(?5, ''))",
                    params![id, state, ident.source, ident.confidence, ident.note, now],
                )?;
            }
            if !s.title.is_empty() {
                // A placeholder title (the key) gives way to a real one.
                tx.execute(
                    "UPDATE items SET title = ?2, sort_title = ?3 WHERE id = ?1 AND (title LIKE 'tmdb:%' OR title LIKE 'path:%')",
                    params![id, s.title, util::sort_title(s.title)],
                )?;
            }
            tx.execute(
                "UPDATE items SET parent_id = COALESCE(?2, parent_id), series_id = COALESCE(?3, series_id),
                 episode_end = COALESCE(?4, episode_end), added_at = MIN(added_at, ?5), part_no = COALESCE(?6, part_no) WHERE id = ?1",
                params![id, s.parent_id, s.series_id, s.episode_end, s.added_at.min(added), s.part_no],
            )?;
            Ok(id)
        }
        None => {
            let id = match s.revive_for {
                Some(f) => revived_id(tx, f)?.unwrap_or_else(util::ulid),
                None => util::ulid(),
            };
            let title = if s.title.is_empty() {
                s.key.rsplit('/').next().unwrap_or(s.key)
            } else {
                s.title
            };
            tx.execute(
                "INSERT INTO items(id, kind, library_id, key, parent_id, series_id, season_no, episode_no, episode_end, title, sort_title, year,
                                   identify_state, identify_source, confidence, identify_note, added_at, updated_at, part_no)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
                params![
                    id, s.kind, s.library_id, s.key, s.parent_id, s.series_id, s.season_no, s.episode_no, s.episode_end, title,
                    util::sort_title(title), s.year, state, ident.source, ident.confidence, ident.note, s.added_at, now, s.part_no
                ],
            )?;
            if let Some(f) = s.revive_for {
                // Its viewings go with it (the file's sessions on the title it leaves).
                tx.execute(
                    "UPDATE play_sessions SET item_id = ?2 WHERE file_id = ?1 AND item_id = (SELECT item_id FROM files WHERE id = ?1)",
                    params![f, id],
                )?;
            }
            Ok(id)
        }
    }
}

/// The id a new item for this file takes back: the file's title was merged into the one it is on
/// now (that one keeps other files), and exactly one merged id points there that is no item now.
fn revived_id(tx: &rusqlite::Transaction, file_id: &str) -> Result<Option<String>> {
    let Some(cur) = tx
        .query_row("SELECT item_id FROM files WHERE id = ?1", [file_id], |r| {
            r.get::<_, Option<String>>(0)
        })
        .optional()?
        .flatten()
    else {
        return Ok(None);
    };
    let others: i64 = tx.query_row(
        &format!(
            "SELECT COUNT(*) FROM files WHERE item_id = ?1 AND id != ?2 AND role IN {} AND missing_since IS NULL",
            super::TITLE_ROLES
        ),
        params![cur, file_id],
        |r| r.get(0),
    )?;
    if others == 0 {
        return Ok(None);
    }
    let mut st =
        tx.prepare("SELECT old_id FROM item_redirects WHERE new_id = ?1 AND NOT EXISTS (SELECT 1 FROM items WHERE id = old_id)")?;
    let olds: Vec<String> = st.query_map([&cur], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let [old] = olds.as_slice() else { return Ok(None) };
    tx.execute("DELETE FROM item_redirects WHERE old_id = ?1", [old])?;
    tracing::info!(item = %old, from = %cur, "a file leaves the title it was merged into: its old id again");
    Ok(Some(old.clone()))
}

/// Points a file at its item; if that moves it off another item which is left empty, the
/// old id redirects to the new one.
pub fn assign(tx: &rusqlite::Transaction, file_id: &str, item_id: &str) -> Result<()> {
    let old: Option<String> = tx
        .query_row("SELECT item_id FROM files WHERE id = ?1", [file_id], |r| r.get(0))
        .optional()?
        .flatten();
    tx.execute(
        "UPDATE files SET item_id = ?2, identified_at = ?3 WHERE id = ?1",
        params![file_id, item_id, now()],
    )?;
    if let Some(old) = old.filter(|o| o != item_id) {
        let left: i64 = tx.query_row("SELECT COUNT(*) FROM files WHERE item_id = ?1", [&old], |r| r.get(0))?;
        if left == 0 {
            tx.execute(
                "INSERT OR REPLACE INTO item_redirects(old_id, new_id) VALUES (?1, ?2)",
                params![old, item_id],
            )?;
            tx.execute("UPDATE item_redirects SET new_id = ?2 WHERE new_id = ?1", params![old, item_id])?;
            // Watch state follows the merge.
            crate::userdata::follow_redirect(tx, &old, item_id)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: i64, title: &str, year: i32) -> Candidate {
        Candidate {
            id,
            title: title.into(),
            original_title: title.into(),
            year: Some(year),
            runtime_minutes: None,
            overview: String::new(),
            seasons: None,
            score: 0.0,
            popularity: 0.0,
            fits: false,
            poster: None,
        }
    }

    #[test]
    fn scoring_and_acceptance() {
        let r = vec![("Kong Skull Island".to_string(), None)];
        let mut cs = vec![cand(1, "Kong: Skull Island", 2017), cand(2, "King Kong", 2005)];
        for c in &mut cs {
            c.score = score(&r, 1, c);
        }
        cs.sort_by(|a, b| b.score.total_cmp(&a.score));
        // No year: exact title alone isn't enough (0.65 + 0.175).
        assert!(accept(&cs, "x").is_none());
        let r = vec![("Taxi 2".to_string(), Some(2000))];
        let mut cs = vec![cand(1, "Taxi 2", 2000), cand(2, "Taxi", 1998), cand(3, "Taxi 3", 2003)];
        for c in &mut cs {
            c.score = score(&r, 1, c);
        }
        cs.sort_by(|a, b| b.score.total_cmp(&a.score));
        assert_eq!(accept(&cs, "x").map(|a| a.0), Some(1));
    }

    #[test]
    fn readings_of_names() {
        let p = |t: &str, y: Option<i32>, alt: &[(&str, Option<i32>)]| super::super::parse::Parsed {
            kind: "movie".into(),
            title: t.into(),
            year: y,
            alt_titles: alt.iter().map(|(a, b)| (a.to_string(), *b)).collect(),
            ..Default::default()
        };
        let (r, n) = movie_readings(&p("Troy Director's Cut", Some(2004), &[]), None);
        assert_eq!((r[1].0.as_str(), r[1].1, n), ("Troy", Some(2004), 2));
        let (r, _) = movie_readings(
            &p("Star Wars Episode V - The Empire Strikes Back", Some(1980), &[("Star Wars", None)]),
            None,
        );
        assert!(r.iter().any(|x| x.0 == "The Empire Strikes Back"));
        assert_eq!(r.last().unwrap().0, "Star Wars", "the folder's reading comes last");
        let (r, n) = movie_readings(&p("Star Trek (IX) Insurrection", None, &[("Star Trek", None), ("IX", None)]), None);
        assert!(r[..n].iter().any(|x| x.0 == "Star Trek Insurrection"));
        assert!(!r[..n].iter().any(|x| x.0 == "Star Trek"), "a fragment isn't the file's own title");
        let (r, _) = movie_readings(&p("Унесенные призраками(2001г", None, &[]), None);
        assert_eq!((r[0].0.as_str(), r[0].1), ("Унесенные призраками", Some(2001)));
        let (_, n) = movie_readings(
            &p(
                "Апокалипсис сегодня Apocalypse Now",
                Some(1979),
                &[("Апокалипсис сегодня", Some(1979)), ("Apocalypse Now", Some(1979))],
            ),
            None,
        );
        assert_eq!(n, 3, "a second script of the same name is the file's own");
        let poc = p(
            "PoC - At Worlds End",
            None,
            &[
                ("Pirates of the Caribbean - At Worlds End", None),
                ("Pirates of the Caribbean - Trilogy", None),
            ],
        );
        let (r, n) = movie_readings(&poc, Some("Pirates of the Caribbean - At Worlds End"));
        assert!(
            r[..n].iter().any(|x| x.0 == "Pirates of the Caribbean - At Worlds End"),
            "its own folder"
        );
        assert!(!r[..n].iter().any(|x| x.0.contains("Trilogy")), "not the collection's");
        let (r, n) = movie_readings(&poc, None);
        assert!(
            !r[..n].iter().any(|x| x.0.starts_with("Pirates")),
            "a folder shared with other titles is weak"
        );
    }

    #[test]
    fn exact_titles_decide() {
        let mut a = cand(1, "Avatar", 2009);
        let mut b = cand(2, "Avatar: Scene Deconstruction", 2009);
        a.score = 1.0;
        b.score = 0.935;
        a.fits = true;
        assert_eq!(accept(&[a.clone(), b.clone()], "x").map(|x| x.0), Some(1), "the only exact title");
        // Two of the same name: the far more popular one, else review.
        let mut c = cand(3, "Avatar", 2009);
        c.fits = true;
        c.score = 1.0;
        a.popularity = 50.0;
        c.popularity = 2.0;
        assert_eq!(accept(&[a.clone(), c.clone()], "x").map(|x| x.0), Some(1));
        c.popularity = 30.0;
        assert_eq!(accept(&[a, c], "x"), None);
        b.score = 0.8;
        assert!(accept(&[b], "x").is_none(), "no exact title, a low score: review");
    }

    #[test]
    fn items_and_redirects() {
        let mut c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch(include_str!("../../migrations/001_init.sql")).unwrap();
        c.execute_batch(include_str!("../../migrations/002_library.sql")).unwrap();
        c.execute_batch(include_str!("../../migrations/004_users.sql")).unwrap();
        c.execute_batch(include_str!("../../migrations/011_parts.sql")).unwrap();
        c.execute("INSERT INTO files(id, library_id, path, size, mtime, first_seen, last_seen, stable_since) VALUES ('f1', 'movies', '/m/a.mkv', 1, 1, 0, 0, 0)", []).unwrap();
        c.execute(
            "INSERT INTO users(id, name, password_hash, created_at) VALUES ('u1', 'sam', 'x', 0)",
            [],
        )
        .unwrap();
        let tx = c.transaction().unwrap();
        let id = Ident {
            key: "path:movies/a.mkv".into(),
            state: "review",
            source: "parse".into(),
            confidence: 0.0,
            note: None,
            ..Default::default()
        };
        let a = ensure_item(
            &tx,
            &ItemSpec {
                key: &id.key,
                kind: "movie",
                library_id: "movies",
                title: "A",
                ..Default::default()
            },
            &id,
        )
        .unwrap();
        assign(&tx, "f1", &a).unwrap();
        tx.execute(
            "INSERT INTO user_data(user_id, item_id, position, updated_at) VALUES ('u1', ?1, 300, 0)",
            [&a],
        )
        .unwrap();
        let id2 = Ident {
            key: "tmdb:movie:5".into(),
            state: "manual",
            source: "admin".into(),
            confidence: 1.0,
            note: None,
            ..Default::default()
        };
        let b = ensure_item(
            &tx,
            &ItemSpec {
                key: &id2.key,
                kind: "movie",
                library_id: "movies",
                title: "B",
                ..Default::default()
            },
            &id2,
        )
        .unwrap();
        assign(&tx, "f1", &b).unwrap();
        let to: String = tx
            .query_row("SELECT new_id FROM item_redirects WHERE old_id = ?1", [&a], |r| r.get(0))
            .unwrap();
        assert_eq!(to, b);
        // The watch state moved with the merge.
        let pos: f64 = tx
            .query_row("SELECT position FROM user_data WHERE item_id = ?1", [&b], |r| r.get(0))
            .unwrap();
        assert_eq!(pos, 300.0);
        // The same key again is the same item; a manual state isn't overridden.
        let again = ensure_item(
            &tx,
            &ItemSpec {
                key: &id2.key,
                kind: "movie",
                library_id: "movies",
                title: "B",
                ..Default::default()
            },
            &id,
        )
        .unwrap();
        assert_eq!(again, b);
        let st: String = tx
            .query_row("SELECT identify_state FROM items WHERE id = ?1", [&b], |r| r.get(0))
            .unwrap();
        assert_eq!(st, "manual");
        tx.commit().unwrap();
    }

    #[test]
    fn a_merged_file_takes_its_old_id_back() {
        // Fix Match merged episode 24's file into episode 23 (another version of it); undone, a new
        // item for that file is episode 24's old id again, with its viewings.
        let mut c = rusqlite::Connection::open_in_memory().unwrap();
        for m in [
            include_str!("../../migrations/001_init.sql"),
            include_str!("../../migrations/002_library.sql"),
            include_str!("../../migrations/004_users.sql"),
            include_str!("../../migrations/011_parts.sql"),
        ] {
            c.execute_batch(m).unwrap();
        }
        for f in ["f23", "f24"] {
            c.execute("INSERT INTO files(id, library_id, path, size, mtime, role, first_seen, last_seen, stable_since) VALUES (?1, 'tv', '/t/' || ?1, 1, 1, 'main', 0, 0, 0)", [f]).unwrap();
        }
        c.execute(
            "INSERT INTO users(id, name, password_hash, created_at) VALUES ('u', 'sam', 'x', 0)",
            [],
        )
        .unwrap();
        let tx = c.transaction().unwrap();
        let ep = |k: &str| Ident {
            key: k.into(),
            state: "bootstrap",
            source: "bootstrap".into(),
            ..Default::default()
        };
        let spec = |k: &'static str| ItemSpec {
            key: k,
            kind: "episode",
            library_id: "tv",
            title: k,
            ..Default::default()
        };
        let e23 = ensure_item(&tx, &spec("tmdb:tv:1:s4e23"), &ep("tmdb:tv:1:s4e23")).unwrap();
        let e24 = ensure_item(&tx, &spec("tmdb:tv:1:s4e24"), &ep("tmdb:tv:1:s4e24")).unwrap();
        assign(&tx, "f23", &e23).unwrap();
        assign(&tx, "f24", &e24).unwrap();
        tx.execute(
            "INSERT INTO play_sessions(id, user_id, item_id, file_id, started_at) VALUES ('s1', 'u', ?1, 'f24', 0)",
            [&e24],
        )
        .unwrap();
        assign(&tx, "f24", &e23).unwrap(); // the mistake
        tx.execute("DELETE FROM items WHERE id = ?1", [&e24]).unwrap();
        let back = ensure_item(
            &tx,
            &ItemSpec {
                revive_for: Some("f24"),
                ..spec("tmdb:tv:1:s4e24")
            },
            &ep("tmdb:tv:1:s4e24"),
        )
        .unwrap();
        assert_eq!(back, e24);
        assign(&tx, "f24", &back).unwrap();
        let redirects: i64 = tx.query_row("SELECT COUNT(*) FROM item_redirects", [], |r| r.get(0)).unwrap();
        assert_eq!(redirects, 0);
        let s1: String = tx
            .query_row("SELECT item_id FROM play_sessions WHERE id = 's1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(s1, e24);
        // A brand-new title for a file alone on its item gets a new id.
        let other = ensure_item(
            &tx,
            &ItemSpec {
                revive_for: Some("f23"),
                ..spec("tmdb:tv:1:s4e22")
            },
            &ep("tmdb:tv:1:s4e22"),
        )
        .unwrap();
        assert!(other != e23 && other != e24);
    }

    #[test]
    fn all_titles_transliterated() {
        let d = json!({"title": "Jurassic Park III", "translations": {"translations": [{"data": {"title": "Парк Юрского периода 3"}}]}});
        let t = all_titles(&d);
        assert!(similarity("Park Yurskogo Perioda 3", t.iter().find(|x| x.starts_with("Парк")).unwrap()) > 0.9);
    }
}
