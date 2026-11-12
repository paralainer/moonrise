# Windows app (`clients/windows/`)

C# and WinUI 3 on .NET, playback through libmpv. [ADR 0014](../adr/0014-windows-app.md) explains
how it's put together and the licence of the bundled libraries.

## Tests, on any OS

`scripts/test-windows-core.sh` (part `windows` of `scripts/test-all.sh` and `make check-quick`)
runs the shared rules' tests in Docker, against `api/examples` and `api/segment-rules.json`.
Rules live in `Moonrise.Core` (no UI); add new ones there with a test, and a shared case under
`api/` when the other apps have the same rule.

## Two rules for the app's code

The tests scan the sources for both (`ClientSourceTests`), because neither failure can be run in a test without a window.

- **An element goes into a parent once.** A XAML element that already has a parent makes the next `Content =`, `Child =` or `Children.Add` throw "already the child of another element". Build a new element, change the one you have in place (`PosterCard.Resize`), or show a long-lived page through `F.ShowOnly`.
- **Dispatcher callbacks, timers and async event handlers go through `Ui/Safe`** (`Enqueue`, `Tick`, `Fire`). A failure in them skips `Application.UnhandledException` and ends the process. `Application.UnhandledException` covers XAML handlers: logged to `moonrise.log` with the stack, and shown (`Shell.Fault`, with Reload and Dismiss).

## Building (Windows only)

```
winget install Microsoft.DotNet.SDK.10 JRSoftware.InnoSetup
powershell -ExecutionPolicy Bypass -File clients\windows\build.ps1
```

`build.ps1` gets the pinned LGPL libmpv (checking its SHA-256, see below), publishes the app
self-contained, and writes the unsigned installer to `clients\windows\build\installer\`. With
`-Dev` it builds Moonrise Dev (separate name, amber icon, data folder) to `build\publish-dev\`
instead: use that for testing, never the app someone watches with. Run it with
`MOONRISE_NO_REPORT=1` set in the environment to keep a test run out of the server's watch
history.

### Getting libmpv

The pinned archive (`$LibMpvFile` in `build.ps1`) is the only asset of the project's GitHub
release `deps-libmpv-<tag>`, an unchanged copy of the upstream `zhongfly/mpv-winbuild` build kept
because the upstream deletes builds after 30 days. `build.ps1` tries, in order:

1. `-LibMpvArchive <path>`: a file you already have. A file whose SHA-256 isn't the pinned one is
   refused.
2. The cache, `%LOCALAPPDATA%\moonrise-build` (or `-CacheDir <dir>`): an archive there from an
   earlier build or put there by hand, again only with the pinned SHA-256.
3. A download from the project's release, then from the upstream.

The project's release is reached without a GitHub sign-in (the script never uses one, and none
belongs on a build machine), so it works once the repository is public. While it can't be reached,
download the archive where you can, `gh release download deps-libmpv-<tag> -R <owner>/<repo>`, and
copy it over for step 1 or 2.

### Updating libmpv

1. Pick a newer `mpv-dev-lgpl-x86_64-*.7z` from the upstream releases and download it.
2. Repeat the licence check in [ADR 0014](../adr/0014-windows-app.md) on it: mpv's configuration
   says `-Dgpl=false`, no GPL parts in FFmpeg or the other linked libraries.
3. Upload it unchanged as a new release of the project: tag `deps-libmpv-<upstream tag>` on a
   commit of `main`, `--prerelease --latest=false`, notes with the upstream link, SHA-256, licence
   and source links (copy the previous deps release's).
4. Change `$LibMpvTag`, `$LibMpvFile` and `$LibMpvSha256` in `build.ps1` together, and the
   version and build tag in `THIRD-PARTY-NOTICES.txt`. `scripts/tests/windows-libmpv-pin.test.sh`
   checks that the URL points at the project's release and the hash is pinned.

Keep older deps releases: a build of an older commit needs its own archive.

Windows App SDK and the other packages come from NuGet at the versions in the project files.
Neither Visual Studio nor git is needed on the Windows machine: building on a VM, copy the tree
over with `git archive` and `scp`.

## Graphics

Hardware decoding and the GPU renderer are mpv's: `vo=gpu-next`, `gpu-context=d3d11`,
`hwdec=auto-safe`. A machine without a D3D11 GPU (a VM) falls back to the WARP software
adapter, which is fine for checking layout and flow, not picture quality or HDR.
