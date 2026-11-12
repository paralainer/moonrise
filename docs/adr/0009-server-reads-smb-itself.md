# 0009. The server reads SMB shares itself

Status: accepted

## Context

Most libraries live on a NAS. Mounting shares on the host needs privileges, host
configuration, and breaks silently; kernel mounts also don't report changes.

## Decision

- A pure-Rust SMB client (vendored `third_party/smb2`, one patch) reads shares directly: no
  mounts, no privileges. A library root names a share folder; credentials are stored apart, so a
  new password doesn't change any file's identity.
- Unsigned sessions unless the NAS requires signing (signed reads were several times slower).
- A block cache serves the header and index reads every ffmpeg run repeats; one read per block
  for everyone waiting.
- ffmpeg reads shares through a **loopback HTTP listener** with a random secret in the URL,
  answering `Range` requests from the SMB client.
- Libraries are polled (quick passes of folder mtimes, a periodic full walk); an unreachable root
  is skipped and nothing under it is marked missing.

## Consequences

- Setup is "pick the share in the app".
- The loopback secret must never reach a log: lines are scrubbed before they're written.
- A Samba-in-Docker integration test (`scripts/test-all.sh smb`) covers the client.
