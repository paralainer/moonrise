# 0006. Continue Watching: one thread per show, collection or film, built on the server

Status: accepted

## Context

A row of every title started becomes a wall of half-finished episodes of the same show, and
each device showing its own row makes switching devices confusing.

## Decision

- Continue Watching is built **on the server** from watch state, so it's the same on every
  device of a user.
- Watching is grouped into **threads**: a show, a TMDB collection of films, or a lone film. Each
  thread gives **at most one card**, decided by its latest activity; cards are ordered by that
  activity.
- A finished episode moves the card to the next episode in airing order (never into specials),
  whether or not it was watched before, so a rewatch carries on. A finished film moves it to the
  next film of its collection by release date. With nothing next, the thread drops out until
  something new arrives.
- A stop before 5 % doesn't count; activity older than 60 days ages out.
- Remove from Continue Watching hides the thread on every device until it's watched again.

## Consequences

- The rule matches Next Episode in the player.
- Clients only render the row (`/v1/home` `continue`); Top Shelf and Android's Watch Next use
  the same cards.
