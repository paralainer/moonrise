package app.moonrise.core

import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Subtitle files, timing labels, track names and the subtitle calls, as the Apple apps have them. */
class SubtitlesTest {
    @Test
    fun srtAndVttAreParsedWithTagsStripped() {
        val srt = "1\r\n00:00:01,000 --> 00:00:02,500\r\n<i>Hello</i> &amp; bye\r\n\r\n2\r\n00:00:02,000 --> 00:00:04,000 X:1\r\n{\\an8}Second\r\nline\r\n\r\n3\r\n00:00:05,000 --> 00:00:06,000\r\n\r\n"
        val doc = SubtitleParser.parse(srt)
        assertEquals(2, doc.cues.size)
        assertEquals("Hello & bye", doc.textAt(1.2))
        assertEquals("Hello & bye\nSecond\nline", doc.textAt(2.2)) // overlapping cues, in order
        assertNull(doc.textAt(0.5))
        assertNull(doc.textAt(4.5))
        assertEquals(2.0, doc.nextEdge(1.2)!!, 0.0)
        assertEquals(2.5, doc.nextEdge(2.2)!!, 0.0)
        val vtt = SubtitleParser.parse("WEBVTT\n\n01:02.500 --> 01:04.000 line:90%\nVTT cue\n")
        assertEquals(62.5, vtt.cues.single().start, 0.0)
        assertEquals(1.0 * 3600 + 2 * 60 + 3.456, SubtitleParser.timestamp("01:02:03,456")!!, 1e-9)
    }

    @Test
    fun assDialogueIsRead() {
        val ass = "[Script Info]\nTitle: x\n\n[Events]\nFormat: Layer, Start, End, Style, Text\nDialogue: 0,0:00:01.00,0:00:03.00,Default,{\\b1}Hi,\\Nthere\n"
        val doc = SubtitleParser.parse(ass, "ass")
        assertEquals("Hi,\nthere", doc.textAt(2.0))
    }

    @Test
    fun windows1252FilesAreDecoded() {
        val bytes = byteArrayOf(0x43, 0x61, 0x66, 0xE9.toByte()) // "Café" in Windows-1252
        assertEquals("Café", SubtitleParser.decode(bytes))
        assertEquals("Ünï", SubtitleParser.decode("﻿Ünï".toByteArray()))
    }

    @Test
    fun timingLabelsAsTheMenusSayThem() {
        assertEquals("Synced (+1.5 s)", SubtitleTiming(state = "synced", offset = 1.5).shortLabel)
        assertEquals("Synced automatically (+1.5 s, 2 parts)", SubtitleTiming(state = "synced", offset = 1.5, pieces = 2).label)
        assertEquals("Synced automatically (speed adjusted)", SubtitleTiming(state = "synced", scale = 1.04).label)
        assertEquals("Syncing…", SubtitleTiming(state = "pending").shortLabel)
        assertEquals("Not synced", SubtitleTiming(state = "failed").shortLabel)
        assertEquals("Original timing", SubtitleTiming(state = "synced", original = true).shortLabel)
        assertNull(SubtitleTiming(state = "none").shortLabel)
    }

    @Test
    fun trackNamesKeepOnlyWhatTheMenuDoesntSay() {
        val audio = Track(index = 1, kind = "Audio", codec = "ac3", language = "rus", title = "DD 5.1 @ 448 kbps (MVO NovGoblin)", channels = 6)
        assertEquals("Russian", audio.audioName)
        assertEquals("MVO NovGoblin · Dolby Digital 5.1", audio.audioDetail)
        val en = Track(index = 2, kind = "Audio", codec = "ac3", language = "eng", title = "DD 5.1 @ 448 kbps (English)", channels = 6)
        assertEquals("Dolby Digital 5.1", en.audioDetail)
        val sub = Track(index = 3, kind = "Subtitle", codec = "subrip", language = "rus", title = "SRT (NovGoblin)", isForced = true)
        assertEquals("Russian · NovGoblin · Forced", sub.subtitleLabel)
        assertEquals("English", Track(index = 4, kind = "Subtitle", language = "eng", title = "SRT (English)").subtitleLabel)
        assertEquals("Track 5", Track(index = 5, kind = "Subtitle").subtitleLabel)
    }

    @Test
    fun preferredAudioSkipsCommentary() {
        val tracks = listOf(
            AudioCandidate(1, "rus", true, "MVO"),
            AudioCandidate(2, "eng", false, "Commentary by the director"),
            AudioCandidate(3, "eng", false, "English"),
        )
        assertEquals(3, TrackRules.preferredAudio(tracks, "en"))
        assertNull(TrackRules.preferredAudio(tracks, ""))
        assertNull(TrackRules.preferredAudio(tracks, "fr"))
        assertTrue(TrackRules.isSDH("English (SDH)"))
        assertTrue(TrackRules.isForced("English · Forced"))
    }

    @Test
    fun subtitleCallsGoToTheServersRoutes() {
        val server = MockWebServer()
        server.enqueue(MockResponse().setBody("""{"subtitles":[{"index":4,"name":"SRT (English) - SUBRIP","language":"en","text":true,"source":"embedded",
            "url":"/v1/items/e1~f1/subtitles/4.srt","timing":{"state":"none","offset":0,"scale":1,"pieces":1,"nudge":0.5,"original":false}},
            {"index":10001,"name":"Release","language":"en","text":true,"source":"opensubtitles","url":"/x.srt"}]}"""))
        server.enqueue(MockResponse().setBody("""{"state":"synced","offset":1.2,"scale":1,"pieces":1,"nudge":0.3,"original":false}"""))
        server.enqueue(MockResponse().setBody("""{"results":[{"id":"9","name":"Rel+1","language":"en","downloads":12,"hash_match":true,"hearing_impaired":true,"machine_translated":false,"fetched":true}],
            "remaining_downloads":3,"quota":{"tier":"anonymous","allowed":5,"remaining":3,"account_allowed":20}}"""))
        server.enqueue(MockResponse().setBody("""{"index":10002,"name":"Rel+1","language":"en","text":true,"source":"opensubtitles","url":"/y.srt"}"""))
        server.enqueue(MockResponse().setBody("""{"window":2,"from":60,"to":90,"complete":false,"cues":[{"start":61,"end":62,"text":"<i>Hi</i>"}]}"""))
        server.enqueue(MockResponse().setBody("1\n00:00:01,000 --> 00:00:02,000\nHi\n"))
        server.start()
        try {
            val api = MoonriseApi(server.url("/"), token = "t0k", deviceId = "d", deviceName = "n")
            runBlocking {
                val media = api.mediaId("e1", "f1")
                val subs = api.subtitles(media)
                assertTrue(subs[0].embedded)
                assertTrue(subs[1].downloaded)
                assertEquals(0.5, subs[0].timing!!.nudge, 0.0)
                assertEquals(0.3, api.setSubtitleTiming(media, 4, nudge = 0.3).nudge, 0.0)
                val found = api.searchSubtitles(media, "en", "Sopranos+")
                assertEquals("EN · 12 downloads · SDH · Saved", found.results.single().detail)
                assertEquals("3 of 5 downloads left today · Add an OpenSubtitles account for 20 a day", found.quota!!.summary)
                assertEquals(10002, api.fetchSubtitle(media, found.results.single()).index)
                assertEquals(1, api.subtitleWindow(media, 4, 2).cues.size)
                assertEquals("Hi", SubtitleParser.parse(api.fetchText(api.resolve("/v1/items/e1~f1/subtitles/4.srt")!!)).textAt(1.5))
            }
            val list = server.takeRequest()
            assertEquals("/v1/items/e1~f1/subtitles", java.net.URLDecoder.decode(list.requestUrl!!.encodedPath, "UTF-8"))
            assertEquals("Bearer t0k", list.getHeader("Authorization"))
            val put = server.takeRequest()
            assertEquals("PUT", put.method)
            assertTrue(put.path!!.endsWith("/subtitles/4/timing"))
            assertEquals("0.3", MoonriseApi.json.parseToJsonElement(put.body.readUtf8()).jsonObject["nudge"]!!.jsonPrimitive.content)
            val search = server.takeRequest().requestUrl!!
            assertEquals("en", search.queryParameter("lang"))
            assertEquals("Sopranos+", search.queryParameter("q")) // "+" kept, not read as a space
            val fetch = server.takeRequest()
            val body = MoonriseApi.json.parseToJsonElement(fetch.body.readUtf8()).jsonObject
            assertEquals("9", body["result_id"]!!.jsonPrimitive.content)
            assertEquals("true", body["hearing_impaired"]!!.jsonPrimitive.content)
            assertEquals("2", server.takeRequest().requestUrl!!.queryParameter("window"))
            assertEquals("Bearer t0k", server.takeRequest().getHeader("Authorization"))
        } finally {
            server.shutdown()
        }
    }

    @Test
    fun playbackAsksForBurnIn() {
        val server = MockWebServer()
        server.enqueue(MockResponse().setBody("""{"supported":true,"master":"/v1/s/abc/master.m3u8","session":"abc"}"""))
        server.start()
        try {
            val api = MoonriseApi(server.url("/"), token = "t", deviceId = "d", deviceName = "n")
            runBlocking { api.playback("m1", start = 30.0, hdr = false, burn = 5) }
            val u = server.takeRequest().requestUrl!!
            assertEquals("5", u.queryParameter("burn"))
            assertEquals("0", u.queryParameter("hdr"))
        } finally {
            server.shutdown()
        }
    }
}
