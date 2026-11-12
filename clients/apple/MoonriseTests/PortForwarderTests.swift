// The real `PortForwarder`, with only the node's C dial replaced (`FakeTunnel`): a player's connection
// never hangs on a dial, and nothing it opened outlives the connection.
import Darwin
import Foundation
import Testing
@testable import Moonrise

/// A plain TCP client, as a player would connect to the forwarder.
private final class Client {
    let fd: Int32

    init(port: UInt16) throws {
        fd = socket(AF_INET, SOCK_STREAM, 0)
        var yes: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &yes, socklen_t(MemoryLayout<Int32>.size))
        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_addr.s_addr = inet_addr("127.0.0.1")
        addr.sin_port = port.bigEndian
        let r = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
        }
        guard r == 0 else { throw POSIXError(.ECONNREFUSED) }
    }

    deinit { Darwin.close(fd) }

    func send(_ text: String) { _ = text.withCString { Darwin.write(fd, $0, strlen($0)) } }

    /// Everything until the connection ends, and how long that took. nil if it didn't end in time.
    func readToEnd(within seconds: Double) -> (data: Data, took: Double)? {
        let began = Date()
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 8192)
        while Date().timeIntervalSince(began) < seconds {
            var p = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
            guard poll(&p, 1, 50) > 0 else { continue }
            let n = Darwin.read(fd, &buffer, buffer.count)
            if n <= 0 { return (data, Date().timeIntervalSince(began)) }
            data.append(contentsOf: buffer[0..<n])
        }
        return nil
    }

    /// The first `count` bytes.
    func read(atLeast count: Int, within seconds: Double) -> Data {
        let began = Date()
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 8192)
        while data.count < count, Date().timeIntervalSince(began) < seconds {
            var p = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
            guard poll(&p, 1, 50) > 0 else { continue }
            let n = Darwin.read(fd, &buffer, buffer.count)
            if n <= 0 { break }
            data.append(contentsOf: buffer[0..<n])
        }
        return data
    }
}

private func waitUntil(within seconds: Double, _ condition: () -> Bool) -> Bool {
    let began = Date()
    while Date().timeIntervalSince(began) < seconds {
        if condition() { return true }
        usleep(20_000)
    }
    return condition()
}

@Suite struct PortForwarderTests {
    private func make(_ tunnel: FakeTunnel, _ limits: PortForwarder.Limits) throws -> PortForwarder {
        try PortForwarder(target: "100.64.0.1:8096", limits: limits, dial: { tunnel.dial($0) })
    }

    @Test func aDialThatBlocksEndsThePlayersConnectionInTime() throws {
        let tunnel = FakeTunnel()
        tunnel.set(.dead)
        var limits = PortForwarder.Limits()
        limits.dialTimeout = 1
        let forwarder = try make(tunnel, limits)
        defer { forwarder.close(); tunnel.set(.up) }
        let client = try Client(port: forwarder.port)
        client.send("GET / HTTP/1.1\r\n\r\n")
        // A failure the player can act on, and not silence.
        let ended = try #require(client.readToEnd(within: 4), "the connection was left hanging")
        #expect(ended.data.isEmpty)
        #expect(ended.took >= 0.8)
        #expect(ended.took < 3)
        #expect(forwarder.counts.dialsTimedOut == 1)
        // The call itself can't be stopped: its thread stays until the node answers, then goes.
        #expect(forwarder.counts.dialsInFlight == 1)
        tunnel.set(.up)
        #expect(waitUntil(within: 3) { forwarder.counts.isIdle })
        // And what it finally dialled, nobody wanted: closed.
        #expect(waitUntil(within: 3) { tunnel.farEndsClosed == 1 })
    }

    @Test func aFailedDialClosesAtOnce() throws {
        let tunnel = FakeTunnel()
        tunnel.set(.refuses)
        let forwarder = try make(tunnel, PortForwarder.Limits())
        defer { forwarder.close() }
        let client = try Client(port: forwarder.port)
        let ended = try #require(client.readToEnd(within: 3))
        #expect(ended.took < 1)
        #expect(forwarder.counts.dialsTimedOut == 0)
        #expect(waitUntil(within: 3) { forwarder.counts.isIdle })
    }

    @Test func blockedDialsAreLimited() throws {
        let tunnel = FakeTunnel()
        tunnel.set(.dead)
        var limits = PortForwarder.Limits()
        limits.dialTimeout = 60
        limits.maxDialsInFlight = 2
        let forwarder = try make(tunnel, limits)
        defer { forwarder.close(); tunnel.set(.up) }
        let clients = try (0..<4).map { _ in try Client(port: forwarder.port) }
        #expect(waitUntil(within: 3) { forwarder.counts.dialsRefused == 2 })
        // The two over the limit are told at once, and no thread was started for them.
        let over = clients.compactMap { $0.readToEnd(within: 1) }
        #expect(over.count == 2)
        #expect(tunnel.dials == 2)
        #expect(forwarder.counts.dialsInFlight == 2)
        tunnel.set(.up)
        #expect(waitUntil(within: 3) { forwarder.counts.dialsInFlight == 0 })
    }

    @Test func aLiveTunnelCarriesTheConnection() throws {
        let server = try FakeMoonrise()
        let route = try FakeRoute(index: 1, server: server)
        defer { route.stop() }
        let tunnel = FakeTunnel(upstream: route.port)
        let forwarder = try make(tunnel, PortForwarder.Limits())
        defer { forwarder.close() }
        var client: Client? = try Client(port: forwarder.port)
        client?.send("GET /v1/info HTTP/1.1\r\nHost: x\r\n\r\n")
        let head = client?.read(atLeast: 15, within: 5) ?? Data()
        #expect(String(decoding: head, as: UTF8.self).hasPrefix("HTTP/1.1 200"))
        #expect(forwarder.counts.bridges == 1)
        #expect(forwarder.counts.pumps == 2)
        client = nil
        #expect(waitUntil(within: 8) { forwarder.counts.isIdle })
        #expect(waitUntil(within: 3) { tunnel.farEndsClosed == 1 })
    }

    /// The tunnel dies under an open connection, then the player gives up on it. Nothing else ever
    /// ends that connection, so before this each one kept two threads and two descriptors.
    @Test func aDeadTunnelsConnectionIsReleasedWhenThePlayerLeaves() throws {
        let server = try FakeMoonrise()
        let route = try FakeRoute(index: 1, server: server)
        defer { route.stop() }
        let tunnel = FakeTunnel(upstream: route.port)
        var limits = PortForwarder.Limits()
        limits.drainIdle = 1
        let forwarder = try make(tunnel, limits)
        defer { forwarder.close(); tunnel.set(.up) }
        var client: Client? = try Client(port: forwarder.port)
        client?.send("GET /v1/info HTTP/1.1\r\nHost: x\r\n\r\n")
        #expect((client?.read(atLeast: 15, within: 5) ?? Data()).count >= 15)
        tunnel.set(.dead)
        client = nil
        let released = waitUntil(within: 5) { forwarder.counts.isIdle }
        #expect(released, "still open: \(forwarder.counts)")
        #expect(waitUntil(within: 3) { tunnel.farEndsClosed == 1 })
    }

    /// A connection's other direction goes on working as long as it moves bytes.
    @Test func aHalfClosedConnectionKeepsDrainingWhileBytesFlow() throws {
        var limits = PortForwarder.Limits()
        limits.drainIdle = 0.5
        let forwarder = try PortForwarder(target: "x:1", limits: limits, dial: { _ in
            var pair: [Int32] = [0, 0]
            guard socketpair(AF_UNIX, SOCK_STREAM, 0, &pair) == 0 else { return nil }
            let far = pair[1]
            Thread.detachNewThread {
                for _ in 0..<8 {
                    usleep(200_000)
                    _ = Darwin.write(far, "x", 1)
                }
                Darwin.close(far)
            }
            return pair[0]
        })
        defer { forwarder.close() }
        let client = try Client(port: forwarder.port)
        Darwin.shutdown(client.fd, SHUT_WR)
        let ended = try #require(client.readToEnd(within: 6))
        #expect(ended.data.count == 8)
    }

    @Test func acceptErrorsThatAreOnlyAMoment() {
        // Out of descriptors must not end the listener: it would go on accepting in the kernel
        // with nobody answering, for good.
        for code in [EINTR, ECONNABORTED, EMFILE, ENFILE, ENOBUFS, ENOMEM] {
            #expect(PortForwarder.acceptShouldRetry(errno: code))
        }
        for code in [EBADF, EINVAL, ENOTSOCK] {
            #expect(!PortForwarder.acceptShouldRetry(errno: code))
        }
    }

    @Test func firstValueGivesUpEvenOnWorkThatIgnoresCancellation() async {
        let began = Date()
        let value: Int? = await firstValue(within: 0.3) {
            // Not cancelled by the caller's cancel.
            await Task.detached { try? await Task.sleep(for: .seconds(3)); return 7 }.value
        }
        #expect(value == nil)
        #expect(Date().timeIntervalSince(began) < 1.5)
    }

    @Test func firstValueAnswersWhenItCan() async {
        let value = await firstValue(within: 5) { () async -> Int? in 42 }
        #expect(value == 42)
    }

    @Test func nudgeOnlyOnARealChange() {
        let wifi = TailnetNudge.Path(online: true, interfaces: ["en0"], gateways: ["192.168.1.1"])
        let cell = TailnetNudge.Path(online: true, interfaces: ["pdp_ip0"], gateways: [])
        let offline = TailnetNudge.Path(online: false, interfaces: [], gateways: [])
        #expect(!TailnetNudge.shouldNudge(from: nil, to: wifi))
        #expect(!TailnetNudge.shouldNudge(from: wifi, to: wifi))
        #expect(TailnetNudge.shouldNudge(from: wifi, to: cell))
        #expect(!TailnetNudge.shouldNudge(from: wifi, to: offline))
        #expect(TailnetNudge.shouldNudge(from: offline, to: cell))
    }

    @Test func nudgeRequestsAreTheNodesLocalAPI() throws {
        let requests = TailnetNudge.requests(host: "127.0.0.1", port: 54321, apiKey: "example-key")
        #expect(requests.map(\.url?.absoluteString) == ["http://127.0.0.1:54321/localapi/v0/debug?action=rebind",
                                                        "http://127.0.0.1:54321/localapi/v0/debug?action=restun"])
        let first = try #require(requests.first)
        #expect(first.httpMethod == "POST")
        #expect(first.value(forHTTPHeaderField: "Sec-Tailscale") == "localapi")
        #expect(first.value(forHTTPHeaderField: "Authorization") == "Basic " + Data("tsnet:example-key".utf8).base64EncodedString())
    }
}
