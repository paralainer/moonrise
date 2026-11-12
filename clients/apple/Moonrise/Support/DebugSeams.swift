#if DEBUG
import Foundation

/// Every launch-environment test seam of the app, in one place. Debug builds only: this file and
/// every use of it are inside `#if DEBUG`, so a release build has none of these names or effects.
///
/// Scripts set them on the simulator as `SIMCTL_CHILD_MOONRISE_…` (`scripts/sim.sh`,
/// `scripts/tvsim.sh`; their short variable is in brackets), or in a scheme's or `open`'s
/// environment. Unset and empty are the same; a flag is on only as `1`.
///
/// Sign-in and launch
/// - `MOONRISE_URL`, `MOONRISE_USER`, `MOONRISE_PASS`: sign in to that server without typing,
///   when nothing is signed in (`DebugSeed`).
/// - `MOONRISE_DEMO=1`: start in the demo, as Try the Demo does, when nothing is signed in [DEMO].
/// - `MOONRISE_SETUP_ADDRESS`, `MOONRISE_SETUP_CODE`, `MOONRISE_SETUP_USER`, `MOONRISE_SETUP_PASS`:
///   prefill the sign-in address and the server setup's code and first account (the Apple TV
///   simulator has no keyboard).
/// - `MOONRISE_LANDSCAPE=1|0`: iPhone/iPad, turn to landscape (or portrait) on launch [LAND].
/// - `MOONRISE_DISCOVER=<seconds>`: log the servers Bonjour and the subnet sweep find [DISCOVER].
/// - `MOONRISE_MAC_SEAM=1`: macOS, the file-driven UI automation (`MacTestSeam`).
///
/// Navigation
/// - `MOONRISE_OPEN_ITEM=<item id>`: open that item's page [OPEN].
/// - `MOONRISE_LIBRARY="lib=<id>;page=…;…"`: open the Library tab there (`LibrarySeam`) [LIB].
/// - `MOONRISE_MANAGE=settings|libraries|review|users|services|playback|activity|about|log|fix:<item id>`:
///   open Settings, Manage Server (that screen), or Fix Match for the item (`ManageSeam`) [MANAGE].
/// - `MOONRISE_MARK=<item id>=watched|unwatched`: mark it 8 s after launch, as the card's menu does
///   (changes the account's watch state) [MARK].
///
/// Playback
/// - `MOONRISE_AUTOPLAY=<item id>`: play it on launch [AUTOPLAY]; `MOONRISE_AUTOPLAY_AT=<seconds>`
///   from there, also for `MOONRISE_PLAY_DOWNLOAD` [AT].
/// - After the first start, as the menus would: `MOONRISE_AUTOPLAY_AUDIO=<track index>` [AUDIO],
///   `MOONRISE_AUTOPLAY_QUALITY=<bits/s>` (0: Original) [QUALITY], `MOONRISE_AUTOPLAY_BURN=<track index>`
///   (a bitmap subtitle, burned in) [BURN], `MOONRISE_AUTOPLAY_SEEK=<seconds>` [SEEK],
///   `MOONRISE_AUTOPLAY_SUB=<option id>` ("t:4", "moonrise:10000", "none") [SUB],
///   `MOONRISE_AUTOPLAY_SUBSEARCH=<language>` (search OpenSubtitles through the server, fetch the
///   first result) [SUBSEARCH], `MOONRISE_AUTOPLAY_VERSION=<file id>` (6 s in) [VERSION].
/// - `MOONRISE_EXTPLAY="8:on,16:sub=t:3,24:sub=none,30:off"`: AirPlay as the simulator can't, with
///   subtitle picks meanwhile (seconds after the first start); also logs the native cues [EXTPLAY].
/// - `MOONRISE_NATIVE_SUBS=1`: show the PiP subtitle rendition inline and log its cues [NATIVE].
/// - `MOONRISE_LOG_OVERLAY=1`: log every subtitle the overlay shows ("MoonriseOverlay") [OVERLAY].
/// - `MOONRISE_OPEN_EPISODES=<seconds>`, `MOONRISE_OPEN_SUBSEARCH=<seconds>`: open the Episodes
///   sheet, or Find Subtitles, that long after the player shows [EPISODES, SUBSEARCHUI].
/// - `MOONRISE_PIN_CONTROLS=1`: the player's controls stay up [PIN].
/// - `MOONRISE_SCRUB_AT=<seconds>`: the scrubber shows its preview there, as if dragged [SCRUB].
/// - `MOONRISE_SLOW_ANIM=<factor>`: slow Play and the page drag's finish, for mid-animation shots [SLOW].
/// - `MOONRISE_RECOVERY_GIVEUP=<seconds>`: stream recovery gives up after that long (default 60) [GIVEUP].
/// - `MOONRISE_NO_REPORT=1`: report no progress or watch state to the server (the scripts set it
///   unless REPORT=1).
///
/// Downloads
/// - `MOONRISE_AUTODOWNLOAD=<item id>[:<bits/s>]`: download it (0 or none: Original) [DL].
/// - `MOONRISE_PLAY_DOWNLOAD=<item id>`: play its finished download, offline [PLAYDL].
/// - `MOONRISE_DELETE_DOWNLOAD=<item id>`: delete its download [DELDL].
///
/// Device and network
/// - `MOONRISE_DV=1`: pretend the display does Dolby Vision [DV, tvsim.sh].
/// - `MOONRISE_LOSSLESS=1`: pretend the audio output is multichannel [LOSSLESS].
/// - `MOONRISE_FORCE_CELLULAR=1`: pretend the network is cellular.
/// - `MOONRISE_FORCE_TAILNET=1`: treat the home network as unreachable [AWAY].
/// - `MOONRISE_TS_AUTHKEY=<auth key>`: sign the embedded Tailscale node in with that key and turn it
///   on (`sim.sh` mints an ephemeral one for AWAY=1).
///
/// Server settings named in the app's text (`MOONRISE_LIBRARIES`, `MOONRISE_HWACCEL`,
/// `MOONRISE_DISABLE`) are the server's environment, not seams.
struct DebugSeams {
    /// The environment the seams are read from.
    let env: [String: String]

    /// The app's launch environment (read when asked, as `ProcessInfo` gives it).
    static var launch: DebugSeams { DebugSeams(env: ProcessInfo.processInfo.environment) }

    /// Every seam's variable.
    enum Key: String, CaseIterable {
        case url = "MOONRISE_URL", user = "MOONRISE_USER", pass = "MOONRISE_PASS"
        case demo = "MOONRISE_DEMO"
        case setupAddress = "MOONRISE_SETUP_ADDRESS", setupCode = "MOONRISE_SETUP_CODE"
        case setupUser = "MOONRISE_SETUP_USER", setupPass = "MOONRISE_SETUP_PASS"
        case landscape = "MOONRISE_LANDSCAPE"
        case discover = "MOONRISE_DISCOVER"
        case macSeam = "MOONRISE_MAC_SEAM"
        case openItem = "MOONRISE_OPEN_ITEM"
        case library = "MOONRISE_LIBRARY"
        case manage = "MOONRISE_MANAGE"
        case mark = "MOONRISE_MARK"
        case autoplay = "MOONRISE_AUTOPLAY", autoplayAt = "MOONRISE_AUTOPLAY_AT"
        case autoplayAudio = "MOONRISE_AUTOPLAY_AUDIO", autoplayQuality = "MOONRISE_AUTOPLAY_QUALITY"
        case autoplayBurn = "MOONRISE_AUTOPLAY_BURN", autoplaySeek = "MOONRISE_AUTOPLAY_SEEK"
        case autoplaySub = "MOONRISE_AUTOPLAY_SUB", autoplaySubSearch = "MOONRISE_AUTOPLAY_SUBSEARCH"
        case autoplayVersion = "MOONRISE_AUTOPLAY_VERSION"
        case extPlay = "MOONRISE_EXTPLAY"
        case nativeSubs = "MOONRISE_NATIVE_SUBS"
        case logOverlay = "MOONRISE_LOG_OVERLAY"
        case openEpisodes = "MOONRISE_OPEN_EPISODES", openSubSearch = "MOONRISE_OPEN_SUBSEARCH"
        case pinControls = "MOONRISE_PIN_CONTROLS"
        case scrubAt = "MOONRISE_SCRUB_AT"
        case slowAnim = "MOONRISE_SLOW_ANIM"
        case recoveryGiveUp = "MOONRISE_RECOVERY_GIVEUP"
        case noReport = "MOONRISE_NO_REPORT"
        case autoDownload = "MOONRISE_AUTODOWNLOAD", playDownload = "MOONRISE_PLAY_DOWNLOAD"
        case deleteDownload = "MOONRISE_DELETE_DOWNLOAD"
        case dolbyVision = "MOONRISE_DV", lossless = "MOONRISE_LOSSLESS"
        case forceCellular = "MOONRISE_FORCE_CELLULAR", forceTailnet = "MOONRISE_FORCE_TAILNET"
        case tailscaleAuthKey = "MOONRISE_TS_AUTHKEY"
    }

    // MARK: - Reading

    /// On only as "1".
    func flag(_ k: Key) -> Bool { env[k.rawValue] == "1" }
    /// The value, nil when unset or empty.
    func text(_ k: Key) -> String? { env[k.rawValue].flatMap { $0.isEmpty ? nil : $0 } }
    /// A number, nil when unset or not one.
    func number(_ k: Key) -> Double? { env[k.rawValue].flatMap(Double.init) }
    /// A whole number, nil when unset or not one.
    func integer(_ k: Key) -> Int? { env[k.rawValue].flatMap(Int.init) }

    // MARK: - Sign-in and launch

    /// Server address, user name and password to sign in with (all three, or nil).
    var signIn: (address: String, user: String, pass: String)? {
        guard let a = text(.url), let u = text(.user), let p = text(.pass) else { return nil }
        return (a, u, p)
    }
    var demo: Bool { flag(.demo) }
    var setupAddress: String? { text(.setupAddress) }
    var setupCode: String? { text(.setupCode) }
    var setupUser: String? { text(.setupUser) }
    var setupPass: String? { text(.setupPass) }
    /// true: landscape, false: portrait, nil: as it is ("1" and "0" only).
    var landscape: Bool? {
        switch env[Key.landscape.rawValue] {
        case "1": return true
        case "0": return false
        default: return nil
        }
    }
    /// Seconds to look for servers for (over 0).
    var discover: Double? { number(.discover).flatMap { $0 > 0 ? $0 : nil } }
    var macSeam: Bool { flag(.macSeam) }

    // MARK: - Navigation

    var openItem: String? { text(.openItem) }
    var library: String? { text(.library) }
    var manage: String? { text(.manage) }
    var mark: String? { text(.mark) }

    // MARK: - Playback

    var autoplay: String? { text(.autoplay) }
    /// Where autoplay (or a downloaded title's play) starts; 0 when unset.
    var autoplayAt: Double { number(.autoplayAt) ?? 0 }
    var autoplayAudio: Int? { integer(.autoplayAudio) }
    var autoplayQuality: Int? { integer(.autoplayQuality) }
    var autoplayBurn: Int? { integer(.autoplayBurn) }
    var autoplaySeek: Double? { number(.autoplaySeek) }
    var autoplaySub: String? { text(.autoplaySub) }
    var autoplaySubSearch: String? { text(.autoplaySubSearch) }
    var autoplayVersion: String? { text(.autoplayVersion) }
    var extPlay: String? { text(.extPlay) }
    var nativeSubs: Bool { flag(.nativeSubs) }
    var logOverlay: Bool { flag(.logOverlay) }
    var openEpisodes: Double? { number(.openEpisodes) }
    var openSubSearch: Double? { number(.openSubSearch) }
    var pinControls: Bool { flag(.pinControls) }
    var scrubAt: Double? { number(.scrubAt) }
    /// Animation slow-down factor (over 0).
    var slowAnim: Double? { number(.slowAnim).flatMap { $0 > 0 ? $0 : nil } }
    /// Seconds before recovery gives up (over 0).
    var recoveryGiveUp: Double? { number(.recoveryGiveUp).flatMap { $0 > 0 ? $0 : nil } }
    var noReport: Bool { flag(.noReport) }

    // MARK: - Downloads

    var autoDownload: String? { text(.autoDownload) }
    var playDownload: String? { text(.playDownload) }
    var deleteDownload: String? { text(.deleteDownload) }

    // MARK: - Device and network

    var dolbyVision: Bool { flag(.dolbyVision) }
    var lossless: Bool { flag(.lossless) }
    var forceCellular: Bool { flag(.forceCellular) }
    var forceTailnet: Bool { flag(.forceTailnet) }
    var tailscaleAuthKey: String? { text(.tailscaleAuthKey) }
}
#endif
