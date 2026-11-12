package app.moonrise.tv.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
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
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.focus.FocusDirection
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.tv.material3.Surface
import androidx.tv.material3.Text
import app.moonrise.core.Item
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import app.moonrise.shared.Log

/** The tabs as on Apple TV: Home, Library, Search, Settings. */
enum class Tab(val title: String) { Home("Home"), Library("Library"), Search("Search"), Settings("Settings") }

/** The tabs' content, kept while a title page or the player is on top (Back lands where you left). */
class MainState {
    var tab by mutableStateOf(Tab.Home)
    val home = HomeState()
    val library = LibraryState()
    val search = SearchState()
    /** The open tab's content is scrolled to its top: the tab bar shows over it. */
    var atTop by mutableStateOf(true)
    /** The open tab's first control: Down from the tab bar lands there when the focus search finds nothing. */
    val entry = FocusRequester()
    /**
     * Brings the open tab's first control on screen and focuses it (set by the tab while it's
     * shown): Down from the tab bar uses it when the focus search finds nothing below.
     */
    var enterContent: (suspend () -> Boolean)? = null
    /** Bumped to move focus to the tab bar. */
    var focusTabs by mutableIntStateOf(0)
    /** The tab bar has focus: a tab's own Back handling leaves Back to the bar then. */
    var tabsFocused by mutableStateOf(false)
    /**
     * The open tab was switched to on the tab bar: it doesn't take focus from the bar to put it
     * back where it was (until a title is opened from it).
     */
    var tabSwitched = false
    /** A title page was opened from the tabs: coming back, focus goes back where it was. */
    var returning = false

    fun clear() {
        tab = Tab.Home
        tabSwitched = false
        home.clear()
        library.clear()
        search.clear()
    }

    /** After playback or a mark: watch state may have changed. */
    fun refreshSoon() {
        home.refreshSoon()
        library.refreshSoon()
        search.refreshSoon()
    }
}

/** The tab bar (a glass capsule at the top that scrolls away with the content) and the open tab. */
@Composable
fun MainScreen(state: MainState, open: (Item) -> Unit, signOut: () -> Unit, changeServer: () -> Unit) {
    val requesters = remember { Tab.entries.associateWith { FocusRequester() } }
    val scope = rememberCoroutineScope()
    var contentFocused by remember { mutableStateOf(false) }
    fun focusTab(t: Tab = state.tab) = runCatching { requesters.getValue(t).requestFocus() }

    // Back in a tab's content goes up to the tab bar; on the tab bar, back to Home; on Home's tab,
    // out of the app (as the Apple TV's Menu button). A tab with levels of its own (the Library's
    // grid, then its sidebar) handles Back first while its content has focus.
    BackHandler(enabled = !state.tabsFocused || state.tab != Tab.Home) {
        if (!state.tabsFocused) {
            if (state.tab == Tab.Home) scope.launch { runCatching { state.home.column.scrollToItem(0) } }
            focusTab()
        } else {
            state.tab = Tab.Home
            state.atTop = true
            state.tabSwitched = true
            focusTab(Tab.Home)
        }
    }
    LaunchedEffect(state.focusTabs) { if (state.focusTabs > 0) focusTab() }
    // (The bar goes away under a title page: it has no focus when the tabs come back.)
    androidx.compose.runtime.DisposableEffect(Unit) { onDispose { state.tabsFocused = false } }
    // Back from a title page or the player: the tab puts focus back on what was opened. If that
    // couldn't happen (the card is gone), focus goes to the tab bar, never nowhere.
    LaunchedEffect(Unit) {
        if (!state.returning) return@LaunchedEffect
        delay(1200)
        state.returning = false
        if (!contentFocused && !state.tabsFocused) {
            Log.i("Moonrise") { "nothing focused: the tab bar" }
            focusTab()
        }
    }

    // Back in front (the TV woke, or the viewer came back from another app): Home loads again, so
    // what another device played meanwhile shows up; a screen that failed loads again.
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { state.home.refreshIfStale() }

    val focusManager = LocalFocusManager.current
    val openTitle: (Item) -> Unit = { state.returning = true; state.tabSwitched = false; open(it) }
    Box(Modifier.fillMaxSize()) {
        // Up from the top of a tab's content goes to the tab bar (it lies over the content, so
        // the focus search alone doesn't find it above the banner).
        Box(Modifier.fillMaxSize().onFocusChanged { contentFocused = it.hasFocus }.onPreviewKeyEvent { e ->
            if (e.type != KeyEventType.KeyDown || e.key != Key.DirectionUp || state.tabsFocused || !state.atTop) return@onPreviewKeyEvent false
            if (!focusManager.moveFocus(FocusDirection.Up)) focusTab()
            true
        }) {
        when (state.tab) {
            Tab.Home -> HomeScreen(state.home, open = openTitle, signOut = signOut, atTop = { state.atTop = it }, main = state)
            Tab.Library -> LibraryScreen(state.library, open = openTitle, atTop = { state.atTop = it }, main = state)
            Tab.Search -> SearchScreen(state.search, open = openTitle, atTop = { state.atTop = it }, entry = state.entry, main = state)
            Tab.Settings -> SettingsScreen(signOut = signOut, changeServer = changeServer, atTop = { state.atTop = it }, entry = state.entry, main = state)
        }
        }
        val shown by animateFloatAsState(if (state.tabsFocused || state.atTop) 1f else 0f, label = "tabs")
        Row(
            Modifier
                .align(Alignment.TopCenter)
                .padding(top = 14.dp)
                .alpha(shown)
                .background(Color.White.copy(alpha = 0.10f), RoundedCornerShape(50))
                .padding(4.dp)
                .onFocusChanged { state.tabsFocused = it.hasFocus }
                .onPreviewKeyEvent { e ->
                    if (e.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                    when (e.key) {
                        // Moving along the bar opens the tab, as on Apple TV (focus landing on a tab
                        // from elsewhere doesn't switch tabs).
                        Key.DirectionLeft, Key.DirectionRight -> {
                            val i = state.tab.ordinal + if (e.key == Key.DirectionLeft) -1 else 1
                            Tab.entries.getOrNull(i)?.let { t ->
                                state.tab = t
                                state.atTop = true
                                state.tabSwitched = true
                                focusTab(t)
                            }
                            true
                        }
                        // Down goes into the tab's content: what the focus search finds below, else
                        // the tab's first control, scrolled back on screen (never stuck on the bar).
                        Key.DirectionDown -> {
                            if (!focusManager.moveFocus(FocusDirection.Down)) scope.launch {
                                val entered = state.enterContent?.invoke() ?: state.entry.focusSoon(10)
                                Log.i("Moonrise") { "down from the tab bar: ${if (entered) "content" else "nothing to focus"}" }
                            }
                            true
                        }
                        else -> false
                    }
                },
            horizontalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Tab.entries.forEach { tab ->
                val selected = tab == state.tab
                FocusSurface(
                    onClick = { state.tab = tab; state.tabSwitched = true },
                    modifier = Modifier.focusRequester(requesters.getValue(tab)).logFocus { "tab ${tab.title}" },
                    shape = RoundedCornerShape(50),
                    focusedScale = 1.05f,
                    containerColor = if (selected) Color.White.copy(alpha = 0.18f) else Color.Transparent,
                        contentColor = Color.White,
                        focusedContainerColor = Color.White,
                        focusedContentColor = Color.Black,
                ) {
                    Text(
                        tab.title, fontSize = 13.sp, fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Medium,
                        modifier = Modifier.padding(horizontal = 14.dp, vertical = 6.dp),
                    )
                }
            }
        }
    }
}

/** Room at the top of a tab's content for the tab bar. */
val TAB_BAR_SPACE = 58.dp

/**
 * While a tab is shown, `enter` is how Down from the tab bar gets into it when the focus search
 * finds nothing below the bar (its first control scrolled away, say).
 */
@Composable
fun EnterContent(main: MainState?, enter: suspend () -> Boolean) {
    if (main == null) return
    val current = androidx.compose.runtime.rememberUpdatedState(enter)
    androidx.compose.runtime.DisposableEffect(main) {
        val e: suspend () -> Boolean = { current.value() }
        main.enterContent = e
        onDispose { if (main.enterContent === e) main.enterContent = null }
    }
}
