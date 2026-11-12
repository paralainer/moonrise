// How the Audio and Subtitles menus describe a file's tracks: TrackInfo's labels in
// Moonrise/Server/MediaModels.swift, PlayerModel.distinctDetails and detailLine.
import Foundation
import Testing
@testable import Moonrise

@MainActor
@Suite struct TrackDetailTests {
    private func subtitle(_ language: String?, _ title: String?, forced: Bool? = nil, index: Int = 3) -> SubtitleOption {
        let t = TrackInfo(kind: .subtitle, index: index, language: language, title: title, isForced: forced)
        return SubtitleOption(id: "t:\(index)", title: t.label, language: language, kind: .burnIn(index: index), trackIndex: index)
    }

    private func audio(_ language: String?, _ title: String?, codec: String? = "ac3", channels: Int? = 6,
                       display: String? = nil, index: Int = 1) -> TrackInfo {
        TrackInfo(kind: .audio, index: index, codec: codec, language: language, title: title, displayTitle: display, channels: channels)
    }

    /// Rows that would read alike get their place among those twins; the others stay as they are.
    @Test func twinsGetTheirPlace() {
        let rows: [(title: String, detail: String?)] = [("Russian", nil), ("Russian", nil), ("English", nil), ("Russian", "Forced")]
        #expect(PlayerModel.distinctDetails(rows) == ["#1", "#2", nil, "Forced"])
        let audio: [(title: String, detail: String?)] = [("Track 2", "Dolby Digital 5.1"), ("Track 3", "Dolby Digital 5.1"),
                                                         ("English", "Dolby Digital 5.1"), ("English", "Dolby Digital 5.1")]
        #expect(PlayerModel.distinctDetails(audio) == ["Dolby Digital 5.1", "Dolby Digital 5.1",
                                                       "Dolby Digital 5.1 · #1", "Dolby Digital 5.1 · #2"])
    }

    /// "Russian Forced - Netflix" under Russian says only the rest; a title that is just the
    /// language says nothing more.
    @Test func languageIsNotRepeated() {
        #expect(subtitle("rus", "Russian Forced - Netflix").detail == "Forced - Netflix")
        #expect(subtitle("eng", "English Full SDH v1").detail == "Full SDH v1")
        #expect(subtitle("eng", "English").detail == nil)
        #expect(subtitle("eng", "Englishman").detail == "Englishman", "only a whole word is the language")
        #expect(audio("eng", "English Commentary").audioDetail == "Commentary · Dolby Digital 5.1")
    }

    /// Forced and full subtitles of one language never read the same, and a forced flag isn't
    /// said twice when the title says it already.
    @Test func forcedAndFullStayApart() {
        let forced = subtitle("rus", "Russian Forced")
        let full = subtitle("rus", "Russian Full")
        #expect(forced.name == "Russian" && full.name == "Russian")
        #expect(forced.detail == "Forced")
        #expect(full.detail == "Full")
        #expect(subtitle("eng", "Forced", forced: true).detail == "Forced")
        #expect(subtitle("eng", nil, forced: true).detail == "Forced")
        #expect(PlayerModel.isForced(forced) && !PlayerModel.isForced(full), "auto-selection still finds the forced track")
    }

    /// An audio track with no language is "Track N" in the menu, its format below once; the
    /// title page's language list (audioName) keeps its old wording.
    @Test func untitledAudioIsTrackN() {
        let t = audio(nil, nil, display: "Dolby Digital - 5.1", index: 2)
        #expect(t.audioMenuName == "Track 2")
        #expect(t.audioDetail == "Dolby Digital 5.1")
        #expect(t.audioName == "Dolby Digital - 5.1")
        #expect(audio("rus", "Dub | WinMedia").audioMenuName == "Russian")
    }

    /// The timing state is added to the details, not put in their place.
    @Test func syncStateIsAppended() {
        #expect(PlayerModel.detailLine("Forced", timing: "In sync") == "Forced · In sync")
        #expect(PlayerModel.detailLine("Full · #2", timing: nil) == "Full · #2")
        #expect(PlayerModel.detailLine(nil, timing: "Synced (+1.5 s)") == "Synced (+1.5 s)")
        #expect(PlayerModel.detailLine(nil, timing: nil) == nil)
    }
}
