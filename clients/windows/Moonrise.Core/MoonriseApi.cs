using System.Globalization;
using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Moonrise.Core;

/// <summary>An HTTP error from the server: its status and the start of its body.</summary>
public sealed class MoonriseHttpException(int code, string body)
    : Exception(string.IsNullOrWhiteSpace(body) ? $"The Moonrise server returned {code}." : $"The Moonrise server returned {code}. {body[..Math.Min(200, body.Length)]}")
{
    public int Code { get; } = code;
    public string Body { get; } = body;
}

public enum ReportEvent { Start, Progress, Stop }

/// <summary>
/// The server's API (/v1/..., Bearer tokens), as the other apps speak it. Tokens travel only in the
/// Authorization header; session URLs (/v1/s/{sid}/...) carry the session id instead.
/// </summary>
public sealed class MoonriseApi
{
    public const int DefaultPort = 8097;

    public static readonly JsonSerializerOptions Json = new(JsonSerializerDefaults.Web)
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        PropertyNameCaseInsensitive = false,
        ReadCommentHandling = JsonCommentHandling.Skip,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        Converters = { new JsonStringEnumConverter() },
    };

    private readonly HttpClient _http;
    private readonly string _deviceId;
    private readonly string _deviceName;
    private ServerInfo? _info;

    public Uri BaseUrl { get; }
    public string? Token { get; set; }

    public MoonriseApi(Uri baseUrl, string? token, string deviceId, string deviceName, HttpClient? http = null)
    {
        BaseUrl = baseUrl;
        Token = token;
        _deviceId = deviceId;
        _deviceName = deviceName;
        _http = http ?? new HttpClient { Timeout = TimeSpan.FromSeconds(30) };
    }

    /// <summary><c>host</c>, <c>host:port</c> or a URL; a bare host gets the server's port (8097).</summary>
    public static Uri? Normalize(string address)
    {
        var s = address.Trim();
        if (s.Length == 0) return null;
        if (!s.Contains("://")) s = "http://" + s;
        s = s.TrimEnd('/');
        var hasPort = System.Text.RegularExpressions.Regex.IsMatch(s, @"^[a-zA-Z]+://(\[[^\]]+\]|[^/:]+):\d+");
        if (!Uri.TryCreate(s, UriKind.Absolute, out var u) || u.Host.Length == 0) return null;
        var b = new UriBuilder(u) { Path = "/", Query = "", Fragment = "" };
        if (!hasPort && u.Scheme == "http") b.Port = DefaultPort;
        return b.Uri;
    }

    public string Url(string path, IDictionary<string, string?>? query = null)
    {
        var sb = new StringBuilder();
        var first = true;
        if (query != null)
            foreach (var (k, v) in query.Where(kv => kv.Value != null).OrderBy(kv => kv.Key, StringComparer.Ordinal))
            {
                sb.Append(first ? '?' : '&').Append(Uri.EscapeDataString(k)).Append('=').Append(Uri.EscapeDataString(v!));
                first = false;
            }
        return new Uri(BaseUrl, "/" + path.TrimStart('/')) + sb.ToString();
    }

    internal async Task<string> Send(HttpMethod method, string url, object? body = null, bool authorized = true, CancellationToken ct = default)
    {
        using var req = new HttpRequestMessage(method, url);
        req.Headers.Accept.ParseAdd("application/json");
        if (authorized && Token != null) req.Headers.Authorization = new AuthenticationHeaderValue("Bearer", Token);
        if (body != null) req.Content = JsonContent.Create(body, options: Json);
        using var resp = await _http.SendAsync(req, ct).ConfigureAwait(false);
        var text = await resp.Content.ReadAsStringAsync(ct).ConfigureAwait(false);
        if (!resp.IsSuccessStatusCode) throw new MoonriseHttpException((int)resp.StatusCode, text[..Math.Min(300, text.Length)].Trim());
        return text;
    }

    private async Task<T> Get<T>(string path, IDictionary<string, string?>? query = null, bool authorized = true, CancellationToken ct = default)
    {
        var text = await Send(HttpMethod.Get, Url(path, query), null, authorized, ct).ConfigureAwait(false);
        return JsonSerializer.Deserialize<T>(text, Json)!;
    }

    private async Task<T> Post<T>(string path, object body, bool authorized = true)
    {
        var text = await Send(HttpMethod.Post, Url(path), body, authorized).ConfigureAwait(false);
        return JsonSerializer.Deserialize<T>(text, Json)!;
    }

    private static string Enc(string s) => Uri.EscapeDataString(s);

    // ---------------------------------------------------------------- server and sign-in

    /// <summary>GET /v1/info (no sign-in).</summary>
    public async Task<ServerInfo> Info(CancellationToken ct = default)
    {
        _info = await Get<ServerInfo>("v1/info", authorized: false, ct: ct).ConfigureAwait(false);
        return _info;
    }

    /// <summary>The server's capabilities, asked once per session.</summary>
    public async Task<ServerInfo> Capabilities() => _info ?? await Info().ConfigureAwait(false);

    public async Task<AuthResult> Login(string username, string password)
    {
        var r = await Post<AuthResult>("v1/auth/login", new { username, password, device_id = _deviceId, device_name = _deviceName }, authorized: false).ConfigureAwait(false);
        Token = r.Token;
        return r;
    }

    public async Task<User> Me() => await Get<User>("v1/me").ConfigureAwait(false);

    public Task<DeviceCode> StartDeviceCode() => Post<DeviceCode>("v1/auth/device", new { device_id = _deviceId, device_name = _deviceName }, authorized: false);

    public async Task<DevicePoll> PollDeviceCode(string deviceCode)
    {
        var a = await Post<PollAnswer>("v1/auth/device/poll", new { device_code = deviceCode }, authorized: false).ConfigureAwait(false);
        switch (a.Status)
        {
            case "approved" when a.Token != null && a.User != null:
                Token = a.Token;
                return new DevicePoll(DevicePollState.Approved, new AuthResult { Token = a.Token, User = a.User });
            case "approved":
            case "expired":
                return new DevicePoll(DevicePollState.Expired);
            case "slow_down":
                return new DevicePoll(DevicePollState.SlowDown);
            default:
                return new DevicePoll(DevicePollState.Pending);
        }
    }

    // ---------------------------------------------------------------- setting up, and invitations

    /// <summary>POST /v1/setup/verify: is this the setup code? (204; 403 wrong; 409 set up already; 429.)</summary>
    public Task VerifySetupCode(string code) => Send(HttpMethod.Post, Url("v1/setup/verify"), new { code }, authorized: false);

    /// <summary>POST /v1/setup/claim: the first account (an admin), signed in on this device.</summary>
    public async Task<AuthResult> Claim(string code, string username, string password)
    {
        var r = await Post<AuthResult>("v1/setup/claim", new { code, username, password, device_id = _deviceId, device_name = _deviceName }, authorized: false).ConfigureAwait(false);
        Token = r.Token;
        return r;
    }

    /// <summary>POST /v1/auth/invite: a new account with an invitation code (capability <c>invites</c>), signed in on this device.</summary>
    public async Task<AuthResult> RedeemInvite(string code, string username, string password)
    {
        var r = await Post<AuthResult>("v1/auth/invite", new { code, username, password, device_id = _deviceId, device_name = _deviceName }, authorized: false).ConfigureAwait(false);
        Token = r.Token;
        return r;
    }

    public async Task<IReadOnlyList<AdminLibrary>> AdminLibraries() => (await Get<AdminLibraryList>("v1/admin/libraries").ConfigureAwait(false)).Items;

    /// <summary>Adds a library of one folder (on the server's computer or in a network share); the server checks it and starts a full scan.</summary>
    public Task<AdminLibrary> CreateLibrary(string name, LibraryRoot root) =>
        Post<AdminLibrary>("v1/admin/libraries", new { name, roots = new[] { root.Body() } });

    /// <summary>GET /v1/admin/fs/browse: the server's folders (no path: where browsing may start).</summary>
    public Task<FolderListing> BrowseServer(string? path) => Get<FolderListing>("v1/admin/fs/browse", Q(("path", path)));

    /// <summary>GET /v1/admin/smb/discover: file servers on the server's network (Bonjour, NetBIOS).</summary>
    public async Task<IReadOnlyList<FileServer>> DiscoverFileServers() =>
        (await Get<FileServerList>("v1/admin/smb/discover", Q(("wait_ms", "2500"))).ConfigureAwait(false)).Servers;

    /// <summary>POST /v1/admin/smb/shares: a file server's shares, read by the server.</summary>
    public async Task<IReadOnlyList<Share>> Shares(string host, ShareLogin? login) =>
        (await Post<ShareList>("v1/admin/smb/shares", ShareBody(login, ("host", host))).ConfigureAwait(false)).Shares;

    /// <summary>POST /v1/admin/smb/browse: a folder in a share.</summary>
    public Task<ShareListing> BrowseShare(string host, string share, string path, ShareLogin? login) =>
        Post<ShareListing>("v1/admin/smb/browse", ShareBody(login, ("host", host), ("share", share), ("path", path)));

    /// <summary>POST /v1/admin/smb/test: would this share folder work as a library folder?</summary>
    public Task<ShareTest> TestShare(string host, string share, string path, ShareLogin? login) =>
        Post<ShareTest>("v1/admin/smb/test", ShareBody(login, ("host", host), ("share", share), ("path", path)));

    private static Dictionary<string, string> ShareBody(ShareLogin? login, params (string k, string v)[] kv)
    {
        var d = kv.ToDictionary(p => p.k, p => p.v);
        if (login != null) { d["username"] = login.Username; d["password"] = login.Password; }
        return d;
    }

    /// <summary>GET /v1/admin/services: the optional services and their state.</summary>
    public async Task<IReadOnlyList<Service>> Services() => (await Get<ServiceList>("v1/admin/services").ConfigureAwait(false)).Services;

    /// <summary>PATCH /v1/admin/settings with dotted keys.</summary>
    public Task UpdateSettings(IReadOnlyDictionary<string, string> changes) => Send(HttpMethod.Patch, Url("v1/admin/settings"), changes);

    public Task<ScanProgress> ScanProgress() => Get<ScanProgress>("v1/admin/scan/progress");

    public async Task Logout()
    {
        try { await Send(HttpMethod.Post, Url("v1/auth/logout")).ConfigureAwait(false); } catch { /* signed out anyway */ }
    }

    // ---------------------------------------------------------------- browsing

    public Task<Home> Home(int limit = 20) => Get<Home>("v1/home", Q(("limit", limit.ToString())));

    public async Task<IReadOnlyList<Library>> Libraries() => (await Get<ListAnswer<Library>>("v1/libraries").ConfigureAwait(false)).Items;

    public Task<Item> Item(string id) => Get<Item>($"v1/items/{Enc(id)}");

    public async Task<IReadOnlyList<Item>> Seasons(string seriesId) => (await Get<ListAnswer<Item>>($"v1/shows/{Enc(seriesId)}/seasons").ConfigureAwait(false)).Items;

    public async Task<IReadOnlyList<Item>> Episodes(string seriesId, string? seasonId = null) =>
        (await Get<ListAnswer<Item>>($"v1/shows/{Enc(seriesId)}/episodes", Q(("season", seasonId))).ConfigureAwait(false)).Items;

    public async Task<Item?> ContinueEpisode(string seriesId) => (await Get<ContinueAnswer>($"v1/shows/{Enc(seriesId)}/continue").ConfigureAwait(false)).Item;

    public async Task<ItemPage> Items(LibraryView view, LibraryFilter filter, int offset, int limit)
    {
        var q = Q(("type", view.Type()), ("sort", filter.Sort.Query()), ("order", filter.Descending ? "desc" : "asc"),
            ("offset", offset.ToString()), ("limit", limit.ToString()));
        foreach (var (k, v) in filter.ServerQuery) q[k] = v;
        var a = await Get<PageAnswer>("v1/items", q).ConfigureAwait(false);
        return new ItemPage(a.Items, a.Total ?? a.Items.Count, a.Index ?? []);
    }

    public Task<LibraryFacets> Facets(LibraryView view, LibraryFilter filter)
    {
        var q = Q(("type", view.Type()), ("recent_days", LibraryFilter.RecentDays.ToString()));
        foreach (var (k, v) in filter.ServerQuery) q[k] = v;
        return Get<LibraryFacets>("v1/facets", q);
    }

    public Task<SearchResults> Search(string term) => Get<SearchResults>("v1/search", Q(("q", term)));

    /// <summary>What Search offers before typing; null when this server doesn't offer it.</summary>
    public async Task<SearchStart?> SearchStart()
    {
        var info = await Capabilities().ConfigureAwait(false);
        if (!info.Has("search-start")) return null;
        return await Get<SearchStart>("v1/search/start").ConfigureAwait(false);
    }

    public Task RecordSearchOpened(string itemId) => Send(HttpMethod.Put, Url($"v1/search/recent/{Enc(itemId)}"));
    public Task ClearSearchRecent() => Send(HttpMethod.Delete, Url("v1/search/recent"));

    public Task SetPlayed(string itemId, bool played) => Send(played ? HttpMethod.Put : HttpMethod.Delete, Url($"v1/items/{Enc(itemId)}/played"));
    public Task RemoveFromContinue(string itemId) => Send(HttpMethod.Delete, Url($"v1/continue/{Enc(itemId)}"));
    public Task ChooseVersion(string itemId, string fileId) => Send(HttpMethod.Put, Url($"v1/items/{Enc(itemId)}/version"), new { file_id = fileId });

    /// <summary>Playback reports: start, progress every 10 s, stop. The server keeps the resume position.</summary>
    public async Task Report(string sessionId, ReportEvent ev, double position, bool paused, int? audio = null, int? subtitle = null, double? credits = null, TrackChoice? tracks = null)
    {
        var body = new Dictionary<string, object?> { ["position"] = position, ["paused"] = paused };
        if (audio != null) body["audio"] = audio;
        if (subtitle != null) body["subtitle"] = subtitle;
        if (credits != null) body["credits"] = credits;
        if (tracks != null) body["tracks"] = TrackReport.Json(tracks);
        await Send(HttpMethod.Post, Url($"v1/sessions/{Enc(sessionId)}/{ev.ToString().ToLowerInvariant()}"), body).ConfigureAwait(false);
    }

    /// <summary>Image ids are content hashes: immutable, so the URL is its own cache key. No token needed.</summary>
    public string ImageUrl(string id, int width) => Url($"v1/images/{Enc(id)}", Q(("w", width.ToString())));

    // ---------------------------------------------------------------- playback

    /// <summary><paramref name="quality"/>: a preset's bits per second, transcoded to; null for the original.</summary>
    public Task<Playback> PlaybackInfo(string itemId, double? start = null, bool hdr = false, string? version = null, bool lossless = false, long? quality = null)
    {
        var q = Q(("hdr", hdr ? "1" : "0"), ("version", version), ("quality", quality?.ToString(CultureInfo.InvariantCulture)));
        if (lossless) q["lossless"] = "1";
        if (start is > 1) q["start"] = start.Value.ToString("F3", CultureInfo.InvariantCulture);
        return Get<Playback>($"v1/items/{Enc(itemId)}/playback", q);
    }

    public string? MasterUrl(Playback p) => p.Master != null ? new Uri(BaseUrl, p.Master).ToString() : null;

    public async Task CloseSession(string sessionId)
    {
        try { await Send(HttpMethod.Delete, Url($"v1/s/{Enc(sessionId)}"), authorized: false).ConfigureAwait(false); } catch { /* already gone */ }
    }

    // ---------------------------------------------------------------- segments and subtitles

    /// <summary>The segments to offer, merged by the server (merged=1); an older server answers the file's own.</summary>
    public async Task<ServerSegments> Segments(string itemId, string? version = null, bool provisional = false, bool community = true)
    {
        var a = await Get<SegmentsAnswer>($"v1/items/{Enc(itemId)}/segments",
            Q(("version", version), ("merged", "1"), ("provisional", provisional ? "1" : null), ("community", community ? null : "0"))).ConfigureAwait(false);
        return Decode(a);
    }

    public static ServerSegments Decode(SegmentsAnswer a)
    {
        var items = a.Items.Select(s => SegmentKinds.Of(s.Kind) is { } k ? new Segment(k, s.Start, s.End, s.Source == "detected" ? "detected" : null) : null)
            .Where(s => s != null).Select(s => s!).ToList();
        return new ServerSegments(items, a.Pending ?? false, Merged: a.Merged ?? false, CommunityPending: a.CommunityPending ?? false);
    }

    public string MediaId(string itemId, string? version) => version != null ? $"{itemId}~{version}" : itemId;

    public async Task<IReadOnlyList<SubtitleTrack>> Subtitles(string mediaId) =>
        (await Get<SubtitleList>($"v1/items/{Enc(mediaId)}/subtitles").ConfigureAwait(false)).Subtitles;

    public string Resolve(string path) => new Uri(BaseUrl, path).ToString();

    public async Task<byte[]> FetchBytes(string url)
    {
        using var req = new HttpRequestMessage(HttpMethod.Get, url);
        if (Token != null) req.Headers.Authorization = new AuthenticationHeaderValue("Bearer", Token);
        using var resp = await _http.SendAsync(req).ConfigureAwait(false);
        var bytes = await resp.Content.ReadAsByteArrayAsync().ConfigureAwait(false);
        if (!resp.IsSuccessStatusCode) throw new MoonriseHttpException((int)resp.StatusCode, Encoding.UTF8.GetString(bytes[..Math.Min(300, bytes.Length)]).Trim());
        return bytes;
    }

    /// <summary>A text track as text (the whole <c>.srt</c>).</summary>
    public async Task<string> FetchText(string url, CancellationToken ct = default)
    {
        using var req = new HttpRequestMessage(HttpMethod.Get, url);
        if (Token != null) req.Headers.Authorization = new AuthenticationHeaderValue("Bearer", Token);
        using var resp = await _http.SendAsync(req, ct).ConfigureAwait(false);
        var bytes = await resp.Content.ReadAsByteArrayAsync(ct).ConfigureAwait(false);
        if (!resp.IsSuccessStatusCode) throw new MoonriseHttpException((int)resp.StatusCode, Encoding.UTF8.GetString(bytes[..Math.Min(300, bytes.Length)]).Trim());
        return SubtitleParser.Decode(bytes);
    }

    /// <summary>
    /// One 30-second window of an embedded text track (capability <c>subtitle-windows</c>), with the
    /// session's id so a window is read for the title playing.
    /// </summary>
    public Task<SubtitleWindow> SubtitleWindow(string mediaId, int index, int window, string? sessionId = null, CancellationToken ct = default) =>
        Get<SubtitleWindow>($"v1/items/{Enc(mediaId)}/subtitles/{index}/cues", Q(("window", window.ToString(CultureInfo.InvariantCulture)), ("sid", sessionId)), ct: ct);

    /// <summary>The viewer's delay for a track the server keeps timing for (every device sees it).</summary>
    public async Task<SubtitleTiming> SetSubtitleNudge(string mediaId, int index, double nudge)
    {
        var text = await Send(HttpMethod.Put, Url($"v1/items/{Enc(mediaId)}/subtitles/{index}/timing"), new { nudge }).ConfigureAwait(false);
        return JsonSerializer.Deserialize<SubtitleTiming>(text, Json)!;
    }

    public Task<Thumbnails> Trickplay(string mediaId, string? sessionId) => Get<Thumbnails>($"v1/items/{Enc(mediaId)}/trickplay", Q(("sid", sessionId)));

    private static Dictionary<string, string?> Q(params (string k, string? v)[] kv)
    {
        var d = new Dictionary<string, string?>();
        foreach (var (k, v) in kv) d[k] = v;
        return d;
    }
}
