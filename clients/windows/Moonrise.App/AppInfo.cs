using System.Runtime.InteropServices;

namespace Moonrise.App;

internal static class AppInfo
{
#if MOONRISE_DEV
    public const string Name = "Moonrise Dev";
    public const bool IsDev = true;
    public const string IconFile = "MoonriseDev";
#else
    public const string Name = "Moonrise";
    public const bool IsDev = false;
    public const string IconFile = "Moonrise";
#endif

    /// <summary>Folder for this app's own data; the Dev app has its own, so testing never touches a real sign-in.</summary>
    public static readonly string DataDir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), Name);
}

internal static class Native
{
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern int ShowCursor(bool show);
    [DllImport("user32.dll")] public static extern bool AllowSetForegroundWindow(uint processId);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool GetWindowPlacement(IntPtr hwnd, ref WindowPlacement placement);
    [DllImport("user32.dll")] public static extern bool SetWindowPlacement(IntPtr hwnd, ref WindowPlacement placement);

    /// <summary>Win32 WINDOWPLACEMENT: the window's state and, maximized or not, where it goes back to.</summary>
    [StructLayout(LayoutKind.Sequential)]
    public struct WindowPlacement
    {
        public int Length;
        public int Flags;
        public int ShowCmd;
        public int MinX, MinY, MaxX, MaxY;
        public int Left, Top, Right, Bottom;

        public static WindowPlacement Empty => new() { Length = Marshal.SizeOf<WindowPlacement>() };
    }
}
