using System.Text.RegularExpressions;
using Xunit;

namespace Moonrise.Core.Tests;

/// <summary>The display stays awake only while the video plays: paused, ended, failed and closed let it sleep.</summary>
public class ScreenAwakeTests
{
    private static bool Hold(bool paused = false, bool ended = false, bool failed = false, bool closed = false) =>
        ScreenAwake.Hold(paused, ended, failed, closed);

    [Fact]
    public void PlayingHoldsAndTheRestRelease()
    {
        Assert.True(Hold(), "playing, loading or buffering (mpv's pause stays false)");
        Assert.False(Hold(paused: true), "paused");
        Assert.False(Hold(ended: true), "ended");
        Assert.False(Hold(failed: true), "an error shows");
        Assert.False(Hold(closed: true), "player closed");
    }

    [Fact]
    public void ResumeTakesItAgain() =>
        Assert.Equal([true, false, true, false], new[] { false, true, false, true }.Select(p => Hold(paused: p)));

    private static string App(string file) =>
        File.ReadAllText(Path.Combine(Path.GetDirectoryName(Repo.ApiDir)!, "clients", "windows", "Moonrise.App", file));

    /// <summary>The player follows the rule on every change, and mpv doesn't hold the display itself.</summary>
    [Fact]
    public void ThePlayerFollowsTheRule()
    {
        var page = App(Path.Combine("Pages", "PlayerPage.cs"));
        var onPaused = Regex.Match(page, @"private void OnPaused\(bool paused\)\s*\{(?<b>[^}]*)\}").Groups["b"].Value;
        Assert.Contains("UpdateAwake();", onPaused);
        var showError = Regex.Match(page, @"private void ShowError\([^)]*\)\s*\{(?<b>[^}]*)\}").Groups["b"].Value;
        Assert.Contains("UpdateAwake();", showError);
        Assert.Contains("_awake.Dispose();", page);
        Assert.Contains("ScreenAwake.Hold(", page);
        Assert.Contains("\"stop-screensaver\", \"no\"", App(Path.Combine("Player", "MpvSurface.cs")));
    }
}
