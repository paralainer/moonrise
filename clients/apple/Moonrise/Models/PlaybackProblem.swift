import Foundation

/// A playback failure in plain words: a short title, one line on what it means or what to
/// do, and the server's own text (ffprobe output, paths, codes) kept for Details. The rule is
/// shared with the Android and Windows apps: api/playback-problem-rules.json.
struct PlaybackProblem: Equatable {
    enum Kind: String, CaseIterable {
        case unreachable, preparing, missing, share, damaged, unindexed, busy, server, unsupported
    }

    let kind: Kind
    /// What the server (or the network) said, for Details (nil: nothing more to say).
    let detail: String?

    var title: String {
        switch kind {
        case .unreachable: "Can't reach the server"
        case .preparing: "Still getting ready"
        case .missing: "File not found"
        case .share: "Can't reach the share"
        case .damaged: "Damaged file"
        case .unindexed: "Can't stream this file"
        case .busy: "Server busy"
        case .server: "Server problem"
        case .unsupported: "Can't play this video"
        }
    }

    var message: String {
        switch kind {
        case .unreachable: "Check that the Moonrise server is on and this device is on its network."
        case .preparing: "The server is still reading this file. Try again in a minute."
        case .missing: "The server can't find this video's file. It may have been moved or deleted, or its share is offline."
        case .share: "The server can't read the network share this video is on. Check that it's on and connected."
        case .damaged: "This file is damaged or incomplete, so it can't play. Replacing it fixes this."
        case .unindexed: "The file has no index to stream from. Saving it again (remuxing) fixes this."
        case .busy: "The server is converting other videos right now. Try again in a moment."
        case .server: "The server ran into a problem with this video. Try again."
        case .unsupported: "The server can't play this video on this device."
        }
    }

    /// - status: the HTTP status of the server's answer; nil when nothing answered.
    /// - text: its body, an offer's reasons, or a file's `broken_reason`.
    /// - pending: an offer whose file is still being indexed.
    static func classify(status: Int?, text: String, pending: Bool = false) -> PlaybackProblem {
        let raw = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return PlaybackProblem(kind: kind(status: status, text: raw, pending: pending), detail: raw.isEmpty ? nil : raw)
    }

    static func kind(status: Int?, text: String, pending: Bool) -> Kind {
        guard let status else { return .unreachable }
        if pending { return .preparing }
        let t = text.lowercased()
        let has = { (words: [String]) in words.contains { t.contains($0) } }
        if has(["no such file or directory"]) { return .missing }
        if has(["can't be reached", "not mounted", "isn't mounted"]) { return .share }
        if has(["damaged", "incomplete", "truncated", "ebml", "invalid data found", "moov atom not found", "end of file"]) { return .damaged }
        if has(["keyframe index pending"]) { return .preparing }
        if has(["keyframe index", "no cues"]) { return .unindexed }
        if has(["slots busy"]) { return .busy }
        // (A title the server lists but can't play from any file it finds.)
        if status == 404 { return .missing }
        if status >= 500 { return .server }
        return .unsupported
    }

    /// A title page's note for a file the server marked broken (its `broken_reason`).
    static func broken(reason: String?) -> PlaybackProblem {
        let p = classify(status: 422, text: reason ?? "")
        // A broken file is one the server couldn't read: damaged, unless the reason says otherwise.
        return p.kind == .unsupported ? PlaybackProblem(kind: .damaged, detail: p.detail) : p
    }
}

extension PlaybackProblem {
    /// Any error from asking for a stream (`MoonriseServer.offer`, `MoonriseServer.call`, a
    /// network failure).
    init(_ error: Error) {
        switch error {
        case let e as MoonriseServer.OfferError:
            switch e {
            case let .unreachable(m): self = .classify(status: nil, text: m)
            case let .notFound(m): self = .classify(status: 404, text: m)
            case let .refused(code, m): self = .classify(status: code, text: m)
            case let .unsupported(reasons, pending): self = .classify(status: 200, text: reasons.joined(separator: "; "), pending: pending)
            case .notAsAsked: self = PlaybackProblem(kind: .unsupported, detail: e.errorDescription)
            }
        case let e as MoonriseServer.ServerError:
            switch e { case let .http(code, body): self = .classify(status: code, text: body) }
        case is URLError:
            self = .classify(status: nil, text: error.localizedDescription)
        default:
            self = PlaybackProblem(kind: .unsupported, detail: error.localizedDescription)
        }
    }
}

extension MediaItem {
    /// No file of the title can play: each is damaged, or none was found on disk. (A
    /// title with a healthy version plays that one; a list item without its files says nothing.)
    var allVersionsUnplayable: Bool { unplayableProblem != nil }

    /// Why the title can't play, for its page and its player (nil: it can, or nothing says).
    var unplayableProblem: PlaybackProblem? {
        if let vs = versions, !vs.isEmpty, vs.allSatisfy(\.isBroken) {
            return .broken(reason: vs.first { $0.brokenReason != nil }?.brokenReason)
        }
        if (versions ?? []).isEmpty, let names = missingFiles, !names.isEmpty {
            let listed = names.filter { !$0.isEmpty }
            return PlaybackProblem(kind: .missing,
                                   detail: (listed.isEmpty ? "" : listed.joined(separator: "\n") + "\n") + "Not found at the server's last scan.")
        }
        return nil
    }
}
