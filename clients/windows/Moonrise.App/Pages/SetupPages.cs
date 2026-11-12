using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Moonrise.App.Player;
using Moonrise.App.Ui;
using Moonrise.Core;
using Windows.UI;

namespace Moonrise.App.Pages;

/// <summary>
/// Set Up This Server (Mac: SetupViews.swift): the setup code from the server's log or its page →
/// the admin account → the first library (a folder on the server's computer, or a network share;
/// a user name and password only when the share asks for one) → the first scan, live → optional
/// extras (own keys and accounts, all skippable) → Home. Signing in as an admin to a server
/// without libraries goes on at the library step. The app takes the server on only when the setup
/// is done, so Home opens on a library.
/// </summary>
public sealed class SetupFlowPage : Grid
{
    private const int Steps = 5;
    private static readonly SolidColorBrush Red = new(Color.FromArgb(255, 255, 105, 97));
    private static readonly SolidColorBrush Green = new(Color.FromArgb(255, 80, 200, 120));
    private readonly Shell _shell;
    private readonly MoonriseApi _api;
    private readonly ServerInfo _info;
    private readonly Action _back;
    private readonly DispatcherQueueTimer _poll;
    private readonly PollGate _gate = new();
    private AuthResult? _auth;
    private string _code = "";
    private int _passesBefore;
    private AdminLibrary? _library;
    private string _libraryName = "Media";
    private LibraryRoot? _root;
    private ScanWatch? _scan;
    private bool _onScan;
    private bool _finished;

    public SetupFlowPage(Shell shell, MoonriseApi api, ServerInfo info, AuthResult? auth, Action back)
    {
        _shell = shell;
        _api = api;
        _info = info;
        _auth = auth;
        _back = back;
        Background = Theme.Background;
        _poll = DispatcherQueue.CreateTimer();
        _poll.Interval = TimeSpan.FromSeconds(1);
        _poll.IsRepeating = false;
        _poll.Tick += Safe.Tick(() => _ = Poll(_gate.Round));
        // Leaving the page ends the polling, a request still out included; coming back to the scan step resumes it.
        Unloaded += (_, _) => StopPolling();
        Loaded += (_, _) => { if (_onScan && !_gate.On) StartPolling(); };
        if (auth != null) LibraryStep(); else CodeStep();
    }

    // ---------------------------------------------------------------- the pieces of a step

    /// <summary>A step's column: back (until the account exists), the step and what it's about, then its rows.</summary>
    private StackPanel Column(int step, string title, string message, Action? back)
    {
        var col = new StackPanel { Spacing = 10, Width = 460, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 60, 0, 40) };
        if (back != null)
        {
            var b = F.GlassCircle("", 40, 14, back, "Back");
            b.HorizontalAlignment = HorizontalAlignment.Left;
            b.Margin = new Thickness(0, 0, 0, 10);
            col.Children.Add(b);
        }
        col.Children.Add(F.Text($"STEP {step} OF {Steps}", Theme.FontCaption, F.SemiBold, Theme.Secondary));
        col.Children.Add(F.Text(title, Theme.FontLargeTitle, F.Bold));
        var m = F.Text(message, Theme.FontSubheadline, fg: Theme.Secondary);
        m.TextWrapping = TextWrapping.Wrap;
        m.Margin = new Thickness(0, 0, 0, 10);
        col.Children.Add(m);
        // Each step on a page of its own.
        F.ShowOnly(this, new ScrollViewer { Content = col, HorizontalScrollMode = ScrollMode.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto });
        return col;
    }

    private static TextBlock Label(string text)
    {
        var t = F.Text(text, Theme.FontSubheadline, F.SemiBold);
        t.Margin = new Thickness(0, 6, 0, 0);
        return t;
    }

    private static TextBlock Note(string text, Brush? fg = null)
    {
        var t = F.Text(text, Theme.FontCaption, fg: fg ?? Theme.Secondary);
        t.TextWrapping = TextWrapping.Wrap;
        return t;
    }

    private sealed record Main(Button Button, ProgressRing Busy, TextBlock Title);

    /// <summary>The step's action: a white capsule, with a spinner while it runs.</summary>
    private static Main MainButton(string title, Action run)
    {
        var label = new TextBlock { Text = title, Foreground = Theme.Black, FontWeight = F.SemiBold, FontSize = Theme.FontHeadline, VerticalAlignment = VerticalAlignment.Center };
        var busy = new ProgressRing { IsActive = false, Width = 16, Height = 16, Foreground = Theme.Black, Visibility = Visibility.Collapsed };
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10, HorizontalAlignment = HorizontalAlignment.Center };
        row.Children.Add(label);
        row.Children.Add(busy);
        var border = F.Capsule(row, Theme.White, 44, 28);
        border.Width = 460;
        var b = F.Bare(border, run);
        b.Margin = new Thickness(0, 10, 0, 0);
        return new Main(b, busy, label);
    }

    /// <summary>A second action under the main one (Skip, Other File Server): a quiet capsule.</summary>
    private static Button SecondButton(string title, Action run)
    {
        var label = new TextBlock { Text = title, Foreground = Theme.White, FontWeight = F.SemiBold, FontSize = Theme.FontHeadline, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        var border = F.Capsule(label, Theme.Surface, 44, 28);
        border.Width = 460;
        return F.Bare(border, run);
    }

    private static void Busy(Main a, bool on)
    {
        a.Button.IsEnabled = !on;
        a.Busy.IsActive = on;
        a.Busy.Visibility = on ? Visibility.Visible : Visibility.Collapsed;
    }

    // ---------------------------------------------------------------- 1: the setup code

    private void CodeStep()
    {
        var col = Column(1, "Set Up Server", SetupRules.CodeMessage(_api.BaseUrl), _back);
        col.Children.Add(Label("Setup Code"));
        var box = new TextBox { PlaceholderText = "123 456", Text = _code, Width = 460, FontSize = 20, FontFamily = new FontFamily("Cascadia Mono, Consolas") };
        col.Children.Add(box);
        col.Children.Add(Note("The code changes when the server restarts."));
        var error = Note("", Red);
        Main go = null!;
        async Task Verify()
        {
            var digits = SetupRules.CodeDigits(box.Text);
            if (digits.Length != 6) { error.Text = "Enter the 6-digit code."; return; }
            Busy(go, true);
            try
            {
                await _api.VerifySetupCode(digits);
                _code = digits;
                AccountStep();
            }
            catch (MoonriseHttpException e) { error.Text = SetupRules.VerifyError(e); }
            catch (Exception e) { error.Text = "Can't reach that server. " + e.Message; }
            finally { Busy(go, false); }
        }
        go = MainButton("Continue", () => _ = Verify());
        // Six digits typed (or pasted): checked at once.
        box.TextChanged += (_, _) =>
        {
            var d = SetupRules.CodeDigits(box.Text);
            if (d.Length == 6 && d != _code && go.Button.IsEnabled) _ = Verify();
        };
        box.KeyDown += (_, e) => { if (e.Key == Windows.System.VirtualKey.Enter) _ = Verify(); };
        col.Children.Add(go.Button);
        col.Children.Add(error);
        box.Loaded += (_, _) => box.Focus(FocusState.Programmatic);
    }

    // ---------------------------------------------------------------- 2: the admin account

    private void AccountStep()
    {
        var col = Column(2, "Your Account", "Create the server's first account. It's the admin: it manages the server from this app. Use it to sign in on your other devices too.", CodeStep);
        col.Children.Add(Label("Admin Account"));
        var user = new TextBox { PlaceholderText = "User name", Width = 460 };
        var pass = new PasswordBox { PlaceholderText = "Password", Width = 460 };
        var confirm = new PasswordBox { PlaceholderText = "Repeat password", Width = 460 };
        col.Children.Add(user);
        col.Children.Add(pass);
        col.Children.Add(confirm);
        var hint = Note("At least 8 characters.");
        col.Children.Add(hint);
        var error = Note("", Red);
        void Check()
        {
            var p = SetupRules.AccountProblem(user.Text, pass.Password, confirm.Password);
            hint.Text = p is { Length: > 0 } && pass.Password.Length > 0 ? p : "At least 8 characters.";
        }
        user.TextChanged += (_, _) => Check();
        pass.PasswordChanged += (_, _) => Check();
        confirm.PasswordChanged += (_, _) => Check();
        Main go = null!;
        async Task Create()
        {
            if (SetupRules.AccountProblem(user.Text, pass.Password, confirm.Password) is { } p) { error.Text = p.Length > 0 ? p : "Fill in all three."; return; }
            Busy(go, true);
            try
            {
                _auth = await _api.Claim(_code, user.Text.Trim(), pass.Password);
                LibraryStep();
            }
            catch (MoonriseHttpException e) { error.Text = SetupRules.ClaimError(e); }
            catch (Exception e) { error.Text = "Can't reach that server. " + e.Message; }
            finally { Busy(go, false); }
        }
        go = MainButton("Create Account", () => _ = Create());
        confirm.KeyDown += (_, e) => { if (e.Key == Windows.System.VirtualKey.Enter) _ = Create(); };
        col.Children.Add(go.Button);
        col.Children.Add(error);
        user.Loaded += (_, _) => user.Focus(FocusState.Programmatic);
    }

    // ---------------------------------------------------------------- 3: the first library

    private void LibraryStep()
    {
        var col = Column(3, "Add Your Videos", SetupRules.LibraryMessage, null);
        col.Children.Add(Label("Library Name"));
        var name = new TextBox { Text = _libraryName, Width = 460 };
        name.TextChanged += (_, _) => _libraryName = name.Text;
        col.Children.Add(name);
        col.Children.Add(Label("Where Are the Videos?"));
        col.Children.Add(PickRow("", "A Folder on the Server", "Folders the server's computer can see", _root?.Source == RootSource.Local, () => FolderStep(null)));
        col.Children.Add(PickRow("", "A Network Share", "A NAS or another computer on your network", _root?.Source == RootSource.Smb, ShareServersStep));
        if (_root != null)
        {
            var chosen = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            chosen.Children.Add(F.Glyph("", 13, Green));
            var t = F.Text(_root.Display, Theme.FontSubheadline, fg: Green, trim: TextTrimming.CharacterEllipsis);
            t.MaxWidth = 430;
            chosen.Children.Add(t);
            col.Children.Add(chosen);
        }
        var error = Note("", Red);
        Main go = null!;
        async Task Add()
        {
            if (_root == null) { error.Text = "Choose the folder of your videos first."; return; }
            if (_libraryName.Trim().Length == 0) { error.Text = "Give the library a name."; return; }
            Busy(go, true);
            try
            {
                try { _passesBefore = (await _api.ScanProgress()).Passes; } catch { _passesBefore = 0; }
                _library = await _api.CreateLibrary(_libraryName.Trim(), _root);
                ScanStep();
            }
            catch (MoonriseHttpException e) { error.Text = SetupRules.ServerText(e); }
            catch (Exception e) { error.Text = "Can't reach that server. " + e.Message; }
            finally { Busy(go, false); }
        }
        go = MainButton("Add Library", () => _ = Add());
        col.Children.Add(go.Button);
        col.Children.Add(error);
    }

    private static FrameworkElement PickRow(string glyph, string title, string detail, bool chosen, Action open)
    {
        var grid = new Grid { ColumnSpacing = 14, Padding = new Thickness(14, 10, 14, 10) };
        grid.ColumnDefinitions.Add(F.Auto);
        grid.ColumnDefinitions.Add(F.Star());
        grid.ColumnDefinitions.Add(F.Auto);
        grid.Children.Add(new Border { Width = 30, Child = F.Glyph(glyph, 18) }.At(0, 0));
        var text = new StackPanel { Spacing = 2 };
        text.Children.Add(F.Text(title, Theme.FontBody, trim: TextTrimming.CharacterEllipsis));
        text.Children.Add(F.Text(detail, Theme.FontCaption, fg: Theme.Secondary, trim: TextTrimming.CharacterEllipsis));
        grid.Children.Add(text.At(0, 1));
        var end = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10, VerticalAlignment = VerticalAlignment.Center };
        if (chosen) end.Children.Add(F.Glyph("", 13, Green));
        end.Children.Add(F.Glyph("", 11, Theme.Secondary));
        grid.Children.Add(end.At(0, 2));
        var b = F.Bare(new Border { Background = Theme.Surface, CornerRadius = new CornerRadius(10), Child = grid }, open, title);
        b.HorizontalAlignment = HorizontalAlignment.Stretch;
        b.HorizontalContentAlignment = HorizontalAlignment.Stretch;
        return b;
    }

    /// <summary>The server's folders (Mac: ServerFolderPicker): open the one with the videos, then use it.</summary>
    private void FolderStep(string? path)
    {
        var col = Column(3, path == null ? "Server Folders" : SetupRules.FolderName(path),
            path == null ? "Folders on the server's computer. Open the one that holds your videos, then choose it." : path, LibraryStep);
        var list = new StackPanel { Spacing = 2 };
        var status = Note("");
        col.Children.Add(list);
        col.Children.Add(status);
        list.Children.Add(new ProgressRing { IsActive = true, Width = 24, Height = 24, HorizontalAlignment = HorizontalAlignment.Left });
        async Task Load()
        {
            try
            {
                var listing = await _api.BrowseServer(path);
                list.Children.Clear();
                if (path != null)
                {
                    var use = MainButton("Use This Folder", () => { _root = LibraryRoot.Local(path); LibraryStep(); });
                    use.Button.Margin = new Thickness(0, 0, 0, 4);
                    list.Children.Add(use.Button);
                    var here = Note(SetupRules.VideosText(listing.VideoFiles, listing.Dirs.Count));
                    here.Margin = new Thickness(0, 0, 0, 12);
                    list.Children.Add(here);
                    list.Children.Add(FolderRow("", "Up", () => FolderStep(listing.Parent)));
                }
                foreach (var d in listing.Dirs) list.Children.Add(FolderRow("", d.Name, () => FolderStep(d.Path)));
                if (listing.Dirs.Count == 0) status.Text = path == null ? SetupRules.NoServerFolders : "No folders in here.";
            }
            catch (MoonriseHttpException e) { list.Children.Clear(); status.Foreground = Red; status.Text = SetupRules.ServerText(e); }
            catch (Exception e) { list.Children.Clear(); status.Foreground = Red; status.Text = "Can't reach that server. " + e.Message; }
        }
        _ = Load();
    }

    private static FrameworkElement FolderRow(string glyph, string name, Action open, bool chevron = true)
    {
        var grid = new Grid { ColumnSpacing = 12, Padding = new Thickness(14, 0, 14, 0), Height = 44 };
        grid.ColumnDefinitions.Add(F.Auto);
        grid.ColumnDefinitions.Add(F.Star());
        grid.ColumnDefinitions.Add(F.Auto);
        grid.Children.Add(new Border { Width = 24, Child = F.Glyph(glyph, 15) }.At(0, 0));
        var t = F.Text(name, Theme.FontBody, trim: TextTrimming.CharacterEllipsis);
        t.VerticalAlignment = VerticalAlignment.Center;
        grid.Children.Add(t.At(0, 1));
        if (chevron) grid.Children.Add(F.Glyph("", 11, Theme.Secondary).At(0, 2));
        var b = F.Bare(new Border { Background = Theme.Surface, CornerRadius = new CornerRadius(10), Child = grid }, open, name);
        b.HorizontalAlignment = HorizontalAlignment.Stretch;
        b.HorizontalContentAlignment = HorizontalAlignment.Stretch;
        return b;
    }

    // ---------------------------------------------------------------- 3: a network share

    // The server reads the share (Mac: SharePicker): file servers it sees, or an address → shares →
    // folders. A user name and password only when the share turns guests away.
    private string _shareAddress = "";
    private ShareLogin? _shareLogin;
    /// <summary>The user name typed for a share, kept when the account is turned down.</summary>
    private string _shareUser = "";
    private IReadOnlyList<Share> _shares = [];
    /// <summary>The share and folder that asked for an account, opened again once signed in.</summary>
    private (string Share, string Path)? _pending;
    private bool _shareBusy;

    private void ShareServersStep()
    {
        _pending = null;
        var col = Column(3, "Network Share", ShareRules.ServersMessage, LibraryStep);
        col.Children.Add(Label("On the Network"));
        var found = new StackPanel { Spacing = 2 };
        var looking = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };
        looking.Children.Add(new ProgressRing { IsActive = true, Width = 16, Height = 16 });
        looking.Children.Add(Note("Looking for file servers…"));
        found.Children.Add(looking);
        col.Children.Add(found);
        col.Children.Add(Label("Or Type Its Address"));
        var address = new TextBox { PlaceholderText = "e.g. 192.168.1.20 or nas.local", Text = _shareAddress, Width = 460 };
        address.TextChanged += (_, _) => _shareAddress = address.Text;
        col.Children.Add(address);
        var error = Note("", Red);
        Main go = null!;
        go = MainButton("Connect", () => _ = Connect(address.Text, null, go, error));
        address.KeyDown += (_, e) => { if (e.Key == Windows.System.VirtualKey.Enter) _ = Connect(address.Text, null, go, error); };
        col.Children.Add(go.Button);
        col.Children.Add(error);
        async Task Discover()
        {
            IReadOnlyList<FileServer> servers;
            try { servers = await _api.DiscoverFileServers(); }
            catch (Exception e) { AppLog.Write("setup: discover file servers: " + e.Message); servers = []; }
            found.Children.Clear();
            foreach (var s in servers) found.Children.Add(PickRow("", s.Name, s.Host, false, () => _ = Connect(s.Host, null, go, error)));
            if (servers.Count == 0) found.Children.Add(Note("None found on the network."));
        }
        _ = Discover();
    }

    /// <summary>A file server's shares, as a guest or with an account; the step it leads to.</summary>
    private async Task Connect(string typed, ShareLogin? login, Main go, TextBlock error)
    {
        var host = typed.Trim();
        if (host.Length == 0) { error.Text = "Type the file server's name or address."; return; }
        if (_shareBusy) return;
        _shareBusy = true;
        Busy(go, true);
        try
        {
            _shares = await _api.Shares(host, login);
            _shareLogin = login;
            if (_pending is { } p) { _pending = null; ShareFolderStep(host, p.Share, p.Path); }
            else SharesStep(host);
        }
        catch (MoonriseHttpException e)
        {
            var o = ShareRules.ConnectFailure(e, host);
            if (o.AskLogin) ShareLoginStep(host, o.Error); else error.Text = o.Error ?? "";
        }
        catch (Exception e) { error.Text = "Can't reach that server. " + e.Message; }
        finally { _shareBusy = false; Busy(go, false); }
    }

    private void ShareLoginStep(string host, string? problem)
    {
        var col = Column(3, host, ShareRules.LoginMessage, ShareServersStep);
        col.Children.Add(Label($"Account on {host}"));
        var user = new TextBox { PlaceholderText = "User name", Width = 460, Text = _shareUser };
        user.TextChanged += (_, _) => _shareUser = user.Text;
        var pass = new PasswordBox { PlaceholderText = "Password", Width = 460 };
        col.Children.Add(user);
        col.Children.Add(pass);
        var error = Note(problem ?? "", Red);
        Main go = null!;
        void Go()
        {
            if (user.Text.Trim().Length == 0) { error.Text = "Enter the user name."; return; }
            _ = Connect(host, new ShareLogin(user.Text.Trim(), pass.Password), go, error);
        }
        go = MainButton("Connect", Go);
        pass.KeyDown += (_, e) => { if (e.Key == Windows.System.VirtualKey.Enter) Go(); };
        col.Children.Add(go.Button);
        col.Children.Add(error);
        col.Children.Add(SecondButton("Other File Server", ShareServersStep));
        user.Loaded += (_, _) => (user.Text.Length == 0 ? (Control)user : pass).Focus(FocusState.Programmatic);
    }

    private void SharesStep(string host)
    {
        var col = Column(3, host, ShareRules.SharesMessage, ShareServersStep);
        col.Children.Add(Label("Shares"));
        foreach (var s in _shares) col.Children.Add(FolderRow("", s.Name, () => ShareFolderStep(host, s.Name, "")));
        if (_shares.Count == 0) col.Children.Add(Note("This file server shows no shares."));
        var other = SecondButton("Other File Server", ShareServersStep);
        other.Margin = new Thickness(0, 10, 0, 0);
        col.Children.Add(other);
    }

    /// <summary>A folder in a share: open the one with the videos, then use it (the server checks it first).</summary>
    private void ShareFolderStep(string host, string share, string path)
    {
        void Up() { if (ShareRules.Up(path) is { } parent) ShareFolderStep(host, share, parent); else SharesStep(host); }
        var col = Column(3, ShareRules.FolderTitle(share, path), ShareRules.Breadcrumb(host, share, path), Up);
        var list = new StackPanel { Spacing = 2 };
        var status = Note("", Red);
        col.Children.Add(list);
        col.Children.Add(status);
        list.Children.Add(new ProgressRing { IsActive = true, Width = 24, Height = 24, HorizontalAlignment = HorizontalAlignment.Left });
        Main use = null!;
        async Task Use()
        {
            if (_shareBusy) return;
            _shareBusy = true;
            Busy(use, true);
            try
            {
                var t = await _api.TestShare(host, share, path, _shareLogin);
                if (ShareRules.TestProblem(t) is { } p) { status.Text = p; return; }
                _root = new LibraryRoot(RootSource.Smb, path, host, share, _shareLogin, t.VideoFiles);
                LibraryStep();
            }
            catch (MoonriseHttpException e) { status.Text = ShareRules.Text(e); }
            catch (Exception e) { status.Text = "Can't reach that server. " + e.Message; }
            finally { _shareBusy = false; Busy(use, false); }
        }
        async Task Load()
        {
            try
            {
                var listing = await _api.BrowseShare(host, share, path, _shareLogin);
                list.Children.Clear();
                use = MainButton("Use This Folder", () => _ = Use());
                use.Button.Margin = new Thickness(0, 0, 0, 4);
                list.Children.Add(use.Button);
                var here = Note(SetupRules.VideosText(listing.VideoFiles, listing.Dirs.Count));
                here.Margin = new Thickness(0, 0, 0, 12);
                list.Children.Add(here);
                list.Children.Add(FolderRow("", path.Trim('/').Length == 0 ? "Shares" : "Up", Up, chevron: false));
                foreach (var d in listing.Dirs) list.Children.Add(FolderRow("", d.Name, () => ShareFolderStep(host, share, d.Path)));
            }
            catch (MoonriseHttpException e)
            {
                list.Children.Clear();
                var o = ShareRules.BrowseFailure(e, _shareLogin != null);
                if (o.AskLogin) { _pending = (share, path); ShareLoginStep(host, o.Error); return; }
                status.Text = o.Error ?? "";
                list.Children.Add(FolderRow("", path.Trim('/').Length == 0 ? "Shares" : "Up", Up, chevron: false));
            }
            catch (Exception e) { list.Children.Clear(); status.Text = "Can't reach that server. " + e.Message; }
        }
        _ = Load();
    }

    // ---------------------------------------------------------------- 4: the first scan

    private StackPanel? _stats;
    private TextBlock? _phase;
    private ProgressRing? _phaseRing;
    private TextBlock? _phaseDone;
    private StackPanel? _notes;

    private void ScanStep()
    {
        _scan = new ScanWatch(_passesBefore, _library?.Id);
        var col = Column(4, _library?.Name ?? "Scanning", "Moonrise is looking through the folder and recognizing your movies and shows.", null);
        var stats = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 36, Margin = new Thickness(0, 6, 0, 6) };
        _stats = stats;
        col.Children.Add(stats);
        var phaseRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        var ring = new ProgressRing { IsActive = true, Width = 18, Height = 18 };
        var done = F.Glyph("", 16, Green);
        done.Visibility = Visibility.Collapsed;
        var phase = F.Text("", Theme.FontBody);
        (_phaseRing, _phaseDone, _phase) = (ring, done, phase);
        phaseRow.Children.Add(ring);
        phaseRow.Children.Add(done);
        phaseRow.Children.Add(phase);
        col.Children.Add(phaseRow);
        var notes = new StackPanel { Spacing = 6 };
        _notes = notes;
        col.Children.Add(notes);
        col.Children.Add(MainButton("Continue", ExtrasStep).Button);
        DrawScan();
        _onScan = true;
        StartPolling();
    }

    private void StartPolling()
    {
        _poll.Stop();
        _ = Poll(_gate.Start());
    }

    private void StopPolling()
    {
        _gate.Stop();
        _poll.Stop();
    }

    /// <summary>One poll of a round: an answer that comes after the round ended (the step or the page left) is dropped and polls no more.</summary>
    private async Task Poll(int round)
    {
        if (!_gate.Continues(round) || _scan == null) return;
        try
        {
            var p = await _api.ScanProgress();
            if (!_gate.Continues(round)) return;
            _scan.Update(p);
            DrawScan();
        }
        catch (Exception e) { AppLog.Write("setup: scan progress: " + e.Message); }
        if (!_gate.Continues(round)) return;
        _poll.Interval = TimeSpan.FromSeconds(_scan.Done ? 5 : 1);
        _poll.Start();
    }

    private void DrawScan()
    {
        if (_scan == null || _stats == null) return;
        _stats.Children.Clear();
        foreach (var (count, label) in _scan.Stats)
        {
            var s = new StackPanel { Spacing = 4 };
            s.Children.Add(F.Text(count.ToString(), 44, F.Bold));
            s.Children.Add(F.Text(label, Theme.FontSubheadline, fg: Theme.Secondary));
            _stats.Children.Add(s);
        }
        _phase!.Text = _scan.PhaseText;
        _phaseRing!.IsActive = !_scan.Done;
        _phaseRing.Visibility = _scan.Done ? Visibility.Collapsed : Visibility.Visible;
        _phaseDone!.Visibility = _scan.Done ? Visibility.Visible : Visibility.Collapsed;
        _notes!.Children.Clear();
        foreach (var n in _scan.Notes) _notes.Children.Add(Note(n));
    }

    // ---------------------------------------------------------------- 5: optional extras

    /// <summary>Own keys and accounts (Mac: SetupExtrasView): all optional; Finish saves what was filled in.</summary>
    private void ExtrasStep()
    {
        _onScan = false;
        StopPolling();
        var col = Column(5, "Extras", ExtrasRules.Message, null);
        var tmdb = new PasswordBox { PlaceholderText = "Your TMDB API key (optional)", Width = 460 };
        var osUser = new TextBox { PlaceholderText = "OpenSubtitles user name (optional)", Width = 460 };
        var osPass = new PasswordBox { PlaceholderText = "OpenSubtitles password", Width = 460 };
        var llm = new PasswordBox { PlaceholderText = "Anthropic API key (optional)", Width = 460 };
        Dictionary<string, string> Changes() => ExtrasRules.Changes(tmdb.Password, osUser.Text, osPass.Password, llm.Password);
        var services = new StackPanel { Spacing = 10 };
        services.Children.Add(new ProgressRing { IsActive = true, Width = 24, Height = 24, HorizontalAlignment = HorizontalAlignment.Left });
        col.Children.Add(services);
        var error = Note("", Red);
        col.Children.Add(error);
        Main go = null!;
        var skip = SecondButton("Skip", Finish);
        skip.Visibility = Visibility.Collapsed;
        void Changed()
        {
            var n = Changes().Count;
            go.Title.Text = ExtrasRules.FinishTitle(n);
            skip.Visibility = n > 0 ? Visibility.Visible : Visibility.Collapsed;
        }
        async Task Save()
        {
            var c = Changes();
            if (c.Count == 0) { Finish(); return; }
            Busy(go, true);
            try
            {
                await _api.UpdateSettings(c);
                Finish();
            }
            catch (MoonriseHttpException e) { error.Text = SetupRules.ServerText(e); }
            catch (Exception e) { error.Text = "Can't reach that server. " + e.Message; }
            finally { Busy(go, false); }
        }
        go = MainButton(ExtrasRules.FinishTitle(0), () => _ = Save());
        col.Children.Add(go.Button);
        col.Children.Add(skip);
        tmdb.PasswordChanged += (_, _) => Changed();
        osUser.TextChanged += (_, _) => Changed();
        osPass.PasswordChanged += (_, _) => Changed();
        llm.PasswordChanged += (_, _) => Changed();
        async Task Load()
        {
            try
            {
                var list = await _api.Services();
                services.Children.Clear();
                foreach (var s in list)
                {
                    services.Children.Add(Label(s.Name));
                    services.Children.Add(ServiceSummary(s));
                    if (!ExtrasRules.HasFields(s)) continue;
                    switch (s.Id)
                    {
                        case "tmdb": services.Children.Add(tmdb); break;
                        case "opensubtitles": services.Children.Add(osUser); services.Children.Add(osPass); break;
                        case "llm": services.Children.Add(llm); break;
                    }
                }
            }
            catch (MoonriseHttpException e) { services.Children.Clear(); error.Text = SetupRules.ServerText(e); }
            catch (Exception e) { services.Children.Clear(); error.Text = "Can't reach that server. " + e.Message; }
        }
        _ = Load();
    }

    /// <summary>A service's state in a line, then what Moonrise does with it and without it (Mac: ServiceSummary).</summary>
    private static FrameworkElement ServiceSummary(Service s)
    {
        var p = new StackPanel { Spacing = 6 };
        var state = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        state.Children.Add(F.Glyph(ExtrasRules.Glyph(s.State), 14));
        var summary = F.Text(s.Summary, Theme.FontBody);
        summary.TextWrapping = TextWrapping.Wrap;
        summary.MaxWidth = 436;
        state.Children.Add(summary);
        p.Children.Add(state);
        foreach (var (head, text) in new[] { ("With it: ", s.With), ("Without it: ", s.Without) })
        {
            var t = Note("");
            t.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = head, FontWeight = F.SemiBold });
            t.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = text });
            p.Children.Add(t);
        }
        return p;
    }

    /// <summary>Done: this is the app's server now (Home opens). The scan goes on in the background.</summary>
    private void Finish()
    {
        _onScan = false;
        StopPolling();
        if (_auth == null || _finished) return;
        _finished = true;
        _shell.Session.SignedIn(_api, _auth);
        _shell.SignedIn();
    }
}

/// <summary>Have an Invitation (Mac: InviteSignInView): the code from an admin, then your own user name and password.</summary>
public sealed class InvitePage : Grid
{
    public InvitePage(Shell shell, MoonriseApi api, ServerInfo info, Action back)
    {
        Background = Theme.Background;
        var col = new StackPanel { Spacing = 10, Width = 460, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 60, 0, 40) };
        var b = F.GlassCircle("\uE72B", 40, 14, back, "Back");
        b.HorizontalAlignment = HorizontalAlignment.Left;
        b.Margin = new Thickness(0, 0, 0, 10);
        col.Children.Add(b);
        col.Children.Add(F.Text("Have an Invitation", Theme.FontLargeTitle, F.Bold));
        var m = F.Text($"Enter the invitation code an admin of {info.Name} gave you, then pick your user name and password.", Theme.FontSubheadline, fg: Theme.Secondary);
        m.TextWrapping = TextWrapping.Wrap;
        m.Margin = new Thickness(0, 0, 0, 10);
        col.Children.Add(m);

        TextBlock Label(string t) { var l = F.Text(t, Theme.FontSubheadline, F.SemiBold); l.Margin = new Thickness(0, 6, 0, 0); return l; }
        col.Children.Add(Label("Invitation Code"));
        var code = new TextBox { PlaceholderText = "XXXX-XXXX", Width = 460, FontSize = 18, FontFamily = new FontFamily("Cascadia Mono, Consolas"), CharacterCasing = CharacterCasing.Upper };
        col.Children.Add(code);
        col.Children.Add(F.Text($"From an admin of {info.Name}.", Theme.FontCaption, fg: Theme.Secondary));
        col.Children.Add(Label("Your Account"));
        var user = new TextBox { PlaceholderText = "User name", Width = 460 };
        var pass = new PasswordBox { PlaceholderText = "Password (at least 8 characters)", Width = 460 };
        col.Children.Add(user);
        col.Children.Add(pass);
        var hint = F.Text("Use these to sign in on your other devices too.", Theme.FontCaption, fg: Theme.Secondary);
        hint.TextWrapping = TextWrapping.Wrap;
        col.Children.Add(hint);
        void Check()
        {
            var p = SetupRules.InviteProblem(code.Text, user.Text, pass.Password);
            hint.Text = p is { Length: > 0 } && pass.Password.Length > 0 ? p : "Use these to sign in on your other devices too.";
        }
        code.TextChanged += (_, _) => Check();
        user.TextChanged += (_, _) => Check();
        pass.PasswordChanged += (_, _) => Check();

        var error = F.Text("", Theme.FontCaption, fg: new SolidColorBrush(Color.FromArgb(255, 255, 105, 97)));
        error.TextWrapping = TextWrapping.Wrap;
        var label = new TextBlock { Text = $"Join {info.Name}", Foreground = Theme.Black, FontWeight = F.SemiBold, FontSize = Theme.FontHeadline, VerticalAlignment = VerticalAlignment.Center };
        var busy = new ProgressRing { IsActive = false, Width = 16, Height = 16, Foreground = Theme.Black, Visibility = Visibility.Collapsed };
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10, HorizontalAlignment = HorizontalAlignment.Center };
        row.Children.Add(label);
        row.Children.Add(busy);
        var capsule = F.Capsule(row, Theme.White, 44, 28);
        capsule.Width = 460;
        Button join = null!;
        async Task Join()
        {
            if (SetupRules.InviteProblem(code.Text, user.Text, pass.Password) is { } p) { error.Text = p.Length > 0 ? p : "Fill in the code, a user name and a password."; return; }
            join.IsEnabled = false;
            busy.IsActive = true;
            busy.Visibility = Visibility.Visible;
            try
            {
                var auth = await api.RedeemInvite(ClientRules.FormatDeviceCode(code.Text), user.Text.Trim(), pass.Password);
                shell.Session.SignedIn(api, auth);
                shell.SignedIn();
            }
            catch (MoonriseHttpException e) { error.Text = SetupRules.InviteError(e); }
            catch (Exception e) { error.Text = "Can't reach that server. " + e.Message; }
            finally
            {
                join.IsEnabled = true;
                busy.IsActive = false;
                busy.Visibility = Visibility.Collapsed;
            }
        }
        join = F.Bare(capsule, () => _ = Join());
        join.Margin = new Thickness(0, 10, 0, 0);
        pass.KeyDown += (_, e) => { if (e.Key == Windows.System.VirtualKey.Enter) _ = Join(); };
        col.Children.Add(join);
        col.Children.Add(error);
        Children.Add(new ScrollViewer { Content = col, HorizontalScrollMode = ScrollMode.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto });
        code.Loaded += (_, _) => code.Focus(FocusState.Programmatic);
    }
}
