package app.moonrise.core

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

// How the player's menus name a file's tracks (`TrackInfo.label`, `audioName`, `audioDetail` in
// `clients/apple/Moonrise/Server/MediaModels.swift`): the language on top, the meaningful part of the track's
// title and its sound underneath.

private val audioTechnical = """dd|ddp|dd\+|ac-?3|e-?ac-?3|dts(-hd)?|ma|truehd|atmos|aac|lc|flac|opus|mp3|pcm|lpcm|stereo|mono|surround|kbps|kb/s|\d+(\.\d)?(ch)?"""
private val subtitleTechnical = """srt|subrip|ass|ssa|vtt|webvtt|pgs|pgssub|sup|vobsub|dvdsub|dvb_subtitle|mov_text|utf-?8|text|full"""

/** The language's name ("Russian"), or null for none or "und". */
fun languageName(code: String?, locale: java.util.Locale = java.util.Locale.getDefault()): String? {
    val c = code?.trim()?.takeIf { it.isNotEmpty() && it != "und" } ?: return null
    val n = Languages.normalized(c) ?: c
    return java.util.Locale(n).getDisplayLanguage(locale).takeIf { it.isNotEmpty() && !it.equals(n, ignoreCase = true) } ?: c.uppercase()
}

private val Track.langName: String? get() = languageName(language)

/**
 * The meaningful part of the track's title. Titles are often technical ("DD 5.1 @ 448 kbps
 * (AVO Goblin)", "SRT (English)"), which the menu already says, so only what's left is kept.
 */
private fun Track.cleanTitle(words: String): String? {
    var t = title?.trim()?.takeIf { it.isNotEmpty() } ?: return null
    val technical = Regex("""^(?i)((\b($words)\b)|[@+\-–,/.\s])+$""")
    fun isTechnical(x: String) = technical.matches(x)
    val open = t.indexOf('(')
    val close = t.lastIndexOf(')')
    if (open >= 0 && close > open && (t.substring(0, open).isBlank() || isTechnical(t.substring(0, open)))) {
        t = t.substring(open + 1, close).trim()
    }
    if (t.isEmpty() || isTechnical(t)) return null
    if (langName?.equals(t, ignoreCase = true) == true) return null
    if (language?.equals(t, ignoreCase = true) == true) return null
    return t
}

/** A subtitle menu row's text: "Russian · NovGoblin · Forced". */
val Track.subtitleLabel: String
    get() {
        val lang = langName
        val parts = mutableListOf(lang ?: title ?: "Track $index")
        if (lang != null) cleanTitle(subtitleTechnical)?.let(parts::add)
        if (isForced == true) parts += "Forced"
        if (isExternal == true) parts += "External"
        return parts.joinToString(" · ")
    }

/** The audio menu's title: the language. */
val Track.audioName: String get() = langName ?: title ?: displayTitle ?: "Track $index"

/** The audio menu's detail: the track's name (a dub studio, commentary), then "Dolby Digital 5.1". */
val Track.audioDetail: String?
    get() {
        val format = when (val c = codec?.lowercase()) {
            null -> null
            "ac3" -> "Dolby Digital"
            "eac3" -> if (profile?.contains("Atmos") == true) "Dolby Atmos" else "Dolby Digital Plus"
            "truehd" -> if (profile?.contains("Atmos") == true) "Dolby Atmos" else "Dolby TrueHD"
            "dts" -> if (profile?.contains("MA") == true) "DTS-HD MA" else "DTS"
            "aac" -> "AAC"
            "flac" -> "FLAC"
            "opus" -> "Opus"
            else -> c.uppercase()
        }
        val layout = when (val ch = channels) {
            null -> null
            1 -> "Mono"
            2 -> "Stereo"
            else -> if (ch > 2) "${ch - 1}.1" else null
        }
        val sound = listOfNotNull(format, layout).joinToString(" ")
        val parts = listOfNotNull(cleanTitle(audioTechnical), sound.ifEmpty { null })
        return parts.joinToString(" · ").ifEmpty { null }
    }

val Track.isAudio: Boolean get() = kind.equals("audio", true)
val Track.isSubtitle: Boolean get() = kind.equals("subtitle", true)

/** An audio track in the preferred language, as the Apple apps pick it (`PlayerModel.preferredAudio`). */
data class AudioCandidate(val id: Int, val language: String?, val isDefault: Boolean, val name: String)

object TrackRules {
    /**
     * The track in the preferred audio language (its default one, else not a commentary), or null:
     * the title's own default plays.
     */
    fun preferredAudio(tracks: List<AudioCandidate>, preferred: String?): Int? {
        val want = Languages.normalized(preferred) ?: return null
        val matching = tracks.filter { Languages.normalized(it.language) == want }
        val main = matching.filter { !Regex("(?i)comment|descri|narrat|director").containsMatchIn(it.name) }
        return (main.firstOrNull { it.isDefault } ?: main.firstOrNull() ?: matching.firstOrNull())?.id
    }

    fun isForced(title: String): Boolean = title.lowercase().contains("forced")
    fun isSDH(title: String): Boolean = Regex("""(?i)\b(sdh|cc|hearing)\b""").containsMatchIn(title)

    /** A subtitle track's language; for a file without one, the code in its name ("Film.en.srt"). */
    fun subtitleLanguage(language: String?, title: String): String? {
        Languages.normalized(language)?.let { return it }
        return title.lowercase().split(".").dropLast(1).reversed().firstOrNull { it.length == 2 || it.length == 3 }?.let { Languages.normalized(it) }
    }

    /** A subtitle row's words past its language ("SDH", "Forced"), as `TrackRules.subtitleDetail` on Apple. */
    fun subtitleDetail(title: String): String? =
        title.split(" · ").drop(1).joinToString(" · ").trim().ifEmpty { null }

    /**
     * Menu rows (title, detail) that would read alike, say two English tracks with no title, get
     * their place among those twins ("#2"), as `PlayerModel.distinctDetails` on Apple.
     */
    fun distinctDetails(rows: List<Pair<String, String?>>): List<String?> = rows.indices.map { i ->
        val twins = rows.indices.filter { rows[it] == rows[i] }
        if (twins.size > 1) listOfNotNull(rows[i].second, "#${twins.indexOf(i) + 1}").joinToString(" · ") else rows[i].second
    }

    /**
     * A remembered subtitle's title. The menus used to add " (burned in)" to a bitmap track, and a
     * choice kept from then may still say it: without those words it matches today's names.
     */
    fun rememberedSubtitleTitle(title: String?): String? =
        title?.replace(" (burned in)", "")?.trim()?.ifEmpty { null }

    // ---------------------------------------------------------------- track memory

    private val extraAudio = Regex("(?i)comment|descri|narrat|director")

    private fun sameText(want: String?, have: String?): Boolean {
        val a = want?.trim()?.lowercase()?.takeIf { it.isNotEmpty() } ?: return false
        return have?.lowercase()?.contains(a) == true
    }

    /**
     * The audio track picked last, in `tracks` (as `TrackRules.rememberedAudio` on Apple): in the
     * same file its index (when its language agrees); else (another file, the next episode) the
     * same language, the same title first, and a commentary or description only if that is what
     * was picked. Null: none like it, the usual rules pick.
     */
    fun rememberedAudio(tracks: List<AudioCandidate>, memory: TrackMemory?): Int? {
        val want = memory?.audio ?: return null
        val lang = Languages.normalized(want.language)
        if (memory.sameFile && want.index != null) {
            val t = tracks.firstOrNull { it.id == want.index }
            if (t != null && (lang == null || Languages.normalized(t.language) == lang)) return t.id
        }
        val same = tracks.filter { Languages.normalized(it.language) == lang }
        if (lang == null && (same.isEmpty() || want.title == null)) return null
        same.firstOrNull { sameText(want.title, it.name) }?.let { return it.id }
        if (lang == null) return null
        val extra = want.title?.let { extraAudio.containsMatchIn(it) } ?: false
        val kind = same.filter { extraAudio.containsMatchIn(it.name) == extra }
        return (kind.firstOrNull { it.isDefault } ?: kind.firstOrNull())?.id
    }

    /**
     * The subtitles picked last, among `options` (as `TrackRules.rememberedSubtitle` on Apple): in
     * the same file the track with its index (when its language agrees); else one of the same
     * language and kind (forced or full), the same SDH and title first, text over bitmap.
     */
    fun rememberedSubtitle(options: List<SubtitleCandidate>, memory: TrackMemory?): SubtitleMemory {
        memory ?: return SubtitleMemory.Usual
        if (memory.subtitleOff) return if (memory.sameFile) SubtitleMemory.Off else SubtitleMemory.UsualOff
        val want = memory.subtitle ?: return SubtitleMemory.Usual
        val lang = Languages.normalized(want.language)
        if (memory.sameFile && want.index != null) {
            val o = options.firstOrNull { it.trackIndex == want.index }
            if (o != null && (lang == null || subtitleLanguage(o.language, o.title) == lang)) return SubtitleMemory.Track(o.id)
        }
        val forced = want.forced ?: false
        val pool = options.filter { subtitleLanguage(it.language, it.title) == lang && isForced(it.title) == forced }
        val title = rememberedSubtitleTitle(want.title)
        if (lang == null && (pool.isEmpty() || title == null)) return SubtitleMemory.Usual
        val sdh = want.sdh ?: false
        val text = pool.filter { !it.burnIn }
        val ordered = if (text.isEmpty()) pool else text + pool.filter { it.burnIn }
        val found = ordered.firstOrNull { isSDH(it.title) == sdh && sameText(title, it.title) }
            ?: ordered.firstOrNull { sameText(title, it.title) && lang != null }
            ?: ordered.firstOrNull { isSDH(it.title) == sdh && lang != null }
            ?: (if (lang != null) ordered.firstOrNull() else null)
        return found?.let { SubtitleMemory.Track(it.id) } ?: SubtitleMemory.Usual
    }
}

/** A subtitle choice as [TrackRules.rememberedSubtitle] sees it. */
data class SubtitleCandidate(val id: String, val title: String, val language: String?, val trackIndex: Int?, val burnIn: Boolean)

/** What the remembered subtitles say for a title starting. */
sealed interface SubtitleMemory {
    /** This option. */
    data class Track(val id: String) : SubtitleMemory
    /** Off, as picked in this file: no subtitles at all. */
    data object Off : SubtitleMemory
    /** Off in another file (the next episode): the usual rules with subtitles off (forced ones still come on). */
    data object UsualOff : SubtitleMemory
    /** Nothing remembered, or nothing like it here: the usual rules. */
    data object Usual : SubtitleMemory
}

/** One track as the server keeps it (`TrackPick` in api/openapi.yaml): any field may be missing. */
@Serializable
data class TrackPick(
    val index: Int? = null,
    val language: String? = null,
    val title: String? = null,
    val forced: Boolean? = null,
    val sdh: Boolean? = null,
) {
    fun json(): JsonObject = JsonObject(buildMap {
        index?.let { put("index", JsonPrimitive(it)) }
        language?.let { put("language", JsonPrimitive(it)) }
        title?.let { put("title", JsonPrimitive(it)) }
        forced?.let { put("forced", JsonPrimitive(it)) }
        sdh?.let { put("sdh", JsonPrimitive(it)) }
    })
}

/**
 * The tracks the user picked last (the playback answer's `tracks`). `sameFile`: picked in this
 * very file, so an index can be trusted (when its language agrees).
 */
@Serializable
data class TrackMemory(
    @SerialName("same_file") val sameFile: Boolean = false,
    val audio: TrackPick? = null,
    val subtitle: TrackPick? = null,
    @SerialName("subtitle_off") val subtitleOff: Boolean = false,
)

/** The tracks playing, as a report sends them (`tracks`): a null subtitle is Off. */
@Serializable
data class TrackChoice(val audio: TrackPick? = null, val subtitle: TrackPick? = null) {
    fun toJson(): String = MoonriseApi.json.encodeToString(serializer(), this)

    companion object {
        fun fromJson(s: String): TrackChoice? = runCatching { MoonriseApi.json.decodeFromString(serializer(), s) }.getOrNull()
    }

    /** As the player's memory: this very file's choice (a download's own). */
    val memory: TrackMemory get() = TrackMemory(sameFile = true, audio = audio, subtitle = subtitle, subtitleOff = subtitle == null)

    fun json(): JsonObject = JsonObject(buildMap<String, JsonElement> {
        audio?.let { put("audio", it.json()) }
        put("subtitle", subtitle?.json() ?: JsonNull)
    })
}
