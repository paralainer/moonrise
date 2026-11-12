import SwiftUI

// Setting up a new Moonrise server from the app (a server with no accounts is "unclaimed", see
// server/src/admin/claim.rs): the setup code from the server's log or its page → the admin account →
// the first library (a folder on the server's computer, or a network share; a user name and
// password only when the share asks for one) → the first scan, live → optional extras (own keys
// and accounts, all skippable) → Home. Signing in as an admin to a server without libraries
// continues at the library step.

/// The setup in progress: the server, and after the account step its admin token. The app only
/// takes the server on (SourceStore) when the setup is finished, so Home opens on a library.
@MainActor
@Observable
final class SetupFlow {
    let url: URL
    let info: MoonriseClient.Info
    let client: MoonriseClient
    var code = ""
    var auth: MoonriseClient.AuthResult?
    /// Scan passes counted before the library was added (the first scan is the next one).
    var passesBefore = 0
    var library: MoonriseClient.AdminLibrary?

    static let steps = 5

    init(url: URL, info: MoonriseClient.Info, auth: MoonriseClient.AuthResult? = nil) {
        self.url = url
        self.info = info
        self.auth = auth
        client = MoonriseClient(sourceId: nil, baseURL: url, token: auth?.token)
        #if DEBUG
        code = DebugSeams.launch.setupCode ?? ""
        #endif
    }

    /// "192.168.1.10:8097": where the setup page is.
    var pageAddress: String {
        var s = url.absoluteString
        if s.hasPrefix("http://") { s.removeFirst(7) }
        return s
    }

    /// Done: this is the app's server now (Home opens).
    func finish() {
        guard let auth else { return }
        SourceStore.shared.signIn(SignInView.source(info: info, url: url, auth: auth))
    }
}

func serverErrorText(_ error: Error) -> String {
    if let e = error as? MoonriseClientError, let m = e.serverMessage, case .http(let code, _) = e, code < 500 { return m.prefix(1).uppercased() + m.dropFirst() }
    return error.localizedDescription
}

// MARK: - Shared pieces

/// iPhone, iPad, Mac: the first row of a setup step (step number and what it is about), aligned
/// with the rows' text below.
private struct SetupHeader: View {
    let step: Int
    let message: String
    var body: some View {
        Section {
            VStack(alignment: .leading, spacing: 8) {
                Text("Step \(step) of \(SetupFlow.steps)")
                    .font(.caption.weight(.semibold)).foregroundStyle(.secondary).textCase(.uppercase)
                Text(message).foregroundStyle(Theme.secondaryText).fixedSize(horizontal: false, vertical: true)
            }
            .listRowBackground(Color.clear)
        }
    }
}

/// A primary action row with a spinner while it runs.
struct BusyLabel: View {
    let title: String
    let busy: Bool
    var body: some View {
        #if os(tvOS)
        HStack(spacing: 16) {
            Text(title).font(.body.bold())
            if busy { ProgressView() }
        }
        .frame(maxWidth: .infinity)
        #else
        HStack { Text(title); if busy { Spacer(); ProgressView() } }
        #endif
    }
}

#if os(tvOS)
private func tvStep(_ step: Int) -> String { "Step \(step) of \(SetupFlow.steps)" }

/// Rows of a list on Apple TV: far enough apart that a focused row's plate (which hangs 18 pt
/// outside it) doesn't cover its neighbours.
let tvRowSpacing: CGFloat = 40

/// A list as tall as its rows (up to `max`), so a short one sits with the rest of the column.
func tvListHeight(rows: Int, max: CGFloat) -> CGFloat {
    min(max, CGFloat(Swift.max(rows, 1)) * (50 + tvRowSpacing) + 48)
}

/// A list row on Apple TV (folders, shares, servers): glass when focused, full width.
struct TVPickRow: View {
    let symbol: String
    let title: String
    var detail: String?
    var chevron = true
    let action: () -> Void
    var body: some View {
        Button(action: action) {
            HStack(spacing: 22) {
                Image(systemName: symbol).frame(width: 40, alignment: .leading)
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).lineLimit(1)
                    if let detail { Text(detail).font(.caption).foregroundStyle(.secondary).lineLimit(1) }
                }
                Spacer(minLength: 0)
                if chevron { Image(systemName: "chevron.right").font(.callout).foregroundStyle(.secondary) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .buttonStyle(TVGlassRowStyle())
    }
}
#endif

// MARK: - Step 1: setup code

struct SetupCodeView: View {
    @Bindable var flow: SetupFlow
    @State private var busy = false
    @State private var error: String?
    @State private var next = false

    private var digits: String { flow.code.filter(\.isNumber) }
    private var message: String {
        "This server is new. To show it's yours, enter the 6-digit setup code from the server's log, or open http://\(flow.pageAddress) in a browser on your network."
    }

    var body: some View {
        content
            .navigationDestination(isPresented: $next) { SetupAccountView(flow: flow) }
            .onChange(of: flow.code) { _, c in
                // Digits only (a pasted "123 456" too), at most six.
                let d = String(c.filter(\.isNumber).prefix(6))
                if d != c { flow.code = d }
            }
            .onChange(of: digits) { _, d in if d.count == 6, flow.auth == nil { Task { await verify() } } }
    }

    #if os(tvOS)
    private var content: some View {
        TVSetupScreen(icon: .symbol("lock.open"), title: "Setup Code", message: message, caption: tvStep(1)) {
            VStack(alignment: .leading, spacing: 28) {
                TVField(label: "Setup code") {
                    MoonriseTextField(title: "Setup code", text: $flow.code, prompt: "123 456")
                        .keyboardType(.numberPad)
                        .onSubmit { Task { await verify() } }
                }
                Button { Task { await verify() } } label: { BusyLabel(title: "Continue", busy: busy) }
                    .tvGlassButton(prominent: true)
                    .disabled(digits.count != 6 || busy)
                if let error { Text(error).font(.callout).foregroundStyle(.red) }
            }
        }
    }
    #else
    private var content: some View {
        Form {
            SetupHeader(step: 1, message: message)
            Section {
                TextField("123 456", text: $flow.code)
                    .font(.title2.monospacedDigit())
                    #if os(iOS)
                    .keyboardType(.numberPad)
                    .textContentType(.oneTimeCode)
                    #endif
                    .onSubmit { Task { await verify() } }
                    .accessibilityIdentifier("setupCode")
                Button { Task { await verify() } } label: { BusyLabel(title: "Continue", busy: busy) }
                    .disabled(digits.count != 6 || busy)
            } header: { Text("Setup Code") } footer: {
                Text("The code changes when the server restarts.")
            }
            if let error { Section { Text(error).foregroundStyle(.red) } }
        }
        .formStyle(.grouped)
        .navigationTitle("Set Up Server")
    }
    #endif

    private func verify() async {
        if flow.auth != nil { next = true; return }
        guard digits.count == 6, !busy else { return }
        busy = true
        defer { busy = false }
        do {
            try await flow.client.verifySetupCode(digits)
            error = nil
            next = true
        } catch MoonriseClientError.http(403, _) {
            error = "That isn't the code. Check the server's log or its page, and try again."
        } catch MoonriseClientError.http(409, _) {
            error = "This server has been set up already. Go back and sign in."
        } catch MoonriseClientError.http(429, _) {
            error = "Too many wrong codes. Wait a few minutes; the server may show a new code."
        } catch {
            self.error = serverErrorText(error)
        }
    }
}

// MARK: - Step 2: the admin account

struct SetupAccountView: View {
    @Bindable var flow: SetupFlow
    @State private var username = ""
    @State private var password = ""
    @State private var confirm = ""
    @State private var busy = false
    @State private var error: String?
    @State private var next = false

    private let message = "Create the server's first account. It's the admin: it manages the server from this app. Use it to sign in on your other devices too."

    private var problem: String? {
        if username.trimmingCharacters(in: .whitespaces).isEmpty { return "" }
        if password.count < 8 { return "The password needs at least 8 characters." }
        #if !os(tvOS)
        if !confirm.isEmpty, confirm != password { return "The passwords don't match." }
        if confirm.isEmpty { return "" }
        #endif
        return nil
    }

    var body: some View {
        content
            .navigationDestination(isPresented: $next) { SetupLibraryView(flow: flow) }
            .onAppear {
                #if DEBUG
                if username.isEmpty, let u = DebugSeams.launch.setupUser { username = u }
                if password.isEmpty, let p = DebugSeams.launch.setupPass { password = p; confirm = p }
                #endif
            }
    }

    #if os(tvOS)
    private var content: some View {
        TVSetupScreen(icon: .symbol("person.crop.circle.badge.plus"), title: "Your Account", message: message, caption: tvStep(2)) {
            VStack(alignment: .leading, spacing: 28) {
                TVField(label: "User name") {
                    MoonriseTextField(title: "User name", text: $username).autocorrectionDisabled()
                }
                TVField(label: "Password (at least 8 characters)") {
                    MoonriseTextField(title: "Password", text: $password, secure: true)
                }
                Button { Task { await create() } } label: { BusyLabel(title: flow.auth == nil ? "Create Account" : "Continue", busy: busy) }
                    .tvGlassButton(prominent: true)
                    .disabled((problem != nil && flow.auth == nil) || busy)
                if let error { Text(error).font(.callout).foregroundStyle(.red) }
                else if let p = problem, !p.isEmpty, !password.isEmpty { Text(p).font(.callout).foregroundStyle(.secondary) }
            }
        }
    }
    #else
    private var content: some View {
        Form {
            SetupHeader(step: 2, message: message)
            Section {
                TextField("User name", text: $username)
                    .autocorrectionDisabled()
                    #if os(iOS)
                    .textInputAutocapitalization(.never)
                    .textContentType(.username)
                    #endif
                    .accessibilityIdentifier("setupUser")
                SecureField("Password", text: $password)
                    #if os(iOS)
                    .textContentType(.password)  // not .newPassword: no generated password to retype on a TV
                    #endif
                    .accessibilityIdentifier("setupPass")
                SecureField("Repeat password", text: $confirm)
                    #if os(iOS)
                    .textContentType(.password)  // not .newPassword: no generated password to retype on a TV
                    #endif
                    .accessibilityIdentifier("setupConfirm")
            } header: { Text("Admin Account") } footer: {
                if let p = problem, !p.isEmpty, !password.isEmpty { Text(p) } else { Text("At least 8 characters.") }
            }
            Section {
                Button { Task { await create() } } label: { BusyLabel(title: flow.auth == nil ? "Create Account" : "Continue", busy: busy) }
                    .disabled((problem != nil && flow.auth == nil) || busy)
            }
            if let error { Section { Text(error).foregroundStyle(.red) } }
        }
        .formStyle(.grouped)
        .navigationTitle("Your Account")
    }
    #endif

    private func create() async {
        if flow.auth != nil { next = true; return }
        guard problem == nil, !busy else { return }
        busy = true
        defer { busy = false }
        do {
            flow.auth = try await flow.client.claim(code: flow.code.filter(\.isNumber), username: username.trimmingCharacters(in: .whitespaces), password: password)
            error = nil
            next = true
        } catch MoonriseClientError.http(403, _) {
            error = "The setup code has changed (the server restarted, or it was guessed at). Go back and enter the new one."
        } catch MoonriseClientError.http(409, _) {
            error = "This server has just been set up from another device. Go back and sign in."
        } catch {
            self.error = serverErrorText(error)
        }
    }
}

// MARK: - Step 3: the first library

struct SetupLibraryView: View {
    @Bindable var flow: SetupFlow
    @State private var name = "Media"
    @State private var root: LibraryRoot?
    @State private var picking: LibraryRoot.Source?
    @State private var busy = false
    @State private var error: String?
    @State private var next = false

    private let message = "Where are your videos? Pick a folder the server's computer can see, or a share on your NAS or another computer. Movies and shows can share a folder: Moonrise tells them apart. You can add more folders later."

    var body: some View {
        content
            .navigationBarBackButtonHidden(flow.auth != nil)
            .navigationDestination(item: $picking) { source in
                Group {
                    switch source {
                    case .local: ServerFolderPicker(client: flow.client) { root = $0; picking = nil }
                    case .smb: SharePicker(client: flow.client) { root = $0; picking = nil }
                    }
                }
            }
            .navigationDestination(isPresented: $next) { SetupScanView(flow: flow) }
    }

    #if os(tvOS)
    private var content: some View {
        TVSetupScreen(icon: .symbol("film.stack"), title: "Add Your Videos", message: message, caption: tvStep(3)) {
            VStack(alignment: .leading, spacing: 28) {
                TVField(label: "Name") {
                    MoonriseTextField(title: "Library name", text: $name)
                }
                TVField(label: "Folder") {
                    VStack(alignment: .leading, spacing: 12) {
                        if let root {
                            Label(root.display, systemImage: "checkmark.circle.fill").foregroundStyle(.green).lineLimit(1)
                                .padding(.leading, 24)
                        }
                        HStack(spacing: 16) {
                            Button { picking = .local } label: { Label("On the Server", systemImage: "internaldrive") }
                                .tvGlassButton(height: 66, selected: root?.source == .local)
                            Button { picking = .smb } label: { Label("Network Share", systemImage: "network") }
                                .tvGlassButton(height: 66, selected: root?.source == .smb)
                        }
                    }
                }
                Button { Task { await add() } } label: { BusyLabel(title: "Add Library", busy: busy) }
                    .tvGlassButton(prominent: true)
                    .disabled(root == nil || name.trimmingCharacters(in: .whitespaces).isEmpty || busy)
                if let error { Text(error).font(.callout).foregroundStyle(.red) }
            }
        }
    }
    #else
    private var content: some View {
        Form {
            SetupHeader(step: 3, message: message)
            Section("Library Name") {
                TextField("Name", text: $name)
                    .accessibilityIdentifier("libraryName")
            }
            Section {
                Button { picking = .local } label: {
                    sourceRow(symbol: "internaldrive", title: "A Folder on the Server", detail: "Folders the server's computer can see", chosen: root?.source == .local)
                }
                .formRow()
                Button { picking = .smb } label: {
                    sourceRow(symbol: "network", title: "A Network Share", detail: "A NAS or another computer on your network", chosen: root?.source == .smb)
                }
                .formRow()
            } header: { Text("Where Are the Videos?") } footer: {
                if let root { Label(root.display, systemImage: "checkmark.circle.fill").foregroundStyle(.green) }
            }
            Section {
                Button { Task { await add() } } label: { BusyLabel(title: "Add Library", busy: busy) }
                    .disabled(root == nil || name.trimmingCharacters(in: .whitespaces).isEmpty || busy)
            }
            if let error { Section { Text(error).foregroundStyle(.red) } }
        }
        .formStyle(.grouped)
        .navigationTitle("Add Your Videos")
    }

    private func sourceRow(symbol: String, title: String, detail: String, chosen: Bool) -> some View {
        HStack(spacing: 14) {
            Image(systemName: symbol).font(.title3).frame(width: 30).foregroundStyle(.white)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).foregroundStyle(.white)
                Text(detail).font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            if chosen { Image(systemName: "checkmark").foregroundStyle(.green) }
            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
        }
        .contentShape(Rectangle())
    }
    #endif

    private func add() async {
        guard let root, !busy else { return }
        busy = true
        defer { busy = false }
        do {
            flow.passesBefore = (try? await flow.client.scanProgress().passes) ?? 0
            flow.library = try await flow.client.createLibrary(name: name.trimmingCharacters(in: .whitespaces), root: root)
            error = nil
            next = true
        } catch {
            self.error = serverErrorText(error)
        }
    }
}

extension LibraryRoot.Source: Identifiable { var id: Self { self } }

// MARK: Folder on the server

/// Browses the server's own folders (`/v1/admin/fs/browse`) and picks one.
struct ServerFolderPicker: View {
    let client: MoonriseClient
    let onPick: (LibraryRoot) -> Void
    @State private var listing: MoonriseClient.FolderListing?
    @State private var path: String?
    @State private var loading = true
    @State private var error: String?

    var body: some View {
        content
            .task(id: path) { await load() }
    }

    private var title: String { path.map { ($0 as NSString).lastPathComponent } ?? "Server Folders" }
    private var empty: String {
        path == nil
            ? "The server can't see any folders. Make your videos available to it (in Docker: mount the folder into the container), or use a network share."
            : "No folders in here."
    }

    #if os(tvOS)
    private var content: some View {
        TVSetupScreen(icon: .symbol("internaldrive"), title: title,
                      message: path == nil ? "Folders on the server's computer. Open the one that holds your videos, then choose it." : (path ?? "")) {
            VStack(alignment: .leading, spacing: 22) {
                if let path, let listing {
                    Button { onPick(LibraryRoot(source: .local, path: path, videoFiles: listing.videoFiles)) } label: {
                        Label("Use This Folder", systemImage: "checkmark").font(.body.bold()).frame(maxWidth: .infinity)
                    }
                    .tvGlassButton(prominent: true)
                    Text(videosText(listing.videoFiles, folders: listing.dirs.count)).font(.callout).foregroundStyle(.secondary).padding(.leading, 24)
                }
                folderList
            }
        }
    }

    private var folderList: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: tvRowSpacing) {
                if path != nil {
                    TVPickRow(symbol: "arrow.turn.left.up", title: "Up", chevron: false) { path = listing?.parent }
                }
                ForEach(listing?.dirs ?? []) { d in
                    TVPickRow(symbol: "folder", title: d.name) { path = d.path }
                }
                if loading { ProgressView().padding(24) }
                if let error { Text(error).font(.callout).foregroundStyle(.red).padding(.leading, 24) }
                else if !loading, listing?.dirs.isEmpty == true { Text(empty).font(.callout).foregroundStyle(.secondary).padding(.leading, 24) }
            }
            .padding(.vertical, 24).padding(.horizontal, 48)
        }
        .frame(height: tvListHeight(rows: (listing?.dirs.count ?? 0) + (path == nil ? 0 : 1), max: path == nil ? 640 : 520))
        // Room for a focused row's plate to grow, the rows' text still on the column's edge + 24.
        .padding(.horizontal, -24)
    }
    #else
    private var content: some View {
        Form {
            if let path, let listing {
                Section {
                    Button { onPick(LibraryRoot(source: .local, path: path, videoFiles: listing.videoFiles)) } label: {
                        Label("Use This Folder", systemImage: "checkmark.circle")
                    }
                    .formRow()
                } header: { Text(path) } footer: { Text(videosText(listing.videoFiles, folders: listing.dirs.count)) }
            }
            Section {
                if path != nil {
                    Button { path = listing?.parent } label: { Label("Up", systemImage: "arrow.turn.left.up") }.formRow()
                }
                ForEach(listing?.dirs ?? []) { d in
                    Button { path = d.path } label: { folderRow(d.name) }.formRow()
                }
                if loading { ProgressView() }
            } header: { Text(path == nil ? "Folders on the Server" : "Folders") } footer: {
                if let error { Text(error).foregroundStyle(.red) }
                else if !loading, listing?.dirs.isEmpty == true { Text(empty) }
                else if path == nil, !loading { Text("Open the folder that holds your videos, then choose it.") }
            }
        }
        .formStyle(.grouped)
        .navigationTitle(title)
        .inlineTitle()
    }
    #endif

    private func load() async {
        loading = true
        defer { loading = false }
        do {
            listing = try await client.browseServer(path)
            error = nil
        } catch {
            if !error.isCancellation { self.error = serverErrorText(error) }
        }
    }
}

func videosText(_ n: Int, folders: Int) -> String {
    let v = n == 1 ? "1 video" : "\(n) videos"
    let f = folders == 1 ? "1 folder" : "\(folders) folders"
    let here = switch (n, folders) {
    case (0, 0): "Nothing in here yet."
    case (0, _): "\(f) in here."
    case (_, 0): "\(v) in here."
    default: "\(v) and \(f) in here."
    }
    return here + " Moonrise looks through every folder inside the one you choose."
}

#if !os(tvOS)
func folderRow(_ name: String, symbol: String = "folder") -> some View {
    HStack {
        Label(name, systemImage: symbol).foregroundStyle(.white)
        Spacer()
        Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
    }
    .contentShape(Rectangle())
}
#endif

// MARK: Network share

/// Picks a folder in a network share through the server (it reads the share itself): file
/// servers it sees on its network or an address → shares → folders. A user name and password
/// only when the share turns guests away.
struct SharePicker: View {
    let client: MoonriseClient
    let onPick: (LibraryRoot) -> Void

    private enum Stage: Equatable {
        case servers
        case login(host: String)
        case shares(host: String)
        case folders(host: String, share: String, path: String)
    }

    @State private var stage = Stage.servers
    @State private var found: [MoonriseClient.FileServer] = []
    @State private var discovering = true
    @State private var address = ""
    @State private var username = ""
    @State private var password = ""
    @State private var login: ShareLogin?
    @State private var shares: [MoonriseClient.Share] = []
    @State private var listing: MoonriseClient.ShareListing?
    /// The share and folder that asked for an account (opened again once signed in).
    @State private var pending: (share: String, path: String)?
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        content
            .task { await discover() }
            .task(id: stage) { if case let .folders(h, s, p) = stage { await browse(h, s, p) } }
    }

    private var title: String {
        switch stage {
        case .servers: "Network Share"
        case let .login(host): host
        case let .shares(host): host
        case let .folders(_, share, path): path.isEmpty ? share : (path as NSString).lastPathComponent
        }
    }

    private var message: String {
        switch stage {
        case .servers: "File servers the Moonrise server sees on your network. Pick yours, or type its name or address."
        case .login: "This file server doesn't let guests in here. Enter an account on it: the Moonrise server keeps it, only to read your videos."
        case .shares: "Shares on this file server. Pick the one with your videos."
        case let .folders(host, share, path): ([host, share] + path.split(separator: "/").map(String.init)).joined(separator: " › ")
        }
    }

    #if os(tvOS)
    private var content: some View {
        TVSetupScreen(icon: .symbol("network"), title: title, message: message) {
            VStack(alignment: .leading, spacing: 22) {
                switch stage {
                case .servers:
                    TVField(label: "Address") {
                        MoonriseTextField(title: "File server address", text: $address, prompt: "e.g. 192.168.1.20 or nas.local")
                            .autocorrectionDisabled()
                            .onSubmit { Task { await connect(address) } }
                    }
                    Button { Task { await connect(address) } } label: { BusyLabel(title: "Connect", busy: busy) }
                        .tvGlassButton(prominent: true)
                        .disabled(address.trimmingCharacters(in: .whitespaces).isEmpty || busy)
                    errorText
                    list {
                        ForEach(found) { s in TVPickRow(symbol: "server.rack", title: s.name, detail: s.host) { Task { await connect(s.host) } } }
                        if discovering { ProgressView().padding(24) }
                        else if found.isEmpty { Text("No file servers found on the network.").font(.callout).foregroundStyle(.secondary).padding(.leading, 24) }
                    }
                case let .login(host):
                    TVField(label: "User name") { MoonriseTextField(title: "User name", text: $username).autocorrectionDisabled() }
                    TVField(label: "Password") { MoonriseTextField(title: "Password", text: $password, secure: true) }
                    Button { Task { await connect(host, login: ShareLogin(username: username, password: password)) } } label: { BusyLabel(title: "Connect", busy: busy) }
                        .tvGlassButton(prominent: true)
                        .disabled(username.isEmpty || busy)
                    Button("Other File Server") { stage = .servers; error = nil }.tvGlassButton(height: 66)
                    errorText
                case let .shares(host):
                    errorText
                    list {
                        TVPickRow(symbol: "arrow.turn.left.up", title: "Other File Server", chevron: false) { stage = .servers; error = nil }
                        ForEach(shares) { s in TVPickRow(symbol: "externaldrive.connected.to.line.below", title: s.name, detail: s.comment.flatMap { $0.isEmpty ? nil : $0 }) { stage = .folders(host: host, share: s.name, path: "") } }
                    }
                case let .folders(host, share, path):
                    if let listing {
                        Button { Task { await use(host, share, path) } } label: {
                            HStack(spacing: 16) { Label("Use This Folder", systemImage: "checkmark").font(.body.bold()); if busy { ProgressView() } }
                                .frame(maxWidth: .infinity)
                        }
                        .tvGlassButton(prominent: true)
                        .disabled(busy)
                        Text(videosText(listing.videoFiles, folders: listing.dirs.count)).font(.callout).foregroundStyle(.secondary).padding(.leading, 24)
                    }
                    errorText
                    list {
                        TVPickRow(symbol: "arrow.turn.left.up", title: path.isEmpty ? "Shares" : "Up", chevron: false) { up(host, share, path) }
                        ForEach(listing?.dirs ?? []) { d in TVPickRow(symbol: "folder", title: d.name) { stage = .folders(host: host, share: share, path: d.path) } }
                        if busy && listing == nil { ProgressView().padding(24) }
                    }
                }
            }
        }
    }

    private var listRows: Int {
        switch stage {
        case .servers: max(found.count, 1)
        case .login: 0
        case .shares: shares.count + 1
        case .folders: (listing?.dirs.count ?? 1) + 1
        }
    }

    @ViewBuilder private var errorText: some View {
        if let error { Text(error).font(.callout).foregroundStyle(.red).padding(.leading, 24) }
    }

    private func list<C: View>(@ViewBuilder _ rows: () -> C) -> some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: tvRowSpacing) { rows() }
                .padding(.vertical, 24).padding(.horizontal, 48)
        }
        .frame(height: tvListHeight(rows: listRows, max: 520))
        .padding(.horizontal, -24)
    }
    #else
    private var content: some View {
        Form {
            switch stage {
            case .servers:
                Section {
                    ForEach(found) { s in
                        Button { Task { await connect(s.host) } } label: {
                            HStack {
                                Label {
                                    VStack(alignment: .leading) {
                                        Text(s.name).foregroundStyle(.white)
                                        Text(s.host).font(.caption).foregroundStyle(.secondary)
                                    }
                                } icon: { Image(systemName: "server.rack") }
                                Spacer()
                                Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
                            }
                            .contentShape(Rectangle())
                        }
                        .formRow()
                    }
                    if discovering { HStack { ProgressView(); Text("Looking for file servers…").foregroundStyle(.secondary) } }
                    else if found.isEmpty { Text("None found on the network.").foregroundStyle(.secondary) }
                } header: { Text("On the Network") } footer: { Text(message) }
                Section("Or Type Its Address") {
                    TextField("e.g. 192.168.1.20 or nas.local", text: $address)
                        .autocorrectionDisabled()
                        #if os(iOS)
                        .textInputAutocapitalization(.never)
                        .keyboardType(.URL)
                        #endif
                        .onSubmit { Task { await connect(address) } }
                        .accessibilityIdentifier("shareAddress")
                    Button { Task { await connect(address) } } label: { BusyLabel(title: "Connect", busy: busy) }
                        .disabled(address.trimmingCharacters(in: .whitespaces).isEmpty || busy)
                }
            case let .login(host):
                Section {
                    TextField("User name", text: $username)
                        .autocorrectionDisabled()
                        #if os(iOS)
                        .textInputAutocapitalization(.never)
                        #endif
                        .accessibilityIdentifier("shareUser")
                    SecureField("Password", text: $password).accessibilityIdentifier("sharePass")
                    Button { Task { await connect(host, login: ShareLogin(username: username, password: password)) } } label: { BusyLabel(title: "Connect", busy: busy) }
                        .disabled(username.isEmpty || busy)
                } header: { Text("Account on \(host)") } footer: { Text(message) }
                Section { Button("Other File Server") { stage = .servers; error = nil }.formRow() }
            case let .shares(host):
                Section {
                    ForEach(shares) { s in
                        Button { stage = .folders(host: host, share: s.name, path: "") } label: { folderRow(s.name, symbol: "externaldrive.connected.to.line.below") }.formRow()
                    }
                } header: { Text("Shares") } footer: { Text(message) }
                Section { Button("Other File Server") { stage = .servers; error = nil }.formRow() }
            case let .folders(host, share, path):
                if let listing {
                    Section {
                        Button { Task { await use(host, share, path) } } label: {
                            HStack { Label("Use This Folder", systemImage: "checkmark.circle"); if busy { Spacer(); ProgressView() } }
                        }
                        .disabled(busy)
                        .formRow()
                    } header: { Text(message) } footer: { Text(videosText(listing.videoFiles, folders: listing.dirs.count)) }
                }
                Section("Folders") {
                    Button { up(host, share, path) } label: { Label(path.isEmpty ? "Shares" : "Up", systemImage: "arrow.turn.left.up") }.formRow()
                    ForEach(listing?.dirs ?? []) { d in
                        Button { stage = .folders(host: host, share: share, path: d.path) } label: { folderRow(d.name) }.formRow()
                    }
                    if busy && listing == nil { ProgressView() }
                }
            }
            if let error { Section { Text(error).foregroundStyle(.red) } }
        }
        .formStyle(.grouped)
        .navigationTitle(title)
        .inlineTitle()
    }
    #endif

    private func up(_ host: String, _ share: String, _ path: String) {
        error = nil
        if path.isEmpty { stage = .shares(host: host); return }
        let parent = (path as NSString).deletingLastPathComponent
        stage = .folders(host: host, share: share, path: parent == "/" ? "" : parent)
    }

    private func discover() async {
        discovering = true
        found = (try? await client.discoverFileServers()) ?? []
        discovering = false
    }

    private func connect(_ host: String, login: ShareLogin? = nil) async {
        let host = host.trimmingCharacters(in: .whitespaces)
        guard !host.isEmpty, !busy else { return }
        busy = true
        defer { busy = false }
        do {
            shares = try await client.shares(host: host, login: login)
            self.login = login
            error = nil
            if let p = pending {
                pending = nil
                stage = .folders(host: host, share: p.share, path: p.path)
            } else {
                stage = .shares(host: host)
            }
        } catch let e as MoonriseClientError {
            switch e.shareProblem?.problem {
            case "auth_required":
                error = nil
                stage = .login(host: host)
            case "wrong_password":
                error = "The user name or password is wrong."
                stage = .login(host: host)
            case "unreachable":
                error = "The Moonrise server can't reach \(host). Check the name or address."
            default:
                error = e.shareProblem.map { $0.message.prefix(1).uppercased() + $0.message.dropFirst() } ?? serverErrorText(e)
            }
        } catch {
            self.error = serverErrorText(error)
        }
    }

    private func browse(_ host: String, _ share: String, _ path: String) async {
        listing = nil
        busy = true
        defer { busy = false }
        do {
            listing = try await client.browseShare(host: host, share: share, path: path, login: login)
            error = nil
        } catch let e as MoonriseClientError where ["auth_required", "wrong_password", "access_denied"].contains(e.shareProblem?.problem ?? "") {
            // Shares can be listed as a guest while the share itself wants an account.
            pending = (share, path)
            if login != nil { error = e.shareProblem?.problem == "access_denied" ? "This account may not open that share." : "The user name or password is wrong." }
            stage = .login(host: host)
        } catch {
            if !error.isCancellation { self.error = (error as? MoonriseClientError)?.shareProblem.map { $0.message.prefix(1).uppercased() + $0.message.dropFirst() } ?? serverErrorText(error) }
        }
    }

    private func use(_ host: String, _ share: String, _ path: String) async {
        busy = true
        defer { busy = false }
        do {
            let t = try await client.testShare(host: host, share: share, path: path, login: login)
            guard t.ok else { error = "This folder is empty."; return }
            onPick(LibraryRoot(source: .smb, path: path, host: host, share: share, login: login, videoFiles: t.videoFiles))
        } catch {
            self.error = (error as? MoonriseClientError)?.shareProblem?.message ?? serverErrorText(error)
        }
    }
}

// MARK: - Step 4: the first scan, live

struct SetupScanView: View {
    let flow: SetupFlow
    @State private var progress: MoonriseClient.ScanProgress?
    @State private var idlePolls = 0
    @State private var done = false
    @State private var next = false

    private var lib: MoonriseClient.ScanProgress.Library? {
        progress?.libraries.first { $0.id == flow.library?.id } ?? progress?.libraries.first
    }
    private var phaseText: String {
        if done { return "Your library is ready." }
        switch progress?.phase {
        case "scanning": return "Looking for videos…"
        case "identifying":
            if let p = progress, p.identify.total > 0 { return "Recognizing titles… \(p.identify.done) of \(p.identify.total)" }
            return "Recognizing titles…"
        case "metadata": return "Getting artwork and details…"
        case "finishing": return "Finishing…"
        default: return "Starting the scan…"
        }
    }

    private var notes: [String] {
        var out: [String] = []
        if let e = progress?.lastPass?.aborted ?? progress?.lastPass?.error, done { out.append("The scan had a problem: \(e)") }
        if let l = lib, l.settling > 0 {
            out.append(l.settling == 1 ? "1 file is still being copied: it's added a few minutes after it stops changing."
                                       : "\(l.settling) files are still being copied: they're added a few minutes after they stop changing.")
        }
        if let l = lib, done {
            let eps = l.reviewEpisodes ?? 0
            let what = l.review == 1 ? "1 title" : "\(l.review) titles"
            let epText = eps == 1 ? "1 episode" : "\(eps) episodes"
            switch (l.review, eps) {
            case (0, 0): break
            case (_, 0): out.append("\(what) couldn't be recognized for sure: \(l.review == 1 ? "it's" : "they're") in the library under the names of the files and folders.")
            case (0, _): out.append("\(epText) couldn't be matched for sure: \(eps == 1 ? "it's" : "they're") in the library under the file names.")
            default: out.append("\(what) and \(epText) couldn't be recognized for sure: they're in the library under the names of the files and folders.")
            }
        }
        if done, let l = lib, l.files == 0 { out.append("No videos were found in that folder.") }
        if !done { out.append("You can continue: the scan goes on in the background.") }
        return out
    }

    /// Movies, shows and episodes as they're recognized; the files found until then.
    private var stats: [(Int, String)] {
        let files = max(lib?.files ?? 0, progress?.phase == "scanning" ? progress?.walked ?? 0 : 0)
        guard let l = lib, l.movies + l.series > 0 else { return [(files, files == 1 ? "File" : "Files"), (0, "Movies"), (0, "Shows")] }
        return [(l.movies, l.movies == 1 ? "Movie" : "Movies"), (l.series, l.series == 1 ? "Show" : "Shows"), (l.episodes, l.episodes == 1 ? "Episode" : "Episodes")]
    }

    var body: some View {
        content
            .navigationBarBackButtonHidden(true)
            .navigationDestination(isPresented: $next) { SetupExtrasView(flow: flow) }
            .task { await poll() }
    }

    private var statsRow: some View {
        HStack(alignment: .firstTextBaseline, spacing: statSpacing) {
            ForEach(Array(stats.enumerated()), id: \.offset) { _, s in
                VStack(alignment: .leading, spacing: 4) {
                    Text("\(s.0)")
                        .font(.system(size: statSize, weight: .bold, design: .rounded))
                        .monospacedDigit()
                        .contentTransition(.numericText(value: Double(s.0)))
                    Text(s.1).font(.callout).foregroundStyle(Theme.secondaryText)
                }
            }
            Spacer(minLength: 0)
        }
        .animation(.snappy, value: stats.map(\.0))
    }

    private var phaseRow: some View {
        HStack(spacing: 12) {
            if done { Image(systemName: "checkmark.circle.fill").foregroundStyle(.green) } else { ProgressView() }
            Text(phaseText)
        }
    }

    #if os(tvOS)
    private let statSize: CGFloat = 96
    private let statSpacing: CGFloat = 70
    private var content: some View {
        TVSetupScreen(icon: .symbol("sparkle.magnifyingglass"), title: flow.library?.name ?? "Scanning",
                      message: "Moonrise is looking through the folder and recognizing your movies and shows.", caption: tvStep(4)) {
            VStack(alignment: .leading, spacing: 36) {
                statsRow.padding(.leading, 24)
                phaseRow.padding(.leading, 24)
                ForEach(notes, id: \.self) { Text($0).font(.callout).foregroundStyle(.secondary).padding(.leading, 24) }
                Button { next = true } label: { Text("Continue").font(.body.bold()).frame(maxWidth: .infinity) }
                    .tvGlassButton(prominent: true)
            }
        }
    }
    #else
    private let statSize: CGFloat = 44
    private let statSpacing: CGFloat = 36
    private var content: some View {
        Form {
            SetupHeader(step: 4, message: "Moonrise is looking through the folder and recognizing your movies and shows.")
            Section {
                statsRow.padding(.vertical, 6)
                phaseRow
            } header: { Text(flow.library?.name ?? "Library") } footer: {
                VStack(alignment: .leading, spacing: 6) { ForEach(notes, id: \.self) { Text($0) } }
            }
            Section {
                Button { next = true } label: { Text("Continue") }.formRow()
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Scanning")
    }
    #endif

    private func poll() async {
        while !Task.isCancelled {
            if let p = try? await flow.client.scanProgress() {
                progress = p
                let finished = p.phase == "idle" && p.passes > flow.passesBefore
                idlePolls = finished ? idlePolls + 1 : 0
                let files = lib?.files ?? 0
                if finished, files > 0 || idlePolls >= 3 { done = true }
            }
            try? await Task.sleep(for: .seconds(done ? 5 : 1))
        }
    }
}

// MARK: - Step 5: optional extras

/// An optional service: its state in a line, then what Moonrise does with it and without it (the
/// setup's Extras and Manage Server's Services).
struct ServiceSummary: View {
    let service: MoonriseClient.Service
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            #if os(tvOS)
            Text(service.name).font(.headline)
            #endif
            Label(service.summary, systemImage: Self.icon(service.state)).foregroundStyle(.white)
            Text("**With it:** \(service.with)").font(.callout).foregroundStyle(Theme.secondaryText)
            Text("**Without it:** \(service.without)").font(.callout).foregroundStyle(Theme.secondaryText)
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    static func icon(_ state: String) -> String {
        switch state {
        case "configured", "builtin": "checkmark.circle"
        case "off": "minus.circle"
        default: "circle.dashed"
        }
    }
}

struct SetupExtrasView: View {
    let flow: SetupFlow
    @State private var services: [MoonriseClient.Service] = []
    @State private var loading = true
    @State private var tmdbKey = ""
    @State private var osUser = ""
    @State private var osPass = ""
    @State private var llmKey = ""
    @State private var busy = false
    @State private var error: String?

    private let message = "Moonrise works without any of these. Each adds something; add your own keys or accounts now or skip — nothing here is required."

    private var changes: [String: Any] {
        var c: [String: Any] = [:]
        let t = { (s: String) in s.trimmingCharacters(in: .whitespacesAndNewlines) }
        if !t(tmdbKey).isEmpty { c["tmdb.api_key"] = t(tmdbKey) }
        if !t(osUser).isEmpty, !osPass.isEmpty { c["opensubtitles.username"] = t(osUser); c["opensubtitles.password"] = osPass }
        if !t(llmKey).isEmpty { c["llm.api_key"] = t(llmKey) }
        return c
    }

    var body: some View {
        content
            .navigationBarBackButtonHidden(true)
            .task { await load() }
    }

    private func service(_ id: String) -> MoonriseClient.Service? { services.first { $0.id == id } }

    #if os(tvOS)
    private var content: some View {
        TVSetupScreen(icon: .symbol("puzzlepiece.extension"), title: "Extras", message: message, caption: tvStep(5)) {
            ScrollView {
                VStack(alignment: .leading, spacing: 34) {
                    HStack(spacing: 20) {
                        Button { Task { await save() } } label: { BusyLabel(title: changes.isEmpty ? "Finish" : "Save and Finish", busy: busy) }
                            .tvGlassButton(prominent: true)
                            .disabled(busy)
                        if !changes.isEmpty {
                            Button("Skip") { flow.finish() }.tvGlassButton()
                        }
                    }
                    if let error { Text(error).font(.callout).foregroundStyle(.red).padding(.leading, 24) }
                    if loading { ProgressView() }
                    ForEach(services) { s in
                        VStack(alignment: .leading, spacing: 14) {
                            ServiceSummary(service: s).padding(.leading, 24)
                            if s.state != "off" { fields(s.id) }
                        }
                    }
                }
                .padding(.vertical, 30).padding(.horizontal, 28)
            }
            .frame(height: 860)
        }
    }

    @ViewBuilder private func fields(_ id: String) -> some View {
        switch id {
        case "tmdb": MoonriseTextField(title: "Your TMDB API key", text: $tmdbKey, prompt: "Your TMDB API key (optional)")
        case "opensubtitles":
            MoonriseTextField(title: "OpenSubtitles user name", text: $osUser, prompt: "OpenSubtitles user name (optional)")
            MoonriseTextField(title: "OpenSubtitles password", text: $osPass, prompt: "OpenSubtitles password", secure: true)
        case "llm": MoonriseTextField(title: "Anthropic API key", text: $llmKey, prompt: "Anthropic API key (optional)")
        default: EmptyView()
        }
    }
    #else
    private var content: some View {
        Form {
            SetupHeader(step: 5, message: message)
            if loading { Section { ProgressView() } }
            ForEach(services) { s in
                Section {
                    ServiceSummary(service: s)
                    if s.state != "off" { fields(s.id) }
                } header: { Text(s.name) }
            }
            if let error { Section { Text(error).foregroundStyle(.red) } }
            Section {
                Button { Task { await save() } } label: { BusyLabel(title: changes.isEmpty ? "Finish" : "Save and Finish", busy: busy) }
                    .disabled(busy)
                    .formRow()
                if !changes.isEmpty { Button("Skip") { flow.finish() }.formRow() }
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Extras")
    }

    @ViewBuilder private func fields(_ id: String) -> some View {
        switch id {
        case "tmdb":
            SecureField("Your TMDB API key (optional)", text: $tmdbKey).accessibilityIdentifier("tmdbKey")
        case "opensubtitles":
            TextField("OpenSubtitles user name (optional)", text: $osUser)
                .autocorrectionDisabled()
                #if os(iOS)
                .textInputAutocapitalization(.never)
                #endif
            SecureField("OpenSubtitles password", text: $osPass)
        case "llm":
            SecureField("Anthropic API key (optional)", text: $llmKey)
        default: EmptyView()
        }
    }
    #endif

    private func load() async {
        loading = true
        defer { loading = false }
        do {
            services = try await flow.client.services()
        } catch {
            self.error = serverErrorText(error)
        }
    }

    private func save() async {
        let c = changes
        guard !c.isEmpty else { flow.finish(); return }
        busy = true
        defer { busy = false }
        do {
            try await flow.client.updateSettings(c)
            flow.finish()
        } catch {
            self.error = serverErrorText(error)
        }
    }
}
