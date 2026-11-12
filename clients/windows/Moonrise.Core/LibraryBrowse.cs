namespace Moonrise.Core;

/// <summary>The Library's category pages (Mac: LibraryCategory), in the sidebar under Movies and Shows.</summary>
public enum LibraryCategory { Genres, Collections, Studios, Decades }

public static class LibraryCategories
{
    public static string Title(this LibraryCategory c, bool shows) => c switch
    {
        LibraryCategory.Genres => "Genres",
        LibraryCategory.Collections => "Collections",
        LibraryCategory.Studios => shows ? "Networks" : "Studios",
        _ => "Decades",
    };

    public static string Singular(this LibraryCategory c, bool shows) => c switch
    {
        LibraryCategory.Genres => "genre",
        LibraryCategory.Collections => "collection",
        LibraryCategory.Studios => shows ? "network" : "studio",
        _ => "decade",
    };

    /// <summary>The sidebar's categories (Mac: PadLibraryScreen): Collections only for movies.</summary>
    public static IReadOnlyList<LibraryCategory> For(bool shows) =>
        Enum.GetValues<LibraryCategory>().Where(c => c != LibraryCategory.Collections || !shows).ToList();

    /// <summary>"1 genre", "12 networks": the count beside a category page's title.</summary>
    public static string CountLine(this LibraryCategory c, bool shows, int n) =>
        n == 1 ? $"1 {c.Singular(shows)}" : $"{n} {c.Title(shows).ToLowerInvariant()}";

    /// <summary>"1 movie", "3 shows": a tile's count.</summary>
    public static string Titles(bool shows, int n) => shows ? (n == 1 ? "1 show" : $"{n} shows") : (n == 1 ? "1 movie" : $"{n} movies");

    /// <summary>A long list (more than this): only the entries with more than one title, until a search.</summary>
    public const int LongList = 24;

    /// <summary>
    /// A category page's tiles (Mac: LibraryCategoryView.entries), each with the filter its page
    /// opens with, locked: genres by count, studios (a long list without a search: those with more
    /// than one title), decades newest first sorted by year, collections sorted by year.
    /// </summary>
    public static IReadOnlyList<CategoryEntry> Entries(LibraryCategory c, LibraryFacets f, string query = "")
    {
        var q = query.Trim();
        var title = new LibraryFilter();
        IEnumerable<CategoryEntry> list = c switch
        {
            LibraryCategory.Genres => f.Genres.Select(g => new CategoryEntry(g.Name, g.Name, g.Count, g.Backdrop, null,
                title with { Genres = new HashSet<string> { g.Name } })),
            LibraryCategory.Studios => (q.Length == 0 ? (f.Studios.Count > LongList ? f.Studios.Where(s => s.Count > 1) : f.Studios)
                    : f.Studios.Where(s => s.Name.Contains(q, StringComparison.CurrentCultureIgnoreCase)))
                .Select(s => new CategoryEntry(s.Name, s.Name, s.Count, s.Backdrop, null, title with { Studio = s.Name })),
            LibraryCategory.Decades => f.Decades.Select(d => new CategoryEntry(d.Label, d.Label, d.Count, d.Backdrop, null,
                title with { Decade = d.Decade, Sort = LibrarySort.Year, Descending = false })),
            _ => (f.Collections.Count > LongList ? f.Collections.Where(x => x.Count > 1) : f.Collections)
                .Select(x => new CategoryEntry(x.Id.ToString(), x.Name, x.Count, x.Backdrop, x.Poster,
                    title with { Collection = new CollectionRef { Id = x.Id, Name = x.Name }, Sort = LibrarySort.Year, Descending = false })),
        };
        return list.Select(e => e with { Filter = e.Filter.LockingCategory }).ToList();
    }

    /// <summary>Studios left out of the page with no search (each has one title).</summary>
    public static int HiddenStudios(LibraryFacets f, int shown) => Math.Max(0, f.Studios.Count - shown);

    /// <summary>A collection's name on its poster tile, without " Collection".</summary>
    public static string PosterName(string name) => name.Replace(" Collection", "");
}

/// <summary>One tile of a category page, and the filter of the page it opens.</summary>
public sealed record CategoryEntry(string Id, string Title, int Count, string? Art, string? Poster, LibraryFilter Filter);

/// <summary>The A–Z strip down the grid's edge (Mac: AlphabetIndex), with a title sort.</summary>
public static class LetterIndex
{
    /// <summary>Every letter the strip shows, the ones without titles dimmed.</summary>
    public static readonly IReadOnlyList<string> All = ["#", .. Enumerable.Range('A', 26).Select(c => ((char)c).ToString())];

    /// <summary>Shown with a title sort, more than one letter, and more than 24 titles.</summary>
    public static bool Shows(LibrarySort sort, int letters, int total) => sort == LibrarySort.Title && letters > 1 && total > 24;

    /// <summary>The nearest letter with titles at or after the one touched, else before (null: none).</summary>
    public static string? Nearest(IReadOnlyList<PageLetter> index, string touched)
    {
        var have = index.Select(l => l.Letter).ToHashSet();
        var i = Math.Max(0, All.ToList().IndexOf(touched));
        return All.Skip(i).FirstOrDefault(have.Contains) ?? All.Take(i + 1).LastOrDefault(have.Contains);
    }

    /// <summary>Where a letter's titles start in the whole list.</summary>
    public static int? Offset(IReadOnlyList<PageLetter> index, string letter) => index.FirstOrDefault(l => l.Letter == letter)?.Offset;

    /// <summary>The letter of the strip at <paramref name="y"/> down a strip <paramref name="height"/> tall.</summary>
    public static string At(double y, double height)
    {
        var i = (int)Math.Floor(y / Math.Max(1, height) * All.Count);
        return All[Math.Clamp(i, 0, All.Count - 1)];
    }
}

/// <summary>
/// The Library grid's visible part, for a grid that makes only the cards near the screen (a
/// library of many thousands): every cell the same size, rows from the top.
/// </summary>
public readonly record struct GridWindow(int Columns, double CellHeight, int Total)
{
    /// <summary>Rows made above and below the screen.</summary>
    public const int SpareRows = 3;

    public int Rows => Columns <= 0 ? 0 : (Total + Columns - 1) / Columns;
    public double Height => Rows * CellHeight;
    public double RowTop(int index) => Columns <= 0 ? 0 : index / Columns * CellHeight;

    /// <summary>The indices to make for a viewport at <paramref name="top"/>, <paramref name="height"/> tall (empty: none).</summary>
    public (int First, int Last) Range(double top, double height)
    {
        if (Total <= 0 || Columns <= 0 || CellHeight <= 0) return (0, -1);
        var firstRow = Math.Max(0, (int)Math.Floor(top / CellHeight) - SpareRows);
        var lastRow = Math.Min(Rows - 1, (int)Math.Floor((top + Math.Max(0, height)) / CellHeight) + SpareRows);
        if (lastRow < firstRow) return (0, -1);
        return (firstRow * Columns, Math.Min(Total - 1, (lastRow + 1) * Columns - 1));
    }
}

/// <summary>
/// The Library's titles as pages of the whole list, fetched as the grid reaches them (not all of
/// the pages before: a letter near the end of a large library loads only its own).
/// </summary>
public sealed class SparsePages(int pageSize)
{
    private readonly Dictionary<int, IReadOnlyList<Item>> _pages = [];
    private readonly HashSet<int> _loading = [];

    public int PageSize { get; } = pageSize;
    public int Generation { get; private set; }
    /// <summary>Null until the first page answers.</summary>
    public int? Total { get; private set; }
    public IReadOnlyList<PageLetter> Index { get; private set; } = [];

    public void Reset()
    {
        Generation++;
        _pages.Clear();
        _loading.Clear();
        Total = null;
        Index = [];
    }

    public static int PageOf(int index, int pageSize) => index / pageSize;

    /// <summary>The pages a range needs that are neither here nor on the way, marked as on the way.</summary>
    public IReadOnlyList<int> Begin(int first, int last)
    {
        if (last < first) return [];
        var want = new List<int>();
        for (var p = PageOf(first, PageSize); p <= PageOf(last, PageSize); p++)
            if (!_pages.ContainsKey(p) && _loading.Add(p)) want.Add(p);
        return want;
    }

    /// <summary>A page's answer; false when the list was reset since (it's for nothing).</summary>
    public bool Accept(int generation, int page, ItemPage answer)
    {
        if (generation != Generation) return false;
        _loading.Remove(page);
        _pages[page] = answer.Items;
        Total = answer.Total;
        if (answer.Index.Count > 0) Index = answer.Index;
        return true;
    }

    /// <summary>A page failed: asked for again the next time it's needed.</summary>
    public void Failed(int generation, int page)
    {
        if (generation == Generation) _loading.Remove(page);
    }

    public Item? At(int index)
    {
        if (!_pages.TryGetValue(PageOf(index, PageSize), out var page)) return null;
        var i = index % PageSize;
        return i < page.Count ? page[i] : null;
    }

    public IEnumerable<Item> Loaded => _pages.Values.SelectMany(p => p);

    /// <summary>A title with newer watch state, where it is in the pages here.</summary>
    public void Replace(Item item)
    {
        foreach (var (k, page) in _pages.ToList())
        {
            var i = page.ToList().FindIndex(x => x.Id == item.Id);
            if (i < 0) continue;
            var copy = page.ToList();
            copy[i] = item;
            _pages[k] = copy;
        }
    }
}
