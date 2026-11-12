package app.moonrise.phone.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Cancel
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import app.moonrise.core.ClientRules
import app.moonrise.core.Item
import app.moonrise.core.SearchStart
import app.moonrise.phone.LocalNav
import app.moonrise.phone.Route
import app.moonrise.shared.Session
import app.moonrise.shared.WatchMarks
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** What Search found, kept while a title page is on top (Back lands on the same results). */
object SearchStore {
    var query by mutableStateOf("")
    var results by mutableStateOf<List<Item>>(emptyList())
    var searched by mutableStateOf("")
    var searching by mutableStateOf(false)
    /** Recent and Suggestions, shown while nothing is typed (null: an older server, or not yet known). */
    var start by mutableStateOf<SearchStart?>(null)
    var startLoaded by mutableStateOf(false)

    fun clear() { query = ""; results = emptyList(); searched = ""; start = null; startLoaded = false }

    /** A title was opened from Search: first in Recent at once, and on the server for every device. */
    fun opened(item: Item) {
        if (start == null) return
        start = start?.opened(item)
        Session.ui.launch { runCatching { Session.api?.recordSearchOpened(item.id) } }
    }

    fun clearRecent() {
        start = start?.recentCleared()
        Session.ui.launch { runCatching { Session.api?.clearSearchRecent() } }
    }
}

/**
 * Search (`SearchView`, iPhone): the field, then the results as you type: Shows and Movies as
 * poster grids, Episodes as a row (shows and episodes can share a title, so the kinds stay apart).
 */
@Composable
fun SearchScreen() {
    val nav = LocalNav.current
    val s = SearchStore
    LaunchedEffect(s.query, WatchMarks.generation) {
        val q = s.query.trim()
        if (q.length < 2) { s.results = emptyList(); s.searched = q; return@LaunchedEffect }
        delay(350)
        s.searching = true
        try {
            val r = Session.api?.search(q)
            if (r != null) s.results = r.titles + r.episodes
            s.searched = q
        } catch (e: Exception) {
            if (!ClientRules.isLoadFailure(e)) throw e
            s.results = emptyList()
        } finally {
            s.searching = false
        }
    }
    // Each time Search shows, and after a title's watched state changed: what to offer before typing.
    LaunchedEffect(Session.api, WatchMarks.generation) {
        val api = Session.api ?: return@LaunchedEffect
        try {
            s.start = api.searchStart()
        } catch (e: Exception) {
            if (!ClientRules.isLoadFailure(e)) throw e
        } finally {
            s.startLoaded = true
        }
    }
    val open = { item: Item -> s.opened(item); nav.push(Route.Title(item)) }
    PageBackground {
        val regular = LocalMetrics.current.regular
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
            if (regular) {
                // A tablet: the title, and a full-width field under it (`.navigationBarDrawer`).
                Text("Search", style = Type.largeTitle, color = Color.White, modifier = Modifier.statusBarsPadding().padding(start = Theme.margin, top = Theme.underTopBar + 4.dp))
                PadSearchField(Modifier.padding(horizontal = Theme.margin).padding(top = 10.dp, bottom = 14.dp))
            } else if (s.query.isEmpty()) {
                // The title goes once something is typed (as the iPhone's search page).
                Text("Search", style = Type.largeTitle, color = Color.White, modifier = Modifier.statusBarsPadding().padding(start = Theme.margin, top = 52.dp))
            } else Spacer(Modifier.statusBarsPadding().height(16.dp))
            Column(Modifier.padding(vertical = 16.dp), verticalArrangement = Arrangement.spacedBy(26.dp)) {
                val start = s.start
                if (SearchStart.shows(s.query) && start != null) StartRows(start, open)
                // A tablet on a server without Recent and Suggestions: what's new, as on Home.
                if (regular && s.query.isEmpty() && s.startLoaded && start == null) {
                    for (section in HomeStore.sections.filter { it.id != HomeStore.CONTINUE }) {
                        Shelf(section.title) {
                            items(section.items, key = { it.id }) { item ->
                                if (section.landscape) LandscapeCard(item, onClick = { nav.push(Route.Title(item)) }, menu = CardMenu())
                                else PosterCard(item, onClick = { nav.push(Route.Title(item)) }, menu = CardMenu())
                            }
                        }
                    }
                }
                Group("Shows", s.results.filter { it.isSeries }, open)
                Group("Movies", s.results.filter { it.isMovie }, open)
                val eps = s.results.filter { it.isEpisode }
                if (eps.isNotEmpty()) Shelf("Episodes") { items(eps, key = { it.id }) { e -> LandscapeCard(e, onClick = { open(e) }, menu = CardMenu()) } }
                if (s.searching) Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { CircularProgressIndicator(color = Color.White) }
                if (!s.searching && s.results.isEmpty() && s.query.isNotBlank() && (s.query.trim().length < 2 || s.searched == s.query.trim())) {
                    EmptyState(Icons.Filled.Search, "No Results", "Nothing matched “${s.query}”.")
                }
            }
            Spacer(Modifier.height(Theme.pageBottom))
        }
    }
}

/**
 * Before anything is typed: Recent as a row with Clear, Suggestions as the results' grid, and a
 * message when there is neither.
 */
@Composable
private fun StartRows(start: SearchStart, open: (Item) -> Unit) {
    if (start.isEmpty) {
        EmptyState(Icons.Filled.Search, "Search Your Library", "Find movies, shows and episodes by name.")
        return
    }
    if (start.recent.isNotEmpty()) {
        Shelf("Recent", action = {
            Text("Clear", style = Type.subheadline, color = Theme.secondaryText, modifier = Modifier.clickable { SearchStore.clearRecent() })
        }) {
            items(start.recent, key = { it.id }) { item -> PosterCard(item, onClick = { open(item) }, menu = CardMenu()) }
        }
    }
    Group("Suggestions", start.suggestions, open)
}

/** A titled grid of posters at the grid's smallest size, centred (as the iPhone's adaptive grid). */
@Composable
private fun Group(title: String, items: List<Item>, open: (Item) -> Unit) {
    if (items.isEmpty()) return
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        SectionHeader(title)
        val metrics = LocalMetrics.current
        // A tablet: the Library's grid (its poster size, gaps and margins).
        if (metrics.regular) BoxWithConstraints(Modifier.fillMaxWidth().padding(horizontal = Theme.margin)) {
            val (n, card) = metrics.grid(maxWidth.value.toDouble())
            Column(verticalArrangement = Arrangement.spacedBy(20.dp)) {
                for (row in items.chunked(n)) Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    for (item in row) PosterCard(item, width = card.dp, onClick = { open(item) }, menu = CardMenu())
                }
            }
        } else
        // SwiftUI's adaptive grid: as many cells of at least 108 as fit, 8 apart, each poster
        // 108 wide in the middle of its cell.
        BoxWithConstraints(Modifier.fillMaxWidth().padding(horizontal = Theme.margin)) {
            val min = 108.dp
            val n = maxOf(1, ((maxWidth + 8.dp) / (min + 8.dp)).toInt())
            val cell = (maxWidth - 8.dp * (n - 1)) / n
            Column(verticalArrangement = Arrangement.spacedBy(18.dp)) {
                for (row in items.chunked(n)) Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    for (item in row) Box(Modifier.width(cell), contentAlignment = Alignment.TopCenter) { PosterCard(item, width = min, onClick = { open(item) }, menu = CardMenu()) }
                }
            }
        }
    }
}

/** The tablet's search field, in the page under the title. */
@Composable
private fun PadSearchField(modifier: Modifier = Modifier) {
    val s = SearchStore
    val focus = LocalFocusManager.current
    Row(modifier.fillMaxWidth().height(44.dp).clip(CircleShape).background(Theme.surfaceStrong).padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Icon(Icons.Filled.Search, null, tint = Theme.secondaryText, modifier = Modifier.size(20.dp))
        Box(Modifier.weight(1f)) {
            if (s.query.isEmpty()) Text("Movies, shows, episodes", style = Type.body, color = Theme.secondaryText, maxLines = 1)
            BasicTextField(s.query, { s.query = it }, singleLine = true, textStyle = Type.body.copy(color = Color.White), cursorBrush = SolidColor(Color.White),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search), keyboardActions = KeyboardActions(onSearch = { focus.clearFocus() }), modifier = Modifier.fillMaxWidth())
        }
        if (s.query.isNotEmpty()) Icon(Icons.Filled.Cancel, "Clear", tint = Theme.secondaryText, modifier = Modifier.size(20.dp).clip(CircleShape).clickable { s.query = "" })
    }
}

/**
 * The search field in the tab bar's place at the bottom (iOS 26's search tab): Home in a circle at
 * the start, the field, and while typing a circle that ends the search.
 */
@Composable
fun SearchBar(home: () -> Unit) {
    val s = SearchStore
    val focus = LocalFocusManager.current
    var focused by remember { mutableStateOf(false) }
    Row(Modifier.fillMaxWidth().navigationBarsPadding().imePadding().padding(start = 21.dp, end = 21.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Box(Modifier.size(48.dp).clip(CircleShape).background(Theme.bar).border(0.5.dp, Color.White.copy(alpha = 0.16f), CircleShape).clickable(onClick = home),
            contentAlignment = Alignment.Center) { Icon(Icons.Filled.Home, "Home", tint = Color.White, modifier = Modifier.size(26.dp)) }
        Row(Modifier.weight(1f).height(48.dp).clip(CircleShape).background(Theme.bar).border(0.5.dp, Color.White.copy(alpha = 0.16f), CircleShape).padding(horizontal = 14.dp),
            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Icon(Icons.Filled.Search, null, tint = Color.White, modifier = Modifier.size(22.dp))
            Box(Modifier.weight(1f)) {
                if (s.query.isEmpty()) Text("Movies, shows, episodes", style = Type.body, color = Theme.secondaryText, maxLines = 1)
                BasicTextField(s.query, { s.query = it }, singleLine = true, textStyle = Type.body.copy(color = Color.White), cursorBrush = SolidColor(Color.White),
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search), keyboardActions = KeyboardActions(onSearch = { focus.clearFocus() }),
                    modifier = Modifier.fillMaxWidth().onFocusChanged { focused = it.isFocused })
            }
            if (s.query.isNotEmpty()) Icon(Icons.Filled.Cancel, "Clear", tint = Theme.secondaryText, modifier = Modifier.size(20.dp).clip(CircleShape).clickable { s.query = "" })
        }
        if (focused) Box(Modifier.size(48.dp).clip(CircleShape).background(Theme.bar).border(0.5.dp, Color.White.copy(alpha = 0.16f), CircleShape)
            .clickable { s.query = ""; focus.clearFocus() }, contentAlignment = Alignment.Center) { Icon(Icons.Filled.Close, "Cancel", tint = Color.White, modifier = Modifier.size(24.dp)) }
    }
}
