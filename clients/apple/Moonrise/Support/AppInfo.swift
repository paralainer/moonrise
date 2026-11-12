import Foundation
#if os(iOS)
import UIKit
#endif

/// The app as it introduces itself to the server (sign-in, device list).
enum AppInfo {
    /// The app's bundle id (Config/Signing.xcconfig: each build's own). Keychain items,
    /// background tasks and the download session are named after it.
    static let bundleId = Bundle.main.bundleIdentifier ?? "org.example.moonrise"

    static let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.1"

    /// The commit the app was built from (git short sha, `-dirty` with uncommitted changes), set by
    /// scripts/build.sh; empty in builds made elsewhere.
    static let buildStamp = Bundle.main.object(forInfoDictionaryKey: "MoonriseBuildStamp") as? String ?? ""

    /// The version as Settings shows it: with the build stamp when there is one, so it's clear
    /// which build a device has.
    static var versionLabel: String { buildStamp.isEmpty ? version : "\(version) (\(buildStamp))" }

    static let deviceId: String = {
        if let id = UserDefaults.standard.string(forKey: "deviceId") { return id }
        let id = UUID().uuidString
        UserDefaults.standard.set(id, forKey: "deviceId")
        return id
    }()

    static var deviceName: String {
        #if os(iOS)
        if ProcessInfo.processInfo.isiOSAppOnMac { return "Mac" }
        return UIDevice.current.userInterfaceIdiom == .pad ? "iPad" : "iPhone"
        #elseif os(tvOS)
        return "Apple TV"
        #else
        return Host.current().localizedName ?? "Mac"
        #endif
    }
}
