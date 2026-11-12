//! Local metadata: what the files and folders say themselves, for titles TMDB doesn't cover
//! (no TMDB key, a name TMDB can't match, an episode TMDB doesn't list) and for art TMDB lacks.
//!
//! - **NFO files** (Kodi's: `<movie>`, `<tvshow>`, `<episodedetails>`): title, original title,
//!   year, plot, tagline, genres, studios, rating, premiere date, and ids (`<uniqueid
//!   type="tmdb|imdb">`, `<tmdbid>`, `<imdbid>`, or a TMDB/IMDb link), which identification
//!   uses before searching. Movie: `<file>.nfo`, else `movie.nfo` in a folder of its own; show:
//!   `tvshow.nfo`; episode: `<file>.nfo`.
//! - **The file's own tags** (from the stored probe, no I/O): title, date or year, description.
//! - **Artwork in the folders**, as media servers name it: movie `<file>-poster.jpg`, and in a
//!   folder of its own `poster.jpg`, `folder.jpg`, `cover.jpg`; backdrops `<file>-fanart.jpg`,
//!   `fanart.jpg`, `backdrop.jpg`, `background.jpg`; logos `<file>-logo.png`, `logo.png`,
//!   `clearlogo.png`; show folders the same; seasons `season01-poster.jpg` (`season-specials-…`)
//!   in the show's folder or `poster.jpg`/`folder.jpg` in the season's; episodes
//!   `<file>-thumb.jpg` or `<file>.jpg`. (`.png`, `.jpeg`, `.webp`… too.)
//! - **Cover art inside the file**: an MP4/MKV attached picture, or a Matroska image attachment.
//! - **A frame of the video** at 10 % of its length, for an episode's thumbnail and a movie's
//!   backdrop when nothing else has one (HDR tone-mapped).
//!
//! Images from these sources are registered like TMDB's (`images.rs`: `file:`, `embed:`,
//! `frame:` sources; the file's mtime is part of the source, so a changed file is a new image)
//! and made on first use or by the background prefetch. TMDB's values always win; for a title
//! TMDB describes, only missing art is filled in. Each title is looked at once per change
//! (`items.local_at`).

use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use super::{
    images,
    util::{now, sort_title},
    Library, TITLE_ROLES,
};

/// What an NFO file says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Nfo {
    pub title: Option<String>,
    pub original_title: Option<String>,
    pub year: Option<i32>,
    pub plot: Option<String>,
    pub tagline: Option<String>,
    pub genres: Vec<String>,
    pub studios: Vec<String>,
    pub rating: Option<f64>,
    pub premiered: Option<String>,
    pub tmdb: Option<i64>,
    pub imdb: Option<String>,
    /// What the NFO describes: `movie`, `tvshow` or `episodedetails`.
    pub root: Option<String>,
    /// An episode NFO's numbers.
    pub season: Option<i32>,
    pub episode: Option<i32>,
    /// A cut or edition (`<edition>`: "Director's Cut").
    pub edition: Option<String>,
}

static TMDB_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"themoviedb\.org/(?:movie|tv)/(\d+)").unwrap());
static IMDB_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(tt\d{6,10})\b").unwrap());
static UNIQUEID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)<uniqueid[^>]*type\s*=\s*"([a-z]+)"[^>]*>\s*([^<\s]+)\s*</uniqueid>"#).unwrap());

fn unescape(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix("<![CDATA[").and_then(|x| x.strip_suffix("]]>")).unwrap_or(s);
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .trim()
        .to_string()
}

/// Every `<tag>…</tag>` value (non-empty), in order.
fn tags(xml: &str, tag: &str) -> Vec<String> {
    let re = Regex::new(&format!(r"(?is)<{tag}(?:\s[^>]*)?>(.*?)</{tag}>")).unwrap();
    re.captures_iter(xml).map(|c| unescape(&c[1])).filter(|v| !v.is_empty()).collect()
}

fn tag(xml: &str, t: &str) -> Option<String> {
    tags(xml, t).into_iter().next()
}

/// Reads an NFO file: Kodi XML, or (as Kodi allows) just a TMDB/IMDb link.
pub fn parse_nfo(text: &str) -> Option<Nfo> {
    let mut n = Nfo::default();
    // Only the title's own part: an episode NFO can carry the show's details after it, a movie
    // NFO actors with their own <name>/<role>.
    let body = text.split("<actor").next().unwrap_or(text);
    let is_xml = ["<movie", "<tvshow", "<episodedetails"].iter().any(|r| text.contains(r));
    if is_xml {
        n.root = ["episodedetails", "tvshow", "movie"]
            .iter()
            .find(|r| text.contains(&format!("<{r}")))
            .map(ToString::to_string);
        if n.root.as_deref() == Some("episodedetails") {
            n.season = tag(body, "season").and_then(|v| v.parse().ok());
            n.episode = tag(body, "episode").and_then(|v| v.parse().ok());
        }
        n.title = tag(body, "title");
        n.original_title = tag(body, "originaltitle");
        n.plot = tag(body, "plot").or_else(|| tag(body, "outline"));
        n.tagline = tag(body, "tagline");
        n.edition = tag(body, "edition");
        n.genres = tags(body, "genre");
        n.studios = tags(body, "studio");
        n.premiered = tag(body, "premiered").or_else(|| tag(body, "aired")).filter(|d| d.len() >= 4);
        n.year = tag(body, "year")
            .and_then(|y| y.parse().ok())
            .or_else(|| n.premiered.as_deref().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok()));
        n.rating = tag(body, "rating")
            .and_then(|r| r.parse::<f64>().ok())
            .or_else(|| tag(body, "value").and_then(|r| r.parse::<f64>().ok()))
            .filter(|r| *r > 0.0 && *r <= 10.0);
        for c in UNIQUEID.captures_iter(body) {
            match c[1].to_ascii_lowercase().as_str() {
                "tmdb" => n.tmdb = c[2].parse().ok(),
                "imdb" => n.imdb = Some(c[2].to_string()),
                _ => {}
            }
        }
        n.tmdb = n.tmdb.or_else(|| tag(body, "tmdbid").and_then(|v| v.parse().ok()));
        n.imdb = n.imdb.or_else(|| tag(body, "imdbid")).or_else(|| tag(body, "imdb_id"));
    }
    n.tmdb = n.tmdb.or_else(|| TMDB_URL.captures(text).and_then(|c| c[1].parse().ok()));
    n.imdb = n
        .imdb
        .or_else(|| IMDB_ID.captures(text).map(|c| c[1].to_string()))
        .filter(|i| i.starts_with("tt"));
    (n != Nfo::default()).then_some(n)
}

const IMAGE_EXTS: [&str; 5] = ["jpg", "jpeg", "png", "webp", "tbn"];

/// The first of `names` (a folder's listing) that is one of the `stems` with an image
/// extension, compared case-insensitively.
pub fn find_image(names: &[String], stems: &[String]) -> Option<String> {
    for stem in stems {
        for n in names {
            if let Some((s, e)) = n.rsplit_once('.') {
                if s.eq_ignore_ascii_case(stem) && IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()) {
                    return Some(n.clone());
                }
            }
        }
    }
    None
}

fn find_name(names: &[String], wanted: &[String]) -> Option<String> {
    wanted
        .iter()
        .find_map(|w| names.iter().find(|n| n.eq_ignore_ascii_case(w)).cloned())
}

/// Art file names in a folder: (poster, backdrop, logo). `stem`: a movie's file stem (its own
/// `-poster` etc.); `whole_folder`: the folder's general names count too (a folder of its own,
/// a show's folder).
pub fn folder_art(names: &[String], stem: Option<&str>, whole_folder: bool) -> (Option<String>, Option<String>, Option<String>) {
    let list = |own: &[&str], general: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = stem.map(|s| own.iter().map(|o| format!("{s}-{o}")).collect()).unwrap_or_default();
        if whole_folder {
            v.extend(general.iter().map(ToString::to_string));
        }
        v
    };
    (
        find_image(
            names,
            &list(&["poster", "cover", "folder"], &["poster", "folder", "cover", "movie", "show"]),
        ),
        find_image(
            names,
            &list(&["fanart", "backdrop", "background"], &["fanart", "backdrop", "background", "art"]),
        ),
        find_image(names, &list(&["logo", "clearlogo"], &["logo", "clearlogo"])),
    )
}

/// A season's poster in the show's folder (`season01-poster.jpg`, `season1-poster.jpg`,
/// `season-specials-poster.jpg`) or in its own folder (`poster.jpg`, `folder.jpg`).
pub fn season_poster(show_names: &[String], season_names: &[String], season: i64) -> Option<String> {
    let mut stems = vec![format!("season{season:02}-poster"), format!("season{season}-poster")];
    if season == 0 {
        stems.push("season-specials-poster".into());
    }
    find_image(show_names, &stems).map(|n| format!("show/{n}")).or_else(|| {
        find_image(season_names, &["poster".to_string(), "folder".to_string(), "cover".to_string()]).map(|n| format!("season/{n}"))
    })
}

/// An episode's thumbnail next to it: `<stem>-thumb.jpg`, `<stem>.jpg`.
pub fn episode_thumb(names: &[String], stem: &str) -> Option<String> {
    find_image(names, &[format!("{stem}-thumb"), stem.to_string()])
}

/// The cover picture inside a file (from its probe): an attached picture, or a Matroska image
/// attachment (named cover/poster/folder first). Its stream index.
pub fn embedded_cover(probe: &Value) -> Option<i64> {
    let streams = probe["streams"].as_array()?;
    if let Some(s) = streams.iter().find(|s| s["disposition"]["attached_pic"].as_i64() == Some(1)) {
        return s["index"].as_i64();
    }
    let images: Vec<&Value> = streams
        .iter()
        .filter(|s| {
            s["codec_type"] == "attachment"
                && s["tags"]["mimetype"]
                    .as_str()
                    .is_some_and(|m| m.starts_with("image/jpeg") || m.starts_with("image/png"))
        })
        .collect();
    let named = |s: &&&Value| {
        s["tags"]["filename"]
            .as_str()
            .map(str::to_ascii_lowercase)
            .is_some_and(|f| ["cover", "poster", "folder"].iter().any(|w| f.starts_with(w)))
    };
    images.iter().find(named).or(images.first()).and_then(|s| s["index"].as_i64())
}

/// The file's own tags: (title, year, description).
pub fn embedded_tags(probe: &Value) -> (Option<String>, Option<i32>, Option<String>) {
    let t = &probe["format"]["tags"];
    let get = |keys: &[&str]| -> Option<String> {
        let o = t.as_object()?;
        keys.iter().find_map(|k| {
            o.iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(k))
                .and_then(|(_, v)| v.as_str())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        })
    };
    let title = get(&["title"]);
    let year = get(&["date_released", "date", "year", "creation_time"])
        .and_then(|d| d.get(..4).and_then(|y| y.parse::<i32>().ok()))
        .filter(|y| (1880..=2100).contains(y));
    // Encoders write tool names into `comment`: only a sentence counts.
    let desc = get(&["description", "synopsis", "summary"]).or_else(|| get(&["comment"]).filter(|c| c.split_whitespace().count() >= 6));
    (title, year, desc)
}

/// The probe says the video is HDR (PQ or HLG): the frame is tone-mapped.
fn is_hdr(probe: &Value) -> bool {
    probe["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|s| s["codec_type"] == "video" && matches!(s["color_transfer"].as_str(), Some("smpte2084" | "arib-std-b67")))
}

/// What one title gets from the local sources.
#[derive(Debug, Default)]
struct Patch {
    images: Value,
    title: Option<String>,
    original_title: Option<String>,
    year: Option<i32>,
    overview: Option<String>,
    tagline: Option<String>,
    genres: Vec<String>,
    studios: Vec<String>,
    rating: Option<f64>,
    premiered: Option<String>,
    runtime: Option<f64>,
    ids: Vec<(&'static str, String)>,
}

/// What the local sources gave for a title.
enum Local {
    /// Its file isn't probed yet: looked at again in a later pass.
    NotReady,
    Nothing,
    Found(Patch),
}

/// An item as the local pass sees it.
struct Row {
    id: String,
    kind: String,
    /// TMDB describes it (only missing art is filled in).
    tmdb: bool,
    images: Value,
    title: String,
    season_no: Option<i64>,
}

/// A file of the item: (path, rel to its library root, size, mtime, duration).
type FileInfo = (String, String, i64, i64, Option<f64>);

impl Library {
    /// Fills in what local sources say for titles that changed since they were last looked at
    /// (at most `limit` per call). Returns how many were updated.
    pub async fn refresh_local(&self, limit: usize) -> Result<usize> {
        let rows: Vec<(String, String, String, Option<String>, String, Option<i64>, Option<i64>)> = self
            .db
            .call(move |c| {
                let mut st = c.prepare(
                    "SELECT id, kind, key, images, title, season_no, refreshed_at FROM items
                     WHERE (local_at IS NULL OR (refreshed_at IS NOT NULL AND refreshed_at > local_at)
                            OR local_at < COALESCE((SELECT MAX(identified_at) FROM files f WHERE f.item_id = items.id), 0))
                       AND NOT (key LIKE 'tmdb:%' AND refreshed_at IS NULL)
                     ORDER BY CASE kind WHEN 'movie' THEN 0 WHEN 'series' THEN 1 WHEN 'season' THEN 2 ELSE 3 END, added_at DESC LIMIT ?1",
                )?;
                let rows = st
                    .query_map([limit as i64], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        if rows.is_empty() {
            return Ok(0);
        }
        let mut changed = 0;
        for (id, kind, key, images, title, season_no, _) in rows {
            let row = Row {
                id: id.clone(),
                kind,
                tmdb: key.starts_with("tmdb:"),
                images: images.and_then(|i| serde_json::from_str(&i).ok()).unwrap_or(json!({})),
                title,
                season_no,
            };
            let patch = match self.local_patch(&row).await {
                Ok(Local::NotReady) => continue,
                Ok(Local::Found(p)) => Some(p),
                Ok(Local::Nothing) => None,
                Err(e) => {
                    tracing::debug!(item = id, "local metadata: {e:#}");
                    None
                }
            };
            let updated = patch.is_some();
            self.db
                .call(move |c| {
                    let tx = c.transaction()?;
                    if let Some(p) = patch {
                        write_patch(&tx, &row, p)?;
                    }
                    tx.execute("UPDATE items SET local_at = ?2 WHERE id = ?1", params![id, now()])?;
                    tx.commit()?;
                    Ok(())
                })
                .await?;
            if updated {
                changed += 1;
            }
        }
        if changed > 0 {
            tracing::info!(items = changed, "local metadata and artwork");
        }
        Ok(changed)
    }

    /// The item's largest present file, or (series, season) one of its episodes'.
    async fn item_file(&self, id: &str, kind: &str) -> Result<Option<FileInfo>> {
        let (id, kind) = (id.to_string(), kind.to_string());
        let libs = self.libs();
        let row: Option<(String, String, i64, i64, Option<f64>)> = self
            .db
            .call(move |c| {
                let sql = match kind.as_str() {
                    "movie" | "episode" => format!("SELECT f.path, f.library_id, f.size, f.mtime, f.duration FROM files f WHERE f.item_id = ?1 AND f.role IN {TITLE_ROLES} AND f.missing_since IS NULL ORDER BY f.size DESC LIMIT 1"),
                    "series" => format!("SELECT f.path, f.library_id, f.size, f.mtime, f.duration FROM files f JOIN items e ON e.id = f.item_id WHERE e.series_id = ?1 AND f.role IN {TITLE_ROLES} AND f.missing_since IS NULL ORDER BY f.path LIMIT 1"),
                    _ => format!("SELECT f.path, f.library_id, f.size, f.mtime, f.duration FROM files f JOIN items e ON e.id = f.item_id WHERE e.parent_id = ?1 AND f.role IN {TITLE_ROLES} AND f.missing_since IS NULL ORDER BY f.path LIMIT 1"),
                };
                Ok(c.query_row(&sql, [&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).optional()?)
            })
            .await?;
        Ok(row.and_then(|(path, lib, size, mtime, dur)| {
            let l = libs.iter().find(|l| l.id == lib)?;
            let root = l.roots.iter().find(|r| path.starts_with(&format!("{r}/")))?;
            let rel = path[root.len() + 1..].to_string();
            Some((path, rel, size, mtime, dur))
        }))
    }

    async fn probe_of(&self, path: &str, size: i64, mtime: i64) -> Value {
        self.db
            .probe_get(path.to_string(), size as u64, mtime)
            .await
            .ok()
            .flatten()
            .map_or(Value::Null, |p| p.probe)
    }

    /// An NFO file's contents (None: absent or unreadable).
    pub(super) async fn read_nfo(&self, dir: &str, names: &[String], wanted: &[String]) -> Option<Nfo> {
        let name = find_name(names, wanted)?;
        let bytes = crate::storage::vfs::read(&crate::storage::vfs::join(dir, &name)).await.ok()?;
        if bytes.len() > 1 << 20 {
            return None;
        }
        parse_nfo(&String::from_utf8_lossy(&bytes))
    }

    /// The NFO next to a movie file (or `movie.nfo` in a folder of its own), or a show's
    /// `tvshow.nfo`: for identification by its ids.
    pub async fn nfo_for(&self, path: &str, rel: &str, kind: &str, own_folder: bool) -> Option<Nfo> {
        let dir = crate::storage::vfs::parent(path)?;
        let names = self.folder_names(dir.clone()).await;
        if kind == "series" {
            return self.read_nfo(&dir, &names, &["tvshow.nfo".to_string()]).await;
        }
        let stem = file_stem(rel);
        let mut wanted = vec![format!("{stem}.nfo")];
        if own_folder {
            wanted.push("movie.nfo".into());
        }
        self.read_nfo(&dir, &names, &wanted).await
    }

    /// A registered image for a file in `dir` (its mtime is part of the source).
    async fn file_image(&self, dir: &str, name: &str, kind: &'static str) -> Option<String> {
        let path = crate::storage::vfs::join(dir, name);
        let meta = crate::storage::vfs::stat(&path).await.ok()?;
        Some(self.register(format!("file:{}:{path}", meta.mtime), kind).await)
    }

    async fn register(&self, source: String, kind: &'static str) -> String {
        let id = images::image_id(&source);
        let i = id.clone();
        let _ = self
            .db
            .call(move |c| {
                c.execute(
                    "INSERT OR IGNORE INTO images(id, source, kind) VALUES (?1, ?2, ?3)",
                    params![i, source, kind],
                )?;
                Ok(())
            })
            .await;
        id
    }

    /// A frame at 10 % of the video, as an image.
    async fn frame_image(&self, f: &FileInfo, probe: &Value, kind: &'static str) -> Option<String> {
        let dur = f.4.filter(|d| *d > 0.0)?;
        let at = (dur * 0.1).round().max(1.0) as i64;
        Some(
            self.register(format!("frame:{at}:{}:{}:{}", u8::from(is_hdr(probe)), f.3, f.0), kind)
                .await,
        )
    }

    async fn local_patch(&self, row: &Row) -> Result<Local> {
        let Some(file) = self.item_file(&row.id, &row.kind).await? else {
            return Ok(Local::Nothing);
        };
        let (path, rel) = (&file.0, &file.1);
        let Some(dir) = crate::storage::vfs::parent(path) else {
            return Ok(Local::Nothing);
        };
        // A title's file not probed yet (no duration, no tags): later.
        if matches!(row.kind.as_str(), "movie" | "episode") && file.4.is_none() {
            return Ok(Local::NotReady);
        }
        let mut p = Patch {
            images: row.images.clone(),
            ..Default::default()
        };
        let mut changed = false;
        let missing = |v: &Value, k: &str| v[k].is_null() || v[k].as_array().is_some_and(Vec::is_empty) || v[k].as_str() == Some("");
        match row.kind.as_str() {
            "movie" => {
                let names = self.folder_names(dir.clone()).await;
                let stem = file_stem(rel);
                let own = rel.contains('/') && self.only_title_in(&dir).await;
                let probe = self.probe_of(path, file.2, file.3).await;
                let (poster, backdrop, logo) = folder_art(&names, Some(&stem), own);
                if missing(&p.images, "poster") {
                    let img = match poster {
                        Some(n) => self.file_image(&dir, &n, "poster").await,
                        None => match embedded_cover(&probe) {
                            Some(idx) => Some(self.register(format!("embed:{idx}:{}:{path}", file.3), "poster").await),
                            None => None,
                        },
                    };
                    if let Some(i) = img {
                        p.images["poster"] = json!(i);
                        changed = true;
                    }
                }
                if missing(&p.images, "backdrop") {
                    let img = match backdrop {
                        Some(n) => self.file_image(&dir, &n, "backdrop").await,
                        None => self.frame_image(&file, &probe, "backdrop").await,
                    };
                    if let Some(i) = img {
                        p.images["backdrop"] = json!([i]);
                        changed = true;
                    }
                }
                if missing(&p.images, "logo") {
                    if let Some(n) = logo {
                        if let Some(i) = self.file_image(&dir, &n, "logo").await {
                            p.images["logo"] = json!(i);
                            changed = true;
                        }
                    }
                }
                if !row.tmdb {
                    let nfo = self.nfo_for(path, rel, "movie", own).await;
                    changed |= describe(&mut p, nfo, &probe, &file, &row.title);
                }
            }
            "episode" => {
                let names = self.folder_names(dir.clone()).await;
                let stem = file_stem(rel);
                let probe = self.probe_of(path, file.2, file.3).await;
                if missing(&p.images, "still") {
                    let img = match episode_thumb(&names, &stem) {
                        Some(n) => self.file_image(&dir, &n, "still").await,
                        None => match embedded_cover(&probe) {
                            Some(idx) => Some(self.register(format!("embed:{idx}:{}:{path}", file.3), "still").await),
                            None => self.frame_image(&file, &probe, "still").await,
                        },
                    };
                    if let Some(i) = img {
                        p.images["still"] = json!(i);
                        changed = true;
                    }
                }
                if !row.tmdb {
                    let nfo = self.read_nfo(&dir, &names, &[format!("{stem}.nfo")]).await;
                    changed |= describe(&mut p, nfo, &probe, &file, &row.title);
                }
            }
            "series" | "season" => {
                // The show's folder: the first part of the episode's path under the root.
                let show_rel = rel.split('/').next().unwrap_or("");
                let root = &path[..path.len() - rel.len() - 1];
                let show_dir = crate::storage::vfs::join(root, show_rel);
                let show_names = self.folder_names(show_dir.clone()).await;
                if row.kind == "series" {
                    let (poster, backdrop, logo) = folder_art(&show_names, None, true);
                    for (k, name, kind) in [
                        ("poster", poster, "poster"),
                        ("backdrop", backdrop, "backdrop"),
                        ("logo", logo, "logo"),
                    ] {
                        if missing(&p.images, k) {
                            if let Some(n) = name {
                                if let Some(i) = self.file_image(&show_dir, &n, kind).await {
                                    p.images[k] = if k == "backdrop" { json!([i]) } else { json!(i) };
                                    changed = true;
                                }
                            }
                        }
                    }
                    if !row.tmdb {
                        if let Some(n) = self.read_nfo(&show_dir, &show_names, &["tvshow.nfo".to_string()]).await {
                            changed |= describe(
                                &mut p,
                                Some(n),
                                &Value::Null,
                                &(String::new(), String::new(), 0, 0, None),
                                &row.title,
                            );
                        }
                    }
                } else if missing(&p.images, "poster") {
                    let season_names = if dir == show_dir {
                        std::sync::Arc::new(Vec::new())
                    } else {
                        self.folder_names(dir.clone()).await
                    };
                    if let Some(n) = season_poster(&show_names, &season_names, row.season_no.unwrap_or(1)) {
                        let (d, name) = match n.split_once('/') {
                            Some(("show", name)) => (show_dir.clone(), name.to_string()),
                            Some((_, name)) => (dir.clone(), name.to_string()),
                            None => (dir.clone(), n.clone()),
                        };
                        if let Some(i) = self.file_image(&d, &name, "poster").await {
                            p.images["poster"] = json!(i);
                            changed = true;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(if changed { Local::Found(p) } else { Local::Nothing })
    }

    /// No other title's file is in `dir` or below it.
    async fn only_title_in(&self, dir: &str) -> bool {
        let d = dir.to_string();
        let n: i64 = self
            .db
            .call(move |c| {
                Ok(c.query_row(
                    &format!("SELECT COUNT(DISTINCT item_id) FROM files WHERE missing_since IS NULL AND role IN {TITLE_ROLES} AND substr(path, 1, length(?1) + 1) = ?1 || '/'"),
                    [d],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap_or(2);
        n <= 1
    }
}

fn file_stem(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.rsplit_once('.').map_or_else(|| name.to_string(), |(s, _)| s.to_string())
}

/// Text for a title TMDB doesn't describe: the NFO first, then the file's tags. Returns whether
/// anything was found.
fn describe(p: &mut Patch, nfo: Option<Nfo>, probe: &Value, file: &FileInfo, current_title: &str) -> bool {
    let (tag_title, tag_year, tag_desc) = embedded_tags(probe);
    let mut any = false;
    if let Some(n) = nfo {
        any = true;
        p.title = n.title;
        p.original_title = n.original_title;
        p.year = n.year;
        p.overview = n.plot;
        p.tagline = n.tagline;
        p.genres = n.genres;
        p.studios = n.studios;
        p.rating = n.rating;
        p.premiered = n.premiered;
        if let Some(t) = n.tmdb {
            p.ids.push(("Tmdb", t.to_string()));
        }
        if let Some(i) = n.imdb {
            p.ids.push(("Imdb", i));
        }
    }
    // A tag title that is just the file's name (as some tools write it) says nothing.
    let tag_title = tag_title
        .filter(|t| !t.contains('.') || t.contains(' '))
        .filter(|t| super::util::normalize(t) != super::util::normalize(current_title) || p.title.is_none());
    if p.title.is_none() && tag_title.is_some() {
        p.title = tag_title;
        any = true;
    }
    if p.year.is_none() && tag_year.is_some() {
        p.year = tag_year;
        any = true;
    }
    if p.overview.is_none() && tag_desc.is_some() {
        p.overview = tag_desc;
        any = true;
    }
    if let Some(d) = file.4.filter(|d| *d > 0.0) {
        p.runtime = Some(d);
        any = true;
    }
    any
}

fn write_patch(tx: &rusqlite::Transaction, row: &Row, p: Patch) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE items SET images = ?2, updated_at = ?3 WHERE id = ?1",
        params![row.id, p.images.to_string(), now()],
    )?;
    if row.tmdb {
        return Ok(());
    }
    if let Some(t) = p.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        tx.execute(
            "UPDATE items SET title = ?2, sort_title = ?3 WHERE id = ?1",
            params![row.id, t, sort_title(t)],
        )?;
    }
    let list = |v: &[String]| (!v.is_empty()).then(|| json!(v).to_string());
    tx.execute(
        "UPDATE items SET original_title = COALESCE(?2, original_title), year = COALESCE(?3, year), overview = COALESCE(?4, overview),
         tagline = COALESCE(?5, tagline), genres = COALESCE(?6, genres), studios = COALESCE(?7, studios),
         community_rating = COALESCE(?8, community_rating), premiere_date = COALESCE(?9, premiere_date), runtime = COALESCE(?10, runtime)
         WHERE id = ?1",
        params![
            row.id,
            p.original_title,
            p.year,
            p.overview,
            p.tagline,
            list(&p.genres),
            list(&p.studios),
            p.rating,
            p.premiered,
            p.runtime
        ],
    )?;
    if !p.ids.is_empty() {
        let old: Option<String> = tx
            .query_row("SELECT provider_ids FROM items WHERE id = ?1", [&row.id], |r| r.get(0))
            .optional()?
            .flatten();
        let mut m: serde_json::Map<String, Value> = old.and_then(|o| serde_json::from_str(&o).ok()).unwrap_or_default();
        for (k, v) in p.ids {
            m.entry(k.to_string()).or_insert(json!(v));
        }
        tx.execute(
            "UPDATE items SET provider_ids = ?2 WHERE id = ?1",
            params![row.id, Value::Object(m).to_string()],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn nfo_files() {
        let movie = r#"<?xml version="1.0" encoding="UTF-8"?>
<movie>
  <title>Blade Runner</title><originaltitle>Blade Runner</originaltitle>
  <year>1982</year>
  <plot><![CDATA[A blade runner must pursue & terminate four replicants.]]></plot>
  <genre>Science Fiction</genre><genre>Drama</genre>
  <ratings><rating name="themoviedb" max="10"><value>7.9</value></rating></ratings>
  <uniqueid type="tmdb" default="true">78</uniqueid><uniqueid type="imdb">tt0083658</uniqueid>
  <actor><name>Harrison Ford</name><role>Deckard</role></actor>
</movie>"#;
        let n = parse_nfo(movie).unwrap();
        assert_eq!(n.title.as_deref(), Some("Blade Runner"));
        assert_eq!((n.year, n.tmdb, n.imdb.as_deref()), (Some(1982), Some(78), Some("tt0083658")));
        assert_eq!(n.plot.as_deref(), Some("A blade runner must pursue & terminate four replicants."));
        assert_eq!(n.genres, ["Science Fiction", "Drama"]);
        assert_eq!(n.rating, Some(7.9));
        // Kodi also takes a bare link.
        let n = parse_nfo("https://www.themoviedb.org/movie/603-the-matrix\n").unwrap();
        assert_eq!((n.tmdb, n.title), (Some(603), None));
        assert_eq!(
            parse_nfo("https://www.imdb.com/title/tt0133093/").unwrap().imdb.as_deref(),
            Some("tt0133093")
        );
        assert!(parse_nfo("nothing here").is_none());
        let ep = "<episodedetails><title>Pilot</title><season>1</season><episode>1</episode><aired>2008-01-20</aired><plot>Walt &amp; Jesse.</plot></episodedetails>";
        let n = parse_nfo(ep).unwrap();
        assert_eq!(
            (n.title.as_deref(), n.year, n.plot.as_deref()),
            (Some("Pilot"), Some(2008), Some("Walt & Jesse."))
        );
    }

    #[test]
    fn artwork_names() {
        let n = names(&["Movie (2001).mkv", "Movie (2001)-poster.JPG", "fanart.jpg", "Logo.png", "notes.txt"]);
        assert_eq!(
            folder_art(&n, Some("Movie (2001)"), true),
            (
                Some("Movie (2001)-poster.JPG".into()),
                Some("fanart.jpg".into()),
                Some("Logo.png".into())
            )
        );
        // In a shared folder, only the file's own names count.
        assert_eq!(
            folder_art(&n, Some("Movie (2001)"), false),
            (Some("Movie (2001)-poster.JPG".into()), None, None)
        );
        let n = names(&["folder.jpg", "a.mkv"]);
        assert_eq!(folder_art(&n, Some("a"), true).0.as_deref(), Some("folder.jpg"));
        let show = names(&["season01-poster.jpg", "season-specials-poster.png", "poster.jpg"]);
        assert_eq!(season_poster(&show, &[], 1).as_deref(), Some("show/season01-poster.jpg"));
        assert_eq!(season_poster(&show, &[], 0).as_deref(), Some("show/season-specials-poster.png"));
        assert_eq!(
            season_poster(&names(&["poster.jpg"]), &names(&["folder.jpg"]), 2).as_deref(),
            Some("season/folder.jpg")
        );
        let eps = names(&["Show S01E01.mkv", "Show S01E01-thumb.jpg", "Show S01E02.mkv", "Show S01E02.jpg"]);
        assert_eq!(episode_thumb(&eps, "Show S01E01").as_deref(), Some("Show S01E01-thumb.jpg"));
        assert_eq!(episode_thumb(&eps, "Show S01E02").as_deref(), Some("Show S01E02.jpg"));
        assert_eq!(episode_thumb(&eps, "Show S01E03"), None);
    }

    #[test]
    fn embedded_metadata() {
        let probe = json!({"format": {"tags": {"TITLE": "The Big Short", "DATE_RELEASED": "2015-12-11", "comment": "x264 encode"}},
            "streams": [{"index": 0, "codec_type": "video"}, {"index": 3, "codec_type": "attachment", "tags": {"mimetype": "image/jpeg", "filename": "small_cover.jpg"}},
                        {"index": 4, "codec_type": "attachment", "tags": {"mimetype": "image/jpeg", "filename": "cover.jpg"}}]});
        assert_eq!(embedded_cover(&probe), Some(4));
        let (t, y, d) = embedded_tags(&probe);
        assert_eq!(
            (t.as_deref(), y, d),
            (Some("The Big Short"), Some(2015), None),
            "a short comment isn't a description"
        );
        let mp4 = json!({"streams": [{"index": 0, "codec_type": "video"}, {"index": 2, "codec_type": "video", "disposition": {"attached_pic": 1}}]});
        assert_eq!(embedded_cover(&mp4), Some(2));
        assert_eq!(embedded_cover(&json!({"streams": []})), None);
        assert!(is_hdr(
            &json!({"streams": [{"codec_type": "video", "color_transfer": "smpte2084"}]})
        ));
    }
}
