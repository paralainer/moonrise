# Changelog

Notable changes to Moonrise. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Versioning

The server and the apps have **separate version numbers**.

- **Server**: [Semantic Versioning](https://semver.org/). The version is in `server/Cargo.toml`, and
  a running server reports it in `GET /v1/info` and in Manage Server → About. While it is below 1.0,
  a minor version (0.x.0) adds features and can change the API; a patch version (0.x.y) is for fixes
  and small changes. Either can add a database migration; the entry says so when it does.
- **Apps**: the Apple apps (iPhone, iPad, Mac, Apple TV) share one version, `MARKETING_VERSION` in
  `clients/apple/project.yml`. The Android TV and phone apps each have their own `versionName`, in
  `clients/android/tv/build.gradle.kts` and `clients/android/phone/build.gradle.kts`, and the
  Windows app its `<Version>` in `clients/windows/Moonrise.App/Moonrise.App.csproj`.
  App versions move on their own schedule, independent of the server's.
- **Compatibility**: apps don't compare version numbers. They read the `capabilities` list in
  `GET /v1/info`, and optional fields in responses, to tell what a server offers, and use a feature
  only when it is there.

The numbered entries below are server releases. The apps haven't had a public release yet (they are
at 0.1.0); app changes that shipped together with a server release are listed under it as "Apps".

## [Unreleased]

### Added

- Server 0.33.0 and all apps: picture subtitles (Blu-ray PGS, DVD, DVB) are drawn by the apps over
  the video, which keeps playing as it is. Before, choosing one burned it into a full video
  transcode: on a 2-core server about 7 s to the first segment instead of 0.05-0.09 s, and every
  seek restarted the transcode. The server decodes the track a 30-second window at a time (only
  the subtitle blocks around it are read; mkvmerge's compressed tracks too) and serves each
  picture as a PNG with its place on the video, through the subtitle windows
  (`…/subtitles/{index}/cues?window=k`, capability `subtitle-pictures`). Subtitle delay applies to
  them. Only embedded tracks in these formats are listed. No migration.
  ([ADR 0015](docs/adr/0015-picture-subtitles-drawn-by-the-apps.md))

- `SECURITY.md` (private vulnerability reporting), `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1),
  issue and pull request templates, and this changelog.
- `THIRD-PARTY-NOTICES.md`: the third-party code each part uses or bundles, and its licence.
- The pre-push hook and `scripts/export-public.sh` also read your own private-detail patterns from
  `~/.config/moonrise/private-patterns.txt` (or `MOONRISE_PRIVATE_PATTERNS`), so every clone uses
  them; the tracked list keeps only generic patterns.

### Changed

- `server/docker-compose.yml` builds the server from source (`docker compose up -d --build` in
  `server/`), as no image is published yet. The README is rewritten for a first visit: every
  platform and its state, the quick start, and how to build each app.
- Licences: the server (`server/`) is now under the AGPL-3.0-or-later (`server/LICENSE`); the apps,
  the API description, tooling and docs are under the MPL-2.0 (`LICENSE`). They were GPL-3.0-or-later.
  The name and icon are covered by [TRADEMARKS.md](TRADEMARKS.md), not by the code licences.
- Apps: the Subtitles menu no longer adds "(burned in)" to picture tracks' names.
- The GitHub Actions workflows run only when started by hand. The local check
  (`scripts/test-all.sh`) is the gate for changes; see `CONTRIBUTING.md`.
- The Apple apps' project moved to `clients/apple/`, beside `clients/android/`. A git-ignored
  `Config/Signing.local.xcconfig` or `Vendor/TailscaleKit.xcframework` from an older checkout moves
  with it, by hand (the scripts say how). On a Mac itself the scripts now build in the checkout.
- Android TV: the release app no longer writes its debug and info lines to the system log (focus,
  player steps, sync); warnings and errors still go there. The Dev app logs as before.

### Fixed

- Mac player: a video narrower than the player (a 4:3 episode in a wide window) keeps its scrubber
  and buttons on the picture, not over the black bars beside it, and subtitles stay above the
  scrubber. Playing on the title page (Mac and iPad), Download sits right after the title instead of
  alone at the far end of the column. Mac menus: the hovered row is readable (the system accent, no
  longer a white box).

### Removed

- Server 0.33.0: burning picture subtitles into the video, with the `burn=` playback option, the
  `burn-in` capability, the `burn` switch (`MOONRISE_DISABLE=burn` and a stored `features.burn` are
  ignored) and the variants' `burn` field. The apps draw these subtitles themselves now.

- Server 0.32.0: migration 021 drops the tables of the one-time import in an early version,
  `jf_map` and `jf_subtitles`, unused since 0.27.2 (identification reads `match_hints`, made from
  `jf_map` by migration 017). **This release can't be rolled back to 0.27.1 or older**: they read
  `jf_map`, which a database migrated by 0.32.0 no longer has. 0.27.2 to 0.31.15 run on it as before.

### Fixed

- Server: video Apple devices can't decode is transcoded instead of copied, where before the
  picture could stay black: H.264 lossless encodes (which x264 makes High 4:4:4 Predictive even at
  4:2:0, as screen recorders' lossless settings do), H.264 in RGB, and HEVC's range extensions
  (4:2:2 and 4:4:4 from cameras, 12-bit). H.264 above High is also decoded on the CPU rather than by
  a GPU decoder that can't take it. Ordinary H.264 and HEVC (4:2:0, 8 or 10 bits) is copied exactly
  as before. No migration.
- Server: a new version of a movie or an episode already in the library (another file of it, or a
  replacement under a new name) brings the title back to Home's Recently Added, a library's latest
  and the Date Added sort, as of when the file arrived. Files found by the library's first scan,
  extras, and files that are only renamed, moved or copied over in place move nothing, so the
  existing order stays as it was. No migration.
- Server: a file copied over another in place (`cp`, a file manager's Replace) is picked up
  without waiting for the full scan hours later. A damaged file replaced that way stops showing as
  damaged and plays once it is probed again; a file playback finds changed is probed again too.
- Server: a video with no audio track at all (a screen recording, a camera clip with its sound
  removed, a silent film) plays as video alone, and can be downloaded, instead of being refused
  with "no audio track playable as is". Its master playlist has no audio rendition or audio group.
  A file whose audio can't be played is refused as before; files with audio are served as before.
- Server: a title opened before the library scan has probed its file is planned at its real
  bitrate, from its own probe, instead of 1 Mb/s. Its master playlist no longer advertises a
  `BANDWIDTH` far below the file's rate, and a quality limit below that rate offers a transcode
  within it instead of only the original.
- Server: a file with a large start time (a recording whose clock starts at 1001 s, say) gets
  segments all through and a timeline from 0 to its length, instead of one long segment or a first
  segment as long as the start time. Its copied and converted audio, the start position of a
  session and the cues of its embedded subtitles are on that timeline. Files whose segments change
  get new ones, as does converted audio of any file with a start time; files starting at 0 keep
  their cached segments.
- Server: a file with a large start time (a TV recording, or a remux of one into Matroska that
  keeps its broadcast clock) also has its chapters, its skip segments from them, its scrubbing
  thumbnails and its subtitle timing match on the timeline from 0. Its chapters were listed past
  the title's end (no Skip Intro or Skip Credits, and a film with a credits chapter was never
  marked played), a Matroska file got thumbnails for its last timestamp (ten times too many, or
  pictures 16 s apart and more), and a downloaded subtitle was matched against the file's own
  clock (moved past the end, or left unmatched). Those files' thumbnails and matches are made
  again; files starting at 0 keep theirs.
- Server: the first segments of a copied track in an AVI, FLV or ASF file whose video has B-frames
  start at the file's start, no longer at its second keyframe (up to seconds of audio and video
  missing at the start, whatever the audio codec). Those files' copied segments are remade.
- Server: copied AAC from MPEG-TS and AVI files plays, instead of its audio segments failing with
  "Malformed AAC bitstream".
- Server: a transcode run in an MPEG-TS file, or a file with a non-zero start time, starts on its
  boundary, and no longer a GOP late or with "no keyframe". The segments of those files are remade.
- Server: a transcode run starting at a keyframe directly followed by another (AVI) starts on it,
  and no longer one frame late.
- Android TV and phones: a title watched before and stopped part way offers Resume again, as on the
  Apple and Windows apps; the phone's Resume on a title page starts where it was left, not at 0.
- Android phones: Play on a card for a title with several versions starts the chosen version where
  it was left, or at the beginning, and no longer at the position of another cut.
- Android TV and phones: after switching quality or version and quickly back, a choice the device
  can't decode returns to the one that last played instead of ending on the can't-play screen.
- Android TV and phones: when a stream stops for good, the message now names the failure that ended
  it, and no longer blames the decoder after a later stall.

## [0.31.4] - unreleased

### Added

- `GET /v1/home?watching=1` answers with the watching rows only (`latest_types` and `latest` empty).
  Android TV's background Watch Next sync uses it, instead of downloading every newest-titles row.

## [0.31.3] - unreleased

### Fixed

- Converted audio: a cached segment that exists but can't be read failed the request with an error;
  a new conversion now makes it again.

## [0.31.2] - unreleased

### Fixed

- Converted audio: a request arriving just as its segment was finished could start a needless second
  conversion, which took a slot (stopping another run if none was free) and rewrote the finished
  segment.

## [0.31.1] - unreleased

### Fixed

- Scrubbing pictures for files whose video isn't the first stream (audio first in a Matroska file,
  say): the picture step asked for the file's stream number in its one-stream keyframe copy, ffmpeg
  refused it and the file got no pictures.

## [0.31.0] - unreleased

### Added

- Fix Match's candidate posters come from the server (capability `match-posters`):
  `GET /v1/admin/match/poster?path=&size=` (admins), and `server_posters=1` on `/v1/admin/review`,
  `/v1/admin/items/{id}/match` and `/v1/admin/match/search` gives candidates a `poster_path` in
  place of the TMDB `poster` URL. Without the flag the answers are unchanged.
- Apps: Fix Match's candidates show their posters on Android phones, and on iPhone, iPad and Mac
  they now load from the server rather than from TMDB's image host. On an older server they show
  the placeholder.

## [0.30.0] - unreleased

### Added

- Search before typing (capability `search-start`, migration 020, table
  `user_search_recent`): `GET /v1/search/start` answers `{recent, suggestions}`,
  `PUT /v1/search/recent/{id}` records a title opened from Search, `DELETE /v1/search/recent`
  forgets them, per user and on every device.
- Apps: Search shows Recent and Suggestions before anything is typed, on iPhone, iPad, Mac, Apple
  TV, Android TV and Android phones, on a server that offers them.

## [0.29.0] - unreleased

### Added

- Track memory (capability `track-memory`, migration 019, table `user_file_tracks`):
  playback reports may carry `tracks` (the audio and subtitle tracks playing, `subtitle: null` for
  Off), kept per user and file, and the playback answer gives them back as `tracks` (this file's,
  else another version's, else the latest in the same show). Older apps send none and get the
  extra field only.
- Apps: resuming a film or an episode (Continue Watching, Resume, reopening it) plays the audio
  and subtitle tracks picked in it last, on any device, also a second dub, SDH, a forced track or
  Off; the subtitle Delay comes back with its track. The next episode starts with the same
  language and kind when it has them. Downloads keep their tracks on the device and report them
  with their playback.

## [0.28.1] - unreleased

### Fixed

- `segments?merged=1` no longer offers end credits (Skip Credits, or Next Episode from them) while
  the title still waits for its community lookup (`community_pending`): the file's credits chapter
  could start before a database start not known yet (Guardians of the Galaxy: chapter 6796 s,
  IntroDB 6852 s). Once the answer is in, or known empty, the credits are offered as before. Intro
  and recap are unchanged (the file's own always win). Apps: when the answer wasn't final, they
  ask again on entering the last 10 minutes and once more 2 minutes before the end.

## [0.28.0] - unreleased

### Added

- The server looks up intro, recap and end-credits times in the community databases TheIntroDB
  and IntroDB itself, when a film or an episode is opened or played (never the whole library; a
  request every 2 s at most per database), and keeps them (migration 018, table
  `community_segments`). `GET /v1/items/{id}/segments?merged=1` (capability `segments-merged`)
  answers with the segments the player offers, by the rules the apps applied
  (`api/segment-rules.json`). Switch: `features.community_segments`.

### Changed

- Apps: all third-party lookups go through the server. The apps no longer ask TheIntroDB, IntroDB,
  OMDb or OpenSubtitles from the device: the Settings for a device OMDb key and a device
  OpenSubtitles account are gone (ratings and subtitle search come from the server). With an older
  server, the apps offer the file's own segments only.

## [0.27.2] - 2026-11-09

### Changed

- Identification reads the ids, episode numbers and dates kept from a one-time import in an early
  version from a table of the server's own, `match_hints`, made from the old one (migration 017).
  No match changes. The old table stays for now, so 0.27.1 still runs on the migrated database; a
  later release removes it.
- Apps: the welcome screen no longer mentions source kinds of older app versions.

## [0.27.1] - 2026-11-09

### Fixed

- Scrubbing thumbnails for MP4 and MOV files read each keyframe on its own through the file's index,
  instead of reading most of the file (a 1.5 GB film: about 40 MB read instead of 1.9 GB).
- The thumbnail pass for AVI files can no longer hang, and putting thumbnails together gives way to
  playback.

### Changed

- When the thumbnail storage budget is full, the least recently used titles' thumbnails are removed,
  so a title you open still gets thumbnails.
- Dolby Vision profile 5 files are skipped; the activity view says why.
- Apps: a thumbnails section in Manage Server.

## [0.27.0] - 2026-11-09

### Added

- Scrubbing thumbnails, made in the background: one picture every 10 seconds, at most 2,000 per file,
  one file at a time and at low priority, paused while anything plays, streams or downloads. They are
  kept outside the segment cache under their own size limit (`performance.trickplay_max_gb`, 10 GB by
  default) and can be switched off (`features.trickplay`).
- Thumbnails are served as sheets, and for clients that ask for it as an I-frame stream in the master
  playlist (used by the Apple TV system player). Without that request the playlists are unchanged.
- Apps: thumbnails while scrubbing on iPhone, iPad, Mac, Apple TV and Android TV.

## [0.26.1] - 2026-11-08

### Changed

- Apps report where they offered the credits; the server marks a film watched from the later of that
  point and the credits chapter, and ignores implausible values.
- Chapter names with a leading language tag (such as "en:End Credits") are recognised as credits.
- Apps: a credits chapter never starts before the community database's credits start.

## [0.26.0] - 2026-11-08

### Added

- Extra ratings: the IMDb rating and vote count, from IMDb's non-commercial dataset (downloaded at low
  priority, checked every 6 hours, only for titles in the library).
- An optional OMDb key (`omdb.api_key`) adds the Rotten Tomatoes critics score and Metacritic, within
  OMDb's request limits.
- Items and episodes carry an optional `ratings` field; switch `ratings`. Database migration 016.

## [0.25.0] - 2026-11-08

### Added

- Opt-in per client: lossless 5.1 to 7.1 audio (TrueHD, DTS-HD MA, FLAC, PCM, ALAC) served as 24-bit
  FLAC with every channel kept (`lossless=1`).
- Opt-in per client: Dolby Vision signalled on the original-quality stream for profiles 8.1, 8.4 and
  5 (`dv=1`).
- Without these options the output is byte-identical to 0.24.0. Downloads are unchanged.

## [0.24.0] - 2026-11-08

### Added

- One Continue Watching section: one card per show, film collection or single film, ordered by latest
  activity. Under 5 % watched doesn't count, and entries drop out after 60 days. The next card is the
  very next episode or collection film. `/v1/home` gains `continue`, and `DELETE /v1/continue/{id}`
  removes an entry. Database migration 015.
- Apps: Home reloads when the app comes back to the front; Top Shelf and Watch Next use the same data.

### Changed

- A film with a credits chapter counts as watched from the start of that chapter, not at 90 %.
- Apps: Skip Credits for films only when the credits end before the end of the file.

## [0.23.2] - 2026-11-07

### Added

- End credits detected on the server: a dark stretch in the last 5 minutes, found from keyframes
  only, at low priority, one file at a time, and stored once per file (switch `credits_detect`).
  Database migration 014.
- `/segments` per version, with `pending` while detection hasn't run yet.
- Apps: community credits timestamps checked against the file; Next Episode from the credits or the
  last 30 seconds.

## [0.23.1] - 2026-11-07

### Fixed

- MKV files with a large trailing SeekHead are indexed correctly.
- Damaged MKV files are marked broken with a clear reason, without a full-file scan; background
  indexing skips them, the item API shows `broken` per file, and a healthy version of the title is
  played instead.
- H.264 with short NAL length fields is copied with 4-byte lengths.

## [0.23.0] - 2026-11-07

### Added

- Background keyframe indexing, on by default, with a switch in Manage Server.

### Fixed

- AVI MPEG-4 keyframes are placed at their shown time. Database migration 013 drops the old AVI
  indexes so they are rebuilt.

## [0.22.3] - 2026-11-07

### Fixed

- Sign in with ChatGPT: sturdier retry, the callback parsed like a browser would, clearer logging
  (never of secret values), and old apps told to update.

## [0.22.2] - 2026-11-07

### Fixed

- Sign in with ChatGPT: an unfinished registration is used only for its one automatic retry, a
  Forget endpoint, scrubbed per-step logging, and the provider's error details shown.

## [0.22.0] - 2026-11-07

### Added

- AI matching providers with presets: Anthropic, OpenAI, ChatGPT plan, Gemini, OpenRouter, Ollama,
  LM Studio, llama.cpp, and any other OpenAI-compatible service. Sign in with ChatGPT.
- Keys are bound to a provider and its address (`llm.keys`). Existing settings are migrated
  automatically.

### Security

- Credentialed requests don't follow redirects; TMDB and OpenSubtitles follow same-origin redirects
  only.

## [0.21.4] - 2026-11-07

### Changed

- Automatic subtitle timing: a second pass that also considers the original timing, sturdier
  handling of unclear cases and short tracks, and a tolerance when other-language text tracks agree.
  Earlier matches are redone once.

## [0.21.3] - 2026-11-06

### Security

- Credentials in URLs (user:password, secret query parameters, internal secrets) are removed from
  every log line, from error messages sent to clients and from stored download jobs.

## [0.21.2] - 2026-11-06

### Fixed

- A stop sent before a segment run starts listening is kept, and prefetch tries once, so starting
  playback partway through no longer fails with "no transcode slot free".

## [0.21.1] - 2026-11-06

### Fixed

- Slow audio cuts are no longer stopped early, failed block reads fail together, and memory stays
  near the cache size.

## [0.21.0] - 2026-11-05

### Added

- Automatic timing for downloaded and uploaded subtitles, and a shared Delay per file and track.

## [0.20.2] - 2026-11-05

### Fixed

- SMB reads run on their own, so a paused reader can't hang segment cuts; silent segment runs are
  stopped after 90 seconds.

## Earlier releases

These came before the current repository history and are listed without dates.

- **0.20.1**: video segments are cut by keyframe presentation time.
- **0.20.0**: Fix Match can be undone; titles with several parts or versions.
- **0.19.0**: the project is named Moonrise.
- **0.18.0**: a native macOS server using VideoToolbox; a Raspberry Pi image.
- **0.17.0**: titles in several parts and double episodes; Next Episode.
- **0.16.0**: hardware detection, packaging for NAS systems, a demo mode, docs, workflows and the
  public export script.
- **0.15.0**: libraries without a type.
- **0.14.0**: Manage Server in the apps.
- **0.13.0**: a setup code and app-guided onboarding.
- **0.12.0**: built-in service keys, with a fallback per service.
- **0.11.0**: SMB built into the server.
- **0.10.0**: settings stored in the database; the admin API.

