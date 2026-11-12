package app.moonrise.core

import kotlinx.serialization.json.JsonNull
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
 * The resume rule's shared cases (`api/resume-rules.json` at the repository's root): the Apple apps
 * (`ResumeRulesCasesTests`) and the Windows app (`SharedCasesTests`) run the same ones. Gradle
 * passes the file (`moonrise.resume.rules`).
 */
class ResumeRulesCasesTest {
    @Test
    fun theSharedCases() {
        val path = System.getProperty("moonrise.resume.rules") ?: error("moonrise.resume.rules isn't set (core/build.gradle.kts)")
        val cases = (MoonriseApi.json.parseToJsonElement(File(path).readText()) as JsonObject)["cases"]!!.jsonArray
        assertTrue(cases.size >= 10)
        for (c in cases.map { it.jsonObject }) {
            val u = c["user"]?.takeIf { it !is JsonNull }?.jsonObject
            val user = u?.let {
                UserData(
                    position = it["position"]?.jsonPrimitive?.double,
                    played = it["played"]?.jsonPrimitive?.boolean,
                )
            }
            val expected = c["expected"]?.takeIf { it !is JsonNull }?.jsonPrimitive?.double
            assertEquals(c["name"]!!.jsonPrimitive.content, expected, Item(id = "m", kind = "movie", user = user).resumeSeconds)
        }
    }
}
