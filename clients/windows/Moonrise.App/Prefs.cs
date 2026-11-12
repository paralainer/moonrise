using Moonrise.App.Player;
using Moonrise.Core;

namespace Moonrise.App;

/// <summary>This device's playback preferences (Settings, and picks made in the player), kept in the app's data folder.</summary>
internal static class Prefs
{
    private static string FilePath => Path.Combine(AppInfo.DataDir, "settings.json");

    public static PlayerSettings Current { get; } = PlayerSettings.Load(FilePath);

    public static void Save()
    {
        try { Current.Save(FilePath); }
        catch (Exception e) { AppLog.Write("settings: " + e.Message); }
    }
}
