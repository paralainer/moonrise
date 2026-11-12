#if os(tvOS)
import SwiftUI

extension HeroCarousel {
    /// Apple TV: one focusable banner. Left/right switch titles, select opens the page.
    @ViewBuilder var body: some View {
        let current = items.first { $0.id == selection } ?? items[0]
        VStack(spacing: 14) {
            NavigationLink(value: Route.item(current)) {
                HeroPage(client: client, item: current, focused: heroFocused)
                    .frame(maxHeight: .infinity, alignment: .top)
                    .id(current.id)
                    .transition(.opacity)
            }
            .plainButton(scale: 1)
            .focused($heroFocused)
            .onMoveCommand { dir in
                guard let i = items.firstIndex(where: { $0.id == current.id }) else { return }
                switch dir {
                case .left: withAnimation { selection = items[(i - 1 + items.count) % items.count].id }
                case .right: withAnimation { selection = items[(i + 1) % items.count].id }
                default: break
                }
            }
            .frame(height: TitleHeader.height + 18 + Self.metaHeight)
            dots
        }
        .onAppear { if selection == nil || !items.contains(where: { $0.id == selection }) { selection = items.first?.id } }
        .task(id: active && !heroFocused) {
            while active, !heroFocused, !Task.isCancelled {
                try? await Task.sleep(for: .seconds(7))
                guard active, !heroFocused, !Task.isCancelled, !items.isEmpty else { return }
                let i = items.firstIndex { $0.id == selection } ?? -1
                withAnimation { selection = items[(i + 1) % items.count].id }
            }
        }
    }
}
#endif
