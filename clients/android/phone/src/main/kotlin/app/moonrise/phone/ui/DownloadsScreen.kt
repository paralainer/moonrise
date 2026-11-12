package app.moonrise.phone.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.outlined.ArrowCircleDown
import androidx.compose.material.icons.outlined.Movie
import androidx.compose.material.icons.outlined.PlayCircle
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.moonrise.core.DownloadRecord
import app.moonrise.core.LayoutMetrics
import app.moonrise.core.DownloadRules
import app.moonrise.core.DownloadState
import app.moonrise.core.bytesText
import app.moonrise.phone.Autoplay
import app.moonrise.phone.LocalNav
import app.moonrise.phone.R
import app.moonrise.phone.Route
import app.moonrise.shared.downloads.Downloads

/**
 * The Downloads tab (`DownloadsView`, iPhone): In Progress, then the films (Ready to Watch, or
 * Movies when there are episodes too), then each show's episodes in airing order. A row opens the
 * title's page, playing it; a swipe or a long press deletes it.
 */
@Composable
fun DownloadsScreen() {
    val groups = DownloadRules.groups(Downloads.records)
    val regular = LocalMetrics.current.regular
    Box(Modifier.fillMaxSize().then(if (regular) Modifier.background(Theme.backgroundGradient) else Modifier.background(Color.Black))) {
        LazyColumn(Modifier.fillMaxSize(), contentPadding = androidx.compose.foundation.layout.PaddingValues(bottom = Theme.pageBottom), verticalArrangement = Arrangement.spacedBy(if (regular) 26.dp else 22.dp)) {
            item("title") {
                Text("Downloads", style = Type.largeTitle, color = Color.White, modifier = Modifier.statusBarsPadding().padding(start = Theme.margin, top = if (regular) Theme.underTopBar + 8.dp else 52.dp))
            }
            if (Downloads.records.isEmpty()) {
                item("empty") { EmptyDownloads() }
            }
            section("In Progress", groups.inProgress, regular)
            section(groups.moviesTitle, groups.movies, regular)
            for (show in groups.shows) section(show.name, show.episodes, regular)
        }
    }
}

private val GROUP = Color(0xFF1F2023)

private fun androidx.compose.foundation.lazy.LazyListScope.section(title: String, records: List<DownloadRecord>, regular: Boolean) {
    if (records.isEmpty()) return
    if (regular) { item(title) { PadSection(title, records) }; return }
    item(title) {
        Column(Modifier.fillMaxWidth().padding(horizontal = Theme.margin), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(title, style = Type.headline, color = Theme.secondaryText, modifier = Modifier.padding(start = 16.dp))
            Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(26.dp)).background(GROUP)) {
                records.forEachIndexed { i, r ->
                    key(r.id) {
                        if (i > 0) Box(Modifier.padding(start = 82.dp).fillMaxWidth().height(0.5.dp).background(Color.White.copy(alpha = 0.12f)))
                        Row_(r)
                    }
                }
            }
        }
    }
}

/** A tablet: the section's title at the page margin, its downloads as a grid of cards (`padSection`). */
@Composable
private fun PadSection(title: String, records: List<DownloadRecord>) {
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        SectionHeader(title)
        BoxWithConstraints(Modifier.fillMaxWidth().padding(horizontal = Theme.margin)) {
            val gap = LayoutMetrics.ROW_SPACING
            val n = maxOf(1, ((maxWidth.value + gap) / (LayoutMetrics.DOWNLOAD_CARD_MIN + gap)).toInt())
            val card = (maxWidth - gap.dp * (n - 1)) / n
            Column(verticalArrangement = Arrangement.spacedBy(gap.dp)) {
                for (row in records.chunked(n)) Row(horizontalArrangement = Arrangement.spacedBy(gap.dp)) {
                    for (r in row) Box(Modifier.width(card).clip(RoundedCornerShape(16.dp)).background(Theme.surface).padding(12.dp)) { key(r.id) { Row_(r, card = true) } }
                }
            }
        }
    }
}

@Composable
private fun EmptyDownloads() {
    Column(Modifier.fillMaxWidth().padding(horizontal = 30.dp, vertical = 10.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        ArtworkImage(R.drawable.empty_downloads, if (LocalMetrics.current.regular) Modifier.width(520.dp).height(300.dp) else Modifier.fillMaxWidth().height(300.dp))
        Text("No Downloads", style = Type.title3, fontWeight = FontWeight.SemiBold, color = Color.White, textAlign = TextAlign.Center)
        Text("Download movies and episodes to watch offline. Choose Original or a transcoded quality to save space.", style = Type.subheadline, color = Theme.secondaryText, textAlign = TextAlign.Center)
    }
}

/** A download's row; `card`: a tablet's card, which has the same long press and no swipe. */
@Composable
private fun Row_(r: DownloadRecord, card: Boolean = false) {
    val nav = LocalNav.current
    var menu by remember { mutableStateOf(false) }
    val row: @Composable () -> Unit = {
        Row(Modifier.fillMaxWidth().cardClick({
            // The title's page, playing it (in progress ones play as far as they are downloaded).
            if (r.isPlayable) nav.stack.add(Route.Title(r.pageItem, Autoplay()))
        }, { menu = true }).then(if (card) Modifier else Modifier.padding(12.dp)), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(Modifier.width(54.dp).height(80.dp).clip(RoundedCornerShape(6.dp)).background(Theme.surface), contentAlignment = Alignment.Center) {
                val image = r.posterImage
                if (image != null) RemoteImage(image, Modifier.fillMaxSize()) else Icon(Icons.Outlined.Movie, null, tint = Theme.secondaryText)
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(r.title, style = Type.headline, color = Color.White, maxLines = 2, overflow = TextOverflow.Ellipsis)
                r.subtitle?.let { Text(it, style = Type.caption, color = Theme.secondaryText) }
                StatusLine(r)
            }
            Controls(r)
        }
        DropdownMenu(menu, { menu = false }, containerColor = Theme.sheet) {
            MenuItem("Delete", Icons.Filled.Delete, destructive = true) { menu = false; Downloads.delete(r.id) }
        }
    }
    if (card) { Box { row() }; return }
    val dismiss = rememberSwipeToDismissBoxState(confirmValueChange = { v ->
        if (v == SwipeToDismissBoxValue.EndToStart) { Downloads.delete(r.id); true } else false
    })
    SwipeToDismissBox(dismiss, enableDismissFromStartToEnd = false, backgroundContent = {
        if (dismiss.dismissDirection == SwipeToDismissBoxValue.EndToStart) Box(Modifier.fillMaxSize().background(Theme.destructive).padding(horizontal = 20.dp), contentAlignment = Alignment.CenterEnd) {
            Icon(Icons.Filled.Delete, "Delete", tint = Color.White)
        }
    }) {
        Box(Modifier.background(GROUP)) { row() }
    }
}

@Composable
private fun StatusLine(r: DownloadRecord) {
    when (r.state) {
        DownloadState.Completed -> Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Badge(r.qualityLabel)
            if (r.bytesWritten > 0) Text(bytesText(r.bytesWritten), style = Type.caption, color = Theme.secondaryText)
            if (r.subtitles.isNotEmpty()) Badge("CC")
        }
        DownloadState.Failed -> Text(r.error ?: "Failed", style = Type.caption, color = Theme.destructive, maxLines = 2, overflow = TextOverflow.Ellipsis)
        else -> Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            LinearProgressIndicator(progress = { r.progress.toFloat() }, modifier = Modifier.fillMaxWidth(), color = Theme.accent, trackColor = Color.White.copy(alpha = 0.2f), drawStopIndicator = {})
            Text(r.rowDetail, style = Type.caption, color = Theme.secondaryText)
        }
    }
}

@Composable
private fun Controls(r: DownloadRecord) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        when (r.state) {
            DownloadState.Downloading, DownloadState.Queued -> RowButton(Icons.Filled.Pause, "Pause") { Downloads.pause(r.id) }
            DownloadState.Paused, DownloadState.Failed -> RowButton(Icons.Filled.Refresh, "Resume") { Downloads.resume(r.id) }
            DownloadState.Completed -> Box(Modifier.size(28.dp).background(Color.White, CircleShape), contentAlignment = Alignment.Center) {
                Icon(Icons.Filled.PlayArrow, "Play", tint = Color.Black, modifier = Modifier.size(20.dp))
            }
        }
        if (r.state != DownloadState.Completed && r.isPlayable) Icon(Icons.Outlined.PlayCircle, "Play", tint = Color.White, modifier = Modifier.size(28.dp))
    }
}

@Composable
private fun RowButton(icon: androidx.compose.ui.graphics.vector.ImageVector, description: String, onClick: () -> Unit) {
    Box(Modifier.size(28.dp).clip(CircleShape).background(Color.White).clickable(onClick = onClick), contentAlignment = Alignment.Center) {
        Icon(icon, description, tint = Color.Black, modifier = Modifier.size(20.dp))
    }
}
