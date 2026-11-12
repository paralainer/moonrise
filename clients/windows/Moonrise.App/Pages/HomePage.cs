using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Moonrise.App.Ui;
using Moonrise.Core;
using Windows.UI;

namespace Moonrise.App.Pages;

/// <summary>Home: the hero carousel, Continue Watching (landscape), then Recently Added rows (posters).</summary>
public sealed class HomePage : Grid, IRefreshable, IReflowable
{
    private readonly Shell _shell;
    private readonly ScrollViewer _scroll = new() { HorizontalScrollMode = ScrollMode.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Hidden };
    private readonly StackPanel _content = new() { Spacing = Theme.SectionGap, Padding = new Thickness(0, 0, 0, 40) };
    private readonly Grid _empty = new() { Visibility = Visibility.Collapsed };
    private readonly ProgressRing _ring = new() { IsActive = true, Width = 40, Height = 40, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
    private DispatcherQueueTimer? _heroTimer;
    private bool _loading;
    private long _lastLoadMs;
    private bool _failed;
    private Home? _home;
    private PageLayout _layout;
    private int _watchSeen;

    public HomePage(Shell shell)
    {
        _shell = shell;
        Background = Theme.Background;
        _scroll.Content = _content;
        Children.Add(_scroll);
        Children.Add(_empty);
        Children.Add(_ring);
        Unloaded += (_, _) => _heroTimer?.Stop();
        Loaded += (_, _) => _heroTimer?.Start();
        _watchSeen = shell.Watch.Version;
    }

    public void Reflow()
    {
        var layout = _shell.Layout;
        if (_home == null || layout == _layout) return;
        Show(_home);
    }

    public void Refresh()
    {
        // Continue Watching follows what was just watched, not only the clock.
        var watched = _shell.Watch.Since(ref _watchSeen).Count > 0;
        if (watched || ClientRules.HomeIsStale(_loading, _failed, _lastLoadMs, Environment.TickCount64) || _lastLoadMs == 0) _ = Load();
    }

    private async Task Load()
    {
        _loading = true;
        try
        {
            var home = await _shell.Api.Home();
            _failed = false;
            _ring.IsActive = false;
            _ring.Visibility = Visibility.Collapsed;
            Show(home);
        }
        catch (Exception e)
        {
            _failed = true;
            _ring.Visibility = Visibility.Collapsed;
            _home = null;
            ShowMessage("Nothing Here", "Can't reach the server. " + e.Message, retry: true);
        }
        _loading = false;
        _lastLoadMs = Environment.TickCount64;
    }

    private void ShowMessage(string title, string text, bool retry = false)
    {
        _empty.Children.Clear();
        var stack = new StackPanel { Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        stack.Children.Add(F.Glyph("", 48, Theme.Secondary));
        var t = F.Text(title, Theme.FontTitle3, F.SemiBold); t.HorizontalAlignment = HorizontalAlignment.Center; stack.Children.Add(t);
        var s = F.Text(text, Theme.FontSubheadline, fg: Theme.Secondary); s.HorizontalAlignment = HorizontalAlignment.Center; stack.Children.Add(s);
        if (retry)
        {
            var again = F.Bare(F.Text("Try Again", Theme.FontBody, F.SemiBold), () => _ = Load());
            again.HorizontalAlignment = HorizontalAlignment.Center;
            stack.Children.Add(again);
        }
        _empty.Children.Add(stack);
        _empty.Visibility = Visibility.Visible;
        _scroll.Visibility = Visibility.Collapsed;
    }

    private void Show(Home home)
    {
        _home = home;
        _layout = _shell.Layout;
        _scroll.Visibility = Visibility.Visible;
        _empty.Visibility = Visibility.Collapsed;
        _content.Children.Clear();
        _heroTimer?.Stop();
        var posterW = _layout.Poster;
        var cardW = _layout.Card;
        var hero = home.LatestTypes.SelectMany(r => r.Items.Where(i => !i.IsEpisode && !i.IsSeason && i.BackdropImage != null).Take(3)).Take(6).ToList();
        if (hero.Count > 0) _content.Children.Add(BuildHero(hero));
        // Without a hero the first row would sit under the top bar.
        _content.Padding = new Thickness(0, hero.Count > 0 ? 0 : Shell.TopBarHeight + 8, 0, 40);

        var cont = home.ContinueRow;
        if (cont.Count > 0)
            _content.Children.Add(Shelf.Build("Continue Watching", cont.Select(i => Cards.Landscape(_shell.Api, i, cardW, () => Activate(i))), cardBottomPad: 40));
        foreach (var row in home.LatestTypes.Where(r => r.Items.Count > 0))
            _content.Children.Add(Shelf.Build("Recently Added " + row.Title, row.Items.Select(i => Cards.Poster(_shell.Api, i, posterW, () => _shell.Open(i)))));
        if (_content.Children.Count == 0) ShowMessage("Nothing Here", "This server has nothing to show yet.");
    }

    private void Activate(Item item)
    {
        // Continue Watching: resume where it was left.
        if (item.IsSeries) _shell.Open(item);
        else _shell.Play(item, item.ResumeSeconds);
    }

    private FrameworkElement BuildHero(List<Item> items)
    {
        var h = _layout.HeroHeight;
        var grid = new Grid { Height = h };
        var images = new List<FrameworkElement>();
        var slides = new HeroSlides(items.Count);
        foreach (var it in items)
        {
            var at = images.Count;
            var art = F.Art(_shell.Api.ImageUrl(it.BackdropImage!, 1280), 1280, opened: img =>
            {
                slides.Opened(at);
                img.Opacity = slides.Opacity(at);
            });
            art.Height = h;
            art.IsHitTestVisible = false;
            images.Add(art);
            grid.Children.Add(art);
        }
        grid.Children.Add(new Border
        {
            IsHitTestVisible = false,
            Background = Theme.Gradient(0.5, 0.3, 0.5, 1, (0, Theme.WithAlpha(Theme.BackgroundColor, 0)), (0.7, Theme.WithAlpha(Theme.BackgroundColor, 0.6)), (1, Theme.BackgroundColor)),
        });
        grid.Children.Add(new Border
        {
            IsHitTestVisible = false,
            Background = Theme.Gradient(0, 0.5, 0.65, 0.5, (0, Theme.WithAlpha(Theme.BackgroundColor, 0.8)), (0.55, Theme.WithAlpha(Theme.BackgroundColor, 0.35)), (1, Theme.WithAlpha(Theme.BackgroundColor, 0))),
        });

        var info = new StackPanel { VerticalAlignment = VerticalAlignment.Bottom, Margin = new Thickness(Theme.Margin, 0, Theme.Margin, 20), Spacing = 6, MaxWidth = 700, HorizontalAlignment = HorizontalAlignment.Left };
        var logo = new Grid { Height = 90, HorizontalAlignment = HorizontalAlignment.Left };
        var titleText = F.Text("", Theme.FontLargeTitle, F.Bold, maxLines: 3); // as on the title page
        var meta = F.Text("", Theme.FontSubheadline, fg: Theme.Secondary, trim: TextTrimming.CharacterEllipsis);
        var buttons = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12, Margin = new Thickness(0, 10, 0, 0) };
        Item current = items[0];
        var playText = new TextBlock { Text = "Play", Foreground = Theme.Black, FontWeight = F.SemiBold, FontSize = Theme.FontHeadline, VerticalAlignment = VerticalAlignment.Center };
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        row.Children.Add(F.Glyph("", 16, Theme.Black));
        row.Children.Add(playText);
        var play = F.Capsule(row, Theme.White, 52, 0);
        play.Width = 300;
        buttons.Children.Add(F.Bare(play, () => _shell.Play(current, current.ResumeSeconds)));
        buttons.Children.Add(F.GlassCircle("", 52, 18, () => _shell.Open(current), "More Info"));
        info.Children.Add(logo);
        info.Children.Add(titleText);
        info.Children.Add(meta);
        info.Children.Add(buttons);
        grid.Children.Add(info);

        var dots = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 7, HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Bottom, Margin = new Thickness(0, 0, Theme.Margin, 28) };
        var dotList = items.Select(_ => new Border { Width = 6, Height = 6, CornerRadius = new CornerRadius(3), Background = new SolidColorBrush(Color.FromArgb(77, 255, 255, 255)) }).ToList();
        foreach (var d in dotList) dots.Children.Add(d);
        if (items.Count > 1) grid.Children.Add(dots);

        var index = 0;
        void Show(int i)
        {
            index = i;
            current = items[i];
            slides.Show(i);
            for (var k = 0; k < items.Count; k++)
            {
                images[k].Opacity = slides.Opacity(k);
                dotList[k].Background = new SolidColorBrush(Color.FromArgb(k == i ? (byte)230 : (byte)77, 255, 255, 255));
            }
            logo.Children.Clear();
            if (current.LogoImage != null)
            {
                logo.Children.Add(F.Art(_shell.Api.ImageUrl(current.LogoImage, 760), 760, Stretch.Uniform));
                ((Image)logo.Children[0]).HorizontalAlignment = HorizontalAlignment.Left;
                logo.Visibility = Visibility.Visible; titleText.Visibility = Visibility.Collapsed;
            }
            else { logo.Visibility = Visibility.Collapsed; titleText.Visibility = Visibility.Visible; titleText.Text = current.CardTitle; }
            meta.Text = string.Join(" · ", new[] { current.Year?.ToString(), Format.Runtime(current.Runtime), string.Join(", ", (current.Genres ?? []).Take(3)) }.Where(x => !string.IsNullOrEmpty(x)));
            playText.Text = current.ResumeSeconds == null ? "Play" : "Resume";
        }
        Show(0);
        if (items.Count > 1)
        {
            _heroTimer = DispatcherQueue.CreateTimer();
            _heroTimer.Interval = TimeSpan.FromSeconds(7);
            _heroTimer.Tick += Safe.Tick(() => Show((index + 1) % items.Count));
            _heroTimer.Start();
        }
        return grid;
    }
}
