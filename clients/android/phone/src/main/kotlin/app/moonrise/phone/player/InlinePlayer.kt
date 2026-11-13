package app.moonrise.phone.player

import android.app.Activity
import android.content.Context
import android.media.AudioManager
import android.view.ViewGroup
import androidx.annotation.OptIn
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.BrightnessHigh
import androidx.compose.material.icons.filled.BrightnessLow
import androidx.compose.material.icons.automirrored.filled.VolumeDown
import androidx.compose.material.icons.automirrored.filled.VolumeOff
import androidx.compose.material.icons.automirrored.filled.VolumeUp
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shadow
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.MediaSession
import androidx.media3.ui.AspectRatioFrameLayout
import androidx.media3.ui.PlayerView
import app.moonrise.shared.player.pictureSubtitles
import app.moonrise.core.NextEpisodeRules
import app.moonrise.phone.ui.DetailsCapsule
import app.moonrise.phone.ui.ProblemDetailsDialog
import app.moonrise.phone.ui.Theme
import app.moonrise.phone.ui.Type
import app.moonrise.shared.AppSettings
import app.moonrise.shared.player.PlayerModel
import kotlinx.coroutines.delay
import java.util.concurrent.atomic.AtomicInteger

private val sessions = AtomicInteger()

/**
 * The page's player (`PlayerScreen` with `InlinePlayback`, iPhone): the picture through Media3's
 * view, the app's own controls and subtitles over it. `fullScreen`: the phone is on its side and
 * the video fills the screen (system bars hidden); else a 16:9 band at the top of the page.
 *
 * A tap shows or hides the controls (they go after 4 s while playing); a double tap on the left or
 * right third skips back or forward (taps in a row add up), in the middle fills or fits the picture,
 * as a pinch does. Full screen, a vertical swipe on the left half sets the brightness, on the right
 * half the volume, with a level at the top right. Leaving the app stops playback (the stop is
 * reported); coming back opens it again where it was, paused.
 */
@OptIn(UnstableApi::class)
@Composable
fun InlinePlayer(host: PlayerHost, fullScreen: Boolean, onBack: () -> Unit, modifier: Modifier = Modifier, toggleFullScreen: (() -> Unit)? = null) {
    val model = host.model ?: return
    val context = LocalContext.current
    val view = LocalView.current

    // Leaving the app stops playback; coming back opens it again, paused.
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    DisposableEffect(lifecycle, model) {
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_STOP -> model.stop("app in background")
                Lifecycle.Event.ON_START -> if (model.stopped) model.open(model.position, autoplay = false)
                else -> {}
            }
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    // The system's media controls (headset buttons, Bluetooth, the media panel) for what plays.
    val player = model.player
    DisposableEffect(player) {
        val session = player?.let { MediaSession.Builder(context, it).setId("moonrise-${sessions.incrementAndGet()}").build() }
        onDispose { session?.release() }
    }
    // Full screen: the system bars go (a swipe from the edge shows them for a moment).
    DisposableEffect(fullScreen) {
        val window = (context as? Activity)?.window
        val controller = window?.let { WindowCompat.getInsetsController(it, view) }
        if (fullScreen && controller != null) {
            controller.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            controller.hide(WindowInsetsCompat.Type.systemBars())
        }
        onDispose {
            controller?.show(WindowInsetsCompat.Type.systemBars())
            // The brightness a swipe set is this window's: back to the system's when full screen ends.
            if (fullScreen) Levels.resetBrightness(context)
        }
    }
    // Back, full screen: back to the page (the phone turns upright), as the chevron does.
    val turn: () -> Unit = toggleFullScreen ?: { Rotation.request(context, landscape = !fullScreen) }
    androidx.activity.compose.BackHandler(enabled = fullScreen) { turn() }
    LaunchedEffect(model.finished) { if (model.finished) host.stop("finished") }

    var controls by remember { mutableStateOf(true) }
    var interaction by remember { mutableIntStateOf(0) }
    var menuOpen by remember { mutableStateOf(false) }
    var scrubbing by remember { mutableStateOf<Double?>(null) }
    var hud by remember { mutableStateOf<Hud?>(null) }
    var episodes by remember { mutableStateOf(false) }
    var search by remember { mutableStateOf(false) }
    // While playing, the controls go after 4 s without a touch (not while a menu is open or scrubbing).
    LaunchedEffect(controls, interaction, model.isPlaying, menuOpen, scrubbing != null) {
        if (controls && model.isPlaying && !menuOpen && scrubbing == null) { delay(4000); controls = false }
    }
    fun touched() { interaction++ }

    Box(modifier.then(if (fullScreen) Modifier.fillMaxSize() else Modifier).background(Color.Black)) {
        AndroidView(
            factory = { ctx ->
                PlayerView(ctx).apply {
                    layoutParams = ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
                    useController = false
                    // Subtitles are the app's own (below).
                    subtitleView?.visibility = android.view.View.GONE
                }
            },
            update = {
                it.player = model.player
                // Only while the video plays: paused lets the screen time out.
                it.keepScreenOn = model.keepScreenOn
                it.resizeMode = if (host.fill) AspectRatioFrameLayout.RESIZE_MODE_ZOOM else AspectRatioFrameLayout.RESIZE_MODE_FIT
            },
            modifier = Modifier.fillMaxSize(),
        )

        // Picture subtitles where the track places them on the picture (cropped with it when it fills the screen).
        Box(Modifier.fillMaxSize().pictureSubtitles(model, fill = { host.fill }, raisedBottom = { if (controls) raisedSubtitleBottom(fullScreen) else null }))
        BoxWithConstraints(Modifier.fillMaxSize()) {
            SubtitleText(model.subtitleText.takeIf { !model.recovering && model.failure == null }, raised = controls, fullScreen = fullScreen)
            // Taps, double taps, pinches; full screen, brightness and volume.
            val width = constraints.maxWidth.toFloat()
            val height = constraints.maxHeight.toFloat()
            val latestControls by rememberUpdatedState(controls)
            Box(Modifier.fillMaxSize()
                .pointerInput(model) {
                    detectTapGestures(
                        onTap = {
                            if (model.autoSkipProgress != null) model.cancelAutoSkip()
                            controls = !latestControls
                            touched()
                        },
                        onDoubleTap = { p ->
                            if (model.autoSkipProgress != null) model.cancelAutoSkip()
                            val third = width / 3
                            when {
                                p.x < third -> { model.skip(-AppSettings.skipBack.toDouble()); hud = Hud.Skip(model.skipTotal ?: -AppSettings.skipBack.toDouble()) }
                                p.x > third * 2 -> { model.skip(AppSettings.skipForward.toDouble()); hud = Hud.Skip(model.skipTotal ?: AppSettings.skipForward.toDouble()) }
                                else -> host.fill = !host.fill
                            }
                            touched()
                        },
                    )
                }
                .pointerInput(model) { detectTransformGestures { _, _, zoom, _ -> if (zoom > 1.15f) host.fill = true else if (zoom < 0.87f) host.fill = false } }
                .then(if (fullScreen) Modifier.pointerInput(model) {
                    var start = 0.0
                    var brightness = true
                    var travel = 0f
                    detectVerticalDragGestures(
                        onDragStart = { p ->
                            brightness = p.x < width / 2
                            travel = 0f
                            start = if (brightness) Levels.brightness(context) else Levels.volume(context)
                        },
                        onDragEnd = { touched() },
                        onVerticalDrag = { _, dy ->
                            travel += dy
                            val v = (start - travel / (height * 0.8f)).coerceIn(0.0, 1.0)
                            if (brightness) { Levels.setBrightness(context, v); hud = Hud.Brightness(v) } else { Levels.setVolume(context, v); hud = Hud.Volume(v) }
                        },
                    )
                } else Modifier))
        }

        // The controls.
        AnimatedVisibility(controls, enter = fadeIn(), exit = fadeOut(), modifier = Modifier.fillMaxSize()) {
            Controls(model, host, fullScreen, scrubbing, onScrub = { scrubbing = it; touched() }, onCommit = { model.seek(it); scrubbing = null; touched() },
                onBack = {
                    if (fullScreen) turn() else onBack()
                },
                onFullScreen = { turn(); touched() },
                onEpisodes = { episodes = true; touched() }, onMenu = { menuOpen = it; touched() }, onSearch = { search = true }, touched = ::touched)
        }

        // Skip Intro and Next Episode, at the end of the bottom row whether the controls show or not.
        val corner = NextEpisodeRules.corner(model.activeSegment, controls, model.segmentPromptVisible, model.autoSkipProgress != null,
            model.nextEpisodeOffered, model.creditsSkipPlaysNext)
        if (!corner.empty && model.failure == null) {
            Box(Modifier.fillMaxSize().then(if (fullScreen) Modifier.windowInsetsPadding(WindowInsets.safeDrawing) else Modifier)
                .padding(horizontal = if (fullScreen) 20.dp else 10.dp, vertical = if (fullScreen) 12.dp else 8.dp), contentAlignment = Alignment.BottomEnd) {
                CornerButtons(model, corner, compact = !fullScreen)
            }
        }

        // Brightness and volume at the top right; a double tap's skip in the middle.
        hud?.let { h ->
            LaunchedEffect(h) { delay(700); hud = null }
            HudView(h, Modifier.fillMaxSize().then(if (fullScreen) Modifier.windowInsetsPadding(WindowInsets.safeDrawing) else Modifier)
                .padding(horizontal = if (fullScreen) 20.dp else 10.dp, vertical = if (fullScreen) 12.dp else 8.dp))
        }

        RecoveryOverlay(model)
        val message = model.error
        if (message != null && !model.recovering && model.failure == null) {
            // The server's (or the network's) answer: its title, a plain line, Details.
            val problem = model.problem
            var details by remember { mutableStateOf(false) }
            Column(Modifier.align(Alignment.Center).widthIn(max = 520.dp).padding(horizontal = 24.dp, vertical = 12.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                problem?.let { Text(it.title, style = Type.headline, color = Color.White, textAlign = TextAlign.Center) }
                Text(message, style = if (problem != null) Type.footnote else Type.subheadline, color = Color.White.copy(alpha = if (problem != null) 0.8f else 1f),
                    textAlign = TextAlign.Center, modifier = Modifier.padding(top = 4.dp))
                if (problem?.detail != null) Box(Modifier.padding(top = 10.dp)) { DetailsCapsule { details = true } }
            }
            if (details && problem != null) ProblemDetailsDialog(problem) { details = false }
        }
        model.notice?.let { n ->
            Text(n, style = Type.footnote, color = Color.White, modifier = Modifier.align(Alignment.TopCenter).padding(top = if (fullScreen) 24.dp else 56.dp)
                .background(Color.Black.copy(alpha = 0.7f), CircleShape).padding(horizontal = 16.dp, vertical = 8.dp))
        }
    }

    if (episodes) EpisodesSheet(model, onDismiss = { episodes = false })
    if (search) SubtitleSearchSheet(model, onDismiss = { search = false })
}

/** The space below subtitles while the controls show (text, and picture subtitles in the lower half). */
private fun raisedSubtitleBottom(fullScreen: Boolean): Dp = if (fullScreen) 110.dp else 52.dp

/** Text subtitles, drawn by the app: white with a shadow, or on a box; higher while the controls show. */
@Composable
private fun SubtitleText(text: String?, raised: Boolean, fullScreen: Boolean) {
    if (text.isNullOrBlank()) return
    val size = (if (fullScreen) 20.sp else 15.sp) * AppSettings.subtitleScale
    val box = AppSettings.subtitleBackground
    val bottom = when { raised -> raisedSubtitleBottom(fullScreen); fullScreen -> 24.dp; else -> 10.dp }
    Box(Modifier.fillMaxSize().padding(bottom = bottom, start = 24.dp, end = 24.dp), contentAlignment = Alignment.BottomCenter) {
        Text(text, color = Color.White, fontSize = size, lineHeight = size * 1.2f, textAlign = TextAlign.Center, fontWeight = FontWeight.Medium,
            style = if (box) TextStyle.Default else TextStyle(shadow = Shadow(Color.Black, blurRadius = 6f)),
            modifier = if (box) Modifier.background(Color.Black.copy(alpha = 0.65f), RoundedCornerShape(4.dp)).padding(horizontal = 8.dp, vertical = 2.dp) else Modifier)
    }
}

/** What a gesture shows: a level (brightness, volume) or a skip's total. */
sealed interface Hud {
    data class Brightness(val value: Double) : Hud
    data class Volume(val value: Double) : Hud
    data class Skip(val seconds: Double) : Hud
}

@Composable
private fun HudView(hud: Hud, modifier: Modifier) {
    when (hud) {
        is Hud.Skip -> Box(modifier, contentAlignment = Alignment.Center) {
            Text((if (hud.seconds > 0) "» " else "« ") + "${Math.abs(hud.seconds).toInt()}s", style = Type.title2, fontWeight = FontWeight.SemiBold, color = Color.White,
                modifier = Modifier.clip(CircleShape).background(Theme.glass).padding(horizontal = 22.dp, vertical = 14.dp))
        }
        is Hud.Brightness -> Level(if (hud.value < 0.4) Icons.Filled.BrightnessLow else Icons.Filled.BrightnessHigh, hud.value, modifier)
        is Hud.Volume -> Level(when { hud.value == 0.0 -> Icons.AutoMirrored.Filled.VolumeOff; hud.value < 0.5 -> Icons.AutoMirrored.Filled.VolumeDown; else -> Icons.AutoMirrored.Filled.VolumeUp }, hud.value, modifier)
    }
}

/** A pill as tall as the top row's buttons, at its right end. */
@Composable
private fun Level(icon: ImageVector, value: Double, modifier: Modifier) {
    Box(modifier, contentAlignment = Alignment.TopEnd) {
        Row(Modifier.height(44.dp).clip(CircleShape).background(Theme.glass).padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Icon(icon, null, tint = Color.White, modifier = Modifier.size(20.dp))
            Box(Modifier.width(110.dp).height(5.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.25f))) {
                Box(Modifier.fillMaxHeight().fillMaxWidth(value.toFloat()).clip(CircleShape).background(Color.White))
            }
        }
    }
}

/** The screen's brightness (this window's) and the media volume, 0…1. */
private object Levels {
    fun brightness(context: Context): Double {
        val w = (context as? Activity)?.window ?: return 0.5
        val b = w.attributes.screenBrightness
        if (b >= 0) return b.toDouble()
        return runCatching { android.provider.Settings.System.getInt(context.contentResolver, android.provider.Settings.System.SCREEN_BRIGHTNESS) / 255.0 }.getOrDefault(0.5)
    }

    fun setBrightness(context: Context, v: Double) {
        val w = (context as? Activity)?.window ?: return
        w.attributes = w.attributes.apply { screenBrightness = v.toFloat().coerceIn(0.01f, 1f) }
    }

    fun resetBrightness(context: Context) {
        val w = (context as? Activity)?.window ?: return
        w.attributes = w.attributes.apply { screenBrightness = android.view.WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE }
    }

    fun volume(context: Context): Double {
        val am = context.getSystemService(Context.AUDIO_SERVICE) as AudioManager
        return am.getStreamVolume(AudioManager.STREAM_MUSIC).toDouble() / am.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
    }

    fun setVolume(context: Context, v: Double) {
        val am = context.getSystemService(Context.AUDIO_SERVICE) as AudioManager
        val max = am.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
        val index = Math.round(v * max).toInt()
        if (index != am.getStreamVolume(AudioManager.STREAM_MUSIC)) am.setStreamVolume(AudioManager.STREAM_MUSIC, index, 0)
    }
}

/** A lost stream being brought back ("Reconnecting…" after a few seconds), or given up on. */
@Composable
private fun RecoveryOverlay(model: PlayerModel) {
    val f = model.failure
    if (!model.recovering && f == null) return
    var slow by remember { mutableStateOf(false) }
    LaunchedEffect(model.recovering) { slow = false; delay(3000); slow = true }
    Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = if (f != null) 0.6f else 0.35f)), contentAlignment = Alignment.Center) {
        if (f != null) {
            Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.widthIn(max = 420.dp).padding(16.dp)) {
                Text(f.title, style = Type.headline, color = Color.White, textAlign = TextAlign.Center)
                Text(f.detail, style = Type.footnote, color = Color.White.copy(alpha = 0.75f), textAlign = TextAlign.Center, modifier = Modifier.padding(top = 4.dp))
                Text("Try Again", style = Type.headline, color = Color.Black, modifier = Modifier.padding(top = 14.dp).clip(CircleShape).background(Color.White)
                    .pointerInput(Unit) { detectTapGestures { model.retryStream() } }.padding(horizontal = 18.dp, vertical = 8.dp))
            }
        } else {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                CircularProgressIndicator(color = Color.White, strokeWidth = 3.dp, modifier = Modifier.size(32.dp))
                if (slow) Text("Reconnecting…", style = Type.subheadline, color = Color.White.copy(alpha = 0.85f), modifier = Modifier.padding(top = 10.dp))
            }
        }
    }
}
