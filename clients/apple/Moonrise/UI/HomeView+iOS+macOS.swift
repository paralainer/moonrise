#if !os(tvOS)
import SwiftUI

/// The Settings gear at the top trailing corner and its sheet. Home always has it; on iPad (regular
/// width) every tab's first screen does (`onlyOnPad`), in the same place.
private struct SettingsButton: ViewModifier {
    let onlyOnPad: Bool
    @Environment(\.metrics) private var metrics
    @State private var showSettings = false

    func body(content: Content) -> some View {
        if onlyOnPad && !metrics.regular {
            content
        } else {
            content
                .toolbar {
                    ToolbarItem(placement: .primaryAction) {
                        Button { showSettings = true } label: { Image(systemName: "gearshape") }
                            .accessibilityLabel("Settings")
                    }
                }
                .sheet(isPresented: $showSettings) {
                    NavigationStack {
                        SettingsView()
                            .toolbar {
                                ToolbarItem(placement: .confirmationAction) { Button("Done") { showSettings = false } }
                            }
                    }
                    #if os(macOS)
                    .frame(minWidth: 520, minHeight: 600)
                    #endif
                }
                #if DEBUG
                .task {
                    // Test seam: MOONRISE_MANAGE opens Settings (then Manage Server).
                    if ManageSeam.opensSettings, !onlyOnPad, !ManageSeam.consumed {
                        try? await Task.sleep(for: .seconds(1.5))
                        showSettings = true
                    }
                }
                #endif
        }
    }
}

extension View {
    func settingsButton(onlyOnPad: Bool = false) -> some View { modifier(SettingsButton(onlyOnPad: onlyOnPad)) }
}
#endif
