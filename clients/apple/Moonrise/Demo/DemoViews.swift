import SwiftUI

/// The welcome screen's way into the demo.
struct TryDemoButton: View {
    var body: some View {
        #if os(tvOS)
        Button { SourceStore.shared.enterDemo() } label: {
            Label("Try the Demo", systemImage: "play.rectangle.on.rectangle").frame(maxWidth: .infinity)
        }
        .tvGlassButton()
        #else
        VStack(spacing: 8) {
            Button { SourceStore.shared.enterDemo() } label: {
                Label("Try the Demo", systemImage: "play.rectangle.on.rectangle")
                    .padding(.horizontal, 24).padding(.vertical, 12)
            }
            .plainButton()
            .glassEffect(.regular.interactive(), in: .capsule)
            Text("No server yet? Browse and play four open movies from the Blender Foundation.")
                .font(.footnote).foregroundStyle(Theme.secondaryText)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 40)
        }
        #endif
    }
}

/// Home's note while the demo runs (iPhone, iPad, Mac): what this is, and the way out.
struct DemoBanner: View {
    var body: some View {
        HStack(alignment: .center, spacing: 14) {
            Image(systemName: "play.rectangle.on.rectangle").font(.title2).foregroundStyle(.white)
            VStack(alignment: .leading, spacing: 3) {
                Text("Demo").font(.headline)
                Text("Four open movies by the Blender Foundation (CC BY), streamed from their public hosts.")
                    .font(.footnote).foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 8)
            Button("Exit Demo") { SourceStore.shared.exitDemo() }
                .font(.subheadline.weight(.semibold))
                .padding(.horizontal, 14).padding(.vertical, 8)
                .plainButton()
                .glassEffect(.regular.interactive(), in: .capsule)
        }
        .padding(16)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
    }
}

/// A small "DEMO" mark (Apple TV: on every tab, beside the tab bar).
struct DemoBadge: View {
    var body: some View {
        Text("DEMO")
            .font(.caption.weight(.heavy)).tracking(2)
            .foregroundStyle(.black)
            .padding(.horizontal, 16).padding(.vertical, 6)
            .background(.yellow, in: Capsule())
            .accessibilityLabel("Demo mode")
    }
}

/// Settings' Server section while the demo runs.
struct DemoServerSection: View {
    var body: some View {
        Section {
            LabeledContent("Library", value: "Demo · 4 open movies")
            LabeledContent("Streams", value: "Their public hosts")
            Button { SourceStore.shared.exitDemo() } label: {
                Label("Exit Demo", systemImage: "rectangle.portrait.and.arrow.right").formRow()
            }
            .formRowButton()
        } header: { Text("Demo") } footer: {
            Text("Big Buck Bunny, Elephants Dream, Sintel and Tears of Steel by the Blender Foundation, under Creative Commons licences (CC BY). Exit to connect to your own Moonrise server or set one up.")
        }
    }
}
