import SwiftUI

/// "Search Online…" sheet: the Moonrise server searches OpenSubtitles.com with its own account (a
/// fetched track is kept on the server and offered on every device). A download's are found the
/// same way, through the server it came from, and the track is kept with the download. The app
/// itself contacts no other service.
struct SubtitleSearchView: View {
    let model: PlayerModel
    @Environment(\.dismiss) private var dismiss

    @State private var language = AppSettings.shared.preferredSubtitleLanguage
    @State private var query = ""
    @State private var moonriseResults: [RemoteSubtitle] = []
    @State private var moonriseRemaining: Int?
    /// The server's OpenSubtitles quota (servers 0.12.0+).
    @State private var moonriseQuota: MoonriseServer.SubtitleQuota?
    @State private var loading = false
    @State private var busyId: String?
    @State private var error: String?

    private var ctx: SubtitleSearchContext { model.searchContext }

    #if os(tvOS)
    @FocusState private var focusedResult: String?
    #endif

    var body: some View {
        #if os(tvOS)
        tvBody
            .task {
                query = ctx.title
                await search()
            }
        #else
        standardBody
        #endif
    }

    private var standardBody: some View {
        NavigationStack {
            List {
                Section {
                    TextField("Title", text: $query)
                    Picker("Language", selection: $language) {
                        ForEach(Self.languages, id: \.0) { Text($0.1).tag($0.0) }
                    }
                    Button {
                        Task { await search() }
                    } label: {
                        HStack {
                            Text("Search")
                            if loading { Spacer(); ProgressView() }
                        }
                    }
                    .disabled(loading)
                } footer: {
                    if let line = quotaLine {
                        Text(line).foregroundStyle(moonriseQuota?.exhausted == true ? Color.orange : Theme.secondaryText)
                    }
                }
                if let error {
                    Section { Text(error).foregroundStyle(.red) }
                }
                if !moonriseResults.isEmpty {
                    Section {
                        ForEach(moonriseResults) { r in
                            row(title: r.name, detail: moonriseDetail(r), match: r.hashMatch, busy: busyId == "moonrise:\(r.id)") {
                                Task { await pickMoonrise(r) }
                            }
                            .disabled(outOfDownloads(r))
                        }
                    } header: {
                        Text("OpenSubtitles · Moonrise Server")
                    }
                }
            }
            .navigationTitle("Find Subtitles")
            .inlineTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } }
            }
        }
        .task {
            query = ctx.title
            await search()
        }
        #if os(macOS)
        .frame(minWidth: 520, minHeight: 520)
        #endif
    }

    #if os(tvOS)
    // MARK: Apple TV

    /// Over the paused player: the search on the left (title, language, Search), results on the
    /// right as focusable rows. Menu closes it (fullScreenCover), playback resumes.
    private var tvBody: some View {
        HStack(alignment: .top, spacing: 80) {
            VStack(alignment: .leading, spacing: 30) {
                VStack(alignment: .leading, spacing: 10) {
                    Text("Find Subtitles").font(.title2.bold())
                    Text(tvContextLine).font(.callout).foregroundStyle(.secondary).lineLimit(2)
                }
                TVField(label: "Title") {
                    MoonriseTextField(title: "Title", text: $query)
                        .autocorrectionDisabled()
                        .onSubmit { Task { await search() } }
                }
                TVField(label: "Language") {
                    Menu {
                        Picker("Language", selection: $language) {
                            ForEach(Self.languages, id: \.0) { Text($0.1).tag($0.0) }
                        }
                        .pickerStyle(.inline)
                    } label: {
                        HStack(spacing: 14) {
                            Image(systemName: "globe")
                            Text(Self.languages.first { $0.0 == language }?.1 ?? language.uppercased())
                            Spacer(minLength: 10)
                            Image(systemName: "chevron.down").font(.caption.weight(.bold))
                        }
                        .frame(maxWidth: .infinity)
                    }
                    .tvGlassButton(height: 66)
                }
                Button { Task { await search() } } label: {
                    HStack(spacing: 14) {
                        Image(systemName: "magnifyingglass")
                        Text("Search").font(.body.bold())
                    }
                    .frame(maxWidth: .infinity)
                }
                .tvGlassButton(prominent: true, height: 66)
                .disabled(loading)
                if let line = quotaLine {
                    Text(line)
                        .font(.caption)
                        .foregroundStyle(moonriseQuota?.exhausted == true ? Color.orange : Color.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(width: 560, alignment: .leading)
            .focusSection()

            tvResults
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .focusSection()
        }
        .padding(.top, 20)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background {
            // The paused picture, dimmed and blurred, stays behind.
            Rectangle().fill(.ultraThinMaterial).overlay(Color.black.opacity(0.45)).ignoresSafeArea()
        }
        .presentationBackground(.clear)
        .onChange(of: language) { _, _ in Task { await search() } }
        // Results in: the first one takes the focus (the remote is on the list straight away).
        .onChange(of: loading) { _, busy in
            guard !busy else { return }
            if let r = moonriseResults.first { focusedResult = "moonrise:\(r.id)" }
        }
    }

    /// "The Sopranos · S4 · E13 · Whitecaps"
    private var tvContextLine: String {
        guard let item = ctx.server?.1 else { return ctx.title }
        if item.isEpisode {
            return [item.seriesName, item.episodeLabel, item.name].compactMap { $0 }.joined(separator: " · ")
        }
        return [item.displayTitle, item.productionYear.map(String.init)].compactMap { $0 }.joined(separator: " · ")
    }

    @ViewBuilder private var tvResults: some View {
        VStack(alignment: .leading, spacing: 24) {
            if let error {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if loading {
                HStack(spacing: 20) { ProgressView(); Text("Searching OpenSubtitles…").foregroundStyle(.secondary) }
                    .padding(.top, 20)
            } else if moonriseResults.isEmpty {
                if error == nil {
                    Text("No subtitles found. Try another title or language.")
                        .foregroundStyle(.secondary).padding(.top, 20)
                }
            } else {
                Text("\(moonriseResults.count) results · OpenSubtitles")
                    .font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                ScrollView(.vertical) {
                    LazyVStack(alignment: .leading, spacing: 18) {
                        ForEach(moonriseResults) { r in
                            tvRow(id: "moonrise:\(r.id)", title: r.name, detail: moonriseDetail(r), match: r.hashMatch, unavailable: outOfDownloads(r)) {
                                Task { await pickMoonrise(r) }
                            }
                        }
                    }
                    // Room for the focused row's lift.
                    .padding(.horizontal, 30).padding(.vertical, 20)
                }
                .padding(.horizontal, -30)
                .scrollClipDisabled()
            }
        }
    }

    private func tvRow(id: String, title: String, detail: String, match: Bool, unavailable: Bool = false, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 20) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(title).font(.callout.weight(.medium)).lineLimit(1)
                    Text(detail).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer(minLength: 20)
                if match { Image(systemName: "checkmark.seal.fill") }
                if busyId == id { ProgressView() }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .buttonStyle(TVGlassRowStyle(cornerRadius: 28, outset: CGSize(width: 28, height: 16), scale: 1.02))
        .focused($focusedResult, equals: id)
        .disabled(busyId != nil && busyId != id)
        .opacity(unavailable ? 0.5 : 1)
    }
    #endif

    private func row(title: String, detail: String, match: Bool, busy: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text(title).lineLimit(2)
                    Text(detail).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                if match { Image(systemName: "checkmark.seal.fill").foregroundStyle(Theme.accent) }
                if busy { ProgressView() }
            }
        }
        .disabled(busyId != nil)
    }

    /// The server and the title to search for: the one playing, or the one a download came from.
    private var target: (client: any MediaServerClient, itemId: String)? {
        if let (source, item) = ctx.server, let client = MediaServers.client(for: source) { return (client, item.id) }
        if let (source, mediaId) = ctx.download, let client = MediaServers.client(for: source) { return (client, mediaId) }
        return nil
    }

    private func search() async {
        loading = true
        error = nil
        defer { loading = false }
        moonriseResults = []
        guard let (client, itemId) = target else {
            error = SubtitleSearchContext.needsServer
            return
        }
        do {
            let a = try await client.searchSubtitles(itemId: itemId, language: language, query: query == ctx.title ? nil : query)
            moonriseResults = a.results
            moonriseRemaining = a.remainingDownloads
            moonriseQuota = a.quota
        } catch let e as MoonriseServer.ServerError where e.status == 429 {
            // The Moonrise server's account is out of downloads for today.
            self.error = e.localizedDescription
        } catch {
            self.error = ctx.download != nil ? "\(SubtitleSearchContext.needsServer) (\(error.localizedDescription))"
                : "Server search failed: \(error.localizedDescription)"
        }
    }

    /// "3 of 5 downloads left today · Add an OpenSubtitles account for 20 a day" (older servers:
    /// what is left).
    private var quotaLine: String? {
        if let q = moonriseQuota { return q.summary }
        return moonriseRemaining.map { $0 == 1 ? "1 download left today on the server's account." : "\($0) downloads left today on the server's account." }
    }

    /// No downloads left: only results already saved on the server can be picked.
    private func outOfDownloads(_ r: RemoteSubtitle) -> Bool {
        !r.fetched && (moonriseQuota?.exhausted == true || moonriseRemaining == 0)
    }

    private var exhaustedMessage: String {
        var parts = ["No OpenSubtitles downloads left today on your server."]
        if let r = moonriseQuota?.resetLine { parts.append(r + ".") } else { parts.append("Try again tomorrow.") }
        if moonriseQuota?.tier == "anonymous" { parts.append("An OpenSubtitles account on the server allows \(moonriseQuota?.accountAllowed ?? 20) a day.") }
        parts.append("Results marked Saved still work.")
        return parts.joined(separator: " ")
    }

    private func moonriseDetail(_ r: RemoteSubtitle) -> String {
        var parts = [r.language?.uppercased() ?? "?", "\(r.downloads ?? 0) downloads"]
        if r.hearingImpaired { parts.append("SDH") }
        if r.machineTranslated { parts.append("Machine translated") }
        if r.fetched { parts.append("Saved") }
        return parts.joined(separator: " · ")
    }

    private func pickMoonrise(_ r: RemoteSubtitle) async {
        guard let (client, itemId) = target, client.moonriseBaseURL != nil else { return }
        busyId = "moonrise:\(r.id)"
        defer { busyId = nil }
        do {
            if case let .moonriseTrack(track) = try await client.fetchSubtitle(itemId: itemId, result: r) {
                if ctx.server != nil {
                    await model.moonriseSubtitleFetched(track, byUser: true)
                } else if let url = client.subtitleURL(itemId: itemId, versionId: "", index: track.index) {
                    // A download: the track's text, from the server, kept with the download.
                    let data = try await client.fetch(url, timeout: 30)
                    model.addDownloadedSubtitle(data, title: r.name, language: r.language ?? track.language)
                }
            }
            dismiss()
        } catch let e as MoonriseServer.ServerError where e.status == 429 {
            // 406/429 from OpenSubtitles: the server's account is out of downloads (or too fast).
            moonriseRemaining = 0
            self.error = exhaustedMessage
        } catch {
            self.error = error.localizedDescription
        }
    }

    static let languages: [(String, String)] = [
        ("en", "English"), ("ru", "Russian"), ("uk", "Ukrainian"), ("es", "Spanish"), ("fr", "French"),
        ("de", "German"), ("it", "Italian"), ("pt", "Portuguese"), ("nl", "Dutch"), ("pl", "Polish"),
        ("sv", "Swedish"), ("ja", "Japanese"), ("ko", "Korean"), ("zh", "Chinese"), ("ar", "Arabic"),
        ("he", "Hebrew"), ("tr", "Turkish"), ("el", "Greek"), ("cs", "Czech"), ("fi", "Finnish"),
    ]
}
