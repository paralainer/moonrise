#if os(macOS)
import SwiftUI

struct EngineViewHost: NSViewRepresentable {
    let view: NSView?
    func makeNSView(context: Context) -> EngineContainer {
        let container = EngineContainer()
        container.wantsLayer = true
        container.layer?.backgroundColor = NSColor.black.cgColor
        container.host(view)
        return container
    }
    func updateNSView(_ nsView: EngineContainer, context: Context) { nsView.host(view) }

    final class EngineContainer: NSView {
        private weak var hosted: NSView?
        func host(_ v: NSView?) {
            guard v !== hosted else { return }
            hosted = v
            attach()
        }
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if window != nil { attach() }
        }
        private func attach() {
            guard let v = hosted, v.superview !== self else { return }
            subviews.forEach { $0.removeFromSuperview() }
            v.frame = bounds
            v.autoresizingMask = [.width, .height]
            addSubview(v)
        }
    }
}

extension PlayerScreen {
    func toggleMacFullScreen() {
        fullScreenAction?()
        showControls()
    }

    /// Tells the Playback menu this is the player to drive (not while one of its sheets is up:
    /// the keys belong to the sheet's fields then), and whether it fills the screen (it still
    /// does under a sheet: the window's toolbar stays hidden).
    func registerWithMenu() {
        let hub = MacPlayerHub.shared
        #if DEBUG
        hub.debugState = { "controls=\(model.controlsVisible) hovering=\(hovering) scrubbing=\(scrubbing.map { "\(Int($0))" } ?? "-") keyFocus=\(keyFocus) inlineFull=\(inline?.fullScreen.description ?? "-")" }
        #endif
        hub.register(hubID, model: model, fullScreen: macFullScreen, sheetOpen: showSubtitleSearch || showEpisodes,
                     toggle: fullScreenAction) { level in
            showControls()
            guard let level else { return }
            hud = level
            hudTask?.cancel()
            hudTask = Task {
                try? await Task.sleep(for: .seconds(1))
                if !Task.isCancelled { hud = nil }
            }
        }
    }

    /// Scrolling sideways over the video: the scrubber follows; it seeks when the swipe ends.
    func scrollScrub(_ seconds: Double, ended: Bool) {
        guard model.duration > 0 else { return }
        let base = scrubbing ?? model.currentTime
        let t = min(max(0, base + seconds), model.duration)
        showControls()
        if ended {
            model.seek(to: scrubbing ?? t)
            scrubbing = nil
            scheduleHide()
        } else {
            scrubbing = t
        }
    }
}

/// Sideways scrolling over its area (trackpad swipe, a mouse's tilt wheel or Shift-wheel) as
/// seconds to move by: (seconds since the last call, the gesture ended). Vertical scrolling passes
/// on (the page under a title page's player scrolls).
struct ScrollScrubber: NSViewRepresentable {
    let onScrub: (Double, Bool) -> Void

    func makeNSView(context: Context) -> CatcherView {
        let v = CatcherView()
        v.onScrub = onScrub
        return v
    }
    func updateNSView(_ nsView: CatcherView, context: Context) { nsView.onScrub = onScrub }
    static func dismantleNSView(_ nsView: CatcherView, coordinator: ()) { nsView.stop() }

    final class CatcherView: NSView {
        var onScrub: ((Double, Bool) -> Void)?
        private var monitor: Any?
        private var active = false
        private var endTask: Task<Void, Never>?

        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            stop()
            guard window != nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] e in
                guard let self else { return e }
                return MainActor.assumeIsolated { self.handle(e) ? nil : e }
            }
        }

        func stop() {
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
        }

        /// True when the event was taken.
        private func handle(_ e: NSEvent) -> Bool {
            guard e.window === window, bounds.contains(convert(e.locationInWindow, from: nil)) else { return false }
            let dx = e.scrollingDeltaX, dy = e.scrollingDeltaY
            let phaseEnded = e.phase == .ended || e.phase == .cancelled
            if !active {
                // Starts only on a mostly sideways movement.
                guard abs(dx) > abs(dy) * 1.5, abs(dx) > 0.5 else { return false }
                active = true
            }
            // Momentum after the fingers lift: ignored, the seek is already made.
            if !e.momentumPhase.isEmpty { return true }
            // Fingers to the right = forward, whatever the scroll direction setting.
            let fingers = e.isDirectionInvertedFromDevice ? dx : -dx
            let seconds = e.hasPreciseScrollingDeltas ? Double(fingers) * 0.25 : Double(fingers) * 5
            onScrub?(seconds, false)
            endTask?.cancel()
            if phaseEnded {
                finish()
            } else {
                // A wheel has no phases: it ends when the clicks stop.
                endTask = Task { @MainActor [weak self] in
                    try? await Task.sleep(for: .milliseconds(e.phase.isEmpty ? 350 : 800))
                    guard !Task.isCancelled else { return }
                    self?.finish()
                }
            }
            return true
        }

        private func finish() {
            guard active else { return }
            active = false
            onScrub?(0, true)
        }
    }
}
#endif
