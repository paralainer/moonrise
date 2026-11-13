// Which audio and subtitle tracks a title starts with (Moonrise/Player/TrackRules.swift, used by
// PlayerModel): the preferred audio language, forced and SDH subtitles, and auto-selection.
import Foundation
import Testing
@testable import Moonrise

@Suite struct TrackRulesTests {
    private func audio(_ id: Int, _ language: String?, _ name: String = "", isDefault: Bool = false) -> TrackRules.AudioCandidate {
        TrackRules.AudioCandidate(id: id, language: language, isDefault: isDefault, name: name)
    }

    private func text(_ id: String, _ title: String, _ language: String?) -> SubtitleOption {
        SubtitleOption(id: id, title: title, language: language, kind: .engine(id.hashValue & 0xffff))
    }

    private func bitmap(_ id: String, _ title: String, _ language: String?) -> SubtitleOption {
        SubtitleOption(id: id, title: title, language: language, kind: .pictures(index: id.hashValue & 0xffff))
    }

    private func pick(_ options: [SubtitleOption], on: Bool = true, _ language: String = "en", sdh: Bool = false,
                      audio: String? = nil) -> String? {
        TrackRules.autoSubtitle(options, autoSelect: on, language: language, preferSDH: sdh, audioLanguage: audio)?.id
    }

    /// The preferred language's default track, else its first that isn't a commentary or audio
    /// description, else its first; nil (the title's default) when none or no preference.
    @Test func preferredAudio() {
        let tracks = [audio(1, "eng", isDefault: true), audio(2, "rus", "Director's Commentary"),
                      audio(3, "rus", "Dub"), audio(4, "rus", "Studio B", isDefault: true)]
        #expect(TrackRules.preferredAudio(tracks, preferred: "ru") == 4, "the language's default")
        #expect(TrackRules.preferredAudio(Array(tracks.prefix(3)), preferred: "ru") == 3, "not the commentary")
        #expect(TrackRules.preferredAudio([audio(1, "eng"), audio(2, "ger", "Audio Description")], preferred: "de") == 2,
                "only a commentary or description: still that language")
        #expect(TrackRules.preferredAudio([audio(1, "eng"), audio(2, "fre", "Narrated")], preferred: "fra") == 2,
                "codes in any form (fre, fra, fr)")
        #expect(TrackRules.preferredAudio(tracks, preferred: "") == nil, "no preference")
        #expect(TrackRules.preferredAudio(tracks, preferred: "ja") == nil, "language not there")
        #expect(TrackRules.preferredAudio([audio(1, nil), audio(2, "und")], preferred: "en") == nil, "unknown languages never match")
    }

    @Test func forcedAndSDH() {
        #expect(TrackRules.isForced(text("a", "English Forced", "eng")))
        #expect(TrackRules.isForced(text("a", "FORCED", nil)))
        #expect(!TrackRules.isForced(text("a", "English Full", "eng")))
        #expect(TrackRules.isSDH(text("a", "English SDH", "eng")))
        #expect(TrackRules.isSDH(text("a", "English (CC)", "eng")))
        #expect(TrackRules.isSDH(text("a", "For the hearing impaired", "eng")))
        #expect(!TrackRules.isSDH(text("a", "English Accent Notes", "eng")), "cc only as a word")
        #expect(!TrackRules.isSDH(text("a", "English", "eng")))
    }

    /// A file's language from its name when the track has none ("Film.en.srt").
    @Test func languageFromAFileName() {
        #expect(TrackRules.subtitleLanguage(text("a", "Film.en.srt", nil)) == "en")
        #expect(TrackRules.subtitleLanguage(text("a", "Film.2010.ger.forced.srt", nil)) == "de")
        #expect(TrackRules.subtitleLanguage(text("a", "English", "eng")) == "en", "the track's own language first")
        #expect(TrackRules.subtitleLanguage(text("a", "Film.srt", nil)) == nil)
    }

    /// Subtitles on: the preferred language's, the kind picked last first, text before pictures,
    /// never a forced track.
    @Test func autoSelectWhenOn() {
        let forced = text("forced", "English Forced", "eng")
        let plain = text("plain", "English", "eng")
        let sdh = text("sdh", "English SDH", "eng")
        let pgs = bitmap("pgs", "English", "eng")
        let other = text("other", "Russian", "rus")
        #expect(pick([other, forced, plain, sdh]) == "plain")
        #expect(pick([other, forced, sdh, plain], sdh: true) == "sdh")
        #expect(pick([other, sdh], sdh: false) == "sdh", "the other kind when that's all there is")
        #expect(pick([pgs, plain]) == "plain", "text before a picture track")
        #expect(pick([pgs, other]) == "pgs", "a picture track when there's no text one")
        #expect(pick([forced, other], audio: "ru") == nil, "never a forced track as the full subtitles")
        #expect(pick([text("file", "Film.en.srt", nil)], "eng") == "file", "by file name; the setting in any form")
    }

    /// Off, or none in the preferred language: forced subtitles for foreign dialogue, in the
    /// audio's language; any forced track when the audio's language isn't known; a picture one
    /// too (Blu-ray forced subtitles are pictures).
    @Test func forcedSubtitlesAutoSelect() {
        let forcedEn = text("forced-en", "English Forced", "eng")
        let forcedRu = text("forced-ru", "Russian Forced", "rus")
        let plainEn = text("plain-en", "English", "eng")
        #expect(pick([plainEn, forcedRu, forcedEn], on: false, audio: "en") == "forced-en", "the audio's language")
        #expect(pick([plainEn, forcedRu, forcedEn], on: false, audio: "ru") == "forced-ru")
        #expect(pick([plainEn, forcedEn], on: false, audio: "ja") == nil, "no forced track in the audio's language: off")
        #expect(pick([plainEn, forcedRu, forcedEn], on: false, audio: nil) == "forced-ru", "audio language unknown: the first forced")
        #expect(pick([bitmap("pgs-forced", "English Forced", "eng")], on: false, audio: "en") == "pgs-forced", "a forced picture track")
        #expect(pick([plainEn], on: false, audio: "en") == nil, "off and nothing forced: nothing")
        #expect(pick([plainEn, forcedRu], on: true, "de", audio: "ru") == "forced-ru", "on, but not in the preferred language")
    }

    // MARK: - Track memory

    private typealias Pick = TrackRules.Pick

    private func memory(same: Bool, audio: Pick? = nil, subtitle: Pick? = nil, off: Bool = false) -> TrackRules.Memory {
        TrackRules.Memory(sameFile: same, audio: audio, subtitle: subtitle, subtitleOff: off)
    }

    private func server(_ index: Int, _ title: String, _ language: String?) -> SubtitleOption {
        SubtitleOption(id: "t:\(index)", title: title, language: language, kind: .engine(index), trackIndex: index)
    }

    private func remembered(_ options: [SubtitleOption], _ m: TrackRules.Memory?) -> String {
        switch TrackRules.rememberedSubtitle(options, memory: m) {
        case let .track(o): o.id
        case .off: "off"
        case .usualOff: "usual-off"
        case .usual: "usual"
        }
    }

    /// Resuming a title plays the audio track picked in it, also the second of one language and
    /// one without a language; the language preference no longer decides (the bug).
    @Test func resumeKeepsTheAudioTrack() {
        let tracks = [audio(1, "eng", isDefault: true), audio(2, "rus", "Dub"), audio(3, "rus", "Studio B"), audio(4, nil, "Track 4")]
        #expect(TrackRules.rememberedAudio(tracks, memory: memory(same: true, audio: Pick(index: 3, language: "ru", title: "Studio B"))) == 3)
        #expect(TrackRules.rememberedAudio(tracks, memory: memory(same: true, audio: Pick(index: 4))) == 4, "no language")
        #expect(TrackRules.rememberedAudio(tracks, memory: memory(same: true, audio: Pick(index: 2, language: "de"))) == nil,
                "the index now has another language (a replaced file), and no German track")
        #expect(TrackRules.rememberedAudio(tracks, memory: memory(same: true, audio: Pick(index: 9, language: "ru", title: "Studio B"))) == 3,
                "the index gone: found by its title")
        #expect(TrackRules.rememberedAudio(tracks, memory: nil) == nil)
        #expect(TrackRules.rememberedAudio(tracks, memory: memory(same: true, subtitle: Pick(index: 1))) == nil, "no audio said")
    }

    /// The next episode (another file): the same language and title, never an index; a
    /// commentary only when that was picked.
    @Test func nextEpisodeAudio() {
        let next = [audio(5, "eng", isDefault: true), audio(6, "rus", "Commentary"), audio(7, "rus", "Studio B"), audio(8, "rus", "Dub")]
        #expect(TrackRules.rememberedAudio(next, memory: memory(same: false, audio: Pick(index: 5, language: "ru", title: "Dub"))) == 8,
                "by title, not the index")
        #expect(TrackRules.rememberedAudio(next, memory: memory(same: false, audio: Pick(language: "rus", title: "Other studio"))) == 7,
                "the language, not its commentary")
        #expect(TrackRules.rememberedAudio(next, memory: memory(same: false, audio: Pick(language: "ru", title: "Director commentary"))) == 6)
        #expect(TrackRules.rememberedAudio(next, memory: memory(same: false, audio: Pick(language: "ja"))) == nil, "not there: the usual rules")
        #expect(TrackRules.rememberedAudio(next, memory: memory(same: false, audio: Pick(index: 5))) == nil, "nothing to match by")
    }

    /// Resuming keeps the subtitles picked in the title: a second track of one language, a
    /// forced one, and Off (which the language preference turned into "forced" before).
    @Test func resumeKeepsTheSubtitles() {
        let options = [server(3, "English", "eng"), server(4, "English · SDH", "eng"), server(5, "English · Forced", "eng"),
                       SubtitleOption(id: "moonrise:20", title: "Russian · OpenSubtitles", language: "rus", kind: .engine(20))]
        #expect(remembered(options, memory(same: true, subtitle: Pick(index: 4, language: "en", title: "SDH", sdh: true))) == "t:4")
        #expect(remembered(options, memory(same: true, subtitle: Pick(index: 5, language: "en", forced: true))) == "t:5", "forced, by hand")
        #expect(remembered(options, memory(same: true, subtitle: Pick(index: 20, language: "ru"))) == "moonrise:20", "a fetched track")
        #expect(remembered(options, memory(same: true, off: true)) == "off")
        #expect(remembered(options, memory(same: true, subtitle: Pick(index: 3, language: "de"))) == "usual", "the index has another language now")
        #expect(remembered(options, memory(same: true, audio: Pick(index: 1))) == "usual", "subtitles not said")
        #expect(remembered(options, nil) == "usual")
    }

    /// The next episode: the same language and kind (forced or full), SDH and title first, text
    /// before pictures; Off stays off but forced subtitles may still come on.
    @Test func nextEpisodeSubtitles() {
        let next = [bitmap("pgs", "English", "eng"), server(7, "English · Forced", "eng"), server(8, "English", "eng"),
                    server(9, "English · SDH", "eng"), server(10, "Russian", "rus")]
        #expect(remembered(next, memory(same: false, subtitle: Pick(index: 4, language: "en", title: "SDH", sdh: true))) == "t:9")
        #expect(remembered(next, memory(same: false, subtitle: Pick(index: 9, language: "en", sdh: false))) == "t:8",
                "the plain one, not the index")
        #expect(remembered(next, memory(same: false, subtitle: Pick(language: "en", forced: true))) == "t:7")
        #expect(remembered(next, memory(same: false, subtitle: Pick(language: "ru"))) == "t:10")
        #expect(remembered(next, memory(same: false, subtitle: Pick(language: "ja"))) == "usual", "not there")
        #expect(remembered(next, memory(same: false, off: true)) == "usual-off")
        #expect(remembered([bitmap("pgs", "English", "eng")], memory(same: false, subtitle: Pick(language: "en"))) == "pgs",
                "a picture track when there's no text one")
    }

    /// What reports send: `subtitle: null` is Off; a download's indexes stay on the device.
    @Test func reportBody() {
        let c = TrackRules.Choice(audio: Pick(index: 2, language: "en", title: "Main"), subtitle: nil)
        #expect(c.json["subtitle"] is NSNull)
        #expect((c.json["audio"] as? [String: Any])?["index"] as? Int == 2)
        #expect(c.withoutIndexes.audio == Pick(language: "en", title: "Main"))
        #expect(c.memory == memory(same: true, audio: c.audio, off: true))
        #expect(TrackRules.subtitleDetail(server(1, "English · SDH", "eng")) == "SDH")
        #expect(TrackRules.subtitleDetail(server(1, "English", "eng")) == nil)
    }

    /// A picture track's menu row reads like a text one's, it is drawn from the server's images,
    /// and one of the same language and title as a text track still reads differently.
    @Test @MainActor func pictureTracksAreNamedLikeTextOnes() throws {
        let track = TrackInfo(kind: .subtitle, index: 4, language: "eng", title: "SDH", isText: false)
        let bitmap = try #require(TrackRules.fileSubtitleOption(track) { nil })
        #expect(bitmap.title == track.label)
        #expect(bitmap.kind == .pictures(index: 4))
        #expect(TrackRules.isPicture(bitmap))

        let url = try #require(URL(string: "https://example.invalid/4.srt"))
        let textTrack = TrackInfo(kind: .subtitle, index: 5, language: "eng", title: "SDH", isText: true)
        let text = try #require(TrackRules.fileSubtitleOption(textTrack) { url })
        #expect(TrackRules.fileSubtitleOption(textTrack) { nil } == nil, "a text track with no address")
        #expect(PlayerModel.distinctDetails([bitmap, text].map { ($0.name, $0.detail) }) == ["SDH · #1", "SDH · #2"])
    }

    /// A download keeps its tracks on the device until it is deleted.
    @Test @MainActor func downloadMemory() throws {
        let defaults = try #require(UserDefaults(suiteName: "TrackRulesTests.\(UUID().uuidString)"))
        let id = UUID()
        #expect(LocalTrackMemory.load(download: id, defaults: defaults) == nil)
        let c = TrackRules.Choice(audio: Pick(index: 1, language: "ja"), subtitle: Pick(language: "en", title: "Signs", forced: true))
        LocalTrackMemory.save(c, download: id, defaults: defaults)
        #expect(LocalTrackMemory.load(download: id, defaults: defaults) == c)
        LocalTrackMemory.forget(download: id, defaults: defaults)
        #expect(LocalTrackMemory.load(download: id, defaults: defaults) == nil)
    }
}

