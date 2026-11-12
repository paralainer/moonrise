package app.moonrise.phone.player

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.outlined.FormatSize
import androidx.compose.material.icons.outlined.History
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.moonrise.core.Languages
import app.moonrise.core.MoonriseHttpException
import app.moonrise.core.SubtitleQuota
import app.moonrise.core.SubtitleResult
import app.moonrise.phone.ui.MenuItem
import app.moonrise.phone.ui.RemoteImage
import app.moonrise.phone.ui.Theme
import app.moonrise.phone.ui.Type
import app.moonrise.phone.ui.imageUrl
import app.moonrise.shared.AppSettings
import app.moonrise.shared.Log
import app.moonrise.shared.player.PlayerModel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** A button in the controls that opens its menu (the iPhone's `MenuButton`); the controls stay while it's open. */
@Composable
internal fun MenuButton(icon: ImageVector, description: String, onMenu: (Boolean) -> Unit, content: @Composable (close: () -> Unit) -> Unit) {
    var open by remember { mutableStateOf(false) }
    fun set(v: Boolean) { open = v; onMenu(v) }
    Box {
        BarButton(icon, description) { set(true) }
        DropdownMenu(open, { set(false) }, containerColor = Theme.sheet) { if (open) content { set(false) } }
    }
}

@Composable
private fun SectionTitle(text: String) {
    Text(text, style = Type.footnote, color = Theme.secondaryText, modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp))
}

/** The audio tracks: the language, then the details (5.1, the codec) (`TrackMenus.audio`). */
@Composable
internal fun AudioMenu(model: PlayerModel, close: () -> Unit) {
    SectionTitle("Audio")
    for (a in model.audioOptions) MenuItem(a.title, checked = a.id == model.selectedAudio, detail = a.detail) { model.chooseAudio(a.id); close() }
}

/** Off and the subtitle tracks; Style, Delay and Search Online (`TrackMenus.subtitles`). */
@Composable
internal fun SubtitlesMenu(model: PlayerModel, close: () -> Unit, onSearch: () -> Unit) {
    var page by remember { mutableStateOf("tracks") }
    model.timingRevision
    when (page) {
        "style" -> {
            SectionTitle("Style")
            for (z in listOf("small", "medium", "large", "huge")) MenuItem(z.replaceFirstChar { it.uppercase() }, checked = AppSettings.subtitleSize == z) { AppSettings.chooseSubtitleSize(z) }
            HorizontalDivider()
            MenuItem("Background", checked = AppSettings.subtitleBackground) { AppSettings.chooseSubtitleBackground(!AppSettings.subtitleBackground) }
            HorizontalDivider()
            MenuItem("Back") { page = "tracks" }
        }
        "delay" -> {
            SectionTitle("Delay %+.1fs".format(java.util.Locale.ROOT, model.subtitleDelay))
            for (d in listOf(-2.0, -1.0, -0.5, -0.1, 0.1, 0.5, 1.0, 2.0)) MenuItem("%+.1fs".format(java.util.Locale.ROOT, d)) { model.nudgeSubtitles(d) }
            MenuItem("Reset") { model.resetSubtitleDelay() }
            if (model.subtitleSyncApplied) MenuItem("Use Original Timing", checked = model.subtitleOriginalTiming) { model.useOriginalSubtitleTiming(!model.subtitleOriginalTiming) }
            HorizontalDivider()
            MenuItem("Back") { page = "tracks" }
        }
        else -> {
            SectionTitle("Subtitles")
            MenuItem("Off", checked = model.selectedSubtitleId == null) { model.chooseSubtitle(null); close() }
            for (o in model.subtitleOptions) MenuItem(o.name, checked = o.id == model.selectedSubtitleId, detail = model.subtitleDetail(o)) { model.chooseSubtitle(o); close() }
            HorizontalDivider()
            SubmenuItem("Style", Icons.Outlined.FormatSize, AppSettings.subtitleSize.replaceFirstChar { it.uppercase() }) { page = "style" }
            SubmenuItem("Delay", Icons.Outlined.History, "%+.1fs".format(java.util.Locale.ROOT, model.subtitleDelay)) { page = "delay" }
            if (model.canSearchSubtitles) MenuItem("Search Online…", Icons.Filled.Search) { close(); onSearch() }
        }
    }
}

@Composable
private fun SubmenuItem(title: String, icon: ImageVector, value: String, onClick: () -> Unit) {
    androidx.compose.material3.DropdownMenuItem(
        text = { Column { Text(title, style = Type.body, color = Color.White); Text(value, style = Type.footnote, color = Theme.secondaryText) } },
        onClick = onClick,
        leadingIcon = { Icon(icon, null, tint = Color.White, modifier = Modifier.size(20.dp)) },
        trailingIcon = { Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, null, tint = Theme.secondaryText) },
    )
}

/** The versions (several files) and the streaming quality, with how the server serves it (`TrackMenus.quality`). */
@Composable
internal fun QualityMenu(model: PlayerModel, close: () -> Unit) {
    if (model.versions.size > 1) {
        SectionTitle("Version")
        for (v in model.versions) MenuItem(v.versionTitle, checked = v.id == model.playingVersion, detail = v.versionDetails, enabled = !v.isBroken || v.id == model.playingVersion) {
            model.chooseVersion(v.id); close()
        }
        HorizontalDivider()
    }
    val method = model.playMethod.substringBefore(" · Moonrise Server")
    SectionTitle(if (method.isEmpty()) "Quality" else "Quality · $method")
    for (q in model.qualityOptions) MenuItem(q.label, checked = q.label == model.quality.label, detail = model.sizeLabel(q)) { model.chooseQuality(q); close() }
}

/** The show's episodes, full screen (`EpisodePicker`): a tap plays one. */
@Composable
internal fun EpisodesSheet(model: PlayerModel, onDismiss: () -> Unit) {
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheet, containerColor = Theme.sheet) {
        val seasons = model.seriesEpisodes.groupBy { it.seasonNo ?: 0 }
        var season by remember { mutableIntStateOf(model.item.seasonNo ?: seasons.keys.firstOrNull() ?: 1) }
        Column(Modifier.padding(bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            LazyRow(contentPadding = PaddingValues(horizontal = 20.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                items(seasons.keys.toList()) { s ->
                    val on = s == season
                    Text(if (s == 0) "Specials" else "Season $s", style = Type.subheadline, fontWeight = FontWeight.SemiBold, color = if (on) Color.Black else Color.White,
                        modifier = Modifier.background(if (on) Color.White else Theme.surfaceStrong, CircleShape).clickable { season = s }.padding(horizontal = 16.dp, vertical = 8.dp))
                }
            }
            LazyRow(contentPadding = PaddingValues(horizontal = 20.dp), horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                items(seasons[season].orEmpty(), key = { it.id }) { ep ->
                    Column(Modifier.size(width = 220.dp, height = 170.dp).clickable { onDismiss(); if (ep.id != model.item.id) model.playEpisode(ep) },
                        verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        Box(Modifier.fillMaxWidth().size(width = 220.dp, height = 124.dp)) {
                            RemoteImage(imageUrl(ep.cardImage, 220.dp), Modifier.fillMaxWidth().size(width = 220.dp, height = 124.dp)
                                .then(Modifier.background(Color.Transparent, RoundedCornerShape(12.dp))))
                            if (ep.id == model.item.id) Text("Now Playing", style = Type.caption, fontWeight = FontWeight.SemiBold, color = Color.White,
                                modifier = Modifier.align(Alignment.Center).background(Theme.glass, CircleShape).padding(horizontal = 10.dp, vertical = 5.dp))
                        }
                        Text(ep.episodeListTitle, style = Type.subheadline, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }
            }
        }
    }
}

/**
 * Search Online (`SubtitleSearchView`): OpenSubtitles through the server, which keeps a fetched
 * track for every device and selects it here. The title and language to search, then the results.
 */
@Composable
internal fun SubtitleSearchSheet(model: PlayerModel, onDismiss: () -> Unit) {
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
    var searches by remember { mutableIntStateOf(0) }
    var languages by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(searches, language) {
        loading = true; error = null; results = emptyList()
        try {
            val a = api.searchSubtitles(model.searchMediaId, language, if (query == defaultQuery) null else query)
            results = a.results; quota = a.quota; remaining = a.remainingDownloads
            Log.i("MoonriseSubSearch") { "${a.results.size} results for $language" }
        } catch (e: Exception) {
            if (e is CancellationException) throw e
            error = if (e is MoonriseHttpException && e.code == 429) "No OpenSubtitles downloads left today on your server." else "Server search failed: ${e.message}"
        }
        loading = false
    }

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), containerColor = Theme.sheet) {
        Column(Modifier.padding(horizontal = 20.dp).padding(bottom = 20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Find Subtitles", style = Type.title3, fontWeight = FontWeight.Bold, color = Color.White)
            OutlinedTextField(query, { query = it }, singleLine = true, label = { Text("Title") }, modifier = Modifier.fillMaxWidth(),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search), keyboardActions = KeyboardActions(onSearch = { searches++ }))
            Box {
                Row(Modifier.clickable { languages = true }, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text("Language: " + (Languages.common.firstOrNull { it.first == language }?.second ?: language.uppercase()), style = Type.subheadline, color = Color.White)
                    Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, null, tint = Theme.secondaryText)
                }
                DropdownMenu(languages, { languages = false }, containerColor = Theme.sheet) {
                    for ((code, name) in Languages.common) MenuItem(name, checked = code == language) { language = code; languages = false }
                }
            }
            (quota?.summary ?: remaining?.let { if (it == 1) "1 download left today on the server's account." else "$it downloads left today on the server's account." })?.let {
                Text(it, style = Type.caption, color = if (quota?.exhausted == true) Color(0xFFFF9F0A) else Theme.secondaryText)
            }
            error?.let { Text(it, style = Type.footnote, color = Color(0xFFFF9F0A)) }
            when {
                loading -> Text("Searching OpenSubtitles…", style = Type.subheadline, color = Theme.secondaryText)
                results.isEmpty() -> if (error == null) Text("No subtitles found. Try another title or language.", style = Type.subheadline, color = Theme.secondaryText)
                else -> LazyColumn(Modifier.fillMaxWidth().size(width = 600.dp, height = 360.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    items(results, key = { it.id }) { r ->
                        val unavailable = r.fetched != true && (quota?.exhausted == true || remaining == 0)
                        Row(Modifier.fillMaxWidth().background(Theme.surfaceStrong, RoundedCornerShape(12.dp)).clickable(enabled = busy == null && !unavailable) {
                            busy = r.id
                            scope.launch {
                                try {
                                    val track = api.fetchSubtitle(model.searchMediaId, r)
                                    model.subtitleFetched(track)
                                    onDismiss()
                                } catch (e: Exception) {
                                    if (e is CancellationException) throw e
                                    error = if (e is MoonriseHttpException && e.code == 429) { remaining = 0; "No OpenSubtitles downloads left today on your server." } else e.message
                                }
                                busy = null
                            }
                        }.padding(horizontal = 14.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                            Column(Modifier.weight(1f)) {
                                Text(r.name, style = Type.footnote, fontWeight = FontWeight.Medium, color = Color.White.copy(alpha = if (unavailable) 0.5f else 1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                                Text(r.detail, style = Type.caption, color = Theme.secondaryText, maxLines = 1)
                            }
                            if (r.hashMatch) Icon(Icons.Filled.Check, "Matches the file", tint = Color.White, modifier = Modifier.size(16.dp))
                            if (busy == r.id) Text(" …", style = Type.footnote, color = Color.White)
                        }
                    }
                }
            }
        }
    }
}
