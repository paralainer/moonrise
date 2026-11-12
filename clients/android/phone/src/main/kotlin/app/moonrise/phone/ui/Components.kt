package app.moonrise.phone.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.outlined.AutoFixHigh
import androidx.compose.material.icons.outlined.RemoveCircleOutline
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material.icons.outlined.Movie
import androidx.compose.material.icons.outlined.Tv
import androidx.compose.material.icons.outlined.Warning
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.moonrise.core.Item
import app.moonrise.core.WatchAction
import app.moonrise.shared.Session
import app.moonrise.shared.WatchMarks
import coil.compose.AsyncImage
import kotlinx.coroutines.launch

/** The server's image `id` at about `width` on screen (pixels), or null. Images need no token. */
@Composable
fun imageUrl(id: String?, width: Dp): String? {
    id ?: return null
    val px = with(LocalDensity.current) { width.roundToPx() }
    // Steps, so the same artwork at nearby sizes is one cached image.
    val w = listOf(240, 400, 600, 900, 1280, 1920).firstOrNull { it >= px } ?: 1920
    return Session.api?.imageUrl(id, w)
}

/** Artwork that fills its frame and crops to it; the surface colour until it loads (`RemoteImage`). */
@Composable
fun RemoteImage(url: String?, modifier: Modifier = Modifier, contentScale: ContentScale = ContentScale.Crop, alignment: Alignment = Alignment.Center, placeholder: Boolean = true) {
    Box(modifier.then(if (placeholder && contentScale == ContentScale.Crop) Modifier.background(Theme.surface) else Modifier)) {
        if (url != null) AsyncImage(model = url, contentDescription = null, contentScale = contentScale, alignment = alignment, modifier = Modifier.fillMaxSize())
    }
}

/** The thin bar of how far a title was watched (`ProgressBar`). */
@Composable
fun ProgressBar(value: Double, modifier: Modifier = Modifier) {
    Box(modifier.fillMaxWidth().height(4.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.5f))) {
        Box(Modifier.fillMaxWidth(value.toFloat().coerceIn(0f, 1f)).fillMaxHeight().clip(CircleShape).background(Theme.accent))
    }
}

/** Watched: a check at the corner; a show with episodes left: how many (`WatchedMark`). */
@Composable
fun WatchedMark(item: Item, modifier: Modifier = Modifier) {
    val n = item.user?.unplayedItemCount
    if (item.user?.played == true) {
        Box(modifier.padding(5.dp).size(18.dp).clip(CircleShape).background(Color.Black), contentAlignment = Alignment.Center) {
            Icon(Icons.Filled.CheckCircle, null, tint = Theme.accent, modifier = Modifier.size(18.dp))
        }
    } else if (n != null && n > 0 && item.isSeries) {
        Text("$n", style = Type.caption2, fontWeight = FontWeight.Bold, color = Color.Black,
            modifier = modifier.padding(5.dp).background(Theme.accent, CircleShape).padding(horizontal = 6.dp, vertical = 2.dp))
    }
}

/** No artwork: the title on a soft gradient, with a film or TV symbol. */
@Composable
private fun NoArtwork(item: Item, iconSize: Dp, lines: Int, title: String) {
    Box(Modifier.fillMaxSize().background(Brush.linearGradient(listOf(Color.White.copy(alpha = 0.14f), Color.White.copy(alpha = 0.04f)))), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(8.dp)) {
            Icon(if (item.isSeries || item.isEpisode) Icons.Outlined.Tv else Icons.Outlined.Movie, null, tint = Color.White.copy(alpha = 0.75f), modifier = Modifier.size(iconSize))
            Text(title, style = Type.caption, fontWeight = FontWeight.SemiBold, color = Color.White.copy(alpha = 0.75f), textAlign = TextAlign.Center, maxLines = lines, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** A tap opens, a long press opens the card's menu (the iPhone's context menu). */
fun Modifier.cardClick(onClick: () -> Unit, onLongClick: (() -> Unit)?): Modifier =
    this.combinedClickable(interactionSource = null, indication = null, onClick = onClick, onLongClick = onLongClick)

/** 2:3 poster with the title and the year (or the season, or "S1 · E3 · Title") below (`PosterCard`). */
@Composable
fun PosterCard(card: Item, width: Dp = LocalMetrics.current.posterWidth.dp, onClick: () -> Unit, menu: CardMenu? = null) {
    val item = WatchMarks.current(card)
    val haptic = LocalHapticFeedback.current
    var open by remember { mutableStateOf(false) }
    Column(Modifier.width(width).cardClick(onClick, if (menu != null) ({ haptic.performHapticFeedback(HapticFeedbackType.LongPress); open = true }) else null),
        verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Box {
            Box(Modifier.width(width).height(width * 1.5f).shadow(6.dp, RoundedCornerShape(12.dp)).clip(RoundedCornerShape(12.dp))) {
                val url = imageUrl(item.posterImage, width)
                if (url != null) RemoteImage(url, Modifier.fillMaxSize()) else NoArtwork(item, width * 0.22f, 3, item.title.orEmpty())
                item.progress?.let { ProgressBar(it, Modifier.align(Alignment.BottomCenter).padding(6.dp)) }
            }
            WatchedMark(item, Modifier.align(Alignment.TopEnd))
            DownloadMark(item)
            if (menu != null) CardMenuPopup(open, { open = false }, item, menu)
        }
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(item.cardTitle, style = Type.caption, fontWeight = FontWeight.Medium, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(item.cardSubtitle, style = Type.caption2, color = Theme.secondaryText, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** 16:9 card for Continue Watching and episodes (`LandscapeCard`). */
@Composable
fun LandscapeCard(card: Item, width: Dp = LocalMetrics.current.landscapeWidth.dp, onClick: () -> Unit, menu: CardMenu? = null) {
    val item = WatchMarks.current(card)
    val haptic = LocalHapticFeedback.current
    var open by remember { mutableStateOf(false) }
    Column(Modifier.width(width).cardClick(onClick, if (menu != null) ({ haptic.performHapticFeedback(HapticFeedbackType.LongPress); open = true }) else null)) {
        Box {
            Box(Modifier.width(width).height(width * 9f / 16f).shadow(6.dp, RoundedCornerShape(16.dp)).clip(RoundedCornerShape(16.dp))) {
                val url = imageUrl(item.cardImage, width)
                if (url != null) RemoteImage(url, Modifier.fillMaxSize())
                else NoArtwork(item, width * 0.12f, 2, if (item.isEpisode) item.title ?: item.cardTitle else item.cardTitle)
                Box(Modifier.fillMaxSize().background(Brush.verticalGradient(0.5f to Color.Transparent, 1f to Color.Black.copy(alpha = 0.6f))))
                item.progress?.let { ProgressBar(it, Modifier.align(Alignment.BottomCenter).padding(8.dp)) }
            }
            DownloadMark(item)
            if (menu != null) CardMenuPopup(open, { open = false }, item, menu)
        }
        Spacer(Modifier.height(6.dp))
        Text(item.cardTitle, style = Type.subheadline, fontWeight = FontWeight.Medium, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Spacer(Modifier.height(6.dp))
        Text(item.cardSubtitle, style = Type.caption, color = Theme.secondaryText, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** What a card's long press offers, beyond Mark as Watched / Unwatched. */
data class CardMenu(
    /** Resume or Play (Continue Watching). */
    val play: (() -> Unit)? = null,
    /** Remove from Continue Watching. */
    val remove: (() -> Unit)? = null,
    /** After a mark the server took. */
    val marked: () -> Unit = {},
)

/** The long-press menu: Resume or Play, Mark as Watched / Unwatched, Remove from Continue Watching. */
@Composable
fun CardMenuPopup(open: Boolean, dismiss: () -> Unit, item: Item, menu: CardMenu) {
    val scope = rememberCoroutineScope()
    DropdownMenu(expanded = open, onDismissRequest = dismiss, containerColor = Theme.sheet) {
        menu.play?.let { play ->
            MenuItem(if (item.resumeSeconds != null) "Resume" else "Play", Icons.Filled.PlayArrow) { dismiss(); play() }
        }
        for (a in WatchAction.options(item)) {
            MenuItem(a.title, if (a.played) Icons.Outlined.Visibility else Icons.Outlined.VisibilityOff) {
                dismiss()
                scope.launch { if (WatchMarks.set(item, a.played)) menu.marked() }
            }
        }
        menu.remove?.let { remove ->
            MenuItem("Remove from Continue Watching", Icons.Outlined.RemoveCircleOutline, destructive = true) { dismiss(); remove() }
        }
        // Admins: Fix Match, on any title (`FixMatchMenuItem`).
        if (AdminState.isAdmin && FixMatchPresenter.applies(item)) {
            MenuItem("Fix Match…", Icons.Outlined.AutoFixHigh) { dismiss(); FixMatchPresenter.present(item.id) }
        }
    }
}

@Composable
fun MenuItem(title: String, icon: ImageVector? = null, destructive: Boolean = false, checked: Boolean? = null, detail: String? = null, enabled: Boolean = true, onClick: () -> Unit) {
    val color = if (destructive) Theme.destructive else Color.White
    DropdownMenuItem(
        text = {
            Column {
                Text(title, style = Type.body, color = if (enabled) color else Theme.secondaryText)
                if (detail != null) Text(detail, style = Type.footnote, color = Theme.secondaryText)
            }
        },
        onClick = onClick,
        enabled = enabled,
        leadingIcon = if (checked != null) ({
            Box(Modifier.width(20.dp)) { if (checked) Icon(Icons.Filled.Check, null, tint = Color.White, modifier = Modifier.size(18.dp)) }
        }) else null,
        trailingIcon = icon?.let { { Icon(it, null, tint = color, modifier = Modifier.size(20.dp)) } },
    )
}

/** A row's title at the page margin (`SectionHeader`). */
@Composable
fun SectionHeader(title: String, modifier: Modifier = Modifier, action: (@Composable () -> Unit)? = null) {
    Row(modifier.fillMaxWidth().padding(horizontal = Theme.margin), verticalAlignment = Alignment.CenterVertically) {
        Text(title, style = Type.title3, fontWeight = FontWeight.SemiBold, color = Color.White, modifier = Modifier.weight(1f))
        action?.invoke()
    }
}

/** A titled row scrolling sideways, its cards starting at the page margin (`Shelf`). */
@Composable
fun Shelf(title: String, state: LazyListState = rememberLazyListState(), action: (@Composable () -> Unit)? = null, content: LazyListScope.() -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        SectionHeader(title, action = action)
        LazyRow(state = state, contentPadding = PaddingValues(horizontal = Theme.margin), horizontalArrangement = Arrangement.spacedBy(Theme.rowSpacing),
            verticalAlignment = Alignment.Top, content = content)
    }
}

/** A small outlined technical badge ("4K", "HDR", "5.1"). */
@Composable
fun Badge(text: String) {
    Text(text, style = Type.caption2, fontWeight = FontWeight.Bold, color = Color.White.copy(alpha = 0.85f), maxLines = 1,
        modifier = Modifier.border(1.dp, Color.White.copy(alpha = 0.5f), RoundedCornerShape(4.dp)).padding(horizontal = 6.dp, vertical = 3.dp))
}

/** A round glass button over the page (Settings, back). */
@Composable
fun GlassCircle(icon: ImageVector, description: String, modifier: Modifier = Modifier, size: Dp = 44.dp, onClick: () -> Unit) {
    Box(modifier.size(size).clip(CircleShape).background(Theme.glass).border(0.5.dp, Color.White.copy(alpha = 0.18f), CircleShape).clickable(onClick = onClick),
        contentAlignment = Alignment.Center) {
        Icon(icon, description, tint = Color.White, modifier = Modifier.size(size * 0.5f))
    }
}

/** A glass capsule with a label (Sort, Filter, Clear). */
@Composable
fun GlassCapsule(modifier: Modifier = Modifier, height: Dp = 44.dp, selected: Boolean = false, onClick: () -> Unit, content: @Composable () -> Unit) {
    Row(modifier.height(height).clip(CircleShape).background(if (selected) Color.White else Theme.glass)
        .border(0.5.dp, Color.White.copy(alpha = 0.18f), CircleShape).clickable(onClick = onClick).padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) { content() }
}

/** Nothing to show, and why (`EmptyStateView`). */
@Composable
fun EmptyState(icon: ImageVector, title: String, message: String, modifier: Modifier = Modifier) {
    Column(modifier.fillMaxWidth().padding(40.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Icon(icon, null, tint = Theme.accent, modifier = Modifier.size(48.dp))
        Text(title, style = Type.title3, fontWeight = FontWeight.SemiBold, color = Color.White)
        Text(message, style = Type.subheadline, color = Theme.secondaryText, textAlign = TextAlign.Center)
    }
}

/** A screen that couldn't load: why, Retry, and Sign In Again when the server no longer takes this sign-in. */
@Composable
fun ErrorPane(message: String, onRetry: () -> Unit, modifier: Modifier = Modifier, signIn: (() -> Unit)? = null) {
    Column(modifier.fillMaxWidth().padding(horizontal = 30.dp, vertical = 40.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Icon(Icons.Outlined.Warning, null, tint = Theme.secondaryText, modifier = Modifier.size(40.dp))
        Text(message, style = Type.subheadline, color = Theme.secondaryText, textAlign = TextAlign.Center)
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            GlassCapsule(onClick = onRetry) { Text("Retry", style = Type.headline, color = Color.White) }
            if (signIn != null) GlassCapsule(onClick = signIn) { Text("Sign In Again", style = Type.headline, color = Color.White) }
        }
    }
}

/** A white capsule button with black text (Connect to Your Server, Sign In). */
@Composable
fun WhiteCapsule(text: String, modifier: Modifier = Modifier, icon: ImageVector? = null, enabled: Boolean = true, onClick: () -> Unit) {
    Row(modifier.clip(CircleShape).background(if (enabled) Color.White else Color.White.copy(alpha = 0.4f)).clickable(enabled = enabled, onClick = onClick)
        .padding(horizontal = 28.dp, vertical = 14.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally)) {
        if (icon != null) Icon(icon, null, tint = Color.Black, modifier = Modifier.size(20.dp))
        Text(text, style = Type.headline, color = Color.Black)
    }
}

/** A page's whole background: the iPhone's gradient. */
@Composable
fun PageBackground(content: @Composable BoxScope.() -> Unit) {
    Box(Modifier.fillMaxSize().background(Theme.backgroundGradient), content = content)
}

@Composable
fun ArtworkImage(id: Int, modifier: Modifier = Modifier) {
    Image(androidx.compose.ui.res.painterResource(id), null, contentScale = ContentScale.Crop, modifier = modifier)
}
