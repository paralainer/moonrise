package app.moonrise.core

import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * How a playback failure is named: the shared cases (`api/playback-problem-rules.json` at
 * the repository's root) that the Apple apps (`PlaybackProblemTests`) and the Windows app
 * (`SharedCasesTests`) run too, and the Android apps' own uses of the rule. Gradle passes the file
 * (`moonrise.playback.problem.rules`).
 */
class PlaybackProblemCasesTest {
    @Test
    fun theSharedCases() {
        val path = System.getProperty("moonrise.playback.problem.rules") ?: error("moonrise.playback.problem.rules isn't set (core/build.gradle.kts)")
        val cases = (MoonriseApi.json.parseToJsonElement(File(path).readText()) as JsonObject)["cases"]!!.jsonArray.map { it.jsonObject }
        assertTrue(cases.size >= 15)
        for (c in cases) {
            val status = c["status"]?.takeIf { it !is JsonNull }?.jsonPrimitive?.int
            val pending = c["pending"]?.jsonPrimitive?.boolean ?: false
            val p = PlaybackProblem.classify(status, c["text"]!!.jsonPrimitive.content, pending)
            assertEquals(c["name"]!!.jsonPrimitive.content, c["expected"]!!.jsonPrimitive.content, p.kind.key)
        }
        assertEquals(PlaybackProblem.Kind.entries.map { it.key }.toSet(), cases.map { it["expected"]!!.jsonPrimitive.content }.toSet())
    }

    @Test
    fun shortWordsWithTheServersTextAsDetails() {
        val raw = "probe: ffprobe: [matroska,webm @ 0x61e4] EBML header parsing failed\n/media/movies/Film.mkv: Invalid data found when processing input"
        val p = PlaybackProblem.of(MoonriseHttpException(422, "  $raw\n"))
        assertEquals("Damaged file", p.title)
        assertFalse(p.message.contains("ffprobe") || p.message.contains("/media/"))
        assertEquals(raw, p.detail)
        assertNull(PlaybackProblem.classify(502, " ").detail)
        assertEquals(PlaybackProblem.Kind.Unreachable, PlaybackProblem.of(java.net.ConnectException("Failed to connect")).kind)
        assertEquals(PlaybackProblem.Kind.Unindexed, PlaybackProblem.of(Playback(supported = false, reasons = listOf("no keyframe index: no Cues found"))).kind)
        assertEquals(PlaybackProblem.Kind.Preparing, PlaybackProblem.of(Playback(supported = false, pending = true, reasons = listOf("keyframe index pending (scanning the file)"))).kind)
        for (k in PlaybackProblem.Kind.entries) {
            val x = PlaybackProblem(k, null)
            assertTrue("$k title is short", x.title.length <= 24)
            assertTrue("$k line fits a small player", x.message.length <= 110)
        }
    }

    @Test
    fun aTitleNoFileOfWhichCanPlay() {
        val bad = MediaFile(id = "a", broken = true, brokenReason = "damaged or incomplete: truncated")
        val healthy = MediaFile(id = "b")
        val gone = MediaFile(id = "c", name = "Film (2017).mkv", missing = true)
        fun movie(vararg f: MediaFile) = Item(id = "m", kind = "movie", files = f.toList())

        assertEquals(PlaybackProblem.Kind.Damaged, movie(bad).unplayableProblem()?.kind)
        assertEquals("damaged or incomplete: truncated", movie(bad).unplayableProblem()?.detail)
        assertNull("a healthy version plays", movie(bad, healthy).unplayableProblem())
        assertNull(movie(healthy).unplayableProblem())
        assertNull("a list item without its files says nothing", Item(id = "m", kind = "movie").unplayableProblem())
        assertEquals(PlaybackProblem.Kind.Missing, movie(gone).unplayableProblem()?.kind)
        assertTrue(movie(gone).unplayableProblem()!!.detail!!.contains("Film (2017).mkv"))
        assertEquals("the file still there is damaged", PlaybackProblem.Kind.Damaged, movie(gone, bad).unplayableProblem()?.kind)
        assertNull(movie(gone, healthy).unplayableProblem())
        assertEquals(PlaybackProblem.Kind.Damaged, PlaybackProblem.broken(null).kind)
        assertEquals(PlaybackProblem.Kind.Unindexed, PlaybackProblem.broken("no keyframe index: no Cues found").kind)
    }
}
