// When a playing stream moves to another route: leaving home at once, coming home only when
// it is in trouble, never on a route that hasn't settled, and not repeatedly on a flapping network.
import Foundation
import Testing
@testable import Moonrise

struct StreamRouteRulesTests {
    private let tailnet = ConnectionRouter.Route.tailnet(host: "100.64.0.1")

    private func move(_ from: ConnectionRouter.Route?, _ to: ConnectionRouter.Route?, stalled: Double = 0, settled: Double = 10, sinceMove: Double = 60) -> Bool {
        StreamRouteRules.shouldMove(from: from, to: to, stalledFor: stalled, routeSettledFor: settled, sinceLastMove: sinceMove)
    }

    @Test func leavingHomeMovesBeforeTheBufferRunsOut() {
        #expect(move(.lan, tailnet))
    }

    @Test func comingHomeLeavesAHealthyStreamAlone() {
        #expect(!move(tailnet, .lan))
        #expect(!move(tailnet, .lan, stalled: 1))
        #expect(move(tailnet, .lan, stalled: 2))
    }

    @Test func noMoveToARouteThatDoesNotWorkYet() {
        #expect(!move(.lan, .unreachable))
        #expect(!move(.lan, .checking))
        #expect(!move(.lan, nil))
        #expect(!move(nil, tailnet))
    }

    @Test func theSameRouteIsNoMove() {
        #expect(!move(.lan, .lan, stalled: 30))
        #expect(!move(tailnet, tailnet, stalled: 30))
    }

    @Test func anotherTailnetAddressMovesOnlyWhenInTrouble() {
        let other = ConnectionRouter.Route.tailnet(host: "100.64.0.2")
        #expect(!move(tailnet, other))
        #expect(move(tailnet, other, stalled: 5))
    }

    @Test func aFlappingNetworkDoesNotRestartTheStream() {
        #expect(!move(.lan, tailnet, settled: 0.2))
        #expect(!move(.lan, tailnet, sinceMove: 2))
    }

    @Test func aStallWithNoEventAsksTheRouterSometimes() {
        #expect(!StreamRouteRules.shouldAskForRoute(stalledFor: 1, sinceLastAsked: 60))
        #expect(StreamRouteRules.shouldAskForRoute(stalledFor: 3, sinceLastAsked: 60))
        #expect(!StreamRouteRules.shouldAskForRoute(stalledFor: 10, sinceLastAsked: 2))
    }
}
