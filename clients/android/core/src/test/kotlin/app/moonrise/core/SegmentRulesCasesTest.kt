package app.moonrise.core

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.double
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * The segment rules' shared cases (`api/segment-rules.json` at the repository's root): the server
 * applies the same rules for `segments?merged=1` (`server/src/media/segment_rules.rs`), the Apple
 * app has the same test (`SegmentRulesCasesTests`). Gradle passes the file (`moonrise.segment.rules`).
 */
class SegmentRulesCasesTest {
    private fun segments(a: JsonArray): List<Segment> = a.map { e ->
        val o = e.jsonObject
        val source = o["source"]?.jsonPrimitive?.content
        // As the client decodes the server's answer: only `detected` is kept.
        Segment(SegmentKind.of(o["kind"]!!.jsonPrimitive.content)!!, o["start"]!!.jsonPrimitive.double, o["end"]!!.jsonPrimitive.double,
            if (source == "detected") "detected" else null)
    }

    @Test
    fun theSharedCases() {
        val path = System.getProperty("moonrise.segment.rules") ?: error("moonrise.segment.rules isn't set (core/build.gradle.kts)")
        val cases = (MoonriseApi.json.parseToJsonElement(File(path).readText()) as JsonObject)["cases"]!!.jsonArray
        assertTrue(cases.size >= 10)
        for (c in cases.map { it.jsonObject }) {
            val name = c["name"]!!.jsonPrimitive.content
            val got = SegmentRules.merge(
                server = segments(c["server"]!!.jsonArray),
                online = c["online"]!!.jsonArray.map { segments(it.jsonArray) },
                runtime = c["runtime"]!!.jsonPrimitive.double,
                pending = c["pending"]?.jsonPrimitive?.boolean ?: false,
                movie = c["movie"]?.jsonPrimitive?.boolean ?: false,
                communityPending = c["community_pending"]?.jsonPrimitive?.boolean ?: false,
            )
            assertEquals(name, segments(c["expected"]!!.jsonArray).map { Triple(it.kind, it.start, it.end) }, got.map { Triple(it.kind, it.start, it.end) })
        }
    }

    /**
     * An answer that isn't final is asked for again on entering the last 10 minutes and 2
     * minutes before the end, once each; jumping straight to the last 2 minutes asks once.
     */
    @Test
    fun askedAgainNearTheEnd() {
        val d = 7268.64
        assertEquals(0, SegmentService.nearEndAsksDue(d, 0.0))
        assertEquals(0, SegmentService.nearEndAsksDue(d, d - 601))
        assertEquals(1, SegmentService.nearEndAsksDue(d, d - 600))
        assertEquals(1, SegmentService.nearEndAsksDue(d, d - 121))
        assertEquals(2, SegmentService.nearEndAsksDue(d, d - 100))
        assertEquals(0, SegmentService.nearEndAsksDue(0.0, 100.0))
    }
}
