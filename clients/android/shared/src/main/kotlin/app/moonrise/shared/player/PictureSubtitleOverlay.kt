package app.moonrise.shared.player

import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import app.moonrise.core.PictureSubtitles
import kotlin.math.roundToInt

/**
 * Picture subtitles (PGS, DVD, DVB) drawn over the video, for the TV and phone players: put it on a
 * layer the size of the player view. Each image goes where the track places it on its canvas, and
 * the canvas is stretched over the video's rectangle in the view (fitted, or filling the view when
 * `fill`, the images then kept inside the view). The subtitle size and background settings are for
 * text and don't apply; the delay does (`PlayerModel.updateSubtitle`).
 */
fun Modifier.pictureSubtitles(model: PlayerModel, fill: () -> Boolean = { false }): Modifier = drawBehind {
    val shown = model.subtitlePictures
    val canvas = model.pictureCanvas
    if (shown.isEmpty() || canvas == null || model.recovering || model.failure != null) return@drawBehind
    val video = PictureSubtitles.videoRect(size.width, size.height, model.videoAspect, fill())
    for (p in shown) {
        val r = PictureSubtitles.keepInside(PictureSubtitles.place(p.cue, canvas, video), size.width, size.height)
        drawImage(
            p.image,
            dstOffset = IntOffset(r.x.roundToInt(), r.y.roundToInt()),
            dstSize = IntSize(r.width.roundToInt().coerceAtLeast(1), r.height.roundToInt().coerceAtLeast(1)),
            filterQuality = FilterQuality.Medium,
        )
    }
}
