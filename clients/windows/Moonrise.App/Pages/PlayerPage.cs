using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using System.Runtime.InteropServices.WindowsRuntime;
using Moonrise.App.Player;
using Moonrise.App.Ui;
using Moonrise.Core;
using VirtualKey = Windows.System.VirtualKey;
using Windows.UI;

namespace Moonrise.App.Pages;

/// <summary>
/// The player: libmpv video with Moonrise's own controls drawn over it (Mac layout). It plays on
/// its title page, or full screen (F, the button, a double-click); Esc leaves full screen, then the player.
/// </summary>
public sealed class PlayerPage : Grid
{
    private readonly Shell _shell;
    private readonly MoonriseApi _api;
    private readonly bool _noReport = Environment.GetEnvironmentVariable("MOONRISE_NO_REPORT") == "1";

    private readonly SwapChainPanel _panel = new();
    // A real focus target: a panel doesn't keep focus through a click, a control does.
    private readonly ContentControl _keySink = new() { IsTabStop = true, Width = 1, Height = 1, Opacity = 0, IsHitTestVisible = false };
    private readonly Grid _chrome = new();
    private readonly TextBlock _titleText = F.Text("", Theme.FontHeadline, F.SemiBold);
    private readonly TextBlock _subtitleText = F.Text("", Theme.FontCaption, fg: new SolidColorBrush(Color.FromArgb(178, 255, 255, 255)));
    private Grid _top = null!;
    private FrameworkElement _closeCircle = null!;
    private FrameworkElement _toolbarCapsule = null!;
    private StackPanel _corner = null!;
    private readonly StackPanel _toolbar = new() { Orientation = Orientation.Horizontal, Spacing = 4 };
    private Button _episodesButton = null!;
    private Button _qualityButton = null!;
    private Button _audioButton = null!;
    private Button _subtitleButton = null!;
    private Button _fillButton = null!;
    private readonly TextBlock _volumeGlyph = F.Glyph("\uE767", 14);
    private FrameworkElement _scrubCapsule = null!;
    private readonly TextBlock _fillGlyph = F.Glyph("\uE9A6", 16);
    private readonly TextBlock _fullGlyph = F.Glyph("\uE740", 13);
    private StackPanel _titles = null!;
    private bool _fullScreen;
    private bool _pointerInside;
    private readonly TextBlock _playGlyph = F.Glyph("\uF8AE", 28);
    private readonly TextBlock _elapsed = Mono();
    private readonly TextBlock _remaining = Mono();
    private readonly Grid _scrubber = new() { Height = 28 };
    private readonly Border _trackBg = new() { Height = 5, CornerRadius = new CornerRadius(3), Background = new SolidColorBrush(Color.FromArgb(64, 255, 255, 255)), VerticalAlignment = VerticalAlignment.Center };
    private readonly Border _trackFill = new() { Height = 5, CornerRadius = new CornerRadius(3), Background = Theme.White, HorizontalAlignment = HorizontalAlignment.Left, VerticalAlignment = VerticalAlignment.Center, Width = 0 };
    private readonly Border _knob = new() { Width = 14, Height = 14, CornerRadius = new CornerRadius(7), Background = Theme.White, HorizontalAlignment = HorizontalAlignment.Left, VerticalAlignment = VerticalAlignment.Center };
    private Button _skipButton = null!;
    private readonly TextBlock _skipText = F.Text("", Theme.FontHeadline, F.SemiBold, Theme.Black);
    private readonly Border _skipFill = new() { HorizontalAlignment = HorizontalAlignment.Left, CornerRadius = new CornerRadius(20), Background = new SolidColorBrush(Color.FromArgb(89, 255, 255, 255)) };
    private Button _nextButton = null!;
    private readonly ProgressRing _buffering = new() { IsActive = false, Width = 64, Height = 64, Foreground = Theme.White, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
    private Border _volumeHud = null!;
    private readonly Border _volumeFill = new() { Height = 5, CornerRadius = new CornerRadius(3), Background = Theme.White, HorizontalAlignment = HorizontalAlignment.Left };
    private readonly StackPanel _errorBox = new() { Spacing = 10, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, Visibility = Visibility.Collapsed };
    private readonly TextBlock _errorText = F.Text("", Theme.FontBody);
    // A problem the server (or the network) answered with: its title, and its own words behind Details.
    private readonly TextBlock _errorTitle = F.Text("", Theme.FontTitle3, F.SemiBold);
    private readonly TextBlock _errorDetail = F.Text("", Theme.FontCaption, fg: Theme.Secondary);
    private Button _detailsButton = null!;
    // Subtitles drawn here: the text in white over four black copies a pixel off (its outline), in a box when Background is on.
    private readonly Grid _subtitleLayer = new() { VerticalAlignment = VerticalAlignment.Bottom, IsHitTestVisible = false, Margin = new Thickness(40, 0, 40, 0) };
    private readonly Border _subtitleBox = new() { HorizontalAlignment = HorizontalAlignment.Center, CornerRadius = new CornerRadius(6), Visibility = Visibility.Collapsed };
    private readonly TextBlock[] _subtitleLines = Enumerable.Range(0, 5).Select(_ => new TextBlock
    {
        FontFamily = new FontFamily("Segoe UI Variable Text, Segoe UI"), FontWeight = F.SemiBold, TextAlignment = TextAlignment.Center,
        TextWrapping = TextWrapping.Wrap, HorizontalAlignment = HorizontalAlignment.Center,
    }).ToArray();
    // The picture above the scrubber while dragging (Mac: ScrubPreview).
    private readonly StackPanel _preview = new() { Spacing = 4, HorizontalAlignment = HorizontalAlignment.Left, VerticalAlignment = VerticalAlignment.Top, Visibility = Visibility.Collapsed, IsHitTestVisible = false };
    private readonly Canvas _previewCanvas = new() { Width = ScrubPreview.Width, Background = new SolidColorBrush(Color.FromArgb(153, 0, 0, 0)) };
    private readonly Image _previewImage = new() { Stretch = Stretch.Fill };
    private readonly TextBlock _previewTime = F.Text("", Theme.FontCaption, F.SemiBold);
    private Border _previewFrame = null!;
    // The Episodes sheet (Mac: EpisodePickerView), over the player.
    private readonly Grid _episodeSheet = new() { Visibility = Visibility.Collapsed, Background = new SolidColorBrush(Color.FromArgb(110, 0, 0, 0)) };
    private readonly StackPanel _episodeList = new() { Spacing = 2, Padding = new Thickness(0, 0, 0, 16) };
    private readonly ScrollViewer _episodeScroll = new() { HorizontalScrollMode = ScrollMode.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
    private readonly TextBlock _episodeSheetTitle = F.Text("Episodes", Theme.FontHeadline, F.SemiBold);

    private readonly PlayerSettings _prefs = Prefs.Current;

    private MpvSurface? _mpv;
    private Item _item;
    private string? _versionId;
    private double? _startAt;
    private readonly IReadOnlyList<Item>? _episodesIn;
    private IReadOnlyList<Item> _episodes = [];
    private Item? _nextEpisode;

    private Playback? _playback;
    private string? _session;
    private List<MpvTrack> _mpvAudio = [];
    private List<MpvTrack> _mpvSubs = [];
    private List<AudioOption> _audioOptions = [];
    private List<SubOption> _subOptions = [];
    private int? _selectedAudio;
    private SubOption? _selectedSub;
    private bool _tracksApplied;
    /// <summary>The stream restarts where it is (quality, version, burned-in subtitles): the tracks picked stay.</summary>
    private bool _restartInPlace;
    private QualityOption? _quality;

    private SubtitleDocument? _subtitleDoc;
    private SubtitleWindowLoader? _windowLoader;
    private int _subtitleGeneration;
    private double _subtitleDelay;
    private string? _subtitleShown;
    private IReadOnlyList<SubtitleTrack>? _serverSubtitles;
    private string? _serverSubtitlesFor;
    private (string Media, int Index, double Nudge)? _pendingNudge;

    private Thumbnails? _thumbs;
    private string? _thumbsFor;
    private long _thumbsAskedAt;
    private readonly Dictionary<int, BitmapImage> _sheets = [];
    private readonly HashSet<int> _sheetLoads = [];

    private double _position;
    private double _duration;
    private bool _paused;
    private bool _dragging;
    private double _dragTime;
    private bool _fill;
    private bool _muted;
    private double _volume = 100;
    private bool _menuOpen;
    private bool _swallowSpaceUp;

    private bool _controlsShown = true;
    private readonly DispatcherQueueTimer _hideTimer;
    private readonly DispatcherQueueTimer _reportTimer;
    private readonly DispatcherQueueTimer _volumeTimer;
    private readonly DispatcherQueueTimer _skipCountdown;
    private readonly DispatcherQueueTimer _refocusTimer;
    private readonly DispatcherQueueTimer _inputGuardTimer;
    private readonly DispatcherQueueTimer _nudgeTimer;
    private readonly List<Action> _releaseTimers = [];
    private bool _inputGuarded = true; // until a moment after the player has opened
    private bool _cursorHidden;
    private bool _reportStarted;
    private double _lastPointerX, _lastPointerY;

    private IReadOnlyList<Segment> _segments = [];
    private bool _segmentsFinal;
    private double _segmentsAskedAt;
    private int _nearEndAsked;
    private Segment? _active;
    private double _autoSkipLeft;
    private double _autoSkipTotal = 3;
    private string? _skippedSegment;
    private bool _serverPending;

    private bool _stopped;
    private int _recoveries;
    private bool _finishing;
    // The display stays awake only while the video plays.
    private readonly DisplayRequest _awake = new("Moonrise playing video");

    private sealed record AudioOption(int Index, string Title, string? Detail, string? Language, string Name, bool IsDefault);
    private sealed record SubOption(string Id, int Index, string Title, string? Language, bool BurnIn);

    public PlayerPage(Shell shell, Item item, double? start, string? version, IReadOnlyList<Item>? episodes, bool startFromBeginning)
    {
        _shell = shell;
        _api = shell.Api;
        _item = item;
        _versionId = version;
        _startAt = startFromBeginning ? null : start;
        _episodesIn = episodes;

        Background = Theme.Black;
        Children.Add(_panel);
        Children.Add(_keySink);
        BuildOverlays();
        BuildChrome();
        Children.Add(_chrome);
        // Over the controls' shading (it would dim them), and kept clear of the controls themselves.
        Children.Add(_subtitleLayer);
        Children.Add(_corner);
        Children.Add(_buffering);
        BuildError();
        Children.Add(_errorBox);
        Children.Add(_episodeSheet);

        _hideTimer = NewTimer(TimeSpan.FromSeconds(4), repeating: true, () => { if (!_paused && !_dragging) ShowControls(false); });
        _reportTimer = NewTimer(TimeSpan.FromSeconds(10), repeating: true, () => _ = Report(ReportEvent.Progress));
        _volumeTimer = NewTimer(TimeSpan.FromSeconds(1), repeating: false, () => _volumeHud.Opacity = 0);
        _skipCountdown = NewTimer(TimeSpan.FromMilliseconds(100), repeating: true, () => TickAutoSkip());
        _refocusTimer = NewTimer(TimeSpan.FromMilliseconds(250), repeating: false, () => TakeKeyboard());
        _inputGuardTimer = NewTimer(TimeSpan.FromMilliseconds(ClientRules.PlayerOpenInputGuardMs), repeating: false, EndInputGuard);
        // Several Delay presses in a row are one change on the server.
        _nudgeTimer = NewTimer(TimeSpan.FromMilliseconds(600), repeating: false, () => _ = SendNudge());
        SizeChanged += (_, _) => PlaceSubtitles();

        Loaded += OnLoaded;
        Unloaded += (_, _) => Stop();
        PointerMoved += OnPointerMoved;
        // On its page the pointer goes over the page too: the cursor is hidden only over the video.
        PointerEntered += (_, _) => _pointerInside = true;
        PointerExited += (_, _) => { _pointerInside = false; if (_cursorHidden) { _cursorHidden = false; Native.ShowCursor(true); } };
        Tapped += OnTapped;
        DoubleTapped += OnDoubleTapped;
        PointerWheelChanged += OnWheel;
        // Preview: the page acts first, so Space never also presses the control that has focus.
        PreviewKeyDown += OnKey;
        PreviewKeyUp += OnKeyUp;
        // Any click, handled or not (picture, scrubber, a control), leaves the keyboard with the player.
        AddHandler(PointerReleasedEvent, new PointerEventHandler((_, _) => TakeKeyboard()), true);
    }

    /// <summary>
    /// A timer's Tick holds this page from the native side, where the garbage collector can't see
    /// the cycle: each page and its video would stay alive for good unless Stop unhooks them.
    /// </summary>
    private DispatcherQueueTimer NewTimer(TimeSpan interval, bool repeating, Action tick)
    {
        var timer = DispatcherQueue.CreateTimer();
        timer.Interval = interval;
        timer.IsRepeating = repeating;
        _releaseTimers.Add(Safe.Hook(timer, tick));
        return timer;
    }

    private static TextBlock Mono() => new()
    {
        FontFamily = new FontFamily("Cascadia Mono, Consolas"), FontSize = 12, Foreground = Theme.White, VerticalAlignment = VerticalAlignment.Center, Text = "0:00",
    };

    // ---------------------------------------------------------------- layout

    private Button ToolCell(TextBlock glyph, Action action, string tip) => F.Bare(new Border { Width = 44, Height = 44, Child = glyph }, action, tip);

    private void BuildChrome()
    {
        _chrome.Background = Theme.Gradient(0.5, 0, 0.5, 1, (0, Color.FromArgb(115, 0, 0, 0)), (0.25, Color.FromArgb(0, 0, 0, 0)), (0.7, Color.FromArgb(0, 0, 0, 0)), (1, Color.FromArgb(128, 0, 0, 0)));

        // Top: back, title (Mac: full screen only), toolbar capsule.
        _top = new Grid { VerticalAlignment = VerticalAlignment.Top };
        var top = _top;
        top.ColumnDefinitions.Add(F.Auto);
        top.ColumnDefinitions.Add(F.Star());
        top.ColumnDefinitions.Add(F.Auto);
        _closeCircle = F.GlassCircle("\uE72B", 44, 16, Close, "Close");
        top.Children.Add(_closeCircle.At(0, 0));
        var titles = new StackPanel { Margin = new Thickness(14, 0, 0, 0), VerticalAlignment = VerticalAlignment.Center };
        titles.Children.Add(_titleText);
        titles.Children.Add(_subtitleText);
        _titles = titles;
        top.Children.Add(titles.At(0, 1));

        // Mac order: Episodes, Quality, Audio, Subtitles, Fill.
        _episodesButton = ToolCell(F.Glyph("\uE8FD", 16), OpenEpisodes, "Episodes");
        _episodesButton.Visibility = Visibility.Collapsed;
        _qualityButton = ToolCell(F.Glyph("\uE9E9", 16), QualityMenu, "Quality");
        _audioButton = ToolCell(F.Glyph("\uE8D6", 16), AudioMenu, "Audio");
        _subtitleButton = ToolCell(F.Glyph("\uED1E", 16), SubtitleMenu, "Subtitles");
        _fillButton = ToolCell(_fillGlyph, ToggleFill, "Fill");
        _toolbar.Children.Add(_episodesButton);
        _toolbar.Children.Add(_qualityButton);
        _toolbar.Children.Add(_audioButton);
        _toolbar.Children.Add(_subtitleButton);
        _toolbar.Children.Add(_fillButton);
        _toolbarCapsule = new Border { Background = Theme.Glass, BorderBrush = Theme.GlassBorder, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(28), Padding = new Thickness(6, 0, 6, 0), Child = _toolbar };
        top.Children.Add(_toolbarCapsule.At(0, 2));
        _chrome.Children.Add(top);
        _chrome.IsHitTestVisible = false; // until the input guard ends
        // The row between the close button and the toolbar moves the window; these two are clicks.
        top.SizeChanged += (_, _) => _shell.TitleRowChanged();
        _toolbarCapsule.SizeChanged += (_, _) => _shell.TitleRowChanged();

        // Centre: skip back, play/pause, skip forward (Mac: gap 40; 59.8 and 80.5 circles).
        var transport = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 40, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        transport.Children.Add(F.Bare(SkipCircle(-1), () => SkipBy(-10), "Back 10 seconds"));
        var playBorder = new Border { Width = 80, Height = 80, CornerRadius = new CornerRadius(40), Background = Theme.Glass, BorderBrush = Theme.GlassBorder, BorderThickness = new Thickness(1), Child = _playGlyph };
        transport.Children.Add(F.Bare(playBorder, TogglePause, "Play/Pause"));
        transport.Children.Add(F.Bare(SkipCircle(1), () => SkipBy(10), "Forward 10 seconds"));
        _chrome.Children.Add(transport);

        // Bottom: scrubber capsule, then Skip and Next at the end of the row.
        var bottomRow = new Grid { VerticalAlignment = VerticalAlignment.Bottom, Margin = new Thickness(20, 0, 20, 14) };
        bottomRow.ColumnDefinitions.Add(F.Star());

        _scrubber.Children.Add(_trackBg);
        _scrubber.Children.Add(_trackFill);
        _scrubber.Children.Add(_knob);
        _scrubber.Background = Theme.Transparent;
        _scrubber.PointerPressed += ScrubDown;
        _scrubber.PointerMoved += ScrubMove;
        _scrubber.PointerReleased += ScrubUp;
        _scrubber.PointerCanceled += ScrubUp;
        var capsuleGrid = new Grid { VerticalAlignment = VerticalAlignment.Center };
        capsuleGrid.ColumnDefinitions.Add(F.Auto);
        capsuleGrid.ColumnDefinitions.Add(F.Star());
        capsuleGrid.ColumnDefinitions.Add(F.Auto);
        capsuleGrid.ColumnDefinitions.Add(F.Auto);
        capsuleGrid.ColumnSpacing = 14;
        capsuleGrid.Children.Add(_elapsed.At(0, 0));
        capsuleGrid.Children.Add(_scrubber.At(0, 1));
        capsuleGrid.Children.Add(_remaining.At(0, 2));
        var fs = F.Bare(new Border { Width = 28, Height = 28, Child = _fullGlyph }, ToggleFull, "Full Screen (F)");
        capsuleGrid.Children.Add(fs.At(0, 3));
        var capsule = new Border
        {
            Background = Theme.Glass, BorderBrush = Theme.GlassBorder, BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(20), Padding = new Thickness(18, 6, 12, 6), Child = capsuleGrid,
        };
        _scrubCapsule = capsule;
        bottomRow.Children.Add(capsule.At(0, 0));
        capsule.SizeChanged += (_, _) => PlaceSubtitles();

        // Skip and Next: on their own layer, so a Skip prompt can show while the controls are hidden.
        var corner = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10, HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Bottom, Margin = new Thickness(0, 0, 20, 14) };
        var nextText = F.Text("Next Episode", Theme.FontHeadline, F.SemiBold, Theme.Black);
        var nextBorder = F.Capsule(nextText, Theme.White, 40, 18);
        nextText.VerticalAlignment = VerticalAlignment.Center;
        _nextButton = F.Bare(nextBorder, PlayNext, "Next Episode");
        _nextButton.Visibility = Visibility.Collapsed;
        corner.Children.Add(_nextButton);
        var skipGrid = new Grid();
        skipGrid.Children.Add(_skipFill);
        _skipText.Padding = new Thickness(18, 0, 18, 0);
        _skipText.VerticalAlignment = VerticalAlignment.Center;
        skipGrid.Children.Add(_skipText);
        var skipBorder = new Border { Height = 40, CornerRadius = new CornerRadius(20), Background = Theme.White, Child = skipGrid };
        _skipButton = F.Bare(skipBorder, SkipSegment, "Skip");
        _skipButton.Visibility = Visibility.Collapsed;
        corner.Children.Add(_skipButton);
        _chrome.Children.Add(bottomRow);
        corner.SizeChanged += (_, _) => { var cw = corner.ActualWidth; bottomRow.Margin = new Thickness(20, 0, cw > 1 ? 20 + cw + 10 : 20, 14); };
        _corner = corner;

        // Volume HUD, top centre.
        _volumeHud = new Border
        {
            Width = 180, Height = 44, CornerRadius = new CornerRadius(22), Background = Theme.Glass, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Top,
            Margin = new Thickness(0, 76, 0, 0), Opacity = 0, IsHitTestVisible = false,
            OpacityTransition = new ScalarTransition { Duration = TimeSpan.FromMilliseconds(150) },
        };
        var volRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        volRow.Children.Add(_volumeGlyph);
        var bar = new Grid { Width = 110, Height = 5 };
        bar.Children.Add(new Border { CornerRadius = new CornerRadius(3), Background = new SolidColorBrush(Color.FromArgb(64, 255, 255, 255)) });
        bar.Children.Add(_volumeFill);
        volRow.Children.Add(bar);
        _volumeHud.Child = volRow;
        _chrome.Children.Add(_volumeHud);
        _chrome.Children.Add(_preview);
    }

    /// <summary>The subtitles, the scrubbing picture and the Episodes sheet: built once, filled as they're used.</summary>
    private void BuildOverlays()
    {
        var lines = new Grid();
        var offsets = new (double X, double Y)[] { (-1.5, 0), (1.5, 0), (0, -1.5), (0, 1.5) };
        for (var i = 0; i < 4; i++)
        {
            _subtitleLines[i].Foreground = Theme.Black;
            _subtitleLines[i].RenderTransform = new TranslateTransform { X = offsets[i].X, Y = offsets[i].Y };
            lines.Children.Add(_subtitleLines[i]);
        }
        _subtitleLines[4].Foreground = Theme.White;
        lines.Children.Add(_subtitleLines[4]);
        _subtitleBox.Child = lines;
        _subtitleLayer.Children.Add(_subtitleBox);

        _previewCanvas.Children.Add(_previewImage);
        _previewFrame = new Border
        {
            CornerRadius = new CornerRadius(8), BorderBrush = new SolidColorBrush(Color.FromArgb(217, 255, 255, 255)), BorderThickness = new Thickness(1.5),
            Child = _previewCanvas, HorizontalAlignment = HorizontalAlignment.Center,
        };
        _preview.Children.Add(_previewFrame);
        _preview.Children.Add(new Border
        {
            HorizontalAlignment = HorizontalAlignment.Center, CornerRadius = new CornerRadius(10), Padding = new Thickness(8, 2, 8, 2),
            Background = new SolidColorBrush(Color.FromArgb(128, 0, 0, 0)), Child = _previewTime,
        });

        // A sheet in the middle (Mac: a sheet with the show's name and Done), the list scrolling inside it.
        var card = new Grid
        {
            Width = 560, MaxHeight = 720, Margin = new Thickness(20, 70, 20, 40), HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center,
            Background = new SolidColorBrush(Color.FromArgb(245, 0x1C, 0x1E, 0x24)), CornerRadius = new CornerRadius(16),
            BorderBrush = Theme.GlassBorder, BorderThickness = new Thickness(1),
        };
        card.RowDefinitions.Add(F.RowAuto);
        card.RowDefinitions.Add(F.RowStar);
        var head = new Grid { Height = 52, Padding = new Thickness(20, 0, 12, 0) };
        _episodeSheetTitle.VerticalAlignment = VerticalAlignment.Center;
        _episodeSheetTitle.HorizontalAlignment = HorizontalAlignment.Center;
        _episodeSheetTitle.TextTrimming = TextTrimming.CharacterEllipsis;
        _episodeSheetTitle.Margin = new Thickness(70, 0, 70, 0);
        head.Children.Add(_episodeSheetTitle);
        var doneText = F.Text("Done", Theme.FontBody, F.SemiBold);
        doneText.VerticalAlignment = VerticalAlignment.Center;
        var done = F.Bare(F.Capsule(doneText, Theme.SurfaceStrong, 32, 14), CloseEpisodes, "Done");
        done.HorizontalAlignment = HorizontalAlignment.Right;
        done.VerticalAlignment = VerticalAlignment.Center;
        head.Children.Add(done);
        card.Children.Add(head.At(0, 0));
        _episodeScroll.Content = _episodeList;
        card.Children.Add(_episodeScroll.At(1, 0));
        _episodeSheet.Children.Add(card);
        // A click beside the sheet closes it, as a click outside a sheet's card.
        _episodeSheet.Tapped += (_, e) => { if (ReferenceEquals(e.OriginalSource, _episodeSheet)) CloseEpisodes(); e.Handled = true; };
        _episodeSheet.DoubleTapped += (_, e) => e.Handled = true;
    }

    private static FrameworkElement SkipCircle(int dir)
    {
        // A circular arrow with the seconds inside (Mac: gobackward.10 / goforward.10).
        var arrow = F.Glyph("\uE72C", 28);
        if (dir < 0) { arrow.RenderTransformOrigin = new Windows.Foundation.Point(0.5, 0.5); arrow.RenderTransform = new ScaleTransform { ScaleX = -1 }; }
        var g = new Grid();
        g.Children.Add(arrow);
        var num = new TextBlock { Text = "10", FontSize = 11, FontWeight = F.SemiBold, Foreground = Theme.White, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(0, 1, 0, 0) };
        g.Children.Add(num);
        return new Border { Width = 60, Height = 60, CornerRadius = new CornerRadius(30), Background = Theme.Glass, BorderBrush = Theme.GlassBorder, BorderThickness = new Thickness(1), Child = g };
    }

    private void BuildError()
    {
        _errorText.TextWrapping = TextWrapping.Wrap;
        _errorText.MaxWidth = 500;
        _errorText.HorizontalAlignment = HorizontalAlignment.Center;
        _errorText.TextAlignment = TextAlignment.Center;
        _errorTitle.HorizontalAlignment = HorizontalAlignment.Center;
        _errorTitle.Visibility = Visibility.Collapsed;
        _errorDetail.TextWrapping = TextWrapping.Wrap;
        _errorDetail.MaxWidth = 500;
        _errorDetail.IsTextSelectionEnabled = true;
        _errorDetail.Visibility = Visibility.Collapsed;
        _errorBox.Children.Add(_errorTitle);
        _errorBox.Children.Add(_errorText);
        _errorBox.Children.Add(_errorDetail);
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10, HorizontalAlignment = HorizontalAlignment.Center };
        _errorBox.Background = Theme.Glass;
        _errorBox.CornerRadius = new CornerRadius(16);
        _errorBox.Padding = new Thickness(24, 18, 24, 18);
        static TextBlock Label(string text, Brush color)
        {
            var t = F.Text(text, Theme.FontHeadline, F.SemiBold, color);
            t.VerticalAlignment = VerticalAlignment.Center;
            return t;
        }
        row.Children.Add(F.Bare(F.Capsule(Label("Try Again", Theme.Black), Theme.White, 40, 18), () => { _errorBox.Visibility = Visibility.Collapsed; UpdateAwake(); _recoveries = 0; Start(_position > 1 ? _position : _startAt); }));
        row.Children.Add(F.Bare(F.Capsule(Label("Close", Theme.White), Theme.SurfaceStrong, 40, 18), Close));
        _detailsButton = F.Bare(F.Capsule(Label("Details", Theme.White), Theme.SurfaceStrong, 40, 18), () =>
            _errorDetail.Visibility = _errorDetail.Visibility == Visibility.Visible ? Visibility.Collapsed : Visibility.Visible);
        row.Children.Add(_detailsButton);
        _errorBox.Children.Add(row);
    }

    // ---------------------------------------------------------------- playback lifecycle

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        try
        {
            _mpv = new MpvSurface(_panel, DispatcherQueue);
        }
        catch (Exception ex)
        {
            ShowError("The player couldn't start: " + ex.Message);
            _inputGuardTimer.Start();
            return;
        }
        _mpv.TimeChanged += OnTime;
        _mpv.DurationChanged += d => { _duration = d; UpdateTimes(); };
        _mpv.PausedChanged += OnPaused;
        _mpv.BufferingChanged += b => _buffering.IsActive = b && !_paused;
        _mpv.TracksChanged += OnTracks;
        _mpv.Ended += OnEnded;
        _mpv.VolumeChanged += v => { _volume = v; UpdateVolumeHud(); };
        _mpv.MuteChanged += m => { _muted = m; UpdateVolumeHud(); };
        StyleSubtitles();
        ShowControls(true);
        UpdateFull();
        _ = LoadEpisodes();
        Start(_startAt);
        UpdateAwake();
        TakeKeyboard();
        _inputGuardTimer.Start(); // from here: a click made while this ran arrives once it has
    }

    private void EndInputGuard()
    {
        _inputGuarded = false;
        _chrome.IsHitTestVisible = _controlsShown;
    }

    private async Task LoadEpisodes()
    {
        if (!_item.IsEpisode || _item.SeriesId == null) return;
        try
        {
            _episodes = _episodesIn ?? await _api.Episodes(_item.SeriesId);
            _nextEpisode = NextEpisodeRules.Next(_episodes, _item);
            _episodesButton.Visibility = _episodes.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        }
        catch { }
    }

    private void Start(double? at)
    {
        _ = StartAsync(ClientRules.StartPosition(at));
    }

    private async Task StartAsync(double? at)
    {
        _tracksApplied = false;
        _buffering.IsActive = true;
        _titleText.Text = _item.CardTitle;
        _subtitleText.Text = _item.IsEpisode ? _item.CardSubtitle : "";
        try
        {
            var full = await _api.Item(_item.Id);
            _item = full;
            _versionId ??= Versions.Chosen(full, null, true, null)?.Id;
        }
        catch { /* the playback answer still works */ }
        _quality ??= QualityChoice.Starting(_prefs.StreamingQuality, VersionFile());

        Playback? p = null;
        var hdr = true; // ask for the HDR original: the GPU tone-maps it to the display, and the server just copies.
        var waited = 0;
        while (!_stopped)
        {
            try
            {
                p = await _api.PlaybackInfo(_item.Id, at, hdr, _versionId, burn: _burn, quality: QualityChoice.Request(_quality));
            }
            catch (Exception ex)
            {
                ShowProblem(PlaybackProblem.Of(ex));
                return;
            }
            if (p.Pending != true || waited > 90) break;
            await Task.Delay(2000);
            waited += 2;
        }
        if (_stopped || p == null) return;
        if (!p.Supported)
        {
            ShowProblem(PlaybackProblem.Of(p));
            return;
        }
        _playback = p;
        ShowTrackButtons();
        _session = p.Session;
        _reportStarted = false;
        _segments = [];
        _segmentsFinal = false;
        _skippedSegment = null;
        _ = LoadSegments(provisional: true);
        _ = LoadThumbnails();
        _mpv?.Load(_api.MasterUrl(p)!, at);
    }

    /// <summary>The file playing: the title's id, with the version when one was picked (the subtitle and thumbnail routes take it).</summary>
    private string MediaId => _api.MediaId(_item.Id, _versionId);

    /// <summary>The stream again from where it is, with what changed (quality, version, burned-in subtitles); the tracks picked stay.</summary>
    private void RestartInPlace()
    {
        _restartInPlace = true;
        _ = CloseCurrent(false);
        Start(_position > 1 ? _position : _startAt);
    }

    private int? _burn;

    private void OnTime(double t)
    {
        _position = t;
        UpdateTimes();
        _windowLoader?.Advance(t - _subtitleDelay);
        UpdateSubtitle(t);
        if (_session != null && !_reportStarted && t > 0.5) _ = BeginReporting();
        CheckSegments(t);
    }

    private void OnPaused(bool paused)
    {
        _paused = paused;
        UpdateAwake();
        _playGlyph.Text = paused ? "\uF5B0" : "\uF8AE";
        if (paused) { ShowControls(true); _hideTimer.Stop(); }
        else { ScheduleHide(); }
        if (_session != null && _reportStarted) _ = Report(ReportEvent.Progress);
    }

    private async Task BeginReporting()
    {
        _reportStarted = true;
        _reportTimer.Start();
        await Report(ReportEvent.Start);
    }

    private async Task Report(ReportEvent ev)
    {
        if (_noReport || _session == null) return;
        try
        {
            await _api.Report(_session, ev, _position, _paused, audio: _selectedAudio, credits: SegmentRules.CreditsStart(_segments), tracks: TracksPlaying());
        }
        catch (MoonriseHttpException e) when (e.Code is 404 or 410) { Recover("session gone"); }
        catch { /* transient: the next report covers it */ }
    }

    private void OnEnded(string reason)
    {
        if (_stopped || _finishing) return;
        AppLog.Write("end-file: " + reason);
        if (reason == "error") { Recover("stream error"); return; }
        if (reason != "eof") return;
        if (!ClientRules.StreamEndIsTheEnd(_position, _duration)) { Recover("stream stopped early"); return; }
        _ = Finish();
    }

    private void Recover(string why)
    {
        if (_stopped) return;
        if (_recoveries++ >= 3) { ShowError("The stream stopped playing. " + why); return; }
        AppLog.Write("recover: " + why);
        RestartInPlace(); // the tracks picked stay
    }

    private async Task Finish()
    {
        _finishing = true;
        UpdateAwake();
        await CloseCurrent(report: true);
        if (_nextEpisode != null)
        {
            var next = _nextEpisode;
            _finishing = false;
            UpdateAwake();
            PlayItem(next, next.ResumeSeconds);
            return;
        }
        _finishing = false;
        Close();
    }

    /// <summary>Another episode in this player (the next one, or one from Episodes), with its own tracks and quality.</summary>
    private void PlayItem(Item episode, double? at)
    {
        _item = episode;
        _versionId = null;
        _quality = null;
        _position = 0;
        _duration = 0;
        _nextEpisode = NextEpisodeRules.Next(_episodes, episode);
        _shell.PlayerItemChanged(episode);
        _segments = [];
        _active = null;
        _nearEndAsked = 0;
        _skipCountdown.Stop();
        _skipButton.Visibility = Visibility.Collapsed;
        _nextButton.Visibility = Visibility.Collapsed;
        _restartInPlace = false;
        _selectedSub = null;
        _selectedAudio = null;
        LoadSubtitle(null);
        _thumbs = null;
        _thumbsFor = null;
        _mpv?.Pause(false); // an episode picked (or the next one) plays, even from a paused one
        Start(at);
    }

    private async Task CloseCurrent(bool report)
    {
        _reportTimer.Stop();
        _skipCountdown.Stop();
        if (report && _reportStarted) await Report(ReportEvent.Stop);
        if (_session != null) { var s = _session; _session = null; _ = _api.CloseSession(s); }
        _reportStarted = false;
    }

    // ---------------------------------------------------------------- tracks

    private void OnTracks(IReadOnlyList<MpvTrack> tracks)
    {
        _mpvAudio = tracks.Where(t => t.Type == "audio").ToList();
        _mpvSubs = tracks.Where(t => t.Type == "sub").ToList();
        AppLog.Write($"tracks: audio [{string.Join(", ", _mpvAudio.Select(t => t.Id + ":" + t.Lang))}] subs [{string.Join(", ", _mpvSubs.Select(t => t.Id + ":" + t.Lang))}]");
        if (_playback == null || _tracksApplied) return;
        if (_mpvAudio.Count == 0 && _mpvSubs.Count == 0) return;
        _tracksApplied = true;
        ShowTrackButtons();
        // The app draws the subtitles; mpv draws none.
        _mpv?.SetSubtitleTrack(null);
        if (_restartInPlace) KeepTracks();
        else ApplyStartingTracks();
        _restartInPlace = false;
    }

    /// <summary>A restart in place: the same audio, and the subtitles as they were (drawn here, so they never stopped).</summary>
    private void KeepTracks()
    {
        if (_selectedAudio is { } a && _audioOptions.All(o => o.Index != a)) _selectedAudio = null;
        _selectedAudio ??= _audioOptions.FirstOrDefault(o => o.IsDefault)?.Index ?? _audioOptions.FirstOrDefault()?.Index;
        ApplyAudio();
        if (_selectedSub is { } s && _subOptions.All(o => o.Id != s.Id)) SelectSubtitle(null, persist: false);
    }

    /// <summary>The options come from the server's answer, so the buttons are set before mpv lists any track: a silent title lists none.</summary>
    private void ShowTrackButtons()
    {
        BuildAudioOptions();
        BuildSubtitleOptions();
        var (audio, subs) = ClientRules.TrackButtons(_audioOptions.Count, _subOptions.Count);
        _audioButton.Visibility = audio ? Visibility.Visible : Visibility.Collapsed;
        _subtitleButton.Visibility = subs ? Visibility.Visible : Visibility.Collapsed;
    }

    private MediaFile? VersionFile() => _item.PresentFiles.FirstOrDefault(f => f.Id == _versionId) ?? _item.PresentFiles.FirstOrDefault();

    private void BuildAudioOptions()
    {
        var offered = _playback?.Audio ?? [];
        var file = VersionFile();
        var tracks = (file?.Tracks ?? []).Where(t => t.IsAudio).ToList();
        _audioOptions = offered.Select(p =>
        {
            var t = tracks.FirstOrDefault(x => x.Index == p.Index);
            var name = TrackLabels.LanguageName(p.Language) ?? p.Name;
            var detail = t != null ? TrackLabels.AudioDetail(t) : p.Name;
            return new AudioOption(p.Index, t != null ? TrackLabels.AudioName(t) : name, detail, p.Language, t != null ? string.Join(" ", new[] { t.Title, t.DisplayTitle }.Where(x => x != null)) : p.Name, p.Default == true);
        }).ToList();
    }

    private void BuildSubtitleOptions()
    {
        var offered = _playback?.Subtitles ?? [];
        var options = new List<SubOption>();
        foreach (var s in offered)
        {
            var lang = TrackLabels.LanguageName(s.Language) ?? s.Name;
            var parts = new List<string> { lang };
            if (s.Forced == true) parts.Add("Forced");
            if (TrackRules.IsSDH(s.Name)) parts.Add("SDH");
            options.Add(new SubOption($"s{s.Index}", s.Index, string.Join(" · ", parts), s.Language, s.Text == false));
        }
        _subOptions = options;
    }

    private void ApplyStartingTracks()
    {
        var memory = _playback?.Tracks;
        var audioCandidates = _audioOptions.Select(o => new AudioCandidate(o.Index, o.Language, o.IsDefault, o.Name)).ToList();
        _selectedAudio = TrackRules.RememberedAudio(audioCandidates, memory)
            ?? TrackRules.PreferredAudio(audioCandidates, _prefs.PreferredAudioLanguage)
            ?? _audioOptions.FirstOrDefault(o => o.IsDefault)?.Index ?? _audioOptions.FirstOrDefault()?.Index;
        ApplyAudio();

        var candidates = _subOptions.Select(s => new SubtitleCandidate(s.Id, s.Title, s.Language, s.Index, s.BurnIn)).ToList();
        var mem = TrackRules.RememberedSubtitle(candidates, memory);
        SubOption? choice = null;
        // Settings' subtitles: on in the preferred language, else forced ones only; the title's own pick first.
        var on = _prefs.AutoSelectSubtitles && mem is not SubtitleMemory.UsualOff;
        switch (mem)
        {
            case SubtitleMemory.Use u: choice = _subOptions.FirstOrDefault(o => o.Id == u.Id); break;
            case SubtitleMemory.OffHere: break;
            default:
                var audioLang = _audioOptions.FirstOrDefault(o => o.Index == _selectedAudio)?.Language;
                var sc = TrackRules.UsualSubtitle(candidates, on, _prefs.PreferredSubtitleLanguage, _prefs.PreferSdhSubtitles, audioLang);
                choice = sc == null ? null : _subOptions.FirstOrDefault(o => o.Id == sc.Id);
                break;
        }
        SelectSubtitle(choice, persist: false);
    }

    private void ApplyAudio()
    {
        if (_selectedAudio == null || _mpv == null) return;
        var option = _audioOptions.FirstOrDefault(o => o.Index == _selectedAudio);
        if (option == null) return;
        var i = _playback!.Audio.ToList().FindIndex(a => a.Index == option.Index);
        var mpvTrack = i >= 0 && i < _mpvAudio.Count ? _mpvAudio[i]
            : _mpvAudio.FirstOrDefault(t => Languages.Normalized(t.Lang) == Languages.Normalized(option.Language));
        if (mpvTrack != null) _mpv.SetAudioTrack(mpvTrack.Id);
        AppLog.Write($"audio: track {option.Index} ({option.Language ?? "?"}), mpv {mpvTrack?.Id.ToString() ?? "none"}");
    }

    private void ChooseAudio(AudioOption o)
    {
        _selectedAudio = o.Index;
        ApplyAudio();
        _prefs.AudioPicked(o.Language);
        Prefs.Save();
        _ = Report(ReportEvent.Progress);
    }

    /// <summary>
    /// A subtitle to show (null: Off). A text track is drawn here; a bitmap track is burned in by the
    /// server, so the stream restarts where it is. <paramref name="persist"/>: the viewer picked it,
    /// so its language (or Off) is the preference from now on.
    /// </summary>
    private void SelectSubtitle(SubOption? o, bool persist)
    {
        if (persist)
        {
            _prefs.SubtitlePicked(o?.Language, o?.Title);
            Prefs.Save();
        }
        // The same bitmap track again: it's already burned into this stream.
        if (o is { BurnIn: true } && o.Id == _selectedSub?.Id && _burn == o.Index) return;
        var wasBurned = _burn != null;
        _selectedSub = o;
        _burn = o is { BurnIn: true } ? o.Index : null;
        AppLog.Write($"subtitles: {(o == null ? "off" : $"track {o.Index} ({o.Language ?? "?"}){(o.BurnIn ? ", burned in" : "")}")}");
        LoadSubtitle(o);
        if (_burn != null || wasBurned) RestartInPlace();
    }

    private TrackChoice? TracksPlaying()
    {
        if (!_tracksApplied) return null;
        var file = VersionFile();
        TrackPick? audio = null;
        if (_selectedAudio is { } idx)
        {
            var t = (file?.Tracks ?? []).FirstOrDefault(x => x.IsAudio && x.Index == idx);
            var o = _audioOptions.FirstOrDefault(x => x.Index == idx);
            audio = new TrackPick { Index = idx, Language = Languages.Normalized(t?.Language ?? o?.Language), Title = t?.Title ?? t?.DisplayTitle ?? o?.Detail };
        }
        TrackPick? sub = null;
        if (_selectedSub is { } s)
            sub = new TrackPick { Index = s.Index, Language = TrackRules.SubtitleLanguage(s.Language, s.Title), Title = TrackRules.SubtitleDetail(s.Title), Forced = TrackRules.IsForced(s.Title), Sdh = TrackRules.IsSDH(s.Title) };
        return new TrackChoice(audio, sub);
    }

    private void AudioMenu()
    {
        var menu = new MenuFlyout();
        foreach (var o in _audioOptions)
        {
            var it = new ToggleMenuFlyoutItem { Text = o.Detail != null ? $"{o.Title}  ·  {o.Detail}" : o.Title, IsChecked = o.Index == _selectedAudio };
            it.Click += (_, _) => ChooseAudio(o);
            menu.Items.Add(it);
        }
        ShowMenu(menu, _audioButton);
    }

    private void SubtitleMenu()
    {
        var menu = new MenuFlyout();
        var off = new ToggleMenuFlyoutItem { Text = "Off", IsChecked = _selectedSub == null };
        off.Click += (_, _) => { SelectSubtitle(null, persist: true); _ = Report(ReportEvent.Progress); };
        menu.Items.Add(off);
        foreach (var o in _subOptions)
        {
            var it = new ToggleMenuFlyoutItem { Text = o.Title, IsChecked = _selectedSub?.Id == o.Id };
            it.Click += (_, _) => { SelectSubtitle(o, persist: true); _ = Report(ReportEvent.Progress); };
            menu.Items.Add(it);
        }
        menu.Items.Add(new MenuFlyoutSeparator());

        // Style: size and background of the subtitles drawn here (Mac: TrackMenus.style).
        var style = new MenuFlyoutSubItem { Text = "Style", Icon = new FontIcon { Glyph = "\uE8D2" } };
        foreach (var size in Enum.GetValues<SubtitleSize>())
        {
            var it = new RadioMenuFlyoutItem { Text = size.Title(), GroupName = "size", IsChecked = _prefs.SubtitleSize == size };
            it.Click += (_, _) => { _prefs.SubtitleSize = size; Prefs.Save(); StyleSubtitles(); };
            style.Items.Add(it);
        }
        style.Items.Add(new MenuFlyoutSeparator());
        var background = new ToggleMenuFlyoutItem { Text = "Background", IsChecked = _prefs.SubtitleBackground };
        background.Click += (_, _) => { _prefs.SubtitleBackground = !_prefs.SubtitleBackground; Prefs.Save(); StyleSubtitles(); };
        style.Items.Add(background);
        menu.Items.Add(style);

        // Delay: moves the text against the picture (Mac: TrackMenus.delay).
        var delay = new MenuFlyoutSubItem { Text = "Delay  " + SubtitleDelay.Label(_subtitleDelay), Icon = new FontIcon { Glyph = "\uE916" }, IsEnabled = _selectedSub is { BurnIn: false } };
        foreach (var step in SubtitleDelay.Steps)
        {
            var it = new MenuFlyoutItem { Text = SubtitleDelay.Label(step) };
            it.Click += (_, _) => ApplyDelay(SubtitleDelay.Nudged(_subtitleDelay, step));
            delay.Items.Add(it);
        }
        delay.Items.Add(new MenuFlyoutSeparator());
        var reset = new MenuFlyoutItem { Text = "Reset" };
        reset.Click += (_, _) => ApplyDelay(0);
        delay.Items.Add(reset);
        menu.Items.Add(delay);
        ShowMenu(menu, _subtitleButton);
    }

    // ---------------------------------------------------------------- subtitles drawn by the app

    /// <summary>
    /// The text track to show: an embedded one window by window around the playhead when the server
    /// serves windows (then whole, once the server has it all), any other whole.
    /// </summary>
    private void LoadSubtitle(SubOption? o)
    {
        _subtitleGeneration++;
        _windowLoader?.Stop();
        _windowLoader = null;
        _subtitleDoc = null;
        _nudgeTimer.Stop();
        if (_pendingNudge != null) _ = SendNudge();
        ShowSubtitle(null);
        if (o == null || o.BurnIn) return;
        var track = _playback?.Subtitles.FirstOrDefault(s => s.Index == o.Index);
        if (track?.Url == null) return;
        _ = LoadSubtitleAsync(o, track, _subtitleGeneration);
    }

    private async Task LoadSubtitleAsync(SubOption o, PlaybackTrack track, int generation)
    {
        var media = MediaId;
        var url = _api.Resolve(track.Url!);
        await LoadServerSubtitles(media);
        if (generation != _subtitleGeneration) return;
        _subtitleDelay = RestoredDelay(o);
        try
        {
            var info = await _api.Capabilities();
            if (generation != _subtitleGeneration) return;
            if (track.Embedded && info.Has("subtitle-windows"))
            {
                var index = track.Index;
                _windowLoader = new SubtitleWindowLoader(
                    (k, ct) => _api.SubtitleWindow(media, index, k, _session, ct),
                    ct => _api.FetchText(url, ct),
                    cues => { if (generation == _subtitleGeneration) { _subtitleDoc = new SubtitleDocument(cues); UpdateSubtitle(_position); } },
                    () => { if (generation == _subtitleGeneration) { _windowLoader = null; _ = LoadWholeSubtitle(url, generation); } },
                    log: AppLog.Write);
                _windowLoader.Advance(_position - _subtitleDelay);
                return;
            }
        }
        catch (Exception e) { AppLog.Write("subtitles: " + e.Message); }
        await LoadWholeSubtitle(url, generation);
    }

    private async Task LoadWholeSubtitle(string url, int generation)
    {
        try
        {
            var text = await _api.FetchText(url);
            if (generation != _subtitleGeneration) return;
            _subtitleDoc = SubtitleParser.Parse(text, Path.GetExtension(new Uri(url).AbsolutePath).TrimStart('.'));
            AppLog.Write($"subtitles: {_subtitleDoc.Cues.Count} cues from {new Uri(url).AbsolutePath}");
            UpdateSubtitle(_position);
        }
        catch (Exception e) { if (generation == _subtitleGeneration) AppLog.Write("subtitles: couldn't load: " + e.Message); }
    }

    /// <summary>The server's list of the file's subtitle tracks, for their timing (the viewer's delay kept there).</summary>
    private async Task LoadServerSubtitles(string media)
    {
        if (_serverSubtitlesFor == media) return;
        try { _serverSubtitles = await _api.Subtitles(media); _serverSubtitlesFor = media; }
        catch { _serverSubtitles = null; }
    }

    private SubtitleTrack? TimedTrack(SubOption o) => _serverSubtitlesFor == MediaId ? _serverSubtitles?.FirstOrDefault(t => t.Index == o.Index && t.Timing != null) : null;

    /// <summary>The track's own delay: kept by the server (every device), or on this device for a server without timing.</summary>
    private double RestoredDelay(SubOption o) =>
        TimedTrack(o)?.Timing?.Nudge ?? (_prefs.SubtitleNudges.TryGetValue(SubtitleDelay.NudgeKey(MediaId, o.Title), out var d) ? d : 0);

    private void ApplyDelay(double value)
    {
        _subtitleDelay = value;
        UpdateSubtitle(_position);
        _windowLoader?.Advance(_position - _subtitleDelay);
        if (_selectedSub is not { BurnIn: false } o) return;
        AppLog.Write($"subtitle delay {SubtitleDelay.Label(value)} for {o.Title}");
        if (TimedTrack(o) is { } track)
        {
            _pendingNudge = (MediaId, track.Index, value);
            _nudgeTimer.Stop();
            _nudgeTimer.Start();
            return;
        }
        var key = SubtitleDelay.NudgeKey(MediaId, o.Title);
        if (value == 0) _prefs.SubtitleNudges.Remove(key); else _prefs.SubtitleNudges[key] = value;
        Prefs.Save();
    }

    private async Task SendNudge()
    {
        if (_pendingNudge is not { } n) return;
        _pendingNudge = null;
        try
        {
            var timing = await _api.SetSubtitleNudge(n.Media, n.Index, n.Nudge);
            if (_serverSubtitlesFor == n.Media)
                _serverSubtitles = _serverSubtitles?.Select(t => t.Index == n.Index ? t with { Timing = timing } : t).ToList();
        }
        catch (Exception e) { AppLog.Write("subtitle delay: " + e.Message); }
    }

    /// <summary>The cue showing at <paramref name="t"/>, moved by the delay.</summary>
    private void UpdateSubtitle(double t) => ShowSubtitle(_subtitleDoc?.TextAt(t - _subtitleDelay));

    private void ShowSubtitle(string? text)
    {
        if (text == _subtitleShown) return;
        _subtitleShown = text;
        foreach (var line in _subtitleLines) line.Text = text ?? "";
        _subtitleBox.Visibility = string.IsNullOrEmpty(text) ? Visibility.Collapsed : Visibility.Visible;
    }

    /// <summary>Size and background as Settings (or the Style menu) say: Mac's 28 pt semibold, scaled.</summary>
    private void StyleSubtitles()
    {
        var size = SubtitleLayout.BaseSize * _prefs.SubtitleSize.Scale();
        foreach (var line in _subtitleLines) line.FontSize = size;
        var box = _prefs.SubtitleBackground;
        _subtitleBox.Background = box ? new SolidColorBrush(Color.FromArgb(153, 0, 0, 0)) : null;
        _subtitleBox.Padding = box ? new Thickness(10, 4, 10, 4) : new Thickness(0);
        for (var i = 0; i < 4; i++) _subtitleLines[i].Visibility = box ? Visibility.Collapsed : Visibility.Visible;
        PlaceSubtitles();
    }

    /// <summary>6 % up with the controls hidden; above the scrubber while they show (Mac: SubtitleOverlay).</summary>
    private void PlaceSubtitles()
    {
        if (ActualHeight <= 0) return;
        double bandTop = 0;
        if (_scrubCapsule.ActualHeight > 0)
            bandTop = _scrubCapsule.TransformToVisual(this).TransformPoint(new Windows.Foundation.Point(0, 0)).Y;
        var pad = SubtitleLayout.BottomPadding(ActualHeight, _controlsShown, bandTop);
        _subtitleLayer.Margin = new Thickness(40, 0, 40, pad);
    }

    // ---------------------------------------------------------------- episodes

    private void OpenEpisodes()
    {
        if (_episodes.Count == 0) return;
        _episodeSheetTitle.Text = _item.SeriesTitle ?? "Episodes";
        _episodeList.Children.Clear();
        FrameworkElement? current = null;
        foreach (var group in EpisodeGroups.Seasons(_episodes))
        {
            var head = F.Text(group.Title, Theme.FontSubheadline, F.SemiBold, Theme.Secondary);
            head.Margin = new Thickness(20, 14, 20, 6);
            _episodeList.Children.Add(head);
            foreach (var ep in group.Episodes)
            {
                var row = EpisodeRow(ep);
                if (ep.Id == _item.Id) current = row;
                _episodeList.Children.Add(row);
            }
        }
        _episodeSheet.Visibility = Visibility.Visible;
        _hideTimer.Stop();
        ShowControls(true);
        // Opens at the playing episode, once the list is laid out.
        if (current != null)
        {
            var target = current;
            void Scroll(object? s, object e)
            {
                _episodeScroll.LayoutUpdated -= Scroll;
                target.StartBringIntoView(new BringIntoViewOptions { VerticalAlignmentRatio = 0.4, AnimationDesired = false });
            }
            _episodeScroll.LayoutUpdated += Scroll;
        }
    }

    private void CloseEpisodes()
    {
        if (_episodeSheet.Visibility != Visibility.Visible) return;
        _episodeSheet.Visibility = Visibility.Collapsed;
        _episodeList.Children.Clear();
        ScheduleHide();
        TakeKeyboard();
    }

    /// <summary>Picture, title, runtime (or Now Playing) and a short synopsis; a click plays it (Mac: EpisodePickerView's row).</summary>
    private FrameworkElement EpisodeRow(Item ep)
    {
        var current = ep.Id == _item.Id;
        var grid = new Grid { ColumnSpacing = 14, Padding = new Thickness(20, 8, 20, 8) };
        grid.ColumnDefinitions.Add(F.Auto);
        grid.ColumnDefinitions.Add(F.Star());
        var art = new Grid { Width = 128, Height = 72, VerticalAlignment = VerticalAlignment.Top };
        var thumb = ep.Images?.Thumb ?? ep.CardImage;
        art.Children.Add(F.Image(thumb != null ? _api.ImageUrl(thumb, 400) : null, 128, 72, 8, 256));
        if (ep.Progress is { } p && !current)
        {
            var bar = new Grid { Height = 3, VerticalAlignment = VerticalAlignment.Bottom, Margin = new Thickness(5, 0, 5, 5) };
            bar.Children.Add(new Border { CornerRadius = new CornerRadius(2), Background = new SolidColorBrush(Color.FromArgb(128, 0, 0, 0)) });
            bar.Children.Add(new Border { CornerRadius = new CornerRadius(2), Background = Theme.White, HorizontalAlignment = HorizontalAlignment.Left, Width = Math.Max(3, 118 * p) });
            art.Children.Add(bar);
        }
        if (current) art.Children.Add(new Border { CornerRadius = new CornerRadius(8), BorderBrush = Theme.White, BorderThickness = new Thickness(2) });
        grid.Children.Add(art.At(0, 0));
        var text = new StackPanel { Spacing = 3 };
        text.Children.Add(F.Text(ep.EpisodeListTitle, Theme.FontSubheadline, current ? F.Bold : null, maxLines: 2));
        var meta = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        if (EpisodeGroups.Detail(ep, _item.Id) is { } detail) meta.Children.Add(F.Text(detail, Theme.FontCaption, fg: Theme.Secondary));
        if (ep.User?.Played == true) meta.Children.Add(F.Glyph("\uE930", 12, Theme.Secondary));
        text.Children.Add(meta);
        if (!string.IsNullOrEmpty(ep.Overview))
        {
            var overview = F.Text(ep.Overview!, Theme.FontCaption, fg: Theme.Secondary, maxLines: 2);
            overview.Margin = new Thickness(0, 1, 0, 0);
            text.Children.Add(overview);
        }
        grid.Children.Add(text.At(0, 1));
        var hover = new Border { Background = new SolidColorBrush(Color.FromArgb(14, 255, 255, 255)), Opacity = 0, IsHitTestVisible = false };
        var host = new Grid();
        host.Children.Add(hover);
        host.Children.Add(grid);
        var button = F.Bare(host, () => PlayEpisode(ep), ep.EpisodeListTitle);
        button.HorizontalContentAlignment = HorizontalAlignment.Stretch;
        button.PointerEntered += (_, _) => hover.Opacity = 1;
        button.PointerExited += (_, _) => hover.Opacity = 0;
        return button;
    }

    private void PlayEpisode(Item ep)
    {
        CloseEpisodes();
        if (ep.Id == _item.Id) return;
        _ = SwitchTo(ep);
    }

    private async Task SwitchTo(Item ep)
    {
        await CloseCurrent(report: true);
        if (_stopped) return;
        PlayItem(ep, ep.ResumeSeconds);
    }

    // ---------------------------------------------------------------- scrubbing thumbnails

    /// <summary>The thumbnails of the file a new session plays (kept when it is the same file).</summary>
    private async Task LoadThumbnails()
    {
        var media = MediaId;
        if (_thumbsFor == media && _thumbs != null) return;
        _thumbsAskedAt = Environment.TickCount64;
        try
        {
            if (!(await _api.Capabilities()).Has("trickplay")) return;
            var t = await _api.Trickplay(media, _session);
            if (media != MediaId) return;
            AppLog.Write(t.Ready ? $"thumbnails: {t.Count} every {t.Interval} s" : "thumbnails: " + t.State);
            if (!t.Ready) return;
            if (_thumbs == null || !_thumbs.Sheets.SequenceEqual(t.Sheets)) { _sheets.Clear(); _sheetLoads.Clear(); }
            _thumbs = t;
            _thumbsFor = media;
        }
        catch (Exception e) { AppLog.Write("thumbnails: " + e.Message); }
    }

    private async Task LoadSheet(int n)
    {
        var thumbs = _thumbs;
        if (thumbs == null || !_sheetLoads.Add(n)) return;
        try
        {
            var bytes = await _api.FetchBytes(_api.Resolve(thumbs.Sheets[n]));
            if (thumbs != _thumbs) return;
            using var stream = new Windows.Storage.Streams.InMemoryRandomAccessStream();
            await stream.WriteAsync(bytes.AsBuffer());
            stream.Seek(0);
            var bmp = new BitmapImage();
            await bmp.SetSourceAsync(stream);
            if (thumbs != _thumbs) return;
            _sheets[n] = bmp;
            if (_dragging) ShowPreview();
        }
        catch (Exception e)
        {
            _sheetLoads.Remove(n);
            AppLog.Write("thumbnail sheet: " + e.Message);
        }
    }

    /// <summary>The picture for the time dragged to, above the knob; its sheet is asked for when it isn't here.</summary>
    private void ShowPreview()
    {
        var t = _thumbs;
        if (t == null || !_dragging || t.TileAt(_dragTime) is not { } tile)
        {
            _preview.Visibility = Visibility.Collapsed;
            // Not ready when the title started (made in the background): ask again now and then.
            if (_dragging && _thumbs == null && Environment.TickCount64 - _thumbsAskedAt > 20_000) _ = LoadThumbnails();
            return;
        }
        var h = ScrubPreview.Height(t);
        _previewCanvas.Height = h;
        _previewCanvas.Clip = new RectangleGeometry { Rect = new Windows.Foundation.Rect(0, 0, ScrubPreview.Width, h) };
        if (_sheets.TryGetValue(tile.Sheet, out var sheet))
        {
            var scale = ScrubPreview.Width / t.Width!.Value;
            _previewImage.Source = sheet;
            _previewImage.Width = t.Columns!.Value * t.Width.Value;
            _previewImage.Height = t.Rows!.Value * t.Height!.Value;
            _previewImage.RenderTransform = new CompositeTransform { ScaleX = scale, ScaleY = scale, TranslateX = -tile.X * scale, TranslateY = -tile.Y * scale };
            _previewImage.Visibility = Visibility.Visible;
        }
        else
        {
            _previewImage.Visibility = Visibility.Collapsed;
            _ = LoadSheet(tile.Sheet);
        }
        _previewTime.Text = Format.Clock(_dragTime);
        var origin = _scrubber.TransformToVisual(_chrome).TransformPoint(new Windows.Foundation.Point(0, 0));
        var knob = Math.Clamp(_dragTime / Math.Max(1, _duration), 0, 1) * _scrubber.ActualWidth;
        _preview.Margin = new Thickness(origin.X + ScrubPreview.Left(knob, _scrubber.ActualWidth), Math.Max(0, origin.Y - h - 42), 0, 0);
        _preview.Visibility = Visibility.Visible;
    }

    private void QualityMenu()
    {
        var menu = new MenuFlyout();
        var file = VersionFile();
        if (_item.HasVersions)
        {
            // The title's versions: picking one restarts it there at the same position (Mac: Version, first).
            var versions = new MenuFlyoutSubItem { Text = "Version", Icon = new FontIcon { Glyph = "\uE8F1" } };
            foreach (var v in _item.PresentFiles)
            {
                var it = new ToggleMenuFlyoutItem { Text = v.VersionTitle, IsChecked = v.Id == file?.Id, IsEnabled = !v.IsBroken || v.Id == file?.Id };
                if (v.VersionDetails is { } d) it.KeyboardAcceleratorTextOverride = d;
                it.Click += (_, _) => ChooseVersion(v);
                versions.Items.Add(it);
            }
            menu.Items.Add(versions);
            menu.Items.Add(new MenuFlyoutSeparator());
        }
        menu.Items.Add(new MenuFlyoutItem { Text = "Quality", IsEnabled = false });
        var runtime = file?.Duration ?? _item.Runtime ?? (_duration > 0 ? _duration : null);
        foreach (var q in QualityOption.Options(file))
        {
            // The size on the right, where a menu shows a shortcut.
            var it = new ToggleMenuFlyoutItem { Text = q.Label, IsChecked = q == _quality, KeyboardAcceleratorTextOverride = q.SizeLabel(runtime, file?.Size) ?? "" };
            it.Click += (_, _) => SetQuality(q);
            menu.Items.Add(it);
        }
        ShowMenu(menu, _qualityButton);
    }

    /// <summary>Another quality: the stream restarts at it where it is, and titles start at it from now on.</summary>
    private void SetQuality(QualityOption q)
    {
        if (q == _quality) return;
        _quality = q;
        _prefs.StreamingQuality = q.Label;
        Prefs.Save();
        RestartInPlace();
    }

    /// <summary>Another version of the title: it plays from the same position, and is this title's version from now on (on every device).</summary>
    private void ChooseVersion(MediaFile v)
    {
        if (v.Id == VersionFile()?.Id) return;
        _versionId = v.Id;
        _quality = QualityChoice.Starting(_prefs.StreamingQuality, v);
        var item = _item.Id;
        _ = Task.Run(async () => { try { await _api.ChooseVersion(item, v.Id); } catch { /* this playback still uses it */ } });
        // Its own tracks, as a title starting.
        _selectedAudio = null;
        _selectedSub = null;
        _burn = null;
        LoadSubtitle(null);
        _restartInPlace = false;
        _ = CloseCurrent(false);
        Start(_position > 1 ? _position : _startAt);
    }

    private void ShowMenu(MenuFlyout menu, Button anchor)
    {
        _menuOpen = true;
        menu.Closed += (_, _) => { _menuOpen = false; ScheduleHide(); TakeKeyboard(); };
        _hideTimer.Stop();
        menu.ShowAt(anchor);
    }

    // ---------------------------------------------------------------- segments

    private async Task LoadSegments(bool provisional)
    {
        try
        {
            var runtime = _item.PresentFiles.FirstOrDefault(f => f.Id == _versionId)?.Duration ?? _item.Runtime ?? 0;
            var server = await _api.Segments(_item.Id, _versionId, provisional);
            _serverPending = server.Pending;
            _segments = SegmentOffer.Offered(server, runtime, provisional, _item.IsMovie);
            _segmentsFinal = !server.Pending && !server.CommunityPending;
            _segmentsAskedAt = _position;
            if (server.CommunityPending)
            {
                await Task.Delay(5000);
                if (!_stopped && _session != null) await LoadSegments(provisional: false);
            }
        }
        catch { /* the player works without them */ }
    }

    private void CheckSegments(double t)
    {
        // Ask again near the end while the answer isn't final (the other apps' rule).
        if (!_segmentsFinal)
        {
            var due = SegmentOffer.NearEndAsksDue(_duration, t);
            if (due > _nearEndAsked) { _nearEndAsked = due; _ = LoadSegments(provisional: false); }
        }
        if (NextEpisodeRules.RefetchDue(_serverPending, _duration, t, _segmentsAskedAt)) { _segmentsAskedAt = t; _ = LoadSegments(provisional: false); }

        var active = NextEpisodeRules.Active(_segments, t, _prefs.SkipMode);
        var creditsPlaysNext = NextEpisodeRules.CreditsSkipPlaysNext(active, _duration, _nextEpisode != null);
        var nextOffered = NextEpisodeRules.Offered(_duration, t, t, active, _segments, _nextEpisode != null);

        // Entering an auto-skip segment: count down, then skip.
        if (active != _active)
        {
            _active = active;
            _skipCountdown.Stop();
            if (active != null && _prefs.SkipMode(active.Kind) == SkipMode.Auto && _skippedSegment != active.Kind + active.Start.ToString() && !_paused)
            {
                // Settings' Auto-Skip Countdown; None skips at once.
                _autoSkipTotal = _prefs.AutoSkipDelay;
                if (_autoSkipTotal <= 0) { SkipSegment(); return; }
                _autoSkipLeft = _autoSkipTotal;
                _skipCountdown.Start();
            }
        }

        var counting = _skipCountdown.IsRunning;
        var corner = NextEpisodeRules.Corner(active, _controlsShown, promptVisible: active != null, counting, nextOffered, creditsPlaysNext);
        _skipButton.Visibility = corner.Skip ? Visibility.Visible : Visibility.Collapsed;
        _skipText.Text = active == null ? "" : (creditsPlaysNext ? "Next Episode" : active.Kind.SkipLabel());
        _nextButton.Visibility = corner.Next ? Visibility.Visible : Visibility.Collapsed;
    }

    private void TickAutoSkip()
    {
        if (_active == null || _paused) { _skipCountdown.Stop(); _skipFill.Width = 0; return; }
        _autoSkipLeft -= 0.1;
        var w = Math.Max(0, _skipButton.ActualWidth * (_autoSkipTotal - _autoSkipLeft) / _autoSkipTotal);
        _skipFill.Width = w;
        if (_autoSkipLeft <= 0) { _skipCountdown.Stop(); _skipFill.Width = 0; SkipSegment(); }
    }

    private void SkipSegment()
    {
        var a = _active;
        if (a == null) return;
        _skippedSegment = a.Kind + a.Start.ToString();
        _skipCountdown.Stop();
        if (NextEpisodeRules.CreditsSkipPlaysNext(a, _duration, _nextEpisode != null)) { PlayNext(); return; }
        _position = a.End;
        _mpv?.Seek(a.End);
    }

    private void PlayNext()
    {
        if (_nextEpisode == null) return;
        _ = Finish();
    }

    // ---------------------------------------------------------------- input and controls

    private void OnPointerMoved(object sender, PointerRoutedEventArgs e)
    {
        var p = e.GetCurrentPoint(this).Position;
        if (Math.Abs(p.X - _lastPointerX) < 2 && Math.Abs(p.Y - _lastPointerY) < 2) return;
        _lastPointerX = p.X; _lastPointerY = p.Y;
        ShowControls(true);
        ScheduleHide();
    }

    private bool HitsControl(RoutedEventArgs e)
    {
        for (var o = e.OriginalSource as DependencyObject; o != null && o != this; o = VisualTreeHelper.GetParent(o))
            if (o is Button || o == _scrubCapsule || o == _episodeSheet) return true;
        return false;
    }

    /// <summary>The player keeps the keyboard: a click anywhere would otherwise leave it with a control, or with nothing.</summary>
    private void TakeKeyboard()
    {
        if (_menuOpen || _stopped) return;
        _keySink.Focus(FocusState.Programmatic);
    }

    private void OnTapped(object s, TappedRoutedEventArgs e)
    {
        TakeKeyboard();
        if (_inputGuarded || HitsControl(e)) return;
        TogglePause();
    }

    private void OnDoubleTapped(object s, DoubleTappedRoutedEventArgs e)
    {
        TakeKeyboard();
        if (_inputGuarded || HitsControl(e)) return;
        TogglePause(); // undo the first click's toggle
        ToggleFull();
    }

    private void OnWheel(object s, PointerRoutedEventArgs e)
    {
        var pp = e.GetCurrentPoint(this);
        if (!pp.Properties.IsHorizontalMouseWheel) return;
        var d = pp.Properties.MouseWheelDelta;
        SkipBy(d / 120.0 * 5);
    }

    private void OnKeyUp(object s, KeyRoutedEventArgs e)
    {
        if (e.Key != VirtualKey.Space || !_swallowSpaceUp) return;
        _swallowSpaceUp = false;
        e.Handled = true;
    }

    private void OnKey(object s, KeyRoutedEventArgs e)
    {
        if (_menuOpen) return;
        var handled = true;
        switch (e.Key)
        {
            case VirtualKey.Space:
                _swallowSpaceUp = true;
                if (!e.KeyStatus.WasKeyDown) TogglePause(); // not again on auto-repeat
                break;
            case VirtualKey.Left: SkipBy(-10); break;
            case VirtualKey.Right: SkipBy(10); break;
            case VirtualKey.Up: Volume(10); break;
            case VirtualKey.Down: Volume(-10); break;
            case VirtualKey.M: _mpv?.ToggleMute(); ShowVolume(); break;
            case VirtualKey.F: ToggleFull(); break;
            case VirtualKey.Escape when _episodeSheet.Visibility == Visibility.Visible:
                CloseEpisodes();
                break;
            case VirtualKey.Escape:
                if (_shell.IsFullScreen) { _shell.LeaveFullScreen(); UpdateFull(); TakeKeyboardSoon(); } else Close();
                break;
            default: handled = false; break;
        }
        if (handled) { e.Handled = true; ShowControls(true); ScheduleHide(); }
    }

    private void Volume(double delta) { _mpv?.AddVolume(delta); ShowVolume(); }
    private void ShowVolume() { _volumeHud.Opacity = 1; _volumeTimer.Stop(); _volumeTimer.Start(); }

    private void UpdateVolumeHud()
    {
        _volumeGlyph.Text = _muted ? "\uE74F" : "\uE767";
        _volumeFill.Width = _muted ? 0 : 110 * Math.Clamp(_volume / 100, 0, 1);
    }

    private void TogglePause() => _mpv?.TogglePause();
    private void SkipBy(double seconds)
    {
        if (_mpv == null) return;
        _position = Math.Clamp(_position + seconds, 0, _duration > 0 ? _duration : double.MaxValue); // a seek may end the title before time-pos reports it
        _mpv.Seek(_position);
    }
    private void ToggleFull() { _shell.ToggleFullScreen(); UpdateFull(); TakeKeyboardSoon(); }

    // A presenter change settles after this returns: focus again once it has.
    public void TakeKeyboardSoon()
    {
        TakeKeyboard();
        _refocusTimer.Stop();
        _refocusTimer.Start();
    }
    private void ToggleFill() { _fill = !_fill; _mpv?.SetFill(_fill); _fillGlyph.Text = _fill ? "\uE9A5" : "\uE9A6"; }

    /// <summary>
    /// The title shows in full screen only (on its page the page shows it), and the button says
    /// which way it goes. Only full screen covers the title bar's row, and it has no caption buttons.
    /// </summary>
    private void UpdateFull()
    {
        _top.Margin = new Thickness(20, 12, 20, 0);
        _titles.Visibility = _fullScreen ? Visibility.Visible : Visibility.Collapsed;
        _fullGlyph.Text = _fullScreen ? "\uE73F" : "\uE740";
    }

    /// <summary>The shell put the player on its page or full screen.</summary>
    public void SetFullScreen(bool full)
    {
        _fullScreen = full;
        UpdateFull();
    }

    private void ShowControls(bool show)
    {
        if (show != _controlsShown)
        {
            _controlsShown = show;
            _chrome.Opacity = show ? 1 : 0;
            _chrome.IsHitTestVisible = show && !_inputGuarded;
            // Hidden only with the pointer over the video (on its page the pointer may be on the page).
            if (show && _cursorHidden) { _cursorHidden = false; Native.ShowCursor(true); }
            else if (!show && !_cursorHidden && (_pointerInside || _fullScreen)) { _cursorHidden = true; Native.ShowCursor(false); }
            PlaceSubtitles();
        }
    }

    private void ScheduleHide()
    {
        _hideTimer.Stop();
        if (_episodeSheet.Visibility == Visibility.Visible) return; // the controls stay while Episodes shows
        if (!_paused && !_stopped) _hideTimer.Start();
    }

    private void UpdateTimes()
    {
        var t = _dragging ? _dragTime : _position;
        _elapsed.Text = Format.Clock(t);
        _remaining.Text = "−" + Format.Clock(Math.Max(0, _duration - t));
        var w = _scrubber.ActualWidth;
        if (w > 0 && _duration > 0)
        {
            var x = Math.Clamp(t / _duration, 0, 1) * w;
            _trackFill.Width = x;
            _knob.Margin = new Thickness(Math.Max(0, x - 7), 0, 0, 0);
        }
    }

    private void ScrubDown(object s, PointerRoutedEventArgs e)
    {
        if (_duration <= 0) return;
        _dragging = true;
        _scrubber.CapturePointer(e.Pointer);
        ScrubTo(e);
        e.Handled = true;
    }

    private void ScrubMove(object s, PointerRoutedEventArgs e) { if (_dragging) { ScrubTo(e); e.Handled = true; } }

    private void ScrubTo(PointerRoutedEventArgs e)
    {
        var x = e.GetCurrentPoint(_scrubber).Position.X;
        _dragTime = Math.Clamp(x / _scrubber.ActualWidth, 0, 1) * _duration;
        UpdateTimes();
        ShowPreview();
    }

    private void ScrubUp(object s, PointerRoutedEventArgs e)
    {
        if (!_dragging) return;
        _dragging = false;
        _preview.Visibility = Visibility.Collapsed;
        _scrubber.ReleasePointerCapture(e.Pointer);
        _mpv?.Seek(_dragTime);
        _position = _dragTime;
        ScheduleHide();
        e.Handled = true;
    }

    /// <summary>Playback can't start: the problem's title and line; the server's words behind Details.</summary>
    private void ShowProblem(PlaybackProblem p)
    {
        // The title page knew why (a damaged file, or none found): its words, with its details.
        if (p.Kind is PlaybackProblem.Kinds.Missing or PlaybackProblem.Kinds.Damaged && PlaybackProblem.Unplayable(_item) is { } known) p = known;
        ShowError(p.Message, p.Title, p.Detail);
    }

    private void ShowError(string message, string? title = null, string? detail = null)
    {
        _buffering.IsActive = false;
        _errorTitle.Text = title ?? "";
        _errorTitle.Visibility = title == null ? Visibility.Collapsed : Visibility.Visible;
        _errorDetail.Text = detail ?? "";
        _errorDetail.Visibility = Visibility.Collapsed;
        _detailsButton.Visibility = detail == null ? Visibility.Collapsed : Visibility.Visible;
        _errorText.Text = message;
        _errorBox.Visibility = Visibility.Visible;
        UpdateAwake();
        ShowControls(true);
    }

    /// <summary>Holds the display awake while the video plays; paused, ended, failed or closed lets it sleep.</summary>
    private void UpdateAwake() =>
        _awake.Set(ScreenAwake.Hold(_paused, ended: _finishing, failed: _errorBox.Visibility == Visibility.Visible, closed: _stopped || _mpv == null));

    private void Close() => _shell.ClosePlayer();

    /// <summary>What needs clicks in the title bar's row; the rest of it moves the window.</summary>
    public FrameworkElement[] TitleRowControls => [_closeCircle, _toolbarCapsule];

    /// <summary>The title whose watch state this playback changes: for an episode, its show.</summary>
    public string WatchedId => _item.SeriesId ?? _item.Id;

    /// <summary>The title or episode playing now.</summary>
    public string ItemId => _item.Id;

    public void Stop()
    {
        if (_stopped) return;
        _stopped = true;
        _windowLoader?.Stop();
        _windowLoader = null;
        if (_pendingNudge != null) _ = SendNudge();
        foreach (var release in _releaseTimers) release();
        _releaseTimers.Clear();
        if (_cursorHidden) { _cursorHidden = false; Native.ShowCursor(true); }
        var session = _session;
        _session = null;
        if (session != null)
        {
            var api = _api; var pos = _position; var paused = _paused; var a = _selectedAudio; var c = SegmentRules.CreditsStart(_segments); var tracks = TracksPlaying();
            var report = _reportStarted && !_noReport;
            _ = Task.Run(async () =>
            {
                if (report) try { await api.Report(session, ReportEvent.Stop, pos, paused, audio: a, credits: c, tracks: tracks); } catch { }
                await api.CloseSession(session);
            });
        }
        var mpv = _mpv;
        _mpv = null;
        mpv?.Dispose();
        _awake.Dispose();
    }
}
