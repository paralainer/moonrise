import Foundation

enum Fmt {
    static func duration(_ seconds: Double) -> String {
        guard seconds.isFinite, seconds >= 0 else { return "--:--" }
        let s = Int(seconds.rounded(.down))
        let h = s / 3600, m = (s % 3600) / 60, sec = s % 60
        return h > 0 ? String(format: "%d:%02d:%02d", h, m, sec) : String(format: "%d:%02d", m, sec)
    }

    /// Always HH:MM:SS ("Resume 00:49:48").
    static func clock(_ seconds: Double) -> String {
        let s = max(0, Int(seconds))
        return String(format: "%02d:%02d:%02d", s / 3600, (s % 3600) / 60, s % 60)
    }

    static func runtime(seconds: Double?) -> String? {
        guard let seconds, seconds > 0 else { return nil }
        let minutes = Int(seconds / 60)
        return minutes >= 60 ? "\(minutes / 60)h \(minutes % 60)m" : "\(minutes)m"
    }

    static func bytes(_ n: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: n, countStyle: .file)
    }

    static func bitrate(_ bps: Int) -> String {
        bps >= 1_000_000 ? String(format: "%g Mbps", (Double(bps) / 100_000).rounded() / 10) : "\(bps / 1000) kbps"
    }
}

extension Double {
    var ticks: Int64 { Int64(self * 10_000_000) }
}

extension Int64 {
    var seconds: Double { Double(self) / 10_000_000 }
}
