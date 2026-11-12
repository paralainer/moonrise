#if os(tvOS)
import SwiftUI

extension PlayerScreen {
    @ViewBuilder var body: some View {
        Group {
            if let av = model.engine as? AVPlayerEngine {
                // Apple TV: the system player (native transport bar, kinetic scrubbing, info panels).
                TVNativePlayer(model: model, engine: av, refresh: nativeRefreshKey,
                               onSearchSubtitles: { openSubtitleSearch() },
                               onTransportBar: { tvBarVisible = $0 })
                    .ignoresSafeArea()
                    .overlay {
                        // Above the transport bar (title, scrubber, the tabs row) while it shows.
                        GeometryReader { geo in
                            SubtitleOverlay(text: model.overlayText, height: geo.size.height,
                                            raisedBottom: tvBarVisible ? Self.tvRaisedBottom : nil)
                        }
                        .allowsHitTesting(false)
                    }
                    .overlay { StreamRecoveryOverlay(model: model, compact: false) }
                    .overlay(alignment: .bottomTrailing) { tvSkipButton }
                    .overlay {
                        if let err = model.errorMessage { ErrorBanner(title: model.errorTitle, message: err, detail: model.errorDetail) { model.errorMessage = nil } }
                    }
            } else if model.engine == nil, model.errorMessage == nil, model.streamFailure == nil {
                // Loading: black with the system spinner, as the TV app does, not our own player.
                Color.black
                    .ignoresSafeArea()
                    .overlay { ProgressView().controlSize(.large).tint(.white) }
                    .focusable()
                    .onExitCommand { close() }
            } else {
                customPlayer // VLC-only files, and a stream that failed before it started
            }
        }
        // Search Online… from the Subtitles menu: over the paused player; Menu closes it and
        // playback picks up again.
        .fullScreenCover(isPresented: $showSubtitleSearch, onDismiss: closeSubtitleSearch) {
            SubtitleSearchView(model: model)
        }
        // The system player pauses while its menu is open and plays again once it closes, under
        // the search screen: hold it until the screen closes.
        .onChange(of: model.isPlaying) { _, playing in
            #if DEBUG
            if showSubtitleSearch { fputs("MoonriseSubSearch: playing=\(playing) under the search\n", stderr) }
            #endif
            guard showSubtitleSearch, playing else { return }
            resumeAfterSearch = true
            model.togglePlay()
        }
        // Started and closed once for the whole screen (the content switches as the engine appears).
        .task { wireEpisodeSwitching(); await model.start() }
        #if DEBUG
        // Test seam: MOONRISE_OPEN_SUBSEARCH=<seconds> opens Find Subtitles that long after the player shows.
        .task {
            if let t = DebugSeams.launch.openSubSearch {
                try? await Task.sleep(for: .seconds(t))
                openSubtitleSearch()
            }
        }
        #endif
        // The idle timer follows playback (PlayerModel's KeepAwakeAssertion): paused
        // lets the Apple TV start its screensaver.
        .onDisappear { model.close() }
    }

    private func openSubtitleSearch() {
        #if DEBUG
        fputs("MoonriseSubSearch: open (playing=\(model.isPlaying))\n", stderr)
        #endif
        resumeAfterSearch = model.isPlaying
        if model.isPlaying { model.togglePlay() }
        showSubtitleSearch = true
    }

    private func closeSubtitleSearch() {
        #if DEBUG
        fputs("MoonriseSubSearch: closed (resume=\(resumeAfterSearch))\n", stderr)
        #endif
        if resumeAfterSearch, !model.isPlaying { model.togglePlay() }
        resumeAfterSearch = false
    }

    /// Skip Intro in glass, in the transport bar's bottom row (the Info/Episodes tabs' row, at its
    /// right end); the same place while the bar is hidden. Not for credits that run to the end:
    /// Next Episode is the swipe-down row's first tab then (TVNativePlayer), the only one.
    @ViewBuilder private var tvSkipButton: some View {
        if let seg = model.activeSegment, model.segmentPromptVisible || model.autoSkipProgress != nil,
           !(seg.kind == .credits && model.creditsSkipPlaysNext) {
            ZStack(alignment: .bottomTrailing) {
                Color.clear
                SkipSegmentButton(model: model, segment: seg)
                    .padding(.trailing, 86)
                    .padding(.bottom, 45)
            }
            .ignoresSafeArea()
            .transition(.opacity)
        }
    }

    /// Changes whenever something the system player shows from the model changes.
    private var nativeRefreshKey: String {
        let tracks: String = model.audioOptions.map(\.title).joined() + "\(model.selectedAudio ?? -1)"
            + model.subtitleOptions.map(\.id).joined() + (model.selectedSubtitleId ?? "-") + model.subtitleTimingKey
        let quality: String = model.quality.label + model.qualityOptions.map(\.label).joined() + (model.playingVersionId ?? "") + "\(model.versions.count)"
        let segment: String = (model.activeSegment?.id ?? "") + "\(model.segmentPromptVisible)"
            + (model.autoSkipProgress.map { "\(Int($0 * 10))" } ?? "-")
        let next: String = "\(model.seriesEpisodes.count)" + (model.isRecovering ? "r" : "") + (model.nextEpisodeOffered ? "n" : "")
        let style = "\(AppSettings.shared.subtitleSize.rawValue)\(AppSettings.shared.subtitleBackground)"
        return model.title + "|" + tracks + "|" + quality + "|" + segment + "|" + next + "|" + style
    }
}
#endif
