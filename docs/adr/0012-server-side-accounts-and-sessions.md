# 0012. Accounts and watch state on the server; sessions in stream URLs

Status: accepted

## Context

People watch on several devices and expect to carry on where they stopped. HLS players
(AVPlayer, AirPlay receivers) fetch segments without the app's headers.

## Decision

- Moonrise has its own accounts (argon2id hashes) and long-lived device tokens (only their hash
  stored, revocable per device). Tokens travel only in `Authorization: Bearer`.
- TVs sign in with a **device code** approved on a phone or in a browser form.
- Opening a stream creates a **playback session**; its random id is in every stream URL instead
  of the token. An unknown session answers `410 Gone`, so the player asks for a new one.
- Watch state and the chosen version are kept per user on the server, from the players'
  `start` / `progress` / `stop` reports, so every device shows the same resume points.

## Consequences

- Signing a device out ends its access without touching others.
- A server restart ends streams; clients recover by asking `/playback` again at their position.
