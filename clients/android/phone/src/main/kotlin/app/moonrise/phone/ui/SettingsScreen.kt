package app.moonrise.phone.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Logout
import androidx.compose.material.icons.outlined.SwapHoriz
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.Icon
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.material.icons.filled.UnfoldMore
import androidx.compose.material.icons.outlined.QrCode2
import androidx.compose.material.icons.outlined.Tune
import app.moonrise.core.Languages
import app.moonrise.core.Quality
import app.moonrise.core.SegmentKind
import app.moonrise.core.SkipMode
import app.moonrise.phone.BuildConfig
import app.moonrise.shared.AppSettings
import app.moonrise.shared.Session

/**
 * Settings (`SettingsView`, iPhone), a page over the tabs: the server (name, address, who is
 * signed in), Change Server and Sign Out; then this device's Playback, Skip Intros & Credits,
 * Audio and Subtitles; the version.
 */
@Composable
fun SettingsScreen(onClose: () -> Unit, signedOut: () -> Unit) {
    BackHandler(onBack = onClose)
    var confirm by remember { mutableStateOf<String?>(null) }
    val pages = remember { PageStack() }
    val api = Session.api
    var serverName by remember { mutableStateOf<String?>(null) }
    var user by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(api) {
        serverName = runCatching { api?.info()?.name }.getOrNull()
        user = runCatching { api?.me()?.name }.getOrNull()
        AdminState.refresh()
    }
    val admin = AdminState.isAdmin
    // Test seam (Dev app): opens Manage Server, or a screen of it.
    LaunchedEffect(admin, app.moonrise.phone.DebugSeams.manage) {
        val seam = app.moonrise.phone.DebugSeams.manage ?: return@LaunchedEffect
        if (!admin) return@LaunchedEffect
        app.moonrise.phone.DebugSeams.manage = null
        pages.popAll()
        pages.push { ManageServerPage(back = pages::pop) }
        when (seam) {
            "libraries" -> pages.push { LibrariesPage() }
            "review" -> pages.push { ReviewListPage() }
            "users" -> pages.push { AccountsPage() }
            "services" -> pages.push { ServicesPage() }
            "aimatching" -> pages.push { AIMatchingPage() }
            "playback" -> pages.push { PlaybackPage() }
            "activity" -> pages.push { ActivityPage() }
            "about" -> pages.push { AboutServerPage() }
            "log" -> pages.push { ServerLogPage() }
        }
    }
    androidx.compose.runtime.CompositionLocalProvider(LocalPages provides pages) {
    Box(Modifier.fillMaxSize().background(Color.Black)) {
    Column(Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = Theme.hPadding, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Spacer(Modifier.weight(1f))
            Text("Done", style = Type.headline, color = Color(0xFF0A84FF), modifier = Modifier.clickable(onClick = onClose).padding(6.dp))
        }
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = Theme.hPadding).padding(bottom = 40.dp),
            verticalArrangement = Arrangement.spacedBy(22.dp)) {
            Text("Settings", style = Type.largeTitle, color = Color.White)
            FormSection("Server", footer = if (admin) "Libraries, accounts, the server's services (TMDB, OpenSubtitles, AI matching), transcoding and preparing files are set in Manage Server, for everyone who uses this server. The settings below are for this device only." else null) {
                Value("Name", serverName ?: "—")
                FormDivider()
                Value("Address", api?.baseUrl?.toString()?.removePrefix("http://")?.trimEnd('/') ?: "—")
                FormDivider()
                Value("Signed In As", user ?: "—")
                FormDivider()
                FormRow(onClick = { pages.push { ApproveDevicePage(back = pages::pop) } }) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Row(Modifier.weight(1f)) { Action("Approve a Device", Icons.Outlined.QrCode2) }
                        Chevron()
                    }
                }
                FormDivider()
                if (admin) {
                    FormRow(onClick = { pages.push { ManageServerPage(back = pages::pop) } }) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Row(Modifier.weight(1f)) { Action("Manage Server", Icons.Outlined.Tune) }
                            Chevron()
                        }
                    }
                    FormDivider()
                }
                FormRow(onClick = { confirm = "change" }) { Action("Change Server", Icons.Outlined.SwapHoriz) }
                FormDivider()
                FormRow(onClick = { confirm = "signout" }) { Action("Sign Out", Icons.AutoMirrored.Outlined.Logout, destructive = true) }
            }
            FormSection("Playback", footer = "Titles start at this quality; you can pick another in the player.") {
                Choice("Streaming Quality", AppSettings.streamingQuality, Quality.all.map { it.label to it.label }) { AppSettings.chooseStreamingQuality(it) }
                FormDivider()
                Choice("Skip Back", "${AppSettings.skipBack} s", listOf(5, 10, 15, 30).map { "$it s" to "$it" }) { AppSettings.chooseSkipBack(it.toInt()) }
                FormDivider()
                Choice("Skip Forward", "${AppSettings.skipForward} s", listOf(10, 15, 30, 45, 60).map { "$it s" to "$it" }) { AppSettings.chooseSkipForward(it.toInt()) }
                FormDivider()
                Toggle("Play Next Episode Automatically", AppSettings.autoplayNext) { AppSettings.chooseAutoplayNext(it) }
            }
            FormSection("Skip Intros & Credits", footer = "Markers come from your server: the files' chapter names and the end credits it finds in them, and, with Online Databases on, the community databases TheIntroDB and IntroDB, which your server looks up. This device asks no one else.") {
                SegmentKind.entries.forEachIndexed { i, k ->
                    if (i > 0) FormDivider()
                    Choice(k.title, AppSettings.skipMode(k).title, SkipMode.entries.map { it.title to it.id }) { id -> SkipMode.of(id)?.let { AppSettings.chooseSkipMode(k, it) } }
                }
                FormDivider()
                Choice("Auto-Skip Countdown", if (AppSettings.autoSkipDelay == 0) "None" else "${AppSettings.autoSkipDelay} s",
                    listOf("None" to "0") + listOf(3, 5, 10).map { "$it s" to "$it" }) { AppSettings.chooseAutoSkipDelay(it.toInt()) }
                FormDivider()
                Toggle("Use Online Databases", AppSettings.onlineSegments) { AppSettings.chooseOnlineSegments(it) }
            }
            FormSection("Audio", footer = "Titles play in this language when they have it. Picking a language or subtitles in the player sets these.") {
                Choice("Preferred Language", languageName(AppSettings.audioLanguage, "Original"), listOf("Original" to "") + Languages.common.map { it.second to it.first }) {
                    AppSettings.chooseAudioLanguage(it)
                }
            }
            FormSection("Subtitles", footer = "How subtitles look on this device.") {
                Choice("Preferred Language", languageName(AppSettings.subtitleLanguage, "None"), Languages.common.map { it.second to it.first }) { AppSettings.chooseSubtitleLanguage(it) }
                FormDivider()
                Toggle("Turn On Automatically", AppSettings.autoSubtitles) { AppSettings.chooseAutoSubtitles(it) }
                FormDivider()
                Choice("Size", AppSettings.subtitleSize.replaceFirstChar { it.uppercase() }, listOf("small", "medium", "large", "huge").map { it.replaceFirstChar { c -> c.uppercase() } to it }) {
                    AppSettings.chooseSubtitleSize(it)
                }
                FormDivider()
                Toggle("Background Box", AppSettings.subtitleBackground) { AppSettings.chooseSubtitleBackground(it) }
            }
            FormSection("Downloads", footer = "Downloads play as soon as they start. Lower qualities are transcoded by your server.") {
                val cellular = androidx.compose.ui.platform.LocalContext.current.packageManager.hasSystemFeature(android.content.pm.PackageManager.FEATURE_TELEPHONY)
                Choice(if (cellular) "Quality on Wi-Fi" else "Default Quality", AppSettings.downloadQualityWiFi, Quality.all.map { it.label to it.label }) { AppSettings.chooseDownloadQualityWiFi(it) }
                if (cellular) {
                    FormDivider()
                    Choice("Quality on Cellular", AppSettings.downloadQualityCellular, Quality.all.map { it.label to it.label }) { AppSettings.chooseDownloadQualityCellular(it) }
                    FormDivider()
                    Toggle("Download over Cellular", AppSettings.allowCellularDownloads) { AppSettings.chooseAllowCellularDownloads(it) }
                }
            }
            FormSection {
                Value("Version", BuildConfig.VERSION_NAME)
            }
        }
    }
    pages.Host()
    }
    }
    val downloadsNote = if (app.moonrise.shared.downloads.Downloads.records.isNotEmpty()) " Your downloads stay on this device and play offline." else ""
    when (confirm) {
        "change" -> Confirm("Change Server?", "You'll be signed out and can sign in to another server.$downloadsNote", "Sign Out and Change", { confirm = null }) {
            confirm = null; Session.changeServer(); HomeStore.clear(); SearchStore.clear(); signedOut()
        }
        "signout" -> Confirm("Sign Out?", "This device stops using the server until you sign in again.$downloadsNote", "Sign Out", { confirm = null }) {
            confirm = null; Session.signOut(); HomeStore.clear(); SearchStore.clear(); signedOut()
        }
    }
}

private fun languageName(code: String, empty: String): String =
    if (code.isEmpty()) empty else Languages.common.firstOrNull { it.first == code }?.second ?: java.util.Locale(code).getDisplayLanguage(java.util.Locale.ENGLISH)

@Composable
private fun Value(label: String, value: String) {
    FormRow {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(label, style = Type.body, color = Color.White, modifier = Modifier.weight(1f))
            Text(value, style = Type.body, color = Theme.secondaryText)
        }
    }
}

@Composable
private fun Action(label: String, icon: androidx.compose.ui.graphics.vector.ImageVector, destructive: Boolean = false) {
    val c = if (destructive) Theme.destructive else Color.White
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Icon(icon, null, tint = c)
        Text(label, style = Type.body, color = c)
    }
}

/** A row with its value and a menu of the choices (the iPhone's menu picker). */
@Composable
private fun Choice(label: String, value: String, options: List<Pair<String, String>>, pick: (String) -> Unit) {
    var open by remember { mutableStateOf(false) }
    FormRow(onClick = { open = true }) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(label, style = Type.body, color = Color.White, modifier = Modifier.weight(1f))
            Box {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(value, style = Type.body, color = Theme.secondaryText)
                    Icon(Icons.Filled.UnfoldMore, null, tint = Theme.secondaryText, modifier = Modifier.padding(start = 4.dp))
                }
                DropdownMenu(open, { open = false }, containerColor = Theme.sheet) {
                    for ((title, v) in options) MenuItem(title, checked = title == value) { open = false; pick(v) }
                }
            }
        }
    }
}

@Composable
private fun Toggle(label: String, on: Boolean, set: (Boolean) -> Unit) {
    FormRow(onClick = { set(!on) }) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(label, style = Type.body, color = Color.White, modifier = Modifier.weight(1f))
            Switch(on, set, modifier = Modifier.height(24.dp), colors = SwitchDefaults.colors(checkedTrackColor = Theme.green, checkedThumbColor = Color.White))
        }
    }
}

@Composable
private fun Confirm(title: String, text: String, action: String, dismiss: () -> Unit, confirm: () -> Unit) {
    AlertDialog(onDismissRequest = dismiss, containerColor = Theme.sheet, title = { Text(title, fontWeight = FontWeight.SemiBold) }, text = { Text(text) },
        confirmButton = { TextButton(onClick = confirm) { Text(action, color = Theme.destructive) } },
        dismissButton = { TextButton(onClick = dismiss) { Text("Cancel", color = Color.White) } })
}
