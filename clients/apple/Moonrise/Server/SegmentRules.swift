import Foundation

/// The rules that pick a title's skippable segments from the file's and the community databases'.
/// Servers 0.28+ apply them (`server/src/media/segment_rules.rs`, `segments?merged=1`); the app
/// keeps them for older servers' answers (without the databases: `SegmentService`). The Android
/// app has the same rules (`Segments.kt`); all three are tested against `api/segment-rules.json`
/// (`SegmentRulesCasesTests`): keep them alike.
enum SegmentRules {
    /// The server's segments, with the community databases' filling the kinds it lacks (the
    /// first database that has one wins), and the end credits checked against the file. A credits
    /// chapter never starts before the latest database start (`notBefore`). While
    /// the server is still looking for the credits in the file (`pending`), the databases' aren't
    /// used: they may be another cut's, and the player asks again before the end. While the title
    /// still waits for its community lookup (`communityPending`), no credits at all: the file's
    /// could start before a database start not known yet. The other kinds are offered:
    /// the file's own always win over the databases'.
    ///
    /// A film (`movie`) may have scenes in or after its credits, so its credits are only skippable
    /// when they end before the end of the file (`creditsEndBeforeEnd`): Skip Credits then jumps to
    /// that end and the scene after them plays. Credits running to the end are left out (the file's
    /// own chapter ones give way to the databases', which may have an end).
    static func merge(server: [MediaSegment], online: [[MediaSegment]], runtime: Double, pending: Bool = false, movie: Bool = false,
                      communityPending: Bool = false) -> [MediaSegment] {
        let detected = server.first { $0.kind == .credits && $0.source == "detected" }
        var found = server.filter { $0.source == nil && !(movie && $0.kind == .credits && !endsBeforeEnd($0, runtime: runtime)) }
        for more in online {
            let have = Set(found.map(\.kind))
            found += more.filter { $0.kind != .credits && !have.contains($0.kind) }
        }
        let community = online.flatMap { $0.filter { $0.kind == .credits } }.filter { plausibleCredits($0, runtime: runtime) }
        if !pending, let latest = community.max(by: { $0.start < $1.start }) {
            found = found.compactMap { s in
                guard s.kind == .credits else { return s }
                let c = notBefore(s, latest)
                return !movie || endsBeforeEnd(c, runtime: runtime) ? c : nil
            }
        }
        if !pending, !found.contains(where: { $0.kind == .credits }) {
            if let c = credits(community: community, detected: detected, runtime: runtime), !movie || endsBeforeEnd(c, runtime: runtime) { found.append(c) }
        }
        return found
            .map { MediaSegment(kind: $0.kind, start: max(0, $0.start), end: runtime > 0 ? min($0.end, runtime) : $0.end, source: $0.source) }
            .filter { $0.end - $0.start >= 3 && !(communityPending && $0.kind == .credits) }
            .sorted { $0.start < $1.start }
    }

    /// Credits chapters never start before the latest database start: like what the file shows,
    /// a chapter only moves the credits later (Guardians of the Galaxy: chapter 6796, IntroDB 6852).
    /// When the database start is past the chapter's end, the database's credits are used.
    static func notBefore(_ chapter: MediaSegment, _ database: MediaSegment) -> MediaSegment {
        guard database.start > chapter.start else { return chapter }
        guard database.start < chapter.end - 3 else { return database }
        return MediaSegment(kind: .credits, start: database.start, end: chapter.end, source: chapter.source)
    }

    /// Where the end credits start, as the progress reports tell the server (`credits`): the
    /// credits the player offers. The server marks a film watched from there rather than at 90 %
    /// (it knows only the file's chapters, not the databases').
    static func creditsStart(_ segments: [MediaSegment]) -> Double? {
        segments.filter { $0.kind == .credits }.map(\.start).max()
    }

    /// A film's credits must end at least this long before the end of the file to be skippable.
    static let creditsEndBeforeEnd: Double = 5

    /// Whether a segment stops before the end of the file (not when the runtime isn't known).
    static func endsBeforeEnd(_ s: MediaSegment, runtime: Double) -> Bool {
        runtime > 0 && s.end < runtime - creditsEndBeforeEnd
    }

    /// Whether community end credits can be this file's: in its second half, starting before its
    /// end, and not ending well after it (a longer cut). Without a runtime nothing is known.
    static func plausibleCredits(_ s: MediaSegment, runtime: Double) -> Bool {
        guard runtime > 0 else { return true }
        return s.start >= runtime / 2 && s.start <= runtime - 3 && s.end <= runtime + 5
    }

    /// Community end credits running to the end are trusted this long before it with nothing to
    /// back them. Measured: right ones started 34–131 s before the end of their file, and the
    /// wrong ones (The Sopranos, another cut, inside the last scene) 170–280 s before it. Longer
    /// credits are kept when another database has credits too (Loki's ~290 s).
    static let unconfirmedCreditsLead: Double = 150
    /// A database start at most this long before the credits found in the file is used as it is
    /// (it times them more closely); one earlier than that gives way to the found start.
    static let detectedSlack: Double = 10

    /// The end credits to use: never earlier than a database's start, and with several, the
    /// latest (later is never story). What the file shows only moves them later.
    /// - With the file's own start: the latest database start, unless the found start is more
    ///   than `detectedSlack` after it (another cut's value inside the last scene, or long credits
    ///   found late in the file): then the found start.
    /// - With none found in the file: the latest database start. A lone database value that runs
    ///   to the end from far before it (more than `unconfirmedCreditsLead`) is left out (the
    ///   lone-credits rule); with two or more values, the latest of them all (Agatha 1x01's 2107
    ///   and 2235: 2235, whichever of them stops before the end).
    static func credits(community: [MediaSegment], detected: MediaSegment?, runtime: Double = 0) -> MediaSegment? {
        guard let detected else {
            guard let latest = community.max(by: { $0.start < $1.start }) else { return nil }
            if community.count >= 2 || runtime <= 0 || latest.end < runtime - 5 || runtime - latest.start <= unconfirmedCreditsLead { return latest }
            return nil
        }
        guard let latest = community.max(by: { $0.start < $1.start }), latest.start >= detected.start - detectedSlack else { return detected }
        return latest
    }

    /// While the server looks a title up in the community databases (`community_pending`), the
    /// player asks again this many times, this many seconds apart.
    static let lookupRetries = 3
    static let lookupRetryDelay: Double = 5

    /// When the answer still isn't final (the server waits for the community databases or the
    /// file, or wasn't reached), the player asks again near the end: on entering the last
    /// `nearEndAsks` seconds, once each. A film's credits can start minutes before the end.
    static let nearEndAsks: [Double] = [600, 120]

    /// How many of the near-end asks are due at `t` (each once): the player asks when this grows.
    static func nearEndAsksDue(duration: Double, time t: Double) -> Int {
        guard duration > 0, t > 0, t < duration else { return 0 }
        return nearEndAsks.filter { duration - t <= $0 }.count
    }

    /// How long before the end Next Episode shows when the credits aren't known: short, since
    /// some shows' story runs to within half a minute of the end.
    static let nextEpisodeLead: Double = 30

    /// Whether Next Episode is offered with the controls: in the end credits (the segment the
    /// player is in, or credits from where they start when they run to the end), else in the last
    /// `nextEpisodeLead` seconds. Never before playback has started or without a next episode.
    /// `shownTime` is where the controls are (a pending skip's target), `time` the player's. The
    /// Android app has the same rule (`NextEpisodeRules.offered`).
    static func nextEpisodeOffered(duration: Double, time: Double, shownTime: Double, active: MediaSegment?,
                                   segments: [MediaSegment], hasNext: Bool) -> Bool {
        guard duration > 0, time > 0, hasNext else { return false }
        if active?.kind == .credits { return true }
        if segments.contains(where: { $0.kind == .credits && shownTime >= $0.start && $0.end >= duration - 5 }) { return true }
        return duration - shownTime <= nextEpisodeLead
    }
}
