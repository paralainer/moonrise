//! Episodes numbered other than TMDB numbers them, within one season of a show: double
//! episodes split over two files (`The One in Vegas (1)` / `(2)`, `My Finale Part 1/2`, a 90-minute
//! premiere as two 45-minute files), hour-long episodes counted twice by other guides (The Office's
//! `S04E01-E02 - Fun Run` is TMDB's S4E1, and every file after it is numbered ahead of TMDB), and
//! files numbered past TMDB's season end whose titles are TMDB episodes.
//!
//! A season's episodes (one entry per item: versions of an episode are one) are aligned with TMDB's
//! episode list by their titles, numbering, runtimes and part markers (`align`): each entry gets one
//! TMDB episode (or a run of up to three: one file with two episodes), several entries in a row may
//! share one (the parts of a split episode), or none (left as they are). The plan replaces the plain
//! numbering only when it explains the season clearly better; then the items are re-keyed in place
//! (`align_parts`), so each keeps its id and watch state: part 1 keeps the episode's key, part N gets
//! `<key>:p<N>` and `part_no` N. They share the episode's TMDB details and artwork (metadata goes by
//! season and episode number). Seasons with a Fix Match choice are left alone.

use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use super::{
    util::{now, similarity, sort_title},
    Library, TITLE_ROLES,
};

/// How much better than the plain numbering a plan must explain a season to be used.
const MARGIN: f64 = 2.0;

/// One episode item of a season, as its file names it.
#[derive(Debug, Clone, Default)]
pub struct Entry {
    pub item_id: String,
    pub key: String,
    pub cur_episode: Option<i32>,
    pub cur_end: Option<i32>,
    pub cur_part: Option<i32>,
    pub state: String,
    /// The numbers in the file's name.
    pub e: i32,
    pub e_end: Option<i32>,
    /// The episode title in the name, without a part marker.
    pub title: String,
    /// `(2)`, `Part 2`, `Pt.2` at the end of the title.
    pub marker: Option<i32>,
    /// Minutes.
    pub minutes: Option<f64>,
}

impl Entry {
    fn last(&self) -> i32 {
        self.e_end.unwrap_or(self.e)
    }
}

/// One of TMDB's episodes of the season.
#[derive(Debug, Clone)]
pub struct TmdbEp {
    pub num: i32,
    pub name: String,
    /// Minutes.
    pub runtime: Option<f64>,
}

/// The TMDB episodes (indices into the season's list, first..=last) an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Slot {
    pub a: usize,
    pub b: usize,
}

static PART_END: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[\s,.:-]*(?:\((\d)\)|\(?(?:part|pt\.?)\s*(\d|one|two|three|i{1,3})\)?)\s*$").unwrap());

/// A title without its part marker, and the marker's number.
pub fn split_part(title: &str) -> (String, Option<i32>) {
    let t = title.trim();
    if let Some(c) = PART_END.captures(t) {
        let n = c
            .get(1)
            .or(c.get(2))
            .map(|m| m.as_str().to_lowercase())
            .and_then(|s| match s.as_str() {
                "one" | "i" => Some(1),
                "two" | "ii" => Some(2),
                "three" | "iii" => Some(3),
                d => d.parse().ok(),
            });
        let rest = t[..c.get(0).unwrap().start()].trim().to_string();
        if n.is_some() && !rest.is_empty() {
            return (rest, n);
        }
    }
    (t.to_string(), None)
}

fn title_score(e: &Entry, eps: &[TmdbEp], s: Slot) -> f64 {
    if e.title.is_empty() {
        return 0.0;
    }
    let joined = eps[s.a..=s.b].iter().map(|x| split_part(&x.name).0).collect::<Vec<_>>().join(" & ");
    ((similarity(&e.title, &joined) - 0.55) * 4.0).clamp(-2.0, 2.0)
}

fn span_runtime(eps: &[TmdbEp], s: Slot) -> Option<f64> {
    eps[s.a..=s.b].iter().map(|x| x.runtime.filter(|r| *r > 0.0)).sum()
}

/// A whole file against the TMDB runtime of what it is.
fn runtime_single(e: &Entry, eps: &[TmdbEp], s: Slot) -> f64 {
    let (Some(d), Some(r)) = (e.minutes, span_runtime(eps, s)) else {
        return 0.0;
    };
    let x = d / r;
    if (0.75..=1.35).contains(&x) {
        0.5
    } else if x < 0.65 {
        -1.5
    } else if x > 1.6 {
        -1.0
    } else {
        0.0
    }
}

/// A season's entries and TMDB episodes, with each entry's title and runtime scores per slot.
struct Ctx<'a> {
    entries: &'a [Entry],
    eps: &'a [TmdbEp],
    /// [entry][a][b - a]
    title: Vec<Vec<[f64; 3]>>,
    runtime: Vec<Vec<[f64; 3]>>,
}

impl<'a> Ctx<'a> {
    fn new(entries: &'a [Entry], eps: &'a [TmdbEp]) -> Self {
        let per = |f: &dyn Fn(&Entry, Slot) -> f64| -> Vec<Vec<[f64; 3]>> {
            entries
                .iter()
                .map(|e| {
                    (0..eps.len())
                        .map(|a| {
                            let mut v = [0.0; 3];
                            for (d, x) in v.iter_mut().enumerate() {
                                if a + d < eps.len() {
                                    *x = f(e, Slot { a, b: a + d });
                                }
                            }
                            v
                        })
                        .collect()
                })
                .collect()
        };
        let title = per(&|e, s| title_score(e, eps, s));
        let runtime = per(&|e, s| runtime_single(e, eps, s));
        Ctx {
            entries,
            eps,
            title,
            runtime,
        }
    }
}

/// What came before an entry in a plan: the last entry with a slot, and how many entries share
/// that slot so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Last {
    idx: usize,
    slot: Slot,
    chain: u8,
}

/// The score of giving entry `i` the slot `s` (None: no slot) after `last`.
fn step(cx: &Ctx, last: Option<Last>, i: usize, s: Option<Slot>) -> Option<(f64, Option<Last>)> {
    let (entries, eps) = (cx.entries, cx.eps);
    let e = &entries[i];
    let Some(s) = s else { return Some((-1.0, last)) };
    let part = last.is_some_and(|l| l.slot.b == s.a);
    if let Some(l) = last {
        if s.a < l.slot.b || (part && (s.a != s.b || l.slot.a != l.slot.b || l.chain >= 3)) {
            return None;
        }
    }
    if s.b >= eps.len() || s.b < s.a || s.b - s.a > 2 {
        return None;
    }
    let mut score = cx.title[i][s.a][s.b - s.a];
    let pred = match last {
        None => e.e,
        Some(l) => eps[l.slot.b].num + (e.e - entries[l.idx].last()),
    };
    score += if eps[s.a].num == pred { 1.0 } else { -1.0 };
    if part {
        let l = last.unwrap();
        let p = &entries[l.idx];
        score -= 1.5;
        if let (Some(mp), Some(mi)) = (p.marker, e.marker) {
            if mi == mp + 1 {
                score += 2.0;
            }
        }
        let fits = match (p.minutes, e.minutes, span_runtime(eps, s)) {
            (Some(dp), Some(di), Some(r)) => {
                let (xp, xi, sum) = (dp / r, di / r, (dp + di) / r);
                (0.8..=1.25).contains(&sum) && (0.3..=0.7).contains(&xp) && (0.3..=0.7).contains(&xi)
            }
            _ => false,
        };
        if fits && l.chain == 1 {
            // Two halves of it: the first one isn't a short episode after all.
            score += 2.0 - cx.runtime[l.idx][l.slot.a][l.slot.b - l.slot.a];
        }
    } else {
        score += cx.runtime[i][s.a][s.b - s.a];
    }
    let chain = if part { last.unwrap().chain + 1 } else { 1 };
    Some((score, Some(Last { idx: i, slot: s, chain })))
}

/// The score of a whole plan (None: not a valid order).
fn score_in(cx: &Ctx, plan: &[Option<Slot>]) -> Option<f64> {
    let mut last = None;
    let mut total = 0.0;
    for (i, s) in plan.iter().enumerate() {
        let (x, l) = step(cx, last, i, *s)?;
        total += x;
        last = l;
    }
    Some(total)
}

/// The plain numbering: each entry the TMDB episode(s) its name numbers (None where TMDB has none).
pub fn by_number(entries: &[Entry], eps: &[TmdbEp]) -> Vec<Option<Slot>> {
    let at = |n: i32| eps.iter().position(|x| x.num == n);
    entries
        .iter()
        .map(|e| {
            let a = at(e.e)?;
            let b = e.e_end.and_then(at).filter(|b| *b >= a && *b - a <= 2).unwrap_or(a);
            Some(Slot { a, b })
        })
        .collect()
}

/// The best plan for a season (entries in their names' order), if it explains the season clearly
/// better than the plain numbering. Returns (plan, its score, the numbering's score).
pub fn align(entries: &[Entry], eps: &[TmdbEp]) -> Option<(Vec<Option<Slot>>, f64, f64)> {
    type Key = Option<Last>;
    if entries.is_empty() || eps.is_empty() {
        return None;
    }
    let cx = Ctx::new(entries, eps);
    // Per step: state → (score, previous state, this entry's slot).
    let mut layers: Vec<HashMap<Key, (f64, Key, Option<Slot>)>> = Vec::new();
    let mut cur: HashMap<Key, f64> = HashMap::from([(None, 0.0)]);
    for i in 0..entries.len() {
        let mut next: HashMap<Key, (f64, Key, Option<Slot>)> = HashMap::new();
        for (&k, &v) in &cur {
            let start = k.map_or(0, |l| l.slot.b);
            // After the first slot, at most a few of TMDB's episodes are skipped (missing files).
            let stop = if k.is_some() { eps.len().min(start + 8) } else { eps.len() };
            let mut options = vec![None];
            for a in start..stop {
                for b in a..eps.len().min(a + 3) {
                    options.push(Some(Slot { a, b }));
                }
            }
            for s in options {
                if let Some((x, nk)) = step(&cx, k, i, s) {
                    let total = v + x;
                    if next.get(&nk).is_none_or(|(best, _, _)| total > *best + 1e-9) {
                        next.insert(nk, (total, k, s));
                    }
                }
            }
        }
        // Only the best states go on (a season has a few dozen episodes at most).
        let mut ranked: Vec<(Key, f64)> = next.iter().map(|(k, (v, _, _))| (*k, *v)).collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        ranked.truncate(600);
        cur = ranked.into_iter().collect();
        layers.push(next);
    }
    let (mut k, best) = cur
        .iter()
        .max_by(|a, b| {
            a.1.partial_cmp(b.1)
                .unwrap()
                .then_with(|| format!("{:?}", b.0).cmp(&format!("{:?}", a.0)))
        })
        .map(|(k, v)| (*k, *v))?;
    let mut plan = vec![None; entries.len()];
    for i in (0..entries.len()).rev() {
        let (_, prev, s) = layers[i][&k];
        plan[i] = s;
        k = prev;
    }
    let base = score_in(&cx, &by_number(entries, eps)).unwrap_or(f64::NEG_INFINITY);
    (best >= base + MARGIN).then_some((plan, best, base))
}

/// What a plan makes of each entry: (episode, episode_end, part).
pub fn targets(eps: &[TmdbEp], plan: &[Option<Slot>]) -> Vec<Option<(i32, Option<i32>, Option<i32>)>> {
    let mut out = Vec::with_capacity(plan.len());
    for (i, s) in plan.iter().enumerate() {
        let Some(s) = s else {
            out.push(None);
            continue;
        };
        let shares = |j: usize| plan.get(j).copied().flatten().is_some_and(|o: Slot| o == *s);
        // Parts: a run of entries with the same single slot.
        let mut first = i;
        while first > 0 && shares(first - 1) {
            first -= 1;
        }
        let mut end = i;
        while shares(end + 1) {
            end += 1;
        }
        let part = (end > first).then_some((i - first + 1) as i32);
        out.push(Some((eps[s.a].num, (s.b > s.a).then_some(eps[s.b].num), part)));
    }
    out
}

/// One re-keyed item, for the log and `align-eval`.
#[derive(Debug, Clone)]
struct Change {
    item_id: String,
    from_key: String,
    to_key: String,
    episode: i32,
    end: Option<i32>,
    part: Option<i32>,
    title: String,
    note: String,
    was_review: bool,
}

// (series id, series key, season, item id, key, ep, end, part, state, parsed json, minutes, manual)
type Row = (
    String,
    String,
    i64,
    String,
    String,
    Option<i32>,
    Option<i32>,
    Option<i32>,
    String,
    Option<String>,
    Option<f64>,
    bool,
);

/// One entry per item of a season, as its file's name numbers it in this season; None when one
/// isn't numbered so (or has a date), or there are none: the season isn't aligned.
fn entries_of(rows: &[Row], season: i64) -> Option<Vec<Entry>> {
    let mut entries = Vec::new();
    for r in rows {
        let p: Value = r.9.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null);
        let (ps, pe) = (p["season"].as_i64(), p["episode"].as_i64());
        if ps != Some(season) || pe.is_none() || p["air_date"].is_string() {
            return None;
        }
        let raw = p["title"].as_str().unwrap_or("");
        let (title, marker) = split_part(raw);
        let marker = marker.or(p["part"].as_i64().map(|x| x as i32));
        entries.push(Entry {
            item_id: r.3.clone(),
            key: r.4.clone(),
            cur_episode: r.5,
            cur_end: r.6,
            cur_part: r.7,
            state: r.8.clone(),
            e: pe.unwrap() as i32,
            e_end: p["episode_end"].as_i64().map(|x| x as i32),
            title,
            marker,
            minutes: r.10,
        });
    }
    if entries.is_empty() {
        return None;
    }
    Some(entries)
}

/// TMDB's list of the season's episodes, by number.
async fn season_episodes(tmdb: &super::tmdb::Tmdb, tid: i64, season: i64) -> Vec<TmdbEp> {
    let sv = tmdb.season(tid, season, 7 * super::tmdb::DAY).await.unwrap_or(Value::Null);
    let mut eps: Vec<TmdbEp> = sv["episodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            Some(TmdbEp {
                num: e["episode_number"].as_i64()? as i32,
                name: e["name"].as_str().unwrap_or("").to_string(),
                runtime: e["runtime"].as_f64().filter(|r| *r > 0.0),
            })
        })
        .collect();
    eps.sort_by_key(|e| e.num);
    eps
}

/// The items the plan moves: from their key to the one it gives them.
fn changes_of(
    entries: &[Entry],
    tg: &[Option<(i32, Option<i32>, Option<i32>)>],
    eps: &[TmdbEp],
    series_key: &str,
    season: i64,
) -> Vec<Change> {
    let mut changes = Vec::new();
    for (e, t) in entries.iter().zip(tg) {
        let Some((ep, end, part)) = *t else { continue };
        let base_key = format!("{series_key}:s{season}e{ep}");
        let to_key = match part {
            Some(p) if p >= 2 => format!("{base_key}:p{p}"),
            _ => base_key,
        };
        if to_key == e.key && Some(ep) == e.cur_episode && end == e.cur_end && part == e.cur_part {
            continue;
        }
        let name = eps.iter().find(|x| x.num == ep).map(|x| split_part(&x.name).0).unwrap_or_default();
        let named = match e.e_end {
            Some(x) => format!("S{season:02}E{:02}-E{x:02}", e.e),
            None => format!("S{season:02}E{:02}", e.e),
        };
        let what = match (part, end) {
            (Some(p), _) => format!("Part {p} of S{season}E{ep}"),
            (None, Some(x)) => format!("S{season}E{ep}-E{x}"),
            (None, None) => format!("S{season}E{ep}"),
        };
        changes.push(Change {
            item_id: e.item_id.clone(),
            from_key: e.key.clone(),
            to_key,
            episode: ep,
            end,
            part,
            title: name,
            note: format!("{what} by TMDB's episode list (its name numbers it {named})"),
            was_review: e.state == "review",
        });
    }
    changes
}

/// A season's report: the plan for every item, and the changes.
fn season_summary(
    series_key: &str,
    season: i64,
    best: f64,
    base: f64,
    entries: &[Entry],
    tg: &[Option<(i32, Option<i32>, Option<i32>)>],
    changes: &[Change],
) -> Value {
    json!({
        "series": series_key, "season": season, "score": (best * 10.0).round() / 10.0, "numbering_score": (base * 10.0).round() / 10.0,
        "plan": entries.iter().zip(tg).map(|(e, t)| json!({
            "name": match e.e_end { Some(x) => format!("E{}-E{x}", e.e), None => format!("E{}", e.e) },
            "title": e.title, "marker": e.marker, "minutes": e.minutes.map(f64::round),
            "to": t.map(|(ep, end, part)| json!({"episode": ep, "end": end, "part": part})),
        })).collect::<Vec<_>>(),
        "changes": changes.iter().map(|c| json!({"item": c.item_id, "from": c.from_key, "to": c.to_key, "title": c.title, "was_review": c.was_review})).collect::<Vec<_>>(),
    })
}

impl Library {
    /// Aligns the seasons of the given shows (all, if None) with TMDB's episode lists and re-keys
    /// what that changes (`apply`; else only reports). Answers a report: per changed season the
    /// items moved, and the counts.
    pub async fn align_parts(&self, series: Option<Vec<String>>, apply: bool) -> Result<Value> {
        let Some(tmdb) = self.tmdb() else {
            return Ok(json!({"skipped": "no TMDB"}));
        };
        let rows = self.episode_rows().await?;
        let wanted: Option<BTreeSet<String>> = series.map(|v| v.into_iter().collect());
        let mut seasons: HashMap<(String, String, i64), Vec<Row>> = HashMap::new();
        for r in rows {
            if wanted.as_ref().is_some_and(|w| !w.contains(&r.0)) {
                continue;
            }
            seasons.entry((r.0.clone(), r.1.clone(), r.2)).or_default().push(r);
        }
        let mut keys: Vec<_> = seasons.keys().cloned().collect();
        keys.sort();
        let mut report = Vec::new();
        let (mut changed_items, mut resolved_review, mut conflicts) = (0, 0, 0);
        for k in keys {
            let rows = &seasons[&k];
            let (series_id, series_key, season) = k.clone();
            if season == 0 || rows.iter().any(|r| r.11) {
                continue;
            }
            let Some(tid) = series_key.strip_prefix("tmdb:tv:").and_then(|t| t.parse::<i64>().ok()) else {
                continue;
            };
            // One entry per item, as its file's name numbers it in this season (else not aligned).
            let Some(mut entries) = entries_of(rows, season) else {
                continue;
            };
            entries.sort_by(|a, b| (a.e, a.marker, &a.key).cmp(&(b.e, b.marker, &b.key)));
            // Two items for one number (a part already split off, or a mismatch): the plain
            // numbering would give them one slot; fine, the plan decides.
            let eps = season_episodes(&tmdb, tid, season).await;
            let Some((plan, best, base)) = align(&entries, &eps) else {
                continue;
            };
            let tg = targets(&eps, &plan);
            let changes = changes_of(&entries, &tg, &eps, &series_key, season);
            if changes.is_empty() {
                continue;
            }
            let summary = season_summary(&series_key, season, best, base, &entries, &tg, &changes);
            if apply {
                let ch = changes.clone();
                let sid = series_id.clone();
                let ok = self.db.call(move |c| rekey(c, &sid, &ch)).await?;
                if !ok {
                    conflicts += 1;
                    tracing::warn!(
                        series = series_key,
                        season,
                        "parts: another item holds a key the plan needs; season left as it is"
                    );
                    report.push(json!({"conflict": true, "detail": summary}));
                    continue;
                }
                tracing::info!(
                    series = series_key,
                    season,
                    items = changes.len(),
                    "parts: season aligned with TMDB's episode list"
                );
            }
            changed_items += changes.len();
            resolved_review += changes.iter().filter(|c| c.was_review).count();
            report.push(summary);
        }
        Ok(
            json!({"applied": apply, "seasons": report.len(), "items": changed_items, "review_resolved": resolved_review, "conflicts": conflicts, "changes": report}),
        )
    }

    /// The episodes of TMDB shows with a file, with what aligning needs of them.
    async fn episode_rows(&self) -> Result<Vec<Row>> {
        let rows: Vec<Row> = self
            .db
            .call(move |c| {
                let mut st = c.prepare(&format!(
                    "SELECT s.id, s.key, i.season_no, i.id, i.key, i.episode_no, i.episode_end, i.part_no, i.identify_state,
                            (SELECT f.parsed FROM files f WHERE f.item_id = i.id AND f.role IN {TITLE_ROLES} AND f.missing_since IS NULL ORDER BY f.size DESC LIMIT 1),
                            (SELECT MAX(f.duration) FROM files f WHERE f.item_id = i.id AND f.role IN {TITLE_ROLES} AND f.missing_since IS NULL) / 60.0,
                            EXISTS (SELECT 1 FROM files f WHERE f.item_id = i.id AND f.manual_key IS NOT NULL) OR i.identify_state = 'manual'
                     FROM items i JOIN items s ON s.id = i.series_id
                     WHERE i.kind = 'episode' AND s.key LIKE 'tmdb:tv:%' AND i.season_no IS NOT NULL
                       AND EXISTS (SELECT 1 FROM files f WHERE f.item_id = i.id AND f.role IN {TITLE_ROLES} AND f.missing_since IS NULL)"
                ))?;
                let rows = st
                    .query_map([], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?))
                    })?
                    .collect::<Result<Vec<Row>, _>>()?;
                Ok(rows)
            })
            .await?;
        Ok(rows)
    }

    /// Once (meta `parts_aligned`): every season; then the shows whose files were just identified.
    pub async fn align_after_identify(&self, since: i64) -> Result<()> {
        if self.tmdb().is_none() {
            return Ok(());
        }
        let done: bool = self
            .db
            .call(|c| {
                Ok(
                    c.query_row("SELECT value FROM meta WHERE key = 'parts_aligned'", [], |r| r.get::<_, String>(0))
                        .optional()?
                        .is_some(),
                )
            })
            .await?;
        let series = if done {
            let s: Vec<String> = self
                .db
                .call(move |c| {
                    let mut st = c.prepare(
                        "SELECT DISTINCT i.series_id FROM files f JOIN items i ON i.id = f.item_id WHERE f.identified_at >= ?1 AND i.kind = 'episode' AND i.series_id IS NOT NULL",
                    )?;
                    let rows = st.query_map([since], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
                    Ok(rows)
                })
                .await?;
            if s.is_empty() {
                return Ok(());
            }
            Some(s)
        } else {
            None
        };
        let r = self.align_parts(series, true).await?;
        if !done {
            tracing::info!(
                seasons = r["seasons"].as_u64(),
                items = r["items"].as_u64(),
                review_resolved = r["review_resolved"].as_u64(),
                "parts: first alignment of all seasons"
            );
            self.db
                .call(|c| {
                    Ok(c.execute(
                        "INSERT OR REPLACE INTO meta(key, value) VALUES ('parts_aligned', ?1)",
                        [now().to_string()],
                    )?)
                })
                .await?;
        }
        Ok(())
    }
}

/// Re-keys a season's items in one transaction (ids and watch state stay). False (nothing
/// changed) when a target key belongs to an item outside the plan.
fn rekey(c: &mut rusqlite::Connection, series_id: &str, changes: &[Change]) -> Result<bool> {
    let tx = c.transaction()?;
    for ch in changes {
        tx.execute("UPDATE items SET key = 'tmp:' || id WHERE id = ?1", [&ch.item_id])?;
    }
    for ch in changes {
        let taken: Option<String> = tx
            .query_row("SELECT id FROM items WHERE key = ?1", [&ch.to_key], |r| r.get(0))
            .optional()?;
        if taken.is_some() {
            return Ok(false); // dropped: rolls back
        }
        tx.execute(
            "UPDATE items SET key = ?2, episode_no = ?3, episode_end = ?4, part_no = ?5,
                    identify_state = CASE identify_state WHEN 'review' THEN 'matched' ELSE identify_state END,
                    identify_source = 'aligned', identify_note = ?6, title = CASE WHEN ?7 = '' THEN title ELSE ?7 END,
                    sort_title = CASE WHEN ?7 = '' THEN sort_title ELSE ?8 END, provider_ids = NULL, refreshed_at = NULL, updated_at = ?9,
                    runtime = CASE WHEN ?5 IS NULL THEN NULL
                                   ELSE (SELECT MAX(f.duration) FROM files f WHERE f.item_id = items.id AND f.role IN ('main', 'broken')) END
             WHERE id = ?1",
            params![ch.item_id, ch.to_key, ch.episode, ch.end, ch.part, ch.note, ch.title, sort_title(&ch.title), now()],
        )?;
        // It is a title of its own again (an old merge may have pointed it elsewhere).
        tx.execute("DELETE FROM item_redirects WHERE old_id = ?1", [&ch.item_id])?;
    }
    // The show's metadata again (episode details and artwork for the new numbers).
    tx.execute("UPDATE items SET refreshed_at = NULL WHERE id = ?1", [series_id])?;
    tx.commit()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(num: i32, name: &str, runtime: f64) -> TmdbEp {
        TmdbEp {
            num,
            name: name.into(),
            runtime: Some(runtime),
        }
    }

    fn entry(e: i32, e_end: Option<i32>, title: &str, minutes: f64) -> Entry {
        let (title, marker) = split_part(title);
        Entry {
            item_id: format!("i{e}"),
            key: format!("k{e}"),
            e,
            e_end,
            title,
            marker,
            minutes: Some(minutes),
            ..Default::default()
        }
    }

    fn plan_of(entries: &[Entry], eps: &[TmdbEp]) -> Option<Vec<Option<(i32, Option<i32>, Option<i32>)>>> {
        align(entries, eps).map(|(p, _, _)| targets(eps, &p))
    }

    #[test]
    fn part_markers() {
        assert_eq!(split_part("The One In Vegas (2)"), ("The One In Vegas".into(), Some(2)));
        assert_eq!(split_part("My Finale Part 2"), ("My Finale".into(), Some(2)));
        assert_eq!(split_part("Lecture Circuit Pt. 1"), ("Lecture Circuit".into(), Some(1)));
        assert_eq!(
            split_part("Part Two"),
            ("Part Two".into(), None),
            "a title that is only a marker stays"
        );
        assert_eq!(split_part("Catch-22"), ("Catch-22".into(), None));
        assert_eq!(split_part("Episode 2"), ("Episode 2".into(), None));
    }

    #[test]
    fn numbering_that_fits_stays() {
        let eps: Vec<_> = (1..=6).map(|n| ep(n, &format!("Title {n}"), 22.0)).collect();
        let entries: Vec<_> = (1..=6).map(|n| entry(n, None, &format!("Title {n}"), 22.0)).collect();
        assert!(plan_of(&entries, &eps).is_none());
        // No titles, a missing file: still the numbering.
        let entries: Vec<_> = [1, 2, 4, 5].iter().map(|&n| entry(n, None, "", 22.0)).collect();
        assert!(plan_of(&entries, &eps).is_none());
        // Past TMDB's season end with nothing to say otherwise: left as it is.
        let entries: Vec<_> = (1..=8).map(|n| entry(n, None, "", 22.0)).collect();
        assert!(plan_of(&entries, &eps).is_none());
    }

    #[test]
    fn split_finale() {
        // Friends S5: (1) is TMDB's E23 (by its number), (2) is numbered past the season.
        let mut eps: Vec<_> = (1..=22).map(|n| ep(n, &format!("The One {n}"), 22.0)).collect();
        eps.push(ep(23, "The One in Vegas", 44.0));
        let mut entries: Vec<_> = (1..=22).map(|n| entry(n, None, &format!("The One {n}"), 22.0)).collect();
        entries.push(entry(23, None, "The One In Vegas (1)", 22.2));
        entries.push(entry(24, None, "The One In Vegas (2)", 22.2));
        let t = plan_of(&entries, &eps).expect("aligned");
        assert_eq!(t[22], Some((23, None, Some(1))));
        assert_eq!(t[23], Some((23, None, Some(2))));
        assert!(t[..22].iter().enumerate().all(|(i, x)| *x == Some((i as i32 + 1, None, None))));
    }

    #[test]
    fn split_mid_season_shifts_the_rest() {
        // Friends S6: "Could Have Been (1)/(2)" is TMDB's E15; everything after is one ahead.
        let names = [
            "A",
            "B",
            "The One That Could Have Been",
            "The One with Unagi",
            "The One Where Ross Dates a Student",
            "The One with Joey's Fridge",
        ];
        let eps: Vec<_> = names
            .iter()
            .enumerate()
            .map(|(i, n)| ep(13 + i as i32, n, if i == 2 { 44.0 } else { 22.0 }))
            .collect();
        let files = [
            "A",
            "B",
            "The One That Could Have Been (1)",
            "The One That Could Have Been (2)",
            "The One With Unagi",
            "The One Where Ross Dates A Student",
            "The One With Joey's Fridge",
        ];
        let entries: Vec<_> = files.iter().enumerate().map(|(i, t)| entry(13 + i as i32, None, t, 22.0)).collect();
        let t = plan_of(&entries, &eps).expect("aligned");
        assert_eq!(
            t,
            vec![
                Some((13, None, None)),
                Some((14, None, None)),
                Some((15, None, Some(1))),
                Some((15, None, Some(2))),
                Some((16, None, None)),
                Some((17, None, None)),
                Some((18, None, None))
            ]
        );
    }

    #[test]
    fn double_numbered_hour_longs() {
        // The Office S4: files numbered as two episodes each are TMDB's one.
        let names = [
            "Fun Run",
            "Dunder Mifflin Infinity",
            "Launch Party",
            "Money",
            "Local Ad",
            "Branch Wars",
        ];
        let eps: Vec<_> = names
            .iter()
            .enumerate()
            .map(|(i, n)| ep(i as i32 + 1, n, if i < 4 { 42.0 } else { 22.0 }))
            .collect();
        let entries = vec![
            entry(1, Some(2), "Fun Run", 42.0),
            entry(3, Some(4), "Dunder Mifflin Infinity", 42.0),
            entry(5, Some(6), "Launch Party", 42.0),
            entry(7, Some(8), "Money", 42.0),
            entry(9, None, "Local Ad", 21.6),
            entry(10, None, "Branch Wars", 21.9),
        ];
        let t = plan_of(&entries, &eps).expect("aligned");
        assert_eq!(t, (1..=6).map(|n| Some((n, None, None))).collect::<Vec<_>>());
    }

    #[test]
    fn two_episodes_in_one_file_stay() {
        let eps = vec![
            ep(11, "Back from Vacation", 22.0),
            ep(12, "Traveling Salesmen", 22.0),
            ep(13, "The Return", 22.0),
            ep(14, "Ben Franklin", 22.0),
        ];
        let entries = vec![
            entry(11, None, "Back from Vacation", 21.0),
            entry(12, Some(13), "Traveling Salesmen & The Return", 42.7),
            entry(14, None, "Ben Franklin", 21.0),
        ];
        assert!(plan_of(&entries, &eps).is_none());
    }

    #[test]
    fn untitled_halves_by_runtime() {
        // House S6: a 90-minute premiere as two 45-minute files, names without titles.
        let mut eps = vec![ep(1, "Broken", 88.0)];
        eps.extend((2..=21).map(|n| ep(n, &format!("T{n}"), 44.0)));
        let entries: Vec<_> = (1..=22).map(|n| entry(n, None, "", 44.5)).collect();
        let t = plan_of(&entries, &eps).expect("aligned");
        assert_eq!(t[0], Some((1, None, Some(1))));
        assert_eq!(t[1], Some((1, None, Some(2))));
        assert_eq!(t[21], Some((21, None, None)));
    }

    #[test]
    fn a_different_cut_is_left_alone() {
        // Ten 52-minute episodes against TMDB's six of an hour, other titles.
        let eps: Vec<_> = [
            "Gentlemen, Start Your Engines",
            "Scotland to Tobruk",
            "Tobruk to Ethiopia",
            "Kenya",
            "Kenya to Tanzania",
            "Tanzania to Cape Town",
        ]
        .iter()
        .enumerate()
        .map(|(i, n)| ep(i as i32 + 1, n, 60.0))
        .collect();
        let files = [
            "Gentlemen Start Your Engines",
            "Scotland to Italy",
            "Rome to Tobruk",
            "Tobruk to Khartoum",
            "Sudan to Ethiopia",
            "Into Kenya",
            "Kenya to Rwanda",
            "Kigali to Malawi",
            "Malawi to Botswana",
            "Botswana to Cape Town",
        ];
        let entries: Vec<_> = files.iter().enumerate().map(|(i, t)| entry(i as i32 + 1, None, t, 52.0)).collect();
        if let Some(t) = plan_of(&entries, &eps) {
            // Whatever it finds, no two whole episodes become parts of one.
            assert!(t.iter().all(|x| x.is_none_or(|(_, _, p)| p.is_none())), "{t:?}");
        }
    }
}
