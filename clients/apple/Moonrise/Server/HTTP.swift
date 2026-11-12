import Foundation

/// The app's one way onto the network: every request to a Moonrise server (the API, playlists,
/// artwork, thumbnails, download sets, discovery) and to the demo's stream hosts goes out through
/// here, so how requests are built, which session sends them, their timeouts and how a status is
/// read live in one place. The sessions with needs of their own are made here too, each with its
/// configuration unchanged.
///
/// Headers: the server's token comes from `MoonriseClient.headers` (or the headers a
/// `MediaServerClient` hands over), and only a request to that server is given it. Requests to
/// other hosts pass no `headers`, so a Moonrise token never leaves for a third party.
///
/// Not here: the background file-download session's tasks, which stay with their delegate
/// (`DownloadManager`), and AVPlayer's own loading of streams.
enum HTTP {
    // MARK: - Requests

    /// A request. Only what is given is set; the rest keeps `URLRequest`'s defaults (GET, a 60 s
    /// timeout, the protocol's cache policy), so the session's configuration applies as before.
    static func request(_ url: URL, method: String? = nil, headers: [String: String] = [:], timeout: TimeInterval? = nil,
                        cachePolicy: URLRequest.CachePolicy? = nil) -> URLRequest {
        var req = URLRequest(url: url)
        if let timeout { req.timeoutInterval = timeout }
        if let method { req.httpMethod = method }
        if let cachePolicy { req.cachePolicy = cachePolicy }
        headers.forEach { req.setValue($1, forHTTPHeaderField: $0) }
        return req
    }

    /// Sends `req`: the body and the HTTP status (0 when the answer isn't HTTP). Throws only when
    /// no answer came (unreachable, timed out, cancelled); a status is the caller's to judge.
    static func data(for req: URLRequest, on session: URLSession = .shared) async throws -> (Data, Int) {
        let (data, resp) = try await session.data(for: req)
        return (data, status(resp))
    }

    /// A plain GET of `url`, built by the session from its configuration.
    static func data(from url: URL, on session: URLSession = .shared) async throws -> (Data, Int) {
        let (data, resp) = try await session.data(from: url)
        return (data, status(resp))
    }

    /// Sends `req` and keeps the body in a temporary file: the file and the HTTP status.
    static func download(for req: URLRequest, on session: URLSession = .shared) async throws -> (URL, Int) {
        let (file, resp) = try await session.download(for: req)
        return (file, status(resp))
    }

    static func status(_ response: URLResponse?) -> Int {
        (response as? HTTPURLResponse)?.statusCode ?? 0
    }

    // MARK: - Sessions

    /// The server's API (`MoonriseClient`). Answers aren't cached (watch state changes under the
    /// same URLs; images have their own cache).
    static let api: URLSession = {
        let config = URLSessionConfiguration.default
        config.timeoutIntervalForRequest = 20
        config.urlCache = nil
        return URLSession(configuration: config)
    }()

    /// Artwork (`ImageCache`, which keeps its own copy on disk).
    static func artworkSession() -> URLSession {
        let config = URLSessionConfiguration.default
        config.urlCache = nil // ImageCache keeps its own copy
        config.timeoutIntervalForRequest = 30
        config.httpMaximumConnectionsPerHost = 6
        return URLSession(configuration: config)
    }

    /// A download's HLS set, fetched segment by segment (`SegmentedDownloads`).
    static func downloadSetSession(cellular: Bool) -> URLSession {
        let config = URLSessionConfiguration.default
        config.httpMaximumConnectionsPerHost = 6
        config.timeoutIntervalForRequest = 90
        config.allowsCellularAccess = cellular
        config.requestCachePolicy = .reloadIgnoringLocalCacheData
        config.urlCache = nil
        return URLSession(configuration: config)
    }

    /// The subnet sweep for servers (`MoonriseDiscovery`): quick, one connection per host.
    static let sweep: URLSession = {
        let c = URLSessionConfiguration.ephemeral
        c.timeoutIntervalForRequest = 1.5
        c.timeoutIntervalForResource = 2
        c.waitsForConnectivity = false
        c.httpMaximumConnectionsPerHost = 1
        return URLSession(configuration: c)
    }()

    /// Original files, downloaded by the system in the background (`DownloadManager`, the
    /// delegate; its events arrive on the main queue).
    static func backgroundFileSession(identifier: String, cellular: Bool, delegate: URLSessionDelegate) -> URLSession {
        let fileConfig = URLSessionConfiguration.background(withIdentifier: identifier)
        fileConfig.isDiscretionary = false
        fileConfig.sessionSendsLaunchEvents = true
        fileConfig.allowsCellularAccess = cellular
        return URLSession(configuration: fileConfig, delegate: delegate, delegateQueue: .main)
    }
}
