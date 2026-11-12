using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Moonrise.App.Ui;
using Moonrise.Core;
using Windows.UI;

namespace Moonrise.App.Pages;

/// <summary>
/// A title's page: backdrop header, Play or Resume, overview, seasons and episodes, cast and
/// details. Playing, the video takes the header's place under the top bar and the page scrolls
/// below it (Mac: ItemDetailView).
/// </summary>
public sealed class DetailPage : Grid, IRefreshable, IReflowable
{
    private readonly Shell _shell;
    private Item _item;
    private readonly ScrollViewer _scroll = new() { HorizontalScrollMode = ScrollMode.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Hidden };
    private readonly StackPanel _column = new() { Spacing = 18, Margin = new Thickness(Theme.Margin, 0, Theme.Margin, 40) };
    private readonly Grid _header = new();
    private IReadOnlyList<Item> _seasons = [];
    private IReadOnlyList<Item> _episodes = [];
    private Item? _focus;
    private string? _seasonId;
    private string? _selectedEpisode;
    private bool _loaded;
    private string? _headerFor;
    // The row shown now, only to read how far it was scrolled: each Render builds a new one.
    private ScrollViewer? _episodeRow;
    private string? _episodeRowSeason;
    private PageLayout _layout;
    // The player's height above the page while it plays here (null: not playing).
    private double? _player;

    public DetailPage(Shell shell, Item item)
    {
        _shell = shell;
        _item = item;
        Background = Theme.Background;
        var outer = new StackPanel();
        outer.Children.Add(_header);
        outer.Children.Add(_column);
        _scroll.Content = outer;
        Children.Add(_scroll);
    }

    public void Refresh()
    {
        // Back from the player or another page: Play and the row follow what the server says is next.
        if (_loaded) { _selectedEpisode = null; _seasonId = null; }
        _ = Load();
    }

    public void Reflow()
    {
        var layout = _shell.Layout;
        if (!_loaded || layout == _layout) return;
        _headerFor = null; // the picture's height follows the window
        Render();
    }

    private async Task Load()
    {
        try
        {
            var item = await _shell.Api.Item(_item.IsEpisode || _item.IsSeason ? _item.SeriesId ?? _item.Id : _item.Id);
            var wantSeason = _item.IsEpisode ? _item.SeasonId : _item.IsSeason ? _item.Id : null;
            var focusEpisode = _item.IsEpisode ? _item.Id : null;
            if (item.IsSeries || item.Kind == "series")
            {
                _seasons = await _shell.Api.Seasons(item.Id);
                _episodes = await _shell.Api.Episodes(item.Id);
                _focus = await _shell.Api.ContinueEpisode(item.Id);
                if (focusEpisode != null) _focus = _episodes.FirstOrDefault(e => e.Id == focusEpisode) ?? _focus;
                _seasonId = wantSeason ?? _seasonId ?? _focus?.SeasonId ?? _seasons.FirstOrDefault()?.Id;
            }
            _item = item;
            if (_selectedEpisode != null) _focus = _episodes.FirstOrDefault(e => e.Id == _selectedEpisode) ?? _focus;
            Render();
            _loaded = true;
        }
        catch (Exception e)
        {
            _column.Children.Clear();
            _column.Margin = new Thickness(Theme.Margin, 100, Theme.Margin, 40);
            _column.Children.Add(F.Text("Can't load this title. " + e.Message, Theme.FontBody));
            _column.Children.Add(F.Bare(F.Text("Try Again", Theme.FontBody, F.SemiBold), () => _ = Load()));
        }
    }

    private double HeaderHeight => _shell.Layout.HeroHeight;

    /// <summary>
    /// The player above the page, this tall (null: stopped, the page as before Play). The page
    /// scrolls below it: the title written out, no Play row (as on the Mac).
    /// </summary>
    public void PlayerShown(double? height)
    {
        var was = _player;
        _player = height;
        _scroll.Margin = new Thickness(0, height is { } h ? Shell.TopBarHeight + h : 0, 0, 0);
        _header.Visibility = height == null ? Visibility.Visible : Visibility.Collapsed;
        if ((was == null) == (height == null)) return; // only a new size
        _scroll.ChangeView(null, 0, null, true);
        if (_loaded) Render();
    }

    /// <summary>The player moved on to another episode: it's the one selected here, in its season.</summary>
    public void Follow(Item ep)
    {
        if (!_item.IsSeries || !ep.IsEpisode || ep.Id == _selectedEpisode) return;
        _selectedEpisode = ep.Id;
        _focus = _episodes.FirstOrDefault(e => e.Id == ep.Id) ?? ep;
        if (ep.SeasonId != null) _seasonId = ep.SeasonId;
        Render();
    }

    private void Render()
    {
        _layout = _shell.Layout;
        // Choosing a season or an episode leaves the picture alone; only a different title rebuilds it.
        var headerKey = $"{_item.Id}|{_item.BackdropImage}|{_item.LogoImage}";
        if (headerKey != _headerFor) { BuildHeader(); _headerFor = headerKey; }
        _column.Children.Clear();
        var playing = _player != null;
        _column.Margin = new Thickness(Theme.Margin, playing ? 18 : -150, Theme.Margin, 40);
        if (playing) _column.Children.Add(F.Text(_item.CardTitle, Theme.FontLargeTitle, F.Bold, maxLines: 2));

        // Title meta: runtime, date, rating badge, tech badges, genres.
        var meta = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10, VerticalAlignment = VerticalAlignment.Center };
        void Text(string? t) { if (!string.IsNullOrEmpty(t)) meta.Children.Add(F.Text(t, Theme.FontSubheadline, fg: Theme.Secondary)); }
        Text(Format.Runtime(_item.Runtime));
        Text(Format.PremiereText(_item.PremiereDate) ?? _item.Year?.ToString());
        if (!string.IsNullOrEmpty(_item.OfficialRating)) meta.Children.Add(Badge(_item.OfficialRating!));
        foreach (var b in Format.TechBadges(_item).Take(3)) meta.Children.Add(Badge(b));
        foreach (var g in (_item.Genres ?? []).Take(3)) Text(g);
        if (meta.Children.Count > 0) _column.Children.Add(meta);

        // Play.
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12, VerticalAlignment = VerticalAlignment.Center };
        var (playTitle, playAction) = PlayAction();
        var inner = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        inner.Children.Add(F.Glyph("", 16, Theme.Black));
        inner.Children.Add(new TextBlock { Text = playTitle, FontSize = Theme.FontHeadline, FontWeight = F.SemiBold, Foreground = Theme.Black, VerticalAlignment = VerticalAlignment.Center });
        var play = F.Capsule(inner, Theme.White, 52, 0);
        play.Width = 300;
        // No file can play (damaged, or gone from disk): what's wrong instead of Play.
        if (!_item.IsSeries && PlaybackProblem.Unplayable(_item) is { } problem) row.Children.Add(UnplayableNote(problem));
        else row.Children.Add(F.Bare(play, playAction));
        row.Children.Add(MoreButton());
        if (!playing) _column.Children.Add(row);

        // Focus line for episodes, and version choice.
        if (_item.IsSeries && _focus != null)
            _column.Children.Add(F.Text($"{_focus.EpisodeLabel} · {_focus.Title}", Theme.FontTitle3, F.SemiBold, trim: TextTrimming.CharacterEllipsis));
        if (_item.HasVersions) _column.Children.Add(VersionLine());

        if (Format.RatingsForPage(_item, _item.IsSeries ? _focus : null) is { } ratings) _column.Children.Add(RatingsRow(ratings));

        var overview = _item.IsSeries && _focus?.Overview is { Length: > 0 } ? _focus.Overview! : _item.Overview;
        if (!string.IsNullOrEmpty(overview))
        {
            var t = F.Text(overview!, Theme.FontBody, fg: new SolidColorBrush(Color.FromArgb(230, 255, 255, 255)), maxLines: 3);
            t.MaxWidth = 640;
            t.HorizontalAlignment = HorizontalAlignment.Left;
            _column.Children.Add(t);
            if (overview!.Length > 140)
            {
                var more = F.Bare(F.Text("More", Theme.FontSubheadline, F.SemiBold), () => t.MaxLines = t.MaxLines == 3 ? 0 : 3);
                more.HorizontalAlignment = HorizontalAlignment.Left;
                _column.Children.Add(more);
            }
        }

        if (_item.IsSeries) _column.Children.Add(SeasonsArea());

        var cast = (_item.People ?? []).Where(p => p.Name != null).Take(24).ToList();
        if (cast.Count > 0) _column.Children.Add(CastRow(cast));
        _column.Children.Add(DetailsGrid());
    }

    /// <summary>IMDb, TMDB's star, Rotten Tomatoes and Metacritic, the ones there are (Mac: ratingsRow).</summary>
    private static FrameworkElement RatingsRow(PageRatings r)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 20 };
        static FrameworkElement Value(string text) => F.Text(text, Theme.FontHeadline, F.SemiBold, new SolidColorBrush(Color.FromArgb(230, 255, 255, 255)));
        static FrameworkElement Mark(string text, Brush bg, Brush fg) => new Border
        {
            Background = bg, CornerRadius = new CornerRadius(3), Padding = new Thickness(4, 1, 4, 1), VerticalAlignment = VerticalAlignment.Center,
            Child = F.Text(text, Theme.FontCaption, Microsoft.UI.Text.FontWeights.ExtraBold, fg),
        };
        FrameworkElement Pair(FrameworkElement a, FrameworkElement b)
        {
            var p = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
            p.Children.Add(a);
            b.VerticalAlignment = VerticalAlignment.Center;
            p.Children.Add(b);
            return p;
        }
        if (r.Imdb != null) row.Children.Add(Pair(Mark("IMDb", Theme.ImdbYellow, Theme.Black), Value(r.Imdb)));
        if (r.Star != null) row.Children.Add(Pair(F.Glyph("\uE735", 13), Value(r.Star)));
        if (r.RottenTomatoes != null) row.Children.Add(Pair(F.Text("RT", Theme.FontCaption, F.Bold, Theme.Secondary), Value(r.RottenTomatoes)));
        if (r.Metacritic != null) row.Children.Add(Pair(Mark("MC", Theme.SurfaceStrong, Theme.White), Value(r.Metacritic)));
        return row;
    }

    /// <summary>In place of Play: the problem's title and line, and Details for the server's own words.</summary>
    private static FrameworkElement UnplayableNote(PlaybackProblem p)
    {
        var text = new StackPanel { Spacing = 2 };
        text.Children.Add(F.Text(p.Title, Theme.FontHeadline, F.SemiBold));
        var line = F.Text(p.Message, Theme.FontSubheadline, fg: Theme.Secondary);
        line.TextWrapping = TextWrapping.Wrap;
        text.Children.Add(line);
        if (p.Detail != null)
        {
            var detail = F.Text(p.Detail, Theme.FontCaption, fg: Theme.Secondary);
            detail.TextWrapping = TextWrapping.Wrap;
            detail.IsTextSelectionEnabled = true;
            detail.Visibility = Visibility.Collapsed;
            var toggle = F.Bare(F.Text("Details", Theme.FontSubheadline, F.SemiBold), () =>
                detail.Visibility = detail.Visibility == Visibility.Visible ? Visibility.Collapsed : Visibility.Visible);
            toggle.Margin = new Thickness(0, 4, 0, 0);
            text.Children.Add(toggle);
            text.Children.Add(detail);
        }
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        row.Children.Add(F.Glyph("\uE7BA", 18, new SolidColorBrush(Color.FromArgb(255, 255, 214, 10))));
        text.Width = 300 - 32 - 18 - 12;
        row.Children.Add(text);
        return new Border { Width = 300, MinHeight = 52, CornerRadius = new CornerRadius(16), Background = Theme.SurfaceStrong, Padding = new Thickness(16, 12, 16, 12), Child = row };
    }

    private (string Title, Action Action) PlayAction()
    {
        if (_item.IsSeries)
        {
            var ep = _focus ?? _episodes.FirstOrDefault();
            if (ep == null) return ("Play", () => { });
            var resume = ep.ResumeSeconds;
            return (resume != null ? $"Resume · {ep.EpisodeLabel}" : $"Play · {ep.EpisodeLabel}", () => _shell.Play(ep, resume, episodes: _episodes));
        }
        var version = Versions.Chosen(_item, null, true, null);
        var pos = Versions.Resume(_item, version);
        var title = pos != null ? $"Resume · {Format.Clock(pos.Value)}" : "Play";
        return (title, () => _shell.Play(_item, pos, version?.Id));
    }

    private FrameworkElement Badge(string text) => new Border
    {
        Padding = new Thickness(6, 2, 6, 2), CornerRadius = new CornerRadius(4), BorderThickness = new Thickness(1),
        BorderBrush = new SolidColorBrush(Color.FromArgb(128, 255, 255, 255)),
        Child = F.Text(text, 10, F.Bold, new SolidColorBrush(Color.FromArgb(217, 255, 255, 255))),
    };

    private FrameworkElement MoreButton()
    {
        Button b = null!;
        b = F.GlassCircle("", 52, 18, () => ShowMoreMenu(b));
        return b;
    }

    private void ShowMoreMenu(Button anchor)
    {
        var menu = new MenuFlyout();
        foreach (var a in WatchAction.Options(_item))
        {
            var it = new MenuFlyoutItem { Text = a.Title };
            it.Click += (_, _) => Safe.Fire(async () =>
            {
                try { await _shell.Api.SetPlayed(_item.Id, a.Played); _shell.Watch.Changed(_item.Id); await Load(); }
                catch (Exception e) { Flyout("Couldn't change that. " + e.Message, anchor); }
            });
            menu.Items.Add(it);
        }
        menu.ShowAt(anchor);
    }

    private void Flyout(string text, FrameworkElement anchor)
    {
        if (!anchor.IsLoaded) return; // the page moved on while the request ran
        var f = new Flyout { Content = new TextBlock { Text = text } };
        f.ShowAt(anchor);
    }

    private FrameworkElement VersionLine()
    {
        var version = Versions.Chosen(_item, null, true, null);
        var line = F.Text("Version: " + (version?.VersionTitle ?? ""), Theme.FontCaption, fg: Theme.Secondary);
        Button b = null!;
        b = F.Bare(line, () =>
        {
            var menu = new MenuFlyout();
            foreach (var f in Versions.Playable(_item.PresentFiles))
            {
                var it = new ToggleMenuFlyoutItem { Text = f.VersionTitle, IsChecked = f.Id == version?.Id };
                it.Click += (_, _) => Safe.Fire(async () => { try { await _shell.Api.ChooseVersion(_item.Id, f.Id); } catch { } await Load(); });
                menu.Items.Add(it);
            }
            menu.ShowAt(b);
        });
        b.HorizontalAlignment = HorizontalAlignment.Left;
        return b;
    }

    private void BuildHeader()
    {
        _header.Children.Clear();
        var h = HeaderHeight;
        _header.Height = h;
        var backdrop = _item.BackdropImage;
        if (backdrop != null)
        {
            var art = F.Art(_shell.Api.ImageUrl(backdrop, 1600), 1600);
            art.Height = h;
            art.IsHitTestVisible = false;
            _header.Children.Add(art);
        }
        else _header.Children.Add(new Border { Background = Theme.Gradient(0.5, 0, 0.5, 1, (0, Color.FromArgb(20, 255, 255, 255)), (1, Color.FromArgb(0, 255, 255, 255))), IsHitTestVisible = false });

        _header.Children.Add(new Border
        {
            IsHitTestVisible = false,
            Background = Theme.Gradient(0.5, 0.3, 0.5, 1, (0, Theme.WithAlpha(Theme.BackgroundColor, 0)), (0.7, Theme.WithAlpha(Theme.BackgroundColor, 0.6)), (1, Theme.BackgroundColor)),
        });
        if (backdrop != null)
            _header.Children.Add(new Border
            {
                IsHitTestVisible = false,
                Background = Theme.Gradient(0, 0.5, 0.65, 0.5, (0, Theme.WithAlpha(Theme.BackgroundColor, 0.8)), (0.55, Theme.WithAlpha(Theme.BackgroundColor, 0.35)), (1, Theme.WithAlpha(Theme.BackgroundColor, 0))),
            });

        var logo = _item.LogoImage;
        FrameworkElement title;
        if (logo != null)
        {
            var img = F.Art(_shell.Api.ImageUrl(logo, 760), 760, Stretch.Uniform);
            img.MaxHeight = Math.Min(130, h * 0.3);
            img.MaxWidth = 380;
            img.HorizontalAlignment = HorizontalAlignment.Left;
            title = img;
        }
        else title = F.Text(_item.CardTitle, Theme.FontLargeTitle, F.Bold, maxLines: 3); // wraps, as on the Mac
        title.HorizontalAlignment = HorizontalAlignment.Left;
        title.VerticalAlignment = VerticalAlignment.Bottom;
        title.Margin = new Thickness(Theme.Margin, 0, Theme.Margin, 170);
        _header.Children.Add(title);
    }

    private FrameworkElement SeasonsArea()
    {
        var area = new StackPanel { Spacing = 14 };
        if (_seasons.Count > 1)
        {
            var chips = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };
            foreach (var s in _seasons)
            {
                var selected = s.Id == _seasonId;
                var text = F.Text(s.Title ?? $"Season {s.SeasonNo}", Theme.FontSubheadline, F.SemiBold, selected ? Theme.Black : Theme.White);
                text.VerticalAlignment = VerticalAlignment.Center;
                var chip = F.Capsule(text, selected ? Theme.White : Theme.SurfaceStrong, 36, 16);
                chips.Children.Add(F.Bare(chip, () => { _seasonId = s.Id; _selectedEpisode = null; Render(); }));
            }
            area.Children.Add(chips);
        }
        var panel = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 14 };
        var w = ClientRules.CardWidth(_shell.PageWidth);
        var list = _episodes.Where(e => _seasonId == null || e.SeasonId == _seasonId).ToList();
        foreach (var ep in list)
            panel.Children.Add(EpisodeCard(ep, w));
        var scroller = F.Row(panel);
        // Selecting a card must not send the row back to its start.
        var keep = _episodeRowSeason == _seasonId ? _episodeRow?.HorizontalOffset ?? 0 : 0;
        if (keep > 0) scroller.Loaded += (_, _) => scroller.ChangeView(keep, null, null, true);
        _episodeRow = scroller;
        _episodeRowSeason = _seasonId;
        area.Children.Add(scroller);
        return area;
    }

    private FrameworkElement EpisodeCard(Item ep, double width)
    {
        var h = Math.Round(width * 9 / 16);
        var selected = _selectedEpisode == ep.Id;
        var art = new Grid { Width = width, Height = h };
        var thumb = ep.Images?.Thumb ?? ep.CardImage;
        var url = thumb != null ? _shell.Api.ImageUrl(thumb, 640) : null;
        art.Children.Add(F.Image(url, width, h, 16, 640));
        var hover = F.HoverOverlay(16);
        art.Children.Add(hover);
        if (selected)
        {
            art.Children.Add(new Border { BorderThickness = new Thickness(3), BorderBrush = Theme.White, CornerRadius = new CornerRadius(19), Margin = new Thickness(-5), IsHitTestVisible = false });
            art.Children.Add(new Border
            {
                Width = 50, Height = 50, CornerRadius = new CornerRadius(25), Background = Theme.Glass, BorderBrush = Theme.GlassBorder, BorderThickness = new Thickness(1),
                HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, IsHitTestVisible = false,
                Child = F.Glyph("", 18),
            });
        }
        if (ep.User?.Played == true)
            art.Children.Add(new Border { HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Top, Margin = new Thickness(0, 6, 8, 0), Child = F.Glyph("", 18) });
        if (ep.Progress is { } p)
        {
            var track = new Grid { Height = 4, VerticalAlignment = VerticalAlignment.Bottom, Margin = new Thickness(8, 0, 8, 8) };
            track.Children.Add(new Border { CornerRadius = new CornerRadius(2), Background = new SolidColorBrush(Color.FromArgb(128, 0, 0, 0)) });
            track.Children.Add(new Border { CornerRadius = new CornerRadius(2), Background = Theme.White, HorizontalAlignment = HorizontalAlignment.Left, Width = Math.Max(4, (width - 16) * p) });
            art.Children.Add(track);
        }
        var stack = new StackPanel { Width = width, Spacing = 8 };
        stack.Children.Add(art);
        var text = new StackPanel { Spacing = 2 };
        text.Children.Add(F.Text(ep.EpisodeListTitle, Theme.FontSubheadline, F.Medium, trim: TextTrimming.CharacterEllipsis));
        var sub = string.Join(" · ", new[] { Format.Runtime(ep.Runtime), Format.PremiereText(ep.PremiereDate) }.Where(x => !string.IsNullOrEmpty(x)));
        if (sub.Length > 0) text.Children.Add(F.Text(sub, Theme.FontCaption, F.Medium, Theme.Secondary));
        if (!string.IsNullOrEmpty(ep.Overview)) text.Children.Add(F.Text(ep.Overview!, Theme.FontCaption, fg: Theme.Secondary, maxLines: 3));
        stack.Children.Add(text);
        var button = F.Bare(stack, () => ActivateEpisode(ep));
        button.HorizontalContentAlignment = HorizontalAlignment.Left;
        button.VerticalAlignment = VerticalAlignment.Top; // cards with a longer overview stay level with the others
        F.HoverReveals(button, hover);
        return button;
    }

    /// <summary>First click selects, the second plays (Mac app); while one plays here, a click plays it.</summary>
    private void ActivateEpisode(Item ep)
    {
        if (_player != null && _shell.PlayingItemId == ep.Id) return; // playing already
        if (_player != null && _selectedEpisode != ep.Id) { _selectedEpisode = ep.Id; _focus = ep; Render(); }
        if (_selectedEpisode == ep.Id) { _shell.Play(ep, ep.ResumeSeconds, episodes: _episodes); return; }
        _selectedEpisode = ep.Id;
        _focus = ep;
        Render();
    }

    private FrameworkElement CastRow(List<Person> cast)
    {
        var header = F.Text("Cast and Crew", Theme.FontTitle3, F.SemiBold);
        header.Margin = new Thickness(0, 0, 0, Theme.ShelfHeaderGap);
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = Theme.CardGap };
        foreach (var p in cast)
        {
            var cell = new StackPanel { Width = 96, Spacing = 4 };
            var url = p.Image != null ? _shell.Api.ImageUrl(p.Image, 200) : null;
            var photo = F.Image(url, 84, 84, 20, 200);
            photo.HorizontalAlignment = HorizontalAlignment.Center;
            cell.Children.Add(photo);
            var name = F.Text(p.Name!, Theme.FontCaption, F.Medium, maxLines: 2); name.TextAlignment = TextAlignment.Center;
            cell.Children.Add(name);
            if (!string.IsNullOrEmpty(p.Role)) { var r = F.Text(p.Role!, Theme.FontCaption2, fg: Theme.Secondary, maxLines: 2); r.TextAlignment = TextAlignment.Center; cell.Children.Add(r); }
            row.Children.Add(cell);
        }
        var s = new StackPanel();
        s.Children.Add(header);
        s.Children.Add(F.Row(row));
        return s;
    }

    private FrameworkElement DetailsGrid()
    {
        var facts = new List<(string Label, string Value)>();
        void Add(string label, string? v) { if (!string.IsNullOrEmpty(v)) facts.Add((label, v)); }
        var studios = _item.Studios ?? [];
        string? Names(string g) { var n = (_item.People ?? []).Where(p => p.Group == g && p.Name != null).Select(p => p.Name!).Distinct().Take(3).ToList(); return n.Count == 0 ? null : string.Join(", ", n); }
        int Count(string g) => (_item.People ?? []).Count(p => p.Group == g);
        if (_item.IsSeries)
        {
            Add(studios.Count > 1 ? "Networks" : "Network", string.Join(", ", studios.Take(3)));
            Add("Seasons", _seasons.Count > 0 ? _seasons.Count.ToString() : null);
            Add("First Aired", Format.PremiereText(_item.PremiereDate));
        }
        else
        {
            Add(Count("Director") > 1 ? "Directors" : "Director", Names("Director"));
            Add(Count("Writer") > 1 ? "Writers" : "Writer", Names("Writer"));
            Add(studios.Count > 1 ? "Studios" : "Studio", string.Join(", ", studios.Take(3)));
            Add("Released", Format.PremiereText(_item.PremiereDate));
        }
        Add("Genres", string.Join(", ", (_item.Genres ?? []).Take(4)));
        var file = _item.IsSeries ? _focus : _item;
        Add("Rated", _item.OfficialRating ?? file?.OfficialRating);
        var version = file?.PresentFiles.FirstOrDefault();
        var tracks = version?.Tracks ?? [];
        var prefix = _item.IsSeries ? "Episode " : "";
        if (tracks.FirstOrDefault(t => t.IsVideo) is { } v)
            Add(prefix + "Video", v.DisplayTitle ?? string.Join(" ", new[] { v.Height is { } h ? $"{h}p" : null, v.Codec?.ToUpperInvariant(), v.VideoRangeType }.Where(x => x != null)));
        Add(prefix + "Audio", Langs(tracks.Where(t => t.IsAudio)));
        Add(prefix + "Subtitles", Langs(tracks.Where(t => t.IsSubtitle)));
        if (version != null)
        {
            var parts = new List<string>();
            if (version.Name != null && version.Name.LastIndexOf('.') is var dot and > 0 && version.Name.Length - dot - 1 is >= 1 and <= 5)
                parts.Add(version.Name[(dot + 1)..].ToUpperInvariant());
            if (version.Size is { } sz) parts.Add(Format.FileSize(sz));
            if (version.Bitrate is { } br) parts.Add($"{br / 1e6:F1} Mbps");
            Add(prefix + "File", string.Join(" · ", parts));
        }
        // A cell is its 240 and the gap after it: the right margin takes the last gap, so the last column fits.
        var grid = new VariableSizedWrapGrid { Orientation = Orientation.Horizontal, ItemWidth = 254, ItemHeight = 58, Margin = new Thickness(0, 0, -14, 0) };
        foreach (var (label, value) in facts)
        {
            var s = new StackPanel { Spacing = 2, Width = 240 };
            s.Children.Add(F.Text(label, Theme.FontCaption2, fg: Theme.Secondary));
            s.Children.Add(F.Text(value, Theme.FontCaption, maxLines: 2));
            grid.Children.Add(s);
        }
        var outer = new StackPanel { Margin = new Thickness(0, 8, 0, 0) };
        outer.Children.Add(F.Text("Details", Theme.FontTitle3, F.SemiBold));
        outer.Children.Add(new Border { Height = Theme.ShelfHeaderGap });
        outer.Children.Add(grid);
        return outer;
    }

    private static string? Langs(IEnumerable<Track> tracks)
    {
        var seen = tracks.Select(TrackLabels.AudioName).Distinct().ToList();
        if (seen.Count == 0) return null;
        return seen.Count > 6 ? string.Join(", ", seen.Take(6)) + $" +{seen.Count - 6}" : string.Join(", ", seen);
    }
}
