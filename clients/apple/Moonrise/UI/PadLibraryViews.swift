import SwiftUI

/// What the area beside the sidebar shows.
enum PadLibraryPage: Hashable {
    case grid
    case category(LibraryCategory)
    /// A category's tile: the library's titles with its filter (back goes to the category).
    case filtered(LibraryCategory, LibraryFilter, String)

    var category: LibraryCategory? {
        switch self {
        case .grid: nil
        case .category(let c), .filtered(let c, _, _): c
        }
    }

    var title: String? {
        if case let .filtered(_, _, t) = self { return t }
        return nil
    }

    /// The phone's pushed pages for it (portrait).
    var depth: Int {
        switch self {
        case .grid: 0
        case .category: 1
        case .filtered: 2
        }
    }
}

extension LibraryFilter {
    /// The category a tile's filter comes from.
    var category: LibraryCategory? {
        if collection != nil { return .collections }
        if studio != nil { return .studios }
        if !genres.isEmpty, locked?.contains(.genre) == true { return .genres }
        if decade != nil, locked?.contains(.decade) == true { return .decades }
        return nil
    }
}

#if !os(tvOS)

// The iPad Library (regular width) (and the Mac's): a sidebar in the tab (Movies, Shows, then the library's
// categories) beside the open page, in the manner of the Apple TV app (and Moonrise's tvOS Library,
// TVLibraryViews.swift). The page's header holds only its title, Sort and Filter; every filter is
// in one panel. Narrow windows and the iPhone keep the chip rows (`LibraryBrowser`'s own header).

enum PadLibraryMetrics {
    /// The sidebar: narrower when the window is taller than wide (the grid keeps 4 columns on the
    /// 11"); the phone in landscape: narrower still.
    static func sidebarWidth(_ m: Metrics) -> CGFloat { !m.regular ? 200 : m.wide ? 280 : 250 }
    /// The sidebar's title row and the page header's title row: one height, so their tops line up.
    static func titleRow(_ m: Metrics) -> CGFloat { m.regular ? 52 : 38 }
    static func rowHeight(_ m: Metrics) -> CGFloat { m.regular ? 44 : 34 }
    /// Space above the title rows (under the floating tab bar; the phone: under nothing).
    static func top(_ m: Metrics) -> CGFloat { m.regular ? 8 : 6 }
    /// The page title; the sidebar's "Library" is the same size, on the same line.
    static func titleFont(_ m: Metrics) -> Font { m.regular ? .largeTitle.weight(.bold) : .title2.weight(.bold) }
    /// Back, Sort, Filter: their height.
    static func buttonHeight(_ m: Metrics) -> CGFloat { m.regular ? 40 : 34 }
}

struct PadLibraryScreen: View {
    let source: Source
    let library: MediaItem
    let libraries: [MediaItem]
    @Binding var selection: String
    @Binding var page: PadLibraryPage
    let retry: () -> Void
    @Environment(\.metrics) private var metrics

    private var shows: Bool { library.isShowLibrary }

    /// As the phone's category row shows them: Collections only for movies.
    private var categories: [LibraryCategory] {
        LibraryCategory.allCases.filter { $0 != .collections || !shows }
    }

    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            sidebar
                .frame(width: PadLibraryMetrics.sidebarWidth(metrics))
            Rectangle()
                .fill(Color.white.opacity(0.08))
                .frame(width: 1)
                .ignoresSafeArea(edges: .bottom)
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        #endif
        .toolbar(removing: .title)
        #if os(iOS)
        // The phone in landscape: no bar over the page (it has nothing to show; height is short).
        .toolbar(metrics.regular ? .automatic : .hidden, for: .navigationBar)
        #endif
    }

    @ViewBuilder private var content: some View {
        switch page {
        case .grid:
            LibraryBrowser(source: source, library: library, libraries: libraries, selection: $selection,
                           retry: retry, padSidebar: true)
                .id(library.id)
        case .category(let c):
            LibraryCategoryView(source: source, library: library, category: c) { f, _ in
                page = .filtered(c, f, Self.title(of: f))
            }
            .id("\(library.id)-\(c.rawValue)")
        case let .filtered(c, f, title):
            LibraryBrowser(source: source, library: library, preset: f, title: title,
                           padSidebar: true, onBack: { page = .category(c) })
                .id("\(library.id)-\(c.rawValue)-\(title)")
        }
    }

    /// A tile's name, from the filter it sets.
    private static func title(of f: LibraryFilter) -> String {
        if let c = f.collection { return c.name ?? "Collection" }
        if let s = f.studio { return s }
        if let g = f.genres.first { return g }
        if let d = f.decade { return "\(d)s" }
        return "Filtered"
    }

    // MARK: Sidebar

    private var sidebar: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 2) {
                Text("Library")
                    .font(PadLibraryMetrics.titleFont(metrics))
                    .lineLimit(1)
                    .frame(height: PadLibraryMetrics.titleRow(metrics), alignment: .leading)
                    .padding(.horizontal, metrics.hPadding)
                    .padding(.bottom, metrics.regular ? 8 : 2)
                ForEach(libraries) { lib in
                    row(title: lib.displayTitle, icon: lib.isShowLibrary ? "tv" : "film",
                        on: page == .grid && lib.id == library.id,
                        // The library a category page belongs to.
                        current: page != .grid && lib.id == library.id) {
                        if lib.id != library.id { selection = lib.id }
                        page = .grid
                    }
                }
                // Libraries above, the open library's categories below.
                Color.clear.frame(height: metrics.regular ? 22 : 12)
                ForEach(categories) { c in
                    row(title: c.title(shows: shows), icon: c.icon(shows: shows), on: page.category == c) {
                        page = .category(c)
                    }
                }
            }
            .padding(.top, PadLibraryMetrics.top(metrics))
            .padding(.bottom, 20)
        }
        .scrollIndicators(.hidden)
    }

    private func row(title: String, icon: String, on: Bool, current: Bool = false, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 12) {
                // One icon column, so the names line up whatever the symbol's width.
                Image(systemName: icon)
                    .font(.body.weight(.medium))
                    .frame(width: 26)
                Text(title)
                    .lineLimit(1)
                    .fontWeight(on || current ? .semibold : .regular)
                Spacer(minLength: 0)
            }
            .foregroundStyle(.white.opacity(on || current ? 1 : 0.85))
            // The icon at the margin, level with "Library"; the tint reaches out around it.
            .padding(.horizontal, 10)
            .frame(height: PadLibraryMetrics.rowHeight(metrics))
            .background(RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(Color.white.opacity(on ? 0.14 : 0)))
            .contentShape(Rectangle())
            .padding(.horizontal, metrics.hPadding - 10)
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(on ? .isSelected : [])
    }
}

// MARK: - Header

/// The page header beside the sidebar: (back) title, then Sort and Filter at the trailing end.
struct PadLibraryHeader<Trailing: View>: View {
    let title: String
    var onBack: (() -> Void)?
    @ViewBuilder var trailing: () -> Trailing
    @Environment(\.metrics) private var metrics

    var body: some View {
        HStack(spacing: 12) {
            if let onBack {
                Button(action: onBack) {
                    Image(systemName: "chevron.left")
                        .font(.title3.weight(.semibold))
                        .frame(width: PadLibraryMetrics.buttonHeight(metrics), height: PadLibraryMetrics.buttonHeight(metrics))
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .glassEffect(.regular.interactive(), in: .circle)
                .accessibilityLabel("Back")
            }
            Text(title)
                .font(PadLibraryMetrics.titleFont(metrics))
                .lineLimit(1)
                .minimumScaleFactor(0.7)
            Spacer(minLength: 12)
            trailing()
        }
        .frame(height: PadLibraryMetrics.titleRow(metrics))
        .padding(.horizontal, metrics.hPadding)
        .padding(.top, PadLibraryMetrics.top(metrics))
    }
}

/// Sort: the current sort as the label ("↑ Title"); the sorts, then the direction.
struct PadSortMenu: View {
    @Binding var filter: LibraryFilter
    let offline: Bool
    /// iPhone held upright, when the sort's name doesn't fit beside the title: "↑ Sort".
    var compact = false
    /// Narrower still (a long tile name): only the arrow, in a circle as tall as Filter.
    var iconOnly = false
    @Environment(\.metrics) private var metrics

    var body: some View {
        Menu {
            Picker("Sort By", selection: Binding(get: { filter.sort }, set: { filter.setSort($0) })) {
                ForEach(LibrarySort.allCases.filter { $0 != .played || !offline }) { s in
                    Label(s.label, systemImage: s.icon).tag(s)
                }
            }
            .pickerStyle(.inline)
            Picker("Order", selection: $filter.descending) {
                Text(filter.sort.padDirections.up).tag(false)
                Text(filter.sort.padDirections.down).tag(true)
            }
            .pickerStyle(.inline)
        } label: {
            if iconOnly {
                let h = PadLibraryMetrics.buttonHeight(metrics)
                Image(systemName: filter.descending ? "arrow.down" : "arrow.up")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.white)
                    .frame(width: h, height: h)
                    .contentShape(Circle())
                    .glassEffect(.regular.interactive(), in: .circle)
            } else {
                PadHeaderButtonLabel(text: compact ? "Sort" : filter.sort.label, icon: filter.descending ? "arrow.down" : "arrow.up")
            }
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .drawnMenu()
        .accessibilityLabel("Sort by \(filter.sort.label), \(filter.descending ? filter.sort.padDirections.down : filter.sort.padDirections.up)")
    }
}

extension LibrarySort {
    /// The two directions' names (ascending, descending).
    var padDirections: (up: String, down: String) {
        switch self {
        case .title: ("A to Z", "Z to A")
        case .added, .year: ("Oldest First", "Newest First")
        case .rating: ("Lowest First", "Highest First")
        case .played: ("Least Recent First", "Most Recent First")
        case .runtime: ("Shortest First", "Longest First")
        }
    }
}

/// Sort's and Filter's look: a glass capsule, as tall as the back button.
struct PadHeaderButtonLabel: View {
    let text: String
    var icon: String?
    var badge: Int = 0
    @Environment(\.metrics) private var metrics

    var body: some View {
        HStack(spacing: 6) {
            if let icon { Image(systemName: icon).font(.subheadline.weight(.semibold)) }
            Text(text).font(.subheadline.weight(.semibold)).lineLimit(1)
            if badge > 0 {
                Text("\(badge)")
                    .font(.caption.weight(.bold))
                    .foregroundStyle(.black)
                    .frame(minWidth: 20, minHeight: 20)
                    .background(Circle().fill(.white))
            }
        }
        .foregroundStyle(.white)
        .padding(.horizontal, metrics.regular ? 16 : 12)
        .frame(height: PadLibraryMetrics.buttonHeight(metrics))
        .contentShape(Capsule())
        .glassEffect(.regular.interactive(), in: .capsule)
    }
}

/// A search field for the page header (Studios, Networks), as tall as Sort and Filter.
struct PadSearchField: View {
    @Binding var text: String
    let prompt: String
    @Environment(\.metrics) private var metrics

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField(prompt, text: $text)
                .textFieldStyle(.plain)
                .autocorrectionDisabled()
                #if os(iOS)
                .textInputAutocapitalization(.never)
                #endif
                .submitLabel(.search)
            if !text.isEmpty {
                Button { text = "" } label: { Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary) }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Clear text")
            }
        }
        .font(.subheadline)
        .padding(.horizontal, 12)
        .frame(width: metrics.regular ? 240 : 180, height: PadLibraryMetrics.buttonHeight(metrics))
        .glassEffect(.regular.interactive(), in: .capsule)
    }
}

// MARK: - Filter panel

/// Every filter of the chip rows in one popover: a list of the facets (each opening its choices,
/// with counts), the switches, and Clear All. Choices apply as they're made; the grid follows.
struct PadFilterPanel: View {
    enum Facet: String, CaseIterable, Identifiable, Hashable {
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
    let offline: Bool
    let total: Int
    let loading: Bool
    var start: Facet?
    @State private var path: [Facet] = []
    @Environment(\.dismiss) private var dismiss

    private var hasDownloads: Bool {
        DownloadManager.isSupported && DownloadManager.shared.records.contains(where: \.isPlayable)
    }

    var body: some View {
        NavigationStack(path: $path) {
            List {
                Section {
                    // The page's own (a category tile's page) isn't offered.
                    ForEach(Facet.allCases.filter { !($0 == .genre && filter.isLocked(.genre)) && !($0 == .year && filter.isLocked(.decade))
                        && ($0 != .library || LibraryFilterBar.offersLibraries(facets, filter)) }) { f in
                        NavigationLink(value: f) {
                            HStack(spacing: 12) {
                                Text(f.title)
                                Spacer(minLength: 12)
                                Text(value(f)).foregroundStyle(.secondary).lineLimit(1)
                                #if os(macOS)
                                // Mac lists draw no disclosure for a link: show that it opens a list.
                                Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
                                #endif
                            }
                        }
                    }
                }
                Section {
                    if (facets?.hdr ?? 0) > 0 || filter.hdr {
                        toggle("HDR", count: facets?.hdr, isOn: $filter.hdr)
                    }
                    toggle("Recently Added", count: facets?.recentlyAdded, isOn: $filter.recentlyAdded)
                    if DownloadManager.isSupported, !offline, hasDownloads || filter.downloaded {
                        toggle("Downloaded", count: nil, isOn: $filter.downloaded)
                    }
                }
                #if os(macOS)
                Section {
                    // A row like the others (not a small bordered button), red when there's something to clear.
                    Button(role: .destructive) { filter = filter.cleared } label: {
                        Text("Clear All").foregroundStyle(filter.isFiltered ? .red : .secondary)
                            .frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .disabled(!filter.isFiltered)
                } footer: {
                    // The popover has no bar for the count: under the list.
                    HStack(spacing: 6) {
                        Text(countTitle).monospacedDigit()
                        if loading { ProgressView().controlSize(.mini) }
                    }
                    .font(.footnote).foregroundStyle(.secondary)
                }
                #else
                Section {
                    Button("Clear All", role: .destructive) { filter = filter.cleared }
                        .disabled(!filter.isFiltered)
                }
                #endif
            }
            .navigationTitle("Filter")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: Self.countPlacement) {
                    HStack(spacing: 6) {
                        Text(countTitle).font(.footnote).foregroundStyle(.secondary).monospacedDigit()
                        if loading { ProgressView().controlSize(.mini) }
                    }
                    .fixedSize()
                }
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
            .navigationDestination(for: Facet.self) { options($0) }
        }
        .task { if let start, path.isEmpty { path = [start] } }
    }

    private var countTitle: String {
        "\(total) " + (isShows ? (total == 1 ? "show" : "shows") : (total == 1 ? "movie" : "movies"))
    }

    private func toggle(_ title: String, count: Int?, isOn: Binding<Bool>) -> some View {
        Toggle(isOn: isOn) {
            HStack {
                Text(title)
                Spacer(minLength: 12)
                if let count { Text("\(count)").foregroundStyle(.secondary).monospacedDigit() }
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
                Button { clear(f) } label: { optionRow(anyLabel, count: nil, on: value(f) == "Any") }
            }
            Section {
                ForEach(list) { o in
                    Button { o.toggle(&filter) } label: { optionRow(o.label, count: o.count, on: o.on) }
                }
            }
        }
        .navigationTitle(f.title)
        #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        #endif
        .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
    }

    private static var countPlacement: ToolbarItemPlacement {
        #if os(iOS)
        return .topBarLeading
        #else
        return .cancellationAction
        #endif
    }

    private func optionRow(_ label: String, count: Int?, on: Bool) -> some View {
        HStack(spacing: 12) {
            Image(systemName: "checkmark")
                .font(.body.weight(.semibold))
                .foregroundStyle(.tint)
                .opacity(on ? 1 : 0)
                .frame(width: 20)
            Text(label).lineLimit(1).foregroundStyle(.primary)
            Spacer(minLength: 12)
            if let count { Text("\(count)").foregroundStyle(.secondary).monospacedDigit() }
        }
        .contentShape(Rectangle())
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
#endif

#if os(iOS)
/// The Library tab's stack: turning the phone carries the open page between the pushed pages
/// (portrait) and the page beside the sidebar (landscape), with its filters and sort. On the
/// stack itself, so it runs whichever page is on top.
struct LibraryTurn: ViewModifier {
    @Environment(\.metrics) private var metrics

    func body(content: Content) -> some View {
        content.onChange(of: metrics.sidebarLibrary) { _, sidebar in
            let nav = AppNavigation.shared
            if sidebar {
                guard let top = nav.libraryPage, nav.libraryPath.count == top.depth else { return }
                nav.libraryPadPage = top
                nav.libraryPage = nil
                nav.libraryPath = NavigationPath()
            } else {
                let page = nav.libraryPadPage
                guard page != .grid, nav.libraryPath.isEmpty else { return }
                let id = UserDefaults.standard.string(forKey: "library.selected") ?? ""
                let lib = LibraryList.view(for: id)
                var path = NavigationPath()
                switch page {
                case .grid: break
                case .category(let c): path.append(Route.category(lib, c))
                case let .filtered(c, f, t): path.append(Route.category(lib, c)); path.append(Route.filtered(lib, f, t))
                }
                nav.libraryPadPage = .grid
                nav.libraryPath = path
            }
        }
    }
}
#endif
