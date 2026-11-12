# 0013. The server is the only integration point; apps talk to it alone

Status: accepted

## Context

The apps used to call some outside services from the device: the community segment databases
(TheIntroDB, IntroDB) for Skip Intro and Skip Credits, OMDb with a device key for ratings, and
OpenSubtitles with a device key for subtitles of downloaded files. That sent title ids from every
device's address to third parties, kept keys on devices, and split one rule (which credits to
offer) across every client, with each one caching its own copy of the answers.

## Decision

- **Apps contact only the Moonrise server** (and the sign-in page an admin opens for ChatGPT, and
  Tailscale when turned on). Every third-party lookup is the server's: TMDB, OpenSubtitles, OMDb,
  IMDb's dataset, TheIntroDB, IntroDB, the LLM.
- The server **looks up on demand, at a polite pace, and keeps the answers** in its database, so a
  title is asked for once for every device, and an answer never waits for a lookup.
- **Rules that combine outside data with the file move to the server** where the server has both:
  the segments a player offers are `segments?merged=1` (`media/segment_rules.rs`). The clients keep
  the same rules only for older servers, and all implementations are tested against one set of
  cases (`api/segment-rules.json`).
- New answers are capability-gated ([ADR 0002](0002-capability-gated-features.md)): older apps keep
  their old behaviour against a new server; a new app on an older server shows less (the file's own
  segments, the server's ratings) rather than calling a third party itself.
- Device settings for third-party keys go away; per-device preferences that only choose what to
  show (such as "Use Online Databases" for segments) are sent to the server as request flags.

## Consequences

- No title ids or keys leave the user's devices for third parties; [privacy](../privacy.md) lists
  the server's destinations only, plus the few above.
- One cache and one rate limit per server instead of one per device; the server needs outbound
  access for these features (each has a switch and a plain fallback, [ADR 0010](0010-optional-services-with-fallbacks.md)).
- A downloaded title's subtitles are searched for through the server, so it needs the server
  reachable at that moment.
