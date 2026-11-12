import SwiftUI

struct HomeSection: Identifiable, Equatable {
    enum Style { case landscape, poster }
    let id: String
    let title: String
    let style: Style
    let items: [MediaItem]

    /// Continue Watching (once Recents and Next Up).
    static let continueID = "continue"
}

/// Home when the server has answered with nothing to show.
struct EmptyHomeView: View {
    var body: some View {
        EmptyStateView(icon: "play.rectangle.on.rectangle", title: "Nothing Here",
                       message: "This server has nothing to show yet.")
    }
}

struct HomeView: View {
    @Environment(\.metrics) private var metrics
    @Environment(\.scenePhase) private var scenePhase
    static let heroZoomID = "hero"
    @Environment(\.zoomNamespace) private var zoom
    /// Leading item per row, so rows stay on the same items when their contents change.
    @State private var rowPositions: [String: String] = [:]
    private var nav: AppNavigation { AppNavigation.shared }
    private var store: SourceStore { SourceStore.shared }
    private var home: HomeStore { HomeStore.shared }
    private var router: ConnectionRouter { ConnectionRouter.shared }

    private var client: (any MediaServerClient)? { store.client }

    /// Why the server can't be reached right now, when we're sure of it.
    private var problem: ConnectionRouter.Problem? {
        guard home.failure != nil || home.sections.isEmpty else { return nil }
        return router.problem
    }

    var body: some View {
        let problem = self.problem
        // The banner's artwork can't load from a server we can't reach.
        let hero = problem == nil ? home.hero : []
        content(problem: problem, hero: hero)
            .zoomSource(Self.heroZoomID, in: zoom, cornerRadius: 0)
        .background(Theme.background.ignoresSafeArea())
        #if os(tvOS)
        .navigationTitle("") // the tab bar already says Home
        #else
        .navigationTitle(hero.isEmpty ? "Home" : "")
        #endif
        #if os(iOS)
        .toolbarBackground(.hidden, for: .navigationBar)
        // Inline like the title page, so the banner sits at exactly the same y as the page header.
        .navigationBarTitleDisplayMode(hero.isEmpty ? .automatic : .inline)
        .ignoresSafeArea(edges: hero.isEmpty ? [] : .top)
        #elseif os(tvOS)
        .ignoresSafeArea(edges: hero.isEmpty ? [] : .top) // banner runs under the floating tab bar
        #elseif os(macOS)
        // As the iPad's: the banner runs under the toolbar's tabs (Liquid Glass over the picture).
        .ignoresSafeArea(edges: hero.isEmpty ? [] : .top)
        #endif
        .refreshable {
            await router.refresh()
            await home.reloadAndWait()
        }
        .task { home.reload() }
        .onChange(of: nav.homeCovered) { _, covered in
            if covered { home.isVisible = false } else { home.homeAppeared(apply: applyUpdate) }
        }
        // Whenever rows change (held update or a fresh load), keep the opened title in view.
        .onChange(of: rowSignature) { old, new in followOpened(old, new) }
        // The server just became reachable (Tailscale connected, back on Wi-Fi) -> load it.
        .onChange(of: router.route) { old, new in
            if router.isReachable, old != new { home.reload() }
        }
        // Back in the app (iPhone, iPad, Mac): load again, so what another device played shows up.
        .onChange(of: scenePhase) { _, phase in
            if phase == .active { home.refreshIfStale() }
        }
        .onReceive(NotificationCenter.default.publisher(for: .playbackDidEnd)) { _ in
            Task { try? await Task.sleep(for: .seconds(1)); home.reload() }
        }
        // Marked from a card's menu: Continue Watching follows.
        .onReceive(NotificationCenter.default.publisher(for: .watchStateDidChange)) { _ in home.reload() }
        #if os(iOS) // tvOS: Settings is a tab (a toolbar button floats over the content there); Mac: the app menu (⌘,)
        .settingsButton()
        #endif
    }

    // MARK: Content

    private func content(problem: ConnectionRouter.Problem?, hero: [MediaItem]) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 28) {
                if !hero.isEmpty {
                    HeroCarousel(items: hero, client: client, active: !nav.homeCovered)
                        .ignoresSafeArea(edges: .horizontal) // edge to edge, like the TV app
                }
                if let problem {
                    // The server doesn't answer: the downloads first, they play.
                    if DownloadManager.shared.records.contains(where: \.isPlayable) {
                        OfflineBanner { home.reload() }
                            .padding(.top, hero.isEmpty ? 8 : 0)
                        OfflineDownloadShelves()
                    }
                    ConnectionProblemView(problem: problem, hasSavedContent: !home.sections.isEmpty) { home.reload() }
                        .padding(.top, hero.isEmpty ? 8 : 0)
                }
                if SourceStore.shared.isDemo {
                    DemoBanner()
                        .padding(.horizontal, max(metrics.hPadding, 16))
                        .padding(.top, hero.isEmpty ? 8 : 0)
                }
                ForEach(home.sections) { section in shelf(section) }
                // The server answered with an error (not a connectivity problem).
                if problem == nil, let msg = home.failure, !msg.isEmpty {
                    Label(msg, systemImage: "exclamationmark.triangle")
                        .font(.footnote).foregroundStyle(Theme.secondaryText)
                        .padding(.horizontal, metrics.hPadding)
                }
                if home.loading && home.sections.isEmpty && problem == nil {
                    ProgressView().frame(maxWidth: .infinity).padding(60)
                }
                if HomeStore.isEmpty(answered: home.answered, loading: home.loading,
                                     failed: home.failure != nil || problem != nil, rows: home.sections.count) {
                    EmptyHomeView().padding(.horizontal, metrics.hPadding)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.bottom, 40)
        }
    }

    private func shelf(_ section: HomeSection) -> some View {
        let position = Binding<String?>(get: { rowPositions[section.id] }, set: { rowPositions[section.id] = $0 })
        return Shelf(title: section.title, position: position) {
            ForEach(section.items) { item in card(section, item) }
        }
    }

    /// Apply results that arrived while a title page was open.
    private func applyUpdate(_ sections: [HomeSection], _ hero: [MediaItem]) {
        home.setContent(sections, hero)
    }

    private var rowSignature: [String: [String]] {
        Dictionary(home.sections.map { ($0.id, $0.items.map(\.id)) }, uniquingKeysWith: { a, _ in a })
    }

    /// If the title you opened moved within its row (e.g. it was just played and is now first in
    /// Continue Watching), scroll the row to it so you land on the same title rather than whatever now sits
    /// at the old offset.
    private func followOpened(_ old: [String: [String]], _ new: [String: [String]]) {
        // Zoom ids are "<row id>/<item id>"; row ids contain no "/".
        guard let z = nav.lastOpened, let slash = z.firstIndex(of: "/") else { return }
        let row = String(z[..<slash]), item = String(z[z.index(after: slash)...])
        guard let before = old[row]?.firstIndex(of: item), let after = new[row]?.firstIndex(of: item),
              before != after else { return }
        rowPositions[row] = item
    }

    @ViewBuilder private func card(_ section: HomeSection, _ item: MediaItem) -> some View {
        switch section.style {
        case .landscape:
            // Episodes open their show with the episode selected; long-press to play directly.
            let zoomID = "\(section.id)/\(item.id)"
            NavigationLink(value: Route.item(item, zoom: zoomID)) {
                LandscapeCard(item: item, client: client, zoomID: zoomID)
            }
            .cardButtonStyle()
            .contextMenu {
                Button {
                    // The title's page opens and plays it (in its header).
                    nav.pendingRoute = Route.item(item, zoom: zoomID, autoplay: Autoplay())
                } label: { Label(item.resumeSeconds != nil ? "Resume" : "Play", systemImage: "play.fill") }
                WatchedMenuItems(item: item, client: client)
                if section.id == HomeSection.continueID, let server = store.server {
                    Button(role: .destructive) {
                        // Hidden here at once; the server hides it on the other devices.
                        RecentsStore.remove(source: server.id, titleId: item.titleId)
                        home.reload()
                        Task {
                            try? await client?.removeFromContinue(item.id)
                            home.reload()
                        }
                    } label: { Label("Remove from Continue Watching", systemImage: "minus.circle") }
                }
            }
        case .poster:
            let zoomID = "\(section.id)/\(item.id)"
            NavigationLink(value: Route.item(item, zoom: zoomID)) {
                PosterCard(item: item, client: client, zoomID: zoomID)
            }
            .cardButtonStyle()
            .watchedMenu(item, client: client)
        }
    }
}

/// Large backdrop banner at the top of Home. Each page is laid out exactly like the top of the
/// title page it opens (same header, logo position and info line), so the transition keeps the
/// picture in place and only the page content appears below it.
struct HeroCarousel: View {
    let items: [MediaItem]
    let client: (any MediaServerClient)?
    /// Auto-advance only while Home is on screen, so you come back to the banner you left.
    var active = true
    /// Selected by item id, so a reload that reorders or replaces items doesn't jump the banner.
    @State var selection: String?
    @Environment(\.metrics) private var metrics
    #if os(iOS)
    /// Changes on rotation: the banner's height (`TitleHeader.height`) is read again.
    @Environment(\.verticalSizeClass) private var verticalSizeClass
    #endif
    static var metaHeight: CGFloat {
        #if os(tvOS)
        return 170
        #else
        return 70
        #endif
    }

    #if os(tvOS)
    @FocusState var heroFocused: Bool

    #else
    /// iPad: the banner's height under the picture (line, genres, Play), less what sits over it.
    private var padInfoHeight: CGFloat { 50 + 16 + PlayCapsule.height - TitleHeader.overlap(metrics) }

    var body: some View {
        VStack(spacing: 10) {
            // A paging scroll view rather than a page-style TabView: the TabView insets its pages
            // a few points from the top, which would misalign the banner with the title page.
            ScrollViewReader { proxy in
                ScrollView(.horizontal) {
                    LazyHStack(spacing: 0) {
                        ForEach(items) { item in
                            Group {
                                if metrics.regular {
                                    // iPad: the page with Play under its line (its own buttons, so
                                    // not one big link).
                                    HeroPage(client: client, item: item, buttons: true)
                                        .frame(maxHeight: .infinity, alignment: .top)
                                } else {
                                    // The zoom source is the whole Home screen (see HomeView), the same frame as
                                    // the title page, so nothing scales: the picture stays put and the page fades in.
                                    NavigationLink(value: Route.item(item, zoom: HomeView.heroZoomID)) {
                                        HeroPage(client: client, item: item)
                                            // Pin to the top: centred, the spare space under a short info line
                                            // would push the picture down and misalign it with the title page.
                                            .frame(maxHeight: .infinity, alignment: .top)
                                    }
                                    .plainButton(scale: 1)
                                }
                            }
                            .containerRelativeFrame(.horizontal)
                            .id(Optional(item.id))
                        }
                    }
                    .scrollTargetLayout()
                }
                .scrollIndicators(.hidden)
                .scrollTargetBehavior(.paging)
                .scrollPosition(id: $selection)
                #if os(macOS)
                // No focus ring around the half-paged banner (Play and More Info take the keys).
                .focusEffectDisabled()
                #endif
                .frame(height: TitleHeader.height(metrics) + 18 + (metrics.regular ? padInfoHeight : Self.metaHeight))
                // Rotated or resized: back onto the page it showed (the offset doesn't follow).
                .onChange(of: metrics.width) {
                    guard let s = selection else { return }
                    Task { try? await Task.sleep(for: .milliseconds(50)); proxy.scrollTo(Optional(s), anchor: .leading) }
                }
            }

            dots
        }
        .onAppear { if selection == nil || !items.contains(where: { $0.id == selection }) { selection = items.first?.id } }
        .task(id: active) {
            while active, !Task.isCancelled {
                try? await Task.sleep(for: .seconds(7))
                guard active, !Task.isCancelled, !items.isEmpty else { return }
                let i = items.firstIndex { $0.id == selection } ?? -1
                withAnimation { selection = items[(i + 1) % items.count].id }
            }
        }
    }
    #endif

    @ViewBuilder var dots: some View {
        if items.count > 1 {
            HStack(spacing: 7) {
                ForEach(items) { item in
                    Circle()
                        .fill(.white.opacity(item.id == selection ? 0.9 : 0.3))
                        .frame(width: 6, height: 6)
                }
            }
            .animation(.easeInOut, value: selection)
        }
    }
}

struct HeroPage: View {
    @Environment(\.metrics) private var metrics
    let client: (any MediaServerClient)?
    let item: MediaItem
    var focused = false
    /// iPad: Play and More Info under the line, the rest of the page opening the title.
    var buttons = false

    private var nav: AppNavigation { AppNavigation.shared }

    var body: some View {
        if buttons { padBody } else { phoneBody }
    }

    /// Laid out as the iPad title page: the logo, line and Play at the leading margin (over the
    /// bottom of the picture when the window is wider than tall), Info where its Download is.
    private var padBody: some View {
        VStack(alignment: .leading, spacing: 18) {
            Button(action: open) {
                TitleHeader(image: client?.backdropURL(item), logo: client?.logoURL(item), title: item.displayTitle)
                    .contentShape(Rectangle())
            }
            .plainButton(scale: 1)
            VStack(alignment: .leading, spacing: 0) {
                TitleMeta(item: item, alignment: .leading)
                    .frame(minHeight: 50, alignment: .topLeading)
                    .contentShape(Rectangle())
                    .onTapGesture(perform: open)
                HStack(spacing: 12) {
                    Button {
                        nav.pendingRoute = Route.item(item, zoom: HomeView.heroZoomID, autoplay: Autoplay())
                    } label: {
                        PlayCapsule(icon: "play.fill", title: item.resumeSeconds == nil ? "Play" : "Resume",
                                    time: item.resumeSeconds, progress: item.progress)
                            .frame(width: PlayCapsule.padWidth)
                    }
                    .plainButton()
                    .shadow(color: .black.opacity(0.3), radius: 8, y: 3)
                    Button(action: open) {
                        Image(systemName: "info")
                            .font(.headline.weight(.bold))
                            .frame(width: PlayCapsule.height, height: PlayCapsule.height)
                            .contentShape(Circle())
                    }
                    .plainButton()
                    .glassEffect(.regular.interactive(), in: .circle)
                    .accessibilityLabel("More Info")
                }
                .padding(.top, 16)
            }
            .padding(.horizontal, metrics.hPadding)
            .padding(.top, -TitleHeader.overlap(metrics, hasImage: client?.backdropURL(item) != nil))
            Spacer(minLength: 0)
        }
        .foregroundStyle(.white)
    }

    private func open() {
        nav.pendingRoute = Route.item(item, zoom: HomeView.heroZoomID)
    }

    private var phoneBody: some View {
        // Same structure and spacing as the title page: header, then (18 pt below) the info line.
        VStack(spacing: 18) {
            TitleHeader(image: client?.backdropURL(item), logo: client?.logoURL(item), title: item.displayTitle)
            TitleMeta(item: item)
                .padding(.horizontal, metrics.hPadding)
            #if os(tvOS)
            // The remote needs to show what's focused: a "More Info" pill lights up on the banner.
            Label("More Info", systemImage: "info.circle")
                .font(.callout.weight(.semibold))
                .padding(.horizontal, 26).padding(.vertical, 12)
                .foregroundStyle(focused ? .black : .white)
                .background(focused ? AnyShapeStyle(.white) : AnyShapeStyle(.white.opacity(0.15)), in: Capsule())
                .animation(.easeOut(duration: 0.15), value: focused)
            #endif
            Spacer(minLength: 0)
        }
        .foregroundStyle(.white)
        .contentShape(Rectangle())
    }
}
