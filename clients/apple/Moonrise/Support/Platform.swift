import SwiftUI

#if os(macOS)
import AppKit
typealias PlatformView = NSView
typealias PlatformImage = NSImage
extension Image {
    init(platformImage: NSImage) { self.init(nsImage: platformImage) }
}
#else
import UIKit
typealias PlatformView = UIView
typealias PlatformImage = UIImage
extension Image {
    init(platformImage: UIImage) { self.init(uiImage: platformImage) }
}
#endif

extension View {
    /// Inline navigation title where the platform supports it.
    @ViewBuilder func inlineTitle() -> some View {
        #if os(iOS)
        self.navigationBarTitleDisplayMode(.inline)
        #else
        self
        #endif
    }

    /// Green switches on touch/mouse platforms. tvOS draws toggles as rows and the tint would
    /// colour the label text instead, so leave its default look.
    @ViewBuilder func switchTint() -> some View {
        #if os(tvOS)
        self
        #else
        self.tint(.green)
        #endif
    }

    /// Cards and artwork: plain on touch/mouse; on tvOS they lift when focused (no grey platter).
    @ViewBuilder func cardButtonStyle() -> some View {
        #if os(tvOS)
        self.buttonStyle(.borderless) // system lockup: lift, specular highlight, parallax
        #else
        self.buttonStyle(.plain)
        #endif
    }

    /// The artwork part of a lockup (gets the tvOS highlight/parallax).
    @ViewBuilder func lockupArtwork() -> some View {
        #if os(tvOS)
        self.hoverEffect(.highlight)
        #else
        self
        #endif
    }

    /// Mac: artwork brightens and gets a light rim under the pointer (no scaling: rows clip).
    /// Unchanged elsewhere.
    @ViewBuilder func hoverHighlight(cornerRadius: CGFloat) -> some View {
        #if os(macOS)
        modifier(HoverHighlight(cornerRadius: cornerRadius))
        #else
        self
        #endif
    }

    /// A Menu whose label draws its own button (circle, capsule): on the Mac, without the
    /// bezel and the disclosure arrow AppKit adds to menu buttons. Unchanged elsewhere.
    @ViewBuilder func drawnMenu() -> some View {
        #if os(macOS)
        self.menuStyle(.button).buttonStyle(.plain).menuIndicator(.hidden)
        #else
        self
        #endif
    }

    /// A text button in Moonrise's Liquid Glass on Apple TV (`TVGlassButtonStyle`); unchanged elsewhere.
    @ViewBuilder func tvGlass() -> some View {
        #if os(tvOS)
        self.buttonStyle(TVGlassButtonStyle(height: 66))
        #else
        self
        #endif
    }

    /// Custom-drawn buttons: plain elsewhere; on tvOS a lift instead of the system's white platter,
    /// which would cover the button's own look.
    @ViewBuilder func plainButton(scale: CGFloat = 1.08) -> some View {
        #if os(tvOS)
        self.buttonStyle(TVFocusStyle(scale: scale))
        #else
        self.buttonStyle(.plain)
        #endif
    }
}

// MARK: - Zoom transitions (iOS): tapped artwork grows into the page it opens.

extension EnvironmentValues {
    /// Shared namespace pairing a tapped card with the page it opens.
    @Entry var zoomNamespace: Namespace.ID? = nil
}

extension View {
    /// Marks this view as the artwork a zoom transition starts from.
    @ViewBuilder func zoomSource(_ id: String?, in ns: Namespace.ID?, cornerRadius: CGFloat = 16) -> some View {
        #if os(iOS)
        if let id, let ns {
            matchedTransitionSource(id: id, in: ns) { $0.clipShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)) }
        } else { self }
        #else
        self
        #endif
    }

    /// Pushes this page with a zoom from its source instead of the default slide.
    @ViewBuilder func zoomDestination(_ id: String?, in ns: Namespace.ID?) -> some View {
        #if os(iOS)
        if let id, let ns { navigationTransition(.zoom(sourceID: id, in: ns)) } else { self }
        #else
        self
        #endif
    }
}

#if os(tvOS)
/// Apple TV Liquid Glass button (tvOS 26): clear glass with a white label at rest; focused, the
/// glass brightens (a white tint) and lifts with a shadow, like the player's Skip button — never
/// the flat white platter of the system's bordered style. `prominent` (a page's Play) carries a
/// light tint at rest as well. Circles are as tall as capsules, so a row of them lines up.
struct TVGlassButtonStyle: ButtonStyle {
    enum Shape { case capsule, circle, rounded(CGFloat) }
    var shape: Shape = .capsule
    var prominent = false
    /// Capsule/circle height; nil sizes to the label (rows of a list, chips).
    var height: CGFloat? = TVGlassButtonStyle.height
    /// Selected (e.g. the season shown): a stronger tint, kept while unfocused.
    var selected = false
    /// No glass at rest unless selected (a row of choices where only the chosen one has a plate).
    var bare = false

    static let height: CGFloat = 76

    func makeBody(configuration: Configuration) -> some View {
        GlassBody(configuration: configuration, style: self)
    }

    struct GlassBody: View {
        let configuration: Configuration
        let style: TVGlassButtonStyle
        @Environment(\.isFocused) private var focused
        @Environment(\.isEnabled) private var enabled
        @State private var width: CGFloat = 0

        /// Focus grows a button by about the same amount whatever its width (as the system's do),
        /// so a wide one doesn't spill out of its column.
        private var lift: CGFloat { width > 0 ? min(1.1, 1 + 26 / width) : 1.08 }

        private var shape: AnyShape {
            switch style.shape {
            case .capsule: AnyShape(Capsule())
            case .circle: AnyShape(Circle())
            case .rounded(let r): AnyShape(RoundedRectangle(cornerRadius: r, style: .continuous))
            }
        }

        private var tint: Double {
            if focused { return 0.35 }
            if style.selected { return 0.2 }
            return style.prominent ? 0.1 : 0
        }

        var body: some View {
            label
                .foregroundStyle(.white)
                .opacity(enabled ? 1 : 0.45)
                .contentShape(shape)
                .glassEffect(style.bare && !focused && !style.selected ? .identity : .regular.tint(.white.opacity(tint)), in: shape)
                .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
                .scaleEffect(configuration.isPressed ? 1.02 : focused ? lift : 1)
                .shadow(color: .black.opacity(focused ? 0.45 : 0), radius: 18, y: 10)
                .animation(.easeOut(duration: 0.18), value: focused)
                .animation(.easeOut(duration: 0.1), value: configuration.isPressed)
        }

        @ViewBuilder private var label: some View {
            switch style.shape {
            case .circle:
                configuration.label
                    .font(.body.weight(.semibold))
                    .frame(width: style.height ?? TVGlassButtonStyle.height, height: style.height ?? TVGlassButtonStyle.height)
            default:
                configuration.label
                    .padding(.horizontal, 34)
                    .padding(.vertical, style.height == nil ? 16 : 0)
                    .frame(height: style.height)
            }
        }
    }
}

extension View {
    /// tvOS: a Liquid Glass button (`TVGlassButtonStyle`).
    func tvGlassButton(_ shape: TVGlassButtonStyle.Shape = .capsule, prominent: Bool = false,
                       height: CGFloat? = TVGlassButtonStyle.height, selected: Bool = false, bare: Bool = false) -> some View {
        buttonStyle(TVGlassButtonStyle(shape: shape, prominent: prominent, height: height, selected: selected, bare: bare))
    }
}

/// Rows and text blocks on Apple TV (list rows, a page's overview): no plate at rest; focused,
/// the row sits on brightened glass and lifts — not the system's white platter. The plate's
/// padding hangs outside the label (`outset`), so text stays on its column's edge.
struct TVGlassRowStyle: ButtonStyle {
    var cornerRadius: CGFloat = 24
    var outset = CGSize(width: 24, height: 18)
    var scale: CGFloat = 1.04

    func makeBody(configuration: Configuration) -> some View { GlassBody(configuration: configuration, style: self) }
    struct GlassBody: View {
        let configuration: Configuration
        let style: TVGlassRowStyle
        @Environment(\.isFocused) private var focused
        var body: some View {
            let shape = RoundedRectangle(cornerRadius: style.cornerRadius, style: .continuous)
            configuration.label
                .foregroundStyle(.white)
                .padding(.horizontal, style.outset.width).padding(.vertical, style.outset.height)
                .background { if focused { Color.clear.glassEffect(.regular.tint(.white.opacity(0.3)), in: shape) } }
                .contentShape(shape)
                .scaleEffect(configuration.isPressed ? 1.0 : focused ? style.scale : 1)
                .shadow(color: .black.opacity(focused ? 0.4 : 0), radius: 18, y: 10)
                .animation(.easeOut(duration: 0.18), value: focused)
                .padding(.horizontal, -style.outset.width).padding(.vertical, -style.outset.height)
        }
    }
}

/// Focus look for custom buttons on Apple TV: grow and cast a shadow, Apple TV app style.
struct TVFocusStyle: ButtonStyle {
    var scale: CGFloat = 1.08
    func makeBody(configuration: Configuration) -> some View {
        FocusLift(configuration: configuration, scale: scale)
    }
    struct FocusLift: View {
        let configuration: Configuration
        let scale: CGFloat
        @Environment(\.isFocused) private var focused
        var body: some View {
            configuration.label
                .scaleEffect(configuration.isPressed ? scale * 0.97 : focused ? scale : 1)
                .shadow(color: .black.opacity(focused ? 0.5 : 0), radius: 18, y: 10)
                .brightness(focused ? 0.04 : 0)
                .animation(.easeOut(duration: 0.18), value: focused)
                .animation(.easeOut(duration: 0.1), value: configuration.isPressed)
        }
    }
}
#endif


/// Text field used in forms. On tvOS the system field shows a blank white platter when focused
/// (its text can't be recoloured), so there the real field is kept nearly invisible — it still
/// takes focus and opens the keyboard — and the value is drawn on top in our own capsule.
struct MoonriseTextField: View {
    let title: String
    @Binding var text: String
    var prompt: String? = nil
    var secure = false
    #if os(tvOS)
    @FocusState private var focused: Bool
    #endif

    var body: some View {
        #if os(tvOS)
        ZStack(alignment: .leading) {
            field
                .textFieldStyle(.plain)
                .foregroundStyle(.clear)
                .focused($focused)
                .opacity(0.011)
            Text(text.isEmpty ? (prompt ?? title) : (secure ? String(repeating: "•", count: text.count) : text))
                .foregroundStyle(text.isEmpty ? Color.white.opacity(focused ? 0.6 : 0.45) : .white)
                .lineLimit(1)
                .allowsHitTesting(false)
        }
        .padding(.horizontal, 24)
        // As tall as a Settings row or a bordered button (66 pt), and as wide as a row.
        .frame(height: 66)
        // Glass like the buttons: clear at rest, brighter and lifted when focused.
        .glassEffect(.regular.tint(.white.opacity(focused ? 0.35 : 0)), in: .capsule)
        .scaleEffect(focused ? 1.02 : 1)
        .shadow(color: .black.opacity(focused ? 0.4 : 0), radius: 16, y: 8)
        .animation(.easeOut(duration: 0.15), value: focused)
        .listRowBackground(Color.clear) // the capsule is the row
        .listRowInsets(EdgeInsets())
        #else
        field
        #endif
    }

    @ViewBuilder private var field: some View {
        if secure {
            SecureField(title, text: $text)
        } else if let prompt {
            TextField(title, text: $text, prompt: Text(prompt))
        } else {
            TextField(title, text: $text)
        }
    }
}

#if os(macOS)
struct HoverHighlight: ViewModifier {
    let cornerRadius: CGFloat
    @State private var hovering = false
    func body(content: Content) -> some View {
        content
            .brightness(hovering ? 0.06 : 0)
            .overlay {
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .strokeBorder(Color.white.opacity(hovering ? 0.55 : 0), lineWidth: 2)
            }
            .animation(.easeOut(duration: 0.15), value: hovering)
            .onHover { hovering = $0 }
    }
}
#endif
