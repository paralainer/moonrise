#if os(iOS)
import ActivityKit
import Foundation

/// Live Activity shown on the Lock Screen and in the Dynamic Island while downloads run.
struct DownloadActivityAttributes: ActivityAttributes {
    struct ContentState: Codable, Hashable {
        var title: String
        var detail: String
        var progress: Double
        var bytesPerSecond: Double
        var queued: Int
        var isPaused: Bool
        var isFinished: Bool
    }

    var startedAt: Date
}
#endif
