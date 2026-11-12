#if os(tvOS)
import SwiftUI

// The Apple TV Library: a sidebar (Movies, Shows, then the library's categories) beside one
// slim header bar and the grid, in the manner of the Apple TV app. Filters open in a
// panel from the right. The data (filters, facets, loading, per-library memory) is the shared
// `LibraryBrowser` / `LibraryCategoryView` / `LibraryFilter`; only the presentation is tvOS's.

enum TVLibraryMetrics {
    /// The sidebar, from the 80-pt safe edge; the content starts a gutter to its right.
    static let sidebarWidth: CGFloat = 400
    static let gutter: CGFloat = 60
    /// The header bar's height: the system buttons' height plus room for their focus growth.
    static let barHeight: CGFloat = 80
    /// The header's glass buttons (as tall as the player's Skip button).
    static let buttonHeight: CGFloat = 66
    /// Sidebar and filter-panel rows: glass under the focused row (no white platter).
    static let rowStyle = TVGlassRowStyle(cornerRadius: 40, outset: CGSize(width: 20, height: 14), scale: 1.03)
    static let rowGap: CGFloat = 8
    /// Between the header's buttons.
    static let buttonGap: CGFloat = 30
    /// Under the header: room for the first row's focused artwork to grow.
    static let headerBottom: CGFloat = 36
    static let titleFont: Font = .title3.weight(.semibold)
    /// Moves the sidebar's first row level with the header bar.
    static let sidebarTop: CGFloat = 19

    static func countTitle(_ n: Int, shows: Bool) -> String {
        "\(n) " + (shows ? (n == 1 ? "Show" : "Shows") : (n == 1 ? "Movie" : "Movies"))
    }

    /// The filters that are on, as one line ("Action · 1990s · Unwatched").
    static func summary(_ f: LibraryFilter) -> String {
        var parts: [String] = []
        if let c = f.collection { parts.append(c.name ?? "Collection") }
        if let s = f.studio { parts.append(s) }
        parts += f.genres.sorted()
        if let d = f.decade { parts.append("\(d)s") }
        parts += f.ratings.sorted().map(LibraryFilterBar.ratingLabel)
        if f.watched != .all { parts.append(f.watched.label) }
        parts += f.resolutions.sorted().map(\.label)
        if f.hdr { parts.append("HDR") }
        if f.downloaded { parts.append("Downloaded") }
        if f.recentlyAdded { parts.append("Added in \(LibraryFilter.recentDays) days") }
        return parts.joined(separator: " · ")
    }
}

extension LibrarySort {
    /// The two directions' names for this sort (ascending, descending).
    var directions: (up: String, down: String) {
        switch self {
        case .title: ("A to Z", "Z to A")
        case .added, .year: ("Oldest First", "Newest First")
        case .rating: ("Lowest First", "Highest First")
        case .played: ("Least Recent First", "Most Recent First")
        case .runtime: ("Shortest First", "Longest First")
        }
    }
}

/// A header button's label: system button text with an optional symbol and a menu chevron.
struct TVHeaderLabel: View {
    let text: String
    var icon: String?
    var menu = false

    var body: some View {
        HStack(spacing: 12) {
            if let icon { Image(systemName: icon) }
            Text(text).lineLimit(1)
            if menu { Image(systemName: "chevron.down").font(.caption.weight(.bold)) }
        }
    }
}

/// Sort: the sorts and the two directions, each with a checkmark on the current one.
struct TVSortMenu: View {
    @Binding var filter: LibraryFilter
    let offline: Bool

    var body: some View {
        Menu {
            Picker("Sort By", selection: Binding(get: { filter.sort }, set: { s in
                guard s != filter.sort else { return }
                filter.sort = s
                filter.descending = s.descending
            })) {
                ForEach(LibrarySort.allCases.filter { $0 != .played || !offline }) { s in
                    Label(s.label, systemImage: s.icon).tag(s)
                }
            }
            .pickerStyle(.inline)
            Divider()
            Picker("Order", selection: $filter.descending) {
                Text(filter.sort.directions.up).tag(false)
                Text(filter.sort.directions.down).tag(true)
            }
            .pickerStyle(.inline)
        } label: {
            TVHeaderLabel(text: "Sort: \(filter.sort.label)", icon: "arrow.up.arrow.down", menu: true)
        }
        .accessibilityLabel("Sort by \(filter.sort.label), \(filter.descending ? filter.sort.directions.down : filter.sort.directions.up)")
    }
}

// MARK: - Focus between the sidebar and the content

extension EnvironmentValues {
    /// Moves focus to the sidebar (left from the content's leading edge).
    @Entry var tvToSidebar: (() -> Void)? = nil
    /// Bumped when focus should come back from the sidebar to the grid's last poster.
    @Entry var tvGridFocusRequest = 0
    /// The content takes focus once it has loaded (it replaced the focused page): "" its first
    /// item, else the item with this id.
    @Entry var tvFocusOnLoad: String? = nil
    /// The room the sidebar takes at the content's leading edge (padding inside the content's
    /// scroll view: as safe-area padding, the header's buttons were found where they'd be
    /// without it, so focus couldn't move right from Sort to Filter, or down from Sort).
    @Entry var tvContentLeading: CGFloat = 0
}

/// The content's scroll view spans the screen (so the tab bar scrolls away with it, as on every
/// other screen), which keeps the focus engine from moving left out of it into the sidebar: the
/// leading column hands focus over itself.
private struct LeadingEdge: ViewModifier {
    let isEdge: Bool
    @Environment(\.tvToSidebar) private var toSidebar
    func body(content: Content) -> some View {
        content.onMoveCommand { if $0 == .left, isEdge { toSidebar?() } }
    }
}

extension View {
    /// tvOS Library: left from this view goes to the sidebar (it's at the content's leading edge).
    func tvLeadingEdge(_ isEdge: Bool = true) -> some View { modifier(LeadingEdge(isEdge: isEdge)) }
}

/// The filter panel is open (the sidebar steps back under it).
struct TVPanelOpenKey: PreferenceKey {
    static let defaultValue = false
    static func reduce(value: inout Bool, nextValue: () -> Bool) { value = value || nextValue() }
}

// MARK: - Screen

/// Where the content beside the sidebar is.
enum TVLibraryPage: Hashable {
    case grid
    case category(LibraryCategory)
}

#if DEBUG
/// MOONRISE_LIBRARY on tvOS: picks the library, sets its filters to the seam's and opens the page,
/// once per launch (the sidebar takes over from there).
enum TVLibrarySeam {
    private static var applied = false
    static var page: TVLibraryPage?

    static func applyOnce() {
        guard !applied, let seam = LibrarySeam.current else { return }
        applied = true
        LibrarySeam.applyOnce() // the library and its filters
        page = seam.page.flatMap(LibraryCategory.init(rawValue:)).map { .category($0) }
    }
}
#endif

struct TVLibraryScreen: View {
    let source: Source
    let library: MediaItem
    let libraries: [MediaItem]
    @Binding var selection: String
    let retry: () -> Void

    @State private var page: TVLibraryPage
    /// The category whose tile filtered the grid: Menu goes back to it.
    @State private var cameFrom: LibraryCategory?
    /// The tile picked there (focused again on the way back).
    @State private var pickedId: String?
    /// Bumped to rebuild the grid with a category's filter.
    @State private var generation = 0
    @State private var gridFocusRequest = 0
    @State private var panelOpen = false
    /// The page was switched from the content (a tile, or Menu back): it takes focus on load.
    @State private var focusOnLoad: String?
    @FocusState private var focus: Row?

    enum Row: Hashable {
        case library(String)
        case category(LibraryCategory)
    }

    init(source: Source, library: MediaItem, libraries: [MediaItem], selection: Binding<String>, retry: @escaping () -> Void) {
        self.source = source
        self.library = library
        self.libraries = libraries
        _selection = selection
        self.retry = retry
        var start = TVLibraryPage.grid
        #if DEBUG
        if let p = TVLibrarySeam.page { start = p; TVLibrarySeam.page = nil }
        #endif
        _page = State(initialValue: start)
    }

    private var shows: Bool { library.isShowLibrary }

    private var categories: [LibraryCategory] {
        LibraryCategory.allCases.filter { $0 != .collections || !shows }
    }

    private var currentRow: Row {
        switch page {
        case .grid: .library(library.id)
        case .category(let c): .category(c)
        }
    }

    var body: some View {
        // The content first: its scroll view is the screen's (the tab bar scrolls away with it).
        ZStack(alignment: .topLeading) {
            content
                .onExitCommand { back() }
                .environment(\.tvToSidebar, { focus = currentRow })
                .environment(\.tvGridFocusRequest, gridFocusRequest)
                .environment(\.tvFocusOnLoad, focusOnLoad)
                .environment(\.tvContentLeading, TVLibraryMetrics.sidebarWidth + TVLibraryMetrics.gutter)
                .onPreferenceChange(TVPanelOpenKey.self) { panelOpen = $0 }
            // Dimmed like the grid while the filter panel (drawn by the grid) is open.
            sidebar
                .frame(width: TVLibraryMetrics.sidebarWidth)
                .disabled(panelOpen)
                .opacity(panelOpen ? 0.6 : 1)
                .animation(.easeOut(duration: 0.25), value: panelOpen)
        }
    }

    @ViewBuilder private var content: some View {
        switch page {
        case .grid:
            LibraryBrowser(source: source, library: library, libraries: libraries, selection: $selection, retry: retry)
                .id("\(library.id)#\(generation)")
        case .category(let c):
            LibraryCategoryView(source: source, library: library, category: c) { pick($0, from: c, id: $1) }
                .id("\(library.id)-\(c.rawValue)")
        }
    }

    // MARK: Sidebar

    private var sidebar: some View {
        List {
            ForEach(libraries) { lib in
                row(.library(lib.id), title: lib.displayTitle, icon: lib.isShowLibrary ? "tv" : "film")
            }
            // Libraries above, the open library's categories below.
            Divider()
                .padding(.horizontal, 20)
                .padding(.vertical, 16)
                .listRowBackground(Color.clear)
            ForEach(categories) { c in
                row(.category(c), title: c.title(shows: shows), icon: c.icon(shows: shows))
            }
        }
        // The first row's middle on the header bar's.
        .padding(.top, TVLibraryMetrics.sidebarTop)
        // Right: back to the poster the grid had (the focus engine alone picks the nearest).
        .onMoveCommand { if $0 == .right, page == .grid { gridFocusRequest += 1 } }
        .focusSection()
        // Moving in from the grid (or Menu) lands on the open page's row.
        .defaultFocus($focus, currentRow, priority: .userInitiated)
        .scrollClipDisabled()
    }

    private func row(_ r: Row, title: String, icon: String) -> some View {
        Button { select(r) } label: {
            HStack(spacing: 0) {
                // One icon column, so the names line up whatever the symbol's width.
                Image(systemName: icon)
                    .frame(width: 44)
                    .padding(.trailing, 20)
                Text(title).lineLimit(1)
                    // The library a category page belongs to.
                    .fontWeight(r != currentRow && r == .library(library.id) ? .semibold : nil)
                Spacer(minLength: 0)
            }
        }
        .buttonStyle(TVLibraryMetrics.rowStyle)
        .focused($focus, equals: r)
        // The open page: a tinted row, the shape and size of the focus platter (as the system's
        // sidebars mark theirs).
        .listRowBackground(Capsule().fill(.white.opacity(r == currentRow ? 0.16 : 0)))
    }

    private func select(_ r: Row) {
        cameFrom = nil
        focusOnLoad = nil
        switch r {
        case .library(let id):
            if id != library.id { selection = id }
            page = .grid
        case .category(let c):
            page = .category(c)
        }
    }

    /// A category tile: the library's grid with that filter (the sort stays, except decades and
    /// collections, which read best oldest first).
    private func pick(_ f: LibraryFilter, from c: LibraryCategory, id: String) {
        var x = f
        if c == .genres || c == .studios {
            let current = LibraryFilter.remembered(for: library) ?? LibraryFilter()
            x.sort = current.sort
            x.descending = current.descending
        }
        x.save(for: library.id)
        cameFrom = c
        pickedId = id
        focusOnLoad = ""
        generation += 1
        page = .grid
    }

    /// Menu in the content: back to the category a tile came from, else to the sidebar.
    private func back() {
        if page == .grid, let c = cameFrom {
            cameFrom = nil
            focusOnLoad = pickedId ?? ""
            page = .category(c)
        } else {
            focus = currentRow
        }
    }
}

// MARK: - Filter panel

/// Every filter as a list with counts, in a full-height panel from the right. Choices apply as
/// they're made (the grid behind follows); Menu goes back a list, then closes.
struct TVFilterPanel: View {
    enum Facet: String, CaseIterable, Identifiable {
        case library, genre, year, rating, watched, resolution
        var id: String { rawValue }
        var title: String {
            switch self {
            case .library: "Library"
            case .genre: "Genre"
            case .year: "Year"
            case .rating: "Rating"
            case .watched: "Watched"
            case .resolution: "Resolution"
            }
        }
    }

    @Binding var filter: LibraryFilter
    let facets: LibraryFacets?
    let isShows: Bool
    let total: Int
    let loading: Bool
    var start: Facet?
    let close: () -> Void

    /// The open list (nil: the main list).
    @State private var open: Facet?
    @FocusState private var focused: String?

    static let width: CGFloat = 820

    // Drawn over the Library (not presented: a presentation takes Menu to close itself, while
    // here Menu first goes back to the main list).
    var body: some View {
        HStack(spacing: 0) {
            Color.black.opacity(0.4)
            page
                .frame(width: Self.width)
                .frame(maxHeight: .infinity)
                .background(.regularMaterial)
                .environment(\.colorScheme, .dark)
        }
        .ignoresSafeArea()
        .focusSection()
        .onExitCommand {
            if let o = open {
                open = nil
                // Back on the row it came from (once the main list is there again).
                Task {
                    try? await Task.sleep(for: .milliseconds(60))
                    focused = o.rawValue
                }
            } else {
                close()
            }
        }
        .task {
            open = start
            try? await Task.sleep(for: .milliseconds(50))
            if start == nil { focused = facetsShown.first?.rawValue }
        }
    }

    private var page: some View {
        VStack(alignment: .leading, spacing: 24) {
            HStack(alignment: .firstTextBaseline, spacing: 20) {
                Text(open?.title ?? "Filter").font(TVLibraryMetrics.titleFont)
                Spacer(minLength: 20)
                if loading { ProgressView().scaleEffect(0.7) }
                Text(TVLibraryMetrics.countTitle(total, shows: isShows)).font(.callout).foregroundStyle(.secondary)
            }
            .lineLimit(1)
            .padding(.horizontal, 20)
            Group {
                if let open { options(open) } else { main }
            }
            .buttonStyle(TVLibraryMetrics.rowStyle)
            .scrollClipDisabled()
        }
        // The safe area's top and trailing edges (the rows' focus platter overhangs by 20).
        .padding(.top, 60)
        .padding(.leading, 60)
        .padding(.trailing, 80)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
    }

    // MARK: Main list

    private var main: some View {
        List {
            Section {
                ForEach(facetsShown) { f in
                    Button { open = f } label: {
                        HStack(spacing: 16) {
                            Text(f.title)
                            Spacer(minLength: 16)
                            Text(value(f)).foregroundStyle(.secondary).lineLimit(1)
                            Image(systemName: "chevron.right").font(.caption.weight(.bold)).foregroundStyle(.secondary)
                        }
                    }
                    .focused($focused, equals: f.rawValue)
                }
                if (facets?.hdr ?? 0) > 0 || filter.hdr {
                    toggleRow("HDR", count: facets?.hdr, isOn: $filter.hdr)
                }
                toggleRow("Recently Added", count: facets?.recentlyAdded, isOn: $filter.recentlyAdded)
            }
            Section {
                Button("Clear All") { filter = filter.cleared }
                    .disabled(!filter.isFiltered)
            }
        }
    }

    /// Library only with several libraries (libraries have no type: it's a filter).
    private var facetsShown: [Facet] { Facet.allCases.filter { $0 != .library || LibraryFilterBar.offersLibraries(facets, filter) } }

    private func toggleRow(_ title: String, count: Int?, isOn: Binding<Bool>) -> some View {
        Button { isOn.wrappedValue.toggle() } label: {
            HStack(spacing: 16) {
                Text(title)
                Spacer(minLength: 16)
                if let count { Text("\(count)").foregroundStyle(.secondary) }
                Checkmark(on: isOn.wrappedValue)
            }
        }
    }

    private func value(_ f: Facet) -> String {
        switch f {
        case .library:
            filter.library == nil ? "Any" : (filter.libraryName ?? "1 library")
        case .genre:
            filter.genres.isEmpty ? "Any" : filter.genres.sorted().joined(separator: ", ")
        case .year:
            filter.decade.map { "\($0)s" } ?? "Any"
        case .rating:
            filter.ratings.isEmpty ? "Any" : filter.ratings.sorted().map(LibraryFilterBar.ratingLabel).joined(separator: ", ")
        case .watched:
            filter.watched == .all ? "Any" : filter.watched.label
        case .resolution:
            filter.resolutions.isEmpty ? "Any" : filter.resolutions.sorted().map(\.label).joined(separator: ", ")
        }
    }

    // MARK: Option lists

    private struct Option: Identifiable {
        let id: String
        let label: String
        let count: Int?
        let on: Bool
        let toggle: (inout LibraryFilter) -> Void
    }

    private func options(_ f: Facet) -> some View {
        let (anyLabel, list) = options(for: f)
        return List {
            Section {
                Button { clear(f) } label: {
                    HStack(spacing: 16) {
                        Text(anyLabel)
                        Spacer(minLength: 16)
                        Checkmark(on: value(f) == "Any")
                    }
                }
            }
            Section {
                ForEach(list) { o in
                    Button { o.toggle(&filter) } label: {
                        HStack(spacing: 16) {
                            Text(o.label).lineLimit(1)
                            Spacer(minLength: 16)
                            if let n = o.count { Text("\(n)").foregroundStyle(.secondary).monospacedDigit() }
                            Checkmark(on: o.on)
                        }
                    }
                }
            }
        }
        .id(f) // a new list starts at its top
    }

    private func clear(_ f: Facet) {
        switch f {
        case .library: filter.library = nil; filter.libraryName = nil
        case .genre: filter.genres = []
        case .year: filter.decade = nil
        case .rating: filter.ratings = []
        case .watched: filter.watched = .all
        case .resolution: filter.resolutions = []
        }
    }

    private func options(for f: Facet) -> (String, [Option]) {
        switch f {
        case .library:
            return ("All Libraries", (facets?.libraryChoices(filter.library) ?? []).map { l in
                Option(id: l.id, label: l.name, count: l.count, on: filter.library == l.id) {
                    let on = $0.library == l.id
                    $0.library = on ? nil : l.id
                    $0.libraryName = on ? nil : l.name
                }
            })
        case .genre:
            let list = merged(facets?.genres ?? [], selected: filter.genres)
            return ("Any Genre", list.map { g in
                Option(id: g.name, label: g.name, count: g.count, on: filter.genres.contains(g.name)) {
                    if $0.genres.contains(g.name) { $0.genres.remove(g.name) } else { $0.genres.insert(g.name) }
                }
            })
        case .year:
            var decades = facets?.decades ?? []
            if let d = filter.decade, !decades.contains(where: { $0.decade == d }) { decades.append(.init(decade: d, count: 0)) }
            return ("Any Year", decades.map { d in
                Option(id: d.label, label: d.label, count: d.count, on: filter.decade == d.decade) {
                    $0.decade = $0.decade == d.decade ? nil : d.decade
                }
            })
        case .rating:
            let list = merged(facets?.ratings ?? [], selected: filter.ratings)
            return ("Any Rating", list.map { r in
                Option(id: r.name, label: LibraryFilterBar.ratingLabel(r.name), count: r.count, on: filter.ratings.contains(r.name)) {
                    if $0.ratings.contains(r.name) { $0.ratings.remove(r.name) } else { $0.ratings.insert(r.name) }
                }
            })
        case .watched:
            return ("Any", [WatchedFilter.unwatched, .inProgress, .watched].map { w in
                Option(id: w.rawValue, label: w.label, count: facets?.watched?[w], on: filter.watched == w) {
                    $0.watched = $0.watched == w ? .all : w
                }
            })
        case .resolution:
            let available = Dictionary((facets?.resolutions ?? []).map { ($0.name, $0.count) }, uniquingKeysWith: +)
            let shown = ResolutionFilter.allCases.filter { available[$0.rawValue] != nil || filter.resolutions.contains($0) }
            return ("Any Resolution", shown.map { r in
                Option(id: r.rawValue, label: r.label, count: available[r.rawValue] ?? 0, on: filter.resolutions.contains(r)) {
                    if $0.resolutions.contains(r) { $0.resolutions.remove(r) } else { $0.resolutions.insert(r) }
                }
            })
        }
    }

    /// The facet's values, plus selected ones that have no titles under the other filters.
    private func merged(_ values: [LibraryFacets.Value], selected: Set<String>) -> [LibraryFacets.Value] {
        values + selected.filter { s in !values.contains { $0.name == s } }.sorted().map { LibraryFacets.Value(name: $0, count: 0) }
    }
}

/// A list row's trailing checkmark (its slot kept when off, so the counts line up).
private struct Checkmark: View {
    let on: Bool
    var body: some View {
        Image(systemName: "checkmark")
            .font(.body.weight(.semibold))
            .opacity(on ? 1 : 0)
            .frame(width: 34)
    }
}
#endif
