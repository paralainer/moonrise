using System.Text.Json;
using System.Text.Json.Nodes;
using Xunit;

namespace Moonrise.Core.Tests;

/// <summary>The Library's categories, A–Z strip and paged grid, and the title page's player size.</summary>
public class LibraryBrowseTests
{
    private static T Example<T>(string name) =>
        JsonSerializer.Deserialize<T>(JsonNode.Parse(File.ReadAllText(Path.Combine(Repo.ApiDir, "examples", name + ".json")))!["value"]!.ToJsonString(), MoonriseApi.Json)!;

    private static LibraryFacets Facets(int studios = 0, int collections = 0) => new()
    {
        Genres = [new FacetNamed { Name = "Drama", Count = 4, Backdrop = "b1" }, new FacetNamed { Name = "Comedy", Count = 2 }],
        Studios = Enumerable.Range(0, studios).Select(i => new FacetNamed { Name = $"Studio {i}", Count = i % 2 == 0 ? 1 : 3 }).ToList(),
        Decades = [new FacetDecade { Decade = 1990, Count = 3 }],
        Collections = Enumerable.Range(0, collections).Select(i => new FacetCollection { Id = 100 + i, Name = $"Saga {i} Collection", Count = i % 2 == 0 ? 1 : 2, Poster = "p" }).ToList(),
    };

    [Fact]
    public void TheSidebarListsCollectionsOnlyForMovies()
    {
        Assert.Equal([LibraryCategory.Genres, LibraryCategory.Collections, LibraryCategory.Studios, LibraryCategory.Decades], LibraryCategories.For(shows: false));
        Assert.Equal([LibraryCategory.Genres, LibraryCategory.Studios, LibraryCategory.Decades], LibraryCategories.For(shows: true));
        Assert.Equal("Networks", LibraryCategory.Studios.Title(shows: true));
        Assert.Equal("Studios", LibraryCategory.Studios.Title(shows: false));
        Assert.Equal("1 network", LibraryCategory.Studios.CountLine(true, 1));
        Assert.Equal("12 genres", LibraryCategory.Genres.CountLine(false, 12));
        Assert.Equal("1 show", LibraryCategories.Titles(true, 1));
        Assert.Equal("3 movies", LibraryCategories.Titles(false, 3));
    }

    [Fact]
    public void ATileOpensItsPageWithItsDimensionLocked()
    {
        var genre = LibraryCategories.Entries(LibraryCategory.Genres, Facets()).First();
        Assert.Equal("Drama", genre.Title);
        Assert.Equal("b1", genre.Art);
        Assert.Contains(LockedDimension.Genre, genre.Filter.Locked);
        Assert.Equal("Drama", genre.Filter.ServerQuery["genre"]);
        Assert.Equal(0, genre.Filter.ActiveCount); // the page's own dimension isn't a filter that's on
        // Clear All keeps what makes the page.
        var more = genre.Filter with { Watched = WatchedFilter.Unwatched, Hdr = true };
        Assert.Equal(2, more.ActiveCount);
        Assert.Equal(["Drama"], more.Cleared.Genres);
        Assert.Equal(WatchedFilter.All, more.Cleared.Watched);

        var decade = LibraryCategories.Entries(LibraryCategory.Decades, Facets()).Single();
        Assert.Equal("1990s", decade.Title);
        Assert.Equal(LibrarySort.Year, decade.Filter.Sort);
        Assert.False(decade.Filter.Descending);
        Assert.Equal("1990", decade.Filter.ServerQuery["year_from"]);
        Assert.Equal("1999", decade.Filter.ServerQuery["year_to"]);
    }

    [Fact]
    public void LongStudioAndCollectionListsShowThoseWithMoreThanOneTitle()
    {
        var f = Facets(studios: 30, collections: 30);
        var studios = LibraryCategories.Entries(LibraryCategory.Studios, f);
        Assert.Equal(15, studios.Count);
        Assert.All(studios, s => Assert.True(s.Count > 1));
        Assert.Equal(15, LibraryCategories.HiddenStudios(f, studios.Count));
        // A search finds the rest.
        Assert.Equal(["Studio 2", "Studio 20", "Studio 21", "Studio 22", "Studio 23", "Studio 24", "Studio 25", "Studio 26", "Studio 27", "Studio 28", "Studio 29"],
            LibraryCategories.Entries(LibraryCategory.Studios, f, "studio 2").Select(s => s.Title));
        var studio = studios[0];
        Assert.Equal(studio.Title, studio.Filter.ServerQuery["studio"]);
        Assert.Contains(LockedDimension.Studio, studio.Filter.Locked);

        Assert.Equal(15, LibraryCategories.Entries(LibraryCategory.Collections, f).Count);
        var c = LibraryCategories.Entries(LibraryCategory.Collections, f).First();
        Assert.Equal("101", c.Filter.ServerQuery["collection"]);
        Assert.Equal("p", c.Poster);
        Assert.Equal("Saga 1", LibraryCategories.PosterName(c.Title));
        // A short list keeps every one.
        Assert.Equal(3, LibraryCategories.Entries(LibraryCategory.Studios, Facets(studios: 3)).Count);
    }

    [Fact]
    public void TheServersFacetsGiveTheTiles()
    {
        var f = Example<LibraryFacets>("facets");
        var studios = LibraryCategories.Entries(LibraryCategory.Studios, f);
        Assert.Equal("Blender Foundation", studios.Single().Title);
        Assert.Equal("f1a66d157c1dc19289c84df8", studios.Single().Art);
        Assert.Equal("2000s", LibraryCategories.Entries(LibraryCategory.Decades, f).Single().Title);
    }

    [Fact]
    public void TheLetterStripJumpsToTheNearestLetterWithTitles()
    {
        var index = Example<PageAnswer>("items").Index!;
        Assert.NotEmpty(index);
        IReadOnlyList<PageLetter> letters = [new() { Letter = "#", Offset = 0 }, new() { Letter = "B", Offset = 2 }, new() { Letter = "M", Offset = 40 }];
        Assert.Equal("B", LetterIndex.Nearest(letters, "A"));
        Assert.Equal("M", LetterIndex.Nearest(letters, "C"));
        Assert.Equal("M", LetterIndex.Nearest(letters, "Z")); // none after: the one before
        Assert.Equal(40, LetterIndex.Offset(letters, "M"));
        Assert.Null(LetterIndex.Offset(letters, "Q"));
        Assert.Equal(27, LetterIndex.All.Count);
        Assert.Equal("#", LetterIndex.At(0, 270));
        Assert.Equal("A", LetterIndex.At(15, 270));
        Assert.Equal("Z", LetterIndex.At(400, 270));
        Assert.True(LetterIndex.Shows(LibrarySort.Title, 3, 25));
        Assert.False(LetterIndex.Shows(LibrarySort.Title, 3, 24));
        Assert.False(LetterIndex.Shows(LibrarySort.Added, 3, 500));
        Assert.False(LetterIndex.Shows(LibrarySort.Title, 1, 500));
    }

    [Fact]
    public void TheGridMakesOnlyTheRowsNearTheScreen()
    {
        var g = new GridWindow(Columns: 6, CellHeight: 300, Total: 10_000);
        Assert.Equal(1667, g.Rows);
        Assert.Equal(1667 * 300, g.Height);
        Assert.Equal((0, 6 * 7 - 1), g.Range(0, 900)); // rows 0-3 on screen, 3 spare below
        var (first, last) = g.Range(300_000, 900);
        Assert.Equal((1000 - 3) * 6, first);
        Assert.Equal((1003 + 3 + 1) * 6 - 1, last);
        Assert.Equal(9_999, g.Range(g.Height - 900, 900).Last);
        Assert.Equal(600, g.RowTop(12));
        Assert.Equal((0, -1), new GridWindow(6, 300, 0).Range(0, 900));
    }

    [Fact]
    public void PagesAreFetchedForTheRangeShownAndAnOldAnswerIsDropped()
    {
        var pages = new SparsePages(60);
        Assert.Equal([0, 1], pages.Begin(0, 70));
        Assert.Empty(pages.Begin(10, 70)); // on the way already
        var gen = pages.Generation;
        Item I(int i) => new() { Id = "i" + i, Kind = "movie", Title = "t" };
        Assert.True(pages.Accept(gen, 1, new ItemPage(Enumerable.Range(60, 60).Select(I).ToList(), 5000, [new() { Letter = "A", Offset = 0 }])));
        Assert.Equal(5000, pages.Total);
        Assert.Equal("i61", pages.At(61)!.Id);
        Assert.Null(pages.At(1));
        // A letter near the end loads only its own pages.
        Assert.Equal([80, 81], pages.Begin(4800, 4900));
        pages.Failed(gen, 80);
        Assert.Equal([80], pages.Begin(4800, 4900)); // asked for again
        pages.Reset();
        Assert.False(pages.Accept(gen, 0, new ItemPage([I(0)], 1, [])));
        Assert.Null(pages.Total);
        Assert.Null(pages.At(61));
    }

    [Theory]
    [InlineData(1280, 752, 451)]  // a wide window: 60 % of the page's height
    [InlineData(800, 900, 450)]   // a tall one: 16:9 across
    [InlineData(1920, 1032, 619)]
    public void TheTitlePagesPlayerIs169UpToSixtyPercentOfThePage(double w, double h, double expected) =>
        Assert.Equal(expected, ClientRules.InlinePlayerHeight(w, h));
}
