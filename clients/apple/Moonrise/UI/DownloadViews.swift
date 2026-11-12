import SwiftUI

/// The downloaded mark on a card: a filled arrow when the title is on the device, a ring while
/// it downloads. Shows and seasons show how many of their episodes are downloaded.
struct DownloadMark: View {
    let item: MediaItem
    private var manager: DownloadManager { DownloadManager.shared }

    private var size: CGFloat {
        #if os(tvOS)
        return 30
        #else
        return 18
        #endif
    }

    var body: some View {
        if item.isSeries || item.isSeason {
            let n = item.isSeries ? manager.seriesCounts[item.id] ?? 0 : manager.seasonCounts[item.id] ?? 0
            if n > 0 {
                HStack(spacing: 3) {
                    Image(systemName: "arrow.down.circle.fill")
                    Text("\(n)")
                }
                .font(.system(size: size * 0.7, weight: .bold))
                .padding(.horizontal, 6).padding(.vertical, 3)
                .background(.black.opacity(0.6), in: Capsule())
                .foregroundStyle(.white)
                .padding(5)
                .accessibilityLabel("\(n) downloaded")
            }
        } else if let b = manager.badges[item.id] {
            DownloadStateIcon(badge: b, size: size)
                .padding(5)
        }
    }
}

/// "Downloaded" in a title's info line (title pages, Home's banner, the TV showcase).
struct DownloadedTag: View {
    let item: MediaItem
    private var manager: DownloadManager { DownloadManager.shared }

    var body: some View {
        let text: String? = if item.isSeries {
            manager.seriesCounts[item.id].map { $0 == 1 ? "1 episode downloaded" : "\($0) episodes downloaded" }
        } else {
            switch manager.badges[item.id] {
            case .downloaded?: "Downloaded"
            case let .downloading(p)?: "Downloading \(p) %"
            case .queued?, .paused?: "Download waiting"
            case .failed?, nil: nil
            }
        }
        if let text {
            Label(text, systemImage: "arrow.down.circle.fill").labelStyle(.titleAndIcon)
        }
    }
}

/// One download's state as a small round icon.
struct DownloadStateIcon: View {
    let badge: DownloadManager.Badge
    var size: CGFloat = 18

    var body: some View {
        Group {
            switch badge {
            case .downloaded:
                Image(systemName: "arrow.down.circle.fill")
                    .font(.system(size: size))
                    .symbolRenderingMode(.palette)
                    .foregroundStyle(.black, .white)
                    .accessibilityLabel("Downloaded")
            case let .downloading(p):
                ZStack {
                    Circle().fill(.black.opacity(0.6))
                    Circle().stroke(Color.white.opacity(0.25), lineWidth: size * 0.14)
                    Circle().trim(from: 0, to: Double(max(p, 3)) / 100)
                        .stroke(Color.white, style: StrokeStyle(lineWidth: size * 0.14, lineCap: .round))
                        .rotationEffect(.degrees(-90))
                    Image(systemName: "arrow.down").font(.system(size: size * 0.45, weight: .bold)).foregroundStyle(.white)
                }
                .frame(width: size, height: size)
                .accessibilityLabel("Downloading, \(p) percent")
            case .queued, .paused:
                Image(systemName: badge == .paused ? "pause.circle.fill" : "clock.fill")
                    .font(.system(size: size))
                    .symbolRenderingMode(.palette)
                    .foregroundStyle(.black, .white.opacity(0.8))
                    .accessibilityLabel(badge == .paused ? "Download paused" : "Download waiting")
            case .failed:
                Image(systemName: "exclamationmark.circle.fill")
                    .font(.system(size: size))
                    .symbolRenderingMode(.palette)
                    .foregroundStyle(.white, .red)
                    .accessibilityLabel("Download failed")
            }
        }
        .shadow(color: .black.opacity(0.5), radius: 3)
    }
}

/// "Downloaded · 480p" / "Downloading 34 %" under a title page's Play row, at the text's left edge.
struct DownloadStatusLine: View {
    let item: MediaItem
    private var manager: DownloadManager { DownloadManager.shared }

    var body: some View {
        if let r = manager.record(forItem: item.id) {
            let text: String = switch r.state {
            case .completed: "Downloaded · \(r.downloadLabel)"
            case .downloading: "Downloading \(Int(r.progress * 100)) %"
            case .queued: "Download waiting"
            case .paused: "Download paused at \(Int(r.progress * 100)) %"
            case .failed: "Download failed: \(r.error ?? "")"
            }
            Label(text, systemImage: r.state == .failed ? "exclamationmark.circle" : "arrow.down.circle")
                .font(.footnote)
                .foregroundStyle(Theme.secondaryText)
                .lineLimit(2)
                .multilineTextAlignment(.leading)
        }
    }
}

// MARK: - Offline

/// A server can't be reached and there are downloads: say so, and show them.
struct OfflineBanner: View {
    @Environment(\.metrics) private var metrics
    var retry: (() -> Void)?
    @State private var retrying = false

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "wifi.slash")
            Text("Offline: showing downloads").font(.subheadline.weight(.semibold))
            Spacer(minLength: 8)
            if let retry {
                Button {
                    retrying = true
                    Task {
                        await ConnectionRouter.shared.refresh()
                        retry()
                        retrying = false
                    }
                } label: {
                    if retrying { ProgressView() } else { Text("Try Again").font(.subheadline) }
                }
                .plainButton()
                .disabled(retrying)
            }
        }
        .foregroundStyle(.white)
        .padding(.horizontal, 18).padding(.vertical, 12)
        .glassEffect(.regular, in: .capsule)
        .padding(.horizontal, metrics.hPadding)
    }
}

/// Downloads as shelves (Home while offline): movies, then each show's episodes. A card opens
/// the title's page, playing it.
struct OfflineDownloadShelves: View {
    @Environment(\.metrics) private var metrics
    private var manager: DownloadManager { DownloadManager.shared }

    var body: some View {
        let playable = manager.records.filter { $0.isPlayable }
        let movies = playable.filter { $0.item?.isEpisode != true }
        let shows = Dictionary(grouping: playable.filter { $0.item?.isEpisode == true }) { $0.seriesId ?? "" }
            .sorted { ($0.value.first?.item?.seriesName ?? "") < ($1.value.first?.item?.seriesName ?? "") }
        VStack(alignment: .leading, spacing: 28) {
            if !movies.isEmpty {
                Shelf(title: "Downloaded Movies") {
                    ForEach(movies) { r in card(r, landscape: false) }
                }
            }
            ForEach(shows, id: \.key) { _, episodes in
                let sorted = episodes.sorted { MediaItem.order($0.item).lexicographicallyPrecedes(MediaItem.order($1.item)) }
                Shelf(title: sorted.first?.item?.seriesName ?? "Episodes") {
                    ForEach(sorted) { r in card(r, landscape: true) }
                }
            }
            if playable.isEmpty {
                Text("Nothing is downloaded yet.").font(.subheadline).foregroundStyle(Theme.secondaryText)
                    .padding(.horizontal, metrics.hPadding)
            }
        }
    }

    private func card(_ r: DownloadRecord, landscape: Bool) -> some View {
        Button {
            AppNavigation.shared.pendingRoute = r.pageRoute()
        } label: {
            DownloadCard(record: r, landscape: landscape)
        }
        .cardButtonStyle()
        .contextMenu {
            Button { AppNavigation.shared.pendingRoute = r.pageRoute(startAt: 0) } label: {
                Label("Play from Beginning", systemImage: "arrow.counterclockwise")
            }
            Button(role: .destructive) { manager.delete(r.id) } label: { Label("Delete Download", systemImage: "trash") }
        }
    }
}

/// A download's card, from what it saved (works with no server).
struct DownloadCard: View {
    @Environment(\.metrics) private var metrics
    let record: DownloadRecord
    let landscape: Bool

    private var width: CGFloat { landscape ? metrics.landscapeWidth : metrics.posterWidth }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ZStack(alignment: .bottom) {
                RemoteImage(url: landscape ? (record.backdropURL ?? record.posterURL) : record.posterURL)
                    .frame(width: width, height: landscape ? width * 9 / 16 : width * 1.5)
                    .overlay {
                        if record.posterURL == nil && record.backdropURL == nil {
                            Image(systemName: "film").font(.title).foregroundStyle(Theme.secondaryText)
                        }
                    }
                if let p = ProgressStore.position(for: record.progressKey), let d = record.runtimeTicks.map({ Double($0) / 10_000_000 }), d > 0 {
                    ProgressBar(value: p / d).padding(6)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: landscape ? 16 : 12, style: .continuous))
            .overlay(alignment: .topLeading) {
                DownloadStateIcon(badge: record.state == .completed ? .downloaded : .downloading(Int(record.progress * 100))).padding(5)
            }
            #if os(tvOS)
            .lockupArtwork()
            #endif
            Text(landscape ? (record.item?.seriesName ?? record.title) : record.title)
                .font(.caption.weight(.medium)).lineLimit(1).foregroundStyle(.white)
            Text(landscape ? ([record.item?.episodeLabel, record.item?.name].compactMap { $0 }.joined(separator: " · ")) : (record.year.map(String.init) ?? " "))
                .font(.caption2).lineLimit(1).foregroundStyle(Theme.secondaryText)
        }
        .frame(width: width)
    }
}
