//! Keyframe indexes made ahead of need, so a title's first play starts as fast as the next ones
//! (the `index_backfill` switch, on by default).
//!
//! A title is cut into segments at its keyframes (`media/mod.rs`). Matroska and MP4 keep an index
//! (a few reads), but AVI, MPEG-PS/TS and the like don't: their index is a packet scan of the
//! whole file (`scan.rs`), seconds for an episode and up to a minute for a film over the share,
//! and the title waits ("pending") until it is done. Indexes are kept in the database
//! (`files_probe.keyframes`) while the file is unchanged.
//!
//! This backfill makes them before anyone asks: for every library file the scanner has probed
//! and that has no index yet (new and changed files, and once, every file without one, such as
//! the AVIs whose index migration 013 dropped), but not files marked broken with a
//! `probe_error` (ffprobe can't open them, or a damaged Matroska file found here or on play)
//! until they change. Order: the titles being watched first (resume
//! points, the next episodes of shows in progress, and the episodes after one that starts
//! playing), then the newest files.
//!
//! It stays out of playback's way: one file at a time, a scan reads at `scan::BACKGROUND_RATE`
//! from a share, `nice`d, and stops (SIGSTOP) while anything is played or read for playback
//! (streams, transcodes, prepared downloads, a scan someone waits for), going on where it was
//! when that ends. A title played while its scan is running takes the scan over at full speed.
//! The database says what is done, so after a restart it goes on with the files left. Matroska
//! and MP4 files without a stored index (a few reads each) are read too, so their first play
//! needn't.

use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::Serialize;
use tokio::sync::Notify;

use crate::{
    admin::config::Feature,
    api::AppState,
    media::scan::{self, Stamp},
    media::{index_plan, IndexKind},
    storage::source::Source,
};

/// A file whose index couldn't be made is tried again after this long (or after a restart).
const RETRY_FAILED: Duration = Duration::from_hours(7 * 24);
/// Streaming counts as going on this long after the last stream request (as for subtitles).
const QUIET_AFTER_STREAMING: Duration = Duration::from_secs(90);

/// The backfill's progress (`GET /v1/admin/activity` → `index_backfill`).
#[derive(Clone, Debug, Default, Serialize)]
pub struct Status {
    /// "off" (switched off), "starting", "running", "paused" (something plays), "idle"
    /// (nothing to do).
    pub state: &'static str,
    /// Files this pass found without an index, and how many of them are done or failed.
    pub files: usize,
    pub done: usize,
    pub failed: usize,
    /// The file being read now (its name).
    pub current: Option<String>,
    pub pass_started: Option<u64>,
    /// Seconds spent reading this pass (not paused).
    pub secs: f64,
}

pub struct IndexBackfill {
    status: Mutex<Status>,
    /// Item ids to do before the rest (the next episodes of what starts playing), most wanted
    /// first.
    hints: Mutex<VecDeque<String>>,
    failed: Mutex<std::collections::HashMap<String, Instant>>,
    wake: Notify,
}

impl Default for IndexBackfill {
    fn default() -> Self {
        IndexBackfill {
            status: Mutex::new(Status {
                state: "starting",
                ..Default::default()
            }),
            hints: Mutex::new(VecDeque::new()),
            failed: Mutex::new(std::collections::HashMap::new()),
            wake: Notify::new(),
        }
    }
}

impl IndexBackfill {
    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    /// Item ids worth doing before the rest (the next episodes of what starts playing).
    pub fn hint(&self, next: Vec<String>) {
        if next.is_empty() {
            return;
        }
        let mut h = self.hints.lock().unwrap();
        for id in next.into_iter().rev() {
            h.retain(|x| *x != id);
            h.push_front(id);
        }
        h.truncate(20);
        drop(h);
        self.wake.notify_one();
    }

    fn set_state(&self, state: &'static str) {
        self.status.lock().unwrap().state = state;
    }
}

/// A file without an index.
#[derive(Clone, Debug)]
struct Todo {
    item: Option<String>,
    path: String,
    size: i64,
    mtime: i64,
    first_seen: i64,
    kind: IndexKind,
    stream: i64,
    timing: scan::Timing,
}

/// Something is played or read for playback now: background reads wait.
pub fn busy(st: &AppState) -> bool {
    st.subs.streaming_within(QUIET_AFTER_STREAMING)
        || crate::subtitles::subs::STREAM_READS.load(std::sync::atomic::Ordering::SeqCst) > 0
        || crate::subtitles::subs::WINDOW_READS.load(std::sync::atomic::Ordering::SeqCst) > 0
        || st.scanner.scanning_for_playback()
        || !st.transcoder.running().is_empty()
        || st.downloads.list().iter().any(|j| j.state == "running")
}

fn enabled(st: &AppState) -> bool {
    st.config.enabled(Feature::IndexBackfill) && st.config.enabled(Feature::ProbeCache)
}

/// The backfill, forever (started with the library).
pub async fn run(st: Arc<AppState>) {
    let (Some(library), Some(_)) = (st.library.clone(), st.db.clone()) else {
        return;
    };
    let ib = st.index_backfill.clone();
    // Let the first scan pass and the probes of new files settle.
    tokio::time::sleep(Duration::from_secs(30)).await;
    loop {
        if !enabled(&st) {
            ib.set_state("off");
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(5)) => {}
                () = ib.wake.notified() => {}
            }
            continue;
        }
        let todo = match list(&st).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("index backfill: {e:#}");
                Vec::new()
            }
        };
        let failed_recently = |path: &str| ib.failed.lock().unwrap().get(path).is_some_and(|at| at.elapsed() < RETRY_FAILED);
        let todo: Vec<Todo> = todo.into_iter().filter(|t| !failed_recently(&t.path)).collect();
        *ib.status.lock().unwrap() = Status {
            state: if todo.is_empty() { "idle" } else { "running" },
            files: todo.len(),
            pass_started: Some(crate::subtitles::subs::now_secs()),
            ..Default::default()
        };
        if !todo.is_empty() {
            let bytes: i64 = todo.iter().filter(|t| t.kind == IndexKind::Scan).map(|t| t.size).sum();
            tracing::info!(
                files = todo.len(),
                scans = todo.iter().filter(|t| t.kind == IndexKind::Scan).count(),
                gb = bytes as f64 / 1e9,
                "index backfill"
            );
        }
        let mut queue: VecDeque<Todo> = todo.into();
        let mut done_paths: HashSet<String> = HashSet::new();
        let t0 = Instant::now();
        let mut last_log = Instant::now();
        while enabled(&st) {
            // A hinted title (the next episodes of what plays) first, if it is in the list.
            let next = {
                let mut hinted = None;
                while let Some(id) = ib.hints.lock().unwrap().pop_front() {
                    if let Some(i) = queue.iter().position(|t| t.item.as_deref() == Some(id.as_str())) {
                        hinted = queue.remove(i);
                        break;
                    }
                }
                hinted.or_else(|| queue.pop_front())
            };
            let Some(t) = next else { break };
            if !done_paths.insert(t.path.clone()) {
                continue;
            }
            // Never start one while something plays.
            while busy(&st) && enabled(&st) {
                ib.set_state("paused");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            if !enabled(&st) {
                break;
            }
            {
                let mut s = ib.status.lock().unwrap();
                s.state = "running";
                s.current = Some(crate::storage::vfs::file_name(&t.path));
            }
            let started = Instant::now();
            let r = one(&st, &t).await;
            let failed = {
                let mut s = ib.status.lock().unwrap();
                s.current = None;
                s.secs = t0.elapsed().as_secs_f64();
                match &r {
                    Ok(true) => s.done += 1,
                    // Already there (played meanwhile), or given up because switched off.
                    Ok(false) if enabled(&st) => s.done += 1,
                    Ok(false) => {}
                    Err(_) => s.failed += 1,
                }
                r.err()
            };
            if let Some(e) = failed {
                tracing::warn!(path = %crate::storage::vfs::file_name(&t.path), "index backfill: {e:#}");
                ib.failed.lock().unwrap().insert(t.path.clone(), Instant::now());
                continue;
            }
            tracing::debug!(path = %crate::storage::vfs::file_name(&t.path), ms = started.elapsed().as_millis() as u64, "index made ahead of need");
            if last_log.elapsed() > Duration::from_secs(600) {
                last_log = Instant::now();
                let s = ib.status();
                tracing::info!(
                    files = s.files,
                    done = s.done,
                    failed = s.failed,
                    secs = s.secs as u64,
                    "index backfill progress"
                );
            }
            // A breath between files.
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let s = ib.status();
        if s.files > 0 {
            tracing::info!(
                files = s.files,
                done = s.done,
                failed = s.failed,
                secs = t0.elapsed().as_secs(),
                "index backfill pass finished"
            );
        }
        if !enabled(&st) {
            continue;
        }
        ib.set_state("idle");
        // New files are probed after a scan pass finds them: look again then.
        tokio::select! {
            () = library.probed.notified() => {}
            () = ib.wake.notified() => {}
            () = tokio::time::sleep(Duration::from_secs(3600)) => {}
        }
    }
}

/// Makes one file's index. Ok(false): nothing was needed or it was given up.
async fn one(st: &Arc<AppState>, t: &Todo) -> anyhow::Result<bool> {
    let db = st.db.clone().ok_or_else(|| anyhow::anyhow!("no database"))?;
    // Unchanged since the library saw it (a changed file is the scanner's to look at again).
    let now = crate::storage::vfs::stat(&t.path).await?;
    if (now.size as i64, now.mtime) != (t.size, t.mtime) {
        return Ok(false);
    }
    // Played (and indexed) meanwhile?
    if db
        .probe_get(t.path.clone(), t.size as u64, t.mtime)
        .await?
        .is_some_and(|p| p.keyframes.is_some())
    {
        return Ok(false);
    }
    let source = Source::of(&t.path);
    let stamp = Stamp {
        path: t.path.clone(),
        size: t.size as u64,
        mtime: t.mtime,
    };
    let read = match t.kind {
        IndexKind::Mkv => match crate::media::mkv::keyframes(&source, &st.http).await {
            Ok(kf) => Some(Ok(kf)),
            // Damaged (zeros, cut short): marked `broken` until the file changes, and `list`
            // passes over it from now on. Not scanned: a packet scan of a Matroska file doesn't
            // make it playable (every copy segment would read the file from its start).
            Err(e) if crate::media::mkv::failure(&e) == crate::media::mkv::Failure::Damaged => {
                crate::library::mark_damaged(&db, stamp.path, stamp.size, stamp.mtime, format!("damaged or incomplete: {e:#}")).await?;
                return Err(e.context("damaged: marked broken"));
            }
            // A read that failed is tried again later (`RETRY_FAILED`, or after a restart).
            Err(e) => Some(Err(e)),
        },
        IndexKind::Mp4 => match crate::media::moov::keyframes(&source, &st.http).await {
            Ok(kf) => Some(Ok(kf)),
            // As `Media::load`: scanned instead.
            Err(_) => None,
        },
        IndexKind::Scan => None,
    };
    if let Some(r) = read {
        db.keyframes_put(stamp.path, stamp.size, stamp.mtime, r?).await?;
        return Ok(true);
    }
    if !st.config.enabled(Feature::ScanIndex) {
        return Ok(false);
    }
    let st2 = st.clone();
    let busy: scan::Busy = Arc::new(move || {
        let b = busy(&st2);
        st2.index_backfill.set_state(if b { "paused" } else { "running" });
        b
    });
    let st3 = st.clone();
    let done = st
        .scanner
        .background(&source, t.stream, t.timing, stamp, busy, move || enabled(&st3))
        .await?;
    Ok(done.is_some())
}

/// The files without an index, in the order to make them: titles being watched first, then
/// the newest.
async fn list(st: &AppState) -> anyhow::Result<Vec<Todo>> {
    let db = st.db.clone().ok_or_else(|| anyhow::anyhow!("no database"))?;
    let config = st.config.clone();
    db.call(move |c| {
        let rows = without_index(c)?;
        let mut todo: Vec<Todo> = rows
            .into_iter()
            .filter_map(|(item, path, size, mtime, first_seen, probe)| {
                let probe: serde_json::Value = serde_json::from_str(&probe).ok()?;
                let (kind, stream, timing) = index_plan(&probe, &config)?;
                Some(Todo {
                    item,
                    path,
                    size,
                    mtime,
                    first_seen,
                    kind,
                    stream,
                    timing,
                })
            })
            .collect();
        if todo.is_empty() {
            return Ok(todo);
        }
        let wanted = watching(c)?;
        let rank = |t: &Todo| {
            t.item
                .as_ref()
                .and_then(|i| wanted.iter().position(|w| w == i))
                .unwrap_or(usize::MAX)
        };
        // Stable: within the same rank, newest first (the query's order).
        todo.sort_by_key(|t| (rank(t), -t.first_seen));
        Ok(todo)
    })
    .await
}

/// What is being watched, most wanted first: every account's resume points and next episodes,
/// with the two episodes after each (item ids). `trickplay.rs` goes in this order too.
pub(crate) fn watching(c: &rusqlite::Connection) -> anyhow::Result<Vec<String>> {
    let users: Vec<String> = c
        .prepare("SELECT id FROM users")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut wanted: Vec<String> = Vec::new();
    let mut after = c.prepare(
        "SELECT n.id FROM items e JOIN items n ON n.series_id = e.series_id AND n.kind = 'episode'
           AND (COALESCE(n.season_no, 1) > COALESCE(e.season_no, 1) OR (COALESCE(n.season_no, 1) = COALESCE(e.season_no, 1) AND COALESCE(n.episode_no, 0) > COALESCE(e.episode_no, 0)))
         WHERE e.id = ?1 AND e.kind = 'episode' AND COALESCE(n.season_no, 1) > 0
         ORDER BY COALESCE(n.season_no, 1), n.episode_no LIMIT 2",
    )?;
    for u in &users {
        let mut ids = crate::userdata::resume(c, u, 20, None)?;
        ids.extend(crate::userdata::next_up(c, u, 30, None, true)?);
        for id in ids {
            let next: Vec<String> = after.query_map([&id], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
            for x in std::iter::once(id).chain(next) {
                if !wanted.contains(&x) {
                    wanted.push(x);
                }
            }
        }
    }
    Ok(wanted)
}

/// Probed title files without an index (item, path, size, mtime, first_seen, probe), newest
/// first; not those marked broken with a `probe_error` (`library::mark_damaged`).
fn without_index(c: &rusqlite::Connection) -> rusqlite::Result<Vec<(Option<String>, String, i64, i64, i64, String)>> {
    let mut q = c.prepare(&format!(
        "SELECT f.item_id, f.path, f.size, f.mtime, f.first_seen, fp.probe FROM files f
         JOIN files_probe fp ON fp.path = f.path AND fp.size = f.size AND fp.mtime = f.mtime
         WHERE f.missing_since IS NULL AND f.role IN {} AND f.probe_error IS NULL AND fp.keyframes IS NULL
         ORDER BY f.first_seen DESC, f.path",
        crate::library::TITLE_ROLES
    ))?;
    let rows = q
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn probe(format: &str, codec: &str, b_frames: i64) -> serde_json::Value {
        json!({"format": {"format_name": format}, "streams": [
            {"index": 0, "codec_type": "video", "codec_name": "mjpeg", "disposition": {"attached_pic": 1}},
            {"index": 1, "codec_type": "video", "codec_name": codec, "has_b_frames": b_frames, "disposition": {"attached_pic": 0}},
            {"index": 2, "codec_type": "audio", "codec_name": "mp3"}]})
    }

    /// A damaged file is marked broken and passed over (the backfill and the scanner's probes)
    /// for as long as it is unchanged; a re-download (new size, mtime) is probed again, made
    /// `main` by a good probe, and indexed.
    #[tokio::test]
    async fn damaged_files_wait_until_they_change() {
        /// (What the backfill would index, what the scanner would probe, the damaged file's role.)
        async fn todo(db: &crate::db::Db) -> anyhow::Result<(Vec<String>, Vec<String>, String)> {
            db.call(|c| {
                let backfill: Vec<String> = without_index(c)?.into_iter().map(|r| r.1).collect();
                let probe: Vec<String> = c
                    .prepare(crate::library::scanner::PROBE_TODO)?
                    .query_map(["p"], |r| r.get(1))?
                    .collect::<Result<_, _>>()?;
                let role: String = c.query_row("SELECT role FROM files WHERE id = 'bad'", [], |r| r.get(0))?;
                Ok((backfill, probe, role))
            })
            .await
        }
        let db = crate::db::Db::memory().unwrap();
        db.call(|c| {
            c.execute_batch(
                "INSERT INTO files(id, library_id, path, size, mtime, role, first_seen, last_seen, stable_since, probed_stamp, container)
                   VALUES ('bad', 'm', '/m/bad.mkv', 10, 5, 'main', 2, 0, 0, '10|5', 'matroska,webm'),
                          ('ok', 'm', '/m/ok.mkv', 20, 5, 'main', 1, 0, 0, '20|5', 'matroska,webm');
                 INSERT INTO files_probe(path, size, mtime, probe, probed_at, prober) VALUES ('/m/bad.mkv', 10, 5, '{}', 0, 'p'), ('/m/ok.mkv', 20, 5, '{}', 0, 'p');",
            )?;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(
            todo(&db).await.unwrap(),
            (vec!["/m/bad.mkv".to_string(), "/m/ok.mkv".to_string()], vec![], "main".to_string())
        );
        // Another size or mtime than the library has: not marked (the file changed meanwhile).
        assert!(!crate::library::mark_damaged(&db, "/m/bad.mkv".into(), 11, 5, "x".into())
            .await
            .unwrap());
        assert!(
            crate::library::mark_damaged(&db, "/m/bad.mkv".into(), 10, 5, "damaged or incomplete: bad element id".into())
                .await
                .unwrap()
        );
        // Neither indexed nor probed again (after a restart too: it's in the database).
        assert_eq!(
            todo(&db).await.unwrap(),
            (vec!["/m/ok.mkv".to_string()], vec![], "broken".to_string())
        );
        // Re-downloaded: the scan pass sees a new size; the scanner probes it again.
        db.call(|c| Ok(c.execute("UPDATE files SET size = 12, mtime = 9 WHERE id = 'bad'", [])?))
            .await
            .unwrap();
        assert_eq!(todo(&db).await.unwrap().1, vec!["/m/bad.mkv".to_string()]);
        // A good probe (as `probe_file` writes it): `main` again, no error, and indexed.
        db.call(|c| {
            c.execute("UPDATE files SET probed_stamp = '12|9', probe_error = NULL, role = CASE WHEN role = 'broken' THEN 'main' ELSE role END WHERE id = 'bad'", [])?;
            c.execute("UPDATE files_probe SET size = 12, mtime = 9, keyframes = NULL WHERE path = '/m/bad.mkv'", [])?;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(
            todo(&db).await.unwrap(),
            (vec!["/m/bad.mkv".to_string(), "/m/ok.mkv".to_string()], vec![], "main".to_string())
        );
    }

    #[test]
    fn plans_follow_the_container() {
        let c = crate::admin::config::Config::from_lookup(|_| None);
        assert_eq!(
            index_plan(&probe("avi", "mpeg4", 1), &c),
            Some((IndexKind::Scan, 1, scan::Timing::AviDelayed))
        );
        assert_eq!(
            index_plan(&probe("avi", "mpeg4", 0), &c),
            Some((IndexKind::Scan, 1, scan::Timing::Avi))
        );
        assert_eq!(
            index_plan(&probe("avi", "h264", 2), &c),
            Some((IndexKind::Scan, 1, scan::Timing::Avi))
        );
        assert_eq!(
            index_plan(&probe("mpegts", "mpeg2video", 1), &c),
            Some((IndexKind::Scan, 1, scan::Timing::Pts))
        );
        assert_eq!(
            index_plan(&probe("matroska,webm", "hevc", 2), &c),
            Some((IndexKind::Mkv, 1, scan::Timing::Pts))
        );
        assert_eq!(
            index_plan(&probe("mov,mp4,m4a,3gp,3g2,mj2", "h264", 2), &c),
            Some((IndexKind::Mp4, 1, scan::Timing::Pts))
        );
        // No MP4 index: scanned, as playback would.
        c.disabled.set(["mp4_index".to_string()].into());
        assert_eq!(
            index_plan(&probe("mov,mp4,m4a,3gp,3g2,mj2", "h264", 2), &c),
            Some((IndexKind::Scan, 1, scan::Timing::Pts))
        );
        // Audio only.
        assert_eq!(
            index_plan(
                &json!({"format": {"format_name": "avi"}, "streams": [{"index": 0, "codec_type": "audio"}]}),
                &c
            ),
            None
        );
    }
}
