//! Keeps `files` in step with the library roots: local folders (CIFS mounts too) or SMB shares
//! read by the server itself (`smb://…`, through `vfs.rs`). Neither tells about changes
//! (inotify sees nothing on CIFS), so this polls (Appendix B of the plan).
//!
//! - **Quick pass** (every `MOONRISE_SCAN_QUICK_SECS`): `stat` the known directories down to two
//!   levels below each root (movie folders, show folders, season folders) and list only those
//!   whose mtime changed (Samba updates it when an entry is added, removed or renamed); new
//!   folders and anything deeper are walked. Files not yet stable, and damaged ones, are
//!   `stat`ed again. A file copied over another in place leaves its folder's mtime alone: a
//!   damaged one is caught by that `stat`, any other when playback finds it changed
//!   (`file_changed`), else by the next full pass.
//! - **Full pass** (every `MOONRISE_SCAN_FULL_HOURS`, and the first time): walk everything and
//!   compare (path, size, mtime) for every file.
//! - **Offline roots**: a root that is missing, empty or unreadable (a share that doesn't
//!   answer, a mount that is down) is *offline* for the pass: it is left out and nothing under
//!   it is marked missing; the other roots are scanned as usual. When it is back it is walked
//!   in full. `Library::root_states` says which roots are offline and why.
//! - **Renames** (popcorn-tracker moves with `os.rename`, keeping the mtime): a new path with
//!   the size, mtime and OpenSubtitles hash of a file that just went missing is that file: it
//!   keeps its id and its title.
//! - **Stability**: a file is probed and identified only once its size and mtime have stayed
//!   the same for 60 s across two looks (or its mtime is more than 10 minutes old), so copies
//!   in progress are left alone.
//! - **Probing** (`probe_loop`): ffprobe (`nice`d, `MOONRISE_SCAN_PROBE_JOBS` at a time) once per
//!   (size, mtime), into the same probe cache playback uses, and the OpenSubtitles hash (two
//!   64 KiB reads). Extras and samples aren't probed.

use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use anyhow::Result;
use futures_util::StreamExt;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use super::{parse, util, Library};
use crate::{admin::config::LibraryConfig, storage::vfs};

/// How long a file's size and mtime must stay put before it is probed.
const STABLE_SECS: i64 = 60;
/// A file whose mtime is older than this is taken as complete at once.
const OLD_MTIME_SECS: i64 = 600;
/// Files shorter than this (seconds) aren't titles.
pub const MIN_DURATION: f64 = 120.0;

#[derive(Debug, Default, Clone, Serialize)]
pub struct ScanReport {
    pub full: bool,
    pub dirs_checked: usize,
    pub dirs_listed: usize,
    pub files_seen: usize,
    pub new_files: usize,
    pub changed: usize,
    pub missing: usize,
    pub returned: usize,
    pub renamed: usize,
    /// Every root was offline (nothing was looked at).
    pub aborted: Option<String>,
    /// Roots left out of this pass: (root, why).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub offline: Vec<(String, String)>,
    pub ms: u64,
}

impl ScanReport {
    pub fn changed_anything(&self) -> bool {
        self.new_files + self.changed + self.missing + self.returned + self.renamed > 0
    }
}

/// What the file system showed.
#[derive(Default)]
pub(crate) struct FsResult {
    /// (library, path, size, mtime ns)
    pub(crate) files: Vec<(String, String, u64, i64)>,
    /// Directories listed completely: known files below them (outside `skipped`) that
    /// weren't seen are gone.
    complete: Vec<String>,
    /// Known directories left out of a listing (unchanged; checked on their own).
    skipped: Vec<String>,
    /// Known directories that are gone.
    gone: Vec<String>,
    /// Directory mtimes to remember (depth ≤ 2).
    pub(crate) dir_mtimes: Vec<(String, i64)>,
    /// Re-stat of files waiting to become stable: path → (size, mtime) or None (gone).
    restat: Vec<(String, Option<(u64, i64)>)>,
    /// Roots that couldn't be looked at: (root, why).
    pub(crate) offline: Vec<(String, String)>,
    /// Roots looked at.
    pub(crate) online: Vec<String>,
    dirs_checked: usize,
    dirs_listed: usize,
}

fn depth_below(root: &str, dir: &str) -> usize {
    dir.strip_prefix(root).map_or(0, |r| r.split('/').filter(|s| !s.is_empty()).count())
}

/// A folder holding a DVD (VIDEO_TS): the folder is the "file" (as media servers treat it).
fn dvd_folder(dir: &str) -> Option<(u64, i64)> {
    let vts = vfs::read_dir_blocking(dir)
        .ok()?
        .into_iter()
        .find(|e| e.name.eq_ignore_ascii_case("VIDEO_TS") && e.is_dir)?;
    let size = vfs::read_dir_blocking(&vts.path)
        .ok()?
        .iter()
        .filter(|e| e.is_file)
        .map(|e| e.size)
        .sum();
    let m = vfs::stat_blocking(dir).ok()?;
    Some((size, m.mtime))
}

/// Video files seen so far by the walk running now (the setup screen's live count).
pub static WALK_FILES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Lists `dir` (recursively), adding video files to `out`. Known directories at depth ≤ 2
/// in `known` are skipped (they are checked by their own mtime) unless `all`.
fn walk(lib: &str, root: &str, dir: &str, all: bool, known: &HashMap<String, i64>, out: &mut FsResult) -> std::io::Result<()> {
    let rd = vfs::read_dir_blocking(dir)?;
    out.dirs_listed += 1;
    let dir_s = dir.to_string();
    out.complete.push(dir_s.clone());
    if depth_below(root, &dir_s) <= 2 {
        if let Ok(m) = vfs::stat_blocking(dir) {
            out.dir_mtimes.push((dir_s.clone(), m.mtime));
        }
    }
    for e in rd {
        let p = e.path.clone();
        if e.is_dir {
            if e.name.eq_ignore_ascii_case("VIDEO_TS") {
                continue;
            }
            if let Some((size, mtime)) = dvd_folder(&p) {
                out.files.push((lib.to_string(), p.clone(), size, mtime));
                WALK_FILES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            if !all && depth_below(root, &p) <= 2 && known.contains_key(&p) {
                out.skipped.push(p);
                continue;
            }
            // A subfolder that can't be read is left alone (not "gone").
            if walk(lib, root, &p, all, known, out).is_err() {
                out.skipped.push(p);
            }
        } else if e.is_file && parse::is_video(&p) {
            out.files.push((lib.to_string(), p, e.size, e.mtime));
            WALK_FILES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    Ok(())
}

/// Whether a root can be scanned: it lists, and isn't empty (an empty one is taken for a share
/// or mount that is down). The reason when it can't.
fn root_check(root: &str) -> Result<(), String> {
    match vfs::read_dir_blocking(root) {
        Ok(l) if l.is_empty() => Err("the folder is empty".into()),
        Ok(_) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// `walk_all`: roots walked in full even in a quick pass (back from offline).
pub(crate) fn scan_fs(
    libs: &[LibraryConfig],
    full: bool,
    known: &HashMap<String, i64>,
    restat: &[String],
    walk_all: &HashSet<String>,
) -> FsResult {
    let mut out = FsResult::default();
    for lib in libs {
        for root in &lib.roots {
            // A root that is down is left out (its files aren't marked missing).
            if let Err(why) = root_check(root) {
                out.offline.push((root.clone(), why));
                continue;
            }
            if full || !known.contains_key(root.as_str()) || walk_all.contains(root) {
                if let Err(e) = walk(&lib.id, root, root, true, known, &mut out) {
                    out.offline.push((root.clone(), format!("walk: {e}")));
                    continue;
                }
                out.online.push(root.clone());
                continue;
            }
            out.online.push(root.clone());
            let mut dirs: Vec<(&String, &i64)> = known
                .iter()
                .filter(|(d, _)| d.as_str() == root || d.starts_with(&format!("{root}/")))
                .collect();
            dirs.sort();
            let mut gone_prefixes: Vec<String> = Vec::new();
            for (d, old) in dirs {
                if gone_prefixes.iter().any(|g| d.starts_with(&format!("{g}/"))) {
                    continue;
                }
                out.dirs_checked += 1;
                match vfs::stat_blocking(d) {
                    Ok(m) if m.is_dir => {
                        if m.mtime != *old {
                            if let Err(e) = walk(&lib.id, root, d, false, known, &mut out) {
                                tracing::warn!(dir = %d, "scan: {e}");
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        out.gone.push(d.clone());
                        gone_prefixes.push(d.clone());
                    }
                    // I/O trouble: leave it for the next pass.
                    Err(e) => tracing::warn!(dir = %d, "scan: {e}"),
                }
            }
        }
    }
    for p in restat {
        // Only files of roots that answered.
        if !out.online.iter().any(|r| p.starts_with(&format!("{r}/"))) {
            continue;
        }
        match vfs::stat_blocking(p) {
            Ok(m) => out.restat.push((p.clone(), Some((m.size, m.mtime)))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => out.restat.push((p.clone(), None)),
            Err(_) => {}
        }
    }
    out
}

/// One known file, as the scan needs it.
struct Known {
    id: String,
    size: i64,
    mtime: i64,
    missing: bool,
}

pub fn stamp(size: i64, mtime: i64) -> String {
    format!("{size}|{mtime}")
}

/// A file may be probed and identified: its size and mtime haven't moved for a while.
pub fn is_stable(now: i64, stable_since: i64, last_seen: i64, mtime_ns: i64) -> bool {
    now - mtime_ns / 1_000_000_000 > OLD_MTIME_SECS || (now - stable_since >= STABLE_SECS && last_seen > stable_since)
}

/// A new path and its rename candidates: (library, path, size, mtime, missing files of that size
/// and mtime with their hashes).
type NewPath = (String, String, u64, i64, Vec<(String, Option<String>)>);
type Candidates = Vec<NewPath>;

/// Pass 1 of a scan: updates what is known, marks what is gone, remembers directory mtimes, and
/// collects new paths with the missing files they might be.
fn update_known(c: &mut rusqlite::Connection, fs: &FsResult, full: bool, now: i64) -> Result<(ScanReport, Candidates)> {
    let tx = c.transaction()?;
    let mut r = ScanReport::default();
    let known = known_files(&tx)?;
    let mut new_paths = Vec::new();
    for (lib, path, size, mtime) in &fs.files {
        match known.get(path) {
            Some(k) => {
                if k.size != *size as i64 || k.mtime != *mtime {
                    tx.execute(
                        "UPDATE files SET size = ?2, mtime = ?3, stable_since = ?4, last_seen = ?4, missing_since = NULL WHERE id = ?1",
                        params![k.id, *size as i64, mtime, now],
                    )?;
                    r.changed += 1;
                } else {
                    tx.execute(
                        "UPDATE files SET last_seen = ?2, missing_since = NULL WHERE id = ?1",
                        params![k.id, now],
                    )?;
                }
                if k.missing {
                    r.returned += 1;
                }
            }
            None => new_paths.push((lib.clone(), path.clone(), *size, *mtime)),
        }
    }
    // Files looked at again on their own (`settling`), unless a listing saw them just now.
    let listed: HashSet<&str> = fs.files.iter().map(|f| f.1.as_str()).collect();
    for (path, st) in &fs.restat {
        if listed.contains(path.as_str()) {
            continue;
        }
        let Some(k) = known.get(path) else { continue };
        match st {
            Some((size, mtime)) if k.size != *size as i64 || k.mtime != *mtime => {
                tx.execute(
                    "UPDATE files SET size = ?2, mtime = ?3, stable_since = ?4, last_seen = ?4 WHERE id = ?1",
                    params![k.id, *size as i64, mtime, now],
                )?;
                r.changed += 1;
            }
            Some(_) => {
                tx.execute("UPDATE files SET last_seen = ?2 WHERE id = ?1", params![k.id, now])?;
            }
            None => {}
        }
    }
    r.missing += mark_lost(&tx, fs, &known, full, now)?;
    remember_dirs(&tx, fs, now)?;
    // Rename candidates for each new path: missing files of the same size and mtime.
    let mut out = Vec::new();
    for (lib, path, size, mtime) in new_paths {
        let mut st = tx.prepare("SELECT id, oshash FROM files WHERE missing_since IS NOT NULL AND size = ?1 AND mtime = ?2")?;
        let cands = st
            .query_map(params![size as i64, mtime], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        out.push((lib, path, size, mtime, cands));
    }
    tx.commit()?;
    Ok((r, out))
}

/// The files known: path → id, size, mtime, missing.
fn known_files(tx: &rusqlite::Transaction) -> Result<HashMap<String, Known>> {
    let mut known: HashMap<String, Known> = HashMap::new();
    {
        let mut st = tx.prepare("SELECT path, id, size, mtime, missing_since FROM files")?;
        let rows = st.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                Known {
                    id: row.get(1)?,
                    size: row.get(2)?,
                    mtime: row.get(3)?,
                    missing: row.get::<_, Option<i64>>(4)?.is_some(),
                },
            ))
        })?;
        for row in rows {
            let (p, k) = row?;
            known.insert(p, k);
        }
    }
    Ok(known)
}

/// Marks the known files that weren't seen and are gone (not those of offline roots, nor those
/// in folders left out of the listing). Answers how many.
fn mark_lost(tx: &rusqlite::Transaction, fs: &FsResult, known: &HashMap<String, Known>, full: bool, now: i64) -> Result<usize> {
    let mut missing = 0;
    let seen: HashSet<&str> = fs.files.iter().map(|f| f.1.as_str()).collect();
    // Gone: below a vanished folder, or in a folder listed completely (outside
    // the unchanged subfolders it skipped). A subfolder skipped by its parent's listing but
    // listed itself (its own mtime changed too) is complete, not skipped.
    let listed: HashSet<&str> = fs.complete.iter().map(String::as_str).collect();
    let skipped: Vec<String> = fs
        .skipped
        .iter()
        .filter(|s| !listed.contains(s.as_str()))
        .map(|s| format!("{s}/"))
        .collect();
    let complete: Vec<String> = fs.complete.iter().map(|s| format!("{s}/")).collect();
    let gone: Vec<String> = fs.gone.iter().map(|s| format!("{s}/")).collect();
    let restat_gone: HashSet<&str> = fs.restat.iter().filter(|(_, s)| s.is_none()).map(|(p, _)| p.as_str()).collect();
    for (path, k) in known {
        if k.missing || seen.contains(path.as_str()) {
            continue;
        }
        let under = |prefixes: &[String]| prefixes.iter().any(|p| path.starts_with(p.as_str()));
        // Files of offline roots are never lost here.
        let in_roots = fs.online.iter().any(|root| path.starts_with(&format!("{root}/")));
        let lost = restat_gone.contains(path.as_str()) || under(&gone) || ((under(&complete) || (full && in_roots)) && !under(&skipped));
        if lost {
            tx.execute(
                "UPDATE files SET missing_since = ?2 WHERE id = ?1 AND missing_since IS NULL",
                params![k.id, now],
            )?;
            missing += 1;
        }
    }
    Ok(missing)
}

/// Remembers directory mtimes (only after the listing succeeded), forgetting vanished ones.
fn remember_dirs(tx: &rusqlite::Transaction, fs: &FsResult, now: i64) -> Result<()> {
    for g in &fs.gone {
        tx.execute(
            "DELETE FROM scan_state WHERE path = ?1 OR path LIKE ?2",
            params![g, format!("{}/%", g.replace('%', "\\%"))],
        )?;
    }
    for (d, m) in &fs.dir_mtimes {
        tx.execute(
            "INSERT OR REPLACE INTO scan_state(path, mtime, scanned_at) VALUES (?1, ?2, ?3)",
            params![d, m, now],
        )?;
    }
    Ok(())
}

impl Library {
    /// One scan pass (quick or full). Changes land in `files`.
    pub async fn scan(&self, full: bool) -> Result<ScanReport> {
        let t0 = Instant::now();
        let mut report = ScanReport {
            full,
            ..Default::default()
        };
        let known_dirs = self.known_dirs().await?;
        let now = util::now();
        // Files still settling: looked at again on every pass.
        let restat = self.settling(now).await?;
        let libs = self.libs();
        let mut walk_all: HashSet<String> = self.offline_roots();
        walk_all.extend(self.take_walk_now());
        WALK_FILES.store(0, std::sync::atomic::Ordering::Relaxed);
        let fs = tokio::task::spawn_blocking(move || scan_fs(&libs, full, &known_dirs, &restat, &walk_all)).await?;
        report.dirs_checked = fs.dirs_checked;
        report.dirs_listed = fs.dirs_listed;
        report.files_seen = fs.files.len();
        report.offline = fs.offline.clone();
        self.set_root_states(&fs.online, &fs.offline);
        if fs.online.is_empty() {
            if !fs.offline.is_empty() {
                report.aborted = Some("every library root is offline".into());
            }
            report.ms = t0.elapsed().as_millis() as u64;
            return Ok(report);
        }

        // Pass 1: update what is known, mark what is gone, collect new paths and the missing
        // files they might be.
        let (r1, new_files) = self.db.call(move |c| update_known(c, &fs, full, now)).await?;
        report.changed = r1.changed;
        report.missing = r1.missing;
        report.returned = r1.returned;

        // Pass 2: renames (confirmed by the hash) and new files.
        for new in new_files {
            self.add_file(new, now, &mut report).await?;
        }
        report.ms = t0.elapsed().as_millis() as u64;
        if report.changed_anything() || full {
            tracing::info!(?report, "scan pass");
        }
        Ok(report)
    }

    /// The directories known (depth ≤ 2) and their mtimes when last listed.
    async fn known_dirs(&self) -> Result<HashMap<String, i64>> {
        let known_dirs: HashMap<String, i64> = self
            .db
            .call(|c| {
                let mut st = c.prepare("SELECT path, mtime FROM scan_state")?;
                let rows = st
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<HashMap<_, _>, _>>()?;
                Ok(rows)
            })
            .await?;
        Ok(known_dirs)
    }

    /// Files to look at again on every pass: those still settling (not yet stable), and damaged
    /// ones (`broken`). A file copied over another keeps its folder's mtime, so the quick pass
    /// doesn't list it; a damaged file is the one people replace that way. There are
    /// few of either.
    async fn settling(&self, now: i64) -> Result<Vec<String>> {
        let restat: Vec<String> = self
            .db
            .call(move |c| {
                let mut st = c.prepare(
                    "SELECT path, stable_since, last_seen, mtime, role = 'broken' FROM files WHERE missing_since IS NULL AND (role = 'broken'
                     OR (role IN ('main', 'pending') AND (probed_stamp IS NULL OR probed_stamp != size || '|' || mtime OR identified_at IS NULL)))",
                )?;
                let rows = st
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, i64>(2)?,
                            r.get::<_, i64>(3)?,
                            r.get::<_, bool>(4)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows
                    .into_iter()
                    .filter(|(_, ss, ls, m, broken)| *broken || !is_stable(now, *ss, *ls, *m))
                    .map(|(p, ..)| p)
                    .collect())
            })
            .await?;
        Ok(restat)
    }

    /// A file found changed outside a scan (playback stats the file it loaded): its new size and
    /// mtime, so it is probed again once it is stable, as a pass that listed it would. A file
    /// copied over another keeps its folder's mtime, so quick passes don't see it.
    /// Answers whether the library knew it with another stamp.
    pub async fn file_changed(&self, path: String, size: u64, mtime: i64) -> Result<bool> {
        let now = util::now();
        let n = self
            .db
            .call(move |c| {
                Ok(c.execute(
                    "UPDATE files SET size = ?2, mtime = ?3, stable_since = ?4, last_seen = ?4
                     WHERE path = ?1 AND missing_since IS NULL AND (size != ?2 OR mtime != ?3)",
                    params![path, size as i64, mtime, now],
                )?)
            })
            .await?;
        if n > 0 {
            self.probe_wake.notify_one();
        }
        Ok(n > 0)
    }

    /// Pass 2 for one new path: a file that went missing, renamed (confirmed by the hash), or a
    /// new file.
    async fn add_file(&self, new: NewPath, now: i64, report: &mut ScanReport) -> Result<()> {
        let (lib, path, size, mtime, cands) = new;
        let renamed_from = self.renamed_from(&path, size, mtime, &cands).await?;
        let libs = self.libs();
        let root = libs
            .iter()
            .find(|l| l.id == lib)
            .and_then(|l| l.roots.iter().find(|r| path.starts_with(&format!("{r}/"))).cloned())
            .unwrap_or_default();
        let rel = path.strip_prefix(&format!("{root}/")).unwrap_or(&path).to_string();
        let parsed = parse::parse(&rel);
        let role = match parsed.kind.as_str() {
            "extra" => "extra",
            "sample" => "sample",
            _ => "main",
        };
        let parsed_json = serde_json::to_string(&parsed)?;
        match renamed_from {
            Some(id) => {
                let p = path.clone();
                let old: Option<String> = self
                        .db
                        .call(move |c| {
                            let tx = c.transaction()?;
                            let old: Option<String> = tx.query_row("SELECT path FROM files WHERE id = ?1", [&id], |r| r.get(0)).optional()?;
                            tx.execute(
                                "UPDATE files SET path = ?2, library_id = ?3, parsed = ?4, role = CASE WHEN role IN ('short', 'broken') THEN role ELSE ?5 END,
                                 missing_since = NULL, last_seen = ?6, identified_at = NULL WHERE id = ?1",
                                params![id, p, lib, parsed_json, role, now],
                            )?;
                            if let Some(o) = &old {
                                // The probe cache follows the file.
                                tx.execute("UPDATE OR IGNORE files_probe SET path = ?2 WHERE path = ?1", params![o, p])?;
                            }
                            tx.commit()?;
                            Ok(old)
                        })
                        .await?;
                tracing::info!(from = ?old, to = %path, "file renamed");
                report.renamed += 1;
            }
            None => {
                let id = util::ulid();
                let p = path.clone();
                self.db
                    .call(move |c| {
                        c.execute(
                            "INSERT INTO files(id, library_id, path, size, mtime, role, parsed, first_seen, last_seen, stable_since)
                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?8)",
                            params![id, lib, p, size as i64, mtime, role, parsed_json, now],
                        )?;
                        Ok(())
                    })
                    .await?;
                report.new_files += 1;
            }
        }
        Ok(())
    }

    /// The missing file (of the same size and mtime) a new path is, if any: the one with its
    /// hash, else the only one without a hash.
    async fn renamed_from(&self, path: &str, size: u64, mtime: i64, cands: &[(String, Option<String>)]) -> Result<Option<String>> {
        let mut renamed_from = None;
        if !cands.is_empty() {
            let hash = oshash_of(path, size).await;
            renamed_from = match (&hash, cands.len()) {
                (Some(h), _) => cands
                    .iter()
                    .find(|(_, oh)| oh.as_deref() == Some(h))
                    .or(if cands.len() == 1 && cands[0].1.is_none() {
                        cands.first()
                    } else {
                        None
                    })
                    .map(|c| c.0.clone()),
                (None, 1) => Some(cands[0].0.clone()),
                _ => None,
            };
            if let (Some(id), Some(h)) = (&renamed_from, hash) {
                let (id, h, st) = (id.clone(), h, stamp(size as i64, mtime));
                self.db
                    .call(move |c| {
                        c.execute("UPDATE files SET oshash = ?2, oshash_stamp = ?3 WHERE id = ?1", params![id, h, st])?;
                        Ok(())
                    })
                    .await?;
            }
        }
        Ok(renamed_from)
    }

    /// Probes stable files that haven't been (in this size and mtime), newest first; forever.
    pub async fn probe_loop(self: std::sync::Arc<Self>) {
        loop {
            let did = match self.probe_some(50).await {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!("probe: {e:#}");
                    0
                }
            };
            if did > 0 {
                self.refresh_video_summary().await;
                self.probed.notify_one();
            }
            if did == 0 {
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_secs(60)) => {}
                    () = self.probe_wake.notified() => {}
                }
            }
        }
    }

    /// Probes up to `n` files; returns how many were looked at.
    ///
    /// (A file with a `probe_error`, `broken`, is probed again only once it changes.)
    pub async fn probe_some(&self, n: usize) -> Result<usize> {
        let now = util::now();
        let todo: Vec<(String, String, i64, i64)> = self
            .db
            .call(move |c| {
                let mut st = c.prepare(PROBE_TODO)?;
                let rows = st
                    .query_map([crate::db::PROBER.get().cloned().filter(|v| !v.is_empty())], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, i64>(2)?,
                            r.get::<_, i64>(3)?,
                            r.get::<_, i64>(4)?,
                            r.get::<_, i64>(5)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows
                    .into_iter()
                    .filter(|r| is_stable(now, r.4, r.5, r.3))
                    .map(|r| (r.0, r.1, r.2, r.3))
                    .take(n)
                    .collect())
            })
            .await?;
        let jobs = self.scan_probe_jobs().max(1);
        let count = todo.len();
        futures_util::stream::iter(todo)
            .for_each_concurrent(jobs, |(id, path, size, mtime)| async move {
                if let Err(e) = self.probe_file(&id, &path, size, mtime).await {
                    tracing::warn!(path, "probe: {e:#}");
                }
            })
            .await;
        Ok(count)
    }

    async fn probe_file(&self, id: &str, path: &str, size: i64, mtime: i64) -> Result<()> {
        let st = stamp(size, mtime);
        if vfs::stat(path).await.is_ok_and(|m| m.is_dir) {
            // A DVD folder: nothing to probe here (not served yet).
            let (id, st) = (id.to_string(), st.clone());
            return self
                .db
                .call(move |c| {
                    c.execute(
                        "UPDATE files SET probed_stamp = ?2, container = 'dvd', role = 'main', probe_error = NULL WHERE id = ?1",
                        params![id, st],
                    )?;
                    Ok(())
                })
                .await;
        }
        let t0 = Instant::now();
        // A probe by another ffprobe (Phase 0 had Debian's) is made again.
        let current = self.db.probe_current(path.to_string(), size as u64, mtime).await?;
        let cached = if current {
            self.db.probe_get(path.to_string(), size as u64, mtime).await?
        } else {
            None
        };
        let probe = match cached {
            Some(c) => Ok(c.probe),
            None => {
                let r = crate::media::probe_path(&self.config.ffprobe, path, true).await;
                if let Ok(v) = &r {
                    self.db.probe_put(path.to_string(), size as u64, mtime, v.clone()).await?;
                }
                r
            }
        };
        let hash = oshash_of(path, size as u64).await;
        let (id, path_s) = (id.to_string(), path.to_string());
        match probe {
            Ok(v) => {
                let duration = v["format"]["duration"].as_str().and_then(|d| d.parse::<f64>().ok());
                let bitrate = v["format"]["bit_rate"].as_str().and_then(|d| d.parse::<i64>().ok());
                let container = v["format"]["format_name"].as_str().map(String::from);
                let has_video = v["streams"].as_array().is_some_and(|s| {
                    s.iter()
                        .any(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"].as_i64() != Some(1))
                });
                let short = duration.is_some_and(|d| d < MIN_DURATION) || !has_video;
                self.db
                    .call(move |c| {
                        let role: String = c.query_row("SELECT role FROM files WHERE id = ?1", [&id], |r| r.get(0))?;
                        let new_role = if short {
                            "short"
                        } else if role == "short" || role == "broken" || role == "pending" {
                            "main"
                        } else {
                            role.as_str()
                        };
                        let changed_role = new_role != role;
                        c.execute(
                            "UPDATE files SET probed_stamp = ?2, duration = ?3, bitrate = ?4, container = ?5, role = ?6, probe_error = NULL,
                             item_id = CASE WHEN ?6 = 'short' THEN NULL ELSE item_id END,
                             oshash = COALESCE(?7, oshash), oshash_stamp = CASE WHEN ?7 IS NULL THEN oshash_stamp ELSE ?2 END,
                             identified_at = CASE WHEN ?8 THEN NULL ELSE identified_at END WHERE id = ?1",
                            params![id, st, duration, bitrate, container, new_role, hash, changed_role],
                        )?;
                        Ok(())
                    })
                    .await?;
                tracing::debug!(path = path_s, ms = t0.elapsed().as_millis() as u64, "probed");
            }
            Err(e) => {
                let msg = format!("{e:#}");
                tracing::warn!(path = path_s, "probe failed: {msg}");
                self.db
                    .call(move |c| {
                        c.execute(
                            "UPDATE files SET probed_stamp = ?2, probe_error = ?3, role = 'broken' WHERE id = ?1",
                            params![id, st, msg],
                        )?;
                        Ok(())
                    })
                    .await?;
            }
        }
        Ok(())
    }
}

/// The files to probe: not yet, changed since, or probed by another ffprobe (?1). A file with a
/// `probe_error` (ffprobe failed, or `mark_damaged`) waits until it changes.
pub(crate) const PROBE_TODO: &str = "SELECT id, path, size, mtime, stable_since, last_seen FROM files
     WHERE missing_since IS NULL AND role IN ('main', 'short', 'broken', 'pending')
     AND (probed_stamp IS NULL OR probed_stamp != size || '|' || mtime
          OR (probe_error IS NULL AND container != 'dvd' AND NOT EXISTS (SELECT 1 FROM files_probe fp WHERE fp.path = files.path
              AND fp.size = files.size AND fp.mtime = files.mtime AND fp.prober IS ?1)))
     ORDER BY first_seen DESC, path LIMIT 500";

/// The OpenSubtitles hash of a library file (two 64 KiB reads).
pub async fn oshash_of(path: &str, size: u64) -> Option<String> {
    if size < 65536 * 2 {
        return None;
    }
    let (head, tail) = tokio::join!(vfs::read_at(path, 0, 65536), vfs::read_at(path, size - 65536, 65536));
    let (head, tail) = (head.ok()?, tail.ok()?);
    if head.len() < 65536 || tail.len() < 65536 {
        return None;
    }
    crate::subtitles::opensubtitles::movie_hash(size, &head, &tail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stability() {
        let now = 10_000_000;
        // Old mtime: stable at once.
        assert!(is_stable(now, now, now, (now - 3600) * 1_000_000_000));
        // Fresh file: needs 60 s and a second look.
        let fresh = now * 1_000_000_000;
        assert!(!is_stable(now, now - 30, now, fresh));
        assert!(!is_stable(now, now - 90, now - 90, fresh));
        assert!(is_stable(now, now - 90, now, fresh));
    }

    #[test]
    fn depths() {
        assert_eq!(depth_below("/media/tv shows", "/media/tv shows"), 0);
        assert_eq!(depth_below("/media/tv shows", "/media/tv shows/X (2000)/Season 1"), 2);
    }

    #[test]
    fn walks_a_tree() {
        let dir = std::env::temp_dir().join(format!("moonrise-scan-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("tv");
        std::fs::create_dir_all(root.join("Show (2000)/Season 1/Featurettes")).unwrap();
        std::fs::create_dir_all(root.join("DVD Movie/VIDEO_TS")).unwrap();
        std::fs::write(root.join("Show (2000)/Season 1/Show S01E01.mkv"), b"x").unwrap();
        std::fs::write(root.join("Show (2000)/Season 1/Show S01E01.en.srt"), b"x").unwrap();
        std::fs::write(root.join("Show (2000)/Season 1/Featurettes/Making of.mkv"), b"x").unwrap();
        std::fs::write(root.join("DVD Movie/VIDEO_TS/VTS_01_1.VOB"), b"xyz").unwrap();
        let r = root.to_string_lossy().into_owned();
        let libs = vec![LibraryConfig {
            id: "shows".into(),
            name: "Shows".into(),
            kind: "shows".into(),
            roots: vec![r.clone()],
        }];
        let none = HashSet::new();
        let fs = scan_fs(&libs, true, &HashMap::new(), &[], &none);
        assert!(fs.offline.is_empty() && fs.online == vec![r.clone()]);
        let mut names: Vec<String> = fs.files.iter().map(|f| f.1.strip_prefix(&r).unwrap().to_string()).collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "/DVD Movie",
                "/Show (2000)/Season 1/Featurettes/Making of.mkv",
                "/Show (2000)/Season 1/Show S01E01.mkv"
            ]
        );
        assert_eq!(fs.files.iter().find(|f| f.1.ends_with("DVD Movie")).unwrap().2, 3);
        // Quick pass with everything known and unchanged: nothing listed.
        let known: HashMap<String, i64> = fs.dir_mtimes.iter().cloned().collect();
        let q = scan_fs(&libs, false, &known, &[], &none);
        assert_eq!((q.dirs_listed, q.files.len()), (0, 0));
        assert!(q.dirs_checked >= 3);
        // Back from offline: walked in full in a quick pass.
        let back: HashSet<String> = [r.clone()].into();
        assert_eq!(scan_fs(&libs, false, &known, &[], &back).files.len(), 3);
        // A missing or empty root is offline; the other roots are still scanned.
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let mut two = libs.clone();
        two[0].roots.push(dir.join("nope").to_string_lossy().into_owned());
        two.push(LibraryConfig {
            id: "x".into(),
            name: "X".into(),
            kind: "movies".into(),
            roots: vec![empty.to_string_lossy().into_owned()],
        });
        let fs = scan_fs(&two, true, &HashMap::new(), &[], &none);
        assert_eq!(fs.offline.len(), 2);
        assert_eq!(fs.online, vec![r.clone()]);
        assert_eq!(fs.files.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
