namespace Moonrise.Core;

/// <summary>A rectangle of the window, in raw pixels.</summary>
public readonly record struct PixelRect(int X, int Y, int Width, int Height);

// Small timing rules the screens and player follow, testable without a UI.
public static class ClientRules
{
    public const long FirstRetryMs = 5_000;
    public const long MaxRetryMs = 30_000;
    public const long HomeRefreshMs = 30_000;

    public static long NextRetryMs(long wait) => Math.Min(wait * 2, MaxRetryMs);

    /// <summary>Where a stream (re)starts in seconds; null: from the start. A restart begins at the position, never at 0 first.</summary>
    public static double? StartPosition(double? start) => start > 1 ? start : null;

    public static bool HomeIsStale(bool loading, bool failed, long lastLoadMs, long nowMs)
    {
        if (loading) return false;
        return failed || (lastLoadMs != 0 && nowMs - lastLoadMs >= HomeRefreshMs);
    }

    /// <summary>A click on the tab already selected goes back to its root; on another tab, to where it was left.</summary>
    public static bool TabClickReturnsToRoot(bool alreadySelected, int stackDepth) => alreadySelected && stackDepth > 1;

    /// <summary>
    /// Which track buttons the player shows, as the Mac app: Audio only with a choice of two or more,
    /// Subtitles with at least one subtitle. A title with neither (a silent one) shows neither.
    /// </summary>
    public static (bool Audio, bool Subtitles) TrackButtons(int audioOptions, int subtitleOptions) =>
        (audioOptions > 1, subtitleOptions > 0);

    /// <summary>
    /// How long after the player opens a click on it means nothing: the second click of a
    /// double-click on Play lands on the player, and would pause the title it just started.
    /// </summary>
    public const int PlayerOpenInputGuardMs = 500;

    /// <summary>
    /// What of the title bar's row is the player's while it shows: its own controls, and the rest
    /// still moves the window. Full screen has no window to move, so all of the row is the player's.
    /// </summary>
    public static IReadOnlyList<PixelRect> PlayerTitleRowPassthrough(bool fullScreen, int rowWidth, int rowHeight, IEnumerable<PixelRect> controls) =>
        fullScreen ? [new PixelRect(0, 0, rowWidth, rowHeight)] : controls.ToList();

    /// <summary>A code typed any way ("abcd1234"): the server's form, "ABCD-1234".</summary>
    public static string FormatDeviceCode(string s)
    {
        var c = new string(s.ToUpperInvariant().Where(char.IsLetterOrDigit).Take(8).ToArray());
        return c.Length > 4 ? c[..4] + "-" + c[4..] : c;
    }

    /// <summary>
    /// The window's poster width: as the Mac app, which lays out from the window's live size with
    /// Metrics.pad (Theme.swift, margin 20), floor((W-20-14*n)/n), with n by width and at least 150.
    /// </summary>
    public static int PosterWidth(double windowWidth)
    {
        var n = windowWidth < 900 ? 5.3 : windowWidth < 1250 ? 6.3 : 7.3;
        return (int)Math.Max(150, Math.Floor((windowWidth - 20 - 14 * Math.Floor(n)) / n));
    }

    public static int CardWidth(double windowWidth)
    {
        var n = windowWidth < 900 ? 2.6 : windowWidth < 1250 ? 3.3 : 3.5;
        return (int)Math.Max(280, Math.Floor((windowWidth - 20 - 14 * Math.Floor(n)) / n));
    }

    /// <summary>
    /// The Mac app's poster grid (Library, and Search's groups): as many columns of at least 150 as fit
    /// with 12 between them, the posters filling the row evenly (LibraryViews.swift, layout).
    /// </summary>
    public static PosterGrid LibraryGrid(double available)
    {
        var avail = Math.Max(200, available);
        var n = Math.Max(2, (int)Math.Floor((avail + PosterGrid.Gap) / (PosterGrid.MinCard + PosterGrid.Gap)));
        return new PosterGrid(n, (int)Math.Floor((avail - PosterGrid.Gap * (n - 1)) / n));
    }

    /// <summary>
    /// mpv also says "end of file" when a stream it can't read any more stops (the server went away):
    /// that isn't the title finishing, unless playback really was near the end.
    /// </summary>
    public static bool StreamEndIsTheEnd(double position, double duration) => duration <= 0 || position >= duration - Math.Max(30, duration * 0.01);

    /// <summary>What a page lays out from the window's size: when none of it changes, a resize leaves the page alone.</summary>
    public static PageLayout PageLayoutFor(double windowWidth, double windowHeight) => new(
        PosterWidth(windowWidth), CardWidth(windowWidth),
        Math.Round(Math.Min(windowWidth * 9 / 16, windowHeight * 0.62)),
        LibraryGrid(windowWidth - 2 * PageMargin));

    public const double PageMargin = 20;

    /// <summary>
    /// The player on a title page (Mac: ItemDetailView): 16:9 across the page, but no more than 60 %
    /// of the page's height (the area under the top bar), so the page shows below it.
    /// </summary>
    public static double InlinePlayerHeight(double pageWidth, double pageHeight) =>
        Math.Round(Math.Max(0, Math.Min(pageWidth * 9 / 16, pageHeight * 0.6)));
}

public readonly record struct PageLayout(int Poster, int Card, double HeroHeight, PosterGrid Grid);

/// <summary>
/// A wrap grid of posters. A cell holds a poster and the gap after it, so n cells are one gap wider
/// than the n posters and their n-1 gaps: the panel gets that width, or the last column doesn't fit.
/// </summary>
public readonly record struct PosterGrid(int Columns, int CardWidth)
{
    public const int Gap = 12;
    public const int MinCard = 150;

    public double CellWidth => CardWidth + Gap;
    public double CellHeight => Math.Round(CardWidth * 1.5) + 66;
    public double PanelWidth => Columns * CellWidth;
}

/// <summary>
/// Paging of a list that can be reloaded at any moment: an answer to an old request is ignored, and
/// an old request still running doesn't keep the new list from starting.
/// </summary>
public sealed class PagedList
{
    private int _running = -1;

    public int Generation { get; private set; }
    public int Loaded { get; private set; }
    public int Total { get; private set; }
    public bool Finished { get; private set; }
    public bool Busy => _running == Generation;

    public void Reset()
    {
        Generation++;
        Loaded = 0;
        Total = 0;
        Finished = false;
    }

    /// <summary>The generation to load for, or null when a load is running or there's nothing more.</summary>
    public int? Begin()
    {
        if (Busy || Finished) return null;
        _running = Generation;
        return Generation;
    }

    /// <summary>False: the list was reloaded since, so this answer is for nothing.</summary>
    public bool Accept(int generation, int count, int total)
    {
        if (generation != Generation) return false;
        Loaded += count;
        Total = total;
        Finished = count == 0 || Loaded >= total;
        return true;
    }

    /// <summary>The request is over, answered or failed.</summary>
    public void End(int generation)
    {
        if (_running == generation) _running = -1;
    }
}

/// <summary>Which titles' watch state has changed, so a page shown again can bring only those up to date.</summary>
public sealed class WatchChanges
{
    private readonly List<string> _changed = [];

    public int Version => _changed.Count;

    public void Changed(string itemId) => _changed.Add(itemId);

    /// <summary>What changed since this page last asked; each page asks with its own <paramref name="seen"/>.</summary>
    public IReadOnlyList<string> Since(ref int seen)
    {
        var news = _changed.Skip(seen).ToList();
        seen = _changed.Count;
        return news;
    }
}

/// <summary>
/// Whether an unexpected error is worth putting on screen. Every one is logged; when they come in a
/// burst (a handler failing on each frame of a resize) the message already showing stays, and the
/// page isn't torn down under it again.
/// </summary>
public sealed class FaultGate(int burst = 3, long windowMs = 5_000)
{
    private readonly Queue<long> _recent = new();

    public bool ShouldShow(long nowMs)
    {
        while (_recent.Count > 0 && nowMs - _recent.Peek() > windowMs) _recent.Dequeue();
        if (_recent.Count >= burst) return false;
        _recent.Enqueue(nowMs);
        return true;
    }
}

/// <summary>
/// The hero's pictures: which one shows. A picture appears only when it's the current one and has
/// arrived, however late or in whatever order the downloads finish.
/// </summary>
public sealed class HeroSlides(int count)
{
    private readonly bool[] _opened = new bool[count];

    public int Current { get; private set; }

    public void Show(int index) => Current = index;

    public void Opened(int index) => _opened[index] = true;

    public double Opacity(int index) => index == Current && _opened[index] ? 1 : 0;
}
