# Building from source

## One command per component

The `Makefile` at the top of the repository wraps the scripts (any POSIX `make`; `make` alone lists
the targets):

| Command | What it does | Needs |
|---|---|---|
| `make check` | Everything a pull request must pass (`scripts/check.sh`): the tests below, `cargo fmt --check` on the server, the server image build, the iPhone and Apple TV simulator builds, and the public-export scan of the committed tree. `make check-quick` skips the image and the Apple builds | Docker; a build Mac for the Apple parts (skipped with a note without one) |
| `make hooks` | Turns on the tracked git hooks in `.githooks/` for this clone (`core.hooksPath`): the pre-push hook refuses pushes to `main` and pushes carrying the private details `scripts/export-public.sh` scans for (see CONTRIBUTING.md). `make check` reminds you while they're off | git |
| `make test` | The automatic tests only (`scripts/test-all.sh`; `make test PARTS="server android"` for some) | Docker; a build Mac for the Apple tests |
| `make server` | Builds the server image from `server/` (`moonrise-server:dev`; `SERVER_IMAGE=` for another name) | Docker |
| `make server-run` | Builds and runs the server from this checkout, then prints the address to open and the setup code (`make server-stop`, `make server-logs`) | Docker with compose, curl |
| `make apple-sim`, `make apple-tv-sim` | Builds the app for the iPhone or Apple TV simulator and launches it there | A Mac with Xcode and XcodeGen (this one, or one over SSH) |
| `make android` | The Android TV app, debug build (Moonrise Dev), in Docker (`clients/android/build.sh phone` for the phone app) | Docker |
| `make export DIR=<folder>` | The public copy of the committed tree, scanned for secrets and private details | Docker |

A first run from a fresh clone, on Linux with a Mac on the network or on a Mac itself:

```sh
make server-run                       # prints http://<this machine>:8097/ and a setup code
MAC=you@your-mac make apple-sim       # on the Mac itself: make apple-sim
```

The app starts at onboarding: pick the server (or type the address printed) and enter the setup
code. `MOONRISE_URL=http://<server>:8097 MOONRISE_CREDS_FILE=<file> make apple-sim` signs the
simulator in instead (the file: user name, then password, chmod 600).

### The server from source (`make server-run`)

`scripts/server-dev.sh` runs the server with `server/compose.dev.yml`, which builds the image from
`server/` (the published `server/docker-compose.yml` pulls a released image). It publishes port 8097
rather than using host networking, so it behaves the same with Docker Desktop on a Mac, and runs as
you. The data is in `.dev-data/` and `.dev-media/` is mounted read-only at `/media` for test videos
(both at the top of the checkout, git-ignored). With a published port the server's own log shows
the container's internal address (`172.17.x.x`) as the setup page, so the script prints this
machine's network address and `localhost` instead, with the setup code from the log. Bonjour is
off in this mode; the apps find the server by sweeping the network, or you type the address.

Settings: `MOONRISE_DEV_PORT` (8097), `COMPOSE_PROJECT_NAME` (`moonrise-dev`; another name runs a
second server), `MOONRISE_DEV_DATA`, `MOONRISE_DEV_MEDIA`, `MOONRISE_DEV_IMAGE`. To start over:
`make server-stop`, then delete `.dev-data/`.

## Tests

`scripts/test-all.sh` runs every automatic test: the shell scripts' tests (`scripts/tests/`), the
server's `cargo test` and clippy (in Docker), the Android unit tests (`clients/android/build.sh test`),
the Windows app's shared-rule tests (`scripts/test-windows-core.sh`, in Docker)
and the Apple apps' unit tests (`MoonriseTests`, Swift Testing, through `scripts/test-apple.sh` on a
throwaway simulator on the build Mac; skipped with a note when there is none). All of it is offline.
Run it, or `make check`, which adds the image and simulator builds and the export scan, before a
pull request. A bug fix comes with a test that fails without it. In Xcode, the
`MoonriseTests` scheme runs the Apple tests (`⌘U`) on an iPhone simulator.

## Toolchains

| Toolchain | Version | Where it's recorded |
|---|---|---|
| Rust (server) | 1.99.1, with clippy and rustfmt | `server/rust-toolchain.toml` |
| Xcode (apps) | 26.3 (26 or later required) | `clients/apple/Config/toolchain.env` |
| XcodeGen (apps) | 2.45.4 | `clients/apple/Config/toolchain.env` |
| Android | JDK 17, Gradle wrapper, AGP and Kotlin | `clients/android/` (see its README) |
| Windows | .NET 10 SDK, Inno Setup (Windows host) | [docs/dev/windows.md](dev/windows.md) |

The server's tests need only Docker: `scripts/rust-dev.sh` builds the dev image
`moonrise-rust-dev:<version>` (`server/docker/dev.Dockerfile`, from the version in
`rust-toolchain.toml`) the first time it's needed, and again when either file changes. Cargo's
download cache is the Docker volume `moonrise-cargo`. With rustup installed, `cargo` in `server/`
picks the same version from `rust-toolchain.toml`. `scripts/xcodegen.sh` checks the Mac's Xcode and
XcodeGen against `clients/apple/Config/toolchain.env`: another version prints a warning, an Xcode older than 26 or
a missing XcodeGen stops with a message.

## Server (`server/`)

Rust (pinned, see above), SQLite bundled. ffmpeg/ffprobe at run time (`MOONRISE_FFMPEG`, `MOONRISE_FFPROBE`); the
Docker image ships jellyfin-ffmpeg (CUDA filters, Quick Sync, VA-API, libx264).

```sh
scripts/rust-dev.sh cargo test --locked                             # in Docker, no Rust install needed
scripts/rust-dev.sh cargo clippy --all-targets -- -D warnings
cd server
cargo test                                                          # or with your own rustup
docker build -t moonrise-server .                                   # this machine's architecture
docker buildx build --platform linux/amd64,linux/arm64 -t ghcr.io/paralainer/moonrise-server:dev .
```

**Built-in keys.** Release images carry the project's TMDB and OpenSubtitles keys, compiled in
from a BuildKit secret (never from the source or a build argument):

```sh
printf 'MOONRISE_BUILTIN_TMDB_KEY=…\nMOONRISE_BUILTIN_OPENSUBTITLES_KEY=…\n' > keys.env   # chmod 600
docker build --secret id=builtin_keys,src=keys.env -t moonrise-server .
```

Without them the server works and asks admins for their own keys (or uses the fallbacks).
Forks: get your own keys (themoviedb.org → API; opensubtitles.com → consumers).

`server/deploy.sh` builds the image and runs it on a Docker host over SSH, described by the
git-ignored `server/deploy/local/` (see the script's header).

## Apps (`clients/apple/`: iOS, iPadOS, macOS, tvOS 26)

Prerequisites on the Mac: Xcode (26 or later; the version used is in `clients/apple/Config/toolchain.env`),
XcodeGen (`brew install xcodegen`), and an iOS and a tvOS simulator runtime (Xcode → Settings →
Components). For the screenshot and input scripts (`shot.sh`, `tap.sh`, `tvshot.sh`, `tvkey.sh`) also
[idb](https://fbidb.io): `brew install facebook/fb/idb-companion`, and the client in a virtualenv at
`~/idb-venv` (`python3 -m venv ~/idb-venv && ~/idb-venv/bin/pip install fb-idb`). Building and
launching on simulators doesn't need idb.

One SwiftUI target for all four platforms plus a widget extension (Live Activities for
downloads) and a Top Shelf extension (Apple TV: Continue Watching above the app in
the top row), generated with [XcodeGen](https://github.com/yonaskolb/XcodeGen).

1. **Your identity** (for devices; simulator builds work without it): copy
   `clients/apple/Config/Signing.local.xcconfig.example` to `clients/apple/Config/Signing.local.xcconfig` (git-ignored) and set
   `MOONRISE_BUNDLE_ID` (e.g. `com.yourname.moonrise`; the widgets are `<id>.widgets`, the Top
   Shelf `<id>.topshelf`) and `MOONRISE_DEVELOPMENT_TEAM`. Keychain items, background tasks and the
   download session are named after the bundle id. The Apple TV app and its Top Shelf share the keychain
   group `<team>.<id>.shared`, which needs no registration with Apple. Without the file a fresh clone is `org.example.moonrise` with no team.
2. **Optional: TailscaleKit** for remote access. `scripts/build-tailscalekit.sh` builds
   `clients/apple/Vendor/TailscaleKit.xcframework` from [libtailscale](https://github.com/tailscale/libtailscale)
   (needs Go). Without it the app builds and runs without the Remote Access section; nothing else
   changes. (tvOS never embeds it.)
3. `scripts/xcodegen.sh` (it includes TailscaleKit when present), then open `clients/apple/Moonrise.xcodeproj`
   and run the **Moonrise** scheme on a simulator or your device.

Simulator builds without signing:

```sh
xcodebuild build -project clients/apple/Moonrise.xcodeproj -scheme Moonrise -destination 'generic/platform=iOS Simulator' CODE_SIGNING_ALLOWED=NO
```

Debug builds have test seams (environment variables `MOONRISE_*`, read only in DEBUG), e.g.
`MOONRISE_DEMO=1` starts in the demo, `MOONRISE_NO_REPORT=1` keeps scripted runs out of watch history.
All of them are listed in `clients/apple/Moonrise/Support/DebugSeams.swift`; release builds have none.

### The repository's own scripts

`scripts/` drives a build Mac over SSH from another machine (this project was developed from
Linux): `build.sh`, `sim.sh`/`tvsim.sh` (install and launch on simulators), `shot.sh`/`tap.sh`/
`tvkey.sh` (screenshots and input through idb), `deploy-phone.sh`/`deploy-tv.sh`/`deploy-mac.sh`
(signed installs; `deploy-android-tv.sh <tv-address>` builds the Android TV app in Docker and
installs it over adb, see `clients/android/README.md`), `ota.sh` (an ad-hoc IPA), `testflight.sh`; `<script> --help` shows a script's
usage. They read the git-ignored `scripts/mac.env` (copy `scripts/mac.env.example`; at least `MAC`,
the SSH target, or run with `MAC=user@host`) and the bundle id from `clients/apple/Config/`.

**On a Mac itself** the scripts run locally: with no `MAC` set (or `MAC=local`) they run the
commands they would send over SSH in a local shell, and build in your checkout (the build folders
and the generated project are git-ignored); a checkout outside your home folder builds in a copy,
`~/moonrise-build`. With `MAC_DIR` set they copy the tree to `~/$MAC_DIR` as they would to a remote
Mac, but never into a folder that exists and isn't an earlier build copy. No Remote Login needed.
You can also ignore the scripts and use Xcode.

**Simulators.** `sim.sh` and `tvsim.sh` use `SIM_PHONE`/`SIM_TV` from `mac.env` (or `SIM=<udid>`).
With neither set they use the simulator `scripts/sim-create.sh` makes, "Moonrise iPhone" or
"Moonrise Apple TV" (the newest installed runtime and a device type it takes, `scripts/sim-new.sh`), creating it the first time; it prints the UDID to put in
`mac.env`. `scripts/sim-create.sh --delete phone|tv` removes it again.

**Two apps side by side.** Every script acts on the app itself by default; with `DEV=1` it acts
on **Moonrise Dev** instead: bundle id `<your id>.dev` (widgets `<id>.dev.widgets`, Top Shelf
`<id>.dev.topshelf`, keychain group `<team>.<id>.dev.shared`), the deep-link scheme
`moonrise-dev`, an amber icon (`scripts/dev-icons.sh`), built into `build-<platform>-dev` on the Mac, and on Mac
`/Applications/Moonrise Dev.app`. It installs next to the app with its own sign-in, data, widgets
and Top Shelf, so changes can be tried on a device without touching the app used for watching.
`DEV=1 scripts/build.sh sim && DEV=1 scripts/sim.sh`, `DEV=1 scripts/deploy-tv.sh`, and so on.
`scripts/ota.sh` and `scripts/testflight.sh` ship the app itself only and refuse `DEV=1`.
On Android TV the same split is the build type: release is Moonrise, debug is Moonrise Dev
(`clients/android/README.md`).

The `deploy-*.sh` scripts check that the build Mac and the device (`deploy-android-tv.sh`: the
TV, over adb) can be reached before they build, and exit 1 with devicectl's answer when the
install doesn't happen. On success they print the build stamp (the commit, `-dirty` with uncommitted changes) that Settings shows next to the
version, so you can tell which build a device has. Device ids in the environment
(`PHONE_DEVICE=<id>`) win over `mac.env`.

A fresh clone needs only the build Mac: `scripts/build.sh sim`, `tvos` and `macos` are unsigned
and use the placeholder bundle id. The signed steps (`build.sh ios|tv|mac`, the `deploy-*.sh`
scripts, `ota.sh`, `testflight.sh`) stop with a message until `clients/apple/Config/Signing.local.xcconfig` has
your bundle id and team, and they also use an App Store Connect API key on the Mac (`ASC_ENV`,
`ASC_KEY_PATH` in `mac.env`). `sim.sh`/`tvsim.sh` sign in to `MOONRISE_SERVER_URL` with
`MOONRISE_CREDS_FILE` (or an executable, git-ignored `scripts/local/creds.sh`); with no server set,
or `SEED=0`, the app starts at onboarding.

Device builds are compiled with `SWIFT_OPTIMIZATION_LEVEL=-O` even in Debug (`build.sh`): an
unoptimised build can overflow the iPad's main-thread stack on a title page.

## Android apps (`clients/android/`)

Only Docker is needed: `clients/android/build.sh` builds in a pinned container (JDK 17, the Android
command-line tools; Gradle, AGP and Kotlin from the wrapper and the version catalog).

```sh
clients/android/build.sh            # Android TV, debug: tv/build/outputs/apk/debug/tv-debug.apk
clients/android/build.sh phone      # phones and tablets, debug: phone/build/outputs/apk/debug/phone-debug.apk
clients/android/build.sh test       # the unit tests (core, against api/)
scripts/deploy-android-tv.sh <tv-address>   # build and install on a TV over adb (DEV=1: the debug app)
```

The debug builds are **Moonrise Dev** (`app.moonrise.tv.dev`, `app.moonrise.phone.dev`), apps of
their own beside the release builds. A release build (`clients/android/build.sh release`) needs your
own signing key; installing on a device, the test seams and the signing are in
[clients/android/README.md](../clients/android/README.md).

## Windows app (`clients/windows/`)

On Windows 10 or 11 (x64) with the .NET 10 SDK and Inno Setup:

```
winget install Microsoft.DotNet.SDK.10 JRSoftware.InnoSetup
powershell -ExecutionPolicy Bypass -File clients\windows\build.ps1        # installer in clients\windows\build\installer
powershell -ExecutionPolicy Bypass -File clients\windows\build.ps1 -Dev   # Moonrise Dev
```

`build.ps1` downloads the pinned LGPL libmpv and checks its SHA-256. The shared rules' tests run on
any OS in Docker: `scripts/test-windows-core.sh`. More, including how libmpv is pinned and updated:
[dev/windows.md](dev/windows.md).

## Releasing

See [publishing.md](publishing.md).
