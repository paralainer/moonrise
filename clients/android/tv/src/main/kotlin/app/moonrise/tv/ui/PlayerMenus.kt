package app.moonrise.tv.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.tv.material3.Text
import app.moonrise.core.Languages
import app.moonrise.core.MoonriseHttpException
import app.moonrise.core.SubtitleQuota
import app.moonrise.core.SubtitleResult
import app.moonrise.shared.player.PlayerModel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import app.moonrise.shared.Log
import app.moonrise.shared.AppSettings

/** The player's menus. */
internal sealed interface PlayerMenu {
    data object Audio : PlayerMenu
    data object Subtitles : PlayerMenu
    data object Style : PlayerMenu
    data object Delay : PlayerMenu
    data object Quality : PlayerMenu
    data object Search : PlayerMenu
}

/** The Audio, Subtitles (with Style, Delay and Search Online) and Quality menus. */
@Composable
internal fun PlayerMenus(model: PlayerModel, menu: PlayerMenu?, open: (PlayerMenu) -> Unit, close: () -> Unit) {
    // Back from Style or Delay: focus goes back to that row, not to the checked track.
    var returnRow by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(menu) { if (menu == null) returnRow = null }
    fun submenu(row: String, m: PlayerMenu) { returnRow = row; open(m) }
    when (menu) {
        null -> {}
        PlayerMenu.Audio -> MenuDialog("Audio", onDismiss = close, width = 380.dp, detailLines = 1, sections = listOf(MenuSection(rows =
            model.audioOptions.map { a -> MenuRow(a.title, a.detail, checked = a.id == model.selectedAudio) { model.chooseAudio(a.id) } }
                .ifEmpty { listOf(MenuRow("No other audio", enabled = false) {}) })))
        PlayerMenu.Subtitles -> {
            model.timingRevision
            MenuDialog("Subtitles", onDismiss = close, width = 400.dp, sections = listOf(
                MenuSection(rows = listOf(MenuRow("Off", checked = model.selectedSubtitleId == null) { model.chooseSubtitle(null); close() }) +
                    model.subtitleOptions.map { o -> MenuRow(o.name, model.subtitleDetail(o), checked = o.id == model.selectedSubtitleId) { model.chooseSubtitle(o); close() } }),
                MenuSection(rows = listOf(
                    MenuRow("Style", AppSettings.subtitleSize.replaceFirstChar { it.uppercase() } + if (AppSettings.subtitleBackground) " · Background" else "") { submenu("Style", PlayerMenu.Style) },
                    MenuRow("Delay", "%+.1fs".format(java.util.Locale.ROOT, model.subtitleDelay)) { submenu("Delay", PlayerMenu.Delay) },
                    MenuRow("Search Online…") { open(PlayerMenu.Search) },
                )),
            ), keepOpen = true, focusLabel = returnRow)
        }
        PlayerMenu.Style -> MenuDialog("Style", onDismiss = { open(PlayerMenu.Subtitles) }, width = 300.dp, keepOpen = true, sections = listOf(
            MenuSection(rows = listOf("small", "medium", "large", "huge").map { z ->
                MenuRow(z.replaceFirstChar { it.uppercase() }, checked = AppSettings.subtitleSize == z) { AppSettings.chooseSubtitleSize(z) }
            }),
            MenuSection(rows = listOf(MenuRow("Background", checked = AppSettings.subtitleBackground) { AppSettings.chooseSubtitleBackground(!AppSettings.subtitleBackground) })),
        ))
        PlayerMenu.Delay -> {
            val steps = listOf(-2.0, -1.0, -0.5, -0.1, 0.1, 0.5, 1.0, 2.0)
            MenuDialog("Delay %+.1fs".format(java.util.Locale.ROOT, model.subtitleDelay), onDismiss = { open(PlayerMenu.Subtitles) }, width = 300.dp, keepOpen = true, sections = listOf(
                MenuSection(rows = steps.map { d -> MenuRow("%+.1fs".format(java.util.Locale.ROOT, d)) { model.nudgeSubtitles(d) } }),
                MenuSection(rows = listOfNotNull(
                    MenuRow("Reset") { model.resetSubtitleDelay() },
                    if (model.subtitleSyncApplied) MenuRow("Use Original Timing", checked = model.subtitleOriginalTiming) { model.useOriginalSubtitleTiming(!model.subtitleOriginalTiming) } else null,
                )),
            ))
        }
        PlayerMenu.Quality -> {
            val versions = model.versions
            val sections = buildList {
                if (versions.size > 1) add(MenuSection("Version", versions.map { v ->
                    MenuRow(v.versionTitle, v.versionDetails, checked = v.id == model.playingVersion, enabled = !v.isBroken || v.id == model.playingVersion) { model.chooseVersion(v.id) }
                }))
                add(MenuSection(if (versions.size > 1) "Quality" else null, model.qualityOptions.map { q ->
                    MenuRow(q.label, model.sizeLabel(q), checked = q.label == model.quality.label) { model.chooseQuality(q) }
                }))
            }
            MenuDialog("Quality", onDismiss = close, width = 380.dp, sections = sections)
        }
        PlayerMenu.Search -> SubtitleSearchDialog(model, onDismiss = close)
    }
}

/**
 * Search Online: OpenSubtitles through the server (a fetched track is kept on the server, for
 * every device). The title and language on the left, the results on the right.
 */
@Composable
private fun SubtitleSearchDialog(model: PlayerModel, onDismiss: () -> Unit) {
    val api = model.api
    val item = model.item
    val defaultQuery = if (item.isEpisode) item.seriesTitle ?: item.title.orEmpty() else item.title.orEmpty()
    var query by remember { mutableStateOf(defaultQuery) }
    var language by remember { mutableStateOf(Languages.normalized(AppSettings.subtitleLanguage) ?: "en") }
    var results by remember { mutableStateOf<List<SubtitleResult>>(emptyList()) }
    var quota by remember { mutableStateOf<SubtitleQuota?>(null) }
    var remaining by remember { mutableStateOf<Int?>(null) }
    var loading by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var picker by remember { mutableStateOf(false) }
    var searches by remember { mutableIntStateOf(0) }
    val firstResult = remember { FocusRequester() }
    val searchButton = remember { FocusRequester() }
    val scope = rememberCoroutineScope()

    LaunchedEffect(searches, language) {
        loading = true
        error = null
        results = emptyList()
        try {
            val a = api.searchSubtitles(model.searchMediaId, language, if (query == defaultQuery) null else query)
            results = a.results
            quota = a.quota
            remaining = a.remainingDownloads
            Log.i("MoonriseSubSearch") { "${a.results.size} results for $language" }
        } catch (e: Exception) {
            if (e is CancellationException) throw e
            error = if (e is MoonriseHttpException && e.code == 429) "No OpenSubtitles downloads left today on your server." else "Server search failed: ${e.message}"
        }
        loading = false
        delay(50)
        // Results in: the first takes the remote.
        if (results.isNotEmpty()) runCatching { firstResult.requestFocus() } else runCatching { searchButton.requestFocus() }
    }

    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Row(Modifier.fillMaxSize().background(Color(0xF0121418)).padding(horizontal = Theme.margin, vertical = 30.dp), horizontalArrangement = Arrangement.spacedBy(40.dp)) {
            Column(Modifier.width(280.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
                Text("Find Subtitles", fontSize = 22.sp, fontWeight = FontWeight.Bold, color = Color.White)
                val line = if (item.isEpisode) listOfNotNull(item.seriesTitle, item.episodeLabel, item.title).joinToString(" · ")
                    else listOfNotNull(item.title, item.year?.toString()).joinToString(" · ")
                Text(line, fontSize = 13.sp, color = Theme.secondaryText, maxLines = 2)
                TvTextField("Title", query, { query = it }, onDone = { searches++ })
                GlassButton(onClick = { picker = true }, modifier = Modifier.fillMaxWidth(), name = "subtitle search: language") {
                    Text("Language: " + (Languages.common.firstOrNull { it.first == language }?.second ?: language.uppercase()), maxLines = 1)
                }
                GlassButton(onClick = { searches++ }, enabled = !loading, modifier = Modifier.fillMaxWidth().focusRequester(searchButton), name = "subtitle search: search") {
                    Text("Search", fontWeight = FontWeight.Bold)
                }
                (quota?.summary ?: remaining?.let { if (it == 1) "1 download left today on the server's account." else "$it downloads left today on the server's account." })?.let {
                    Text(it, fontSize = 11.sp, color = if (quota?.exhausted == true) Color(0xFFFF9F0A) else Theme.secondaryText)
                }
            }
            Column(Modifier.weight(1f).fillMaxHeight(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                error?.let { Text("⚠  $it", fontSize = 13.sp, color = Color(0xFFFF9F0A)) }
                when {
                    loading -> Text("Searching OpenSubtitles…", fontSize = 14.sp, color = Theme.secondaryText, modifier = Modifier.padding(top = 10.dp))
                    results.isEmpty() -> if (error == null) Text("No subtitles found. Try another title or language.", fontSize = 14.sp, color = Theme.secondaryText, modifier = Modifier.padding(top = 10.dp))
                    else -> {
                        Text("${results.size} results · OpenSubtitles", fontSize = 11.sp, fontWeight = FontWeight.SemiBold, color = Theme.secondaryText)
                        LazyColumn(verticalArrangement = Arrangement.spacedBy(6.dp), contentPadding = PaddingValues(vertical = 6.dp, horizontal = 4.dp)) {
                            itemsIndexed(results, key = { _, r -> r.id }) { i, r ->
                                val unavailable = r.fetched != true && (quota?.exhausted == true || remaining == 0)
                                FocusSurface(
                                    onClick = {
                                        if (busy != null || unavailable) return@FocusSurface
                                        busy = r.id
                                        scope.launch {
                                            try {
                                                val track = api.fetchSubtitle(model.searchMediaId, r)
                                                Log.i("MoonriseSubSearch") { "fetched ${r.id} as track ${track.index}" }
                                                model.subtitleFetched(track)
                                                onDismiss()
                                            } catch (e: Exception) {
                                                if (e is CancellationException) throw e
                                                error = if (e is MoonriseHttpException && e.code == 429) {
                                                    remaining = 0
                                                    "No OpenSubtitles downloads left today on your server. Results marked Saved still work."
                                                } else e.message
                                            }
                                            busy = null
                                        }
                                    },
                                    modifier = (if (i == 0) Modifier.focusRequester(firstResult) else Modifier).fillMaxWidth().logFocus { "subtitle search: result #$i" },
                                    shape = RoundedCornerShape(14.dp), focusedScale = 1.02f,
                                    containerColor = Color.White.copy(alpha = 0.08f), contentColor = Color.White.copy(alpha = if (unavailable) 0.5f else 1f),
                                    focusedContainerColor = Color.White, focusedContentColor = Color.Black,
                                ) {
                                    Row(Modifier.padding(horizontal = 14.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                                        Column(Modifier.weight(1f)) {
                                            Text(r.name, fontSize = 13.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                            Text(r.detail, fontSize = 11.sp, maxLines = 1, color = androidx.tv.material3.LocalContentColor.current.copy(alpha = 0.65f))
                                        }
                                        if (r.hashMatch) Text("✓", fontSize = 13.sp, fontWeight = FontWeight.Bold)
                                        if (busy == r.id) Text("  …", fontSize = 13.sp)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if (picker) MenuDialog("Language", onDismiss = { picker = false }, sections = listOf(MenuSection(rows =
        Languages.common.map { (code, name) -> MenuRow(name, checked = code == language) { language = code } })))
}
