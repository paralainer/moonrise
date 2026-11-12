package app.moonrise.shared.downloads

import android.content.Context
import app.moonrise.core.LocalReport
import app.moonrise.core.MoonriseHttpException
import app.moonrise.core.TrackChoice
import app.moonrise.core.TrackMemory
import app.moonrise.shared.Log
import app.moonrise.shared.Session
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

/**
 * Watch progress of downloads played on the device, for the server's watch state (`LocalReports`):
 * kept on disk and sent (`POST /v1/sessions/local`) when the server can be reached, so playback
 * offline counts once the phone is back. Only the latest progress of a viewing is kept while it
 * waits. Changed on the main thread.
 */
object LocalReports {
    private lateinit var file: File
    private var pending: List<LocalReport> = emptyList()
    private var sending = false
    private var again = false

    /** Test seam (debug builds): reports are dropped, so scripted runs leave no watch state. */
    var disabled = false

    fun init(context: Context) {
        file = File(context.filesDir, "local-reports.json")
        pending = runCatching { LocalReport.listFrom(file.readText()) }.getOrNull().orEmpty()
    }

    fun hasPending(item: String): Boolean = pending.any { it.item == item }

    fun add(r: LocalReport) {
        if (disabled) return
        pending = LocalReport.added(pending, r)
        save()
        flushSoon()
    }

    private fun save() {
        val text = LocalReport.listJson(pending)
        Session.background.launch { runCatching { Atomic.write(file, text) } }
    }

    fun flushSoon() { Session.ui.launch { flush() } }

    /** Send what is waiting, when the server can be reached. Others' reports (another server) are dropped. */
    private suspend fun flush() {
        if (pending.isEmpty()) return
        val api = Session.api ?: return
        val server = ServerReach.serverId ?: return
        if (sending) { again = true; return }
        sending = true
        try {
            do {
                again = false
                if (pending.any { it.serverId != server }) { pending = pending.filter { it.serverId == server }; save() }
                if (pending.isEmpty() || !ServerReach.reachable) return
                val batch = pending.take(LocalReport.BATCH)
                try {
                    withContext(Dispatchers.IO) { api.reportLocal(batch) }
                    pending = pending - batch.toSet()
                    save()
                    Log.i("MoonriseLocalReports") { "sent ${batch.size}: ${batch.joinToString { "${it.event}@${it.position.toInt()}" }}, ${pending.size} waiting" }
                } catch (e: Exception) {
                    if (e is kotlinx.coroutines.CancellationException) throw e
                    Log.i("MoonriseLocalReports") { "${batch.size} not sent (${e.message}), kept" }
                    val code = (e as? MoonriseHttpException)?.code
                    if (LocalReport.dropOnFailure(code)) { pending = pending - batch.toSet(); save() }
                    return
                }
            } while (again)
        } finally {
            sending = false
        }
    }
}

/** The tracks last played in each download, on this device: a download starts with them again. */
object LocalTrackMemory {
    private lateinit var app: Context
    private fun prefs() = app.getSharedPreferences("downloadTracks", Context.MODE_PRIVATE)

    fun init(context: Context) { app = context.applicationContext }

    fun load(download: String): TrackMemory? =
        prefs().getString(download, null)?.let { TrackChoice.fromJson(it) }?.memory

    fun save(choice: TrackChoice, download: String) {
        prefs().edit().putString(download, choice.toJson()).apply()
    }

    fun forget(download: String) { prefs().edit().remove(download).apply() }
}

/** Resume positions of downloads on this device (`ProgressStore`). */
object ProgressStore {
    private lateinit var app: Context
    private fun prefs() = app.getSharedPreferences("downloadProgress", Context.MODE_PRIVATE)

    fun init(context: Context) { app = context.applicationContext }

    fun position(itemId: String): Double? = prefs().getFloat(itemId, -1f).takeIf { it >= 0 }?.toDouble()

    fun save(itemId: String, position: Double) { prefs().edit().putFloat(itemId, position.toFloat()).apply() }
}

/** Writes a file whole: a reader never sees half of it. */
internal object Atomic {
    @Synchronized
    fun write(file: File, text: String) {
        file.parentFile?.mkdirs()
        val tmp = File(file.parentFile, file.name + ".tmp")
        tmp.writeText(text)
        if (!tmp.renameTo(file)) { file.delete(); tmp.renameTo(file) }
    }
}
