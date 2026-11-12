import Foundation

/// A skippable stretch of an episode/movie (intro, recap, credits…), from the server (named
/// chapters) or the community databases.
struct MediaSegment: Codable, Hashable, Identifiable {
    enum Kind: String, Codable, CaseIterable, Identifiable {
        case intro, recap, credits, preview, commercial
        var id: String { rawValue }
        var title: String {
            switch self {
            case .intro: "Intro"
            case .recap: "Recap"
            case .credits: "Credits"
            case .preview: "Preview"
            case .commercial: "Commercial"
            }
        }
        var skipLabel: String { "Skip \(title)" }
        /// Default behaviour: auto-skip everything except credits (skipping those just ends the video).
        var defaultMode: SkipMode { self == .credits ? .button : .auto }

        /// Chapter names commonly used for these parts, after a leading language tag
        /// ("en:End Credits", as the server reads them).
        init?(chapterName: String) {
            var name = Substring(chapterName)
            if let colon = name.firstIndex(of: ":"), (2...3).contains(name.distance(from: name.startIndex, to: colon)),
               name[..<colon].allSatisfy({ $0.isASCII && $0.isLetter }) {
                name = name[name.index(after: colon)...].drop(while: { $0 == " " })
            }
            let n = name.lowercased()
            if ["intro", "opening", "op", "title sequence", "opening credits", "theme song"].contains(where: { n == $0 || n.hasPrefix($0 + " ") }) { self = .intro }
            else if n.contains("recap") || n.hasPrefix("previously") { self = .recap }
            else if ["credits", "end credits", "ending", "ed", "outro", "closing credits"].contains(where: { n == $0 || n.hasPrefix($0 + " ") }) { self = .credits }
            else if n.contains("preview") || n.contains("next time") { self = .preview }
            else { return nil }
        }
    }

    let kind: Kind
    let start: Double
    let end: Double
    /// Where it came from, when that matters: "detected" (the server found the end credits in
    /// the file itself), else nil.
    var source: String? = nil
    var id: String { "\(kind.rawValue)-\(Int(start))" }
}

enum SkipMode: String, Codable, CaseIterable, Identifiable {
    case auto, button, off
    var id: String { rawValue }
    var title: String {
        switch self {
        case .auto: "Skip Automatically"
        case .button: "Show Skip Button"
        case .off: "Off"
        }
    }
}
