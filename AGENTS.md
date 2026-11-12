# AGENTS.md

Guidance for anyone, human or coding agent, changing this repository. Keep it short; the depth is
in [docs/dev/](docs/dev/README.md).

## What Moonrise is

A self-hosted video library: a Rust server that scans your folders and network shares, identifies
movies and shows, keeps accounts and watch state, and streams HLS (the original copied whenever
possible); and native clients for iPhone, iPad, Mac and Apple TV (SwiftUI), Android TV (Compose
for TV), Android phones (Compose) and Windows (C#/WinUI 3). The server is AGPL-3.0-or-later; the apps, API, tooling and docs are MPL-2.0 (see
[README.md](README.md#license)). The name and icon are trademarks ([TRADEMARKS.md](TRADEMARKS.md)).

## Layout

| Path | What |
|---|---|
| `api/` | The API contract: `openapi.yaml` (OpenAPI 3.1) and its `examples/`; a server test checks it against the router |
| `server/` | Rust server (`src/`), migrations, Dockerfile, packaging. User docs: [server/README.md](server/README.md); internals: [docs/dev/server.md](docs/dev/server.md) |
| `clients/apple/` | One SwiftUI target for iOS, iPadOS, macOS, tvOS (+ widgets, Top Shelf), XcodeGen `project.yml`; tests in `MoonriseTests/` |
| `clients/android/` | Self-contained Gradle project: `core/` (API and rules, plain Kotlin, JVM tests), `shared/` (session, settings, player), `tv/` (the TV app), `phone/` (the phone app). See its [README](clients/android/README.md) |
| `clients/windows/` | Windows app: `Moonrise.Core` (API and rules, plain C#, tests on any OS), `Moonrise.App` (WinUI 3, libmpv), `build.ps1`, an Inno Setup installer. See [docs/dev/windows.md](docs/dev/windows.md) |
| `scripts/` | Build, test, check and export scripts (`<script> --help`); their tests in `scripts/tests/` |
| `docs/` | User docs; `docs/dev/` for contributors; `docs/adr/` for decisions |
| `assets/brand/` | Icon artwork (see its README); private working art stays out of the repo |

## Commands

| Command | What |
|---|---|
| `make` | Lists the targets |
| `make hooks` | Turn on the tracked git hooks (once per clone) |
| `make check` | Everything a pull request must pass: all tests, server image, Apple simulator builds, public-export scan |
| `make check-quick` | The same without the server image and Apple builds |
| `make test` | Tests only (`PARTS="server android"` for some) |
| `make server-run` / `server-stop` / `server-logs` | A server from this checkout, with its address and setup code |
| `make apple-sim`, `make apple-tv-sim` | Build and launch on a simulator (a Mac, or `MAC=user@host`) |
| `make android` | Android TV debug build (the Dev app) in Docker |
| `scripts/rust-dev.sh cargo test --locked` | Server tests in Docker, toolchain pinned |
| `clients/android/build.sh test` | Android unit tests |
| `scripts/test-windows-core.sh` | Windows app shared-rule tests, in Docker (Windows builds: [docs/dev/windows.md](docs/dev/windows.md)) |
| `scripts/api-lint.sh` | Validate `api/openapi.yaml` and its examples (Redocly CLI in Docker) |
| `scripts/test-apple.sh` | Apple unit tests on the build Mac (`ONLY=<Suite>`) |

Toolchains and the build Mac: [docs/building.md](docs/building.md).

## Rules

- **Every bug fix ships with a regression test** that fails without the fix, and a row in the
  regression map ([docs/dev/testing.md](docs/dev/testing.md)). Move logic into a testable function
  if it isn't one.
- **Refactors change no behaviour, and prove it**: same tests, same output (playlists, segments,
  JSON, screenshots) before and after. Keep refactors and features in separate pull requests.
- **Run `make check` before a pull request**, and say in it that it passed or what was skipped.
  It enforces rustfmt on the server: run `scripts/rust-dev.sh cargo fmt` after editing Rust.
  Never push to `main`; never skip the hooks with `--no-verify`.
- **Old clients keep working**: new stream features are asked for by a flag and announced as a
  capability; without the flag the output is byte-identical
  ([ADR 0002](docs/adr/0002-capability-gated-features.md)). JSON answers only gain fields.
  Migrations stay harmless to older servers. A new or changed route goes into
  [api/openapi.yaml](api/openapi.yaml) in the same pull request (a server test fails otherwise).
- **Shared client rules stay alike**: segment, track, version and Continue Watching rules exist in
  both clients with tests; change both ([docs/dev/adding-a-client.md](docs/dev/adding-a-client.md)).
- **Privacy**: no personal details in the tree (names, hosts, addresses, device ids, library
  contents) and no secrets. Personal settings go in the git-ignored places (`scripts/mac.env`,
  `clients/apple/Config/Signing.local.xcconfig`, `server/deploy/local/`, `scripts/local/`). Add your
  own patterns to `~/.config/moonrise/private-patterns.txt` (every clone of yours reads it) or to
  the git-ignored `scripts/local/private-patterns.txt`, never to the tracked list; the pre-push
  hook and `scripts/export-public.sh` refuse anything matching them, say so when the user file is
  missing, and fail when `MOONRISE_PRIVATE_PATTERNS` names one that isn't there. Test values that
  look like keys must be obviously fake.
- **Test on devices only with the Dev app**: `DEV=1` builds Moonrise Dev, a separate app with its
  own id, sign-in and amber icon (`DEV=1 scripts/build.sh sim && DEV=1 scripts/sim.sh`,
  `DEV=1 scripts/deploy-tv.sh`; Android's debug build). Never install over, uninstall or sign out
  the app someone watches with. Scripted runs use `MOONRISE_NO_REPORT=1` so they don't change
  watch history, and anything that writes watch state goes to a throwaway server.
- **No comparisons with other players**, in code, docs, commit messages or UI text.
- **No internal ticket numbers** in the repository or its history: code, comments, docs,
  CHANGELOG, test names, commit messages, pull request titles and bodies, and branch names. The
  maintainers' tracker is private, so say what a change does in words; the link between a ticket
  and its pull request lives in the tracker's comments. `scripts/tests/no-ticket-refs.test.sh`
  (in `make check-quick`) fails on a tracked file that has one.
- **No analytics**, and no new dependencies without a reason.
- New UI is checked on iPhone, iPad, Mac and Apple TV, and lines up with the existing rows and
  margins.

## Where to read more

- [docs/dev/architecture.md](docs/dev/architecture.md): modules, clients, data flow
- [api/openapi.yaml](api/openapi.yaml): the API contract (client, setup and admin)
- [docs/dev/server.md](docs/dev/server.md): server internals and the full API
- [docs/dev/testing.md](docs/dev/testing.md): test locations, rules, regression map
- [docs/adr/](docs/adr/README.md): decisions, and why
- [CONTRIBUTING.md](CONTRIBUTING.md): issues, pull requests, code style
