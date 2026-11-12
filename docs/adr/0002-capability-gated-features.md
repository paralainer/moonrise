# 0002. New stream features are asked for, and old clients get the same bytes

Status: accepted

## Context

Apps already installed on people's devices can't be updated at once, and an HLS player that
meets something it doesn't understand (a codec string, a box in the init section) may refuse the
whole stream. The server is updated more often than the apps.

## Decision

- The server announces what it can do in `GET /v1/info` `capabilities` (`dolby-vision`,
  `lossless-audio`, `trickplay`, `subtitle-windows`, …). Clients check a capability before using
  the feature.
- A feature that changes the stream is **asked for by the client** with a query flag on
  `/playback`: `dv=1` (Dolby Vision signalling), `lossless=1` (FLAC audio), `trick=1` (the I-frame
  playlist), `abr=1` (the lower variants). A server that doesn't know a flag ignores it.
- **Without the flag, the output is byte-identical** to what it was before the feature: master,
  media playlists, init sections and segments. Where a flagged session needs
  different bytes, they're made for it alone, so it never changes what an unflagged session
  receives.
- JSON answers only gain fields. Old values (such as a library's `kind`) stay answered for older
  apps even when nothing decides by them any more.

## Consequences

- A new app works with an older server, and an older app with a new server.
- Tests assert both sides: the flagged output, and that unflagged output is unchanged (for
  example `lossless_audio_only_when_asked`, `dolby_vision_only_for_players_that_show_it`).
- Some per-session work is needed where a cached piece would differ (the Dolby Vision box is
  added per session; the cached init is the same for everyone).
