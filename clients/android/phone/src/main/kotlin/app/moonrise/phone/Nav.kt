package app.moonrise.phone

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import app.moonrise.core.Item
import app.moonrise.core.LibraryCategory
import app.moonrise.core.LibraryFilter
import app.moonrise.core.LibraryView
import app.moonrise.phone.ui.TitlePageState
import java.util.concurrent.atomic.AtomicInteger

/** The tabs at the bottom, as the iPhone's tab bar. */
enum class Tab(val title: String) { Home("Home"), Library("Library"), Downloads("Downloads"), Search("Search") }

/** Play as soon as a title page opens (`start` null: resume), as the iPhone's `Autoplay`. */
data class Autoplay(val start: Double? = null)

private val serial = AtomicInteger()

/** A page pushed on a tab (the iPhone's `Route`). */
sealed class Route {
    /** Each push is its own page (the same title twice on a stack keeps two). */
    val key: String = "route-${serial.incrementAndGet()}"

    /** A title's page; an episode or season opens its show's, about it. */
    class Title(val item: Item, val autoplay: Autoplay? = null) : Route() {
        /** The page's data and its player, kept while other pages are on top. */
        val page = TitlePageState(item)
    }

    /** A library's genres, collections, studios or decades. */
    class Category(val view: LibraryView, val category: LibraryCategory) : Route()

    /** A library's grid with a filter (a category picked), titled. */
    class Filtered(val view: LibraryView, val filter: LibraryFilter, val title: String) : Route()
}

/** Where the app is: the tab, each tab's pages, and Settings (a sheet over them). */
class AppNav {
    var tab by mutableStateOf(Tab.Home)
    val stacks: Map<Tab, MutableList<Route>> = Tab.entries.associateWith { mutableStateListOf() }
    var settings by mutableStateOf(false)
    /** The player fills the screen (landscape): no tab bar. */
    var fullScreen by mutableStateOf(false)

    val stack: MutableList<Route> get() = stacks.getValue(tab)
    val top: Route? get() = stack.lastOrNull()

    fun push(r: Route) { stack.add(r) }

    /** Back: Settings, then the tab's page, then Home; false when there's nothing to go back to. */
    fun back(): Boolean {
        when {
            settings -> settings = false
            stack.isNotEmpty() -> stack.removeAt(stack.lastIndex)
            tab != Tab.Home -> tab = Tab.Home
            else -> return false
        }
        return true
    }

    /** A title opened from outside a tab's lists (a link, a test seam): on Home. */
    fun openOnHome(r: Route) {
        settings = false
        tab = Tab.Home
        stacks.getValue(Tab.Home).add(r)
    }

    fun reset() {
        stacks.values.forEach { it.clear() }
        tab = Tab.Home
        settings = false
        fullScreen = false
    }
}

val LocalNav = staticCompositionLocalOf<AppNav> { error("no AppNav") }
