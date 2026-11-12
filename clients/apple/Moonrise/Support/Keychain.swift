import Foundation
import Security

/// Minimal generic-password Keychain wrapper keyed by account string.
enum Keychain {
    private static let service = AppInfo.bundleId

    static func set(_ value: String?, for account: String) {
        let base: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(base as CFDictionary)
        UserDefaults.standard.removeObject(forKey: fallbackKey(account))
        guard let value, let data = value.data(using: .utf8) else { return }
        var add = base
        add[kSecValueData as String] = data
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        if SecItemAdd(add as CFDictionary, nil) == errSecMissingEntitlement {
            // Unsigned simulator builds have no keychain access.
            UserDefaults.standard.set(value, forKey: fallbackKey(account))
        }
    }

    private static func fallbackKey(_ account: String) -> String { "kc-fallback." + account }

    static func get(_ account: String) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var out: AnyObject?
        guard SecItemCopyMatching(query as CFDictionary, &out) == errSecSuccess,
              let data = out as? Data else { return UserDefaults.standard.string(forKey: fallbackKey(account)) }
        return String(data: data, encoding: .utf8)
    }
}
