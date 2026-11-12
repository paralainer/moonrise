package app.moonrise.core

// What the player does when a stream fails: which choice to go back to, and what to say when it
// stops trying. Plain rules here so the unit tests cover them.

/** A quality and a version of the title: what the viewer can switch between. */
data class StreamChoice(val quality: Quality, val version: String?)

/**
 * What to go back to when this device can't decode a quality or version just picked. A pick is on
 * trial until its stream has played [TRIAL_SECONDS]. The choice to return to is the last one that
 * actually played, and never the one that was just refused.
 */
class StreamSwitches {
    private var settled: StreamChoice? = null
    private var brief: StreamChoice? = null
    private var origin: StreamChoice? = null
    private var seconds = 0

    /** A pick is still on trial. */
    var onTrial = false
        private set

    /** The viewer picked another quality or version. */
    fun picked(from: StreamChoice) {
        if (!onTrial) origin = from
        onTrial = true
        seconds = 0
    }

    /** The stream of [now] played for another second. True when that ends a pick's trial. */
    fun played(now: StreamChoice): Boolean {
        seconds++
        brief = now
        if (seconds < TRIAL_SECONDS) return false
        settled = now
        val ended = onTrial
        onTrial = false
        return ended
    }

    /**
     * Where to go when [refused] can't be decoded, or null when no fallback applies (no pick on
     * trial, or nothing else to go to): the last choice to play its full trial, else the last to
     * play at all, else the one the viewer switched from.
     */
    fun fallback(refused: StreamChoice): StreamChoice? =
        if (!onTrial) null else listOfNotNull(settled, brief, origin).firstOrNull { it != refused }

    /** Going back: the trial is over, and [refused] no longer counts as having played. */
    fun fellBack(refused: StreamChoice) {
        onTrial = false
        seconds = 0
        if (settled == refused) settled = null
        if (brief == refused) brief = null
    }

    companion object {
        const val TRIAL_SECONDS = 5
    }
}

/** What the viewer is told when recovery stops. */
enum class StreamGiveUp { CantDecode, StreamKeepsFailing }

/**
 * The tries at recovering a stream. The message when they run out comes from the failure that
 * used the last try, as in the Apple apps: an earlier decoder error says nothing about a stream
 * that then stalled.
 */
class RecoveryTries {
    var count = 0
        private set

    /** One more try: null to go on, or why to stop. [everReady]: the stream has played. */
    fun next(everReady: Boolean, decoder: Boolean): StreamGiveUp? {
        count++
        if (count <= (if (everReady) 4 else 2)) return null
        return if (decoder) StreamGiveUp.CantDecode else StreamGiveUp.StreamKeepsFailing
    }

    fun reset() {
        count = 0
    }
}
