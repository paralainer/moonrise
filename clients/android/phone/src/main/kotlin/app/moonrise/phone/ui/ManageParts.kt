// SPDX-License-Identifier: MPL-2.0
package app.moonrise.phone.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Remove
import androidx.compose.material.icons.filled.UnfoldMore
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.Icon
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.moonrise.core.AdminRules
import app.moonrise.core.adminErrorText
import app.moonrise.shared.Session
import kotlinx.coroutines.CancellationException

// The pieces of the admin screens (Manage Server, the setup, Fix Match), as the iPhone's forms have
// them (`InfoRow`, `InfoRowStack`, `NoteRow`, `ManageLink`, `LogLine`, `BusyLabel`).

val Warn = Color(0xFFFF9F0A)

/**
 * Whether the signed-in account is an admin, asked of the server (`AdminState`, iPhone): Manage
 * Server and Fix Match are hidden otherwise, the server refuses them anyway.
 */
object AdminState {
    var isAdmin by mutableStateOf(false)
        private set
    private var checked: String? = null

    suspend fun refresh() {
        val api = Session.api ?: run { isAdmin = false; checked = null; return }
        val who = api.baseUrl.toString() + api.token
        if (checked != who) isAdmin = false
        try {
            isAdmin = api.me().isAdmin == true
            checked = who
        } catch (e: app.moonrise.core.MoonriseHttpException) {
            if (e.code == 401) isAdmin = false
        } catch (e: Exception) {
            if (e is CancellationException) throw e
        }
    }
}

/** A push-only stack of pages over a screen (the iPhone's `NavigationStack` in Settings or a sheet): each keeps its state while others are on top. */
class PageStack {
    private var serial = 0
    private class Page(val key: Int, val content: @Composable () -> Unit)
    private val pages = mutableStateListOf<Page>()

    fun push(content: @Composable () -> Unit) { pages.add(Page(serial++, content)) }
    fun pop() { if (pages.isNotEmpty()) pages.removeAt(pages.lastIndex) }
    fun popAll() { pages.clear() }

    @Composable
    fun Host() {
        for (p in pages) key(p.key) {
            // The page takes the touches under it (nothing reaches the page below).
            Box(Modifier.fillMaxSize().background(Color.Black).pointerInput(Unit) {}) {
                BackHandler { pop() }
                CompositionLocalProvider(LocalPageShowing provides (p === pages.last())) { p.content() }
            }
        }
    }
}

val LocalPages = staticCompositionLocalOf<PageStack> { error("no PageStack") }

/** Whether this page is the one on screen. The pages under it stay composed, so an effect keyed on nothing doesn't run again on the way back. */
val LocalPageShowing = compositionLocalOf { true }

/** Runs a call of the admin API; a failure goes to `onError` as the server's reason (a cancelled one stays cancelled). Null when it failed. */
suspend fun <T> adminCall(onError: (String) -> Unit, block: suspend () -> T): T? = try {
    block()
} catch (e: Exception) {
    if (e is CancellationException) throw e
    onError(adminErrorText(e))
    null
}

@Composable
fun Spinner(size: androidx.compose.ui.unit.Dp = 18.dp) = CircularProgressIndicator(color = Theme.secondaryText, strokeWidth = 2.dp, modifier = Modifier.size(size))

/** A progress row of its own (`ProgressView` in a section). */
@Composable
fun SpinnerRow() { FormRow { Spinner() } }

@Composable
fun SpinnerSection() { FormSection { SpinnerRow() } }

/** Text in a section of its own: a note, an error (`NoteRow`). */
@Composable
fun NoteSection(text: String, color: Color = Theme.secondaryText) { FormSection { NoteRow(text, color) } }

@Composable
fun NoteRow(text: String, color: Color = Theme.secondaryText) { FormRow { Text(text, style = Type.callout, color = color) } }

@Composable
fun Chevron() = Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, null, tint = Theme.secondaryText.copy(alpha = 0.6f), modifier = Modifier.size(20.dp))

/** A row that opens a screen: icon, title, a detail on the right (`ManageLink`). */
@Composable
fun ManageLink(title: String, icon: ImageVector? = null, detail: String? = null, detailColor: Color = Theme.secondaryText, onClick: () -> Unit) {
    FormRow(onClick = onClick) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            if (icon != null) Icon(icon, null, tint = Color.White, modifier = Modifier.size(24.dp))
            Text(title, style = Type.body, color = Color.White, modifier = Modifier.weight(1f))
            if (detail != null) Text(detail, style = Type.body, color = detailColor, maxLines = 1)
            Chevron()
        }
    }
}

/** A plain action row: an icon and its title (a button with a label). */
@Composable
fun ActionRow(title: String, icon: ImageVector, enabled: Boolean = true, color: Color = Color.White, onClick: () -> Unit) {
    val c = if (enabled) color else Theme.secondaryText
    FormRow(onClick = onClick, enabled = enabled) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Icon(icon, null, tint = c, modifier = Modifier.size(24.dp))
            Text(title, style = Type.body, color = c)
        }
    }
}

/** A title on the left and its value on the right when both fit on one line, the value under it otherwise (`InfoRow`). */
@Composable
fun InfoRow(title: String, value: String?, icon: ImageVector? = null, tint: Color? = null) {
    FormRow {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            if (icon != null) Icon(icon, null, tint = tint ?: Color.White, modifier = Modifier.size(24.dp))
            Layout(content = {
                Text(title, style = Type.body, color = Color.White)
                if (value != null) Text(value, style = Type.body, color = tint ?: Theme.secondaryText)
            }, modifier = Modifier.weight(1f)) { measurables, constraints ->
                val gap = 12.dp.roundToPx()
                val free = constraints.copy(minWidth = 0, minHeight = 0, maxWidth = constraints.maxWidth, maxHeight = Int.MAX_VALUE)
                val titleP = measurables[0].measure(free)
                val valueP = measurables.getOrNull(1)?.measure(free)
                val oneLine = valueP == null || titleP.width + gap + valueP.width <= constraints.maxWidth
                val h = if (oneLine) maxOf(titleP.height, valueP?.height ?: 0) else titleP.height + (valueP?.height ?: 0) + 3.dp.roundToPx()
                layout(constraints.maxWidth, h) {
                    if (oneLine) {
                        titleP.placeRelative(0, (h - titleP.height) / 2)
                        valueP?.placeRelative(constraints.maxWidth - valueP.width, (h - valueP.height) / 2)
                    } else {
                        titleP.placeRelative(0, 0)
                        valueP?.placeRelative(0, titleP.height + 3.dp.roundToPx())
                    }
                }
            }
        }
    }
}

/** A title on one line, a detail under it (`InfoRowStack`). */
@Composable
fun InfoRowStack(title: String, detail: String, icon: ImageVector? = null, trailing: (@Composable () -> Unit)? = null) {
    FormRow {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            if (icon != null) Icon(icon, null, tint = Color.White, modifier = Modifier.size(24.dp))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Text(title, style = Type.body, color = Color.White)
                Text(detail, style = Type.caption, color = Theme.secondaryText)
            }
            trailing?.invoke()
        }
    }
}

/** A primary action row with a spinner while it runs (`BusyLabel`). */
@Composable
fun BusyRow(title: String, busy: Boolean, enabled: Boolean = true, onClick: () -> Unit) {
    val can = enabled && !busy
    FormRow(onClick = onClick, enabled = can) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(title, style = Type.body, color = if (can) Color.White else Theme.secondaryText, modifier = Modifier.weight(1f))
            if (busy) Spinner()
        }
    }
}

@Composable
fun ToggleRow(title: String, on: Boolean, enabled: Boolean = true, icon: ImageVector? = null, set: (Boolean) -> Unit) {
    FormRow(onClick = if (enabled) ({ set(!on) }) else null) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            if (icon != null) Icon(icon, null, tint = Color.White, modifier = Modifier.size(24.dp))
            Text(title, style = Type.body, color = if (enabled) Color.White else Theme.secondaryText, modifier = Modifier.weight(1f))
            Switch(on, if (enabled) set else null, enabled = enabled, modifier = Modifier.height(24.dp), colors = SwitchDefaults.colors(checkedTrackColor = Theme.green, checkedThumbColor = Color.White))
        }
    }
}

/** A row with its value and a menu of the choices (`Picker`, as a menu). `options`: label → value. */
@Composable
fun ChoiceRow(title: String, value: String, options: List<Pair<String, String>>, enabled: Boolean = true, pick: (String) -> Unit) {
    var open by remember { mutableStateOf(false) }
    FormRow(onClick = { open = true }, enabled = enabled) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(title, style = Type.body, color = if (enabled) Color.White else Theme.secondaryText, modifier = Modifier.weight(1f))
            Box {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(value, style = Type.body, color = Theme.secondaryText, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Icon(Icons.Filled.UnfoldMore, null, tint = Theme.secondaryText, modifier = Modifier.padding(start = 4.dp))
                }
                DropdownMenu(open, { open = false }, containerColor = Theme.sheet) {
                    for ((label, v) in options) MenuItem(label, checked = label == value) { open = false; pick(v) }
                }
            }
        }
    }
}

/** A `Stepper` row: the title, the value, and minus and plus, the value in step-sized jumps. */
@Composable
fun StepperRow(p: AdminRules.PerfSetting, value: Int, edited: Boolean, set: (Int) -> Unit) {
    val suffix = if (p.unit.isEmpty()) "" else " ${p.unit}"
    FormRow {
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(p.title, style = Type.body, color = Color.White, modifier = Modifier.weight(1f))
                Text("$value$suffix", style = Type.body, color = if (edited) Color.White else Theme.secondaryText)
                if (!p.locked) {
                    Row(Modifier.clip(CircleShape).background(Theme.surfaceStrong), verticalAlignment = Alignment.CenterVertically) {
                        Box(Modifier.size(36.dp).clickable(enabled = value - p.step >= p.range.first) { set(value - p.step) }, contentAlignment = Alignment.Center) { Icon(Icons.Filled.Remove, "Less", tint = Color.White, modifier = Modifier.size(18.dp)) }
                        Box(Modifier.size(36.dp).clickable(enabled = value + p.step <= p.range.last) { set(value + p.step) }, contentAlignment = Alignment.Center) { Icon(Icons.Filled.Add, "More", tint = Color.White, modifier = Modifier.size(18.dp)) }
                    }
                }
            }
            if (p.locked && p.env != null) Text("Set by ${p.env} in the server's environment", style = Type.caption, color = Warn)
            else if (p.pending) Text("Changed: waiting for a restart", style = Type.caption, color = Warn)
        }
    }
}

/** A log line: its level and time, then the message (`LogLine`). */
@Composable
fun LogLineRow(line: String) {
    val (time, level, message) = AdminRules.logLine(line)
    FormRow {
        Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(level, style = Type.caption, fontWeight = FontWeight.Bold, color = if (level == "ERROR") Theme.destructive else if (level == "WARN") Warn else Theme.secondaryText)
                Text(time, style = Type.caption, color = Theme.secondaryText)
            }
            Text(message, style = Type.caption, fontFamily = FontFamily.Monospace, color = Color.White, maxLines = 6, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** A confirmation (`confirmationDialog`): its title, why, and the actions; the last action is drawn as the iPhone's destructive one when asked. */
@Composable
fun ConfirmDialog(title: String, message: String?, action: String, destructive: Boolean = true, secondary: Pair<String, () -> Unit>? = null, dismiss: () -> Unit, confirm: () -> Unit) {
    AlertDialog(onDismissRequest = dismiss, containerColor = Theme.sheet, title = { Text(title, fontWeight = FontWeight.SemiBold) },
        text = if (message != null) ({ Text(message) }) else null,
        confirmButton = {
            Column(horizontalAlignment = Alignment.End) {
                if (secondary != null) TextButton(onClick = secondary.second) { Text(secondary.first, color = Color.White) }
                TextButton(onClick = confirm) { Text(action, color = if (destructive) Theme.destructive else Color.White) }
            }
        },
        dismissButton = { TextButton(onClick = dismiss) { Text("Cancel", color = Color.White) } })
}

/** A label on the left and its field on the right, the text at the trailing edge (`LabeledContent` with a `TextField`). */
@Composable
fun InputRow(label: String, value: String, set: (String) -> Unit, placeholder: String, keyboard: androidx.compose.ui.text.input.KeyboardType = androidx.compose.ui.text.input.KeyboardType.Text, onDone: () -> Unit = {}) {
    FormRow {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(label, style = Type.body, color = Color.White)
            androidx.compose.foundation.text.BasicTextField(value, set, singleLine = true,
                textStyle = Type.body.copy(color = Theme.secondaryText, textAlign = androidx.compose.ui.text.style.TextAlign.End),
                cursorBrush = androidx.compose.ui.graphics.SolidColor(Color.White),
                keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(keyboardType = keyboard, imeAction = androidx.compose.ui.text.input.ImeAction.Search, autoCorrectEnabled = false),
                keyboardActions = androidx.compose.foundation.text.KeyboardActions(onSearch = { onDone() }),
                modifier = Modifier.weight(1f),
                decorationBox = { inner ->
                    Box(contentAlignment = Alignment.CenterEnd) {
                        if (value.isEmpty()) Text(placeholder, style = Type.body, color = Theme.secondaryText.copy(alpha = 0.5f))
                        inner()
                    }
                })
        }
    }
}

@Composable
fun StatusDot(online: Boolean) = Box(Modifier.size(8.dp).clip(CircleShape).background(if (online) Theme.green else Theme.destructive))

@Composable
fun ColumnSpacer(height: androidx.compose.ui.unit.Dp = 0.dp) = Spacer(Modifier.height(height).fillMaxWidth())
