package app.moonrise.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The TV app's timing and buffering rules ([ClientRules]), each a fix from a review. */
class ClientRulesTest {
    /** An audio switch during a burned-in stream left the new audio unloaded ("stuck buffering"). */
    @Test
    fun loadingGoesOnWhileATrackHasRunDry() {
        assertTrue(ClientRules.keepLoading(0))
        assertTrue(ClientRules.keepLoading(400_000))   // the 500 ms Media3 gave up at
        assertTrue(ClientRules.keepLoading(4_999_999))
        assertFalse(ClientRules.keepLoading(5_000_000))
        assertFalse(ClientRules.keepLoading(50_000_000))
    }

    /** With nothing asked for (a title page's Resume on the phone), the player started at 0. */
    @Test
    fun nothingAskedForResumesWhereItWasLeft() {
        fun movie(position: Double?, played: Boolean = false) = Item(id = "m", kind = "movie", user = UserData(position = position, played = played))
        assertEquals(142.5, ClientRules.startPosition(null, movie(142.5))!!, 1e-9)
        assertEquals(142.5, ClientRules.startPosition(null, movie(142.5, played = true))!!, 1e-9)
        assertNull(ClientRules.startPosition(null, movie(0.0)))
        // Asked for: that, even 0 (Play from Beginning).
        assertEquals(0.0, ClientRules.startPosition(0.0, movie(142.5))!!, 1e-9)
        assertEquals(30.0, ClientRules.startPosition(30.0, movie(142.5))!!, 1e-9)
        // The player opens there.
        val player = java.io.File("../shared/src/main/kotlin/app/moonrise/shared/player/PlayerModel.kt").readText()
        assertTrue(player.contains("val start = ClientRules.startPosition(requestedStart, item, wantedVersion)"))
    }

    /** A card's Play on a title with several versions could start at the position another cut was left at. */
    @Test
    fun aVersionWithoutAPositionStartsAtTheBeginning() {
        fun file(id: String, pos: Double?) = MediaFile(id = id, position = pos)
        val cut = Item(id = "m", kind = "movie", files = listOf(file("long", 3600.0), file("short", 0.0)), user = UserData(position = 3600.0))
        assertEquals(3600.0, ClientRules.startPosition(null, cut, "long")!!, 1e-9)
        assertNull(ClientRules.startPosition(null, cut, "short"))
        assertEquals(30.0, ClientRules.startPosition(30.0, cut, "short")!!, 1e-9)
        // No version given, one the title doesn't have, or a single file: the title's.
        assertEquals(3600.0, ClientRules.startPosition(null, cut)!!, 1e-9)
        assertEquals(3600.0, ClientRules.startPosition(null, cut, "gone")!!, 1e-9)
        val one = Item(id = "o", kind = "movie", files = listOf(file("only", null)), user = UserData(position = 90.0))
        assertEquals(90.0, ClientRules.startPosition(null, one, "only")!!, 1e-9)
    }

    /** Autoplay fired on a list's item, which doesn't say which version plays or where it was left. */
    @Test
    fun autoplayWaitsForTheVersionsOfATitleWithSeveral() {
        val listed = Item(id = "m", kind = "movie", versions = 2, user = UserData(position = 3600.0))
        assertFalse(Versions.readyToAutoplay(listed, finished = false))
        // The load failed or ended without its files: it still plays.
        assertTrue(Versions.readyToAutoplay(listed, finished = true))
        val full = listed.copy(files = listOf(MediaFile(id = "a"), MediaFile(id = "b")))
        assertTrue(Versions.readyToAutoplay(full, finished = false))
        // One version, or a server that doesn't say: at once, as before.
        assertTrue(Versions.readyToAutoplay(listed.copy(versions = 1), finished = false))
        assertTrue(Versions.readyToAutoplay(listed.copy(versions = null), finished = false))
        // The phone's title page goes by it.
        val page = java.io.File("../phone/src/main/kotlin/app/moonrise/phone/ui/TitleScreen.kt").readText()
        assertTrue(page.contains("LaunchedEffect(focus?.id, readyToAutoplay)"))
        assertTrue(page.contains("if (!readyToAutoplay || page.autoplayed"))
    }

    /** Every restart loaded from 0 and then seeked (a wasted transcode run, an intro at 0 marked reached). */
    @Test
    fun restartsBeginAtThePosition() {
        assertEquals(754_500L, ClientRules.startPositionMs(754.5))
        assertEquals(2_500L, ClientRules.startPositionMs(2.5))
        assertNull(ClientRules.startPositionMs(null))
        assertNull(ClientRules.startPositionMs(0.0))
        assertNull(ClientRules.startPositionMs(1.0))
    }

    /** Screens retry by themselves, 5 s then doubling, never more than 30 s apart. */
    @Test
    fun retriesBackOff() {
        val waits = generateSequence(ClientRules.FIRST_RETRY_MS) { ClientRules.nextRetryMs(it) }.take(6).toList()
        assertEquals(listOf(5_000L, 10_000L, 20_000L, 30_000L, 30_000L, 30_000L), waits)
    }

    /** A handoff from another device showed on Home only after a failed load. */
    @Test
    fun homeReloadsWhenTheAppComesBack() {
        val now = 10_000_000L
        assertTrue(ClientRules.homeIsStale(loading = false, failed = false, lastLoadMs = now - 30_001, nowMs = now))
        assertFalse(ClientRules.homeIsStale(loading = false, failed = false, lastLoadMs = now - 5_000, nowMs = now))
        assertTrue(ClientRules.homeIsStale(loading = false, failed = true, lastLoadMs = now - 5_000, nowMs = now))
        assertFalse(ClientRules.homeIsStale(loading = true, failed = true, lastLoadMs = now - 600_000, nowMs = now))
        // Never loaded yet: Home's first load is on its way.
        assertFalse(ClientRules.homeIsStale(loading = false, failed = false, lastLoadMs = 0, nowMs = now))
        assertTrue(ClientRules.homeIsStale(loading = false, failed = true, lastLoadMs = 0, nowMs = now))
    }

    /**
     * Rows and the page scrolled to a card as drawn, mid-lift, so each move along a Home
     * row bobbed the whole page. They now get the card at rest, whatever its lift.
     */
    @Test
    fun aLiftedCardIsShownAtItsRestingBounds() {
        val (w, h) = 440f to 247.5f
        // The whole card, as drawn at each step of its lift: always its bounds at rest.
        for (lift in listOf(0f, 0.25f, 0.5f, 1f)) {
            val s = 1f + (ClientRules.LIFT_SCALE - 1f) * lift
            val drawn = floatArrayOf(w / 2 * (1 - s), h / 2 * (1 - s), w / 2 * (1 + s), h / 2 * (1 + s))
            val r = ClientRules.restingRect(drawn[0], drawn[1], drawn[2], drawn[3], w, h, s)
            assertEquals("left at lift $lift", 0f, r[0], 0.01f)
            assertEquals("top at lift $lift", 0f, r[1], 0.01f)
            assertEquals("right at lift $lift", w, r[2], 0.01f)
            assertEquals("bottom at lift $lift", h, r[3], 0.01f)
        }
        // A part of it (its lower half, drawn fully lifted): the same part at rest.
        val s = ClientRules.LIFT_SCALE
        val r = ClientRules.restingRect(w / 2 * (1 - s), h / 2, w / 2 * (1 + s), h / 2 * (1 + s), w, h, s)
        assertEquals(h / 2, r[1], 0.01f)
        assertEquals(h, r[3], 0.01f)
        // At rest, nothing changes.
        assertEquals(listOf(1f, 2f, 3f, 4f), ClientRules.restingRect(1f, 2f, 3f, 4f, w, h, 1f).toList())
    }

    /** Switching tabs while Home loaded logged "home failed: The coroutine scope left the composition". */
    @Test
    fun aCancelledLoadIsNotAFailure() {
        assertFalse(ClientRules.isLoadFailure(kotlinx.coroutines.CancellationException("left the composition")))
        assertFalse(ClientRules.isLoadFailure(java.util.concurrent.CancellationException()))
        assertTrue(ClientRules.isLoadFailure(java.io.IOException("timeout")))
        assertTrue(ClientRules.isLoadFailure(MoonriseHttpException(500, "")))
    }

    /**
     * Leaving the Library during a load logged "library failed: The coroutine scope left
     * the composition". Every catch-all in the phone app has to let a cancellation through.
     */
    @Test
    fun everyPhoneScreenTreatsACancelledLoadAsACancellation() {
        val sources = java.io.File("../phone/src/main/kotlin").walkTopDown().filter { it.extension == "kt" }.toList()
        assertTrue("the phone app's sources are where this test expects", sources.size > 10)
        val catchAll = Regex("""catch \(\w+: (Exception|Throwable)\)""")
        val careless = sources.flatMap { file ->
            val text = file.readText()
            catchAll.findAll(text).mapNotNull { m ->
                val endOfNextLine = text.indexOf('\n', text.indexOf('\n', m.range.last) + 1).let { if (it < 0) text.length else it }
                val body = text.substring(m.range.last, endOfNextLine)
                if ("isLoadFailure" in body || "CancellationException" in body) null
                else "${file.name}:${text.substring(0, m.range.first).count { it == '\n' } + 1}"
            }.toList()
        }
        assertEquals(emptyList<String>(), careless)
    }

    /** Home stayed blank when the server answered with no rows. */
    @Test
    fun homeSaysSoOnlyWhenTheServerAnsweredWithNothing() {
        assertTrue(ClientRules.homeIsEmpty(loaded = true, loading = false, failed = false, sections = 0))
        assertFalse("before the first answer", ClientRules.homeIsEmpty(loaded = false, loading = false, failed = false, sections = 0))
        assertFalse("while loading", ClientRules.homeIsEmpty(loaded = true, loading = true, failed = false, sections = 0))
        assertFalse("after a failure", ClientRules.homeIsEmpty(loaded = true, loading = false, failed = true, sections = 0))
        assertFalse("with rows", ClientRules.homeIsEmpty(loaded = true, loading = false, failed = false, sections = 1))
    }

    /** On a 411 dp wide phone (239 dp for the text) the last character of the code wrapped. */
    @Test
    fun theSignInCodeFitsOnOneLine() {
        val size = ClientRules.deviceCodeFontSize(9, 239f)
        assertTrue("got $size", size < ClientRules.DEVICE_CODE_MAX_SIZE)
        assertTrue("got $size", 9 * size * (0.6f + ClientRules.DEVICE_CODE_KERNING) <= 239f)
        // Room to spare: the full size, never larger.
        assertEquals(ClientRules.DEVICE_CODE_MAX_SIZE, ClientRules.deviceCodeFontSize(9, 800f), 0f)
        assertEquals(ClientRules.DEVICE_CODE_MAX_SIZE, ClientRules.deviceCodeFontSize(0, 239f), 0f)
    }

    @Test
    fun deviceCodesAreTypedAnyWay() {
        assertEquals("ABCD-1234", ClientRules.formatDeviceCode("abcd1234"))
        assertEquals("ABCD-1234", ClientRules.formatDeviceCode("ABCD 1234"))
        assertEquals("AB", ClientRules.formatDeviceCode("ab"))
        assertEquals("ABCD-1234", ClientRules.formatDeviceCode("abcd-1234-extra"))
    }

    @Test
    fun anInvitationNeedsItsCodeNameAndAPassword() {
        assertEquals("", ClientRules.invitationProblem("abcd", "me", "password1"))
        assertEquals("", ClientRules.invitationProblem("ABCD-1234", " ", "password1"))
        assertEquals("The password needs at least 8 characters.", ClientRules.invitationProblem("ABCD-1234", "me", "short"))
        assertEquals(null, ClientRules.invitationProblem("ABCD-1234", "me", "password1"))
    }
}
