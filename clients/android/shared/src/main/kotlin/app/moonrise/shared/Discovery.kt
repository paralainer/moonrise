package app.moonrise.shared

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.os.Handler
import android.os.Looper
import kotlinx.coroutines.async
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import java.net.Inet4Address

/** A Moonrise server found on the local network (Bonjour `_moonrise._tcp`, as the server advertises it). */
data class FoundServer(val name: String, val address: String, val unclaimed: Boolean)

/**
 * Servers on the local network as they're found, the whole list each time. Stops looking when
 * the collector stops.
 */
fun discoverServers(context: Context): Flow<List<FoundServer>> = callbackFlow {
    val nsd = context.getSystemService(Context.NSD_SERVICE) as NsdManager
    val wifi = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
    // Some devices drop multicast packets unless an app holds this lock.
    val lock = wifi?.createMulticastLock("moonrise-discovery")?.apply { setReferenceCounted(false); acquire() }
    val found = linkedMapOf<String, FoundServer>()
    val main = Handler(Looper.getMainLooper())
    // One resolve at a time (older Android versions refuse a second while one runs).
    val queue = ArrayDeque<NsdServiceInfo>()
    var resolving = false

    fun next() {
        if (resolving) return
        val info = queue.removeFirstOrNull() ?: return
        resolving = true
        @Suppress("DEPRECATION")
        nsd.resolveService(info, object : NsdManager.ResolveListener {
            override fun onResolveFailed(s: NsdServiceInfo, error: Int) {
                main.post { resolving = false; next() }
            }

            override fun onServiceResolved(s: NsdServiceInfo) {
                main.post {
                    resolving = false
                    @Suppress("DEPRECATION")
                    val host = s.host
                    if (host != null) {
                        val h = if (host is Inet4Address) host.hostAddress else "[${host.hostAddress}]"
                        val txt = s.attributes.mapValues { (_, v) -> v?.let { String(it) } }
                        val name = txt["name"]?.takeIf { it.isNotBlank() } ?: s.serviceName
                        found[s.serviceName] = FoundServer(name, "$h:${s.port}", txt["setup"] == "unclaimed")
                        trySend(found.values.toList())
                    }
                    next()
                }
            }
        })
    }

    val listener = object : NsdManager.DiscoveryListener {
        override fun onDiscoveryStarted(serviceType: String) {}
        override fun onDiscoveryStopped(serviceType: String) {}
        override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) { close() }
        override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {}
        override fun onServiceFound(info: NsdServiceInfo) {
            main.post { queue.addLast(info); next() }
        }
        override fun onServiceLost(info: NsdServiceInfo) {
            main.post { if (found.remove(info.serviceName) != null) trySend(found.values.toList()) }
        }
    }
    trySend(emptyList())
    nsd.discoverServices("_moonrise._tcp", NsdManager.PROTOCOL_DNS_SD, listener)
    awaitClose {
        runCatching { nsd.stopServiceDiscovery(listener) }
        lock?.release()
    }
}

/**
 * Servers found by asking every address of this device's local /24 networks for `GET /v1/info` on
 * the server's port (`MoonriseDiscovery.sweep`, Apple apps), for networks where Bonjour doesn't
 * get through. Each answer is a server; `timeoutMs` per address, all asked at once.
 */
suspend fun sweepServers(timeoutMs: Long = 2000): List<FoundServer> = kotlinx.coroutines.coroutineScope {
    val own = runCatching {
        java.net.NetworkInterface.getNetworkInterfaces().toList().filter { it.isUp && !it.isLoopback && !it.isVirtual }
            .flatMap { it.inetAddresses.toList() }.filterIsInstance<Inet4Address>().mapNotNull { it.hostAddress }
    }.getOrDefault(emptyList())
    val http = app.moonrise.core.MoonriseApi.defaultHttp.newBuilder()
        .connectTimeout(timeoutMs, java.util.concurrent.TimeUnit.MILLISECONDS).readTimeout(timeoutMs, java.util.concurrent.TimeUnit.MILLISECONDS).build()
    app.moonrise.core.LocalSubnet.hosts(own).map { h ->
        async(kotlinx.coroutines.Dispatchers.IO) {
            val address = "$h:${app.moonrise.core.MoonriseApi.DEFAULT_PORT}"
            val base = app.moonrise.core.MoonriseApi.normalize(address) ?: return@async null
            runCatching { app.moonrise.core.MoonriseApi(base, null, "", "", http).info() }.getOrNull()?.let { FoundServer(it.name, address, it.unclaimed) }
        }
    }.mapNotNull { it.await() }
}
