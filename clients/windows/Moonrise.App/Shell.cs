using Microsoft.UI.Dispatching;
using Microsoft.UI.Input;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.Graphics;
using Moonrise.App.Pages;
using Moonrise.App.Player;
using Moonrise.App.Ui;
using Moonrise.Core;

namespace Moonrise.App;

public enum Tab { Home, Library, Search }

/// <summary>
/// The window: the top tab bar (Mac: tabs in the toolbar), one navigation stack per tab, and the
/// player: on its title page under the top bar (Mac: ItemDetailView), or over everything in full
/// screen. The same player either way, so the stream goes on.
/// </summary>
public sealed class Shell
{
    private readonly MainWindow _window;
    private readonly Grid _root;
    private readonly Grid _pageHost = new();
    private readonly Grid _topBar = new() { Height = TopBarHeight, VerticalAlignment = VerticalAlignment.Top };
    private readonly Grid _tabs = new() { HorizontalAlignment = HorizontalAlignment.Center };
    private readonly Button _back;
    private readonly Button _account;
    private readonly Grid _playerHost = new() { Visibility = Visibility.Collapsed };
    private readonly Grid _faultHost = new() { Visibility = Visibility.Collapsed };
    private readonly FaultGate _faults = new();
    private readonly DispatcherQueueTimer _reflow;
    private readonly DispatcherQueueTimer _refocus;
    private readonly Dictionary<Tab, Stack<FrameworkElement>> _stacks = new()
    {
        [Tab.Home] = new(), [Tab.Library] = new(), [Tab.Search] = new(),
    };
    private readonly Dictionary<Tab, Border> _tabBorders = new();
    private Tab _tab = Tab.Home;
    private PlayerPage? _player;
    // The title page the player plays on: leaving it stops the player (Mac).
    private DetailPage? _playerPage;
    private AppWindowPresenterKind _beforeFull = AppWindowPresenterKind.Overlapped;
    private Native.WindowPlacement? _windowBeforeFull;
    private OverlappedPresenterState? _windowState;

    public const double TopBarHeight = 48;

    public Session Session { get; }
    public MoonriseApi Api => Session.Api!;
    public MainWindow Window => _window;
    public double PageWidth => _root.ActualWidth > 0 ? _root.ActualWidth : 1280;
    public double PageHeight => _root.ActualHeight > 0 ? _root.ActualHeight : 800;
    public PageLayout Layout => ClientRules.PageLayoutFor(PageWidth, PageHeight);
    public WatchChanges Watch { get; } = new();

    public Shell(MainWindow window, Grid root)
    {
        _window = window;
        _root = root;
        Session = Session.Load();
        _root.Background = Theme.Background;
        _root.Children.Add(_pageHost);

        _back = F.Bare(new Border { Width = 40, Height = 32, Child = F.Glyph("", 14) }, Back, "Back");
        _back.HorizontalAlignment = HorizontalAlignment.Left;
        _back.Margin = new Thickness(12, 0, 0, 0);
        _back.Visibility = Visibility.Collapsed;
        _account = F.Bare(new Border { Width = 36, Height = 32, Child = F.Glyph("", 15, Theme.Secondary) }, AccountMenu, "Account");
        _account.HorizontalAlignment = HorizontalAlignment.Right;
        _account.Margin = new Thickness(0, 0, 150, 0);

        BuildTabs();
        _topBar.Children.Add(_tabs);
        _topBar.Children.Add(_back);
        _topBar.Children.Add(_account);
        _root.Children.Add(_topBar);
        _root.Children.Add(_playerHost);
        _root.Children.Add(_faultHost);

        // Pages built from the window's size are drawn again once a resize has settled.
        _reflow = _window.DispatcherQueue.CreateTimer();
        _reflow.Interval = TimeSpan.FromMilliseconds(180);
        _reflow.IsRepeating = false;
        _reflow.Tick += Safe.Tick(() => ReflowCurrent());
        _root.SizeChanged += (_, _) => { LayoutPlayer(); _reflow.Stop(); _reflow.Start(); };
        _refocus = _window.DispatcherQueue.CreateTimer();
        _refocus.Interval = TimeSpan.FromMilliseconds(250);
        _refocus.IsRepeating = false;
        _refocus.Tick += Safe.Tick(FocusPage);

        _root.Loaded += (_, _) =>
        {
            _window.SetTitleBar(_topBar);
            UpdatePassthrough();
            Start();
        };
        _topBar.SizeChanged += (_, _) => UpdatePassthrough();
        _windowState = (_window.AppWin.Presenter as OverlappedPresenter)?.State;
        _window.AppWin.Changed += (_, a) => { if (a.DidSizeChange || a.DidPresenterChange) Safe.Run(WindowStateMaybeChanged); };
    }

    /// <summary>
    /// Maximize or Restore from the caption buttons left the keyboard with nothing in the window: the
    /// player's keys stopped until a click on it, and typing on Search went nowhere.
    /// The player, or else the page's main control, takes it back after the change.
    /// </summary>
    private void WindowStateMaybeChanged()
    {
        var state = (_window.AppWin.Presenter as OverlappedPresenter)?.State;
        if (state == _windowState) return;
        _windowState = state;
        if (state == OverlappedPresenterState.Minimized) return;
        LayoutPlayer();
        if (_player != null) _player.TakeKeyboardSoon();
        else FocusPageSoon();
    }

    // The state change settles after this returns: focus again once it has.
    private void FocusPageSoon()
    {
        FocusPage();
        _refocus.Stop();
        _refocus.Start();
    }

    private void FocusPage()
    {
        if (_player == null && _stacks[_tab].TryPeek(out var page)) (page as IFocusable)?.TakeFocus();
    }

    private void BuildTabs()
    {
        // Mac: Home, Library, Search in the toolbar; a capsule with the selected tab raised.
        var panel = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
        Add(panel, Tab.Home, "Home", "");
        Add(panel, Tab.Library, "Library", "");
        Add(panel, Tab.Search, "Search", "");
        _tabs.Children.Add(new Border
        {
            Background = Theme.SurfaceStrong, CornerRadius = new CornerRadius(17), Padding = new Thickness(3), Height = 34, Child = panel,
        });
    }

    private void Add(StackPanel panel, Tab tab, string title, string glyph)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, VerticalAlignment = VerticalAlignment.Center, Padding = new Thickness(14, 0, 14, 0) };
        row.Children.Add(F.Glyph(glyph, 13));
        row.Children.Add(F.Text(title, Theme.FontBody, F.Medium));
        var border = new Border { CornerRadius = new CornerRadius(14), Child = row };
        _tabBorders[tab] = border;
        var b = F.Bare(border, () => Select(tab));
        panel.Children.Add(b);
    }

    private void ReflowCurrent()
    {
        if (_stacks[_tab].TryPeek(out var page)) (page as IReflowable)?.Reflow();
    }

    private void UpdatePassthrough()
    {
        // Only where clicks reach the title bar's row; nothing to tell anyone if it can't be set now.
        try { SetPassthrough(); }
        catch (Exception e) { AppLog.Write("passthrough: " + e.Message); }
    }

    private void SetPassthrough()
    {
        var src = InputNonClientPointerSource.GetForWindowId(_window.AppWin.Id);
        var scale = _root.XamlRoot?.RasterizationScale ?? 1;
        // On its page the player sits under the top bar; only full screen puts it over the title bar's row.
        var covering = _player != null && IsFullScreen;
        var controls = covering && _player != null ? _player.TitleRowControls : new FrameworkElement[] { _tabs, _back, _account };
        var rects = controls.Where(e => e.Visibility == Visibility.Visible && e.ActualWidth > 0).Select(e => InWindow(e, scale)).ToList();
        if (covering)
            rects = ClientRules.PlayerTitleRowPassthrough(IsFullScreen, (int)(_root.ActualWidth * scale), (int)(TopBarHeight * scale), rects).ToList();
        src.SetRegionRects(NonClientRegionKind.Passthrough, rects.Select(r => new RectInt32(r.X, r.Y, r.Width, r.Height)).ToArray());
    }

    private PixelRect InWindow(FrameworkElement e, double scale)
    {
        var pt = e.TransformToVisual(_root).TransformPoint(new Windows.Foundation.Point(0, 0));
        return new PixelRect((int)(pt.X * scale), (int)(pt.Y * scale), (int)(e.ActualWidth * scale), (int)(e.ActualHeight * scale));
    }

    /// <summary>The player's controls in the title bar's row moved or changed size.</summary>
    public void TitleRowChanged() => UpdatePassthrough();

    private void Start()
    {
        if (Session.Api == null) ShowSignIn();
        else Select(Tab.Home);
    }

    public void ShowSignIn()
    {
        if (_player != null) ClosePlayer();
        _topBar.Visibility = Visibility.Collapsed;
        foreach (var s in _stacks.Values) s.Clear();
        F.ShowOnly(_pageHost, new SignInPage(this));
    }

    public void SignedIn()
    {
        _topBar.Visibility = Visibility.Visible;
        Select(Tab.Home);
    }

    public void Select(Tab tab)
    {
        if (_stacks[tab].Count == 0)
            _stacks[tab].Push(tab switch { Tab.Home => new HomePage(this), Tab.Library => new LibraryPage(this), _ => new SearchPage(this) });
        if (ClientRules.TabClickReturnsToRoot(tab == _tab, _stacks[tab].Count))
            while (_stacks[tab].Count > 1) _stacks[tab].Pop();
        _tab = tab;
        foreach (var (t, b) in _tabBorders) b.Background = t == tab ? Theme.SurfaceStrong : Theme.Transparent;
        ShowCurrent();
    }

    private void ShowCurrent()
    {
        if (!_stacks[_tab].TryPeek(out var page)) return;
        // Leaving the title page that plays stops it (Mac).
        if (_player != null && !ReferenceEquals(page, _playerPage)) ClosePlayer();
        F.ShowOnly(_pageHost, page);
        _back.Visibility = _stacks[_tab].Count > 1 ? Visibility.Visible : Visibility.Collapsed;
        (page as IRefreshable)?.Refresh();
        (page as IReflowable)?.Reflow(); // the window may have changed while this page was hidden
        _window.DispatcherQueue.Enqueue(UpdatePassthrough);
    }

    public void Push(FrameworkElement page)
    {
        _stacks[_tab].Push(page);
        ShowCurrent();
    }

    public void Back()
    {
        if (_stacks[_tab].Count > 1) { _stacks[_tab].Pop(); ShowCurrent(); }
    }

    public void Open(Item item) => Push(new DetailPage(this, item));

    /// <summary>Settings, on the tab showing; already open there, it stays as it is.</summary>
    public void OpenSettings()
    {
        if (_stacks[_tab].TryPeek(out var top) && top is SettingsPage) return;
        Push(new SettingsPage());
    }

    /// <summary>
    /// Plays on the title page showing, or on the title's page opened for it (Home's Play and
    /// Continue Watching: Mac's autoplay), under the top bar.
    /// </summary>
    public void Play(Item item, double? start = null, string? version = null, IReadOnlyList<Item>? episodes = null, bool startFromBeginning = false)
    {
        if (!_stacks[_tab].TryPeek(out var top) || top is not DetailPage page)
        {
            page = new DetailPage(this, item);
            if (_player != null) ClosePlayer();
            Push(page);
        }
        _player?.Stop();
        _playerPage = page;
        _player = new PlayerPage(this, item, start, version, episodes, startFromBeginning);
        F.ShowOnly(_playerHost, _player);
        _playerHost.Visibility = Visibility.Visible;
        LayoutPlayer();
        _window.DispatcherQueue.Enqueue(UpdatePassthrough);
    }

    /// <summary>
    /// The player's place: on its page, 16:9 across under the top bar but no more than 60 % of the
    /// page (the page shows below it); full screen, all of the window. The page makes room for it.
    /// </summary>
    private void LayoutPlayer()
    {
        if (_player == null) return;
        var full = IsFullScreen;
        if (full)
        {
            _player.Height = double.NaN;
            _player.VerticalAlignment = VerticalAlignment.Stretch;
            _player.Margin = new Thickness(0);
        }
        else
        {
            _player.Height = ClientRules.InlinePlayerHeight(PageWidth, PageHeight - TopBarHeight);
            _player.VerticalAlignment = VerticalAlignment.Top;
            _player.Margin = new Thickness(0, TopBarHeight, 0, 0);
        }
        _player.SetFullScreen(full);
        _playerPage?.PlayerShown(full ? null : _player.Height);
    }

    public string? PlayingItemId => _player?.ItemId;

    /// <summary>The player moved on to another episode (Next, Episodes): its page follows.</summary>
    public void PlayerItemChanged(Item item) => _playerPage?.Follow(item);

    public void ClosePlayer()
    {
        LeaveFullScreen();
        if (_player?.WatchedId is { } id) Watch.Changed(id);
        _player?.Stop();
        _player = null;
        _playerHost.Children.Clear();
        _playerHost.Visibility = Visibility.Collapsed;
        var playedOn = _playerPage;
        _playerPage = null;
        playedOn?.PlayerShown(null);
        if (_stacks[_tab].TryPeek(out var page)) (page as IRefreshable)?.Refresh();
        _window.DispatcherQueue.Enqueue(UpdatePassthrough);
    }

    public void WindowClosed() => _player?.Stop();

    /// <summary>
    /// An error nothing else caught (App.UnhandledException, already logged): the app stays up and
    /// says so, with a way back to a page built from nothing.
    /// </summary>
    public void Fault(Exception e)
    {
        if (!_faults.ShouldShow(Environment.TickCount64)) return;
        _window.DispatcherQueue.Enqueue(() =>
        {
            try { ShowFault(e.Message); }
            catch (Exception shown) { AppLog.Write("fault screen failed: " + shown); }
        });
    }

    private void ShowFault(string detail)
    {
        _faultHost.Children.Clear();
        _faultHost.Background = new SolidColorBrush(Windows.UI.Color.FromArgb(217, 0x0D, 0x0F, 0x14));
        var box = new StackPanel { Spacing = 10, MaxWidth = 520, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        var title = F.Text("Something Went Wrong", Theme.FontLargeTitle, F.Bold);
        title.HorizontalAlignment = HorizontalAlignment.Center;
        box.Children.Add(title);
        var text = F.Text("Moonrise hit an unexpected error. " + detail, Theme.FontBody, fg: Theme.Secondary, maxLines: 4);
        text.TextAlignment = TextAlignment.Center;
        box.Children.Add(text);
        var log = F.Text("Details are in " + AppLog.FilePath, Theme.FontCaption, fg: Theme.Secondary, maxLines: 2);
        log.TextAlignment = TextAlignment.Center;
        box.Children.Add(log);
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 8, 0, 0) };
        row.Children.Add(F.Bare(F.Capsule(FaultLabel("Reload", Theme.Black), Theme.White, 40, 18), ReloadAfterFault));
        row.Children.Add(F.Bare(F.Capsule(FaultLabel("Dismiss", Theme.White), Theme.SurfaceStrong, 40, 18), DismissFault));
        box.Children.Add(row);
        _faultHost.Children.Add(box);
        _faultHost.Visibility = Visibility.Visible;
    }

    private static TextBlock FaultLabel(string text, Brush color)
    {
        var t = F.Text(text, Theme.FontHeadline, F.SemiBold, color);
        t.VerticalAlignment = VerticalAlignment.Center;
        return t;
    }

    private void DismissFault()
    {
        _faultHost.Visibility = Visibility.Collapsed;
        _faultHost.Children.Clear();
    }

    private void ReloadAfterFault()
    {
        DismissFault();
        if (_player != null) ClosePlayer();
        if (Session.Api == null) { ShowSignIn(); return; }
        _stacks[_tab].Clear();
        Select(_tab);
    }

    public bool IsFullScreen => _window.AppWin.Presenter.Kind == AppWindowPresenterKind.FullScreen;

    public void ToggleFullScreen()
    {
        if (IsFullScreen) LeaveFullScreen();
        else
        {
            var win = _window.AppWin;
            _beforeFull = win.Presenter.Kind;
            // Left to itself the presenter can come back to where the window started, not where it was moved to;
            // the placement also keeps a maximized window's own place to restore to.
            var placement = Native.WindowPlacement.Empty;
            _windowBeforeFull = Native.GetWindowPlacement(_window.Hwnd, ref placement) ? placement : null;
            win.SetPresenter(AppWindowPresenterKind.FullScreen);
            _window.Activate();
        }
        LayoutPlayer();
        _window.DispatcherQueue.Enqueue(UpdatePassthrough);
    }

    public void LeaveFullScreen()
    {
        if (!IsFullScreen) return;
        _window.AppWin.SetPresenter(_beforeFull == AppWindowPresenterKind.FullScreen ? AppWindowPresenterKind.Overlapped : _beforeFull);
        if (_windowBeforeFull is { } placement) Native.SetWindowPlacement(_window.Hwnd, ref placement);
        _windowBeforeFull = null;
        _window.Activate(); // the presenter change takes keyboard focus away from the window's content
        LayoutPlayer();
        _window.DispatcherQueue.Enqueue(UpdatePassthrough);
    }

    private void AccountMenu()
    {
        var menu = new MenuFlyout();
        menu.Items.Add(new MenuFlyoutItem { Text = "Signed in as " + (Session.UserName ?? ""), IsEnabled = false });
        var settings = new MenuFlyoutItem { Text = "Settings", Icon = new FontIcon { Glyph = "" } };
        settings.Click += (_, _) => OpenSettings();
        menu.Items.Add(settings);
        menu.Items.Add(new MenuFlyoutSeparator());
        var signOut = new MenuFlyoutItem { Text = "Sign Out" };
        signOut.Click += (_, _) => Safe.Fire(async () =>
        {
            try { await Session.SignOut(); }
            catch (Exception e) { AppLog.Write("sign out: " + e.Message); }
            ShowSignIn();
        });
        menu.Items.Add(signOut);
        menu.ShowAt(_account);
    }
}

public interface IRefreshable { void Refresh(); }

/// <summary>A page laid out from the window's size: lays itself out again when that has changed.</summary>
public interface IReflowable { void Reflow(); }

/// <summary>A page with a main control (Search's box) that has the keyboard when nothing else on it does.</summary>
public interface IFocusable { void TakeFocus(); }
