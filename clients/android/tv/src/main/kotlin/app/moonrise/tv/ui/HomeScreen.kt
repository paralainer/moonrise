package app.moonrise.tv.ui

import android.view.KeyEvent
import androidx.compose.animation.Crossfade
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.ClickableSurfaceDefaults
import androidx.tv.material3.Surface
import androidx.tv.material3.Text
import app.moonrise.core.ClientRules
import app.moonrise.core.Item
import app.moonrise.core.MoonriseHttpException
import app.moonrise.core.runtimeText
import app.moonrise.core.techBadges
import androidx.compose.runtime.snapshotFlow
import coil.compose.AsyncImage
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import app.moonrise.shared.errorText
import app.moonrise.shared.AutoRetry
import app.moonrise.shared.Log
import app.moonrise.shared.Session
import app.moonrise.shared.WatchMarks
import app.moonrise.shared.Network

/** Home's Continue Watching row. */
const val CONTINUE_ROW = "continue"

/** How often coming back to the app (or to Home) loads Home again. */
const val REFRESH_MS = ClientRules.HOME_REFRESH_MS

/** A row of Home: landscape cards (Continue Watching) or posters (Recently Added). */
data class HomeRowData(val id: String, val title: String, val landscape: Boolean, val items: List<Item>)

/** Home's content, kept while a title page or the player is on top, so Back lands where you left. */
class HomeState {
    var rows by mutableStateOf<List<HomeRowData>>(emptyList())
    var hero by mutableStateOf<List<Item>>(emptyList())
    var heroIndex by mutableStateOf(0)
    var loading by mutableStateOf(false)
    /** The server has answered at least once since sign-in. */
    var loaded by mutableStateOf(false)
    var error by mutableStateOf<String?>(null)
    var unauthorized by mutableStateOf(false)
    /** The card last opened (row "hero" for the banner): focused again when Home comes back. */
    var opened: Opened? = null
    /** "<row>/<item>" of the card to focus now, carrying [restore] until it has focus. */
    var restoreKey by mutableStateOf<String?>(null)
    val restore = FocusRequester()
    /** Scroll positions of the page and of each row, kept while Home is covered. */
    var column = LazyListState()
        private set
    private val rowStates = mutableMapOf<String, LazyListState>()
    /** Each row's card focused last. */
    val lastInRow = androidx.compose.runtime.mutableStateMapOf<String, String>()
    fun rowState(id: String): LazyListState = rowStates.getOrPut(id) { LazyListState() }
    private var stale = true

    data class Opened(val row: String, val itemId: String, val index: Int)

    /**
     * The banner took focus on the first visit, and back from the title it opened (that opening).
     * Once each: the banner is a lazy item, composed again whenever it comes back into view (or the
     * focus search looks past the rows on screen), and each time it would take focus from the card
     * the viewer is on.
     */
    var bannerFocusedOnce = false
    var bannerRestoredFor: Opened? = null

    fun clear() {
        rows = emptyList(); hero = emptyList(); heroIndex = 0; opened = null; restoreKey = null; loaded = false; error = null; stale = true; lastLoad = 0L
        bannerFocusedOnce = false; bannerRestoredFor = null
        column = LazyListState(); rowStates.clear(); lastInRow.clear()
    }

    /** After playback: watch state may have changed. */
    fun refreshSoon() { stale = true }

    /** Reloads when the last load failed (the network came back, the app came to the front). */
    var retries by mutableStateOf(0)
    fun retryIfFailed() { if (error != null && !loading) retries++ }

    /** When the last load started ([SystemClock.elapsedRealtime]), for [refreshIfStale]. */
    private var lastLoad = 0L

    /**
     * Back in front (or back on Home): load again, so what another device played shows up here and
     * in Watch Next. Not while a load runs, at most every [REFRESH_MS] unless the last one failed.
     * Rows that come back unchanged are equal, so nothing recomposes and focus stays put.
     */
    fun refreshIfStale() {
        // Never loaded yet: the first load is on its way.
        if (!ClientRules.homeIsStale(loading, error != null, lastLoad, android.os.SystemClock.elapsedRealtime())) return
        stale = true
        retries++
    }

    private var seenMarks = -1

    suspend fun loadIfNeeded() {
        if (!stale && rows.isNotEmpty() && seenMarks == WatchMarks.generation && error == null) return
        val api = Session.api ?: return
        seenMarks = WatchMarks.generation
        loading = true
        lastLoad = android.os.SystemClock.elapsedRealtime()
        try {
            val home = api.home(limit = app.moonrise.core.WatchNextRules.MAX)
            val out = mutableListOf<HomeRowData>()
            // As `HomeStore.fetch` on Apple TV: Continue Watching (the server's, the same on every
            // device), then Recently Added by type.
            val cont = home.continueRow
            if (cont.isNotEmpty()) out += HomeRowData(CONTINUE_ROW, "Continue Watching", true, cont)
            val heroOut = mutableListOf<Item>()
            for (row in home.latestTypes) {
                if (row.items.isEmpty()) continue
                out += HomeRowData("latest-${row.type}", "Recently Added ${row.title}", false, row.items)
                if (heroOut.size < 6) heroOut += row.items.filter { !it.isEpisode && it.kind != "season" && it.backdropImage != null }.take(3)
            }
            // A row scrolled to its start stays there when titles come in at the front (a handoff:
            // played on another device), rather than keeping its old first card in place.
            val atStart = rows.associate { it.id to it.items.firstOrNull()?.id }.filter { (id, first) ->
                val st = rowStates[id]
                st != null && st.firstVisibleItemIndex == 0 && st.firstVisibleItemScrollOffset == 0 &&
                    out.firstOrNull { it.id == id }?.items?.firstOrNull()?.id.let { it != null && it != first }
            }.keys
            rows = out
            hero = heroOut
            if (atStart.isNotEmpty()) {
                // Once the new cards are laid out (a scroll before that keeps the old first card).
                androidx.compose.runtime.withFrameNanos {}
                androidx.compose.runtime.withFrameNanos {}
                for (id in atStart) runCatching { rowState(id).scrollToItem(0) }
            }
            if (heroIndex >= heroOut.size) heroIndex = 0
            loaded = true
            error = null
            stale = false
            // The launcher's Watch Next row follows the same answer.
            app.moonrise.tv.WatchNext.syncFrom(home, "home loaded")
        } catch (e: MoonriseHttpException) {
            if (e.code == 401) unauthorized = true
            error = e.message
        } catch (e: Exception) {
            Log.w("Moonrise", "home failed: ${e.message}")
            error = errorText(e)
        } finally {
            loading = false
        }
    }
}

@Composable
fun HomeScreen(state: HomeState, open: (Item) -> Unit, signOut: () -> Unit, atTop: (Boolean) -> Unit = {}, main: MainState? = null) {
    // Down from the tab bar, when the focus search finds nothing: the banner (or the first row).
    EnterContent(main) {
        if (state.rows.isEmpty()) return@EnterContent false
        runCatching { state.column.scrollToItem(0) }
        if (state.hero.isEmpty()) runCatching { state.rowState(state.rows.first().id).scrollToItem(0) }
        main?.entry?.focusSoon(20) ?: false
    }
    LaunchedEffect(state.retries, WatchMarks.generation) { state.loadIfNeeded() }
    // Back on Home from a title page: catch up with other devices too (throttled).
    LaunchedEffect(Unit) { state.refreshIfStale() }
    // The network came back (say the TV woke before its Wi-Fi): a failed load runs again.
    LaunchedEffect(Network.generation) { state.retryIfFailed() }
    AutoRetry(state.error) { state.retryIfFailed() }
    val listState = state.column
    LaunchedEffect(listState) {
        snapshotFlow { listState.firstVisibleItemIndex == 0 && listState.firstVisibleItemScrollOffset < 40 }.collect(atTop)
    }
    // Back from a title page: focus the card that was opened, in its row, scrolled into view.
    // (Not when the tab was switched to on the tab bar: focus stays on the bar.)
    LaunchedEffect(state.rows) { if (main?.tabSwitched != true) restoreFocus(state) }
    var menuFor by remember { mutableStateOf<Pair<Item, String>?>(null) }
    menuFor?.let { (item, row) ->
        // Continue Watching: also Remove from Continue Watching (for every device).
        val extra = if (row == CONTINUE_ROW) listOf(MenuRow("Remove from Continue Watching", destructive = true) {
            Session.ui.launch { WatchMarks.removeFromContinue(item) }
        }) else emptyList()
        WatchMenu(item, onDismiss = { menuFor = null }, extra = extra, done = {})
    }

    if (state.rows.isEmpty()) {
        when {
            state.error != null -> ErrorPane(
                state.error!!, onRetry = { state.retries++ },
                signIn = if (state.unauthorized) signOut else null,
            )
            ClientRules.homeIsEmpty(state.loaded, state.loading, state.error != null, state.rows.size) ->
                Box(Modifier.fillMaxSize().padding(Theme.margin), contentAlignment = Alignment.Center) {
                    EmptyState("Nothing Here", "This server has nothing to show yet.")
                }
            else -> Column(Modifier.fillMaxSize().padding(Theme.margin), verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
                Text("Loading…", color = Theme.secondaryText)
            }
        }
        return
    }

    LazyColumn(
        state = listState,
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(bottom = 40.dp),
        verticalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        if (state.hero.isNotEmpty()) {
            item(key = "hero") { HeroBanner(state, open, main) }
        }
        itemsIndexed(state.rows, key = { _, it -> it.id }) { i, row ->
            Shelf(row, state, open, menu = { menuFor = it to row.id }, entry = if (i == 0 && state.hero.isEmpty()) main?.entry else null)
        }
    }
}

/** One row: its title at the margin, then cards that scroll sideways. */
@OptIn(ExperimentalComposeUiApi::class)
@Composable
private fun Shelf(row: HomeRowData, state: HomeState, open: (Item) -> Unit, menu: (Item) -> Unit, entry: FocusRequester? = null) {
    Column {
        Text(
            row.title, fontSize = 18.sp, fontWeight = FontWeight.Bold, color = Color.White,
            modifier = Modifier.padding(start = Theme.margin, bottom = 10.dp),
        )
        // Coming back into the row focuses the card left last; after Home was covered, the card
        // that was opened; else the first one. (Not focusRestorer: it pins a card, and crashed
        // when a row's cards changed.)
        val last = remember { FocusRequester() }
        val lastId = state.lastInRow[row.id]?.takeIf { id -> row.items.any { it.id == id } } ?: row.items.firstOrNull()?.id
        val restoring = state.restoreKey?.startsWith("${row.id}/") == true
        LazyRow(
            state = state.rowState(row.id),
            modifier = Modifier.enterOn(if (restoring) state.restore else last) {
                restoring || state.rowState(row.id).layoutInfo.visibleItemsInfo.any { it.key == lastId }
            },
            contentPadding = PaddingValues(horizontal = Theme.margin, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(Theme.rowSpacing),
        ) {
            itemsIndexed(row.items, key = { _, it -> it.id }) { index, item ->
                val key = "${row.id}/${item.id}"
                var m: Modifier = Modifier.rowEdge(index, row.items.lastIndex).onFocusChanged { if (it.isFocused) state.lastInRow[row.id] = item.id }
                if (item.id == lastId) m = m.focusRequester(last)
                if (state.restoreKey == key) m = m.focusRequester(state.restore)
                if (entry != null && index == 0) m = m.focusRequester(entry)
                val click = { state.opened = HomeState.Opened(row.id, item.id, index); open(item) }
                if (row.landscape) LandscapeCard(item, click, m, onLongClick = { menu(item) })
                else PosterCard(item, click, m, onLongClick = { menu(item) })
            }
        }
    }
}

/**
 * Focuses the card opened last (or the one now at its place, if the rows changed), after
 * scrolling the page and its row so it's on screen. Lazy rows compose their cards during layout,
 * so this waits a few frames for the card to be there.
 */
private suspend fun restoreFocus(state: HomeState) {
    val o = state.opened ?: return
    if (o.row == "hero" || state.rows.isEmpty()) return
    val r = state.rows.indexOfFirst { it.id == o.row }.takeIf { it >= 0 } ?: return
    val row = state.rows[r]
    if (row.items.isEmpty()) return
    val index = row.items.indexOfFirst { it.id == o.itemId }.takeIf { it >= 0 } ?: o.index.coerceIn(0, row.items.lastIndex)
    state.restoreKey = "${row.id}/${row.items[index].id}"
    withFrameNanos { }
    val columnIndex = r + if (state.hero.isNotEmpty()) 1 else 0
    if (state.column.layoutInfo.visibleItemsInfo.none { it.index == columnIndex }) state.column.scrollToItem(columnIndex)
    withFrameNanos { }
    val rowState = state.rowState(row.id)
    if (rowState.layoutInfo.visibleItemsInfo.none { it.index == index }) rowState.scrollToItem(index)
    repeat(30) {
        withFrameNanos { }
        if (runCatching { state.restore.requestFocus() }.isSuccess) {
            Log.i("Moonrise") { "focus back on ${row.id} #$index" }
            withFrameNanos { }
            state.restoreKey = null
            return
        }
    }
    state.restoreKey = null
}

/**
 * The banner at the top (`HeroCarousel`, tvOS): one focusable banner. Left/right switch titles,
 * select opens the title; it moves on by itself every 7 s while not focused.
 */
@Composable
private fun HeroBanner(state: HomeState, open: (Item) -> Unit, main: MainState? = null) {
    val entry = main?.entry
    val items = state.hero
    // (From the focus system itself: focus interactions could be missed.)
    var focused by remember { mutableStateOf(false) }
    val requester = remember { FocusRequester() }
    val current = items[state.heroIndex.coerceIn(0, items.lastIndex)]

    LaunchedEffect(focused, items) {
        while (!focused && items.size > 1) {
            delay(7000)
            state.heroIndex = (state.heroIndex + 1) % items.size
        }
    }
    // First visit: focus starts on the banner, as on Apple TV.
    // (Not when Home was switched to on the tab bar: focus stays on the bar.)
    LaunchedEffect(Unit) {
        if (main?.tabSwitched == true) return@LaunchedEffect
        val opened = state.opened
        val first = opened == null && !state.bannerFocusedOnce
        val back = opened?.row == "hero" && opened !== state.bannerRestoredFor
        if (!first && !back) return@LaunchedEffect
        state.bannerFocusedOnce = true
        state.bannerRestoredFor = opened
        runCatching { requester.requestFocus() }
    }

    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Surface(
            onClick = { state.opened = HomeState.Opened("hero", current.id, 0); open(current) },
            modifier = Modifier
                .fillMaxWidth()
                .focusRequester(requester)
                .onFocusChanged { focused = it.isFocused }
                .then(if (entry != null) Modifier.focusRequester(entry) else Modifier)
                .logFocus { "banner" }
                .onPreviewKeyEvent { e ->
                    if (e.nativeKeyEvent.action != KeyEvent.ACTION_DOWN) return@onPreviewKeyEvent false
                    when (e.nativeKeyEvent.keyCode) {
                        KeyEvent.KEYCODE_DPAD_LEFT -> { state.heroIndex = (state.heroIndex - 1 + items.size) % items.size; true }
                        KeyEvent.KEYCODE_DPAD_RIGHT -> { state.heroIndex = (state.heroIndex + 1) % items.size; true }
                        else -> false
                    }
                },
            shape = ClickableSurfaceDefaults.shape(RoundedCornerShape(0.dp)),
            scale = ClickableSurfaceDefaults.scale(focusedScale = 1f),
            colors = ClickableSurfaceDefaults.colors(containerColor = Color.Transparent, focusedContainerColor = Color.Transparent),
        ) {
            Crossfade(current, label = "hero") { item ->
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    TitleHeader(item, height = HEADER_HEIGHT)
                    Spacer(Modifier.height(9.dp))
                    // Always two lines tall, with or without genres or badges: the banner moves on
                    // by itself while the viewer is in the rows below, and a banner changing height
                    // moved every row on screen.
                    Box(contentAlignment = Alignment.TopCenter) {
                        Column(Modifier.alpha(0f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                            Text(" ", fontSize = 13.sp); Text(" ", fontSize = 13.sp)
                        }
                        TitleMeta(item)
                    }
                    Spacer(Modifier.height(9.dp))
                    // The remote needs to show what's focused: a "More Info" pill lights up on the banner.
                    Text(
                        "ⓘ  More Info", fontSize = 13.sp, fontWeight = FontWeight.SemiBold,
                        color = if (focused) Color.Black else Color.White,
                        modifier = Modifier
                            .background(if (focused) Color.White else Color.White.copy(alpha = 0.15f), RoundedCornerShape(50))
                            .padding(horizontal = 13.dp, vertical = 6.dp),
                    )
                }
            }
        }
        Spacer(Modifier.height(10.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            items.indices.forEach { i ->
                Box(Modifier.size(6.dp).background(if (i == state.heroIndex) Color.White else Color.White.copy(alpha = 0.3f), CircleShape))
            }
        }
    }
}

/** The tvOS header height (620 pt of 1080). */
val HEADER_HEIGHT = 310.dp

/**
 * The picture across the screen, fading into the background, with the title's logo (or its name)
 * centred at the bottom (`TitleHeader`).
 */
@Composable
fun TitleHeader(item: Item, height: androidx.compose.ui.unit.Dp) {
    Box(Modifier.fillMaxWidth().height(height), contentAlignment = Alignment.BottomCenter) {
        imageUrl(item.backdropImage, 960.dp)?.let {
            AsyncImage(model = it, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
        }
        Box(
            Modifier.fillMaxSize().background(
                Brush.verticalGradient(0.3f to Color.Transparent, 0.75f to Theme.background.copy(alpha = 0.6f), 1f to Theme.background),
            ),
        )
        val logo = imageUrl(item.logoImage, 300.dp)
        val title = if (item.isEpisode) item.seriesTitle ?: item.title.orEmpty() else item.title.orEmpty()
        if (logo != null) {
            AsyncImage(
                model = logo, contentDescription = title, contentScale = ContentScale.Fit,
                modifier = Modifier.padding(bottom = 3.dp).size(width = 150.dp, height = 55.dp),
            )
        } else {
            Text(title, fontSize = 30.sp, fontWeight = FontWeight.Bold, color = Color.White, textAlign = TextAlign.Center, modifier = Modifier.padding(horizontal = Theme.margin, vertical = 3.dp))
        }
    }
}

/** Runtime · year · rating and tech badges, then genres (`TitleMeta`). */
@Composable
fun TitleMeta(item: Item, focus: Item? = null) {
    val f = focus ?: item
    Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(3.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(5.dp), verticalAlignment = Alignment.CenterVertically) {
            runtimeText(f.runtime)?.let { Text(it, fontSize = 13.sp, color = Theme.secondaryText) }
            item.year?.let { Text("$it", fontSize = 13.sp, color = Theme.secondaryText) }
            (item.officialRating ?: f.officialRating)?.let { Badge(it) }
            f.techBadges.take(3).forEach { Badge(it) }
        }
        item.genres?.takeIf { it.isNotEmpty() }?.let {
            Text(it.take(3).joinToString(", "), fontSize = 13.sp, color = Theme.secondaryText, maxLines = 1)
        }
    }
}

@Composable
fun Badge(text: String) {
    Text(
        text, fontSize = 10.sp, fontWeight = FontWeight.SemiBold, color = Theme.secondaryText,
        modifier = Modifier
            .background(Color.White.copy(alpha = 0.12f), RoundedCornerShape(3.dp))
            .padding(horizontal = 4.dp, vertical = 1.dp),
    )
}
