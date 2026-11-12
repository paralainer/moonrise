import SwiftUI

/// Overlay in Liquid Glass: close + title and track/quality/output buttons on top,
/// transport in the middle, a minimal time + scrubber bar at the bottom.
struct PlayerControls: View {
    let model: PlayerModel
    @Binding var scrubbing: Double?
    let onClose: () -> Void
    let onInteraction: () -> Void
    let onSearchSubtitles: () -> Void
    /// A title page's player in the page (not full screen): smaller controls, no title (the page
    /// shows it), chevron to close, and a full-screen button.
    var compact = false
    /// Opens the episode picker (shows only).
    var onEpisodes: (() -> Void)? = nil
    /// A menu opened (true) or closed (false): the controls stay up while one is open.
    var onMenu: (Bool) -> Void = { _ in }
    /// A title page's player: enters/leaves full screen (instead of rotating the phone).
    var onFullScreen: (() -> Void)? = nil
    /// Mac: the player is full screen now (the button's symbol and name).
    var fullScreenActive = false
    /// Overrides the close button's symbol.
    var closeIcon: String? = nil
    /// iPhone, a title page's player: Download as one more button with the track buttons.
    var download: InlineDownload? = nil

    private var s: AppSettings { AppSettings.shared }

    #if os(tvOS)
    let iconSize: CGFloat = 34
    private let playSize: CGFloat = 64
    #else
    var iconSize: CGFloat { compact ? 18 : 22 }
    private var playSize: CGFloat { compact ? 30 : 46 }
    #endif
    @Environment(\.metrics) private var metrics
    /// The page's small player on the phone; on iPad its video is big enough for full-size buttons.
    private var smallTransport: Bool { compact && !metrics.regular }
    #if os(tvOS)
    private var transportIcon: CGFloat { iconSize }
    private var transportPlay: CGFloat { playSize }
    #else
    private var transportIcon: CGFloat { smallTransport ? 18 : 22 }
    private var transportPlay: CGFloat { smallTransport ? 30 : 46 }
    #endif

    var body: some View {
        ZStack {
            LinearGradient(colors: [.black.opacity(0.45), .clear, .clear, .black.opacity(0.5)],
                           startPoint: .top, endPoint: .bottom)
                .ignoresSafeArea()
                .allowsHitTesting(false)

            #if os(iOS)
            if !compact {
                fullScreenLayout
                    .padding(.horizontal, Self.rowInsets(compact: false).width)
                    .padding(.vertical, Self.rowInsets(compact: false).height)
                // In the middle of the screen, regardless of the bars' heights and the home
                // indicator's inset.
                transport
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .ignoresSafeArea()
            } else {
                standardLayout
            }
            #else
            standardLayout
            #endif
        }
        .foregroundStyle(.white)
    }

    private var standardLayout: some View {
            VStack(spacing: 0) {
                topBar
                Spacer()
                #if os(tvOS)
                // The remote's buttons play/pause and skip; just show the state.
                if !model.isPlaying && !model.isBuffering && !model.isRecovering && model.streamFailure == nil {
                    Image(systemName: "pause.fill").font(.system(size: 70)).shadow(radius: 12)
                        .controlsOccupied(.centre)
                }
                #else
                transport
                #endif
                Spacer()
                HStack(spacing: Self.rowSpacing) {
                    bottomBar
                    skipPlaceholder
                }
                .controlsOccupied(.bottom)
            }
            .padding(.horizontal, Self.rowInsets(compact: compact).width)
            .padding(.vertical, Self.rowInsets(compact: compact).height)
    }

    /// Hit/focus size of the top-bar buttons (TV: big enough to read from the sofa).
    var buttonSize: CGFloat { Self.barButtonSize }

    /// The top-bar buttons' size, so the top row's height: the gesture HUD lines up with it.
    static var barButtonSize: CGFloat {
        #if os(tvOS)
        return 76
        #else
        return 44
        #endif
    }

    /// The controls' distance from the stage's sides (width) and bottom (height): where the
    /// bottom row, and Skip Intro at its end, sit.
    static func rowInsets(compact: Bool) -> CGSize {
        #if os(tvOS)
        return CGSize(width: 80, height: 12)
        #else
        return CGSize(width: compact ? 10 : 20, height: compact ? 8 : 12)
        #endif
    }

    /// Between the scrubber capsule and Skip Intro.
    static let rowSpacing: CGFloat = 10

    /// Skip Intro's place in the bottom row: PlayerScreen draws the button itself over it (it
    /// shows while the controls are hidden too), so here it only takes up the room.
    @ViewBuilder var skipPlaceholder: some View {
        #if !os(tvOS) // the Apple TV's system player has its own (TVNativePlayer)
        if model.activeSegment != nil || Self.showsNext(model, segment: model.activeSegment != nil) {
            CornerButtons(model: model, segment: model.activeSegment, next: Self.showsNext(model, segment: model.activeSegment != nil), compact: compact)
                .hidden()
                .accessibilityHidden(true)
        }
        #endif
    }

    /// Next Episode by Skip Intro, unless Skip (end credits to the end) already plays it.
    static func showsNext(_ model: PlayerModel, segment shown: Bool) -> Bool {
        model.nextEpisodeOffered && !(shown && model.creditsSkipPlaysNext)
    }

    // MARK: Top

    #if !os(tvOS)
    var closeButton: some View {
        Button(action: onClose) {
            Image(systemName: closeIcon ?? (compact ? "chevron.down" : "xmark"))
                .font(.system(size: iconSize - 4, weight: .semibold))
                .frame(width: buttonSize, height: buttonSize)
                .glassEffect(.regular.interactive(), in: .circle)
                .padding(8)
                .contentShape(Rectangle()) // generous hit area around the circle, not just the glyph
        }
        .plainButton()
        .padding(-8)
        .keyboardShortcut(.cancelAction)
        .accessibilityLabel(closeIcon == "chevron.left" ? "Back" : "Close")
    }
    #endif

    private var topBar: some View {
        HStack(alignment: .center, spacing: 14) {
            #if os(macOS)
            // A title page's player: the window's toolbar keeps the page's back button, and full
            // screen ends with F, Escape, a double-click or the button by the scrubber. Only the
            // Downloads player (no page under it) has a close button.
            if closeIcon == nil { closeButton.controlsOccupied(.top) }
            #elseif !os(tvOS)
            closeButton.controlsOccupied(.top)
            #endif

            if !compact {
                VStack(alignment: .leading, spacing: 2) {
                    Text(model.title).font(.headline).lineLimit(1)
                    if let sub = model.subtitle, !sub.isEmpty {
                        Text(sub).font(.caption).foregroundStyle(.white.opacity(0.7)).lineLimit(1)
                    }
                }
                .shadow(color: .black.opacity(0.5), radius: 6)
                .controlsOccupied(.top)
            }

            Spacer()

            HStack(spacing: 4) {
                if let onEpisodes, !model.seriesEpisodes.isEmpty, !compact {
                    Button { onEpisodes(); onInteraction() } label: {
                        Image(systemName: "rectangle.stack")
                            .frame(width: buttonSize, height: buttonSize)
                            .contentShape(Rectangle())
                    }
                    .plainButton()
                    .accessibilityLabel("Episodes")
                }
                if model.qualityAvailable {
                    #if os(iOS)
                    qualityButton
                    #else
                    qualityMenu.frame(width: buttonSize, height: buttonSize)
                    #endif
                }
                trackButtons
                downloadButton
                #if os(macOS)
                if model.engine?.supportsPictureInPicture == true {
                    Button { model.engine?.togglePictureInPicture(); onInteraction() } label: {
                        Image(systemName: model.isInPictureInPicture ? "pip.exit" : "pip.enter")
                            .frame(width: buttonSize, height: buttonSize)
                            .contentShape(Rectangle())
                    }
                    .plainButton()
                    .help(model.isInPictureInPicture ? "Exit Picture in Picture" : "Picture in Picture")
                    .accessibilityLabel("Picture in Picture")
                }
                MacRoutePicker(player: (model.engine as? AVPlayerEngine)?.player)
                    .frame(width: buttonSize, height: buttonSize)
                    .help("AirPlay")
                    .accessibilityLabel("AirPlay")
                #endif
                if !compact {
                    Button { model.fill.toggle() } label: {
                        Image(systemName: model.fill ? "rectangle.arrowtriangle.2.inward" : "rectangle.arrowtriangle.2.outward")
                            .frame(width: buttonSize, height: buttonSize)
                            .contentShape(Rectangle())
                    }
                    .plainButton()
                    .accessibilityLabel(model.fill ? "Fit" : "Fill")
                }
            }
            .font(.system(size: iconSize - 4, weight: .medium))
            .padding(.horizontal, 6)
            .glassEffect(.regular.interactive(), in: .capsule)
            .controlsOccupied(.top)
        }
    }

    // MARK: Middle

    private var transport: some View {
        GlassEffectContainer(spacing: smallTransport ? 28 : 40) {
            HStack(spacing: smallTransport ? 28 : 40) {
                Button { model.skip(-s.skipBackward); onInteraction() } label: {
                    Image(systemName: "gobackward.\(Int(s.skipBackward))")
                        .font(.system(size: transportIcon + 2, weight: .medium))
                        .frame(width: transportPlay * 1.3, height: transportPlay * 1.3)
                        .contentShape(Circle())
                }
                .glassEffect(.regular.interactive(), in: .circle)
                .controlsOccupied(.centre, round: true)
                .overlay(alignment: .bottom) { skipTotal(forward: false) }
                .accessibilityLabel("Go Backward \(Int(s.skipBackward)) Seconds")

                Button { model.togglePlay(); onInteraction() } label: {
                    Image(systemName: model.isFinished ? "arrow.counterclockwise" : model.isPlaying ? "pause.fill" : "play.fill")
                        .font(.system(size: transportPlay * 0.75))
                        .frame(width: transportPlay * 1.75, height: transportPlay * 1.75)
                        .contentShape(Circle())
                }
                .glassEffect(.regular.interactive(), in: .circle)
                .controlsOccupied(.centre, round: true)
                // Buffering (a seek, a slow stream): a ring around Play, the buttons stay put.
                .overlay {
                    if model.isBuffering && model.streamFailure == nil {
                        BufferingRing().padding(-4).allowsHitTesting(false)
                    }
                }
                .accessibilityLabel(model.isPlaying ? "Pause" : "Play")

                Button { model.skip(s.skipForward); onInteraction() } label: {
                    Image(systemName: "goforward.\(Int(s.skipForward))")
                        .font(.system(size: transportIcon + 2, weight: .medium))
                        .frame(width: transportPlay * 1.3, height: transportPlay * 1.3)
                        .contentShape(Circle())
                }
                .glassEffect(.regular.interactive(), in: .circle)
                .controlsOccupied(.centre, round: true)
                .overlay(alignment: .bottom) { skipTotal(forward: true) }
                .accessibilityLabel("Go Forward \(Int(s.skipForward)) Seconds")
            }
        }
        .plainButton()
        // Given up: Try Again's place. (Buffering keeps the buttons: the ring shows it.)
        .opacity(model.streamFailure != nil ? 0 : 1)
        .animation(.easeOut(duration: 0.15), value: model.skipTotal)
    }

    /// Skips pressed in a row add up: their total under the button, until the seek.
    @ViewBuilder private func skipTotal(forward: Bool) -> some View {
        if let total = model.skipTotal, total != 0, (total > 0) == forward {
            Text(String(format: "%+.0f s", total))
                .font((smallTransport ? Font.caption2 : .caption).weight(.semibold).monospacedDigit())
                .foregroundStyle(.white)
                .padding(.horizontal, 7).padding(.vertical, 3)
                .background(.black.opacity(0.45), in: Capsule())
                .fixedSize()
                .offset(y: smallTransport ? 22 : 26)
                .transition(.opacity)
                .allowsHitTesting(false)
        }
    }

    // MARK: Bottom

    private var bottomBar: some View {
        HStack(spacing: 14) {
            Text(Fmt.duration(scrubbing ?? model.shownTime))
            Scrubber(current: scrubbing ?? model.shownTime, duration: model.duration,
                     onScrub: { scrubbing = $0; onInteraction() },
                     onCommit: { model.seek(to: $0); scrubbing = nil; onInteraction() },
                     thumbnails: model.thumbnails)
            // The phone's small player has no room for it beside Skip Intro.
            if !(smallTransport && (model.activeSegment != nil || model.nextEpisodeOffered)) {
                Text("−" + Fmt.duration(max(0, model.duration - (scrubbing ?? model.shownTime))))
            }
            #if os(iOS)
            RoutePickerButton()
                .frame(width: 28, height: 28)
                .accessibilityLabel("AirPlay")
            if onFullScreen != nil || UIDevice.current.userInterfaceIdiom == .phone { rotateButton }
            #elseif os(macOS)
            if let onFullScreen {
                Button { onFullScreen(); onInteraction() } label: {
                    Image(systemName: fullScreenActive ? "arrow.down.right.and.arrow.up.left" : "arrow.up.left.and.arrow.down.right")
                        .frame(width: 28, height: 28)
                        .contentShape(Rectangle())
                }
                .plainButton()
                .help(fullScreenActive ? "Exit Full Screen (F)" : "Full Screen (F)")
                .accessibilityLabel(fullScreenActive ? "Exit Full Screen" : "Full Screen")
            }
            #endif
        }
        .font(.footnote.monospacedDigit().weight(.medium))
        .padding(.horizontal, compact ? 12 : 18)
        .padding(.vertical, 6)
        .glassEffect(.regular, in: .capsule)
    }

    /// The TV app's layout: separate Audio and Subtitles buttons, each a menu with the
    /// language on the first line and track details (SDH, 5.1…) on the second.
    @ViewBuilder var downloadButton: some View {
        #if os(iOS)
        if let download, DownloadManager.isSupported {
            DownloadButton(item: download.item, source: download.source, size: buttonSize,
                           glyph: .system(size: iconSize - 4, weight: .medium), ringInset: 0.24,
                           onMenu: onMenu)
                .frame(width: buttonSize, height: buttonSize)
        }
        #endif
    }

    @ViewBuilder var trackButtons: some View {
        #if os(iOS)
        if model.audioOptions.count > 1 {
            MenuButton(systemImage: "waveform", pointSize: iconSize - 4, accessibilityLabel: "Audio",
                       menu: { [model] in TrackMenus.audio(model) }, onMenu: onMenu)
                .frame(width: buttonSize, height: buttonSize)
        }
        // The menu is built when this view updates: it says how the track's timing stands.
        let _ = model.subtitleTimingKey
        MenuButton(systemImage: model.selectedSubtitleId == nil ? "captions.bubble" : "captions.bubble.fill",
                   pointSize: iconSize - 4, accessibilityLabel: "Subtitles",
                   menu: { [model, onSearchSubtitles] in
                       var extras: [UIMenuElement] = [TrackMenus.delay(model)]
                       if model.canSearchSubtitles {
                           extras.append(UIAction(title: "Search Online…", image: UIImage(systemName: "magnifyingglass")) { _ in onSearchSubtitles() })
                       }
                       return TrackMenus.subtitles(model, extras: extras)
                   },
                   onMenu: onMenu)
            .frame(width: buttonSize, height: buttonSize)
        #else
        if model.audioOptions.count > 1 { audioMenu }
        subtitleMenu
        #endif
    }

    private var subtitleMenu: some View {
        Menu {
            Section("Subtitles") {
                menuTracker
                trackRow("Off", nil, model.selectedSubtitleId == nil) { model.chooseSubtitle(nil) }
                ForEach(model.subtitleOptions) { o in
                    trackRow(o.name, model.subtitleDetail(o), model.selectedSubtitleId == o.id) { model.chooseSubtitle(o) }
                }
            }
            Section {
                Menu {
                    Picker("Size", selection: Binding(get: { s.subtitleSize }, set: { s.subtitleSize = $0 })) {
                        ForEach(SubtitleSize.allCases) { Text($0.rawValue.capitalized).tag($0) }
                    }
                    .pickerStyle(.inline)
                    Toggle("Background", isOn: Binding(get: { s.subtitleBackground }, set: { s.subtitleBackground = $0 }))
                } label: {
                    Label("Style", systemImage: "textformat.size")
                }
                Menu {
                    ForEach([-2.0, -1.0, -0.5, -0.1, 0.1, 0.5, 1.0, 2.0], id: \.self) { d in
                        Button(String(format: "%+.1fs", d)) { model.nudgeSubtitles(by: d) }
                    }
                    Button("Reset") { model.resetSubtitleDelay() }
                    if model.subtitleSyncApplied {
                        Toggle("Use Original Timing", isOn: Binding(get: { model.subtitleOriginalTiming }, set: { model.useOriginalSubtitleTiming($0) }))
                    }
                } label: {
                    Label("Delay", systemImage: "clock.arrow.2.circlepath")
                    Text(String(format: "%+.1fs", model.subtitleDelay))
                }
                if model.canSearchSubtitles {
                    Button(action: onSearchSubtitles) {
                        Label("Search Online…", systemImage: "magnifyingglass")
                    }
                }
            }
        } label: {
            trackIcon(model.selectedSubtitleId == nil ? "captions.bubble" : "captions.bubble.fill")
        }
        .accessibilityLabel("Subtitles")
        .menuStyle(.button)
        .plainButton()
        .drawnMenu()
    }

    private var audioMenu: some View {
        Menu {
            Section("Audio") {
                menuTracker
                ForEach(model.audioOptions) { a in
                    trackRow(a.title, a.detail, model.selectedAudio == a.id) { model.chooseAudio(a.id) }
                }
            }
        } label: {
            trackIcon("waveform")
        }
        .accessibilityLabel("Audio")
        .menuStyle(.button)
        .plainButton()
        .drawnMenu()
    }

    /// Checkmarked row with the title and a secondary line, as the system menu draws toggles.
    private func trackRow(_ title: String, _ detail: String?, _ on: Bool, select: @escaping () -> Void) -> some View {
        Toggle(isOn: Binding(get: { on }, set: { _ in if !on { select() } })) {
            Text(title)
            if let detail { Text(detail) }
        }
    }

    private func trackIcon(_ name: String) -> some View {
        Image(systemName: name)
            .frame(width: buttonSize, height: buttonSize)
            .contentShape(Rectangle())
    }

    /// Menu contents exist only while the menu is shown, so their appear/disappear marks it
    /// opening and closing (SwiftUI's Menu has no presentation callback).
    private var menuTracker: some View {
        Color.clear.frame(width: 0, height: 0)
            .onAppear { onMenu(true) }
            .onDisappear { onMenu(false) }
    }

    private var qualityMenu: some View {
        Menu {
            if model.versions.count > 1 {
                // The title's versions: picking one restarts it there at the same position.
                Section("Version") {
                    ForEach(model.versions) { v in
                        Toggle(isOn: Binding(get: { model.playingVersionId == v.id }, set: { if $0 { model.chooseVersion(v.id) } })) {
                            Text(PlayerModel.versionTitle(v))
                            if let d = PlayerModel.versionDetails(v) { Text(d) }
                        }
                        .disabled(v.isBroken && model.playingVersionId != v.id)
                    }
                }
            }
            Section(model.playMethodLabel.isEmpty ? "Quality" : "Quality · \(model.playMethodLabel)") {
                menuTracker
                // Toggle rows: the system's checkmark column (as the iOS track menus), the size
                // on the second line.
                ForEach(model.qualityOptions) { q in
                    Toggle(isOn: Binding(get: { model.quality == q }, set: { if $0 { model.setQuality(q) } })) {
                        Text(q.label)
                        if let size = model.sizeLabel(q) { Text(size) }
                    }
                }
            }
        } label: {
            Image(systemName: "dial.medium")
                .frame(width: buttonSize, height: buttonSize)
                .contentShape(Rectangle())
        }
        .accessibilityLabel("Quality")
        .menuStyle(.button)
        .plainButton()
        .drawnMenu()
    }
}

/// Thin capsule scrubber with a growing knob while dragging.
struct Scrubber: View {
    let current: Double
    let duration: Double
    let onScrub: (Double) -> Void
    let onCommit: (Double) -> Void
    /// Pictures shown above the knob while dragging (`ScrubThumbnails`; nil: none).
    var thumbnails: ScrubThumbnails? = nil
    @State private var dragging = false
    /// Finger position while dragging, drawn directly so the knob tracks without waiting for the
    /// parent to re-render.
    @State private var dragValue: Double?
    /// Debug seam: MOONRISE_SCRUB_AT=<seconds> shows the scrub preview there, as if dragged.
    private static let pinnedScrub: Double? = {
        #if DEBUG
        DebugSeams.launch.scrubAt
        #else
        nil
        #endif
    }()

    var body: some View {
        GeometryReader { g in
            let value = dragValue ?? Self.pinnedScrub ?? current
            let frac = duration > 0 ? min(1, max(0, value / duration)) : 0
            ZStack(alignment: .leading) {
                Capsule().fill(Color.white.opacity(0.25)).frame(height: dragging ? 8 : 5)
                Capsule().fill(Theme.accent).frame(width: g.size.width * frac, height: dragging ? 8 : 5)
                Circle().fill(.white)
                    .frame(width: dragging ? 22 : 14, height: dragging ? 22 : 14)
                    .offset(x: g.size.width * frac - (dragging ? 11 : 7))
            }
            .frame(maxHeight: .infinity)
            .overlay(alignment: .bottomLeading) {
                if dragging || Self.pinnedScrub != nil, let thumbnails {
                    let w = ScrubPreview.width
                    // Over the knob, kept within the bar's ends.
                    let x = min(max(0, g.size.width * frac - w / 2), max(0, g.size.width - w))
                    ScrubPreview(image: thumbnails.image(at: value), aspect: thumbnails.aspect, time: value)
                        .fixedSize()
                        .offset(x: x, y: -(g.size.height + 4))
                        .transition(.opacity)
                }
            }
            .contentShape(Rectangle())
            #if !os(tvOS)
            .gesture(DragGesture(minimumDistance: 0)
                .onChanged { v in
                    dragging = true
                    let p = position(v.location.x, g.size.width)
                    dragValue = p
                    onScrub(p)
                }
                .onEnded { v in
                    dragging = false
                    let p = position(v.location.x, g.size.width)
                    onCommit(p)
                    dragValue = nil
                })
            #endif
            .animation(.easeOut(duration: 0.15), value: dragging)
            .transaction { if dragging { $0.animation = nil } } // no easing lag on the knob
        }
        .frame(height: 28)
    }

    private func position(_ x: CGFloat, _ width: CGFloat) -> Double {
        guard width > 0 else { return 0 }
        return Double(min(max(0, x / width), 1)) * duration
    }
}

/// An indeterminate ring (buffering) drawn around a round button.
struct BufferingRing: View {
    @State private var spin = false
    var body: some View {
        Circle()
            .trim(from: 0, to: 0.28)
            .stroke(Color.white.opacity(0.9), style: StrokeStyle(lineWidth: 3, lineCap: .round))
            .rotationEffect(.degrees(spin ? 360 : 0))
            .animation(.linear(duration: 0.9).repeatForever(autoreverses: false), value: spin)
            .onAppear { spin = true }
    }
}
