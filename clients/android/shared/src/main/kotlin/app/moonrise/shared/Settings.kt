package app.moonrise.shared

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.moonrise.core.Item
import app.moonrise.core.LibraryFilter
import app.moonrise.core.LibraryView
import app.moonrise.core.Quality
import app.moonrise.core.SegmentKind
import app.moonrise.core.SkipMode
import app.moonrise.core.UserData
import app.moonrise.core.marked

private fun onMobileData(): Boolean = app.moonrise.shared.Network.cellular

/**
 * This device's settings (the Apple TV app's Settings, kept on the device): playback, audio and
 * subtitle preferences, and the Library's view and filters. Compose state, so screens follow.
 */
object AppSettings {
    private lateinit var app: Context
    private fun prefs() = app.getSharedPreferences("settings", Context.MODE_PRIVATE)

    var streamingQuality by mutableStateOf(Quality.original.label)
        private set
    var skipBack by mutableIntStateOf(10)
        private set
    var skipForward by mutableIntStateOf(10)
        private set
    /** "" plays the title's own default track (the Apple apps' "Original"). */
    var audioLanguage by mutableStateOf("")
        private set
    var subtitleLanguage by mutableStateOf("en")
        private set
    var autoSubtitles by mutableStateOf(false)
        private set
    /** The subtitle picked last in the player was SDH: such a track is preferred in that language. */
    var preferSDHSubtitles by mutableStateOf(false)
        private set
    /** "small", "medium", "large" or "huge" (`SubtitleSize`). */
    var subtitleSize by mutableStateOf("medium")
        private set
    var subtitleBackground by mutableStateOf(false)
        private set
    /** Play the next episode (across seasons) when one ends. */
    var autoplayNext by mutableStateOf(true)
        private set
    /** The quality a download starts at (`downloadQuality`), on Wi-Fi and on mobile data. */
    var downloadQualityWiFi by mutableStateOf("1080p · 10 Mbps")
        private set
    var downloadQualityCellular by mutableStateOf("720p · 4 Mbps")
        private set
    var allowCellularDownloads by mutableStateOf(false)
        private set
    /** What the player does on entering each kind of segment (unset: the kind's default). */
    val skipModes = mutableStateMapOf<SegmentKind, SkipMode>()
    /** Seconds the auto-skip counts down first (0: skips at once). */
    var autoSkipDelay by mutableIntStateOf(3)
        private set
    /** Use the community databases' segments (TheIntroDB, IntroDB), which the server looks up (`community=0` when off). */
    var onlineSegments by mutableStateOf(true)
        private set

    fun skipMode(kind: SegmentKind): SkipMode = skipModes[kind] ?: kind.defaultMode

    /** The subtitle size's scale (`SubtitleSize.scale`). */
    val subtitleScale: Float
        get() = when (subtitleSize) { "small" -> 0.8f; "large" -> 1.25f; "huge" -> 1.55f; else -> 1f }

    fun init(context: Context) {
        app = context.applicationContext
        val p = prefs()
        streamingQuality = p.getString("streamingQuality", null) ?: Quality.original.label
        skipBack = p.getInt("skipBack", 10)
        skipForward = p.getInt("skipFwd", 10)
        audioLanguage = p.getString("audioLang", null) ?: ""
        subtitleLanguage = p.getString("subLang", null) ?: "en"
        autoSubtitles = p.getBoolean("autoSubs", false)
        preferSDHSubtitles = p.getBoolean("subSDH", false)
        subtitleSize = p.getString("subSize", null) ?: "medium"
        subtitleBackground = p.getBoolean("subBg", false)
        autoplayNext = p.getBoolean("autoplayNext", true)
        downloadQualityWiFi = p.getString("downloadQuality", null) ?: "1080p · 10 Mbps"
        downloadQualityCellular = p.getString("downloadQualityCell", null) ?: "720p · 4 Mbps"
        allowCellularDownloads = p.getBoolean("cellDl", false)
        autoSkipDelay = p.getInt("autoSkipDelay", 3)
        onlineSegments = p.getBoolean("onlineSegments", true)
        for (k in SegmentKind.entries) SkipMode.of(p.getString("skipMode.${k.id}", null))?.let { skipModes[k] = it }
    }

    fun chooseStreamingQuality(v: String) { streamingQuality = v; prefs().edit().putString("streamingQuality", v).apply() }
    fun chooseSkipBack(v: Int) { skipBack = v; prefs().edit().putInt("skipBack", v).apply() }
    fun chooseSkipForward(v: Int) { skipForward = v; prefs().edit().putInt("skipFwd", v).apply() }
    fun chooseAudioLanguage(v: String) { audioLanguage = v; prefs().edit().putString("audioLang", v).apply() }
    fun chooseSubtitleLanguage(v: String) { subtitleLanguage = v; prefs().edit().putString("subLang", v).apply() }
    fun chooseAutoSubtitles(v: Boolean) { autoSubtitles = v; prefs().edit().putBoolean("autoSubs", v).apply() }
    fun choosePreferSDH(v: Boolean) { preferSDHSubtitles = v; prefs().edit().putBoolean("subSDH", v).apply() }
    fun chooseSubtitleSize(v: String) { subtitleSize = v; prefs().edit().putString("subSize", v).apply() }
    fun chooseSubtitleBackground(v: Boolean) { subtitleBackground = v; prefs().edit().putBoolean("subBg", v).apply() }
    fun chooseAutoplayNext(v: Boolean) { autoplayNext = v; prefs().edit().putBoolean("autoplayNext", v).apply() }
    fun chooseDownloadQualityWiFi(v: String) { downloadQualityWiFi = v; prefs().edit().putString("downloadQuality", v).apply() }
    fun chooseDownloadQualityCellular(v: String) { downloadQualityCellular = v; prefs().edit().putString("downloadQualityCell", v).apply() }
    fun chooseAllowCellularDownloads(v: Boolean) { allowCellularDownloads = v; prefs().edit().putBoolean("cellDl", v).apply() }
    /** The quality a download starts at on this network. */
    val downloadQuality: String get() = if (onMobileData()) downloadQualityCellular else downloadQualityWiFi
    fun chooseAutoSkipDelay(v: Int) { autoSkipDelay = v; prefs().edit().putInt("autoSkipDelay", v).apply() }
    fun chooseOnlineSegments(v: Boolean) { onlineSegments = v; prefs().edit().putBoolean("onlineSegments", v).apply() }
    fun chooseSkipMode(kind: SegmentKind, mode: SkipMode) { skipModes[kind] = mode; prefs().edit().putString("skipMode.${kind.id}", mode.id).apply() }

    /** The Library's view in use (Movies or Shows). */
    var libraryView: LibraryView
        get() = LibraryView.of(prefs().getString("library.selected", null))
        set(v) { prefs().edit().putString("library.selected", v.id).apply() }

    /** A view's filters and sort, as left last. */
    fun filter(view: LibraryView): LibraryFilter =
        prefs().getString("library.filter.${view.id}", null)?.let { LibraryFilter.fromJson(it) } ?: LibraryFilter()

    fun saveFilter(view: LibraryView, f: LibraryFilter) {
        prefs().edit().putString("library.filter.${view.id}", f.toJson()).apply()
    }
}

/**
 * Marks made here (Mark as Watched / Unwatched), shown at once on every card of the title
 * (`WatchState`, Apple apps): a mark applies to item values with the watch state it was made
 * on, and stops applying once a fresh one, with the server's new state, replaces them.
 */
object WatchMarks {
    private data class Mark(val before: UserData?, val played: Boolean)
    private val marks = mutableStateMapOf<String, Mark>()
    /** Bumped by every mark the server took: screens reload what they show. */
    var generation by mutableIntStateOf(0)
        private set

    fun current(item: Item): Item {
        val m = marks[item.id] ?: return item
        return if (m.before == item.user) item.marked(m.played) else item
    }

    /** Marks `item` (a show or season: all its episodes) on the server. False: refused, undone. */
    suspend fun set(item: Item, played: Boolean): Boolean {
        val api = Session.api ?: return false
        marks[item.id] = Mark(item.user, played)
        return try {
            api.setPlayed(item.id, played)
            Log.i("Moonrise") { "watch: ${item.id} marked ${if (played) "watched" else "unwatched"}" }
            generation++
            WatchHooks.changed("marked")
            true
        } catch (e: Exception) {
            Log.w("Moonrise", "watch: ${item.id} mark failed: ${e.message}")
            marks.remove(item.id)
            false
        }
    }

    /** Remove from Continue Watching, on every device. False: the server refused. */
    suspend fun removeFromContinue(item: Item): Boolean {
        val api = Session.api ?: return false
        return try {
            api.removeFromContinue(item.id)
            Log.i("Moonrise") { "watch: ${item.id} removed from continue watching" }
            generation++
            WatchHooks.changed("removed")
            true
        } catch (e: Exception) {
            Log.w("Moonrise", "watch: ${item.id} remove failed: ${e.message}")
            false
        }
    }
}

/** Versions picked on this device since launch (item id → file id), ahead of the server's answer. */
object VersionPicks {
    val picks = mutableStateMapOf<String, String>()
}

/**
 * Whether the device has a network, and a counter bumped each time one comes up: screens showing
 * an error load again then (after the TV wakes, the first load can fail before Wi-Fi is back).
 */
object Network {
    var generation by mutableIntStateOf(0)
        private set
    var online by mutableStateOf(true)
        private set
    /** On mobile data now (`NetworkType.isCellular`). */
    var cellular by mutableStateOf(false)
        private set

    private fun readCellular(cm: ConnectivityManager) =
        cm.getNetworkCapabilities(cm.activeNetwork)?.hasTransport(android.net.NetworkCapabilities.TRANSPORT_CELLULAR) == true

    fun init(context: Context) {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        online = cm.activeNetwork != null
        cellular = readCellular(cm)
        cm.registerDefaultNetworkCallback(object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                Log.i("Moonrise") { "network available" }
                Session.main.post { online = true; cellular = readCellular(cm); generation++; app.moonrise.shared.downloads.ServerReach.nudge() }
            }

            override fun onLost(network: Network) {
                Log.i("Moonrise") { "network lost" }
                Session.main.post { online = false; cellular = false; app.moonrise.shared.downloads.ServerReach.nudge() }
            }
        })
    }
}
