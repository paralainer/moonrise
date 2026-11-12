namespace Moonrise.Core;

public enum LibraryView { Movies, Shows }

public enum LibrarySort { Title, Added, Year, Rating, Played, Runtime }

public enum WatchedFilter { All, Unwatched, InProgress, Watched }

public static class LibraryText
{
    public static string Type(this LibraryView v) => v == LibraryView.Movies ? "movie" : "series";
    public static string Title(this LibraryView v) => v == LibraryView.Movies ? "Movies" : "Shows";

    public static string Query(this LibrarySort s) => s.ToString().ToLowerInvariant();
    public static string Label(this LibrarySort s) => s switch
    {
        LibrarySort.Title => "Title",
        LibrarySort.Added => "Date Added",
        LibrarySort.Year => "Year",
        LibrarySort.Rating => "Rating",
        LibrarySort.Played => "Last Played",
        _ => "Runtime",
    };
    public static bool Descending(this LibrarySort s) => s != LibrarySort.Title;

    public static (string Up, string Down) Directions(this LibrarySort s) => s switch
    {
        LibrarySort.Title => ("A to Z", "Z to A"),
        LibrarySort.Added or LibrarySort.Year => ("Oldest First", "Newest First"),
        LibrarySort.Rating => ("Lowest First", "Highest First"),
        LibrarySort.Played => ("Least Recent First", "Most Recent First"),
        _ => ("Shortest First", "Longest First"),
    };

    public static string Label(this WatchedFilter w) => w switch
    {
        WatchedFilter.All => "All",
        WatchedFilter.Unwatched => "Unwatched",
        WatchedFilter.InProgress => "In Progress",
        _ => "Watched",
    };
}

public enum LockedDimension { Genre, Studio, Decade, Collection }

public sealed record LibraryFilter
{
    public const int RecentDays = 30;

    public LibrarySort Sort { get; init; } = LibrarySort.Title;
    public bool Descending { get; init; }
    public IReadOnlySet<string> Genres { get; init; } = new HashSet<string>();
    public int? Decade { get; init; }
    public IReadOnlySet<string> Ratings { get; init; } = new HashSet<string>();
    public WatchedFilter Watched { get; init; } = WatchedFilter.All;
    public bool Hdr { get; init; }
    public bool RecentlyAdded { get; init; }
    public string? Library { get; init; }
    /// <summary>Categories (set by a category tile's page).</summary>
    public string? Studio { get; init; }
    public CollectionRef? Collection { get; init; }
    /// <summary>
    /// A category tile's page ("Action", "HBO", "1990s"): the dimensions that make the page, which
    /// stay put (not counted, not in the filter menu, kept by Clear All). Mac: LibraryFilter.locked.
    /// </summary>
    public IReadOnlySet<LockedDimension> Locked { get; init; } = new HashSet<LockedDimension>();

    public bool IsLocked(LockedDimension d) => Locked.Contains(d);

    /// <summary>The same sort, no filters but the page's own.</summary>
    public LibraryFilter Cleared => new()
    {
        Sort = Sort, Descending = Descending, Locked = Locked,
        Genres = IsLocked(LockedDimension.Genre) ? Genres : new HashSet<string>(),
        Studio = IsLocked(LockedDimension.Studio) ? Studio : null,
        Decade = IsLocked(LockedDimension.Decade) ? Decade : null,
        Collection = IsLocked(LockedDimension.Collection) ? Collection : null,
    };

    /// <summary>The page's own dimensions locked: whichever category fields are set.</summary>
    public LibraryFilter LockingCategory
    {
        get
        {
            var l = new HashSet<LockedDimension>();
            if (Genres.Count > 0) l.Add(LockedDimension.Genre);
            if (Studio != null) l.Add(LockedDimension.Studio);
            if (Decade != null) l.Add(LockedDimension.Decade);
            if (Collection != null) l.Add(LockedDimension.Collection);
            return this with { Locked = l };
        }
    }

    public int ActiveCount => (IsLocked(LockedDimension.Genre) ? 0 : Genres.Count) + (Decade == null || IsLocked(LockedDimension.Decade) ? 0 : 1)
        + Ratings.Count + (Watched == WatchedFilter.All ? 0 : 1) + (Hdr ? 1 : 0) + (RecentlyAdded ? 1 : 0) + (Library == null ? 0 : 1)
        + (Studio == null || IsLocked(LockedDimension.Studio) ? 0 : 1) + (Collection == null || IsLocked(LockedDimension.Collection) ? 0 : 1);

    public LibraryFilter SortedBy(LibrarySort s) => s == Sort ? this : this with { Sort = s, Descending = s.Descending() };

    public IReadOnlyDictionary<string, string> ServerQuery
    {
        get
        {
            var q = new Dictionary<string, string>();
            if (Genres.Count > 0) q["genre"] = string.Join(",", Genres.OrderBy(x => x, StringComparer.Ordinal));
            if (Decade is { } d) { q["year_from"] = d.ToString(); q["year_to"] = (d + 9).ToString(); }
            if (Ratings.Count > 0) q["rating"] = string.Join(",", Ratings.OrderBy(x => x, StringComparer.Ordinal));
            switch (Watched)
            {
                case WatchedFilter.Unwatched: q["unplayed"] = "1"; break;
                case WatchedFilter.InProgress: q["in_progress"] = "1"; break;
                case WatchedFilter.Watched: q["played"] = "1"; break;
            }
            if (Hdr) q["hdr"] = "1";
            if (RecentlyAdded) q["added_days"] = RecentDays.ToString();
            if (Studio != null) q["studio"] = Studio;
            if (Collection != null) q["collection"] = Collection.Id.ToString();
            if (Library != null) q["library"] = Library;
            return q;
        }
    }

    /// <summary>"7 movies · Title, A to Z", "1 show · Date Added, newest first".</summary>
    public string CountLine(bool shows, int total)
    {
        var noun = shows ? (total == 1 ? "show" : "shows") : (total == 1 ? "movie" : "movies");
        var (up, down) = Sort.Directions();
        var dir = Descending ? down : up;
        if (Sort != LibrarySort.Title) dir = dir.ToLowerInvariant();
        return $"{total} {noun} · {Sort.Label()}, {dir}";
    }
}

public sealed record FacetNamed { public required string Name { get; init; } public int Count { get; init; } public string? Backdrop { get; init; } }
public sealed record FacetDecade { public int Decade { get; init; } public int Count { get; init; } public string? Backdrop { get; init; } public string Label => $"{Decade}s"; }
public sealed record FacetCollection { public int Id { get; init; } public required string Name { get; init; } public int Count { get; init; } public string? Poster { get; init; } public string? Backdrop { get; init; } }
public sealed record FacetWatched { public int Unplayed { get; init; } public int InProgress { get; init; } public int Played { get; init; } }
public sealed record FacetRecent { public int Count { get; init; } }
public sealed record FacetLibrary { public required string Id { get; init; } public required string Name { get; init; } public int Count { get; init; } }

public sealed record LibraryFacets
{
    public int Total { get; init; }
    public IReadOnlyList<FacetNamed> Genres { get; init; } = [];
    public IReadOnlyList<FacetNamed> Studios { get; init; } = [];
    public IReadOnlyList<FacetDecade> Decades { get; init; } = [];
    public IReadOnlyList<FacetNamed> Ratings { get; init; } = [];
    public IReadOnlyList<FacetCollection> Collections { get; init; } = [];
    public IReadOnlyList<FacetNamed> Resolutions { get; init; } = [];
    public int Hdr { get; init; }
    public FacetWatched? Watched { get; init; }
    public FacetRecent? RecentlyAdded { get; init; }
    public IReadOnlyList<FacetLibrary> Libraries { get; init; } = [];
}
