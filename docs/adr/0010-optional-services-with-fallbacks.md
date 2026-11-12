# 0010. Optional services, each with a plain fallback; built-in keys never committed

Status: accepted

## Context

Asking a new user for a TMDB key, an OpenSubtitles account and an LLM key before anything plays
loses them. But every outside service can be missing, rate-limited or unwanted.

## Decision

- Moonrise needs nothing but media folders. TMDB, OpenSubtitles, an LLM, OMDb and Tailscale
  each improve it, and each has a fallback that's easy to explain (names from files, local
  artwork, Fix Match candidates, home network only…). The apps say what each adds and what
  happens without it.
- Release builds carry the project's TMDB and OpenSubtitles keys, **compiled in from a BuildKit
  secret**, never in the source or a build argument. A user's own key or account wins.
- Only what's needed is asked for, and keys are bound to the provider and address they were
  saved for (an LLM key is never sent elsewhere; redirects aren't followed).

## Consequences

- A build without the secret works, with the fallbacks.
- `scripts/export-public.sh` checks that the real built-in keys appear nowhere in the tree.
- Nothing is sent anywhere else, and there are no analytics ([privacy](../privacy.md)).
