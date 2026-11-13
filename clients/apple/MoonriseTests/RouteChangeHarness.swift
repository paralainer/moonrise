// A stand-in for the network around a playing stream: an HTTP "server" on the loopback that answers
// what a Moonrise server answers for one title and streams a real (small, generated) HLS set, reached
// through "routes" that can die the way real ones do. Real AVPlayer, real sockets, no other host.
//
// A dead route either refuses (nothing listens any more; live connections are reset) or hangs (it
// still accepts, and nothing it holds is ever answered again: what a Wi-Fi network that has gone
// away, or a tailnet path that hasn't come back, looks like to the app).
//
// The stream (`clients/apple/TestMedia/route-change-stream`) is 90 s of ffmpeg's `testsrc2` at 160x90 and
// two `sine` AAC tracks, as single-file byte-range HLS (`-hls_flags single_file -hls_time 2`).
import Darwin
import Foundation
@testable import Moonrise

/// What the server was asked, for the tests' assertions.
struct FakeSession: Sendable {
    let id: String
    let start: Double?
    let version: String?
    let quality: String?
    /// The route it was opened on.
    let route: Int
}

struct FakeReport: Sendable {
    let session: String
    let event: String
    let position: Double
}

/// The server behind the routes (shared by them: a session opened on one answers on the other).
final class FakeMoonrise: @unchecked Sendable {
    private let lock = NSLock()
    private var _sessions: [FakeSession] = []
    private var _reports: [FakeReport] = []
    private var _deleted: [String] = []
    private var firstMedia: [String: Date] = [:]
    /// Sessions up to this count no longer exist.
    private var forgotten = 0
    /// Media further ahead of the viewer than this is held back, so a stall comes soon (seconds).
    var bufferAhead: Double = 6
    /// The next session opened serves only this many seconds from where it starts, and never the
    /// rest: a buffer that stays that small however long the test takes to look.
    private var nextSessionLimit: Double?
    private var limits: [String: Double] = [:]

    private let itemJSON: Data
    private let playbackTemplate: [String: Any]
    private let media: [String: Data]
    private let playlists: [String: String]
    /// Each .ts file's segments: the byte offset each starts at and its time in the title.
    private let segments: [String: [(offset: Int, time: Double)]]

    static let itemId = "01exampleid000000000000002"

    init() throws {
        let bundle = Bundle(for: FakeMoonrise.self)
        func value(_ name: String) throws -> [String: Any] {
            let url = bundle.url(forResource: name, withExtension: "json", subdirectory: "examples")!
            let top = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as! [String: Any]
            return top["value"] as! [String: Any]
        }
        // The API's own example answers, with a second audio track.
        var item = try value("item-movie")
        var files = item["files"] as! [[String: Any]]
        var tracks = files[0]["tracks"] as! [[String: Any]]
        var german = tracks.first { ($0["kind"] as? String) == "Audio" }!
        german["index"] = 4
        german["language"] = "deu"
        german["display_title"] = "German - AAC - Mono"
        tracks.append(german)
        files[0]["tracks"] = tracks
        item["files"] = files
        itemJSON = try JSONSerialization.data(withJSONObject: item)

        var playback = try value("playback")
        var audio = playback["audio"] as! [[String: Any]]
        var second = audio[0]
        second["index"] = 4
        second["name"] = "German - AAC - Mono"
        second["language"] = "de"
        second["default"] = false
        audio.append(second)
        playback["audio"] = audio
        playback.removeValue(forKey: "tracks")
        playbackTemplate = playback

        var media: [String: Data] = [:]
        var playlists: [String: String] = [:]
        var segments: [String: [(offset: Int, time: Double)]] = [:]
        for name in ["v", "a_en", "a_de"] {
            let m3u8 = bundle.url(forResource: name, withExtension: "m3u8", subdirectory: "route-change-stream")!
            let ts = bundle.url(forResource: name, withExtension: "ts", subdirectory: "route-change-stream")!
            let text = try String(contentsOf: m3u8, encoding: .utf8)
            playlists["\(name).m3u8"] = text
            media["\(name).ts"] = try Data(contentsOf: ts)
            var list: [(offset: Int, time: Double)] = []
            var time = 0.0, length = 0.0
            for line in text.split(separator: "\n") {
                if line.hasPrefix("#EXTINF:") {
                    length = Double(line.dropFirst(8).dropLast()) ?? 0
                } else if line.hasPrefix("#EXT-X-BYTERANGE:"), let at = line.split(separator: "@").last, let offset = Int(at) {
                    list.append((offset, time))
                    time += length
                }
            }
            segments["\(name).ts"] = list
        }
        self.media = media
        self.playlists = playlists
        self.segments = segments
    }

    var sessions: [FakeSession] { lock.withLock { _sessions } }
    var reports: [FakeReport] { lock.withLock { _reports } }
    var deleted: [String] { lock.withLock { _deleted } }

    /// The next session opened serves only `seconds` of media from where it starts.
    func limitNextSession(to seconds: Double) { lock.withLock { nextSessionLimit = seconds } }

    /// A restart: every session's URLs answer 410 from now on.
    func forgetSessions() { lock.withLock { forgotten = _sessions.count } }

    struct Answer {
        var status = 200
        var type = "application/json"
        var body = Data()
        /// Not to be sent before this (the held-back media).
        var notBefore: Date?
        var extra: [String] = []
    }

    func answer(method: String, path: String, query: [String: String], headers: [String: String], body: Data, route: Int) -> Answer {
        let parts = path.split(separator: "/").map(String.init)
        if path == "/v1/info" {
            return json(["server_id": "route-test-server", "name": "route-test", "version": "0.30.0",
                         "capabilities": ["hls-copy", "audio-renditions", "session-delete"]])
        }
        if parts.count == 3, parts[1] == "items", method == "GET" { return Answer(body: itemJSON) }
        if parts.count == 4, parts[1] == "items", parts[3] == "playback" {
            let sid = "session\(sessions.count + 1)"
            let s = FakeSession(id: sid, start: query["start"].flatMap(Double.init), version: query["version"],
                                quality: query["quality"], route: route)
            lock.withLock {
                _sessions.append(s)
                if let limit = nextSessionLimit { limits[sid] = limit; nextSessionLimit = nil }
            }
            var p = playbackTemplate
            p["session"] = sid
            p["master"] = "/v1/s/\(sid)/master.m3u8"
            return json(p)
        }
        if parts.count == 4, parts[1] == "sessions", method == "POST" {
            let position = (try? JSONSerialization.jsonObject(with: body) as? [String: Any])?["position"] as? Double ?? -1
            lock.withLock { _reports.append(FakeReport(session: parts[2], event: parts[3], position: position)) }
            return json([:])
        }
        if parts.count == 3, parts[1] == "s", method == "DELETE" {
            lock.withLock { _deleted.append(parts[2]) }
            return json([:])
        }
        if parts.count == 4, parts[1] == "s" { return stream(session: parts[2], file: parts[3], headers: headers) }
        if path.hasSuffix("/logout") { return json([:]) }
        return Answer(status: 404, body: Data("{}".utf8))
    }

    private func json(_ object: [String: Any]) -> Answer {
        Answer(body: (try? JSONSerialization.data(withJSONObject: object)) ?? Data("{}".utf8))
    }

    private func stream(session sid: String, file: String, headers: [String: String]) -> Answer {
        let live = lock.withLock { Array(_sessions.dropFirst(forgotten)) }
        guard let s = live.first(where: { $0.id == sid }) else { return Answer(status: 410) }
        let start = s.start.map { String(format: "#EXT-X-START:TIME-OFFSET=%.3f,PRECISE=YES\n", $0) } ?? ""
        if file == "master.m3u8" {
            let text = "#EXTM3U\n#EXT-X-VERSION:6\n\(start)"
                + "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"a_en.m3u8\"\n"
                + "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"German\",LANGUAGE=\"de\",DEFAULT=NO,AUTOSELECT=YES,URI=\"a_de.m3u8\"\n"
                + "#EXT-X-STREAM-INF:BANDWIDTH=200000,CODECS=\"avc1.42c00c,mp4a.40.2\",RESOLUTION=160x90,AUDIO=\"aud\"\nv.m3u8\n"
            return Answer(type: "application/vnd.apple.mpegurl", body: Data(text.utf8))
        }
        if let text = playlists[file] {
            let withStart = text.replacingOccurrences(of: "#EXTM3U\n", with: "#EXTM3U\n\(start)")
            return Answer(type: "application/vnd.apple.mpegurl", body: Data(withStart.utf8))
        }
        guard let data = media[file] else { return Answer(status: 404) }
        var from = 0, to = data.count - 1
        var ranged = false
        if let r = headers["range"], r.hasPrefix("bytes=") {
            let ends = r.dropFirst(6).split(separator: "-", omittingEmptySubsequences: false)
            if ends.count == 2, let a = Int(ends[0]) {
                from = a
                to = Int(ends[1]) ?? to
                ranged = true
            }
        }
        // Held back until the viewer is near it, as a slow enough network would.
        let time = segments[file]?.last { $0.offset <= from }?.time ?? 0
        let first = lock.withLock {
            if firstMedia[sid] == nil { firstMedia[sid] = Date() }
            return firstMedia[sid]!
        }
        let wait = time - (s.start ?? 0) - bufferAhead
        var a = Answer(type: "video/mp2t", body: data.subdata(in: from..<min(to + 1, data.count)))
        if wait > 0 { a.notBefore = first.addingTimeInterval(wait) }
        // Past a session's limit: never.
        if let limit = lock.withLock({ limits[sid] }), time - (s.start ?? 0) >= limit { a.notBefore = .distantFuture }
        if ranged {
            a.status = 206
            a.extra = ["Content-Range: bytes \(from)-\(min(to, data.count - 1))/\(data.count)"]
        }
        return a
    }
}

/// One address of the server (home network or tailnet), which can die and come back.
final class FakeRoute: @unchecked Sendable {
    /// `mediaHung`: the API and playlists answer, segments never do (a tunnel that passes small
    /// answers and not a stream).
    /// `stale`: connections already open hang, new ones are served (a loopback forwarder whose
    /// tunnel died under it; AVPlayer keeps its connections to the same address open).
    enum Mode: Sendable { case up, refused, hung, stale, mediaHung }

    let index: Int
    let server: FakeMoonrise
    private(set) var port: UInt16 = 0
    private let lock = NSLock()
    private var _mode: Mode = .up
    private var listenFD: Int32 = -1
    private var connections: Set<Int32> = []
    private var frozen: Set<Int32> = []
    private var stopped = false

    var url: URL { URL(string: "http://127.0.0.1:\(port)")! }
    var mode: Mode { lock.withLock { _mode } }

    init(index: Int, server: FakeMoonrise) throws {
        self.index = index
        self.server = server
        try open(port: 0)
    }

    private func open(port wanted: UInt16) throws {
        let fd = socket(AF_INET, SOCK_STREAM, 0)
        guard fd >= 0 else { throw POSIXError(.EMFILE) }
        var yes: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &yes, socklen_t(MemoryLayout<Int32>.size))
        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_addr.s_addr = inet_addr("127.0.0.1")
        addr.sin_port = wanted.bigEndian
        let bound = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
        }
        guard bound == 0, listen(fd, 64) == 0 else { Darwin.close(fd); throw POSIXError(.EADDRINUSE) }
        var len = socklen_t(MemoryLayout<sockaddr_in>.size)
        _ = withUnsafeMutablePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { getsockname(fd, $0, &len) }
        }
        port = UInt16(bigEndian: addr.sin_port)
        lock.withLock { listenFD = fd }
        Thread.detachNewThread { [self] in acceptLoop(fd) }
    }

    /// Changes what the route does: live connections die or hang with it.
    func set(_ mode: Mode) {
        var closing: [Int32] = []
        var listener: Int32 = -1
        var reopen = false
        lock.withLock {
            guard mode != _mode else { return }
            reopen = _mode == .refused
            _mode = mode
            if mode == .refused {
                listener = listenFD
                listenFD = -1
                closing = Array(connections)
            }
            if mode == .stale { frozen = connections }
        }
        if reopen { try? open(port: port) }
        if listener >= 0 { Darwin.shutdown(listener, SHUT_RDWR); Darwin.close(listener) }
        for fd in closing {
            var off = linger(l_onoff: 1, l_linger: 0)
            setsockopt(fd, SOL_SOCKET, SO_LINGER, &off, socklen_t(MemoryLayout<linger>.size)) // closes with a reset
            Darwin.shutdown(fd, SHUT_RDWR)
        }
    }

    func stop() {
        var fds: [Int32] = []
        lock.withLock {
            stopped = true
            if listenFD >= 0 { fds.append(listenFD); listenFD = -1 }
            fds += connections
            connections.removeAll()
        }
        for fd in fds { Darwin.shutdown(fd, SHUT_RDWR) }
    }

    private func acceptLoop(_ fd: Int32) {
        while true {
            let client = accept(fd, nil, nil)
            guard client >= 0 else {
                if errno == EINTR { continue }
                return
            }
            var yes: Int32 = 1
            setsockopt(client, SOL_SOCKET, SO_NOSIGPIPE, &yes, socklen_t(MemoryLayout<Int32>.size))
            lock.withLock { _ = connections.insert(client) }
            Thread.detachNewThread { [self] in
                serve(client)
                lock.withLock { connections.remove(client); frozen.remove(client) }
                Darwin.close(client)
            }
        }
    }

    /// Waits out a hang (a hung route never answers; it ends only when the test stops or the
    /// connection goes). False: stop serving.
    private func stillServing(_ fd: Int32) -> Bool {
        while true {
            let (mode, gone, stale) = lock.withLock { (_mode, stopped, frozen.contains(fd)) }
            if gone || mode == .refused { return false }
            if mode == .up || mode == .mediaHung || (mode == .stale && !stale) { return true }
            usleep(50_000)
            var byte: UInt8 = 0
            let peeked = recv(fd, &byte, 1, MSG_PEEK | MSG_DONTWAIT)
            if peeked == 0 { return false } // the client gave up
        }
    }

    private func serve(_ fd: Int32) {
        var buffer = Data()
        var chunk = [UInt8](repeating: 0, count: 8192)
        while true {
            // One request: headers, then the body.
            var headerEnd: Range<Data.Index>?
            while headerEnd == nil {
                headerEnd = buffer.range(of: Data("\r\n\r\n".utf8))
                if headerEnd != nil { break }
                let n = read(fd, &chunk, chunk.count)
                if n < 0, errno == EINTR { continue }
                if n <= 0 { return }
                buffer.append(contentsOf: chunk[0..<n])
            }
            guard let end = headerEnd else { return }
            let head = String(decoding: buffer[buffer.startIndex..<end.lowerBound], as: UTF8.self)
            var lines = head.components(separatedBy: "\r\n")
            let first = lines.removeFirst().split(separator: " ").map(String.init)
            guard first.count >= 2 else { return }
            var headers: [String: String] = [:]
            for line in lines {
                guard let c = line.firstIndex(of: ":") else { continue }
                headers[line[..<c].lowercased()] = line[line.index(after: c)...].trimmingCharacters(in: .whitespaces)
            }
            let length = headers["content-length"].flatMap(Int.init) ?? 0
            var rest = Data(buffer[end.upperBound...])
            while rest.count < length {
                let n = read(fd, &chunk, chunk.count)
                if n < 0, errno == EINTR { continue }
                if n <= 0 { return }
                rest.append(contentsOf: chunk[0..<n])
            }
            let body = rest.prefix(length)
            buffer = Data(rest.dropFirst(length))
            // A hung route accepts and reads, and then says nothing.
            guard stillServing(fd) else { return }
            let comps = URLComponents(string: first[1])
            let query = Dictionary((comps?.queryItems ?? []).map { ($0.name, $0.value ?? "") }, uniquingKeysWith: { $1 })
            // Segments of a route passing only small answers: nothing ever comes.
            while self.mode == .mediaHung, (comps?.path ?? "").hasSuffix(".ts"), !lock.withLock({ stopped }) {
                usleep(100_000)
                if recvGone(fd) { return }
            }
            let a = server.answer(method: first[0], path: comps?.path ?? first[1], query: query, headers: headers, body: Data(body), route: index)
            if let t = a.notBefore {
                while Date() < t {
                    guard stillServing(fd) else { return }
                    usleep(50_000)
                    if recvGone(fd) { return }
                }
            }
            guard stillServing(fd) else { return }
            var text = "HTTP/1.1 \(a.status) X\r\nContent-Type: \(a.type)\r\nContent-Length: \(a.body.count)\r\nConnection: keep-alive\r\nAccept-Ranges: bytes\r\n"
            for e in a.extra { text += e + "\r\n" }
            text += "\r\n"
            guard write(fd, Data(text.utf8)) else { return }
            // In pieces, so a route dying mid-answer leaves the rest unsent.
            var offset = 0
            while offset < a.body.count {
                guard stillServing(fd) else { return }
                let n = min(16 * 1024, a.body.count - offset)
                guard write(fd, a.body.subdata(in: offset..<offset + n)) else { return }
                offset += n
            }
        }
    }

    /// The client closed its side while its answer was being held back.
    private func recvGone(_ fd: Int32) -> Bool {
        var byte: UInt8 = 0
        return recv(fd, &byte, 1, MSG_PEEK | MSG_DONTWAIT) == 0
    }

    private func write(_ fd: Int32, _ data: Data) -> Bool {
        data.withUnsafeBytes { raw in
            var off = 0
            while off < raw.count {
                let n = Darwin.write(fd, raw.baseAddress! + off, raw.count - off)
                if n < 0, errno == EINTR { continue }
                if n <= 0 { return false }
                off += n
            }
            return true
        }
    }
}

/// Stands in for the embedded Tailscale node under the real `PortForwarder`: a dial the way
/// `tailscale_dial` does it (blocking; a socketpair whose far end is carried to the server), which
/// can stop answering the way a dead tunnel does: dials block, live connections go silent.
final class FakeTunnel: @unchecked Sendable {
    enum Mode: Sendable { case up, dead, refuses }

    private let upstream: UInt16?
    private let condition = NSCondition()
    private var _mode: Mode = .up
    private var _dials = 0
    private var _farEndsClosed = 0
    /// Connections from before the tunnel died: they stay silent for good.
    private var deadConnections = 0
    private var opened = 0
    private var allClosed = false

    /// - upstream: the loopback port the tunnel leads to (a `FakeRoute`), or nil for no server.
    init(upstream: UInt16? = nil) { self.upstream = upstream }

    var dials: Int { condition.withLock { _dials } }
    /// How many of the tunnel's connections the forwarder has closed (the descriptor truly went).
    var farEndsClosed: Int { condition.withLock { _farEndsClosed } }

    func set(_ mode: Mode) {
        condition.lock()
        if mode == .dead { deadConnections = opened }
        _mode = mode
        condition.broadcast()
        condition.unlock()
    }

    /// The node goes away: every connection it carried closes.
    func closeAll() { condition.withLock { allClosed = true } }

    func dial(_ target: String) -> Int32? {
        condition.lock()
        _dials += 1
        while _mode == .dead { condition.wait() }
        let mode = _mode
        condition.unlock()
        guard mode == .up else { return nil }
        var pair: [Int32] = [0, 0]
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &pair) == 0 else { return nil }
        guard let upstream else { startRelay(far: pair[1], server: nil); return pair[0] }
        let server = socket(AF_INET, SOCK_STREAM, 0)
        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_addr.s_addr = inet_addr("127.0.0.1")
        addr.sin_port = upstream.bigEndian
        let connected = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.connect(server, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
        }
        guard connected == 0 else {
            Darwin.close(server); Darwin.close(pair[0]); Darwin.close(pair[1])
            return nil
        }
        startRelay(far: pair[1], server: server)
        return pair[0]
    }

    private func startRelay(far: Int32, server: Int32?) {
        var yes: Int32 = 1
        setsockopt(far, SOL_SOCKET, SO_NOSIGPIPE, &yes, socklen_t(MemoryLayout<Int32>.size))
        if let server { setsockopt(server, SOL_SOCKET, SO_NOSIGPIPE, &yes, socklen_t(MemoryLayout<Int32>.size)) }
        let number = condition.withLock { opened += 1; return opened }
        Thread.detachNewThread { [self] in relay(far: far, server: server, number: number) }
    }

    private func awaitFullClose(_ far: Int32) {
        while !condition.withLock({ allClosed }) {
            // An empty send sends nothing, and fails only once the other end is gone for good.
            if send(far, [UInt8](), 0, MSG_DONTWAIT) < 0, errno == EPIPE { return }
            usleep(50_000)
        }
    }

    private func isDead(_ number: Int) -> Bool { condition.withLock { number <= deadConnections } }

    /// Carries bytes between the forwarder's socket and the server until the forwarder hangs up.
    /// A dead connection keeps swallowing what the forwarder sends and never answers, as a tunnel
    /// with no path does; only the forwarder closing its end finishes it.
    private func relay(far: Int32, server: Int32?, number: Int) {
        var buffer = [UInt8](repeating: 0, count: 32 * 1024)
        defer {
            condition.withLock { _farEndsClosed += 1 }
            Darwin.close(far)
            if let server { Darwin.close(server) }
        }
        var fds = [pollfd(fd: far, events: Int16(POLLIN), revents: 0)]
        if let server { fds.append(pollfd(fd: server, events: Int16(POLLIN), revents: 0)) }
        while true {
            guard poll(&fds, nfds_t(fds.count), 100) >= 0 || errno == EINTR else { return }
            if condition.withLock({ allClosed }) { return }
            let dead = isDead(number)
            if fds[0].revents != 0 {
                let n = read(far, &buffer, buffer.count)
                if n <= 0 {
                    guard dead else { return }
                    // The forwarder only half-closed: the node's side stays open while the path
                    // is dead (its copy from the tunnel is still blocked), until the forwarder
                    // really closes.
                    awaitFullClose(far)
                    return
                }
                if !dead, let server, Darwin.write(server, buffer, n) <= 0 { return }
            }
            if !dead, fds.count > 1, fds[1].revents != 0 {
                let n = read(fds[1].fd, &buffer, buffer.count)
                if n <= 0 { Darwin.shutdown(far, SHUT_WR); fds.removeLast(); continue }
                var off = 0
                while off < n {
                    let w = buffer[off..<n].withUnsafeBytes { Darwin.write(far, $0.baseAddress!, $0.count) }
                    if w < 0, errno == EINTR { continue }
                    if w <= 0 { return }
                    off += w
                }
            }
            if dead { usleep(20_000) }
        }
    }
}

/// How late this test process's threads get to run: a thread that asks to wake every 10 ms adds
/// up how much later than that it wakes. On an idle machine that is close to nothing; with every
/// core busy (other builds, other simulators) it is the time the app's own code, the player's and
/// the test's were kept waiting for a CPU.
final class LoadMeter: @unchecked Sendable {
    static let shared = LoadMeter()
    private let lock = NSLock()
    private var _late: TimeInterval = 0

    private init() {
        let thread = Thread { [self] in
            let tick: TimeInterval = 0.01
            var last = ProcessInfo.processInfo.systemUptime
            while true {
                usleep(useconds_t(tick * 1_000_000))
                let now = ProcessInfo.processInfo.systemUptime
                let late = now - last - tick
                // Below that is the timer's own slack (wakeups are coalesced), not a wait for a CPU.
                if late > 0.005 { lock.withLock { _late += late } }
                last = now
            }
        }
        thread.qualityOfService = .userInitiated
        thread.start()
    }

    /// Seconds of waiting for a CPU since the meter started.
    var late: TimeInterval { lock.withLock { _late } }
}

/// A stopwatch for the route-change tests' waits and limits: wall time less the time the process
/// waited for a CPU meanwhile (`LoadMeter`). The app's own waits (a probe's timeout, the stall
/// checks, the give-up limit) are real time and pass as they do on a phone; what a loaded build
/// machine adds on top is the time nothing of the app could run, and that is not counted against
/// a limit. On an idle machine it reads as wall time.
struct TestWatch {
    private let wall = ProcessInfo.processInfo.systemUptime
    private let late = LoadMeter.shared.late

    /// Seconds since the watch started, not counting the time the process waited for a CPU.
    var elapsed: TimeInterval {
        let wallNow = ProcessInfo.processInfo.systemUptime
        return max(0, (wallNow - wall) - (LoadMeter.shared.late - late))
    }

    /// The part of the wall time since the start that was spent waiting for a CPU.
    var lost: TimeInterval { LoadMeter.shared.late - late }
}
