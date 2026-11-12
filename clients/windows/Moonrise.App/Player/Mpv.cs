using System.Runtime.InteropServices;
using System.Text;

namespace Moonrise.App.Player;

/// <summary>The part of libmpv's client API the player uses.</summary>
internal sealed partial class Mpv : IDisposable
{
    private const string Lib = "libmpv-2.dll";
    private IntPtr _h;

    [LibraryImport(Lib)] private static partial IntPtr mpv_create();
    [LibraryImport(Lib)] private static partial int mpv_initialize(IntPtr h);
    [LibraryImport(Lib)] private static partial void mpv_terminate_destroy(IntPtr h);
    [LibraryImport(Lib, StringMarshalling = StringMarshalling.Utf8)] private static partial int mpv_set_option_string(IntPtr h, string name, string value);
    [LibraryImport(Lib, StringMarshalling = StringMarshalling.Utf8)] private static partial int mpv_set_property_string(IntPtr h, string name, string value);
    [LibraryImport(Lib, StringMarshalling = StringMarshalling.Utf8)] private static partial IntPtr mpv_get_property_string(IntPtr h, string name);
    [LibraryImport(Lib, StringMarshalling = StringMarshalling.Utf8)] private static partial int mpv_get_property(IntPtr h, string name, int format, out long data);
    [LibraryImport(Lib)] private static partial void mpv_free(IntPtr p);
    [LibraryImport(Lib)] private static partial int mpv_command(IntPtr h, IntPtr[] args);
    [LibraryImport(Lib)] private static partial IntPtr mpv_wait_event(IntPtr h, double timeout);
    [LibraryImport(Lib)] private static partial void mpv_wakeup(IntPtr h);
    [LibraryImport(Lib, StringMarshalling = StringMarshalling.Utf8)] private static partial int mpv_request_log_messages(IntPtr h, string level);
    [LibraryImport(Lib, StringMarshalling = StringMarshalling.Utf8)] private static partial int mpv_observe_property(IntPtr h, ulong id, string name, int format);

    public enum EventId { None = 0, Shutdown = 1, LogMessage = 2, StartFile = 6, EndFile = 7, FileLoaded = 8, VideoReconfig = 17, PlaybackRestart = 21, PropertyChange = 22 }

    public const int FormatNone = 0, FormatString = 1, FormatFlag = 3, FormatInt64 = 4, FormatDouble = 5;

    public Mpv()
    {
        _h = mpv_create();
        if (_h == IntPtr.Zero) throw new InvalidOperationException("mpv_create failed");
    }

    public void Option(string n, string v) => Check(mpv_set_option_string(_h, n, v), n);
    public void Initialize() => Check(mpv_initialize(_h), "init");
    public int TrySet(string n, string v) => mpv_set_property_string(_h, n, v);

    public string? GetString(string n)
    {
        var p = mpv_get_property_string(_h, n);
        if (p == IntPtr.Zero) return null;
        var s = Marshal.PtrToStringUTF8(p);
        mpv_free(p);
        return s;
    }

    public long? GetInt64(string n) => mpv_get_property(_h, n, FormatInt64, out var v) >= 0 ? v : null;

    public void Command(params string[] args)
    {
        var native = new IntPtr[args.Length + 1];
        try
        {
            for (int i = 0; i < args.Length; i++) native[i] = Marshal.StringToCoTaskMemUTF8(args[i]);
            mpv_command(_h, native);
        }
        finally { foreach (var p in native) if (p != IntPtr.Zero) Marshal.FreeCoTaskMem(p); }
    }

    public void Observe(ulong id, string name, int format) => mpv_observe_property(_h, id, name, format);
    public void RequestLog(string level) => mpv_request_log_messages(_h, level);
    public void Wakeup() => mpv_wakeup(_h);

    [StructLayout(LayoutKind.Sequential)]
    public struct Event { public int Id; public int Error; public ulong ReplyUserdata; public IntPtr Data; }

    [StructLayout(LayoutKind.Sequential)]
    public struct LogMessage { public IntPtr Prefix; public IntPtr Level; public IntPtr Text; public int LogLevel; }

    [StructLayout(LayoutKind.Sequential)]
    public struct PropertyEvent { public IntPtr Name; public int Format; public IntPtr Data; }

    [StructLayout(LayoutKind.Sequential)]
    public struct EndFileEvent { public int Reason; public int Error; public long PlaylistEntryId; public long PlaylistInsertId; public int PlaylistInsertNum; }

    public Event Wait(double timeout)
    {
        var p = mpv_wait_event(_h, timeout);
        return p == IntPtr.Zero ? default : Marshal.PtrToStructure<Event>(p);
    }

    public void Dispose()
    {
        if (_h != IntPtr.Zero) { mpv_terminate_destroy(_h); _h = IntPtr.Zero; }
    }

    private static void Check(int r, string what)
    {
        if (r < 0) throw new InvalidOperationException($"mpv {what}: error {r}");
    }
}
