// SPDX-License-Identifier: MPL-2.0
package app.moonrise.core

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject

// The server's admin API as the phone app's Manage Server, setup and Fix Match screens use it
// (`MoonriseAdminAPI.swift` and `MoonriseSetupAPI.swift` on the iPhone; server: admin/manage.rs,
// admin/services.rs, library/api.rs). The answers are in api/examples/admin-*.json.

// ---------------------------------------------------------------- libraries and folders

@Serializable
data class ManagedRoot(
    val source: String,
    val path: String,
    val available: Boolean? = null,
    val state: String? = null,
    val error: String? = null,
    val host: String? = null,
    val share: String? = null,
    val url: String? = null,
    val username: String? = null,
    @SerialName("credential_id") val credentialId: String? = null,
) {
    /** As shown: "/media/movies", "192.168.1.20 › Media › Movies". */
    val display: String get() = if (source != "smb" || host == null || share == null) path else shareDisplay(host, share, path)

    val online: Boolean get() = if (state != null) state == "online" else (available ?: true)

    /** As the server takes it back unchanged (its stored credential is kept). */
    val body: JsonObject get() {
        if (source != "smb") return JsonObject(mapOf("source" to JsonPrimitive("local"), "path" to JsonPrimitive(path)))
        val b = mutableMapOf<String, JsonElement>("source" to JsonPrimitive("smb"), "host" to JsonPrimitive(host.orEmpty()), "share" to JsonPrimitive(share.orEmpty()), "path" to JsonPrimitive(path))
        credentialId?.let { b["credential_id"] = JsonPrimitive(it) }
        return JsonObject(b)
    }
}

internal fun shareDisplay(host: String, share: String, path: String): String =
    (listOf(host, share) + path.split('/').filter { it.isNotEmpty() }).joinToString(" › ")

@Serializable
data class ManagedLibrary(val id: String, val name: String, val kind: String? = null, val roots: List<ManagedRoot> = emptyList())

@Serializable
data class LibraryList(val items: List<ManagedLibrary> = emptyList(), @SerialName("managed_by") val managedBy: String = "store")

/** A user name and password for a network share (only asked for when the share needs one). */
data class ShareLogin(val username: String, val password: String) {
    val fields: Map<String, JsonElement> get() = mapOf("username" to JsonPrimitive(username), "password" to JsonPrimitive(password))
}

/** A library's folder being added: on the server's computer, or in a network share (`LibraryRoot`). */
data class LibraryRoot(val smb: Boolean, val path: String, val host: String = "", val share: String = "", val login: ShareLogin? = null, val videoFiles: Int = 0) {
    val body: JsonObject get() = JsonObject(
        if (!smb) mapOf("source" to JsonPrimitive("local"), "path" to JsonPrimitive(path))
        else mapOf("source" to JsonPrimitive("smb"), "host" to JsonPrimitive(host), "share" to JsonPrimitive(share), "path" to JsonPrimitive(path)) + (login?.fields ?: emptyMap()),
    )

    /** As shown: "/media/movies", "nas › Media › movies". */
    val display: String get() = if (smb) shareDisplay(host, share, path) else path
}

@Serializable
data class Folder(val name: String, val path: String)

@Serializable
data class FolderListing(
    val path: String? = null,
    val parent: String? = null,
    val dirs: List<Folder> = emptyList(),
    @SerialName("video_files") val videoFiles: Int = 0,
    val truncated: Boolean? = null,
)

@Serializable
data class FileServer(val name: String, val host: String)

@Serializable
internal data class FileServerList(val servers: List<FileServer> = emptyList())

@Serializable
data class Share(val name: String, val comment: String? = null)

@Serializable
internal data class ShareList(val shares: List<Share> = emptyList())

@Serializable
data class ShareListing(val path: String = "", val parent: String? = null, val dirs: List<Folder> = emptyList(), @SerialName("video_files") val videoFiles: Int = 0)

@Serializable
data class ShareTest(val ok: Boolean, val url: String? = null, val entries: Int = 0, @SerialName("video_files") val videoFiles: Int = 0, val error: String? = null)

/** A network-share failure's `problem` (auth_required, wrong_password, access_denied, not_found, unreachable, other). */
data class ShareProblem(val problem: String, val message: String)

/** The problem a failed share request answered with (`MoonriseClientError.shareProblem`), or null. */
fun Throwable.shareProblem(): ShareProblem? {
    val e = this as? MoonriseHttpException ?: return null
    val o = runCatching { MoonriseApi.json.parseToJsonElement(e.body).jsonObject }.getOrNull() ?: return null
    val p = (o["problem"] as? JsonPrimitive)?.takeIf { it.isString }?.content ?: return null
    return ShareProblem(p, (o["error"] as? JsonPrimitive)?.takeIf { it.isString }?.content ?: p)
}

// ---------------------------------------------------------------- scans

@Serializable
data class ScanIdentify(val done: Int = 0, val total: Int = 0)

@Serializable
data class ScanLibrary(
    val id: String,
    val name: String,
    val kind: String? = null,
    val files: Int = 0,
    val identified: Int = 0,
    val movies: Int = 0,
    val series: Int = 0,
    val episodes: Int = 0,
    val review: Int = 0,
    @SerialName("review_episodes") val reviewEpisodes: Int? = null,
    val settling: Int = 0,
)

@Serializable
data class ScanLastPass(val error: String? = null, val aborted: String? = null, val at: Long? = null)

@Serializable
data class ScanProgress(
    /** idle, queued, scanning, identifying, metadata, finishing */
    val phase: String,
    val walked: Int = 0,
    val identify: ScanIdentify = ScanIdentify(),
    val passes: Int = 0,
    @SerialName("last_pass") val lastPass: ScanLastPass? = null,
    val libraries: List<ScanLibrary> = emptyList(),
)

// ---------------------------------------------------------------- services

@Serializable
data class ServiceQuota(val tier: String? = null, val allowed: Int? = null, val remaining: Int? = null)

@Serializable
data class AdminService(
    val id: String,
    val name: String,
    /** configured, builtin, not_set, off */
    val state: String,
    val summary: String = "",
    val with: String = "",
    val without: String = "",
    val account: Boolean? = null,
    val quota: ServiceQuota? = null,
    val provider: String? = null,
    @SerialName("provider_name") val providerName: String? = null,
    val attribution: String? = null,
)

@Serializable
internal data class ServiceList(val services: List<AdminService> = emptyList())

// ---------------------------------------------------------------- AI matching

@Serializable
data class SecretState(val set: Boolean = false, val last4: String? = null)

@Serializable
data class LLMPresetSaved(@SerialName("api_key") val apiKey: SecretState? = null, val model: String? = null, @SerialName("base_url") val baseUrl: String? = null, val output: String? = null)

@Serializable
data class LLMPreset(
    val id: String,
    val name: String,
    val style: String = "",
    @SerialName("base_url") val baseUrl: String = "",
    val model: String = "",
    val output: String = "auto",
    /** required, optional, sign_in */
    val key: String = "required",
    val help: String = "",
    val local: Boolean = false,
    val saved: LLMPresetSaved? = null,
)

@Serializable
data class LLMCurrent(
    val provider: String,
    @SerialName("base_url") val baseUrl: String = "",
    val model: String = "",
    val output: String = "auto",
    @SerialName("output_in_use") val outputInUse: String? = null,
    @SerialName("api_key") val apiKey: SecretState = SecretState(),
    @SerialName("keys_aside") val keysAside: List<String>? = null,
    val enabled: Boolean = false,
)

@Serializable
data class ChatGPTAccount(val id: String, val label: String = "", val email: String = "", @SerialName("signed_in") val signedIn: Boolean = false, val active: Boolean = false)

@Serializable
data class ChatGPTState(
    @SerialName("signed_in") val signedIn: Boolean = false,
    val email: String? = null,
    @SerialName("needs_sign_in") val needsSignIn: Boolean? = null,
    val account: String? = null,
    val accounts: List<ChatGPTAccount>? = null,
)

@Serializable
data class LLMOverview(val presets: List<LLMPreset> = emptyList(), val current: LLMCurrent, val chatgpt: ChatGPTState = ChatGPTState())

@Serializable
data class LLMModel(val id: String, val name: String)

@Serializable
internal data class LLMModelList(val models: List<LLMModel> = emptyList())

@Serializable
data class LLMTestResult(
    val ok: Boolean = false,
    val correct: Boolean? = null,
    val output: String? = null,
    val ms: Int? = null,
    val error: String? = null,
    /** unreachable, auth, not_found, format, limit, other */
    val problem: String? = null,
)

@Serializable
data class ChatGPTSignedOut(val revoked: Boolean? = null, val message: String? = null)

// ---------------------------------------------------------------- accounts

@Serializable
data class Account(
    val id: String,
    val name: String,
    @SerialName("is_admin") val isAdmin: Boolean = false,
    @SerialName("created_at") val createdAt: String? = null,
    val devices: Int = 0,
    @SerialName("last_used") val lastUsed: String? = null,
    val me: Boolean = false,
)

@Serializable
data class AccountDevice(
    val id: String,
    @SerialName("device_name") val deviceName: String? = null,
    @SerialName("created_at") val createdAt: String? = null,
    @SerialName("last_used") val lastUsed: String? = null,
    val current: Boolean = false,
)

@Serializable
data class Invite(val id: String, val admin: Boolean = false, @SerialName("expires_at") val expiresAt: String, val code: String? = null)

// ---------------------------------------------------------------- the server

@Serializable
data class SystemSizes(val database: Long = 0, val subtitles: Long = 0, val artwork: Long = 0, val data: Long = 0, val cache: Long = 0)

@Serializable
data class SystemGpu(val vendor: String = "", val name: String? = null, val driver: String? = null, @SerialName("memory_mb") val memoryMb: Int? = null)

@Serializable
data class SystemOtherGpu(val vendor: String = "", val device: String = "", val usable: Boolean = false)

@Serializable
data class SystemCpu(val model: String? = null, val threads: Int = 0)

@Serializable
data class SystemHardware(
    val nvidia: List<SystemGpu> = emptyList(),
    @SerialName("other_gpus") val otherGpus: List<SystemOtherGpu> = emptyList(),
    val cpu: SystemCpu = SystemCpu(),
    val ffmpeg: String? = null,
)

@Serializable
data class TranscodeChoice(val key: String, val label: String)

@Serializable
data class SystemTranscode(
    val backend: String = "",
    val label: String = "",
    val device: String? = null,
    val decode: String = "",
    val tonemap: String = "",
    val encode: String = "",
    @SerialName("hdr_kept") val hdrKept: Boolean = false,
    @SerialName("max_height") val maxHeight: Int = 0,
    val requested: String = "auto",
    val notes: List<String> = emptyList(),
    val available: Boolean = false,
    val choices: List<TranscodeChoice>? = null,
)

@Serializable
data class SystemInfo(
    val name: String = "",
    val version: String = "",
    @SerialName("started_at") val startedAt: String? = null,
    @SerialName("uptime_secs") val uptimeSecs: Int = 0,
    val sizes: SystemSizes = SystemSizes(),
    @SerialName("cache_max_bytes") val cacheMaxBytes: Long = 0,
    @SerialName("downloads_bytes") val downloadsBytes: Long = 0,
    @SerialName("download_max_bytes") val downloadMaxBytes: Long = 0,
    val hardware: SystemHardware = SystemHardware(),
    /** nvenc, qsv, vaapi, cpu, videotoolbox, v4l2, off, unavailable */
    val transcoding: String = "unavailable",
    val transcode: SystemTranscode? = null,
    @SerialName("can_restart") val canRestart: Boolean = false,
    @SerialName("restart_required") val restartRequired: List<String> = emptyList(),
)

@Serializable
data class ActivitySession(
    val sid: String,
    val user: String? = null,
    val title: String? = null,
    @SerialName("series_title") val seriesTitle: String? = null,
    @SerialName("episode_label") val episodeLabel: String? = null,
    val method: String = "copy",
    val variant: String? = null,
    val tonemap: Boolean? = null,
    val burn: Int? = null,
    val duration: Double? = null,
    val position: Double? = null,
)

@Serializable
data class ActivityDownload(val id: String, val label: String = "", val method: String = "", val state: String = "", val progress: Double = 0.0, val error: String? = null)

@Serializable
data class SubtitleExtraction(val item: String = "", val urgent: Boolean = false, val reading: Boolean = false)

@Serializable
data class SubtitleBackfill(val done: Int? = null, val files: Int? = null, val failed: Int? = null, val current: String? = null)

@Serializable
data class ActivitySubtitles(val extractions: List<SubtitleExtraction> = emptyList(), val backfill: SubtitleBackfill? = null)

@Serializable
data class IndexBackfill(val state: String? = null, val files: Int? = null, val done: Int? = null, val failed: Int? = null, val current: String? = null)

@Serializable
data class TrickplayProgress(
    val state: String? = null,
    val files: Int? = null,
    val done: Int? = null,
    val failed: Int? = null,
    val skipped: Int? = null,
    val bytes: Long? = null,
    @SerialName("max_bytes") val maxBytes: Long? = null,
)

@Serializable
data class ActivityLastPass(val at: String? = null, val ms: Int? = null, val error: String? = null, val aborted: String? = null, @SerialName("files_seen") val filesSeen: Int? = null)

@Serializable
data class ActivityScan(val phase: String? = null, val full: Boolean? = null, @SerialName("started_at") val startedAt: String? = null, @SerialName("last_pass") val lastPass: ActivityLastPass? = null)

@Serializable
data class ActivityTranscode(val track: String = "", val owner: String = "")

@Serializable
data class Activity(
    val sessions: List<ActivitySession> = emptyList(),
    val transcodes: List<ActivityTranscode> = emptyList(),
    @SerialName("transcode_slots") val transcodeSlots: Int = 0,
    val subtitles: ActivitySubtitles = ActivitySubtitles(),
    val downloads: List<ActivityDownload> = emptyList(),
    val scan: ActivityScan? = null,
    @SerialName("index_backfill") val indexBackfill: IndexBackfill? = null,
    val trickplay: TrickplayProgress? = null,
    val errors: List<String> = emptyList(),
)

@Serializable
internal data class LogLines(val lines: List<String> = emptyList())

// ---------------------------------------------------------------- Fix Match

@Serializable
data class MatchInLibrary(val item: String? = null, val files: Int? = null, val titles: Int? = null)

@Serializable
data class MatchCandidate(
    @SerialName("tmdb_id") val tmdbId: Int,
    val title: String,
    @SerialName("original_title") val originalTitle: String? = null,
    val year: Int? = null,
    val overview: String? = null,
    /** For [matchPosterUrl]. Candidates' TMDB `poster` addresses are never read: apps talk only to the server. */
    @SerialName("poster_path") val posterPath: String? = null,
    @SerialName("in_library") val inLibrary: MatchInLibrary? = null,
)

@Serializable
data class ReviewItem(
    val id: String,
    val kind: String,
    val title: String,
    val year: Int? = null,
    val note: String? = null,
    val files: List<String>? = null,
    val candidates: List<MatchCandidate> = emptyList(),
    val library: String = "",
    @SerialName("series_id") val seriesId: String? = null,
    @SerialName("series_title") val seriesTitle: String? = null,
    @SerialName("series_tmdb_id") val seriesTmdbId: Int? = null,
    val season: Int? = null,
    val episode: Int? = null,
    @SerialName("episode_end") val episodeEnd: Int? = null,
) {
    val isEpisode: Boolean get() = kind == "episode"

    /** "The A-Team · S1E14", "Alien (1979)". */
    val heading: String get() =
        if (isEpisode && seriesTitle != null) "$seriesTitle · S${season ?: 0}E${episode ?: 0}" + (episodeEnd?.let { "–E$it" } ?: "")
        else year?.let { "$title ($it)" } ?: title

    val fileName: String? get() = files?.firstOrNull()?.substringAfterLast('/')
}

@Serializable
internal data class ReviewList(val items: List<ReviewItem> = emptyList())

@Serializable
data class MatchFile(
    val id: String,
    val name: String? = null,
    val size: Long? = null,
    val duration: Double? = null,
    val manual: Boolean = false,
    val split: Boolean? = null,
)

@Serializable
data class MatchInfo(
    val id: String,
    val kind: String,
    val title: String,
    val year: Int? = null,
    val state: String = "",
    val note: String? = null,
    val candidates: List<MatchCandidate> = emptyList(),
    val season: Int? = null,
    val episode: Int? = null,
    val part: Int? = null,
    @SerialName("series_title") val seriesTitle: String? = null,
    @SerialName("series_tmdb_id") val seriesTmdbId: Int? = null,
    @SerialName("tmdb_id") val tmdbId: Int? = null,
    val local: Boolean = false,
    val files: List<String> = emptyList(),
    val tmdb: Boolean = false,
    val manual: Boolean? = null,
    @SerialName("file_list") val fileList: List<MatchFile>? = null,
)

@Serializable
internal data class MatchCandidateList(val items: List<MatchCandidate> = emptyList())

@Serializable
data class TMDBEpisode(
    val episode: Int,
    val season: Int,
    val title: String? = null,
    @SerialName("air_date") val airDate: String? = null,
    @SerialName("in_library") val inLibrary: MatchInLibrary? = null,
)

@Serializable
data class TMDBSeason(
    @SerialName("tmdb_id") val tmdbId: Int,
    val title: String? = null,
    val seasons: List<Int> = emptyList(),
    val season: Int = 1,
    val episodes: List<TMDBEpisode> = emptyList(),
)

@Serializable
data class IdentifyAnswer(val item: String? = null, @SerialName("file_item") val fileItem: String? = null)

/** A TMDB title with its `tmdb_id` and a corrected type, and the rest of what Fix Match can ask (`identify`, iPhone). */
data class IdentifyRequest(
    val tmdbId: Int? = null,
    val season: Int? = null,
    val episode: Int? = null,
    val local: Boolean = false,
    /** "movie" or "episode": makes it that type. */
    val type: String? = null,
    /** The episode's next part (not another version of it). */
    val part: Boolean = false,
    /** Keeping a part of an episode keeps it that part. */
    val keepPart: Int? = null,
    /** That file alone. */
    val fileId: String? = null,
) {
    val body: JsonObject get() {
        val b = mutableMapOf<String, JsonElement>()
        if (local) b["local"] = JsonPrimitive(true)
        type?.let { b["type"] = JsonPrimitive(it) }
        tmdbId?.let { b["tmdb_id"] = JsonPrimitive(it) }
        season?.let { b["season"] = JsonPrimitive(it) }
        episode?.let { b["episode"] = JsonPrimitive(it) }
        if (part) b["part"] = keepPart?.let { JsonPrimitive(it) } ?: JsonPrimitive(true)
        fileId?.let { b["file_id"] = JsonPrimitive(it) }
        return JsonObject(b)
    }
}

// ---------------------------------------------------------------- the calls

private fun obj(vararg pairs: Pair<String, JsonElement>): JsonObject = JsonObject(mapOf(*pairs))

suspend fun MoonriseApi.managedLibraries(): LibraryList = get("v1/admin/libraries", LibraryList.serializer())

suspend fun MoonriseApi.renameLibrary(id: String, name: String) { send("PATCH", url("v1/admin/libraries/${enc(id)}"), obj("name" to JsonPrimitive(name))) }

/** The library's folders as a whole (each one checked by the server; a full scan follows). */
suspend fun MoonriseApi.setLibraryRoots(id: String, roots: List<JsonObject>) {
    send("PATCH", url("v1/admin/libraries/${enc(id)}"), obj("roots" to JsonArray(roots)), timeoutSeconds = 60)
}

suspend fun MoonriseApi.deleteLibrary(id: String) { send("DELETE", url("v1/admin/libraries/${enc(id)}")) }

/** Adds a library; the server checks its folder and starts a full scan. */
suspend fun MoonriseApi.createLibrary(name: String, root: LibraryRoot): ManagedLibrary =
    MoonriseApi.json.decodeFromString(ManagedLibrary.serializer(), send("POST", url("v1/admin/libraries"), obj("name" to JsonPrimitive(name), "roots" to JsonArray(listOf(root.body))), timeoutSeconds = 60))

/** Scan Now: one library's folders walked in full (`library`), or every library (`full`). */
suspend fun MoonriseApi.startScan(library: String? = null, full: Boolean = false) {
    send("POST", url("v1/admin/scan", mapOf("wait" to "0", "library" to library, "full" to if (library == null && full) "1" else null)))
}

suspend fun MoonriseApi.scanProgress(): ScanProgress = get("v1/admin/scan/progress", ScanProgress.serializer())

/** The server's folders (no path: where browsing may start). */
suspend fun MoonriseApi.browseServer(path: String?): FolderListing = get("v1/admin/fs/browse", FolderListing.serializer(), mapOf("path" to path))

/** File servers on the server's network. */
suspend fun MoonriseApi.discoverFileServers(): List<FileServer> =
    get("v1/admin/smb/discover", FileServerList.serializer(), mapOf("wait_ms" to "2500"), timeoutSeconds = 15).servers

private fun shareBody(fields: Map<String, String>, login: ShareLogin?): JsonObject = JsonObject(fields.mapValues { JsonPrimitive(it.value) } + (login?.fields ?: emptyMap()))

suspend fun MoonriseApi.shares(host: String, login: ShareLogin?): List<Share> =
    MoonriseApi.json.decodeFromString(ShareList.serializer(), send("POST", url("v1/admin/smb/shares"), shareBody(mapOf("host" to host), login), timeoutSeconds = 40)).shares

suspend fun MoonriseApi.browseShare(host: String, share: String, path: String, login: ShareLogin?): ShareListing =
    MoonriseApi.json.decodeFromString(ShareListing.serializer(), send("POST", url("v1/admin/smb/browse"), shareBody(mapOf("host" to host, "share" to share, "path" to path), login), timeoutSeconds = 40))

suspend fun MoonriseApi.testShare(host: String, share: String, path: String, login: ShareLogin?): ShareTest =
    MoonriseApi.json.decodeFromString(ShareTest.serializer(), send("POST", url("v1/admin/smb/test"), shareBody(mapOf("host" to host, "share" to share, "path" to path), login), timeoutSeconds = 40))

suspend fun MoonriseApi.services(): List<AdminService> = get("v1/admin/services", ServiceList.serializer()).services

/** `PATCH /v1/admin/settings` with dotted keys. */
suspend fun MoonriseApi.updateSettings(changes: Map<String, JsonElement>) { send("PATCH", url("v1/admin/settings"), JsonObject(changes)) }

/** The settings document's values and fields (`GET /v1/admin/settings`), as JSON. */
suspend fun MoonriseApi.settingsDocument(): JsonObject = MoonriseApi.json.parseToJsonElement(send("GET", url("v1/admin/settings"))).jsonObject

/** Signs in to OpenSubtitles with these (nothing saved): null when it worked, else why not. */
suspend fun MoonriseApi.checkOpenSubtitles(username: String, password: String): String? = try {
    send("POST", url("v1/admin/services/opensubtitles/check"), obj("username" to JsonPrimitive(username), "password" to JsonPrimitive(password)), timeoutSeconds = 40)
    null
} catch (e: MoonriseHttpException) {
    if (e.code == 422 || e.code == 409) e.body else throw e
}

suspend fun MoonriseApi.llmOverview(): LLMOverview? = try {
    get("v1/admin/llm", LLMOverview.serializer(), timeoutSeconds = 15)
} catch (e: MoonriseHttpException) {
    if (e.code == 404) null else throw e
}

suspend fun MoonriseApi.llmModels(body: JsonObject): List<LLMModel> =
    MoonriseApi.json.decodeFromString(LLMModelList.serializer(), send("POST", url("v1/admin/llm/models"), body, timeoutSeconds = 30)).models

/** One real identification with these settings (nothing saved). */
suspend fun MoonriseApi.llmTest(body: JsonObject): LLMTestResult =
    MoonriseApi.json.decodeFromString(LLMTestResult.serializer(), send("POST", url("v1/admin/llm/test"), body, timeoutSeconds = 300))

suspend fun MoonriseApi.chatGPTSignOut(account: String? = null): ChatGPTSignedOut {
    val text = send("POST", url("v1/admin/llm/chatgpt/sign-out"), JsonObject(account?.let { mapOf("account" to JsonPrimitive(it)) } ?: emptyMap()), timeoutSeconds = 30)
    return runCatching { MoonriseApi.json.decodeFromString(ChatGPTSignedOut.serializer(), text) }.getOrDefault(ChatGPTSignedOut())
}

suspend fun MoonriseApi.accounts(): List<Account> = get("v1/admin/users", ListAnswer.serializer(Account.serializer())).items

suspend fun MoonriseApi.createAccount(name: String, password: String, admin: Boolean) {
    send("POST", url("v1/admin/users"), obj("name" to JsonPrimitive(name), "password" to JsonPrimitive(password), "admin" to JsonPrimitive(admin)))
}

suspend fun MoonriseApi.updateAccount(id: String, password: String? = null, admin: Boolean? = null) {
    val b = mutableMapOf<String, JsonElement>()
    password?.let { b["password"] = JsonPrimitive(it) }
    admin?.let { b["admin"] = JsonPrimitive(it) }
    send("PATCH", url("v1/admin/users/${enc(id)}"), JsonObject(b))
}

suspend fun MoonriseApi.deleteAccount(id: String) { send("DELETE", url("v1/admin/users/${enc(id)}")) }

suspend fun MoonriseApi.devices(user: String): List<AccountDevice> = get("v1/admin/users/${enc(user)}/devices", ListAnswer.serializer(AccountDevice.serializer())).items

suspend fun MoonriseApi.signOutDevice(user: String, device: String) { send("DELETE", url("v1/admin/users/${enc(user)}/devices/${enc(device)}")) }

suspend fun MoonriseApi.invites(): List<Invite> = get("v1/admin/invites", ListAnswer.serializer(Invite.serializer())).items

suspend fun MoonriseApi.createInvite(admin: Boolean): Invite =
    MoonriseApi.json.decodeFromString(Invite.serializer(), send("POST", url("v1/admin/invites"), obj("admin" to JsonPrimitive(admin))))

suspend fun MoonriseApi.deleteInvite(id: String) { send("DELETE", url("v1/admin/invites/${enc(id)}")) }

suspend fun MoonriseApi.systemInfo(): SystemInfo = get("v1/admin/system", SystemInfo.serializer(), timeoutSeconds = 30)

suspend fun MoonriseApi.activity(): Activity = get("v1/admin/activity", Activity.serializer())

suspend fun MoonriseApi.serverLog(lines: Int, level: String? = null): List<String> =
    get("v1/admin/logs", LogLines.serializer(), mapOf("lines" to "$lines", "level" to level)).lines

suspend fun MoonriseApi.restartServer() { send("POST", url("v1/admin/restart")) }

/** A cheap "is it back" probe for after a restart (`info(timeout:)`, iPhone). */
suspend fun MoonriseApi.isUp(): Boolean = runCatching { info() }.isSuccess

/** Asks for candidates with `poster_path`. A server that doesn't know it ignores it: no posters then, only the placeholder. */
private val SERVER_POSTERS = mapOf("server_posters" to "1")

suspend fun MoonriseApi.reviewList(): List<ReviewItem> = get("v1/admin/review", ReviewList.serializer(), SERVER_POSTERS, timeoutSeconds = 30).items

/** A title as Fix Match shows it (`compute`: TMDB searched now when no candidates were kept). */
suspend fun MoonriseApi.matchInfo(id: String, compute: Boolean): MatchInfo =
    get("v1/admin/items/${enc(id)}/match", MatchInfo.serializer(), SERVER_POSTERS + (if (compute) mapOf("compute" to "1") else emptyMap()), timeoutSeconds = 40)

suspend fun MoonriseApi.searchMatches(kind: String, query: String, year: Int?): List<MatchCandidate> =
    get("v1/admin/match/search", MatchCandidateList.serializer(), SERVER_POSTERS + mapOf("kind" to kind, "q" to query, "year" to year?.toString()), timeoutSeconds = 30).items

/**
 * Where a Fix Match candidate's poster is on this server (null: no poster, or a path of any other
 * form, and the row keeps its placeholder). Admins only, so the request needs the token.
 */
fun MoonriseApi.matchPosterUrl(path: String?, size: String = "w185"): String? =
    FixMatchRules.posterPath(path)?.let { url("v1/admin/match/poster", mapOf("path" to it, "size" to size)).toString() }

suspend fun MoonriseApi.tmdbSeason(show: Int, season: Int?): TMDBSeason =
    get("v1/admin/match/episodes", TMDBSeason.serializer(), mapOf("tmdb_id" to "$show", "season" to season?.toString()), timeoutSeconds = 30)

/** Fix Match: answers the title's id after the change. */
suspend fun MoonriseApi.identify(id: String, request: IdentifyRequest): String? {
    val a = identifyCall(id, request.body)
    return if (request.fileId == null) a.item else (a.fileItem ?: a.item)
}

/** Back to Automatic: forgets the Fix Match choices (a title's files, or one file's) and identifies them again. */
suspend fun MoonriseApi.matchAutomatically(id: String, fileId: String? = null): IdentifyAnswer =
    identifyCall(id, JsonObject(mapOf<String, JsonElement>("automatic" to JsonPrimitive(true)) + (fileId?.let { mapOf("file_id" to JsonPrimitive(it)) } ?: emptyMap())))

/** Split Off: one of the title's files becomes a title of its own. Answers its id (`fileItem`). */
suspend fun MoonriseApi.splitOff(id: String, fileId: String): IdentifyAnswer = identifyCall(id, obj("split" to JsonPrimitive(true), "file_id" to JsonPrimitive(fileId)))

private suspend fun MoonriseApi.identifyCall(id: String, body: JsonObject): IdentifyAnswer =
    MoonriseApi.json.decodeFromString(IdentifyAnswer.serializer(), send("POST", url("v1/admin/items/${enc(id)}/identify"), body, timeoutSeconds = 60))

