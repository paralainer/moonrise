package app.moonrise.tv

import android.content.BroadcastReceiver
import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import androidx.tvprovider.media.tv.TvContractCompat
import androidx.tvprovider.media.tv.WatchNextProgram
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import app.moonrise.core.MoonriseHttpException
import app.moonrise.core.WatchNextEntry
import app.moonrise.core.WatchNextKind
import app.moonrise.core.WatchNextOp
import app.moonrise.core.WatchNextRow
import app.moonrise.core.WatchNextRules
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.util.concurrent.TimeUnit
import app.moonrise.shared.Log
import app.moonrise.shared.Session

/**
 * The TV home screen's Watch Next row ("Continue watching" on Google TV): the titles the Home tab
 * shows in Continue Watching, written to the system's TV provider (`WatchNextRules` decides
 * what). Kept in step with the server when the app starts, when playback stops, after a mark,
 * and every 30 minutes in the background (watching on another device shows up here too). Signing
 * out removes every title. Selecting one opens the app and plays it where it was ([DeepLinks]).
 */
object WatchNext {
    private const val TAG = "MoonriseWatchNext"
    private const val WORK = "watch-next"
    private lateinit var app: Context
    private val mutex = Mutex()
    private var pending: Job? = null

    /** The device has the TV provider (Android TV, Google TV). */
    private val supported: Boolean
        get() = app.packageManager.hasSystemFeature(PackageManager.FEATURE_LEANBACK)

    fun init(context: Context) {
        app = context.applicationContext
        if (!supported) return
        val work = PeriodicWorkRequestBuilder<WatchNextWorker>(30, TimeUnit.MINUTES)
            .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
            .build()
        runCatching { WorkManager.getInstance(app).enqueueUniquePeriodicWork(WORK, ExistingPeriodicWorkPolicy.KEEP, work) }
            .onFailure { Log.w(TAG, "periodic sync not scheduled: ${it.message}") }
        syncSoon("app start", 3000)
    }

    /** A sync shortly (several asks close together make one: a stop report lands first). */
    fun syncSoon(why: String, delayMs: Long = 1500) {
        if (!::app.isInitialized || !supported) return
        pending?.cancel()
        pending = Session.background.launch {
            delay(delayMs)
            sync(why)
        }
    }

    /** Removes every title (signed out, or another server). */
    fun clear(why: String) {
        if (!::app.isInitialized || !supported) return
        pending?.cancel()
        Session.background.launch { mutex.withLock { clearRows(why) } }
    }

    /** Home was just loaded (`limit` = [WatchNextRules.MAX]): the row follows it, without asking the server again. */
    fun syncFrom(home: app.moonrise.core.Home, why: String) {
        if (!::app.isInitialized || !supported) return
        pending?.cancel()
        pending = Session.background.launch { sync(why, home) }
    }

    suspend fun sync(why: String, loaded: app.moonrise.core.Home? = null) {
        if (!::app.isInitialized || !supported) return
        mutex.withLock {
            val api = Session.api ?: return clearRows("$why, signed out")
            val home = loaded ?: try {
                api.homeWatching()
            } catch (e: MoonriseHttpException) {
                if (e.code == 401) clearRows("$why, sign-in refused") else Log.w(TAG, "sync ($why): ${e.message}")
                return
            } catch (e: Exception) {
                // Kept as they are until the server answers.
                Log.w(TAG, "sync ($why): server not reached: ${e.message}")
                return
            }
            if (Session.api !== api) return
            val rows = runCatching { readRows() }.getOrElse { Log.w(TAG, "can't read the row: ${it.message}"); return }
            val desired = WatchNextRules.entries(home, rows.map { it.second }, System.currentTimeMillis())
            val ops = WatchNextRules.plan(desired, rows.map { it.second })
            var failed = 0
            for (op in ops) runCatching { apply(op) }.onFailure { failed++; Log.w(TAG, "$op failed: ${it.message}") }
            Log.i(TAG) { "sync ($why): ${desired.size} titles (${desired.count { it.kind == WatchNextKind.Continue }} continue, " +
                "${desired.count { it.kind == WatchNextKind.Next }} next); +${ops.count { it is WatchNextOp.Insert }} " +
                "~${ops.count { it is WatchNextOp.Update }} -${ops.count { it is WatchNextOp.Delete }}${if (failed > 0) ", $failed failed" else ""}" }
        }
    }

    /** Logs the row as the provider holds it (debug seam `--ez watchnext true`; adb can't read it). */
    fun dump() {
        if (!::app.isInitialized || !supported) return
        Session.background.launch {
            mutex.withLock {
                val rows = runCatching { readRows() }.getOrElse { Log.w(TAG, "dump: ${it.message}"); return@withLock }
                Log.i(TAG) { "dump: ${rows.size} rows" }
                for ((p, _) in rows) Log.i(TAG) {
                    "row ${p.id}: ${p.internalProviderId} type=${p.type} next=${p.watchNextType} \"${p.title}\" ep=\"${p.episodeTitle}\" " +
                        "S${p.seasonNumber}E${p.episodeNumber} at ${p.lastPlaybackPositionMillis}/${p.durationMillis} ms " +
                        "engaged=${p.lastEngagementTimeUtcMillis} browsable=${p.isBrowsable} art=${p.posterArtUri} " +
                        "aspect=${p.posterArtAspectRatio} intent=${p.intentUri}"
                }
            }
        }
    }

    private fun readRows(): List<Pair<WatchNextProgram, WatchNextRow>> {
        val out = mutableListOf<Pair<WatchNextProgram, WatchNextRow>>()
        app.contentResolver.query(TvContractCompat.WatchNextPrograms.CONTENT_URI, WatchNextProgram.PROJECTION, null, null, null)?.use { c ->
            while (c.moveToNext()) {
                val p = WatchNextProgram.fromCursor(c)
                val data = p.internalProviderDataByteArray?.let { String(it, Charsets.UTF_8) }
                out += p to WatchNextRow(p.id, p.internalProviderId, p.lastEngagementTimeUtcMillis, p.isBrowsable, data)
            }
        }
        return out
    }

    private fun apply(op: WatchNextOp) {
        val cr = app.contentResolver
        when (op) {
            is WatchNextOp.Insert -> cr.insert(TvContractCompat.WatchNextPrograms.CONTENT_URI, program(op.entry).toContentValues())
            is WatchNextOp.Update -> cr.update(TvContractCompat.buildWatchNextProgramUri(op.rowId), program(op.entry).toContentValues(), null, null)
            is WatchNextOp.Delete -> cr.delete(TvContractCompat.buildWatchNextProgramUri(op.rowId), null, null)
        }
    }

    private fun clearRows(why: String) {
        val rows = runCatching { readRows() }.getOrElse { return }
        for ((p, _) in rows) runCatching { app.contentResolver.delete(ContentUris.withAppendedId(TvContractCompat.WatchNextPrograms.CONTENT_URI, p.id), null, null) }
        if (rows.isNotEmpty()) Log.i(TAG) { "cleared ${rows.size} titles ($why)" }
    }

    private fun program(e: WatchNextEntry): WatchNextProgram {
        val b = WatchNextProgram.Builder()
            .setType(if (e.isEpisode) TvContractCompat.WatchNextPrograms.TYPE_TV_EPISODE else TvContractCompat.WatchNextPrograms.TYPE_MOVIE)
            .setWatchNextType(
                if (e.kind == WatchNextKind.Continue) TvContractCompat.WatchNextPrograms.WATCH_NEXT_TYPE_CONTINUE
                else TvContractCompat.WatchNextPrograms.WATCH_NEXT_TYPE_NEXT,
            )
            .setLastEngagementTimeUtcMillis(e.engagementMs)
            .setTitle(e.title)
            .setInternalProviderId(e.itemId)
            .setContentId(e.itemId)
            .setInternalProviderData(e.fingerprint.toByteArray(Charsets.UTF_8))
            .setIntentUri(Uri.parse(DeepLinks.playUri(app, e.itemId)))
        e.episodeTitle?.let { b.setEpisodeTitle(it) }
        e.seasonNumber?.let { b.setSeasonNumber(it) }
        e.episodeNumber?.let { b.setEpisodeNumber(it) }
        e.description?.let { b.setDescription(it) }
        e.year?.let { b.setReleaseDate(it.toString()) }
        if (e.durationMs > 0) b.setDurationMillis(e.durationMs.coerceAtMost(Int.MAX_VALUE.toLong()).toInt())
        if (e.positionMs > 0) b.setLastPlaybackPositionMillis(e.positionMs.coerceAtMost(Int.MAX_VALUE.toLong()).toInt())
        val api = Session.api
        val image = e.image
        if (image != null && api != null) {
            b.setPosterArtUri(Uri.parse(api.imageUrl(image, if (e.posterImage) 400 else 720)))
            b.setPosterArtAspectRatio(
                if (e.posterImage) TvContractCompat.PreviewPrograms.ASPECT_RATIO_2_3 else TvContractCompat.PreviewPrograms.ASPECT_RATIO_16_9,
            )
        }
        return b.build()
    }
}

/** The periodic sync (WorkManager keeps it across reboots). */
class WatchNextWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        WatchNext.sync("periodic")
        return Result.success()
    }
}

/**
 * The launcher asks a newly installed app to fill its rows (`INITIALIZE_PROGRAMS`), and tells it
 * when the viewer removed one of its titles from the row (kept removed by the next sync).
 */
class WatchNextReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        Log.i("MoonriseWatchNext") { "launcher: ${intent.action}" }
        if (intent.action != TvContractCompat.ACTION_INITIALIZE_PROGRAMS) return
        val done = goAsync()
        Session.background.launch {
            try { WatchNext.sync("launcher asked") } finally { done.finish() }
        }
    }
}

/**
 * Links into the app from outside it: the Watch Next row's titles. `moonrise://play/<item id>`
 * plays the title from where it was (its page underneath, so Back lands there), once signed in.
 */
object DeepLinks {
    fun playUri(context: Context, itemId: String): String =
        Intent(Intent.ACTION_VIEW, Uri.parse("moonrise://play/${Uri.encode(itemId)}"))
            .setClassName(context.packageName, MainActivity::class.java.name)
            .toUri(Intent.URI_INTENT_SCHEME)

    /** The title an intent asks to play, if it is such a link. */
    fun take(intent: Intent?): String? {
        val data = intent?.data ?: return null
        if (data.scheme != "moonrise" || data.host != "play") return null
        val id = data.pathSegments.firstOrNull()?.takeIf { it.isNotBlank() } ?: return null
        intent.data = null
        return id
    }
}
