package app.moonrise.tv.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.tv.material3.ClickableSurfaceDefaults
import androidx.tv.material3.Surface
import androidx.tv.material3.Text
import app.moonrise.core.ClientRules
import app.moonrise.core.MoonriseHttpException
import android.view.KeyEvent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.composed
import androidx.compose.ui.input.key.onPreviewKeyEvent
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import app.moonrise.shared.Log
import app.moonrise.shared.Session
import app.moonrise.shared.WatchMarks

/** One row of a menu: a label, an optional second line, a checkmark, greyed out when disabled. */
data class MenuRow(
    val label: String,
    val detail: String? = null,
    val checked: Boolean = false,
    val enabled: Boolean = true,
    val destructive: Boolean = false,
    val action: () -> Unit,
)

data class MenuSection(val title: String? = null, val rows: List<MenuRow>)

/**
 * A menu over the screen, as tvOS presents its menus: a dark glass panel with sections of rows,
 * the chosen ones checked. Selecting a row runs it and closes the menu (unless `keepOpen`); Back
 * closes it. Focus starts on `focusLabel`'s row, else the checked one, else the first one that can be chosen.
 */
@Composable
fun MenuDialog(
    title: String?,
    sections: List<MenuSection>,
    onDismiss: () -> Unit,
    message: String? = null,
    keepOpen: Boolean = false,
    width: androidx.compose.ui.unit.Dp = 360.dp,
    /** The row focus starts on (a submenu's row when coming back from it), else the checked one. */
    focusLabel: String? = null,
    /** Lines a row's detail may take (one keeps a long list of tracks on the screen). */
    detailLines: Int = 2,
) {
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        val rows = sections.flatMap { it.rows }
        val start = focusLabel?.let { l -> rows.indexOfFirst { it.label == l && it.enabled }.takeIf { it >= 0 } }
            ?: rows.indexOfFirst { it.checked && it.enabled }.takeIf { it >= 0 } ?: rows.indexOfFirst { it.enabled }
        val first = remember { FocusRequester() }
        Column(
            Modifier
                .width(width)
                .heightIn(max = 480.dp)
                .background(Color(0xF2202229), RoundedCornerShape(18.dp))
                .padding(vertical = 14.dp),
        ) {
            title?.let {
                Text(it, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, color = Color.White, textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 4.dp))
            }
            message?.let {
                Text(it, fontSize = 12.sp, lineHeight = 16.sp, color = Theme.secondaryText, textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 4.dp))
            }
            // Clipped: rows scrolled up go out of sight rather than under the title.
            LazyColumn(Modifier.clipToBounds(), contentPadding = PaddingValues(horizontal = 10.dp, vertical = 6.dp)) {
                var index = 0
                sections.forEachIndexed { s, section ->
                    if (s > 0) item { Spacer(Modifier.height(8.dp)) }
                    section.title?.let { t ->
                        item { Text(t, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, color = Theme.secondaryText, modifier = Modifier.padding(start = 12.dp, top = 4.dp, bottom = 4.dp)) }
                    }
                    section.rows.forEach { row ->
                        val i = index++
                        item {
                            MenuRowView(row, detailLines, if (i == start) Modifier.focusRequester(first) else Modifier) {
                                row.action()
                                if (!keepOpen) onDismiss()
                            }
                        }
                    }
                }
            }
        }
        LaunchedEffect(Unit) {
            repeat(10) {
                if (runCatching { first.requestFocus() }.isSuccess) return@LaunchedEffect
                delay(30)
            }
        }
    }
}

@Composable
private fun MenuRowView(row: MenuRow, detailLines: Int, modifier: Modifier, onClick: () -> Unit) {
    FocusSurface(
        onClick = onClick,
        enabled = row.enabled,
        modifier = modifier.fillMaxWidth().padding(vertical = 1.dp),
        shape = RoundedCornerShape(10.dp),
        focusedScale = 1.02f,
        containerColor = Color.Transparent, contentColor = Color.White,
            focusedContainerColor = Color.White, focusedContentColor = Color.Black,
            disabledContainerColor = Color.Transparent, disabledContentColor = Color.White.copy(alpha = 0.35f),
    ) {
        Row(Modifier.padding(horizontal = 12.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(row.label, fontSize = 14.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis,
                    color = if (row.destructive && row.enabled) Color(0xFFFF6B6B) else androidx.tv.material3.LocalContentColor.current)
                row.detail?.let {
                    Text(it, fontSize = 11.sp, lineHeight = 14.sp, maxLines = detailLines, overflow = TextOverflow.Ellipsis,
                        color = androidx.tv.material3.LocalContentColor.current.copy(alpha = 0.65f))
                }
            }
            Text(if (row.checked) "✓" else "", fontSize = 14.sp, fontWeight = FontWeight.Bold, modifier = Modifier.width(22.dp), textAlign = TextAlign.End)
        }
    }
}

/** A confirmation as tvOS asks it: a title, a message, the action and Cancel. */
@Composable
fun ConfirmDialog(title: String, message: String, action: String, onConfirm: () -> Unit, onDismiss: () -> Unit) {
    MenuDialog(
        title = title, message = message, onDismiss = onDismiss, width = 420.dp,
        sections = listOf(MenuSection(rows = listOf(
            MenuRow(action, destructive = true, action = onConfirm),
            MenuRow("Cancel", action = {}),
        ))),
    )
}

/**
 * A screen that couldn't load: the reason, Retry, and Sign In Again when the server no longer
 * takes this device's sign-in. Screens also retry by themselves when the network comes back.
 */
@Composable
fun ErrorPane(message: String, onRetry: () -> Unit, modifier: Modifier = Modifier, signIn: (() -> Unit)? = null) {
    val retry = remember { FocusRequester() }
    Column(modifier.fillMaxSize().padding(Theme.margin), verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
        Text("⚠", fontSize = 30.sp, color = Theme.secondaryText)
        Spacer(Modifier.height(10.dp))
        Text(message, color = Theme.secondaryText, textAlign = TextAlign.Center, fontSize = 15.sp, modifier = Modifier.widthIn(max = 520.dp))
        Spacer(Modifier.height(18.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            GlassButton(onClick = onRetry, modifier = Modifier.focusRequester(retry), name = "Retry") { Text("Retry") }
            if (signIn != null) GlassButton(onClick = signIn) { Text("Sign In Again") }
        }
    }
    LaunchedEffect(message) { delay(50); runCatching { retry.requestFocus() } }
}

/** A titled row of lockups (`TVShelf`): the title at the margin, the lockups scrolling sideways. */
@Composable
fun <T> Shelf(
    title: String,
    items: List<T>,
    key: (T) -> Any,
    modifier: Modifier = Modifier,
    state: androidx.compose.foundation.lazy.LazyListState = androidx.compose.foundation.lazy.rememberLazyListState(),
    rowModifier: Modifier = Modifier,
    /** A last control after the cards (Clear, say). */
    trailing: (@Composable () -> Unit)? = null,
    content: @Composable (index: Int, item: T) -> Unit,
) {
    Column(modifier) {
        Text(title, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, color = Theme.secondaryText,
            modifier = Modifier.padding(start = Theme.margin, bottom = 8.dp))
        LazyRow(
            state = state,
            modifier = rowModifier,
            contentPadding = PaddingValues(horizontal = Theme.margin, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(Theme.rowSpacing),
        ) {
            val last = if (trailing != null) items.size else items.lastIndex
            itemsIndexed(items, key = { _, it -> key(it) }) { i, it -> Box(Modifier.rowEdge(i, last)) { content(i, it) } }
            if (trailing != null) item(key = "trailing") { Box(Modifier.rowEdge(last, last)) { trailing() } }
        }
    }
}

/** A centred message (an empty list). */
@Composable
fun EmptyState(title: String, message: String, modifier: Modifier = Modifier) {
    Column(modifier.fillMaxWidth().padding(vertical = 40.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Text(title, fontSize = 17.sp, fontWeight = FontWeight.SemiBold, color = Color.White)
        Spacer(Modifier.height(6.dp))
        Text(message, fontSize = 13.sp, color = Theme.secondaryText, textAlign = TextAlign.Center)
    }
}

/**
 * A title's long-press menu (`WatchedMenuItems`): Mark as Watched / Unwatched, as its state allows,
 * and `extra` rows (an episode's Play from Beginning). The mark shows on every card at once.
 */
@Composable
fun WatchMenu(item: app.moonrise.core.Item, onDismiss: () -> Unit, extra: List<MenuRow> = emptyList(), done: () -> Unit = {}) {
    val scope = Session.ui
    val current = WatchMarks.current(item)
    val title = if (item.isEpisode) listOfNotNull(item.episodeLabel, item.title).joinToString(" · ") else item.title
    MenuDialog(
        title = title, onDismiss = onDismiss,
        sections = listOf(MenuSection(rows = app.moonrise.core.WatchAction.options(current).map { a ->
            MenuRow(a.title) { scope.launch { if (WatchMarks.set(item, a.played)) done() } }
        } + extra)),
    )
}

/**
 * Focus entering this list goes to `target` when it's on screen (`available`), else where the
 * focus search lands. Unlike `focusRestorer`, it doesn't pin a list item, which crashed when the
 * list's items changed under it.
 */
fun Modifier.enterOn(target: FocusRequester, available: () -> Boolean): Modifier =
    this.focusProperties { enter = { if (available()) target else FocusRequester.Default } }

/**
 * Left on a row's first card and right on its last stay there (as on Apple TV), instead of the
 * focus search jumping to whatever lies up and to the side (the tab bar, a search field).
 */
fun Modifier.rowEdge(index: Int, last: Int): Modifier = this.focusProperties {
    if (index == 0) left = FocusRequester.Cancel
    if (index == last) right = FocusRequester.Cancel
}

/**
 * Scrolls only as far as needed to show the focused control (Compose's TV default keeps it at a
 * pivot a third down the screen, which pushed a title page's showcase off the top).
 */
@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
val MinimalScroll = object : androidx.compose.foundation.gestures.BringIntoViewSpec {
    override fun calculateScrollDistance(offset: Float, size: Float, containerSize: Float): Float {
        val trailing = offset + size
        return when {
            offset >= 0 && trailing <= containerSize -> 0f
            offset < 0 && trailing > containerSize -> 0f
            kotlin.math.abs(offset) < kotlin.math.abs(trailing - containerSize) -> offset
            else -> trailing - containerSize
        }
    }
}

/**
 * Select on the remote: a press runs `onClick`, holding it runs `onLongClick` (the title's menu, as
 * a long press on Apple TV) and not the click. Handled here from the key's repeats, which every
 * remote sends while held.
 */
fun Modifier.selectOrHold(onClick: () -> Unit, onLongClick: (() -> Unit)?): Modifier =
    if (onLongClick == null) this else composed {
        var down by remember { mutableStateOf(false) }
        var held by remember { mutableStateOf(false) }
        onPreviewKeyEvent { e ->
            val k = e.nativeKeyEvent
            if (k.keyCode != KeyEvent.KEYCODE_DPAD_CENTER && k.keyCode != KeyEvent.KEYCODE_ENTER &&
                k.keyCode != KeyEvent.KEYCODE_NUMPAD_ENTER) return@onPreviewKeyEvent false
            when (k.action) {
                KeyEvent.ACTION_DOWN -> {
                    if (k.repeatCount == 0 && !k.isLongPress) { down = true; held = false }
                    else if (down && !held) { held = true; onLongClick() }
                    true
                }
                KeyEvent.ACTION_UP -> {
                    if (down && !held) onClick()
                    down = false
                    held = false
                    true
                }
                else -> false
            }
        }
    }

/**
 * Asks `requester` for focus over the next frames, until it's attached (a lazy list composes an
 * item a frame or two after it's scrolled to). `focused` tells when focus has really landed.
 */
suspend fun FocusRequester.focusSoon(frames: Int = 30, focused: (() -> Boolean)? = null): Boolean {
    repeat(frames) {
        androidx.compose.runtime.withFrameNanos { }
        if (runCatching { requestFocus() }.isSuccess) {
            if (focused == null) return true
            androidx.compose.runtime.withFrameNanos { }
            if (focused()) return true
        }
    }
    return false
}

/** Logs where focus lands (`adb logcat -s MoonriseFocus`), to check focus without a screenshot. */
fun Modifier.logFocus(name: () -> String): Modifier = composed {
    var had by remember { mutableStateOf(false) }
    onFocusChanged {
        if (it.isFocused && !had) Log.d("MoonriseFocus") { name() }
        else if (!it.isFocused && had) Log.d("MoonriseFocus") { "(left ${name()})" }
        had = it.isFocused
    }
}

/**
 * A clickable surface whose focused look (colours, scale, glow) follows the focus system's own
 * state (`onFocusChanged`). tv-material's Surface draws it from focus interactions, which a
 * control focused as it appears, or during quick key presses, could miss or keep.
 */
@Composable
fun FocusSurface(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    shape: androidx.compose.ui.graphics.Shape = RoundedCornerShape(8.dp),
    focusedScale: Float = 1f,
    focusedGlow: androidx.tv.material3.Glow = androidx.tv.material3.Glow.None,
    containerColor: Color = Color.Transparent,
    contentColor: Color = Color.White,
    focusedContainerColor: Color = containerColor,
    focusedContentColor: Color = contentColor,
    disabledContainerColor: Color = containerColor,
    disabledContentColor: Color = contentColor.copy(alpha = 0.4f),
    interactionSource: androidx.compose.foundation.interaction.MutableInteractionSource? = null,
    content: @Composable androidx.compose.foundation.layout.BoxScope.() -> Unit,
) {
    var focused by remember { mutableStateOf(false) }
    val container by androidx.compose.animation.animateColorAsState(if (focused) focusedContainerColor else containerColor, FocusMotion.spec(), label = "surface")
    val text by androidx.compose.animation.animateColorAsState(if (focused) focusedContentColor else contentColor, FocusMotion.spec(), label = "surface text")
    val scale by androidx.compose.animation.core.animateFloatAsState(if (focused) focusedScale else 1f, FocusMotion.spec(), label = "surface scale")
    val glow = if (focused) focusedGlow else androidx.tv.material3.Glow.None
    Surface(
        onClick = onClick,
        modifier = modifier.onFocusChanged { focused = it.isFocused },
        enabled = enabled,
        interactionSource = interactionSource,
        shape = ClickableSurfaceDefaults.shape(shape),
        scale = ClickableSurfaceDefaults.scale(scale = scale, focusedScale = scale, pressedScale = scale),
        glow = ClickableSurfaceDefaults.glow(glow = glow, focusedGlow = glow, pressedGlow = glow),
        colors = ClickableSurfaceDefaults.colors(
            containerColor = container, contentColor = text,
            focusedContainerColor = container, focusedContentColor = text,
            pressedContainerColor = container, pressedContentColor = text,
            disabledContainerColor = disabledContainerColor, disabledContentColor = disabledContentColor,
        ),
        content = content,
    )
}
