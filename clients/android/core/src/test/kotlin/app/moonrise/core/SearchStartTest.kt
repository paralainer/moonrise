package app.moonrise.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SearchStartTest {
    private fun item(id: String) = Item(id = id, kind = "movie", title = id)
    private fun ids(l: List<Item>) = l.map { it.id }

    @Test
    fun openedTitleGoesFirstOnceAndLeavesSuggestions() {
        val s = SearchStart(recent = listOf(item("a"), item("b")), suggestions = listOf(item("c"), item("d")))
        val fromSuggestions = s.opened(item("c"))
        assertEquals(listOf("c", "a", "b"), ids(fromSuggestions.recent))
        assertEquals(listOf("d"), ids(fromSuggestions.suggestions))
        val again = s.opened(item("b"))
        assertEquals(listOf("b", "a"), ids(again.recent))
        assertEquals(listOf("c", "d"), ids(again.suggestions))
    }

    @Test
    fun recentKeepsTheLatest() {
        var s = SearchStart()
        for (n in 0 until SearchStart.RECENT_KEPT + 5) s = s.opened(item("m$n"))
        assertEquals(SearchStart.RECENT_KEPT, s.recent.size)
        assertEquals("m${SearchStart.RECENT_KEPT + 4}", s.recent.first().id)
    }

    @Test
    fun clearingRecentKeepsSuggestions() {
        val s = SearchStart(recent = listOf(item("a")), suggestions = listOf(item("b"))).recentCleared()
        assertTrue(s.recent.isEmpty())
        assertEquals(listOf("b"), ids(s.suggestions))
        assertFalse(s.isEmpty)
        assertTrue(s.copy(suggestions = emptyList()).isEmpty)
    }

    @Test
    fun rowsOnlyWhileNothingIsTyped() {
        assertTrue(SearchStart.shows(""))
        assertTrue(SearchStart.shows("   "))
        assertFalse(SearchStart.shows("a"))
    }
}
