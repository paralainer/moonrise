#if os(tvOS)
import AVKit
import SwiftUI

/// Apple TV playback in the system player (AVPlayerViewController): the native transport bar
/// with kinetic, clickpad and swipe scrubbing, the info/audio/subtitle panels on swipe-down, and
/// Siri Remote behaviour people know from the TV app. Moonrise's extras plug into its extension
/// points: audio, subtitles (with Search Online…) and quality as transport-bar menus,
/// title/overview as the item's metadata. Moonrise's subtitles and Skip Intro (a glass button in
/// the transport bar's bottom row; the system's contextual action is a white platter) are drawn
/// above it by PlayerScreen.
struct TVNativePlayer: UIViewControllerRepresentable {
    let model: PlayerModel
    let engine: AVPlayerEngine
    /// Changes when model state shown by the system player changes, so SwiftUI calls update.
    let refresh: String
    /// "Search Online…" in the Subtitles menu.
    let onSearchSubtitles: () -> Void
    /// The transport bar is about to show (true) or hide (false).
    var onTransportBar: (Bool) -> Void = { _ in }

    func makeUIViewController(context: Context) -> AVPlayerViewController {
        let vc = AVPlayerViewController()
        vc.player = engine.player
        vc.appliesPreferredDisplayCriteriaAutomatically = true // match frame rate / dynamic range
        vc.allowsPictureInPicturePlayback = false
        // Hide the system subtitle menu (embedded CC etc.): Moonrise's own Subtitles menu covers them.
        vc.allowedSubtitleOptionLanguages = ["zxx"]
        vc.delegate = context.coordinator
        context.coordinator.attach(vc)
        return vc
    }

    func updateUIViewController(_ vc: AVPlayerViewController, context: Context) {
        if vc.player !== engine.player { vc.player = engine.player }
        context.coordinator.update(vc)
    }

    /// Leaving (another episode replaces this player): close the swipe-down panel. An open panel
    /// isn't taken down with the player and stayed over the next one.
    static func dismantleUIViewController(_ vc: AVPlayerViewController, coordinator: Coordinator) {
        coordinator.detach(vc)
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(model: model, onSearchSubtitles: onSearchSubtitles, onTransportBar: onTransportBar)
    }

    @MainActor
    final class Coordinator: NSObject, AVPlayerViewControllerDelegate {
        let model: PlayerModel
        let onSearchSubtitles: () -> Void
        let onTransportBar: (Bool) -> Void
        private var lastQualities: [String] = []

        init(model: PlayerModel, onSearchSubtitles: @escaping () -> Void, onTransportBar: @escaping (Bool) -> Void) {
            self.model = model
            self.onSearchSubtitles = onSearchSubtitles
            self.onTransportBar = onTransportBar
        }

        /// Moonrise's subtitles move up out of the bar's way while it shows (PlayerScreen).
        nonisolated func playerViewController(_ playerViewController: AVPlayerViewController,
                                              willTransitionToVisibilityOfTransportBar visible: Bool,
                                              with coordinator: any AVPlayerViewControllerAnimationCoordinator) {
            MainActor.assumeIsolated {
                #if DEBUG
                fputs("MoonriseTransportBar: \(visible ? "shown" : "hidden")\n", stderr)
                #endif
                onTransportBar(visible)
            }
        }

        func attach(_ vc: AVPlayerViewController) {
            player = vc
            update(vc)
        }

        /// The player is going away (another episode replaces it): an open swipe-down panel would
        /// outlive it, over the next episode's player, so it closes first; the select watcher
        /// leaves the window.
        func detach(_ vc: AVPlayerViewController) {
            if let selectPress { selectPress.view?.removeGestureRecognizer(selectPress) }
            selectPress = nil
            nextTab = nil
            pressPlays = nil
            // The open panel is presented by the player (AVInfoPanelViewController); taking the
            // tabs away instead crashed the system player.
            if vc.presentedViewController != nil {
                #if DEBUG
                fputs("MoonriseNext: closing the swipe-down panel\n", stderr)
                #endif
                vc.dismiss(animated: false)
            }
        }

        // MARK: Select on the Next Episode tab

        /// The tab row's buttons are the system player's own: select on one only opens or closes
        /// its panel. A select-press watcher in the window (it takes nothing from the system
        /// player) plays the next episode when the press is meant for the Next Episode tab.
        private weak var player: AVPlayerViewController?
        private var selectPress: UITapGestureRecognizer?
        private weak var nextTab: UIViewController?
        /// The episode the press that just started is for: it began on the Next Episode tab
        /// (decided then, as focus can move before the press ends).
        private var pressPlays: MediaItem?

        private func watchSelect(in window: UIWindow) {
            guard selectPress?.view !== window else { return }
            if let selectPress { selectPress.view?.removeGestureRecognizer(selectPress) }
            let g = UITapGestureRecognizer(target: self, action: #selector(selectPressed))
            g.allowedPressTypes = [NSNumber(value: UIPress.PressType.select.rawValue)]
            g.allowedTouchTypes = []
            g.cancelsTouchesInView = false
            g.delegate = self
            window.addGestureRecognizer(g)
            selectPress = g
        }

        fileprivate func pressBegan() {
            let target = pressTarget
            pressPlays = target == .nextTab || target == .nextPanelOpening ? model.nextEpisode : nil
            #if DEBUG
            fputs("MoonriseNext: select press began: \(target)\n", stderr)
            #endif
        }

        @objc private func selectPressed() {
            guard let episode = pressPlays else { return }
            pressPlays = nil
            #if DEBUG
            fputs("MoonriseNext: select on the Next Episode tab\n", stderr)
            #endif
            model.playEpisode(episode)
        }

        private enum PressTarget { case none, nextTab, nextPanelOpening, inNextPanel, otherTab, panelClosing }

        /// What a select press starting now is for, as the viewer sees it:
        /// - the focused item is the Next Episode tab: a view of the system player's tab row whose
        ///   only text is that title (with the controls hidden the focus is on the player's
        ///   whole-screen container, which holds every tab's title);
        /// - or the Next Episode panel is opening with its tab drawn focused, but focus hasn't got
        ///   there yet. A swipe down presents the panel at once and moves focus a moment later
        ///   (longer while the app is busy, as in a slow or stalled start), and the system player
        ///   drops a select in between: the viewer's click did nothing.
        /// Not when focus is in our panel (its still is a button that plays it), on another tab
        /// (its own panel is up then), or while the panel closes (Menu, then select at once).
        private var pressTarget: PressTarget {
            guard let vc = player, let tab = nextTab, let panel = tab.viewIfLoaded, model.nextEpisodeOffered, model.nextEpisode != nil else { return .none }
            let item = UIFocusSystem.focusSystem(for: vc)?.focusedItem
            if let item, Self.isInside(item, tab: tab, panel: panel) { return .inNextPanel }
            let focused = item as? UIView
            let presented = vc.presentedViewController
            if presented?.isBeingDismissed == true { return .panelClosing }
            let texts = focused.map(Self.texts(in:)) ?? []
            if texts == [Self.nextEpisodeTitle] { return .nextTab }
            if texts.count == 1 { return .otherTab }
            // The swipe-down panel (AVKit presents it) is up and shows ours.
            if let presented, panel.isDescendant(of: presented.view) { return .nextPanelOpening }
            return .none
        }

        /// The focused item is in our Next Episode panel. The still is a SwiftUI button, whose focus
        /// item isn't a view: its focus environments lead up to the panel's hosting view and
        /// controller.
        private static func isInside(_ item: UIFocusItem, tab: UIViewController, panel: UIView) -> Bool {
            var env: UIFocusEnvironment? = item
            while let e = env {
                if e === tab { return true }
                if let v = e as? UIView, v.isDescendant(of: panel) { return true }
                env = e.parentFocusEnvironment
            }
            return false
        }

        private static func texts(in view: UIView) -> Set<String> {
            var found = Set([(view as? UILabel)?.text, view.accessibilityLabel].compactMap { $0 }.filter { !$0.isEmpty })
            for sub in view.subviews { found.formUnion(texts(in: sub)) }
            return found
        }

        static let nextEpisodeTitle = "Next Episode"

        private var episodesTabKey = ""

        func update(_ vc: AVPlayerViewController) {
            if let window = vc.view.window { watchSelect(in: window) }
            setEpisodesTab(vc)
            setMetadata(vc)
            setQualityMenu(vc)
        }

        /// The Info tab (swipe down). AVKit shows the artwork; the subtitle as the heading, else the
        /// title; the overview; then genres, year, runtime, age rating and its own badges for the
        /// stream playing (HD, CC…). What goes in: `PlayerInfoFields`. Near the end, title and
        /// subtitle only, so Next Episode is the row's first tab and a swipe down lands on it.
        private func setMetadata(_ vc: AVPlayerViewController) {
            guard let item = vc.player?.currentItem else { return }
            let next = model.nextEpisodeOffered
            loadInfoExtras()
            let key = "\(ObjectIdentifier(item).hashValue)|\(next)|\(artwork?.count ?? 0)|\(titleDetails?.id ?? "")"
            guard key != metadataKey else { return }
            metadataKey = key
            let fields = PlayerInfoFields.make(item: model.item, title: model.title, subtitle: model.subtitle,
                                               details: titleDetails, nearEnd: next)
            var meta: [AVMetadataItem] = [metadata(.commonIdentifierTitle, fields.title)]
            if let s = fields.subtitle { meta.append(metadata(.iTunesMetadataTrackSubTitle, s)) }
            if !next {
                if let o = fields.overview { meta.append(metadata(.commonIdentifierDescription, o)) }
                meta += infoDetails(fields)
            }
            // Artwork once shown stays (and with it the Info tab) when the item is left out: an
            // artwork item with no value takes it away.
            if next, artwork != nil {
                let m = AVMutableMetadataItem()
                m.identifier = .commonIdentifierArtwork
                m.extendedLanguageTag = "und"
                meta.append(m.copy() as! AVMetadataItem)
            }
            item.externalMetadata = meta
        }

        private var metadataKey = ""
        /// The item the artwork and details below are for.
        private var infoItemId: String?
        /// JPEG: a movie's poster, an episode's still.
        private var artwork: Data?
        /// Where the age rating and genres come from: the movie, or an episode's show.
        private var titleDetails: MediaItem?

        /// Fetches the artwork and an episode's show once per item, without holding up playback;
        /// the metadata is set again when they arrive.
        private func loadInfoExtras() {
            guard let item = model.item, item.id != infoItemId else { return }
            infoItemId = item.id
            artwork = nil
            titleDetails = item.isEpisode ? nil : item
            guard let client = model.client else { return }
            let url: URL? = item.isEpisode
                ? (item.images.primary ?? item.images.thumb).map { client.imageURL($0, width: 800) }
                : (item.images.poster.map { client.imageURL($0, width: 400) } ?? item.images.backdrop.map { client.imageURL($0, width: 800) })
            let seriesId = item.isEpisode ? item.seriesId : nil
            Task { [weak self] in
                async let image: PlatformImage? = { guard let url else { return nil }; return await ImageCache.shared.image(for: url) }()
                async let show: MediaItem? = { guard let seriesId else { return nil }; return try? await client.item(seriesId) }()
                let (img, series) = await (image, show)
                guard let self, self.infoItemId == item.id else { return }
                self.artwork = img?.jpegData(compressionQuality: 0.85)
                if let series { self.titleDetails = series }
                if let vc = self.player { self.setMetadata(vc) }
            }
        }

        /// Artwork, age rating, genres and year: the identifiers AVKit's Info tab shows.
        private func infoDetails(_ fields: PlayerInfoFields) -> [AVMetadataItem] {
            var out: [AVMetadataItem] = []
            if let artwork {
                let m = AVMutableMetadataItem()
                m.identifier = .commonIdentifierArtwork
                m.value = artwork as NSData
                m.dataType = kCMMetadataBaseDataType_JPEG as String
                m.extendedLanguageTag = "und"
                out.append(m.copy() as! AVMetadataItem)
            }
            if let r = fields.rating { out.append(metadata(.iTunesMetadataContentRating, r)) }
            if let g = fields.genres { out.append(metadata(.quickTimeMetadataGenre, g)) }
            if let date = fields.releaseDate {
                // A date: AVKit shows its year (a string it reads as something else).
                let m = AVMutableMetadataItem()
                m.identifier = .commonIdentifierCreationDate
                m.value = date as NSDate
                m.extendedLanguageTag = "und"
                out.append(m.copy() as! AVMetadataItem)
            }
            return out
        }

        private func metadata(_ id: AVMetadataIdentifier, _ value: String) -> AVMetadataItem {
            let m = AVMutableMetadataItem()
            m.identifier = id
            m.value = value as NSString
            m.extendedLanguageTag = "und"
            return m.copy() as! AVMetadataItem
        }

        /// Audio, Subtitles and Quality in the transport bar. Moonrise renders text subtitles itself, so
        /// its Subtitles menu replaces the system's (with Search Online…). Its Audio menu says what
        /// tells a file's tracks apart (dub studio, commentary, Dolby Digital 5.1) under each
        /// language; the system's own audio button stays (it can't be hidden) and only names the
        /// language, "Russian" five times over. A pick there is the viewer's choice too
        /// (PlayerModel.engineAudioSelected).
        private func setQualityMenu(_ vc: AVPlayerViewController) {
            let audio: [String] = [model.audioOptions.map { String($0.id) }.joined(separator: ","), model.selectedAudio.map(String.init) ?? "-"]
            let key: [String] = audio + [model.subtitleOptions.map(\.id).joined(separator: ","), model.selectedSubtitleId ?? "-",
                       model.qualityAvailable ? model.qualityOptions.map(\.label).joined() : "", model.quality.label,
                       model.versions.map(\.id).joined(separator: ","), model.playingVersionId ?? "-",
                       String(model.seriesEpisodes.count), String(model.canSearchSubtitles),
                       AppSettings.shared.subtitleSize.rawValue, String(AppSettings.shared.subtitleBackground), model.subtitleTimingKey]
            guard key != lastQualities else { return }
            lastQualities = key
            let canSearch = model.searchContext.server != nil && model.canSearchSubtitles
            let search = UIAction(title: "Search Online…", image: UIImage(systemName: "magnifyingglass")) { [weak self] _ in
                self?.onSearchSubtitles()
            }
            vc.transportBarCustomMenuItems = [
                TrackMenus.audio(model),
                model.subtitleOptions.isEmpty && !canSearch ? nil : TrackMenus.subtitles(model, extras: [TrackMenus.delay(model)] + (canSearch ? [search] : [])),
                TrackMenus.quality(model),
            ].compactMap { $0 }
        }

        /// "Episodes" tab next to "Info" in the swipe-down panel (as in the TV app), and near the
        /// end of an episode "Next Episode" first: swiping down lands on it, its panel previews the
        /// next episode, and a click on the tab or on the still plays it.
        private func setEpisodesTab(_ vc: AVPlayerViewController) {
            let key = "\(model.seriesEpisodes.count)|\(model.nextEpisodeOffered ? model.nextEpisode?.id ?? "" : "")"
            guard key != episodesTabKey else { return }
            episodesTabKey = key
            guard !model.seriesEpisodes.isEmpty else { vc.customInfoViewControllers = []; return }
            var tabs: [UIViewController] = []
            nextTab = nil
            if model.nextEpisodeOffered, let next = model.nextEpisode {
                let tab = panel(TVNextEpisodeTab(model: model, episode: next), title: Self.nextEpisodeTitle)
                nextTab = tab
                tabs.append(tab)
            }
            tabs.append(panel(TVEpisodesTab(model: model), title: "Episodes"))
            vc.customInfoViewControllers = tabs
            #if DEBUG
            fputs("MoonriseNext: tabs \(tabs.map { $0.title ?? "" })\n", stderr)
            #endif
        }

        private func panel(_ view: some View, title: String) -> UIViewController {
            let host = UIHostingController(rootView: view)
            host.title = title
            host.preferredContentSize = CGSize(width: 1920, height: TVEpisodesTab.height)
            // Reopened while it was still closing, the panel was laid out from the hosting view's
            // own fitting size (much taller), so it rose up the screen. Pin the view's height.
            host.sizingOptions = []
            host.view.translatesAutoresizingMaskIntoConstraints = true
            let pin = host.view.heightAnchor.constraint(equalToConstant: TVEpisodesTab.height)
            pin.priority = .required
            pin.isActive = true
            host.view.setContentHuggingPriority(.required, for: .vertical)
            host.view.setContentCompressionResistancePriority(.required, for: .vertical)
            return host
        }
    }
}

extension TVNativePlayer.Coordinator: UIGestureRecognizerDelegate {
    nonisolated func gestureRecognizer(_ g: UIGestureRecognizer, shouldReceive press: UIPress) -> Bool {
        MainActor.assumeIsolated { pressBegan() }
        return true
    }

    /// The system player handles the same press as usual.
    nonisolated func gestureRecognizer(_ g: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool { true }
}

/// Episode shelf for the info panel: current season first, then the rest in order.
private struct TVEpisodesTab: View {
    /// The panels' fixed height: smaller lockups than the page's, so the panel covers less of
    /// the picture (and a late size change can't push it further up).
    static let height: CGFloat = 300
    static let thumb = CGSize(width: 304, height: 171)
    let model: PlayerModel
    @State private var position: String?
    @FocusState private var focused: String?

    var body: some View {
        ScrollView(.horizontal) {
            LazyHStack(alignment: .top, spacing: 32) {
                ForEach(model.seriesEpisodes) { ep in
                    Button { model.playEpisode(ep) } label: {
                        ZStack(alignment: .bottom) {
                            RemoteImage(url: model.client?.thumbURL(ep, maxWidth: 600))
                            if let p = ep.progress { ProgressBar(value: p).padding(10) }
                        }
                        .frame(width: Self.thumb.width, height: Self.thumb.height)
                        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                        .overlay(alignment: .topTrailing) { WatchedMark(item: ep) }
                        .hoverEffect(.highlight)
                        VStack(alignment: .leading, spacing: 2) {
                            Text("\(ep.episodeLabel ?? "") · \(ep.displayTitle)").font(.caption.weight(.medium))
                                .lineLimit(1)
                            Text(ep.id == model.item?.id ? "Now Playing" : (Fmt.runtime(seconds: ep.runtime) ?? " "))
                                .font(.caption2).foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                        .frame(width: Self.thumb.width, alignment: .leading)
                    }
                    .buttonStyle(.borderless)
                    .focused($focused, equals: ep.id)
                    .id(ep.id)
                }
            }
            .scrollTargetLayout()
            // The system Info tab's text inset; its panel already sits on the 80-pt margin.
            .padding(.horizontal, 24)
            .padding(.vertical, 26)
        }
        // A panel like the Info tab's, so the titles read over any picture; it clips the row
        // (scrolled lockups don't spill past it), the padding leaves room for the focus lift.
        .background(.regularMaterial)
        .clipShape(RoundedRectangle(cornerRadius: 40, style: .continuous))
        .environment(\.colorScheme, .dark)
        // Open on the episode that's playing, with focus on it.
        .scrollPosition(id: $position, anchor: .leading)
        .defaultFocus($focused, model.item?.id)
        .onAppear { position = model.item?.id }
    }
}
/// The next episode in the swipe-down panel: its still (a click plays it), title, length and
/// overview, in the Episodes tab's panel.
private struct TVNextEpisodeTab: View {
    let model: PlayerModel
    let episode: MediaItem
    @FocusState private var focused: Bool

    var body: some View {
        HStack(alignment: .top, spacing: 36) {
            Button { model.playNextEpisode() } label: {
                RemoteImage(url: model.client?.thumbURL(episode, maxWidth: 600))
                    .frame(width: TVEpisodesTab.thumb.width, height: TVEpisodesTab.thumb.height)
                    .overlay {
                        Image(systemName: "play.fill").font(.title2).foregroundStyle(.white)
                            .padding(22).glassEffect(.regular, in: .circle)
                    }
                    .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                    .hoverEffect(.highlight)
            }
            .buttonStyle(.borderless)
            .focused($focused)
            VStack(alignment: .leading, spacing: 8) {
                Text([episode.episodeLabel, Fmt.runtime(seconds: episode.runtime)].compactMap { $0 }.joined(separator: " · "))
                    .font(.caption).foregroundStyle(.secondary)
                Text(episode.displayTitle).font(.headline).lineLimit(1)
                if let o = episode.overview, !o.isEmpty {
                    Text(o).font(.caption).foregroundStyle(.secondary).lineLimit(4)
                }
            }
            .frame(maxWidth: 1000, alignment: .leading)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 26)
        // No fixed height on the root: it kept focus from moving down into the panel.
        .background(.regularMaterial)
        .clipShape(RoundedRectangle(cornerRadius: 40, style: .continuous))
        .environment(\.colorScheme, .dark)
        .defaultFocus($focused, true)
    }
}
#endif
