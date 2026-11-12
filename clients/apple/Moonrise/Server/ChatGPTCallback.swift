import Foundation

/// The request the browser brings to the app's loopback listener at the end of Sign in with
/// ChatGPT (`GET /auth/callback?code=…&scope=…&state=…&client_id=…`), read exactly as OpenAI's
/// reference client reads it (`new URL(…).searchParams`, i.e. `application/x-www-form-urlencoded`):
/// the query is split on `&`, each pair at its first `=`, a `+` is a space and `%XX` is a byte;
/// a `%` not followed by two hex digits is kept as it is. A value is decoded once, never twice,
/// and nothing else is changed: a code of any length, with `+`, `/`, `=`, `%`, `.`, `-`, `_`, `~`,
/// comes out as OpenAI encoded it. No `URLComponents`: it refuses some targets outright and its
/// `queryItems` leaves `+` undecoded.
struct ChatGPTCallback: Equatable {
    struct Param: Equatable {
        let name: String
        let value: String
    }

    /// The method of the request line (`GET`).
    let method: String
    /// The path, without the query (`/auth/callback`).
    let path: String
    /// The query's parameters, in order, repeats included.
    let params: [Param]

    func value(_ name: String) -> String? { params.first { $0.name == name }?.value }
    func count(_ name: String) -> Int { params.filter { $0.name == name }.count }
    /// The parameter names, in order (for the server's log: never the values).
    var names: [String] { params.map(\.name) }

    /// From a whole request head (`GET <target> HTTP/1.1\r\n…`); nil if it has no request line.
    init?(requestHead: Data) {
        let text = String(decoding: requestHead, as: UTF8.self)
        let line = text.components(separatedBy: "\r\n").first ?? ""
        let parts = line.split(separator: " ", omittingEmptySubsequences: true)
        guard parts.count >= 2 else { return nil }
        self.init(method: String(parts[0]), target: String(parts[1]))
    }

    /// From a request target (`/auth/callback?…`). A fragment is never sent by browsers; one
    /// here is ignored.
    init(method: String = "GET", target: String) {
        self.method = method
        var t = Substring(target)
        if let hash = t.firstIndex(of: "#") { t = t[..<hash] }
        let query: Substring
        if let q = t.firstIndex(of: "?") {
            path = String(t[..<q])
            query = t[t.index(after: q)...]
        } else {
            path = String(t)
            query = ""
        }
        params = query.split(separator: "&", omittingEmptySubsequences: true).map { pair in
            if let eq = pair.firstIndex(of: "=") {
                return Param(name: Self.formDecode(pair[..<eq]), value: Self.formDecode(pair[pair.index(after: eq)...]))
            }
            return Param(name: Self.formDecode(pair), value: "")
        }
    }

    /// `application/x-www-form-urlencoded` decoding of one name or value, as `URLSearchParams`.
    static func formDecode(_ s: Substring) -> String {
        let bytes = Array(s.utf8)
        var out: [UInt8] = []
        out.reserveCapacity(bytes.count)
        var i = 0
        while i < bytes.count {
            let b = bytes[i]
            if b == UInt8(ascii: "+") {
                out.append(UInt8(ascii: " "))
            } else if b == UInt8(ascii: "%"), i + 2 < bytes.count, let hi = hex(bytes[i + 1]), let lo = hex(bytes[i + 2]) {
                out.append(hi << 4 | lo)
                i += 2
            } else {
                out.append(b)
            }
            i += 1
        }
        return String(decoding: out, as: UTF8.self)
    }

    private static func hex(_ c: UInt8) -> UInt8? {
        switch c {
        case UInt8(ascii: "0")...UInt8(ascii: "9"): c - UInt8(ascii: "0")
        case UInt8(ascii: "a")...UInt8(ascii: "f"): c - UInt8(ascii: "a") + 10
        case UInt8(ascii: "A")...UInt8(ascii: "F"): c - UInt8(ascii: "A") + 10
        default: nil
        }
    }
}
