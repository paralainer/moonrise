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
}
