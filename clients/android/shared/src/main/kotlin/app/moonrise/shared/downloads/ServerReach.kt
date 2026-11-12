package app.moonrise.shared.downloads

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.moonrise.core.MoonriseApi
import app.moonrise.shared.Log
import app.moonrise.shared.Session
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import okhttp3.OkHttpClient
import java.util.concurrent.TimeUnit

/**
 * Whether the signed-in server answers (the iPhone's `ConnectionRouter`, for one route): asked
 * every 30 s while it does and every 5 s while it doesn't, and at once when the network changes.
 * What the screens read to show the downloads instead of an error, and what sends waiting reports.
 */
object ServerReach {
    private const val PREFS = "reach"
    private lateinit var app: Context
    private var started = false

    /** The server answered the last time it was asked (true until the first answer). */
    var reachable by mutableStateOf(true)
        private set

    /** Asked at least once since this launch. */
    var checked by mutableStateOf(false)
        private set

    /** The server's own id, kept so downloads can be told apart with no server. */
    var serverId: String?
        get() = prefs().getString("server_id", null)
        private set(v) { prefs().edit().putString("server_id", v).apply() }

    private fun prefs() = app.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    private val wake = MutableSharedFlow<Unit>(extraBufferCapacity = 1)
    private val probe = OkHttpClient.Builder().connectTimeout(4, TimeUnit.SECONDS).readTimeout(4, TimeUnit.SECONDS).build()

    fun start(context: Context) {
        app = context.applicationContext
        if (started) return
        started = true
        Session.background.launch {
            while (true) {
                check()
                val wait = if (reachable) 30_000L else 5_000L
                withTimeoutOrNull(wait) { wake.first() }
            }
        }
    }

    /** Ask now (the network changed, a screen failed to load). */
    fun nudge() { wake.tryEmit(Unit) }

    /** A new sign-in: its server is asked afresh. */
    fun signedIn() {
        if (started) prefs().edit().remove("server_id").apply()
        checked = false
        reachable = true
        nudge()
    }

    private suspend fun check() {
        val api = Session.api ?: return
        val client = MoonriseApi(api.baseUrl, null, Session.deviceId, Session.deviceName, probe)
        val info = runCatching { client.info() }.getOrNull()
        withContext(kotlinx.coroutines.Dispatchers.Main) {
            val was = reachable
            info?.let { serverId = it.serverId }
            checked = true
            reachable = info != null
            if (was != reachable) Log.i("Moonrise") { "server ${if (reachable) "reachable" else "unreachable"}" }
            if (reachable) { LocalReports.flushSoon(); Downloads.resumeWaiting() }
        }
    }

    /** Checks now and waits for the answer (the player, a retry). */
    suspend fun refresh() {
        check()
    }

    /** The server the download came from can be reached now. */
    fun canReach(serverId: String): Boolean = reachable && this.serverId == serverId

    /** Waits (up to `ms`) for the first answer of this launch. */
    suspend fun ready(ms: Long = 5000) {
        withTimeoutOrNull(ms) { while (!checked) delay(50) }
    }
}
