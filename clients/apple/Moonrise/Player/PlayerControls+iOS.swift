#if os(iOS)
import SwiftUI

extension PlayerControls {
    /// Full screen, laid out like the TV app: close and AirPlay/Fill/rotate top left, title
    /// bottom left, Quality/Audio/Subtitles above the scrubber on the right, Episodes below it.
    var fullScreenLayout: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                closeButton
                    .controlsOccupied(.top)
                HStack(spacing: 4) {
                    RoutePickerButton()
                        .frame(width: buttonSize, height: buttonSize)
                        .accessibilityLabel("AirPlay")
                    // (No full-screen button: the chevron, and a swipe down, go back to the page.)
                    fillButton
                }
                .font(.system(size: iconSize - 2, weight: .medium))
                .padding(.horizontal, 6)
                .glassEffect(.regular.interactive(), in: .capsule)
                .controlsOccupied(.top)
                Spacer()
            }

            Spacer()

            HStack(alignment: .bottom, spacing: 16) {
                VStack(alignment: .leading, spacing: 2) {
                    if let sub = model.subtitle, !sub.isEmpty {
                        Text(sub).font(.subheadline.weight(.semibold)).lineLimit(1)
                    }
                    Text(model.title).font(.title2.bold()).lineLimit(1)
                }
                .shadow(color: .black.opacity(0.5), radius: 6)
                .controlsOccupied(.bottom)
                Spacer()
                HStack(spacing: 4) {
                    if model.qualityAvailable { qualityButton }
                    trackButtons
                    downloadButton
                }
                .padding(.horizontal, 6)
                .glassEffect(.regular.interactive(), in: .capsule)
                .controlsOccupied(.bottom)
            }
            .padding(.bottom, 12)

            HStack(spacing: 12) {
                Text(Fmt.duration(scrubbing ?? model.shownTime))
                Scrubber(current: scrubbing ?? model.shownTime, duration: model.duration,
                         onScrub: { scrubbing = $0; onInteraction() },
                         onCommit: { model.seek(to: $0); scrubbing = nil; onInteraction() },
                         thumbnails: model.thumbnails)
                Text("−" + Fmt.duration(max(0, model.duration - (scrubbing ?? model.shownTime))))
            }
            .font(.footnote.monospacedDigit().weight(.medium))
            .shadow(color: .black.opacity(0.5), radius: 4)
            .controlsOccupied(.bottom)

            // Episodes on the left, Skip Intro on the right (PlayerScreen draws Skip over its
            // place here, so it stays put as the controls come and go).
            if hasEpisodes || model.activeSegment != nil || model.nextEpisodeOffered {
                HStack {
                    if let onEpisodes, hasEpisodes {
                        Button { onEpisodes(); onInteraction() } label: {
                            Text("Episodes")
                                .font(.headline)
                                .padding(.horizontal, 18)
                                .frame(height: 40)
                                .contentShape(Capsule())
                        }
                        .plainButton()
                        .glassEffect(.regular.interactive(), in: .capsule)
                        .controlsOccupied(.bottom)
                    }
                    Spacer()
                    skipPlaceholder.controlsOccupied(.bottom)
                }
                .padding(.top, 8)
            }
        }
    }

    private var hasEpisodes: Bool { onEpisodes != nil && !model.seriesEpisodes.isEmpty }

    private var fillButton: some View {
        Button { model.fill.toggle(); onInteraction() } label: {
            Image(systemName: model.fill ? "rectangle.arrowtriangle.2.inward" : "rectangle.arrowtriangle.2.outward")
                .frame(width: buttonSize, height: buttonSize)
                .contentShape(Rectangle())
        }
        .plainButton()
        .accessibilityLabel(model.fill ? "Fit" : "Fill")
    }

    /// Portrait <-> landscape without needing to turn the phone (works with rotation lock on).
    var rotateButton: some View {
        Button {
            if let onFullScreen { onFullScreen() } else { SystemControls.rotate(toLandscape: compact) }
            onInteraction()
        } label: {
            Image(systemName: compact ? "arrow.up.left.and.arrow.down.right" : "arrow.down.right.and.arrow.up.left")
                .frame(width: compact ? 28 : buttonSize, height: compact ? 28 : buttonSize)
                .contentShape(Rectangle())
        }
        .plainButton()
        .accessibilityLabel(compact ? "Full Screen" : "Exit Full Screen")
    }

    var qualityButton: some View {
        MenuButton(systemImage: "dial.medium", pointSize: iconSize - 4, accessibilityLabel: "Quality",
                   menu: { [model] in
                       TrackMenus.quality(model, title: model.playMethodLabel.isEmpty ? "Quality" : "Quality · \(model.playMethodLabel)")
                   },
                   onMenu: onMenu)
            .frame(width: buttonSize, height: buttonSize)
    }
}
#endif
