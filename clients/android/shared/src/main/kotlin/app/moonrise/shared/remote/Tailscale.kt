package app.moonrise.shared.remote

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.moonrise.core.NetPath
import app.moonrise.core.RouteRules
import app.moonrise.core.TailnetAccess
import app.moonrise.core.TailnetPeer
import app.moonrise.core.TailscaleState
import app.moonrise.core.TailscaleSwitch
import app.moonrise.shared.BuildConfig
import app.moonrise.shared.Log
import app.moonrise.shared.Session
import app.moonrise.shared.downloads.ServerReach
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.net.Inet6Address
import java.net.NetworkInterface

/** The Tailscale node, as the app's build has it (`TailscaleEngines`): the Go library, or none. */
interface TailscaleEngine {
    /** What the node tells the app, and asks of it (`moonrisets.Host`). */
    interface Host {
        fun onState(state: String, detail: String)
        fun interfaces(): String
        fun log(line: String)
    }

    fun start(dir: String, hostName: String, authKey: String, verbose: Boolean, host: Host)
    fun stop()
    fun peers(): String
    fun forward(target: String, preferredPort: Int): Int
    fun rebind()
}

/**
 * The embedded Tailscale node (userspace, no VPN): with Connect Through Tailscale on, the app joins
 * the tailnet as its own device, so the home server is reachable away from home without the
 * Tailscale app or Android's VPN slot. As the Apple apps' `TailscaleService`. The node
 * talks to Tailscale's coordination service only while this is on (ADR 0013's stated exception).
 */
object Tailscale : TailnetAccess {
    private const val PREFS = "tailscale"
    private lateinit var app: Context
    private var hostName = "moonrise-android"
    private val engine: TailscaleEngine? get() = TailscaleEngines.engine

    /** This build has the Tailscale library (else no Remote Access section). */
    val isSupported: Boolean get() = TailscaleEngines.engine != null

    // The setting and the node's state (`TailscaleSwitch`, in core), mirrored for Compose.
    private val switch = TailscaleSwitch(start = { gen -> launch(gen) }, stop = { halt() })
    private var stateMirror by mutableStateOf<TailscaleState>(TailscaleState.Off)
    private var enabledMirror by mutableStateOf(false)

    override val state: TailscaleState get() = stateMirror

    /** Connect Through Tailscale: on starts the node, off stops it (its sign-in is kept). */
    var enabled: Boolean
        get() = enabledMirror
        set(on) {
            if (!isSupported) return
            if (switch.set(on)) prefs().edit().putBoolean("enabled", on).apply()
            sync()
            if (!on) ServerReach.nudge()
        }

    val isRunning: Boolean get() = state is TailscaleState.Running

    /** Debug seam (`--es tsauthkey <key>`, the Dev app only): signs in with an auth key, as an ephemeral node. */
    var debugAuthKey: String? = null
        set(v) {
            field = v
            if (v != null && ::app.isInitialized) { if (enabled) restart() else enabled = true }
        }

    // One start or stop at a time, in order: a restart's start waits for the stop to finish.
    @OptIn(ExperimentalCoroutinesApi::class)
    private val engineScope = CoroutineScope(SupervisorJob() + Dispatchers.IO.limitedParallelism(1))
    private var nudge: Job? = null
    private var lastPath: NetPath? = null

    private val stateDir: File get() = File(app.filesDir, "tailscale")

    /** At launch: [hostName] is the name the device gets on the tailnet. */
    fun init(context: Context, hostName: String) {
        app = context.applicationContext
        this.hostName = hostName
        switch.load(isSupported && prefs().getBoolean("enabled", false))
        sync()
    }

    private fun sync() {
        stateMirror = switch.state
        enabledMirror = switch.enabled
    }

    /** Start [gen]: the node comes up on the engine's thread, and reports back on the main one. */
    private fun launch(gen: Int) {
        val e = engine ?: return
        val dir = stateDir.apply { mkdirs() }.path
        val key = debugAuthKey.orEmpty()
        val host = object : TailscaleEngine.Host {
            override fun onState(state: String, detail: String) {
                val s = TailscaleState.of(state, detail)
                Log.i(TAG) { "state: ${if (s is TailscaleState.NeedsLogin) "needs login${if (s.url != null) " (page ready)" else ""}" else s.toString()}" }
                Session.main.post {
                    val cameUp = switch.report(gen, s)
                    sync()
                    if (cameUp) ServerReach.nudge()
                }
            }

            override fun interfaces(): String = interfaceList()
            override fun log(line: String) { Log.d(TAG) { line } }
        }
        engineScope.launch {
            runCatching { e.start(dir, hostName, key, BuildConfig.DEBUG, host) }.onFailure { err ->
                Log.w(TAG, "couldn't start: ${err.message}")
                Session.main.post { switch.startFailed(gen, err.message ?: "Tailscale couldn't start."); sync() }
            }
        }
    }

    private fun halt() {
        val e = engine ?: return
        nudge?.cancel()
        engineScope.launch { runCatching { e.stop() } }
    }

    /** Retry after a failure: a fresh node with the same sign-in. */
    fun restart() {
        switch.restart()
        sync()
    }

    /** Forget this device's sign-in to the tailnet: the next start asks to sign in again. */
    fun signOut() {
        val dir = stateDir
        switch.signOut()
        sync()
        engineScope.launch { dir.deleteRecursively() }
        prefs().edit().clear().apply()
        ServerReach.nudge()
    }

    override suspend fun peers(): List<TailnetPeer> {
        val e = engine ?: return emptyList()
        if (!isRunning) return emptyList()
        return withContext(Dispatchers.IO) { runCatching { RouteRules.parsePeers(e.peers()) }.getOrDefault(emptyList()) }
    }

    /** The local port that tunnels to host:port; the same port as last time when it is free. */
    override fun forward(host: String, port: Int): Int {
        val e = engine ?: error("no Tailscale in this build")
        check(isRunning) { "Tailscale isn't running" }
        val key = "port.$host:$port"
        val local = e.forward("$host:$port", prefs().getInt(key, 0))
        prefs().edit().putInt(key, local).apply()
        return local
    }

    /**
     * The device's network: when it really changed, tell the node to rebind its sockets (they may
     * belong to a network that has gone), a moment after the burst of updates ends. Best effort.
     */
    fun networkChanged(path: NetPath) {
        val e = engine ?: return
        val old = lastPath
        lastPath = path
        if (!isRunning || !RouteRules.shouldNudge(old, path)) return
        nudge?.cancel()
        nudge = Session.background.launch {
            delay(1000)
            Log.i(TAG) { "network changed: rebind" }
            runCatching { e.rebind() }
        }
    }

    private fun prefs() = app.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private const val TAG = "MoonriseTS"
}

/**
 * The device's network interfaces for the node (Android doesn't let apps read the kernel's list),
 * one per line: `name|index|mtu|flags|addr/prefix,…`, flags u (up), l (loopback), m (multicast),
 * p (point-to-point). As `moonrisets.parseInterfaces` reads them.
 */
internal fun interfaceList(): String = runCatching {
    NetworkInterface.getNetworkInterfaces()?.toList().orEmpty().mapNotNull { ni ->
        runCatching {
            val flags = buildString {
                if (ni.isUp) append('u')
                if (ni.isLoopback) append('l')
                if (ni.supportsMulticast()) append('m')
                if (ni.isPointToPoint) append('p')
            }
            val addrs = ni.interfaceAddresses.mapNotNull { a ->
                val ip = a.address ?: return@mapNotNull null
                val host = (ip.hostAddress ?: return@mapNotNull null).substringBefore('%')
                val max = if (ip is Inet6Address) 128 else 32
                "$host/${a.networkPrefixLength.toInt().coerceIn(0, max)}"
            }.joinToString(",")
            "${ni.name}|${ni.index}|${ni.mtu}|$flags|$addrs"
        }.getOrNull()
    }.joinToString("\n")
}.getOrDefault("")
