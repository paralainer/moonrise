package app.moonrise.tv.ui

import android.view.ViewGroup
import androidx.annotation.OptIn
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.media3.common.util.UnstableApi
import androidx.media3.ui.AspectRatioFrameLayout
import androidx.media3.ui.PlayerView
import app.moonrise.shared.player.pictureSubtitles
import androidx.tv.material3.Text
import app.moonrise.core.Item
import app.moonrise.shared.player.PlayerModel
import app.moonrise.shared.displayHdr
import app.moonrise.shared.Session

// The player screen. The rest of the player's UI is beside it: PlayerControls.kt (where the remote
// is and what each press does), PlayerOverlays.kt (subtitles, the controls, the corner, recovery),
// PlayerPanel.kt (the panel from the top) and PlayerMenus.kt (Audio, Subtitles, Quality, Search).

/**
 * Plays a title as the Apple TV app does: the server's session through Media3, with the app's own
 * controls over the picture.
 *
 * - **The remote, controls hidden:** select or play/pause pauses and resumes (and shows the
 *   controls); left and right skip by the Settings' lengths (presses add up to one seek); up shows
 *   the controls; down opens the panel; Back leaves.
 * - **The controls:** the title, the scrubber with the time played and the time left, and Audio,
 *   Subtitles and Quality above it on the right (up from the scrubber). Down goes to Skip or Next
 *   Episode when they show, else opens the panel; Back hides the controls.
 * - **The panel** (the Apple TV's swipe-down): Next Episode near the end of an episode, Info
 *   (artwork, title, overview, age rating, genres, year, quality) and Episodes.
 * - **Skip Intro, Skip Credits and Next Episode** in the bottom corner take the remote as they
 *   appear: select skips; any other press keeps watching.
 */
@OptIn(UnstableApi::class)
@Composable
fun PlayerScreen(item: Item, start: Double?, version: String?, onExit: () -> Unit, onPlayEpisode: (Item) -> Unit) {
    val context = LocalContext.current
    val api = Session.api ?: return onExit()
    val model = remember { PlayerModel(context, api, item, version, start, displayHdr(context), onPlayEpisode) }

    val lifecycle = LocalLifecycleOwner.current.lifecycle
    DisposableEffect(lifecycle) {
        // Compose stops recomposing while the activity is stopped, so this works on the model directly.
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_STOP -> model.stop("app in background")
                Lifecycle.Event.ON_START -> if (model.stopped) model.open(model.position, autoplay = false)
                else -> {}
            }
        }
        model.start()
        lifecycle.addObserver(observer)
        onDispose {
            lifecycle.removeObserver(observer)
            model.close("left the player")
        }
    }
    LaunchedEffect(model.finished) { if (model.finished) onExit() }

    val controls = rememberPlayerControls(model, onExit)
    PlayerControlsEffects(controls, model)
    val zone = controls.zone
    val controlsVisible = controls.controlsVisible
    val corner = controls.corner

    Box(
        Modifier
            .fillMaxSize()
            .background(Color.Black)
            .onPreviewKeyEvent { e -> controls.onKey(e.nativeKeyEvent) }
            .focusRequester(controls.keys)
            .focusable(),
    ) {
        AndroidView(
            factory = { ctx ->
                PlayerView(ctx).apply {
                    layoutParams = ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
                    useController = false
                    resizeMode = AspectRatioFrameLayout.RESIZE_MODE_FIT
                    // Subtitles are the app's own (below).
                    subtitleView?.visibility = android.view.View.GONE
                    isFocusable = false
                    descendantFocusability = ViewGroup.FOCUS_BLOCK_DESCENDANTS
                }
            },
            update = {
                it.player = if (app.moonrise.tv.DebugSeams.noVideo) null else model.player
                // Only while the video plays: paused lets the TV's screensaver start.
                it.keepScreenOn = model.keepScreenOn
            },
            modifier = Modifier.fillMaxSize(),
        )

        // Picture subtitles where the track places them on the picture; text ones below.
        Box(Modifier.fillMaxSize().pictureSubtitles(model, raisedBottom = { if (controlsVisible) RAISED_SUBTITLE_BOTTOM else null }))
        BoxWithConstraints(Modifier.fillMaxSize()) {
            SubtitleOverlay(model.subtitleText.takeIf { !model.recovering && model.failure == null }, raised = controlsVisible, height = maxHeight)
        }

        // The controls.
        AnimatedVisibility(controlsVisible, enter = fadeIn(), exit = fadeOut(), modifier = Modifier.align(Alignment.BottomCenter)) {
            TransportBar(model, focus = zone, buttonIndex = controls.buttonIndex)
        }

        // Skip Intro, Next Episode.
        if (corner.isNotEmpty()) {
            CornerButtons(model, corner, focusIndex = if (zone == Zone.Corner) controls.cornerIndex else -1,
                Modifier.align(Alignment.BottomEnd).padding(end = Theme.margin, bottom = 22.dp))
        }

        // The panel from the top.
        AnimatedVisibility(zone == Zone.Panel, enter = fadeIn(), exit = fadeOut(), modifier = Modifier.align(Alignment.TopCenter)) {
            InfoPanel(model, panelTabs(model), controls.panelTab, controls.panelOnContent, controls.episodeIndex)
        }

        // Loading, errors, a lost stream.
        RecoveryOverlay(model)
        val message = model.error ?: model.status
        if (message != null && !model.recovering && model.failure == null) {
            Column(Modifier.align(Alignment.Center).widthIn(max = 600.dp).padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                // The server's (or the network's) answer: its title and a plain line.
                Text(model.problem?.title ?: model.item.title.orEmpty(), fontSize = 20.sp, fontWeight = FontWeight.Bold, color = Color.White)
                Text(message, fontSize = 15.sp, color = Theme.secondaryText, textAlign = TextAlign.Center, modifier = Modifier.padding(top = 8.dp))
            }
        }
        model.notice?.let { n ->
            Text(n, fontSize = 13.sp, color = Color.White, modifier = Modifier.align(Alignment.TopCenter).padding(top = 24.dp)
                .background(Color.Black.copy(alpha = 0.7f), RoundedCornerShape(50)).padding(horizontal = 16.dp, vertical = 8.dp))
        }
    }

    PlayerMenus(model, controls.menu, open = { controls.openMenu(it) }, close = { controls.closeMenu() })
}
