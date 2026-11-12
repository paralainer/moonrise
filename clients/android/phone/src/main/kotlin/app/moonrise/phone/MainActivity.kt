package app.moonrise.phone

import android.app.Application
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowCircleDown
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.VideoLibrary
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.moonrise.core.LayoutMetrics
import app.moonrise.phone.ui.CategoryScreen
import app.moonrise.phone.ui.GlassCircle
import app.moonrise.phone.ui.LocalMetrics
import app.moonrise.phone.ui.PadLibraryScreen
import app.moonrise.phone.ui.HomeScreen
import app.moonrise.phone.ui.LibraryScreen
import app.moonrise.phone.ui.MoonriseTheme
import app.moonrise.phone.ui.OnboardingScreen
import app.moonrise.phone.ui.SearchBar
import app.moonrise.phone.ui.SearchScreen
import app.moonrise.phone.ui.SettingsScreen
import app.moonrise.phone.ui.Theme
import app.moonrise.phone.ui.TitleScreen
import app.moonrise.shared.AppSettings
import app.moonrise.shared.Log
import app.moonrise.shared.Network
import kotlinx.coroutines.launch
import app.moonrise.shared.Session
import app.moonrise.core.Quality
import app.moonrise.shared.downloads.Downloads
import app.moonrise.shared.downloads.LocalReports
import app.moonrise.shared.downloads.ServerReach
import app.moonrise.phone.ui.DownloadsScreen
import app.moonrise.phone.ui.AdminState
import app.moonrise.phone.ui.FixMatchHost

class PhoneApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        Session.init(this)
        AppSettings.init(this)
        Network.init(this)
        Downloads.init(this)
        Downloads.onWorkChanged = { DownloadService.sync(this) }
    }
}

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        // Light status and navigation bar content over the dark pages (the iPhone app is dark).
        enableEdgeToEdge(SystemBarStyle.dark(android.graphics.Color.TRANSPARENT), SystemBarStyle.dark(android.graphics.Color.TRANSPARENT))
        super.onCreate(savedInstanceState)
        DebugSeams.take(this, intent)
        setContent { MoonriseTheme { App() } }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        DebugSeams.take(this, intent)
    }
}

/**
 * Test seams, debug builds only (the Dev app): `am start -n app.moonrise.phone.dev/app.moonrise.phone.MainActivity
 * --es open <item id>` opens that title's page on Home; `--es play <item id> [--es start <seconds>]` opens it
 * and plays (as Play on its page); `--es tab home|library|search` switches tab. Add
 * `--activity-single-top` while the app runs. `--ez awake true` keeps the screen on while the app is
 * in front (scripted checks on a test phone, without changing its settings). Downloads:
 * `--es autodownload <item id>[:<bits/s>]` downloads it (0 or none: Original), `--es playdownload
 * <item id>` opens its title page in Downloads, playing the download (offline too),
 * `--es deletedownload <item id>` deletes it. `--ez noreport true`: watch reports stop here.
 * `--es manage libraries|review|users|services|aimatching|playback|activity|about|log` opens Manage Server there; `fix:<item id>` Fix Match.
 * `--es orient landscape|portrait|auto` asks for the screen's orientation (the tablet layout's rotation, without touching a setting).
 */
object DebugSeams {
    var open by mutableStateOf<Pair<String, Autoplay?>?>(null)
    var tab by mutableStateOf<Tab?>(null)
    var playDownload by mutableStateOf<String?>(null)
    var manage by mutableStateOf<String?>(null)

    fun take(activity: ComponentActivity, intent: Intent?) {
        if (activity.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE == 0 || intent == null) return
        if (intent.getBooleanExtra("awake", false)) {
            activity.window.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            intent.removeExtra("awake")
        }
        if (intent.getBooleanExtra("noreport", false)) { LocalReports.disabled = true; intent.removeExtra("noreport") }
        intent.getStringExtra("orient")?.let { o ->
            intent.removeExtra("orient")
            activity.requestedOrientation = when (o) {
                "landscape" -> android.content.pm.ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
                "portrait" -> android.content.pm.ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
                else -> android.content.pm.ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
            }
        }
        intent.getStringExtra("autodownload")?.let { spec ->
            intent.removeExtra("autodownload")
            val id = spec.substringBefore(':')
            val bits = spec.substringAfter(':', "0").toLongOrNull() ?: 0
            Downloads.forItem(id)?.let { old -> Downloads.delete(old.id) }
            Session.background.launch {
                val item = Session.api?.item(id) ?: return@launch
                Downloads.download(item, Quality.all.firstOrNull { it.bitrate == bits } ?: Quality.original, item.presentFiles.firstOrNull())
            }
        }
        intent.getStringExtra("deletedownload")?.let { id -> intent.removeExtra("deletedownload"); Downloads.forItem(id)?.let { Downloads.delete(it.id) } }
        intent.getStringExtra("playdownload")?.let { playDownload = it; intent.removeExtra("playdownload") }
        intent.getStringExtra("manage")?.let { manage = it; intent.removeExtra("manage") }
        intent.getStringExtra("tab")?.let { t -> tab = Tab.entries.firstOrNull { it.name.equals(t, true) }; intent.removeExtra("tab") }
        intent.getStringExtra("open")?.let { id -> open = id to null; intent.removeExtra("open") }
        intent.getStringExtra("play")?.let { id ->
            open = id to Autoplay(intent.getStringExtra("start")?.toDoubleOrNull())
            Log.i("Moonrise") { "test seam: asked to play $id" }
            intent.removeExtra("play")
        }
    }
}

@Composable
private fun App() {
    // The window's real size (the app draws under the bars): a tablet's layout comes from it.
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val smallest = LocalConfiguration.current.smallestScreenWidthDp
        val metrics = remember(maxWidth, maxHeight, smallest) { LayoutMetrics.of(maxWidth.value.toDouble(), maxHeight.value.toDouble(), smallest.toDouble()) }
        CompositionLocalProvider(LocalMetrics provides metrics) { AppContent() }
    }
}

@Composable
private fun AppContent() {
    var signedIn by remember { mutableStateOf(Session.api != null) }
    val nav = remember { AppNav() }
    if (!signedIn) {
        OnboardingScreen(onSignedIn = { nav.reset(); ServerReach.signedIn(); signedIn = true })
        return
    }
    fun signedOut() { nav.reset(); signedIn = false }

    // Admins get Manage Server and Fix Match.
    LaunchedEffect(signedIn) { AdminState.refresh() }
    LaunchedEffect(DebugSeams.open) {
        val (id, autoplay) = DebugSeams.open ?: return@LaunchedEffect
        try {
            val item = runCatching { Session.api?.item(id) }.onFailure { Log.w("Moonrise", "can't open $id: ${it.message}") }.getOrNull() ?: return@LaunchedEffect
            nav.stacks.getValue(Tab.Home).clear()
            nav.openOnHome(Route.Title(item, autoplay))
        } finally {
            DebugSeams.open = null
        }
    }
    LaunchedEffect(DebugSeams.playDownload) {
        val id = DebugSeams.playDownload ?: return@LaunchedEffect
        DebugSeams.playDownload = null
        Downloads.playable(id)?.let { r -> nav.tab = Tab.Downloads; nav.stacks.getValue(Tab.Downloads).add(Route.Title(r.pageItem, Autoplay())) }
    }
    // The server doesn't answer at launch: open on the downloads, they play.
    var launchChecked by remember { mutableStateOf(false) }
    LaunchedEffect(ServerReach.checked) {
        if (ServerReach.checked && !launchChecked) {
            launchChecked = true
            if (!ServerReach.reachable && Downloads.hasPlayable && nav.tab == Tab.Home && nav.stack.isEmpty()) nav.tab = Tab.Downloads
        }
    }
    // Test seam: `--es manage <screen>` opens Settings → Manage Server (→ that screen); `fix:<id>` opens Fix Match.
    LaunchedEffect(DebugSeams.manage) {
        val v = DebugSeams.manage ?: return@LaunchedEffect
        if (v.startsWith("fix:")) { DebugSeams.manage = null; app.moonrise.phone.ui.FixMatchPresenter.present(v.removePrefix("fix:")) } else nav.settings = true
    }
    LaunchedEffect(DebugSeams.tab) { DebugSeams.tab?.let { nav.tab = it; DebugSeams.tab = null } }

    BackHandler(enabled = nav.settings || nav.stack.isNotEmpty() || nav.tab != Tab.Home) { nav.back() }

    val saveable = rememberSaveableStateHolder()
    val metrics = LocalMetrics.current
    CompositionLocalProvider(LocalNav provides nav) {
        Box(Modifier.fillMaxSize().background(Theme.background)) {
            val top = nav.top
            // A tablet's Library keeps its sidebar under its pages (a title's page covers the tab).
            val sidebar = metrics.regular && nav.tab == Tab.Library && top !is Route.Title
            // Each page keeps its scroll position while others are on top.
            key(nav.tab, if (sidebar) "sidebar" else top?.key) {
                saveable.SaveableStateProvider("${nav.tab}-${if (sidebar) "sidebar" else top?.key ?: "root"}") {
                    if (sidebar) PadLibraryScreen() else when (top) {
                        null -> when (nav.tab) {
                            Tab.Home -> HomeScreen(signOut = ::signedOut)
                            Tab.Library -> LibraryScreen(null)
                            Tab.Downloads -> DownloadsScreen()
                            Tab.Search -> SearchScreen()
                        }
                        is Route.Title -> TitleScreen(top, signOut = ::signedOut)
                        is Route.Category -> CategoryScreen(top)
                        is Route.Filtered -> LibraryScreen(top)
                    }
                }
            }
            AnimatedVisibility(!nav.fullScreen, enter = fadeIn(), exit = fadeOut(), modifier = Modifier.align(if (metrics.regular) Alignment.TopCenter else Alignment.BottomCenter)) {
                if (metrics.regular) PadTopBar(nav)
                // The search tab: its field in the tab bar's place.
                else if (nav.tab == Tab.Search && nav.stack.isEmpty()) SearchBar(home = { nav.tab = Tab.Home }) else TabBar(nav)
            }
            if (nav.settings) {
                val settings: @Composable () -> Unit = { SettingsScreen(onClose = { nav.settings = false }, signedOut = ::signedOut) }
                if (metrics.regular) SettingsSheet(onDismiss = { nav.settings = false }, content = settings) else settings()
            }
            FixMatchHost()
        }
    }
}

/**
 * The tab bar floating over the page, as on the iPhone: Home and Library in a glass capsule (the
 * tab in use on a lighter pill), Search in a glass circle of its own at the end. A tab picked
 * again goes back to its first page.
 */
@Composable
private fun TabBar(nav: AppNav) {
    Row(Modifier.fillMaxWidth().navigationBarsPadding().padding(start = 21.dp, end = 21.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween) {
        Row(Modifier.height(62.dp).clip(CircleShape).background(Theme.bar).border(0.5.dp, Color.White.copy(alpha = 0.16f), CircleShape).padding(4.dp)) {
            TabButton(nav, Tab.Home, Icons.Filled.Home)
            TabButton(nav, Tab.Library, Icons.Filled.VideoLibrary)
            TabButton(nav, Tab.Downloads, Icons.Filled.ArrowCircleDown)
        }
        Box(Modifier.size(62.dp).clip(CircleShape).background(if (nav.tab == Tab.Search) Color(0xFF3A3C44) else Theme.bar)
            .border(0.5.dp, Color.White.copy(alpha = 0.16f), CircleShape).clickable { pick(nav, Tab.Search) }, contentAlignment = Alignment.Center) {
            Icon(Icons.Filled.Search, "Search", tint = Color.White, modifier = Modifier.size(28.dp))
        }
    }
}

/** A tablet's Settings: a centred sheet over the page (the iPad's form sheet), not the whole screen. */
@Composable
private fun SettingsSheet(onDismiss: () -> Unit, content: @Composable () -> Unit) {
    val metrics = LocalMetrics.current
    val none = remember { MutableInteractionSource() }
    Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.5f)).clickable(none, null, onClick = onDismiss), contentAlignment = Alignment.Center) {
        Box(Modifier.width(580.dp).height(minOf(760.0, metrics.height - 80).dp).clip(RoundedCornerShape(26.dp)).clickable(none, null) {}.consumeWindowInsets(WindowInsets.systemBars)) { content() }
    }
}

/**
 * A tablet's bar at the top, as on the iPad: the tabs by name in a glass capsule (Search as its
 * symbol at the end), the gear at the top trailing corner on every tab's first page.
 */
@Composable
private fun PadTopBar(nav: AppNav) {
    Box(Modifier.fillMaxWidth().statusBarsPadding().padding(top = 8.dp)) {
        Row(Modifier.align(Alignment.Center).height(44.dp).clip(CircleShape).background(Theme.bar).border(0.5.dp, Color.White.copy(alpha = 0.16f), CircleShape).padding(4.dp)) {
            for (tab in listOf(Tab.Home, Tab.Library, Tab.Downloads, Tab.Search)) {
                val on = nav.tab == tab
                Box(Modifier.height(36.dp).clip(CircleShape).background(if (on) Color.White.copy(alpha = 0.14f) else Color.Transparent).clickable { pick(nav, tab) }.padding(horizontal = 15.dp), contentAlignment = Alignment.Center) {
                    if (tab == Tab.Search) Icon(Icons.Filled.Search, "Search", tint = Color.White, modifier = Modifier.size(22.dp))
                    else Text(tab.title, fontSize = 17.sp, fontWeight = FontWeight.Medium, color = Color.White, maxLines = 1)
                }
            }
        }
        // Beside the Library's sidebar a category or a tile's titles still count as the tab's first page.
        if ((nav.stack.isEmpty() || (nav.tab == Tab.Library && nav.top !is Route.Title)) && !nav.settings) GlassCircle(Icons.Outlined.Settings, "Settings", Modifier.align(Alignment.CenterEnd).padding(end = 10.dp)) { nav.settings = true }
    }
}

private fun pick(nav: AppNav, tab: Tab) {
    if (nav.tab == tab) nav.stack.clear() else nav.tab = tab
}

@Composable
private fun TabButton(nav: AppNav, tab: Tab, icon: androidx.compose.ui.graphics.vector.ImageVector) {
    val on = nav.tab == tab
    val width = minOf(96, (LocalConfiguration.current.screenWidthDp - 42 - 62 - 8 - 12) / 3).dp
    Column(Modifier.width(width).fillMaxSize().clip(CircleShape).background(if (on) Color.White.copy(alpha = 0.14f) else Color.Transparent)
        .clickable { pick(nav, tab) }, horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
        Icon(icon, null, tint = Color.White, modifier = Modifier.size(26.dp))
        Text(tab.title, fontSize = 11.sp, fontWeight = FontWeight.Medium, color = Color.White)
    }
}
