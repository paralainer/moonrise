# 0003. Every audio track a rendition; lossless multichannel as FLAC

Status: accepted

## Context

Films carry several audio tracks (languages, commentaries) in codecs Apple devices can't all
play (DTS, TrueHD, PCM, Vorbis, Opus…). Switching audio should not reload the video, and owners of
surround systems want the lossless tracks they paid for.

## Decision

- **One HLS audio rendition per source track**, so the track indices map 1:1 and switching is
  instant. AAC, AC-3 and E-AC-3 are copied.
- Others are converted: up to 2 channels to AAC-LC; more to E-AC-3 5.1 at 640 kb/s.
- When the player asks with `lossless=1` (it decodes FLAC in fMP4 and its output takes more than
  2 channels), lossless multichannel sources (TrueHD, DTS-HD MA, FLAC, PCM, ALAC) become **24-bit
  FLAC in fMP4**, every channel kept, bit-exact. FLAC frames are 1024 samples, so the largest 7.1
  frame fits Android's 32 KiB decoder input buffers (a compile-time check).
- Conversion runs as long-lived *runs* that encode forward from a segment boundary on one global
  frame grid, so segments from different runs join without a gap or a click.

## Consequences

- About 2.5–3.5 Mb/s for lossless 7.1, against 640 kb/s for E-AC-3.
- TrueHD's Atmos objects aren't carried; its 7.1 bed is.
- Players that don't ask get E-AC-3 as before (ADR 0002).
