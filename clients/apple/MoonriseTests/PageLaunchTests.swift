// The title page's Play animation state (PageDrag in Moonrise/UI/InlinePageGestures.swift): once,
// right after autoplay the page sometimes had no title, or still showed the Play row.
import CoreGraphics
import Observation
import Testing
@testable import Moonrise

/// Reads `value` the way SwiftUI did when that broke: when told of a change, at once (before the new value
/// is stored, as it does when another update is waiting), then watches again. `seen` is what a
/// view drawn from it last showed.
@MainActor
private final class EagerReader<Value> {
    private let value: () -> Value
    private(set) var seen: Value

    init(_ value: @escaping () -> Value) {
        self.value = value
        seen = value()
        watch()
    }

    private func watch() {
        withObservationTracking { _ = value() } onChange: { [self] in
            MainActor.assumeIsolated {
                seen = value()
                watch()
            }
        }
    }
}

@MainActor
@Suite struct PageLaunchTests {
    /// Play from the page: the header shrinks away (`launch` 1 → 0). A view that reads `launch`
    /// on being told of the change ends up with the final value, not the one before it: the title
    /// written out is there (1 - launch) and the Play row is gone (launch).
    @Test func viewsEndWithTheFinishedLaunch() {
        let drag = PageDrag()
        let launch = EagerReader { drag.launch }
        let opacity = EagerReader { drag.videoOpacity }
        let flight = EagerReader { drag.flight }
        drag.prepareLaunch(card: nil, still: nil, fromHeader: true)
        #expect(launch.seen == 1)
        #expect(opacity.seen == 0)
        drag.runLaunch()
        #expect(drag.launch == 0)
        #expect(launch.seen == 0)
        #expect(opacity.seen == 1)
        #expect(flight.seen == 0)
    }

    /// From an episode's card while playing (another episode): the video grows out of the card.
    @Test func anotherEpisodeFromItsCard() {
        let drag = PageDrag()
        let launch = EagerReader { drag.launch }
        let flight = EagerReader { drag.flight }
        drag.prepareLaunch(card: CGRect(x: 0, y: 400, width: 250, height: 140), still: nil, fromHeader: false)
        #expect(flight.seen == 1)
        drag.runLaunch()
        #expect(launch.seen == 0)
        #expect(flight.seen == 0)
    }

    /// The video dragged down and let go short: everything springs back to playing.
    @Test func aDragDownLetGoShortComesBack() {
        let drag = PageDrag()
        let launch = EagerReader { drag.launch }
        let opacity = EagerReader { drag.videoOpacity }
        drag.videoDragged(80, id: nil)
        #expect(drag.launch > 0) // (mid-drag, the next move redraws: not checked)
        drag.videoReleased(stop: false) {}
        #expect(launch.seen == 0)
        #expect(opacity.seen == 1)
    }

    /// The details panel sent away and brought back.
    @Test func thePanelAwayAndBack() {
        let drag = PageDrag()
        drag.resize(500)
        let collapsed = EagerReader { drag.collapsed }
        let panel = EagerReader { drag.panel }
        drag.setCollapsed(true)
        #expect(collapsed.seen == true)
        #expect(panel.seen == 500)
        drag.setCollapsed(false)
        #expect(collapsed.seen == false)
        #expect(panel.seen == 0)
    }
}
