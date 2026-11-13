// SPDX-License-Identifier: MPL-2.0
package app.moonrise.core

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import okhttp3.HttpUrl.Companion.toHttpUrl
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.Instant
import java.time.ZoneOffset

/** Manage Server's, the setup's and Fix Match's rules ([AdminRules], [FixMatchRules], [AdminFmt]), as on the iPhone. */
class AdminRulesTest {
    private val now = Instant.parse("2026-11-09T12:00:00Z")

    @Test
    fun datesAreRelative() {
        assertEquals("just now", AdminFmt.ago("2026-11-09T11:59:30.000Z", now))
        assertEquals("12 min. ago", AdminFmt.ago("2026-11-09T11:48:00Z", now))
        assertEquals("3 hr. ago", AdminFmt.ago("2026-11-09T09:00:00.000Z", now))
        assertEquals("2 days ago", AdminFmt.ago("2026-11-07T11:00:00Z", now))
        assertEquals("in 6 days", AdminFmt.ago("2026-11-15T20:00:00Z", now))
        assertNull(AdminFmt.ago(null, now))
    }

    @Test
    fun uptimeBytesAndCounts() {
        assertEquals("2 days, 3 h", AdminFmt.uptime(2 * 86_400 + 3 * 3600 + 5))
        assertEquals("4 h 2 min", AdminFmt.uptime(4 * 3600 + 120))
        assertEquals("1 min", AdminFmt.uptime(10))
        assertEquals("None", AdminFmt.bytes(0))
        assertEquals("3 GB", AdminFmt.bytes(3L * 1024 * 1024 * 1024))
        assertEquals("1 MB", AdminFmt.bytes(1_069_887))
        assertEquals("1.7 MB", AdminFmt.bytes(1_730_216))
        assertEquals("2,589 episodes", AdminFmt.count(2589, "episode"))
        assertEquals("1 library", AdminFmt.count(1, "library", "libraries"))
    }

    /**
     * A new invitation (expiring seven days after the server made it) said "in 1 wk." and,
     * a minute later, "in 6 days". Whole seconds rounded the moment since it was made away.
     */
    @Test
    fun aMomentLessThanAWeekIsSixDays() {
        val week = now.plusSeconds(7 * 86_400).toString()
        assertEquals("in 1 wk.", AdminFmt.ago(week, now))
        assertEquals("in 6 days", AdminFmt.ago(week, now.plusMillis(1)))
        assertEquals("in 6 days", AdminFmt.ago(week, now.plusMillis(400)))
        assertEquals("in 6 days", AdminFmt.ago(week, now.plusSeconds(60)))
        assertEquals("6 days ago", AdminFmt.ago(now.minusSeconds(7 * 86_400).plusMillis(400).toString(), now))
    }

    /**
     * The pages under the top one stay composed, so Manage Server's summary was only ever
     * loaded once and stayed stale on the way back from a child page. It loads each time it shows.
     */
    @Test
    fun manageServerReloadsItsSummaryWhenItShowsAgain() {
        val ui = "../phone/src/main/kotlin/app/moonrise/phone/ui"
        val host = java.io.File("$ui/ManageParts.kt").readText().substringAfter("fun Host()").substringBefore("val LocalPages")
        assertTrue("only the top page counts as showing", "LocalPageShowing provides (p === pages.last())" in host)
        val page = java.io.File("$ui/ManageServer.kt").readText().substringAfter("fun ManageServerPage(").substringBefore("FormPage(")
        assertTrue("the summary's load must run again when the page shows", "LaunchedEffect(showing)" in page)
        assertFalse("nor once and for all", "LaunchedEffect(Unit)" in page)
        assertTrue("and only while it shows", "if (!showing) return@LaunchedEffect" in page)
    }

    /**
     * Compose keeps the first `confirmValueChange` a swipe state was given, for as long as its
     * place in the composition lasts. So `SwipeToRemove` has to go through the current `onRemove`, and
     * the invitation rows have to be tied to their invitation: a new invitation moves the others down,
     * and a swipe on the top row cancelled the one first drawn there.
     */
    @Test
    fun aSwipeRemovesTheRowItIsOn() {
        val ui = "../phone/src/main/kotlin/app/moonrise/phone/ui"
        val swipe = java.io.File("$ui/ManageServer.kt").readText()
            .substringAfter("fun SwipeToRemove(").substringBefore("SwipeToDismissBox(state")
        assertTrue("SwipeToRemove must keep the current onRemove", "rememberUpdatedState(onRemove)" in swipe)
        assertFalse("SwipeToRemove must not call the first composition's onRemove", "onRemove()" in swipe)
        val invitation = java.io.File("$ui/ManageAccounts.kt").readText()
            .substringAfter("invites.forEachIndexed").substringBefore("SwipeToRemove(")
        assertTrue("each invitation row must be keyed by its id", "key(inv.id)" in invitation)
    }

    /**
     * A form card only painted its rounded shape, so the opaque background of a swipe row
     * at the top of the card squared off the corners. The card has to clip its rows, and the red
     * action behind a row only exists during a swipe, or it shows along the clipped corners.
     * This reads the source; how the corners look was checked on a device.
     */
    @Test
    fun aFormCardClipsItsRowsToItsRoundedCorners() {
        val ui = "../phone/src/main/kotlin/app/moonrise/phone/ui"
        val card = java.io.File("$ui/SignInScreen.kt").readText()
            .substringAfter("fun FormSection(").substringBefore("if (footer != null)")
        assertTrue("FormSection must clip to its shape before painting", ".clip(RoundedCornerShape(26.dp)).background(" in card)
        val swipe = java.io.File("$ui/ManageServer.kt").readText()
            .substringAfter("fun SwipeToRemove(").substringBefore("private fun Modifier.padding16")
        assertTrue(
            "the red action must only be drawn during a swipe",
            swipe.substringAfter("backgroundContent = {").substringBefore("Theme.destructive")
                .contains("if (state.dismissDirection == SwipeToDismissBoxValue.EndToStart)"),
        )
    }

    @Test
    fun scanPhaseAndCounts() {
        val p = ScanProgress("scanning", walked = 40)
        assertEquals("Looking for videos… 40 files", AdminRules.scanPhaseText(p, now))
        assertEquals("Recognizing titles… 1 of 3", AdminRules.scanPhaseText(ScanProgress("identifying", identify = ScanIdentify(1, 3)), now))
        assertEquals("Up to date · scanned just now", AdminRules.scanPhaseText(ScanProgress("idle", lastPass = ScanLastPass(at = now.epochSecond - 10)), now))
        assertEquals("Up to date", AdminRules.scanPhaseText(ScanProgress("idle"), now))
        assertEquals("—", AdminRules.scanPhaseText(null, now))
        assertEquals("42 shows · 2,589 episodes", AdminRules.libraryCounts(ScanLibrary("a", "a", series = 42, episodes = 2589)))
        assertEquals("3 files", AdminRules.libraryCounts(ScanLibrary("a", "a", files = 3)))
    }

    @Test
    fun rootsKeepTheirStoredCredential() {
        val r = ManagedRoot("smb", "Video/Shows", host = "192.168.1.30", share = "Media", credentialId = "c1", username = "demo", state = "offline", error = "no route")
        assertEquals("192.168.1.30 › Media › Video › Shows", r.display)
        assertFalse(r.online)
        assertEquals("Offline: no route · as demo", AdminRules.rootStatus(r))
        assertEquals(JsonPrimitive("c1"), r.body["credential_id"])
        assertEquals(JsonPrimitive("smb"), r.body["source"])
        val local = ManagedRoot("local", "/media/movies")
        assertTrue(local.online)
        assertEquals(setOf("source", "path"), local.body.keys)
    }

    @Test
    fun aNewRootCarriesItsLogin() {
        val r = LibraryRoot(true, "Video", host = "nas", share = "Media", login = ShareLogin("u", "p"))
        assertEquals("nas › Media › Video", r.display)
        assertEquals(JsonPrimitive("u"), r.body["username"])
        assertEquals(JsonPrimitive("p"), r.body["password"])
        assertEquals(setOf("source", "path"), LibraryRoot(false, "/m").body.keys)
    }

    @Test
    fun hardwareChoicesAndFooter() {
        val t = SystemTranscode(choices = listOf(TranscodeChoice("auto", "Automatic"), TranscodeChoice("videotoolbox", "VideoToolbox"), TranscodeChoice("cpu", "CPU")))
        assertEquals("Automatic tries VideoToolbox, else the CPU.", AdminRules.automaticText(t))
        assertEquals("Automatic finds what transcodes here.", AdminRules.automaticText(SystemTranscode(choices = listOf(TranscodeChoice("auto", "Automatic"), TranscodeChoice("cpu", "CPU")))))
        assertEquals(AdminRules.hardwareChoices, AdminRules.choices(SystemTranscode()))
        assertTrue(AdminRules.isHardware("nvenc"))
        assertFalse(AdminRules.isHardware("cpu"))
        assertEquals("Intel Core i5-7500 CPU", AdminRules.cpuName("Intel(R) Core(TM) i5-7500 CPU"))
        assertEquals("CPU", AdminRules.cpuName(null))
    }

    @Test
    fun progressLines() {
        assertEquals("120 of 281", AdminRules.preparingText(IndexBackfill("running", 281, 120, 0)))
        assertEquals("Paused while playing · 120 of 281", AdminRules.preparingText(IndexBackfill("paused", 281, 120, 0)))
        assertEquals("All files ready · 2 failed", AdminRules.preparingText(IndexBackfill("idle", 281, 279, 2)))
        assertEquals("Starting", AdminRules.preparingText(IndexBackfill(null)))
        assertEquals("Space full · 3 of 9", AdminRules.thumbnailsText(TrickplayProgress("full", 9, 3)))
        assertEquals("All titles ready · 2 skipped", AdminRules.thumbnailsText(TrickplayProgress("idle", 9, 7, 0, 2)))
        assertEquals("1 GB of 10 GB", AdminRules.thumbnailSpaceText(TrickplayProgress(bytes = 1L shl 30, maxBytes = 10L shl 30)))
        assertNull(AdminRules.thumbnailSpaceText(TrickplayProgress()))
    }

    @Test
    fun perfSettingsComeFromTheDocument() {
        val doc = MoonriseApi.json.parseToJsonElement("""
            {"settings": {"performance": {"transcode_jobs": 2, "scan_quick_secs": 300}, "transcode": {"hardware": "auto"}, "features": {"index_backfill": true}},
             "fields": {"performance.transcode_jobs": {"apply": "restart", "source": "env", "env": "MOONRISE_TRANSCODE_JOBS"},
                        "performance.scan_quick_secs": {"apply": "live", "source": "default", "restart_pending": true},
                        "features.index_backfill": {"source": "env"}}}
        """).let { it as JsonObject }
        val list = AdminRules.perfSettings(doc)
        assertEquals(listOf("performance.transcode_jobs", "performance.scan_quick_secs"), list.map { it.key })
        assertTrue(list[0].locked)
        assertEquals("MOONRISE_TRANSCODE_JOBS", list[0].env)
        assertEquals(30, list[1].step)
        assertTrue(list[1].pending)
        val p = AdminRules.playbackDoc(doc)
        assertEquals("auto", p.hardware)
        assertEquals(true, p.prepareAhead)
        assertTrue(p.prepareLocked)
        assertNull(p.thumbnails)
    }

    @Test
    fun savedNoteSaysWhatWaits() {
        assertEquals("Saved: in effect now.", AdminRules.savedNote(emptyList()))
        assertEquals("Saved. Transcodes at Once takes effect after a restart.", AdminRules.savedNote(listOf("performance.transcode_jobs")))
        assertEquals("Saved. Stream Cache, Background Jobs take effect after a restart.", AdminRules.savedNote(listOf("performance.cache_max_gb", "performance.jobs")))
    }

    @Test
    fun activityLines() {
        val s = ActivitySession("s", user = "demo", title = "Episode 1", seriesTitle = "Demo", episodeLabel = "S1E1", method = "transcode", variant = "720p", tonemap = true, duration = 3600.0, position = 60.0)
        assertEquals("Demo · S1E1", AdminRules.sessionTitle(s))
        assertEquals("demo · Transcoding · 720p · HDR→SDR · 1:00 of 1:00:00", AdminRules.sessionLine(s))
        assertEquals("Direct", AdminRules.sessionLine(ActivitySession("s")))
        assertEquals("Off", AdminRules.scanText(null, now))
        assertEquals("Idle", AdminRules.scanText(ActivityScan(), now))
        assertEquals("Last scanned 1 hr. ago", AdminRules.scanText(ActivityScan(phase = "idle", lastPass = ActivityLastPass(at = "2026-11-09T11:00:00Z")), now))
    }

    @Test
    fun logLinesSplit() {
        val (t, level, msg) = AdminRules.logLine("2026-11-09T00:19:10.146851Z  WARN req{auth=\"moonrise\"}: moonrise_server::api: 502 Bad Gateway", ZoneOffset.UTC)
        assertEquals("12:19:10 AM", t)
        assertEquals("WARN", level)
        assertEquals("req{auth=\"moonrise\"}: moonrise_server::api: 502 Bad Gateway", msg)
        assertEquals(Triple("", "", "short"), AdminRules.logLine("short"))
    }

    @Test
    fun formProblems() {
        assertEquals("", AdminRules.newAccountProblem(" ", "password1", "password1"))
        assertEquals("The password needs at least 8 characters.", AdminRules.newAccountProblem("a", "short", "short"))
        assertEquals("The passwords don't match.", AdminRules.newAccountProblem("a", "password1", "password2"))
        assertNull(AdminRules.newAccountProblem("a", "password1", "password1"))
        assertEquals("At least 8 characters.", AdminRules.resetPasswordProblem("x", "x"))
        assertEquals("", AdminRules.setupAccountProblem("a", "password1", ""))
        assertEquals("The passwords don't match.", AdminRules.setupAccountProblem("a", "password1", "other"))
        assertNull(AdminRules.setupAccountProblem("a", "password1", "password1"))
        assertEquals("123456", AdminRules.setupCodeDigits("123 4567"))
    }

    @Test
    fun deviceLines() {
        assertEquals("This device", AdminRules.deviceDetail(AccountDevice("d", current = true), now))
        assertEquals("Used 3 hr. ago", AdminRules.deviceDetail(AccountDevice("d", lastUsed = "2026-11-09T09:00:00Z"), now))
        assertEquals("Signed in 2 days ago", AdminRules.deviceDetail(AccountDevice("d", createdAt = "2026-11-07T11:00:00Z"), now))
        assertEquals("Never used", AdminRules.deviceDetail(AccountDevice("d"), now))
        assertEquals("Admin · 1 device · active just now", AdminRules.accountDetail(Account("a", "a", isAdmin = true, devices = 1, lastUsed = "2026-11-09T11:59:50Z"), now))
    }

    @Test
    fun errorsReadAsTheServerSaysThem() {
        assertEquals("That library doesn't exist.", adminErrorText(MoonriseHttpException(404, "that library doesn't exist.")))
        assertEquals("The Moonrise server returned 500.", adminErrorText(MoonriseHttpException(500, "")))
        assertTrue(adminErrorText(java.io.IOException("timeout")).startsWith("Couldn't reach the server"))
    }

    @Test
    fun shareProblemsAreRead() {
        val e = MoonriseHttpException(502, """{"detail":"x","error":"the server can't be reached","problem":"unreachable"}""")
        assertEquals(ShareProblem("unreachable", "the server can't be reached"), e.shareProblem())
        assertNull(MoonriseHttpException(502, "plain text").shareProblem())
        assertNull(java.io.IOException("x").shareProblem())
    }

    @Test
    fun aFolderSaysWhatItHolds() {
        assertEquals("Nothing in here yet. Moonrise looks through every folder inside the one you choose.", AdminRules.videosText(0, 0))
        assertEquals("1 video and 2 folders in here. Moonrise looks through every folder inside the one you choose.", AdminRules.videosText(1, 2))
    }

    @Test
    fun theFirstScanIsCounted() {
        val l = ScanLibrary("m", "Movies", files = 2)
        assertEquals(listOf(5 to "Files", 0 to "Movies", 0 to "Shows"), AdminRules.setupStats(ScanProgress("scanning", walked = 5), l))
        assertEquals(listOf(1 to "Movie", 0 to "Shows", 0 to "Episodes"), AdminRules.setupStats(ScanProgress("idle"), l.copy(movies = 1)))
        assertEquals("Your library is ready.", AdminRules.setupPhaseText(null, done = true))
        assertEquals("Starting the scan…", AdminRules.setupPhaseText(null, done = false))
        assertEquals(listOf("You can continue: the scan goes on in the background."), AdminRules.setupNotes(ScanProgress("scanning"), l, done = false))
        assertEquals(listOf("No videos were found in that folder."), AdminRules.setupNotes(ScanProgress("idle"), ScanLibrary("m", "Movies"), done = true))
        assertTrue(AdminRules.setupNotes(ScanProgress("idle"), l.copy(review = 1), done = true).single().startsWith("1 title couldn't be recognized for sure: it's"))
    }

    @Test
    fun anLlmKeyStaysWithItsOrigin() {
        assertEquals("https://api.openai.com:443", AdminRules.origin("https://api.openai.com/v1"))
        assertEquals("http://server:8000", AdminRules.origin("http://Server:8000/v1"))
        assertNull(AdminRules.origin("not a url"))
    }

    private fun info(file: String = "/m/Kill.Bill.Vol.1.2003.1080p.mkv", title: String = "Kill Bill") =
        MatchInfo("i", "movie", title, files = listOf(file))

    private fun episode() = MatchInfo("i", "episode", "Pilot", seriesTitle = "Show", seriesTmdbId = 7, season = 1, files = listOf("/m/Kill.Bill.Vol.1.2003.1080p.mkv"))

    /**
     * The search right after a switch used the kind from before it, so an episode taken
     * for a movie listed TV shows, and picking one sent a show's id as `type=movie`.
     */
    @Test
    fun aSwitchSearchesTheKindSwitchedTo() {
        val toMovie = FixMatchRules.switchType("movie", episode(), year = "")
        assertEquals("movie", toMovie.asType)
        assertEquals(MatchSearch("movie", "Kill Bill Vol 1", 2003), toMovie.search)
        assertEquals("2003", toMovie.year)
        assertEquals(MatchShowStep.Keep, toMovie.showStep)

        val toEpisode = FixMatchRules.switchType("episode", info(), year = "1999")
        assertEquals("episode", toEpisode.asType)
        assertEquals(MatchSearch("tv", "Kill Bill Vol 1", 1999), toEpisode.search)
        assertNull("the year stays as it is", toEpisode.year)
        assertEquals(MatchShowStep.FindShow, toEpisode.showStep)

        val seriesToMovie = FixMatchRules.switchType("movie", MatchInfo("i", "series", "Show", files = listOf("/m/Show.2010.mkv")), year = "")
        assertEquals(MatchSearch("movie", "Show", 2010), seriesToMovie.search)
    }

    @Test
    fun aSwitchBackToItsOwnKindDoesNotSearch() {
        val own = FixMatchRules.switchType("episode", episode(), year = "2003")
        assertNull(own.asType)
        assertNull(own.search)
        assertEquals("Show", own.query)
        assertEquals(MatchShowStep.OwnShow, own.showStep)

        val movie = FixMatchRules.switchType("movie", MatchInfo("i", "movie", "Kill Bill", year = 2003), year = "1999")
        assertNull(movie.asType)
        assertNull(movie.search)
        assertEquals("Kill Bill", movie.query)
        assertEquals("2003", movie.year)
    }

    @Test
    fun onlyAMovieSearchesMovies() {
        assertEquals(MatchSearch("movie", "Kill Bill", 2003), FixMatchRules.search("movie", " Kill Bill ", "2003"))
        assertEquals(MatchSearch("tv", "Show", null), FixMatchRules.search("episode", "Show", ""))
        assertEquals(MatchSearch("tv", "Show", null), FixMatchRules.search("series", "Show", "any"))
        assertEquals("movie", FixMatchRules.kind("movie", "episode"))
        assertEquals("episode", FixMatchRules.kind(null, "episode"))
        assertEquals("movie", FixMatchRules.kind(null, null))
        assertTrue(FixMatchRules.isSwitched("movie", "episode"))
        assertFalse(FixMatchRules.isSwitched("episode", "episode"))
        assertFalse(FixMatchRules.isSwitched(null, "episode"))
    }

    /** An answer to a search from before a switch would list titles of the other kind. */
    @Test
    fun anAnswerFromBeforeASwitchIsDropped() {
        val searches = LatestSearch()
        val before = searches.begin()
        assertTrue(searches.isNewest(before))
        searches.forget()
        assertFalse("a switch outdates the search before it", searches.isNewest(before))
        val after = searches.begin()
        assertFalse(searches.isNewest(before))
        assertTrue(searches.isNewest(after))
        val newer = searches.begin()
        assertFalse("a newer search outdates it too", searches.isNewest(after))
        assertTrue(searches.isNewest(newer))
    }

    /** The page runs what the rules say, with no kind of its own and no answer from before a switch. */
    @Test
    fun theFixMatchPageRunsTheRulesSearch() {
        val page = java.io.File("../phone/src/main/kotlin/app/moonrise/phone/ui/FixMatch.kt").readText()
        assertEquals(1, Regex("api\\.searchMatches\\(").findAll(page).count())
        assertTrue("the search must use the request's kind", "api.searchMatches(s.kind, s.query, s.year)" in page)
        val run = page.substringAfter("fun runSearch(").substringBefore("fun search()")
        assertEquals("results only from the newest search", 2, Regex("searches\\.isNewest\\(mine\\)").findAll(run).count())
        val switch = page.substringAfter("fun switchType(").substringBefore("fun apply(")
        assertTrue("FixMatchRules.switchType(to, i, year)" in switch)
        assertTrue("a switch must outdate the search before it", "searches.forget()" in switch)
        assertFalse("a switch runs the rules' search, not the page's", Regex("(?<![A-Za-z.])search\\(\\)").containsMatchIn(switch))
    }

    @Test
    fun fileNamesGiveTitleAndYear() {
        assertEquals("Kill Bill Vol 1", FixMatchRules.fileTitle(info()))
        assertEquals(2003, FixMatchRules.fileYear(info()))
        assertEquals("Show Name", FixMatchRules.fileTitle(info("/m/Show.Name.S01E03.720p.mkv")))
        assertEquals("Stop", FixMatchRules.fileTitle(info("/m/Stop [Group].mkv")))
        assertEquals("Kill Bill", FixMatchRules.fileTitle(MatchInfo("i", "movie", "Kill Bill")))
    }

    @Test
    fun episodesMatchByTitle() {
        val i = info(title = "Ozymandias")
        assertTrue(FixMatchRules.likely(TMDBEpisode(14, 5, "Ozymandias!"), i))
        assertFalse(FixMatchRules.likely(TMDBEpisode(14, 5, "Pilot"), i))
        assertFalse(FixMatchRules.likely(TMDBEpisode(14, 5, null), i))
    }

    @Test
    fun reviewListReads() {
        val movie = ReviewItem("a", "movie", "Alien", year = 1979, note = "no confident TMDB match", candidates = listOf(MatchCandidate(1, "A"), MatchCandidate(2, "B")))
        assertEquals("Alien (1979)", movie.heading)
        assertEquals("2 possible matches", FixMatchRules.reason(movie))
        val ep = ReviewItem("b", "episode", "Pilot", seriesTitle = "Show", season = 1, episode = 2, note = "S1E2 isn't in TMDB's season list", files = listOf("/x/Show S01E02.mkv"))
        assertEquals("S1E2 · Pilot", FixMatchRules.rowTitle(ep))
        assertEquals("Show S01E02.mkv", FixMatchRules.reason(ep))
        assertEquals(listOf("Movies and Shows", "Show"), FixMatchRules.groups(listOf(ep, movie)).map { it.first })
        assertEquals("TMDB doesn't list this episode under this number: pick the one it is, or keep the number.", FixMatchRules.groupFooter(listOf(ep)))
        assertNull(FixMatchRules.groupFooter(listOf(movie)))
    }

    @Test
    fun matchStateAndVersions() {
        val review = MatchInfo("i", "movie", "A", state = "review", note = "two close matches")
        assertEquals("Not recognized for sure: two close matches.", FixMatchRules.stateText(review))
        assertEquals("Named from its files (not matched on TMDB).", FixMatchRules.stateText(MatchInfo("i", "movie", "A", local = true)))
        assertEquals("Matched by AI.", FixMatchRules.stateText(MatchInfo("i", "movie", "A", state = "llm")))
        assertTrue(FixMatchRules.isOtherTitle(MatchCandidate(1, "A", inLibrary = MatchInLibrary("other", 2)), "mine"))
        assertFalse(FixMatchRules.isOtherTitle(MatchCandidate(1, "A", inLibrary = MatchInLibrary("mine", 2)), "mine"))
    }

    @Test
    fun anEpisodeCountsOnlyTheOthersFiles() {
        val e = TMDBEpisode(3, 1, "x", inLibrary = MatchInLibrary(files = 2, titles = 1))
        val mine = MatchInfo("i", "episode", "x", seriesTmdbId = 7, season = 1, episode = 3)
        assertEquals(1 to 0, FixMatchRules.otherFilesOfEpisode(e, mine, show = 7, fileId = null))
        assertEquals(2 to 1, FixMatchRules.otherFilesOfEpisode(e, mine, show = 8, fileId = null))
    }

    @Test
    fun identifyAsksWhatWasPicked() {
        val body = IdentifyRequest(tmdbId = 5, season = 1, episode = 2, type = "episode", part = true, keepPart = 2, fileId = "f").body
        assertEquals(JsonPrimitive(2), body["part"])
        assertEquals(JsonPrimitive("f"), body["file_id"])
        assertEquals(JsonPrimitive(true), IdentifyRequest(tmdbId = 5, part = true).body["part"])
        assertEquals(JsonPrimitive(true), IdentifyRequest(local = true).body["local"])
        assertFalse("part" in IdentifyRequest(tmdbId = 5).body)
    }

    private val notPosterPaths = listOf(
        null, "", "abc.jpg", "/abc", "/abc.svg", "/a/b.jpg", "/a.b.jpg", "/abc.jpg?x=1", "//evil.example/x.jpg",
        "https://evil.example/x.jpg", "https://image.tmdb.org/t/p/w342/abc.jpg", "/../x.jpg", "/abc.jpg\n",
    )

    /** Apps talk only to the server. Every poster address is the server's, and anything else is no poster. */
    @Test
    fun aMatchPosterIsOnlyEverLoadedFromTheServer() {
        val api = MoonriseApi("http://192.168.1.10:8097/".toHttpUrl(), token = "fake-token", deviceId = "d", deviceName = "Test")
        val u = api.matchPosterUrl("/abc.jpg")!!.toHttpUrl()
        assertEquals("192.168.1.10", u.host)
        assertEquals(8097, u.port)
        assertEquals("/v1/admin/match/poster", u.encodedPath)
        assertEquals("/abc.jpg", u.queryParameter("path"))
        assertEquals("w185", u.queryParameter("size"))
        for (p in notPosterPaths) assertNull("$p", api.matchPosterUrl(p))
    }

    /** An older server's answer has only TMDB's address: the row has no poster, and nothing is loaded from TMDB. */
    @Test
    fun aCandidateWithOnlyATmdbAddressHasNoPoster() {
        val c = Json { ignoreUnknownKeys = true }.decodeFromString(
            MatchCandidate.serializer(),
            """{"tmdb_id":1,"title":"A","poster":"https://image.tmdb.org/t/p/w342/abc.jpg"}""",
        )
        assertNull(c.posterPath)
        assertNull(MoonriseApi("http://192.168.1.10:8097/".toHttpUrl(), token = "t", deviceId = "d", deviceName = "Test").matchPosterUrl(c.posterPath))
    }
}
