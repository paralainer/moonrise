import Darwin
import Foundation
import Network

/// Finds Moonrise servers on the local network: by Bonjour (the server announces `_moonrise._tcp`), and,
/// for networks where multicast doesn't get through, by asking every address of the local /24
/// networks for `GET /v1/info` on port 8097 (unicast needs only Local Network permission).
/// Every server found is confirmed by its `/v1/info`.
enum MoonriseDiscovery {
    struct Server: Identifiable, Hashable {
        /// The server id (`/v1/info`).
        let id: String
        let name: String
        let version: String
        /// http://ip:port
        let address: String
        /// Found by Bonjour (rather than the subnet sweep).
        let bonjour: Bool
        /// A new server, not set up yet (`/v1/info` setup: unclaimed).
        var unclaimed = false
    }

    static let serviceType = "_moonrise._tcp"

    static func discover(timeout: TimeInterval = 3) async -> [Server] {
        async let viaBonjour = bonjour(timeout: timeout)
        async let viaSweep = sweep(timeout: min(timeout, 2))
        var found: [String: Server] = [:]
        for s in await viaSweep { found[s.id] = s }
        // Bonjour's answer wins (it is the address the server announces).
        for s in await viaBonjour { found[s.id] = s }
        return found.values.sorted { ($0.name, $0.address) < ($1.name, $1.address) }
    }

    // MARK: Bonjour

    /// Browse for `_moonrise._tcp` and resolve each service to an IPv4 address and port.
    static func bonjour(timeout: TimeInterval) async -> [Server] {
        let endpoints = await browse(timeout: timeout)
        return await withTaskGroup(of: Server?.self) { g in
            for e in endpoints {
                g.addTask {
                    guard let base = await resolve(e, timeout: 2) else { return nil }
                    return await confirm(base, bonjour: true)
                }
            }
            var out: [Server] = []
            for await s in g { if let s { out.append(s) } }
            return out
        }
    }

    private static func browse(timeout: TimeInterval) async -> [NWEndpoint] {
        await withCheckedContinuation { (cont: CheckedContinuation<[NWEndpoint], Never>) in
            let browser = NWBrowser(for: .bonjour(type: serviceType, domain: nil), using: .tcp)
            let queue = DispatchQueue(label: "moonrise.discovery.browse")
            var results: [NWEndpoint] = []
            var done = false
            func finish() {
                guard !done else { return }
                done = true
                browser.cancel()
                cont.resume(returning: results)
            }
            browser.browseResultsChangedHandler = { r, _ in results = r.map(\.endpoint) }
            browser.stateUpdateHandler = { state in
                #if DEBUG
                fputs("MoonriseDiscovery: browser \(state)\n", stderr)
                #endif
                if case .failed = state { queue.async { finish() } }
            }
            browser.start(queue: queue)
            queue.asyncAfter(deadline: .now() + timeout) { finish() }
        }
    }

    /// The service's address, by connecting to it (IPv4 only: the URL must work from any app).
    private static func resolve(_ endpoint: NWEndpoint, timeout: TimeInterval) async -> URL? {
        await withCheckedContinuation { (cont: CheckedContinuation<URL?, Never>) in
            let params = NWParameters.tcp
            if let ip = params.defaultProtocolStack.internetProtocol as? NWProtocolIP.Options { ip.version = .v4 }
            let conn = NWConnection(to: endpoint, using: params)
            let queue = DispatchQueue(label: "moonrise.discovery.resolve")
            var done = false
            func finish(_ url: URL?) {
                guard !done else { return }
                done = true
                conn.cancel()
                cont.resume(returning: url)
            }
            conn.stateUpdateHandler = { state in
                #if DEBUG
                fputs("MoonriseDiscovery: resolve \(endpoint) \(state) \(String(describing: conn.currentPath?.remoteEndpoint))\n", stderr)
                #endif
                switch state {
                case .ready:
                    if case let .hostPort(host, port)? = conn.currentPath?.remoteEndpoint, case let .ipv4(v4) = host {
                        // "192.168.1.10" (the interface suffix some descriptions carry is dropped).
                        let ip = "\(v4)".split(separator: "%").first.map(String.init) ?? "\(v4)"
                        finish(URL(string: "http://\(ip):\(port.rawValue)"))
                    } else {
                        finish(nil)
                    }
                case .failed, .cancelled: finish(nil)
                default: break
                }
            }
            conn.start(queue: queue)
            queue.asyncAfter(deadline: .now() + timeout) { finish(nil) }
        }
    }

    // MARK: Subnet sweep

    static func sweep(timeout: TimeInterval) async -> [Server] {
        let hosts = LocalSubnet.hosts().map { addr -> String in
            var a = in_addr(s_addr: addr)
            var buf = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
            inet_ntop(AF_INET, &a, &buf, socklen_t(INET_ADDRSTRLEN))
            return String(cString: buf)
        }
        return await withTaskGroup(of: Server?.self) { g in
            for h in hosts {
                guard let base = URL(string: "http://\(h):\(MoonriseServer.port)") else { continue }
                g.addTask { await confirm(base, bonjour: false, timeout: timeout) }
            }
            var out: [Server] = []
            for await s in g { if let s { out.append(s) } }
            return out
        }
    }

    /// The Moonrise server at `base`, if one answers `/v1/info`.
    private static func confirm(_ base: URL, bonjour: Bool, timeout: TimeInterval = 3) async -> Server? {
        let req = HTTP.request(base.appendingPathComponent("v1/info"), timeout: timeout, cachePolicy: .reloadIgnoringLocalCacheData)
        guard let (data, status) = try? await HTTP.data(for: req, on: HTTP.sweep), status == 200,
              let info = try? MoonriseClient.decoder.decode(MoonriseClient.Info.self, from: data) else { return nil }
        return Server(id: info.serverId, name: info.name, version: info.version, address: base.absoluteString, bonjour: bonjour,
                      unclaimed: info.unclaimed)
    }
}
