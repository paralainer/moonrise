using System.Text.Json.Serialization;

namespace Moonrise.Core;

// The server's JSON (/v1/...): snake_case, times in seconds, dates RFC 3339. Only the fields the
// client uses are declared; the decoder ignores the rest. Fields the contract guarantees are
// `required`, so a renamed one fails the shared-examples test.

public sealed record ServerInfo
{
    public required string ServerId { get; init; }
    public required string Name { get; init; }
    public required string Version { get; init; }
    public IReadOnlyList<string> Capabilities { get; init; } = [];
    /// <summary>"unclaimed": a new server that still needs setting up; "claimed"; null on older servers.</summary>
    public string? Setup { get; init; }

    [JsonIgnore] public bool Unclaimed => Setup == "unclaimed";
    /// <summary>Check a capability before using the feature it names, so older servers keep working.</summary>
    public bool Has(string capability) => Capabilities.Contains(capability);
}

public sealed record User
{
    public required string Id { get; init; }
    public required string Name { get; init; }
    public bool? IsAdmin { get; init; }
}

public sealed record AuthResult
{
    public required string Token { get; init; }
    public required User User { get; init; }
}

public sealed record DeviceCode
{
    [JsonPropertyName("device_code")] public required string Code { get; init; }
    public required string UserCode { get; init; }
    public required string VerifyUrl { get; init; }
    public required string VerifyUrlComplete { get; init; }
    public required int Interval { get; init; }
    public required int ExpiresIn { get; init; }
}

public sealed record PollAnswer
{
    public required string Status { get; init; }
    public string? Token { get; init; }
    public User? User { get; init; }
}

public enum DevicePollState { Pending, SlowDown, Expired, Approved }
public sealed record DevicePoll(DevicePollState State, AuthResult? Auth = null);

public sealed record ItemImages
{
    public string? Poster { get; init; }
    public IReadOnlyList<string>? Backdrop { get; init; }
    public string? Logo { get; init; }
    public string? Thumb { get; init; }
    public string? SeasonPoster { get; init; }
}

public sealed record UserData
{
    public double? Position { get; init; }
    public string? Version { get; init; }
    public bool? Played { get; init; }
    public int? PlayCount { get; init; }
    public double? PlayedPercentage { get; init; }
    public int? UnplayedItemCount { get; init; }
    public string? LastPlayedAt { get; init; }
}

public sealed record Track
{
    public required int Index { get; init; }
    public required string Kind { get; init; }
    public string? Codec { get; init; }
    public string? Language { get; init; }
    public string? Title { get; init; }
    public string? DisplayTitle { get; init; }
    public bool? IsDefault { get; init; }
    public bool? IsForced { get; init; }
    public bool? IsText { get; init; }
    public int? Width { get; init; }
    public int? Height { get; init; }
    public int? Channels { get; init; }
    public string? VideoRangeType { get; init; }
    public string? Profile { get; init; }
    public bool? IsExternal { get; init; }
    public bool? IsHearingImpaired { get; init; }

    [JsonIgnore] public bool IsAudio => Kind.Equals("audio", StringComparison.OrdinalIgnoreCase);
    [JsonIgnore] public bool IsSubtitle => Kind.Equals("subtitle", StringComparison.OrdinalIgnoreCase);
    [JsonIgnore] public bool IsVideo => Kind.Equals("video", StringComparison.OrdinalIgnoreCase);
}

public sealed record VersionInfo
{
    public string? Label { get; init; }
    public string? Details { get; init; }
    public string? Edition { get; init; }
    public string? Hdr { get; init; }
    public string? Resolution { get; init; }
    public IReadOnlyList<string>? AudioLanguages { get; init; }
}

/// <summary>A file of a title: one version of it.</summary>
public sealed record MediaFile
{
    public required string Id { get; init; }
    public string? Name { get; init; }
    public double? Duration { get; init; }
    public long? Size { get; init; }
    public long? Bitrate { get; init; }
    public string? VersionLabel { get; init; }
    public bool? Missing { get; init; }
    public bool? Broken { get; init; }
    public string? BrokenReason { get; init; }
    public double? Position { get; init; }
    public VersionInfo? Version { get; init; }
    public IReadOnlyList<Track> Tracks { get; init; } = [];

    [JsonIgnore] public bool IsBroken => Broken == true;
    [JsonIgnore] public string VersionTitle => Version?.Label ?? VersionLabel ?? Name ?? "Version";
    [JsonIgnore] public string? VersionDetails => IsBroken ? "Damaged or incomplete file · can't play" : Version?.Details;
    [JsonIgnore] public int? VideoHeight => Tracks.FirstOrDefault(t => t.IsVideo)?.Height;
}

public sealed record Person
{
    public required string Id { get; init; }
    public string? Name { get; init; }
    public string? Kind { get; init; }
    public string? Role { get; init; }
    public string? Image { get; init; }

    [JsonIgnore]
    public string Group => Kind?.ToLowerInvariant() switch
    {
        "actor" or "cast" or "guest_star" or "gueststar" => "Actor",
        "director" => "Director",
        "writer" or "screenplay" => "Writer",
        null => "",
        _ => char.ToUpperInvariant(Kind![0]) + Kind[1..],
    };
}

public sealed record Score { public double? Value { get; init; } public long? Votes { get; init; } }

public sealed record Ratings
{
    public Score? Imdb { get; init; }
    public Score? RottenTomatoes { get; init; }
    public Score? Metacritic { get; init; }
}

public sealed record ContinueInfo
{
    public required string Reason { get; init; }
    public string? At { get; init; }
    public string? From { get; init; }
}

public sealed record CollectionRef { public int Id { get; init; } public string? Name { get; init; } }

public sealed record MediaSummaryVideo { public string? Codec { get; init; } public int? Width { get; init; } public int? Height { get; init; } public string? Range { get; init; } }
public sealed record MediaSummaryAudio { public string? Codec { get; init; } public int? Channels { get; init; } public string? Profile { get; init; } public bool? Default { get; init; } }
public sealed record MediaSummarySubtitle { public string? Codec { get; init; } public string? Language { get; init; } }
public sealed record MediaSummary
{
    public MediaSummaryVideo? Video { get; init; }
    public IReadOnlyList<MediaSummaryAudio> Audio { get; init; } = [];
    public IReadOnlyList<MediaSummarySubtitle> Subtitles { get; init; } = [];
}

public sealed record Item
{
    public required string Id { get; init; }
    /// <summary>"movie", "series", "season" or "episode".</summary>
    public required string Kind { get; init; }
    public string? Title { get; init; }
    public int? Year { get; init; }
    public string? Overview { get; init; }
    public string? Tagline { get; init; }
    public string? OfficialRating { get; init; }
    public double? CommunityRating { get; init; }
    public double? CriticRating { get; init; }
    public Ratings? Ratings { get; init; }
    public IReadOnlyList<string>? Genres { get; init; }
    public double? Runtime { get; init; }
    public string? PremiereDate { get; init; }
    public IReadOnlyList<string>? Studios { get; init; }
    public IReadOnlyList<Person>? People { get; init; }
    public ItemImages? Images { get; init; }
    public ItemImages? ParentImages { get; init; }
    public string? SeriesId { get; init; }
    public string? SeriesTitle { get; init; }
    public string? SeasonId { get; init; }
    public string? SeasonTitle { get; init; }
    public int? SeasonNo { get; init; }
    public int? EpisodeNo { get; init; }
    public int? EpisodeEnd { get; init; }
    public int? Part { get; init; }
    public int? Parts { get; init; }
    public string? LibraryId { get; init; }
    public string? AddedAt { get; init; }
    public CollectionRef? Collection { get; init; }
    public UserData? User { get; init; }
    public IReadOnlyList<MediaFile>? Files { get; init; }
    public int? Versions { get; init; }
    public MediaSummary? Media { get; init; }
    public IReadOnlyDictionary<string, string>? ProviderIds { get; init; }
    [JsonPropertyName("continue")] public ContinueInfo? ContinueInfo { get; init; }

    [JsonIgnore] public bool IsEpisode => Kind == "episode";
    [JsonIgnore] public bool IsSeries => Kind == "series";
    [JsonIgnore] public bool IsSeason => Kind == "season";
    [JsonIgnore] public bool IsMovie => Kind == "movie";
    [JsonIgnore] public bool IsPlayable => IsMovie || IsEpisode;
    [JsonIgnore] public IReadOnlyList<MediaFile> PresentFiles => (Files ?? []).Where(f => f.Missing != true).ToList();
    [JsonIgnore] public bool HasVersions => PresentFiles.Count > 1;

    /// <summary>"3", or "12–13" for a file with two episodes.</summary>
    [JsonIgnore]
    public string? EpisodeNumberText => EpisodeNo is { } n
        ? (EpisodeEnd is { } e && e > n ? $"{n}–{e}" : $"{n}")
        : null;

    /// <summary>An episode in a season's list: "4. Title", "12–13. Title", "23. Part 2 · Title".</summary>
    [JsonIgnore]
    public string EpisodeListTitle =>
        (EpisodeNumberText is { } n ? $"{n}. " : "") + (Part is { } p ? $"Part {p} · " : "") + (Title ?? "Untitled");

    /// <summary>"S1 · E3", "S3 · E12–13", "S5 · E23 · Part 2".</summary>
    [JsonIgnore]
    public string? EpisodeLabel
    {
        get
        {
            if (!IsEpisode) return null;
            string? e = EpisodeNo is { } n ? (EpisodeEnd is { } end && end > n ? $"E{n}–{end}" : $"E{n}") : null;
            return string.Join(" · ", new[] { SeasonNo is { } s ? $"S{s}" : null, e, Part is { } p ? $"Part {p}" : null }.Where(x => x != null));
        }
    }

    /// <summary>The title under a card: the show's name for an episode or season.</summary>
    [JsonIgnore]
    public string CardTitle => ((IsEpisode || IsSeason) ? SeriesTitle : null) ?? Title ?? "Untitled";

    /// <summary>The second line under a card: year, season name, or "S2 · E6 · Title".</summary>
    [JsonIgnore]
    public string CardSubtitle =>
        IsSeason ? Title ?? "Untitled"
        : IsEpisode ? string.Join(" · ", new[] { string.IsNullOrEmpty(EpisodeLabel) ? null : EpisodeLabel, Title }.Where(x => x != null))
        : Year?.ToString() ?? " ";

    /// <summary>The resume position, when there is one (> 0), watched before or not: as the Apple apps.</summary>
    [JsonIgnore]
    public double? ResumeSeconds => User?.Position is > 0 ? User.Position : null;

    /// <summary>0-1 progress for the bar on cards; a Continue Watching resume counts even when watched before.</summary>
    [JsonIgnore]
    public double? Progress
    {
        get
        {
            // A watched title shows no bar on its page or in other rows, only as a Continue Watching resume.
            double? p = User?.Position is >= 1 && (User.Played != true || ContinueInfo?.Reason == "resume") ? User.Position : null;
            if (p is null) return null;
            double? total = Runtime ?? PresentFiles.FirstOrDefault()?.Duration;
            if (total is null) return User?.PlayedPercentage / 100;
            return total > 0 ? Math.Clamp(p.Value / total.Value, 0, 1) : null;
        }
    }

    [JsonIgnore] public string? PosterImage => Images?.Poster ?? ParentImages?.Poster;
    [JsonIgnore] public string? BackdropImage => Images?.Backdrop?.FirstOrDefault() ?? ParentImages?.Backdrop?.FirstOrDefault();
    [JsonIgnore] public string? LogoImage => Images?.Logo ?? ParentImages?.Logo;
    [JsonIgnore] public string? CardImage => Images?.Thumb ?? ParentImages?.Thumb ?? BackdropImage ?? PosterImage;
}

public sealed record Library
{
    public required string Id { get; init; }
    public required string Name { get; init; }
    public string? CollectionType { get; init; }
    public string? Backdrop { get; init; }
    public int? ItemCount { get; init; }
}

public sealed record ListAnswer<T> { public required IReadOnlyList<T> Items { get; init; } public int? Total { get; init; } }

public sealed record HomeRow
{
    public required string Type { get; init; }
    public required string Title { get; init; }
    public required IReadOnlyList<Item> Items { get; init; }
}

public sealed record Home
{
    [JsonPropertyName("continue")] public IReadOnlyList<Item>? ContinueWatching { get; init; }
    public IReadOnlyList<Item> Resume { get; init; } = [];
    public IReadOnlyList<Item> NextUp { get; init; } = [];
    public IReadOnlyList<HomeRow> LatestTypes { get; init; } = [];

    /// <summary>The Continue Watching row: the server's, or (older servers) its resume points, then
    /// next episodes of shows not already in progress.</summary>
    [JsonIgnore]
    public IReadOnlyList<Item> ContinueRow
    {
        get
        {
            if (ContinueWatching != null) return ContinueWatching;
            var inProgress = Resume.Where(i => i.IsEpisode && i.User?.Played != true).Select(i => i.SeriesId).Where(s => s != null).ToHashSet();
            return Resume.Concat(NextUp.Where(i => !inProgress.Contains(i.SeriesId))).ToList();
        }
    }
}

public sealed record PlaybackVideo { public required string Codec { get; init; } public required string Range { get; init; } public int? Width { get; init; } public int? Height { get; init; } }
public sealed record PlaybackTrack
{
    public required int Index { get; init; }
    public required string Name { get; init; }
    public string? Language { get; init; }
    public bool? Default { get; init; }
    public bool? Converted { get; init; }
    /// <summary>Subtitles: a text track (else bitmap, burned in by the server).</summary>
    public bool? Text { get; init; }
    public bool? Forced { get; init; }
    public bool? External { get; init; }
    /// <summary>Subtitles: "embedded", "sidecar", "opensubtitles" or "upload".</summary>
    public string? Source { get; init; }
    public string? Url { get; init; }

    /// <summary>A text track the server takes out of the video (served in windows until it's all out).</summary>
    [JsonIgnore] public bool Embedded => Text != false && (Source == "embedded" || (Source == null && External != true));
}

/// <summary>GET /v1/items/{id}/playback: how the server serves a title, and its HLS session.</summary>
public sealed record Playback
{
    public required bool Supported { get; init; }
    public IReadOnlyList<string> Reasons { get; init; } = [];
    public string? Master { get; init; }
    public PlaybackVideo? Video { get; init; }
    public IReadOnlyList<PlaybackTrack> Audio { get; init; } = [];
    public IReadOnlyList<PlaybackTrack> Subtitles { get; init; } = [];
    public string? Method { get; init; }
    public string? Variant { get; init; }
    public string? Session { get; init; }
    public bool? Pending { get; init; }
    public string? MediaSourceId { get; init; }
    public double? Duration { get; init; }
    public TrackMemory? Tracks { get; init; }
}

public sealed record SubtitleTiming
{
    public string State { get; init; } = "none";
    public double Offset { get; init; }
    public double Scale { get; init; } = 1.0;
    public int Pieces { get; init; } = 1;
    public double Nudge { get; init; }
    /// <summary>The viewer chose the original timing over the server's match.</summary>
    public bool Original { get; init; }
}

public sealed record SubtitleTrack
{
    public required int Index { get; init; }
    public string Name { get; init; } = "";
    public string? Language { get; init; }
    public bool Text { get; init; } = true;
    public bool? External { get; init; }
    public bool? Forced { get; init; }
    public string? Source { get; init; }
    public string? Url { get; init; }
    public SubtitleTiming? Timing { get; init; }
    [JsonIgnore] public bool Embedded => Text && (Source == "embedded" || (Source == null && External != true));
}

public sealed record SubtitleList { public IReadOnlyList<SubtitleTrack> Subtitles { get; init; } = []; }

public sealed record ContinueAnswer { public Item? Item { get; init; } }

public sealed record SearchResults
{
    public IReadOnlyList<Item> Titles { get; init; } = [];
    public IReadOnlyList<Item> Episodes { get; init; } = [];
}

public sealed record PageLetter { public required string Letter { get; init; } public required int Offset { get; init; } }
public sealed record PageAnswer { public required IReadOnlyList<Item> Items { get; init; } public int? Total { get; init; } public IReadOnlyList<PageLetter>? Index { get; init; } }
public sealed record ItemPage(IReadOnlyList<Item> Items, int Total, IReadOnlyList<PageLetter> Index);

public sealed record SegmentItem { public required string Kind { get; init; } public required double Start { get; init; } public required double End { get; init; } public string? Source { get; init; } }
public sealed record SegmentsAnswer
{
    public IReadOnlyList<SegmentItem> Items { get; init; } = [];
    public bool? Pending { get; init; }
    public bool? Merged { get; init; }
    public bool? CommunityPending { get; init; }
}

public sealed record Thumbnails
{
    public required string State { get; init; }
    public double? Interval { get; init; }
    public int? Width { get; init; }
    public int? Height { get; init; }
    public int? Columns { get; init; }
    public int? Rows { get; init; }
    public int? Count { get; init; }
    public IReadOnlyList<string> Sheets { get; init; } = [];

    public sealed record Tile(int Sheet, int X, int Y);

    [JsonIgnore]
    public bool Ready => State == "ready" && Interval > 0 && Width > 0 && Height > 0 && Columns > 0 && Rows > 0 && Count > 0 && Sheets.Count > 0;

    public Tile? TileAt(double time)
    {
        if (!Ready || double.IsNaN(time)) return null;
        var i = Math.Clamp((int)(time / Interval!.Value), 0, Count!.Value - 1);
        var per = Columns!.Value * Rows!.Value;
        var sheet = i / per;
        if (sheet >= Sheets.Count) return null;
        var k = i % per;
        return new Tile(sheet, (k % Columns.Value) * Width!.Value, (k / Columns.Value) * Height!.Value);
    }
}
