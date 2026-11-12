package app.moonrise.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The tablet layout's rules ([LayoutMetrics]), the iPad's `Metrics`; a phone keeps its constants. */
class LayoutMetricsTest {
    private val phone = LayoutMetrics()
    private val tall = LayoutMetrics.of(800.0, 1280.0, 800.0)
    private val wide = LayoutMetrics.of(1280.0, 800.0, 800.0)

    @Test
    fun aPhoneKeepsThePhoneLayoutEitherWayRound() {
        assertEquals(phone, LayoutMetrics.of(411.0, 731.0, 411.0))
        assertEquals(phone, LayoutMetrics.of(731.0, 411.0, 411.0))
        assertFalse(phone.regular)
        assertEquals(16.0, phone.hPadding, 0.0)
        assertEquals(118.0, phone.posterWidth, 0.0)
        assertEquals(250.0, phone.landscapeWidth, 0.0)
        assertEquals(104.0, phone.gridMin, 0.0)
    }

    @Test
    fun aNarrowWindowOnATabletKeepsThePhoneLayout() {
        assertEquals(phone, LayoutMetrics.of(400.0, 1280.0, 400.0))
        assertEquals(phone, LayoutMetrics.of(800.0, 500.0, 500.0))
    }

    @Test
    fun aTabletFromSixHundredWideUsesTabletSizes() {
        assertTrue(LayoutMetrics.of(600.0, 960.0, 600.0).regular)
        assertTrue(tall.regular)
        assertEquals(20.0, tall.hPadding, 0.0)
        assertEquals(150.0, tall.gridMin, 0.0)
    }

    /** The iPad's table: 5.3 posters and 2.6 cards under 900, 6.3 and 3.3 under 1250, then 7.3 and 3.5. */
    @Test
    fun rowsShowAsManyAsTheWidthHolds() {
        assertEquals(133.0, tall.posterWidth, 0.0)   // (800-20-14*5)/5.3
        assertEquals(289.0, tall.landscapeWidth, 0.0) // (800-20-14*2)/2.6
        assertEquals(159.0, wide.posterWidth, 0.0)   // (1280-20-14*7)/7.3
        assertEquals(348.0, wide.landscapeWidth, 0.0) // (1280-20-14*3)/3.5
        // A small tablet: never smaller than the phone's cards.
        val small = LayoutMetrics.of(600.0, 960.0, 600.0)
        assertEquals(118.0, small.posterWidth, 0.0)
        assertEquals(250.0, small.landscapeWidth, 0.0)
    }

    @Test
    fun theHeaderIs16By9CappedByTheWindowsHeight() {
        assertEquals(440.0, phone.headerHeight(440.0), 0.0)
        assertEquals(450.0, tall.headerHeight(440.0), 0.0)  // 800*9/16, under 44% of 1280
        assertEquals(496.0, wide.headerHeight(440.0), 0.0)  // 62% of 800, under 720
        assertTrue(wide.wide)
        assertFalse(tall.wide)
    }

    @Test
    fun onlyAWideTabletPutsTheTitleOverThePicture() {
        assertEquals(150.0, wide.headerOverlap(), 0.0)
        assertEquals(0.0, wide.headerOverlap(hasImage = false), 0.0)
        assertEquals(0.0, tall.headerOverlap(), 0.0)
        assertEquals(0.0, phone.headerOverlap(), 0.0)
    }

    @Test
    fun theSidebarNarrowsWhenTall() {
        assertEquals(280.0, wide.sidebarWidth, 0.0)
        assertEquals(250.0, tall.sidebarWidth, 0.0)
    }

    /** The real 600 dp tablet: 250 of sidebar left "Movies" as "Mov...", and a genre's page with Back as "..". */
    @Test
    fun theTitleHasItsOwnRowWhenThePageIsNarrow() {
        assertTrue(LayoutMetrics.of(600.0, 960.0, 600.0).libraryHeaderStacked)
        assertFalse(tall.libraryHeaderStacked)
        assertFalse(wide.libraryHeaderStacked)
        assertFalse(LayoutMetrics.of(960.0, 600.0, 600.0).libraryHeaderStacked)
        assertFalse(phone.libraryHeaderStacked)
    }

    /** Set smaller to fit, as far as the iPad's 0.7; no larger than its own size. */
    @Test
    fun aTitleShrinksToFitThenStops() {
        assertEquals(1.0, LayoutMetrics.titleScale(200.0, 300.0), 0.0)
        assertEquals(1.0, LayoutMetrics.titleScale(200.0, 200.0), 0.0)
        // Exactly 0.8 would just not fit (rounding): a step smaller.
        assertEquals(0.75, LayoutMetrics.titleScale(250.0, 200.0), 0.001)
        assertEquals(0.85, LayoutMetrics.titleScale(300.0, 260.0), 0.001)
        assertEquals(0.7, LayoutMetrics.titleScale(1000.0, 200.0), 0.0)
    }

    /** 560 in a 600 tall window didn't fit under the button: the system moved it up over the tabs. */
    @Test
    fun theFilterPanelFitsBelowItsButton() {
        val landscape = LayoutMetrics.of(960.0, 600.0, 600.0)
        val height = landscape.filterPanelHeight(buttonBottom = 130.0, bottomInset = 48.0)
        assertEquals(402.0, height, 0.0) // 600-130-8-48-12
        assertTrue(130.0 + LayoutMetrics.FILTER_PANEL_GAP + height + 48.0 <= 600.0)
        // Portrait has the room: the panel keeps its size.
        assertEquals(560.0, LayoutMetrics.of(600.0, 960.0, 600.0).filterPanelHeight(130.0, 48.0), 0.0)
        // The Sort menu likewise, inside the margin Compose keeps for its menus.
        assertEquals(354.0, landscape.sortMenuHeight(buttonBottom = 130.0, bottomInset = 48.0), 0.0) // 600-130-8-48-48-12
        assertEquals(560.0, LayoutMetrics.of(600.0, 960.0, 600.0).sortMenuHeight(130.0, 48.0), 0.0)
        // Never a sliver.
        assertEquals(200.0, landscape.filterPanelHeight(500.0, 48.0), 0.0)
    }

    /** iPad beside the sidebar: at least 4 across, the posters smaller. */
    @Test
    fun theGridFillsTheRowEvenlyWithAtLeastFourBesideTheSidebar() {
        assertEquals(6 to 150.0, wide.grid(960.0))
        val (n, card) = tall.grid(469.0, atLeast = 4)
        assertEquals(4, n)
        assertEquals(108.0, card, 0.0)
        assertEquals(2, phone.grid(200.0).first)
    }

    /** The real 600 dp tablet: 4 across left 68 dp posters, smaller than the phone's, and cut captions. */
    @Test
    fun aNarrowLibraryPageShowsThreeAcrossNotFourTinyPosters() {
        val small = LayoutMetrics.of(600.0, 960.0, 600.0)
        assertEquals(349.0, small.libraryPageWidth, 0.0)
        assertEquals(3 to 95.0, small.libraryGrid(309.0)) // the page less two 20 margins
        // The line: a page of 399 is 3 across, of 400 is 4.
        assertEquals(3, LayoutMetrics.of(650.0, 960.0, 650.0).libraryGrid(359.0).first)
        assertEquals(4, LayoutMetrics.of(651.0, 960.0, 651.0).libraryGrid(360.0).first)
        // Wider pages keep 4 and up.
        assertEquals(4 to 118.0, tall.libraryGrid(509.0))
        assertEquals(4, LayoutMetrics.of(960.0, 600.0, 600.0).libraryGrid(639.0).first)
        assertEquals(5, wide.libraryGrid(959.0).first)
    }

    @Test
    fun aPhonePlaysFullScreenOnItsSideATabletWhenWideOrAsked() {
        assertTrue(phone.playerFullScreen(landscape = true, requested = false))
        assertFalse(phone.playerFullScreen(landscape = false, requested = true))
        assertTrue(wide.playerFullScreen(landscape = true, requested = false))
        assertFalse(tall.playerFullScreen(landscape = false, requested = false))
        assertTrue(tall.playerFullScreen(landscape = false, requested = true))
        assertTrue(wide.stopsOnFullScreenExit)
        assertFalse(tall.stopsOnFullScreenExit)
    }

    @Test
    fun theVideoBandLeavesRoomForThePage() {
        assertEquals(450.0, tall.playerBand(800.0, 1280.0), 0.0)
        assertEquals(480.0, wide.playerBand(1280.0, 800.0), 0.0)
    }

    @Test
    fun theSelectedEpisodeSitsFirstOnATabletAndMiddleOnAPhone() {
        assertEquals(0.0, tall.episodeOffset(800.0, 289.0), 0.0)
        assertEquals(64.5, phone.episodeOffset(411.0, 250.0), 0.0)
    }
}

/** The Details section of a tablet's title page ([TitleFacts]). */
class TitleFactsTest {
    private val file = MediaFile(id = "f", size = 2_000_000_000, bitrate = 4_500_000, tracks = listOf(
        Track(index = 0, kind = "video", displayTitle = "1080p H264"),
        Track(index = 1, kind = "audio", language = "en"),
        Track(index = 2, kind = "subtitle", language = "fr"),
    ))
    private val locale = java.util.Locale.ENGLISH

    @Test
    fun aMovieNamesItsPeopleStudioAndFile() {
        val movie = Item(id = "m", kind = "movie", premiereDate = "2017-10-06", studios = listOf("A", "B"), genres = listOf("Sci-Fi"), officialRating = "R",
            people = listOf(Person(id = "1", name = "D One", kind = "Director"), Person(id = "2", name = "W One", kind = "Writer"), Person(id = "3", name = "W Two", kind = "Writer")), files = listOf(file))
        val facts = TitleFacts.of(movie, null, 0, locale).associate { it.label to it.value }
        assertEquals("D One", facts["Director"])
        assertEquals("W One, W Two", facts["Writers"])
        assertEquals("A, B", facts["Studios"])
        assertEquals("1080p H264", facts["Video"])
        assertEquals("R", facts["Rated"])
        assertTrue(facts.getValue("File").endsWith("4.5 Mbps"))
        assertFalse(facts.containsKey("Seasons"))
    }

    @Test
    fun aShowCountsSeasonsAndDescribesTheEpisodesFile() {
        val show = Item(id = "s", kind = "series", premiereDate = "2022-02-17", studios = listOf("Net"), genres = listOf("Drama"))
        val ep = Item(id = "e", kind = "episode", officialRating = "TV-MA", files = listOf(file))
        val facts = TitleFacts.of(show, ep, 2, locale).associate { it.label to it.value }
        assertEquals("Net", facts["Network"])
        assertEquals("2", facts["Seasons"])
        assertEquals("TV-MA", facts["Rated"])
        assertEquals("1080p H264", facts["Episode Video"])
        assertTrue(facts.containsKey("Episode Audio"))
    }
}
