package app.moonrise.core

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrl

// How the apps reach the server: the home network's address when it answers, else the server's
// tailnet address through the app's embedded Tailscale node (remote access). The Apple
// apps' `ConnectionRouter`, `TailnetNudge` and `StreamRouteRules`, with the same rules.

/** How the server is reached. */
sealed interface Route {
    data object Checking : Route
    data object Lan : Route
    data class Tailnet(val host: String) : Route
    data object Unreachable : Route

    val label: String
        get() = when (this) {
            Checking -> "Checking…"
            Lan -> "Local network"
            is Tailnet -> "Tailscale · $host"
            Unreachable -> "Unreachable"
        }

    /** The server answers on it. */
    val usable: Boolean get() = this == Lan || this is Tailnet
}

/** The embedded Tailscale node. */
sealed interface TailscaleState {
    data object Off : TailscaleState
    data object Starting : TailscaleState
    /** Waiting for the sign-in; [url] is null while the node is still asking for one. */
    data class NeedsLogin(val url: String?) : TailscaleState
    data class Running(val ip: String) : TailscaleState
    data class Failed(val message: String) : TailscaleState

    companion object {
        /** What the node reports (`moonrisets.Host.OnState`): state and detail. */
        fun of(state: String, detail: String): TailscaleState = when (state) {
            "starting" -> Starting
            "needslogin" -> NeedsLogin(detail.ifEmpty { null })
            "running" -> Running(detail)
            "failed" -> Failed(detail.ifEmpty { "Tailscale stopped." })
            else -> Off
        }
    }
}

/** Why the server can't be reached, with what the viewer can do about it. */
sealed interface ConnectionProblem {
    data object Offline : ConnectionProblem
    data object TailscaleOff : ConnectionProblem
    data class TailscaleNeedsLogin(val url: String?) : ConnectionProblem
    data object TailscaleStarting : ConnectionProblem
    data class TailscaleFailed(val message: String) : ConnectionProblem
    data object ServerDown : ConnectionProblem
}

/** Another device on the tailnet (`moonrisets.Peers`). */
@Serializable
data class TailnetPeer(
    @SerialName("host_name") val hostName: String = "",
    @SerialName("dns_name") val dnsName: String = "",
    val ips: List<String> = emptyList(),
    val online: Boolean = false,
    /** Tagged infrastructure (exit nodes, relays): never the server. */
    val tagged: Boolean = false,
) {
    /** Its IPv4 address if it has one. */
    val ip: String? get() = ips.firstOrNull { ':' !in it } ?: ips.firstOrNull()
}

/** What the app remembers about its server, for finding it on the tailnet. */
data class KnownServer(val name: String?, val serverId: String?, val tailnetHost: String?)

/** The device's network, as far as the node's sockets care. */
data class NetPath(val online: Boolean, val interfaces: List<String>, val gateways: List<String>)

object RouteRules {
    /** How long a route check waits for a node that is starting. */
    const val STARTING_WAIT_MS = 12_000L

    /** Why the server can't be reached (null: it can, or it's still being checked). */
    fun problem(route: Route?, online: Boolean, tailscaleSupported: Boolean, tailscale: TailscaleState): ConnectionProblem? {
        if (route != Route.Unreachable) return null
        if (!online) return ConnectionProblem.Offline
        if (!tailscaleSupported) return ConnectionProblem.ServerDown
        return when (tailscale) {
            TailscaleState.Off -> ConnectionProblem.TailscaleOff
            TailscaleState.Starting -> ConnectionProblem.TailscaleStarting
            is TailscaleState.NeedsLogin -> ConnectionProblem.TailscaleNeedsLogin(tailscale.url)
            is TailscaleState.Failed -> ConnectionProblem.TailscaleFailed(tailscale.message)
            is TailscaleState.Running -> ConnectionProblem.ServerDown
        }
    }

    /**
     * Where to look for the server on the tailnet: the host it was found at before, then online
     * peers named like it, then (only when it has never been found there and its id can tell it
     * apart) up to 8 other online peers. Tagged peers are infrastructure and never asked.
     */
    fun tailnetCandidates(known: KnownServer, peers: List<TailnetPeer>): List<String> {
        val out = mutableListOf<String>()
        known.tailnetHost?.let { out += it }
        val online = peers.filter { it.online && !it.tagged && it.ip != null }.sortedBy { it.hostName }
        val name = known.name?.lowercase()?.takeIf { it.isNotBlank() }
        if (name != null) {
            out += online.filter {
                val h = it.hostName.lowercase()
                val d = it.dnsName.lowercase()
                h == name || h.startsWith("$name-") || d.startsWith("$name.") || d.startsWith("$name-")
            }.mapNotNull { it.ip }
        }
        if (out.isEmpty() && known.serverId != null) out += online.take(8).mapNotNull { it.ip }
        return out.distinct()
    }

    fun parsePeers(json: String): List<TailnetPeer> =
        runCatching { MoonriseApi.json.decodeFromString(ListSerializer(TailnetPeer.serializer()), json) }.getOrDefault(emptyList())

    /**
     * Worth telling the node to rebind its sockets: the path really changed (not the first report,
     * which only says what is there at launch) and there is something to bind to.
     */
    fun shouldNudge(old: NetPath?, new: NetPath): Boolean = old != null && old != new && new.online

    /** The local address that tunnels to the server on the tailnet. */
    fun forwardedUrl(port: Int): HttpUrl = "http://127.0.0.1:$port/".toHttpUrl()

    /** The problem's title, message and wording, as `ConnectionProblemView` ([name]: the server's, when known). */
    fun title(p: ConnectionProblem, name: String?): String = titleNamed(p, name ?: "Your Server")

    private fun titleNamed(p: ConnectionProblem, name: String): String = when (p) {
        ConnectionProblem.Offline -> "You're Offline"
        ConnectionProblem.TailscaleOff, is ConnectionProblem.TailscaleNeedsLogin, is ConnectionProblem.TailscaleFailed -> "Can't Reach $name"
        ConnectionProblem.TailscaleStarting -> "Connecting to $name…"
        ConnectionProblem.ServerDown -> "$name Isn't Responding"
    }

    fun message(p: ConnectionProblem, name: String?, cellular: Boolean, tailscaleSupported: Boolean, tv: Boolean): String =
        messageNamed(p, name ?: "your server", cellular, tailscaleSupported, tv)

    private fun messageNamed(p: ConnectionProblem, name: String, cellular: Boolean, tailscaleSupported: Boolean, tv: Boolean): String = when (p) {
        ConnectionProblem.Offline ->
            if (tv) "Connect this TV to your network to reach your library." else "Turn on Wi-Fi or mobile data to reach your library. Downloads still play."
        ConnectionProblem.TailscaleOff ->
            if (cellular) "You're on mobile data. Join your home Wi-Fi, or connect through Tailscale to reach your library from anywhere."
            else "Moonrise reaches $name on your home network. Away from home? Connect through Tailscale to watch from anywhere."
        is ConnectionProblem.TailscaleNeedsLogin -> "Tailscale needs you to sign in before Moonrise can reach your home server."
        is ConnectionProblem.TailscaleFailed -> "Tailscale couldn't start: ${p.message}"
        ConnectionProblem.TailscaleStarting -> "Joining your tailnet."
        ConnectionProblem.ServerDown ->
            if (tailscaleSupported) "It didn't answer on your home network or over Tailscale. The server or its computer may be off or asleep."
            else "It didn't answer on your home network. The server or its computer may be off or asleep."
    }
}

/** The embedded node, as a route check uses it. */
interface TailnetAccess {
    val state: TailscaleState
    /** The tailnet's other devices (empty when it can't say within a few seconds). */
    suspend fun peers(): List<TailnetPeer>
    /** The local port that tunnels to host:port on the tailnet. Throws when the node can't. */
    fun forward(host: String, port: Int): Int
}

/** A route check's answer: the route, the address to use, and what was learned about the server. */
data class RoutePlan(val route: Route, val baseUrl: HttpUrl, val serverId: String? = null, val tailnetHost: String? = null)

/**
 * One route check (`ConnectionRouter.probe`): the home network's address first; then, when the
 * node is up (a starting one gets [RouteRules.STARTING_WAIT_MS] to come up), the tailnet
 * candidates, each through a local forwarder and accepted only when it is the same server.
 * [serverIdAt] answers the id of the Moonrise server at an address (`GET /v1/info`), null when none.
 * [forceTailnet] pretends the home network doesn't answer (a debug seam).
 */
suspend fun planRoute(
    lan: HttpUrl,
    online: Boolean,
    known: KnownServer,
    tailnet: TailnetAccess?,
    serverIdAt: suspend (HttpUrl) -> String?,
    waitWhileStarting: suspend (() -> Boolean) -> Unit,
    forceTailnet: Boolean = false,
): RoutePlan {
    val unreachable = RoutePlan(Route.Unreachable, lan)
    // No network at all: nothing to probe, and probing would only add timeouts.
    if (!online) return unreachable
    if (!forceTailnet) serverIdAt(lan)?.let { return RoutePlan(Route.Lan, lan, serverId = it) }
    if (tailnet == null) return unreachable
    if (tailnet.state == TailscaleState.Starting) waitWhileStarting { tailnet.state == TailscaleState.Starting }
    if (tailnet.state !is TailscaleState.Running) return unreachable
    val port = lan.port
    for (host in RouteRules.tailnetCandidates(known, tailnet.peers())) {
        val local = runCatching { tailnet.forward(host, port) }.getOrNull() ?: continue
        val url = RouteRules.forwardedUrl(local)
        val id = serverIdAt(url) ?: continue
        if (known.serverId != null && id != known.serverId) continue
        return RoutePlan(Route.Tailnet(host), url, serverId = id, tailnetHost = host)
    }
    return unreachable
}

/** When a stream playing from the server moves to another route (`StreamRouteRules`). */
object StreamRouteRules {
    /** A route that has only just changed may change back (networks settle in bursts). */
    const val ROUTE_SETTLE_S = 1.0
    /** At most one move in this long, so a flapping network doesn't restart the stream repeatedly. */
    const val MIN_BETWEEN_MOVES_S = 5.0
    /** How long a tailnet stream may be stalled before it moves (off the tailnet a healthy one keeps playing). */
    const val STALL_BEFORE_LEAVING_TAILNET_S = 2.0
    /** How long a stall lasts before the router is asked to look for a route. */
    const val STALL_BEFORE_ASKING_S = 3.0
    const val MIN_BETWEEN_ASKS_S = 5.0

    /** Move the stream now? [from] is the route it was opened on, [to] the router's current one. */
    fun shouldMove(from: Route?, to: Route?, stalledFor: Double, routeSettledFor: Double, sinceLastMove: Double): Boolean {
        if (from == null || to == null || from == to || !to.usable) return false
        if (routeSettledFor < ROUTE_SETTLE_S || sinceLastMove < MIN_BETWEEN_MOVES_S) return false
        // Off the home network its address can't work any more: don't wait for the buffer to run out.
        if (from == Route.Lan) return true
        // Otherwise (back home, another tailnet address) only when the stream is in trouble.
        return stalledFor >= STALL_BEFORE_LEAVING_TAILNET_S
    }

    fun shouldAskForRoute(stalledFor: Double, sinceLastAsked: Double): Boolean =
        stalledFor >= STALL_BEFORE_ASKING_S && sinceLastAsked >= MIN_BETWEEN_ASKS_S
}

/**
 * Asks for another route check. A nudge that comes while a check runs is kept for after it (the
 * network can change, or a seam be set, during one), and several nudges make one more check.
 */
class RecheckSignal {
    private val wake = kotlinx.coroutines.channels.Channel<Unit>(kotlinx.coroutines.channels.Channel.CONFLATED)

    fun nudge() { wake.trySend(Unit) }

    /** Waits up to [ms] for a nudge (one already made counts). True if one came. */
    suspend fun await(ms: Long): Boolean = kotlinx.coroutines.withTimeoutOrNull(ms) { wake.receive() } != null
}
