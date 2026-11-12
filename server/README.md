# Moonrise server

The server the Moonrise apps play from. It runs next to your videos (a Linux box, a NAS, a Mac or a
Raspberry Pi), keeps its own library, metadata and artwork, accounts and watch state, and streams
to the apps on iPhone, iPad, Mac, Apple TV, Android TV, Android phones and tablets, and Windows.
It needs no other media server.

- **Plays almost anything at once**: the original video is streamed as it is (HLS, cut at the
  file's own keyframes), so playback and seeking start in about a second. Audio and video the
  device can't play are converted on the fly, on a GPU when there is one.
- **Every audio track and subtitle**, embedded, next to the file, or found on OpenSubtitles.
- **Downloads** for watching offline, as the original file or a smaller copy made for the trip.
- **One library for everything**: movies and shows can share a folder; the server works out what
  each file is. Local folders and network shares (SMB), no mounts needed.
- **Set up and managed in the app**: no web admin, nothing to edit by hand.

How it works inside, and every API: [docs/dev/server.md](../docs/dev/server.md).

## Getting started

1. **Run the server** next to your videos: `docker compose up -d --build` in this folder of a
   clone, with [`docker-compose.yml`](docker-compose.yml), which builds the image from source
   (host networking, one folder `./moonrise-data`;
   mount local media read-only, or leave it out if the videos are on a NAS: the app adds the
   share). Other ways to run it are under [Install](#install).
2. **Install Moonrise** on a phone, tablet, Mac, PC or TV (building the apps:
   [docs/building.md](../docs/building.md)) and open it: **Connect to Your Server**. The app finds the server on the network (Bonjour, else a sweep of the local
   network), or you type its address. A new server shows as "New, not set up".
3. **Set Up This Server** and enter the **6-digit setup code**. The server prints it in its log
   (`docker logs moonrise-server 2>&1 | grep "setup code"`) and shows it on its page,
   `http://<server>:8097/`, to browsers on the local network. It changes when the server restarts;
   5 wrong codes from one address lock that address out for 15 minutes, and after 10 wrong codes
   in all the server makes (and logs) a new one.
4. **Create the admin account** (user name, password of at least 8 characters). It's signed in on
   this device, and the server is now *claimed*: the code is gone, and `/` just says it's set up.
5. **Add your first library**: a name ("Media") and its folder, either **a folder on the server**
   (browsed in the app) or **a network share** (file servers on the network are listed; a user
   name and password are asked for only when the share turns guests away). Movies and shows can
   share the folder. The app shows the first scan live.
6. **Extras** (optional, skippable): your own TMDB key, an OpenSubtitles account, an LLM key. Each
   says what it adds and what happens without it (see [Optional services](#optional-services)).

Other devices sign in with that account (on a TV: **Sign In with a Code**,
approved on the phone or at `http://<server>:8097/link`). Without the app,
`docker exec -i moonrise-server moonrise-server user add <name> --admin` (password on stdin)
creates an admin and claims the server just the same.

## Install

| Where | How |
|---|---|
| Linux with Docker | [`docker-compose.yml`](docker-compose.yml): host networking (so the apps find it with Bonjour), `restart: unless-stopped`, GPU blocks commented out (NVIDIA runtime, or `/dev/dri` for Intel and AMD) |
| Unraid | [`packaging/unraid/`](packaging/unraid/) template |
| Synology | [`packaging/synology/`](packaging/synology/README.md) |
| TrueNAS SCALE | [`packaging/truenas/`](packaging/truenas/README.md) |
| Raspberry Pi 4 / 5 | [docs/install-raspberry-pi.md](../docs/install-raspberry-pi.md) |
| A Mac (native, for VideoToolbox) | [docs/install-macos.md](../docs/install-macos.md) |
| From source | `make server-run` at the top of the repository ([docs/building.md](../docs/building.md)) |

The image is built for linux/amd64 and linux/arm64. Its ffmpeg is the jellyfin-ffmpeg package
(NVENC/NVDEC and CUDA filters, Quick Sync, VA-API on Intel and AMD, libx264). Which GPU is used
and how fast it is: [docs/hardware-transcoding.md](../docs/hardware-transcoding.md).

With `docker run -p 8097:8097` instead of host networking the server works the same, but Bonjour
can't announce it: the apps find it by sweeping the network, or you type the address. The setup
page address in its log is then the container's own (`172.17.x.x`); use the machine's address.

**Updating**: pull the new image and start the container again (`docker compose pull && docker
compose up -d`). The database is migrated at start.

## Settings

Everything is set in the app (**Settings → Server → Manage Server**, admins only) and kept in the
server's database. Environment variables still work and win over what the app stores. The full
list, with defaults: [docs/server-settings.md](../docs/server-settings.md). The ones you may want
in the compose file:

| Variable | What it does |
|---|---|
| `MOONRISE_LISTEN` | Address and port (`0.0.0.0:8097`) |
| `MOONRISE_DATA_DIR`, `MOONRISE_CACHE_DIR` | State and cache folders (`/data`, `/cache`; the compose example puts the cache in `/data/cache`) |
| `MOONRISE_RESTART=exit` | Lets the app's Restart Server exit the server for Docker to start again |
| `MOONRISE_HWACCEL` | Transcoding hardware: `auto`, `nvidia`, `qsv`, `vaapi`, `videotoolbox`, `v4l2`, `cpu` |
| `MOONRISE_DISABLE` | Switches features off, comma-separated (e.g. `transcode,mdns`) |
| `MOONRISE_MDNS_INTERFACES` | The network interfaces Bonjour announces on (default: the LAN ones) |
| `RUST_LOG` | Log level (`moonrise_server=info`) |

Network shares: [docs/smb.md](../docs/smb.md). Remote access: [docs/remote-access.md](../docs/remote-access.md).

## Optional services

Moonrise needs nothing but media folders. Each service makes it better and has a fallback; release
builds carry Moonrise's own TMDB and OpenSubtitles keys, so a new server asks for no key at all. An
admin's own key or account wins over the built-in one.

| Service | With it | Without it |
|---|---|---|
| TMDB (built-in key; your own optional) | Posters, backdrops, plots, cast, genres, ratings, episode names | Titles and years from file and folder names, NFO files and the files' own tags; artwork from the folders, cover art inside the files, or a frame of the video |
| OpenSubtitles (built-in key; your account optional) | Find Subtitles in the player: 5 downloads a day, 20 with a free account, more with VIP | Find Subtitles is hidden; subtitles inside and next to the files still play |
| LLM (Anthropic, OpenAI, a ChatGPT plan, Gemini, OpenRouter, Ollama, LM Studio, llama.cpp, any OpenAI-compatible endpoint) | Unclear or transliterated file names matched by a model, every answer checked | Matched by name, year and runtime; the rest wait in Fix Match with their likely candidates |
| Ratings (IMDb's public dataset, no key; an OMDb key for more) | IMDb, and with OMDb Rotten Tomatoes and Metacritic, on title pages | TMDB's rating |
| Remote access (Tailscale) | The apps reach the server from anywhere, nothing opened to the internet | Home network only; downloads play anywhere |

What is sent where: [docs/privacy.md](../docs/privacy.md).

## Command line

Inside the container (`docker exec -it moonrise-server moonrise-server …`):

| Command | What it does |
|---|---|
| `user add <name> [--admin]` | Creates an account (password on stdin); the first one claims a new server |
| `user passwd <name>`, `user list`, `user revoke <name>`, `user delete <name> --yes` | Manage accounts and sign devices out |
| `users reset-data [--user <name>] --yes` | Forgets watch state, sessions, tokens and device codes (users stay) |
| `scan [--full]`, `stats [--recent <hours>]` | Scan the libraries now; library and usage numbers |
| `llm-test` | One call to the configured LLM, to check the setup |
| `smb-credential add --user <u> [--host <h>]` | Stores a share password in the running server (on stdin) and prints its id |
| `migrate-root <old> <new> [--credential <id>] [--apply]` | Moves a library between a mounted folder and a share without losing watch state (server stopped) |
| `healthcheck` | The image's health check |

## Data and backups

Everything lives in the data folder (`/data`, `./moonrise-data` in the compose example):

- `moonrise.db`: settings (including keys and share passwords, so it's readable by its owner
  only), accounts, watch history and the library. Back it up.
- `subs/`: subtitles, including those fetched from OpenSubtitles (fetching them again uses your
  download quota). Back it up.
- `images/`: artwork, fetched again when missing.
- `cache/` (with the compose example): segments, prepared downloads and scrubbing thumbnails. It
  can be left out of backups and deleted at any time.

## Checking it

- `curl http://<server>:8097/health` answers `ok`; `/v1/info` gives the name, version and features.
- From a Mac on the network: `dns-sd -B _moonrise._tcp` lists the servers Bonjour announces.
- Manage Server → Activity and Logs in the app show what plays, background work, and the last
  warnings and errors (keys and passwords are taken out).

## License

AGPL-3.0-or-later ([LICENSE](LICENSE)). The apps, the API description and the tooling are MPL-2.0
(see the [root README](../README.md#license)). The vendored SMB client in `third_party/smb2` keeps
its MIT/Apache-2.0 license.
