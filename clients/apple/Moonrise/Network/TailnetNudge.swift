import Foundation

/// The device's network changed: tell the embedded node to rebind now.
///
/// The node watches for link changes itself (its route socket, debounced by a second), but whether
/// those messages reach an ordinary sandboxed app isn't something it can be asked, and a stale UDP
/// socket is what leaves dials blocked for minutes. A rebind is the same thing it does on a link
/// change, so one more is harmless. It is best effort: the answer doesn't change what the app does.
enum TailnetNudge {
    /// What of the device's network matters to the node's sockets.
    struct Path: Equatable, Sendable {
        var online: Bool
        var interfaces: [String]
        var gateways: [String]
    }

    /// Worth a rebind: the path really changed (not the first report, which only says what is
    /// there at launch) and there's something to bind to.
    static func shouldNudge(from old: Path?, to new: Path) -> Bool {
        guard let old, old != new else { return false }
        return new.online
    }

    /// The node's own "rebind" and "re-STUN" LocalAPI calls (`tailscale debug rebind`, `restun`),
    /// to its loopback address with its LocalAPI credential.
    static func requests(host: String, port: Int, apiKey: String) -> [URLRequest] {
        let auth = Data("tsnet:\(apiKey)".utf8).base64EncodedString()
        return ["rebind", "restun"].compactMap { action in
            guard let url = URL(string: "http://\(host):\(port)/localapi/v0/debug?action=\(action)") else { return nil }
            return HTTP.request(url, method: "POST",
                                headers: ["Sec-Tailscale": "localapi", "Authorization": "Basic \(auth)"], timeout: 2)
        }
    }
}

/// The value of `work`, or nil if it doesn't come within `seconds`. The limit holds even for work
/// that ignores cancellation (it is cancelled anyway, and left to finish by itself).
func firstValue<T: Sendable>(within seconds: Double, _ work: @escaping @Sendable () async -> T?) async -> T? {
    let once = Once()
    return await withCheckedContinuation { (continuation: CheckedContinuation<T?, Never>) in
        let running = Task { @Sendable in
            let value = await work()
            if once.claim() { continuation.resume(returning: value) }
        }
        Task { @Sendable in
            try? await Task.sleep(for: .seconds(seconds))
            running.cancel()
            if once.claim() { continuation.resume(returning: nil) }
        }
    }
}

private final class Once: @unchecked Sendable {
    private let lock = NSLock()
    private var done = false
    func claim() -> Bool { lock.withLock { defer { done = true }; return !done } }
}
