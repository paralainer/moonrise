import Foundation

/// A stream that stops moving while its session answers: the link carries less than the stream
/// needs. Opening the same stream again throws away what had arrived and starts over, so it can't
/// get ahead; recovery goes down to a quality the link carries instead, and when there is no lower
/// one it says the connection is too slow (the server isn't failing).
enum SlowLinkRules {
    /// Share of the measured rate a lower quality may use (a transcode's bitrate varies, and the
    /// player needs some to spare to build a buffer).
    static let headroom = 0.75

    /// What recovery does about a stall.
    enum Action: Equatable {
        /// Open the same stream again (a dead connection, a server that went away).
        case restart
        /// Too slow for this quality: go down to this one.
        case lower(QualityOption)
        /// Too slow, and nothing lower: stop and say so.
        case tooSlow
    }

    /// Recovery's choice for a stall (no progress while it should play): `linkTooSlow`, then
    /// `lowerQuality`.
    static func action(sessionAnswered: Bool, measured: Double?, needed: Double?, answeredRestarts: Int,
                       current: QualityOption, options: [QualityOption], sourceBitrate: Int?) -> Action {
        guard linkTooSlow(sessionAnswered: sessionAnswered, measured: measured, needed: needed,
                          answeredRestarts: answeredRestarts) else { return .restart }
        return lowerQuality(than: current, options: options, sourceBitrate: sourceBitrate, measured: measured)
            .map(Action.lower) ?? .tooSlow
    }

    /// Is the stall the link being too slow for this stream?
    /// - sessionAnswered: the session's playlist answered during the stall (the server is there).
    /// - measured: bits/s that arrived for the stream during the stall, when known (0: nothing,
    ///   a dead connection, which a new stream does fix).
    /// - needed: the stream's bits/s, when known.
    /// - answeredRestarts: stalls since playback last ran for a while that ended in opening the
    ///   same stream again although the session answered.
    static func linkTooSlow(sessionAnswered: Bool, measured: Double?, needed: Double?, answeredRestarts: Int) -> Bool {
        guard sessionAnswered else { return false }
        if let measured, let needed, needed > 0 { return measured > 0 && measured < needed }
        // No measure: once the same stream, opened again, stalled again with the server there.
        return answeredRestarts >= 1
    }

    /// The quality to go down to from `current`, or nil when none is lower: the best below it
    /// that fits the measured rate with headroom, else the lowest; the next one down when nothing
    /// was measured. `sourceBitrate` is Original's.
    static func lowerQuality(than current: QualityOption, options: [QualityOption], sourceBitrate: Int?,
                             measured: Double?) -> QualityOption? {
        let now = Double(current.bitrate ?? sourceBitrate ?? .max)
        let below = options.filter { ($0.bitrate.map(Double.init) ?? .infinity) < now }
            .sorted { ($0.bitrate ?? 0) > ($1.bitrate ?? 0) }
        guard let next = below.first else { return nil }
        guard let measured, measured > 0 else { return next }
        return below.first { Double($0.bitrate ?? 0) <= measured * headroom } ?? below.last
    }
}
