import CoreImage
import CoreImage.CIFilterBuiltins
import SwiftUI

// Signing in to the Moonrise server (onboarding, shown while there is none): found on the network
// (Bonjour, else a subnet sweep) or typed in, then a password or a device code (approved on the
// iPhone or at the server's /link page). Also the iPhone's "Approve a Device" screen.

/// The app with no server: welcome, then sign in. Downloads kept from before still play.
struct OnboardingView: View {
    #if os(tvOS)
    var body: some View {
        NavigationStack { SignInView() }
    }
    #else
    @State private var showDownloads = false
    private var hasDownloads: Bool { DownloadManager.isSupported && !DownloadManager.shared.records.isEmpty }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 22) {
                    WelcomeHero()
                    NavigationLink { SignInView() } label: {
                        // Custom capsule: the white accent makes a prominent system button white on white.
                        Label("Connect to Your Server", systemImage: "server.rack")
                            .font(.headline)
                            .foregroundStyle(.black)
                            .padding(.horizontal, 28).padding(.vertical, 14)
                            .background(.white, in: Capsule())
                    }
                    .plainButton()
                    TryDemoButton()
                    if hasDownloads {
                        Button { showDownloads = true } label: {
                            Label("Downloads", systemImage: "arrow.down.circle")
                                .padding(.horizontal, 24).padding(.vertical, 12)
                        }
                        .plainButton()
                        .glassEffect(.regular.interactive(), in: .capsule)
                    }
                }
                .frame(maxWidth: .infinity)
                .padding(.bottom, 40)
            }
            .background(Theme.background.ignoresSafeArea())
            .ignoresSafeArea(edges: .top)
            .navigationDestination(isPresented: $showDownloads) { DownloadsView() }
        }
    }
    #endif
}

/// First-run hero over the MOSS theatre artwork.
private struct WelcomeHero: View {
    var body: some View {
        ZStack(alignment: .bottom) {
            Color.clear
                .frame(height: 420)
                .frame(maxWidth: .infinity)
                .overlay { Image("WelcomeTheater").resizable().aspectRatio(contentMode: .fill) }
                .clipped()
            LinearGradient(colors: [.clear, Theme.background], startPoint: .center, endPoint: .bottom)
            VStack(spacing: 8) {
                Text("Welcome to Moonrise").font(.largeTitle.weight(.bold))
                Text(SourceStore.welcomeMessage)
                    .font(.subheadline).foregroundStyle(Theme.secondaryText).multilineTextAlignment(.center)
            }
            .padding(.horizontal, 30)
            .padding(.bottom, 20)
        }
        .foregroundStyle(.white)
    }
}

/// Signs in to a Moonrise server; it becomes the app's server.
struct SignInView: View {
    @State private var address = ""
    @State private var username = ""
    @State private var password = ""
    @State private var info: MoonriseClient.Info?
    @State private var busy = false
    @State private var error: String?
    @State private var deviceCode: DeviceCodeSession?
    @State private var discovered: [MoonriseDiscovery.Server] = []
    @State private var discovering = true
    /// Setting up a new server (unclaimed), or finishing the setup of one without libraries.
    @State private var setup: SetupFlow?
    @State private var showSetup = false
    @State private var resumeSetup = false

    private var unclaimed: Bool { info?.unclaimed == true }

    private var deviceCodeAvailable: Bool { info?.capabilities.contains("device-code") == true }
    private var invitesAvailable: Bool { info?.capabilities.contains("invites") == true }
    @State private var showInvite = false

    var body: some View {
        content
            .task(id: address) {
                try? await Task.sleep(for: .milliseconds(600))
                if !Task.isCancelled, MoonriseClient.normalize(address) != nil { await check() }
            }
            .task {
                #if DEBUG
                if address.isEmpty, let a = DebugSeams.launch.setupAddress { address = a }
                #endif
                discovered = await MoonriseDiscovery.discover()
                discovering = false
                if address.isEmpty, discovered.count == 1 { address = discovered[0].address }
            }
            .navigationDestination(isPresented: $showSetup) { if let setup { SetupCodeView(flow: setup) } }
            .navigationDestination(isPresented: $resumeSetup) { if let setup { SetupLibraryView(flow: setup) } }
            .navigationDestination(isPresented: $showInvite) {
                if let info, let url = MoonriseClient.normalize(address) { InviteSignInView(url: url, info: info) }
            }
            .moonriseSheet(item: $deviceCode) { session in
                DeviceCodeView(session: session) { result in
                    deviceCode = nil
                    if case let .success(source) = result {
                        SourceStore.shared.signIn(source)
                    } else if case let .failure(e) = result, !e.isCancellation {
                        error = e.localizedDescription
                    }
                }
            }
    }

    #if os(tvOS)
    private var content: some View {
        TVSetupScreen(icon: .mark, title: "Welcome to Moonrise",
                      message: SourceStore.welcomeMessage + " Sign in with a code: approve it on your iPhone (Settings → Approve a Device) or in any browser — no typing passwords with the remote.") {
            VStack(alignment: .leading, spacing: 28) {
                if discovering || !discovered.isEmpty {
                    TVField(label: discovering ? "Looking for servers…" : "On Your Network") {
                        HStack(spacing: 16) {
                            ForEach(discovered) { srv in
                                Button { address = srv.address } label: {
                                    Label(srv.unclaimed ? "\(srv.name) · New" : srv.name,
                                          systemImage: address == srv.address ? "checkmark.circle.fill" : "server.rack")
                                }
                                .tvGlassButton(height: 66, selected: address == srv.address)
                            }
                            if discovering { ProgressView().padding(.leading, 24) }
                        }
                    }
                }
                TVField(label: "Server") {
                    MoonriseTextField(title: "Server address", text: $address, prompt: "e.g. 192.168.1.10")
                        .autocorrectionDisabled()
                        .onSubmit { Task { await check() } }
                }
                if let info {
                    Label("\(info.name) · \(info.version)", systemImage: "checkmark.circle.fill")
                        .font(.callout).foregroundStyle(.green)
                        .padding(.leading, 24)
                }
                if unclaimed {
                    Text("This server is new and isn't set up yet. You'll need its setup code (in the server's log, or on its page in a browser), then you create its admin account and add your videos.")
                        .font(.callout).foregroundStyle(.secondary).padding(.leading, 24)
                        .fixedSize(horizontal: false, vertical: true)
                    Button { startSetup() } label: {
                        Label("Set Up This Server", systemImage: "wand.and.sparkles").font(.body.bold()).frame(maxWidth: .infinity)
                    }
                    .tvGlassButton(prominent: true)
                } else {
                    tvSignIn
                }
                if let error { Text(error).font(.callout).foregroundStyle(.red) }
                TVOrDivider(label: "no server yet? four open movies from the Blender Foundation")
                TryDemoButton()
                    .padding(.bottom, 60)
            }
        }
    }

    @ViewBuilder private var tvSignIn: some View {
        if deviceCodeAvailable {
            Button { startDeviceCode() } label: {
                Label("Sign In with a Code", systemImage: "qrcode.viewfinder")
                    .font(.body.bold()).frame(maxWidth: .infinity)
            }
            .tvGlassButton(prominent: true)
            TVOrDivider()
        }
        TVField(label: "Username") {
            MoonriseTextField(title: "Username", text: $username).autocorrectionDisabled()
        }
        TVField(label: "Password") {
            MoonriseTextField(title: "Password", text: $password, secure: true)
        }
        Button { Task { await signIn() } } label: {
            HStack(spacing: 16) {
                Text("Sign In").font(.body.bold())
                if busy { ProgressView() }
            }
            .frame(maxWidth: .infinity)
        }
        .tvGlassButton(prominent: true)
        .disabled(address.isEmpty || username.isEmpty || busy)
        if invitesAvailable {
            Button { showInvite = true } label: {
                Label("Have an Invitation", systemImage: "envelope.open").frame(maxWidth: .infinity)
            }
            .tvGlassButton()
        }
    }
    #else
    private var content: some View {
        Form {
            if discovering || !discovered.isEmpty {
                Section {
                    ForEach(discovered) { srv in
                        Button { address = srv.address } label: {
                            HStack {
                                Label {
                                    VStack(alignment: .leading) {
                                        Text(srv.name).foregroundStyle(.white)
                                        Text(srv.unclaimed ? "\(srv.address) · New, not set up" : srv.address)
                                            .font(.caption).foregroundStyle(.secondary)
                                    }
                                } icon: { Image(systemName: "server.rack") }
                                Spacer()
                                if address == srv.address { Image(systemName: "checkmark").foregroundStyle(.green) }
                            }
                        }
                    }
                    if discovering { HStack { ProgressView(); Text("Looking for servers…").foregroundStyle(.secondary) } }
                } header: { Text("On Your Network") }
            }
            Section {
                MoonriseTextField(title: "Server address", text: $address, prompt: "e.g. 192.168.1.10")
                    .autocorrectionDisabled()
                    #if os(iOS)
                    .textInputAutocapitalization(.never)
                    .keyboardType(.URL)
                    #endif
                    .onSubmit { Task { await check() } }
                if let info { Label("\(info.name) · \(info.version)", systemImage: "checkmark.circle.fill").foregroundStyle(.green) }
            } header: { Text("Server") }

            if unclaimed {
                Section {
                    Button { startSetup() } label: { Label("Set Up This Server", systemImage: "wand.and.sparkles") }
                        .formRow()
                } header: { Text("New Server") } footer: {
                    Text("This server isn't set up yet. You'll need its setup code (in the server's log, or on its page in a browser), then you create its admin account and add your videos.")
                }
            } else {
                signInSections
            }

            if let error { Section { Text(error).foregroundStyle(.red) } }
        }
        .formStyle(.grouped)
        .navigationTitle("Sign In")
    }

    @ViewBuilder private var signInSections: some View {
        if deviceCodeAvailable {
            Section {
                Button { startDeviceCode() } label: {
                    Label("Sign In with a Code", systemImage: "qrcode.viewfinder")
                }
            } footer: {
                Text("Shows a code to approve on another device you're signed in on, or in any browser — no password needed.")
            }
        }

        Section(deviceCodeAvailable ? "Or Sign In with Password" : "Account") {
            MoonriseTextField(title: "Username", text: $username)
                .autocorrectionDisabled()
                #if os(iOS)
                .textInputAutocapitalization(.never)
                #endif
            MoonriseTextField(title: "Password", text: $password, secure: true)
            Button {
                Task { await signIn() }
            } label: {
                HStack { Text("Sign In"); if busy { Spacer(); ProgressView() } }
            }
            .disabled(address.isEmpty || username.isEmpty || busy)
        }
        if invitesAvailable {
            Section {
                Button { showInvite = true } label: {
                    HStack {
                        Label("Have an Invitation?", systemImage: "envelope.open")
                        Spacer()
                        Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
                    }
                    .contentShape(Rectangle())
                }
                .formRow()
            } footer: {
                Text("New here? With a code from an admin of this server, pick your own user name and password.")
            }
        }
    }
    #endif

    private func startSetup() {
        guard let info, let url = MoonriseClient.normalize(address) else { return }
        setup = SetupFlow(url: url, info: info)
        showSetup = true
    }

    private func check() async {
        guard let url = MoonriseClient.normalize(address) else { error = MoonriseClientError.badURL.localizedDescription; return }
        do {
            info = try await MoonriseClient(sourceId: nil, baseURL: url, token: nil).info()
            error = nil
        } catch {
            info = nil
            if !error.isCancellation { self.error = "Couldn't reach the server: \(error.localizedDescription)" }
        }
    }

    private func startDeviceCode() {
        guard let url = MoonriseClient.normalize(address) else { return }
        deviceCode = DeviceCodeSession(url: url)
    }

    private func signIn() async {
        busy = true
        defer { busy = false }
        guard let url = MoonriseClient.normalize(address) else { error = MoonriseClientError.badURL.localizedDescription; return }
        let c = MoonriseClient(sourceId: nil, baseURL: url, token: nil)
        do {
            let info = try await c.info()
            let auth = try await c.login(username: username, password: password)
            // An admin of a server without libraries (a setup left before its library step):
            // continue the setup there.
            if auth.user.isAdmin == true, let libs = try? await c.adminLibraries(), libs.isEmpty {
                setup = SetupFlow(url: url, info: info, auth: auth)
                resumeSetup = true
                return
            }
            SourceStore.shared.signIn(Self.source(info: info, url: url, auth: auth))
        } catch let MoonriseClientError.http(401, _) {
            error = "Wrong user name or password."
        } catch let MoonriseClientError.http(429, _) {
            error = "Too many attempts. Try again in a few minutes."
        } catch {
            self.error = error.localizedDescription
        }
    }

    static func source(info: MoonriseClient.Info, url: URL, auth: MoonriseClient.AuthResult) -> Source {
        var source = Source(kind: .moonrise, name: info.name, address: url.absoluteString, username: auth.user.name, userId: auth.user.id)
        source.serverId = info.serverId
        source.secret = auth.token
        return source
    }
}

extension View {
    /// A sheet (full screen on Apple TV, whose sheets are small popovers).
    @ViewBuilder func moonriseSheet<Item: Identifiable, V: View>(item: Binding<Item?>, @ViewBuilder content: @escaping (Item) -> V) -> some View {
        #if os(tvOS)
        fullScreenCover(item: item, content: content)
        #else
        sheet(item: item, content: content)
        #endif
    }
}

struct DeviceCodeSession: Identifiable {
    let id = UUID()
    let url: URL
}

/// Shows a device code (and a QR code of the page that approves it) and waits for approval.
struct DeviceCodeView: View {
    let session: DeviceCodeSession
    let onFinish: (Result<Source, Error>) -> Void
    @State private var code: MoonriseClient.DeviceCode?
    @State private var qr: CGImage?
    @State private var expired = false
    @State private var error: String?
    @State private var attempt = 0

    var body: some View {
        VStack(spacing: spacing) {
            #if os(tvOS)
            HStack(alignment: .center, spacing: 90) {
                if let qr { qrView(qr).frame(width: 360, height: 360) }
                // Cancel in the text column, on its leading edge (not centred under both).
                VStack(alignment: .leading, spacing: 30) { details; cancel }
                    .frame(maxWidth: 900, alignment: .leading)
            }
            #else
            details
            if let qr { qrView(qr).frame(width: 180, height: 180) }
            cancel
            #endif
        }
        .padding(50)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.background.ignoresSafeArea())
        .task(id: attempt) { await run() }
    }

    private var cancel: some View {
        Button("Cancel", role: .cancel) { onFinish(.failure(CancellationError())) }
            .tvGlass()
    }

    @ViewBuilder private var details: some View {
        Text("Sign In with a Code").font(.title.weight(.bold))
        if let error {
            Text(error).foregroundStyle(.red).multilineTextAlignment(alignment)
            Button("Try Again") { self.error = nil; code = nil; qr = nil; attempt += 1 }.tvGlass()
        } else if let code, !expired {
            Text(code.userCode)
                .font(.system(size: codeSize, weight: .bold, design: .monospaced))
                .kerning(codeSize * 0.12)
                .padding(.horizontal, 36).padding(.vertical, 18)
                .background(.white.opacity(0.1), in: RoundedRectangle(cornerRadius: 22, style: .continuous))
                .accessibilityIdentifier("deviceCode")
            Text("On your iPhone, open Moonrise → **Settings → Approve a Device** and enter this code. Or scan the QR code, or open **\(code.verifyUrl)** in any browser.")
                .foregroundStyle(Theme.secondaryText)
                .multilineTextAlignment(alignment)
                .frame(maxWidth: 700, alignment: frameAlignment)
            HStack(spacing: 10) {
                ProgressView()
                Text("Waiting for approval…").foregroundStyle(Theme.secondaryText)
            }
        } else if expired {
            Text("The code expired.").foregroundStyle(Theme.secondaryText)
            Button("Get a New Code") { expired = false; code = nil; qr = nil; attempt += 1 }.tvGlass()
        } else {
            ProgressView()
        }
    }

    private func qrView(_ image: CGImage) -> some View {
        Image(decorative: image, scale: 1)
            .interpolation(.none)
            .resizable()
            .scaledToFit()
            .padding(14)
            .background(.white, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
            .accessibilityLabel("QR code of the sign-in page")
    }

    #if os(tvOS)
    private let codeSize: CGFloat = 96
    private let spacing: CGFloat = 50
    private let alignment = TextAlignment.leading
    private let frameAlignment = Alignment.leading
    #else
    private let codeSize: CGFloat = 40
    private let spacing: CGFloat = 26
    private let alignment = TextAlignment.center
    private let frameAlignment = Alignment.center
    #endif

    private func run() async {
        let client = MoonriseClient(sourceId: nil, baseURL: session.url, token: nil)
        do {
            let info = try await client.info()
            let c = try await client.startDeviceCode()
            code = c
            qr = Self.qrCode(c.verifyUrlComplete)
            #if DEBUG
            fputs("MoonriseDeviceCode: \(c.userCode) \(c.verifyUrl)\n", stderr)
            #endif
            var interval = max(c.interval, 2)
            let deadline = Date().addingTimeInterval(TimeInterval(c.expiresIn))
            var failures = 0
            while !Task.isCancelled {
                try await Task.sleep(for: .seconds(interval))
                if Date() > deadline { expired = true; return }
                let state: MoonriseClient.DevicePoll
                do {
                    state = try await client.pollDeviceCode(c.deviceCode)
                    failures = 0
                } catch where !error.isCancellation {
                    // A blip (Wi-Fi, the server restarting): keep waiting, give up after a minute.
                    failures += 1
                    if failures * interval >= 60 { throw error }
                    continue
                }
                switch state {
                case .pending: continue
                case .slowDown: interval += 5
                case .expired: expired = true; return
                case let .approved(auth):
                    onFinish(.success(SignInView.source(info: info, url: session.url, auth: auth)))
                    return
                }
            }
        } catch where error.isCancellation {
        } catch {
            self.error = "Couldn't sign in with a code: \(error.localizedDescription)"
        }
    }

    static func qrCode(_ text: String) -> CGImage? {
        let f = CIFilter.qrCodeGenerator()
        f.message = Data(text.utf8)
        f.correctionLevel = "M"
        guard let out = f.outputImage else { return nil }
        let scaled = out.transformed(by: CGAffineTransform(scaleX: 10, y: 10))
        return CIContext().createCGImage(scaled, from: scaled.extent)
    }
}

#if !os(tvOS)
/// Settings → Approve a Device: sign another device (an Apple TV showing a code) in to the
/// server with this device's sign-in.
struct ApproveDeviceView: View {
    @State private var code = ""
    @State private var busy = false
    @State private var result: String?
    @State private var error: String?

    var body: some View {
        Form {
            Section {
                TextField("ABCD-1234", text: $code)
                    .font(.title2.monospaced())
                    .autocorrectionDisabled()
                    #if os(iOS)
                    .textInputAutocapitalization(.characters)
                    #endif
                    .onSubmit { Task { await approve() } }
                    .accessibilityIdentifier("approveCode")
                Button {
                    Task { await approve() }
                } label: {
                    HStack { Text("Approve"); if busy { Spacer(); ProgressView() } }
                }
                .disabled(Self.format(code).count < 9 || busy)
            } footer: {
                Text("Enter the code the other device shows (Apple TV: Sign In with a Code). It signs in as you.")
            }
            if let result { Section { Label(result, systemImage: "checkmark.circle.fill").foregroundStyle(.green) } }
            if let error { Section { Text(error).foregroundStyle(.red) } }
        }
        .formStyle(.grouped)
        .navigationTitle("Approve a Device")
    }

    /// "abcd1234" → "ABCD-1234".
    static func format(_ s: String) -> String {
        let chars = s.uppercased().filter { $0.isLetter || $0.isNumber }.prefix(8)
        return chars.count > 4 ? String(chars.prefix(4)) + "-" + String(chars.dropFirst(4)) : String(chars)
    }

    private func approve() async {
        guard let source = SourceStore.shared.server, let client = MoonriseClient.for(source) else { return }
        busy = true
        defer { busy = false }
        result = nil
        error = nil
        await ConnectionRouter.shared.ready()
        do {
            // Typed any way ("abcd1234", "ABCD 1234"): the server's form is "ABCD-1234".
            let name = try await client.approveDevice(userCode: Self.format(code))
            result = "Approved\(name.map { " · \($0)" } ?? ""). It is signing in now."
            code = ""
        } catch MoonriseClientError.http(404, _) {
            error = "No device is waiting with that code (it may have expired)."
        } catch MoonriseClientError.http(let status, let body) where status == 400 || status == 410 {
            error = body.isEmpty ? "That code can't be approved." : body
        } catch {
            self.error = error.localizedDescription
        }
    }
}
#endif
