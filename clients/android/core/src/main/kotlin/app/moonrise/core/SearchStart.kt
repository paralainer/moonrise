package app.moonrise.core

import kotlinx.serialization.Serializable

/**
 * What Search offers before anything is typed (`GET /v1/search/start`, capability `search-start`):
 * Recent, the titles opened from Search (on every device), and Suggestions from the library.
 * The same rules as the Apple apps' `SearchStart`.
 */
@Serializable
data class SearchStart(val recent: List<Item> = emptyList(), val suggestions: List<Item> = emptyList()) {
    val isEmpty: Boolean get() = recent.isEmpty() && suggestions.isEmpty()

    /** A title was opened from Search: first in Recent at once, and no longer suggested. */
    fun opened(item: Item): SearchStart = copy(
        recent = (listOf(item) + recent.filter { it.id != item.id }).take(RECENT_KEPT),
        suggestions = suggestions.filter { it.id != item.id },
    )

    fun recentCleared(): SearchStart = copy(recent = emptyList())

    companion object {
        const val RECENT_KEPT = 20

        /** The rows are shown only while nothing is typed. */
        fun shows(query: String): Boolean = query.isBlank()
    }
}
