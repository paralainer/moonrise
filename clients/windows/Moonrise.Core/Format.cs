using System.Globalization;

namespace Moonrise.Core;

/// <summary>What the title pages and banners say about a title (the other apps' wording).</summary>
public static class Format
{
    /// <summary>"1h 34m", "48m".</summary>
    public static string? Runtime(double? seconds)
    {
        if (seconds is null or <= 0) return null;
        var minutes = (int)(seconds.Value / 60);
        return minutes >= 60 ? $"{minutes / 60}h {minutes % 60}m" : $"{minutes}m";
    }

    /// <summary>"1:02:05", "4:09".</summary>
    public static string Clock(double seconds)
    {
        var s = Math.Max(0, (long)seconds);
        var h = s / 3600; var m = (s % 3600) / 60; var sec = s % 60;
        return h > 0 ? $"{h}:{m:00}:{sec:00}" : $"{m}:{sec:00}";
    }

    /// <summary>"900 bytes", "512 KB", "48.3 MB", "1.23 GB": decimal units with as many digits as the Mac app's sizes.</summary>
    public static string FileSize(long bytes, CultureInfo? culture = null)
    {
        if (bytes < 1000) return Math.Max(bytes, 0) == 1 ? "1 byte" : $"{Math.Max(bytes, 0)} bytes";
        (string Unit, int Digits)[] units = [("KB", 0), ("MB", 1), ("GB", 2), ("TB", 2), ("PB", 2)];
        var value = bytes / 1000.0;
        var i = 0;
        // 999,950 bytes rounds to "1000 KB" otherwise.
        while (i < units.Length - 1 && Math.Round(value, units[i].Digits) >= 1000) { value /= 1000; i++; }
        var pattern = units[i].Digits == 0 ? "0" : "0." + new string('#', units[i].Digits);
        return $"{value.ToString(pattern, culture ?? CultureInfo.CurrentCulture)} {units[i].Unit}";
    }

    public static string? Resolution(int? width, int? height)
    {
        if (width is null) return null;
        var w = width.Value; var h = height ?? 0;
        return w >= 3200 || h >= 1700 ? "4K" : w >= 1200 || h >= 700 ? "HD" : "SD";
    }

    private static string? Hdr(string? range) => range switch
    {
        null or "SDR" or "Unknown" => null,
        var r when r.Contains("DOVI") => "Dolby Vision",
        var r when r.Contains("HDR10Plus") => "HDR10+",
        _ => "HDR",
    };

    private static string? Channels(int? ch) => ch switch { null => null, >= 8 => "7.1", >= 6 => "5.1", 2 => "Stereo", _ => "Mono" };

    /// <summary>Resolution, HDR, Atmos, channels and CC, from the file's tracks or the list's summary.</summary>
    public static IReadOnlyList<string> TechBadges(Item item)
    {
        var o = new List<string>();
        void Add(string? b) { if (b != null) o.Add(b); }
        var file = item.PresentFiles.FirstOrDefault();
        if (file != null && file.Tracks.Count > 0)
        {
            var v = file.Tracks.FirstOrDefault(t => t.IsVideo);
            if (v != null) { Add(Resolution(v.Width, v.Height)); Add(Hdr(v.VideoRangeType)); }
            var audio = file.Tracks.Where(t => t.IsAudio).ToList();
            var a = audio.FirstOrDefault(t => t.IsDefault == true) ?? audio.FirstOrDefault();
            if (a != null) { if (a.DisplayTitle?.Contains("Atmos") == true) o.Add("Atmos"); Add(Channels(a.Channels)); }
            if (file.Tracks.Any(t => t.IsSubtitle)) o.Add("CC");
        }
        else if (item.Media != null)
        {
            if (item.Media.Video is { } v) { Add(Resolution(v.Width, v.Height)); Add(Hdr(v.Range)); }
            var a = item.Media.Audio.FirstOrDefault(x => x.Default == true) ?? item.Media.Audio.FirstOrDefault();
            if (a != null) { if (a.Profile?.Contains("Atmos") == true) o.Add("Atmos"); Add(Channels(a.Channels)); }
            if (item.Media.Subtitles.Count > 0) o.Add("CC");
        }
        return o;
    }

    public static string? PremiereText(string? date, CultureInfo? culture = null)
    {
        if (date == null || date.Length < 10) return null;
        if (!DateTime.TryParse(date[..10], CultureInfo.InvariantCulture, DateTimeStyles.None, out var d)) return null;
        return d.ToString("d MMM yyyy", culture ?? CultureInfo.CurrentCulture);
    }

    /// <summary>"★ 7.9   IMDb 8.1   RT 94%   MC 78", or null.</summary>
    /// <summary>
    /// A title page's scores, as the Mac app: the title's own (an episode's, or the film's), the
    /// critics' scores topped up with the show's, IMDb staying the title's own. Null: nothing to show.
    /// </summary>
    public static PageRatings? RatingsForPage(Item item, Item? focus)
    {
        var own = (focus ?? item).Ratings;
        var show = item.IsSeries ? item.Ratings : null;
        var imdb = own?.Imdb?.Value;
        var rt = own?.RottenTomatoes?.Value ?? show?.RottenTomatoes?.Value ?? (focus?.CriticRating ?? item.CriticRating);
        var mc = own?.Metacritic?.Value ?? show?.Metacritic?.Value;
        var star = focus?.CommunityRating ?? item.CommunityRating;
        if (imdb == null && rt == null && mc == null && star == null) return null;
        var c = CultureInfo.InvariantCulture;
        return new PageRatings(imdb?.ToString("F1", c), star?.ToString("F1", c), rt is { } r ? $"{Math.Round(r)}%" : null, mc is { } m ? $"{Math.Round(m)}" : null);
    }
}

public sealed record PageRatings(string? Imdb, string? Star, string? RottenTomatoes, string? Metacritic);
