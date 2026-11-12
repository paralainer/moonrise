import SwiftUI

// Versions on title pages: a title with several files (cuts, qualities, dubs) plays the one the
// user chose (`VersionChoices`). iPhone, iPad and Mac show it under Play as a menu line
// ("Version: 4K HDR10 · Director's Cut"); Apple TV as a round button in the action row.

/// The full item for a title page's focus when it has several versions but came from a list
/// (episode rows carry only how many): loaded once per id.
@MainActor
@Observable
final class VersionDetails {
    private(set) var items: [String: MediaItem] = [:]
    private var loading: Set<String> = []

    /// `item` with its versions (loaded now when a list item says it has several).
    func full(_ item: MediaItem) -> MediaItem {
        if (item.versions?.count ?? 0) > 1 { return item }
        // (One version now, e.g. after a Fix Match: the files loaded before don't count.)
        guard (item.versionCount ?? 0) > 1 else { return item }
        return items[item.id] ?? item
    }

    func load(_ item: MediaItem, client: (any MediaServerClient)?) async {
        // Full already (the page's first item): kept for when the season listing replaces it.
        if item.hasVersions { items[item.id] = item; return }
        guard (item.versionCount ?? 0) > 1, items[item.id] == nil,
              !loading.contains(item.id), let client else { return }
        loading.insert(item.id)
        defer { loading.remove(item.id) }
        if let full = try? await client.item(item.id) { items[item.id] = full }
    }

    /// A Fix Match changed titles' files: load them again.
    func forgetAll() { items = [:] }

    /// What `.task(id:)` keys the load on: the title, and how many versions it says it has.
    static func key(_ item: MediaItem) -> String { "\(item.id)|\(item.versions?.count ?? 0)|\(item.versionCount ?? 0)" }
}

extension MediaItem {
    /// Has several versions to choose from.
    var hasVersions: Bool { (versions?.count ?? 0) > 1 }

    /// The item as its chosen version is (its tracks and length: the badges and the line say them).
    @MainActor var asChosenVersion: MediaItem {
        guard hasVersions, let v = VersionChoices.shared.version(for: self) else { return self }
        var i = self
        i.tracks = v.tracks ?? i.tracks
        if let r = v.runtime { i.runtime = r }
        return i
    }
}

/// What a title page plays: the version and where it starts.
struct VersionPlay {
    let version: MediaVersion?
    /// The version's own resume point, else the title's (one version).
    let resume: Double?
    let progress: Double?

    @MainActor init(_ item: MediaItem) {
        guard item.hasVersions, let v = VersionChoices.shared.version(for: item) else {
            version = nil
            resume = item.resumeSeconds
            progress = item.progress
            return
        }
        version = v
        let p = (v.position ?? 0) > 0 ? v.position : nil
        resume = p
        progress = p.flatMap { p in (v.runtime ?? item.runtime).map { min(1, p / max($0, 1)) } }
    }
}

/// The version picker's rows: each version's label, its details below, the one playing checked
/// (Toggle rows: the system's checkmark column, as the player's track menus).
struct VersionPickerRows: View {
    let item: MediaItem
    let source: Source?
    /// (MediaItem compares by id and watch state only: its versions changing must update the view.)
    private let versions: [MediaVersion]

    init(item: MediaItem, source: Source?) {
        self.item = item
        self.source = source
        versions = item.versions ?? []
    }

    var body: some View {
        let current = VersionChoices.shared.version(for: item)?.id
        ForEach(versions) { v in
            Toggle(isOn: Binding(get: { v.id == current }, set: { if $0 { choose(v.id) } })) {
                Text(PlayerModel.versionTitle(v))
                if let d = PlayerModel.versionDetails(v) { Text(d) }
            }
            // A damaged file can't play: shown, greyed out (the server plays a healthy one).
            .disabled(v.isBroken && v.id != current)
        }
    }

    private func choose(_ id: String) {
        VersionChoices.shared.set(id, for: item.id)
        guard let source, let c = MediaServers.client(for: source) as? MoonriseClient else { return }
        Task { await c.chooseVersion(itemId: item.id, fileId: id) }
    }
}

#if !os(tvOS)
/// iPhone, iPad, Mac: "Version: 4K HDR10 · Director's Cut" under Play, a menu of the versions.
/// Admins also get Fix Match for the versions (split one off, match one alone).
struct VersionLine: View {
    let item: MediaItem
    let source: Source?
    /// (MediaItem compares by id and watch state only: its versions changing must update the view.)
    private let versions: [MediaVersion]

    init(item: MediaItem, source: Source?) {
        self.item = item
        self.source = source
        versions = item.versions ?? []
    }

    var body: some View {
        if versions.count > 1, let v = VersionChoices.shared.version(for: item) {
            Menu {
                Section("Version") { VersionPickerRows(item: item, source: source) }
                if AdminState.shared.isAdmin {
                    Section {
                        Button { FixMatchPresenter.shared.present(item.id) } label: {
                            Label("Fix Match Versions…", systemImage: "wand.and.stars")
                        }
                    }
                }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "square.stack")
                    Text("Version: ").foregroundStyle(Theme.secondaryText) + Text(PlayerModel.versionTitle(v)).foregroundStyle(.white)
                    Image(systemName: "chevron.up.chevron.down").font(.caption2.weight(.semibold))
                }
                .font(.footnote)
                .foregroundStyle(Theme.secondaryText)
                .lineLimit(1)
                .contentShape(Rectangle())
            }
            .drawnMenu()
            .accessibilityLabel("Version: \(PlayerModel.versionTitle(v))")
            .accessibilityIdentifier("versionMenu")
        }
    }
}
#endif

/// In place of Play on a title page when no file of the title can play: what's
/// wrong in a line, and Details for the server's own words. No Play and no Download.
struct DamagedTitleNote: View {
    let problem: PlaybackProblem
    @State private var showDetail = false

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.title3)
                .foregroundStyle(.yellow)
            VStack(alignment: .leading, spacing: 2) {
                Text(problem.title).font(.headline)
                Text(problem.message)
                    .font(.subheadline)
                    .foregroundStyle(Theme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if problem.detail != nil {
                #if os(tvOS)
                Button("Details") { showDetail = true }.tvGlass()
                #else
                Button("Details") { showDetail = true }
                    .buttonStyle(.glass)
                    .controlSize(.small)
                #endif
            }
        }
        .foregroundStyle(.white)
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .background(.white.opacity(0.1), in: RoundedRectangle(cornerRadius: 16))
        .alert(problem.title, isPresented: $showDetail) {
            Button("OK") {}
        } message: {
            Text(problem.detail ?? "")
        }
    }
}
