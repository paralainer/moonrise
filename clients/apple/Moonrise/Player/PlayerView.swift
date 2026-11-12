import SwiftUI

/// Hosts the engine's platform view; re-created when the engine is swapped.
struct VideoSurface: View {
    let model: PlayerModel
    var fullBleed = true

    var body: some View {
        EngineViewHost(view: model.engine?.view)
            .id(model.engineGeneration)
            .ignoresSafeArea(edges: fullBleed ? .all : [])
    }
}

/// The engine's view is one view that moves between hosts: a title page draws the same player
/// in its header or full screen. The host that last entered a window takes it, so the one going
/// away can't take it back.
#if !os(macOS)
struct EngineViewHost: UIViewRepresentable {
    let view: UIView?
    func makeUIView(context: Context) -> EngineContainer {
        let container = EngineContainer()
        container.backgroundColor = .black
        container.host(view)
        return container
    }
    func updateUIView(_ uiView: EngineContainer, context: Context) { uiView.host(view) }

    final class EngineContainer: UIView {
        private weak var hosted: UIView?
        func host(_ v: UIView?) {
            guard v !== hosted else { return }
            hosted = v
            attach()
        }
        override func didMoveToWindow() {
            super.didMoveToWindow()
            if window != nil { attach() }
        }
        private func attach() {
            guard let v = hosted, v.superview !== self else { return }
            subviews.forEach { $0.removeFromSuperview() }
            v.frame = bounds
            v.autoresizingMask = [.flexibleWidth, .flexibleHeight]
            addSubview(v)
        }
    }
}
#endif

/// A title page's player (iPhone, iPad, Mac): the page owns the model (starts, switches and
/// closes it); the same model is drawn in the page's header or full screen.
struct InlinePlayerHooks {
    /// Full screen (else the header's 16:9 box, with compact controls).
    var fullScreen: Bool
    var toggleFullScreen: () -> Void
    /// The video swiped up.
    var swipeUp: () -> Void
    /// The header's top-left button: leave the page (as its back button does; that stops playback).
    var back: () -> Void
    /// The video dragged down: (distance, velocity, let go); far enough stops playback on the page.
    var dragDown: (_ dy: CGFloat, _ velocity: CGFloat, _ ended: Bool) -> Void
    /// One of the player's sheets opened (true) or closed: the page is covered, not left.
    var onModal: (Bool) -> Void
    /// iPhone: Download in the player's controls (the page has no row for it while playing).
    var download: InlineDownload? = nil
}

struct PlayerScreen: View {
    @State var model: PlayerModel
    var onClose: () -> Void = {}
    /// Set: drawn inside a title page instead of as its own screen.
    var inline: InlinePlayerHooks? = nil

    /// Where the controls' parts are drawn on the stage (for the subtitles).
    @State private var controlsLayout = ControlsLayout()
    @State private var hideTask: Task<Void, Never>?
    /// A track or quality menu is open: keep the controls up until it closes.
    @State private var menuOpen = false
    @State var scrubbing: Double?
    @State var hud: GestureHUD?
    @State var showSubtitleSearch = false
    @State var showEpisodes = false
    #if os(tvOS)
    @FocusState private var focused: Bool
    /// Playing when Search Online… opened: play again when it closes.
    @State var resumeAfterSearch = false
    /// The system player's transport bar is showing.
    @State var tvBarVisible = false
    /// The subtitles' bottom edge above the screen's bottom while the transport bar shows.
    static let tvRaisedBottom: CGFloat = 370
    #endif

    @Environment(PlaybackCoordinator.self) private var playback
    #if os(macOS)
    /// This player's registration with the Playback menu.
    @State var hubID = UUID()
    /// The pointer is over the player.
    @State var hovering = false
    @State var hudTask: Task<Void, Never>?
    @FocusState var keyFocus: Bool
    #endif

    #if !os(tvOS)
    var body: some View {
        customPlayer
    }
    #endif

    var customPlayer: some View {
        GeometryReader { geo in
            ZStack {
                Color.black.ignoresSafeArea(edges: inlineCompact ? [] : .all)
                stage(size: geo.size, compact: inlineCompact)
                if let err = model.errorMessage {
                    ErrorBanner(title: model.errorTitle, message: err, detail: model.errorDetail) { model.errorMessage = nil }
                }
            }
        }
        .preferredColorScheme(.dark)
        .animation(.easeInOut(duration: 0.2), value: model.controlsVisible)
        .animation(.snappy, value: model.activeSegment)
        .animation(.snappy, value: model.nextEpisodeOffered)
        #if os(tvOS)
        .task { scheduleHide() }
        #else
        .task { if inline == nil { await model.start() }; scheduleHide() }
        #endif
        // A title page's player moving between the header and full screen (rotating) is drawn by a
        // new view, which keeps the controls as they were and starts its own hide timer. This
        // one's would judge by its own (stale) menu and scrubbing state, so it stops here.
        .onDisappear { hideTask?.cancel() }
        .onChange(of: model.isPlaying) { _, playing in
            if !playing { showControls() } else { scheduleHide() }
        }
        .sheet(isPresented: $showSubtitleSearch) {
            SubtitleSearchView(model: model)
        }
        .sheet(isPresented: $showEpisodes) { EpisodePickerView(model: model) }
        #if DEBUG
        // Test seam: MOONRISE_OPEN_EPISODES=<seconds> opens the Episodes sheet that long after the player shows;
        // MOONRISE_OPEN_SUBSEARCH=<seconds> Find Subtitles.
        .task {
            if let t = DebugSeams.launch.openEpisodes {
                try? await Task.sleep(for: .seconds(t))
                showEpisodes = true
            }
            if let t = DebugSeams.launch.openSubSearch {
                try? await Task.sleep(for: .seconds(t))
                showSubtitleSearch = true
            }
        }
        #endif
        .onChange(of: showSubtitleSearch || showEpisodes) { _, open in inline?.onModal(open) }
        // On a title page the page wires episode switching (and starts and closes the model).
        .onAppear { if inline == nil { wireEpisodeSwitching() } }
        #if os(iOS)
        .statusBarHidden(inline.map { $0.fullScreen && !model.controlsVisible } ?? !model.controlsVisible)
        .persistentSystemOverlays(inlineCompact ? .automatic : .hidden)
        // Don't let the top-edge system swipe swallow taps on the close button.
        .defersSystemGestures(on: inlineCompact ? [] : .top)
        .background(alignment: .topLeading) { HiddenVolumeView().frame(width: 100, height: 40).allowsHitTesting(false) }
        // A hardware keyboard (iPad): ←/→ skip (adding up, as the buttons), Space plays/pauses.
        // Not while a sheet (subtitle search) has the keys.
        .background {
            if !(showSubtitleSearch || showEpisodes) {
                Group {
                    Button("Go Backward") { model.skip(-AppSettings.shared.skipBackward); showControls() }
                        .keyboardShortcut(.leftArrow, modifiers: [])
                    Button("Go Forward") { model.skip(AppSettings.shared.skipForward); showControls() }
                        .keyboardShortcut(.rightArrow, modifiers: [])
                    Button(model.isPlaying ? "Pause" : "Play") { model.togglePlay(); showControls() }
                        .keyboardShortcut(.space, modifiers: [])
                }
                .frame(width: 0, height: 0)
                .opacity(0)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
            }
        }
        #endif
        #if os(tvOS)
        .focusable()
        .focused($focused)
        .onAppear { focused = true }
        .onPlayPauseCommand { model.togglePlay(); showControls() }
        .onMoveCommand { dir in
            switch dir {
            case .left: model.skip(-AppSettings.shared.skipBackward)
            case .right: model.skip(AppSettings.shared.skipForward)
            default: break
            }
            showControls()
        }
        .onExitCommand { model.controlsVisible ? close() : showControls() }
        #endif
        #if os(macOS)
        // The pointer moving over the video brings the controls (and the pointer) back; they go
        // again after a few idle seconds while playing, and as soon as the pointer leaves.
        .onContinuousHover { phase in
            switch phase {
            case .active:
                hovering = true
                showControls()
            case .ended:
                hovering = false
                if model.isPlaying, !pinned, !menuOpen, scrubbing == nil { model.controlsVisible = false }
            }
        }
        .onChange(of: model.controlsVisible) { _, visible in
            // Hidden until it moves again, as QuickTime and the TV app do.
            if !visible, hovering { NSCursor.setHiddenUntilMouseMoves(true) }
        }
        // The Playback menu lists Space, ←/→, ↑/↓, M and F; AppKit hands plain keys (no ⌘) to the
        // focused view first, so the player takes them here too. Escape leaves full screen (on
        // the page it does nothing: it isn't Stop).
        .onKeyPress(.escape) {
            if inlineCompact { return .ignored }
            close(); return .handled
        }
        .onKeyPress(keys: [.space, .leftArrow, .rightArrow, .upArrow, .downArrow, "f", "m"]) { press in
            // ⌘←, ⇧Space…: not the player's (arrows carry the numeric-pad and function flags).
            guard press.modifiers.isDisjoint(with: [.command, .control, .option, .shift]) else { return .ignored }
            let hub = MacPlayerHub.shared
            switch press.key {
            case .space: hub.togglePlay()
            case .leftArrow: hub.skip(false)
            case .rightArrow: hub.skip(true)
            case .upArrow: hub.changeVolume(0.1)
            case .downArrow: hub.changeVolume(-0.1)
            case "m": hub.toggleMute()
            case "f": toggleMacFullScreen()
            default: return .ignored
            }
            return .handled
        }
        .focusable()
        .focusEffectDisabled()
        .focused($keyFocus)
        .onAppear { registerWithMenu(); keyFocus = true }
        // Clicking the video (or full screen) keeps the keys on the player.
        .onChange(of: macFullScreen) { _, _ in keyFocus = true }
        .onChange(of: showSubtitleSearch || showEpisodes) { _, _ in registerWithMenu() }
        .onChange(of: inline?.fullScreen) { _, _ in registerWithMenu() }
        .onDisappear { MacPlayerHub.shared.unregister(hubID) }
        #endif
    }

    /// The controls' Play button shows buffering (a ring) while they're up: no spinner over it.
    private var spinnerInControls: Bool {
        #if os(tvOS)
        return false
        #else
        return model.controlsVisible
        #endif
    }

    /// In a title page's header (not full screen).
    private var inlineCompact: Bool { inline.map { !$0.fullScreen } ?? false }

    /// The full-screen button's action: a title page's player switches between the page and full
    /// screen.
    var fullScreenAction: (() -> Void)? { inline?.toggleFullScreen }

    /// The player fills the screen (Mac: the window is in full screen with only the player in it).
    var macFullScreen: Bool {
        #if os(macOS)
        return inline?.fullScreen ?? MacWindow.shared.isFullScreen
        #else
        return !inlineCompact
        #endif
    }

    /// A title page's player, full screen on iPad: swiping down from the top goes back to the page.
    private var padFullScreenOnPage: Bool {
        #if os(iOS)
        return UIDevice.current.userInterfaceIdiom == .pad && inline?.fullScreen == true
        #else
        return false
        #endif
    }

    /// On a title page: the video swiped up (full screen, or the details back; down: `DownwardPan`).
    private func inlineDrag(_ dy: CGFloat, ended: Bool) {
        if ended, dy < -60 { inline?.swipeUp() }
    }

    // MARK: Stage

    /// Video plus everything drawn over it: full screen, or the page's video box (`compact`).
    private func stage(size: CGSize, compact: Bool) -> some View {
        ZStack {
            Color.black
            VideoSurface(model: model, fullBleed: !compact)

            SubtitleOverlay(text: model.overlayText, height: size.height,
                            controls: controlsShown ? controlsLayout : nil,
                            liftedBottom: fullScreenControls && !compact ? max(size.height * 0.2, 175) : size.height * 0.2)

            #if os(macOS)
            // Click: play/pause; double-click: full screen (QuickTime, IINA, the TV app).
            // Two-finger swipe sideways (or a mouse's horizontal wheel): scrub.
            Color.clear
                .contentShape(Rectangle())
                .onTapGesture(count: 2) {
                    if model.autoSkipProgress != nil { model.cancelAutoSkip() }
                    toggleMacFullScreen()
                }
                .onTapGesture {
                    // During the auto-skip countdown a click only stops it (Skip stays).
                    if model.autoSkipProgress != nil { model.cancelAutoSkip() } else { model.togglePlay() }
                    showControls(); keyFocus = true
                }
                .background(ScrollScrubber { seconds, ended in scrollScrub(seconds, ended: ended) })
            #endif
            #if os(iOS)
            GestureLayer(model: model, size: size, hud: $hud,
                         onTap: videoTapped, onInteraction: scheduleHide,
                         onVerticalDrag: compact && inline != nil ? inlineDrag : nil,
                         fullBleed: !inlineCompact,
                         onSwipeDownFromTop: padFullScreenOnPage ? { inline?.toggleFullScreen() } : nil)
            #endif

            if model.isRecovering || model.streamFailure != nil {
                // The last frame, dimmed: playback isn't going on.
                Color.black.opacity(model.streamFailure != nil ? 0.6 : 0.35).allowsHitTesting(false)
            } else if model.isBuffering && model.errorMessage == nil && !spinnerInControls {
                ProgressView().controlSize(compact ? .regular : .large).tint(.white)
            }

            if controlsShown {
                PlayerControls(model: model, scrubbing: $scrubbing, onClose: close,
                               onInteraction: scheduleHide, onSearchSubtitles: { showSubtitleSearch = true },
                               compact: compact, onEpisodes: { showEpisodes = true },
                               onMenu: { open in menuOpen = open; scheduleHide() },
                               onFullScreen: fullScreenAction,
                               fullScreenActive: macFullScreen,
                               closeIcon: inline.map { $0.fullScreen ? "chevron.down" : "chevron.left" },
                               download: inline?.download)
                    .transition(.opacity)
            }

            StreamRecoveryOverlay(model: model, compact: compact, scrim: false,
                                  failureInsets: recoveryInsets(size: size, compact: compact))

            if let hud {
                let place = GestureHUD.Placement(hud, compact: compact)
                GestureHUDView(hud: hud)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: place.alignment)
                    .padding(place.insets)
                    .transition(.opacity)
            }

            #if !os(tvOS)
            let seg = model.activeSegment.flatMap { model.controlsVisible || model.segmentPromptVisible || model.autoSkipProgress != nil ? $0 : nil }
            let next = model.controlsVisible && PlayerControls.showsNext(model, segment: seg != nil)
            if seg != nil || next {
                // At the end of the controls' bottom row (by Episodes, or after the scrubber),
                // where PlayerControls leaves its room: the same place with the controls up or not.
                let inset = PlayerControls.rowInsets(compact: compact)
                CornerButtons(model: model, segment: seg, next: next, compact: compact)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomTrailing)
                    .padding(.horizontal, inset.width)
                    .padding(.vertical, inset.height)
                    .transition(.opacity)
            }
            #endif

            VStack {
                Spacer()
                if let n = model.notice {
                    Text(n)
                        .font(.footnote.weight(.medium))
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 14).padding(.vertical, 8)
                        .glassEffect(.regular, in: .capsule)
                        .onTapGesture { model.notice = nil }
                }
            }
            .padding(.bottom, size.height * 0.2)
            .foregroundStyle(.white)
            .animation(.easeInOut, value: model.notice)
        }
        // Portrait: keep the video inside its 16:9 box. Full screen: don't clip, or the video
        // stops at the safe area and leaves a bar along the home indicator.
        .clipped(antialiased: false, if: compact)
        .coordinateSpace(.named(ControlsLayout.space))
        // Kept while the controls are hidden, so the text moves with them as they come back
        // (they report their frames only once drawn); a new layout (rotation) moves it smoothly.
        .onPreferenceChange(ControlsLayoutKey.self) { layout in
            guard !layout.parts.isEmpty else { return }
            withAnimation(.easeInOut(duration: 0.2)) { controlsLayout = layout }
        }
        #if os(iOS)
        .downwardPan(inlineCompact ? inline?.dragDown : nil)
        #endif
    }

    /// Another episode replaces this one inside the same full-screen player.
    func wireEpisodeSwitching() {
        let playback = playback
        let request = model.request
        model.onPlayEpisode = { ep in
            if let next = request.episode(ep) { playback.play(next) }
        }
    }

    func close() {
        // On a title page: the header's button leaves the page; full screen's goes back to it.
        if let inline {
            inline.fullScreen ? inline.toggleFullScreen() : inline.back()
            return
        }
        model.close()
        onClose()
    }

    /// Debug seam: MOONRISE_PIN_CONTROLS=1 keeps controls up for scripted UI checks.
    private var pinned: Bool {
        #if DEBUG
        return DebugSeams.launch.pinControls
        #else
        return false
        #endif
    }

    /// A tap on the video: stops the auto-skip countdown (and brings the controls up), else shows
    /// or hides the controls.
    private func videoTapped() {
        if model.autoSkipProgress != nil {
            model.cancelAutoSkip()
            showControls()
            return
        }
        toggleControls()
    }

    /// The controls are drawn: asked for, or an error or a failed stream keeps them up.
    private var controlsShown: Bool {
        model.controlsVisible || model.errorMessage != nil || model.streamFailure != nil
    }

    private func toggleControls() {
        if pinned { return }
        model.controlsVisible ? (model.controlsVisible = false) : showControls()
    }

    /// The non-compact stage on iOS uses the TV-app layout, whose title row and buttons reach higher.
    private var fullScreenControls: Bool {
        #if os(iOS)
        return true
        #else
        return false
        #endif
    }

    /// The give-up message goes between the top bar and the bottom controls (they show with it);
    /// the full-screen iOS controls take the lower fifth or more.
    private func recoveryInsets(size: CGSize, compact: Bool) -> EdgeInsets {
        guard !compact else { return EdgeInsets() }
        let bottom = fullScreenControls ? max(size.height * 0.2, 175) : 90
        return EdgeInsets(top: 64, leading: 0, bottom: bottom, trailing: 0)
    }

    func showControls() {
        model.controlsVisible = true
        scheduleHide()
    }

    func scheduleHide() {
        hideTask?.cancel()
        hideTask = Task {
            try? await Task.sleep(for: .seconds(4))
            guard !Task.isCancelled, model.isPlaying, scrubbing == nil, !pinned, !menuOpen else { return }
            model.controlsVisible = false
        }
    }
}

// MARK: - Subtitles

struct SubtitleOverlay: View {
    let text: String?
    let height: CGFloat
    /// Apple TV's system transport bar is showing: the text's bottom edge this far up.
    var raisedBottom: CGFloat? = nil
    /// Our controls are showing, here: the text goes where they don't cover it.
    var controls: ControlsLayout? = nil
    /// The text's bottom edge with the controls up where no spot is clear of them (the old lift).
    var liftedBottom: CGFloat = 0
    /// The text box's sides and height on the stage (not how high it sits, which this view sets).
    @State private var textBox = TextBox()
    /// This overlay's frame on the stage.
    @State private var stage: CGRect = .zero
    private var settings: AppSettings { AppSettings.shared }

    private var baseSize: CGFloat {
        #if os(tvOS)
        return 46
        #elseif os(macOS)
        return 28
        #else
        return max(18, min(30, height * 0.055))
        #endif
    }

    /// The text's distance from the bottom. With the controls up, the first of these spots
    /// that no control covers: (a) where it is with them hidden; (b) just above the bottom
    /// block (title, track buttons, scrubber, Episodes), taken as one full-width band so the
    /// text never sits inside it; (c) just above the centre buttons, under the top bar. If
    /// none is clear (the page's small player, big text): the old fixed lift, (b) or (c),
    /// whichever hides the least of the letters.
    private var bottomPadding: CGFloat {
        Self.bottomPadding(height: height, baseSize: baseSize, raisedBottom: raisedBottom, controls: controls,
                           liftedBottom: liftedBottom, textBox: textBox, stage: stage,
                           background: settings.subtitleBackground, sizeScale: settings.subtitleSize.scale)
    }

    /// `bottomPadding` for the overlay's state (a function of it alone, for MoonriseTests): the
    /// picture's `height`, the text's base size, the overlay's frame on the stage (`stage`) and
    /// the text box's, the controls' parts, and the subtitle settings (background, size scale).
    static func bottomPadding(height: CGFloat, baseSize: CGFloat, raisedBottom: CGFloat?, controls: ControlsLayout?,
                              liftedBottom: CGFloat, textBox: TextBox, stage: CGRect,
                              background: Bool, sizeScale: CGFloat) -> CGFloat {
        // Controls hidden: 6% of the picture's height, but at least the text's base size, so it
        // doesn't sit on the bottom edge of a small player (the phone's on a title page). Large
        // pictures (full screen, the iPad's, the TV) keep their 6%.
        let normal = max(height * 0.06, baseSize)
        if let raisedBottom { return raisedBottom }
        guard let controls, textBox.height > 0, stage.height > 0 else { return normal }
        let th = textBox.height
        let bandTop = controls.parts.filter { $0.kind == .bottom }.map(\.frame.minY).min() ?? stage.maxY
        let centreTop = controls.parts.filter { $0.kind == .centre && $0.overlaps(textBox) }.map(\.frame.minY).min()
        /// The text's box with its bottom edge at `bottom` (stage coordinates, y down).
        func box(_ bottom: CGFloat) -> CGRect {
            CGRect(x: textBox.minX, y: bottom - th, width: textBox.maxX - textBox.minX, height: th)
        }
        func clear(_ bottom: CGFloat) -> Bool {
            let r = box(bottom)
            return r.minY >= stage.minY && r.maxY <= bandTop
                && !controls.parts.contains { $0.kind != .bottom && $0.hides(r) }
        }
        var spots = [stage.maxY - normal, bandTop - 8, bandTop - 4]
        if let centreTop { spots += [centreTop - 8, centreTop - 4] }
        if let spot = spots.first(where: clear) { return stage.maxY - spot }

        // Nothing is clear: of the old fixed lift and spots (b) and (c), the one where controls
        // cover the least of the lines' letters (the middle of each line), measured across.
        let pad: CGFloat = background ? 4 : 0
        let lineHeight = baseSize * sizeScale * 1.2
        let lines = max(1, Int(((th - 2 * pad) / lineHeight).rounded()))
        let step = (th - 2 * pad) / CGFloat(lines)
        func hidden(_ bottom: CGFloat) -> CGFloat {
            (0..<lines).reduce(0) { sum, i in
                let top = bottom - th + pad + CGFloat(i) * step
                let letters = CGRect(x: textBox.minX, y: top + step * 0.3,
                                     width: textBox.maxX - textBox.minX, height: step * 0.4)
                return sum + controls.parts.filter { $0.hides(letters) }
                    .reduce(0) { $0 + $1.frame.intersection(letters).width }
            }
        }
        var fallback = [stage.maxY - liftedBottom, bandTop - 8]
        if let centreTop { fallback.append(centreTop - 4) }
        fallback = fallback.map { max($0, stage.minY + th) }
        // The old lift first, and only a spot that hides less replaces it: never worse than before.
        let best = fallback.dropFirst().reduce(fallback[0]) { hidden($1) < hidden($0) - 1 ? $1 : $0 }
        return stage.maxY - best
    }

    var body: some View {
        VStack {
            Spacer()
            if let text, !text.isEmpty {
                Text(text)
                    .font(.system(size: baseSize * settings.subtitleSize.scale, weight: .semibold))
                    .multilineTextAlignment(.center)
                    .foregroundStyle(.white)
                    .shadow(color: .black, radius: 0, x: 1, y: 1)
                    .shadow(color: .black, radius: 0, x: -1, y: -1)
                    .shadow(color: .black.opacity(0.8), radius: 3)
                    .padding(.horizontal, settings.subtitleBackground ? 10 : 0)
                    .padding(.vertical, settings.subtitleBackground ? 4 : 0)
                    .background(settings.subtitleBackground ? Color.black.opacity(0.6) : .clear, in: RoundedRectangle(cornerRadius: 6))
                    .onGeometryChange(for: TextBox.self) { geo in
                        let f = geo.frame(in: .named(ControlsLayout.space))
                        return TextBox(minX: f.minX, maxX: f.maxX, height: f.height)
                    } action: { textBox = $0 }
                    .padding(.horizontal, 40)
            }
        }
        .padding(.bottom, bottomPadding)
        .frame(maxWidth: .infinity, maxHeight: .infinity) // centred even inside a GeometryReader
        .onGeometryChange(for: CGRect.self) { $0.frame(in: .named(ControlsLayout.space)) } action: { stage = $0 }
        .animation(.easeInOut(duration: 0.2), value: controls != nil || raisedBottom != nil)
        .allowsHitTesting(false)
    }
}

/// Where PlayerControls' buttons, rows and titles are drawn on the stage (coordinate space
/// `space`), so the subtitles can stay clear of them.
struct ControlsLayout: Equatable {
    static let space = "playerStage"
    struct Part: Equatable {
        enum Kind { case top, centre, bottom }
        var kind: Kind
        var frame: CGRect
        /// A circle (the centre buttons): its corners don't hide anything.
        var round = false

        func overlaps(_ box: TextBox) -> Bool { frame.maxX > box.minX && frame.minX < box.maxX }

        func hides(_ r: CGRect) -> Bool {
            guard round else { return frame.intersects(r) }
            let x = min(max(frame.midX, r.minX), r.maxX), y = min(max(frame.midY, r.minY), r.maxY)
            let radius = frame.width / 2
            return (x - frame.midX) * (x - frame.midX) + (y - frame.midY) * (y - frame.midY) < radius * radius
        }
    }
    var parts: [Part] = []
}

/// The subtitle box's sides and height on the stage.
struct TextBox: Equatable, Sendable {
    var minX: CGFloat = 0
    var maxX: CGFloat = 0
    var height: CGFloat = 0
}

struct ControlsLayoutKey: PreferenceKey {
    static let defaultValue = ControlsLayout()
    static func reduce(value: inout ControlsLayout, nextValue: () -> ControlsLayout) {
        value.parts += nextValue().parts
    }
}

extension View {
    /// Reports this part of the controls' frame on the stage (`ControlsLayoutKey`): the
    /// subtitles keep out from under it.
    func controlsOccupied(_ kind: ControlsLayout.Part.Kind, round: Bool = false) -> some View {
        background {
            GeometryReader { geo in
                Color.clear.preference(key: ControlsLayoutKey.self, value: ControlsLayout(parts: [
                    .init(kind: kind, frame: geo.frame(in: .named(ControlsLayout.space)), round: round)
                ]))
            }
        }
    }
}

/// A lost stream being recovered (a spinner, "Reconnecting…" after a few seconds), or given
/// up on (what happened, and Try Again). Centred on the video; the controls stay usable.
struct StreamRecoveryOverlay: View {
    let model: PlayerModel
    var compact: Bool
    /// Draw the dimming too (the Apple TV's system player has no stage of ours to do it).
    var scrim = true
    /// Room the controls take around the give-up message.
    var failureInsets = EdgeInsets()
    @State private var slow = false
    /// The give-up's Details, showing.
    @State private var detail: String?

    var body: some View {
        ZStack {
            if scrim, model.isRecovering || model.streamFailure != nil {
                Color.black.opacity(model.streamFailure != nil ? 0.6 : 0.35).ignoresSafeArea().allowsHitTesting(false)
            }
            if let failure = model.streamFailure {
                GeometryReader { geo in
                    // Too little room (a phone's header, or between the controls in landscape): the small layout.
                    let room = geo.size.height - failureInsets.top - failureInsets.bottom
                    failed(failure, small: room < 230)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .padding(failureInsets)
                }
            } else if model.isRecovering {
                VStack(spacing: compact ? 8 : 14) {
                    ProgressView().controlSize(compact ? .regular : .large).tint(.white)
                    Text("Reconnecting…")
                        .font(compact ? .footnote.weight(.medium) : .callout.weight(.medium))
                        .foregroundStyle(.white.opacity(0.85))
                        .opacity(slow ? 1 : 0)
                }
                .shadow(color: .black.opacity(0.5), radius: 6)
                .allowsHitTesting(false)
                // Quick recoveries (a restarted server's new session) show the spinner only.
                .task {
                    slow = false
                    try? await Task.sleep(for: .seconds(3))
                    withAnimation(.easeIn(duration: 0.3)) { slow = true }
                }
            }
        }
        .animation(.easeInOut(duration: 0.2), value: model.recovery)
        .alert(model.streamFailure?.title ?? "Details", isPresented: Binding(get: { detail != nil }, set: { if !$0 { detail = nil } })) {
            Button("OK") {}
        } message: {
            Text(detail ?? "")
        }
    }

    private func failed(_ f: PlayerModel.StreamFailure, small compact: Bool) -> some View {
        VStack(spacing: compact ? 6 : 12) {
            if !compact {
                Image(systemName: "wifi.exclamationmark")
                    .font(.system(size: tv ? 64 : 40, weight: .medium))
                    .padding(.bottom, 4)
            }
            Text(f.title)
                .font(compact ? .subheadline.weight(.semibold) : .title3.weight(.semibold))
            Text(f.detail)
                .font(compact ? .caption : tv ? .body : .subheadline)
                .foregroundStyle(.white.opacity(0.75))
                .lineLimit(compact ? 3 : 4)
                .minimumScaleFactor(0.85)
            Group {
            #if os(tvOS)
            // The system player keeps the focus: Play (or Select) on the remote tries again.
            Label("Press Play to try again", systemImage: "playpause.fill")
                .font(.body.weight(.semibold))
                .foregroundStyle(.white.opacity(0.9))
            #else
            HStack(spacing: 10) {
            Button { model.retryStream() } label: {
                Label("Try Again", systemImage: "arrow.clockwise")
                    .font(compact ? .footnote.weight(.semibold) : .body.weight(.semibold))
                    .foregroundStyle(.black)
                    .padding(.horizontal, compact ? 16 : 24)
                    .frame(minHeight: compact ? 32 : 44)
                    .background(.white, in: Capsule())
                    .contentShape(Capsule())
            }
            .plainButton()
            if let more = f.more {
                // The server's own words.
                Button { detail = more } label: {
                    Text("Details")
                        .font(compact ? .footnote.weight(.semibold) : .body.weight(.semibold))
                        .foregroundStyle(.white)
                        .padding(.horizontal, compact ? 14 : 20)
                        .frame(minHeight: compact ? 32 : 44)
                        .background(.white.opacity(0.18), in: Capsule())
                        .contentShape(Capsule())
                }
                .plainButton()
            }
            }
            #endif
            }
            .padding(.top, compact ? 4 : 8)
        }
        .multilineTextAlignment(.center)
        .foregroundStyle(.white)
        .frame(maxWidth: tv ? 720 : 340)
        .padding(.horizontal, 24)
        .shadow(color: .black.opacity(0.4), radius: 8)
        .transition(.opacity)
    }

    private var tv: Bool {
        #if os(tvOS)
        return true
        #else
        return false
        #endif
    }
}

/// Why playback couldn't start: a short title and one plain line, OK, and Details for the
/// server's own text when there is any. In a small player (the phone's portrait header)
/// it drops the symbol and tightens, so the line is never cut.
struct ErrorBanner: View {
    var title: String? = nil
    let message: String
    var detail: String? = nil
    let dismiss: () -> Void
    @State private var showDetail = false

    var body: some View {
        GeometryReader { geo in
            let small = geo.size.height < 300
            content(small: small)
                .frame(maxWidth: small ? 360 : 460)
                .padding(small ? 12 : 24)
                .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 16))
                .padding(small ? 8 : 40)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .alert(title ?? "Details", isPresented: $showDetail) {
            Button("OK") {}
        } message: {
            Text(detail ?? "")
        }
    }

    private func content(small: Bool) -> some View {
        VStack(spacing: small ? 6 : 12) {
            if !small {
                Image(systemName: "exclamationmark.triangle.fill").font(.largeTitle).foregroundStyle(Theme.accent)
            }
            if let title {
                Text(title).font(small ? .subheadline.weight(.semibold) : .title3.weight(.semibold))
            }
            Text(message)
                .font(small ? .caption : .body)
                .foregroundStyle(title == nil ? .white : .white.opacity(0.8))
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 12) {
                #if os(tvOS)
                Button("OK", action: dismiss).tvGlass()
                if detail != nil { Button("Details") { showDetail = true }.tvGlass() }
                #else
                Button("OK", action: dismiss).buttonStyle(.glass).controlSize(small ? .small : .regular)
                if detail != nil {
                    Button("Details") { showDetail = true }.buttonStyle(.glass).controlSize(small ? .small : .regular)
                }
                #endif
            }
            .padding(.top, small ? 2 : 0)
        }
        .multilineTextAlignment(.center)
        .foregroundStyle(.white)
    }
}

// MARK: - Gesture HUD

enum GestureHUD: Equatable {
    case brightness(Double)
    case volume(Double)
    case skip(Double)

    /// Brightness or volume, not a skip.
    var isLevel: Bool {
        if case .skip = self { return false }
        return true
    }

    /// Where the HUD sits on the stage (inside its safe area, as the controls are).
    struct Placement: Equatable {
        var alignment: Alignment
        var insets: EdgeInsets

        init(_ hud: GestureHUD, compact: Bool) {
            #if os(macOS)
            self.init(hud, compact: compact, mac: true)
            #else
            self.init(hud, compact: compact, mac: false)
            #endif
        }

        /// - A skip: in the middle, where the double-tap was aimed.
        /// - Mac (the keys' volume): under the top bar, whose right end holds buttons, clear of
        ///   the transport in the middle.
        /// - iPhone/iPad (the swipes' brightness and volume): the top row's right end, off the
        ///   picture. The full-screen top row has its buttons on the left, so it's free; the pill is
        ///   as tall as the row and keeps the bottom row's side inset.
        init(_ hud: GestureHUD, compact: Bool, mac: Bool) {
            guard hud.isLevel else {
                self.init(alignment: .center, insets: EdgeInsets())
                return
            }
            if mac {
                self.init(alignment: .top, insets: EdgeInsets(top: compact ? 64 : 76, leading: 0, bottom: 0, trailing: 0))
                return
            }
            let row = PlayerControls.rowInsets(compact: compact)
            self.init(alignment: .topTrailing,
                      insets: EdgeInsets(top: row.height, leading: row.width, bottom: 0, trailing: row.width))
        }

        init(alignment: Alignment, insets: EdgeInsets) {
            self.alignment = alignment
            self.insets = insets
        }
    }
}

struct GestureHUDView: View {
    let hud: GestureHUD
    var body: some View {
        Group {
            switch hud {
            case let .brightness(v): level(icon: v < 0.4 ? "sun.min.fill" : "sun.max.fill", value: v)
            case let .volume(v): level(icon: v == 0 ? "speaker.slash.fill" : v < 0.5 ? "speaker.wave.1.fill" : "speaker.wave.3.fill", value: v)
            case let .skip(d):
                Label(d > 0 ? "\(Int(d))s" : "\(Int(-d))s", systemImage: d > 0 ? "goforward" : "gobackward")
                    .font(.title2.weight(.semibold))
                    .padding(.horizontal, 22).padding(.vertical, 14)
                    .background(.ultraThinMaterial, in: Capsule())
            }
        }
        .foregroundStyle(.white)
        .allowsHitTesting(false)
    }

    /// A compact pill as tall as the top bar's buttons.
    private func level(icon: String, value: Double) -> some View {
        HStack(spacing: 10) {
            Image(systemName: icon)
                .font(.system(size: 16, weight: .medium))
                .frame(width: 22)
            GeometryReader { g in
                ZStack(alignment: .leading) {
                    Capsule().fill(Color.white.opacity(0.25))
                    Capsule().fill(Color.white).frame(width: g.size.width * value)
                }
            }
            .frame(width: 110, height: 5)
        }
        .padding(.horizontal, 16)
        .frame(height: PlayerControls.barButtonSize)
        .glassEffect(.regular, in: .capsule)
    }
}

private extension View {
    @ViewBuilder func clipped(antialiased: Bool, if condition: Bool) -> some View {
        if condition { clipped(antialiased: antialiased) } else { self }
    }

    #if os(iOS)
    /// On a title page: the video dragged down (see `DownwardPan`).
    @ViewBuilder func downwardPan(_ action: ((CGFloat, CGFloat, Bool) -> Void)?) -> some View {
        if let action { gesture(DownwardPan(onChange: action)) } else { self }
    }
    #endif
}
