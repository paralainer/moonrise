// SPDX-License-Identifier: MPL-2.0
package app.moonrise.phone.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.moonrise.core.AdminRules
import app.moonrise.core.AdminService
import app.moonrise.core.LibraryRoot
import app.moonrise.core.ManagedLibrary
import app.moonrise.core.MoonriseApi
import app.moonrise.core.MoonriseHttpException
import app.moonrise.core.ScanProgress
import app.moonrise.core.createLibrary
import app.moonrise.core.scanProgress
import app.moonrise.core.services
import app.moonrise.core.updateSettings
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

// Setting up a new server from the app (`SetupViews.swift`, iPhone): the setup code → the admin
// account → the first library → the first scan, live → optional extras → Home. Signing in as an
// admin to a server without libraries continues at the library step (`SetupLibrary`).

private const val STEPS = 5

/** The first row of a setup step: the step number and what it is about (`SetupHeader`). */
@Composable
private fun SetupHeader(step: Int, message: String) {
    Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("STEP $step OF $STEPS", style = Type.caption, fontWeight = FontWeight.SemiBold, color = Theme.secondaryText)
        Text(message, style = Type.body, color = Theme.secondaryText)
    }
}

/** Set Up This Server: the setup code (`SetupCodeView`), then the account (`SetupAccountView`). */
@Composable
internal fun SetupCodePage(client: MoonriseApi, address: String, back: () -> Unit, verified: (String) -> Unit) {
    val scope = rememberCoroutineScope()
    var code by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val digits = AdminRules.setupCodeDigits(code)
    fun verify() {
        if (digits.length != 6 || busy) return
        busy = true
        scope.launch {
            try {
                if (client.verifySetupCode(digits)) { error = null; verified(digits) }
                else error = "That isn't the code. Check the server's log or its page, and try again."
            } catch (e: MoonriseHttpException) {
                error = when (e.code) { 409 -> "This server has been set up already. Go back and sign in."; 429 -> "Too many wrong codes. Wait a few minutes; the server may show a new code."; else -> app.moonrise.core.adminErrorText(e) }
            } catch (e: Exception) {
                if (e is CancellationException) throw e
                error = app.moonrise.core.adminErrorText(e)
            } finally { busy = false }
        }
    }
    LaunchedEffect(digits) { if (digits.length == 6) verify() }
    FormPage("Set Up Server", back) {
        SetupHeader(1, "This server is new. To show it's yours, enter the 6-digit setup code from the server's log, or open http://$address in a browser on your network.")
        FormSection("Setup Code", footer = "The code changes when the server restarts.") {
            FormField(code, { code = AdminRules.setupCodeDigits(it) }, "123 456", keyboard = KeyboardType.Number, ime = ImeAction.Go, onDone = { verify() })
            FormDivider()
            BusyRow("Continue", busy, enabled = digits.length == 6) { verify() }
        }
        error?.let { NoteSection(it, Theme.destructive) }
    }
}

@Composable
internal fun SetupAccountPage(client: MoonriseApi, code: String, claimedAlready: Boolean, back: () -> Unit, claimed: () -> Unit) {
    val scope = rememberCoroutineScope()
    var username by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var confirm by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val problem = AdminRules.setupAccountProblem(username, password, confirm)
    fun create() {
        if (claimedAlready) { claimed(); return }
        if (problem != null || busy) return
        busy = true
        scope.launch {
            try {
                client.claim(code, username.trim(), password)
                error = null
                claimed()
            } catch (e: MoonriseHttpException) {
                error = when (e.code) {
                    403 -> "The setup code has changed (the server restarted, or it was guessed at). Go back and enter the new one."
                    409 -> "This server has just been set up from another device. Go back and sign in."
                    else -> app.moonrise.core.adminErrorText(e)
                }
            } catch (e: Exception) {
                if (e is CancellationException) throw e
                error = app.moonrise.core.adminErrorText(e)
            } finally { busy = false }
        }
    }
    FormPage("Your Account", back) {
        SetupHeader(2, "Create the server's first account. It's the admin: it manages the server from this app. Use it to sign in on your other devices too.")
        FormSection("Admin Account", footer = if (!problem.isNullOrEmpty() && password.isNotEmpty()) problem else "At least 8 characters.") {
            FormField(username, { username = it }, "User name")
            FormDivider()
            FormField(password, { password = it }, "Password", secure = true, keyboard = KeyboardType.Password)
            FormDivider()
            FormField(confirm, { confirm = it }, "Repeat password", secure = true, keyboard = KeyboardType.Password, ime = ImeAction.Done, onDone = { create() })
        }
        FormSection {
            BusyRow(if (claimedAlready) "Continue" else "Create Account", busy, enabled = claimedAlready || problem == null) { create() }
        }
        error?.let { NoteSection(it, Theme.destructive) }
    }
}

/** Step 3: the first library (`SetupLibraryView`); a server without libraries comes here after an admin signs in. */
@Composable
internal fun SetupLibraryPage(client: MoonriseApi, added: (ManagedLibrary, Int) -> Unit) {
    val scope = rememberCoroutineScope()
    var name by remember { mutableStateOf("Media") }
    var root by remember { mutableStateOf<LibraryRoot?>(null) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val pages = remember { PageStack() }
    androidx.compose.runtime.CompositionLocalProvider(LocalPages provides pages) {
        androidx.compose.foundation.layout.Box {
            FormPage("Add Your Videos", back = {}, backLabel = "") {
                SetupHeader(3, "Where are your videos? Pick a folder the server's computer can see, or a share on your NAS or another computer. Movies and shows can share a folder: Moonrise tells them apart. You can add more folders later.")
                LibraryForm(client, name, { name = it }, root, { root = it })
                FormSection {
                    BusyRow("Add Library", busy, enabled = root != null && name.isNotBlank()) {
                        val r = root ?: return@BusyRow
                        busy = true
                        scope.launch {
                            adminCall({ error = it }) {
                                val before = runCatching { client.scanProgress().passes }.getOrDefault(0)
                                val lib = client.createLibrary(name.trim(), r)
                                error = null
                                added(lib, before)
                            }
                            busy = false
                        }
                    }
                }
                error?.let { NoteSection(it, Theme.destructive) }
            }
            pages.Host()
        }
    }
}

@Composable
private fun StatsRow(stats: List<Pair<Int, String>>) {
    Row(horizontalArrangement = Arrangement.spacedBy(36.dp)) {
        stats.forEach { (n, label) ->
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text("$n", fontSize = 44.sp, fontWeight = FontWeight.Bold, color = Color.White)
                Text(label, style = Type.callout, color = Theme.secondaryText)
            }
        }
    }
}

/** Step 4: the first scan, live (`SetupScanView`). */
@Composable
internal fun SetupScanPage(client: MoonriseApi, library: ManagedLibrary?, passesBefore: Int, next: () -> Unit) {
    var progress by remember { mutableStateOf<ScanProgress?>(null) }
    var idlePolls by remember { mutableIntStateOf(0) }
    var done by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        while (true) {
            runCatching { client.scanProgress() }.getOrNull()?.let { p ->
                progress = p
                val finished = p.phase == "idle" && p.passes > passesBefore
                idlePolls = if (finished) idlePolls + 1 else 0
                val files = p.libraries.firstOrNull { it.id == library?.id }?.files ?: p.libraries.firstOrNull()?.files ?: 0
                if (finished && (files > 0 || idlePolls >= 3)) done = true
            }
            delay(if (done) 5000 else 1000)
        }
    }
    val lib = progress?.libraries?.let { ls -> ls.firstOrNull { it.id == library?.id } ?: ls.firstOrNull() }
    val notes = AdminRules.setupNotes(progress, lib, done)
    FormPage("Scanning", back = {}, backLabel = "") {
        SetupHeader(4, "Moonrise is looking through the folder and recognizing your movies and shows.")
        FormSection(library?.name ?: "Library") {
            FormRow { StatsRow(AdminRules.setupStats(progress, lib)) }
            FormDivider()
            FormRow {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    if (done) Icon(Icons.Filled.CheckCircle, null, tint = Theme.green, modifier = Modifier.size(22.dp)) else Spinner()
                    Text(AdminRules.setupPhaseText(progress, done), style = Type.body, color = Color.White)
                }
            }
        }
        Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            notes.forEach { Text(it, style = Type.footnote, color = Theme.secondaryText) }
        }
        FormSection { FormRow(onClick = next) { Text("Continue", style = Type.body, color = Color.White) } }
    }
}

/** Step 5: optional extras (`SetupExtrasView`), then Home. */
@Composable
internal fun SetupExtrasPage(client: MoonriseApi, finish: () -> Unit) {
    val scope = rememberCoroutineScope()
    var services by remember { mutableStateOf<List<AdminService>>(emptyList()) }
    var loading by remember { mutableStateOf(true) }
    val fields = remember { ServiceFields() }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) {
        adminCall({ error = it }) { services = client.services() }
        loading = false
    }
    val changes = fields.changes
    FormPage("Extras", back = {}, backLabel = "") {
        SetupHeader(5, "Moonrise works without any of these. Each adds something; add your own keys or accounts now or skip — nothing here is required.")
        if (loading) SpinnerSection()
        services.forEach { s ->
            FormSection(s.name) {
                ServiceSummary(s)
                if (s.state != "off") fields.Rows(s.id, optional = true)
            }
        }
        error?.let { NoteSection(it, Theme.destructive) }
        FormSection {
            BusyRow(if (changes.isEmpty()) "Finish" else "Save and Finish", busy) {
                if (changes.isEmpty()) { finish(); return@BusyRow }
                busy = true
                scope.launch {
                    adminCall({ error = it }) { client.updateSettings(changes); finish() }
                    busy = false
                }
            }
            if (changes.isNotEmpty()) { FormDivider(); FormRow(onClick = finish) { Text("Skip", style = Type.body, color = Color.White) } }
        }
    }
}

private enum class SetupStep { Code, Account, Library, Scan, Extras }

/** The setup in progress (`SetupFlow`): the app only takes the server on when it's finished, so Home opens on a library. */
@Composable
internal fun SetupFlowScreen(client: MoonriseApi, address: String, back: () -> Unit, finished: () -> Unit, startAtLibrary: Boolean = false) {
    var step by remember { mutableStateOf(if (startAtLibrary) SetupStep.Library else SetupStep.Code) }
    var code by remember { mutableStateOf("") }
    var claimed by remember { mutableStateOf(startAtLibrary) }
    var library by remember { mutableStateOf<ManagedLibrary?>(null) }
    var passesBefore by remember { mutableIntStateOf(0) }
    val goBack: () -> Unit = {
        when (step) {
            SetupStep.Account -> step = SetupStep.Code
            SetupStep.Code -> back()
            else -> {}   // after the account, as on the iPhone (the back button is hidden).
        }
    }
    androidx.activity.compose.BackHandler(onBack = goBack)
    when (step) {
        SetupStep.Code -> SetupCodePage(client, address, goBack) { code = it; step = SetupStep.Account }
        SetupStep.Account -> SetupAccountPage(client, code, claimed, goBack) { claimed = true; step = SetupStep.Library }
        SetupStep.Library -> SetupLibraryPage(client) { lib, before -> library = lib; passesBefore = before; step = SetupStep.Scan }
        SetupStep.Scan -> SetupScanPage(client, library, passesBefore) { step = SetupStep.Extras }
        SetupStep.Extras -> SetupExtrasPage(client, finished)
    }
}

