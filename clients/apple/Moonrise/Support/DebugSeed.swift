#if DEBUG
import Foundation

/// Test seam for headless UI checks: `DebugSeams.signIn` (MOONRISE_URL, MOONRISE_USER, MOONRISE_PASS)
/// signs in to a Moonrise server without typing. Debug builds only.
enum DebugSeed {
    @MainActor
    static func run() async {
        if let seed = DebugSeams.launch.signIn,
           SourceStore.shared.server == nil, let url = MoonriseClient.normalize(seed.address) {
            let c = MoonriseClient(sourceId: nil, baseURL: url, token: nil)
            do {
                let info = try await c.info()
                let auth = try await c.login(username: seed.user, password: seed.pass)
                SourceStore.shared.signIn(SignInView.source(info: info, url: url, auth: auth))
            } catch {
                fputs("DebugSeed: Moonrise sign-in failed: \(error.localizedDescription)\n", stderr)
            }
        }
    }
}
#endif
