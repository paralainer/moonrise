//! The segments the players offer (Skip Intro, Skip Credits, Next Episode), merged from the
//! file's own (chapter names, end credits found in it: `credits.rs`) and the community databases'
//! (`library/community.rs`). `GET /v1/items/{id}/segments?merged=1` answers with them.
//!
//! The rules were the apps' (`SegmentRules.swift`, Android `Segments.kt`), which keep them for
//! older servers; all three are checked against the same cases (`api/segment-rules.json`).
//! [ADR 0005](../../../docs/adr/0005-end-credits-never-earlier.md): a database's credits start is
//! never moved earlier, and with several the latest is used; what the file shows only moves it
//! later.

use serde::{Deserialize, Serialize};

/// A film's credits must end at least this long before the end of the file to be skippable.
pub const CREDITS_END_BEFORE_END: f64 = 5.0;
/// Community end credits running to the end are trusted this long before it with nothing to
/// back them (right ones started 34–131 s before the end of their file, wrong ones 170–280 s).
pub const UNCONFIRMED_CREDITS_LEAD: f64 = 150.0;
/// A database start at most this long before the credits found in the file is used as it is;
/// one earlier than that gives way to the found start.
pub const DETECTED_SLACK: f64 = 10.0;
/// Shorter segments aren't offered.
const MIN_LENGTH: f64 = 3.0;

/// A skippable stretch. `source`: `chapters`, `detected` (found in the file), or the community
/// database's name (`theintrodb`, `introdb`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub kind: String,
    pub start: f64,
    pub end: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl Segment {
    pub fn new(kind: &str, start: f64, end: f64, source: Option<&str>) -> Segment {
        Segment {
            kind: kind.to_string(),
            start,
            end,
            source: source.map(String::from),
        }
    }

    fn is_credits(&self) -> bool {
        self.kind == "credits"
    }

    fn detected(&self) -> bool {
        self.source.as_deref() == Some("detected")
    }
}

/// The first segment with the latest start (as Swift's `max(by:)` and Kotlin's `maxByOrNull`
/// pick: ties keep the first).
fn latest<'a>(list: impl IntoIterator<Item = &'a Segment>) -> Option<&'a Segment> {
    let mut best: Option<&Segment> = None;
    for s in list {
        if best.is_none_or(|b| s.start > b.start) {
            best = Some(s);
        }
    }
    best
}

/// The file's segments, with the community databases' (one list per database, in the order
/// asked) filling the kinds it lacks (the first database that has one wins), and the end credits
/// checked against the file. A credits chapter never starts before the latest database start
/// ([`not_before`]). While the server is still looking for the credits in the file (`pending`),
/// the databases' credits aren't used: they may be another cut's. While the title still waits for
/// its community lookup (`community_pending`), no credits at all: the file's could start before a
/// database start not known yet (Guardians: chapter 6796, IntroDB 6852), so Skip Credits and Next
/// Episode from the credits wait for the answer. Intro, recap and the other kinds don't:
/// the file's own always win over the databases', so they can't change when the answer comes.
///
/// A film (`movie`) may have scenes in or after its credits, so its credits are only skippable
/// when they end before the end of the file ([`CREDITS_END_BEFORE_END`]): Skip Credits then jumps
/// to that end. Credits running to the end are left out (the file's own chapter ones give way to
/// the databases', which may have an end).
pub fn merge(
    server: &[Segment],
    online: &[Vec<Segment>],
    runtime: f64,
    pending: bool,
    movie: bool,
    community_pending: bool,
) -> Vec<Segment> {
    let detected = server.iter().find(|s| s.is_credits() && s.detected());
    let mut found: Vec<Segment> = server
        .iter()
        .filter(|s| !s.detected() && (!movie || !s.is_credits() || ends_before_end(s, runtime)))
        .cloned()
        .collect();
    for more in online {
        let have: Vec<String> = found.iter().map(|s| s.kind.clone()).collect();
        found.extend(more.iter().filter(|s| !s.is_credits() && !have.contains(&s.kind)).cloned());
    }
    let community: Vec<Segment> = online
        .iter()
        .flatten()
        .filter(|s| s.is_credits() && plausible_credits(s, runtime))
        .cloned()
        .collect();
    if !pending {
        if let Some(latest) = latest(&community) {
            found = found
                .into_iter()
                .filter_map(|s| {
                    if !s.is_credits() {
                        return Some(s);
                    }
                    let c = not_before(&s, latest);
                    (!movie || ends_before_end(&c, runtime)).then_some(c)
                })
                .collect();
        }
    }
    if !pending && !found.iter().any(Segment::is_credits) {
        if let Some(c) = credits(&community, detected, runtime) {
            if !movie || ends_before_end(&c, runtime) {
                found.push(c);
            }
        }
    }
    let mut out: Vec<Segment> = found
        .into_iter()
        .map(|s| Segment {
            start: s.start.max(0.0),
            end: if runtime > 0.0 { s.end.min(runtime) } else { s.end },
            ..s
        })
        .filter(|s| s.end - s.start >= MIN_LENGTH && !(community_pending && s.is_credits()))
        .collect();
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}

/// Credits chapters never start before the latest database start: like what the file shows, a
/// chapter only moves the credits later (a chapter at 6796 with a database at 6852: 6852 to the
/// chapter's end). When the database start is past the chapter's end, the database's credits.
pub fn not_before(chapter: &Segment, database: &Segment) -> Segment {
    if database.start <= chapter.start {
        return chapter.clone();
    }
    if database.start >= chapter.end - 3.0 {
        return database.clone();
    }
    Segment {
        kind: "credits".into(),
        start: database.start,
        end: chapter.end,
        source: chapter.source.clone(),
    }
}

/// Whether a segment stops before the end of the file (not when the runtime isn't known).
pub fn ends_before_end(s: &Segment, runtime: f64) -> bool {
    runtime > 0.0 && s.end < runtime - CREDITS_END_BEFORE_END
}

/// Whether community end credits can be this file's: in its second half, starting before its
/// end, and not ending well after it (a longer cut). Without a runtime nothing is known.
pub fn plausible_credits(s: &Segment, runtime: f64) -> bool {
    if runtime <= 0.0 {
        return true;
    }
    s.start >= runtime / 2.0 && s.start <= runtime - 3.0 && s.end <= runtime + 5.0
}

/// The end credits to use: never earlier than a database's start, and with several, the latest.
/// - With the file's own start: the latest database start, unless the found start is more than
///   [`DETECTED_SLACK`] after it; then the found start.
/// - With none found: the latest database start. A lone value that runs to the end from more
///   than [`UNCONFIRMED_CREDITS_LEAD`] before it is left out; with two or more, the latest.
pub fn credits(community: &[Segment], detected: Option<&Segment>, runtime: f64) -> Option<Segment> {
    let Some(detected) = detected else {
        let latest = latest(community)?;
        let keep =
            community.len() >= 2 || runtime <= 0.0 || latest.end < runtime - 5.0 || runtime - latest.start <= UNCONFIRMED_CREDITS_LEAD;
        return keep.then(|| latest.clone());
    };
    match latest(community) {
        Some(l) if l.start >= detected.start - DETECTED_SLACK => Some(l.clone()),
        _ => Some(detected.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn segs(v: &Value) -> Vec<Segment> {
        serde_json::from_value(v.clone()).unwrap()
    }

    /// The cases all three implementations are checked against (`api/segment-rules.json`): the
    /// same answers as the apps' rules gave on the same inputs.
    #[test]
    fn shared_cases() {
        let file: Value = serde_json::from_str(include_str!("../../../api/segment-rules.json")).unwrap();
        let cases = file["cases"].as_array().unwrap();
        assert!(cases.len() >= 10);
        for c in cases {
            let name = c["name"].as_str().unwrap();
            let online: Vec<Vec<Segment>> = c["online"].as_array().unwrap().iter().map(segs).collect();
            let got = merge(
                &segs(&c["server"]),
                &online,
                c["runtime"].as_f64().unwrap(),
                c["pending"].as_bool().unwrap_or(false),
                c["movie"].as_bool().unwrap_or(false),
                c["community_pending"].as_bool().unwrap_or(false),
            );
            let got: Vec<(String, f64, f64)> = got.into_iter().map(|s| (s.kind, s.start, s.end)).collect();
            let want: Vec<(String, f64, f64)> = segs(&c["expected"]).into_iter().map(|s| (s.kind, s.start, s.end)).collect();
            assert_eq!(got, want, "{name}");
        }
    }

    #[test]
    fn ties_keep_the_first() {
        let a = Segment::new("credits", 2000.0, 2100.0, None);
        let b = Segment::new("credits", 2000.0, 2200.0, None);
        assert_eq!(latest([&a, &b]).unwrap().end, 2100.0);
        assert_eq!(credits(&[a.clone(), b], None, 2300.0).unwrap(), a);
    }

    #[test]
    fn a_chapter_never_starts_before_a_database_start() {
        let chapter = Segment::new("credits", 6796.6, 7210.8, Some("chapters"));
        let db = Segment::new("credits", 6852.0, 7210.0, Some("introdb"));
        let m = merge(std::slice::from_ref(&chapter), &[vec![], vec![db]], 7266.0, false, true, false);
        assert_eq!(m, vec![Segment::new("credits", 6852.0, 7210.8, Some("chapters"))]);
        // While the file is still being looked at, the database's credits wait.
        let m = merge(
            std::slice::from_ref(&chapter),
            &[vec![Segment::new("credits", 6852.0, 7210.0, None)]],
            7266.0,
            true,
            true,
            false,
        );
        assert_eq!(m, vec![chapter.clone()]);
    }

    #[test]
    fn found_in_the_file_only_moves_later() {
        let found = Segment::new("credits", 2400.0, 2520.0, Some("detected"));
        // Another cut's value inside the last scene (more than 10 s before what the file shows).
        let early = Segment::new("credits", 2300.0, 2520.0, Some("theintrodb"));
        assert_eq!(
            merge(std::slice::from_ref(&found), &[vec![early]], 2520.0, false, false, false),
            vec![found.clone()]
        );
        // Within 10 s: the database's.
        let close = Segment::new("credits", 2392.0, 2520.0, Some("introdb"));
        assert_eq!(
            merge(std::slice::from_ref(&found), &[vec![close.clone()]], 2520.0, false, false, false),
            vec![close]
        );
    }

    /// While the title waits for its community lookup, no credits are offered (the
    /// file's chapter could start before the database start not known yet); the intro chapter is.
    #[test]
    fn no_credits_while_the_community_lookup_is_pending() {
        let intro = Segment::new("intro", 60.0, 120.0, Some("chapters"));
        let chapter = Segment::new("credits", 6796.6, 7210.8, Some("chapters"));
        let file = [intro.clone(), chapter];
        assert_eq!(merge(&file, &[vec![], vec![]], 7266.0, false, true, true), vec![intro.clone()]);
        // An episode's credits found in the file wait too.
        let found = Segment::new("credits", 2400.0, 2520.0, Some("detected"));
        assert_eq!(merge(&[found], &[vec![], vec![]], 2520.0, false, false, true), vec![]);
        // Once the answer is in, the database start.
        let db = Segment::new("credits", 6852.0, 7210.0, Some("introdb"));
        let m = merge(&file, &[vec![], vec![db]], 7266.0, false, true, false);
        assert_eq!(m[1], Segment::new("credits", 6852.0, 7210.8, Some("chapters")));
    }
}
