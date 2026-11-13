package app.moonrise.shared.downloads

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.moonrise.core.ConnectionProblem
import app.moonrise.core.KnownServer
import app.moonrise.core.MoonriseApi
import app.moonrise.core.Route
import app.moonrise.core.RouteRules
import app.moonrise.core.RecheckSignal
import app.moonrise.core.planRoute
import app.moonrise.shared.Log
import app.moonrise.shared.Network
import app.moonrise.shared.Session
import app.moonrise.shared.remote.Tailscale
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import okhttp3.HttpUrl
import okhttp3.OkHttpClient
import java.util.concurrent.TimeUnit

/**
 * How the signed-in server is reached (the Apple apps' `ConnectionRouter`): its home address when
 * that answers, else its tailnet address through the embedded Tailscale node; the
 * server's client (`Session.api`) is pointed at the route found. Asked every 30 s while it answers
 * and every 5 s while it doesn't, and at once when the network changes or the app comes to the
 * front. What the screens read to show the downloads instead of an error, and what sends waiting
 * reports.
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

    /** How the server is reached now; null before the first check (or signed out). */
    var route by mutableStateOf<Route?>(null)
        private set

    /** When [route] last changed (wall time, ms), for the player's moves between routes. */
    @Volatile var routeChangedAt = 0L
        private set

    /** Debug seam (`--ez forcetailnet true`): pretend the home network doesn't answer. */
    @Volatile var forceTailnet = false

    /** The server's own id, kept so downloads can be told apart with no server. */
    var serverId: String?
        get() = prefs().getString("server_id", null)
        private set(v) { prefs().edit().putString("server_id", v).apply() }

    /** The server's name, as it answered at home (for finding it on the tailnet, and the problem's words). */
    val serverName: String? get() = if (::app.isInitialized) prefs().getString("server_name", null) else null

    private fun prefs() = app.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    private val wake = RecheckSignal()
    private val probe = OkHttpClient.Builder().connectTimeout(4, TimeUnit.SECONDS).readTimeout(4, TimeUnit.SECONDS).build()
    private val passes = Mutex()
    private val AWAY = okhttp3.HttpUrl.Builder().scheme("http").host("127.0.0.1").port(9).build()

    fun start(context: Context) {
        app = context.applicationContext
        if (started) return
        started = true
        // Requests wait for the first check of a launch, so screens don't load against a home
        // address that doesn't answer away from home.
        Session.routeGate = { if (!checked) ready() }
        // Back in front (after the device slept, say): the network may have changed meanwhile.
        (app as? android.app.Application)?.registerActivityLifecycleCallbacks(object : android.app.Application.ActivityLifecycleCallbacks {
            override fun onActivityStarted(activity: android.app.Activity) = nudge()
            override fun onActivityCreated(activity: android.app.Activity, state: android.os.Bundle?) {}
            override fun onActivityResumed(activity: android.app.Activity) {}
            override fun onActivityPaused(activity: android.app.Activity) {}
            override fun onActivityStopped(activity: android.app.Activity) {}
            override fun onActivitySaveInstanceState(activity: android.app.Activity, state: android.os.Bundle) {}
            override fun onActivityDestroyed(activity: android.app.Activity) {}
        })
        Session.background.launch {
            while (true) {
                check()
                val wait = if (reachable) 30_000L else 5_000L
                wake.await(wait)
            }
        }
    }

    /** Ask now (the network changed, a screen failed to load, the app came to the front). */
    fun nudge() = wake.nudge()

    /** A new sign-in: its server is asked afresh. */
    fun signedIn() {
        if (started) prefs().edit().remove("server_id").remove("server_name").remove("tailnet_host").apply()
        checked = false
        reachable = true
        route = null
        nudge()
    }

    /** Why the server can't be reached (null: it can, or it's still being checked). */
    val problem: ConnectionProblem?
        get() = if (!::app.isInitialized) null else RouteRules.problem(route, Network.online, Tailscale.isSupported, Tailscale.state)

    /** One pass at a time; one asked for during another runs after it, with fresh answers. */
    private suspend fun check() = passes.withLock { pass() }

    private suspend fun pass() {
        val api = Session.api ?: run { withContext(Dispatchers.Main) { route = null }; return }
        val lan = Session.homeAddress ?: api.baseUrl
        if (route == null) withContext(Dispatchers.Main) { route = Route.Checking }
        val p = prefs()
        val known = KnownServer(p.getString("server_name", null), p.getString("server_id", null), p.getString("tailnet_host", null))
        var name: String? = null
        val plan = planRoute(
            lan = lan,
            online = Network.online,
            known = known,
            tailnet = if (Tailscale.isSupported) Tailscale else null,
            serverIdAt = { url -> serverAt(url)?.let { (id, n) -> if (url == lan) name = n; id } },
            waitWhileStarting = { still -> withTimeoutOrNull(RouteRules.STARTING_WAIT_MS) { while (still()) delay(300) } },
            forceTailnet = forceTailnet,
        )
        // Signed out, or into another server, while asking: the answer is stale.
        if (Session.api !== api) return
        // The seam pretends to be away: the home address mustn't answer the screens either.
        api.baseUrl = if (forceTailnet && plan.route == Route.Unreachable) AWAY else plan.baseUrl
        withContext(Dispatchers.Main) {
            val was = reachable
            plan.serverId?.let { serverId = it }
            prefs().edit().apply {
                name?.let { putString("server_name", it) }
                plan.tailnetHost?.let { putString("tailnet_host", it) }
            }.apply()
            if (route != plan.route) {
                Log.i("Moonrise") { "route: ${route?.label ?: "-"} -> ${plan.route.label}" }
                route = plan.route
                routeChangedAt = System.currentTimeMillis()
            }
            checked = true
            reachable = plan.route.usable
            if (was != reachable) Log.i("Moonrise") { "server ${if (reachable) "reachable" else "unreachable"}" }
            if (reachable) { LocalReports.flushSoon(); Downloads.resumeWaiting() }
        }
    }

    /** The id and name of the Moonrise server answering at [base] (`GET /v1/info`), null when none does. */
    private suspend fun serverAt(base: HttpUrl): Pair<String, String>? {
        val client = MoonriseApi(base, null, Session.deviceId, Session.deviceName, probe)
        return runCatching { client.info() }.getOrNull()?.let { it.serverId to it.name }
    }

    /** Checks now and waits for the answer (the player, a retry). */
    suspend fun refresh() {
        check()
    }

    /** The server the download came from can be reached now. */
    fun canReach(serverId: String): Boolean = reachable && this.serverId == serverId

    /**
     * Waits (up to `ms`) for the first answer of this launch. With Tailscale on, the first answer
     * away from home can wait for the node to come up.
     */
    suspend fun ready(ms: Long = if (Tailscale.enabled) 20_000 else 5_000) {
        withTimeoutOrNull(ms) { while (!checked) delay(50) }
    }
}
