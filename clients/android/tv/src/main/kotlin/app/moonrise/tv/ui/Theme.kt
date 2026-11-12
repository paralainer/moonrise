package app.moonrise.tv.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.unit.dp
import androidx.tv.material3.LocalContentColor
import androidx.tv.material3.MaterialTheme
import androidx.tv.material3.darkColorScheme

/**
 * The Apple TV app's look (`clients/apple/Moonrise/Support/Theme.swift`): content artwork carries the colour,
 * the chrome stays white on near-black. Sizes are the tvOS ones halved: tvOS lays out 1920×1080
 * points, a 1080p Android TV 960×540 dp.
 */
object Theme {
    val background = Color(0xFF0D0F14)
    val surface = Color.White.copy(alpha = 0.07f)
    val surfaceStrong = Color.White.copy(alpha = 0.12f)
    val secondaryText = Color.White.copy(alpha = 0.62f)
    val backgroundGradient = Brush.verticalGradient(listOf(Color(0xFF1A1C26), background))

    /** The screen's side margin (tvOS: 80 pt). */
    val margin = 40.dp
    val posterWidth = 110.dp
    val landscapeWidth = 200.dp
    val rowSpacing = 14.dp
    val cardRadius = 8.dp
}

/**
 * Focus motion, as tvOS moves focus: the focused control lifts quickly and settles without a
 * bounce, and the one left behind sinks back the same way. Every focus look animates with it.
 */
object FocusMotion {
    fun <T> spec(): androidx.compose.animation.core.AnimationSpec<T> =
        androidx.compose.animation.core.spring(dampingRatio = 0.9f, stiffness = 520f)

    /** How long a screen takes to come in (a title page, the player, back to the tabs). */
    const val SCREEN_MS = 240
}

/**
 * A screen coming in: it fades up from the background and settles from a slight zoom (`from`),
 * as tvOS pushes a page. Drawing only: focus and layout are untouched, so focus restores as before.
 */
@Composable
fun ScreenEnter(screen: Any, from: Float = 1.03f, content: @Composable () -> Unit) {
    // A new screen (`screen` changed) starts from nothing in the same frame: no flash of it at full.
    val t = androidx.compose.runtime.remember(screen) { androidx.compose.animation.core.Animatable(0f) }
    androidx.compose.runtime.LaunchedEffect(t) {
        t.animateTo(1f, androidx.compose.animation.core.tween(FocusMotion.SCREEN_MS, easing = androidx.compose.animation.core.FastOutSlowInEasing))
    }
    androidx.compose.foundation.layout.Box(
        androidx.compose.ui.Modifier.graphicsLayer {
            alpha = t.value
            val s = from + (1f - from) * t.value
            scaleX = s; scaleY = s
        },
    ) { content() }
}

@Composable
fun MoonriseTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = darkColorScheme(
            primary = Color.White,
            onPrimary = Color.Black,
            background = Theme.background,
            onBackground = Color.White,
            surface = Theme.background,
            onSurface = Color.White,
            onSurfaceVariant = Theme.secondaryText,
        ),
    ) {
        // Text outside a tv-material Surface takes this colour (black by default).
        CompositionLocalProvider(LocalContentColor provides Color.White, content = content)
    }
}
