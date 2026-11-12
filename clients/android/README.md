# Moonrise for Android TV and Android phones

The Android clients for the Moonrise server: Google TV and Android TV, and Android phones. The TV app follows
the Apple TV app (`clients/apple/Moonrise/`, tvOS target): the same screens, wording and layout, in Jetpack
Compose for TV with Media3 for playback.

This folder is self-contained: it builds on its own and can move as a whole.

| Module | What it is |
|---|---|
| `core/` | The server's API (`/v1/...`) in plain Kotlin/JVM, with no Android classes, so a phone or tablet app can share it. Unit tests run on the JVM. |
| `shared/` | What the Android apps share beyond `core`, with Android classes but no screens: the signed-in server (`Session`), this device's settings (`AppSettings`), servers found on the network, a title page's data (`DetailsState`), the sign-in code's QR and the player (`PlayerModel`: Media3, the server's sessions, tracks, segments and reports). |
| `tv/` | The TV app: sign-in, Home, title pages, the player's screens and the home screen's Watch Next row. |
| `phone/` | The phone app, which follows the iPhone app (see [The phone app](#the-phone-app)). |
| `tailscale/` | Remote access: a small Go package over Tailscale's `tsnet`, bound to Kotlin with gomobile into `vendor/moonrise-tailscale.aar` (see [Remote access](#remote-access-tailscale)). |
| `tools/` | `artwork.sh`: the icon and banner from the Apple TV app's artwork. |

## Build

Only Docker is needed. From the repository root:

```sh
clients/android/build.sh           # debug APK: clients/android/tv/build/outputs/apk/debug/tv-debug.apk
clients/android/build.sh test      # core's unit tests, and a build without Tailscale
clients/android/build.sh --shell   # a shell in the build container
clients/android/build.sh <tasks>   # any Gradle tasks, e.g. :tv:assembleRelease
```

The first run builds the image `moonrise-android-build:1` (JDK 17 and the Android command-line
tools, `docker/Dockerfile`) and installs the SDK packages. The base image, the command-line tools
and the Gradle distribution are pinned by checksum, and the image is rebuilt by itself when
`docker/` changes. Later runs reuse two named volumes:

- `moonrise-android-sdk`: the Android SDK (platform 35, build tools, platform-tools).
- `moonrise-android-gradle`: Gradle and its caches, plus the debug signing key
  (`android-user/debug.keystore`), so each debug build installs over the last one.

Remove both volumes to start from scratch. Removing the Gradle volume makes a new debug key, and
the next install then needs the old app uninstalled first.

## Remote access (Tailscale)

Both apps can reach the server away from home through Tailscale, built in as on the Apple apps
([docs/remote-access.md](../../docs/remote-access.md)): **Settings → Remote Access → Connect Through
Tailscale**. The app runs its own Tailscale node in userspace (no Tailscale app, and not Android's
VPN slot) and joins the tailnet as its own device (`moonrise-android`, `moonrise-androidtv`). The
phone signs in in the browser; the TV shows the sign-in page as a QR code and its address.

- **The library:** `scripts/build-tailscale-android.sh` (Docker only) builds `tailscale/` into
  `vendor/moonrise-tailscale.aar` for arm, arm64 and x86_64 (`TS_ANDROID_TARGETS` for others, e.g.
  `android/386` for an x86 TV emulator), after running its Go tests. It is git-ignored; `shared`
  includes it (`src/tailscale`) when it is there, and otherwise builds `src/notailscale`: no Remote
  Access section and no node. `-Pmoonrise.tailscale=false` leaves it out on purpose; `build.sh
  test` compiles that build too. It adds about 22 MB per ABI to an APK.
- **The route** (`ServerReach`, the rules in `core`'s `Routing.kt`, as the Apple apps'
  `ConnectionRouter`): the home address first; when it doesn't answer and the node is up, the
  server on the tailnet (the host it was found at before, then peers named like it), through a
  local forwarder on `127.0.0.1` that the HTTP clients and the player use like any address, and
  only when it is the same server (its id). `Session.api.baseUrl` follows the route; requests wait
  for the first check of a launch. Checked again when the network changes, the app comes to the
  front, and every 30 s (5 s while the server can't be reached).
- **Playback:** a stream opened at home moves to the tailnet as soon as the route changes, and one
  on the tailnet moves home once it stalls (`StreamRouteRules`).
- **When the server can't be reached,** the screens say why and offer what helps: Connect with
  Tailscale, Sign In to Tailscale (the QR code on a TV), Restart Tailscale.
- **Logs:** `adb logcat -s MoonriseTS Moonrise` (debug builds): the node's state and, with its
  own lines, what it does; `route: … -> …` for each route change. The node sends no logs to
  Tailscale.
- **Test seams** (debug builds): `--es tsauthkey <auth key>` turns it on and signs in with the key
  (an ephemeral device, no browser), `--ez forcetailnet true` (until `false`) skips the home
  address, so the tailnet route can be tried at home.

## Two apps: Moonrise and Moonrise Dev

The two build types are two apps, which sit side by side on one TV:

| Build | App | What for |
|---|---|---|
| release | **Moonrise**, `app.moonrise.tv` | Watching. R8, signed with the release key (below). |
| debug | **Moonrise Dev**, `app.moonrise.tv.dev` | Trying out changes and scripted tests (the test seams below). Amber icon, "Moonrise Dev" banner. |

Each keeps its own sign-in, settings and Watch Next rows (the TV provider keeps an app's rows to
itself), so installing or clearing one never touches the other. Both keep the class names of
`app.moonrise.tv` (the code's namespace), so the Dev app's activity is
`app.moonrise.tv.dev/app.moonrise.tv.MainActivity`.

## Install on a TV

Turn on developer options and network debugging on the TV, then from the repository root:

```sh
scripts/deploy-android-tv.sh 192.168.1.50            # Moonrise (release): builds, installs over adb, opens it
DEV=1 scripts/deploy-android-tv.sh 192.168.1.50      # Moonrise Dev (debug)
ANDROID_SERIAL=<serial> scripts/deploy-android-tv.sh # a device the adb server already lists (USB, emulator)
ADB_PORT=5037 ANDROID_SERIAL=<serial> scripts/deploy-android-tv.sh   # through the computer's own adb server
```

It stops with a non-zero exit and adb's answer when the install doesn't happen (the TV can't be
reached or hasn't allowed this computer, or the app on it is signed with another key). It never
uninstalls anything, as that would sign the app out. `NO_BUILD=1` installs the last build,
`NO_LAUNCH=1` leaves the app closed; `ANDROID_TV` can stand for the address.

A TV on USB is seen by the adb server of the computer it's plugged into (usually on port 5037),
not by `adb.sh`'s own server: `ADB_PORT=5037` sends the commands there, and `ANDROID_SERIAL` names
the device as `adb devices` lists it.

By hand:

```sh
clients/android/adb.sh connect <tv-address>:5555     # the TV asks once to allow this computer
clients/android/adb.sh -s <tv-address>:5555 install -r clients/android/tv/build/outputs/apk/debug/tv-debug.apk
clients/android/adb.sh -s <tv-address>:5555 shell am start -n app.moonrise.tv.dev/app.moonrise.tv.MainActivity
clients/android/adb.sh -s <tv-address>:5555 exec-out screencap -p > screen.png
clients/android/adb.sh stop                          # stop the adb server container
```

## Release build and signing

```sh
MOONRISE_ANDROID_SIGNING=~/.config/moonrise/signing.properties clients/android/build.sh release
# → clients/android/tv/build/outputs/apk/release/tv-<abi>-release.apk and tv-universal-release.apk
MOONRISE_ANDROID_SIGNING=… clients/android/build.sh phone-release   # the phone app: phone/…/phone-<abi>-release.apk
```

The release build is shrunk and optimised by R8 (much smoother on a TV than a debug build) and
signed with a release key that never goes in the repository. `signing.properties` names it:

```properties
storeFile=moonrise-release.jks      # relative to this file
storePassword=…
keyAlias=moonrise
keyPassword=…                        # a PKCS12 store: the same as storePassword
```

`build.sh` mounts the file's folder read-only into the build container. Gradle also reads
`MOONRISE_ANDROID_SIGNING` directly, or a git-ignored `clients/android/signing.properties`;
without either, `:tv:assembleRelease` makes unsigned `*-release-unsigned.apk` files and
`build.sh release` stops.

Release builds are split by CPU type (ABI): `armeabi-v7a`, `arm64-v8a` and `x86_64`, each with
only its own native libraries (Tailscale's and the FFmpeg audio decoders'), and a universal APK
with all of them. `scripts/deploy-android-tv.sh` installs the one for the device's
`ro.product.cpu.abilist` (its first entry with an APK, else the universal one; a TV with a 64-bit
CPU and a 32-bit system lists `armeabi-v7a` first): `scripts/android-apk.sh`. `APP=phone` does the
same for the phone app (`APP=phone ANDROID_SERIAL=<serial> ADB_PORT=5037
scripts/deploy-android-tv.sh`). Debug builds stay one APK. To make a key:

```sh
keytool -genkeypair -keystore moonrise-release.jks -storetype PKCS12 -alias moonrise \
    -keyalg RSA -keysize 4096 -validity 10000 -dname "CN=Moonrise"
```

Keep the keystore and its password safe and backed up: an app signed with another key can't
update it, and a TV then has to uninstall it (losing its sign-in) first. `.jks`, `.keystore` and
`signing.properties` are git-ignored here.

## Artwork

The icon (an adaptive icon: the Moonrise mark on black) and the home screen banner (the Moonrise mark and the
name, 320 × 180 dp) come from the Apple TV app's artwork in `clients/apple/Moonrise/AssetsTV.xcassets`.
The Dev app's (`src/debug/res`) are the same logo on amber and a banner reading "Moonrise Dev".
`tools/artwork.sh` makes the banners and the icon again (ImageMagick in a container) when that
artwork changes.

## adb

`adb.sh` runs adb from the SDK volume in a container (`moonrise-android-adb`, host network, adb
server on port 5038 so it doesn't disturb another adb server). It uses this computer's adb key
in `~/.android` (`ADB_KEYS=<dir>` for another). `ADB_PORT=<port>` picks the port: when an adb
server already answers there (the computer's own, say), commands go to it and nothing is started.

Screenshots show the app's screens, but not the video while it plays: the video goes straight to
the display, so a screenshot of the player is blank.

## What it does

- **Sign-in:** servers found on the network (Bonjour `_moonrise._tcp`) or a typed address; then
  a code to approve on an iPhone or in a browser (with a QR code of the approval page), or a user
  name and password. As on Apple TV, the keyboard opens when you select a field, not when focus
  passes over it. The token is kept encrypted with a key in the Android Keystore.
- **Tabs:** Home, Library, Search and Settings in a bar at the top, over the content, which
  scrolls away with it. Moving along the bar opens a tab, and focus stays on the bar; Down goes
  into the tab (to its first control, scrolled back into view, when nothing is below the bar).
  Back in a tab goes up to the bar (in the Library, from the grid or tiles to the sidebar first),
  and Back on the bar goes to Home, then out of the app. Coming back from a title page, focus
  returns to the card it was opened from, on every tab.
- **Home:** the banner of recently added titles (left and right switch, select opens), then
  Continue Watching (the server's, the same on every device; long press for Remove from Continue
  Watching), Recently Added Movies and Recently Added Shows, with posters and stills
  that grow and glow when focused. Under each card, the title (the show's name for an episode),
  then the year or "S1 · E3 · Title". Back from a title focuses the card it was opened from.
- **Library:** a sidebar (Movies, Shows, then Genres, Collections, Studios or Networks, Decades)
  beside the poster grid. The header has the count, Jump To (title sort), Sort (six sorts, both
  directions) and Filter, a panel from the right with Genre, Year, Rating, Watched, Resolution,
  HDR and Recently Added, with counts; choices apply at once, and the filters that are on show
  under the header with Clear. A category's tile filters the grid and focus goes to its first
  title once it's loaded; Back returns to the tile. Each view remembers its filters and sort.
- **Search:** titles and episodes as you type, in Shows, Movies and Episodes rows. Each row
  comes back to the card focused there last.
- **Title page:** the backdrop across the screen, the logo, the episode it's about, the title's
  line (runtime, date, rating, picture and sound badges, genres), Play or Resume, Play from
  Beginning, Version (several files; a damaged one is greyed out and can't be picked) and More
  (Mark as Watched or Unwatched), the overview (select shows all of it), Starring, Director and
  Rating. A show adds its seasons (moving onto one shows its episodes) and episodes with progress
  and watched marks; select plays an episode. Then Cast & Crew.
- **Long press** on a card, a season or an episode: Mark as Watched or Unwatched (and Play from
  Beginning for an episode). The mark shows on every card of the title at once.
- **Player:** the server's playback session and HLS master through Media3, with the app's own
  controls over the picture (Media3's view only shows it).
  - **The remote, controls hidden:** select or play/pause pauses and resumes; left and right skip
    by the Settings' Skip Back and Skip Forward (presses add up to one seek, as on Apple TV); up
    shows the controls; down opens the panel; Back leaves.
  - **The controls:** the title, the scrubber with the time played and the time left, and Audio,
    Subtitles and Quality above it on the right (up from the scrubber). Down goes to Skip or Next
    Episode when they show, else opens the panel. Back hides the controls. They go after 5 s
    while playing.
  - **The panel from the top** (the Apple TV's swipe-down): Next Episode near the end of an
    episode, Info (the artwork, title, overview, genres, year, runtime, age rating, the picture and
    sound badges, and how the server serves it) and Episodes (select plays one).
  - **Audio:** the track in the language set in Settings (picking one in the player sets it), else
    the title's default. Audio the TV can't decode (AC-3, E-AC-3, DTS, TrueHD without a receiver)
    is decoded in software (`media3-ffmpeg-decoder`).
  - **Subtitles:** text tracks are the server's files, drawn by the app (an embedded track is read
    in 30 s windows at the playhead until the server has all of it); picture tracks (PGS, DVD, DVB)
    are drawn the same way from the server's images. Turned on by themselves in the language set in Settings (SDH or plain,
    as picked last), else forced ones in the audio's language. The Subtitles menu has Style (size,
    background box), Delay (kept on the server per file and track, for every device; Use Original
    Timing when the server synced a downloaded track) and Search Online (OpenSubtitles through the
    server; a fetched track is kept on the server, synced to the file, and selected).
  - **Quality:** the streaming quality (capped to the file) and, for a title with several, the
    version; picking one restarts the stream where it is.
  - **Skip Intro, Skip Credits and Next Episode** follow the Apple TV app's rules
    (`SegmentRules` and `NextEpisodeRules` in `core`): the server's merged segments (named
    chapters, an episode's end credits found in the file, gaps filled from TheIntroDB and IntroDB,
    which the server looks up itself; servers 0.28+), asked for again a few seconds later while the
    server is still looking the title up, and from 5 min before the end while it is still looking
    in the file. With an older server, its segments alone by the same rules. The app contacts only
    the Moonrise server. Skip shows in the bottom corner and takes the remote (select
    skips; any other press keeps watching); auto-skip counts down first. Next Episode is offered
    from end credits that run to the end, else in the last 30 s, and plays the next episode across
    seasons (not into the specials). At the end of an episode the next one plays by itself.
  - **HDR and 4K:** HDR titles play as HDR when the display reports any HDR type; otherwise the
    server tone-maps them. When the display also lists Dolby Vision and the device has a Dolby
    Vision decoder, the player asks for it (`dv=1`): a Dolby Vision title's Original then plays
    through that decoder (profile 8 is signalled with `SUPPLEMENTAL-CODECS`, which Media3 1.6+
    reads; profile 5 as `dvh1`). Elsewhere profile 8 plays its HDR10/HLG base layer. 4K HEVC is
    copied when the TV decodes it. Debug builds: `--ez dv true` asks for Dolby Vision whatever the
    display.
  - **A lost stream** (the server restarted, the network dropped): the player opens a new session
    where it was, with the same audio, subtitles and quality, playing or paused as before
    ("Reconnecting…" after a few seconds). After a minute without the server it stops and says
    so; Play tries again.
  - **A quality or version this TV can't decode** (its decoder refuses it, or has none for the
    video): the player goes back to the one that played, where it was, and says so ("This TV
    can't play 1080p · 10 Mbps. Back to Original."). A title that never plays says the TV can't
    play its video; errors that keep coming back say which choice the TV can't decode.
  - Leaving the app stops playback and closes the session; coming back reopens the title where it
    was, paused.
- **Watch state:** reported as the Apple apps report it (`/v1/sessions/{sid}/start|progress|stop`):
  start once playback starts, progress every 10 s while it plays, stop when it ends, the player
  is left or the app goes to the background. The server keeps the resume position (per version)
  and marks the title played near its end. Marks use `PUT`/`DELETE /v1/items/{id}/played`, and a
  version picked on the title page is remembered for every device (`PUT /v1/items/{id}/version`).
- **Settings:** the server (name, address, who is signed in, connection), Change Server and Sign
  Out (both revoke this device's token), Remote Access (Tailscale, above), Streaming Quality, Skip Back and Skip Forward, Play Next
  Episode Automatically, Skip Intros & Credits (per kind: Skip Automatically, Show Skip Button or
  Off; the auto-skip countdown; Use Online Databases), the preferred audio language, and
  subtitles (preferred language, Turn On Automatically, size and background box).
- **When the server can't be reached:** the screen (Home, the Library's grid and category tiles,
  Search, a title page that hasn't loaded) says so with Retry, and tries again by itself: after
  5 s, then waiting twice as long each time up to every 30 s, and when the network comes back
  (Home also when the app comes back to the front, as after the TV wakes).

- **The TV's home screen:** the titles in Continue Watching appear in the system's Watch Next row
  ("Continue watching" on Google TV), with their artwork, progress, and season and episode.
  Selecting one opens the app and plays it from where it was (its title page underneath, for
  Back). The row follows the server: when the app starts, when playback stops (a title finished
  leaves it, its next episode joins), after a mark, and every 30 minutes in the background
  (WorkManager), so watching on another device shows up too. A title the viewer removed from the
  row stays out until it's watched again. Signing out removes them all. The rules are
  `WatchNextRules` in `core`, with unit tests.
- **Focus and motion:** a focused card lifts (scales up) with a soft shadow below it, a rim light
  and a faint sheen across the artwork; its caption moves down to make room. Buttons and rows
  ease between their looks, and each screen fades in (a title page with a slight zoom).

## Checking from logcat

The picture can't be captured, so the player logs what it does (`adb logcat -s` with these tags):
`MoonrisePlayer` (sessions, reports, recovery; every 10 s the position and decoded frames),
`MoonriseSegments` and `MoonriseNext` (segments, the Next Episode offer), `MoonriseSkip`,
`MoonriseAudio`, `MoonriseSubs` and `MoonriseOverlay` (the subtitle text on screen),
`MoonriseSubSearch`, `MoonriseFocus` (where focus is, in the player too) and `MoonriseWatchNext`
(the home screen's row: each sync and what it changed).

These debug and info lines are written by debug builds (the Dev app) only: the app logs through
`app.moonrise.shared.Log`, which leaves them out of release builds (R8 drops the calls and their
text). Warnings and errors (`Log.w`, `Log.e`: a failed request, a playback error) are logged in
every build. Use `Log` from the app's package, not `android.util.Log`.

Debug builds (the Dev app) take a test seam: `adb shell am start [--activity-single-top] -n
app.moonrise.tv.dev/app.moonrise.tv.MainActivity --es play <item id> [--es start <seconds>]` plays a title (its page
underneath), and `--es open <item id>` opens its page. `--ez watchnext true` logs the Watch Next
rows as the TV provider holds them (`MoonriseWatchNext`; adb itself can't read them), and
`--ez refuseoriginal true` (until `false`) makes players find no video decoder at the Original
quality, to check the fallback above. `--ez lossless true` (until `false`) makes players ask for
lossless multichannel audio (FLAC, `lossless=1`) as if the HDMI output took more than 2 channels. A Watch Next title's link works in any build:
`am start 'intent://play/<item id>#Intent;scheme=moonrise;component=app.moonrise.tv/.MainActivity;end'`
(for the Dev app, `component=app.moonrise.tv.dev/app.moonrise.tv.MainActivity`).

## The phone app

`phone/` is Moonrise for Android phones. It follows the iPhone app (`clients/apple/Moonrise`, iOS):
the same screens in the same order, with its wording, sizes (the iPhone's points as dp) and
behaviour, and Android's own back navigation, system bars and media controls. It shares `core`
and `shared` with the TV app: the sign-in, settings, a title page's data and the player.

```sh
clients/android/build.sh phone      # phone/build/outputs/apk/debug/phone-debug.apk (Moonrise Dev, app.moonrise.phone.dev)
adb install -r clients/android/phone/build/outputs/apk/debug/phone-debug.apk
adb shell am start -n app.moonrise.phone.dev/app.moonrise.phone.MainActivity
```

As the TV app, the debug build is **Moonrise Dev** (`app.moonrise.phone.dev`, amber icon), its
own app beside the release one (`app.moonrise.phone`), with its own sign-in and settings.

On a tablet (a window at least 600 dp wide, on a device whose smallest width is at least 600 dp)
the app follows the iPad instead: `LayoutMetrics` in `core` (the Apple apps' `Metrics`) gives the
margins and the rows' sizes from the window's width, the tabs go in a capsule at the top with the
Settings gear at the corner, the Library has a sidebar, title pages sit at the leading margin (over the
picture when the window is wider than tall, where the video always plays full screen), Search has
a field at the top and Downloads a grid of cards. A phone and a narrow split-screen window keep
the phone layout, unchanged.

- **Sign-in:** the welcome, then the servers on the network (Bonjour, and a sweep of the local /24
  networks on port 8097), the address (checked as it's typed), Sign In with a Code (a code and a
  QR code to approve on another device or in a browser) or a user name and password. A new server
  offers Set Up This Server: the setup code, the admin account, the first library (a folder on
  the server or a network share), the first scan and the optional extras; an admin signing in to
  a server with no libraries continues at the library step. A server with invitations also offers
  Have an Invitation? (a code, a user name and a password). Settings has Approve a Device, which
  signs a device showing a code in with this device's sign-in.
- **Tabs:** Home and Library in a glass capsule at the bottom, Search in a circle beside it; on the
  Search tab its field takes the bar's place. A tab picked again goes back to its first page; Back
  goes back a page, then to Home, then out of the app. Settings is the gear on Home.
- **Home:** the banner (recently added titles; each page is the top of the title page it opens),
  Continue Watching (long press: Resume, Mark as Watched or Unwatched, Remove from Continue
  Watching), Recently Added Movies and Shows. Pull down to load again.
- **Library:** Library with Sort and Filter, Movies | Shows, the categories (Genres, Collections,
  Studios or Networks, Decades: pages of tiles that filter the grid), the filters that are on, the
  count with Jump To, then the poster grid. Each view keeps its filters and sort.
- **Search:** Shows and Movies as poster grids, Episodes as a row.
- **Title page:** the backdrop and logo, the episode's line and the title's line, Play or Resume
  (filled as far as it was watched) and the Version menu, the overview, the ratings (IMDb, TMDB,
  Rotten Tomatoes, Metacritic), a show's season menu and episodes (a tap selects, a tap on the
  selected one plays), Cast and Crew. Long press an episode or a card: Mark as Watched / Unwatched.
- **Player:** on the title page, as on the iPhone: the video takes the header's place, the page
  under it; turning the phone (or the full-screen button) fills the screen. Back, play/pause and
  the skips in the middle; Quality, Audio and Subtitles (Style, Delay, Search Online) menus; the
  scrubber with thumbnails; Skip Intro, Skip Credits and Next Episode from the server's merged
  segments (`NextEpisodeRules.corner` in `core`); full screen, vertical swipes set the brightness
  (left) and the volume (right) with a level at the top right, double taps skip, a pinch fills.
  Playback reports, track memory, recovery and the capability flags are the TV app's
  (`shared`'s `PlayerModel`). Headset and Bluetooth buttons and the system's media panel work
  through a media session. Leaving the page or the app stops playback.
- **Downloads** (`shared`'s `Downloads`, the rules in `core`'s `DownloadRules`): the iPhone's
  segmented downloads. The server's HLS segments of the quality picked and the title's text
  subtitles and skip markers are saved one by one, one title at a time (a foreground service
  keeps it going in the background, with its progress in a notification), and it plays from
  the device through `DownloadDataSource` as soon as segments are in: a segment that isn't here
  yet is fetched at once, and with no server the playlist ends where the download does. A download
  opens the normal title page. With no server (`ServerReach`) that page is built from what the
  download saved, server-only actions are hidden, Home and the Library show the downloads instead
  of an error, the next episode is the next downloaded one, and watch progress is queued
  (`LocalReports`, `POST /v1/sessions/local`) until the server is back. The Downloads tab, the
  Download button on title pages and in the player, season downloads and the quality per network
  (Settings) are as on the iPhone.
- **Manage Server** (Settings → Manage Server, admins; rules and wording in `core`'s `AdminRules`,
  the API in `core`'s `Admin.kt`, as `ManageServerViews.swift` and friends): Libraries (the scan,
  each library and its folders on the server or on a network share, Add Library), Fix Match,
  Users & Devices (accounts, their devices, invitations), Services (TMDB, OpenSubtitles, AI
  Matching and its provider, Ratings), Playback & Hardware, Activity, About This Server and its log.
  Signing in with ChatGPT is on the iPhone, iPad and Mac for now.
- **Fix Match** (admins; `FixMatchRules` in `core`): Manage Server's list of titles waiting, and
  any title's long press (the title block, a card's menu, Fix Match Versions… in the Version menu)
  opens it over the app. TMDB's candidates (no artwork here: apps talk only to the server), a
  search, episodes in TMDB's numbering, a title's files one by one (split off, match alone), Back
  to Automatic.

Debug builds take test seams: `--es open <item id>` opens a title's page on Home, `--es play <item
id> [--es start <seconds>]` plays it there, `--es tab home|library|search` switches tab, and
`--ez awake true` keeps the screen on while the app is in front (add `--activity-single-top`
while it runs). Downloads: `--es autodownload <item id>[:<bits/s>]`, `--es playdownload <item id>`,
`--es deletedownload <item id>`; `--ez noreport true` drops their watch reports. Manage Server:
`--es manage libraries|review|users|services|aimatching|playback|activity|about|log` opens it
there; `fix:<item id>` opens Fix Match on a title.

