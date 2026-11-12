// SPDX-License-Identifier: MPL-2.0
package app.moonrise.phone.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Cancel
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.RemoveCircleOutline
import androidx.compose.material.icons.outlined.RadioButtonUnchecked
import androidx.compose.material.icons.outlined.Warning
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
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
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import app.moonrise.core.AdminRules
import app.moonrise.core.AdminService
import app.moonrise.core.LLMModel
import app.moonrise.core.LLMOverview
import app.moonrise.core.LLMPreset
import app.moonrise.core.LLMTestResult
import app.moonrise.core.adminErrorText
import app.moonrise.core.chatGPTSignOut
import app.moonrise.core.checkOpenSubtitles
import app.moonrise.core.llmModels
import app.moonrise.core.llmOverview
import app.moonrise.core.llmTest
import app.moonrise.core.services
import app.moonrise.core.settingsDocument
import app.moonrise.core.updateSettings
import app.moonrise.shared.Session
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import java.util.Locale

/** The icon for a service's state (`ServiceSummary.icon`). */
private fun stateIcon(state: String) = when (state) {
    "configured", "builtin" -> Icons.Outlined.CheckCircle
    "off" -> Icons.Outlined.RemoveCircleOutline
    else -> Icons.Outlined.RadioButtonUnchecked
}

/** A service's state in a line, then what Moonrise does with it and without it (`ServiceSummary`). */
@Composable
internal fun ServiceSummary(s: AdminService) {
    FormRow {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Icon(stateIcon(s.state), null, tint = Color.White, modifier = Modifier.size(22.dp))
                Text(s.summary, style = Type.body, color = Color.White, modifier = Modifier.weight(1f))
            }
            Text(buildAnnotatedString { withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { append("With it: ") }; append(s.with) }, style = Type.callout, color = Theme.secondaryText)
            Text(buildAnnotatedString { withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { append("Without it: ") }; append(s.without) }, style = Type.callout, color = Theme.secondaryText)
        }
    }
}

/** Keys and accounts in a service's fields (Manage Server's Services and the setup's Extras). */
internal class ServiceFields {
    var tmdbKey by mutableStateOf("")
    var osUser by mutableStateOf("")
    var osPass by mutableStateOf("")
    var llmKey by mutableStateOf("")
    var omdbKey by mutableStateOf("")

    val changes: Map<String, JsonElement> get() {
        val c = mutableMapOf<String, JsonElement>()
        if (tmdbKey.isNotBlank()) c["tmdb.api_key"] = JsonPrimitive(tmdbKey.trim())
        if (osUser.isNotBlank() && osPass.isNotEmpty()) { c["opensubtitles.username"] = JsonPrimitive(osUser.trim()); c["opensubtitles.password"] = JsonPrimitive(osPass) }
        if (llmKey.isNotBlank()) c["llm.api_key"] = JsonPrimitive(llmKey.trim())
        if (omdbKey.isNotBlank()) c["omdb.api_key"] = JsonPrimitive(omdbKey.trim())
        return c
    }

    fun clear() { tmdbKey = ""; osUser = ""; osPass = ""; llmKey = ""; omdbKey = "" }

    /** `hint`: "(optional)" in the setup's Extras. */
    @Composable
    fun Rows(id: String, optional: Boolean = false) {
        when (id) {
            "tmdb" -> { FormDivider(); FormField(tmdbKey, { tmdbKey = it }, "Your TMDB API key" + if (optional) " (optional)" else "", secure = true, keyboard = KeyboardType.Password) }
            "opensubtitles" -> {
                FormDivider()
                FormField(osUser, { osUser = it }, "OpenSubtitles user name" + if (optional) " (optional)" else "")
                FormDivider()
                FormField(osPass, { osPass = it }, "OpenSubtitles password", secure = true, keyboard = KeyboardType.Password)
            }
            "llm" -> { FormDivider(); FormField(llmKey, { llmKey = it }, "Anthropic API key" + if (optional) " (optional)" else "", secure = true, keyboard = KeyboardType.Password) }
            "ratings" -> if (!optional) { FormDivider(); FormField(omdbKey, { omdbKey = it }, "Your OMDb API key (free at omdbapi.com)", secure = true, keyboard = KeyboardType.Password) }
        }
    }
}

/** Services (`ServicesAdminView`): each optional service, and your own keys or accounts for them. */
@Composable
fun ServicesPage() {
    val api = Session.api ?: return
    val pages = LocalPages.current
    val scope = rememberCoroutineScope()
    var services by remember { mutableStateOf<List<AdminService>>(emptyList()) }
    var loading by remember { mutableStateOf(true) }
    val fields = remember { ServiceFields() }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var note by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) {
        adminCall({ error = it }) { services = api.services(); error = null }
        loading = false
    }
    FormPage("Services", pages::pop) {
        if (loading) SpinnerSection()
        services.forEach { s ->
            FormSection(s.name, footer = s.attribution) {
                ServiceSummary(s)
                AdminRules.quotaText(s)?.let { FormDivider(); InfoRow("Downloads Today", it) }
                if (s.id == "llm" && s.provider != null) {
                    FormDivider()
                    ManageLink("Provider", stateIcon(s.state), detail = s.providerName.orEmpty()) { pages.push { AIMatchingPage() } }
                } else if (s.state != "off") fields.Rows(s.id)
            }
        }
        if (services.isNotEmpty()) {
            FormSection(footer = "Keys and passwords are kept on the server; they're never shown again. An OpenSubtitles account is checked before it's saved.") {
                BusyRow("Save", busy, enabled = fields.changes.isNotEmpty()) {
                    val ch = fields.changes
                    busy = true
                    note = null
                    scope.launch {
                        adminCall({ error = it }) {
                            val u = (ch["opensubtitles.username"] as? JsonPrimitive)?.content
                            val p = (ch["opensubtitles.password"] as? JsonPrimitive)?.content
                            val why = if (u != null && p != null) api.checkOpenSubtitles(u, p) else null
                            if (why != null) { error = why; return@adminCall }
                            api.updateSettings(ch)
                            fields.clear()
                            error = null
                            note = "Saved."
                            services = runCatching { api.services() }.getOrDefault(services)
                        }
                        busy = false
                    }
                }
            }
        }
        note?.let { NoteSection(it, Theme.green) }
        error?.let { NoteSection(it, Theme.destructive) }
    }
}

/** A field with its name above it (a filled-in field alone doesn't say what it is; `LabeledField`). */
@Composable
private fun LabeledField(label: String, value: String, set: (String) -> Unit, placeholder: String, secure: Boolean = false, keyboard: KeyboardType = KeyboardType.Text) {
    FormRow {
        Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Text(label, style = Type.caption, color = Theme.secondaryText)
            androidx.compose.foundation.text.BasicTextField(value, set, singleLine = true, textStyle = Type.body.copy(color = Color.White),
                cursorBrush = androidx.compose.ui.graphics.SolidColor(Color.White),
                visualTransformation = if (secure) androidx.compose.ui.text.input.PasswordVisualTransformation() else androidx.compose.ui.text.input.VisualTransformation.None,
                keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(keyboardType = keyboard, autoCorrectEnabled = false),
                decorationBox = { inner ->
                    androidx.compose.foundation.layout.Box {
                        if (value.isEmpty()) Text(placeholder, style = Type.body, color = Theme.secondaryText)
                        inner()
                    }
                })
        }
    }
}

/** AI Matching (`AIMatchingAdminView`): which language model the server asks about unclear file names. */
@Composable
fun AIMatchingPage() {
    val api = Session.api ?: return
    val pages = LocalPages.current
    val scope = rememberCoroutineScope()
    var overview by remember { mutableStateOf<LLMOverview?>(null) }
    var loading by remember { mutableStateOf(true) }
    var provider by remember { mutableStateOf("") }
    var baseUrl by remember { mutableStateOf("") }
    var model by remember { mutableStateOf("") }
    var apiKey by remember { mutableStateOf("") }
    var output by remember { mutableStateOf("auto") }
    var models by remember { mutableStateOf<List<LLMModel>>(emptyList()) }
    var testing by remember { mutableStateOf(false) }
    var test by remember { mutableStateOf<LLMTestResult?>(null) }
    var testError by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var accountNote by remember { mutableStateOf<Pair<String, Boolean>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var note by remember { mutableStateOf<String?>(null) }
    var lockedBy by remember { mutableStateOf<String?>(null) }

    val preset: LLMPreset? = overview?.presets?.firstOrNull { it.id == provider }
    val isCurrent = provider == overview?.current?.provider
    val chatGPT = preset?.key == "sign_in"
    data class Start(val base: String, val model: String, val output: String)
    fun initial(p: LLMPreset): Start {
        val c = overview?.current
        if (c != null && c.provider == p.id) return Start(c.baseUrl, c.model, c.output)
        return Start(p.saved?.baseUrl ?: p.baseUrl, p.saved?.model ?: p.model, p.saved?.output ?: p.output)
    }
    val savedKey = if (isCurrent) overview?.current?.apiKey else preset?.saved?.apiKey
    val base = baseUrl.trim(); val mod = model.trim(); val key = apiKey.trim()
    val changes: Map<String, JsonElement> = run {
        val p = preset ?: return@run emptyMap()
        val start = initial(p)
        val c = mutableMapOf<String, JsonElement>()
        if (!isCurrent) c["llm.provider"] = JsonPrimitive(p.id)
        if (!chatGPT && base != start.base) c["llm.base_url"] = JsonPrimitive(base)
        if (mod != start.model) c["llm.model"] = JsonPrimitive(mod)
        if (key.isNotEmpty()) c["llm.api_key"] = JsonPrimitive(key)
        if (output != start.output) c["llm.output"] = JsonPrimitive(output)
        c
    }
    val candidate: JsonObject = JsonObject(buildMap {
        put("provider", JsonPrimitive(provider)); put("output", JsonPrimitive(output))
        if (!chatGPT && base.isNotEmpty()) put("base_url", JsonPrimitive(base))
        if (mod.isNotEmpty()) put("model", JsonPrimitive(mod))
        if (key.isNotEmpty()) put("api_key", JsonPrimitive(key))
    })
    val addressMoved = preset?.let { AdminRules.origin(base) != AdminRules.origin(initial(it).base) } ?: false
    val keyPrompt = when {
        addressMoved && savedKey?.set == true -> "API key for this address"
        savedKey?.set == true -> savedKey.last4?.let { "Saved (…$it)" } ?: "Saved"
        preset?.key == "required" -> "API key"
        else -> "API key (if it needs one)"
    }
    val keyFooter: String? = when {
        chatGPT -> null
        addressMoved && savedKey?.set == true -> "The saved key stays with its own address and isn't sent to this one. Enter a key for this address, or change the address back."
        isCurrent && !addressMoved && overview?.current?.keysAside?.isNotEmpty() == true ->
            "A key saved for ${overview?.current?.keysAside?.mapNotNull { runCatching { java.net.URI(it).host }.getOrNull() }?.joinToString(", ")} is kept aside: it's used again if you go back to that address."
        else -> null
    }
    fun choose(id: String) {
        provider = id
        val p = overview?.presets?.firstOrNull { it.id == id } ?: return
        val s = initial(p)
        baseUrl = s.base; model = s.model; output = s.output; apiKey = ""; models = emptyList(); test = null; testError = null; note = null
    }

    LaunchedEffect(Unit) {
        adminCall({ error = it }) {
            val ov = api.llmOverview()
            if (ov == null) { error = "This server is too old for these settings: update it to 0.22 or later."; return@adminCall }
            val fields = runCatching { api.settingsDocument()["fields"]?.jsonObject }.getOrNull()
            val pf = fields?.get("llm.provider")?.jsonObject
            val src = (pf?.get("source") as? JsonPrimitive)?.contentOrNull
            lockedBy = if (src == "env" || src == "file") (pf?.get("env") as? JsonPrimitive)?.contentOrNull ?: "a variable" else null
            overview = ov
            if (provider.isEmpty() || ov.presets.none { it.id == provider }) choose(ov.current.provider)
            error = null
        }
        loading = false
    }

    FormPage("AI Matching", pages::pop) {
        if (loading && overview == null) SpinnerSection()
        val o = overview
        if (o != null) {
            FormSection(footer = if (lockedBy != null) "Set by $lockedBy in the server's environment." else preset?.help) {
                ChoiceRow("Provider", preset?.name ?: provider, o.presets.map { it.name to it.id }, enabled = lockedBy == null && !busy) { choose(it) }
            }
            if (chatGPT) ChatGPTAccountSection(o, accountNote) {
                scope.launch {
                    accountNote = null
                    adminCall({ accountNote = it to false }) {
                        val r = api.chatGPTSignOut()
                        overview = api.llmOverview() ?: overview
                        accountNote = if (r.revoked == false) (r.message ?: "Signed out here, but OpenAI didn't confirm that it ended the sign-in. To be sure, disconnect Moonrise in ChatGPT's settings.") to false
                            else "Signed out. Unclear file names wait in Fix Match until another provider is set up." to true
                    }
                }
            }
            if (!chatGPT || o.chatgpt.signedIn) {
                FormSection(if (chatGPT) "Model" else "Connection", footer = keyFooter) {
                    if (!chatGPT) {
                        LabeledField("Server URL", baseUrl, { baseUrl = it }, preset?.baseUrl?.takeIf { it.isNotEmpty() } ?: "http://server:8000/v1", keyboard = KeyboardType.Uri)
                        FormDivider()
                        LabeledField("API Key", apiKey, { apiKey = it }, keyPrompt, secure = true, keyboard = KeyboardType.Password)
                        FormDivider()
                    }
                    LabeledField("Model", model, { model = it }, if (provider == "llamacpp") "Optional: it serves the one it loaded" else "Model name")
                    FormDivider()
                    FormRow(onClick = { pages.push { ModelPickerPage(candidate, model, models) { model = it } } }) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text("Choose a Model", style = Type.body, color = Color.White, modifier = Modifier.weight(1f))
                            Chevron()
                        }
                    }
                }
                FormSection(footer = run {
                    var t = "Automatic tries a JSON schema, then a tool call, then JSON asked for in the prompt, and keeps the first that works."
                    if (isCurrent && o.current.output == "auto" && o.current.outputInUse != null && o.current.outputInUse != "auto") {
                        t += " Now: ${AdminRules.llmOutputs.firstOrNull { it.first == o.current.outputInUse }?.second ?: o.current.outputInUse}."
                    }
                    t
                }) {
                    ChoiceRow("Answer Format", AdminRules.llmOutputs.firstOrNull { it.first == output }?.second ?: output, AdminRules.llmOutputs.map { it.second to it.first }) { output = it }
                }
            }
            FormSection(footer = "Asks the model about one sample file name, with the settings above, before you save them.") {
                BusyRow("Test", testing, enabled = !busy && !(chatGPT && !o.chatgpt.signedIn)) {
                    testing = true; test = null; testError = null
                    scope.launch {
                        try { test = api.llmTest(candidate) } catch (e: Exception) { if (app.moonrise.core.ClientRules.isLoadFailure(e)) testError = adminErrorText(e) else throw e }
                        testing = false
                    }
                }
                test?.let { FormDivider(); TestRow(it) }
                testError?.let { FormDivider(); NoteRow(it, Theme.destructive) }
            }
            FormSection(footer = "Keys are kept on the server and never shown again. A key is only ever sent to the provider and address it was saved for: changing the address sets it aside, and it comes back if you return to that address or provider.") {
                BusyRow("Save", busy, enabled = changes.isNotEmpty() && lockedBy == null && !(chatGPT && !o.chatgpt.signedIn)) {
                    busy = true; note = null
                    scope.launch {
                        adminCall({ error = it }) {
                            api.updateSettings(changes)
                            apiKey = ""
                            note = "Saved."
                            error = null
                            overview = api.llmOverview() ?: overview
                        }
                        busy = false
                    }
                }
            }
        }
        note?.let { NoteSection(it, Theme.green) }
        error?.let { NoteSection(it, Theme.destructive) }
    }
}

/** The ChatGPT account the server signs in with: who, and Sign Out. Signing in is on the iPhone, iPad or Mac for now. */
@Composable
private fun ChatGPTAccountSection(o: LLMOverview, note: Pair<String, Boolean>?, signOut: () -> Unit) {
    val active = o.chatgpt.accounts?.firstOrNull { it.active }
    FormSection("ChatGPT Account", footer = if (o.chatgpt.needsSignIn == true) "The sign-in has ended: sign in again to keep matching with your plan." else "Uses your ChatGPT plan's included usage. In ChatGPT's settings you can set a weekly limit for Moonrise or disconnect it.") {
        if (o.chatgpt.signedIn) {
            InfoRow("Signed In As", active?.label ?: o.chatgpt.email ?: "ChatGPT")
            FormDivider()
            FormRow(onClick = signOut) { Text("Sign Out", style = Type.body, color = Theme.destructive) }
        } else {
            NoteRow("Sign in with ChatGPT on an iPhone, iPad or Mac (Settings → Manage Server → Services). This app can't sign in to ChatGPT yet.")
        }
        note?.let { (text, ok) -> FormDivider(); NoteRow(text, if (ok) Theme.green else Theme.destructive) }
    }
}

@Composable
private fun TestRow(r: LLMTestResult) {
    val secs = r.ms?.let { String.format(Locale.US, "%.1f s", it / 1000.0) }.orEmpty()
    val format = r.output?.let { o -> AdminRules.llmOutputs.firstOrNull { it.first == o }?.second }
    FormRow {
        when {
            r.ok && r.correct == true -> IconLine(Icons.Outlined.CheckCircle, "Works: it recognised the sample in $secs" + (format?.let { ", with $it." } ?: "."), Theme.green)
            r.ok -> IconLine(Icons.Outlined.Warning, "It answered in $secs, but picked the wrong title. A bigger model may match better; its answers are still checked before they're used.", Warn)
            else -> Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                IconLine(Icons.Outlined.Cancel, AdminRules.llmProblemText(r.problem), Theme.destructive)
                r.error?.let { Text(it, style = Type.caption, color = Theme.secondaryText) }
            }
        }
    }
}

@Composable
private fun IconLine(icon: androidx.compose.ui.graphics.vector.ImageVector, text: String, color: Color) {
    Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        Icon(icon, null, tint = color, modifier = Modifier.size(22.dp))
        Text(text, style = Type.body, color = color)
    }
}

/** The models an endpoint offers (its model list), searchable; picking one fills in the field (`LLMModelPicker`). */
@Composable
private fun ModelPickerPage(request: JsonObject, selected: String, preloaded: List<LLMModel>, pick: (String) -> Unit) {
    val api = Session.api ?: return
    val pages = LocalPages.current
    var models by remember { mutableStateOf<List<LLMModel>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var query by remember { mutableStateOf("") }
    LaunchedEffect(Unit) {
        if (preloaded.isNotEmpty()) { models = preloaded; return@LaunchedEffect }
        adminCall({ error = "Couldn't list the models: $it" }) { models = api.llmModels(request) }
    }
    val q = query.trim().lowercase()
    val shown = models.orEmpty().filter { q.isEmpty() || q in it.id.lowercase() || q in it.name.lowercase() }
    FormPage("Models", pages::pop) {
        FormSection {
            TextField(query, { query = it }, placeholder = { Text("Search", color = Theme.secondaryText) }, singleLine = true, modifier = Modifier.padding(horizontal = 0.dp),
                colors = TextFieldDefaults.colors(focusedContainerColor = Color.Transparent, unfocusedContainerColor = Color.Transparent, focusedIndicatorColor = Color.Transparent, unfocusedIndicatorColor = Color.Transparent, cursorColor = Color.White))
        }
        if (models == null && error == null) SpinnerSection()
        error?.let { NoteSection(it, Theme.destructive) }
        if (models?.isEmpty() == true) NoteSection("The server lists no models. Type the model's name instead.")
        if (shown.isNotEmpty()) {
            FormSection {
                shown.forEachIndexed { i, m ->
                    if (i > 0) FormDivider()
                    FormRow(onClick = { pick(m.id); pages.pop() }) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                                Text(m.name, style = Type.body, color = Color.White)
                                if (m.name != m.id) Text(m.id, style = Type.caption, color = Theme.secondaryText)
                            }
                            if (m.id == selected) Icon(Icons.Filled.Check, null, tint = Color(0xFF0A84FF))
                        }
                    }
                }
            }
        }
    }
}

