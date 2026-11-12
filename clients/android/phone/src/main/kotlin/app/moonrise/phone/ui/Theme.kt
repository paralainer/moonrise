package app.moonrise.phone.ui

import app.moonrise.core.LayoutMetrics
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** The window's layout sizes: a tablet's come from its measured width (`Metrics`, Apple apps). */
val LocalMetrics = staticCompositionLocalOf { LayoutMetrics() }

/**
 * The iPhone app's look (`Theme`, `Metrics`): a neutral dark page, white chrome, the artwork
 * carrying the colour. Sizes are the iPhone's points as dp, so the two lay out alike.
 */
object Theme {
    val accent = Color.White
    val background = Color(0xFF0D0F14)
    val backgroundTop = Color(0xFF1A1C26)
    val surface = Color.White.copy(alpha = 0.07f)
    val surfaceStrong = Color.White.copy(alpha = 0.12f)
    val secondaryText = Color.White.copy(alpha = 0.62f)
    /** The glass of buttons over pictures and the tab bar. */
    val glass = Color(0xFF26282E).copy(alpha = 0.94f)
    val sheet = Color(0xFF1C1E25)
    /** The tab bar: glass without the blur behind it, so nearly opaque to keep its labels clear. */
    val bar = Color(0xFF202227)
    val destructive = Color(0xFFFF453A)
    val green = Color(0xFF30D158)

    val backgroundGradient: Brush get() = Brush.verticalGradient(listOf(backgroundTop, background))

    val posterWidth = 118.dp
    val landscapeWidth = 250.dp
    val gridMin = 104.dp
    val hPadding = 16.dp

    /** The page margin: the phone's [hPadding], a tablet's own. */
    val margin: Dp @Composable @ReadOnlyComposable get() = LocalMetrics.current.hPadding.dp

    /** A tablet's pages below the tab bar at the top (under the status bar), else nothing. */
    val underTopBar: Dp @Composable @ReadOnlyComposable get() = if (LocalMetrics.current.regular) 62.dp else 0.dp

    /** The empty end of a scrolling page (the phone: clear of the bottom tab bar). */
    val pageBottom: Dp @Composable get() = if (LocalMetrics.current.regular) 40.dp + WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding() else 120.dp

    /** A back button at the top of the page (the iPad's nav bar sits nearer the edge). */
    val backStart: Dp @Composable @ReadOnlyComposable get() = if (LocalMetrics.current.regular) 10.dp else hPadding
    val rowSpacing = 14.dp

    /** The title page's and Home banner's header (`TitleHeader.height`, iPhone held upright). */
    val headerHeight = 440.dp
}

/** iOS's text styles, by name (Dynamic Type's default sizes). */
object Type {
    val largeTitle = TextStyle(fontSize = 34.sp, fontWeight = FontWeight.Bold, lineHeight = 41.sp)
    val title = TextStyle(fontSize = 28.sp, fontWeight = FontWeight.Bold, lineHeight = 34.sp)
    val title2 = TextStyle(fontSize = 22.sp, lineHeight = 28.sp)
    val title3 = TextStyle(fontSize = 20.sp, lineHeight = 25.sp)
    val headline = TextStyle(fontSize = 17.sp, fontWeight = FontWeight.SemiBold, lineHeight = 22.sp)
    val body = TextStyle(fontSize = 17.sp, lineHeight = 22.sp)
    val callout = TextStyle(fontSize = 16.sp, lineHeight = 21.sp)
    val subheadline = TextStyle(fontSize = 15.sp, lineHeight = 20.sp)
    val footnote = TextStyle(fontSize = 13.sp, lineHeight = 18.sp)
    val caption = TextStyle(fontSize = 12.sp, lineHeight = 16.sp)
    val caption2 = TextStyle(fontSize = 11.sp, lineHeight = 13.sp)
}

@Composable
fun MoonriseTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = darkColorScheme(
            primary = Color.White,
            onPrimary = Color.Black,
            secondary = Color.White,
            background = Theme.background,
            surface = Theme.sheet,
            surfaceContainer = Theme.sheet,
            surfaceContainerHigh = Color(0xFF26282F),
            surfaceContainerHighest = Color(0xFF2E3038),
            onSurface = Color.White,
            onSurfaceVariant = Theme.secondaryText,
            outline = Color.White.copy(alpha = 0.3f),
            error = Theme.destructive,
        ),
        content = content,
    )
}
