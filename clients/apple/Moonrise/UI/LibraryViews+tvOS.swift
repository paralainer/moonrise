#if os(tvOS)
import SwiftUI

extension LibraryBrowser {
    var panelSeamStart: TVFilterPanel.Facet? {
        #if DEBUG
        if let p = LibrarySeam.current?.panel, p.hasPrefix("filter:") { return TVFilterPanel.Facet(rawValue: String(p.dropFirst(7))) }
        #endif
        return nil
    }

    /// Apple TV: one slim bar (count; Jump To, Sort, Filter) and, while filters are on, one line
    /// naming them with Clear.
    var tvHeader: some View {
        VStack(alignment: .leading, spacing: TVLibraryMetrics.rowGap) {
            HStack(spacing: TVLibraryMetrics.buttonGap) {
                HStack(alignment: .firstTextBaseline, spacing: 20) {
                    Text(isRoot ? TVLibraryMetrics.countTitle(total, shows: isShows) : (title ?? library.displayTitle))
                        .font(TVLibraryMetrics.titleFont)
                    if !isRoot {
                        Text(TVLibraryMetrics.countTitle(total, shows: isShows).lowercased())
                            .font(.callout).foregroundStyle(.secondary)
                    }
                    if loading { ProgressView().scaleEffect(0.7) }
                }
                .lineLimit(1)
                Spacer(minLength: 20)
                if filter.sort == .title, index.count > 1 {
                    Menu {
                        ForEach(index.map(\.letter), id: \.self) { l in Button(l) { Task { await jump(to: l) } } }
                    } label: {
                        TVHeaderLabel(text: "Jump To", menu: true)
                    }
                    .tvGlassButton(height: TVLibraryMetrics.buttonHeight)
                    .tvLeadingEdge()
                }
                TVSortMenu(filter: $filter, offline: offline)
                    .tvGlassButton(height: TVLibraryMetrics.buttonHeight)
                    .tvLeadingEdge(!(filter.sort == .title && index.count > 1))
                Button { showFilters = true } label: {
                    TVHeaderLabel(text: filter.activeCount > 0 ? "Filter · \(filter.activeCount)" : "Filter",
                                  icon: "line.3.horizontal.decrease", menu: false)
                }
                .tvGlassButton(height: TVLibraryMetrics.buttonHeight)
                .focused($filterButtonFocused)
                .onChange(of: restoreFilterFocus) { _, on in
                    guard on else { return }
                    restoreFilterFocus = false
                    Task { try? await Task.sleep(for: .milliseconds(50)); filterButtonFocused = true }
                }
            }
            .frame(height: TVLibraryMetrics.barHeight)
            .focusSection()
            if filter.isFiltered {
                HStack(spacing: TVLibraryMetrics.buttonGap) {
                    Text(TVLibraryMetrics.summary(filter))
                        .font(.callout.weight(.medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    Spacer(minLength: 20)
                    // Clear goes away with the filters: focus moves up to Filter.
                    Button { filter = filter.cleared; restoreFilterFocus = true } label: { TVHeaderLabel(text: "Clear", icon: "xmark", menu: false) }
                        .tvGlassButton(height: TVLibraryMetrics.buttonHeight)
                        .tvLeadingEdge()
                }
                .frame(height: TVLibraryMetrics.barHeight)
                .focusSection()
            }
        }
        // Room under the bar for the first row's focus growth.
        .padding(.bottom, TVLibraryMetrics.headerBottom)
    }
}

extension SearchView {
    @ViewBuilder func tvShelf(_ title: String, _ items: [MediaItem], landscape: Bool = false, clear: (() -> Void)? = nil) -> some View {
        if !items.isEmpty {
            TVShelf(title: title) {
                ForEach(items) { item in
                    NavigationLink(value: Route.searchItem(item)) {
                        if landscape {
                            TVLandscapeLockup(item: item, client: client)
                        } else {
                            TVPosterLockup(item: item, client: client)
                        }
                    }
                    .watchedMenu(item, client: client)
                }
                if clear != nil {
                    Button { confirmClearRecent = true } label: {
                        Text("Clear Recent")
                            .font(.callout.weight(.medium))
                            .multilineTextAlignment(.center)
                            .padding(.horizontal, 12)
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                            .background(Color.white.opacity(0.1))
                            .aspectRatio(2 / 3, contentMode: .fit)
                            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                            .hoverEffect(.highlight)
                            .containerRelativeFrame(.horizontal, count: 7, spacing: 40)
                    }
                    // Entering the row from the keyboard can land here, so a stray press asks first.
                    .confirmationDialog("Clear Recent?", isPresented: $confirmClearRecent) {
                        Button("Clear Recent", role: .destructive) { clear?() }
                        Button("Cancel", role: .cancel) {}
                    } message: {
                        Text("These titles are removed from Recent on all your devices.")
                    }
                }
            }
        }
    }
}
#endif
