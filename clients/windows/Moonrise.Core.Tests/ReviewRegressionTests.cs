using Xunit;

namespace Moonrise.Core.Tests;

/// <summary>Regressions from the first review of the Windows app. The map is in docs/dev/testing.md.</summary>
public class ReviewRegressionTests
{
    private static Item Movie(double? position, bool played = false, string? reason = null, double runtime = 1000) => new()
    {
        Id = "m", Kind = "movie", Runtime = runtime,
        User = new UserData { Position = position, Played = played },
        ContinueInfo = reason == null ? null : new ContinueInfo { Reason = reason },
    };

    [Fact]
    public void ATitleWatchedBeforeStillResumes()
    {
        // The Apple apps' rule: any position past 0, watched before or not.
        Assert.Equal(34.167, Movie(34.167, played: true).ResumeSeconds);
        Assert.Equal(120, Movie(120).ResumeSeconds);
        Assert.Equal(0.5, Movie(0.5).ResumeSeconds);
        Assert.Null(Movie(0).ResumeSeconds);
        Assert.Null(Movie(null).ResumeSeconds);
        Assert.Null(new Item { Id = "x", Kind = "movie" }.ResumeSeconds);
    }

    [Fact]
    public void AWatchedTitleShowsABarOnlyAsAContinueWatchingResume()
    {
        Assert.Null(Movie(250, played: true).Progress);
        Assert.Equal(0.25, Movie(250, played: true, reason: "resume").Progress);
        Assert.Equal(0.25, Movie(250).Progress);
    }

    [Fact]
    public void AHeroPictureThatArrivesLateDoesNotCoverTheCurrentOne()
    {
        var hero = new HeroSlides(3);
        hero.Show(0);
        hero.Opened(2); // the last download finishes first
        Assert.Equal(0, hero.Opacity(2));
        Assert.Equal(0, hero.Opacity(0)); // nothing shown until the current one is here
        hero.Opened(0);
        Assert.Equal(1, hero.Opacity(0));
        hero.Opened(1);
        Assert.Equal(0, hero.Opacity(1));
    }

    [Fact]
    public void AHeroPictureAlreadyArrivedShowsWhenItsTurnComes()
    {
        var hero = new HeroSlides(2);
        hero.Opened(0);
        hero.Opened(1);
        hero.Show(1);
        Assert.Equal(1, hero.Opacity(1));
        Assert.Equal(0, hero.Opacity(0));
    }

    [Theory]
    [InlineData("eng", "en")]
    [InlineData("rus", "ru")]
    [InlineData("deu", "de")]
    [InlineData("ger", "de")]
    [InlineData("zho", "zh")]
    [InlineData("chi", "zh")]
    [InlineData("en-US", "en")]
    [InlineData("fil", "fil")]
    [InlineData("und", null)]
    public void LanguagesNormalizeWithoutTheSystemsCultures(string input, string? expected) =>
        Assert.Equal(expected, Languages.Normalized(input));

    [Theory]
    [InlineData(true, 3, true)]
    [InlineData(true, 1, false)]
    [InlineData(false, 3, false)]
    public void ClickingTheSelectedTabGoesToItsRoot(bool selected, int depth, bool expected) =>
        Assert.Equal(expected, ClientRules.TabClickReturnsToRoot(selected, depth));

    [Theory] // A silent title showed Audio and Subtitles with nothing to pick
    [InlineData(0, 0, false, false)]
    [InlineData(1, 0, false, false)]
    [InlineData(2, 0, true, false)]
    [InlineData(0, 1, false, true)]
    [InlineData(3, 2, true, true)]
    public void TrackButtonsShowOnlyWithSomethingToPick(int audio, int subs, bool showsAudio, bool showsSubs) =>
        Assert.Equal((showsAudio, showsSubs), ClientRules.TrackButtons(audio, subs));

    [Fact]
    public void RatingsAreTheFocusedEpisodesWithTheShowsCriticsAdded()
    {
        var show = new Item
        {
            Id = "s", Kind = "series", CommunityRating = 8.0,
            Ratings = new Ratings { Imdb = new Score { Value = 9.1 }, RottenTomatoes = new Score { Value = 93 }, Metacritic = new Score { Value = 81 } },
        };
        var episode = new Item { Id = "e", Kind = "episode", CommunityRating = 7.4, Ratings = new Ratings { Imdb = new Score { Value = 7.9 } } };
        var r = Format.RatingsForPage(show, episode)!;
        Assert.Equal("7.9", r.Imdb);
        Assert.Equal("7.4", r.Star);
        Assert.Equal("93%", r.RottenTomatoes);
        Assert.Equal("81", r.Metacritic);
    }

    [Fact]
    public void RatingsAreLeftOutWhenThereAreNone()
    {
        Assert.Null(Format.RatingsForPage(new Item { Id = "m", Kind = "movie" }, null));
        var criticOnly = Format.RatingsForPage(new Item { Id = "m", Kind = "movie", CriticRating = 88 }, null)!;
        Assert.Equal("88%", criticOnly.RottenTomatoes);
        Assert.Null(criticOnly.Imdb);
    }

    [Theory]
    // Metrics.pad in clients/apple/Moonrise/Support/Theme.swift, which the Mac uses at its live width.
    [InlineData(1000, 150, 284)]
    [InlineData(1280, 159, 348)]
    [InlineData(1600, 203, 439)]
    public void CardSizesAreTheMacs(double window, int poster, int card)
    {
        Assert.Equal(poster, ClientRules.PosterWidth(window));
        Assert.Equal(card, ClientRules.CardWidth(window));
    }

    // Second review: the Library grid, paging while the list reloads, and badges after Mark as Watched.
    [Theory]
    // (avail + 12) / 162 columns, floor((avail - 12(n-1)) / n) wide: LibraryViews.swift, layout.
    [InlineData(944, 5, 179)]
    [InlineData(1000, 6, 156)]
    [InlineData(479, 3, 151)]
    [InlineData(200, 2, 94)]
    [InlineData(3000, 18, 155)]
    public void TheLibraryGridIsTheMacs(double available, int columns, int card)
    {
        var g = ClientRules.LibraryGrid(available);
        Assert.Equal(columns, g.Columns);
        Assert.Equal(card, g.CardWidth);
    }

    [Fact]
    public void TheLibraryGridNeverAsksForLessThanTheMacsMinimum() =>
        Assert.Equal(ClientRules.LibraryGrid(200), ClientRules.LibraryGrid(-50));

    [Theory]
    [InlineData(479)]
    [InlineData(944)]
    [InlineData(1000)]
    [InlineData(1620)]
    [InlineData(3000)]
    public void AllColumnsFitInThePanelTheGridIsGiven(double available)
    {
        // The wrap grid's cells carry the gap after each poster: this panel holds n of them exactly,
        // and the row of posters with its gaps still fits the width that was available.
        var g = ClientRules.LibraryGrid(available);
        Assert.Equal(g.Columns, (int)Math.Floor(g.PanelWidth / g.CellWidth));
        Assert.True(g.Columns * g.CardWidth + (g.Columns - 1) * PosterGrid.Gap <= available);
        Assert.True(g.PanelWidth - PosterGrid.Gap <= available);
    }

    [Fact]
    public void AnAnswerForAnOldListIsIgnored()
    {
        var list = new PagedList();
        var first = list.Begin()!.Value;
        list.Reset();
        Assert.False(list.Accept(first, 80, 200));
        Assert.Equal(0, list.Loaded);
    }

    [Fact]
    public void AReloadWhileARequestRunsStillLoadsTheNewList()
    {
        // Sort chosen while a page was on its way: the new list used to wait for nothing and stay empty.
        var list = new PagedList();
        var first = list.Begin()!.Value;
        Assert.Null(list.Begin()); // one at a time
        list.Reset();
        var second = list.Begin();
        Assert.NotNull(second);
        list.End(first); // the old request ending doesn't free the new one
        Assert.True(list.Busy);
        Assert.True(list.Accept(second!.Value, 80, 200));
        list.End(second.Value);
        Assert.False(list.Busy);
        Assert.Equal(80, list.Loaded);
    }

    [Fact]
    public void PagingStopsAtTheEndAndOnAnEmptyAnswer()
    {
        var list = new PagedList();
        var g = list.Begin()!.Value;
        list.Accept(g, 80, 80);
        list.End(g);
        Assert.True(list.Finished);
        Assert.Null(list.Begin());

        list.Reset();
        g = list.Begin()!.Value;
        list.Accept(g, 0, 500); // the total says more, the answer says there's no more
        list.End(g);
        Assert.Null(list.Begin());
    }

    [Fact]
    public void AFailedPageCanBeTriedAgain()
    {
        var list = new PagedList();
        var g = list.Begin()!.Value;
        list.End(g); // the request threw
        Assert.NotNull(list.Begin());
    }

    [Fact]
    public void EachPageHearsOfAWatchChangeOnce()
    {
        var watch = new WatchChanges();
        int library = 0, home = 0;
        watch.Changed("show");
        Assert.Equal(["show"], watch.Since(ref library));
        Assert.Empty(watch.Since(ref library));
        Assert.Equal(["show"], watch.Since(ref home));
    }

    [Fact]
    public void AnErrorRepeatingOnEveryFrameIsShownOnlyAFewTimes()
    {
        var gate = new FaultGate(burst: 3, windowMs: 5000);
        Assert.True(gate.ShouldShow(0));
        Assert.True(gate.ShouldShow(1));
        Assert.True(gate.ShouldShow(2));
        Assert.False(gate.ShouldShow(3));
        Assert.True(gate.ShouldShow(5100));
    }

    [Theory]
    [InlineData(0, "0 bytes")]
    [InlineData(1, "1 byte")]
    [InlineData(900, "900 bytes")]
    [InlineData(512_000, "512 KB")]
    [InlineData(999_950, "1 MB")]            // not "1000 KB"
    [InlineData(23_429_343, "23.4 MB")]      // this read "0.0 GB"
    [InlineData(49_999_999, "50 MB")]
    [InlineData(1_230_000_000, "1.23 GB")]
    [InlineData(18_200_000_000, "18.2 GB")]
    [InlineData(4_000_000_000_000, "4 TB")]
    public void FileSizesReadInTheUnitThatFits(long bytes, string text) =>
        Assert.Equal(text, Format.FileSize(bytes, System.Globalization.CultureInfo.InvariantCulture));

    [Fact]
    public void ThePlayersTitleRowMovesTheWindowBetweenItsControls()
    {
        PixelRect[] controls = [new(20, 12, 44, 44), new(1000, 12, 150, 44)];
        Assert.Equal(controls, ClientRules.PlayerTitleRowPassthrough(fullScreen: false, 1280, 48, controls));
        // Full screen has no window to move: all of the row is the player's.
        Assert.Equal([new PixelRect(0, 0, 3840, 144)], ClientRules.PlayerTitleRowPassthrough(fullScreen: true, 3840, 144, controls));
    }

    [Theory]
    [InlineData(95, 400, false)]   // the server went away with the title half played
    [InlineData(3000, 7200, false)]
    [InlineData(385, 400, true)]
    [InlineData(7150, 7200, true)]
    [InlineData(10, 0, true)]      // nothing to measure against
    public void AStreamThatStopsEarlyIsNotTheTitleFinishing(double position, double duration, bool end) =>
        Assert.Equal(end, ClientRules.StreamEndIsTheEnd(position, duration));
}
