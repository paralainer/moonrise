# Contributing to Moonrise

Thanks for helping! Moonrise is a Rust server (`server/`) and native apps: SwiftUI for iPhone,
iPad, Mac and Apple TV (`clients/apple/`), Kotlin and Compose for Android TV and Android phones
(`clients/android/`), and C# and WinUI 3 for Windows (`clients/windows/`).

The rules and commands for working in the repository are in [AGENTS.md](AGENTS.md) (for people
and coding agents alike); architecture, server internals, testing and how to add a client are in
[docs/dev/](docs/dev/README.md), and the design decisions in [docs/adr/](docs/adr/README.md).

- **Bugs**: open an issue with the app version (Settings), the server version (Manage Server →
  About), the device, and what you did. For playback problems, the title's container and codecs
  (Manage Server → Activity, or `ffprobe`) and the server log lines around it help most. Don't
  paste tokens, keys or share passwords (the app's log view redacts them; raw `docker logs` do too
  for keys the server knows, but check).
- **Features**: open an issue first for anything large, so we can agree on the shape.
- **Security problems**: don't open a public issue; see [SECURITY.md](SECURITY.md).
- **Pull requests**: one topic each; describe what you tested (simulators, devices, server
  hardware). The [pull request template](.github/pull_request_template.md) has the checklist. A bug
  fix comes with a test that fails without it (and a row in the
  [regression map](docs/dev/testing.md#regression-map)), and a refactor changes no behaviour. Add a line to
  [CHANGELOG.md](CHANGELOG.md) under "Unreleased" for anything a user would notice.

Everyone taking part follows the [code of conduct](CODE_OF_CONDUCT.md).

## Checks

The local check is the gate for every change: run `make check` (`scripts/check.sh`) before you open
a pull request, and say in it that it passed. It runs the server tests and clippy, the Android and
Apple unit tests and the scripts' tests (`scripts/test-all.sh`, also `make test`), builds the server
image and the iOS, tvOS and macOS apps, and scans the committed tree as `scripts/export-public.sh`
does. Without a build Mac the Apple parts are skipped with a note; say so in the pull request.
The Windows app's shared rules are tested in Docker on any OS (`scripts/test-windows-core.sh`);
building the app itself needs Windows ([docs/dev/windows.md](docs/dev/windows.md)).
`make help` lists the other targets.

Run `make hooks` once per clone: it turns on the tracked git hooks in `.githooks/` (`git config
core.hooksPath .githooks`). The pre-push hook refuses a push to `main` (changes go through pull
requests) and any push whose files, commit messages or author and committer identities match the
private-detail patterns of `scripts/export-public.sh`, plus your own (one extended regex a line):
in `~/.config/moonrise/private-patterns.txt`, which every clone of yours reads (or the file named
by `MOONRISE_PRIVATE_PATTERNS`), and in this clone's git-ignored `scripts/local/private-patterns.txt`.
Put your own addresses, names, device ids and accounts there, never in the tracked list. Without
`~/.config/moonrise/private-patterns.txt` the hook and `scripts/export-public.sh` say so and check
only the generic patterns, so copy the file to every machine you push from. A file named by
`MOONRISE_PRIVATE_PATTERNS` must exist and be readable: if it doesn't, the hook refuses the push and
the export stops (exit status 4), so a typo can't drop your patterns. It reports the place and the
pattern, never the matching line. Fix the commit rather than skipping the hook with `--no-verify`.
`make check` reminds you when the hooks are off.

GitHub Actions doesn't run on pushes or pull requests. The workflows in `.github/workflows` are
manual only (`workflow_dispatch`): a maintainer can start one by hand for a hosted build, but a
green run there is not required and doesn't replace the local check.

## Versions

The server and the apps have separate version numbers; see [CHANGELOG.md](CHANGELOG.md#versioning).

## Building

See [docs/building.md](docs/building.md). In short: `make server-run` runs the server from your
checkout and prints its address and setup code; `make apple-sim` builds the iPhone app and launches
it on a simulator (on a Mac, or with `MAC=user@host`). Server tests: `scripts/rust-dev.sh cargo test` (Docker; the toolchain is pinned in
`server/rust-toolchain.toml`); apps (Xcode and XcodeGen versions in `clients/apple/Config/toolchain.env`):
`scripts/xcodegen.sh`, open `clients/apple/Moonrise.xcodeproj` (simulators build as cloned; for devices put your
team and bundle id in `clients/apple/Config/Signing.local.xcconfig`). The `scripts/` that drive a build Mac over
SSH need `scripts/mac.env` (copy `scripts/mac.env.example`). Android: `clients/android/build.sh`
(TV) and `clients/android/build.sh phone`, Docker only ([clients/android/README.md](clients/android/README.md)).
Windows: `clients\windows\build.ps1` on Windows ([docs/dev/windows.md](docs/dev/windows.md)).

## Code style

- Swift: the existing style (SwiftUI, `@Observable`, async/await); match the patterns already in
  the app (menus with checkmark columns like `TrackMenus.swift`, glass buttons on tvOS, rows
  aligned to the content margins). New UI must be checked on iPhone, iPad, Mac and Apple TV.
- Kotlin and C#: the style of the code around it; shared rules go in `core/` (Android) and
  `Moonrise.Core` (Windows) with tests, not in the screens.
- Rust: `rustfmt` is enforced (`server/rustfmt.toml`, lines up to 140; `make check` runs `cargo fmt
  --check`): run `scripts/rust-dev.sh cargo fmt` before committing. Comments say *why*.
- No new dependencies without a reason; no analytics, ever (see [docs/privacy.md](docs/privacy.md)).

## Licence

A contribution is licensed under the licence of the directory it changes: `server/` under the
AGPL-3.0-or-later (`server/LICENSE`), everything else under the MPL-2.0 (`LICENSE`). There is no
contributor licence agreement. A pull request that touches both is licensed part by part.
Third-party code in `server/third_party/` keeps its own licence.

The name "Moonrise" and its icon aren't covered by either licence; a fork uses its own name and
icon ([TRADEMARKS.md](TRADEMARKS.md)).
