package app.moonrise.tv

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Modifier
import app.moonrise.core.Item
import app.moonrise.tv.ui.DetailsScreen
import app.moonrise.tv.ui.MainScreen
import app.moonrise.tv.ui.MainState
import app.moonrise.tv.ui.MoonriseTheme
import app.moonrise.tv.ui.PlayerScreen
import app.moonrise.tv.ui.ScreenEnter
import app.moonrise.tv.ui.SignInScreen
import app.moonrise.tv.ui.Theme
import app.moonrise.shared.DetailsState
import app.moonrise.shared.Log
import app.moonrise.shared.PlayerSeams
import app.moonrise.shared.Session

/** The app's screens, as a stack: Back pops one. */
sealed interface Screen {
    data object SignIn : Screen
    /** The tabs: Home, Library, Search, Settings. */
    data object Main : Screen
    data class Details(val item: Item) : Screen {
        /** The page's content, kept while the player is on top. */
        val state = DetailsState(item)
    }
    /** `start`: seconds to start at (a resume position; 0 from the beginning), else where the server says. */
    data class Player(val item: Item, val start: Double?, val version: String?) : Screen
}

class MainActivity : ComponentActivity() {
    private fun take(intent: android.content.Intent?) {
        DeepLinks.take(intent)?.let { id ->
            Log.i("Moonrise") { "link: play $id" }
            DebugSeams.play = id to null
            return
        }
        DebugSeams.take(this, intent)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        take(intent)
        setContent { MoonriseTheme { App() } }
    }

    override fun onNewIntent(intent: android.content.Intent) {
        super.onNewIntent(intent)
        take(intent)
    }
}

/**
 * Test seam, debug builds only (the Dev app): `am start -n app.moonrise.tv.dev/app.moonrise.tv.MainActivity --es play <item id>
 * [--es start <seconds>]` opens that title's page and plays it, and `--es open <item id>` opens the
 * page alone (for scripted checks without walking the screens with the remote); `--ez novideo
 * true` leaves the picture out, so screenshots show the controls. Add
 * `--activity-single-top` while the app runs. `--ez watchnext true` logs the home screen's Watch Next
 * rows as the TV provider holds them (`MoonriseWatchNext`).
 */
object DebugSeams {
    var play by androidx.compose.runtime.mutableStateOf<Pair<String, Double?>?>(null)
    /**
     * `--ez novideo true` with `play`: the picture isn't drawn (the player's controls, panel and
     * menus can then be captured with `screencap`, which shows nothing while video is drawn).
     */
    var noVideo = false
    // `--ez refuseoriginal true`, `--ez lossless true` and `--ez dv true` (each until `false`) set
    // the player's seams ([PlayerSeams]): no video decoder for the Original quality, lossless
    // multichannel audio, and Dolby Vision when HDR is on.
    /** `--es open <item id>`: that title's page only. */
    var open by androidx.compose.runtime.mutableStateOf<String?>(null)

    fun take(activity: ComponentActivity, intent: android.content.Intent?) {
        val debuggable = activity.applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE != 0
        if (!debuggable) return
        if (intent?.hasExtra("refuseoriginal") == true) {
            PlayerSeams.refuseOriginalVideo = intent.getBooleanExtra("refuseoriginal", false)
            Log.i("Moonrise") { "test seam: refuse the Original quality's video: ${PlayerSeams.refuseOriginalVideo}" }
            intent.removeExtra("refuseoriginal")
        }
        if (intent?.hasExtra("lossless") == true) {
            PlayerSeams.lossless = intent.getBooleanExtra("lossless", false)
            Log.i("Moonrise") { "test seam: ask for lossless audio: ${PlayerSeams.lossless}" }
            intent.removeExtra("lossless")
        }
        if (intent?.hasExtra("dv") == true) {
            PlayerSeams.dolbyVision = intent.getBooleanExtra("dv", false)
            Log.i("Moonrise") { "test seam: ask for Dolby Vision: ${PlayerSeams.dolbyVision}" }
            intent.removeExtra("dv")
        }
        if (intent?.getBooleanExtra("watchnext", false) == true) { intent.removeExtra("watchnext"); WatchNext.dump(); return }
        intent?.getStringExtra("open")?.let { id -> open = id; intent.removeExtra("open"); return }
        val id = intent?.getStringExtra("play") ?: return
        noVideo = intent.getBooleanExtra("novideo", false)
        play = id to intent.getStringExtra("start")?.toDoubleOrNull()
        Log.i("Moonrise") { "test seam: asked to play $id" }
        intent.removeExtra("play")
    }
}

@Composable
private fun App() {
    val stack = remember { mutableStateListOf<Screen>(if (Session.api != null) Screen.Main else Screen.SignIn) }
    // The tabs keep their content and focus while a title page or the player is on top.
    val main = remember { MainState() }
    val saveable = rememberSaveableStateHolder()
    val top = stack.last()
    fun push(s: Screen) = stack.add(s)
    fun pop() { if (stack.size > 1) stack.removeAt(stack.lastIndex) }
    fun toSignIn() {
        main.clear()
        stack.clear()
        stack.add(Screen.SignIn)
    }

    BackHandler(enabled = stack.size > 1) { pop() }

    androidx.compose.runtime.LaunchedEffect(DebugSeams.play) {
        val (id, start) = DebugSeams.play ?: return@LaunchedEffect
        // Cleared once handled (clearing it first would restart this effect and cancel it).
        try {
            // Signed out (or not yet signed in): the link is dropped, sign-in shows.
            val api = Session.api ?: return@LaunchedEffect
            if (stack.firstOrNull() == Screen.SignIn) return@LaunchedEffect
            Log.i("Moonrise") { "play $id at ${start ?: "resume"}" }
            val item = runCatching { api.item(id) }.onFailure { Log.w("Moonrise", "can't open $id: ${it.message}") }.getOrNull()
                ?: return@LaunchedEffect
            while (stack.size > 1) stack.removeAt(stack.lastIndex)
            stack.add(Screen.Details(item))
            stack.add(Screen.Player(item, start ?: item.resumeSeconds, null))
        } finally {
            DebugSeams.play = null
        }
    }
    androidx.compose.runtime.LaunchedEffect(DebugSeams.open) {
        val id = DebugSeams.open ?: return@LaunchedEffect
        val item = runCatching { Session.api?.item(id) }.getOrNull()
        if (item != null) {
            while (stack.size > 1) stack.removeAt(stack.lastIndex)
            stack.add(Screen.Details(item))
        }
        DebugSeams.open = null
    }

    Box(Modifier.fillMaxSize().background(Theme.background)) {
        // Each screen comes in with a short fade (a title page with a slight zoom, as tvOS).
        ScreenEnter(top, from = when (top) { is Screen.Details -> 1.03f; Screen.Main -> 0.985f; else -> 1f }) {
            when (top) {
                Screen.SignIn -> SignInScreen(onSignedIn = {
                    main.clear()
                    stack.clear()
                    stack.add(Screen.Main)
                })
                Screen.Main -> saveable.SaveableStateProvider("main") {
                    MainScreen(
                        state = main,
                        open = { push(Screen.Details(it)) },
                        signOut = { Session.signOut(); toSignIn() },
                        changeServer = { Session.changeServer(); toSignIn() },
                    )
                }
                is Screen.Details -> saveable.SaveableStateProvider("details-${top.item.id}-${stack.size}") {
                    DetailsScreen(
                        state = top.state,
                        play = { item, start, version -> push(Screen.Player(item, start, version)) },
                        signIn = { Session.signOut(); toSignIn() },
                    )
                }
                // A new player for each title (Next Episode replaces this one).
                is Screen.Player -> key(top) {
                    PlayerScreen(
                        item = top.item, start = top.start, version = top.version,
                        onExit = {
                            pop()
                            main.refreshSoon()
                        },
                        onPlayEpisode = { ep ->
                            // Another episode of the show: it replaces this player, from its resume point.
                            // Back from it returns to the episode on the title page.
                            stack.filterIsInstance<Screen.Details>().lastOrNull()?.state?.returnToEpisode(ep)
                            stack[stack.lastIndex] = Screen.Player(ep, ep.resumeSeconds, null)
                        },
                    )
                }
            }
        }
    }
}
