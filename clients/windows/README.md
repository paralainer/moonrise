# Moonrise for Windows

A native Windows client: C# and WinUI 3 on .NET, playback through libmpv, styled after the Mac
app. [ADR 0014](../../docs/adr/0014-windows-app.md) has the decisions, the licences and how the
controls are drawn over the video.

| Folder | What |
|---|---|
| `Moonrise.Core/` | The API client and shared rules (segments, tracks, versions, watch state): plain C#, no UI |
| `Moonrise.Core.Tests/` | Its tests, checked against the shared cases in `api/`; run on any OS |
| `Moonrise.App/` | The WinUI app: screens (`Pages/`), the libmpv player (`Player/`), Mac-style building blocks (`Ui/`) |
| `installer/moonrise.iss` | The Inno Setup installer script |
| `build.ps1` | The repeatable build: fetches the pinned LGPL libmpv, publishes self-contained, makes the installer |

Build (on Windows, see [docs/dev/windows.md](../../docs/dev/windows.md)):

    winget install Microsoft.DotNet.SDK.10 JRSoftware.InnoSetup
    powershell -ExecutionPolicy Bypass -File clients\windows\build.ps1        # installer in build\installer
    powershell -ExecutionPolicy Bypass -File clients\windows\build.ps1 -Dev   # Moonrise Dev in build\publish-dev

Tests, without Windows: `scripts/test-windows-core.sh`.

What it does: sign-in (servers on the network, a code, or a user name and password; invitations),
Set Up This Server for a new server, Home, Library, Search, title pages with the player, and
Settings. Not yet: downloads, Manage Server, Fix Match, remote access, casting, code signing,
Microsoft Store.
