// SPDX-License-Identifier: MPL-2.0
package app.moonrise.core

import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import java.time.Duration
import java.time.Instant
import java.time.OffsetDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale
import kotlin.math.abs

// What the Manage Server, setup and Fix Match screens say and decide, worded as the iPhone words it
// (`ManageServerViews.swift`, `ManageAccountsViews.swift`, `SetupViews.swift`, `FixMatchViews.swift`).

/** The text of a failed admin call (`serverErrorText`, iPhone): the server's plain reason, capitalised. */
fun adminErrorText(e: Throwable): String {
    if (e is MoonriseHttpException && e.body.isNotEmpty() && e.code < 500) return e.body.replaceFirstChar { it.uppercase() }
    if (e is java.io.IOException) return "Couldn't reach the server: ${e.message ?: "no answer"}"
    return e.message ?: "It didn't work."
}

/** Dates and sizes as Manage Server shows them (`AdminFmt`, iPhone). */
object AdminFmt {
    fun date(s: String?): Instant? {
        s ?: return null
        return runCatching { OffsetDateTime.parse(s).toInstant() }.getOrNull() ?: runCatching { Instant.parse(s) }.getOrNull()
    }

    fun ago(s: String?, now: Instant = Instant.now()): String? = date(s)?.let { ago(it, now) }

    /** "just now", "12 min. ago", "3 hr. ago", "2 days ago", "in 6 days". */
    fun ago(d: Instant, now: Instant = Instant.now()): String {
        // To the millisecond: seven days less a moment is 6 days, and only the very instant is 1 wk.
        val secs = Duration.between(d, now).toMillis() / 1000
        if (abs(secs) < 60) return "just now"
        val (n, unit) = when {
            abs(secs) < 3_600 -> abs(secs) / 60 to "min."
            abs(secs) < 86_400 -> abs(secs) / 3_600 to "hr."
            abs(secs) < 7 * 86_400 -> abs(secs) / 86_400 to "day"
            abs(secs) < 30 * 86_400 -> abs(secs) / (7 * 86_400) to "wk."
            abs(secs) < 365 * 86_400 -> abs(secs) / (30 * 86_400) to "mo."
            else -> abs(secs) / (365 * 86_400) to "yr."
        }
        val amount = if (unit == "day") "$n day${if (n == 1L) "" else "s"}" else "$n $unit"
        return if (secs >= 0) "$amount ago" else "in $amount"
    }

    fun uptime(secs: Int): String {
        val d = secs / 86_400; val h = secs % 86_400 / 3600; val m = secs % 3600 / 60
        return when {
            d > 0 -> "$d day${if (d == 1) "" else "s"}, $h h"
            h > 0 -> "$h h $m min"
            else -> "${maxOf(m, 1)} min"
        }
    }

    /** In binary units ("3 GB" for a 3 GiB limit); "None" for 0. */
    fun bytes(n: Long): String {
        if (n <= 0) return "None"
        val units = listOf("KB" to 0, "MB" to 1, "GB" to 2, "TB" to 2)
        if (n < 1024) return "$n bytes"
        var v = n.toDouble()
        var u = -1
        while (v >= 1024 && u < units.lastIndex) { v /= 1024; u++ }
        val (name, decimals) = units[u]
        val text = String.format(Locale.US, "%.${decimals}f", v).let { if ('.' in it) it.trimEnd('0').trimEnd('.') else it }
        return "$text $name"
    }

    /** "2,589 episodes", "1 show". */
    fun count(n: Int, one: String, many: String = one + "s"): String = String.format(Locale.US, "%,d %s", n, if (n == 1) one else many)
}

/** What a Manage Server row or footer says. */
object AdminRules {
    /** The library's passes in a line (`ScanWatcher.phaseText`). */
    fun scanPhaseText(p: ScanProgress?, now: Instant = Instant.now()): String {
        p ?: return "—"
        return when (p.phase) {
            "queued" -> "Starting a scan…"
            "scanning" -> if (p.walked > 0) "Looking for videos… ${p.walked} files" else "Looking for videos…"
            "identifying" -> if (p.identify.total > 0) "Recognizing titles… ${p.identify.done} of ${p.identify.total}" else "Recognizing titles…"
            "metadata" -> "Getting artwork and details…"
            "finishing" -> "Finishing…"
            else -> p.lastPass?.at?.let { at ->
                val d = Instant.ofEpochSecond(at)
                if (now.epochSecond - at < 60) "Up to date · scanned just now" else "Up to date · scanned ${AdminFmt.ago(d, now)}"
            } ?: "Up to date"
        }
    }

    /** What a library holds (libraries have no type): "300 movies", "42 shows · 2,589 episodes"; the files until something is recognized. */
    fun libraryCounts(l: ScanLibrary?): String? {
        l ?: return null
        val parts = mutableListOf<String>()
        if (l.movies > 0) parts += AdminFmt.count(l.movies, "movie")
        if (l.series > 0) parts += listOf(AdminFmt.count(l.series, "show"), AdminFmt.count(l.episodes, "episode"))
        return if (parts.isEmpty()) AdminFmt.count(l.files, "file") else parts.joined()
    }

    private fun List<String>.joined(): String = joinToString(" · ")

    /** Movies and shows with no sure match, and episodes, waiting in Fix Match. */
    fun waiting(l: ScanLibrary): Int = l.review + (l.reviewEpisodes ?: 0)

    /** Manage Server's detail on the Fix Match row. */
    fun waitingText(n: Int?): String? = n?.let { if (it == 0) "Nothing to fix" else "$it waiting" }

    /** "Offline: the reason", "Online · as demo". */
    fun rootStatus(r: ManagedRoot): String =
        (if (r.online) "Online" else "Offline" + (r.error?.let { ": $it" } ?: "")) + (r.username?.let { " · as $it" } ?: "")

    // Playback & Hardware

    /** A GPU or another video engine transcodes. */
    fun isHardware(transcoding: String): Boolean = transcoding !in listOf("cpu", "off", "unavailable")

    fun transcodingText(s: SystemInfo): String = when (s.transcoding) {
        "off" -> "Switched off"
        "unavailable" -> if (s.transcode == null) "No NVIDIA GPU" else "Not available"
        else -> s.transcode?.label ?: if (s.transcoding == "nvenc") "NVIDIA NVENC" else s.transcoding
    }

    /** The footer under Hardware (the iPhone's words, "your devices" for "Apple devices"). */
    fun hardwareFooter(s: SystemInfo): String = when (s.transcoding) {
        "nvenc", "qsv", "vaapi" -> "Lower qualities, codecs your devices can't play, HDR for SDR screens and burned-in subtitles are made on the GPU. Everything else is streamed as it is."
        "videotoolbox" -> "Lower qualities, codecs your devices can't play, HDR for SDR screens and burned-in subtitles are made by the Mac's media engine (VideoToolbox). Everything else is streamed as it is."
        "v4l2" -> "Lower qualities (up to 1080p), codecs your devices can't play and burned-in subtitles are made by the board's video encoder. 4K and 10-bit HEVC titles are too much for it: they play as they are. Everything else is streamed as it is."
        "cpu" -> "No GPU is passed to the server: transcodes run on the CPU (up to 1080p), which can be slow on a small machine. Everything your devices play directly is streamed as it is."
        "off" -> "Transcoding is switched off: titles stream as they are (audio is still converted when needed)."
        "unavailable" -> if (s.transcode == null) "Transcoding needs an NVIDIA GPU passed to the server (NVIDIA Container Toolkit). Without one, titles stream as they are: what your devices can play directly plays fine."
            else "Nothing on the server can transcode: titles stream as they are; what your devices can play directly plays fine."
        else -> "Lower qualities, codecs your devices can't play, HDR for SDR screens and burned-in subtitles are made by the server's video hardware. Everything else is streamed as it is."
    }

    val hardwareChoices = listOf("auto" to "Automatic", "nvidia" to "NVIDIA", "qsv" to "Intel Quick Sync", "vaapi" to "VA-API (Intel, AMD)", "cpu" to "CPU")

    /** The server's choices, else the Linux ones. */
    fun choices(t: SystemTranscode): List<Pair<String, String>> = t.choices?.takeIf { it.isNotEmpty() }?.map { it.key to it.label } ?: hardwareChoices

    /** "Automatic tries VideoToolbox, then the CPU." */
    fun automaticText(t: SystemTranscode): String {
        val names = choices(t).filter { it.first != "auto" }.map { if (it.first == "cpu") "the CPU" else it.second }
        if (names.size <= 1) return "Automatic finds what transcodes here."
        return "Automatic tries " + names.dropLast(1).joinToString(", then ") + ", else " + names.last() + "."
    }

    /** "Intel(R) Core(TM) i5-7500 CPU @ 3.40GHz" → "Intel Core i5-7500 CPU @ 3.40GHz". */
    fun cpuName(model: String?): String =
        if (model.isNullOrEmpty()) "CPU" else model.replace("(R)", "").replace("(TM)", "").replace("  ", " ")

    /** "120 of 281", "Paused while playing · 120 of 281", "All files ready". */
    fun preparingText(b: IndexBackfill?): String {
        b ?: return "…"
        val files = b.files ?: 0; val done = b.done ?: 0; val failed = b.failed ?: 0
        val failedText = if (failed > 0) " · $failed failed" else ""
        return when {
            b.state == "off" -> "Off"
            b.state == "starting" || b.state == null -> "Starting"
            b.state == "paused" && done + failed < files -> "Paused while playing · $done of $files"
            b.state == "running" && done + failed < files -> "$done of $files$failedText"
            else -> "All files ready$failedText"
        }
    }

    /** "120 of 281", "Paused while playing · 120 of 281", "Space full · 120 of 281", "All titles ready · 2 skipped". */
    fun thumbnailsText(t: TrickplayProgress?): String {
        t ?: return "…"
        val files = t.files ?: 0; val done = t.done ?: 0; val failed = t.failed ?: 0; val skipped = t.skipped ?: 0
        val extra = (if (failed > 0) " · $failed failed" else "") + (if (skipped > 0) " · $skipped skipped" else "")
        return when {
            t.state == "off" -> "Off"
            t.state == "starting" || t.state == null -> "Starting"
            t.state == "full" -> "Space full" + if (files > 0) " · $done of $files" else ""
            t.state == "paused" && done + failed < files -> "Paused while playing · $done of $files"
            t.state == "running" && done + failed < files -> "$done of $files$extra"
            else -> "All titles ready$extra"
        }
    }

    /** "1.2 GB of 10 GB" (null before the server says). */
    fun thumbnailSpaceText(t: TrickplayProgress?): String? {
        val used = t?.bytes ?: return null
        val max = t.maxBytes ?: return null
        return if (max > 0) "${AdminFmt.bytes(used)} of ${AdminFmt.bytes(max)}" else null
    }

    /** One `performance.*` setting as the settings document describes it (`PerfSetting`). */
    data class PerfSetting(
        val key: String, val title: String, val unit: String, val range: IntRange, val value: Int,
        /** live, restart, env-only */
        val apply: String,
        /** env, file, store, builtin, default */
        val source: String, val env: String?, val pending: Boolean,
    ) {
        val locked: Boolean get() = source == "env" || source == "file"
        val step: Int get() = when (key) {
            "performance.scan_quick_secs" -> 30
            "performance.cache_max_gb", "performance.download_max_gb" -> 5
            else -> 1
        }
    }

    private val knownPerf = listOf(
        listOf("performance.transcode_jobs", "Transcodes at Once", "", 1..16),
        listOf("performance.convert_jobs", "Audio Conversions at Once", "", 1..64),
        listOf("performance.jobs", "Background Jobs", "", 1..64),
        listOf("performance.download_jobs", "Download Jobs at Once", "", 1..8),
        listOf("performance.cache_max_gb", "Stream Cache", "GB", 1..100_000),
        listOf("performance.download_max_gb", "Prepared Downloads", "GB", 1..100_000),
        listOf("performance.scan_quick_secs", "Quick Scan Every", "s", 10..86_400),
        listOf("performance.scan_full_hours", "Full Scan Every", "h", 1..720),
        listOf("performance.scan_probe_jobs", "Scan Reads at Once", "", 1..16),
    )

    /** The settings Playback & Hardware shows: the ones the document has, in the iPhone's order. */
    fun perfSettings(doc: JsonObject): List<PerfSetting> {
        val settings = doc["settings"]?.jsonObject
        val perf = settings?.get("performance")?.jsonObject ?: return emptyList()
        val fields = doc["fields"]?.jsonObject
        return knownPerf.mapNotNull { k ->
            val key = k[0] as String
            val v = (perf[key.removePrefix("performance.")] as? JsonPrimitive)?.intOrNull ?: return@mapNotNull null
            val f = fields?.get(key)?.jsonObject
            @Suppress("UNCHECKED_CAST")
            PerfSetting(key, k[1] as String, k[2] as String, k[3] as IntRange, v, f.str("apply") ?: "live", f.str("source") ?: "default", f.str("env"), f?.get("restart_pending").bool() ?: false)
        }
    }

    private fun JsonObject?.str(k: String): String? = (this?.get(k) as? JsonPrimitive)?.contentOrNull
    private fun JsonElement?.bool(): Boolean? = (this as? JsonPrimitive)?.booleanOrNull

    /** `features.index_backfill`, `features.trickplay` as stored, `transcode.hardware`, and whether the environment sets them. */
    data class PlaybackDoc(
        val hardware: String?, val hardwareLocked: Boolean,
        val prepareAhead: Boolean?, val prepareLocked: Boolean,
        val thumbnails: Boolean?, val thumbnailsLocked: Boolean,
    )

    fun playbackDoc(doc: JsonObject): PlaybackDoc {
        val settings = doc["settings"]?.jsonObject
        val fields = doc["fields"]?.jsonObject
        val features = settings?.get("features")?.jsonObject
        fun fromEnv(k: String) = fields?.get(k)?.jsonObject.str("source") == "env"
        return PlaybackDoc(
            settings?.get("transcode")?.jsonObject.str("hardware"), fromEnv("transcode.hardware"),
            features?.get("index_backfill").bool(), fromEnv("features.index_backfill"),
            features?.get("trickplay").bool(), fromEnv("features.trickplay"),
        )
    }

    /** The name of a key waiting for a restart (`title(_:)`). */
    fun settingTitle(key: String): String = knownPerf.firstOrNull { it[0] == key }?.get(1) as? String ?: if (key == "transcode.hardware") "Transcode With" else key

    /** The line under Save: what takes effect when. */
    fun savedNote(later: List<String>): String =
        if (later.isEmpty()) "Saved: in effect now." else "Saved. ${later.joinToString(", ", transform = ::settingTitle)} take${if (later.size == 1) "s" else ""} effect after a restart."

    fun quotaText(s: AdminService): String? {
        if (s.id != "opensubtitles") return null
        val allowed = s.quota?.allowed ?: return null
        return "${s.quota.remaining ?: allowed} of $allowed left"
    }

    // Activity

    /** "S1E1 of Demo Series" as the row's title, and how it plays (`sessionRow`). */
    fun sessionTitle(s: ActivitySession): String =
        s.seriesTitle?.let { "$it · ${s.episodeLabel.orEmpty()}" } ?: s.title ?: "A title"

    fun sessionLine(s: ActivitySession): String {
        val how = when (s.method) {
            "transcode" -> "Transcoding" + (s.variant?.let { " · $it" } ?: "") + (if (s.tonemap == true) " · HDR→SDR" else "") + (if (s.burn != null) " · subtitles burned in" else "")
            "convert" -> "Direct · audio converted"
            else -> "Direct"
        }
        val pos = s.position?.let { p -> s.duration?.let { d -> "${clockText(p)} of ${clockText(d)}" } ?: clockText(p) }
        return listOfNotNull(s.user, how, pos).joinToString(" · ")
    }

    fun scanText(s: ActivityScan?, now: Instant = Instant.now()): String {
        s ?: return "Off"
        return when (s.phase) {
            "idle", null -> AdminFmt.ago(s.lastPass?.at, now)?.let { "Last scanned $it" } ?: "Idle"
            "queued" -> "Scan starting"
            else -> "Scanning (${s.phase})"
        }
    }

    fun subsText(s: ActivitySubtitles): String {
        if (s.extractions.isEmpty()) return "Idle"
        return AdminFmt.count(s.extractions.size, "title") + if (s.extractions.any { it.urgent }) " · for playback" else ""
    }

    /** A log line in its three parts (`LogLine.parts`): "2:26:06 PM", "WARN", the message. */
    fun logLine(line: String, zone: ZoneId = ZoneId.systemDefault()): Triple<String, String, String> {
        val w = line.split(' ').filter { it.isNotEmpty() }
        if (w.size < 3) return Triple("", "", line)
        val time = AdminFmt.date(w[0])?.let { DateTimeFormatter.ofPattern("h:mm:ss a", Locale.US).withZone(zone).format(it) } ?: w[0]
        return Triple(time, w[1], line.substringAfter(w[1]).trim())
    }

    // Accounts

    /** "Admin · 2 devices · active 3 hr. ago". */
    fun accountDetail(a: Account, now: Instant = Instant.now()): String {
        val parts = mutableListOf(if (a.isAdmin) "Admin" else "Viewer", AdminFmt.count(a.devices, "device"))
        AdminFmt.ago(a.lastUsed, now)?.let { parts += "active $it" }
        return parts.joined()
    }

    fun deviceDetail(d: AccountDevice, now: Instant = Instant.now()): String {
        if (d.current) return "This device"
        AdminFmt.ago(d.lastUsed, now)?.let { return "Used $it" }
        AdminFmt.ago(d.createdAt, now)?.let { return "Signed in $it" }
        return "Never used"
    }

    /** null when it can be sent, "" when not ready, else what to say (`NewAccountView.problem`). */
    fun newAccountProblem(name: String, password: String, again: String): String? = when {
        name.isBlank() -> ""
        password.length < 8 -> "The password needs at least 8 characters."
        password != again -> "The passwords don't match."
        else -> null
    }

    /** `ResetPasswordView.problem`. */
    fun resetPasswordProblem(password: String, again: String): String? = when {
        password.length < 8 -> "At least 8 characters."
        password != again -> "The passwords don't match."
        else -> null
    }

    /** `SetupAccountView.problem`. */
    fun setupAccountProblem(name: String, password: String, confirm: String): String? = when {
        name.isBlank() -> ""
        password.length < 8 -> "The password needs at least 8 characters."
        confirm.isNotEmpty() && confirm != password -> "The passwords don't match."
        confirm.isEmpty() -> ""
        else -> null
    }

    /** Digits only (a pasted "123 456" too), at most six. */
    fun setupCodeDigits(s: String): String = s.filter { it.isDigit() }.take(6)

    /** `videosText`: what a folder holds, in the picker. */
    fun videosText(n: Int, folders: Int): String {
        val v = if (n == 1) "1 video" else "$n videos"
        val f = if (folders == 1) "1 folder" else "$folders folders"
        val here = when {
            n == 0 && folders == 0 -> "Nothing in here yet."
            n == 0 -> "$f in here."
            folders == 0 -> "$v in here."
            else -> "$v and $f in here."
        }
        return "$here Moonrise looks through every folder inside the one you choose."
    }

    // AI matching

    val llmOutputs = listOf("auto" to "Automatic", "schema" to "JSON Schema", "tool" to "Tool Call", "prompt" to "JSON in the Prompt")

    /** scheme://host:port of an address, as the server binds keys to it (`origin`). */
    fun origin(s: String): String? = runCatching {
        val u = java.net.URI(s.trim())
        val scheme = u.scheme?.lowercase() ?: return null
        val host = u.host?.lowercase() ?: return null
        val port = if (u.port >= 0) u.port else if (scheme == "https") 443 else if (scheme == "http") 80 else 0
        "$scheme://$host:$port"
    }.getOrNull()

    fun llmProblemText(p: String?): String = when (p) {
        "unreachable" -> "The server couldn't reach that address."
        "auth" -> "The key or sign-in was refused."
        "not_found" -> "Not found: check the address and the model name."
        "format" -> "The model didn't answer in a form Moonrise can read. Try another answer format or model."
        "limit" -> "The usage limit is reached."
        else -> "It didn't work."
    }

    // The first scan's screen (`SetupScanView`)

    fun setupPhaseText(p: ScanProgress?, done: Boolean): String {
        if (done) return "Your library is ready."
        return when (p?.phase) {
            "scanning" -> "Looking for videos…"
            "identifying" -> if ((p.identify.total) > 0) "Recognizing titles… ${p.identify.done} of ${p.identify.total}" else "Recognizing titles…"
            "metadata" -> "Getting artwork and details…"
            "finishing" -> "Finishing…"
            else -> "Starting the scan…"
        }
    }

    fun setupStats(p: ScanProgress?, l: ScanLibrary?): List<Pair<Int, String>> {
        val files = maxOf(l?.files ?: 0, if (p?.phase == "scanning") p.walked else 0)
        if (l == null || l.movies + l.series == 0) return listOf(files to (if (files == 1) "File" else "Files"), 0 to "Movies", 0 to "Shows")
        return listOf(l.movies to (if (l.movies == 1) "Movie" else "Movies"), l.series to (if (l.series == 1) "Show" else "Shows"), l.episodes to (if (l.episodes == 1) "Episode" else "Episodes"))
    }

    fun setupNotes(p: ScanProgress?, l: ScanLibrary?, done: Boolean): List<String> {
        val out = mutableListOf<String>()
        val problem = p?.lastPass?.aborted ?: p?.lastPass?.error
        if (done && problem != null) out += "The scan had a problem: $problem"
        if (l != null && l.settling > 0) out += if (l.settling == 1) "1 file is still being copied: it's added a few minutes after it stops changing."
            else "${l.settling} files are still being copied: they're added a few minutes after they stop changing."
        if (l != null && done) {
            val eps = l.reviewEpisodes ?: 0
            val what = if (l.review == 1) "1 title" else "${l.review} titles"
            val epText = if (eps == 1) "1 episode" else "$eps episodes"
            when {
                l.review == 0 && eps == 0 -> {}
                eps == 0 -> out += "$what couldn't be recognized for sure: ${if (l.review == 1) "it's" else "they're"} in the library under the names of the files and folders."
                l.review == 0 -> out += "$epText couldn't be matched for sure: ${if (eps == 1) "it's" else "they're"} in the library under the file names."
                else -> out += "$what and $epText couldn't be recognized for sure: they're in the library under the names of the files and folders."
            }
        }
        if (done && l != null && l.files == 0) out += "No videos were found in that folder."
        if (!done) out += "You can continue: the scan goes on in the background."
        return out
    }
}

/** A TMDB search to run: for a kind of title (`movie` or `tv`), a title and perhaps a year. */
data class MatchSearch(val kind: String, val query: String, val year: Int?)

/** What a switch of "This Is" does to the show being matched to. */
enum class MatchShowStep { Keep, FindShow, OwnShow }

/** The page after a switch of "This Is": [year] null leaves the year as it is; [search] is run at once. */
data class MatchTypeSwitch(
    val asType: String?,
    val query: String,
    val year: String?,
    val showStep: MatchShowStep,
    val search: MatchSearch?,
)

/** Which search may still answer: a switch of "This Is" or a newer search outdates the ones before. */
class LatestSearch {
    private var newest = 0

    fun begin(): Int = ++newest
    fun forget() { newest++ }
    fun isNewest(search: Int): Boolean = search == newest
}

/** What Fix Match says and decides (`FixMatchView`, iPhone). */
object FixMatchRules {
    private const val NOT_IN_SEASON_LIST = "isn't in TMDB's season list"
    private val POSTER_PATH = Regex("/[A-Za-z0-9_-]{1,64}\\.(jpg|jpeg|png|webp)")

    /** A candidate's `poster_path` if it is one, the form the server's poster route takes: never an address. */
    fun posterPath(path: String?): String? = path?.takeIf { POSTER_PATH.matches(it) }

    /** movie, episode or series, as it will be matched: what "This Is" says, else its own kind. */
    fun kind(asType: String?, own: String?): String = asType ?: own ?: "movie"

    fun isSwitched(asType: String?, own: String?): Boolean = asType != null && asType != own

    /** TMDB's movie and TV ids are separate: only a movie searches movies. */
    fun searchKind(kind: String): String = if (kind == "movie") "movie" else "tv"

    fun search(kind: String, query: String, year: String): MatchSearch =
        MatchSearch(searchKind(kind), query.trim(), year.toIntOrNull())

    /**
     * Switching "This Is" to [to]. Everything, the search included, is worked out from the kind after
     * the switch: the page's own values are still the ones from before it. [year] is the Year field now.
     */
    fun switchType(to: String, i: MatchInfo, year: String): MatchTypeSwitch {
        val asType = if (to == i.kind) null else to
        val kind = kind(asType, i.kind)
        val switched = isSwitched(asType, i.kind)
        return when {
            kind == "movie" -> {
                val q = if (switched) fileTitle(i) else i.title
                val y = (if (switched) fileYear(i) else i.year)?.toString() ?: ""
                MatchTypeSwitch(asType, q, y, MatchShowStep.Keep, if (switched) search(kind, q, y) else null)
            }
            kind == "episode" && switched -> {
                val q = fileTitle(i)
                MatchTypeSwitch(asType, q, null, MatchShowStep.FindShow, search(kind, q, year))
            }
            kind == "episode" -> MatchTypeSwitch(asType, i.seriesTitle.orEmpty(), null, MatchShowStep.OwnShow, null)
            else -> MatchTypeSwitch(asType, i.title, null, MatchShowStep.Keep, null)
        }
    }

    /** The year a file name gives ("Kill.Bill.Vol.1.2003.1080p.mkv" → 2003). */
    fun fileYear(i: MatchInfo): Int? {
        val f = i.files.firstOrNull() ?: return null
        val name = f.substringAfterLast('/')
        return Regex("(?<![0-9])(19|20)[0-9]{2}(?![0-9])").find(name)?.value?.toInt()
    }

    /** The title a file name gives ("Kill.Bill.Vol.1.2003.1080p.mkv" → "Kill Bill Vol 1"). */
    fun fileTitle(i: MatchInfo): String {
        val f = i.files.firstOrNull() ?: return i.seriesTitle ?: i.title
        var name = f.substringAfterLast('/')
        if ('.' in name) name = name.substringBeforeLast('.')
        name = name.replace('.', ' ').replace('_', ' ')
        val words = mutableListOf<String>()
        for (w in name.split(' ').filter { it.isNotEmpty() }) {
            val y = w.takeIf { it.length == 4 }?.toIntOrNull()
            if (y != null && y in 1900..2099) break
            if (Regex("^[sS]\\d{1,2}[eE]\\d{1,3}").containsMatchIn(w)) break
            if (w.first() == '[' || w.first() == '(' || w.lowercase() in listOf("1080p", "720p", "2160p", "480p")) break
            words += w
        }
        val t = words.joinToString(" ").trim(' ', '-')
        return t.ifEmpty { i.title }
    }

    /** TMDB's episode has this file's title. */
    fun likely(e: TMDBEpisode, i: MatchInfo): Boolean {
        fun norm(s: String) = s.lowercase().filter { it.isLetterOrDigit() }
        val t = e.title?.takeIf { it.isNotEmpty() } ?: return false
        val a = norm(t); val b = norm(i.title)
        return a.isNotEmpty() && (a == b || (b.length > 5 && (a.contains(b) || b.contains(a))))
    }

    /** The list's rows: "No sure match on TMDB", "2 possible matches", the file's name. */
    fun reason(r: ReviewItem): String {
        if (r.isEpisode) return r.fileName ?: r.note ?: ""
        val n = r.note ?: return r.fileName ?: ""
        if (n.endsWith(NOT_IN_SEASON_LIST)) return "TMDB numbers this show's episodes differently"
        if (n == "no confident TMDB match") return if (r.candidates.isEmpty()) "No sure match on TMDB" else AdminFmt.count(r.candidates.size, "possible match", "possible matches")
        return n
    }

    fun rowTitle(r: ReviewItem): String =
        if (r.isEpisode) "S${r.season ?: 0}E${r.episode ?: 0}${r.episodeEnd?.let { "–E$it" } ?: ""} · ${r.title}" else r.heading

    /** Movies and shows first, then episodes by show. */
    fun groups(items: List<ReviewItem>): List<Pair<String, List<ReviewItem>>> {
        val out = mutableListOf<Pair<String, List<ReviewItem>>>()
        val titles = items.filter { !it.isEpisode }
        if (titles.isNotEmpty()) out += "Movies and Shows" to titles
        val bySeries = linkedMapOf<String, MutableList<ReviewItem>>()
        for (e in items.filter { it.isEpisode }) bySeries.getOrPut(e.seriesTitle ?: "Episodes") { mutableListOf() } += e
        return out + bySeries.map { it.key to it.value.toList() }
    }

    /** The footer when every item of a group is numbered differently on TMDB, else null. */
    fun groupFooter(items: List<ReviewItem>): String? {
        if (!items.all { it.isEpisode && (it.note ?: "").endsWith(NOT_IN_SEASON_LIST) }) return null
        return if (items.size == 1) "TMDB doesn't list this episode under this number: pick the one it is, or keep the number."
        else "TMDB doesn't list these episodes under these numbers: pick the ones they are, or keep the numbers."
    }

    fun heading(i: MatchInfo): String =
        if (i.kind == "episode") "${i.seriesTitle.orEmpty()} · S${i.season ?: 0}E${i.episode ?: 0}${i.part?.let { " · Part $it" } ?: ""} · ${i.title}"
        else i.year?.let { "${i.title} ($it)" } ?: i.title

    fun stateText(i: MatchInfo): String {
        if (i.note != null && i.state == "review") return if (i.note.endsWith(NOT_IN_SEASON_LIST)) "TMDB numbers this show's episodes differently." else "Not recognized for sure: ${i.note}."
        if (i.local) return "Named from its files (not matched on TMDB)."
        return when (i.state) {
            "manual" -> "Matched by hand."
            "llm" -> "Matched by AI."
            else -> "Matched on TMDB."
        }
    }

    /** A movie of the library other than this one (matching it adds a version to it). */
    fun isOtherTitle(c: MatchCandidate, itemId: String): Boolean {
        val other = c.inLibrary?.item ?: return false
        return (c.inLibrary.files ?: 0) > 0 && other != itemId
    }

    /** Files (and titles, parts) of a TMDB episode besides this one. */
    fun otherFilesOfEpisode(e: TMDBEpisode, i: MatchInfo?, show: Int, fileId: String?): Pair<Int, Int> {
        var files = e.inLibrary?.files ?: 0
        var titles = e.inLibrary?.titles ?: 0
        if (i != null && fileId == null && i.seriesTmdbId == show && i.season == e.season && i.episode == e.episode) {
            files -= maxOf(1, i.fileList?.size ?: 1)
            titles -= 1
        }
        return maxOf(0, files) to maxOf(0, titles)
    }

    fun fileLine(f: MatchFile): String =
        listOfNotNull(f.duration?.let { clockText(it) }, f.size?.let { AdminFmt.bytes(it) }, if (f.manual) "matched by hand" else null).joinToString(" · ")
}
