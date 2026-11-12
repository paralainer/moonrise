package app.moonrise.phone.ui

import android.content.res.Configuration
import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.ArrowCircleDown
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.MoreHoriz
import androidx.compose.material.icons.filled.UnfoldMore
import androidx.compose.material.icons.outlined.ArrowCircleDown
import androidx.compose.material.icons.outlined.AutoFixHigh
import androidx.compose.material.icons.outlined.Layers
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
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
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import app.moonrise.core.Item
import app.moonrise.core.LayoutMetrics
import app.moonrise.core.MediaFile
import app.moonrise.core.OfflineTitlePage
import app.moonrise.core.Quality
import app.moonrise.core.Versions
import app.moonrise.core.unplayableProblem
import app.moonrise.phone.LocalNav
import app.moonrise.phone.Route
import app.moonrise.phone.player.InlinePlayer
import app.moonrise.phone.player.PlayerHost
import app.moonrise.shared.AppSettings
import app.moonrise.shared.AutoRetry
import app.moonrise.shared.DetailsState
import app.moonrise.shared.Log
import app.moonrise.shared.Network
import app.moonrise.shared.Session
import app.moonrise.shared.VersionPicks
import app.moonrise.shared.WatchMarks
import app.moonrise.shared.displayHdr
import app.moonrise.shared.downloads.Downloads
import app.moonrise.shared.downloads.ProgressStore
import app.moonrise.shared.downloads.ServerReach
import kotlinx.coroutines.launch

/** A title page's data and its player, kept while other pages are on top of it. */
class TitlePageState(opened: Item) {
    val details = DetailsState(opened, offlinePage = { id -> OfflineTitlePage.build(id, Downloads.records) })
    var overviewExpanded by mutableStateOf(false)
    /** Plays on the page (the iPhone's `InlinePlayback`). */
    val player = PlayerHost(onEpisode = { ep -> follow(ep) })
    var autoplayed = false

    /** Another episode plays (Next Episode, the end of one): the page is about it now. */
    private fun follow(ep: Item) {
        details.returnToEpisode(ep)
        overviewExpanded = false
    }
}

/**
 * A title's page (`ItemDetailView`, iPhone): the backdrop and logo; the title's line; Play (or
 * Resume, filled as far as it was watched) with the version under it; the overview (More) and the
 * ratings; for a show its seasons (a menu) and episodes (a row: a tap selects one, a tap on the
 * selected one plays it); then the cast and crew. A show's page is about one episode, the one to
 * continue with or the one opened.
 *
 * Play plays on the page: the video takes the header's place (the title written out below it), and
 * turning the phone, or its full-screen button, fills the screen with it. Leaving the page stops it.
 */
@Composable
fun TitleScreen(route: Route.Title, signOut: () -> Unit) {
    val nav = LocalNav.current
    val page = route.page
    val state = page.details
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var reloads by remember { mutableIntStateOf(0) }

    LaunchedEffect(reloads, WatchMarks.generation, FixMatchPresenter.rematched) { if (state.loaded) state.refresh() else state.load() }
    LaunchedEffect(Network.generation) { if (state.error != null) reloads++ }
    // Back online: the server's page replaces the downloads' one.
    LaunchedEffect(ServerReach.reachable) { if (ServerReach.reachable && state.offline != null) reloads++ }
    AutoRetry(state.error, busy = { state.loading }) { reloads++ }
    // Back from playing: what was watched shows.
    LaunchedEffect(page.player.stops) { if (page.player.stops > 0) state.refresh() }

    val hdr = remember { displayHdr(context) }
    val language = AppSettings.audioLanguage.ifEmpty { java.util.Locale.getDefault().language }
    val focus = state.focus
    val version = focus?.let { f -> if (f.hasVersions) Versions.chosen(f, VersionPicks.picks[f.id], hdr, language) else null }
    // With no server, where this device got to in the download (the page's item is as it was saved).
    val resume = focus?.let { f -> (if (state.offline != null) ProgressStore.position(f.id)?.takeIf { it > 0 } else null) ?: Versions.resume(f, version) }

    fun play(target: Item, start: Double?, versionId: String?) {
        page.player.play(context, target, start, versionId, hdr)
    }

    // Opened to play (a card's Play, a link): once the page knows what to play.
    val readyToAutoplay = focus?.let { Versions.readyToAutoplay(it, finished = !state.loading && (state.loaded || state.error != null)) } == true
    LaunchedEffect(focus?.id, readyToAutoplay) {
        val a = route.autoplay ?: return@LaunchedEffect
        val f = focus ?: return@LaunchedEffect
        if (!readyToAutoplay || page.autoplayed || !f.isPlayable) return@LaunchedEffect
        page.autoplayed = true
        // A download finds its own place (this device's, else the server's).
        play(f, a.start ?: if (Downloads.playable(f.id) != null) null else resume, version?.id)
    }
    // Leaving the page stops it (reporting where it was), and the screen turns with the phone again.
    DisposableEffect(Unit) {
        onDispose {
            page.player.stop("left the title page")
            app.moonrise.phone.player.Rotation.release(context)
        }
    }
    // The player went on to an episode of another season: its episodes.
    LaunchedEffect(state.selectedSeason?.id) {
        val s = state.selectedSeason ?: return@LaunchedEffect
        if (state.loaded && state.episodes.none { it.seasonId == s.id }) runCatching { state.loadEpisodes(selectFirst = false) }
    }

    val metrics = LocalMetrics.current
    val landscape = LocalConfiguration.current.orientation == Configuration.ORIENTATION_LANDSCAPE
    val playing = page.player.model != null
    val fullScreen = playing && metrics.playerFullScreen(landscape, page.player.fullScreenAsked)
    // No tab bar over the video: a phone full screen, a tablet whenever it plays (its bar is at the top, where the video sits).
    val hideTabs = if (metrics.regular) playing else fullScreen
    LaunchedEffect(hideTabs) { nav.fullScreen = hideTabs }
    // A tablet turned upright while full screen: back to the player in the page, the stream goes on.
    LaunchedEffect(metrics.wide) { if (!metrics.wide) page.player.fullScreenAsked = false }
    DisposableEffect(Unit) { onDispose { nav.fullScreen = false } }

    val e = state.error
    val show = state.show
    if (e != null && !state.loaded && show == null) {
        PageBackground { ErrorPane(e, onRetry = { reloads++ }, signIn = if (state.unauthorized) signOut else null, modifier = Modifier.align(Alignment.Center)) }
        BackButton()
        return
    }
    if (show == null) {
        PageBackground { CircularProgressIndicator(color = Color.White, modifier = Modifier.align(Alignment.Center)) }
        BackButton()
        return
    }

    // The full-screen button on a tablet: no rotation; a wide one has no small player to go back to.
    val toggleFullScreen: (() -> Unit)? = if (metrics.regular) ({
        if (fullScreen && metrics.stopsOnFullScreenExit) page.player.stop("left full screen")
        else page.player.fullScreenAsked = !page.player.fullScreenAsked
    }) else null

    if (fullScreen) {
        // Turned on its side, or a wide tablet: the video fills the screen.
        InlinePlayer(page.player, fullScreen = true, onBack = { nav.back() }, toggleFullScreen = toggleFullScreen)
        return
    }

    Box(Modifier.fillMaxSize().background(Theme.background)) {
        if (playing) {
            Column(Modifier.fillMaxSize()) {
                Box(Modifier.fillMaxWidth().background(Color.Black).statusBarsPadding())
                val band = if (metrics.regular) Modifier.fillMaxWidth().height(metrics.playerBand(metrics.width, metrics.height).dp) else Modifier.fillMaxWidth().aspectRatio(16f / 9f)
                InlinePlayer(page.player, fullScreen = false, onBack = { nav.back() }, modifier = band, toggleFullScreen = toggleFullScreen)
                Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                    Spacer(Modifier.height(18.dp))
                    PageBody(page, state, show, header = false, version = version, resume = resume, play = ::play)
                }
            }
        } else {
            Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                val headerHeight = if (metrics.regular) metrics.headerHeight(0.0).dp else if (landscape) (LocalConfiguration.current.screenHeightDp * 0.38f).dp else Theme.headerHeight
                val image = headerImage(state, show)
                TitleHeader(image, if (state.offline != null) null else imageUrl(show.logoImage, if (metrics.regular) 380.dp else 300.dp), show.title.orEmpty(), headerHeight)
                Spacer(Modifier.height(18.dp))
                PageBody(page, state, show, header = true, version = version, resume = resume, play = ::play, hasImage = image != null)
            }
            BackButton()
        }
    }
}

/** Backdrop, else (shows) the episode's still, else the poster. */
@Composable
private fun headerImage(state: DetailsState, show: Item): String? {
    val width = if (LocalMetrics.current.regular) LocalMetrics.current.width.dp else 600.dp
    return if (state.offline != null) offlineArtwork(state, show) else
        imageUrl(show.images?.backdrop?.firstOrNull(), width)
            ?: (if (state.isShow) imageUrl(state.current?.images?.thumb, width) else null)
            ?: imageUrl(show.posterImage, width)
}

/** The page's picture from the downloads: the selected episode's still, else the title's own. */
private fun offlineArtwork(state: DetailsState, show: Item): String? {
    if (state.isShow) {
        state.current?.let { e -> Downloads.forItem(e.id)?.pageArtwork }?.let { return it }
        return Downloads.episodes(show.id).firstNotNullOfOrNull { it.pageArtwork }
    }
    return Downloads.forItem(show.id)?.pageArtwork
}

@Composable
private fun BackButton() {
    val nav = LocalNav.current
    GlassCircle(Icons.AutoMirrored.Filled.ArrowBack, "Back", Modifier.statusBarsPadding().padding(start = Theme.backStart, top = 8.dp)) { nav.back() }
}

/** Everything under the header (or the playing video). */
@Composable
private fun PageBody(page: TitlePageState, state: DetailsState, show: Item, header: Boolean, version: MediaFile?, resume: Double?, play: (Item, Double?, String?) -> Unit, hasImage: Boolean = true) {
    val focus = state.focus
    val metrics = LocalMetrics.current
    val regular = metrics.regular
    Column(verticalArrangement = Arrangement.spacedBy(18.dp), modifier = Modifier.padding(bottom = Theme.pageBottom)) {
        // Title, line, Play and overview: centred; a tablet, at the page's leading margin, as wide as text reads well
        // (wider than tall, over the bottom of the picture).
        Column((if (regular) Modifier.padding(horizontal = Theme.margin).widthIn(max = LayoutMetrics.TEXT_COLUMN.dp).fillMaxWidth().then(if (header) Modifier.pullUp(metrics.headerOverlap(hasImage).dp) else Modifier)
            else Modifier.fillMaxWidth().padding(horizontal = Theme.margin).widthIn(max = 700.dp)), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Column {
                // A tablet playing it: Download at the end of the title (no Play row).
                if (regular && focus != null && focus.isPlayable && !header && page.player.playing(focus.id) != null) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Box(Modifier.weight(1f)) { TitleBlock(state, show, named = true, fixMatch = state.offline == null && AdminState.isAdmin) }
                        if (state.offline == null || Downloads.forItem(focus.id) != null) DownloadButton(focus, Modifier.clip(CircleShape).background(Theme.glass).border(0.5.dp, Color.White.copy(alpha = 0.18f), CircleShape))
                    }
                } else TitleBlock(state, show, named = !header, fixMatch = state.offline == null && AdminState.isAdmin)
                val unplayable = focus?.takeIf { it.isPlayable }?.unplayableProblem()
                // No file can play (damaged, or gone from disk): no Play or Download, what's wrong instead.
                if (unplayable != null) DamagedTitleNote(unplayable, regular)
                else if (focus != null && focus.isPlayable && page.player.playing(focus.id) == null) {
                    Column(Modifier.padding(top = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        // Play, and Download beside it (as tall, 12 apart).
                        Row(if (regular) Modifier else Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                            PlayCapsule(if (resume == null) "Play" else "Resume", resume, resume?.let { r ->
                                val total = version?.duration ?: focus.runtime ?: return@let null
                                if (total > 0) r / total else null
                            }, (if (regular) Modifier.width(LayoutMetrics.PLAY_WIDTH.dp) else Modifier.weight(1f)).clickable { play(focus, if (version != null) resume ?: 0.0 else null, version?.id) })
                            if (state.offline == null || Downloads.forItem(focus.id) != null) DownloadButton(focus, Modifier.clip(CircleShape).background(Theme.glass).border(0.5.dp, Color.White.copy(alpha = 0.18f), CircleShape))
                        }
                        if (state.offline == null) VersionLine(focus, version)
                        DownloadStatusLine(focus)
                    }
                }
            }
            Overview(page, (focus?.overview ?: show.overview))
            RatingsRow(show, focus, state.isShow, leading = regular)
        }
        if (state.isShow) Episodes(page, state, play)
        val people = show.people.orEmpty().filter { it.group in setOf("Actor", "Director", "Writer") }.take(24)
        if (people.isNotEmpty()) Shelf("Cast and Crew") { items(people, key = { it.id + it.group + (it.role ?: "") }) { CastCard(it) } }
        if (regular) TitleDetails(show, focus, state.seasons.size)
        state.error?.takeIf { state.loaded }?.let { Text(it, style = Type.footnote, color = Theme.destructive, modifier = Modifier.padding(horizontal = Theme.margin)) }
    }
}

/** The episode's line ("S1 • E3 - Title") and the title's line; the title itself when the header is the video. */
@Composable
private fun TitleBlock(state: DetailsState, show: Item, named: Boolean, fixMatch: Boolean) {
    val menu = fixMatchMenuOpen()
    val lead = LocalMetrics.current.regular
    val textAlign = if (lead) TextAlign.Start else TextAlign.Center
    val align = if (lead) Alignment.Start else Alignment.CenterHorizontally
    Box(Modifier.fillMaxWidth(), contentAlignment = if (lead) Alignment.TopStart else Alignment.TopCenter) {
    Column(Modifier.fillMaxWidth().then(if (fixMatch && FixMatchPresenter.applies(show)) Modifier.combinedClickable(interactionSource = null, indication = null, onClick = {}, onLongClick = { menu.value = true }) else Modifier), horizontalAlignment = align) {
        if (named) Text(show.title.orEmpty(), style = if (state.isShow) Type.headline else Type.title2.copy(fontWeight = FontWeight.Bold),
            color = if (state.isShow) Theme.secondaryText else Color.White, textAlign = textAlign, modifier = Modifier.padding(bottom = 6.dp))
        Column(horizontalAlignment = align, verticalArrangement = Arrangement.spacedBy(6.dp)) {
            val e = state.current
            if (state.isShow && e != null) {
                Text(listOfNotNull(e.episodeLabel?.replace(" · ", " • "), e.title).joinToString(" - "), style = Type.title3, fontWeight = FontWeight.SemiBold,
                    color = Color.White, textAlign = textAlign)
            }
            TitleMeta(show, state.focus, leading = lead)
        }
    }
    FixMatchMenu(show, menu)
    }
}

/** "Version: 1080p · Director's Cut", a menu of the title's versions (several files only). */
@Composable
private fun VersionLine(item: Item, chosen: MediaFile?) {
    if (!item.hasVersions || chosen == null) return
    val scope = rememberCoroutineScope()
    var open by remember { mutableStateOf(false) }
    Box {
        Row(Modifier.clickable { open = true }, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Icon(Icons.Outlined.Layers, null, tint = Theme.secondaryText, modifier = Modifier.size(15.dp))
            Text(buildAnnotatedString {
                withStyle(SpanStyle(color = Theme.secondaryText)) { append("Version: ") }
                withStyle(SpanStyle(color = Color.White)) { append(chosen.versionTitle) }
            }, style = Type.footnote, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Icon(Icons.Filled.UnfoldMore, null, tint = Theme.secondaryText, modifier = Modifier.size(14.dp))
        }
        DropdownMenu(open, { open = false }, containerColor = Theme.sheet) {
            Text("Version", style = Type.footnote, color = Theme.secondaryText, modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp))
            for (v in item.presentFiles) {
                MenuItem(v.versionTitle, checked = v.id == chosen.id, detail = v.versionDetails, enabled = !v.isBroken) {
                    open = false
                    VersionPicks.picks[item.id] = v.id
                    scope.launch { runCatching { Session.api?.chooseVersion(item.id, v.id) }.onFailure { Log.w("Moonrise", "version pick failed: ${it.message}") } }
                }
            }
            // Admins also get Fix Match for the versions (split one off, match one alone).
            if (AdminState.isAdmin) {
                HorizontalDivider(color = Color.White.copy(alpha = 0.12f))
                MenuItem("Fix Match Versions…", Icons.Outlined.AutoFixHigh) { open = false; FixMatchPresenter.present(item.id) }
            }
        }
    }
}

/** Three lines, More shows all of it. */
@Composable
private fun Overview(page: TitlePageState, text: String?) {
    if (text.isNullOrBlank()) return
    Column(Modifier.fillMaxWidth().animateContentSize(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(text, style = Type.body, color = Color.White.copy(alpha = 0.9f), maxLines = if (page.overviewExpanded) Int.MAX_VALUE else 3, overflow = TextOverflow.Ellipsis)
        if (!page.overviewExpanded && text.length > 140) {
            Text("More", style = Type.footnote, fontWeight = FontWeight.SemiBold, color = Theme.secondaryText, modifier = Modifier.clickable { page.overviewExpanded = true })
        }
    }
}

/** The season menu and the season's episodes, the selected one centred. */
@Composable
private fun Episodes(page: TitlePageState, state: DetailsState, play: (Item, Double?, String?) -> Unit) {
    val scope = rememberCoroutineScope()
    var open by remember { mutableStateOf(false) }
    val metrics = LocalMetrics.current
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (metrics.regular) SeasonChips(page, state) else
        Box(Modifier.padding(horizontal = Theme.margin)) {
            Row(Modifier.clickable { open = true }, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(state.selectedSeason?.title ?: "Episodes", style = Type.title3, fontWeight = FontWeight.SemiBold, color = Color.White)
                Icon(Icons.Filled.KeyboardArrowDown, null, tint = Color.White, modifier = Modifier.size(20.dp))
                state.selectedSeason?.let { s -> Downloads.seasonCounts[s.id]?.takeIf { it > 0 } }?.let { n ->
                    Row(Modifier.padding(start = 6.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(3.dp)) {
                        Icon(Icons.Filled.ArrowCircleDown, null, tint = Theme.secondaryText, modifier = Modifier.size(14.dp))
                        Text("$n", style = Type.footnote, fontWeight = FontWeight.SemiBold, color = Theme.secondaryText)
                    }
                }
            }
            DropdownMenu(open, { open = false }, containerColor = Theme.sheet) {
                for (s in state.seasons) {
                    MenuItem(s.title ?: "Season ${s.seasonNo ?: ""}", checked = s.id == state.selectedSeason?.id,
                        detail = Downloads.seasonCounts[s.id]?.takeIf { it > 0 }?.let { "$it downloaded" }) {
                        open = false
                        page.overviewExpanded = false
                        scope.launch { state.selectSeason(s) }
                    }
                }
                val season = state.selectedSeason
                if (state.offline == null && season != null && state.episodes.isNotEmpty()) {
                    val missing = state.episodes.filter { it.isPlayable && Downloads.forItem(it.id) == null }
                    HorizontalDivider(color = Color.White.copy(alpha = 0.12f))
                    MenuItem(if (missing.isEmpty()) "${season.title} Downloaded" else "Download ${season.title} (${missing.size})", Icons.Outlined.ArrowCircleDown, enabled = missing.isNotEmpty()) {
                        open = false
                        scope.launch { val q = Quality.of(AppSettings.downloadQuality); for (ep in missing) Downloads.download(ep, q, ep.presentFiles.firstOrNull()) }
                    }
                }
            }
        }
        val list = rememberLazyListState()
        val width = metrics.landscapeWidth.dp
        val screen = LocalConfiguration.current.screenWidthDp.dp
        val density = LocalDensity.current
        // The selected episode in the middle of the row.
        LaunchedEffect(state.episodesVersion, state.current?.id) {
            val i = state.episodes.indexOfFirst { it.id == state.current?.id }
            if (i == 0) runCatching { list.animateScrollToItem(0) }
            else if (i > 0) {
                val offset = with(density) { metrics.episodeOffset(screen.value.toDouble(), width.value.toDouble()).dp.roundToPx() }
                runCatching { list.animateScrollToItem(i, -offset) }
            }
        }
        LazyRow(state = list, contentPadding = PaddingValues(horizontal = Theme.margin, vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(14.dp)) {
            items(state.episodes, key = { it.id }) { ep ->
                val nowPlaying = page.player.item?.id == ep.id
                EpisodeCard(ep, width, selected = ep.id == state.current?.id, nowPlaying = nowPlaying, onClick = {
                    when {
                        nowPlaying -> state.current = ep
                        // Playing: another episode switches the video to it.
                        ep.id == state.current?.id || page.player.model != null -> play(ep, null, null)
                        else -> { state.current = ep; page.overviewExpanded = false; scope.launch { state.loadFull() } }
                    }
                }, menu = if (state.offline == null) CardMenu(marked = { scope.launch { state.refresh() } }) else null,
                    image = if (state.offline != null) Downloads.forItem(ep.id)?.pageArtwork else null, details = metrics.regular)
            }
        }
    }
}

/** A tablet's seasons: chips in a row, the season's download in a menu at the end (`seasonChips`). */
@Composable
private fun SeasonChips(page: TitlePageState, state: DetailsState) {
    val scope = rememberCoroutineScope()
    var menu by remember { mutableStateOf(false) }
    LazyRow(contentPadding = PaddingValues(horizontal = Theme.margin), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
        items(state.seasons, key = { it.id }) { s ->
            val on = s.id == state.selectedSeason?.id
            val n = Downloads.seasonCounts[s.id] ?: 0
            Row(Modifier.height(36.dp).clip(CircleShape).background(if (on) Color.White else Theme.surfaceStrong).clickable(enabled = !on) {
                page.overviewExpanded = false
                scope.launch { state.selectSeason(s) }
            }.padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(s.title ?: "Season ${s.seasonNo ?: ""}", style = Type.subheadline, fontWeight = FontWeight.SemiBold, color = if (on) Color.Black else Color.White)
                if (n > 0) Icon(Icons.Filled.ArrowCircleDown, null, tint = if (on) Color.Black else Color.White, modifier = Modifier.size(14.dp))
            }
        }
        val season = state.selectedSeason
        if (state.offline == null && season != null && state.episodes.isNotEmpty()) item("options") {
            val missing = state.episodes.filter { it.isPlayable && Downloads.forItem(it.id) == null }
            Box {
                Box(Modifier.size(36.dp).clip(CircleShape).background(Theme.surfaceStrong).clickable { menu = true }, contentAlignment = Alignment.Center) {
                    Icon(Icons.Filled.MoreHoriz, "Season options", tint = Color.White, modifier = Modifier.size(20.dp))
                }
                DropdownMenu(menu, { menu = false }, containerColor = Theme.sheet) {
                    MenuItem(if (missing.isEmpty()) "${season.title} Downloaded" else "Download ${season.title} (${missing.size})", Icons.Outlined.ArrowCircleDown, enabled = missing.isNotEmpty()) {
                        menu = false
                        scope.launch { val q = Quality.of(AppSettings.downloadQuality); for (ep in missing) Downloads.download(ep, q, ep.presentFiles.firstOrNull()) }
                    }
                }
            }
        }
    }
}
