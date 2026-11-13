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

One App Store Connect app for iPhone, iPad, Apple TV and Mac (universal purchase) under your bundle
id (`MOONRISE_BUNDLE_ID` in the git-ignored `clients/apple/Config/Signing.local.xcconfig`), with
the extensions `<id>.widgets` (iOS) and `<id>.topshelf` (tvOS). `scripts/release-apple.sh` does
everything the App Store Connect API allows, on the build Mac, with fastlane
(`clients/apple/fastlane`); `scripts/release-apple.sh --help` lists its commands.

**Once, on the build Mac** (`scripts/mac.env` names the files; nothing secret is in the repository):

- Xcode, XcodeGen and fastlane (Homebrew), and in the login keychain the certificates
  *Apple Distribution* and, for the Mac's installer package, *3rd Party Mac Developer Installer*.
  `MAC_UNLOCK` unlocks the keychain over SSH.
- An App Store Connect API key (App Manager or Admin): `ASC_ENV`, a file exporting `ASC_KEY_ID`,
  `ASC_ISSUER_ID`, `ASC_KEY_PATH` (the .p8) and `APPLE_TEAM_ID`.
- The App Review contact: `REVIEW_ENV`, a file exporting `MOONRISE_REVIEW_FIRST_NAME`,
  `MOONRISE_REVIEW_LAST_NAME`, `MOONRISE_REVIEW_PHONE`, `MOONRISE_REVIEW_EMAIL`.

**The first release:**

1. `scripts/release-apple.sh ids` registers the three bundle ids. No App Groups or iCloud
   containers are used; the Apple TV app and its Top Shelf share a keychain group, which needs
   no registration.
2. On the website (the API can't): App Store Connect → Apps → **New App**: iOS, then add tvOS and
   macOS to the same app; the bundle id, a SKU, English (U.S.). "Moonrise" is taken on the store:
   the listing's name is in `clients/apple/fastlane/metadata/en-US/name.txt`, and the name on the
   device stays Moonrise.
3. Also on the website: **App Privacy** → "Data Not Collected" (see [privacy.md](privacy.md)), the
   **age rating** questionnaire (no objectionable content; unrestricted web access: no), the
   category, and the privacy policy URL.
4. `scripts/release-apple.sh testers` makes the internal TestFlight group "Team" (every build,
   the team's admins; internal testing needs no review).
5. `scripts/release-apple.sh ios` (then `tvos`, `mac`): the next build number (one past the
   platform's latest on TestFlight), App Store profiles (made when missing, named
   `<id> AppStore ios|tvos|macos`), a signed archive with TailscaleKit, and the exported .ipa or
   .pkg. `scripts/release-apple.sh upload ios` has App Store Connect validate it, then uploads it;
   `builds` shows TestFlight's processing (10 to 30 minutes).
6. The listing: `scripts/release-apple.sh check-metadata`, then `metadata` (texts, URLs,
   categories, copyright, App Review notes and contact) and `screenshots`
   (`clients/apple/fastlane/screenshots`: `scripts/store-shots.sh` takes the iPhone, iPad and Apple
   TV ones from the demo on simulators, so they show the Blender Foundation films credited in
   [demo.md](demo.md); the Mac's, 2880 x 1800, are taken by hand on a Mac with
   macOS 26, as `mac/en-US/*.jpg`). `status` shows what App Store Connect has.
7. `scripts/release-apple.sh review` attaches the newest processed build to each platform's version
   with the review notes; `submit ios,tvos,mac --yes` sends them to App Review, with manual release
   once approved. Submitting is the maintainers' decision, never a script's default.

**Lessons from App Review.** A new developer account can get a Mac "2.1 Information Needed"
asking for a screen recording on a physical Mac with the latest macOS, and for the app's purpose,
audience, services and regions: the review notes
(`clients/apple/fastlane/metadata/review_information/notes.txt`) answer the questions up front,
and a recording may still be asked for. Rejection texts appear only in App Store Connect on the web.

Export compliance: `ITSAppUsesNonExemptEncryption = false` is in Info.plist (standard HTTPS only).
The server's setup page links to the App Store once there is a link: `APP_STORE_URL` in
`server/src/admin/claim.rs`, and the README's Status and Quick start.

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
