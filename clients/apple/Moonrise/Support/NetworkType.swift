import Foundation
import Network

/// Whether the device is currently on a cellular connection (for picking Wi-Fi vs cellular quality).
final class NetworkType: @unchecked Sendable {
    static let shared = NetworkType()
    private let monitor = NWPathMonitor()
    private let lock = NSLock()
    private var cellular = false

    private init() {
        monitor.pathUpdateHandler = { [weak self] path in
            let cell = path.usesInterfaceType(.cellular) && !path.usesInterfaceType(.wifi) && !path.usesInterfaceType(.wiredEthernet)
            self?.lock.withLock { self?.cellular = cell }
        }
        monitor.start(queue: DispatchQueue(label: "moonrise.nettype"))
    }

    var isCellular: Bool {
        #if DEBUG
        if DebugSeams.launch.forceCellular { return true }
        #endif
        return lock.withLock { cellular }
    }

    /// Only phones and cellular iPads can be on cellular; elsewhere the setting is hidden.
    static var hasCellular: Bool {
        #if os(iOS)
        return true
        #else
        return false
        #endif
    }
}
