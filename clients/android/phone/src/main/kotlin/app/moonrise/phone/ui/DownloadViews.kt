package app.moonrise.phone.ui

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.content.pm.PackageManager
import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowCircleDown
import androidx.compose.material.icons.filled.ArrowDownward
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Error
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PauseCircle
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.PriorityHigh
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Replay
import androidx.compose.material.icons.filled.Schedule
import androidx.compose.material.icons.filled.WifiOff
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Movie
import androidx.compose.material.icons.outlined.Tv
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.HorizontalDivider
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
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.moonrise.core.DownloadBadge
import app.moonrise.core.DownloadRecord
import app.moonrise.core.DownloadRules
import app.moonrise.core.DownloadState
import app.moonrise.core.Item
import app.moonrise.core.MediaFile
import app.moonrise.core.Quality
import app.moonrise.phone.Autoplay
import app.moonrise.phone.LocalNav
import app.moonrise.phone.Route
import app.moonrise.phone.Tab
import app.moonrise.shared.AppSettings
import app.moonrise.shared.Session
import app.moonrise.shared.downloads.Downloads
import app.moonrise.shared.downloads.ProgressStore
import app.moonrise.shared.downloads.ServerReach
import kotlinx.coroutines.launch

/** A file a download saved, as an image address. */
fun localImage(id: String, file: String?): String? = file?.let { "file://" + Downloads.file(id, it).path }

/** Wide artwork saved with it (an episode's still, a film's backdrop), else its poster. */
val DownloadRecord.pageArtwork: String? get() = localImage(id, backdropFile ?: posterFile)
val DownloadRecord.posterImage: String? get() = localImage(id, posterFile)

/** The downloaded mark on a card (`DownloadMark`): a filled arrow, a ring while it downloads; shows and seasons count their episodes. */
@Composable
fun DownloadMark(item: Item, modifier: Modifier = Modifier) {
    val size = 18.dp
    if (item.isSeries || item.isSeason) {
        val n = (if (item.isSeries) Downloads.seriesCounts[item.id] else Downloads.seasonCounts[item.id]) ?: 0
        if (n > 0) Row(modifier.padding(5.dp).background(Color.Black.copy(alpha = 0.6f), CircleShape).padding(horizontal = 6.dp, vertical = 3.dp),
            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(3.dp)) {
            Icon(Icons.Filled.ArrowCircleDown, null, tint = Color.White, modifier = Modifier.size(size * 0.7f))
            Text("$n", style = Type.caption2, fontWeight = FontWeight.Bold, color = Color.White)
        }
    } else {
        Downloads.badge(item.id)?.let { DownloadStateIcon(it, size, modifier.padding(5.dp)) }
    }
}

/** One download's state as a small round icon (`DownloadStateIcon`). */
@Composable
fun DownloadStateIcon(badge: DownloadBadge, size: Dp = 18.dp, modifier: Modifier = Modifier) {
    when (badge) {
        DownloadBadge.Downloaded -> Box(modifier.size(size).background(Color.White, CircleShape), contentAlignment = Alignment.Center) {
            Icon(Icons.Filled.ArrowDownward, "Downloaded", tint = Color.Black, modifier = Modifier.size(size * 0.7f))
        }
        is DownloadBadge.Downloading -> Box(modifier.size(size).background(Color.Black.copy(alpha = 0.6f), CircleShape).drawBehind {
            val w = size.toPx() * 0.14f
            drawArc(Color.White.copy(alpha = 0.25f), 0f, 360f, false, topLeft = Offset(w / 2, w / 2), size = Size(this.size.width - w, this.size.height - w), style = Stroke(w))
            drawArc(Color.White, -90f, 360f * maxOf(badge.percent, 3) / 100f, false, topLeft = Offset(w / 2, w / 2), size = Size(this.size.width - w, this.size.height - w), style = Stroke(w, cap = StrokeCap.Round))
        }, contentAlignment = Alignment.Center) {
            Icon(Icons.Filled.ArrowDownward, "Downloading, ${badge.percent} percent", tint = Color.White, modifier = Modifier.size(size * 0.55f))
        }
        DownloadBadge.Queued -> Icon(Icons.Filled.Schedule, "Download waiting", tint = Color.White.copy(alpha = 0.8f), modifier = modifier.size(size).background(Color.Black, CircleShape))
        DownloadBadge.Paused -> Icon(Icons.Filled.PauseCircle, "Download paused", tint = Color.White.copy(alpha = 0.8f), modifier = modifier.size(size).background(Color.Black, CircleShape))
        DownloadBadge.Failed -> Icon(Icons.Filled.Error, "Download failed", tint = Theme.destructive, modifier = modifier.size(size).background(Color.White, CircleShape))
    }
}

/** "Downloaded · 480p" under a title page's Play row, at the text's left edge (`DownloadStatusLine`). */
@Composable
fun DownloadStatusLine(item: Item, modifier: Modifier = Modifier) {
    val r = Downloads.forItem(item.id) ?: return
    Row(modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        Icon(if (r.state == DownloadState.Failed) Icons.Outlined.ErrorOutline else Icons.Filled.ArrowCircleDown, null, tint = Theme.secondaryText, modifier = Modifier.size(15.dp))
        Text(r.statusLine, style = Type.footnote, color = Theme.secondaryText, maxLines = 2, overflow = TextOverflow.Ellipsis)
    }
}

/**
 * Round Download button (`DownloadButton`): a tap downloads at the default quality, a long press
 * offers the qualities (with a title of several versions: which one first); once queued, its menu
 * pauses, resumes or deletes it. `size`: the page's Play row (52) or the player's buttons.
 */
@Composable
fun DownloadButton(item: Item, modifier: Modifier = Modifier, size: Dp = 52.dp, iconSize: Dp = 22.dp, ringInset: Float = 0.12f, onMenu: (Boolean) -> Unit = {}) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var open by remember { mutableStateOf(false) }
    var version by remember { mutableStateOf<MediaFile?>(null) }
    var full by remember(item.id) { mutableStateOf(item.takeIf { it.files != null }) }
    val rec = Downloads.forItem(item.id)
    fun close() { open = false; version = null; onMenu(false) }
    fun show() { open = true; onMenu(true) }
    fun startDownload(q: Quality, v: MediaFile?) {
        close()
        askNotifications(context)
        scope.launch {
            val f = full ?: runCatching { Session.api?.item(item.id) }.getOrNull()?.also { full = it } ?: item
            Downloads.download(f, q, v ?: f.presentFiles.firstOrNull())
        }
    }
    Box(modifier.size(size).combinedClickable(
        onClick = {
            if (rec != null) show()
            else scope.launch {
                val f = full ?: runCatching { Session.api?.item(item.id) }.getOrNull()?.also { full = it } ?: item
                if (f.presentFiles.size > 1) show()
                else startDownload(Quality.of(AppSettings.downloadQuality).capped(f.presentFiles.firstOrNull()?.bitrate, f.presentFiles.firstOrNull()?.videoHeight), null)
            }
        },
        onLongClick = { show() },
    ), contentAlignment = Alignment.Center) {
        when (rec?.state) {
            DownloadState.Completed -> Icon(Icons.Filled.Check, "Download options", tint = Color.White, modifier = Modifier.size(iconSize))
            DownloadState.Failed -> Icon(Icons.Filled.PriorityHigh, "Download options", tint = Theme.destructive, modifier = Modifier.size(iconSize))
            null -> Icon(Icons.Filled.ArrowDownward, "Download", tint = Color.White, modifier = Modifier.size(iconSize))
            else -> {
                val ring = size * ringInset
                val paused = rec.state == DownloadState.Paused
                val p = rec.progress.toFloat().coerceAtLeast(0.001f)
                Box(Modifier.fillMaxSize().padding(ring).drawBehind {
                    drawArc(Color.White.copy(alpha = 0.22f), 0f, 360f, false, style = Stroke(3.dp.toPx()))
                    drawArc(Color.White.copy(alpha = if (paused) 0.6f else 1f), -90f, 360f * p, false, style = Stroke(3.dp.toPx(), cap = StrokeCap.Round))
                })
                Icon(if (paused) Icons.Filled.Pause else Icons.Filled.ArrowDownward, "Download options", tint = Color.White, modifier = Modifier.size(iconSize * 0.7f))
            }
        }
        DropdownMenu(open, { close() }, containerColor = Theme.sheet) {
            if (rec != null) {
                MenuHeader("${if (rec.state == DownloadState.Completed) "Downloaded" else "Download"} · ${rec.downloadLabel}")
                when (rec.state) {
                    DownloadState.Downloading, DownloadState.Queued -> MenuItem("Pause Download", Icons.Filled.Pause) { close(); Downloads.pause(rec.id) }
                    DownloadState.Paused, DownloadState.Failed -> MenuItem("Resume Download", Icons.Filled.Replay) { close(); Downloads.resume(rec.id) }
                    DownloadState.Completed -> {}
                }
                MenuItem("Delete Download", Icons.Filled.Delete, destructive = true) { close(); Downloads.delete(rec.id) }
            } else {
                MenuHeader("Download")
                val f = full ?: item
                val versions = f.presentFiles
                val v = version ?: versions.singleOrNull()
                if (v == null && versions.size > 1) {
                    for (x in versions) MenuItem(x.versionTitle, detail = x.versionDetails, enabled = !x.isBroken) { version = x }
                } else {
                    for (q in Quality.options(v?.bitrate, v?.videoHeight)) {
                        MenuItem(if (q == Quality.original) "Original" else q.label, detail = q.sizeLabel(v?.duration ?: f.runtime, v?.size)) { startDownload(q, v) }
                    }
                }
            }
        }
    }
}

@Composable
private fun MenuHeader(text: String) {
    Text(text, style = Type.footnote, color = Theme.secondaryText, modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp))
}

/** Android 13+: the progress notification needs the permission; asked once, with the first download. */
private fun askNotifications(context: Context) {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
    if (context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) return
    tailrec fun Context.activity(): Activity? = when (this) { is Activity -> this; is ContextWrapper -> baseContext.activity(); else -> null }
    context.activity()?.requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 41)
}

/** A server can't be reached and there are downloads: say so, and show them (`OfflineBanner`). */
@Composable
fun OfflineBanner(retry: () -> Unit, modifier: Modifier = Modifier) {
    var retrying by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    Row(modifier.fillMaxWidth().padding(horizontal = Theme.margin).clip(CircleShape).background(Theme.glass).padding(horizontal = 18.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Icon(Icons.Filled.WifiOff, null, tint = Color.White, modifier = Modifier.size(20.dp))
        Text("Offline: showing downloads", style = Type.subheadline, fontWeight = FontWeight.SemiBold, color = Color.White, modifier = Modifier.weight(1f))
        if (retrying) CircularProgressIndicator(color = Color.White, strokeWidth = 2.dp, modifier = Modifier.size(16.dp))
        else Text("Try Again", style = Type.subheadline, color = Color.White, modifier = Modifier.clickable {
            retrying = true
            scope.launch { ServerReach.refresh(); retry(); retrying = false }
        })
    }
}

/** A download's card from what it saved (`DownloadCard`). */
@Composable
fun DownloadCard(record: DownloadRecord, landscape: Boolean, onClick: () -> Unit, menu: (@Composable (dismiss: () -> Unit) -> Unit)? = null) {
    val width = if (landscape) Theme.landscapeWidth else Theme.posterWidth
    val height = if (landscape) width * 9f / 16f else width * 1.5f
    val radius = if (landscape) 16.dp else 12.dp
    var open by remember { mutableStateOf(false) }
    Column(Modifier.width(width).cardClick(onClick, if (menu != null) ({ open = true }) else null), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Box {
            Box(Modifier.width(width).height(height).shadow(6.dp, RoundedCornerShape(radius)).clip(RoundedCornerShape(radius))) {
                val image = if (landscape) record.pageArtwork else record.posterImage
                if (image != null) RemoteImage(image, Modifier.fillMaxSize()) else Box(Modifier.fillMaxSize().background(Theme.surface), contentAlignment = Alignment.Center) {
                    Icon(Icons.Outlined.Movie, null, tint = Theme.secondaryText, modifier = Modifier.size(28.dp))
                }
                val p = ProgressStore.position(record.itemId)
                val d = record.runtime
                if (p != null && d != null && d > 0) ProgressBar(p / d, Modifier.align(Alignment.BottomCenter).padding(6.dp))
            }
            DownloadStateIcon(if (record.state == DownloadState.Completed) DownloadBadge.Downloaded else DownloadBadge.Downloading((record.progress * 100).toInt()), modifier = Modifier.padding(5.dp))
            if (menu != null) DropdownMenu(open, { open = false }, containerColor = Theme.sheet) { menu { open = false } }
        }
        Text(if (landscape) record.item?.seriesTitle ?: record.title else record.title, style = Type.caption, fontWeight = FontWeight.Medium, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text(if (landscape) listOfNotNull(record.item?.episodeLabel, record.item?.title).joinToString(" · ") else record.year?.toString() ?: " ",
            style = Type.caption2, color = Theme.secondaryText, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** Downloads as shelves (Home while offline): movies, then each show's episodes (`OfflineDownloadShelves`). */
@Composable
fun OfflineDownloadShelves() {
    val nav = LocalNav.current
    val playable = Downloads.records.filter { it.isPlayable }
    val movies = playable.filter { it.item?.isEpisode != true }
    val shows = playable.filter { it.item?.isEpisode == true }.groupBy { it.seriesId ?: "" }.values.sortedBy { it.first().item?.seriesTitle ?: "" }
    Column(verticalArrangement = Arrangement.spacedBy(28.dp)) {
        fun open(r: DownloadRecord, start: Double? = null) { nav.tab = Tab.Downloads; nav.stack.add(Route.Title(r.pageItem, Autoplay(start))) }
        @Composable fun card(r: DownloadRecord, landscape: Boolean) {
            DownloadCard(r, landscape, onClick = { open(r) }, menu = { dismiss ->
                MenuItem("Play from Beginning", Icons.Filled.Refresh) { dismiss(); open(r, 0.0) }
                MenuItem("Delete Download", Icons.Filled.Delete, destructive = true) { dismiss(); Downloads.delete(r.id) }
            })
        }
        if (movies.isNotEmpty()) Shelf("Downloaded Movies") { items(movies, key = { it.id }) { card(it, false) } }
        for (episodes in shows) {
            val sorted = DownloadRules.inOrder(episodes)
            Shelf(sorted.first().item?.seriesTitle ?: "Episodes") { items(sorted, key = { it.id }) { card(it, true) } }
        }
    }
}

/** A downloaded title's poster card, the artwork saved with the download (`OfflineTitleCard`). */
@Composable
fun OfflineTitleCard(item: Item, width: Dp, onClick: () -> Unit) {
    val recs = if (item.isSeries) Downloads.episodes(item.id).filter { it.isPlayable } else listOfNotNull(Downloads.playable(item.id))
    val poster = recs.firstNotNullOfOrNull { it.posterImage }
    Column(Modifier.width(width).cardClick(onClick, null), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Box {
            Box(Modifier.width(width).height(width * 1.5f).shadow(6.dp, RoundedCornerShape(12.dp)).clip(RoundedCornerShape(12.dp))) {
                if (poster != null) RemoteImage(poster, Modifier.fillMaxSize())
                else Box(Modifier.fillMaxSize().background(Theme.surface), contentAlignment = Alignment.Center) {
                    Icon(if (item.isSeries) Icons.Outlined.Tv else Icons.Outlined.Movie, null, tint = Theme.secondaryText, modifier = Modifier.size(28.dp))
                }
            }
            DownloadMark(item)
            WatchedMark(item, Modifier.align(Alignment.TopEnd))
        }
        Text(item.title.orEmpty(), style = Type.caption, fontWeight = FontWeight.Medium, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text(if (item.isSeries) (if (recs.size == 1) "1 episode" else "${recs.size} episodes") else item.year?.toString() ?: " ",
            style = Type.caption2, color = Theme.secondaryText, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}
