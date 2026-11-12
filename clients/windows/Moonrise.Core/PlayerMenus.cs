namespace Moonrise.Core;

/// <summary>The player's Episodes panel (Mac: EpisodePickerView): the show's episodes by season, in airing order.</summary>
public static class EpisodeGroups
{
    public sealed record Group(string Title, IReadOnlyList<Item> Episodes);

    /// <summary>Seasons in the order their first episode comes; "Season 2", or the season's own name.</summary>
    public static IReadOnlyList<Group> Seasons(IReadOnlyList<Item> episodes)
    {
        var order = new List<string>();
        var groups = new Dictionary<string, List<Item>>();
        foreach (var ep in episodes)
        {
            var key = ep.SeasonTitle ?? (ep.SeasonNo is { } n ? $"Season {n}" : "Episodes");
            if (!groups.TryGetValue(key, out var list)) { order.Add(key); groups[key] = list = []; }
            list.Add(ep);
        }
        return order.Select(k => new Group(k, groups[k])).ToList();
    }

    /// <summary>The second line of an episode's row: "Now Playing" for the one playing, else its runtime.</summary>
    public static string? Detail(Item episode, string? playingId) =>
        episode.Id == playingId ? "Now Playing" : Format.Runtime(episode.Runtime ?? episode.PresentFiles.FirstOrDefault()?.Duration);
}

/// <summary>Scrubbing thumbnails (Mac: ScrubThumbnails, ScrubPreview).</summary>
public static class ScrubPreview
{
    /// <summary>The preview's width on a computer (Mac: 220).</summary>
    public const double Width = 220;

    /// <summary>The preview's height for a picture of <paramref name="t"/>'s proportions.</summary>
    public static double Height(Thumbnails t) => Math.Round(Width / Math.Max(0.5, (double)t.Width!.Value / Math.Max(1, t.Height!.Value)));

    /// <summary>
    /// The preview's left edge, centred on the knob and kept inside the track: the scrubber's own x
    /// of the knob, its width, and the preview's width.
    /// </summary>
    public static double Left(double knobX, double trackWidth, double previewWidth = Width) =>
        trackWidth <= previewWidth ? (trackWidth - previewWidth) / 2 : Math.Clamp(knobX - previewWidth / 2, 0, trackWidth - previewWidth);
}

/// <summary>The quality a title starts at and the menu offers.</summary>
public static class QualityChoice
{
    /// <summary>
    /// The setting's quality, when the file playing is above it; a file at or below it plays as it
    /// is (Original), so the menu always has the quality playing checked.
    /// </summary>
    public static QualityOption Starting(string? setting, MediaFile? version)
    {
        var q = QualityOption.Find(setting ?? QualityOption.Original.Label);
        return QualityOption.Options(version).Contains(q) ? q : QualityOption.Original;
    }

    /// <summary>The <c>quality=</c> sent with the playback request: none for Original.</summary>
    public static long? Request(QualityOption q) => q.Bitrate;
}
