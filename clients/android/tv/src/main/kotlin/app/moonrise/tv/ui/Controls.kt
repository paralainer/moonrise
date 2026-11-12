package app.moonrise.tv.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusDirection
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.Button
import androidx.tv.material3.ButtonDefaults
import androidx.tv.material3.Glow
import androidx.tv.material3.Text

/**
 * The tvOS "glass" button: translucent white at rest, solid white with black text when focused,
 * slightly larger. `prominent`: the screen's main action, full width.
 */
@Composable
fun GlassButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    /** What the focus log calls it (`MoonriseFocus`). */
    name: String? = null,
    content: @Composable RowScope.() -> Unit,
) {
    // The look follows the focus system's own state (`onFocusChanged`), not the button's focus
    // interactions, which a button focused as it appears (an error's Retry) could miss.
    var focused by remember { mutableStateOf(false) }
    val container by androidx.compose.animation.animateColorAsState(if (focused) Color.White else Color.White.copy(alpha = 0.14f), FocusMotion.spec(), label = "button")
    val text by androidx.compose.animation.animateColorAsState(if (focused) Color.Black else Color.White, FocusMotion.spec(), label = "button text")
    val glow = if (focused) Glow(Color.White.copy(alpha = 0.25f), 8.dp) else Glow.None
    val scale by androidx.compose.animation.core.animateFloatAsState(if (focused) 1.06f else 1f, FocusMotion.spec(), label = "button scale")
    Button(
        onClick = onClick,
        modifier = modifier.onFocusChanged { focused = it.isFocused }.logFocus { name ?: "button" },
        enabled = enabled,
        shape = ButtonDefaults.shape(shape = RoundedCornerShape(50)),
        colors = ButtonDefaults.colors(
            containerColor = container, contentColor = text,
            focusedContainerColor = container, focusedContentColor = text,
            pressedContainerColor = container, pressedContentColor = text,
            disabledContainerColor = Color.White.copy(alpha = 0.06f),
            disabledContentColor = Color.White.copy(alpha = 0.4f),
        ),
        scale = ButtonDefaults.scale(scale = scale, focusedScale = scale, pressedScale = scale),
        glow = ButtonDefaults.glow(glow = glow, focusedGlow = glow, pressedGlow = glow),
        contentPadding = PaddingValues(horizontal = 22.dp, vertical = 10.dp),
        content = content,
    )
}

/**
 * A labelled text field for the remote: a rounded field that lights up when focused. As on Apple
 * TV, focus only highlights it; selecting it opens the keyboard. Done or Back closes the keyboard
 * and leaves focus on the field; up and down move on.
 */
@Composable
fun TvTextField(
    label: String,
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    placeholder: String = "",
    secure: Boolean = false,
    keyboardType: KeyboardType = KeyboardType.Text,
    onDone: () -> Unit = {},
    /** Focuses the field (at rest), for screens that move focus to it. */
    focusRequester: FocusRequester? = null,
) {
    var editFocused by remember { mutableStateOf(false) }
    var editing by remember { mutableStateOf(false) }
    var refocus by remember { mutableStateOf(false) }
    // Up or down on the remote while typing: focus goes on to the next control.
    var movingOn by remember { mutableStateOf(false) }
    // Done pressed: focus goes back to the field at rest, then the text field goes.
    var closing by remember { mutableStateOf(false) }
    val ownField = remember { FocusRequester() }
    val field = focusRequester ?: ownField
    val edit = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    val focusManager = LocalFocusManager.current
    val fieldInteraction = remember { MutableInteractionSource() }
    // The text field's own focus (its interactions don't report it while the keyboard is up).
    // (From the focus system itself: focus interactions could be missed.)
    var restFocused by remember { mutableStateOf(false) }
    val focused = (restFocused && !editing) || editFocused
    val shown = if (secure) PasswordVisualTransformation() else VisualTransformation.None

    LaunchedEffect(editing) {
        if (editing) {
            // The field is made this frame (in a lazy list, a frame or two later): until it takes
            // focus, try again, so focus isn't left with nothing focused.
            for (i in 0 until 20) {
                androidx.compose.runtime.withFrameNanos { }
                if (runCatching { edit.requestFocus() }.isFailure) continue
                androidx.compose.runtime.withFrameNanos { }
                if (editFocused) break
            }
            keyboard?.show()
        } else if (refocus) {
            refocus = false
            runCatching { field.requestFocus() }
        }
    }
    LaunchedEffect(closing) {
        if (closing) {
            androidx.compose.runtime.withFrameNanos { }
            if (runCatching { field.requestFocus() }.isFailure) { closing = false; refocus = true; editing = false }
        }
    }

    @Composable
    fun Frame(content: @Composable () -> Unit) {
        Box(
            Modifier
                .fillMaxWidth()
                .height(44.dp)
                .background(if (focused) Color.White else Theme.surfaceStrong, RoundedCornerShape(12.dp))
                .border(1.dp, if (focused) Color.White else Color.Transparent, RoundedCornerShape(12.dp))
                .padding(horizontal = 16.dp),
            contentAlignment = Alignment.CenterStart,
        ) {
            if (value.isEmpty() && placeholder.isNotEmpty()) {
                Text(placeholder, fontSize = 17.sp, color = if (focused) Color.Black.copy(alpha = 0.4f) else Theme.secondaryText.copy(alpha = 0.5f))
            }
            content()
        }
    }

    val textStyle = TextStyle(color = if (focused) Color.Black else Color.White, fontSize = 17.sp)
    Column(modifier) {
        Text(label, fontSize = 13.sp, color = Theme.secondaryText, modifier = Modifier.padding(start = 12.dp, bottom = 4.dp))
        // The field at rest stays until the text field has taken focus from it (and comes back
        // before the text field goes), so focus never drops to nothing in between, which sent it
        // to the tab bar for a moment.
        Box {
            if (editing) {
                BasicTextField(
                    value = value,
                    onValueChange = onValueChange,
                    singleLine = true,
                    textStyle = textStyle,
                    cursorBrush = SolidColor(if (focused) Color.Black else Color.White),
                    visualTransformation = shown,
                    keyboardOptions = KeyboardOptions(
                        keyboardType = if (secure) KeyboardType.Password else keyboardType,
                        imeAction = ImeAction.Done,
                        autoCorrectEnabled = false,
                    ),
                    keyboardActions = KeyboardActions(onDone = {
                        keyboard?.hide()
                        closing = true
                        onDone()
                    }),
                    // Up and down leave the field for the controls above and below (a single-line field
                    // would otherwise keep them for its cursor); leaving it ends the editing.
                    modifier = Modifier
                        .fillMaxWidth()
                        .focusRequester(edit)
                        .onFocusChanged {
                            if (it.hasFocus) editFocused = true
                            else if (editFocused) {
                                // Focus taken some other way than up or down: put it back on the field.
                                editFocused = false
                                refocus = !movingOn && !closing
                                movingOn = false
                                closing = false
                                editing = false
                            }
                        }
                        .onPreviewKeyEvent { e ->
                            if (e.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                            when (e.key) {
                                // Select on the field (after Back closed the keyboard) opens it again.
                                Key.DirectionCenter, Key.Enter, Key.NumPadEnter -> { keyboard?.show(); true }
                                Key.DirectionDown -> { movingOn = true; focusManager.moveFocus(FocusDirection.Down).also { if (!it) movingOn = false } }
                                Key.DirectionUp -> { movingOn = true; focusManager.moveFocus(FocusDirection.Up).also { if (!it) movingOn = false } }
                                else -> false
                            }
                        },
                    decorationBox = { inner -> Frame(inner) },
                )
            }
            if (!editing || !editFocused || closing) {
                // At rest: a button showing the text. Select opens the keyboard.
                Box(
                    Modifier
                        .fillMaxWidth()
                        .focusRequester(field)
                        .onFocusChanged { restFocused = it.isFocused }
                        .logFocus { "field $label" }
                        .clickable(interactionSource = fieldInteraction, indication = null) { editing = true },
                ) {
                    Frame {
                        if (value.isNotEmpty()) {
                            Text(shown.filter(AnnotatedString(value)).text, style = textStyle, maxLines = 1)
                        }
                    }
                }
            }
        }
    }
}
