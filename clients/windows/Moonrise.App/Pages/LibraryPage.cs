using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Moonrise.App.Player;
using Moonrise.App.Ui;
using Moonrise.Core;
using Windows.UI;

namespace Moonrise.App.Pages;

/// <summary>
/// Library (Mac: PadLibraryScreen): a sidebar (Movies, Shows, then the categories of the one open)
/// beside its page: the grid, a category's tiles, or a tile's own grid.
/// </summary>
public sealed class LibraryPage : Grid, IRefreshable
{
    private const double SidebarWidth = 280;
    private readonly Shell _shell;
    private readonly StackPanel _sidebarRows = new() { Spacing = 2 };
    private readonly StackPanel _categoryRows = new() { Spacing = 2, Margin = new Thickness(0, 22, 0, 0) };
    private readonly Grid _content = new();
    private readonly Dictionary<LibraryView, (Border Border, TextBlock Text)> _rows = new();
    private readonly Dictionary<LibraryView, LibraryFilter> _filters = new() { [LibraryView.Movies] = new(), [LibraryView.Shows] = new() };
    private readonly LibraryGridPage _root;
    private LibraryView _view = LibraryView.Movies;
    private LibraryCategory? _category;
    private CategoryPage? _categoryPage;
    private LibraryGridPage? _tilePage;
    private bool _rootLoaded;

    public LibraryPage(Shell shell)
    {
        _shell = shell;
        Background = Theme.Gradient(0.5, 0, 0.5, 1, (0, Color.FromArgb(255, 0x1A, 0x1C, 0x26)), (1, Theme.BackgroundColor));
        ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(SidebarWidth) });
        ColumnDefinitions.Add(F.Auto);
        ColumnDefinitions.Add(F.Star());

        var side = new StackPanel { Margin = new Thickness(Theme.Margin, Shell.TopBarHeight + 8, Theme.Margin, 0), Spacing = 8 };
        var heading = F.Text("Library", Theme.FontLargeTitle, F.Bold);
        heading.VerticalAlignment = VerticalAlignment.Center;
        side.Children.Add(new Grid { Height = LibraryGridPage.TitleRow, Children = { heading } });
        side.Children.Add(_sidebarRows);
        Row(LibraryView.Movies, "\uE8B2");
        Row(LibraryView.Shows, "\uE7F4");
        side.Children.Add(_categoryRows);
        Children.Add(new ScrollViewer { Content = side, VerticalScrollBarVisibility = ScrollBarVisibility.Hidden }.At(0, 0));
        Children.Add(new Border { Width = 1, Background = new SolidColorBrush(Color.FromArgb(20, 255, 255, 255)), Margin = new Thickness(0, Shell.TopBarHeight, 0, 0) }.At(0, 1));
        Children.Add(_content.At(0, 2));

        _root = new LibraryGridPage(shell, null, f => _filters[_view] = f);
        BuildCategoryRows();
    }

    private void Row(LibraryView view, string glyph)
    {
        var text = F.Text(view.Title(), Theme.FontBody, F.Medium);
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12, VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(10, 0, 10, 0) };
        row.Children.Add(new Border { Width = 26, Child = F.Glyph(glyph, 16) });
        row.Children.Add(text);
        var border = new Border { Height = 44, CornerRadius = new CornerRadius(10), Child = row };
        _rows[view] = (border, text);
        var b = F.Bare(border, () => Select(view));
        b.HorizontalAlignment = HorizontalAlignment.Stretch;
        b.HorizontalContentAlignment = HorizontalAlignment.Stretch;
        _sidebarRows.Children.Add(b);
    }

    /// <summary>The open library's categories under Movies and Shows (Collections only for movies).</summary>
    private void BuildCategoryRows()
    {
        _categoryRows.Children.Clear();
        var shows = _view == LibraryView.Shows;
        foreach (var c in LibraryCategories.For(shows))
        {
            var on = _category == c;
            var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12, VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(10, 0, 10, 0) };
            row.Children.Add(new Border { Width = 26, Child = F.Glyph(CategoryPage.Glyph(c, shows), 16) });
            row.Children.Add(F.Text(c.Title(shows), Theme.FontBody, on ? F.SemiBold : F.Medium));
            var border = new Border { Height = 44, CornerRadius = new CornerRadius(10), Child = row, Background = on ? Theme.SurfaceStrong : Theme.Transparent };
            var b = F.Bare(border, () => OpenCategory(c));
            b.HorizontalAlignment = HorizontalAlignment.Stretch;
            b.HorizontalContentAlignment = HorizontalAlignment.Stretch;
            _categoryRows.Children.Add(b);
        }
        foreach (var (v, (border, text)) in _rows)
        {
            border.Background = v == _view && _category == null ? Theme.SurfaceStrong : Theme.Transparent;
            text.FontWeight = v == _view ? F.SemiBold : F.Medium; // the library a category page belongs to
        }
    }

    public void Refresh()
    {
        if (!_rootLoaded) { Select(_view); return; }
        if (_tilePage != null) _tilePage.Refresh();
        else if (_category == null) _root.Refresh();
    }

    /// <summary>A library's grid, from its sidebar row.</summary>
    private void Select(LibraryView v)
    {
        var changed = v != _view || !_rootLoaded;
        _view = v;
        _category = null;
        _categoryPage = null;
        _tilePage = null;
        BuildCategoryRows();
        F.ShowOnly(_content, _root);
        if (changed) { _rootLoaded = true; _root.Show(v, _filters[v], v.Title()); }
    }

    private void OpenCategory(LibraryCategory c)
    {
        _category = c;
        _tilePage = null;
        BuildCategoryRows();
        if (_categoryPage == null || _categoryPage.Category != c || _categoryPage.View != _view)
            _categoryPage = new CategoryPage(_shell, _view, c, OpenTile);
        F.ShowOnly(_content, _categoryPage);
    }

    /// <summary>A tile's page: the library's titles with its filter, back to its category.</summary>
    private void OpenTile(CategoryEntry e)
    {
        var c = _category;
        _tilePage = new LibraryGridPage(_shell, () => { if (c is { } cat) OpenCategory(cat); }, null);
        F.ShowOnly(_content, _tilePage);
        _tilePage.Show(_view, e.Filter, e.Title);
    }
}

/// <summary>
/// One grid of titles (the library's, or a tile's): its header (back, title, Sort, Filter), the
/// count, the posters and the A–Z strip. Only the posters near the screen are made, and pages of
/// the list are fetched as the grid reaches them, so a library of many thousands scrolls as one of
/// a hundred, and a letter far down loads only its own page.
/// </summary>
public sealed class LibraryGridPage : Grid
{
    public const double TitleRow = 52;
    private const int PageSize = 100;
    private const double StripWidth = 20;
    private readonly Shell _shell;
    private readonly Action<LibraryFilter>? _remember;
    private readonly TextBlock _title = F.Text("", Theme.FontLargeTitle, F.Bold, trim: TextTrimming.CharacterEllipsis);
    private readonly TextBlock _countLine = F.Text("", Theme.FontCaption, fg: Theme.Secondary);
    private readonly Button _sort;
    private readonly TextBlock _sortText = F.Text("", Theme.FontSubheadline, F.SemiBold);
    private readonly Button _filter;
    private readonly Border _filterBadge;
    private readonly TextBlock _filterBadgeText = F.Text("", 10, F.Bold, Theme.Black);
    private readonly ScrollViewer _scroll = new() { HorizontalScrollMode = ScrollMode.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
    private readonly Canvas _canvas = new() { HorizontalAlignment = HorizontalAlignment.Left, VerticalAlignment = VerticalAlignment.Top };
    private readonly Grid _gridArea = new();
    private readonly Grid _empty = new() { Visibility = Visibility.Collapsed };
    private readonly Grid _strip = new() { Width = StripWidth, HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Center, Visibility = Visibility.Collapsed };
    private readonly Grid _stripLetters = new();
    private readonly Border _bubble = new() { Width = 64, Height = 64, CornerRadius = new CornerRadius(16), Background = Theme.Glass, HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Center, Visibility = Visibility.Collapsed, IsHitTestVisible = false };
    private readonly TextBlock _bubbleText = F.Text("", 34, F.Bold);
    private readonly Dictionary<int, PosterCard> _cards = [];
    private readonly SparsePages _pages = new(PageSize);
    private LibraryView _view;
    private LibraryFilter _f = new();
    private LibraryFacets? _facets;
    private PosterGrid _metrics;
    private GridWindow _window;
    private bool _stripDragging;
    private string? _stripCurrent;
    private int _watchSeen;
    private long _lastLoad = long.MinValue / 2;
    private bool _failed;

    public LibraryGridPage(Shell shell, Action? onBack, Action<LibraryFilter>? remember)
    {
        _shell = shell;
        _remember = remember;
        RowDefinitions.Add(F.RowAuto);
        RowDefinitions.Add(F.RowAuto);
        RowDefinitions.Add(F.RowStar);

        // Mac: PadLibraryHeader, (back) title, then Sort and Filter at the end; as tall as the sidebar's "Library".
        var head = new Grid { Margin = new Thickness(Theme.Margin, Shell.TopBarHeight + 8, Theme.Margin, 0), Height = TitleRow, ColumnSpacing = 12 };
        // Back only on a tile's page: the root's title starts at the margin, over the grid.
        head.ColumnDefinitions.Add(onBack != null ? F.Auto : new ColumnDefinition { Width = new GridLength(0) });
        head.ColumnDefinitions.Add(F.Star());
        head.ColumnDefinitions.Add(F.Auto);
        if (onBack != null)
        {
            var back = F.GlassCircle("\uE72B", 40, 14, onBack, "Back");
            back.VerticalAlignment = VerticalAlignment.Center;
            head.Children.Add(back.At(0, 0));
        }
        else head.ColumnSpacing = 0;
        _title.VerticalAlignment = VerticalAlignment.Center;
        head.Children.Add(_title.At(0, 1));
        var tools = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10, VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(onBack != null ? 0 : 12, 0, 0, 0) };
        _sort = F.Bare(F.Capsule(_sortText, Theme.Glass, 40, 16), SortMenu);
        var filterRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, VerticalAlignment = VerticalAlignment.Center };
        filterRow.Children.Add(F.Glyph("\uE71C", 14));
        filterRow.Children.Add(F.Text("Filter", Theme.FontSubheadline, F.SemiBold));
        _filterBadge = new Border { Width = 20, Height = 20, CornerRadius = new CornerRadius(10), Background = Theme.White, Child = _filterBadgeText, Visibility = Visibility.Collapsed };
        _sortText.VerticalAlignment = VerticalAlignment.Center;
        _filterBadgeText.HorizontalAlignment = HorizontalAlignment.Center;
        _filterBadgeText.VerticalAlignment = VerticalAlignment.Center;
        filterRow.Children.Add(_filterBadge);
        _filter = F.Bare(F.Capsule(filterRow, Theme.Glass, 40, 16), FilterMenu);
        tools.Children.Add(_sort);
        tools.Children.Add(_filter);
        head.Children.Add(tools.At(0, 2));
        Children.Add(head.At(0, 0));

        _countLine.Margin = new Thickness(Theme.Margin, 6, Theme.Margin, 4);
        Children.Add(_countLine.At(1, 0));

        _scroll.Content = new Border { Child = _canvas, Padding = new Thickness(Theme.Margin, 12, Theme.Margin, 12) };
        _gridArea.Children.Add(_scroll);
        _empty.Margin = new Thickness(0, 120, 0, 0);
        _gridArea.Children.Add(_empty);
        // Mac: AlphabetIndex, down the grid's edge with a title sort; click or drag to a letter.
        _strip.Children.Add(new Border { CornerRadius = new CornerRadius(StripWidth / 2), Background = Theme.Glass });
        _stripLetters.Margin = new Thickness(0, 6, 0, 6);
        foreach (var _ in LetterIndex.All) _stripLetters.RowDefinitions.Add(F.RowStar); // all 27 fit, whatever the rounding
        _strip.Children.Add(_stripLetters);
        _strip.Margin = new Thickness(0, 0, 8, 0);
        _strip.Background = Theme.Transparent;
        _bubble.Margin = new Thickness(0, 0, StripWidth + 24, 0);
        _bubbleText.HorizontalAlignment = HorizontalAlignment.Center;
        _bubbleText.VerticalAlignment = VerticalAlignment.Center;
        _bubble.Child = _bubbleText;
        _strip.PointerPressed += (_, e) => { _stripDragging = true; _strip.CapturePointer(e.Pointer); StripAt(e); e.Handled = true; };
        _strip.PointerMoved += (_, e) => { if (_stripDragging) { StripAt(e); e.Handled = true; } };
        _strip.PointerReleased += (_, e) => StripUp(e);
        _strip.PointerCanceled += (_, e) => StripUp(e);
        _strip.PointerCaptureLost += (_, _) => { _stripDragging = false; _bubble.Visibility = Visibility.Collapsed; };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(_strip, "Jump to letter");
        _gridArea.Children.Add(_bubble);
        _gridArea.Children.Add(_strip);
        Children.Add(_gridArea.At(2, 0));

        _scroll.ViewChanged += (_, _) => Realize();
        _gridArea.SizeChanged += (_, _) => PlaceStrip();
        _scroll.SizeChanged += (_, _) => Layout();
        _watchSeen = shell.Watch.Version;
    }

    // ---------------------------------------------------------------- loading

    /// <summary>Shows a view with these filters (the root grid on a view's row; a tile's page once).</summary>
    public void Show(LibraryView view, LibraryFilter filter, string title)
    {
        _view = view;
        _f = filter;
        _title.Text = title;
        Reload();
    }

    public void Refresh()
    {
        // Keeps its place while it's fresh; a new title shows after a few minutes away.
        var changed = _shell.Watch.Since(ref _watchSeen);
        if (_failed || Environment.TickCount64 - _lastLoad > 300_000) Reload();
        else if (changed.Count > 0)
        {
            // A title can leave a filtered list; otherwise only its badge changes, in place.
            if (_f.Watched != WatchedFilter.All) Reload();
            else _ = UpdateWatchState(changed);
        }
    }

    private async Task UpdateWatchState(IReadOnlyList<string> ids)
    {
        foreach (var id in ids.Distinct())
        {
            var card = _cards.Values.FirstOrDefault(c => c.Item.Id == id);
            try
            {
                var item = await _shell.Api.Item(id);
                _pages.Replace(item);
                card?.Update(item);
            }
            catch (Exception e) { AppLog.Write("library: watch state not refreshed: " + e.Message); }
        }
    }

    private void SetFilter(LibraryFilter f)
    {
        _f = f;
        _remember?.Invoke(f);
        Reload();
    }

    private void Reload()
    {
        _pages.Reset();
        _failed = false;
        foreach (var c in _cards.Values) _canvas.Children.Remove(c.Button);
        _cards.Clear();
        _canvas.Height = 0;
        _empty.Visibility = Visibility.Collapsed;
        _scroll.ChangeView(null, 0, null, true);
        UpdateChrome();
        _ = LoadFacets();
        Fetch(0, PageSize - 1);
    }

    private async Task LoadFacets()
    {
        var gen = _pages.Generation;
        try { var f = await _shell.Api.Facets(_view, _f); if (gen == _pages.Generation) _facets = f; } catch { }
    }

    /// <summary>The pages of a range of the list that aren't here or on their way.</summary>
    private void Fetch(int first, int last)
    {
        foreach (var page in _pages.Begin(first, last)) _ = LoadPage(page);
    }

    private async Task LoadPage(int page)
    {
        var gen = _pages.Generation;
        try
        {
            var answer = await _shell.Api.Items(_view, _f, page * PageSize, PageSize);
            if (!_pages.Accept(gen, page, answer)) return;
            _lastLoad = Environment.TickCount64;
            if (answer.Total == 0) ShowEmpty();
            UpdateChrome();
            Layout(force: true);
        }
        catch (Exception e)
        {
            _pages.Failed(gen, page);
            if (gen != _pages.Generation) return;
            _failed = true;
            if (_cards.Count == 0) ShowFailed(e.Message);
            else _countLine.Text = "Can't load this library. " + e.Message;
        }
    }

    // ---------------------------------------------------------------- the grid

    private bool StripShows => _pages.Total is { } total && LetterIndex.Shows(_f.Sort, _pages.Index.Count, total);

    /// <summary>Columns and the canvas's size for the width there is, the cards in place (resized where they changed).</summary>
    private void Layout(bool force = false)
    {
        var avail = _scroll.ActualWidth - 2 * Theme.Margin - (StripShows ? StripWidth + 8 : 0);
        if (avail <= 0) return;
        var g = ClientRules.LibraryGrid(avail);
        var w = new GridWindow(g.Columns, g.CellHeight, _pages.Total ?? 0);
        if (!force && g == _metrics && w == _window) return;
        var moved = g != _metrics;
        _metrics = g;
        _window = w;
        _canvas.Width = g.PanelWidth;
        _canvas.Height = w.Height;
        if (moved)
            foreach (var (i, c) in _cards) { c.Resize(g.CardWidth); Place(c, i); }
        Realize();
    }

    private void Place(PosterCard card, int index)
    {
        Canvas.SetLeft(card.Button, index % Math.Max(1, _metrics.Columns) * _metrics.CellWidth);
        Canvas.SetTop(card.Button, _window.RowTop(index));
    }

    /// <summary>The cards for the rows on screen (and a few around them); the rest let go.</summary>
    private void Realize()
    {
        UpdateStripVisibility();
        if (_pages.Total is not { } total || total == 0 || _metrics.Columns == 0) return;
        var top = _scroll.VerticalOffset - 12;
        var (first, last) = _window.Range(top, _scroll.ViewportHeight > 0 ? _scroll.ViewportHeight : ActualHeight);
        foreach (var i in _cards.Keys.Where(i => i < first || i > last).ToList())
        {
            _canvas.Children.Remove(_cards[i].Button);
            _cards.Remove(i);
        }
        var missing = false;
        for (var i = first; i <= last; i++)
        {
            if (_cards.ContainsKey(i)) continue;
            if (_pages.At(i) is not { } item) { missing = true; continue; }
            var card = new PosterCard(_shell.Api, item, _metrics.CardWidth, () => _shell.Open(item));
            _cards[i] = card;
            Place(card, i);
            _canvas.Children.Add(card.Button);
        }
        if (missing) Fetch(first, last);
    }

    private void UpdateChrome()
    {
        _sortText.Text = (_f.Descending ? "↓ " : "↑ ") + _f.Sort.Label();
        _filterBadge.Visibility = _f.ActiveCount > 0 ? Visibility.Visible : Visibility.Collapsed;
        _filterBadgeText.Text = _f.ActiveCount.ToString();
        _countLine.Text = _pages.Total is > 0 ? _f.CountLine(_view == LibraryView.Shows, _pages.Total.Value) : " ";
        UpdateStripVisibility();
    }

    private void ShowFailed(string reason)
    {
        _empty.Children.Clear();
        var stack = new StackPanel { Spacing = 10, HorizontalAlignment = HorizontalAlignment.Center };
        var t = F.Text("Can't load this library.", Theme.FontTitle3, F.SemiBold); t.HorizontalAlignment = HorizontalAlignment.Center;
        var s = F.Text(reason, Theme.FontSubheadline, fg: Theme.Secondary, maxLines: 3); s.HorizontalAlignment = HorizontalAlignment.Center; s.MaxWidth = 460;
        var again = F.Bare(F.Text("Try Again", Theme.FontBody, F.SemiBold), Reload); again.HorizontalAlignment = HorizontalAlignment.Center;
        stack.Children.Add(t); stack.Children.Add(s); stack.Children.Add(again);
        _empty.Children.Add(stack);
        _empty.Visibility = Visibility.Visible;
    }

    private void ShowEmpty()
    {
        _empty.Children.Clear();
        var stack = new StackPanel { Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center };
        stack.Children.Add(F.Glyph("\uE8B2", 48, Theme.Secondary));
        var filtered = _f.ActiveCount > 0;
        var t = F.Text(filtered ? "No Matches" : "Nothing Here", Theme.FontTitle3, F.SemiBold); t.HorizontalAlignment = HorizontalAlignment.Center;
        var s = F.Text(filtered ? "No titles match these filters." : "This library is empty.", Theme.FontSubheadline, fg: Theme.Secondary); s.HorizontalAlignment = HorizontalAlignment.Center;
        stack.Children.Add(t); stack.Children.Add(s);
        if (filtered)
        {
            var clear = F.Bare(F.Capsule(F.Text("Clear Filters", Theme.FontSubheadline, F.SemiBold), Theme.Glass, 36, 16), () => SetFilter(_f.Cleared));
            clear.HorizontalAlignment = HorizontalAlignment.Center;
            clear.Margin = new Thickness(0, 6, 0, 0);
            stack.Children.Add(clear);
        }
        _empty.Children.Add(stack);
        _empty.Visibility = Visibility.Visible;
    }

    // ---------------------------------------------------------------- A–Z

    private void StripUp(PointerRoutedEventArgs e)
    {
        if (!_stripDragging) return;
        _stripDragging = false;
        _strip.ReleasePointerCapture(e.Pointer);
        _bubble.Visibility = Visibility.Collapsed;
        _stripCurrent = null;
        DrawLetters();
    }

    private void StripAt(PointerRoutedEventArgs e)
    {
        var y = e.GetCurrentPoint(_stripLetters).Position.Y;
        if (LetterIndex.Nearest(_pages.Index, LetterIndex.At(y, _stripLetters.ActualHeight)) is not { } letter || letter == _stripCurrent) return;
        _stripCurrent = letter;
        _bubbleText.Text = letter;
        _bubble.Visibility = Visibility.Visible;
        DrawLetters();
        Jump(letter);
    }

    /// <summary>Scrolls to where a letter's titles start; the grid fetches that page as it gets there.</summary>
    private void Jump(string letter)
    {
        if (LetterIndex.Offset(_pages.Index, letter) is not { } at) return;
        _scroll.ChangeView(null, Math.Min(_scroll.ScrollableHeight, _window.RowTop(at)), null, true);
    }

    private void DrawLetters()
    {
        _stripLetters.Children.Clear();
        var have = _pages.Index.Select(l => l.Letter).ToHashSet();
        var size = Math.Min(11, _strip.Height / LetterIndex.All.Count * 0.75);
        for (var row = 0; row < LetterIndex.All.Count; row++)
        {
            var l = LetterIndex.All[row];
            var current = l == _stripCurrent;
            var t = F.Text(l, size, F.Bold, current ? Theme.Black : have.Contains(l) ? Theme.White : new SolidColorBrush(Color.FromArgb(64, 255, 255, 255)));
            t.HorizontalAlignment = HorizontalAlignment.Center;
            t.VerticalAlignment = VerticalAlignment.Center;
            var cell = new Grid();
            Grid.SetRow(cell, row);
            if (current) cell.Children.Add(new Border { Width = 15, Height = 15, CornerRadius = new CornerRadius(7.5), Background = Theme.White, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center });
            cell.Children.Add(t);
            _stripLetters.Children.Add(cell);
        }
    }

    /// <summary>Mac: at most 432 tall, centred beside the grid; hidden when there's no room for 180.</summary>
    private void PlaceStrip()
    {
        var h = Math.Min(432, _gridArea.ActualHeight - 20);
        _strip.Height = Math.Max(0, h);
        if (StripShows && h >= 180) DrawLetters();
        UpdateStripVisibility();
    }

    private void UpdateStripVisibility()
    {
        var on = StripShows && _strip.Height >= 180;
        var v = on ? Visibility.Visible : Visibility.Collapsed;
        if (_strip.Visibility == v) return;
        _strip.Visibility = v;
        if (on) DrawLetters();
        Layout(force: true); // the grid gives the strip its width
    }

    // ---------------------------------------------------------------- Sort and Filter

    private void SortMenu()
    {
        var menu = new MenuFlyout();
        foreach (var s in Enum.GetValues<LibrarySort>())
        {
            var item = new ToggleMenuFlyoutItem { Text = s.Label(), IsChecked = _f.Sort == s };
            item.Click += (_, _) => SetFilter(_f.SortedBy(s));
            menu.Items.Add(item);
        }
        menu.Items.Add(new MenuFlyoutSeparator());
        var (up, down) = _f.Sort.Directions();
        foreach (var (label, desc) in new[] { (up, false), (down, true) })
        {
            var item = new ToggleMenuFlyoutItem { Text = label, IsChecked = _f.Descending == desc };
            item.Click += (_, _) => SetFilter(_f with { Descending = desc });
            menu.Items.Add(item);
        }
        menu.ShowAt(_sort);
    }

    private void FilterMenu()
    {
        var menu = new MenuFlyout();
        if (_f.ActiveCount > 0)
        {
            var c = new MenuFlyoutItem { Text = "Clear All" };
            c.Click += (_, _) => SetFilter(_f.Cleared);
            menu.Items.Add(c);
            menu.Items.Add(new MenuFlyoutSeparator());
        }

        var watched = new MenuFlyoutSubItem { Text = "Watched" };
        foreach (var w in Enum.GetValues<WatchedFilter>())
        {
            var it = new ToggleMenuFlyoutItem { Text = w.Label(), IsChecked = _f.Watched == w };
            it.Click += (_, _) => SetFilter(_f with { Watched = w });
            watched.Items.Add(it);
        }
        menu.Items.Add(watched);

        var f = _facets;
        if (f != null)
        {
            // A tile's page keeps what makes it out of the menu (Mac: locked dimensions).
            if (f.Genres.Count > 0 && !_f.IsLocked(LockedDimension.Genre))
            {
                var genre = new MenuFlyoutSubItem { Text = "Genre" };
                foreach (var g in f.Genres)
                {
                    var it = new ToggleMenuFlyoutItem { Text = $"{g.Name}  ({g.Count})", IsChecked = _f.Genres.Contains(g.Name) };
                    it.Click += (_, _) => { var set = new HashSet<string>(_f.Genres); if (!set.Remove(g.Name)) set.Add(g.Name); SetFilter(_f with { Genres = set }); };
                    genre.Items.Add(it);
                }
                menu.Items.Add(genre);
            }
            if (f.Decades.Count > 0 && !_f.IsLocked(LockedDimension.Decade))
            {
                var year = new MenuFlyoutSubItem { Text = "Year" };
                foreach (var d in f.Decades)
                {
                    var it = new ToggleMenuFlyoutItem { Text = $"{d.Label}  ({d.Count})", IsChecked = _f.Decade == d.Decade };
                    it.Click += (_, _) => SetFilter(_f with { Decade = _f.Decade == d.Decade ? null : d.Decade });
                    year.Items.Add(it);
                }
                menu.Items.Add(year);
            }
            if (f.Ratings.Count > 0)
            {
                var rating = new MenuFlyoutSubItem { Text = "Rating" };
                foreach (var r in f.Ratings)
                {
                    var it = new ToggleMenuFlyoutItem { Text = $"{(r.Name == "none" ? "Not Rated" : r.Name)}  ({r.Count})", IsChecked = _f.Ratings.Contains(r.Name) };
                    it.Click += (_, _) => { var set = new HashSet<string>(_f.Ratings); if (!set.Remove(r.Name)) set.Add(r.Name); SetFilter(_f with { Ratings = set }); };
                    rating.Items.Add(it);
                }
                menu.Items.Add(rating);
            }
        }
        var hdr = new ToggleMenuFlyoutItem { Text = "HDR", IsChecked = _f.Hdr };
        hdr.Click += (_, _) => SetFilter(_f with { Hdr = !_f.Hdr });
        var recent = new ToggleMenuFlyoutItem { Text = "Recently Added", IsChecked = _f.RecentlyAdded };
        recent.Click += (_, _) => SetFilter(_f with { RecentlyAdded = !_f.RecentlyAdded });
        menu.Items.Add(hdr);
        menu.Items.Add(recent);
        menu.ShowAt(_filter);
    }
}

/// <summary>
/// A category's page (Mac: LibraryCategoryView): its tiles with their artwork and counts
/// (collections as posters), each opening the library's titles with that filter. Studios and
/// networks have a search.
/// </summary>
public sealed class CategoryPage : Grid
{
    private const double TileMin = 165, TileGap = 14, PosterMin = 110, PosterGapH = 14, PosterGapV = 20;
    private readonly Shell _shell;
    private readonly Action<CategoryEntry> _open;
    private readonly TextBlock _count = F.Text("", Theme.FontSubheadline, fg: Theme.Secondary);
    private readonly TextBox _search = new() { Width = 220, Height = 34, VerticalAlignment = VerticalAlignment.Center };
    private readonly ScrollViewer _scroll = new() { HorizontalScrollMode = ScrollMode.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
    private readonly StackPanel _body = new() { Spacing = 14, Padding = new Thickness(Theme.Margin, 12, Theme.Margin, Theme.Margin) };
    private LibraryFacets? _facets;
    private string? _error;
    private double _lastWidth;

    public LibraryView View { get; }
    public LibraryCategory Category { get; }
    private bool Shows => View == LibraryView.Shows;

    public CategoryPage(Shell shell, LibraryView view, LibraryCategory category, Action<CategoryEntry> open)
    {
        _shell = shell;
        _open = open;
        View = view;
        Category = category;
        RowDefinitions.Add(F.RowAuto);
        RowDefinitions.Add(F.RowStar);
        var head = new Grid { Margin = new Thickness(Theme.Margin, Shell.TopBarHeight + 8, Theme.Margin, 0), Height = LibraryGridPage.TitleRow, ColumnSpacing = 12 };
        head.ColumnDefinitions.Add(F.Star());
        head.ColumnDefinitions.Add(F.Auto);
        var title = F.Text(category.Title(Shows), Theme.FontLargeTitle, F.Bold, trim: TextTrimming.CharacterEllipsis);
        title.VerticalAlignment = VerticalAlignment.Center;
        head.Children.Add(title.At(0, 0));
        var tools = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 14, VerticalAlignment = VerticalAlignment.Center };
        if (category == LibraryCategory.Studios)
        {
            _search.PlaceholderText = Shows ? "Networks" : "Studios";
            _search.TextChanged += (_, _) => Draw();
            tools.Children.Add(_search);
        }
        _count.VerticalAlignment = VerticalAlignment.Center;
        tools.Children.Add(_count);
        head.Children.Add(tools.At(0, 1));
        Children.Add(head.At(0, 0));
        _scroll.Content = _body;
        Children.Add(_scroll.At(1, 0));
        SizeChanged += (_, e) => { if (Math.Abs(e.NewSize.Width - _lastWidth) >= 1) { _lastWidth = e.NewSize.Width; Draw(); } };
        _ = Load();
    }

    public static string Glyph(LibraryCategory c, bool shows) => c switch
    {
        LibraryCategory.Genres => "\uE8EC",
        LibraryCategory.Collections => "",
        LibraryCategory.Studios => shows ? "" : "",
        _ => "",
    };

    private async Task Load()
    {
        try { _facets = await _shell.Api.Facets(View, new LibraryFilter()); _error = null; }
        catch (Exception e) { _error = e.Message; }
        Draw();
    }

    private void Draw()
    {
        _body.Children.Clear();
        if (_facets == null)
        {
            FrameworkElement state = _error != null
                ? F.Text("Can't load this page. " + _error, Theme.FontBody, fg: Theme.Secondary, maxLines: 3)
                : new ProgressRing { IsActive = true, Width = 32, Height = 32, HorizontalAlignment = HorizontalAlignment.Left };
            _body.Children.Add(state);
            if (_error != null) _body.Children.Add(F.Bare(F.Text("Try Again", Theme.FontBody, F.SemiBold), () => { _error = null; Draw(); _ = Load(); }));
            return;
        }
        var query = _search.Text;
        var entries = LibraryCategories.Entries(Category, _facets, query);
        _count.Text = Category.CountLine(Shows, entries.Count);
        var avail = Math.Max(200, _scroll.ActualWidth - 2 * Theme.Margin);
        if (entries.Count == 0)
        {
            var empty = new StackPanel { Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 80, 0, 0) };
            empty.Children.Add(F.Glyph(Glyph(Category, Shows), 44, Theme.Secondary));
            var t = F.Text("Nothing Here", Theme.FontTitle3, F.SemiBold); t.HorizontalAlignment = HorizontalAlignment.Center;
            var s = F.Text(query.Trim().Length == 0 ? $"No {Category.Title(Shows).ToLowerInvariant()} in this library." : $"Nothing matched “{query.Trim()}”.", Theme.FontSubheadline, fg: Theme.Secondary);
            s.HorizontalAlignment = HorizontalAlignment.Center;
            empty.Children.Add(t); empty.Children.Add(s);
            _body.Children.Add(empty);
        }
        else if (Category == LibraryCategory.Collections)
        {
            var (n, w) = Columns(avail, PosterMin, PosterGapH);
            var grid = new VariableSizedWrapGrid { Orientation = Orientation.Horizontal, MaximumRowsOrColumns = n, ItemWidth = w + PosterGapH, ItemHeight = Math.Round(w * 1.5) + 44 + PosterGapV };
            grid.Width = n * (w + PosterGapH);
            foreach (var e in entries) grid.Children.Add(PosterTile(e, w));
            _body.Children.Add(grid);
        }
        else
        {
            var (n, w) = Columns(avail, TileMin, TileGap);
            var grid = new VariableSizedWrapGrid { Orientation = Orientation.Horizontal, MaximumRowsOrColumns = n, ItemWidth = w + TileGap, ItemHeight = Math.Round(w * 9 / 16) + TileGap };
            grid.Width = n * (w + TileGap);
            foreach (var e in entries) grid.Children.Add(Tile(e, w));
            _body.Children.Add(grid);
        }
        if (Category == LibraryCategory.Studios && query.Trim().Length == 0 && LibraryCategories.HiddenStudios(_facets, entries.Count) is > 0 and var more)
            _body.Children.Add(F.Text($"{more} more with one title each: search to find them.", Theme.FontCaption, fg: Theme.Secondary));
    }

    /// <summary>An adaptive grid's columns and width (Mac: GridItem(.adaptive(minimum:))).</summary>
    private static (int Columns, double Width) Columns(double avail, double min, double gap)
    {
        var n = Math.Max(1, (int)Math.Floor((avail + gap) / (min + gap)));
        return (n, Math.Floor((avail - (n - 1) * gap) / n));
    }

    /// <summary>Mac: LibraryTile, wide artwork with the name and count over a scrim.</summary>
    private FrameworkElement Tile(CategoryEntry e, double width)
    {
        var height = Math.Round(width * 9 / 16);
        var art = new Grid { Width = width, Height = height, CornerRadius = new CornerRadius(16) };
        if (e.Art != null)
        {
            var img = F.Art(_shell.Api.ImageUrl(e.Art, 800), 800);
            img.Width = width; img.Height = height;
            art.Children.Add(img);
        }
        else
        {
            art.Children.Add(new Border { Background = Theme.Gradient(0, 0, 1, 1, (0, Color.FromArgb(56, 255, 255, 255)), (1, Color.FromArgb(18, 255, 255, 255))) });
            art.Children.Add(F.Glyph(Glyph(Category, Shows), 40, new SolidColorBrush(Color.FromArgb(115, 255, 255, 255))));
        }
        art.Children.Add(new Border
        {
            IsHitTestVisible = false,
            Background = Theme.Gradient(0.5, 0, 0.5, 1, (0.25, Color.FromArgb(0, 0, 0, 0)), (0.55, Color.FromArgb(115, 0, 0, 0)), (1, Color.FromArgb(217, 0, 0, 0))),
        });
        var label = new StackPanel { Spacing = 1, VerticalAlignment = VerticalAlignment.Bottom, Margin = new Thickness(12) };
        label.Children.Add(F.Text(e.Title, Theme.FontHeadline, F.SemiBold, trim: TextTrimming.CharacterEllipsis));
        label.Children.Add(F.Text(LibraryCategories.Titles(Shows, e.Count), Theme.FontCaption, F.Medium, new SolidColorBrush(Color.FromArgb(217, 255, 255, 255))));
        art.Children.Add(label);
        var hover = F.HoverOverlay(16);
        art.Children.Add(hover);
        var b = F.Bare(art, () => _open(e), e.Title);
        F.HoverReveals(b, hover);
        return b;
    }

    /// <summary>A collection: its poster with the count in a badge, the name under it.</summary>
    private FrameworkElement PosterTile(CategoryEntry e, double width)
    {
        var height = Math.Round(width * 1.5);
        var art = new Grid { Width = width, Height = height };
        var image = e.Poster ?? e.Art;
        art.Children.Add(F.Image(image != null ? _shell.Api.ImageUrl(image, 500) : null, width, height, 12, (int)Math.Min(500, width * 2)));
        if (image == null) art.Children.Add(F.Glyph(Glyph(LibraryCategory.Collections, false), 28, Theme.Secondary));
        art.Children.Add(new Border
        {
            HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Top, Margin = new Thickness(6),
            Background = new SolidColorBrush(Color.FromArgb(153, 0, 0, 0)), CornerRadius = new CornerRadius(9), Padding = new Thickness(7, 2, 7, 2),
            Child = F.Text(e.Count.ToString(), Theme.FontCaption2, F.Bold),
        });
        var hover = F.HoverOverlay(12);
        art.Children.Add(hover);
        var stack = new StackPanel { Width = width, Spacing = 6 };
        stack.Children.Add(art);
        stack.Children.Add(F.Text(LibraryCategories.PosterName(e.Title), Theme.FontCaption, F.Medium, maxLines: 2));
        var b = F.Bare(stack, () => _open(e), e.Title);
        b.HorizontalContentAlignment = HorizontalAlignment.Left;
        b.VerticalAlignment = VerticalAlignment.Top;
        F.HoverReveals(b, hover);
        return b;
    }
}
