# Server internals

How the Moonrise server (`server/`, Rust, port 8097) works inside, and its HTTP API. Paths such as
`src/media/segment.rs`, `tools/` and `migrations/` are relative to `server/`. To install and run a server,
see [server/README.md](../../server/README.md) and the [user docs](../); for the big picture,
[architecture.md](architecture.md); for why things are the way they are, the [ADRs](../adr/).

The server has its own library, metadata and artwork, its own accounts and watch state, and the
streams. It needs no other media server. The streams:

- **Instant HLS at any position.** Segments are cut independently at the file's own keyframes
  (read from the Matroska index), by copying, never re-encoding. Nothing restarts on a seek.
- **Every audio track as a native rendition**, so switching doesn't reload the stream. Tracks
  Apple devices can't play (DTS, TrueHD, MP3, Vorbis, Opus, FLAC, PCM…) are converted (below).
  A file with no audio track at all (a screen recording, a clip with its sound removed) plays
  as video alone: no audio rendition and no audio group (0.31.12); one whose audio can't be
  played is refused as before.
- **Text subtitles**, extracted here and kept (below): as SRT files for the app's overlay and
  as WebVTT renditions with correct timing (for PiP and AirPlay).
- **Downloads**: the original file (resumable), or a progressive MP4 prepared here (the MKV's
  original remuxed, or any quality transcoded) that AVPlayer plays offline.
- **Subtitle search** on OpenSubtitles.com (Moonrise's built-in key; anonymous, or with an account);
  fetched subtitles become tracks of the title on every device.
- **Video transcoded** when needed (below), on an NVIDIA, Intel or AMD GPU, else the CPU: a
  quality ladder, codecs Apple devices can't play, HDR tone-mapped for SDR displays, bitmap
  subtitles burned in. Every variant has the copied original's segment timeline.

## Rules for changing the server

- **Old clients get the same bytes.** A new stream feature is asked for by the client (a query
  flag such as `dv=1`, `lossless=1`, `trick=1`) and announced in `GET /v1/info` `capabilities`.
  Without the flag, playlists, init sections and segments stay byte-identical to what they were
  ([ADR 0002](../adr/0002-capability-gated-features.md)). Answers only gain fields; renamed or
  removed fields break the apps already installed.
- **The client API is described in [api/openapi.yaml](../../api/openapi.yaml)**: a new or
  changed route, parameter or field goes there in the same change. `api_contract_tests.rs` fails
  when a router route is missing from it (or it names one the router lacks), admin and setup
  routes included, and `scripts/api-lint.sh` (in `make check`) checks the spec and validates its
  examples against the schemas, unknown fields included. A new example answer must also decode
  in both clients' example tests (`APIExamplesTests.swift`, `APIExamplesTest.kt`), or be listed
  there as one that client doesn't use.
- **Segments are cut from the file's own keyframes** and every variant shares the copy's
  timeline ([ADR 0001](../adr/0001-hls-remux-first.md)). A change that moves a boundary changes
  every cached segment and every client's resume point.
- **Migrations stay harmless to older servers** (numbered files in `migrations/`, applied at
  start): add tables and columns, don't repurpose or drop them in the same release.
- **Background work yields to playback.** Scans, indexes, subtitle extraction, thumbnails and
  credit analysis run one at a time, `nice`d, read slowly from shares, and pause while anything
  streams.
- **Secrets never reach a log or an answer**: keys are shown as `{set, last4?}`, and log lines go
  through `logbuf::scrub`. A test value that looks like a key must be obviously fake.
- **Every bug fix ships with a test that fails without it** (a unit test in the module's
  `mod tests`). See [testing.md](testing.md).

Tests and lint: `scripts/rust-dev.sh cargo test --locked` and `scripts/rust-dev.sh cargo clippy
--all-targets -- -D warnings` from the repository root (Docker only; the toolchain is pinned in
`server/rust-toolchain.toml`), or `cargo test` in `server/` with your own rustup. The Samba
integration tests: `scripts/test-all.sh smb`.

### Lints

The lint policy is the `[lints]` table in `server/Cargo.toml`, so `cargo clippy` applies it
everywhere (tests and the binary too) with no flags to remember; `make check` runs it with
`-D warnings`, so a warning fails the check.

- **Clippy's default set plus `pedantic`**, minus the pedantic lints that are noise here. Each
  allow in the table has a one-line reason: the `cast_*` family (media arithmetic casts between
  known ranges all the time), `doc_markdown`, `too_many_lines`, the naming lints, and a few whose
  suggested rewrite reads worse (`single_match_else`, `format_collect`, `assert_is_empty`).
- **Every `unsafe` block has a `// SAFETY:` comment** right above it, saying why it is sound
  (what the pointers point at, why a pid is still our child), enforced by
  `undocumented_unsafe_blocks`. Keep `unsafe` to small FFI calls (`libc`, macOS `dns_sd`).
- **Every `#[allow]` has a reason** (`allow_attributes_without_reason`):
  `#[allow(clippy::float_cmp, reason = "…")]`, as close to the code as it can be. Prefer fixing
  the code; allow locally when the fix would change behaviour (a case-insensitive file name match,
  a checked subtraction that no longer panics) or read worse. An `#[ignore]`d test says why too
  (`#[ignore = "needs a Samba server"]`).
- To try a lint the policy leaves out, run clippy with `-W clippy::<name>` before adding it to the
  table, and fix or allow what it finds in the same change.
- `third_party/` (the vendored `smb2`) is its own package: the policy doesn't apply to it.

## Streams

Clients authenticate with a Moonrise account token (`Authorization: Bearer`, see "Accounts");
opening a stream makes a session whose random id is in the stream URLs instead of the token.
When a title can't be served, `GET /v1/items/{id}/playback` says why (`supported`, `reasons`).

Served today: any container with a keyframe index (Matroska, MP4/MOV, scanned AVI/MPEG…);
H.264/HEVC copied, anything else NVDEC or ffmpeg decodes transcoded; any audio (AAC/AC-3/E-AC-3
copied, the rest converted). Not yet: DVD folders.

**Track indices.** The app (and the stream URLs) use the library's track indices
(`library/streams.rs`). They are not the file's: external subtitle files are numbered first and
every embedded stream is shifted by their count. `media::map_streams` maps them to ffprobe's (`stream` in the playback JSON), checking that
each lands on a stream of the same kind, and pairs them up by order otherwise.

**Keyframe index** (segment boundaries), kept in the database per file: Matroska from its Cues
(found through the SeekHeads, or the top-level elements ahead of the first Cluster),
MP4/MOV from the `moov` box (`stts`/`ctts`/`stss`/`elst`, a few range reads), anything else
(AVI, MPEG-PS/TS…, and MP4 whose `moov` can't be read) from a one-time `ffprobe` packet scan (one at a time, `nice`d); until that is
done the title is answered with `supported: false, pending: true` (the app asks again). AVI video
is never copied (no presentation times for B-frames). Matroska whose Cues can't be read isn't
served: a damaged file (bad element ids, zeros, cut short where the index should be) is answered
`supported: false` with "the file is damaged or incomplete … re-download it" and marked `broken`
in the library (`probe_error` says why) until it changes, when the scanner probes it again; a
failed read (the share) only fails that request. **Made ahead of need** (`media/index_backfill.rs`,
switch `index_backfill`, on by default): every probed library file without an index, new and
changed ones after each scan, titles being watched first (resume points, next episodes, and the
episodes after one that starts playing), then the newest. One file at a time; a scan reads at
10 MiB/s from a share (`scan::BACKGROUND_RATE`, through the loopback listener), `nice -n 19`,
and is stopped (SIGSTOP) while anything streams (and 90 s after), a transcode or prepared
download runs, or a scan for playback is waiting; a title played meanwhile takes its scan over
at full speed. Matroska and MP4 files without a stored index are read too; files marked
`broken` are passed over. Progress:
`GET /v1/admin/activity` → `index_backfill {state: off|starting|running|paused|idle, files, done,
failed, current, secs}`.

**Segment plan** (`media::plan`, `media::Timeline`): boundaries at keyframes at least 6 s apart,
on the file's own clock, from the file's start time to its end (start time plus ffprobe's duration;
Matroska's duration is its end already). Players count the start time as 0, so the playlists, a
session's `start` and the cues of embedded subtitle tracks (`X-TIMESTAMP-MAP` maps 00:00 to it)
are on that timeline; the segments keep the file's timestamps. Segment 0 is cut from the file's
beginning, its converted audio from the start time. Files starting at 0 (most) see no difference.
What is read from the file outside the plan follows for a start time past 0.1 s (0.31.11,
`media::OwnClock`, `Media::clock_offset`): chapters (item, `/segments`, the credits chapter that
marks a title played) and the title's length, which in Matroska is its last timestamp; scrubbing
thumbnails (planned for the length; sets made before for the last timestamp are made again); and
subtitle timing's references (the file's own text track, the audio read from 0 without `-copyts`;
earlier answers for such files are found again). Chapters with none at or past the start time
were written from 0 and are taken as they are.

**Audio conversion** (`media/audio.rs`). Up to 2 channels: AAC-LC (192 kb/s stereo, 96 kb/s mono);
more: E-AC-3 5.1 at 640 kb/s (6.1/7.1 downmixed). Players that ask with `lossless=1` (their
output takes more than 2 channels) get lossless 5.1/6.1/7.1 sources (TrueHD, DTS-HD MA, FLAC,
PCM, ALAC) as 24-bit FLAC in fMP4 instead, every channel kept and bit-exact (about 2.5–3.5 Mb/s
for 7.1; TrueHD's Atmos objects are not carried, its 7.1 bed is). One rendition per source track, so the index
mapping stays 1:1; `audio[].converted` marks them, `method` is "convert" when any is. Segments
come from *runs*: a decoder→encoder pair started at a segment boundary that keeps encoding
forward (paused by pipe back-pressure when 60 s ahead of what's been asked for, stopped after
3 min without requests or when it meets cached ground), cut into segments here. All runs put
frames on one global grid (frame `k` decodes to samples `k*F - priming`) and feed the encoder
from exactly the run's first sample, placed from the median of the decoded frames' own
timestamps (snapped to the source's frame grid for DTS/MPEG audio/FLAC), so segments of
different runs join without a gap, an overlap or a click. A new run's first segment is ready
in about a second.

**Video transcode** (`media/transcode.rs`). `GET /v1/items/{id}/playback?quality=<bps>&hdr=0|1&burn=<index>`
answers with the title's ladder (`variants[]`, best first) and the one played (`variant`); the
master lists that one (with `abr=1`, the lower ones too).

- *Ladder*: the copied original, then the app's presets (40, 20, 10, 4, 1.5 Mb/s, fitted into
  4K/1080p/1080p/720p/480p, never upscaled) below 90 % of the source's bitrate. `quality=` picks
  the best one within it. Codecs that can't be copied (MPEG-4 Part 2, MPEG-2, AV1, VC-1, 10-bit
  H.264, anything in AVI) have no copy: "Original" is a transcode at the source's size and 1.5×
  its bitrate (clamped per size). The source's bitrate is the library's (the scan's probe), else
  the load's own probe (a title opened before the scan has probed it), else its size over its
  duration; at least 1 Mb/s.
- *HDR*: kept for HDR displays (HEVC Main 10, PQ/HLG tags); with `hdr=0` every variant,
  "Original" too, is tone-mapped to SDR H.264 (`tonemap_cuda`, BT.2390). Burn-in always
  tone-maps (the overlay is 8-bit). Dolby Vision without an HDR10 base layer is never transcoded.
- *Dolby Vision*: the copy keeps the RPUs. Players that show Dolby Vision send `dv=1`; their
  master then says so for the "Original" (profile 8.1/8.4: `SUPPLEMENTAL-CODECS="dvh1.08.LL/db1p"`
  or `db4h` next to `hvc1`; profile 5: `CODECS="dvh1.05.LL"`, entry `dvh1`), and its init section
  gets the `dvvC`/`dvcC` box, added per session (the cached init is the same for everyone).
  Others play the base layer (HDR10/HLG) as before. Profile 7 and 8.2 aren't signalled. The
  Apple players send `dv=1` for a display with Dolby Vision; Android TV when the display lists it
  and a `video/dolby-vision` decoder exists (Media3 1.6+ reads `SUPPLEMENTAL-CODECS`).
- *Burn-in*: `burn=<track index>` of an embedded PGS/DVD/DVB track: decoded to pictures
  (sub2video, from a second reading of the file 20 s earlier, so a line already on screen at
  the start is drawn), scaled, `overlay_cuda` on the frame.
- *One timeline*: NVENC is forced to an IDR at exactly the copy's boundaries (`-force_key_frames`
  5 ms early, `-forced-idr`, no B-frames, no scene-cut or periodic keyframes), timestamps stay
  the file's (`-copyts`, `-enc_time_base demux`, `-fps_mode passthrough`), and the init has no
  edit list, so a segment's tfdt is its boundary. ffmpeg's input seek is asked for 3/23 s past
  the boundary (it subtracts that for B-frame streams), so it lands on the boundary's keyframe.
- *Runs*, as for audio: one ffmpeg (NVDEC → filters → NVENC → fragmented MP4) encodes forward from
  a boundary; fragments are grouped into segments by decode time and cached. A run stays 40 s
  ahead of the last request (pipe back-pressure), stops after 60 s without requests, when it
  reaches cached ground, when its session seeks elsewhere (the new run replaces it), or on
  `DELETE /v1/s/{sid}`. At most `MOONRISE_TRANSCODE_JOBS` (2) run at once; starting another stops
  the least recently used. A stop counts even when it comes before the run has started (0.21.2:
  a playback's prewarm stopped by the player's first request, for a later segment, kept the slot,
  and the request failed after 10 s with "no transcode slot free"); the same holds for audio
  conversion runs and download jobs. A new session whose variant needs a slot while all are in use by
  others (asked within 30 s) gets the copy (`notes`) or `supported: false`.
- Decoding on the GPU for 8-bit H.264, HEVC, AV1, VP9, MPEG-2, VC-1; on the CPU (3 threads,
  `nice`) for MPEG-4 Part 2 and 10-bit H.264. 4K output uses NVENC preset p1, smaller p4.
- *Hardware* (`media/hwaccel.rs`, 0.16.0): the above is the NVIDIA path. At start the server finds what
  can transcode (`transcode.hardware` / `MOONRISE_HWACCEL`: auto, nvidia, qsv, vaapi, videotoolbox,
  v4l2, cpu; 0.18.0: VideoToolbox on macOS — `scale_vt` scales and tone-maps, Intel Macs encode
  nothing narrower than 640 so those go to libx264 — and V4L2 M2M on a Raspberry Pi 4, at most
  1080p, 4K/10-bit HEVC left as they are, as on a board's CPU, which makes at most 720p): NVIDIA
  (NVDEC/NVENC, CUDA filters), Intel Quick Sync (VA-API decode mapped to QSV, `scale_qsv`,
  `vpp_qsv` tone-mapping, `h264_qsv`/`hevc_qsv`), VA-API on Intel or AMD (`scale_vaapi`,
  `tonemap_vaapi` on Intel, `h264_vaapi`/`hevc_vaapi`), else the CPU (libx264 `veryfast`, at most
  1080p, threads = cores − 1 shared by the slots). QSV/VA-API are tested with a 5-frame encode
  first; HEVC Main 10 and GPU tone-mapping are tested apart (without them HDR is tone-mapped to
  H.264 in every transcode, on the CPU when the GPU can't, and the HDR copy stays for HDR screens).
  On those backends burned-in subtitles are laid over on the CPU. The same timeline rules hold
  (IDR at the boundaries: NVENC/x264 `forced-idr`, QSV `forced_idr`, VA-API's forced frames are
  IDRs). `GET /v1/admin/system` → `transcoding` (`nvenc|qsv|vaapi|cpu|off|unavailable`) and
  `transcode` (the chain, notes). See [docs/hardware-transcoding.md](../hardware-transcoding.md).

Measured on an RTX 3050: 4K HDR → 4K SDR tone-mapped at 5.7×
(p1); 1080p 10-bit H.264 (CPU decode) at 4.8×; AV1 1080p at 10×.

`GET /v1/jobs` lists the transcode runs in progress.

**Text subtitles** (`subtitles/subs.rs`). `GET /v1/items/{id}/subtitles` lists a title's tracks (embedded,
sidecar, fetched; `url` for text ones); `GET /v1/items/{id}/subtitles/{index}.srt|.vtt` serves
one (the token, or `sid=` of a session playing it).

- *Embedded*: every text track of a file is written as SRT at once (the file's own times), kept
  in `/data/subs/v1/<file key>/` (path, size and mtime; never evicted, tens of KB per track;
  `done.json` / `failed.json` mark the file). Matroska is read directly (`subtitles/subx.rs`, no ffmpeg):
  when the Cues list the subtitle blocks with their position in the cluster (mkvmerge does, so most
  files), only those blocks are read, neighbours merged, 8 reads in flight: a film's
  tracks in 13-15 s (three feature films: 270 s → 13.5 s, 255 s → 13.4 s; a 90-minute episode
  → 14.8 s; the share answers ~90 small reads a second). Otherwise the clusters are
  scanned, three parts of the file at once (a 9.8 GB film: 91 s, ~108 MB/s); other
  containers go through one ffmpeg pass. The output is the same as ffmpeg's.
- *Scheduling*: one file is read at a time. A title being played (from its session's start until
  it closes) and a client waiting for a track are urgent, the latest asked for first (the others
  pause; ffmpeg runs are stopped with SIGSTOP); a job nobody waits for any more carries on as
  background work. Extraction holds its next read while a stream segment is being cut, so a
  played title's own segments don't queue behind its subtitles. Background work (prepared
  downloads, the next two episodes of what plays, then the **backfill**: every title of the
  library once, newest first, again when a scan finds new or changed files) reads slower, and much
  slower while anything streams. `GET /v1/jobs` shows the extractions and the backfill.
- *Sidecars* are read from the media share next to the file (decoded: UTF-8/16, else
  Windows-1251 for Cyrillic, else -1252; ASS/VTT converted by ffmpeg).
- *Windows* (`GET /v1/items/{id}/subtitles/{index}/cues?window=<k>` or `?at=<seconds>`, JSON
  `{index, window, from, to, span, complete, source, cues: [{start, end, text}]}`): the cues
  showing during 30 s of the file's time ([30k, 30k+30)), so a track shows from the first second
  and right after any seek, before the whole track is extracted. From the extracted track when
  it is there (`complete: true`: the app fetches the `.srt` once then); otherwise read for that
  window alone, kept in memory: Matroska with subtitle cue points: only the blocks whose cues
  fall in it (by their cue duration, or starting ≤ 10 s before), a few small reads, 30-300 ms;
  Matroska without them: the clusters from the keyframe 6 s before the window to the
  one 2 s past it (no lead when the window before is known), in 2 parts, every text track at
  once, each read waiting (≤ 1.5 s per window) while a stream segment is cut: ~0.2-1 s
  (bitrate-bound; the bytes also warm the page cache for the cuts); MP4: the `tx3g` samples from
  the `moov` sample tables (same text as ffmpeg's, styles as tags), 50-200 ms; anything else:
  ffmpeg seeking there. One open handle and one index read per file, shared with the
  extraction. Extraction holds its reads while windows are read; a played title read whole
  (a scan or ffmpeg: minutes) is no longer urgent, only first among the background work (the
  app shows windows meanwhile); old apps waiting for the `.srt` still make it urgent.
  Measured (cold titles, the share busy): indexed MKV Play→first cue 0.03-0.35 s (0.8.0: 13 s),
  unindexed 0.2-2 s (0.8.0: 45-90 s), MP4 0.1-0.2 s (0.8.0: 40 s); a seek to 90 min the same.
- *WebVTT* (for PiP and AirPlay): segments of 30 s (`subs/<index>/<n>.vtt`) are the windows
  above, so they carry cues from the start (a segment answers without cues only after 6 s:
  AVPlayer fails the whole stream when a selected rendition stalls); `cues.vtt` is still the
  whole track. `X-TIMESTAMP-MAP=MPEGTS:0` (the segments' tfdt is the file's time too). The SRT
  request waits for the extraction.

**OpenSubtitles** (`subtitles/opensubtitles.rs`; tiers and fallbacks: see "Optional services"). `GET /v1/items/{id}/subtitles/search?lang=&q=` searches
by the item's IMDb/TMDb id (an episode's own, else the series' with season and episode) and the
movie hash of the file (two 64 KB range reads), hash matches first; `q=` searches by text.
`POST /v1/items/{id}/subtitles/fetch {result_id}` downloads one into
`/data/subs/<media key>/os-<file id>.srt` (listed in `extras.json` there, indices 10000+) and
answers the new track; nothing is written to the media share. The search answers `{results,
quota: {tier, allowed, remaining, reset_at, account_allowed}, remaining_downloads}`. Credentials:
the `opensubtitles.*` settings (or `OPENSUBTITLES_API_KEY`, `_USERNAME`, `_PASSWORD` in the
environment or in `MOONRISE_OPENSUBTITLES_ENV`, literal `KEY=value` lines, which win), else the
built-in key; new ones apply at once. A key alone downloads anonymously (5 a day per IP
address), with a username and password the account's quota counts (20 a day free, more VIP).
Limits: calls spaced 250 ms (the API allows 5/s), the login token reused for 20 h, searches
cached 6 h, a file fetched before (for any title) is copied instead of downloaded again; 429 when
the day's downloads are used up.

**Timing** (`subtitles/subsync.rs`, 0.21.0). A downloaded track, fetched or uploaded, is matched to the file
when it arrives (or, fetched before 0.21.0, when first asked for), in the background. The file's
own tracks, embedded or sidecar, came with it and aren't matched: `.srt`, the cues and the WebVTT serve
the match as soon as there is one, the original until then and whenever there is no clear answer.
The reference is the file's fullest own text track, in any language, else the speech detected
in its default audio track (the centre channel of a surround track): the whole track of a file up
to 1.5 GB, otherwise 12–36 windows of 60 s, about 1.5 GB read over a share and more only to
locate a cut or settle a part that couldn't be placed; read by ffmpeg at low priority, held while segments or windows are read, and kept
per file (`speech.json`). The model is `video = scale × subtitle + offset`, the scale one of the
conversions between 23.976, 24 and 25 fps, the offset changing at up to 5 places (a cut, a recap),
each such piece kept only if its cues stand clear of anywhere else they could go (a part too
short for that may still keep its original timing, 0.21.4). It is applied only when the
correlation's peak stands out, the fit beats chance, it beats the original timing by more than
chance would (the difference measured against the same two timings moved anywhere else), and no
large part of the track could be damaged by it: one that can't be placed, one better off at its
original timing, or one moved where the reference says nothing (then the answer is `unsure`). A
subtitle already within 0.7 s of the audio (0.15 s of a text track in its language, 0.6 s of one
in another) is left alone. Matches by an older version are made again when next asked for. `GET /v1/items/{id}/subtitles/{index}/timing` answers `{state: none|pending|synced|in_sync|
unsure|failed, reference, offset, scale, pieces, nudge, original}` and starts the match if it
hasn't run (`none`: the file's own track, or not asked for yet; `pending`: running). `PUT` with `{nudge?, original?}` stores the viewer's correction, or the choice of the
original timing, in `timing.json` per file and track, for every device: the app applies the
nudge itself; HLS renditions (PiP, AirPlay, downloads) get it from the server. The originals are
never changed. `subs_sync` switches the matching off; `MOONRISE_SUBSYNC_REFERENCE=text|audio`
limits the reference and `MOONRISE_SUBSYNC_WINDOWS=whole` reads the whole audio track (for
testing; `subtitles/subsync_eval.rs` measures the matcher on real titles read that way).

**Downloads** (`downloads.rs`). `GET /v1/items/{id}/download` serves the original with
`Range`, `If-Range`, `ETag` and `Last-Modified` (background `URLSession` resumes it). `POST /v1/items/{id}/download-jobs
{quality, audio[], hdr, container: "mp4"}` prepares a progressive MP4 (`+faststart`) in
`/cache/dl/`: the copied video (an MKV's original, remuxed) or the ladder's variant for
`quality` transcoded on the GPU (the playback chain with a keyframe every 4 s), and the audio
tracks asked for (default: one per language, the default track first, at most 4; copied, or
converted as for streaming). `202` with the job (`200` with an existing one for the same file
and options); `422` when it can't be made, `507` when the space is full. `GET
/v1/download-jobs[/{job}]` shows state, progress and the space used; `GET
/v1/download-jobs/{job}/file` serves it (`Range`; the random job id is the credential), `DELETE`
drops it. At most `MOONRISE_DOWNLOAD_JOBS` (1) transcodes on the GPU, on top of the playback runs,
and 2 remuxes. A finished file is pinned until it has been fetched (then kept 1 h more for
retries) or for 48 h; files take at most `MOONRISE_DOWNLOAD_MAX_GB` (12) and the disk keeps 3 GB
free. Finished jobs survive a restart (`/cache/dl/<id>.json`); the segment cache's trimming
leaves `dl/` alone. Measured: a 44-min 720p episode at 1.5 Mb/s in 48 s; a 900 MB MKV
remuxed in 45 s.

## Library

The server knows the library itself (`src/library/`). TMDB and IMDb ids, episode numbers, titles
and dates from a one-time import in an early version stay in the database as **match hints**
(`match_hints`, used only by the `bootstrap` identification step below, and linked to their files
after each scan so they follow renames); nothing adds to them any more, and a new server's table
is empty. Migration 017 made them from that import's own table, `jf_map`; migration 021 (0.32.0)
dropped it and `jf_subtitles`, so a database migrated by 0.32.0 or later no longer runs on 0.27.1 or
older (0.27.2 and later never used them).

- **Libraries** are settings (see "Settings": `GET|POST /v1/admin/libraries`, …; or
  `MOONRISE_LIBRARIES="Media=/media;Movies=movies:/media/movies"`, which then wins).
  A root is a local folder or a folder on an SMB share the server reads itself (see "Network
  shares"). Removing a library removes its files and titles from the database.
- **Mixed libraries** (0.15.0): libraries have **no type**. Any folder may hold movies, show
  folders and loose episodes side by side, at any depth; the server decides per file
  (`library/parse.rs` `classify`, then `identify.rs` `classify`):
  - *Episode*: `SxxEyy` (`S01E01E02`, `S04E18-E19`), `1x02`, a season folder (`Season 2`,
    `Series 2`, `S02`, `Specials`, `Сезон 2`) with a number in the name (`Episode 3`, `03 -
    Title`, `E03`), `Episode N` in a name without a year, an air date (`Show.2024.03.15`,
    matched to TMDB's episode of that date), a fansub name (`[Group] Show - 12`); absolute
    numbers (`One Piece - 1071`) are mapped through TMDB's seasons. *Weakly*: three or more videos
    in a folder named alike but for a number (`Chernobyl.Part.1..3`, `01 - Pilot`): these become
    a movie instead when their show isn't on TMDB but a movie of the file's name is.
  - *Movie*: everything else (title and year, a lone video in its folder, edition words).
  - An NFO next to the file (`<episodedetails>` with its numbers, `<movie>`) or a `tvshow.nfo`
    in its folder or the one above wins over the name; Fix Match's choice and the match hints'
    (`match_hints`) over everything.
  - An episode's show is the folder above its season folder; else the nearest folder (above
    per-episode and season-pack release folders, never a sorting folder like "TV Shows") named
    like the show in the file name; else that name (`TV/Show.S01E02.mkv` → show "Show").
  - Extras and samples as before (`Featurettes`, `Extras`, `Sample`… folders; `Other`, `Shorts`,
    `Clips`, `Scenes` only below a title's folder), plus `-trailer`/`-featurette`/… name endings.
  - `kind` (`movies`, `shows`, `mixed`) is still accepted and stored, **only for older apps**
    (`GET /v1/libraries` answers it with `collection_type`; `GET /v1/items?library=` without a
    `type` and `/v1/libraries/{id}/latest` follow it, a `mixed` library answering both). It never
    decides what a file is. New libraries are `mixed`. Nothing identified is identified again
    because of this: items keep their ids, keys, watch state and artwork (identification only
    runs for files that are new, renamed, or changed by Fix Match). Migration 010 lets the
    `libraries` table hold `mixed`.
  - `moonrise-server identify-eval [--typed]` measures it on a COPY of the database: every title
    classified and identified again from its path alone (no stored ids, no LLM), all roots as
    one mixed library (paths below their common folder; `--typed`: below each root), compared
    with the database (kind, movie, show, episode numbers). On a real library of a few thousand
    files (a movies folder and a shows folder as one library) every file's kind, every show and
    every episode came out right, and a few movies with transliterated names went to review
    (none wrong); a synthetic mixed folder (movies,
    a show named like a movie, loose and season-pack episodes, a miniseries by parts, anime
    absolute numbers, an air-dated episode, NFO-only episode) comes out entirely right.
- **Split and double episodes** (`parts.rs`, server 0.17): each season of a TMDB show is
  aligned with TMDB's episode list by the files' titles, numbering, runtimes and part markers
  (`(1)`/`(2)`, `Part 2`, `Pt.1`). Several files that are one TMDB episode become **parts**: one
  item per file (own watch state and resume), Part 1 keeping the episode's key, Part N keyed
  `<key>:p<N>` with `part_no` N, both with the episode's details and artwork; the API answers
  `part` and `parts` (Part 1 of 2), next up and the episode lists go Part 1 → Part 2, and a show
  or season counts the episode watched once all its parts are. Files numbered ahead of TMDB
  (another guide counting hour-long episodes twice: `S04E03-E04 - Dunder Mifflin Infinity` is
  TMDB's S4E2) move to the episode their title is; a file with two TMDB episodes keeps
  `episode_end` (S3E12–13). A plan replaces the plain numbering only when it explains the season
  clearly better; items are re-keyed in place, so ids and watch state stay. Runs once over every
  season (meta `parts_aligned`), then for the shows whose files were just identified; seasons
  with a Fix Match choice are left alone. `moonrise-server align-eval` shows what it would change on
  a COPY of the database. On a real library it re-keyed a dozen seasons (two-part episodes, and
  sitcoms numbered by another guide) and resolved most of the episodes waiting in Fix Match; what
  was left were episodes TMDB doesn't list yet and series released in a different cut.
- **Scanning** (`scanner.rs`): neither CIFS mounts nor SMB tell about
  changes, so it polls. A quick pass every `MOONRISE_SCAN_QUICK_SECS` (120) `stat`s the known folders two
  levels deep and lists only changed ones (0.4-0.7 s, mount or SMB), and `stat`s damaged files
  again (a file copied over another in place leaves its folder's mtime alone; playback reports
  any other file it finds changed); a full walk every `MOONRISE_SCAN_FULL_HOURS` (6)
  and at start (a few seconds for a few thousand files on a NAS). A root that is
  missing, empty or unreadable (a share that doesn't answer or turns the account away, a mount
  that is down) is **offline** for that pass: it is left out, nothing under it is marked missing,
  the other roots are scanned as usual; it is walked in full once it is back. The admin API
  shows each root's `state` (`online`/`offline`), `error` and `since`. Renames (a tool's `os.rename` keeps the mtime) are recognised by size,
  mtime and OpenSubtitles hash: the file keeps its id and title. A file is probed and identified
  only once stable (unchanged for 60 s across two looks, or an mtime over 10 minutes old); a new
  file is re-checked 65 s later. Probing (ffprobe `nice`d, `MOONRISE_SCAN_PROBE_JOBS`=1, once per
  size+mtime) fills the same probe cache playback uses, plus the OpenSubtitles hash. Extras
  (`Featurettes`, `Extras`, `Behind the Scenes`, … folders) and samples aren't titles; files
  under 2 minutes aren't either; a DVD folder (`VIDEO_TS`) is one title (not served yet).
- **Identification** (`parse.rs`, `identify.rs`): `manual` (`POST /v1/admin/items/{id}/identify
  {tmdb_id}`, never overridden) → `bootstrap` (the match hint for the path or series folder) →
  NFO ids (`<file>.nfo`, `movie.nfo`, `tvshow.nfo`: `uniqueid`, `tmdbid`, `imdbid` or a link;
  state `matched`, source `nfo`) → `matched` (TMDB search, scored on transliteration-aware title
  similarity, year, runtime; accepted at ≥ 0.85 with a 0.15 lead, or when exactly one candidate's
  title is one of the file's own readings with a fitting year (±1) — or, without a year in the
  name, a fitting runtime — or several are and one is 3× as popular as the rest) → `llm`
  (verdict checked: a candidate, year ±1, runtime ±15 % or ±45 % for a cut named as
  such, title similarity ≥ 0.5; cached in `llm_verdicts`) → `review` (browsable under its parsed
  name; `GET /v1/admin/review` lists it with the TMDB candidates it was scored against, for Fix
  Match). Without a TMDB key titles are `local` (named from their files; identified again once a
  key is there). The file's own readings: the parsed title, it without edition words ("Director's
  Cut", "Extended", "Redux"…), without bracketed parts, the part after " - " ("Star Wars Episode V -
  The Empire Strikes Back"), "2001г"-style years, and the name of a folder that holds this title
  alone; a collection folder's name counts less. Items
  are keyed by identity (`tmdb:movie:603`, `tmdb:tv:1398:s4e3`, else `path:…`), so two files of
  one movie are two versions of one item; a file
  moving to another item leaves a redirect (`item_redirects`). Episode numbers come from the
  name (`S01E02`, `1x02`, `S01E01E02`, `S04E18-E19`, `s01e16-17`, `Episode 2`); the stored
  bootstrap numbers win where they differ. Episodes TMDB doesn't list (other numbering) stay, flagged review.
- **Metadata** (`metadata.rs`, `tmdb.rs`): TMDB, language and certification country from the settings (`metadata.language`, default `en`;
  `metadata.country`, default `US`; then US).
  Refresh: added in the last 30 days weekly, continuing series daily, the rest monthly. Answers
  are cached in `tmdb_cache` (by URL without the key); requests are spaced 125 ms and retried on
  429. Provider ids: the bootstrap ones stay, TMDB's fill gaps (episodes get TVDB/IMDb ids for the app's
  skip markers).
- **Local metadata** (`local.rs`), for what TMDB doesn't cover: NFO files (title, plot, year,
  genres, studios, rating), the file's own tags (title, date, description), artwork in the folders
  (`poster.jpg`, `folder.jpg`, `<file>-poster.jpg`, `fanart.jpg`, `logo.png`, `season01-poster.jpg`,
  `<episode>-thumb.jpg`…), cover art inside the file (MP4 attached picture, Matroska attachment),
  and a frame at 10 % of the video (HDR tone-mapped) for episode thumbnails and movie backdrops.
  TMDB's values win; for a title TMDB describes only missing art is filled in. Once per title
  and change (`items.local_at`), 400 titles a pass.
- **Artwork** (`images.rs`): TMDB's, and the local art above (sources `file:`, `embed:`,
  `frame:` with the file's mtime). An image's id is a hash of its source (immutable), so it is
  the cache tag: `GET /v1/images/{id}?w=` (no auth) answers
  `Cache-Control: public, max-age=31536000, immutable`. Masters (≤ 1920 px) in `/data/images`,
  fetched in the background for titles' art and on first request for people; resized copies in
  the segment cache (`img/`).
- **Search**: FTS5 (`unicode61 remove_diacritics 2`, every word a prefix) over title, original
  title and series name; titles and episodes apart.
- **Playback by id** (`Library::playback_item`): an item plays its largest file with tracks
  numbered and labelled by `streams.rs` (sidecars first, then the file's streams; display titles
  built from language, codec, layout and flags). Ids are the library's own (ULIDs); a merged id
  answers with the new item. Opening a title asks nothing of the share: the file's size and mtime
  come from the scan (checked afterwards, off the request: a changed file is loaded again), and
  folder listings (sidecars) are kept (listed at start, refreshed in the background after 2 min).
  While the share was busy, the `stat`s and listing alone took 1.5-3 s; now a load is ~5-20 ms.
  Concurrent requests for the same title share one load.
- **LLM** (`llm/mod.rs`, `llm/admin.rs`, `llm/chatgpt.rs`): a provider preset (`MOONRISE_LLM_PROVIDER` / `llm.provider`:
  `anthropic` (default), `openai`, `chatgpt`, `gemini`, `openrouter`, `ollama`, `lmstudio`, `llamacpp`,
  `custom`) gives the API style, address, default model and answer format; `MOONRISE_LLM_BASE_URL`,
  `MOONRISE_LLM_MODEL` and `MOONRISE_LLM_OUTPUT=auto|schema|tool|prompt` change them. Anthropic: the
  Messages API (`claude-haiku-5-5`, `output_config.format` JSON schema, `effort: low`, no sampling
  parameters, `anthropic-version: 2026-10-21`); the others: Chat Completions (OpenAI: `max_completion_tokens`
  and `reasoning_effort` for reasoning models); `chatgpt`: the Responses API with a ChatGPT plan's
  token (Sign in with ChatGPT, see `llm/chatgpt.rs`). `auto` tries schema, tool, prompt and keeps the
  first that works. The key is read by reference: `MOONRISE_LLM_API_KEY`, else the variable
  `MOONRISE_LLM_API_KEY_ENV` names (default: the provider's, e.g. `ANTHROPIC_API_KEY`), or the `llm.*`
  settings; each key is bound to the provider and address (origin) it was saved for
  (`llm.keys`, `llm/keys.rs`) and the request layer sends it only there; redirects aren't followed. At most `MOONRISE_LLM_MAX_CALLS_PER_SCAN` (200) calls per
  pass; usage per day in `usage` (`moonrise-server stats`). `moonrise-server llm-test` makes one
  call to check the setup; `llm-replay [<n>]` asks the last questions again and compares.
  Admin API: `GET /v1/admin/llm` (presets, current, ChatGPT account), `POST /v1/admin/llm/models`,
  `POST /v1/admin/llm/test`, `POST /v1/admin/llm/chatgpt/start|finish|sign-out`. The `chatgpt`
  provider is always used at `https://api.openai.com/v1` (another `base_url` for it is refused);
  its ID tokens are verified against OpenAI's JWKS, and sign-out revokes the refresh token at
  OpenAI's `revocation_endpoint` before clearing the tokens (the registration and host id stay).
  Each ChatGPT account is a separate registration (`llm.chatgpt`: `{active, accounts}`); another
  account is added, never written over another's.

API (a Moonrise token; all JSON snake_case, lists `{items, total}`, times in seconds, dates RFC
3339; `user`: watch state for Moonrise tokens, null otherwise): `GET /v1/libraries`, `GET /v1/items?library=&type=&sort=title|added|year|rating&order=&offset=&limit=`
(or `?ids=`, `?parent=`), `GET /v1/items/{id}` (people, files with tracks, chapters), `GET /v1/shows/{id}/seasons`, `GET /v1/shows/{id}/episodes?season=` (airing
order, specials last), `GET /v1/search?q=` (`{titles, episodes}`), `GET /v1/home`
(`latest_types: [{type: movie|series, title, items}]` across libraries, new episodes grouped
into their shows; `latest` per library for older apps; plus `continue` and `resume`/`next_up`
for Moonrise tokens, see "Accounts"), `GET /v1/facets?type=movie|series&<filters>` (all libraries; `library=<id>` is
a filter like the others; answers `libraries: [{id, name, count}]` too),
`GET /v1/libraries/{id}/latest`, `GET /v1/items/{id}/segments` (from chapter names, the app's
rules, a leading language tag such as `en:` left out; for an episode without credits chapters, the end credits found in the file, `source:
detected`, and `pending: true` while they are being looked for, see "End credits"),
`GET /v1/images/{id}?w=`. Admin: `POST
/v1/admin/scan?full=1[&reidentify=1][&metadata=force]`, `GET /v1/admin/stats?recent=<hours>`,
`GET /v1/admin/review[?source=llm]`, `POST /v1/admin/items/{id}/identify {tmdb_id}` (see
"Managing the server" for Fix Match and Scan Now),
`GET /v1/admin/scan/progress` (live: the running pass's `phase`, files its walk has seen,
`identify {done, total}`, and per library files, movies, shows, episodes, titles in review and
files still being copied; the app's setup polls it every second).

Commands (in the container): `moonrise-server scan [--full]`, `moonrise-server stats [--recent <h>]`,
`moonrise-server identify-eval [--typed]` (on a COPY of the database: classifies and identifies every
title again from its path alone, all roots as one mixed library, no stored ids, no LLM, and
compares with the database; see "Mixed libraries"), `moonrise-server align-eval` (on a COPY: what
aligning every season with TMDB's episode list would change; see "Split and double episodes"),
`moonrise-server smb-credential add` (through the running server, with the local admin token in
`/data/secrets/admin.token`), `moonrise-server llm-test`, `moonrise-server migrate-root` (server stopped;
see "Network shares").

## Network shares

The server reads SMB shares itself (`src/storage/smb.rs`, `src/storage/vfs.rs`): no host mounts, no privileges,
nothing to configure on the host. A library root `{source: "smb", host, share, path,
credential_id}` names a folder on a share; its files are kept as `smb://host/share/path/…` (the
name carries no credentials, so a new password changes no file's identity). Local folders work as
before and stay on their own fast path.

- **Client**: the `smb2` crate (pure Rust, MIT/Apache-2.0, SMB 2.0.2-3.1.1, NTLMv2 and guest,
  signing and encryption as the server asks, pipelined reads, re-dials and signs in again after a
  drop), vendored in `third_party/smb2` with one patch (`MOONRISE-PATCH.txt`): a session the server
  doesn't require to sign isn't signed (only its tree connects are), as with the Linux kernel's
  client; `smb.signing = always` signs everything. On a typical NAS signed reads ran at 18 MB/s,
  unsigned at 79-95 MB/s. One connection per (server, share, credentials); open handles are kept a
  minute; a share that failed to connect isn't asked again for 5 s.
- **Reads** go through a block cache (256 KiB blocks, `MOONRISE_SMB_CACHE_MB`, 384): every ffmpeg
  run reads the container's header and index again, and an audio conversion reads the clusters a
  video cut just read; the mounts had the kernel's page cache for that. Small reads away from a
  file's head (2 MiB) and tail (4 MiB), such as subtitle blocks, go straight to the share. The
  Matroska/MP4 index readers, subtitle extraction and windows, the OpenSubtitles hash, sidecar
  files and folder listings all read through `storage/vfs.rs`. A block is read once for everyone who
  wants it, and if that read fails they all get the error (0.21.1), rather than each reading it
  again after the one before timed out. The process keeps two malloc arenas and hands free memory
  back every 10 s (0.21.1; `MALLOC_ARENA_MAX` overrides): with an arena per thread, blocks freed
  and read again took RSS past 1 GB with the default cache.
- **ffmpeg** (the jellyfin-ffmpeg build has no SMB protocol) opens a file on a share through a
  loopback HTTP listener of the server (`127.0.0.1`, a random port and a random secret in the
  URL) that answers `Range` requests from the SMB client; ffmpeg seeks by asking for another
  range. Both ends use small socket buffers (128 KiB): the listener reads ahead as ffmpeg takes
  the bytes, and kernel buffers would otherwise pull megabytes ffmpeg never reads. A segment run
  is stopped when it has neither written nor read anything for 90 s; what it read is counted at
  the listener for a share (an audio run writes nothing until it ends), from `/proc` for a local
  file (0.21.1).
- **Adding a share** (admin API, the app's "Network share" flow; passwords only in request
  bodies, never answered):
  - `GET /v1/admin/smb/discover[?wait_ms=2000]` → `{servers: [{name, host, addresses, via}]}`:
    Bonjour `_smb._tcp` and a NetBIOS node-status broadcast (Windows, most NAS), merged by
    address. (No WS-Discovery: Windows 10+ machines without NetBIOS are typed in by name.)
  - `POST /v1/admin/smb/shares {host, credential_id | username, password, domain?}` →
    `{host, guest, shares: [{name, comment}]}` (disk shares; `$` ones hidden). No user name: guest.
  - `POST /v1/admin/smb/browse {host, share, path?, …credentials}` → `{url, path, parent, dirs:
    [{name, path}], video_files, truncated}`.
  - `POST /v1/admin/smb/test {host, share, path, …credentials}` → `{ok, url, ms, entries, dirs,
    video_files}` (what saving the root checks).
  - Failures answer `{error, problem, detail}`: `problem` = `unreachable` (502), `auth_required`
    (a guest turned away), `wrong_password` (also an account answered with a guest session),
    `access_denied` (422), `not_found` (404), `other` (502).
  - `GET|POST /v1/admin/smb/credentials`, `PATCH|DELETE /v1/admin/smb/credentials/{id}`:
    `{host?, username, password, domain?}` stored in the settings (`smb.credentials`), the
    password shown as `{set}`, `used_by` the libraries; one in use can't be deleted (409). A root
    given with `username`/`password` instead of `credential_id` stores them (an identical
    credential is reused).
- **Moving a root** between a mount and a share (or back) without losing anything: with the
  server stopped, `moonrise-server migrate-root /media/movies smb://nas/Media/movies --credential <id>
  [--apply]` rewrites the files' paths (ids, titles, watch state stay), the probe and keyframe
  cache, the scan state, `item_video`, the library's root, and renames the subtitle folders
  keyed by the file. A file's size and mtime read over SMB are those the kernel's client
  reported (the same 100 ns ticks), so nothing looks changed afterwards. `moonrise-server
  smb-credential add --user <u> [--host <h>]` (password on stdin) stores a credential in the
  running server and prints its id.
- **Measured** (a NAS, from the server's host, interleaved, 30 titles × 3 cold cuts each,
  medians / p90; the mounts: kernel CIFS 3.0, rsize 1 MiB): video segment cut 0.35 / 0.84 s over
  SMB vs 0.42 / 1.03 s mounted (first cut of a title 0.48 vs 0.75 s; the mounts had one 31 s
  stall); audio cut 0.31 vs 0.43 s; playback answer 0.41 vs 2.9 s; subtitle window 0.20 vs
  0.18 s; whole-track subtitle extraction within ±20 % either way; original download 62-95 vs
  49-76 MB/s; a full scan found the same folders and files with the same sizes and
  mtimes. Raw: sequential 1 GiB 83-88 MB/s vs 85-98 MB/s, random 64 KiB reads p50 17.6 vs 16.2 ms.
- **Test**: `tools/smb-test.sh` starts Samba in Docker with a fixture folder and runs the
  ignored tests in `src/storage/smb_it.rs` (shares, guest/wrong password/unreachable, reads against the
  bytes on disk, the loopback listener, a scan with an offline root, Samba restarted under a
  running client).

## Accounts and watch state

Moonrise's own users, tokens and watch state (`src/auth/`, `src/userdata/`).

- **Users**: argon2id hashes, an admin flag. `docker exec -i moonrise-server moonrise-server user add
  <name> [--admin]` (password on stdin), `user passwd <name>`, `user list`, `user revoke
  <name>`, `user delete <name> --yes`, `users reset-data [--user <name>] --yes` (forgets watch state, playback sessions,
  tokens and device codes; users stay).
- **Tokens**: `moonrise_` + 256 random bits; only the SHA-256 is stored; no expiry; revocable
  (logout, CLI). `Authorization: Bearer`; nothing else is accepted (the CLI uses its local admin
  token). A token's user is cached 60 s. Titles must be in the library and their file readable
  (otherwise 404/422). Admin endpoints: the CLI's admin token or a Moonrise admin.
- **Claim** (`src/admin/claim.rs`, 0.13.0): a server with no users is *unclaimed*: `GET /v1/info` says
  `setup: "unclaimed"` (else `"claimed"`), Bonjour's TXT carries `setup=unclaimed`, `GET /` is a
  tiny page with the setup code (only to private, link-local, loopback and Tailscale addresses;
  others are pointed to the log). `POST /v1/setup/verify {code}` → 204 | 403 wrong | 409 claimed
  | 429; `POST /v1/setup/claim {code, username, password, device_id, device_name}` → `{token,
  user}` (the first user, an admin), 422 for a bad name or password (the code stays valid). The
  code: 6 digits, a new one at every start while unclaimed, logged; 5 wrong per address in 15
  min, then 429; 10 wrong in all replace it. An account made with the CLI claims the server too.
- **Sign-in**: `POST /v1/auth/login {username, password, device_id, device_name}` → `{token,
  user}`; `POST /v1/auth/logout`; `GET /v1/me`. Failed passwords: 10 per 15 min per address and
  per user name, then 429 (an unknown name costs as long as a wrong password).
- **Device code** (the Quick Connect replacement): `POST /v1/auth/device` → `{device_code,
  user_code: "ABCD-1234", verify_url, verify_url_complete, interval: 5, expires_in: 600}`;
  `POST /v1/auth/device/poll {device_code}` → `pending | slow_down | expired | approved (token,
  user; once)`; `POST /v1/auth/device/approve {user_code}` (Bearer). `GET /link` is a plain
  form (code, user name, password) for any browser: no scripts, CSP `default-src 'none'`, a
  SameSite=Strict CSRF cookie matched against a hidden field, Origin checked, same rate limits.
- **Watch state** (`user_data`, movies and episodes; series/seasons derived, marking them
  cascades), the usual media-server rules: under 5 % no resume point; over 90 % or inside the final
  credits: played, position cleared. The credits start is the file's credits chapter or the one
  the app reports (`credits`, the credits it offers, never before a community database's start),
  the later of the two, so a report from a current app is never played before the database
  start (an older app reports none: the chapter alone); a movie whose credits start is known is played only from it, not at 90 % while its
  story runs; titles under 5 min: played instead of a resume point. `play_count` counts completed viewings (once per playback session). Merged items take
  their watch state along.
- **Reporting**: `POST /v1/sessions/{sid}/start|progress|stop {position, paused, audio,
  subtitle, credits, tracks}`, `sid` = the playback session (`session` in the playback answer). Sessions opened
  with a Moonrise token are kept in `play_sessions` (reports still land after a restart; 30 days).
- **Search before typing** (0.30.0, `userdata/search_start.rs`, capability `search-start`):
  `GET /v1/search/start` answers `{recent, suggestions}`. Recent: what the user opened from Search
  (`PUT /v1/search/recent/{id}`, `DELETE /v1/search/recent`; 20 kept, newest first, following
  merges). Suggestions: movies and shows not started, those in the top 3 genres watched most first,
  then best rated, movies and shows taking turns; 20 at most, none of what's in Recent.
- **Track memory** (0.29.0, `userdata/tracks.rs`, capability `track-memory`): `tracks {audio,
  subtitle}` in a report (and in `/v1/sessions/local` reports, with the download's `file`) is
  kept per user and file in `user_file_tracks`; `subtitle: null` is Off, a missing key keeps what
  is kept. Each track is a description (`index, language, title, forced, sdh`), not only its
  index. The playback answer's `tracks {same_file, audio, subtitle, subtitle_off}` gives back the
  file's choice, else the latest in another version of the title, else the latest in the same
  show. The apps find the track by index only in the same file (and only when its language
  agrees), else by language and kind (`TrackRules.rememberedAudio` / `rememberedSubtitle`). The
  subtitle Delay is not kept here: it is the track's timing `nudge`, which comes back with it.
- **Items** carry `user {position, played, play_count, last_played_at, played_percentage}`
  (series/seasons: `unplayed_item_count`, `played`). `PUT|DELETE /v1/items/{id}/played`.
  `GET /v1/items?unplayed=1`, `sort=played|last_played`.
- **Home**: `GET /v1/home` → `continue` (Continue Watching, below), `resume` and `next_up`
  (for older apps: resume points, latest first; per series the first unplayed episode after the
  most recently played one, airing order across seasons, specials excluded, in-progress ones
  left to `resume`), `latest` per library (watched titles left out, episodes grouped into
  series). `GET /v1/shows/{id}/continue` → `{item, reason: resume|next|first}`.
- **Continue Watching** (0.24.0, `userdata/continuing.rs`): one row, the same on every device
  of a user, built from the server's watch state, latest activity first. Each item carries
  `continue {reason, at, from}`:
  - Watching is grouped into threads: a show, a TMDB collection of films, or a film outside any
    collection. Each thread gives at most one card, decided by its latest activity (a progress
    report, a stop, a mark as watched). Cards are ordered by that activity's time.
  - Part-way through (a resume point) → that title, `resume`.
  - An episode finished (watched, or stopped in its credits) → the episode right after it in
    airing order, across seasons, never into specials, **whether or not it was watched
    before** (the same rule as Next Episode in the player, so a rewatch carries on: after
    S1E1 comes S1E2), `next_episode`. When there is none yet the show drops out, and comes
    back once the next episode is added. A finished special doesn't move the show along.
  - A film finished → the film of its TMDB collection released right after it that the
    library has (release order; same-day films by title), **whether or not it was watched**,
    `collection_next`; when there is none, the film drops out.
  - If that next episode or film is part-way through, its card is `resume`.
  - Started but stopped before 5 % (no resume point): not counted.
  - Activity older than **60 days** ages out.
  - `DELETE /v1/continue/{id}` (Remove from Continue Watching) hides the item's thread on every
    device until it is watched again (table `continue_hidden`). Mark as Watched is
    `PUT /v1/items/{id}/played`: the card moves on to the next episode or film.
  - `from` is the title the activity was on (for a next episode, the one finished).
- Logs: every request runs in a span `req{auth=moonrise|admin|none}`. Kill switch
  `MOONRISE_DISABLE=moonrise_auth` (Bearer refused, these endpoints 503; only the CLI works).

## Managing the server (admin API)

There is no web admin: an admin manages the server in the apps (Settings → Server → **Manage
Server**, shown only to admins; Apple TV has a reduced, read-mostly set). Everything below needs
an admin's token (or the CLI's local admin token); other accounts get 403.

- **Libraries**: `/v1/admin/libraries` (see "Settings": add, rename, change folders, remove), each
  root with `state` online/offline and its error; counts from `GET /v1/admin/scan/progress`.
  **Scan Now**: `POST /v1/admin/scan?library=<id>` (that library's folders walked in full in the
  next pass, started at once) or `?full=1&wait=0` (every library); progress as above.
- **Fix Match** (`library/api.rs`): `GET /v1/admin/review` lists titles in review (episodes with
  their show, its TMDB id, season and episode); `GET /v1/admin/items/{id}/match[?compute=1]` one
  title with its kept candidates (`compute`: searched now when none were kept, e.g. reviews older
  than 0.12.0); `GET /v1/admin/match/search?kind=movie|tv&q=&year=`;
  `GET /v1/admin/match/episodes?tmdb_id=&season=` (a TMDB season's episodes, for shows numbered
  differently from TMDB); `GET /v1/admin/match/poster?path=&size=w185|w342` (0.31.0, capability
  `match-posters`, `library/match_posters.rs`): a candidate's poster from the server, so the apps
  contact nothing else. Review, match and search asked for with `server_posters=1` give each
  candidate a `poster_path` instead of the TMDB `poster` URL; without the flag they are unchanged.
  It takes only a TMDB file path of TMDB's own form (never an address), `w185` or `w342`, builds
  the address from a constant, follows no redirect, caps the answer at 4 MiB and passes on only
  JPEG, PNG or WebP bytes. Posters are kept in the segment cache (`match-poster/`), which evicts them with the
  rest; `POST /v1/admin/items/{id}/identify` `{tmdb_id}` (a movie, or a show:
  all its episodes), `{tmdb_id, season, episode}` (an episode, in TMDB's numbering; its own
  numbers keep it as it is but out of review), or `{local: true}` (movies and shows: named from
  the files, never looked up again; the manual key `local`). `type: "movie" | "episode"` corrects
  a wrong classification: a movie that is an episode (`{type: "episode", tmdb_id: <show>, season,
  episode}`), an episode or a whole show that is a movie (`{type: "movie", tmdb_id}` or `{type:
  "movie", local: true}`); the files are read as that type from then on (manual keys
  `tmdb:movie:…`, `tmdb:tv:…:sXeY`, `local:movie`, `local:episode`). The pass that applies it runs at
  once (answer within 40 s, else `pending`); watch state follows to the new item id.
- **Users and devices** (`admin/manage.rs`): `GET|POST /v1/admin/users`, `PATCH|DELETE
  /v1/admin/users/{id}` (`password`, `admin`; not yourself, never the last admin),
  `GET /v1/admin/users/{id}/devices` (tokens: `id` = 16 hex of the token's hash, device name,
  created, last used, `current`), `DELETE /v1/admin/users/{id}/devices/{id}` signs one device out
  at once (not the one asking), `DELETE /v1/admin/users/{id}/devices` all but the one asking.
- **Invitations**: `POST /v1/admin/invites {admin?, days?}` → a one-time code `XXXX-XXXX` (7 days;
  shown once, only its hash kept), `GET` pending ones, `DELETE /v1/admin/invites/{id}`. The new
  person's app: sign-in → **Have an Invitation?** → `POST /v1/auth/invite {code, username,
  password, device_*}` → `{token, user}` (wrong codes count like wrong passwords: 10 per 15 min
  per address). `/v1/info` lists the capability `invites`.
- **Services**: `GET /v1/admin/services` (above) and `PATCH /v1/admin/settings`; an
  OpenSubtitles account is checked first with `POST /v1/admin/services/opensubtitles/check
  {username, password}` (signs in with the key in use, saves nothing; 422 with the reason).
- **Playback and hardware**: `GET /v1/admin/system`: version, uptime, disk use (database,
  subtitles, artwork, data, cache and its limit, prepared downloads), transcoding hardware
  (`nvidia-smi`, render nodes in `/sys/class/drm`, the CPU, ffmpeg's encoders), `transcoding`
  `nvenc|off|unavailable`, `can_restart`, `restart_required`. The `performance.*` settings are
  edited through `/v1/admin/settings` (`apply` says live or restart).
- **Activity**: `GET /v1/admin/activity`: playback sessions used in the last 90 s (who, what,
  copy/convert/transcode, variant, position), transcode runs and slots, subtitle extraction and
  backfill, keyframe indexes made ahead of need (`index_backfill`), prepared downloads, the
  library's pass, the last warnings and errors.
- **Log**: `GET /v1/admin/logs?lines=200[&level=warn|error]`: the last lines kept in memory
  (2000; warnings and errors kept apart, 200), redacted when read (`admin/logbuf.rs`: every key,
  password and token the server knows, and anything shaped like one). Every line, on stderr or
  in the log file too, has credentials in URLs taken out before it is written (0.21.3,
  `logbuf::scrub`): the secret of the SMB loopback URLs ffmpeg reads shares through
  (`/f/[redacted]/…`), `user:password@` and secret-named query parameters. ffmpeg's errors
  that reach clients (a failed download job, a 500) are scrubbed the same way.
- **Restart**: `POST /v1/admin/restart` exits the server (code 75) a moment after answering, only
  when `MOONRISE_RESTART=exit` says a supervisor starts it again (the example compose sets it, with
  `restart: unless-stopped`); otherwise 409.

## Discovery

The server announces itself on the local network with Bonjour (mDNS) as `_moonrise._tcp` on its
port, instance name the server's name plus the first 6 characters of its id ("Moonrise Server (6bc367)",
0.18.0: mdns-sd doesn't rename on a conflict, and two servers under one name merged into one
entry; the apps show `/v1/info`'s name, re-announced when it changes), TXT `id=<server_id> version=<version> path=/v1/info`
(and `setup=unclaimed` while the server has no accounts; re-announced when it is claimed)
(`src/mdns.rs`, the `mdns-sd` crate; with host networking it shares port 5353 with the host's
avahi). Only LAN interfaces take part (Docker bridges, veths, Tailscale and loopback are left out),
so the address announced is one a phone on the Wi-Fi can reach; `MOONRISE_MDNS_INTERFACES=eno1,…`
names them instead. The app browses for it and also probes its /24 networks on 8097 with `GET
/v1/info` (unicast, for networks that drop multicast). Kill switch `MOONRISE_DISABLE=mdns`.
On macOS the system's mDNSResponder announces it (`DNSServiceRegister`), under the plain name
(renamed "Moonrise Server (2)" by mDNSResponder on a conflict): a second responder's packets don't
reach apps on the same Mac. The apps' sweep includes the device's own address (the Mac app on the
server's Mac). Check from a Mac: `dns-sd -B _moonrise._tcp`, `dns-sd -L "Moonrise Server (6bc367)" _moonrise._tcp`.

## Optional services

Moonrise needs nothing but media folders. Each service below makes it better and has a fallback
that is easy to explain; release builds carry Moonrise's own TMDB and OpenSubtitles keys, so a new server asks for no key at all. An admin's own key or account (settings
or environment) wins over the built-in one. `GET /v1/admin/services` (admins) says per service
`state` (`configured`: the admin's own; `builtin`; `not_set`; `off`: a switch), `summary`, `with`
and `without` in plain words, and for OpenSubtitles the `quota`, for the LLM the titles waiting in
Fix Match, for remote access the server's Tailscale address.

| Service | With it | Without it |
|---|---|---|
| TMDB (built-in key; `tmdb.api_key` for your own) | Titles matched on TMDB: posters, backdrops, plots, cast, genres, ratings, episode names | Titles and years from file and folder names, NFO files and the files' own tags; artwork from the folders, cover art inside the files, or a frame of the video. Identified on TMDB as soon as a key is there |
| OpenSubtitles (built-in consumer key; `opensubtitles.*`) | Find Subtitles: search, fetch once for every device. Key alone: 5 downloads a day per IP address; with an account (`opensubtitles.username/password`) 20 a day, VIP more. The apps show what is left and when it resets | Find Subtitles is hidden (the apps say why in Settings); subtitles inside and next to the files still play |
| LLM (`llm.*`; Anthropic, OpenAI, a ChatGPT plan, Gemini, OpenRouter, or any OpenAI-compatible endpoint) | Unclear or transliterated names matched by a model, every answer checked | Matched by name, year and runtime (on a real library: about 97 % of movie files and every show, none wrong); the rest wait in Fix Match with their likely candidates |
| Remote access (Tailscale on the server's computer and the devices) | The apps reach the server from anywhere, nothing opened to the internet | Home network only; the apps explain how to set it up; downloads play anywhere |
| Ratings (IMDb's dataset, no key; `omdb.api_key` for a free OMDb key) | Title pages show IMDb (from IMDb's public dataset), Rotten Tomatoes and Metacritic (through OMDb) beside TMDB | IMDb's and TMDB's ratings; switch `ratings` off: TMDB's only |
| Community segment times (TheIntroDB, IntroDB; no key) | Skip Intro, Skip Recap and Skip Credits for titles whose files have no chapters for them (see "End credits") | The file's chapters and the end credits found in it; Next Episode in the last 30 s otherwise; switch `community_segments` off |

**Ratings (0.26.0, `src/library/ratings.rs`).** IMDb's rating and votes come from IMDb's
non-commercial dataset `title.ratings.tsv.gz` (about 8 MB): fetched a few minutes after start, then
when the last copy is a week old (a day, while titles of the library still have none), read at a
low priority keeping only the library's titles, and deleted. With an OMDb key (free at
omdbapi.com: 1,000 requests a day) films and shows get Rotten Tomatoes (critics) and Metacritic
too: one request every 3 s, at most 500 a day, each title again after 14 days (7 for recent ones);
a title page opened for a title not looked up yet moves it to the front. No built-in key: a free
key is personal. Kept in `ratings` (by IMDb id); the item and episode answers carry `ratings
{imdb: {value, votes}, rotten_tomatoes: {value}, metacritic: {value}}` when there are any. The
apps show the attribution IMDb asks for: "Information courtesy of IMDb (https://www.imdb.com).
Used with permission." (personal, non-commercial use); OMDb's data is CC BY-NC 4.0.

**Built-in keys.** Compiled in at build time from `MOONRISE_BUILTIN_TMDB_KEY` and
`MOONRISE_BUILTIN_OPENSUBTITLES_KEY`, never in the source: the Dockerfile reads them from an optional
BuildKit secret `builtin_keys` (`KEY=value` lines; `docker build --secret
id=builtin_keys,src=keys.env .`); `deploy.sh` passes what the git-ignored
`deploy/local/builtin-keys.sh` prints (e.g. read from a password manager).
Without the secret the build has none (`source: default`), and the fallbacks apply. The settings
API shows a built-in key as `{set: true, builtin: true}`, `fields[…].source: "builtin"`.

## Settings

The configuration lives in the database (`settings` table, `src/admin/settings.rs`) and is edited by an
admin through the API (`src/admin/mod.rs`; the apps' admin screens use it). Environment variables
still work and **win**: per setting, a variable that is set (not empty) is used, then the stored
value, then the default. The API shows where each value comes from.

| Setting | Variable | Applies |
|---|---|---|
| `server.name` (Moonrise Server) | `MOONRISE_NAME` | live (Bonjour re-announced) |
| `metadata.language` (en), `metadata.country` (US) | `MOONRISE_TMDB_LANGUAGE`, `MOONRISE_TMDB_COUNTRY` | live |
| `tmdb.api_key` (built-in key) | `TMDB_API_KEY` | live |
| `omdb.api_key` (none: IMDb's dataset only) | `OMDB_API_KEY` | live |
| `opensubtitles.api_key` (built-in key), `.username`, `.password` (an account: more downloads) | `OPENSUBTITLES_*`, or the file `MOONRISE_OPENSUBTITLES_ENV` (default `/data/secrets/opensubtitles.env`) | live |
| `llm.enabled` (true = auto), `.provider` (a preset: anthropic, openai, chatgpt, gemini, openrouter, ollama, lmstudio, llamacpp, custom), `.base_url`, `.model`, `.api_key`, `.output` (auto/schema/tool/prompt), `.max_calls_per_scan` | `MOONRISE_LLM_ENABLED` (auto/off), `MOONRISE_LLM_PROVIDER` (or `MOONRISE_LLM_API_STYLE`), `MOONRISE_LLM_BASE_URL`, `MOONRISE_LLM_MODEL`, `MOONRISE_LLM_API_KEY` or the variable `MOONRISE_LLM_API_KEY_ENV` names (default: the provider's), `MOONRISE_LLM_OUTPUT`, `MOONRISE_LLM_MAX_CALLS_PER_SCAN` | live |
| `performance.jobs` (4), `.convert_jobs` (4), `.transcode_jobs` (2), `.download_jobs` (1), `.download_max_gb` (12), `.cache_max_gb` (10), `.trickplay_max_gb` (10) | `MOONRISE_JOBS`, `MOONRISE_CONVERT_JOBS`, `MOONRISE_TRANSCODE_JOBS`, `MOONRISE_DOWNLOAD_JOBS`, `MOONRISE_DOWNLOAD_MAX_GB`, `MOONRISE_CACHE_MAX_GB`, `MOONRISE_TRICKPLAY_MAX_GB` | restart |
| `performance.scan_quick_secs` (120), `.scan_full_hours` (6), `.scan_probe_jobs` (1) | `MOONRISE_SCAN_QUICK_SECS`, `MOONRISE_SCAN_FULL_HOURS`, `MOONRISE_SCAN_PROBE_JOBS` | live |
| `features.<switch>` (true; false = off) | `MOONRISE_DISABLE` (comma-separated; it can only switch off) | live, except `library`, `subs_extract`, `subs_backfill`, `transcode`: restart |
| `libraries` | `MOONRISE_LIBRARIES` | live (tables rewritten, full scan) |
| `smb.signing` (auto: only when the server requires it; always) | `MOONRISE_SMB_SIGNING` | live (new sessions) |
| `transcode.hardware` (auto, nvidia, qsv, vaapi, videotoolbox, v4l2, cpu), `transcode.device` (a render node, e.g. `/dev/dri/renderD129`) | `MOONRISE_HWACCEL`, `MOONRISE_HWACCEL_DEVICE` | restart |

Only in the environment: `MOONRISE_LISTEN`, `MOONRISE_DATA_DIR` (SQLite `moonrise.db`), `MOONRISE_CACHE_DIR`
(default `/data`, `/cache`; on macOS `~/Library/Application Support/Moonrise Server` and
`~/Library/Caches/Moonrise Server`), `MOONRISE_FFMPEG`, `MOONRISE_FFPROBE` (macOS: Homebrew's when not on the
PATH), `MOONRISE_LOG_FILE` (log to that file instead of stderr, rotated at 10 MB; the macOS service
sets it), `RUST_LOG`, `MOONRISE_MDNS_INTERFACES`, `MOONRISE_BROWSE_ROOTS`, `MOONRISE_RESTART`, the
switches `db` and `moonrise_auth` (`MOONRISE_DISABLE`), and the LLM's advanced knobs
(`MOONRISE_LLM_ANTHROPIC_VERSION`, `_EFFORT`, `_MAX_TOKENS`, `_TIMEOUT_SECS`, `_VOTES`,
`_EXTRA_HEADERS`). The switches: `db`, `probe_cache`, `convert`, `mp4_index`, `scan_index`,
`transcode`, `burn`, `subs_extract` (no text subtitles then), `subs_windows` (windows off: apps
fetch whole tracks), `subs_backfill` (no library backfill), `subs_sync` (fetched and uploaded
subtitles served as they are), `index_backfill` (keyframe indexes made only when a title is first
played), `credits_detect` (end credits not looked for in episodes' files), `downloads`, `opensubtitles`,
`library`, `llm`, `moonrise_auth`, `mdns`, `ratings` (no IMDb, Rotten Tomatoes or Metacritic ratings),
`trickplay` (no scrubbing thumbnails made or offered), `community_segments` (TheIntroDB and IntroDB
not asked; segments from the files alone).

**Migration.** The first start of 0.10.0 copies everything the environment sets (keys from
`env_file`s and the OpenSubtitles credentials file included, the variable `MOONRISE_LLM_API_KEY_ENV`
names resolved) and the libraries into the store, once (`meta.settings_imported`); defaults
aren't copied. Nothing changes then (the variables still win), and they can be dropped from the
compose file afterwards.

**API** (admins: a Moonrise admin's token, or the CLI's admin token):

- `GET /v1/admin/settings` → `{settings, fields, libraries, restart_required}`. Secrets are
  `{set, last4?}` (an API key's last 4 characters; nothing of a password), never the value.
  `fields["llm.model"]` = `{source: env|file|store|builtin|default, env?: "<VARIABLE>", apply:
  live|restart|env-only, restart_pending?: true}`.
- `PATCH /v1/admin/settings` with the same shape, partial, nested or dotted (`{"metadata.country":
  "GB", "tmdb": {"api_key": "…"}, "features": {"burn": false}}`); `null` or `""` removes a value
  (back to the default). Answers the document plus `changed`, `overridden_by_env` (stored, but
  a variable wins) and `restart_required` (stored; the server runs with the old value until it
  restarts).
- `GET|POST /v1/admin/libraries`, `GET|PATCH|DELETE /v1/admin/libraries/{id}`: `{name, kind?:
  mixed (default)|movies|shows (kept for older apps only), roots: ["/media/Movies" | {source: "local", path} | {source: "smb", host, share,
  path, credential_id | username, password, domain}]}`. A local root must be an existing,
  non-empty folder the server may browse; an SMB one is signed in to and listed (non-empty);
  neither may be inside or around another library's. The id comes from the name and stays. Applied at once (tables, then a full scan). 409 while `MOONRISE_LIBRARIES` is
  set. Roots answer `available` and, after a pass, `state`/`error`/`since`; SMB ones `host`,
  `share`, `url`, `credential_id`, `username`.
- `GET /v1/admin/fs/browse[?path=]`: folders for picking a root: without `path` the browsable
  roots (`MOONRISE_BROWSE_ROOTS`, comma-separated; default every top-level folder but the system's
  and the server's own `/data` and `/cache`; macOS: `~/Movies`, `~`, `/Users/Shared`, `/Volumes/*`), with one its sub-folders (`dirs[{name, path}]`,
  `parent`) and `video_files` (how many it holds directly). Symlinks are resolved before the
  check; nothing but folder names is answered.

## Run

`docker-compose.yml` is the public example: host networking, one folder `./moonrise-data` for
everything (`MOONRISE_CACHE_DIR=/data/cache`), local media mounted read-only (not needed for media on
a NAS: add the share from the app), GPU blocks commented (NVIDIA runtime, or `/dev/dri` for
Intel/AMD). `packaging/` has Unraid, Synology and TrueNAS SCALE templates. The image's ffmpeg is the
jellyfin-ffmpeg package (only an ffmpeg build: NVENC/NVDEC, `scale_cuda`, `tonemap_cuda`,
`overlay_cuda`, Quick Sync, VA-API with Intel and AMD drivers, libx264). The image is built for
linux/amd64 and linux/arm64 (`docker buildx build --platform linux/amd64,linux/arm64 .`; the Rust
build cross-compiles, only the runtime stage runs the target's apt); `moonrise-server healthcheck` is
its HEALTHCHECK.

`deploy.sh` builds the image and runs it on a Docker host over SSH, as described in the
git-ignored `deploy/local/` (see the script's header). It keeps the running image as
`moonrise-server:previous` and rolls back by itself when `/health` doesn't answer `ok`.

**Migrations stay harmless to older servers**, so an older image still runs on a newer database:
add tables and columns, don't repurpose them. The library can be switched off (`features.library`,
then nothing plays), and its tables rebuilt by deleting `moonrise.db` (then a scan;
identification starts over from TMDB and the LLM; the settings go too, and are imported from the
environment again).

## State

`/data/moonrise.db` (SQLite, WAL; mode 0600: it holds the settings' secrets).
`/data/opensubtitles-quota.json`: the OpenSubtitles download quota as last known. Schema from the
numbered migrations in `migrations/`, applied at start. It holds the settings, the library (files, items, people, images, match hints, LLM verdicts, the TMDB
answer cache, search index), the server id (`GET /v1/info`) and the probe cache: ffprobe output and the
keyframe index per file, valid while its size and mtime are unchanged, so a title opens
in a few ms after a restart. It can be deleted: everything in it is rebuilt on demand (the
server id changes). `/data/subs` holds extracted subtitles (rebuilt on demand) and fetched ones
(not rebuildable without using download quota: keep it; kept per file id, `f-<id>`, so renames
keep them, with `timing.json`: nudges, and matches made again on demand); `/data/images` artwork masters; `/data/reports` old reports; `/data/secrets`
the CLI's admin token (and, from before 0.10.0, the OpenSubtitles credentials file).

## Checks and diagnostic tools

`tools/matrix.py` keeps one sample title per format class in `tools/matrix.json` (Moonrise ids,
paths, format characteristics, and the expected `supported` result) and checks a server
against it, with a Moonrise account token:

```
export MOONRISE_URL=http://<server>:8097   # default http://localhost:8097
export MOONRISE_SSH=user@host  # the Docker host, for the --check moonrise log check (default: here)
export MOONRISE_TOKEN_FILE=…   # a Moonrise token (POST /v1/auth/login); never printed
python3 tools/matrix.py --check all          # info, playback (vs expectations), stream, transcode,
                                             # subs (extracted SRT and the WebVTT rendition), download
python3 tools/matrix.py --check playback --record   # accept the current results (also transcode)
python3 tools/matrix.py --check moonrise         # end to end: browse, home, playback, reports, refusals,
                                             # and the server log mentions no other media server
```

`moonrise-server keyframes-test <file.mkv>…` (or one path per line on stdin) reads Matroska
files' Cues as the server would and prints one JSON line per file: the keyframe count, first
and last, and a hash of the times to compare two builds, or why the Cues can't be read
(`failure`: `Damaged`, `Read` or `NoCues`).

`moonrise-server credits-test <file>…` (or one path per line on stdin; `--frames` adds the
keyframes' darkness) looks for the end credits in local files as the server would and prints one
JSON line per file: `start` (seconds, or null), the keyframes measured, and how long it took.

## Scrubbing thumbnails

The apps show a picture above the scrubber while seeking (`media/trickplay.rs`, switch `trickplay`, on by
default, capability `trickplay`). The server makes one picture every 10 s of every title (at most
2,000 per file), 320 px wide, ahead of need, like the keyframe index backfill: one file at a time,
the titles being watched and what is opened in a player first, then the newest, and it pauses
before every picture while anything plays, streams, transcodes or downloads (a longer ffmpeg run,
like the AVI pass or the final sheets and track, is stopped until then). Each picture is one
keyframe decoded alone (`nice`d, HDR tone-mapped, a share read at 8 MB/s), so a file is read only at
its pictures, never whole:
- MP4/MOV with H.264 or HEVC: the keyframes' places come from the file's `moov`, read once, and
  each picture's keyframe sample alone is read.
- Other files (MKV and the rest): its packet is copied out after a seek (`-ss`, at most 2 MB of
  probing), then decoded.
- AVI's MPEG-4, whose packets can't be copied: one paced pass over the whole file (given up after
  three times its reading time).
- Dolby Vision profile 5 (no HDR10 or SDR base layer) is skipped: without the Dolby Vision
  metadata applied its pictures have the wrong colours. The count and reason are in the progress.

They are kept as JPEG sheets of 10 × 10 (for the iPhone, iPad, Mac and
Android TV apps' scrubbers, `GET /v1/items/{id}/trickplay`) and as a small H.264 track of the same
pictures, every one a keyframe, which the master playlist lists as an `EXT-X-I-FRAME-STREAM-INF`
stream when the player asks with `trick=1` (the Apple TV's system player shows its own scrub
preview from it). They live in `<cache>/trickplay/<file key>/`, apart from the segment cache, within
`performance.trickplay_max_gb` (`MOONRISE_TRICKPLAY_MAX_GB`, default 10). When that is full the
backfill stops; a title opened in a player is still made, and the sets used longest ago are deleted
to make room for it (a set counts as used when a player asks for it). Sets of files that changed or left the library are deleted. Progress:
`GET /v1/admin/activity` → `trickplay`.

## End credits

The apps' Skip Credits and Next Episode need to know where an episode's end credits start. The
community databases time them on their contributors' copies, which can be another cut
(on The Sopranos one of them is about two minutes early, inside the last scene). So the first time
an episode's segments are asked for (when it starts playing), the server looks at the file once
(`media/credits.rs`, switch `credits_detect`, on by default): only the keyframes of the last 5 minutes
are decoded (`-skip_frame nokey`, a few dozen small pictures), `nice`d and read at 4 MB/s from a
share (typically 10–80 s). The credits start where a stretch of dark keyframes (≥ 85 % of pixels
dark) begins that lasts at least 20 s, has no lighter gap over 6 s, and reaches into the last 25 s
(studio logos may follow). Credits over pictures (much animation, art cards) aren't found, and a
dense crawl can break a stretch, so a start found here can be late, but it is never in the story.
The answer (`{"v", "start"}`) is kept in `files_probe.credits` while the file is unchanged
(migration 014).

**One file at a time.** A season's download asks for all its episodes at once, so analyses queue:
one runs at a time (`AT_ONCE`), 20 s after its request (the episode's own start comes first), and
a request for a file already queued joins it. Every request while its file waits or runs answers
`pending: true`; the apps ask again near the end. A failed analysis is tried again after 6 h.

**The community databases (0.28.0, `src/library/community.rs`).** The server, not the apps, asks
TheIntroDB (`api.theintrodb.org/v4/media`: by the show's TVDB id, else TMDB, else IMDb, with season,
episode, the episode's TVDB id and the file's length) and IntroDB (`api.introdb.app/segments`: by
IMDb id, with season and episode, or `is_movie`). An episode is asked by its show's ids; parts of
split episodes and episodes without numbers aren't asked. Only on demand: opening a film or an
episode, or asking for its segments, queues it (and the next episode) at the front of a short
queue; one worker asks each database at most every 2 s and 2,000 times a day, a 429 pauses that
database (its `Retry-After`, else 15 minutes), and a failed request is tried again after an hour
(not kept as "nothing"). Answers are kept in `community_segments` (migration 018) by the query
asked: asked again after 30 days, or 7 when the database had nothing; a stale answer is used
meanwhile. Switch `community_segments`.

**The rules** (`src/media/segment_rules.rs`; the apps keep the same ones for older servers,
`clients/apple/Moonrise/Server/SegmentRules.swift`, Android `core/Segments.kt`; all three are
tested against `api/segment-rules.json`). `segments?merged=1` (capability `segments-merged`)
answers with the segments the player offers, by these rules, and never waits for a lookup:
`community_pending: true` while the title waits in the queue (the apps ask again a few seconds
later, and again on entering the last 10 minutes and 2 minutes before the end). While it does, no
end credits are offered at all: the file's could start before a database start not known yet;
intro, recap and the other kinds are, since the file's own always win over the
databases'. A database that can't be asked (paused, failing) doesn't hold them back: the title
isn't waiting then, and the file's credits are offered as without databases. `provisional=1` uses the databases' credits while the file is still looked at (a download
saved now); `community=0` leaves the databases out. Without `merged`, the answer is the file's
alone, as before (older apps merge themselves, and ask the databases from the device). A
database's credits start is never moved earlier, and with several, the latest is used: later is
never story. What the file shows only moves the start later.
- Credits chapters win, but never start before the latest database start: like what the file
  shows, a chapter only moves the credits later (Guardians of the Galaxy: "en:End Credits" at
  6796 s, IntroDB 6852 s: from 6852, ending where the chapter ends; a database start past the
  chapter's end: the database's credits). While the server is `pending`, the databases' credits
  are held back.
- A database's credits must fit the file: in its second half, starting before its end, not
  ending more than 5 s after it.
- **With credits found in the file:** the latest database start, unless the found start is more
  than 10 s after it; then the found start. That leaves out another cut's value inside the last
  scene (The Sopranos), and puts long credits that the picture finds late (The Rings of Power's
  dense crawl, Marvel's art cards) at the found start: late, but inside the credits.
- **Without:** the latest database start. A lone database value that runs to the end from more
  than 150 s before it (the lone-credits rule) is left out; with two or more values, the latest
  of them all, whether or not it runs to the end (one database 2107–2300, the other 2235 to the
  end: 2235).
- Without any, Next Episode shows in the last 30 s.
- **Films** may have scenes in or after their credits (Marvel's), so a film's credits are only
  offered when they end more than 5 s before the end of the file, and Skip Credits jumps to that
  end. Credits chapters that run to the end give way to the databases' (IntroDB usually has an
  end: Avengers: Infinity War 8251–8852 s of 8979); the credits picked as above are left out
  when they run to the end. Episodes are unchanged.
