package app.moonrise.core

import kotlinx.coroutines.CoroutineExceptionHandler
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import okhttp3.mockwebserver.Dispatcher
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okhttp3.mockwebserver.RecordedRequest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlin.concurrent.thread

/** A download deleted while something is still being fetched for it leaves no folder. */
class DownloadDeleteTest {
    @get:Rule val tmp = TemporaryFolder()

    /** A server that runs `during` while it answers: the moment a fetch is in the air. */
    private fun fetching(code: Int = 200, during: () -> Unit = {}, block: suspend (MoonriseApi) -> Unit) {
        val server = MockWebServer()
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse {
                during()
                return MockResponse().setResponseCode(code).setBody("segment")
            }
        }
        server.start()
        try {
            runBlocking { block(MoonriseApi(server.url("/"), token = "t0k", deviceId = "dev", deviceName = "Test")) }
        } finally {
            server.shutdown()
        }
    }

    private fun folder() = tmp.newFolder("download").also { File(it, "hls/video").mkdirs() }

    private suspend fun MoonriseApi.segment(folder: File, kept: () -> Boolean) =
        DownloadRules.writingInto(folder, kept) { downloadFile("/set", "video/0.m4s", null, File(folder, "hls/video/0.m4s")) }

    @Test
    fun aDeleteDuringTheFetchLeavesNoFolder() {
        val folder = folder()
        val gone = AtomicBoolean(false)
        fetching(during = { gone.set(true); folder.deleteRecursively() }) { api ->
            api.segment(folder) { !gone.get() }
        }
        assertFalse(folder.exists())
    }

    @Test
    fun aFetchThatFailsAfterTheDeleteLeavesNoFolder() {
        val folder = folder()
        val gone = AtomicBoolean(false)
        fetching(code = 500, during = { gone.set(true); folder.deleteRecursively(); folder.mkdirs() }) { api ->
            runCatching { api.segment(folder) { !gone.get() } }
        }
        assertFalse(folder.exists())
    }

    @Test
    fun aDownloadStillThereKeepsItsFile() {
        val folder = folder()
        fetching { api -> api.segment(folder) { true } }
        assertEquals("segment", File(folder, "hls/video/0.m4s").readText())
    }

    /** The delete's own walk runs while writers are at work: a file written behind it would keep the folder alive. */
    @Test
    fun aDeleteRacingTheWritersLeavesNoFolder() {
        repeat(300) { round ->
            val folder = tmp.newFolder("race$round")
            val gone = AtomicBoolean(false)
            val start = CountDownLatch(1)
            val writers = List(4) { w ->
                thread {
                    start.await()
                    DownloadRules.writingInto(folder, { !gone.get() }) {
                        val dir = File(folder, "hls/video").apply { mkdirs() }
                        repeat(5) { n -> runCatching { File(dir, "$w-$n.m4s").writeText("x") } }
                    }
                }
            }
            val deleter = thread {
                start.await()
                gone.set(true)
                folder.deleteRecursively()
            }
            start.countDown()
            (writers + deleter).forEach { it.join() }
            assertFalse("round $round", folder.exists())
        }
    }

    /** The artwork job runs in a scope with nothing to catch an error, so one from here would end the app. */
    private fun savingArtwork(api: MoonriseApi, folder: File, kept: () -> Boolean): Pair<Boolean, Throwable?> {
        val uncaught = AtomicReference<Throwable?>(null)
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO + CoroutineExceptionHandler { _, e -> uncaught.set(e) })
        var saved = false
        runBlocking {
            scope.launch {
                DownloadRules.writingInto(folder, kept) {
                    saved = DownloadRules.saveArtwork(File(folder, "poster.jpg")) { api.fetchBytes(api.imageUrl("poster", 500)) }
                }
            }.join()
        }
        return saved to uncaught.get()
    }

    @Test
    fun artworkArrivingAfterTheDeleteEndsQuietly() {
        val folder = folder()
        val gone = AtomicBoolean(false)
        fetching(during = { gone.set(true); folder.deleteRecursively() }) { api ->
            val (saved, uncaught) = savingArtwork(api, folder) { !gone.get() }
            assertFalse(saved)
            assertNull(uncaught)
        }
        assertFalse(folder.exists())
    }

    @Test
    fun artworkTheServerDoesNotHaveWritesNothing() {
        val folder = folder()
        fetching(code = 404) { api ->
            val (saved, uncaught) = savingArtwork(api, folder) { true }
            assertFalse(saved)
            assertNull(uncaught)
        }
        assertFalse(File(folder, "poster.jpg").exists())
    }

    @Test
    fun artworkOfADownloadStillThereIsKept() {
        val folder = folder()
        fetching { api ->
            val (saved, uncaught) = savingArtwork(api, folder) { true }
            assertTrue(saved)
            assertNull(uncaught)
        }
        assertEquals("segment", File(folder, "poster.jpg").readText())
    }

    /** `Downloads` can't run in a JVM test: its artwork goes through the rule above, and nothing there writes by itself. */
    @Test
    fun artworkIsOnlyWrittenThroughTheRule() {
        val source = File("../shared/src/main/kotlin/app/moonrise/shared/downloads/Downloads.kt").readText()
        val artwork = source.substringAfter("private suspend fun fetchArtworkAndSegments").substringBefore("private val refreshing")
        assertFalse(artwork.contains("writeBytes"))
        assertEquals(2, artwork.split("DownloadRules.saveArtwork(").size - 1)
    }

    /** The order `writingInto` counts on, and what goes through it. */
    @Test
    fun deletingDropsTheRecordFirstAndEveryWriterCleansUp() {
        val source = File("../shared/src/main/kotlin/app/moonrise/shared/downloads/Downloads.kt").readText()
        val delete = source.substringAfter("fun delete(id: String)").substringBefore("private fun stopJob")
        assertTrue(delete.indexOf("records = records.filter") in 0 until delete.indexOf("deleteRecursively"))
        assertTrue(source.contains("intoFolder(rec.id) { api.downloadFile("))
        assertTrue(source.contains("intoFolder(next.id) { run(next.id) }"))
        assertTrue(source.contains("intoFolder(rec.id) { fetchArtworkAndSegments(rec.id, api); fetchSubtitles(rec.id, api) }"))
    }
}
