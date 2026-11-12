# 0008. Setup and management live in the apps; no web admin

Status: accepted

## Context

A server people install once and forget shouldn't need a browser, config files or a second
password to set up, and a web admin is a large surface to secure.

## Decision

- The server has only a tiny page with the setup code (shown only to local network addresses)
  and a device-code sign-in form. Everything else is done in the apps (Manage Server, admins
  only).
- **Claiming**: a new server prints a 6-digit setup code in its log and on that page; the first
  account created with it is the admin. Wrong codes are rate-limited per address and in total.
- Settings live in the database, edited through the admin API. Environment variables still work
  and win, per setting: environment, then the stored value, then a built-in key, then the
  default. The API says where each value comes from.

## Consequences

- The admin API is a first-class client API; any client can offer management.
- Docker users can still pin everything in a compose file.
