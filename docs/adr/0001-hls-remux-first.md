# 0001. HLS remux first, segments cut at the file's own keyframes

Status: accepted

## Context

Players on Apple devices and Android TV play HLS well. A server that transcodes everything
needs a big GPU and restarts encoders on every seek; a server that serves files whole leaves out
most containers (Matroska) and every device that can't decode a codec.

## Decision

- **Copy first.** The original video is served as it is whenever the device can decode it
  (H.264, HEVC). Only what the device can't play is converted (audio) or transcoded (video).
- **Segments come from the file's own keyframes**, read once from its index (Matroska Cues, the
  MP4 `moov`, or a one-time packet scan for anything else) and kept in the database.
- **Each segment is cut on its own** by a short ffmpeg run that copies one track, so any segment
  can be made at any time, in any order. Nothing restarts on a seek.
- **Boundaries are presentation times.** A fragment belongs to the segment its keyframe's
  presentation time falls in; in open-GOP files the keyframe is decoded well before it is shown.
- **Transcoded variants share the copy's timeline**: the encoder is forced to an IDR at exactly
  the copy's boundaries, and timestamps stay the file's, so the player can switch variants and
  resume points are the same in all of them.

## Consequences

- Playback and seeking start in about a second on modest hardware (a Raspberry Pi can serve 4K
  copies).
- Segment lengths follow the file's GOPs and vary; players cope.
- A file without a readable index isn't served until it's indexed (`pending: true`); a damaged one
  is refused with a reason rather than stalling.
- Changing how boundaries are chosen changes every cached segment and every variant: treat it as
  a breaking change.
