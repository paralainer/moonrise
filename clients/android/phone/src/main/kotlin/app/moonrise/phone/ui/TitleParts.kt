package app.moonrise.phone.ui

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
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Eco
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Star
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.SubcomposeLayout
import androidx.compose.ui.layout.layout
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.moonrise.core.Item
import app.moonrise.core.Person
import app.moonrise.core.TitleFacts
import app.moonrise.core.clockText
import app.moonrise.core.premiereText
import app.moonrise.core.runtimeText
import app.moonrise.core.phoneTechBadges
import app.moonrise.shared.WatchMarks

/**
 * The backdrop and logo at the top of a title page and of each page of Home's banner (`TitleHeader`):
 * drawn alike, so opening a title from the banner keeps the picture in place. No artwork: a short
 * band with the title.
 */
@Composable
fun TitleHeader(image: String?, logo: String?, title: String, height: Dp = LocalMetrics.current.headerHeight(Theme.headerHeight.value.toDouble()).dp) {
    val h = if (image == null) height * 0.45f else height
    val metrics = LocalMetrics.current
    val regular = metrics.regular
    val overlap = metrics.headerOverlap(image != null)
    Box(Modifier.fillMaxWidth().height(h), contentAlignment = Alignment.BottomCenter) {
        if (image != null) RemoteImage(image, Modifier.fillMaxSize())
        else Box(Modifier.fillMaxSize().background(Brush.verticalGradient(listOf(Color.White.copy(alpha = 0.08f), Color.Transparent))))
        Box(Modifier.fillMaxSize().background(Brush.verticalGradient(0.3f to Color.Transparent, 0.65f to Theme.background.copy(alpha = 0.6f), 1f to Theme.background)))
        // The last row exactly the page's colour, so the picture never shows as a line at the edge (a wide bitmap's filtering).
        if (regular && image != null) Box(Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(2.dp).background(Theme.background))
        if (regular) {
            // A tablet: a shade behind the title and Play on the picture, the logo at the margin.
            if (overlap > 0) Box(Modifier.fillMaxSize().background(Brush.horizontalGradient(0f to Theme.background.copy(alpha = 0.8f), 0.33f to Theme.background.copy(alpha = 0.35f), 0.65f to Color.Transparent)))
            val bottom = (6 + overlap + (if (overlap > 0) 14 else 0)).dp
            if (logo != null) RemoteImage(logo, Modifier.align(Alignment.BottomStart).padding(start = Theme.margin, bottom = bottom).widthIn(max = 380.dp).fillMaxWidth(0.6f).height(minOf(130.dp, h * 0.3f)),
                contentScale = ContentScale.Fit, alignment = Alignment.BottomStart, placeholder = false)
            else Text(title, style = Type.largeTitle, color = Color.White, modifier = Modifier.align(Alignment.BottomStart).padding(start = Theme.margin, end = Theme.margin, bottom = bottom))
        } else if (logo != null) {
            RemoteImage(logo, Modifier.padding(bottom = 6.dp).widthIn(max = 300.dp).heightIn(max = minOf(110.dp, h * 0.45f)).fillMaxWidth(0.75f).height(minOf(110.dp, h * 0.45f)),
                contentScale = ContentScale.Fit, alignment = Alignment.BottomCenter, placeholder = false)
        } else {
            Text(title, style = Type.largeTitle, color = Color.White, textAlign = TextAlign.Center,
                modifier = Modifier.padding(horizontal = Theme.margin).padding(bottom = 6.dp))
        }
    }
}

/** Runtime · date · age rating · picture and sound badges, then the genres (`TitleMeta`). `focus`: a show's episode. */
@Composable
fun TitleMeta(item: Item, focus: Item? = null, modifier: Modifier = Modifier, leading: Boolean = false) {
    val f = focus ?: item
    Column(modifier, horizontalAlignment = if (leading) Alignment.Start else Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        // The badges that fit on the line (as SwiftUI drops what doesn't fit in one line).
        FittingRow(spacing = 10.dp, leading = leading) {
            runtimeText(f.runtime)?.let { Text(it, style = Type.subheadline, color = Theme.secondaryText, maxLines = 1) }
            val date = premiereText(f.premiereDate) ?: item.year?.toString()
            date?.let { Text(it, style = Type.subheadline, color = Theme.secondaryText, maxLines = 1) }
            (item.officialRating ?: f.officialRating)?.let { Badge(it) }
            f.phoneTechBadges.take(3).forEach { Badge(it) }
        }
        val g = item.genres.orEmpty()
        if (g.isNotEmpty()) Text(g.take(3).joinToString(", "), style = Type.subheadline, color = Theme.secondaryText, textAlign = if (leading) TextAlign.Start else TextAlign.Center)
    }
}

/** Lays children in a row, centred or from the leading edge, leaving out those at the end that don't fit. */
@Composable
private fun FittingRow(spacing: Dp, leading: Boolean = false, content: @Composable () -> Unit) {
    SubcomposeLayout { c ->
        val gap = spacing.roundToPx()
        val ps = subcompose(Unit, content).map { it.measure(Constraints()) }
        val kept = mutableListOf<androidx.compose.ui.layout.Placeable>()
        var w = 0
        for (p in ps) {
            val next = w + (if (kept.isEmpty()) 0 else gap) + p.width
            if (next > c.maxWidth) break
            kept += p; w = next
        }
        val h = kept.maxOfOrNull { it.height } ?: 0
        layout(c.maxWidth, h) {
            var x = if (leading) 0 else (c.maxWidth - w) / 2
            for (p in kept) { p.place(x, (h - p.height) / 2); x += p.width + gap }
        }
    }
}

/** Play: a white capsule, filled as far as it was watched, with the resume time (`PlayCapsule`). */
@Composable
fun PlayCapsule(title: String, time: Double?, progress: Double?, modifier: Modifier = Modifier) {
    Box(modifier.height(52.dp).clip(CircleShape).background(Color.White.copy(alpha = if (progress == null) 1f else 0.55f))) {
        if (progress != null) Box(Modifier.fillMaxHeight().fillMaxWidth(progress.toFloat().coerceIn(0.08f, 1f)).clip(CircleShape).background(Color.White))
        Row(Modifier.fillMaxSize(), horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Filled.PlayArrow, null, tint = Color.Black, modifier = Modifier.size(26.dp))
            Spacer(Modifier.width(4.dp))
            Text(title, style = Type.headline, color = Color.Black)
            if (time != null) Text(" " + clockText(time), style = Type.headline.copy(fontWeight = FontWeight.Normal), color = Color.Black)
        }
    }
}

/** IMDb, TMDB's star, Rotten Tomatoes and Metacritic, the ones the server has (the iPhone's ratings row). */
@Composable
fun RatingsRow(item: Item, focus: Item?, isShow: Boolean, leading: Boolean = false) {
    val own = (focus ?: item).ratings
    val show = if (isShow) item.ratings else null
    val imdb = own?.imdb?.value ?: show?.imdb?.value
    val rt = (own?.rottenTomatoes?.value ?: show?.rottenTomatoes?.value)?.let { Math.round(it).toInt() }
    val mc = (own?.metacritic?.value ?: show?.metacritic?.value)?.let { Math.round(it).toInt() }
    val community = focus?.communityRating ?: item.communityRating
    if (imdb == null && rt == null && mc == null && community == null) return
    val style = Type.headline.copy(color = Color.White.copy(alpha = 0.9f))
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(if (listOfNotNull(imdb, rt, mc, community).size > 3) 12.dp else 20.dp, if (leading) Alignment.Start else Alignment.CenterHorizontally),
        verticalAlignment = Alignment.CenterVertically) {
        if (imdb != null) Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Text("IMDb", style = Type.caption, fontWeight = FontWeight.Black, color = Color.Black,
                modifier = Modifier.background(Color(0xFFF5C518), RoundedCornerShape(3.dp)).padding(horizontal = 4.dp, vertical = 1.dp))
            Text("%.1f".format(java.util.Locale.ROOT, imdb), style = style)
        }
        if (community != null) Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Icon(Icons.Filled.Star, "TMDB", tint = Color.White.copy(alpha = 0.9f), modifier = Modifier.size(20.dp))
            Text("%.1f".format(java.util.Locale.ROOT, community), style = style)
        }
        if (rt != null) Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Tomato(rt >= 60)
            Text("$rt%", style = style)
        }
        if (mc != null) Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            val bg = when { mc >= 61 -> Color(0xFF66CC33); mc >= 40 -> Color(0xFFFFCC33); else -> Color(0xFFFF0000) }
            Text("MC", style = Type.caption, fontWeight = FontWeight.Black, color = if (mc in 40..60) Color.Black else Color.White,
                modifier = Modifier.background(bg, RoundedCornerShape(3.dp)).padding(horizontal = 4.dp, vertical = 1.dp))
            Text("$mc", style = style)
        }
    }
}

/** Rotten Tomatoes: a red tomato when fresh (60%+), a green splat when rotten. */
@Composable
private fun Tomato(fresh: Boolean) {
    if (fresh) {
        Box(Modifier.size(18.dp)) {
            Box(Modifier.size(18.dp).clip(CircleShape).background(Color(0xFFFA331A)))
            Icon(Icons.Filled.Eco, null, tint = Color(0xFF339926), modifier = Modifier.size(9.dp).align(Alignment.TopCenter).offset(y = (-4).dp).rotate(-30f))
        }
    } else {
        Box(Modifier.size(18.dp).clip(RoundedCornerShape(5.dp)).background(Color(0xFF6BBA33)))
    }
}

/**
 * A 16:9 episode in the title page's row (`EpisodeCard`): the selected one has a white outline and
 * a play symbol (a tap plays it; on another, a tap selects it), the playing one says Now Playing.
 */
@Composable
fun EpisodeCard(episode: Item, width: Dp, selected: Boolean, nowPlaying: Boolean, onClick: () -> Unit, menu: CardMenu?, image: String? = null, details: Boolean = false) {
    val ep = WatchMarks.current(episode)
    var open by remember { mutableStateOf(false) }
    Column(Modifier.width(width).cardClick(onClick, if (menu != null) ({ open = true }) else null), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Box(Modifier.width(width).height(width * 9f / 16f)) {
            Box(Modifier.fillMaxSize().clip(RoundedCornerShape(16.dp))) {
                RemoteImage(image ?: imageUrl(ep.cardImage, width), Modifier.fillMaxSize())
                if (!nowPlaying) ep.progress?.let { ProgressBar(it, Modifier.align(Alignment.BottomCenter).padding(start = 10.dp, end = 10.dp, bottom = 8.dp)) }
                if (nowPlaying) {
                    Row(Modifier.align(Alignment.Center).clip(CircleShape).background(Theme.glass).padding(horizontal = 10.dp, vertical = 5.dp),
                        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        Icon(Icons.Filled.PlayArrow, null, tint = Color.White, modifier = Modifier.size(14.dp))
                        Text("Now Playing", style = Type.caption, fontWeight = FontWeight.SemiBold, color = Color.White)
                    }
                } else if (selected) {
                    Box(Modifier.align(Alignment.Center).size(50.dp).clip(CircleShape).background(Theme.glass), contentAlignment = Alignment.Center) {
                        Icon(Icons.Filled.PlayArrow, "Play", tint = Color.White, modifier = Modifier.size(30.dp))
                    }
                }
            }
            WatchedMark(ep, Modifier.align(Alignment.TopEnd))
            DownloadMark(ep)
            // The selected one's white outline, 5 dp outside the picture.
            if (selected) Box(Modifier.matchParentSize().drawBehind {
                val o = 5.dp.toPx() - 1.5.dp.toPx()
                drawRoundRect(Color.White, topLeft = androidx.compose.ui.geometry.Offset(-o, -o),
                    size = androidx.compose.ui.geometry.Size(size.width + 2 * o, size.height + 2 * o),
                    cornerRadius = androidx.compose.ui.geometry.CornerRadius(19.dp.toPx() - 1.5.dp.toPx()),
                    style = androidx.compose.ui.graphics.drawscope.Stroke(3.dp.toPx()))
            })
            if (menu != null) CardMenuPopup(open, { open = false }, ep, menu)
        }
        Text(ep.episodeListTitle, style = Type.subheadline, fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Normal,
            color = if (selected || details) Color.White else Theme.secondaryText, maxLines = 1, overflow = TextOverflow.Ellipsis)
        // A tablet: runtime, air date and the synopsis under the title.
        if (details) Column(Modifier.width(width), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(listOfNotNull(runtimeText(ep.runtime), premiereText(ep.premiereDate)).joinToString(" · "), style = Type.caption, fontWeight = FontWeight.Medium, color = Theme.secondaryText, maxLines = 1)
            Text(ep.overview.orEmpty(), style = Type.caption, color = Theme.secondaryText, minLines = 3, maxLines = 3, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** Someone in the cast or crew: their picture, name and role (`CastCard`). */
@Composable
fun CastCard(person: Person) {
    Column(Modifier.width(96.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Box(Modifier.size(84.dp).clip(RoundedCornerShape(20.dp)).background(Theme.surface), contentAlignment = Alignment.Center) {
            val url = imageUrl(person.image, 84.dp)
            if (url != null) RemoteImage(url, Modifier.fillMaxSize()) else Icon(Icons.Filled.Person, null, tint = Theme.secondaryText)
        }
        Text(person.name.orEmpty(), style = Type.caption, color = Color.White, maxLines = 2, textAlign = TextAlign.Center, overflow = TextOverflow.Ellipsis)
        Text(if (person.group == "Actor") person.role.orEmpty() else person.group, style = Type.caption2, color = Theme.secondaryText, maxLines = 2,
            textAlign = TextAlign.Center, overflow = TextOverflow.Ellipsis)
    }
}

/**
 * Pulls this content up by `by` without moving what follows (the tablet's title and Play over the
 * bottom of the picture): measured as usual, it takes that much less room and is placed higher.
 */
fun Modifier.pullUp(by: Dp): Modifier = layout { measurable, constraints ->
    val p = measurable.measure(constraints)
    val up = by.roundToPx()
    layout(p.width, maxOf(0, p.height - up)) { p.place(0, -up) }
}

/** Details, a tablet's last section (`TitleDetails`): the title's facts in columns. */
@Composable
fun TitleDetails(item: Item, focus: Item?, seasons: Int) {
    val facts = TitleFacts.of(item, focus, seasons)
    if (facts.isEmpty()) return
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        SectionHeader("Details")
        BoxWithConstraints(Modifier.fillMaxWidth().padding(horizontal = Theme.margin)) {
            val gap = 24.dp
            val n = maxOf(1, ((maxWidth + gap) / (230.dp + gap)).toInt())
            val cell = (maxWidth - gap * (n - 1)) / n
            Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
                for (row in facts.chunked(n)) Row(horizontalArrangement = Arrangement.spacedBy(gap)) {
                    for (f in row) Column(Modifier.width(cell), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                        Text(f.label, style = Type.subheadline, fontWeight = FontWeight.SemiBold, color = Color.White)
                        Text(f.value, style = Type.subheadline, color = Theme.secondaryText, maxLines = 3, overflow = TextOverflow.Ellipsis)
                    }
                }
            }
        }
    }
}
