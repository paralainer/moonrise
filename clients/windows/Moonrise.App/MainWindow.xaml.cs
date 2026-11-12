using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Windows.Graphics;
using WinRT.Interop;

namespace Moonrise.App;

public sealed partial class MainWindow : Window
{
    public Shell Shell { get; }
    public AppWindow AppWin { get; }
    public IntPtr Hwnd { get; }

    public MainWindow()
    {
        InitializeComponent();
        Hwnd = WindowNative.GetWindowHandle(this);
        AppWin = AppWindow.GetFromWindowId(Win32Interop.GetWindowIdFromWindow(Hwnd));
        AppWin.Title = AppInfo.Name;
        var icon = Path.Combine(AppContext.BaseDirectory, "Assets", AppInfo.IconFile + ".ico");
        if (File.Exists(icon)) AppWin.SetIcon(icon);

        ExtendsContentIntoTitleBar = true;
        var tb = AppWin.TitleBar;
        tb.ButtonBackgroundColor = Colors.Transparent;
        tb.ButtonInactiveBackgroundColor = Colors.Transparent;
        tb.ButtonForegroundColor = Colors.White;
        tb.ButtonHoverBackgroundColor = Windows.UI.Color.FromArgb(40, 255, 255, 255);

        ResizeToMacDefault();
        Root.RequestedTheme = ElementTheme.Dark;
        Shell = new Shell(this, Root);
        Closed += (_, _) => Shell.WindowClosed();
    }

    /// <summary>A second launch of the app: this window comes to the front, from the taskbar if minimized.</summary>
    public void BringForward()
    {
        if (AppWin.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Minimized } p) p.Restore();
        Activate();
        Native.SetForegroundWindow(Hwnd);
    }

    private void ResizeToMacDefault()
    {
        // Mac app: default 1280x800, minimum 800x540.
        var scale = (double)Native.GetDpiForWindow(Hwnd) / 96;
        var work = DisplayArea.GetFromWindowId(AppWin.Id, DisplayAreaFallback.Primary).WorkArea;
        var w = Math.Min((int)(1280 * scale), work.Width);
        var h = Math.Min((int)(800 * scale), work.Height);
        AppWin.MoveAndResize(new RectInt32(work.X + (work.Width - w) / 2, work.Y + (work.Height - h) / 2, w, h));
        if (AppWin.Presenter is OverlappedPresenter p)
        {
            p.PreferredMinimumWidth = (int)(800 * scale);
            p.PreferredMinimumHeight = (int)(540 * scale);
        }
    }
}
