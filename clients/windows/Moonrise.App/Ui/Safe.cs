using Microsoft.UI.Dispatching;
using Moonrise.App.Player;
using Windows.Foundation;

namespace Moonrise.App.Ui;

/// <summary>
/// Dispatcher callbacks, timers and "async void": a failure in one of these never reaches
/// Application.UnhandledException, it ends the process. So every one goes through here, and comes
/// out as an error the app can show (Shell.Fault).
/// </summary>
public static class Safe
{
    public static void Run(Action action)
    {
        try { action(); }
        catch (Exception e)
        {
            AppLog.Write("unhandled: " + e);
            App.Failed(e);
        }
    }

    /// <summary>Awaited work started from an event: instead of an "async void", which ends the process when it fails.</summary>
    public static void Fire(Func<Task> work) => _ = Awaited(work);

    private static async Task Awaited(Func<Task> work)
    {
        try { await work(); }
        catch (Exception e)
        {
            AppLog.Write("unhandled: " + e);
            App.Failed(e);
        }
    }

    public static bool Enqueue(this DispatcherQueue queue, Action action) => queue.TryEnqueue(() => Run(action));

    public static TypedEventHandler<DispatcherQueueTimer, object> Tick(Action action) => (_, _) => Run(action);

    /// <summary>A timer calling through the guard; what comes back stops it and takes the handler off it again.</summary>
    public static Action Hook(DispatcherQueueTimer timer, Action action)
    {
        var handler = Tick(action);
        timer.Tick += handler;
        return () => { timer.Stop(); timer.Tick -= handler; };
    }
}
