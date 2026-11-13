# 0015. Picture subtitles are drawn by the apps, never burned in

Status: accepted

## Context

Picture subtitle tracks (Blu-ray PGS, DVD VobSub, DVB) can't be HLS renditions: HLS subtitles are
WebVTT or IMSC text. The server used to burn a chosen picture track into the video (`burn=`), which
turned a copied stream into a full transcode. Playback then took seconds to start (about 7 s to the
first segment of a 1080p film on a 2-core server, against 0.05-0.09 s for the copy), every seek
restarted the transcode, the picture lost quality, and a small server could be busy with one
viewer. None of the players can read the track itself the way we stream: AVPlayer can't draw them
at all, and Media3 and libmpv see only the HLS renditions, not the file's own track.

## Decision

- **The server decodes picture tracks into timed images** (`server/src/subtitles/bitmap.rs`) and
  serves them through the same 30-second subtitle windows as text tracks
  (`GET /v1/items/{id}/subtitles/{index}/cues?window=k`, `pictures.rs`): each cue is a PNG with
  its place on the track's canvas. A window reads only the subtitle blocks around it, so the first
  picture arrives as fast as a text cue, and a seek costs one more window.
- **Every app draws them** in its own subtitle overlay, scaling the canvas onto the video it
  shows. The video plays as it would without subtitles.
- **Images, not text recognition**: images keep the original look exactly (fonts, colours,
  placement, signs), cost little CPU to make, and can't be misread; recognising the text would
  need language data per language and far more CPU, which is wrong for a small server.
- **Burn-in is removed**, with `burn=`, its capability and switch: all the apps draw the pictures,
  and there are no older apps to keep.

## Consequences

- Picture tracks show at once and seek freely; the stream stays the copy (or whatever variant it
  would be without subtitles).
- The apps' subtitle delay applies to pictures too; text size and background settings don't.
- Only embedded tracks in these three formats are offered; sidecar `.sup`/`.sub` files aren't read,
  and tracks in other picture formats aren't listed.
- AirPlay and Picture in Picture on Apple devices, which show only what's in the stream, don't
  show picture subtitles.
