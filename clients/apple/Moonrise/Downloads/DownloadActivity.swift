import Foundation

#if os(iOS)
import ActivityKit
import BackgroundTasks

/// Drives the downloads Live Activity (Lock Screen + Dynamic Island).
@MainActor
final class DownloadActivityController {
    private var activity: Activity<DownloadActivityAttributes>?
    private var lastUpdate = Date.distantPast
    private var lastState: DownloadActivityAttributes.ContentState?

    func begin(title: String) {
        guard ActivityAuthorizationInfo().areActivitiesEnabled else { return }
        if activity == nil {
            // Re-attach to one that survived an app relaunch.
            activity = Activity<DownloadActivityAttributes>.activities.first
        }
        guard activity == nil else { return }
        let state = DownloadActivityAttributes.ContentState(
            title: title, detail: "Starting…", progress: 0, bytesPerSecond: 0, queued: 0, isPaused: false, isFinished: false)
        activity = try? Activity.request(
            attributes: DownloadActivityAttributes(startedAt: Date()),
            content: ActivityContent(state: state, staleDate: nil),
            pushType: nil)
    }

    func update(title: String, detail: String, progress: Double, bytesPerSecond: Double, queued: Int) {
        guard let activity else { return }
        let state = DownloadActivityAttributes.ContentState(
            title: title, detail: detail, progress: progress, bytesPerSecond: bytesPerSecond,
            queued: queued, isPaused: false, isFinished: false)
        // ActivityKit throttles; ~1 update/sec is plenty and keeps us inside the budget.
        let significant = abs((lastState?.progress ?? -1) - progress) > 0.01 || lastState?.title != title
        guard Date().timeIntervalSince(lastUpdate) > 1, significant || Date().timeIntervalSince(lastUpdate) > 5 else { return }
        lastUpdate = Date()
        lastState = state
        Task { await activity.update(ActivityContent(state: state, staleDate: Date().addingTimeInterval(120))) }
    }

    func end(completedTitle: String?) {
        guard let activity else { return }
        self.activity = nil
        lastState = nil
        let final = DownloadActivityAttributes.ContentState(
            title: completedTitle ?? "Downloads", detail: "Ready to watch offline", progress: 1,
            bytesPerSecond: 0, queued: 0, isPaused: false, isFinished: true)
        Task {
            await activity.end(ActivityContent(state: final, staleDate: nil), dismissalPolicy: .after(Date().addingTimeInterval(8)))
        }
    }
}

/// iOS 26 continued processing: a user-initiated task that keeps the app running in the
/// background with a system progress UI while downloads (notably over the tailnet) are in flight.
@available(iOS 26.0, *)
@MainActor
final class ContinuedDownloadTask {
    static let shared = ContinuedDownloadTask()
    private static let prefix = AppInfo.bundleId + ".downloads."

    private var task: BGContinuedProcessingTask?
    private var pendingIdentifier: String?

    func begin(title: String) {
        guard task == nil, pendingIdentifier == nil else { return }
        let identifier = Self.prefix + UUID().uuidString
        pendingIdentifier = identifier
        BGTaskScheduler.shared.register(forTaskWithIdentifier: identifier, using: .main) { [weak self] t in
            MainActor.assumeIsolated {
                guard let self, let t = t as? BGContinuedProcessingTask else { return }
                self.task = t
                self.pendingIdentifier = nil
                t.progress.totalUnitCount = 1000
                t.expirationHandler = { [weak self] in
                    // System reclaimed us; URLSession downloads continue on their own.
                    MainActor.assumeIsolated { self?.task = nil }
                }
            }
        }
        let request = BGContinuedProcessingTaskRequest(identifier: identifier, title: title, subtitle: "Preparing…")
        request.strategy = .queue
        do {
            try BGTaskScheduler.shared.submit(request)
        } catch {
            pendingIdentifier = nil
        }
    }

    func update(fraction: Double, title: String, subtitle: String) {
        guard let task else { return }
        task.progress.completedUnitCount = Int64(fraction * 1000)
        task.updateTitle("Downloading \(title)", subtitle: subtitle)
    }

    func finish(success: Bool) {
        if let task {
            task.progress.completedUnitCount = task.progress.totalUnitCount
            task.setTaskCompleted(success: success)
        }
        task = nil
        if let id = pendingIdentifier {
            BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: id)
            pendingIdentifier = nil
        }
    }
}
#else

/// Live Activities are iOS-only; no-op elsewhere.
@MainActor
final class DownloadActivityController {
    func begin(title: String) {}
    func update(title: String, detail: String, progress: Double, bytesPerSecond: Double, queued: Int) {}
    func end(completedTitle: String?) {}
}
#endif
