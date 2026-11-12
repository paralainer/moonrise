package app.moonrise.core

import org.junit.Assert.assertEquals
import org.junit.Test

/** The phone Library's count line, worded as the iPhone's (`LibraryBrowser.countLine`). */
class LibraryCountLineTest {
    @Test
    fun titleSortKeepsItsCapitals() {
        assertEquals("7 movies · Title, A to Z", LibraryFilter().countLine(shows = false, total = 7))
        assertEquals("7 movies · Title, Z to A", LibraryFilter(descending = true).countLine(shows = false, total = 7))
    }

    @Test
    fun otherSortsSayTheirDirectionInLowerCase() {
        assertEquals("1 show · Date Added, newest first", LibraryFilter().sortedBy(LibrarySort.Added).countLine(shows = true, total = 1))
        assertEquals("2 shows · Rating, lowest first", LibraryFilter(sort = LibrarySort.Rating, descending = false).countLine(shows = true, total = 2))
        assertEquals("0 movies · Last Played, most recent first", LibraryFilter().sortedBy(LibrarySort.Played).countLine(shows = false, total = 0))
    }
}

/** The phone's badges name the resolution as the server's buckets do, as the iPhone's title line. */
class PhoneBadgesTest {
    @Test
    fun resolutionBuckets() {
        assertEquals(ResolutionFilter.Sd, ResolutionFilter.of(720, 576))
        assertEquals(ResolutionFilter.Hd, ResolutionFilter.of(1280, 720))
        assertEquals(ResolutionFilter.Fhd, ResolutionFilter.of(1920, 800))
        assertEquals(ResolutionFilter.Uhd, ResolutionFilter.of(3840, 1600))
    }

    @Test
    fun phoneBadgesSayTheBucket() {
        val item = Item(id = "m", kind = "movie", media = MediaSummary(video = MediaSummary.Video(width = 1280, height = 720), audio = listOf(MediaSummary.Audio(channels = 6))))
        assertEquals(listOf("720p", "5.1"), item.phoneTechBadges)
        assertEquals(listOf("HD", "5.1"), item.techBadges)
    }
}
