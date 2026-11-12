package app.moonrise.core

import kotlinx.coroutines.runBlocking
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The segment rules of `SegmentService.swift` and `PlayerModel.swift`, with the cases measured on
 * real titles (The Sopranos and others: the file's own credits, the databases' credits, other cuts).
 */
class SegmentsTest {
    private fun credits(start: Double, end: Double, source: String? = null) = Segment(SegmentKind.Credits, start, end, source)
    private fun intro(start: Double, end: Double) = Segment(SegmentKind.Intro, start, end)

    @Test
    fun sopranos6x06DetectedCreditsWinOverAnEarlyDatabaseValue() {
        // TheIntroDB 3048 runs to the end, 2 min early (inside the last scene); found in the file at 3180.1.
        val r = SegmentRules.merge(
            server = listOf(credits(3180.1, 3324.672, "detected")),
            online = listOf(listOf(credits(3048.0, 3324.672)), emptyList()),
            runtime = 3324.672,
        )
        assertEquals(listOf(credits(3180.1, 3324.672, "detected")), r)
    }

    @Test
    fun sopranos4x01LoneOpenEndedCreditsFarFromTheEndAreLeftOut() {
        // Nothing found in the file; TheIntroDB 3227 runs to the end from 240 s before it, IntroDB has none.
        val r = SegmentRules.merge(server = emptyList(), online = listOf(listOf(credits(3227.0, 3467.552)), emptyList()), runtime = 3467.552)
        assertTrue(r.none { it.kind == SegmentKind.Credits })
    }

    @Test
    fun sopranos6x02DatabaseStartCloseToTheDetectedOneIsUsed() {
        // Found at 3135.6; TheIntroDB's 3133 is within 10 s before it and times the credits more closely.
        val r = SegmentRules.merge(
            server = listOf(credits(3135.6, 3290.0, "detected")),
            online = listOf(listOf(credits(3133.0, 3290.0))),
            runtime = 3290.0,
        )
        assertEquals(3133.0, r.single().start, 0.0)
        assertNull(r.single().source)
    }

    @Test
    fun longCreditsAreKeptWhenBothDatabasesAgree() {
        // Loki 2x03: TheIntroDB 2924 and IntroDB 2915 agree within 15 s: the later one is used.
        val c = SegmentRules.credits(listOf(credits(2924.0, 3212.0), credits(2915.0, 3212.0)), detected = null, runtime = 3212.0)
        assertEquals(2924.0, c!!.start, 0.0)
        // Agatha 1x01 (art cards from 2107, the crawl from 2235, nothing found in the file): both run to
        // the end from far before it and disagree by 128 s: the later one.
        assertEquals(2235.0, SegmentRules.credits(listOf(credits(2107.0, 2389.792), credits(2235.0, 2392.0)), detected = null, runtime = 2389.792)!!.start, 0.0)
        // Credits that stop before the end aren't open-ended: trusted as they are.
        assertEquals(1000.0, SegmentRules.credits(listOf(credits(1000.0, 1100.0)), null, 2600.0)!!.start, 0.0)
        // Close to the end (under 150 s): trusted alone.
        assertEquals(2500.0, SegmentRules.credits(listOf(credits(2500.0, 2600.0)), null, 2600.0)!!.start, 0.0)
    }

    @Test
    fun lonePairNeverOffersCreditsBeforeTheLaterDatabaseStart() {
        // One database's credits stop before the end (2107-2300), the other runs to the end from
        // far before it (2235): never earlier than a database start, so 2235, in either order.
        val pair = listOf(credits(2107.0, 2300.0), credits(2235.0, 2400.0))
        assertEquals(credits(2235.0, 2400.0), SegmentRules.credits(pair, detected = null, runtime = 2400.0))
        assertEquals(credits(2235.0, 2400.0), SegmentRules.credits(pair.reversed(), detected = null, runtime = 2400.0))
        val r = SegmentRules.merge(server = emptyList(), online = listOf(listOf(credits(2107.0, 2300.0)), listOf(credits(2235.0, 2400.0))), runtime = 2400.0)
        assertEquals(listOf(credits(2235.0, 2400.0)), r)
    }

    @Test
    fun detectedCreditsTakeTheLatestDatabaseStartNotTooFarBefore() {
        val detected = credits(3000.0, 3300.0, "detected")
        // Never earlier than a database start: the latest, 3010 (2985 and 2995 are earlier).
        val c = SegmentRules.credits(listOf(credits(2985.0, 3300.0), credits(3010.0, 3300.0), credits(2995.0, 3300.0)), detected)
        assertEquals(3010.0, c!!.start, 0.0)
        // The latest more than 10 s before the found start: the found start (later).
        assertEquals(detected, SegmentRules.credits(listOf(credits(2900.0, 3300.0), credits(2985.0, 3300.0)), detected))
        assertEquals(detected, SegmentRules.credits(emptyList(), detected))
    }

    private fun start(runtime: Double, vararg db: Double, detected: Double? = null): Double? {
        val server = detected?.let { listOf(credits(it, runtime, "detected")) } ?: emptyList()
        return SegmentRules.merge(server, db.map { listOf(credits(it, runtime)) }, runtime).firstOrNull { it.kind == SegmentKind.Credits }?.start
    }

    @Test
    fun measuredEpisodesNeverStartBeforeADatabaseValue() {
        // The same cases are in clients/apple/MoonriseTests/SegmentRulesTests.swift.
        // The Sopranos 4x13: TheIntroDB 4215 is in the story, found 4393.3; even a database start
        // a few seconds into the last shot (4380) gives way to the found start.
        assertEquals(4393.306, start(4510.848, 4215.0, detected = 4393.306)!!, 0.0)
        assertEquals(4393.306, start(4510.848, 4380.0, detected = 4393.306)!!, 0.0)
        // The Sopranos 1x02: both databases within 10 s before the found 2920.5: the later, 2921.
        assertEquals(2921.0, start(2983.648, 2911.0, 2921.0, detected = 2920.543)!!, 0.0)
        // The Sopranos 2x05 and 2x13: TheIntroDB in the last scene, IntroDB at the credits.
        assertEquals(3050.0, start(3115.68, 2926.0, 3050.0, detected = 3049.046)!!, 0.0)
        assertEquals(3451.0, start(3554.688, 3311.0, 3451.0, detected = 3454.451)!!, 0.0)
        // Sherlock 1x01: the later database value, after the found start.
        assertEquals(5263.0, start(5282.04, 5263.0, 5245.29, detected = 5250.92)!!, 0.0)
        // The Rings of Power 2x03, 2x06: credits on black from TheIntroDB's 3704.3 and 3497.3, but the
        // dense crawl is found only at 3818 and 3614: late, inside the credits.
        assertEquals(3818.0, start(4003.136, 3704.311, detected = 3818.0)!!, 0.0)
        assertEquals(3614.0, start(3786.72, 3497.274, detected = 3614.0)!!, 0.0)
        // The Punisher 1x05, 2x01: the databases are a few seconds into the last shot; found later.
        assertEquals(3255.252, start(3309.728, 3176.718, 3176.72, detected = 3255.252)!!, 0.0)
        assertEquals(3135.132, start(3189.92, 3053.233, 3061.68, detected = 3135.132)!!, 0.0)
    }

    /** The credits a film gets: the file's chapter credits (`chapter`: start to end) and the databases'. */
    private fun film(runtime: Double, vararg db: Pair<Double, Double>, chapter: Pair<Double, Double>? = null): Segment? =
        SegmentRules.merge(
            chapter?.let { listOf(credits(it.first, it.second)) } ?: emptyList(),
            db.map { listOf(credits(it.first, it.second)) },
            runtime,
            movie = true,
        ).firstOrNull { it.kind == SegmentKind.Credits }

    @Test
    fun reportedCreditsStartIsTheCreditsOffered() {
        // As in clients/apple/MoonriseTests/SegmentRulesTests.swift: Thor: Ragnarok, IntroDB alone (7409 of
        // 7831; 90 % is 7048): the server marks it watched from 7409.
        val ragnarok = SegmentRules.merge(server = listOf(intro(10.0, 40.0)), online = listOf(listOf(credits(7409.277, 7769.637))), runtime = 7830.793, movie = true)
        assertEquals(7409.277, SegmentRules.creditsStart(ragnarok)!!, 1e-9)
        assertNull(SegmentRules.creditsStart(listOf(intro(10.0, 40.0))))
        assertNull(SegmentRules.creditsStart(emptyList()))
        assertEquals(7409.0, SegmentRules.creditsStart(listOf(credits(300.0, 450.0), credits(7409.0, 7769.0)))!!, 1e-9)
    }

    @Test
    fun chapterCreditsNeverStartBeforeADatabaseStart() {
        // As SegmentRulesTests.chaptersNeverBeforeADatabaseStart. Guardians of the Galaxy:
        // "en:End Credits" 6796.6-7210.8, IntroDB 6852-7210: from 6852, reported as 6852.
        val gotg = SegmentRules.merge(server = listOf(credits(6796.622, 7210.828)), online = listOf(listOf(credits(6852.0, 7210.0))), runtime = 7268.64, movie = true)
        assertEquals(listOf(credits(6852.0, 7210.828)), gotg)
        assertEquals(6852.0, SegmentRules.creditsStart(gotg)!!, 1e-9)
        // A chapter after the database start is kept.
        assertEquals(listOf(credits(7700.0, 8189.0)), SegmentRules.merge(listOf(credits(7700.0, 8189.0)), listOf(listOf(credits(7624.0, 8189.0))), 8300.0, movie = true))
        // An episode's chapter 14 s before a database start: the latest database start.
        assertEquals(
            listOf(credits(3414.0, 3600.0)),
            SegmentRules.merge(listOf(credits(3400.0, 3600.0)), listOf(listOf(credits(3414.0, 3600.0)), listOf(credits(3405.0, 3600.0))), 3600.0),
        )
        // A database start past the chapter's end: the database's credits. No database: the chapter.
        assertEquals(credits(3200.0, 3600.0), SegmentRules.notBefore(credits(3000.0, 3100.0), credits(3200.0, 3600.0)))
        assertEquals(listOf(credits(6796.622, 7210.828)), SegmentRules.merge(listOf(credits(6796.622, 7210.828)), emptyList(), 7268.64, movie = true))
    }

    @Test
    fun filmCreditsAreSkippableOnlyWithAnEndBeforeTheEndOfTheFile() {
        // The same cases are in clients/apple/MoonriseTests/SegmentRulesTests.swift.
        // Avengers: Infinity War: the "End Credits" chapter runs to the end (a scene after them);
        // IntroDB 8251-8852 ends before it: Skip Credits jumps to 8852.
        assertEquals(credits(8251.0, 8852.0), film(8978.528, 8251.0 to 8852.0, chapter = 8251.285 to 8978.528))
        // Thor: The Dark World: the same, IntroDB 6109-6656.
        assertEquals(credits(6109.0, 6656.0), film(6723.411, 6109.0 to 6656.0, chapter = 6109.52 to 6723.411))
        // Guardians of the Galaxy: IntroDB 6852-7210, where the file's "End Tag" chapter starts.
        assertEquals(credits(6852.0, 7210.0), film(7268.64, 6852.0 to 7210.0))
        // A credits chapter that ends before the end of the file is kept as it is.
        assertEquals(credits(6852.0, 7210.828), film(7268.64, chapter = 6852.0 to 7210.828))
        // Deadpool & Wolverine: TheIntroDB 7181-7601, IntroDB 7363-7609: the later start, ending at 7609.
        assertEquals(credits(7363.0, 7609.0), film(7753.209, 7181.041 to 7601.041, 7363.0 to 7609.0))
        // Thor: Ragnarok: IntroDB alone, 7409.3-7769.6, the end scene after it.
        assertEquals(credits(7409.277, 7769.637), film(7830.793, 7409.277 to 7769.637))
        // No end known (a database without one, or a chapter to the end): nothing offered.
        assertNull(film(10871.55, 10200.0 to 10871.55))
        assertNull(film(8978.528, chapter = 8251.285 to 8978.528))
        assertNull(film(8978.528, 8251.0 to 8976.0)) // within 5 s of the end
        assertNull(film(0.0, 8251.0 to 8852.0)) // no runtime: the end isn't known
        // The pick is as for episodes, then dropped when it runs to the end (never an earlier start instead).
        assertNull(film(7753.209, 7363.0 to 7609.0, 7650.0 to 7753.209))
        // Episodes are unchanged: credits to the end are offered.
        assertEquals(3451.0, start(3554.688, 3451.0)!!, 0.0)
    }

    @Test
    fun implausibleDatabaseCreditsAreRejected() {
        val runtime = 1300.0
        assertFalse(SegmentRules.plausibleCredits(credits(0.0, 1300.0), runtime)) // from the start (SoA 5x09)
        assertFalse(SegmentRules.plausibleCredits(credits(1310.0, 1320.0), runtime)) // after the end
        assertFalse(SegmentRules.plausibleCredits(credits(1250.0, 1320.0), runtime)) // ends well after: a longer cut
        assertTrue(SegmentRules.plausibleCredits(credits(1250.0, 1304.0), runtime))
        assertTrue(SegmentRules.plausibleCredits(credits(10.0, 20.0), 0.0)) // no runtime: nothing known
    }

    @Test
    fun pendingHoldsBackDatabaseCreditsButNotIntros() {
        val online = listOf(listOf(intro(60.0, 90.0), credits(1250.0, 1300.0)))
        val held = SegmentRules.merge(emptyList(), online, runtime = 1300.0, pending = true)
        assertEquals(listOf(intro(60.0, 90.0)), held)
        val all = SegmentRules.merge(emptyList(), online, runtime = 1300.0, pending = false)
        assertEquals(listOf(SegmentKind.Intro, SegmentKind.Credits), all.map { it.kind })
    }

    @Test
    fun serverChaptersComeFirstAndTheFirstDatabaseFillsGaps() {
        val server = listOf(intro(50.0, 80.0))
        val online = listOf(
            listOf(intro(0.0, 30.0), Segment(SegmentKind.Recap, 0.0, 45.0)),
            listOf(Segment(SegmentKind.Recap, 0.0, 40.0), Segment(SegmentKind.Preview, 1200.0, 1240.0)),
        )
        val r = SegmentRules.merge(server, online, runtime = 1300.0)
        assertEquals(listOf(Segment(SegmentKind.Recap, 0.0, 45.0), intro(50.0, 80.0), Segment(SegmentKind.Preview, 1200.0, 1240.0)), r)
    }

    @Test
    fun segmentsAreClampedAndShortOnesDropped() {
        val r = SegmentRules.merge(listOf(intro(-2.0, 2.5), Segment(SegmentKind.Credits, 1290.0, 1400.0)), emptyList(), runtime = 1300.0)
        // The intro is 2.5 s once clamped (< 3): dropped. Credits end at the runtime.
        assertEquals(listOf(Segment(SegmentKind.Credits, 1290.0, 1300.0)), r)
    }

    private fun ep(id: String, season: Int, n: Int) = Item(id = id, kind = "episode", seasonNo = season, episodeNo = n)

    @Test
    fun nextEpisodeCrossesSeasonsButNotIntoSpecials() {
        val eps = listOf(ep("a", 1, 1), ep("b", 1, 2), ep("c", 2, 1), ep("s", 0, 1), ep("t", 0, 2))
        assertEquals("c", NextEpisodeRules.next(eps, eps[1])?.id)
        assertNull(NextEpisodeRules.next(eps, eps[2])) // the specials only follow a special
        assertEquals("t", NextEpisodeRules.next(eps, eps[3])?.id)
        assertNull(NextEpisodeRules.next(eps, eps[4]))
        assertNull(NextEpisodeRules.next(eps, ep("x", 3, 1)))
    }

    @Test
    fun nextEpisodeOfferedFromCreditsToTheEndElseTheLast30Seconds() {
        val d = 3324.672
        val segs = listOf(credits(3180.1, d, "detected"))
        // In the story: no.
        assertFalse(NextEpisodeRules.offered(d, 3100.0, 3100.0, null, segs, hasNext = true))
        // From where the credits start.
        assertTrue(NextEpisodeRules.offered(d, 3180.5, 3180.5, null, segs, hasNext = true))
        // The last episode, or not started: never.
        assertFalse(NextEpisodeRules.offered(d, 3200.0, 3200.0, null, segs, hasNext = false))
        assertFalse(NextEpisodeRules.offered(d, 0.0, 3200.0, null, segs, hasNext = true))
        // No credits known (Sopranos 4x01): the last 30 s only.
        val d2 = 3467.552
        assertFalse(NextEpisodeRules.offered(d2, 3430.0, 3430.0, null, emptyList(), hasNext = true))
        assertTrue(NextEpisodeRules.offered(d2, 3438.0, 3438.0, null, emptyList(), hasNext = true))
        // Credits that stop well before the end (a mid-credits scene follows): not from there.
        assertFalse(NextEpisodeRules.offered(2000.0, 1800.0, 1800.0, null, listOf(credits(1700.0, 1900.0)), hasNext = true))
        // Inside a credits segment the player is in: yes.
        assertTrue(NextEpisodeRules.offered(2000.0, 1800.0, 1800.0, credits(1700.0, 1900.0), emptyList(), hasNext = true))
        // A pending skip's target counts (the controls show it).
        assertTrue(NextEpisodeRules.offered(d, 3100.0, 3190.0, null, segs, hasNext = true))
    }

    @Test
    fun skippingCreditsToTheEndPlaysTheNextEpisode() {
        assertTrue(NextEpisodeRules.creditsSkipPlaysNext(credits(3180.0, 3322.0), 3324.672, hasNext = true))
        assertFalse(NextEpisodeRules.creditsSkipPlaysNext(credits(3180.0, 3300.0), 3324.672, hasNext = true))
        assertFalse(NextEpisodeRules.creditsSkipPlaysNext(credits(3180.0, 3324.0), 3324.672, hasNext = false))
        assertFalse(NextEpisodeRules.creditsSkipPlaysNext(intro(0.0, 60.0), 3324.672, hasNext = true))
    }

    @Test
    fun pendingSegmentsAreAskedAgainNearTheEndEachMinute() {
        val d = 3000.0
        assertFalse(NextEpisodeRules.refetchDue(true, d, 2600.0, Double.NEGATIVE_INFINITY)) // more than 5 min left
        assertTrue(NextEpisodeRules.refetchDue(true, d, 2750.0, Double.NEGATIVE_INFINITY))
        assertFalse(NextEpisodeRules.refetchDue(true, d, 2780.0, 2750.0)) // asked less than a minute ago
        assertTrue(NextEpisodeRules.refetchDue(true, d, 2810.0, 2750.0))
        assertFalse(NextEpisodeRules.refetchDue(true, d, 2950.0, 2810.0)) // the last minute
        assertFalse(NextEpisodeRules.refetchDue(false, d, 2810.0, 2750.0))
    }

    @Test
    fun activeSegmentFollowsTheSkipModes() {
        val segs = listOf(intro(60.0, 90.0), credits(1200.0, 1300.0))
        assertEquals(intro(60.0, 90.0), NextEpisodeRules.active(segs, 70.0) { it.defaultMode })
        assertNull(NextEpisodeRules.active(segs, 89.5) { it.defaultMode }) // the last second isn't offered
        assertNull(NextEpisodeRules.active(segs, 70.0) { if (it == SegmentKind.Intro) SkipMode.Off else it.defaultMode })
        assertEquals(SkipMode.Button, SegmentKind.Credits.defaultMode)
        assertEquals(SkipMode.Auto, SegmentKind.Intro.defaultMode)
    }

    @Test
    fun serviceAsksForTheMergedAnswerAndCachesOnlyFinalOnes() {
        val server = MockWebServer()
        // Still looking in the file and in the databases; then the merged answer.
        server.enqueue(MockResponse().setBody("""{"items":[{"kind":"intro","start":0,"end":98.5,"source":"chapters"}],"total":1,"pending":true,"merged":true,"community_pending":true}"""))
        server.enqueue(MockResponse().setBody("""{"items":[{"kind":"intro","start":0,"end":98.5,"source":"chapters"},{"kind":"credits","start":3180.1,"end":3324.672,"source":"detected"}],"total":2,"pending":false,"merged":true,"community_pending":false}"""))
        server.start()
        try {
            val api = MoonriseApi(server.url("/"), token = "t", deviceId = "d", deviceName = "n")
            var online = true
            val svc = SegmentService(api, useOnline = { online })
            val item = Item(id = "e1", kind = "episode", seasonNo = 6, episodeNo = 6, runtime = 3324.672)
            runBlocking {
                val first = svc.segments(item, version = "f1")
                assertEquals(listOf(SegmentKind.Intro), first.map { it.kind })
                assertTrue(svc.awaitingServer(item, "f1"))
                assertTrue(svc.awaitingCommunity(item, "f1"))
                online = false
                val r = svc.segments(item, version = "f1", provisional = true)
                // A merged answer is offered as it is.
                assertEquals(listOf(SegmentKind.Intro, SegmentKind.Credits), r.map { it.kind })
                assertEquals(3180.1, r.last().start, 0.0)
                assertFalse(svc.awaitingServer(item, "f1"))
                assertFalse(svc.awaitingCommunity(item, "f1"))
                // Final: answered from the cache.
                assertEquals(r, svc.segments(item, version = "f1"))
            }
            val a = server.takeRequest().requestUrl!!
            assertEquals("/v1/items/e1/segments", a.encodedPath)
            assertEquals(listOf("f1", "1", null, null), listOf("version", "merged", "provisional", "community").map { a.queryParameter(it) })
            val b = server.takeRequest().requestUrl!!
            assertEquals(listOf("1", "1", "0"), listOf("merged", "provisional", "community").map { b.queryParameter(it) })
            assertEquals(2, server.requestCount)
        } finally {
            server.shutdown()
        }
    }

    @Test
    fun anOlderServersAnswerGetsTheRulesWithoutDatabases() {
        // An older server ignores merged=1: the file's own, by the same rules, and no one else is asked.
        val film = ServerSegments(listOf(credits(8251.285, 8978.528)))
        assertTrue(SegmentService.offered(film, 8978.528, provisional = false, movie = true).isEmpty())
        val episode = ServerSegments(listOf(credits(3180.1, 3324.672, "detected")))
        assertEquals(listOf(3180.1), SegmentService.offered(episode, 3324.672, provisional = false, movie = false).map { it.start })
        val merged = ServerSegments(listOf(credits(6852.0, 7210.828)), merged = true)
        assertEquals(merged.items, SegmentService.offered(merged, 7268.64, provisional = false, movie = true))
    }
}
