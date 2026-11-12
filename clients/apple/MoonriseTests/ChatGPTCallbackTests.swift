// How the app reads the Sign in with ChatGPT callback (Moonrise/Server/ChatGPTCallback.swift),
// against what OpenAI's reference client gets from `URLSearchParams`. The codes are made
// up: long, JWT-shaped, with every character a URL can carry. Moved here from scripts/tests/chatgpt-callback.
import Foundation
import Testing
@testable import Moonrise

private func check(_ ok: Bool, _ what: String, sourceLocation: SourceLocation = #_sourceLocation) {
    #expect(ok, Comment(rawValue: what), sourceLocation: sourceLocation)
}

/// Form-encodes as `URLSearchParams.toString()` does (space as `+`; `*-._` and alphanumerics kept).
private func formEncode(_ s: String) -> String {
    var out = ""
    for b in s.utf8 {
        switch b {
        case UInt8(ascii: " "): out += "+"
        case UInt8(ascii: "a")...UInt8(ascii: "z"), UInt8(ascii: "A")...UInt8(ascii: "Z"), UInt8(ascii: "0")...UInt8(ascii: "9"),
             UInt8(ascii: "*"), UInt8(ascii: "-"), UInt8(ascii: "."), UInt8(ascii: "_"):
            out.append(Character(UnicodeScalar(b)))
        default: out += String(format: "%%%02X", b)
        }
    }
    return out
}

private let header = Data(#"{"alg":"RS256","kid":"k1","typ":"JWT"}"#.utf8).base64EncodedString()
    .replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
private let codes = [
    "\(header).\(String(repeating: "aB3-_", count: 300)).\(String(repeating: "sig_-9Z", count: 49))",  // JWT-like, ~1900 chars
    "ac_" + String(repeating: "Zz09-_.~", count: 237),
    String(repeating: "Ab+C9/Ef=", count: 211) + "+/=",  // standard base64 characters
    String(repeating: "a%2Bb%2Fc%3D%25.d-e_f~g h", count: 70),  // a literal %, which must not be decoded twice
    "short",
]
private let state = "Xy_-0123456789abcdefghijklmnopqrstuvwxyzABCDEF"
private let scope = "chatgpt.tokens.use.direct email offline_access openid profile resource.invoke"

@Suite struct ChatGPTCallbackTests {
    /// Long JWT-shaped codes come out exactly as sent, in OpenAI's order or fully %-encoded.
    @Test func codesRoundTrip() {
        for code in codes {
            // OpenAI's order (its sign-in page), each value form-encoded.
            let target = "/auth/callback?code=\(formEncode(code))&scope=\(formEncode(scope))&state=\(state)&client_id=oaiapp_fXrDhCBncVfWaaSG7g8QsW5xS"
            let head = Data("GET \(target) HTTP/1.1\r\nHost: 127.0.0.1:1455\r\nAccept: */*\r\n\r\n".utf8)
            guard let cb = ChatGPTCallback(requestHead: head) else { check(false, "no request line"); continue }
            check(cb.method == "GET" && cb.path == "/auth/callback", "path \(cb.path)")
            check(cb.value("code") == code, "code of \(code.count) chars came out as \(cb.value("code")?.count ?? -1) chars")
            check(cb.value("scope") == scope, "scope")
            check(cb.value("state") == state && cb.count("state") == 1, "state")
            check(cb.value("client_id") == "oaiapp_fXrDhCBncVfWaaSG7g8QsW5xS", "client_id")
            check(cb.names == ["code", "scope", "state", "client_id"], "names \(cb.names)")
            // Percent-encoding every character, as some servers do, reads the same.
            let all = code.utf8.map { String(format: "%%%02X", $0) }.joined()
            check(ChatGPTCallback(target: "/auth/callback?state=\(state)&code=\(all)").value("code") == code, "fully %-encoded code")
        }
    }

    @Test func oddInputReadsAsURLSearchParamsDoes() {
        // What URLSearchParams does with odd input.
        check(ChatGPTCallback(target: "/auth/callback?code=a+b%2Bc%2bd").value("code") == "a b+c+d", "+ and %2B")
        check(ChatGPTCallback(target: "/auth/callback?code=50%&x=%zz%4").value("code") == "50%", "a lone % stays")
        check(ChatGPTCallback(target: "/auth/callback?code=50%&x=%zz%4").value("x") == "%zz%4", "bad escapes stay")
        check(ChatGPTCallback(target: "/auth/callback?code=a=b=c").value("code") == "a=b=c", "split at the first =")
        check(ChatGPTCallback(target: "/auth/callback?&&code=x&&").names == ["code"], "empty pairs skipped")
        check(ChatGPTCallback(target: "/auth/callback?code=x#frag=y").value("code") == "x", "fragment ignored")
        check(ChatGPTCallback(target: "/auth/callback?code=x&code=y").count("code") == 2, "repeats counted")
        check(ChatGPTCallback(target: "/auth/callback?error=access_denied&error_description=User+said+no%21&state=s").value("error_description") == "User said no!", "error description")
        check(ChatGPTCallback(target: "/auth/callback?code={raw|chars}^").value("code") == "{raw|chars}^", "characters URLComponents refuses")
        check(ChatGPTCallback(target: "/auth/callback?code=%E2%9C%93").value("code") == "✓", "UTF-8")
        check(ChatGPTCallback(requestHead: Data("garbage".utf8)) == nil, "no request line")
        check(ChatGPTCallback(target: "/favicon.ico").path == "/favicon.ico", "other paths")
    }
}
