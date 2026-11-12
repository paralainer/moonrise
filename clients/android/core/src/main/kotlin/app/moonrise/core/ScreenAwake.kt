package app.moonrise.core

/**
 * When the player keeps the screen on: only while the video plays, or is on its way to
 * play (buffering after a seek or a slow segment, a lost stream coming back). Paused, ended,
 * failed or closed lets the device's own screen timeout or screensaver run.
 *
 * Follows what the viewer asked for (the player's play-when-ready), not whether frames are
 * moving this instant, so a seek or a buffer while playing doesn't drop the hold and take it
 * again.
 */
object ScreenAwake {
    /**
     * [playWhenReady]: the player is set to play. [ended]: it played to the end. [failed]: an
     * error or a give-up message shows. [recovering]: a lost stream is being brought back, the
     * old player paused on purpose; [resumesAfterRecovery] says whether it plays again then.
     * [closed]: no player (left, or stopped in the background).
     */
    fun hold(
        playWhenReady: Boolean,
        ended: Boolean = false,
        failed: Boolean = false,
        recovering: Boolean = false,
        resumesAfterRecovery: Boolean = false,
        closed: Boolean = false,
    ): Boolean = when {
        closed || failed -> false
        recovering -> resumesAfterRecovery
        else -> playWhenReady && !ended
    }
}
