using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Moonrise.App.Ui;
using Moonrise.Core;

namespace Moonrise.App.Pages;

/// <summary>Search: Recent and Suggestions before typing; Shows, Movies and Episodes as you type.</summary>
public sealed class SearchPage : Grid, IRefreshable, IReflowable, IFocusable
{
    private readonly Shell _shell;
    private readonly TextBox _box = new() { PlaceholderText = "Movies, shows, episodes", Width = 520, HorizontalAlignment = HorizontalAlignment.Center, Height = 36 };
    private readonly ScrollViewer _scroll = new() { HorizontalScrollMode = ScrollMode.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Hidden };
    private readonly StackPanel _content = new() { Spacing = 26, Padding = new Thickness(0, 16, 0, 40) };
    private readonly DispatcherQueueTimer _debounce;
    private SearchStart? _start;
    private int _generation;
    private PageLayout _layout;
    private Action? _redraw; // what's on screen, to draw again at a new width

    public SearchPage(Shell shell)
    {
        _shell = shell;
        Background = Theme.Gradient(0.5, 0, 0.5, 1, (0, Windows.UI.Color.FromArgb(255, 0x1A, 0x1C, 0x26)), (1, Theme.BackgroundColor));
        RowDefinitions.Add(F.RowAuto);
        RowDefinitions.Add(F.RowStar);
        var head = new Grid { Margin = new Thickness(0, Shell.TopBarHeight + 6, 0, 0) };
        head.Children.Add(_box);
        Children.Add(head.At(0, 0));
        _scroll.Content = _content;
        Children.Add(_scroll.At(1, 0));
        _debounce = DispatcherQueue.CreateTimer();
        _debounce.Interval = TimeSpan.FromMilliseconds(350);
        _debounce.IsRepeating = false;
        _debounce.Tick += Safe.Tick(() => _ = Run());
        _box.TextChanged += (_, _) => Changed();
        Loaded += (_, _) => TakeFocus();
    }

    public void TakeFocus() => _box.Focus(FocusState.Programmatic);

    public void Refresh()
    {
        if (SearchStart.Shows(_box.Text)) _ = LoadStart();
    }

    public void Reflow()
    {
        var layout = _shell.Layout;
        if (layout == _layout) return;
        _layout = layout;
        _redraw?.Invoke();
    }

    private void Changed()
    {
        var q = _box.Text.Trim();
        if (SearchStart.Shows(q)) { _debounce.Stop(); _ = LoadStart(); return; }
        _debounce.Start();
    }

    private async Task LoadStart()
    {
        var gen = ++_generation;
        try
        {
            var start = await _shell.Api.SearchStart();
            if (gen != _generation) return;
            _start = start;
            ShowStart();
        }
        catch (Exception e) { if (gen == _generation) Message(e.Message, ""); }
    }

    private void ShowStart()
    {
        _redraw = ShowStart;
        _layout = _shell.Layout;
        _content.Children.Clear();
        if (_start == null || _start.IsEmpty)
        {
            Message("Search Your Library", "Find movies, shows and episodes by name.");
            return;
        }
        var width = _shell.PageWidth;
        var posterW = ClientRules.PosterWidth(width);
        if (_start.Recent.Count > 0)
        {
            var clear = F.Bare(F.Text("Clear", Theme.FontSubheadline, fg: Theme.Secondary), () => _ = ClearRecent());
            _content.Children.Add(Shelf.Build("Recent", _start.Recent.Select(i => Cards.Poster(_shell.Api, i, posterW, () => Opened(i))), accessory: clear));
        }
        if (_start.Suggestions.Count > 0) _content.Children.Add(PosterRows("Suggestions", _start.Suggestions));
    }

    private async Task ClearRecent()
    {
        try { await _shell.Api.ClearSearchRecent(); } catch { }
        _start = _start?.RecentCleared();
        ShowStart();
    }

    private void Opened(Item item)
    {
        _start = _start?.Opened(item);
        _ = _shell.Api.RecordSearchOpened(item.Id);
        _shell.Open(item);
    }

    private async Task Run()
    {
        var q = _box.Text.Trim();
        if (q.Length < 2) return;
        var gen = ++_generation;
        try
        {
            var r = await _shell.Api.Search(q);
            if (gen != _generation) return;
            Show(q, r);
        }
        catch (Exception e) { if (gen == _generation) Message("Can't search", e.Message); }
    }

    private void Show(string q, SearchResults r)
    {
        _redraw = () => Show(q, r);
        _layout = _shell.Layout;
        _content.Children.Clear();
        var shows = r.Titles.Where(t => t.IsSeries).ToList();
        var movies = r.Titles.Where(t => t.IsMovie).ToList();
        if (shows.Count == 0 && movies.Count == 0 && r.Episodes.Count == 0)
        {
            Message("No Results", $"Nothing matched “{q}”.");
            return;
        }
        var width = _shell.PageWidth;
        if (shows.Count > 0) _content.Children.Add(PosterRows("Shows", shows));
        if (movies.Count > 0) _content.Children.Add(PosterRows("Movies", movies));
        if (r.Episodes.Count > 0)
            _content.Children.Add(Shelf.Build("Episodes", r.Episodes.Select(i => Cards.Landscape(_shell.Api, i, ClientRules.CardWidth(width), () => Opened(i))), cardBottomPad: 40));
    }

    private FrameworkElement PosterRows(string title, IReadOnlyList<Item> items)
    {
        var g = _layout.Grid;
        var header = F.Text(title, Theme.FontTitle3, F.SemiBold);
        header.Margin = new Thickness(Theme.Margin, 0, Theme.Margin, Theme.ShelfHeaderGap);
        var grid = new VariableSizedWrapGrid
        {
            Orientation = Orientation.Horizontal, ItemWidth = g.CellWidth, ItemHeight = g.CellHeight, MaximumRowsOrColumns = g.Columns,
            Width = g.PanelWidth, HorizontalAlignment = HorizontalAlignment.Left, Margin = new Thickness(Theme.Margin, 0, 0, 0),
        };
        foreach (var i in items) grid.Children.Add(Cards.Poster(_shell.Api, i, g.CardWidth, () => Opened(i)));
        var outer = new StackPanel();
        outer.Children.Add(header);
        outer.Children.Add(grid);
        return outer;
    }

    private void Message(string title, string text)
    {
        _redraw = null;
        _content.Children.Clear();
        var s = new StackPanel { Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 80, 0, 0) };
        s.Children.Add(F.Glyph("", 48, Theme.Secondary));
        var t = F.Text(title, Theme.FontTitle3, F.SemiBold); t.HorizontalAlignment = HorizontalAlignment.Center; s.Children.Add(t);
        if (text.Length > 0) { var x = F.Text(text, Theme.FontSubheadline, fg: Theme.Secondary); x.HorizontalAlignment = HorizontalAlignment.Center; s.Children.Add(x); }
        _content.Children.Add(s);
    }
}
