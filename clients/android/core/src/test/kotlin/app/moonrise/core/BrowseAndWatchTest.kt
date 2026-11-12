package app.moonrise.core

import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The Library, search, versions and watch-state calls, as the Apple apps make them. */
class BrowseAndWatchTest {
    private fun withServer(vararg bodies: String, block: suspend (MoonriseApi, MockWebServer) -> Unit) {
        val server = MockWebServer()
        bodies.forEach { server.enqueue(MockResponse().setBody(it).setHeader("Content-Type", "application/json")) }
        server.start()
        try {
            runBlocking { block(MoonriseApi(server.url("/"), token = "t0k", deviceId = "dev", deviceName = "Test TV"), server) }
        } finally {
            server.shutdown()
        }
    }

    @Test
    fun libraryPageSendsFiltersAndReadsIndex() = withServer(
        """{"items":[{"id":"m1","kind":"movie","title":"Alpha"}],"total":120,"offset":0,
           "index":[{"letter":"#","offset":0},{"letter":"A","offset":4}]}""",
    ) { api, server ->
        val f = LibraryFilter(sort = LibrarySort.Title, genres = setOf("Drama", "Action"), decade = 1990,
            watched = WatchedFilter.Unwatched, resolutions = setOf(ResolutionFilter.Sd, ResolutionFilter.Uhd), ratings = setOf("MA 15+"))
        val page = api.items(LibraryView.Movies, f, offset = 60, limit = 60)
        assertEquals(120, page.total)
        assertEquals(ItemPage.Letter("A", 4), page.index[1])
        val url = server.takeRequest().requestUrl!!
        assertEquals("/v1/items", url.encodedPath)
        assertEquals("movie", url.queryParameter("type"))
        assertEquals("title", url.queryParameter("sort"))
        assertEquals("asc", url.queryParameter("order"))
        assertEquals("60", url.queryParameter("offset"))
        assertEquals("Action,Drama", url.queryParameter("genre"))
        assertEquals("1990", url.queryParameter("year_from"))
        assertEquals("1999", url.queryParameter("year_to"))
        assertEquals("1", url.queryParameter("unplayed"))
        assertEquals("4k,sd", url.queryParameter("resolution"))
        // "+" must reach the server as a plus, not a space.
        assertTrue(url.encodedQuery!!.contains("rating=MA%2015%2B"))
        assertEquals("MA 15+", url.queryParameter("rating"))
    }

    @Test
    fun facetsAndCategoryTiles() = withServer(
        """{"total":300,"genres":[{"name":"Drama","count":12,"backdrop":"b1"}],"studios":[],"decades":[{"decade":1990,"count":5}],
           "ratings":[{"name":"none","count":2}],"collections":[{"id":7,"name":"Saga","count":3,"poster":"p7"}],
           "resolutions":[{"name":"4k","count":31}],"hdr":24,"watched":{"all":300,"in_progress":2,"played":5,"unplayed":293},
           "recently_added":{"count":17,"days":30},"libraries":[{"id":"movies","name":"Media","count":300}],"kind":"movie"}""",
    ) { api, server ->
        val f = api.facets(LibraryView.Shows, LibraryFilter(hdr = true))
        assertEquals(293, f.watched!!.count(WatchedFilter.Unwatched))
        assertEquals(17, f.recentlyAdded!!.count)
        val decade = f.tiles(LibraryCategory.Decades).single()
        assertEquals("1990s", decade.title)
        assertEquals(LibrarySort.Year, decade.filter.sort)
        assertFalse(decade.filter.descending)
        assertEquals(CollectionRef(7, "Saga"), f.tiles(LibraryCategory.Collections).single().filter.collection)
        assertEquals(setOf("Drama"), f.tiles(LibraryCategory.Genres).single().filter.genres)
        val url = server.takeRequest().requestUrl!!
        assertEquals("/v1/facets", url.encodedPath)
        assertEquals("series", url.queryParameter("type"))
        assertEquals("1", url.queryParameter("hdr"))
        assertEquals("30", url.queryParameter("recent_days"))
    }

    @Test
    fun filterSummaryAndClear() {
        val f = LibraryFilter(sort = LibrarySort.Added, descending = true, genres = setOf("Action"), decade = 1990,
            watched = WatchedFilter.Unwatched, ratings = setOf("none"))
        assertEquals("Action · 1990s · Not Rated · Unwatched", f.summary)
        assertEquals(4, f.activeCount)
        assertTrue(f.isFiltered)
        assertEquals(LibraryFilter(sort = LibrarySort.Added, descending = true), f.cleared)
        assertFalse(f.cleared.isFiltered)
        assertEquals(LibrarySort.Rating to true, f.sortedBy(LibrarySort.Rating).let { it.sort to it.descending })
        assertEquals("Newest First", LibrarySort.Added.directions.second)
        // Kept as JSON between launches.
        val json = Json.encodeToString(LibraryFilter.serializer(), f)
        assertEquals(f, Json.decodeFromString(LibraryFilter.serializer(), json))
    }

    @Test
    fun searchSplitsTitlesAndEpisodes() = withServer(
        """{"titles":[{"id":"s1","kind":"series","title":"Friends"},{"id":"m1","kind":"movie","title":"Friends with Benefits"}],
           "episodes":[{"id":"e1","kind":"episode","title":"Pilot","series_title":"Friends","season_no":1,"episode_no":1}]}""",
    ) { api, server ->
        val r = api.search("friends")
        assertEquals(listOf("s1", "m1"), r.titles.map { it.id })
        assertEquals("S1 · E1", r.episodes[0].episodeLabel)
        assertEquals("friends", server.takeRequest().requestUrl!!.queryParameter("q"))
    }

    @Test
    fun marksAndVersionChoice() = withServer("""{"changed":1}""", """{"changed":1}""", """{"version":"f2"}""") { api, server ->
        api.setPlayed("e1", true)
        api.setPlayed("e1", false)
        api.chooseVersion("m1", "f2")
        server.takeRequest().let { assertEquals("PUT", it.method); assertEquals("/v1/items/e1/played", it.path) }
        server.takeRequest().let { assertEquals("DELETE", it.method); assertEquals("/v1/items/e1/played", it.path) }
        server.takeRequest().let {
            assertEquals("PUT", it.method)
            assertEquals("/v1/items/m1/version", it.path)
            assertEquals("f2", Json.parseToJsonElement(it.body.readUtf8()).jsonObject["file_id"]!!.jsonPrimitive.content)
            assertEquals("Bearer t0k", it.getHeader("Authorization"))
        }
    }

    @Test
    fun reportsAsTheAppleApps() = withServer("""{"recorded":true}""", """{"recorded":true}""", """{"recorded":true}""") { api, server ->
        api.report("sid1", ReportEvent.Start, 12.5, paused = false, audio = 2)
        api.report("sid1", ReportEvent.Progress, 22.5, paused = true, audio = 2, credits = 7409.25)
        api.report("sid1", ReportEvent.Stop, 30.0, paused = true)
        for ((path, pos, paused) in listOf(Triple("start", 12.5, false), Triple("progress", 22.5, true), Triple("stop", 30.0, true))) {
            val r = server.takeRequest()
            assertEquals("POST", r.method)
            assertEquals("/v1/sessions/sid1/$path", r.path)
            assertEquals("Bearer t0k", r.getHeader("Authorization"))
            assertTrue(r.getHeader("Content-Type")!!.startsWith("application/json"))
            val body = Json.parseToJsonElement(r.body.readUtf8()).jsonObject
            assertEquals(pos, body["position"]!!.jsonPrimitive.content.toDouble(), 1e-9)
            assertEquals(paused, body["paused"]!!.jsonPrimitive.content.toBoolean())
            if (path == "stop") assertNull(body["audio"]) else assertEquals(2, body["audio"]!!.jsonPrimitive.content.toInt())
            // The credits start, when known.
            if (path == "progress") assertEquals(7409.25, body["credits"]!!.jsonPrimitive.content.toDouble(), 1e-9) else assertNull(body["credits"])
        }
    }

    @Test
    fun playbackSendsQuality() = withServer("""{"supported":true,"master":"/v1/s/x/master.m3u8","session":"x"}""") { api, server ->
        api.playback("m1", start = null, hdr = false, version = "f1", quality = 4_000_000)
        val url = server.takeRequest().requestUrl!!
        assertEquals("4000000", url.queryParameter("quality"))
        assertEquals("f1", url.queryParameter("version"))
    }

    private fun file(id: String, broken: Boolean = false, langs: List<String> = emptyList(), hdr: String? = null, height: Int = 1080, pos: Double? = null) =
        MediaFile(id = id, broken = broken, position = pos, version = VersionInfo(label = id, hdr = hdr, audioLanguages = langs),
            tracks = listOf(Track(index = 0, kind = "video", height = height)))

    @Test
    fun versionRules() {
        val healthy = file("a", langs = listOf("rus", "eng"), height = 720)
        val damaged = file("b", broken = true, langs = listOf("eng"))
        val item = Item(id = "m", kind = "movie", files = listOf(healthy, damaged), user = UserData(version = "b"))
        // A damaged version is never the one while there's another, even when chosen.
        assertEquals("a", Versions.chosen(item, pick = null, hdrDisplay = true, language = "en")!!.id)
        assertEquals("Damaged or incomplete file · can't play", damaged.versionDetails)
        // The preferred language, where the versions differ in it.
        val ru = file("ru", langs = listOf("rus"))
        val en = file("en", langs = listOf("eng"))
        assertEquals("en", Versions.best(listOf(ru, en), hdrDisplay = true, language = "en")!!.id)
        assertEquals("ru", Versions.best(listOf(ru, en), hdrDisplay = true, language = "ru-RU")!!.id)
        // No HDR display: an SDR version as sharp.
        val dv = file("dv", hdr = "Dolby Vision", height = 2160)
        val sdr4k = file("sdr", height = 2160)
        assertEquals("sdr", Versions.best(listOf(dv, sdr4k), hdrDisplay = false, language = null)!!.id)
        assertEquals("dv", Versions.best(listOf(dv, sdr4k), hdrDisplay = true, language = null)!!.id)
        // A pick made here wins over the server's.
        val two = Item(id = "m", kind = "movie", files = listOf(ru, en), user = UserData(version = "ru"))
        assertEquals("en", Versions.chosen(two, pick = "en", hdrDisplay = true, language = null)!!.id)
        assertEquals("ru", Versions.chosen(two, pick = null, hdrDisplay = true, language = "en")!!.id)
        // Each version keeps its own resume point.
        val cut = Item(id = "m", kind = "movie", files = listOf(file("x", pos = 300.0), file("y")), user = UserData(position = 300.0))
        assertEquals(300.0, Versions.resume(cut, cut.files!![0])!!, 1e-9)
        assertNull(Versions.resume(cut, cut.files!![1]))
    }

    @Test
    fun watchActionsAndMarks() {
        val fresh = Item(id = "m", kind = "movie", user = UserData(position = 0.0, played = false))
        assertEquals(listOf(WatchAction.Watched), WatchAction.options(fresh))
        val started = fresh.copy(user = UserData(position = 120.0))
        assertEquals(listOf(WatchAction.Watched, WatchAction.Unwatched), WatchAction.options(started))
        val done = fresh.copy(user = UserData(played = true))
        assertEquals(listOf(WatchAction.Unwatched), WatchAction.options(done))
        val show = Item(id = "s", kind = "series", user = UserData(lastPlayedAt = "2026-11-01T10:00:00Z", unplayedItemCount = 4))
        assertEquals(2, WatchAction.options(show).size)
        val marked = show.marked(true)
        assertEquals(true, marked.user!!.played)
        assertEquals(0, marked.user!!.unplayedItemCount)
        assertNull(started.marked(true).resumeSeconds)
    }

    @Test
    fun rewatchResumeCardShowsProgress() {
        val rewatch = Item(id = "m", kind = "movie", runtime = 1000.0, user = UserData(position = 250.0, played = true))
        // Its Resume button, on every surface: the position, though it was watched before.
        assertEquals(250.0, rewatch.resumeSeconds!!, 1e-9)
        assertEquals(250.0, Versions.resume(rewatch, null)!!, 1e-9)
        // On a title page or another row: watched, so no bar.
        assertNull(rewatch.progress)
        // As a Continue Watching resume card: the bar shows where the rewatch is.
        assertEquals(0.25, rewatch.copy(continueInfo = ContinueInfo(reason = "resume")).progress!!, 1e-9)
        // A next card for a watched title has no bar.
        assertNull(rewatch.copy(continueInfo = ContinueInfo(reason = "next_episode")).progress)
        // Resume counts from above 0; the bar still starts at a second (not a change of the bar).
        val moment = Item(id = "m", kind = "movie", runtime = 1000.0, user = UserData(position = 0.5))
        assertEquals(0.5, moment.resumeSeconds!!, 1e-9)
        assertNull(moment.progress)
    }

    @Test
    fun episodeListTitles() {
        val e = Item(id = "e", kind = "episode", title = "Pilot", episodeNo = 12, episodeEnd = 13)
        assertEquals("12–13. Pilot", e.episodeListTitle)
        assertEquals("23. Part 2 · Finale", Item(id = "f", kind = "episode", title = "Finale", episodeNo = 23, part = 2).episodeListTitle)
        assertEquals("Director", Person(id = "1", kind = "director").group)
        assertEquals("Actor", Person(id = "2", kind = "Actor").group)
    }

    @Test
    fun qualityIsCappedToTheSource() {
        val q = Quality.of("4K · 40 Mbps")
        // A 1080p file at 8 Mbps: 4K and 1080p presets are no smaller.
        assertEquals(Quality.of("720p · 4 Mbps"), q.capped(8_000_000, 1080))
        assertEquals(Quality.original, Quality.of("480p · 1.5 Mbps").capped(1_000_000, 480))
        assertEquals("fr", Languages.normalized("fre"))
        assertEquals("ru", Languages.normalized("rus"))
        assertNull(Languages.normalized("und"))
    }
}
