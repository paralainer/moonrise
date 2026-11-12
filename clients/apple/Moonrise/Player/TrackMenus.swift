#if canImport(UIKit)
import SwiftUI
import UIKit

/// Audio, Subtitles and Quality menus in UIKit, shared by the iOS controls and the tvOS
/// transport bar. Rows follow the TV app: checkmark, language on top, details below.
@MainActor
enum TrackMenus {
    /// On the Apple TV it sits beside the system's own audio button (a waveform: Reduce Loud
    /// Sounds, and a track list that names each track by its language only), so its icon is the
    /// speaker.
    static func audio(_ model: PlayerModel) -> UIMenu? {
        guard model.audioOptions.count > 1 else { return nil }
        #if os(tvOS)
        let icon = "speaker.wave.2"
        #else
        let icon = "waveform"
        #endif
        return UIMenu(title: "Audio", image: UIImage(systemName: icon), children: model.audioOptions.map { a in
            UIAction(title: a.title, subtitle: a.detail, state: a.id == model.selectedAudio ? .on : .off) { [weak model] _ in
                model?.chooseAudio(a.id)
            }
        })
    }

    /// `extras` go below the languages and Style (Delay, Search Online).
    static func subtitles(_ model: PlayerModel, extras: [UIMenuElement] = []) -> UIMenu {
        let off = UIAction(title: "Off", state: model.selectedSubtitleId == nil ? .on : .off) { [weak model] _ in
            model?.chooseSubtitle(nil)
        }
        let languages = UIMenu(options: .displayInline, children: [off] + model.subtitleOptions.map { o in
            UIAction(title: o.name, subtitle: model.subtitleDetail(o), state: o.id == model.selectedSubtitleId ? .on : .off) { [weak model] _ in
                model?.chooseSubtitle(o)
            }
        })
        return UIMenu(title: "Subtitles", image: UIImage(systemName: "captions.bubble"),
                      children: [languages, UIMenu(options: .displayInline, children: [style()] + extras)])
    }

    /// Quality, and (a title with several versions) its Version submenu first: cut, quality, dub.
    static func quality(_ model: PlayerModel, title: String = "Quality") -> UIMenu? {
        guard model.qualityAvailable else { return nil }
        let qualities: [UIMenuElement] = model.qualityOptions.map { q in
            UIAction(title: q.label, subtitle: model.sizeLabel(q), state: q == model.quality ? .on : .off) { [weak model] _ in
                afterMenu { model?.setQuality(q) }
            }
        }
        guard let versions = version(model) else {
            return UIMenu(title: title, image: UIImage(systemName: "dial.medium"), children: qualities)
        }
        return UIMenu(title: title, image: UIImage(systemName: "dial.medium"),
                      children: [UIMenu(options: .displayInline, children: [versions]), UIMenu(options: .displayInline, children: qualities)])
    }

    /// The title's versions (several only): picking one restarts it there at the same position.
    static func version(_ model: PlayerModel) -> UIMenu? {
        guard model.versions.count > 1 else { return nil }
        let menu = UIMenu(title: "Version", image: UIImage(systemName: "square.stack"), children: model.versions.map { v in
            UIAction(title: PlayerModel.versionTitle(v), subtitle: PlayerModel.versionDetails(v),
                     attributes: v.isBroken && v.id != model.playingVersionId ? .disabled : [],
                     state: v.id == model.playingVersionId ? .on : .off) { [weak model] _ in
                afterMenu { model?.chooseVersion(v.id) }
            }
        })
        menu.subtitle = model.versions.first { $0.id == model.playingVersionId }.map(PlayerModel.versionTitle)
        return menu
    }

    /// A pick that restarts the stream (quality, version). On the Apple TV it waits for the
    /// transport bar's menu to close: a stream restarted while it was closing played on with a
    /// black picture.
    private static func afterMenu(_ action: @escaping @MainActor () -> Void) {
        #if os(tvOS)
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(600))
            action()
        }
        #else
        action()
        #endif
    }

    /// Size and background of Moonrise's subtitle overlay.
    static func style() -> UIMenu {
        let settings = AppSettings.shared
        let sizes = SubtitleSize.allCases.map { size in
            UIAction(title: size.rawValue.capitalized, state: settings.subtitleSize == size ? .on : .off) { _ in
                settings.subtitleSize = size
            }
        }
        let background = UIAction(title: "Background", state: settings.subtitleBackground ? .on : .off) { _ in
            settings.subtitleBackground.toggle()
        }
        return UIMenu(title: "Style", image: UIImage(systemName: "textformat.size"),
                      children: [UIMenu(options: .displayInline, children: sizes), background])
    }

    static func delay(_ model: PlayerModel) -> UIMenu {
        let steps = [-2.0, -1.0, -0.5, -0.1, 0.1, 0.5, 1.0, 2.0].map { d in
            UIAction(title: String(format: "%+.1fs", d)) { [weak model] _ in model?.nudgeSubtitles(by: d) }
        }
        let reset = UIAction(title: "Reset") { [weak model] _ in model?.resetSubtitleDelay() }
        var below: [UIMenuElement] = [reset]
        if model.subtitleSyncApplied {
            let original = model.subtitleOriginalTiming
            below.append(UIAction(title: "Use Original Timing", state: original ? .on : .off) { [weak model] _ in
                model?.useOriginalSubtitleTiming(!original)
            })
        }
        let menu = UIMenu(title: "Delay", image: UIImage(systemName: "clock.arrow.2.circlepath"),
                          children: [UIMenu(options: .displayInline, children: steps), UIMenu(options: .displayInline, children: below)])
        menu.subtitle = String(format: "%+.1fs", model.subtitleDelay)
        return menu
    }
}
#endif

#if os(iOS)
/// A glyph button that opens a UIKit menu on tap and reports when the menu opens and closes
/// (SwiftUI's Menu has no close callback), so the player can keep its controls up meanwhile.
struct MenuButton: UIViewRepresentable {
    let systemImage: String
    let pointSize: CGFloat
    let accessibilityLabel: String
    let menu: () -> UIMenu?
    let onMenu: (Bool) -> Void

    func makeUIView(context: Context) -> TrackedMenuButton {
        let b = TrackedMenuButton(type: .system)
        b.showsMenuAsPrimaryAction = true
        b.tintColor = .white
        update(b)
        return b
    }

    func updateUIView(_ b: TrackedMenuButton, context: Context) { update(b) }

    /// Fill the frame SwiftUI gives it, so the whole slot is tappable, not just the glyph.
    func sizeThatFits(_ proposal: ProposedViewSize, uiView: TrackedMenuButton, context: Context) -> CGSize? {
        CGSize(width: proposal.width ?? 44, height: proposal.height ?? 44)
    }

    private func update(_ b: TrackedMenuButton) {
        let config = UIImage.SymbolConfiguration(pointSize: pointSize, weight: .medium)
        b.setImage(UIImage(systemName: systemImage, withConfiguration: config), for: .normal)
        b.accessibilityLabel = accessibilityLabel
        b.onMenu = onMenu
        // Don't swap the menu while it's showing: UIKit would rebuild it under the finger.
        if !b.menuShowing { b.menu = menu() }
        b.pendingMenu = menu
    }
}

final class TrackedMenuButton: UIButton {
    var onMenu: ((Bool) -> Void)?
    var pendingMenu: (() -> UIMenu?)?
    private(set) var menuShowing = false

    override func contextMenuInteraction(_ interaction: UIContextMenuInteraction,
                                         willDisplayMenuFor configuration: UIContextMenuConfiguration,
                                         animator: (any UIContextMenuInteractionAnimating)?) {
        super.contextMenuInteraction(interaction, willDisplayMenuFor: configuration, animator: animator)
        menuShowing = true
        onMenu?(true)
    }

    override func contextMenuInteraction(_ interaction: UIContextMenuInteraction,
                                         willEndFor configuration: UIContextMenuConfiguration,
                                         animator: (any UIContextMenuInteractionAnimating)?) {
        super.contextMenuInteraction(interaction, willEndFor: configuration, animator: animator)
        menuShowing = false
        if let pendingMenu { menu = pendingMenu() } // pick up a selection made in the menu
        onMenu?(false)
    }
}
#endif
