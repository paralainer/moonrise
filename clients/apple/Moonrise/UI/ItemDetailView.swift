import SwiftUI

/// Detail page. For shows it is centred on one selected episode: title, meta,
/// Resume, actions and overview all describe that episode, with a horizontal episode carousel
/// below. Opening an episode from anywhere lands here with that episode selected.
/// Play turns the header into the video (iPhone, iPad, Mac; full screen in landscape or with
/// its button); leaving the page stops it.
struct ItemDetailView: View {
    @Environment(\.metrics) private var metrics
    let source: Source

    @Environment(PlaybackCoordinator.self) private var playback
    @Environment(\.dismiss) private var dismiss
    @State private var inline = InlinePlayback()
    /// Playing: the details panel dragged away (YouTube-style), Play and the video dragged down.
    @State private var drag = PageDrag()
    /// The page's scroll position when not playing (back from playing: where the drag left it).
    @State private var idleScroll = ScrollPosition(edge: .top)
    /// Bumped to lay the playing page out again (iPad: back from full screen).
    @State private var relayoutTick = 0
    #if os(iOS)
    @Environment(\.verticalSizeClass) private var verticalSizeClass
    #endif
    private let autoplay: Autoplay?
    @State private var autoplayed = false
    // MediaItem's == compares only the id and the watch state, so a plain @State would drop a
    // refetched item that only adds details (cast and crew, director, overview) as "no change"
    // and the page would show the stub until something else redrew it. `Fresh` counts every write.
    @State private var itemState: Fresh<MediaItem>
    private var item: MediaItem {
        get { itemState.value }
        nonmutating set { itemState = itemState.next(newValue) }
    }
    @State private var seasons: [MediaItem] = []
    @State private var selectedSeason: MediaItem?
    @State private var episodesState = Fresh<[MediaItem]>(value: [])
    private var episodes: [MediaItem] {
        get { episodesState.value }
        nonmutating set { episodesState = episodesState.next(newValue) }
    }
    @State private var selectedEpisodeState: Fresh<MediaItem?>
    private var selectedEpisode: MediaItem? {
        get { selectedEpisodeState.value }
        nonmutating set { selectedEpisodeState = selectedEpisodeState.next(newValue) }
    }
    @State private var overviewExpanded = false
    @State private var error: String?
    /// The server can't be reached, or didn't answer: the page is what the downloads saved,
    /// and what needs the server isn't offered.
    @State private var offline: OfflineTitlePage?
    /// Season to open on when the page was entered from a Season item (e.g. "Recently Added").
    private let preselectedSeasonId: String?

    init(source: Source, item: MediaItem, autoplay: Autoplay? = nil) {
        self.source = source
        self.autoplay = autoplay
        // Episodes and seasons open their show; the episode/season becomes the selection.
        let inShow = item.isEpisode || item.isSeason
        _itemState = State(initialValue: Fresh(value: inShow ? Self.showStub(for: item) : item))
        _selectedEpisodeState = State(initialValue: Fresh(value: item.isEpisode ? item : nil))
        preselectedSeasonId = item.isSeason ? item.id : nil
    }

    /// The show, as far as the episode/season already knows it: enough artwork (backdrop, logo) for
    /// the header to be there during the zoom transition instead of popping in after the fetch.
    static func showStub(for child: MediaItem) -> MediaItem {
        MediaItem(id: child.seriesId ?? child.id, kind: .series, name: child.seriesName, images: child.seriesImages ?? ImageSet())
    }

    private var client: (any MediaServerClient)? { MediaServers.client(for: source) }
    private var isShow: Bool { item.isSeries }
    /// What the buttons and text act on: the selected episode for shows, the item itself otherwise.
    /// With a mark from an episode card's long-press menu applied until the reload shows it.
    private var focus: MediaItem? { (isShow ? selectedEpisode : item).map { WatchState.shared.current(versionDetails.full($0).asChosenVersion) } }
    /// Episodes from a list with several versions: their files, loaded for the version picker.
    @State private var versionDetails = VersionDetails()


    var body: some View {
        content
        .background(Theme.background.ignoresSafeArea())
        #if os(tvOS)
        .ignoresSafeArea(edges: .top) // header runs under the floating tab bar
        #endif
        #if os(iOS)
        // Playing: the video sits right under the status bar; its own back button replaces the page's.
        .ignoresSafeArea(edges: inline.model == nil ? .top : [])
        .toolbarBackground(.hidden, for: .navigationBar)
        .scrollEdgeEffectStyle(.soft, for: .top)
        // Playing: nothing scrolls under the status bar (its blur kept a slice of the artwork there).
        .scrollEdgeEffectHidden(inline.model != nil, for: .top)
        .toolbar(inline.model != nil ? .hidden : .automatic, for: .navigationBar)
        .toolbar(tabBarHidden ? .hidden : .automatic, for: .tabBar)
        #endif
        #if os(macOS)
        // As the iPad's: the artwork runs under the toolbar (back button, tabs); playing, the
        // video sits below it.
        .ignoresSafeArea(edges: inline.model == nil ? .top : [])
        #endif
        .inlineTitle()
        .task {
            await load()
            if let autoplay, !autoplayed, let target = focus, target.isPlayable {
                autoplayed = true
                play(PlaybackRequest(target: .server(source, target), startAt: autoplay.startAt))
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .moonriseItemRematched)) { _ in
            versionDetails.forgetAll()
            Task { await load() }
        }
        .onReceive(NotificationCenter.default.publisher(for: .playbackDidEnd)) { _ in
            Task { try? await Task.sleep(for: .seconds(1)); await refreshUserData() }
        }
        // Back online: the server's page replaces the downloads' one.
        .onChange(of: ConnectionRouter.shared.isReachable) { _, reachable in
            if reachable, offline != nil { Task { await load() } }
        }
        .onAppear { inline.pageAppeared() }
        .onDisappear { inline.pageDisappeared() }
        // Up Next or the full-screen picker moved on to another episode: the page follows.
        .onChange(of: inline.item?.id) { _, _ in follow(inline.item) }
        .onChange(of: inline.model == nil) { _, stopped in if stopped { drag.reset() } }
        // iPad turned taller while playing full screen: back to the player in the page (turned
        // wider, `playerFullScreen` makes it full screen). Same player: the stream goes on.
        .onChange(of: inline.fullScreen) { _, full in if !full { relayoutSoon() } }
        #if os(iOS)
        .onChange(of: metrics.wide) { _, wide in
            guard !wide, inline.model != nil else { return }
            if inline.fullScreen { inline.setFullScreen(false) }
            relayoutSoon()
        }
        #endif
        #if os(iOS)
        .background(ContentPopBlocker(blocked: inline.model != nil))
        #endif
    }

    @ViewBuilder private var content: some View {
        if let model = inline.model {
            if playerFullScreen {
                // Reaches under the safe areas itself (the controls stay inside them).
                PlayerScreen(model: model, inline: inlineHooks(fullScreen: true))
                    .id(ObjectIdentifier(model))
            } else {
                GeometryReader { geo in
                    // iPad, back from full screen: the page's safe area can come back without the
                    // status bar's part (the video would sit under it); the window still has it.
                    let _ = relayoutTick
                    let extra = Self.missingTopInset(geo.safeAreaInsets.top)
                    let insetTop = geo.safeAreaInsets.top + extra
                    let pageHeight = geo.size.height - extra
                    // 16:9 across the page, but leaving room for the page on wide windows.
                    let height = min(geo.size.width * 9 / 16, pageHeight * 0.6)
                    let gOrigin = geo.frame(in: .global).origin
                    let origin = CGPoint(x: gOrigin.x, y: gOrigin.y + extra)
                    let rest = CGRect(x: origin.x, y: origin.y, width: geo.size.width, height: height)
                    // The page when not playing starts at the screen's top (under the status bar).
                    let top = origin.y - insetTop
                    // The header's height on the page now (it changes with the orientation).
                    let headerHeight = headerImage == nil ? TitleHeader.height(metrics) * 0.45 : TitleHeader.height(metrics)
                    let span = drag.span(top: top, restBottom: rest.maxY, headerHeight: headerHeight)
                    Group {
                        ZStack(alignment: .top) {
                            LaunchHeader(drag: drag, top: top, restBottom: rest.maxY) { header }
                            StatusBarCover(drag: drag, height: insetTop)
                            VStack(spacing: 0) {
                                // Panel going away: the video moves to the middle of the screen (not
                                // just of the area under the status bar) over black, with the drag.
                                CollapseFollower(drag: drag, travel: (pageHeight - height + geo.safeAreaInsets.bottom - insetTop) / 2,
                                                 rest: rest) {
                                    PlayerScreen(model: model, inline: inlineHooks(fullScreen: false))
                                        .id(ObjectIdentifier(model))
                                        .frame(width: geo.size.width, height: height)
                                        .overlay { LaunchStill(drag: drag, model: model) }
                                }
                                .zIndex(1)
                                PanelHost(drag: drag, restBottom: rest.maxY) { panel } handle: { EmptyView() }
                            }
                        }
                        // Details away: a swipe up anywhere (the black, the video) brings them
                        // back, following the finger.
                        .contentShape(Rectangle())
                        #if !os(tvOS)
                        .simultaneousGesture(DragGesture(minimumDistance: 10)
                            .onChanged { v in
                                if drag.restoring == nil { drag.restoring = drag.collapsed && v.translation.height < 0 }
                                if drag.restoring == true { drag.handleDragged(-v.translation.height) }
                            }
                            .onEnded { v in
                                if drag.restoring == true { drag.settle(velocity: v.velocity.height); drag.restoredAt = Date() }
                                drag.restoring = nil
                            })
                        #endif
                        .accessibilityAction(named: "Show Details") { drag.setCollapsed(false) }
                    }
                    .onChange(of: span, initial: true) { [drag] in drag.span = span; drag.headerHeight = headerHeight }
                    .background(CollapseBlackout(drag: drag))
                    .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { [drag] in drag.pageFrame = $0 }
                    .padding(.top, extra)
                }
            }
        } else {
            // Back from playing, it opens scrolled as far as the drag down left it.
            ScrollView {
                PagePart { page(header: true) }
                    #if os(iOS)
                    .background(ZoomPullDismiss(dismiss: { [dismiss] in dismiss() }).frame(width: 0, height: 0))
                    #endif
            }
                .scrollPosition($idleScroll)
                .onScrollGeometryChange(for: CGFloat.self) { $0.contentOffset.y + $0.contentInsets.top } action: { [drag] _, y in drag.idleScroll = y }
                .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { [drag] in drag.screenFrame = $0 }
        }
    }

    /// The page's gap between its sections (header, title block, episodes, cast). Playing, the
    /// title block sits as far below the video as it sits below the artwork header otherwise.
    static let sectionGap: CGFloat = 18

    /// Its sections are `PagePart`s: the page's type stays shallow and small, and no one body
    /// builds the whole page (see `PagePart`).
    private func page(header showHeader: Bool) -> some View {
        VStack(spacing: Self.sectionGap) {
            if showHeader {
                header
            }
            if metrics.regular {
                PagePart { padColumn(showHeader: showHeader) }
            } else {
                PagePart { phoneColumn(showHeader: showHeader) }
            }

            if isShow { PagePart { episodeSection } }

            if let people = item.people?.filter({ ["Actor", "Director", "Writer"].contains($0.kind ?? "") }).prefix(24), !people.isEmpty {
                PagePart {
                    Shelf(title: "Cast and Crew") {
                        ForEach(Array(people)) { p in CastCard(person: p, client: client) }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            if metrics.regular {
                PagePart {
                    TitleDetails(item: item, focus: focus, seasons: seasons.count)
                        .id(focus.flatMap { VersionChoices.shared.version(for: $0)?.id })
                }
            }
            if let error { Text(error).font(.footnote).foregroundStyle(.red).padding(.horizontal, metrics.hPadding) }
        }
        .padding(.top, showHeader ? 0 : PageDrag.panelInset)
        .padding(.bottom, 40)
    }

    /// iPad: title, line, Play and overview at the page's leading margin (not centred), as wide
    /// as text reads well. Wider than tall, they sit over the bottom of the picture. Playing (the
    /// video above), Download is at the title's end rather than alone on a line of its own.
    private func padColumn(showHeader: Bool) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            VStack(alignment: .leading, spacing: 0) {
                PagePart { padTitleRow(showHeader: showHeader) }
                PagePart { playButton(idle: showHeader) }
                PagePart { downloadStatus(idle: showHeader) }
            }
            PagePart { overview }
            PagePart { ratings }
        }
        .frame(maxWidth: Self.padColumnWidth, alignment: .leading)
        .padding(.horizontal, metrics.hPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.top, showHeader ? -TitleHeader.overlap(metrics, hasImage: headerImage != nil) : 0)
    }

    /// iPad and Mac: the title block; playing it, with Download right after the title's text
    /// (`TitleAccessoryRow`), not at the column's far end.
    @ViewBuilder private func padTitleRow(showHeader: Bool) -> some View {
        if !showHeader, let target = focus, target.isPlayable, DownloadManager.isSupported, playing(target) != nil {
            TitleAccessoryRow(spacing: Self.titleAccessoryGap) {
                titleBlock(named: true, fill: false)
                LaunchReveal(drag: drag, grows: false) { downloadButton(target, size: Self.statusButton) }
            }
        } else {
            titleBlock(named: !showHeader)
        }
    }

    /// iPhone: title, line, Play and overview, centred. Playing, the title is written out and Play
    /// isn't there; dragging the video down turns them back into the page's as the header comes back.
    private func phoneColumn(showHeader: Bool) -> some View {
        VStack(spacing: 16) {
            VStack(spacing: 0) {
                PagePart { titleBlock(named: !showHeader) }
                PagePart { playButton(idle: showHeader) }
                PagePart { downloadStatus(idle: showHeader) }
            }
            PagePart { overview }
            PagePart { ratings }
        }
        .padding(.horizontal, metrics.hPadding)
        .frame(maxWidth: Self.columnWidth)
    }

    /// Lays the playing page out again once a rotation or the way out of full screen is over
    /// (the window's status bar inset is final by then; see `missingTopInset`).
    private func relayoutSoon() {
        Task {
            for delay in [150, 600] {
                try? await Task.sleep(for: .milliseconds(delay))
                relayoutTick += 1
            }
        }
    }

    /// iPad, back from full screen: how much of the window's top safe area (the status bar) the
    /// page's lost. 0 elsewhere (iPhone untouched).
    private static func missingTopInset(_ pageTop: CGFloat) -> CGFloat {
        #if os(iOS)
        guard UIDevice.current.userInterfaceIdiom == .pad,
              let scene = UIApplication.shared.connectedScenes.first as? UIWindowScene else { return 0 }
        let statusBar = max(scene.keyWindow?.safeAreaInsets.top ?? 0, scene.statusBarManager?.statusBarFrame.height ?? 0)
        return max(0, statusBar - pageTop)
        #else
        return 0
        #endif
    }

    /// iPad: the text column's width (about 80 characters of the overview).
    private static let padColumnWidth: CGFloat = 640

    // MARK: Playing on the page

    /// iPhone: hidden in full screen and while the panel is away. iPad: its tab bar floats over
    /// the top, where the video now sits, so hidden while playing.
    private var tabBarHidden: Bool {
        guard inline.model != nil else { return false }
        #if os(iOS)
        if UIDevice.current.userInterfaceIdiom == .pad { return true }
        #endif
        return playerFullScreen || drag.collapsed
    }

    /// Everything under the playing video. With its list at the top, dragging down moves the
    /// whole panel with the finger and lets it go away (or spring back); the handle left at the
    /// bottom brings it back the same way.
    private var panel: some View {
        ScrollView {
            PagePart { page(header: false) }
                #if os(iOS)
                .background(PanelPullDown(drag: drag).frame(width: 0, height: 0))
                #endif
        }
        .onScrollGeometryChange(for: CGFloat.self) { $0.contentOffset.y + $0.contentInsets.top } action: { [drag] _, y in drag.panelScroll = y }
        .onScrollGeometryChange(for: CGFloat.self) { $0.contentSize.height } action: { [drag] _, h in drag.panelContentHeight = h }
        .background(Theme.background)
    }

    /// iPhone: full screen in landscape (its button rotates the phone). iPad wider than tall:
    /// always full screen (the page shows again when it stops). iPad taller/Mac: the button.
    private var playerFullScreen: Bool {
        guard inline.model != nil else { return false }
        #if os(iOS)
        if UIDevice.current.userInterfaceIdiom == .phone { return verticalSizeClass == .compact }
        if metrics.wide { return true }
        #endif
        return inline.fullScreen
    }

    /// iPad wider than tall: leaving full screen is leaving the player (back to the page as
    /// before Play; there's no small player in this layout).
    private var padStopsOnExit: Bool {
        #if os(iOS)
        return UIDevice.current.userInterfaceIdiom == .pad && metrics.wide
        #else
        return false
        #endif
    }

    private func inlineHooks(fullScreen: Bool) -> InlinePlayerHooks {
        InlinePlayerHooks(
            fullScreen: fullScreen,
            toggleFullScreen: { [inline, padStopsOnExit] in
                #if os(iOS)
                if UIDevice.current.userInterfaceIdiom == .phone {
                    SystemControls.rotate(toLandscape: !fullScreen)
                    return
                }
                if fullScreen, padStopsOnExit {
                    inline.stop()
                    return
                }
                #endif
                inline.setFullScreen(!fullScreen)
            },
            // Swipe up on the video: full screen, unless the details are away (that brings them back).
            swipeUp: { [drag, inline] in
                guard !drag.collapsed, drag.restoring == nil, Date().timeIntervalSince(drag.restoredAt) > 0.5 else { return }
                #if os(iOS)
                if UIDevice.current.userInterfaceIdiom == .phone { SystemControls.rotate(toLandscape: true); return }
                #endif
                inline.setFullScreen(true)
            },
            back: { [dismiss] in dismiss() },
            // The video dragged down shrinks into its card as the page comes back (Play in
            // reverse, following the finger); let go far or fast enough, it lands there, then
            // playback stops (reporting where it was) and the page is as before Play.
            dragDown: { [drag, inline, idleScroll = $idleScroll] dy, velocity, ended in
                guard ended else { drag.videoDragged(dy, id: inline.item?.id); return }
                drag.videoReleased(stop: dy > 120 || (velocity > 800 && dy > 20)) { [drag] in
                    // Laid out as the page is now: no animation, it's already there.
                    idleScroll.wrappedValue.scrollTo(y: drag.restoreScroll)
                    inline.stop()
                    #if DEBUG
                    Task { [drag] in
                        try? await Task.sleep(for: .milliseconds(300))
                        fputs("MoonriseDismiss: page back at scroll \(drag.idleScroll) (asked \(drag.restoreScroll))\n", stderr)
                    }
                    #endif
                }
            },
            onModal: { [inline] open in inline.modalOpen = open },
            download: phoneDownload)
    }

    /// iPhone: the playing title, for Download in the player's controls.
    private var phoneDownload: InlineDownload? {
        #if os(iOS)
        guard UIDevice.current.userInterfaceIdiom == .phone, let it = inline.item, it.isPlayable else { return nil }
        return InlineDownload(item: it, source: source)
        #else
        return nil
        #endif
    }

    /// Plays on the page; the Apple TV keeps its full-screen player.
    private func play(_ request: PlaybackRequest) {
        #if os(tvOS)
        playback.play(request)
        #else
        // The video grows out of the episode's card (its still until it plays) into its place
        // while the header shrinks to its height; with no card on screen it fades in.
        var still: URL?
        var id: String?
        if case let .server(_, it) = request.target { id = it.id; still = client?.thumbURL(it, maxWidth: 600) }
        let playing = inline.model != nil
        drag.prepareLaunch(card: drag.visibleCard(id, playing: playing), still: still, fromHeader: !playing)
        inline.play(request, coordinator: playback)
        Task { [drag] in
            try? await Task.sleep(for: .milliseconds(30)) // laid out in its place first
            drag.runLaunch()
        }
        #endif
    }

    /// The model playing this title or episode, if it's the one playing.
    private func playing(_ target: MediaItem) -> PlayerModel? {
        inline.item?.id == target.id ? inline.model : nil
    }

    /// Another episode plays: select it (and its season).
    private func follow(_ ep: MediaItem?) {
        guard isShow, let ep, ep.isEpisode, ep.id != selectedEpisode?.id else { return }
        selectedEpisode = episodes.first { $0.id == ep.id } ?? ep
        overviewExpanded = false
        if ep.seasonId != nil, ep.seasonId != selectedSeason?.id, let season = seasons.first(where: { $0.id == ep.seasonId }) {
            selectedSeason = season
            Task { await loadEpisodes(selectFirst: false) }
        }
    }

    // MARK: Header

    /// Backdrop → (shows) the selected episode's still → poster. Some library items have no artwork at all.
    private var headerImage: URL? {
        if offline != nil { return offlineArtwork }
        if let b = client?.backdropURL(item) { return b }
        if isShow, let e = selectedEpisode, let still = e.images.primary { return client?.imageURL(still, width: 1400) }
        return client?.posterURL(item, maxWidth: 1200)
    }

    /// The page's picture from the downloads: the selected episode's still, else the title's own.
    private var offlineArtwork: URL? {
        let manager = DownloadManager.shared
        if isShow {
            if let e = selectedEpisode, let url = manager.record(forItem: e.id)?.pageArtwork { return url }
            return manager.episodes(ofSeries: item.id).lazy.compactMap(\.pageArtwork).first
        }
        return manager.record(forItem: item.id)?.pageArtwork
    }

    private var header: some View {
        TitleHeader(image: headerImage, logo: offline == nil ? client?.logoURL(item) : nil, title: item.displayTitle)
            // Edge to edge (iPhone landscape: behind the Dynamic Island and the side insets).
            .ignoresSafeArea(edges: .horizontal)
    }

    private static var columnWidth: CGFloat {
        #if os(tvOS)
        return 1100
        #else
        return 700
        #endif
    }

    // MARK: Title + meta

    /// `named`: the header (with the logo) is the video now, so the title is written out.
    /// `fill`: as wide as the column (else as wide as its text, with something after it).
    private func titleBlock(named: Bool, fill: Bool = true) -> some View {
        let lead = metrics.regular
        return VStack(alignment: lead ? .leading : .center, spacing: 0) {
            if named {
                // Goes as the header (with the logo) comes back.
                LaunchReveal(drag: drag, grows: false) {
                    Text(item.displayTitle)
                        .font(isShow ? .headline : .title2.weight(.bold))
                        .foregroundStyle(isShow ? Theme.secondaryText : .white)
                        .multilineTextAlignment(lead ? .leading : .center)
                        .padding(.bottom, 6)
                }
            }
            VStack(alignment: lead ? .leading : .center, spacing: 6) {
                if isShow, let e = selectedEpisode {
                    Text([e.episodeLabel?.replacingOccurrences(of: " · ", with: " • "), e.name].compactMap { $0 }.joined(separator: " - "))
                        .font(.title3.weight(.semibold))
                        .multilineTextAlignment(lead ? .leading : .center)
                }
                TitleMeta(item: item, focus: focus, downloadTag: false, alignment: lead ? .leading : .center)
                    // (Another version chosen: its badges; the item compares equal.)
                    .id(focus.flatMap { VersionChoices.shared.version(for: $0)?.id })
            }
        }
        .foregroundStyle(.white)
        .frame(maxWidth: fill ? .infinity : nil, alignment: lead ? .leading : .center)
        // Admins: long-press (Mac: right-click) the title for Fix Match.
        .modifier(FixMatchTitleMenu(item: item))
    }

    // MARK: Play

    /// Below the title, 16 apart: Play, and Download beside it (as tall, 12 apart, the row as wide
    /// as the text below). Playing it, the video's own controls play and pause: only the row's
    /// space opens as the video is dragged down (`idle` false); Download is on the status line then.
    @ViewBuilder private func playButton(idle: Bool) -> some View {
        if let target = focus, target.isPlayable, let problem = target.unplayableProblem {
            // No file can play (damaged, or gone from disk): no Play or Download, what's wrong instead.
            DamagedTitleNote(problem: problem)
                .frame(maxWidth: metrics.regular ? Self.notePadWidth : .infinity, alignment: .leading)
                .padding(.top, 16)
        } else if let target = focus, target.isPlayable {
            // Several versions: the chosen one plays, from its own resume point.
            let vp = VersionPlay(target)
            let row = VStack(alignment: .leading, spacing: Self.statusGap) {
                HStack(spacing: Self.rowGap) {
                    Button {
                        play(PlaybackRequest(target: .server(source, target), startAt: vp.version != nil ? (vp.resume ?? 0) : nil,
                                             versionId: vp.version?.id))
                    } label: {
                        PlayCapsule(icon: "play.fill", title: vp.resume == nil ? "Play" : "Resume",
                                    time: vp.resume, progress: vp.progress)
                            .frame(width: metrics.regular ? PlayCapsule.padWidth : nil)
                    }
                    .plainButton()
                    .shadow(color: .black.opacity(0.3), radius: 8, y: 3)
                    if DownloadManager.isSupported { downloadButton(target, size: PlayCapsule.height) }
                }
                #if !os(tvOS)
                // "Version: 4K HDR10 · Director's Cut", at the row's leading edge.
                VersionLine(item: target, source: source)
                #endif
            }
            .padding(.top, 16)
            .task(id: VersionDetails.key(target)) { await versionDetails.load(target, client: client) }
            if idle || playing(target) == nil {
                row
            } else {
                LaunchReveal(drag: drag, grows: true) { row }
            }
        }
    }

    /// "Downloaded · 480p …" under the Play row. Playing it (no Play row), Download is at the
    /// line's end, or on a line of its own at the right with nothing downloaded; dragging the
    /// video down, it goes back beside Play as the row opens.
    @ViewBuilder private func downloadStatus(idle: Bool) -> some View {
        if let target = focus, target.isPlayable, !target.allVersionsUnplayable {
            let hasLine = DownloadManager.shared.record(forItem: target.id) != nil
            let line = DownloadStatusLine(item: target)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, Self.statusGap)
            if idle || playing(target) == nil || !DownloadManager.isSupported || metrics.regular {
                if hasLine { line }
            } else if phoneDownload != nil {
                // iPhone: Download is in the player's controls; no row of its own here.
                EmptyView()
            } else {
                LaunchSwap(drag: drag) {
                    HStack(spacing: Self.rowGap) {
                        if hasLine {
                            DownloadStatusLine(item: target)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        } else {
                            Spacer(minLength: 0)
                        }
                        downloadButton(target, size: Self.statusButton)
                    }
                    .padding(.top, Self.statusGap)
                } idle: {
                    if hasLine { line }
                }
            }
        }
    }

    private func downloadButton(_ target: MediaItem, size: CGFloat) -> some View {
        DownloadButton(item: target, source: source, size: size)
            .frame(width: size, height: size)
            .glassEffect(.regular.interactive(), in: .circle)
            .foregroundStyle(.white)
    }

    private static let rowGap: CGFloat = 12
    /// Between the title's text and Download after it (the meta line's badges' gap, doubled).
    private static let titleAccessoryGap: CGFloat = 16
    /// iPad: the damaged note's width (Play's and Download's, with the gap between).
    private static let notePadWidth: CGFloat = PlayCapsule.padWidth + rowGap + PlayCapsule.height
    private static let statusGap: CGFloat = 12
    /// Download on the status line while playing: a touch target, not as tall as Play.
    private static let statusButton: CGFloat = 44

    // MARK: Overview + ratings

    @ViewBuilder private var overview: some View {
        if let text = focus?.overview ?? item.overview {
            VStack(alignment: .leading, spacing: 4) {
                Text(text)
                    .font(.body)
                    .foregroundStyle(.white.opacity(0.9))
                    .lineLimit(overviewExpanded ? nil : 3)
                if !overviewExpanded, text.count > 140 {
                    Button("More") { withAnimation { overviewExpanded = true } }
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(Theme.secondaryText)
                        .plainButton()
                }
                if let credit = DemoClient.credit(item.id) {
                    Text(credit).font(.footnote).foregroundStyle(Theme.secondaryText).padding(.top, 4)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder private var ratings: some View {
        // The server's scores; the app looks nothing up itself.
        let r = ExternalRatings.page(own: (focus ?? item).ratings, show: isShow ? item.ratings : nil)
        // The critic rating shows as Rotten Tomatoes when OMDb has it; TMDB's star stays beside them.
        let rt = r.rottenTomatoes ?? (focus?.criticRating ?? item.criticRating).map { Int($0) }
        let community = focus?.communityRating ?? item.communityRating
        if r.imdb != nil || rt != nil || r.metacritic != nil || community != nil {
            // All four on a narrow phone: closer together.
            ViewThatFits(in: .horizontal) {
                ratingsRow(r, rt: rt, community: community, compact: false)
                ratingsRow(r, rt: rt, community: community, compact: true)
            }
            .font(.headline)
            .foregroundStyle(.white.opacity(0.9))
            .frame(maxWidth: .infinity, alignment: metrics.regular ? .leading : .center)
        }
    }

    /// IMDb, TMDB's star, Rotten Tomatoes and Metacritic, the ones there are.
    private func ratingsRow(_ r: ExternalRatings, rt: Int?, community: Double?, compact: Bool) -> some View {
        HStack(spacing: compact ? 12 : 20) {
            if let v = r.imdb {
                HStack(spacing: 6) {
                    Text("IMDb")
                        .font(.caption.weight(.heavy))
                        .foregroundStyle(.black)
                        .padding(.horizontal, 4).padding(.vertical, 1)
                        .background(Color(red: 0.96, green: 0.77, blue: 0.09), in: RoundedRectangle(cornerRadius: 3))
                    Text(String(format: "%.1f", v))
                }
                .accessibilityElement(children: .combine)
                .accessibilityLabel(String(format: "IMDb %.1f", v))
            }
            if let c = community {
                Label(String(format: "%.1f", c), systemImage: "star.fill")
                    .accessibilityLabel(String(format: "TMDB %.1f", c))
            }
            if let v = rt {
                HStack(spacing: 6) {
                    TomatoIcon(fresh: v >= 60)
                    Text("\(v)%")
                }
                .accessibilityLabel("Rotten Tomatoes \(v)%")
            }
            if let v = r.metacritic {
                // "MC" in Metacritic's score colour, then the score, as IMDb's badge.
                HStack(spacing: 6) {
                    Text("MC")
                        .font(.caption.weight(.heavy))
                        .foregroundStyle(v >= 61 || v < 40 ? .white : .black)
                        .padding(.horizontal, 4).padding(.vertical, 1)
                        .background(v >= 61 ? Color(red: 0.4, green: 0.8, blue: 0.2)
                                    : v >= 40 ? Color(red: 1, green: 0.8, blue: 0.2) : Color(red: 1, green: 0, blue: 0),
                                    in: RoundedRectangle(cornerRadius: 3))
                    Text("\(v)")
                }
                .accessibilityElement(children: .combine)
                .accessibilityLabel("Metacritic \(v)")
            }
        }
    }

    // MARK: Episodes

    private var episodeCardWidth: CGFloat {
        #if os(tvOS)
        return 420
        #else
        return 250
        #endif
    }

    /// iPad: the seasons as a row of chips (the selected one white), and the season's Download
    /// in a menu at the row's end.
    private var seasonChips: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 10) {
                    ForEach(seasons) { s in
                        let on = s.id == selectedSeason?.id
                        let n = DownloadManager.shared.seasonCounts[s.id] ?? 0
                        Button {
                            guard !on else { return }
                            selectedSeason = s
                            Task { await loadEpisodes(selectFirst: true) }
                        } label: {
                            HStack(spacing: 6) {
                                Text(s.displayTitle)
                                if n > 0 { Image(systemName: "arrow.down.circle.fill").font(.footnote) }
                            }
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(on ? .black : .white)
                            .padding(.horizontal, 16)
                            .frame(height: 36)
                            .background(on ? AnyShapeStyle(.white) : AnyShapeStyle(Theme.surfaceStrong), in: Capsule())
                        }
                        .plainButton()
                        .id(s.id)
                    }
                    if DownloadManager.isSupported, offline == nil, let season = selectedSeason, !episodes.isEmpty {
                        let missing = episodes.filter { $0.isPlayable && DownloadManager.shared.record(forItem: $0.id) == nil }
                        Menu {
                            Button {
                                let q = QualityOption.find(AppSettings.shared.downloadQuality)
                                for ep in missing { DownloadManager.shared.download(item: ep, from: source, quality: q) }
                            } label: {
                                Label(missing.isEmpty ? "\(season.displayTitle) Downloaded" : "Download \(season.displayTitle) (\(missing.count))",
                                      systemImage: "arrow.down.circle")
                            }
                            .disabled(missing.isEmpty)
                        } label: {
                            Image(systemName: "ellipsis")
                                .font(.subheadline.weight(.bold))
                                .foregroundStyle(.white)
                                .frame(width: 36, height: 36)
                                .background(Theme.surfaceStrong, in: Circle())
                        }
                        .drawnMenu()
                        .accessibilityLabel("Season options")
                    }
                }
            }
            .contentMargins(.horizontal, metrics.hPadding, for: .scrollContent)
            .onAppear { if let id = selectedSeason?.id { proxy.scrollTo(id) } }
            .onChange(of: selectedSeason?.id) { _, id in if let id { withAnimation { proxy.scrollTo(id) } } }
        }
    }

    private var seasonMenu: some View {
        Menu {
            // Toggle rows: the system's checkmark column, so the names line up (as the track
            // menus); what's downloaded on the second line.
            ForEach(seasons) { s in
                let n = DownloadManager.shared.seasonCounts[s.id] ?? 0
                Toggle(isOn: Binding(get: { s.id == selectedSeason?.id }, set: { on in
                    guard on, s.id != selectedSeason?.id else { return }
                    selectedSeason = s
                    Task { await loadEpisodes(selectFirst: true) }
                })) {
                    Text(s.displayTitle)
                    if n > 0 { Text("\(n) downloaded") }
                }
            }
            if DownloadManager.isSupported, offline == nil, let season = selectedSeason, !episodes.isEmpty {
                let missing = episodes.filter { $0.isPlayable && DownloadManager.shared.record(forItem: $0.id) == nil }
                Divider()
                Button {
                    let q = QualityOption.find(AppSettings.shared.downloadQuality)
                    for ep in missing { DownloadManager.shared.download(item: ep, from: source, quality: q) }
                } label: {
                    Label(missing.isEmpty ? "\(season.displayTitle) Downloaded" : "Download \(season.displayTitle) (\(missing.count))",
                          systemImage: "arrow.down.circle")
                }
                .disabled(missing.isEmpty)
            }
        } label: {
            HStack(spacing: 6) {
                Text(selectedSeason?.displayTitle ?? "Episodes").font(.title3.weight(.semibold))
                Image(systemName: "chevron.down").font(.footnote.weight(.bold))
                if let s = selectedSeason, let n = DownloadManager.shared.seasonCounts[s.id], n > 0 {
                    Label("\(n)", systemImage: "arrow.down.circle.fill")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(Theme.secondaryText)
                        .padding(.leading, 6)
                }
            }
            .foregroundStyle(.white)
        }
        .plainButton()
        .padding(.horizontal, metrics.hPadding)
    }

    /// The selected episode's place in the row: the iPad's first slot, the phone's middle.
    private var episodeAnchor: UnitPoint { metrics.regular ? .leading : .center }

    private var episodeSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            if metrics.regular {
                seasonChips
            } else {
                seasonMenu
            }

            ScrollViewReader { proxy in
                ScrollView(.horizontal, showsIndicators: false) {
                    LazyHStack(alignment: .top, spacing: 14) {
                        ForEach(episodes) { ep in
                            EpisodeCard(episode: ep, client: client,
                                        image: offline != nil ? DownloadManager.shared.record(forItem: ep.id)?.pageArtwork : nil,
                                        width: metrics.regular ? metrics.landscapeWidth : episodeCardWidth,
                                        details: metrics.regular,
                                        selected: ep.id == selectedEpisode?.id,
                                        nowPlaying: ep.id == inline.item?.id,
                                        reportFrame: { [drag] in drag.cardFrames[ep.id] = $0 }) {
                                if ep.id == inline.item?.id {
                                    withAnimation(.snappy) { selectedEpisode = ep } // already playing
                                } else if ep.id == selectedEpisode?.id || inline.model != nil {
                                    // Playing: another episode switches the video to it.
                                    play(PlaybackRequest(target: .server(source, ep)))
                                } else {
                                    withAnimation(.snappy) { selectedEpisode = ep; overviewExpanded = false }
                                }
                            }
                            // Long press: Mark as Watched / Unwatched (the server keeps it).
                            .contextMenu {
                                if offline == nil { WatchedMenuItems(item: ep, client: client) { await refreshUserData() } }
                            }
                            .id(ep.id)
                        }
                    }
                    .padding(.horizontal, metrics.regular ? 0 : metrics.hPadding)
                    .padding(.vertical, 6)
                }
                // iPad: margins (not padding), so the selected episode lands at the page margin.
                .contentMargins(.horizontal, metrics.regular ? metrics.hPadding : 0, for: .scrollContent)
                .scrollClipDisabled()
                .onChange(of: episodes.map(\.id)) { _, _ in
                    drag.cardFrames = [:]
                    if let id = selectedEpisode?.id { proxy.scrollTo(id, anchor: episodeAnchor) }
                }
                .onAppear {
                    if let id = selectedEpisode?.id { proxy.scrollTo(id, anchor: episodeAnchor) }
                }
                // iPad: the cards' size follows the window's (measured after the first layout).
                .onChange(of: metrics.width) {
                    guard let id = selectedEpisode?.id else { return }
                    Task { try? await Task.sleep(for: .milliseconds(60)); proxy.scrollTo(id, anchor: episodeAnchor) }
                }
                .onChange(of: selectedEpisode?.id) { _, id in
                    if let id { withAnimation(.snappy) { proxy.scrollTo(id, anchor: episodeAnchor) } }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    // MARK: Loading

    private func load() async {
        guard let client else { return }
        // No server: the downloads' page at once, rather than after the requests time out.
        await ConnectionRouter.shared.ready()
        if !ConnectionRouter.shared.isReachable, await loadOffline() { return }
        do {
            item = try await client.item(item.id)
            offline = nil
            guard isShow else { return }
            seasons = try await client.seasons(seriesId: item.id)
            if let sid = preselectedSeasonId, selectedSeason == nil {
                selectedSeason = seasons.first { $0.id == sid }
                // Continue in that season if that's where you are, else its first unwatched episode.
                let cont = try? await client.continueEpisode(seriesId: item.id)
                if cont?.seasonId == sid { selectedEpisode = cont }
                await loadEpisodes(selectFirst: selectedEpisode == nil)
                return
            }
            if selectedEpisode == nil {
                selectedEpisode = try? await client.continueEpisode(seriesId: item.id)
            }
            selectedSeason = seasons.first { $0.id == selectedEpisode?.seasonId }
                ?? seasons.first { $0.seasonNumber == selectedEpisode?.seasonNumber }
                ?? seasons.first
            await loadEpisodes(selectFirst: selectedEpisode == nil)
        } catch {
            if error.isCancellation { return }
            // The server went away (or doesn't answer): the downloads' page, quietly.
            if await loadOffline() { return }
            self.error = error.localizedDescription
        }
    }

    /// The page from what this title's downloads saved. False when nothing of it plays
    /// from the device.
    private func loadOffline() async -> Bool {
        guard let page = OfflineTitlePage.build(itemId: item.id, records: DownloadManager.shared.records) else { return false }
        offline = page
        error = nil
        item = page.item
        guard isShow else { return true }
        seasons = page.seasons
        // The episode it opened on (the download playing), else the first downloaded one.
        selectedSeason = page.season(of: selectedEpisode)
            ?? preselectedSeasonId.flatMap { sid in page.seasons.first { $0.id == sid } }
            ?? page.seasons.first
        await loadEpisodes(selectFirst: selectedEpisode == nil || page.season(of: selectedEpisode) == nil)
        return true
    }

    private func loadEpisodes(selectFirst: Bool) async {
        guard let season = selectedSeason else { return }
        if let offline {
            episodes = offline.episodes(season: season)
        } else {
            guard let client else { return }
            episodes = (try? await client.episodes(seriesId: item.id, seasonId: season.id)) ?? []
        }
        if selectFirst {
            selectedEpisode = episodes.first { $0.user?.played != true } ?? episodes.first
        } else if let sel = selectedEpisode, let fresh = episodes.first(where: { $0.id == sel.id }) {
            selectedEpisode = fresh // pick up overview/streams/user data from the season listing
        }
    }

    private func refreshUserData() async {
        guard let client, offline == nil else { return }
        // The server away (a download played): the page stays as it is.
        guard let fresh = try? await client.item(item.id) else { return }
        item = fresh
        if isShow { await loadEpisodes(selectFirst: false) }
    }
}

/// A section of a big page, built in its own body update and type-erased. Layout-neutral.
///
/// Built inline, the title page was one value about 30 KB big with a type nested over 50 generics
/// deep, all made in one body update: on the iPad, building it and resolving its type's metadata
/// ran out of the 1 MB main-thread stack and crashed as soon as the page opened. Each part
/// is a small view of its own whose content is made only when SwiftUI updates that part, as a
/// `GeometryReader`'s is, so the page's type stays shallow and the stack depth stays flat.
/// The content reads the page's state when it's built, as other deferred content does.
struct PagePart: View {
    private let content: () -> AnyView

    init<Content: View>(@ViewBuilder _ content: @escaping () -> Content) {
        self.content = { AnyView(content()) }
    }

    var body: some View { content() }
}

/// A state value whose every write is a change (see `ItemDetailView.item`).
struct Fresh<Value>: Equatable {
    var value: Value
    var revision = 0
    func next(_ v: Value) -> Fresh { Fresh(value: v, revision: revision + 1) }
    static func == (a: Fresh, b: Fresh) -> Bool { a.revision == b.revision }
}

/// The title page's Play button: white capsule, filled as far as it was watched.
struct PlayCapsule: View {
    let icon: String
    let title: String
    var time: Double?
    var progress: Double?

    static let height: CGFloat = 52
    /// iPad: Play's width (on the phone it spans the column).
    static let padWidth: CGFloat = 300

    var body: some View {
        ZStack(alignment: .leading) {
            Capsule().fill(Color.white.opacity(progress == nil ? 1 : 0.55))
            if let p = progress {
                GeometryReader { g in Capsule().fill(.white).frame(width: max(g.size.height, g.size.width * p)) }
            }
            HStack(spacing: 6) {
                Image(systemName: icon)
                Text(title).fontWeight(.semibold)
                if let time { Text(Fmt.clock(time)).monospacedDigit() }
            }
            .font(.headline)
            .foregroundStyle(.black)
            .frame(maxWidth: .infinity)
        }
        .frame(height: Self.height)
        .clipShape(Capsule())
    }
}

/// 16:9 episode card for the horizontal carousel; the selected one gets a white outline.
struct EpisodeCard: View {
    let episode: MediaItem
    let client: (any MediaServerClient)?
    /// Its picture from the device (a download's, with no server), instead of the server's.
    var image: URL? = nil
    let width: CGFloat
    /// iPad: runtime, air date and the synopsis under the title.
    var details = false
    let selected: Bool
    /// Playing in the page's header.
    var nowPlaying = false
    /// Gets the picture's global frame (the playing card: the video can shrink into it).
    var reportFrame: ((CGRect) -> Void)? = nil
    let action: () -> Void

    var body: some View {
        let episode = WatchState.shared.current(self.episode) // marked from its long-press menu
        Button(action: action) {
            VStack(alignment: .leading, spacing: 8) {
                ZStack(alignment: .bottom) {
                    RemoteImage(url: image ?? client?.thumbURL(episode, maxWidth: 600))
                    if let p = episode.progress, !nowPlaying { ProgressBar(value: p).padding(.horizontal, 10).padding(.bottom, 8) }
                    if nowPlaying {
                        Label("Now Playing", systemImage: "play.fill")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.white)
                            .padding(.horizontal, 10).padding(.vertical, 5)
                            .glassEffect(.regular, in: .capsule)
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    } else if selected {
                        Image(systemName: "play.fill")
                            .font(.title2)
                            .foregroundStyle(.white)
                            .frame(width: 50, height: 50)
                            .glassEffect(.regular, in: .circle)
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                }
                .frame(width: width, height: width * 9 / 16)
                .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                .hoverHighlight(cornerRadius: 16)
                .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { reportFrame?($0) }
                .overlay(alignment: .topTrailing) { WatchedMark(item: episode) }
                .overlay(alignment: .topLeading) { DownloadMark(item: episode) }
                .overlay {
                    RoundedRectangle(cornerRadius: 19, style: .continuous)
                        .strokeBorder(Color.white, lineWidth: selected ? 3 : 0)
                        .padding(-5)
                }
                Text(episode.episodeListTitle)
                    .font(.subheadline.weight(selected ? .semibold : .regular))
                    .foregroundStyle(selected || details ? .white : Theme.secondaryText)
                    .lineLimit(1)
                    .frame(width: width, alignment: .leading)
                if details {
                    VStack(alignment: .leading, spacing: 4) {
                        Text([Fmt.runtime(seconds: episode.runtime), TitleMeta.premiere(episode)].compactMap { $0 }.joined(separator: " · "))
                            .font(.caption.weight(.medium))
                            .foregroundStyle(Theme.secondaryText)
                            .lineLimit(1)
                        Text(episode.overview ?? "")
                            .font(.caption)
                            .foregroundStyle(Theme.secondaryText)
                            .lineLimit(3, reservesSpace: true)
                            .multilineTextAlignment(.leading)
                    }
                    .frame(width: width, alignment: .leading)
                    .padding(.top, -4)
                }
            }
        }
        .plainButton()
    }
}

struct CastCard: View {
    let person: MediaPerson
    let client: (any MediaServerClient)?

    var body: some View {
        VStack(spacing: 6) {
            RemoteImage(url: person.image.flatMap { client?.imageURL($0, width: 240) })
                .frame(width: 84, height: 84)
                .clipShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
                .overlay { if person.image == nil { Image(systemName: "person.fill").foregroundStyle(Theme.secondaryText) } }
            Text(person.name ?? "").font(.caption).lineLimit(2).multilineTextAlignment(.center).foregroundStyle(.white)
            Text(person.kind == "Actor" ? (person.role ?? "") : (person.kind ?? ""))
                .font(.caption2).lineLimit(2).multilineTextAlignment(.center).foregroundStyle(Theme.secondaryText)
        }
        .frame(width: 96)
    }
}

/// Rotten Tomatoes mark: a red tomato when fresh (60%+), a green splat when rotten.
private struct TomatoIcon: View {
    let fresh: Bool
    var body: some View {
        if fresh {
            Circle()
                .fill(Color(red: 0.98, green: 0.2, blue: 0.1))
                .frame(width: 18, height: 18)
                .overlay(alignment: .top) {
                    Image(systemName: "leaf.fill")
                        .font(.system(size: 8, weight: .bold))
                        .foregroundStyle(Color(red: 0.2, green: 0.6, blue: 0.15))
                        .rotationEffect(.degrees(-30))
                        .offset(y: -4)
                }
        } else {
            Image(systemName: "burst.fill")
                .font(.system(size: 18))
                .foregroundStyle(Color(red: 0.42, green: 0.73, blue: 0.2))
        }
    }
}

/// Backdrop + logo header shared by title pages and Home's banner. Both draw it at the same height
/// with the logo in the same place, so opening a title from the banner keeps the picture still.
struct TitleHeader: View {
    @Environment(\.metrics) private var metrics
    let image: URL?
    let logo: URL?
    let title: String

    static var height: CGFloat { height(Metrics()) }

    /// The header's height in a window with these metrics.
    static func height(_ m: Metrics) -> CGFloat {
        #if os(tvOS)
        return 620
        #elseif os(macOS)
        // As the iPad's: 16:9 across the window, at most 62% of its height (44% when taller than wide).
        if m.regular {
            return min(m.width * 9 / 16, m.height * (m.wide ? 0.62 : 0.44)).rounded()
        }
        return 420
        #else
        // iPad: 16:9 across the window, at most 62% of its height when it's wider than tall
        // (title and Play then sit over the picture) or 44% when it's taller.
        if m.regular {
            return min(m.width * 9 / 16, m.height * (m.wide ? 0.62 : 0.44)).rounded()
        }
        // iPhone in landscape: a wide, short band (38% of the screen's height), so the title,
        // Play and some of the page show without scrolling.
        if UIDevice.current.userInterfaceIdiom == .phone, let s = SystemControls.screen,
           let scene = UIApplication.shared.connectedScenes.first as? UIWindowScene, scene.interfaceOrientation.isLandscape {
            return (min(s.bounds.width, s.bounds.height) * 0.38).rounded()
        }
        return 440
        #endif
    }

    /// iPad wider than tall: how far the title, its line and Play reach up over the picture
    /// (the page and Home's banner pull them up by this much; the logo sits above them).
    static func overlap(_ m: Metrics, hasImage: Bool = true) -> CGFloat {
        m.wide && hasImage ? 150 : 0
    }

    #if os(iOS)
    /// Changes on rotation, so the height above is read again.
    @Environment(\.verticalSizeClass) private var verticalSizeClass
    #endif

    var body: some View {
        // No artwork: a compact header instead of a tall empty block.
        let full = Self.height(metrics)
        let height = image == nil ? full * 0.45 : full
        ZStack(alignment: .bottom) {
            Color.clear
                .frame(height: height)
                .frame(maxWidth: .infinity)
                .overlay {
                    if let image {
                        RemoteImage(url: image, immediate: true)
                    } else {
                        LinearGradient(colors: [Color.white.opacity(0.08), .clear], startPoint: .top, endPoint: .bottom)
                    }
                }
                .clipped()
            LinearGradient(colors: [.clear, Theme.background.opacity(0.6), Theme.background],
                           startPoint: .init(x: 0.5, y: 0.3), endPoint: .bottom)
            if metrics.regular {
                // iPad: the logo at the page's leading margin, over a shade that keeps the
                // title and Play readable on the picture.
                if Self.overlap(metrics, hasImage: image != nil) > 0 {
                    LinearGradient(colors: [Theme.background.opacity(0.8), Theme.background.opacity(0.35), .clear],
                                   startPoint: .leading, endPoint: .init(x: 0.65, y: 0.5))
                }
                Group {
                    if let logo {
                        RemoteImage(url: logo, contentMode: .fit, alignment: .bottomLeading, immediate: true)
                            .frame(maxWidth: 380, maxHeight: min(130, height * 0.3))
                    } else {
                        Text(title).font(.largeTitle.weight(.bold)).multilineTextAlignment(.leading)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, metrics.hPadding)
                .padding(.bottom, 6 + Self.overlap(metrics, hasImage: image != nil) + (Self.overlap(metrics, hasImage: image != nil) > 0 ? 14 : 0))
            } else {
                Group {
                    if let logo {
                        RemoteImage(url: logo, contentMode: .fit, immediate: true).frame(maxWidth: 300, maxHeight: min(110, height * 0.45))
                    } else {
                        Text(title).font(.largeTitle.weight(.bold)).multilineTextAlignment(.center)
                            .padding(.horizontal, metrics.hPadding)
                    }
                }
                .padding(.bottom, 6)
            }
        }
        // Fixed height: given spare room the gradient would grow and shift the picture down,
        // so the banner and the title page would no longer line up.
        .frame(height: height)
        .foregroundStyle(.white)
    }
}

/// Runtime · date · rating · tech badges, then genres. `focus` is the selected episode on show pages.
struct TitleMeta: View {
    let item: MediaItem
    var focus: MediaItem? = nil
    /// "Downloaded" under the line (title pages say it under Play instead).
    var downloadTag = true
    /// iPad: at the page's leading margin.
    var alignment: HorizontalAlignment = .center

    var body: some View {
        let f = focus ?? item
        VStack(alignment: alignment, spacing: 6) {
            HStack(spacing: 10) {
                if let r = Fmt.runtime(seconds: f.runtime) { Text(r) }
                if let d = Self.premiere(f) { Text(d) }
                else if let y = item.productionYear { Text(String(y)) }
                if let r = item.officialRating ?? f.officialRating { Badge(text: r) }
                ForEach(f.techBadges.prefix(3), id: \.self) { Badge(text: $0) }
            }
            .font(.subheadline)
            .foregroundStyle(Theme.secondaryText)
            .lineLimit(1)
            if let g = item.genres, !g.isEmpty {
                Text(g.prefix(3).joined(separator: ", ")).font(.subheadline).foregroundStyle(Theme.secondaryText)
            }
            if downloadTag {
                DownloadedTag(item: f.isEpisode ? f : item)
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(.white.opacity(0.85))
            }
        }
    }

    static func premiere(_ i: MediaItem?) -> String? {
        // The date part is enough.
        guard let s = i?.premiereDate?.prefix(10),
              let d = try? Date(String(s), strategy: .iso8601.year().month().day()) else { return nil }
        return d.formatted(date: .abbreviated, time: .omitted)
    }
}

/// iPad: the title's facts under the page, in columns: who made it, where it's from, and what the
/// file holds (picture, sound and subtitle languages, size). Only what the server sends; a fact
/// it doesn't know is left out. Shows: the show's facts, then the selected episode's file.
struct TitleDetails: View {
    let item: MediaItem
    /// The selected episode on a show's page.
    var focus: MediaItem?
    var seasons = 0
    @Environment(\.metrics) private var metrics

    private struct Fact: Identifiable {
        let label: String
        let value: String
        var id: String { label }
    }

    private func names(_ kind: String, in item: MediaItem) -> String? {
        var n: [String] = []
        for p in item.people ?? [] where p.kind == kind { if let name = p.name, !n.contains(name) { n.append(name) } }
        return n.isEmpty ? nil : n.prefix(3).joined(separator: ", ")
    }

    private static func languages(_ tracks: [TrackInfo]) -> String? {
        var seen: [String] = []
        for t in tracks where !seen.contains(t.audioName) { seen.append(t.audioName) }
        guard !seen.isEmpty else { return nil }
        return seen.count > 6 ? seen.prefix(6).joined(separator: ", ") + " +\(seen.count - 6)" : seen.joined(separator: ", ")
    }

    private var facts: [Fact] {
        var out: [Fact] = []
        func add(_ label: String, _ value: String?) { if let value, !value.isEmpty { out.append(Fact(label: label, value: value)) } }
        let file = item.isSeries ? focus : item
        if item.isSeries {
            add(item.studios.map { $0.count > 1 ? "Networks" : "Network" } ?? "Network", item.studios?.prefix(3).joined(separator: ", "))
            add("Seasons", seasons > 0 ? "\(seasons)" : nil)
            add("First Aired", TitleMeta.premiere(item))
        } else {
            add(item.people?.filter { $0.kind == "Director" }.count ?? 0 > 1 ? "Directors" : "Director", names("Director", in: item))
            add(item.people?.filter { $0.kind == "Writer" }.count ?? 0 > 1 ? "Writers" : "Writer", names("Writer", in: item))
            add(item.studios.map { $0.count > 1 ? "Studios" : "Studio" } ?? "Studio", item.studios?.prefix(3).joined(separator: ", "))
            add("Released", TitleMeta.premiere(item))
        }
        add("Genres", item.genres?.prefix(4).joined(separator: ", "))
        add("Rated", item.officialRating ?? file?.officialRating)
        if let file {
            let chosen = VersionChoices.shared.version(for: file) ?? file.versions?.first
            let tracks = chosen?.tracks ?? file.tracks ?? []
            let prefix = item.isSeries ? "Episode " : ""
            if let v = tracks.first(where: { $0.kind == .video }) {
                add(prefix + "Video", v.displayTitle ?? [v.height.map { "\($0)p" }, v.codec?.uppercased(), v.videoRange].compactMap { $0 }.joined(separator: " "))
            }
            add(prefix + "Audio", Self.languages(tracks.filter { $0.kind == .audio }))
            add(prefix + "Subtitles", Self.languages(tracks.filter { $0.kind == .subtitle }))
            if let version = chosen {
                let parts = [file.container?.uppercased(), version.size.map { Fmt.bytes($0) },
                             version.bitrate.map { String(format: "%.1f Mbps", Double($0) / 1_000_000) }].compactMap { $0 }
                add(prefix + "File", parts.isEmpty ? nil : parts.joined(separator: " · "))
            }
        }
        return out
    }

    var body: some View {
        let facts = self.facts
        if !facts.isEmpty {
            VStack(alignment: .leading, spacing: 12) {
                SectionHeader(title: "Details")
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 230), spacing: 24, alignment: .topLeading)],
                          alignment: .leading, spacing: 16) {
                    ForEach(facts) { f in
                        VStack(alignment: .leading, spacing: 3) {
                            Text(f.label).font(.subheadline.weight(.semibold)).foregroundStyle(.white)
                            Text(f.value).font(.subheadline).foregroundStyle(Theme.secondaryText)
                                .lineLimit(3)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
                .padding(.horizontal, metrics.hPadding)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// A title block and a button just after its text (its widest line), centred on the block;
/// the title wraps before it pushes the button out of the row.
struct TitleAccessoryRow: Layout {
    var spacing: CGFloat

    /// Where the title (`title`: its size in the room the button leaves) and the button go:
    /// the button `spacing` after the title's text, both centred on the taller.
    static func frames(title: CGSize, accessory: CGSize, spacing: CGFloat) -> (title: CGRect, accessory: CGRect) {
        let h = max(title.height, accessory.height)
        return (CGRect(x: 0, y: (h - title.height) / 2, width: title.width, height: title.height),
                CGRect(x: title.width + spacing, y: (h - accessory.height) / 2, width: accessory.width, height: accessory.height))
    }

    private func sizes(_ proposal: ProposedViewSize, _ subviews: Subviews) -> (CGSize, CGSize) {
        guard subviews.count == 2 else { return (.zero, .zero) }
        let accessory = subviews[1].sizeThatFits(.unspecified)
        let room = proposal.width.map { max(0, $0 - accessory.width - spacing) }
        let title = subviews[0].sizeThatFits(ProposedViewSize(width: room, height: nil))
        return (title, accessory)
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let (title, accessory) = sizes(proposal, subviews)
        let f = Self.frames(title: title, accessory: accessory, spacing: spacing)
        return CGSize(width: proposal.width ?? f.accessory.maxX, height: max(f.title.maxY, f.accessory.maxY))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard subviews.count == 2 else { return }
        let (title, accessory) = sizes(ProposedViewSize(width: bounds.width, height: bounds.height), subviews)
        let f = Self.frames(title: title, accessory: accessory, spacing: spacing)
        subviews[0].place(at: CGPoint(x: bounds.minX + f.title.minX, y: bounds.minY + f.title.minY),
                          proposal: ProposedViewSize(f.title.size))
        subviews[1].place(at: CGPoint(x: bounds.minX + f.accessory.minX, y: bounds.minY + f.accessory.minY),
                          proposal: ProposedViewSize(f.accessory.size))
    }
}
