//! How the matcher does on real titles: an ignored test, run by hand.
//!
//! `MOONRISE_SUBSYNC_EVAL=<dir> cargo test --release eval -- --ignored --nocapture`
//!
//! `<dir>` holds a folder per file: `meta.json` (`name`, `duration`, `size`, `truth`: the stream
//! of a track timed for this file, `texts`: `[stream, language, forced, title, cues]` of its text
//! tracks), `speech.json` (what `audio_fit` keeps, read with `MOONRISE_SUBSYNC_WINDOWS=whole`) and
//! `s<stream>.srt` of its text tracks. Each file's true track is changed the ways other releases
//! differ (offsets, speeds, cuts), or swapped for another title's, and matched against the
//! file's audio, read in the windows a real run would read (the whole speech cut down to them),
//! and against its text tracks in other languages. Every trial is compared cue by cue with
//! the true timing; `MOONRISE_SUBSYNC_EVAL_OUT` gets a JSON line per trial.
//!
//! What counts:
//! - **fixed**: after the match (or without one, if it kept the original) at least 90% of the
//!   lines are within 0.75 s of the true timing (1.0 s when a cut is involved).
//! - **harm**: at least 10% of the lines end up more than 1.5 s further from the true timing
//!   than the original had them. A match must never do this.
//! - **false in sync**: labelled in sync while at least 10% of the lines are over 1.5 s off.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};

use serde_json::{json, Value};

use super::*;

struct Title {
    name: String,
    dir: PathBuf,
    duration: f64,
    size: Option<u64>,
    /// Spoken lines of the true track.
    truth: Vec<Span>,
    /// The whole audio's speech.
    full: Reference,
    /// Other-language text tracks: language and spoken lines.
    others: Vec<(String, Vec<Span>)>,
    show: Option<String>,
    /// Where the true track's lines sit against the speech heard (the peak of their correlation
    /// within 3 s): a track timed for this file can still lag its audio a little. Audio trials
    /// are judged against the true timing moved by this.
    bias: f64,
}

fn bias(truth: &[Span], r: &Reference) -> f64 {
    let density = r.density();
    let (on, covered) = weights(density);
    let mut bins = Bins::new(-3.0, 3.0, 0.01);
    for &c in truth {
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
    bins.lo + near[near.len() / 2] as f64 * 0.01
}

fn load(dir: &Path) -> Option<Title> {
    let meta: Value = serde_json::from_slice(&std::fs::read(dir.join("meta.json")).ok()?).ok()?;
    let speech: Speech = serde_json::from_slice(&std::fs::read(dir.join("speech.json")).ok()?).ok()?;
    let cues = |stream: i64| {
        std::fs::read_to_string(dir.join(format!("s{stream}.srt")))
            .ok()
            .map(|s| dialogue(&subs::parse_srt(&s)))
    };
    let truth = cues(meta["truth"].as_i64()?)?;
    let mut others = Vec::new();
    for t in meta["texts"].as_array()? {
        let (stream, lang, forced) = (t[0].as_i64()?, t[1].as_str().unwrap_or(""), t[2].as_i64().unwrap_or(0));
        if lang != "eng" && forced == 0 && t[4].as_i64().unwrap_or(0) >= 200 {
            if let Some(c) = cues(stream).filter(|c| c.len() >= 150) {
                others.push((lang.to_string(), c));
            }
        }
    }
    let name = meta["name"].as_str()?.to_string();
    let show = name
        .rsplit_once(" S")
        .filter(|(_, e)| e.contains('E') && e.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|(s, _)| s.to_string());
    let duration = meta["duration"].as_f64()?;
    let mut full = speech.reference;
    full.end = duration;
    let bias = bias(&truth, &full);
    Some(Title {
        name,
        dir: dir.to_path_buf(),
        duration,
        size: meta["size"].as_u64(),
        truth,
        full,
        others,
        show,
        bias,
    })
}

/// A change another release makes: each true line's start `t` becomes `Some(subtitle time)`, or
/// `None` where that release lacks the scene.
#[derive(Clone, Debug)]
enum Case {
    Same,
    Shift(f64),
    /// Another speed (`subtitle = rate × t + shift`).
    Rate(f64, f64),
    /// The subtitle's release has `secs` more at `at` (a share of the film) than the file, or
    /// (negative) lacks that much there; plus an offset throughout.
    Cut {
        at: f64,
        secs: f64,
        shift: f64,
    },
    /// Two of them.
    Cuts {
        at: [f64; 2],
        secs: [f64; 2],
    },
    /// Another title's track.
    Wrong(usize),
}

impl Case {
    fn label(&self, titles: &[Title]) -> String {
        match self {
            Case::Same => "same".into(),
            Case::Shift(s) => format!("shift{s:+}"),
            Case::Rate(r, s) => format!("rate{r:.5}{s:+}"),
            Case::Cut { at, secs, shift } => format!(
                "cut{secs:+}@{at}{}",
                if *shift == 0.0 { String::new() } else { format!("{shift:+}") }
            ),
            Case::Cuts { at, secs } => format!("cuts{:+}@{},{:+}@{}", secs[0], at[0], secs[1], at[1]),
            Case::Wrong(i) => format!("wrong:{}", titles[*i].name),
        }
    }

    fn is_cut(&self) -> bool {
        matches!(self, Case::Cut { .. } | Case::Cuts { .. })
    }

    /// The subtitle's lines, each with its true start (none for another title's).
    fn apply(&self, t: &Title, titles: &[Title]) -> Vec<(Span, Option<f64>)> {
        // Every cut before a line moves it; a line inside a scene the release lacks is gone.
        let cut = |cuts: &[(f64, f64)], x: f64| -> Option<f64> {
            let mut moved = 0.0;
            for &(at, secs) in cuts {
                let from = at * t.duration;
                if x >= from {
                    if secs < 0.0 && x < from - secs {
                        return None;
                    }
                    moved += secs;
                }
            }
            Some(x + moved)
        };
        let f = |x: f64| -> Option<f64> {
            match self {
                Case::Same => Some(x),
                Case::Shift(s) => Some(x + s),
                Case::Rate(r, s) => Some(x * r + s),
                Case::Cut { at, secs, shift } => cut(&[(*at, *secs)], x).map(|v| v + shift),
                Case::Cuts { at, secs } => cut(&[(at[0], secs[0]), (at[1], secs[1])], x),
                Case::Wrong(_) => unreachable!(),
            }
        };
        if let Case::Wrong(i) = self {
            return titles[*i].truth.iter().map(|c| (*c, None)).collect();
        }
        let rate = if let Case::Rate(r, _) = self { *r } else { 1.0 };
        t.truth
            .iter()
            .filter_map(|c| f(c.0).map(|s| ((s, s + (c.1 - c.0) * rate), Some(c.0))))
            .collect()
    }
}

/// The audio a real run would read, cut down from the whole: the fit and the windows read.
fn audio_run(cues: &[Span], t: &Title) -> (Option<Fit>, usize) {
    let plan = plan_windows(t.duration, t.size);
    if plan == vec![(0.0, t.duration)] {
        return (align(cues, &t.full), 0);
    }
    let read = |w: Span| -> Vec<Span> {
        t.full
            .speech
            .iter()
            .filter(|s| s.1 > w.0 && s.0 < w.1)
            .map(|s| (s.0.max(w.0), s.1.min(w.1)))
            .collect()
    };
    let mut known = Reference {
        speech: Vec::new(),
        coverage: Vec::new(),
        end: t.duration,
    };
    let (mut todo, mut extra, mut windows) = (plan, 0, 0);
    loop {
        for w in std::mem::take(&mut todo) {
            known.speech.extend(read(w));
            known.coverage.push(w);
            windows += 1;
        }
        known.speech = merge(std::mem::take(&mut known.speech));
        known.coverage = merge(std::mem::take(&mut known.coverage));
        let Some(fit) = align(cues, &known) else { return (None, windows) };
        todo = next_windows(&fit, &known.coverage, t.duration, extra);
        if todo.is_empty() {
            return (Some(fit), windows);
        }
        extra += todo.len();
    }
}

fn trial(t: &Title, titles: &[Title], case: &Case, reference: &str) -> Value {
    let lines = case.apply(t, titles);
    let mut cues: Vec<Span> = lines.iter().map(|l| l.0).collect();
    cues.sort_by(|a, b| a.0.total_cmp(&b.0));
    let audio = reference == "audio";
    let (fit, windows) = if audio {
        audio_run(&cues, t)
    } else {
        let other = t.others.iter().find(|o| o.0 == reference).map(|o| o.1.clone()).unwrap_or_default();
        (align(&cues, &Reference::from_cues(&other, t.duration)), 0)
    };
    let v = fit
        .as_ref()
        .map_or(Verdict::Unsure, |f| verdict(f, if audio { Basis::Audio } else { Basis::OtherText }));
    let state = match v {
        Verdict::Apply => "synced",
        Verdict::InSync => "in_sync",
        Verdict::Unsure => "unsure",
    };
    // Cue by cue, against the true timing.
    let (mut ok, mut bad_before, mut bad_after, mut harm, mut n) = (0, 0, 0, 0, 0);
    let tol = if case.is_cut() { 1.0 } else { 0.75 };
    for (c, truth) in &lines {
        let Some(truth) = truth.map(|x| if audio { x + t.bias } else { x }) else {
            continue;
        };
        let after = match (&fit, v) {
            (Some(f), Verdict::Apply) => f.map(c.0),
            _ => c.0,
        };
        let (eb, ea) = ((c.0 - truth).abs(), (after - truth).abs());
        n += 1;
        ok += (ea <= tol) as usize;
        bad_before += (eb > 1.5) as usize;
        bad_after += (ea > 1.5) as usize;
        harm += (ea > eb + 1.5) as usize;
    }
    let share = |x: usize| if n > 0 { x as f64 / n as f64 } else { 0.0 };
    let wrong = matches!(case, Case::Wrong(_));
    let outcome = if wrong {
        if v == Verdict::Unsure {
            "refused"
        } else {
            "HARM"
        }
    } else if share(harm) >= 0.1 {
        "HARM"
    } else if v == Verdict::InSync && share(bad_before) >= 0.1 {
        "false_in_sync"
    } else if share(ok) >= 0.9 {
        "fixed"
    } else if v == Verdict::Unsure {
        "unsure"
    } else {
        "partly"
    };
    json!({
        "title": t.name, "case": case.label(titles), "reference": reference, "state": state, "outcome": outcome,
        "ok": share(ok), "bad_before": share(bad_before), "bad_after": share(bad_after), "harm": share(harm),
        "windows": windows, "bias": t.bias,
        "scale": fit.as_ref().map(|f| f.scale), "pieces": fit.as_ref().map(|f| f.pieces.iter().map(|p| (p.offset * 100.0).round() / 100.0).collect::<Vec<_>>()),
        "strength": fit.as_ref().map(|f| (f.strength * 100.0).round() / 100.0), "gain": fit.as_ref().map(|f| (f.gain * 100.0).round() / 100.0),
        "support": fit.as_ref().map(|f| f.support), "after": fit.as_ref().map(|f| f.after), "before": fit.as_ref().map(|f| f.before), "chance": fit.as_ref().map(|f| f.chance),
    })
}

fn cases(i: usize, titles: &[Title]) -> Vec<Case> {
    let t = &titles[i];
    let mut out = vec![
        Case::Same,
        Case::Shift(1.0),
        Case::Shift(-1.5),
        Case::Shift(2.0),
        Case::Shift(-3.0),
        Case::Shift(7.3),
        Case::Rate(25.0 / 24.0, 1.5),
        Case::Rate(24.0 / 25.0, -2.0),
        Case::Rate(1.001, 0.0),
        Case::Rate(1.001, 2.0),
        Case::Cut {
            at: 0.4,
            secs: 60.0,
            shift: 0.0,
        },
        Case::Cut {
            at: 0.4,
            secs: -45.0,
            shift: 0.0,
        },
        Case::Cut {
            at: 0.4,
            secs: 60.0,
            shift: 2.0,
        },
        Case::Cut {
            at: 0.15,
            secs: 70.0,
            shift: 0.0,
        },
        Case::Cut {
            at: 0.7,
            secs: -60.0,
            shift: 0.0,
        },
        Case::Cuts {
            at: [0.3, 0.65],
            secs: [45.0, -30.0],
        },
    ];
    // Another title: the next episode of the same show if there is one, else the next film.
    let other = (1..titles.len())
        .map(|k| (i + k) % titles.len())
        .find(|&j| titles[j].show.is_some() && titles[j].show == t.show)
        .or_else(|| {
            (1..titles.len())
                .map(|k| (i + k) % titles.len())
                .find(|&j| titles[j].show.is_none() && t.show.is_none())
        });
    out.extend(other.map(Case::Wrong));
    out
}

#[test]
#[ignore = "needs fixtures in MOONRISE_SUBSYNC_EVAL; an evaluation run by hand"]
fn eval() {
    let Ok(root) = std::env::var("MOONRISE_SUBSYNC_EVAL") else { return };
    let only = std::env::var("MOONRISE_SUBSYNC_EVAL_ONLY").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root).unwrap().filter_map(|e| e.ok().map(|e| e.path())).collect();
    dirs.sort();
    let titles: Vec<Title> = dirs
        .iter()
        .filter_map(|d| load(d))
        .filter(|t| only.is_empty() || t.name.contains(&only))
        .collect();
    let mut jobs: Vec<(usize, Case, String)> = Vec::new();
    for i in 0..titles.len() {
        for c in cases(i, &titles) {
            jobs.push((i, c.clone(), "audio".into()));
        }
        // Text references: the first other-language track, for the cases a text track decides.
        if let Some((lang, _)) = titles[i].others.first() {
            for c in [
                Case::Same,
                Case::Shift(2.0),
                Case::Rate(25.0 / 24.0, 1.5),
                Case::Cut {
                    at: 0.4,
                    secs: 60.0,
                    shift: 0.0,
                },
                Case::Cut {
                    at: 0.4,
                    secs: -45.0,
                    shift: 0.0,
                },
            ] {
                jobs.push((i, c, lang.clone()));
            }
        }
    }
    // Only cases whose label has this (and audio only, if it says so).
    if let Ok(case) = std::env::var("MOONRISE_SUBSYNC_EVAL_CASE") {
        jobs.retain(|(i, c, reference)| c.label(&titles) == case && (reference == "audio" || i == &usize::MAX));
    }
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let k = next.fetch_add(1, Ordering::SeqCst);
                let Some((i, case, reference)) = jobs.get(k) else { break };
                let r = trial(&titles[*i], &titles, case, reference);
                results.lock().unwrap().push((k, r));
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|r| r.0);
    if let Ok(out) = std::env::var("MOONRISE_SUBSYNC_EVAL_OUT") {
        let lines: Vec<String> = results.iter().map(|r| r.1.to_string()).collect();
        std::fs::write(out, lines.join("\n") + "\n").unwrap();
    }
    // Outcomes by kind of case and reference.
    let mut table: BTreeMap<(String, String), BTreeMap<String, usize>> = BTreeMap::new();
    for (_, r) in &results {
        let case = r["case"].as_str().unwrap();
        let kind = case
            .split(['+', '-', '@', ':'])
            .next()
            .unwrap()
            .trim_end_matches(char::is_numeric)
            .to_string();
        let kind = if case.starts_with("rate") { case.to_string() } else { kind };
        let mode = if r["reference"] == "audio" {
            if r["windows"].as_u64().unwrap_or(0) > 0 {
                "audio-windows"
            } else {
                "audio-whole"
            }
        } else {
            "text"
        };
        *table
            .entry((mode.into(), kind))
            .or_default()
            .entry(r["outcome"].as_str().unwrap().into())
            .or_default() += 1;
    }
    for ((mode, kind), counts) in &table {
        println!("{mode:14} {kind:22} {counts:?}");
    }
    let harm = results.iter().filter(|r| r.1["outcome"] == "HARM").count();
    let false_sync = results.iter().filter(|r| r.1["outcome"] == "false_in_sync").count();
    let fixed = results
        .iter()
        .filter(|r| r.1["outcome"] == "fixed" || r.1["outcome"] == "refused")
        .count();
    println!(
        "titles {} trials {} right {} harm {} false-in-sync {}",
        titles.len(),
        results.len(),
        fixed,
        harm,
        false_sync
    );
    for (_, r) in &results {
        if r["outcome"] != "fixed" && r["outcome"] != "refused" {
            println!(
                "  {:9} {:13} {:40} {:22} {:8} ok {:.2} harm {:.2} {} {}",
                r["outcome"].as_str().unwrap(),
                r["reference"].as_str().unwrap(),
                r["title"].as_str().unwrap(),
                r["case"].as_str().unwrap(),
                r["state"].as_str().unwrap(),
                r["ok"].as_f64().unwrap(),
                r["harm"].as_f64().unwrap(),
                r["pieces"],
                r["gain"]
            );
        }
    }
    for t in &titles {
        println!("bias {:+.2} {} ({})", t.bias, t.name, t.dir.display());
    }
}

/// Real downloads: `MOONRISE_SUBSYNC_EVAL_REAL=<dir>` of `<title folder>__<name>.srt`. Their true
/// timing is their match against the file's own track in their language (`Basis::Text`, which
/// the trials above find exact); each is matched from audio as a run would, and compared.
#[test]
#[ignore = "needs real downloads in MOONRISE_SUBSYNC_EVAL_REAL"]
fn eval_real() {
    let (Ok(root), Ok(real)) = (std::env::var("MOONRISE_SUBSYNC_EVAL"), std::env::var("MOONRISE_SUBSYNC_EVAL_REAL")) else {
        return;
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&real).unwrap().filter_map(|e| e.ok().map(|e| e.path())).collect();
    files.sort();
    for f in files {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let Some((folder, _)) = name.split_once("__") else { continue };
        let Some(t) = load(&Path::new(&root).join(folder)) else { continue };
        let cues = dialogue(&subs::parse_srt(&std::fs::read_to_string(&f).unwrap()));
        let Some(truth) = align(&cues, &Reference::from_cues(&t.truth, t.duration)).filter(|x| verdict(x, Basis::Text) != Verdict::Unsure)
        else {
            println!("{name}: no clear match to the file's own track");
            continue;
        };
        let (fit, windows) = audio_run(&cues, &t);
        let v = fit.as_ref().map_or(Verdict::Unsure, |f| verdict(f, Basis::Audio));
        let (mut ok, mut harm) = (0, 0);
        for c in &cues {
            let want = truth.map(c.0) + t.bias;
            let after = match (&fit, v) {
                (Some(f), Verdict::Apply) => f.map(c.0),
                _ => c.0,
            };
            ok += ((after - want).abs() <= 0.75) as usize;
            harm += ((after - want).abs() > (c.0 - want).abs() + 1.5) as usize;
        }
        let n = cues.len() as f64;
        println!(
            "{:40} true: scale {:.5} pieces {:?} | audio: {v:?} scale {:?} pieces {:?} windows {windows} | within 0.75 s {:.3} worse {:.3}",
            format!("{} {}", t.name, name.split_once("__").unwrap().1),
            truth.scale,
            truth.pieces.iter().map(|p| (p.offset * 100.0).round() / 100.0).collect::<Vec<_>>(),
            fit.as_ref().map(|f| f.scale),
            fit.as_ref()
                .map(|f| f.pieces.iter().map(|p| (p.offset * 100.0).round() / 100.0).collect::<Vec<_>>()),
            ok as f64 / n,
            harm as f64 / n
        );
    }
}
