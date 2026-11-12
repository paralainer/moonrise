# The Moonrise server on a Raspberry Pi

The image `ghcr.io/paralainer/moonrise-server` has an **arm64** variant for a Raspberry Pi 4 or 5 with a
64-bit OS (Raspberry Pi OS 64-bit, Debian, Ubuntu). It serves what the apps play directly
(H.264, HEVC — 4K and HDR included — with any audio) with almost no CPU, which covers most
libraries. Transcoding is where a Pi is limited; see below.

> **The image `ghcr.io/paralainer/moonrise-server` isn't published yet** (Moonrise is pre-release).
> Until it is, build the server from source with the [Quick start](../README.md#quick-start) in the README, or build
> the image yourself (`docker build -t moonrise-server:local server` in a clone) and use
> `moonrise-server:local` as the image name below.

## Install (Docker)

Docker as most Pis have it (`curl -fsSL https://get.docker.com | sh`). Save this as
`docker-compose.yml` in an empty folder (also in
[server/packaging/raspberry-pi/](../server/packaging/raspberry-pi/docker-compose.yml)):

```yaml
services:
  moonrise-server:
    image: ghcr.io/paralainer/moonrise-server:latest
    container_name: moonrise-server
    restart: unless-stopped
    network_mode: host          # the apps find it on the network
    environment:
      MOONRISE_LISTEN: 0.0.0.0:8097
      MOONRISE_CACHE_DIR: /data/cache
      MOONRISE_RESTART: exit
    # The Pi 4's hardware video encoder and decoder (V4L2):
    devices:
      - /dev/video10:/dev/video10
      - /dev/video11:/dev/video11
      - /dev/video12:/dev/video12
    group_add:
      - video
    mem_limit: 1500m            # leaves room for everything else on the Pi
    volumes:
      - ./moonrise-data:/data
      # videos on this Pi (a USB disk), read-only; a NAS needs nothing here (add it in the app)
      # - /mnt/videos:/media:ro
```

`docker compose up -d`, then `docker logs moonrise-server 2>&1 | grep "setup code"` and continue in
the Moonrise app (*Connect to Your Server* → the new server → the code → account → folder or network
share). Put `moonrise-data` on a USB SSD if you can: the database and artwork like fast storage.

The image is built without emulation (`docker buildx build --platform linux/arm64 server/`): the
server is cross-compiled and Debian's arm64 ffmpeg (which has the V4L2 encoder; jellyfin-ffmpeg's
arm64 build doesn't) is unpacked on the build machine.

## What a Pi can transcode

| | Raspberry Pi 4 | Raspberry Pi 5 |
|---|---|---|
| Hardware encoder | H.264 (V4L2 `h264_v4l2m2m`), up to 1080p | none (the Pi 5 dropped it) |
| Hardware decoder ffmpeg can use | H.264 up to 1080p (V4L2) | none (its HEVC decoder needs Raspberry Pi's own ffmpeg) |
| Transcodes | V4L2: H.264 up to 1080p. CPU (no V4L2): up to 720p, libx264 *superfast* | CPU: up to 720p, libx264 *superfast* |
| 4K, 10-bit HEVC (HDR) | **not transcoded**: they play as they are | not transcoded |

"Not transcoded" is graceful: Apple devices play 4K HEVC and HDR directly, so those titles still
play (on an SDR screen the device tone-maps them itself). Only what *needs* a transcode — a lower
quality over a slow link, or a format Apple devices can't play — is refused for them, and the app
says why ("2160p video is too much to transcode on this server's hardware").

The server picks all of this by itself; Manage Server → Playback & Hardware shows what it found
(*Raspberry Pi V4L2*, or *CPU* with the reason V4L2 wasn't used).

**Measured** on a Raspberry Pi 4 (4 GB) in Docker, CPU path:

| | |
|---|---|
| Direct play / remux (H.264, HEVC, 4K HDR) | 15–33× real time, 10–20 % CPU |
| 1080p H.264 → 720p (libx264 superfast, 3 threads) | **1.3×**, 87 % CPU |
| DivX SD → H.264 | 2.4× |
| Memory | ~180 MB after setup and a scan, ~370 MB peak while transcoding |

Without cooling a Pi 4 throttles under a long transcode (the test Pi reached 82 °C and dropped to
800 MHz, which halves those numbers): use a heatsink or a fan case.

## Troubleshooting V4L2

The log line `transcoding with …` and Playback & Hardware give the reason when V4L2 isn't used:

- *"/dev/video\* can't be opened (permission denied …)"*: the devices aren't passed in (`devices:`
  above), or **Docker runs rootless**: rootless containers can't open host devices (the `video`
  group isn't mapped into them). Use Docker as root (the default install), or run the server
  without Docker.
- *"VIDIOC_REQBUFS failed: Cannot allocate memory"*: the codec's contiguous memory (CMA) is used
  up or fragmented, often after weeks of uptime. A reboot fixes it; a larger pool helps
  (`dtoverlay=vc4-kms-v3d,cma-512` in `/boot/firmware/config.txt`).
- `mem_limit` is ignored under rootless Docker without cgroup memory delegation (`docker run`
  warns "Limitation discarded").
