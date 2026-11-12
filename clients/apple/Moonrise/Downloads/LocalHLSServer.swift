import Foundation
import Network

/// A loopback HTTP server that plays segmented downloads: AVPlayer gets
/// `http://127.0.0.1:<port>/dl/<download id>/<mode>/master.m3u8` and everything under it comes
/// from the download's `hls/` directory (`SegmentedDownloads`). AVPlayer loads HLS segments only
/// over http(s), so a file URL or a resource loader can't do this.
///
/// - Mode `v` (the server can be reached, or the download is complete): the playlists as the
///   server made them (VOD, every segment). A segment that isn't on the device yet is fetched
///   from the server right then (the request waits for it), so seeking ahead works.
/// - Mode `l` (no server, download incomplete): the media playlists end where the downloaded
///   part stops (EVENT, no ENDLIST), and grow as more arrives (AVPlayer reloads them).
///
/// Every segment the player asks for moves the download there (it fetches from the play
/// position on).
final class LocalHLSServer: @unchecked Sendable {
    static let shared = LocalHLSServer()

    private let queue = DispatchQueue(label: "moonrise.localhls", attributes: .concurrent)
    private let lock = NSLock()
    private var listener: NWListener?
    private var port: UInt16?
    private var waiters: [(UInt16?) -> Void] = []

    /// How long a request for a file that isn't there waits for it.
    static let fileWait: TimeInterval = 60

    /// The playlist URL for a download, nil if the server can't start.
    func url(for id: UUID, growing: Bool) async -> URL? {
        guard let port = await start() else { return nil }
        return URL(string: "http://127.0.0.1:\(port)/dl/\(id.uuidString)/\(growing ? "l" : "v")/master.m3u8")
    }

    private func start() async -> UInt16? {
        await withCheckedContinuation { (c: CheckedContinuation<UInt16?, Never>) in
            lock.lock()
            if let port {
                lock.unlock()
                c.resume(returning: port)
                return
            }
            waiters.append { c.resume(returning: $0) }
            let needsListener = listener == nil
            lock.unlock()
            guard needsListener else { return }
            let params = NWParameters.tcp
            params.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: .any)
            params.allowLocalEndpointReuse = true
            guard let l = try? NWListener(using: params) else { finishStart(nil); return }
            lock.lock()
            listener = l
            lock.unlock()
            l.stateUpdateHandler = { [weak self] state in
                guard let self else { return }
                switch state {
                case .ready: self.finishStart(l.port?.rawValue)
                case .failed, .cancelled:
                    self.lock.lock()
                    self.listener = nil
                    self.port = nil
                    self.lock.unlock()
                    self.finishStart(nil)
                default: break
                }
            }
            l.newConnectionHandler = { [weak self] c in self?.accept(c) }
            l.start(queue: queue)
        }
    }

    private func finishStart(_ port: UInt16?) {
        lock.lock()
        self.port = port
        let w = waiters
        waiters = []
        lock.unlock()
        w.forEach { $0(port) }
    }

    // MARK: - Requests

    private func accept(_ c: NWConnection) {
        c.start(queue: queue)
        readHead(c, Data())
    }

    private func readHead(_ c: NWConnection, _ buffer: Data) {
        c.receive(minimumIncompleteLength: 1, maximumLength: 16_384) { [weak self] data, _, complete, error in
            guard let self else { c.cancel(); return }
            var buf = buffer
            if let data { buf.append(data) }
            if let end = buf.range(of: Data("\r\n\r\n".utf8)) {
                let head = String(decoding: buf[..<end.lowerBound], as: UTF8.self)
                Task { await self.handle(c, head) }
            } else if complete || error != nil || buf.count > 65_536 {
                c.cancel()
            } else {
                self.readHead(c, buf)
            }
        }
    }

    private func handle(_ c: NWConnection, _ head: String) async {
        let lines = head.components(separatedBy: "\r\n")
        let parts = lines.first?.split(separator: " ") ?? []
        guard parts.count >= 2 else { return respond(c, status: 400, body: Data()) }
        let method = String(parts[0])
        let rawPath = String(parts[1]).components(separatedBy: "?")[0]
        var range: String?
        for l in lines.dropFirst() where l.lowercased().hasPrefix("range:") {
            range = l.dropFirst(6).trimmingCharacters(in: .whitespaces)
        }
        // /dl/<id>/<mode>/<path…>
        let comps = rawPath.split(separator: "/", omittingEmptySubsequences: true).map(String.init)
        guard comps.count >= 4, comps[0] == "dl", let id = UUID(uuidString: comps[1]),
              let path = comps.dropFirst(3).joined(separator: "/").removingPercentEncoding,
              !path.contains("..") else {
            return respond(c, status: 404, body: Data())
        }
        let growing = comps[2] == "l"
        let file = await MainActor.run { DownloadManager.shared.segmented.hlsDirectory(id).appendingPathComponent(path) }

        if path.hasSuffix(".m3u8") {
            guard var text = try? String(contentsOf: file, encoding: .utf8) else { return respond(c, status: 404, body: Data()) }
            if growing, path != "master.m3u8", !path.hasPrefix("subs/") {
                let have = await MainActor.run { () -> Int in
                    guard let rec = DownloadManager.shared.record(id) else { return 0 }
                    return DownloadManager.shared.segmented.contiguous(rec)
                }
                text = Self.growing(text, segments: have)
            }
            return respond(c, status: 200, type: "application/vnd.apple.mpegurl", body: Data(text.utf8), method: method)
        }

        if let n = SegmentedDownloads.segment(path) {
            await MainActor.run { DownloadManager.shared.segmented.playerAsked(id, segment: n) }
        }
        var there = FileManager.default.fileExists(atPath: file.path)
        if !there {
            #if DEBUG
            let t0 = Date()
            #endif
            // AVPlayer gives up on a request after a while and asks again on a new connection:
            // stop waiting for this one then (the fetch itself goes on for the next).
            let gone = Flag()
            c.receive(minimumIncompleteLength: 1, maximumLength: 1) { _, _, complete, error in
                if complete || error != nil { gone.set() }
            }
            there = await DownloadManager.shared.segmented.fetchNow(id, path, wait: Self.fileWait, abandoned: { gone.isSet })
            if gone.isSet { c.cancel(); return }
            #if DEBUG
            fputs(String(format: "MoonriseLocalHLS: %@ %@ on demand (%.1f s)\n", path, there ? "fetched" : "not available", Date().timeIntervalSince(t0)), stderr)
            #endif
        }
        guard there, let data = try? Data(contentsOf: file, options: .mappedIfSafe) else {
            return respond(c, status: there ? 500 : 503, body: Data())
        }
        let type = path.hasSuffix(".vtt") ? "text/vtt" : "video/mp4"
        if let range, let r = Self.parseRange(range, size: data.count) {
            return respond(c, status: 206, type: type, body: data.subdata(in: r), method: method,
                           extra: "Content-Range: bytes \(r.lowerBound)-\(r.upperBound - 1)/\(data.count)\r\n")
        }
        respond(c, status: 200, type: type, body: data, method: method)
    }

    /// A media playlist cut to its first `segments` segments, as a growing (EVENT) playlist.
    static func growing(_ text: String, segments: Int) -> String {
        var out: [String] = []
        var seen = 0
        var complete = true
        var pendingInf: String?
        for line in text.split(separator: "\n", omittingEmptySubsequences: false).map(String.init) {
            if line.hasPrefix("#EXT-X-PLAYLIST-TYPE") { out.append("#EXT-X-PLAYLIST-TYPE:EVENT"); continue }
            if line == "#EXT-X-ENDLIST" { continue }
            if line.hasPrefix("#EXTINF") { pendingInf = line; continue }
            if !line.hasPrefix("#"), !line.isEmpty, pendingInf != nil {
                if seen < segments {
                    out.append(pendingInf!)
                    out.append(line)
                    seen += 1
                } else {
                    complete = false
                }
                pendingInf = nil
                continue
            }
            if !line.isEmpty { out.append(line) }
        }
        if complete { out.append("#EXT-X-ENDLIST") }
        return out.joined(separator: "\n") + "\n"
    }

    static func parseRange(_ h: String, size: Int) -> Range<Int>? {
        guard h.hasPrefix("bytes="), !h.contains(",") else { return nil }
        let spec = h.dropFirst(6).split(separator: "-", omittingEmptySubsequences: false)
        guard spec.count == 2 else { return nil }
        if spec[0].isEmpty, let suffix = Int(spec[1]), suffix > 0 { return max(0, size - suffix)..<size }
        guard let start = Int(spec[0]), start < size else { return nil }
        let end = Int(spec[1]).map { min($0, size - 1) } ?? size - 1
        return end >= start ? start..<(end + 1) : nil
    }

    private func respond(_ c: NWConnection, status: Int, type: String = "text/plain", body: Data, method: String = "GET", extra: String = "") {
        let reason = [200: "OK", 206: "Partial Content", 400: "Bad Request", 404: "Not Found", 500: "Internal Server Error", 503: "Service Unavailable"][status] ?? "Error"
        let head = "HTTP/1.1 \(status) \(reason)\r\nContent-Type: \(type)\r\nContent-Length: \(body.count)\r\nAccept-Ranges: bytes\r\nCache-Control: no-cache\r\n\(extra)Connection: close\r\n\r\n"
        var out = Data(head.utf8)
        if method != "HEAD" { out.append(body) }
        c.send(content: out, completion: .contentProcessed { _ in c.cancel() })
    }
}

/// A thread-safe one-way flag.
final class Flag: @unchecked Sendable {
    private let lock = NSLock()
    private var value = false
    func set() { lock.lock(); value = true; lock.unlock() }
    var isSet: Bool { lock.lock(); defer { lock.unlock() }; return value }
}
