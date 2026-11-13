using System.Text.Json;
using System.Text.Json.Nodes;
using Xunit;

namespace Moonrise.Core.Tests;

/// <summary>The shared cases in api/: the same files the server, Apple and Android tests read.</summary>
public class SharedCasesTests
{
    private static List<Segment> Segments(JsonArray a) => a.Select(e =>
    {
        var o = e!.AsObject();
        var source = o["source"]?.GetValue<string>();
        return new Segment(SegmentKinds.Of(o["kind"]!.GetValue<string>())!.Value, o["start"]!.GetValue<double>(), o["end"]!.GetValue<double>(),
            source == "detected" ? "detected" : null);
    }).ToList();

    [Fact]
    public void SegmentRulesCases()
    {
        var root = JsonNode.Parse(File.ReadAllText(Path.Combine(Repo.ApiDir, "segment-rules.json")))!.AsObject();
        var cases = root["cases"]!.AsArray();
        Assert.True(cases.Count >= 10);
        foreach (var c in cases.Select(n => n!.AsObject()))
        {
            var name = c["name"]!.GetValue<string>();
            var got = SegmentRules.Merge(
                Segments(c["server"]!.AsArray()),
                c["online"]!.AsArray().Select(l => (IReadOnlyList<Segment>)Segments(l!.AsArray())).ToList(),
                c["runtime"]!.GetValue<double>(),
                pending: c["pending"]?.GetValue<bool>() ?? false,
                movie: c["movie"]?.GetValue<bool>() ?? false,
                communityPending: c["community_pending"]?.GetValue<bool>() ?? false);
            var expected = Segments(c["expected"]!.AsArray());
            Assert.True(expected.Select(s => (s.Kind, s.Start, s.End)).SequenceEqual(got.Select(s => (s.Kind, s.Start, s.End))), name);
        }
    }

    [Fact]
    public void ResumeRuleCases()
    {
        var root = JsonNode.Parse(File.ReadAllText(Path.Combine(Repo.ApiDir, "resume-rules.json")))!.AsObject();
        var cases = root["cases"]!.AsArray();
        Assert.True(cases.Count >= 10);
        foreach (var c in cases.Select(n => n!.AsObject()))
        {
            var u = c["user"]?.AsObject();
            var item = new Item
            {
                Id = "m", Kind = "movie",
                User = u == null ? null : new UserData { Position = u["position"]?.GetValue<double>(), Played = u["played"]?.GetValue<bool>() },
            };
            var expected = c["expected"]?.GetValue<double>();
            Assert.True(expected == item.ResumeSeconds, $"{c["name"]}: expected {expected}, got {item.ResumeSeconds}");
        }
    }

    [Fact]
    public void PlaybackProblemCases()
    {
        var root = JsonNode.Parse(File.ReadAllText(Path.Combine(Repo.ApiDir, "playback-problem-rules.json")))!.AsObject();
        var cases = root["cases"]!.AsArray().Select(n => n!.AsObject()).ToList();
        Assert.True(cases.Count >= 15);
        foreach (var c in cases)
        {
            var p = PlaybackProblem.Classify(c["status"]?.GetValue<int>(), c["text"]!.GetValue<string>(), c["pending"]?.GetValue<bool>() ?? false);
            Assert.True(c["expected"]!.GetValue<string>() == p.Key, $"{c["name"]}: expected {c["expected"]}, got {p.Key}");
        }
        Assert.Equal(Enum.GetValues<PlaybackProblem.Kinds>().Select(k => k.ToString().ToLowerInvariant()).ToHashSet(),
            cases.Select(c => c["expected"]!.GetValue<string>()).ToHashSet());
    }

    [Fact]
    public void AskedAgainNearTheEnd()
    {
        const double d = 7268.64;
        Assert.Equal(0, SegmentOffer.NearEndAsksDue(d, 0));
        Assert.Equal(0, SegmentOffer.NearEndAsksDue(d, d - 601));
        Assert.Equal(1, SegmentOffer.NearEndAsksDue(d, d - 600));
        Assert.Equal(1, SegmentOffer.NearEndAsksDue(d, d - 121));
        Assert.Equal(2, SegmentOffer.NearEndAsksDue(d, d - 100));
        Assert.Equal(0, SegmentOffer.NearEndAsksDue(0, 100));
    }
}

/// <summary>
/// The API contract's examples (api/examples/*.json, each {summary, value}) decode with this
/// client's models. Every example is decoded or listed with why not, so a new one fails until it is.
/// </summary>
public class ApiExamplesTests
{
    private static JsonElement Value(string file)
    {
        var doc = JsonDocument.Parse(File.ReadAllText(Path.Combine(Repo.ApiDir, "examples", file)));
        return doc.RootElement.GetProperty("value");
    }

    private static readonly Dictionary<string, Action<JsonElement>> Decoded = new()
    {
        ["info.json"] = v => Assert.NotEmpty(D<ServerInfo>(v).Capabilities),
        ["sign-in.json"] = v => Assert.NotEmpty(D<AuthResult>(v).Token),
        ["me.json"] = v => D<User>(v),
        ["device-code.json"] = v => D<DeviceCode>(v),
        ["device-poll-approved.json"] = v => Assert.Equal("approved", D<PollAnswer>(v).Status),
        ["libraries.json"] = v => Assert.NotEmpty(D<ListAnswer<Library>>(v).Items),
        ["facets.json"] = v => D<LibraryFacets>(v),
        ["items.json"] = v => Assert.NotEmpty(D<PageAnswer>(v).Items),
        ["item-movie.json"] = v => D<Item>(v),
        ["item-episode.json"] = v => D<Item>(v),
        ["seasons.json"] = v => Assert.NotEmpty(D<ListAnswer<Item>>(v).Items),
        ["episodes.json"] = v => Assert.NotEmpty(D<ListAnswer<Item>>(v).Items),
        ["show-continue.json"] = v => D<ContinueAnswer>(v),
        ["search.json"] = v => D<SearchResults>(v),
        ["search-start.json"] = v => { var s = D<SearchStart>(v); Assert.NotEmpty(s.Recent); Assert.NotEmpty(s.Suggestions); },
        ["home.json"] = v => D<Home>(v),
        ["segments.json"] = v => D<SegmentsAnswer>(v),
        ["segments-merged.json"] = v => D<SegmentsAnswer>(v),
        ["playback.json"] = v =>
        {
            var p = D<Playback>(v);
            Assert.True(p.Tracks?.SameFile);
            Assert.Equal(2, p.Tracks?.Audio?.Index);
            Assert.Equal("fr", p.Tracks?.Subtitle?.Language);
            // The sidecar is fetched whole; the embedded track window by window.
            Assert.False(p.Subtitles.Single(s => s.Index == 0).Embedded);
            Assert.True(p.Subtitles.Single(s => s.Index == 3).Embedded);
            Assert.EndsWith("/subtitles/3.srt", p.Subtitles.Single(s => s.Index == 3).Url);
        },
        ["subtitles.json"] = v => Assert.NotEmpty(D<SubtitleList>(v).Subtitles),
        ["subtitle-track.json"] = v => D<SubtitleTrack>(v),
        ["trickplay.json"] = v => Assert.NotNull(D<Thumbnails>(v).TileAt(0)),
        ["subtitle-cues.json"] = v =>
        {
            var w = D<SubtitleWindow>(v);
            Assert.True(w.Complete);
            Assert.Equal(new SubtitleCue(1.0, 4.0, "Hello, world."), Assert.Single(w.Cues));
        },
        ["subtitle-picture-cues.json"] = v =>
        {
            var w = D<SubtitleWindow>(v);
            Assert.False(w.Complete);
            Assert.Equal(new PictureCanvas { Width = 1920, Height = 1080 }, w.Canvas);
            var c = Assert.Single(w.Cues);
            Assert.Equal((777, 949, 2, 1), (c.Image!.X, c.Image.Y, c.Image.Width, c.Image.Height));
            Assert.StartsWith("iVBOR", c.Image.Png);
        },
        ["subtitle-timing.json"] = v => D<SubtitleTiming>(v),
    };

    private static readonly Dictionary<string, string> NotUsed = new()
    {
        ["latest.json"] = "per-library Recently Added: Home's rows are used",
        ["report.json"] = "playback reports: the answer isn't read",
        ["local-reports.json"] = "no downloads in this milestone",
        ["continue-hidden.json"] = "Remove from Continue Watching: the answer isn't read",
        ["played.json"] = "played marks: the answer isn't read",
        ["search-recent-recorded.json"] = "a title opened from Search: the answer isn't read",
        ["search-recent-cleared.json"] = "clearing Search's Recent: the answer isn't read",
        ["version.json"] = "choosing a version: the answer isn't read",
        ["subtitle-search.json"] = "OpenSubtitles search: not in this milestone",
        ["download-job.json"] = "no downloads in this milestone",
        ["download-jobs.json"] = "no downloads in this milestone",
        ["offline.json"] = "no downloads in this milestone",
        ["setup-claim.json"] = "the answer is an AuthResult, as sign-in.json",
    };

    private static T D<T>(JsonElement v) => v.Deserialize<T>(MoonriseApi.Json)!;

    [Fact]
    public void EveryExampleDecodesOrIsListed()
    {
        var dir = Path.Combine(Repo.ApiDir, "examples");
        var files = Directory.GetFiles(dir, "*.json").Select(Path.GetFileName).OrderBy(x => x).ToList();
        Assert.True(files.Count > 30, "no examples in " + dir);
        var failures = new List<string>();
        foreach (var f in files)
        {
            if (Decoded.TryGetValue(f!, out var decode))
            {
                try { decode(Value(f!)); }
                catch (Exception e) { failures.Add($"{f}: {e.GetType().Name}: {e.Message}"); }
            }
            else if (!NotUsed.ContainsKey(f!) && !f!.StartsWith("admin-")) failures.Add($"{f}: not decoded and not listed as unused");
        }
        foreach (var name in Decoded.Keys.Concat(NotUsed.Keys).Where(n => !files.Contains(n))) failures.Add($"{name}: listed, but there is no such example");
        Assert.True(failures.Count == 0, string.Join("\n", failures));
    }

    [Fact]
    public void ARenamedFieldFails()
    {
        var broken = JsonDocument.Parse("""{"serverId": "x", "name": "n", "version": "1"}""").RootElement;
        Assert.ThrowsAny<JsonException>(() => broken.Deserialize<ServerInfo>(MoonriseApi.Json));
    }
}
