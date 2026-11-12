package app.moonrise.core

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.util.concurrent.TimeUnit

/** An HTTP error from the server: `code` and the start of its body. */
class MoonriseHttpException(val code: Int, val body: String) :
    Exception(if (body.isBlank()) "The Moonrise server returned $code." else "The Moonrise server returned $code. ${body.take(200)}")

/**
 * The Moonrise server's API (`/v1/...`, Bearer tokens), as the Apple apps speak it
 * (`clients/apple/Moonrise/Server/MoonriseClient.swift`). Plain Kotlin/JVM: no Android classes.
 *
 * Tokens travel only in the `Authorization` header. Session URLs (`/v1/s/{sid}/...`) need no
 * token: the session id is the capability.
 */
class MoonriseApi(
    val baseUrl: HttpUrl,
    @Volatile var token: String? = null,
    private val deviceId: String,
    private val deviceName: String,
    val http: OkHttpClient = defaultHttp,
) {
    companion object {
        const val DEFAULT_PORT = 8097

        val defaultHttp: OkHttpClient = OkHttpClient.Builder()
            .connectTimeout(8, TimeUnit.SECONDS)
            .readTimeout(20, TimeUnit.SECONDS)
            .build()

        val json = Json {
            ignoreUnknownKeys = true
            explicitNulls = false
            coerceInputValues = true
        }

        /** `host`, `host:port` or a URL; a bare host gets the server's port (8097). As `MoonriseClient.normalize`. */
        fun normalize(address: String): HttpUrl? {
            var s = address.trim()
            if (s.isEmpty()) return null
            if (!s.contains("://")) s = "http://$s"
            s = s.trimEnd('/')
            val explicitPort = Regex("^[a-zA-Z]+://(\\[[^]]+]|[^/:]+):\\d+").containsMatchIn(s)
            val url = s.toHttpUrlOrNull() ?: return null
            if (url.host.isEmpty()) return null
            val b = url.newBuilder().encodedPath("/").query(null).fragment(null)
            if (!explicitPort && url.scheme == "http") b.port(DEFAULT_PORT)
            return b.build()
        }
    }

    private val jsonType = "application/json".toMediaType()

    fun url(path: String, query: Map<String, String?> = emptyMap()): HttpUrl {
        val b = baseUrl.newBuilder().encodedPath("/" + path.trimStart('/'))
        query.toSortedMap().forEach { (k, v) -> if (v != null) b.addQueryParameter(k, v) }
        return b.build()
    }

    internal suspend fun send(method: String, url: HttpUrl, body: JsonObject? = null, authorized: Boolean = true, timeoutSeconds: Long? = null): String =
        withContext(Dispatchers.IO) {
            val rb = Request.Builder().url(url).header("Accept", "application/json")
            if (authorized) token?.let { rb.header("Authorization", "Bearer $it") }
            val payload = body?.toString()?.toRequestBody(jsonType)
            rb.method(method, payload ?: if (method == "POST" || method == "PUT") ByteArray(0).toRequestBody(null) else null)
            val client = if (timeoutSeconds != null) http.newBuilder().readTimeout(timeoutSeconds, TimeUnit.SECONDS).build() else http
            client.newCall(rb.build()).execute().use { resp ->
                val text = resp.body?.string().orEmpty()
                if (!resp.isSuccessful) throw MoonriseHttpException(resp.code, text.take(300).trim())
                text
            }
        }

    internal suspend fun <T> get(path: String, ser: KSerializer<T>, query: Map<String, String?> = emptyMap(), authorized: Boolean = true, timeoutSeconds: Long? = null): T =
        json.decodeFromString(ser, send("GET", url(path, query), authorized = authorized, timeoutSeconds = timeoutSeconds))

    private suspend fun <T> post(path: String, ser: KSerializer<T>, body: Map<String, String>, authorized: Boolean = true): T =
        json.decodeFromString(ser, send("POST", url(path), JsonObject(body.mapValues { JsonPrimitive(it.value) }), authorized))

    private val device: Map<String, String> get() = mapOf("device_id" to deviceId, "device_name" to deviceName)

    // ---------------------------------------------------------------- server and sign-in

    /** `GET /v1/info` (no sign-in): the server's id, name, version and what it can do. */
    suspend fun info(): ServerInfo = get("v1/info", ServerInfo.serializer(), authorized = false)

    suspend fun login(username: String, password: String): AuthResult {
        val r = post("v1/auth/login", AuthResult.serializer(), mapOf("username" to username, "password" to password) + device, authorized = false)
        token = r.token
        return r
    }

    /** Approves another device's code with this device's sign-in. Answers that device's name. */
    suspend fun approveDevice(userCode: String): String? =
        post("v1/auth/device/approve", ApproveAnswer.serializer(), mapOf("user_code" to userCode)).deviceName

    /** A new account with an invitation code: signed in on this device (`POST /v1/auth/invite`). */
    suspend fun redeemInvite(code: String, username: String, password: String): AuthResult {
        val r = post("v1/auth/invite", AuthResult.serializer(), mapOf("code" to code, "username" to username, "password" to password) + device, authorized = false)
        token = r.token
        return r
    }

    suspend fun me(): User = get("v1/me", User.serializer())

    /** A code shown on the TV, approved on another device or at `verifyUrl`. */
    suspend fun startDeviceCode(): DeviceCode = post("v1/auth/device", DeviceCode.serializer(), device, authorized = false)

    suspend fun pollDeviceCode(deviceCode: String): DevicePoll {
        val a = post("v1/auth/device/poll", PollAnswer.serializer(), mapOf("device_code" to deviceCode), authorized = false)
        return when (a.status) {
            "approved" -> if (a.token != null && a.user != null) {
                token = a.token
                DevicePoll.Approved(AuthResult(a.token, a.user))
            } else DevicePoll.Expired
            "slow_down" -> DevicePoll.SlowDown
            "expired" -> DevicePoll.Expired
            else -> DevicePoll.Pending
        }
    }

    /** Checks a new server's 6-digit setup code (`POST /v1/setup/verify`): false when it isn't the code. */
    suspend fun verifySetupCode(code: String): Boolean = try {
        send("POST", url("v1/setup/verify"), JsonObject(mapOf("code" to JsonPrimitive(code))), authorized = false)
        true
    } catch (e: MoonriseHttpException) {
        if (e.code == 403) false else throw e
    }

    /**
     * Sets up a new server (`POST /v1/setup/claim`): its first account, an admin, made with the
     * setup code, and signed in on this device.
     */
    suspend fun claim(code: String, username: String, password: String): AuthResult {
        val r = post("v1/setup/claim", AuthResult.serializer(), mapOf("code" to code, "username" to username, "password" to password) + device, authorized = false)
        token = r.token
        return r
    }

    /** Revokes this device's token on the server. */
    suspend fun logout() {
        runCatching { send("POST", url("v1/auth/logout")) }
    }

    // ---------------------------------------------------------------- browsing

    suspend fun home(limit: Int = 20): Home = get("v1/home", Home.serializer(), mapOf("limit" to "$limit"))

    /** Home's watching rows only, for the TV's Watch Next sync (an older server answers in full). */
    suspend fun homeWatching(): Home =
        get("v1/home", Home.serializer(), mapOf("limit" to "${WatchNextRules.MAX}", "watching" to "1"))

    suspend fun libraries(): List<Library> = get("v1/libraries", ListAnswer.serializer(Library.serializer())).items

    suspend fun item(id: String): Item = get("v1/items/${enc(id)}", Item.serializer())

    suspend fun seasons(seriesId: String): List<Item> = get("v1/shows/${enc(seriesId)}/seasons", ListAnswer.serializer(Item.serializer())).items

    suspend fun episodes(seriesId: String, seasonId: String? = null): List<Item> =
        get("v1/shows/${enc(seriesId)}/episodes", ListAnswer.serializer(Item.serializer()), mapOf("season" to seasonId)).items

    /** The episode to continue a show with (in progress, else the next unwatched). */
    suspend fun continueEpisode(seriesId: String): Item? {
        return get("v1/shows/${enc(seriesId)}/continue", ContinueAnswer.serializer()).item
    }

    /** Items of a library (or `type=movie|series` across libraries), sorted by `sort`. */
    suspend fun items(library: String? = null, type: String? = null, sort: String = "added", descending: Boolean = true, limit: Int = 40): List<Item> =
        get(
            "v1/items", ListAnswer.serializer(Item.serializer()),
            mapOf("library" to library, "type" to type, "sort" to sort, "order" to if (descending) "desc" else "asc", "offset" to "0", "limit" to "$limit"),
        ).items

    /**
     * One page of a Library view's titles (`type=movie|series` across libraries), with its filters
     * and sort. A title sort answers where each letter starts too.
     */
    suspend fun items(view: LibraryView, filter: LibraryFilter, offset: Int, limit: Int): ItemPage {
        val q = mutableMapOf<String, String?>(
            "type" to view.type, "sort" to filter.sort.query, "order" to if (filter.descending) "desc" else "asc",
            "offset" to "$offset", "limit" to "$limit",
        )
        q.putAll(filter.serverQuery)
        val a = get("v1/items", PageAnswer.serializer(), q)
        return ItemPage(a.items, a.total ?: a.items.size, a.index.orEmpty())
    }

    /** What a Library view's filters can pick, with counts, under the other filters. */
    suspend fun facets(view: LibraryView, filter: LibraryFilter): LibraryFacets {
        val q = mutableMapOf<String, String?>("type" to view.type, "recent_days" to "${LibraryFilter.RECENT_DAYS}")
        q.putAll(filter.serverQuery)
        return get("v1/facets", LibraryFacets.serializer(), q)
    }

    /** Titles (movies and shows) and episodes matching `term`. */
    suspend fun search(term: String): SearchResults = get("v1/search", SearchResults.serializer(), mapOf("q" to term))

    /** What Search offers before typing; null: this server doesn't offer it (capability `search-start`). */
    suspend fun searchStart(): SearchStart? =
        if ("search-start" !in info().capabilities) null else get("v1/search/start", SearchStart.serializer())

    /** A title was opened from Search: first in this user's Recent, on every device. */
    suspend fun recordSearchOpened(itemId: String) {
        send("PUT", url("v1/search/recent/${enc(itemId)}"))
    }

    /** Forgets the titles opened from Search, on every device. */
    suspend fun clearSearchRecent() {
        send("DELETE", url("v1/search/recent"))
    }

    /** Marks a title watched or not (a show or season: all its episodes). */
    suspend fun setPlayed(itemId: String, played: Boolean) {
        send(if (played) "PUT" else "DELETE", url("v1/items/${enc(itemId)}/played"))
    }

    /**
     * Remove from Continue Watching, on every device: the title's show (or film, or collection)
     * stays out of the row until it's played again.
     */
    suspend fun removeFromContinue(itemId: String) {
        send("DELETE", url("v1/continue/${enc(itemId)}"))
    }

    /** Remembers the version (file) this user plays of a title, on every device. */
    suspend fun chooseVersion(itemId: String, fileId: String) {
        send("PUT", url("v1/items/${enc(itemId)}/version"), JsonObject(mapOf("file_id" to JsonPrimitive(fileId))))
    }

    /**
     * Playback reports, as the Apple apps send them: `start` when playback starts, `progress`
     * every 10 s, `stop` when it ends or the player goes. `audio`: the server's index of the
     * audio track playing. `credits`: where the end credits start ([SegmentRules.creditsStart]).
     * `tracks`: the audio and subtitle tracks playing, kept by the server for the file.
     * The server keeps the resume position and marks the title played.
     */
    suspend fun report(
        sessionId: String, event: ReportEvent, position: Double, paused: Boolean, audio: Int? = null, subtitle: Int? = null, credits: Double? = null,
        tracks: TrackChoice? = null,
    ) {
        val body = mutableMapOf<String, JsonElement>("position" to JsonPrimitive(position), "paused" to JsonPrimitive(paused))
        audio?.let { body["audio"] = JsonPrimitive(it) }
        subtitle?.let { body["subtitle"] = JsonPrimitive(it) }
        credits?.let { body["credits"] = JsonPrimitive(it) }
        tracks?.let { body["tracks"] = it.json() }
        send("POST", url("v1/sessions/${enc(sessionId)}/${event.path}"), JsonObject(body))
    }

    /** Image ids are hashes of the artwork: immutable, so the URL is its own cache key. Images need no token. */
    fun imageUrl(id: String, width: Int): String = url("v1/images/${enc(id)}", mapOf("w" to "$width")).toString()

    // ---------------------------------------------------------------- playback

    /**
     * Starts a playback session. `hdr`: the display shows HDR (else the server tone-maps HDR
     * titles to SDR). `start`: seconds. `version`: the file to play (else the viewer's chosen one,
     * else the server's pick). `burn`: a bitmap subtitle track the server burns into the picture.
     * `lossless`: the audio output takes more than 2 channels, so lossless multichannel tracks
     * (TrueHD, DTS-HD MA…) come as FLAC with every channel instead of E-AC-3 5.1 (servers that
     * don't know it ignore it). `dolbyVision`: the display shows Dolby Vision and a decoder for it
     * exists ([ClientRules.wantsDolbyVision]): `dv=1`, sent only with `hdr`.
     */
    suspend fun playback(
        itemId: String, start: Double? = null, hdr: Boolean, version: String? = null, quality: Long? = null, burn: Int? = null,
        lossless: Boolean = false, dolbyVision: Boolean = false,
    ): Playback {
        val q = mutableMapOf<String, String?>("hdr" to if (hdr) "1" else "0", "version" to version, "quality" to quality?.toString(), "burn" to burn?.toString())
        if (lossless) q["lossless"] = "1"
        if (hdr && dolbyVision) q["dv"] = "1"
        if (start != null && start > 1) q["start"] = String.format(java.util.Locale.ROOT, "%.3f", start)
        return get("v1/items/${enc(itemId)}/playback", Playback.serializer(), q)
    }

    /** The session's master playlist as an absolute URL. */
    fun masterUrl(p: Playback): String? = p.master?.let { baseUrl.resolve(it)?.toString() }

    /** Ends a session: the server stops its transcode and conversion runs. */
    suspend fun closeSession(sessionId: String) {
        runCatching { send("DELETE", url("v1/s/${enc(sessionId)}"), authorized = false) }
    }

    // ---------------------------------------------------------------- segments and subtitles

    /**
     * The title's skippable segments to offer, merged by the server with the community databases'
     * (`merged=1`, servers 0.28+; `provisional`: their credits while the file is still looked at;
     * `community` false leaves them out). An older server answers with the file's own (named
     * chapters, and an episode's end credits found in the file: `source: detected`). `pending`
     * while the server still looks for those. `version`: the file played (its chapters and credits).
     */
    suspend fun segments(itemId: String, version: String? = null, provisional: Boolean = false, community: Boolean = true): ServerSegments {
        val query = mapOf("version" to version, "merged" to "1", "provisional" to if (provisional) "1" else null, "community" to if (community) null else "0")
        return serverSegments(get("v1/items/${enc(itemId)}/segments", SegmentsAnswer.serializer(), query))
    }

    /** `itemId~version` when a version is chosen: the subtitle routes take the file that way. */
    fun mediaId(itemId: String, version: String?): String = if (version != null) "$itemId~$version" else itemId

    /** The title's subtitle tracks (embedded, sidecar and fetched), with their files and timing. */
    suspend fun subtitles(mediaId: String): List<SubtitleTrack> =
        get("v1/items/${enc(mediaId)}/subtitles", SubtitleList.serializer()).subtitles

    /** A server path (a track's `url`) as an absolute URL. */
    fun resolve(path: String): String? = baseUrl.resolve(path)?.toString()

    /** The text of a file on the server (a subtitle track), with the token. */
    suspend fun fetchText(url: String): String = withContext(Dispatchers.IO) {
        val rb = Request.Builder().url(url)
        token?.let { rb.header("Authorization", "Bearer $it") }
        http.newCall(rb.build()).execute().use { resp ->
            val bytes = resp.body?.bytes() ?: ByteArray(0)
            if (!resp.isSuccessful) throw MoonriseHttpException(resp.code, String(bytes).take(300).trim())
            SubtitleParser.decode(bytes)
        }
    }

    /** A window of an embedded track (30 s of the file's time), while the whole track isn't out yet. */
    suspend fun subtitleWindow(mediaId: String, index: Int, window: Int): SubtitleWindow =
        get("v1/items/${enc(mediaId)}/subtitles/$index/cues", SubtitleWindow.serializer(), mapOf("window" to "$window"))

    /** A track's timing now; asking starts its match if that's still to do. */
    suspend fun subtitleTiming(mediaId: String, index: Int): SubtitleTiming =
        get("v1/items/${enc(mediaId)}/subtitles/$index/timing", SubtitleTiming.serializer())

    /** Keeps the viewer's nudge, or their choice of the original timing, on the server. */
    suspend fun setSubtitleTiming(mediaId: String, index: Int, nudge: Double? = null, original: Boolean? = null): SubtitleTiming {
        val body = mutableMapOf<String, JsonPrimitive>()
        nudge?.let { body["nudge"] = JsonPrimitive(it) }
        original?.let { body["original"] = JsonPrimitive(it) }
        return json.decodeFromString(SubtitleTiming.serializer(), send("PUT", url("v1/items/${enc(mediaId)}/subtitles/$index/timing"), JsonObject(body)))
    }

    /** OpenSubtitles results for a title, through the server (`language`: ISO 639-1; `query`: search by text). */
    suspend fun searchSubtitles(mediaId: String, language: String, query: String? = null): SubtitleSearch =
        get("v1/items/${enc(mediaId)}/subtitles/search", SubtitleSearch.serializer(), mapOf("lang" to language, "q" to query?.takeIf { it.isNotBlank() }))

    /** Fetches a result: it becomes a track of the title, on every device. Answers the track. */
    suspend fun fetchSubtitle(mediaId: String, result: SubtitleResult): SubtitleTrack {
        val body = mutableMapOf<String, JsonPrimitive>(
            "result_id" to JsonPrimitive(result.id), "name" to JsonPrimitive(result.name), "hearing_impaired" to JsonPrimitive(result.hearingImpaired),
        )
        result.language?.let { body["language"] = JsonPrimitive(it) }
        return json.decodeFromString(SubtitleTrack.serializer(), send("POST", url("v1/items/${enc(mediaId)}/subtitles/fetch"), JsonObject(body)))
    }

    /** The scrubbing thumbnails of a title (`mediaId`), or of the file a session plays (`sessionId`). */
    suspend fun thumbnails(mediaId: String, sessionId: String?): Thumbnails =
        get("v1/items/${enc(mediaId)}/trickplay", Thumbnails.serializer(), mapOf("sid" to sessionId))

    /** The bytes of a file on the server (a thumbnail sheet), with the token. */
    suspend fun fetchBytes(url: String): ByteArray = withContext(Dispatchers.IO) {
        val rb = Request.Builder().url(url)
        token?.let { rb.header("Authorization", "Bearer $it") }
        http.newCall(rb.build()).execute().use { resp ->
            val bytes = resp.body?.bytes() ?: ByteArray(0)
            if (!resp.isSuccessful) throw MoonriseHttpException(resp.code, String(bytes).take(300).trim())
            bytes
        }
    }

    // ---------------------------------------------------------------- downloads

    /**
     * Describes a segmented offline set (`GET /v1/items/{id}/offline`, capability `offline-sets`):
     * what to fetch for the title at `quality` bits/s (null: the original), its `version`.
     */
    suspend fun offlineSet(itemId: String, hdr: Boolean, quality: Long? = null, version: String? = null): OfflineSet {
        val q = mapOf("hdr" to if (hdr) "1" else "0", "quality" to quality?.toString(), "version" to version)
        return get("v1/items/${enc(itemId)}/offline", OfflineSet.serializer(), q)
    }

    /** Playback of downloads on this device (`POST /v1/sessions/local`), sent long after it happened. */
    suspend fun reportLocal(reports: List<LocalReport>) {
        val body = JsonObject(mapOf("reports" to kotlinx.serialization.json.JsonArray(reports.map { it.json() })))
        send("POST", url("v1/sessions/local"), body)
    }

    /** A file of an offline set (`base` + "/" + `path`), written whole to `to` (a partial file is never there). */
    suspend fun downloadFile(base: String, path: String, version: String?, to: java.io.File, timeoutSeconds: Long = 90): Long = withContext(Dispatchers.IO) {
        val b = baseUrl.newBuilder().encodedPath(base.trimEnd('/') + "/" + path.trimStart('/'))
        version?.let { b.addQueryParameter("v", it) }
        fetchToFile(b.build().toString(), to, timeoutSeconds)
    }

    /** A file on the server (a subtitle track's path), written whole to `to`. */
    suspend fun downloadUrl(path: String, to: java.io.File, timeoutSeconds: Long = 90): Long =
        fetchToFile(baseUrl.resolve(path)?.toString() ?: throw java.io.IOException("bad path"), to, timeoutSeconds)

    private suspend fun fetchToFile(url: String, to: java.io.File, timeoutSeconds: Long): Long = withContext(Dispatchers.IO) {
        val rb = Request.Builder().url(url)
        token?.let { rb.header("Authorization", "Bearer $it") }
        val client = http.newBuilder().readTimeout(timeoutSeconds, TimeUnit.SECONDS).build()
        client.newCall(rb.build()).execute().use { resp ->
            if (!resp.isSuccessful) throw MoonriseHttpException(resp.code, resp.body?.string().orEmpty().take(300).trim())
            to.parentFile?.mkdirs()
            val tmp = java.io.File(to.parentFile, ".tmp-${to.name}-${System.nanoTime()}")
            try {
                resp.body!!.byteStream().use { input -> tmp.outputStream().use { input.copyTo(it, 64 * 1024) } }
                if (!tmp.renameTo(to)) throw java.io.IOException("can't save ${to.name}")
            } finally {
                tmp.delete()
            }
            to.length()
        }
    }

    internal fun enc(s: String): String = java.net.URLEncoder.encode(s, "UTF-8").replace("+", "%20")
}

enum class ReportEvent(val path: String) { Start("start"), Progress("progress"), Stop("stop") }

@Serializable
data class SearchResults(val titles: List<Item> = emptyList(), val episodes: List<Item> = emptyList())

@Serializable
internal data class PageAnswer(val items: List<Item>, val total: Int? = null, val index: List<ItemPage.Letter>? = null)

@Serializable
internal data class PollAnswer(val status: String, val token: String? = null, val user: User? = null)

@Serializable
internal data class ApproveAnswer(@SerialName("device_name") val deviceName: String? = null)

@Serializable
internal data class ContinueAnswer(val item: Item? = null)

@Serializable
internal data class SegmentsAnswer(
    val items: List<SegmentItem> = emptyList(),
    val pending: Boolean? = null,
    val merged: Boolean? = null,
    @SerialName("community_pending") val communityPending: Boolean? = null,
)

/** The answer as the app uses it: only `detected` is kept as a source (the rules tell it apart). */
internal fun serverSegments(a: SegmentsAnswer): ServerSegments {
    val items = a.items.mapNotNull { s ->
        SegmentKind.of(s.kind)?.let { Segment(it, s.start, s.end, if (s.source == "detected") "detected" else null) }
    }
    return ServerSegments(items, pending = a.pending ?: false, merged = a.merged ?: false, communityPending = a.communityPending ?: false)
}

@Serializable
internal data class SegmentItem(val kind: String, val start: Double, val end: Double, val source: String? = null)
