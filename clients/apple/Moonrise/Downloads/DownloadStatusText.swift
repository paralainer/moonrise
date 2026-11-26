import Foundation

/// What a download's row, badge and title-page line say, from its state and why the server
/// can't be reached (`ConnectionRouter.problem`). A download that can't move says "waiting" and
/// why, never "Downloading N %".
enum DownloadStatusText {
    /// Why a running download isn't moving, or nil while it is.
    /// `error`: what the download itself last said ("Waiting for Wi-Fi…", "Waiting for the server…").
    /// `problem`: why its server can't be reached right now (nil: it can, or it is another server's).
    static func waitReason(state: DownloadRecord.State, error: String?,
                           problem: ConnectionRouter.Problem?, serverName: String) -> String? {
        guard state == .downloading else { return nil }
        // The cellular rule holds it whatever the route: say so, it is the user's setting.
        if error == SegmentedDownloads.waitingForWiFiText { return error }
        if let problem { return reason(problem, serverName: serverName) }
        return error
    }

    /// A short reason, for a row or an info line.
    static func reason(_ problem: ConnectionRouter.Problem, serverName: String) -> String {
        switch problem {
        case .offline: "You're offline"
        case .tailscaleOff: "Tailscale is off"
        case .tailscaleNeedsLogin: "Tailscale needs you to sign in"
        case .tailscaleStarting: "Connecting to \(serverName)…"
        case .tailscaleFailed: "Tailscale couldn't start"
        case .serverDown: "Can't reach \(serverName)"
        }
    }

    /// The badge a title's download shows on cards and pages.
    static func badge(state: DownloadRecord.State, progress: Double, waitReason: String?) -> DownloadManager.Badge {
        let p = Int(progress * 100)
        return switch state {
        case .completed: .downloaded
        case .downloading: waitReason.map { .waiting(p, $0) } ?? .downloading(p)
        case .queued: .queued
        case .paused: .paused
        case .failed: .failed
        }
    }

    /// A title's info line (Home's banner, title pages).
    static func tag(_ badge: DownloadManager.Badge) -> String? {
        switch badge {
        case .downloaded: "Downloaded"
        case let .downloading(p): "Downloading \(p) %"
        case let .waiting(_, reason): "Download waiting: \(reason)"
        case .queued, .paused: "Download waiting"
        case .failed: nil
        }
    }

    /// Under a title page's Play row.
    static func pageLine(state: DownloadRecord.State, progress: Double, error: String?,
                         waitReason: String?, downloadLabel: String) -> String {
        let p = Int(progress * 100)
        return switch state {
        case .completed: "Downloaded · \(downloadLabel)"
        case .downloading: waitReason.map { "Download waiting at \(p) %: \($0)" } ?? "Downloading \(p) %"
        case .queued: "Download waiting"
        case .paused: "Download paused at \(p) %"
        case .failed: "Download failed: \(error ?? "")"
        }
    }

    /// Home's banner while the server can't be reached and the downloads are shown under it (with
    /// ": showing downloads" after the server's name it no longer fits an iPhone's width).
    static func offlineBanner(isOnline: Bool, serverName: String) -> String {
        isOnline ? "Can't reach \(serverName)" : "Offline: showing downloads"
    }
}
