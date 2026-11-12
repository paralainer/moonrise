namespace Moonrise.Core;

// Skippable segments and when the player offers Next Episode: the same rules as
// clients/apple/Moonrise/Server/SegmentRules.swift and clients/android/core/.../Segments.kt,
// tested against api/segment-rules.json. Servers 0.28+ apply the segment rules themselves
// (segments?merged=1); the client keeps them for older servers.

public enum SegmentKind { Intro, Recap, Credits, Preview, Commercial }
public enum SkipMode { Auto, Button, Off }

public static class SegmentKinds
{
    public static SegmentKind? Of(string? id) => id switch
    {
        "intro" => SegmentKind.Intro, "recap" => SegmentKind.Recap, "credits" => SegmentKind.Credits,
        "preview" => SegmentKind.Preview, "commercial" => SegmentKind.Commercial, _ => null,
    };
    public static string Title(this SegmentKind k) => k.ToString();
    public static string SkipLabel(this SegmentKind k) => $"Skip {k.Title()}";
    public static SkipMode DefaultMode(this SegmentKind k) => k == SegmentKind.Credits ? SkipMode.Button : SkipMode.Auto;
}

/// <summary>A skippable stretch of a title. Source: "detected" when the server found the credits in the file.</summary>
public sealed record Segment(SegmentKind Kind, double Start, double End, string? Source = null);

public sealed record ServerSegments(IReadOnlyList<Segment> Items, bool Pending = false, bool Reached = true, bool Merged = false, bool CommunityPending = false)
{
    public static ServerSegments Unreached { get; } = new([], Reached: false);
}

public static class SegmentRules
{
    public const double CreditsEndBeforeEnd = 5.0;
    public const double UnconfirmedCreditsLead = 150.0;
    public const double DetectedSlack = 10.0;

    public static IReadOnlyList<Segment> Merge(
        IReadOnlyList<Segment> server, IReadOnlyList<IReadOnlyList<Segment>> online, double runtime,
        bool pending = false, bool movie = false, bool communityPending = false)
    {
        var detected = server.FirstOrDefault(s => s.Kind == SegmentKind.Credits && s.Source == "detected");
        var found = server.Where(s => s.Source == null && !(movie && s.Kind == SegmentKind.Credits && !EndsBeforeEnd(s, runtime))).ToList();
        foreach (var more in online)
        {
            var have = found.Select(s => s.Kind).ToHashSet();
            found.AddRange(more.Where(s => s.Kind != SegmentKind.Credits && !have.Contains(s.Kind)));
        }
        var community = online.SelectMany(l => l.Where(s => s.Kind == SegmentKind.Credits)).Where(s => PlausibleCredits(s, runtime)).ToList();
        var latest = community.Count == 0 ? null : community.MaxBy(s => s.Start);
        if (!pending && latest != null)
        {
            var kept = new List<Segment>();
            foreach (var s in found)
            {
                if (s.Kind != SegmentKind.Credits) { kept.Add(s); continue; }
                var c = NotBefore(s, latest);
                if (!movie || EndsBeforeEnd(c, runtime)) kept.Add(c);
            }
            found = kept;
        }
        if (!pending && found.All(s => s.Kind != SegmentKind.Credits))
        {
            var c = Credits(community, detected, runtime);
            if (c != null && (!movie || EndsBeforeEnd(c, runtime))) found.Add(c);
        }
        return found
            .Select(s => new Segment(s.Kind, Math.Max(0, s.Start), runtime > 0 ? Math.Min(s.End, runtime) : s.End, s.Source))
            .Where(s => s.End - s.Start >= 3 && !(communityPending && s.Kind == SegmentKind.Credits))
            .OrderBy(s => s.Start)
            .ToList();
    }

    public static Segment NotBefore(Segment chapter, Segment database)
    {
        if (database.Start <= chapter.Start) return chapter;
        if (database.Start >= chapter.End - 3) return database;
        return new Segment(SegmentKind.Credits, database.Start, chapter.End, chapter.Source);
    }

    public static double? CreditsStart(IEnumerable<Segment> segments)
    {
        var c = segments.Where(s => s.Kind == SegmentKind.Credits).ToList();
        return c.Count == 0 ? null : c.Max(s => s.Start);
    }

    public static bool EndsBeforeEnd(Segment s, double runtime) => runtime > 0 && s.End < runtime - CreditsEndBeforeEnd;

    public static bool PlausibleCredits(Segment s, double runtime)
    {
        if (runtime <= 0) return true;
        return s.Start >= runtime / 2 && s.Start <= runtime - 3 && s.End <= runtime + 5;
    }

    public static Segment? Credits(IReadOnlyList<Segment> community, Segment? detected, double runtime = 0)
    {
        var latest = community.Count == 0 ? null : community.MaxBy(s => s.Start);
        if (detected == null)
        {
            if (latest == null) return null;
            if (community.Count >= 2 || runtime <= 0 || latest.End < runtime - 5 || runtime - latest.Start <= UnconfirmedCreditsLead) return latest;
            return null;
        }
        if (latest == null) return detected;
        return latest.Start >= detected.Start - DetectedSlack ? latest : detected;
    }
}

public static class NextEpisodeRules
{
    /// <summary>How long before the end Next Episode shows when the credits aren't known.</summary>
    public const double Lead = 30.0;

    /// <summary>The episode after <paramref name="current"/> in airing order, crossing seasons; specials only follow a special.</summary>
    public static Item? Next(IReadOnlyList<Item> episodes, Item? current)
    {
        if (current == null) return null;
        var i = episodes.ToList().FindIndex(e => e.Id == current.Id);
        if (i < 0 || i + 1 >= episodes.Count) return null;
        var next = episodes[i + 1];
        if (next.SeasonNo == 0 && (episodes[i].SeasonNo ?? 0) != 0) return null;
        return next;
    }

    public static bool Offered(double duration, double currentTime, double shown, Segment? active, IReadOnlyList<Segment> segments, bool hasNext)
    {
        if (duration <= 0 || currentTime <= 0 || !hasNext) return false;
        if (active?.Kind == SegmentKind.Credits) return true;
        if (segments.Any(s => s.Kind == SegmentKind.Credits && shown >= s.Start && s.End >= duration - 5)) return true;
        return duration - shown <= Lead;
    }

    public sealed record CornerState(bool Skip, bool Next) { public bool Empty => !Skip && !Next; }

    public static CornerState Corner(Segment? active, bool controlsVisible, bool promptVisible, bool counting, bool nextOffered, bool creditsSkipPlaysNext)
    {
        var skip = active != null && (controlsVisible || promptVisible || counting);
        var next = controlsVisible && nextOffered && !(skip && creditsSkipPlaysNext);
        return new CornerState(skip, next);
    }

    public static bool CreditsSkipPlaysNext(Segment? active, double duration, bool hasNext) =>
        active != null && active.Kind == SegmentKind.Credits && hasNext && active.End >= duration - 5;

    public static bool RefetchDue(bool pending, double duration, double t, double askedAt) =>
        pending && duration > 0 && duration - t < 300 && duration - t > 60 && Math.Abs(t - askedAt) >= 60;

    /// <summary>The segment playback is inside at t that can be skipped (its kind isn't Off).</summary>
    public static Segment? Active(IReadOnlyList<Segment> segments, double t, Func<SegmentKind, SkipMode> mode) =>
        segments.FirstOrDefault(s => t >= s.Start && t < s.End - 1 && mode(s.Kind) != SkipMode.Off);
}

public static class SegmentOffer
{
    /// <summary>Where the near-end re-asks start (seconds before the end).</summary>
    public static readonly double[] NearEndAsks = [600, 120];

    public static int NearEndAsksDue(double duration, double t) =>
        duration <= 0 || t <= 0 || t >= duration ? 0 : NearEndAsks.Count(a => duration - t <= a);

    /// <summary>The segments to offer: as they are when the server merged them, else the file's own by the same rules.</summary>
    public static IReadOnlyList<Segment> Offered(ServerSegments server, double runtime, bool provisional, bool movie) =>
        server.Merged ? server.Items
        : SegmentRules.Merge(server.Items, [], runtime, pending: server.Pending && !provisional, movie: movie);
}
