package app.moonrise.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Connect Through Tailscale on and off. */
class TailscaleSwitchTest {
    private val calls = mutableListOf<String>()
    private val starts = mutableListOf<Int>()
    private val sw = TailscaleSwitch(start = { calls += "start"; starts += it }, stop = { calls += "stop" })

    @Test
    fun offAtLaunchNothingStarts() {
        sw.load(false)
        assertTrue(calls.isEmpty())
        assertEquals(TailscaleState.Off, sw.state)
    }

    @Test
    fun onAtLaunchTheNodeStarts() {
        sw.load(true)
        assertEquals(listOf("start"), calls)
        assertEquals(TailscaleState.Starting, sw.state)
    }

    @Test
    fun turningItOnStartsAndOffStops() {
        assertTrue(sw.set(true))
        assertEquals(TailscaleState.Starting, sw.state)
        assertFalse(sw.set(true))
        assertTrue(sw.set(false))
        assertEquals(listOf("start", "stop"), calls)
        assertEquals(TailscaleState.Off, sw.state)
        assertFalse(sw.enabled)
    }

    @Test
    fun theSignInAndComingUpAreFollowedAndComingUpAsksForARoute() {
        sw.set(true)
        val g = starts.last()
        assertFalse(sw.report(g, TailscaleState.NeedsLogin(null)))
        assertFalse(sw.report(g, TailscaleState.NeedsLogin("https://login.example/a")))
        assertEquals(TailscaleState.NeedsLogin("https://login.example/a"), sw.state)
        assertTrue(sw.report(g, TailscaleState.Running("100.64.0.5")))
        assertFalse(sw.report(g, TailscaleState.Running("100.64.0.5")))
    }

    @Test
    fun aStoppedNodesLateWordsAreDropped() {
        sw.set(true)
        val old = starts.last()
        sw.set(false)
        assertFalse(sw.report(old, TailscaleState.Running("100.64.0.5")))
        assertEquals(TailscaleState.Off, sw.state)
        sw.set(true)
        assertFalse(sw.report(old, TailscaleState.Failed("closed")))
        assertEquals(TailscaleState.Starting, sw.state)
    }

    @Test
    fun aFailedStartCanBeRetried() {
        sw.set(true)
        sw.startFailed(starts.last(), "no network")
        assertEquals(TailscaleState.Failed("no network"), sw.state)
        sw.restart()
        // The failed start was already over: nothing to stop, a new start.
        assertEquals(listOf("start", "start"), calls)
        assertEquals(TailscaleState.Starting, sw.state)
    }

    @Test
    fun restartStopsThenStartsAgain() {
        sw.set(true)
        sw.restart()
        assertEquals(listOf("start", "stop", "start"), calls)
        assertEquals(2, starts.distinct().size)
    }

    @Test
    fun signingOutTurnsItOff() {
        sw.set(true)
        sw.signOut()
        assertFalse(sw.enabled)
        assertEquals(TailscaleState.Off, sw.state)
        assertEquals(listOf("start", "stop"), calls)
        sw.restart()
        assertEquals(listOf("start", "stop"), calls)
    }
}
