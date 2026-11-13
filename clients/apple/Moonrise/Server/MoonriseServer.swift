import AVFoundation
import Foundation

/// The Moonrise server's playback layer (server/ in this repo). For titles it can serve, it makes
/// HLS that starts at once at any position, carries every audio track as a native rendition
/// (switching doesn't reload the stream; tracks Apple devices can't play, such as DTS, TrueHD,
/// MP3 or Vorbis, are converted to AAC or E-AC-3) and has correctly timed subtitle renditions.
/// Video is copied, or transcoded on its GPU for a lower quality, codecs Apple devices can't
/// play and HDR on displays without it (tone-mapped); picture subtitles come as images the app
/// draws over the video (`subtitles/{index}/cues`). It also
/// extracts text subtitles (kept on the server, so a track is quick after the first play),
/// prepares downloads (the original, or a progressive MP4 AVPlayer plays offline, at any
/// quality) and searches/fetches subtitles on OpenSubtitles.com with its own account.
enum MoonriseServer {
    static let port = 8097

    struct Playback: Decodable {
        let supported: Bool
        let reasons: [String]
        let master: String?
        /// The video as served (the variant played, when the server has variants).
        let video: Video?
        let audio: [Track]
        let subtitles: [Track]
        /// "copy", "convert" (some audio converted) or "transcode" (the video is re-encoded).
        let method: String?
        /// The variant played and the title's ladder. Absent from servers without transcoding.
        let variant: String?
        let variants: [Variant]?
        /// The playback session (its id is in `master`): reports go to `/v1/sessions/{session}/…`.
        /// Absent from servers before accounts (Phase 3).
        let session: String?
        /// The keyframe index is still being made: ask again later.
        let pending: Bool?
        /// The file played (the library's file id).
        let mediaSourceId: String?
        /// Seconds.
        let duration: Double?
        /// The tracks this user picked last in the file, the title or the show (server 0.29.0+,
        /// `track-memory`).
        var tracks: TrackRules.Memory? = nil

        enum CodingKeys: String, CodingKey {
            case supported, reasons, master, video, audio, subtitles, method, variant, variants, session, pending, duration, tracks
            case mediaSourceId = "media_source_id"
        }

        struct Video: Decodable {
            let codec: String
            /// SDR, PQ or HLG.
            let range: String
        }

        struct Track: Decodable {
            /// The server's track index.
            let index: Int
            let name: String
            let language: String?
            /// Served converted (AAC/E-AC-3) rather than copied. Absent from older servers.
            let converted: Bool?
        }

        struct Variant: Decodable {
            let id: String
            let label: String
            let bitrate: Int
            let transcode: Bool
        }

        /// The variant played.
        var served: Variant? { variants?.first { $0.id == variant } }
    }

    /// A subtitle track of a title (`GET /v1/items/{id}/subtitles`, and `subtitles[]` above).
    struct SubtitleTrack: Decodable, Hashable {
        /// The server's track index; tracks fetched from OpenSubtitles have their own (10000+).
        let index: Int
        let name: String
        let language: String?
        let text: Bool
        let external: Bool?
        let forced: Bool?
        /// "embedded", "sidecar" or "opensubtitles".
        let source: String?
        /// Text tracks: the SRT, relative to the server.
        let url: String?
        /// How its timing stands (servers since 0.21).
        var timing: SubtitleTiming?

        var fetched: Bool { source == "opensubtitles" }
    }

    /// A text track's timing: the server's match of a downloaded (fetched or uploaded) track to
    /// this file, and the viewer's nudge, kept per file and track for every device.
    struct SubtitleTiming: Decodable, Hashable {
        /// "none" (a file's own track, or a match not asked for yet), "pending" (running),
        /// "synced", "in_sync", "unsure" or "failed".
        var state: String
        var offset: Double
        var scale: Double
        var pieces: Int
        var nudge: Double
        /// The viewer chose the original timing over the match.
        var original: Bool

        var pending: Bool { state == "pending" }
        var synced: Bool { state == "synced" }

        /// The same for a narrow row (the Apple TV's menus cut their second line short).
        var shortLabel: String? {
            if original { return label }
            switch state {
            case "synced": return scale == 1 ? String(format: "Synced (%+.1f s)", offset) : "Synced (speed)"
            case "in_sync": return "In sync"
            case "unsure", "failed": return "Not synced"
            default: return label
            }
        }

        /// What the menu says about it.
        var label: String? {
            if original { return synced ? "Original timing" : nil }
            switch state {
            case "pending": return "Syncing…"
            case "synced":
                var parts: [String] = []
                if scale != 1 { parts.append("speed adjusted") }
                if pieces > 1 { parts.append("\(pieces) parts") }
                if scale == 1 { parts.insert(String(format: "%+.1f s", offset), at: 0) }
                return "Synced automatically (\(parts.joined(separator: ", ")))"
            case "in_sync": return "In sync with this file"
            case "unsure", "failed": return "Couldn't sync automatically"
            default: return nil
            }
        }
    }

    /// An OpenSubtitles result from the server's search.
    struct SearchResult: Decodable, Identifiable, Hashable {
        let id: String
        let name: String
        let language: String?
        let downloads: Int
        let hashMatch: Bool
        let hearingImpaired: Bool
        let machineTranslated: Bool?
        let aiTranslated: Bool?
        /// Already fetched for this title.
        let fetched: Bool?
    }

    struct SearchAnswer: Decodable {
        let results: [SearchResult]
        let remainingDownloads: Int?
        /// Servers from 0.12.0: which download quota applies and what is left of it.
        let quota: SubtitleQuota?
    }

    /// The OpenSubtitles download quota the server fetches with.
    struct SubtitleQuota: Decodable, Hashable {
        /// "anonymous" (the server's key alone: a few a day), "user" or "vip".
        let tier: String
        let allowed: Int?
        let remaining: Int?
        /// When the count starts again (RFC 3339), when known.
        let resetAt: String?
        /// Downloads a day with a free account.
        let accountAllowed: Int?

        var resetDate: Date? {
            guard let resetAt else { return nil }
            let f = ISO8601DateFormatter()
            f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
            return f.date(from: resetAt) ?? ISO8601DateFormatter().date(from: resetAt)
        }

        var exhausted: Bool { remaining == 0 }

        /// "3 of 5 downloads left today"
        var leftLine: String? {
            guard let remaining else { return nil }
            let n = remaining == 1 ? "1 download" : "\(remaining) downloads"
            let base = allowed.map { "\(remaining) of \($0) downloads left today" } ?? "\(n) left today"
            return tier == "vip" ? "VIP · " + base : base
        }

        /// What to do for more: an account (anonymous), or wait for the reset.
        var hint: String? {
            if tier == "anonymous" { return "Add an OpenSubtitles account for \(accountAllowed ?? 20) a day" }
            return nil
        }

        /// "More at 14:05" / "More tomorrow at 09:00"
        var resetLine: String? {
            guard let d = resetDate, d > Date() else { return nil }
            let time = d.formatted(date: .omitted, time: .shortened)
            return Calendar.current.isDateInToday(d) ? "More at \(time)" : "More tomorrow at \(time)"
        }

        /// One line for the search screen.
        var summary: String? {
            if exhausted {
                return (["No downloads left today", resetLine, hint].compactMap { $0 }).joined(separator: " · ")
            }
            let line = ([leftLine, hint].compactMap { $0 }).joined(separator: " · ")
            return line.isEmpty ? nil : line
        }
    }

    /// A prepared download (a progressive MP4 made on the server).
    struct DownloadJob: Decodable {
        let id: String
        /// "queued", "running", "done", "failed"
        let state: String
        let progress: Double
        let size: Int64
        let expected: Int64
        let label: String
        let method: String
        let error: String?
        /// Where to get it, relative to the server (done jobs).
        let file: String?
    }

    enum ServerError: LocalizedError {
        case http(Int, String)
        var errorDescription: String? {
            switch self {
            case let .http(code, body): body.isEmpty ? "Moonrise server returned \(code)." : String(body.prefix(200))
            }
        }
        var status: Int { if case let .http(c, _) = self { return c }; return 0 }
    }

    /// The decoder for the server's JSON (snake_case keys).
    static let snake: JSONDecoder = {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }()

    private static func call(_ url: URL, method: String = "GET", body: [String: Any]? = nil, headers: [String: String],
                             timeout: TimeInterval = 20) async throws -> Data {
        var req = HTTP.request(url, method: method, headers: headers, timeout: timeout, cachePolicy: .reloadIgnoringLocalCacheData)
        if let body {
            req.httpBody = try JSONSerialization.data(withJSONObject: body)
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }
        let (data, code) = try await HTTP.data(for: req)
        guard (200..<300).contains(code) else { throw ServerError.http(code, String(decoding: data, as: UTF8.self)) }
        return data
    }

    /// A path the server answered with ("/v1/…"), as a URL.
    static func resolve(_ path: String, base: URL) -> URL? {
        URL(string: path, relativeTo: base)?.absoluteURL
    }

    /// The title's subtitle tracks (embedded, sidecar and fetched), or nil when the server
    /// doesn't list them (absent, or too old).
    /// `GET /v1/items/{id}/subtitles`'s answer.
    struct SubtitleList: Decodable { let subtitles: [SubtitleTrack] }

    static func subtitles(base: URL, itemId: String, headers: [String: String]) async -> [SubtitleTrack]? {
        guard let data = try? await call(base.appendingPathComponent("v1/items/\(itemId)/subtitles"), headers: headers, timeout: 8),
              let a = try? snake.decode(SubtitleList.self, from: data) else { return nil }
        // A server too old to serve the files: none.
        guard a.subtitles.contains(where: { $0.url != nil }) || !a.subtitles.contains(where: \.text) else { return nil }
        return a.subtitles
    }

    /// A track's timing now; asking starts its match if that's still to do.
    static func subtitleTiming(base: URL, itemId: String, index: Int, headers: [String: String]) async throws -> SubtitleTiming {
        let url = base.appendingPathComponent("v1/items/\(itemId)/subtitles/\(index)/timing")
        return try snake.decode(SubtitleTiming.self, from: try await call(url, headers: headers, timeout: 15))
    }

    /// Keeps the viewer's nudge, or their choice of the original timing, on the server.
    static func setSubtitleTiming(base: URL, itemId: String, index: Int, nudge: Double? = nil, original: Bool? = nil,
                                  headers: [String: String]) async throws -> SubtitleTiming {
        var body: [String: Any] = [:]
        if let nudge { body["nudge"] = nudge }
        if let original { body["original"] = original }
        let url = base.appendingPathComponent("v1/items/\(itemId)/subtitles/\(index)/timing")
        return try snake.decode(SubtitleTiming.self, from: try await call(url, method: "PUT", body: body, headers: headers, timeout: 15))
    }

    /// OpenSubtitles results for a title (`language`: ISO 639-1 or -2; `query`: search by text).
    static func searchSubtitles(base: URL, itemId: String, language: String, query: String?, headers: [String: String]) async throws -> SearchAnswer {
        var c = URLComponents(url: base.appendingPathComponent("v1/items/\(itemId)/subtitles/search"), resolvingAgainstBaseURL: false)
        c?.queryItems = [URLQueryItem(name: "lang", value: language)]
        if let query, !query.isEmpty { c?.queryItems?.append(URLQueryItem(name: "q", value: query)) }
        // A "+" left as it is reads as a space on the server (see MoonriseClient.url).
        let encoded = c?.percentEncodedQuery?.replacingOccurrences(of: "+", with: "%2B")
        c?.percentEncodedQuery = encoded
        guard let url = c?.url else { throw ServerError.http(0, "bad URL") }
        return try snake.decode(SearchAnswer.self, from: try await call(url, headers: headers, timeout: 30))
    }

    /// Fetch a result: it becomes a track of the title (on every device). Answers the track.
    static func fetchSubtitle(base: URL, itemId: String, result: SearchResult, headers: [String: String]) async throws -> SubtitleTrack {
        var body: [String: Any] = ["result_id": result.id, "name": result.name, "hearing_impaired": result.hearingImpaired]
        if let l = result.language { body["language"] = l }
        let data = try await call(base.appendingPathComponent("v1/items/\(itemId)/subtitles/fetch"), method: "POST", body: body,
                                  headers: headers, timeout: 60)
        return try snake.decode(SubtitleTrack.self, from: data)
    }

    /// Send a subtitle file only this device has (an OpenSubtitles download made with the app's own
    /// key): it becomes a track of the title, so the stream carries it (AirPlay shows renditions,
    /// not the app's overlay). Answers the track.
    static func uploadSubtitle(base: URL, itemId: String, data: Data, name: String, language: String?,
                               headers: [String: String]) async throws -> SubtitleTrack {
        guard let text = String(data: data, encoding: .utf8) ?? String(data: data, encoding: .windowsCP1252) else {
            throw ServerError.http(0, "unreadable subtitle file")
        }
        var body: [String: Any] = ["srt": text, "name": name]
        if let language { body["language"] = language }
        let data = try await call(base.appendingPathComponent("v1/items/\(itemId)/subtitles/upload"), method: "POST", body: body,
                                  headers: headers, timeout: 30)
        return try snake.decode(SubtitleTrack.self, from: data)
    }

    /// A window of a subtitle track (`GET /v1/items/{id}/subtitles/{index}/cues?window=k`): the
    /// cues showing during [from, to) of the file's time, read on their own while the whole track
    /// isn't extracted yet (a fraction of a second). `complete`: the whole track is ready.
    /// A picture track (PGS, DVD, DVB) answers the same way with images: the `canvas` they are
    /// placed on, and each cue's `image` (its place on the canvas and a PNG), with no text.
    struct SubtitleWindow: Decodable {
        let window: Int
        let from: Double
        let to: Double
        let complete: Bool
        let source: String?
        let canvas: Canvas?
        let cues: [Cue]

        struct Cue: Decodable {
            let start: Double
            let end: Double
            let text: String
            let image: Image?
        }

        struct Canvas: Decodable {
            let width: Double
            let height: Double
        }

        struct Image: Decodable {
            let x: Double
            let y: Double
            let width: Double
            let height: Double
            /// Base64.
            let png: String
        }
    }

    static func subtitleWindow(base: URL, itemId: String, index: Int, window: Int, headers: [String: String]) async throws -> SubtitleWindow {
        var c = URLComponents(url: base.appendingPathComponent("v1/items/\(itemId)/subtitles/\(index)/cues"), resolvingAgainstBaseURL: false)
        c?.queryItems = [URLQueryItem(name: "window", value: String(window))]
        guard let url = c?.url else { throw ServerError.http(0, "bad URL") }
        return try snake.decode(SubtitleWindow.self, from: try await call(url, headers: headers, timeout: 30))
    }

    /// `/v1/info` capabilities by server address, kept a few minutes (a server can be updated
    /// while the app runs).
    @MainActor private static var capabilityCache: [String: (Set<String>, Date)] = [:]

    /// Whether the server serves subtitle windows (`subtitle-windows`); asked again after 5 minutes.
    @MainActor static func servesSubtitleWindows(base: URL) async -> Bool {
        await serves("subtitle-windows", base: base) ?? false
    }

    /// Whether the server offers a capability (nil: it didn't answer); asked again after 5 minutes.
    @MainActor static func serves(_ capability: String, base: URL) async -> Bool? {
        let key = base.absoluteString
        if let (caps, at) = capabilityCache[key], Date().timeIntervalSince(at) < 300 { return caps.contains(capability) }
        guard let caps = await capabilities(base: base) else { return nil }
        capabilityCache[key] = (caps, Date())
        return caps.contains(capability)
    }

    /// What the server offers (`/v1/info` capabilities), nil when it doesn't answer.
    static func capabilities(base: URL) async -> Set<String>? {
        struct Info: Decodable { let capabilities: [String] }
        guard let data = try? await call(base.appendingPathComponent("v1/info"), headers: [:], timeout: 5),
              let i = try? snake.decode(Info.self, from: data) else { return nil }
        return Set(i.capabilities)
    }

    /// The original file (Range/ETag: background URLSession resumes it).
    static func originalDownloadURL(base: URL, itemId: String) -> URL {
        base.appendingPathComponent("v1/items/\(itemId)/download")
    }

    /// Ask for a prepared MP4 (`quality` in bits/s, nil: the original remuxed).
    static func createDownloadJob(base: URL, itemId: String, quality: Int?, audio: [Int]?, headers: [String: String]) async throws -> DownloadJob {
        var body: [String: Any] = ["container": "mp4", "hdr": AVPlayer.eligibleForHDRPlayback]
        if let quality { body["quality"] = quality }
        if let audio { body["audio"] = audio }
        let data = try await call(base.appendingPathComponent("v1/items/\(itemId)/download-jobs"), method: "POST", body: body,
                                  headers: headers, timeout: 30)
        return try snake.decode(DownloadJob.self, from: data)
    }

    static func downloadJob(base: URL, id: String, headers: [String: String]) async throws -> DownloadJob {
        try snake.decode(DownloadJob.self, from: try await call(base.appendingPathComponent("v1/download-jobs/\(id)"), headers: headers, timeout: 15))
    }

    static func deleteDownloadJob(base: URL, id: String, headers: [String: String]) async {
        _ = try? await call(base.appendingPathComponent("v1/download-jobs/\(id)"), method: "DELETE", headers: headers, timeout: 5)
    }

    /// Whether a server answers at `base`.
    static func isUp(_ base: URL) async -> Bool {
        let req = HTTP.request(base.appendingPathComponent("health"), timeout: 2, cachePolicy: .reloadIgnoringLocalCacheData)
        guard let (data, code) = try? await HTTP.data(for: req) else { return false }
        return code == 200 && data == Data("ok".utf8)
    }

    /// Why the server didn't offer its stream (`offer`).
    enum OfferError: LocalizedError {
        /// The request failed (unreachable, timed out).
        case unreachable(String)
        /// 404: not in the library, or (a Moonrise token) not on a mounted share.
        case notFound(String)
        /// 422 and other refusals, with the server's message.
        case refused(Int, String)
        /// `supported: false`, with the server's reasons; `pending`: its keyframe index is still
        /// being made (ask again later).
        case unsupported([String], pending: Bool)
        /// Served, but not as asked (HDR to an SDR display, a quality it doesn't honour).
        case notAsAsked

        var errorDescription: String? {
            switch self {
            case let .unreachable(m): "The Moonrise server didn't answer: \(m)"
            case .notFound: "This title isn't available on the Moonrise server: it isn't in its library, or the share it is on isn't mounted."
            case let .refused(code, m): m.isEmpty ? "The Moonrise server can't play this title (\(code))." : "The Moonrise server can't play this title: \(m)"
            case let .unsupported(r, pending): pending
                ? "The Moonrise server is still indexing this file. Try again in a minute."
                : "The Moonrise server can't play this title" + (r.isEmpty ? "." : ": " + r.joined(separator: "; "))
            case .notAsAsked: "The Moonrise server can't play this title as asked."
            }
        }
    }

    /// The Mac app: it asks for neither `dv=1` nor `lossless=1`, whatever its display or
    /// audio output (macOS has no `AVPlayer.availableHDRModes`; its output stays on the lossy tracks).
    static let isMac: Bool = {
        #if os(macOS)
        true
        #else
        false
        #endif
    }()

    /// `dv=1` for a display that shows Dolby Vision (and HDR): the server then says so for a Dolby
    /// Vision title's original (profile 8.1/8.4, 5), and the display switches to Dolby Vision.
    /// Without it the base layer plays as HDR10/HLG. Servers that don't know it ignore it.
    /// Never on the Mac.
    static func dolbyVisionQuery(hdr: Bool, dolbyVision: Bool, mac: Bool = isMac) -> URLQueryItem? {
        !mac && hdr && dolbyVision ? URLQueryItem(name: "dv", value: "1") : nil
    }

    /// The display shows Dolby Vision.
    static var dolbyVisionDisplay: Bool {
        #if DEBUG
        // Test seam (simulators have no Dolby Vision display): `DV=1 scripts/tvsim.sh`.
        if DebugSeams.launch.dolbyVision { return true }
        #endif
        #if os(macOS)
        return false
        #else
        return AVPlayer.availableHDRModes.contains(.dolbyVision)
        #endif
    }

    /// The server's stream for `itemId`, starting at `start`, at `quality` (bits/s; nil: the
    /// original), or nil when it can't serve that.
    static func playback(base: URL, itemId: String, start: Double, quality: Int? = nil,
                         headers: [String: String]) async -> (URL, Playback)? {
        try? await offer(base: base, itemId: itemId, start: start, quality: quality, headers: headers)
    }

    /// `lossless=1` for an audio output that takes more than 2 channels: lossless multichannel
    /// tracks (TrueHD, DTS-HD MA…) then come as FLAC with every channel rather than E-AC-3 5.1.
    /// Servers that don't know it ignore it. Never on the Mac.
    static func losslessQuery(multichannel: Bool, mac: Bool = isMac) -> URLQueryItem? {
        !mac && multichannel ? URLQueryItem(name: "lossless", value: "1") : nil
    }

    /// The audio output takes more than 2 channels: HDMI to a TV or receiver that accepts
    /// multichannel PCM, AirPlay to a surround set. Apple devices decode FLAC in HLS; the Mac's
    /// output is left on the lossy tracks.
    static var multichannelOutput: Bool {
        #if DEBUG
        // Test seam (simulators have a stereo output): `LOSSLESS=1 scripts/tvsim.sh`.
        if DebugSeams.launch.lossless { return true }
        #endif
        #if os(iOS) || os(tvOS)
        return AVAudioSession.sharedInstance().maximumOutputNumberOfChannels > 2
        #else
        return false
        #endif
    }

    /// `playback`, saying why when the server doesn't serve it.
    static func offer(base: URL, itemId: String, start: Double, quality: Int? = nil, version: String? = nil,
                      headers: [String: String], timeout: TimeInterval = 6) async throws -> (URL, Playback) {
        var c = URLComponents(url: base.appendingPathComponent("v1/items/\(itemId)/playback"), resolvingAgainstBaseURL: false)
        // AVPlayer refuses an HDR-only stream on a display that can't show HDR; the server then
        // tone-maps HDR titles to SDR.
        let hdr = AVPlayer.eligibleForHDRPlayback
        c?.queryItems = [URLQueryItem(name: "hdr", value: hdr ? "1" : "0")]
        if let lossless = losslessQuery(multichannel: multichannelOutput) { c?.queryItems?.append(lossless) }
        if start > 1 { c?.queryItems?.append(URLQueryItem(name: "start", value: String(format: "%.3f", start))) }
        if let quality { c?.queryItems?.append(URLQueryItem(name: "quality", value: String(quality))) }
        if let dv = dolbyVisionQuery(hdr: hdr, dolbyVision: dolbyVisionDisplay) { c?.queryItems?.append(dv) }
        if let version { c?.queryItems?.append(URLQueryItem(name: "version", value: version)) }
        #if os(tvOS)
        // The system player shows its scrub preview from an I-frame stream: the server lists its
        // thumbnails as one (`trickplay`) once they are made. Older servers ignore it.
        c?.queryItems?.append(URLQueryItem(name: "trick", value: "1"))
        #endif
        guard let url = c?.url else { throw OfferError.unreachable("bad URL") }
        let req = HTTP.request(url, headers: headers, timeout: timeout)
        let data: Data, code: Int
        do { (data, code) = try await HTTP.data(for: req) } catch { throw OfferError.unreachable(error.localizedDescription) }
        let body = String(decoding: data.prefix(300), as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        if code == 404 { throw OfferError.notFound(body) }
        guard code == 200 else { throw OfferError.refused(code, body) }
        guard let p = try? JSONDecoder().decode(Playback.self, from: data) else { throw OfferError.refused(code, "unreadable answer") }
        // HDR as served on a display without it: refused (an unlabelled HDR stream didn't
        // play through a master playlist in testing). Servers that transcode tone-map it for us,
        // so `video` says SDR; older ones send the copy.
        let hdrServed = p.video.map { $0.range != "SDR" } ?? false
        // A quality only a server with variants honours (older ones send the copy).
        let honoured = quality == nil || p.served != nil
        guard p.supported, let master = p.master, let masterURL = URL(string: master, relativeTo: base)?.absoluteURL else {
            #if DEBUG
            fputs("MoonriseServer: not served (\(p.reasons.joined(separator: "; ")))\n", stderr)
            #endif
            throw OfferError.unsupported(p.reasons, pending: p.pending == true)
        }
        guard !(hdrServed && !hdr), honoured else {
            #if DEBUG
            fputs("MoonriseServer: not served as asked (hdr \(hdrServed), variant \(p.variant ?? "-"))\n", stderr)
            #endif
            throw OfferError.notAsAsked
        }
        #if DEBUG
        fputs("MoonriseServer: variant \(p.variant ?? "-") (\(p.served?.label ?? "-"), \(p.method ?? "-"))\n", stderr)
        #endif
        return (masterURL, p)
    }

    /// Playback of a session ended: the server stops its transcode and conversion jobs.
    /// `master` is the session's master playlist URL.
    static func close(master: URL) async {
        let s = master.absoluteString
        guard s.hasSuffix("/master.m3u8"), let session = URL(string: String(s.dropLast("/master.m3u8".count))) else { return }
        _ = try? await HTTP.data(for: HTTP.request(session, method: "DELETE", timeout: 3))
    }
}
