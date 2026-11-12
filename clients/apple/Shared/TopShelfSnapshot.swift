import Foundation
import Security

/// What the Apple TV's Top Shelf shows of this viewer's watching, written by the app and read by
/// the Top Shelf extension (`MoonriseTopShelf`). It's a small keychain item in an access group the
/// two share (`MoonriseKeychainGroup` in their Info.plist), so the extension needs no token: it
/// only reads, and the cards' artwork comes from the server's image URLs (which need none).
/// Signed out (or in the demo) there's no snapshot, and the system shows the static Top Shelf image.
struct TopShelfSnapshot: Codable, Equatable {
    /// Bump when the format changes incompatibly; the extension ignores other versions.
    static let currentVersion = 1

    struct Section: Codable, Equatable {
        var title: String
        var items: [Item]
    }

    struct Item: Codable, Equatable {
        /// The server's item id: the deep links open or play it.
        var id: String
        var title: String
        /// 0…1 for a title part-way through; nil for one not started (a next episode or film).
        var progress: Double?
        /// Wide artwork on the server (its LAN address).
        var image: URL?
    }

    var version = currentVersion
    var sections: [Section]
}

/// Where the snapshot is kept, and the deep links its cards open.
enum TopShelfShared {
    /// Cards link to `moonrise://open/<item id>` (its page) and `moonrise://play/<item id>`
    /// (play or resume it, its page underneath). Registered in the app's Info.plist; the Dev app
    /// (the scripts' DEV=1 builds) uses `moonrise-dev`, so its cards open it and not the other app.
    static let scheme = (Bundle.main.object(forInfoDictionaryKey: "MoonriseURLScheme") as? String)
        .flatMap { $0.isEmpty ? nil : $0.lowercased() } ?? "moonrise"

    enum Link: Equatable {
        case open(String)
        case play(String)

        var url: URL {
            switch self {
            case let .open(id): URL(string: "\(scheme)://open/\(Self.escape(id))")!
            case let .play(id): URL(string: "\(scheme)://play/\(Self.escape(id))")!
            }
        }

        init?(_ url: URL) {
            guard url.scheme?.lowercased() == scheme, let host = url.host?.lowercased() else { return nil }
            let id = url.path.trimmingCharacters(in: CharacterSet(charactersIn: "/")).removingPercentEncoding ?? ""
            guard !id.isEmpty else { return nil }
            switch host {
            case "open": self = .open(id)
            case "play": self = .play(id)
            default: return nil
            }
        }

        private static func escape(_ id: String) -> String {
            id.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed.subtracting(CharacterSet(charactersIn: "/"))) ?? id
        }
    }

    /// The keychain access group the app and the extension share (tvOS builds only; nil elsewhere).
    static var keychainGroup: String? {
        (Bundle.main.object(forInfoDictionaryKey: "MoonriseKeychainGroup") as? String).flatMap { $0.isEmpty ? nil : $0 }
    }

    private static let service = "moonrise.topshelf"
    private static let account = "snapshot"

    private static func query(_ group: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrAccessGroup as String: group,
        ]
    }

    static func load() -> TopShelfSnapshot? {
        guard let group = keychainGroup else { return nil }
        var q = query(group)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: AnyObject?
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess, let data = out as? Data,
              let snapshot = try? JSONDecoder().decode(TopShelfSnapshot.self, from: data),
              snapshot.version == TopShelfSnapshot.currentVersion else { return nil }
        return snapshot
    }

    /// Saves the snapshot, or deletes it (nil). False when the keychain refused.
    @discardableResult
    static func save(_ snapshot: TopShelfSnapshot?) -> Bool {
        guard let group = keychainGroup else { return false }
        let q = query(group)
        guard let snapshot else {
            let status = SecItemDelete(q as CFDictionary)
            return status == errSecSuccess || status == errSecItemNotFound
        }
        guard let data = try? JSONEncoder().encode(snapshot) else { return false }
        let status = SecItemUpdate(q as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecSuccess { return true }
        guard status == errSecItemNotFound else { return false }
        var add = q
        add[kSecValueData as String] = data
        // The system may ask the extension for the shelf at any time after the first unlock.
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        return SecItemAdd(add as CFDictionary, nil) == errSecSuccess
    }
}
