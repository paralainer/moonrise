using System.Globalization;
using System.Text.RegularExpressions;

namespace Moonrise.Core;

// How the player's menus name a file's tracks and how the starting tracks are picked: the same
// rules as clients/android/core/.../Tracks.kt and clients/apple/Moonrise/Player/TrackRules.swift.

public static class TrackLabels
{
    private const string AudioTechnical = @"dd|ddp|dd\+|ac-?3|e-?ac-?3|dts(-hd)?|ma|truehd|atmos|aac|lc|flac|opus|mp3|pcm|lpcm|stereo|mono|surround|kbps|kb/s|\d+(\.\d)?(ch)?";
    private const string SubtitleTechnical = "srt|subrip|ass|ssa|vtt|webvtt|pgs|pgssub|sup|vobsub|dvdsub|dvb_subtitle|mov_text|utf-?8|text|full";

    /// <summary>The language's name ("Russian"), or null for none or "und".</summary>
    public static string? LanguageName(string? code)
    {
        var c = code?.Trim();
        if (string.IsNullOrEmpty(c) || c == "und") return null;
        var n = Languages.Normalized(c) ?? c;
        try
        {
            var name = new CultureInfo(n).EnglishName;
            var open = name.IndexOf('(');
            if (open > 0) name = name[..open].Trim();
            return name.Equals(n, StringComparison.OrdinalIgnoreCase) ? c.ToUpperInvariant() : name;
        }
        catch (CultureNotFoundException) { return c.ToUpperInvariant(); }
    }

    private static string? CleanTitle(Track t, string words)
    {
        var title = t.Title?.Trim();
        if (string.IsNullOrEmpty(title)) return null;
        var technical = new Regex($@"^(?i)((\b({words})\b)|[@+\-–,/.\s])+$");
        var open = title.IndexOf('(');
        var close = title.LastIndexOf(')');
        if (open >= 0 && close > open && (string.IsNullOrWhiteSpace(title[..open]) || technical.IsMatch(title[..open])))
            title = title[(open + 1)..close].Trim();
        if (title.Length == 0 || technical.IsMatch(title)) return null;
        if (string.Equals(LanguageName(t.Language), title, StringComparison.OrdinalIgnoreCase)) return null;
        if (string.Equals(t.Language, title, StringComparison.OrdinalIgnoreCase)) return null;
        return title;
    }

    public static string SubtitleLabel(Track t)
    {
        var lang = LanguageName(t.Language);
        var parts = new List<string> { lang ?? t.Title ?? $"Track {t.Index}" };
        if (lang != null && CleanTitle(t, SubtitleTechnical) is { } c) parts.Add(c);
        if (t.IsForced == true) parts.Add("Forced");
        if (t.IsExternal == true) parts.Add("External");
        return string.Join(" · ", parts);
    }

    public static string AudioName(Track t) => LanguageName(t.Language) ?? t.Title ?? t.DisplayTitle ?? $"Track {t.Index}";

    public static string? AudioDetail(Track t)
    {
        var c = t.Codec?.ToLowerInvariant();
        string? format = c switch
        {
            null => null,
            "ac3" => "Dolby Digital",
            "eac3" => t.Profile?.Contains("Atmos") == true ? "Dolby Atmos" : "Dolby Digital Plus",
            "truehd" => t.Profile?.Contains("Atmos") == true ? "Dolby Atmos" : "Dolby TrueHD",
            "dts" => t.Profile?.Contains("MA") == true ? "DTS-HD MA" : "DTS",
            "aac" => "AAC", "flac" => "FLAC", "opus" => "Opus",
            _ => c.ToUpperInvariant(),
        };
        string? layout = t.Channels switch
        {
            null => null,
            1 => "Mono",
            2 => "Stereo",
            int ch when ch > 2 => $"{ch - 1}.1",
            _ => null,
        };
        var sound = string.Join(" ", new[] { format, layout }.Where(x => x != null));
        var parts = new[] { CleanTitle(t, AudioTechnical), sound.Length == 0 ? null : sound }.Where(x => x != null).ToList();
        return parts.Count == 0 ? null : string.Join(" · ", parts);
    }
}

public sealed record AudioCandidate(int Id, string? Language, bool IsDefault, string Name);
/// <summary>A subtitle track to pick from: text and picture tracks alike (the app draws both).</summary>
public sealed record SubtitleCandidate(string Id, string Title, string? Language, int? TrackIndex);

public sealed record TrackPick
{
    public int? Index { get; init; }
    public string? Language { get; init; }
    public string? Title { get; init; }
    public bool? Forced { get; init; }
    public bool? Sdh { get; init; }
}

public sealed record TrackMemory
{
    public bool SameFile { get; init; }
    public TrackPick? Audio { get; init; }
    public TrackPick? Subtitle { get; init; }
    public bool SubtitleOff { get; init; }
}

/// <summary>The tracks playing, as a report sends them: a null subtitle is Off.</summary>
public sealed record TrackChoice(TrackPick? Audio, TrackPick? Subtitle);

public abstract record SubtitleMemory
{
    public sealed record Use(string Id) : SubtitleMemory;
    public sealed record OffHere : SubtitleMemory;
    public sealed record UsualOff : SubtitleMemory;
    public sealed record Usual : SubtitleMemory;
}

public static class TrackRules
{
    private static readonly Regex ExtraAudio = new("(?i)comment|descri|narrat|director");
    private static readonly Regex Sdh = new(@"(?i)\b(sdh|cc|hearing)\b");

    public static int? PreferredAudio(IReadOnlyList<AudioCandidate> tracks, string? preferred)
    {
        var want = Languages.Normalized(preferred);
        if (want == null) return null;
        var matching = tracks.Where(t => Languages.Normalized(t.Language) == want).ToList();
        var main = matching.Where(t => !ExtraAudio.IsMatch(t.Name)).ToList();
        return (main.FirstOrDefault(t => t.IsDefault) ?? main.FirstOrDefault() ?? matching.FirstOrDefault())?.Id;
    }

    public static bool IsForced(string title) => title.Contains("forced", StringComparison.OrdinalIgnoreCase);
    public static bool IsSDH(string title) => Sdh.IsMatch(title);

    /// <summary>A subtitle track's language; for a file without one, the code in its name ("Film.en.srt").</summary>
    public static string? SubtitleLanguage(string? language, string title)
    {
        var n = Languages.Normalized(language);
        if (n != null) return n;
        var parts = title.ToLowerInvariant().Split('.');
        foreach (var p in parts.Take(parts.Length - 1).Reverse())
            if (p.Length is 2 or 3) return Languages.Normalized(p);
        return null;
    }

    public static string? SubtitleDetail(string title)
    {
        var d = string.Join(" · ", title.Split(" · ").Skip(1)).Trim();
        return d.Length == 0 ? null : d;
    }

    public static string? RememberedSubtitleTitle(string? title)
    {
        var t = title?.Trim();
        return string.IsNullOrEmpty(t) ? null : t;
    }

    private static bool SameText(string? want, string? have)
    {
        var a = want?.Trim().ToLowerInvariant();
        return !string.IsNullOrEmpty(a) && have != null && have.Contains(a, StringComparison.OrdinalIgnoreCase);
    }

    public static int? RememberedAudio(IReadOnlyList<AudioCandidate> tracks, TrackMemory? memory)
    {
        var want = memory?.Audio;
        if (want == null) return null;
        var lang = Languages.Normalized(want.Language);
        if (memory!.SameFile && want.Index != null)
        {
            var t = tracks.FirstOrDefault(x => x.Id == want.Index);
            if (t != null && (lang == null || Languages.Normalized(t.Language) == lang)) return t.Id;
        }
        var same = tracks.Where(t => Languages.Normalized(t.Language) == lang).ToList();
        if (lang == null && (same.Count == 0 || want.Title == null)) return null;
        var titled = same.FirstOrDefault(t => SameText(want.Title, t.Name));
        if (titled != null) return titled.Id;
        if (lang == null) return null;
        var extra = want.Title != null && ExtraAudio.IsMatch(want.Title);
        var kind = same.Where(t => ExtraAudio.IsMatch(t.Name) == extra).ToList();
        return (kind.FirstOrDefault(t => t.IsDefault) ?? kind.FirstOrDefault())?.Id;
    }

    public static SubtitleMemory RememberedSubtitle(IReadOnlyList<SubtitleCandidate> options, TrackMemory? memory)
    {
        if (memory == null) return new SubtitleMemory.Usual();
        if (memory.SubtitleOff) return memory.SameFile ? new SubtitleMemory.OffHere() : new SubtitleMemory.UsualOff();
        var want = memory.Subtitle;
        if (want == null) return new SubtitleMemory.Usual();
        var lang = Languages.Normalized(want.Language);
        if (memory.SameFile && want.Index != null)
        {
            var o = options.FirstOrDefault(x => x.TrackIndex == want.Index);
            if (o != null && (lang == null || SubtitleLanguage(o.Language, o.Title) == lang)) return new SubtitleMemory.Use(o.Id);
        }
        var forced = want.Forced ?? false;
        var pool = options.Where(o => SubtitleLanguage(o.Language, o.Title) == lang && IsForced(o.Title) == forced).ToList();
        var title = RememberedSubtitleTitle(want.Title);
        if (lang == null && (pool.Count == 0 || title == null)) return new SubtitleMemory.Usual();
        var sdh = want.Sdh ?? false;
        var found = pool.FirstOrDefault(o => IsSDH(o.Title) == sdh && SameText(title, o.Title))
            ?? pool.FirstOrDefault(o => SameText(title, o.Title) && lang != null)
            ?? pool.FirstOrDefault(o => IsSDH(o.Title) == sdh && lang != null)
            ?? (lang != null ? pool.FirstOrDefault() : null);
        return found != null ? new SubtitleMemory.Use(found.Id) : new SubtitleMemory.Usual();
    }

    /// <summary>
    /// The subtitle a title starts with, as the other apps: subtitles in the preferred language when
    /// on (the kind picked last first; text and picture tracks alike, in the file's order), else
    /// forced subtitles in the audio's language. Null: Off.
    /// </summary>
    public static SubtitleCandidate? UsualSubtitle(IReadOnlyList<SubtitleCandidate> options, bool on, string? preferredLanguage, bool preferSdh, string? audioLanguage)
    {
        var lang = Languages.Normalized(preferredLanguage);
        if (on && lang != null)
        {
            var pool = options.Where(o => SubtitleLanguage(o.Language, o.Title) == lang && !IsForced(o.Title)).ToList();
            var o = pool.FirstOrDefault(x => IsSDH(x.Title) == preferSdh) ?? pool.FirstOrDefault();
            if (o != null) return o;
        }
        var forced = options.Where(o => IsForced(o.Title)).ToList();
        var audio = Languages.Normalized(audioLanguage);
        return audio != null ? forced.FirstOrDefault(o => SubtitleLanguage(o.Language, o.Title) == audio) : forced.FirstOrDefault();
    }
}

public static class TrackReport
{
    /// <summary>The body of a tracks report: picks with no null fields, the subtitle key always present.</summary>
    public static Dictionary<string, object?> Json(TrackChoice c)
    {
        var d = new Dictionary<string, object?>();
        if (c.Audio != null) d["audio"] = Pick(c.Audio);
        d["subtitle"] = c.Subtitle != null ? Pick(c.Subtitle) : null;
        return d;
    }

    private static Dictionary<string, object?> Pick(TrackPick p)
    {
        var d = new Dictionary<string, object?>();
        if (p.Index != null) d["index"] = p.Index;
        if (p.Language != null) d["language"] = p.Language;
        if (p.Title != null) d["title"] = p.Title;
        if (p.Forced != null) d["forced"] = p.Forced;
        if (p.Sdh != null) d["sdh"] = p.Sdh;
        return d;
    }
}
