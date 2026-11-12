package app.moonrise.shared.downloads

import android.content.Context
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.moonrise.core.DownloadBadge
import app.moonrise.core.DownloadRecord
import app.moonrise.core.DownloadRules
import app.moonrise.core.DownloadState
import app.moonrise.core.Item
import app.moonrise.core.MediaFile
import app.moonrise.core.MoonriseApi
import app.moonrise.core.MoonriseHttpException
import app.moonrise.core.OfflineSet
import app.moonrise.core.Quality
import app.moonrise.core.SegmentService
import app.moonrise.core.LocalSubtitle
import app.moonrise.shared.AppSettings
import app.moonrise.shared.Log
import app.moonrise.shared.Network
import app.moonrise.shared.Session
import app.moonrise.shared.displayHdr
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import java.io.File
import java.util.Collections
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

/**
 * Every download on this phone (`DownloadManager`, `SegmentedDownloads`): the server's HLS
 * segments of one video variant and the title's audio tracks and subtitles (an "offline set",
 * `GET /v1/items/{id}/offline`), saved one by one under `<download>/hls/`, played by
 * `DownloadDataSource` from the moment the first ones are in. Segments are fetched in order from
 * where the viewer is, then from the start; one the player asks for that isn't here yet is
 * fetched at once. One download runs at a time. A foreground service (the phone app's) keeps the
 * app running while one does.
 *
 * [records] is what screens observe; it changes on the main thread only.
 */
object Downloads {
    private const val TAG = "MoonriseDownload"
    private const val PARALLEL = DownloadRules.PARALLEL

    private lateinit var app: Context
    private lateinit var root: File

    var records by mutableStateOf<List<DownloadRecord>>(emptyList())
        private set

    private val counts by derivedStateOf { DownloadRules.counts(records) }
    val seriesCounts: Map<String, Int> get() = counts.first
    val seasonCounts: Map<String, Int> get() = counts.second

    /** A download is being fetched or waiting: the app's foreground service runs meanwhile. */
    var onWorkChanged: () -> Unit = {}

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val saver = Dispatchers.IO.limitedParallelism(1)
    private val jobs = HashMap<String, Job>()
    private val done = ConcurrentHashMap<String, MutableSet<Int>>()
    private val playhead = ConcurrentHashMap<String, Pair<Int, Long>>()
    private val inflight = ConcurrentHashMap<String, Deferred<Unit>>()
    private val bytes = ConcurrentHashMap<String, java.util.concurrent.atomic.AtomicLong>()
    private val lastSave = ConcurrentHashMap<String, Long>()
    private var lastIndexSave = 0L

    fun init(context: Context) {
        app = context.applicationContext
        root = File(app.filesDir, "Downloads").apply { mkdirs() }
        records = runCatching { File(root, "index.json").takeIf { it.exists() }?.readText()?.let { DownloadRecord.listFrom(it) } }.getOrNull().orEmpty()
        // The index is written every few seconds at most: what's on disk says how far each got.
        records = records.map { r ->
            val count = r.segmentCount
            if (r.state == DownloadState.Completed || !r.hasPlaylist || count == null) r
            else scanComplete(r).size.let { n -> r.copy(segmentsDone = n, progress = n.toDouble() / maxOf(count, 1)) }
        }
        Log.i(TAG) { "${records.size} downloads" }
        LocalReports.init(app)
        LocalTrackMemory.init(app)
        ProgressStore.init(app)
        ServerReach.start(app)
        if (records.any { it.state == DownloadState.Downloading || it.state == DownloadState.Queued }) startNext()
        records.filter { it.segmentsPending && it.item != null }.forEach { refreshSegments(it.id) }
    }

    fun record(id: String): DownloadRecord? = records.firstOrNull { it.id == id }
    fun forItem(itemId: String): DownloadRecord? = records.firstOrNull { it.itemId == itemId }
    /** The download of a title that plays from the device now. */
    fun playable(itemId: String): DownloadRecord? = records.firstOrNull { it.itemId == itemId && it.isPlayable }
    fun badge(itemId: String): DownloadBadge? = forItem(itemId)?.badge
    fun episodes(seriesId: String): List<DownloadRecord> = DownloadRules.episodes(records, seriesId)
    val hasPlayable: Boolean get() = records.any { it.isPlayable }
    val active: List<DownloadRecord> get() = records.filter { it.state == DownloadState.Downloading || it.state == DownloadState.Queued }

    /** A file of a download, by its relative path. */
    fun file(id: String, relative: String): File = File(File(root, id), relative)

    private fun hls(id: String) = File(File(root, id), "hls")

    /** Anything that writes into a download's folder: no folder is left behind if the download is deleted meanwhile. */
    private inline fun <T> intoFolder(id: String, write: () -> T): T =
        DownloadRules.writingInto(File(root, id), { record(id) != null }, write)

    // ---------------------------------------------------------------- enqueue

    /** Which server an item would download from (known once the server has answered). */
    private suspend fun serverId(api: MoonriseApi): String = ServerReach.serverId ?: api.info().serverId

    /** `version`: which of the title's files (several versions: the one picked). */
    suspend fun download(item: Item, quality: Quality, version: MediaFile? = null) {
        val api = Session.api ?: return
        if (forItem(item.id) != null) return
        val sid = runCatching { serverId(api) }.getOrNull() ?: return
        val rec = DownloadRules.newRecord(item, sid, quality, version, UUID.randomUUID().toString(), System.currentTimeMillis())
        withContext(Dispatchers.Main) {
            if (forItem(item.id) != null) return@withContext
            records = listOf(rec) + records
            save(force = true)
            Log.i(TAG) { "start ${rec.title} ${rec.qualityLabel}" }
            scope.launch { intoFolder(rec.id) { fetchArtworkAndSegments(rec.id, api); fetchSubtitles(rec.id, api) } }
            startNext()
        }
    }

    /** One at a time: the oldest one waiting. */
    private fun startNext() {
        Session.main.post {
            val waiting = records.filter { it.state == DownloadState.Downloading || it.state == DownloadState.Queued }
            if (waiting.any { jobs[it.id]?.isActive == true }) return@post
            val next = waiting.minByOrNull { it.createdAt }
            if (next == null) { onWorkChanged(); return@post }
            update(next.id) { it.copy(state = DownloadState.Downloading, error = null) }
            waiting.filter { it.id != next.id && it.state == DownloadState.Downloading }.forEach { r -> update(r.id) { it.copy(state = DownloadState.Queued) } }
            jobs[next.id] = scope.launch { intoFolder(next.id) { run(next.id) } }
            onWorkChanged()
        }
    }

    /** The server came back, or the network: a download waiting for it goes on (it retries by itself meanwhile). */
    fun resumeWaiting() { startNext() }

    private class Failure(val permanent: Boolean, message: String) : Exception(message)

    private fun reachable(rec: DownloadRecord): Boolean = ServerReach.canReach(rec.serverId)

    private suspend fun run(id: String) {
        var backoff = 2000L
        while (kotlin.coroutines.coroutineContext.isActive) {
            val rec = record(id) ?: return
            if (rec.state != DownloadState.Downloading && rec.state != DownloadState.Queued) return
            try {
                step(id)
                jobsDone(id)
                return
            } catch (e: Failure) {
                if (e.permanent) {
                    Log.i(TAG) { "${rec.title}: failed: ${e.message}" }
                    updateMain(id) { it.copy(state = DownloadState.Failed, error = e.message) }
                    jobsDone(id)
                    return
                }
                wait(id, e.message, backoff)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                wait(id, e.message, backoff)
            }
            backoff = minOf(backoff * 2, 15_000)
        }
    }

    private suspend fun wait(id: String, why: String?, backoff: Long) {
        Log.i(TAG) { "${record(id)?.title}: waiting ($why), again in ${backoff / 1000} s" }
        updateMain(id) { it.copy(error = if (cellularBlocked()) "Waiting for Wi-Fi…" else "Waiting for the server…") }
        ServerReach.nudge()
        delay(backoff)
    }

    private fun jobsDone(id: String) {
        Session.main.post { jobs.remove(id); onWorkChanged(); startNext() }
    }

    /** Whether this download may run on this network (`Download over Cellular`). */
    private fun cellularBlocked(): Boolean = Network.cellular && !AppSettings.allowCellularDownloads

    /** Describes the set (once), then fetches what is missing. Returns when complete. */
    private suspend fun step(id: String) {
        var rec = record(id) ?: return
        val api = Session.api ?: throw Failure(false, "not signed in")
        if (cellularBlocked()) throw Failure(false, "cellular")
        if (!reachable(rec)) throw Failure(false, "unreachable")
        if (!rec.hasPlaylist) {
            describe(rec, api)
            rec = record(id) ?: return
        }
        updateMain(id) { it.copy(state = DownloadState.Downloading, error = null) }
        val count = rec.segmentCount ?: return
        val tracks = DownloadRules.tracks(rec)
        for (t in tracks) if (!exists(id, "$t/init.mp4")) fetch(rec, api, "$t/init.mp4")
        // Subtitles: one file per track, made by the server from the whole title.
        val subs = scope.launch {
            for (s in rec.subtitleTracks.orEmpty()) if (!exists(id, "subs/$s/cues.vtt")) runCatching { fetch(rec, api, "subs/$s/cues.vtt", timeoutSeconds = 900) }
        }
        val claimed = mutableSetOf<Int>()
        coroutineScope {
            repeat(PARALLEL) { worker ->
                launch {
                    while (isActive) {
                        // While someone watches it, one at a time: the player's own requests come first.
                        if (worker > 0 && playerActive(id)) {
                            if (synchronized(claimed) { DownloadRules.nextSegment(complete(rec), claimed, playhead[id]?.first, count) } == null) break
                            delay(500)
                            continue
                        }
                        val n = synchronized(claimed) {
                            DownloadRules.nextSegment(complete(rec), claimed, playhead[id]?.first, count)?.also { claimed += it }
                        } ?: break
                        for (t in tracks) if (!exists(id, "$t/$n.m4s")) fetch(rec, api, "$t/$n.m4s")
                        segmentComplete(id, n, count)
                        if (record(id)?.state != DownloadState.Downloading) throw CancellationException("no longer downloading")
                    }
                }
            }
        }
        subs.join()
        if (complete(rec).size < count) throw Failure(false, "segments missing")
        val size = hls(id).walkTopDown().filter { it.isFile }.sumOf { it.length() }
        Log.i(TAG) { "${rec.title}: complete, $count segments, ${size shr 20} MB" }
        updateMain(id) {
            it.copy(state = DownloadState.Completed, progress = 1.0, segmentsDone = count, bytesWritten = size, bytesExpected = size, error = null)
        }
    }

    private fun playerActive(id: String): Boolean = playhead[id]?.let { System.currentTimeMillis() - it.second < 20_000 } == true

    /** Complete segments of a download (every track has them), read from its folder the first time. */
    fun complete(rec: DownloadRecord): Set<Int> = done.getOrPut(rec.id) {
        Collections.synchronizedSet(HashSet<Int>().apply { addAll(scanComplete(rec)) })
    }

    private fun scanComplete(rec: DownloadRecord): Set<Int> {
        val count = rec.segmentCount ?: return emptySet()
        fun present(sub: String) = File(hls(rec.id), sub).list().orEmpty().mapNotNull { n -> n.takeIf { it.endsWith(".m4s") }?.removeSuffix(".m4s")?.toIntOrNull() }.toSet()
        var all = present("video")
        for (a in rec.audioTracks.orEmpty()) all = all intersect present("audio/${a.index}")
        return all.filter { it < count }.toSet()
    }

    /** Seconds of the title the contiguous downloaded part covers (from the start). */
    fun contiguousSeconds(rec: DownloadRecord): Double {
        val n = DownloadRules.contiguous(complete(rec).toSet())
        if (n == 0) return 0.0
        val text = runCatching { File(hls(rec.id), "video/index.m3u8").readText() }.getOrNull() ?: return 0.0
        return DownloadRules.seconds(text, n)
    }

    /** The player's playlist, cut at the contiguous part (no server, download incomplete). */
    fun growingPlaylist(id: String, relative: String): ByteArray? {
        val rec = record(id) ?: return null
        val text = runCatching { File(hls(id), relative).readText() }.getOrNull() ?: return null
        return DownloadRules.truncated(text, DownloadRules.contiguous(complete(rec).toSet())).toByteArray()
    }

    /** The player reads this file of the download; a segment moves the download there. */
    fun playerAsked(id: String, relative: String) {
        DownloadRules.segment(relative)?.let { playhead[id] = it to System.currentTimeMillis() }
    }

    private suspend fun describe(rec: DownloadRecord, api: MoonriseApi) {
        val set: OfflineSet = try {
            api.offlineSet(rec.itemId, displayHdr(app), Quality.of(rec.qualityLabel).bitrate, rec.mediaSourceId)
        } catch (e: MoonriseHttpException) {
            if (e.code >= 500) throw Failure(false, e.message ?: "server error")
            throw Failure(true, e.body.ifEmpty { "The server answered ${e.code}." })
        }
        val dir = hls(rec.id).apply { mkdirs() }
        File(dir, "master.m3u8").writeText(set.master)
        File(dir, "video").mkdirs()
        File(dir, "video/index.m3u8").writeText(set.playlist)
        for (a in set.audio) {
            val d = File(dir, "audio/${a.index}").apply { mkdirs() }
            File(d, "index.m3u8").writeText(set.playlist)
        }
        for (s in set.subtitles) {
            val d = File(dir, "subs/${s.index}").apply { mkdirs() }
            File(d, "index.m3u8").writeText(set.subsPlaylist)
        }
        Log.i(TAG) { "${rec.title}: set ${set.variant.id} (${set.variant.label}), ${set.segments} segments, audio ${set.audio.map { it.index }}, ${set.subtitles.size} subtitles, ~${set.expected shr 20} MB" }
        done[rec.id] = Collections.synchronizedSet(HashSet())
        // Here before the caller reads the record back.
        withContext(Dispatchers.Main) { update(rec.id) { DownloadRules.described(it, set) }; save(force = true) }
    }

    private fun exists(id: String, relative: String) = File(hls(id), relative).exists()

    /** Fetches one file of the set (shared with a request for the same file). */
    private suspend fun fetch(rec: DownloadRecord, api: MoonriseApi, path: String, timeoutSeconds: Long = 90) {
        val base = rec.setBase ?: throw Failure(true, "No download set")
        val key = "${rec.id}/$path"
        val task = inflight.getOrPut(key) {
            scope.async {
                try {
                    val n = intoFolder(rec.id) { api.downloadFile(base, path, rec.setVersion, File(hls(rec.id), path), timeoutSeconds) }
                    bytes.getOrPut(rec.id) { java.util.concurrent.atomic.AtomicLong() }.addAndGet(n)
                } catch (e: MoonriseHttpException) {
                    throw when (e.code) {
                        409 -> Failure(true, "The file changed on the server. Delete the download and download it again.")
                        401, 403 -> Failure(true, "Your sign-in has expired. Sign in again in Settings.")
                        404 -> Failure(true, "The server doesn't have this title any more.")
                        else -> Failure(false, "HTTP ${e.code}")
                    }
                } finally {
                    inflight.remove(key)
                }
                Unit
            }
        }
        task.await()
    }

    private fun segmentComplete(id: String, n: Int, count: Int) {
        val d = done.getOrPut(id) { Collections.synchronizedSet(HashSet()) }
        d += n
        val total = d.size
        val now = System.currentTimeMillis()
        if (total != count && now - (lastSave[id] ?: 0) <= 1000) return
        lastSave[id] = now
        val written = bytes[id]?.get() ?: 0
        Session.main.post {
            update(id) { r ->
                val progress = total.toDouble() / maxOf(count, 1)
                val w = maxOf(r.bytesWritten, written)
                r.copy(segmentsDone = total, progress = progress, bytesWritten = w,
                    bytesExpected = if (progress > 0.02 && w > 0) maxOf(r.bytesExpected, (w / progress).toLong()) else r.bytesExpected)
            }
        }
    }

    /**
     * A file the player asked for that isn't on the device: fetched now when the server can be
     * reached, else waits (up to `waitSeconds`) for it to arrive. True when it is there. On the
     * player's loading thread.
     */
    fun fetchNow(id: String, relative: String, waitSeconds: Int): Boolean = runBlocking {
        val deadline = System.currentTimeMillis() + waitSeconds * 1000L
        while (System.currentTimeMillis() < deadline) {
            if (exists(id, relative)) return@runBlocking true
            val rec = record(id)
            if (rec != null && rec.hasPlaylist && reachable(rec)) {
                val api = Session.api
                if (api != null && !cellularBlocked()) {
                    val ok = runCatching { fetch(rec, api, relative, ((deadline - System.currentTimeMillis()) / 1000).coerceAtLeast(5)); true }.getOrElse { false }
                    if (ok) {
                        DownloadRules.segment(relative)?.let { n ->
                            val count = rec.segmentCount
                            if (count != null && DownloadRules.tracks(rec).all { exists(id, "$it/$n.m4s") }) segmentComplete(id, n, count)
                        }
                        return@runBlocking true
                    }
                }
            }
            delay(700)
        }
        exists(id, relative)
    }

    // ---------------------------------------------------------------- what's saved with it

    private suspend fun fetchArtworkAndSegments(id: String, api: MoonriseApi) {
        val rec = record(id) ?: return
        val item = rec.item ?: return
        val dir = File(root, id).apply { mkdirs() }
        if (item.isEpisode) item.seriesId?.let { sid ->
            runCatching { api.item(sid) }.getOrNull()?.let { show -> updateMain(id) { it.copy(series = DownloadRules.snapshot(show)) } }
        }
        saveSegments(id, api)
        if (record(id)?.segmentsPending == true) refreshSegments(id)
        item.posterImage?.let { poster ->
            if (DownloadRules.saveArtwork(File(dir, "poster.jpg")) { api.fetchBytes(api.imageUrl(poster, 500)) }) {
                updateMain(id) { it.copy(posterFile = "poster.jpg") }
            }
        }
        // Wide artwork: an episode's own still, else the title's backdrop.
        val wide = if (item.isEpisode) item.images?.thumb?.let { api.imageUrl(it, 1000) } else null
        (wide ?: item.backdropImage?.let { api.imageUrl(it, 1400) })?.let { url ->
            if (DownloadRules.saveArtwork(File(dir, "backdrop.jpg")) { api.fetchBytes(url) }) {
                updateMain(id) { it.copy(backdropFile = "backdrop.jpg") }
            }
        }
    }

    private val refreshing = ConcurrentHashMap.newKeySet<String>()

    /** Asks for a download's segments again each minute until the server's answer is final, for up to an hour. */
    private fun refreshSegments(id: String) {
        if (!refreshing.add(id)) return
        scope.launch {
            try {
                repeat(60) {
                    delay(60_000)
                    if (record(id)?.segmentsPending != true) return@launch
                    val api = Session.api ?: return@launch
                    if (reachable(record(id) ?: return@launch)) saveSegments(id, api)
                }
            } finally {
                refreshing.remove(id)
            }
        }
    }

    /** The skippable segments, kept with the download for offline playback (`saveSegments`). */
    private suspend fun saveSegments(id: String, api: MoonriseApi) {
        val rec = record(id) ?: return
        val item = rec.item ?: return
        val service = SegmentService(api, { AppSettings.onlineSegments }, log = { s -> Log.i("MoonriseSegments") { s } })
        val version = rec.mediaSourceId
        val segments = service.segments(item, version, provisional = true)
        val pending = (item.isEpisode || service.awaitingCommunity(item, version)) && service.awaitingServer(item, version)
        updateMain(id) { it.copy(segments = if (segments.isNotEmpty()) segments else it.segments, segmentsPending = pending) }
        Log.i(TAG) { "segments saved: ${segments.size}${if (pending) ", asked again later" else ""}" }
    }

    /** Text subtitles as .srt so they work offline (the server's tracks, also those it fetched from OpenSubtitles). */
    private suspend fun fetchSubtitles(id: String, api: MoonriseApi) {
        val rec = record(id) ?: return
        val mediaId = api.mediaId(rec.itemId, rec.mediaSourceId)
        val tracks = runCatching { api.subtitles(mediaId) }.getOrNull() ?: return
        val dir = File(root, id).apply { mkdirs() }
        val subs = mutableListOf<LocalSubtitle>()
        for (t in tracks) {
            if (!t.text) continue
            val path = t.url ?: continue
            val name = "sub-${t.index}.srt"
            val ok = runCatching { api.downloadUrl(path, File(dir, name), timeoutSeconds = 900) }.isSuccess
            if (ok) subs += LocalSubtitle(t.name.replace(" - ", " · "), t.language, name, t.name)
        }
        Log.i(TAG) { "${subs.size} of ${tracks.count { it.text }} subtitle tracks saved" }
        updateMain(id) { it.copy(subtitles = subs) }
    }

    /** Keeps a subtitle fetched during playback with the download of the same title. */
    fun attachSubtitle(itemId: String, title: String, language: String?, text: ByteArray): LocalSubtitle? {
        val rec = forItem(itemId) ?: return null
        val name = "os-${UUID.randomUUID().toString().take(8)}.srt"
        File(File(root, rec.id).apply { mkdirs() }, name).writeBytes(text)
        val s = LocalSubtitle(title, language, name)
        Session.main.post { update(rec.id) { it.copy(subtitles = it.subtitles + s) } }
        return s
    }

    // ---------------------------------------------------------------- controls

    fun pause(id: String) {
        stopJob(id)
        update(id) { it.copy(state = DownloadState.Paused) }
        onWorkChanged()
        startNext()
    }

    fun resume(id: String) {
        update(id) { it.copy(state = DownloadState.Queued, error = null) }
        startNext()
    }

    fun delete(id: String) {
        stopJob(id)
        done.remove(id)
        playhead.remove(id)
        bytes.remove(id)
        LocalTrackMemory.forget(id)
        // The record first: see `DownloadRules.writingInto`.
        records = records.filter { it.id != id }
        scope.launch { runCatching { File(root, id).deleteRecursively() } }
        save(force = true)
        onWorkChanged()
        startNext()
    }

    private fun stopJob(id: String) { jobs.remove(id)?.cancel() }

    private fun update(id: String, change: (DownloadRecord) -> DownloadRecord) {
        val i = records.indexOfFirst { it.id == id }
        if (i < 0) return
        val before = records[i]
        val after = change(before)
        if (after == before) return
        records = records.toMutableList().also { it[i] = after }
        save(force = before.state != after.state || after.progress >= 1)
    }

    private fun updateMain(id: String, change: (DownloadRecord) -> DownloadRecord) {
        Session.main.post { update(id, change) }
    }

    private fun save(force: Boolean) {
        val now = System.currentTimeMillis()
        if (!force && now - lastIndexSave < 5000) return
        lastIndexSave = now
        val text = DownloadRecord.listJson(records)
        val file = File(root, "index.json")
        scope.launch(saver) { runCatching { Atomic.write(file, text) } }
    }
}
