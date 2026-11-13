# Publishing checklist

What has to happen outside the repository before Moonrise is public. The links and image names
already point to `github.com/paralainer/moonrise` and `ghcr.io/paralainer/moonrise-server`
(`scripts/tests/no-owner-placeholder.test.sh` fails if an `OWNER` placeholder comes back); a fork
changes them to its own account.

## Source

1. **The history.** `scripts/export-public.sh <dir>` scans the committed tree (gitleaks and the
   private-detail patterns, including your own, see CONTRIBUTING.md) and makes a clean copy with a
   fresh one-commit history; it fails on any finding, and also when
   `server/deploy/local/builtin-keys.sh` exists but can't give every built-in key to search for.
   The tree scan doesn't look at earlier commits, commit messages or committer identities: before
   making a repository with history public, scan those too (gitleaks over the whole history, and
   the patterns over every commit), or publish the one-commit copy instead.
2. Make the GitHub repository public (or create it and push the copy), set the description and
   topics, and enable Issues and private vulnerability reporting (SECURITY.md points to it).
3. The Windows build gets libmpv from the project's `deps-libmpv-*` release (`$ProjectRepo` in
   `clients/windows/build.ps1`). If the public repository is a different one, upload the same
   archives to it as releases with the same tags and change `$ProjectRepo`.

## Server image

1. Repository secrets `MOONRISE_BUILTIN_TMDB_KEY`, `MOONRISE_BUILTIN_OPENSUBTITLES_KEY` (the project's
   keys; they are compiled into release images only).
2. Tag `v<version>` (the version in `server/Cargo.toml`, e.g. `v0.33.0`), then run the Server
   workflow by hand on that tag (Actions → Server → Run workflow; it is manual only): it tests,
   builds linux/amd64 + linux/arm64 and pushes `ghcr.io/paralainer/moonrise-server:{0.33.0,0.33,latest}`. Make the package public (GitHub → Packages →
   moonrise-server → Settings → Visibility).
3. Unraid: submit `server/packaging/unraid/moonrise-server.xml` to a Community Applications template
   repository (or host the template in the repo and add its URL).

## App Store

1. App Store Connect → **New App**: platforms iOS + tvOS + macOS (one universal purchase record),
   bundle id = `MOONRISE_BUNDLE_ID`, SKU, primary language. Name "Moonrise" may be taken: have an
   alternative ("Moonrise Player", "Moonrise for Home Media").
2. **Privacy**: App Privacy → "Data Not Collected" (see docs/privacy.md); privacy policy URL (a
   page in the repository, e.g. docs/privacy.md on GitHub, is fine).
3. **Age rating** questionnaire: no objectionable content in the app itself (a media player; user
   content): typically 4+/9+ — note "unrestricted web access: no".
4. **Export compliance**: `ITSAppUsesNonExemptEncryption = false` is already in Info.plist
   (standard HTTPS only).
5. **App Review information**: notes from docs/demo.md ("App Review notes"), no demo account needed.
6. Screenshots (6.9" iPhone, 13" iPad, Apple TV, Mac) — the demo makes good ones; description,
   keywords, support URL (GitHub issues), marketing URL (the repository).
7. `scripts/testflight.sh ios|tvos` uploads a build once the record exists (macOS: archive in
   Xcode → Distribute). Internal TestFlight first, then submit.
8. The server's setup page links to the App Store once there is a link:
   `APP_STORE_URL` in `server/src/admin/claim.rs`, and the README's Status and Quick start.

## Google Play (Android TV and phone)

1. Decide the application ids for good: `app.moonrise.tv` and `app.moonrise.phone` today
   (`clients/android/*/build.gradle.kts`). Play never lets an app change its id.
2. Make the release key (clients/android/README.md, "Release build and signing") and keep it
   backed up; with Play App Signing it becomes the upload key.
3. Play Console: one listing per app (a TV app needs the Android TV form factor and its banner),
   Data safety ("no data collected", as docs/privacy.md), content rating, screenshots. Play wants
   an Android App Bundle: `clients/android/build.sh :tv:bundleRelease` (and `:phone:bundleRelease`).
4. The GPL-3.0 of `media3-ffmpeg-decoder` applies to the built apps (README, License): the
   listing or the app links to the source.

## Windows

1. The installer is unsigned: Windows SmartScreen warns on first run. A code-signing certificate
   (or the Microsoft Store) removes the warning; `build.ps1` has no signing step yet.
2. Attach `Moonrise-Setup-<version>.exe` from `clients\windows\build\installer\` to a GitHub
   release, with THIRD-PARTY-NOTICES.txt (it is also installed with the app).
