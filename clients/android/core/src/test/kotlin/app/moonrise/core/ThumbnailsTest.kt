package app.moonrise.core

import kotlinx.coroutines.runBlocking
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ThumbnailsTest {
    private val t = Thumbnails("ready", interval = 10.0, width = 320, height = 180, columns = 10, rows = 10, count = 250,
        sheets = listOf("/a/0.jpg", "/a/1.jpg", "/a/2.jpg"))

    @Test
    fun picturesAreFoundOnTheirSheets() {
        assertEquals(Thumbnails.Tile(0, 0, 0), t.tile(0.0))
        assertEquals(Thumbnails.Tile(0, 0, 0), t.tile(9.9))
        assertEquals(Thumbnails.Tile(0, 320, 0), t.tile(10.0))
        // Picture 23: row 2, column 3.
        assertEquals(Thumbnails.Tile(0, 960, 360), t.tile(235.0))
        // Picture 123: the second sheet.
        assertEquals(Thumbnails.Tile(1, 960, 360), t.tile(1234.0))
        // Past the end: the last picture (249); before the start: the first.
        assertEquals(Thumbnails.Tile(2, 9 * 320, 4 * 180), t.tile(99_999.0))
        assertEquals(Thumbnails.Tile(0, 0, 0), t.tile(-5.0))
    }

    @Test
    fun onlyReadyAnswersHavePictures() {
        assertFalse(Thumbnails("pending").ready)
        assertNull(Thumbnails("pending").tile(10.0))
        assertNull(t.copy(sheets = emptyList()).tile(10.0))
        assertTrue(t.ready)
    }

    @Test
    fun asksForTheSessionsFile() {
        val server = MockWebServer()
        server.enqueue(MockResponse().setBody("""{"state":"ready","interval":10.0,"width":320,"height":134,"columns":10,"rows":10,"count":3,"sheets":["/v1/trickplay/0123456789abcdef/0.jpg"]}"""))
        server.start()
        try {
            val api = MoonriseApi(server.url("/"), token = "t0k", deviceId = "dev", deviceName = "Test TV")
            val th = runBlocking { api.thumbnails("item~v2", "sess1") }
            assertTrue(th.ready)
            assertEquals(134, th.height)
            val req = server.takeRequest()
            assertEquals("/v1/items/item%7Ev2/trickplay?sid=sess1", req.path)
            assertEquals("Bearer t0k", req.getHeader("Authorization"))
        } finally {
            server.shutdown()
        }
    }
}
