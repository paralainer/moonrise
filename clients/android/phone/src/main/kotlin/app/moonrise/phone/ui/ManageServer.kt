// SPDX-License-Identifier: MPL-2.0
package app.moonrise.phone.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.outlined.AutoFixHigh
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.Extension
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.Lan
import androidx.compose.material.icons.outlined.Layers
import androidx.compose.material.icons.outlined.Memory
import androidx.compose.material.icons.outlined.MonitorHeart
import androidx.compose.material.icons.outlined.Movie
import androidx.compose.material.icons.outlined.People
import androidx.compose.material.icons.outlined.Storage
import androidx.compose.material3.Icon
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.moonrise.core.AdminFmt
import app.moonrise.core.AdminRules
import app.moonrise.core.LibraryRoot
import app.moonrise.core.ManagedLibrary
import app.moonrise.core.ManagedRoot
import app.moonrise.core.MoonriseApi
import app.moonrise.core.ScanLibrary
import app.moonrise.core.ScanProgress
import app.moonrise.core.accounts
import app.moonrise.core.createLibrary
import app.moonrise.core.deleteLibrary
import app.moonrise.core.managedLibraries
import app.moonrise.core.renameLibrary
import app.moonrise.core.scanProgress
import app.moonrise.core.setLibraryRoots
import app.moonrise.core.startScan
import app.moonrise.shared.Session
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.foundation.background
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Manage Server (`ManageServerView`, iPhone), admins only; the server has no other admin screens. */
@Composable
fun ManageServerPage(back: () -> Unit) {
    val api = Session.api ?: return
    val pages = LocalPages.current
    var libraries by remember { mutableStateOf<Int?>(null) }
    var review by remember { mutableStateOf<Int?>(null) }
    var accounts by remember { mutableStateOf<Int?>(null) }
    var scanning by remember { mutableStateOf(false) }
    // Each time the page comes into view, so the summary is current on the way back from a child page.
    val showing = LocalPageShowing.current
    LaunchedEffect(showing) {
        if (!showing) return@LaunchedEffect
        runCatching { api.managedLibraries() }.getOrNull()?.let { libraries = it.items.size }
        runCatching { api.scanProgress() }.getOrNull()?.let { p ->
            review = p.libraries.sumOf { AdminRules.waiting(it) }
            scanning = p.phase !in listOf("idle", "queued")
        }
        runCatching { api.accounts() }.getOrNull()?.let { accounts = it.size }
    }
    FormPage("Manage Server", back) {
        FormSection("Library") {
            ManageLink("Libraries", Icons.Outlined.Layers, detail = if (scanning) "Scanning…" else libraries?.let { AdminFmt.count(it, "library", "libraries") }) { pages.push { LibrariesPage() } }
            FormDivider()
            ManageLink("Fix Match", Icons.Outlined.AutoFixHigh, detail = AdminRules.waitingText(review), detailColor = if ((review ?: 0) > 0) Warn else Theme.secondaryText) { pages.push { ReviewListPage() } }
        }
        FormSection("People") {
            ManageLink("Users & Devices", Icons.Outlined.People, detail = accounts?.let { AdminFmt.count(it, "user") }) { pages.push { AccountsPage() } }
        }
        FormSection("Server") {
            ManageLink("Services", Icons.Outlined.Extension) { pages.push { ServicesPage() } }
            FormDivider()
            ManageLink("Playback & Hardware", Icons.Outlined.Memory) { pages.push { PlaybackPage() } }
            FormDivider()
            ManageLink("Activity", Icons.Outlined.MonitorHeart) { pages.push { ActivityPage() } }
            FormDivider()
            ManageLink("About This Server", Icons.Outlined.Info) { pages.push { AboutServerPage() } }
        }
    }
}

/** The library's passes, polled while a libraries screen is open (`ScanWatcher`). */
class ScanWatcher {
    var progress by mutableStateOf<ScanProgress?>(null)
    var error by mutableStateOf<String?>(null)
    val busy: Boolean get() = progress?.let { it.phase != "idle" } ?: false
    private var fastUntil = 0L

    /** A scan was just asked for: poll fast for a while, a quick one is over in a second or two. */
    fun kick() { fastUntil = System.currentTimeMillis() + 20_000 }

    suspend fun watch(api: MoonriseApi) {
        while (true) {
            try {
                progress = api.scanProgress(); error = null
            } catch (e: Exception) {
                if (e is CancellationException) throw e
                error = app.moonrise.core.adminErrorText(e)
            }
            delay(if (busy || System.currentTimeMillis() < fastUntil) 700 else 3000)
        }
    }

    fun library(id: String): ScanLibrary? = progress?.libraries?.firstOrNull { it.id == id }
}

/** A library folder: where it is, and whether the server can reach it (`RootRow`). */
@Composable
private fun RootRow(root: ManagedRoot) {
    FormRow {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Icon(if (root.source == "smb") Icons.Outlined.Lan else Icons.Outlined.Storage, null, tint = Color.White, modifier = Modifier.size(24.dp))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Text(root.display, style = Type.body, color = Color.White, maxLines = 2)
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    StatusDot(root.online)
                    Text(AdminRules.rootStatus(root), style = Type.caption, color = if (root.online) Theme.secondaryText else Theme.destructive, maxLines = 2)
                }
            }
        }
    }
}

/** Libraries (`LibrariesAdminView`): the scan, each library and its folders, Add Library. */
@Composable
fun LibrariesPage() {
    val api = Session.api ?: return
    val pages = LocalPages.current
    val scope = rememberCoroutineScope()
    val scan = remember { ScanWatcher() }
    var list by remember { mutableStateOf<app.moonrise.core.LibraryList?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var asked by remember { mutableStateOf<String?>(null) }
    var reloads by remember { mutableStateOf(0) }
    LaunchedEffect(reloads) { adminCall({ error = it }) { list = api.managedLibraries(); error = null } }
    LaunchedEffect(Unit) { scan.watch(api) }
    val editable = list?.managedBy != "env"

    fun start(library: String?) {
        asked = library ?: "*"
        scan.kick()
        scope.launch {
            adminCall({ error = it }) { api.startScan(library = library, full = library == null) }
            delay(4000)
            asked = null
        }
    }

    FormPage("Libraries", pages::pop) {
        FormSection("Scanning", footer = "Moonrise looks for new and changed files every few minutes on its own. Scan Now lists every folder again.") {
            FormRow {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    if (scan.busy) Spinner(24.dp) else Icon(Icons.Filled.CheckCircle, null, tint = Theme.green, modifier = Modifier.size(24.dp))
                    Text(AdminRules.scanPhaseText(scan.progress), style = Type.body, color = Color.White)
                }
            }
            scan.progress?.lastPass?.let { last -> (last.aborted ?: last.error)?.let { e -> FormDivider(); NoteRow("Last scan: $e", Warn) } }
            FormDivider()
            ActionRow(if (asked == "*") "Scan Started" else "Scan All Libraries", Icons.Filled.Refresh, enabled = !scan.busy && asked != "*") { start(null) }
        }
        list?.items?.forEach { lib ->
            val l = scan.library(lib.id)
            val waiting = l?.let { AdminRules.waiting(it) } ?: 0
            FormSection(lib.name, footer = if (waiting > 0) "${AdminFmt.count(waiting, "title")} waiting in Fix Match." else null) {
                FormRow(onClick = { pages.push { LibraryEditPage(lib, editable) { reloads++ } } }) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Icon(Icons.Outlined.Movie, null, tint = Color.White, modifier = Modifier.size(24.dp))
                        Text(AdminRules.libraryCounts(l) ?: "Movies and shows", style = Type.body, color = Color.White, modifier = Modifier.weight(1f))
                        Chevron()
                    }
                }
                lib.roots.forEach { r -> FormDivider(); RootRow(r) }
                FormDivider()
                ActionRow(if (asked == lib.id) "Scan Started" else "Scan Now", Icons.Filled.Refresh, enabled = !scan.busy && asked != lib.id) { start(lib.id) }
            }
        }
        if (editable) {
            FormSection(footer = if (list?.items?.isEmpty() == true) "No libraries yet: add a folder with your movies or shows." else null) {
                ActionRow("Add Library", Icons.Filled.Add) { pages.push { AddLibraryPage { pages.pop(); reloads++ } } }
            }
        } else if (list != null) {
            NoteSection("The libraries are set by MOONRISE_LIBRARIES in the server's environment: remove it there to manage them here.")
        }
        (error ?: scan.error)?.let { NoteSection(it, Theme.destructive) }
    }
}

/** A library: its name, its folders (add, remove), remove the library (`LibraryEditView`). */
@Composable
private fun LibraryEditPage(library: ManagedLibrary, editable: Boolean, changed: () -> Unit) {
    val api = Session.api ?: return
    val pages = LocalPages.current
    val scope = rememberCoroutineScope()
    var name by remember { mutableStateOf(library.name) }
    var roots by remember { mutableStateOf(library.roots) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var confirmRemove by remember { mutableStateOf(false) }
    var addOpen by remember { mutableStateOf(false) }
    val trimmed = name.trim()

    fun save(body: List<kotlinx.serialization.json.JsonObject>) {
        busy = true
        scope.launch {
            adminCall({ error = it }) {
                api.setLibraryRoots(library.id, body)
                error = null
                runCatching { api.managedLibraries().items.firstOrNull { it.id == library.id } }.getOrNull()?.let { roots = it.roots }
                changed()
            }
            busy = false
        }
    }

    fun addRoot(r: LibraryRoot) = save(roots.map { it.body } + r.body)

    FormPage(library.name, pages::pop) {
        FormSection("Name") {
            if (editable) FormField(name, { name = it }, "Name") else FormRow { Text(name, style = Type.body, color = Color.White) }
            if (trimmed != library.name && trimmed.isNotEmpty()) {
                FormDivider()
                BusyRow("Rename", busy) {
                    busy = true
                    scope.launch {
                        adminCall({ error = it }) { api.renameLibrary(library.id, trimmed); error = null; changed(); pages.pop() }
                        busy = false
                    }
                }
            }
        }
        FormSection("Folders", footer = if (roots.size > 1) "Swipe a folder to remove it. Changing the folders starts a full scan." else "Changing the folders starts a full scan.") {
            roots.forEachIndexed { i, r ->
                if (i > 0) FormDivider()
                if (editable && roots.size > 1) SwipeToRemove({ save(roots.filterIndexed { j, _ -> j != i }.map { it.body }) }) { RootRow(r) }
                else RootRow(r)
            }
            if (editable) {
                FormDivider()
                androidx.compose.foundation.layout.Box {
                    ActionRow("Add Folder", Icons.Filled.Add, enabled = !busy) { addOpen = true }
                    DropdownMenu(addOpen, { addOpen = false }, containerColor = Theme.sheet) {
                        MenuItem("A Folder on the Server", Icons.Outlined.Storage) { addOpen = false; pages.push { ServerFolderPicker(api, pages::pop) { r -> pages.pop(); addRoot(r) } } }
                        MenuItem("A Network Share", Icons.Outlined.Lan) { addOpen = false; pages.push { SharePicker(api, pages::pop) { r -> pages.pop(); addRoot(r) } } }
                    }
                }
            }
        }
        if (editable) {
            FormSection(footer = "Its titles leave Moonrise; the files themselves aren't touched.") {
                FormRow(onClick = { confirmRemove = true }, enabled = !busy) { Text("Remove Library", style = Type.body, color = Theme.destructive) }
            }
        }
        error?.let { NoteSection(it, Theme.destructive) }
    }
    if (confirmRemove) ConfirmDialog("Remove ${library.name}?", "Its titles and their artwork leave Moonrise, with everyone's watch history for them. The video files stay where they are.", "Remove Library", dismiss = { confirmRemove = false }) {
        confirmRemove = false
        busy = true
        scope.launch {
            adminCall({ error = it }) { api.deleteLibrary(library.id); changed(); pages.pop() }
            busy = false
        }
    }
}

/** A row that a swipe from the right removes (the iPhone's swipe to delete). */
@Composable
fun SwipeToRemove(onRemove: () -> Unit, content: @Composable () -> Unit) {
    // The swipe state keeps the first lambda it is given, so it has to read the current action.
    val remove by rememberUpdatedState(onRemove)
    val state = rememberSwipeToDismissBoxState(confirmValueChange = { if (it == SwipeToDismissBoxValue.EndToStart) { remove(); false } else false })
    SwipeToDismissBox(state, enableDismissFromStartToEnd = false, backgroundContent = {
        // Only during a swipe: under a row at rest it would show as a red hairline along the card's rounded corners.
        if (state.dismissDirection == SwipeToDismissBoxValue.EndToStart) androidx.compose.foundation.layout.Box(Modifier.fillMaxWidth().background(Theme.destructive).padding16(), contentAlignment = Alignment.CenterEnd) { Text("Remove", style = Type.body, fontWeight = FontWeight.SemiBold, color = Color.White) }
    }) {
        androidx.compose.foundation.layout.Box(Modifier.background(Color(0xFF1F2023))) { content() }
    }
}

private fun Modifier.padding16(): Modifier = this.then(Modifier.padding(16.dp))

/** A name and a folder: another library, or the setup's first (`AddLibraryView`, `SetupLibraryView`'s form). */
@Composable
internal fun LibraryForm(client: MoonriseApi, name: String, setName: (String) -> Unit, root: LibraryRoot?, setRoot: (LibraryRoot) -> Unit) {
    val pages = LocalPages.current
    FormSection("Library Name") { FormField(name, setName, "Name") }
    FormSection("Where Are the Videos?") {
        SourceRow(Icons.Outlined.Storage, "A Folder on the Server", "Folders the server's computer can see", root?.smb == false) {
            pages.push { ServerFolderPicker(client, pages::pop) { r -> setRoot(r); pages.pop() } }
        }
        FormDivider()
        SourceRow(Icons.Outlined.Lan, "A Network Share", "A NAS or another computer on your network", root?.smb == true) {
            pages.push { SharePicker(client, pages::pop) { r -> setRoot(r); pages.pop() } }
        }
        root?.let { r ->
            FormDivider()
            FormRow {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    Icon(Icons.Filled.CheckCircle, null, tint = Theme.green, modifier = Modifier.size(20.dp))
                    Text(r.display, style = Type.subheadline, color = Theme.green)
                }
            }
        }
    }
}

@Composable
private fun SourceRow(icon: androidx.compose.ui.graphics.vector.ImageVector, title: String, detail: String, chosen: Boolean, onClick: () -> Unit) {
    FormRow(onClick = onClick) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
            Icon(icon, null, tint = Color.White, modifier = Modifier.size(26.dp))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(title, style = Type.body, color = Color.White)
                Text(detail, style = Type.caption, color = Theme.secondaryText)
            }
            if (chosen) Icon(Icons.Outlined.Check, null, tint = Theme.green, modifier = Modifier.size(20.dp))
            Chevron()
        }
    }
}

/** Another library (`AddLibraryView`). Libraries have no type: movies and shows can share one. */
@Composable
private fun AddLibraryPage(done: () -> Unit) {
    val api = Session.api ?: return
    val pages = LocalPages.current
    val scope = rememberCoroutineScope()
    var name by remember { mutableStateOf("Media") }
    var root by remember { mutableStateOf<LibraryRoot?>(null) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    FormPage("Add Library", pages::pop) {
        LibraryForm(api, name, { name = it }, root, { root = it })
        FormSection(footer = "Movies and shows can share a folder: Moonrise tells them apart. It scans the folder right away; Libraries shows how it goes.") {
            BusyRow("Add Library", busy, enabled = root != null && name.isNotBlank()) {
                val r = root ?: return@BusyRow
                busy = true
                scope.launch {
                    adminCall({ error = it }) { api.createLibrary(name.trim(), r); done() }
                    busy = false
                }
            }
        }
        error?.let { NoteSection(it, Theme.destructive) }
    }
}
