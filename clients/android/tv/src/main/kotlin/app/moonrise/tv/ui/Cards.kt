package app.moonrise.tv.ui

import androidx.compose.foundation.background
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.drawOutline
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.zIndex
import androidx.compose.ui.composed
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.foundation.relocation.bringIntoViewResponder
import app.moonrise.core.ClientRules
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.Border
import androidx.tv.material3.Card
import androidx.tv.material3.CardDefaults
import androidx.tv.material3.Glow
import androidx.tv.material3.Text
import app.moonrise.core.Item
import coil.compose.AsyncImage
import app.moonrise.shared.Session
import app.moonrise.shared.WatchMarks

/** The server's image `id`, sized for `width` on this screen. */
@Composable
fun imageUrl(id: String?, width: Dp): String? {
    val api = Session.api ?: return null
    val px = with(LocalDensity.current) { width.roundToPx() }
    return id?.let { api.imageUrl(it, px) }
}

/**
 * Artwork that grows and glows when focused, as tvOS lockups do: the picture scales up with a
 * soft light behind it, and the title below stays put.
 *
 * The scale and the glow follow the focus system's own state (`onFocusChanged`), not the card's
 * focus interactions: after quick key presses those could leave an unfocused card glowing.
 */
@Composable
private fun Lockup(
    width: Dp,
    aspect: Float,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    onLongClick: (() -> Unit)? = null,
    shape: androidx.compose.ui.graphics.Shape = RoundedCornerShape(Theme.cardRadius),
    name: String = "",
    artwork: @Composable () -> Unit,
    below: @Composable (focused: Boolean) -> Unit,
) {
    var focused by remember { mutableStateOf(false) }
    // One value drives the lift: the scale, the shadow under the card, its rim light and the sheen
    // across the artwork, so they move together (tvOS's focused lockup). It is read only while
    // drawing (the lambdas below), so an animation frame redraws the card without composing or
    // measuring anything again.
    val lift = animateFloatAsState(if (focused) 1f else 0f, FocusMotion.spec(), label = "lockup")
    val gap = with(LocalDensity.current) { 6.dp.toPx() }
    Column(modifier.width(width).zIndex(if (focused) 1f else 0f)) {
        Card(
            onClick = onClick,
            modifier = Modifier
                .fillMaxWidth()
                .aspectRatio(aspect)
                // Rows and the page scroll to the card at rest, not as drawn mid-lift.
                .restingBringIntoView { 1f + (ClientRules.LIFT_SCALE - 1f) * lift.value }
                .graphicsLayer {
                    scaleX = 1f + (ClientRules.LIFT_SCALE - 1f) * lift.value; scaleY = scaleX
                }
                .liftShadow(shape) { lift.value }
                .glow(shape) { lift.value }
                .sheen(shape) { lift.value }
                .onFocusChanged { focused = it.isFocused }
                .logFocus { "card $name" }
                .selectOrHold(onClick, onLongClick),
            shape = CardDefaults.shape(shape),
            scale = CardDefaults.scale(scale = 1f, focusedScale = 1f, pressedScale = 1f),
            border = CardDefaults.border(border = Border.None, focusedBorder = Border.None, pressedBorder = Border.None),
            glow = CardDefaults.glow(glow = Glow.None, focusedGlow = Glow.None, pressedGlow = Glow.None),
            colors = CardDefaults.colors(containerColor = Theme.surface, focusedContainerColor = Theme.surface, pressedContainerColor = Theme.surface),
        ) { artwork() }
        // The caption makes room for the grown card: it moves down with the lift (drawn, not laid out
        // again). Above the card's layer (tv-material raises a focused card), so the shadow stays under it.
        Box(Modifier.zIndex(2f).padding(top = 6.dp).graphicsLayer { translationY = gap * lift.value }) { below(focused) }
    }
}

/**
 * Asks the rows and the page around a card to show it at its resting size, whatever `scale` it is
 * drawn at (its lift). Compose's TV scrolling keeps the focused card at a fixed point measured
 * from its bounds as drawn, and reads them again every frame while it scrolls: a card growing
 * into focus moved that point, so each move along a row bobbed the whole page.
 * Placed outside the card's scaling layer, it gets the asked-for part as drawn and passes it on
 * at rest ([ClientRules.restingRect]).
 */
@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
private fun Modifier.restingBringIntoView(scale: () -> Float): Modifier = composed {
    val responder = remember { RestingResponder() }
    responder.scale = scale
    this.onSizeChanged { responder.size = it }.bringIntoViewResponder(responder)
}

@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
private class RestingResponder : androidx.compose.foundation.relocation.BringIntoViewResponder {
    var size = androidx.compose.ui.unit.IntSize.Zero
    var scale: () -> Float = { 1f }

    override fun calculateRectForParent(localRect: androidx.compose.ui.geometry.Rect): androidx.compose.ui.geometry.Rect {
        val r = ClientRules.restingRect(localRect.left, localRect.top, localRect.right, localRect.bottom, size.width.toFloat(), size.height.toFloat(), scale())
        return androidx.compose.ui.geometry.Rect(r[0], r[1], r[2], r[3])
    }

    // Nothing to scroll here: the rows and the page above do it, with the rect above.
    override suspend fun bringChildIntoView(localRect: () -> androidx.compose.ui.geometry.Rect?) {}
}

/**
 * The shadow a lifted card casts on the page: soft and dropped a little below it, drawn as a few
 * widening translucent outlines (cheap on a TV's GPU, unlike a blur). Drawn behind the card, so the
 * caption below stays on top of it.
 */
private fun Modifier.liftShadow(shape: androidx.compose.ui.graphics.Shape, amountOf: () -> Float): Modifier = drawBehind {
    val amount = amountOf()
    if (amount <= 0.01f) return@drawBehind
    val round = shape == androidx.compose.foundation.shape.CircleShape
    val drop = 8.dp.toPx() * amount
    val step = 3.dp.toPx()
    for (i in 1..6) {
        val grow = step * i
        val r = if (round) (size.minDimension / 2 + grow) else Theme.cardRadius.toPx() + grow
        drawRoundRect(
            Color.Black.copy(alpha = 0.075f * amount),
            topLeft = androidx.compose.ui.geometry.Offset(-grow, -grow + drop),
            size = androidx.compose.ui.geometry.Size(size.width + 2 * grow, size.height + 2 * grow),
            cornerRadius = androidx.compose.ui.geometry.CornerRadius(r, r),
        )
    }
}

/** A faint light across a lifted card's artwork, brightest at its top left (tvOS's specular sheen). */
private fun Modifier.sheen(shape: androidx.compose.ui.graphics.Shape, amountOf: () -> Float): Modifier = drawWithContent {
    drawContent()
    val amount = amountOf()
    if (amount <= 0.01f) return@drawWithContent
    val outline = shape.createOutline(size, layoutDirection, this)
    drawOutline(
        outline,
        Brush.linearGradient(
            0f to Color.White.copy(alpha = 0.16f * amount), 0.45f to Color.White.copy(alpha = 0.04f * amount), 1f to Color.Transparent,
            start = androidx.compose.ui.geometry.Offset.Zero, end = androidx.compose.ui.geometry.Offset(size.width, size.height),
        ),
    )
}

/** A soft white light around `shape`, at `amount` (0: none), as tv-material's focused glow. */
private fun Modifier.glow(shape: androidx.compose.ui.graphics.Shape, amountOf: () -> Float): Modifier = drawBehind {
    val amount = amountOf()
    if (amount <= 0f) return@drawBehind
    val outline = shape.createOutline(size, layoutDirection, this)
    val paint = androidx.compose.ui.graphics.Paint().apply {
        color = Color.White.copy(alpha = 0.35f * amount)
        asFrameworkPaint().maskFilter = android.graphics.BlurMaskFilter(12.dp.toPx(), android.graphics.BlurMaskFilter.Blur.NORMAL)
    }
    drawIntoCanvas { it.drawOutline(outline, paint) }
}

/**
 * Landscape card (`LandscapeCard`): an episode's still or the title's backdrop, progress, title and a
 * second line. Long press: the title's menu (Mark as Watched / Unwatched).
 */
@Composable
fun LandscapeCard(card: Item, onClick: () -> Unit, modifier: Modifier = Modifier, width: Dp = Theme.landscapeWidth, onLongClick: (() -> Unit)? = null) {
    val item = WatchMarks.current(card)
    Lockup(width, 16f / 9f, onClick, modifier, onLongClick, name = item.cardTitle, artwork = {
        Box(Modifier.fillMaxSize()) {
            val url = imageUrl(item.cardImage, width)
            if (url != null) {
                AsyncImage(model = url, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
            } else {
                Placeholder(if (item.isEpisode) item.title ?: "" else item.title ?: "")
            }
            Box(Modifier.fillMaxSize().background(Brush.verticalGradient(0.5f to Color.Transparent, 1f to Color.Black.copy(alpha = 0.6f))))
            item.progress?.let { ProgressBar(it.toFloat(), Modifier.align(Alignment.BottomCenter).padding(6.dp)) }
            WatchedMark(item, Modifier.align(Alignment.TopEnd))
        }
    }) {
        CardLabels(item, titleSize = 14.5.sp, subtitleSize = 12.5.sp)
    }
}

/** Poster card (`PosterCard`): the poster, progress, a watched mark or unwatched count, and two lines below. */
@Composable
fun PosterCard(card: Item, onClick: () -> Unit, modifier: Modifier = Modifier, width: Dp = Theme.posterWidth, onLongClick: (() -> Unit)? = null) {
    val item = WatchMarks.current(card)
    Lockup(width, 2f / 3f, onClick, modifier, onLongClick, name = item.cardTitle, artwork = {
        Box(Modifier.fillMaxSize()) {
            val url = imageUrl(item.posterImage, width)
            if (url != null) {
                AsyncImage(model = url, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
            } else {
                Placeholder(item.title ?: "")
            }
            item.progress?.let { ProgressBar(it.toFloat(), Modifier.align(Alignment.BottomCenter).padding(5.dp)) }
            WatchedMark(item, Modifier.align(Alignment.TopEnd))
        }
    }) { CardLabels(item, titleSize = 12.5.sp, subtitleSize = 11.5.sp) }
}

/** A poster-sized control at the end of a row of posters (Clear, say): its label on the card. */
@Composable
fun PosterAction(label: String, onClick: () -> Unit, modifier: Modifier = Modifier) {
    Lockup(Theme.posterWidth, 2f / 3f, onClick, modifier, name = label, artwork = {
        Box(Modifier.fillMaxSize().background(Color.White.copy(alpha = 0.10f)), contentAlignment = Alignment.Center) {
            Text(label, fontSize = 14.sp, fontWeight = FontWeight.Medium, color = Color.White, textAlign = TextAlign.Center, modifier = Modifier.padding(horizontal = 12.dp))
        }
    }) { }
}

/**
 * An episode in a show's page (`TVEpisodeLockup`): its still with progress and a watched mark, then
 * "4. Title" (bold for the episode the page is about) and its runtime.
 */
@Composable
fun EpisodeCard(card: Item, current: Boolean, onClick: () -> Unit, modifier: Modifier = Modifier, width: Dp = 200.dp, onLongClick: (() -> Unit)? = null) {
    val item = WatchMarks.current(card)
    Lockup(width, 16f / 9f, onClick, modifier, onLongClick, name = item.episodeListTitle, artwork = {
        Box(Modifier.fillMaxSize()) {
            val url = imageUrl(item.images?.thumb ?: item.cardImage, width)
            if (url != null) AsyncImage(model = url, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
            else Placeholder(item.title ?: "")
            item.progress?.let { ProgressBar(it.toFloat(), Modifier.align(Alignment.BottomCenter).padding(6.dp)) }
            WatchedMark(item, Modifier.align(Alignment.TopEnd))
        }
    }) {
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(item.episodeListTitle, fontSize = 14.sp, fontWeight = if (current) FontWeight.SemiBold else FontWeight.Medium,
                color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(app.moonrise.core.runtimeText(item.runtime) ?: " ", fontSize = 12.sp, color = Theme.secondaryText, maxLines = 1)
        }
    }
}

/** Someone in the cast or crew (`TVPersonLockup`): a round photo, the name and the role. */
@Composable
fun PersonCard(person: app.moonrise.core.Person, modifier: Modifier = Modifier, width: Dp = 100.dp) {
    Lockup(width, 1f, onClick = {}, modifier = modifier, name = person.name.orEmpty(), shape = androidx.compose.foundation.shape.CircleShape, artwork = {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            val url = imageUrl(person.image, width)
            if (url != null) AsyncImage(model = url, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
            else Text("👤", fontSize = 28.sp, color = Theme.secondaryText)
        }
    }) {
        Column(Modifier.width(width), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(1.dp)) {
            Text(person.name.orEmpty(), fontSize = 12.sp, fontWeight = FontWeight.Medium, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(if (person.group == "Actor") person.role.orEmpty() else person.group, fontSize = 11.sp, color = Theme.secondaryText, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** A category's tile (`LibraryTile`): artwork with a scrim, the name and how many titles. */
@Composable
fun TileCard(title: String, detail: String?, art: String?, onClick: () -> Unit, modifier: Modifier = Modifier, width: Dp = Theme.landscapeWidth) {
    Lockup(width, 16f / 9f, onClick, modifier, name = "tile $title", artwork = {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.BottomStart) {
            val url = imageUrl(art, width)
            if (url != null) AsyncImage(model = url, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
            else Box(Modifier.fillMaxSize().background(Brush.linearGradient(listOf(Color.White.copy(alpha = 0.22f), Theme.surface))))
            Box(Modifier.fillMaxSize().background(Brush.verticalGradient(0.25f to Color.Transparent, 0.55f to Color.Black.copy(alpha = 0.45f), 1f to Color.Black.copy(alpha = 0.85f))))
            Column(Modifier.padding(10.dp)) {
                Text(title, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis)
                detail?.let { Text(it, fontSize = 11.sp, fontWeight = FontWeight.Medium, color = Color.White.copy(alpha = 0.85f)) }
            }
        }
    }) { }
}

/**
 * The two lines under a card, as on Apple TV (tvOS `caption`/`caption2` under posters and
 * `subheadline`/`caption` under landscape cards, at half their point size): the title (the show's
 * name for episodes and seasons), then the year, the season, or "S2 · E6 · Title".
 */
@Composable
private fun CardLabels(item: Item, titleSize: TextUnit, subtitleSize: TextUnit) {
    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
        Text(item.cardTitle, fontSize = titleSize, fontWeight = FontWeight.Medium, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text(item.cardSubtitle, fontSize = subtitleSize, color = Theme.secondaryText, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** A library's card: its backdrop with its name over it. */
@Composable
fun LibraryCard(name: String, backdrop: String?, onClick: () -> Unit, modifier: Modifier = Modifier, width: Dp = Theme.landscapeWidth) {
    Lockup(width, 16f / 9f, onClick, modifier, artwork = {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            imageUrl(backdrop, width)?.let {
                AsyncImage(model = it, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
            }
            Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.45f)))
            Text(name, fontSize = 20.sp, fontWeight = FontWeight.Bold, color = Color.White)
        }
    }) { Text(" ", fontSize = 11.sp) }
}

@Composable
private fun Placeholder(title: String) {
    Box(
        Modifier.fillMaxSize().background(Brush.linearGradient(listOf(Color.White.copy(alpha = 0.14f), Color.White.copy(alpha = 0.04f)))),
        contentAlignment = Alignment.Center,
    ) {
        Text(title, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, color = Color.White.copy(alpha = 0.75f), textAlign = TextAlign.Center, maxLines = 2, modifier = Modifier.padding(8.dp))
    }
}

@Composable
fun ProgressBar(value: Float, modifier: Modifier = Modifier) {
    Box(modifier.fillMaxWidth().height(4.dp).clip(RoundedCornerShape(2.dp)).background(Color.White.copy(alpha = 0.3f))) {
        Box(Modifier.fillMaxWidth(value.coerceIn(0f, 1f)).height(4.dp).background(Color.White))
    }
}

@Composable
private fun WatchedMark(item: Item, modifier: Modifier) {
    val n = item.user?.unplayedItemCount
    when {
        item.user?.played == true -> Box(modifier.padding(5.dp).width(16.dp).height(16.dp).background(Color.White, RoundedCornerShape(50)), contentAlignment = Alignment.Center) {
            Text("✓", fontSize = 10.sp, color = Color.Black, fontWeight = FontWeight.Bold)
        }
        item.isSeries && n != null && n > 0 -> Text(
            "$n", fontSize = 10.sp, fontWeight = FontWeight.Bold, color = Color.Black,
            modifier = modifier.padding(5.dp).background(Color.White, RoundedCornerShape(50)).padding(horizontal = 5.dp, vertical = 1.dp),
        )
    }
}
