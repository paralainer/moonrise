package app.moonrise.core

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

// Subtitles as the Apple apps show them (`clients/apple/Moonrise/Subtitles/`): the server's text tracks are
// fetched as files and drawn by the app (so their delay can be changed), picture tracks (PGS, DVD,
// DVB) come as timed images in the same windows and are drawn over the video, and tracks can be
// found on OpenSubtitles through the server.

data class SubtitleCue(val start: Double, val end: Double, val text: String)

/** A parsed track: the cues sorted by start. */
class SubtitleDocument(val cues: List<SubtitleCue>) {
    /** The text showing at `t` (overlapping cues joined, in order), or null. */
    fun textAt(t: Double): String? {
        var lo = 0
        var hi = cues.size - 1
        var found = -1
        while (lo <= hi) {
            val mid = (lo + hi) / 2
            if (cues[mid].start <= t) { found = mid; lo = mid + 1 } else hi = mid - 1
        }
        if (found < 0) return null
        // Overlapping cues: walk back a little to collect everything active.
        val lines = ArrayDeque<String>()
        var i = found
        while (i >= 0 && i >= found - 4) {
            val c = cues[i]
            if (c.start <= t && t < c.end) lines.addFirst(c.text)
            i--
        }
        return if (lines.isEmpty()) null else lines.joinToString("\n")
    }

    /** The next time after `t` where the visible text changes (a cue starts or ends). */
    fun nextEdge(after: Double): Double? {
        var lo = 0
        var hi = cues.size
        while (lo < hi) {
            val mid = (lo + hi) / 2
            if (cues[mid].start <= after) lo = mid + 1 else hi = mid
        }
        var edge: Double? = if (lo < cues.size) cues[lo].start else null
        for (c in cues.subList(maxOf(0, lo - 5), lo)) if (c.end > after) edge = minOf(edge ?: c.end, c.end)
        return edge
    }
}

object SubtitleParser {
    fun parse(text: String, ext: String? = null): SubtitleDocument {
        val lower = ext?.lowercase()
        if (lower == "ass" || lower == "ssa" || text.contains("[Script Info]")) return SubtitleDocument(parseAss(text))
        return SubtitleDocument(parseSrtOrVtt(text))
    }

    /** UTF-8, else Windows-1252 (older SRT files). */
    fun decode(bytes: ByteArray): String {
        val decoder = Charsets.UTF_8.newDecoder()
        return runCatching { decoder.decode(java.nio.ByteBuffer.wrap(bytes)).toString() }
            .getOrElse { String(bytes, charset("windows-1252")) }
            .removePrefix("﻿")
    }

    /** SRT and WebVTT share the "start --> end" block structure. */
    fun parseSrtOrVtt(raw: String): List<SubtitleCue> {
        val normalized = raw.replace("\r\n", "\n").replace("\r", "\n")
        val cues = mutableListOf<SubtitleCue>()
        for (block in normalized.split("\n\n")) {
            val lines = block.split("\n")
            val ti = lines.indexOfFirst { it.contains("-->") }
            if (ti < 0) continue
            val parts = lines[ti].split("-->")
            if (parts.size != 2) continue
            val s = timestamp(parts[0]) ?: continue
            val e = timestamp(parts[1].trim().split(" ")[0]) ?: continue
            val body = lines.drop(ti + 1).joinToString("\n").trim()
            if (body.isEmpty()) continue
            cues += SubtitleCue(s, e, stripTags(body))
        }
        return cues.sortedBy { it.start }
    }

    /** Accepts "01:02:03,456", "01:02:03.456" and "02:03.456". */
    fun timestamp(s: String): Double? {
        val t = s.trim().replace(',', '.')
        val comps = t.split(":")
        if (comps.size < 2) return null
        var total = 0.0
        for (c in comps) {
            val v = c.toDoubleOrNull() ?: return null
            total = total * 60 + v
        }
        return total
    }

    fun stripTags(s: String): String =
        s.replace(Regex("<[^>]+>"), "").replace(Regex("\\{[^}]*\\}"), "")
            .replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ")

    /** Minimal Advanced SubStation parser: dialogue text only, styling dropped. */
    fun parseAss(raw: String): List<SubtitleCue> {
        var format = emptyList<String>()
        val cues = mutableListOf<SubtitleCue>()
        var inEvents = false
        for (line in raw.lines()) {
            val l = line.trim()
            if (l.startsWith("[")) { inEvents = l.lowercase() == "[events]"; continue }
            if (!inEvents) continue
            if (l.startsWith("Format:")) {
                format = l.drop(7).split(",").map { it.trim().lowercase() }
            } else if (l.startsWith("Dialogue:") && format.isNotEmpty()) {
                val fields = l.drop(9).split(",", limit = format.size)
                if (fields.size != format.size) continue
                val si = format.indexOf("start")
                val ei = format.indexOf("end")
                val ti = format.indexOf("text")
                if (si < 0 || ei < 0 || ti < 0) continue
                val s = timestamp(fields[si]) ?: continue
                val e = timestamp(fields[ei]) ?: continue
                val text = stripTags(fields[ti]).replace("\\N", "\n").replace("\\n", "\n").replace("\\h", " ").trim()
                if (text.isNotEmpty()) cues += SubtitleCue(s, e, text)
            }
        }
        return cues.sortedBy { it.start }
    }
}

/**
 * A text track's timing (`GET|PUT /v1/items/{id}/subtitles/{index}/timing`): the server's match of
 * a downloaded (fetched or uploaded) track to this file, and the viewer's nudge, kept per file and
 * track for every device.
 */
@Serializable
data class SubtitleTiming(
    /** "none", "pending", "synced", "in_sync", "unsure" or "failed". */
    val state: String = "none",
    val offset: Double = 0.0,
    val scale: Double = 1.0,
    val pieces: Int = 1,
    val nudge: Double = 0.0,
    /** The viewer chose the original timing over the match. */
    val original: Boolean = false,
) {
    val pending: Boolean get() = state == "pending"
    val synced: Boolean get() = state == "synced"

    /** What the menu says about it. */
    val label: String?
        get() {
            if (original) return if (synced) "Original timing" else null
            return when (state) {
                "pending" -> "Syncing…"
                "synced" -> {
                    val parts = mutableListOf<String>()
                    if (scale != 1.0) parts += "speed adjusted"
                    if (pieces > 1) parts += "$pieces parts"
                    if (scale == 1.0) parts.add(0, "%+.1f s".format(java.util.Locale.ROOT, offset))
                    "Synced automatically (${parts.joinToString(", ")})"
                }
                "in_sync" -> "In sync with this file"
                "unsure", "failed" -> "Couldn't sync automatically"
                else -> null
            }
        }

    /** The same for a narrow row (a TV menu's second line). */
    val shortLabel: String?
        get() {
            if (original) return label
            return when (state) {
                "synced" -> if (scale == 1.0) "Synced (%+.1f s)".format(java.util.Locale.ROOT, offset) else "Synced (speed)"
                "in_sync" -> "In sync"
                "unsure", "failed" -> "Not synced"
                else -> label
            }
        }
}

/** A subtitle track of a title (`GET /v1/items/{id}/subtitles`). */
@Serializable
data class SubtitleTrack(
    /** The server's track index; tracks fetched from OpenSubtitles have their own (10000+). */
    val index: Int,
    val name: String = "",
    val language: String? = null,
    val text: Boolean = true,
    val external: Boolean? = null,
    val forced: Boolean? = null,
    /** "embedded", "sidecar", "opensubtitles" or "upload". */
    val source: String? = null,
    /** Text tracks: the SRT, relative to the server. */
    val url: String? = null,
    val timing: SubtitleTiming? = null,
) {
    /** A track the server takes out of the video (served in windows until it's all out). */
    val embedded: Boolean get() = text && (source == "embedded" || (source == null && external != true))
    /** Fetched or uploaded: the server matches it to this file. */
    val downloaded: Boolean get() = text && (source == "opensubtitles" || source == "upload")
}

@Serializable
internal data class SubtitleList(val subtitles: List<SubtitleTrack> = emptyList())

/**
 * A window of an embedded track (`/subtitles/{index}/cues?window=k`): the cues of 30 s of the file.
 * A picture track's window has a `canvas` and its cues an `image` each (and no text).
 */
@Serializable
data class SubtitleWindow(
    val window: Int,
    val from: Double = 0.0,
    val to: Double = 0.0,
    val complete: Boolean = false,
    val source: String? = null,
    val canvas: Canvas? = null,
    val cues: List<Cue> = emptyList(),
) {
    @Serializable
    data class Cue(val start: Double, val end: Double, val text: String = "", val image: Image? = null)

    /** Where on the canvas, and the PNG (base64). */
    @Serializable
    data class Image(val x: Int, val y: Int, val width: Int, val height: Int, val png: String)

    /** The frame a picture track's images are placed on (the video's frame they were made for). */
    @Serializable
    data class Canvas(val width: Int, val height: Int)

    /** The picture cues of this window (none for a text track). */
    val pictures: List<PictureCue>
        get() = cues.mapNotNull { c -> c.image?.let { PictureCue(c.start, c.end, it.x, it.y, it.width, it.height, it.png) } }

    companion object {
        const val SPAN = 30.0
    }
}

/** An OpenSubtitles result from the server's search. */
@Serializable
data class SubtitleResult(
    val id: String,
    val name: String,
    val language: String? = null,
    val downloads: Int = 0,
    @SerialName("hash_match") val hashMatch: Boolean = false,
    @SerialName("hearing_impaired") val hearingImpaired: Boolean = false,
    @SerialName("machine_translated") val machineTranslated: Boolean? = null,
    @SerialName("ai_translated") val aiTranslated: Boolean? = null,
    /** Already fetched for this title. */
    val fetched: Boolean? = null,
) {
    /** "EN · 1234 downloads · SDH · Saved" */
    val detail: String
        get() {
            val parts = mutableListOf(language?.uppercase() ?: "?", "$downloads downloads")
            if (hearingImpaired) parts += "SDH"
            if (machineTranslated == true) parts += "Machine translated"
            if (fetched == true) parts += "Saved"
            return parts.joinToString(" · ")
        }
}

/** The OpenSubtitles download quota the server fetches with. */
@Serializable
data class SubtitleQuota(
    /** "anonymous", "user" or "vip". */
    val tier: String = "",
    val allowed: Int? = null,
    val remaining: Int? = null,
    @SerialName("reset_at") val resetAt: String? = null,
    @SerialName("account_allowed") val accountAllowed: Int? = null,
) {
    val exhausted: Boolean get() = remaining == 0

    private val leftLine: String?
        get() {
            val r = remaining ?: return null
            val n = if (r == 1) "1 download" else "$r downloads"
            val base = allowed?.let { "$r of $it downloads left today" } ?: "$n left today"
            return if (tier == "vip") "VIP · $base" else base
        }

    private val hint: String? get() = if (tier == "anonymous") "Add an OpenSubtitles account for ${accountAllowed ?: 20} a day" else null

    /** One line for the search screen. */
    val summary: String?
        get() {
            if (exhausted) return listOfNotNull("No downloads left today", hint).joinToString(" · ")
            return listOfNotNull(leftLine, hint).joinToString(" · ").ifEmpty { null }
        }
}

@Serializable
data class SubtitleSearch(
    val results: List<SubtitleResult> = emptyList(),
    @SerialName("remaining_downloads") val remainingDownloads: Int? = null,
    val quota: SubtitleQuota? = null,
)

/** One image of a picture track, shown from `start` to `end`, at (x, y) on the track's canvas; `png` is base64. */
data class PictureCue(val start: Double, val end: Double, val x: Int, val y: Int, val width: Int, val height: Int, val png: String) {
    /** The same picture in two windows is one: by its time and place. */
    val key: String get() = "$start|$end|$x|$y"
}

/** A rectangle in view pixels. */
data class PictureRect(val x: Float, val y: Float, val width: Float, val height: Float)

/**
 * A picture track as far as its windows have come: the cues by key (a picture in two windows
 * counts once), sorted by start, and which show at a time.
 */
class PictureCues {
    private val byKey = LinkedHashMap<String, PictureCue>()
    var canvas: SubtitleWindow.Canvas? = null
        private set
    var cues: List<PictureCue> = emptyList()
        private set

    /** Adds a window's pictures; answers the ones that are new. */
    fun add(w: SubtitleWindow): List<PictureCue> {
        w.canvas?.let { canvas = it }
        val new = w.pictures.filter { byKey.putIfAbsent(it.key, it) == null }
        if (new.isNotEmpty()) cues = byKey.values.sortedWith(compareBy({ it.start }, { it.end }))
        return new
    }

    /** The pictures showing at `t` (the track's time: the playhead minus the delay). */
    fun at(t: Double): List<PictureCue> = PictureSubtitles.at(cues, t)
}

/** How picture subtitles are timed and placed, as the other apps do. */
object PictureSubtitles {
    /** The pictures of `cues` (sorted by start) showing at `t`. */
    fun at(cues: List<PictureCue>, t: Double): List<PictureCue> = cues.filter { it.start <= t && t < it.end }

    /**
     * Where the video shows in a view of `viewW` × `viewH`: fitted (letter- or pillar-boxed), or
     * filling it (`fill`, cropped), for a picture of aspect `videoAspect` (width / height as shown).
     */
    fun videoRect(viewW: Float, viewH: Float, videoAspect: Float?, fill: Boolean = false): PictureRect {
        if (videoAspect == null || videoAspect <= 0f || viewW <= 0f || viewH <= 0f) return PictureRect(0f, 0f, viewW, viewH)
        val viewAspect = viewW / viewH
        val wide = videoAspect > viewAspect
        val (w, h) = if (wide != fill) viewW to viewW / videoAspect else viewH * videoAspect to viewH
        return PictureRect((viewW - w) / 2f, (viewH - h) / 2f, w, h)
    }

    /**
     * A picture's place in the view: the canvas stretched over the video's rectangle, across and
     * down separately (a DVD's 720 × 480 canvas covers a 16:9 picture).
     */
    fun place(cue: PictureCue, canvas: SubtitleWindow.Canvas, video: PictureRect): PictureRect {
        val sx = video.width / canvas.width.coerceAtLeast(1)
        val sy = video.height / canvas.height.coerceAtLeast(1)
        return PictureRect(video.x + cue.x * sx, video.y + cue.y * sy, cue.width * sx, cue.height * sy)
    }

    /**
     * A placed picture moved into the view when the video fills it (cropped at the edges): a line at
     * the bottom of a 2.39:1 picture filling a phone stays on screen, not cut off with the video.
     */
    /**
     * How far up a picture in the lower half moves while the controls show, so it stays clear of
     * them as text does (raised to `bandTop`, the bottom of raised text): 0 when it already is, or
     * the controls are hidden (as `SubtitlePictures.Lift` in the Windows app).
     */
    fun lift(top: Float, height: Float, viewH: Float, controlsShown: Boolean, bandTop: Float): Float {
        if (!controlsShown || bandTop <= 0f || top + height / 2f < viewH / 2f) return 0f
        return maxOf(0f, top + height - (bandTop - 8f))
    }

    fun keepInside(r: PictureRect, viewW: Float, viewH: Float): PictureRect {
        val x = if (r.width >= viewW) (viewW - r.width) / 2f else r.x.coerceIn(0f, viewW - r.width)
        val y = if (r.height >= viewH) (viewH - r.height) / 2f else r.y.coerceIn(0f, viewH - r.height)
        return PictureRect(x, y, r.width, r.height)
    }
}
