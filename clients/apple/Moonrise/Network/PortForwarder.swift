import Darwin
import Foundation

/// Listens on 127.0.0.1:<ephemeral> and pipes every accepted connection to `target` on the tailnet
/// through the embedded Tailscale node. This lets AVPlayer, VLC and URLSession reach tailnet hosts
/// with plain http://127.0.0.1:port URLs — none of them know Tailscale is involved.
///
/// The node's dial is a C call that can't be cancelled and has no timeout of its own: with a dead
/// tunnel it blocks for minutes, or for as long as the node isn't running. So a player's connection
/// never waits on it for long: at `Limits.dialTimeout` the connection is closed and the player sees
/// a failure instead of silence. The thread in the call can't be stopped; it stays until
/// the call returns, closes whatever the late dial produced, and there are never more than
/// `Limits.maxDialsInFlight` of them (past that a new connection is closed at once).
final class PortForwarder: @unchecked Sendable {
    /// Opens the tunnel to `target`: a connected descriptor the forwarder then owns, or nil. May
    /// block for as long as it likes.
    typealias Dial = @Sendable (_ target: String) -> Int32?

    struct Limits: Sendable {
        /// How long a connection waits for the tunnel: long enough for a WireGuard handshake to
        /// be retried once (they retry every 5 s), far shorter than the player's own waits.
        var dialTimeout: TimeInterval = 8
        /// Dials that may be blocked at once. Generous enough that a connection isn't refused when
        /// the tunnel has just come back and older dials are still waiting on their retransmits.
        var maxDialsInFlight = 32
        /// Once one side of a connection has ended, how long the other may stay silent before the
        /// connection is torn down. A dead tunnel never ends by itself, so it's released this way.
        var drainIdle: TimeInterval = 5
    }

    /// What is alive now.
    struct Counts: Equatable, Sendable {
        var dialsInFlight = 0
        var bridges = 0
        var pumps = 0
        /// Dials the player stopped waiting for, since the forwarder started.
        var dialsTimedOut = 0
        /// Connections closed at once, because too many dials were blocked.
        var dialsRefused = 0
        var isIdle: Bool { dialsInFlight == 0 && bridges == 0 && pumps == 0 }
    }

    let target: String
    private(set) var port: UInt16 = 0
    private var listenFD: Int32 = -1
    private let state = State()

    /// - preferredPort: reuse the same port across launches so URLs handed to background
    ///   downloads stay valid; falls back to an ephemeral port if it's taken.
    init(target: String, preferredPort: UInt16 = 0, limits: Limits = Limits(), dial: @escaping Dial) throws {
        self.target = target

        listenFD = socket(AF_INET, SOCK_STREAM, 0)
        guard listenFD >= 0 else { throw POSIXError(.EMFILE) }
        var yes: Int32 = 1
        setsockopt(listenFD, SOL_SOCKET, SO_REUSEADDR, &yes, socklen_t(MemoryLayout<Int32>.size))

        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_addr.s_addr = inet_addr("127.0.0.1")
        func tryBind(_ p: UInt16) -> Int32 {
            addr.sin_port = p.bigEndian
            return withUnsafePointer(to: &addr) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(listenFD, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
            }
        }
        var bound = preferredPort != 0 ? tryBind(preferredPort) : -1
        if bound != 0 { bound = tryBind(0) }
        guard bound == 0, listen(listenFD, 64) == 0 else {
            Darwin.close(listenFD)
            throw POSIXError(.EADDRINUSE)
        }

        var len = socklen_t(MemoryLayout<sockaddr_in>.size)
        _ = withUnsafeMutablePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { getsockname(listenFD, $0, &len) }
        }
        port = UInt16(bigEndian: addr.sin_port)

        let fd = listenFD, target = target, state = state
        Thread.detachNewThread { Self.acceptLoop(fd, target: target, limits: limits, dial: dial, state: state) }
    }

    deinit { close() }

    var counts: Counts { state.counts }

    func close() {
        guard state.markClosed() else { return }
        Darwin.shutdown(listenFD, SHUT_RDWR)
        Darwin.close(listenFD)
    }

    /// Worth accepting again after this `accept` error: it says the moment is bad (out of
    /// descriptors, an aborted connection), not that the listener is gone. A listener nobody
    /// accepts on still completes connections, and every one of them would hang unanswered.
    static func acceptShouldRetry(errno code: Int32) -> Bool {
        [EINTR, ECONNABORTED, EMFILE, ENFILE, ENOBUFS, ENOMEM].contains(code)
    }

    private static func acceptLoop(_ fd: Int32, target: String, limits: Limits, dial: @escaping Dial, state: State) {
        while true {
            let client = accept(fd, nil, nil)
            if client < 0 {
                let code = errno
                if state.isClosed || !acceptShouldRetry(errno: code) { return }
                if code != EINTR { usleep(100_000) } // until a descriptor is free again
                continue
            }
            noSigPipe(client)
            guard state.beginDial(limit: limits.maxDialsInFlight) else {
                Darwin.close(client)
                continue
            }
            Thread.detachNewThread { connection(client, target: target, limits: limits, dial: dial, state: state) }
        }
    }

    /// One accepted connection: this thread makes the dial, and a timer stops the player waiting.
    private static func connection(_ client: Int32, target: String, limits: Limits, dial: Dial, state: State) {
        let race = Race()
        DispatchQueue.global().asyncAfter(deadline: .now() + limits.dialTimeout) {
            guard race.claim() else { return }
            state.dialTimedOut()
            Darwin.close(client)
        }
        let remote = dial(target).flatMap { $0 >= 0 ? $0 : nil }
        state.endDial()
        guard race.claim() else {
            if let remote { Darwin.close(remote) } // too late: nobody is waiting for it any more
            return
        }
        guard let remote else {
            Darwin.close(client)
            return
        }
        noSigPipe(remote)
        bridge(client, remote, drainIdle: limits.drainIdle, state: state)
    }

    /// Copy bytes both ways until either side closes, then close both.
    private static func bridge(_ a: Int32, _ b: Int32, drainIdle: TimeInterval, state: State) {
        state.adjust { $0.bridges += 1 }
        let watch = BridgeWatch()
        for (from, to) in [(a, b), (b, a)] {
            state.adjust { $0.pumps += 1 }
            Thread.detachNewThread {
                pump(from: from, to: to, watch: watch)
                watch.pumpEnded()
                state.adjust { $0.pumps -= 1 }
            }
        }
        watch.waitForPumps(drainIdle: drainIdle) {
            // A pump in read() or write() on a dead tunnel only returns once its socket is shut.
            Darwin.shutdown(a, SHUT_RDWR)
            Darwin.shutdown(b, SHUT_RDWR)
        }
        Darwin.close(a)
        Darwin.close(b)
        state.adjust { $0.bridges -= 1 }
    }

    private static func pump(from: Int32, to: Int32, watch: BridgeWatch) {
        let size = 64 * 1024
        let buf = UnsafeMutableRawPointer.allocate(byteCount: size, alignment: 8)
        defer { buf.deallocate() }
        outer: while true {
            let n = read(from, buf, size)
            if n < 0, errno == EINTR { continue }
            if n <= 0 { break }
            watch.activity()
            var off = 0
            while off < n {
                let w = write(to, buf + off, n - off)
                if w < 0, errno == EINTR { continue }
                if w <= 0 { break outer }
                off += w
                watch.activity()
            }
        }
        // Half-close so the other direction can finish draining.
        Darwin.shutdown(to, SHUT_WR)
    }

    /// Who is first: the dial coming back, or the timer giving up on it.
    private final class Race: @unchecked Sendable {
        private let lock = NSLock()
        private var claimed = false
        /// True only for the first caller.
        func claim() -> Bool { lock.withLock { defer { claimed = true }; return !claimed } }
    }

    /// What a connection's two pumps are doing, for the thread that waits for them.
    private final class BridgeWatch: @unchecked Sendable {
        private let condition = NSCondition()
        private var running = 2
        private var lastActivity = DispatchTime.now().uptimeNanoseconds

        func activity() {
            condition.lock()
            lastActivity = DispatchTime.now().uptimeNanoseconds
            condition.unlock()
        }

        func pumpEnded() {
            condition.lock()
            running -= 1
            lastActivity = DispatchTime.now().uptimeNanoseconds
            condition.broadcast()
            condition.unlock()
        }

        /// Returns when both pumps have ended. Once the first has, the other gets `drainIdle` of
        /// silence (each byte it moves starts that again), then `tearDown` makes it end.
        func waitForPumps(drainIdle: TimeInterval, tearDown: () -> Void) {
            condition.lock()
            defer { condition.unlock() }
            while running == 2 { condition.wait() }
            var tornDown = false
            while running > 0 {
                let idle = Double(DispatchTime.now().uptimeNanoseconds - lastActivity) / 1_000_000_000
                if !tornDown, idle >= drainIdle {
                    tornDown = true
                    tearDown()
                    continue
                }
                condition.wait(until: Date().addingTimeInterval(tornDown ? 1 : min(1, max(0.05, drainIdle - idle))))
            }
        }
    }

    private final class State: @unchecked Sendable {
        private let lock = NSLock()
        private var now = Counts()
        private var closed = false

        var counts: Counts { lock.withLock { now } }
        var isClosed: Bool { lock.withLock { closed } }

        /// True the first time.
        func markClosed() -> Bool { lock.withLock { defer { closed = true }; return !closed } }

        func beginDial(limit: Int) -> Bool {
            lock.withLock {
                guard now.dialsInFlight < limit else { now.dialsRefused += 1; return false }
                now.dialsInFlight += 1
                return true
            }
        }

        func endDial() { adjust { $0.dialsInFlight -= 1 } }
        func dialTimedOut() { adjust { $0.dialsTimedOut += 1 } }
        func adjust(_ change: (inout Counts) -> Void) { lock.withLock { change(&now) } }
    }
}

private func noSigPipe(_ fd: Int32) {
    var yes: Int32 = 1
    setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &yes, socklen_t(MemoryLayout<Int32>.size))
}
