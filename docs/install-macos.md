# The Moonrise server on a Mac

A Mac mini (or any Mac that stays on) makes a good home server: quiet, frugal, and its media
engine transcodes in hardware. The server runs **natively** on macOS, not in Docker: Docker on a
Mac runs a Linux VM, which can't use VideoToolbox. Apple silicon and Intel Macs, macOS 12 or later.

## Install

1. Get the server: the release archive `moonrise-server-<version>-macos.tar.gz` (one universal binary
   for Apple silicon and Intel), or build it yourself on a Mac with Rust:
   `server/packaging/macos/build.sh` (makes the same archive in `server/target/macos/`).
2. Unpack it and run the installer **in Terminal, as the user the server should run as**:

   ```sh
   tar xzf moonrise-server-*-macos.tar.gz && cd moonrise-server-*-macos
   ./install.sh
   ```

   It prints the server's address and the **6-digit setup code**. Open the Moonrise app on your
   iPhone, iPad, Mac or Apple TV → *Connect to Your Server* → pick the new server → enter the code.
   The app walks you through the rest (account, video folders or network shares, first scan).

Options: `--ffmpeg /path/to/ffmpeg` (use that one), `--download-ffmpeg` (always fetch the static
build below), `--port 8097`, `--at-boot` (see "Starting"). Run `./install.sh --help` for all.

### ffmpeg

The server needs ffmpeg with VideoToolbox. The installer uses, in this order:

1. one installed earlier into the server's folder;
2. **Homebrew's** (`/opt/homebrew/bin/ffmpeg` or `/usr/local/bin/ffmpeg`) if it has VideoToolbox;
3. otherwise it downloads a **static ffmpeg 9.0.2** build (arm64 or x86_64, built, signed and
   notarized by [martin-riedl.de](https://ffmpeg.martin-riedl.de); VideoToolbox, libx264, zimg),
   pinned by version and SHA-256 checksum, into the server's folder.

Homebrew's ffmpeg works well on Apple silicon. On **Intel Macs Homebrew no longer has bottles**
(since September 2026), so `brew install ffmpeg` compiles it and about 25 dependencies from source,
which takes a long time; the static build is the easy way there. Homebrew's ffmpeg has no `zimg`,
so HDR can only be tone-mapped by VideoToolbox itself (which every supported Mac does; see below).

Licensing: those ffmpeg builds are GPL (version 3), which the server's AGPL-3.0 allows using
together. Moonrise doesn't ship ffmpeg; the installer fetches it from its builder, who publishes the
build configuration.

## What goes where

| | |
|---|---|
| `~/Applications/Moonrise Server/` | the program: `moonrise-server`, `uninstall.sh`, and ffmpeg/ffprobe if downloaded |
| `~/Library/Application Support/Moonrise Server/` | the data: database (accounts, watch history, settings), subtitles, artwork. **Back this up.** |
| `~/Library/Caches/Moonrise Server/` | stream cache (safe to delete) |
| `~/Library/Logs/Moonrise Server/server.log` | the log (kept under 10 MB; the previous one is `server.log.1`). Console.app shows it too. |
| `~/Library/LaunchAgents/org.moonrise.server.plist` | the service (or `/Library/LaunchDaemons/` with `--at-boot`) |

The server runs as you, not as root, and reads only what you can read.

## Starting

- **Default: a LaunchAgent.** The server starts when you log in to the Mac and keeps running
  (restarted within seconds if it ever stops; *Restart Server* in the app works the same way). It
  runs in your desktop session, so macOS's privacy prompts (below) appear on the Mac's screen.
  Most home servers log in automatically (System Settings → Users & Groups → *Automatically log in
  as*), which makes this start at boot too.
- **No one logged in** (a Mac you manage over SSH): the installer notices and starts the server in
  your background session instead; it says so. It won't come back by itself after a restart until
  you log in, so use the next option.
- **`--at-boot`: a LaunchDaemon** that starts at boot, before anyone logs in, still running as you
  (asks for your password once). Choose this for a Mac mini without automatic login. Background
  services can't show privacy prompts: give the server Full Disk Access if it reads protected
  folders (below).

With FileVault on, nothing starts after a restart until someone unlocks the disk by logging in
at the Mac (that also starts the LaunchAgent).

Keep the Mac awake: System Settings → Energy (or Battery → Options) → *Prevent automatic sleeping
when the display is off*. A sleeping Mac can't stream.

If the macOS firewall is on, allow incoming connections for `moonrise-server` when asked (or in
System Settings → Network → Firewall → Options).

## Folders and macOS privacy

macOS asks before a program reads some places. What that means for the server:

| Where the videos are | What happens |
|---|---|
| `~/Movies`, other folders in your home, `/Users/Shared` | Nothing to allow. The app's folder picker starts with these. |
| A **NAS or another computer** | Add it in the app as a **network share**: the server reads SMB itself, nothing is mounted in Finder, nothing to allow. |
| An **external drive** (`/Volumes/…`) | macOS asks "moonrise-server would like to access files on a removable volume": click Allow on the Mac. |
| Desktop, Documents, Downloads | Asked the same way the first time. |
| A share **mounted in Finder** (`/Volumes/…` of a server) | Asked for "network volumes". Prefer adding the share in the app instead. |

With `--at-boot` (or no one logged in), nothing can be asked: reading those places just fails
(the app shows the folder as unreadable). Grant **Full Disk Access** instead: System Settings →
Privacy & Security → Full Disk Access → **+** → press ⌘⇧G and enter
`~/Applications/Moonrise Server/moonrise-server` → turn it on → *Restart Server* in the app. The program is
signed ad hoc, so after an **update** macOS may treat it as new: turn its switch off and on again.
ffmpeg, started by the server, inherits the permission.

## Hardware transcoding

The server finds VideoToolbox by itself at start (Manage Server → Playback & Hardware shows
*Apple VideoToolbox*): decoding (H.264, HEVC incl. 10-bit), scaling (`scale_vt`), **HDR→SDR
tone-mapping** (VideoToolbox's pixel transfer, `scale_vt` with BT.709 output) and encoding
(`h264_videotoolbox`; `hevc_videotoolbox` Main 10 keeps HDR for HDR screens) all on the media
engine. DivX, AV1 and others are decoded on the CPU and uploaded. `MOONRISE_HWACCEL=videotoolbox|cpu`
or *Transcode With* picks one.

Measured (2-minute 1080p/4K test clips, the server's own command lines, segments fetched back to
back):

| | Apple M4 (MacBook Air, macOS 27) | Intel i7-8559U (Mac mini 2018 class, macOS 15) |
|---|---|---|
| 1080p H.264 → 720p | 16× real time | 14× |
| 1080p H.264 → 1080p | 12× | 12× |
| 4K H.264 → 1080p | 6.4× | 6.6× |
| 4K HEVC 10-bit HDR → 4K SDR (tone-mapped) | 3.2× | 2.3× |
| 4K HEVC 10-bit HDR → 1080p SDR | 6.5–8× | 2.5× |
| DivX (MPEG-4 ASP, SD) → H.264 | 48× | 21× |
| Direct play / remux (no transcode) | 60–90× | 100×+ |

Intel Macs' encoder (Quick Sync) takes nothing narrower than 640 pixels: such small pictures are
encoded with libx264 (cheap at that size); the server finds this out by itself.

## Update and uninstall

- **Update:** unpack the new archive and run its `install.sh`. It replaces the program and
  restarts the server; data and settings stay.
- **Uninstall:** `~/Applications/Moonrise\ Server/uninstall.sh` removes the service and the program
  and keeps your data; `uninstall.sh --all` deletes the data, cache and logs too. Homebrew's
  ffmpeg stays (`brew uninstall ffmpeg`).

## Later: a menu-bar app

A small signed *Moonrise Server* menu-bar app is planned (optional): it would show the status and the
setup code, start and stop the server, and hold the folder permissions itself (a signed app keeps
Full Disk Access across updates). Until then the scripts above are the way.
