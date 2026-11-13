package app.moonrise.shared

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.moonrise.core.ClientRules
import app.moonrise.core.Item
import app.moonrise.core.MoonriseHttpException
import kotlinx.coroutines.delay

// Loading what screens show, and what they say when it fails: the same in every app.

/**
 * What went wrong, in the app's words: the server's sign-in refused, or not reachable (and then
 * why, when the route check knows: offline, Tailscale off or signed out, the server down).
 */
fun errorText(e: Throwable): String = when {
    e is MoonriseHttpException && e.code == 401 -> "The Moonrise server doesn't accept this sign-in any more. Sign in again."
    e is MoonriseHttpException -> e.message ?: "The Moonrise server returned ${e.code}."
    else -> connectionProblemText() ?: "Can't reach the server. Check that it's running and on this network."
}

/** The route check's reason the server can't be reached, as a sentence or two (null: no reason known). */
fun connectionProblemText(tv: Boolean = false): String? {
    val p = app.moonrise.shared.downloads.ServerReach.problem ?: return null
    val name = app.moonrise.shared.downloads.ServerReach.serverName
    val title = app.moonrise.core.RouteRules.title(p, name)
    return (if (title.endsWith("…")) "$title " else "$title. ") +
        app.moonrise.core.RouteRules.message(p, name, Network.cellular, app.moonrise.shared.remote.Tailscale.isSupported, tv)
}

/**
 * While a screen shows an error, it tries again by itself: after 5 s, then less often, up to every
 * 30 s (after the TV wakes, the network or the server can take a while to answer). Screens keep
 * their error while a retry runs and clear it only when a load works, so the waits keep growing.
 */
@Composable
fun AutoRetry(error: String?, busy: () -> Boolean = { false }, retry: () -> Unit) {
    LaunchedEffect(error) {
        if (error == null) return@LaunchedEffect
        var wait = ClientRules.FIRST_RETRY_MS
        while (true) {
            delay(wait)
            // (A try still under way finishes first: it isn't cut short by the next.)
            while (busy()) delay(500)
            Log.i("Moonrise") { "retrying after an error" }
            retry()
            wait = ClientRules.nextRetryMs(wait)
        }
    }
}

/** The page's data: the show (or film), its seasons and episodes, and the episode it's about. */
class DetailsState(val opened: Item, private val offlinePage: ((String) -> app.moonrise.core.OfflineTitlePage?)? = null) {
    /** The page is about a show (opened from it, one of its seasons or one of its episodes). */
    val isShow = opened.isSeries || opened.isSeason || opened.isEpisode
    private val showId = if (opened.isEpisode || opened.isSeason) opened.seriesId ?: opened.id else opened.id
    private val preselectedSeason = if (opened.isSeason) opened.id else null

    var show by mutableStateOf<Item?>(if (opened.isSeries || !isShow) opened else null)
    var seasons by mutableStateOf<List<Item>>(emptyList())
    var selectedSeason by mutableStateOf<Item?>(null)
    var episodes by mutableStateOf<List<Item>>(emptyList())
    /** The episode the page is about (Play's target): the opened one, else the one to continue with. */
    var current by mutableStateOf<Item?>(if (opened.isEpisode) opened else null)
    /** Full items (with their files) of episodes from a list that has several versions. */
    val full = mutableStateMapOf<String, Item>()
    var error by mutableStateOf<String?>(null)
    var unauthorized by mutableStateOf(false)
    /** No server: the page is what the downloads saved (`ItemDetailView.offline`). */
    var offline by mutableStateOf<app.moonrise.core.OfflineTitlePage?>(null)
    var loaded by mutableStateOf(false)
    /** A first load is under way. */
    var loading by mutableStateOf(false)
    /** Where focus goes when the page comes back from the player: "play" or "ep:<id>". */
    var returnTo: String? = null

    /**
     * The player went on to another episode (Next Episode, its Episodes panel): the page is about
     * that one now, and Back from the player lands where this one would have (Play, or the episode
     * in the list).
     */
    fun returnToEpisode(ep: Item) {
        if (!isShow) return
        current = ep
        seasons.firstOrNull { it.id == ep.seasonId }?.let { selectedSeason = it }
        if (returnTo?.startsWith("ep:") == true) returnTo = "ep:${ep.id}"
    }
    /** Bumped when the episodes change, so the row scrolls to the current one. */
    var episodesVersion by mutableIntStateOf(0)

    /** What Play plays: the film, or the show's current episode (with its files when it has versions). */
    val focus: Item?
        get() {
            val f = (if (isShow) current else show) ?: return null
            return WatchMarks.current(full[f.id] ?: f)
        }

    private fun fail(e: Exception) {
        Log.w("Moonrise", "title page failed: ${e.message}")
        unauthorized = e is MoonriseHttpException && e.code == 401
        error = errorText(e)
    }

    /** The server answered: the downloads' page (if any) gives way to the server's. */
    private fun online() { if (offline != null) { offline = null; loaded = false } }

    /** The page from what this title's downloads saved. False when nothing of it plays from the device. */
    private fun loadOffline(): Boolean {
        val page = offlinePage?.invoke(showId) ?: return false
        offline = page
        error = null
        unauthorized = false
        show = page.item
        if (isShow) {
            seasons = page.seasons
            // The episode it opened on (the download playing), else the first downloaded one.
            val at = page.season(current)
            selectedSeason = at ?: preselectedSeason?.let { sid -> page.seasons.firstOrNull { it.id == sid } } ?: page.seasons.firstOrNull()
            episodes = page.episodes(selectedSeason)
            val c = current
            current = if (at == null) episodes.firstOrNull { it.user?.played != true } ?: episodes.firstOrNull()
            else c?.let { cur -> episodes.firstOrNull { it.id == cur.id } } ?: c
            episodesVersion++
        }
        loaded = true
        return true
    }

    suspend fun load() {
        val api = Session.api ?: return
        // (An error stays up until a load works: the automatic retries keep backing off.)
        loading = true
        try {
            // No server: the downloads' page at once, rather than after the requests time out.
            if (offlinePage != null && !app.moonrise.shared.downloads.ServerReach.reachable && loadOffline()) return
            show = api.item(showId)
            if (isShow) {
                seasons = api.seasons(showId)
                if (current == null) {
                    val cont = api.continueEpisode(showId)
                    if (preselectedSeason == null || cont?.seasonId == preselectedSeason) current = cont
                }
                selectedSeason = seasons.firstOrNull { it.id == (preselectedSeason ?: current?.seasonId) }
                    ?: seasons.firstOrNull { it.seasonNo == current?.seasonNo } ?: seasons.firstOrNull()
                loadEpisodes(selectFirst = current == null)
            }
            online()
            loaded = true
            error = null
            loadFull()
        } catch (e: Exception) {
            // The server went away (or doesn't answer): the downloads' page, quietly.
            if (!loadOffline()) fail(e)
        } finally {
            loading = false
        }
    }

    /** After playback or a mark: the title, its episodes and seasons again. */
    suspend fun refresh() {
        val api = Session.api ?: return
        if (offline != null && offlinePage != null && !app.moonrise.shared.downloads.ServerReach.reachable) { loadOffline(); return }
        try {
            show = api.item(showId)
            online()
            if (isShow) {
                seasons = api.seasons(showId)
                selectedSeason = seasons.firstOrNull { it.id == selectedSeason?.id } ?: selectedSeason
                loadEpisodes(selectFirst = false)
            }
            full.clear()
            loadFull()
            error = null
        } catch (e: Exception) {
            if (offlinePage != null && loadOffline()) return
            if (!loaded) fail(e) else Log.w("Moonrise", "title page refresh failed: ${e.message}")
        }
    }

    suspend fun loadEpisodes(selectFirst: Boolean) {
        val api = Session.api ?: return
        val season = selectedSeason ?: return
        val page = offline
        episodes = if (page != null) page.episodes(season) else api.episodes(showId, season.id)
        val c = current
        current = if (selectFirst) episodes.firstOrNull { it.user?.played != true } ?: episodes.firstOrNull()
        else c?.let { cur -> episodes.firstOrNull { it.id == cur.id } } ?: c
        episodesVersion++
    }

    suspend fun selectSeason(s: Item) {
        if (s.id == selectedSeason?.id) return
        selectedSeason = s
        runCatching { loadEpisodes(selectFirst = true) }.onFailure { Log.w("Moonrise", "episodes failed: ${it.message}") }
        loadFull()
    }

    /** The current episode's files, when its list says it has several versions (`VersionDetails`). */
    suspend fun loadFull() {
        val f = (if (isShow) current else show) ?: return
        if (f.hasVersions) { full[f.id] = f; return }
        if ((f.versions ?: 0) <= 1 || full.containsKey(f.id) || offline != null) return
        runCatching { Session.api?.item(f.id) }.getOrNull()?.let { full[f.id] = it }
    }
}
