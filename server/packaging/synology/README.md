# Moonrise server on Synology (Container Manager)

DSM 7.2 or later with **Container Manager** (Package Center). Intel-based models with a
graphics core (most "+" models with Celeron J/N CPUs) can transcode with Quick Sync; others use
the CPU (fine when your devices play the files as they are, which Moonrise does whenever it can).

> **The image `ghcr.io/paralainer/moonrise-server` isn't published yet** (Moonrise is pre-release).
> Until it is, build the server from source with the [Quick start](../../../README.md#quick-start) in the README, or build
> the image yourself (`docker build -t moonrise-server:local server` in a clone) and use
> `moonrise-server:local` as the image name below.

1. **File Station**: create a folder for the server's state, e.g. `docker/moonrise-server`
   (`/volume1/docker/moonrise-server`).
2. **Container Manager → Project → Create**: name `moonrise-server`, path the folder above, source
   *Create docker-compose.yml*, and paste [`docker-compose.yml`](docker-compose.yml) from this
   folder.
   - Your videos on this NAS: keep the `/volume1/video:/media:ro` line (change the shared folder
     name to yours). Videos on another machine: delete it and add the share in the app.
   - Intel GPU: keep the `devices` lines. If `/dev/dri` doesn't exist on your model, delete
     them (the server then transcodes on the CPU).
3. **Next → Done**. The project starts; open its container's **Log** to see the
   **setup code** (or open `http://<nas>:8097/` from a computer on the same network).
4. Open the Moonrise app on your iPhone, iPad, Mac or Apple TV: **Connect to Your Server** → the NAS
   shows as "New" → enter the setup code → create your account → pick your video folder.

Notes

- Host networking lets the apps find the NAS by itself. If you change it to bridge networking,
  map port 8097 and type the NAS's address in the app.
- Updating: Container Manager → Project → moonrise-server → Action → **Build** (pulls the new image)
  or Image → moonrise-server → Update.
- Back up `/volume1/docker/moonrise-server` (Hyper Backup): it holds accounts, watch history and
  settings. `data/cache/` inside it can be left out.
