import SwiftUI

/// Drags on a title page that plays in its header: the details panel under the video follows
/// the finger down and away (and back up); the video dragged down shrinks into its episode's
/// card in the row below while the artwork header grows back, and let go far enough, playback
/// stops there. Play runs the same thing forwards: the header shrinks to the video's height while
/// the video grows out of its episode's card into its place. One number, `launch`, drives both
/// (1: the page as it is when not playing, 0: playing), so the two are exact reverses.
/// Kept apart from the page's state so a moving finger redraws only what moves, not the page.
@MainActor
@Observable
final class PageDrag {
    /// The panel's offset below its open place: 0 open, `height` collapsed.
    var panel: CGFloat = 0
    /// The panel is away (only the video shows).
    var collapsed = false
    /// The current drag brings the panel back (decided when it starts; nil: no drag).
    @ObservationIgnored var restoring: Bool?
    /// When such a drag ended (the video's own swipe-up, seen after it, isn't full screen then).
    @ObservationIgnored var restoredAt = Date.distantPast
    @ObservationIgnored private(set) var height: CGFloat = 600

    /// 0…1: the video on its way into the playing episode's card.
    var flight: CGFloat = 0
    /// Where it lands (global frame, where the card is once the page is back); nil: no card.
    @ObservationIgnored private(set) var flightTarget: CGRect?
    /// The episode cards, the video at rest and the page while playing (global frames).
    @ObservationIgnored var cardFrames: [String: CGRect] = [:]
    @ObservationIgnored var videoFrame: CGRect = .zero
    @ObservationIgnored var pageFrame: CGRect = .zero
    @ObservationIgnored var screenFrame: CGRect = .zero

    /// 1: the page as when not playing (artwork header, Play button), 0: playing (header gone,
    /// the text right under the video). Play runs it 1 → 0; dragging the video down, 0 → 1.
    var launch: CGFloat = 0
    /// How far the text moves between the two (`launch` 1 → 0), set by the page from its layout:
    /// from under the artwork header up to under the video.
    @ObservationIgnored var span: CGFloat = 0
    /// The artwork header's height on the page when not playing.
    @ObservationIgnored var headerHeight: CGFloat = 0
    /// The scroll offsets of the page when not playing and of the panel while playing.
    @ObservationIgnored var idleScroll: CGFloat = 0
    @ObservationIgnored var panelScroll: CGFloat = 0
    /// The panel's content height (how far the page can scroll once it's back).
    @ObservationIgnored var panelContentHeight: CGFloat = 0
    /// The page at `launch` 1 is scrolled this much further than the panel: Play from a scrolled
    /// page; the drag down, as far as it takes to keep the episode's card on screen (the row stays
    /// about where it is instead of being pushed off the bottom by the header).
    var shift: CGFloat = 0
    /// The panel's scroll offset when the drag down started (the page comes back scrolled as far).
    var launchScroll: CGFloat = 0
    /// The page, back, should open scrolled this far (set when the drag down finishes).
    @ObservationIgnored private(set) var restoreScroll: CGFloat = 0
    /// While playing, the title written out over the details (it goes as the header comes back)
    /// and the space of the Play button (it comes back with it): their full heights.
    @ObservationIgnored var titleRevealHeight: CGFloat = 0
    @ObservationIgnored var playRevealHeight: CGFloat = 0
    @ObservationIgnored var playRevealActive = false
    /// The status line under it, as tall as it is while playing (with Download) and on the page.
    @ObservationIgnored var swapPlayingHeight: CGFloat = 0
    @ObservationIgnored var swapIdleHeight: CGFloat = 0
    @ObservationIgnored var swapActive = false

    /// The video fading in (no card to grow out of) or out (dragged down with no card to land in).
    var videoOpacity: CGFloat = 1
    /// No card: how far the video follows the finger down, and 0…1 how far it has shrunk.
    var drop: CGFloat = 0
    var fall: CGFloat = 0
    /// The episode's still, shown in the video's place until its first frame plays.
    var launchStill: URL?
    /// The card's corner radius (the video takes it on the way in).
    static let cardRadius: CGFloat = 16
    /// The text starts a section gap below the header on the page, and as far below the video in
    /// the panel (`panelInset`), so it keeps its distance from what's above it as Play runs.
    static let panelInset = ItemDetailView.sectionGap
    /// How much further below the header the text is than below the video (none now).
    static let textGap = ItemDetailView.sectionGap - panelInset

    /// The drag down under way: what it started from.
    @ObservationIgnored private var dismissing = false
    @ObservationIgnored private var dismissDistance: CGFloat = 1
    @ObservationIgnored private var dismissFromCollapsed = false

    /// The card of `id` when it's on screen (Play from the page; below the video while playing).
    func visibleCard(_ id: String?, playing: Bool) -> CGRect? {
        guard let id, let card = cardFrames[id], !collapsed else { return nil }
        let bounds = playing ? pageFrame : screenFrame
        guard bounds.contains(CGPoint(x: card.midX, y: card.midY)), card.maxY <= bounds.maxY else { return nil }
        return !playing || card.minY >= videoFrame.maxY ? card : nil
    }

    /// How far the text moves (`span`): from `restBottom` (under the video, global) to under the
    /// artwork header (`headerHeight` tall) that starts at `top` (the screen's top, global).
    func span(top: CGFloat, restBottom: CGFloat, headerHeight: CGFloat) -> CGFloat {
        top + headerHeight + Self.textGap - restBottom
    }

    /// Play pressed: the video starts in `card` (with `still` until it plays), or fades in; the
    /// header shrinks if it was showing (`fromHeader`). Runs in `runLaunch` once laid out.
    func prepareLaunch(card: CGRect?, still: URL?, fromHeader: Bool) {
        flightTarget = card
        flight = card == nil ? 0 : 1
        videoOpacity = card == nil ? 0 : 1
        launchStill = card == nil ? nil : still
        launch = fromHeader ? 1 : 0
        shift = fromHeader ? idleScroll : 0
        launchScroll = 0
        drop = 0
        fall = 0
        dismissing = false
        republish()
    }

    func runLaunch() {
        withAnimation(.spring(response: Self.slowed(0.45), dampingFraction: 0.88), completionCriteria: .logicallyComplete) {
            flight = 0
            launch = 0
            videoOpacity = 1
            republish()
        } completion: { [self] in
            if flight == 0 { flightTarget = nil }
        }
    }

    /// The video dragged down by `dy`: it shrinks towards the playing episode's card (`id`) as the
    /// header grows back and the text moves down under it; with no card on screen there it follows
    /// the finger and fades. The panel stays where it is (if it was away, it comes back).
    func videoDragged(_ dy: CGFloat, id: String?) {
        if !dismissing { beginDismiss(id: id) }
        let p = min(max(dy / dismissDistance, 0), 1)
        launch = p
        if flightTarget != nil {
            flight = p
        } else {
            drop = dy
            fall = p
            videoOpacity = 1 - p
        }
        if dismissFromCollapsed { panel = height * (1 - p) }
    }

    private func beginDismiss(id: String?) {
        dismissing = true
        dismissFromCollapsed = collapsed
        if launch == 0 { shift = 0 }
        launchScroll = panelScroll
        launchStill = nil
        flightTarget = nil
        // Where the card will be once the page is back: the text moves down by `span`, the panel
        // (if away) comes back, and above the row the title written out goes as Play comes back.
        let grow = (playRevealActive ? playRevealHeight : 0) - titleRevealHeight
            - (swapActive ? swapPlayingHeight - swapIdleHeight : 0)
        let video = videoFrame.offsetBy(dx: 0, dy: collapseProgress * travel)
        if let id, let card = cardFrames[id] {
            var target = card.offsetBy(dx: 0, dy: span - shift - panel + grow)
            let over = target.maxY + 12 - pageFrame.maxY
            if over > 0, launch == 0 {
                // As far as the page can scroll then.
                let most = headerHeight + Self.textGap + panelContentHeight + grow - screenFrame.height - launchScroll
                shift = max(0, min(over, most))
                target = target.offsetBy(dx: 0, dy: -shift)
            }
            if pageFrame.contains(CGPoint(x: target.midX, y: target.midY)), target.maxY <= pageFrame.maxY,
               target.midY - video.midY > 60 {
                flightTarget = target
                dismissDistance = target.midY - video.midY
            } else if launch == 0 {
                shift = 0
            }
        }
        if flightTarget == nil { dismissDistance = max(220, pageFrame.height * 0.35) }
        #if DEBUG
        fputs("MoonriseDismiss: begin target=\(flightTarget.map { "\($0)" } ?? "none") span=\(span) shift=\(shift) grow=\(grow) scroll=\(launchScroll) distance=\(dismissDistance) collapsed=\(dismissFromCollapsed)\n", stderr)
        #endif
    }

    /// Let go: `stop` (far or fast enough) finishes the way back — the video lands in its card (or
    /// fades), the header and the page are back — then `done` stops playback; else it springs back.
    func videoReleased(stop: Bool, then done: @escaping () -> Void) {
        guard dismissing else { return }
        dismissing = false
        let landing = flightTarget != nil
        if stop {
            restoreScroll = launchScroll + shift
            withAnimation(.spring(response: Self.slowed(0.36), dampingFraction: 0.9), completionCriteria: .logicallyComplete) {
                launch = 1
                if landing { flight = 1 } else { drop += 80; fall = 1; videoOpacity = 0 }
                panel = 0
                collapsed = false
                republish()
            } completion: { done() }
        } else {
            withAnimation(.spring(response: Self.slowed(0.38), dampingFraction: 0.86), completionCriteria: .logicallyComplete) {
                launch = 0
                flight = 0
                drop = 0
                fall = 0
                videoOpacity = 1
                if dismissFromCollapsed { panel = height }
                republish()
            } completion: { [self] in
                if !dismissing, flight == 0 { flightTarget = nil }
            }
        }
    }

    /// The video's travel to the middle of the screen as the panel goes away (set by its view).
    @ObservationIgnored var travel: CGFloat = 0

    /// 0 with the panel open … 1 with it away.
    var collapseProgress: CGFloat { height > 0 ? min(max(panel / height, 0), 1) : 0 }

    private static let spring = Animation.spring(response: 0.38, dampingFraction: 0.86)

    /// Test seam: MOONRISE_SLOW_ANIM=<factor> slows Play and the drag's finish down for mid-animation screenshots.
    static func slowed(_ response: Double) -> Double {
        #if DEBUG
        if let f = DebugSeams.launch.slowAnim { return response * f }
        #endif
        return response
    }

    func resize(_ h: CGFloat) {
        guard h > 0 else { return }
        if collapsed, !dismissing { panel = h }
        height = h
    }

    /// Dragged down from open by `d`.
    func panelDragged(_ d: CGFloat) { panel = min(max(0, d), height) }

    /// Dragged up from collapsed by `d`.
    func handleDragged(_ d: CGFloat) { panel = min(max(0, height - d), height) }

    /// Let go: on to whichever end the finger was heading for. `velocity` in points/s, down > 0.
    func settle(velocity: CGFloat) {
        let projected = panel + velocity * 0.25
        setCollapsed(collapsed ? projected > height * 0.65 : projected > height * 0.35)
    }

    func setCollapsed(_ c: Bool) {
        withAnimation(Self.spring) {
            panel = c ? height : 0
            collapsed = c
            republish()
        }
    }

    func reset() {
        flight = 0
        flightTarget = nil
        launch = 0
        shift = 0
        launchScroll = 0
        videoOpacity = 1
        drop = 0
        fall = 0
        launchStill = nil
        panel = 0
        collapsed = false
        dismissing = false
        republish()
    }

    /// Tells the views once more that what they draw from changed, now that every value holds its
    /// new one. SwiftUI hears of a change before the value is stored, and with another
    /// update waiting (the change just before it) it can bring a view up to date right then, from
    /// the old value; the change itself doesn't touch that view again. Play ended that way with the
    /// title still hidden (`launch` read as 1) or the Play row still there, until something else
    /// redrew them. Called after the last change of a group, inside its animation.
    func republish() {
        withMutation(keyPath: \.launch) {}
        withMutation(keyPath: \.flight) {}
        withMutation(keyPath: \.videoOpacity) {}
        withMutation(keyPath: \.drop) {}
        withMutation(keyPath: \.fall) {}
        withMutation(keyPath: \.panel) {}
        withMutation(keyPath: \.collapsed) {}
        withMutation(keyPath: \.shift) {}
        withMutation(keyPath: \.launchScroll) {}
        withMutation(keyPath: \.launchStill) {}
    }
}

/// Moves the video down by `travel` (to the middle) as the panel goes away, and shrinks it into
/// the episode's card as it's dragged there (`flight`), or with no card, down with the finger.
struct CollapseFollower<Content: View>: View {
    let drag: PageDrag
    let travel: CGFloat
    /// The video's place (global frame).
    let rest: CGRect
    @ViewBuilder let content: Content

    var body: some View {
        let f = drag.flight
        let from = rest
        // Scaled about its centre: move the centre onto the card's, shrink to its width; the
        // corner radius is drawn before the scale, so it's divided by it to end at the card's.
        let to = drag.flightTarget ?? from
        let scale = (from.width > 0 ? to.width / from.width : 1) * (1 - 0.3 * drag.fall)
        let shrink = 1 + (scale - 1) * max(f, drag.fall)
        content
            .clipShape(RoundedRectangle(cornerRadius: max(f, drag.fall) * PageDrag.cardRadius / max(scale, 0.01), style: .continuous))
            .opacity(drag.videoOpacity)
            .onChange(of: rest, initial: true) { drag.videoFrame = rest }
            .onChange(of: travel, initial: true) { drag.travel = travel }
            .scaleEffect(shrink)
            .offset(x: (to.midX - from.midX) * f, y: drag.collapseProgress * travel + (to.midY - from.midY) * f + drag.drop)
    }
}

/// The artwork header between the video's height and its full height (`launch` 0…1), where the
/// page has it when not playing: from the screen's top (`top`, global), scrolled as the page
/// will be, cut off just above the text, which moves with the panel. Gone at `launch` 0.
struct LaunchHeader<Header: View>: View {
    let drag: PageDrag
    let top: CGFloat
    /// The video's bottom on screen (global).
    let restBottom: CGFloat
    @ViewBuilder let header: Header

    var body: some View {
        if drag.launch > 0 {
            let bottom = PanelHost<EmptyView, EmptyView>.textTop(drag, restBottom: restBottom) - PageDrag.textGap - PageDrag.panelInset
            Color.clear
                .frame(height: max(0, bottom - top))
                .frame(maxWidth: .infinity)
                .overlay(alignment: .top) { header.offset(y: -(drag.shift + drag.launchScroll)) }
                .clipped()
                .opacity(drag.launch)
                .frame(maxHeight: .infinity, alignment: .top)
                .ignoresSafeArea(edges: .top)
                .allowsHitTesting(false)
        }
    }
}

/// The page's background behind the status bar while playing (over the shrinking artwork, so
/// none of it stays there), giving way to the black of the collapsed view.
struct StatusBarCover: View {
    let drag: PageDrag
    let height: CGFloat

    var body: some View {
        Theme.background
            .frame(height: height)
            .frame(maxWidth: .infinity)
            .offset(y: -height)
            .opacity((1 - drag.launch) * (1 - drag.collapseProgress))
            .allowsHitTesting(false)
    }
}

/// The episode's still in the video's place until playback has its first frame.
struct LaunchStill: View {
    let drag: PageDrag
    let model: PlayerModel

    var body: some View {
        if let url = drag.launchStill {
            RemoteImage(url: url, immediate: true)
                .allowsHitTesting(false)
                .transition(.opacity)
                // `initial`: this view is only in the page's portrait layout. Started in landscape
                // (full screen), or turned there before the first frame, playback is already under
                // way when the phone turns upright: the change has gone by, and the still would
                // stay over the playing video.
                .onChange(of: model.isBuffering, initial: true) { _, buffering in
                    if !buffering { withAnimation(.easeOut(duration: 0.3)) { drag.launchStill = nil } }
                }
        }
    }
}

/// Black behind the video, whole screen, as the panel goes away (video only, cinema-style).
struct CollapseBlackout: View {
    let drag: PageDrag

    var body: some View {
        Color.black.opacity(drag.collapseProgress).ignoresSafeArea().allowsHitTesting(false)
    }
}

/// The details panel; it sits `drag.panel` below its place, and with the header showing
/// (`launch` > 0) as far down as the page has its text then. The part of the list scrolled up out
/// of the panel shows above it again as the header comes back (the page is that far scrolled).
struct PanelHost<Panel: View, Handle: View>: View {
    let drag: PageDrag
    /// The video's bottom on screen (global): the panel's top.
    var restBottom: CGFloat = 0
    @ViewBuilder let panel: Panel
    @ViewBuilder let handle: Handle

    /// Where the top of the list's content is (global; above the panel when it's scrolled).
    static func textTop(_ drag: PageDrag, restBottom: CGFloat) -> CGFloat {
        restBottom + drag.panel + drag.launch * (drag.span - drag.shift) + PageDrag.panelInset - drag.launchScroll
    }

    var body: some View {
        let l = drag.launch
        let offset = drag.panel + l * (drag.span - drag.shift)
        // Shown from the panel's top; with the header coming back, from just under it (but not
        // over the video until it's on its way).
        let headerBottom = Self.textTop(drag, restBottom: restBottom) - PageDrag.textGap - PageDrag.panelInset
        let maskTop = l > 0 ? min(offset, max(headerBottom - restBottom, -l * restBottom)) : 0
        ZStack(alignment: .bottom) {
            handle
            panel
                .scrollClipDisabled(l > 0)
                .frame(maxHeight: .infinity)
                .offset(y: offset)
                .allowsHitTesting(!drag.collapsed && l == 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .mask(alignment: .top) {
            GeometryReader { g in
                Rectangle().frame(width: g.size.width, height: max(0, g.size.height - maskTop)).offset(y: maskTop)
            }
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { drag.resize($0) }
    }
}

/// A layout showing `fraction` of its content's height (from the top), animatable.
struct PartialHeight: Layout {
    var fraction: CGFloat
    var animatableData: CGFloat {
        get { fraction }
        set { fraction = newValue }
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard let s = subviews.first else { return .zero }
        let size = s.sizeThatFits(ProposedViewSize(width: proposal.width, height: nil))
        return CGSize(width: size.width, height: size.height * min(max(fraction, 0), 1))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        subviews.first?.place(at: CGPoint(x: bounds.midX, y: bounds.minY), anchor: .top,
                              proposal: ProposedViewSize(width: bounds.width, height: nil))
    }
}

/// While playing, a part of the details that isn't there on the page when not playing (`grows`
/// false: the title written out) or is only there then (`grows` true: the Play button): it
/// opens or closes with `launch`, so at `launch` 1 the details are laid out as on that page.
struct LaunchReveal<Content: View>: View {
    let drag: PageDrag
    let grows: Bool
    @ViewBuilder let content: Content

    var body: some View {
        let f = grows ? drag.launch : 1 - drag.launch
        PartialHeight(fraction: f) {
            content
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { [drag, grows] h in
                    if grows { drag.playRevealHeight = h } else { drag.titleRevealHeight = h }
                }
        }
        .opacity(f)
        .clipped()
        .allowsHitTesting(f >= 1)
        .onAppear { [drag, grows] in if grows { drag.playRevealActive = true } }
        .onDisappear { [drag, grows] in if grows { drag.playRevealActive = false } }
    }
}

/// While playing, a part of the details laid out one way then (`playing`) and another on the page
/// when not playing (`idle`): one fades into the other, and the height between theirs, with
/// `launch`, so at `launch` 1 the details are laid out as on that page.
struct LaunchSwap<Playing: View, Idle: View>: View {
    let drag: PageDrag
    @ViewBuilder let playing: Playing
    @ViewBuilder let idle: Idle

    var body: some View {
        let f = drag.launch
        BlendedHeight(fraction: f) {
            playing
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { [drag] in drag.swapPlayingHeight = $0 }
                // One after the other (two texts wrapped differently don't overlap).
                .opacity(max(0, 1 - 2 * Double(f)))
                .allowsHitTesting(f < 0.5)
            // (A stack: there, and measured, with nothing in it too.)
            VStack(spacing: 0) { idle }
                .frame(maxWidth: .infinity, alignment: .leading)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { [drag] in drag.swapIdleHeight = $0 }
                .opacity(max(0, 2 * Double(f) - 1))
                .allowsHitTesting(f >= 0.5)
        }
        .onAppear { [drag] in drag.swapActive = true }
        .onDisappear { [drag] in drag.swapActive = false }
    }
}

/// Two subviews on top of each other (from the top), as tall as the first at `fraction` 0 and as
/// the second at 1; animatable.
struct BlendedHeight: Layout {
    var fraction: CGFloat
    var animatableData: CGFloat {
        get { fraction }
        set { fraction = newValue }
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let sizes = subviews.map { $0.sizeThatFits(ProposedViewSize(width: proposal.width, height: nil)) }
        guard let a = sizes.first else { return .zero }
        let b = sizes.count > 1 ? sizes[1] : .zero
        let f = min(max(fraction, 0), 1)
        return CGSize(width: proposal.width ?? max(a.width, b.width), height: a.height + (b.height - a.height) * f)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for s in subviews {
            s.place(at: CGPoint(x: bounds.minX, y: bounds.minY), anchor: .topLeading,
                    proposal: ProposedViewSize(width: bounds.width, height: nil))
        }
    }
}

#if os(iOS)
/// Put inside the panel's ScrollView: a pan on that scroll view that, when the list is at its
/// top and the finger moves down, stops the scrolling and moves the whole panel instead (as a
/// sheet does). A drag that starts with the list scrolled just scrolls.
struct PanelPullDown: UIViewRepresentable {
    let drag: PageDrag

    func makeCoordinator() -> Coordinator { Coordinator(drag: drag) }

    func makeUIView(context: Context) -> Finder {
        let v = Finder()
        v.isUserInteractionEnabled = false
        v.coordinator = context.coordinator
        return v
    }

    func updateUIView(_ uiView: Finder, context: Context) { context.coordinator.drag = drag }

    /// Finds the enclosing scroll view once it is in the window.
    final class Finder: UIView {
        weak var coordinator: Coordinator?
        override func didMoveToWindow() {
            super.didMoveToWindow()
            guard window != nil else { return }
            var v = superview
            while let s = v, !(s is UIScrollView) { v = s.superview }
            if let sv = v as? UIScrollView { coordinator?.attach(sv) }
        }
    }

    @MainActor
    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        var drag: PageDrag
        private weak var scrollView: UIScrollView?
        private let pan = UIPanGestureRecognizer()
        private var startedAtTop = false
        private var tracking = false
        private var baseline: CGFloat = 0

        init(drag: PageDrag) {
            self.drag = drag
            super.init()
            pan.addTarget(self, action: #selector(handle(_:)))
            pan.delegate = self
        }

        func attach(_ sv: UIScrollView) {
            guard scrollView !== sv else { return }
            scrollView?.removeGestureRecognizer(pan)
            scrollView = sv
            sv.addGestureRecognizer(pan)
        }

        nonisolated func gestureRecognizer(_ g: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool { true }

        @objc private func handle(_ p: UIPanGestureRecognizer) {
            guard let sv = scrollView else { return }
            let top = -sv.adjustedContentInset.top
            let t = p.translation(in: nil)
            switch p.state {
            case .began:
                startedAtTop = sv.contentOffset.y <= top + 1
                tracking = false
            case .changed:
                if !tracking, startedAtTop, sv.contentOffset.y <= top + 1, t.y > 0, t.y > abs(t.x) {
                    tracking = true
                    baseline = t.y
                    // The list stops here (no rubber band); the panel moves instead.
                    sv.panGestureRecognizer.isEnabled = false
                    sv.panGestureRecognizer.isEnabled = true
                    sv.setContentOffset(CGPoint(x: sv.contentOffset.x, y: top), animated: false)
                }
                if tracking { drag.panelDragged(t.y - baseline) }
            case .ended, .cancelled, .failed:
                if tracking { drag.settle(velocity: p.state == .ended ? p.velocity(in: nil).y : 0) }
                tracking = false
            default: break
            }
        }
    }
}

/// Put inside the page's ScrollView (not playing): a page opened with the zoom transition closes
/// on any downward swipe at its top. The system's own dismiss pan only starts on a near-vertical
/// drag; one a little diagonal went to the scroll view instead, which pulled the page down past
/// its top (scroll bar showing) and sprang back. When the scroll view does that at its top with
/// the finger going down, the system's gesture didn't take it: this closes the page (it zooms
/// back into its card as on a tap of Back). Pages pushed without the zoom ignore it.
struct ZoomPullDismiss: UIViewRepresentable {
    let dismiss: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator(dismiss: dismiss) }

    func makeUIView(context: Context) -> Finder {
        let v = Finder()
        v.isUserInteractionEnabled = false
        v.coordinator = context.coordinator
        return v
    }

    func updateUIView(_ uiView: Finder, context: Context) { context.coordinator.dismiss = dismiss }

    final class Finder: UIView {
        weak var coordinator: Coordinator?
        override func didMoveToWindow() {
            super.didMoveToWindow()
            guard window != nil else { return }
            var v = superview
            while let s = v, !(s is UIScrollView) { v = s.superview }
            if let sv = v as? UIScrollView { coordinator?.attach(sv, finder: self) }
        }
    }

    @MainActor
    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        var dismiss: () -> Void
        private weak var scrollView: UIScrollView?
        private weak var finder: UIView?
        private let pan = UIPanGestureRecognizer()
        private var startedAtTop = false
        private var fired = false
        /// Pulled this far past the top (points), the page closes while the finger is down; less,
        /// it closes on a downward flick at release.
        static let pullToClose: CGFloat = 44

        init(dismiss: @escaping () -> Void) {
            self.dismiss = dismiss
            super.init()
            pan.addTarget(self, action: #selector(handle(_:)))
            pan.delegate = self
        }

        func attach(_ sv: UIScrollView, finder: UIView) {
            self.finder = finder
            guard scrollView !== sv else { return }
            scrollView?.removeGestureRecognizer(pan)
            scrollView = sv
            sv.addGestureRecognizer(pan)
        }

        nonisolated func gestureRecognizer(_ g: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool { true }

        /// The page was pushed with the zoom transition (and is on screen, not on its way in or out).
        private var zoomPage: UIViewController? {
            var r: UIResponder? = finder
            while let x = r, !(x is UIViewController) { r = x.next }
            guard let vc = r as? UIViewController else { return nil }
            let page = sequence(first: vc, next: { $0.parent }).first { $0.preferredTransition != nil }
            guard let page, page.transitionCoordinator == nil, !page.isBeingDismissed, !page.isMovingFromParent else { return nil }
            return page
        }

        @objc private func handle(_ p: UIPanGestureRecognizer) {
            guard let sv = scrollView else { return }
            let top = -sv.adjustedContentInset.top
            // How far the scroll view pulls the page down past its top (rubber band).
            let pulled = top - sv.contentOffset.y
            let t = p.translation(in: nil)
            switch p.state {
            case .began:
                startedAtTop = sv.contentOffset.y <= top + 1
                fired = false
            case .changed:
                if startedAtTop, !fired, t.y > 0, pulled >= Self.pullToClose { close(sv, why: "pulled \(Int(pulled))") }
            case .ended:
                let v = p.velocity(in: nil).y
                if startedAtTop, !fired, t.y > 0, pulled >= 12, v > 250 { close(sv, why: "flick \(Int(v))") }
            default: break
            }
        }

        private func close(_ sv: UIScrollView, why: String) {
            guard zoomPage != nil else { return }
            fired = true
            #if DEBUG
            fputs("MoonrisePullDismiss: \(why)\n", stderr)
            #endif
            // The page stops where it is (no spring back) and zooms back into its card.
            sv.panGestureRecognizer.isEnabled = false
            sv.panGestureRecognizer.isEnabled = true
            dismiss()
        }
    }
}

/// A downward pan that works alongside the player's own taps and drags (UIKit, so nothing
/// above it can swallow it). Reports (distance down, velocity, ended).
struct DownwardPan: UIGestureRecognizerRepresentable {
    let onChange: (_ dy: CGFloat, _ velocity: CGFloat, _ ended: Bool) -> Void

    func makeCoordinator(converter: CoordinateSpaceConverter) -> Coordinator { Coordinator() }

    func makeUIGestureRecognizer(context: Context) -> UIPanGestureRecognizer {
        let p = UIPanGestureRecognizer()
        p.delegate = context.coordinator
        return p
    }

    func handleUIGestureRecognizerAction(_ r: UIPanGestureRecognizer, context: Context) {
        let dy = max(0, r.translation(in: nil).y)
        switch r.state {
        case .began, .changed: onChange(dy, r.velocity(in: nil).y, false)
        case .ended: onChange(dy, r.velocity(in: nil).y, true)
        case .cancelled, .failed: onChange(0, 0, true)
        default: break
        }
    }

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        /// Only a drag that starts downwards (not a scrub, not an upward swipe).
        func gestureRecognizerShouldBegin(_ g: UIGestureRecognizer) -> Bool {
            guard let p = g as? UIPanGestureRecognizer else { return false }
            // (Synthesized touches can begin with no translation yet: then the velocity says.)
            let tr = p.translation(in: nil)
            let t = tr == .zero ? p.velocity(in: nil) : tr
            return t.y > 0 && t.y > abs(t.x)
        }

        func gestureRecognizer(_ g: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool { true }
    }
}

/// While the title plays in the page, the system's ways of leaving it by dragging its content
/// are off, so only the video's back button and the screen-edge swipe leave it:
/// - iOS 26 pops a page on a rightward swipe anywhere in its content (a sideways drag on the
///   details or the episodes left the page and stopped playback);
/// - a page opened with the zoom transition (from a Home card) shrinks into a card and closes
///   when its content is dragged down or pinched (dragging the details panel did that).
struct ContentPopBlocker: UIViewControllerRepresentable {
    var blocked: Bool

    func makeUIViewController(context: Context) -> Controller { Controller() }

    func updateUIViewController(_ vc: Controller, context: Context) {
        vc.blocked = blocked
        vc.apply()
    }

    final class Controller: UIViewController {
        var blocked = false

        override func didMove(toParent parent: UIViewController?) {
            super.didMove(toParent: parent)
            apply()
        }

        override func viewDidAppear(_ animated: Bool) {
            super.viewDidAppear(animated)
            apply()
        }

        override func viewWillDisappear(_ animated: Bool) {
            super.viewWillDisappear(animated)
            set(enabled: true)
        }

        func apply() { set(enabled: !blocked) }

        private func set(enabled: Bool) {
            if let pop = navigationController?.interactiveContentPopGestureRecognizer, pop.isEnabled != enabled {
                pop.isEnabled = enabled
                #if DEBUG
                fputs("MoonrisePop: content pop \(enabled ? "on" : "off")\n", stderr)
                #endif
            }
            // The zoom transition's dismiss gestures sit on the pushed page's view (no public switch).
            let page = sequence(first: self as UIViewController, next: { $0.parent }).first { $0.preferredTransition != nil }
            for g in page?.view.gestureRecognizers ?? [] where Self.isZoomDismiss(g) && g.isEnabled != enabled {
                g.isEnabled = enabled
                #if DEBUG
                fputs("MoonrisePop: \(type(of: g)) \(enabled ? "on" : "off")\n", stderr)
                #endif
            }
        }

        private static func isZoomDismiss(_ g: UIGestureRecognizer) -> Bool {
            let name = String(describing: type(of: g))
            return name.contains("SwipeDismiss") || name.contains("TransformGesture")
        }
    }
}
#endif
