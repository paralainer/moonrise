# Architecture decisions

Short records of the technical decisions that shape Moonrise: what was decided, why, and what
follows from it. Read the relevant one before changing the code it covers; if a decision should
change, add a new record that supersedes it rather than editing the old one.

| # | Decision |
|---|---|
| [0001](0001-hls-remux-first.md) | HLS remux first, segments cut at the file's own keyframes |
| [0002](0002-capability-gated-features.md) | New stream features are asked for, and old clients get the same bytes |
| [0003](0003-audio-renditions-and-flac.md) | Every audio track a rendition; lossless multichannel as FLAC |
| [0004](0004-trickplay-sheets-and-iframe-playlist.md) | Scrubbing thumbnails as JPEG sheets and an I-frame playlist |
| [0005](0005-end-credits-never-earlier.md) | End credits: a community timestamp is never moved earlier |
| [0006](0006-continue-watching-threads.md) | Continue Watching: one thread per show, collection or film, built on the server |
| [0007](0007-libraries-without-a-type.md) | Libraries have no type; split episodes are parts; duplicates are versions |
| [0008](0008-setup-and-management-in-the-apps.md) | Setup and management live in the apps; no web admin |
| [0009](0009-server-reads-smb-itself.md) | The server reads SMB shares itself |
| [0010](0010-optional-services-with-fallbacks.md) | Optional services, each with a plain fallback; built-in keys never committed |
| [0011](0011-apple-tv-system-player.md) | Apple TV uses the system player |
| [0012](0012-server-side-accounts-and-sessions.md) | Accounts and watch state on the server; sessions in stream URLs |
| [0013](0013-the-server-is-the-only-integration-point.md) | The server is the only integration point; apps talk to it alone |
| [0014](0014-windows-app.md) | The Windows app: C# and WinUI 3, video through libmpv's composition swap chain |
| [0015](0015-picture-subtitles-drawn-by-the-apps.md) | Picture subtitles are drawn by the apps, never burned in |

To add one: copy the shape of an existing record (Context, Decision, Consequences), take the next
number, add it to this table, and link it from the code or docs it explains.
