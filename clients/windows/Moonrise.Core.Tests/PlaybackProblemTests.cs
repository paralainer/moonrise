using Xunit;

namespace Moonrise.Core.Tests;

/// <summary>How a playback failure is named; the shared cases are in SharedCasesTests.</summary>
public class PlaybackProblemTests
{
    [Fact]
    public void ShortWordsWithTheServersTextAsDetails()
    {
        const string raw = "probe: ffprobe: [matroska,webm @ 0x61e4] EBML header parsing failed\n/media/movies/Film.mkv: Invalid data found when processing input";
        var p = PlaybackProblem.Of(new MoonriseHttpException(422, "  " + raw + "\n"));
        Assert.Equal("Damaged file", p.Title);
        Assert.DoesNotContain("ffprobe", p.Message);
        Assert.Equal(raw, p.Detail);
        Assert.Null(PlaybackProblem.Classify(502, " ").Detail);
        Assert.Equal(PlaybackProblem.Kinds.Unreachable, PlaybackProblem.Of(new HttpRequestException("No connection could be made")).Kind);
        Assert.Equal(PlaybackProblem.Kinds.Unindexed, PlaybackProblem.Of(new Playback { Supported = false, Reasons = ["no keyframe index: no Cues found"] }).Kind);
        foreach (var k in Enum.GetValues<PlaybackProblem.Kinds>())
        {
            var x = new PlaybackProblem(k, null);
            Assert.True(x.Title.Length <= 24, $"{k} title is short");
            Assert.True(x.Message.Length <= 110, $"{k} line fits a small player");
        }
    }

    [Fact]
    public void ATitleNoFileOfWhichCanPlay()
    {
        var bad = new MediaFile { Id = "a", Broken = true, BrokenReason = "damaged or incomplete: truncated" };
        var healthy = new MediaFile { Id = "b" };
        var gone = new MediaFile { Id = "c", Name = "Film (2017).mkv", Missing = true };
        static Item Movie(params MediaFile[] f) => new() { Id = "m", Kind = "movie", Files = f };

        Assert.Equal(PlaybackProblem.Kinds.Damaged, PlaybackProblem.Unplayable(Movie(bad))?.Kind);
        Assert.Equal("damaged or incomplete: truncated", PlaybackProblem.Unplayable(Movie(bad))?.Detail);
        Assert.Null(PlaybackProblem.Unplayable(Movie(bad, healthy)));
        Assert.Null(PlaybackProblem.Unplayable(Movie(healthy)));
        Assert.Null(PlaybackProblem.Unplayable(new Item { Id = "m", Kind = "movie" }));
        Assert.Equal(PlaybackProblem.Kinds.Missing, PlaybackProblem.Unplayable(Movie(gone))?.Kind);
        Assert.Contains("Film (2017).mkv", PlaybackProblem.Unplayable(Movie(gone))!.Detail);
        Assert.Equal(PlaybackProblem.Kinds.Damaged, PlaybackProblem.Unplayable(Movie(gone, bad))?.Kind);
        Assert.Null(PlaybackProblem.Unplayable(Movie(gone, healthy)));
        Assert.Equal(PlaybackProblem.Kinds.Damaged, PlaybackProblem.Broken(null).Kind);
    }
}
