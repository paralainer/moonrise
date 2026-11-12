//! Artwork: TMDB's posters, backdrops, logos, episode stills and people, and local art
//! (`local.rs`): image files in the media folders (`file:<mtime>:<path>`), cover art inside a
//! video (`embed:<stream>:<mtime>:<path>`) and frames of a video (`frame:<secs>:<hdr>:<mtime>:<path>`).
//!
//! An image's id is a hash of its TMDB file path. TMDB never changes the bytes behind a file
//! path (new art gets a new path), so the id is also the cache tag: `GET /v1/images/{id}?w=`
//! is served `immutable`. The master (the original, scaled to at most 1920 px on its long
//! side) is kept in `/data/images`; it is fetched in the background for titles' art and on
//! first request for the rest (people). Resized copies live in the segment cache (`img/`) and
//! are evicted with it.

use std::{
    collections::HashMap,
    io::Cursor,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use super::util::now;
use crate::{db::Db, storage::cache::Cache};

const MAX_EDGE: u32 = 1920;

pub struct Images {
    dir: PathBuf,
    ffmpeg: String,
    db: Db,
    cache: Arc<Cache>,
    http: reqwest::Client,
    /// Masters being fetched (shared by concurrent requests).
    inflight: Mutex<HashMap<String, watch::Receiver<Option<Result<(), String>>>>>,
    fetch_slots: tokio::sync::Semaphore,
}

/// The id of an image from `source` ("tmdb:/abc.jpg").
pub fn image_id(source: &str) -> String {
    let h = Sha256::digest(source.as_bytes());
    h.iter().take(12).map(|b| format!("{b:02x}")).collect()
}

/// Registers a TMDB image (`file_path` as TMDB gives it); returns its id. SVG files are left
/// out (None).
pub fn register(c: &rusqlite::Connection, file_path: Option<&str>, kind: &str) -> Result<Option<String>> {
    let Some(fp) = file_path.filter(|f| f.starts_with('/') && !f.to_ascii_lowercase().ends_with(".svg")) else {
        return Ok(None);
    };
    let source = format!("tmdb:{fp}");
    let id = image_id(&source);
    c.execute(
        "INSERT OR IGNORE INTO images(id, source, kind) VALUES (?1, ?2, ?3)",
        params![id, source, kind],
    )?;
    Ok(Some(id))
}

impl Images {
    pub fn new(dir: PathBuf, db: Db, cache: Arc<Cache>, ffmpeg: String) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("http client");
        Images {
            dir,
            ffmpeg,
            db,
            cache,
            http,
            inflight: Mutex::new(HashMap::new()),
            fetch_slots: tokio::sync::Semaphore::new(3),
        }
    }

    /// The image as served: the master, or scaled to width `w` (JPEG, or PNG for images with
    /// transparency). Returns the bytes and the content type.
    pub async fn serve(&self, id: &str, w: Option<u32>) -> Result<Option<(Bytes, &'static str)>> {
        let Some((path, width)) = self.master(id).await? else {
            return Ok(None);
        };
        let png = path.extension().is_some_and(|e| e == "png");
        let ctype = if png { "image/png" } else { "image/jpeg" };
        let w = w.map(|w| w.clamp(32, MAX_EDGE));
        match w {
            Some(w) if (w as i64) < width => {
                let key = format!("img/{}/{id}-{w}.{}", &id[..2], if png { "png" } else { "jpg" });
                if let Some(b) = self.cache.get(&key).await {
                    return Ok(Some((b, ctype)));
                }
                let src = tokio::fs::read(&path).await?;
                let out = tokio::task::spawn_blocking(move || resize(&src, w, png)).await??;
                self.cache.put(&key, &out).await;
                Ok(Some((Bytes::from(out), ctype)))
            }
            _ => Ok(Some((Bytes::from(tokio::fs::read(&path).await?), ctype))),
        }
    }

    /// The master's path and width, fetching it first if needed. None: no such image.
    async fn master(&self, id: &str) -> Result<Option<(PathBuf, i64)>> {
        if id.len() != 24 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(None);
        }
        let row = self.row(id).await?;
        let Some((source, path, width, error, fetched_at)) = row else {
            return Ok(None);
        };
        if let (Some(p), Some(w)) = (&path, width) {
            let full = self.dir.join(p);
            if full.is_file() {
                return Ok(Some((full, w)));
            }
        }
        if error.is_some() && now() - fetched_at.unwrap_or(0) < 3600 {
            bail!("image unavailable: {}", error.unwrap_or_default());
        }
        self.fetch(id, &source).await?;
        match self.row(id).await? {
            Some((_, Some(p), Some(w), _, _)) => Ok(Some((self.dir.join(p), w))),
            _ => bail!("image not stored"),
        }
    }

    async fn row(&self, id: &str) -> Result<Option<(String, Option<String>, Option<i64>, Option<String>, Option<i64>)>> {
        let id = id.to_string();
        self.db
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT source, path, width, error, fetched_at FROM images WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()?)
            })
            .await
    }

    /// Fetches and stores the master (one fetch per image at a time, at most 3 at once).
    pub async fn fetch(&self, id: &str, source: &str) -> Result<()> {
        let (tx, mut rx) = {
            let mut inflight = self.inflight.lock().unwrap();
            match inflight.get(id) {
                Some(rx) => (None, rx.clone()),
                None => {
                    let (tx, rx) = watch::channel(None);
                    inflight.insert(id.to_string(), rx.clone());
                    (Some(tx), rx)
                }
            }
        };
        if let Some(tx) = tx {
            let r = match self.fetch_slots.acquire().await {
                Ok(_permit) => fetch_master(&self.dir, &self.db, &self.http, &self.ffmpeg, id, source)
                    .await
                    .map_err(|e| format!("{e:#}")),
                Err(e) => Err(e.to_string()),
            };
            self.inflight.lock().unwrap().remove(id);
            let _ = tx.send(Some(r.clone()));
            return r.map_err(|e| anyhow::anyhow!(e));
        }
        loop {
            if let Some(r) = rx.borrow().clone() {
                return r.map_err(|e| anyhow::anyhow!(e));
            }
            if rx.changed().await.is_err() {
                bail!("image fetch abandoned");
            }
        }
    }

    /// Background: fetch the masters of titles' art (not people's) that aren't there yet,
    /// gently (one at a time, a short pause between).
    pub async fn prefetch_loop(self: Arc<Self>) {
        loop {
            let todo: Vec<(String, String)> = self
                .db
                .call(|c| {
                    let mut st = c.prepare(
                        "SELECT id, source FROM images WHERE path IS NULL AND kind != 'profile'
                         AND (error IS NULL OR fetched_at < ?1) LIMIT 200",
                    )?;
                    let rows = st
                        .query_map([now() - 86_400], |r| Ok((r.get(0)?, r.get(1)?)))?
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(rows)
                })
                .await
                .unwrap_or_default();
            if todo.is_empty() {
                tokio::time::sleep(Duration::from_secs(120)).await;
                continue;
            }
            let mut done = 0;
            for (id, source) in &todo {
                if let Err(e) = self.fetch(id, source).await {
                    tracing::debug!(id, "image prefetch: {e:#}");
                } else {
                    done += 1;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            tracing::info!(fetched = done, of = todo.len(), "image masters prefetched");
        }
    }
}

/// The original bytes of an image source, and whether it is a PNG (that may stay one).
async fn source_bytes(http: &reqwest::Client, ffmpeg: &str, source: &str) -> Result<(Bytes, bool)> {
    let (scheme, rest) = source.split_once(':').context("bad image source")?;
    match scheme {
        "tmdb" => {
            let url = format!("https://image.tmdb.org/t/p/original{rest}");
            let resp = http.get(&url).send().await.context("image request")?;
            if !resp.status().is_success() {
                bail!("HTTP {}", resp.status());
            }
            Ok((resp.bytes().await?, rest.to_ascii_lowercase().ends_with(".png")))
        }
        "file" => {
            let (_mtime, path) = rest.split_once(':').context("bad file source")?;
            let b = crate::storage::vfs::read(path).await.with_context(|| format!("read {path}"))?;
            Ok((Bytes::from(b), path.to_ascii_lowercase().ends_with(".png")))
        }
        "embed" => {
            let mut it = rest.splitn(3, ':');
            let (idx, _mtime, path) = (
                it.next().context("bad source")?,
                it.next().context("bad source")?,
                it.next().context("bad source")?,
            );
            let idx: u32 = idx.parse().context("bad stream index")?;
            // An attached picture decodes as a video frame; a Matroska attachment is dumped.
            let mut args: Vec<String> = vec!["-v".into(), "error".into()];
            args.extend(crate::storage::vfs::ffmpeg_input_args(path));
            args.extend([
                "-map".into(),
                format!("0:{idx}"),
                "-frames:v".into(),
                "1".into(),
                "-f".into(),
                "image2pipe".into(),
                "-c:v".into(),
                "png".into(),
                "-".into(),
            ]);
            if let Ok(b) = run_ffmpeg(ffmpeg, &args).await {
                return Ok((b, true));
            }
            let tmp = std::env::temp_dir().join(format!("moonrise-attach-{}", image_id(source)));
            let mut args: Vec<String> = vec![
                "-v".into(),
                "error".into(),
                "-y".into(),
                format!("-dump_attachment:{idx}"),
                tmp.to_string_lossy().into_owned(),
            ];
            args.extend(crate::storage::vfs::ffmpeg_input_args(path));
            // ffmpeg dumps on opening the input and then complains there is no output.
            let _ = run_ffmpeg(ffmpeg, &args).await;
            let b = tokio::fs::read(&tmp).await.context("attachment not dumped")?;
            let _ = tokio::fs::remove_file(&tmp).await;
            let png = b.starts_with(b"\x89PNG");
            Ok((Bytes::from(b), png))
        }
        "frame" => {
            let mut it = rest.splitn(4, ':');
            let (at, hdr, _mtime, path) = (
                it.next().context("bad source")?,
                it.next().context("bad source")?,
                it.next().context("bad source")?,
                it.next().context("bad source")?,
            );
            let at: u64 = at.parse().context("bad time")?;
            let scale = "scale='min(1280,iw)':-2";
            let vf = if hdr == "1" {
                format!("zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p,{scale}")
            } else {
                scale.to_string()
            };
            let mut args: Vec<String> = vec!["-v".into(), "error".into(), "-ss".into(), at.to_string()];
            args.extend(crate::storage::vfs::ffmpeg_input_args(path));
            args.extend([
                "-map".into(),
                "0:v:0".into(),
                "-frames:v".into(),
                "1".into(),
                "-vf".into(),
                vf,
                "-f".into(),
                "image2pipe".into(),
                "-c:v".into(),
                "mjpeg".into(),
                "-q:v".into(),
                "3".into(),
                "-".into(),
            ]);
            Ok((run_ffmpeg(ffmpeg, &args).await?, false))
        }
        _ => bail!("unknown image source"),
    }
}

/// ffmpeg's stdout (`nice`d, at most a minute).
async fn run_ffmpeg(ffmpeg: &str, args: &[String]) -> Result<Bytes> {
    let mut cmd = tokio::process::Command::new("nice");
    cmd.arg("-n")
        .arg("10")
        .arg(ffmpeg)
        .args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(60), cmd.output())
        .await
        .context("ffmpeg timed out")??;
    if !out.status.success() || out.stdout.is_empty() {
        bail!(
            "ffmpeg: {}",
            crate::admin::logbuf::scrub(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("no output"))
        );
    }
    Ok(Bytes::from(out.stdout))
}

async fn fetch_master(dir: &std::path::Path, db: &Db, http: &reqwest::Client, ffmpeg: &str, id: &str, source: &str) -> Result<()> {
    let result: Result<(String, u32, u32, usize)> = async {
        let (bytes, png_source) = source_bytes(http, ffmpeg, source).await?;
        let (data, w, h, png) = tokio::task::spawn_blocking(move || master_bytes(&bytes, png_source)).await??;
        let rel = format!("{}/{id}.{}", &id[..2], if png { "png" } else { "jpg" });
        let full = dir.join(&rel);
        tokio::fs::create_dir_all(full.parent().unwrap()).await?;
        let tmp = full.with_extension("part");
        tokio::fs::write(&tmp, &data).await?;
        tokio::fs::rename(&tmp, &full).await?;
        Ok((rel, w, h, data.len()))
    }
    .await;
    let id = id.to_string();
    match result {
        Ok((rel, w, h, len)) => {
            db.call(move |c| {
                c.execute(
                    "UPDATE images SET path = ?2, width = ?3, height = ?4, bytes = ?5, fetched_at = ?6, error = NULL WHERE id = ?1",
                    params![id, rel, w, h, len as i64, now()],
                )?;
                Ok(())
            })
            .await
        }
        Err(e) => {
            let msg = format!("{e:#}");
            db.call(move |c| {
                c.execute(
                    "UPDATE images SET error = ?2, fetched_at = ?3 WHERE id = ?1",
                    params![id, msg, now()],
                )?;
                Ok(())
            })
            .await?;
            Err(e)
        }
    }
}

/// The stored master: as fetched when it is small enough, else scaled to MAX_EDGE. PNGs with
/// transparency (logos) stay PNG. Returns (bytes, width, height, png).
fn master_bytes(src: &[u8], png_source: bool) -> Result<(Vec<u8>, u32, u32, bool)> {
    let img = image::load_from_memory(src).context("decode image")?;
    let (w, h) = (img.width(), img.height());
    let png = png_source && img.color().has_alpha();
    if w.max(h) <= MAX_EDGE && (png || !png_source) {
        return Ok((src.to_vec(), w, h, png));
    }
    let scaled = if w.max(h) > MAX_EDGE {
        img.resize(MAX_EDGE, MAX_EDGE, image::imageops::FilterType::CatmullRom)
    } else {
        img
    };
    let out = encode(&scaled, png)?;
    Ok((out, scaled.width(), scaled.height(), png))
}

fn resize(src: &[u8], w: u32, png: bool) -> Result<Vec<u8>> {
    let img = image::load_from_memory(src).context("decode image")?;
    let h = ((img.height() as f64) * (w as f64) / (img.width() as f64)).round().max(1.0) as u32;
    let scaled = img.resize_exact(w, h, image::imageops::FilterType::CatmullRom);
    encode(&scaled, png)
}

fn encode(img: &image::DynamicImage, png: bool) -> Result<Vec<u8>> {
    let mut out = Cursor::new(Vec::new());
    if png {
        img.write_to(&mut out, image::ImageFormat::Png)?;
    } else {
        let rgb = img.to_rgb8();
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88);
        rgb.write_with_encoder(enc)?;
    }
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_scaling() {
        let id = image_id("tmdb:/abc.jpg");
        assert_eq!(id.len(), 24);
        assert_eq!(id, image_id("tmdb:/abc.jpg"));
        assert_ne!(id, image_id("tmdb:/abd.jpg"));
        let img = image::DynamicImage::new_rgb8(3000, 1500);
        let jpg = encode(&img, false).unwrap();
        let (data, w, h, png) = master_bytes(&jpg, false).unwrap();
        assert_eq!((w, h, png), (1920, 960, false));
        let small = resize(&data, 400, false).unwrap();
        let s = image::load_from_memory(&small).unwrap();
        assert_eq!((s.width(), s.height()), (400, 200));
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE images(id TEXT PRIMARY KEY, source TEXT, kind TEXT)")
            .unwrap();
        assert!(register(&c, Some("/logo.svg"), "logo").unwrap().is_none());
        assert_eq!(register(&c, Some("/a.png"), "logo").unwrap(), Some(image_id("tmdb:/a.png")));
    }
}
