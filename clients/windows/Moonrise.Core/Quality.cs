using System.Globalization;

namespace Moonrise.Core;

/// <summary>
/// A streaming quality: Original (the copy, when the server can) or a preset the server transcodes
/// to (<c>quality=</c> bits per second). The Mac app's presets and rules
/// (clients/apple/Moonrise/Models/Settings.swift, QualityOption).
/// </summary>
public sealed record QualityOption(string Label, long? Bitrate, int? Height = null)
{
    public static readonly QualityOption Original = new("Original", null);

    public static readonly IReadOnlyList<QualityOption> Presets =
    [
        new("4K · 40 Mbps", 40_000_000, 2160),
        new("1080p · 20 Mbps", 20_000_000, 1080),
        new("1080p · 10 Mbps", 10_000_000, 1080),
        new("720p · 4 Mbps", 4_000_000, 720),
        new("480p · 1.5 Mbps", 1_500_000, 480),
    ];

    public static readonly IReadOnlyList<QualityOption> All = [Original, .. Presets];

    /// <summary>
    /// Original plus only the presets below the source: less bandwidth (as the server's ladder, which
    /// has no rung within 90 % of the source's bitrate) and no larger picture, by class (a 1920×800
    /// scope film still counts as 1080p).
    /// </summary>
    public static IReadOnlyList<QualityOption> Options(long? sourceBitrate, int? sourceHeight = null) =>
        [Original, .. Presets.Where(p =>
        {
            if (sourceBitrate is > 0 && p.Bitrate!.Value >= sourceBitrate.Value * 0.9) return false;
            if (sourceHeight is > 0 && p.Height is { } ph)
            {
                var h = sourceHeight.Value;
                var tier = h > 1080 ? 2160 : h > 720 ? 1080 : h > 480 ? 720 : 480;
                if (ph > tier) return false;
            }
            return true;
        })];

    public static IReadOnlyList<QualityOption> Options(MediaFile? version) => Options(version?.Bitrate, version?.VideoHeight);

    /// <summary>This, or the best option the source has when this is above it (Original when none is).</summary>
    public QualityOption CappedTo(MediaFile? version)
    {
        var options = Options(version);
        if (options.Contains(this)) return this;
        return options.FirstOrDefault(o => o != Original && (o.Bitrate ?? long.MaxValue) <= (Bitrate ?? long.MaxValue)) ?? Original;
    }

    /// <summary>Approximate transfer size: the file's size for Original, bitrate × runtime otherwise.</summary>
    public long? EstimatedBytes(double? runtime, long? sourceSize)
    {
        if (Bitrate == null) return sourceSize;
        if (runtime is not > 0) return null;
        return (long)(Bitrate.Value / 8.0 * runtime.Value);
    }

    /// <summary>The menu's second line: "1.23 GB" for Original, "≈ 450 MB" for a preset.</summary>
    public string? SizeLabel(double? runtime, long? sourceSize, CultureInfo? culture = null) =>
        EstimatedBytes(runtime, sourceSize) is { } b ? (Bitrate == null ? "" : "≈ ") + Format.FileSize(b, culture) : null;

    /// <summary>The option with this label (what Settings keeps); an unknown one is 1080p · 10 Mbps, as the Mac's.</summary>
    public static QualityOption Find(string? label) =>
        All.FirstOrDefault(o => o.Label == label) ?? Presets[2];
}
