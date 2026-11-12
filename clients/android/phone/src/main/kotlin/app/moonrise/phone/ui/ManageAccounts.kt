// SPDX-License-Identifier: MPL-2.0
package app.moonrise.phone.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.AccountCircle
import androidx.compose.material.icons.outlined.AdminPanelSettings
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.Drafts
import androidx.compose.material.icons.outlined.Key
import androidx.compose.material.icons.outlined.Laptop
import androidx.compose.material.icons.outlined.PersonAdd
import androidx.compose.material.icons.outlined.Share
import androidx.compose.material.icons.outlined.Smartphone
import androidx.compose.material.icons.outlined.Tablet
import androidx.compose.material.icons.outlined.Tv
import androidx.compose.material.icons.outlined.VerifiedUser
import androidx.compose.material.icons.outlined.Computer
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.moonrise.core.Account
import app.moonrise.core.AccountDevice
import app.moonrise.core.AdminFmt
import app.moonrise.core.AdminRules
import app.moonrise.core.Invite
import app.moonrise.core.accounts
import app.moonrise.core.createAccount
import app.moonrise.core.createInvite
import app.moonrise.core.deleteAccount
import app.moonrise.core.deleteInvite
import app.moonrise.core.devices
import app.moonrise.core.invites
import app.moonrise.core.signOutDevice
import app.moonrise.core.updateAccount
import app.moonrise.shared.Session
import kotlinx.coroutines.launch
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

/** Users & Devices (`AccountsAdminView`): the accounts, new accounts, invitations. */
@Composable
fun AccountsPage() {
    val api = Session.api ?: return
    val pages = LocalPages.current
    val scope = rememberCoroutineScope()
    var accounts by remember { mutableStateOf<List<Account>>(emptyList()) }
    var invites by remember { mutableStateOf<List<Invite>>(emptyList()) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }
    var inviteAdmin by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var reloads by remember { mutableStateOf(0) }
    LaunchedEffect(reloads) {
        loading = true
        adminCall({ error = it }) { accounts = api.accounts(); invites = api.invites(); error = null }
        loading = false
    }
    FormPage("Users & Devices", pages::pop) {
        FormSection("Users") {
            if (loading && accounts.isEmpty()) SpinnerRow()
            accounts.forEachIndexed { i, a ->
                if (i > 0) FormDivider()
                AccountRow(a) { pages.push { AccountDetailPage(a) { reloads++ } } }
            }
            if (accounts.isNotEmpty()) FormDivider()
            ActionRow("Add User", Icons.Outlined.PersonAdd) { pages.push { NewAccountPage { pages.pop(); reloads++ } } }
        }
        FormSection("Invitations", footer = "An invitation is a one-time code, good for 7 days. The new person opens Moonrise, picks this server and taps Have an Invitation, then chooses their own user name and password.") {
            invites.forEachIndexed { i, inv ->
                key(inv.id) {
                    if (i > 0) FormDivider()
                    SwipeToRemove({ scope.launch { adminCall({ error = it }) { api.deleteInvite(inv.id); invites = invites.filterNot { it.id == inv.id } } } }) {
                        FormRow {
                            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                Icon(Icons.Outlined.Drafts, null, tint = Color.White, modifier = Modifier.size(24.dp))
                                Text(if (inv.admin) "Admin Invitation" else "Invitation", style = Type.body, color = Color.White, modifier = Modifier.weight(1f))
                                Text("expires ${AdminFmt.ago(inv.expiresAt) ?: inv.expiresAt}", style = Type.body, color = Theme.secondaryText)
                            }
                        }
                    }
                }
            }
            if (invites.isNotEmpty()) FormDivider()
            ToggleRow("New Invitations Make Admins", inviteAdmin) { inviteAdmin = it }
            FormDivider()
            BusyRow("Invite Someone", busy) {
                busy = true
                scope.launch {
                    adminCall({ error = it }) {
                        val inv = api.createInvite(inviteAdmin)
                        invites = runCatching { api.invites() }.getOrDefault(invites)
                        pages.push { InviteCodePage(inv) }
                    }
                    busy = false
                }
            }
        }
        error?.let { NoteSection(it, Theme.destructive) }
    }
}

@Composable
private fun AccountRow(a: Account, onClick: () -> Unit) {
    FormRow(onClick = onClick) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Icon(if (a.isAdmin) Icons.Outlined.VerifiedUser else Icons.Outlined.AccountCircle, null, tint = Color.White, modifier = Modifier.size(24.dp))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(a.name, style = Type.body, color = Color.White)
                    if (a.me) Text("You", style = Type.caption2, fontWeight = FontWeight.SemiBold, color = Color.Black, modifier = Modifier.clip(CircleShape).background(Color.White).padding(horizontal = 6.dp, vertical = 1.dp))
                }
                Text(AdminRules.accountDetail(a), style = Type.caption, color = Theme.secondaryText)
            }
            Chevron()
        }
    }
}

private fun deviceIcon(name: String?) = (name ?: "").lowercase().let { n ->
    when {
        "tv" in n -> Icons.Outlined.Tv
        "ipad" in n || "tablet" in n -> Icons.Outlined.Tablet
        "mac" in n -> Icons.Outlined.Laptop
        "iphone" in n || "pixel" in n || "phone" in n -> Icons.Outlined.Smartphone
        else -> Icons.Outlined.Computer
    }
}

/** One account: its devices (sign one out), password, admin rights, delete (`AccountDetailView`). */
@Composable
private fun AccountDetailPage(account: Account, changed: () -> Unit) {
    val api = Session.api ?: return
    val pages = LocalPages.current
    val scope = rememberCoroutineScope()
    var devices by remember { mutableStateOf<List<AccountDevice>>(emptyList()) }
    var isAdmin by remember { mutableStateOf(account.isAdmin) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }
    var note by remember { mutableStateOf<String?>(null) }
    var confirmDelete by remember { mutableStateOf(false) }
    var signingOut by remember { mutableStateOf<AccountDevice?>(null) }
    LaunchedEffect(Unit) {
        adminCall({ error = it }) { devices = api.devices(account.id); error = null }
        loading = false
    }
    FormPage(account.name, pages::pop) {
        note?.let { NoteSection(it, Theme.green) }
        error?.let { NoteSection(it, Theme.destructive) }
        FormSection(footer = if (account.me) "You can't take away your own admin rights." else "Admins manage the server from Settings → Manage Server.") {
            ToggleRow("Admin", isAdmin, enabled = !account.me, icon = Icons.Outlined.AdminPanelSettings) { v ->
                val old = isAdmin
                isAdmin = v
                scope.launch { if (adminCall({ error = it; isAdmin = old }) { api.updateAccount(account.id, admin = v); error = null; changed() } == null) isAdmin = old }
            }
            FormDivider()
            ActionRow("Reset Password", Icons.Outlined.Key) { pages.push { ResetPasswordPage(account) { pages.pop(); note = "Password changed." } } }
        }
        FormSection("Signed In On", footer = "Signing a device out ends its sign-in at once; it can sign in again with the password.") {
            if (loading && devices.isEmpty()) SpinnerRow()
            if (!loading && devices.isEmpty()) NoteRow("Not signed in anywhere.")
            devices.forEachIndexed { i, d ->
                if (i > 0) FormDivider()
                InfoRowStack(d.deviceName ?: "A device", AdminRules.deviceDetail(d), deviceIcon(d.deviceName), trailing = if (!d.current) ({
                    Text("Sign Out", style = Type.callout, color = Theme.destructive, modifier = Modifier.clickable { signingOut = d })
                }) else null)
            }
        }
        if (!account.me) {
            FormSection(footer = "Their watch history goes with the account; their devices are signed out.") {
                FormRow(onClick = { confirmDelete = true }) { Text("Delete ${account.name}", style = Type.body, color = Theme.destructive) }
            }
        }
    }
    if (confirmDelete) ConfirmDialog("Delete ${account.name}?", "Their watch history is deleted and every device they're signed in on is signed out.", "Delete Account", dismiss = { confirmDelete = false }) {
        confirmDelete = false
        scope.launch { adminCall({ error = it }) { api.deleteAccount(account.id); changed(); pages.pop() } }
    }
    signingOut?.let { d ->
        ConfirmDialog("Sign out ${d.deviceName ?: "this device"}?", null, "Sign Out", dismiss = { signingOut = null }) {
            signingOut = null
            scope.launch {
                adminCall({ error = it }) {
                    api.signOutDevice(account.id, d.id)
                    devices = devices.filterNot { it.id == d.id }
                    note = "${d.deviceName ?: "The device"} is signed out."
                    changed()
                }
            }
        }
    }
}

@Composable
private fun TextAction(title: String, enabled: Boolean, busy: Boolean = false, onClick: () -> Unit) {
    if (busy) GlassCapsule(onClick = {}) { Spinner(20.dp) }
    else GlassCapsule(onClick = { if (enabled) onClick() }) { Text(title, style = Type.headline, color = if (enabled) Color.White else Theme.secondaryText) }
}

/** A new account: a user name, a password (at least 8 characters), admin or not (`NewAccountView`). */
@Composable
private fun NewAccountPage(done: () -> Unit) {
    val api = Session.api ?: return
    val pages = LocalPages.current
    val scope = rememberCoroutineScope()
    var name by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var again by remember { mutableStateOf("") }
    var admin by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val problem = AdminRules.newAccountProblem(name, password, again)
    fun create() {
        busy = true
        scope.launch {
            adminCall({ error = it }) { api.createAccount(name.trim(), password, admin); done() }
            busy = false
        }
    }
    FormPage("New User", pages::pop, backLabel = "Cancel", action = { TextAction("Add", problem == null, busy) { create() } }) {
        FormSection(footer = if (!problem.isNullOrEmpty() && password.isNotEmpty()) problem else "They sign in with these on their devices. Admins can manage the server.") {
            FormField(name, { name = it }, "User name")
            FormDivider()
            FormField(password, { password = it }, "Password", secure = true, keyboard = KeyboardType.Password)
            FormDivider()
            FormField(again, { again = it }, "Password again", secure = true, keyboard = KeyboardType.Password, ime = ImeAction.Done)
            FormDivider()
            ToggleRow("Admin", admin) { admin = it }
        }
        error?.let { NoteSection(it, Theme.destructive) }
    }
}

@Composable
private fun ResetPasswordPage(account: Account, done: () -> Unit) {
    val api = Session.api ?: return
    val pages = LocalPages.current
    val scope = rememberCoroutineScope()
    var password by remember { mutableStateOf("") }
    var again by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val problem = AdminRules.resetPasswordProblem(password, again)
    fun save() {
        busy = true
        scope.launch {
            adminCall({ error = it }) { api.updateAccount(account.id, password = password); done() }
            busy = false
        }
    }
    FormPage("Password for ${account.name}", pages::pop, backLabel = "Cancel", action = { TextAction("Save", problem == null, busy) { save() } }) {
        FormSection(footer = if (problem != null && password.isNotEmpty()) problem else "Devices already signed in stay signed in.") {
            FormField(password, { password = it }, "New password", secure = true, keyboard = KeyboardType.Password)
            FormDivider()
            FormField(again, { again = it }, "New password again", secure = true, keyboard = KeyboardType.Password, ime = ImeAction.Done)
        }
        error?.let { NoteSection(it, Theme.destructive) }
    }
}

/** The new invitation's code (shown once), to read out or share (`InviteCodeView`). */
@Composable
private fun InviteCodePage(invite: Invite) {
    val pages = LocalPages.current
    val context = LocalContext.current
    val code = invite.code.orEmpty()
    val server = remember { mutableStateOf("the server") }
    LaunchedEffect(Unit) { runCatching { Session.api?.info()?.name }.getOrNull()?.let { server.value = it } }
    val until = AdminFmt.date(invite.expiresAt)?.let { DateTimeFormatter.ofPattern("MMM d, yyyy", Locale.US).withZone(ZoneId.systemDefault()).format(it) } ?: "it expires"
    val shareText = "Join ${server.value} in Moonrise: open Moonrise, pick ${server.value}, tap Have an Invitation and enter $code. The code works once, for 7 days."
    FormPage("Invitation", pages::pop, backLabel = "Done") {
        FormSection(footer = "Works once, until $until${if (invite.admin) ", for an admin account" else ""}. It isn't shown again.") {
            Box(Modifier.fillMaxWidth().padding(vertical = 12.dp), contentAlignment = Alignment.Center) {
                Text(code, fontSize = 34.sp, fontWeight = FontWeight.Bold, fontFamily = FontFamily.Monospace, color = Color.White, textAlign = TextAlign.Center)
            }
        }
        FormSection {
            ActionRow("Share Invitation", Icons.Outlined.Share) {
                context.startActivity(Intent.createChooser(Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, shareText), null))
            }
            FormDivider()
            ActionRow("Copy Code", Icons.Outlined.ContentCopy) {
                (context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager).setPrimaryClip(ClipData.newPlainText("Invitation code", code))
            }
        }
    }
}
