import Foundation

struct SubtitleCue: Hashable {
    let start: Double
    let end: Double
    let text: String
}

/// Parsed external subtitle track, rendered by `SubtitleOverlay`.
struct SubtitleDocument {
    let cues: [SubtitleCue]

    /// The next time after `t` where the visible text changes (a cue starts or ends).
    func nextEdge(after t: Double) -> Double? {
        var lo = 0, hi = cues.count
        while lo < hi {
            let mid = (lo + hi) / 2
            if cues[mid].start <= t { lo = mid + 1 } else { hi = mid }
        }
        var edge = lo < cues.count ? cues[lo].start : nil
        for c in cues[max(0, lo - 5)..<lo] where c.end > t { edge = min(edge ?? c.end, c.end) }
        return edge
    }

    /// Binary search for the cue(s) visible at `t`.
    func text(at t: Double) -> String? {
        var lo = 0, hi = cues.count - 1, found = -1
        while lo <= hi {
            let mid = (lo + hi) / 2
            if cues[mid].start <= t { found = mid; lo = mid + 1 } else { hi = mid - 1 }
        }
        guard found >= 0 else { return nil }
        // Overlapping cues: walk back a little to collect everything active.
        var lines: [String] = []
        var i = found
        while i >= 0, i >= found - 4 {
            let c = cues[i]
            if c.start <= t, t < c.end { lines.insert(c.text, at: 0) }
            i -= 1
        }
        return lines.isEmpty ? nil : lines.joined(separator: "\n")
    }
}

enum SubtitleParser {
    static func parse(_ data: Data, ext: String? = nil) -> SubtitleDocument {
        let text = decode(data)
        let lower = ext?.lowercased()
        if lower == "ass" || lower == "ssa" || text.contains("[Script Info]") {
            return SubtitleDocument(cues: parseASS(text))
        }
        return SubtitleDocument(cues: parseSRTorVTT(text))
    }

    private static func decode(_ data: Data) -> String {
        if let s = String(data: data, encoding: .utf8) { return s }
        if let s = String(data: data, encoding: .windowsCP1252) { return s }
        return String(decoding: data, as: UTF8.self)
    }

    /// SRT and WebVTT share the "start --> end" block structure.
    static func parseSRTorVTT(_ raw: String) -> [SubtitleCue] {
        let normalized = raw.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\r", with: "\n")
        var cues: [SubtitleCue] = []
        for block in normalized.components(separatedBy: "\n\n") {
            let lines = block.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
            guard let ti = lines.firstIndex(where: { $0.contains("-->") }) else { continue }
            let parts = lines[ti].components(separatedBy: "-->")
            guard parts.count == 2,
                  let s = timestamp(parts[0]),
                  let e = timestamp(parts[1].trimmingCharacters(in: .whitespaces).components(separatedBy: " ")[0])
            else { continue }
            let body = lines[(ti + 1)...].joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
            guard !body.isEmpty else { continue }
            cues.append(SubtitleCue(start: s, end: e, text: stripTags(body)))
        }
        return cues.sorted { $0.start < $1.start }
    }

    /// Accepts "01:02:03,456", "01:02:03.456" and "02:03.456".
    static func timestamp(_ s: String) -> Double? {
        let t = s.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: ",", with: ".")
        let comps = t.split(separator: ":")
        guard comps.count >= 2 else { return nil }
        var total = 0.0
        for c in comps {
            guard let v = Double(c) else { return nil }
            total = total * 60 + v
        }
        return total
    }

    static func stripTags(_ s: String) -> String {
        var out = s.replacingOccurrences(of: "<[^>]+>", with: "", options: .regularExpression)
        out = out.replacingOccurrences(of: "\\{[^}]*\\}", with: "", options: .regularExpression)
        out = out.replacingOccurrences(of: "&amp;", with: "&")
            .replacingOccurrences(of: "&lt;", with: "<")
            .replacingOccurrences(of: "&gt;", with: ">")
            .replacingOccurrences(of: "&nbsp;", with: " ")
        return out
    }

    /// Minimal Advanced SubStation parser: dialogue text only, styling dropped.
    static func parseASS(_ raw: String) -> [SubtitleCue] {
        var format: [String] = []
        var cues: [SubtitleCue] = []
        var inEvents = false
        for line in raw.components(separatedBy: .newlines) {
            let l = line.trimmingCharacters(in: .whitespaces)
            if l.hasPrefix("[") { inEvents = l.lowercased() == "[events]"; continue }
            guard inEvents else { continue }
            if l.hasPrefix("Format:") {
                format = l.dropFirst(7).split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces).lowercased() }
            } else if l.hasPrefix("Dialogue:"), !format.isEmpty {
                let body = String(l.dropFirst(9))
                let fields = body.split(separator: ",", maxSplits: format.count - 1, omittingEmptySubsequences: false).map(String.init)
                guard fields.count == format.count,
                      let si = format.firstIndex(of: "start"), let ei = format.firstIndex(of: "end"),
                      let ti = format.firstIndex(of: "text"),
                      let s = timestamp(fields[si]), let e = timestamp(fields[ei]) else { continue }
                let text = stripTags(fields[ti])
                    .replacingOccurrences(of: "\\N", with: "\n")
                    .replacingOccurrences(of: "\\n", with: "\n")
                    .replacingOccurrences(of: "\\h", with: " ")
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                if !text.isEmpty { cues.append(SubtitleCue(start: s, end: e, text: text)) }
            }
        }
        return cues.sorted { $0.start < $1.start }
    }
}

/// A subtitle choice offered in the player menu.
struct SubtitleOption: Identifiable, Hashable {
    enum Kind: Hashable {
        /// Fetch the file and render it with our overlay.
        case external(URL)
        /// A picture track (PGS, DVD, DVB): its images come from the server in windows and are
        /// drawn over the video.
        case pictures(index: Int)
        /// Embedded track handled by the engine itself.
        case engine(Int)
    }

    let id: String
    let title: String
    let language: String?
    let kind: Kind
    /// The server's index of the track (nil for files added on this device).
    var trackIndex: Int?

    /// Menu title (the language) and secondary line (SDH, Forced, External…), split from `title`.
    var name: String { title.components(separatedBy: " · ").first ?? title }
    var detail: String? {
        let rest = title.components(separatedBy: " · ").dropFirst()
        return rest.isEmpty ? nil : rest.joined(separator: " · ")
    }
}
