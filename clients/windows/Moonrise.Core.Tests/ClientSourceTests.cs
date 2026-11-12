using System.Text.RegularExpressions;
using Xunit;

namespace Moonrise.Core.Tests;

/// <summary>Scans of the WinUI sources, for what can't run without a window (the regression map is in docs/dev/testing.md).</summary>
public partial class ClientSourceTests
{
    private static readonly string App = Path.Combine(Path.GetDirectoryName(Repo.ApiDir)!, "clients", "windows", "Moonrise.App");

    private static string Read(params string[] parts) => File.ReadAllText(Path.Combine([App, .. parts]));

    private static IEnumerable<(string File, string Source)> Sources() =>
        Directory.GetFiles(App, "*.cs", SearchOption.AllDirectories)
            .Where(f => !f.Contains($"{Path.DirectorySeparatorChar}obj{Path.DirectorySeparatorChar}") && !f.Contains($"{Path.DirectorySeparatorChar}bin{Path.DirectorySeparatorChar}"))
            .Select(f => (Path.GetFileName(f), File.ReadAllText(f)));

    /// <summary>The block-bodied methods of a file: name and body, at the class's indentation.</summary>
    private static IEnumerable<(string Name, string Body)> Methods(string source)
    {
        var lines = source.Split('\n').Select(l => l.TrimEnd('\r')).ToList();
        for (var i = 0; i + 1 < lines.Count; i++)
        {
            var head = Head().Match(lines[i]);
            if (!head.Success || lines[i].Contains("=>") || lines[i + 1] != "    {") continue;
            var end = lines.FindIndex(i + 1, l => l == "    }");
            yield return (head.Groups["name"].Value, string.Join("\n", lines.Skip(i + 2).Take(end - i - 2)));
        }
    }

    [GeneratedRegex(@"^    (?:public|private|internal)\b.*?(?<name>\w+)(?:<[^>]+>)?\(.*\)\s*$")]
    private static partial Regex Head();

    // Called only while a screen is first built, so these hand their fields to a parent once.
    private static readonly (string File, string Method)[] BuiltOnce =
        [("PlayerPage.cs", "BuildChrome"), ("PlayerPage.cs", "BuildError"), ("PlayerPage.cs", "BuildOverlays"), ("Shell.cs", "BuildTabs")];

    /// <summary>A file without its constructors and built-once methods: the code that runs again and again.</summary>
    private static string RunsAgain(string file, string source)
    {
        var classes = Regex.Matches(source, @"\bclass\s+(\w+)").Select(m => m.Groups[1].Value).ToHashSet();
        foreach (var (name, body) in Methods(source))
            if (classes.Contains(name) || BuiltOnce.Contains((file, name))) source = source.Replace(body, "");
        return source;
    }

    /// <summary>
    /// Every way an element built once could be placed again. XAML throws "already the child of
    /// another element", and from a click or size handler that ended the app: the show page's row
    /// (a field), the Library grid on a resize (a call's result, on a button already showing).
    /// </summary>
    private static IEnumerable<string> ReplacementSites(string file, string source)
    {
        foreach (Match m in TakenOut().Matches(source)) yield return $"{file}: reads an element out of its parent: {m.Value}";
        var again = RunsAgain(file, source);
        foreach (Match m in HandedOver().Matches(again)) yield return $"{file}: a field given a new parent: {m.Value}";
        foreach (Match m in AddedAgain().Matches(again)) yield return $"{file}: a field added to a panel: {m.Value}";
        foreach (Match m in Reassigned().Matches(again))
            if (!Regex.IsMatch(again, $@"\bvar\s+{m.Groups["target"].Value}\s*=\s*new\b"))
                yield return $"{file}: content of an element not built here replaced: {m.Value}";
    }

    [Fact]
    public void AnElementIsPlacedOnce()
    {
        Assert.Empty(Sources().SelectMany(s => ReplacementSites(s.File, s.Source)));
    }

    [Theory]
    // The two crashes the reviews found, and the other forms of the same mistake.
    [InlineData("foreach (var c in _grid.Children) if (c is Button b) b.Content = PosterContent(item, w);")]
    [InlineData("private FrameworkElement P() => (FrameworkElement)((Button)Cards.Poster(a)).Content!;")]
    [InlineData("var scroller = new ScrollViewer { Content = _episodesPanel };")]
    [InlineData("_host.Child = Build();")]
    [InlineData("_panel.Children.Add(_page);")]
    [InlineData("_panel.Children.Insert(0, _page);")]
    [InlineData("var inner = other.Child;")]
    public void TheScanCatchesEveryFormOfIt(string line)
    {
        var source = $"class X\n{{\n    public void Handler()\n    {{\n        {line}\n    }}\n}}\n";
        Assert.NotEmpty(ReplacementSites("X.cs", source));
    }

    [Theory]
    [InlineData("var b = new Border { Child = F.Text(t) };")]
    [InlineData("var host = new Border(); host.Child = clip;")]
    [InlineData("_panel.Children.Add(F.Text(t));")]
    [InlineData("_column.Children.Add(meta);")]
    public void TheScanLeavesFreshElementsAlone(string line)
    {
        var source = $"class X\n{{\n    public void Handler()\n    {{\n        {line}\n    }}\n}}\n";
        Assert.Empty(ReplacementSites("X.cs", source));
    }

    [Fact]
    public void BuiltOnceMethodsAreOnlyCalledFromTheConstructor()
    {
        foreach (var (file, method) in BuiltOnce)
        {
            var source = Sources().Single(s => s.File == file).Source;
            var cls = Path.GetFileNameWithoutExtension(file);
            Assert.Contains($"{method}()", Methods(source).Single(m => m.Name == cls).Body);
            Assert.Equal(2, Regex.Matches(source, $@"\b{method}\(\)").Count); // its definition and that one call
        }
    }

    [Fact]
    public void PagesOnTheStacksAreShownThroughOneDoor()
    {
        // ShowOnly clears first, and leaves a page already showing alone.
        var shell = Read("Shell.cs");
        Assert.DoesNotContain("_pageHost.Children.Add(", shell);
        Assert.DoesNotContain("_playerHost.Children.Add(", shell.Replace("_root.Children.Add(_playerHost)", ""));
        var factory = Read("Ui", "Factory.cs");
        var (_, showOnly) = Methods(factory).Single(m => m.Name == "ShowOnly");
        Assert.True(showOnly.IndexOf("Clear()", StringComparison.Ordinal) < showOnly.IndexOf(".Add(", StringComparison.Ordinal));
        Assert.Contains("ReferenceEquals", showOnly);
    }

    [Fact]
    public void AResizeChangesPosterCardsInPlace()
    {
        var card = Read("Ui", "Cards.cs");
        Assert.Contains("public void Resize(double width)", card);
        Assert.DoesNotContain("PosterContent", Read("Pages", "LibraryPage.cs"));
        Assert.Contains("c.Resize(g.CardWidth)", Read("Pages", "LibraryPage.cs"));
    }

    [Fact]
    public void WrapGridsGetThePanelWidthThatHoldsTheirColumns()
    {
        foreach (var page in new[] { "LibraryPage.cs", "SearchPage.cs" })
        {
            var source = Read("Pages", page);
            Assert.Contains("PanelWidth", source);
            Assert.Contains("CellWidth", source);
            Assert.DoesNotContain("w + 12", source);
            Assert.DoesNotContain("cardWidth + 14", source);
        }
        Assert.Contains("ItemWidth = 254, ItemHeight = 58, Margin = new Thickness(0, 0, -14, 0)", Read("Pages", "DetailPage.cs"));
    }

    [Fact]
    public void AMissedErrorIsLoggedAndShownNotTheEndOfTheApp()
    {
        var app = Read("App.xaml.cs");
        Assert.Contains("UnhandledException += (_, e) =>", app);
        Assert.Contains("e.Handled = true", app);
        Assert.Contains("Failed(e.Exception)", app);
        Assert.Contains("window.Shell.Fault(e)", app);
        Assert.Contains("AppLog.Write", app);
        Assert.Contains("TaskScheduler.UnobservedTaskException", app);
        Assert.Contains("_faultHost", Read("Shell.cs"));
    }

    [Fact]
    public void DispatcherCallbacksAndTimersGoThroughTheGuard()
    {
        // A failure in one of these skips Application.UnhandledException and ends the process
        // (0xc000027b, in CoreMessaging), seen on the VM with an injected throw.
        foreach (var (file, source) in Sources())
        {
            if (file == "Safe.cs") continue;
            Assert.DoesNotContain("TryEnqueue(", source);
            Assert.Empty(Regex.Matches(source, @"\.Tick \+= (?!Safe\.Tick\()").Select(m => $"{file}: {m.Value}"));
            Assert.DoesNotContain("async void", source);
            Assert.DoesNotContain("+= async", source); // Safe.Fire instead
        }
    }

    [Fact]
    public void TheMpvThreadSurvivesAnError()
    {
        var mpv = Read("Player", "MpvSurface.cs");
        Assert.Contains("try { HandleEvent(_mpv.Wait(1.0)); }", mpv);
    }

    [Fact]
    public void AnEarlyEndOfStreamGoesThroughRecovery()
    {
        var (_, ended) = Methods(Read("Pages", "PlayerPage.cs")).Single(m => m.Name == "OnEnded");
        Assert.Contains("StreamEndIsTheEnd(_position, _duration)", ended);
    }

    [Fact]
    public void WatchChangesReachTheCardsThatShowThem()
    {
        Assert.Contains("_shell.Watch.Changed(_item.Id)", Read("Pages", "DetailPage.cs"));
        Assert.Contains("Watch.Changed(id)", Read("Shell.cs"));
        Assert.Contains("_pages.Replace(item)", Read("Pages", "LibraryPage.cs"));
        Assert.Contains("card?.Update(item)", Read("Pages", "LibraryPage.cs"));
        Assert.Contains("_shell.Watch.Since", Read("Pages", "HomePage.cs"));
    }

    [GeneratedRegex(@"(?<![\w""])(?:\w+|\))\s*\.(?:Content|Child)\b(?!\s*=(?!=))")]
    private static partial Regex TakenOut();

    [GeneratedRegex(@"\b(?:Content|Child)\s*=\s*_\w+")]
    private static partial Regex HandedOver();

    [GeneratedRegex(@"\.Children\.(?:Add|Insert)\(\s*(?:[^,()]+,\s*)?_\w+")]
    private static partial Regex AddedAgain();

    [GeneratedRegex(@"\b(?<target>\w+)\.(?:Content|Child)\s*=(?!=)")]
    private static partial Regex Reassigned();

    [Fact]
    public void TheEpisodeRowIsBuiltForEachRender()
    {
        var detail = Read("Pages", "DetailPage.cs");
        Assert.DoesNotContain("_episodesPanel", detail);
        var (_, area) = Methods(detail).Single(m => m.Name == "SeasonsArea");
        Assert.Contains("new StackPanel", area);
        Assert.Contains("F.Row(panel)", area);
    }

    [Fact]
    public void TheHeroDecidesWhichPictureShows()
    {
        // F.Art used to set Opacity = 1 on every picture that opened: the last download sat on top.
        var factory = Read("Ui", "Factory.cs");
        var (_, art) = Methods(factory).Single(m => m.Name == "Art");
        Assert.Contains("opened != null", art);
        var home = Read("Pages", "HomePage.cs");
        Assert.Contains("slides.Opened(at)", home);
        Assert.Contains("images[k].Opacity = slides.Opacity(k)", home);
        Assert.DoesNotContain("images[k].Opacity = k == i", home);
    }

    [Fact]
    public void ThePlayerKeepsTheKeyboardAfterAnyClick()
    {
        var player = Read("Pages", "PlayerPage.cs");
        Assert.Contains("PreviewKeyDown += OnKey", player);
        Assert.Contains("PreviewKeyUp += OnKeyUp", player);
        Assert.DoesNotContain("AddHandler(KeyDownEvent", player);
        Assert.Contains("AddHandler(PointerReleasedEvent", player);
        Assert.Contains("TakeKeyboard()", Methods(player).Single(m => m.Name == "OnLoaded").Body);
        Assert.Contains("o == _scrubCapsule", player);
        Assert.Contains("_keySink.Focus", player); // a panel doesn't keep focus through a click
        Assert.Contains("TakeKeyboardSoon()", Methods(player).Single(m => m.Name == "OnKey").Body);
        // A full screen change takes keyboard focus from the window's content, until reactivated.
        var shell = Read("Shell.cs");
        Assert.Equal(2, shell.Split("_window.Activate()").Length - 1);
    }

    [Fact]
    public void TheHudShowsTheMutedState()
    {
        var player = Read("Pages", "PlayerPage.cs");
        Assert.Contains("MuteChanged", player);
        Assert.Contains("_muted ? 0", player);
        Assert.Contains("\"mute\"", Read("Player", "MpvSurface.cs"));
    }

    [Fact]
    public void TheVerticalWheelMovesThePageNotARow()
    {
        var offenders = new List<string>();
        foreach (var (file, source) in Sources())
        {
            if (file == "Factory.cs") continue;
            if (source.Contains("HorizontalScrollMode = ScrollMode.Enabled")) offenders.Add(file);
        }
        Assert.Empty(offenders); // every sideways row goes through F.Row
        var factory = Read("Ui", "Factory.cs");
        Assert.Contains("IsHorizontalMouseWheel", factory);
    }

    [Fact]
    public void HomeWithoutAHeroLeavesRoomForTheTopBar() =>
        Assert.Contains("hero.Count > 0 ? 0 : Shell.TopBarHeight", Read("Pages", "HomePage.cs"));

    [Fact]
    public void TheWindowStillMovesWhileThePlayerIsOpen()
    {
        var shell = Read("Shell.cs");
        var (_, play) = Methods(shell).Single(m => m.Name == "Play");
        var (_, close) = Methods(shell).Single(m => m.Name == "ClosePlayer");
        Assert.DoesNotContain("_topBar", play); // the title bar's element stays, or nothing is left to drag
        Assert.DoesNotContain("_topBar", close);
        Assert.Contains("_player.TitleRowControls", shell);
        Assert.Contains("ClientRules.PlayerTitleRowPassthrough(IsFullScreen", shell);
    }

    [Fact]
    public void TheClickThatOpenedThePlayerDoesNothingOnIt()
    {
        var player = Read("Pages", "PlayerPage.cs");
        var methods = Methods(player).ToDictionary(m => m.Name, m => m.Body);
        Assert.Contains("_inputGuarded || HitsControl(e)", methods["OnTapped"]);
        Assert.Contains("_inputGuarded || HitsControl(e)", methods["OnDoubleTapped"]);
        Assert.Contains("_inputGuardTimer.Start()", methods["OnLoaded"]);
        Assert.Contains("_chrome.IsHitTestVisible = false", player); // a Button or the scrubber would take it instead
        Assert.Contains("_chrome.IsHitTestVisible = show && !_inputGuarded", methods["ShowControls"]);
        Assert.Contains("ClientRules.PlayerOpenInputGuardMs", player);
    }

    [Fact]
    public void ClosingThePlayerLeavesNothingHoldingItAlive()
    {
        var player = Read("Pages", "PlayerPage.cs");
        var methods = Methods(player).ToDictionary(m => m.Name, m => m.Body);
        Assert.Single(Regex.Matches(player, @"CreateTimer\(\)")); // every timer through NewTimer
        Assert.Contains("Safe.Hook(timer, tick)", methods["NewTimer"]);
        Assert.Contains("timer.Tick -= handler", Read("Ui", "Safe.cs"));
        Assert.Contains("_releaseTimers", methods["Stop"]);
        Assert.Contains("if (!_paused && !_stopped) _hideTimer.Start()", player);
        var (_, dispose) = Methods(Read("Player", "MpvSurface.cs")).Single(m => m.Name == "Dispose");
        Assert.Contains("SetPanelSwapChain(null)", dispose);
    }

    [Fact]
    public void ThePlayerFollowsTheSettings()
    {
        // Settings (and picks in the player) decide the starting tracks, the skip modes and countdown, and the quality.
        var player = Read("Pages", "PlayerPage.cs");
        var methods = Methods(player).ToDictionary(m => m.Name, m => m.Body);
        Assert.Contains("TrackRules.PreferredAudio(audioCandidates, _prefs.PreferredAudioLanguage)", methods["ApplyStartingTracks"]);
        Assert.Contains("_prefs.AutoSelectSubtitles", methods["ApplyStartingTracks"]);
        Assert.Contains("_prefs.PreferredSubtitleLanguage, _prefs.PreferSdhSubtitles", methods["ApplyStartingTracks"]);
        Assert.Contains("NextEpisodeRules.Active(_segments, t, _prefs.SkipMode)", methods["CheckSegments"]);
        Assert.Contains("_prefs.AutoSkipDelay", methods["CheckSegments"]);
        Assert.DoesNotContain("DefaultMode()", player);
        Assert.Contains("quality: QualityChoice.Request(_quality)", methods["StartAsync"]);
        Assert.Contains("_prefs.SubtitlePicked(", methods["SelectSubtitle"]);
        Assert.Contains("_prefs.AudioPicked(", methods["ChooseAudio"]);
        Assert.Contains("_prefs.StreamingQuality = q.Label", methods["SetQuality"]);
    }

    [Fact]
    public void SubtitlesAreDrawnByTheAppNotByMpv()
    {
        // Drawn here, so Style and Delay apply; mpv would draw a second copy.
        Assert.Contains("_mpv.Option(\"sid\", \"no\")", Read("Player", "MpvSurface.cs"));
        var player = Read("Pages", "PlayerPage.cs");
        Assert.DoesNotContain("SetSubtitleTrack(t", player);
        Assert.Contains("new SubtitleWindowLoader(", player);
        Assert.Contains("info.Has(\"subtitle-windows\")", player);
        Assert.Contains("_windowLoader?.Advance(t - _subtitleDelay)", Methods(player).Single(m => m.Name == "OnTime").Body);
        // Over the controls' shading, which dimmed them.
        Assert.True(player.IndexOf("Children.Add(_subtitleLayer)", StringComparison.Ordinal) > player.IndexOf("Children.Add(_chrome)", StringComparison.Ordinal));
    }

    [Fact]
    public void AnErrorBeforeThereIsAWindowEndsTheProcess()
    {
        var (_, failed) = Methods(Read("App.xaml.cs")).Single(m => m.Name == "Failed");
        Assert.Contains("Window is { } window", failed);
        Assert.Contains("Environment.Exit(1)", failed);
        foreach (var (file, source) in Sources())
            Assert.False(source.Contains("Window?.Shell.Fault"), $"{file}: does nothing while there's no window");
        Assert.Equal(2, Regex.Matches(Read("Ui", "Safe.cs"), @"App\.Failed\(e\)").Count);
    }

    [Fact]
    public void ASecondLaunchBringsTheRunningCopyForward()
    {
        var app = Read("App.xaml.cs");
        Assert.True(app.IndexOf("var running = RunningCopy()", StringComparison.Ordinal) < app.IndexOf("Window = new MainWindow()", StringComparison.Ordinal));
        Assert.Contains("FindOrRegisterForKey(AppInfo.Name)", app); // one copy per app: the Dev app has its own
        Assert.Contains("RedirectActivationToAsync", app);
        Assert.Contains("BringForward()", app);
        Assert.Contains("SetForegroundWindow", Read("MainWindow.xaml.cs"));
    }

    [Fact]
    public void LongTitlesOnTheTitlePageEndInAnEllipsis()
    {
        var detail = Read("Pages", "DetailPage.cs");
        Assert.Contains("F.Text(_item.CardTitle, Theme.FontLargeTitle, F.Bold, maxLines: 3)", detail);
        Assert.Matches(@"_focus\.EpisodeLabel.*trim: TextTrimming\.CharacterEllipsis", detail);
    }

    [Fact]
    public void ALongHeroTitleEndsInAnEllipsis() =>
        Assert.Contains("var titleText = F.Text(\"\", Theme.FontLargeTitle, F.Bold, maxLines: 3)", Read("Pages", "HomePage.cs"));

    [Fact]
    public void LeavingFullScreenReturnsTheWindowToWhereItWas()
    {
        var methods = Methods(Read("Shell.cs"));
        var toggle = methods.Single(m => m.Name == "ToggleFullScreen").Body;
        var leave = methods.Single(m => m.Name == "LeaveFullScreen").Body;
        // The whole placement, read before the presenter changes, maximized or not: a maximized
        // window keeps the place it restores to (its bounds alone lost it, and it came back somewhere older).
        Assert.Matches(@"(?s)GetWindowPlacement\(.*SetPresenter\(AppWindowPresenterKind\.FullScreen\)", toggle);
        Assert.DoesNotContain("OverlappedPresenterState.Maximized", toggle);
        // Put back after the presenter changes, or the presenter's own position wins.
        Assert.Matches(@"(?s)SetPresenter\(.*SetWindowPlacement\(", leave);
        Assert.DoesNotContain("MoveAndResize", leave);
    }

    [Fact]
    public void ThePlayerTakesTheKeyboardBackAfterMaximizeOrRestore()
    {
        // Maximize or Restore from the caption buttons left the player's keys dead until a click on it.
        var shell = Read("Shell.cs");
        Assert.Contains("_window.AppWin.Changed +=", shell);
        var changed = Methods(shell).Single(m => m.Name == "WindowStateMaybeChanged").Body;
        Assert.Contains("OverlappedPresenter", changed);
        Assert.Contains("_player.TakeKeyboardSoon()", changed);
        Assert.Contains("OverlappedPresenterState.Minimized", changed); // never on the way to the taskbar
    }

    [Fact]
    public void SearchsBoxTakesTheKeyboardBackAfterMaximizeOrRestore()
    {
        // With the player closed, Maximize or Restore from the caption buttons left the search
        // box without focus, and typing went nowhere. The page's main control takes it back.
        var shell = Read("Shell.cs");
        var methods = Methods(shell);
        var changed = methods.Single(m => m.Name == "WindowStateMaybeChanged").Body;
        Assert.Matches(@"(?s)if \(_player != null\) _player\.TakeKeyboardSoon\(\);\s*else FocusPageSoon\(\);", changed);
        Assert.Matches(@"(?s)Minimized\) return;.*FocusPageSoon\(\)", changed); // never on the way to the taskbar
        // Again once the change has settled, as the player does.
        var soon = methods.Single(m => m.Name == "FocusPageSoon").Body;
        Assert.Contains("FocusPage();", soon);
        Assert.Contains("_refocus.Start()", soon);
        Assert.Contains("_refocus.Tick += Safe.Tick(FocusPage)", shell);
        Assert.Contains("(page as IFocusable)?.TakeFocus()", methods.Single(m => m.Name == "FocusPage").Body);
        var search = Read("Pages", "SearchPage.cs");
        Assert.Contains("IFocusable", search);
        Assert.Contains("public void TakeFocus() => _box.Focus(FocusState.Programmatic);", search);
    }

    [Fact]
    public void TrackButtonsAreSetWhenThePlaybackAnswerArrives()
    {
        // A silent title has no audio or subtitle track for mpv to list, so the buttons were
        // never set and Audio opened an empty menu. They follow the server's answer, not mpv's list.
        var methods = Methods(Read("Pages", "PlayerPage.cs"));
        Assert.Matches(@"(?s)_playback = p;\s*ShowTrackButtons\(\);", methods.Single(m => m.Name == "StartAsync").Body);
        var show = methods.Single(m => m.Name == "ShowTrackButtons").Body;
        Assert.Contains("ClientRules.TrackButtons(", show);
        Assert.Contains("_audioButton.Visibility", show);
        Assert.Contains("_subtitleButton.Visibility", show);
    }

    [Fact]
    public void TheHeaderIsBuiltOnlyForADifferentTitle()
    {
        var (_, render) = Methods(Read("Pages", "DetailPage.cs")).Single(m => m.Name == "Render");
        Assert.Contains("headerKey != _headerFor", render);
    }

    [Fact]
    public void ThePlayerPlaysOnItsTitlePageAndGoesFullScreenFromThere()
    {
        // As the Mac, under the top bar, 16:9 up to 60 % of the page; full screen is the same player.
        var shell = Read("Shell.cs");
        var methods = Methods(shell).ToDictionary(m => m.Name, m => m.Body);
        Assert.Contains("ClientRules.InlinePlayerHeight(PageWidth, PageHeight - TopBarHeight)", methods["LayoutPlayer"]);
        Assert.Contains("_playerPage?.PlayerShown(full ? null : _player.Height)", methods["LayoutPlayer"]);
        Assert.Contains("LayoutPlayer()", methods["ToggleFullScreen"]);
        Assert.Contains("LayoutPlayer()", methods["LeaveFullScreen"]);
        Assert.Contains("LayoutPlayer()", methods["WindowStateMaybeChanged"]);
        Assert.Contains("LayoutPlayer();", shell.Split("_root.SizeChanged +=")[1].Split('\n')[0]);
        // Home's Play opens the title's page and plays there; leaving the page stops it.
        Assert.Contains("top is not DetailPage page", methods["Play"]);
        Assert.Contains("!ReferenceEquals(page, _playerPage)) ClosePlayer()", methods["ShowCurrent"]);
        // On its page the player is under the title bar's row: the row is the page's then.
        Assert.Contains("var covering = _player != null && IsFullScreen", methods["SetPassthrough"]);
        // Never moved between parents (that would unload it, and Unloaded stops it): only its place changes.
        Assert.Single(Regex.Matches(shell, @"F\.ShowOnly\(_playerHost, _player\)"));
        var detail = Read("Pages", "DetailPage.cs");
        Assert.Contains("if (!playing) _column.Children.Add(row)", detail);
        Assert.Contains("Shell.TopBarHeight + h", detail);
        var player = Read("Pages", "PlayerPage.cs");
        Assert.Contains("(_pointerInside || _fullScreen)", Methods(player).Single(m => m.Name == "ShowControls").Body);
        Assert.Contains("_shell.PlayerItemChanged(episode)", Methods(player).Single(m => m.Name == "PlayItem").Body);
    }

    [Fact]
    public void TheLibraryGridMakesOnlyTheCardsNearTheScreen()
    {
        // A library of many thousands; pages fetched where the grid is, a letter loads its own.
        var library = Read("Pages", "LibraryPage.cs");
        var methods = Methods(library).GroupBy(m => m.Name).ToDictionary(g => g.Key, g => string.Join("\n", g.Select(m => m.Body)));
        Assert.Contains("_window.Range(", methods["Realize"]);
        Assert.Contains("_canvas.Children.Remove(", methods["Realize"]);
        Assert.Contains("Fetch(first, last)", methods["Realize"]);
        Assert.Contains("_pages.Begin(first, last)", methods["Fetch"]);
        Assert.Contains("_window.RowTop(at)", methods["Jump"]);
        Assert.Contains("LetterIndex.Shows(", library);
        Assert.Contains("LibraryCategories.For(shows)", library);
        Assert.Contains("LibraryCategories.Entries(Category, _facets, query)", library);
        Assert.Contains("!_f.IsLocked(LockedDimension.Genre)", methods["FilterMenu"]);
    }

    [Fact]
    public void ANewServerIsSetUpAndAnInvitationJoinsFromSignIn()
    {
        var signIn = Read("Pages", "SignInPage.cs");
        var methods = Methods(signIn).ToDictionary(m => m.Name, m => m.Body);
        Assert.Contains("info?.Unclaimed == true", methods["ShowFor"]);
        Assert.Contains("info?.Has(\"invites\") == true", methods["ShowFor"]);
        Assert.Contains("new SetupFlowPage(_shell, api, info, null, ShowForm)", methods["StartSetup"]);
        Assert.Contains("new InvitePage(_shell, api, _info, ShowForm)", methods["OpenInvite"]);
        Assert.Contains("NoLibraries(api)", methods["SignIn"]); // an admin's setup left before its library goes on
        var setup = Read("Pages", "SetupPages.cs");
        Assert.Contains("_api.VerifySetupCode(digits)", setup);
        Assert.Contains("_api.Claim(_code,", setup);
        Assert.Contains("_api.CreateLibrary(", setup);
        Assert.Contains("api.RedeemInvite(", setup);
        // The server becomes the app's only when the setup is done.
        Assert.Single(Regex.Matches(Methods(setup).Single(m => m.Name == "Finish").Body + "", @"Session\.SignedIn"));
    }

    [Fact]
    public void TheSetupOffersANetworkShareAndExtrasAndStopsPollingWhenLeft()
    {
        // The Mac's five steps, a network share beside a server folder, and polling that ends with the step.
        var setup = Read("Pages", "SetupPages.cs");
        var methods = Methods(setup).ToDictionary(m => m.Name, m => m.Body);
        Assert.Contains("private const int Steps = 5;", setup);
        Assert.Contains("ShareServersStep", methods["LibraryStep"]);
        Assert.Contains("_api.CreateLibrary(_libraryName.Trim(), _root)", methods["LibraryStep"]);
        Assert.Contains("_api.Shares(host, login)", methods["Connect"]);
        Assert.Contains("_api.TestShare(", methods["ShareFolderStep"]);
        Assert.Contains("MainButton(\"Continue\", ExtrasStep)", methods["ScanStep"]);
        Assert.Contains("StopPolling()", methods["ExtrasStep"]);
        Assert.Contains("_api.UpdateSettings(c)", methods["ExtrasStep"]);
        Assert.Contains("Unloaded += (_, _) => StopPolling();", setup);
        Assert.Contains("_gate.Stop()", methods["StopPolling"]);
        // A poll's answer is checked against its round before it draws or polls again.
        var poll = methods["Poll"];
        Assert.True(poll.IndexOf("await _api.ScanProgress()") < poll.IndexOf("_gate.Continues(round)", poll.IndexOf("await _api.ScanProgress()")));
        Assert.DoesNotContain("_polling", setup);
    }
}
