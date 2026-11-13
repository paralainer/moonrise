using System.Globalization;
using Xunit;

namespace Moonrise.Core.Tests;

/// <summary>Settings, Quality, Episodes, scrubbing thumbnails and app-drawn subtitles, as the Mac app has them.</summary>
public class PlayerParityTests
{
    // ---------------------------------------------------------------- quality

    private static MediaFile File(long? bitrate, int? height, long? size = null, double? duration = null) => new()
    {
        Id = "f", Bitrate = bitrate, Size = size, Duration = duration,
        Tracks = height == null ? [] : [new Track { Index = 0, Kind = "video", Height = height }],
    };

    private static string[] Labels(IEnumerable<QualityOption> q) => q.Select(o => o.Label).ToArray();

    [Fact]
    public void QualityOffersOnlyPresetsBelowTheSource()
    {
        // As the Mac's QualityOption.options: no rung within 90 % of the source's bitrate, none above its class.
        Assert.Equal(["Original", "720p · 4 Mbps", "480p · 1.5 Mbps"], Labels(QualityOption.Options(8_000_000, 1080)));
        Assert.Equal(["Original", "1080p · 20 Mbps", "1080p · 10 Mbps", "720p · 4 Mbps", "480p · 1.5 Mbps"], Labels(QualityOption.Options(60_000_000, 1080)));
        Assert.Equal(Labels(QualityOption.All), Labels(QualityOption.Options(80_000_000, 2160)));
        // A 1920×800 scope film is still 1080p; 10 Mbps at 90 % of 11 isn't offered.
        Assert.Equal(["Original", "720p · 4 Mbps", "480p · 1.5 Mbps"], Labels(QualityOption.Options(11_000_000, 800)));
        Assert.Equal(["Original", "480p · 1.5 Mbps"], Labels(QualityOption.Options(3_000_000, 720)));
        // Unknown source: everything.
        Assert.Equal(Labels(QualityOption.All), Labels(QualityOption.Options(null, null)));
        Assert.Equal(["Original", "720p · 4 Mbps", "480p · 1.5 Mbps"], Labels(QualityOption.Options(File(8_000_000, 1080))));
    }

    [Fact]
    public void QualityStartsAtTheSettingWhenTheFileIsAboveIt()
    {
        var film = File(8_000_000, 1080);
        Assert.Equal("720p · 4 Mbps", QualityChoice.Starting("720p · 4 Mbps", film).Label);
        // A setting above the file: the file as it is, so the menu has it checked.
        Assert.Equal(QualityOption.Original, QualityChoice.Starting("1080p · 20 Mbps", film));
        Assert.Equal(QualityOption.Original, QualityChoice.Starting(null, film));
        Assert.Null(QualityChoice.Request(QualityOption.Original));
        Assert.Equal(4_000_000, QualityChoice.Request(QualityOption.Find("720p · 4 Mbps")));
        // An unknown label is 1080p · 10 Mbps, as the Mac's find.
        Assert.Equal("1080p · 10 Mbps", QualityOption.Find("8K").Label);
    }

    [Fact]
    public void QualitySizesAreTheFileForOriginalAndAnEstimateOtherwise()
    {
        var en = CultureInfo.GetCultureInfo("en-US");
        Assert.Equal("522 MB", QualityOption.Original.SizeLabel(600, 522_000_000, en));
        Assert.Equal("≈ 300 MB", QualityOption.Find("720p · 4 Mbps").SizeLabel(600, 522_000_000, en));
        Assert.Null(QualityOption.Find("720p · 4 Mbps").SizeLabel(null, 522_000_000, en));
        Assert.Null(QualityOption.Original.SizeLabel(600, null, en));
    }

    // ---------------------------------------------------------------- settings

    [Fact]
    public void SettingsDefaultsAreTheMacs()
    {
        var s = new PlayerSettings();
        Assert.Equal("Original", s.StreamingQuality);
        Assert.Equal(3, s.AutoSkipDelay);
        Assert.Equal("", s.PreferredAudioLanguage);
        Assert.Equal("en", s.PreferredSubtitleLanguage);
        Assert.False(s.AutoSelectSubtitles);
        Assert.Equal(SubtitleSize.Medium, s.SubtitleSize);
        Assert.False(s.SubtitleBackground);
        Assert.Equal(SkipMode.Auto, s.SkipMode(SegmentKind.Intro));
        Assert.Equal(SkipMode.Button, s.SkipMode(SegmentKind.Credits));
        Assert.Equal([0.8, 1.0, 1.25, 1.55], Enum.GetValues<SubtitleSize>().Select(z => z.Scale()));
        Assert.Equal("Show Skip Button", SkipMode.Button.Title());
    }

    [Fact]
    public void SettingsSurviveARestartAndABadFile()
    {
        var dir = Directory.CreateTempSubdirectory("moonrise-settings-");
        try
        {
            var path = Path.Combine(dir.FullName, "sub", "settings.json");
            var s = new PlayerSettings
            {
                StreamingQuality = "720p · 4 Mbps", AutoSkipDelay = 0, PreferredAudioLanguage = "ru", AutoSelectSubtitles = true,
                SubtitleSize = SubtitleSize.Huge, SubtitleBackground = true,
            };
            s.SkipModes[SegmentKind.Intro] = SkipMode.Off;
            s.SubtitleNudges["m|English"] = -0.5;
            s.Save(path);
            var back = PlayerSettings.Load(path);
            Assert.Equal("720p · 4 Mbps", back.StreamingQuality);
            Assert.Equal(0, back.AutoSkipDelay);
            Assert.Equal("ru", back.PreferredAudioLanguage);
            Assert.True(back.AutoSelectSubtitles);
            Assert.Equal(SubtitleSize.Huge, back.SubtitleSize);
            Assert.True(back.SubtitleBackground);
            Assert.Equal(SkipMode.Off, back.SkipMode(SegmentKind.Intro));
            Assert.Equal(SkipMode.Button, back.SkipMode(SegmentKind.Credits));
            Assert.Equal(-0.5, back.SubtitleNudges["m|English"]);
            Assert.False(System.IO.File.Exists(path + ".tmp"));

            System.IO.File.WriteAllText(path, "{ not json");
            Assert.Equal("Original", PlayerSettings.Load(path).StreamingQuality);
            Assert.Equal("en", PlayerSettings.Load(Path.Combine(dir.FullName, "none.json")).PreferredSubtitleLanguage);
            // A file from an older app, with fewer keys: the rest are the defaults.
            Assert.Equal(3, PlayerSettings.FromJson("""{"SubtitleBackground": true}""").AutoSkipDelay);
        }
        finally { dir.Delete(true); }
    }

    [Fact]
    public void PickingTracksInThePlayerSetsThePreferences()
    {
        var s = new PlayerSettings();
        s.SubtitlePicked("ru", "Russian · SDH");
        Assert.Equal(("ru", true, true), (s.PreferredSubtitleLanguage, s.PreferSdhSubtitles, s.AutoSelectSubtitles));
        // Forced subtitles come on by themselves: picking them is like Off.
        s.SubtitlePicked("en", "English · Forced");
        Assert.Equal(("ru", false), (s.PreferredSubtitleLanguage, s.AutoSelectSubtitles));
        s.SubtitlePicked("fr", "French");
        Assert.Equal(("fr", false, true), (s.PreferredSubtitleLanguage, s.PreferSdhSubtitles, s.AutoSelectSubtitles));
        s.SubtitlePicked(null, null);
        Assert.False(s.AutoSelectSubtitles);
        Assert.Equal("fr", s.PreferredSubtitleLanguage);
        s.AudioPicked("rus");
        Assert.Equal("ru", s.PreferredAudioLanguage);
        s.AudioPicked("und");
        Assert.Equal("ru", s.PreferredAudioLanguage);
    }

    [Fact]
    public void TheSettingsLanguagesAreUsedToStartATitle()
    {
        AudioCandidate[] audio = [new(1, "en", true, "English"), new(2, "ru", false, "Russian")];
        Assert.Equal(2, TrackRules.PreferredAudio(audio, new PlayerSettings { PreferredAudioLanguage = "ru" }.PreferredAudioLanguage));
        Assert.Null(TrackRules.PreferredAudio(audio, new PlayerSettings().PreferredAudioLanguage));
        SubtitleCandidate[] subs = [new("s3", "English", "en", 3), new("s4", "Russian", "ru", 4)];
        var on = new PlayerSettings { AutoSelectSubtitles = true, PreferredSubtitleLanguage = "ru" };
        Assert.Equal("s4", TrackRules.UsualSubtitle(subs, on.AutoSelectSubtitles, on.PreferredSubtitleLanguage, on.PreferSdhSubtitles, "en")?.Id);
        var off = new PlayerSettings();
        Assert.Null(TrackRules.UsualSubtitle(subs, off.AutoSelectSubtitles, off.PreferredSubtitleLanguage, off.PreferSdhSubtitles, "en"));
        Assert.Equal(PlayerSettings.Languages, PlayerSettings.LanguageChoices("de"));
        Assert.Equal(("hu", "Hungarian"), PlayerSettings.LanguageChoices("hu")[^1]);
    }

    // ---------------------------------------------------------------- episodes and thumbnails

    private static Item Ep(string id, int season, int n, string? seasonTitle = null) =>
        new() { Id = id, Kind = "episode", SeasonNo = season, EpisodeNo = n, SeasonTitle = seasonTitle, Runtime = 2700 };

    [Fact]
    public void EpisodesAreGroupedBySeasonInAiringOrder()
    {
        var groups = EpisodeGroups.Seasons([Ep("a", 1, 1), Ep("b", 1, 2), Ep("c", 2, 1, "The Return"), Ep("x", 0, 1, "Specials")]);
        Assert.Equal(["Season 1", "The Return", "Specials"], groups.Select(g => g.Title));
        Assert.Equal(["a", "b"], groups[0].Episodes.Select(e => e.Id));
        Assert.Equal("Now Playing", EpisodeGroups.Detail(Ep("b", 1, 2), "b"));
        Assert.Equal("45m", EpisodeGroups.Detail(Ep("a", 1, 1), "b"));
        Assert.Equal("Episodes", EpisodeGroups.Seasons([new Item { Id = "z", Kind = "episode" }])[0].Title);
    }

    [Fact]
    public void TheScrubPreviewFollowsTheKnobInsideTheTrack()
    {
        Assert.Equal(0, ScrubPreview.Left(10, 1000));
        Assert.Equal(390, ScrubPreview.Left(500, 1000));
        Assert.Equal(780, ScrubPreview.Left(990, 1000));
        Assert.Equal(-10, ScrubPreview.Left(50, 200));
        var t = new Thumbnails { State = "ready", Interval = 10, Width = 320, Height = 180, Columns = 10, Rows = 10, Count = 150, Sheets = ["/0.jpg", "/1.jpg"] };
        Assert.Equal(124, ScrubPreview.Height(t));
        Assert.Equal(new Thumbnails.Tile(1, 320 * 3, 180 * 2), t.TileAt(1235));
        Assert.Equal(new Thumbnails.Tile(1, 320 * 9, 180 * 4), t.TileAt(99999)); // the last picture
    }

    // ---------------------------------------------------------------- subtitles

    [Fact]
    public void SrtVttAndAssParse()
    {
        var srt = "1\r\n00:00:01,000 --> 00:00:03,500\r\n<i>Hello</i>\r\nthere\r\n\r\n2\r\n00:00:02,000 --> 00:00:04,000\r\nTwo &amp; more\r\n\r\n";
        var doc = SubtitleParser.Parse(srt);
        Assert.Equal(2, doc.Cues.Count);
        Assert.Equal("Hello\nthere", doc.TextAt(1.5));
        Assert.Equal("Hello\nthere\nTwo & more", doc.TextAt(2.5));
        Assert.Equal("Two & more", doc.TextAt(3.6));
        Assert.Null(doc.TextAt(4.0));
        Assert.Null(doc.TextAt(0.5));
        Assert.Equal(2.0, doc.NextEdge(1.5));
        Assert.Equal(3.5, doc.NextEdge(2.5));
        Assert.Null(doc.NextEdge(4.5));

        var vtt = SubtitleParser.Parse("WEBVTT\n\n00:01.000 --> 00:02.000 align:start\nVTT line\n");
        Assert.Equal("VTT line", vtt.TextAt(1.2));

        var ass = "[Script Info]\nTitle: x\n\n[Events]\nFormat: Layer, Start, End, Style, Text\nDialogue: 0,0:00:05.00,0:00:06.00,Default,{\\i1}Hi,\\Nthere\n";
        Assert.Equal("Hi,\nthere", SubtitleParser.Parse(ass).TextAt(5.5));
    }

    [Fact]
    public void OldSrtFilesDecodeAsWindows1252()
    {
        Assert.Equal("café", SubtitleParser.Decode([0xEF, 0xBB, 0xBF, (byte)'c', (byte)'a', (byte)'f', 0xC3, 0xA9]));
        Assert.Equal("café “x”", SubtitleParser.Decode([(byte)'c', (byte)'a', (byte)'f', 0xE9, (byte)' ', 0x93, (byte)'x', 0x94]));
    }

    [Fact]
    public void TheDelayMenuMovesByStepsAndSaysSo()
    {
        Assert.Equal([-2.0, -1.0, -0.5, -0.1, 0.1, 0.5, 1.0, 2.0], SubtitleDelay.Steps);
        var d = 0.0;
        for (var i = 0; i < 10; i++) d = SubtitleDelay.Nudged(d, 0.1);
        Assert.Equal(1.0, d);
        Assert.Equal("+1.0s", SubtitleDelay.Label(d));
        Assert.Equal("-0.5s", SubtitleDelay.Label(-0.5));
        Assert.Equal("+0.0s", SubtitleDelay.Label(0));
        Assert.Equal("+0.0s", SubtitleDelay.Label(-0.01));
        Assert.Equal("item~v|English", SubtitleDelay.NudgeKey("item~v", "English"));
    }

    [Fact]
    public void SubtitlesSitAboveTheControlsWhenTheyShow()
    {
        Assert.Equal(60, SubtitleLayout.BottomPadding(1000, false, 900));
        Assert.Equal(28, SubtitleLayout.BottomPadding(300, false, 250));
        Assert.Equal(108, SubtitleLayout.BottomPadding(1000, true, 900));
        Assert.Equal(60, SubtitleLayout.BottomPadding(1000, true, 0));
    }

    // ---------------------------------------------------------------- picture subtitles

    /// <summary>A picture track's window as the server answers it (a PGS track, 1920 × 1080).</summary>
    private const string PictureWindowJson = """
        {"index":2,"window":0,"from":0.0,"to":30.0,"span":30.0,"complete":false,"source":"index",
         "canvas":{"width":1920,"height":1080},
         "cues":[{"start":0.023,"end":3.023,"text":"","image":{"x":777,"y":949,"width":354,"height":48,"png":"iVBORw0KGgo="}},
                 {"start":15.023,"end":18.023,"text":"","image":{"x":777,"y":63,"width":360,"height":48,"png":"iVBORw0KGgp="}}]}
        """;

    [Fact]
    public void APictureWindowIsRead()
    {
        var w = System.Text.Json.JsonSerializer.Deserialize<SubtitleWindow>(PictureWindowJson, MoonriseApi.Json)!;
        Assert.False(w.Complete);
        Assert.Equal(new PictureCanvas { Width = 1920, Height = 1080 }, w.Canvas);
        Assert.Equal(2, w.Cues.Count);
        var c = w.Cues[1];
        Assert.Equal((15.023, 18.023, ""), (c.Start, c.End, c.Text));
        Assert.Equal((777, 63, 360, 48), (c.Image!.X, c.Image.Y, c.Image.Width, c.Image.Height));
        Assert.Equal("iVBORw0KGgp=", c.Image.Png);
    }

    [Fact]
    public async Task PictureCuesArePlacedOnTheWindowsCanvasAndNotRepeated()
    {
        var server = new FakeServer();
        IReadOnlyList<SubtitleCue> shown = [];
        var loader = new SubtitleWindowLoader(server.Window, server.FullTrack, c => shown = c, () => { }, (_, _) => Task.CompletedTask);
        var w = System.Text.Json.JsonSerializer.Deserialize<SubtitleWindow>(PictureWindowJson, MoonriseApi.Json)!;
        loader.Advance(1);
        server.Pending[0].SetResult(w);
        await Until(() => shown.Count == 2);
        Assert.All(shown, c => Assert.Equal((1920, 1080), (c.Image!.CanvasWidth, c.Image.CanvasHeight)));
        // The next window has the cue still showing at its start again: one cue, not two.
        await Until(() => server.Pending.ContainsKey(1));
        server.Pending[1].SetResult(w with { Window = 1, From = 30, To = 60, Cues = [w.Cues[1], new SubtitleCue(31, 33, "") { Image = w.Cues[0].Image }] });
        await Until(() => shown.Count == 3);
        Assert.Equal(0, server.FullAsked);
        // Pictures aren't text: the text line stays empty, and the picture shows from start to end.
        var doc = new SubtitleDocument(shown);
        Assert.Null(doc.TextAt(16));
        Assert.Equal(63, Assert.Single(SubtitlePictures.At(doc.Cues, 16)).Image!.Y);
        Assert.Empty(SubtitlePictures.At(doc.Cues, 18.023));
        Assert.Equal(18.023, doc.NextEdge(16));
    }

    [Fact]
    public void PicturesSitOnTheVideoWhereTheTrackPutsThem()
    {
        // 1080p in a 2000 × 1000 window: pillarboxed, 1777.8 wide from x 111.1.
        var video = SubtitlePictures.VideoRect(2000, 1000, 1920, 1080, fill: false)!.Value;
        Assert.Equal(1000, video.Height, 3);
        Assert.Equal(111.111, video.X, 3);
        var r = SubtitlePictures.Place(new CueImage { X = 960, Y = 540, Width = 192, Height = 108, CanvasWidth = 1920, CanvasHeight = 1080 }, video)!.Value;
        Assert.Equal((1000.0, 500.0, 177.778, 100.0), (Math.Round(r.X, 3), Math.Round(r.Y, 3), Math.Round(r.Width, 3), Math.Round(r.Height, 3)));
        // A DVD's 720 × 480 canvas on its 16:9 picture: across and down by their own scales.
        var dvd = SubtitlePictures.VideoRect(1920, 1080, 853, 480, fill: false)!.Value;
        var d = SubtitlePictures.Place(new CueImage { X = 360, Y = 400, Width = 72, Height = 40, CanvasWidth = 720, CanvasHeight = 480 }, dvd)!.Value;
        Assert.Equal(960, d.X + 0, 0);
        Assert.Equal(900, d.Y, 3);
        Assert.Equal(192, d.Width, 0);
        Assert.Equal(90, d.Height, 3);
        // Filling the window: the picture is cut at the sides, and the pictures follow it.
        var fill = SubtitlePictures.VideoRect(1000, 1000, 1920, 1080, fill: true)!.Value;
        Assert.Equal((1000.0, -388.889), (Math.Round(fill.Height, 3), Math.Round(fill.X, 3)));
        Assert.Null(SubtitlePictures.VideoRect(1000, 1000, 0, 0, false));
        Assert.Null(SubtitlePictures.Place(new CueImage(), video));
    }

    [Fact]
    public void PicturesLowDownStayClearOfTheControls()
    {
        // A picture at the bottom moves up above the scrubber while the controls show.
        Assert.Equal(48, SubtitlePictures.Lift(900, 60, 1000, controlsShown: true, bandTop: 920));
        Assert.Equal(0, SubtitlePictures.Lift(900, 60, 1000, controlsShown: false, bandTop: 920));
        Assert.Equal(0, SubtitlePictures.Lift(800, 60, 1000, controlsShown: true, bandTop: 920));
        // One at the top stays where it is.
        Assert.Equal(0, SubtitlePictures.Lift(60, 60, 1000, controlsShown: true, bandTop: 50));
    }

    // ---------------------------------------------------------------- subtitle windows

    private sealed class FakeServer
    {
        public readonly List<int> Asked = [];
        public readonly Dictionary<int, TaskCompletionSource<SubtitleWindow>> Pending = [];
        public int FullAsked;
        public string Full = "1\n00:00:01,000 --> 00:00:02,000\nWhole track\n";

        public Task<SubtitleWindow> Window(int k, CancellationToken ct)
        {
            Asked.Add(k);
            var tcs = new TaskCompletionSource<SubtitleWindow>(TaskCreationOptions.RunContinuationsAsynchronously);
            ct.Register(() => tcs.TrySetCanceled());
            Pending[k] = tcs;
            return tcs.Task;
        }

        public Task<string> FullTrack(CancellationToken ct) { FullAsked++; return Task.FromResult(Full); }

        public void Answer(int k, bool complete = false, params SubtitleCue[] cues) =>
            Pending[k].SetResult(new SubtitleWindow { Window = k, From = k * 30, To = k * 30 + 30, Complete = complete, Cues = cues });

        public void Fail(int k, int code, string body) => Pending[k].SetException(new MoonriseHttpException(code, body));
    }

    private static async Task Until(Func<bool> condition)
    {
        for (var i = 0; i < 200 && !condition(); i++) await Task.Delay(5);
        Assert.True(condition());
    }

    [Fact]
    public async Task WindowsComeAtThePlayheadThenTheOneAfter()
    {
        var server = new FakeServer();
        IReadOnlyList<SubtitleCue> shown = [];
        var fellBack = false;
        var loader = new SubtitleWindowLoader(server.Window, server.FullTrack, c => shown = c, () => fellBack = true, (_, _) => Task.CompletedTask);
        loader.Advance(65); // window 2
        Assert.Equal([2], server.Asked);
        loader.Advance(66); // still fetching the one wanted: no second request
        Assert.Equal([2], server.Asked);
        server.Answer(2, false, new SubtitleCue(61, 63, "<b>Sixty</b>"));
        await Until(() => server.Asked.Count == 2);
        Assert.Equal([2, 3], server.Asked);
        Assert.Equal("Sixty", Assert.Single(shown).Text);
        // A seek elsewhere drops the request for window 3: the playhead's first.
        loader.Advance(305);
        Assert.Equal([2, 3, 10], server.Asked);
        Assert.True(server.Pending[3].Task.IsCanceled);
        server.Answer(10, false, new SubtitleCue(301, 302, "Three hundred"));
        await Until(() => server.Asked.Count == 4);
        Assert.Equal(11, server.Asked[^1]);
        Assert.Equal(["Sixty", "Three hundred"], shown.Select(c => c.Text)); // nothing already shown goes away
        // The server says the whole track is ready: fetched once, and no more windows.
        server.Answer(11, true, new SubtitleCue(331, 332, "Later"));
        await Until(() => server.FullAsked == 1);
        await Until(() => shown.Count == 1 && shown[0].Text == "Whole track");
        loader.Advance(500);
        Assert.Equal(4, server.Asked.Count);
        Assert.False(fellBack);
    }

    [Fact]
    public async Task AServerWithoutWindowsGetsTheWholeTrackAndTheEndIsKnown()
    {
        var server = new FakeServer();
        var fellBack = false;
        var loader = new SubtitleWindowLoader(server.Window, server.FullTrack, _ => { }, () => fellBack = true, (_, _) => Task.CompletedTask);
        loader.Advance(0);
        server.Answer(0);
        await Until(() => server.Asked.Count == 2);
        // Past the end: window 1 is the last there is; nothing past it is asked for.
        server.Fail(1, 404, "window past the end");
        await Task.Delay(30);
        loader.Advance(40);
        Assert.Equal([0, 1], server.Asked);
        Assert.Empty(loader.Needed(40));
        Assert.False(fellBack);

        var old = new FakeServer();
        var loader2 = new SubtitleWindowLoader(old.Window, old.FullTrack, _ => { }, () => fellBack = true, (_, _) => Task.CompletedTask);
        loader2.Advance(0);
        old.Fail(0, 404, "Not Found");
        await Until(() => fellBack);
        loader2.Advance(100);
        Assert.Single(old.Asked);
    }

    [Fact]
    public async Task AFailedWindowIsAskedForAgain()
    {
        var server = new FakeServer();
        var waits = new List<TimeSpan>();
        var loader = new SubtitleWindowLoader(server.Window, server.FullTrack, _ => { }, () => { }, (d, _) => { waits.Add(d); return Task.CompletedTask; });
        loader.Advance(0);
        server.Fail(0, 500, "busy");
        await Until(() => server.Asked.Count == 2);
        server.Fail(0, 500, "busy");
        await Until(() => server.Asked.Count == 3);
        Assert.Equal([0, 0, 0], server.Asked);
        Assert.Equal([TimeSpan.FromMilliseconds(500), TimeSpan.FromMilliseconds(1000)], waits);
        loader.Stop();
        Assert.True(server.Pending[0].Task.IsCanceled);
    }
}
