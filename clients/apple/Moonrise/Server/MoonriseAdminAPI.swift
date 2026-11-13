import Foundation

// The server's admin API as the Manage Server screens use it (server: admin.rs, manage.rs,
// library/api.rs, services.rs): libraries and scans, Fix Match, accounts and their devices,
// invitations, the services, performance settings, what the server is doing, its log.

extension MoonriseClient {
    // MARK: Who is asking

    struct Me: Decodable { let id: String; let name: String; let isAdmin: Bool }

    func me() async throws -> Me { try await get("v1/me", timeout: 10) }

    // MARK: Libraries

    struct ManagedLibrary: Decodable, Identifiable, Hashable {
        struct Root: Decodable, Hashable {
            let source: String
            let path: String
            let available: Bool?
            let state: String?
            let error: String?
            let host: String?
            let share: String?
            let url: String?
            let username: String?
            let credentialId: String?

            /// As shown: "/media/movies", "192.168.1.20 › Media › Movies".
            var display: String {
                guard source == "smb", let host, let share else { return path }
                return ([host, share] + path.split(separator: "/").map(String.init)).joined(separator: " › ")
            }

            var online: Bool { state.map { $0 == "online" } ?? (available ?? true) }

            /// As the server takes it back unchanged (its stored credential is kept).
            var body: [String: Any] {
                guard source == "smb" else { return ["source": "local", "path": path] }
                var b: [String: Any] = ["source": "smb", "host": host ?? "", "share": share ?? "", "path": path]
                if let credentialId { b["credential_id"] = credentialId }
                return b
            }
        }
        let id: String
        let name: String
        /// What it was made as (movies, shows, mixed); libraries have no type now.
        var kind: String?
        let roots: [Root]
    }

    struct LibraryList: Decodable {
        let items: [ManagedLibrary]
        /// "store", or "env" (MOONRISE_LIBRARIES: not editable here).
        let managedBy: String
    }

    func managedLibraries() async throws -> LibraryList { try await get("v1/admin/libraries") }

    func renameLibrary(_ id: String, name: String) async throws {
        _ = try await send("PATCH", url("v1/admin/libraries/\(id)"), body: ["name": name])
    }

    /// The library's folders as a whole (each one checked by the server; a full scan follows).
    func setLibraryRoots(_ id: String, roots: [[String: Any]]) async throws {
        _ = try await send("PATCH", url("v1/admin/libraries/\(id)"), body: ["roots": roots], timeout: 60)
    }

    func deleteLibrary(_ id: String) async throws {
        _ = try await send("DELETE", url("v1/admin/libraries/\(id)"))
    }

    /// Scan Now: one library's folders walked in full (`library`), or every library (`full`).
    func startScan(library: String? = nil, full: Bool = false) async throws {
        var q: [String: String?] = ["wait": "0"]
        if let library { q["library"] = library } else if full { q["full"] = "1" }
        _ = try await send("POST", url("v1/admin/scan", q))
    }

    // MARK: Fix Match

    /// Asks for candidates with `poster_path`. A server that doesn't know it ignores it: no
    /// posters then, only the placeholder.
    private static let serverPosters: [String: String?] = ["server_posters": "1"]

    /// A candidate's `poster_path` if it is one, the form the server's poster route takes.
    static func posterPath(_ path: String?) -> String? {
        guard let path, path.range(of: "^/[A-Za-z0-9_-]{1,64}\\.(jpg|jpeg|png|webp)$", options: .regularExpression)
                == path.startIndex..<path.endIndex else { return nil }
        return path
    }

    /// Where a Fix Match candidate's poster is on this server (nil: no poster, or a path of any
    /// other form, and the row keeps its placeholder). Admins only: load it with `headers`.
    func matchPosterURL(_ path: String?, size: String = "w185") -> URL? {
        Self.posterPath(path).map { url("v1/admin/match/poster", ["path": $0, "size": size]) }
    }

    struct MatchCandidate: Decodable, Identifiable, Hashable {
        let tmdbId: Int
        let title: String
        let originalTitle: String?
        let year: Int?
        let overview: String?
        /// For `matchPosterURL`. Candidates' TMDB `poster` addresses are never read: apps talk only to the server.
        let posterPath: String?
        /// A movie the library has already (its title id and files): matching it adds a version.
        let inLibrary: InLibrary?
        var id: Int { tmdbId }

        struct InLibrary: Decodable, Hashable {
            let item: String?
            let files: Int?
            let titles: Int?
        }
    }

    struct ReviewItem: Decodable, Identifiable, Hashable {
        let id: String
        let kind: String
        let title: String
        let year: Int?
        let note: String?
        let files: [String]?
        let candidates: [MatchCandidate]
        let library: String
        let seriesId: String?
        let seriesTitle: String?
        let seriesTmdbId: Int?
        let season: Int?
        let episode: Int?
        let episodeEnd: Int?

        var isEpisode: Bool { kind == "episode" }
        /// "The A-Team · S1E14", "Alien (1979)".
        var heading: String {
            if isEpisode, let s = seriesTitle { return "\(s) · S\(season ?? 0)E\(episode ?? 0)" + (episodeEnd.map { "–E\($0)" } ?? "") }
            return year.map { "\(title) (\($0))" } ?? title
        }
        var fileName: String? { files?.first.map { ($0 as NSString).lastPathComponent } }
    }

    /// `GET /v1/admin/review`'s answer.
    struct ReviewList: Decodable { let items: [ReviewItem] }

    func reviewList() async throws -> [ReviewItem] {
        let a: ReviewList = try await get("v1/admin/review", Self.serverPosters, timeout: 30)
        return a.items
    }

    /// A title as Fix Match shows it (`compute`: TMDB searched now when no candidates were kept).
    struct MatchInfo: Decodable {
        let id: String
        let kind: String
        let title: String
        let year: Int?
        let state: String
        let note: String?
        let candidates: [MatchCandidate]
        let season: Int?
        let episode: Int?
        /// A part of an episode split over several files (server 0.20).
        let part: Int?
        let seriesTitle: String?
        let seriesTmdbId: Int?
        let tmdbId: Int?
        let local: Bool
        let files: [String]
        /// The server has a TMDB key (searches work).
        let tmdb: Bool
        /// Matched by hand (here, or a file of it): Back to Automatic forgets it (server 0.20).
        let manual: Bool?
        /// A movie's or episode's files, one by one (server 0.20).
        let fileList: [MatchFile]?
    }

    struct MatchFile: Decodable, Identifiable, Hashable {
        let id: String
        let name: String?
        let size: Int64?
        let duration: Double?
        let manual: Bool
        let split: Bool?
    }

    func matchInfo(_ id: String, compute: Bool) async throws -> MatchInfo {
        try await get("v1/admin/items/\(id)/match", Self.serverPosters.merging(compute ? ["compute": "1"] : [:]) { a, _ in a }, timeout: 40)
    }

    /// `GET /v1/admin/match/search`'s answer.
    struct MatchCandidateList: Decodable { let items: [MatchCandidate] }

    func searchMatches(kind: String, query: String, year: Int?) async throws -> [MatchCandidate] {
        let a: MatchCandidateList = try await get("v1/admin/match/search", ["kind": kind, "q": query, "year": year.map(String.init), "server_posters": "1"], timeout: 30)
        return a.items
    }

    struct TMDBSeason: Decodable {
        struct Episode: Decodable, Identifiable, Hashable {
            let episode: Int
            let season: Int
            let title: String?
            let airDate: String?
            /// What the library has of it: titles (parts) and files (server 0.20).
            let inLibrary: MatchCandidate.InLibrary?
            var id: Int { episode }
        }
        let tmdbId: Int
        let title: String?
        let seasons: [Int]
        let season: Int
        let episodes: [Episode]
    }

    func tmdbSeason(show: Int, season: Int?) async throws -> TMDBSeason {
        try await get("v1/admin/match/episodes", ["tmdb_id": String(show), "season": season.map(String.init)], timeout: 30)
    }

    /// Fix Match: a TMDB title (an episode: of that show, with TMDB's season and episode), or
    /// `local` (named from the files). Answers the title's id after the change.
    /// Fix Match. `type` ("movie" or "episode") makes it that type (server 0.15).
    /// `part`: the episode's next part (not another version of it); `fileId`: that file alone.
    func identify(_ id: String, tmdbId: Int?, season: Int? = nil, episode: Int? = nil, local: Bool = false, type: String? = nil,
                  part: Bool = false, keepPart: Int? = nil, fileId: String? = nil) async throws -> String? {
        var body: [String: Any] = [:]
        if local { body["local"] = true }
        if let type { body["type"] = type }
        if let tmdbId { body["tmdb_id"] = tmdbId }
        if let season { body["season"] = season }
        if let episode { body["episode"] = episode }
        if part { body["part"] = keepPart.map { $0 as Any } ?? true }
        if let fileId { body["file_id"] = fileId }
        let a = try await identifyCall(id, body)
        return fileId == nil ? a.item : (a.fileItem ?? a.item)
    }

    struct IdentifyAnswer: Decodable {
        let item: String?
        let fileItem: String?
    }

    /// Back to Automatic: the server forgets the Fix Match choices for the title's files (or one
    /// file's) and identifies them on its own again. Answers the title's id after it.
    func matchAutomatically(_ id: String, fileId: String? = nil) async throws -> IdentifyAnswer {
        var body: [String: Any] = ["automatic": true]
        if let fileId { body["file_id"] = fileId }
        return try await identifyCall(id, body)
    }

    /// Split Off: one of the title's files becomes a title of its own. Answers its id (`fileItem`).
    func splitOff(_ id: String, fileId: String) async throws -> IdentifyAnswer {
        try await identifyCall(id, ["split": true, "file_id": fileId])
    }

    private func identifyCall(_ id: String, _ body: [String: Any]) async throws -> IdentifyAnswer {
        let data = try await send("POST", url("v1/admin/items/\(id)/identify"), body: body, timeout: 60)
        return try Self.decoder.decode(IdentifyAnswer.self, from: data)
    }

    // MARK: Accounts

    struct Account: Decodable, Identifiable, Hashable {
        let id: String
        let name: String
        let isAdmin: Bool
        let createdAt: String?
        let devices: Int
        let lastUsed: String?
        let me: Bool
    }

    /// `GET /v1/admin/users`'s answer.
    struct AccountList: Decodable { let items: [Account] }

    func accounts() async throws -> [Account] {
        let a: AccountList = try await get("v1/admin/users")
        return a.items
    }

    func createAccount(name: String, password: String, admin: Bool) async throws {
        _ = try await send("POST", url("v1/admin/users"), body: ["name": name, "password": password, "admin": admin])
    }

    func updateAccount(_ id: String, password: String? = nil, admin: Bool? = nil) async throws {
        var body: [String: Any] = [:]
        if let password { body["password"] = password }
        if let admin { body["admin"] = admin }
        _ = try await send("PATCH", url("v1/admin/users/\(id)"), body: body)
    }

    func deleteAccount(_ id: String) async throws {
        _ = try await send("DELETE", url("v1/admin/users/\(id)"))
    }

    struct Device: Decodable, Identifiable, Hashable {
        let id: String
        let deviceName: String?
        let createdAt: String?
        let lastUsed: String?
        let current: Bool
    }

    /// `GET /v1/admin/users/{id}/devices`'s answer.
    struct DeviceList: Decodable { let items: [Device] }

    func devices(of user: String) async throws -> [Device] {
        let a: DeviceList = try await get("v1/admin/users/\(user)/devices")
        return a.items
    }

    func signOutDevice(user: String, device: String) async throws {
        _ = try await send("DELETE", url("v1/admin/users/\(user)/devices/\(device)"))
    }

    // MARK: Invitations

    struct Invite: Decodable, Identifiable, Hashable {
        let id: String
        let admin: Bool
        let expiresAt: String
        let code: String?
    }

    /// `GET /v1/admin/invites`'s answer.
    struct InviteList: Decodable { let items: [Invite] }

    func invites() async throws -> [Invite] {
        let a: InviteList = try await get("v1/admin/invites")
        return a.items
    }

    func createInvite(admin: Bool) async throws -> Invite {
        try await post("v1/admin/invites", ["admin": admin])
    }

    func deleteInvite(_ id: String) async throws {
        _ = try await send("DELETE", url("v1/admin/invites/\(id)"))
    }

    /// A new account with an invitation code (the sign-in screen): signed in on this device.
    func redeemInvite(code: String, username: String, password: String) async throws -> AuthResult {
        let body: [String: Any] = ["code": code, "username": username, "password": password].merging(device) { a, _ in a }
        let r: AuthResult = try await post("v1/auth/invite", body, authorized: false)
        token = r.token
        return r
    }

    // MARK: Services

    /// Signs in to OpenSubtitles with these (nothing saved): nil when it worked, else why not.
    func checkOpenSubtitles(username: String, password: String) async throws -> String? {
        do {
            _ = try await send("POST", url("v1/admin/services/opensubtitles/check"), body: ["username": username, "password": password], timeout: 40)
            return nil
        } catch let e as MoonriseClientError {
            if case let .http(code, body) = e, code == 422 || code == 409 { return body }
            throw e
        }
    }

    /// The settings document's values and fields (`GET /v1/admin/settings`), as JSON.
    func settingsDocument() async throws -> [String: Any] {
        let data = try await send("GET", url("v1/admin/settings"))
        return (try JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
    }

    // MARK: The server

    struct SystemInfo: Decodable {
        struct Sizes: Decodable { let database: Int64; let subtitles: Int64; let artwork: Int64; let data: Int64; let cache: Int64 }
        struct Hardware: Decodable {
            struct GPU: Decodable, Hashable { let vendor: String; let name: String?; let driver: String?; let memoryMb: Int? }
            struct Other: Decodable, Hashable { let vendor: String; let device: String; let usable: Bool }
            struct CPU: Decodable { let model: String?; let threads: Int }
            let nvidia: [GPU]
            let otherGpus: [Other]
            let cpu: CPU
            let ffmpeg: String?
        }
        let name: String
        let version: String
        let startedAt: String
        let uptimeSecs: Int
        let sizes: Sizes
        let cacheMaxBytes: Int64
        let downloadsBytes: Int64
        let downloadMaxBytes: Int64
        /// What transcodes video (servers from 0.16.0): the backend found at start and its chain.
        struct Transcode: Decodable {
            let backend: String
            let label: String
            let device: String?
            let decode: String
            let tonemap: String
            let encode: String
            let hdrKept: Bool
            let maxHeight: Int
            let requested: String
            let notes: [String]
            let available: Bool
            /// What `transcode.hardware` may be set to on this server (0.18.0): auto first.
            let choices: [Choice]?
            struct Choice: Decodable, Hashable { let key: String; let label: String }
        }
        let hardware: Hardware
        /// nvenc, qsv, vaapi, cpu (0.16.0), videotoolbox, v4l2 (0.18.0), off, unavailable
        let transcoding: String
        let transcode: Transcode?
        let canRestart: Bool
        let restartRequired: [String]
    }

    func systemInfo() async throws -> SystemInfo { try await get("v1/admin/system", timeout: 30) }

    struct Activity: Decodable {
        struct Session: Decodable, Identifiable, Hashable {
            let sid: String
            let user: String?
            let title: String?
            let seriesTitle: String?
            let episodeLabel: String?
            let method: String
            let variant: String?
            let tonemap: Bool?
            let duration: Double?
            let position: Double?
            let idleSecs: Int
            var id: String { sid }
        }
        struct Transcode: Decodable, Hashable { let track: String; let owner: String; let idleSecs: Int }
        struct Download: Decodable, Identifiable, Hashable {
            let id: String
            let label: String
            let method: String
            let state: String
            let progress: Double
            let error: String?
        }
        struct Subtitles: Decodable {
            struct Extraction: Decodable, Hashable { let item: String; let urgent: Bool; let reading: Bool }
            struct Backfill: Decodable { let done: Int?; let files: Int?; let failed: Int?; let current: String? }
            let extractions: [Extraction]
            let backfill: Backfill?
        }
        /// Keyframe indexes made ahead of need (server 0.23: `index_backfill.rs`).
        struct IndexBackfill: Decodable {
            /// off, starting, running, paused, idle
            let state: String?
            let files: Int?
            let done: Int?
            let failed: Int?
            let current: String?
        }
        /// Scrubbing thumbnails made ahead of need (server 0.27: `trickplay.rs`).
        struct Trickplay: Decodable {
            /// off, starting, running, paused, full (the space for them is used up), idle
            let state: String?
            let files: Int?
            let done: Int?
            let failed: Int?
            /// Files that get none (0.27.1: Dolby Vision profile 5).
            let skipped: Int?
            let bytes: Int64?
            let maxBytes: Int64?
        }
        struct Scan: Decodable {
            struct Last: Decodable { let at: String?; let ms: Int?; let error: String?; let aborted: String?; let filesSeen: Int? }
            let phase: String?
            let full: Bool?
            let startedAt: String?
            let lastPass: Last?
        }
        let sessions: [Session]
        let transcodes: [Transcode]
        let transcodeSlots: Int
        let subtitles: Subtitles
        let downloads: [Download]
        let scan: Scan?
        let indexBackfill: IndexBackfill?
        let trickplay: Trickplay?
        let errors: [String]
    }

    func activity() async throws -> Activity { try await get("v1/admin/activity", timeout: 20) }

    /// `GET /v1/admin/logs`'s answer.
    struct LogLines: Decodable { let lines: [String] }

    func serverLog(lines: Int, level: String? = nil) async throws -> [String] {
        let a: LogLines = try await get("v1/admin/logs", ["lines": String(lines), "level": level])
        return a.lines
    }

    func restartServer() async throws {
        _ = try await send("POST", url("v1/admin/restart"))
    }
}

// MARK: - AI matching's provider (server 0.22: llm_admin.rs)

extension MoonriseClient {
    struct LLMPreset: Decodable, Identifiable, Hashable {
        struct Saved: Decodable, Hashable { let apiKey: SecretState?; let model: String?; let baseUrl: String?; let output: String? }
        let id: String
        let name: String
        let style: String
        let baseUrl: String
        let model: String
        let output: String
        /// required, optional, sign_in
        let key: String
        let help: String
        let local: Bool
        /// Settings kept from when it was chosen before.
        let saved: Saved?
    }

    struct SecretState: Decodable, Hashable { let set: Bool; let last4: String? }

    struct LLMOverview: Decodable {
        struct Current: Decodable {
            let provider: String
            let baseUrl: String
            let model: String
            let output: String
            let outputInUse: String?
            let apiKey: SecretState
            /// Origins this provider has keys kept for, other than its current address.
            let keysAside: [String]?
            let enabled: Bool
        }
        /// One ChatGPT account registration kept on the server.
        struct ChatGPTAccount: Decodable, Hashable, Identifiable { let id: String; let label: String; let email: String; let signedIn: Bool; let active: Bool }
        struct ChatGPT: Decodable {
            let signedIn: Bool
            let email: String?
            let needsSignIn: Bool?
            /// The registration in use.
            let account: String?
            let accounts: [ChatGPTAccount]?
            /// Registrations OpenAI issued whose sign-in never completed (servers from 0.22.2).
            let unfinished: [ChatGPTUnfinished]?
        }
        /// A registration whose sign-in never completed: no sign-in uses it; it can be forgotten.
        struct ChatGPTUnfinished: Decodable, Hashable, Identifiable { let id: String; let keptAt: Int? }
        let presets: [LLMPreset]
        let current: Current
        let chatgpt: ChatGPT
    }

    struct LLMModel: Decodable, Hashable, Identifiable { let id: String; let name: String }

    struct LLMTestResult: Decodable {
        struct Verdict: Decodable { let kind: String?; let tmdbId: Int?; let confidence: Double?; let reason: String? }
        let ok: Bool
        let correct: Bool?
        let verdict: Verdict?
        let output: String?
        let ms: Int?
        let error: String?
        /// unreachable, auth, not_found, format, limit, other
        let problem: String?
    }

    /// nil from servers before 0.22.
    func llmOverview() async throws -> LLMOverview? {
        do {
            return try await get("v1/admin/llm", timeout: 15)
        } catch MoonriseClientError.http(404, _) {
            return nil
        }
    }

    /// `POST /v1/admin/llm/models`'s answer.
    struct LLMModelList: Decodable { let models: [LLMModel] }

    /// `body`: provider, base_url, api_key (what isn't given comes from the server's settings).
    func llmModels(_ body: [String: Any]) async throws -> [LLMModel] {
        let data = try await send("POST", url("v1/admin/llm/models"), body: body, timeout: 30)
        return try Self.decoder.decode(LLMModelList.self, from: data).models
    }

    /// One real identification with these settings (nothing saved).
    func llmTest(_ body: [String: Any]) async throws -> LLMTestResult {
        let data = try await send("POST", url("v1/admin/llm/test"), body: body, timeout: 300)
        return try Self.decoder.decode(LLMTestResult.self, from: data)
    }

    /// The app's side of Sign in with ChatGPT, for the server's log (`app_flow`): 2 tries again by
    /// itself after a refused code and reads the callback as `URLSearchParams` does. Raise it
    /// when that side changes.
    static let chatGPTFlow = 2

    /// `POST /v1/admin/llm/chatgpt/start`'s answer: the page to open.
    struct ChatGPTStartAnswer: Decodable { let url: String }

    /// Sign in with ChatGPT, step 1: the page to open; OpenAI sends the browser back to `redirect`.
    /// `account`: sign in again with that registration (default: the one in use); `newAccount`:
    /// register Moonrise for another ChatGPT account (a separate registration).
    /// `retry`: the `state` of an attempt whose code OpenAI refused (409 from `chatGPTFinish`): one
    /// more try with that attempt's registration.
    func chatGPTStart(redirect: String, newAccount: Bool = false, account: String? = nil, retry: String? = nil) async throws -> URL {
        var body: [String: Any] = ["redirect_uri": redirect, "app_flow": Self.chatGPTFlow]
        if newAccount { body["new_account"] = true }
        if let account { body["account"] = account }
        if let retry { body["retry"] = retry }
        let data = try await send("POST", url("v1/admin/llm/chatgpt/start"), body: body, timeout: 20)
        guard let u = URL(string: try Self.decoder.decode(ChatGPTStartAnswer.self, from: data).url) else { throw URLError(.badURL) }
        return u
    }

    struct ChatGPTSignedIn: Decodable { let email: String; let models: [LLMModel] }

    /// Step 3: what came back to the app's loopback listener; the server exchanges the code.
    /// `callbackParams`: the names (not values) of the callback's parameters, for the server's log.
    /// 409: OpenAI refused the code; start again with `retry` = `state`, once.
    func chatGPTFinish(state: String, code: String, clientId: String?, callbackParams: [String] = []) async throws -> ChatGPTSignedIn {
        var body: [String: Any] = ["state": state, "code": code, "app_flow": Self.chatGPTFlow]
        if let clientId { body["client_id"] = clientId }
        if !callbackParams.isEmpty { body["callback_params"] = callbackParams }
        let data = try await send("POST", url("v1/admin/llm/chatgpt/finish"), body: body, timeout: 60)
        return try Self.decoder.decode(ChatGPTSignedIn.self, from: data)
    }

    /// `revoked`: OpenAI confirmed the sign-in ended; otherwise `message` says what to do.
    /// OpenAI sent the browser back with an error instead of a code: tells the server (which logs
    /// it and ends the attempt). Errors are ignored: older servers don't know this.
    func chatGPTReportError(state: String, error: String, description: String?) async {
        var body: [String: Any] = ["state": state, "error": error, "app_flow": Self.chatGPTFlow]
        if let description { body["error_description"] = description }
        _ = try? await send("POST", url("v1/admin/llm/chatgpt/finish"), body: body, timeout: 20)
    }

    /// Removes a registration that isn't signed in (an unfinished one) from the server.
    func chatGPTForget(account: String) async throws {
        _ = try await send("POST", url("v1/admin/llm/chatgpt/forget"), body: ["account": account], timeout: 20)
    }

    struct ChatGPTSignedOut: Decodable { let revoked: Bool?; let message: String? }

    func chatGPTSignOut(account: String? = nil) async throws -> ChatGPTSignedOut {
        let body: [String: Any] = account.map { ["account": $0] } ?? [:]
        let data = try await send("POST", url("v1/admin/llm/chatgpt/sign-out"), body: body, timeout: 30)
        return (try? Self.decoder.decode(ChatGPTSignedOut.self, from: data)) ?? ChatGPTSignedOut(revoked: nil, message: nil)
    }
}
