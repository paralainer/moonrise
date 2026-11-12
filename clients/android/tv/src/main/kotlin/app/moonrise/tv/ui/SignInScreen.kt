package app.moonrise.tv.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.Text
import app.moonrise.core.DeviceCode
import app.moonrise.core.DevicePoll
import app.moonrise.core.MoonriseApi
import app.moonrise.core.MoonriseHttpException
import app.moonrise.core.ServerInfo
import app.moonrise.tv.R
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import app.moonrise.shared.FoundServer
import app.moonrise.shared.Session
import app.moonrise.shared.qrBitmap
import app.moonrise.shared.discoverServers
import app.moonrise.shared.Log

/**
 * Sign-in, as on Apple TV (`SignInViews.swift`, tvOS): servers found on the network or an
 * address, then a code approved on another device or in a browser, or a user name and password.
 */
@Composable
fun SignInScreen(onSignedIn: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var address by remember { mutableStateOf(Session.lastAddress?.removePrefix("http://")?.trimEnd('/')?.removeSuffix(":8097") ?: "") }
    var info by remember { mutableStateOf<ServerInfo?>(null) }
    var username by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var discovered by remember { mutableStateOf<List<FoundServer>>(emptyList()) }
    var discovering by remember { mutableStateOf(true) }
    var codeFor by remember { mutableStateOf<MoonriseApi?>(null) }

    LaunchedEffect(Unit) {
        launch { delay(8000); discovering = false }
        discoverServers(context).collect { discovered = it }
    }
    // Check the server as the address settles.
    LaunchedEffect(address) {
        info = null
        if (address.isBlank()) return@LaunchedEffect
        delay(600)
        info = runCatching { Session.client(address)?.info() }.getOrNull()
    }

    fun signIn() {
        val client = Session.client(address) ?: run { error = "Invalid server address."; return }
        busy = true
        error = null
        scope.launch {
            try {
                client.login(username.trim(), password)
                Session.signedIn(client)
                onSignedIn()
            } catch (e: MoonriseHttpException) {
                error = if (e.code == 401) "Wrong user name or password." else e.message
            } catch (e: Exception) {
                Log.w("Moonrise", "sign-in failed", e)
                error = "Can't reach the server: ${e.message}"
            } finally {
                busy = false
            }
        }
    }

    val c = codeFor
    if (c != null) {
        DeviceCodeScreen(c, onCancel = { codeFor = null }, onApproved = {
            Session.signedIn(c)
            onSignedIn()
        })
        return
    }

    SetupScreen(title = "Welcome to Moonrise", message = "Connect to your Moonrise server to sign in. Sign in with a code: approve it on your iPhone (Settings → Approve a Device) or in any browser, with no typing passwords with the remote.") {
        if (discovering || discovered.isNotEmpty()) {
            FieldLabel(if (discovering && discovered.isEmpty()) "Looking for servers…" else "On Your Network")
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                discovered.forEach { srv ->
                    GlassButton(onClick = { address = srv.address }) {
                        Text((if (address == srv.address) "✓ " else "") + srv.name + if (srv.unclaimed) " · New" else "")
                    }
                }
            }
            Spacer(Modifier.height(14.dp))
        }
        TvTextField("Server", address, { address = it }, placeholder = "e.g. 192.168.1.10", keyboardType = KeyboardType.Uri)
        info?.let {
            Text("✓ ${it.name} · ${it.version}", color = Color(0xFF4CD964), fontSize = 14.sp, modifier = Modifier.padding(start = 12.dp, top = 6.dp))
        }
        Spacer(Modifier.height(16.dp))
        if (info?.unclaimed == true) {
            Text(
                "This server is new and isn't set up yet. Set it up from the Moonrise app on an iPhone, iPad or Mac, or on its page in a browser.",
                color = Theme.secondaryText, fontSize = 14.sp,
            )
        } else {
            if (info?.deviceCode != false) {
                GlassButton(onClick = { Session.client(address)?.let { codeFor = it } }, enabled = address.isNotBlank(), modifier = Modifier.fillMaxWidth()) {
                    Text("Sign In with a Code", fontWeight = FontWeight.Bold, modifier = Modifier.fillMaxWidth(), textAlign = androidx.compose.ui.text.style.TextAlign.Center)
                }
                OrDivider()
            }
            TvTextField("Username", username, { username = it })
            Spacer(Modifier.height(10.dp))
            TvTextField("Password", password, { password = it }, secure = true, onDone = { if (!busy) signIn() })
            Spacer(Modifier.height(16.dp))
            GlassButton(onClick = { signIn() }, enabled = address.isNotBlank() && username.isNotBlank() && !busy, modifier = Modifier.fillMaxWidth()) {
                Text(if (busy) "Signing In…" else "Sign In", fontWeight = FontWeight.Bold, modifier = Modifier.fillMaxWidth(), textAlign = androidx.compose.ui.text.style.TextAlign.Center)
            }
        }
        error?.let { Text(it, color = Color(0xFFFF6B6B), fontSize = 14.sp, modifier = Modifier.padding(top = 12.dp)) }
    }
}

/** The tvOS setup screen (`TVSetupScreen`): the mark, title and message on the left, the form on the right. */
@Composable
fun SetupScreen(title: String, message: String, content: @Composable () -> Unit) {
    Row(
        Modifier
            .fillMaxSize()
            .background(Theme.background)
            .background(Brush.radialGradient(listOf(Color(1f, 0.6f, 0.45f, 0.12f), Color.Transparent), center = Offset.Zero, radius = 1300f))
            .padding(horizontal = 60.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(60.dp),
    ) {
        Column(Modifier.width(310.dp), verticalArrangement = Arrangement.spacedBy(15.dp)) {
            Image(painterResource(R.drawable.moonrise_mark), contentDescription = null, modifier = Modifier.size(120.dp))
            Text(title, fontSize = 32.sp, fontWeight = FontWeight.Bold, color = Color.White, lineHeight = 36.sp)
            Text(message, fontSize = 14.sp, color = Theme.secondaryText, lineHeight = 19.sp)
        }
        Column(
            Modifier
                .width(410.dp)
                .verticalScroll(rememberScrollState())
                .padding(vertical = 30.dp),
        ) { content() }
    }
}

@Composable
private fun FieldLabel(text: String) {
    Text(text, fontSize = 13.sp, color = Theme.secondaryText, modifier = Modifier.padding(start = 12.dp, bottom = 6.dp))
}

@Composable
private fun OrDivider() {
    Row(Modifier.fillMaxWidth().padding(vertical = 14.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.weight(1f).height(1.dp).background(Theme.surfaceStrong))
        Text("or", fontSize = 13.sp, color = Theme.secondaryText, modifier = Modifier.padding(horizontal = 12.dp))
        Box(Modifier.weight(1f).height(1.dp).background(Theme.surfaceStrong))
    }
}

/** Shows a device code and a QR code of the page that approves it, and waits for approval (`DeviceCodeView`). */
@Composable
private fun DeviceCodeScreen(client: MoonriseApi, onCancel: () -> Unit, onApproved: () -> Unit) {
    var code by remember { mutableStateOf<DeviceCode?>(null) }
    var expired by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var attempt by remember { mutableStateOf(0) }
    val cancelFocus = remember { FocusRequester() }

    LaunchedEffect(attempt) {
        try {
            client.info()
            val c = client.startDeviceCode()
            code = c
            Log.i("Moonrise") { "device code started" }
            var interval = maxOf(c.interval, 2) * 1000L
            val deadline = System.currentTimeMillis() + c.expiresIn * 1000L
            while (System.currentTimeMillis() < deadline) {
                delay(interval)
                when (val r = runCatching { client.pollDeviceCode(c.deviceCode) }.getOrDefault(DevicePoll.Pending)) {
                    is DevicePoll.Approved -> { onApproved(); return@LaunchedEffect }
                    DevicePoll.SlowDown -> interval += 2000
                    DevicePoll.Expired -> break
                    DevicePoll.Pending -> {}
                }
            }
            expired = true
        } catch (e: Exception) {
            error = "Can't get a code from the server: ${e.message}"
        }
    }
    LaunchedEffect(Unit) { runCatching { cancelFocus.requestFocus() } }

    Row(
        Modifier.fillMaxSize().background(Theme.background).padding(50.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(45.dp, Alignment.CenterHorizontally),
    ) {
        val c = code
        if (c != null && !expired && error == null) {
            val qr = remember(c.verifyUrlComplete) { qrBitmap(c.verifyUrlComplete, 360).asImageBitmap() }
            Box(Modifier.size(180.dp).background(Color.White, RoundedCornerShape(8.dp)).padding(7.dp)) {
                Image(qr, contentDescription = "QR code of the sign-in page", modifier = Modifier.fillMaxSize())
            }
        }
        Column(Modifier.width(450.dp), verticalArrangement = Arrangement.spacedBy(15.dp)) {
            Text("Sign In with a Code", fontSize = 28.sp, fontWeight = FontWeight.Bold)
            when {
                error != null -> {
                    Text(error!!, color = Color(0xFFFF6B6B))
                    GlassButton(onClick = { error = null; code = null; attempt++ }) { Text("Try Again") }
                }
                expired -> {
                    Text("The code expired.", color = Theme.secondaryText)
                    GlassButton(onClick = { expired = false; code = null; attempt++ }) { Text("Get a New Code") }
                }
                c != null -> {
                    Text(
                        c.userCode,
                        fontSize = 48.sp, fontWeight = FontWeight.Bold, fontFamily = FontFamily.Monospace, letterSpacing = 6.sp,
                        modifier = Modifier.background(Color.White.copy(alpha = 0.1f), RoundedCornerShape(11.dp)).padding(horizontal = 18.dp, vertical = 9.dp),
                    )
                    Text(
                        "On your iPhone, open Moonrise → Settings → Approve a Device and enter this code. Or scan the QR code, or open ${c.verifyUrl} in any browser.",
                        color = Theme.secondaryText, fontSize = 15.sp, lineHeight = 20.sp,
                    )
                    Text("Waiting for approval…", color = Theme.secondaryText, fontSize = 14.sp)
                }
                else -> Text("Getting a code…", color = Theme.secondaryText)
            }
            GlassButton(onClick = onCancel, modifier = Modifier.focusRequester(cancelFocus)) { Text("Cancel") }
        }
    }
}
