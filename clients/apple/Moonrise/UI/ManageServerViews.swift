import SwiftUI

// Manage Server (Settings → Server, admins only; the server has no web admin): libraries and
// scans, Fix Match, accounts and devices, the optional services, playback and hardware, what the
// server is doing, and about it (log, restart). Server side: admin.rs, manage.rs, services.rs,
// library/api.rs. Apple TV gets a reduced set (no Fix Match, no editing of libraries, accounts or
// keys — typing with the remote; see each screen).

// MARK: - Who may manage

/// Whether the signed-in account is an admin (asked of the server; the section is hidden
/// otherwise — the server refuses non-admins anyway).
@MainActor
@Observable
final class AdminState {
    static let shared = AdminState()
    private(set) var isAdmin = false
    private var checked: UUID?

    var client: MoonriseClient? { SourceStore.shared.server.flatMap { MoonriseClient.for($0) } }

    func refresh() async {
        guard let s = SourceStore.shared.server, let c = MoonriseClient.for(s) else { isAdmin = false; checked = nil; return }
        if checked != s.id { isAdmin = false }
        do {
            isAdmin = try await c.me().isAdmin
            checked = s.id
        } catch MoonriseClientError.http(401, _) {
            isAdmin = false
        } catch {}
    }
}

#if DEBUG
/// Test seam: MOONRISE_MANAGE=libraries|review|users|services|playback|activity|about|log opens
/// Settings → Manage Server (→ that screen); `fix:<item id>` opens Fix Match for the item.
enum ManageSeam {
    static let value = DebugSeams.launch.manage
    @MainActor static var consumed = false
    static var screen: ManageScreen? { value.flatMap { ManageScreen(rawValue: $0) } }
    static var opensSettings: Bool { value != nil && !(value?.hasPrefix("fix:") ?? false) }
}
#endif

enum ManageScreen: String, Identifiable, Hashable {
    case libraries, review, users, services, aiMatching, playback, activity, about, log
    var id: String { rawValue }
}

// MARK: - Shared pieces

/// Dates the server sends (RFC 3339), relative: "2 hours ago".
enum AdminFmt {
    private static let iso: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()
    private static let isoPlain = ISO8601DateFormatter()

    static func date(_ s: String?) -> Date? {
        guard let s else { return nil }
        return iso.date(from: s) ?? isoPlain.date(from: s)
    }

    static func ago(_ s: String?) -> String? {
        guard let d = date(s) else { return nil }
        return ago(d)
    }

    static func ago(_ d: Date) -> String {
        if abs(d.timeIntervalSinceNow) < 60 { return "just now" }
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .short
        return f.localizedString(for: d, relativeTo: Date())
    }

    static func uptime(_ secs: Int) -> String {
        let d = secs / 86_400, h = secs % 86_400 / 3600, m = secs % 3600 / 60
        if d > 0 { return "\(d) day\(d == 1 ? "" : "s"), \(h) h" }
        if h > 0 { return "\(h) h \(m) min" }
        return "\(max(m, 1)) min"
    }

    /// Sizes as the system shows disk space in binary units ("3 GB" for a 3 GiB limit); "None" for 0.
    static func bytes(_ n: Int64) -> String {
        n <= 0 ? "None" : ByteCountFormatter.string(fromByteCount: n, countStyle: .binary)
    }

    static func count(_ n: Int, _ one: String, _ many: String? = nil) -> String {
        "\(n.formatted()) \(n == 1 ? one : (many ?? one + "s"))"
    }
}

/// A Manage Server screen's frame: on Apple TV the Settings layout (what the screen is about on
/// the left, the list on the right, rows on glass); elsewhere an inline title.
struct ManagePage: ViewModifier {
    let title: String
    let symbol: String
    var message: String? = nil

    func body(content: Content) -> some View {
        #if os(tvOS)
        content
            .buttonStyle(TVGlassRowStyle(cornerRadius: 40, outset: CGSize(width: 20, height: 14), scale: 1.02))
            .safeAreaPadding(.leading, 600)
            .overlay(alignment: .leading) {
                VStack(alignment: .leading, spacing: 28) {
                    Spacer(minLength: 0)
                    Image(systemName: symbol)
                        .font(.system(size: 80, weight: .light))
                        .frame(width: 180, height: 180)
                        .background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 40, style: .continuous))
                    Text(title).font(.system(size: 56, weight: .bold)).lineLimit(2).minimumScaleFactor(0.6)
                    if let message { Text(message).font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true) }
                    Spacer(minLength: 0)
                }
                .foregroundStyle(.white)
                .frame(width: 520, alignment: .leading)
                .frame(maxHeight: .infinity)
                .allowsHitTesting(false)
            }
            .toolbar(.hidden, for: .navigationBar)
        #else
        content
            .navigationTitle(title)
            .inlineTitle()
        #endif
    }
}

extension View {
    func managePage(_ title: String, symbol: String, message: String? = nil) -> some View {
        modifier(ManagePage(title: title, symbol: symbol, message: message))
    }
}

/// A value in a row. On Apple TV a row has to take focus for the list to scroll to it, so it
/// is a button that does nothing (or `action`).
struct InfoRow: View {
    let title: String
    var value: String?
    var symbol: String?
    var tint: Color?
    var action: (() -> Void)?

    var body: some View {
        #if os(tvOS)
        Button { action?() } label: { content }
        #else
        if let action {
            Button(action: action) { content.contentShape(Rectangle()) }.formRow().formRowButton()
        } else {
            content
        }
        #endif
    }

    /// Title on the left and the value on the right when both fit on one line; otherwise the value
    /// goes under the title, leading-aligned like every other two-line row (never a ragged,
    /// right-aligned block in the middle of the row).
    private var content: some View {
        ViewThatFits(in: .horizontal) {
            LabeledContent {
                if let value { valueText(value).lineLimit(1) }
            } label: {
                labeled(Text(title).foregroundStyle(.white))
            }
            labeled(
                VStack(alignment: .leading, spacing: 3) {
                    Text(title).foregroundStyle(.white)
                    if let value { valueText(value).multilineTextAlignment(.leading).fixedSize(horizontal: false, vertical: true) }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            )
        }
    }

    private func valueText(_ value: String) -> some View {
        Text(value).foregroundStyle(tint ?? Theme.secondaryText)
    }

    @ViewBuilder private func labeled(_ text: some View) -> some View {
        if let symbol {
            Label { text } icon: { Image(systemName: symbol).foregroundStyle(tint ?? .white) }
        } else {
            text
        }
    }
}

/// Any row content that has to take focus on Apple TV (a button that does nothing there).
struct InfoRowContent<Content: View>: View {
    @ViewBuilder let content: () -> Content
    var body: some View {
        #if os(tvOS)
        Button {} label: { content().frame(maxWidth: .infinity, alignment: .leading) }
        #else
        content()
        #endif
    }
}

/// Text in a section of its own (a note, an error) that also takes focus on Apple TV.
struct NoteRow: View {
    let text: String
    var color: Color = Theme.secondaryText
    var body: some View {
        #if os(tvOS)
        Button {} label: { Text(text).font(.callout).foregroundStyle(color).frame(maxWidth: .infinity, alignment: .leading) }
        #else
        Text(text).font(.callout).foregroundStyle(color)
        #endif
    }
}

/// A row that opens a screen: icon, title, a detail on the right.
private struct ManageLink<Destination: View>: View {
    let title: String
    let symbol: String
    var detail: String?
    var detailColor: Color = Theme.secondaryText
    @ViewBuilder let destination: () -> Destination

    var body: some View {
        NavigationLink {
            destination()
        } label: {
            LabeledContent {
                if let detail { Text(detail).foregroundStyle(detailColor) }
            } label: {
                Label { Text(title).foregroundStyle(.white) } icon: { Image(systemName: symbol).foregroundStyle(.white) }
            }
        }
    }
}

// MARK: - Manage Server

struct ManageServerView: View {
    @State private var libraries: Int?
    @State private var review: Int?
    @State private var accounts: Int?
    @State private var scanning = false
    @State private var seamScreen: ManageScreen?

    private var client: MoonriseClient? { AdminState.shared.client }

    var body: some View {
        Form { Group {
            Section {
                ManageLink(title: "Libraries", symbol: "square.stack", detail: scanning ? "Scanning…" : libraries.map { AdminFmt.count($0, "library", "libraries") }) { LibrariesAdminView() }
                #if !os(tvOS)
                ManageLink(title: "Fix Match", symbol: "wand.and.stars", detail: review.map { $0 == 0 ? "Nothing to fix" : "\($0) waiting" },
                           detailColor: (review ?? 0) > 0 ? .orange : Theme.secondaryText) { ReviewListView() }
                #endif
            } header: { Text("Library") }
            Section {
                ManageLink(title: "Users & Devices", symbol: "person.2", detail: accounts.map { AdminFmt.count($0, "user") }) { AccountsAdminView() }
            } header: { Text("People") }
            Section {
                ManageLink(title: "Services", symbol: "puzzlepiece.extension") { ServicesAdminView() }
                ManageLink(title: "Playback & Hardware", symbol: "cpu") { PlaybackAdminView() }
                ManageLink(title: "Activity", symbol: "waveform.path.ecg") { ActivityAdminView() }
                ManageLink(title: "About This Server", symbol: "info.circle") { AboutServerView() }
            } header: { Text("Server") }
        }.tvGlassRows() }
        .formStyle(.grouped)
        .managePage("Manage Server", symbol: "server.rack", message: "Libraries, accounts and the server's services. Only admins see this.")
        .task { await load() }
        #if DEBUG
        .navigationDestination(item: $seamScreen) { s in
            switch s {
            case .libraries: LibrariesAdminView()
            case .review:
                #if os(tvOS)
                LibrariesAdminView()
                #else
                ReviewListView()
                #endif
            case .users: AccountsAdminView()
            case .services: ServicesAdminView()
            case .aiMatching:
                #if os(tvOS)
                ServicesAdminView()
                #else
                AIMatchingAdminView()
                #endif
            case .playback: PlaybackAdminView()
            case .activity: ActivityAdminView()
            case .about: AboutServerView()
            case .log: ServerLogView()
            }
        }
        .task {
            if let s = ManageSeam.screen, !ManageSeam.consumed {
                ManageSeam.consumed = true
                try? await Task.sleep(for: .milliseconds(400))
                seamScreen = s
            }
        }
        #endif
    }

    private func load() async {
        guard let c = client else { return }
        async let libs = try? c.managedLibraries()
        async let progress = try? c.scanProgress()
        async let users = try? c.accounts()
        if let l = await libs { libraries = l.items.count }
        if let p = await progress {
            review = p.libraries.reduce(0) { $0 + $1.review + ($1.reviewEpisodes ?? 0) }
            scanning = !["idle", "queued"].contains(p.phase)
        }
        if let u = await users { accounts = u.count }
    }
}

// MARK: - Libraries

/// The library's passes, polled while a libraries screen is open.
@MainActor
@Observable
final class ScanWatcher {
    var progress: MoonriseClient.ScanProgress?
    var error: String?

    var busy: Bool { progress.map { $0.phase != "idle" } ?? false }

    var phaseText: String {
        guard let p = progress else { return "—" }
        switch p.phase {
        case "queued": return "Starting a scan…"
        case "scanning": return p.walked > 0 ? "Looking for videos… \(p.walked) files" : "Looking for videos…"
        case "identifying": return p.identify.total > 0 ? "Recognizing titles… \(p.identify.done) of \(p.identify.total)" : "Recognizing titles…"
        case "metadata": return "Getting artwork and details…"
        case "finishing": return "Finishing…"
        default:
            if let at = p.lastPass?.at {
                let d = Date(timeIntervalSince1970: TimeInterval(at))
                return Date().timeIntervalSince(d) < 60 ? "Up to date · scanned just now" : "Up to date · scanned \(AdminFmt.ago(d))"
            }
            return "Up to date"
        }
    }

    /// Polls fast for a while (a scan was just asked for: a quick one is over in a second or two).
    @ObservationIgnored private var fastUntil = Date.distantPast
    func kick() { fastUntil = Date().addingTimeInterval(20) }

    func watch(_ client: MoonriseClient) async {
        while !Task.isCancelled {
            do {
                progress = try await client.scanProgress()
                error = nil
            } catch {
                if !error.isCancellation { self.error = serverErrorText(error) }
            }
            try? await Task.sleep(for: .seconds(busy || Date() < fastUntil ? 0.7 : 3))
        }
    }

    func library(_ id: String) -> MoonriseClient.ScanProgress.Library? { progress?.libraries.first { $0.id == id } }

    /// What a library holds (libraries have no type): "300 movies", "42 shows · 2,589 episodes",
    /// "12 movies · 3 shows · 30 episodes"; the files until something is recognized.
    func counts(_ id: String) -> String? {
        guard let l = library(id) else { return nil }
        var parts: [String] = []
        if l.movies > 0 { parts.append(AdminFmt.count(l.movies, "movie")) }
        if l.series > 0 { parts += [AdminFmt.count(l.series, "show"), AdminFmt.count(l.episodes, "episode")] }
        return parts.isEmpty ? AdminFmt.count(l.files, "file") : parts.joined(separator: " · ")
    }
}

struct LibrariesAdminView: View {
    @State private var list: MoonriseClient.LibraryList?
    @State private var scan = ScanWatcher()
    @State private var error: String?
    @State private var adding = false
    @State private var asked: String?

    private var client: MoonriseClient? { AdminState.shared.client }
    private var editable: Bool { list?.managedBy != "env" }

    var body: some View {
        Form { Group {
            Section {
                InfoRowContent {
                    Label {
                        Text(scan.phaseText).foregroundStyle(.white)
                    } icon: {
                        if scan.busy { ProgressView() } else { Image(systemName: "checkmark.circle.fill").foregroundStyle(.green) }
                    }
                }
                if let last = scan.progress?.lastPass, let e = last.aborted ?? last.error {
                    NoteRow(text: "Last scan: \(e)", color: .orange)
                }
                Button { start(nil) } label: { Label(asked == "*" ? "Scan Started" : "Scan All Libraries", systemImage: "arrow.clockwise").formRow() }
                    .formRowButton()
                    .disabled(scan.busy || asked == "*")
            } header: { Text("Scanning") } footer: {
                Text("Moonrise looks for new and changed files every few minutes on its own. Scan Now lists every folder again.")
            }
            ForEach(list?.items ?? []) { lib in
                Section {
                    let counts = Label { Text(scan.counts(lib.id) ?? "Movies and shows").foregroundStyle(.white) } icon: {
                        Image(systemName: "film.stack").foregroundStyle(.white)
                    }
                    #if os(tvOS)
                    InfoRowContent { counts }
                    #else
                    NavigationLink { LibraryEditView(library: lib, editable: editable) { await reload() } } label: { counts }
                    #endif
                    ForEach(lib.roots, id: \.self) { root in RootRow(root: root) }
                    Button { start(lib.id) } label: { Label(asked == lib.id ? "Scan Started" : "Scan Now", systemImage: "arrow.clockwise").formRow() }
                        .formRowButton()
                        .disabled(scan.busy || asked == lib.id)
                } header: { Text(lib.name) } footer: {
                    if let l = scan.library(lib.id), l.review + (l.reviewEpisodes ?? 0) > 0 {
                        #if os(tvOS)
                        Text("\(AdminFmt.count(l.review + (l.reviewEpisodes ?? 0), "title")) waiting in Fix Match (on iPhone, iPad or Mac).")
                        #else
                        Text("\(AdminFmt.count(l.review + (l.reviewEpisodes ?? 0), "title")) waiting in Fix Match.")
                        #endif
                    }
                }
            }
            #if !os(tvOS)
            if editable {
                Section {
                    Button { adding = true } label: { Label("Add Library", systemImage: "plus").formRow() }.formRowButton()
                } footer: {
                    if list?.items.isEmpty == true { Text("No libraries yet: add a folder with your movies or shows.") }
                }
            } else if list != nil {
                Section { NoteRow(text: "The libraries are set by MOONRISE_LIBRARIES in the server's environment: remove it there to manage them here.") }
            }
            #endif
            if let error = error ?? scan.error { Section { NoteRow(text: error, color: .red) } }
        }.tvGlassRows() }
        .formStyle(.grouped)
        .managePage("Libraries", symbol: "square.stack", message: "Where your videos are, how many Moonrise found, and whether each folder can be reached.")
        .task { await reload() }
        .task { if let c = client { await scan.watch(c) } }
        #if !os(tvOS)
        .navigationDestination(isPresented: $adding) { AddLibraryView { adding = false; Task { await reload() } } }
        #endif
    }

    private func reload() async {
        guard let c = client else { return }
        do {
            list = try await c.managedLibraries()
            error = nil
        } catch {
            if !error.isCancellation { self.error = serverErrorText(error) }
        }
    }

    private func start(_ library: String?) {
        guard let c = client else { return }
        asked = library ?? "*"
        scan.kick()
        Task {
            do {
                try await c.startScan(library: library, full: library == nil)
            } catch {
                self.error = serverErrorText(error)
            }
            try? await Task.sleep(for: .seconds(4))
            asked = nil
        }
    }
}

/// A library folder: where it is, and whether the server can reach it.
private struct RootRow: View {
    let root: MoonriseClient.ManagedLibrary.Root
    var body: some View {
        #if os(tvOS)
        Button {} label: { content.frame(maxWidth: .infinity, alignment: .leading) }
        #else
        content
        #endif
    }

    private var content: some View {
        Label {
            VStack(alignment: .leading, spacing: 3) {
                Text(root.display).foregroundStyle(.white).lineLimit(2)
                HStack(spacing: 6) {
                    Circle().fill(root.online ? Color.green : .red).frame(width: 8, height: 8)
                    Text((root.online ? "Online" : "Offline\(root.error.map { ": \($0)" } ?? "")") + (root.username.map { " · as \($0)" } ?? ""))
                        .font(.caption).foregroundStyle(root.online ? Theme.secondaryText : .red).lineLimit(2)
                }
            }
        } icon: {
            Image(systemName: root.source == "smb" ? "network" : "internaldrive").foregroundStyle(.white)
        }
    }
}

#if !os(tvOS)
/// A library: its name, its folders (add, remove), remove the library.
private struct LibraryEditView: View {
    let library: MoonriseClient.ManagedLibrary
    let editable: Bool
    let changed: () async -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name: String
    @State private var roots: [MoonriseClient.ManagedLibrary.Root]
    @State private var picking: LibraryRoot.Source?
    @State private var busy = false
    @State private var error: String?
    @State private var confirmRemove = false
    @State private var saved = false

    init(library: MoonriseClient.ManagedLibrary, editable: Bool, changed: @escaping () async -> Void) {
        self.library = library
        self.editable = editable
        self.changed = changed
        _name = State(initialValue: library.name)
        _roots = State(initialValue: library.roots)
    }

    private var client: MoonriseClient? { AdminState.shared.client }
    private var trimmed: String { name.trimmingCharacters(in: .whitespaces) }

    var body: some View {
        Form {
            Section {
                TextField("Name", text: $name).disabled(!editable).accessibilityIdentifier("libraryName")
                if trimmed != library.name, !trimmed.isEmpty {
                    Button { Task { await rename() } } label: { BusyLabel(title: "Rename", busy: busy) }.formRow().disabled(busy)
                }
            } header: { Text("Name") }
            Section {
                ForEach(roots, id: \.self) { r in RootRow(root: r) }
                    .onDelete(perform: editable && roots.count > 1 ? { idx in Task { await removeRoots(idx) } } : nil)
                if editable {
                    Menu {
                        Button { picking = .local } label: { Label("A Folder on the Server", systemImage: "internaldrive") }
                        Button { picking = .smb } label: { Label("A Network Share", systemImage: "network") }
                    } label: { Label("Add Folder", systemImage: "plus").formRow() }
                    .formRowButton()
                    .disabled(busy)
                }
            } header: { Text("Folders") } footer: {
                Text(roots.count > 1 ? "Swipe a folder to remove it. Changing the folders starts a full scan." : "Changing the folders starts a full scan.")
            }
            if editable {
                Section {
                    Button(role: .destructive) { confirmRemove = true } label: { Text("Remove Library").foregroundStyle(.red).formRow() }
                        .formRowButton()
                        .disabled(busy)
                } footer: {
                    Text("Its titles leave Moonrise; the files themselves aren't touched.")
                }
            }
            if let error { Section { Text(error).foregroundStyle(.red) } }
        }
        .formStyle(.grouped)
        .navigationTitle(library.name)
        .inlineTitle()
        .navigationDestination(item: $picking) { source in
            Group {
                if let c = client {
                    switch source {
                    case .local: ServerFolderPicker(client: c) { r in picking = nil; Task { await addRoot(r) } }
                    case .smb: SharePicker(client: c) { r in picking = nil; Task { await addRoot(r) } }
                    }
                }
            }
        }
        .confirmationDialog("Remove \(library.name)?", isPresented: $confirmRemove, titleVisibility: .visible) {
            Button("Remove Library", role: .destructive) { Task { await remove() } }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Its titles and their artwork leave Moonrise, with everyone's watch history for them. The video files stay where they are.")
        }
    }

    private func rename() async {
        guard let c = client else { return }
        busy = true
        defer { busy = false }
        do {
            try await c.renameLibrary(library.id, name: trimmed)
            error = nil
            await changed()
            dismiss()
        } catch { self.error = serverErrorText(error) }
    }

    private func save(_ body: [[String: Any]]) async -> Bool {
        guard let c = client else { return false }
        busy = true
        defer { busy = false }
        do {
            try await c.setLibraryRoots(library.id, roots: body)
            error = nil
            if let fresh = try? await c.managedLibraries().items.first(where: { $0.id == library.id }) { roots = fresh.roots }
            await changed()
            return true
        } catch {
            self.error = serverErrorText(error)
            return false
        }
    }

    private func addRoot(_ r: LibraryRoot) async {
        _ = await save(roots.map(\.body) + [r.body])
    }

    private func removeRoots(_ idx: IndexSet) async {
        var keep = roots
        keep.remove(atOffsets: idx)
        _ = await save(keep.map(\.body))
    }

    private func remove() async {
        guard let c = client else { return }
        busy = true
        defer { busy = false }
        do {
            try await c.deleteLibrary(library.id)
            await changed()
            dismiss()
        } catch { self.error = serverErrorText(error) }
    }
}

/// Another library (as the setup's library step: a name, a folder on the server or a share).
/// Libraries have no type: movies and shows can share one.
private struct AddLibraryView: View {
    let done: () -> Void
    @State private var name = "Media"
    @State private var root: LibraryRoot?
    @State private var picking: LibraryRoot.Source?
    @State private var busy = false
    @State private var error: String?

    private var client: MoonriseClient? { AdminState.shared.client }

    var body: some View {
        Form {
            Section("Library Name") {
                TextField("Name", text: $name)
            }
            Section {
                Button { picking = .local } label: { sourceRow("internaldrive", "A Folder on the Server", "Folders the server's computer can see", root?.source == .local) }.formRow()
                Button { picking = .smb } label: { sourceRow("network", "A Network Share", "A NAS or another computer on your network", root?.source == .smb) }.formRow()
            } header: { Text("Where Are the Videos?") } footer: {
                if let root { Label(root.display, systemImage: "checkmark.circle.fill").foregroundStyle(.green) }
            }
            Section {
                Button { Task { await add() } } label: { BusyLabel(title: "Add Library", busy: busy) }
                    .formRow()
                    .disabled(root == nil || name.trimmingCharacters(in: .whitespaces).isEmpty || busy)
            } footer: { Text("Movies and shows can share a folder: Moonrise tells them apart. It scans the folder right away; Libraries shows how it goes.") }
            if let error { Section { Text(error).foregroundStyle(.red) } }
        }
        .formStyle(.grouped)
        .navigationTitle("Add Library")
        .inlineTitle()
        .navigationDestination(item: $picking) { source in
            Group {
                if let c = client {
                    switch source {
                    case .local: ServerFolderPicker(client: c) { root = $0; picking = nil }
                    case .smb: SharePicker(client: c) { root = $0; picking = nil }
                    }
                }
            }
        }
    }

    private func sourceRow(_ symbol: String, _ title: String, _ detail: String, _ chosen: Bool) -> some View {
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

    private func add() async {
        guard let root, let c = client, !busy else { return }
        busy = true
        defer { busy = false }
        do {
            _ = try await c.createLibrary(name: name.trimmingCharacters(in: .whitespaces), root: root)
            done()
        } catch { self.error = serverErrorText(error) }
    }
}
#endif

// MARK: - Services

struct ServicesAdminView: View {
    @State private var services: [MoonriseClient.Service] = []
    @State private var loading = true
    @State private var tmdbKey = ""
    @State private var osUser = ""
    @State private var osPass = ""
    @State private var llmKey = ""
    @State private var omdbKey = ""
    @State private var busy = false
    @State private var error: String?
    @State private var note: String?

    private var client: MoonriseClient? { AdminState.shared.client }

    private var changes: [String: Any] {
        var c: [String: Any] = [:]
        let t = { (s: String) in s.trimmingCharacters(in: .whitespacesAndNewlines) }
        if !t(tmdbKey).isEmpty { c["tmdb.api_key"] = t(tmdbKey) }
        if !t(osUser).isEmpty, !osPass.isEmpty { c["opensubtitles.username"] = t(osUser); c["opensubtitles.password"] = osPass }
        if !t(llmKey).isEmpty { c["llm.api_key"] = t(llmKey) }
        if !t(omdbKey).isEmpty { c["omdb.api_key"] = t(omdbKey) }
        return c
    }

    var body: some View {
        Form { Group {
            if loading { Section { ProgressView() } }
            #if os(tvOS)
            Section {
                ForEach(services) { s in
                    InfoRowStack(title: s.name, detail: s.summary + (quotaText(s).map { " \($0) today." } ?? ""), symbol: ServiceSummary.icon(s.state))
                }
            }
            #else
            ForEach(services) { s in
                Section {
                    ServiceSummary(service: s)
                    if let q = quotaText(s) { LabeledContent("Downloads Today", value: q) }
                    if s.id == "llm", s.provider != nil {
                        // Servers from 0.22: any provider, on its own screen.
                        NavigationLink { AIMatchingAdminView() } label: {
                            LabeledContent { Text(s.providerName ?? "").foregroundStyle(Theme.secondaryText) } label: { Text("Provider").foregroundStyle(.white) }
                        }
                        .accessibilityIdentifier("llmProviderLink")
                    } else if s.state != "off" {
                        fields(s.id)
                    }
                } header: { Text(s.name) } footer: {
                    if let a = s.attribution { Text(a) }
                }
            }
            #endif
            #if os(tvOS)
            if !services.isEmpty {
                Section { NoteRow(text: "Add your own keys and accounts on iPhone, iPad or Mac: Settings → Manage Server → Services.") }
            }
            #else
            if !services.isEmpty {
                Section {
                    Button { Task { await save() } } label: { BusyLabel(title: "Save", busy: busy) }
                        .formRow()
                        .disabled(changes.isEmpty || busy)
                } footer: {
                    Text("Keys and passwords are kept on the server; they're never shown again. An OpenSubtitles account is checked before it's saved.")
                }
            }
            #endif
            if let note { Section { NoteRow(text: note, color: .green) } }
            if let error { Section { NoteRow(text: error, color: .red) } }
        }.tvGlassRows() }
        .formStyle(.grouped)
        .managePage("Services", symbol: "puzzlepiece.extension", message: "Moonrise works without any of these; each adds something.")
        .task { await load() }
    }

    private func quotaText(_ s: MoonriseClient.Service) -> String? {
        guard s.id == "opensubtitles", let q = s.quota, let allowed = q.allowed else { return nil }
        return "\(q.remaining ?? allowed) of \(allowed) left"
    }

    #if !os(tvOS)
    @ViewBuilder private func fields(_ id: String) -> some View {
        switch id {
        case "tmdb":
            SecureField("Your TMDB API key", text: $tmdbKey).accessibilityIdentifier("tmdbKey")
        case "opensubtitles":
            TextField("OpenSubtitles user name", text: $osUser)
                .autocorrectionDisabled()
                #if os(iOS)
                .textInputAutocapitalization(.never)
                .textContentType(.username)
                #endif
                .accessibilityIdentifier("osUser")
            SecureField("OpenSubtitles password", text: $osPass).accessibilityIdentifier("osPass")
        case "llm":
            SecureField("Anthropic API key", text: $llmKey).accessibilityIdentifier("llmKey")
        case "ratings":
            SecureField("Your OMDb API key (free at omdbapi.com)", text: $omdbKey).accessibilityIdentifier("omdbKey")
        default: EmptyView()
        }
    }
    #endif

    private func load() async {
        guard let c = client else { return }
        loading = true
        defer { loading = false }
        do {
            services = try await c.services()
            error = nil
        } catch {
            if !error.isCancellation { self.error = serverErrorText(error) }
        }
    }

    private func save() async {
        guard let c = client else { return }
        let ch = changes
        guard !ch.isEmpty else { return }
        busy = true
        defer { busy = false }
        note = nil
        do {
            if let u = ch["opensubtitles.username"] as? String, let p = ch["opensubtitles.password"] as? String,
               let why = try await c.checkOpenSubtitles(username: u, password: p) {
                error = why
                return
            }
            try await c.updateSettings(ch)
            tmdbKey = ""; osUser = ""; osPass = ""; llmKey = ""; omdbKey = ""
            error = nil
            note = "Saved."
            services = (try? await c.services()) ?? services
        } catch {
            self.error = serverErrorText(error)
        }
    }
}

// MARK: - Playback & hardware

/// One `performance.*` setting as the server's settings document describes it.
private struct PerfSetting: Identifiable {
    let key: String
    let title: String
    let unit: String
    let range: ClosedRange<Int>
    var value: Int
    /// live, restart, env-only
    var apply: String
    /// env, file, store, builtin, default
    var source: String
    var env: String?
    var pending: Bool
    var id: String { key }

    static let known: [(String, String, String, ClosedRange<Int>)] = [
        ("performance.transcode_jobs", "Transcodes at Once", "", 1...16),
        ("performance.convert_jobs", "Audio Conversions at Once", "", 1...64),
        ("performance.jobs", "Background Jobs", "", 1...64),
        ("performance.download_jobs", "Download Jobs at Once", "", 1...8),
        ("performance.cache_max_gb", "Stream Cache", "GB", 1...100_000),
        ("performance.download_max_gb", "Prepared Downloads", "GB", 1...100_000),
        ("performance.scan_quick_secs", "Quick Scan Every", "s", 10...86_400),
        ("performance.scan_full_hours", "Full Scan Every", "h", 1...720),
        ("performance.scan_probe_jobs", "Scan Reads at Once", "", 1...16),
    ]
}

struct PlaybackAdminView: View {
    @State private var system: MoonriseClient.SystemInfo?
    @State private var settings: [PerfSetting] = []
    @State private var edited: [String: Int] = [:]
    @State private var busy = false
    @State private var error: String?
    @State private var note: String?
    @State private var confirmRestart = false
    /// `transcode.hardware` as stored (auto, nvidia, qsv, vaapi, videotoolbox, v4l2, cpu); nil on
    /// older servers.
    @State private var hardware: String?
    @State private var hardwareLocked = false
    /// `features.index_backfill` (nil: a server before 0.23), and whether the environment sets it.
    @State private var prepareAhead: Bool?
    @State private var prepareLocked = false
    @State private var preparing: MoonriseClient.Activity.IndexBackfill?
    /// `features.trickplay` (nil: a server before 0.27), whether the environment sets it, and
    /// its progress and space.
    @State private var thumbnails: Bool?
    @State private var thumbnailsLocked = false
    @State private var thumbnailing: MoonriseClient.Activity.Trickplay?

    private var client: MoonriseClient? { AdminState.shared.client }

    var body: some View {
        Form { Group {
            if let s = system {
                Section {
                    InfoRow(title: "Video Transcoding", value: transcodingText(s), symbol: "film.stack", tint: Self.isHardware(s.transcoding) ? .green : (s.transcoding == "off" || s.transcoding == "cpu" ? nil : .orange))
                    if let t = s.transcode, s.transcoding != "off", t.available {
                        InfoRow(title: "Decoding", value: t.decode, symbol: "arrow.down.right.and.arrow.up.left")
                        InfoRow(title: "Encoding", value: t.encode + (t.maxHeight < 2160 ? " · up to \(t.maxHeight)p" : ""), symbol: "arrow.up.left.and.arrow.down.right")
                        InfoRow(title: "HDR to SDR", value: t.tonemap, symbol: "sun.max")
                    }
                    ForEach(s.hardware.nvidia, id: \.self) { g in
                        InfoRow(title: g.name ?? "NVIDIA GPU", value: [g.driver.map { "Driver \($0)" }, g.memoryMb.map { "\($0 / 1024) GB" }].compactMap { $0 }.joined(separator: " · "), symbol: "memorychip")
                    }
                    ForEach(s.hardware.otherGpus, id: \.self) { g in
                        InfoRow(title: "\(g.vendor) GPU", value: g.usable ? g.device : "Not passed to the server", symbol: "memorychip")
                    }
                    InfoRowStack(title: cpuName(s.hardware.cpu.model), detail: AdminFmt.count(s.hardware.cpu.threads, "thread"), symbol: "cpu")
                } header: { Text("Hardware") } footer: {
                    Text(([hardwareFooter(s)] + (s.transcode?.notes ?? [])).joined(separator: " "))
                }
                if let t = s.transcode, let current = hardware {
                    Section {
                        #if os(tvOS)
                        InfoRow(title: "Transcode With", value: Self.choices(t).first { $0.0 == current }?.1 ?? current)
                        #else
                        Picker("Transcode With", selection: Binding(get: { current }, set: { v in Task { await setHardware(v) } })) {
                            ForEach(Self.choices(t), id: \.0) { Text($0.1).tag($0.0) }
                        }
                        .disabled(busy || hardwareLocked)
                        #endif
                    } footer: {
                        Text(hardwareLocked ? "Set by MOONRISE_HWACCEL in the server's environment." :
                            Self.automaticText(t) + " A choice that doesn't work falls back to Automatic. Takes effect when the server restarts." + (t.requested != current ? " Waiting for a restart." : ""))
                    }
                }
            }
            if !settings.isEmpty {
                Section {
                    ForEach(settings.filter { $0.apply != "live" }) { p in perfRow(p) }
                } header: { Text("Streaming and Jobs") } footer: {
                    Text("These take effect when the server restarts.")
                }
                Section {
                    ForEach(settings.filter { $0.apply == "live" }) { p in perfRow(p) }
                } header: { Text("Scanning") } footer: {
                    #if os(tvOS)
                    Text("These take effect at once. Change them on iPhone, iPad or Mac.")
                    #else
                    Text("These take effect at once. A setting given in the server's environment can't be changed here.")
                    #endif
                }
                #if !os(tvOS)
                Section {
                    Button { Task { await save() } } label: { BusyLabel(title: "Save", busy: busy) }.formRow().disabled(edited.isEmpty || busy)
                }
                #endif
            }
            if let on = prepareAhead {
                Section {
                    #if os(tvOS)
                    InfoRow(title: "Prepare Files Ahead", value: on ? "On" : "Off")
                    #else
                    Toggle("Prepare Files Ahead", isOn: Binding(get: { on }, set: { v in Task { await setPrepareAhead(v) } }))
                        .switchTint().disabled(busy || prepareLocked)
                    #endif
                    if on { InfoRow(title: "Progress", value: Self.preparingText(preparing)) }
                } header: { Text("Preparing Files") } footer: {
                    Text(prepareLocked ? "Switched off by MOONRISE_DISABLE in the server's environment." :
                        "AVI and similar files are read once before their first play, so it starts at once. One file at a time, paused while anything plays.")
                }
            }
            if let on = thumbnails {
                Section {
                    #if os(tvOS)
                    InfoRow(title: "Scrubbing Thumbnails", value: on ? "On" : "Off")
                    #else
                    Toggle("Scrubbing Thumbnails", isOn: Binding(get: { on }, set: { v in Task { await setThumbnails(v) } }))
                        .switchTint().disabled(busy || thumbnailsLocked)
                    #endif
                    if on { InfoRow(title: "Progress", value: Self.thumbnailsText(thumbnailing)) }
                    if let used = Self.thumbnailSpaceText(thumbnailing) { InfoRow(title: "Space Used", value: used) }
                } header: { Text("Scrubbing Thumbnails") } footer: {
                    Text(thumbnailsLocked ? "Switched off by MOONRISE_DISABLE in the server's environment." :
                        "A picture every 10 seconds of every title, shown above the scrubber while seeking. Made one file at a time, paused while anything plays. When the space is full, titles you play push out the thumbnails used longest ago.")
                }
            }
            if let s = system, !s.restartRequired.isEmpty {
                Section {
                    NoteRow(text: "Waiting for a restart: \(s.restartRequired.map(title).joined(separator: ", ")).", color: .orange)
                    if s.canRestart {
                        Button { confirmRestart = true } label: { Label("Restart Server", systemImage: "arrow.clockwise.circle").formRow() }.formRowButton()
                    }
                }
            }
            if let note { Section { NoteRow(text: note, color: .green) } }
            if let error { Section { NoteRow(text: error, color: .red) } }
        }.tvGlassRows() }
        .formStyle(.grouped)
        .managePage("Playback & Hardware", symbol: "cpu", message: "What the server transcodes with, and how much it does at once.")
        .task { await load() }
        .task {
            // The progress of Preparing Files.
            while !Task.isCancelled {
                if let c = client, let a = try? await c.activity() { preparing = a.indexBackfill; thumbnailing = a.trickplay }
                try? await Task.sleep(for: .seconds(5))
            }
        }
        .restartDialog(isPresented: $confirmRestart, client: client) { note = nil; await load() }
    }

    private func title(_ key: String) -> String { PerfSetting.known.first { $0.0 == key }?.1 ?? (key == "transcode.hardware" ? "Transcode With" : key) }

    /// "Intel(R) Core(TM) i5-7500 CPU @ 3.40GHz" → "Intel Core i5-7500 CPU @ 3.40GHz".
    private func cpuName(_ model: String?) -> String {
        guard let m = model, !m.isEmpty else { return "CPU" }
        return m.replacingOccurrences(of: "(R)", with: "").replacingOccurrences(of: "(TM)", with: "").replacingOccurrences(of: "  ", with: " ")
    }

    static let hardwareChoices: [(String, String)] = [("auto", "Automatic"), ("nvidia", "NVIDIA"), ("qsv", "Intel Quick Sync"), ("vaapi", "VA-API (Intel, AMD)"), ("cpu", "CPU")]

    /// The server's choices (0.18.0: what fits its platform), else the Linux ones.
    static func choices(_ t: MoonriseClient.SystemInfo.Transcode) -> [(String, String)] {
        guard let c = t.choices, !c.isEmpty else { return hardwareChoices }
        return c.map { ($0.key, $0.label) }
    }

    /// "Automatic tries VideoToolbox, then the CPU."
    static func automaticText(_ t: MoonriseClient.SystemInfo.Transcode) -> String {
        let names = choices(t).filter { $0.0 != "auto" }.map { $0.0 == "cpu" ? "the CPU" : $0.1 }
        guard names.count > 1 else { return "Automatic finds what transcodes here." }
        return "Automatic tries " + names.dropLast().joined(separator: ", then ") + ", else " + names.last! + "."
    }

    /// A GPU or another video engine transcodes (anything but the CPU, off, unavailable).
    static func isHardware(_ transcoding: String) -> Bool { !["cpu", "off", "unavailable"].contains(transcoding) }

    private func transcodingText(_ s: MoonriseClient.SystemInfo) -> String {
        switch s.transcoding {
        case "off": return "Switched off"
        case "unavailable": return s.transcode == nil ? "No NVIDIA GPU" : "Not available"
        default: return s.transcode?.label ?? (s.transcoding == "nvenc" ? "NVIDIA NVENC" : s.transcoding)
        }
    }

    private func hardwareFooter(_ s: MoonriseClient.SystemInfo) -> String {
        switch s.transcoding {
        case "nvenc", "qsv", "vaapi": return "Lower qualities, codecs Apple devices can't play and HDR for SDR screens are made on the GPU. Everything else is streamed as it is."
        case "videotoolbox": return "Lower qualities, codecs Apple devices can't play and HDR for SDR screens are made by the Mac's media engine (VideoToolbox). Everything else is streamed as it is."
        case "v4l2": return "Lower qualities (up to 1080p) and codecs Apple devices can't play are made by the board's video encoder. 4K and 10-bit HEVC titles are too much for it: they play as they are. Everything else is streamed as it is."
        case "cpu": return "No GPU is passed to the server: transcodes run on the CPU (up to 1080p), which can be slow on a small machine. Everything Apple devices play directly is streamed as it is."
        case "off": return "Transcoding is switched off: titles stream as they are (audio is still converted when needed)."
        case "unavailable": return s.transcode == nil
            ? "Transcoding needs an NVIDIA GPU passed to the server (NVIDIA Container Toolkit). Without one, titles stream as they are: what Apple devices can play directly plays fine."
            : "Nothing on the server can transcode: titles stream as they are; what Apple devices can play directly plays fine."
        default: return "Lower qualities, codecs Apple devices can't play and HDR for SDR screens are made by the server's video hardware. Everything else is streamed as it is."
        }
    }

    /// "120 of 281", "Paused while playing · 120 of 281", "All files ready".
    static func preparingText(_ b: MoonriseClient.Activity.IndexBackfill?) -> String {
        guard let b else { return "…" }
        let files = b.files ?? 0, done = b.done ?? 0, failed = b.failed ?? 0
        let failedText = failed > 0 ? " · \(failed) failed" : ""
        switch b.state {
        case "off": return "Off"
        case "starting", nil: return "Starting"
        case "paused" where done + failed < files: return "Paused while playing · \(done) of \(files)"
        case "running" where done + failed < files: return "\(done) of \(files)" + failedText
        default: return "All files ready" + failedText
        }
    }

    /// "120 of 281", "Paused while playing · 120 of 281", "Space full · 120 of 281", "All titles
    /// ready · 2 skipped".
    static func thumbnailsText(_ t: MoonriseClient.Activity.Trickplay?) -> String {
        guard let t else { return "…" }
        let files = t.files ?? 0, done = t.done ?? 0, failed = t.failed ?? 0, skipped = t.skipped ?? 0
        let extra = (failed > 0 ? " · \(failed) failed" : "") + (skipped > 0 ? " · \(skipped) skipped" : "")
        switch t.state {
        case "off": return "Off"
        case "starting", nil: return "Starting"
        case "full": return "Space full" + (files > 0 ? " · \(done) of \(files)" : "")
        case "paused" where done + failed < files: return "Paused while playing · \(done) of \(files)"
        case "running" where done + failed < files: return "\(done) of \(files)" + extra
        default: return "All titles ready" + extra
        }
    }

    /// "1.2 GB of 10 GB" (nil before the server says).
    static func thumbnailSpaceText(_ t: MoonriseClient.Activity.Trickplay?) -> String? {
        guard let used = t?.bytes, let max = t?.maxBytes, max > 0 else { return nil }
        return "\(AdminFmt.bytes(used)) of \(AdminFmt.bytes(max))"
    }

    private func setThumbnails(_ v: Bool) async {
        guard let c = client else { return }
        busy = true
        defer { busy = false }
        do {
            try await c.updateSettings(["features.trickplay": v])
            thumbnails = v
            note = v ? "Scrubbing thumbnails are made from now on." : "No more scrubbing thumbnails are made or shown."
            if let a = try? await c.activity() { thumbnailing = a.trickplay }
        } catch { self.error = serverErrorText(error) }
    }

    private func setPrepareAhead(_ v: Bool) async {
        guard let c = client else { return }
        busy = true
        defer { busy = false }
        do {
            try await c.updateSettings(["features.index_backfill": v])
            prepareAhead = v
            note = v ? "Files are prepared ahead from now on." : "Files are prepared when first played."
            if let a = try? await c.activity() { preparing = a.indexBackfill }
        } catch { self.error = serverErrorText(error) }
    }

    private func setHardware(_ v: String) async {
        guard let c = client else { return }
        busy = true
        defer { busy = false }
        do {
            try await c.updateSettings(["transcode.hardware": v])
            note = "Saved. Takes effect after a restart."
            await load()
        } catch { self.error = serverErrorText(error) }
    }

    @ViewBuilder private func perfRow(_ p: PerfSetting) -> some View {
        let value = edited[p.key] ?? p.value
        let suffix = p.unit.isEmpty ? "" : " \(p.unit)"
        let locked = p.source == "env" || p.source == "file"
        #if os(tvOS)
        InfoRow(title: p.title, value: "\(value)\(suffix)")
        #else
        VStack(alignment: .leading, spacing: 4) {
            if locked {
                LabeledContent(p.title) { Text("\(value)\(suffix)").monospacedDigit().foregroundStyle(Theme.secondaryText) }
            } else {
                Stepper(value: Binding(get: { value }, set: { edited[p.key] = $0 == p.value ? nil : $0 }), in: p.range, step: step(p)) {
                    LabeledContent(p.title) { Text("\(value)\(suffix)").monospacedDigit().foregroundStyle(edited[p.key] != nil ? .white : Theme.secondaryText) }
                }
            }
            if locked, let env = p.env {
                Text("Set by \(env) in the server's environment").font(.caption).foregroundStyle(.orange)
            } else if p.pending {
                Text("Changed: waiting for a restart").font(.caption).foregroundStyle(.orange)
            }
        }
        #endif
    }

    private func step(_ p: PerfSetting) -> Int {
        switch p.key {
        case "performance.scan_quick_secs": return 30
        case "performance.cache_max_gb", "performance.download_max_gb": return 5
        default: return 1
        }
    }

    private func load() async {
        guard let c = client else { return }
        do {
            async let sys = c.systemInfo()
            async let doc = c.settingsDocument()
            let (s, d) = try await (sys, doc)
            system = s
            let perf = ((d["settings"] as? [String: Any])?["performance"] as? [String: Any]) ?? [:]
            let fields = (d["fields"] as? [String: Any]) ?? [:]
            settings = PerfSetting.known.compactMap { key, title, unit, range in
                let short = String(key.dropFirst("performance.".count))
                guard let v = perf[short] as? Int else { return nil }
                let f = fields[key] as? [String: Any] ?? [:]
                return PerfSetting(key: key, title: title, unit: unit, range: range, value: v, apply: f["apply"] as? String ?? "live",
                                   source: f["source"] as? String ?? "default", env: f["env"] as? String, pending: f["restart_pending"] as? Bool ?? false)
            }
            hardware = ((d["settings"] as? [String: Any])?["transcode"] as? [String: Any])?["hardware"] as? String
            let hf = fields["transcode.hardware"] as? [String: Any] ?? [:]
            hardwareLocked = (hf["source"] as? String) == "env"
            prepareAhead = ((d["settings"] as? [String: Any])?["features"] as? [String: Any])?["index_backfill"] as? Bool
            prepareLocked = ((fields["features.index_backfill"] as? [String: Any])?["source"] as? String) == "env"
            thumbnails = ((d["settings"] as? [String: Any])?["features"] as? [String: Any])?["trickplay"] as? Bool
            thumbnailsLocked = ((fields["features.trickplay"] as? [String: Any])?["source"] as? String) == "env"
            edited = [:]
            error = nil
        } catch {
            if !error.isCancellation { self.error = serverErrorText(error) }
        }
    }

    private func save() async {
        guard let c = client, !edited.isEmpty else { return }
        busy = true
        defer { busy = false }
        do {
            try await c.updateSettings(edited)
            let later = edited.keys.filter { k in settings.first { $0.key == k }?.apply == "restart" }
            note = later.isEmpty ? "Saved: in effect now." : "Saved. \(later.map(title).joined(separator: ", ")) take\(later.count == 1 ? "s" : "") effect after a restart."
            await load()
        } catch { self.error = serverErrorText(error) }
    }
}

extension View {
    /// "Restart the server?" and the restart itself (the server exits; Docker starts it again).
    func restartDialog(isPresented: Binding<Bool>, client: MoonriseClient?, after: @escaping () async -> Void) -> some View {
        confirmationDialog("Restart the server?", isPresented: isPresented, titleVisibility: .visible) {
            Button("Restart", role: .destructive) {
                Task {
                    try? await client?.restartServer()
                    // Back in a few seconds; whoever is watching sees a short stall (the app recovers).
                    for _ in 0..<30 {
                        try? await Task.sleep(for: .seconds(2))
                        if (try? await client?.info(timeout: 2)) != nil { break }
                    }
                    await after()
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("It's back in a few seconds. Anyone watching sees a short pause.")
        }
    }
}

// MARK: - Activity

struct ActivityAdminView: View {
    @State private var activity: MoonriseClient.Activity?
    @State private var error: String?

    private var client: MoonriseClient? { AdminState.shared.client }

    var body: some View {
        Form { Group {
            Section {
                if let a = activity {
                    if a.sessions.isEmpty { NoteRow(text: "Nothing is playing.") }
                    ForEach(a.sessions) { s in sessionRow(s) }
                } else { ProgressView() }
            } header: { Text("Playing Now") }
            if let a = activity {
                Section {
                    InfoRow(title: "Library", value: scanText(a.scan))
                    InfoRow(title: "Video Transcodes", value: a.transcodes.isEmpty ? "None" : "\(a.transcodes.count) of \(a.transcodeSlots) slots")
                    InfoRow(title: "Subtitle Extraction", value: subsText(a.subtitles))
                    if let b = a.subtitles.backfill, let files = b.files, files > 0 {
                        InfoRow(title: "Subtitles Read Ahead", value: "\(b.done ?? 0) of \(files)" + ((b.failed ?? 0) > 0 ? " · \(b.failed ?? 0) failed" : ""))
                    }
                    if let b = a.indexBackfill {
                        InfoRow(title: "Preparing Files", value: PlaybackAdminView.preparingText(b))
                    }
                } header: { Text("Jobs") }
                if !a.downloads.isEmpty {
                    Section {
                        ForEach(a.downloads) { d in
                            InfoRow(title: d.label, value: d.state == "running" ? "\(Int(d.progress * 100))%" : d.state.capitalized, tint: d.state == "failed" ? .red : nil)
                        }
                    } header: { Text("Prepared Downloads") }
                }
                Section {
                    if a.errors.isEmpty { NoteRow(text: "No warnings or errors since the server started.") }
                    ForEach(Array(a.errors.suffix(12).reversed().enumerated()), id: \.offset) { _, line in
                        LogLine(line: line)
                    }
                } header: { Text("Recent Problems") }
            }
            if let error { Section { NoteRow(text: error, color: .red) } }
        }.tvGlassRows() }
        .formStyle(.grouped)
        .managePage("Activity", symbol: "waveform.path.ecg", message: "What the server is doing right now.")
        .task {
            while !Task.isCancelled {
                await load()
                try? await Task.sleep(for: .seconds(4))
            }
        }
    }

    private func sessionRow(_ s: MoonriseClient.Activity.Session) -> some View {
        let title = s.seriesTitle.map { "\($0) · \(s.episodeLabel ?? "")" } ?? s.title ?? "A title"
        let how: String = switch s.method {
        case "transcode": "Transcoding" + (s.variant.map { " · \($0)" } ?? "") + (s.tonemap == true ? " · HDR→SDR" : "")
        case "convert": "Direct · audio converted"
        default: "Direct"
        }
        let pos = s.position.map { p in s.duration.map { "\(Fmt.duration(p)) of \(Fmt.duration($0))" } ?? Fmt.duration(p) }
        let line = [s.user, how, pos].compactMap { $0 }.joined(separator: " · ")
        return InfoRowStack(title: title, detail: line, symbol: s.method == "transcode" ? "gearshape.2" : "play.circle")
    }

    private func scanText(_ s: MoonriseClient.Activity.Scan?) -> String {
        guard let s else { return "Off" }
        switch s.phase {
        case "idle", nil:
            if let at = s.lastPass?.at, let ago = AdminFmt.ago(at) { return "Last scanned \(ago)" }
            return "Idle"
        case "queued": return "Scan starting"
        default: return "Scanning (\(s.phase ?? ""))"
        }
    }

    private func subsText(_ s: MoonriseClient.Activity.Subtitles) -> String {
        if s.extractions.isEmpty { return "Idle" }
        return AdminFmt.count(s.extractions.count, "title") + (s.extractions.contains { $0.urgent } ? " · for playback" : "")
    }

    private func load() async {
        guard let c = client else { return }
        do {
            activity = try await c.activity()
            error = nil
        } catch {
            if !error.isCancellation { self.error = serverErrorText(error) }
        }
    }
}

/// A title on one line, a detail under it.
struct InfoRowStack: View {
    let title: String
    let detail: String
    var symbol: String?
    var body: some View {
        #if os(tvOS)
        Button {} label: { content.frame(maxWidth: .infinity, alignment: .leading) }
        #else
        content
        #endif
    }
    @ViewBuilder private var content: some View {
        let text = VStack(alignment: .leading, spacing: 3) {
            Text(title).foregroundStyle(.white)
            Text(detail).font(.caption).foregroundStyle(Theme.secondaryText)
        }
        if let symbol {
            Label { text } icon: { Image(systemName: symbol).foregroundStyle(.white) }
        } else {
            text
        }
    }
}

/// A log line: its time and level, then the message.
private struct LogLine: View {
    let line: String

    private var parts: (time: String, level: String, message: String) {
        let w = line.split(separator: " ", maxSplits: 3, omittingEmptySubsequences: true).map(String.init)
        guard w.count >= 3 else { return ("", "", line) }
        let time = AdminFmt.date(w[0]).map { $0.formatted(date: .omitted, time: .standard) } ?? w[0]
        return (time, w[1], w.dropFirst(2).joined(separator: " "))
    }

    var body: some View {
        let p = parts
        let text = VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 8) {
                Text(p.level).font(.caption.weight(.bold)).foregroundStyle(p.level == "ERROR" ? .red : (p.level == "WARN" ? .orange : Theme.secondaryText))
                Text(p.time).font(.caption).foregroundStyle(Theme.secondaryText)
            }
            Text(p.message).font(.system(.caption, design: .monospaced)).foregroundStyle(.white).lineLimit(6)
        }
        #if os(tvOS)
        Button {} label: { text.frame(maxWidth: .infinity, alignment: .leading) }
        #else
        text.textSelection(.enabled)
        #endif
    }
}

// MARK: - About

struct AboutServerView: View {
    @State private var system: MoonriseClient.SystemInfo?
    @State private var error: String?
    @State private var confirmRestart = false

    private var client: MoonriseClient? { AdminState.shared.client }

    var body: some View {
        Form { Group {
            if let s = system {
                Section {
                    InfoRow(title: "Name", value: s.name)
                    InfoRow(title: "Version", value: s.version)
                    InfoRow(title: "Running For", value: AdminFmt.uptime(s.uptimeSecs))
                    if let addr = SourceStore.shared.server?.address { InfoRow(title: "Address", value: addr.replacingOccurrences(of: "http://", with: "")) }
                } header: { Text("Server") }
                Section {
                    InfoRow(title: "Database", value: AdminFmt.bytes(s.sizes.database))
                    InfoRow(title: "Subtitles", value: AdminFmt.bytes(s.sizes.subtitles))
                    InfoRow(title: "Artwork", value: AdminFmt.bytes(s.sizes.artwork))
                    InfoRow(title: "Stream Cache", value: "\(AdminFmt.bytes(s.sizes.cache)) of \(AdminFmt.bytes(s.cacheMaxBytes))")
                    InfoRow(title: "Prepared Downloads", value: "\(AdminFmt.bytes(s.downloadsBytes)) of \(AdminFmt.bytes(s.downloadMaxBytes))")
                } header: { Text("Storage") } footer: {
                    Text("The server's data (\(AdminFmt.bytes(s.sizes.data)) in all) holds the library, accounts and watch history; the cache is refilled as needed.")
                }
                #if !os(tvOS)
                Section {
                    NavigationLink { ServerLogView() } label: { Label("Server Log", systemImage: "doc.plaintext") }
                    if s.canRestart {
                        Button { confirmRestart = true } label: { Label("Restart Server", systemImage: "arrow.clockwise.circle").formRow() }.formRowButton()
                    }
                } footer: {
                    Text(s.canRestart ? "The log shows the server's recent lines, without keys or passwords." :
                        "The log shows the server's recent lines, without keys or passwords. This server can't restart itself: restart it where it runs (docker restart moonrise-server).")
                }
                #else
                if s.canRestart {
                    Section {
                        Button { confirmRestart = true } label: { Label("Restart Server", systemImage: "arrow.clockwise.circle").formRow() }
                    }
                }
                #endif
            } else if error == nil {
                Section { ProgressView() }
            }
            if let error { Section { NoteRow(text: error, color: .red) } }
        }.tvGlassRows() }
        .formStyle(.grouped)
        .managePage("About This Server", symbol: "info.circle", message: SourceStore.shared.server?.name)
        .task { await load() }
        .restartDialog(isPresented: $confirmRestart, client: client) { await load() }
    }

    private func load() async {
        guard let c = client else { return }
        do {
            system = try await c.systemInfo()
            error = nil
        } catch {
            if !error.isCancellation { self.error = serverErrorText(error) }
        }
    }
}

struct ServerLogView: View {
    @State private var lines: [String] = []
    @State private var problemsOnly = false
    @State private var loading = true
    @State private var error: String?

    private var client: MoonriseClient? { AdminState.shared.client }

    var body: some View {
        Form { Group {
            Section {
                Toggle("Warnings and Errors Only", isOn: $problemsOnly).switchTint()
            } footer: { Text("The last \(lines.count) lines, newest first. Keys, passwords and tokens are left out.") }
            Section {
                if loading && lines.isEmpty { ProgressView() }
                ForEach(Array(lines.reversed().enumerated()), id: \.offset) { _, l in LogLine(line: l) }
            }
            if let error { Section { NoteRow(text: error, color: .red) } }
        }.tvGlassRows() }
        .formStyle(.grouped)
        .managePage("Server Log", symbol: "doc.plaintext")
        .task(id: problemsOnly) { await load() }
        .refreshable { await load() }
    }

    private func load() async {
        guard let c = client else { return }
        loading = true
        defer { loading = false }
        do {
            lines = try await c.serverLog(lines: 300, level: problemsOnly ? "warn" : nil)
            error = nil
        } catch {
            if !error.isCancellation { self.error = serverErrorText(error) }
        }
    }
}
