#if os(iOS)
import AVFoundation
import AVKit
import MediaPlayer
import SwiftUI
import UIKit

/// Screen brightness + system volume for the swipe gestures.
/// Volume can only be set through MPVolumeView's slider, so a hidden one is kept in the hierarchy.
@MainActor
enum SystemControls {
    fileprivate static weak var volumeView: MPVolumeView?
    fileprivate static weak var cachedSlider: UISlider?

    /// MPVolumeView builds its slider lazily and nests it, so search the whole tree (and retry on demand).
    private static var volumeSlider: UISlider? {
        if let s = cachedSlider { return s }
        guard let v = volumeView else { return nil }
        v.layoutIfNeeded()
        func find(_ view: UIView) -> UISlider? {
            if let s = view as? UISlider { return s }
            for sub in view.subviews { if let s = find(sub) { return s } }
            return nil
        }
        cachedSlider = find(v)
        return cachedSlider
    }

    static var screen: UIScreen? {
        (UIApplication.shared.connectedScenes.first { $0.activationState == .foregroundActive } as? UIWindowScene)?.screen
    }

    static var brightness: CGFloat {
        get { screen?.brightness ?? 0.5 }
        set { screen?.brightness = min(1, max(0, newValue)) }
    }

    /// False when iOS exposes no volume slider (e.g. simulator); callers fall back to player volume.
    static var canSetSystemVolume: Bool { volumeSlider != nil }

    static var volume: Float {
        get { AVAudioSession.sharedInstance().outputVolume }
        set {
            guard let slider = volumeSlider else { return }
            slider.setValue(min(1, max(0, newValue)), animated: false)
            slider.sendActions(for: .valueChanged)
            slider.sendActions(for: .touchUpInside)
        }
    }

    /// Rotate the interface without locking it, so turning the phone still rotates it back.
    static func rotate(toLandscape landscape: Bool) {
        AppDelegate.orientationLock = .allButUpsideDown
        guard UIDevice.current.userInterfaceIdiom == .phone,
              let scene = UIApplication.shared.connectedScenes.first as? UIWindowScene else { return }
        scene.keyWindow?.rootViewController?.setNeedsUpdateOfSupportedInterfaceOrientations()
        scene.requestGeometryUpdate(.iOS(interfaceOrientations: landscape ? .landscapeRight : .portrait)) { error in
            #if DEBUG
            fputs("MoonriseRotate: \(landscape ? "landscape" : "portrait") failed: \(error.localizedDescription)\n", stderr)
            #endif
        }
        #if DEBUG
        fputs("MoonriseRotate: \(landscape ? "landscape" : "portrait")\n", stderr)
        #endif
    }

    static var isLandscape: Bool {
        (UIApplication.shared.connectedScenes.first as? UIWindowScene)?.interfaceOrientation.isLandscape ?? false
    }

    /// Ask for landscape while the player is up, restore afterwards.
    static func setLandscape(_ landscape: Bool) {
        AppDelegate.orientationLock = landscape ? .landscape : .allButUpsideDown
        guard let scene = UIApplication.shared.connectedScenes.first as? UIWindowScene else { return }
        // Refresh the allowed set first, then ask for the rotation (the other order is ignored).
        scene.keyWindow?.rootViewController?.setNeedsUpdateOfSupportedInterfaceOrientations()
        if UIDevice.current.userInterfaceIdiom == .phone {
            scene.requestGeometryUpdate(.iOS(interfaceOrientations: landscape ? .landscapeRight : .portrait)) { _ in }
        }
    }
}

/// Invisible MPVolumeView. Also suppresses the system volume HUD while the player is on screen.
struct HiddenVolumeView: UIViewRepresentable {
    func makeUIView(context: Context) -> MPVolumeView {
        // Must be in the window and not fully transparent, or iOS ignores it and shows its own HUD.
        let v = MPVolumeView(frame: CGRect(x: 0, y: 0, width: 100, height: 40))
        v.alpha = 0.01
        v.isUserInteractionEnabled = false
        SystemControls.volumeView = v
        SystemControls.cachedSlider = nil
        return v
    }

    func updateUIView(_ uiView: MPVolumeView, context: Context) {}
}

struct RoutePickerButton: UIViewRepresentable {
    func makeUIView(context: Context) -> AVRoutePickerView {
        let v = AVRoutePickerView()
        v.tintColor = .white
        v.activeTintColor = UIColor(Theme.accent)
        v.prioritizesVideoDevices = true
        return v
    }

    func updateUIView(_ uiView: AVRoutePickerView, context: Context) {}
}
#endif
