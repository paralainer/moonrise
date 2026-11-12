# Adding a client

A Moonrise client is a player for one server's HTTP API. Three exist: the Apple apps
(`clients/apple/`, Swift, `Server/MoonriseClient.swift`), Android (`clients/android/`, Kotlin) and
Windows (`clients/windows/`, C#). Use them as references: `clients/android/core` is the easiest to read, because it's the
whole API and the shared rules in plain Kotlin with no UI (`MoonriseApi.kt` and its tests).

The client API is described in [api/openapi.yaml](../../api/openapi.yaml) (OpenAPI 3.1: every
endpoint, its parameters and the fields of its answers, with example answers in
[api/examples/](../../api/examples/)); a server test fails when a route is missing from it, and
each client has a test decoding every example with its models (a new client should too). How
each part works inside the server is in [server.md](server.md). This page is the order to build
them in, and the rules a client must keep.

## Where it goes

- A new client gets its own folder under `clients/` (`clients/web/`, …), self-contained: its own
  build, its own README, nothing outside the folder referring into it.
- Another Android form factor is a sibling module of `clients/android/tv` and
  `clients/android/phone` (which also serves tablets), sharing `clients/android/core` and
  `clients/android/shared`.
- Add its unit tests to `scripts/test-all.sh` (and so `make check`), and a `make` target if it
  builds with one command.

## The API in the order you need it

All JSON is snake_case; lists are `{items, total}`; times are in seconds; dates are RFC 3339.

1. **Finding the server.** Bonjour `_moonrise._tcp` (TXT `id`, `version`, `path=/v1/info`,
   `setup=unclaimed` while new), else a sweep of the local /24 on port 8097 with
   `GET /v1/info`, else an address typed in. `GET /health` answers exactly `ok`.
2. **What it can do.** `GET /v1/info` (no token): `server_id`, `name`, `version`,
   `capabilities[]`, `setup`. **Check a capability before using the feature it names**
   (`trickplay`, `lossless-audio`, `subtitle-search`, `subtitle-windows`, `device-code`, …), so
   the client works with older servers, and older servers with it.
3. **Signing in.** `POST /v1/auth/login {username, password, device_id, device_name}` →
   `{token, user}`. On a TV, the device code instead: `POST /v1/auth/device`, show `user_code`,
   poll `POST /v1/auth/device/poll` every `interval` seconds. Send the token only as
   `Authorization: Bearer <token>`. `GET /v1/me`, `POST /v1/auth/logout`. Setting up a new server
   (`/v1/setup/verify`, `/v1/setup/claim`) and the admin API (`/v1/admin/…`) are optional for a
   new client.
4. **Browsing.** `GET /v1/home` (`continue`, `latest_types`), `GET /v1/libraries`,
   `GET /v1/items?…` with `GET /v1/facets` for filters, `GET /v1/items/{id}`,
   `GET /v1/shows/{id}/seasons`, `GET /v1/shows/{id}/episodes?season=`,
   `GET /v1/shows/{id}/continue`, `GET /v1/search?q=`. Images: `GET /v1/images/{id}?w=` (no
   token; the id is a content hash, so cache it forever).
5. **Playing.** `GET /v1/items/{id}/playback?hdr=0|1&start=&version=&quality=` → `master` (a
   relative URL to resolve against the server), `session`, tracks, `variants`, `supported` and
   `reasons`, `pending` (ask again shortly). Play the master with the platform's HLS player. The
   stream URLs carry the session id instead of the token. Ask for extras only when the device
   can show them: `dv=1` (Dolby Vision display), `lossless=1` (FLAC in fMP4 and more than 2
   output channels), `trick=1` (an I-frame playlist for the system scrubber), `burn=<index>` for
   a bitmap subtitle. `DELETE /v1/s/{sid}` when playback ends. A `410` on a session URL means the
   session is gone (server restart, expiry): ask `/playback` again at the current position.
6. **Reporting.** `POST /v1/sessions/{sid}/start`, then `progress` every 10 s, then `stop`, with
   `{position, paused, audio, subtitle, credits}`. The server keeps the resume point and marks
   titles played; don't keep watch state on the device. `PUT|DELETE /v1/items/{id}/played`,
   `DELETE /v1/continue/{id}`, `PUT /v1/items/{id}/version {file_id}`.
7. **Segments.** `GET /v1/items/{id}/segments?version=&merged=1` (capability `segments-merged`)
   for the intro, recap and credits to offer, already merged by the server; ask again a few
   seconds later while `community_pending`. On an older server, apply the
   [segment rules](#rules-to-keep) to its answer alone. A client talks to the Moonrise server only
   ([ADR 0013](../adr/0013-the-server-is-the-only-integration-point.md)).
8. **Subtitles.** `GET /v1/items/{id}/subtitles` lists the tracks. Show text subtitles yourself
   from `…/subtitles/{index}/cues?window=<k>` (30 s windows, available before the whole track is
   extracted) or the whole `.srt`; or select the WebVTT renditions in the HLS stream. Optional:
   timing (`…/timing`), OpenSubtitles search and fetch, upload. A version's tracks are addressed
   as `<item id>~<version>`.
9. **Scrubbing thumbnails.** `GET /v1/items/{id}/trickplay` → sheets of `columns` × `rows`
   pictures every `interval` seconds; `GET /v1/trickplay/{key}/{n}.jpg`.
10. **Downloads** (optional): the original (`GET /v1/items/{id}/download`, resumable), a
    prepared MP4 (`POST /v1/items/{id}/download-jobs`), or segmented offline sets
    (`GET /v1/items/{id}/offline`).

## Rules to keep

These are product rules both clients implement alike, each in a small, tested file. A new client
should port them with their tests rather than reinvent them:

| Rule | Apple | Android | Windows |
|---|---|---|---|
| Which intro, recap and credits to offer (the server's `merged=1` answer applies them; a client keeps them for older servers, tested against `api/segment-rules.json`), and when Next Episode shows: a community database's credits start is never moved earlier, and what the file shows only moves it later ([ADR 0005](../adr/0005-end-credits-never-earlier.md)) | `Server/SegmentRules.swift` | `core/…/Segments.kt` | `Moonrise.Core/Segments.cs` |
| When a title offers Resume, and where: any position above 0, watched before or not, tested against `api/resume-rules.json` | `Server/MediaModels.swift` (`resumeSeconds`) | `core/…/Models.kt` (`resumeSeconds`) | `Moonrise.Core/Models.cs` (`ResumeSeconds`) |
| How a playback failure is shown: a short title and one plain line by kind (no answer, still indexing, file not found, share unreadable, damaged, no index, busy, server error, unsupported), the server's own text kept for Details, tested against `api/playback-problem-rules.json`; a title no file of which can play (all damaged, or none found on disk) shows that instead of Play | `Models/PlaybackProblem.swift` | `core/…/PlaybackProblem.kt` | `Moonrise.Core/PlaybackProblem.cs` |
| The starting audio and subtitle tracks: the ones picked last in the title, or like them in the show (the playback answer's `tracks`, reported back with `tracks`), else the preferred language, forced and SDH subtitles | `Player/TrackRules.swift` | `core/…/Tracks.kt` | `Moonrise.Core/Tracks.cs` |
| Which version plays (a healthy file; the viewer's choice; the preferred language) | `Models/VersionChoices.swift` | `core/…/Watch.kt` (`Versions`) | `Moonrise.Core/Watch.cs` |
| Continue Watching cards, focus after a change, and Watch Next | `Server/MediaModels.swift`, `UI/HomeFocus.swift` | `core/…/Watch.kt`, `WatchNext.kt` | Continue Watching: `Moonrise.Core/Models.cs` (`Home`), `Moonrise.App/Pages/HomePage.cs` |
| Retries with backoff, buffering while a track runs dry, restarting at the position | `Network/ConnectionRouter.swift`, `Player/PlayerModel+Recovery.swift`, `Player/PlayerModel+Local.swift` | `core/…/ClientRules.kt` | `Moonrise.Core/ClientRules.cs`, `Moonrise.App/Pages/PlayerPage.cs` (restart) |

Also:
- **Match the existing clients' screens, wording and layout** where the platform allows; the
  Android TV app follows the Apple TV app, the Android phone app the iPhone and iPad app, and the
  Windows app the Mac app.
- **Never compare Moonrise with other players** in the UI, docs or store text.
- **No analytics**, and nothing sent anywhere but the user's own server and the services the
  user turned on ([privacy](../privacy.md)).
- A debug build is a separate **Dev** app (its own id, an amber icon), so testing never touches the
  app someone uses for watching.
