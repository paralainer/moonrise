using System.Globalization;
using System.Text;
using System.Text.RegularExpressions;

namespace Moonrise.Core;

// Subtitles as the other apps show them (clients/apple/Moonrise/Subtitles/, clients/android/core/
// .../Subtitles.kt): the server's text tracks are fetched and drawn by the app, so their size,
// background and delay can change; bitmap tracks are burned in by the server.

public sealed record SubtitleCue(double Start, double End, string Text);

/// <summary>A parsed track: the cues sorted by start.</summary>
public sealed class SubtitleDocument(IReadOnlyList<SubtitleCue> cues)
{
    public IReadOnlyList<SubtitleCue> Cues { get; } = cues;

    /// <summary>The text showing at <paramref name="t"/> (overlapping cues joined, in order), or null.</summary>
    public string? TextAt(double t)
    {
        int lo = 0, hi = Cues.Count - 1, found = -1;
        while (lo <= hi)
        {
            var mid = (lo + hi) / 2;
            if (Cues[mid].Start <= t) { found = mid; lo = mid + 1; } else hi = mid - 1;
        }
        if (found < 0) return null;
        // Overlapping cues: walk back a little to collect everything active.
        var lines = new List<string>();
        for (var i = found; i >= 0 && i >= found - 4; i--)
            if (Cues[i].Start <= t && t < Cues[i].End) lines.Insert(0, Cues[i].Text);
        return lines.Count == 0 ? null : string.Join("\n", lines);
    }

    /// <summary>The next time after <paramref name="after"/> where the text showing changes (a cue starts or ends).</summary>
    public double? NextEdge(double after)
    {
        int lo = 0, hi = Cues.Count;
        while (lo < hi)
        {
            var mid = (lo + hi) / 2;
            if (Cues[mid].Start <= after) lo = mid + 1; else hi = mid;
        }
        double? edge = lo < Cues.Count ? Cues[lo].Start : null;
        for (var i = Math.Max(0, lo - 5); i < lo; i++)
            if (Cues[i].End > after) edge = Math.Min(edge ?? Cues[i].End, Cues[i].End);
        return edge;
    }
}

public static partial class SubtitleParser
{
    public static SubtitleDocument Parse(string text, string? ext = null)
    {
        var lower = ext?.ToLowerInvariant();
        if (lower is "ass" or "ssa" || text.Contains("[Script Info]")) return new(ParseAss(text));
        return new(ParseSrtOrVtt(text));
    }

    /// <summary>UTF-8, else Windows-1252 (older SRT files); without a byte order mark.</summary>
    public static string Decode(byte[] bytes)
    {
        string s;
        try { s = new UTF8Encoding(false, true).GetString(bytes); }
        catch (DecoderFallbackException) { s = Latin1252(bytes); }
        return s.TrimStart('﻿');
    }

    // Windows-1252 without the code-pages provider: Latin-1 with its own 0x80-0x9F.
    private static readonly string Cp1252High = "€\u0081‚ƒ„…†‡ˆ‰Š‹Œ\u008DŽ\u008F\u0090‘’“”•–—˜™š›œ\u009DžŸ";

    private static string Latin1252(byte[] bytes)
    {
        var sb = new StringBuilder(bytes.Length);
        foreach (var b in bytes) sb.Append(b is >= 0x80 and <= 0x9F ? Cp1252High[b - 0x80] : (char)b);
        return sb.ToString();
    }

    /// <summary>SRT and WebVTT share the "start --> end" block structure.</summary>
    public static IReadOnlyList<SubtitleCue> ParseSrtOrVtt(string raw)
    {
        var normalized = raw.Replace("\r\n", "\n").Replace('\r', '\n');
        var cues = new List<SubtitleCue>();
        foreach (var block in normalized.Split("\n\n"))
        {
            var lines = block.Split('\n');
            var ti = Array.FindIndex(lines, l => l.Contains("-->"));
            if (ti < 0) continue;
            var parts = lines[ti].Split("-->");
            if (parts.Length != 2) continue;
            if (Timestamp(parts[0]) is not { } s) continue;
            if (Timestamp(parts[1].Trim().Split(' ')[0]) is not { } e) continue;
            var body = string.Join("\n", lines.Skip(ti + 1)).Trim();
            if (body.Length == 0) continue;
            cues.Add(new SubtitleCue(s, e, StripTags(body)));
        }
        return cues.OrderBy(c => c.Start).ToList();
    }

    /// <summary>"01:02:03,456", "01:02:03.456" and "02:03.456".</summary>
    public static double? Timestamp(string s)
    {
        var comps = s.Trim().Replace(',', '.').Split(':');
        if (comps.Length < 2) return null;
        var total = 0.0;
        foreach (var c in comps)
        {
            if (!double.TryParse(c, NumberStyles.Float, CultureInfo.InvariantCulture, out var v)) return null;
            total = total * 60 + v;
        }
        return total;
    }

    public static string StripTags(string s) =>
        Braces().Replace(Tags().Replace(s, ""), "")
            .Replace("&amp;", "&").Replace("&lt;", "<").Replace("&gt;", ">").Replace("&nbsp;", " ");

    /// <summary>A minimal Advanced SubStation parser: the dialogue's text, styling dropped.</summary>
    public static IReadOnlyList<SubtitleCue> ParseAss(string raw)
    {
        var format = new List<string>();
        var cues = new List<SubtitleCue>();
        var inEvents = false;
        foreach (var line in raw.Replace("\r\n", "\n").Split('\n'))
        {
            var l = line.Trim();
            if (l.StartsWith('[')) { inEvents = l.Equals("[events]", StringComparison.OrdinalIgnoreCase); continue; }
            if (!inEvents) continue;
            if (l.StartsWith("Format:")) format = l[7..].Split(',').Select(f => f.Trim().ToLowerInvariant()).ToList();
            else if (l.StartsWith("Dialogue:") && format.Count > 0)
            {
                var fields = l[9..].Split(',', format.Count);
                if (fields.Length != format.Count) continue;
                int si = format.IndexOf("start"), ei = format.IndexOf("end"), xi = format.IndexOf("text");
                if (si < 0 || ei < 0 || xi < 0) continue;
                if (Timestamp(fields[si]) is not { } s || Timestamp(fields[ei]) is not { } e) continue;
                var text = StripTags(fields[xi]).Replace("\\N", "\n").Replace("\\n", "\n").Replace("\\h", " ").Trim();
                if (text.Length > 0) cues.Add(new SubtitleCue(s, e, text));
            }
        }
        return cues.OrderBy(c => c.Start).ToList();
    }

    [GeneratedRegex("<[^>]+>")]
    private static partial Regex Tags();

    [GeneratedRegex(@"\{[^}]*\}")]
    private static partial Regex Braces();
}

/// <summary>A window of an embedded track (<c>/subtitles/{index}/cues?window=k</c>): the cues showing during 30 s of the file.</summary>
public sealed record SubtitleWindow
{
    public const double Span = 30.0;

    public int Index { get; init; }
    public required int Window { get; init; }
    public double From { get; init; }
    public double To { get; init; }
    public bool Complete { get; init; }
    public string? Source { get; init; }
    public IReadOnlyList<SubtitleCue> Cues { get; init; } = [];
}

/// <summary>The subtitle Delay menu (Mac: TrackMenus.delay).</summary>
public static class SubtitleDelay
{
    public static readonly IReadOnlyList<double> Steps = [-2.0, -1.0, -0.5, -0.1, 0.1, 0.5, 1.0, 2.0];

    /// <summary>"+0.5s", "-1.0s", "+0.0s".</summary>
    public static string Label(double seconds) =>
        (seconds < 0 && Math.Round(seconds, 1) != 0 ? "-" : "+") + Math.Abs(seconds).ToString("0.0", CultureInfo.InvariantCulture) + "s";

    /// <summary>A delay after a nudge, to the millisecond (ten presses of +0.1 are +1.0, not 0.9999).</summary>
    public static double Nudged(double delay, double by) => Math.Round(delay + by, 3);

    /// <summary>The key a delay is kept under on this device, for a track the server keeps no timing for.</summary>
    public static string NudgeKey(string mediaId, string trackTitle) => mediaId + "|" + trackTitle;
}

/// <summary>Where the subtitle text sits (Mac: SubtitleOverlay), in the player's own coordinates.</summary>
public static class SubtitleLayout
{
    /// <summary>The text's size before the Size setting (Mac: 28).</summary>
    public const double BaseSize = 28;

    /// <summary>
    /// The text's distance from the bottom: with the controls hidden 6 % of the picture's height (at
    /// least the text's size); with them showing, just above the scrubber's band.
    /// </summary>
    public static double BottomPadding(double height, bool controlsShown, double bandTop)
    {
        var normal = Math.Max(height * 0.06, BaseSize);
        if (!controlsShown || bandTop <= 0) return normal;
        return Math.Max(normal, height - bandTop + 8);
    }
}

/// <summary>
/// An embedded subtitle track of a Moonrise server title, shown before the server has extracted
/// all of it (the Mac's SubtitleWindowLoader): fetched in windows of 30 s of the file's time at the
/// playhead and the one after it, one request at a time, the playhead's first (a seek drops a
/// request for elsewhere). Cues are merged as they come, so nothing already shown goes away. Once a
/// window says the whole track is ready, the whole track is fetched once and replaces the windows.
/// </summary>
public sealed class SubtitleWindowLoader
{
    /// <summary>Windows fetched past the playhead's.</summary>
    public const int Ahead = 1;

    private readonly Func<int, CancellationToken, Task<SubtitleWindow>> _window;
    private readonly Func<CancellationToken, Task<string>> _full;
    private readonly Action<IReadOnlyList<SubtitleCue>> _onCues;
    private readonly Action _onFallback;
    private readonly Func<TimeSpan, CancellationToken, Task> _delay;
    private readonly Action<string> _log;

    private readonly HashSet<SubtitleCue> _cues = [];
    private readonly HashSet<int> _loaded = [];
    private int? _last;
    private double _position;
    private int? _current;
    private CancellationTokenSource? _fetching;
    private readonly CancellationTokenSource _life = new();
    private bool _complete;
    private bool _stopped;

    public SubtitleWindowLoader(
        Func<int, CancellationToken, Task<SubtitleWindow>> window, Func<CancellationToken, Task<string>> full,
        Action<IReadOnlyList<SubtitleCue>> onCues, Action onFallback,
        Func<TimeSpan, CancellationToken, Task>? delay = null, Action<string>? log = null)
    {
        _window = window;
        _full = full;
        _onCues = onCues;
        _onFallback = onFallback;
        _delay = delay ?? Task.Delay;
        _log = log ?? (_ => { });
    }

    /// <summary>The window being fetched now, if any (for tests and the log).</summary>
    public int? Fetching => _current;
    public bool Complete => _complete;

    public void Stop()
    {
        _stopped = true;
        _life.Cancel();
    }

    /// <summary>The windows <paramref name="t"/> (the file's time) needs that aren't here yet, the playhead's first.</summary>
    public IReadOnlyList<int> Needed(double t)
    {
        var k = (int)(Math.Max(0, t) / SubtitleWindow.Span);
        return Enumerable.Range(k, Ahead + 1).Where(w => !_loaded.Contains(w) && (_last == null || w <= _last)).ToList();
    }

    /// <summary>The playhead is at <paramref name="t"/> (the file's time): fetch what's missing around it.</summary>
    public void Advance(double t)
    {
        _position = Math.Max(0, t);
        if (_stopped || _complete) return;
        var needed = Needed(_position);
        if (needed.Count == 0) return;
        if (_current is { } current)
        {
            // Already fetching one that's wanted (a seek lands a little before its target): it finishes first.
            if (needed.Contains(current)) return;
            // A seek elsewhere: the playhead's window first.
            _fetching?.Cancel();
        }
        _ = Fetch(needed[0]);
    }

    private async Task Fetch(int k)
    {
        _current = k;
        var cts = CancellationTokenSource.CreateLinkedTokenSource(_life.Token);
        _fetching = cts;
        var ct = cts.Token;
        _log($"subtitle window {k} asked for (playhead {_position:0.0})");
        var wait = TimeSpan.FromMilliseconds(500);
        try
        {
            while (true)
            {
                try
                {
                    var w = await _window(k, ct);
                    if (ct.IsCancellationRequested) return;
                    Received(w);
                    break;
                }
                catch (MoonriseHttpException e) when (e.Code == 404 && e.Body.Contains("past the end"))
                {
                    _last = k - 1;
                    break;
                }
                catch (MoonriseHttpException e) when (e.Code is 404 or 405 or 410 or 501 or 503)
                {
                    // A server without windows (rolled back, or switched off): the whole track.
                    if (ct.IsCancellationRequested) return;
                    _log($"subtitle windows unavailable ({e.Code}): the whole track instead");
                    Stop();
                    _onFallback();
                    return;
                }
                catch (Exception e) when (e is not OperationCanceledException)
                {
                    if (ct.IsCancellationRequested) return;
                    // Not ready or unreachable: again soon.
                    _log($"subtitle window {k} failed ({e.Message}), again in {wait.TotalSeconds} s");
                    await _delay(wait, ct);
                    wait = TimeSpan.FromMilliseconds(Math.Min(wait.TotalMilliseconds * 2, 4000));
                }
            }
        }
        catch (OperationCanceledException) { return; }
        finally
        {
            if (_current == k && ReferenceEquals(_fetching, cts)) _current = null;
        }
        if (ct.IsCancellationRequested) return;
        Advance(_position);
    }

    private void Received(SubtitleWindow w)
    {
        _loaded.Add(w.Window);
        var before = _cues.Count;
        foreach (var c in w.Cues) _cues.Add(c with { Text = SubtitleParser.StripTags(c.Text).Trim() });
        _log($"subtitle window {w.Window}: {w.Cues.Count} cues ({w.Source ?? "?"}){(w.Complete ? ", track complete" : "")}");
        if (_cues.Count != before) _onCues(_cues.OrderBy(c => c.Start).ThenBy(c => c.End).ToList());
        if (w.Complete) _ = LoadFull();
    }

    /// <summary>The whole track is ready on the server: one fetch, then no more windows.</summary>
    private async Task LoadFull()
    {
        if (_complete) return;
        _complete = true;
        try
        {
            var text = await _full(_life.Token);
            if (_stopped) return;
            var doc = SubtitleParser.Parse(text);
            _log($"subtitle track complete: {doc.Cues.Count} cues");
            _fetching?.Cancel();
            _onCues(doc.Cues);
        }
        catch (Exception) when (!_stopped)
        {
            // Windows go on.
            _complete = false;
            Advance(_position);
        }
        catch (Exception) { }
    }
}
