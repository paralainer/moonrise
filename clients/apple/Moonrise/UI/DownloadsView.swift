import SwiftUI

struct DownloadsView: View {
    private var manager: DownloadManager { DownloadManager.shared }

    @Environment(\.metrics) private var metrics

    var body: some View {
        Group {
            if metrics.regular { padBody } else { list }
        }
        .navigationTitle("Downloads")
        #if os(iOS)
        .settingsButton(onlyOnPad: true)
        #endif
    }

    /// Movies, then each show's episodes (airing order).
    private var showGroups: [(name: String, episodes: [DownloadRecord])] {
        let episodes: [DownloadRecord] = manager.records.filter { $0.state == .completed && $0.item?.isEpisode == true }
        let bySeries: [String: [DownloadRecord]] = Dictionary(grouping: episodes) { $0.seriesId ?? "" }
        var groups: [(name: String, episodes: [DownloadRecord])] = []
        for (_, recs) in bySeries {
            let sorted = recs.sorted { a, b in
                let ka = MediaItem.order(a.item)
                let kb = MediaItem.order(b.item)
                return ka.lexicographicallyPrecedes(kb)
            }
            groups.append((name: recs.first?.item?.seriesName ?? "Episodes", episodes: sorted))
        }
        return groups.sorted { $0.name < $1.name }
    }

    /// iPad: the same sections as the list, as a grid of cards at the page margins.
    private var padBody: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 26) {
                if manager.records.isEmpty {
                    EmptyStateView(icon: "arrow.down.circle", title: "No Downloads",
                                   message: "Download movies and episodes to watch offline. Choose Original or a transcoded quality to save space.",
                                   artwork: "EmptyDownloads")
                }
                let inProgress = manager.records.filter { $0.state != .completed }
                padSection("In Progress", inProgress)
                let done = manager.records.filter { $0.state == .completed }
                let movies = done.filter { $0.item?.isEpisode != true }
                padSection(done.count == movies.count ? "Ready to Watch" : "Movies", movies)
                ForEach(showGroups, id: \.name) { show in padSection(show.name, show.episodes) }
            }
            .padding(.vertical, 12)
        }
        .background(Theme.backgroundGradient.ignoresSafeArea())
    }

    @ViewBuilder private func padSection(_ title: String, _ records: [DownloadRecord]) -> some View {
        if !records.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                SectionHeader(title: title)
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 340), spacing: 14, alignment: .top)], alignment: .leading, spacing: 14) {
                    ForEach(records) { r in
                        row(r)
                            .padding(12)
                            .background(Theme.surface, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                    }
                }
                .padding(.horizontal, metrics.hPadding)
            }
        }
    }

    private var list: some View {
        List {
            if manager.records.isEmpty {
                EmptyStateView(icon: "arrow.down.circle", title: "No Downloads",
                               message: "Download movies and episodes to watch offline. Choose Original or a transcoded quality to save space.",
                               artwork: "EmptyDownloads")
                    .listRowBackground(Color.clear)
            }
            let inProgress = manager.records.filter { $0.state != .completed }
            if !inProgress.isEmpty {
                Section("In Progress") {
                    ForEach(inProgress) { row($0) }
                }
            }
            let done = manager.records.filter { $0.state == .completed }
            let movies = done.filter { $0.item?.isEpisode != true }
            if !movies.isEmpty {
                Section(done.count == movies.count ? "Ready to Watch" : "Movies") {
                    ForEach(movies) { row($0) }
                }
            }
            // Episodes grouped by show, in airing order.
            let shows = Dictionary(grouping: done.filter { $0.item?.isEpisode == true }) { $0.seriesId ?? "" }
                .map { (name: $0.value.first?.item?.seriesName ?? "Episodes", episodes: $0.value) }
                .sorted { $0.name < $1.name }
            ForEach(shows, id: \.name) { show in
                Section(show.name) {
                    ForEach(show.episodes.sorted { MediaItem.order($0.item).lexicographicallyPrecedes(MediaItem.order($1.item)) }) { row($0) }
                }
            }
        }
    }

    private func row(_ r: DownloadRecord) -> some View {
        Button {
            // The title's page, playing it. In-progress segmented downloads play too (as
            // far as they are downloaded, or on demand).
            if r.isPlayable, let route = r.pageRoute() { AppNavigation.shared.downloadsPath.append(route) }
        } label: {
            HStack(spacing: 12) {
                RemoteImage(url: r.posterURL)
                    .frame(width: 54, height: 80)
                    .clipShape(RoundedRectangle(cornerRadius: 6))
                    .overlay {
                        if r.posterURL == nil { Image(systemName: "film").foregroundStyle(Theme.secondaryText) }
                    }
                VStack(alignment: .leading, spacing: 4) {
                    Text(r.title).font(.headline).foregroundStyle(.white).lineLimit(2)
                    if let s = r.subtitle { Text(s).font(.caption).foregroundStyle(Theme.secondaryText) }
                    statusLine(r)
                }
                Spacer()
                controls(r)
            }
        }
        .plainButton()
        #if !os(tvOS)
        .swipeActions {
            Button(role: .destructive) { manager.delete(r.id) } label: { Label("Delete", systemImage: "trash") }
                .tint(.red)
        }
        #endif
        .contextMenu {
            Button(role: .destructive) { manager.delete(r.id) } label: { Label("Delete", systemImage: "trash") }
        }
    }

    @ViewBuilder private func statusLine(_ r: DownloadRecord) -> some View {
        switch r.state {
        case .completed:
            HStack(spacing: 6) {
                Badge(text: r.qualityLabel)
                if let size = r.localURL.flatMap(Self.size) { Text(Fmt.bytes(size)) }
                if !r.subtitles.isEmpty { Badge(text: "CC") }
            }
            .font(.caption).foregroundStyle(Theme.secondaryText)
        case .failed:
            Text(r.error ?? "Failed").font(.caption).foregroundStyle(.red).lineLimit(2)
        default:
            VStack(alignment: .leading, spacing: 4) {
                ProgressView(value: r.progress).tint(Theme.accent)
                Text(r.state == .paused ? "Paused · \(Int(r.progress * 100))%" :
                     r.state == .queued ? "Waiting…" :
                     r.isSegmented && r.error != nil ? "\(r.error!) · \(Int(r.progress * 100))%" :
                     r.isSegmented && r.isPlayable ? "\(Int(r.progress * 100))% · \(Fmt.bytes(r.bytesWritten)) · Tap to play now" :
                     r.preparing == true ? "Preparing on the server · \(Int(r.progress * 100))%" :
                     r.bytesExpected > 0 ? "\(Fmt.bytes(r.bytesWritten)) of \(Fmt.bytes(r.bytesExpected))" : "\(Int(r.progress * 100))%")
                    .font(.caption).foregroundStyle(Theme.secondaryText)
            }
        }
    }

    @ViewBuilder private func controls(_ r: DownloadRecord) -> some View {
        switch r.state {
        case .downloading, .queued:
            Button { manager.pause(r.id) } label: { Image(systemName: "pause.circle.fill").font(.title2) }
                .plainButton()
        case .paused, .failed:
            Button { manager.resume(r.id) } label: { Image(systemName: "arrow.clockwise.circle.fill").font(.title2) }
                .plainButton()
        case .completed:
            Image(systemName: "play.circle.fill").font(.title2).foregroundStyle(Theme.accent)
        }
        if r.state != .completed, r.isPlayable {
            Image(systemName: "play.circle").font(.title2).foregroundStyle(Theme.accent)
        }
    }

    private static func size(_ url: URL) -> Int64? {
        let fm = FileManager.default
        var isDir: ObjCBool = false
        guard fm.fileExists(atPath: url.path, isDirectory: &isDir) else { return nil }
        if !isDir.boolValue { return (try? fm.attributesOfItem(atPath: url.path)[.size] as? Int64) ?? nil }
        // .movpkg is a directory
        var total: Int64 = 0
        if let e = fm.enumerator(at: url, includingPropertiesForKeys: [.fileSizeKey]) {
            for case let f as URL in e { total += Int64((try? f.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0) }
        }
        return total
    }
}
