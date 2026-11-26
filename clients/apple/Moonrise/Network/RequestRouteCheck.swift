import Foundation

/// A request to the server failed: should the router look for a route again? It only looks by
/// itself on a network change, on coming back to the app, and while the server is unreachable, so
/// an address that stops answering with the device's network unchanged (the server's LAN address
/// gone quiet, a Wi-Fi that stays joined but passes nothing) would otherwise be kept until the app
/// is reopened.
enum RequestRouteCheck {
    /// At most one check in this long: a screen fires several requests at once, and each fails.
    static let minimumBetweenChecks: TimeInterval = 5

    /// - route: the router's current route.
    /// - sinceLastCheck: since a failed request last asked.
    static func shouldCheck(_ error: Error, route: ConnectionRouter.Route?, sinceLastCheck: TimeInterval) -> Bool {
        // The server answered (an HTTP error) or the request was abandoned: the route works.
        guard error.isConnectivity else { return false }
        // Unreachable: the router already tries again by itself. Checking, or no server: nothing to redo.
        switch route {
        case .lan?, .tailnet?: return sinceLastCheck >= minimumBetweenChecks
        default: return false
        }
    }
}
