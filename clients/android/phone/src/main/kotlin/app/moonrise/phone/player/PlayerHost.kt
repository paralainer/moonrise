package app.moonrise.phone.player

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.content.pm.ActivityInfo
import android.view.OrientationEventListener
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.moonrise.core.Item
import app.moonrise.shared.Log
import app.moonrise.shared.Session
import app.moonrise.shared.downloads.Downloads
import app.moonrise.shared.player.PlayerModel

/**
 * A title page's player (the iPhone's `InlinePlayback`): one [PlayerModel] at a time, for the
 * film or the episode playing. Next Episode (and the end of an episode) plays the next one here,
 * from its resume point, and the page follows it.
 */
class PlayerHost(private val onEpisode: (Item) -> Unit) {
    var model by mutableStateOf<PlayerModel?>(null)
        private set
    /** Fill the screen (crop) rather than fit the picture: a pinch or a double tap in the middle. */
    var fill by mutableStateOf(false)
    /** A tablet held upright: its full-screen button asked for the screen (no rotation). */
    var fullScreenAsked by mutableStateOf(false)
    /** Bumped each time playback stops: the page loads what was watched. */
    var stops by mutableIntStateOf(0)
        private set

    /** The title playing (with its details once loaded). */
    val item: Item? get() = model?.item

    /** The model when `id` is what plays. */
    fun playing(id: String): PlayerModel? = model?.takeIf { it.item.id == id }

    fun play(context: Context, item: Item, start: Double?, version: String?, hdr: Boolean) {
        val api = Session.api ?: return
        model?.close("another title")
        Log.i("MoonrisePlayer") { "play ${item.id} at ${start ?: "resume"} version=${version ?: "-"}" }
        val app = context.applicationContext
        // Downloaded: the copy on the device plays.
        val m = PlayerModel(app, api, item, version, start, hdr, { ep ->
            onEpisode(ep)
            play(app, ep, if (Downloads.playable(ep.id) != null) null else ep.resumeSeconds, null, hdr)
        }, download = Downloads.playable(item.id))
        model = m
        m.start()
    }

    /** Stops and closes the session (reporting where it was). */
    fun stop(why: String) {
        val m = model ?: return
        model = null
        fullScreenAsked = false
        m.close(why)
        stops++
    }
}

/**
 * Turns the phone's screen for the full-screen button (the iPhone's `SystemControls.rotate`): the
 * orientation is held until the phone is held that way, then the sensor (and the system's
 * auto-rotate setting) has it again, so turning it back leaves full screen.
 */
object Rotation {
    private var listener: OrientationEventListener? = null

    fun request(context: Context, landscape: Boolean) {
        val activity = context.activity() ?: return
        activity.requestedOrientation = if (landscape) ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE else ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
        listener?.disable()
        listener = object : OrientationEventListener(activity) {
            override fun onOrientationChanged(degrees: Int) {
                if (degrees == ORIENTATION_UNKNOWN) return
                val heldLandscape = degrees in 60..120 || degrees in 240..300
                val heldPortrait = degrees < 30 || degrees > 330
                if ((landscape && heldLandscape) || (!landscape && heldPortrait)) release(activity)
            }
        }.also { it.enable() }
    }

    /** The page went away: back to the sensor. */
    fun release(context: Context) {
        listener?.disable()
        listener = null
        context.activity()?.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
    }

    private tailrec fun Context.activity(): Activity? = when (this) {
        is Activity -> this
        is ContextWrapper -> baseContext.activity()
        else -> null
    }
}
