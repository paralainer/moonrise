using System.Runtime.InteropServices;

namespace Moonrise.App.Player;

/// <summary>
/// A Windows power request that keeps the display on (and with it the PC awake) while
/// <see cref="Set"/> is true; cleared by <c>Set(false)</c> or <see cref="Dispose"/>. It shows in
/// <c>powercfg /requests</c> under DISPLAY and SYSTEM with its reason. When to hold it is
/// <see cref="Moonrise.Core.ScreenAwake.Hold"/>.
/// </summary>
internal sealed class DisplayRequest : IDisposable
{
    private const int PowerRequestDisplayRequired = 0, PowerRequestSystemRequired = 1;
    private const uint PowerRequestContextVersion = 0, PowerRequestContextSimpleString = 1;

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct ReasonContext
    {
        public uint Version;
        public uint Flags;
        [MarshalAs(UnmanagedType.LPWStr)] public string SimpleReasonString;
    }

    [DllImport("kernel32.dll", SetLastError = true)] private static extern IntPtr PowerCreateRequest(ref ReasonContext context);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern bool PowerSetRequest(IntPtr request, int type);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern bool PowerClearRequest(IntPtr request, int type);
    [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);

    private IntPtr _request;
    public bool IsHeld { get; private set; }

    public DisplayRequest(string reason)
    {
        var context = new ReasonContext { Version = PowerRequestContextVersion, Flags = PowerRequestContextSimpleString, SimpleReasonString = reason };
        _request = PowerCreateRequest(ref context);
        if (_request == IntPtr.Zero || _request == new IntPtr(-1))
        {
            AppLog.Write("display request: couldn't create one (" + Marshal.GetLastWin32Error() + ")");
            _request = IntPtr.Zero;
        }
    }

    public void Set(bool hold)
    {
        if (hold == IsHeld || _request == IntPtr.Zero) return;
        if (hold)
        {
            if (!PowerSetRequest(_request, PowerRequestDisplayRequired)) return;
            PowerSetRequest(_request, PowerRequestSystemRequired);
        }
        else
        {
            PowerClearRequest(_request, PowerRequestDisplayRequired);
            PowerClearRequest(_request, PowerRequestSystemRequired);
        }
        IsHeld = hold;
    }

    public void Dispose()
    {
        Set(false);
        if (_request != IntPtr.Zero) CloseHandle(_request);
        _request = IntPtr.Zero;
    }
}
