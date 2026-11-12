# Security policy

## Reporting a vulnerability

Please report security problems privately, not in a public issue or pull request.

Use GitHub's private vulnerability reporting: open the repository's **Security** tab, choose
**Report a vulnerability**, and fill in the advisory form. Only the maintainers can see it. If that
button isn't there, open an issue that says only that you have a security report and ask for a
private channel; don't include any details.

Helpful things to include:

- what an attacker can do, and what they need first (an account on the server, access to the local
  network, a link to the server from outside, a crafted media file, ...);
- the server version (Manage Server → About, or `/v1/info`) and the app version (Settings);
- how the server runs (Docker, the macOS package, a NAS package) and whether it is reachable from
  outside the local network;
- steps to reproduce, or a proof of concept.

Don't put real tokens, keys, passwords or server addresses in the report; describe them instead.

## What to expect

- We confirm we've received the report within a week.
- We work on a fix in a private advisory, and tell you when we expect it to ship.
- Once a fixed version is released we publish the advisory, crediting you unless you'd rather not
  be named.

Please give us a reasonable time to ship a fix before you talk about the problem in public.

## Supported versions

Fixes go into the latest server release and the latest app release. Older versions don't get
security fixes, so update the server and the apps to receive them. See
[CHANGELOG.md](CHANGELOG.md#versioning) for how the versions are numbered.

## Scope

In scope: the Moonrise server (`server/`), the Apple apps (`clients/apple/`), the Android TV and
phone apps (`clients/android/`), the Windows app (`clients/windows/`), and the scripts and
packaging in this repository.

Out of scope: problems in third-party projects Moonrise uses (ffmpeg, the operating system, Docker,
metadata and subtitle services); please report those to the project concerned. A problem in how
Moonrise *uses* one of them is in scope.
