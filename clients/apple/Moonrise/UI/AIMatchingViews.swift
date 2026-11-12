#if !os(tvOS)
import AuthenticationServices
import Network
import SwiftUI

// Manage Server → Services → AI Matching: which language model the server asks about unclear
// file names (server 0.22, llm_admin.rs). A provider preset fills in the address, model and
// answer format; any OpenAI-compatible server works through "Other"; a ChatGPT plan is used
// through Sign in with ChatGPT. Test runs one real identification before anything is saved.

struct AIMatchingAdminView: View {
    @State private var overview: MoonriseClient.LLMOverview?
    @State private var loading = true
    @State private var provider = ""
    @State private var baseURL = ""
    @State private var model = ""
    @State private var apiKey = ""
    @State private var output = "auto"
    @State private var models: [MoonriseClient.LLMModel] = []
    @State private var testing = false
    @State private var test: MoonriseClient.LLMTestResult?
    @State private var testError: String?
    @State private var busy = false
    @State private var signingIn = false
    /// The sign-in in progress: a registration's id, "new" for another account, "" for the one
    /// in use.
    @State private var signingInAs: String?
    /// What the last sign-in or sign-out said, shown in the account section.
    @State private var accountNote: (text: String, ok: Bool)?
    @State private var error: String?
    @State private var note: String?
    /// `llm.provider` comes from the server's environment: it can't be changed here.
    @State private var lockedBy: String?

    private var client: MoonriseClient? { AdminState.shared.client }
    private var preset: MoonriseClient.LLMPreset? { overview?.presets.first { $0.id == provider } }
    private var isCurrent: Bool { provider == overview?.current.provider }
    private var chatGPT: Bool { preset?.key == "sign_in" }

    static let outputs: [(String, String)] = [("auto", "Automatic"), ("schema", "JSON Schema"), ("tool", "Tool Call"), ("prompt", "JSON in the Prompt")]

    /// What a provider starts from: the server's settings for the current one, else what was
    /// kept from before, else the preset.
    private func initial(_ p: MoonriseClient.LLMPreset) -> (base: String, model: String, output: String) {
        if let c = overview?.current, c.provider == p.id { return (c.baseUrl, c.model, c.output) }
        return (p.saved?.baseUrl ?? p.baseUrl, p.saved?.model ?? p.model, p.saved?.output ?? p.output)
    }

    private var savedKey: MoonriseClient.SecretState? {
        isCurrent ? overview?.current.apiKey : preset?.saved?.apiKey
    }

    private var trimmed: (base: String, model: String, key: String) {
        let t = { (s: String) in s.trimmingCharacters(in: .whitespacesAndNewlines) }
        return (t(baseURL), t(model), t(apiKey))
    }

    /// The settings to change (dotted keys); empty: nothing to save.
    private var changes: [String: Any] {
        guard let p = preset else { return [:] }
        let start = initial(p)
        var c: [String: Any] = [:]
        if !isCurrent { c["llm.provider"] = p.id }
        if !chatGPT, trimmed.base != start.base { c["llm.base_url"] = trimmed.base }
        if trimmed.model != start.model { c["llm.model"] = trimmed.model }
        if !trimmed.key.isEmpty { c["llm.api_key"] = trimmed.key }
        if output != start.output { c["llm.output"] = output }
        return c
    }

    /// What the server's test and model list are asked about.
    private var candidate: [String: Any] {
        var b: [String: Any] = ["provider": provider, "output": output]
        if !chatGPT, !trimmed.base.isEmpty { b["base_url"] = trimmed.base }
        if !trimmed.model.isEmpty { b["model"] = trimmed.model }
        if !trimmed.key.isEmpty { b["api_key"] = trimmed.key }
        return b
    }

    var body: some View {
        Form {
            if loading, overview == nil { Section { ProgressView() } }
            if let o = overview {
                Section {
                    Picker("Provider", selection: Binding(get: { provider }, set: { choose($0) })) {
                        ForEach(o.presets) { p in Text(p.name).tag(p.id) }
                    }
                    .disabled(lockedBy != nil || busy || signingIn)
                    .accessibilityIdentifier("llmProvider")
                } footer: {
                    if let lockedBy {
                        Text("Set by \(lockedBy) in the server's environment.")
                    } else if let p = preset {
                        Text(p.help)
                    }
                }
                if chatGPT { accountSection(o) }
                if !chatGPT || o.chatgpt.signedIn { connectionSection }
                if !chatGPT || o.chatgpt.signedIn {
                    Section {
                        Picker("Answer Format", selection: $output) {
                            ForEach(Self.outputs, id: \.0) { Text($0.1).tag($0.0) }
                        }
                    } footer: {
                        Text(outputFooter(o))
                    }
                }
                Section {
                    Button { Task { await runTest() } } label: { BusyLabel(title: "Test", busy: testing) }
                        .formRow()
                        .disabled(testing || busy || (chatGPT && !o.chatgpt.signedIn))
                        .accessibilityIdentifier("llmTest")
                    if let test { testRow(test) }
                    if let testError { Text(testError).font(.callout).foregroundStyle(.red) }
                } footer: {
                    Text("Asks the model about one sample file name, with the settings above, before you save them.")
                }
                Section {
                    Button { Task { await save() } } label: { BusyLabel(title: "Save", busy: busy) }
                        .formRow()
                        .disabled(changes.isEmpty || busy || lockedBy != nil || (chatGPT && !o.chatgpt.signedIn))
                        .accessibilityIdentifier("llmSave")
                } footer: {
                    Text("Keys are kept on the server and never shown again. A key is only ever sent to the provider and address it was saved for: changing the address sets it aside, and it comes back if you return to that address or provider.")
                }
            }
            if let note { Section { NoteRow(text: note, color: .green) } }
            if let error { Section { NoteRow(text: error, color: .red) } }
        }
        .formStyle(.grouped)
        .managePage("AI Matching", symbol: "sparkles", message: "The language model that reads unclear file names.")
        .task { await load() }
    }

    @ViewBuilder private func accountSection(_ o: MoonriseClient.LLMOverview) -> some View {
        let accounts = o.chatgpt.accounts ?? []
        let active = accounts.first { $0.active }
        Section {
            if o.chatgpt.signedIn {
                LabeledContent("Signed In As", value: active?.label ?? o.chatgpt.email ?? "ChatGPT")
                Button(role: .destructive) { Task { await signOut() } } label: { Text("Sign Out").formRow() }
                    .formRowButton()
                    .disabled(signingIn)
                    .accessibilityIdentifier("chatgptSignOut")
            } else {
                Button { Task { await signIn() } } label: {
                    BusyLabel(title: (active?.label ?? o.chatgpt.email).flatMap { $0.isEmpty ? nil : "Sign In as \($0)" } ?? "Sign In with ChatGPT", busy: signingInAs == "")
                }
                    .formRow()
                    .disabled(signingIn)
                    .accessibilityIdentifier("chatgptSignIn")
            }
            // The other accounts Moonrise is registered with: each keeps its own registration,
            // and switching back signs in with it again.
            ForEach(accounts.filter { !$0.active }) { a in
                Button { Task { await signIn(account: a.id) } } label: { BusyLabel(title: "Switch to \(a.label)", busy: signingInAs == a.id) }
                    .formRow()
                    .disabled(signingIn)
            }
            if !accounts.isEmpty || o.chatgpt.email != nil {
                Button { Task { await signIn(newAccount: true) } } label: { BusyLabel(title: "Use Another Account", busy: signingInAs == "new") }
                    .formRow()
                    .disabled(signingIn)
                    .accessibilityIdentifier("chatgptOtherAccount")
            }
            // Registrations OpenAI made for a sign-in that never finished: nothing uses them.
            ForEach(o.chatgpt.unfinished ?? []) { u in
                LabeledContent("Unfinished Sign-In") {
                    Button("Forget") { Task { await forget(u.id) } }
                        .disabled(signingIn)
                        .accessibilityIdentifier("chatgptForget")
                }
            }
            if let n = accountNote { NoteRow(text: n.text, color: n.ok ? .green : .red) }
        } header: { Text("ChatGPT Account") } footer: {
            Text(accountFooter(o))
        }
    }

    private func accountFooter(_ o: MoonriseClient.LLMOverview) -> String {
        var text = o.chatgpt.needsSignIn == true
            ? "The sign-in has ended: sign in again to keep matching with your plan."
            : "Uses your ChatGPT plan's included usage. In ChatGPT's settings you can set a weekly limit for Moonrise or disconnect it."
        if o.chatgpt.unfinished?.isEmpty == false {
            text += " An unfinished sign-in left Moonrise registered with OpenAI without an account; signing in doesn't use it. Forget removes it here; if ChatGPT's settings list it, you can remove it there too."
        }
        return text
    }

    @ViewBuilder private var connectionSection: some View {
        Section {
            if !chatGPT {
                LabeledField(label: "Server URL") {
                    TextField("Server URL", text: $baseURL, prompt: Text(preset?.baseUrl.isEmpty == false ? preset!.baseUrl : "http://server:8000/v1"))
                        .textContentType(.URL)
                        .autocorrectionDisabled()
                        #if os(iOS)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        #endif
                        .accessibilityIdentifier("llmBaseURL")
                }
            }
            if !chatGPT {
                LabeledField(label: "API Key") {
                    SecureField("API key", text: $apiKey, prompt: Text(keyPrompt))
                        .accessibilityIdentifier("llmKey")
                }
            }
            LabeledField(label: "Model") {
                TextField("Model", text: $model, prompt: Text(provider == "llamacpp" ? "Optional: it serves the one it loaded" : "Model name"))
                    .autocorrectionDisabled()
                    #if os(iOS)
                    .textInputAutocapitalization(.never)
                    #endif
                    .accessibilityIdentifier("llmModel")
            }
            NavigationLink {
                LLMModelPicker(client: client, request: candidate, selection: $model, preloaded: models)
            } label: {
                Text("Choose a Model").foregroundStyle(.white)
            }
            .accessibilityIdentifier("llmModels")
        } header: { Text(chatGPT ? "Model" : "Connection") } footer: {
            if let f = keyFooter { Text(f) }
        }
    }

    /// Where the saved key stays when the address changes.
    private var keyFooter: String? {
        guard !chatGPT else { return nil }
        if addressMoved, savedKey?.set == true {
            return "The saved key stays with its own address and isn't sent to this one. Enter a key for this address, or change the address back."
        }
        if isCurrent, !addressMoved, let aside = overview?.current.keysAside, !aside.isEmpty {
            let hosts = aside.compactMap { URLComponents(string: $0)?.host }.joined(separator: ", ")
            return "A key saved for \(hosts) is kept aside: it's used again if you go back to that address."
        }
        return nil
    }

    /// The address field points somewhere other than where the saved key is kept.
    private var addressMoved: Bool {
        guard let p = preset else { return false }
        return Self.origin(trimmed.base) != Self.origin(initial(p).base)
    }

    /// scheme://host:port of an address, as the server binds keys to it.
    static func origin(_ s: String) -> String? {
        guard let u = URLComponents(string: s), let scheme = u.scheme?.lowercased(), let host = u.host?.lowercased() else { return nil }
        let port = u.port ?? (scheme == "https" ? 443 : scheme == "http" ? 80 : 0)
        return "\(scheme)://\(host):\(port)"
    }

    private var keyPrompt: String {
        if addressMoved, savedKey?.set == true { return "API key for this address" }
        if let s = savedKey, s.set { return s.last4.map { "Saved (…\($0))" } ?? "Saved" }
        return preset?.key == "required" ? "API key" : "API key (if it needs one)"
    }

    private func outputFooter(_ o: MoonriseClient.LLMOverview) -> String {
        var t = "Automatic tries a JSON schema, then a tool call, then JSON asked for in the prompt, and keeps the first that works."
        if isCurrent, o.current.output == "auto", let used = o.current.outputInUse, used != "auto" {
            t += " Now: \(Self.outputs.first { $0.0 == used }?.1 ?? used)."
        }
        return t
    }

    @ViewBuilder private func testRow(_ r: MoonriseClient.LLMTestResult) -> some View {
        let secs = r.ms.map { String(format: "%.1f\u{00A0}s", Double($0) / 1000) } ?? ""
        let format = r.output.flatMap { o in Self.outputs.first { $0.0 == o }?.1 }
        if r.ok, r.correct == true {
            Label("Works: it recognised the sample in \(secs)" + (format.map { ", with \($0)." } ?? "."), systemImage: "checkmark.circle")
                .foregroundStyle(.green)
        } else if r.ok {
            Label("It answered in \(secs), but picked the wrong title. A bigger model may match better; its answers are still checked before they're used.", systemImage: "exclamationmark.triangle")
                .foregroundStyle(.orange)
        } else {
            VStack(alignment: .leading, spacing: 4) {
                Label(problemText(r.problem), systemImage: "xmark.circle").foregroundStyle(.red)
                if let e = r.error { Text(e).font(.caption).foregroundStyle(Theme.secondaryText).textSelection(.enabled) }
            }
        }
    }

    private func problemText(_ p: String?) -> String {
        switch p {
        case "unreachable": return "The server couldn't reach that address."
        case "auth": return "The key or sign-in was refused."
        case "not_found": return "Not found: check the address and the model name."
        case "format": return "The model didn't answer in a form Moonrise can read. Try another answer format or model."
        case "limit": return "The usage limit is reached."
        default: return "It didn't work."
        }
    }

    private func choose(_ id: String) {
        provider = id
        guard let p = preset else { return }
        let s = initial(p)
        baseURL = s.base
        model = s.model
        output = s.output
        apiKey = ""
        models = []
        test = nil
        testError = nil
        note = nil
    }

    private func load() async {
        guard let c = client else { return }
        loading = true
        defer { loading = false }
        do {
            async let o = c.llmOverview()
            async let d = c.settingsDocument()
            guard let ov = try await o else {
                error = "This server is too old for these settings: update it to 0.22 or later."
                return
            }
            let fields = (try? await d)?["fields"] as? [String: Any] ?? [:]
            let pf = fields["llm.provider"] as? [String: Any] ?? [:]
            let src = pf["source"] as? String
            lockedBy = (src == "env" || src == "file") ? (pf["env"] as? String ?? "a variable") : nil
            overview = ov
            if provider.isEmpty || !ov.presets.contains(where: { $0.id == provider }) {
                choose(ov.current.provider)
            }
            error = nil
        } catch {
            if !error.isCancellation { self.error = serverErrorText(error) }
        }
    }

    private func runTest() async {
        guard let c = client else { return }
        testing = true
        defer { testing = false }
        test = nil
        testError = nil
        do {
            test = try await c.llmTest(candidate)
        } catch {
            testError = serverErrorText(error)
        }
    }

    private func save() async {
        guard let c = client else { return }
        let ch = changes
        guard !ch.isEmpty else { return }
        busy = true
        defer { busy = false }
        note = nil
        do {
            try await c.updateSettings(ch)
            apiKey = ""
            note = "Saved."
            error = nil
            overview = try await c.llmOverview() ?? overview
        } catch {
            self.error = serverErrorText(error)
        }
    }

    private func signIn(newAccount: Bool = false, account: String? = nil) async {
        guard let c = client else { return }
        signingIn = true
        signingInAs = newAccount ? "new" : (account ?? "")
        defer { signingIn = false; signingInAs = nil }
        error = nil
        note = nil
        accountNote = nil
        do {
            let r: MoonriseClient.ChatGPTSignedIn
            do {
                r = try await ChatGPTSignIn().run(client: c, newAccount: newAccount, account: account)
            } catch ChatGPTSignIn.Failure.refused(let state) {
                // OpenAI turned the one-time code down. As OpenAI asks, sign in once more with the
                // registration that attempt made (only this retry uses it), in a private browser
                // session in case a stale OpenAI session in Safari got in the way. A moment first,
                // so the first sheet is gone before the second one opens (on a device, a sheet
                // opened while another is closing may not show).
                accountNote = ("OpenAI didn't accept that sign-in. Trying once more…", true)
                try? await Task.sleep(for: .milliseconds(800))
                r = try await ChatGPTSignIn().run(client: c, retry: state, fresh: true)
            }
            // The plan is the provider from now on, with the account's first model unless one
            // was chosen.
            var ch: [String: Any] = [:]
            if overview?.current.provider != "chatgpt" { ch["llm.provider"] = "chatgpt" }
            let current = overview?.current.provider == "chatgpt" ? overview?.current.model ?? "" : ""
            if current.isEmpty, let first = r.models.first { ch["llm.model"] = first.id }
            if !ch.isEmpty { try await c.updateSettings(ch) }
            overview = try await c.llmOverview() ?? overview
            choose("chatgpt")
            models = r.models
            accountNote = ("Signed in as \(r.email).", true)
        } catch ChatGPTSignIn.Failure.cancelled {
            // Closed by the user. A retry closed so leaves its registration unfinished.
            accountNote = nil
            overview = (try? await c.llmOverview()) ?? overview
        } catch {
            accountNote = (serverErrorText(error), false)
            overview = (try? await c.llmOverview()) ?? overview
        }
    }

    private func forget(_ id: String) async {
        guard let c = client else { return }
        signingIn = true
        defer { signingIn = false }
        accountNote = nil
        do {
            try await c.chatGPTForget(account: id)
            overview = try await c.llmOverview() ?? overview
            accountNote = ("Forgotten. If ChatGPT's settings still list it, you can remove it there.", true)
        } catch {
            accountNote = (serverErrorText(error), false)
        }
    }

    private func signOut() async {
        guard let c = client else { return }
        signingIn = true
        defer { signingIn = false }
        error = nil
        note = nil
        accountNote = nil
        do {
            let r = try await c.chatGPTSignOut()
            overview = try await c.llmOverview() ?? overview
            if r.revoked == false {
                accountNote = (r.message ?? "Signed out here, but OpenAI didn't confirm that it ended the sign-in. To be sure, disconnect Moonrise in ChatGPT's settings.", false)
            } else {
                accountNote = ("Signed out. Unclear file names wait in Fix Match until another provider is set up.", true)
            }
        } catch {
            accountNote = (serverErrorText(error), false)
        }
    }
}

/// A field with its name above it (a filled-in field alone doesn't say what it is).
private struct LabeledField<Field: View>: View {
    let label: String
    @ViewBuilder let field: () -> Field
    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(label).font(.caption).foregroundStyle(Theme.secondaryText)
            field().labelsHidden()
        }
    }
}

/// The models an endpoint offers (its model list), searchable; picking one fills in the field.
private struct LLMModelPicker: View {
    let client: MoonriseClient?
    let request: [String: Any]
    @Binding var selection: String
    var preloaded: [MoonriseClient.LLMModel] = []
    @State private var models: [MoonriseClient.LLMModel]?
    @State private var error: String?
    @State private var query = ""
    @Environment(\.dismiss) private var dismiss

    private var shown: [MoonriseClient.LLMModel] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        return (models ?? []).filter { q.isEmpty || $0.id.lowercased().contains(q) || $0.name.lowercased().contains(q) }
    }

    var body: some View {
        Form {
            if models == nil, error == nil { Section { ProgressView() } }
            if let error { Section { NoteRow(text: error, color: .red) } }
            if let models, models.isEmpty { Section { NoteRow(text: "The server lists no models. Type the model's name instead.") } }
            if !shown.isEmpty {
                Section {
                    ForEach(shown) { m in
                        Button {
                            selection = m.id
                            dismiss()
                        } label: {
                            HStack {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(m.name).foregroundStyle(.white)
                                    if m.name != m.id { Text(m.id).font(.caption).foregroundStyle(Theme.secondaryText) }
                                }
                                Spacer()
                                if m.id == selection { Image(systemName: "checkmark").foregroundStyle(Color.accentColor) }
                            }
                            .formRow()
                        }
                        .formRowButton()
                    }
                }
            }
        }
        .formStyle(.grouped)
        .searchable(text: $query)
        .navigationTitle("Models")
        .inlineTitle()
        .task {
            if !preloaded.isEmpty { models = preloaded; return }
            guard let client else { return }
            do { models = try await client.llmModels(request) } catch { self.error = "Couldn't list the models: " + serverErrorText(error) }
        }
    }
}

/// The browser half of Sign in with ChatGPT. OpenAI sends the browser back to a loopback
/// address (`http://127.0.0.1:<port>/auth/callback`), so the app listens there while the sign-in
/// page is open, then hands the code to the server, which holds the PKCE verifier and exchanges
/// it. No token passes through the app.
@MainActor
final class ChatGPTSignIn: NSObject, ASWebAuthenticationPresentationContextProviding {
    enum Failure: LocalizedError {
        case cancelled
        case denied(String)
        case noListener
        case noBrowser
        /// OpenAI refused the code (the server answered 409): one retry may use this attempt's
        /// `state`.
        case refused(state: String)
        var errorDescription: String? {
            switch self {
            case .cancelled: "Cancelled."
            case .refused: "OpenAI didn't accept that sign-in."
            case .denied(let why): "ChatGPT didn't sign in: \(why.hasSuffix(".") ? String(why.dropLast()) : why)."
            case .noListener: "Couldn't receive the sign-in on this device."
            case .noBrowser: "Couldn't open the sign-in page. Try again."
            }
        }
    }

    private var listener: NWListener?
    private var session: ASWebAuthenticationSession?
    private var continuation: CheckedContinuation<ChatGPTCallback, Error>?
    /// The `state` of this attempt's authorization URL: a callback with any other is not this
    /// sign-in's, and is turned away without ending it.
    private var expectedState: String?
    /// The request bytes of each connection, until its head is complete.
    private var heads: [ObjectIdentifier: Data] = [:]

    /// `fresh`: a browser session that shares no cookies with Safari (a second try, so that a
    /// stale OpenAI session in Safari can't get in the way).
    /// `retry`: the `state` of the attempt whose code was refused ([`Failure.refused`]).
    func run(client: MoonriseClient, newAccount: Bool = false, account: String? = nil, retry: String? = nil, fresh: Bool = false) async throws -> MoonriseClient.ChatGPTSignedIn {
        let port = try await listen()
        defer { stop() }
        let auth = try await client.chatGPTStart(redirect: "http://127.0.0.1:\(port)/auth/callback", newAccount: newAccount, account: account, retry: retry)
        expectedState = URLComponents(url: auth, resolvingAgainstBaseURL: false)?.queryItems?.first { $0.name == "state" }?.value
        let callback = try await withCheckedThrowingContinuation { (k: CheckedContinuation<ChatGPTCallback, Error>) in
            continuation = k
            let s = ASWebAuthenticationSession(url: auth, callback: .customScheme("moonrise-signin")) { [weak self] _, error in
                // Only a close by the user ends it here; the answer arrives at the listener.
                Task { @MainActor in
                    if let error, (error as? ASWebAuthenticationSessionError)?.code == .canceledLogin {
                        self?.resume(.failure(Failure.cancelled))
                    } else if let error {
                        self?.resume(.failure(error))
                    }
                }
            }
            s.presentationContextProvider = self
            s.prefersEphemeralWebBrowserSession = fresh
            session = s
            if !s.start() {
                // The sheet couldn't open (another one still closing, say): once more, shortly.
                Task { @MainActor [weak self] in
                    try? await Task.sleep(for: .seconds(1))
                    guard let self, self.continuation != nil else { return }
                    if self.session?.start() != true { self.resume(.failure(Failure.noBrowser)) }
                }
            }
        }
        session?.cancel()
        // Read as OpenAI's reference client reads it (form-encoded, decoded once).
        let value = callback.value
        let count = callback.count
        if let err = value("error") {
            // For the server's log, and to end the attempt there.
            if let state = value("state") { await client.chatGPTReportError(state: state, error: err, description: value("error_description")) }
            throw Failure.denied(value("error_description") ?? err)
        }
        guard let code = value("code"), !code.isEmpty, let state = value("state") else { throw Failure.denied("no code in the answer") }
        // One code and at most one client id: anything else is not a clean answer from OpenAI.
        guard count("code") == 1, count("state") == 1, count("client_id") <= 1 else { throw Failure.denied("the answer was malformed (a parameter given twice)") }
        do {
            return try await client.chatGPTFinish(state: state, code: code, clientId: value("client_id"), callbackParams: callback.names)
        } catch MoonriseClientError.http(409, _) {
            throw Failure.refused(state: state)
        }
    }

    private func resume(_ r: Result<ChatGPTCallback, Error>) {
        guard let k = continuation else { return }
        continuation = nil
        k.resume(with: r)
    }

    private func stop() {
        listener?.cancel()
        listener = nil
        session?.cancel()
        session = nil
    }

    /// Listens on 127.0.0.1 (1455, else a port of the system's choosing); answers the port.
    private func listen() async throws -> UInt16 {
        for port: UInt16 in [1455, 0] {
            let params = NWParameters.tcp
            params.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: NWEndpoint.Port(rawValue: port)!)
            params.allowLocalEndpointReuse = true
            guard let l = try? NWListener(using: params) else { continue }
            l.newConnectionHandler = { [weak self] conn in
                Task { @MainActor in self?.serve(conn) }
            }
            let ready: UInt16? = await withCheckedContinuation { k in
                var done = false
                l.stateUpdateHandler = { state in
                    guard !done else { return }
                    switch state {
                    case .ready: done = true; k.resume(returning: l.port?.rawValue)
                    case .failed, .cancelled: done = true; k.resume(returning: nil)
                    default: break
                    }
                }
                l.start(queue: .main)
            }
            if let p = ready, p != 0 {
                listener = l
                return p
            }
            l.cancel()
        }
        throw Failure.noListener
    }

    /// One request to the listener: this sign-in's callback (answered with a short page), or
    /// anything else (turned away). The whole request head is read before answering.
    private func serve(_ conn: NWConnection) {
        conn.start(queue: .main)
        receiveHead(conn)
    }

    private func receiveHead(_ conn: NWConnection) {
        conn.receive(minimumIncompleteLength: 1, maximumLength: 16_384) { [weak self] data, _, isComplete, error in
            Task { @MainActor in
                guard let self else { conn.cancel(); return }
                let key = ObjectIdentifier(conn)
                var head = self.heads[key] ?? Data()
                if let data { head.append(data) }
                let ended = head.range(of: Data("\r\n\r\n".utf8)) != nil
                if !ended, !isComplete, error == nil, head.count < 65_536 {
                    self.heads[key] = head
                    self.receiveHead(conn)
                    return
                }
                self.heads[key] = nil
                self.answer(conn, head: head)
            }
        }
    }

    private func answer(_ conn: NWConnection, head: Data) {
        let callback = ChatGPTCallback(requestHead: head)
        let isCallback = callback.map { $0.method == "GET" && $0.path == "/auth/callback" && $0.count("state") == 1 && $0.value("state") == expectedState } == true
            && continuation != nil && expectedState != nil
        let page = isCallback
            ? "<!doctype html><meta name=viewport content='width=device-width'><body style='font:17px -apple-system;padding:2em'><h2>Moonrise</h2><p>Moonrise has the sign-in. You can go back to the app.</p>"
            : "Not found"
        let reply = "HTTP/1.1 \(isCallback ? "200 OK" : "404 Not Found")\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Length: \(page.utf8.count)\r\nConnection: close\r\n\r\n\(page)"
        conn.send(content: Data(reply.utf8), completion: .contentProcessed { _ in conn.cancel() })
        if isCallback, let callback { resume(.success(callback)) }
    }

    nonisolated func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        MainActor.assumeIsolated {
            #if os(macOS)
            return NSApplication.shared.keyWindow ?? NSApplication.shared.windows.first ?? ASPresentationAnchor()
            #else
            let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            return scenes.flatMap(\.windows).first { $0.isKeyWindow } ?? scenes.first?.windows.first ?? ASPresentationAnchor()
            #endif
        }
    }
}
#endif
