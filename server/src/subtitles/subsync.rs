//! Downloaded subtitles matched to this file's timing.
//!
//! A subtitle from elsewhere was often timed for another release: a fixed offset, another speed
//! (23.976, 24 and 25 fps conversions), or a cut or recap the other way. Only tracks that came
//! from elsewhere are matched: fetched (OpenSubtitles) and uploaded ones, including those fetched
//! before matching existed. The file's own tracks, embedded or sidecar, came with it and are
//! taken as they are. When such a track is fetched or uploaded (or first asked for, for older
//! ones), a background job compares its cues with what the file itself says:
//!
//! - **A reference.** An embedded text track, in any language (its cues fall when people speak),
//!   as extracted anyway (`subs.rs`). Else, or if that gives no clear answer, the speech detected
//!   in the main audio track: the whole track of a small file, else windows spread over the film
//!   (`plan_windows`; they cost a read each over a share, and more of them are read only to
//!   locate a cut or to settle a part of the track that couldn't be placed, `next_windows`).
//!   What was detected is kept per file, so another subtitle costs no read.
//! - **The model.** `video time = scale × subtitle time + offset`, the scale one of the frame
//!   rate conversions and the offset allowed to change at a few places. For each scale the
//!   best overall offset comes from correlating cue and reference intervals; around it a Viterbi
//!   pass over the cues (states: offsets, each change of offset costing `SPLIT_COST`) finds the
//!   splits, and each piece is kept only if its cues stand clear of anywhere else they could go
//!   (`confirm_splits`). For the chosen scale a second pass (`among`) chooses again among just
//!   those offsets, the original timing, and where stretches of the track stand out on their own:
//!   a part too short to pay for a split among thousands of offsets can still show it belongs at
//!   one of a few. Time outside the reference's coverage counts neither way.
//! - **The verdict.** `verdict` applies the result only if the fit is clearly better than chance
//!   and than the original timing, measured against what chance makes of the same two timings
//!   (`gain`), not a fixed share; and only if no large part of the track is left that it could
//!   damage: one that can't be placed, one clearly better off at its original timing, or one
//!   moved where nothing says it belongs (`Fit::unresolved`). Otherwise the original stays, and
//!   the track says why.
//!
//! How this does on real titles is measured by `subsync_eval.rs`.
//!
//! The original is never changed: the matched track is a second file next to `timing.json`, in
//! the directory of the file's fetched tracks. `timing.json` also keeps the viewer's nudge and
//! "original timing" choice, per file and track, for every device. The app draws its overlay
//! from the `.srt` with the nudge as its own delay; HLS renditions (PiP, AirPlay, downloads) have
//! the nudge applied here, since their player can't delay them.

#![allow(
    clippy::float_cmp,
    reason = "scales are compared with the SCALES they were set from, offsets and sentinels with the values they were copied from"
)]

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{atomic::Ordering, Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncReadExt, sync::watch};

use crate::{
    media::{fnv, Media, SubtitleTrack},
    subtitles::subs::{self, Subs},
    subtitles::subx::{self, Cue},
};

/// Results of an older algorithm are found again: an older match could damage a large part of a
/// track (`damage`).
const ALGORITHM: u32 = 3;

pub type Span = (f64, f64);

// ---------------------------------------------------------------- the aligner

/// `video = scale × subtitle`: as is, and the conversions between 24000/1001, 24 and 25 fps.
const SCALES: [f64; 7] = [
    1.0,
    24.0 / 25.0,
    25.0 / 24.0,
    24000.0 / 1001.0 / 25.0,
    25.0 * 1001.0 / 24000.0,
    1000.0 / 1001.0,
    1001.0 / 1000.0,
];
/// The overall offset is looked for this far either way (seconds), in steps of `COARSE`.
const RANGE: f64 = 900.0;
const COARSE: f64 = 0.05;
/// A split may change the offset this far from the overall one, in steps of `STEP`.
const SPLIT_RANGE: f64 = 600.0;
const STEP: f64 = 0.1;
/// A peak in the overall correlation is judged against this much either side (seconds).
const PEAK_REACH: f64 = 15.0;
/// What a change of offset costs, in seconds of cue time matched (a cue is about 2 s). High
/// enough that a few cues finding a better place by chance, among thousands of offsets, don't
/// split; a real cut leaves far more cues than that off. If that still gives more pieces than a
/// film has edits (`MAX_PIECES`, densely spoken titles can pile cues onto the talkiest stretch),
/// it is tried again at twice the cost.
const SPLIT_COST: f64 = 40.0;
/// Subtitles come in order, so a later line landing before an earlier one needs explaining: a
/// recap only the subtitle's release has, say. Each second of that costs this much on top: on
/// talkative titles, without it, stretches of cues found more speech somewhere minutes away.
const BACK_COST: f64 = 0.35;
/// The second pass (`among`): a change of offset among a few costs this share of `SPLIT_COST`.
/// The few are offsets at least `DISTINCT` (seconds) apart, among them those where a stretch of
/// the track stands out by `MIN_PEAK` on its own (`part_peaks`; a run there still has to be
/// confirmed, `confirm_splits`).
const AMONG_COST: f64 = 0.25;
const DISTINCT: f64 = 1.5;
const MIN_PEAK: f64 = 2.0;
/// A run of cues with at least this share of the lines that can't be placed leaves the match
/// unsure (`confirm_splits`). Smaller ones follow the rest.
const UNRESOLVED_SHARE: f64 = 0.1;
/// Moving lines less than this (seconds) is no great damage, whichever way is right.
const SMALL_MOVE: f64 = 1.0;
/// Another speed has to be this much better than the file's own (seconds of cue time matched).
/// A track shorter than `SHORT_TRACK` (seconds) takes a speed a thousandth off at
/// `SHORT_SCALE_MARGIN` if that is also `MIN_SCALE_GAIN` clear (`gain`).
const SCALE_MARGIN: f64 = 10.0;
const SHORT_TRACK: f64 = 2500.0;
const SHORT_SCALE_MARGIN: f64 = 4.0;
const MIN_SCALE_GAIN: f64 = 2.5;

/// What the cues are compared with: when speech is known to happen, and when that is known at
/// all (audio read only in places).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Reference {
    pub speech: Vec<Span>,
    pub coverage: Vec<Span>,
    /// The end of the video. Before its start and after this there is certainly no speech: a cue
    /// moved there doesn't get away with counting nothing.
    pub end: f64,
}

impl Reference {
    /// Cues of a track of the file itself: known from start to end.
    pub fn from_cues(cues: &[Span], duration: f64) -> Reference {
        let end = cues.iter().map(|c| c.1).fold(duration, f64::max);
        Reference {
            speech: merge(cues.to_vec()),
            coverage: vec![(0.0, end)],
            end,
        }
    }

    /// With the time outside the video counted as known and silent.
    fn with_outside(&self) -> Reference {
        let far = 1.0e6;
        let mut coverage = vec![(-far, 0.0), (self.end, far)];
        coverage.extend(self.coverage.iter().copied());
        Reference {
            speech: self.speech.clone(),
            coverage: merge(coverage),
            end: self.end,
        }
    }

    /// The share of the covered time with speech.
    fn density(&self) -> f64 {
        let total: f64 = self.coverage.iter().map(|c| c.1 - c.0).sum();
        let speech: f64 = self.speech.iter().map(|c| c.1 - c.0).sum();
        if total <= 0.0 {
            return 0.0;
        }
        speech / total
    }
}

/// Sorted, without overlaps.
pub fn merge(mut spans: Vec<Span>) -> Vec<Span> {
    spans.retain(|s| s.1 > s.0);
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for s in spans {
        match out.last_mut() {
            Some(l) if s.0 <= l.1 => l.1 = l.1.max(s.1),
            _ => out.push(s),
        }
    }
    out
}

/// Time in `spans` (sorted, disjoint) during [a, b].
fn overlap(spans: &[Span], a: f64, b: f64) -> f64 {
    let first = spans.partition_point(|s| s.1 <= a);
    spans[first..].iter().take_while(|s| s.0 < b).map(|s| s.1.min(b) - s.0.max(a)).sum()
}

/// A line of speech, not a sound effect, a song or the uploader's credit.
fn is_dialogue(text: &str) -> bool {
    const CREDITS: [&str; 12] = [
        "www.",
        ".com",
        ".org",
        ".net",
        "opensubtitles",
        "addic7ed",
        "subtitles by",
        "subtitled by",
        "synced by",
        "sync by",
        "corrected by",
        "captioned by",
    ];
    let mut plain = String::with_capacity(text.len());
    let mut until: Option<char> = None;
    for c in text.chars() {
        match until {
            Some(end) => {
                if c == end {
                    until = None;
                }
            }
            None => match c {
                '<' => until = Some('>'),
                '{' => until = Some('}'),
                '[' => until = Some(']'),
                '(' => until = Some(')'),
                '♪' | '♫' => return false,
                _ => plain.push(c),
            },
        }
    }
    if !plain.chars().any(char::is_alphabetic) {
        return false;
    }
    let lower = plain.to_lowercase();
    !CREDITS.iter().any(|c| lower.contains(c))
}

/// The cues that count for timing: spoken lines of a sensible length.
pub fn dialogue(cues: &[Cue]) -> Vec<Span> {
    cues.iter()
        .filter(|c| (0.2..=15.0).contains(&(c.end - c.start)) && is_dialogue(&c.text))
        .map(|c| (c.start, c.end))
        .collect()
}

/// A grid of offsets, and the second differences of a sum of overlaps over it: a cue slid over
/// a span overlaps it by a trapezoid in the offset, four changes of slope.
struct Bins {
    lo: f64,
    /// `lo` in steps of `dt`.
    first: i64,
    dt: f64,
    second: Vec<f64>,
    /// What changes of slope before the grid amount to at its start.
    value: f64,
    slope: f64,
}

impl Bins {
    fn new(lo: f64, hi: f64, dt: f64) -> Bins {
        let first = (lo / dt).round() as i64;
        Bins {
            lo: first as f64 * dt,
            first,
            dt,
            second: vec![0.0; ((hi - lo) / dt).round() as usize + 1],
            value: 0.0,
            slope: 0.0,
        }
    }

    fn clear(&mut self) {
        self.second.fill(0.0);
        self.value = 0.0;
        self.slope = 0.0;
    }

    /// A change of slope at `at` steps of the grid from offset 0.
    fn impulse(&mut self, at: i64, weight: f64) {
        let p = at - self.first;
        if p < 0 {
            self.value += weight * -p as f64;
            self.slope += weight;
        } else if let Some(s) = self.second.get_mut(p as usize) {
            *s += weight;
        }
    }

    /// `weight` × the overlap of the cue [s, e], moved by each offset, with `spans`. All four ends
    /// are rounded to the grid first: rounded apart, a trapezoid wouldn't come back to 0, and a
    /// film's worth of those would leave a drift bigger than the peak.
    fn add(&mut self, (s, e): Span, spans: &[Span], weight: f64) {
        let (lo, dt) = (self.lo, self.dt);
        let hi = lo + dt * (self.second.len() - 1) as f64;
        let on_grid = |t: f64| (t / dt).round() as i64;
        let (qs, qe) = (on_grid(s), on_grid(e));
        let first = spans.partition_point(|x| x.1 + dt <= s + lo);
        for &(a, b) in spans[first..].iter().take_while(|x| x.0 - dt < e + hi) {
            let (qa, qb) = (on_grid(a), on_grid(b));
            self.impulse(qa - qe, weight);
            self.impulse(qa - qs, -weight);
            self.impulse(qb - qe, -weight);
            self.impulse(qb - qs, weight);
        }
    }

    /// The sum at every offset of the grid (seconds).
    fn sum(&self, out: &mut Vec<f64>) {
        out.clear();
        let (mut value, mut slope) = (self.value, self.slope);
        for d in &self.second {
            slope += d;
            out.push(value * self.dt);
            value += slope;
        }
    }
}

/// The offset where the sum stands out most from what surrounds it, and by how many standard
/// deviations: the true offset is a narrow peak, while how much of a track falls on the reference
/// at all changes slowly with the offset (and a densely spoken title is on speech nearly anywhere).
fn sharpest(sums: &[f64], reach: usize) -> Option<(usize, f64)> {
    let n = sums.len();
    if n < 2 * reach + 1 {
        return None;
    }
    let rest = detrended(sums, reach);
    let mean = rest.iter().sum::<f64>() / n as f64;
    let sd = (rest.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64).sqrt();
    let (at, peak) = rest.iter().copied().enumerate().max_by(|a, b| a.1.total_cmp(&b.1))?;
    Some((at, if sd > 0.0 { (peak - mean) / sd } else { 0.0 }))
}

/// What stands out of `sums` from the mean of `reach` either side.
fn detrended(sums: &[f64], reach: usize) -> Vec<f64> {
    let n = sums.len();
    let mut prefix = Vec::with_capacity(n + 1);
    prefix.push(0.0);
    for x in sums {
        prefix.push(prefix[prefix.len() - 1] + x);
    }
    (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(reach), (i + reach + 1).min(n));
            sums[i] - (prefix[b] - prefix[a]) / (b - a) as f64
        })
        .collect()
}

/// `spans` moved by `offset`, as `video = scale × subtitle + offset` until subtitle time `until`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Piece {
    /// Subtitle time (before scaling) this piece ends at.
    pub until: f64,
    pub offset: f64,
}

/// An alignment and how good it looks.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fit {
    pub scale: f64,
    pub pieces: Vec<Piece>,
    /// Share of the cue time in the reference's coverage that overlaps speech: after, and with
    /// the original timing.
    pub after: f64,
    pub before: f64,
    /// The same by chance (the density of speech).
    pub chance: f64,
    /// The overall correlation's peak in standard deviations of all offsets.
    pub strength: f64,
    /// How clearly the fit beats the original timing (`gain`).
    #[serde(default)]
    pub gain: f64,
    /// Cue time, and cues, with evidence.
    pub covered_secs: f64,
    pub covered_cues: usize,
    /// Where parts of the track that couldn't be placed could be, that the fit makes clearly worse
    /// than their original timing, or that it moves where nothing says they belong (video time):
    /// no answer, unless more audio settles it.
    #[serde(default)]
    pub unresolved: Vec<Span>,
    /// How far above chance the least supported large stretch the fit moves sits where it puts
    /// it (`support`), in standard deviations.
    #[serde(default)]
    pub support: f64,
}

impl Fit {
    /// A time of the subtitle in the video.
    pub fn map(&self, t: f64) -> f64 {
        let piece = self.pieces.iter().find(|p| t < p.until).or(self.pieces.last());
        self.scale * t + piece.map_or(0.0, |p| p.offset)
    }

    /// How far the cues move, typically: the median over `cues`.
    pub fn typical_offset(&self, cues: &[Span]) -> f64 {
        let mut d: Vec<f64> = cues.iter().map(|c| self.map(c.0) - c.0).collect();
        d.sort_by(f64::total_cmp);
        d.get(d.len() / 2).copied().unwrap_or(0.0)
    }
}

/// What to do with a fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Better than chance, better than the original: use it.
    Apply,
    /// The reference agrees with the original timing.
    InSync,
    /// No clear answer (another cut, another title, too little to go on): the original stays.
    Unsure,
}

/// Enough evidence and a fit clearly better than chance, on a scale where 0 is chance and 1 is
/// every cue on speech.
const MIN_LIFT_TEXT: f64 = 0.45;
const MIN_LIFT_AUDIO: f64 = 0.1;
const MIN_STRENGTH: f64 = 5.5;
const MIN_COVERED_SECS: f64 = 40.0;
const MIN_COVERED_CUES: usize = 15;
/// The new timing has to beat the original by this many standard deviations (`gain`), and a split
/// the timing of the cues next to it (`confirm_splits`).
const MIN_GAIN: f64 = 3.0;
const MIN_SPLIT_GAIN: f64 = 3.0;
/// No fit may damage a large part of the track (`align`): the match is unsure if the original
/// timing of a part the fit moves is this much better for it (lower than what a change needs:
/// keeping the original is never the damage), or if a stretch of at least `SUPPORT_SHARE` of the
/// lines that it moves sits less than `MIN_SUPPORT` above chance where it puts it (`support`).
const MIN_HARM_GAIN: f64 = 2.0;
const SUPPORT_SHARE: f64 = 0.1;
const MIN_SUPPORT: f64 = 2.0;
/// Against audio, an offset smaller than this is left alone: speech is found to a few tenths of
/// a second at best, and a subtitle that close was right more often than the detector.
const AUDIO_TOLERANCE: f64 = 0.7;
/// Against a text track, an offset smaller than this is no offset; against one in another
/// language, this (measured: such tracks of one file differ by up to about half a second).
const IDENTITY: f64 = 0.15;
const IDENTITY_OTHER: f64 = 0.6;
/// More places than this where the offset changes isn't an edit, it's a poor match.
const MAX_PIECES: usize = 6;

/// What a fit was made against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    Audio,
    /// A text track of the file in the subtitle's language.
    Text,
    /// One in another language (or one not known): its lines start and end a little differently.
    OtherText,
}

impl Basis {
    fn audio(self) -> bool {
        self == Basis::Audio
    }

    /// An offset smaller than this is no offset.
    fn tolerance(self) -> f64 {
        match self {
            Basis::Audio => AUDIO_TOLERANCE,
            Basis::Text => IDENTITY,
            Basis::OtherText => IDENTITY_OTHER,
        }
    }
}

pub fn verdict(fit: &Fit, basis: Basis) -> Verdict {
    if fit.covered_secs < MIN_COVERED_SECS || fit.covered_cues < MIN_COVERED_CUES {
        return Verdict::Unsure;
    }
    let lift = |x: f64| (x - fit.chance) / (1.0 - fit.chance).max(0.05);
    let min_lift = if basis.audio() { MIN_LIFT_AUDIO } else { MIN_LIFT_TEXT };
    let close = fit.scale == 1.0 && fit.pieces.len() == 1 && fit.pieces[0].offset.abs() < basis.tolerance();
    if fit.strength >= MIN_STRENGTH && lift(fit.before) >= min_lift && close && fit.unresolved.is_empty() {
        return Verdict::InSync;
    }
    // A match elsewhere that isn't clearly better than the original timing is no answer either way.
    if lift(fit.after) < min_lift
        || fit.strength < MIN_STRENGTH
        || fit.pieces.len() > MAX_PIECES
        || fit.gain < MIN_GAIN
        || !fit.unresolved.is_empty()
    {
        return Verdict::Unsure;
    }
    Verdict::Apply
}

/// Share of `cues`' time in the coverage that overlaps speech, that time, and the cues counted.
fn measure(cues: impl Iterator<Item = Span>, r: &Reference) -> (f64, f64, usize) {
    let (mut on, mut covered, mut n) = (0.0, 0.0, 0);
    for (a, b) in cues {
        let c = overlap(&r.coverage, a, b);
        if c > 0.5 * (b - a) {
            n += 1;
        }
        covered += c;
        on += overlap(&r.speech, a, b);
    }
    (if covered > 0.0 { on / covered } else { 0.0 }, covered, n)
}

/// One scale's best: its total (seconds of cue time matched, less the splits), a piece per
/// run of cues with one offset, and the overall peak's strength.
#[derive(Clone)]
struct Candidate {
    scale: f64,
    total: f64,
    strength: f64,
    /// Index of each run's first cue, and its offset.
    runs: Vec<(usize, f64)>,
    /// Where runs that couldn't be placed could be (video time).
    unresolved: Vec<Span>,
    /// The overall offset, what a split cost, and the offsets of the runs found before they
    /// were confirmed.
    center: f64,
    cost: f64,
    found: Vec<f64>,
}

/// The chance-free score of a cue at each offset: +1 a second on speech, and off speech as much
/// less as makes a random offset come to 0. A cue outside the coverage counts nothing.
fn weights(density: f64) -> (f64, f64) {
    let p = density.clamp(0.02, 0.9);
    (1.0 + p / (1.0 - p), -p / (1.0 - p))
}

fn candidate(cues: &[Span], r: &Reference, density: f64, scale: f64) -> Option<Candidate> {
    let scaled: Vec<Span> = cues.iter().map(|c| (c.0 * scale, c.1 * scale)).collect();
    let (on, covered) = weights(density);

    // The overall offset.
    let mut bins = Bins::new(-RANGE, RANGE, COARSE);
    for &c in &scaled {
        bins.add(c, &r.speech, on);
        bins.add(c, &r.coverage, covered);
    }
    let mut sums = Vec::new();
    bins.sum(&mut sums);
    let (peak_at, strength) = sharpest(&sums, (PEAK_REACH / COARSE) as usize)?;
    let center = -RANGE + peak_at as f64 * COARSE;

    // Splits around it, each one only if it clearly earns its place.
    let mut cost = SPLIT_COST;
    loop {
        let (_, found) = splits(&scaled, r, density, center, cost);
        // As splits were judged before `among`: what scales are compared by.
        let (runs, unresolved) = confirm_splits(&scaled, r, density, found.clone(), false, false);
        if runs.len() <= MAX_PIECES {
            let total = runs_total(&scaled, r, density, &runs) - cost * (runs.len() - 1) as f64;
            let found = found.into_iter().map(|x| x.1).collect();
            return Some(Candidate {
                scale,
                total,
                strength,
                runs,
                unresolved,
                center,
                cost,
                found,
            });
        }
        cost *= 2.0;
    }
}

/// The chosen scale's splits again, among just the offsets that came out of the first pass, the
/// original timing, and those where a stretch of the track stands out on its own: with few places
/// to choose from, a part needs far less evidence to show it belongs at one rather than another
/// (than at none of thousands of others, by chance, first). A part the first pass couldn't
/// split off, too short to pay for a split among all offsets, keeps its original timing that way
/// instead of following the rest of the track, or leaves the match unsure.
fn among(c: &mut Candidate, cues: &[Span], r: &Reference, density: f64) {
    let scaled: Vec<Span> = cues.iter().map(|x| (x.0 * c.scale, x.1 * c.scale)).collect();
    let original = c.scale == 1.0;
    let mut offsets = vec![c.center];
    offsets.extend(c.runs.iter().map(|x| x.1));
    offsets.extend(c.found.iter().copied());
    if original {
        offsets.push(0.0);
    }
    offsets.extend(
        part_peaks(&scaled, r, density, c.center)
            .into_iter()
            .filter(|p| p.1 >= MIN_PEAK)
            .map(|p| p.0),
    );
    let mut distinct: Vec<f64> = Vec::new();
    for o in offsets {
        if distinct.iter().all(|d| (d - o).abs() >= DISTINCT) {
            distinct.push(o);
        }
    }
    let runs = splits_among(&scaled, r, density, &distinct, c.cost * AMONG_COST);
    let (runs, unresolved) = confirm_splits(&scaled, r, density, runs, original, true);
    if runs.len() <= MAX_PIECES {
        c.runs = runs;
        c.unresolved = unresolved;
    }
}

/// The track in this many parts (each at least `MIN_PART` lines) when looking for parts that
/// stand out on their own, or that a fit makes clearly worse (`align`).
const PARTS: usize = 10;
const MIN_PART: usize = 25;

/// The parts of a track: index ranges of `n` cues.
fn parts(n: usize) -> Vec<(usize, usize)> {
    let size = (n / PARTS).max(MIN_PART);
    (0..n)
        .step_by(size)
        .map(|a| (a, (a + size).min(n)))
        .filter(|(a, b)| b - a >= MIN_PART / 2)
        .collect()
}

/// The stretches of a track looked at on their own for where they stand out: its parts, and
/// everything before or after each tenth (one cut leaves those apart, however few lines tell).
fn stretches(n: usize) -> Vec<(usize, usize)> {
    let mut out = parts(n);
    for k in 1..PARTS {
        let at = n * k / PARTS;
        out.extend([(0, at), (at, n)].into_iter().filter(|(a, b)| b - a >= MIN_PART));
    }
    out
}

/// Offsets where a stretch of the track stands out clearly on its own (`standout`), around
/// `center`, with by how much.
fn part_peaks(scaled: &[Span], r: &Reference, density: f64, center: f64) -> Vec<(f64, f64)> {
    let (on, covered) = weights(density);
    let reach = (PEAK_REACH / STEP) as usize;
    let mut out = Vec::new();
    for (a, b) in stretches(scaled.len()) {
        let mut bins = Bins::new(center - SPLIT_RANGE, center + SPLIT_RANGE, STEP);
        for &c in &scaled[a..b] {
            bins.add(c, &r.speech, on);
            bins.add(c, &r.coverage, covered);
        }
        let mut sums = Vec::new();
        bins.sum(&mut sums);
        let rest = detrended(&sums, reach);
        let n = rest.len() as f64;
        let mean = rest.iter().sum::<f64>() / n;
        let sd = (rest.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt();
        let Some((at, peak)) = rest.iter().copied().enumerate().max_by(|x, y| x.1.total_cmp(&y.1)) else {
            continue;
        };
        let best = rest
            .iter()
            .enumerate()
            .filter(|(i, _)| i.abs_diff(at) > reach)
            .map(|(_, x)| *x)
            .fold(f64::MIN, f64::max);
        if sd > 0.0 {
            out.push((bins.lo + at as f64 * STEP, (peak - best) / sd));
        }
    }
    out
}

/// What cues placed at these spans score: +1 a second on speech, less off it (`weights`).
fn score(cues: impl Iterator<Item = Span>, r: &Reference, density: f64) -> f64 {
    let (on, covered) = weights(density);
    cues.map(|(a, b)| on * overlap(&r.speech, a, b) + covered * overlap(&r.coverage, a, b))
        .sum()
}

/// The runs' cues (index of each run's first cue, and its offset), as they score where the runs put
/// them.
fn runs_total(scaled: &[Span], r: &Reference, density: f64, runs: &[(usize, f64)]) -> f64 {
    runs.iter()
        .enumerate()
        .map(|(n, &(first, offset))| {
            let end = runs.get(n + 1).map_or(scaled.len(), |x| x.0);
            score(scaled[first..end].iter().map(|c| (c.0 + offset, c.1 + offset)), r, density)
        })
        .sum()
}

/// How clearly one placement of the same cues (`after`, one span per cue of `before`) beats
/// another: the difference of their scores, in standard deviations of that difference with both
/// moved together anywhere else, where neither belongs and only chance tells them apart. Placements
/// a little apart differ little by chance, so a small but real offset still shows; one far apart
/// can win by chance as much as any random offset.
fn gain(before: &[Span], after: &[Span], r: &Reference, density: f64) -> f64 {
    let (on, covered) = weights(density);
    let sums = |cues: &[Span]| {
        let mut bins = Bins::new(-RANGE, RANGE, COARSE);
        for &c in cues {
            bins.add(c, &r.speech, on);
            bins.add(c, &r.coverage, covered);
        }
        let mut out = Vec::new();
        bins.sum(&mut out);
        out
    };
    // Judged against its neighbourhood, as the peak is (`sharpest`): how much of the cues falls in
    // the coverage at all changes slowly with the offset, and differently for the two.
    let d: Vec<f64> = sums(after).iter().zip(sums(before)).map(|(a, b)| a - b).collect();
    let d = detrended(&d, (PEAK_REACH / COARSE) as usize);
    let at = (RANGE / COARSE).round() as usize;
    let n = d.len() as f64;
    let mean = d.iter().sum::<f64>() / n;
    let sd = (d.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt();
    if sd <= 0.0 {
        return 0.0;
    }
    (d[at] - mean) / sd
}

/// How far a run of cues at `offset` stands out from anywhere else it could go, `other` (the
/// offset of the run next to it) included: its score there less the best elsewhere, each judged
/// against its neighbourhood as the overall peak is (`sharpest`), in standard deviations of
/// those. The best of thousands of offsets always looks good; only one well clear of the next
/// best is a place of its own.
fn standout(cues: &[Span], r: &Reference, density: f64, offset: f64, other: f64) -> f64 {
    let (on, covered) = weights(density);
    let mut bins = Bins::new(offset.min(other) - SPLIT_RANGE, offset.max(other) + SPLIT_RANGE, STEP);
    for &c in cues {
        bins.add(c, &r.speech, on);
        bins.add(c, &r.coverage, covered);
    }
    let mut sums = Vec::new();
    bins.sum(&mut sums);
    let reach = (PEAK_REACH / STEP) as usize;
    let rest = detrended(&sums, reach);
    let n = rest.len() as f64;
    let mean = rest.iter().sum::<f64>() / n;
    let sd = (rest.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt();
    let index = |o: f64| (((o - bins.lo) / STEP).round().max(0.0) as usize).min(rest.len() - 1);
    let (at, near) = (index(offset), index(other));
    // Its own peak, a little either way (the offset was found on another grid).
    let own = rest[at.saturating_sub(2)..(at + 3).min(rest.len())]
        .iter()
        .copied()
        .fold(f64::MIN, f64::max);
    let best = rest
        .iter()
        .enumerate()
        .filter(|(i, _)| i.abs_diff(at) > reach)
        .map(|(_, x)| *x)
        .fold(f64::MIN, f64::max);
    if sd <= 0.0 {
        return 0.0;
    }
    (own - best.max(rest[near])) / sd
}

/// How clearly the same cues belong at offset `a` rather than at `b` (`gain`).
fn prefer(cues: &[Span], r: &Reference, density: f64, a: f64, b: f64) -> f64 {
    let at = |o: f64| cues.iter().map(|c| (c.0 + o, c.1 + o)).collect::<Vec<Span>>();
    gain(&at(b), &at(a), r, density)
}

/// Splits that hold up on their own. Every run of cues has to stand out at its offset by
/// `MIN_SPLIT_GAIN` from where it would go otherwise: the offset of the run before or after it,
/// or its original timing (`original`, a track at its own speed), whichever suits it best. An
/// offset found by searching, among thousands, has to stand out from anywhere else as well
/// (`standout`); the original timing was never searched for, so beating the other place is
/// enough (`prefer`). A run that doesn't joins that place, the weakest first: a few dozen lines
/// can find a better place by chance among thousands of offsets, all the more where the speech
/// detector hears little (a recap under music), and a split that moved them there would damage a
/// track that was right.
///
/// Except a large run (`UNRESOLVED_SHARE`) clearly better at its own offset than at that place:
/// moving it there could damage a large part of the track. It stays as it is, and where it could
/// be is returned (video time): no answer, unless more of the audio settles it.
fn confirm_splits(
    scaled: &[Span],
    r: &Reference,
    density: f64,
    mut runs: Vec<(usize, f64)>,
    original: bool,
    hold: bool,
) -> (Vec<(usize, f64)>, Vec<Span>) {
    let mut held: Vec<(usize, Span)> = Vec::new();
    while runs.len() > 1 {
        let mut weakest: Option<(usize, f64, f64, bool)> = None;
        for n in 0..runs.len() {
            let (first, offset) = runs[n];
            if held.iter().any(|h| h.0 == first) {
                continue;
            }
            let end = runs.get(n + 1).map_or(scaled.len(), |x| x.0);
            let cues = &scaled[first..end];
            let at = |o: f64| cues.iter().map(move |c| (c.0 + o, c.1 + o));
            let is_original = original && offset.abs() < STEP / 2.0;
            let mut places: Vec<f64> = [n.checked_sub(1).map(|m| runs[m].1), runs.get(n + 1).map(|x| x.1)]
                .into_iter()
                .flatten()
                .collect();
            if original && !is_original {
                places.push(0.0);
            }
            let Some(other) = places
                .into_iter()
                .max_by(|a, b| score(at(*a), r, density).total_cmp(&score(at(*b), r, density)))
            else {
                continue;
            };
            let z = if is_original {
                prefer(cues, r, density, offset, other)
            } else {
                standout(cues, r, density, offset, other)
            };
            if z < MIN_SPLIT_GAIN && weakest.is_none_or(|w| z < w.1) {
                weakest = Some((n, z, other, is_original));
            }
        }
        let Some((n, _, other, _)) = weakest else { break };
        let (first, offset) = runs[n];
        let end = runs.get(n + 1).map_or(scaled.len(), |x| x.0);
        let cues = &scaled[first..end];
        if hold && (end - first) as f64 >= UNRESOLVED_SHARE * scaled.len() as f64 {
            // Where it is, its original timing, where it fits best on its own: if any of those is
            // clearly better for it, it can't simply follow its neighbour.
            let mut places = vec![offset, own_peak(cues, r, density, offset)];
            if original {
                places.push(0.0);
            }
            if let Some(p) = places
                .into_iter()
                .find(|p| (p - other).abs() > SMALL_MOVE && prefer(cues, r, density, *p, other) >= MIN_SPLIT_GAIN)
            {
                let span = (
                    cues[0].0 + p.min(other),
                    cues.iter().map(|c| c.1).fold(f64::MIN, f64::max) + p.max(other),
                );
                held.push((first, span));
                continue;
            }
        }
        runs[n].1 = other;
        let mut merged: Vec<(usize, f64)> = Vec::with_capacity(runs.len());
        for x in runs {
            match merged.last() {
                Some(l) if l.1 == x.1 => {}
                _ => merged.push(x),
            }
        }
        runs = merged;
    }
    let unresolved = held.into_iter().filter(|h| runs.iter().any(|r| r.0 == h.0)).map(|h| h.1).collect();
    (runs, unresolved)
}

/// Where cues fit best within `SPLIT_RANGE` of `around`, judged against their neighbourhood.
fn own_peak(cues: &[Span], r: &Reference, density: f64, around: f64) -> f64 {
    let (on, covered) = weights(density);
    let mut bins = Bins::new(around - SPLIT_RANGE, around + SPLIT_RANGE, STEP);
    for &c in cues {
        bins.add(c, &r.speech, on);
        bins.add(c, &r.coverage, covered);
    }
    let mut sums = Vec::new();
    bins.sum(&mut sums);
    let rest = detrended(&sums, (PEAK_REACH / STEP) as usize);
    rest.iter()
        .copied()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(around, |(i, _)| bins.lo + i as f64 * STEP)
}

/// The best run of offsets among `offsets` for the cues, by a Viterbi pass as `splits` makes
/// over all of them: each run's first cue with its offset.
fn splits_among(scaled: &[Span], r: &Reference, density: f64, offsets: &[f64], cost: f64) -> Vec<(usize, f64)> {
    let (on, covered) = weights(density);
    let k = offsets.len();
    let mut from = vec![0usize; k * scaled.len()];
    let mut best = vec![0.0f64; k];
    let mut next = vec![0.0f64; k];
    for (i, &c) in scaled.iter().enumerate() {
        let gap = if i == 0 { 0.0 } else { (c.0 - scaled[i - 1].1).max(0.0) };
        for t in 0..k {
            let (mut v, mut j) = (best[t], t);
            for (s, &o) in offsets.iter().enumerate() {
                if s == t {
                    continue;
                }
                let back = BACK_COST * ((o - offsets[t]) - gap).max(0.0);
                let x = best[s] - cost - back;
                if x > v {
                    (v, j) = (x, s);
                }
            }
            let (a, b) = (c.0 + offsets[t], c.1 + offsets[t]);
            next[t] = v + on * overlap(&r.speech, a, b) + covered * overlap(&r.coverage, a, b);
            from[i * k + t] = j;
        }
        std::mem::swap(&mut best, &mut next);
    }
    let mut at = best
        .iter()
        .copied()
        .enumerate()
        .fold((0, f64::MIN), |m, (j, x)| if x > m.1 { (j, x) } else { m })
        .0;
    let mut runs = Vec::new();
    for i in (0..scaled.len()).rev() {
        let was = from[i * k + at];
        if was != at {
            runs.push((i, offsets[at]));
            at = was;
        } else if i == 0 {
            runs.push((0, offsets[at]));
        }
    }
    runs.reverse();
    runs
}

/// The offset of every run of cues around `center`, by a Viterbi pass whose states are offsets:
/// its total, and each run's first cue with its offset. A change of offset costs `cost`, plus
/// `BACK_COST` per second it sends a cue back before the one that came before it.
fn splits(scaled: &[Span], r: &Reference, density: f64, center: f64, cost: f64) -> (f64, Vec<(usize, f64)>) {
    let (on, covered) = weights(density);
    let mut row_bins = Bins::new(center - SPLIT_RANGE, center + SPLIT_RANGE, STEP);
    let lo = row_bins.lo;
    let k = row_bins.second.len();
    // Which state each state of each cue came from: the offsets fit 16 bits.
    let mut from = vec![0u16; k * scaled.len()];
    let mut best = vec![0.0f64; k];
    let mut next = vec![0.0f64; k];
    let mut row = Vec::with_capacity(k);
    let mut after: Vec<(f64, u16)> = vec![(0.0, 0); k + 1];
    let per_step = BACK_COST * STEP;
    for (i, &c) in scaled.iter().enumerate() {
        row_bins.clear();
        row_bins.add(c, &r.speech, on);
        row_bins.add(c, &r.coverage, covered);
        row_bins.sum(&mut row);
        // From offset j to k the cue lands (j - k) later than it would have: free up to the gap
        // since the last cue, then charged.
        let gap = if i == 0 {
            0
        } else {
            ((c.0 - scaled[i - 1].1).max(0.0) / STEP) as usize
        };
        after[k] = (f64::MIN, 0);
        for j in (0..k).rev() {
            let v = best[j] - per_step * j as f64;
            after[j] = if v > after[j + 1].0 { (v, j as u16) } else { after[j + 1] };
        }
        let mut before = (f64::MIN, 0u16);
        let mut seen = 0;
        let froms = &mut from[i * k..(i + 1) * k];
        for t in 0..k {
            // Everything up to t + gap costs only the split.
            let free_to = (t + gap).min(k - 1);
            while seen <= free_to {
                if best[seen] > before.0 {
                    before = (best[seen], seen as u16);
                }
                seen += 1;
            }
            let mut jump = before;
            if free_to + 1 < k {
                let (v, j) = after[free_to + 1];
                let charged = v + per_step * (t + gap) as f64;
                if charged > jump.0 {
                    jump = (charged, j);
                }
            }
            let stay = best[t];
            if jump.0 - cost > stay {
                froms[t] = jump.1;
                next[t] = jump.0 - cost + row[t];
            } else {
                froms[t] = t as u16;
                next[t] = stay + row[t];
            }
        }
        std::mem::swap(&mut best, &mut next);
    }
    let (mut at, total) = best
        .iter()
        .copied()
        .enumerate()
        .fold((0, f64::MIN), |m, (j, x)| if x > m.1 { (j, x) } else { m });
    // Back through the cues.
    let mut runs = Vec::new();
    let offset = |j: usize| lo + j as f64 * STEP;
    for i in (0..scaled.len()).rev() {
        let was = from[i * k + at] as usize;
        if was != at {
            runs.push((i, offset(at)));
            at = was;
        } else if i == 0 {
            runs.push((0, offset(at)));
        }
    }
    runs.reverse();
    (total, runs)
}

/// The offset of a run of cues to a fraction of `STEP`: the middle of the best ones near it.
fn refine(cues: &[Span], r: &Reference, density: f64, around: f64) -> f64 {
    const DT: f64 = 0.01;
    let (on, covered) = weights(density);
    let mut bins = Bins::new(around - STEP * 2.0, around + STEP * 2.0, DT);
    for &c in cues {
        bins.add(c, &r.speech, on);
        bins.add(c, &r.coverage, covered);
    }
    let mut sums = Vec::new();
    bins.sum(&mut sums);
    let top = sums.iter().copied().fold(f64::MIN, f64::max);
    let near: Vec<usize> = sums
        .iter()
        .enumerate()
        .filter(|(_, x)| **x >= top - 0.002 * top.abs())
        .map(|(i, _)| i)
        .collect();
    match near.get(near.len() / 2) {
        Some(i) => bins.lo + *i as f64 * DT,
        None => around,
    }
}

/// How far above chance cues sit where they are: their score there against the same cues moved
/// anywhere within `SPLIT_RANGE`, judged against its neighbourhood (`sharpest`), in standard
/// deviations.
fn support(cues: &[Span], r: &Reference, density: f64) -> f64 {
    let (on, covered) = weights(density);
    let mut bins = Bins::new(-SPLIT_RANGE, SPLIT_RANGE, STEP);
    for &c in cues {
        bins.add(c, &r.speech, on);
        bins.add(c, &r.coverage, covered);
    }
    let mut sums = Vec::new();
    bins.sum(&mut sums);
    let rest = detrended(&sums, (PEAK_REACH / STEP) as usize);
    let n = rest.len() as f64;
    let mean = rest.iter().sum::<f64>() / n;
    let sd = (rest.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt();
    let at = (SPLIT_RANGE / STEP).round() as usize;
    // Its own peak, a little either way (the offset was refined off the grid).
    let own = rest[at - 2..=at + 2].iter().copied().fold(f64::MIN, f64::max);
    if sd > 0.0 {
        (own - mean) / sd
    } else {
        0.0
    }
}

/// Where moving `cues` to `moved` could damage a large part of the track (video time): parts
/// clearly better off at their original timing, and stretches moved where nothing says they
/// belong (a part with too little evidence of its own follows the rest of the track, rightly or
/// not). And the least `support` of a moved stretch (0 if none moves).
fn damage(cues: &[Span], moved: &[Span], r: &Reference, density: f64) -> (Vec<Span>, f64) {
    let moves = |a: usize, b: usize| {
        let mut m: Vec<f64> = (a..b).map(|i| (moved[i].0 - cues[i].0).abs()).collect();
        m.sort_by(f64::total_cmp);
        m[m.len() / 2] > SMALL_MOVE
    };
    let span = |a: usize, b: usize| {
        let both = || moved[a..b].iter().chain(&cues[a..b]);
        (
            both().map(|c| c.0).fold(f64::MAX, f64::min),
            both().map(|c| c.1).fold(f64::MIN, f64::max),
        )
    };
    let mut out = Vec::new();
    for (a, b) in parts(cues.len()) {
        if moves(a, b) && gain(&moved[a..b], &cues[a..b], r, density) >= MIN_HARM_GAIN {
            out.push(span(a, b));
        }
    }
    let mut least = f64::MAX;
    for (a, b) in stretches(cues.len()) {
        if ((b - a) as f64) < SUPPORT_SHARE * cues.len() as f64 || !moves(a, b) {
            continue;
        }
        let z = support(&moved[a..b], r, density);
        least = least.min(z);
        if z < MIN_SUPPORT {
            out.push(span(a, b));
        }
    }
    (out, if least == f64::MAX { 0.0 } else { least })
}

/// The best alignment of `cues` (spoken lines, sorted by start) to `r`.
pub fn align(cues: &[Span], r: &Reference) -> Option<Fit> {
    if cues.is_empty() || r.speech.is_empty() {
        return None;
    }
    let mut cues = cues.to_vec();
    cues.sort_by(|a, b| a.0.total_cmp(&b.0));
    let density = r.density();
    let r = &r.with_outside();
    let candidates: Vec<Candidate> = SCALES.iter().filter_map(|&s| candidate(&cues, r, density, s)).collect();
    let native = candidates.iter().find(|c| c.scale == 1.0)?;
    let other = candidates
        .iter()
        .filter(|c| c.scale != 1.0)
        .max_by(|a, b| a.total.total_cmp(&b.total));
    let place = |c: &Candidate| {
        let mut pieces = Vec::with_capacity(c.runs.len());
        for (n, &(first, offset)) in c.runs.iter().enumerate() {
            let next = c.runs.get(n + 1).map(|r| r.0);
            let part: Vec<Span> = cues[first..next.unwrap_or(cues.len())]
                .iter()
                .map(|x| (x.0 * c.scale, x.1 * c.scale))
                .collect();
            let until = next.map_or(f64::MAX, |j| f64::midpoint(cues[j - 1].1, cues[j].0));
            pieces.push(Piece {
                until,
                offset: refine(&part, r, density, offset),
            });
        }
        let fit = Fit {
            scale: c.scale,
            pieces,
            after: 0.0,
            before: 0.0,
            chance: density,
            strength: c.strength,
            gain: 0.0,
            covered_secs: 0.0,
            covered_cues: 0,
            unresolved: c.unresolved.clone(),
            support: 0.0,
        };
        let moved: Vec<Span> = cues
            .iter()
            .map(|x| (fit.map(x.0), fit.map(x.0) + (x.1 - x.0) * fit.scale))
            .collect();
        (fit, moved)
    };
    let mut best = match other {
        Some(o) if o.total > native.total + SCALE_MARGIN => o,
        _ => native,
    }
    .clone();
    // On a short track a speed a thousandth off (1000/1001) moves lines too little (a second at
    // most) for the totals to reach `SCALE_MARGIN`. There a smaller margin will do if the fit
    // is also clearly better than at the file's own speed (`gain`) and the opposite thousandth
    // isn't: chance favours both alike.
    let span = cues[cues.len() - 1].1 - cues[0].0;
    if best.scale == 1.0 && native.runs.len() == 1 && span <= SHORT_TRACK {
        let thousandth = |s: f64| candidates.iter().find(|c| c.scale == s && c.runs.len() == 1);
        for (s, opposite) in [(1000.0 / 1001.0, 1001.0 / 1000.0), (1001.0 / 1000.0, 1000.0 / 1001.0)] {
            let (Some(c), Some(o)) = (thousandth(s), thousandth(opposite)) else {
                continue;
            };
            if c.total > native.total + SHORT_SCALE_MARGIN
                && o.total < native.total
                && gain(&place(native).1, &place(c).1, r, density) >= MIN_SCALE_GAIN
            {
                best = c.clone();
                break;
            }
        }
    }
    among(&mut best, &cues, r, density);
    let (mut fit, moved) = place(&best);
    let (after, secs, n) = measure(moved.iter().copied(), r);
    let (before, _, _) = measure(cues.iter().copied(), r);
    fit.gain = gain(&cues, &moved, r, density);
    let (unresolved, support) = damage(&cues, &moved, r, density);
    fit.unresolved.extend(unresolved);
    fit.support = support;
    fit.after = after;
    fit.before = before;
    fit.covered_secs = secs;
    fit.covered_cues = n;
    Some(fit)
}

/// The track with a fit applied: every cue (also the ones that didn't count) moved by its piece
/// and scaled. Where a piece starts earlier than the last one ended, the earlier cue gives way.
pub fn apply(cues: &[Cue], fit: &Fit) -> Vec<Cue> {
    let mut out: Vec<Cue> = cues
        .iter()
        .map(|c| {
            let start = fit.map(c.start);
            Cue {
                start,
                end: start + (c.end - c.start) * fit.scale,
                text: c.text.clone(),
            }
        })
        .filter(|c| c.end > 0.0)
        .map(|c| Cue {
            start: c.start.max(0.0),
            ..c
        })
        .collect();
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    if fit.pieces.len() > 1 {
        for i in 1..out.len() {
            let next = out[i].start;
            if out[i - 1].end > next && next - out[i - 1].start >= 0.5 {
                out[i - 1].end = next;
            }
        }
    }
    out
}

// ---------------------------------------------------------------- speech in audio

/// Detection works on frames of this length (seconds), at this sample rate.
const HOP: f64 = 0.02;
const RATE: usize = 16_000;
/// Speech goes up and down with its syllables, 3–8 times a second; music, crowds and effects
/// mostly don't. Frames count as speech when the level (voice band, dB) moves that way by this
/// much, over about half a second, and is above this (dBFS) at all. Measured on real titles:
/// loudness alone finds no peak at all on densely scored or dubbed ones, this does.
const MIN_MODULATION: f64 = 3.0;
const ABSOLUTE: f32 = -55.0;
const FAST_FRAMES: usize = 3;
const SLOW_FRAMES: usize = 9;
const MODULATION_FRAMES: usize = 31;
/// Gaps shorter than this are inside a sentence, shorter runs aren't one, longer ones are not
/// speech whatever they do.
const CLOSE_GAP: f64 = 0.2;
const MIN_RUN: f64 = 0.2;
const MAX_RUN: f64 = 25.0;

/// Frame levels from raw 16-bit mono samples, as they arrive.
#[derive(Default)]
pub struct Levels {
    pub db: Vec<f32>,
    sum: f64,
    count: usize,
    half: Option<u8>,
}

impl Levels {
    pub fn feed(&mut self, mut bytes: &[u8]) {
        let frame = (RATE as f64 * HOP) as usize;
        if let Some(low) = self.half.take() {
            let Some((&high, rest)) = bytes.split_first() else {
                self.half = Some(low);
                return;
            };
            self.sample(i16::from_le_bytes([low, high]), frame);
            bytes = rest;
        }
        let (pairs, rest) = bytes.as_chunks::<2>();
        for c in pairs {
            self.sample(i16::from_le_bytes(*c), frame);
        }
        self.half = rest.first().copied();
    }

    fn sample(&mut self, s: i16, frame: usize) {
        let x = s as f64 / 32768.0;
        self.sum += x * x;
        self.count += 1;
        if self.count == frame {
            self.db.push((10.0 * (self.sum / frame as f64).max(1e-10).log10()) as f32);
            self.sum = 0.0;
            self.count = 0;
        }
    }
}

/// The moving average of `x` over `n` frames (odd), centred.
fn moving_average(x: &[f64], n: usize) -> Vec<f64> {
    let mut prefix = Vec::with_capacity(x.len() + 1);
    prefix.push(0.0);
    for v in x {
        prefix.push(prefix[prefix.len() - 1] + v);
    }
    (0..x.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(n / 2), (i + n / 2 + 1).min(x.len()));
            (prefix[b] - prefix[a]) / (b - a) as f64
        })
        .collect()
}

/// Runs of speech in frame levels (seconds from the first frame).
pub fn speech_runs(db: &[f32]) -> Vec<Span> {
    let levels: Vec<f64> = db.iter().map(|x| *x as f64).collect();
    let (fast, slow) = (moving_average(&levels, FAST_FRAMES), moving_average(&levels, SLOW_FRAMES));
    let moved: Vec<f64> = fast.iter().zip(&slow).map(|(a, b)| (a - b).powi(2)).collect();
    let modulation = moving_average(&moved, MODULATION_FRAMES);
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for i in 0..=db.len() {
        let on = i < db.len() && db[i] > ABSOLUTE && modulation[i].sqrt() > MIN_MODULATION;
        match (on, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                runs.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    let gap = (CLOSE_GAP / HOP) as usize;
    let mut closed: Vec<(usize, usize)> = Vec::with_capacity(runs.len());
    for r in runs {
        match closed.last_mut() {
            Some(l) if r.0 - l.1 <= gap => l.1 = r.1,
            _ => closed.push(r),
        }
    }
    closed
        .into_iter()
        .map(|(a, b)| (a as f64 * HOP, b as f64 * HOP))
        .filter(|(a, b)| (MIN_RUN..=MAX_RUN).contains(&(b - a)))
        .collect()
}

/// Windows of a film read for speech.
const WINDOW_SECS: f64 = 60.0;
const MIN_WINDOWS: usize = 12;
const MAX_WINDOWS: usize = 36;
/// About this much is read of a big file (its bitrate says how much a minute takes).
const READ_BUDGET: f64 = 1.5e9;
/// The first and last part of a film (logos, credits) tell little.
const SKIP_START: f64 = 0.03;
const SKIP_END: f64 = 0.07;
/// A cut is located to within this much of the film, at most `MAX_EXTRA` more windows.
const LOCATE_GAP: f64 = 150.0;
const MAX_EXTRA: usize = 12;
/// Windows read at a time where a part of the track couldn't be placed.
const UNRESOLVED_READS: usize = 3;

/// Where to read a film's audio. `size` bytes in `duration` seconds: everything if that's about
/// the budget or most of the film would be read anyway, else windows spread over it.
pub fn plan_windows(duration: f64, size: Option<u64>) -> Vec<Span> {
    let per_second = size.map_or(1.0e6, |s| s as f64 / duration.max(1.0));
    let n = ((READ_BUDGET / (per_second * WINDOW_SECS)) as usize).clamp(MIN_WINDOWS, MAX_WINDOWS);
    let (from, to) = (duration * SKIP_START, duration * (1.0 - SKIP_END));
    let reach = to - from;
    if per_second * duration <= READ_BUDGET || n as f64 * WINDOW_SECS >= 0.7 * reach {
        return vec![(0.0, duration)];
    }
    let slot = reach / n as f64;
    (0..n)
        .map(|i| {
            let center = from + slot * (i as f64 + 0.5);
            (center - WINDOW_SECS / 2.0, center + WINDOW_SECS / 2.0)
        })
        .collect()
}

/// Windows to read next to locate the fit's splits: the middle of the unread stretch around each
/// one, if that's still long.
fn locate_windows(fit: &Fit, coverage: &[Span], duration: f64) -> Vec<Span> {
    let mut out = Vec::new();
    for pair in fit.pieces.windows(2) {
        let at = fit.scale * pair[0].until + pair[0].offset;
        let before = coverage.iter().map(|c| c.1).filter(|e| *e <= at).fold(0.0, f64::max);
        let after = coverage.iter().map(|c| c.0).filter(|s| *s >= at).fold(duration, f64::min);
        if after - before > LOCATE_GAP {
            let mid = f64::midpoint(before, after);
            let w = (mid - WINDOW_SECS / 2.0, mid + WINDOW_SECS / 2.0);
            if !out.contains(&w) {
                out.push(w);
            }
        }
    }
    out
}

/// What to read next after a fit on the audio known so far (`extra` windows read already for
/// this): nothing if the fit stands as it is.
fn next_windows(fit: &Fit, coverage: &[Span], duration: f64, extra: usize) -> Vec<Span> {
    if extra >= MAX_EXTRA {
        return Vec::new();
    }
    // Where parts of the track couldn't be placed: the longest stretches not read yet there.
    let mut todo: Vec<Span> = Vec::new();
    for &(a, b) in &fit.unresolved {
        let (a, b) = (a.max(0.0), b.min(duration));
        let mut gaps: Vec<Span> = Vec::new();
        let mut from = a;
        for c in coverage.iter().filter(|c| c.1 > a && c.0 < b) {
            gaps.push((from, c.0.max(from)));
            from = from.max(c.1);
        }
        gaps.push((from, b));
        gaps.retain(|g| g.1 - g.0 >= 2.0 * WINDOW_SECS);
        gaps.sort_by(|x, y| (y.1 - y.0).total_cmp(&(x.1 - x.0)));
        for g in gaps.into_iter().take(UNRESOLVED_READS) {
            let mid = f64::midpoint(g.0, g.1);
            let w = (mid - WINDOW_SECS / 2.0, mid + WINDOW_SECS / 2.0);
            if !todo.contains(&w) {
                todo.push(w);
            }
        }
    }
    if todo.is_empty() && verdict(fit, Basis::Audio) == Verdict::Apply && fit.pieces.len() >= 2 {
        todo = locate_windows(fit, coverage, duration);
    }
    todo.truncate(MAX_EXTRA - extra);
    todo
}

// ---------------------------------------------------------------- what is kept per file

/// What the job found for a track (its original text's `stamp`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outcome {
    pub stamp: u64,
    pub algorithm: u32,
    /// "synced", "in_sync", "unsure" or "failed".
    pub state: String,
    /// "subtitles" or "audio".
    pub reference: Option<String>,
    /// How far cues move (the median), seconds.
    pub offset: f64,
    pub scale: f64,
    pub pieces: usize,
    pub fit: Option<Fit>,
    /// The matched track, next to `timing.json`.
    pub file: Option<String>,
    pub at: u64,
    /// Matched on the title's timeline, for a file with a start time (`Media::clock_offset`).
    /// Before 0.31.11 such a file's references were on its own clock: those answers are found
    /// again (`current`). Not written for other files, whose answers stay as they are.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub timeline: bool,
}

impl Outcome {
    /// Found for this file as it is matched now (`timeline`).
    fn current(&self, media: &Media) -> bool {
        self.timeline || media.clock_offset() == 0.0
    }
}

/// A track's entry in `timing.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Entry {
    pub file_key: String,
    /// The viewer's correction (seconds, positive: later).
    #[serde(default)]
    pub nudge: f64,
    /// The viewer turned the automatic match off.
    #[serde(default)]
    pub original: bool,
    #[serde(default)]
    pub outcome: Option<Outcome>,
}

/// A track's timing as the apps see it.
#[derive(Clone, Debug, Serialize)]
pub struct Timing {
    /// "none" (nothing to match: the file's own track, or switched off; or a downloaded track
    /// whose match hasn't been asked for yet), "pending" (running), "synced", "in_sync",
    /// "unsure" or "failed".
    pub state: String,
    /// Against what, when known: "subtitles" or "audio".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// How far the match moved the cues (the median), seconds.
    pub offset: f64,
    pub scale: f64,
    pub pieces: usize,
    pub nudge: f64,
    /// The viewer chose the original timing over the match.
    pub original: bool,
}

/// How a request changes a track's timing.
#[derive(Debug, Deserialize)]
pub struct TimingChange {
    pub nudge: Option<f64>,
    pub original: Option<bool>,
}

/// The largest nudge kept (seconds).
const MAX_NUDGE: f64 = 3600.0;

/// A file's speech as read so far, kept in its extraction directory.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Speech {
    stream: i64,
    reference: Reference,
    /// Read on the title's timeline, for a file with a start time (`Outcome::timeline`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    timeline: bool,
}

/// The jobs running (`Subs::sync`).
pub struct Jobs {
    running: Mutex<HashMap<String, watch::Receiver<bool>>>,
    /// One file's audio read at a time.
    audio: tokio::sync::Semaphore,
    /// Serialises writes of `timing.json`.
    store: tokio::sync::Mutex<()>,
}

impl Jobs {
    pub fn new() -> Jobs {
        Jobs {
            running: Mutex::default(),
            audio: tokio::sync::Semaphore::new(1),
            store: tokio::sync::Mutex::default(),
        }
    }

    pub fn running_count(&self) -> usize {
        self.running.lock().unwrap().len()
    }
}

fn track_key(t: &SubtitleTrack) -> String {
    if let Some(f) = &t.file {
        return format!("x:{}", f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    }
    if let Some(s) = t.stream {
        return format!("e:{s}");
    }
    format!("s:{}", t.path.as_deref().map(crate::storage::vfs::file_name).unwrap_or_default())
}

/// Only downloaded subtitles (fetched or uploaded, kept in the data directory) are matched. A
/// file's own tracks, embedded or sidecar, came with it: they are its timing.
pub fn matchable(t: &SubtitleTrack) -> bool {
    t.text && t.file.is_some()
}

fn read_store(dir: &Path) -> HashMap<String, Entry> {
    std::fs::read(dir.join("timing.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

impl Subs {
    fn timing_dir(&self, media: &Media) -> PathBuf {
        self.extras_dir(media)
    }

    /// A track's entry, if it is about this very file.
    fn entry(&self, media: &Media, t: &SubtitleTrack) -> Entry {
        read_store(&self.timing_dir(media))
            .remove(&track_key(t))
            .filter(|e| e.file_key == media.file_key)
            .unwrap_or_default()
    }

    async fn update_entry(&self, media: &Media, t: &SubtitleTrack, f: impl FnOnce(&mut Entry)) -> Result<Entry> {
        let _g = self.sync.store.lock().await;
        let dir = self.timing_dir(media);
        tokio::fs::create_dir_all(&dir).await?;
        let mut all = read_store(&dir);
        let e = all.entry(track_key(t)).or_default();
        if e.file_key != media.file_key {
            *e = Entry {
                file_key: media.file_key.clone(),
                ..Default::default()
            };
        }
        f(e);
        let out = e.clone();
        subs::write_atomic(&dir.join("timing.json"), &serde_json::to_vec_pretty(&all)?).await?;
        Ok(out)
    }

    fn job_key(media: &Media, t: &SubtitleTrack) -> String {
        format!("{}|{}", media.file_key, track_key(t))
    }

    /// A track as the apps see it.
    pub fn timing(&self, media: &Media, t: &SubtitleTrack) -> Timing {
        let e = self.entry(media, t);
        let running = self.sync.running.lock().unwrap().contains_key(&Self::job_key(media, t));
        // Switched off: tracks are served as they are, whatever was found before.
        let o = e.outcome.as_ref().filter(|o| self.sync_enabled() && o.current(media));
        // "pending" only while a match runs: a track fetched before matching existed has none
        // until it is first asked for.
        let state = match (running, o) {
            (true, _) if self.sync_enabled() => "pending".to_string(),
            (false, Some(o)) => o.state.clone(),
            _ => "none".to_string(),
        };
        Timing {
            state,
            reference: o.and_then(|o| o.reference.clone()),
            offset: o.map_or(0.0, |o| o.offset),
            scale: o.map_or(1.0, |o| o.scale),
            pieces: o.map_or(1, |o| o.pieces),
            nudge: e.nudge,
            original: e.original,
        }
    }

    /// The track's timing, its match started if that's still to do (the app asks after picking a
    /// track, and until the answer stops being "pending").
    pub async fn timing_now(self: &Arc<Self>, media: &Arc<Media>, index: i64) -> Result<Timing> {
        let t = media
            .subtitles
            .iter()
            .find(|s| s.index == index && s.text)
            .context("no such text subtitle track")?
            .clone();
        self.ensure_sync(media, &t).await?;
        Ok(self.timing(media, &t))
    }

    /// Starts a downloaded track's match if it is still to do.
    async fn ensure_sync(self: &Arc<Self>, media: &Arc<Media>, t: &SubtitleTrack) -> Result<()> {
        if matchable(t) && self.sync_enabled() {
            let original = self.original_srt(media, t).await?;
            self.start_sync(media, t, &original);
        }
        Ok(())
    }

    pub async fn change_timing(self: &Arc<Self>, media: &Arc<Media>, index: i64, change: TimingChange) -> Result<Timing> {
        let t = media
            .subtitles
            .iter()
            .find(|s| s.index == index && s.text)
            .context("no such text subtitle track")?
            .clone();
        self.update_entry(media, &t, |e| {
            if let Some(n) = change.nudge.filter(|n| n.is_finite()) {
                e.nudge = (n.clamp(-MAX_NUDGE, MAX_NUDGE) * 1000.0).round() / 1000.0;
            }
            if let Some(o) = change.original {
                e.original = o;
            }
        })
        .await?;
        Ok(self.timing(media, &t))
    }

    /// The viewer's nudge of a track (what renditions apply).
    pub fn nudge(&self, media: &Media, index: i64) -> f64 {
        media
            .subtitles
            .iter()
            .find(|s| s.index == index)
            .map_or(0.0, |t| self.entry(media, t).nudge)
    }

    /// What `srt` serves for a matchable track: the match if there is one and it is wanted, else
    /// the original; a match still to do starts now.
    pub async fn timed(self: &Arc<Self>, media: &Arc<Media>, t: &SubtitleTrack, original: String) -> String {
        if !self.sync_enabled() {
            return original;
        }
        self.start_sync(media, t, &original);
        let e = self.entry(media, t);
        match e
            .outcome
            .filter(|o| !e.original && o.stamp == fnv(&original) && o.current(media))
            .and_then(|o| o.file)
        {
            Some(f) => tokio::fs::read_to_string(self.timing_dir(media).join(f)).await.unwrap_or(original),
            None => original,
        }
    }

    /// Waits for a downloaded track's match, started now if it is still to do (a download keeps
    /// what it fetches).
    pub async fn settled_sync(self: &Arc<Self>, media: &Arc<Media>, index: i64) {
        let Some(t) = media.subtitles.iter().find(|s| s.index == index) else {
            return;
        };
        if let Err(e) = self.ensure_sync(media, t).await {
            tracing::info!(item = %media.item_id, index, "subtitle timing not started: {e:#}");
        }
        let job = self.sync.running.lock().unwrap().get(&Self::job_key(media, t)).cloned();
        if let Some(mut rx) = job {
            while !*rx.borrow_and_update() {
                if rx.changed().await.is_err() {
                    break;
                }
            }
        }
    }

    /// Starts the track's match unless it has an answer for this text (or one is running).
    pub fn start_sync(self: &Arc<Self>, media: &Arc<Media>, t: &SubtitleTrack, original: &str) {
        if !matchable(t) || !self.sync_enabled() {
            return;
        }
        let stamp = fnv(original);
        let known = self.entry(media, t).outcome.is_some_and(|o| {
            o.stamp == stamp
                && o.current(media)
                && match o.state.as_str() {
                    "failed" => subs::now_secs().saturating_sub(o.at) < 86_400,
                    _ => o.algorithm == ALGORITHM,
                }
        });
        if known {
            return;
        }
        let key = Self::job_key(media, t);
        let (tx, rx) = watch::channel(false);
        {
            let mut running = self.sync.running.lock().unwrap();
            if running.contains_key(&key) {
                return;
            }
            running.insert(key.clone(), rx);
        }
        let (this, media, track, original) = (self.clone(), media.clone(), t.clone(), original.to_string());
        tokio::spawn(async move {
            let t0 = std::time::Instant::now();
            let outcome = match this.sync_track(&media, &track, &original).await {
                Ok(o) => o,
                Err(e) => {
                    tracing::warn!(item = %media.item_id, track = track.index, "subtitle timing: {e:#}");
                    Outcome {
                        stamp: fnv(&original),
                        algorithm: ALGORITHM,
                        state: "failed".into(),
                        reference: None,
                        offset: 0.0,
                        scale: 1.0,
                        pieces: 1,
                        fit: None,
                        file: None,
                        at: subs::now_secs(),
                        timeline: media.clock_offset() > 0.0,
                    }
                }
            };
            tracing::info!(item = %media.item_id, track = track.index, state = outcome.state, reference = outcome.reference, offset = outcome.offset, scale = outcome.scale,
                pieces = outcome.pieces, after = outcome.fit.as_ref().map(|f| f.after), before = outcome.fit.as_ref().map(|f| f.before), chance = outcome.fit.as_ref().map(|f| f.chance),
                strength = outcome.fit.as_ref().map(|f| f.strength), secs = t0.elapsed().as_secs_f32(), "subtitle timing");
            if let Err(e) = this.update_entry(&media, &track, |e| e.outcome = Some(outcome)).await {
                tracing::warn!(item = %media.item_id, "subtitle timing not stored: {e:#}");
            }
            this.sync.running.lock().unwrap().remove(&key);
            let _ = tx.send(true);
        });
    }

    /// The match itself: against the file's own text tracks, else its audio.
    async fn sync_track(self: &Arc<Self>, media: &Arc<Media>, t: &SubtitleTrack, original: &str) -> Result<Outcome> {
        let cues = subs::parse_srt(original);
        let spoken = dialogue(&cues);
        let stamp = fnv(original);
        let done = |state: &str, reference: Option<&str>, fit: Option<Fit>| Outcome {
            stamp,
            algorithm: ALGORITHM,
            state: state.into(),
            reference: reference.map(String::from),
            offset: 0.0,
            scale: 1.0,
            pieces: 1,
            fit,
            file: None,
            at: subs::now_secs(),
            timeline: media.clock_offset() > 0.0,
        };
        if spoken.len() < MIN_COVERED_CUES {
            return Ok(done("unsure", None, None));
        }
        // A seam for testing: only this kind of reference.
        let only = std::env::var("MOONRISE_SUBSYNC_REFERENCE").unwrap_or_default();
        let mut last = None;
        if only != "audio" {
            if let Some((r, language)) = self.text_reference(media, spoken.len()).await {
                let same =
                    |a: Option<&str>, b: Option<&str>| a.zip(b).is_some_and(|(a, b)| subs::language_name(a) == subs::language_name(b));
                let basis = if same(t.language.as_deref(), language.as_deref()) {
                    Basis::Text
                } else {
                    Basis::OtherText
                };
                let (cues2, r2) = (spoken.clone(), r);
                let fit = tokio::task::spawn_blocking(move || align(&cues2, &r2)).await?;
                if let Some(fit) = fit {
                    match verdict(&fit, basis) {
                        Verdict::Apply => return self.store_match(media, t, stamp, &cues, &spoken, fit, "subtitles").await,
                        Verdict::InSync => return Ok(done("in_sync", Some("subtitles"), Some(fit))),
                        Verdict::Unsure => last = Some(done("unsure", Some("subtitles"), Some(fit))),
                    }
                }
            }
        }
        if only != "text" {
            let _turn = self.sync.audio.acquire().await?;
            if let Some(fit) = self.audio_fit(media, &spoken).await? {
                match verdict(&fit, Basis::Audio) {
                    Verdict::Apply => return self.store_match(media, t, stamp, &cues, &spoken, fit, "audio").await,
                    Verdict::InSync => return Ok(done("in_sync", Some("audio"), Some(fit))),
                    Verdict::Unsure => last = Some(done("unsure", Some("audio"), Some(fit))),
                }
            }
        }
        Ok(last.unwrap_or_else(|| done("unsure", None, None)))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the steps' inputs, passed through; a struct for one call would only rename them"
    )]
    async fn store_match(
        &self,
        media: &Media,
        t: &SubtitleTrack,
        stamp: u64,
        cues: &[Cue],
        spoken: &[Span],
        fit: Fit,
        reference: &str,
    ) -> Result<Outcome> {
        let dir = self.timing_dir(media);
        tokio::fs::create_dir_all(&dir).await?;
        let name = format!("sync-{:016x}.srt", fnv(&format!("{}|{}", track_key(t), media.file_key)));
        let srt = subs::normalize_srt(&subx::to_srt(&apply(cues, &fit)));
        subs::write_atomic(&dir.join(&name), srt.as_bytes()).await?;
        Ok(Outcome {
            stamp,
            algorithm: ALGORITHM,
            state: "synced".into(),
            reference: Some(reference.into()),
            offset: (fit.typical_offset(spoken) * 100.0).round() / 100.0,
            scale: fit.scale,
            pieces: fit.pieces.len(),
            fit: Some(fit),
            file: Some(name),
            at: subs::now_secs(),
            timeline: media.clock_offset() > 0.0,
        })
    }

    /// The file's fullest own text track that isn't forced or a commentary, extracted if needed,
    /// when it has about as many lines as the subtitle; and its language.
    async fn text_reference(self: &Arc<Self>, media: &Arc<Media>, lines: usize) -> Option<(Reference, Option<String>)> {
        if !self.enabled() || !subs::has_embedded_text(media) {
            return None;
        }
        if let Err(e) = self.ensure_extracted(media, subs::Demand::Background).await {
            tracing::info!(item = %media.item_id, "subtitle timing: no extracted tracks ({e:#})");
            return None;
        }
        let dir = self.extract_dir(media);
        let mut best: Option<(Vec<Span>, Option<String>)> = None;
        for t in media
            .subtitles
            .iter()
            .filter(|s| s.text && !s.forced && !s.name.to_lowercase().contains("comment"))
        {
            let Some(stream) = t.stream else { continue };
            let Ok(srt) = tokio::fs::read_to_string(dir.join(format!("s{stream}.srt"))).await else {
                continue;
            };
            let spans = own_track_dialogue(&srt, media.clock_offset());
            if best.as_ref().is_none_or(|b| spans.len() > b.0.len()) {
                best = Some((spans, t.language.clone()));
            }
        }
        let (best, language) = best.filter(|b| b.0.len() >= 30 && b.0.len() * 2 >= lines)?;
        Some((Reference::from_cues(&best, media.duration), language))
    }

    /// The fit against the main audio track: what is known of it, read further where needed.
    async fn audio_fit(&self, media: &Media, spoken: &[Span]) -> Result<Option<Fit>> {
        let Some(audio) = media.audio.iter().find(|a| a.default).or(media.audio.first()) else {
            return Ok(None);
        };
        let path = self.extract_dir(media).join("speech.json");
        let mut known: Speech = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .filter(|s: &Speech| s.stream == audio.stream && (s.timeline || media.clock_offset() == 0.0))
            .unwrap_or_default();
        known.stream = audio.stream;
        known.timeline = media.clock_offset() > 0.0;
        known.reference.end = media.duration;
        // A seam for testing: the whole track, whatever its size (what a test compares windows with).
        let whole = std::env::var("MOONRISE_SUBSYNC_WINDOWS").is_ok_and(|v| v == "whole");
        let plan = if whole {
            vec![(0.0, media.duration)]
        } else {
            plan_windows(media.duration, media.size)
        };
        let mut todo = if known.reference.coverage.is_empty() { plan } else { Vec::new() };
        let mut extra = 0;
        loop {
            let mut read_any = false;
            for w in std::mem::take(&mut todo) {
                match self.read_speech(media, audio.stream, audio.channels, w).await {
                    Ok(runs) => {
                        known.reference.speech.extend(runs);
                        known.reference.coverage.push(w);
                        read_any = true;
                    }
                    Err(e) => tracing::info!(item = %media.item_id, from = w.0, "subtitle timing: audio window: {e:#}"),
                }
            }
            if read_any {
                known.reference.speech = merge(std::mem::take(&mut known.reference.speech));
                known.reference.coverage = merge(std::mem::take(&mut known.reference.coverage));
                if let Some(d) = path.parent() {
                    tokio::fs::create_dir_all(d).await?;
                }
                subs::write_atomic(&path, &serde_json::to_vec(&known)?).await?;
            }
            anyhow::ensure!(!known.reference.coverage.is_empty(), "no audio could be read");
            let (c, r) = (spoken.to_vec(), known.reference.clone());
            let Some(fit) = tokio::task::spawn_blocking(move || align(&c, &r)).await? else {
                return Ok(None);
            };
            todo = next_windows(&fit, &known.reference.coverage, media.duration, extra);
            if todo.is_empty() {
                return Ok(Some(fit));
            }
            extra += todo.len();
        }
    }

    /// Speech in [from, to] of the file's time. Reads through ffmpeg, in the background: it holds
    /// still while a stream segment or a subtitle window is read.
    async fn read_speech(&self, media: &Media, stream: i64, channels: i64, (from, to): Span) -> Result<Vec<Span>> {
        for center in if channels >= 6 { &[true, false][..] } else { &[false][..] } {
            match self.read_levels(media, stream, *center, (from, to)).await {
                Ok(db) if !db.is_empty() => {
                    return Ok(speech_runs(&db).into_iter().map(|(a, b)| (a + from, (b + from).min(to))).collect());
                }
                Ok(_) => tracing::info!(item = %media.item_id, from, "subtitle timing: no audio read"),
                Err(e) if *center => tracing::info!(item = %media.item_id, "subtitle timing: no centre channel ({e:#})"),
                Err(e) => return Err(e),
            }
        }
        anyhow::bail!("no audio in {from:.0}..{to:.0}")
    }

    async fn read_levels(&self, media: &Media, stream: i64, center: bool, (from, to): Span) -> Result<Vec<f32>> {
        // Read with no data for this long: give up (paused time doesn't count).
        const QUIET: Duration = Duration::from_secs(120);
        const TICK: Duration = Duration::from_millis(100);
        let args = levels_args(media, stream, center, (from, to));
        let mut child = tokio::process::Command::new("nice")
            .args(["-n", "15", &self.ffmpeg])
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("ffmpeg")?;
        let pid = child.id();
        let mut stdout = child.stdout.take().context("ffmpeg stdout")?;
        let mut stderr = child.stderr.take().context("ffmpeg stderr")?;
        let errors = tokio::spawn(async move {
            let mut s = Vec::new();
            let _ = stderr.read_to_end(&mut s).await;
            String::from_utf8_lossy(&s).lines().take(3).collect::<Vec<_>>().join(" / ")
        });
        let mut levels = Levels::default();
        let mut buf = vec![0u8; 1 << 16];
        let mut stopped = false;
        let mut quiet = Duration::ZERO;
        // One ticker for the whole read: a sleep made anew for each read of stdout would never
        // finish while data keeps coming, and the pause would wait for a lull.
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                n = stdout.read(&mut buf) => {
                    let n = n.context("read audio")?;
                    if n == 0 {
                        break;
                    }
                    levels.feed(&buf[..n]);
                    quiet = Duration::ZERO;
                }
                _ = tick.tick() => {
                    let go = subs::STREAM_READS.load(Ordering::SeqCst) == 0 && subs::WINDOW_READS.load(Ordering::SeqCst) == 0;
                    if let Some(pid) = pid {
                        if go == stopped {
                            // SAFETY: kill only takes integers. `pid` is our ffmpeg child, which isn't reaped before
                            // `child.wait()` below, so the id is still its.
                            unsafe { libc::kill(pid as i32, if go { libc::SIGCONT } else { libc::SIGSTOP }); }
                            stopped = !go;
                        }
                    }
                    if go {
                        quiet += TICK;
                        anyhow::ensure!(quiet < QUIET, "ffmpeg stopped reading");
                    }
                }
            }
        }
        let status = child.wait().await.context("ffmpeg")?;
        let err = crate::admin::logbuf::scrub(&errors.await.unwrap_or_default()).into_owned();
        anyhow::ensure!(status.success(), "ffmpeg: {err}");
        Ok(levels.db)
    }
}

/// The dialogue of a track extracted from the file (its own timestamps), on the title's timeline
/// as the track matched is: `offset` earlier (`Media::clock_offset`).
fn own_track_dialogue(srt: &str, offset: f64) -> Vec<Span> {
    let spans = dialogue(&subs::parse_srt(srt));
    if offset == 0.0 {
        return spans;
    }
    spans.into_iter().map(|(a, b)| (a - offset, b - offset)).collect()
}

/// ffmpeg's arguments for `read_levels`: the audio track `stream` in [from, to] of the title's time
/// as 16-bit mono samples at `RATE`.
fn levels_args(media: &Media, stream: i64, center: bool, (from, to): Span) -> Vec<String> {
    let mut args: Vec<String> = ["-nostdin", "-hide_banner", "-v", "error"].map(String::from).to_vec();
    if from > 0.0 {
        args.extend(["-ss".into(), format!("{from:.3}")]);
    }
    args.extend(["-t".into(), format!("{:.3}", to - from)]);
    args.extend(media.source.input_args());
    // Dialogue: the centre channel of a surround track, in the voice band. Samples count from
    // `from` of the title's time (as the cues do): an accurate seek there, or from the start
    // with silence for a track that starts late. Gaps in the track are filled. The title's
    // time is the file's own, but for a file with a start time (`Media::clock_offset`), whose
    // time counts from that start: ffmpeg's, without `-copyts`.
    let mix = if center { "pan=mono|c0=FC" } else { "aformat=channel_layouts=mono" };
    let resample = if from > 0.0 {
        format!("aresample={RATE}:async=1")
    } else {
        format!("aresample={RATE}:async=1:first_pts=0")
    };
    if from <= 0.0 && media.clock_offset() == 0.0 {
        args.push("-copyts".into());
    }
    let filter = format!("{mix},highpass=f=200,lowpass=f=3500,{resample}");
    args.extend([
        "-map".into(),
        format!("0:{stream}"),
        "-af".into(),
        filter,
        "-f".into(),
        "s16le".into(),
        "-c:a".into(),
        "pcm_s16le".into(),
        "pipe:1".into(),
    ]);
    args
}

#[cfg(test)]
#[path = "subsync_eval.rs"]
mod eval;

#[cfg(test)]
mod tests {
    use super::*;

    /// A believable film: lines of 1–4 s with pauses, from a small seeded generator.
    fn film(seed: u64, minutes: f64) -> Vec<Span> {
        let mut x = seed.wrapping_mul(2862933555777941757).wrapping_add(3037000493);
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % 1000) as f64 / 1000.0
        };
        let mut out = Vec::new();
        let mut t = 5.0 + next() * 20.0;
        while t < minutes * 60.0 {
            let len = 1.0 + next() * 3.0;
            out.push((t, t + len));
            t += len + 0.2 + next() * next() * 18.0;
        }
        out
    }

    fn map(cues: &[Span], f: impl Fn(f64) -> f64) -> Vec<Span> {
        cues.iter().map(|c| (f(c.0), f(c.0) + c.1 - c.0)).collect()
    }

    fn text_ref(cues: &[Span]) -> Reference {
        let end = cues.last().map_or(0.0, |c| c.1) + 60.0;
        Reference::from_cues(cues, end)
    }

    /// The reference's own lines, cut differently: the same speech in another language's track.
    fn resegment(cues: &[Span]) -> Vec<Span> {
        cues.iter()
            .enumerate()
            .filter(|(i, _)| i % 11 != 0)
            .map(|(i, c)| (c.0 + 0.1 * (i % 3) as f64, c.1 - 0.1 * (i % 2) as f64))
            .collect()
    }

    #[test]
    fn finds_a_plain_offset() {
        let video = film(1, 100.0);
        let sub = map(&resegment(&video), |t| t - 7.3);
        let fit = align(&sub, &text_ref(&video)).unwrap();
        assert_eq!(fit.scale, 1.0);
        assert_eq!(fit.pieces.len(), 1, "{fit:?}");
        assert!((fit.pieces[0].offset - 7.3).abs() < 0.12, "{fit:?}");
        assert!(fit.after > 0.8 && fit.before < 0.5, "{fit:?}");
        assert_eq!(verdict(&fit, Basis::Text), Verdict::Apply, "{fit:?}");
    }

    #[test]
    fn finds_every_frame_rate_conversion() {
        let video = film(2, 100.0);
        for rate in [
            25.0 / 24.0,
            24.0 / 25.0,
            25.0 * 1001.0 / 24000.0,
            24000.0 / 1001.0 / 25.0,
            1001.0 / 1000.0,
            1000.0 / 1001.0,
        ] {
            // Subtitles for a release at another speed, plus an offset.
            let sub: Vec<Span> = resegment(&video).iter().map(|c| ((c.0 - 3.0) * rate, (c.1 - 3.0) * rate)).collect();
            let fit = align(&sub, &text_ref(&video)).unwrap();
            assert!((fit.scale - 1.0 / rate).abs() < 1e-3, "rate {rate}: {fit:?}");
            assert_eq!(fit.pieces.len(), 1, "rate {rate}: {fit:?}");
            assert_eq!(verdict(&fit, Basis::Text), Verdict::Apply, "rate {rate}: {fit:?}");
        }
    }

    #[test]
    fn finds_a_cut() {
        let video = film(3, 100.0);
        // The subtitle's release has 94 s more at minute 40 (a recap), and starts 2 s earlier.
        let sub = map(&resegment(&video), |t| if t < 2400.0 { t - 2.0 } else { t - 2.0 + 94.0 });
        let fit = align(&sub, &text_ref(&video)).unwrap();
        assert_eq!(fit.scale, 1.0);
        assert_eq!(fit.pieces.len(), 2, "{fit:?}");
        assert!((fit.pieces[0].offset - 2.0).abs() < 0.12, "{fit:?}");
        assert!((fit.pieces[1].offset - (2.0 - 94.0)).abs() < 0.12, "{fit:?}");
        // The split falls between the last line before the cut and the first after it.
        let until = fit.pieces[0].until;
        assert!((until - 2398.0).abs() < 100.0, "{fit:?}");
        assert_eq!(verdict(&fit, Basis::Text), Verdict::Apply, "{fit:?}");
    }

    #[test]
    fn leaves_a_synced_subtitle_alone() {
        let video = film(4, 100.0);
        let fit = align(&resegment(&video), &text_ref(&video)).unwrap();
        assert_eq!(verdict(&fit, Basis::Text), Verdict::InSync, "{fit:?}");
    }

    /// Speech as a detector hears it: each line smeared by `smear` either side (so the overlap
    /// changes slowly with the offset), a tenth missed, and noise from another film.
    fn heard(video: &[Span], smear: f64, noise_seed: u64) -> Vec<Span> {
        let mut speech: Vec<Span> = video
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 10 != 3)
            .map(|(_, c)| (c.0 - smear, c.1 + smear))
            .collect();
        speech.extend(film(noise_seed, 100.0).into_iter().step_by(6));
        merge(speech)
    }

    #[test]
    fn fixes_a_small_offset_found_clearly() {
        // 1 to 3 s off: little of the cue time moves off speech that is heard smeared, yet the
        // peak is clear. Such a subtitle is out of sync, not "in sync".
        let video = film(16, 45.0);
        let r = Reference {
            speech: heard(&video, 0.4, 17),
            coverage: vec![(0.0, 2700.0)],
            end: 2700.0,
        };
        for off in [1.0, -1.5, 2.0, 3.0] {
            let sub = map(&video, |t| t - off);
            let fit = align(&sub, &r).unwrap();
            assert_eq!(verdict(&fit, Basis::Audio), Verdict::Apply, "{off}: {fit:?}");
            assert!((fit.map(sub[0].0) - video[0].0).abs() < 0.5, "{off}: {fit:?}");
        }
        let fit = align(&video, &r).unwrap();
        assert_eq!(verdict(&fit, Basis::Audio), Verdict::InSync, "{fit:?}");
    }

    #[test]
    fn no_split_for_lines_the_detector_misses() {
        // A recap under music: the detector hears none of its 45 lines, only noise. The lines
        // must stay where they are, not move to wherever the noise suits them best.
        let video = film(18, 45.0);
        let mut recap: Vec<Span> = (0..45).map(|i| (1.0 + i as f64, 1.8 + i as f64)).collect();
        recap.extend(video.iter().map(|c| (c.0 + 50.0, c.1 + 50.0)));
        let mut speech: Vec<Span> = heard(&video, 0.3, 19).into_iter().map(|c| (c.0 + 50.0, c.1 + 50.0)).collect();
        speech.extend(
            film(20, 1.0)
                .into_iter()
                .map(|c| (c.0 * 0.8, c.1 * 0.8 - 0.5))
                .filter(|c| c.1 < 49.0),
        );
        let r = Reference {
            speech: merge(speech),
            coverage: vec![(0.0, 2760.0)],
            end: 2760.0,
        };
        let fit = align(&recap, &r).unwrap();
        assert_eq!(fit.pieces.len(), 1, "{fit:?}");
        assert_eq!(verdict(&fit, Basis::Audio), Verdict::InSync, "{fit:?}");
    }

    #[test]
    fn refuses_another_title() {
        let video = film(5, 45.0);
        for seed in 6..12 {
            let other = film(seed, 45.0);
            let fit = align(&other, &text_ref(&video)).unwrap();
            assert_eq!(verdict(&fit, Basis::Text), Verdict::Unsure, "seed {seed}: {fit:?}");
        }
    }

    #[test]
    fn refuses_too_few_lines() {
        let video = film(13, 100.0);
        let few: Vec<Span> = video.iter().take(6).copied().collect();
        let fit = align(&map(&few, |t| t + 4.0), &text_ref(&video)).unwrap();
        assert_eq!(verdict(&fit, Basis::Text), Verdict::Unsure, "{fit:?}");
    }

    #[test]
    fn a_cut_in_a_sampled_film_never_moves_the_part_that_was_right() {
        // Speech known in 18 windows of a film; the subtitle's release has a minute more 40%
        // in. Its first part is right as it is: a match may fix the rest or give no answer, but
        // never take the first part along with the rest.
        for seed in [31, 33, 35, 37] {
            let video = film(seed, 110.0);
            let windows = plan_windows(6600.0, Some(16_000_000_000));
            let inside = |s: &Span| windows.iter().any(|w| s.0 < w.1 && s.1 > w.0);
            let r = Reference {
                speech: merge(heard(&video, 0.4, seed + 1).into_iter().filter(inside).collect()),
                coverage: windows.clone(),
                end: 6600.0,
            };
            let sub = map(&video, |t| if t < 2640.0 { t } else { t + 60.0 });
            let fit = align(&sub, &r).unwrap();
            if verdict(&fit, Basis::Audio) == Verdict::Apply {
                assert!((fit.map(sub[0].0) - video[0].0).abs() < 1.0, "{seed}: {fit:?}");
                let last = video.len() - 1;
                assert!((fit.map(sub[last].0) - video[last].0).abs() < 1.0, "{seed}: {fit:?}");
            }
        }
    }

    #[test]
    fn no_part_is_moved_where_nothing_says_it_belongs() {
        // Speech known only for the second half (as when audio was read in places): the first
        // half's lines, 60 s off from the rest, can't be placed. Taking them along with the rest
        // could put a minute out a large part of a track that was right: no answer instead.
        let video = film(23, 90.0);
        let r = Reference {
            speech: merge(heard(&video, 0.3, 24).into_iter().filter(|s| s.0 >= 2700.0).collect()),
            coverage: vec![(2700.0, 5400.0)],
            end: 5400.0,
        };
        let sub = map(&video, |t| if t < 2700.0 { t } else { t + 60.0 });
        let fit = align(&sub, &r).unwrap();
        assert!(!fit.unresolved.is_empty(), "{fit:?}");
        assert_eq!(verdict(&fit, Basis::Audio), Verdict::Unsure, "{fit:?}");
    }

    #[test]
    fn another_language_is_in_sync_a_little_apart() {
        // Tracks in two languages for one file start their lines a little apart: the subtitle
        // isn't moved by that much.
        let video = film(25, 100.0);
        let fit = align(&map(&resegment(&video), |t| t + 0.4), &text_ref(&video)).unwrap();
        assert_eq!(verdict(&fit, Basis::OtherText), Verdict::InSync, "{fit:?}");
        assert_eq!(verdict(&fit, Basis::Text), Verdict::Apply, "{fit:?}");
    }

    #[test]
    fn sampled_audio_is_enough() {
        // Speech known in 13 windows of a film, to near its end (where `next_windows` reads once
        // lines are moved there); a sloppy detector: a tenth of it missed, and noise.
        let video = film(14, 110.0);
        let windows: Vec<Span> = (0..13).map(|i| (300.0 + i as f64 * 500.0, 360.0 + i as f64 * 500.0)).collect();
        let mut speech: Vec<Span> = video.iter().enumerate().filter(|(i, _)| i % 10 != 0).map(|(_, c)| *c).collect();
        speech.extend(film(15, 110.0).into_iter().step_by(5));
        let inside = |s: &Span| windows.iter().any(|w| s.0 < w.1 && s.1 > w.0);
        let r = Reference {
            speech: merge(speech.into_iter().filter(inside).collect()),
            coverage: windows.clone(),
            end: 110.0 * 60.0,
        };
        let sub = map(&video, |t| t * 25.0 / 24.0 + 1.5);
        let fit = align(&sub, &r).unwrap();
        assert!((fit.scale - 24.0 / 25.0).abs() < 1e-6, "{fit:?}");
        assert!((fit.map(sub[0].0) - video[0].0).abs() < 0.2, "{fit:?}");
        assert_eq!(verdict(&fit, Basis::Audio), Verdict::Apply, "{fit:?}");
    }

    #[test]
    fn applying_moves_every_cue_and_keeps_the_text() {
        let fit = Fit {
            scale: 1.0,
            pieces: vec![
                Piece { until: 100.0, offset: 2.0 },
                Piece {
                    until: f64::MAX,
                    offset: -50.0,
                },
            ],
            after: 1.0,
            before: 0.0,
            chance: 0.3,
            strength: 10.0,
            gain: 10.0,
            covered_secs: 100.0,
            covered_cues: 50,
            unresolved: Vec::new(),
            support: 10.0,
        };
        let cue = |s: f64, e: f64, t: &str| Cue {
            start: s,
            end: e,
            text: t.into(),
        };
        let out = apply(
            &[
                cue(-3.0, -2.5, "gone"),
                cue(10.0, 12.0, "a"),
                cue(99.0, 103.0, "b"),
                cue(149.0, 151.0, "c"),
                cue(160.0, 161.0, "d"),
            ],
            &fit,
        );
        let texts: Vec<&str> = out.iter().map(|c| c.text.as_str()).collect();
        // "b" moves to 101..105, "c" to 99..101, "d" to 110..111: in the order they now show.
        assert_eq!(texts, ["a", "c", "b", "d"]);
        assert_eq!((out[0].start, out[0].end), (12.0, 14.0));
        assert!(out.windows(2).all(|w| w[0].start <= w[1].start));
    }

    #[test]
    fn a_stored_fit_reads_back() {
        // The last piece's end is the largest number there is: JSON has no infinity, and a store
        // that doesn't parse would lose every nudge of the file.
        let fit = Fit {
            scale: 0.96,
            pieces: vec![Piece {
                until: f64::MAX,
                offset: -1.5,
            }],
            after: 0.6,
            before: 0.4,
            chance: 0.4,
            strength: 9.0,
            gain: 9.0,
            covered_secs: 500.0,
            covered_cues: 200,
            unresolved: Vec::new(),
            support: 10.0,
        };
        let entry = Entry {
            file_key: "k".into(),
            nudge: 0.3,
            original: false,
            outcome: Some(Outcome {
                stamp: u64::MAX,
                algorithm: ALGORITHM,
                state: "synced".into(),
                reference: Some("audio".into()),
                offset: -1.5,
                scale: 0.96,
                pieces: 1,
                fit: Some(fit),
                file: Some("sync.srt".into()),
                at: 1,
                timeline: false,
            }),
        };
        let back: Entry = serde_json::from_slice(&serde_json::to_vec(&entry).unwrap()).unwrap();
        let o = back.outcome.unwrap();
        assert_eq!(o.stamp, u64::MAX);
        assert_eq!(back.nudge, 0.3);
        assert!((o.fit.unwrap().map(100.0) - 94.5).abs() < 1e-9);
    }

    #[test]
    fn a_file_with_a_start_time_is_matched_on_its_timeline() {
        // A TV recording keeps its broadcast clock: it starts at 1002.597 s, which players count
        // as 0, and a downloaded subtitle is timed from there. Before 0.31.11 the references were
        // on the file's clock: the file's own track 1002.6 s late (an in-sync subtitle was moved
        // 1002.6 s, past the end), and the audio read from 0 with `-copyts` 1002.6 s late too.
        let mut m = crate::media::transcode::tests::media(crate::media::transcode::tests::video("h264", 1920, 1080, 4_000_000, "SDR"));
        let srt = "1\n00:16:45,000 --> 00:16:47,200\nSo where were you last night?\n";
        (m.start_time, m.origin) = (1002.597, 1002.597);
        assert_eq!(m.clock_offset(), 1002.597);
        let spans = own_track_dialogue(srt, m.clock_offset());
        assert!((spans[0].0 - 2.403).abs() < 1e-6 && (spans[0].1 - 4.603).abs() < 1e-6, "{spans:?}");
        let args = levels_args(&m, 1, false, (0.0, 600.0));
        assert!(!args.contains(&"-copyts".to_string()), "{args:?}");
        assert!(args.iter().any(|a| a.ends_with("first_pts=0")));
        // A window further in seeks on the title's time (ffmpeg adds the start time to `-ss`).
        let args = levels_args(&m, 1, false, (600.0, 900.0));
        assert_eq!(args[4..6], ["-ss".to_string(), "600.000".to_string()]);
        // Answers found on the file's clock are found again; others are kept.
        let mut o = Outcome {
            stamp: 1,
            algorithm: ALGORITHM,
            state: "synced".into(),
            reference: Some("subtitles".into()),
            offset: 1002.6,
            scale: 1.0,
            pieces: 1,
            fit: None,
            file: Some("sync.srt".into()),
            at: 1,
            timeline: false,
        };
        assert!(!o.current(&m));
        o.timeline = true;
        assert!(o.current(&m));
        // Files starting at 0, or a frame later, as before.
        for start in [0.0, 0.08] {
            (m.start_time, m.origin) = (start, start);
            assert_eq!(m.clock_offset(), 0.0);
            assert_eq!(own_track_dialogue(srt, m.clock_offset())[0], (1005.0, 1007.2));
            assert!(levels_args(&m, 1, false, (0.0, 600.0)).contains(&"-copyts".to_string()));
            o.timeline = false;
            assert!(o.current(&m));
            assert!(
                !serde_json::to_string(&o).unwrap().contains("timeline"),
                "the stored answer is as before"
            );
        }
    }

    #[test]
    fn only_downloaded_tracks_are_matched() {
        let track = |stream: Option<i64>, path: Option<&str>, file: Option<&str>| SubtitleTrack {
            index: 0,
            stream,
            text: true,
            codec: "subrip".into(),
            name: String::new(),
            language: None,
            forced: false,
            default: false,
            external: path.is_some(),
            source: "",
            url: None,
            path: path.map(String::from),
            file: file.map(PathBuf::from),
            timing: None,
        };
        assert!(!matchable(&track(Some(3), None, None)), "embedded");
        assert!(!matchable(&track(None, Some("smb://nas/Film.en.srt"), None)), "sidecar");
        assert!(
            matchable(&track(None, None, Some("/data/subs/f-1/os-1.srt"))),
            "fetched or uploaded"
        );
    }

    #[test]
    fn dialogue_only() {
        let c = |t: &str| Cue {
            start: 1.0,
            end: 3.0,
            text: t.into(),
        };
        assert_eq!(dialogue(&[c("Hello there")]).len(), 1);
        assert_eq!(dialogue(&[c("<i>Привет</i>")]).len(), 1);
        assert_eq!(dialogue(&[c("[door creaks]")]).len(), 0);
        assert_eq!(dialogue(&[c("♪ la la la ♪")]).len(), 0);
        assert_eq!(dialogue(&[c("Support us at www.example.com")]).len(), 0);
        assert_eq!(dialogue(&[c("- ...")]).len(), 0);
        assert_eq!(
            dialogue(&[Cue {
                start: 1.0,
                end: 40.0,
                text: "credits".into()
            }])
            .len(),
            0
        );
    }

    #[test]
    fn speech_from_levels() {
        // 40 s: quiet at -60 dB, with 2 s lines at -20 dB wobbling like syllables, and music at
        // an even -25 dB from 20 s to 30 s.
        let frames = (40.0 / HOP) as usize;
        let mut db = vec![-60.0f32; frames];
        for s in [2.0, 6.0, 9.0, 13.0] {
            for i in 0..(2.0 / HOP) as usize {
                db[(s / HOP) as usize + i] = if i % 12 < 8 { -20.0 } else { -32.0 };
            }
        }
        for x in db.iter_mut().take((30.0 / HOP) as usize).skip((20.0 / HOP) as usize) {
            *x = -25.0;
        }
        let runs = speech_runs(&db);
        for line in [2.0, 6.0, 9.0, 13.0] {
            assert!(runs.iter().any(|r| r.0 <= line + 0.3 && r.1 >= line + 1.7), "{line}: {runs:?}");
        }
        assert!(!runs.iter().any(|r| r.0 > 21.0 && r.1 < 29.0), "{runs:?}");
    }

    #[test]
    fn levels_from_bytes_in_any_chunks() {
        let frame = (RATE as f64 * HOP) as usize;
        let bytes: Vec<u8> = (0..frame * 3).flat_map(|_| 16384i16.to_le_bytes()).collect();
        let mut l = Levels::default();
        for c in bytes.chunks(7) {
            l.feed(c);
        }
        assert_eq!(l.db.len(), 3);
        assert!((l.db[0] + 6.02).abs() < 0.1, "{:?}", l.db);
    }

    #[test]
    fn plan_reads_small_files_whole() {
        assert_eq!(plan_windows(1320.0, Some(400_000_000)), vec![(0.0, 1320.0)]);
        let big = plan_windows(7200.0, Some(40_000_000_000));
        assert_eq!(big.len(), MIN_WINDOWS);
        assert!(big[0].0 > 0.0 && big.last().unwrap().1 < 7200.0 * 0.95);
        let mid = plan_windows(7200.0, Some(8_000_000_000));
        assert!(mid.len() > MIN_WINDOWS && mid.len() <= MAX_WINDOWS, "{}", mid.len());
    }
}
