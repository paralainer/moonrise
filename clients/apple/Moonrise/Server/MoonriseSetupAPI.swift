import Foundation

// The server's setup and admin API as the onboarding uses it (server: claim.rs, admin.rs,
// library/api.rs, services.rs): claiming a new server with its setup code, picking a library's
// folder (the server's own folders, or a network share), the first scan's progress, and the
// optional services.

extension MoonriseClient {
    // MARK: Claim

    /// `POST /v1/setup/verify`: is this the setup code? (204; 403 wrong; 409 set up already; 429.)
    func verifySetupCode(_ code: String) async throws {
        _ = try await send("POST", url("v1/setup/verify"), body: ["code": code], authorized: false)
    }

    /// `POST /v1/setup/claim`: the first account (an admin), signed in on this device.
    func claim(code: String, username: String, password: String) async throws -> AuthResult {
        let body: [String: Any] = ["code": code, "username": username, "password": password].merging(device) { a, _ in a }
        let r: AuthResult = try await post("v1/setup/claim", body, authorized: false)
        token = r.token
        return r
    }

    // MARK: Libraries

    struct AdminLibrary: Decodable, Identifiable {
        let id: String
        let name: String
        /// What it was made as (movies, shows, mixed); libraries have no type now.
        var kind: String?
    }

    /// `GET /v1/admin/libraries`'s answer, as the setup reads it.
    struct AdminLibraryList: Decodable { let items: [AdminLibrary] }

    func adminLibraries() async throws -> [AdminLibrary] {
        let a: AdminLibraryList = try await get("v1/admin/libraries")
        return a.items
    }

    /// Adds a library; the server checks its folder and starts a full scan.
    /// Libraries have no type: movies and shows can share one.
    func createLibrary(name: String, root: LibraryRoot) async throws -> AdminLibrary {
        try await post("v1/admin/libraries", ["name": name, "roots": [root.body]])
    }

    // MARK: Folders

    struct Folder: Decodable, Hashable, Identifiable {
        let name: String
        let path: String
        var id: String { path }
    }

    struct FolderListing: Decodable {
        /// nil: the list of browsable roots.
        let path: String?
        let parent: String?
        let dirs: [Folder]
        let videoFiles: Int
        let truncated: Bool?
    }

    /// `GET /v1/admin/fs/browse`: the server's folders (no path: where browsing may start).
    func browseServer(_ path: String?) async throws -> FolderListing {
        try await get("v1/admin/fs/browse", ["path": path])
    }

    // MARK: Network shares

    struct FileServer: Decodable, Hashable, Identifiable {
        let name: String
        let host: String
        var id: String { host }
    }

    struct Share: Decodable, Hashable, Identifiable {
        let name: String
        let comment: String?
        var id: String { name }
    }

    struct ShareListing: Decodable {
        let path: String
        let parent: String?
        let dirs: [Folder]
        let videoFiles: Int
    }

    struct ShareTest: Decodable {
        let ok: Bool
        let url: String
        let entries: Int
        let videoFiles: Int
        let error: String?
    }

    /// `GET /v1/admin/smb/discover`'s answer.
    struct FileServerList: Decodable { let servers: [FileServer] }

    /// File servers on the server's network (Bonjour, NetBIOS).
    func discoverFileServers() async throws -> [FileServer] {
        let a: FileServerList = try await get("v1/admin/smb/discover", ["wait_ms": "2500"], timeout: 15)
        return a.servers
    }

    /// `POST /v1/admin/smb/shares`'s answer.
    struct ShareList: Decodable { let shares: [Share] }

    func shares(host: String, login: ShareLogin?) async throws -> [Share] {
        let a: ShareList = try await post("v1/admin/smb/shares", ["host": host].merging(login?.body ?? [:]) { a, _ in a })
        return a.shares
    }

    func browseShare(host: String, share: String, path: String, login: ShareLogin?) async throws -> ShareListing {
        try await post("v1/admin/smb/browse", ["host": host, "share": share, "path": path].merging(login?.body ?? [:]) { a, _ in a })
    }

    func testShare(host: String, share: String, path: String, login: ShareLogin?) async throws -> ShareTest {
        try await post("v1/admin/smb/test", ["host": host, "share": share, "path": path].merging(login?.body ?? [:]) { a, _ in a })
    }

    // MARK: Scan progress

    struct ScanProgress: Decodable {
        struct Identify: Decodable { let done: Int; let total: Int }
        struct Library: Decodable, Identifiable {
            let id: String
            let name: String
            var kind: String?
            let files: Int
            let identified: Int
            let movies: Int
            let series: Int
            let episodes: Int
            let review: Int
            var reviewEpisodes: Int? = nil
            let settling: Int
        }
        struct LastPass: Decodable { let error: String?; let aborted: String?; var at: Int? = nil }
        /// idle, queued, scanning, identifying, metadata, finishing
        let phase: String
        let walked: Int
        let identify: Identify
        let passes: Int
        let lastPass: LastPass?
        let libraries: [Library]
    }

    func scanProgress() async throws -> ScanProgress {
        try await get("v1/admin/scan/progress", timeout: 10)
    }

    // MARK: Optional services

    struct Service: Decodable, Identifiable {
        let id: String
        let name: String
        /// configured, builtin, not_set, off
        let state: String
        let summary: String
        let with: String
        let without: String
        let account: Bool?
        /// OpenSubtitles: downloads a day, and left today.
        let quota: Quota?
        /// AI matching (servers from 0.22): the provider's preset id and name.
        let provider: String?
        let providerName: String?
        /// Ratings (servers from 0.26): the credit the sources ask for.
        var attribution: String? = nil

        struct Quota: Decodable { let tier: String?; let allowed: Int?; let remaining: Int? }
    }

    /// `GET /v1/admin/services`'s answer.
    struct ServiceList: Decodable { let services: [Service] }

    func services() async throws -> [Service] {
        let a: ServiceList = try await get("v1/admin/services")
        return a.services
    }

    /// `PATCH /v1/admin/settings` with dotted keys.
    func updateSettings(_ changes: [String: Any]) async throws {
        _ = try await send("PATCH", url("v1/admin/settings"), body: changes)
    }
}

/// A user name and password for a network share (only asked for when the share needs one).
struct ShareLogin: Hashable {
    var username: String
    var password: String
    var body: [String: Any] { ["username": username, "password": password] }
}

/// A library's folder: on the server's computer, or in a network share.
struct LibraryRoot: Hashable {
    enum Source: Hashable { case local, smb }
    var source: Source
    var path: String
    var host = ""
    var share = ""
    var login: ShareLogin?
    /// Videos directly in the folder, as last listed.
    var videoFiles = 0

    var body: [String: Any] {
        switch source {
        case .local: return ["source": "local", "path": path]
        case .smb: return ["source": "smb", "host": host, "share": share, "path": path].merging(login?.body ?? [:]) { a, _ in a }
        }
    }

    /// As shown: "/media/movies", "nas › Media › movies".
    var display: String {
        switch source {
        case .local: return path
        case .smb: return ([host, share] + path.split(separator: "/").map(String.init)).joined(separator: " › ")
        }
    }
}

extension MoonriseClientError {
    /// A network-share failure's `problem` (auth_required, wrong_password, access_denied,
    /// not_found, unreachable, other) and message, from the server's JSON answer.
    var shareProblem: (problem: String, message: String)? {
        guard case let .http(_, body) = self, let data = body.data(using: .utf8),
              let o = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let p = o["problem"] as? String else { return nil }
        return (p, (o["error"] as? String) ?? p)
    }

    /// The server's plain-text reason (shown as it is).
    var serverMessage: String? {
        guard case let .http(_, body) = self, !body.isEmpty else { return nil }
        return body
    }
}
