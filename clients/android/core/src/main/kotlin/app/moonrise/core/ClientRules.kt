package app.moonrise.core

import kotlin.coroutines.cancellation.CancellationException

// Small timing and buffering rules of the TV app, here (plain Kotlin) so the unit tests cover
// them: each was the fix of a bug found in review.

/** Rules the TV app's screens and player follow. */
object ClientRules {
    /** Below this much buffered media (µs), the player never stops loading. */
    const val STARVED_US = 5_000_000L

    /**
     * Media3's buffer budget is full: load anyway when the track playing next has run dry, as
     * after an audio switch during a burned-in subtitle stream (without it, playback got stuck buffering).
     */
    fun keepLoading(bufferedDurationUs: Long): Boolean = bufferedDurationUs < STARVED_US

    /**
     * Where a title starts when it opens: the position asked for, else (nothing asked for) where it
     * was left. With several versions and `version` one of them, that's where this version was left,
     * else the beginning: the title's position can be another cut's (as the Apple apps'
     * `PlayerModel.start`).
     */
    fun startPosition(requested: Double?, item: Item, version: String? = null): Double? =
        requested ?: Versions.resume(item, item.presentFiles.firstOrNull { it.id == version })

    /**
     * Where a stream (re)starts, in ms, given the position in seconds; null: from the start.
     * A restart begins at the position, never at 0 first (the server would start a
     * transcode at segment 0, and an intro there would count as reached).
     */
    fun startPositionMs(start: Double?): Long? = if (start != null && start > 1) (start * 1000).toLong() else null

    /**
     * Ask for lossless multichannel audio (`lossless=1`): an HDMI output (TV, receiver,
     * eARC soundbar) says it takes more than 2 channels of PCM. `hdmiChannelCounts`: each HDMI
     * output's channel counts (an empty list: it didn't say, so not assumed).
     */
    fun wantsLossless(hdmiChannelCounts: List<IntArray>): Boolean = hdmiChannelCounts.any { c -> c.any { it > 2 } }

    /** Android's `Display.HdrCapabilities.HDR_TYPE_DOLBY_VISION`. */
    const val HDR_TYPE_DOLBY_VISION = 1

    /**
     * Ask for Dolby Vision (`dv=1`): HDR is on, the display lists Dolby Vision among its
     * HDR types, and the device has a `video/dolby-vision` decoder. The server then signals a
     * Dolby Vision copy (`SUPPLEMENTAL-CODECS` for profile 8, `dvh1` for profile 5), which Media3
     * 1.6+ plays with that decoder. Anything less: the HDR10/HLG base layer, as before.
     */
    fun wantsDolbyVision(hdrDisplay: Boolean, displayHdrTypes: IntArray, dolbyVisionDecoder: Boolean): Boolean =
        hdrDisplay && dolbyVisionDecoder && HDR_TYPE_DOLBY_VISION in displayHdrTypes

    /** A screen showing an error tries again after this long first… */
    const val FIRST_RETRY_MS = 5_000L

    /** …then twice as long each time, up to this. */
    const val MAX_RETRY_MS = 30_000L

    /** The wait after `wait` before the next retry. */
    fun nextRetryMs(wait: Long): Long = minOf(wait * 2, MAX_RETRY_MS)

    /** How often coming back to the app (or to Home) loads Home again. */
    const val HOME_REFRESH_MS = 30_000L

    /**
     * Whether Home loads again when the app comes back to the front (a handoff from
     * another device showed only after a failure): not while a load runs, nor before the first
     * (`lastLoadMs` 0), at most every [HOME_REFRESH_MS], at once after a failed load.
     */
    fun homeIsStale(loading: Boolean, failed: Boolean, lastLoadMs: Long, nowMs: Long): Boolean {
        if (loading) return false
        if (!failed && (lastLoadMs == 0L || nowMs - lastLoadMs < HOME_REFRESH_MS)) return false
        return true
    }

    /**
     * Whether a caught exception is a failure to report. A cancelled load isn't one: the screen
     * went away (switching tabs while Home loaded logged and showed an error).
     */
    fun isLoadFailure(e: Throwable): Boolean = e !is CancellationException

    /**
     * Home has nothing to show and says so: the server has answered with no rows. Not
     * before the first answer, during a load (the spinner) or after a failure (the error).
     */
    fun homeIsEmpty(loaded: Boolean, loading: Boolean, failed: Boolean, sections: Int): Boolean =
        loaded && !loading && !failed && sections == 0

    const val DEVICE_CODE_MAX_SIZE = 40f
    const val DEVICE_CODE_KERNING = 0.12f
    /** A monospaced glyph's width, in ems (0.6, and a little to spare). */
    private const val DEVICE_CODE_ADVANCE = 0.62f

    /**
     * The sign-in code's text size for the width it has, so it stays on one line (on a
     * 411 dp wide phone the last character of a 9-character code wrapped).
     */
    fun deviceCodeFontSize(characters: Int, widthDp: Float): Float =
        if (characters <= 0) DEVICE_CODE_MAX_SIZE
        else (widthDp / (characters * (DEVICE_CODE_ADVANCE + DEVICE_CODE_KERNING))).coerceIn(12f, DEVICE_CODE_MAX_SIZE)

    /** How much a focused card grows (its lift, drawn as a scale about its centre). */
    const val LIFT_SCALE = 1.1f

    /**
     * The part of a card a row and the page scroll to show, given the part asked for as drawn
     * (`left`, `top`, `right`, `bottom`, in the card's own coordinates) while the card is drawn
     * at `scale` about the centre of its `width` x `height` bounds: the same part at rest.
     *
     * Home's rows and page keep the focused card at a fixed point (Compose's TV pivot),
     * measured from the card as drawn. A card growing into focus moved that point as it grew,
     * so every move along a row bobbed the whole page up and back down by ~10 px.
     */
    fun restingRect(left: Float, top: Float, right: Float, bottom: Float, width: Float, height: Float, scale: Float): FloatArray {
        if (scale <= 0f || scale == 1f) return floatArrayOf(left, top, right, bottom)
        val cx = width / 2; val cy = height / 2
        return floatArrayOf(cx + (left - cx) / scale, cy + (top - cy) / scale, cx + (right - cx) / scale, cy + (bottom - cy) / scale)
    }

    /** A code typed any way ("abcd1234", "ABCD 1234"): the server's form, "ABCD-1234" (`ApproveDeviceView.format`). */
    fun formatDeviceCode(s: String): String {
        val c = s.uppercase().filter { it.isLetterOrDigit() }.take(8)
        return if (c.length > 4) c.take(4) + "-" + c.drop(4) else c
    }

    /** What's wrong with an invitation's form: null when it can be sent, "" when not ready, else what to say (`InviteSignInView.problem`). */
    fun invitationProblem(code: String, username: String, password: String): String? = when {
        code.count { it.isLetterOrDigit() } != 8 -> ""
        username.isBlank() -> ""
        password.length < 8 -> "The password needs at least 8 characters."
        else -> null
    }
}
