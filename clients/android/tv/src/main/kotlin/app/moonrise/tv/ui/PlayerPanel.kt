package app.moonrise.tv.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.Text
import app.moonrise.core.Item
import app.moonrise.core.runtimeText
import app.moonrise.core.techBadges
import app.moonrise.shared.player.PlayerModel
import coil.compose.AsyncImage

/** A tab of the panel that comes down from the top (the Apple TV's swipe-down panel). */
internal enum class PanelTab(val title: String) { Next("Next Episode"), Info("Info"), Episodes("Episodes") }

internal fun panelTabs(model: PlayerModel): List<PanelTab> = buildList {
    if (model.nextEpisodeOffered && model.nextEpisode != null) add(PanelTab.Next)
    add(PanelTab.Info)
    if (model.seriesEpisodes.isNotEmpty()) add(PanelTab.Episodes)
}

/** Down near the end lands on Next Episode, else on Info. */
internal fun firstTab(model: PlayerModel): PanelTab = panelTabs(model).first()

/** The panel from the top: its tabs, then the chosen one (the Apple TV's swipe-down panel). */
@Composable
internal fun InfoPanel(model: PlayerModel, tabs: List<PanelTab>, tab: PanelTab, onContent: Boolean, episodeIndex: Int) {
    Column(Modifier.fillMaxWidth().padding(horizontal = Theme.margin, vertical = 18.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(start = 6.dp, bottom = 10.dp)) {
            tabs.forEach { t ->
                val selected = t == tab
                val f = selected && !onContent
                Text(t.title, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, color = if (f) Color.Black else Color.White.copy(alpha = if (selected) 1f else 0.7f),
                    modifier = Modifier.background(if (f) Color.White else if (selected) Color.White.copy(alpha = 0.2f) else Color.Transparent, RoundedCornerShape(50))
                        .padding(horizontal = 14.dp, vertical = 5.dp))
            }
        }
        Box(Modifier.fillMaxWidth().height(150.dp).clip(RoundedCornerShape(20.dp)).background(Color(0xE61C1E24))) {
            when (tab) {
                PanelTab.Info -> InfoTab(model)
                PanelTab.Episodes -> EpisodesTab(model, if (onContent) episodeIndex else -1)
                PanelTab.Next -> model.nextEpisode?.let { NextTab(it, onContent) }
            }
        }
    }
}

/**
 * Info: the artwork (a film's poster, an episode's still), the heading (an episode's show and
 * number, else the title), the overview, then genres, year, runtime, age rating and the stream's
 * quality.
 */
@Composable
private fun InfoTab(model: PlayerModel) {
    val item = model.item
    val details = model.titleDetails ?: item
    Row(Modifier.fillMaxSize().padding(14.dp), horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        val art = if (item.isEpisode) item.images?.thumb ?: item.backdropImage else item.posterImage ?: item.backdropImage
        val url = imageUrl(art, if (item.isEpisode) 216.dp else 82.dp)
        if (url != null) AsyncImage(url, contentDescription = null, contentScale = ContentScale.Crop,
            modifier = Modifier.width(if (item.isEpisode) 216.dp else 82.dp).fillMaxHeight().clip(RoundedCornerShape(8.dp)))
        Column(Modifier.weight(1f)) {
            val heading = if (item.isEpisode) listOfNotNull(item.seriesTitle, item.episodeLabel).joinToString(" · ") else item.title.orEmpty()
            Text(heading, fontSize = 16.sp, fontWeight = FontWeight.Bold, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (item.isEpisode) Text(item.title.orEmpty(), fontSize = 13.sp, fontWeight = FontWeight.SemiBold, color = Color.White.copy(alpha = 0.85f), maxLines = 1)
            item.overview?.let {
                Text(it, fontSize = 12.sp, lineHeight = 16.sp, color = Color.White.copy(alpha = 0.8f), maxLines = if (item.isEpisode) 3 else 4,
                    overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 4.dp))
            }
            Spacer(Modifier.weight(1f))
            val year = item.premiereDate?.take(4) ?: item.year?.toString()
            val line = listOfNotNull(details.genres?.take(2)?.joinToString(", ")?.ifEmpty { null }, year, runtimeText(item.runtime ?: model.duration.takeIf { it > 0 }))
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(line.joinToString(" · "), fontSize = 11.sp, color = Theme.secondaryText, maxLines = 1)
                details.officialRating?.let { Badge(it) }
                item.techBadges.take(4).forEach { Badge(it) }
            }
            Text(listOfNotNull(model.videoLine, model.playMethod.ifEmpty { null }).joinToString(" · "), fontSize = 10.sp,
                color = Theme.secondaryText.copy(alpha = 0.8f), maxLines = 1, modifier = Modifier.padding(top = 3.dp))
        }
    }
}

/** Episodes: the show's episodes, opening on the one playing (select plays one). */
@Composable
private fun EpisodesTab(model: PlayerModel, focusIndex: Int) {
    val episodes = model.seriesEpisodes
    val state = rememberLazyListState()
    val playing = episodes.indexOfFirst { it.id == model.item.id }.coerceAtLeast(0)
    // Opens on the episode playing; moves along with the focus.
    LaunchedEffect(episodes.size) { runCatching { state.scrollToItem(maxOf(0, playing - 1)) } }
    LaunchedEffect(focusIndex) { if (focusIndex >= 0) runCatching { state.animateScrollToItem(maxOf(0, focusIndex - 1)) } }
    LazyRow(state = state, contentPadding = PaddingValues(12.dp), horizontalArrangement = Arrangement.spacedBy(14.dp)) {
        itemsIndexed(episodes, key = { _, e -> e.id }) { i, ep ->
            val f = i == focusIndex
            Column(Modifier.width(152.dp)) {
                Box(Modifier.width(152.dp).height(86.dp).scale(if (f) 1.06f else 1f).clip(RoundedCornerShape(6.dp))
                    .border(if (f) 2.dp else 0.dp, if (f) Color.White else Color.Transparent, RoundedCornerShape(6.dp)).background(Color.White.copy(alpha = 0.08f))) {
                    imageUrl(ep.images?.thumb ?: ep.cardImage, 152.dp)?.let { AsyncImage(it, null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize()) }
                    ep.progress?.let { ProgressBar(it.toFloat(), Modifier.align(Alignment.BottomCenter).padding(5.dp)) }
                    if (ep.user?.played == true) Text("✓", fontSize = 9.sp, color = Color.Black, fontWeight = FontWeight.Bold,
                        modifier = Modifier.align(Alignment.TopEnd).padding(4.dp).background(Color.White, CircleShape).padding(horizontal = 4.dp))
                }
                Text("${ep.episodeLabel ?: ""} · ${ep.title ?: ""}", fontSize = 11.sp, fontWeight = FontWeight.Medium, color = Color.White, maxLines = 1,
                    overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 4.dp))
                Text(if (ep.id == model.item.id) "Now Playing" else runtimeText(ep.runtime) ?: " ", fontSize = 10.sp, color = Theme.secondaryText, maxLines = 1)
            }
        }
    }
}

/** Next Episode: its still (select plays it), title, length and overview. */
@Composable
private fun NextTab(ep: Item, focused: Boolean) {
    Row(Modifier.fillMaxSize().padding(14.dp), horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        Box(Modifier.width(216.dp).fillMaxHeight().scale(if (focused) 1.04f else 1f).clip(RoundedCornerShape(8.dp))
            .border(if (focused) 2.dp else 0.dp, if (focused) Color.White else Color.Transparent, RoundedCornerShape(8.dp)).background(Color.White.copy(alpha = 0.08f)),
            contentAlignment = Alignment.Center) {
            imageUrl(ep.images?.thumb ?: ep.cardImage, 216.dp)?.let { AsyncImage(it, null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize()) }
            Text("▶", fontSize = 18.sp, color = Color.White, modifier = Modifier.background(Color.Black.copy(alpha = 0.4f), CircleShape).padding(12.dp))
        }
        Column(Modifier.weight(1f)) {
            Text(listOfNotNull(ep.episodeLabel, runtimeText(ep.runtime)).joinToString(" · "), fontSize = 11.sp, color = Theme.secondaryText)
            Text(ep.title.orEmpty(), fontSize = 15.sp, fontWeight = FontWeight.Bold, color = Color.White, maxLines = 1)
            ep.overview?.let {
                Text(it, fontSize = 12.sp, lineHeight = 16.sp, color = Color.White.copy(alpha = 0.8f), maxLines = 4, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 4.dp))
            }
        }
    }
}
