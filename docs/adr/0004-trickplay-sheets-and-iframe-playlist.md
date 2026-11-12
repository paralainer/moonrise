# 0004. Scrubbing thumbnails as JPEG sheets and an I-frame playlist

Status: accepted

## Context

The apps show a picture above the scrubber while seeking. Apple TV's system player draws its
own scrub preview, but only from an HLS I-frame playlist; the custom players (iPhone, iPad, Mac,
Android TV) draw thumbnails themselves. Making pictures must not slow playback or read whole
files from a NAS.

## Decision

- One picture every 10 s (at most 2,000 per file), 320 px wide, each decoded from a **single
  keyframe** (only the keyframe's bytes are read; the MP4 sample tables or a short seek find it).
- Kept twice: as **JPEG sheets of 10 × 10** for the apps' own scrubbers
  (`GET /v1/items/{id}/trickplay`), and as a **small all-keyframe H.264 track** listed as an
  `EXT-X-I-FRAME-STREAM-INF` stream when the player asks with `trick=1`.
- Made **ahead of need**, one file at a time, titles being watched first, pausing while anything
  plays; within a size budget (`performance.trickplay_max_gb`), evicting the sets used longest ago
  for a title being played.

## Consequences

- Thumbnails appear without the player waiting; nothing is made during playback of another
  title.
- Dolby Vision profile 5 files are skipped (their pictures would have the wrong colours without
  the Dolby Vision metadata), with the reason in the progress.
- The feature is a capability (`trickplay`) and a switch.
