using System.Net;
using System.Net.NetworkInformation;
using System.Net.Sockets;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Moonrise.App.Ui;
using Moonrise.Core;
using Windows.UI;

namespace Moonrise.App.Pages;

/// <summary>
/// Sign-in: the same flow as the other apps (a server found on the network or typed in, then a
/// password or a code). A new server is set up from here, and an invitation makes an account.
/// </summary>
public sealed class SignInPage : Grid
{
    private readonly Shell _shell;
    private readonly TextBox _address = new() { PlaceholderText = "e.g. 192.168.1.10", Width = 360 };
    private readonly TextBox _user = new() { PlaceholderText = "Username", Width = 360 };
    private readonly PasswordBox _password = new() { PlaceholderText = "Password", Width = 360 };
    private readonly TextBlock _status = F.Text("", Theme.FontCaption, fg: Theme.Secondary);
    private readonly TextBlock _serverLine = F.Text("", Theme.FontCaption);
    private readonly StackPanel _found = new() { Spacing = 4 };
    private readonly Button _signIn;
    private readonly Button _codeButton;
    private readonly StackPanel _form = new() { Spacing = 10, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, MaxWidth = 460, Margin = new Thickness(0, 40, 0, 40) };
    private readonly ScrollViewer _formScroll = new() { HorizontalScrollMode = ScrollMode.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
    private readonly Grid _host = new();
    // An account on the server (password or code), or setting up a new one.
    private readonly StackPanel _signInSection = new() { Spacing = 10 };
    private readonly StackPanel _setupSection = new() { Spacing = 10, Visibility = Visibility.Collapsed };
    private readonly Button _inviteButton;
    private CancellationTokenSource? _check;
    private CancellationTokenSource? _codeWait;
    private MoonriseApi? _api;
    private ServerInfo? _info;

    public SignInPage(Shell shell)
    {
        _shell = shell;
        Background = Theme.Background;

        var logoPath = Path.Combine(AppContext.BaseDirectory, "Assets", AppInfo.IconFile + ".png");
        if (File.Exists(logoPath))
            _form.Children.Add(new Border
            {
                Width = 88, Height = 88, CornerRadius = new CornerRadius(20), HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 0, 0, 6),
                Child = new Image { Source = new Microsoft.UI.Xaml.Media.Imaging.BitmapImage(new Uri(logoPath)), Stretch = Stretch.UniformToFill },
            });
        var welcome = F.Text("Welcome to Moonrise", Theme.FontLargeTitle, F.Bold);
        welcome.HorizontalAlignment = HorizontalAlignment.Center;
        _form.Children.Add(welcome);
        var sub = F.Text("Connect to your Moonrise server to sign in.", Theme.FontSubheadline, fg: Theme.Secondary);
        sub.HorizontalAlignment = HorizontalAlignment.Center;
        sub.Margin = new Thickness(0, 0, 0, 14);
        _form.Children.Add(sub);

        var serverLabel = F.Text("Server", Theme.FontSubheadline, F.SemiBold);
        _form.Children.Add(serverLabel);
        _address.TextChanged += (_, _) => CheckAddress(_address.Text);
        _form.Children.Add(_address);
        _form.Children.Add(_serverLine);

        var network = F.Text("On Your Network", Theme.FontSubheadline, F.SemiBold);
        network.Margin = new Thickness(0, 8, 0, 0);
        _form.Children.Add(network);
        _found.Children.Add(F.Text("Looking for servers…", Theme.FontCaption, fg: Theme.Secondary));
        _form.Children.Add(_found);

        var pw = F.Text("Or Sign In with Password", Theme.FontSubheadline, F.SemiBold);
        pw.Margin = new Thickness(0, 12, 0, 0);
        _signInSection.Children.Add(pw);
        _signInSection.Children.Add(_user);
        _signInSection.Children.Add(_password);
        _password.KeyDown += (_, e) => { if (e.Key == Windows.System.VirtualKey.Enter) _ = SignIn(); };

        _signIn = F.Bare(PrimaryButton("Sign In"), () => _ = SignIn());
        _signIn.Margin = new Thickness(0, 6, 0, 0);
        _signInSection.Children.Add(_signIn);

        var codeLabel = F.Text("Sign In with a Code", Theme.FontSubheadline, fg: Theme.Secondary);
        codeLabel.HorizontalAlignment = HorizontalAlignment.Center;
        _codeButton = F.Bare(codeLabel, () => _ = SignInWithCode());
        _codeButton.HorizontalAlignment = HorizontalAlignment.Center;
        _signInSection.Children.Add(_codeButton);

        // Mac: "Have an Invitation?", for a server that offers invitations.
        var invite = new StackPanel { Spacing = 2, HorizontalAlignment = HorizontalAlignment.Center };
        var inviteTitle = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center };
        inviteTitle.Children.Add(F.Glyph("\uE715", 14));
        inviteTitle.Children.Add(F.Text("Have an Invitation?", Theme.FontSubheadline, F.SemiBold));
        invite.Children.Add(inviteTitle);
        var inviteNote = F.Text("New here? With a code from an admin of this server, pick your own user name and password.", Theme.FontCaption, fg: Theme.Secondary);
        inviteNote.TextWrapping = TextWrapping.Wrap;
        inviteNote.TextAlignment = TextAlignment.Center;
        inviteNote.MaxWidth = 360;
        invite.Children.Add(inviteNote);
        _inviteButton = F.Bare(invite, OpenInvite, "Have an Invitation?");
        _inviteButton.HorizontalAlignment = HorizontalAlignment.Center;
        _inviteButton.Margin = new Thickness(0, 10, 0, 0);
        _inviteButton.Visibility = Visibility.Collapsed;
        _signInSection.Children.Add(_inviteButton);
        _form.Children.Add(_signInSection);

        // A server that isn't set up yet: its setup instead of a sign-in (Mac: "New Server").
        var newServer = F.Text("New Server", Theme.FontSubheadline, F.SemiBold);
        newServer.Margin = new Thickness(0, 12, 0, 0);
        _setupSection.Children.Add(newServer);
        var note = F.Text(SetupRules.UnclaimedNote, Theme.FontCaption, fg: Theme.Secondary);
        note.TextWrapping = TextWrapping.Wrap;
        note.Width = 360;
        note.HorizontalAlignment = HorizontalAlignment.Left;
        _setupSection.Children.Add(note);
        var setup = F.Bare(PrimaryButton("Set Up This Server"), StartSetup);
        setup.Margin = new Thickness(0, 6, 0, 0);
        _setupSection.Children.Add(setup);
        _form.Children.Add(_setupSection);

        _status.HorizontalAlignment = HorizontalAlignment.Center;
        _status.TextWrapping = TextWrapping.Wrap;
        _form.Children.Add(_status);

        _formScroll.Content = _form;
        _host.Children.Add(_formScroll);
        Children.Add(_host);
        _ = Sweep();
    }

    /// <summary>The white capsule of a primary action, as wide as the fields.</summary>
    public static Border PrimaryButton(string title)
    {
        var label = new TextBlock { Text = title, Foreground = Theme.Black, FontWeight = F.SemiBold, FontSize = Theme.FontHeadline, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        var border = F.Capsule(label, Theme.White, 44, 28);
        border.Width = 360;
        return border;
    }

    /// <summary>A step (setup, invitation) over the form; back shows the form as it was.</summary>
    private void ShowStep(FrameworkElement step) => F.ShowOnly(_host, step);

    private void ShowForm() => F.ShowOnly(_host, _formScroll);

    /// <summary>What the server answered decides what's offered: its setup, or a sign-in (with invitations when it has them).</summary>
    private void ShowFor(ServerInfo? info)
    {
        _info = info;
        var unclaimed = info?.Unclaimed == true;
        _setupSection.Visibility = unclaimed ? Visibility.Visible : Visibility.Collapsed;
        _signInSection.Visibility = unclaimed ? Visibility.Collapsed : Visibility.Visible;
        _inviteButton.Visibility = info?.Has("invites") == true ? Visibility.Visible : Visibility.Collapsed;
    }

    private void StartSetup()
    {
        var api = ServerApi();
        if (api == null || _info is not { Unclaimed: true } info) return;
        ShowStep(new SetupFlowPage(_shell, api, info, null, ShowForm));
    }

    private void OpenInvite()
    {
        var api = ServerApi();
        if (api == null || _info == null) return;
        ShowStep(new InvitePage(_shell, api, _info, ShowForm));
    }

    private void CheckAddress(string text)
    {
        _check?.Cancel();
        var cts = _check = new CancellationTokenSource();
        var url = MoonriseApi.Normalize(text);
        _serverLine.Text = "";
        ShowFor(null);
        if (url == null) return;
        _ = RunCheck(url, cts.Token);
    }

    private async Task RunCheck(Uri url, CancellationToken ct)
    {
        try
        {
            await Task.Delay(600, ct);
            var api = _shell.Session.NewApi(url);
            var info = await api.Info(ct);
            if (ct.IsCancellationRequested) return;
            _api = api;
            _serverLine.Foreground = new SolidColorBrush(Color.FromArgb(255, 80, 200, 120));
            _serverLine.Text = $"{info.Name} · {info.Version}";
            ShowFor(info);
        }
        catch (OperationCanceledException) { }
        catch { if (!ct.IsCancellationRequested) { _serverLine.Foreground = Theme.Secondary; _serverLine.Text = "No Moonrise server answered at that address."; ShowFor(null); } }
    }

    /// <summary>A sweep of the local /24 on port 8097, as the other apps do without Bonjour.</summary>
    private async Task Sweep()
    {
        var prefixes = NetworkInterface.GetAllNetworkInterfaces()
            .Where(n => n.OperationalStatus == OperationalStatus.Up && n.NetworkInterfaceType != NetworkInterfaceType.Loopback)
            .SelectMany(n => n.GetIPProperties().UnicastAddresses)
            .Where(a => a.Address.AddressFamily == AddressFamily.InterNetwork && !IPAddress.IsLoopback(a.Address))
            .Select(a => string.Join('.', a.Address.ToString().Split('.').Take(3)))
            .Distinct().Take(3).ToList();
        var found = new List<(string Address, ServerInfo Info)>();
        using var gate = new SemaphoreSlim(48);
        using var http = new HttpClient { Timeout = TimeSpan.FromSeconds(1.5) };
        var probes = prefixes.SelectMany(p => Enumerable.Range(1, 254).Select(i => $"{p}.{i}")).Select(async ip =>
        {
            await gate.WaitAsync();
            try
            {
                var api = new MoonriseApi(MoonriseApi.Normalize(ip)!, null, "probe", "probe", http);
                var info = await api.Info();
                lock (found) found.Add((ip, info));
            }
            catch { /* nothing there */ }
            finally { gate.Release(); }
        });
        await Task.WhenAll(probes);
        _found.Children.Clear();
        if (found.Count == 0)
        {
            _found.Children.Add(F.Text("None found. Type the server's address above.", Theme.FontCaption, fg: Theme.Secondary));
            return;
        }
        foreach (var (address, info) in found)
        {
            var t = F.Text(info.Unclaimed ? $"{info.Name} · {address} · New, not set up" : $"{info.Name} · {address}", Theme.FontSubheadline);
            var b = F.Bare(t, () => _address.Text = address);
            b.HorizontalAlignment = HorizontalAlignment.Left;
            _found.Children.Add(b);
        }
    }

    private MoonriseApi? ServerApi()
    {
        var url = MoonriseApi.Normalize(_address.Text);
        if (url == null) { _status.Text = "Enter the server's address first."; return null; }
        return _api?.BaseUrl == url ? _api : (_api = _shell.Session.NewApi(url));
    }

    private async Task SignIn()
    {
        var api = ServerApi();
        if (api == null) return;
        if (string.IsNullOrWhiteSpace(_user.Text)) { _status.Text = "Enter your username."; return; }
        _status.Text = "Signing in…";
        _signIn.IsEnabled = false;
        try
        {
            var info = await api.Info();
            var r = await api.Login(_user.Text.Trim(), _password.Password);
            // An admin of a server without libraries (a setup left before its library step): the setup goes on there.
            if (r.User.IsAdmin == true && await NoLibraries(api)) { ShowStep(new SetupFlowPage(_shell, api, info, r, ShowForm)); _status.Text = ""; return; }
            Done(api, r);
        }
        catch (MoonriseHttpException e) when (e.Code is 401 or 403) { _status.Text = "Wrong user name or password."; }
        catch (MoonriseHttpException e) when (e.Code == 429) { _status.Text = "Too many attempts. Try again in a few minutes."; }
        catch (Exception e) { _status.Text = "Can't reach that server. " + e.Message; }
        finally { _signIn.IsEnabled = true; }
    }

    private static async Task<bool> NoLibraries(MoonriseApi api)
    {
        try { return (await api.AdminLibraries()).Count == 0; }
        catch { return false; }
    }

    private async Task SignInWithCode()
    {
        var api = ServerApi();
        if (api == null) return;
        _codeWait?.Cancel();
        var cts = _codeWait = new CancellationTokenSource();
        try
        {
            var info = await api.Info();
            if (!info.Has("device-code")) { _status.Text = "This server doesn't offer sign-in with a code."; return; }
            var code = await api.StartDeviceCode();
            _status.Text = $"Open {code.VerifyUrlComplete} on a device that's signed in, and approve {code.UserCode}. Waiting for approval…";
            var interval = Math.Max(1, code.Interval);
            var deadline = DateTime.UtcNow.AddSeconds(code.ExpiresIn);
            while (DateTime.UtcNow < deadline && !cts.IsCancellationRequested)
            {
                await Task.Delay(TimeSpan.FromSeconds(interval), cts.Token);
                var p = await api.PollDeviceCode(code.Code);
                if (p.State == DevicePollState.Approved) { Done(api, p.Auth!); return; }
                if (p.State == DevicePollState.Expired) break;
                if (p.State == DevicePollState.SlowDown) interval += 2;
            }
            _status.Text = "The code expired. Try again.";
        }
        catch (OperationCanceledException) { }
        catch (Exception e) { _status.Text = "Can't reach that server. " + e.Message; }
    }

    private void Done(MoonriseApi api, AuthResult auth)
    {
        _shell.Session.SignedIn(api, auth);
        _shell.SignedIn();
    }
}
