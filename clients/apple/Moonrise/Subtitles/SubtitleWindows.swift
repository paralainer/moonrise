import Foundation

/// An embedded subtitle track of a Moonrise server title, shown before the server has extracted all
/// of it: fetched in windows of 30 s of the file's time (`MoonriseServer.subtitleWindow`) at the
/// playhead and the one after it, one request at a time, the one at the playhead first (a seek
/// drops a request for elsewhere). Cues are merged as they come, so nothing already shown goes
/// away. Once a window says the whole track is ready, the `.srt` is fetched once and replaces the
/// windows (seeking anywhere is then free).
@MainActor
final class SubtitleWindowLoader {
    static let span = 30.0
    /// Windows fetched past the one at the playhead (one is 30 s of lead; a window takes well
    /// under a second, and every extra one is read off the share while the video starts).
    private static let ahead = 1

    private let base: URL
    private let itemId: String
    private let index: Int
    private let headers: [String: String]
    /// The whole track (`.srt`).
    private let fullURL: URL
    private let fetchFull: (URL) async throws -> Data
    /// The cues known so far (sorted), whenever they change.
    private let onCues: ([SubtitleCue]) -> Void
    /// The server doesn't serve windows (any more): load the whole track instead.
    private let onFallback: () -> Void
    private let log: (String) -> Void

    private var cues = Set<SubtitleCue>()
    private var loaded = Set<Int>()
    /// The last window there is (the server said "past the end" for the next).
    private var last: Int?
    private var position = 0.0
    private var current: Int?
    private var task: Task<Void, Never>?
    private var fullTask: Task<Void, Never>?
    private var complete = false
    private var stopped = false
    private let started = Date()
    private var first = true

    init(base: URL, itemId: String, index: Int, headers: [String: String], fullURL: URL,
         fetchFull: @escaping (URL) async throws -> Data, onCues: @escaping ([SubtitleCue]) -> Void,
         onFallback: @escaping () -> Void, log: @escaping (String) -> Void) {
        self.base = base
        self.itemId = itemId
        self.index = index
        self.headers = headers
        self.fullURL = fullURL
        self.fetchFull = fetchFull
        self.onCues = onCues
        self.onFallback = onFallback
        self.log = log
    }

    func stop() {
        stopped = true
        task?.cancel()
        fullTask?.cancel()
    }

    /// The playhead is at `t` (the file's time): fetch what's missing around it.
    func advance(to t: Double) {
        position = max(0, t)
        guard !stopped, !complete else { return }
        let k = Int(position / Self.span)
        let needed = (k...(k + Self.ahead)).filter { w in !loaded.contains(w) && (last.map { w <= $0 } ?? true) }
        guard let next = needed.first else { return }
        if let current {
            // Already fetching one that's wanted (the player lands a little before a seek's
            // target, often in the window before): it finishes first.
            if needed.contains(current) { return }
            // A seek elsewhere: the playhead's window first.
            task?.cancel()
        }
        fetch(next)
    }

    private func fetch(_ k: Int) {
        current = k
        log(String(format: "subtitle window %d asked for (playhead %.1f)", k, position))
        // The request goes out now, off the main actor (busy setting up the player when a title
        // starts); only its answer waits for it.
        let request = { [base, itemId, index, headers] in
            Task.detached { try await MoonriseServer.subtitleWindow(base: base, itemId: itemId, index: index, window: k, headers: headers) }
        }
        let firstAttempt = request()
        let t0 = Date()
        task = Task { [weak self] in
            var attempt = firstAttempt
            var delay = 0.5
            while true {
                let pending = attempt
                do {
                    let w = try await withTaskCancellationHandler { try await pending.value } onCancel: { pending.cancel() }
                    guard let self, !Task.isCancelled, !self.stopped else { return }
                    self.received(w, ms: Date().timeIntervalSince(t0) * 1000)
                    break
                } catch let MoonriseServer.ServerError.http(code, body) where code == 404 && body.contains("past the end") {
                    self?.last = k - 1
                    break
                } catch let MoonriseServer.ServerError.http(code, _) where [404, 405, 410, 501, 503].contains(code) {
                    // A server without windows (rolled back, or switched off): the whole track.
                    guard let self, !Task.isCancelled, !self.stopped else { return }
                    self.log("subtitle windows unavailable (\(code)): the whole track instead")
                    self.stop()
                    self.onFallback()
                    return
                } catch {
                    guard let self, !Task.isCancelled, !self.stopped else { return }
                    // Not ready or unreachable: again soon.
                    self.log("subtitle window \(k) failed (\(error.localizedDescription)), again in \(delay) s")
                    try? await Task.sleep(for: .seconds(delay))
                    if Task.isCancelled { return }
                    delay = min(delay * 2, 4)
                    attempt = request()
                }
            }
            guard let self, !Task.isCancelled, self.current == k else { return }
            self.current = nil
            self.advance(to: self.position)
        }
    }

    private func received(_ w: MoonriseServer.SubtitleWindow, ms: Double) {
        loaded.insert(w.window)
        let before = cues.count
        for c in w.cues {
            cues.insert(SubtitleCue(start: c.start, end: c.end, text: SubtitleParser.stripTags(c.text).trimmingCharacters(in: .whitespacesAndNewlines)))
        }
        log(String(format: "subtitle window %d: %d cues (%@) in %.0f ms%@%@", w.window, w.cues.count, w.source ?? "?", ms,
                   first ? String(format: ", %.0f ms after the track was chosen", Date().timeIntervalSince(started) * 1000) : "",
                   w.complete ? ", track complete" : ""))
        first = false
        if cues.count != before { onCues(cues.sorted { ($0.start, $0.end) < ($1.start, $1.end) }) }
        if w.complete { loadFull() }
    }

    /// The whole track is ready on the server: one fetch, then no more windows.
    private func loadFull() {
        guard !complete else { return }
        complete = true
        fullTask = Task { [weak self] in
            guard let self else { return }
            do {
                let data = try await self.fetchFull(self.fullURL)
                guard !Task.isCancelled, !self.stopped else { return }
                let doc = SubtitleParser.parse(data)
                self.log("subtitle track complete: \(doc.cues.count) cues")
                self.task?.cancel()
                self.onCues(doc.cues)
            } catch {
                guard !Task.isCancelled, !self.stopped else { return }
                // Windows go on.
                self.complete = false
                self.advance(to: self.position)
            }
        }
    }
}
