package app.moonrise.core

import org.junit.Assert.assertEquals
import org.junit.Test

/** The phone player's Skip and Next Episode buttons (`NextEpisodeRules.corner`, as the iPhone's `PlayerView`). */
class CornerRulesTest {
    private val intro = Segment(SegmentKind.Intro, 10.0, 30.0)
    private val credits = Segment(SegmentKind.Credits, 120.0, 150.0)

    private fun corner(active: Segment?, controls: Boolean = false, prompt: Boolean = false, counting: Boolean = false, next: Boolean = false, creditsNext: Boolean = false) =
        NextEpisodeRules.corner(active, controls, prompt, counting, next, creditsNext)

    @Test
    fun skipShowsOnEnteringASegmentThenOnlyWithTheControls() {
        assertEquals(NextEpisodeRules.Corner(skip = true, next = false), corner(intro, prompt = true))
        assertEquals(NextEpisodeRules.Corner(skip = false, next = false), corner(intro))
        assertEquals(NextEpisodeRules.Corner(skip = true, next = false), corner(intro, controls = true))
    }

    @Test
    fun anAutoSkipCountdownKeepsSkipUp() {
        assertEquals(NextEpisodeRules.Corner(skip = true, next = false), corner(intro, counting = true))
    }

    @Test
    fun noSegmentNoSkip() {
        assertEquals(NextEpisodeRules.Corner(skip = false, next = false), corner(null, controls = true, prompt = true, counting = true))
    }

    @Test
    fun nextEpisodeOnlyWithTheControls() {
        assertEquals(NextEpisodeRules.Corner(skip = false, next = false), corner(null, next = true))
        assertEquals(NextEpisodeRules.Corner(skip = false, next = true), corner(null, controls = true, next = true))
    }

    @Test
    fun creditsToTheEndShowOneButtonThatPlaysTheNextEpisode() {
        // Skip Credits says Next Episode then: no second Next Episode beside it.
        assertEquals(NextEpisodeRules.Corner(skip = true, next = false), corner(credits, controls = true, next = true, creditsNext = true))
        // Credits that stop before the end: Skip Credits and Next Episode both.
        assertEquals(NextEpisodeRules.Corner(skip = true, next = true), corner(credits, controls = true, next = true, creditsNext = false))
        // Skip hidden (controls away, prompt over): nothing, as Next Episode waits for the controls.
        assertEquals(NextEpisodeRules.Corner(skip = false, next = false), corner(credits, next = true, creditsNext = true))
    }
}
