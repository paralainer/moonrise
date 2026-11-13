using Xunit;

namespace Moonrise.Core.Tests;

public class RulesTests
{
    private static Item M(string id) => new() { Id = id, Kind = "movie", Title = id };

    [Theory]
    [InlineData("192.168.1.10", "http://192.168.1.10:8097/")]
    [InlineData("192.168.1.10:9000", "http://192.168.1.10:9000/")]
    [InlineData("https://a.example", "https://a.example/")]
    [InlineData("http://b.example/x/", "http://b.example:8097/")]
    public void AddressesNormalizeLikeTheOtherApps(string input, string expected) =>
        Assert.Equal(expected, MoonriseApi.Normalize(input)!.ToString());

    [Fact]
    public void QueryBuildsWithEscapingAndSortedKeys()
    {
        var api = new MoonriseApi(new Uri("http://x:8097/"), null, "d", "n");
        Assert.Equal("http://x:8097/v1/items?a=1&q=a%20b", api.Url("v1/items", new Dictionary<string, string?> { ["q"] = "a b", ["a"] = "1", ["skip"] = null }));
    }

    [Fact]
    public void SearchStartOpenedAndCleared()
    {
        var s = new SearchStart { Recent = [M("a"), M("b")], Suggestions = [M("c"), M("d")] };
        var fromSuggestions = s.Opened(M("c"));
        Assert.Equal(["c", "a", "b"], fromSuggestions.Recent.Select(i => i.Id));
        Assert.Equal(["d"], fromSuggestions.Suggestions.Select(i => i.Id));
        Assert.Empty(s.RecentCleared().Recent);
        Assert.True(SearchStart.Shows("  "));
        Assert.False(SearchStart.Shows("a"));
    }

    [Fact]
    public void RuntimeAndClockText()
    {
        Assert.Equal("1h 34m", Format.Runtime(5640));
        Assert.Equal("48m", Format.Runtime(2880));
        Assert.Null(Format.Runtime(0));
        Assert.Equal("1:02:05", Format.Clock(3725));
        Assert.Equal("4:09", Format.Clock(249));
    }

    [Fact]
    public void CountLineMatchesTheOtherApps()
    {
        Assert.Equal("7 movies · Title, A to Z", new LibraryFilter().CountLine(false, 7));
        var byDate = new LibraryFilter().SortedBy(LibrarySort.Added);
        Assert.Equal("1 show · Date Added, newest first", byDate.CountLine(true, 1));
    }

    [Fact]
    public void RememberedAudioFollowsTheFileThenTheLanguage()
    {
        var a = new AudioCandidate(2, "eng", true, "English");
        var b = new AudioCandidate(3, "rus", false, "Russian");
        var c = new AudioCandidate(4, "rus", false, "Russian Commentary");
        Assert.Equal(3, TrackRules.RememberedAudio([a, b, c], new TrackMemory { SameFile = true, Audio = new TrackPick { Index = 3, Language = "ru" } }));
        Assert.Equal(3, TrackRules.RememberedAudio([a, b, c], new TrackMemory { Audio = new TrackPick { Language = "ru" } }));
        Assert.Equal(4, TrackRules.RememberedAudio([a, b, c], new TrackMemory { Audio = new TrackPick { Language = "ru", Title = "commentary" } }));
        Assert.Null(TrackRules.RememberedAudio([a], null));
    }

    [Fact]
    public void RememberedSubtitle()
    {
        var options = new[] { new SubtitleCandidate("s1", "English", "eng", 5), new SubtitleCandidate("s2", "Russian", "ru", 6) };
        Assert.IsType<SubtitleMemory.Usual>(TrackRules.RememberedSubtitle(options, null));
        Assert.IsType<SubtitleMemory.OffHere>(TrackRules.RememberedSubtitle(options, new TrackMemory { SameFile = true, SubtitleOff = true }));
        Assert.IsType<SubtitleMemory.UsualOff>(TrackRules.RememberedSubtitle(options, new TrackMemory { SubtitleOff = true }));
        var hit = Assert.IsType<SubtitleMemory.Use>(TrackRules.RememberedSubtitle(options, new TrackMemory { SameFile = true, Subtitle = new TrackPick { Index = 6, Language = "ru" } }));
        Assert.Equal("s2", hit.Id);
    }

    [Fact]
    public void ForcedSubtitleFollowsTheAudioLanguage()
    {
        var options = new[] { new SubtitleCandidate("en", "English", "en", 5), new SubtitleCandidate("f", "English · Forced", "en", 6) };
        Assert.Equal("f", TrackRules.UsualSubtitle(options, on: false, "ru", false, "en")!.Id);
        Assert.Null(TrackRules.UsualSubtitle(options, on: false, "ru", false, "ja"));
        Assert.Equal("en", TrackRules.UsualSubtitle(options, on: true, "en", false, "ja")!.Id);
    }

    [Fact]
    public void PictureSubtitlesArePickedLikeTextOnes()
    {
        // The app draws picture tracks (PGS, DVD, DVB) as it draws text ones: the file's order decides.
        var options = new[] { new SubtitleCandidate("pgs", "English - PGSSUB", "en", 2), new SubtitleCandidate("srt", "English - SUBRIP", "en", 3), new SubtitleCandidate("dvd", "English · Forced - DVDSUB", "en", 4) };
        Assert.Equal("pgs", TrackRules.UsualSubtitle(options, on: true, "en", false, "ja")!.Id);
        Assert.Equal("dvd", TrackRules.UsualSubtitle(options, on: false, "en", false, "en")!.Id);
        var remembered = Assert.IsType<SubtitleMemory.Use>(TrackRules.RememberedSubtitle(options, new TrackMemory { Subtitle = new TrackPick { Language = "en", Title = "English" } }));
        Assert.Equal("pgs", remembered.Id);
    }

    [Fact]
    public void NextEpisodeIsOfferedNearTheEnd()
    {
        var segs = new List<Segment> { new(SegmentKind.Credits, 2700, 2800) };
        Assert.False(NextEpisodeRules.Offered(2800, 100, 100, null, segs, true));
        Assert.True(NextEpisodeRules.Offered(2800, 2770, 2775, null, [], true));
        Assert.True(NextEpisodeRules.Offered(2800, 2650, 2700, segs[0], segs, true));
        Assert.False(NextEpisodeRules.Offered(2800, 2780, 2780, null, segs, false));
        var c = NextEpisodeRules.Corner(segs[0], true, false, false, true, creditsSkipPlaysNext: true);
        Assert.True(c.Skip);
        Assert.False(c.Next);
    }

    [Fact]
    public void NextEpisodeFollowsAiringOrderAndKeepsSpecialsApart()
    {
        var a = new Item { Id = "a", Kind = "episode", SeasonNo = 1 };
        var b = new Item { Id = "b", Kind = "episode", SeasonNo = 2 };
        var s = new Item { Id = "s", Kind = "episode", SeasonNo = 0 };
        Assert.Equal("b", NextEpisodeRules.Next([a, b, s], a)?.Id);
        Assert.Null(NextEpisodeRules.Next([a, b, s], b));
        Assert.Null(NextEpisodeRules.Next([a, b], b));
    }

    [Fact]
    public void ChosenVersionPrefersTheViewersThenHealthy()
    {
        var bad = new MediaFile { Id = "bad", Broken = true };
        var a = new MediaFile { Id = "a" };
        var b = new MediaFile { Id = "b" };
        var item = new Item { Id = "t", Kind = "movie", Files = [bad, a, b], User = new UserData { Version = "b" } };
        Assert.Equal("b", Versions.Chosen(item, null, false, null)!.Id);
        Assert.Equal("a", Versions.Chosen(item with { User = new UserData { Version = "bad" } }, "a", false, null)!.Id);
        Assert.Equal("a", Versions.Chosen(item with { User = null }, null, false, null)!.Id);
    }

    [Fact]
    public void LanguagesNormalize()
    {
        Assert.Equal("de", Languages.Normalized("ger"));
        Assert.Equal("en", Languages.Normalized("eng"));
        Assert.Equal("en", Languages.Normalized("en-US"));
        Assert.Null(Languages.Normalized("und"));
    }

    [Fact]
    public void WatchedActionsFollowProgress()
    {
        var t = new Item { Id = "x", Kind = "movie" };
        Assert.Equal([WatchAction.Watched], WatchAction.Options(t));
        Assert.Equal([WatchAction.Unwatched], WatchAction.Options(t with { User = new UserData { Played = true } }));
        Assert.Equal([WatchAction.Watched, WatchAction.Unwatched], WatchAction.Options(t with { User = new UserData { Position = 90 } }));
    }

    [Fact]
    public void MacGridAndCardMetrics()
    {
        Assert.Equal(159, ClientRules.PosterWidth(1280));
        Assert.Equal(348, ClientRules.CardWidth(1280));
        var grid = ClientRules.LibraryGrid(1000);
        Assert.Equal(6, grid.Columns);
        Assert.Equal(156, grid.CardWidth);
        Assert.Equal("ABCD-1234", ClientRules.FormatDeviceCode("abcd 1234"));
    }

    [Fact]
    public void TheShowsOwnNameIsOnEpisodeCards()
    {
        var e = new Item { Id = "e", Kind = "episode", SeriesTitle = "Show", Title = "Pilot", SeasonNo = 2, EpisodeNo = 6 };
        Assert.Equal("Show", e.CardTitle);
        Assert.Equal("S2 · E6 · Pilot", e.CardSubtitle);
        Assert.Equal("6. Pilot", e.EpisodeListTitle);
    }
}
