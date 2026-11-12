#if os(iOS)
import SwiftUI

/// Gestures: vertical swipe on the left half = brightness, right half = volume,
/// double-tap left/right = skip, pinch = fill/fit, tap = controls. Horizontal swipes do nothing
/// (seeking is the scrubber's).
struct GestureLayer: View {
    let model: PlayerModel
    let size: CGSize
    @Binding var hud: GestureHUD?
    let onTap: () -> Void
    let onInteraction: () -> Void
    /// Portrait: vertical drags go here (translation, ended) instead of brightness/volume.
    var onVerticalDrag: ((CGFloat, Bool) -> Void)? = nil
    /// Reaches under the safe areas (not in a title page's header, below its navigation bar).
    var fullBleed = true
    /// A title page's full-screen player on iPad: a swipe down from the top quarter leaves full
    /// screen (lower down, vertical swipes stay brightness and volume).
    var onSwipeDownFromTop: (() -> Void)? = nil

    private enum Mode { case brightness, volume, vertical, dismiss, ignored }
    @State private var mode: Mode?
    @State private var startValue: Double = 0
    @State private var hudTask: Task<Void, Never>?

    var body: some View {
        Color.clear
            .contentShape(Rectangle())
            .gesture(drag)
            .simultaneousGesture(MagnifyGesture().onEnded { v in
                model.fill = v.magnification > 1
            })
            .gesture(SpatialTapGesture(count: 2).onEnded { v in
                if model.autoSkipProgress != nil { model.cancelAutoSkip() } // a tap on the video stops it
                let third = size.width / 3
                if v.location.x < third {
                    // Double-taps in a row add up, as the buttons do: the running total shows.
                    model.skip(-AppSettings.shared.skipBackward); flash(.skip(model.skipTotal ?? -AppSettings.shared.skipBackward))
                } else if v.location.x > third * 2 {
                    model.skip(AppSettings.shared.skipForward); flash(.skip(model.skipTotal ?? AppSettings.shared.skipForward))
                } else {
                    model.fill.toggle()
                }
            }.exclusively(before: TapGesture().onEnded { onTap() }))
            .ignoresSafeArea(edges: fullBleed ? .all : [])
    }

    private var drag: some Gesture {
        DragGesture(minimumDistance: 12)
            .onChanged { v in
                if mode == nil {
                    // Ignore swipes starting at the very edges (system gestures).
                    guard !fullBleed || (v.startLocation.y > 30 && v.startLocation.y < size.height - 20) else { return }
                    if abs(v.translation.width) > abs(v.translation.height) {
                        mode = .ignored
                    } else if onSwipeDownFromTop != nil, v.translation.height > 0, v.startLocation.y < size.height * 0.25 {
                        mode = .dismiss
                    } else if onVerticalDrag != nil {
                        mode = .vertical
                    } else if v.startLocation.x < size.width / 2 {
                        mode = .brightness
                        startValue = SystemControls.brightness
                    } else {
                        mode = .volume
                        startValue = Double(SystemControls.canSetSystemVolume ? SystemControls.volume : model.playerVolume)
                    }
                }
                hudTask?.cancel()
                switch mode {
                case .brightness:
                    let v2 = clamp(startValue - v.translation.height / (size.height * 0.8))
                    SystemControls.brightness = v2
                    hud = .brightness(v2)
                case .volume:
                    let v2 = clamp(startValue - v.translation.height / (size.height * 0.8))
                    if SystemControls.canSetSystemVolume {
                        SystemControls.volume = Float(v2)
                    } else {
                        model.playerVolume = Float(v2)
                    }
                    hud = .volume(v2)
                case .vertical:
                    onVerticalDrag?(v.translation.height, false)
                case .dismiss, .ignored, nil: break
                }
            }
            .onEnded { v in
                if mode == .vertical { onVerticalDrag?(v.translation.height, true) }
                if mode == .dismiss, v.translation.height > 80 || v.predictedEndTranslation.height > 200 { onSwipeDownFromTop?() }
                mode = nil
                onInteraction()
                hideHUD()
            }
    }

    private func clamp(_ v: Double) -> Double { min(1, max(0, v)) }

    private func flash(_ h: GestureHUD) {
        hud = h
        hideHUD()
    }

    private func hideHUD() {
        hudTask?.cancel()
        hudTask = Task {
            try? await Task.sleep(for: .milliseconds(700))
            if !Task.isCancelled { hud = nil }
        }
    }
}
#endif
