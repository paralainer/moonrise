//! The library: the files on the shares (`scanner`), what they are (`parse`, `identify`),
//! TMDB metadata and artwork (`metadata`, `tmdb`, `images`), search (`search`), and the browse API
//! (`api`).
//!
//! One background loop runs passes: a quick scan every `performance.scan_quick_secs`, a full one
//! every `performance.scan_full_hours` (and at start, and when the libraries change); each pass identifies what is new, fetches due
//! metadata and rebuilds the search index if anything changed. Probing (ffprobe, the
//! OpenSubtitles hash), artwork fetching and ratings (`ratings`) run in their own gentle loops.

pub mod api;
pub mod browse;
pub mod community;
#[cfg(test)]
mod golden_tests;
pub mod identify;
pub mod images;
pub mod local;
pub mod match_posters;
pub mod metadata;
pub mod parse;
pub mod parts;
pub mod ratings;
pub mod scanner;
pub mod search;
pub(crate) mod store;
pub mod streams;
pub mod tmdb;
pub mod util;
pub mod versions;

use std::{
    collections::HashMap,
    path::Path,
    sync::{atomic::AtomicUsize, Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};

use crate::{
    admin::config::{Config, LibraryConfig},
    db::Db,
    llm::Llm,
    media::item,
    storage::cache::Cache,
};

/// File roles that are titles (a broken probe is still a title).
pub const TITLE_ROLES: &str = "('main', 'broken')";

/// Marks a library file whose contents turned out damaged after its probe went through (a
/// Matroska file whose index isn't there: zeros, cut short): role `broken`, with why in
/// `probe_error`, as a file ffprobe can't open. Only while it is unchanged (`size`, `mtime`): the
/// scanner probes a changed file (a re-download) again, and a good probe makes it `main` with no
/// error. The keyframe index backfill passes over files with a `probe_error`. Returns whether a
/// file was marked.
pub async fn mark_damaged(db: &Db, path: String, size: u64, mtime: i64, why: String) -> Result<bool> {
    db.call(move |c| {
        let n = c.execute(
            "UPDATE files SET role = 'broken', probe_error = ?4
             WHERE path = ?1 AND size = ?2 AND mtime = ?3 AND role IN ('main', 'broken') AND missing_since IS NULL",
            params![path, size as i64, mtime, why],
        )?;
        Ok(n > 0)
    })
    .await
}

/// The file playback serves for a title (`?1`), given the version asked for (`?2`, or empty):
/// that one, unless it is `broken` and the title has a healthy file; otherwise the largest
/// healthy file; a `broken` one only when there is nothing else.
fn playback_file_sql() -> String {
    format!(
        "SELECT id, path, size, mtime, duration, bitrate, container FROM files WHERE item_id = ?1 AND role IN {TITLE_ROLES} AND missing_since IS NULL
         ORDER BY role = 'broken', id = ?2 DESC, size DESC LIMIT 1"
    )
}

pub struct Library {
    pub db: Db,
    /// As the server started (static things: directories, ffprobe; the switches are shared).
    pub config: Config,
    /// What the settings can change while running (`reconfigure`).
    live: std::sync::RwLock<Live>,
    /// Wakes the scan loop for a pass now (`force_full`: a full one).
    rescan: tokio::sync::Notify,
    force_full: std::sync::atomic::AtomicBool,
    pub images: Arc<images::Images>,
    /// Fix Match's candidate posters.
    pub match_posters: match_posters::MatchPosters,
    pub probe_wake: tokio::sync::Notify,
    /// A pass found new or changed titles (the subtitle backfill looks again).
    pub files_changed: tokio::sync::Notify,
    /// Probes of new or changed files were made (the keyframe index backfill looks again).
    pub probed: tokio::sync::Notify,
    /// Folder listings (sidecars): folder → (names, listed at).
    folders: Arc<Mutex<HashMap<String, (Arc<Vec<String>>, Instant)>>>,
    /// Each root's state after the last pass (offline roots are left out of passes).
    roots: Mutex<HashMap<String, RootState>>,
    /// LLM calls in the current pass (capped by `MOONRISE_LLM_MAX_CALLS_PER_SCAN`).
    pub llm_calls: AtomicUsize,
    pass_lock: tokio::sync::Mutex<()>,
    pub last: Mutex<Passes>,
    /// What the pass running now is doing (`GET /v1/admin/scan/progress`).
    pub activity: Mutex<Activity>,
    /// Files to identify in this pass, and done so far.
    pub identify_total: AtomicUsize,
    pub identify_done: AtomicUsize,
    /// Roots to walk in full in the next pass (a library's "Scan Now").
    walk_now: Mutex<std::collections::HashSet<String>>,
    /// IMDb, Rotten Tomatoes and Metacritic ratings, fetched in the background (`ratings`).
    pub ratings: Arc<ratings::Ratings>,
    /// The community databases' segment times (`community`).
    pub community: Arc<community::Community>,
}

/// The current pass, for the setup screen's live progress.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Activity {
    /// `idle`, `scanning`, `identifying`, `metadata` or `finishing`.
    pub phase: &'static str,
    pub full: bool,
    /// When the pass started (Unix seconds).
    pub started_at: Option<i64>,
}

/// The library's settings that apply without a restart.
struct Live {
    libs: Vec<LibraryConfig>,
    tmdb: Option<Arc<tmdb::Tmdb>>,
    /// (key, language, country) the client was made with.
    tmdb_conf: (crate::admin::config::Secret, String, String),
    llm: Option<Arc<Llm>>,
    llm_conf: crate::admin::config::LlmConfig,
    scan_quick_secs: u64,
    scan_full_hours: u64,
    scan_probe_jobs: usize,
}

impl Live {
    fn new(config: &Config, db: &Db) -> Live {
        let tmdb_conf = (config.tmdb_key.clone(), config.tmdb_language.clone(), config.tmdb_country.clone());
        Live {
            libs: config.libraries.clone(),
            tmdb: (!config.tmdb_key.expose().is_empty()).then(|| {
                Arc::new(tmdb::Tmdb::new(
                    config.tmdb_key.clone(),
                    db.clone(),
                    config.tmdb_language.clone(),
                    config.tmdb_country.clone(),
                ))
            }),
            tmdb_conf,
            llm: config.llm.enabled.then(|| Arc::new(Llm::new(config.llm.clone()))),
            llm_conf: config.llm.clone(),
            scan_quick_secs: config.scan_quick_secs,
            scan_full_hours: config.scan_full_hours,
            scan_probe_jobs: config.scan_probe_jobs,
        }
    }
}

/// Whether a root answered in the last pass.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RootState {
    pub online: bool,
    /// Why it is offline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Since when it is in this state (Unix seconds).
    pub since: i64,
    pub checked_at: i64,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Passes {
    pub last_quick: Option<PassReport>,
    pub last_full: Option<PassReport>,
    pub passes: u64,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct PassReport {
    pub at: i64,
    pub scan: scanner::ScanReport,
    pub identified: usize,
    pub metadata: usize,
    pub llm_calls: usize,
    pub ms: u64,
    pub error: Option<String>,
}

/// Counts a call to a paid or rate-limited service (per day).
pub async fn count_usage(db: &Db, service: &str, input_tokens: i64, output_tokens: i64) {
    let day = chrono_day(util::now());
    let s = service.to_string();
    let _ = db
        .call(move |c| {
            c.execute(
                "INSERT INTO usage(day, service, calls, input_tokens, output_tokens) VALUES (?1, ?2, 1, ?3, ?4)
                 ON CONFLICT(day, service) DO UPDATE SET calls = calls + 1, input_tokens = input_tokens + ?3, output_tokens = output_tokens + ?4",
                params![day, s, input_tokens, output_tokens],
            )?;
            Ok(())
        })
        .await;
}

/// `YYYY-MM-DD` (UTC) of Unix seconds.
pub fn chrono_day(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

/// RFC 3339 with milliseconds (UTC) of Unix seconds.
pub fn rfc3339(secs: i64) -> String {
    let t = secs.rem_euclid(86_400);
    format!("{}T{:02}:{:02}:{:02}.000Z", chrono_day(secs), t / 3600, t % 3600 / 60, t % 60)
}

impl Library {
    pub fn new(config: Config, db: Db, cache: Arc<Cache>) -> Self {
        Library {
            match_posters: match_posters::MatchPosters::new(cache.clone()),
            images: Arc::new(images::Images::new(
                config.data_dir.join("images"),
                db.clone(),
                cache,
                config.ffmpeg.clone(),
            )),
            live: std::sync::RwLock::new(Live::new(&config, &db)),
            ratings: ratings::Ratings::new(db.clone(), &config),
            community: community::Community::new(db.clone(), &config),
            rescan: tokio::sync::Notify::new(),
            force_full: std::sync::atomic::AtomicBool::new(false),
            db,
            config,
            probe_wake: tokio::sync::Notify::new(),
            files_changed: tokio::sync::Notify::new(),
            probed: tokio::sync::Notify::new(),
            folders: Arc::new(Mutex::new(HashMap::new())),
            roots: Mutex::new(HashMap::new()),
            llm_calls: AtomicUsize::new(0),
            pass_lock: tokio::sync::Mutex::new(()),
            last: Mutex::new(Passes::default()),
            activity: Mutex::new(Activity::default()),
            identify_total: AtomicUsize::new(0),
            identify_done: AtomicUsize::new(0),
            walk_now: Mutex::new(std::collections::HashSet::new()),
        }
    }

    /// The configured libraries.
    pub fn libs(&self) -> Vec<LibraryConfig> {
        self.live.read().unwrap().libs.clone()
    }

    /// The roots' states after the last pass (a root not scanned yet isn't listed).
    pub fn root_states(&self) -> HashMap<String, RootState> {
        self.roots.lock().unwrap().clone()
    }

    /// Roots that were offline in the last pass.
    pub fn offline_roots(&self) -> std::collections::HashSet<String> {
        self.roots
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, s)| !s.online)
            .map(|(r, _)| r.clone())
            .collect()
    }

    /// Whether the file is under a root that was offline in the last pass.
    pub fn offline_path(&self, path: &str) -> Option<String> {
        self.roots
            .lock()
            .unwrap()
            .iter()
            .find(|(r, s)| !s.online && path.starts_with(&format!("{r}/")))
            .map(|(r, s)| format!("{r} is offline: {}", s.error.as_deref().unwrap_or("")))
    }

    pub(crate) fn set_root_states(&self, online: &[String], offline: &[(String, String)]) {
        let now = util::now();
        let mut m = self.roots.lock().unwrap();
        for r in online {
            let was = m.get(r).cloned();
            if was.as_ref().is_some_and(|w| !w.online) {
                tracing::info!(root = %r, "library root back online");
            }
            let since = was.filter(|w| w.online).map_or(now, |w| w.since);
            m.insert(
                r.clone(),
                RootState {
                    online: true,
                    error: None,
                    since,
                    checked_at: now,
                },
            );
        }
        for (r, why) in offline {
            let was = m.get(r).cloned();
            if was.as_ref().is_none_or(|w| w.online || w.error.as_deref() != Some(why.as_str())) {
                tracing::warn!(root = %r, "library root offline (left out of scans; its titles are kept): {why}");
            }
            let since = was.filter(|w| !w.online).map_or(now, |w| w.since);
            m.insert(
                r.clone(),
                RootState {
                    online: false,
                    error: Some(why.clone()),
                    since,
                    checked_at: now,
                },
            );
        }
        let all: std::collections::HashSet<&String> = online.iter().chain(offline.iter().map(|(r, _)| r)).collect();
        m.retain(|r, _| all.contains(r));
    }

    /// The TMDB client (None: no key).
    pub fn tmdb(&self) -> Option<Arc<tmdb::Tmdb>> {
        self.live.read().unwrap().tmdb.clone()
    }

    /// The LLM (None: not configured).
    pub fn llm(&self) -> Option<Arc<Llm>> {
        self.live.read().unwrap().llm.clone()
    }

    fn scan_probe_jobs(&self) -> usize {
        self.live.read().unwrap().scan_probe_jobs
    }

    /// Applies changed settings: libraries (their tables rewritten, then a full pass), the TMDB
    /// key, language and country, the LLM, scan intervals. Answers what changed.
    pub fn reconfigure(self: &Arc<Self>, c: &Config) -> Vec<&'static str> {
        let mut changed = Vec::new();
        self.ratings.set_key(&c.omdb_key);
        let mut live = self.live.write().unwrap();
        let tmdb_conf = (c.tmdb_key.clone(), c.tmdb_language.clone(), c.tmdb_country.clone());
        if tmdb_conf != live.tmdb_conf {
            live.tmdb = (!c.tmdb_key.expose().is_empty()).then(|| {
                Arc::new(tmdb::Tmdb::new(
                    c.tmdb_key.clone(),
                    self.db.clone(),
                    c.tmdb_language.clone(),
                    c.tmdb_country.clone(),
                ))
            });
            live.tmdb_conf = tmdb_conf;
            changed.push("tmdb");
        }
        if c.llm != live.llm_conf {
            live.llm = c.llm.enabled.then(|| Arc::new(Llm::new(c.llm.clone())));
            live.llm_conf = c.llm.clone();
            changed.push("llm");
        }
        if (live.scan_quick_secs, live.scan_full_hours, live.scan_probe_jobs) != (c.scan_quick_secs, c.scan_full_hours, c.scan_probe_jobs) {
            (live.scan_quick_secs, live.scan_full_hours, live.scan_probe_jobs) = (c.scan_quick_secs, c.scan_full_hours, c.scan_probe_jobs);
            changed.push("scan");
        }
        let libs_changed = live.libs != c.libraries;
        if libs_changed {
            live.libs.clone_from(&c.libraries);
            changed.push("libraries");
        }
        drop(live);
        if libs_changed {
            // Between passes: the tables first, then a full pass (new roots walked, removed
            // libraries' titles gone).
            let this = self.clone();
            tokio::spawn(async move {
                {
                    let _g = this.pass_lock.lock().await;
                    if let Err(e) = this.init().await {
                        tracing::warn!("libraries: {e:#}");
                    }
                }
                this.request_pass(true);
            });
        } else if changed.contains(&"tmdb") || changed.contains(&"llm") || changed.contains(&"scan") {
            // What waited for a key is looked at again; new intervals start now.
            self.request_pass(false);
        }
        if !changed.is_empty() {
            tracing::info!(?changed, libraries = ?self.libs(), tmdb = self.tmdb().is_some(), llm = ?self.llm().map(|l| l.describe()), "library settings applied");
        }
        changed
    }

    fn set_phase(&self, phase: &'static str, full: bool, started_at: Option<i64>) {
        *self.activity.lock().unwrap() = Activity { phase, full, started_at };
    }

    /// A full pass was asked for and hasn't started yet.
    pub fn pass_queued(&self) -> bool {
        self.force_full.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Live progress of the library's passes (the setup screen polls it): what the pass running
    /// now does, the files its walk has seen, identification done of total, and per library the
    /// files and titles so far.
    pub async fn progress(&self) -> Result<Value> {
        let act = self.activity.lock().unwrap().clone();
        let phase = if act.phase.is_empty() { "idle" } else { act.phase };
        let passes = self.last.lock().unwrap().clone();
        let walked = scanner::WALK_FILES.load(std::sync::atomic::Ordering::Relaxed);
        let libs = self.libs();
        let now = util::now();
        let per_lib = self
            .db
            .call(move |c| {
                let mut out = Vec::new();
                for l in &libs {
                    let id = l.id.clone();
                    let one = |sql: &str| -> rusqlite::Result<i64> { c.query_row(sql, [&id], |r| r.get(0)) };
                    let files = one(&format!("SELECT COUNT(*) FROM files WHERE library_id = ?1 AND missing_since IS NULL AND role IN {TITLE_ROLES}"))?;
                    let identified = one(&format!("SELECT COUNT(*) FROM files WHERE library_id = ?1 AND missing_since IS NULL AND role IN {TITLE_ROLES} AND item_id IS NOT NULL"))?;
                    let visible = format!("EXISTS (SELECT 1 FROM files f WHERE f.item_id = e.id AND f.role IN {TITLE_ROLES} AND f.missing_since IS NULL)");
                    let movies = one(&format!("SELECT COUNT(*) FROM items e WHERE e.library_id = ?1 AND e.kind = 'movie' AND {visible}"))?;
                    let episodes = one(&format!("SELECT COUNT(*) FROM items e WHERE e.library_id = ?1 AND e.kind = 'episode' AND {visible}"))?;
                    let series = one(&format!(
                        "SELECT COUNT(*) FROM items s WHERE s.library_id = ?1 AND s.kind = 'series' AND EXISTS (SELECT 1 FROM items e WHERE e.series_id = s.id AND e.kind = 'episode' AND {visible})"
                    ))?;
                    // Titles (movies, shows) and episodes waiting in Fix Match.
                    let review = one("SELECT COUNT(*) FROM items WHERE library_id = ?1 AND identify_state = 'review' AND kind IN ('movie', 'series')")?;
                    let review_episodes = one(
                        "SELECT COUNT(*) FROM items e WHERE e.library_id = ?1 AND e.identify_state = 'review' AND e.kind = 'episode'
                         AND NOT EXISTS (SELECT 1 FROM items s WHERE s.id = e.series_id AND s.identify_state = 'review')",
                    )?;
                    // New files still being written wait (a few minutes) before they are identified.
                    let mut st = c.prepare(&format!(
                        "SELECT stable_since, last_seen, mtime FROM files WHERE library_id = ?1 AND missing_since IS NULL AND role IN {TITLE_ROLES} AND identified_at IS NULL"
                    ))?;
                    let settling = st
                        .query_map([&id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))?
                        .filter_map(std::result::Result::ok)
                        .filter(|(ss, ls, m)| !scanner::is_stable(now, *ss, *ls, *m))
                        .count();
                    out.push(json!({"id": l.id, "name": l.name, "kind": l.kind, "files": files, "identified": identified, "movies": movies,
                                    "series": series, "episodes": episodes, "review": review, "review_episodes": review_episodes, "settling": settling}));
                }
                Ok(out)
            })
            .await?;
        let last = passes.last_full.as_ref().or(passes.last_quick.as_ref()).map(|r| {
            json!({"at": r.at, "ms": r.ms, "error": r.error, "aborted": r.scan.aborted, "offline": r.scan.offline, "files_seen": r.scan.files_seen})
        });
        Ok(json!({
            "phase": if phase == "idle" && self.pass_queued() { "queued" } else { phase },
            "full": act.full,
            "started_at": act.started_at,
            "walked": walked,
            "identify": {"done": self.identify_done.load(std::sync::atomic::Ordering::Relaxed), "total": self.identify_total.load(std::sync::atomic::Ordering::Relaxed)},
            "passes": passes.passes,
            "last_pass": last,
            "roots": self.root_states(),
            "libraries": per_lib,
        }))
    }

    /// Runs a pass now (`full`: a full one) instead of at the next interval.
    pub fn request_pass(&self, full: bool) {
        if full {
            self.force_full.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.rescan.notify_one();
    }

    /// One library walked in full now (its folders listed again; the others get a quick pass).
    /// False: no such library.
    pub fn request_library_scan(&self, id: &str) -> bool {
        let Some(l) = self.libs().into_iter().find(|l| l.id == id) else {
            return false;
        };
        self.walk_now.lock().unwrap().extend(l.roots);
        self.rescan.notify_one();
        true
    }

    /// The roots asked to be walked in full (taken: the pass about to run walks them).
    pub(crate) fn take_walk_now(&self) -> std::collections::HashSet<String> {
        std::mem::take(&mut *self.walk_now.lock().unwrap())
    }

    /// Writes the configured libraries to the database.
    pub async fn init(&self) -> Result<()> {
        let libs = self.libs();
        self.db
            .call(move |c| {
                let tx = c.transaction()?;
                tx.execute("DELETE FROM libraries", [])?;
                for (i, l) in libs.iter().enumerate() {
                    tx.execute(
                        "INSERT INTO libraries(id, name, kind, roots, sort) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![l.id, l.name, l.kind, json!(l.roots).to_string(), i as i64],
                    )?;
                }
                // A library taken out of the configuration takes its files and titles with it
                // (scan_state rows of its roots go too).
                let gone_files = tx.execute("DELETE FROM files WHERE library_id NOT IN (SELECT id FROM libraries)", [])?;
                let gone_items = tx.execute("DELETE FROM items WHERE library_id NOT IN (SELECT id FROM libraries)", [])?;
                if gone_files + gone_items > 0 {
                    tracing::info!(files = gone_files, items = gone_items, "removed with their library");
                    tx.execute("DELETE FROM item_people WHERE item_id NOT IN (SELECT id FROM items)", [])?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
    }

    /// Starts the background loops (scan passes, probing, artwork).
    pub fn spawn(self: &Arc<Self>) {
        tracing::info!(libraries = ?self.libs(), tmdb = self.tmdb().is_some(), llm = ?self.llm().map(|l| l.describe()), "library");
        let this = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let mut last_full: Option<Instant> = None;
            loop {
                let (quick_secs, full_hours) = {
                    let l = this.live.read().unwrap();
                    (l.scan_quick_secs, l.scan_full_hours)
                };
                let forced = this.force_full.swap(false, std::sync::atomic::Ordering::Relaxed);
                let full = forced || last_full.is_none_or(|t| t.elapsed() >= Duration::from_secs(full_hours.max(1) * 3600));
                match this.pass(full).await {
                    Ok(r) if full && r.scan.aborted.is_none() => last_full = Some(Instant::now()),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("library pass: {e:#}"),
                }
                // New files waiting for their second look come back sooner (just past the
                // stability gate) than the next quick pass.
                let settling: i64 = this
                    .db
                    .call(|c| {
                        Ok(c.query_row(
                            &format!("SELECT COUNT(*) FROM files WHERE missing_since IS NULL AND role IN {TITLE_ROLES} AND identified_at IS NULL"),
                            [],
                            |r| r.get(0),
                        )?)
                    })
                    .await
                    .unwrap_or(0);
                let wait = if settling > 0 { 65.min(quick_secs) } else { quick_secs };
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_secs(wait.max(10))) => {}
                    () = this.rescan.notified() => {}
                }
            }
        });
        let this = self.clone();
        tokio::spawn(async move { this.probe_loop().await });
        let images = self.images.clone();
        tokio::spawn(async move { images.prefetch_loop().await });
        self.ratings.spawn();
        self.community.spawn();
        // Collections of movies identified before Phase 7 (after the first pass has settled).
        let this = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if let Err(e) = this.backfill_collections().await {
                tracing::warn!("collection backfill: {e:#}");
            }
        });
    }

    /// One pass: scan, identify, metadata, search index.
    pub async fn pass(&self, full: bool) -> Result<PassReport> {
        let _g = self.pass_lock.lock().await;
        let t0 = Instant::now();
        self.llm_calls.store(0, std::sync::atomic::Ordering::Relaxed);
        let mut r = PassReport {
            at: util::now(),
            ..Default::default()
        };
        self.identify_total.store(0, std::sync::atomic::Ordering::Relaxed);
        self.identify_done.store(0, std::sync::atomic::Ordering::Relaxed);
        self.set_phase("scanning", full, Some(r.at));
        let result: Result<()> = async {
            r.scan = self.scan(full).await?;
            if r.scan.aborted.is_some() {
                return Ok(());
            }
            if r.scan.new_files + r.scan.changed + r.scan.renamed > 0 {
                self.probe_wake.notify_one();
            }
            self.link_match_hints().await?;
            self.set_phase("identifying", full, Some(r.at));
            r.identified = self.identify_pending().await?;
            self.cleanup_items().await?;
            // Split and double episodes, and numbering ahead of TMDB's (parts.rs).
            if let Err(e) = self.align_after_identify(r.at).await {
                tracing::warn!("parts: {e:#}");
            }
            // What the NFOs say of the versions of titles with several files.
            if let Err(e) = self.refresh_editions(200).await {
                tracing::warn!("editions: {e:#}");
            }
            self.set_phase("metadata", full, Some(r.at));
            r.metadata = self.refresh_metadata(false).await?;
            // What the files and folders say, where TMDB says nothing (gently: a batch a pass).
            r.metadata += self.refresh_local(400).await.unwrap_or_else(|e| {
                tracing::warn!("local metadata: {e:#}");
                0
            });
            self.set_phase("finishing", full, Some(r.at));
            if full {
                self.migrate_fetched_subtitles().await?;
            }
            if r.scan.changed_anything() || r.identified + r.metadata > 0 || full {
                self.db.call(search::rebuild).await?;
            }
            if r.scan.changed_anything() || r.identified > 0 {
                self.files_changed.notify_one();
            }
            self.refresh_video_summary().await;
            Ok(())
        }
        .await;
        if let Err(e) = &result {
            r.error = Some(format!("{e:#}"));
        }
        r.llm_calls = self.llm_calls.load(std::sync::atomic::Ordering::Relaxed);
        r.ms = t0.elapsed().as_millis() as u64;
        self.set_phase("idle", false, None);
        {
            let mut last = self.last.lock().unwrap();
            last.passes += 1;
            if full {
                last.last_full = Some(r.clone());
            } else {
                last.last_quick = Some(r.clone());
            }
        }
        if full || r.identified + r.metadata > 0 || r.scan.changed_anything() {
            tracing::info!(full, identified = r.identified, metadata = r.metadata, llm_calls = r.llm_calls, ms = r.ms, error = ?r.error, "library pass");
        }
        result.map(|()| r)
    }

    /// Subtitles fetched from OpenSubtitles were kept by the file's path (`p<hash>`): move them
    /// to the file's id (`f-<id>`), which survives renames.
    async fn migrate_fetched_subtitles(&self) -> Result<()> {
        let dir = self.config.data_dir.join("subs");
        let files: Vec<(String, String)> = self
            .db
            .call(|c| {
                let mut st = c.prepare("SELECT id, path FROM files WHERE missing_since IS NULL")?;
                let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        tokio::task::spawn_blocking(move || {
            for (id, path) in files {
                let legacy = dir.join(format!("p{:016x}", crate::media::fnv(&path)));
                let new = dir.join(format!("f-{id}"));
                if legacy.is_dir() && !new.exists() {
                    match std::fs::rename(&legacy, &new) {
                        Ok(()) => tracing::info!(path, "fetched subtitles moved to the file's id"),
                        Err(e) => tracing::warn!(path, "move fetched subtitles: {e}"),
                    }
                }
            }
        })
        .await?;
        Ok(())
    }

    /// Counts and state for `moonrise-server stats` / `GET /v1/admin/stats`.
    pub async fn stats(&self, recent_hours: Option<i64>) -> Result<Value> {
        let passes = self.last.lock().unwrap().clone();
        let mut v = self
            .db
            .call(move |c| {
                let q = |sql: &str| -> rusqlite::Result<Vec<(String, String, i64)>> {
                    let mut st = c.prepare(sql)?;
                    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect();
                    rows
                };
                let visible = format!(
                    "EXISTS (SELECT 1 FROM files f WHERE f.item_id = e.id AND f.role IN {TITLE_ROLES} AND (f.missing_since IS NULL OR f.missing_since > {}))",
                    util::now() - 86_400
                );
                let mut libs = serde_json::Map::new();
                for (lib, kind, n) in q(&format!("SELECT e.library_id, e.kind, COUNT(*) FROM items e WHERE e.kind IN ('movie', 'episode') AND {visible} GROUP BY 1, 2"))? {
                    libs.entry(lib).or_insert_with(|| json!({})).as_object_mut().unwrap().insert(format!("{kind}s"), json!(n));
                }
                for (lib, kind, n) in q(&format!(
                    "SELECT s.library_id, s.kind, COUNT(*) FROM items s WHERE s.kind IN ('series', 'season') AND EXISTS (SELECT 1 FROM items e WHERE (e.series_id = s.id OR e.parent_id = s.id) AND e.kind = 'episode' AND {visible}) GROUP BY 1, 2"
                ))? {
                    let key = if kind == "series" { "series".to_string() } else { "seasons".to_string() };
                    libs.entry(lib).or_insert_with(|| json!({})).as_object_mut().unwrap().insert(key, json!(n));
                }
                let mut files = serde_json::Map::new();
                for (lib, role, n) in q("SELECT library_id, role || CASE WHEN missing_since IS NULL THEN '' ELSE ' (missing)' END, COUNT(*) FROM files GROUP BY 1, 2")? {
                    files.entry(lib).or_insert_with(|| json!({})).as_object_mut().unwrap().insert(role, json!(n));
                }
                let mut states = serde_json::Map::new();
                for (kind, state, n) in q("SELECT kind, identify_state, COUNT(*) FROM items GROUP BY 1, 2")? {
                    states.entry(kind).or_insert_with(|| json!({})).as_object_mut().unwrap().insert(state, json!(n));
                }
                let pending_probe: i64 = c.query_row(
                    &format!("SELECT COUNT(*) FROM files WHERE missing_since IS NULL AND role IN {TITLE_ROLES} AND (probed_stamp IS NULL OR probed_stamp != size || '|' || mtime)"),
                    [],
                    |r| r.get(0),
                )?;
                let unidentified: i64 = c.query_row(&format!("SELECT COUNT(*) FROM files WHERE missing_since IS NULL AND role IN {TITLE_ROLES} AND item_id IS NULL"), [], |r| r.get(0))?;
                let images: (i64, i64, i64) = c.query_row("SELECT COUNT(*), COUNT(path), COALESCE(SUM(bytes), 0) FROM images", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                let mut usage = Vec::new();
                {
                    let mut st = c.prepare("SELECT day, service, calls, input_tokens, output_tokens FROM usage ORDER BY day DESC, service LIMIT 30")?;
                    for r in st.query_map([], |r| Ok(json!({"day": r.get::<_, String>(0)?, "service": r.get::<_, String>(1)?, "calls": r.get::<_, i64>(2)?, "input_tokens": r.get::<_, i64>(3)?, "output_tokens": r.get::<_, i64>(4)?})))? {
                        usage.push(r?);
                    }
                }
                let hints: i64 = c.query_row("SELECT COUNT(*) FROM match_hints", [], |r| r.get(0))?;
                let mut recent = Vec::new();
                if let Some(h) = recent_hours {
                    let mut st = c.prepare(
                        "SELECT f.path, f.role, datetime(f.first_seen, 'unixepoch'), i.kind, i.title, i.season_no, i.episode_no, i.identify_state
                         FROM files f LEFT JOIN items i ON i.id = f.item_id WHERE f.first_seen > ?1 ORDER BY f.first_seen DESC LIMIT 100",
                    )?;
                    for r in st.query_map([util::now() - h * 3600], |r| {
                        Ok(json!({"path": r.get::<_, String>(0)?, "role": r.get::<_, String>(1)?, "first_seen": r.get::<_, String>(2)?, "kind": r.get::<_, Option<String>>(3)?,
                                  "title": r.get::<_, Option<String>>(4)?, "season": r.get::<_, Option<i64>>(5)?, "episode": r.get::<_, Option<i64>>(6)?, "state": r.get::<_, Option<String>>(7)?}))
                    })? {
                        recent.push(r?);
                    }
                }
                Ok(json!({
                    "libraries": libs, "files": files, "identify_states": states, "pending_probe": pending_probe, "unidentified": unidentified,
                    "images": {"known": images.0, "stored": images.1, "bytes": images.2}, "usage": usage, "match_hints": hints, "recent": recent,
                }))
            })
            .await?;
        v["passes"] = serde_json::to_value(passes)?;
        v["llm"] = json!(self.llm().map(|l| l.describe()));
        v["tmdb"] = json!(self.tmdb().is_some());
        Ok(v)
    }

    /// The Moonrise id an id stands for (following merges).
    pub async fn resolve_id(&self, id: &str) -> Result<String> {
        let id = id.to_string();
        self.db
            .call(move |c| {
                let mut cur = id;
                for _ in 0..5 {
                    match c
                        .query_row("SELECT new_id FROM item_redirects WHERE old_id = ?1", [&cur], |r| {
                            r.get::<_, String>(0)
                        })
                        .optional()?
                    {
                        Some(n) => cur = n,
                        None => break,
                    }
                }
                Ok(cur)
            })
            .await
    }

    /// What playback needs for an id, from the library alone: the item with its best file and
    /// that file's tracks. None: not known here.
    /// `id` may name a version: `<item>~<file>` (that file; another of the title's when it is gone,
    /// or when it is `broken` (damaged) and the title has a healthy one: an older app, or a
    /// version chosen before the damage was found). Otherwise the largest file, one not `broken`
    /// first.
    pub async fn playback_item(&self, id: &str) -> Result<Option<item::Item>> {
        let (base, version) = match id.split_once('~') {
            Some((b, v)) => (b, Some(v.to_string())),
            None => (id, None),
        };
        let id = self.resolve_id(base).await?;
        let row = {
            let id = id.clone();
            let version = version.clone();
            self.db
                .call(move |c| {
                    let item: Option<(String, String, Option<String>, Option<String>, Option<i64>, Option<i64>)> = c
                        .query_row(
                            "SELECT kind, title, provider_ids, series_id, season_no, episode_no FROM items WHERE id = ?1 AND kind IN ('movie', 'episode')",
                            [&id],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                        )
                        .optional()?;
                    let file: Option<(String, String, i64, i64, Option<f64>, Option<i64>, Option<String>)> = c
                        .query_row(&playback_file_sql(), rusqlite::params![id, version.unwrap_or_default()], |r| {
                            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
                        })
                        .optional()?;
                    Ok(item.zip(file))
                })
                .await?
        };
        let Some(((kind, title, ids, series_id, season, episode), (file_id, path, size, mtime, duration, bitrate, container))) = row else {
            return Ok(None);
        };
        // (Known from the scan: asking the share is slow while it is busy.)
        if container.as_deref() == Some("dvd") {
            anyhow::bail!("DVD folders aren't served yet");
        }
        let streams = self.streams_for(&path, size, mtime).await?;
        // A version asked for keeps its key (its media, subtitle and download addresses carry it).
        let key = match &version {
            Some(v) if *v == file_id => format!("{id}~{v}"),
            Some(v) => {
                tracing::info!(item = %id, version = %v, playing = %file_id, "that version is gone or damaged: playing the title's default");
                id.clone()
            }
            None => id.clone(),
        };
        let mut item = item::Item {
            id: key,
            name: Some(title),
            kind: Some(if kind == "movie" { "Movie".into() } else { "Episode".into() }),
            provider_ids: ids_map(ids.as_deref()),
            parent_index_number: season,
            index_number: episode,
            media_sources: vec![item::MediaSource {
                id: file_id,
                path: Some(path),
                run_time_ticks: duration.map(|d| (d * 1e7) as i64),
                bitrate,
                media_streams: streams.iter().map(streams::Stream::to_item_stream).collect(),
                stamp: Some((size as u64, mtime)),
            }],
            ..Default::default()
        };
        if let Some(sid) = series_id {
            self.attach_series(&mut item, &sid).await?;
            item.series_id = Some(sid);
        }
        Ok(Some(item))
    }

    async fn attach_series(&self, item: &mut item::Item, series_id: &str) -> Result<()> {
        let sid = series_id.to_string();
        let s: Option<(String, Option<String>)> = self
            .db
            .call(move |c| {
                Ok(c.query_row("SELECT title, provider_ids FROM items WHERE id = ?1", [sid], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?)
            })
            .await?;
        if let Some((title, ids)) = s {
            item.series_name = Some(title);
            item.series_provider_ids = ids_map(ids.as_deref());
        }
        Ok(())
    }

    /// The file's tracks, from the stored probe (probed now if there is none).
    pub async fn streams_for(&self, path: &str, size: i64, mtime: i64) -> Result<Vec<streams::Stream>> {
        let probe = match self.db.probe_get(path.to_string(), size as u64, mtime).await? {
            Some(p) => p.probe,
            None => {
                let v = crate::media::probe_path(&self.config.ffprobe, path, false).await.context("probe")?;
                self.db.probe_put(path.to_string(), size as u64, mtime, v.clone()).await?;
                // probe_put keeps chapters apart; streams don't need them.
                v
            }
        };
        let names = match crate::storage::vfs::parent(path) {
            Some(dir) => self.folder_names(dir).await,
            None => Arc::new(Vec::new()),
        };
        let sidecars = streams::sidecars_in(Path::new(path), &names);
        Ok(streams::derive(Path::new(path), &probe, &sidecars))
    }

    /// Lists every folder holding titles (in the background, a few seconds), so the first play
    /// of a title doesn't wait for its folder's listing.
    pub async fn warm_folders(&self) {
        let dirs: Vec<String> = self
            .db
            .call(|c| {
                let mut q = c.prepare(&format!(
                    "SELECT DISTINCT path FROM files WHERE missing_since IS NULL AND role IN {TITLE_ROLES}"
                ))?;
                let v = q.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
                Ok(v)
            })
            .await
            .unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        let t0 = Instant::now();
        for d in dirs.iter().filter_map(|p| crate::storage::vfs::parent(p)) {
            if self.offline_path(&format!("{d}/x")).is_some() {
                continue;
            }
            if seen.insert(d.clone()) {
                let _ = self.folder_names(d).await;
            }
        }
        tracing::info!(folders = seen.len(), ms = t0.elapsed().as_millis() as u64, "folders listed");
    }

    /// A folder's file names (for sidecars), kept a while: listing a folder of the share takes
    /// seconds while it is busy. A stale listing is used and refreshed in the background.
    async fn folder_names(&self, dir: String) -> Arc<Vec<String>> {
        const FRESH: Duration = Duration::from_secs(120);
        const USABLE: Duration = Duration::from_hours(6);
        let list = |dir: String| -> Arc<Vec<String>> {
            Arc::new(
                crate::storage::vfs::read_dir_blocking(&dir)
                    .map(|l| l.into_iter().map(|e| e.name).collect())
                    .unwrap_or_default(),
            )
        };
        let cached = self.folders.lock().unwrap().get(&dir).cloned();
        match cached {
            Some((names, at)) if at.elapsed() < FRESH => names,
            Some((names, at)) if at.elapsed() < USABLE => {
                let folders = self.folders.clone();
                // Marked fresh now, so one refresh runs at a time.
                folders.lock().unwrap().insert(dir.clone(), (names.clone(), Instant::now()));
                tokio::task::spawn_blocking(move || {
                    let fresh = list(dir.clone());
                    folders.lock().unwrap().insert(dir, (fresh, Instant::now()));
                });
                names
            }
            _ => {
                let d = dir.clone();
                let names = tokio::task::spawn_blocking(move || list(d)).await.unwrap_or_default();
                let mut f = self.folders.lock().unwrap();
                f.retain(|_, (_, at)| at.elapsed() < USABLE);
                f.insert(dir, (names.clone(), Instant::now()));
                names
            }
        }
    }
}

pub fn ids_map(json: Option<&str>) -> HashMap<String, String> {
    json.and_then(|j| serde_json::from_str::<HashMap<String, Value>>(j).ok())
        .map(|m| m.into_iter().filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string()))).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(chrono_day(0), "1970-01-01");
        assert_eq!(chrono_day(1_793_577_600), "2026-11-02");
        assert_eq!(rfc3339(1_793_577_600 + 3661), "2026-11-02T01:01:01.000Z");
        assert_eq!(chrono_day(951_782_400), "2000-02-29");
    }

    #[tokio::test]
    async fn playback_skips_a_broken_version() {
        let db = Db::memory().unwrap();
        db.call(|c| {
            c.execute_batch(
                "INSERT INTO files(id, library_id, path, size, mtime, role, first_seen, last_seen, stable_since, item_id)
                   VALUES ('remux', 'm', '/m/remux.mkv', 30, 5, 'broken', 0, 0, 0, 't3'),
                          ('small', 'm', '/m/small.mkv', 5, 5, 'main', 0, 0, 0, 't3'),
                          ('mid', 'm', '/m/mid.mkv', 10, 5, 'main', 0, 0, 0, 't3'),
                          ('only', 'm', '/m/only.mkv', 10, 5, 'broken', 0, 0, 0, 'solo');",
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let pick = |item: &'static str, version: &'static str| {
            let db = db.clone();
            async move {
                db.call(move |c| Ok(c.query_row(&playback_file_sql(), rusqlite::params![item, version], |r| r.get::<_, String>(0))?))
                    .await
                    .unwrap()
            }
        };
        // No version named: the largest healthy file.
        assert_eq!(pick("t3", "").await, "mid");
        // A healthy version named: that one.
        assert_eq!(pick("t3", "small").await, "small");
        // The broken one named (an older app, a choice made before the damage was found): a healthy one.
        assert_eq!(pick("t3", "remux").await, "mid");
        // Nothing healthy: the broken file (its answer says why it can't play).
        assert_eq!(pick("solo", "").await, "only");
        assert_eq!(pick("solo", "only").await, "only");
    }
}
