# Developer docs

For contributors and coding agents. Start with [AGENTS.md](../../AGENTS.md) for the rules and
commands, then:

| Doc | What's in it |
|---|---|
| [architecture.md](architecture.md) | The map: repository layout, server modules by layer, clients, and the data flow (scanning, identify, playback and HLS, segments and credits, trickplay, ratings, watch state and Continue Watching) |
| [server.md](server.md) | Server internals and every HTTP endpoint, with the rules for changing the server |
| [testing.md](testing.md) | The testing rules, where the tests are, and the regression map of past bugs and their tests |
| [adding-a-client.md](adding-a-client.md) | What a new client implements, in order, and the shared rules it must keep |
| [windows.md](windows.md) | Building and testing the Windows app |
| [../building.md](../building.md) | Building and running everything from source: `make` targets, toolchains, the build Mac, the Dev app |
| [../adr/](../adr/README.md) | Architecture decision records: why things are the way they are |

User-facing docs (installing, settings, shares, remote access, privacy) are one folder up, in
[docs/](../).
