package app.moonrise.phone.player

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.ClosedCaption
import androidx.compose.material.icons.filled.CloseFullscreen
import androidx.compose.material.icons.filled.Forward10
import androidx.compose.material.icons.filled.Forward30
import androidx.compose.material.icons.filled.Forward5
import androidx.compose.material.icons.filled.GraphicEq
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.OpenInFull
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Replay
import androidx.compose.material.icons.filled.Replay10
import androidx.compose.material.icons.filled.Replay30
import androidx.compose.material.icons.filled.Replay5
import androidx.compose.material.icons.filled.ZoomInMap
import androidx.compose.material.icons.filled.ZoomOutMap
import androidx.compose.material.icons.outlined.ClosedCaption
import androidx.compose.material.icons.outlined.Speed
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import app.moonrise.core.NextEpisodeRules
import app.moonrise.core.SegmentKind
import app.moonrise.core.clockText
import app.moonrise.phone.ui.Theme
import app.moonrise.phone.ui.Type
import app.moonrise.shared.AppSettings
import app.moonrise.shared.player.PlayerModel

/**
 * The controls over the picture (`PlayerControls`, iPhone), in glass.
 *
 * - On the page (the 16:9 band): Back at the top left; Quality, Audio and Subtitles in a capsule at
 *   the top right; back, play/pause and forward in the middle; the time, the scrubber, the time
 *   left and Full Screen in a capsule at the bottom.
 * - Full screen: the chevron (back to the page) and Fill at the top left; the title at the bottom
 *   left, Quality, Audio and Subtitles at the bottom right; the scrubber under them; Episodes
 *   (shows) under it on the left.
 */
@Composable
internal fun Controls(
    model: PlayerModel, host: PlayerHost, fullScreen: Boolean, scrubbing: Double?,
    onScrub: (Double) -> Unit, onCommit: (Double) -> Unit, onBack: () -> Unit, onFullScreen: () -> Unit,
    onEpisodes: () -> Unit, onMenu: (Boolean) -> Unit, onSearch: () -> Unit, touched: () -> Unit,
) {
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(Brush.verticalGradient(0f to Color.Black.copy(alpha = 0.45f), 0.33f to Color.Transparent, 0.66f to Color.Transparent, 1f to Color.Black.copy(alpha = 0.5f))))
        if (fullScreen) {
            Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing).padding(horizontal = 20.dp, vertical = 12.dp)) {
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    GlassButton(Icons.Filled.KeyboardArrowDown, "Back to the page", onClick = onBack)
                    GlassBar {
                        BarButton(if (host.fill) Icons.Filled.ZoomInMap else Icons.Filled.ZoomOutMap, if (host.fill) "Fit" else "Fill") { host.fill = !host.fill; touched() }
                    }
                }
                Spacer(Modifier.weight(1f))
                Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        subtitleLine(model)?.let { Text(it, style = Type.subheadline, fontWeight = FontWeight.SemiBold, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                        Text(titleLine(model), style = Type.title2, fontWeight = FontWeight.Bold, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    GlassBar { TrackButtons(model, onMenu, onSearch) }
                }
                Spacer(Modifier.height(12.dp))
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    TimeText(clockText(scrubbing ?: model.shownTime))
                    Scrubber(model, scrubbing, onScrub, onCommit, Modifier.weight(1f))
                    TimeText("−" + clockText(maxOf(0.0, model.duration - (scrubbing ?: model.shownTime))))
                }
                val corner = NextEpisodeRules.corner(model.activeSegment, true, model.segmentPromptVisible, model.autoSkipProgress != null, model.nextEpisodeOffered, model.creditsSkipPlaysNext)
                if (model.seriesEpisodes.isNotEmpty() || !corner.empty) {
                    Row(Modifier.padding(top = 8.dp).height(40.dp), verticalAlignment = Alignment.CenterVertically) {
                        if (model.seriesEpisodes.isNotEmpty()) {
                            Text("Episodes", style = Type.headline, color = Color.White, modifier = Modifier.height(40.dp).clip(CircleShape).background(Theme.glass)
                                .clickable(onClick = onEpisodes).padding(horizontal = 18.dp, vertical = 9.dp))
                        }
                    }
                }
            }
            Transport(model, small = false, Modifier.align(Alignment.Center), touched)
        } else {
            Column(Modifier.fillMaxSize().padding(horizontal = 10.dp, vertical = 8.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    GlassButton(Icons.AutoMirrored.Filled.ArrowBack, "Back", onClick = onBack)
                    Spacer(Modifier.weight(1f))
                    GlassBar { TrackButtons(model, onMenu, onSearch) }
                }
                Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) { Transport(model, small = true, Modifier, touched) }
                val corner = NextEpisodeRules.corner(model.activeSegment, true, model.segmentPromptVisible, model.autoSkipProgress != null, model.nextEpisodeOffered, model.creditsSkipPlaysNext)
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    Row(Modifier.weight(1f).height(40.dp).clip(CircleShape).background(Theme.glass).padding(horizontal = 12.dp),
                        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        TimeText(clockText(scrubbing ?: model.shownTime))
                        Scrubber(model, scrubbing, onScrub, onCommit, Modifier.weight(1f))
                        // No room for it beside Skip Intro.
                        if (corner.empty) TimeText("−" + clockText(maxOf(0.0, model.duration - (scrubbing ?: model.shownTime))))
                        Icon(Icons.Filled.OpenInFull, "Full Screen", tint = Color.White, modifier = Modifier.size(28.dp).clip(CircleShape).clickable(onClick = onFullScreen).padding(5.dp))
                    }
                    // Skip's place: drawn over it by the player, the same with the controls up or not.
                    if (!corner.empty) CornerButtons(model, corner, compact = true, placeholder = true)
                }
            }
        }
    }
}

private fun titleLine(model: PlayerModel): String {
    val i = model.item
    return if (i.isEpisode) i.title ?: i.seriesTitle.orEmpty() else i.title.orEmpty()
}

/** Full screen, above the title: the show and the episode's number ("Harbor Lights · S1 · E1"). */
private fun subtitleLine(model: PlayerModel): String? {
    val i = model.item
    if (!i.isEpisode) return i.year?.toString()
    return listOfNotNull(i.seriesTitle, i.episodeLabel?.takeIf { it.isNotEmpty() }).joinToString(" · ").ifEmpty { null }
}

@Composable
private fun TimeText(text: String) {
    Text(text, style = Type.footnote, fontWeight = FontWeight.Medium, color = Color.White, maxLines = 1)
}

/** Back by the Settings' length, play/pause (a ring while it buffers), forward; skips pressed in a row add up. */
@Composable
private fun Transport(model: PlayerModel, small: Boolean, modifier: Modifier, touched: () -> Unit) {
    if (model.failure != null) return
    val play = if (small) 30.dp else 46.dp
    val icon = if (small) 18.dp else 22.dp
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(if (small) 28.dp else 40.dp), verticalAlignment = Alignment.CenterVertically) {
        val back = AppSettings.skipBack
        val fwd = AppSettings.skipForward
        Box(contentAlignment = Alignment.BottomCenter) {
            RoundGlass(play * 1.3f, if (back == 5) Icons.Filled.Replay5 else if (back == 30) Icons.Filled.Replay30 else if (back == 10) Icons.Filled.Replay10 else Icons.Filled.Replay,
                icon + 8.dp, "Go Backward $back Seconds") { model.skip(-back.toDouble()); touched() }
            SkipTotal(model, forward = false, small)
        }
        Box(contentAlignment = Alignment.Center) {
            RoundGlass(play * 1.75f, if (model.isPlaying) Icons.Filled.Pause else Icons.Filled.PlayArrow, play * 0.85f, if (model.isPlaying) "Pause" else "Play") { model.togglePlay(); touched() }
            if (model.buffering && model.player != null) CircularProgressIndicator(color = Color.White, strokeWidth = 2.dp, modifier = Modifier.size(play * 1.75f + 8.dp))
        }
        Box(contentAlignment = Alignment.BottomCenter) {
            RoundGlass(play * 1.3f, if (fwd == 5) Icons.Filled.Forward5 else if (fwd == 30) Icons.Filled.Forward30 else Icons.Filled.Forward10,
                icon + 8.dp, "Go Forward $fwd Seconds") { model.skip(fwd.toDouble()); touched() }
            SkipTotal(model, forward = true, small)
        }
    }
}

@Composable
private fun SkipTotal(model: PlayerModel, forward: Boolean, small: Boolean) {
    val total = model.skipTotal ?: return
    if (total == 0.0 || (total > 0) != forward) return
    Text("%+.0f s".format(java.util.Locale.ROOT, total), style = if (small) Type.caption2 else Type.caption, fontWeight = FontWeight.SemiBold, color = Color.White,
        modifier = Modifier.offset(y = if (small) 22.dp else 26.dp).background(Color.Black.copy(alpha = 0.45f), CircleShape).padding(horizontal = 7.dp, vertical = 3.dp))
}

@Composable
private fun RoundGlass(size: Dp, icon: ImageVector, iconSize: Dp, description: String, onClick: () -> Unit) {
    Box(Modifier.size(size).clip(CircleShape).background(Theme.glass.copy(alpha = 0.6f)).border(0.5.dp, Color.White.copy(alpha = 0.2f), CircleShape).clickable(onClick = onClick),
        contentAlignment = Alignment.Center) {
        Icon(icon, description, tint = Color.White, modifier = Modifier.size(iconSize))
    }
}

/** A round glass button at the top left (Back, the full-screen chevron). */
@Composable
private fun GlassButton(icon: ImageVector, description: String, onClick: () -> Unit) {
    Box(Modifier.size(44.dp).clip(CircleShape).background(Theme.glass.copy(alpha = 0.6f)).border(0.5.dp, Color.White.copy(alpha = 0.2f), CircleShape).clickable(onClick = onClick),
        contentAlignment = Alignment.Center) {
        Icon(icon, description, tint = Color.White, modifier = Modifier.size(22.dp))
    }
}

/** A glass capsule of 44 dp buttons. */
@Composable
private fun GlassBar(content: @Composable RowScope.() -> Unit) {
    Row(Modifier.height(44.dp).clip(CircleShape).background(Theme.glass.copy(alpha = 0.6f)).border(0.5.dp, Color.White.copy(alpha = 0.2f), CircleShape).padding(horizontal = 6.dp),
        verticalAlignment = Alignment.CenterVertically, content = content)
}

@Composable
internal fun BarButton(icon: ImageVector, description: String, onClick: () -> Unit) {
    Box(Modifier.size(44.dp).clip(CircleShape).clickable(onClick = onClick), contentAlignment = Alignment.Center) {
        Icon(icon, description, tint = Color.White, modifier = Modifier.size(22.dp))
    }
}

/** Quality, Audio (several tracks) and Subtitles, each a menu. */
@Composable
private fun TrackButtons(model: PlayerModel, onMenu: (Boolean) -> Unit, onSearch: () -> Unit) {
    // A download plays as it was saved (the iPhone's Download button here instead).
    if (model.local == null) MenuButton(Icons.Outlined.Speed, "Quality", onMenu) { close -> QualityMenu(model, close) }
    app.moonrise.phone.ui.DownloadButton(model.item, size = 32.dp, iconSize = 22.dp, ringInset = 0.24f, onMenu = onMenu)
    if (model.audioOptions.size > 1) MenuButton(Icons.Filled.GraphicEq, "Audio", onMenu) { close -> AudioMenu(model, close) }
    MenuButton(if (model.selectedSubtitleId == null) Icons.Outlined.ClosedCaption else Icons.Filled.ClosedCaption, "Subtitles", onMenu) { close ->
        SubtitlesMenu(model, close, onSearch)
    }
}

/** The scrubber: drag to a time (its thumbnail above the finger), let go to go there. */
@Composable
private fun Scrubber(model: PlayerModel, scrubbing: Double?, onScrub: (Double) -> Unit, onCommit: (Double) -> Unit, modifier: Modifier) {
    val d = model.duration
    val t = scrubbing ?: model.shownTime
    val frac = if (d > 0) (t / d).toFloat().coerceIn(0f, 1f) else 0f
    BoxWithConstraints(modifier.height(28.dp), contentAlignment = Alignment.CenterStart) {
        val w = constraints.maxWidth.toFloat()
        val wDp = maxWidth
        fun at(x: Float) = if (d > 0) (x / w).coerceIn(0f, 1f) * d else 0.0
        Box(Modifier.fillMaxWidth().height(4.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.3f))) {
            Box(Modifier.fillMaxWidth(frac).fillMaxHeight().background(Color.White))
        }
        Box(Modifier.offset(x = wDp * frac - 7.dp).size(14.dp).clip(CircleShape).background(Color.White))
        Box(Modifier.fillMaxSize()
            .pointerInput(d) { detectTapGestures { p -> onCommit(at(p.x)) } }
            .pointerInput(d) {
                var last = 0.0
                detectHorizontalDragGestures(
                    onDragStart = { p -> last = at(p.x); onScrub(last) },
                    onDragEnd = { onCommit(last) },
                    onDragCancel = { onCommit(last) },
                    onHorizontalDrag = { c, _ -> last = at(c.position.x); onScrub(last) },
                )
            })
        // While dragging (or skipping), the picture there, above the bar.
        val thumbs = model.thumbnails
        val shown = scrubbing ?: model.previewTime
        if (shown != null && thumbs != null && thumbs.ready) {
            val pw = 160.dp
            val ph = pw * (thumbs.height!!.toFloat() / thumbs.width!!)
            val atFrac = if (d > 0) (shown / d).toFloat().coerceIn(0f, 1f) else 0f
            val tile = thumbs.tile(shown)
            val sheet = tile?.let { model.thumbnailSheet(it.sheet) }
            Canvas(Modifier.offset(x = (wDp * atFrac - pw / 2).coerceIn(0.dp, maxOf(0.dp, wDp - pw)), y = -(ph + 10.dp)).size(pw, ph)
                .clip(RoundedCornerShape(8.dp)).background(Color.Black.copy(alpha = 0.6f)).border(1.5.dp, Color.White.copy(alpha = 0.85f), RoundedCornerShape(8.dp))) {
                if (tile != null && sheet != null) {
                    drawImage(sheet, srcOffset = IntOffset(tile.x, tile.y), srcSize = IntSize(thumbs.width!!, thumbs.height!!), dstSize = IntSize(size.width.toInt(), size.height.toInt()))
                }
            }
        }
    }
}

/**
 * Next Episode (with the controls, near the end) and the segment's Skip (Intro, Recap, Credits…),
 * in glass; an auto-skip's countdown fills Skip from the left. `placeholder`: only its room.
 */
@Composable
internal fun CornerButtons(model: PlayerModel, corner: NextEpisodeRules.Corner, compact: Boolean, placeholder: Boolean = false) {
    Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        if (corner.next) CornerButton("Next Episode", null, compact, placeholder) { model.playNextEpisode() }
        val seg = model.activeSegment
        if (corner.skip && seg != null) {
            val label = if (seg.kind == SegmentKind.Credits && model.creditsSkipPlaysNext) "Next Episode" else seg.kind.skipLabel
            CornerButton(label, model.autoSkipProgress, compact, placeholder) { model.skipSegment() }
        }
    }
}

@Composable
private fun CornerButton(label: String, progress: Double?, compact: Boolean, placeholder: Boolean, onClick: () -> Unit) {
    val style = if (compact) Type.footnote.copy(fontWeight = FontWeight.SemiBold) else Type.headline
    Box(Modifier.height(40.dp).clip(CircleShape).then(if (placeholder) Modifier else Modifier.background(Theme.glass).clickable(onClick = onClick)),
        contentAlignment = Alignment.CenterStart) {
        if (!placeholder && progress != null) Box(Modifier.matchParentSize()) {
            Box(Modifier.fillMaxHeight().fillMaxWidth(progress.toFloat().coerceIn(0f, 1f)).background(Color.White.copy(alpha = 0.35f)))
        }
        Text(label, style = style, color = if (placeholder) Color.Transparent else Color.White, modifier = Modifier.padding(horizontal = if (compact) 12.dp else 18.dp))
    }
}
