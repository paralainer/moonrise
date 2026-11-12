# Moonrise server on TrueNAS SCALE

TrueNAS SCALE 24.10 (Electric Eel) or later runs apps with Docker and can install one from a
compose file: **Apps → Discover Apps → ⋮ → Install via YAML**.

> **The image `ghcr.io/paralainer/moonrise-server` isn't published yet** (Moonrise is pre-release).
> Until it is, build the server from source with the [Quick start](../../../README.md#quick-start) in the README, or build
> the image yourself (`docker build -t moonrise-server:local server` in a clone) and use
> `moonrise-server:local` as the image name below.

1. **Datasets**: create one for the server's state, e.g. `tank/apps/moonrise-server`. The container
   runs as root; nothing else needs permissions. Your media dataset is mounted read-only.
2. **Apps → Discover Apps → ⋮ → Install via YAML**, name `moonrise-server`, and paste:

```yaml
services:
  moonrise-server:
    image: ghcr.io/paralainer/moonrise-server:latest
    container_name: moonrise-server
    restart: unless-stopped
    network_mode: host
    environment:
      MOONRISE_LISTEN: 0.0.0.0:8097
      MOONRISE_CACHE_DIR: /data/cache
      MOONRISE_RESTART: exit
      # NVIDIA (TrueNAS installs the driver when "Install NVIDIA Drivers" is on in
      # Apps → Configuration → Settings):
      # NVIDIA_VISIBLE_DEVICES: all
      # NVIDIA_DRIVER_CAPABILITIES: compute,video,utility
    # runtime: nvidia
    # Intel / AMD GPU:
    # devices:
    #   - /dev/dri:/dev/dri
    volumes:
      - /mnt/tank/apps/moonrise-server:/data
      - /mnt/tank/media:/media:ro
```

   Change the two `/mnt/tank/…` paths to your datasets.
3. **Save**. When the app is *Running*, open its logs (**Apps → moonrise-server → Workloads → Logs**)
   for the **setup code**, or browse to `http://<truenas>:8097/` from your network.
4. Open the Moonrise app: **Connect to Your Server** → "New" → setup code → account → library.

Older SCALE releases (Dragonfish 24.04 and before, Kubernetes apps): use **Custom App** with the
image `ghcr.io/paralainer/moonrise-server`, *Host Network* on, environment `MOONRISE_CACHE_DIR=/data/cache`,
`MOONRISE_RESTART=exit`, a host path for `/data`, and your media at `/media` (read-only).
