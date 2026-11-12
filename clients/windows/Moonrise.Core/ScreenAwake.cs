namespace Moonrise.Core;

/// <summary>
/// When the player keeps the display awake: only while the video plays, or is on its way
/// to play (loading, buffering, a stream being restarted). Paused, ended, failed or closed lets
/// Windows turn the display off and sleep as usual. mpv's <c>pause</c> stays false while it waits
/// for data, so a seek or a buffer while playing keeps the hold. The Apple and Android apps follow
/// the same rule (KeepAwake.swift, ScreenAwake.kt).
/// </summary>
public static class ScreenAwake
{
    public static bool Hold(bool paused, bool ended, bool failed, bool closed) =>
        !paused && !ended && !failed && !closed;
}
