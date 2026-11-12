import SwiftUI

/// "Skip Intro" capsule in the player's bottom row (next to Episodes). During an auto-skip
/// countdown it fills left-to-right like a progress bar and skips when full; tap it to skip now.
/// Anything else (a tap on the video, a remote press or swipe) stops the countdown.
struct SkipSegmentButton: View {
    let model: PlayerModel
    let segment: MediaSegment
    /// The page's small player: the scrubber row's smaller type.
    var compact = false
    #if os(tvOS)
    @FocusState private var focused: Bool
    #endif

    /// As tall as the row's other buttons: Episodes and the scrubber capsule (40), the Apple TV's
    /// Info/Episodes tabs (66).
    static var height: CGFloat {
        #if os(tvOS)
        return 66
        #else
        return 40
        #endif
    }

    var body: some View {
        Button { model.skipSegment() } label: {
            Text(title)
                .font(font)
                .monospacedDigit()
                .padding(.horizontal, padding)
                .frame(height: Self.height)
                .background(alignment: .leading) {
                    if let p = model.autoSkipProgress {
                        GeometryReader { g in
                            Rectangle()
                                .fill(.white.opacity(0.35))
                                .frame(width: g.size.width * p)
                                .animation(.linear(duration: 0.25), value: p)
                        }
                    }
                }
                .clipShape(Capsule())
                .contentShape(Capsule())
                #if os(tvOS)
                // Focused: brighter glass and a lift, not the system's white platter.
                .glassEffect(.regular.tint(.white.opacity(focused ? 0.22 : 0)), in: .capsule)
                #endif
        }
        .plainButton()
        #if !os(tvOS)
        .glassEffect(.regular.interactive(), in: .capsule)
        #endif
        .foregroundStyle(.white)
        .fixedSize()
        #if os(tvOS)
        // It takes the remote as it appears: select skips; a swipe, arrow or Play/Pause stops the
        // countdown and hands the remote back to the system player (Menu leaves the player, as
        // it does with the transport bar hidden).
        .focused($focused)
        .onMoveCommand(perform: focused ? { _ in letGo() } : nil)
        .onPlayPauseCommand(perform: focused ? { letGo(); model.togglePlay() } : nil)
        .onAppear { focused = true }
        .onChange(of: counting) { _, now in if now { focused = true } }
        #endif
    }

    #if os(tvOS)
    private func letGo() {
        #if DEBUG
        fputs("MoonriseSkip: keep watching (counting=\(counting))\n", stderr)
        #endif
        focused = false
        model.dismissSegmentPrompt()
    }
    #endif

    private var counting: Bool { model.autoSkipProgress != nil }

    private var title: String {
        // Skipping credits that run to the end plays the next episode: say so.
        let label = segment.kind == .credits && model.creditsSkipPlaysNext ? NextEpisodeButton.title : segment.kind.skipLabel
        #if os(tvOS)
        // From the sofa: the seconds left as well as the fill.
        if let p = model.autoSkipProgress {
            let left = max(1, Int(((1 - p) * AppSettings.shared.autoSkipDelay).rounded(.up)))
            return "\(label) (\(left))"
        }
        #endif
        return label
    }

    var font: Font { Self.font(compact: compact) }

    static func font(compact: Bool) -> Font {
        #if os(tvOS)
        return .callout.weight(.medium)
        #else
        // The page's player: the scrubber row's type.
        return compact ? .footnote.weight(.semibold) : .headline
        #endif
    }

    var padding: CGFloat { Self.padding(compact: compact) }

    static func padding(compact: Bool) -> CGFloat {
        #if os(tvOS)
        return 28
        #else
        return compact ? 12 : 18
        #endif
    }
}

/// "Next Episode" in glass, in Skip Intro's place at the end of the controls' bottom row: offered
/// with the controls near the end of an episode (`PlayerModel.nextEpisodeOffered`), across
/// seasons too. Looks like Skip Intro. Apple TV: it takes the remote as it appears (the system
/// player can't hand focus to it); a swipe, arrow or Play/Pause gives the remote back.
struct NextEpisodeButton: View {
    static let title = "Next Episode"
    let model: PlayerModel
    var compact = false
    #if os(tvOS)
    /// The remote did something else: hide until the transport bar shows again.
    var onLetGo: () -> Void = {}
    @FocusState private var focused: Bool
    #endif

    var body: some View {
        Button { model.playNextEpisode() } label: {
            Text(Self.title)
            .font(SkipSegmentButton.font(compact: compact))
            .padding(.horizontal, SkipSegmentButton.padding(compact: compact))
            .frame(height: SkipSegmentButton.height)
            .clipShape(Capsule())
            .contentShape(Capsule())
            #if os(tvOS)
            .glassEffect(.regular.tint(.white.opacity(focused ? 0.22 : 0)), in: .capsule)
            #endif
        }
        .plainButton()
        #if !os(tvOS)
        .glassEffect(.regular.interactive(), in: .capsule)
        #endif
        .foregroundStyle(.white)
        .fixedSize()
        .accessibilityLabel(model.nextEpisode?.name.map { "Next Episode, \($0)" } ?? Self.title)
        #if os(tvOS)
        .focused($focused)
        .onMoveCommand(perform: focused ? { _ in letGo() } : nil)
        .onPlayPauseCommand(perform: focused ? { letGo(); model.togglePlay() } : nil)
        .onAppear { focused = true }
        #endif
    }

    #if os(tvOS)
    private func letGo() {
        focused = false
        onLetGo()
    }
    #endif
}

/// Skip Intro's corner of the bottom row: Next Episode and/or the segment's Skip, side by side.
struct CornerButtons: View {
    let model: PlayerModel
    let segment: MediaSegment?
    let next: Bool
    var compact = false

    var body: some View {
        HStack(spacing: PlayerControls.rowSpacing) {
            if next { NextEpisodeButton(model: model, compact: compact).transition(.opacity) }
            if let segment { SkipSegmentButton(model: model, segment: segment, compact: compact) }
        }
    }
}
