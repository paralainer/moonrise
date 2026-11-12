package app.moonrise.core

import kotlin.math.floor
import kotlin.math.roundToInt

/**
 * Layout sizes for the window the screens are in, in dp (the Apple apps' `Metrics`). A phone, and
 * a window narrower than a tablet's, gets the phone's constants; a tablet window of regular width
 * gets sizes from its measured width: the iPad's margins, rows that show about as many posters as
 * the width holds (with the next one peeking in), and the rules for the iPad's layouts.
 */
data class LayoutMetrics(
    /** Tablet, regular width (the iPad's regular size class). */
    val regular: Boolean = false,
    /** The window's size (zero on a phone: only read when [regular]). */
    val width: Double = 0.0,
    val height: Double = 0.0,
    val hPadding: Double = PHONE_MARGIN,
    val posterWidth: Double = 118.0,
    val landscapeWidth: Double = 250.0,
    /** Library grid: the smallest poster. */
    val gridMin: Double = 104.0,
) {
    /** Wider than tall: title pages and Home's banner put the title and Play over the picture. */
    val wide: Boolean get() = regular && width > height

    /** The title page's and Home banner's header height; `phone` is the phone layout's own. */
    fun headerHeight(phone: Double): Double =
        if (regular) minOf(width * 9 / 16, height * (if (wide) 0.62 else 0.44)).roundToInt().toDouble() else phone

    /** How far the title, its line and Play reach up over the picture (wide tablets, with artwork). */
    fun headerOverlap(hasImage: Boolean = true): Double = if (wide && hasImage) 150.0 else 0.0

    /** Home's banner under the picture (line, genres, Play), less what sits over the picture. */
    val heroInfoHeight: Double get() = 50 + 16 + PLAY_HEIGHT - headerOverlap()

    /** The Library's sidebar: narrower when the window is taller than wide. */
    val sidebarWidth: Double get() = if (wide) 280.0 else 250.0

    /** The Library's page beside the sidebar and the line between them. */
    val libraryPageWidth: Double get() = width - sidebarWidth - 1

    /** A page too narrow for the title beside Sort and Filter: they go on a row of their own under it. */
    val libraryHeaderStacked: Boolean get() = regular && libraryPageWidth < LIBRARY_HEADER_MIN

    /**
     * The tablet's Filter panel: under its button (whose bottom is `buttonBottom` down the window),
     * no further than the window's edge above the system bar `bottomInset` tall.
     */
    fun filterPanelHeight(buttonBottom: Double, bottomInset: Double): Double =
        (height - buttonBottom - FILTER_PANEL_GAP - bottomInset - FILTER_PANEL_EDGE).coerceIn(FILTER_PANEL_MIN_HEIGHT, FILTER_PANEL_MAX_HEIGHT)

    /** The tablet's Sort menu, likewise; a menu that would need more than this is moved over its button. */
    fun sortMenuHeight(buttonBottom: Double, bottomInset: Double): Double = filterPanelHeight(buttonBottom, bottomInset + MENU_EDGE)

    /** Columns and card width of a poster grid `available` wide. */
    fun grid(available: Double, atLeast: Int = 2, spacing: Double = GRID_SPACING): Pair<Int, Double> {
        val n = maxOf(atLeast, floor((available + spacing) / (gridMin + spacing)).toInt())
        return n to floor((available - (n - 1) * spacing) / n)
    }

    /** The grid beside the sidebar, `available` wide within the page: at least 4 across, or 3 on a page too narrow for 4 readable posters. */
    fun libraryGrid(available: Double): Pair<Int, Double> =
        grid(available, atLeast = if (libraryPageWidth < LIBRARY_FOUR_ACROSS_MIN) 3 else 4)

    /**
     * Play fills the screen: a phone on its side; a wide tablet always (no small player in that
     * layout), and a tall one with the full-screen button (`requested`).
     */
    fun playerFullScreen(landscape: Boolean, requested: Boolean): Boolean = if (regular) wide || requested else landscape

    /** Leaving full screen is leaving the player: a wide tablet has no small player to go back to. */
    val stopsOnFullScreenExit: Boolean get() = wide

    /** The video's band across a page `pageHeight` tall: 16:9, leaving room for the page. */
    fun playerBand(pageWidth: Double, pageHeight: Double): Double = minOf(pageWidth * 9 / 16, pageHeight * 0.6)

    /** Selected episode: the tablet's first slot (at the page margin), the phone's middle. */
    fun episodeOffset(screen: Double, card: Double): Double = if (regular) 0.0 else (screen - card) / 2 - hPadding

    companion object {
        const val PHONE_MARGIN = 16.0
        /** The tablet page margin (the iPad's system margin). */
        const val TABLET_MARGIN = 20.0
        const val ROW_SPACING = 14.0
        const val GRID_SPACING = 12.0
        /** The smallest window dimension of a tablet (Android's own line for one). */
        const val TABLET_MIN = 600.0
        const val PLAY_HEIGHT = 52.0
        const val PLAY_WIDTH = 300.0
        /** The title page's text column on a tablet (about 80 characters of the overview). */
        const val TEXT_COLUMN = 640.0
        /** The sidebar's title row and the page header's: one height, so their tops line up. */
        const val TITLE_ROW = 52.0
        const val SIDEBAR_ROW = 44.0
        const val SIDEBAR_TOP = 8.0
        /** Back, Sort, Filter on the tablet's Library header: their height. */
        const val HEADER_BUTTON = 40.0
        const val FILTER_PANEL_WIDTH = 380.0
        const val FILTER_PANEL_MAX_HEIGHT = 560.0
        const val FILTER_PANEL_MIN_HEIGHT = 200.0
        /** The Filter panel's gap below its button, and what it keeps clear of the window's edge. */
        const val FILTER_PANEL_GAP = 8.0
        const val FILTER_PANEL_EDGE = 12.0
        /** What Compose's dropdown menus keep clear of the window's edge. */
        const val MENU_EDGE = 48.0
        /** The Library page from which the title stays beside Sort and Filter (Back, the longest Sort, a badge, a long name at 70%). */
        const val LIBRARY_HEADER_MIN = 540.0
        /** The Library page from which 4 posters fit across beside the sidebar (under it, 4 would be smaller than the phone's). */
        const val LIBRARY_FOUR_ACROSS_MIN = 400.0
        /** A page title shrinks this far to fit, then is cut off (the iPad's `minimumScaleFactor`). */
        const val TITLE_MIN_SCALE = 0.7
        /** A download's card on the tablet: the smallest width of a column. */
        const val DOWNLOAD_CARD_MIN = 340.0

        /**
         * The size to set a title at, of its full size, when it is `natural` wide with `available`
         * to fit in. In steps, rounded down: text at exactly the fitting size can still be cut off.
         */
        fun titleScale(natural: Double, available: Double): Double {
            if (natural <= available) return 1.0
            return (floor(available / natural * 0.99 * 20) / 20).coerceAtLeast(TITLE_MIN_SCALE)
        }

        /**
         * The layout for a window of this size. Tablet needs both the window's width and the
         * device's smallest width (so a phone on its side stays a phone, and a narrow split-screen
         * window keeps the phone layout), as the iPad's regular size class does.
         */
        fun of(width: Double, height: Double, smallestWidth: Double): LayoutMetrics {
            if (width < TABLET_MIN || smallestWidth < TABLET_MIN) return LayoutMetrics()
            // Items visible across a row (the fraction peeks in at the trailing edge).
            val posters = if (width < 900) 5.3 else if (width < 1250) 6.3 else 7.3
            val cards = if (width < 900) 2.6 else if (width < 1250) 3.3 else 3.5
            fun size(count: Double) = floor((width - TABLET_MARGIN - ROW_SPACING * floor(count)) / count)
            return LayoutMetrics(
                regular = true, width = width, height = height, hPadding = TABLET_MARGIN,
                posterWidth = maxOf(118.0, size(posters)), landscapeWidth = maxOf(250.0, size(cards)), gridMin = 150.0,
            )
        }
    }
}
