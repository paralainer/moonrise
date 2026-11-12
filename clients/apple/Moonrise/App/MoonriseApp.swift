import SwiftUI

@main
struct MoonriseApp: App {
    #if os(iOS)
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    #endif

    /// Hosting the unit tests (MoonriseTests): the app starts nothing (no downloads, no server
    /// connection, no discovery) and shows an empty window, so the tests run offline.
    static let isUnitTesting = ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil
        || ProcessInfo.processInfo.environment["XCTestBundlePath"] != nil

    init() {
        if Self.isUnitTesting { return }
        URLCache.shared = URLCache(memoryCapacity: 64 << 20, diskCapacity: 512 << 20)
        _ = DownloadManager.shared
        Task { @MainActor in
            _ = TailscaleService.shared        // starts the embedded node if enabled
            ConnectionRouter.shared.start()     // picks LAN vs tailnet per server
            // Signed out (or the token was dropped): the Apple TV's Top Shelf shows nothing of before.
            if SourceStore.shared.server?.kind != .moonrise { TopShelfStore.clear() }
        }
    }

    var body: some Scene {
        WindowGroup {
            if Self.isUnitTesting {
                Color.clear
            } else {
                RootView()
                    #if !os(tvOS)
                    // (tvOS: forcing dark breaks focused text fields — white text on the white platter.
                    // Apple TV runs dark by default.)
                    .preferredColorScheme(.dark)
                    // tvOS draws focused controls on a white platter in the tint colour: white would
                    // make focused text fields unreadable, so it keeps the system tint.
                    .tint(Theme.accent)
                    #endif
            }
        }
        #if os(macOS)
        .defaultSize(width: 1280, height: 800)
        .commands { MoonriseCommands() }
        #endif
        #if os(macOS)
        MacSettingsScene()
        #endif
    }
}

#if os(iOS)
import UIKit

final class AppDelegate: NSObject, UIApplicationDelegate {
    nonisolated(unsafe) static var orientationLock: UIInterfaceOrientationMask = .allButUpsideDown

    func application(_ application: UIApplication, supportedInterfaceOrientationsFor window: UIWindow?) -> UIInterfaceOrientationMask {
        UIDevice.current.userInterfaceIdiom == .pad ? .all : Self.orientationLock
    }

    /// Relaunched in the background because a background URLSession finished work. Hosting the
    /// unit tests, nothing is started (MoonriseApp.isUnitTesting): no download manager.
    func application(_ application: UIApplication, handleEventsForBackgroundURLSession identifier: String,
                     completionHandler: @escaping () -> Void) {
        if MoonriseApp.isUnitTesting { completionHandler(); return }
        MainActor.assumeIsolated {
            DownloadManager.shared.backgroundCompletionHandlers[identifier] = completionHandler
        }
    }
}
#endif
