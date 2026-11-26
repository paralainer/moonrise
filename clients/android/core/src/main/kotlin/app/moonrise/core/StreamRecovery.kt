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

/**
 * A stream that stops moving while its session answers: the link carries less than the stream
 * needs. Opening the same stream again throws away what had arrived and starts over, so it can't
 * get ahead; recovery goes down to a quality the link carries instead, and when there is no lower
 * one it says the connection is too slow (the server isn't failing). As the Apple apps'
 * `SlowLinkRules`.
 */
object SlowLink {
    /** Share of the measured rate a lower quality may use (a transcode's bitrate varies, and the player needs some to spare). */
    const val HEADROOM = 0.75

    /** What recovery does about a stall. */
    sealed interface Action {
        /** Open the same stream again (a dead connection, a server that went away). */
        data object Restart : Action

        /** Too slow for this quality: go down to [quality]. */
        data class Lower(val quality: Quality) : Action

        /** Too slow, and nothing lower: stop and say so. */
        data object TooSlow : Action
    }

    /** Recovery's choice for a stall (no progress while it should play): [linkTooSlow], then [lowerQuality]. */
    fun action(
        sessionAnswered: Boolean, measured: Double?, needed: Double?, answeredRestarts: Int,
        current: Quality, options: List<Quality>, sourceBitrate: Long?,
    ): Action {
        if (!linkTooSlow(sessionAnswered, measured, needed, answeredRestarts)) return Action.Restart
        return lowerQuality(current, options, sourceBitrate, measured)?.let { Action.Lower(it) } ?: Action.TooSlow
    }

    /**
     * Is the stall the link being too slow for this stream? [sessionAnswered]: the session's
     * playlist answered during the stall. [measured]: bits/s that arrived for the stream during
     * the stall, when known (0: nothing, a dead connection, which a new stream does fix).
     * [needed]: the stream's bits/s, when known. [answeredRestarts]: stalls since playback last
     * ran for a while that opened the same stream again although the session answered.
     */
    fun linkTooSlow(sessionAnswered: Boolean, measured: Double?, needed: Double?, answeredRestarts: Int): Boolean {
        if (!sessionAnswered) return false
        if (measured != null && needed != null && needed > 0) return measured > 0 && measured < needed
        // No measure: once the same stream, opened again, stalled again with the server there.
        return answeredRestarts >= 1
    }

    /**
     * The quality to go down to from [current], or null when none is lower: the best below it that
     * fits the measured rate with headroom, else the lowest; the next one down when nothing was
     * measured. [sourceBitrate] is Original's.
     */
    fun lowerQuality(current: Quality, options: List<Quality>, sourceBitrate: Long?, measured: Double?): Quality? {
        val now = (current.bitrate ?: sourceBitrate)?.toDouble() ?: Double.MAX_VALUE
        val below = options.filter { (it.bitrate?.toDouble() ?: Double.POSITIVE_INFINITY) < now }.sortedByDescending { it.bitrate ?: 0 }
        val next = below.firstOrNull() ?: return null
        if (measured == null || measured <= 0) return next
        return below.firstOrNull { (it.bitrate ?: 0) <= measured * HEADROOM } ?: below.last()
    }
}
