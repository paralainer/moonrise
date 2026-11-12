using System.Text.Json;
using System.Text.Json.Nodes;
using Xunit;

namespace Moonrise.Core.Tests;

/// <summary>Set Up This Server and Have an Invitation, as the Mac app does them.</summary>
public class SetupTests
{
    private static T Example<T>(string name) =>
        JsonSerializer.Deserialize<T>(JsonNode.Parse(File.ReadAllText(Path.Combine(Repo.ApiDir, "examples", name + ".json")))!["value"]!.ToJsonString(), MoonriseApi.Json)!;

    [Fact]
    public void TheServersSetupAnswersDecode()
    {
        Assert.True(Example<ServerInfo>("info").Unclaimed);
        Assert.True(Example<ServerInfo>("info").Has("setup-claim"));
        var claim = Example<AuthResult>("setup-claim");
        Assert.True(claim.User.IsAdmin);
        var folder = Example<FolderListing>("admin-fs-browse");
        Assert.Equal("/media", folder.Path);
        Assert.Equal(["Movies", "Shows"], folder.Dirs.Select(d => d.Name));
        Assert.Equal("/media/Movies", folder.Dirs[0].Path);
        Assert.Equal("movies", Example<AdminLibrary>("admin-library").Id);
        var scan = Example<ScanProgress>("admin-scan-progress");
        Assert.Equal("identifying", scan.Phase);
        Assert.Equal(3, scan.Identify.Total);
        Assert.Equal(2, scan.Libraries.Count);
    }

    [Theory]
    [InlineData("123 456", "123456")]
    [InlineData("12-34-56-78", "123456")]
    [InlineData("abc", "")]
    public void TheSetupCodeIsSixDigits(string typed, string digits) => Assert.Equal(digits, SetupRules.CodeDigits(typed));

    [Fact]
    public void TheSetupPageAddressHasNoScheme()
    {
        Assert.Equal("192.168.1.10:8097", SetupRules.PageAddress(MoonriseApi.Normalize("192.168.1.10")!));
        Assert.Contains("http://192.168.1.10:8097 in a browser", SetupRules.CodeMessage(MoonriseApi.Normalize("192.168.1.10")!));
    }

    [Fact]
    public void AccountAndInvitationAskForWhatIsMissing()
    {
        Assert.Equal("", SetupRules.AccountProblem(" ", "password1", "password1"));
        Assert.Equal("The password needs at least 8 characters.", SetupRules.AccountProblem("me", "short", ""));
        Assert.Equal("", SetupRules.AccountProblem("me", "password1", ""));
        Assert.Equal("The passwords don't match.", SetupRules.AccountProblem("me", "password1", "password2"));
        Assert.Null(SetupRules.AccountProblem("me", "password1", "password1"));

        Assert.Equal("", SetupRules.InviteProblem("ABCD-EFG", "me", "password1"));
        Assert.Null(SetupRules.InviteProblem("abcd efgh", "me", "password1"));
        Assert.Equal("The password needs at least 8 characters.", SetupRules.InviteProblem("ABCD-EFGH", "me", "short"));
    }

    [Fact]
    public void RefusalsSayWhatToDo()
    {
        Assert.StartsWith("That isn't the code", SetupRules.VerifyError(new MoonriseHttpException(403, "")));
        Assert.StartsWith("This server has been set up already", SetupRules.VerifyError(new MoonriseHttpException(409, "")));
        Assert.StartsWith("Too many wrong codes. Wait", SetupRules.VerifyError(new MoonriseHttpException(429, "")));
        Assert.StartsWith("The setup code has changed", SetupRules.ClaimError(new MoonriseHttpException(403, "")));
        Assert.Equal("Username taken", SetupRules.ClaimError(new MoonriseHttpException(400, "username taken")));
        Assert.StartsWith("Too many wrong codes. Try again", SetupRules.InviteError(new MoonriseHttpException(429, "")));
        Assert.Equal("Invitation not found", SetupRules.InviteError(new MoonriseHttpException(404, "invitation not found")));
    }

    [Fact]
    public void FolderTexts()
    {
        Assert.StartsWith("Nothing in here yet.", SetupRules.VideosText(0, 0));
        Assert.StartsWith("1 video and 2 folders in here.", SetupRules.VideosText(1, 2));
        Assert.Equal("Movies", SetupRules.FolderName("/media/Movies"));
        Assert.Equal("Movies", SetupRules.FolderName("/media/Movies/"));
        Assert.Equal("/", SetupRules.FolderName("/"));
    }

    [Fact]
    public void TheFirstScanIsDoneAfterAPassThatFoundFiles()
    {
        static ScanProgress P(string phase, int passes, int files, int movies = 0) => new()
        {
            Phase = phase, Passes = passes, Walked = files,
            Libraries = [new ScanProgress.LibraryCount { Id = "media", Name = "Media", Files = files, Movies = movies }],
        };
        var w = new ScanWatch(passesBefore: 1, libraryId: "media");
        Assert.Equal("Starting the scan…", w.PhaseText);
        w.Update(P("idle", 1, 0)); // the pass from before the library
        Assert.False(w.Done);
        w.Update(P("scanning", 1, 7));
        Assert.Equal("Looking for videos…", w.PhaseText);
        Assert.Equal((7, "Files"), w.Stats[0]);
        w.Update(P("idle", 2, 7, movies: 5));
        Assert.True(w.Done);
        Assert.Equal("Your library is ready.", w.PhaseText);
        Assert.Equal((5, "Movies"), w.Stats[0]);
        Assert.DoesNotContain("You can continue: the scan goes on in the background.", w.Notes);

        // An empty folder: done after three idle polls, and it says so.
        var empty = new ScanWatch(0, "media");
        for (var i = 0; i < 2; i++) { empty.Update(P("idle", 1, 0)); Assert.False(empty.Done); }
        empty.Update(P("idle", 1, 0));
        Assert.True(empty.Done);
        Assert.Contains("No videos were found in that folder.", empty.Notes);
    }

    // ---------------------------------------------------------------- A network share, and Extras

    private sealed class Recorder(string answer, int status = 200) : HttpMessageHandler
    {
        public readonly List<(string Method, string Url, string Body)> Requests = [];
        protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken ct)
        {
            Requests.Add((request.Method.Method, request.RequestUri!.PathAndQuery, request.Content == null ? "" : await request.Content.ReadAsStringAsync(ct)));
            return new HttpResponseMessage((System.Net.HttpStatusCode)status) { Content = new StringContent(answer) };
        }
    }

    private static (MoonriseApi Api, Recorder Rec) Api(string answer, int status = 200)
    {
        var rec = new Recorder(answer, status);
        return (new MoonriseApi(MoonriseApi.Normalize("192.168.1.10")!, "token", "dev", "Test", new HttpClient(rec)), rec);
    }

    private static string ExampleJson(string name) => JsonNode.Parse(File.ReadAllText(Path.Combine(Repo.ApiDir, "examples", name + ".json")))!["value"]!.ToJsonString();

    [Fact]
    public void TheNetworkShareAnswersDecode()
    {
        var found = Example<FileServerList>("admin-smb-discover");
        Assert.Equal(("NAS", "192.168.1.30"), (found.Servers[0].Name, found.Servers[0].Host));
        Assert.Equal(["Media", "Backup"], Example<ShareList>("admin-smb-shares").Shares.Select(s => s.Name));
        var folder = Example<ShareListing>("admin-smb-browse");
        Assert.Equal("Video", folder.Path);
        Assert.Equal("Video/Movies", folder.Dirs[0].Path);
        var test = Example<ShareTest>("admin-smb-test");
        Assert.True(test.Ok);
        Assert.Equal(2, test.VideoFiles);
        var services = Example<ServiceList>("admin-services").Services;
        Assert.Equal("tmdb", services[0].Id);
        Assert.Equal("not_set", services[0].State);
        Assert.StartsWith("Titles are matched on TMDB", services[0].With);
    }

    [Fact]
    public async Task AShareFolderBecomesTheLibrarysRootWithItsAccount()
    {
        var (api, rec) = Api(ExampleJson("admin-library"));
        var root = new LibraryRoot(RootSource.Smb, "Video/Movies", "192.168.1.30", "Media", new ShareLogin("demo", "not-a-real-password"));
        await api.CreateLibrary("Media", root);
        var body = JsonNode.Parse(rec.Requests.Single().Body)!;
        Assert.Equal("Media", (string)body["name"]!);
        var r = body["roots"]![0]!;
        Assert.Equal(("smb", "192.168.1.30", "Media", "Video/Movies", "demo", "not-a-real-password"),
            ((string)r["source"]!, (string)r["host"]!, (string)r["share"]!, (string)r["path"]!, (string)r["username"]!, (string)r["password"]!));

        // A guest share sends no account; a server folder only its path.
        Assert.False(new LibraryRoot(RootSource.Smb, "", "nas.local", "Media").Body().ContainsKey("username"));
        Assert.Equal(new Dictionary<string, object> { ["source"] = "local", ["path"] = "/media" }, LibraryRoot.Local("/media").Body());
        Assert.Equal("192.168.1.30 › Media › Video › Movies", root.Display);
        Assert.Equal("/media/Movies", LibraryRoot.Local("/media/Movies").Display);
    }

    [Fact]
    public async Task ShareRequestsGoToTheServerWithTheAccountOnlyWhenThereIsOne()
    {
        var (api, rec) = Api(ExampleJson("admin-smb-shares"));
        await api.Shares("192.168.1.30", null);
        Assert.Equal(("POST", "/v1/admin/smb/shares"), (rec.Requests[0].Method, rec.Requests[0].Url));
        Assert.Null(JsonNode.Parse(rec.Requests[0].Body)!["username"]);
        await api.Shares("192.168.1.30", new ShareLogin("demo", "not-a-real-password"));
        Assert.Equal("demo", (string)JsonNode.Parse(rec.Requests[1].Body)!["username"]!);

        var (d, drec) = Api(ExampleJson("admin-smb-discover"));
        await d.DiscoverFileServers();
        Assert.Equal("/v1/admin/smb/discover?wait_ms=2500", drec.Requests[0].Url);
    }

    [Fact]
    public void ShareFailuresSayWhatToDo()
    {
        static MoonriseHttpException E(string problem) => new(400, $"{{\"error\":\"the server can't be reached\",\"problem\":\"{problem}\"}}");
        var failure = new MoonriseHttpException(502, ExampleJson("admin-smb-failure"));
        Assert.Equal(new ShareProblem("unreachable", "the server can't be reached"), ShareRules.Problem(failure));
        Assert.Null(ShareRules.Problem(new MoonriseHttpException(400, "plain words")));
        Assert.Equal("The server can't be reached", ShareRules.Text(failure));

        Assert.Equal(new ShareOutcome(true, null), ShareRules.ConnectFailure(E("auth_required"), "nas.local"));
        Assert.Equal(new ShareOutcome(true, "The user name or password is wrong."), ShareRules.ConnectFailure(E("wrong_password"), "nas.local"));
        Assert.Equal(new ShareOutcome(false, "The Moonrise server can't reach nas.local. Check the name or address."), ShareRules.ConnectFailure(failure, "nas.local"));
        Assert.Equal(new ShareOutcome(false, "The server can't be reached"), ShareRules.ConnectFailure(E("other"), "nas.local"));
        Assert.Equal(new ShareOutcome(false, "Host is required"), ShareRules.ConnectFailure(new MoonriseHttpException(400, "host is required"), "nas.local"));

        // The shares listed as a guest, the share itself wanting an account.
        Assert.Equal(new ShareOutcome(true, null), ShareRules.BrowseFailure(E("access_denied"), hadLogin: false));
        Assert.Equal(new ShareOutcome(true, "This account may not open that share."), ShareRules.BrowseFailure(E("access_denied"), hadLogin: true));
        Assert.Equal(new ShareOutcome(true, "The user name or password is wrong."), ShareRules.BrowseFailure(E("wrong_password"), hadLogin: true));
        Assert.Equal(new ShareOutcome(false, "The server can't be reached"), ShareRules.BrowseFailure(E("not_found"), hadLogin: true));

        Assert.Equal("This folder is empty.", ShareRules.TestProblem(new ShareTest { Ok = false }));
        Assert.Null(ShareRules.TestProblem(new ShareTest { Ok = true }));
    }

    [Fact]
    public void ShareFoldersGoUpToTheShares()
    {
        Assert.Equal("Video", ShareRules.Up("Video/Movies"));
        Assert.Equal("", ShareRules.Up("Video"));
        Assert.Null(ShareRules.Up(""));
        Assert.Equal("Media", ShareRules.FolderTitle("Media", ""));
        Assert.Equal("Movies", ShareRules.FolderTitle("Media", "Video/Movies"));
        Assert.Equal("nas › Media", ShareRules.Breadcrumb("nas", "Media", ""));
    }

    [Fact]
    public async Task ExtrasSaveOnlyWhatWasFilledIn()
    {
        Assert.Empty(ExtrasRules.Changes(" ", "", "", ""));
        Assert.Equal("Finish", ExtrasRules.FinishTitle(0));
        Assert.Equal("Save and Finish", ExtrasRules.FinishTitle(1));
        // An OpenSubtitles account needs both its name and password.
        Assert.Empty(ExtrasRules.Changes("", "me", "", ""));
        var c = ExtrasRules.Changes(" fake-tmdb-key ", " me ", "fake-pass", "fake-llm-key");
        Assert.Equal(new Dictionary<string, string>
        {
            ["tmdb.api_key"] = "fake-tmdb-key", ["opensubtitles.username"] = "me", ["opensubtitles.password"] = "fake-pass", ["llm.api_key"] = "fake-llm-key",
        }, c);

        var (api, rec) = Api("{}");
        await api.UpdateSettings(c);
        Assert.Equal(("PATCH", "/v1/admin/settings"), (rec.Requests[0].Method, rec.Requests[0].Url));
        Assert.Equal("fake-tmdb-key", (string)JsonNode.Parse(rec.Requests[0].Body)!["tmdb.api_key"]!); // dotted keys as they are

        var services = Example<ServiceList>("admin-services").Services;
        Assert.True(ExtrasRules.HasFields(services.Single(s => s.Id == "tmdb")));
        Assert.False(ExtrasRules.HasFields(services.Single(s => s.Id == "ratings")));
        Assert.False(ExtrasRules.HasFields(services.Single(s => s.Id == "tmdb") with { State = "off" }));
    }

    [Fact]
    public void APollAnsweredAfterTheScanStepIsLeftDoesNotPollAgain()
    {
        // Leaving stopped the timer, but a request still out came back to a flag still on and polled again.
        var gate = new PollGate();
        var round = gate.Start();
        Assert.True(gate.Continues(round));
        gate.Stop(); // the step or the page left while the request was out
        Assert.False(gate.Continues(round));
        // Back on the step: a new round; the old one's late answer still doesn't go on, so only one loop runs.
        var again = gate.Start();
        Assert.True(gate.Continues(again));
        Assert.False(gate.Continues(round));
    }
}
