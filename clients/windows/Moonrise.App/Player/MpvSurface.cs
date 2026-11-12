using Moonrise.App.Ui;
using System.Globalization;
using System.Runtime.InteropServices;
using System.Text.Json;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using WinRT;

namespace Moonrise.App.Player;

/// <summary>
/// libmpv in Direct3D 11 composition mode: mpv makes a composition swap chain (no window) and it
/// is shown in a SwapChainPanel, so ordinary XAML (the player controls) composes over the video
/// in the same window. See docs/adr/0014-windows-app.md.
/// </summary>
internal sealed unsafe class MpvSurface : IDisposable
{
    private const ulong TimeId = 1, PausedId = 2, DurationId = 3, BufferingId = 4, TracksId = 5, EofId = 6, ReachedEndId = 7, IdleId = 8, VolumeId = 9, MuteId = 10;
    // ISwapChainPanelNative from microsoft.ui.xaml.media.dxinterop.h
    private static readonly Guid IidSwapChainPanelNative = new("63AAD0B8-7C24-40FF-85A8-640D944CC325");
    private static readonly Guid IidSwapChain2 = new("A8BE2AC4-199F-4946-B331-79599FB98DE7");

    private readonly SwapChainPanel _panel;
    private readonly DispatcherQueue _ui;
    private readonly Mpv _mpv = new();
    private long _swapChain;
    private volatile bool _disposed;

    public event Action<double>? TimeChanged;
    public event Action<double>? DurationChanged;
    public event Action<bool>? PausedChanged;
    public event Action<bool>? BufferingChanged;
    public event Action<IReadOnlyList<MpvTrack>>? TracksChanged;
    public event Action<string>? Ended;
    public event Action<double>? VolumeChanged;
    public event Action<bool>? MuteChanged;

    public MpvSurface(SwapChainPanel panel, DispatcherQueue ui)
    {
        _panel = panel;
        _ui = ui;
        _mpv.Option("vo", "gpu-next,gpu");
        _mpv.Option("gpu-api", "d3d11");
        _mpv.Option("gpu-context", "d3d11");
        _mpv.Option("d3d11-output-mode", "composition");
        _mpv.Option("d3d11-composition-size", SizeString());
        // Software adapter as a last resort (no GPU driver, a VM); a normal PC never uses it.
        _mpv.Option("d3d11-warp", "auto");
        _mpv.Option("hwdec", "auto-safe");
        _mpv.Option("keep-open", "no");
        _mpv.Option("idle", "yes");
        _mpv.Option("force-window", "no");
        _mpv.Option("input-default-bindings", "no");
        _mpv.Option("input-vo-keyboard", "no");
        _mpv.Option("osc", "no");
        _mpv.Option("cursor-autohide", "no");
        // The player page holds the display awake while playing (DisplayRequest), not mpv.
        _mpv.Option("stop-screensaver", "no");
        _mpv.Option("audio-client-name", AppInfo.Name);
        _mpv.Option("title", AppInfo.Name);
        // The server's HLS: its own bitrates and tracks, a generous read-ahead.
        _mpv.Option("hls-bitrate", "max");
        _mpv.Option("demuxer-max-bytes", "150MiB");
        _mpv.Option("demuxer-max-back-bytes", "50MiB");
        // Subtitles are drawn by the app (PlayerPage), so their style and delay can change: mpv shows none.
        _mpv.Option("sid", "no");
        _mpv.Initialize();
        _mpv.RequestLog("warn");
        _mpv.Observe(TimeId, "time-pos", Mpv.FormatDouble);
        _mpv.Observe(PausedId, "pause", Mpv.FormatFlag);
        _mpv.Observe(DurationId, "duration", Mpv.FormatDouble);
        _mpv.Observe(BufferingId, "paused-for-cache", Mpv.FormatFlag);
        _mpv.Observe(TracksId, "track-list", Mpv.FormatNone);
        _mpv.Observe(VolumeId, "volume", Mpv.FormatDouble);
        _mpv.Observe(MuteId, "mute", Mpv.FormatFlag);
        _panel.SizeChanged += OnSizeChanged;
        _panel.CompositionScaleChanged += OnScaleChanged;
        new Thread(EventLoop) { IsBackground = true, Name = "mpv-events" }.Start();
    }

    private void OnSizeChanged(object s, SizeChangedEventArgs e) => Resize();
    private void OnScaleChanged(SwapChainPanel s, object e) { Resize(); ApplyScale(); }

    public void Load(string url, double? startAt)
    {
        var opts = startAt is > 1 ? "start=" + startAt.Value.ToString("F3", CultureInfo.InvariantCulture) : "";
        _mpv.Command("loadfile", url, "replace", "-1", opts);
    }

    public void Pause(bool paused) => _mpv.TrySet("pause", paused ? "yes" : "no");
    public void TogglePause() => _mpv.Command("cycle", "pause");
    public void Seek(double seconds) => _mpv.Command("seek", seconds.ToString("F3", CultureInfo.InvariantCulture), "absolute+exact");
    public void AddVolume(double delta) => _mpv.Command("add", "volume", delta.ToString(CultureInfo.InvariantCulture));
    public void ToggleMute() => _mpv.Command("cycle", "mute");
    public void SetAudioTrack(int id) => _mpv.TrySet("aid", id.ToString());
    public void SetSubtitleTrack(int? id) => _mpv.TrySet("sid", id?.ToString() ?? "no");
    public void SetFill(bool fill) => _mpv.TrySet("panscan", fill ? "1.0" : "0.0");
    public double? Position => double.TryParse(_mpv.GetString("time-pos"), NumberStyles.Float, CultureInfo.InvariantCulture, out var v) ? v : null;

    private string SizeString()
    {
        var w = Math.Max(16, (int)Math.Round(_panel.ActualWidth * _panel.CompositionScaleX));
        var h = Math.Max(16, (int)Math.Round(_panel.ActualHeight * _panel.CompositionScaleY));
        return $"{w}x{h}";
    }

    private void Resize() { if (!_disposed) _mpv.TrySet("d3d11-composition-size", SizeString()); }

    private void ApplyScale()
    {
        if (_swapChain == 0) return;
        var sc = (void*)_swapChain;
        var iid = IidSwapChain2;
        void* sc2 = null;
        if (((delegate* unmanaged[Stdcall]<void*, Guid*, void**, int>)(*(void***)sc)[0])(sc, &iid, &sc2) < 0) return;
        // IDXGISwapChain2::SetMatrixTransform is vtable slot 34: draw at the panel's scale, not the system's.
        var m = stackalloc float[6] { 1f / _panel.CompositionScaleX, 0, 0, 1f / _panel.CompositionScaleY, 0, 0 };
        ((delegate* unmanaged[Stdcall]<void*, float*, int>)(*(void***)sc2)[34])(sc2, m);
        ((delegate* unmanaged[Stdcall]<void*, uint>)(*(void***)sc2)[2])(sc2);
    }

    private void AttachSwapChain()
    {
        var addr = _mpv.GetInt64("display-swapchain");
        if (addr is null or 0 || addr == _swapChain) return;
        Post(() =>
        {
            _swapChain = addr.Value;
            SetPanelSwapChain((void*)addr.Value);
            ApplyScale();
        });
    }

    private void SetPanelSwapChain(void* swapChain)
    {
        var unk = ((IWinRTObject)_panel).NativeObject.ThisPtr;
        var iid = IidSwapChainPanelNative;
        Marshal.ThrowExceptionForHR(Marshal.QueryInterface(unk, in iid, out var native));
        // ISwapChainPanelNative::SetSwapChain is slot 3.
        var hr = ((delegate* unmanaged[Stdcall]<void*, void*, int>)(*(void***)native)[3])((void*)native, swapChain);
        Marshal.Release(native);
        Marshal.ThrowExceptionForHR(hr);
    }

    private void Post(Action a) => _ui.Enqueue(() => { if (!_disposed) a(); });

    private void EventLoop()
    {
        while (!_disposed)
        {
            // This thread has no handler above it: an error here would end the process.
            try { HandleEvent(_mpv.Wait(1.0)); }
            catch (Exception ex) { AppLog.Write("mpv event: " + ex); }
        }
        // mpv_wait_event may not run while the handle is torn down: destroy it here, on this thread.
        _mpv.Dispose();
    }

    private void HandleEvent(Mpv.Event e)
    {
        switch ((Mpv.EventId)e.Id)
        {
            case Mpv.EventId.VideoReconfig:
            case Mpv.EventId.PlaybackRestart:
            case Mpv.EventId.FileLoaded:
                AttachSwapChain();
                break;
            case Mpv.EventId.EndFile:
                var ef = Marshal.PtrToStructure<Mpv.EndFileEvent>(e.Data);
                var reason = ef.Reason switch { 0 => "eof", 2 => "stop", 3 => "quit", 4 => "error", _ => "redirect" };
                Post(() => Ended?.Invoke(reason));
                break;
            case Mpv.EventId.PropertyChange:
                HandleProperty(e);
                break;
            case Mpv.EventId.LogMessage:
                var m = Marshal.PtrToStructure<Mpv.LogMessage>(e.Data);
                AppLog.Write($"mpv [{Marshal.PtrToStringUTF8(m.Prefix)}] {Marshal.PtrToStringUTF8(m.Text)?.TrimEnd()}");
                break;
            case Mpv.EventId.Shutdown:
                _disposed = true;
                break;
        }
    }

    private void HandleProperty(Mpv.Event e)
    {
        var p = Marshal.PtrToStructure<Mpv.PropertyEvent>(e.Data);
        switch (e.ReplyUserdata)
        {
            case TimeId when p.Format == Mpv.FormatDouble:
                var t = Marshal.PtrToStructure<double>(p.Data); Post(() => TimeChanged?.Invoke(t)); break;
            case DurationId when p.Format == Mpv.FormatDouble:
                var d = Marshal.PtrToStructure<double>(p.Data); Post(() => DurationChanged?.Invoke(d)); break;
            case PausedId when p.Format == Mpv.FormatFlag:
                var pz = Marshal.ReadInt32(p.Data) != 0; Post(() => PausedChanged?.Invoke(pz)); break;
            case BufferingId when p.Format == Mpv.FormatFlag:
                var b = Marshal.ReadInt32(p.Data) != 0; Post(() => BufferingChanged?.Invoke(b)); break;
            case VolumeId when p.Format == Mpv.FormatDouble:
                var v = Marshal.PtrToStructure<double>(p.Data); Post(() => VolumeChanged?.Invoke(v)); break;
            case MuteId when p.Format == Mpv.FormatFlag:
                var m = Marshal.ReadInt32(p.Data) != 0; Post(() => MuteChanged?.Invoke(m)); break;
            case TracksId:
                var json = _mpv.GetString("track-list");
                if (json != null)
                {
                    var tracks = JsonSerializer.Deserialize<List<MpvTrack>>(json, new JsonSerializerOptions(JsonSerializerDefaults.Web)) ?? [];
                    Post(() => TracksChanged?.Invoke(tracks));
                }
                break;
        }
    }

    public void Dispose()
    {
        if (_disposed) return;
        _disposed = true;
        _panel.SizeChanged -= OnSizeChanged;
        _panel.CompositionScaleChanged -= OnScaleChanged;
        // The panel holds its own reference to mpv's swap chain and buffers: let them go with mpv.
        try { if (_swapChain != 0) SetPanelSwapChain(null); }
        catch (Exception e) { AppLog.Write("swap chain: " + e.Message); }
        _swapChain = 0;
        _mpv.Wakeup();
    }
}

internal sealed record MpvTrack
{
    public int Id { get; init; }
    public string? Type { get; init; }
    public string? Lang { get; init; }
    public string? Title { get; init; }
    public bool Selected { get; init; }
}

internal static class AppLog
{
    private static readonly string LogFile = Path.Combine(AppInfo.DataDir, "moonrise.log");

    public static string FilePath => LogFile;
    private static readonly object Lock = new();

    public static void Write(string line)
    {
        try
        {
            lock (Lock)
            {
                Directory.CreateDirectory(AppInfo.DataDir);
                File.AppendAllText(LogFile, $"{DateTime.Now:HH:mm:ss.fff} {line}\n");
            }
        }
        catch { }
    }
}
