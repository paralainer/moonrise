# 0007. Libraries have no type; split episodes are parts; duplicates are versions

Status: accepted

## Context

People keep movies and shows in one folder, mixed with extras, season packs and odd names. A
library type ("Movies", "Shows") forces them to reorganise and still gets files wrong.

## Decision

- **A library is just folders.** Each file is classified as a movie or an episode by itself
  (name patterns, folders, NFO files), then identified. The Library tab shows Movies and Shows
  across all libraries. The old `kind` is accepted and answered only for older apps.
- **One episode split over several files becomes parts**: one item per file, Part 1 keeping the
  episode's key and Part N keyed `<key>:p<N>`, each with its own watch state. Parts are **not
  stitched** into one continuous playback; Next Episode goes from Part 1 to Part 2.
- **Several files of one title are versions** (quality, cut, translation). The viewer chooses one
  per title, kept on the server; each version has its own resume point.
- **Mistakes are fixable in the app** (Fix Match, Back to Automatic, Split Off This File), never
  only in the database.

## Consequences

- Identification runs per file; items are keyed by identity (`tmdb:movie:603`, …), so ids and
  watch state survive a re-identification.
- `identify-eval` and `align-eval` measure classification and part alignment on a copy of a
  database before a change ships.
