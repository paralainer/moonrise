// The end-credits rules (Moonrise/Server/SegmentRules.swift) on episodes and films measured on a
// copy of a real library: the community databases' values and the credits
// the server found in the file. The Android app runs the same cases
// (clients/android/core/src/test/.../SegmentsTest.kt). Moved here from scripts/tests/segments.
import Foundation
import Testing
@testable import Moonrise

private func check(_ ok: Bool, _ what: String, sourceLocation: SourceLocation = #_sourceLocation) {
    #expect(ok, Comment(rawValue: what), sourceLocation: sourceLocation)
}

private func credits(_ start: Double, _ end: Double, _ source: String? = nil) -> MediaSegment { MediaSegment(kind: .credits, start: start, end: end, source: source) }
private func intro(_ start: Double, _ end: Double) -> MediaSegment { MediaSegment(kind: .intro, start: start, end: end) }

/// The credits start the rules give an episode: the databases' starts (each running to the end),
/// and the start the server found in the file.
private func start(_ runtime: Double, _ db: Double..., detected: Double? = nil) -> Double? {
    let server = detected.map { [credits($0, runtime, "detected")] } ?? []
    return SegmentRules.merge(server: server, online: db.map { [credits($0, runtime)] }, runtime: runtime).first { $0.kind == .credits }?.start
}

/// The credits a film gets: the file's chapter credits (`chapter`: start, end) and the databases'.
private func film(_ runtime: Double, chapter: (Double, Double)? = nil, _ db: (Double, Double)...) -> MediaSegment? {
    let server = chapter.map { [credits($0.0, $0.1)] } ?? []
    return SegmentRules.merge(server: server, online: db.map { [credits($0.0, $0.1)] }, runtime: runtime, movie: true).first { $0.kind == .credits }
}

@Suite struct SegmentRulesTests {
    /// Never earlier than a database start.
    @Test func neverEarlierThanADatabaseStart() {
        // The Sopranos 4x13: TheIntroDB 4215 is in the story, found 4393.3. Even a database start a few
        // seconds into the last shot (4380) gives way to the found start.
        check(start(4510.848, 4215, detected: 4393.306) == 4393.306, "Sopranos 4x13")
        check(start(4510.848, 4380, detected: 4393.306) == 4393.306, "Sopranos 4x13, a value in the last shot")
        // The Sopranos 1x02: both databases within 10 s before the found 2920.5: the later, 2921.
        check(start(2983.648, 2911, 2921, detected: 2920.543) == 2921, "Sopranos 1x02")
        // The Sopranos 2x05 and 2x13: TheIntroDB in the last scene, IntroDB at the credits.
        check(start(3115.68, 2926, 3050, detected: 3049.046) == 3050, "Sopranos 2x05")
        check(start(3554.688, 3311, 3451, detected: 3454.451) == 3451, "Sopranos 2x13")
        // The Sopranos 6x06: TheIntroDB 3048, 2 min early; found 3180.1. 6x02: TheIntroDB 3133 within 10 s.
        check(start(3324.672, 3048, detected: 3180.094) == 3180.094, "Sopranos 6x06")
        check(start(3255.52, 3133, detected: 3135.591) == 3133, "Sopranos 6x02")
        // The Sopranos 4x01: nothing found, TheIntroDB alone 240 s before the end: left out.
        check(start(3467.552, 3227) == nil, "Sopranos 4x01")
        // Sherlock 1x01: the later database value, after the found start.
        check(start(5282.04, 5263, 5245.29, detected: 5250.92) == 5263, "Sherlock 1x01")
        // The Rings of Power 2x03, 2x06: credits on black from TheIntroDB's 3704.3 and 3497.3, but the dense
        // crawl is found only at 3818 and 3614: late, inside the credits.
        check(start(4003.136, 3704.311, detected: 3818) == 3818, "Rings of Power 2x03")
        check(start(3786.72, 3497.274, detected: 3614) == 3614, "Rings of Power 2x06")
        // The Punisher 1x05, 2x01: the databases are a few seconds into the last shot; found later.
        check(start(3309.728, 3176.718, 3176.72, detected: 3255.252) == 3255.252, "Punisher 1x05")
        check(start(3189.92, 3053.233, 3061.68, detected: 3135.132) == 3135.132, "Punisher 2x01")
        // Agatha All Along 1x01: art cards from 2107 (TheIntroDB), the crawl from 2235 (IntroDB), nothing
        // found. Both run to the end from far before it and disagree: the later, the crawl.
        check(start(2389.792, 2107, 2235) == 2235, "Agatha 1x01")
    }

    /// Films: never past a scene in or after the credits.
    @Test func filmsNeverSkipPastAScene() {
        // Avengers: Infinity War: the "End Credits" chapter runs to the end (a scene after them); IntroDB
        // 8251-8852 ends before it: Skip Credits jumps to 8852.
        check(film(8978.528, chapter: (8251.285, 8978.528), (8251, 8852)) == credits(8251, 8852), "Infinity War")
        // Thor: The Dark World: the same, IntroDB 6109-6656.
        check(film(6723.411, chapter: (6109.52, 6723.411), (6109, 6656)) == credits(6109, 6656), "Thor: The Dark World")
        // Guardians of the Galaxy: IntroDB 6852-7210, where the file's "End Tag" chapter starts.
        check(film(7268.64, (6852, 7210)) == credits(6852, 7210), "Guardians of the Galaxy")
        // A credits chapter that ends before the end of the file is kept as it is.
        check(film(7268.64, chapter: (6852, 7210.828)) == credits(6852, 7210.828), "a chapter with an end")
        // Deadpool & Wolverine: TheIntroDB 7181-7601, IntroDB 7363-7609: the later start, ending at 7609.
        check(film(7753.209, (7181.041, 7601.041), (7363, 7609)) == credits(7363, 7609), "Deadpool & Wolverine")
        // Thor: Ragnarok: IntroDB alone, 7409.3-7769.6, the end scene after it.
        check(film(7830.793, (7409.277, 7769.637)) == credits(7409.277, 7769.637), "Thor: Ragnarok")
        // Avengers: Endgame-like: no end known (a database without one, or a chapter to the end): nothing offered.
        check(film(10871.55, (10200, 10871.55)) == nil, "a film's open-ended database credits")
        check(film(8978.528, chapter: (8251.285, 8978.528)) == nil, "a film's chapter credits to the end")
        check(film(8978.528, (8251, 8976)) == nil, "ending within 5 s of the end")
        check(film(0, (8251, 8852)) == nil, "no runtime: the end isn't known")
        // The pick is as for episodes, then dropped when it runs to the end (never an earlier start instead).
        check(film(7753.209, (7363, 7609), (7650, 7753.209)) == nil, "the later database start runs to the end")
        // Episodes are unchanged: credits to the end are offered.
        check(start(3554.688, 3451) == 3451, "an episode's credits to the end")
    }

    /// The credits start the progress reports carry: the server marks a film watched from
    /// there, not at 90 %, also when only a database knows the credits.
    @Test func reportedCreditsStart() {
        // Thor: Ragnarok: IntroDB alone (7409 of 7831; 90 % is 7048).
        let ragnarok = SegmentRules.merge(server: [intro(10, 40)], online: [[credits(7409.277, 7769.637)]], runtime: 7830.793, movie: true)
        check(SegmentRules.creditsStart(ragnarok) == 7409.277, "Thor: Ragnarok")
        check(SegmentRules.creditsStart([intro(10, 40)]) == nil, "no credits: none sent")
        check(SegmentRules.creditsStart([]) == nil, "nothing known")
        check(SegmentRules.creditsStart([credits(300, 450), credits(7409, 7769)]) == 7409, "the last credits")
    }

    /// A credits chapter never starts before a database start: it only moves the credits later.
    @Test func chaptersNeverBeforeADatabaseStart() {
        // Guardians of the Galaxy: "en:End Credits" 6796.6-7210.8, IntroDB 6852-7210: from 6852.
        check(film(7268.64, chapter: (6796.622, 7210.828), (6852, 7210)) == credits(6852, 7210.828), "Guardians")
        check(SegmentRules.creditsStart(SegmentRules.merge(server: [credits(6796.622, 7210.828)], online: [[credits(6852, 7210)]], runtime: 7268.64, movie: true)) == 6852, "Guardians: the start reported")
        // A chapter after the database start is kept (The Matrix Reloaded-like: later is never story).
        check(film(8300, chapter: (7700, 8189), (7624, 8189)) == credits(7700, 8189), "a later chapter")
        // Episodes too: a chapter 14 s before a database start (a Shōgun-like case), the latest of two.
        check(SegmentRules.merge(server: [credits(3400, 3600)], online: [[credits(3414, 3600)], [credits(3405, 3600)]], runtime: 3600).first { $0.kind == .credits } == credits(3414, 3600), "an episode's chapter")
        // A database start past the chapter's end: the database's credits.
        check(SegmentRules.notBefore(credits(3000, 3100), credits(3200, 3600)) == credits(3200, 3600), "past the chapter")
        // No database value: the chapter as it is.
        check(film(7268.64, chapter: (6796.622, 7210.828)) == credits(6796.622, 7210.828), "chapter alone")
    }

    /// Chapter names with a language tag (Guardians of the Galaxy: "en:End Credits").
    @Test func chapterNames() {
        check(MediaSegment.Kind(chapterName: "en:End Credits") == .credits, "en:End Credits")
        check(MediaSegment.Kind(chapterName: "eng: Intro") == .intro, "eng: Intro")
        check(MediaSegment.Kind(chapterName: "End Credits") == .credits, "End Credits")
        check(MediaSegment.Kind(chapterName: "en:Chapter 12") == nil, "en:Chapter 12")
        check(MediaSegment.Kind(chapterName: "Part 2: Credits") == nil, "only a language tag is dropped")
    }

    /// The rule itself.
    @Test func theRule() {

        let found = credits(3000, 3300, "detected")
        check(SegmentRules.credits(community: [credits(2985, 3300), credits(3010, 3300), credits(2995, 3300)], detected: found)?.start == 3010, "the latest database start")
        check(SegmentRules.credits(community: [credits(2900, 3300), credits(2985, 3300)], detected: found) == found, "latest more than 10 s before")
        check(SegmentRules.credits(community: [], detected: found) == found, "no database")
        check(SegmentRules.credits(community: [credits(2924, 3212), credits(2915, 3212)], detected: nil, runtime: 3212)?.start == 2924, "Loki 2x03: agreeing values")
        check(SegmentRules.credits(community: [credits(1000, 1100)], detected: nil, runtime: 2600)?.start == 1000, "not open-ended")
        check(SegmentRules.credits(community: [credits(2500, 2600)], detected: nil, runtime: 2600)?.start == 2500, "close to the end")
        // One database's credits stop before the end (2107-2300), the other runs to the end from
        // far before it (2235): never earlier than a database start, so 2235, in either order.
        let pair = [credits(2107, 2300), credits(2235, 2400)]
        check(SegmentRules.credits(community: pair, detected: nil, runtime: 2400) == credits(2235, 2400), "lone-credits pair")
        check(SegmentRules.credits(community: pair.reversed(), detected: nil, runtime: 2400) == credits(2235, 2400), "lone-credits pair, reversed")
        check(SegmentRules.merge(server: [], online: [[credits(2107, 2300)], [credits(2235, 2400)]], runtime: 2400) == [credits(2235, 2400)], "lone-credits pair, merged")
        check(!SegmentRules.plausibleCredits(credits(0, 1300), runtime: 1300), "from the start")
        check(!SegmentRules.plausibleCredits(credits(1310, 1320), runtime: 1300), "after the end")
        check(!SegmentRules.plausibleCredits(credits(1250, 1320), runtime: 1300), "a longer cut")
        check(SegmentRules.plausibleCredits(credits(1250, 1304), runtime: 1300), "fits")
        let held = SegmentRules.merge(server: [], online: [[intro(60, 90), credits(1250, 1300)]], runtime: 1300, pending: true)
        check(held == [intro(60, 90)], "pending holds back the databases' credits")
    }

    /// Next Episode near the end: from where end credits running to the end start, else
    /// only in the last 30 s; never in the story. Same cases as the Android app's
    /// `SegmentsTest.nextEpisodeOfferedFromCreditsToTheEndElseTheLast30Seconds`.
    @Test func nextEpisodeOfferedFromCreditsToTheEndElseTheLast30Seconds() {
        func offered(_ d: Double, _ time: Double, _ shown: Double, _ active: MediaSegment? = nil, _ segs: [MediaSegment] = [],
                     hasNext: Bool = true) -> Bool {
            SegmentRules.nextEpisodeOffered(duration: d, time: time, shownTime: shown, active: active, segments: segs, hasNext: hasNext)
        }
        let d = 3324.672
        let segs = [credits(3180.1, d, "detected")]
        check(!offered(d, 3100, 3100, nil, segs), "in the story")
        check(offered(d, 3180.5, 3180.5, nil, segs), "from where the credits start")
        check(!offered(d, 3200, 3200, nil, segs, hasNext: false), "the last episode (or a film)")
        check(!offered(d, 0, 3200, nil, segs), "not started")
        check(!offered(0, 3200, 3200, nil, segs), "no duration")
        // No credits known: the last 30 s only.
        let d2 = 3467.552
        check(!offered(d2, 3430, 3430), "37 s before the end, no credits known")
        check(offered(d2, 3438, 3438), "29.6 s before the end")
        // Credits that stop well before the end (a scene follows): not from there.
        check(!offered(2000, 1800, 1800, nil, [credits(1700, 1900)]), "credits ending 100 s before the end")
        // Inside a credits segment the player is in: yes.
        check(offered(2000, 1800, 1800, credits(1700, 1900)), "in the active credits segment")
        check(!offered(2000, 1800, 1800, intro(1700, 1900)), "an active intro isn't credits")
        // A pending skip's target counts (the controls show it).
        check(offered(d, 3100, 3190, nil, segs), "a skip into the credits")
    }
}
