package app.moonrise.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.Instant

/** The download and offline rules, as the iPhone's (`DownloadRecord`, `DownloadsView`, `OfflineTitlePage`, `LocalReports`). */
class DownloadsTest {
    private fun rec(id: String, state: DownloadState = DownloadState.Completed, item: Item? = null, series: Item? = null, done: Int? = null) =
        DownloadRecord(id, state, title = item?.title ?: id, serverId = "srv", itemId = item?.id ?: id, qualityLabel = "720p · 4 Mbps",
            item = item, series = series, setBase = "/v1/items/x/offline/v", segmentCount = 10, segmentsDone = done)

    private fun episode(id: String, s: Int, e: Int, show: String = "show1") =
        Item(id = id, kind = "episode", title = "E$e", seriesId = show, seriesTitle = show, seasonId = "s$s", seasonNo = s, episodeNo = e)

    @Test
    fun playableOnceSomethingIsDown() {
        assertTrue(rec("a").isPlayable)
        assertFalse(rec("b", DownloadState.Queued, done = 0).isPlayable)
        assertTrue(rec("c", DownloadState.Downloading, done = 3).isPlayable)
    }

    @Test
    fun statusLines() {
        assertEquals("Downloaded · 720p · 4 Mbps", rec("a").statusLine)
        assertEquals("Downloading 34 %", rec("a", DownloadState.Downloading).copy(progress = 0.34).statusLine)
        assertEquals("Paused · 34%", rec("a", DownloadState.Paused).copy(progress = 0.34).rowDetail)
        assertEquals("Waiting…", rec("a", DownloadState.Queued).rowDetail)
    }

    @Test
    fun groupsAsTheDownloadsTab() {
        val film = rec("f", item = Item(id = "f", kind = "movie", title = "Film"))
        val going = rec("g", DownloadState.Downloading, item = Item(id = "g", kind = "movie", title = "Going"))
        assertEquals("Ready to Watch", DownloadRules.groups(listOf(film)).moviesTitle)
        val mixed = DownloadRules.groups(listOf(film, going, rec("e2", item = episode("e2", 1, 2)), rec("e1", item = episode("e1", 1, 1))))
        assertEquals("Movies", mixed.moviesTitle)
        assertEquals(listOf("g"), mixed.inProgress.map { it.id })
        assertEquals(listOf("e1", "e2"), mixed.shows.single().episodes.map { it.id })
    }

    @Test
    fun nextSegmentFromThePlayheadThenTheStart() {
        assertEquals(5, DownloadRules.nextSegment(setOf(0, 1), emptySet(), 5, 10))
        assertEquals(2, DownloadRules.nextSegment(setOf(0, 1, 5, 6, 7, 8, 9), emptySet(), 5, 10))
        assertNull(DownloadRules.nextSegment((0..9).toSet(), emptySet(), null, 10))
        assertEquals(3, DownloadRules.nextSegment(setOf(0, 1), setOf(2), null, 10))
    }

    @Test
    fun contiguousAndSeconds() {
        assertEquals(2, DownloadRules.contiguous(setOf(0, 1, 3)))
        assertEquals(10.0, DownloadRules.seconds("#EXTINF:6.0,\na\n#EXTINF:4.0,\nb\n#EXTINF:6.0,\nc\n", 2), 0.001)
    }

    @Test
    fun aGrowingPlaylistEndsWhereTheDownloadDoes() {
        val p = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:6.0,\n0.m4s\n#EXTINF:6.0,\n1.m4s\n#EXTINF:6.0,\n2.m4s\n#EXT-X-ENDLIST\n"
        val t = DownloadRules.truncated(p, 2)
        assertTrue(t.contains("0.m4s") && t.contains("1.m4s"))
        assertFalse(t.contains("2.m4s"))
        assertTrue(t.trim().endsWith("#EXT-X-ENDLIST"))
        assertTrue(t.contains("init.mp4"))
    }

    @Test
    fun anOfflineShowsPage() {
        val show = Item(id = "show1", kind = "series", title = "Show")
        val records = listOf(
            rec("b", item = episode("e2", 2, 1), series = show),
            rec("a", item = episode("e1", 1, 1), series = show),
            rec("z", DownloadState.Queued, item = episode("e9", 3, 1)),
        )
        val page = OfflineTitlePage.build("show1", records)!!
        assertEquals("Show", page.item.title)
        assertEquals(listOf("s1", "s2"), page.seasons.map { it.id })
        assertEquals(listOf("e1"), page.episodes(page.seasons.first()).map { it.id })
        assertEquals("s2", page.season(episode("e2", 2, 1))?.id)
        assertNull(OfflineTitlePage.build("nothing", records))
    }

    @Test
    fun aFilmsOfflinePage() {
        val film = Item(id = "f", kind = "movie", title = "Film")
        val page = OfflineTitlePage.build("f", listOf(rec("x", item = film)))!!
        assertEquals("f", page.item.id)
        assertTrue(page.seasons.isEmpty())
    }

    @Test
    fun nextEpisodeUsesTheDownloadedEpisodes() {
        val eps = DownloadRules.episodes(listOf(rec("b", item = episode("e3", 1, 3)), rec("a", item = episode("e1", 1, 1))), "show1").mapNotNull { it.item }
        assertEquals("e3", NextEpisodeRules.next(eps, episode("e1", 1, 1))?.id)
    }

    @Test
    fun theOfflineLibraryFiltersAndSorts() {
        val a = Item(id = "a", kind = "movie", title = "The Alpha", year = 1995, genres = listOf("Drama"), addedAt = "2026-01-02T00:00:00Z")
        val b = Item(id = "b", kind = "movie", title = "Beta", year = 2005, genres = listOf("Action"), addedAt = "2026-02-02T00:00:00Z")
        val records = listOf(rec("x", item = b), rec("y", item = a))
        assertEquals(listOf("a", "b"), DownloadRules.offlineLibrary(records, false, LibraryFilter()).map { it.id })
        assertEquals(listOf("b", "a"), DownloadRules.offlineLibrary(records, false, LibraryFilter(sort = LibrarySort.Added, descending = true)).map { it.id })
        assertEquals(listOf("a"), DownloadRules.offlineLibrary(records, false, LibraryFilter(genres = setOf("drama"))).map { it.id })
        assertEquals(listOf("b"), DownloadRules.offlineLibrary(records, false, LibraryFilter(decade = 2000)).map { it.id })
        assertEquals("A", DownloadRules.letter("The Alpha"))
    }

    @Test
    fun recentlyAddedIsDated() {
        val now = Instant.parse("2026-03-01T00:00:00Z")
        val a = Item(id = "a", kind = "movie", title = "A", addedAt = "2026-02-20T00:00:00Z")
        val old = Item(id = "b", kind = "movie", title = "B", addedAt = "2025-02-20T00:00:00Z")
        assertTrue(LibraryFilter(recentlyAdded = true).matches(a, now = now))
        assertFalse(LibraryFilter(recentlyAdded = true).matches(old, now = now))
    }

    @Test
    fun aShowsTitleInTheOfflineLibrary() {
        val ep = episode("e1", 1, 1, "show1").copy(parentImages = ItemImages(poster = "p"))
        val titles = DownloadRules.titles(listOf(rec("a", item = ep)), shows = true)
        assertEquals("show1", titles.single().item.id)
        assertEquals("p", titles.single().item.images?.poster)
        assertEquals(listOf("e1"), titles.single().episodes.map { it.id })
    }

    @Test
    fun reportsKeepOnlyTheLatestProgressOfAViewing() {
        fun r(v: String, e: String, p: Double) = LocalReport("srv", v, "item", e, p, 1)
        var q = LocalReport.added(emptyList(), r("v", "start", 0.0))
        q = LocalReport.added(q, r("v", "progress", 10.0))
        q = LocalReport.added(q, r("v", "progress", 20.0))
        q = LocalReport.added(q, r("w", "progress", 5.0))
        assertEquals(listOf("v/start/0.0", "v/progress/20.0", "w/progress/5.0"), q.map { "${it.viewing}/${it.event}/${it.position}" })
        assertEquals(1, q.first().json().size - 4)
    }

    @Test
    fun badRequestsAreDroppedOthersKept() {
        assertTrue(LocalReport.dropOnFailure(400))
        assertFalse(LocalReport.dropOnFailure(401))
        assertFalse(LocalReport.dropOnFailure(403))
        assertFalse(LocalReport.dropOnFailure(500))
        assertFalse(LocalReport.dropOnFailure(null))
    }

    @Test
    fun recordsSurviveTheirIndexFile() {
        val r = rec("a", item = Item(id = "a", kind = "movie", title = "T"), done = 2).copy(segments = listOf(Segment(SegmentKind.Intro, 1.0, 2.0)), audioTracks = listOf(PlaybackTrack(1, "English", "en", true)))
        assertEquals(listOf(r), DownloadRecord.listFrom(DownloadRecord.listJson(listOf(r))))
        val c = TrackChoice(TrackPick(index = 1), null)
        assertEquals(c, TrackChoice.fromJson(c.toJson()))
        assertTrue(c.memory.subtitleOff && c.memory.sameFile)
    }

    @Test
    fun theSetTheServerDescribes() {
        val set = MoonriseApi.json.decodeFromString(OfflineSet.serializer(), """{"version":"v","variant":{"id":"copy","label":"Original","bitrate":1000,"transcode":false},"duration":150.5,"segments":3,"audio":[{"index":2,"name":"English - AAC","language":"en","default":true}],"subtitles":[{"index":3,"name":"English - SUBRIP","language":"en","forced":false}],"master":"m","playlist":"p","subs_playlist":"s","expected":999,"base":"/b"}""")
        val r = DownloadRules.described(rec("a").copy(subtitles = listOf(LocalSubtitle("English", "en", "sub-3.srt"))), set)
        assertEquals("Original", r.qualityLabel)
        assertEquals(3, r.segmentCount)
        assertEquals(listOf("video", "audio/2"), DownloadRules.tracks(r))
        assertEquals("English - SUBRIP", r.subtitles.single().rendition)
    }

    @Test
    fun bytesAsTheFileSizesRead() {
        assertEquals("64 MB", bytesText(64_000_000))
        assertEquals("39.7 MB", bytesText(39_700_000))
        assertEquals("1.35 GB", bytesText(1_350_000_000))
        assertEquals("12 KB", bytesText(12_000))
    }

    @Test
    fun qualitySizes() {
        assertEquals("≈ 450 MB", Quality.presets.first { it.bitrate == 4_000_000L }.sizeLabel(900.0, 123))
        assertEquals("1.5 GB", Quality.original.sizeLabel(900.0, 1_500_000_000))
        assertNull(Quality.presets.first().sizeLabel(null, null))
    }
}
