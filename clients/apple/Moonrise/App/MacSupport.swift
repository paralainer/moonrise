#if os(macOS)
import AppKit
import SwiftUI

// The Mac's window, menu bar and keyboard: one main window (its frame remembered, a minimum
// size, full screen with the green button or ⌃⌘F), tabs in the View menu (⌘1–⌘4, ⌘F for
// Search), Settings in the app menu (⌘,), and a Playback menu that drives the player on screen
// (Space, ←/→, ↑/↓, M, F) as the Apple TV app, QuickTime and IINA do.

/// The app's main window and whether it is in full screen.
@MainActor
@Observable
final class MacWindow {
    static let shared = MacWindow()
    @ObservationIgnored weak var window: NSWindow?
    private(set) var isFullScreen = false
    @ObservationIgnored private var observers: [NSObjectProtocol] = []

    static let minSize = CGSize(width: 800, height: 540)

    func attach(_ w: NSWindow) {
        guard w !== window else { return }
        window = w
        w.tabbingMode = .disallowed
        w.collectionBehavior.insert(.fullScreenPrimary)
        w.contentMinSize = Self.minSize
        // Same size and place at the next launch.
        w.setFrameAutosaveName("MoonriseMain")
        isFullScreen = w.styleMask.contains(.fullScreen)
        // SwiftUI still puts Toggle Sidebar in the menu bar (Help); there's no app sidebar.
        DispatchQueue.main.async {
            for top in NSApp.mainMenu?.items ?? [] {
                for item in top.submenu?.items ?? [] where item.action == #selector(NSSplitViewController.toggleSidebar(_:)) {
                    top.submenu?.removeItem(item)
                }
            }
        }
        observers.forEach(NotificationCenter.default.removeObserver)
        let nc = NotificationCenter.default
        observers = [
            nc.addObserver(forName: NSWindow.didEnterFullScreenNotification, object: w, queue: .main) { _ in
                MainActor.assumeIsolated {
                    MacWindow.shared.isFullScreen = true
                    MacWindow.shared.applyToolbar()
                }
            },
            // AppKit puts back the toolbar it had before full screen: show ours as it should be.
            nc.addObserver(forName: NSWindow.didExitFullScreenNotification, object: w, queue: .main) { _ in
                MainActor.assumeIsolated { MacWindow.shared.applyToolbar() }
            },
            nc.addObserver(forName: NSWindow.willExitFullScreenNotification, object: w, queue: .main) { _ in
                MainActor.assumeIsolated {
                    MacWindow.shared.isFullScreen = false
                    // Left with the green button, ⌃⌘F or the menu: the player leaves its full screen too.
                    MacPlayerHub.shared.windowLeftFullScreen?()
                }
            },
        ]
    }

    /// The toolbar (tabs, back button) shows unless the player fills the screen.
    func applyToolbar() {
        let visible = !MacPlayerHub.shared.playerFullScreen
        if let toolbar = window?.toolbar, toolbar.isVisible != visible { toolbar.isVisible = visible }
    }

    func setFullScreen(_ on: Bool) {
        guard let w = window ?? NSApp.keyWindow, w.styleMask.contains(.fullScreen) != on else { return }
        w.toggleFullScreen(nil)
    }
}

/// Finds the window it is in and hands it to `MacWindow`.
struct MacWindowReader: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { Reader() }
    func updateNSView(_ nsView: NSView, context: Context) {}
    private final class Reader: NSView {
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let w = window { MacWindow.shared.attach(w) }
        }
    }
}

/// The player on screen (a title page's, or the Downloads tab's), for the Playback menu.
@MainActor
@Observable
final class MacPlayerHub {
    static let shared = MacPlayerHub()
    /// Set while a player shows and no sheet covers it.
    private(set) var model: PlayerModel?
    @ObservationIgnored private var owner: UUID?
    /// The player fills the window (and the screen): no toolbar (tabs, back button) over it, as
    /// in QuickTime and the TV app; it comes back when the player leaves full screen.
    var playerFullScreen = false {
        didSet { if playerFullScreen != oldValue { MacWindow.shared.applyToolbar() } }
    }
    @ObservationIgnored var toggleFullScreen: (() -> Void)?
    /// The window left full screen on its own: the player's full screen ends with it.
    @ObservationIgnored var windowLeftFullScreen: (() -> Void)?
    /// Shows the controls and a level (volume) over the video.
    @ObservationIgnored var feedback: ((GestureHUD?) -> Void)?
    #if DEBUG
    @ObservationIgnored var debugState: (() -> String)?
    var debugDescription: String {
        guard let m = model else { return "no player" }
        return "playing=\(m.isPlaying) t=\(Int(m.currentTime)) volume=\(volume) muted=\(muted) playerFull=\(playerFullScreen) windowFull=\(MacWindow.shared.isFullScreen) \(debugState?() ?? "")"
    }
    #endif

    var volume: Float = UserDefaults.standard.object(forKey: "mac.volume") as? Float ?? 1 {
        didSet { UserDefaults.standard.set(volume, forKey: "mac.volume"); apply() }
    }
    var muted = false { didSet { apply() } }

    func register(_ id: UUID, model: PlayerModel, fullScreen: Bool, toggle: (() -> Void)?,
                  feedback: @escaping (GestureHUD?) -> Void) {
        owner = id
        if self.model !== model { self.model = model }
        playerFullScreen = fullScreen
        toggleFullScreen = toggle
        self.feedback = feedback
        apply()
    }

    func unregister(_ id: UUID) {
        guard owner == id else { return }
        owner = nil
        model = nil
        playerFullScreen = false
        toggleFullScreen = nil
        feedback = nil
    }

    private func apply() { model?.playerVolume = muted ? 0 : volume }

    func changeVolume(_ delta: Float) {
        muted = false
        volume = min(1, max(0, ((volume + delta) * 100).rounded() / 100))
        feedback?(.volume(Double(volume)))
    }

    func toggleMute() {
        muted.toggle()
        feedback?(.volume(muted ? 0 : Double(volume)))
    }

    func togglePlay() { model?.togglePlay(); feedback?(nil) }
    func skip(_ forward: Bool) {
        let s = AppSettings.shared
        model?.skip(forward ? s.skipForward : -s.skipBackward)
        feedback?(nil)
    }
}

/// Focuses Search's field (⌘F).
@MainActor
@Observable
final class MacSearchFocus {
    static let shared = MacSearchFocus()
    var request = 0
}

struct MoonriseCommands: Commands {
    private var nav: AppNavigation { AppNavigation.shared }

    var body: some Commands {
        // One window: the tabs and the player are app-wide state.
        CommandGroup(replacing: .newItem) {}
        // No app sidebar (Library has its own): no Toggle Sidebar.
        CommandGroup(replacing: .sidebar) {}
        CommandGroup(after: .textEditing) {
            Button("Search…") { showSearch() }
                .keyboardShortcut("f", modifiers: .command)
        }
        CommandMenu("Playback") { PlaybackMenuItems() }
        CommandGroup(before: .toolbar) { TabMenuItems() }
    }

    private func showSearch() {
        nav.tab = 2
        MacSearchFocus.shared.request += 1
    }
}

/// View: the tabs (a view, so the checkmark follows the tab).
private struct TabMenuItems: View {
    private var nav: AppNavigation { AppNavigation.shared }
    var body: some View {
        tab("Home", 0, "1")
        tab("Library", 1, "2")
        if DownloadManager.isSupported { tab("Downloads", AppNavigation.downloadsTab, "3") }
        tab("Search", 2, "4")
        Divider()
        // The window's full screen (the green button). Leaving it also ends the player's.
        Button(MacWindow.shared.isFullScreen ? "Exit Full Screen" : "Enter Full Screen") {
            MacWindow.shared.setFullScreen(!MacWindow.shared.isFullScreen)
        }
        .keyboardShortcut("f", modifiers: [.control, .command])
        Divider()
    }
    private func tab(_ title: String, _ value: Int, _ key: KeyEquivalent) -> some View {
        Toggle(title, isOn: Binding(get: { nav.tab == value }, set: { if $0 { nav.tab = value } }))
            .keyboardShortcut(key, modifiers: .command)
    }
}

/// Playback: a view, so its titles and enabled states follow the player on screen.
private struct PlaybackMenuItems: View {
    private var hub: MacPlayerHub { MacPlayerHub.shared }
    var body: some View {
        let on = hub.model != nil
        Button(hub.model?.isPlaying == true ? "Pause" : "Play") { hub.togglePlay() }
            .keyboardShortcut(.space, modifiers: [])
            .disabled(!on)
        Button("Skip Forward \(Int(AppSettings.shared.skipForward)) Seconds") { hub.skip(true) }
            .keyboardShortcut(.rightArrow, modifiers: [])
            .disabled(!on)
        Button("Skip Back \(Int(AppSettings.shared.skipBackward)) Seconds") { hub.skip(false) }
            .keyboardShortcut(.leftArrow, modifiers: [])
            .disabled(!on)
        Divider()
        Button("Volume Up") { hub.changeVolume(0.1) }
            .keyboardShortcut(.upArrow, modifiers: [])
            .disabled(!on)
        Button("Volume Down") { hub.changeVolume(-0.1) }
            .keyboardShortcut(.downArrow, modifiers: [])
            .disabled(!on)
        Button(hub.muted ? "Unmute" : "Mute") { hub.toggleMute() }
            .keyboardShortcut("m", modifiers: [])
            .disabled(!on)
        Divider()
        Button(hub.playerFullScreen ? "Exit Full Screen Player" : "Full Screen Player") { hub.toggleFullScreen?() }
            .keyboardShortcut("f", modifiers: [])
            .disabled(!on || hub.toggleFullScreen == nil)
    }
}

/// The app menu's Settings… (⌘,): the same screens as the iPad's Settings sheet.
struct MacSettingsScene: Scene {
    var body: some Scene {
        Settings {
            NavigationStack { SettingsView() }
                .frame(width: 560, height: 640)
                .preferredColorScheme(.dark)
                .tint(Theme.accent)
        }
    }
}
#endif

#if os(macOS)
import AVKit

/// AirPlay: the system's route button, drawn plain like the player's buttons.
struct MacRoutePicker: NSViewRepresentable {
    var player: AVPlayer?
    func makeNSView(context: Context) -> AVRoutePickerView {
        let v = AVRoutePickerView()
        v.isRoutePickerButtonBordered = false
        v.setRoutePickerButtonColor(.white, for: .normal)
        v.setRoutePickerButtonColor(.white, for: .normalHighlighted)
        v.player = player
        return v
    }
    func updateNSView(_ v: AVRoutePickerView, context: Context) {
        if v.player !== player { v.player = player }
    }
}
#endif
