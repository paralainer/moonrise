using System.Text.Json;
using System.Text.Json.Serialization;

namespace Moonrise.Core;

public enum SubtitleSize { Small, Medium, Large, Huge }

public static class SubtitleSizes
{
    /// <summary>The text's scale (Mac: SubtitleSize.scale).</summary>
    public static double Scale(this SubtitleSize s) => s switch
    {
        SubtitleSize.Small => 0.8,
        SubtitleSize.Large => 1.25,
        SubtitleSize.Huge => 1.55,
        _ => 1.0,
    };

    public static string Title(this SubtitleSize s) => s.ToString();
}

public static class SkipModes
{
    public static string Title(this SkipMode m) => m switch
    {
        SkipMode.Auto => "Skip Automatically",
        SkipMode.Button => "Show Skip Button",
        _ => "Off",
    };
}

/// <summary>
/// This device's playback preferences, as the Mac app's AppSettings keeps them (the same defaults),
/// in a JSON file in the app's data folder. Picking tracks or a quality in the player changes them.
/// </summary>
public sealed class PlayerSettings
{
    /// <summary>Settings' language choices (the Mac app's list).</summary>
    public static readonly IReadOnlyList<(string Code, string Name)> Languages =
    [
        ("en", "English"), ("ru", "Russian"), ("uk", "Ukrainian"), ("es", "Spanish"), ("fr", "French"),
        ("de", "German"), ("it", "Italian"), ("pt", "Portuguese"), ("nl", "Dutch"), ("pl", "Polish"),
        ("sv", "Swedish"), ("ja", "Japanese"), ("ko", "Korean"), ("zh", "Chinese"), ("ar", "Arabic"),
        ("he", "Hebrew"), ("tr", "Turkish"), ("el", "Greek"), ("cs", "Czech"), ("fi", "Finnish"),
    ];

    /// <summary>The choices, plus <paramref name="current"/> when it isn't one of them.</summary>
    public static IReadOnlyList<(string Code, string Name)> LanguageChoices(string current) =>
        string.IsNullOrEmpty(current) || Languages.Any(l => l.Code == current)
            ? Languages
            : [.. Languages, (current, TrackLabels.LanguageName(current) ?? current.ToUpperInvariant())];

    /// <summary>The Auto-Skip countdown's choices (0: none, skipped at once).</summary>
    public static readonly IReadOnlyList<double> AutoSkipDelays = [0, 3, 5, 10];

    public string StreamingQuality { get; set; } = QualityOption.Original.Label;
    public Dictionary<SegmentKind, SkipMode> SkipModes { get; set; } = [];
    public double AutoSkipDelay { get; set; } = 3;
    /// <summary>ISO 639-1; "" plays each title's own default.</summary>
    public string PreferredAudioLanguage { get; set; } = "";
    public string PreferredSubtitleLanguage { get; set; } = "en";
    public bool AutoSelectSubtitles { get; set; }
    /// <summary>The subtitle picked last was SDH: prefer such a track in the preferred language.</summary>
    public bool PreferSdhSubtitles { get; set; }
    public SubtitleSize SubtitleSize { get; set; } = SubtitleSize.Medium;
    public bool SubtitleBackground { get; set; }
    /// <summary>The viewer's subtitle delay for a track the server keeps no timing for, by "media id|track title".</summary>
    public Dictionary<string, double> SubtitleNudges { get; set; } = [];

    public SkipMode SkipMode(SegmentKind kind) => SkipModes.TryGetValue(kind, out var m) ? m : kind.DefaultMode();

    /// <summary>
    /// The viewer picked subtitles in the player: that language (and kind) from now on, and on. Off,
    /// or forced subtitles (foreign dialogue only, which come on by themselves), turn them off.
    /// </summary>
    public void SubtitlePicked(string? language, string? title)
    {
        if (title != null && !TrackRules.IsForced(title))
        {
            var lang = TrackRules.SubtitleLanguage(language, title);
            if (lang == null) return;
            PreferredSubtitleLanguage = lang;
            PreferSdhSubtitles = TrackRules.IsSDH(title);
            AutoSelectSubtitles = true;
        }
        else AutoSelectSubtitles = false;
    }

    /// <summary>The viewer picked an audio track: its language from now on.</summary>
    public void AudioPicked(string? language)
    {
        if (Moonrise.Core.Languages.Normalized(language) is { } lang) PreferredAudioLanguage = lang;
    }

    private static readonly JsonSerializerOptions Json = new()
    {
        WriteIndented = true,
        Converters = { new JsonStringEnumConverter() },
    };

    public string ToJson() => JsonSerializer.Serialize(this, Json);

    /// <summary>Settings from their JSON; anything missing or unreadable is the default.</summary>
    public static PlayerSettings FromJson(string? json)
    {
        if (string.IsNullOrWhiteSpace(json)) return new();
        try { return JsonSerializer.Deserialize<PlayerSettings>(json, Json) ?? new(); }
        catch (JsonException) { return new(); }
    }

    public static PlayerSettings Load(string path)
    {
        try { return File.Exists(path) ? FromJson(File.ReadAllText(path)) : new(); }
        catch (IOException) { return new(); }
        catch (UnauthorizedAccessException) { return new(); }
    }

    /// <summary>Written whole to a new file first, so a crash mid-write never leaves half a file.</summary>
    public void Save(string path)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        var tmp = path + ".tmp";
        File.WriteAllText(tmp, ToJson());
        File.Move(tmp, path, overwrite: true);
    }
}
