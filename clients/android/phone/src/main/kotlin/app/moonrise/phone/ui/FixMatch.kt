// SPDX-License-Identifier: MPL-2.0
package app.moonrise.phone.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Undo
import androidx.compose.material.icons.outlined.AutoAwesome
import androidx.compose.material.icons.outlined.AutoFixHigh
import androidx.compose.material.icons.outlined.CallSplit
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.CheckCircleOutline
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.Layers
import androidx.compose.material.icons.outlined.MoreHoriz
import androidx.compose.material.icons.outlined.Movie
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.moonrise.core.FixMatchRules
import app.moonrise.core.IdentifyRequest
import app.moonrise.core.Item
import app.moonrise.core.LatestSearch
import app.moonrise.core.MatchCandidate
import app.moonrise.core.MatchFile
import app.moonrise.core.MatchInfo
import app.moonrise.core.MatchSearch
import app.moonrise.core.MatchShowStep
import app.moonrise.core.ReviewItem
import app.moonrise.core.TMDBEpisode
import app.moonrise.core.TMDBSeason
import app.moonrise.core.identify
import app.moonrise.core.matchAutomatically
import app.moonrise.core.matchInfo
import app.moonrise.core.matchPosterUrl
import app.moonrise.core.reviewList
import app.moonrise.core.searchMatches
import app.moonrise.core.splitOff
import app.moonrise.core.tmdbSeason
import app.moonrise.shared.Session
import coil.compose.AsyncImage
import coil.request.ImageRequest
import kotlinx.coroutines.launch

/** Presents Fix Match over everything, from a title's long-press menu (`FixMatchPresenter`). */
object FixMatchPresenter {
    var target by mutableStateOf<String?>(null)
    /** Counts matches made by hand: a title page showing one reloads (`moonriseItemRematched`). */
    var rematched by mutableIntStateOf(0)

    fun present(id: String) { target = id }
    fun applies(item: Item): Boolean = item.isMovie || item.isSeries || item.isEpisode
}

/** The Fix Match "sheet" over the app (a page with Cancel), with its own stack. */
@Composable
fun FixMatchHost() {
    val target = FixMatchPresenter.target ?: return
    androidx.compose.runtime.key(target) {
        val pages = remember { PageStack() }
        androidx.activity.compose.BackHandler { FixMatchPresenter.target = null }
        CompositionLocalProvider(LocalPages provides pages) {
            Box(Modifier.fillMaxSize().background(Color.Black).pointerInput(Unit) {}) {
                FixMatchPage(target, inSheet = true) { newId ->
                    FixMatchPresenter.target = null
                    if (newId != null) FixMatchPresenter.rematched++
                }
                pages.Host()
            }
        }
    }
}

/** A title's long press: Fix Match… (admins; the iPhone's context menu on the title). */
@Composable
fun fixMatchMenuOpen(): androidx.compose.runtime.MutableState<Boolean> = remember { mutableStateOf(false) }

@Composable
fun FixMatchMenu(item: Item, open: androidx.compose.runtime.MutableState<Boolean>) {
    DropdownMenu(open.value, { open.value = false }, containerColor = Theme.sheet) {
        MenuItem("Fix Match…", Icons.Outlined.AutoFixHigh) { open.value = false; FixMatchPresenter.present(item.id) }
    }
}

/** Fix Match in Manage Server: the titles Moonrise couldn't recognize for sure (`ReviewListView`). */
@Composable
fun ReviewListPage() {
    val api = Session.api ?: return
    val pages = LocalPages.current
    var items by remember { mutableStateOf<List<ReviewItem>>(emptyList()) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }
    var reloads by remember { mutableIntStateOf(0) }
    LaunchedEffect(reloads) {
        loading = true
        adminCall({ error = it }) { items = api.reviewList(); error = null }
        loading = false
    }
    FormPage("Fix Match", pages::pop) {
        if (loading && items.isEmpty()) SpinnerSection()
        if (!loading && items.isEmpty() && error == null) {
            FormSection { FormRow {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Icon(Icons.Outlined.CheckCircleOutline, null, tint = Theme.green, modifier = Modifier.size(22.dp))
                    Text("Every title is recognized.", style = Type.body, color = Theme.green)
                }
            } }
        }
        for ((title, group) in FixMatchRules.groups(items)) {
            FormSection(title, footer = FixMatchRules.groupFooter(group)) {
                group.forEachIndexed { i, r ->
                    if (i > 0) FormDivider()
                    FormRow(onClick = { pages.push { FixMatchPage(r.id, inSheet = false) { reloads++ } } }) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                                Text(FixMatchRules.rowTitle(r), style = Type.body, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                Text(FixMatchRules.reason(r), style = Type.caption, color = Theme.secondaryText, maxLines = 2)
                            }
                            Chevron()
                        }
                    }
                }
            }
        }
        error?.let { NoteSection(it, Theme.destructive) }
    }
}

private sealed interface Pick {
    data class Title(val c: MatchCandidate) : Pick
    data class Episode(val show: Int, val season: Int, val episode: Int, val title: String, val existing: Int = 0, val parts: Int = 0) : Pick
    data object Local : Pick
}

/**
 * One title (`FixMatchView`): a movie or show is matched to a TMDB title (the candidates kept from
 * identification, found now when none were kept, or a search), or kept as named from its files; an
 * episode to one of its show's episodes in TMDB's numbering (or kept as numbered).
 */
@Composable
fun FixMatchPage(itemId: String, inSheet: Boolean, fileId: String? = null, done: (String?) -> Unit) {
    val api = Session.api ?: return
    val pages = LocalPages.current
    val scope = rememberCoroutineScope()
    var info by remember { mutableStateOf<MatchInfo?>(null) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }
    var query by remember { mutableStateOf("") }
    var year by remember { mutableStateOf("") }
    var results by remember { mutableStateOf<List<MatchCandidate>?>(null) }
    var searching by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var pick by remember { mutableStateOf<Pick?>(null) }
    var show by remember { mutableStateOf<Pair<Int, String>?>(null) }
    var season by remember { mutableStateOf<TMDBSeason?>(null) }
    var seasonNo by remember { mutableStateOf<Int?>(null) }
    var changingShow by remember { mutableStateOf(false) }
    var asType by remember { mutableStateOf<String?>(null) }
    var confirmAutomatic by remember { mutableStateOf<String?>(null) }
    var askedAutomatic by remember { mutableStateOf(false) }
    var notice by remember { mutableStateOf<String?>(null) }
    var reloads by remember { mutableIntStateOf(0) }

    val searches = remember { LatestSearch() }

    val kind = FixMatchRules.kind(asType, info?.kind)
    val switched = FixMatchRules.isSwitched(asType, info?.kind)
    val isEpisode = kind == "episode"
    val searchKind = FixMatchRules.searchKind(kind)

    fun load() { reloads++ }
    LaunchedEffect(reloads) {
        loading = true
        adminCall({ error = it }) {
            val i = api.matchInfo(itemId, compute = true)
            info = i
            if (query.isEmpty()) query = if (i.kind == "episode") i.seriesTitle.orEmpty() else i.title
            if (i.kind == "movie" && i.year != null && year.isEmpty()) year = "${i.year}"
            if (i.kind == "episode" && i.seriesTmdbId != null) { show = i.seriesTmdbId!! to i.seriesTitle.orEmpty(); seasonNo = i.season }
            error = null
        }
        loading = false
    }
    LaunchedEffect(show?.first, seasonNo) {
        val s = show ?: return@LaunchedEffect
        val r = adminCall({ error = it; season = null }) { api.tmdbSeason(s.first, seasonNo).also { error = null } }
        if (r != null) { season = r; if (seasonNo == null) seasonNo = r.season }
    }

    // The kind searched comes with the request: the vals above are this composition's, and a switch
    // changes `asType` before the next one. An answer to anything but the newest search is dropped.
    fun runSearch(s: MatchSearch) {
        val mine = searches.begin()
        searching = true
        scope.launch {
            val found = adminCall({ if (searches.isNewest(mine)) error = it }) { api.searchMatches(s.kind, s.query, s.year) }
            if (searches.isNewest(mine)) {
                searching = false
                if (found != null) { results = found; error = null }
            }
        }
    }
    fun search() = runSearch(FixMatchRules.search(FixMatchRules.kind(asType, info?.kind), query, year))
    fun switchType(to: String, i: MatchInfo) {
        val s = FixMatchRules.switchType(to, i, year)
        searches.forget()
        searching = false
        asType = s.asType
        results = null
        pick = null
        query = s.query
        s.year?.let { year = it }
        when (s.showStep) {
            MatchShowStep.FindShow -> { show = null; season = null; seasonNo = null; changingShow = true }
            MatchShowStep.OwnShow -> { changingShow = false; i.seriesTmdbId?.let { show = it to i.seriesTitle.orEmpty(); seasonNo = i.season } }
            MatchShowStep.Keep -> Unit
        }
        s.search?.let { runSearch(it) }
    }
    fun apply(p: Pick, part: Boolean = false) {
        pick = null
        busy = true
        scope.launch {
            adminCall({ error = it }) {
                val type = if (switched && kind != "series") kind else null
                val newId = when (p) {
                    is Pick.Title -> api.identify(itemId, IdentifyRequest(tmdbId = p.c.tmdbId, type = type, fileId = fileId))
                    is Pick.Episode -> {
                        val keep = if (p.title.isEmpty() && !switched) info?.part else null
                        api.identify(itemId, IdentifyRequest(tmdbId = p.show, season = p.season, episode = p.episode, type = type, part = part || keep != null, keepPart = keep, fileId = fileId))
                    }
                    Pick.Local -> api.identify(itemId, IdentifyRequest(local = true, type = type, fileId = fileId))
                }
                HomeStore.reload()
                done(newId ?: itemId)
                if (!inSheet) pages.pop()
            }
            busy = false
        }
    }
    fun automatic(file: String?) {
        confirmAutomatic = null
        askedAutomatic = false
        busy = true
        scope.launch {
            adminCall({ error = it }) {
                val a = api.matchAutomatically(itemId, file)
                HomeStore.reload()
                if (file == null) {
                    done(a.item ?: itemId)
                    if (!inSheet) pages.pop()
                } else {
                    FixMatchPresenter.rematched++
                    notice = "That file is identified automatically again."
                    load()
                }
            }
            busy = false
        }
    }
    fun split(file: String) {
        busy = true
        scope.launch {
            adminCall({ error = it }) {
                val a = api.splitOff(itemId, file)
                HomeStore.reload()
                FixMatchPresenter.rematched++
                load()
                val next = a.fileItem
                if (next != null && next != itemId) pages.push { FixMatchPage(next, inSheet = false, done = done) }
            }
            busy = false
        }
    }
    fun episodePick(tmdb: Int, e: TMDBEpisode, title: String? = null): Pick {
        val (files, titles) = FixMatchRules.otherFilesOfEpisode(e, info, tmdb, fileId)
        return Pick.Episode(tmdb, e.season, e.episode, title ?: e.title.orEmpty(), files, titles)
    }
    fun isOther(c: MatchCandidate) = FixMatchRules.isOtherTitle(c, itemId)

    @Composable
    fun candidateRow(c: MatchCandidate, current: Boolean) {
        FormRow(onClick = {
            if (isEpisode) { show = c.tmdbId to c.title; seasonNo = null; changingShow = false; results = null } else pick = Pick.Title(c)
        }) {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                MatchPoster(c.posterPath)
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        Text(c.year?.let { "${c.title} ($it)" } ?: c.title, style = Type.body, color = Color.White, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                        if (current) Icon(Icons.Outlined.Check, null, tint = Theme.green, modifier = Modifier.size(18.dp))
                    }
                    if (!c.originalTitle.isNullOrEmpty() && c.originalTitle != c.title) Text(c.originalTitle!!, style = Type.caption, color = Theme.secondaryText, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    if (isOther(c)) Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        Icon(Icons.Outlined.Layers, null, tint = Theme.green, modifier = Modifier.size(13.dp))
                        Text("In your library: adds a version", style = Type.caption, color = Theme.green)
                    }
                    if (!c.overview.isNullOrEmpty()) Text(c.overview!!, style = Type.caption, color = Theme.secondaryText, maxLines = 2, overflow = TextOverflow.Ellipsis)
                }
            }
        }
    }

    @Composable
    fun candidates(list: List<MatchCandidate>, empty: String, current: (MatchCandidate) -> Boolean) {
        if (list.isEmpty()) FormRow { Text(empty, style = Type.body, color = Theme.secondaryText) }
        list.forEachIndexed { i, c -> if (i > 0) FormDivider(); candidateRow(c, current(c)) }
    }

    @Composable
    fun searchSection() {
        FormSection(if (isEpisode) "Find the Show" else "Search") {
            InputRow("Title", query, { query = it }, if (searchKind == "movie") "Movie title" else "Show title", onDone = { if (query.isNotBlank()) search() })
            if (!isEpisode) { FormDivider(); InputRow("Year", year, { year = it }, "Any", keyboard = KeyboardType.Number) }
            FormDivider()
            BusyRow("Search TMDB", searching, enabled = query.isNotBlank()) { search() }
        }
    }

    Box(Modifier.fillMaxSize()) {
        FormPage("Fix Match", if (inSheet) ({ done(null) }) else pages::pop, backLabel = if (inSheet) "Cancel" else null) {
            val i = info
            if (i != null) {
                FormSection(if (fileId != null) "This File" else if (i.kind == "episode") "This Episode" else "This Title",
                    footer = if (fileId == null && i.manual == true) "Back to Automatic undoes the matches made by hand: Moonrise identifies ${if (i.kind == "series") "the show's episodes" else "its files"} on its own again." else null) {
                    FormRow {
                        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Text(FixMatchRules.heading(i), style = Type.headline, color = Color.White)
                            matchFileName(i, fileId)?.let { Text(it, style = Type.caption, color = Theme.secondaryText, maxLines = 2) }
                            Text(FixMatchRules.stateText(i), style = Type.caption, color = if (i.state == "review") Warn else Theme.secondaryText)
                        }
                    }
                    notice?.let { FormDivider(); FormRow { Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) { Icon(Icons.Outlined.CheckCircleOutline, null, tint = Theme.green, modifier = Modifier.size(18.dp)); Text(it, style = Type.footnote, color = Theme.green) } } }
                    if (fileId == null && i.manual == true) { FormDivider(); ActionRow("Back to Automatic", Icons.AutoMirrored.Outlined.Undo) { askedAutomatic = true } }
                }
                if (fileId == null && (i.fileList?.size ?: 0) > 1) {
                    FormSection("Files", footer = "These files play as versions of one title. If one is something else, split it off and match it on its own.") {
                        i.fileList.orEmpty().forEachIndexed { n, f ->
                            if (n > 0) FormDivider()
                            FileRow(f, match = { pages.push { FixMatchPage(itemId, inSheet = false, fileId = f.id, done = done) } }, split = { split(f.id) }, automatic = { confirmAutomatic = f.id })
                        }
                    }
                }
                if (i.tmdb && i.kind in listOf("movie", "episode", "series")) {
                    val own = i.kind
                    val other = if (own == "movie") "episode" else own
                    FormSection("This Is", footer = if (!switched) null else if (kind == "movie") "Pick the movie below: ${if (own == "series") "its files" else "the file"} will be a movie from now on." else "Pick its show and episode below: the file will be an episode from now on.") {
                        Segmented(listOf("A Movie" to "movie", (if (own == "series") "A Show" else "An Episode") to other), if (kind == "movie") "movie" else other) { switchType(it, i) }
                    }
                }
                if (!i.tmdb) {
                    NoteSection("The server has no TMDB key, so titles can't be looked up. Add one in Manage Server → Services.", Warn)
                } else if (isEpisode) {
                    val s = season
                    FormSection("Match To", footer = when {
                        switched && show == null -> "Find its show below, then pick the episode."
                        switched -> "Pick the episode as TMDB numbers it."
                        else -> "Pick the episode as TMDB numbers it. Keeping its own number takes it off this list; it plays as it is, without TMDB's details."
                    }) {
                        InfoRow("Show", show?.second ?: i.seriesTitle ?: "—")
                        if (s != null && s.seasons.size > 1) {
                            FormDivider()
                            ChoiceRow("Season", seasonName(seasonNo ?: s.season), s.seasons.map { seasonName(it) to "$it" }) { seasonNo = it.toInt() }
                        }
                        if (s != null && s.tmdbId == show?.first) {
                            s.episodes.filter { FixMatchRules.likely(it, i) }.forEach { e ->
                                FormDivider()
                                ActionRow("S${e.season}E${e.episode} · ${e.title.orEmpty()}", Icons.Outlined.AutoAwesome, color = Color.White) { pick = episodePick(s.tmdbId, e) }
                            }
                        }
                        val sh = show
                        if (!switched && sh != null && sh.first == i.seriesTmdbId && i.season != null && i.episode != null) {
                            FormDivider()
                            ActionRow("Keep as S${i.season}E${i.episode}${i.part?.let { " Part $it" } ?: ""}", Icons.Outlined.CheckCircleOutline) { pick = Pick.Episode(sh.first, i.season!!, i.episode!!, "") }
                        }
                        if (!switched || show != null) {
                            FormDivider()
                            ActionRow(if (changingShow) "Keep This Show" else "A Different Show…", Icons.Outlined.Search, enabled = !(switched && show == null)) { changingShow = !changingShow; results = null }
                        }
                    }
                    if (changingShow) {
                        searchSection()
                        results?.let { rs -> FormSection("Shows") { candidates(rs, "Nothing found.") { it.tmdbId == show?.first } } }
                    }
                    if (s != null) {
                        FormSection(if (s.season == 0) "Specials on TMDB" else "Season ${s.season} on TMDB") {
                            s.episodes.forEachIndexed { n, e ->
                                if (n > 0) FormDivider()
                                FormRow(onClick = { pick = episodePick(s.tmdbId, e) }) {
                                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                        Text("E${e.episode}", style = Type.body, color = Theme.secondaryText, modifier = Modifier.width(44.dp))
                                        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                                            Text(e.title ?: "Episode ${e.episode}", style = Type.body, color = Color.White)
                                            if (!e.airDate.isNullOrEmpty()) Text(e.airDate!!, style = Type.caption, color = Theme.secondaryText)
                                        }
                                        if (FixMatchRules.likely(e, i)) Text("Same title", style = Type.caption, color = Theme.green)
                                    }
                                }
                            }
                        }
                    } else if (show != null) SpinnerSection()
                } else {
                    if (!switched) FormSection("Likely Matches") { candidates(i.candidates, "No likely matches: search below.") { it.tmdbId == i.tmdbId && !i.local } }
                    searchSection()
                    results?.let { rs -> FormSection("Results") { candidates(rs, "Nothing found.") { it.tmdbId == i.tmdbId && !i.local } } }
                    if (!i.local || switched) {
                        FormSection(footer = "Named from the file and its folder, with artwork from the folder or the video; it isn't looked up again.") {
                            ActionRow("Don't Match: Use the File Name", Icons.Outlined.Description) { pick = Pick.Local }
                        }
                    }
                }
            } else if (loading) {
                FormSection { FormRow { Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) { Spinner(); Text("Looking for matches…", style = Type.body, color = Theme.secondaryText) } } }
            }
            error?.let { NoteSection(it, Theme.destructive) }
        }
        if (busy) {
            Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.35f)).pointerInput(Unit) {}, contentAlignment = Alignment.Center) {
                Row(Modifier.clip(RoundedCornerShape(16.dp)).background(Theme.sheet).padding(24.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Spinner()
                    Text("Matching…", style = Type.body, color = Color.White)
                }
            }
        }
    }
    pick?.let { p -> PickDialog(p, switched, info, isOther = ::isOther, dismiss = { pick = null }, apply = { part -> apply(p, part) }) }
    if (askedAutomatic) {
        ConfirmDialog("Back to automatic?", "Moonrise forgets the matches made by hand for ${if (info?.kind == "series") "this show's episodes" else "this title's files"} and identifies them on its own again. Watch history stays.", "Back to Automatic", destructive = false, dismiss = { askedAutomatic = false }) { automatic(null) }
    }
    confirmAutomatic?.let { f ->
        ConfirmDialog("This file back to automatic?", "Moonrise forgets the match made by hand for this file and identifies it on its own again. Watch history stays.", "Back to Automatic", destructive = false, dismiss = { confirmAutomatic = null }) { automatic(f) }
    }
}

private fun matchFileName(i: MatchInfo, fileId: String?): String? {
    if (fileId != null) i.fileList?.firstOrNull { it.id == fileId }?.let { return it.name }
    if ((i.fileList?.size ?: 0) > 1) return null
    return i.files.firstOrNull()?.substringAfterLast('/')
}

private fun seasonName(n: Int) = if (n == 0) "Specials" else "Season $n"

/** A segmented control (`Picker`, `.segmented`). */
@Composable
private fun Segmented(options: List<Pair<String, String>>, selected: String, pick: (String) -> Unit) {
    Row(Modifier.fillMaxWidth().padding(8.dp).clip(RoundedCornerShape(10.dp)).background(Theme.surface)) {
        for ((label, value) in options) {
            val on = value == selected
            Box(Modifier.weight(1f).clip(RoundedCornerShape(10.dp)).background(if (on) Theme.surfaceStrong else Color.Transparent).clickable { if (!on) pick(value) }.padding(vertical = 8.dp), contentAlignment = Alignment.Center) {
                Text(label, style = Type.subheadline, color = if (on) Color.White else Theme.secondaryText)
            }
        }
    }
}

/** A file of a title with several, and what can be done with it (a menu). */
@Composable
private fun FileRow(f: MatchFile, match: () -> Unit, split: () -> Unit, automatic: () -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        FormRow(onClick = { open = true }) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(f.name ?: "File", style = Type.body, color = Color.White, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    Text(FixMatchRules.fileLine(f), style = Type.caption, color = Theme.secondaryText)
                }
                Icon(Icons.Outlined.MoreHoriz, null, tint = Theme.secondaryText)
            }
        }
        DropdownMenu(open, { open = false }, containerColor = Theme.sheet) {
            MenuItem("Match This File…", Icons.Outlined.AutoFixHigh) { open = false; match() }
            MenuItem("Split Off This File", Icons.Outlined.CallSplit) { open = false; split() }
            if (f.manual) MenuItem("Back to Automatic", Icons.AutoMirrored.Outlined.Undo) { open = false; automatic() }
        }
    }
}

/** A candidate's poster from the server (its route is for admins: the request carries the token); the film icon when there is none or it fails. */
@Composable
private fun MatchPoster(path: String?) {
    val api = Session.api
    val url = api?.matchPosterUrl(path)
    val token = api?.token
    val context = LocalContext.current
    val request = remember(url, token) {
        url?.let { u -> ImageRequest.Builder(context).data(u).apply { token?.let { addHeader("Authorization", "Bearer $it") } }.build() }
    }
    Box(Modifier.width(46.dp).height(69.dp).clip(RoundedCornerShape(6.dp)).background(Theme.surfaceStrong), contentAlignment = Alignment.Center) {
        Icon(Icons.Outlined.Movie, null, tint = Theme.secondaryText, modifier = Modifier.size(22.dp))
        if (request != null) AsyncImage(model = request, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
    }
}

@Composable
private fun PickDialog(p: Pick, switched: Boolean, info: MatchInfo?, isOther: (MatchCandidate) -> Boolean, dismiss: () -> Unit, apply: (Boolean) -> Unit) {
    val title = when (p) {
        is Pick.Title -> "Match to ${p.c.year?.let { "${p.c.title} ($it)" } ?: p.c.title}?"
        is Pick.Episode -> if (p.existing > 0) "S${p.season}E${p.episode} “${p.title}” is in your library"
            else if (p.title.isEmpty()) "Keep as S${p.season}E${p.episode}?" else "Match to S${p.season}E${p.episode} “${p.title}”?"
        Pick.Local -> "Use the file name?"
    }
    val message = when (p) {
        is Pick.Title -> if (isOther(p.c)) "You have this movie already: this file becomes another version of it (another cut, quality or language), and you pick which one plays."
            else if (switched) "It becomes a movie: Moonrise identifies it again and fetches its artwork and details. Watch history stays."
            else "Moonrise identifies it again and fetches its artwork and details. Watch history stays."
        is Pick.Episode -> when {
            p.existing > 0 -> "Is this file the next part of that episode (one episode split over several files), or another version of it (another cut, quality or language)?"
            switched -> "It becomes an episode of that show, with its details. Watch history stays."
            p.title.isEmpty() -> "It leaves Fix Match and plays as it is, under its own number."
            else -> "Moonrise files it under that episode and fetches its details. Watch history stays."
        }
        Pick.Local -> "It stays in the library under the name of its file or folder."
    }
    when {
        p is Pick.Episode && p.existing > 0 -> ConfirmDialog(title, message, "Another Version of It", destructive = false, secondary = "Part ${maxOf(p.parts, 1) + 1} of This Episode" to { apply(true) }, dismiss = dismiss) { apply(false) }
        p is Pick.Title && isOther(p.c) -> ConfirmDialog(title, message, "Add as Another Version", destructive = false, dismiss = dismiss) { apply(false) }
        else -> ConfirmDialog(title, message, if (p == Pick.Local) "Use the File Name" else if (p is Pick.Episode && p.title.isEmpty()) "Keep" else "Match", destructive = false, dismiss = dismiss) { apply(false) }
    }
}

