// SPDX-License-Identifier: MPL-2.0
package app.moonrise.phone.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.Storage
import androidx.compose.material.icons.outlined.SubdirectoryArrowLeft
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.moonrise.core.AdminRules
import app.moonrise.core.FileServer
import app.moonrise.core.FolderListing
import app.moonrise.core.LibraryRoot
import app.moonrise.core.MoonriseApi
import app.moonrise.core.Share
import app.moonrise.core.ShareListing
import app.moonrise.core.ShareLogin
import app.moonrise.core.adminErrorText
import app.moonrise.core.browseServer
import app.moonrise.core.browseShare
import app.moonrise.core.discoverFileServers
import app.moonrise.core.shareProblem
import app.moonrise.core.shares
import app.moonrise.core.testShare
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** A folder or share row that opens the next level (`folderRow`). */
@Composable
private fun FolderRow(name: String, icon: androidx.compose.ui.graphics.vector.ImageVector = Icons.Outlined.Folder, onClick: () -> Unit) {
    FormRow(onClick = onClick) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Icon(icon, null, tint = Color.White, modifier = Modifier.size(24.dp))
            Text(name, style = Type.body, color = Color.White, modifier = Modifier.weight(1f))
            Chevron()
        }
    }
}

/** Use This Folder. */
@Composable
private fun UseRow(busy: Boolean = false, onClick: () -> Unit) {
    FormRow(onClick = onClick, enabled = !busy) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Icon(Icons.Outlined.CheckCircle, null, tint = Color.White, modifier = Modifier.size(24.dp))
            Text("Use This Folder", style = Type.body, color = Color.White, modifier = Modifier.weight(1f))
            if (busy) Spinner()
        }
    }
}

/** Browses the server's own folders (`ServerFolderPicker`) and picks one. */
@Composable
fun ServerFolderPicker(client: MoonriseApi, back: () -> Unit, onPick: (LibraryRoot) -> Unit) {
    var path by remember { mutableStateOf<String?>(null) }
    var listing by remember { mutableStateOf<FolderListing?>(null) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(path) {
        loading = true
        adminCall({ error = it }) { listing = client.browseServer(path); error = null }
        loading = false
    }
    val current = path
    val title = current?.trimEnd('/')?.substringAfterLast('/')?.ifEmpty { current } ?: "Server Folders"
    FormPage(title, back) {
        val l = listing
        if (current != null && l != null) {
            FormSection(current, footer = AdminRules.videosText(l.videoFiles, l.dirs.size)) {
                UseRow { onPick(LibraryRoot(false, current, videoFiles = l.videoFiles)) }
            }
        }
        val empty = if (current == null) "The server can't see any folders. Make your videos available to it (in Docker: mount the folder into the container), or use a network share." else "No folders in here."
        FormSection(if (current == null) "Folders on the Server" else "Folders", footer = when {
            error != null -> error
            !loading && l?.dirs?.isEmpty() == true -> empty
            current == null && !loading -> "Open the folder that holds your videos, then choose it."
            else -> null
        }) {
            if (current != null) {
                FolderRow("Up", Icons.Outlined.SubdirectoryArrowLeft) { path = l?.parent }
                if (l?.dirs?.isNotEmpty() == true) FormDivider()
            }
            l?.dirs?.forEachIndexed { i, d ->
                if (i > 0) FormDivider()
                FolderRow(d.name) { path = d.path }
            }
            if (loading) { if (current != null || l?.dirs?.isNotEmpty() == true) FormDivider(); SpinnerRow() }
        }
    }
}

private sealed interface ShareStage {
    data object Servers : ShareStage
    data class Login(val host: String) : ShareStage
    data class Shares(val host: String) : ShareStage
    data class Folders(val host: String, val share: String, val path: String) : ShareStage
}

/**
 * Picks a folder in a network share through the server (it reads the share itself): file servers it
 * sees on its network or an address → shares → folders. A user name and password only when the share
 * turns guests away (`SharePicker`).
 */
@Composable
fun SharePicker(client: MoonriseApi, back: () -> Unit, onPick: (LibraryRoot) -> Unit) {
    var stage by remember { mutableStateOf<ShareStage>(ShareStage.Servers) }
    var found by remember { mutableStateOf<List<FileServer>>(emptyList()) }
    var discovering by remember { mutableStateOf(true) }
    var address by remember { mutableStateOf("") }
    var username by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var login by remember { mutableStateOf<ShareLogin?>(null) }
    var shares by remember { mutableStateOf<List<Share>>(emptyList()) }
    var listing by remember { mutableStateOf<ShareListing?>(null) }
    var pending by remember { mutableStateOf<Pair<String, String>?>(null) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    fun problemText(e: Throwable): String = e.shareProblem()?.let { it.message.replaceFirstChar { c -> c.uppercase() } } ?: adminErrorText(e)

    fun connect(hostIn: String, loginIn: ShareLogin? = null) {
        val host = hostIn.trim()
        if (host.isEmpty() || busy) return
        busy = true
        scope.launch {
            try {
                shares = client.shares(host, loginIn)
                login = loginIn
                error = null
                val p = pending
                if (p != null) { pending = null; stage = ShareStage.Folders(host, p.first, p.second) } else stage = ShareStage.Shares(host)
            } catch (e: Exception) {
                if (e is CancellationException) throw e
                when (e.shareProblem()?.problem) {
                    "auth_required" -> { error = null; stage = ShareStage.Login(host) }
                    "wrong_password" -> { error = "The user name or password is wrong."; stage = ShareStage.Login(host) }
                    "unreachable" -> error = "The Moonrise server can't reach $host. Check the name or address."
                    else -> error = problemText(e)
                }
            } finally { busy = false }
        }
    }

    LaunchedEffect(Unit) {
        found = runCatching { client.discoverFileServers() }.getOrDefault(emptyList())
        discovering = false
    }
    LaunchedEffect(stage) {
        val s = stage as? ShareStage.Folders ?: return@LaunchedEffect
        listing = null
        busy = true
        try {
            listing = client.browseShare(s.host, s.share, s.path, login)
            error = null
        } catch (e: Exception) {
            if (e is CancellationException) throw e
            val p = e.shareProblem()?.problem
            if (p in listOf("auth_required", "wrong_password", "access_denied")) {
                // Shares can be listed as a guest while the share itself wants an account.
                pending = s.share to s.path
                if (login != null) error = if (p == "access_denied") "This account may not open that share." else "The user name or password is wrong."
                stage = ShareStage.Login(s.host)
            } else error = problemText(e)
        } finally { busy = false }
    }

    fun use(s: ShareStage.Folders) {
        busy = true
        scope.launch {
            try {
                val t = client.testShare(s.host, s.share, s.path, login)
                if (!t.ok) error = "This folder is empty."
                else onPick(LibraryRoot(true, s.path, host = s.host, share = s.share, login = login, videoFiles = t.videoFiles))
            } catch (e: Exception) {
                if (e is CancellationException) throw e
                error = e.shareProblem()?.message ?: adminErrorText(e)
            } finally { busy = false }
        }
    }

    val s = stage
    val title = when (s) {
        ShareStage.Servers -> "Network Share"
        is ShareStage.Login -> s.host
        is ShareStage.Shares -> s.host
        is ShareStage.Folders -> if (s.path.isEmpty()) s.share else s.path.substringAfterLast('/')
    }
    FormPage(title, back) {
        when (s) {
            ShareStage.Servers -> {
                FormSection("On the Network", footer = "File servers the Moonrise server sees on your network. Pick yours, or type its name or address.") {
                    found.forEachIndexed { i, f ->
                        if (i > 0) FormDivider()
                        FormRow(onClick = { connect(f.host) }) {
                            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                Icon(Icons.Outlined.Dns, null, tint = Color.White, modifier = Modifier.size(24.dp))
                                Column(Modifier.weight(1f)) {
                                    Text(f.name, style = Type.body, color = Color.White)
                                    Text(f.host, style = Type.caption, color = Theme.secondaryText)
                                }
                                Chevron()
                            }
                        }
                    }
                    if (discovering) {
                        if (found.isNotEmpty()) FormDivider()
                        FormRow {
                            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                Spinner()
                                Text("Looking for file servers…", style = Type.body, color = Theme.secondaryText)
                            }
                        }
                    } else if (found.isEmpty()) FormRow { Text("None found on the network.", style = Type.body, color = Theme.secondaryText) }
                }
                FormSection("Or Type Its Address") {
                    FormField(address, { address = it }, "e.g. 192.168.1.20 or nas.local", keyboard = KeyboardType.Uri, ime = ImeAction.Go, onDone = { connect(address) })
                    FormDivider()
                    BusyRow("Connect", busy, enabled = address.isNotBlank()) { connect(address) }
                }
            }
            is ShareStage.Login -> {
                FormSection("Account on ${s.host}", footer = "This file server doesn't let guests in here. Enter an account on it: the Moonrise server keeps it, only to read your videos.") {
                    FormField(username, { username = it }, "User name")
                    FormDivider()
                    FormField(password, { password = it }, "Password", secure = true, keyboard = KeyboardType.Password, ime = ImeAction.Go, onDone = { if (username.isNotEmpty()) connect(s.host, ShareLogin(username, password)) })
                    FormDivider()
                    BusyRow("Connect", busy, enabled = username.isNotEmpty()) { connect(s.host, ShareLogin(username, password)) }
                }
                FormSection { FormRow(onClick = { stage = ShareStage.Servers; error = null }) { Text("Other File Server", style = Type.body, color = Color.White) } }
            }
            is ShareStage.Shares -> {
                FormSection("Shares", footer = "Shares on this file server. Pick the one with your videos.") {
                    shares.forEachIndexed { i, sh ->
                        if (i > 0) FormDivider()
                        FolderRow(sh.name, Icons.Outlined.Storage) { stage = ShareStage.Folders(s.host, sh.name, "") }
                    }
                }
                FormSection { FormRow(onClick = { stage = ShareStage.Servers; error = null }) { Text("Other File Server", style = Type.body, color = Color.White) } }
            }
            is ShareStage.Folders -> {
                val l = listing
                if (l != null) {
                    FormSection((listOf(s.host, s.share) + s.path.split('/').filter { it.isNotEmpty() }).joinToString(" › "), footer = AdminRules.videosText(l.videoFiles, l.dirs.size)) {
                        UseRow(busy) { use(s) }
                    }
                }
                FormSection("Folders") {
                    FolderRow(if (s.path.isEmpty()) "Shares" else "Up", Icons.Outlined.SubdirectoryArrowLeft) {
                        error = null
                        stage = if (s.path.isEmpty()) ShareStage.Shares(s.host)
                        else ShareStage.Folders(s.host, s.share, s.path.substringBeforeLast('/', ""))
                    }
                    l?.dirs?.forEach { d ->
                        FormDivider()
                        FolderRow(d.name) { stage = ShareStage.Folders(s.host, s.share, d.path) }
                    }
                    if (busy && l == null) { FormDivider(); SpinnerRow() }
                }
            }
        }
        error?.let { NoteSection(it, Theme.destructive) }
    }
}
