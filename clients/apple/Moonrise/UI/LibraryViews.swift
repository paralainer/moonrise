import SwiftUI

// The Library tab (Phase 7): Movies and Shows as a split, each a full poster grid with filter
// chips, sorts, an alphabet index and category pages. The server filters and counts; with no
// server the same filters run over the downloads. Libraries have no type (server 0.15): Movies
// and Shows are views by type across all libraries; with several libraries, Library is a filter.

/// The server's libraries, kept for launches with no server (offline Library).
enum LibraryList {
    private static let key = "library.list"

    static func saved() -> [MediaItem] {
        UserDefaults.standard.data(forKey: key).flatMap { try? JSONDecoder().decode([MediaItem].self, from: $0) } ?? []
    }

    static func save(_ list: [MediaItem]) {
        if let d = try? JSONEncoder().encode(list) { UserDefaults.standard.set(d, forKey: key) }
    }

    /// The Library's views: the movies and the shows of every library (`@type`: the client asks
    /// for `type=` across libraries).
    static let views = [
        MediaItem(id: "@movie", kind: .library, collectionType: "movies", isFolder: true, name: "Movies"),
        MediaItem(id: "@series", kind: .library, collectionType: "tvshows", isFolder: true, name: "Shows"),
    ]

    /// The view for a remembered selection (older builds remembered a library: its type's view).
    static func view(for id: String) -> MediaItem {
        if let v = views.first(where: { $0.id == id }) { return v }
        if id == "shows" || saved().first(where: { $0.id == id })?.isShowLibrary == true { return views[1] }
        return views[0]
    }
}

extension MediaItem {
    var isShowLibrary: Bool { collectionType == "tvshows" }
}

extension LibraryFilter {
    /// A view's remembered filters; the first time, those older builds kept for the library of
    /// its type ("movies", "shows").
    static func remembered(for view: MediaItem) -> LibraryFilter? {
        if let f = saved(for: view.id) { return f }
        guard view.id.hasPrefix("@"), let old = LibraryList.saved().first(where: { $0.isShowLibrary == view.isShowLibrary }) else { return nil }
        return saved(for: old.id)
    }
}

enum LibraryCategory: String, Hashable, CaseIterable, Identifiable {
    case genres, collections, studios, decades
    var id: String { rawValue }
    func title(shows: Bool) -> String {
        switch self {
        case .genres: "Genres"
        case .collections: "Collections"
        case .studios: shows ? "Networks" : "Studios"
        case .decades: "Decades"
        }
    }
    func singular(shows: Bool) -> String {
        switch self {
        case .genres: "genre"
        case .collections: "collection"
        case .studios: shows ? "network" : "studio"
        case .decades: "decade"
        }
    }
    func icon(shows: Bool) -> String {
        switch self {
        case .genres: "theatermasks"
        case .collections: "square.stack.3d.up"
        case .studios: shows ? "tv" : "building.2"
        case .decades: "calendar"
        }
    }
}

/// Library tab: the libraries as a split (Movies | Shows), the chosen one's grid below.
struct LibraryRootView: View {
    @State private var libraries: [MediaItem] = LibraryList.saved()
    @AppStorage("library.selected") private var selectedId = ""
    @State private var reload = 0
    /// iPad: the page beside the sidebar.
    @Bindable private var nav = AppNavigation.shared
    @Environment(\.metrics) private var metrics
    private var router: ConnectionRouter { ConnectionRouter.shared }
    private var offline: Bool { router.route == .unreachable }
    private var hasDownloads: Bool { DownloadManager.isSupported && DownloadManager.shared.records.contains(where: \.isPlayable) }

    #if DEBUG
    init() {
        #if os(tvOS)
        TVLibrarySeam.applyOnce()
        #else
        LibrarySeam.applyOnce()
        #endif
    }
    #endif

    var body: some View {
        let libs = LibraryList.views
        let current = LibraryList.view(for: selectedId)
        // The split shows the view in use (an older build's remembered library maps to its type's).
        let selection = Binding(get: { current.id }, set: { selectedId = $0 })
        Group {
            if let source = SourceStore.shared.server, !offline || hasDownloads {
                #if os(tvOS)
                // Apple TV: a sidebar (libraries, categories) beside the grid.
                TVLibraryScreen(source: source, library: current, libraries: libs, selection: selection, retry: { reload += 1 })
                #else
                if metrics.sidebarLibrary {
                    // iPad, and the iPhone in landscape: a sidebar (libraries, categories) beside the page.
                    PadLibraryScreen(source: source, library: current, libraries: libs, selection: selection,
                                     page: $nav.libraryPadPage, retry: { reload += 1 })
                } else {
                    LibraryBrowser(source: source, library: current, libraries: libs, selection: selection, retry: { reload += 1 })
                        .id(current.id)
                }
                #endif
            } else if offline, let problem = router.problem {
                ScrollView { ConnectionProblemView(problem: problem) { reload += 1 }.padding(.vertical) }
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .background(Theme.backgroundGradient.ignoresSafeArea())
        #if os(tvOS)
        .navigationTitle("") // the tab bar already says Library
        #else
        .navigationTitle("Library")
        #endif
        #if os(iOS)
        .settingsButton(onlyOnPad: true)
        #endif
        // Load again when the server becomes reachable or the user retries.
        .task(id: "\(String(describing: router.route))-\(reload)") {
            await router.ready()
            // A few tries: right after sign-in (or a route switch) the first request can fail.
            var answer: [MediaItem]?
            for attempt in 0..<6 where !offline {
                if let c = SourceStore.shared.client, let v = try? await c.libraries() { answer = v; break }
                try? await Task.sleep(for: .seconds(1 + Double(attempt)))
                if Task.isCancelled { return }
            }
            guard let v = answer, let c = SourceStore.shared.client else { return }
            if v != libraries { libraries = v }
            LibraryList.save(v)
            if DownloadManager.isSupported { await DownloadManager.shared.refreshSnapshots(client: c) }
        }
        #if DEBUG
        .task {
            try? await Task.sleep(for: .milliseconds(300)) // the window measured (iPad: the sidebar's pages)
            applySeam(libs)
        }
        #endif
    }

    #if DEBUG
    /// Test seam MOONRISE_LIBRARY="lib=shows;page=genres;genre=Drama;…" (see `LibrarySeam`): its page,
    /// once per launch (the library and its filters were set by `LibrarySeam.applyOnce`).
    private func applySeam(_ libs: [MediaItem]) {
        #if !os(tvOS) // tvOS: the sidebar opens the page (TVLibraryScreen)
        guard let seam = LibrarySeam.takePage(), let page = seam.page else { return }
        let lib = LibraryList.view(for: seam.library ?? selectedId)
        if metrics.sidebarLibrary, let c = LibraryCategory(rawValue: page) {
            nav.libraryPadPage = .category(c)
        } else if metrics.sidebarLibrary, page == "filtered" {
            nav.libraryPadPage = .filtered(.genres, LibraryFilter(seam: seam.raw).lockingCategory, seam.title ?? "Filtered")
        } else if let c = LibraryCategory(rawValue: page) {
            AppNavigation.shared.libraryPath.append(Route.category(lib, c))
        } else if page == "filtered" {
            AppNavigation.shared.libraryPath.append(Route.filtered(lib, LibraryFilter(seam: seam.raw).lockingCategory, seam.title ?? "Filtered"))
        }
        #endif
    }
    #endif
}

#if DEBUG
/// MOONRISE_LIBRARY="lib=<id>;page=genres|collections|studios|decades|filtered;title=…;jump=M;<filter keys>":
/// opens the Library tab on a library, with filters (the root grid, or a `filtered` page), a
/// category page, and a jump to a letter.
struct LibrarySeam {
    let raw: String
    private var values: [String: String] {
        Dictionary(raw.split(separator: ";").compactMap { p -> (String, String)? in
            let kv = p.split(separator: "=", maxSplits: 1).map(String.init)
            return kv.count == 2 ? (kv[0], kv[1]) : nil
        }, uniquingKeysWith: { _, b in b })
    }
    /// "movies"/"shows" (or "@movie"/"@series").
    var library: String? { values["lib"].map { LibraryList.view(for: $0).id } }
    var page: String? { values["page"] }
    var title: String? { values["title"] }
    var jump: String? { values["jump"] }
    /// tvOS: "filter" opens the filter panel, "filter:genre" one of its lists.
    var panel: String? { values["panel"] }
    static var current: LibrarySeam? {
        DebugSeams.launch.library.map { LibrarySeam(raw: $0) }
    }

    private static var applied = false
    private static var pageTaken = false
    private static var jumpTaken = false

    /// Once per launch: picks the library and sets its filters to the seam's (none = cleared), as
    /// if chosen there; from then on the Library remembers as usual. A `filtered` page keeps its
    /// filters to itself.
    static func applyOnce() {
        guard !applied, let seam = current else { return }
        applied = true
        if let lib = seam.library { UserDefaults.standard.set(lib, forKey: "library.selected") }
        let lib = seam.library ?? UserDefaults.standard.string(forKey: "library.selected") ?? "@movie"
        if seam.page != "filtered" { LibraryFilter(seam: seam.raw).save(for: lib) }
    }

    /// The page to open, once per launch.
    static func takePage() -> LibrarySeam? {
        guard !pageTaken, let seam = current else { return nil }
        pageTaken = true
        return seam
    }

    /// The letter to jump to, once per launch.
    static func takeJump() -> String? {
        guard !jumpTaken, let j = current?.jump else { return nil }
        jumpTaken = true
        return j
    }
}
#endif

/// One library's grid: filter bar, active filters, count, posters, alphabet index. The root
/// (with the library split) remembers its filters per library; category pages start from theirs.
struct LibraryBrowser: View {
    @Environment(\.metrics) var metrics
    let source: Source
    let library: MediaItem
    var libraries: [MediaItem] = []
    var selection: Binding<String>?
    var title: String?
    var retry: (() -> Void)?
    /// iPad: beside the Library's sidebar (a header with the title, Sort and Filter; no chip rows).
    var padSidebar = false
    /// iPad: the header's back button (a category's tile page).
    var onBack: (() -> Void)?

    @State var filter: LibraryFilter
    /// The filter the page opened with (a category tile's).
    private let preset: LibraryFilter
    /// iPad: the filter panel is open.
    @State var showFilters = false
    @State var items: [MediaItem] = []
    @State var total = 0
    @State var index: [ItemPage.Letter] = []
    @State var facets: LibraryFacets?
    @State var loading = false
    @State private var error: String?
    @State private var width: CGFloat = 0
    /// Where to scroll: a letter jump to its first title, a filter change back to the top. (Not a
    /// tracked scroll position: one keeps the title it last saw in place when the list changes,
    /// pushing the header away, and lines the grid's leading edge up with the screen's.)
    @State private var scrollRequest: ScrollRequest?
    private struct ScrollRequest: Equatable {
        let id: String
        let serial: Int
    }
    private static let topId = "library-top"

    /// Bumped by each reload: a page asked for under older filters is dropped.
    @State private var generation = 0
    /// The end of the grid came into view while a load was running: load more after it.
    @State private var moreRequested = false
    /// Where the header (chips, count line) ends on screen, in the page's space: the A–Z strip
    /// stays below it.
    @State private var headerBottom: CGFloat = 0
    private static let pageSpace = "library-page"
    #if os(tvOS)
    /// The focused poster, and the last one: coming back from the sidebar lands there.
    @FocusState private var focusedItem: String?
    @State private var lastFocusedItem: String?
    /// The poster focus left the grid from (the focus engine may land elsewhere on the way back).
    @State private var parkedItem: String?
    @Environment(\.tvGridFocusRequest) private var gridFocusRequest
    @Environment(\.tvFocusOnLoad) private var focusOnLoad
    @Environment(\.tvContentLeading) private var contentLeading
    @State private var panelSeamDone = false
    /// Focus goes back to Filter when the panel closes.
    @State var restoreFilterFocus = false
    @FocusState var filterButtonFocused: Bool
    #endif
    #if os(iOS)
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.verticalSizeClass) private var verticalSizeClass
    #endif

    private var router: ConnectionRouter { ConnectionRouter.shared }
    var offline: Bool { router.route == .unreachable }
    var isRoot: Bool { selection != nil }
    var isShows: Bool { library.isShowLibrary }
    private var client: (any MediaServerClient)? { MediaServers.client(for: source) }
    private var manager: DownloadManager { DownloadManager.shared }
    private let pageSize = 60
    @Environment(\.dismiss) var dismiss

    init(source: Source, library: MediaItem, libraries: [MediaItem] = [], selection: Binding<String>? = nil,
         preset: LibraryFilter? = nil, title: String? = nil, retry: (() -> Void)? = nil,
         padSidebar: Bool = false, onBack: (() -> Void)? = nil) {
        self.padSidebar = padSidebar
        self.onBack = onBack
        self.source = source
        self.library = library
        self.libraries = libraries
        self.selection = selection
        self.title = title
        self.retry = retry
        self.preset = preset ?? LibraryFilter()
        var f = preset ?? (selection != nil ? LibraryFilter.remembered(for: library) : nil) ?? LibraryFilter()
        if !DownloadManager.isSupported { f.downloaded = false }
        _filter = State(initialValue: f)
    }

    /// What a reload depends on.
    private struct LoadKey: Hashable {
        var filter: LibraryFilter
        var offline: Bool
        var downloads: Int
    }

    var body: some View {
        ScrollView {
            ScrollViewReader { proxy in
                VStack(alignment: .leading, spacing: 14) {
                    header
                    grid
                        .onGeometryChange(for: CGFloat.self) { $0.frame(in: .named(Self.pageSpace)).minY } action: { headerBottom = $0 }
                }
                .padding(.vertical, 12)
                #if os(tvOS)
                .padding(.leading, contentLeading)
                #endif
                // The very top of the page (above the padding).
                .background(alignment: .topLeading) { Color.clear.frame(width: 1, height: 1).id(Self.topId) }
                .onChange(of: scrollRequest) { _, r in
                    guard let r else { return }
                    proxy.scrollTo(r.id, anchor: .top)
                }
            }
        }
        #if os(tvOS)
        .scrollClipDisabled() // beside the sidebar: the first column's focus growth
        .defaultFocus($focusedItem, lastFocusedItem ?? items.first?.id, priority: .userInitiated)
        .onChange(of: focusedItem) { _, id in
            if let id { lastFocusedItem = id } else { parkedItem = lastFocusedItem }
        }
        .onChange(of: items.first?.id) { old, first in
            // Reached from a category tile: the first poster takes focus.
            guard old == nil, let first, focusOnLoad != nil else { return }
            Task { try? await Task.sleep(for: .milliseconds(80)); focusedItem = first }
        }
        .onChange(of: gridFocusRequest) {
            // The poster from before the sidebar, set once the focus engine's own move (to the
            // nearest poster or the header) has landed.
            guard let target = parkedItem ?? items.first?.id else { return }
            Task {
                try? await Task.sleep(for: .milliseconds(30))
                focusedItem = target
            }
        }
        #endif
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
        #if !os(tvOS)
        // Never over the header's chips or count line: below them while they show.
        .overlay {
            if showIndex {
                GeometryReader { g in
                    let f = indexFrame(in: g.size, top: g.frame(in: .named(Self.pageSpace)).minY)
                    if f.height >= 180 {
                        AlphabetIndex(letters: index.map(\.letter)) { l in Task { await jump(to: l) } }
                            .frame(height: f.height)
                            .position(x: g.size.width - 12, y: f.midY)
                    }
                }
            }
        }
        .coordinateSpace(name: Self.pageSpace)
        #endif
        #if DEBUG && !os(tvOS)
        .task {
            // Test seam: panel=filter[:genre] opens the iPad filter panel.
            guard padSidebar || phonePortrait, isRoot, LibrarySeam.current?.panel?.hasPrefix("filter") == true else { return }
            try? await Task.sleep(for: .seconds(2))
            showFilters = true
        }
        #endif
        #if DEBUG
        .onChange(of: index) { _, _ in
            guard isRoot, let l = LibrarySeam.takeJump(), index.contains(where: { $0.letter == l }) else { return }
            Task { try? await Task.sleep(for: .milliseconds(600)); await jump(to: l) }
        }
        #endif
        .background(Theme.backgroundGradient.ignoresSafeArea())
        #if os(tvOS)
        .navigationTitle(title ?? "")
        #else
        .navigationTitle(title ?? (isRoot ? "Library" : library.displayTitle))
        #endif
        #if os(iOS)
        .navigationBarTitleDisplayMode(isRoot && !padSidebar ? .large : .inline)
        // iPhone held upright: the iPad's header made compact (title, Sort, Filter) pinned in
        // place of the navigation bar.
        .toolbar(phonePortrait ? .hidden : .automatic, for: .navigationBar)
        // With the bar hidden UIKit turns off the swipe back: a tile's page keeps it.
        .background { if phonePortrait && !isRoot { SwipeBackKeeper().frame(width: 0, height: 0) } }
        #endif
        #if !os(tvOS)
        .modifier(PadPageChrome(on: padSidebar || phonePortrait) {
            if padSidebar { padHeader } else { phoneTitleRow }
        })
        // A category tile's page is the Library's page: turning the phone carries it (with its
        // filters and sort) between the pushed pages and the sidebar's.
        .onAppear { if !isRoot, !padSidebar { AppNavigation.shared.libraryPage = tilePage } }
        .onDisappear {
            if !isRoot, !padSidebar, AppNavigation.shared.libraryPage?.title == tilePage?.title { AppNavigation.shared.libraryPage = nil }
        }
        .onChange(of: filter) {
            guard !isRoot, let page = tilePage else { return }
            if padSidebar {
                if case .filtered = AppNavigation.shared.libraryPadPage { AppNavigation.shared.libraryPadPage = page }
            } else {
                AppNavigation.shared.libraryPage = page
            }
        }
        #endif
        .task(id: LoadKey(filter: filter, offline: offline, downloads: manager.records.filter(\.isPlayable).count)) {
            await reload()
        }
        .onChange(of: filter) { _, f in
            if isRoot { f.save(for: library.id) }
            scrollRequest = ScrollRequest(id: Self.topId, serial: (scrollRequest?.serial ?? 0) + 1)
        }
        #if os(tvOS)
        // A panel from the right over the grid: the grid behind it follows each choice. The
        // Library under it (sidebar, tab bar) can't take focus while it's open.
        .disabled(showFilters)
        .overlay {
            if showFilters {
                TVFilterPanel(filter: $filter, facets: facets, isShows: isShows, total: total, loading: loading,
                              start: panelSeamStart) {
                    showFilters = false
                    restoreFilterFocus = true
                }
                .transition(.move(edge: .trailing))
            }
        }
        .animation(.easeOut(duration: 0.25), value: showFilters)
        .toolbar(showFilters ? .hidden : .automatic, for: .tabBar)
        .preference(key: TVPanelOpenKey.self, value: showFilters)
        #if DEBUG
        .task {
            guard isRoot, !panelSeamDone, LibrarySeam.current?.panel != nil else { return }
            panelSeamDone = true
            try? await Task.sleep(for: .seconds(2))
            showFilters = true
        }
        #endif
        #endif
    }

    // MARK: Header

    /// The A–Z strip's place in the overlay (`size`, its top at `top` in the page's space):
    /// iPhone low on the screen, iPad and Mac centred, always below the header while it shows
    /// (shorter when it must be; hidden when there's no room until the header scrolls away).
    private func indexFrame(in size: CGSize, top: CGFloat) -> CGRect {
        #if os(iOS)
        let low = sizeClass != .regular
        #else
        let low = false
        #endif
        let minY = max(compactHeight ? 4 : 12, headerBottom - top + 6)
        let maxY = size.height - (compactHeight ? 46 : 10)
        let h = min(432, maxY - minY)
        guard h > 0 else { return .zero }
        let y = low ? maxY - h : min(max(minY, (size.height - h) / 2), maxY - h)
        return CGRect(x: 0, y: y, width: 20, height: h)
    }

    /// The split shares the categories' row: iPhone landscape (no height to spare) and iPad (the
    /// width to spare; the grid starts higher).
    var sharedRow: Bool { compactHeight || metrics.regular }

    /// iPhone landscape.
    private var compactHeight: Bool {
        #if os(iOS)
        return verticalSizeClass == .compact
        #else
        return false
        #endif
    }

    @ViewBuilder var picker: some View {
        if isRoot, libraries.count > 1, let selection {
            Picker("Library", selection: selection) {
                ForEach(libraries) { Text($0.displayTitle).tag($0.id) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            // iPad: as tall as the category chips beside it.
            .controlSize(metrics.regular ? .extraLarge : .regular)
        }
    }

    @ViewBuilder private var header: some View {
        #if os(tvOS)
        tvHeader
        #else
        if padSidebar {
            padSubheader
        } else if phonePortrait {
            phonePortraitHeader
        } else {
            phoneHeader
        }
        #endif
    }

    /// A category tile's page, as the sidebar layout names it (with the filters as they are now).
    private var tilePage: PadLibraryPage? {
        guard let c = preset.category else { return nil }
        return .filtered(c, filter, title ?? library.displayTitle)
    }

    var countRow: some View {
        HStack(spacing: 6) {
            Text(countLine)
            if loading && !items.isEmpty { ProgressView().controlSize(.small) }
        }
        .font(.footnote)
        .foregroundStyle(Theme.secondaryText)
        .padding(.horizontal, metrics.hPadding)
    }

    var countLine: String {
        let noun = isShows ? (total == 1 ? "show" : "shows") : (total == 1 ? "movie" : "movies")
        let dir: String = switch filter.sort {
        case .title: filter.descending ? "Z to A" : "A to Z"
        case .added: filter.descending ? "newest first" : "oldest first"
        case .year: filter.descending ? "newest first" : "oldest first"
        case .rating: filter.descending ? "highest first" : "lowest first"
        case .played: filter.descending ? "most recent first" : "least recent first"
        case .runtime: filter.descending ? "longest first" : "shortest first"
        }
        return "\(total) \(noun) · \(filter.sort.label), \(dir)" + (offline ? " · downloaded" : "")
    }

    // MARK: Grid

    private var spacing: CGFloat {
        #if os(tvOS)
        return 40 // Home's shelf spacing
        #else
        return 12
        #endif
    }

    /// Between grid rows: tvOS leaves room for the focused poster's lift and its title moving down.
    private var rowSpacing: CGFloat {
        #if os(tvOS)
        return 60
        #else
        return spacing + 8
        #endif
    }

    private var showIndex: Bool {
        #if os(tvOS)
        return false
        #else
        return filter.sort == .title && index.count > 1 && total > 24
        #endif
    }

    /// Columns and card width: posters fill the row evenly.
    private var layout: (columns: Int, card: CGFloat) {
        var minCard = Theme.gridMin
        #if os(iOS)
        minCard = metrics.regular ? metrics.gridMin : padSidebar ? 112 : sizeClass == .regular ? 140 : 104
        #elseif os(macOS)
        minCard = metrics.gridMin
        #endif
        var avail = max(200, width - metrics.hPadding * 2 - (showIndex ? 20 : 0))
        #if os(tvOS)
        avail = max(200, avail - contentLeading) // beside the sidebar
        #endif
        var n = max(2, Int((avail + spacing) / (minCard + spacing)))
        // iPad beside the sidebar: at least 4 across (the 11" held upright), the posters smaller.
        if padSidebar { n = max(n, 4) }
        return (n, floor((avail - CGFloat(n - 1) * spacing) / CGFloat(n)))
    }

    @ViewBuilder private var grid: some View {
        let l = layout
        if items.isEmpty && !loading && error == nil && facets != nil {
            VStack(spacing: 12) {
                EmptyStateView(icon: filter.isFiltered ? "line.3.horizontal.decrease.circle" : (isShows ? "tv" : "film"),
                               title: filter.isFiltered ? "No Matches" : "Nothing Here",
                               message: filter.isFiltered ? "No \(isShows ? "shows" : "movies") match these filters."
                                   : offline ? "Nothing in this library is downloaded." : "This library is empty.")
                #if !os(tvOS) // tvOS: Clear is on the filter line just above
                if filter.isFiltered {
                    Button("Clear Filters") { filter = filter.cleared }
                        .buttonStyle(.glass)
                }
                #endif
            }
            .frame(maxWidth: .infinity)
        } else if width > 0 {
            LazyVGrid(columns: Array(repeating: GridItem(.fixed(l.card), spacing: spacing, alignment: .top), count: l.columns),
                      alignment: .leading, spacing: rowSpacing) {
                #if os(tvOS)
                let edge = Set(stride(from: 0, to: items.count, by: l.columns).map { items[$0].id })
                #endif
                ForEach(items) { item in
                    card(item, width: l.card)
                        #if os(tvOS)
                        .tvLeadingEdge(edge.contains(item.id))
                        #endif
                        .id(item.id)
                        .onAppear { if isNearEnd(item) { Task { await loadMore() } } }
                }
            }
            .padding(.leading, metrics.hPadding)
            if loading && items.isEmpty { ProgressView().frame(maxWidth: .infinity).padding() }
            if let error { Text(error).font(.footnote).foregroundStyle(.red).padding(.horizontal, metrics.hPadding) }
        }
    }

    @ViewBuilder private func card(_ item: MediaItem, width: CGFloat) -> some View {
        if offline {
            OfflineTitleCard(item: item, library: library, width: width)
        } else {
            // Zooms out of its poster as on Home (and closes with a swipe down).
            NavigationLink(value: Route.item(item, zoom: "library/\(item.id)")) {
                PosterCard(item: item, client: client, width: width, zoomID: "library/\(item.id)")
            }
            .cardButtonStyle()
            #if os(tvOS)
            .focused($focusedItem, equals: item.id)
            #endif
            .watchedMenu(item, client: client)
        }
    }

    // MARK: Loading

    /// The titles downloaded in this library (Downloaded filter), as the server knows them: the
    /// ones that play here (as the offline Library shows them).
    private var downloadedIds: [String] {
        let recs = manager.records.filter { $0.itemId != nil && $0.isPlayable }
        return isShows ? Array(Set(recs.compactMap { $0.item?.isEpisode == true ? $0.seriesId : nil })).sorted()
            : recs.filter { $0.item?.isEpisode != true }.compactMap(\.itemId)
    }

    /// Among the last rows of what's loaded: time to load the next page.
    private func isNearEnd(_ item: MediaItem) -> Bool {
        guard items.count < total, let i = items.lastIndex(where: { $0.id == item.id }) else { return false }
        return i >= items.count - max(12, layout.columns * 3)
    }

    private func reload() async {
        error = nil
        generation += 1
        moreRequested = false
        if offline {
            loadOffline()
            return
        }
        guard let client else { return }
        // `loading` and the lists belong to the latest reload only (an older one may still be
        // finishing after a quick second change).
        let gen = generation
        loading = true
        let only = filter.downloaded ? downloadedIds : nil
        let f = filter
        async let facetAnswer = try? client.facets(libraryId: library.id, filter: f, only: only)
        do {
            let r = try await client.items(ItemQuery(parent: library, filter: f, only: only, start: 0, limit: pageSize))
            if gen == generation, !Task.isCancelled {
                items = r.items
                total = r.total ?? r.items.count
                index = r.index
            }
        } catch {
            if !error.isCancellation, gen == generation { self.error = error.localizedDescription }
        }
        guard gen == generation else { _ = await facetAnswer; return }
        // The grid can page on while the counts load.
        loading = false
        if let fa = await facetAnswer, gen == generation, !Task.isCancelled {
            facets = fa
            // A library chosen without its name (a remembered filter): named from the facets.
            if let l = filter.library, filter.libraryName == nil, let name = fa.libraries.first(where: { $0.id == l })?.name {
                filter.libraryName = name
            }
        }
        await loadRequestedMore()
    }

    /// Loads the next page (through `target`, an offset, when given). One load at a time: asked
    /// for during another, it runs after it.
    private func loadMore(through target: Int? = nil) async {
        guard !offline, let client, items.count < total else { return }
        guard !loading else { moreRequested = true; return }
        loading = true
        let gen = generation
        let f = filter
        let want = min(500, max(pageSize, (target ?? 0) - items.count + pageSize))
        do {
            let r = try await client.items(ItemQuery(parent: library, filter: f, only: f.downloaded ? downloadedIds : nil,
                                                     start: items.count, limit: want))
            // Filters changed meanwhile: this page belongs to the old list.
            if gen == generation {
                let known = Set(items.map(\.id))
                items += r.items.filter { !known.contains($0.id) }
                total = r.total ?? total
            }
        } catch {
            if !error.isCancellation, gen == generation { self.error = error.localizedDescription }
        }
        guard gen == generation else { return }
        loading = false
        await loadRequestedMore()
    }

    private func loadRequestedMore() async {
        guard moreRequested, !loading else { return }
        moreRequested = false
        await loadMore()
    }

    /// Scrolls to where a letter starts, loading the pages through it first, and a page past it
    /// (else a letter near the end of what's loaded can't scroll to the top).
    func jump(to letter: String) async {
        guard let at = index.first(where: { $0.letter == letter })?.offset else { return }
        if at + pageSize > items.count, items.count < total {
            // Wait out a load that's running (it may be the first page).
            for _ in 0..<50 where loading { try? await Task.sleep(for: .milliseconds(100)) }
            await loadMore(through: at + pageSize)
        }
        guard at < items.count else { return }
        let id = items[at].id
        scrollRequest = ScrollRequest(id: id, serial: (scrollRequest?.serial ?? 0) + 1)
        #if os(tvOS)
        // Then the letter's first title takes focus (once the grid has made it, and the menu
        // has gone): else focus stays on Jump To, which keeps the header on screen.
        for ms in [60, 150, 300, 500] {
            try? await Task.sleep(for: .milliseconds(ms))
            if focusedItem == id { break }
            scrollRequest = ScrollRequest(id: id, serial: (scrollRequest?.serial ?? 0) + 1)
            try? await Task.sleep(for: .milliseconds(30))
            focusedItem = id
        }
        #endif
    }

    /// The downloads of this library, filtered and sorted here.
    private func loadOffline() {
        let titles = OfflineLibrary.titles(shows: isShows)
        var f = filter
        f.downloaded = false
        let matching = titles.filter { f.matches($0.item, episodes: $0.episodes) }
        // A show's Date Added is its latest episode's (as the server sorts).
        let latest = Dictionary(matching.map { t in (t.item.id, t.episodes.compactMap(\.addedAt).max()) }, uniquingKeysWith: { a, _ in a })
        items = f.sorted(matching.map(\.item), added: { isShows ? (latest[$0.id] ?? nil) ?? $0.addedAt : $0.addedAt })
        total = items.count
        var seen: [ItemPage.Letter] = []
        if f.sort == .title {
            for (i, item) in items.enumerated() {
                let l = LibraryFilter.letter(item.displayTitle)
                if !seen.contains(where: { $0.letter == l }) { seen.append(.init(letter: l, offset: i)) }
            }
        }
        index = seen
        facets = LibraryFacets.local(titles, filter: f)
    }
}

/// The downloads as library titles: movies, and shows made of their downloaded episodes.
enum OfflineLibrary {
    @MainActor static func titles(shows: Bool) -> [(item: MediaItem, episodes: [MediaItem])] {
        let recs = DownloadManager.shared.records.filter(\.isPlayable)
        if !shows {
            return recs.compactMap { r in r.item.flatMap { $0.isEpisode ? nil : (item: $0, episodes: []) } }
        }
        let byShow = Dictionary(grouping: recs.filter { $0.item?.isEpisode == true }) { $0.seriesId ?? "" }
        return byShow.compactMap { sid, eps in
            guard let first = eps.first, let ep = first.item else { return nil }
            let show = eps.compactMap(\.series).first ?? MediaItem(
                id: sid, kind: .series, isFolder: true, name: ep.seriesName,
                images: ep.seriesImages ?? ImageSet(poster: ep.images.poster))
            return (item: show, episodes: eps.compactMap(\.item))
        }
    }

    @MainActor static func episodes(ofSeries id: String) -> [DownloadRecord] {
        DownloadManager.shared.episodes(ofSeries: id).filter(\.isPlayable)
    }
}

/// A downloaded title's poster card (artwork saved with the download).
struct OfflineTitleCard: View {
    let item: MediaItem
    let library: MediaItem
    let width: CGFloat

    /// The title's page (made from its downloads with no server); a film's plays as it opens.
    var body: some View {
        NavigationLink(value: Route.item(item, autoplay: item.isSeries ? nil : Autoplay())) { label }
            .cardButtonStyle()
    }

    private var poster: URL? {
        let recs = item.isSeries ? OfflineLibrary.episodes(ofSeries: item.id) : DownloadManager.shared.playable(forItem: item.id).map { [$0] } ?? []
        return recs.lazy.compactMap(\.posterURL).first
    }

    private var label: some View {
        VStack(alignment: .leading, spacing: 6) {
            RemoteImage(url: poster)
                .frame(width: width, height: width * 1.5)
                .overlay {
                    if poster == nil { Image(systemName: item.isSeries ? "tv" : "film").font(.title).foregroundStyle(Theme.secondaryText) }
                }
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                .overlay(alignment: .topLeading) { DownloadMark(item: item) }
                .overlay(alignment: .topTrailing) { WatchedMark(item: item) }
                #if os(tvOS)
                .lockupArtwork()
                #else
                .shadow(color: .black.opacity(0.4), radius: 6, y: 3)
                #endif
            Text(item.displayTitle).font(.caption.weight(.medium)).lineLimit(1).foregroundStyle(.white)
            Text(item.isSeries ? episodesLine : (item.productionYear.map(String.init) ?? " "))
                .font(.caption2).lineLimit(1).foregroundStyle(Theme.secondaryText)
        }
        .frame(width: width)
    }

    private var episodesLine: String {
        let n = OfflineLibrary.episodes(ofSeries: item.id).count
        return n == 1 ? "1 episode" : "\(n) episodes"
    }
}

/// Wide artwork tile with a glass label (category tiles).
struct LibraryTile: View {
    let title: String
    let icon: String
    let image: URL?
    var detail: String?

    var body: some View {
        ZStack(alignment: .bottomLeading) {
            if let image {
                RemoteImage(url: image)
            } else {
                LinearGradient(colors: [Theme.accent.opacity(0.22), Theme.surface], startPoint: .topLeading, endPoint: .bottomTrailing)
                    .overlay { Image(systemName: icon).font(.system(size: 40, weight: .light)).foregroundStyle(.white.opacity(0.45)) }
            }
            // A scrim under the label, so the name and count read on any artwork (light ones too).
            LinearGradient(stops: [.init(color: .clear, location: 0.25),
                                   .init(color: .black.opacity(0.45), location: 0.55),
                                   .init(color: .black.opacity(0.85), location: 1)],
                           startPoint: .top, endPoint: .bottom)
            VStack(alignment: .leading, spacing: 1) {
                Text(title).font(.headline).lineLimit(1)
                    #if os(tvOS)
                    .minimumScaleFactor(0.75) // long studio names, three tiles beside the sidebar
                    #endif
                if let detail { Text(detail).font(.caption.weight(.medium)).foregroundStyle(.white.opacity(0.85)) }
            }
            .foregroundStyle(.white)
            .shadow(color: .black.opacity(0.8), radius: 3, y: 1)
            #if os(tvOS)
            .padding(24) // inside the rounded corner, clear of the TV's focus highlight
            #else
            .padding(12)
            #endif
        }
        .aspectRatio(16 / 9, contentMode: .fit)
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        #if os(tvOS)
        .lockupArtwork()
        #else
        .shadow(color: .black.opacity(0.35), radius: 8, y: 4)
        #endif
    }
}

struct SearchView: View {
    @Environment(\.metrics) private var metrics
    @State private var query = ""
    @State private var results: [MediaItem] = []
    @State private var searching = false
    var client: (any MediaServerClient)? { SourceStore.shared.client }
    private var startStore: SearchStartStore { .shared }
    #if os(tvOS)
    @State var confirmClearRecent = false
    #endif
    #if os(macOS)
    @FocusState private var fieldFocused: Bool
    #endif

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: groupSpacing) {
                // Series and episodes can share a title (pilots), so keep the kinds visually apart.
                #if os(tvOS)
                if SearchStart.shows(query: query), let s = startStore.start {
                    if s.isEmpty {
                        startEmptyState
                    } else {
                        tvShelf("Recent", s.recent, clear: { startStore.clearRecent(client: client) })
                        tvShelf("Suggestions", s.suggestions)
                    }
                }
                // Home's shelves: same row titles, lockups and leading edge.
                tvShelf("Shows", results.filter(\.isSeries))
                tvShelf("Movies", results.filter(\.isMovie))
                tvShelf("Episodes", results.filter(\.isEpisode), landscape: true)
                #else
                startRows
                group("Shows", results.filter(\.isSeries))
                group("Movies", results.filter(\.isMovie))
                episodes(results.filter(\.isEpisode))
                #endif
                if searching { ProgressView().frame(maxWidth: .infinity) }
                if !searching && results.isEmpty && !query.isEmpty {
                    EmptyStateView(icon: "magnifyingglass", title: "No Results", message: "Nothing matched “\(query)”.")
                }
            }
            .padding(.vertical)
        }
        .background(Theme.backgroundGradient.ignoresSafeArea())
        #if os(tvOS)
        .navigationTitle("") // the tab bar already says Search
        #else
        .navigationTitle("Search")
        #endif
        #if os(iOS)
        .settingsButton(onlyOnPad: true)
        #endif
        #if os(iOS)
        // iPad: a full-width field under the title (the top trailing corner keeps the Settings
        // gear, where every tab has it); iPhone: as before.
        .searchable(text: $query, placement: metrics.regular ? .navigationBarDrawer(displayMode: .always) : .automatic,
                    prompt: "Movies, shows, episodes")
        #else
        .searchable(text: $query, prompt: "Movies, shows, episodes")
        #endif
        #if os(macOS)
        // ⌘F (Edit ▸ Search…) and the Search tab put the cursor in the field.
        .searchFocused($fieldFocused)
        .onChange(of: MacSearchFocus.shared.request) { _, _ in fieldFocused = true }
        .onAppear { fieldFocused = true }
        #endif
        // Each time Search shows (also back from a title): Recent and Suggestions.
        .onAppear { Task { await startStore.load(client) } }
        .task(id: query) {
            guard query.count >= 2 else { results = []; return }
            try? await Task.sleep(for: .milliseconds(350))
            guard !Task.isCancelled else { return }
            searching = true
            defer { searching = false }
            results = (try? await client?.search(query)) ?? []
        }
    }

    private var groupSpacing: CGFloat {
        #if os(tvOS)
        return 70 // Home's row spacing
        #else
        return 26
        #endif
    }

    /// iPad: Library's grid (its poster size, gaps and margins), so Shows and Movies line up.
    private var padGrid: (columns: [GridItem], card: CGFloat)? {
        guard metrics.regular else { return nil }
        let spacing: CGFloat = 12
        let avail = max(200, metrics.width - metrics.hPadding * 2)
        let n = max(2, Int((avail + spacing) / (metrics.gridMin + spacing)))
        let card = floor((avail - CGFloat(n - 1) * spacing) / CGFloat(n))
        return (Array(repeating: GridItem(.fixed(card), spacing: spacing, alignment: .top), count: n), card)
    }

    var startEmptyState: some View {
        EmptyStateView(icon: "magnifyingglass", title: "Search Your Library", message: "Find movies, shows and episodes by name.")
    }

    #if !os(tvOS)
    /// Nothing typed yet: Recent as a row with Clear, Suggestions as the results' grid. On a server
    /// without them, iPad shows what's new, as on Home (instead of an empty page).
    @ViewBuilder private var startRows: some View {
        if SearchStart.shows(query: query), let s = startStore.start {
            if s.isEmpty {
                startEmptyState
            } else {
                if !s.recent.isEmpty {
                    Shelf(title: "Recent", action: ("Clear", { startStore.clearRecent(client: client) })) {
                        ForEach(s.recent) { item in
                            NavigationLink(value: Route.searchItem(item, zoom: "search-recent/\(item.id)")) {
                                PosterCard(item: item, client: client, zoomID: "search-recent/\(item.id)")
                            }
                            .cardButtonStyle()
                            .watchedMenu(item, client: client)
                        }
                    }
                }
                group("Suggestions", s.suggestions)
            }
        } else if startStore.loaded, metrics.regular, query.isEmpty {
            ForEach(HomeStore.shared.sections.filter { $0.id != HomeSection.continueID }) { section in
                Shelf(title: section.title) {
                    ForEach(section.items) { item in
                        let zoomID = "search-\(section.id)/\(item.id)"
                        NavigationLink(value: Route.item(item, zoom: zoomID)) {
                            if section.style == .landscape {
                                LandscapeCard(item: item, client: client, zoomID: zoomID)
                            } else {
                                PosterCard(item: item, client: client, zoomID: zoomID)
                            }
                        }
                        .cardButtonStyle()
                        .watchedMenu(item, client: client)
                    }
                }
            }
        }
    }
    #endif

    @ViewBuilder private func group(_ title: String, _ items: [MediaItem]) -> some View {
        if !items.isEmpty {
            let pad = padGrid
            VStack(alignment: .leading, spacing: 10) {
                SectionHeader(title: title)
                LazyVGrid(columns: pad?.columns ?? [GridItem(.adaptive(minimum: Theme.gridMin), alignment: .top)],
                          alignment: pad == nil ? .center : .leading, spacing: pad == nil ? 18 : 20) {
                    ForEach(items) { item in
                        NavigationLink(value: Route.searchItem(item, zoom: "search/\(item.id)")) {
                            PosterCard(item: item, client: client, width: pad?.card ?? Theme.gridMin, zoomID: "search/\(item.id)")
                        }
                        .cardButtonStyle()
                        .watchedMenu(item, client: client)
                    }
                }
                .padding(.horizontal, metrics.hPadding)
            }
        }
    }

    @ViewBuilder private func episodes(_ items: [MediaItem]) -> some View {
        if !items.isEmpty {
            Shelf(title: "Episodes") {
                ForEach(items) { item in
                    NavigationLink(value: Route.searchItem(item, zoom: "search-episodes/\(item.id)")) {
                        LandscapeCard(item: item, client: client, zoomID: "search-episodes/\(item.id)")
                    }
                    .cardButtonStyle()
                    .watchedMenu(item, client: client)
                }
            }
        }
    }
}
