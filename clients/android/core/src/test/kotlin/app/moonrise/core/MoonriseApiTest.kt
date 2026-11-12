package app.moonrise.core

import kotlinx.coroutines.runBlocking
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class MoonriseApiTest {
    @Test
    fun normalizesAddresses() {
        assertEquals("http://192.168.1.10:8097/", MoonriseApi.normalize("192.168.1.10")?.toString())
        assertEquals("http://192.168.1.10:9000/", MoonriseApi.normalize(" 192.168.1.10:9000/ ")?.toString())
        assertEquals("http://media.local:8097/", MoonriseApi.normalize("http://media.local/")?.toString())
        assertEquals("https://example.org/", MoonriseApi.normalize("https://example.org")?.toString())
        assertNull(MoonriseApi.normalize("  "))
    }

    private fun withServer(vararg bodies: String, block: suspend (MoonriseApi, MockWebServer) -> Unit) {
        val server = MockWebServer()
        bodies.forEach { server.enqueue(MockResponse().setBody(it).setHeader("Content-Type", "application/json")) }
        server.start()
        try {
            val api = MoonriseApi(server.url("/"), token = "t0k", deviceId = "dev", deviceName = "Test TV")
            runBlocking { block(api, server) }
        } finally {
            server.shutdown()
        }
    }

    @Test
    fun decodesHomeAndSendsToken() = withServer(
        """{"resume":[{"id":"e1","kind":"episode","title":"Pilot","series_title":"Show","season_no":1,"episode_no":1,
           "runtime":1000,"user":{"position":250.0,"played":false},"images":{"thumb":"aaaaaaaaaaaaaaaaaaaaaaaa"},
           "parent_images":{"poster":"bbbbbbbbbbbbbbbbbbbbbbbb","backdrop":["cccccccccccccccccccccccc"]}}],
           "next_up":[],"latest_types":[{"type":"movie","title":"Movies","items":[{"id":"m1","kind":"movie","title":"Film","year":2024}]}],
           "latest":{},"extra":1}""",
    ) { api, server ->
        val home = api.home()
        assertEquals(1, home.resume.size)
        val e = home.resume[0]
        assertEquals("S1 · E1", e.episodeLabel)
        assertEquals("Show", e.cardTitle)
        assertEquals("S1 · E1 · Pilot", e.cardSubtitle)
        val film = home.latestTypes[0].items[0]
        assertEquals("Film", film.cardTitle)
        assertEquals("2024", film.cardSubtitle)
        assertEquals(0.25, e.progress!!, 1e-9)
        assertEquals("aaaaaaaaaaaaaaaaaaaaaaaa", e.cardImage)
        assertEquals("bbbbbbbbbbbbbbbbbbbbbbbb", e.posterImage)
        assertEquals("Movies", home.latestTypes[0].title)
        val req = server.takeRequest()
        assertEquals("Bearer t0k", req.getHeader("Authorization"))
        assertTrue(req.path!!.startsWith("/v1/home"))
    }

    /** The Watch Next sync asks only for the watching rows; Home still asks for everything. */
    @Test
    fun theWatchNextSyncAsksOnlyForWatchingRows() = withServer("{}", "{}") { api, server ->
        api.homeWatching()
        val sync = server.takeRequest().requestUrl!!
        assertEquals("/v1/home", sync.encodedPath)
        assertEquals("1", sync.queryParameter("watching"))
        assertEquals("${WatchNextRules.MAX}", sync.queryParameter("limit"))
        api.home()
        assertEquals(null, server.takeRequest().requestUrl!!.queryParameter("watching"))
    }

    /** Fix Match's three answers are asked for with candidates' posters on the server. */
    @Test
    fun fixMatchAsksForServerPosters() = withServer(
        """{"items":[]}""",
        """{"id":"a","kind":"movie","title":"A","candidates":[]}""",
        """{"items":[]}""",
    ) { api, server ->
        api.reviewList()
        api.matchInfo("a", compute = true)
        api.searchMatches("movie", "a", null)
        repeat(3) { assertEquals("1", server.takeRequest().requestUrl!!.queryParameter("server_posters")) }
    }


    @Test
    fun decodesContinueWatching() = withServer(
        """{"continue":[{"id":"e2","kind":"episode","title":"Two","series_id":"s1","user":{"position":0.0,"played":false},
           "continue":{"reason":"next_episode","at":"2026-11-07T11:00:00Z","from":"e1"}}],
           "resume":[],"next_up":[],"latest_types":[]}""",
    ) { api, _ ->
        val home = api.home()
        val c = home.continueRow.single()
        assertEquals("e2", c.id)
        assertEquals("next_episode", c.continueInfo?.reason)
        assertEquals("e1", c.continueInfo?.from)
    }

    @Test
    fun playbackAndMasterUrl() = withServer(
        """{"supported":true,"reasons":[],"master":"/v1/s/abc/master.m3u8","method":"copy","session":"abc",
           "video":{"codec":"h264","range":"SDR"},"audio":[{"index":1,"name":"English","language":"en","default":true,"converted":false}],
           "subtitles":[],"media_source_id":"f1","duration":5400.5}""",
    ) { api, server ->
        val p = api.playback("m1", start = 120.0, hdr = false)
        assertEquals("abc", p.session)
        assertEquals(server.url("/v1/s/abc/master.m3u8").toString(), api.masterUrl(p))
        val path = server.takeRequest().path!!
        assertTrue(path, path.contains("hdr=0") && path.contains("start=120.000") && !path.contains("lossless") && !path.contains("dv="))
    }

    @Test
    fun playbackAsksForLosslessOnlyWhenTold() = withServer(
        """{"supported":true,"reasons":[],"master":"/v1/s/abc/master.m3u8","method":"convert","session":"abc","audio":[],"subtitles":[]}""",
    ) { api, server ->
        api.playback("m1", hdr = true, lossless = true)
        val path = server.takeRequest().path!!
        assertTrue(path, path.contains("hdr=1") && path.contains("lossless=1"))
    }

    @Test
    fun losslessNeedsAMultichannelHdmiOutput() {
        // Only an HDMI output that says it takes more than 2 channels.
        assertTrue(ClientRules.wantsLossless(listOf(intArrayOf(2, 6, 8))))
        assertTrue(ClientRules.wantsLossless(listOf(intArrayOf(2), intArrayOf(6))))
        assertTrue(!ClientRules.wantsLossless(listOf(intArrayOf(2))))
        assertTrue("an output that doesn't say isn't assumed", !ClientRules.wantsLossless(listOf(intArrayOf())))
        assertTrue(!ClientRules.wantsLossless(emptyList()))
    }

    @Test
    fun playbackAsksForDolbyVisionOnlyWhenTold() = withServer(
        """{"supported":true,"reasons":[],"master":"/v1/s/abc/master.m3u8","method":"copy","session":"abc","audio":[],"subtitles":[]}""",
    ) { api, server ->
        api.playback("m1", hdr = true, dolbyVision = true)
        val path = server.takeRequest().path!!
        assertTrue(path, path.contains("hdr=1") && path.contains("dv=1"))
    }

    @Test
    fun playbackNeverAsksForDolbyVisionWithoutHdr() = withServer(
        """{"supported":true,"reasons":[],"master":"/v1/s/abc/master.m3u8","method":"copy","session":"abc","audio":[],"subtitles":[]}""",
    ) { api, server ->
        api.playback("m1", hdr = false, dolbyVision = true)
        val path = server.takeRequest().path!!
        assertTrue(path, path.contains("hdr=0") && !path.contains("dv="))
    }

    @Test
    fun dolbyVisionNeedsTheDisplayAndADecoder() {
        // HDR on, Dolby Vision among the display's HDR types (1), and a video/dolby-vision decoder.
        val dv = ClientRules.HDR_TYPE_DOLBY_VISION
        assertTrue(ClientRules.wantsDolbyVision(hdrDisplay = true, displayHdrTypes = intArrayOf(2, dv, 3), dolbyVisionDecoder = true))
        assertTrue("HDR10 and HLG only", !ClientRules.wantsDolbyVision(true, intArrayOf(2, 3), dolbyVisionDecoder = true))
        assertTrue("no decoder", !ClientRules.wantsDolbyVision(true, intArrayOf(dv, 2), dolbyVisionDecoder = false))
        assertTrue("HDR off", !ClientRules.wantsDolbyVision(false, intArrayOf(dv), dolbyVisionDecoder = true))
        assertTrue("no HDR types", !ClientRules.wantsDolbyVision(true, intArrayOf(), dolbyVisionDecoder = true))
    }

    @Test
    fun deviceCodePoll() = withServer(
        """{"status":"pending"}""",
        """{"status":"approved","token":"new","user":{"id":"u1","name":"viewer"}}""",
    ) { api, server ->
        api.token = null
        assertEquals(DevicePoll.Pending, api.pollDeviceCode("dc"))
        val r = api.pollDeviceCode("dc")
        assertTrue(r is DevicePoll.Approved)
        assertEquals("new", api.token)
        assertNull(server.takeRequest().getHeader("Authorization"))
    }

    @Test
    fun cardLabelsAsOnAppleTv() {
        val double = Item(id = "e", kind = "episode", title = "Two", seriesTitle = "Show", seasonNo = 3, episodeNo = 12, episodeEnd = 13)
        assertEquals("S3 · E12–13 · Two", double.cardSubtitle)
        val part = Item(id = "e", kind = "episode", title = "End", seriesTitle = "Show", seasonNo = 5, episodeNo = 23, part = 2)
        assertEquals("S5 · E23 · Part 2 · End", part.cardSubtitle)
        val season = Item(id = "s", kind = "season", title = "Season 2", seriesTitle = "Show")
        assertEquals("Show", season.cardTitle)
        assertEquals("Season 2", season.cardSubtitle)
        val noYear = Item(id = "m", kind = "movie", title = "Film")
        assertEquals(" ", noYear.cardSubtitle)
        assertEquals(null, noYear.episodeLabel)
    }

    @Test
    fun setsUpANewServer() {
        val server = MockWebServer()
        server.enqueue(MockResponse().setResponseCode(403).setBody("Not the code."))
        server.enqueue(MockResponse().setResponseCode(204))
        server.enqueue(MockResponse().setBody("""{"token":"moonrise_new","user":{"id":"u1","name":"admin","is_admin":true}}""").setHeader("Content-Type", "application/json"))
        server.start()
        try {
            val api = MoonriseApi(server.url("/"), token = null, deviceId = "dev", deviceName = "Phone")
            runBlocking {
                assertEquals(false, api.verifySetupCode("000 000"))
                assertEquals(true, api.verifySetupCode("123 456"))
                val r = api.claim("123456", "admin", "secret")
                assertEquals("moonrise_new", r.token)
                assertEquals("moonrise_new", api.token)
            }
            server.takeRequest()
            val verify = server.takeRequest()
            assertEquals("/v1/setup/verify", verify.path)
            assertTrue(verify.body.readUtf8().contains("\"code\":\"123 456\""))
            val claim = server.takeRequest()
            assertEquals("/v1/setup/claim", claim.path)
            assertNull(claim.getHeader("Authorization"))
            val body = claim.body.readUtf8()
            assertTrue(body.contains("\"username\":\"admin\"") && body.contains("\"device_id\":\"dev\""))
        } finally {
            server.shutdown()
        }
    }
}
