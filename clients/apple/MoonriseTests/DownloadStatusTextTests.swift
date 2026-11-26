// What a download's row, badge and title-page line say while its server can't be reached
// (Moonrise/Downloads/DownloadStatusText.swift), and Home's banner while only the server is down.
import Foundation
import Testing
@testable import Moonrise

@MainActor
struct DownloadStatusTextTests {
    typealias S = DownloadStatusText

    private func reason(_ error: String?, _ problem: ConnectionRouter.Problem?,
                        state: DownloadRecord.State = .downloading) -> String? {
        S.waitReason(state: state, error: error, problem: problem, serverName: "Den")
    }

    @Test func aDownloadWaitingWhileTailscaleIsOffSaysSo() {
        // The download only knows the server didn't answer; the router knows why.
        #expect(reason("Waiting for the server…", .tailscaleOff) == "Tailscale is off")
        #expect(reason("Waiting for the server…", .serverDown) == "Can't reach Den")
        #expect(reason("Waiting for the server…", .offline) == "You're offline")
        // Before the download's own second failure, the route already tells it waits.
        #expect(reason(nil, .tailscaleOff) == "Tailscale is off")
    }

    @Test func theCellularRuleAndTheDownloadsOwnWordsStay() {
        #expect(reason(SegmentedDownloads.waitingForWiFiText, .tailscaleOff) == "Waiting for Wi-Fi…")
        #expect(reason(SegmentedDownloads.waitingForWiFiText, nil) == "Waiting for Wi-Fi…")
        // The route is fine but the server keeps failing (busy, a dropped link).
        #expect(reason("Waiting for the server…", nil) == "Waiting for the server…")
        #expect(reason(nil, nil) == nil)
    }

    @Test func onlyARunningDownloadWaits() {
        #expect(reason(nil, .serverDown, state: .paused) == nil)
        #expect(reason(nil, .serverDown, state: .completed) == nil)
        #expect(reason(nil, .serverDown, state: .queued) == nil)
    }

    @Test func aWaitingDownloadIsNotShownAsDownloading() {
        let b = S.badge(state: .downloading, progress: 0.19, waitReason: "Tailscale is off")
        #expect(b == .waiting(19, "Tailscale is off"))
        #expect(S.tag(b) == "Download waiting: Tailscale is off")
        #expect(S.pageLine(state: .downloading, progress: 0.19, error: "Waiting for the server…",
                              waitReason: "Tailscale is off", downloadLabel: "480p") == "Download waiting at 19 %: Tailscale is off")
        // Moving: as before.
        let moving = S.badge(state: .downloading, progress: 0.19, waitReason: nil)
        #expect(moving == .downloading(19))
        #expect(S.tag(moving) == "Downloading 19 %")
        #expect(S.pageLine(state: .downloading, progress: 0.19, error: nil, waitReason: nil, downloadLabel: "480p")
                == "Downloading 19 %")
    }

    @Test func homeSaysTheServerCantBeReachedUnlessTheDeviceIsOffline() {
        #expect(S.offlineBanner(isOnline: true, serverName: "Den") == "Can't reach Den")
        #expect(S.offlineBanner(isOnline: false, serverName: "Den") == "Offline: showing downloads")
    }
}
