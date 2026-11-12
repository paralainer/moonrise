import Foundation

/// When a stream playing from the server moves to another route. The stream holds the address of
/// the route it was opened on, so nothing tells it the route changed.
enum StreamRouteRules {
    /// A route that has only just changed may change back (networks settle in bursts).
    static let routeSettleTime: TimeInterval = 1
    /// At most one move in this long, so a flapping network doesn't restart the stream repeatedly.
    static let minimumBetweenMoves: TimeInterval = 5
    /// How long a tailnet stream may be stalled before it moves. Off the tailnet it goes on working
    /// while it has a path, so a healthy one isn't interrupted just because home came back.
    static let stallBeforeLeavingTailnet: TimeInterval = 2
    /// How long a stall lasts before the router is asked to look for a route: no route-change
    /// event comes when a path hangs with the network still "up".
    static let stallBeforeAskingForRoute: TimeInterval = 3
    static let minimumBetweenAsks: TimeInterval = 5

    /// Move the stream now? `from` is the route it was opened on, `to` the router's current one.
    static func shouldMove(from: ConnectionRouter.Route?, to: ConnectionRouter.Route?, stalledFor: TimeInterval,
                           routeSettledFor: TimeInterval, sinceLastMove: TimeInterval) -> Bool {
        guard let from, let to, from != to, routeUsable(to), routeSettledFor >= routeSettleTime,
              sinceLastMove >= minimumBetweenMoves else { return false }
        // Off the home network its address can't work any more: don't wait for the buffer to run out.
        if from == .lan { return true }
        // Otherwise (back home, another tailnet address) only when the stream is in trouble.
        return stalledFor >= stallBeforeLeavingTailnet
    }

    static func shouldAskForRoute(stalledFor: TimeInterval, sinceLastAsked: TimeInterval) -> Bool {
        stalledFor >= stallBeforeAskingForRoute && sinceLastAsked >= minimumBetweenAsks
    }

    private static func routeUsable(_ route: ConnectionRouter.Route) -> Bool {
        route == .lan || { if case .tailnet = route { return true } else { return false } }()
    }
}
