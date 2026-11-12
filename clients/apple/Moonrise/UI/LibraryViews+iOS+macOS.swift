#if !os(tvOS)
import SwiftUI

extension LibraryBrowser {
    /// The filter popover's height (the Mac's rows are shorter).
    private static var filterPanelHeight: CGFloat {
        #if os(macOS)
        return 520
        #else
        return 650
        #endif
    }

    /// iPad beside the sidebar: under the pinned header, the filters that are on (removable) and
    /// the count line, on the grid's leading edge.
    @ViewBuilder var padSubheader: some View {
        if offline {
            OfflineBanner(retry: retry)
        }
        ActiveFilterChips(filter: $filter)
        countRow
    }

    /// iPad: the page's title, then Sort and Filter (its badge: how many filters are on).
    var padHeader: some View {
        PadLibraryHeader(title: title ?? library.displayTitle, onBack: onBack) {
            PadSortMenu(filter: $filter, offline: offline)
            filterButton
        }
    }

    /// Filter, with how many are on; it opens the panel of every filter.
    private var filterButton: some View {
        Button { showFilters = true } label: {
            PadHeaderButtonLabel(text: "Filter", icon: "line.3.horizontal.decrease", badge: filter.activeCount)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(filter.activeCount > 0 ? "Filter, \(filter.activeCount) on" : "Filter")
        .popover(isPresented: $showFilters, arrowEdge: .top) {
            PadFilterPanel(filter: $filter, facets: facets, isShows: isShows, offline: offline,
                           total: total, loading: loading, start: padPanelSeamStart)
                // iPad: a popover; the phone: a sheet (it scrolls in the short height).
                .frame(width: metrics.regular ? 380 : nil, height: metrics.regular ? Self.filterPanelHeight : nil)
                // Solid, so the posters behind don't show through the lists.
                .presentationBackground(Color(red: 0.11, green: 0.12, blue: 0.15))
                .environment(\.colorScheme, .dark)
        }
    }

    /// iPhone held upright (not the iPad's narrow windows, not the Mac).
    var phonePortrait: Bool {
        #if os(iOS)
        return UIDevice.current.userInterfaceIdiom == .phone && !padSidebar
        #else
        return false
        #endif
    }

    /// iPhone upright: the page's title (back on a tile's page), Sort and Filter at the end,
    /// as tall as each other. When the sort's name won't fit beside the title, Sort says "Sort",
    /// then shows only its arrow; a title still too long is set smaller.
    var phoneTitleRow: some View {
        let name = isRoot ? "Library" : (title ?? library.displayTitle)
        return ViewThatFits(in: .horizontal) {
            phoneTitleRow(name, sort: .full, fixedTitle: true)
            phoneTitleRow(name, sort: .short, fixedTitle: true)
            phoneTitleRow(name, sort: .icon, fixedTitle: true)
            phoneTitleRow(name, sort: .icon, fixedTitle: false)
        }
        .padding(.horizontal, metrics.hPadding)
        .padding(.top, 4)
        .padding(.bottom, 6)
    }

    private enum SortLabel { case full, short, icon }

    private func phoneTitleRow(_ name: String, sort: SortLabel, fixedTitle: Bool) -> some View {
        let h = PadLibraryMetrics.buttonHeight(metrics)
        return HStack(spacing: 8) {
            if !isRoot {
                Button { dismiss() } label: {
                    Image(systemName: "chevron.left")
                        .font(.title3.weight(.semibold))
                        .frame(width: h, height: h)
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .glassEffect(.regular.interactive(), in: .circle)
                .accessibilityLabel("Back")
            }
            // A long tile name: the landscape header's size (it rarely needs more).
            Text(name)
                .font(fixedTitle ? .largeTitle.weight(.bold) : .title2.weight(.bold))
                .lineLimit(1)
                .minimumScaleFactor(fixedTitle ? 1 : 0.75)
                .fixedSize(horizontal: fixedTitle, vertical: false)
            Spacer(minLength: 8)
            PadSortMenu(filter: $filter, offline: offline, compact: sort == .short, iconOnly: sort == .icon)
            filterButton
        }
        .frame(height: 44)
    }

    /// iPhone upright, under the pinned title: Movies | Shows across the margins, the categories,
    /// the filters that are on (removable, Clear All), then the count with Jump To at its end.
    @ViewBuilder var phonePortraitHeader: some View {
        if isRoot, libraries.count > 1 {
            picker.padding(.horizontal, metrics.hPadding)
        }
        if offline {
            OfflineBanner(retry: retry)
        }
        if isRoot {
            LibraryCategoryStrip(library: library, facets: facets, leading: nil)
        }
        ActiveFilterChips(filter: $filter)
        HStack(spacing: 6) {
            Text(countLine)
            if loading && !items.isEmpty { ProgressView().controlSize(.small) }
            Spacer(minLength: 8)
            if filter.sort == .title, index.count > 1 {
                Menu {
                    ForEach(index.map(\.letter), id: \.self) { l in Button(l) { Task { await jump(to: l) } } }
                } label: {
                    HStack(spacing: 3) {
                        Text("Jump To")
                        Image(systemName: "chevron.down").font(.caption2.weight(.semibold))
                    }
                    .foregroundStyle(.white.opacity(0.85))
                }
                .accessibilityLabel("Jump To")
            }
        }
        .lineLimit(1)
        .font(.footnote)
        .foregroundStyle(Theme.secondaryText)
        .padding(.horizontal, metrics.hPadding)
    }

    private var padPanelSeamStart: PadFilterPanel.Facet? {
        #if DEBUG
        if let p = LibrarySeam.current?.panel, p.hasPrefix("filter:") { return PadFilterPanel.Facet(rawValue: String(p.dropFirst(7))) }
        #endif
        return nil
    }

    @ViewBuilder var phoneHeader: some View {

        if !sharedRow, isRoot, libraries.count > 1 {
            picker
            .frame(maxWidth: 420)
            .padding(.horizontal, metrics.hPadding)
        }
        if offline {
            OfflineBanner(retry: retry)
        }
        if isRoot {
            LibraryCategoryStrip(library: library, facets: facets,
                                 leading: sharedRow && libraries.count > 1 ? AnyView(picker.frame(width: metrics.regular ? 280 : 240)) : nil)
        }
        LibraryFilterBar(filter: $filter, facets: facets, isShows: isShows, offline: offline,
                         letters: filter.sort == .title ? index.map(\.letter) : [], onJump: { l in Task { await jump(to: l) } })
        ActiveFilterChips(filter: $filter)
        countRow

    }
}

/// A–Z down the grid's edge (title sort): touch or drag to a letter.
struct AlphabetIndex: View {
    /// The letters present, in list order.
    let letters: [String]
    let onSelect: (String) -> Void
    @State private var current: String?

    private static let all = ["#"] + (65...90).map { String(UnicodeScalar(UInt8($0))) }

    var body: some View {
        GeometryReader { g in
            VStack(spacing: 0) {
                ForEach(Self.all, id: \.self) { l in
                    Text(l)
                        // A line is ~1.2× the font size: 0.75 of a row keeps all 27 inside a short strip.
                        .font(.system(size: min(11, g.size.height / CGFloat(Self.all.count) * 0.75), weight: .bold, design: .rounded))
                        .foregroundStyle(letters.contains(l) ? (current == l ? Color.black : .white) : .white.opacity(0.25))
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .background { if current == l { Circle().fill(.white).frame(width: 15, height: 15) } }
                }
            }
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0).onChanged { v in
                let i = min(Self.all.count - 1, max(0, Int(v.location.y / max(1, g.size.height) * CGFloat(Self.all.count))))
                // The nearest letter that has titles, at or after the one touched (else before).
                let after = Self.all[i...].first(where: letters.contains)
                let before = Self.all[...i].last(where: letters.contains)
                if let l = after ?? before, l != current {
                    current = l
                    onSelect(l)
                }
            }.onEnded { _ in
                Task { try? await Task.sleep(for: .milliseconds(450)); current = nil }
            })
        }
        .frame(width: 20)
        .frame(maxHeight: 420)
        .padding(.vertical, 6)
        .glassEffect(.regular, in: .capsule)
        .overlay(alignment: .leading) {
            if let current {
                Text(current)
                    .font(.system(size: 34, weight: .bold, design: .rounded))
                    .foregroundStyle(.white)
                    .frame(width: 64, height: 64)
                    .glassEffect(.regular, in: .rect(cornerRadius: 16))
                    .offset(x: -80)
                    .transition(.opacity)
            }
        }
        .sensoryFeedback(.selection, trigger: current)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Jump to letter")
    }
}

/// iPad beside the Library's sidebar: the page's header pinned over its scroll view (the content
/// scrolls under it, softened), and no title in the navigation bar (the sidebar says Library).
struct PadPageChrome<Header: View>: ViewModifier {
    let on: Bool
    @ViewBuilder var header: () -> Header

    func body(content: Content) -> some View {
        if on {
            content
                .safeAreaBar(edge: .top, spacing: 0) { header() }
                .toolbar(removing: .title)
        } else {
            content
        }
    }
}
#endif
