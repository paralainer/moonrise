// When a failed request to the server makes the router look for a route again: only when the
// request couldn't get through on a route believed to work, and not once per request of a burst.
import Foundation
import Testing
@testable import Moonrise

struct RequestRouteCheckTests {
    private let tailnet = ConnectionRouter.Route.tailnet(host: "100.64.0.1")
    private let lost = URLError(.networkConnectionLost)

    private func check(_ error: Error, _ route: ConnectionRouter.Route?, since: Double = 60) -> Bool {
        RequestRouteCheck.shouldCheck(error, route: route, sinceLastCheck: since)
    }

    @Test func aRequestThatCannotGetThroughChecksAgain() {
        #expect(check(lost, .lan))
        #expect(check(URLError(.cannotConnectToHost), .lan))
        #expect(check(URLError(.timedOut), .lan))
        #expect(check(lost, tailnet))
    }

    @Test func theFirstFailureChecks() {
        #expect(check(lost, .lan, since: Date().timeIntervalSince(.distantPast)))
    }

    @Test func aBurstOfFailuresChecksOnce() {
        #expect(!check(lost, .lan, since: 0))
        #expect(!check(lost, .lan, since: RequestRouteCheck.minimumBetweenChecks - 0.1))
        #expect(check(lost, .lan, since: RequestRouteCheck.minimumBetweenChecks))
    }

    @Test func anAnswerFromTheServerIsNoReasonToCheck() {
        #expect(!check(MoonriseClientError.http(500, ""), .lan))
        #expect(!check(MoonriseClientError.http(401, ""), .lan))
        #expect(!check(DecodingError.dataCorrupted(.init(codingPath: [], debugDescription: "")), .lan))
    }

    @Test func anAbandonedRequestIsNoReasonToCheck() {
        #expect(!check(URLError(.cancelled), .lan))
        #expect(!check(CancellationError(), .lan))
    }

    @Test func noCheckWhenTheRouterIsAlreadyOnIt() {
        // Unreachable: it retries by itself. Checking, or nothing to route: nothing to start.
        #expect(!check(lost, .unreachable))
        #expect(!check(lost, .checking))
        #expect(!check(lost, nil))
    }
}
