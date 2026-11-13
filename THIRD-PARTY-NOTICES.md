# Third-party notices

Moonrise's own code is under the licences in the [README](README.md#license). This page lists the
third-party code each part of Moonrise uses or bundles, and its licence. Each project's licence
applies to that project, not to Moonrise.

## Server (`server/`)

- **Rust crates**, compiled into the binary, at the versions in `server/Cargo.lock`. All are under
  permissive licences compatible with the AGPL-3.0: MIT, Apache-2.0 (most are dual MIT OR
  Apache-2.0), BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unlicense, BSL-1.0, Unicode-3.0 (the ICU
  crates) and CDLA-Permissive-2.0 (`webpki-roots`, Mozilla's root certificates). To list them,
  run `scripts/rust-dev.sh cargo metadata --format-version 1` and read each package's `license`.
- **SQLite**, bundled through `rusqlite`: public domain.
- **smb2** 0.42.0, vendored with one change in `server/third_party/smb2/` (see its
  `MOONRISE-PATCH.txt`): MIT OR Apache-2.0, texts in that folder.
- **ffmpeg and ffprobe** are run as separate programs, not linked. The Docker image includes
  [jellyfin-ffmpeg](https://github.com/jellyfin/jellyfin-ffmpeg) on amd64 and Debian's `ffmpeg`
  packages on arm64, both built with GPL parts (libx264 and others), under the GPL; their sources
  are at those projects. The macOS installer uses an ffmpeg already there (Homebrew's) or
  downloads a static GPL build from [martin-riedl.de](https://ffmpeg.martin-riedl.de), pinned by
  checksum ([docs/install-macos.md](docs/install-macos.md)).
- **Debian** (the image's base system): each package's licence is in the image under
  `/usr/share/doc/<package>/copyright`.

## Apple apps (`clients/apple/`)

- No third-party code is compiled in by default; the apps use Apple's frameworks only.
- **TailscaleKit**, only when you build it (`scripts/build-tailscalekit.sh`), from
  [libtailscale](https://github.com/tailscale/libtailscale): BSD-3-Clause. Not in the tvOS app.
- **The demo's films and artwork** (Big Buck Bunny, Elephants Dream, Sintel, Tears of Steel) are by
  the Blender Foundation under Creative Commons licences; see [docs/demo.md](docs/demo.md).

## Android apps (`clients/android/`)

At the versions in `clients/android/gradle/libs.versions.toml`:

- **AndroidX** (Media3 / ExoPlayer, Compose, Compose for TV, Activity, Lifecycle, TV Provider,
  WorkManager), **Kotlin** and **kotlinx** (coroutines, serialization), **OkHttp**, **Coil** and
  **ZXing**: Apache-2.0.
- **media3-ffmpeg-decoder** (`org.jellyfin.media3`), software audio decoders built from FFmpeg:
  GPL-3.0. A built Android app as a whole is therefore distributed under the GPL-3.0; its source
  stays MPL-2.0 (see the README's License).
- JUnit (tests only, not in the apps): EPL-1.0.

## Windows app (`clients/windows/`)

The full list ships with the app as
[`clients/windows/THIRD-PARTY-NOTICES.txt`](clients/windows/THIRD-PARTY-NOTICES.txt): libmpv
(LGPL-2.1-or-later, built without GPL parts, with FFmpeg under the LGPL), .NET and the Windows
App SDK (Microsoft, MIT and the Windows App SDK licence terms). Why and how libmpv is bundled:
[ADR 0014](docs/adr/0014-windows-app.md). The tests use xUnit (Apache-2.0) and the .NET test SDK
(MIT), not shipped.

## Artwork

The Moonrise name, icon and logo are covered by [TRADEMARKS.md](TRADEMARKS.md), not by the code
licences. The artwork sources and how the icons are made: [assets/brand/README.md](assets/brand/README.md).
The apps' two illustrations (the welcome screen's theatre, the empty Downloads screen) are
Moonrise's own, generated from text prompts with the open MOSS-Image model (Apache-2.0); their
origin is in the same README.
