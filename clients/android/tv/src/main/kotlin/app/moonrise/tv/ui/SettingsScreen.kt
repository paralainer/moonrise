package app.moonrise.tv.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.Surface
import androidx.tv.material3.Text
import app.moonrise.core.Languages
import app.moonrise.core.RATINGS_ATTRIBUTION
import app.moonrise.core.Quality
import app.moonrise.core.SegmentKind
import app.moonrise.core.SkipMode
import app.moonrise.tv.R
import app.moonrise.shared.Session
import app.moonrise.shared.AppSettings
import app.moonrise.shared.Network
import app.moonrise.shared.downloads.ServerReach
import app.moonrise.shared.remote.Tailscale
import app.moonrise.core.Route
import app.moonrise.core.TailscaleState

/**
 * Settings, laid out as on Apple TV: the app's mark, version and connection on the left; on the
 * right the server (who is signed in, how it's reached; Change Server and Sign Out, both of which
 * sign out), then this device's playback, audio and subtitle preferences.
 */
@Composable
fun SettingsScreen(
    signOut: () -> Unit, changeServer: () -> Unit, atTop: (Boolean) -> Unit,
    entry: androidx.compose.ui.focus.FocusRequester? = null, main: MainState? = null,
) {
    val context = LocalContext.current
    val api = Session.api
    var serverName by remember { mutableStateOf<String?>(null) }
    var userName by remember { mutableStateOf<String?>(null) }
    var picker by remember { mutableStateOf<Picker?>(null) }
    var confirm by remember { mutableStateOf<String?>(null) }
    val list = rememberLazyListState()
    // Down from the tab bar, when the focus search finds nothing below: the first row, scrolled back.
    EnterContent(main) {
        runCatching { list.scrollToItem(0) }
        entry?.focusSoon(20) ?: false
    }
    LaunchedEffect(list) { snapshotFlow { list.firstVisibleItemIndex == 0 && list.firstVisibleItemScrollOffset < 40 }.collect(atTop) }
    // The route check says how the server is reached (home, the tailnet, or not at all).
    val route = ServerReach.route
    val reachable: Boolean? = when { route == null || route == Route.Checking -> null; else -> route.usable }
    LaunchedEffect(Network.generation, reachable) {
        val a = api ?: return@LaunchedEffect
        if (reachable != true) return@LaunchedEffect
        runCatching { serverName = a.info().name }
        runCatching { userName = a.me().name }
    }
    val version = remember { runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull() ?: "" }
    val address = Session.homeAddress?.let { "${it.host}:${it.port}" } ?: "—"
    val connection = route?.label ?: "Checking…"
    val name = serverName ?: "Moonrise Server"

    Row(Modifier.fillMaxSize().padding(top = TAB_BAR_SPACE)) {
        // Left pane: the mark, the version and the server's status.
        Column(
            Modifier.width(260.dp).fillMaxHeight().padding(start = Theme.margin, end = 20.dp),
            verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Image(painterResource(R.drawable.moonrise_mark), contentDescription = null, modifier = Modifier.size(150.dp))
            Spacer(Modifier.height(13.dp))
            Text("Moonrise", fontSize = 22.sp, fontWeight = FontWeight.Bold)
            Text("Version $version", fontSize = 13.sp, color = Theme.secondaryText)
            Spacer(Modifier.height(13.dp))
            Row(
                Modifier.fillMaxWidth().background(Color.White.copy(alpha = 0.06f), RoundedCornerShape(12.dp)).padding(12.dp),
                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Box(Modifier.size(7.dp).background(when (reachable) { null -> Color(0xFFFFD60A); true -> Color(0xFF30D158); false -> Color(0xFFFF453A) }, CircleShape))
                Column {
                    Text(name, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(connection, fontSize = 11.sp, color = Theme.secondaryText)
                }
            }
            Spacer(Modifier.height(13.dp))
            Text(RATINGS_ATTRIBUTION, fontSize = 9.sp, lineHeight = 12.sp, color = Theme.secondaryText, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
        }
        @OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
        androidx.compose.runtime.CompositionLocalProvider(androidx.compose.foundation.gestures.LocalBringIntoViewSpec provides MinimalScroll) {
        LazyColumn(
            state = list,
            modifier = Modifier.weight(1f).fillMaxHeight(),
            contentPadding = PaddingValues(start = 20.dp, end = Theme.margin, top = 6.dp, bottom = 40.dp),
            verticalArrangement = Arrangement.spacedBy(3.dp),
        ) {
            item { Header("Server") }
            item { InfoRow("Name", name) }
            item { InfoRow("Address", address) }
            item { InfoRow("Signed In As", userName ?: "—") }
            item { InfoRow("Connection", connection) }
            item { ActionRow("Change Server", modifier = entry?.let { Modifier.focusRequester(it) } ?: Modifier) { confirm = "change" } }
            item { ActionRow("Sign Out", destructive = true) { confirm = "signout" } }

            if (Tailscale.isSupported) remoteAccess()

            item { Header("Playback") }
            item { ValueRow("Streaming Quality", AppSettings.streamingQuality) { picker = Picker.Quality } }
            // The note is about the quality, so it sits right under it.
            item { Footer("Titles start at this quality; you can pick another in the player.", bottom = 6.dp) }
            item { ValueRow("Skip Back", "${AppSettings.skipBack} s") { picker = Picker.SkipBack } }
            item { ValueRow("Skip Forward", "${AppSettings.skipForward} s") { picker = Picker.SkipForward } }
            item { ToggleRow("Play Next Episode Automatically", AppSettings.autoplayNext) { AppSettings.chooseAutoplayNext(!AppSettings.autoplayNext) } }

            item { Header("Skip Intros & Credits") }
            SegmentKind.entries.forEach { kind ->
                item(key = "skip-${kind.id}") { ValueRow(kind.title, AppSettings.skipMode(kind).title) { picker = Picker.Skip(kind) } }
            }
            item { ValueRow("Auto-Skip Countdown", if (AppSettings.autoSkipDelay == 0) "None" else "${AppSettings.autoSkipDelay} s") { picker = Picker.Countdown } }
            item { ToggleRow("Use Online Databases", AppSettings.onlineSegments) { AppSettings.chooseOnlineSegments(!AppSettings.onlineSegments) } }
            item { Footer("Markers come from your server: the files' chapter names and the end credits it finds in them, and, with Online Databases on, the community databases TheIntroDB and IntroDB, which your server looks up. This device asks no one else.") }

            item { Header("Audio") }
            item { ValueRow("Preferred Language", languageName(AppSettings.audioLanguage, "Original")) { picker = Picker.Audio } }
            item { Footer("Titles play in this language when they have it.") }

            item { Header("Subtitles") }
            item { ValueRow("Preferred Language", languageName(AppSettings.subtitleLanguage, "None")) { picker = Picker.Subtitles } }
            item { ToggleRow("Turn On Automatically", AppSettings.autoSubtitles) { AppSettings.chooseAutoSubtitles(!AppSettings.autoSubtitles) } }
            item { ValueRow("Size", AppSettings.subtitleSize.replaceFirstChar { it.uppercase() }) { picker = Picker.SubtitleSize } }
            item { ToggleRow("Background Box", AppSettings.subtitleBackground) { AppSettings.chooseSubtitleBackground(!AppSettings.subtitleBackground) } }
            item { Footer("How subtitles look on this device.") }
        }
        }
    }

    when (val p = picker) {
        null -> {}
        is Picker.Skip -> MenuDialog(p.kind.title, onDismiss = { picker = null }, sections = listOf(MenuSection(rows =
            SkipMode.entries.map { m -> MenuRow(m.title, checked = AppSettings.skipMode(p.kind) == m) { AppSettings.chooseSkipMode(p.kind, m) } })))
        Picker.Countdown -> MenuDialog("Auto-Skip Countdown", onDismiss = { picker = null }, width = 260.dp, sections = listOf(MenuSection(rows =
            listOf(0, 3, 5, 10).map { s -> MenuRow(if (s == 0) "None" else "$s s", checked = s == AppSettings.autoSkipDelay) { AppSettings.chooseAutoSkipDelay(s) } })))
        Picker.SubtitleSize -> MenuDialog("Size", onDismiss = { picker = null }, width = 240.dp, sections = listOf(MenuSection(rows =
            listOf("small", "medium", "large", "huge").map { z -> MenuRow(z.replaceFirstChar { it.uppercase() }, checked = z == AppSettings.subtitleSize) { AppSettings.chooseSubtitleSize(z) } })))
        Picker.Quality -> MenuDialog("Streaming Quality", onDismiss = { picker = null }, sections = listOf(MenuSection(rows =
            Quality.all.map { q -> MenuRow(q.label, checked = q.label == AppSettings.streamingQuality) { AppSettings.chooseStreamingQuality(q.label) } })))
        Picker.SkipBack -> MenuDialog("Skip Back", onDismiss = { picker = null }, width = 240.dp, sections = listOf(MenuSection(rows =
            listOf(5, 10, 15, 30).map { s -> MenuRow("$s s", checked = s == AppSettings.skipBack) { AppSettings.chooseSkipBack(s) } })))
        Picker.SkipForward -> MenuDialog("Skip Forward", onDismiss = { picker = null }, width = 240.dp, sections = listOf(MenuSection(rows =
            listOf(10, 15, 30, 45, 60).map { s -> MenuRow("$s s", checked = s == AppSettings.skipForward) { AppSettings.chooseSkipForward(s) } })))
        Picker.Audio -> MenuDialog("Preferred Language", onDismiss = { picker = null }, sections = listOf(MenuSection(rows =
            listOf(MenuRow("Original", checked = AppSettings.audioLanguage.isEmpty()) { AppSettings.chooseAudioLanguage("") }) +
                languages(AppSettings.audioLanguage).map { (code, n) -> MenuRow(n, checked = code == AppSettings.audioLanguage) { AppSettings.chooseAudioLanguage(code) } })))
        Picker.Subtitles -> MenuDialog("Preferred Language", onDismiss = { picker = null }, sections = listOf(MenuSection(rows =
            languages(AppSettings.subtitleLanguage).map { (code, n) -> MenuRow(n, checked = code == AppSettings.subtitleLanguage) { AppSettings.chooseSubtitleLanguage(code) } })))
    }
    when (confirm) {
        "change" -> ConfirmDialog("Change server?", "You'll be signed out of $name and can sign in to another server.", "Sign Out and Change",
            onConfirm = changeServer, onDismiss = { confirm = null })
        "signout" -> ConfirmDialog("Sign out of $name?", "This device stops using $name until you sign in again.", "Sign Out",
            onConfirm = signOut, onDismiss = { confirm = null })
    }
}

/**
 * Remote Access: Connect Through Tailscale; the sign-in as a QR code and its address (a
 * TV has no browser to sign in with), the node's state and Sign Out of Tailscale.
 */
private fun androidx.compose.foundation.lazy.LazyListScope.remoteAccess() {
    item { Header("Remote Access") }
    item { ToggleRow("Connect Through Tailscale", Tailscale.enabled) { Tailscale.enabled = !Tailscale.enabled } }
    item(key = "tailscale-state") {
        if (!Tailscale.enabled) return@item
        Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
            when (val st = Tailscale.state) {
                TailscaleState.Off, TailscaleState.Starting -> InfoRow("Status", "Starting…")
                is TailscaleState.NeedsLogin -> {
                    val url = st.url
                    if (url == null) InfoRow("Status", "Preparing sign-in…") else TailscaleSignIn(url, Modifier.padding(vertical = 6.dp))
                }
                is TailscaleState.Running -> {
                    InfoRow("Status", "Connected · ${st.ip}")
                    ActionRow("Sign Out of Tailscale", destructive = true) { Tailscale.signOut() }
                }
                is TailscaleState.Failed -> {
                    InfoRow("Status", st.message)
                    ActionRow("Retry") { Tailscale.restart() }
                }
            }
        }
    }
    item { Footer("Moonrise joins your tailnet as its own device, so your home server is reachable away from home without the Tailscale app or a VPN. At home it uses the local network directly.") }
}

private sealed interface Picker {
    data object Quality : Picker
    data object SkipBack : Picker
    data object SkipForward : Picker
    data object Audio : Picker
    data object Subtitles : Picker
    data object Countdown : Picker
    data object SubtitleSize : Picker
    data class Skip(val kind: SegmentKind) : Picker
}

private fun languages(current: String): List<Pair<String, String>> =
    if (current.isEmpty() || Languages.common.any { it.first == current }) Languages.common
    else Languages.common + (current to java.util.Locale(current).getDisplayLanguage(java.util.Locale.ENGLISH))

private fun languageName(code: String, empty: String): String =
    if (code.isEmpty()) empty else Languages.common.firstOrNull { it.first == code }?.second ?: java.util.Locale(code).getDisplayLanguage(java.util.Locale.ENGLISH)

@Composable
private fun Header(text: String) {
    Text(text.uppercase(), fontSize = 11.sp, fontWeight = FontWeight.SemiBold, color = Theme.secondaryText,
        modifier = Modifier.padding(start = 14.dp, top = 16.dp, bottom = 4.dp))
}

@Composable
private fun Footer(text: String, bottom: androidx.compose.ui.unit.Dp = 0.dp) {
    Text(text, fontSize = 11.sp, color = Theme.secondaryText, modifier = Modifier.padding(start = 14.dp, top = 2.dp, bottom = bottom))
}

/** A fact (not focusable): the label, its value on the right. */
@Composable
private fun InfoRow(label: String, value: String) {
    Row(
        Modifier.fillMaxWidth().background(Color.White.copy(alpha = 0.05f), RoundedCornerShape(50)).padding(horizontal = 14.dp, vertical = 9.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, fontSize = 14.sp, modifier = Modifier.weight(1f))
        Text(value, fontSize = 14.sp, color = Theme.secondaryText, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
private fun ValueRow(label: String, value: String, chevron: Boolean = true, onClick: () -> Unit) {
    SettingsSurface(onClick, name = label) {
        Text(label, fontSize = 14.sp, modifier = Modifier.weight(1f))
        Text(value, fontSize = 14.sp, color = androidx.tv.material3.LocalContentColor.current.copy(alpha = 0.6f), maxLines = 1)
        if (chevron) Text("  ›", fontSize = 15.sp, color = androidx.tv.material3.LocalContentColor.current.copy(alpha = 0.6f))
    }
}

/** An on/off setting: select switches it. */
@Composable
private fun ToggleRow(label: String, on: Boolean, onClick: () -> Unit) = ValueRow(label, if (on) "On" else "Off", chevron = false, onClick = onClick)

@Composable
private fun ActionRow(label: String, destructive: Boolean = false, modifier: Modifier = Modifier, onClick: () -> Unit) {
    SettingsSurface(onClick, modifier, name = label) {
        Text(label, fontSize = 14.sp, color = if (destructive) Color(0xFFFF453A) else androidx.tv.material3.LocalContentColor.current)
    }
}

@Composable
private fun SettingsSurface(onClick: () -> Unit, modifier: Modifier = Modifier, name: String, content: @Composable androidx.compose.foundation.layout.RowScope.() -> Unit) {
    FocusSurface(
        onClick = onClick,
        modifier = modifier.fillMaxWidth().logFocus { "settings: $name" },
        shape = RoundedCornerShape(50),
        focusedScale = 1.02f,
        containerColor = Color.White.copy(alpha = 0.08f), contentColor = Color.White,
            focusedContainerColor = Color.White, focusedContentColor = Color.Black,
    ) {
        Row(Modifier.padding(horizontal = 14.dp, vertical = 9.dp), verticalAlignment = Alignment.CenterVertically, content = content)
    }
}
