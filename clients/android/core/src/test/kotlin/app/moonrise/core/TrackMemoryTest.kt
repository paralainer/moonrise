package app.moonrise.core

import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Track memory: resuming a title keeps the audio and subtitle tracks picked in it, and
 * the next episode gets the same language and kind. The same cases as the Apple apps'
 * TrackRulesTests (`resumeKeepsTheAudioTrack` … `nextEpisodeSubtitles`).
 */
class TrackMemoryTest {
    private fun audio(id: Int, language: String?, name: String = "", isDefault: Boolean = false) = AudioCandidate(id, language, isDefault, name)
    private fun memory(same: Boolean, audio: TrackPick? = null, subtitle: TrackPick? = null, off: Boolean = false) =
        TrackMemory(sameFile = same, audio = audio, subtitle = subtitle, subtitleOff = off)
    private fun server(index: Int, title: String, language: String?) = SubtitleCandidate("t:$index", title, language, index, burnIn = false)
    private fun bitmap(id: String, title: String, language: String?) = SubtitleCandidate(id, title, language, null, burnIn = true)
    private fun remembered(options: List<SubtitleCandidate>, m: TrackMemory?): String = when (val r = TrackRules.rememberedSubtitle(options, m)) {
        is SubtitleMemory.Track -> r.id
        SubtitleMemory.Off -> "off"
        SubtitleMemory.UsualOff -> "usual-off"
        SubtitleMemory.Usual -> "usual"
    }

    @Test
    fun resumeKeepsTheAudioTrack() {
        val tracks = listOf(audio(1, "eng", isDefault = true), audio(2, "rus", "Dub"), audio(3, "rus", "Studio B"), audio(4, null, "Track 4"))
        assertEquals(3, TrackRules.rememberedAudio(tracks, memory(true, audio = TrackPick(index = 3, language = "ru", title = "Studio B"))))
        assertEquals("no language", 4, TrackRules.rememberedAudio(tracks, memory(true, audio = TrackPick(index = 4))))
        assertNull("the index now has another language", TrackRules.rememberedAudio(tracks, memory(true, audio = TrackPick(index = 2, language = "de"))))
        assertEquals("the index gone: by title", 3, TrackRules.rememberedAudio(tracks, memory(true, audio = TrackPick(index = 9, language = "ru", title = "Studio B"))))
        assertNull(TrackRules.rememberedAudio(tracks, null))
        assertNull("no audio said", TrackRules.rememberedAudio(tracks, memory(true, subtitle = TrackPick(index = 1))))
    }

    @Test
    fun nextEpisodeAudio() {
        val next = listOf(audio(5, "eng", isDefault = true), audio(6, "rus", "Commentary"), audio(7, "rus", "Studio B"), audio(8, "rus", "Dub"))
        assertEquals("by title, not the index", 8, TrackRules.rememberedAudio(next, memory(false, audio = TrackPick(index = 5, language = "ru", title = "Dub"))))
        assertEquals("not its commentary", 7, TrackRules.rememberedAudio(next, memory(false, audio = TrackPick(language = "rus", title = "Other studio"))))
        assertEquals(6, TrackRules.rememberedAudio(next, memory(false, audio = TrackPick(language = "ru", title = "Director commentary"))))
        assertNull("not there", TrackRules.rememberedAudio(next, memory(false, audio = TrackPick(language = "ja"))))
        assertNull("nothing to match by", TrackRules.rememberedAudio(next, memory(false, audio = TrackPick(index = 5))))
    }

    @Test
    fun resumeKeepsTheSubtitles() {
        val options = listOf(
            server(3, "English", "eng"), server(4, "English · SDH", "eng"), server(5, "English · Forced", "eng"),
            SubtitleCandidate("moonrise:20", "Russian · OpenSubtitles", "rus", 20, burnIn = false),
        )
        assertEquals("t:4", remembered(options, memory(true, subtitle = TrackPick(index = 4, language = "en", title = "SDH", sdh = true))))
        assertEquals("forced, by hand", "t:5", remembered(options, memory(true, subtitle = TrackPick(index = 5, language = "en", forced = true))))
        assertEquals("a fetched track", "moonrise:20", remembered(options, memory(true, subtitle = TrackPick(index = 20, language = "ru"))))
        assertEquals("off", remembered(options, memory(true, off = true)))
        assertEquals("another language now", "usual", remembered(options, memory(true, subtitle = TrackPick(index = 3, language = "de"))))
        assertEquals("subtitles not said", "usual", remembered(options, memory(true, audio = TrackPick(index = 1))))
        assertEquals("usual", remembered(options, null))
    }

    @Test
    fun nextEpisodeSubtitles() {
        val next = listOf(
            bitmap("pgs", "English", "eng"), server(7, "English · Forced", "eng"), server(8, "English", "eng"),
            server(9, "English · SDH", "eng"), server(10, "Russian", "rus"),
        )
        assertEquals("t:9", remembered(next, memory(false, subtitle = TrackPick(index = 4, language = "en", title = "SDH", sdh = true))))
        assertEquals("the plain one, not the index", "t:8", remembered(next, memory(false, subtitle = TrackPick(index = 9, language = "en", sdh = false))))
        assertEquals("t:7", remembered(next, memory(false, subtitle = TrackPick(language = "en", forced = true))))
        assertEquals("t:10", remembered(next, memory(false, subtitle = TrackPick(language = "ru"))))
        assertEquals("not there", "usual", remembered(next, memory(false, subtitle = TrackPick(language = "ja"))))
        assertEquals("usual-off", remembered(next, memory(false, off = true)))
        assertEquals("a bitmap track when there's no text one", "pgs",
            remembered(listOf(bitmap("pgs", "English", "eng")), memory(false, subtitle = TrackPick(language = "en"))))
    }

    @Test
    fun reportBody() {
        val body = TrackChoice(TrackPick(index = 2, language = "en", title = "Main"), null).json()
        assertEquals(JsonNull, body["subtitle"])
        assertEquals(2, body["audio"]!!.jsonObject["index"]!!.jsonPrimitive.int)
        assertEquals(setOf("index", "language", "title"), body["audio"]!!.jsonObject.keys)
        assertEquals("SDH", TrackRules.subtitleDetail("English · SDH"))
        assertNull(TrackRules.subtitleDetail("English"))
    }

    /** No app's menus add " (burned in)" to a bitmap track's name (PlayerModel, TV, phone). */
    @Test
    fun noMenuSaysBurnedIn() {
        val sources = listOf("shared", "tv", "phone").flatMap { app ->
            java.io.File("../$app/src/main/kotlin").walkTopDown().filter { it.extension == "kt" }.toList()
        }
        assertTrue("the apps' sources are where this test expects", sources.size > 20)
        assertEquals(emptyList<String>(), sources.filter { "(burned in)" in it.readText() }.map { it.name })
    }

    /** Without the words, a bitmap track and a text one of the same name still read differently. */
    @Test
    fun rowsThatReadAlikeGetTheirPlace() {
        val rows = listOf("English" to null, "English" to null, "English" to "SDH", "Russian" to null, "Russian" to "SDH", "Russian" to "SDH")
        assertEquals(listOf("#1", "#2", "SDH", null, "SDH · #1", "SDH · #2"), TrackRules.distinctDetails(rows))
    }

    /** A choice remembered while the menus said " (burned in)" still finds its track. */
    @Test
    fun aChoiceFromBeforeTheLabelWentStillMatches() {
        val next = listOf(server(8, "English", "eng"), bitmap("pgs", "English · Signs", "eng"))
        assertEquals("pgs", remembered(next, memory(false, subtitle = TrackPick(language = "en", title = "Signs (burned in)"))))
        assertEquals("pgs", remembered(next, memory(false, subtitle = TrackPick(language = "en", title = "English · Signs (burned in)"))))
        assertEquals("Signs", TrackRules.rememberedSubtitleTitle("Signs (burned in)"))
        assertNull(TrackRules.rememberedSubtitleTitle(" (burned in)"))
        assertNull(TrackRules.rememberedSubtitleTitle(null))
    }
}
