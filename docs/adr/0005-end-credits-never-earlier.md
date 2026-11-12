# 0005. End credits: a community timestamp is never moved earlier

Status: accepted

## Context

Skip Credits and Next Episode need the start of the end credits. Community databases time them
on their contributors' copies, which may be another cut, and a value that's too early cuts off
the end of the story, which is far worse than an offer that comes a little late. Analysing the
file (dark keyframes near the end) finds credits on black reliably but can be late on credits over
pictures, and chapters can be placed loosely.

## Decision

- **A database's credits start is never moved earlier.** With several, the latest is used.
- **What the file shows only moves the start later**: the server's detected start (dark
  keyframes in the last 5 minutes) is used when it's more than 10 s after the database's; a
  credits chapter is used but never before the latest database start.
- A database value must fit the file (in its second half, ending no later than 5 s after it). A
  lone value that runs to the end from far before it is left out.
- Films may have scenes after their credits, so their credits are offered only when they end
  before the end of the file, and Skip Credits jumps to that end.
- Without any value, Next Episode shows in the last 30 s.
- The same rules are implemented in both clients (`SegmentRules.swift`, `Segments.kt`) with the
  same tests; the server reports the credits start the app offered so watch state agrees.
- Since server 0.28 the server applies them itself (`segments?merged=1`, ADR 0013); the clients
  keep them for older servers, and the three are tested against `api/segment-rules.json`.

## Consequences

- Offers are sometimes a few seconds late, never inside the story.
- The rule is deliberately simple; it doesn't chase frame precision. Analysis only ever moves
  the start later.
