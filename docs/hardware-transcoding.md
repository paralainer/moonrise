# Hardware transcoding

The server streams what an Apple device can play as it is (no re-encoding: H.264 and HEVC video,
AAC/AC-3/E-AC-3 audio; other audio is converted on the CPU, which is cheap). It **transcodes
video** only for: a lower quality (the player's quality menu, cellular), codecs Apple devices
can't play (MPEG-4 Part 2/DivX, MPEG-2, VC-1, AV1 on older devices, 10-bit H.264), HDR on an SDR
screen (tone-mapping), and picture subtitles (PGS, DVD) burned in.

## What it uses

At start the server looks for, in order (the log says `transcoding with …`, and the apps show it
in **Manage Server → Playback & Hardware**):

| Backend | Found when | Decoding | Scaling / HDR → SDR | Encoding |
|---|---|---|---|---|
| **NVIDIA** (`nvidia`) | the NVIDIA runtime passes the GPU in (`nvidia-smi` works) | NVDEC (CPU for DivX and 10-bit H.264) | `scale_cuda` / `tonemap_cuda` | `h264_nvenc`, `hevc_nvenc` (HDR kept for HDR screens) |
| **Intel Quick Sync** (`qsv`) | an Intel `/dev/dri/renderD*` is passed in and a test encode works | VA-API (H.264, HEVC, MPEG-2), else CPU | `scale_qsv` / `vpp_qsv` tone-mapping (else CPU) | `h264_qsv`, `hevc_qsv` when the GPU has Main 10 |
| **VA-API** (`vaapi`) | an Intel or AMD render node and a test encode works | VA-API (H.264, HEVC, MPEG-2), else CPU | `scale_vaapi` / `tonemap_vaapi` (Intel; else CPU) | `h264_vaapi`, `hevc_vaapi` when supported |
| **VideoToolbox** (`videotoolbox`, macOS only) | a test encode works (Apple silicon, Intel Macs) | VideoToolbox (H.264, HEVC incl. 10-bit), else CPU and uploaded | `scale_vt` / `scale_vt` to BT.709 (VideoToolbox's pixel transfer tone-maps) | `h264_videotoolbox`, `hevc_videotoolbox` Main 10 (HDR kept for HDR screens) |
| **V4L2** (`v4l2`, Raspberry Pi 4) | `/dev/video*` passed in and a test encode works | V4L2 (H.264 ≤1080p), else CPU | `scale` / `zscale`+`tonemap` (CPU) | `h264_v4l2m2m`, at most 1080p; 4K and 10-bit HEVC aren't transcoded |
| **CPU** (`cpu`) | always (libx264) | CPU | `scale`, `zscale`+`tonemap` | `libx264 -preset veryfast`, at most 1080p, threads = cores − 1 shared by the transcode slots (single-board computers: `superfast`, at most 720p, no 4K or 10-bit HEVC) |

On macOS the order is VideoToolbox, then the CPU; on Linux NVIDIA, Quick Sync, VA-API, V4L2, CPU.
Each backend is tested once at start with a 5-frame encode (HEVC Main 10 and GPU tone-mapping
separately); what fails is left out and the next option used. The chosen chain is shown with its
notes (for example "HDR is tone-mapped on the CPU").

## Choosing

**Manage Server → Playback & Hardware → Transcode With** (`transcode.hardware`; or
`MOONRISE_HWACCEL=auto|nvidia|qsv|vaapi|videotoolbox|v4l2|cpu` in the environment, which wins). Takes effect at the
next start. A choice that doesn't work falls back to automatic, with the reason shown.
`MOONRISE_HWACCEL_DEVICE=/dev/dri/renderD129` picks a render node when there are several.
**Transcodes at Once** (`performance.transcode_jobs`, default 2) limits concurrent runs.
Switching transcoding off (`features.transcode = false` / `MOONRISE_DISABLE=transcode`) leaves
everything that plays directly playing.

## Docker

```yaml
# NVIDIA (NVIDIA Container Toolkit installed on the host)
    runtime: nvidia
    environment:
      NVIDIA_VISIBLE_DEVICES: all
      NVIDIA_DRIVER_CAPABILITIES: compute,video,utility

# Intel / AMD
    devices:
      - /dev/dri:/dev/dri
```

The image's ffmpeg (jellyfin-ffmpeg) brings its own Intel (iHD, i965) and AMD (radeonsi) VA-API
drivers; the host needs only the kernel driver (i915/xe, amdgpu). The arm64 image uses Debian's
ffmpeg instead (V4L2 M2M for a Raspberry Pi 4: pass `/dev/video10-12` and the `video` group, see
[install-raspberry-pi.md](install-raspberry-pi.md)). On a Mac the server runs natively, not in
Docker ([install-macos.md](install-macos.md)).

## Measured

2-minute test clips (Blender's *Sintel*: 1080p H.264, the same upscaled to 4K H.264 and to 4K HEVC
10-bit HDR10, DivX SD), transcoded by the server with its own command lines, segments fetched back
to back (× real time):

| | 1080p H.264 → 720p | 4K H.264 → 1080p | 4K HDR → 4K SDR | DivX → H.264 |
|---|---|---|---|---|
| NVIDIA RTX 3050 (NVENC) | — | — | 4K encode 5.7× (p1, used for 4K; earlier measurement) | — |
| Intel Quick Sync / VA-API (Linux) | not measured yet | | | |
| VideoToolbox, Apple M4 | 16× | 6.4× | 3.2× | 48× |
| VideoToolbox, Intel i7-8559U (macOS) | 14× | 6.6× | 2.3× | 21× |
| V4L2, Raspberry Pi 4 | not measured (see below) | not transcoded | not transcoded | |
| CPU, Raspberry Pi 4 (720p superfast) | 1.3× (87 % CPU) | not transcoded | not transcoded | 2.4× |
| CPU, Intel i7-8559U (libx264 veryfast) | 5.1× | | | |

The Pi 4's V4L2 encoder couldn't be measured on the test Pi: its Docker runs rootless (which
can't open host devices) and, run natively, the codec's CMA memory was fragmented after weeks of
uptime. The server then fell back to the CPU, as it should.

## Tested

- NVIDIA: the production path (RTX 3050), unchanged command lines (byte-for-byte, checked against
  the previous version for a matrix of sources).
- CPU: real runs in a container without a GPU (DivX AVI, HDR HEVC tone-mapped, 1080p H.264 to
  720p).
- Quick Sync and VA-API: command lines covered by unit tests (`transcode::tests`); they have not
  been run on Intel or AMD hardware yet. Reports welcome.
- VideoToolbox: real runs on an Apple M4 (macOS 27) and an Intel i7-8559U (macOS 15): HDR
  tone-mapping checked against the SDR source (PSNR 43 dB; the CPU `zscale`/Hable path scores 32),
  keyframes on the segment boundaries, playback in the app.
- V4L2: detection and the fallback to the CPU checked on a Raspberry Pi 4; command lines unit-tested.
