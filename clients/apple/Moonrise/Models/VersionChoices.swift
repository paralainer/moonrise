import AVFoundation
import Foundation

/// Versions of a title (one title, several files: cuts, qualities, dubs). Which one plays: the
/// one the user chose (remembered by the server for their account, so every device plays it;
/// picks made here show at once), else the best for this device: one in the preferred audio
/// language when the versions differ in it, an SDR one where this display can't show HDR and
/// there's one as sharp, else the server's order (its first is its default). A version the server
/// found damaged is never the one, chosen or not, while the title has another.
@MainActor
@Observable
final class VersionChoices {
    static let shared = VersionChoices()

    /// Picks made on this device since launch (item id → file id), ahead of the server's answer.
    private(set) var picks: [String: String] = [:]

    func set(_ fileId: String, for itemId: String) { picks[itemId] = fileId }

    /// The version that plays for `item` (nil: it has no versions listed).
    func version(for item: MediaItem) -> MediaVersion? {
        guard let vs = item.versions, !vs.isEmpty else { return nil }
        let playable = Self.playable(vs)
        if let id = picks[item.id] ?? item.chosenVersionId, let v = playable.first(where: { $0.id == id }) { return v }
        return Self.best(vs)
    }

    /// The versions that can play: the healthy ones (all of them when none is).
    static func playable(_ vs: [MediaVersion]) -> [MediaVersion] {
        let healthy = vs.filter { !$0.isBroken }
        return healthy.isEmpty ? vs : healthy
    }

    /// The best version for this device (see the type's note).
    static func best(_ vs: [MediaVersion], hdrDisplay: Bool? = nil, language: String?? = nil) -> MediaVersion? {
        let hdrDisplay = hdrDisplay ?? AVPlayer.eligibleForHDRPlayback
        let language = language ?? (LanguageCode.normalized(AppSettings.shared.preferredAudioLanguage)
            ?? Locale.preferredLanguages.lazy.compactMap { LanguageCode.normalized($0) }.first)
        var pool = playable(vs)
        // The preferred language, where the versions differ in it (the default track's first).
        if let want = language {
            let speaks = { (v: MediaVersion) in (v.audioLanguages ?? []).contains { LanguageCode.normalized($0) == want } }
            let first = pool.filter { LanguageCode.normalized($0.audioLanguages?.first) == want }
            let any = pool.filter(speaks)
            if !first.isEmpty, first.count < pool.count { pool = first } else if !any.isEmpty, any.count < pool.count { pool = any }
        }
        // HDR only where it can be shown, when an SDR version is as sharp.
        if !hdrDisplay {
            let height = { (v: MediaVersion) in v.videoHeight ?? 0 }
            let top = pool.map(height).max() ?? 0
            if let sdr = pool.first(where: { $0.hdr == nil && height($0) >= top }) { return sdr }
        }
        return pool.first
    }
}
