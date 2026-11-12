package app.moonrise.tv.ui

import android.view.KeyEvent
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.focus.FocusRequester
import app.moonrise.core.SegmentKind
import app.moonrise.shared.player.PlayerModel
import kotlinx.coroutines.delay
import app.moonrise.shared.Log
import app.moonrise.shared.AppSettings

internal const val FOCUS = "MoonriseFocus"

/** Where the remote is in the player (the controls are drawn by the app; Media3's view only shows the picture). */
internal enum class Zone { None, Scrubber, Buttons, Corner, Panel }

/** The menu buttons above the scrubber, left to right. */
internal val menuButtons = listOf("Audio", "Subtitles", "Quality")

/** The player's remote and what it has open: the controls, the corner, the panel and the menus. */
@Composable
internal fun rememberPlayerControls(model: PlayerModel, onExit: () -> Unit): PlayerControls {
    val exit by rememberUpdatedState(onExit)
    return remember { PlayerControls(model) { exit() } }
}

/**
 * Where the remote is and what each press does (see [PlayerScreen] for the remote's rules). The
 * state the screen draws from; [PlayerControlsEffects] runs its timers.
 */
@Stable
internal class PlayerControls(private val model: PlayerModel, private val onExit: () -> Unit) {
    var zone by mutableStateOf(Zone.None)
        private set
    var buttonIndex by mutableIntStateOf(1)
        private set
    var menu by mutableStateOf<PlayerMenu?>(null)
        private set
    var panelTab by mutableStateOf(PanelTab.Info)
        private set
    var panelOnContent by mutableStateOf(false)
        private set
    var episodeIndex by mutableIntStateOf(0)
        private set
    var lastKey by mutableStateOf(System.currentTimeMillis())
        private set
    /** Next Episode was let go of: hidden until the controls show again. */
    private var nextLetGo by mutableStateOf(false)
    /** The corner took the remote while the controls showed (they stay up behind it). */
    var cornerOverControls by mutableStateOf(false)
        private set
    /** Playing when Search Online opened: plays again when it closes. */
    private var resumeAfterSearch by mutableStateOf(false)
    var cornerIndex by mutableIntStateOf(0)
        private set
    /** Takes the remote's presses whenever no menu is open. */
    val keys = FocusRequester()

    val controlsVisible: Boolean get() = zone == Zone.Scrubber || zone == Zone.Buttons || (zone == Zone.Corner && cornerOverControls)

    // The corner's buttons: Next Episode (near the end) and the segment's Skip. Not Skip Credits for
    // credits that run to the end: Next Episode is offered then.
    private val skipShown: Boolean get() = model.activeSegment?.let { seg ->
        (model.segmentPromptVisible || model.autoSkipProgress != null || controlsVisible) && !(seg.kind == SegmentKind.Credits && model.creditsSkipPlaysNext)
    } ?: false
    val nextShown: Boolean get() = model.nextEpisodeOffered && !nextLetGo
    val corner: List<String> get() = listOfNotNull(if (nextShown) "next" else null, if (skipShown) "skip" else null)
    /** A corner button that takes the remote as it appears. */
    val prompting: Boolean get() = model.segmentPromptVisible || model.autoSkipProgress != null || nextShown

    fun go(z: Zone, why: String = "") {
        if (z != zone) Log.d(FOCUS) { "player: ${z.name.lowercase()}${if (why.isNotEmpty()) " ($why)" else ""}" }
        val barNow = zone == Zone.Scrubber || zone == Zone.Buttons
        // The controls coming up again bring Next Episode back.
        if ((z == Zone.Scrubber || z == Zone.Buttons) && !barNow && !(zone == Zone.Corner && cornerOverControls)) nextLetGo = false
        if (z == Zone.Corner) cornerOverControls = barNow || (zone == Zone.Corner && cornerOverControls)
        zone = z
    }

    /** A corner button appeared: it takes the remote (as on Apple TV). */
    fun cornerAppeared(corner: List<String>) {
        cornerIndex = corner.lastIndex
        go(Zone.Corner, "${corner.last()} appeared")
    }

    /** The corner is empty again, or has fewer buttons. */
    fun cornerChanged(corner: List<String>) {
        if (corner.isEmpty() && zone == Zone.Corner) go(if (cornerOverControls) Zone.Scrubber else Zone.None, "corner gone")
        if (cornerIndex > corner.lastIndex) cornerIndex = maxOf(0, corner.lastIndex)
    }

    /** No press for a while: the controls go (Skip or Next Episode keeps the remote; only the controls go). */
    fun idle() {
        if (zone == Zone.Corner) { cornerOverControls = false; Log.d(FOCUS) { "player: controls hidden (idle), corner kept" } } else go(Zone.None, "idle")
    }

    fun openMenu(m: PlayerMenu) {
        Log.d(FOCUS) { "player: menu $m" }
        if (m == PlayerMenu.Search) {
            resumeAfterSearch = model.isPlaying
            if (model.isPlaying) model.togglePlay()
        }
        menu = m
    }

    fun closeMenu() {
        if (menu == PlayerMenu.Search && resumeAfterSearch && !model.isPlaying) model.togglePlay()
        if (menu == PlayerMenu.Search) resumeAfterSearch = false
        Log.d(FOCUS) { "player: menu closed" }
        menu = null
    }

    private fun letGoCorner() {
        val corner = corner
        Log.i("MoonriseSkip") { "let go of the corner (${corner.getOrNull(cornerIndex)})" }
        if ("skip" in corner) model.dismissSegmentPrompt()
        if ("next" in corner) nextLetGo = true
        go(if (cornerOverControls) Zone.Scrubber else Zone.None, "let go")
    }

    private fun activateCorner() {
        when (corner.getOrNull(cornerIndex)) {
            "next" -> model.playNextEpisode()
            "skip" -> model.skipSegment()
        }
    }

    private fun openPanel(why: String) {
        panelTab = firstTab(model)
        panelOnContent = false
        go(Zone.Panel, why)
        Log.d(FOCUS) { "player: panel ${panelTab.title} tab" }
    }

    /** A press of the remote; true when the player took it. */
    fun onKey(e: KeyEvent): Boolean {
        val code = e.keyCode
        val isDown = e.action == KeyEvent.ACTION_DOWN
        lastKey = System.currentTimeMillis()
        val select = code == KeyEvent.KEYCODE_DPAD_CENTER || code == KeyEvent.KEYCODE_ENTER || code == KeyEvent.KEYCODE_NUMPAD_ENTER
        // Back is handled on its way up (so the screen doesn't also leave on the down).
        if (code == KeyEvent.KEYCODE_BACK) {
            if (e.action != KeyEvent.ACTION_UP) return true
            when (zone) {
                Zone.Panel -> go(Zone.None, "back")
                Zone.Buttons, Zone.Scrubber -> go(Zone.None, "back")
                Zone.Corner -> letGoCorner()
                Zone.None -> { Log.i("MoonrisePlayer") { "back: leaving" }; onExit() }
            }
            return true
        }
        val media = code in setOf(KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE, KeyEvent.KEYCODE_MEDIA_PLAY, KeyEvent.KEYCODE_MEDIA_PAUSE,
            KeyEvent.KEYCODE_MEDIA_REWIND, KeyEvent.KEYCODE_MEDIA_FAST_FORWARD)
        val dpad = code in setOf(KeyEvent.KEYCODE_DPAD_LEFT, KeyEvent.KEYCODE_DPAD_RIGHT, KeyEvent.KEYCODE_DPAD_UP, KeyEvent.KEYCODE_DPAD_DOWN)
        if (!select && !media && !dpad) return false
        if (!isDown) return true
        if (select && e.repeatCount > 0) return true
        // Media keys work everywhere.
        when (code) {
            KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE, KeyEvent.KEYCODE_MEDIA_PLAY, KeyEvent.KEYCODE_MEDIA_PAUSE -> {
                if (zone == Zone.Corner) letGoCorner()
                when (code) {
                    KeyEvent.KEYCODE_MEDIA_PLAY -> model.play(true)
                    KeyEvent.KEYCODE_MEDIA_PAUSE -> model.play(false)
                    else -> model.togglePlay()
                }
                if (zone == Zone.None) go(Zone.Scrubber, "play/pause")
                return true
            }
            KeyEvent.KEYCODE_MEDIA_REWIND -> { model.skip(-AppSettings.skipBack.toDouble()); if (zone == Zone.None) go(Zone.Scrubber, "rewind"); return true }
            KeyEvent.KEYCODE_MEDIA_FAST_FORWARD -> { model.skip(AppSettings.skipForward.toDouble()); if (zone == Zone.None) go(Zone.Scrubber, "fast forward"); return true }
        }
        // Stopped and given up: select tries again.
        if (model.failure != null && select) { model.retryStream(); return true }
        val corner = corner
        when (zone) {
            Zone.None -> when {
                select -> { model.togglePlay(); go(Zone.Scrubber, "select") }
                code == KeyEvent.KEYCODE_DPAD_LEFT -> { model.skip(-AppSettings.skipBack.toDouble()); go(Zone.Scrubber, "skip back") }
                code == KeyEvent.KEYCODE_DPAD_RIGHT -> { model.skip(AppSettings.skipForward.toDouble()); go(Zone.Scrubber, "skip forward") }
                code == KeyEvent.KEYCODE_DPAD_UP -> go(Zone.Scrubber, "up")
                code == KeyEvent.KEYCODE_DPAD_DOWN -> openPanel("down")
            }
            Zone.Scrubber -> when {
                select -> model.togglePlay()
                code == KeyEvent.KEYCODE_DPAD_LEFT -> model.skip(-AppSettings.skipBack.toDouble())
                code == KeyEvent.KEYCODE_DPAD_RIGHT -> model.skip(AppSettings.skipForward.toDouble())
                code == KeyEvent.KEYCODE_DPAD_UP -> { go(Zone.Buttons, "up"); Log.d(FOCUS) { "player: button ${menuButtons[buttonIndex]}" } }
                code == KeyEvent.KEYCODE_DPAD_DOWN -> if (corner.isNotEmpty()) { cornerIndex = corner.lastIndex; go(Zone.Corner, "down"); Log.d(FOCUS) { "player: corner ${corner[cornerIndex]}" } }
                    else openPanel("down")
            }
            Zone.Buttons -> when {
                select -> openMenu(when (menuButtons[buttonIndex]) { "Audio" -> PlayerMenu.Audio; "Subtitles" -> PlayerMenu.Subtitles; else -> PlayerMenu.Quality })
                code == KeyEvent.KEYCODE_DPAD_LEFT -> { buttonIndex = (buttonIndex - 1).coerceAtLeast(0); Log.d(FOCUS) { "player: button ${menuButtons[buttonIndex]}" } }
                code == KeyEvent.KEYCODE_DPAD_RIGHT -> { buttonIndex = (buttonIndex + 1).coerceAtMost(menuButtons.lastIndex); Log.d(FOCUS) { "player: button ${menuButtons[buttonIndex]}" } }
                code == KeyEvent.KEYCODE_DPAD_DOWN -> go(Zone.Scrubber, "down")
            }
            Zone.Corner -> when {
                select -> activateCorner()
                code == KeyEvent.KEYCODE_DPAD_LEFT && cornerIndex > 0 -> { cornerIndex--; Log.d(FOCUS) { "player: corner ${corner[cornerIndex]}" } }
                code == KeyEvent.KEYCODE_DPAD_RIGHT && cornerIndex < corner.lastIndex -> { cornerIndex++; Log.d(FOCUS) { "player: corner ${corner[cornerIndex]}" } }
                code == KeyEvent.KEYCODE_DPAD_UP && cornerOverControls -> go(Zone.Scrubber, "up")
                code == KeyEvent.KEYCODE_DPAD_DOWN -> openPanel("down")
                // Anything else keeps watching: the button goes, and the remote is the player's again.
                else -> letGoCorner()
            }
            Zone.Panel -> onPanelKey(code, select)
        }
        return true
    }

    /** A press while the panel is open: its tabs, then the chosen tab's content. */
    private fun onPanelKey(code: Int, select: Boolean) {
        val tabs = panelTabs(model)
        if (panelTab !in tabs) panelTab = tabs.first()
        val ti = tabs.indexOf(panelTab)
        val episodes = model.seriesEpisodes
        when {
            !panelOnContent && code == KeyEvent.KEYCODE_DPAD_LEFT -> if (ti > 0) panelTab = tabs[ti - 1]
            !panelOnContent && code == KeyEvent.KEYCODE_DPAD_RIGHT -> if (ti < tabs.lastIndex) panelTab = tabs[ti + 1]
            !panelOnContent && code == KeyEvent.KEYCODE_DPAD_UP -> go(Zone.None, "panel closed")
            !panelOnContent && (code == KeyEvent.KEYCODE_DPAD_DOWN || select) -> when (panelTab) {
                PanelTab.Info -> if (select) go(Zone.None, "panel closed")
                PanelTab.Next -> panelOnContent = true
                PanelTab.Episodes -> {
                    panelOnContent = episodes.isNotEmpty()
                    episodeIndex = episodes.indexOfFirst { it.id == model.item.id }.coerceAtLeast(0)
                }
            }
            panelOnContent && code == KeyEvent.KEYCODE_DPAD_UP -> panelOnContent = false
            panelOnContent && panelTab == PanelTab.Episodes && code == KeyEvent.KEYCODE_DPAD_LEFT -> episodeIndex = (episodeIndex - 1).coerceAtLeast(0)
            panelOnContent && panelTab == PanelTab.Episodes && code == KeyEvent.KEYCODE_DPAD_RIGHT -> episodeIndex = (episodeIndex + 1).coerceAtMost(episodes.lastIndex)
            panelOnContent && select -> when (panelTab) {
                PanelTab.Next -> model.playNextEpisode()
                PanelTab.Episodes -> episodes.getOrNull(episodeIndex)?.let { ep ->
                    if (ep.id == model.item.id) go(Zone.None, "panel closed") else { Log.i("MoonriseNext") { "episodes panel: ${ep.id}" }; model.playEpisode(ep) }
                }
                PanelTab.Info -> {}
            }
        }
        if (zone == Zone.Panel) Log.d(FOCUS) { "player: panel ${panelTab.title}${if (panelOnContent) " content" + (if (panelTab == PanelTab.Episodes) " #$episodeIndex" else "") else " tab"}" }
    }
}

/** The controls' timers: the corner taking the remote, the controls hiding when idle and showing when paused, focus back after a menu. */
@Composable
internal fun PlayerControlsEffects(controls: PlayerControls, model: PlayerModel) {
    // As the screen was last drawn (the keys below), like the corner it shows.
    val corner = controls.corner
    val prompting = controls.prompting
    val zone = controls.zone
    val menu = controls.menu
    // A corner button appearing takes the remote (as on Apple TV).
    LaunchedEffect(model.activeSegment?.id, model.segmentPromptVisible, controls.nextShown) {
        if (corner.isNotEmpty() && prompting && controls.menu == null && controls.zone != Zone.Panel) controls.cornerAppeared(corner)
    }
    LaunchedEffect(corner.isEmpty(), zone) { controls.cornerChanged(corner) }

    // The controls go after 5 s without a press while playing.
    LaunchedEffect(zone, controls.lastKey, model.isPlaying, menu, controls.cornerOverControls) {
        if (controls.controlsVisible && model.isPlaying && controls.menu == null) {
            delay(5000)
            controls.idle()
        }
    }
    // Paused by the viewer: the controls show.
    LaunchedEffect(model.isPlaying, model.status) {
        if (!model.isPlaying && model.status == null && controls.zone == Zone.None && model.player?.playWhenReady == false) controls.go(Zone.Scrubber, "paused")
    }

    LaunchedEffect(menu) { if (controls.menu == null) { delay(30); runCatching { controls.keys.requestFocus() } } }
}
