namespace Moonrise.Core;

/// <summary>
/// A playback failure in plain words: a short title, one line on what it means or what to
/// do, and the server's own text (ffprobe output, paths, codes) kept as Detail for Details. The rule
/// is shared with the Apple and Android apps: api/playback-problem-rules.json.
/// </summary>
public sealed record PlaybackProblem(PlaybackProblem.Kinds Kind, string? Detail)
{
    public enum Kinds { Unreachable, Preparing, Missing, Share, Damaged, Unindexed, Busy, Server, Unsupported }

    /// <summary>The kind's name in the shared cases ("unreachable", "damaged"…).</summary>
    public string Key => Kind.ToString().ToLowerInvariant();

    public string Title => Kind switch
    {
        Kinds.Unreachable => "Can't reach the server",
        Kinds.Preparing => "Still getting ready",
        Kinds.Missing => "File not found",
        Kinds.Share => "Can't reach the share",
        Kinds.Damaged => "Damaged file",
        Kinds.Unindexed => "Can't stream this file",
        Kinds.Busy => "Server busy",
        Kinds.Server => "Server problem",
        _ => "Can't play this video",
    };

    public string Message => Kind switch
    {
        Kinds.Unreachable => "Check that the Moonrise server is on and this device is on its network.",
        Kinds.Preparing => "The server is still reading this file. Try again in a minute.",
        Kinds.Missing => "The server can't find this video's file. It may have been moved or deleted, or its share is offline.",
        Kinds.Share => "The server can't read the network share this video is on. Check that it's on and connected.",
        Kinds.Damaged => "This file is damaged or incomplete, so it can't play. Replacing it fixes this.",
        Kinds.Unindexed => "The file has no index to stream from. Saving it again (remuxing) fixes this.",
        Kinds.Busy => "The server is converting other videos right now. Try again in a moment.",
        Kinds.Server => "The server ran into a problem with this video. Try again.",
        _ => "The server can't play this video on this device.",
    };

    /// <param name="status">The HTTP status of the server's answer; null when nothing answered.</param>
    /// <param name="text">Its body, an offer's reasons, or a file's broken_reason.</param>
    /// <param name="pending">An offer whose file is still being indexed.</param>
    public static PlaybackProblem Classify(int? status, string? text, bool pending = false)
    {
        var raw = (text ?? "").Trim();
        return new PlaybackProblem(KindOf(status, raw, pending), raw.Length == 0 ? null : raw);
    }

    public static Kinds KindOf(int? status, string text, bool pending)
    {
        if (status == null) return Kinds.Unreachable;
        if (pending) return Kinds.Preparing;
        var t = text.ToLowerInvariant();
        bool Has(params string[] words) => words.Any(t.Contains);
        if (Has("no such file or directory")) return Kinds.Missing;
        if (Has("can't be reached", "not mounted", "isn't mounted")) return Kinds.Share;
        if (Has("damaged", "incomplete", "truncated", "ebml", "invalid data found", "moov atom not found", "end of file")) return Kinds.Damaged;
        if (Has("keyframe index pending")) return Kinds.Preparing;
        if (Has("keyframe index", "no cues")) return Kinds.Unindexed;
        if (Has("slots busy")) return Kinds.Busy;
        // (A title the server lists but can't play from any file it finds.)
        if (status == 404) return Kinds.Missing;
        if (status >= 500) return Kinds.Server;
        return Kinds.Unsupported;
    }

    /// <summary>A title page's note for a file the server marked broken (its broken_reason).</summary>
    public static PlaybackProblem Broken(string? reason)
    {
        var p = Classify(422, reason);
        // A broken file is one the server couldn't read: damaged, unless the reason says otherwise.
        return p.Kind == Kinds.Unsupported ? p with { Kind = Kinds.Damaged } : p;
    }

    /// <summary>Any error from asking for a stream: an HTTP refusal, else no answer.</summary>
    public static PlaybackProblem Of(Exception e) => e is MoonriseHttpException h ? Classify(h.Code, h.Body) : Classify(null, e.Message);

    /// <summary>An offer the server answered but can't serve (supported: false).</summary>
    public static PlaybackProblem Of(Playback p) => Classify(200, string.Join("; ", p.Reasons), p.Pending == true);

    /// <summary>
    /// Why a title can't play, for its page (null: it can, or nothing says): every file the server
    /// lists is damaged, or none was found on disk. A title with a healthy version plays that one.
    /// </summary>
    public static PlaybackProblem? Unplayable(Item item)
    {
        var all = item.Files ?? [];
        if (all.Count == 0) return null;
        var present = item.PresentFiles;
        if (present.Count > 0)
            return present.All(f => f.IsBroken) ? Broken(present.FirstOrDefault(f => f.BrokenReason != null)?.BrokenReason) : null;
        var names = all.Select(f => f.Name).Where(n => !string.IsNullOrEmpty(n));
        return new PlaybackProblem(Kinds.Missing, string.Concat(names.Select(n => n + "\n")) + "Not found at the server's last scan.");
    }
}
