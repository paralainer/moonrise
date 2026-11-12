import Darwin
import Foundation

/// The local network's addresses, for finding a Moonrise server by asking each host directly
/// (iOS only lets apps with the multicast entitlement send broadcasts).
enum LocalSubnet {
    /// Every host address (network byte order) of the device's private IPv4 /24 networks.
    static func hosts() -> [in_addr_t] {
        var out: [in_addr_t] = []
        var ifap: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&ifap) == 0, let first = ifap else { return [] }
        defer { freeifaddrs(ifap) }
        var seen = Set<UInt32>()
        for p in sequence(first: first, next: { $0.pointee.ifa_next }) {
            let ifa = p.pointee
            guard let sa = ifa.ifa_addr, sa.pointee.sa_family == sa_family_t(AF_INET),
                  ifa.ifa_flags & UInt32(IFF_LOOPBACK) == 0, ifa.ifa_flags & UInt32(IFF_UP) != 0 else { continue }
            let name = String(cString: ifa.ifa_name)
            guard name.hasPrefix("en") || name.hasPrefix("bridge") else { continue } // skip VPN/cellular
            let ip = sa.withMemoryRebound(to: sockaddr_in.self, capacity: 1) { UInt32(bigEndian: $0.pointee.sin_addr.s_addr) }
            let a = ip >> 24, b = (ip >> 16) & 0xff
            guard a == 10 || (a == 192 && b == 168) || (a == 172 && (16...31).contains(b)) else { continue }
            let net = ip & 0xffff_ff00
            guard seen.insert(net).inserted else { continue }
            // This device's own address too: the Mac app (or a simulator) on the Mac that runs
            // the server.
            for host in 1...254 { out.append((net | UInt32(host)).bigEndian) }
        }
        return out
    }
}
