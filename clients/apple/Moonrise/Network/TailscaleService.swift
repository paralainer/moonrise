import Foundation
import Observation
#if os(iOS)
import UIKit
#endif

#if canImport(TailscaleKit)
import TailscaleKit
#endif

/// Embedded Tailscale node (userspace, no VPN profile). The app joins the tailnet as its own device,
/// so the home server is reachable without the Tailscale app or a system VPN.
@MainActor
@Observable
final class TailscaleService {
    static let shared = TailscaleService()

    enum Status: Equatable {
        case off
        case starting
        case needsLogin(URL?)
        case running(ip: String)
        case failed(String)
    }

    static var isSupported: Bool {
        #if canImport(TailscaleKit)
        return true
        #else
        return false
        #endif
    }

    private(set) var status: Status = .off

    var enabled: Bool {
        didSet {
            UserDefaults.standard.set(enabled, forKey: "tailscaleEnabled")
            enabled ? start() : stop()
        }
    }

    var isRunning: Bool {
        if case .running = status { return true }
        return false
    }

    #if canImport(TailscaleKit)
    @ObservationIgnored private var node: TailscaleNode?
    @ObservationIgnored private var handle: TailscaleHandle?
    @ObservationIgnored private var api: LocalAPIClient?
    @ObservationIgnored private var processor: MessageProcessor?
    @ObservationIgnored private var forwarders: [String: PortForwarder] = [:]
    @ObservationIgnored private var nudge: Task<Void, Never>?
    #endif

    private var stateDir: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("tailscale", isDirectory: true)
    }

    /// Debug seam: MOONRISE_TS_AUTHKEY=<auth key> signs the node in without the web login (an
    /// ephemeral, pre-authorised key: `scripts/sim.sh` mints one for AWAY=1) and turns it on.
    private static var debugAuthKey: String? {
        #if DEBUG
        return DebugSeams.launch.tailscaleAuthKey
        #else
        return nil
        #endif
    }

    init() {
        enabled = UserDefaults.standard.bool(forKey: "tailscaleEnabled") || Self.debugAuthKey != nil
        if enabled { start() }
        #if os(iOS)
        NotificationCenter.default.addObserver(forName: UIApplication.willEnterForegroundNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated {
                TailscaleService.shared.reviveForwarders()
                Task { await ConnectionRouter.shared.refresh() }
            }
        }
        #endif
    }

    // MARK: Lifecycle

    func start() {
        #if canImport(TailscaleKit)
        guard node == nil else { return }
        status = .starting
        Task {
            do {
                try FileManager.default.createDirectory(at: stateDir, withIntermediateDirectories: true)
                let key = Self.debugAuthKey
                let config = Configuration(hostName: Self.hostName, path: stateDir.path, authKey: key,
                                           controlURL: kDefaultControlURL, ephemeral: key != nil)
                let node = try TailscaleNode(config: config, logger: TSLog())
                self.node = node
                self.handle = await node.tailscale
                let api = LocalAPIClient(localNode: node, logger: TSLog())
                self.api = api
                let consumer = BusConsumer { [weak self] notify in
                    Task { @MainActor in self?.handle(notify) }
                }
                processor = try await api.watchIPNBus(mask: [.initialState, .noPrivateKeys], consumer: consumer)
                // Blocks until the node is authorised (first run: after the user finishes web login).
                try await node.up()
                let ips = try await node.addrs()
                status = .running(ip: ips.ip4 ?? ips.ip6 ?? "")
                await ConnectionRouter.shared.refresh()
            } catch {
                if enabled { status = .failed(error.localizedDescription) }
            }
        }
        #endif
    }

    func stop() {
        #if canImport(TailscaleKit)
        forwarders.values.forEach { $0.close() }
        forwarders.removeAll()
        processor?.cancel()
        processor = nil
        let n = node
        node = nil
        handle = nil
        api = nil
        Task { try? await n?.close() }
        #endif
        status = .off
        Task { await ConnectionRouter.shared.refresh() }
    }

    /// iOS may tear down our listening sockets while suspended; rebuild them on the same ports.
    func reviveForwarders() {
        #if canImport(TailscaleKit)
        guard isRunning, let handle else { return }
        for (key, old) in forwarders {
            let port = old.port
            old.close()
            forwarders[key] = try? PortForwarder(target: key, preferredPort: port, dial: Self.tailnetDial(handle))
        }
        #endif
    }

    /// The device's network changed: the node's sockets may belong to a network that has gone.
    /// Waits a moment for the burst of path updates to end; best effort (see `TailnetNudge`).
    func networkChanged() {
        #if canImport(TailscaleKit)
        nudge?.cancel()
        nudge = Task { [weak self] in
            try? await Task.sleep(for: .seconds(1))
            guard !Task.isCancelled, let self, self.isRunning, let node = self.node else { return }
            guard let config = try? await node.loopback(), let host = config.ip, let port = config.port else { return }
            for request in TailnetNudge.requests(host: host, port: port, apiKey: config.localAPIKey) {
                let status = try? await HTTP.data(for: request).1
                #if DEBUG
                NSLog("MoonriseTS %@: %@", request.url?.query ?? "?", status.map(String.init) ?? "no answer")
                #endif
            }
        }
        #endif
    }

    /// Forget this device's login; next start asks to sign in again.
    func signOut() {
        stop()
        try? FileManager.default.removeItem(at: stateDir)
        enabled = false
    }

    #if canImport(TailscaleKit)
    private func handle(_ n: Ipn.Notify) {
        if let s = n.BrowseToURL, let url = URL(string: s) {
            #if DEBUG
            NSLog("MoonriseTS login URL %@", s)
            #endif
            status = .needsLogin(url)
        }
        switch n.State {
        case .NeedsLogin?:
            if case .needsLogin = status {} else { status = .needsLogin(nil) }
            Task { try? await api?.startLoginInteractive() }
        case .NeedsMachineAuth?:
            status = .failed("Approve this device in the Tailscale admin console.")
        default:
            break
        }
    }
    #endif

    // MARK: Tailnet access

    struct Peer: Identifiable, Hashable {
        var id: String { ip }
        let hostName: String
        let dnsName: String
        let ip: String
        let online: Bool
    }

    func peers() async -> [Peer] {
        #if canImport(TailscaleKit)
        // LocalAPI calls time out after 60 s; routing can't wait that long.
        guard let api else { return [] }
        // Cancelling is not enough: the request is cancellable, but getting to it waits on the node's actor.
        guard let status = await firstValue(within: 3, { try? await api.backendStatus() }) else { return [] }
        return (status.Peer.map { Array($0.values) } ?? []).compactMap { p in
            guard let ip = p.TailscaleIPs?.first(where: { !$0.contains(":") }) ?? p.TailscaleIPs?.first,
                  p.Tags == nil // skip tagged infra (exit nodes, relays)
            else { return nil }
            return Peer(hostName: p.HostName, dnsName: p.DNSName, ip: ip, online: p.Online)
        }
        .sorted { $0.hostName < $1.hostName }
        #else
        return []
        #endif
    }

    /// Local loopback port that tunnels to host:port on the tailnet.
    func forward(host: String, port: Int) throws -> UInt16 {
        #if canImport(TailscaleKit)
        let key = "\(host):\(port)"
        if let f = forwarders[key] { return f.port }
        guard isRunning, let handle else { throw URLError(.notConnectedToInternet) }
        let saved = UInt16(UserDefaults.standard.integer(forKey: "tsPort." + key))
        let f = try PortForwarder(target: key, preferredPort: saved, dial: Self.tailnetDial(handle))
        UserDefaults.standard.set(Int(f.port), forKey: "tsPort." + key)
        forwarders[key] = f
        return f.port
        #else
        throw URLError(.unsupportedURL)
        #endif
    }

    #if canImport(TailscaleKit)
    /// The node's dial (a C call: it can't be cancelled; `PortForwarder` copes with that).
    private static func tailnetDial(_ handle: TailscaleHandle) -> PortForwarder.Dial {
        { target in
            var remote: TailscaleConnection = 0
            return tailscale_dial(handle, "tcp", target, &remote) == 0 ? remote : nil
        }
    }
    #endif

    private static var hostName: String {
        #if os(macOS)
        return "moonrise-mac"
        #else
        return "moonrise-" + (ProcessInfo.processInfo.isiOSAppOnMac ? "mac" : "iphone")
        #endif
    }
}

#if canImport(TailscaleKit)
private struct TSLog: LogSink {
    var logFileHandle: Int32? { nil }
    func log(_ message: String) {
        #if DEBUG
        print("tailscale: \(message)")
        #endif
    }
}

private actor BusConsumer: MessageConsumer {
    let onNotify: @Sendable (Ipn.Notify) -> Void
    init(_ onNotify: @escaping @Sendable (Ipn.Notify) -> Void) { self.onNotify = onNotify }
    func notify(_ notify: Ipn.Notify) { onNotify(notify) }
    func error(_ error: Error) {}
}
#endif
