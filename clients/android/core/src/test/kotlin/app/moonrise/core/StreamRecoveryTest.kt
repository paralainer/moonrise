package app.moonrise.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Where a refused quality or version goes back to, and the message when recovery stops. */
class StreamRecoveryTest {
    private val a = StreamChoice(Quality.original, "v1")
    private val b = StreamChoice(Quality.presets[3], "v1")
    private val c = StreamChoice(Quality.presets[4], "v1")

    private fun StreamSwitches.play(choice: StreamChoice, seconds: Int) = repeat(seconds) { played(choice) }

    @Test
    fun aRefusedPickGoesBackToWhatPlayed() {
        val s = StreamSwitches()
        s.play(a, 30)
        s.picked(from = a)
        assertEquals(a, s.fallback(b))
    }

    @Test
    fun switchingBackQuicklyFallsBackToTheChoiceThatPlayedNotTheRefusedOne() {
        val s = StreamSwitches()
        s.play(a, 30)
        s.picked(from = a)
        s.play(b, 3)
        s.picked(from = b)
        assertEquals(b, s.fallback(a))
    }

    @Test
    fun aPickThatNeverPlayedIsNotATarget() {
        val s = StreamSwitches()
        s.play(a, 30)
        s.picked(from = a)
        s.picked(from = b)
        assertNull("nothing else played: the viewer gets the message, not a loop", s.fallback(a))
    }

    @Test
    fun severalPicksStillGoBackToTheLastThatPlayedItsTrial() {
        val s = StreamSwitches()
        s.play(a, 30)
        s.picked(from = a)
        s.play(b, 2)
        s.picked(from = b)
        assertEquals(a, s.fallback(c))
    }

    @Test
    fun beforeAnythingPlayedThePreviousChoiceIsStillTried() {
        val s = StreamSwitches()
        s.picked(from = a)
        assertEquals(a, s.fallback(b))
    }

    @Test
    fun aTrialEndsAfterFiveSecondsAndThenNothingFallsBack() {
        val s = StreamSwitches()
        s.play(a, 30)
        s.picked(from = a)
        repeat(StreamSwitches.TRIAL_SECONDS - 1) { assertFalse(s.played(b)) }
        assertTrue(s.onTrial)
        assertTrue(s.played(b))
        assertFalse(s.onTrial)
        assertNull(s.fallback(b))
    }

    @Test
    fun withoutAPickThereIsNoFallback() {
        val s = StreamSwitches()
        s.play(a, 30)
        assertNull(s.fallback(a))
    }

    @Test
    fun afterFallingBackTheRefusedChoiceIsNoLongerATarget() {
        val s = StreamSwitches()
        s.play(a, 30)
        s.picked(from = a)
        s.play(b, 3)
        s.picked(from = b)
        s.fellBack(a)
        assertFalse(s.onTrial)
        s.picked(from = b)
        assertEquals(b, s.fallback(c))
    }

    private fun RecoveryTries.run(everReady: Boolean, vararg decoder: Boolean) =
        decoder.map { next(everReady, it) }.last()

    @Test
    fun stallsAfterADecoderErrorDontBlameTheDecoder() {
        assertEquals(StreamGiveUp.StreamKeepsFailing, RecoveryTries().run(true, true, true, false, false, false))
    }

    @Test
    fun aDecoderErrorThatUsesTheLastTryBlamesTheDecoder() {
        assertEquals(StreamGiveUp.CantDecode, RecoveryTries().run(true, false, false, false, false, true))
    }

    @Test
    fun aStreamThatNeverPlayedGetsFewerTries() {
        val t = RecoveryTries()
        assertNull(t.next(everReady = false, decoder = false))
        assertNull(t.next(everReady = false, decoder = false))
        assertEquals(StreamGiveUp.StreamKeepsFailing, t.next(everReady = false, decoder = false))
    }

    @Test
    fun playingForAWhileStartsTheCountAgain() {
        val t = RecoveryTries()
        repeat(4) { assertNull(t.next(everReady = true, decoder = true)) }
        t.reset()
        assertNull(t.next(everReady = true, decoder = true))
        assertEquals(1, t.count)
    }

    // A stall while the session answers and data arrives slower than the stream needs: down a
    // quality (or "too slow"), never the same stream again, which throws away what had arrived.
    private val p720 = Quality.of("720p · 4 Mbps")
    private val p480 = Quality.of("480p · 1.5 Mbps")
    /** A 1080p film at 8 Mbit/s: Original, 720p, 480p. */
    private val film = Quality.options(8_000_000, 1080)

    private fun slow(answered: Boolean = true, measured: Double?, needed: Double? = 8e6, restarts: Int = 0, current: Quality = Quality.original) =
        SlowLink.action(answered, measured, needed, restarts, current, film, 8_000_000)

    @Test
    fun originalOverALinkSlowerThanTheFileGoesDownAQuality() {
        assertEquals(SlowLink.Action.Lower(p480), slow(measured = 4.8e6))
        assertEquals(SlowLink.Action.Lower(p720), slow(measured = 7e6))
    }

    @Test
    fun aDeadConnectionOrAGoneServerOpensTheSameStreamAgain() {
        assertEquals(SlowLink.Action.Restart, slow(measured = 0.0))
        assertEquals(SlowLink.Action.Restart, slow(answered = false, measured = 4.8e6))
        assertEquals(SlowLink.Action.Restart, slow(measured = 9e6))
    }

    @Test
    fun withoutAMeasureTheSecondAnsweredStallGoesDown() {
        assertEquals(SlowLink.Action.Restart, slow(measured = null, restarts = 0))
        assertEquals(SlowLink.Action.Lower(p720), slow(measured = null, restarts = 1))
        assertEquals(SlowLink.Action.Lower(p720), slow(measured = null, needed = null, restarts = 1))
    }

    @Test
    fun nothingLowerSaysTheConnectionIsTooSlow() {
        assertEquals(SlowLink.Action.TooSlow, slow(measured = 6e5, needed = 1.5e6, current = p480))
    }

    @Test
    fun aLinkTooSlowForEveryQualityTakesTheLowest() {
        assertEquals(SlowLink.Action.Lower(p480), slow(measured = 1e6))
        assertEquals(SlowLink.Action.Lower(p480), slow(measured = 2e6, needed = 4e6, current = p720))
    }
}
