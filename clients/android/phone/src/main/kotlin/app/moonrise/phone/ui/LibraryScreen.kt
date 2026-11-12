package app.moonrise.phone.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.ArrowDownward
import androidx.compose.material.icons.filled.ArrowUpward
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.FilterList
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.outlined.Apartment
import androidx.compose.material.icons.outlined.CalendarMonth
import androidx.compose.material.icons.outlined.CollectionsBookmark
import androidx.compose.material.icons.outlined.FilterAltOff
import androidx.compose.material.icons.outlined.Movie
import androidx.compose.material.icons.outlined.TheaterComedy
import androidx.compose.material.icons.outlined.Tv
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupProperties
import app.moonrise.core.ClientRules
import app.moonrise.core.Item
import app.moonrise.core.LayoutMetrics
import app.moonrise.core.ItemPage
import app.moonrise.core.LibraryCategory
import app.moonrise.core.LibraryFacets
import app.moonrise.core.LibraryFilter
import app.moonrise.core.LibrarySort
import app.moonrise.core.LibraryView
import app.moonrise.core.MoonriseHttpException
import app.moonrise.core.ResolutionFilter
import app.moonrise.core.WatchedFilter
import app.moonrise.core.countLine
import app.moonrise.core.ratingLabel
import app.moonrise.phone.LocalNav
import app.moonrise.phone.Route
import app.moonrise.shared.AppSettings
import app.moonrise.shared.AutoRetry
import app.moonrise.shared.Log
import app.moonrise.shared.Network
import app.moonrise.shared.Session
import app.moonrise.shared.WatchMarks
import app.moonrise.core.DownloadRules
import app.moonrise.shared.downloads.Downloads
import app.moonrise.shared.downloads.ServerReach
import app.moonrise.phone.Autoplay
import app.moonrise.shared.errorText
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

private const val PAGE = 60

/** One Library grid's titles, filters and paging (`LibraryBrowser`'s state). */
private class GridState(val view: LibraryView, initial: LibraryFilter, val root: Boolean) {
    var filter by mutableStateOf(initial)
    var items by mutableStateOf<List<Item>>(emptyList())
    var total by mutableIntStateOf(0)
    var index by mutableStateOf<List<ItemPage.Letter>>(emptyList())
    var facets by mutableStateOf<LibraryFacets?>(null)
    var loading by mutableStateOf(false)
    var error by mutableStateOf<String?>(null)
    var unauthorized by mutableStateOf(false)
    var reloads by mutableIntStateOf(0)
    /** No server: the downloads, filtered and sorted here. */
    var offline by mutableStateOf(false)
    private var generation = 0
    private var more = false

    fun apply(f: LibraryFilter) {
        if (f == filter) return
        filter = f
        if (root) AppSettings.saveFilter(view, f)
    }

    fun loadOffline() {
        generation++
        offline = true
        val f = filter
        val titles = DownloadRules.titles(Downloads.records, view.shows)
        items = DownloadRules.offlineLibrary(Downloads.records, view.shows, f)
        total = items.size
        index = if (f.sort == LibrarySort.Title) DownloadRules.letters(items) else emptyList()
        facets = DownloadRules.offlineFacets(titles, f)
        loading = false
        error = null
    }

    suspend fun reload() {
        val api = Session.api ?: return
        if (!ServerReach.reachable && Downloads.hasPlayable) { loadOffline(); return }
        offline = false
        val gen = ++generation
        val f = filter
        loading = true
        try {
            coroutineScope {
                val fa = async { runCatching { api.facets(view, f) }.getOrNull() }
                val page = api.items(view, f, 0, PAGE)
                if (gen != generation) return@coroutineScope
                items = page.items; total = page.total; index = page.index
                error = null; unauthorized = false
                loading = false
                fa.await()?.let { if (gen == generation) facets = it }
            }
        } catch (e: Exception) {
            if (!ClientRules.isLoadFailure(e)) throw e
            if (gen != generation) return
            // The server went away: the downloads, quietly.
            if (Downloads.hasPlayable) { ServerReach.nudge(); loadOffline(); return }
            Log.w("Moonrise", "library failed: ${e.message}")
            unauthorized = e is MoonriseHttpException && e.code == 401
            error = errorText(e)
        } finally {
            if (gen == generation) loading = false
        }
    }

    suspend fun loadMore(through: Int = 0) {
        if (more || items.size >= total || offline) return
        val api = Session.api ?: return
        more = true
        val gen = generation
        try {
            val page = api.items(view, filter, items.size, minOf(500, maxOf(PAGE, through - items.size + PAGE)))
            if (gen == generation) {
                val known = items.map { it.id }.toSet()
                items = items + page.items.filter { it.id !in known }
                total = page.total
            }
        } catch (e: Exception) {
            if (!ClientRules.isLoadFailure(e)) throw e
            Log.w("Moonrise", "library page failed: ${e.message}")
        } finally {
            more = false
        }
    }
}

/**
 * The Library, iPhone held upright (`LibraryBrowser`): the title with Sort and Filter beside it,
 * Movies | Shows, the categories (Genres, Collections, Studios or Networks, Decades), the filters
 * that are on (a tap removes one), the count with Jump To (title sort), then the poster grid,
 * which loads more as it scrolls. `filtered`: a category's tile, its grid titled with its name.
 * Each view remembers its filters and sort.
 */
@Composable
fun LibraryScreen(filtered: Route.Filtered?, sidebarView: LibraryView? = null) {
    val nav = LocalNav.current
    val pad = sidebarView != null
    var view by remember { mutableStateOf(filtered?.view ?: sidebarView ?: AppSettings.libraryView) }
    val state = remember(view) { GridState(view, filtered?.filter ?: AppSettings.filter(view), root = filtered == null) }
    val grid = rememberLazyGridState()
    val scope = rememberCoroutineScope()
    var filters by remember { mutableStateOf(false) }

    LaunchedEffect(state, state.filter, state.reloads, WatchMarks.generation, ServerReach.reachable) { state.reload() }
    LaunchedEffect(Downloads.records.count { it.isPlayable }) { if (state.offline) state.loadOffline() }
    // Other filters or sort: back to the top.
    LaunchedEffect(state, state.filter) { grid.scrollToItem(0) }
    LaunchedEffect(Network.generation) { if (state.error != null) state.reloads++ }
    AutoRetry(state.error, busy = { state.loading }) { state.reloads++ }
    // Near the end of what's loaded: the next page.
    LaunchedEffect(grid, state) {
        snapshotFlow { grid.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 }.distinctUntilChanged().collect { last ->
            if (last >= state.items.size - 12) state.loadMore()
        }
    }

    PageBackground {
        Column(Modifier.fillMaxSize()) {
            // The title row stays at the top (the iPhone's pinned header).
            val margin = Theme.margin
            if (pad) PadTitleRow(filtered?.title ?: view.title, back = if (filtered != null) ({ nav.back() }) else null, state = state, view = view, filters = filters, setFilters = { filters = it })
            else TitleRow(if (filtered == null) "Library" else filtered.title, back = if (filtered != null) ({ nav.back() }) else null, state = state, openFilters = { filters = true })
            BoxWithConstraints(Modifier.fillMaxSize()) {
                val spacing = 12.dp
                val avail = maxWidth - margin * 2
                val metrics = LocalMetrics.current
                val padGrid = if (pad) metrics.libraryGrid(avail.value.toDouble()) else null
                val columns = padGrid?.first ?: maxOf(2, ((avail + spacing) / (Theme.gridMin + spacing)).toInt())
                val card = padGrid?.second?.dp ?: ((avail - spacing * (columns - 1)) / columns)
                LazyVerticalGrid(GridCells.Fixed(columns), state = grid, contentPadding = PaddingValues(start = margin, end = margin, bottom = Theme.pageBottom),
                    horizontalArrangement = Arrangement.spacedBy(spacing), verticalArrangement = Arrangement.spacedBy(spacing + 8.dp)) {
                    item(span = { GridItemSpan(maxLineSpan) }, key = "header") {
                        // Edge to edge (the grid's margins are the cells'): each row keeps the page margin itself.
                        Column(verticalArrangement = Arrangement.spacedBy(14.dp), modifier = Modifier.padding(top = 4.dp).layout { m, c ->
                            val extra = (margin * 2).roundToPx()
                            val p = m.measure(c.copy(minWidth = c.maxWidth + extra, maxWidth = c.maxWidth + extra))
                            layout(c.maxWidth, p.height) { p.place(-extra / 2, 0) }
                        }) {
                            // The sidebar has the views and the categories.
                            if (filtered == null && !pad) ViewSwitch(view) { v -> if (v != view) { AppSettings.libraryView = v; view = v } }
                            if (state.offline) OfflineBanner(retry = { state.reloads++ })
                            if (filtered == null && !state.offline && !pad) CategoryStrip(view, state.facets)
                            ActiveFilters(state)
                            CountRow(state, view, onJump = { letter ->
                                scope.launch {
                                    val at = state.index.firstOrNull { it.letter == letter }?.offset ?: return@launch
                                    state.loadMore(through = at + 1)
                                    grid.animateScrollToItem(1 + at)
                                }
                            })
                        }
                    }
                    if (state.error != null && state.items.isEmpty()) {
                        item(span = { GridItemSpan(maxLineSpan) }, key = "error") { ErrorPane(state.error!!, onRetry = { state.reloads++ }) }
                    } else if (state.items.isEmpty() && !state.loading && state.facets != null) {
                        item(span = { GridItemSpan(maxLineSpan) }, key = "empty") {
                            Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
                                EmptyState(if (state.filter.isFiltered) Icons.Outlined.FilterAltOff else if (view.shows) Icons.Outlined.Tv else Icons.Outlined.Movie,
                                    if (state.filter.isFiltered) "No Matches" else "Nothing Here",
                                    if (state.filter.isFiltered) "No ${if (view.shows) "shows" else "movies"} match these filters."
                                    else if (state.offline) "Nothing in this library is downloaded." else "This library is empty.")
                                if (state.filter.isFiltered) GlassCapsule(onClick = { state.apply(state.filter.cleared) }) { Text("Clear Filters", style = Type.headline, color = Color.White) }
                            }
                        }
                    }
                    items(state.items, key = { it.id }) { item ->
                        if (state.offline) OfflineTitleCard(item, card, onClick = { nav.push(Route.Title(item, if (item.isSeries) null else Autoplay())) })
                        else PosterCard(item, width = card, onClick = { nav.push(Route.Title(item)) }, menu = CardMenu(marked = { state.reloads++ }))
                    }
                    if (state.loading && state.items.isEmpty()) item(span = { GridItemSpan(maxLineSpan) }, key = "loading") {
                        Box(Modifier.fillMaxWidth().padding(40.dp), contentAlignment = Alignment.Center) { CircularProgressIndicator(color = Color.White) }
                    }
                }
            }
        }
    }
    if (filters && !pad) FilterSheet(state, view, onDismiss = { filters = false })
}

/**
 * The tablet's Library (`PadLibraryScreen`): a sidebar (Movies, Shows, then the view's categories)
 * beside the open page. The tab's stack holds the page, as on a phone: a category over the grid,
 * a tile's titles over the category, so Back and a change of window size need nothing of their own.
 */
@Composable
fun PadLibraryScreen() {
    val nav = LocalNav.current
    val metrics = LocalMetrics.current
    var view by remember { mutableStateOf(AppSettings.libraryView) }
    val top = nav.top
    val category = nav.stack.filterIsInstance<Route.Category>().lastOrNull()?.category
    val pages = rememberSaveableStateHolder()
    PageBackground {
        Row(Modifier.fillMaxSize().statusBarsPadding().padding(top = Theme.underTopBar)) {
            Column(Modifier.width(metrics.sidebarWidth.dp).fillMaxHeight().verticalScroll(rememberScrollState()).padding(top = LayoutMetrics.SIDEBAR_TOP.dp, bottom = 20.dp),
                verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text("Library", style = Type.largeTitle, color = Color.White, maxLines = 1, modifier = Modifier.height(LayoutMetrics.TITLE_ROW.dp).padding(horizontal = Theme.margin).padding(top = 6.dp))
                Spacer(Modifier.height(8.dp))
                for (v in LibraryView.entries) SidebarRow(v.title, if (v.shows) Icons.Outlined.Tv else Icons.Outlined.Movie, on = top == null && v == view, current = top != null && v == view) {
                    if (v != view) { AppSettings.libraryView = v; view = v }
                    nav.stack.clear()
                }
                // The views above, the open view's categories below.
                Spacer(Modifier.height(22.dp))
                for (c in LibraryCategory.of(view)) SidebarRow(c.title(view.shows), c.icon(view.shows), on = category == c) {
                    if (category != c || top !is Route.Category) { nav.stack.clear(); nav.push(Route.Category(view, c)) }
                }
            }
            Box(Modifier.width(1.dp).fillMaxHeight().background(Color.White.copy(alpha = 0.08f)))
            Box(Modifier.weight(1f).fillMaxHeight()) {
                key(view, top?.key) {
                    pages.SaveableStateProvider("${view.id}-${top?.key ?: "grid"}") {
                        when (top) {
                            is Route.Category -> CategoryScreen(top, pad = true)
                            is Route.Filtered -> LibraryScreen(top, sidebarView = top.view)
                            else -> LibraryScreen(null, sidebarView = view)
                        }
                    }
                }
            }
        }
    }
}

/** A sidebar row: the icon at the margin, level with "Library"; the tint reaches out around it. */
@Composable
private fun SidebarRow(title: String, icon: ImageVector, on: Boolean, current: Boolean = false, onClick: () -> Unit) {
    val strong = on || current
    Row(Modifier.fillMaxWidth().padding(horizontal = Theme.margin - 10.dp).height(LayoutMetrics.SIDEBAR_ROW.dp).clip(RoundedCornerShape(10.dp))
        .background(Color.White.copy(alpha = if (on) 0.14f else 0f)).clickable(onClick = onClick).padding(horizontal = 10.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Box(Modifier.width(26.dp), contentAlignment = Alignment.Center) { Icon(icon, null, tint = Color.White.copy(alpha = if (strong) 1f else 0.85f), modifier = Modifier.size(22.dp)) }
        Text(title, style = Type.body, fontWeight = if (strong) FontWeight.SemiBold else FontWeight.Normal, color = Color.White.copy(alpha = if (strong) 1f else 0.85f), maxLines = 1)
    }
}

/** The page's header beside the sidebar (`PadLibraryHeader`): (Back) the title, then Sort and Filter. */
@Composable
private fun PadTitleRow(title: String, back: (() -> Unit)?, state: GridState, view: LibraryView, filters: Boolean, setFilters: (Boolean) -> Unit) {
    // Where the button ends, for how much room the panel has below it.
    var buttonBottom by remember { mutableIntStateOf(0) }
    PadHeaderRow(title, back, stacked = LocalMetrics.current.libraryHeaderStacked) {
        SortMenu(state, height = LayoutMetrics.HEADER_BUTTON.dp, small = true)
        Box(Modifier.onGloballyPositioned { buttonBottom = it.boundsInWindow().bottom.roundToInt() }) {
            GlassCapsule(height = LayoutMetrics.HEADER_BUTTON.dp, onClick = { setFilters(true) }) {
                Icon(Icons.Filled.FilterList, null, tint = Color.White, modifier = Modifier.size(18.dp))
                Text("Filter", style = Type.subheadline.copy(fontWeight = FontWeight.SemiBold), color = Color.White)
                if (state.filter.activeCount > 0) Text("${state.filter.activeCount}", style = Type.caption, fontWeight = FontWeight.Bold, color = Color.Black,
                    modifier = Modifier.background(Color.White, CircleShape).padding(horizontal = 6.dp, vertical = 1.dp))
            }
            if (filters) FilterPanel(state, view, buttonBottom, onDismiss = { setFilters(false) })
        }
    }
}

/**
 * One row height for the sidebar's "Library" and the page's title, so their tops line up.
 * `stacked`: the trailing buttons on a row of their own, under the title, where the page is narrow.
 */
@Composable
private fun PadHeaderRow(title: String, back: (() -> Unit)?, stacked: Boolean = false, trailing: @Composable () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(top = LayoutMetrics.SIDEBAR_TOP.dp)) {
        Row(Modifier.fillMaxWidth().height(LayoutMetrics.TITLE_ROW.dp).padding(horizontal = Theme.margin),
            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            if (back != null) GlassCircle(Icons.AutoMirrored.Filled.ArrowBack, "Back", size = LayoutMetrics.HEADER_BUTTON.dp, onClick = back)
            FittedTitle(title, Modifier.weight(1f))
            if (!stacked) trailing()
        }
        if (stacked) Row(Modifier.fillMaxWidth().padding(horizontal = Theme.margin).padding(top = 4.dp, bottom = 4.dp),
            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp, Alignment.End)) { trailing() }
    }
}

/** A large title on one line, set smaller to fit (down to [LayoutMetrics.TITLE_MIN_SCALE]) before it is cut off. */
@Composable
private fun FittedTitle(title: String, modifier: Modifier) {
    BoxWithConstraints(modifier, contentAlignment = Alignment.CenterStart) {
        val measurer = rememberTextMeasurer()
        val natural = remember(title) { measurer.measure(title, Type.largeTitle, maxLines = 1, softWrap = false).size.width }
        val scale = LayoutMetrics.titleScale(natural.toDouble(), constraints.maxWidth.toDouble()).toFloat()
        Text(title, style = Type.largeTitle.copy(fontSize = Type.largeTitle.fontSize * scale, lineHeight = Type.largeTitle.lineHeight * scale), color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** The tablet's Filter panel (`PadFilterPanel`): under its button, the same filters as the sheet. */
@Composable
private fun FilterPanel(state: GridState, view: LibraryView, buttonBottom: Int, onDismiss: () -> Unit) {
    val density = LocalDensity.current
    val y = with(density) { (LayoutMetrics.HEADER_BUTTON.dp + LayoutMetrics.FILTER_PANEL_GAP.dp).roundToPx() }
    val inset = WindowInsets.navigationBars.getBottom(density)
    val maxHeight = LocalMetrics.current.filterPanelHeight((buttonBottom / density.density).toDouble(), (inset / density.density).toDouble())
    Popup(alignment = Alignment.TopEnd, offset = IntOffset(0, y), onDismissRequest = onDismiss, properties = PopupProperties(focusable = true)) {
        Column(Modifier.width(LayoutMetrics.FILTER_PANEL_WIDTH.dp).heightIn(max = maxHeight.dp).clip(RoundedCornerShape(22.dp)).background(Theme.sheet)
            .border(0.5.dp, Color.White.copy(alpha = 0.16f), RoundedCornerShape(22.dp))) {
            FilterContent(state, view, top = 16.dp)
        }
    }
}

/** The page's title (with Back on a tile's page), Sort and Filter, as tall as each other. */
@Composable
private fun TitleRow(title: String, back: (() -> Unit)?, state: GridState, openFilters: () -> Unit) {
    Row(Modifier.fillMaxWidth().statusBarsPadding().padding(horizontal = Theme.margin).padding(top = 4.dp, bottom = 6.dp).height(44.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        if (back != null) GlassCircle(Icons.AutoMirrored.Filled.ArrowBack, "Back", onClick = back)
        Text(title, style = if (back == null) Type.largeTitle else Type.title2.copy(fontWeight = FontWeight.Bold), color = Color.White, maxLines = 1,
            overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
        SortMenu(state)
        GlassCapsule(onClick = openFilters) {
            Icon(Icons.Filled.FilterList, null, tint = Color.White, modifier = Modifier.size(20.dp))
            Text("Filter", style = Type.headline.copy(fontWeight = FontWeight.Medium), color = Color.White)
            if (state.filter.activeCount > 0) Text("${state.filter.activeCount}", style = Type.caption, fontWeight = FontWeight.Bold, color = Color.Black,
                modifier = Modifier.background(Color.White, CircleShape).padding(horizontal = 6.dp, vertical = 1.dp))
        }
    }
}

/** Sort: the six sorts, then the direction; the button shows the sort and its arrow. */
@Composable
private fun SortMenu(state: GridState, height: Dp = 44.dp, small: Boolean = false) {
    var open by remember { mutableStateOf(false) }
    // On a tablet the menu stays under its button: as much of it as fits there, scrolling.
    var buttonBottom by remember { mutableIntStateOf(0) }
    val f = state.filter
    val density = LocalDensity.current
    // Compose places a menu inside the visible frame, which leaves out the status bar too.
    val inset = WindowInsets.navigationBars.getBottom(density) + WindowInsets.statusBars.getTop(density)
    val menuHeight = if (small) LocalMetrics.current.sortMenuHeight((buttonBottom / density.density).toDouble(), (inset / density.density).toDouble()).dp else Dp.Unspecified
    Box(Modifier.onGloballyPositioned { buttonBottom = it.boundsInWindow().bottom.roundToInt() }) {
        GlassCapsule(height = height, onClick = { open = true }) {
            Icon(if (f.descending) Icons.Filled.ArrowDownward else Icons.Filled.ArrowUpward, null, tint = Color.White, modifier = Modifier.size(if (small) 18.dp else 20.dp))
            Text(f.sort.label, style = if (small) Type.subheadline.copy(fontWeight = FontWeight.SemiBold) else Type.headline.copy(fontWeight = FontWeight.Medium), color = Color.White, maxLines = 1)
        }
        DropdownMenu(open, { open = false }, modifier = Modifier.heightIn(max = menuHeight), containerColor = Theme.sheet) {
            Text("Sort By", style = Type.footnote, color = Theme.secondaryText, modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp))
            for (s in LibrarySort.entries) MenuItem(s.label, checked = s == f.sort) { state.apply(f.sortedBy(s)) ; open = false }
            HorizontalDivider()
            Text("Order", style = Type.footnote, color = Theme.secondaryText, modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp))
            MenuItem(f.sort.directions.first, checked = !f.descending) { state.apply(f.copy(descending = false)); open = false }
            MenuItem(f.sort.directions.second, checked = f.descending) { state.apply(f.copy(descending = true)); open = false }
        }
    }
}

/** Movies | Shows, across the margins (the iPhone's segmented control). */
@Composable
private fun ViewSwitch(view: LibraryView, pick: (LibraryView) -> Unit) {
    Row(Modifier.fillMaxWidth().padding(horizontal = Theme.margin).height(32.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.1f)).padding(2.dp)) {
        for (v in LibraryView.entries) {
            val on = v == view
            Box(Modifier.weight(1f).fillMaxSize().clip(CircleShape).background(if (on) Color.White.copy(alpha = 0.28f) else Color.Transparent).clickable { pick(v) },
                contentAlignment = Alignment.Center) {
                Text(v.title, style = Type.subheadline.copy(fontWeight = if (on) FontWeight.SemiBold else FontWeight.Normal), color = Color.White)
            }
        }
    }
}

private fun LibraryCategory.icon(shows: Boolean): ImageVector = when (this) {
    LibraryCategory.Genres -> Icons.Outlined.TheaterComedy
    LibraryCategory.Collections -> Icons.Outlined.CollectionsBookmark
    LibraryCategory.Studios -> if (shows) Icons.Outlined.Tv else Icons.Outlined.Apartment
    LibraryCategory.Decades -> Icons.Outlined.CalendarMonth
}

/** The categories, as chips that open their pages (`LibraryCategoryStrip`). */
@Composable
private fun CategoryStrip(view: LibraryView, facets: LibraryFacets?) {
    val nav = LocalNav.current
    val categories = LibraryCategory.of(view).filter { c ->
        when (c) {
            LibraryCategory.Collections -> !(facets?.collections.isNullOrEmpty())
            LibraryCategory.Studios -> !(facets?.studios.isNullOrEmpty())
            else -> true
        }
    }
    LazyRow(contentPadding = PaddingValues(horizontal = Theme.margin), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        items(categories) { c ->
            Row(Modifier.height(44.dp).clip(RoundedCornerShape(14.dp)).background(Color.White.copy(alpha = 0.06f))
                .border(0.5.dp, Color.White.copy(alpha = 0.16f), RoundedCornerShape(14.dp)).clickable { nav.push(Route.Category(view, c)) }.padding(horizontal = 16.dp),
                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                Icon(c.icon(view.shows), null, tint = Color.White, modifier = Modifier.size(22.dp))
                Text(c.title(view.shows), style = Type.headline.copy(fontWeight = FontWeight.Medium), color = Color.White)
            }
        }
    }
}

/** The filters that are on: a tap removes one; Clear All. */
@Composable
private fun ActiveFilters(state: GridState) {
    val f = state.filter
    val tokens = buildList<Pair<String, () -> LibraryFilter>> {
        f.collection?.let { add((it.name ?: "Collection") to { f.copy(collection = null) }) }
        f.studio?.let { add(it to { f.copy(studio = null) }) }
        f.genres.sorted().forEach { g -> add(g to { f.copy(genres = f.genres - g) }) }
        f.decade?.let { add("${it}s" to { f.copy(decade = null) }) }
        f.ratings.sorted().forEach { r -> add(ratingLabel(r) to { f.copy(ratings = f.ratings - r) }) }
        if (f.watched != WatchedFilter.All) add(f.watched.label to { f.copy(watched = WatchedFilter.All) })
        f.resolutions.sortedBy { it.ordinal }.forEach { r -> add(r.label to { f.copy(resolutions = f.resolutions - r) }) }
        if (f.hdr) add("HDR" to { f.copy(hdr = false) })
        if (f.recentlyAdded) add("Recently Added" to { f.copy(recentlyAdded = false) })
    }
    if (tokens.isEmpty()) return
    LazyRow(contentPadding = PaddingValues(horizontal = Theme.margin), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        items(tokens) { (label, removed) ->
            Row(Modifier.height(32.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.14f)).clickable { state.apply(removed()) }.padding(horizontal = 12.dp),
                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(label, style = Type.subheadline, color = Color.White)
                Icon(Icons.Filled.Close, "Remove $label", tint = Theme.secondaryText, modifier = Modifier.size(14.dp))
            }
        }
        if (tokens.size > 1) item {
            Box(Modifier.height(32.dp).clip(CircleShape).clickable { state.apply(f.cleared) }.padding(horizontal = 12.dp), contentAlignment = Alignment.Center) {
                Text("Clear All", style = Type.subheadline, fontWeight = FontWeight.SemiBold, color = Theme.secondaryText)
            }
        }
    }
}

/** "7 movies · Title, A to Z", and Jump To at the end with a title sort. */
@Composable
private fun CountRow(state: GridState, view: LibraryView, onJump: (String) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Row(Modifier.fillMaxWidth().padding(horizontal = Theme.margin), verticalAlignment = Alignment.CenterVertically) {
        Text(state.filter.countLine(view.shows, state.total) + if (state.offline) " · downloaded" else "", style = Type.subheadline, color = Theme.secondaryText, modifier = Modifier.weight(1f), maxLines = 1)
        if (state.loading && state.items.isNotEmpty()) CircularProgressIndicator(color = Theme.secondaryText, strokeWidth = 2.dp, modifier = Modifier.size(14.dp))
        if (state.filter.sort == LibrarySort.Title && state.index.size > 1) Box {
            Row(Modifier.clickable { open = true }, verticalAlignment = Alignment.CenterVertically) {
                Text("Jump To", style = Type.subheadline, color = Theme.secondaryText)
                Icon(Icons.Filled.KeyboardArrowDown, null, tint = Theme.secondaryText, modifier = Modifier.size(18.dp))
            }
            DropdownMenu(open, { open = false }, containerColor = Theme.sheet) {
                for (l in state.index) MenuItem(l.letter) { open = false; onJump(l.letter) }
            }
        }
    }
}

/** Filter (the iPhone's filter sheet): every filter with its count; each choice applies at once. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun FilterSheet(state: GridState, view: LibraryView, onDismiss: () -> Unit) {
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), containerColor = Theme.sheet) {
        FilterContent(state, view, top = 0.dp)
    }
}

/** Every filter with its count; each choice applies at once (the phone's sheet, the tablet's panel). */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun FilterContent(state: GridState, view: LibraryView, top: Dp) {
    val f = state.filter
    val fa = state.facets
    androidx.compose.foundation.lazy.LazyColumn(Modifier.fillMaxWidth(), contentPadding = PaddingValues(start = 20.dp, end = 20.dp, top = top, bottom = 32.dp), verticalArrangement = Arrangement.spacedBy(18.dp)) {
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Filter", style = Type.title3, fontWeight = FontWeight.Bold, color = Color.White, modifier = Modifier.weight(1f))
                Text("${state.total} ${if (view.shows) "shows" else "movies"}", style = Type.subheadline, color = Theme.secondaryText)
                if (f.isFiltered) Text("Clear All", style = Type.subheadline, fontWeight = FontWeight.SemiBold, color = Color.White,
                    modifier = Modifier.padding(start = 16.dp).clickable { state.apply(f.cleared) })
            }
        }
        fun chips(title: String, options: List<Triple<String, Int?, Boolean>>, toggle: (String) -> Unit) {
            if (options.isEmpty()) return
            item(title) {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(title, style = Type.headline, color = Color.White)
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        for ((label, count, on) in options) {
                            Text(if (count != null) "$label  $count" else label, style = Type.subheadline, color = if (on) Color.Black else Color.White,
                                modifier = Modifier.clip(CircleShape).background(if (on) Color.White else Theme.surfaceStrong).clickable { toggle(label) }
                                    .padding(horizontal = 14.dp, vertical = 7.dp))
                        }
                    }
                }
            }
        }
        chips("Genre", fa?.genres.orEmpty().map { Triple(it.name, it.count, it.name in f.genres) }) { g -> state.apply(f.copy(genres = if (g in f.genres) f.genres - g else f.genres + g)) }
        chips("Decade", fa?.decades.orEmpty().map { Triple(it.label, it.count, it.decade == f.decade) }) { l ->
            val d = l.removeSuffix("s").toIntOrNull(); state.apply(f.copy(decade = if (d == f.decade) null else d))
        }
        chips("Rating", fa?.ratings.orEmpty().map { Triple(ratingLabel(it.name), it.count, it.name in f.ratings) }) { l ->
            val r = fa?.ratings?.firstOrNull { ratingLabel(it.name) == l }?.name ?: return@chips
            state.apply(f.copy(ratings = if (r in f.ratings) f.ratings - r else f.ratings + r))
        }
        chips("Watched", WatchedFilter.entries.filter { it != WatchedFilter.All }.map { Triple(it.label, fa?.watched?.count(it), it == f.watched) }) { l ->
            val w = WatchedFilter.entries.first { it.label == l }; state.apply(f.copy(watched = if (w == f.watched) WatchedFilter.All else w))
        }
        chips("Resolution", fa?.resolutions.orEmpty().mapNotNull { n -> ResolutionFilter.of(n.name)?.let { Triple(it.label, n.count, it in f.resolutions) } }) { l ->
            val r = ResolutionFilter.entries.first { it.label == l }; state.apply(f.copy(resolutions = if (r in f.resolutions) f.resolutions - r else f.resolutions + r))
        }
        chips("More", listOfNotNull(
            if ((fa?.hdr ?: 0) > 0 || f.hdr) Triple("HDR", fa?.hdr, f.hdr) else null,
            Triple("Recently Added", fa?.recentlyAdded?.count, f.recentlyAdded),
        )) { l -> state.apply(if (l == "HDR") f.copy(hdr = !f.hdr) else f.copy(recentlyAdded = !f.recentlyAdded)) }
    }
}


/**
 * A category's page (`LibraryCategoryView`): its tiles (genres, collections, studios or networks,
 * decades) with their artwork and counts; a tile opens the library's grid with that filter.
 */
@Composable
fun CategoryScreen(route: Route.Category, pad: Boolean = false) {
    val nav = LocalNav.current
    var facets by remember { mutableStateOf<LibraryFacets?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var reloads by remember { mutableIntStateOf(0) }
    LaunchedEffect(reloads) {
        val api = Session.api ?: return@LaunchedEffect
        try { facets = api.facets(route.view, LibraryFilter()); error = null } catch (e: Exception) { if (ClientRules.isLoadFailure(e)) error = errorText(e) }
    }
    AutoRetry(error) { reloads++ }
    val title = route.category.title(route.view.shows)
    PageBackground {
        Column(Modifier.fillMaxSize()) {
            val tiles = facets?.tiles(route.category).orEmpty()
            if (pad) PadHeaderRow(title, back = null) {
                if (tiles.isNotEmpty()) Text("${tiles.size} ${title.lowercase()}", style = Type.subheadline, color = Theme.secondaryText)
            } else Row(Modifier.fillMaxWidth().statusBarsPadding().padding(horizontal = Theme.margin).padding(top = 4.dp, bottom = 10.dp).height(44.dp),
                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                GlassCircle(Icons.AutoMirrored.Filled.ArrowBack, "Back") { nav.back() }
                Text(title, style = Type.title2.copy(fontWeight = FontWeight.Bold), color = Color.White)
            }
            when {
                error != null && facets == null -> ErrorPane(error!!, onRetry = { reloads++ })
                facets == null -> Box(Modifier.fillMaxWidth().padding(40.dp), contentAlignment = Alignment.Center) { CircularProgressIndicator(color = Color.White) }
                else -> LazyVerticalGrid(GridCells.Adaptive(if (pad) 165.dp else 160.dp), contentPadding = PaddingValues(start = Theme.margin, end = Theme.margin, top = if (pad) Theme.margin else 0.dp, bottom = Theme.pageBottom),
                    horizontalArrangement = Arrangement.spacedBy(if (pad) 14.dp else 12.dp), verticalArrangement = Arrangement.spacedBy(if (pad) 14.dp else 12.dp)) {
                    items(tiles, key = { it.id }) { t ->
                        Box(Modifier.fillMaxWidth().aspectRatio(16f / 9f).clip(RoundedCornerShape(14.dp)).background(Theme.surface)
                            .clickable { nav.push(Route.Filtered(route.view, t.filter, t.title)) }) {
                            RemoteImage(imageUrl(t.art, 300.dp), Modifier.fillMaxSize())
                            Box(Modifier.fillMaxSize().background(Brush.verticalGradient(0.3f to Color.Transparent, 1f to Color.Black.copy(alpha = 0.75f))))
                            Column(Modifier.align(Alignment.BottomStart).padding(10.dp)) {
                                Text(t.title, style = Type.headline, color = Color.White, maxLines = 2, overflow = TextOverflow.Ellipsis)
                                Text(if (pad) "${t.count} ${if (t.count == 1) (if (route.view.shows) "show" else "movie") else (if (route.view.shows) "shows" else "movies")}" else "${t.count}", style = Type.caption, color = if (pad) Color.White else Theme.secondaryText)
                            }
                        }
                    }
                }
            }
        }
    }
}
