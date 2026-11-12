import Foundation

extension PlayerModel {
    /// What the overlay draws.
    var overlayText: String? { subtitlesOnReceiver || recovery != .none ? nil : subtitleText }

    /// The server this plays from (a download: the one it came from), for downloading other episodes.
    var source: Source? {
        switch request.target {
        case let .server(source, _): source
        }
    }

    /// The version playing (its id), for the Version menu.
    var playingVersionId: String? { mediaSource?.id }
    /// The id the server's media addresses take: the title, or the version playing
    /// (`<item>~<file>`), so subtitles and searches are that file's.
    var mediaId: String {
        guard let item else { return "" }
        guard versions.count > 1, let v = mediaSource?.id ?? wantedVersion else { return item.id }
        return "\(item.id)~\(v)"
    }

    var isRecovering: Bool { recovery == .recovering }
    var streamFailure: StreamFailure? {
        if case let .failed(f) = recovery { return f }
        return nil
    }

    /// "4K HDR10 · Director's Cut" for a version (the server's label, else its file's details).
    static func versionTitle(_ v: MediaVersion) -> String {
        v.label ?? v.name ?? "Version"
    }

    /// A version's second line in menus: its details, or that it can't play (damaged).
    static func versionDetails(_ v: MediaVersion) -> String? {
        v.isBroken ? "Damaged or incomplete file · can't play" : v.details
    }

    var qualityOptions: [QualityOption] { QualityOption.options(for: mediaSource) }

    func sizeLabel(_ q: QualityOption) -> String? {
        q.sizeLabel(runtime: mediaSource?.runtime ?? item?.runtime, sourceSize: mediaSource?.size)
    }
}
