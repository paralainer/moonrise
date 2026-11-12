package app.moonrise.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The home screen's Watch Next row: which titles, and the changes that keep it in step. */
class WatchNextTest {
    private val now = 1_800_000_000_000L

    private fun movie(id: String, position: Double? = null, played: Boolean = false, at: String? = null, images: ItemImages? = null) = Item(
        id = id, kind = "movie", title = "Film $id", year = 2001, runtime = 6000.0, overview = "About $id",
        images = images ?: ItemImages(backdrop = listOf("bd-$id"), poster = "p-$id"),
        user = UserData(position = position, played = played, lastPlayedAt = at),
    )

    private fun episode(id: String, series: String, position: Double? = null, at: String? = null) = Item(
        id = id, kind = "episode", title = "Ep $id", seriesId = series, seriesTitle = "Show $series", seasonNo = 2, episodeNo = 5,
        runtime = 3000.0, images = ItemImages(thumb = "th-$id"), parentImages = ItemImages(poster = "sp-$series"),
        user = UserData(position = position, lastPlayedAt = at),
    )

    @Test
    fun recentsThenNextUpWithoutShowsMidEpisode() {
        val home = Home(
            resume = listOf(
                movie("m1", position = 600.0, at = "2026-11-07T10:00:00Z"),
                episode("e1", "s1", position = 120.0, at = "2026-11-07T09:00:00+00:00"),
                movie("m2", position = 300.0, played = true),
                movie("m3", position = null),
            ),
            nextUp = listOf(episode("e2", "s1"), episode("e9", "s9")),
        )
        val e = WatchNextRules.entries(home, emptyList(), now)
        // m2 was watched before and is being watched again: it has a resume point, so it's in the row.
        assertEquals(listOf("m1", "e1", "m2", "e9"), e.map { it.itemId })
        assertEquals(listOf(WatchNextKind.Continue, WatchNextKind.Continue, WatchNextKind.Continue, WatchNextKind.Next), e.map { it.kind })

        val m1 = e[0]
        assertEquals(600_000L, m1.positionMs)
        assertEquals(6_000_000L, m1.durationMs)
        assertEquals(WatchNextRules.playedAt("2026-11-07T10:00:00Z"), m1.engagementMs)
        assertEquals("bd-m1", m1.image)
        assertFalse(m1.posterImage)
        assertFalse(m1.isEpisode)

        val e1 = e[1]
        assertEquals("Show s1", e1.title)
        assertEquals("Ep e1", e1.episodeTitle)
        assertEquals(2, e1.seasonNumber)
        assertEquals(5, e1.episodeNumber)
        assertEquals("th-e1", e1.image)

        assertEquals(300_000L, e[2].positionMs)

        // Not played yet: it shows up now, at the start (a second earlier per place in the row).
        assertEquals(0L, e[3].positionMs)
        assertEquals(now - 4 * 1000L, e[3].engagementMs)
    }

    @Test
    fun continueWatchingFromTheServer() {
        fun info(reason: String, at: String) = ContinueInfo(reason = reason, at = at)
        val home = Home(
            continueWatching = listOf(
                episode("e6", "s1").copy(continueInfo = info("next_episode", "2026-11-07T11:00:00Z")),
                movie("m2", position = 300.0, played = true).copy(continueInfo = info("resume", "2026-11-07T10:00:00Z")),
                movie("m7").copy(continueInfo = info("collection_next", "2026-11-07T09:00:00Z")),
            ),
            // Older apps' fields: not used when `continue` is there.
            resume = listOf(movie("m1", position = 600.0)),
            nextUp = listOf(episode("e9", "s9")),
        )
        val e = WatchNextRules.entries(home, emptyList(), now)
        assertEquals(listOf("e6", "m2", "m7"), e.map { it.itemId })
        assertEquals(listOf(WatchNextKind.Next, WatchNextKind.Continue, WatchNextKind.Next), e.map { it.kind })
        // Ordered by the server's time of the activity, even a next episode.
        assertEquals(WatchNextRules.playedAt("2026-11-07T11:00:00Z"), e[0].engagementMs)
        // A film being watched again resumes where it was.
        assertEquals(300_000L, e[1].positionMs)
        assertEquals(listOf("e6", "m2", "m7"), home.continueRow.map { it.id })
    }

    @Test
    fun rewatchOffersTheNextEpisodeAndFilmThoughWatched() {
        fun info(reason: String) = ContinueInfo(reason = reason, at = "2026-11-07T11:00:00Z")
        val watched = UserData(played = true)
        val home = Home(
            continueWatching = listOf(
                episode("e2", "s1").copy(user = watched, continueInfo = info("next_episode")),
                movie("m2", played = true).copy(continueInfo = info("collection_next")),
            ),
        )
        val e = WatchNextRules.entries(home, emptyList(), now)
        assertEquals(listOf("e2", "m2"), e.map { it.itemId })
        assertEquals(listOf(WatchNextKind.Next, WatchNextKind.Next), e.map { it.kind })
    }

    @Test
    fun legacyRowWithoutContinue() {
        val home = Home(resume = listOf(episode("e1", "s1", position = 100.0)), nextUp = listOf(episode("e2", "s1"), episode("e9", "s9")))
        assertEquals(listOf("e1", "e9"), home.continueRow.map { it.id })
    }

    @Test
    fun posterWhenNoLandscapeArtwork() {
        val home = Home(resume = listOf(movie("m1", position = 600.0, images = ItemImages(poster = "p-m1"))))
        val e = WatchNextRules.entries(home, emptyList(), now).single()
        assertEquals("p-m1", e.image)
        assertTrue(e.posterImage)
    }

    @Test
    fun nextEpisodeKeepsTheTimeItFirstShowedUp() {
        val home = Home(nextUp = listOf(episode("e2", "s1")))
        val rows = listOf(WatchNextRow(7, "e2", engagementMs = 1234L, browsable = true, fingerprint = null))
        assertEquals(1234L, WatchNextRules.entries(home, rows, now).single().engagementMs)
    }

    @Test
    fun cappedAtMax() {
        val home = Home(resume = (1..30).map { movie("m$it", position = 100.0) })
        assertEquals(WatchNextRules.MAX, WatchNextRules.entries(home, emptyList(), now).size)
    }

    @Test
    fun planInsertsUpdatesAndDeletes() {
        val home = Home(resume = listOf(movie("m1", position = 600.0, at = "2026-11-07T10:00:00Z"), movie("m2", position = 50.0, at = "2026-11-07T08:00:00Z")))
        val desired = WatchNextRules.entries(home, emptyList(), now)
        val rows = listOf(
            WatchNextRow(1, "m1", desired[0].engagementMs, true, desired[0].fingerprint), // unchanged
            WatchNextRow(2, "m2", 0, true, "old"), // changed
            WatchNextRow(3, "gone", 0, true, "x"), // finished: no longer wanted
            WatchNextRow(4, null, 0, true, null), // not ours to keep
        )
        val ops = WatchNextRules.plan(desired, rows)
        assertEquals(
            setOf(WatchNextOp.Update(2, desired[1]), WatchNextOp.Delete(3), WatchNextOp.Delete(4)),
            ops.toSet(),
        )
        // Nothing left to do once applied.
        val after = listOf(
            WatchNextRow(1, "m1", desired[0].engagementMs, true, desired[0].fingerprint),
            WatchNextRow(2, "m2", desired[1].engagementMs, true, desired[1].fingerprint),
        )
        assertTrue(WatchNextRules.plan(desired, after).isEmpty())
    }

    @Test
    fun newTitleInserted() {
        val desired = WatchNextRules.entries(Home(resume = listOf(movie("m1", position = 600.0))), emptyList(), now)
        assertEquals(listOf(WatchNextOp.Insert(desired[0])), WatchNextRules.plan(desired, emptyList()))
    }

    @Test
    fun removedByTheViewerStaysRemovedUntilWatchedAgain() {
        val at = "2026-11-07T10:00:00Z"
        val desired = WatchNextRules.entries(Home(resume = listOf(movie("m1", position = 600.0, at = at))), emptyList(), now)
        val removed = WatchNextRow(5, "m1", WatchNextRules.playedAt(at)!!, browsable = false, fingerprint = "old")
        assertTrue(WatchNextRules.plan(desired, listOf(removed)).isEmpty())

        // Played again later: back as a new row.
        val later = WatchNextRules.entries(Home(resume = listOf(movie("m1", position = 900.0, at = "2026-11-07T12:00:00Z"))), emptyList(), now)
        assertEquals(listOf(WatchNextOp.Delete(5), WatchNextOp.Insert(later[0])), WatchNextRules.plan(later, listOf(removed)))

        // No longer wanted at all: the removed row goes too.
        assertEquals(listOf(WatchNextOp.Delete(5)), WatchNextRules.plan(emptyList(), listOf(removed)))
    }

    @Test
    fun removedNextEpisodeStaysRemoved() {
        val rows = listOf(WatchNextRow(8, "e2", 1234L, browsable = false, fingerprint = "x"))
        val desired = WatchNextRules.entries(Home(nextUp = listOf(episode("e2", "s1"))), rows, now)
        assertTrue(WatchNextRules.plan(desired, rows).isEmpty())
    }

    @Test
    fun duplicatesForOneTitleDeleted() {
        val desired = WatchNextRules.entries(Home(resume = listOf(movie("m1", position = 600.0))), emptyList(), now)
        val rows = listOf(
            WatchNextRow(1, "m1", desired[0].engagementMs, true, desired[0].fingerprint),
            WatchNextRow(2, "m1", desired[0].engagementMs, true, desired[0].fingerprint),
        )
        assertEquals(listOf(WatchNextOp.Delete(2)), WatchNextRules.plan(desired, rows))
    }

    @Test
    fun homeDecodesLastPlayedAt() {
        val home = MoonriseApi.json.decodeFromString(
            Home.serializer(),
            """{"resume":[{"id":"m1","kind":"movie","title":"A","runtime":100,"user_data":{},"user":{"position":40,"last_played_at":"2026-11-07T10:00:00Z"}}],"next_up":[]}""",
        )
        val e = WatchNextRules.entries(home, emptyList(), now).single()
        assertEquals(40_000L, e.positionMs)
        assertEquals(WatchNextRules.playedAt("2026-11-07T10:00:00Z"), e.engagementMs)
    }
}
