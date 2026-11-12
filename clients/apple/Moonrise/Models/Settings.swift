import Foundation
import Observation
import SwiftUI

/// Streaming / download quality presets. `bitrate == nil` means original (direct play when possible).
struct QualityOption: Identifiable, Hashable, Codable {
    var id: String { label }
    let label: String
    let bitrate: Int?
    /// The preset's picture height (nil: Original).
    var height: Int? = nil

    static let original = QualityOption(label: "Original", bitrate: nil)
    static let presets: [QualityOption] = [
        QualityOption(label: "4K · 40 Mbps", bitrate: 40_000_000, height: 2160),
        QualityOption(label: "1080p · 20 Mbps", bitrate: 20_000_000, height: 1080),
        QualityOption(label: "1080p · 10 Mbps", bitrate: 10_000_000, height: 1080),
        QualityOption(label: "720p · 4 Mbps", bitrate: 4_000_000, height: 720),
        QualityOption(label: "480p · 1.5 Mbps", bitrate: 1_500_000, height: 480),
    ]
    static let all: [QualityOption] = [.original] + presets

    /// Original plus only the presets below the source: less bandwidth (as the server's ladder,
    /// which has no rung within 90 % of the source's bitrate) and no larger picture.
    static func options(sourceBitrate: Int?, sourceHeight: Int? = nil) -> [QualityOption] {
        [.original] + presets.filter { p in
            if let src = sourceBitrate, src > 0, Double(p.bitrate!) >= Double(src) * 0.9 { return false }
            // A 4K preset on a 1080p file is the same picture. By class, so a 1920×800 scope film
            // still counts as 1080p.
            if let h = sourceHeight, h > 0, let ph = p.height {
                let tier = h > 1080 ? 2160 : h > 720 ? 1080 : h > 480 ? 720 : 480
                if ph > tier { return false }
            }
            return true
        }
    }

    static func options(for version: MediaVersion?) -> [QualityOption] {
        options(sourceBitrate: version?.bitrate, sourceHeight: version?.videoHeight)
    }

    /// `self`, or the best option the source has when it is above the source (Original when none is).
    func capped(to version: MediaVersion?) -> QualityOption {
        let opts = Self.options(for: version)
        if opts.contains(self) { return self }
        return opts.first { ($0.bitrate ?? .max) <= (bitrate ?? .max) && $0 != .original } ?? .original
    }

    /// Approximate transfer size: real file size for Original, bitrate × runtime otherwise.
    /// `runtime` in seconds.
    func estimatedBytes(runtime: Double?, sourceSize: Int64?) -> Int64? {
        guard let bitrate else { return sourceSize }
        guard let t = runtime, t > 0 else { return nil }
        return Int64(Double(bitrate) / 8 * t)
    }

    func sizeLabel(runtime: Double?, sourceSize: Int64?) -> String? {
        estimatedBytes(runtime: runtime, sourceSize: sourceSize).map { (bitrate == nil ? "" : "≈ ") + Fmt.bytes($0) }
    }

    static func find(_ label: String) -> QualityOption {
        all.first { $0.label == label } ?? (label == original.label ? .original : presets[2])
    }
}

enum SubtitleSize: String, CaseIterable, Identifiable, Codable {
    case small, medium, large, huge
    var id: String { rawValue }
    var scale: CGFloat {
        switch self {
        case .small: 0.8
        case .medium: 1.0
        case .large: 1.25
        case .huge: 1.55
        }
    }
}

/// User preferences, persisted in UserDefaults (OpenSubtitles password in Keychain).
@MainActor
@Observable
final class AppSettings {
    static let shared = AppSettings()
    private let d = UserDefaults.standard

    var streamingQualityWiFi: String { didSet { d.set(streamingQualityWiFi, forKey: "streamingQuality") } }
    var streamingQualityCellular: String { didSet { d.set(streamingQualityCellular, forKey: "streamingQualityCell") } }
    var downloadQualityWiFi: String { didSet { d.set(downloadQualityWiFi, forKey: "downloadQuality") } }
    var downloadQualityCellular: String { didSet { d.set(downloadQualityCellular, forKey: "downloadQualityCell") } }

    /// Default for the network the device is on right now. Setting it (e.g. picking a quality in the
    /// player) updates that network's default only.
    var streamingQuality: String {
        get { NetworkType.shared.isCellular ? streamingQualityCellular : streamingQualityWiFi }
        set { if NetworkType.shared.isCellular { streamingQualityCellular = newValue } else { streamingQualityWiFi = newValue } }
    }
    var downloadQuality: String {
        NetworkType.shared.isCellular ? downloadQualityCellular : downloadQualityWiFi
    }

    /// What to do with each kind of segment, and how long auto-skip counts down first.
    var skipModes: [MediaSegment.Kind: SkipMode] {
        didSet { d.set(Dictionary(uniqueKeysWithValues: skipModes.map { ($0.key.rawValue, $0.value.rawValue) }), forKey: "skipModes") }
    }
    var autoSkipDelay: Double { didSet { d.set(autoSkipDelay, forKey: "autoSkipDelay") } }
    /// Use the community databases' segments (TheIntroDB, IntroDB), which the server looks up
    /// (sent as `community=0` when off).
    var onlineSegments: Bool { didSet { d.set(onlineSegments, forKey: "onlineSegments") } }
    func skipMode(_ kind: MediaSegment.Kind) -> SkipMode { skipModes[kind] ?? kind.defaultMode }

    /// Play the next episode (across seasons) after a countdown at the credits / end.
    var autoplayNext: Bool { didSet { d.set(autoplayNext, forKey: "autoplayNext") } }

    var preferredSubtitleLanguage: String { didSet { d.set(preferredSubtitleLanguage, forKey: "subLang") } }
    var autoSelectSubtitles: Bool { didSet { d.set(autoSelectSubtitles, forKey: "autoSubs") } }
    /// The subtitle picked last in the player was SDH (for the deaf and hard of hearing): prefer
    /// such a track in the preferred language, else a plain one.
    var preferSDHSubtitles: Bool { didSet { d.set(preferSDHSubtitles, forKey: "subSDH") } }
    /// Audio language to play when a title has it (ISO 639-1; "" = each title's own default).
    /// Picking an audio track in the player sets it.
    var preferredAudioLanguage: String { didSet { d.set(preferredAudioLanguage, forKey: "audioLang") } }
    var subtitleSize: SubtitleSize { didSet { d.set(subtitleSize.rawValue, forKey: "subSize") } }
    var subtitleBackground: Bool { didSet { d.set(subtitleBackground, forKey: "subBg") } }
    var skipForward: Double { didSet { d.set(skipForward, forKey: "skipFwd") } }
    var skipBackward: Double { didSet { d.set(skipBackward, forKey: "skipBack") } }
    var allowCellularDownloads: Bool { didSet { d.set(allowCellularDownloads, forKey: "cellDl") } }

    init() {
        streamingQualityWiFi = d.string(forKey: "streamingQuality") ?? QualityOption.original.label
        streamingQualityCellular = d.string(forKey: "streamingQualityCell") ?? "720p · 4 Mbps"
        downloadQualityWiFi = d.string(forKey: "downloadQuality") ?? "1080p · 10 Mbps"
        downloadQualityCellular = d.string(forKey: "downloadQualityCell") ?? "720p · 4 Mbps"
        let modes = d.dictionary(forKey: "skipModes") as? [String: String] ?? [:]
        skipModes = Dictionary(uniqueKeysWithValues: modes.compactMap { k, v in
            guard let kind = MediaSegment.Kind(rawValue: k), let mode = SkipMode(rawValue: v) else { return nil }
            return (kind, mode)
        })
        autoSkipDelay = d.object(forKey: "autoSkipDelay") as? Double ?? 3
        onlineSegments = d.object(forKey: "onlineSegments") as? Bool ?? true
        autoplayNext = d.object(forKey: "autoplayNext") as? Bool ?? true
        preferredSubtitleLanguage = d.string(forKey: "subLang") ?? "en"
        autoSelectSubtitles = d.object(forKey: "autoSubs") as? Bool ?? false
        preferSDHSubtitles = d.object(forKey: "subSDH") as? Bool ?? false
        preferredAudioLanguage = d.string(forKey: "audioLang") ?? ""
        subtitleSize = SubtitleSize(rawValue: d.string(forKey: "subSize") ?? "") ?? .medium
        subtitleBackground = d.object(forKey: "subBg") as? Bool ?? false
        skipForward = d.object(forKey: "skipFwd") as? Double ?? 10
        skipBackward = d.object(forKey: "skipBack") as? Double ?? 10
        allowCellularDownloads = d.object(forKey: "cellDl") as? Bool ?? false
        Self.forgetDeviceKeys(d)
    }

    /// The third-party keys older versions kept on the device (an OMDb key, an OpenSubtitles
    /// account): the server does those lookups now, so they are removed.
    static func forgetDeviceKeys(_ d: UserDefaults) {
        for key in ["omdbKey", "osApiKey", "osUser"] where d.object(forKey: key) != nil { d.removeObject(forKey: key) }
        if Keychain.get("opensubtitles.password") != nil { Keychain.set(nil, for: "opensubtitles.password") }
    }
}

/// Track languages compared as ISO 639-1 codes: servers and playlists say "eng", "ger" or
/// "en-US", settings store "en".
enum LanguageCode {
    /// ISO 639-2/B codes that differ from the terminology ones Locale knows.
    private static let bibliographic = [
        "ger": "de", "fre": "fr", "dut": "nl", "chi": "zh", "cze": "cs", "gre": "el", "per": "fa", "rum": "ro",
        "slo": "sk", "ice": "is", "arm": "hy", "baq": "eu", "bur": "my", "geo": "ka", "mac": "mk", "mao": "mi",
        "may": "ms", "alb": "sq", "tib": "bo", "wel": "cy",
    ]

    /// nil for unknown ("und", "mul", none).
    static func normalized(_ value: String?) -> String? {
        guard var s = value?.trimmingCharacters(in: .whitespaces).lowercased(), !s.isEmpty else { return nil }
        if let cut = s.firstIndex(where: { $0 == "-" || $0 == "_" }) { s = String(s[..<cut]) }
        if ["und", "mul", "zxx", "mis", "qaa"].contains(s) { return nil }
        if let b = bibliographic[s] { return b }
        if s.count == 3, let two = Locale.LanguageCode(s).identifier(.alpha2) { return two }
        return s
    }

    /// The language's name in the device's language ("French").
    static func name(_ code: String) -> String {
        Locale.current.localizedString(forLanguageCode: code) ?? code.uppercased()
    }

    /// Settings' language choices: the common ones, plus `current` when it isn't one of them.
    static func choices(including current: String) -> [(String, String)] {
        let common = SubtitleSearchView.languages
        guard !current.isEmpty, !common.contains(where: { $0.0 == current }) else { return common }
        return common + [(current, name(current))]
    }
}

/// Local resume positions (offline copies, and a fallback while the server can't be reached).
enum ProgressStore {
    private static let d = UserDefaults.standard

    static func position(for key: String) -> Double? {
        let v = d.double(forKey: "pos." + key)
        return v > 0 ? v : nil
    }

    /// The key of a server title.
    static func key(itemId: String) -> String { "moonrise:" + itemId }

    static func save(_ seconds: Double, duration: Double, for key: String) {
        // Treat the last 5% as finished.
        if duration > 0, seconds / duration > 0.95 || seconds < 10 {
            d.removeObject(forKey: "pos." + key)
        } else {
            d.set(seconds, forKey: "pos." + key)
        }
    }
}
