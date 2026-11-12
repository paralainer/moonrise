package app.moonrise.tv.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Text
import app.moonrise.core.Item
import app.moonrise.core.SearchStart
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import app.moonrise.shared.WatchMarks
import app.moonrise.shared.errorText
import app.moonrise.shared.AutoRetry
import app.moonrise.shared.Log
import app.moonrise.shared.Session
import app.moonrise.shared.Network

/** The Search tab's query and results, kept while a title page is on top. */
class SearchState {
    var query by mutableStateOf("")
    var results by mutableStateOf<List<Item>>(emptyList())
    var searched by mutableStateOf("")
    var searching by mutableStateOf(false)
    var error by mutableStateOf<String?>(null)
    /** Recent and Suggestions, shown while nothing is typed (null: an older server, or not yet known). */
    var start by mutableStateOf<SearchStart?>(null)
    var reloads by mutableIntStateOf(0)
    val column = LazyListState()
    /** The results are older than the last playback or mark: searched again, in place. */
    var stale = false
    /** The card opened last: focused again when Search comes back. */
    var opened: Opened? = null
    /** "<row>/<item>" of the card to focus now, carrying [restore] until it has focus. */
    var restoreKey by mutableStateOf<String?>(null)
    val restore = FocusRequester()
    /** The card that has focus ("<row>/<item>"), null when focus is elsewhere. */
    var focusedKey: String? = null
    /** Each row's card focused last (coming back into a row lands on it). */
    val lastInRow = mutableStateMapOf<String, String>()
    private val rowStates = mutableMapOf<String, LazyListState>()
    fun rowState(row: String): LazyListState = rowStates.getOrPut("$row-$searched") { LazyListState() }

    data class Opened(val row: String, val itemId: String, val index: Int)

    /** A title was opened from Search: first in Recent at once, and on the server for every device. */
    fun recordOpened(item: Item) {
        if (start == null) return
        start = start?.opened(item)
        Session.ui.launch { runCatching { Session.api?.recordSearchOpened(item.id) } }
    }

    fun clearRecent() {
        lastInRow.remove("recent")
        start = start?.recentCleared()
        Session.ui.launch { runCatching { Session.api?.clearSearchRecent() } }
    }

    fun clear() {
        query = ""; results = emptyList(); searched = ""; error = null; opened = null; restoreKey = null
        lastInRow.clear(); rowStates.clear(); stale = false; start = null
        runCatching { column.requestScrollToItem(0) }
    }
    fun refreshSoon() { stale = true }

    /** The result rows as shown: Shows, Movies, then Episodes. */
    fun rows(): List<Pair<String, List<Item>>> {
        val s = start
        if (s != null && SearchStart.shows(query)) return listOf("recent" to s.recent, "suggestions" to s.suggestions).filter { it.second.isNotEmpty() }
        return resultRows()
    }
    private fun resultRows(): List<Pair<String, List<Item>>> = listOf(
        "shows" to results.filter { it.isSeries },
        "movies" to results.filter { it.isMovie },
        "episodes" to results.filter { it.isEpisode },
    ).filter { it.second.isNotEmpty() }
}

private val ROW_TITLES = mapOf("recent" to "Recent", "suggestions" to "Suggestions", "shows" to "Shows", "movies" to "Movies", "episodes" to "Episodes")

/**
 * Search (`SearchView`): the field, then the results as Home's shelves: Shows, Movies, then
 * Episodes (shows and episodes can share a title, so the kinds stay apart). Back from a title
 * focuses the card it was opened from.
 */
@Composable
fun SearchScreen(
    state: SearchState, open: (Item) -> Unit, atTop: (Boolean) -> Unit,
    entry: androidx.compose.ui.focus.FocusRequester? = null, main: MainState? = null,
) {
    var menuFor by remember { mutableStateOf<Item?>(null) }
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    LaunchedEffect(state.column) { snapshotFlow { state.column.firstVisibleItemIndex == 0 && state.column.firstVisibleItemScrollOffset < 40 }.collect(atTop) }
    LaunchedEffect(state.query, state.reloads, WatchMarks.generation) {
        val q = state.query.trim()
        if (q.length < 2) { state.results = emptyList(); state.searched = ""; state.error = null; return@LaunchedEffect }
        if (q == state.searched && state.error == null && !state.stale) return@LaunchedEffect
        // (Searching the same words again, after playback: at once, and the rows stay as they are.)
        if (q != state.searched) delay(350)
        val api = Session.api ?: return@LaunchedEffect
        state.searching = true
        try {
            val r = api.search(q)
            if (q != state.searched) { state.lastInRow.clear(); state.opened = null }
            state.results = r.titles + r.episodes
            state.searched = q
            state.stale = false
            state.error = null
        } catch (e: Exception) {
            Log.w("Moonrise", "search failed: ${e.message}")
            state.error = errorText(e)
        } finally {
            state.searching = false
        }
    }
    // Each time Search shows, and after a title's watched state changed: what to offer before typing.
    LaunchedEffect(Session.api, WatchMarks.generation) {
        val api = Session.api ?: return@LaunchedEffect
        runCatching { api.searchStart() }.onSuccess { state.start = it }.onFailure { Log.w("Moonrise", "search start failed: ${it.message}") }
    }
    LaunchedEffect(Network.generation) { if (state.error != null) state.reloads++ }
    AutoRetry(state.error, busy = { state.searching }) { state.reloads++ }
    // Back from a title page: the card it was opened from.
    // (Not when the tab was switched to on the tab bar: focus stays on the bar.)
    LaunchedEffect(Unit) { if (main?.tabSwitched != true) restoreSearchFocus(state) }
    // Down from the tab bar, when the focus search finds nothing below: the field, scrolled back.
    EnterContent(main) {
        runCatching { state.column.scrollToItem(0) }
        entry?.focusSoon(20) ?: false
    }

    val rows = state.rows()
    LazyColumn(
        state = state.column,
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(top = TAB_BAR_SPACE, bottom = 40.dp),
        verticalArrangement = Arrangement.spacedBy(28.dp),
    ) {
        item(key = "field") {
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                TvTextField(
                    label = "Search", value = state.query, onValueChange = { state.query = it },
                    placeholder = "Movies, shows, episodes",
                    focusRequester = entry,
                    modifier = Modifier.widthIn(max = 520.dp).fillMaxWidth().padding(horizontal = Theme.margin),
                )
            }
        }
        // (Keyed by the search, so a new search's rows start at their first result.)
        rows.forEach { (row, items) ->
            item(key = "$row-${state.searched}") {
                ResultRow(state, row, items, open, menu = { menuFor = it }, clear = if (row == "recent") ({
                    // (Focus moves first: the row, with the control that has it, is about to go.)
                    scope.launch { entry?.focusSoon(20); state.clearRecent() }
                }) else null)
            }
        }
        if (state.start?.isEmpty == true && SearchStart.shows(state.query)) {
            item(key = "start-empty") { EmptyState("Search Your Library", "Find movies, shows and episodes by name.", modifier = Modifier.padding(top = 20.dp)) }
        }
        val err = state.error
        when {
            err != null -> item(key = "error") { ErrorPane(err, onRetry = { state.reloads++ }, modifier = Modifier.padding(top = 20.dp)) }
            state.searching && state.results.isEmpty() -> item(key = "searching") {
                Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { Text("Searching…", color = Theme.secondaryText) }
            }
            !state.searching && state.results.isEmpty() && state.searched.isNotEmpty() -> item(key = "none") {
                EmptyState("No Results", "Nothing matched “${state.searched}”.")
            }
        }
    }
    menuFor?.let { WatchMenu(it, onDismiss = { menuFor = null }) }
}

/** A row of results: coming back into it lands on the card focused there last. */
@Composable
private fun ResultRow(state: SearchState, row: String, items: List<Item>, open: (Item) -> Unit, menu: (Item) -> Unit, clear: (() -> Unit)? = null) {
    val last = remember { FocusRequester() }
    val lastId = state.lastInRow[row]?.takeIf { id -> items.any { it.id == id } } ?: items.firstOrNull()?.id
    val restoring = state.restoreKey?.startsWith("$row/") == true
    val rowState = state.rowState(row)
    Shelf(
        ROW_TITLES.getValue(row), items, key = { it.id }, state = rowState,
        rowModifier = Modifier.enterOn(if (restoring) state.restore else last) {
            restoring || rowState.layoutInfo.visibleItemsInfo.any { it.key == lastId }
        },
        trailing = clear?.let { c -> { PosterAction("Clear Recent", onClick = c) } },
    ) { index, item ->
        val key = "$row/${item.id}"
        var m: Modifier = Modifier.onFocusChanged {
            if (it.isFocused) { state.lastInRow[row] = item.id; state.focusedKey = key }
            else if (state.focusedKey == key) state.focusedKey = null
        }
        if (item.id == lastId) m = m.focusRequester(last)
        if (state.restoreKey == key) m = m.focusRequester(state.restore)
        val click = { state.opened = SearchState.Opened(row, item.id, index); state.recordOpened(item); open(item) }
        if (row == "episodes") LandscapeCard(item, onClick = click, modifier = m, onLongClick = { menu(item) })
        else PosterCard(item, onClick = click, modifier = m, onLongClick = { menu(item) })
    }
}

/**
 * Focuses the card opened last (or the one now at its place), after scrolling the results and its
 * row so it's on screen; else the field. Lazy rows compose their cards during layout, so this
 * waits a few frames for the card to be there.
 */
private suspend fun restoreSearchFocus(state: SearchState) {
    val o = state.opened ?: return
    val rows = state.rows()
    val r = rows.indexOfFirst { it.first == o.row }
    if (r < 0) return
    val items = rows[r].second
    val index = items.indexOfFirst { it.id == o.itemId }.takeIf { it >= 0 } ?: o.index.coerceIn(0, items.lastIndex)
    val key = "${o.row}/${items[index].id}"
    state.restoreKey = key
    withFrameNanos { }
    val columnIndex = r + 1 // (the field is first)
    if (state.column.layoutInfo.visibleItemsInfo.none { it.index == columnIndex }) runCatching { state.column.scrollToItem(columnIndex) }
    withFrameNanos { }
    val rowState = state.rowState(o.row)
    if (rowState.layoutInfo.visibleItemsInfo.none { it.index == index }) runCatching { rowState.scrollToItem(index) }
    if (state.restore.focusSoon(30) { state.focusedKey == key }) Log.i("Moonrise") { "search: focus back on ${o.row} #$index" }
    else Log.w("Moonrise", "search: couldn't focus ${o.row} #$index")
    state.restoreKey = null
}
