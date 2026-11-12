// SPDX-License-Identifier: MPL-2.0
package app.moonrise.phone.ui

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.FileDownload
import androidx.compose.material.icons.outlined.FileUpload
import androidx.compose.material.icons.outlined.Memory
import androidx.compose.material.icons.outlined.Movie
import androidx.compose.material.icons.outlined.PlayCircle
import androidx.compose.material.icons.outlined.RestartAlt
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.WbSunny
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import app.moonrise.core.Activity
import app.moonrise.core.AdminFmt
import app.moonrise.core.AdminRules
import app.moonrise.core.SystemInfo
import app.moonrise.core.activity
import app.moonrise.core.isUp
import app.moonrise.core.restartServer
import app.moonrise.core.serverLog
import app.moonrise.core.settingsDocument
import app.moonrise.core.systemInfo
import app.moonrise.core.updateSettings
import app.moonrise.shared.Session
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonPrimitive

/** "Restart the server?" and the restart itself (the server exits; Docker starts it again; back in a few seconds). */
@Composable
private fun RestartDialog(open: Boolean, scope: kotlinx.coroutines.CoroutineScope, dismiss: () -> Unit, after: suspend () -> Unit) {
    if (!open) return
    ConfirmDialog("Restart the server?", "It's back in a few seconds. Anyone watching sees a short pause.", "Restart", dismiss = dismiss) {
        dismiss()
        scope.launch {
            val api = Session.api ?: return@launch
            runCatching { api.restartServer() }
            for (i in 0 until 30) {
                delay(2000)
                if (api.isUp()) break
            }
            after()
        }
    }
}

/** Playback & Hardware (`PlaybackAdminView`): what transcodes, how much at once, Prepare Files Ahead, Scrubbing Thumbnails. */
@Composable
fun PlaybackPage() {
    val api = Session.api ?: return
    val pages = LocalPages.current
    val scope = rememberCoroutineScope()
    var system by remember { mutableStateOf<SystemInfo?>(null) }
    var settings by remember { mutableStateOf<List<AdminRules.PerfSetting>>(emptyList()) }
    val edited = remember { mutableStateMapOf<String, Int>() }
    var doc by remember { mutableStateOf<AdminRules.PlaybackDoc?>(null) }
    var activity by remember { mutableStateOf<Activity?>(null) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var note by remember { mutableStateOf<String?>(null) }
    var confirmRestart by remember { mutableStateOf(false) }

    suspend fun load() {
        adminCall({ error = it }) {
            val s = api.systemInfo()
            val d = api.settingsDocument()
            system = s
            settings = AdminRules.perfSettings(d)
            doc = AdminRules.playbackDoc(d)
            edited.clear()
            error = null
        }
    }
    LaunchedEffect(Unit) { load() }
    LaunchedEffect(Unit) {
        // The progress of Preparing Files and Scrubbing Thumbnails.
        while (true) {
            runCatching { api.activity() }.getOrNull()?.let { activity = it }
            delay(5000)
        }
    }
    fun change(key: String, value: kotlinx.serialization.json.JsonElement, then: suspend () -> Unit) {
        busy = true
        scope.launch {
            adminCall({ error = it }) { api.updateSettings(mapOf(key to value)); then() }
            busy = false
        }
    }

    FormPage("Playback & Hardware", pages::pop) {
        val s = system
        val d = doc
        if (s != null) {
            val t = s.transcode
            FormSection("Hardware", footer = (listOf(AdminRules.hardwareFooter(s)) + (t?.notes ?: emptyList())).joinToString(" ")) {
                InfoRow("Video Transcoding", AdminRules.transcodingText(s), Icons.Outlined.Movie, tint = if (AdminRules.isHardware(s.transcoding)) Theme.green else if (s.transcoding == "off" || s.transcoding == "cpu") null else Warn)
                if (t != null && s.transcoding != "off" && t.available) {
                    FormDivider()
                    InfoRow("Decoding", t.decode, Icons.Outlined.FileDownload)
                    FormDivider()
                    InfoRow("Encoding", t.encode + if (t.maxHeight < 2160) " · up to ${t.maxHeight}p" else "", Icons.Outlined.FileUpload)
                    FormDivider()
                    InfoRow("HDR to SDR", t.tonemap, Icons.Outlined.WbSunny)
                }
                s.hardware.nvidia.forEach { g ->
                    FormDivider()
                    InfoRow(g.name ?: "NVIDIA GPU", listOfNotNull(g.driver?.let { "Driver $it" }, g.memoryMb?.let { "${it / 1024} GB" }).joinToString(" · "), Icons.Outlined.Memory)
                }
                s.hardware.otherGpus.forEach { g ->
                    FormDivider()
                    InfoRow("${g.vendor} GPU", if (g.usable) g.device else "Not passed to the server", Icons.Outlined.Memory)
                }
                FormDivider()
                InfoRowStack(AdminRules.cpuName(s.hardware.cpu.model), AdminFmt.count(s.hardware.cpu.threads, "thread"), Icons.Outlined.Memory)
            }
            val current = d?.hardware
            if (t != null && current != null) {
                val choices = AdminRules.choices(t)
                val footer = if (d.hardwareLocked) "Set by MOONRISE_HWACCEL in the server's environment."
                else AdminRules.automaticText(t) + " A choice that doesn't work falls back to Automatic. Takes effect when the server restarts." + (if (t.requested != current) " Waiting for a restart." else "")
                FormSection(footer = footer) {
                    ChoiceRow("Transcode With", choices.firstOrNull { it.first == current }?.second ?: current, choices.map { it.second to it.first }, enabled = !busy && !d.hardwareLocked) { v ->
                        change("transcode.hardware", JsonPrimitive(v)) { note = "Saved. Takes effect after a restart."; load() }
                    }
                }
            }
        }
        if (settings.isNotEmpty()) {
            val perfRow: @Composable (AdminRules.PerfSetting) -> Unit = { p ->
                StepperRow(p, edited[p.key] ?: p.value, edited.containsKey(p.key)) { v -> if (v == p.value) edited.remove(p.key) else edited[p.key] = v }
            }
            FormSection("Streaming and Jobs", footer = "These take effect when the server restarts.") {
                settings.filter { it.apply != "live" }.forEachIndexed { i, p -> if (i > 0) FormDivider(); perfRow(p) }
            }
            FormSection("Scanning", footer = "These take effect at once. A setting given in the server's environment can't be changed here.") {
                settings.filter { it.apply == "live" }.forEachIndexed { i, p -> if (i > 0) FormDivider(); perfRow(p) }
            }
            FormSection {
                BusyRow("Save", busy, enabled = edited.isNotEmpty()) {
                    busy = true
                    scope.launch {
                        adminCall({ error = it }) {
                            api.updateSettings(edited.mapValues { JsonPrimitive(it.value) })
                            val later = edited.keys.filter { k -> settings.firstOrNull { it.key == k }?.apply == "restart" }
                            note = AdminRules.savedNote(later)
                            load()
                        }
                        busy = false
                    }
                }
            }
        }
        val prepare = d?.prepareAhead
        if (prepare != null) {
            FormSection("Preparing Files", footer = if (d.prepareLocked) "Switched off by MOONRISE_DISABLE in the server's environment." else "AVI and similar files are read once before their first play, so it starts at once. One file at a time, paused while anything plays.") {
                ToggleRow("Prepare Files Ahead", prepare, enabled = !busy && !d.prepareLocked) { v ->
                    change("features.index_backfill", JsonPrimitive(v)) { doc = d.copy(prepareAhead = v); note = if (v) "Files are prepared ahead from now on." else "Files are prepared when first played." }
                }
                if (prepare) { FormDivider(); InfoRow("Progress", AdminRules.preparingText(activity?.indexBackfill)) }
            }
        }
        val thumbs = d?.thumbnails
        if (thumbs != null) {
            FormSection("Scrubbing Thumbnails", footer = if (d.thumbnailsLocked) "Switched off by MOONRISE_DISABLE in the server's environment." else "A picture every 10 seconds of every title, shown above the scrubber while seeking. Made one file at a time, paused while anything plays. When the space is full, titles you play push out the thumbnails used longest ago.") {
                ToggleRow("Scrubbing Thumbnails", thumbs, enabled = !busy && !d.thumbnailsLocked) { v ->
                    change("features.trickplay", JsonPrimitive(v)) { doc = d.copy(thumbnails = v); note = if (v) "Scrubbing thumbnails are made from now on." else "No more scrubbing thumbnails are made or shown." }
                }
                if (thumbs) { FormDivider(); InfoRow("Progress", AdminRules.thumbnailsText(activity?.trickplay)) }
                AdminRules.thumbnailSpaceText(activity?.trickplay)?.let { FormDivider(); InfoRow("Space Used", it) }
            }
        }
        if (s != null && s.restartRequired.isNotEmpty()) {
            FormSection {
                NoteRow("Waiting for a restart: ${s.restartRequired.joinToString(", ", transform = AdminRules::settingTitle)}.", Warn)
                if (s.canRestart) { FormDivider(); ActionRow("Restart Server", Icons.Outlined.RestartAlt) { confirmRestart = true } }
            }
        }
        note?.let { NoteSection(it, Theme.green) }
        error?.let { NoteSection(it, Theme.destructive) }
    }
    RestartDialog(confirmRestart, scope, { confirmRestart = false }) { note = null; load() }
}

/** Activity (`ActivityAdminView`): what's playing, the jobs, recent problems. */
@Composable
fun ActivityPage() {
    val api = Session.api ?: return
    val pages = LocalPages.current
    var activity by remember { mutableStateOf<Activity?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) {
        while (true) {
            adminCall({ error = it }) { activity = api.activity(); error = null }
            delay(4000)
        }
    }
    FormPage("Activity", pages::pop) {
        val a = activity
        FormSection("Playing Now") {
            when {
                a == null -> SpinnerRow()
                a.sessions.isEmpty() -> NoteRow("Nothing is playing.")
                else -> a.sessions.forEachIndexed { i, s ->
                    if (i > 0) FormDivider()
                    InfoRowStack(AdminRules.sessionTitle(s), AdminRules.sessionLine(s), if (s.method == "transcode") Icons.Outlined.Settings else Icons.Outlined.PlayCircle)
                }
            }
        }
        if (a != null) {
            FormSection("Jobs") {
                InfoRow("Library", AdminRules.scanText(a.scan))
                FormDivider()
                InfoRow("Video Transcodes", if (a.transcodes.isEmpty()) "None" else "${a.transcodes.size} of ${a.transcodeSlots} slots")
                FormDivider()
                InfoRow("Subtitle Extraction", AdminRules.subsText(a.subtitles))
                a.subtitles.backfill?.let { b ->
                    val files = b.files ?: 0
                    if (files > 0) {
                        FormDivider()
                        InfoRow("Subtitles Read Ahead", "${b.done ?: 0} of $files" + if ((b.failed ?: 0) > 0) " · ${b.failed} failed" else "")
                    }
                }
                a.indexBackfill?.let { FormDivider(); InfoRow("Preparing Files", AdminRules.preparingText(it)) }
            }
            if (a.downloads.isNotEmpty()) {
                FormSection("Prepared Downloads") {
                    a.downloads.forEachIndexed { i, d ->
                        if (i > 0) FormDivider()
                        InfoRow(d.label, if (d.state == "running") "${(d.progress * 100).toInt()}%" else d.state.replaceFirstChar { it.uppercase() }, tint = if (d.state == "failed") Theme.destructive else null)
                    }
                }
            }
            FormSection("Recent Problems") {
                if (a.errors.isEmpty()) NoteRow("No warnings or errors since the server started.")
                a.errors.takeLast(12).reversed().forEachIndexed { i, line -> if (i > 0) FormDivider(); LogLineRow(line) }
            }
        }
        error?.let { NoteSection(it, Theme.destructive) }
    }
}

/** About This Server (`AboutServerView`): name, version, storage, the log, Restart Server. */
@Composable
fun AboutServerPage() {
    val api = Session.api ?: return
    val pages = LocalPages.current
    var system by remember { mutableStateOf<SystemInfo?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var confirmRestart by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    suspend fun load() { adminCall({ error = it }) { system = api.systemInfo(); error = null } }
    LaunchedEffect(Unit) { load() }
    FormPage("About This Server", pages::pop) {
        val s = system
        if (s != null) {
            FormSection("Server") {
                InfoRow("Name", s.name)
                FormDivider()
                InfoRow("Version", s.version)
                FormDivider()
                InfoRow("Running For", AdminFmt.uptime(s.uptimeSecs))
                FormDivider()
                InfoRow("Address", api.baseUrl.toString().removePrefix("http://").trimEnd('/'))
            }
            FormSection("Storage", footer = "The server's data (${AdminFmt.bytes(s.sizes.data)} in all) holds the library, accounts and watch history; the cache is refilled as needed.") {
                InfoRow("Database", AdminFmt.bytes(s.sizes.database))
                FormDivider()
                InfoRow("Subtitles", AdminFmt.bytes(s.sizes.subtitles))
                FormDivider()
                InfoRow("Artwork", AdminFmt.bytes(s.sizes.artwork))
                FormDivider()
                InfoRow("Stream Cache", "${AdminFmt.bytes(s.sizes.cache)} of ${AdminFmt.bytes(s.cacheMaxBytes)}")
                FormDivider()
                InfoRow("Prepared Downloads", "${AdminFmt.bytes(s.downloadsBytes)} of ${AdminFmt.bytes(s.downloadMaxBytes)}")
            }
            FormSection(footer = if (s.canRestart) "The log shows the server's recent lines, without keys or passwords." else "The log shows the server's recent lines, without keys or passwords. This server can't restart itself: restart it where it runs (docker restart moonrise-server).") {
                ManageLink("Server Log", Icons.Outlined.Description) { pages.push { ServerLogPage() } }
                if (s.canRestart) { FormDivider(); ActionRow("Restart Server", Icons.Outlined.RestartAlt) { confirmRestart = true } }
            }
        } else if (error == null) SpinnerSection()
        error?.let { NoteSection(it, Theme.destructive) }
    }
    RestartDialog(confirmRestart, scope, { confirmRestart = false }) { load() }
}

/** Server Log (`ServerLogView`): the last lines, newest first. */
@Composable
fun ServerLogPage() {
    val api = Session.api ?: return
    val pages = LocalPages.current
    var lines by remember { mutableStateOf<List<String>>(emptyList()) }
    var problemsOnly by remember { mutableStateOf(false) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(problemsOnly) {
        loading = true
        adminCall({ error = it }) { lines = api.serverLog(300, if (problemsOnly) "warn" else null); error = null }
        loading = false
    }
    FormPage("Server Log", pages::pop) {
        FormSection(footer = "The last ${lines.size} lines, newest first. Keys, passwords and tokens are left out.") {
            ToggleRow("Warnings and Errors Only", problemsOnly) { problemsOnly = it }
        }
        FormSection {
            if (loading && lines.isEmpty()) SpinnerRow()
            lines.reversed().forEachIndexed { i, l -> if (i > 0) FormDivider(); LogLineRow(l) }
        }
        error?.let { NoteSection(it, Theme.destructive) }
    }
}
