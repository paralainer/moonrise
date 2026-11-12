package app.moonrise.phone.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.outlined.AutoFixHigh
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material.icons.outlined.Drafts
import androidx.compose.material.icons.outlined.QrCodeScanner
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import app.moonrise.core.ClientRules
import app.moonrise.core.DeviceCode
import app.moonrise.core.DevicePoll
import app.moonrise.core.MoonriseApi
import app.moonrise.core.MoonriseHttpException
import app.moonrise.core.ServerInfo
import app.moonrise.core.managedLibraries
import app.moonrise.phone.R
import app.moonrise.shared.FoundServer
import app.moonrise.shared.Log
import app.moonrise.shared.Session
import app.moonrise.shared.discoverServers
import app.moonrise.shared.qrBitmap
import app.moonrise.shared.sweepServers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private enum class Step { Welcome, SignIn, Code, Setup, ResumeSetup, Invite }

/** First run (`OnboardingView`): the welcome, then signing in to a server or setting a new one up. */
@Composable
fun OnboardingScreen(onSignedIn: () -> Unit) {
    var step by remember { mutableStateOf(Step.Welcome) }
    var client by remember { mutableStateOf<MoonriseApi?>(null) }
    var inviteInfo by remember { mutableStateOf<ServerInfo?>(null) }
    // Kept while a code or the setup is shown, so Back from them finds the form as it was.
    val address = remember { mutableStateOf(Session.lastAddress?.removePrefix("http://")?.trimEnd('/')?.removeSuffix(":8097") ?: "") }
    BackHandler(step != Step.Welcome) { step = if (step == Step.SignIn) Step.Welcome else Step.SignIn }
    fun done(c: MoonriseApi) { Session.signedIn(c); onSignedIn() }
    when (step) {
        Step.Welcome -> Welcome { step = Step.SignIn }
        Step.SignIn -> SignIn(address, back = { step = Step.Welcome }, signedIn = ::done,
            resumeSetup = { client = it; step = Step.ResumeSetup },
            code = { client = it; step = Step.Code }, setup = { client = it; step = Step.Setup },
            invite = { c, info -> client = c; inviteInfo = info; step = Step.Invite })
        Step.Code -> DeviceCodePage(client!!, cancel = { step = Step.SignIn }, approved = { done(client!!) })
        Step.Setup -> SetupFlowScreen(client!!, address.value.ifBlank { client!!.baseUrl.host }.let { if (':' in it) it else "$it:8097" }, back = { step = Step.SignIn }, finished = { done(client!!) })
        Step.ResumeSetup -> SetupFlowScreen(client!!, client!!.baseUrl.host, back = { step = Step.SignIn }, finished = { done(client!!) }, startAtLibrary = true)
        Step.Invite -> InvitePage(client!!, inviteInfo!!, back = { step = Step.SignIn }, joined = { done(client!!) })
    }
}

/** The welcome over the theatre artwork, and Connect to Your Server. */
@Composable
private fun Welcome(connect: () -> Unit) {
    Column(Modifier.fillMaxSize().background(Theme.background).verticalScroll(rememberScrollState()), horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(22.dp)) {
        Box(Modifier.fillMaxWidth().height(420.dp), contentAlignment = Alignment.BottomCenter) {
            Image(painterResource(R.drawable.welcome_theater), null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
            Box(Modifier.fillMaxSize().background(Brush.verticalGradient(0.5f to Color.Transparent, 1f to Theme.background)))
            Column(Modifier.padding(horizontal = 30.dp).padding(bottom = 20.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Welcome to Moonrise", style = Type.largeTitle, color = Color.White, textAlign = TextAlign.Center)
                Text("Connect to your Moonrise server to sign in, or to set up a new one.", style = Type.subheadline, color = Theme.secondaryText, textAlign = TextAlign.Center)
            }
        }
        WhiteCapsule("Connect to Your Server", icon = Icons.Outlined.Dns, onClick = connect)
        Spacer(Modifier.height(40.dp))
    }
}

/** A page with a title and Back, its sections grouped as the iPhone's forms. */
@Composable
internal fun FormPage(title: String, back: () -> Unit, backLabel: String? = null, action: (@Composable () -> Unit)? = null, content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.fillMaxSize().background(Color.Black).statusBarsPadding().navigationBarsPadding().imePadding()) {
        Box(Modifier.fillMaxWidth().padding(horizontal = Theme.hPadding, vertical = 8.dp), contentAlignment = Alignment.Center) {
            // A sheet's Cancel or Done (`backLabel`), else Back.
            if (!backLabel.isNullOrEmpty()) GlassCapsule(Modifier.align(Alignment.CenterStart), onClick = back) { Text(backLabel, style = Type.headline, color = Color.White) }
            else if (backLabel == null) GlassCircle(Icons.AutoMirrored.Filled.ArrowBack, "Back", Modifier.align(Alignment.CenterStart), onClick = back)
            Text(title, style = Type.headline, color = Color.White, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis, modifier = Modifier.padding(horizontal = 80.dp))
            action?.let { Box(Modifier.align(Alignment.CenterEnd)) { it() } }
        }
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = Theme.hPadding).padding(top = 8.dp, bottom = 40.dp),
            verticalArrangement = Arrangement.spacedBy(22.dp)) {
            content()
        }
    }
}

/** A grouped section: a small header, rows on a rounded card, a footer. */
@Composable
fun FormSection(header: String? = null, footer: String? = null, content: @Composable ColumnScope.() -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        if (header != null) Text(header, style = Type.headline, color = Theme.secondaryText, modifier = Modifier.padding(start = 16.dp, bottom = 4.dp))
        Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(26.dp)).background(Color(0xFF1F2023)).padding(vertical = 4.dp), content = content)
        if (footer != null) Text(footer, style = Type.footnote, color = Theme.secondaryText, modifier = Modifier.padding(horizontal = 16.dp))
    }
}

@Composable
fun FormRow(onClick: (() -> Unit)? = null, enabled: Boolean = true, content: @Composable () -> Unit) {
    Box(Modifier.fillMaxWidth().then(if (onClick != null) Modifier.clickable(enabled = enabled, onClick = onClick) else Modifier).padding(horizontal = 16.dp, vertical = 12.dp)) { content() }
}

@Composable
fun FormDivider() = HorizontalDivider(Modifier.padding(start = 16.dp), color = Color.White.copy(alpha = 0.12f))

@Composable
internal fun FormField(value: String, onChange: (String) -> Unit, placeholder: String, secure: Boolean = false, keyboard: KeyboardType = KeyboardType.Text,
                      ime: ImeAction = ImeAction.Next, onDone: () -> Unit = {}) {
    TextField(value, onChange, placeholder = { Text(placeholder, color = Theme.secondaryText) }, singleLine = true, modifier = Modifier.fillMaxWidth(),
        visualTransformation = if (secure) PasswordVisualTransformation() else androidx.compose.ui.text.input.VisualTransformation.None,
        keyboardOptions = KeyboardOptions(keyboardType = keyboard, imeAction = ime, autoCorrectEnabled = false), keyboardActions = KeyboardActions(onDone = { onDone() }, onGo = { onDone() }),
        colors = TextFieldDefaults.colors(focusedContainerColor = Color.Transparent, unfocusedContainerColor = Color.Transparent,
            focusedIndicatorColor = Color.Transparent, unfocusedIndicatorColor = Color.Transparent, cursorColor = Color.White))
}

@Composable
private fun RowLabel(text: String, icon: androidx.compose.ui.graphics.vector.ImageVector, enabled: Boolean = true, trailing: @Composable () -> Unit = {}) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Icon(icon, null, tint = if (enabled) Color.White else Theme.secondaryText, modifier = Modifier.size(24.dp))
        Text(text, style = Type.body, color = if (enabled) Color.White else Theme.secondaryText, modifier = Modifier.weight(1f))
        trailing()
    }
}

/**
 * Sign In (`SignInView`, iPhone): the servers found on the network (Bonjour and a sweep of the
 * local network), the address (checked as it's typed), then Sign In with a Code, or a user name
 * and password; a new server offers Set Up This Server instead.
 */
@Composable
private fun SignIn(addressState: androidx.compose.runtime.MutableState<String>, back: () -> Unit, signedIn: (MoonriseApi) -> Unit, resumeSetup: (MoonriseApi) -> Unit, code: (MoonriseApi) -> Unit, setup: (MoonriseApi) -> Unit, invite: (MoonriseApi, ServerInfo) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var address by addressState
    var info by remember { mutableStateOf<ServerInfo?>(null) }
    var username by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var bonjour by remember { mutableStateOf<List<FoundServer>>(emptyList()) }
    var swept by remember { mutableStateOf<List<FoundServer>>(emptyList()) }
    var discovering by remember { mutableStateOf(true) }
    val discovered = (bonjour + swept.filter { s -> bonjour.none { it.name == s.name } }).distinctBy { it.address }
    val keyboard = LocalSoftwareKeyboardController.current

    LaunchedEffect(Unit) {
        launch { swept = sweepServers(); delay(3000); discovering = false }
        discoverServers(context).collect { bonjour = it }
    }
    LaunchedEffect(Unit) { delay(3500); if (address.isBlank() && discovered.size == 1) address = discovered[0].address }
    LaunchedEffect(address) {
        info = null
        if (address.isBlank()) return@LaunchedEffect
        delay(600)
        val c = Session.client(address) ?: return@LaunchedEffect
        try { info = c.info(); error = null } catch (e: Exception) { if (ClientRules.isLoadFailure(e)) error = "Couldn't reach the server: ${e.message}" }
    }

    /** Enter in the Server field (`check()`, iPhone): asks the server now, without the wait after typing. */
    fun checkAddress() {
        val client = Session.client(address) ?: run { error = "Invalid server address."; return }
        scope.launch {
            try { info = client.info(); error = null }
            catch (e: Exception) { info = null; if (ClientRules.isLoadFailure(e)) error = "Couldn't reach the server: ${e.message}" }
        }
    }

    fun signIn() {
        val client = Session.client(address) ?: run { error = "Invalid server address."; return }
        busy = true
        error = null
        scope.launch {
            try {
                val auth = client.login(username.trim(), password)
                // An admin of a server without libraries (a setup left before its library step) continues the setup there.
                if (auth.user.isAdmin == true && runCatching { client.managedLibraries().items.isEmpty() }.getOrDefault(false)) resumeSetup(client)
                else signedIn(client)
            } catch (e: MoonriseHttpException) {
                error = when (e.code) { 401 -> "Wrong user name or password."; 429 -> "Too many attempts. Try again in a few minutes."; else -> e.message }
            } catch (e: Exception) {
                if (!ClientRules.isLoadFailure(e)) throw e
                Log.w("Moonrise", "sign-in failed", e)
                error = "Couldn't reach the server: ${e.message}"
            } finally {
                busy = false
            }
        }
    }

    FormPage("Sign In", back) {
        if (discovering || discovered.isNotEmpty()) {
            FormSection("On Your Network") {
                discovered.forEachIndexed { i, srv ->
                    if (i > 0) FormDivider()
                    FormRow(onClick = { address = srv.address }) {
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            Icon(Icons.Outlined.Dns, null, tint = Color.White, modifier = Modifier.size(24.dp))
                            Column(Modifier.weight(1f)) {
                                Text(srv.name, style = Type.body, color = Color.White)
                                Text(if (srv.unclaimed) "${srv.address} · New, not set up" else srv.address, style = Type.caption, color = Theme.secondaryText)
                            }
                            if (address == srv.address) Icon(Icons.Filled.Check, null, tint = Theme.green)
                        }
                    }
                }
                if (discovering) {
                    if (discovered.isNotEmpty()) FormDivider()
                    FormRow {
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            CircularProgressIndicator(color = Theme.secondaryText, strokeWidth = 2.dp, modifier = Modifier.size(18.dp))
                            Text("Looking for servers…", style = Type.body, color = Theme.secondaryText)
                        }
                    }
                }
            }
        }
        FormSection("Server") {
            // Enter checks the address, as on iPhone, and focus stays here. A "next" action would move focus
            // to Sign In with a Code's row, and the key's release would press it.
            FormField(address, { address = it.trim() }, "e.g. 192.168.1.10", keyboard = KeyboardType.Uri, ime = ImeAction.Done,
                onDone = { checkAddress(); keyboard?.hide() })
            info?.let {
                FormDivider()
                FormRow {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Icon(Icons.Filled.CheckCircle, null, tint = Theme.green, modifier = Modifier.size(22.dp))
                        Text("${it.name} · ${it.version}", style = Type.body, color = Theme.green)
                    }
                }
            }
        }
        if (info?.unclaimed == true) {
            FormSection("New Server", footer = "This server isn't set up yet. You'll need its setup code (in the server's log, or on its page in a browser), then you create its admin account and add your videos.") {
                FormRow(onClick = { Session.client(address)?.let(setup) }) { RowLabel("Set Up This Server", Icons.Outlined.AutoFixHigh) }
            }
        } else {
            val codes = info?.deviceCode != false
            if (codes) {
                FormSection(footer = "Shows a code to approve on another device you're signed in on, or in any browser — no password needed.") {
                    FormRow(onClick = { Session.client(address)?.let(code) }, enabled = address.isNotBlank()) { RowLabel("Sign In with a Code", Icons.Outlined.QrCodeScanner, enabled = address.isNotBlank()) }
                }
            }
            FormSection(if (codes) "Or Sign In with Password" else "Account") {
                FormField(username, { username = it }, "Username")
                FormDivider()
                FormField(password, { password = it }, "Password", secure = true, keyboard = KeyboardType.Password, ime = ImeAction.Go, onDone = { if (!busy && username.isNotBlank()) signIn() })
                FormDivider()
                val can = address.isNotBlank() && username.isNotBlank() && !busy
                FormRow(onClick = { signIn() }, enabled = can) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text("Sign In", style = Type.body, color = if (can) Color.White else Theme.secondaryText, modifier = Modifier.weight(1f))
                        if (busy) CircularProgressIndicator(color = Theme.secondaryText, strokeWidth = 2.dp, modifier = Modifier.size(18.dp))
                    }
                }
            }
        }
        info?.takeIf { it.invites && !it.unclaimed }?.let { server ->
            FormSection(footer = "New here? With a code from an admin of this server, pick your own user name and password.") {
                FormRow(onClick = { Session.client(address)?.let { invite(it, server) } }) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Icon(Icons.Outlined.Drafts, null, tint = Color.White, modifier = Modifier.size(24.dp))
                        Text("Have an Invitation?", style = Type.body, color = Color.White, modifier = Modifier.weight(1f))
                        Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, null, tint = Theme.secondaryText, modifier = Modifier.size(20.dp))
                    }
                }
            }
        }
        error?.let { e -> FormSection { FormRow { Text(e, style = Type.body, color = Theme.destructive) } } }
    }
}

/** A new account with an invitation code (`InviteSignInView`): the code, a user name and password, Join. */
@Composable
private fun InvitePage(client: MoonriseApi, info: ServerInfo, back: () -> Unit, joined: () -> Unit) {
    var code by remember { mutableStateOf("") }
    var username by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val problem = ClientRules.invitationProblem(code, username, password)
    fun join() {
        busy = true
        error = null
        scope.launch {
            try {
                client.redeemInvite(ClientRules.formatDeviceCode(code), username.trim(), password)
                joined()
            } catch (e: MoonriseHttpException) {
                error = if (e.code == 429) "Too many wrong codes. Try again in a few minutes." else e.body.ifEmpty { e.message }
            } catch (e: Exception) {
                if (!ClientRules.isLoadFailure(e)) throw e
                error = "Couldn't reach the server: ${e.message}"
            } finally {
                busy = false
            }
        }
    }
    FormPage("Have an Invitation", back) {
        FormSection("Invitation Code", footer = "From an admin of ${info.name}.") {
            FormField(code, { code = it }, "XXXX-XXXX", ime = ImeAction.Next)
        }
        FormSection("Your Account", footer = if (!problem.isNullOrEmpty() && password.isNotEmpty()) problem else "Use these to sign in on your other devices too.") {
            FormField(username, { username = it }, "User name", ime = ImeAction.Next)
            FormDivider()
            FormField(password, { password = it }, "Password (at least 8 characters)", secure = true, keyboard = KeyboardType.Password, ime = ImeAction.Done, onDone = { if (problem == null && !busy) join() })
        }
        FormSection {
            val can = problem == null && !busy
            FormRow(onClick = { join() }, enabled = can) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text("Join ${info.name}", style = Type.body, color = if (can) Color.White else Theme.secondaryText, modifier = Modifier.weight(1f))
                    if (busy) CircularProgressIndicator(color = Theme.secondaryText, strokeWidth = 2.dp, modifier = Modifier.size(18.dp))
                }
            }
        }
        error?.let { e -> FormSection { FormRow { Text(e, style = Type.body, color = Theme.destructive) } } }
    }
}

/** Settings → Approve a Device (`ApproveDeviceView`): sign another device showing a code in with this device's sign-in. */
@Composable
fun ApproveDevicePage(back: () -> Unit) {
    BackHandler(onBack = back)
    var code by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var result by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val formatted = ClientRules.formatDeviceCode(code)
    val can = formatted.length >= 9 && !busy
    fun approve() {
        val api = Session.api ?: return
        busy = true
        result = null
        error = null
        scope.launch {
            try {
                val name = api.approveDevice(formatted)
                result = "Approved" + (name?.let { " · $it" } ?: "") + ". It is signing in now."
                code = ""
            } catch (e: MoonriseHttpException) {
                error = when {
                    e.code == 404 -> "No device is waiting with that code (it may have expired)."
                    e.code == 400 || e.code == 410 -> e.body.ifEmpty { "That code can't be approved." }
                    else -> e.message
                }
            } catch (e: Exception) {
                if (!ClientRules.isLoadFailure(e)) throw e
                error = "Couldn't reach the server: ${e.message}"
            } finally {
                busy = false
            }
        }
    }
    FormPage("Approve a Device", back) {
        FormSection(footer = "Enter the code the other device shows (Sign In with a Code). It signs in as you.") {
            FormField(code, { code = it }, "ABCD-1234", ime = ImeAction.Done, onDone = { if (can) approve() })
            FormDivider()
            FormRow(onClick = { approve() }, enabled = can) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text("Approve", style = Type.body, color = if (can) Color.White else Theme.secondaryText, modifier = Modifier.weight(1f))
                    if (busy) CircularProgressIndicator(color = Theme.secondaryText, strokeWidth = 2.dp, modifier = Modifier.size(18.dp))
                }
            }
        }
        result?.let { r ->
            FormSection { FormRow {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    Icon(Icons.Filled.CheckCircle, null, tint = Theme.green, modifier = Modifier.size(22.dp))
                    Text(r, style = Type.body, color = Theme.green)
                }
            } }
        }
        error?.let { e -> FormSection { FormRow { Text(e, style = Type.body, color = Theme.destructive) } } }
    }
}

/** Shows a device code (and a QR code of the page that approves it) and waits for approval (`DeviceCodeView`). */
@Composable
private fun DeviceCodePage(client: MoonriseApi, cancel: () -> Unit, approved: () -> Unit) {
    var code by remember { mutableStateOf<DeviceCode?>(null) }
    var expired by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var attempt by remember { mutableIntStateOf(0) }
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
                when (runCatching { client.pollDeviceCode(c.deviceCode) }.getOrDefault(DevicePoll.Pending)) {
                    is DevicePoll.Approved -> { approved(); return@LaunchedEffect }
                    DevicePoll.SlowDown -> interval += 2000
                    DevicePoll.Expired -> break
                    DevicePoll.Pending -> {}
                }
            }
            expired = true
        } catch (e: Exception) {
            if (!ClientRules.isLoadFailure(e)) throw e
            error = "Can't get a code from the server: ${e.message}"
        }
    }
    Column(Modifier.fillMaxSize().background(Theme.background).statusBarsPadding().padding(50.dp).verticalScroll(rememberScrollState()),
        horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(26.dp, Alignment.CenterVertically)) {
        Text("Sign In with a Code", style = Type.title, color = Color.White, textAlign = TextAlign.Center)
        val c = code
        when {
            error != null -> {
                Text(error!!, color = Theme.destructive, textAlign = TextAlign.Center)
                GlassCapsule(onClick = { error = null; code = null; attempt++ }) { Text("Try Again", color = Color.White) }
            }
            expired -> {
                Text("The code expired.", color = Theme.secondaryText)
                GlassCapsule(onClick = { expired = false; code = null; attempt++ }) { Text("Get a New Code", color = Color.White) }
            }
            c != null -> {
                BoxWithConstraints(contentAlignment = Alignment.Center) {
                    val size = with(LocalDensity.current) { ClientRules.deviceCodeFontSize(c.userCode.length, (maxWidth - 72.dp).value).dp.toSp() }
                    Text(c.userCode, fontSize = size, fontWeight = FontWeight.Bold, fontFamily = FontFamily.Monospace, letterSpacing = ClientRules.DEVICE_CODE_KERNING.em, color = Color.White,
                        maxLines = 1, softWrap = false,
                        modifier = Modifier.background(Color.White.copy(alpha = 0.1f), RoundedCornerShape(22.dp)).padding(horizontal = 36.dp, vertical = 18.dp))
                }
                Text("On another device signed in to Moonrise, open Settings → Approve a Device and enter this code. Or scan the QR code, or open ${c.verifyUrl} in any browser.",
                    style = Type.body, color = Theme.secondaryText, textAlign = TextAlign.Center, modifier = Modifier.widthIn(max = 700.dp))
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    CircularProgressIndicator(color = Theme.secondaryText, strokeWidth = 2.dp, modifier = Modifier.size(18.dp))
                    Text("Waiting for approval…", color = Theme.secondaryText)
                }
                val qr = remember(c.verifyUrlComplete) { qrBitmap(c.verifyUrlComplete, 360).asImageBitmap() }
                Image(qr, "QR code of the sign-in page", modifier = Modifier.size(180.dp).background(Color.White, RoundedCornerShape(16.dp)).padding(14.dp))
            }
            else -> CircularProgressIndicator(color = Color.White)
        }
        GlassCapsule(onClick = cancel) { Text("Cancel", style = Type.headline, color = Color.White) }
    }
}
