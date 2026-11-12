package app.moonrise.core

/**
 * Connect Through Tailscale, the setting and the node's state, without the node itself:
 * turning it on starts the node, off stops it, and what a stopped node says late is dropped (each
 * start has its own number). Used from the main thread only. [start] gets the start's number to
 * hand back with each report; [stop] stops the node, keeping its sign-in.
 */
class TailscaleSwitch(private val start: (generation: Int) -> Unit, private val stop: () -> Unit) {
    var enabled = false
        private set
    var state: TailscaleState = TailscaleState.Off
        private set
    private var generation = 0
    private var running = false

    /** At launch: the saved setting (a build without Tailscale passes false). */
    fun load(enabled: Boolean) {
        this.enabled = enabled
        if (enabled) begin()
    }

    /** The toggle. Answers whether anything changed (and the setting is to be saved). */
    fun set(on: Boolean): Boolean {
        if (on == enabled) return false
        enabled = on
        if (on) begin() else end()
        return true
    }

    /** Retry after a failure (or with a new auth key): a fresh node with the same sign-in. */
    fun restart() {
        end()
        if (enabled) begin()
    }

    /** Sign Out of Tailscale: the node stops and the setting goes off (the caller forgets the sign-in). */
    fun signOut() {
        end()
        enabled = false
    }

    /**
     * What start [generation]'s node says. Answers true when it is taken (not a stopped node's)
     * and the node has just come up, so the route is worth checking again.
     */
    fun report(generation: Int, s: TailscaleState): Boolean {
        if (generation != this.generation || !running) return false
        val was = state
        state = s
        return s is TailscaleState.Running && was !is TailscaleState.Running
    }

    /** Start [generation] couldn't even begin: it is over, and the toggle can try again. */
    fun startFailed(generation: Int, message: String) {
        if (generation != this.generation || !running) return
        running = false
        state = TailscaleState.Failed(message)
    }

    private fun begin() {
        if (running) return
        running = true
        state = TailscaleState.Starting
        start(++generation)
    }

    private fun end() {
        val was = running
        generation++
        running = false
        state = TailscaleState.Off
        if (was) stop()
    }
}
