package app.moonrise.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The screen stays on only while the video plays: paused, ended and failed let it sleep. */
class ScreenAwakeTest {
    @Test
    fun playingHoldsAndPausedReleases() {
        assertTrue("playing", ScreenAwake.hold(playWhenReady = true))
        assertFalse("paused", ScreenAwake.hold(playWhenReady = false))
    }

    @Test
    fun endedErrorAndClosedRelease() {
        assertFalse("ended (the player keeps play-when-ready)", ScreenAwake.hold(playWhenReady = true, ended = true))
        assertFalse("an error", ScreenAwake.hold(playWhenReady = true, failed = true))
        assertFalse("closed", ScreenAwake.hold(playWhenReady = true, closed = true))
    }

    @Test
    fun resumeTakesItAgain() {
        val steps = listOf(true, false, true, false).map { ScreenAwake.hold(playWhenReady = it) }
        assertEquals(listOf(true, false, true, false), steps)
    }

    /** A seek or a buffer while playing: still set to play, so the hold stays (no flicker). */
    @Test
    fun bufferingWhilePlayingHolds() {
        assertTrue(ScreenAwake.hold(playWhenReady = true, ended = false))
    }

    @Test
    fun recoveryFollowsWhatResumes() {
        // The old player is paused on purpose while a lost stream comes back.
        assertTrue("coming back to play", ScreenAwake.hold(playWhenReady = false, recovering = true, resumesAfterRecovery = true))
        assertFalse("coming back paused", ScreenAwake.hold(playWhenReady = false, recovering = true, resumesAfterRecovery = false))
        assertFalse("gave up", ScreenAwake.hold(playWhenReady = false, recovering = false, failed = true))
    }
}
