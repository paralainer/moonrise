import SwiftUI

// Fix Match (admins; iPhone, iPad, Mac): the titles Moonrise couldn't recognize for sure (Manage
// Server → Fix Match) and any title's "Fix Match…" (its long-press menu). A movie or show is
// matched to a TMDB title (the candidates kept from identification, found now when none were
// kept, or a search), or kept as named from its files; an episode to one of its show's episodes
// in TMDB's numbering (or kept as numbered). The server identifies it again and fetches its
// artwork and details. Apple TV doesn't offer it (typing searches with the remote).

/// Presents Fix Match over everything (from a title's long-press menu).
@MainActor
@Observable
final class FixMatchPresenter {
    static let shared = FixMatchPresenter()
    struct Target: Identifiable { let id: String }
    var target: Target?

    func present(_ id: String) { target = Target(id: id) }
}

extension Notification.Name {
    /// A title was matched again by hand (object: its id now): pages showing it reload.
    static let moonriseItemRematched = Notification.Name("moonriseItemRematched")
}

/// A title page's title: long-press for Fix Match (admins only; nothing for others).
struct FixMatchTitleMenu: ViewModifier {
    let item: MediaItem
    func body(content: Content) -> some View {
        #if os(tvOS)
        content
        #else
        if AdminState.shared.isAdmin {
            content.contextMenu { FixMatchMenuItem(item: item) }
        } else {
            content
        }
        #endif
    }
}

/// A candidate's poster from the server (its route is for admins: the request carries the token);
/// the film icon when there is none or it fails.
private struct MatchPoster: View {
    let url: URL?
    let headers: [String: String]
    @State private var loaded: (url: URL, image: PlatformImage)?

    var body: some View {
        Rectangle().fill(Theme.surfaceStrong)
            .overlay {
                if let loaded, loaded.url == url {
                    Image(platformImage: loaded.image).resizable().aspectRatio(contentMode: .fill)
                } else {
                    Image(systemName: "film").foregroundStyle(.secondary)
                }
            }
            .frame(width: 46, height: 69)
            .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
            .task(id: url) {
                guard let url, let image = await ImageCache.shared.image(for: url, headers: headers) else { return }
                loaded = (url, image)
            }
    }
}

#if !os(tvOS)
/// "Fix Match…" in a title's long-press menu, for admins (movies, shows, episodes).
struct FixMatchMenuItem: View {
    let item: MediaItem
    var body: some View {
        if AdminState.shared.isAdmin, item.kind == .movie || item.kind == .series || item.kind == .episode {
            Button { FixMatchPresenter.shared.present(item.id) } label: { Label("Fix Match…", systemImage: "wand.and.stars") }
        }
    }
}

extension View {
    /// The root's Fix Match sheet (`FixMatchPresenter`).
    func fixMatchSheet() -> some View {
        modifier(FixMatchSheet())
    }
}

private struct FixMatchSheet: ViewModifier {
    @Bindable private var presenter = FixMatchPresenter.shared
    func body(content: Content) -> some View {
        content.sheet(item: $presenter.target) { t in
            NavigationStack {
                FixMatchView(itemId: t.id, inSheet: true) { newId in
                    presenter.target = nil
                    if let newId { NotificationCenter.default.post(name: .moonriseItemRematched, object: newId) }
                }
            }
            #if os(macOS)
            .frame(minWidth: 520, minHeight: 600)
            #endif
        }
    }
}

// MARK: - The review list

struct ReviewListView: View {
    @State private var items: [MoonriseClient.ReviewItem] = []
    @State private var loading = true
    @State private var error: String?

    private var client: MoonriseClient? { AdminState.shared.client }

    /// Movies and shows first, then episodes by show.
    private var groups: [(title: String, items: [MoonriseClient.ReviewItem])] {
        var out: [(String, [MoonriseClient.ReviewItem])] = []
        let titles = items.filter { !$0.isEpisode }
        if !titles.isEmpty { out.append(("Movies and Shows", titles)) }
        var order: [String] = []
        var bySeries: [String: [MoonriseClient.ReviewItem]] = [:]
        for e in items where e.isEpisode {
            let k = e.seriesTitle ?? "Episodes"
            if bySeries[k] == nil { order.append(k) }
            bySeries[k, default: []].append(e)
        }
        out += order.map { ($0, bySeries[$0] ?? []) }
        return out
    }

    var body: some View {
        Form {
            if loading && items.isEmpty { Section { ProgressView() } }
            if !loading && items.isEmpty && error == nil {
                Section { Label("Every title is recognized.", systemImage: "checkmark.circle").foregroundStyle(.green) }
            }
            ForEach(groups, id: \.title) { g in
                Section {
                    ForEach(g.items) { r in
                        NavigationLink { FixMatchView(itemId: r.id, inSheet: false) { _ in Task { await load() } } } label: { row(r) }
                    }
                } header: { Text(g.title) } footer: {
                    if g.items.allSatisfy({ $0.isEpisode && ($0.note ?? "").hasSuffix("isn't in TMDB's season list") }) {
                        Text(g.items.count == 1 ? "TMDB doesn't list this episode under this number: pick the one it is, or keep the number."
                                                : "TMDB doesn't list these episodes under these numbers: pick the ones they are, or keep the numbers.")
                    }
                }
            }
            if let error { Section { Text(error).foregroundStyle(.red) } }
        }
        .formStyle(.grouped)
        .navigationTitle("Fix Match")
        .inlineTitle()
        .task { await load() }
        .refreshable { await load() }
    }

    private func row(_ r: MoonriseClient.ReviewItem) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(r.isEpisode ? "S\(r.season ?? 0)E\(r.episode ?? 0)\(r.episodeEnd.map { "–E\($0)" } ?? "") · \(r.title)" : r.heading)
                .foregroundStyle(.white).lineLimit(1)
            Text(reason(r)).font(.caption).foregroundStyle(Theme.secondaryText).lineLimit(2)
        }
    }

    private func reason(_ r: MoonriseClient.ReviewItem) -> String {
        if r.isEpisode { return r.fileName ?? r.note ?? "" }
        guard let n = r.note else { return r.fileName ?? "" }
        if n.hasSuffix("isn't in TMDB's season list") { return "TMDB numbers this show's episodes differently" }
        if n == "no confident TMDB match" { return r.candidates.isEmpty ? "No sure match on TMDB" : "\(AdminFmt.count(r.candidates.count, "possible match", "possible matches"))" }
        return n
    }

    private func load() async {
        guard let c = client else { return }
        loading = true
        defer { loading = false }
        do {
            items = try await c.reviewList()
            error = nil
        } catch {
            if !error.isCancellation { self.error = serverErrorText(error) }
        }
    }
}

// MARK: - One title

struct FixMatchView: View {
    let itemId: String
    let inSheet: Bool
    /// One of the title's files alone (its other files stay as they are).
    var fileId: String? = nil
    /// Done (the title's id now), or cancelled (nil).
    let done: (String?) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var info: MoonriseClient.MatchInfo?
    @State private var loading = true
    @State private var error: String?
    @State private var query = ""
    @State private var year = ""
    @State private var results: [MoonriseClient.MatchCandidate]?
    @State private var searching = false
    @State private var busy = false
    @State private var pick: Pick?
    /// Episodes: the show whose episodes are listed, and the season shown.
    @State private var show: (id: Int, title: String)?
    @State private var season: MoonriseClient.TMDBSeason?
    @State private var seasonNo: Int?
    @State private var changingShow = false
    /// What it is taken for here: its own kind, or the other ("movie" or "episode": a wrong
    /// classification corrected).
    @State private var asType: String?
    @State private var confirmAutomatic: AutomaticTarget?
    /// A file split off: its own Fix Match next.
    @State private var splitItem: SplitItem?
    @State private var notice: String?

    struct AutomaticTarget: Identifiable { let fileId: String?; var id: String { fileId ?? "-" } }
    struct SplitItem: Identifiable, Hashable { let id: String }

    private var client: MoonriseClient? { AdminState.shared.client }
    /// movie, episode or series, as it will be matched.
    private var kind: String { asType ?? info?.kind ?? "movie" }
    private var switched: Bool { asType != nil && asType != info?.kind }
    private var isEpisode: Bool { kind == "episode" }
    private var searchKind: String { kind == "movie" ? "movie" : "tv" }

    /// What will be done once confirmed.
    enum Pick: Identifiable {
        case title(MoonriseClient.MatchCandidate)
        /// `existing`: files the library has of that episode already (then: a part, or a version?).
        case episode(show: Int, season: Int, episode: Int, title: String, existing: Int = 0, parts: Int = 0)
        case local
        var id: String {
            switch self {
            case let .title(c): "t\(c.tmdbId)"
            case let .episode(s, se, e, _, _, _): "e\(s):\(se):\(e)"
            case .local: "local"
            }
        }
    }

    /// A TMDB episode as a pick: with what the library has of it besides this title.
    private func episodePick(show: Int, _ e: MoonriseClient.TMDBSeason.Episode, title: String? = nil) -> Pick {
        var files = e.inLibrary?.files ?? 0, titles = e.inLibrary?.titles ?? 0
        if let i = info, fileId == nil, i.seriesTmdbId == show, i.season == e.season, i.episode == e.episode {
            // This title is there already: only the others count.
            files -= max(1, i.fileList?.count ?? 1)
            titles -= 1
        }
        return .episode(show: show, season: e.season, episode: e.episode, title: title ?? e.title ?? "", existing: max(0, files), parts: max(0, titles))
    }

    var body: some View {
        Form {
            if let i = info {
                Section {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(heading(i)).font(.headline).foregroundStyle(.white)
                        if let f = fileName(i) { Text(f).font(.caption).foregroundStyle(Theme.secondaryText).lineLimit(2) }
                        Text(stateText(i)).font(.caption).foregroundStyle(i.state == "review" ? .orange : Theme.secondaryText)
                    }
                    .padding(.vertical, 2)
                    if let n = notice { Label(n, systemImage: "checkmark.circle").font(.footnote).foregroundStyle(.green) }
                    if fileId == nil, i.manual == true {
                        Button { confirmAutomatic = AutomaticTarget(fileId: nil) } label: {
                            Label("Back to Automatic", systemImage: "arrow.uturn.backward").formRow()
                        }
                        .formRowButton()
                        .accessibilityIdentifier("backToAutomatic")
                    }
                } header: { Text(fileId != nil ? "This File" : i.kind == "episode" ? "This Episode" : "This Title") } footer: {
                    if fileId == nil, i.manual == true {
                        Text("Back to Automatic undoes the matches made by hand: Moonrise identifies \(i.kind == "series" ? "the show's episodes" : "its files") on its own again.")
                    }
                }
                if fileId == nil, let files = i.fileList, files.count > 1 { filesSection(i, files) }
                if i.tmdb, i.kind == "movie" || i.kind == "episode" || i.kind == "series" {
                    typeSection(i)
                }
                if !i.tmdb {
                    Section { Text("The server has no TMDB key, so titles can't be looked up. Add one in Manage Server → Services.").foregroundStyle(.orange) }
                } else if isEpisode {
                    episodeSections(i)
                } else {
                    titleSections(i)
                }
            } else if loading {
                Section { HStack { ProgressView(); Text("Looking for matches…").foregroundStyle(.secondary) } }
            }
            if let error { Section { Text(error).foregroundStyle(.red) } }
        }
        .formStyle(.grouped)
        .navigationTitle("Fix Match")
        .inlineTitle()
        .toolbar {
            if inSheet { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { done(nil) } } }
        }
        .disabled(busy)
        .overlay { if busy { ProgressView("Matching…").padding(24).background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16)) } }
        .task { await load() }
        .task(id: seasonKey) { await loadSeason() }
        .confirmationDialog(pickTitle, isPresented: Binding(get: { pick != nil }, set: { if !$0 { pick = nil } }), titleVisibility: .visible, presenting: pick) { p in
            if case let .episode(_, _, _, _, existing, parts) = p, existing > 0 {
                // That episode has a file: this one is another part of it, or another version.
                Button("Part \(max(parts, 1) + 1) of This Episode") { Task { await apply(p, part: true) } }
                Button("Another Version of It") { Task { await apply(p) } }
            } else if case let .title(c) = p, isOtherTitle(c) {
                Button("Add as Another Version") { Task { await apply(p) } }
            } else {
                Button(p.id == "local" ? "Use the File Name" : (pickIsKeep(p) ? "Keep" : "Match")) { Task { await apply(p) } }
            }
            Button("Cancel", role: .cancel) {}
        } message: { p in
            Text(pickMessage(p))
        }
        .confirmationDialog(confirmAutomatic?.fileId == nil ? "Back to automatic?" : "This file back to automatic?",
                            isPresented: Binding(get: { confirmAutomatic != nil }, set: { if !$0 { confirmAutomatic = nil } }),
                            titleVisibility: .visible, presenting: confirmAutomatic) { t in
            Button("Back to Automatic") { Task { await automatic(t.fileId) } }
            Button("Cancel", role: .cancel) {}
        } message: { t in
            Text(t.fileId == nil
                 ? "Moonrise forgets the matches made by hand for \(info?.kind == "series" ? "this show's episodes" : "this title's files") and identifies them on its own again. Watch history stays."
                 : "Moonrise forgets the match made by hand for this file and identifies it on its own again. Watch history stays.")
        }
        .navigationDestination(item: $splitItem) { t in
            FixMatchView(itemId: t.id, inSheet: false, done: done)
        }
    }

    private var seasonKey: String { "\(show?.id ?? 0):\(seasonNo ?? -1)" }

    // MARK: Type

    /// A movie that is an episode, or an episode (or a whole show) that is a movie: Moonrise tells
    /// them apart by the file names and folders, and this corrects it.
    @ViewBuilder private func typeSection(_ i: MoonriseClient.MatchInfo) -> some View {
        let own = i.kind
        // The other choice: an episode for a movie; its own kind (episode or show) otherwise.
        let other = own == "movie" ? "episode" : own
        Section {
            Picker("This Is", selection: Binding(get: { kind == "movie" ? "movie" : other }, set: { switchType(to: $0, from: i) })) {
                Text("A Movie").tag("movie")
                Text(own == "series" ? "A Show" : "An Episode").tag(other)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .accessibilityIdentifier("matchType")
        } header: { Text("This Is") } footer: {
            if switched {
                Text(kind == "movie" ? "Pick the movie below: \(own == "series" ? "its files" : "the file") will be a movie from now on."
                                     : "Pick its show and episode below: the file will be an episode from now on.")
            }
        }
    }

    private func switchType(to t: String, from i: MoonriseClient.MatchInfo) {
        let own = i.kind
        asType = t == own ? nil : t
        results = nil
        pick = nil
        if kind == "movie" {
            // Look the file's own title up as a movie.
            query = switched ? Self.fileTitle(i) : i.title
            if let y = switched ? Self.fileYear(i) : i.year { year = String(y) } else { year = "" }
            if switched { Task { await search() } }
        } else if kind == "episode" {
            if switched {
                show = nil
                season = nil
                seasonNo = nil
                changingShow = true
                query = Self.fileTitle(i)
                Task { await search() }
            } else {
                changingShow = false
                query = i.seriesTitle ?? ""
                if let s = i.seriesTmdbId { show = (s, i.seriesTitle ?? ""); seasonNo = i.season }
            }
        } else {
            query = i.title
        }
    }

    /// The year a file name gives ("Kill.Bill.Vol.1.2003.1080p.mkv" → 2003).
    static func fileYear(_ i: MoonriseClient.MatchInfo) -> Int? {
        guard let f = i.files.first else { return nil }
        let name = (f as NSString).lastPathComponent
        guard let r = name.range(of: #"(?<![0-9])(19|20)[0-9]{2}(?![0-9])"#, options: .regularExpression) else { return nil }
        return Int(name[r])
    }

    /// The title a file name gives ("Kill.Bill.Vol.1.2003.1080p.mkv" → "Kill Bill Vol 1").
    static func fileTitle(_ i: MoonriseClient.MatchInfo) -> String {
        guard let f = i.files.first else { return i.seriesTitle ?? i.title }
        var name = ((f as NSString).lastPathComponent as NSString).deletingPathExtension
        name = name.replacingOccurrences(of: ".", with: " ").replacingOccurrences(of: "_", with: " ")
        var words: [String] = []
        for w in name.split(separator: " ") {
            let s = String(w)
            if s.count == 4, let y = Int(s), (1900...2099).contains(y) { break }
            if s.range(of: #"^[sS]\d{1,2}[eE]\d{1,3}"#, options: .regularExpression) != nil { break }
            if s.first == "[" || s.first == "(" || ["1080p", "720p", "2160p", "480p"].contains(s.lowercased()) { break }
            words.append(s)
        }
        let t = words.joined(separator: " ").trimmingCharacters(in: CharacterSet(charactersIn: " -"))
        return t.isEmpty ? i.title : t
    }

    // MARK: Movies and shows

    @ViewBuilder private func titleSections(_ i: MoonriseClient.MatchInfo) -> some View {
        if !switched {
            Section {
                if i.candidates.isEmpty { Text("No likely matches: search below.").foregroundStyle(Theme.secondaryText) }
                ForEach(i.candidates) { c in candidateRow(c, current: c.tmdbId == i.tmdbId && !i.local) }
            } header: { Text("Likely Matches") }
        }
        searchSection
        if let results {
            Section {
                if results.isEmpty { Text("Nothing found.").foregroundStyle(Theme.secondaryText) }
                ForEach(results) { c in candidateRow(c, current: c.tmdbId == i.tmdbId && !i.local) }
            } header: { Text("Results") }
        }
        if !i.local || switched {
            Section {
                Button { pick = .local } label: { Label("Don't Match: Use the File Name", systemImage: "doc.text").formRow() }.formRowButton()
            } footer: {
                Text("Named from the file and its folder, with artwork from the folder or the video; it isn't looked up again.")
            }
        }
    }

    private var searchSection: some View {
        Section {
            LabeledContent("Title") {
                TextField(searchKind == "movie" ? "Movie title" : "Show title", text: $query)
                    .multilineTextAlignment(.trailing)
                    .autocorrectionDisabled()
                    .onSubmit { Task { await search() } }
                    .accessibilityIdentifier("matchQuery")
            }
            if !isEpisode {
                LabeledContent("Year") {
                    TextField("Any", text: $year)
                        .multilineTextAlignment(.trailing)
                        #if os(iOS)
                        .keyboardType(.numberPad)
                        #endif
                        .accessibilityIdentifier("matchYear")
                }
            }
            Button { Task { await search() } } label: { BusyLabel(title: "Search TMDB", busy: searching) }
                .formRow()
                .disabled(query.trimmingCharacters(in: .whitespaces).isEmpty || searching)
        } header: { Text(isEpisode ? "Find the Show" : "Search") }
    }

    private func candidateRow(_ c: MoonriseClient.MatchCandidate, current: Bool) -> some View {
        Button {
            if isEpisode {
                show = (c.tmdbId, c.title)
                seasonNo = nil
                changingShow = false
                results = nil
            } else {
                pick = .title(c)
            }
        } label: {
            HStack(alignment: .top, spacing: 12) {
                MatchPoster(url: client?.matchPosterURL(c.posterPath), headers: client?.headers ?? [:])
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 6) {
                        Text(c.year.map { "\(c.title) (\($0))" } ?? c.title).foregroundStyle(.white).lineLimit(2)
                        if current { Image(systemName: "checkmark").foregroundStyle(.green) }
                    }
                    if let o = c.originalTitle, o != c.title, !o.isEmpty { Text(o).font(.caption).foregroundStyle(Theme.secondaryText).lineLimit(1) }
                    if isOtherTitle(c) { Text("\(Image(systemName: "square.stack")) In your library: adds a version").font(.caption).foregroundStyle(.green) }
                    if let ov = c.overview, !ov.isEmpty { Text(ov).font(.caption).foregroundStyle(Theme.secondaryText).lineLimit(2) }
                }
                Spacer(minLength: 0)
            }
            .contentShape(Rectangle())
        }
        .formRow()
        .formRowButton()
    }

    // MARK: Episodes

    @ViewBuilder private func episodeSections(_ i: MoonriseClient.MatchInfo) -> some View {
        Section {
            LabeledContent("Show", value: show?.title ?? i.seriesTitle ?? "—")
            if let s = season, s.seasons.count > 1 {
                Picker("Season", selection: Binding(get: { seasonNo ?? s.season }, set: { seasonNo = $0 })) {
                    ForEach(s.seasons, id: \.self) { n in Text(n == 0 ? "Specials" : "Season \(n)").tag(n) }
                }
            }
            if let s = season, s.tmdbId == show?.id {
                ForEach(s.episodes.filter { likely($0, i) }) { e in
                    Button { pick = episodePick(show: s.tmdbId, e) } label: {
                        Label { Text("S\(e.season)E\(e.episode) · \(e.title ?? "")").foregroundStyle(.white) } icon: { Image(systemName: "sparkles").foregroundStyle(.green) }
                            .formRow()
                    }
                    .formRowButton()
                }
            }
            if !switched, let s = show, s.id == i.seriesTmdbId, let se = i.season, let ep = i.episode {
                Button { pick = .episode(show: s.id, season: se, episode: ep, title: "") } label: {
                    Label("Keep as S\(se)E\(ep)\(i.part.map { " Part \($0)" } ?? "")", systemImage: "checkmark.circle").formRow()
                }
                .formRowButton()
            }
            if !switched || show != nil {
                Button { changingShow.toggle(); results = nil } label: {
                    Label(changingShow ? "Keep This Show" : "A Different Show…", systemImage: "magnifyingglass").formRow()
                }
                .formRowButton()
                .disabled(switched && show == nil)
            }
        } header: { Text("Match To") } footer: {
            if switched {
                Text(show == nil ? "Find its show below, then pick the episode." : "Pick the episode as TMDB numbers it.")
            } else {
                Text("Pick the episode as TMDB numbers it. Keeping its own number takes it off this list; it plays as it is, without TMDB's details.")
            }
        }
        if changingShow {
            searchSection
            if let results {
                Section {
                    if results.isEmpty { Text("Nothing found.").foregroundStyle(Theme.secondaryText) }
                    ForEach(results) { c in candidateRow(c, current: c.tmdbId == show?.id) }
                } header: { Text("Shows") }
            }
        }
        if let s = season {
            Section {
                ForEach(s.episodes) { e in
                    Button { pick = episodePick(show: s.tmdbId, e) } label: {
                        HStack(alignment: .firstTextBaseline, spacing: 12) {
                            Text("E\(e.episode)").monospacedDigit().foregroundStyle(Theme.secondaryText).frame(width: 44, alignment: .leading)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(e.title ?? "Episode \(e.episode)").foregroundStyle(.white)
                                if let d = e.airDate, !d.isEmpty { Text(d).font(.caption).foregroundStyle(Theme.secondaryText) }
                            }
                            Spacer(minLength: 0)
                            if likely(e, i) { Text("Same title").font(.caption).foregroundStyle(.green) }
                        }
                        .contentShape(Rectangle())
                    }
                    .formRow()
                    .formRowButton()
                }
            } header: { Text(s.season == 0 ? "Specials on TMDB" : "Season \(s.season) on TMDB") }
        } else if show != nil {
            Section { ProgressView() }
        }
    }

    /// TMDB's episode has this file's title.
    private func likely(_ e: MoonriseClient.TMDBSeason.Episode, _ i: MoonriseClient.MatchInfo) -> Bool {
        let norm = { (s: String) in s.lowercased().filter { $0.isLetter || $0.isNumber } }
        guard let t = e.title, !t.isEmpty else { return false }
        let a = norm(t), b = norm(i.title)
        return !a.isEmpty && (a == b || (b.count > 5 && (a.contains(b) || b.contains(a))))
    }

    // MARK: Text

    private func heading(_ i: MoonriseClient.MatchInfo) -> String {
        if i.kind == "episode" {
            return "\(i.seriesTitle ?? "") · S\(i.season ?? 0)E\(i.episode ?? 0)\(i.part.map { " · Part \($0)" } ?? "") · \(i.title)"
        }
        return i.year.map { "\(i.title) (\($0))" } ?? i.title
    }

    private func stateText(_ i: MoonriseClient.MatchInfo) -> String {
        if let n = i.note, i.state == "review" {
            return n.hasSuffix("isn't in TMDB's season list") ? "TMDB numbers this show's episodes differently." : "Not recognized for sure: \(n)."
        }
        if i.local { return "Named from its files (not matched on TMDB)." }
        switch i.state {
        case "manual": return "Matched by hand."
        case "llm": return "Matched by AI."
        default: return "Matched on TMDB."
        }
    }

    private var pickTitle: String {
        switch pick {
        case let .title(c): return "Match to \(c.year.map { "\(c.title) (\($0))" } ?? c.title)?"
        case let .episode(_, s, e, t, existing, _):
            if existing > 0 { return "S\(s)E\(e) “\(t)” is in your library" }
            return t.isEmpty ? "Keep as S\(s)E\(e)?" : "Match to S\(s)E\(e) “\(t)”?"
        case .local: return "Use the file name?"
        case nil: return ""
        }
    }

    private func pickIsKeep(_ p: Pick) -> Bool {
        if case let .episode(_, _, _, t, _, _) = p { return t.isEmpty }
        return false
    }

    private func pickMessage(_ p: Pick) -> String {
        switch p {
        case let .title(c) where isOtherTitle(c):
            return "You have this movie already: this file becomes another version of it (another cut, quality or language), and you pick which one plays."
        case .title: return switched ? "It becomes a movie: Moonrise identifies it again and fetches its artwork and details. Watch history stays."
                                     : "Moonrise identifies it again and fetches its artwork and details. Watch history stays."
        case let .episode(_, _, _, _, existing, _) where existing > 0:
            return "Is this file the next part of that episode (one episode split over several files), or another version of it (another cut, quality or language)?"
        case let .episode(_, _, _, t, _, _):
            if switched { return "It becomes an episode of that show, with its details. Watch history stays." }
            return t.isEmpty ? "It leaves Fix Match and plays as it is, under its own number." : "Moonrise files it under that episode and fetches its details. Watch history stays."
        case .local: return "It stays in the library under the name of its file or folder."
        }
    }

    /// A movie of the library other than this one (matching it adds a version to it).
    private func isOtherTitle(_ c: MoonriseClient.MatchCandidate) -> Bool {
        guard let other = c.inLibrary?.item, (c.inLibrary?.files ?? 0) > 0 else { return false }
        return other != itemId
    }

    private func fileName(_ i: MoonriseClient.MatchInfo) -> String? {
        if let fileId, let f = i.fileList?.first(where: { $0.id == fileId }) { return f.name }
        // Several files: the Files section names them all.
        if (i.fileList?.count ?? 0) > 1 { return nil }
        return i.files.first.map { ($0 as NSString).lastPathComponent }
    }

    // MARK: Files

    /// A title with several files (versions, or files merged by mistake): each can be matched on
    /// its own, split off into a title of its own, or (matched by hand) go back to automatic.
    private func filesSection(_ i: MoonriseClient.MatchInfo, _ files: [MoonriseClient.MatchFile]) -> some View {
        Section {
            ForEach(files) { f in
                Menu {
                    NavigationLink { FixMatchView(itemId: itemId, inSheet: false, fileId: f.id, done: done) } label: {
                        Label("Match This File…", systemImage: "wand.and.stars")
                    }
                    Button { Task { await split(f.id) } } label: { Label("Split Off This File", systemImage: "square.split.2x1") }
                    if f.manual {
                        Button { confirmAutomatic = AutomaticTarget(fileId: f.id) } label: { Label("Back to Automatic", systemImage: "arrow.uturn.backward") }
                    }
                } label: {
                    HStack(alignment: .firstTextBaseline, spacing: 12) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(f.name ?? "File").foregroundStyle(.white).lineLimit(2).multilineTextAlignment(.leading)
                            Text([f.duration.map { Fmt.clock($0) }, f.size.map { Fmt.bytes($0) },
                                  f.manual ? "matched by hand" : nil].compactMap { $0 }.joined(separator: " · "))
                                .font(.caption).foregroundStyle(Theme.secondaryText)
                        }
                        Spacer(minLength: 0)
                        Image(systemName: "ellipsis.circle").foregroundStyle(Theme.secondaryText)
                    }
                    .contentShape(Rectangle())
                    .formRow()
                }
                .formRowButton()
                .accessibilityIdentifier("file-\(f.id)")
            }
        } header: { Text("Files") } footer: {
            Text("These files play as versions of one title. If one is something else, split it off and match it on its own.")
        }
    }

    private func split(_ file: String) async {
        guard let c = client else { return }
        busy = true
        defer { busy = false }
        do {
            let a = try await c.splitOff(itemId, fileId: file)
            HomeStore.shared.reload()
            NotificationCenter.default.post(name: .moonriseItemRematched, object: a.item ?? itemId)
            await load()
            if let next = a.fileItem, next != itemId { splitItem = SplitItem(id: next) }
        } catch { self.error = serverErrorText(error) }
    }

    private func automatic(_ file: String?) async {
        guard let c = client else { return }
        busy = true
        defer { busy = false }
        do {
            let a = try await c.matchAutomatically(itemId, fileId: file)
            HomeStore.shared.reload()
            if file == nil {
                done(a.item ?? itemId)
                if !inSheet { dismiss() }
            } else {
                NotificationCenter.default.post(name: .moonriseItemRematched, object: a.item ?? itemId)
                notice = "That file is identified automatically again."
                await load()
            }
        } catch { self.error = serverErrorText(error) }
    }

    // MARK: Actions

    private func load() async {
        guard let c = client else { return }
        loading = true
        defer { loading = false }
        do {
            let i = try await c.matchInfo(itemId, compute: true)
            info = i
            if query.isEmpty { query = i.kind == "episode" ? (i.seriesTitle ?? "") : i.title }
            if i.kind == "movie", let y = i.year, year.isEmpty { year = String(y) }
            if i.kind == "episode", let s = i.seriesTmdbId {
                show = (s, i.seriesTitle ?? "")
                seasonNo = i.season
            }
            error = nil
        } catch {
            if !error.isCancellation { self.error = serverErrorText(error) }
        }
    }

    private func loadSeason() async {
        guard let c = client, let s = show else { return }
        do {
            let r = try await c.tmdbSeason(show: s.id, season: seasonNo)
            season = r
            if seasonNo == nil { seasonNo = r.season }
            error = nil
        } catch {
            season = nil
            if !error.isCancellation { self.error = serverErrorText(error) }
        }
    }

    private func search() async {
        guard let c = client else { return }
        searching = true
        defer { searching = false }
        do {
            results = try await c.searchMatches(kind: searchKind, query: query.trimmingCharacters(in: .whitespaces), year: Int(year))
            error = nil
        } catch { self.error = serverErrorText(error) }
    }

    private func apply(_ p: Pick, part: Bool = false) async {
        guard let c = client else { return }
        busy = true
        defer { busy = false }
        do {
            let newId: String?
            // A corrected type goes with the match (movie, or episode of a show).
            let type = switched ? (kind == "series" ? nil : kind) : nil
            switch p {
            case let .title(cand): newId = try await c.identify(itemId, tmdbId: cand.tmdbId, type: type, fileId: fileId)
            case let .episode(s, se, e, t, _, _):
                // Keeping a part of an episode keeps it that part (not another version of it).
                let keep = t.isEmpty && !switched ? info?.part : nil
                newId = try await c.identify(itemId, tmdbId: s, season: se, episode: e, type: type, part: part || keep != nil,
                                             keepPart: keep, fileId: fileId)
            case .local: newId = try await c.identify(itemId, tmdbId: nil, local: true, type: type, fileId: fileId)
            }
            HomeStore.shared.reload()
            done(newId ?? itemId)
            if !inSheet { dismiss() }
        } catch {
            self.error = serverErrorText(error)
        }
    }
}
#endif
