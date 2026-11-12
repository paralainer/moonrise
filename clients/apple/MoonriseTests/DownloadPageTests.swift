// Downloads open the normal title page: Moonrise/Downloads/OfflineTitlePage.swift (the
// route a download opens, the page made from what downloads saved when there is no server), and
// the old downloads-only player and episode screens are gone.
import Foundation
import Testing
@testable import Moonrise

@Suite struct DownloadPageTests {
    private static let source = UUID()

    private func record(_ item: MediaItem?, itemId: String? = nil, state: DownloadRecord.State = .completed,
                        series: MediaItem? = nil) -> DownloadRecord {
        var r = DownloadRecord(kind: .segments, state: state, title: item?.displayTitle ?? "A download",
                               sourceId: Self.source, itemId: itemId ?? item?.id, qualityLabel: "Original", fileExtension: "m3u8")
        r.item = item
        r.series = series
        return r
    }

    private func episode(_ id: String, season: Int, number: Int, seasonId: String? = nil) -> MediaItem {
        var e = Fixture.episode(id, series: "s1", season: season, number: number)
        e.seasonId = seasonId
        return e
    }

    // MARK: The route

    /// A download opens its title's page, which plays it as it opens (resuming), not a player of
    /// its own.
    @Test func aDownloadOpensItsTitlePagePlaying() {
        let film = Fixture.movie("m1")
        guard case let .item(item, zoom, autoplay)? = record(film).pageRoute() else {
            Issue.record("not a title page")
            return
        }
        #expect(item.id == "m1")
        #expect(zoom == nil)
        #expect(autoplay == Autoplay(startAt: nil))
    }

    /// An episode's download opens its show's page on that episode (the page opens an episode's
    /// show with the episode selected); Play from Beginning asks for 0.
    @Test func anEpisodesDownloadOpensOnTheEpisodeFromTheStart() {
        let ep = episode("e2", season: 1, number: 2)
        guard case let .item(item, _, autoplay)? = record(ep).pageRoute(startAt: 0) else {
            Issue.record("not a title page")
            return
        }
        #expect(item.id == "e2")
        #expect(item.isEpisode)
        #expect(autoplay?.startAt == 0)
        #expect(ItemDetailView.showStub(for: item).id == "s1")
    }

    /// A record saved before titles were kept with downloads still opens a page, from its own
    /// fields; one without even an id opens none.
    @Test func anOldRecordOpensAPageFromItsOwnFields() {
        var r = record(nil, itemId: "m9")
        r.year = 1999
        let item = r.pageItem
        #expect(item?.id == "m9")
        #expect(item?.displayTitle == "A download")
        #expect(item?.productionYear == 1999)
        #expect(record(nil).pageRoute() == nil)
    }

    // MARK: The page with no server

    /// A show's page from its downloads: the show as saved, seasons made from the downloaded
    /// episodes in order, each season's episodes in airing order; ones that don't play from the
    /// device yet and other shows' are left out.
    @Test func aShowsPageIsMadeFromItsDownloadedEpisodes() throws {
        var show = Fixture.series("s1")
        show.overview = "The show."
        let records = [
            record(episode("e3", season: 2, number: 1, seasonId: "sea2"), series: show),
            record(episode("e2", season: 1, number: 2, seasonId: "sea1"), series: show),
            record(episode("e1", season: 1, number: 1, seasonId: "sea1"), series: show),
            record(episode("e4", season: 2, number: 2, seasonId: "sea2"), state: .queued, series: show),
            record(Fixture.episode("x1", series: "other")),
            record(Fixture.movie("m1")),
        ]
        let page = try #require(OfflineTitlePage.build(itemId: "s1", records: records))
        #expect(page.item.id == "s1")
        #expect(page.item.overview == "The show.")
        #expect(page.seasons.map(\.id) == ["sea1", "sea2"])
        #expect(page.seasons.map(\.displayTitle) == ["Season 1", "Season 2"])
        #expect(page.seasons.allSatisfy { $0.isSeason })
        #expect(page.episodes(season: page.seasons[0]).map(\.id) == ["e1", "e2"])
        #expect(page.episodes(season: page.seasons[1]).map(\.id) == ["e3"], "e4 doesn't play from the device yet")
        #expect(page.season(of: records[0].item)?.id == "sea2")
    }

    /// Episodes saved without the show or season ids still make a page: the show from what the
    /// episode knows of it, seasons by number (0: Specials).
    @Test func aShowsPageWithoutSavedIds() throws {
        var ep = episode("e1", season: 0, number: 1)
        ep.seriesImages = ImageSet(poster: ImageRef(id: "p"))
        let page = try #require(OfflineTitlePage.build(itemId: "s1", records: [record(ep), record(episode("e2", season: 3, number: 1))]))
        #expect(page.item.isSeries)
        #expect(page.item.displayTitle == "Show s1")
        #expect(page.item.images.poster?.id == "p")
        #expect(page.seasons.map(\.id) == ["season:0", "season:3"])
        #expect(page.seasons.map(\.displayTitle) == ["Specials", "Season 3"])
    }

    /// A film's page is the film as its download saved it; nothing downloaded (or nothing that
    /// plays yet): no page, so the page shows the server's error as before.
    @Test func aFilmsPageAndNone() throws {
        var film = Fixture.movie("m1")
        film.overview = "What happens."
        let page = try #require(OfflineTitlePage.build(itemId: "m1", records: [record(film)]))
        #expect(page.item.overview == "What happens.")
        #expect(page.seasons.isEmpty)
        #expect(OfflineTitlePage.build(itemId: "m2", records: [record(film)]) == nil)
        #expect(OfflineTitlePage.build(itemId: "m1", records: [record(film, state: .paused)]) == nil)
    }

    // MARK: The old screens are gone

    /// No downloads-only player, portrait episode panel or offline episode grid is left, and
    /// nothing plays a download other than through its title's page.
    @Test func theOldDownloadScreensAreGone() throws {
        let sources = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Moonrise")
        let files = try #require(FileManager.default.enumerator(at: sources, includingPropertiesForKeys: nil))
        var text = ""
        var count = 0
        for case let url as URL in files where url.pathExtension == "swift" {
            text += try String(contentsOf: url, encoding: .utf8)
            count += 1
        }
        #expect(count > 50, "the app's sources were found")
        for gone in ["PlayerInfoPanel", "OfflineShowView", "offlineShow", "target: .download(", "case download(DownloadRecord)"] {
            #expect(!text.contains(gone), "\(gone) is still there")
        }
    }
}
