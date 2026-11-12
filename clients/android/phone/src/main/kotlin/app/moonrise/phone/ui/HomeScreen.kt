package app.moonrise.phone.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.VideoLibrary
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import app.moonrise.core.ClientRules
import app.moonrise.core.Item
import app.moonrise.core.LayoutMetrics
import app.moonrise.core.MoonriseHttpException
import app.moonrise.phone.Autoplay
import app.moonrise.phone.LocalNav
import app.moonrise.phone.Route
import app.moonrise.shared.AutoRetry
import app.moonrise.shared.Log
import app.moonrise.shared.Network
import app.moonrise.shared.Session
import app.moonrise.shared.WatchMarks
import app.moonrise.shared.downloads.Downloads
import app.moonrise.shared.downloads.ServerReach
import app.moonrise.shared.errorText
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** A row of Home: landscape cards (Continue Watching) or posters (Recently Added). */
data class HomeSection(val id: String, val title: String, val landscape: Boolean, val items: List<Item>)

/**
 * Home's content (`HomeStore`): Continue Watching (the server's, the same on every device), then
 * Recently Added by type; the banner: up to six recently added titles with a backdrop. Kept while
 * title pages are on top, so Back lands on the same rows.
 */
object HomeStore {
    const val CONTINUE = "continue"
    var sections by mutableStateOf<List<HomeSection>>(emptyList())
    var hero by mutableStateOf<List<Item>>(emptyList())
    var loading by mutableStateOf(false)
    /** The server has answered at least once since sign-in. */
    var loaded by mutableStateOf(false)
    var error by mutableStateOf<String?>(null)
    var unauthorized by mutableStateOf(false)
    /** Bumped to load again (Retry, back on Home after playback or a mark). */
    var reloads by mutableIntStateOf(0)
    private var lastLoad = 0L
    private var seenMarks = -1

    fun clear() { sections = emptyList(); hero = emptyList(); loaded = false; error = null; unauthorized = false; lastLoad = 0L; seenMarks = -1 }

    fun reload() { reloads++ }

    /** Back in front: load again, at most every 30 s unless the last load failed (`ClientRules.homeIsStale`). */
    fun refreshIfStale() {
        if (ClientRules.homeIsStale(loading, error != null, lastLoad, android.os.SystemClock.elapsedRealtime())) reloads++
    }

    suspend fun load() {
        val api = Session.api ?: return
        loading = true
        lastLoad = android.os.SystemClock.elapsedRealtime()
        seenMarks = WatchMarks.generation
        try {
            val home = api.home()
            val out = mutableListOf<HomeSection>()
            val cont = home.continueRow
            if (cont.isNotEmpty()) out += HomeSection(CONTINUE, "Continue Watching", true, cont)
            val heroOut = mutableListOf<Item>()
            for (row in home.latestTypes) {
                if (row.items.isEmpty()) continue
                out += HomeSection("latest-${row.type}", "Recently Added ${row.title}", false, row.items)
                if (heroOut.size < 6) heroOut += row.items.filter { !it.isEpisode && !it.isSeason && it.backdropImage != null }.take(3)
            }
            sections = out
            hero = heroOut
            loaded = true
            error = null
            unauthorized = false
        } catch (e: Exception) {
            if (!ClientRules.isLoadFailure(e)) throw e
            Log.w("Moonrise", "home failed: ${e.message}")
            unauthorized = e is MoonriseHttpException && e.code == 401
            error = errorText(e)
        } finally {
            loading = false
        }
    }

    val marksChanged: Boolean get() = seenMarks != WatchMarks.generation
}

/**
 * Home (`HomeView`): the banner of recently added titles across the top, edge to edge under the
 * status bar (each page laid out as the top of the title page it opens), the Settings gear at the
 * top right, then the rows. Pull down to load again.
 */
@Composable
fun HomeScreen(signOut: () -> Unit) {
    val nav = LocalNav.current
    val store = HomeStore
    val scope = rememberCoroutineScope()
    LaunchedEffect(store.reloads) { store.load() }
    LaunchedEffect(Unit) { if (store.marksChanged && store.sections.isNotEmpty()) store.reload() }
    LaunchedEffect(Network.generation) { if (store.error != null) store.reload() }
    // Back online: Home loads again.
    LaunchedEffect(ServerReach.reachable) { if (ServerReach.reachable && store.error != null) store.reload() }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { store.refreshIfStale() }
    AutoRetry(store.error, busy = { store.loading }) { store.reload() }

    val problem = store.error?.takeIf { store.sections.isEmpty() }
    val hero = if (problem == null) store.hero else emptyList()
    var refreshing by remember { mutableStateOf(false) }
    Box(Modifier.fillMaxSize().background(Theme.background)) {
        PullToRefreshBox(isRefreshing = refreshing, onRefresh = { scope.launch { refreshing = true; store.load(); refreshing = false } }) {
            LazyColumn(Modifier.fillMaxSize(), state = rememberLazyListState(), verticalArrangement = Arrangement.spacedBy(28.dp)) {
                if (hero.isNotEmpty()) item("hero") { HeroCarousel(hero) }
                else item("title") {
                    Text("Home", style = Type.largeTitle, color = Color.White, modifier = Modifier.statusBarsPadding().padding(start = Theme.margin, top = 52.dp))
                }
                // The server doesn't answer: the downloads first, they play.
                if (!ServerReach.reachable && Downloads.hasPlayable) {
                    item("offline-banner") { OfflineBanner(retry = { store.reload() }, modifier = Modifier.padding(top = if (hero.isEmpty()) 8.dp else 0.dp)) }
                    item("offline-shelves") { OfflineDownloadShelves() }
                }
                if (problem != null) item("problem") { ErrorPane(problem, onRetry = { store.reload() }, signIn = if (store.unauthorized) signOut else null) }
                items(store.sections, key = { it.id }) { section -> HomeShelf(section) }
                if (problem == null && store.loading && store.sections.isEmpty()) item("loading") {
                    Box(Modifier.fillMaxWidth().padding(60.dp), contentAlignment = Alignment.Center) { CircularProgressIndicator(color = Color.White) }
                }
                if (ClientRules.homeIsEmpty(store.loaded, store.loading, store.error != null, store.sections.size)) item("empty") {
                    EmptyState(Icons.Outlined.VideoLibrary, "Nothing Here", "This server has nothing to show yet.")
                }
                item("end") { Spacer(Modifier.height(Theme.pageBottom)) }
            }
        }
        // A tablet has the gear in the top bar, on every tab.
        if (!LocalMetrics.current.regular) GlassCircle(Icons.Outlined.Settings, "Settings", Modifier.align(Alignment.TopEnd).statusBarsPadding().padding(top = 8.dp, end = Theme.margin)) { nav.settings = true }
    }
}

@Composable
private fun HomeShelf(section: HomeSection) {
    val nav = LocalNav.current
    val scope = rememberCoroutineScope()
    Shelf(section.title) {
        items(section.items, key = { it.id }) { item ->
            if (section.landscape) {
                LandscapeCard(item, onClick = { nav.push(Route.Title(item)) }, menu = CardMenu(
                    // The title's page opens and plays it (in its header).
                    play = { nav.push(Route.Title(item, Autoplay())) },
                    remove = if (section.id == HomeStore.CONTINUE) ({
                        // Hidden here at once; the server hides it on the other devices.
                        HomeStore.sections = HomeStore.sections.map { s -> if (s.id == section.id) s.copy(items = s.items.filter { it.id != item.id }) else s }.filter { it.items.isNotEmpty() }
                        scope.launch { WatchMarks.removeFromContinue(item); HomeStore.reload() }
                    }) else null,
                    marked = { HomeStore.reload() },
                ))
            } else {
                PosterCard(item, onClick = { nav.push(Route.Title(item)) }, menu = CardMenu(marked = { HomeStore.reload() }))
            }
        }
    }
}

/**
 * A tablet's banner page (`HeroPage.padBody`): laid out as the tablet's title page, the logo, line and
 * Play at the page margin (over the bottom of the picture when the window is wider than tall), with
 * More Info where the title page has Download.
 */
@Composable
private fun PadHeroPage(item: Item, metrics: LayoutMetrics) {
    val nav = LocalNav.current
    val image = imageUrl(item.backdropImage, metrics.width.dp)
    // Unbounded height: a wide window's page is shorter than the picture (the line and Play sit over it).
    Column(Modifier.fillMaxWidth().wrapContentHeight(Alignment.Top, unbounded = true), verticalArrangement = Arrangement.spacedBy(18.dp)) {
        Box(Modifier.clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) { nav.push(Route.Title(item)) }) {
            TitleHeader(image, imageUrl(item.logoImage, 380.dp), item.title.orEmpty())
        }
        Column(Modifier.padding(horizontal = Theme.margin).pullUp(metrics.headerOverlap(image != null).dp)) {
            TitleMeta(item, leading = true, modifier = Modifier.heightIn(min = 50.dp))
            Row(Modifier.padding(top = 16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                PlayCapsule(if (item.resumeSeconds == null) "Play" else "Resume", item.resumeSeconds, item.progress,
                    Modifier.width(LayoutMetrics.PLAY_WIDTH.dp).clickable { nav.push(Route.Title(item, Autoplay())) })
                GlassCircle(Icons.Outlined.Info, "More Info", size = LayoutMetrics.PLAY_HEIGHT.dp) { nav.push(Route.Title(item)) }
            }
        }
    }
}

/**
 * The banner (`HeroCarousel`): pages of recently added titles, each the header and info line of
 * its title page; a tap opens it. It moves on every 7 s while Home is on screen; dots below.
 */
@Composable
private fun HeroCarousel(items: List<Item>) {
    val nav = LocalNav.current
    val pager = rememberPagerState { items.size }
    LaunchedEffect(items.size) {
        while (items.size > 1) {
            delay(7000)
            pager.animateScrollToPage((pager.currentPage + 1) % items.size)
        }
    }
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        val metrics = LocalMetrics.current
        val pageHeight = if (metrics.regular) metrics.headerHeight(0.0).dp + 18.dp + metrics.heroInfoHeight.dp else Theme.headerHeight + 18.dp + 70.dp
        HorizontalPager(pager, Modifier.fillMaxWidth().height(pageHeight), verticalAlignment = Alignment.Top, key = { items[it].id }) { i ->
            val item = items[i]
            if (metrics.regular) { PadHeroPage(item, metrics); return@HorizontalPager }
            Column(Modifier.fillMaxSize().clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) { nav.push(Route.Title(item)) },
                horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(18.dp)) {
                TitleHeader(imageUrl(item.backdropImage, 600.dp), imageUrl(item.logoImage, 300.dp), item.title.orEmpty())
                TitleMeta(item, modifier = Modifier.padding(horizontal = Theme.margin))
            }
        }
        if (items.size > 1) Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(7.dp, Alignment.CenterHorizontally)) {
            items.indices.forEach { i ->
                Box(Modifier.size(6.dp).clip(CircleShape).background(Color.White.copy(alpha = if (i == pager.currentPage) 0.9f else 0.3f)))
            }
        }
    }
}
