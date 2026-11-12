import SwiftUI

// The Library's filter chips, the active-filter row and the category pages (Phase 7).

extension EnvironmentValues {
    /// tvOS: the chip's button has the focus (`ChipFocusStyle`).
    @Entry var chipFocused = false
}

#if os(tvOS)
/// tvOS chips: the focused one turns white and lifts, like the system's buttons.
struct ChipFocusStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View { Lift(configuration: configuration) }
    struct Lift: View {
        let configuration: Configuration
        @Environment(\.isFocused) private var focused
        var body: some View {
            configuration.label
                .environment(\.chipFocused, focused)
                .scaleEffect(configuration.isPressed ? 1.04 : focused ? 1.1 : 1)
                .shadow(color: .black.opacity(focused ? 0.5 : 0), radius: 16, y: 8)
                .animation(.easeOut(duration: 0.15), value: focused)
        }
    }
}
#endif

/// A filter chip: glass when off, white when on (the app's accent is white). tvOS: the focused
/// chip is white, an active one light grey.
struct FilterChipLabel: View {
    let text: String
    var icon: String?
    var active = false
    var menu = false
    @Environment(\.chipFocused) private var focused

    var body: some View {
        HStack(spacing: 5) {
            if let icon { Image(systemName: icon).imageScale(.small) }
            Text(text).lineLimit(1)
            if menu { Image(systemName: "chevron.down").font(.caption2.weight(.bold)).opacity(0.7) }
        }
        #if os(tvOS)
        .font(.callout.weight(.semibold))
        .padding(.horizontal, 22).padding(.vertical, 12)
        #else
        .font(.subheadline.weight(.semibold))
        .padding(.horizontal, 13).padding(.vertical, 8)
        #endif
        .foregroundStyle(active || focused ? Color.black : .white)
        .modifier(ChipBackground(active: active, focused: focused, shape: Capsule()))
        .contentShape(Capsule())
    }
}

struct ChipBackground<S: Shape>: ViewModifier {
    let active: Bool
    var focused = false
    let shape: S
    func body(content: Content) -> some View {
        if focused {
            content.background(shape.fill(.white))
        } else if active {
            #if os(tvOS)
            content.background(shape.fill(.white.opacity(0.78)))
            #else
            content.background(shape.fill(.white))
            #endif
        } else {
            content.glassEffect(.regular, in: shape)
        }
    }
}

/// A category button's label (white when focused on tvOS).
private struct CategoryLabel: View {
    let title: String
    let icon: String
    @Environment(\.chipFocused) private var focused

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: icon)
            Text(title)
        }
        #if os(tvOS)
        .font(.callout.weight(.semibold))
        .padding(.horizontal, 26).padding(.vertical, 16)
        #else
        .font(.subheadline.weight(.semibold))
        .padding(.horizontal, 16).padding(.vertical, 11)
        #endif
        .foregroundStyle(focused ? Color.black : .white)
        .modifier(ChipBackground(active: false, focused: focused, shape: RoundedRectangle(cornerRadius: 14)))
        .contentShape(RoundedRectangle(cornerRadius: 14))
    }
}

/// A removable active filter (white when focused on tvOS).
private struct TokenLabel: View {
    let text: String
    @Environment(\.chipFocused) private var focused

    var body: some View {
        HStack(spacing: Self.gap) {
            Text(text).lineLimit(1)
            Image(systemName: "xmark").font(.caption2.weight(.bold))
        }
        #if os(tvOS)
        // As tall as the filter chips above (FilterChipLabel).
        .font(.callout.weight(.semibold))
        .padding(.horizontal, 22).padding(.vertical, 12)
        #else
        .font(.caption.weight(.semibold))
        .padding(.horizontal, 10).padding(.vertical, 5)
        #endif
        .foregroundStyle(focused ? Color.black : .white)
        .background(Capsule().fill(focused ? Color.white : Theme.surfaceStrong))
        .contentShape(Capsule())
    }

    #if os(tvOS)
    private static let gap: CGFloat = 10
    #else
    private static let gap: CGFloat = 4
    #endif
}

private extension View {
    /// Multi-select menus stay open while picking (macOS menus close).
    @ViewBuilder func keepsMenuOpen() -> some View {
        #if os(macOS)
        self
        #else
        self.menuActionDismissBehavior(.disabled)
        #endif
    }

    /// Menus and buttons that draw their own chip.
    func chipButton() -> some View {
        #if os(tvOS)
        self.buttonStyle(ChipFocusStyle())
        #else
        self.buttonStyle(.plain)
        #endif
    }
}

/// Sort, Library (with several), Genre, Decade, Rating, Watched, Resolution menus and HDR /
/// Downloaded / Recently Added toggles.
struct LibraryFilterBar: View {
    @Environment(\.metrics) private var metrics
    @Binding var filter: LibraryFilter
    let facets: LibraryFacets?
    let isShows: Bool
    let offline: Bool
    /// tvOS: the alphabet index as a menu (title sort).
    var letters: [String] = []
    var onJump: ((String) -> Void)?

    private var hasDownloads: Bool {
        DownloadManager.isSupported && DownloadManager.shared.records.contains(where: \.isPlayable)
    }

    var body: some View {
        ScrollView(.horizontal) {
            HStack(spacing: chipSpacing) {
                sortMenu
                if Self.offersLibraries(facets, filter) { libraryMenu }
                if !filter.isLocked(.genre) { genreMenu }
                if !filter.isLocked(.decade) { decadeMenu }
                ratingMenu
                watchedMenu
                resolutionMenu
                if (facets?.hdr ?? 0) > 0 || filter.hdr {
                    toggle("HDR", icon: "sparkles", on: filter.hdr) { filter.hdr.toggle() }
                }
                if !offline, hasDownloads || filter.downloaded {
                    toggle("Downloaded", icon: "arrow.down.circle", on: filter.downloaded) { filter.downloaded.toggle() }
                }
                toggle("Recently Added", icon: "clock", on: filter.recentlyAdded) { filter.recentlyAdded.toggle() }
                #if os(tvOS)
                if !letters.isEmpty, let onJump {
                    Menu {
                        ForEach(letters, id: \.self) { l in Button(l) { onJump(l) } }
                    } label: { FilterChipLabel(text: "Jump to", icon: "textformat.abc", menu: true) }
                    .menuStyle(.button)
                    .chipButton()
                }
                #endif
            }
            .padding(.horizontal, metrics.hPadding)
            #if os(tvOS)
            .padding(.vertical, 14)
            #endif
        }
        .scrollIndicators(.hidden)
        #if os(tvOS)
        .scrollClipDisabled()
        .focusSection()
        #endif
    }

    private var chipSpacing: CGFloat {
        #if os(tvOS)
        return 20
        #else
        return 8
        #endif
    }

    private func toggle(_ title: String, icon: String, on: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) { FilterChipLabel(text: title, icon: on ? "checkmark" : icon, active: on) }
            .chipButton()
    }

    private func count(_ n: Int?) -> String { n.map { "\($0)" } ?? "" }

    // MARK: Menus

    private var sortMenu: some View {
        Menu {
            // A picker: the system's checkmark column (names line up, as the track menus).
            Picker("Sort By", selection: Binding(get: { filter.sort }, set: { filter.setSort($0) })) {
                ForEach(LibrarySort.allCases.filter { $0 != .played || !offline }) { s in
                    Text(s.label).tag(s)
                }
            }
            .pickerStyle(.inline)
            Divider()
            Button { filter.descending.toggle() } label: {
                Label(filter.descending ? "Ascending" : "Descending", systemImage: filter.descending ? "arrow.up" : "arrow.down")
            }
        } label: {
            FilterChipLabel(text: filter.sort.label, icon: filter.descending ? "arrow.down" : "arrow.up", menu: true)
        }
        .menuStyle(.button)
        .chipButton()
        .accessibilityLabel("Sort by \(filter.sort.label)")
    }

    /// The Library filter: with more than one library (or one chosen).
    static func offersLibraries(_ facets: LibraryFacets?, _ filter: LibraryFilter) -> Bool {
        (facets?.libraryChoices(filter.library).count ?? 0) > 1 || filter.library != nil
    }

    private var libraryMenu: some View {
        Menu {
            // A picker: the system's checkmark column (names line up, as the track menus).
            Picker("Library", selection: Binding(get: { filter.library ?? "" }, set: { id in
                filter.library = id.isEmpty ? nil : id
                filter.libraryName = facets?.libraries.first { $0.id == id }?.name
            })) {
                Text("All Libraries").tag("")
                ForEach(facets?.libraryChoices(filter.library) ?? []) { l in
                    Text(l.name + " · \(l.count)").tag(l.id)
                }
            }
            .pickerStyle(.inline)
        } label: {
            FilterChipLabel(text: filter.library == nil ? "Library" : (filter.libraryName ?? "Library"), active: filter.library != nil, menu: true)
        }
        .menuStyle(.button)
        .chipButton()
    }

    private var genreMenu: some View {
        let list = merged(facets?.genres ?? [], selected: filter.genres)
        return Menu {
            if !filter.genres.isEmpty {
                Button("Any Genre", systemImage: "xmark") { filter.genres = [] }
            }
            ForEach(list) { g in
                Toggle(isOn: Binding(get: { filter.genres.contains(g.name) },
                                     set: { if $0 { filter.genres.insert(g.name) } else { filter.genres.remove(g.name) } })) {
                    Text(g.name)
                    Text(count(g.count))
                }
            }
        } label: {
            FilterChipLabel(text: filter.genres.isEmpty ? "Genre" : filter.genres.count == 1 ? filter.genres.first! : "\(filter.genres.count) Genres",
                            active: !filter.genres.isEmpty, menu: true)
        }
        .keepsMenuOpen()
        .menuStyle(.button)
        .chipButton()
    }

    private var decadeMenu: some View {
        Menu {
            if filter.decade != nil {
                Button("Any Year", systemImage: "xmark") { filter.decade = nil }
            }
            // Toggle rows: the system's checkmark column, counts as the second line (as Genre's).
            ForEach(facets?.decades ?? []) { d in
                Toggle(isOn: Binding(get: { filter.decade == d.decade }, set: { filter.decade = $0 ? d.decade : nil })) {
                    Text(d.label)
                    Text(count(d.count))
                }
            }
        } label: {
            FilterChipLabel(text: filter.decade.map { "\($0)s" } ?? "Year", active: filter.decade != nil, menu: true)
        }
        .menuStyle(.button)
        .chipButton()
    }

    private var ratingMenu: some View {
        let list = merged(facets?.ratings ?? [], selected: filter.ratings)
        return Menu {
            if !filter.ratings.isEmpty {
                Button("Any Rating", systemImage: "xmark") { filter.ratings = [] }
            }
            ForEach(list) { r in
                Toggle(isOn: Binding(get: { filter.ratings.contains(r.name) },
                                     set: { if $0 { filter.ratings.insert(r.name) } else { filter.ratings.remove(r.name) } })) {
                    Text(Self.ratingLabel(r.name))
                    Text(count(r.count))
                }
            }
        } label: {
            FilterChipLabel(text: filter.ratings.isEmpty ? "Rating" : filter.ratings.count == 1 ? Self.ratingLabel(filter.ratings.first!) : "\(filter.ratings.count) Ratings",
                            active: !filter.ratings.isEmpty, menu: true)
        }
        .keepsMenuOpen()
        .menuStyle(.button)
        .chipButton()
    }

    static func ratingLabel(_ r: String) -> String { r == "none" ? "Not Rated" : r }

    private var watchedMenu: some View {
        Menu {
            Picker("Watched", selection: $filter.watched) {
                ForEach(WatchedFilter.allCases) { w in
                    if w == .all {
                        Text(w.label).tag(w)
                    } else {
                        Text(w.label + (facets?.watched?[w].map { " · \($0)" } ?? "")).tag(w)
                    }
                }
            }
            .pickerStyle(.inline)
        } label: {
            FilterChipLabel(text: filter.watched == .all ? "Watched" : filter.watched.label, active: filter.watched != .all, menu: true)
        }
        .menuStyle(.button)
        .chipButton()
    }

    private var resolutionMenu: some View {
        let available = Dictionary((facets?.resolutions ?? []).map { ($0.name, $0.count) }, uniquingKeysWith: +)
        let shown = ResolutionFilter.allCases.filter { available[$0.rawValue] != nil || filter.resolutions.contains($0) }
        return Menu {
            if !filter.resolutions.isEmpty {
                Button("Any Resolution", systemImage: "xmark") { filter.resolutions = [] }
            }
            ForEach(shown) { r in
                Toggle(isOn: Binding(get: { filter.resolutions.contains(r) },
                                     set: { if $0 { filter.resolutions.insert(r) } else { filter.resolutions.remove(r) } })) {
                    Text(r.label)
                    Text(count(available[r.rawValue] ?? 0))
                }
            }
        } label: {
            FilterChipLabel(text: filter.resolutions.isEmpty ? "Resolution" : filter.resolutions.sorted().map(\.label).joined(separator: ", "),
                            active: !filter.resolutions.isEmpty, menu: true)
        }
        .keepsMenuOpen()
        .menuStyle(.button)
        .chipButton()
    }

    /// The facet's values, plus selected ones that have no titles under the other filters.
    private func merged(_ values: [LibraryFacets.Value], selected: Set<String>) -> [LibraryFacets.Value] {
        values + selected.filter { s in !values.contains { $0.name == s } }.sorted().map { LibraryFacets.Value(name: $0, count: 0) }
    }
}

/// The filters that are on, each removable, and Clear All.
struct ActiveFilterChips: View {
    @Environment(\.metrics) private var metrics
    @Binding var filter: LibraryFilter

    private struct Token: Identifiable {
        let id: String
        let label: String
        let remove: (inout LibraryFilter) -> Void
    }

    private var tokens: [Token] {
        var t: [Token] = []
        // The page's own (a category tile's page) isn't a tag: it's the page.
        if let c = filter.collection, !filter.isLocked(.collection) { t.append(Token(id: "c", label: c.name ?? "Collection") { $0.collection = nil }) }
        if let s = filter.studio, !filter.isLocked(.studio) { t.append(Token(id: "s", label: s) { $0.studio = nil }) }
        if filter.library != nil { t.append(Token(id: "l", label: filter.libraryName ?? "Library") { $0.library = nil; $0.libraryName = nil }) }
        if !filter.isLocked(.genre) {
            for g in filter.genres.sorted() { t.append(Token(id: "g" + g, label: g) { $0.genres.remove(g) }) }
        }
        if let d = filter.decade, !filter.isLocked(.decade) { t.append(Token(id: "d", label: "\(d)s") { $0.decade = nil }) }
        for r in filter.ratings.sorted() { t.append(Token(id: "r" + r, label: LibraryFilterBar.ratingLabel(r)) { $0.ratings.remove(r) }) }
        if filter.watched != .all { t.append(Token(id: "w", label: filter.watched.label) { $0.watched = .all }) }
        for r in filter.resolutions.sorted() { t.append(Token(id: "x" + r.rawValue, label: r.label) { $0.resolutions.remove(r) }) }
        if filter.hdr { t.append(Token(id: "h", label: "HDR") { $0.hdr = false }) }
        if filter.downloaded { t.append(Token(id: "dl", label: "Downloaded") { $0.downloaded = false }) }
        if filter.recentlyAdded { t.append(Token(id: "ra", label: "Added in \(LibraryFilter.recentDays) days") { $0.recentlyAdded = false }) }
        return t
    }

    var body: some View {
        if filter.isFiltered {
            ScrollView(.horizontal) {
                HStack(spacing: tokenSpacing) {
                    ForEach(tokens) { t in
                        Button { t.remove(&filter) } label: { TokenLabel(text: t.label) }
                        .chipButton()
                        .accessibilityLabel("Remove \(t.label)")
                    }
                    Button { filter = filter.cleared } label: { ClearAllLabel() }
                        .chipButton()
                }
                .padding(.horizontal, metrics.hPadding)
                #if os(tvOS)
                .padding(.vertical, 14)
                #endif
            }
            .scrollIndicators(.hidden)
            #if os(tvOS)
            .scrollClipDisabled()
            .focusSection()
            #endif
        }
    }
}

extension ActiveFilterChips {
    /// The filter chips' spacing on tvOS (LibraryFilterBar).
    var tokenSpacing: CGFloat {
        #if os(tvOS)
        return 20
        #else
        return 6
        #endif
    }
}

private struct ClearAllLabel: View {
    @Environment(\.chipFocused) private var focused
    var body: some View {
        Text("Clear All")
            #if os(tvOS)
            .font(.callout.weight(.semibold))
            .padding(.horizontal, 22).padding(.vertical, 12)
            #else
            .font(.caption.weight(.semibold))
            .padding(.horizontal, 6).padding(.vertical, 5)
            #endif
            .foregroundStyle(focused ? Color.black : Theme.secondaryText)
            .background(Capsule().fill(focused ? Color.white : .clear))
            .contentShape(Capsule())
    }
}

/// Genres, Collections, Studios (Networks), Decades: each opens its category page.
struct LibraryCategoryStrip: View {
    @Environment(\.metrics) private var metrics
    let library: MediaItem
    let facets: LibraryFacets?
    /// Drawn first in the row (iPhone landscape: the library split, to save a row).
    var leading: AnyView?

    private var categories: [LibraryCategory] {
        LibraryCategory.allCases.filter { c in
            switch c {
            case .collections: !library.isShowLibrary && !(facets?.collections.isEmpty ?? true)
            case .studios: !(facets?.studios.isEmpty ?? true)
            default: true
            }
        }
    }

    var body: some View {
        ScrollView(.horizontal) {
            HStack(spacing: spacing) {
                if let leading { leading }
                ForEach(categories) { c in
                    NavigationLink(value: Route.category(library, c)) {
                        CategoryLabel(title: c.title(shows: library.isShowLibrary), icon: c.icon(shows: library.isShowLibrary))
                    }
                    .chipButton()
                }
            }
            .padding(.horizontal, metrics.hPadding)
            #if os(tvOS)
            .padding(.vertical, 14)
            #endif
        }
        .scrollIndicators(.hidden)
        #if os(tvOS)
        .scrollClipDisabled()
        .focusSection()
        #endif
    }

    private var spacing: CGFloat {
        #if os(tvOS)
        return 24
        #else
        return 10
        #endif
    }
}

/// A category page: genres, collections, studios/networks or decades of a library, each opening
/// the library's grid with that filter.
struct LibraryCategoryView: View {
    @Environment(\.metrics) private var metrics
    let source: Source
    let library: MediaItem
    let category: LibraryCategory
    /// Apple TV sidebar: the page sits beside the sidebar and a tile filters the library's grid
    /// (instead of opening a page of its own).
    var onPick: ((LibraryFilter, String) -> Void)? = nil

    @State private var facets: LibraryFacets?
    @State private var query = ""
    #if os(tvOS)
    @State private var width: CGFloat = 0
    @FocusState private var focusedTile: String?
    @Environment(\.tvFocusOnLoad) private var focusOnLoad
    @Environment(\.tvContentLeading) private var contentLeading
    #endif
    @State private var error: String?
    private var client: (any MediaServerClient)? { MediaServers.client(for: source) }
    private var offline: Bool { ConnectionRouter.shared.route == .unreachable }
    private var shows: Bool { library.isShowLibrary }

    private struct Entry: Identifiable {
        let id: String
        let title: String
        let count: Int
        let art: ImageRef?
        let poster: ImageRef?
        let filter: LibraryFilter
    }

    private var entries: [Entry] {
        #if os(tvOS)
        return rawEntries
        #else
        // A tile opens a page of its own: what makes the page is fixed there.
        return rawEntries.map { Entry(id: $0.id, title: $0.title, count: $0.count, art: $0.art, poster: $0.poster, filter: $0.filter.lockingCategory) }
        #endif
    }

    private var rawEntries: [Entry] {
        guard let f = facets else { return [] }
        let base = LibraryFilter(sort: .title)
        switch category {
        case .genres:
            return f.genres.map { g in
                var x = base; x.genres = [g.name]
                return Entry(id: g.name, title: g.name, count: g.count, art: g.backdrop, poster: nil, filter: x)
            }
        case .studios:
            let q = query.trimmingCharacters(in: .whitespaces)
            // With no search and a long list: the ones with more than one title (hundreds have one).
            let list = q.isEmpty ? (f.studios.count > 24 ? f.studios.filter { $0.count > 1 } : f.studios)
                : f.studios.filter { $0.name.localizedCaseInsensitiveContains(q) }
            return list.map { s in
                var x = base; x.studio = s.name
                return Entry(id: s.name, title: s.name, count: s.count, art: s.backdrop, poster: nil, filter: x)
            }
        case .decades:
            return f.decades.map { d in
                var x = base; x.decade = d.decade; x.sort = .year; x.descending = false
                return Entry(id: d.label, title: d.label, count: d.count, art: d.backdrop, poster: nil, filter: x)
            }
        case .collections:
            // A long list: the collections with more than one title here.
            return (f.collections.count > 24 ? f.collections.filter { $0.count > 1 } : f.collections).map { c in
                var x = base; x.collection = MediaCollectionRef(id: c.id, name: c.name); x.sort = .year; x.descending = false
                return Entry(id: "\(c.id)", title: c.name, count: c.count, art: c.backdrop, poster: c.poster, filter: x)
            }
        }
    }

    var body: some View {
        ScrollView {
          VStack {
            #if os(tvOS)
            if onPick != nil { tvHeader }
            #endif
            if category == .collections {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: posterMin), spacing: posterSpacing.h, alignment: .top)],
                          spacing: posterSpacing.v) {
                    ForEach(entries) { e in
                        link(e) { posterTile(e) }
                            .cardButtonStyle()
                    }
                }
                .padding(pagePadding)
            } else {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: tileMin), spacing: tileSpacing, alignment: .top)], spacing: tileSpacing) {
                    ForEach(entries) { e in
                        link(e) {
                            LibraryTile(title: e.title, icon: category.icon(shows: shows), image: e.art.flatMap { url($0, 800) },
                                        detail: titles(e.count))
                        }
                        .cardButtonStyle()
                    }
                }
                .padding(pagePadding)
            }
            if facets == nil {
                if let error { Text(error).foregroundStyle(.red).padding() } else { ProgressView().padding() }
            } else if entries.isEmpty {
                EmptyStateView(icon: category.icon(shows: shows), title: "Nothing Here",
                               message: query.isEmpty ? "No \(category.title(shows: shows).lowercased()) in this library." : "Nothing matched “\(query)”.")
            }
            if category == .studios, query.isEmpty, let f = facets, f.studios.count > entries.count {
                // tvOS has no search on this page.
                Text("\(f.studios.count - entries.count) more with one title each" + (Self.canSearch ? ": search to find them." : "."))
                    .font(.footnote).foregroundStyle(Theme.secondaryText)
                    .padding(.horizontal, metrics.hPadding).padding(.bottom)
            }
          }
          #if os(tvOS)
          .padding(.leading, contentLeading)
          #endif
        }
        #if os(tvOS)
        .scrollClipDisabled() // beside the sidebar: the first column's focus growth
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
        .onChange(of: facets == nil) { _, missing in
            // Back from the grid a tile filtered: that tile again.
            guard !missing, let want = focusOnLoad else { return }
            let list = entries
            let id = list.contains(where: { $0.id == want }) ? want : list.first?.id
            Task { try? await Task.sleep(for: .milliseconds(80)); focusedTile = id }
        }
        #endif
        .background(Theme.backgroundGradient.ignoresSafeArea())
        #if !os(tvOS)
        // iPad beside the sidebar: the page's name and count, pinned.
        .modifier(PadPageChrome(on: onPick != nil) {
            PadLibraryHeader(title: category.title(shows: shows)) {
                if category == .studios {
                    // The name search, in the header (not the navigation bar).
                    PadSearchField(text: $query, prompt: shows ? "Networks" : "Studios")
                }
                if facets != nil {
                    Text(entries.count == 1 ? "1 \(category.singular(shows: shows))" : "\(entries.count) \(category.title(shows: shows).lowercased())")
                        .font(.subheadline).foregroundStyle(Theme.secondaryText)
                }
            }
        })
        #endif
        .navigationTitle(onPick != nil ? "" : category.title(shows: shows))
        #if !os(tvOS)
        .modifier(StudioSearch(enabled: category == .studios && onPick == nil, query: $query, prompt: shows ? "Networks" : "Studios"))
        // The phone: this page is the Library's page (for turning to landscape).
        .onAppear {
            // Only on top (pushed with a tile's page above it, it appears too).
            if onPick == nil, AppNavigation.shared.libraryPath.count == 1 { AppNavigation.shared.libraryPage = .category(category) }
        }
        .onDisappear { if onPick == nil, AppNavigation.shared.libraryPage == .category(category) { AppNavigation.shared.libraryPage = nil } }
        #endif
        .task(id: offline) {
            await ConnectionRouter.shared.ready()
            error = nil
            if offline {
                facets = LibraryFacets.local(OfflineLibrary.titles(shows: shows), filter: LibraryFilter())
                return
            }
            do {
                facets = try await client?.facets(libraryId: library.id, filter: LibraryFilter(), only: nil)
            } catch {
                if !error.isCancellation { self.error = error.localizedDescription }
            }
        }
    }

    /// A tile: filters the grid beside the sidebar (tvOS), else opens the filtered page.
    @ViewBuilder private func link<L: View>(_ e: Entry, @ViewBuilder label: () -> L) -> some View {
        if let onPick {
            Button { onPick(e.filter, e.id) } label: { label() }
                #if os(tvOS)
                .tvLeadingEdge(leadingIds.contains(e.id))
                .focused($focusedTile, equals: e.id)
                #endif
        } else {
            NavigationLink(value: Route.filtered(library, e.filter, e.title)) { label() }
        }
    }

    #if os(tvOS)
    /// The tiles in the first column (an adaptive grid's column count, from the width).
    private var leadingIds: Set<String> {
        let (min, gap) = category == .collections ? (posterMin, posterSpacing.h) : (tileMin, tileSpacing)
        let n = max(1, Int((width - contentLeading + gap) / (min + gap)))
        let list = entries
        return Set(stride(from: 0, to: list.count, by: n).map { list[$0].id })
    }

    /// The grid's header bar, with the page's name and count.
    private var tvHeader: some View {
        HStack(alignment: .firstTextBaseline, spacing: 20) {
            Text(category.title(shows: shows)).font(TVLibraryMetrics.titleFont)
            if facets != nil {
                Text(entries.count == 1 ? "1 \(category.singular(shows: shows))" : "\(entries.count) \(category.title(shows: shows).lowercased())")
                    .font(.callout).foregroundStyle(.secondary)
            }
        }
        .lineLimit(1)
        .frame(maxWidth: .infinity, minHeight: TVLibraryMetrics.barHeight, alignment: .leading)
        .padding(.top, 12)
        .padding(.bottom, TVLibraryMetrics.headerBottom)
    }
    #endif

    private func titles(_ n: Int) -> String {
        shows ? (n == 1 ? "1 show" : "\(n) shows") : (n == 1 ? "1 movie" : "\(n) movies")
    }

    private func url(_ ref: ImageRef, _ w: Int) -> URL? { offline ? nil : client?.imageURL(ref, width: w) }

    #if os(tvOS)
    private static let canSearch = false
    /// Three tiles across (long studio names fit), Home's 40-pt gaps; beside the sidebar too.
    private var tileMin: CGFloat { onPick != nil ? 380 : 480 }
    private let tileSpacing: CGFloat = 40
    /// The Library grid's poster size and gaps (room under each row for the focused poster).
    private let posterMin: CGFloat = Theme.gridMin
    private let posterSpacing: (h: CGFloat, v: CGFloat) = (40, 60)
    /// The content edge comes from the safe area; just space above and below (beside the
    /// sidebar the header has it).
    private var pagePadding: EdgeInsets { EdgeInsets(top: onPick != nil ? 0 : 20, leading: 0, bottom: 60, trailing: 0) }
    #else
    private static let canSearch = true
    private let tileMin: CGFloat = 165
    private let tileSpacing: CGFloat = 14
    private let posterMin: CGFloat = 110
    private let posterSpacing: (h: CGFloat, v: CGFloat) = (14, 20)
    private var pagePadding: EdgeInsets { EdgeInsets(top: metrics.hPadding, leading: metrics.hPadding, bottom: metrics.hPadding, trailing: metrics.hPadding) }
    #endif

    @ViewBuilder private func posterTile(_ e: Entry) -> some View {
        #if os(tvOS)
        // A system lockup: the name moves down out of the focused poster's way.
        posterArt(e).lockupArtwork()
        posterName(e)
        #else
        VStack(alignment: .leading, spacing: 6) {
            posterArt(e).shadow(color: .black.opacity(0.4), radius: 6, y: 3)
            posterName(e)
        }
        #endif
    }

    private func posterName(_ e: Entry) -> some View {
        Text(e.title.replacingOccurrences(of: " Collection", with: ""))
            .font(.caption.weight(.medium)).lineLimit(2).foregroundStyle(.white)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func posterArt(_ e: Entry) -> some View {
        Color.clear
            .aspectRatio(2 / 3, contentMode: .fit)
            .overlay { RemoteImage(url: (e.poster ?? e.art).flatMap { url($0, 500) }) }
            .overlay {
                if e.poster == nil && e.art == nil {
                    Image(systemName: "square.stack.3d.up").font(.title).foregroundStyle(Theme.secondaryText)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay(alignment: .topTrailing) {
                Text("\(e.count)")
                    .font(.caption2.weight(.bold))
                    .padding(.horizontal, 7).padding(.vertical, 3)
                    .background(.black.opacity(0.6), in: Capsule())
                    .foregroundStyle(.white)
                    .padding(Self.badgeInset)
            }
    }

    #if os(tvOS)
    private static let badgeInset: CGFloat = 10
    #else
    private static let badgeInset: CGFloat = 6
    #endif
}

#if !os(tvOS)
private struct StudioSearch: ViewModifier {
    let enabled: Bool
    @Binding var query: String
    let prompt: String
    func body(content: Content) -> some View {
        if enabled { content.searchable(text: $query, prompt: prompt) } else { content }
    }
}
#endif
