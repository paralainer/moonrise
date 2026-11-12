package app.moonrise.core

import kotlinx.coroutines.runBlocking
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrl
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** How the apps reach the server at home and away: the Apple apps' `ConnectionRouter` rules. */
class RoutingTest {
    private val lan = "http://192.168.1.20:8097/".toHttpUrl()
    private val known = KnownServer(name = "nas", serverId = "id-one", tailnetHost = null)

    private fun peer(host: String, ip: String, online: Boolean = true, tagged: Boolean = false, dns: String = "$host.tail0.ts.net.") =
        TailnetPeer(host, dns, listOf(ip, "fd7a:115c:a1e0::1"), online, tagged)

    /** A node in [state] whose forwarders map 100.x hosts to local ports, recording what was asked. */
    private class FakeNode(override var state: TailscaleState, val peerList: List<TailnetPeer> = emptyList()) : TailnetAccess {
        val forwarded = mutableListOf<String>()
        var peersAsked = 0
        override suspend fun peers(): List<TailnetPeer> { peersAsked++; return peerList }
        override fun forward(host: String, port: Int): Int {
            forwarded += "$host:$port"
            return 40000 + host.substringAfterLast('.').toInt()
        }
    }

    /** Which addresses answer, and with which server id. */
    private fun answers(vararg pairs: Pair<HttpUrl, String>): suspend (HttpUrl) -> String? {
        val m = pairs.toMap()
        return { m[it] }
    }

    private fun plan(
        node: TailnetAccess?,
        serverIdAt: suspend (HttpUrl) -> String?,
        online: Boolean = true,
        k: KnownServer = known,
        force: Boolean = false,
        wait: suspend (() -> Boolean) -> Unit = {},
    ) = runBlocking { planRoute(lan, online, k, node, serverIdAt, wait, force) }

    @Test
    fun atHomeTheLocalNetworkIsUsedAndTheTailnetIsNotAsked() {
        val node = FakeNode(TailscaleState.Running("100.64.0.5"), listOf(peer("nas", "100.64.0.9")))
        val p = plan(node, answers(lan to "id-one"))
        assertEquals(Route.Lan, p.route)
        assertEquals(lan, p.baseUrl)
        assertEquals("id-one", p.serverId)
        assertEquals(0, node.peersAsked)
        assertTrue(node.forwarded.isEmpty())
    }

    @Test
    fun homeBehaviourIsUnchangedWithoutTailscale() {
        assertEquals(Route.Lan, plan(null, answers(lan to "id-one")).route)
        val away = plan(null, answers())
        assertEquals(Route.Unreachable, away.route)
        assertEquals(lan, away.baseUrl)
    }

    @Test
    fun awayTheServerIsReachedThroughTheTailnetByItsName() {
        val node = FakeNode(TailscaleState.Running("100.64.0.5"), listOf(peer("laptop", "100.64.0.7"), peer("nas", "100.64.0.9")))
        val p = plan(node, answers(RouteRules.forwardedUrl(40009) to "id-one"))
        assertEquals(Route.Tailnet("100.64.0.9"), p.route)
        assertEquals("http://127.0.0.1:40009/".toHttpUrl(), p.baseUrl)
        assertEquals("100.64.0.9", p.tailnetHost)
        assertEquals(listOf("100.64.0.9:8097"), node.forwarded)
    }

    @Test
    fun theHostFoundBeforeIsTriedFirst() {
        val node = FakeNode(TailscaleState.Running("100.64.0.5"), listOf(peer("nas", "100.64.0.9")))
        val p = plan(node, answers(RouteRules.forwardedUrl(40003) to "id-one"), k = known.copy(tailnetHost = "100.64.0.3"))
        assertEquals(Route.Tailnet("100.64.0.3"), p.route)
        assertEquals(listOf("100.64.0.3:8097"), node.forwarded)
    }

    @Test
    fun anotherMoonriseServerOnTheTailnetIsNotTakenForThisOne() {
        val node = FakeNode(TailscaleState.Running("100.64.0.5"), listOf(peer("nas", "100.64.0.9"), peer("nas-2", "100.64.0.10")))
        val p = plan(node, answers(RouteRules.forwardedUrl(40009) to "someone-else", RouteRules.forwardedUrl(40010) to "id-one"))
        assertEquals(Route.Tailnet("100.64.0.10"), p.route)
    }

    @Test
    fun awayWithTailscaleOffOrSignedOutIsUnreachable() {
        for (s in listOf(TailscaleState.Off, TailscaleState.NeedsLogin("https://login.example/a"), TailscaleState.Failed("no"))) {
            val node = FakeNode(s, listOf(peer("nas", "100.64.0.9")))
            val p = plan(node, answers(RouteRules.forwardedUrl(40009) to "id-one"))
            assertEquals(s.toString(), Route.Unreachable, p.route)
            assertEquals(lan, p.baseUrl)
            assertTrue(node.forwarded.isEmpty())
        }
    }

    @Test
    fun offlineNothingIsProbed() {
        var asked = 0
        val p = plan(FakeNode(TailscaleState.Running("100.64.0.5")), { asked++; "id-one" }, online = false)
        assertEquals(Route.Unreachable, p.route)
        assertEquals(0, asked)
    }

    @Test
    fun aStartingNodeIsWaitedFor() {
        val node = FakeNode(TailscaleState.Starting, listOf(peer("nas", "100.64.0.9")))
        var waited = false
        val p = plan(node, answers(RouteRules.forwardedUrl(40009) to "id-one"), wait = { still ->
            assertTrue(still())
            waited = true
            node.state = TailscaleState.Running("100.64.0.5")
            assertFalse(still())
        })
        assertTrue(waited)
        assertEquals(Route.Tailnet("100.64.0.9"), p.route)
    }

    @Test
    fun theForceTailnetSeamSkipsTheLocalNetwork() {
        val node = FakeNode(TailscaleState.Running("100.64.0.5"), listOf(peer("nas", "100.64.0.9")))
        val p = plan(node, answers(lan to "id-one", RouteRules.forwardedUrl(40009) to "id-one"), force = true)
        assertEquals(Route.Tailnet("100.64.0.9"), p.route)
    }

    @Test
    fun aForwarderThatCantOpenMovesOnToTheNextCandidate() {
        val node = object : TailnetAccess {
            override val state = TailscaleState.Running("100.64.0.5")
            override suspend fun peers() = listOf(peer("nas", "100.64.0.9"), peer("nas-b", "100.64.0.10"))
            override fun forward(host: String, port: Int): Int = if (host.endsWith(".9")) error("no port") else 40010
        }
        assertEquals(Route.Tailnet("100.64.0.10"), plan(node, answers(RouteRules.forwardedUrl(40010) to "id-one")).route)
    }

    @Test
    fun candidatesAreNamedPeersOnlineAndUntagged() {
        val peers = listOf(
            peer("nas", "100.64.0.9"),
            peer("NAS-backup", "100.64.0.10"),
            peer("box", "100.64.0.11", dns = "nas.tail0.ts.net."),
            peer("nas-offline", "100.64.0.12", online = false),
            peer("nas-exit", "100.64.0.13", tagged = true),
            peer("phone", "100.64.0.14"),
        )
        val c = RouteRules.tailnetCandidates(known, peers)
        assertEquals(3, c.size)
        assertEquals(setOf("100.64.0.9", "100.64.0.10", "100.64.0.11"), c.toSet())
        // The host found before leads, even when the node doesn't list it as online.
        assertEquals("100.64.0.3", RouteRules.tailnetCandidates(known.copy(tailnetHost = "100.64.0.3"), peers).first())
    }

    @Test
    fun theWholeTailnetIsSweptOnlyWhenTheServerCanBeToldApart() {
        val peers = (1..10).map { peer("dev$it", "100.64.0.${20 + it}") }
        assertEquals(8, RouteRules.tailnetCandidates(known.copy(name = "other"), peers).size)
        assertTrue(RouteRules.tailnetCandidates(known.copy(name = "other", serverId = null), peers).isEmpty())
    }

    @Test
    fun peersAreReadFromTheNodesAnswer() {
        val json = """[{"host_name":"nas","dns_name":"nas.tail0.ts.net.","ips":["fd7a:115c:a1e0::9","100.64.0.9"],"online":true,"tagged":false}]"""
        val p = RouteRules.parsePeers(json).single()
        assertEquals("100.64.0.9", p.ip)
        assertTrue(RouteRules.parsePeers("not json").isEmpty())
    }

    @Test
    fun theProblemSaysWhatToDo() {
        val off = TailscaleState.Off
        assertNull(RouteRules.problem(Route.Lan, true, true, off))
        assertNull(RouteRules.problem(Route.Checking, true, true, off))
        assertEquals(ConnectionProblem.Offline, RouteRules.problem(Route.Unreachable, false, true, off))
        assertEquals(ConnectionProblem.ServerDown, RouteRules.problem(Route.Unreachable, true, false, off))
        assertEquals(ConnectionProblem.TailscaleOff, RouteRules.problem(Route.Unreachable, true, true, off))
        assertEquals(ConnectionProblem.TailscaleStarting, RouteRules.problem(Route.Unreachable, true, true, TailscaleState.Starting))
        assertEquals(ConnectionProblem.TailscaleNeedsLogin("u"), RouteRules.problem(Route.Unreachable, true, true, TailscaleState.NeedsLogin("u")))
        assertEquals(ConnectionProblem.ServerDown, RouteRules.problem(Route.Unreachable, true, true, TailscaleState.Running("100.64.0.5")))
        assertEquals("Can't Reach nas", RouteRules.title(ConnectionProblem.TailscaleOff, "nas"))
        // A server whose name this device hasn't learned yet.
        assertEquals("Can't Reach Your Server", RouteRules.title(ConnectionProblem.TailscaleOff, null))
        assertTrue(RouteRules.message(ConnectionProblem.TailscaleOff, null, false, true, false).contains("reaches your server on"))
    }

    @Test
    fun theNodesStateIsRead() {
        assertEquals(TailscaleState.NeedsLogin(null), TailscaleState.of("needslogin", ""))
        assertEquals(TailscaleState.NeedsLogin("https://login.example/a"), TailscaleState.of("needslogin", "https://login.example/a"))
        assertEquals(TailscaleState.Running("100.64.0.5"), TailscaleState.of("running", "100.64.0.5"))
        assertEquals(TailscaleState.Starting, TailscaleState.of("starting", ""))
        assertTrue(TailscaleState.of("failed", "") is TailscaleState.Failed)
    }

    @Test
    fun theNodeIsNudgedOnlyWhenTheNetworkReallyChanged() {
        val wifi = NetPath(true, listOf("wlan0"), listOf("192.168.1.1"))
        val cell = NetPath(true, listOf("rmnet0"), emptyList())
        assertFalse(RouteRules.shouldNudge(null, wifi))
        assertFalse(RouteRules.shouldNudge(wifi, wifi))
        assertTrue(RouteRules.shouldNudge(wifi, cell))
        assertFalse(RouteRules.shouldNudge(wifi, NetPath(false, emptyList(), emptyList())))
    }

    @Test
    fun aRequestWaitsForTheRouteAndGoesTheWayItFound() {
        val server = okhttp3.mockwebserver.MockWebServer()
        server.enqueue(okhttp3.mockwebserver.MockResponse().setBody("""{"server_id":"id-one","name":"nas","version":"1"}"""))
        server.start()
        try {
            // Signed in at home (an address that doesn't answer here), the route found is the tailnet's forwarder.
            val api = MoonriseApi("http://127.0.0.1:9/".toHttpUrl(), "t0k", "dev", "Test")
            var waited = false
            api.awaitRoute = { waited = true; api.baseUrl = server.url("/") }
            val info = runBlocking { api.info() }
            assertTrue(waited)
            assertEquals("id-one", info.serverId)
            assertEquals("/v1/info", server.takeRequest().path)
        } finally {
            server.shutdown()
        }
    }

    @Test
    fun aNudgeDuringACheckIsKeptForAfterIt() = runBlocking {
        val signal = RecheckSignal()
        // The check is running (nobody waits on the signal) when the network changes.
        signal.nudge()
        signal.nudge()
        assertTrue(signal.await(10))
        // Two nudges made one more check, not two.
        assertFalse(signal.await(10))
    }

    // On Android too, a stream follows the route it can use.

    @Test
    fun aHomeStreamMovesAtOnceWhenTheRouteGoesToTheTailnet() {
        assertTrue(StreamRouteRules.shouldMove(Route.Lan, Route.Tailnet("100.64.0.9"), stalledFor = 0.0, routeSettledFor = 1.5, sinceLastMove = 60.0))
    }

    @Test
    fun aTailnetStreamStaysUntilItStalls() {
        assertFalse(StreamRouteRules.shouldMove(Route.Tailnet("100.64.0.9"), Route.Lan, 0.0, 10.0, 60.0))
        assertTrue(StreamRouteRules.shouldMove(Route.Tailnet("100.64.0.9"), Route.Lan, 2.5, 10.0, 60.0))
    }

    @Test
    fun aStreamDoesntMoveToAnUnsettledOrUnusableRouteOrTooOften() {
        assertFalse(StreamRouteRules.shouldMove(Route.Lan, Route.Tailnet("h"), 0.0, routeSettledFor = 0.5, sinceLastMove = 60.0))
        assertFalse(StreamRouteRules.shouldMove(Route.Lan, Route.Tailnet("h"), 0.0, 10.0, sinceLastMove = 2.0))
        assertFalse(StreamRouteRules.shouldMove(Route.Lan, Route.Unreachable, 10.0, 10.0, 60.0))
        assertFalse(StreamRouteRules.shouldMove(Route.Lan, Route.Checking, 10.0, 10.0, 60.0))
        assertFalse(StreamRouteRules.shouldMove(Route.Lan, Route.Lan, 10.0, 10.0, 60.0))
        assertFalse(StreamRouteRules.shouldMove(null, Route.Lan, 10.0, 10.0, 60.0))
    }

    @Test
    fun aStallAsksTheRouterToLookNotTooOften() {
        assertFalse(StreamRouteRules.shouldAskForRoute(2.0, 60.0))
        assertTrue(StreamRouteRules.shouldAskForRoute(3.0, 60.0))
        assertFalse(StreamRouteRules.shouldAskForRoute(10.0, 1.0))
    }
}
