# 0014. The Windows app: C# and WinUI 3, video through libmpv's composition swap chain

Status: accepted

## Context

Moonrise needed a Windows client close to the Mac app in look and feel. The owner chose native
C# with WinUI 3 on .NET (no Java) and libmpv for playback. libmpv is a native library that has
always drawn into a window of its own: a child HWND cannot be covered by XAML in the same window
(the airspace problem), and the controls must be Moonrise's, drawn over the video.

## Decision

- **Code.** `clients/windows/`: `Moonrise.Core` (API client and shared rules, plain C#, no UI, tests
  on any OS against `api/examples` and `api/segment-rules.json`), `Moonrise.App` (the WinUI 3 app,
  unpackaged and self-contained on .NET 10, Windows App SDK 2.x), `build.ps1` and an Inno Setup
  installer. Apps talk only to the server ([ADR 0013](0013-the-server-is-the-only-integration-point.md)).
- **Video and controls.** mpv runs with `--vo=gpu-next --gpu-context=d3d11
  --d3d11-output-mode=composition`: it creates a composition swap chain with no window, and the
  app hands that swap chain (the `display-swapchain` property) to a XAML `SwapChainPanel`
  (`ISwapChainPanelNative`). The panel is an ordinary element in the visual tree, so the controls
  above it are plain XAML with transparency, composed by the system in the same window. The
  panel's size and DPI scale feed mpv's `d3d11-composition-size` and the swap chain's
  `SetMatrixTransform`. The render API (OpenGL into a texture) and a separate overlay window were
  not needed.
- **Streams.** The app plays the server's HLS master with libmpv; audio choices select the
  rendition tracks mpv lists from it. Text subtitles are drawn by the app over the video, as the
  other apps draw them (fetched from the server, embedded tracks window by window), so their size,
  background and delay can change; mpv shows none. Picture tracks are drawn the same way, as
  images from the server ([ADR 0015](0015-picture-subtitles-drawn-by-the-apps.md)). Reports, resume, segments and Next Episode follow
  the same rules and shared cases as the other apps.
- **HDR.** The app asks for the HDR original (`hdr=1`) so the server copies it; mpv tone-maps to
  SDR with the GPU. Composition swap chains report no HDR output, so HDR passthrough to the
  display isn't used yet.
- **Bundled libraries.** `build.ps1` uses a pinned LGPL build of libmpv
  (`zhongfly/mpv-winbuild`, `mpv-dev-lgpl-x86_64`) and checks its SHA-256. Licence check of that
  build: mpv's embedded configuration says `-Dgpl=false`, with rubberband (GPL) disabled and the
  build's recipe removing x264/x265; FFmpeg is LGPL 3 or later without GPL parts. The DLL stays a
  replaceable file next to the app (LGPL), and `THIRD-PARTY-NOTICES.txt` ships with it, with the
  source links. Native binaries are never committed. The upstream keeps its releases for 30 days,
  so the project keeps its own unchanged copy of the pinned archive as the only asset of a
  GitHub release on the project repository, tagged `deps-libmpv-<upstream tag>` (prerelease, not
  an app release; its notes give the upstream, the SHA-256, the licence and the source links).
  `build.ps1` downloads that copy first and the upstream only as a fallback, or takes the file
  from a local path or its cache. Re-pinning means a new upstream build, this licence check
  repeated on it, a new deps release and the new hash ([docs/dev/windows.md](../dev/windows.md#updating-libmpv)).
- **Packaging.** An unsigned per-user installer made with Inno Setup (no administrator rights, no
  separate runtime to install). A Dev build (`build.ps1 -Dev`) has its own name, amber icon and
  data folder.

## Consequences

- The controls are ordinary XAML, so the Mac layout carries over directly and there is no second
  window to keep in step with the video.
- Hardware decoding and HDR depend on the GPU; the build VM has none, so those need checking on a
  real PC (D3D11 falls back to the WARP software adapter there).
- Maintenance of the third-party build: when the pin ages out of the upstream's retention it has
  to be re-pinned or re-hosted.
- The API client and shared rules exist a third time, in C#, kept in step by the shared cases.
