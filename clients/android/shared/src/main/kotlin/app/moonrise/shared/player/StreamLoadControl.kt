package app.moonrise.shared.player

import app.moonrise.core.ClientRules
import androidx.annotation.OptIn
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.DefaultLoadControl
import androidx.media3.exoplayer.LoadControl
import app.moonrise.shared.Log

/**
 * Media3's buffering rules, with one change: a track that has run dry is always loaded.
 *
 * The server's streams keep video and audio in separate renditions. When the viewer picks
 * another audio track, Media3 drops the buffered audio but keeps the video, which can already
 * fill the whole buffer budget (a 20 Mbps transcode: 50 s of it is
 * about the size of the default budget). The default rules then refuse to load anything ("target
 * buffer size reached with less than 500 ms of buffered media data"), the new audio never comes,
 * and after 4 s Media3 fails with "Playback stuck buffering and not loading"
 * (ERROR_CODE_FAILED_RUNTIME_CHECK).
 *
 * Here, while less than [STARVED_US] is buffered, loading goes on whatever the budget. Media3
 * loads the track that is furthest behind first, so it is the new audio that comes, a few hundred
 * kilobytes over the budget; the video buffer is left as it is.
 */
@OptIn(UnstableApi::class)
class StreamLoadControl : DefaultLoadControl() {
    private var starved = false

    override fun shouldContinueLoading(parameters: LoadControl.Parameters): Boolean {
        if (super.shouldContinueLoading(parameters)) {
            starved = false
            return true
        }
        val load = keepLoading(parameters.bufferedDurationUs)
        if (load != starved) {
            starved = load
            if (load) Log.i("MoonrisePlayer") { "buffer budget full with ${parameters.bufferedDurationUs / 1000} ms buffered: loading anyway" }
        }
        return load
    }

    companion object {
        /** Below this much buffered media, loading never stops. */
        const val STARVED_US = ClientRules.STARVED_US

        /** The default rules said no: load anyway when the track playing next has run dry. */
        fun keepLoading(bufferedDurationUs: Long): Boolean = ClientRules.keepLoading(bufferedDurationUs)
    }
}
