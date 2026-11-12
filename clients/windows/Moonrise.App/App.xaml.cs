using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;
using Moonrise.App.Player;
using Moonrise.App.Ui;

namespace Moonrise.App;

public partial class App : Application
{
    public static MainWindow? Window { get; private set; }

    public App()
    {
        InitializeComponent();
        // A failure in a XAML or dispatcher callback: logged with its stack and shown (Shell.Fault),
        // and the app carries on instead of the process ending.
        UnhandledException += (_, e) =>
        {
            AppLog.Write($"unhandled: {e.Message}\n{e.Exception}");
            e.Handled = true;
            Failed(e.Exception);
        };
        // Neither of these can be shown from here. A background thread's failure still ends the
        // process, as it should; this is what says why.
        AppDomain.CurrentDomain.UnhandledException += (_, e) => AppLog.Write($"fatal: {e.ExceptionObject}");
        TaskScheduler.UnobservedTaskException += (_, e) => { AppLog.Write($"unobserved: {e.Exception}"); e.SetObserved(); };
    }

    /// <summary>
    /// An error nothing else caught, already logged. Shown when there's a window; before there is
    /// one nothing can show it, and the app must not stay running with nothing on the screen.
    /// </summary>
    public static void Failed(Exception e)
    {
        if (Window is { } window) { window.Shell.Fault(e); return; }
        AppLog.Write("no window yet: exiting");
        Environment.Exit(1);
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        var running = RunningCopy();
        if (running != null) { Safe.Fire(() => HandOverTo(running)); return; }
        Window = new MainWindow();
        Window.Activate();
    }

    /// <summary>
    /// The copy of this app already running, or null when this is the first (it then answers
    /// later launches). One copy per app: the Dev app has its own name and runs beside the other.
    /// </summary>
    private static AppInstance? RunningCopy()
    {
        try
        {
            var first = AppInstance.FindOrRegisterForKey(AppInfo.Name);
            if (!first.IsCurrent) return first;
            first.Activated += (_, _) => Window?.DispatcherQueue.Enqueue(() => Window?.BringForward());
        }
        catch (Exception e) { AppLog.Write("single instance: " + e.Message); } // not worth refusing to start over
        return null;
    }

    /// <summary>A second launch: the running copy comes forward, and this one leaves without a window.</summary>
    private static async Task HandOverTo(AppInstance running)
    {
        Native.AllowSetForegroundWindow(running.ProcessId); // only the foreground process may let it take the front
        await running.RedirectActivationToAsync(AppInstance.GetCurrent().GetActivatedEventArgs());
        Environment.Exit(0);
    }
}
