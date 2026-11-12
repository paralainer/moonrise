package app.moonrise.tv.ui

import android.content.Context
import android.hardware.display.DisplayManager
import android.view.Display
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.wrapContentSize
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.layout.onPlaced
import androidx.compose.ui.layout.positionInParent
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.tv.material3.Glow
import androidx.tv.material3.Surface
import androidx.tv.material3.Text
import app.moonrise.core.Item
import app.moonrise.core.MediaFile
import app.moonrise.core.MoonriseHttpException
import app.moonrise.core.Versions
import app.moonrise.core.WatchAction
import app.moonrise.core.clockText
import app.moonrise.core.premiereText
import app.moonrise.core.ratingLine
import app.moonrise.core.runtimeText
import app.moonrise.core.techBadges
import app.moonrise.core.unplayableProblem
import coil.compose.AsyncImage
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import app.moonrise.shared.DetailsState
import app.moonrise.shared.AutoRetry
import app.moonrise.shared.displayHdr
import app.moonrise.shared.Log
import app.moonrise.shared.Session
import app.moonrise.shared.AppSettings
import app.moonrise.shared.WatchMarks
import app.moonrise.shared.VersionPicks
import app.moonrise.shared.Network

/**
 * A title's page, as the Apple TV's product page (`TVItemDetailView`): the backdrop across the
 * screen; above the fold the logo, the episode line, the title's line, the actions (Play or
 * Resume, Play from Beginning, Version, More), the overview and the credits; below it, for a
 * show, its seasons and their episodes, then the cast and crew.
 *
 * An episode or a season opens its show's page, about that episode or season.
 */
@Composable
fun DetailsScreen(state: DetailsState, play: (item: Item, start: Double?, version: String?) -> Unit, signIn: () -> Unit) {
    val context = LocalContext.current
    var reloads by remember { mutableIntStateOf(0) }

    // First time: load; back from the player (the page comes back), after a mark or a Retry: refresh.
    LaunchedEffect(reloads, WatchMarks.generation) { if (state.loaded) state.refresh() else state.load() }
    LaunchedEffect(Network.generation) { if (state.error != null) reloads++ }
    AutoRetry(state.error, busy = { state.loading }) { reloads++ }

    val show = state.show
    // Not loaded (the server can't be reached): the reason and Retry, not the card's data with a
    // Play that can't work. A refresh failing on a loaded page keeps the page.
    val e = state.error
    if (e != null && !state.loaded) {
        ErrorPane(e, onRetry = { reloads++ }, signIn = if (state.unauthorized) signIn else null)
        return
    }
    if (show == null) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text("Loading…", color = Theme.secondaryText) }
        return
    }

    val scroll = rememberScrollState()
    val pageScope = rememberCoroutineScope()
    var belowTop by remember { mutableIntStateOf(0) }
    val topGap = with(LocalDensity.current) { 30.dp.roundToPx() }
    val belowFold by remember { derivedOf { scroll.value > 40 } }
    var showOverview by remember { mutableStateOf(false) }
    var menu by remember { mutableStateOf<Menu?>(null) }
    val focus = state.focus
    val hdr = remember { displayHdr(context) }
    val language = AppSettings.audioLanguage.ifEmpty { java.util.Locale.getDefault().language }
    val version = focus?.let { f -> if (f.hasVersions) Versions.chosen(f, VersionPicks.picks[f.id], hdr, language) else null }
    val resume = focus?.let { Versions.resume(it, version) }

    fun playFocus(fromStart: Boolean) {
        val f = focus ?: return
        state.returnTo = "play"
        play(f, if (fromStart) 0.0 else resume ?: 0.0, version?.id)
    }

    Box(Modifier.fillMaxSize()) {
        Backdrop(imageUrl(show.backdropImage, 960.dp), belowFold)
        val height = LocalConfiguration.current.screenHeightDp.dp
        @OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
        androidx.compose.runtime.CompositionLocalProvider(androidx.compose.foundation.gestures.LocalBringIntoViewSpec provides MinimalScroll) {
        Column(Modifier.fillMaxSize().verticalScroll(scroll).padding(bottom = 40.dp)) {
            // Above the fold: the showcase at the bottom of 90 % of the screen.
            Column(
                Modifier.fillMaxWidth().heightIn(min = height * 0.9f).padding(horizontal = Theme.margin)
                    .onFocusChanged { if (it.hasFocus) pageScope.launch { scroll.animateScrollTo(0) } },
                verticalArrangement = Arrangement.Bottom,
            ) {
                Showcase(state, show, focus, version, resume,
                    onPlay = { playFocus(false) }, onRestart = { playFocus(true) },
                    onVersion = { menu = Menu.Version }, onMore = { menu = Menu.More },
                    onOverview = { showOverview = true })
            }
            if (state.isShow) {
                Spacer(Modifier.height(35.dp))
                // Below the fold, as on Apple TV: focus moving into the seasons and episodes
                // brings them up the screen; back up on the showcase, the page goes back to its top.
                Column(Modifier
                    .onPlaced { belowTop = it.positionInParent().y.toInt() + scroll.value }
                    .onFocusChanged { f ->
                        if (f.hasFocus) pageScope.launch { scroll.animateScrollTo(maxOf(0, belowTop - topGap)) }
                    }) {
                SeasonsAndEpisodes(state, play = { ep, fromStart ->
                    state.returnTo = "ep:${ep.id}"
                    val v = if (ep.hasVersions) Versions.chosen(ep, VersionPicks.picks[ep.id], hdr, language) else null
                    play(ep, if (fromStart) 0.0 else Versions.resume(ep, v) ?: 0.0, v?.id)
                }, menu = { menu = it })
                }
            }
            val people = show.people.orEmpty().filter { it.group in setOf("Actor", "Director", "Writer") }.take(20)
            if (people.isNotEmpty()) {
                Spacer(Modifier.height(35.dp))
                Shelf("Cast & Crew", people, key = { it.id + it.group + (it.role ?: "") }) { _, p -> PersonCard(p) }
            }
        }
        }
    }

    if (showOverview) {
        Dialog(onDismissRequest = { showOverview = false }, properties = DialogProperties(usePlatformDefaultWidth = false)) {
            val f = remember { FocusRequester() }
            Column(
                Modifier.fillMaxSize().background(Color(0xF0121318)).padding(horizontal = 80.dp, vertical = 40.dp)
                    .focusRequester(f).focusable().verticalScroll(rememberScrollState()),
            ) {
                Text((focus ?: show).let { if (it.isEpisode) it.title ?: "" else it.title ?: "" }, fontSize = 24.sp, fontWeight = FontWeight.Bold)
                Spacer(Modifier.height(16.dp))
                Text(focus?.overview ?: show.overview ?: "", fontSize = 16.sp, lineHeight = 24.sp)
            }
            LaunchedEffect(Unit) { runCatching { f.requestFocus() } }
        }
    }

    when (val m = menu) {
        null -> {}
        Menu.Version -> focus?.let { f ->
            val current = version?.id
            MenuDialog(
                title = "Version", onDismiss = { menu = null }, width = 440.dp,
                sections = listOf(MenuSection(rows = f.presentFiles.map { v ->
                    // A damaged file can't play: shown, greyed out (the server plays a healthy one).
                    MenuRow(v.versionTitle, v.versionDetails, checked = v.id == current, enabled = !v.isBroken || v.id == current) {
                        VersionPicks.picks[f.id] = v.id
                        Session.ui.launch { runCatching { Session.api?.chooseVersion(f.id, v.id) } }
                        Log.i("Moonrise") { "version ${v.id} chosen for ${f.id}" }
                    }
                })),
            )
        }
        Menu.More -> focus?.let { f -> WatchMenu(f, onDismiss = { menu = null }) }
        is Menu.Watch -> WatchMenu(m.item, onDismiss = { menu = null }, extra = m.extra)
    }
}

private sealed interface Menu {
    data object Version : Menu
    data object More : Menu
    data class Watch(val item: Item, val extra: List<MenuRow> = emptyList()) : Menu
}

private fun <T> derivedOf(calc: () -> T) = androidx.compose.runtime.derivedStateOf(calc)


/** Full-screen artwork: clear at the top above the fold, darker below it (`TVBackdrop`). */
@Composable
private fun Backdrop(url: String?, belowFold: Boolean) {
    val dim by animateFloatAsState(if (belowFold) 1f else 0f, label = "fold")
    Box(Modifier.fillMaxSize().background(Theme.background)) {
        if (url != null) AsyncImage(model = url, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
        Box(Modifier.fillMaxSize().background(Brush.verticalGradient(
            0f to Color.Transparent, 0.4f to Theme.background.copy(alpha = 0.25f + 0.6f * dim), 0.8f to Theme.background.copy(alpha = 0.92f), 1f to Theme.background,
        )))
        Box(Modifier.fillMaxSize().background(Brush.horizontalGradient(0f to Color.Black.copy(alpha = 0.55f), 0.6f to Color.Black.copy(alpha = 0.1f), 1f to Color.Transparent)))
        Box(Modifier.fillMaxSize().background(Theme.background.copy(alpha = 0.55f * dim)))
    }
}

@Composable
private fun Showcase(
    state: DetailsState, show: Item, focus: Item?, version: MediaFile?, resume: Double?,
    onPlay: () -> Unit, onRestart: () -> Unit, onVersion: () -> Unit, onMore: () -> Unit, onOverview: () -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(13.dp)) {
        val logo = imageUrl(show.logoImage, 300.dp)
        if (logo != null) {
            AsyncImage(model = logo, contentDescription = show.title, contentScale = ContentScale.Fit, alignment = Alignment.CenterStart,
                modifier = Modifier.size(width = 300.dp, height = 105.dp))
        } else {
            Text(show.title ?: "Untitled", fontSize = 38.sp, fontWeight = FontWeight.Bold, maxLines = 2, overflow = TextOverflow.Ellipsis, lineHeight = 42.sp)
        }
        val e = state.current
        if (state.isShow && e != null) {
            Text(listOfNotNull(e.episodeLabel, e.title).joinToString(" · "), fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
        }
        // (Another version chosen: its badges and length.)
        MetaLine(focus?.let { f -> version?.let { v -> f.asVersion(v) } ?: f } ?: show, show)
        Row(horizontalArrangement = Arrangement.spacedBy(30.dp)) {
            Column(Modifier.width(300.dp)) {
                Actions(state, focus, version, resume, onPlay, onRestart, onVersion, onMore)
                if (focus != null && focus.hasVersions && version != null) {
                    Text(buildVersionLine(version.versionTitle), fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.padding(top = 28.dp))
                }
            }
            (focus?.overview ?: show.overview)?.let { text ->
                FocusSurface(
                    onClick = onOverview,
                    modifier = Modifier.widthIn(max = 380.dp).weight(1f, fill = false).logFocus { "title page: overview" },
                    shape = RoundedCornerShape(12.dp),
                    focusedScale = 1.03f,
                    containerColor = Color.Transparent, contentColor = Color.White,
                        focusedContainerColor = Color.White.copy(alpha = 0.16f), focusedContentColor = Color.White,
                ) {
                    Text(text, fontSize = 13.sp, lineHeight = 18.sp, maxLines = 6, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(8.dp))
                }
            }
            Credits(show)
        }
    }
}

private fun buildVersionLine(v: String) = androidx.compose.ui.text.buildAnnotatedString {
    pushStyle(androidx.compose.ui.text.SpanStyle(color = Theme.secondaryText)); append("Version: "); pop()
    append(v)
}

/** The title as its chosen version is: its file first (the badges read it) and its length. */
private fun Item.asVersion(v: MediaFile): Item =
    copy(files = listOf(v) + files.orEmpty().filter { it.id != v.id }, runtime = v.duration ?: runtime)

/** Runtime, premiere date (or year), rating and tech badges, genres: one line, left aligned (`TVMetaLine`). */
@Composable
private fun MetaLine(item: Item, show: Item) {
    Row(horizontalArrangement = Arrangement.spacedBy(7.dp), verticalAlignment = Alignment.CenterVertically) {
        runtimeText(item.runtime)?.let { Text(it, fontSize = 12.sp, color = Theme.secondaryText) }
        (premiereText(item.premiereDate) ?: item.year?.toString())?.let { Text(it, fontSize = 12.sp, color = Theme.secondaryText) }
        (item.officialRating ?: show.officialRating)?.let { Badge(it) }
        item.techBadges.take(4).forEach { Badge(it) }
        (item.genres?.takeIf { it.isNotEmpty() } ?: show.genres)?.take(3)?.takeIf { it.isNotEmpty() }?.let {
            Text(it.joinToString(", "), fontSize = 12.sp, color = Theme.secondaryText, maxLines = 1)
        }
    }
}

/** Starring, Director and Rating, beside the overview. */
@Composable
private fun Credits(show: Item) {
    val people = show.people.orEmpty()
    val actors = people.filter { it.group == "Actor" }.take(4).mapNotNull { it.name }
    val directors = people.filter { it.group == "Director" }.take(2).mapNotNull { it.name }
    if (actors.isEmpty() && directors.isEmpty()) return
    Column(Modifier.widthIn(max = 190.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        @Composable fun line(title: String, value: String) {
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(title, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, color = Theme.secondaryText)
                Text(value, fontSize = 11.sp, lineHeight = 15.sp, maxLines = 3, overflow = TextOverflow.Ellipsis)
            }
        }
        if (actors.isNotEmpty()) line("Starring", actors.joinToString(", "))
        if (directors.isNotEmpty()) line("Director", directors.joinToString(", "))
        // The film's or show's (as the cast is): TMDB's star, then IMDb, Rotten Tomatoes, Metacritic.
        ratingLine(show.communityRating, show.ratings)?.let { line("Rating", it) }
    }
}

/**
 * The action row (`actions`): one wide Play/Resume button, then round buttons for Play from
 * Beginning (with a resume position), Version (several versions) and More (Mark as Watched /
 * Unwatched). A focused round button names itself underneath.
 */
@Composable
private fun Actions(
    state: DetailsState, focus: Item?, version: MediaFile?, resume: Double?,
    onPlay: () -> Unit, onRestart: () -> Unit, onVersion: () -> Unit, onMore: () -> Unit,
) {
    val f = focus ?: return
    if (!f.isPlayable) return
    val playFocus = remember { FocusRequester() }
    // No file can play (damaged, or gone from disk): what's wrong instead of Play.
    f.unplayableProblem()?.let { p ->
        Column {
            Text(p.title, color = Color.White, fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
            Text(p.message, color = Theme.secondaryText, fontSize = 13.sp, modifier = Modifier.widthIn(max = 520.dp).padding(top = 2.dp))
        }
        return
    }
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
        GlassButton(onClick = onPlay, modifier = Modifier.focusRequester(playFocus).widthIn(min = 150.dp), name = "title page: ${if (resume != null) "Resume" else "Play"}") {
            Text(if (resume != null) "▶  Resume ${clockText(resume)}" else "▶  Play", fontWeight = FontWeight.Bold, maxLines = 1)
        }
        if (resume != null) RoundButton("↺", "Play from Beginning", onRestart)
        if (f.hasVersions && version != null) RoundButton("❐", "Version", onVersion)
        RoundButton("•••", "More", onMore)
    }
    // The page opens on Play, and comes back to it from the player. (Not when the episode it's
    // about changes: moving along the episodes changes it, and focus stays there.)
    LaunchedEffect(Unit) {
        if (state.returnTo == null || state.returnTo == "play") {
            delay(30)
            runCatching { playFocus.requestFocus() }
        }
    }
}

@Composable
private fun RoundButton(symbol: String, caption: String, onClick: () -> Unit) {
    var focused by remember { mutableStateOf(false) }
    // The caption is drawn outside the button's 40 dp (nothing in the row moves).
    Box(Modifier.size(40.dp), contentAlignment = Alignment.TopCenter) {
        FocusSurface(
            onClick = onClick,
            modifier = Modifier.fillMaxSize().onFocusChanged { focused = it.isFocused }.logFocus { "title page: $caption" },
            shape = CircleShape,
            focusedScale = 1.1f,
            focusedGlow = Glow(Color.White.copy(alpha = 0.25f), 8.dp),
            containerColor = Color.White.copy(alpha = 0.14f), contentColor = Color.White,
                focusedContainerColor = Color.White, focusedContentColor = Color.Black,
        ) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text(symbol, fontSize = 15.sp, fontWeight = FontWeight.Bold) }
        }
        // Under the button, outside the layout, so nothing moves.
        if (focused) {
            Text(caption, fontSize = 10.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, softWrap = false,
                modifier = Modifier.offset(y = 48.dp).wrapContentSize(unbounded = true))
        }
    }
}


/**
 * Below the fold (`seasonsAndEpisodes`): the seasons (moving onto one shows its episodes; long
 * press marks it), then the episodes (select plays; long press: Mark as Watched / Unwatched, Play
 * from Beginning). Coming down into the episodes lands on the current one.
 */
@Composable
private fun SeasonsAndEpisodes(state: DetailsState, play: (Item, fromStart: Boolean) -> Unit, menu: (Menu) -> Unit) {
    val scope = rememberCoroutineScope()
    var switchJob by remember { mutableStateOf<kotlinx.coroutines.Job?>(null) }
    val seasons = state.seasons
    if (seasons.size > 1) {
        val seasonFocus = remember { FocusRequester() }
        val seasonRow = rememberLazyListState()
        LazyRow(
            state = seasonRow,
            modifier = Modifier.enterOn(seasonFocus) { seasonRow.layoutInfo.visibleItemsInfo.any { it.key == state.selectedSeason?.id } },
            contentPadding = PaddingValues(horizontal = Theme.margin, vertical = 6.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            itemsIndexed(seasons, key = { _, s -> s.id }) { si, s ->
                val selected = s.id == state.selectedSeason?.id
                FocusSurface(
                    onClick = { scope.launch { state.selectSeason(s) } },
                    modifier = Modifier.rowEdge(si, seasons.lastIndex).then(if (selected) Modifier.focusRequester(seasonFocus) else Modifier)
                        .selectOrHold(onClick = { scope.launch { state.selectSeason(s) } }, onLongClick = { menu(Menu.Watch(s)) })
                        // Moving onto a season shows its episodes (after a moment, so sweeping
                        // across the row doesn't load every season on the way).
                        .onFocusChanged { fs ->
                            if (fs.isFocused && s.id != state.selectedSeason?.id) {
                                switchJob?.cancel()
                                switchJob = scope.launch { delay(350); state.selectSeason(s) }
                            }
                        },
                    shape = RoundedCornerShape(50),
                    focusedScale = 1.05f,
                    containerColor = if (selected) Color.White.copy(alpha = 0.16f) else Color.Transparent, contentColor = Color.White,
                        focusedContainerColor = Color.White, focusedContentColor = Color.Black,
                ) {
                    Text(s.title ?: "Season ${s.seasonNo ?: ""}", fontSize = 14.sp, fontWeight = if (selected) FontWeight.Bold else FontWeight.Normal,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 7.dp))
                }
            }
        }
        Spacer(Modifier.height(10.dp))
    }
    val eps = state.episodes
    if (eps.isEmpty()) return
    val rowState = rememberLazyListState()
    val currentFocus = remember { FocusRequester() }
    val currentIndex = eps.indexOfFirst { it.id == state.current?.id }.coerceAtLeast(0)
    // Keep the current episode in view, so focus coming down lands on it.
    LaunchedEffect(state.episodesVersion) { runCatching { rowState.scrollToItem(currentIndex) } }
    // Back from playing an episode: focus on it again.
    var episodeFocused by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(state.episodesVersion, state.returnTo) {
        val r = state.returnTo ?: return@LaunchedEffect
        if (r.startsWith("ep:") && eps.any { "ep:${it.id}" == r }) {
            val id = r.removePrefix("ep:")
            runCatching { rowState.scrollToItem(eps.indexOfFirst { it.id == id }.coerceAtLeast(0)) }
            // Until it has really landed (the page may still be settling after the player).
            val landed = currentFocus.focusSoon(40) { episodeFocused == id }
            Log.d("MoonriseFocus") { "title page: focus back on episode $id: $landed" }
            if (landed) state.returnTo = null
        }
    }
    Column {
        Text(if (seasons.size > 1) "Episodes" else (state.selectedSeason?.title ?: "Episodes"), fontSize = 15.sp, fontWeight = FontWeight.SemiBold,
            color = Theme.secondaryText, modifier = Modifier.padding(start = Theme.margin, bottom = 8.dp))
        LazyRow(
            state = rowState,
            modifier = Modifier.enterOn(currentFocus) {
                val target = state.returnTo?.removePrefix("ep:") ?: state.current?.id
                rowState.layoutInfo.visibleItemsInfo.any { it.key == target }
            },
            contentPadding = PaddingValues(horizontal = Theme.margin, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(Theme.rowSpacing),
        ) {
            itemsIndexed(eps, key = { _, e -> e.id }) { i, ep ->
                val isTarget = state.returnTo?.takeIf { it.startsWith("ep:") }?.let { it == "ep:${ep.id}" } ?: (i == currentIndex)
                EpisodeCard(
                    ep, current = ep.id == state.current?.id,
                    onClick = { play(ep, false) },
                    onLongClick = { menu(Menu.Watch(ep, listOf(MenuRow("Play from Beginning") { play(ep, true) }))) },
                    modifier = Modifier.rowEdge(i, eps.lastIndex).then(if (isTarget) Modifier.focusRequester(currentFocus) else Modifier)
                        .onFocusChanged {
                            if (it.isFocused) episodeFocused = ep.id else if (episodeFocused == ep.id) episodeFocused = null
                            if (it.isFocused && state.current?.id != ep.id) { state.current = ep; scope.launch { state.loadFull() } }
                        },
                )
            }
        }
    }
}
