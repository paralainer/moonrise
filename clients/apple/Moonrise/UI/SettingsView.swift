import SwiftUI

struct SettingsView: View {
    @Bindable private var s = AppSettings.shared
    @State private var manageSeam = false

    var body: some View {
        #if os(tvOS)
        // Apple TV Settings layout: identity on the left, the list on the right. The list is the
        // screen's scroll view (full width, its rows kept right of the sidebar), so the tab bar
        // scrolls away with it instead of floating over the rows; the sidebar stays put.
        form
            // Rows lift on glass when focused, like the rest of the app's buttons.
            .buttonStyle(TVGlassRowStyle(cornerRadius: 40, outset: CGSize(width: 20, height: 14), scale: 1.02))
            // The safe area already keeps 80 pt on each side; the sidebar takes 520 + 80.
            .safeAreaPadding(.leading, 600)
            .overlay(alignment: .leading) {
                // On the content edge (the overlay is already inside the safe area), 80 pt
                // clear of the list.
                TVSettingsSidebar()
                    .frame(width: 520)
                    .allowsHitTesting(false)
            }
            .toolbar(.hidden, for: .navigationBar)
        #else
        form
            .navigationTitle("Settings")
        #endif
    }

    private var form: some View {
        Form { Group {
            ServerSection(manage: $manageSeam)

            if TailscaleService.isSupported, !SourceStore.shared.isDemo { TailscaleSection() }

            // Everything below is kept on this device. Server settings (libraries, accounts, the
            // server's services, transcoding, preparing files) are only in Manage Server, for admins.
            Section {
                Picker(NetworkType.hasCellular ? "Streaming on Wi-Fi" : "Streaming Quality", selection: $s.streamingQualityWiFi) {
                    ForEach(QualityOption.all) { Text($0.label).tag($0.label) }
                }
                if NetworkType.hasCellular {
                    Picker("Streaming on Cellular", selection: $s.streamingQualityCellular) {
                        ForEach(QualityOption.all) { Text($0.label).tag($0.label) }
                    }
                }
                Picker("Skip Back", selection: $s.skipBackward) {
                    ForEach([5.0, 10, 15, 30], id: \.self) { Text("\(Int($0)) s").tag($0) }
                }
                Picker("Skip Forward", selection: $s.skipForward) {
                    ForEach([10.0, 15, 30, 45, 60], id: \.self) { Text("\(Int($0)) s").tag($0) }
                }
                Toggle("Play Next Episode Automatically", isOn: $s.autoplayNext).switchTint()
            } header: {
                Text("Playback")
            } footer: {
                Text("Titles start at this quality; you can pick another in the player.")
            }

            Section {
                ForEach(MediaSegment.Kind.allCases) { kind in
                    Picker(kind.title, selection: Binding(get: { s.skipMode(kind) }, set: { s.skipModes[kind] = $0 })) {
                        ForEach(SkipMode.allCases) { Text($0.title).tag($0) }
                    }
                }
                Picker("Auto-Skip Countdown", selection: $s.autoSkipDelay) {
                    Text("None").tag(0.0)
                    ForEach([3.0, 5, 10], id: \.self) { Text("\(Int($0)) s").tag($0) }
                }
                Toggle("Use Online Databases", isOn: $s.onlineSegments).switchTint()
            } header: {
                Text("Skip Intros & Credits")
            } footer: {
                Text("Markers come from your server: the files' chapter names and the end credits it finds in them, and, with Online Databases on, the community databases TheIntroDB and IntroDB, which your server looks up. This device asks no one else.")
            }

            Section {
                Picker("Preferred Language", selection: $s.preferredAudioLanguage) {
                    Text("Original").tag("")
                    ForEach(LanguageCode.choices(including: s.preferredAudioLanguage), id: \.0) { Text($0.1).tag($0.0) }
                }
            } header: {
                Text("Audio")
            } footer: {
                Text("Titles play in this language when they have it. Picking a language or subtitles in the player sets these.")
            }

            Section {
                Picker("Preferred Language", selection: $s.preferredSubtitleLanguage) {
                    ForEach(LanguageCode.choices(including: s.preferredSubtitleLanguage), id: \.0) { Text($0.1).tag($0.0) }
                }
                Toggle("Turn On Automatically", isOn: $s.autoSelectSubtitles).switchTint()
                Picker("Size", selection: $s.subtitleSize) {
                    ForEach(SubtitleSize.allCases) { Text($0.rawValue.capitalized).tag($0) }
                }
                Toggle("Background Box", isOn: $s.subtitleBackground).switchTint()
            } header: {
                Text("Subtitles")
            } footer: {
                Text("How subtitles look on this device.")
            }

            if DownloadManager.isSupported {
                Section {
                    Picker(NetworkType.hasCellular ? "Quality on Wi-Fi" : "Default Quality", selection: $s.downloadQualityWiFi) {
                        ForEach(QualityOption.all) { Text($0.label).tag($0.label) }
                    }
                    if NetworkType.hasCellular {
                        Picker("Quality on Cellular", selection: $s.downloadQualityCellular) {
                            ForEach(QualityOption.all) { Text($0.label).tag($0.label) }
                        }
                    }
                    Toggle("Download over Cellular", isOn: $s.allowCellularDownloads).switchTint()
                } header: {
                    Text("Downloads")
                } footer: {
                    Text("Downloads play as soon as they start. Lower qualities are transcoded by your server.")
                }
            }

            #if !os(tvOS)
            // Apple TV: the version is in the sidebar.
            Section {
                LabeledContent("Version", value: AppInfo.versionLabel)
            } footer: {
                Text(ExternalRatings.attribution)
            }
            #endif
        }.tvGlassRows() }
        .formStyle(.grouped)
        .navigationDestination(isPresented: $manageSeam) { ManageServerView() }
    }
}

#if os(tvOS)
/// Left pane of the Apple TV Settings screen: app mark, version and connection status.
private struct TVSettingsSidebar: View {
    private var router: ConnectionRouter { ConnectionRouter.shared }

    var body: some View {
        VStack(spacing: 26) {
            Spacer(minLength: 0)
            Image("MoonriseMark")
                .resizable()
                .scaledToFit()
                .frame(width: 300, height: 300)
                .shadow(color: Color(red: 1, green: 0.7, blue: 0.55).opacity(0.25), radius: 40)
            VStack(spacing: 8) {
                Text("Moonrise").font(.title2.weight(.bold))
                Text("Version \(AppInfo.versionLabel)").font(.callout).foregroundStyle(Theme.secondaryText)
            }
            Text(ExternalRatings.attribution)
                .font(.caption2).foregroundStyle(Theme.secondaryText)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 420)
            if let server = SourceStore.shared.server {
                let route = router.route ?? .checking
                HStack(spacing: 12) {
                    Circle().fill(route == .unreachable ? Color.red : route == .checking ? .yellow : .green)
                        .frame(width: 12, height: 12)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(server.name).font(.callout.weight(.semibold))
                        Text(server.kind == .demo ? "Open movies, no server" : route.label).font(.caption).foregroundStyle(Theme.secondaryText)
                    }
                }
                .padding(24)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 24, style: .continuous))
            }
            Spacer(minLength: 0)
        }
        .foregroundStyle(.white)
        .frame(maxHeight: .infinity)
    }
}
#endif

/// The server: who is signed in and how it is reached; sign out or switch servers. Both sign
/// out (the token is revoked) and go back to onboarding.
private struct ServerSection: View {
    @State private var confirmChange = false
    @State private var confirmSignOut = false
    @State private var signingOut = false
    /// Test seam: Manage Server opened on launch.
    @Binding var manage: Bool
    private var router: ConnectionRouter { ConnectionRouter.shared }
    private var admin: AdminState { AdminState.shared }

    var body: some View {
        if SourceStore.shared.isDemo {
            DemoServerSection()
        } else if let server = SourceStore.shared.server {
            Section {
                LabeledContent("Name", value: server.name)
                LabeledContent("Address", value: Self.display(server.address))
                LabeledContent("Signed In As", value: server.username)
                LabeledContent("Connection", value: router.route?.label ?? "—")
                #if !os(tvOS)
                // Sign in an Apple TV (or any device showing a code) with this device's sign-in.
                NavigationLink { ApproveDeviceView() } label: { Label("Approve a Device", systemImage: "qrcode").formRow() }
                #endif
                if admin.isAdmin {
                    // The one way into the server's own settings: libraries, accounts, services
                    // (TMDB, OpenSubtitles, AI matching), transcoding and preparing files.
                    NavigationLink { ManageServerView() } label: { Label("Manage Server", systemImage: "slider.horizontal.3").formRow() }
                }
                Button { confirmChange = true } label: { Label("Change Server", systemImage: "arrow.left.arrow.right").formRow() }
                    .formRowButton()
                    .disabled(signingOut)
                    .confirmationDialog("Change server?", isPresented: $confirmChange, titleVisibility: .visible) {
                        Button("Sign Out and Change", role: .destructive) { signOut() }
                        Button("Cancel", role: .cancel) {}
                    } message: {
                        Text("You'll be signed out of \(server.name) and can sign in to another server." + Self.downloadsNote)
                    }
                Button(role: .destructive) { confirmSignOut = true } label: {
                    HStack {
                        Label("Sign Out", systemImage: "rectangle.portrait.and.arrow.right").foregroundStyle(.red)
                        if signingOut { Spacer(); ProgressView() }
                    }
                    .formRow()
                }
                .formRowButton()
                .disabled(signingOut)
                .confirmationDialog("Sign out of \(server.name)?", isPresented: $confirmSignOut, titleVisibility: .visible) {
                    Button("Sign Out", role: .destructive) { signOut() }
                    Button("Cancel", role: .cancel) {}
                } message: {
                    Text("This device stops using \(server.name) until you sign in again." + Self.downloadsNote)
                }
            } header: {
                Text("Server")
            } footer: {
                if admin.isAdmin {
                    Text("Libraries, accounts, the server's services (TMDB, OpenSubtitles, AI matching), transcoding and preparing files are set in Manage Server, for everyone who uses this server. The settings below are for this device only.")
                }
            }
            .task { await admin.refresh(); await openSeam() }
        }
    }

    private func openSeam() async {
        #if DEBUG
        // MOONRISE_MANAGE=settings stops at Settings.
        if ManageSeam.value == "settings" { ManageSeam.consumed = true; return }
        if ManageSeam.opensSettings, !ManageSeam.consumed, admin.isAdmin {
            try? await Task.sleep(for: .milliseconds(300))
            manage = true
            if ManageSeam.screen == nil { ManageSeam.consumed = true }
        }
        #endif
    }

    private func signOut() {
        signingOut = true
        Task { await SourceStore.shared.signOut() }
    }

    private static var downloadsNote: String {
        DownloadManager.isSupported && !DownloadManager.shared.records.isEmpty
            ? " Your downloads stay on this device and play offline." : ""
    }

    /// "http://192.168.1.10:8097" → "192.168.1.10:8097".
    private static func display(_ address: String) -> String {
        guard let url = MoonriseClient.normalize(address), let host = url.host else { return address }
        return url.port.map { "\(host):\($0)" } ?? host
    }
}

/// Remote access through the embedded Tailscale node.
struct TailscaleSection: View {
    @Bindable private var ts = TailscaleService.shared
    @Environment(\.openURL) private var openURL

    var body: some View {
        Section {
            Toggle("Connect Through Tailscale", isOn: $ts.enabled).switchTint()
            if ts.enabled {
                switch ts.status {
                case .off, .starting:
                    HStack { Text("Starting…"); Spacer(); ProgressView() }
                case let .needsLogin(url):
                    if let url {
                        Button { openURL(url) } label: { Text("Sign In to Tailscale").formRow() }.formRowButton()
                    } else {
                        HStack { Text("Preparing sign-in…"); Spacer(); ProgressView() }
                    }
                case let .running(ip):
                    LabeledContent("Status", value: "Connected · \(ip)")
                    Button(role: .destructive) { ts.signOut() } label: { Text("Sign Out of Tailscale").foregroundStyle(.red).formRow() }.formRowButton()
                case let .failed(msg):
                    Text(msg).foregroundStyle(.red)
                    Button { ts.stop(); ts.start() } label: { Text("Retry").formRow() }.formRowButton()
                }
            }
        } header: {
            Text("Remote Access")
        } footer: {
            Text("Moonrise joins your tailnet as its own device, so your home server is reachable away from home without the Tailscale app or a VPN. At home it uses the local network directly.")
        }
    }
}

#if os(tvOS)
// MARK: - Apple TV setup screens

/// Two-column setup screen (Apple TV sign-in style): artwork and explanation on the left, the
/// controls on the right, vertically centred, with generous margins.
struct TVSetupScreen<Content: View>: View {
    enum Icon { case mark, symbol(String) }
    let icon: Icon
    let title: String
    let message: String
    /// A small line above the title ("Step 2 of 5").
    var caption: String? = nil
    @ViewBuilder let content: () -> Content

    var body: some View {
        HStack(alignment: .center, spacing: 120) {
            VStack(alignment: .leading, spacing: 30) {
                switch icon {
                case .mark:
                    Image("MoonriseMark").resizable().scaledToFit().frame(width: 240, height: 240)
                case let .symbol(name):
                    Image(systemName: name)
                        .font(.system(size: 90, weight: .light))
                        .frame(width: 200, height: 200)
                        .background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 44, style: .continuous))
                }
                VStack(alignment: .leading, spacing: 10) {
                    if let caption {
                        Text(caption).font(.caption.weight(.semibold)).foregroundStyle(.secondary).textCase(.uppercase)
                    }
                    Text(title).font(.system(size: 64, weight: .bold)).lineLimit(2).minimumScaleFactor(0.6)
                }
                Text(message).font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            .frame(width: 620, alignment: .leading)

            content()
                .frame(width: 820)
                .padding(.vertical, 60)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background {
            ZStack {
                Theme.background
                RadialGradient(colors: [Color(red: 1, green: 0.6, blue: 0.45).opacity(0.12), .clear],
                               center: .topLeading, startRadius: 0, endRadius: 1200)
            }
            .ignoresSafeArea()
        }
        .toolbar(.hidden, for: .navigationBar)
    }
}

/// Small caption above a field.
struct TVField<Content: View>: View {
    let label: String
    @ViewBuilder let content: () -> Content
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(label).font(.caption.weight(.semibold)).foregroundStyle(.secondary).padding(.leading, 24)
            content()
        }
    }
}

struct TVOrDivider: View {
    var label = "or sign in with a password"
    var body: some View {
        HStack(spacing: 20) {
            Rectangle().fill(.white.opacity(0.15)).frame(height: 1)
            Text(label).font(.caption).foregroundStyle(.secondary).fixedSize()
            Rectangle().fill(.white.opacity(0.15)).frame(height: 1)
        }
        .padding(.vertical, 6)
    }
}
#endif

extension View {
    /// Mac: a Settings row that is a button reads as a row (full width, like Approve a Device),
    /// not as a small bordered button inside the row. Unchanged elsewhere.
    @ViewBuilder func formRow() -> some View {
        #if os(macOS) || os(tvOS)
        // Apple TV: the focused row's glass plate spans the row, not just its label.
        self.frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
        #else
        self
        #endif
    }

    /// Apple TV: Settings rows on clear glass instead of flat dark plates. Unchanged elsewhere.
    @ViewBuilder func tvGlassRows() -> some View {
        #if os(tvOS)
        self.listRowBackground(Color.clear.glassEffect(.regular, in: .capsule))
        #else
        self
        #endif
    }

    @ViewBuilder func formRowButton() -> some View {
        #if os(macOS)
        self.buttonStyle(.plain)
        #else
        self
        #endif
    }
}
