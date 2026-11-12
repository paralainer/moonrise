import SwiftUI

/// Presents the Apple TV's full-screen player from anywhere in the app. (Title pages on iPhone,
/// iPad and Mac play in the page: `InlinePlayback`; downloads too, on their title's page.)
@MainActor
@Observable
final class PlaybackCoordinator {
    var request: PlaybackRequest?
    /// A title page's player whose page was left while it was in Picture in Picture.
    @ObservationIgnored private var pictureInPicture: PlayerModel?

    func play(_ r: PlaybackRequest) {
        stopPictureInPicture()
        request = r
    }

    /// Keeps it playing in PiP; it stops (and reports) when PiP ends.
    func keepInPictureInPicture(_ m: PlayerModel) {
        stopPictureInPicture()
        pictureInPicture = m
        m.onPictureInPictureEnded = { [weak self, weak m] in
            m?.close()
            if let self, self.pictureInPicture === m { self.pictureInPicture = nil }
        }
    }

    /// Something else plays now.
    func stopPictureInPicture() {
        pictureInPicture?.close()
        pictureInPicture = nil
    }
}

/// Play as soon as a title page opens (`startAt` nil: resume).
struct Autoplay: Hashable {
    var startAt: Double?
}

enum Route: Hashable {
    /// `zoom`: id of the card it was opened from, for the zoom transition. `autoplay`: start
    /// playing on the page (iPhone, iPad, Mac).
    case item(MediaItem, zoom: String? = nil, autoplay: Autoplay? = nil)
    /// A title opened from Search: as `item`, and remembered for Search's Recent.
    case searchItem(MediaItem, zoom: String? = nil)
    case library(MediaItem)
    /// A library's genres, collections, studios or decades.
    case category(MediaItem, LibraryCategory)
    /// A library's grid with a filter (a category picked), titled.
    case filtered(MediaItem, LibraryFilter, String)
}

struct RootView: View {
    @State private var playback = PlaybackCoordinator()
    @Bindable private var nav = AppNavigation.shared
    @State private var homePath = NavigationPath()
    @Namespace private var zoom

    private func updateHomeCovered() {
        nav.homeCovered = !homePath.isEmpty || nav.tab != 0
    }

    private var store: SourceStore { SourceStore.shared }

    var body: some View {
        Group {
            if let server = store.server {
                // Another server is another app's worth of state: start the tabs afresh.
                tabs.id(server.id)
            } else {
                OnboardingView()
            }
        }
        .environment(playback)
        .environment(\.zoomNamespace, zoom)
        // Admins get Manage Server and Fix Match.
        .task(id: store.server?.id) { await AdminState.shared.refresh() }
        #if !os(tvOS)
        .fixMatchSheet()
        #endif
        // The Top Shelf's cards (Apple TV): moonrise://open/<id> opens the title's page,
        // moonrise://play/<id> plays or resumes it with its page underneath.
        .onOpenURL { url in
            guard let link = TopShelfShared.Link(url) else { return }
            Task { await open(link) }
        }
        .onChange(of: store.server == nil) { _, signedOut in
            guard signedOut else { return }
            homePath = NavigationPath()
            nav.downloadsPath = NavigationPath()
            nav.tab = 0
            playback.request = nil
            playback.stopPictureInPicture()
        }
        #if DEBUG
        .task {
            #if os(macOS)
            MacTestSeam.start()
            #endif
            // Test seam: MOONRISE_DEMO=1 starts in the demo (as Try the Demo does).
            if DebugSeams.launch.demo, SourceStore.shared.server == nil {
                SourceStore.shared.enterDemo()
            }
            await DebugSeed.run()
            #if os(iOS)
            // Test seam: MOONRISE_LANDSCAPE=1 turns the (simulated) phone to landscape on launch, 0 to portrait.
            if let land = DebugSeams.launch.landscape,
               let scene = UIApplication.shared.connectedScenes.first as? UIWindowScene {
                scene.requestGeometryUpdate(.iOS(interfaceOrientations: land ? .landscapeRight : .portrait)) { _ in }
            }
            #endif
            // Test seam: MOONRISE_DISCOVER=<seconds> logs the Moonrise servers Bonjour and the subnet sweep find.
            if let secs = DebugSeams.launch.discover {
                Task.detached {
                    async let b = MoonriseDiscovery.bonjour(timeout: max(secs, 3))
                    async let s = MoonriseDiscovery.sweep(timeout: 2)
                    let (viaBonjour, viaSweep) = await (b, s)
                    fputs("MoonriseDiscovery: bonjour \(viaBonjour.map { "\($0.name) \($0.address) \($0.id)" }) sweep \(viaSweep.map { "\($0.name) \($0.address)" })\n", stderr)
                }
            }
            // Test seam: MOONRISE_MANAGE=… opens Manage Server (ManageSeam); fix:<id> opens Fix Match.
            if let v = ManageSeam.value {
                await ConnectionRouter.shared.ready()
                await AdminState.shared.refresh()
                if v.hasPrefix("fix:") {
                    #if !os(tvOS)
                    FixMatchPresenter.shared.present(String(v.dropFirst(4)))
                    #endif
                } else {
                    #if os(tvOS)
                    nav.tab = 4
                    #endif
                }
            }
            // Test seam: MOONRISE_LIBRARY=… opens the Library tab (LibrarySeam).
            if LibrarySeam.current != nil {
                nav.tab = 1
                await ConnectionRouter.shared.ready()
                nav.tab = 1
            }
            // Test seam: MOONRISE_OPEN_ITEM=<item id> opens that item's page on launch.
            if let id = DebugSeams.launch.openItem,
               let item = try? await SourceStore.shared.client?.item(id) {
                homePath.append(Route.item(item))
            }
            // Test seam: MOONRISE_MARK=<item id>=watched|unwatched marks it after 8 s (as a card's
            // long-press Mark Watched / Unwatched does), with whatever screen is open by then.
            if let spec = DebugSeams.launch.mark {
                let parts = spec.split(separator: "=").map(String.init)
                Task {
                    try? await Task.sleep(for: .seconds(8))
                    guard parts.count == 2, let client = SourceStore.shared.client,
                          let item = try? await client.item(parts[0]) else { return }
                    await WatchState.shared.set(item, played: parts[1] == "watched", client: client)
                }
            }
            // Test seam: MOONRISE_DELETE_DOWNLOAD=<item id> deletes its download (as the Delete Download menu does).
            if let id = DebugSeams.launch.deleteDownload,
               let r = DownloadManager.shared.record(forItem: id) {
                fputs("MoonriseDownload: delete \(r.title)\n", stderr)
                DownloadManager.shared.delete(r.id)
            }
            // Test seam: MOONRISE_AUTOPLAY=<item id> starts playing it (device debugging without a remote).
            if let id = DebugSeams.launch.autoplay {
                await ConnectionRouter.shared.ready()
                fputs("MoonriseAutoplay: \(id) server=\(SourceStore.shared.server?.name ?? "-")\n", stderr)
            }
            if let id = DebugSeams.launch.autoplay,
               let source = SourceStore.shared.server,
               let item = try? await MediaServers.client(for: source)?.item(id) {
                let at = DebugSeams.launch.autoplayAt
                #if os(tvOS)
                playback.play(PlaybackRequest(target: .server(source, item), startAt: at))
                #else
                // As Play does: on the title's page.
                homePath.append(Route.item(item, autoplay: Autoplay(startAt: at)))
                #endif
            }
            // Test seams: MOONRISE_AUTODOWNLOAD=<item id>[:<bits/s>] downloads it (0 or none:
            // Original); MOONRISE_PLAY_DOWNLOAD=<item id> opens its title page in Downloads, playing
            // the download (offline too), as tapping it there does.
            if let spec = DebugSeams.launch.autoDownload {
                await ConnectionRouter.shared.ready()
                let parts = spec.split(separator: ":").map(String.init)
                let bps = parts.count > 1 ? Int(parts[1]) ?? 0 : 0
                let quality = bps == 0 ? QualityOption.original : QualityOption.presets.first { $0.bitrate == bps } ?? .original
                if let source = SourceStore.shared.server, let item = try? await MediaServers.client(for: source)?.item(parts[0]) {
                    if let old = DownloadManager.shared.record(forItem: item.id) { DownloadManager.shared.delete(old.id) }
                    fputs("MoonriseDownload: start \(item.id) \(quality.label)\n", stderr)
                    DownloadManager.shared.download(item: item, from: source, quality: quality)
                }
            }
            if let id = DebugSeams.launch.playDownload,
               let r = DownloadManager.shared.record(forItem: id), r.isPlayable {
                fputs("MoonriseDownload: play \(r.kind.rawValue) \(r.localPath ?? "-") subs=\(r.subtitles.count)\n", stderr)
                if let route = r.pageRoute(startAt: DebugSeams.launch.autoplayAt) {
                    nav.tab = AppNavigation.downloadsTab
                    nav.downloadsPath.append(route)
                }
            }
        }
        #endif
        #if os(tvOS)
        // Presented while there's a request; a new request (another episode) swaps the player
        // inside the same cover instead of dismissing it.
        .fullScreenCover(isPresented: Binding(get: { playback.request != nil },
                                              set: { if !$0 { playback.request = nil } })) {
            if let r = playback.request {
                PlayerScreen(model: PlayerModel(request: r)) { playback.request = nil }
                    .id(r.id)
                    .environment(playback)
            }
        }
        #endif
        #if !os(tvOS)
        .modifier(WindowMetrics())
        #endif
        #if os(macOS)
        .background(MacWindowReader())
        #endif
    }

    /// A deep link: the title's page on Home (over Home, Back returns there), and for play the
    /// player over it, resuming. Signed out, or a title the server doesn't have: nothing happens.
    private func open(_ link: TopShelfShared.Link) async {
        let id: String
        switch link {
        case let .open(i), let .play(i): id = i
        }
        await ConnectionRouter.shared.ready()
        guard let source = store.server, source.kind == .moonrise,
              let item = try? await MediaServers.client(for: source)?.item(id),
              store.server?.id == source.id else { return }
        let play = link == .play(id)
        // Whatever plays now gives way.
        playback.request = nil
        playback.stopPictureInPicture()
        nav.tab = 0
        #if os(tvOS)
        nav.focusOnOpen = item.id
        homePath = NavigationPath([Route.item(item)])
        if play, item.isPlayable {
            // Once the page is up and focused, so the player covers it and Back from the player
            // lands on it.
            try? await Task.sleep(for: .milliseconds(600))
            playback.play(PlaybackRequest(target: .server(source, item)))
        }
        #else
        homePath = NavigationPath([Route.item(item, autoplay: play && item.isPlayable ? Autoplay() : nil)])
        #endif
    }

    private var tabs: some View {
        TabView(selection: $nav.tab) {
            Tab("Home", systemImage: "house.fill", value: 0) {
                NavigationStack(path: $homePath) {
                    #if os(tvOS)
                    TVHomeView().withRoutes()
                    #else
                    HomeView().withRoutes()
                    #endif
                }
            }
            Tab("Library", systemImage: "square.stack.fill", value: 1) {
                NavigationStack(path: $nav.libraryPath) { LibraryRootView().withRoutes() }
                    #if os(iOS)
                    .modifier(LibraryTurn())
                    #endif
            }
            Tab("Search", systemImage: "magnifyingglass", value: 2, role: .search) {
                NavigationStack { SearchView().withRoutes() }
            }
            if DownloadManager.isSupported {
                Tab("Downloads", systemImage: "arrow.down.circle.fill", value: 3) {
                    NavigationStack(path: $nav.downloadsPath) { DownloadsView().withRoutes() }
                }
            }
            #if os(tvOS)
            Tab("Settings", systemImage: "gearshape", value: 4) {
                NavigationStack { SettingsView() }
            }
            #endif
        }
        #if os(tvOS)
        .overlay(alignment: .topTrailing) {
            if SourceStore.shared.isDemo { DemoBadge().padding(.top, 20).allowsHitTesting(false) }
        }
        #endif
        #if os(macOS)
        // As the iPad: the tabs at the top (in the toolbar), no sidebar; Library has its own.
        .tabViewStyle(.tabBarOnly)
        #elseif os(iOS)
        // iPad: the floating tab bar at the top, no sidebar (it only repeated these four tabs and
        // took a quarter of the width; Library's own switches and filters are in the page).
        // iPhone and narrow iPad windows: the bottom tab bar either way.
        .tabViewStyle(.tabBarOnly)
        #endif
        // The server doesn't answer at launch: open on the downloads, they play.
        .task {
            let router = ConnectionRouter.shared
            await router.ready()
            if router.route == .unreachable, DownloadManager.shared.records.contains(where: \.isPlayable), nav.tab == 0 {
                nav.tab = AppNavigation.downloadsTab
            }
            await LocalReports.shared.flush()
        }
        // Back online: send what was watched offline.
        .onChange(of: ConnectionRouter.shared.route) { _, _ in Task { await LocalReports.shared.flush() } }
        .onChange(of: homePath.count) { updateHomeCovered() }
        // A title to open on Home (e.g. a card's Play: its page plays it).
        .onChange(of: nav.pendingRoute) { _, route in
            guard let route else { return }
            nav.pendingRoute = nil
            nav.tab = 0
            homePath.append(route)
        }
        .onChange(of: nav.tab) { updateHomeCovered() }
    }
}

extension View {
    func withRoutes() -> some View {
        navigationDestination(for: Route.self) { route in
            RouteDestination(route: route)
        }
    }
}

private struct RouteDestination: View {
    let route: Route
    @Environment(\.zoomNamespace) private var zoom

    var body: some View {
        if let s = SourceStore.shared.server {
            switch route {
            case let .item(item, zoomID, autoplay):
                #if os(tvOS)
                TVItemDetailView(source: s, item: item)
                #else
                ItemDetailView(source: s, item: item, autoplay: autoplay)
                    .zoomDestination(zoomID, in: zoom)
                    .onAppear { if let zoomID { AppNavigation.shared.lastOpened = zoomID } }
                #endif
            case let .searchItem(item, zoomID):
                let openedFromSearch = { SearchStartStore.shared.opened(item, client: SourceStore.shared.client) }
                #if os(tvOS)
                TVItemDetailView(source: s, item: item)
                    .onAppear(perform: openedFromSearch)
                #else
                ItemDetailView(source: s, item: item, autoplay: nil)
                    .zoomDestination(zoomID, in: zoom)
                    .onAppear {
                        if let zoomID { AppNavigation.shared.lastOpened = zoomID }
                        openedFromSearch()
                    }
                #endif
            case let .library(view):
                LibraryBrowser(source: s, library: view, title: view.displayTitle)
            case let .category(library, category):
                LibraryCategoryView(source: s, library: library, category: category)
            case let .filtered(library, filter, title):
                LibraryBrowser(source: s, library: library, preset: filter, title: title)
            }
        }
    }
}
