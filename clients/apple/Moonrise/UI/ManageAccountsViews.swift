import SwiftUI

// Manage Server → Users & Devices: the accounts, each one's signed-in devices (sign one out),
// new accounts, password resets, admin rights, deleting an account, and invitations (a one-time
// code a new person enters on the sign-in screen, choosing their own user name and password).
// Apple TV lists accounts and devices only.

struct AccountsAdminView: View {
    @State private var accounts: [MoonriseClient.Account] = []
    @State private var invites: [MoonriseClient.Invite] = []
    @State private var loading = true
    @State private var error: String?
    @State private var adding = false
    @State private var newInvite: MoonriseClient.Invite?
    @State private var inviteAdmin = false
    @State private var busy = false

    private var client: MoonriseClient? { AdminState.shared.client }

    var body: some View {
        Form { Group {
            Section {
                if loading && accounts.isEmpty { ProgressView() }
                ForEach(accounts) { a in
                    NavigationLink { AccountDetailView(account: a) { await load() } } label: { AccountRow(account: a) }
                }
                #if !os(tvOS)
                Button { adding = true } label: { Label("Add User", systemImage: "person.badge.plus").formRow() }.formRowButton()
                #endif
            } header: { Text("Users") }
            #if !os(tvOS)
            Section {
                ForEach(invites) { i in
                    LabeledContent {
                        Text("expires \(AdminFmt.ago(i.expiresAt) ?? i.expiresAt)").foregroundStyle(Theme.secondaryText)
                    } label: {
                        Label(i.admin ? "Admin Invitation" : "Invitation", systemImage: "envelope.open")
                    }
                    .swipeActions { Button("Cancel", role: .destructive) { Task { await cancel(i) } } }
                }
                Toggle("New Invitations Make Admins", isOn: $inviteAdmin).switchTint()
                Button { Task { await invite() } } label: { BusyLabel(title: "Invite Someone", busy: busy) }.formRow().disabled(busy)
            } header: { Text("Invitations") } footer: {
                Text("An invitation is a one-time code, good for 7 days. The new person opens Moonrise, picks this server and taps Have an Invitation, then chooses their own user name and password.")
            }
            #endif
            if let error { Section { NoteRow(text: error, color: .red) } }
        }.tvGlassRows() }
        .formStyle(.grouped)
        .managePage("Users & Devices", symbol: "person.2", message: "Everyone who can sign in, and the devices they're signed in on.")
        .task { await load() }
        #if !os(tvOS)
        .sheet(isPresented: $adding) { NavigationStack { NewAccountView { adding = false; Task { await load() } } } }
        .sheet(item: $newInvite) { inv in NavigationStack { InviteCodeView(invite: inv) } }
        #endif
    }

    private func load() async {
        guard let c = client else { return }
        loading = true
        defer { loading = false }
        do {
            accounts = try await c.accounts()
            #if !os(tvOS)
            invites = try await c.invites()
            #endif
            error = nil
        } catch {
            if !error.isCancellation { self.error = serverErrorText(error) }
        }
    }

    private func invite() async {
        guard let c = client else { return }
        busy = true
        defer { busy = false }
        do {
            newInvite = try await c.createInvite(admin: inviteAdmin)
            invites = (try? await c.invites()) ?? invites
        } catch { self.error = serverErrorText(error) }
    }

    private func cancel(_ i: MoonriseClient.Invite) async {
        guard let c = client else { return }
        do {
            try await c.deleteInvite(i.id)
            invites.removeAll { $0.id == i.id }
        } catch { self.error = serverErrorText(error) }
    }
}

private struct AccountRow: View {
    let account: MoonriseClient.Account
    var body: some View {
        Label {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(account.name).foregroundStyle(.white)
                    if account.me { Text("You").font(.caption2.weight(.semibold)).foregroundStyle(.black).padding(.horizontal, 6).padding(.vertical, 1).background(.white, in: Capsule()) }
                }
                Text(detail).font(.caption).foregroundStyle(Theme.secondaryText)
            }
        } icon: {
            Image(systemName: account.isAdmin ? "person.crop.circle.badge.checkmark" : "person.crop.circle").foregroundStyle(.white)
        }
    }

    private var detail: String {
        var parts = [account.isAdmin ? "Admin" : "Viewer", AdminFmt.count(account.devices, "device")]
        if let ago = AdminFmt.ago(account.lastUsed) { parts.append("active \(ago)") }
        return parts.joined(separator: " · ")
    }
}

/// One account: its devices (sign one out), password, admin rights, delete.
private struct AccountDetailView: View {
    let account: MoonriseClient.Account
    let changed: () async -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var devices: [MoonriseClient.Device] = []
    @State private var isAdmin: Bool
    @State private var loading = true
    @State private var error: String?
    @State private var note: String?
    @State private var resetting = false
    @State private var confirmDelete = false
    @State private var signingOut: MoonriseClient.Device?

    init(account: MoonriseClient.Account, changed: @escaping () async -> Void) {
        self.account = account
        self.changed = changed
        _isAdmin = State(initialValue: account.isAdmin)
    }

    private var client: MoonriseClient? { AdminState.shared.client }

    var body: some View {
        Form { Group {
            if let note { Section { NoteRow(text: note, color: .green) } }
            if let error { Section { NoteRow(text: error, color: .red) } }
            #if !os(tvOS)
            Section {
                Toggle(isOn: Binding(get: { isAdmin }, set: { v in Task { await setAdmin(v) } })) { Label("Admin", systemImage: "person.badge.key") }
                    .switchTint().disabled(account.me)
                Button { resetting = true } label: { Label("Reset Password", systemImage: "key").formRow() }.formRowButton()
            } footer: {
                Text(account.me ? "You can't take away your own admin rights." : "Admins manage the server from Settings → Manage Server.")
            }
            #endif
            Section {
                if loading && devices.isEmpty { ProgressView() }
                if !loading && devices.isEmpty { NoteRow(text: "Not signed in anywhere.") }
                ForEach(devices) { d in
                    #if os(tvOS)
                    InfoRowStack(title: d.deviceName ?? "A device", detail: deviceDetail(d), symbol: symbol(d))
                    #else
                    HStack {
                        InfoRowStack(title: d.deviceName ?? "A device", detail: deviceDetail(d), symbol: symbol(d))
                        Spacer(minLength: 0)
                        if !d.current {
                            Button("Sign Out") { signingOut = d }.font(.callout).foregroundStyle(.red).buttonStyle(.borderless)
                        }
                    }
                    #endif
                }
            } header: { Text("Signed In On") } footer: {
                #if !os(tvOS)
                Text("Signing a device out ends its sign-in at once; it can sign in again with the password.")
                #endif
            }
            #if !os(tvOS)
            if !account.me {
                Section {
                    Button(role: .destructive) { confirmDelete = true } label: { Text("Delete \(account.name)").foregroundStyle(.red).formRow() }.formRowButton()
                } footer: { Text("Their watch history goes with the account; their devices are signed out.") }
            }
            #endif
        }.tvGlassRows() }
        .formStyle(.grouped)
        .managePage(account.name, symbol: "person.crop.circle", message: account.isAdmin ? "An admin of this server." : "Can watch; can't manage the server.")
        .task { await load() }
        #if !os(tvOS)
        .sheet(isPresented: $resetting) { NavigationStack { ResetPasswordView(account: account) { resetting = false; note = "Password changed." } } }
        .confirmationDialog("Delete \(account.name)?", isPresented: $confirmDelete, titleVisibility: .visible) {
            Button("Delete Account", role: .destructive) { Task { await delete() } }
            Button("Cancel", role: .cancel) {}
        } message: { Text("Their watch history is deleted and every device they're signed in on is signed out.") }
        .confirmationDialog("Sign out \(signingOut?.deviceName ?? "this device")?", isPresented: Binding(get: { signingOut != nil }, set: { if !$0 { signingOut = nil } }), titleVisibility: .visible) {
            Button("Sign Out", role: .destructive) { if let d = signingOut { Task { await signOut(d) } } }
            Button("Cancel", role: .cancel) {}
        }
        #endif
    }

    private func symbol(_ d: MoonriseClient.Device) -> String {
        let n = (d.deviceName ?? "").lowercased()
        if n.contains("tv") { return "appletv" }
        if n.contains("ipad") { return "ipad" }
        if n.contains("mac") { return "laptopcomputer" }
        if n.contains("iphone") { return "iphone" }
        return "desktopcomputer"
    }

    private func deviceDetail(_ d: MoonriseClient.Device) -> String {
        if d.current { return "This device" }
        var parts: [String] = []
        if let a = AdminFmt.ago(d.lastUsed) { parts.append("Used \(a)") }
        else if let s = AdminFmt.ago(d.createdAt) { parts.append("Signed in \(s)") }
        return parts.isEmpty ? "Never used" : parts.joined(separator: " · ")
    }

    private func load() async {
        guard let c = client else { return }
        loading = true
        defer { loading = false }
        do {
            devices = try await c.devices(of: account.id)
            error = nil
        } catch {
            if !error.isCancellation { self.error = serverErrorText(error) }
        }
    }

    private func setAdmin(_ v: Bool) async {
        guard let c = client else { return }
        let old = isAdmin
        isAdmin = v
        do {
            try await c.updateAccount(account.id, admin: v)
            error = nil
            await changed()
        } catch {
            isAdmin = old
            self.error = serverErrorText(error)
        }
    }

    private func signOut(_ d: MoonriseClient.Device) async {
        guard let c = client else { return }
        do {
            try await c.signOutDevice(user: account.id, device: d.id)
            devices.removeAll { $0.id == d.id }
            note = "\(d.deviceName ?? "The device") is signed out."
            await changed()
        } catch { self.error = serverErrorText(error) }
    }

    private func delete() async {
        guard let c = client else { return }
        do {
            try await c.deleteAccount(account.id)
            await changed()
            dismiss()
        } catch { self.error = serverErrorText(error) }
    }
}

#if !os(tvOS)
/// A new account: a user name, a password (at least 8 characters), admin or not.
private struct NewAccountView: View {
    let done: () -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var password = ""
    @State private var again = ""
    @State private var admin = false
    @State private var busy = false
    @State private var error: String?

    private var client: MoonriseClient? { AdminState.shared.client }
    private var problem: String? {
        if name.trimmingCharacters(in: .whitespaces).isEmpty { return "" }
        if password.count < 8 { return "The password needs at least 8 characters." }
        if password != again { return "The passwords don't match." }
        return nil
    }

    var body: some View {
        Form {
            Section {
                TextField("User name", text: $name)
                    .autocorrectionDisabled()
                    #if os(iOS)
                    .textInputAutocapitalization(.never)
                    #endif
                    .accessibilityIdentifier("newUser")
                // Someone else's account: no user-name content type, so iOS doesn't offer to
                // generate (and save to this device) a strong password for it.
                SecureField("Password", text: $password).textContentType(.password).accessibilityIdentifier("newPass")
                SecureField("Password again", text: $again).textContentType(.password).accessibilityIdentifier("newPass2")
                Toggle("Admin", isOn: $admin).switchTint()
            } footer: {
                if let p = problem, !p.isEmpty, !password.isEmpty { Text(p).foregroundStyle(.orange) }
                else { Text("They sign in with these on their devices. Admins can manage the server.") }
            }
            if let error { Section { Text(error).foregroundStyle(.red) } }
        }
        .formStyle(.grouped)
        .navigationTitle("New User")
        .inlineTitle()
        .toolbar {
            ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            ToolbarItem(placement: .confirmationAction) {
                Button { Task { await create() } } label: { if busy { ProgressView() } else { Text("Add") } }
                    .disabled(problem != nil || busy)
            }
        }
    }

    private func create() async {
        guard let c = client else { return }
        busy = true
        defer { busy = false }
        do {
            try await c.createAccount(name: name.trimmingCharacters(in: .whitespaces), password: password, admin: admin)
            done()
        } catch { self.error = serverErrorText(error) }
    }
}

private struct ResetPasswordView: View {
    let account: MoonriseClient.Account
    let done: () -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var password = ""
    @State private var again = ""
    @State private var busy = false
    @State private var error: String?

    private var client: MoonriseClient? { AdminState.shared.client }
    private var problem: String? {
        if password.count < 8 { return "At least 8 characters." }
        if password != again { return "The passwords don't match." }
        return nil
    }

    var body: some View {
        Form {
            Section {
                SecureField("New password", text: $password).textContentType(.password).accessibilityIdentifier("resetPass")
                SecureField("New password again", text: $again).textContentType(.password).accessibilityIdentifier("resetPass2")
            } footer: {
                if let p = problem, !password.isEmpty { Text(p).foregroundStyle(.orange) }
                else { Text("Devices already signed in stay signed in.") }
            }
            if let error { Section { Text(error).foregroundStyle(.red) } }
        }
        .formStyle(.grouped)
        .navigationTitle("Password for \(account.name)")
        .inlineTitle()
        .toolbar {
            ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            ToolbarItem(placement: .confirmationAction) {
                Button { Task { await save() } } label: { if busy { ProgressView() } else { Text("Save") } }.disabled(problem != nil || busy)
            }
        }
    }

    private func save() async {
        guard let c = client else { return }
        busy = true
        defer { busy = false }
        do {
            try await c.updateAccount(account.id, password: password)
            done()
        } catch { self.error = serverErrorText(error) }
    }
}

/// The new invitation's code (shown once), to read out or share.
private struct InviteCodeView: View {
    let invite: MoonriseClient.Invite
    @Environment(\.dismiss) private var dismiss

    private var code: String { invite.code ?? "" }
    private var server: String { SourceStore.shared.server?.name ?? "the server" }
    private var shareText: String {
        "Join \(server) in Moonrise: open Moonrise, pick \(server), tap Have an Invitation and enter \(code). The code works once, for 7 days."
    }

    var body: some View {
        Form {
            Section {
                Text(code)
                    .font(.system(size: 44, weight: .bold, design: .monospaced))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 12)
                    .textSelection(.enabled)
                    .accessibilityIdentifier("inviteCode")
            } footer: {
                Text("Works once, until \(AdminFmt.date(invite.expiresAt)?.formatted(date: .abbreviated, time: .omitted) ?? "it expires")\(invite.admin ? ", for an admin account" : ""). It isn't shown again.")
            }
            Section {
                ShareLink(item: shareText) { Label("Share Invitation", systemImage: "square.and.arrow.up") }
                Button { copy() } label: { Label("Copy Code", systemImage: "doc.on.doc").formRow() }.formRowButton()
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Invitation")
        .inlineTitle()
        .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
    }

    private func copy() {
        #if os(iOS)
        UIPasteboard.general.string = code
        #elseif os(macOS)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(code, forType: .string)
        #endif
    }
}
#endif

// MARK: - Sign-in with an invitation

/// The sign-in screen's "Have an Invitation?": the code from an admin, then a user name and
/// password of one's own; signed in on this device.
struct InviteSignInView: View {
    let url: URL
    let info: MoonriseClient.Info
    @State private var code = ""
    @State private var username = ""
    @State private var password = ""
    @State private var busy = false
    @State private var error: String?

    private var problem: String? {
        if code.filter({ $0.isLetter || $0.isNumber }).count != 8 { return "" }
        if username.trimmingCharacters(in: .whitespaces).isEmpty { return "" }
        if password.count < 8 { return "The password needs at least 8 characters." }
        return nil
    }

    var body: some View {
        content
    }

    #if os(tvOS)
    private var content: some View {
        TVSetupScreen(icon: .symbol("envelope.open"), title: "Join \(info.name)",
                      message: "Enter the invitation code an admin of this server gave you, then pick your user name and password.") {
            VStack(alignment: .leading, spacing: 28) {
                TVField(label: "Invitation code") { MoonriseTextField(title: "Invitation code", text: $code, prompt: "XXXX-XXXX").autocorrectionDisabled() }
                TVField(label: "User name") { MoonriseTextField(title: "User name", text: $username).autocorrectionDisabled() }
                TVField(label: "Password") { MoonriseTextField(title: "Password", text: $password, secure: true) }
                Button { Task { await join() } } label: { BusyLabel(title: "Join", busy: busy) }
                    .tvGlassButton(prominent: true)
                    .disabled(problem != nil || busy)
                if let error { Text(error).font(.callout).foregroundStyle(.red) }
            }
        }
    }
    #else
    private var content: some View {
        Form {
            Section {
                TextField("XXXX-XXXX", text: $code)
                    .font(.system(.title3, design: .monospaced))
                    .autocorrectionDisabled()
                    #if os(iOS)
                    .textInputAutocapitalization(.characters)
                    #endif
                    .accessibilityIdentifier("inviteCodeField")
            } header: { Text("Invitation Code") } footer: { Text("From an admin of \(info.name).") }
            Section {
                TextField("User name", text: $username)
                    .autocorrectionDisabled()
                    #if os(iOS)
                    .textInputAutocapitalization(.never)
                    .textContentType(.username)
                    #endif
                    .accessibilityIdentifier("inviteUser")
                SecureField("Password (at least 8 characters)", text: $password).textContentType(.password).accessibilityIdentifier("invitePass")
            } header: { Text("Your Account") } footer: {
                if let p = problem, !p.isEmpty, !password.isEmpty { Text(p).foregroundStyle(.orange) }
                else { Text("Use these to sign in on your other devices too.") }
            }
            Section {
                Button { Task { await join() } } label: { BusyLabel(title: "Join \(info.name)", busy: busy) }.formRow().disabled(problem != nil || busy)
            }
            if let error { Section { Text(error).foregroundStyle(.red) } }
        }
        .formStyle(.grouped)
        .navigationTitle("Have an Invitation")
        .inlineTitle()
    }
    #endif

    private func join() async {
        busy = true
        defer { busy = false }
        let c = MoonriseClient(sourceId: nil, baseURL: url, token: nil)
        do {
            let auth = try await c.redeemInvite(code: code, username: username.trimmingCharacters(in: .whitespaces), password: password)
            SourceStore.shared.signIn(SignInView.source(info: info, url: url, auth: auth))
        } catch let MoonriseClientError.http(429, _) {
            error = "Too many wrong codes. Try again in a few minutes."
        } catch {
            self.error = serverErrorText(error)
        }
    }
}
