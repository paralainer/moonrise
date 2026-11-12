package app.moonrise.tv.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyGridState
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.foundation.focusable
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.tv.material3.Surface
import androidx.tv.material3.Text
import app.moonrise.core.Item
import app.moonrise.core.ItemPage
import app.moonrise.core.LibraryCategory
import app.moonrise.core.LibraryFacets
import app.moonrise.core.LibraryFilter
import app.moonrise.core.LibrarySort
import app.moonrise.core.LibraryView
import app.moonrise.core.MoonriseHttpException
import app.moonrise.core.ResolutionFilter
import app.moonrise.core.WatchedFilter
import app.moonrise.core.ratingLabel
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import app.moonrise.shared.AppSettings
import app.moonrise.shared.errorText
import app.moonrise.shared.AutoRetry
import app.moonrise.shared.Log
import app.moonrise.shared.Session
import app.moonrise.shared.WatchMarks
import app.moonrise.shared.Network

/** What's beside the Library's sidebar: the grid, or a category's tiles. */
sealed interface LibraryPage {
    data object Grid : LibraryPage
    data class Category(val category: LibraryCategory) : LibraryPage
}

/**
 * The Library tab's content (`TVLibraryScreen`, `LibraryBrowser`), kept while a title page is on
 * top: the view (Movies or Shows), its filters and sort (remembered per view), the titles loaded
 * so far, the counts the filters can pick, and where focus was.
 */
class LibraryState {
    var view by mutableStateOf(AppSettings.libraryView)
        private set
    var filter by mutableStateOf(AppSettings.filter(view))
        private set
    var page by mutableStateOf<LibraryPage>(LibraryPage.Grid)
    var items by mutableStateOf<List<Item>>(emptyList())
    var total by mutableIntStateOf(0)
    var index by mutableStateOf<List<ItemPage.Letter>>(emptyList())
    var facets by mutableStateOf<LibraryFacets?>(null)
    var loading by mutableStateOf(false)
    var error by mutableStateOf<String?>(null)
    var unauthorized = false
    /** The category a tile filtered the grid from: Back goes back to it, onto the tile. */
    var cameFrom: LibraryCategory? = null
    var pickedTile: String? = null
    /** The title opened last: focused again when the Library comes back. */
    var opened: String? = null
    val grid = LazyGridState()
    /** The poster focused last (focus coming back into the grid lands on it). */
    var focused: String? = null
    /** Bumped when the list starts over (another view): the grid goes back to its top. */
    var toTop by mutableIntStateOf(0)
    /** Bumped to load again (Retry, the network back, after playback or a mark). */
    var reloads by mutableIntStateOf(0)
    /** A tile was picked: focus goes to the first title once the grid has loaded for it. */
    var focusFirst = false
    /**
     * Focus is held by the content area itself while it changes pages (a tile to its grid, the
     * grid back to the tiles), so it doesn't fall onto the sidebar while the new page loads.
     */
    var holding = false
    /** Each view's category tiles, shown at once when a category page comes back. */
    val tileFacets = mutableMapOf<LibraryView, LibraryFacets>()
    private var loadedKey by mutableStateOf<Any?>(null)
    /** The titles shown are the ones for the view and filters chosen (not a load behind). */
    val upToDate: Boolean get() = loadedKey == key && !loading
    private var generation = 0
    private var moreRunning = false

    fun clear() {
        view = AppSettings.libraryView; filter = AppSettings.filter(view); page = LibraryPage.Grid
        items = emptyList(); total = 0; index = emptyList(); facets = null; error = null; opened = null; cameFrom = null
        loadedKey = null; focused = null; focusFirst = false; holding = false; tileFacets.clear(); toTop++
    }

    fun refreshSoon() { loadedKey = null }

    fun select(v: LibraryView) {
        cameFrom = null
        pickedTile = null
        page = LibraryPage.Grid
        if (v == view) return
        view = v
        AppSettings.libraryView = v
        filter = AppSettings.filter(v)
        // (Nothing of the other view stays on screen: not its titles, nor its count.)
        items = emptyList(); total = 0; index = emptyList(); facets = null; focused = null; opened = null; toTop++
    }

    /** A category's tile: the grid with its filter, focus on its first title once loaded. */
    fun pickTile(from: LibraryCategory, tile: String, f: LibraryFilter) {
        cameFrom = from
        pickedTile = tile
        opened = null
        focused = null
        focusFirst = true
        if (f != filter) { items = emptyList(); total = 0; index = emptyList(); toTop++ }
        applyFilter(f)
        page = LibraryPage.Grid
    }

    fun applyFilter(f: LibraryFilter) {
        if (f == filter) return
        filter = f
        AppSettings.saveFilter(view, f)
    }

    private val key get() = Triple(view, filter, Pair(reloads, WatchMarks.generation))

    /** The first page and the facets, unless they're loaded for these filters already. */
    suspend fun loadIfNeeded() {
        val k = key
        if (k == loadedKey && error == null) return
        val api = Session.api ?: return
        val gen = ++generation
        val keepPlace = loadedKey?.let { (it as Triple<*, *, *>).first == view && it.second == filter } == true
        loading = true
        // (An error stays up until a load works: the automatic retries keep backing off.)
        val f = filter
        val v = view
        try {
            coroutineScopeOf {
                val fa = async { runCatching { api.facets(v, f) }.getOrNull() }
                val page = api.items(v, f, 0, maxOf(PAGE, if (keepPlace) items.size else 0))
                if (gen != generation) return@coroutineScopeOf
                items = page.items
                total = page.total
                index = page.index
                error = null
                loading = false
                fa.await()?.let { got ->
                    if (gen != generation) return@let
                    facets = got
                    // A library chosen without its name (a remembered filter): named from the facets.
                    val l = filter.library
                    if (l != null && filter.libraryName == null) got.libraries.firstOrNull { it.id == l }?.let { applyFilter(filter.copy(libraryName = it.name)) }
                }
            }
            if (gen == generation) loadedKey = key
        } catch (e: Exception) {
            if (gen != generation) return
            Log.w("Moonrise", "library failed: ${e.message}")
            unauthorized = e is MoonriseHttpException && e.code == 401
            error = errorText(e)
        } finally {
            if (gen == generation) loading = false
        }
    }

    /** The next page, when the grid nears the end of what's loaded. */
    suspend fun loadMore(through: Int = 0) {
        if (moreRunning || items.size >= total) return
        val api = Session.api ?: return
        moreRunning = true
        val gen = generation
        try {
            val want = minOf(500, maxOf(PAGE, through - items.size + PAGE))
            val page = api.items(view, filter, items.size, want)
            if (gen == generation) {
                val known = items.map { it.id }.toSet()
                items = items + page.items.filter { it.id !in known }
                total = page.total
            }
        } catch (e: Exception) {
            Log.w("Moonrise", "library page failed: ${e.message}")
        } finally {
            moreRunning = false
        }
    }

    companion object {
        const val PAGE = 60
    }
}

private suspend fun coroutineScopeOf(block: suspend kotlinx.coroutines.CoroutineScope.() -> Unit) = kotlinx.coroutines.coroutineScope(block)

private val SIDEBAR = 180.dp
private val GUTTER = 28.dp

/**
 * The Apple TV Library: a sidebar (Movies, Shows, then the view's categories) beside a slim
 * header bar (the count; Jump To, Sort, Filter; the filters that are on, with Clear) and the
 * poster grid. Filter opens a panel from the right; each choice applies as it's made.
 */
@Composable
fun LibraryScreen(state: LibraryState, open: (Item) -> Unit, atTop: (Boolean) -> Unit, main: MainState? = null) {
    /** Focus is beside the sidebar: in the grid, its header or a category's tiles. */
    var contentFocused by remember { mutableStateOf(false) }
    val sidebarRows = remember { mutableMapOf<String, FocusRequester>() }
    fun rowRequester(id: String) = sidebarRows.getOrPut(id) { FocusRequester() }
    val currentRow = when (val p = state.page) { LibraryPage.Grid -> state.view.id; is LibraryPage.Category -> p.category.key }

    LaunchedEffect(state.view, state.filter, state.reloads, WatchMarks.generation) { state.loadIfNeeded() }
    LaunchedEffect(Network.generation) { if (state.error != null) state.reloads++ }
    // (Only for the grid: a category page retries its own tiles.)
    AutoRetry(state.error.takeIf { state.page == LibraryPage.Grid }, busy = { state.loading }) { state.reloads++ }

    val holder = remember { FocusRequester() }
    fun hold() {
        state.holding = true
        if (runCatching { holder.requestFocus() }.isFailure) state.holding = false
    }
    // Holding ended without a page taking focus (it failed to load): the sidebar.
    fun release(focusedSomething: Boolean) {
        if (!state.holding) return
        state.holding = false
        if (!focusedSomething) runCatching { rowRequester(currentRow).requestFocus() }
    }

    // Down from the tab bar, when the focus search finds nothing below: the sidebar's row.
    EnterContent(main) { rowRequester(currentRow).focusSoon(10) }

    // Back in the content: back to the category a tile came from, else to the sidebar. (On the
    // sidebar or the tab bar, Back is the tab bar's: up to the bar, then Home, then out.)
    BackHandler(enabled = contentFocused) {
        val c = state.cameFrom
        if (state.page == LibraryPage.Grid && c != null) {
            state.cameFrom = null
            hold()
            state.page = LibraryPage.Category(c)
        } else {
            runCatching { rowRequester(currentRow).requestFocus() }
        }
    }

    Row(Modifier.fillMaxSize().padding(top = TAB_BAR_SPACE)) {
        // Sidebar
        LazyColumn(
            Modifier
                .width(SIDEBAR + Theme.margin)
                .fillMaxHeight()
                .enterOn(rowRequester(currentRow)) { true },
            contentPadding = PaddingValues(start = Theme.margin - 10.dp, top = 4.dp, bottom = 30.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            items(LibraryView.entries, key = { it.id }) { v ->
                SidebarRow(v.title, selected = v.id == currentRow, emphasised = v == state.view && currentRow != v.id,
                    modifier = Modifier.focusRequester(rowRequester(v.id))) { state.select(v) }
            }
            item { Box(Modifier.padding(horizontal = 10.dp, vertical = 8.dp).fillMaxWidth().height(1.dp).background(Color.White.copy(alpha = 0.15f))) }
            items(LibraryCategory.of(state.view), key = { it.key }) { c ->
                SidebarRow(c.title(state.view.shows), selected = c.key == currentRow, modifier = Modifier.focusRequester(rowRequester(c.key))) {
                    state.cameFrom = null
                    state.pickedTile = null
                    state.page = LibraryPage.Category(c)
                }
            }
        }
        Spacer(Modifier.width(GUTTER))
        Box(
            Modifier.weight(1f).fillMaxHeight()
                .onFocusChanged { contentFocused = it.hasFocus }
                .focusRequester(holder)
                // Focusable itself only while it holds focus between pages.
                .focusProperties { canFocus = state.holding }
                .focusable(),
        ) {
            when (val p = state.page) {
                LibraryPage.Grid -> LibraryGrid(state, open, atTop, fromTabs = { main?.tabSwitched == true }, release = ::release)
                is LibraryPage.Category -> CategoryPage(state, p.category, atTop, release = ::release) { tile ->
                    // A tile: the view's grid with that filter (genres and studios keep the sort).
                    var f = tile.filter
                    if (p.category == LibraryCategory.Genres || p.category == LibraryCategory.Studios) {
                        f = f.copy(sort = state.filter.sort, descending = state.filter.descending)
                    }
                    hold()
                    state.pickTile(p.category, tile.id, f)
                }
            }
        }
    }
}

@Composable
private fun SidebarRow(title: String, selected: Boolean, modifier: Modifier = Modifier, emphasised: Boolean = false, onClick: () -> Unit) {
    FocusSurface(
        onClick = onClick,
        modifier = modifier.fillMaxWidth().logFocus { "sidebar $title" },
        shape = RoundedCornerShape(50),
        focusedScale = 1.03f,
        containerColor = if (selected) Color.White.copy(alpha = 0.16f) else Color.Transparent, contentColor = Color.White,
            focusedContainerColor = Color.White, focusedContentColor = Color.Black,
    ) {
        Text(title, fontSize = 15.sp, fontWeight = if (emphasised || selected) FontWeight.SemiBold else FontWeight.Normal, maxLines = 1,
            modifier = Modifier.padding(horizontal = 14.dp, vertical = 8.dp))
    }
}

/** The header bar's glass buttons (`TVHeaderLabel`). */
@Composable
private fun HeaderButton(text: String, modifier: Modifier = Modifier, menu: Boolean = false, onClick: () -> Unit) {
    GlassButton(onClick = onClick, modifier = modifier) {
        Text(text + if (menu) "  ▾" else "", fontSize = 13.sp, maxLines = 1)
    }
}

@Composable
private fun LibraryGrid(state: LibraryState, open: (Item) -> Unit, atTop: (Boolean) -> Unit, fromTabs: () -> Boolean, release: (Boolean) -> Unit) {
    val scope = rememberCoroutineScope()
    var sortMenu by remember { mutableStateOf(false) }
    var jumpMenu by remember { mutableStateOf(false) }
    var panel by remember { mutableStateOf(false) }
    var menuFor by remember { mutableStateOf<Item?>(null) }
    val filterButton = remember { FocusRequester() }
    val restore = remember { FocusRequester() }
    val f = state.filter
    val grid = state.grid

    LaunchedEffect(grid) { snapshotFlow { grid.firstVisibleItemIndex == 0 && grid.firstVisibleItemScrollOffset < 40 }.collect(atTop) }
    LaunchedEffect(state.toTop) { if (state.toTop > 0) runCatching { grid.scrollToItem(0) } }
    // Near the end of what's loaded: the next page.
    LaunchedEffect(grid, state.items.size) {
        snapshotFlow { grid.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 }.collect { last ->
            if (last >= state.items.size - 18 && state.items.size < state.total) state.loadMore()
        }
    }
    // A tile was picked: the content area holds focus until this filter's titles are in, then it
    // goes to the first one (with none, the Filter button; on an error, its Retry).
    LaunchedEffect(Unit) {
        if (!state.focusFirst) return@LaunchedEffect
        try {
            snapshotFlow { state.upToDate || state.error != null }.first { it }
            state.focusFirst = false
            if (state.error != null && state.items.isEmpty()) return@LaunchedEffect
            val ok = if (state.items.isNotEmpty()) {
                runCatching { grid.scrollToItem(0) }
                restore.focusSoon(30) { state.focused == state.items.firstOrNull()?.id }
            } else filterButton.focusSoon(10)
            Log.i("Moonrise") { "library: tile's grid ${if (ok) "focused" else "not focused"}" }
            release(ok)
        } finally {
            // (An error's Retry takes focus by itself.)
            if (state.holding && state.error != null) state.holding = false
        }
    }
    // Back from a title page: focus the poster opened (not when the tab was switched to on the tab bar).
    LaunchedEffect(Unit) {
        if (state.focusFirst || fromTabs()) return@LaunchedEffect
        val target = state.opened ?: return@LaunchedEffect
        val i = state.items.indexOfFirst { it.id == target }.takeIf { it >= 0 } ?: return@LaunchedEffect
        if (grid.layoutInfo.visibleItemsInfo.none { it.index == i + 1 }) runCatching { grid.scrollToItem(i + 1) }
        if (restore.focusSoon(30) { state.focused == target }) Log.i("Moonrise") { "library: focus back on #$i" }
    }

    BoxWithConstraints(Modifier.fillMaxSize()) {
        val (columns, card) = gridLayout(maxWidth - Theme.margin, min = 110.dp, spacing = 20.dp)
        LazyVerticalGrid(
            state = grid,
            columns = GridCells.Fixed(columns),
            modifier = Modifier.fillMaxSize().enterOn(restore) {
                val target = state.focused ?: state.items.firstOrNull()?.id
                grid.layoutInfo.visibleItemsInfo.any { it.key == target }
            },
            contentPadding = PaddingValues(end = Theme.margin, bottom = 40.dp, top = 4.dp),
            horizontalArrangement = Arrangement.spacedBy(20.dp),
            verticalArrangement = Arrangement.spacedBy(26.dp),
        ) {
            item(span = { GridItemSpan(maxLineSpan) }, key = "header") {
                Column(verticalArrangement = Arrangement.spacedBy(6.dp), modifier = Modifier.padding(bottom = 4.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                        // (No count until this view's titles are in.)
                        val counted = state.items.isNotEmpty() || state.upToDate
                        Text(if (counted) state.view.countTitle(state.total) else state.view.title, fontSize = 18.sp, fontWeight = FontWeight.SemiBold, maxLines = 1)
                        if (state.loading) Text("…", color = Theme.secondaryText)
                        Spacer(Modifier.weight(1f))
                        if (f.sort == LibrarySort.Title && state.index.size > 1) HeaderButton("Jump To", menu = true) { jumpMenu = true }
                        HeaderButton("Sort: ${f.sort.label}", menu = true) { sortMenu = true }
                        HeaderButton(if (f.activeCount > 0) "Filter · ${f.activeCount}" else "Filter", Modifier.focusRequester(filterButton)) { panel = true }
                    }
                    if (f.isFiltered) {
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                            Text(f.summary, fontSize = 13.sp, fontWeight = FontWeight.Medium, color = Theme.secondaryText, maxLines = 1,
                                overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                            HeaderButton("✕  Clear") {
                                state.cameFrom = null
                                state.applyFilter(f.cleared)
                                scope.launch { delay(50); runCatching { filterButton.requestFocus() } }
                            }
                        }
                    }
                }
            }
            val err = state.error
            if (err != null && state.items.isEmpty()) {
                item(span = { GridItemSpan(maxLineSpan) }, key = "error") {
                    ErrorPane(err, onRetry = { state.reloads++ }, modifier = Modifier.heightIn(min = 300.dp))
                }
            } else if (state.items.isEmpty() && !state.loading && state.facets != null) {
                item(span = { GridItemSpan(maxLineSpan) }, key = "empty") {
                    val noun = if (state.view.shows) "shows" else "movies"
                    if (f.isFiltered) EmptyState("No Matches", "No $noun match these filters.") else EmptyState("Nothing Here", "This library is empty.")
                }
            }
            itemsIndexed(state.items, key = { _, it -> it.id }) { _, item ->
                val target = state.opened ?: state.focused ?: state.items.firstOrNull()?.id
                val m = (if (item.id == target) Modifier.focusRequester(restore) else Modifier)
                    .onFocusChanged { if (it.isFocused) state.focused = item.id }
                PosterCard(item, onClick = { state.opened = item.id; open(item) }, modifier = m, width = card, onLongClick = { menuFor = item })
            }
        }
    }

    if (sortMenu) {
        val (up, down) = f.sort.directions
        MenuDialog(
            title = "Sort", onDismiss = { sortMenu = false },
            sections = listOf(
                MenuSection("Sort By", LibrarySort.entries.map { s -> MenuRow(s.label, checked = s == f.sort) { state.applyFilter(f.sortedBy(s)) } }),
                MenuSection("Order", listOf(
                    MenuRow(up, checked = !f.descending) { state.applyFilter(f.copy(descending = false)) },
                    MenuRow(down, checked = f.descending) { state.applyFilter(f.copy(descending = true)) },
                )),
            ),
        )
    }
    if (jumpMenu) {
        MenuDialog(
            title = "Jump To", onDismiss = { jumpMenu = false }, width = 200.dp,
            sections = listOf(MenuSection(rows = state.index.map { l ->
                MenuRow(l.letter) {
                    scope.launch {
                        if (l.offset >= state.items.size) state.loadMore(through = l.offset + LibraryState.PAGE)
                        val id = state.items.getOrNull(l.offset)?.id ?: return@launch
                        state.opened = id
                        runCatching { grid.scrollToItem(l.offset + 1) }
                        repeat(20) { delay(30); if (runCatching { restore.requestFocus() }.isSuccess) return@launch }
                    }
                }
            })),
        )
    }
    if (panel) FilterPanel(state, onClose = {
        panel = false
        scope.launch { delay(60); runCatching { filterButton.requestFocus() } }
    })
    menuFor?.let { WatchMenu(it, onDismiss = { menuFor = null }) }
}

/** Columns and card width: posters fill the row evenly (at least `min` wide). */
private fun gridLayout(width: Dp, min: Dp, spacing: Dp): Pair<Int, Dp> {
    val n = maxOf(2, ((width + spacing) / (min + spacing)).toInt())
    return n to (width - spacing * (n - 1)) / n
}

/** A category's tiles (genres, studios, decades: backdrops; collections: posters). */
@Composable
private fun CategoryPage(
    state: LibraryState, category: LibraryCategory, atTop: (Boolean) -> Unit, release: (Boolean) -> Unit,
    pick: (LibraryFacets.Tile) -> Unit,
) {
    var facets by remember(state.view, category) { mutableStateOf(state.tileFacets[state.view]) }
    var error by remember { mutableStateOf<String?>(null) }
    var reloads by remember { mutableIntStateOf(0) }
    var fetching by remember { mutableStateOf(false) }
    val restore = remember { FocusRequester() }
    val grid = remember(category) { LazyGridState() }
    LaunchedEffect(grid) { snapshotFlow { grid.firstVisibleItemIndex == 0 && grid.firstVisibleItemScrollOffset < 40 }.collect(atTop) }
    LaunchedEffect(state.view, category, reloads) {
        val api = Session.api ?: return@LaunchedEffect
        // The category's tiles over the whole view (a page of its own: no other filters).
        fetching = true
        try {
            val got = api.facets(state.view, LibraryFilter())
            state.tileFacets[state.view] = got
            facets = got
            error = null
        } catch (e: Exception) {
            if (facets == null) error = errorText(e)
            // (The Retry takes focus.)
            if (facets == null) state.holding = false
        } finally {
            fetching = false
        }
    }
    LaunchedEffect(Network.generation) { if (error != null) reloads++ }
    AutoRetry(error, busy = { fetching }) { reloads++ }
    val tiles = facets?.tiles(category).orEmpty()
    // Back from the grid: the tile that was picked.
    LaunchedEffect(tiles) {
        if (tiles.isEmpty() || state.pickedTile == null) return@LaunchedEffect
        val i = tiles.indexOfFirst { it.id == state.pickedTile }.coerceAtLeast(0)
        if (grid.layoutInfo.visibleItemsInfo.none { it.index == i + 1 }) runCatching { grid.scrollToItem(i + 1) }
        val ok = restore.focusSoon(30)
        Log.i("Moonrise") { "library: focus back on tile #$i: $ok" }
        state.pickedTile = null
        release(ok)
    }
    val posters = category == LibraryCategory.Collections
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val (columns, card) = if (posters) gridLayout(maxWidth - Theme.margin, 110.dp, 20.dp) else gridLayout(maxWidth - Theme.margin, 190.dp, 20.dp)
        LazyVerticalGrid(
            state = grid,
            columns = GridCells.Fixed(columns),
            modifier = Modifier.fillMaxSize().enterOn(restore) {
                val target = state.pickedTile ?: tiles.firstOrNull()?.id
                grid.layoutInfo.visibleItemsInfo.any { it.key == target }
            },
            contentPadding = PaddingValues(end = Theme.margin, bottom = 40.dp, top = 4.dp),
            horizontalArrangement = Arrangement.spacedBy(20.dp),
            verticalArrangement = Arrangement.spacedBy(if (posters) 26.dp else 20.dp),
        ) {
            item(span = { GridItemSpan(maxLineSpan) }, key = "header") {
                Row(Modifier.height(44.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(category.title(state.view.shows), fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
                }
            }
            error?.let { e ->
                item(span = { GridItemSpan(maxLineSpan) }, key = "error") { ErrorPane(e, onRetry = { reloads++ }, modifier = Modifier.heightIn(min = 300.dp)) }
            }
            itemsIndexed(tiles, key = { _, t -> t.id }) { i, t ->
                val picked = state.pickedTile?.takeIf { id -> tiles.any { it.id == id } }
                val m = if (t.id == picked || (picked == null && i == 0)) Modifier.focusRequester(restore) else Modifier
                val count = "${t.count} " + if (state.view.shows) (if (t.count == 1) "show" else "shows") else (if (t.count == 1) "movie" else "movies")
                if (posters) {
                    PosterCard(Item(id = t.id, kind = "collection", title = t.title, images = app.moonrise.core.ItemImages(poster = t.poster)),
                        onClick = { pick(t) }, modifier = m, width = card)
                } else {
                    TileCard(t.title, count, t.art, onClick = { pick(t) }, modifier = m, width = card)
                }
            }
        }
    }
}

/**
 * Every filter as a list with counts, in a panel from the right (`TVFilterPanel`). Choices apply
 * as they're made (the grid behind follows); Back goes back a list, then closes.
 */
@Composable
private fun FilterPanel(state: LibraryState, onClose: () -> Unit) {
    var open by remember { mutableStateOf<Facet?>(null) }
    var returnTo by remember { mutableStateOf<Facet?>(null) }
    val f = state.filter
    val facets = state.facets
    Dialog(
        onDismissRequest = { if (open != null) { returnTo = open; open = null } else onClose() },
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Row(Modifier.fillMaxSize()) {
            Box(Modifier.weight(1f).fillMaxHeight().background(Color.Black.copy(alpha = 0.4f)))
            Column(Modifier.width(410.dp).fillMaxHeight().background(Color(0xF51B1D24)).padding(start = 30.dp, end = 40.dp, top = 30.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp)) {
                    Text(open?.title ?: "Filter", fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
                    Spacer(Modifier.weight(1f))
                    if (state.loading) Text("…  ", color = Theme.secondaryText)
                    Text(state.view.countTitle(state.total), fontSize = 13.sp, color = Theme.secondaryText)
                }
                val first = remember(open) { FocusRequester() }
                LazyColumn(contentPadding = PaddingValues(vertical = 8.dp, horizontal = 0.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    val o = open
                    if (o == null) {
                        val shown = Facet.entries.filter { it != Facet.Library || (facets?.libraryChoices(f.library)?.size ?: 0) > 1 || f.library != null }
                        items(shown, key = { it.name }) { facet ->
                            PanelRow(facet.title, value = facet.value(f), chevron = true,
                                modifier = if (facet == (returnTo ?: shown.first())) Modifier.focusRequester(first) else Modifier) { open = facet }
                        }
                        if ((facets?.hdr ?: 0) > 0 || f.hdr) item(key = "hdr") {
                            PanelRow("HDR", count = facets?.hdr, checked = f.hdr) { state.applyFilter(f.copy(hdr = !f.hdr)) }
                        }
                        item(key = "recent") {
                            PanelRow("Recently Added", count = facets?.recentlyAdded?.count, checked = f.recentlyAdded) { state.applyFilter(f.copy(recentlyAdded = !f.recentlyAdded)) }
                        }
                        item(key = "clear") {
                            Spacer(Modifier.height(10.dp))
                            PanelRow("Clear All", enabled = f.isFiltered) { state.applyFilter(f.cleared) }
                        }
                    } else {
                        val (anyLabel, options) = o.options(f, facets)
                        item(key = "any") {
                            PanelRow(anyLabel, checked = o.value(f) == "Any", modifier = Modifier.focusRequester(first)) { state.applyFilter(o.clear(f)) }
                            Spacer(Modifier.height(8.dp))
                        }
                        items(options, key = { it.id }) { opt ->
                            PanelRow(opt.label, count = opt.count, checked = opt.on) { state.applyFilter(opt.toggle(state.filter)) }
                        }
                    }
                }
                LaunchedEffect(open) { repeat(15) { delay(30); if (runCatching { first.requestFocus() }.isSuccess) return@LaunchedEffect } }
            }
        }
    }
}

@Composable
private fun PanelRow(
    title: String, modifier: Modifier = Modifier, value: String? = null, count: Int? = null, checked: Boolean? = null,
    chevron: Boolean = false, enabled: Boolean = true, onClick: () -> Unit,
) {
    FocusSurface(
        onClick = onClick, enabled = enabled,
        modifier = modifier.fillMaxWidth(),
        shape = RoundedCornerShape(50),
        focusedScale = 1.02f,
        containerColor = Color.Transparent, contentColor = Color.White,
            focusedContainerColor = Color.White, focusedContentColor = Color.Black,
            disabledContainerColor = Color.Transparent, disabledContentColor = Color.White.copy(alpha = 0.35f),
    ) {
        Row(Modifier.padding(horizontal = 14.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Text(title, fontSize = 14.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
            val dim = androidx.tv.material3.LocalContentColor.current.copy(alpha = 0.6f)
            value?.let { Text(it, fontSize = 13.sp, color = dim, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 180.dp)) }
            count?.let { Text("$it", fontSize = 13.sp, color = dim) }
            if (chevron) Text("›", fontSize = 16.sp, color = dim)
            if (checked != null) Text(if (checked) "✓" else "", fontSize = 14.sp, fontWeight = FontWeight.Bold, modifier = Modifier.width(18.dp))
        }
    }
}

private data class Option(val id: String, val label: String, val count: Int?, val on: Boolean, val toggle: (LibraryFilter) -> LibraryFilter)

private enum class Facet(val title: String) {
    Library("Library"), Genre("Genre"), Year("Year"), Rating("Rating"), Watched("Watched"), Resolution("Resolution");

    fun value(f: LibraryFilter): String = when (this) {
        Library -> if (f.library == null) "Any" else f.libraryName ?: "1 library"
        Genre -> if (f.genres.isEmpty()) "Any" else f.genres.sorted().joinToString(", ")
        Year -> f.decade?.let { "${it}s" } ?: "Any"
        Rating -> if (f.ratings.isEmpty()) "Any" else f.ratings.sorted().joinToString(", ") { ratingLabel(it) }
        Watched -> if (f.watched == WatchedFilter.All) "Any" else f.watched.label
        Resolution -> if (f.resolutions.isEmpty()) "Any" else f.resolutions.sortedBy { it.ordinal }.joinToString(", ") { it.label }
    }

    fun clear(f: LibraryFilter): LibraryFilter = when (this) {
        Library -> f.copy(library = null, libraryName = null)
        Genre -> f.copy(genres = emptySet())
        Year -> f.copy(decade = null)
        Rating -> f.copy(ratings = emptySet())
        Watched -> f.copy(watched = WatchedFilter.All)
        Resolution -> f.copy(resolutions = emptySet())
    }

    /** The "Any" row's label and the facet's values, plus chosen ones with no titles under the other filters. */
    fun options(f: LibraryFilter, facets: LibraryFacets?): Pair<String, List<Option>> = when (this) {
        Library -> "All Libraries" to (facets?.libraryChoices(f.library) ?: emptyList()).map { l ->
            Option(l.id, l.name, l.count, f.library == l.id) { x -> if (x.library == l.id) x.copy(library = null, libraryName = null) else x.copy(library = l.id, libraryName = l.name) }
        }
        Genre -> "Any Genre" to merged(facets?.genres.orEmpty(), f.genres).map { g ->
            Option(g.name, g.name, g.count, g.name in f.genres) { x -> x.copy(genres = if (g.name in x.genres) x.genres - g.name else x.genres + g.name) }
        }
        Year -> {
            val decades = facets?.decades.orEmpty().toMutableList()
            f.decade?.let { d -> if (decades.none { it.decade == d }) decades += LibraryFacets.Decade(d, 0) }
            "Any Year" to decades.map { d -> Option(d.label, d.label, d.count, f.decade == d.decade) { x -> x.copy(decade = if (x.decade == d.decade) null else d.decade) } }
        }
        Rating -> "Any Rating" to merged(facets?.ratings.orEmpty(), f.ratings).map { r ->
            Option(r.name, ratingLabel(r.name), r.count, r.name in f.ratings) { x -> x.copy(ratings = if (r.name in x.ratings) x.ratings - r.name else x.ratings + r.name) }
        }
        Watched -> "Any" to listOf(WatchedFilter.Unwatched, WatchedFilter.InProgress, WatchedFilter.Watched).map { w ->
            Option(w.name, w.label, facets?.watched?.count(w), f.watched == w) { x -> x.copy(watched = if (x.watched == w) WatchedFilter.All else w) }
        }
        Resolution -> {
            val available = facets?.resolutions.orEmpty().associate { it.name to it.count }
            "Any Resolution" to ResolutionFilter.entries.filter { it.key in available || it in f.resolutions }.map { r ->
                Option(r.key, r.label, available[r.key] ?: 0, r in f.resolutions) { x -> x.copy(resolutions = if (r in x.resolutions) x.resolutions - r else x.resolutions + r) }
            }
        }
    }

    private fun merged(values: List<LibraryFacets.Named>, selected: Set<String>): List<LibraryFacets.Named> =
        values + selected.filter { s -> values.none { it.name == s } }.sorted().map { LibraryFacets.Named(it, 0) }
}
