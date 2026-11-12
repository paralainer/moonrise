import Foundation
import Observation

enum SourceKind: String, Codable {
    /// The Moonrise server's own API (`server/`): library, accounts, watch state, streams.
    case moonrise
    /// "Try the Demo": open movies, no server (`DemoClient`). Never stored.
    case demo
    /// Kinds older versions stored. They still decode, so an old list (and the Moonrise server in
    /// it) loads; `SourceStore` drops them on load.
    case jellyfin, smb
}

/// The Moonrise server the app is signed in to. Its token lives in the Keychain.
struct Source: Codable, Identifiable, Hashable {
    var id = UUID()
    var kind: SourceKind
    var name: String
    /// Base URL (http://host:8097).
    var address: String
    var username: String
    /// The server's user id.
    var userId: String?
    /// Server id (/v1/info), used to recognise the server on the tailnet.
    var serverId: String?
    /// Tailnet host (100.x IP or MagicDNS name) used when the LAN address is unreachable.
    var tailnetHost: String?

    var secret: String? {
        get { Keychain.get("source.\(id.uuidString)") }
        nonmutating set { Keychain.set(newValue, for: "source.\(id.uuidString)") }
    }
}

/// The one Moonrise server the app uses (none until the user signs in).
///
/// Downloads, Recents and offline progress reports are stamped with the server's `id`; signing
/// back in to the same server (same `serverId`) reuses it, so they stay connected to it.
@MainActor
@Observable
final class SourceStore {
    static let shared = SourceStore()

    private(set) var server: Source?

    private let fileURL: URL = {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("sources.json")
    }()

    /// tvOS doesn't keep files in Application Support between launches (only caches, which the
    /// system may purge), so there the server lives in UserDefaults. Secrets are in the Keychain.
    /// Stored as a list (older versions kept several): it holds zero or one server.
    private static let defaultsKey = "sources.v1"
    /// The server last signed out of (no secret), so signing in to it again keeps its id.
    private static let previousKey = "sources.previous"

    init() {
        #if os(tvOS)
        let data = UserDefaults.standard.data(forKey: Self.defaultsKey)
        #else
        let data = try? Data(contentsOf: fileURL)
        #endif
        let stored = data.flatMap { try? JSONDecoder().decode([Source].self, from: $0) } ?? []
        server = stored.first { $0.kind == .moonrise }
        let dropped = stored.filter { $0.id != server?.id }
        if !dropped.isEmpty {
            retire(dropped)
            save()
        }
    }

    /// Older versions could keep several servers, and sources of other kinds. Only the first Moonrise
    /// server stays: the others are signed out on their servers, and their secrets, downloads and
    /// Recents are forgotten.
    private func retire(_ sources: [Source]) {
        for s in sources {
            if s.kind == .moonrise, let url = MoonriseClient.normalize(s.address), let token = s.secret {
                let client = MoonriseClient(sourceId: s.id, baseURL: url, token: token)
                Task { await client.logout() }
            }
            s.secret = nil
        }
        let ids = Set(sources.map(\.id))
        Task { @MainActor in DownloadManager.shared.removeAll(fromSources: ids) }
        RecentsStore.forget(sources: ids)
    }

    /// The welcome screen's line.
    static let welcomeMessage = "Connect to your Moonrise server to sign in, or to set up a new one."

    /// The demo is what the app shows (no server).
    var isDemo: Bool { server?.kind == .demo }

    /// "Try the Demo" (onboarding): the app with `DemoClient`'s films, until `exitDemo`. Kept in
    /// memory only: a relaunch starts at the welcome screen again.
    func enterDemo() {
        guard server == nil else { return }
        DemoClient.shared.reset()
        HomeStore.shared.clear()
        DemoMode.isActive = true
        server = DemoClient.source
        Task { await ConnectionRouter.shared.refresh() }
    }

    /// Back to the welcome screen; nothing of the demo stays (its Home, Recents, progress).
    func exitDemo() {
        guard isDemo else { return }
        server = nil
        DemoMode.isActive = false
        DemoClient.shared.reset()
        RecentsStore.forget(sources: [DemoClient.sourceId])
        ConnectionRouter.shared.reset()
        HomeStore.shared.clear()
    }

    /// The server, if `id` is it: downloads and reports carry the id of the server they came from.
    func source(_ id: UUID) -> Source? { server?.id == id ? server : nil }

    /// The client for the server.
    var client: (any MediaServerClient)? { server.flatMap { MediaServers.client(for: $0) } }

    /// Signed in (onboarding): this is the server now.
    func signIn(_ source: Source) {
        if isDemo { exitDemo() }
        var source = source
        let token = source.secret
        if let data = UserDefaults.standard.data(forKey: Self.previousKey),
           let previous = try? JSONDecoder().decode(Source.self, from: data),
           previous.serverId != nil, previous.serverId == source.serverId, previous.id != source.id {
            // The same server again: keep its id, so its downloads and Recents are its own again.
            source.secret = nil
            source.id = previous.id
            source.tailnetHost = source.tailnetHost ?? previous.tailnetHost
            source.secret = token
        }
        UserDefaults.standard.removeObject(forKey: Self.previousKey)
        if let old = server, old.id != source.id { old.secret = nil }
        server = source
        save()
        // Find its route now (Settings shows it; Home waits for it).
        Task { await ConnectionRouter.shared.refresh() }
    }

    /// What the router learned (server id, tailnet host).
    func update(_ source: Source) {
        guard source.id == server?.id else { return }
        server = source
        save()
    }

    /// Signs out: the token is revoked on the server and forgotten here. Downloads stay (they
    /// play offline); progress watched offline is sent first while the token still works.
    func signOut() async {
        guard let s = server else { return }
        if let client = MoonriseClient.for(s) {
            await LocalReports.shared.flush()
            await client.logout()
        }
        UserDefaults.standard.set(try? JSONEncoder().encode(s), forKey: Self.previousKey)
        s.secret = nil
        MoonriseClient.forget(s.id)
        server = nil
        save()
        ConnectionRouter.shared.reset()
        HomeStore.shared.clear()
    }

    private func save() {
        guard !isDemo, let data = try? JSONEncoder().encode(server.map { [$0] } ?? []) else { return }
        #if os(tvOS)
        UserDefaults.standard.set(data, forKey: Self.defaultsKey)
        #else
        try? data.write(to: fileURL, options: .atomic)
        #endif
    }
}
