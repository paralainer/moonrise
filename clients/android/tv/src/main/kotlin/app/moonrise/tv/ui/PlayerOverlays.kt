package app.moonrise.tv.ui

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.draw.scale
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shadow
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.Text
import app.moonrise.core.SegmentKind
import app.moonrise.core.Thumbnails
import app.moonrise.core.clockText
import app.moonrise.shared.player.PlayerModel
import kotlinx.coroutines.delay
import app.moonrise.shared.AppSettings

// What the player draws over the picture: subtitles, the controls, the corner's buttons and the
// recovery overlay.

/** Text subtitles, drawn by the app: white with a shadow, or on a box; higher while the controls show. */
/** The space below subtitles while the controls show (text, and picture subtitles in the lower half). */
internal val RAISED_SUBTITLE_BOTTOM = 150.dp

@Composable
internal fun SubtitleOverlay(text: String?, raised: Boolean, height: Dp) {
    if (text.isNullOrBlank()) return
    val size = 23.sp * AppSettings.subtitleScale
    val box = AppSettings.subtitleBackground
    Box(Modifier.fillMaxSize().padding(bottom = if (raised) RAISED_SUBTITLE_BOTTOM else maxOf(height * 0.06f, 23.dp), start = 60.dp, end = 60.dp), contentAlignment = Alignment.BottomCenter) {
        Text(
            text, color = Color.White, fontSize = size, lineHeight = size * 1.2f, textAlign = TextAlign.Center, fontWeight = FontWeight.Medium,
            style = if (box) TextStyle.Default else TextStyle(shadow = Shadow(Color.Black, blurRadius = 6f)),
            modifier = if (box) Modifier.background(Color.Black.copy(alpha = 0.65f), RoundedCornerShape(4.dp)).padding(horizontal = 8.dp, vertical = 2.dp) else Modifier,
        )
    }
}

/** The bottom of the screen while the controls show: title, menus, scrubber, times and the panel's tabs. */
@Composable
internal fun TransportBar(model: PlayerModel, focus: Zone, buttonIndex: Int) {
    val item = model.item
    val title = if (item.isEpisode) item.seriesTitle ?: item.title.orEmpty() else item.title.orEmpty()
    val sub = if (item.isEpisode) listOfNotNull(item.episodeLabel, item.title).joinToString(" · ") else item.year?.toString()
    Column(
        Modifier.fillMaxWidth()
            .background(Brush.verticalGradient(listOf(Color.Transparent, Color.Black.copy(alpha = 0.75f))))
            .padding(start = Theme.margin, end = Theme.margin, top = 60.dp, bottom = 22.dp),
    ) {
        Row(verticalAlignment = Alignment.Bottom) {
            Column(Modifier.weight(1f)) {
                if (!sub.isNullOrEmpty()) Text(sub, fontSize = 13.sp, color = Theme.secondaryText, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(title, fontSize = 20.sp, fontWeight = FontWeight.Bold, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                menuButtons.forEachIndexed { i, b ->
                    val f = focus == Zone.Buttons && i == buttonIndex
                    Text(b, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, color = if (f) Color.Black else Color.White,
                        modifier = Modifier.scale(if (f) 1.06f else 1f)
                            .background(if (f) Color.White else Color.White.copy(alpha = 0.16f), RoundedCornerShape(50))
                            .padding(horizontal = 14.dp, vertical = 6.dp))
                }
            }
        }
        Spacer(Modifier.height(14.dp))
        Scrubber(model, focused = focus == Zone.Scrubber)
        Spacer(Modifier.height(10.dp))
        // The panel's tabs, as the Apple TV shows them under its scrubber (down opens it). As
        // tall as Skip and Next Episode, which sit at the end of this row.
        Row(Modifier.height(34.dp), horizontalArrangement = Arrangement.spacedBy(18.dp), verticalAlignment = Alignment.CenterVertically) {
            panelTabs(model).forEach { t -> Text(t.title, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, color = Color.White.copy(alpha = 0.55f)) }
        }
    }
}

@Composable
private fun Scrubber(model: PlayerModel, focused: Boolean) {
    val d = model.duration
    val t = model.shownTime
    val frac = if (d > 0) (t / d).toFloat().coerceIn(0f, 1f) else 0f
    val playedFrac = if (d > 0) (model.currentTime / d).toFloat().coerceIn(0f, 1f) else 0f
    val buffered = model.player?.let { p -> if (d > 0) (p.bufferedPosition / 1000.0 / d).toFloat().coerceIn(0f, 1f) else 0f } ?: 0f
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        val w = maxWidth
        // While seeking (presses adding up), the picture there, over the bar.
        val thumbs = model.thumbnails
        val at = model.previewTime
        if (at != null && thumbs != null) {
            val pw = 208.dp
            val ph = pw * (thumbs.height!!.toFloat() / thumbs.width!!)
            val atFrac = if (d > 0) (at / d).toFloat().coerceIn(0f, 1f) else 0f
            ScrubThumbnail(model, thumbs, at, Modifier.offset(x = (w * atFrac - pw / 2).coerceIn(0.dp, maxOf(0.dp, w - pw)), y = -(ph + 14.dp)).size(pw, ph))
        }
        Column {
            Box(Modifier.fillMaxWidth().height(if (focused) 8.dp else 6.dp).clip(RoundedCornerShape(50)).background(Color.White.copy(alpha = 0.22f))) {
                Box(Modifier.fillMaxWidth(buffered).fillMaxHeight().background(Color.White.copy(alpha = 0.25f)))
                if (model.skipTarget != null) Box(Modifier.fillMaxWidth(frac).fillMaxHeight().background(Color.White.copy(alpha = 0.55f)))
                Box(Modifier.fillMaxWidth(playedFrac).fillMaxHeight().background(Color.White))
            }
            Spacer(Modifier.height(6.dp))
            Box(Modifier.fillMaxWidth().height(18.dp)) {
                val glyph = if (model.isPlaying) "❚❚" else "▶"
                val total = model.skipTotal?.let { "  %+d s".format(it.toInt()) } ?: ""
                Text("$glyph  ${clockText(t)}$total", fontSize = 13.sp, color = Color.White, fontWeight = FontWeight.SemiBold, maxLines = 1, softWrap = false,
                    modifier = Modifier.offset(x = (w * frac - 30.dp).coerceIn(0.dp, maxOf(0.dp, w - 160.dp))))
                if (d > 0) Text("-" + clockText(maxOf(0.0, d - t)), fontSize = 13.sp, color = Theme.secondaryText, modifier = Modifier.align(Alignment.CenterEnd))
            }
        }
    }
}

/** The thumbnail for `time`: its tile of the sheet, or a dark frame while the sheet loads. */
@Composable
private fun ScrubThumbnail(model: PlayerModel, thumbs: Thumbnails, time: Double, modifier: Modifier) {
    val tile = thumbs.tile(time)
    val sheet = tile?.let { model.thumbnailSheet(it.sheet) }
    Canvas(modifier.clip(RoundedCornerShape(8.dp)).background(Color.Black.copy(alpha = 0.6f)).border(1.5.dp, Color.White.copy(alpha = 0.85f), RoundedCornerShape(8.dp))) {
        if (tile != null && sheet != null) {
            drawImage(sheet, srcOffset = IntOffset(tile.x, tile.y), srcSize = IntSize(thumbs.width!!, thumbs.height!!),
                dstSize = IntSize(size.width.toInt(), size.height.toInt()))
        }
    }
}

/** The bottom corner: Next Episode (near the end) and the segment's Skip; `focusIndex` has the remote, if any. */
@Composable
internal fun CornerButtons(model: PlayerModel, corner: List<String>, focusIndex: Int, modifier: Modifier) {
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        corner.forEachIndexed { i, c ->
            val focused = i == focusIndex
            when (c) {
                "next" -> CornerButton("Next Episode", null, focused)
                "skip" -> model.activeSegment?.let { seg ->
                    val label = if (seg.kind == SegmentKind.Credits && model.creditsSkipPlaysNext) "Next Episode" else seg.kind.skipLabel
                    val left = model.autoSkipProgress?.let { p -> maxOf(1, Math.ceil((1 - p) * AppSettings.autoSkipDelay).toInt()) }
                    CornerButton(if (left != null) "$label ($left)" else label, model.autoSkipProgress, focused)
                }
            }
        }
    }
}

/** Skip Intro / Next Episode in glass; a countdown fills it left to right. */
@Composable
private fun CornerButton(label: String, progress: Double?, focused: Boolean) {
    Box(
        Modifier.scale(if (focused) 1.06f else 1f).height(34.dp).clip(RoundedCornerShape(50))
            .background(Color.White.copy(alpha = if (focused) 0.32f else 0.16f))
            .drawBehind { progress?.let { p -> drawRect(Color.White.copy(alpha = 0.35f), size = Size(size.width * p.toFloat(), size.height)) } }
            .border(1.dp, Color.White.copy(alpha = if (focused) 0.7f else 0.2f), RoundedCornerShape(50)),
        contentAlignment = Alignment.CenterStart,
    ) {
        Text(label, fontSize = 14.sp, fontWeight = FontWeight.Medium, color = Color.White, modifier = Modifier.padding(horizontal = 18.dp))
    }
}

/** A spinning ring. */
@Composable
private fun Spinner(size: Dp) {
    val t = rememberInfiniteTransition(label = "spinner")
    val angle by t.animateFloat(0f, 360f, infiniteRepeatable(tween(900, easing = LinearEasing), RepeatMode.Restart), label = "angle")
    Canvas(Modifier.size(size).rotate(angle)) {
        drawArc(Color.White, 0f, 270f, useCenter = false, style = Stroke(width = 3.dp.toPx(), cap = StrokeCap.Round))
    }
}

/** A lost stream being brought back (a spinner, "Reconnecting…" after a few seconds), or given up on. */
@Composable
internal fun RecoveryOverlay(model: PlayerModel) {
    val f = model.failure
    if (!model.recovering && f == null) return
    var slow by remember { mutableStateOf(false) }
    LaunchedEffect(model.recovering) { slow = false; delay(3000); slow = true }
    Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = if (f != null) 0.6f else 0.35f)), contentAlignment = Alignment.Center) {
        if (f != null) {
            Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.widthIn(max = 520.dp)) {
                Text("⚠", fontSize = 32.sp, color = Color.White)
                Text(f.title, fontSize = 18.sp, fontWeight = FontWeight.SemiBold, color = Color.White, modifier = Modifier.padding(top = 6.dp))
                Text(f.detail, fontSize = 14.sp, color = Color.White.copy(alpha = 0.75f), textAlign = TextAlign.Center, modifier = Modifier.padding(top = 4.dp))
                Text("Press Play to try again", fontSize = 14.sp, fontWeight = FontWeight.SemiBold, color = Color.White.copy(alpha = 0.9f), modifier = Modifier.padding(top = 14.dp))
            }
        } else {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Spinner(36.dp)
                if (slow) Text("Reconnecting…", fontSize = 14.sp, fontWeight = FontWeight.Medium, color = Color.White.copy(alpha = 0.85f), modifier = Modifier.padding(top = 10.dp))
            }
        }
    }
}
