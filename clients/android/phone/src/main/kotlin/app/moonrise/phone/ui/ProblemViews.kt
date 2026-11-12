package app.moonrise.phone.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.moonrise.core.LayoutMetrics
import app.moonrise.core.PlaybackProblem

/** A problem's Details: the server's own words, behind a tap. */
@Composable
fun ProblemDetailsDialog(problem: PlaybackProblem, onDismiss: () -> Unit) {
    AlertDialog(onDismissRequest = onDismiss, containerColor = Theme.sheet, title = { Text(problem.title, fontWeight = FontWeight.SemiBold) },
        text = { Text(problem.detail.orEmpty(), style = Type.footnote) },
        confirmButton = { TextButton(onClick = onDismiss) { Text("OK", color = Color.White) } })
}

/** "Details", as a small glass capsule. */
@Composable
fun DetailsCapsule(onClick: () -> Unit) {
    Text("Details", style = Type.subheadline, color = Color.White, modifier = Modifier.clip(CircleShape).background(Theme.surfaceStrong)
        .clickable(onClick = onClick).padding(horizontal = 14.dp, vertical = 6.dp))
}

/**
 * In place of Play on a title page when no file of the title can play: what's wrong in a
 * line, and Details for the server's own words. No Play and no Download.
 */
@Composable
fun DamagedTitleNote(problem: PlaybackProblem, regular: Boolean) {
    var details by remember { mutableStateOf(false) }
    Row(
        (if (regular) Modifier.width((LayoutMetrics.PLAY_WIDTH + 12 + 52).dp) else Modifier.fillMaxWidth())
            .padding(top = 16.dp).clip(RoundedCornerShape(16.dp)).background(Theme.surfaceStrong).padding(horizontal = 16.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Filled.Warning, contentDescription = null, tint = Color(0xFFFFD60A), modifier = Modifier.size(24.dp))
        Column(Modifier.weight(1f)) {
            Text(problem.title, style = Type.headline, color = Color.White)
            Text(problem.message, style = Type.subheadline, color = Theme.secondaryText)
        }
        if (problem.detail != null) DetailsCapsule { details = true }
    }
    if (details) ProblemDetailsDialog(problem) { details = false }
}
