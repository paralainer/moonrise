#if os(tvOS)
import SwiftUI

// Apple TV screens, following Apple's tvOS media catalog patterns (WWDC24 "Migrate your TVML app
// to SwiftUI"): a full-screen backdrop with a material gradient, a showcase above the fold,
// shelves of system lockups (borderless buttons with a highlight hover effect), and Liquid Glass
// buttons (TVGlassButtonStyle) on product pages. The iPhone layouts don't translate to a 10-foot screen.

// MARK: - Backdrop

/// Full-screen artwork with a material gradient: clear at the top above the fold, fully blurred
/// below it (the Apple TV app's product-page look).
struct TVBackdrop: View {
    let url: URL?
    let belowFold: Bool
    /// The edge a new image slides in from (paging the showcase); nil cross-fades.
    var slideFrom: Edge?

    var body: some View {
        ZStack {
            Theme.background
            if let url {
                RemoteImage(url: url, immediate: true)
                    .id(url)
                    .transition(slideFrom.map { .push(from: $0) } ?? .opacity)
            }
        }
        .overlay {
            Rectangle()
                .fill(.regularMaterial)
                .mask {
                    LinearGradient(stops: [
                        .init(color: .black, location: 0.2),
                        .init(color: .black.opacity(belowFold ? 1 : 0.5), location: 0.4),
                        .init(color: .black.opacity(belowFold ? 1 : 0), location: 0.6),
                    ], startPoint: .bottom, endPoint: .top)
                }
        }
        .overlay {
            // Keep text legible on bright artwork: one light scrim (stacked ones made every
            // backdrop far too dark); text over bright artwork gets a shadow instead.
            LinearGradient(colors: [.black.opacity(0.55), .black.opacity(0.1), .clear],
                           startPoint: .bottomLeading, endPoint: .topTrailing)
        }
        .animation(.easeInOut(duration: 0.4), value: belowFold)
        .animation(.easeInOut(duration: 0.5), value: url)
        .ignoresSafeArea()
    }
}

// MARK: - Lockups

/// 16:9 lockup: artwork with progress, title and subtitle underneath (they move aside on focus).
/// The text takes the artwork's width, so a long title truncates instead of widening the lockup.
struct TVLandscapeLockup: View {
    let item: MediaItem
    let client: (any MediaServerClient)?
    var count = 4

    var body: some View {
        let item = WatchState.shared.current(self.item) // marked from a long-press menu
        ZStack(alignment: .bottom) {
            RemoteImage(url: client?.thumbURL(item))
            if let p = item.progress { ProgressBar(value: p).padding(.horizontal, 14).padding(.bottom, 12) }
        }
        .aspectRatio(16 / 9, contentMode: .fit)
        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(alignment: .topTrailing) { WatchedMark(item: item) }
        .overlay(alignment: .topLeading) { DownloadMark(item: item) }
        .hoverEffect(.highlight)
        .containerRelativeFrame(.horizontal, count: count, spacing: 40)
        VStack(alignment: .leading, spacing: 2) {
            Text(item.isEpisode ? (item.seriesName ?? item.displayTitle) : item.displayTitle)
                .font(.callout.weight(.medium))
            Text(item.isEpisode ? [item.episodeLabel, item.name].compactMap { $0 }.joined(separator: " · ")
                 : (item.productionYear.map(String.init) ?? " "))
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .lineLimit(1)
        .containerRelativeFrame(.horizontal, count: count, spacing: 40, alignment: .leading)
    }
}

/// 2:3 poster lockup with the title underneath.
struct TVPosterLockup: View {
    let item: MediaItem
    let client: (any MediaServerClient)?
    var count = 7

    var body: some View {
        let item = WatchState.shared.current(self.item) // marked from a long-press menu
        ZStack(alignment: .bottom) {
            RemoteImage(url: client?.posterURL(item, maxWidth: 500))
                .overlay {
                    if client?.posterURL(item) == nil {
                        Image(systemName: item.isSeries ? "tv" : "film")
                            .font(.system(size: 60, weight: .light))
                            .foregroundStyle(.secondary)
                    }
                }
            if let p = item.progress { ProgressBar(value: p).padding(10) }
        }
        .aspectRatio(2 / 3, contentMode: .fit)
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(alignment: .topTrailing) { WatchedMark(item: item) }
        .overlay(alignment: .topLeading) { DownloadMark(item: item) }
        .hoverEffect(.highlight)
        .containerRelativeFrame(.horizontal, count: count, spacing: 40)
        Text((item.isSeason || item.isEpisode) ? (item.seriesName ?? item.displayTitle) : item.displayTitle)
            .font(.caption.weight(.medium))
            .lineLimit(1)
            .containerRelativeFrame(.horizontal, count: count, spacing: 40, alignment: .leading)
    }
}

/// A titled horizontal shelf of lockups.
struct TVShelf<Content: View>: View {
    let title: String
    /// Keep this lockup (its `.id`) scrolled into view, so focus entering the shelf can land on it.
    var scrollTo: String? = nil
    @ViewBuilder let content: () -> Content

    var body: some View {
        // 32 pt under the title: a focused lockup grows ~20 pt upwards without reaching it.
        VStack(alignment: .leading, spacing: 32) {
            Text(title).font(.headline).foregroundStyle(.secondary)
            ScrollViewReader { proxy in
                ScrollView(.horizontal) {
                    LazyHStack(alignment: .top, spacing: 40) { content() }
                }
                .scrollClipDisabled()
                .scrollIndicators(.hidden)
                .buttonStyle(.borderless)
                .onAppear { if let scrollTo { proxy.scrollTo(scrollTo, anchor: .leading) } }
                .onChange(of: scrollTo) { _, id in if let id { proxy.scrollTo(id, anchor: .leading) } }
            }
        }
        .focusSection()
    }
}

// MARK: - Home

struct TVHomeView: View {
    @Environment(PlaybackCoordinator.self) private var playback
    @Environment(\.scenePhase) private var scenePhase
    @State private var heroIndex = 0
    /// Which way the showcase last paged, so titles and artwork slide the matching way.
    @State private var heroForward = true
    @State private var belowFold = false
    /// Background follows the focused lockup below the fold.
    @State private var focusedBackdrop: URL?
    @State private var backdropTask: Task<Void, Never>?
    @FocusState private var showcaseFocus: ShowcaseFocus?
    private enum ShowcaseFocus { case card, previous, next }
    private var showcaseFocused: Bool { showcaseFocus != nil }
    @FocusState private var focusedLockup: String?
    /// The Continue Watching card just removed from its menu: the focus engine may drop focus with
    /// its view before `home.sections` changes, so keep its key to move focus to its neighbour.
    @State private var removedLockup: String?
    /// Home is the page on screen (no title page pushed, its tab selected).
    @State private var onScreen = false
    private var home: HomeStore { HomeStore.shared }
    private var store: SourceStore { SourceStore.shared }
    private var router: ConnectionRouter { ConnectionRouter.shared }

    private var client: (any MediaServerClient)? { store.client }

    private var hero: MediaItem? {
        home.hero.isEmpty ? nil : home.hero[heroPosition]
    }

    /// The showcase's page (0 with no titles: signing out empties them under a live view).
    private var heroPosition: Int {
        home.hero.isEmpty ? 0 : heroIndex % home.hero.count
    }

    var body: some View {
        ZStack {
            TVBackdrop(url: belowFold ? (focusedBackdrop ?? heroBackdrop) : heroBackdrop, belowFold: belowFold,
                       slideFrom: belowFold ? nil : heroForward ? .trailing : .leading)
            ScrollView(.vertical) {
                VStack(alignment: .leading, spacing: 70) {
                    showcase
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .focusSection()
                        .containerRelativeFrame(.vertical, alignment: .bottomLeading) { h, _ in h * 0.86 }
                        .onScrollVisibilityChange(threshold: 0.5) { visible in
                            withAnimation { belowFold = !visible }
                        }
                    if let problem = router.problem {
                        ConnectionProblemView(problem: problem) { home.reload() }
                    }
                    ForEach(home.sections) { section in
                        shelf(section)
                            // The banner is full width, so left alone the focus engine picks the
                            // card nearest its centre. Only moves from the banner: Up from a lower
                            // row and the way back from a title page keep theirs.
                            .defaultFocus($focusedLockup, showcaseFocused ? HomeFocus.firstLockup(home.sections) : nil,
                                          priority: .userInitiated)
                    }
                }
                .padding(.bottom, 80)
            }
            .scrollClipDisabled()
        }
        .task { home.reload() }
        // Back in the app, or back from a title page: load again, so what another device played
        // shows up here and on the Top Shelf. Unchanged rows leave focus where it is.
        .onChange(of: scenePhase) { _, phase in
            if phase == .active { home.refreshIfStale() }
        }
        .onAppear { onScreen = true; home.refreshIfStale() }
        .onDisappear { onScreen = false }
        .onChange(of: home.sections) { old, new in
            // A refresh can take away the focused card (another device finished it, or removed it).
            if let target = HomeFocus.target(focused: focusedLockup, removed: removedLockup, old: old, new: new) {
                keepFocus(on: target)
            }
            if let removed = removedLockup, !HomeFocus.lockups(new).contains(where: { $0.id == removed }) {
                removedLockup = nil
            }
        }
        .onChange(of: focusedLockup) { _, key in
            // The backdrop follows the focused title below the fold.
            guard let key, let item = HomeFocus.lockups(home.sections).first(where: { $0.id == key })?.item else { return }
            let page = item.isEpisode || item.isSeason ? ItemDetailView.showStub(for: item) : item
            let url = client?.backdropURL(page, maxWidth: 1920)
            // Only once focus rests on a title: sweeping along a row keeps the current picture.
            backdropTask?.cancel()
            backdropTask = Task {
                try? await Task.sleep(for: .milliseconds(900))
                guard !Task.isCancelled else { return }
                focusedBackdrop = url
            }
        }
        .onChange(of: router.route) { old, new in
            if router.isReachable, old != new { home.reload() }
        }
        .onReceive(NotificationCenter.default.publisher(for: .watchStateDidChange)) { _ in home.reload() }
        .onReceive(NotificationCenter.default.publisher(for: .playbackDidEnd)) { _ in
            Task { try? await Task.sleep(for: .seconds(1)); home.reload() }
        }
        // Rotate the showcase while nobody is looking at it below the fold or pressing its buttons.
        .task(id: belowFold || showcaseFocused) {
            guard !belowFold, !showcaseFocused else { return }
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(9))
                guard !Task.isCancelled else { continue }
                page(by: 1)
            }
        }
    }

    private var heroBackdrop: URL? {
        hero.flatMap { client?.backdropURL($0, maxWidth: 1920) }
    }

    @ViewBuilder private var showcase: some View {
        if let item = hero {
            // The showcase is one focusable card: left/right pages through it (like the TV app),
            // select opens the title's page, press and hold offers Play.
            HStack(spacing: 0) {
                pageStop(.previous)
                NavigationLink(value: Route.item(item)) {
                    VStack(alignment: .leading, spacing: 22) {
                        VStack(alignment: .leading, spacing: 22) {
                            if let logo = client?.logoURL(item) {
                                RemoteImage(url: logo, contentMode: .fit, alignment: .leading, immediate: true)
                                    .frame(width: 560, height: 200, alignment: .leading)
                            } else {
                                Text(item.displayTitle).font(.system(size: 76, weight: .bold)).lineLimit(2)
                            }
                            TVMetaLine(item: item)
                            if let text = item.overview {
                                // Focus shows as the text brightening (and the dots): the card has no platter.
                                Text(text).font(.callout).foregroundStyle(showcaseFocused ? .primary : .secondary)
                                    .lineLimit(3).frame(maxWidth: 900, alignment: .leading)
                            }
                        }
                        .id(item.id)
                        .transition(.push(from: heroForward ? .trailing : .leading))
                        pageDots
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(ShowcaseButtonStyle())
                .focused($showcaseFocus, equals: .card)
                .contextMenu {
                    Button { Task { await playHero(item) } } label: {
                        Label(item.resumeSeconds != nil ? "Resume" : "Play", systemImage: "play.fill")
                    }
                    WatchedMenuItems(item: item, client: client)
                }
                pageStop(.next)
            }
        } else if home.loading {
            ProgressView().frame(maxWidth: .infinity)
        } else if HomeStore.isEmpty(answered: home.answered, loading: home.loading,
                                    failed: home.failure != nil || router.problem != nil, rows: home.sections.count) {
            EmptyHomeView().frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    /// Invisible focus stop on either side of the card. Moving onto it pages the showcase and
    /// puts focus back on the card. A move command alone isn't enough: swiping on the remote's
    /// touch surface only moves focus, it sends no move command when focus has nowhere to go.
    private func pageStop(_ stop: ShowcaseFocus) -> some View {
        Color.clear
            .frame(width: 1, height: 200)
            .focusable(home.hero.count > 1)
            .focused($showcaseFocus, equals: stop)
            .onChange(of: showcaseFocus == stop) { _, landed in
                guard landed else { return }
                page(by: stop == .next ? 1 : -1)
                showcaseFocus = .card
            }
    }

    private func page(by step: Int) {
        guard home.hero.count > 1 else { return }
        // Set the direction first, so the outgoing title also leaves the right way.
        heroForward = step > 0
        Task { @MainActor in
            let n = home.hero.count
            guard n > 0 else { return }
            withAnimation(.smooth(duration: 0.5)) {
                heroIndex = (heroIndex + step + n) % n
            }
        }
    }

    @ViewBuilder private var pageDots: some View {
        if home.hero.count > 1 {
            HStack(spacing: 10) {
                ForEach(home.hero.indices, id: \.self) { i in
                    Capsule()
                        .fill(.white.opacity(i == heroPosition ? (showcaseFocused ? 1 : 0.7) : showcaseFocused ? 0.55 : 0.25))
                        .frame(width: i == heroPosition ? 28 : 10, height: 10)
                }
            }
            .animation(.easeInOut, value: heroIndex)
        }
    }

    /// Never leave Home with nothing focused: when a refresh takes away the focused lockup's view,
    /// focus its card again (same thread) or the one now in its place. Only while Home is on screen
    /// and nothing else took focus meanwhile.
    private func keepFocus(on target: String) {
        Task { @MainActor in
            // Let the focus engine settle on the new views first. While a context menu is still
            // closing (Remove from Continue Watching) the request is ignored, so try a few times.
            for _ in 0..<5 {
                try? await Task.sleep(for: .milliseconds(300))
                guard onScreen, showcaseFocus == nil else { return }
                if let now = focusedLockup, HomeFocus.lockups(home.sections).contains(where: { $0.id == now }) { return }
                focusedLockup = target
            }
        }
    }

    private func shelf(_ section: HomeSection) -> some View {
        TVShelf(title: section.title) {
            ForEach(HomeFocus.lockups(section)) { lockup in
                let item = lockup.item
                NavigationLink(value: Route.item(item)) {
                    if section.style == .landscape {
                        TVLandscapeLockup(item: item, client: client)
                    } else {
                        TVPosterLockup(item: item, client: client)
                    }
                }
                .focused($focusedLockup, equals: lockup.id)
                .contextMenu {
                    Button { play(item) } label: {
                        Label(item.resumeSeconds != nil ? "Resume" : "Play", systemImage: "play.fill")
                    }
                    WatchedMenuItems(item: item, client: client)
                    if section.id == HomeSection.continueID {
                        Button(role: .destructive) { removeFromContinue(lockup) } label: {
                            Label("Remove from Continue Watching", systemImage: "minus.circle")
                        }
                    }
                }
            }
        }
    }

    /// Hidden here at once; the server hides it on the other devices. The reloads also update the
    /// Top Shelf.
    private func removeFromContinue(_ lockup: HomeFocus.Lockup) {
        guard let server = store.server else { return }
        removedLockup = lockup.id
        RecentsStore.remove(source: server.id, titleId: lockup.item.titleId)
        home.reload()
        Task {
            try? await client?.removeFromContinue(lockup.item.id)
            home.reload()
        }
    }

    /// Movies play directly; shows continue where you are (the server's resume / next up).
    private func playHero(_ item: MediaItem) async {
        if item.isSeries {
            guard let ep = try? await client?.continueEpisode(seriesId: item.id) else { return }
            play(ep)
        } else {
            play(item)
        }
    }

    private func play(_ item: MediaItem) {
        guard let server = store.server else { return }
        playback.play(PlaybackRequest(target: .server(server, item)))
    }
}

/// The showcase card has no platter: focus shows as a slight lift, brighter overview and page dots.
private struct ShowcaseButtonStyle: ButtonStyle {
    @Environment(\.isFocused) private var focused

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.98 : focused ? 1.03 : 1, anchor: .leading)
            .animation(.easeOut(duration: 0.2), value: focused)
    }
}

/// Year/runtime/rating/badges on one line, left aligned (TV product pages read left to right).
struct TVMetaLine: View {
    let item: MediaItem
    var body: some View {
        HStack(spacing: 14) {
            if let r = Fmt.runtime(seconds: item.runtime) { Text(r) }
            if let d = TitleMeta.premiere(item) { Text(d) } else if let y = item.productionYear { Text(String(y)) }
            if let r = item.officialRating { Badge(text: r) }
            ForEach(item.techBadges.prefix(4), id: \.self) { Badge(text: $0) }
            if let g = item.genres?.prefix(3), !g.isEmpty { Text(g.joined(separator: ", ")) }
            DownloadedTag(item: item)
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .lineLimit(1)
    }
}

// MARK: - Product page

struct TVItemDetailView: View {
    let source: Source
    @Environment(PlaybackCoordinator.self) private var playback
    @State private var item: MediaItem
    @State private var seasons: [MediaItem] = []
    @State private var selectedSeason: MediaItem?
    @State private var episodes: [MediaItem] = []
    /// The episode the page is about (Play/Resume target): the opened one, else continue/next up.
    @State private var current: MediaItem?
    @State private var belowFold = false
    @State private var showOverview = false
    @FocusState private var focusedEpisode: String?
    @FocusState private var actionFocus: ActionFocus?
    @FocusState private var seasonFocus: String?
    @State private var seasonSwitch: Task<Void, Never>?
    private enum ActionFocus { case play, restart, version, more }
    private let preselectedSeasonId: String?
    /// The item the page was opened with (an episode opens its show's page).
    private let openedId: String

    init(source: Source, item: MediaItem) {
        self.source = source
        openedId = item.id
        let inShow = item.isEpisode || item.isSeason
        _item = State(initialValue: inShow ? ItemDetailView.showStub(for: item) : item)
        _current = State(initialValue: item.isEpisode ? item : nil)
        preselectedSeasonId = item.isSeason ? item.id : nil
    }

    private var client: (any MediaServerClient)? { MediaServers.client(for: source) }
    private var isShow: Bool { item.isSeries }
    private var focus: MediaItem? { (isShow ? current : item).map { versionDetails.full($0).asChosenVersion } }
    /// Episodes from a list with several versions: their files, loaded for the version picker.
    @State private var versionDetails = VersionDetails()

    var body: some View {
        ZStack {
            TVBackdrop(url: client?.backdropURL(item, maxWidth: 1920), belowFold: belowFold)
            ScrollView(.vertical) {
                VStack(alignment: .leading, spacing: 70) {
                    showcase
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .focusSection()
                        // The page opens on Play, and coming back up from the seasons lands on
                        // it, as in the TV app.
                        .defaultFocus($actionFocus, .play, priority: .userInitiated)
                        .containerRelativeFrame(.vertical, alignment: .bottomLeading) { h, _ in h * 0.9 }
                        .onScrollVisibilityChange(threshold: 0.5) { visible in withAnimation { belowFold = !visible } }
                    if isShow { seasonsAndEpisodes }
                    cast
                }
                .padding(.bottom, 80)
            }
            .scrollClipDisabled()
        }
        .toolbar(.hidden, for: .tabBar)
        .task { await load() }
        .task {
            // Opened by a deep link (the Top Shelf): take focus, which may still be on the tab bar.
            guard AppNavigation.shared.focusOnOpen == openedId else { return }
            AppNavigation.shared.focusOnOpen = nil
            try? await Task.sleep(for: .milliseconds(250))
            actionFocus = .play
        }
        .onChange(of: focusedEpisode) { _, id in
            if let id, let ep = episodes.first(where: { $0.id == id }) { current = ep }
        }
        // Moving onto a season shows its episodes, no click needed (as in the TV app). Debounced,
        // so sweeping across the row doesn't load every season on the way.
        .onChange(of: seasonFocus) { _, id in
            seasonSwitch?.cancel()
            guard let id, id != selectedSeason?.id, let s = seasons.first(where: { $0.id == id }) else { return }
            seasonSwitch = Task {
                try? await Task.sleep(for: .milliseconds(350))
                guard !Task.isCancelled else { return }
                selectedSeason = s
                await loadEpisodes(selectFirst: true)
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .playbackDidEnd)) { _ in
            Task { try? await Task.sleep(for: .seconds(1)); await refresh() }
        }
        .fullScreenCover(isPresented: $showOverview) {
            ScrollView {
                VStack(alignment: .leading, spacing: 30) {
                    Text(focus?.displayTitle ?? item.displayTitle).font(.title2.bold())
                    Text((focus?.overview ?? item.overview) ?? "").font(.body)
                }
                .frame(maxWidth: 1100, alignment: .leading)
                .padding(80)
            }
            .frame(maxWidth: .infinity)
            .background(.regularMaterial)
        }
    }

    // MARK: Above the fold

    private var showcase: some View {
        VStack(alignment: .leading, spacing: 26) {
            if let logo = client?.logoURL(item) {
                RemoteImage(url: logo, contentMode: .fit, alignment: .leading, immediate: true)
                    .frame(width: 600, height: 210, alignment: .leading)
            } else {
                Text(item.displayTitle).font(.system(size: 76, weight: .bold)).lineLimit(2)
            }
            if isShow, let e = current {
                Text([e.episodeLabel, e.name].compactMap { $0 }.joined(separator: " · "))
                    .font(.title3.weight(.semibold))
            }
            TVMetaLine(item: focus ?? item)
                // (Another version chosen: its badges; the item compares equal.)
                .id(focus.flatMap { VersionChoices.shared.version(for: $0)?.id })

            HStack(alignment: .top, spacing: 60) {
                VStack(alignment: .leading, spacing: 0) {
                    actions
                    // The version that plays, below the row (clear of the focused button's name).
                    if let target = focus, target.hasVersions, let v = VersionPlay(target).version {
                        (Text("Version: ").foregroundStyle(.secondary) + Text(PlayerModel.versionTitle(v)))
                            .font(.caption)
                            .lineLimit(1)
                            .padding(.top, 64)
                    }
                }
                .frame(width: 600, alignment: .leading)
                if let text = focus?.overview ?? item.overview {
                    // Selectable so the full text can be read (Apple TV product-page pattern).
                    Button { showOverview = true } label: {
                        VStack(alignment: .leading, spacing: 14) {
                            Text(text).font(.callout).lineLimit(6).multilineTextAlignment(.leading)
                            if let credit = DemoClient.credit(item.id) {
                                Text(credit).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                        .frame(maxWidth: 820, alignment: .leading)
                    }
                    .buttonStyle(TVGlassRowStyle(scale: 1.03))
                }
                credits
            }
        }
    }

    /// The TV app's action row: one wide Play/Resume button, then round glass buttons for the
    /// rest — Play from Beginning (with a resume position) and More (Mark as Watched/Unwatched).
    /// A focused round button names itself underneath.
    @ViewBuilder private var actions: some View {
        if let target = focus, target.isPlayable, let problem = target.unplayableProblem {
            // No file can play (damaged, or gone from disk): no Play, what's wrong instead.
            DamagedTitleNote(problem: problem)
                .frame(maxWidth: 820, alignment: .leading)
        } else if let target = focus, target.isPlayable {
            // Several versions: the chosen one plays, from its own resume point.
            let vp = VersionPlay(target)
            HStack(spacing: 24) {
                Button {
                    playback.play(PlaybackRequest(target: .server(source, target), startAt: vp.version != nil ? (vp.resume ?? 0) : nil,
                                                  versionId: vp.version?.id))
                } label: {
                    Label(vp.resume.map { "Resume \(Fmt.clock($0))" } ?? "Play", systemImage: "play.fill")
                        .font(.body.bold())
                        .lineLimit(1)
                        .frame(minWidth: 300)
                }
                .tvGlassButton(prominent: true)
                .focused($actionFocus, equals: .play)
                if vp.resume != nil {
                    Button { playback.play(PlaybackRequest(target: .server(source, target), startAt: 0, versionId: vp.version?.id)) } label: {
                        Image(systemName: "arrow.counterclockwise")
                    }
                    .tvGlassButton(.circle)
                    .accessibilityLabel("Play from Beginning")
                    .focused($actionFocus, equals: .restart)
                    .overlay(alignment: .bottom) { actionCaption("Play from Beginning", .restart) }
                }
                if target.hasVersions, let v = vp.version {
                    // The versions (cuts, qualities, dubs): the one playing checked.
                    Menu {
                        Section("Version") { VersionPickerRows(item: target, source: source) }
                    } label: {
                        Image(systemName: "square.stack")
                    }
                    .tvGlassButton(.circle)
                    .accessibilityLabel("Version")
                    .focused($actionFocus, equals: .version)
                    .overlay(alignment: .bottom) { actionCaption("Version", .version) }
                }
                Menu {
                    ForEach(WatchAction.options(for: target)) { action in
                        Button { Task { await setPlayed(target, action.played) } } label: {
                            Label(action.title, systemImage: action.icon)
                        }
                    }
                } label: {
                    Image(systemName: "ellipsis")
                }
                .tvGlassButton(.circle)
                .accessibilityLabel("More")
                .focused($actionFocus, equals: .more)
                .overlay(alignment: .bottom) { actionCaption("More", .more) }
            }
            .task(id: VersionDetails.key(target)) { await versionDetails.load(target, client: client) }
        }
    }

    /// The focused round button's name, under it (outside the layout, so nothing moves).
    private func actionCaption(_ text: String, _ which: ActionFocus) -> some View {
        Text(text)
            .font(.caption2.weight(.semibold))
            .fixedSize()
            .opacity(actionFocus == which ? 1 : 0)
            .offset(y: 54)
            .animation(.easeOut(duration: 0.15), value: actionFocus)
            .allowsHitTesting(false)
    }

    @ViewBuilder private var credits: some View {
        let people = item.people ?? []
        let actors = people.filter { $0.kind == "Actor" }.prefix(4).compactMap(\.name)
        let directors = people.filter { $0.kind == "Director" }.prefix(2).compactMap(\.name)
        // The film's or show's (as the cast is): TMDB's star, then IMDb, Rotten Tomatoes and
        // Metacritic as the server has them.
        let rating = ExternalRatings.line(tmdb: item.communityRating, item.ratings ?? ExternalRatings())
        if !actors.isEmpty || !directors.isEmpty {
            VStack(alignment: .leading, spacing: 16) {
                if !actors.isEmpty { creditLine("Starring", actors.joined(separator: ", ")) }
                if !directors.isEmpty { creditLine("Director", directors.joined(separator: ", ")) }
                if let rating { creditLine("Rating", rating) }
            }
            .frame(maxWidth: 380, alignment: .leading)
        }
    }

    private func creditLine(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            Text(value).font(.caption).lineLimit(3)
        }
    }

    // MARK: Below the fold

    private var seasonsAndEpisodes: some View {
        VStack(alignment: .leading, spacing: 30) {
            if seasons.count > 1 {
                ScrollView(.horizontal) {
                    HStack(spacing: 20) {
                        ForEach(seasons) { s in
                            Button {
                                selectedSeason = s
                                Task { await loadEpisodes(selectFirst: true) }
                            } label: {
                                // Only the season shown sits on glass (bold); the others are bare text.
                                HStack(spacing: 10) {
                                    Text(s.displayTitle)
                                    if let n = DownloadManager.shared.seasonCounts[s.id], n > 0 {
                                        Label("\(n)", systemImage: "arrow.down.circle.fill").font(.caption)
                                    }
                                }
                                .fontWeight(s.id == selectedSeason?.id ? .bold : .regular)
                            }
                            .tvGlassButton(height: 66, selected: s.id == selectedSeason?.id, bare: true)
                            .focused($seasonFocus, equals: s.id)
                            // Long press: the whole season watched / unwatched.
                            .contextMenu {
                                WatchedMenuItems(item: s, client: client) { await reloadSeasons() }
                            }
                        }
                    }
                }
                .scrollClipDisabled()
                .focusSection()
                // Coming down from the page's buttons (or up from the episodes): the season shown.
                .defaultFocus($seasonFocus, selectedSeason?.id, priority: .userInitiated)
            }
            // Coming down into the episodes lands on the current one (next up), not the first.
            TVShelf(title: seasons.count > 1 ? "Episodes" : (selectedSeason?.displayTitle ?? "Episodes"),
                    scrollTo: focusedEpisode == nil ? current?.id : nil) {
                ForEach(episodes) { ep in
                    Button { playback.play(PlaybackRequest(target: .server(source, ep))) } label: {
                        TVEpisodeLockup(episode: ep, client: client, isCurrent: ep.id == current?.id)
                    }
                    .focused($focusedEpisode, equals: ep.id)
                    .id(ep.id)
                    .contextMenu {
                        WatchedMenuItems(item: ep, client: client) { await refresh() }
                        Button { playback.play(PlaybackRequest(target: .server(source, ep), startAt: 0)) } label: {
                            Label("Play from Beginning", systemImage: "arrow.counterclockwise")
                        }
                    }
                }
            }
            .defaultFocus($focusedEpisode, current?.id, priority: .userInitiated)
        }
    }

    @ViewBuilder private var cast: some View {
        let people = (item.people ?? []).filter { ["Actor", "Director", "Writer"].contains($0.kind ?? "") }.prefix(20)
        if !people.isEmpty {
            TVShelf(title: "Cast & Crew") {
                ForEach(Array(people)) { p in
                    Button {} label: { TVPersonLockup(person: p, client: client) }
                        .buttonBorderShape(.circle)
                }
            }
        }
    }

    // MARK: Data

    private func load() async {
        guard let client else { return }
        if let fresh = try? await client.item(item.id) { item = fresh }
        guard isShow else { return }
        seasons = (try? await client.seasons(seriesId: item.id)) ?? []
        if current == nil {
            let cont = try? await client.continueEpisode(seriesId: item.id)
            if preselectedSeasonId == nil || cont?.seasonId == preselectedSeasonId { current = cont }
        }
        selectedSeason = seasons.first { $0.id == (preselectedSeasonId ?? current?.seasonId) }
            ?? seasons.first { $0.seasonNumber == current?.seasonNumber } ?? seasons.first
        await loadEpisodes(selectFirst: current == nil)
    }

    private func loadEpisodes(selectFirst: Bool) async {
        guard let client, let season = selectedSeason else { return }
        episodes = (try? await client.episodes(seriesId: item.id, seasonId: season.id)) ?? []
        if selectFirst {
            current = episodes.first { $0.user?.played != true } ?? episodes.first
        } else if let c = current, let fresh = episodes.first(where: { $0.id == c.id }) {
            current = fresh
        }
    }

    private func refresh() async {
        guard let client else { return }
        if let fresh = try? await client.item(item.id) { item = fresh }
        if isShow { await loadEpisodes(selectFirst: false) }
    }

    /// After a season was marked: its state (and its episodes', and the show's next up) changed.
    private func reloadSeasons() async {
        guard let client else { return }
        if let fresh = try? await client.seasons(seriesId: item.id) {
            seasons = fresh
            if let id = selectedSeason?.id { selectedSeason = fresh.first { $0.id == id } ?? selectedSeason }
        }
        await refresh()
    }

    private func setPlayed(_ target: MediaItem, _ played: Bool) async {
        await WatchState.shared.set(target, played: played, client: client)
        await refresh()
    }
}

struct TVPersonLockup: View {
    let person: MediaPerson
    let client: (any MediaServerClient)?

    private var photo: URL? { person.image.flatMap { client?.imageURL($0, width: 300) } }
    private var role: String { person.kind == "Actor" ? (person.role ?? "") : (person.kind ?? "") }

    var body: some View {
        RemoteImage(url: photo)
            .overlay {
                if photo == nil { Image(systemName: "person.fill").font(.largeTitle).foregroundStyle(.secondary) }
            }
            // As wide as the name under it, so the row starts on the content edge.
            .frame(width: 200, height: 200)
            .clipShape(Circle())
            .hoverEffect(.highlight)
        VStack(spacing: 2) {
            Text(person.name ?? "").font(.caption.weight(.medium))
            Text(role).font(.caption2).foregroundStyle(.secondary)
        }
        .lineLimit(1)
        .frame(width: 200)
    }
}

struct TVEpisodeLockup: View {
    let episode: MediaItem
    let client: (any MediaServerClient)?
    let isCurrent: Bool

    var body: some View {
        let episode = WatchState.shared.current(self.episode) // marked from its long-press menu
        ZStack(alignment: .bottom) {
            RemoteImage(url: client?.thumbURL(episode, maxWidth: 800))
            if let p = episode.progress { ProgressBar(value: p).padding(.horizontal, 14).padding(.bottom, 12) }
        }
        .aspectRatio(16 / 9, contentMode: .fit)
        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(alignment: .topTrailing) { WatchedMark(item: episode) }
        .overlay(alignment: .topLeading) { DownloadMark(item: episode) }
        .hoverEffect(.highlight)
        .containerRelativeFrame(.horizontal, count: 4, spacing: 40)
        VStack(alignment: .leading, spacing: 2) {
            Text(episode.episodeListTitle)
                .font(.callout.weight(isCurrent ? .semibold : .medium))
            Text(Fmt.runtime(seconds: episode.runtime) ?? " ").font(.caption).foregroundStyle(.secondary)
        }
        .lineLimit(1)
        .containerRelativeFrame(.horizontal, count: 4, spacing: 40, alignment: .leading)
    }
}
#endif
