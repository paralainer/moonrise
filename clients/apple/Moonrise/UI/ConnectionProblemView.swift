import SwiftUI

/// Selected tab, shared so screens can send the user elsewhere (e.g. to Downloads when offline).
@MainActor
@Observable
final class AppNavigation {
    static let shared = AppNavigation()
    var tab = 0
    static let downloadsTab = 3
    /// Zoom id ("<row id>/<item id>") of the card the current title page was opened from.
    @ObservationIgnored var lastOpened: String?
    /// Home is covered by a pushed page or another tab. (With zoom transitions the source screen
    /// never gets onDisappear, so this is derived from navigation state instead.)
    var homeCovered = false
    /// A page to push on Home (`RootView` takes it).
    var pendingRoute: Route?
    /// Apple TV: the item whose page a deep link (the Top Shelf) just opened. That page takes focus
    /// itself, since on a cold launch focus is still on the (hidden) tab bar.
    @ObservationIgnored var focusOnOpen: String?
    /// The Library tab's pushed pages.
    var libraryPath = NavigationPath()
    /// The Downloads tab's pushed pages (a download's title page).
    var downloadsPath = NavigationPath()
    /// The phone: the Library page on top of its pushed pages (a category, or a tile's titles).
    @ObservationIgnored var libraryPage: PadLibraryPage?
    /// The Library's page beside its sidebar (iPad; the iPhone in landscape).
    var libraryPadPage: PadLibraryPage = .grid
}

extension Error {
    /// The request was abandoned (view went away, a newer load replaced it): not a real failure.
    var isCancellation: Bool {
        if self is CancellationError { return true }
        if let u = self as? URLError, u.code == .cancelled { return true }
        return (self as NSError).domain == NSURLErrorDomain && (self as NSError).code == NSURLErrorCancelled
    }

    /// Couldn't talk to the server at all (as opposed to the server answering with an error).
    var isConnectivity: Bool {
        guard let u = self as? URLError else { return false }
        return [.notConnectedToInternet, .timedOut, .cannotConnectToHost, .networkConnectionLost, .cannotFindHost,
                .dnsLookupFailed, .dataNotAllowed, .internationalRoamingOff, .secureConnectionFailed].contains(u.code)
    }

    /// Short, human message for anything that isn't a connectivity problem.
    var friendlyMessage: String {
        let status: Int? = if case let .http(c, _)? = self as? MoonriseClientError { c } else { nil }
        if let code = status {
            switch code {
            case 401, 403: return "Your sign-in has expired. Sign in again in Settings."
            case 500...599: return "The server ran into a problem (\(code)). Try again in a moment."
            default: return "The server answered with an error (\(code))."
            }
        }
        return localizedDescription
    }
}

/// Explains why the server can't be reached and offers the fix: Tailscale, Wi-Fi, Downloads or retry.
struct ConnectionProblemView: View {
    @Environment(\.metrics) private var metrics
    let problem: ConnectionRouter.Problem
    var hasSavedContent = false
    let retry: () -> Void

    @Environment(\.openURL) private var openURL
    @State private var retrying = false
    @State private var showGuide = false
    private var router: ConnectionRouter { ConnectionRouter.shared }
    private var name: String { SourceStore.shared.server?.name ?? "Your Server" }

    var body: some View {
        VStack(spacing: 14) {
            Image(systemName: icon)
                .font(.system(size: Self.iconSize, weight: .semibold))
                .symbolRenderingMode(.hierarchical)
                .frame(height: Self.iconSize + 6)
            VStack(spacing: 6) {
                Text(title).font(.headline)
                Text(message)
                    .font(.subheadline)
                    .foregroundStyle(Theme.secondaryText)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
            }
            VStack(spacing: 10) {
                actions
            }
            .padding(.top, 4)
        }
        #if !os(tvOS)
        .foregroundStyle(.white)
        #endif
        .padding(Self.inset)
        .frame(maxWidth: Self.width)
        .glassEffect(.regular, in: .rect(cornerRadius: Self.inset + 6))
        .padding(.horizontal, metrics.hPadding)
        .frame(maxWidth: .infinity)
        #if !os(tvOS)
        .sheet(isPresented: $showGuide) { RemoteAccessGuide() }
        #endif
    }

    // Apple TV: a card wide enough for the title on one line and the message in a few lines,
    // with system buttons (their focus is the white platter).
    #if os(tvOS)
    private static let width: CGFloat = 760
    private static let inset: CGFloat = 44
    private static let iconSize: CGFloat = 56
    #else
    private static let width: CGFloat = 460
    private static let inset: CGFloat = 22
    private static let iconSize: CGFloat = 34
    #endif

    // MARK: Content

    private var icon: String {
        switch problem {
        case .offline: "wifi.slash"
        case .tailscaleOff, .tailscaleNeedsLogin, .tailscaleFailed: "network.slash"
        case .tailscaleStarting: "network"
        case .serverDown: "server.rack"
        }
    }

    private var title: String {
        switch problem {
        case .offline: "You're Offline"
        case .tailscaleOff, .tailscaleNeedsLogin, .tailscaleFailed: "Can't Reach \(name)"
        case .tailscaleStarting: "Connecting to \(name)…"
        case .serverDown: "\(name) Isn't Responding"
        }
    }

    private var message: String {
        let saved = hasSavedContent ? " Showing what was loaded last time." : ""
        switch problem {
        case .offline:
            return "Turn on Wi-Fi or mobile data in Control Center to reach your library. Downloads still play." + saved
        case .tailscaleOff:
            return (router.onCellular
                ? "You're on mobile data. Join your home Wi-Fi, or connect through Tailscale to reach your library from anywhere."
                : "Moonrise reaches \(name) on your home network. Away from home? Connect through Tailscale to watch from anywhere.") + saved
        case .tailscaleNeedsLogin:
            return "Tailscale needs you to sign in before Moonrise can reach your home server." + saved
        case let .tailscaleFailed(msg):
            return "Tailscale couldn't start: \(msg)" + saved
        case .tailscaleStarting:
            return "Joining your tailnet."
        case .serverDown:
            return (TailscaleService.isSupported
                ? "It didn't answer on your home network or over Tailscale. The server or its computer may be off or asleep."
                : "It didn't answer on your home network. The server or its computer may be off or asleep.") + saved
        }
    }

    @ViewBuilder private var actions: some View {
        switch problem {
        // iOS has no public way to open Wi-Fi settings (the old App-Prefs URL now lands on the
        // Apps list), so the copy points to Control Center instead of offering a button.
        case .offline:
            downloadsButton
            retryButton
        case .tailscaleOff:
            primary("Connect with Tailscale", "lock.shield") { TailscaleService.shared.enabled = true }
            secondary("Set Up Remote Access", "questionmark.circle") { showGuide = true }
            downloadsButton
        case let .tailscaleNeedsLogin(url):
            if let url {
                primary("Sign In to Tailscale", "person.badge.key") { openURL(url) }
            } else {
                ProgressView().tint(.white)
            }
            downloadsButton
        case .tailscaleFailed:
            primary("Restart Tailscale", "arrow.clockwise") {
                let ts = TailscaleService.shared
                ts.stop(); ts.start()
            }
            downloadsButton
        case .tailscaleStarting:
            ProgressView().tint(.white)
        case .serverDown:
            retryButton
            downloadsButton
        }
    }

    @ViewBuilder private var downloadsButton: some View {
        if DownloadManager.isSupported, !DownloadManager.shared.records.isEmpty {
            secondary("Go to Downloads", "arrow.down.circle") { AppNavigation.shared.tab = AppNavigation.downloadsTab }
        }
    }

    private var retryButton: some View {
        Button {
            retrying = true
            Task {
                await router.refresh()
                retry()
                retrying = false
            }
        } label: {
            Group {
                if retrying { ProgressView().tint(.white) } else { Label("Try Again", systemImage: "arrow.clockwise") }
            }
            .frame(maxWidth: .infinity, minHeight: 44)
        }
        .problemButton()
        .disabled(retrying)
    }

    private func primary(_ title: String, _ icon: String, action: @escaping () -> Void) -> some View {
        #if os(tvOS)
        Button(action: action) {
            Label(title, systemImage: icon).font(.body.weight(.semibold)).frame(maxWidth: .infinity)
        }
        .tvGlassButton(prominent: true)
        #else
        Button(action: action) {
            Label(title, systemImage: icon)
                .font(.body.weight(.semibold))
                .foregroundStyle(.black)
                .frame(maxWidth: .infinity, minHeight: 44)
                .background(.white, in: Capsule())
                .contentShape(Capsule())
        }
        .plainButton()
        #endif
    }

    private func secondary(_ title: String, _ icon: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Label(title, systemImage: icon).frame(maxWidth: .infinity, minHeight: 44)
        }
        .problemButton()
    }
}

private extension View {
    /// Glass capsule (tvOS: Moonrise's glass button).
    @ViewBuilder func problemButton() -> some View {
        #if os(tvOS)
        self.tvGlassButton()
        #else
        self.plainButton().glassEffect(.regular.interactive(), in: .capsule)
        #endif
    }
}

#if !os(tvOS)
/// How remote access works, in three steps: the server's computer and this device join the
/// same Tailscale network; at home Moonrise uses the home network, away it goes through Tailscale.
struct RemoteAccessGuide: View {
    @Environment(\.dismiss) private var dismiss
    private var name: String { SourceStore.shared.server?.name ?? "your server" }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    Text("Moonrise talks to \(name) directly, and nothing on your network is opened to the internet. To reach it away from home, put both on your own private network with Tailscale, free for personal use.")
                        .fixedSize(horizontal: false, vertical: true)
                }
                Section("Three steps") {
                    step(1, "On the computer that runs Moonrise Server, install Tailscale from tailscale.com/download and sign in.")
                    step(2, "Here, tap Connect with Tailscale and sign in with the same account.")
                    step(3, "Done. At home Moonrise uses your Wi-Fi; away, it goes through Tailscale on its own.")
                }
                Section {
                    Button {
                        TailscaleService.shared.enabled = true
                        dismiss()
                    } label: {
                        Label("Connect with Tailscale", systemImage: "lock.shield")
                    }
                } footer: {
                    Text("Without it, Moonrise works on your home network, and downloads play anywhere.")
                }
            }
            .navigationTitle("Watch Away from Home")
            .inlineTitle()
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
        }
        #if os(macOS)
        .frame(minWidth: 460, minHeight: 420)
        #endif
    }

    private func step(_ n: Int, _ text: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text("\(n)")
                .font(.subheadline.weight(.bold))
                .frame(width: 24, height: 24)
                .background(Circle().fill(Theme.accent.opacity(0.25)))
            Text(text).fixedSize(horizontal: false, vertical: true)
        }
        .padding(.vertical, 2)
    }
}
#endif
