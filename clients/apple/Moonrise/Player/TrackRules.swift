import Foundation

/// Which audio and subtitle tracks a title starts with (`PlayerModel` applies them): pure, so
/// the tests (MoonriseTests/TrackRulesTests.swift) can check them without a player.
enum TrackRules {
    struct AudioCandidate {
        let id: Int
        let language: String?
        let isDefault: Bool
        /// Title words: commentary and audio description tracks are passed over.
        let name: String
    }

    /// The track in the preferred audio language (`preferred`, any form `LanguageCode` reads; its
    /// default one, else not a commentary), or nil: the title's own default plays.
    static func preferredAudio(_ tracks: [AudioCandidate], preferred: String) -> Int? {
        guard let want = LanguageCode.normalized(preferred) else { return nil }
        let matching = tracks.filter { LanguageCode.normalized($0.language) == want }
        let main = matching.filter { $0.name.range(of: #"(?i)comment|descri|narrat|director"#, options: .regularExpression) == nil }
        return (main.first(where: \.isDefault) ?? main.first ?? matching.first)?.id
    }

    static func isForced(_ o: SubtitleOption) -> Bool { o.title.lowercased().contains("forced") }
    static func isSDH(_ o: SubtitleOption) -> Bool {
        o.title.range(of: #"(?i)\b(sdh|cc|hearing)\b"#, options: .regularExpression) != nil
    }

    static func isPicture(_ o: SubtitleOption) -> Bool {
        if case .pictures = o.kind { return true }
        return false
    }

    /// The option's language; for a file without one, the code in its name ("Film.en.srt").
    static func subtitleLanguage(_ o: SubtitleOption) -> String? {
        if let l = LanguageCode.normalized(o.language) { return l }
        let parts = o.title.lowercased().split(separator: ".").dropLast()
        return parts.reversed().compactMap { p in p.count == 2 || p.count == 3 ? LanguageCode.normalized(String(p)) : nil }
            .first { Locale.current.localizedString(forLanguageCode: $0) != nil }
    }

    /// A file's subtitle track as a menu choice: a text track needs its file's address (`url`), a
    /// picture one (PGS, DVD, DVB) comes as images from the server. Nil: a text track with no
    /// address.
    static func fileSubtitleOption(_ s: TrackInfo, url: () -> URL?) -> SubtitleOption? {
        let kind: SubtitleOption.Kind
        if s.isText ?? true {
            guard let u = url() else { return nil }
            kind = .external(u)
        } else {
            kind = .pictures(index: s.index)
        }
        return SubtitleOption(id: "t:\(s.index)", title: s.label, language: s.language, kind: kind, trackIndex: s.index)
    }

    /// The subtitles a title starts with: when they are on (`autoSelect`), the preferred language's
    /// (`language`; the kind picked last first, SDH or plain, `preferSDH`; a picture track, whose
    /// size and style can't change, only when there is no text one), never a forced one; else
    /// forced subtitles for foreign dialogue in the audio's language (`audioLanguage`,
    /// normalized), text or pictures, or the first forced track when the audio's language isn't
    /// known. Nil: subtitles stay off.
    static func autoSubtitle(_ options: [SubtitleOption], autoSelect: Bool, language: String, preferSDH: Bool,
                             audioLanguage: String?) -> SubtitleOption? {
        let lang = LanguageCode.normalized(language) ?? language.lowercased()
        func matches(_ o: SubtitleOption, _ lang: String) -> Bool { subtitleLanguage(o) == lang }
        if autoSelect {
            let all = options.filter { matches($0, lang) && !isForced($0) }
            let text = all.filter { !isPicture($0) }
            let pool = text.isEmpty ? all : text
            if let o = pool.first(where: { isSDH($0) == preferSDH }) ?? pool.first { return o }
        }
        // Forced subtitles translate the foreign dialogue for viewers of the audio's language.
        let forced = options.filter { isForced($0) }
        if let audioLanguage { return forced.first { matches($0, audioLanguage) } }
        return forced.first
    }

    // MARK: - Track memory

    /// One track as the server keeps it (`TrackPick` in api/openapi.yaml): any field may be missing.
    struct Pick: Codable, Equatable, Hashable {
        var index: Int?
        var language: String?
        var title: String?
        var forced: Bool?
        var sdh: Bool?

        /// The body of a report (`tracks.audio`, `tracks.subtitle`).
        var json: [String: Any] {
            var d: [String: Any] = [:]
            if let index { d["index"] = index }
            if let language { d["language"] = language }
            if let title { d["title"] = title }
            if let forced { d["forced"] = forced }
            if let sdh { d["sdh"] = sdh }
            return d
        }
    }

    /// The tracks the user picked last (the playback answer's `tracks`, or a download's own).
    /// `sameFile`: picked in this very file, so an index can be trusted (when its language agrees).
    struct Memory: Codable, Equatable {
        var sameFile: Bool
        var audio: Pick?
        var subtitle: Pick?
        var subtitleOff: Bool

        enum CodingKeys: String, CodingKey {
            case audio, subtitle
            case sameFile = "same_file"
            case subtitleOff = "subtitle_off"
        }
    }

    /// The tracks playing, as a report sends them (`tracks`): a nil subtitle is Off.
    struct Choice: Codable, Equatable {
        var audio: Pick?
        var subtitle: Pick?

        var json: [String: Any] {
            var d: [String: Any] = ["subtitle": subtitle?.json ?? NSNull()]
            if let audio { d["audio"] = audio.json }
            return d
        }

        /// Without the device's own indexes (a download's): the server keeps its own indexes.
        var withoutIndexes: Choice {
            var c = self
            c.audio?.index = nil
            c.subtitle?.index = nil
            return c
        }

        /// As remembered for the same file (a download's own memory).
        var memory: Memory { Memory(sameFile: true, audio: audio, subtitle: subtitle, subtitleOff: subtitle == nil) }
    }

    /// A subtitle option's words past its language ("SDH", "Forced"): what tells it from others
    /// of its language, in any app and any device language.
    static func subtitleDetail(_ o: SubtitleOption) -> String? {
        let d = o.detail?.trimmingCharacters(in: .whitespaces)
        return d?.isEmpty == false ? d : nil
    }

    private static let extraAudio = #"(?i)comment|descri|narrat|director"#

    private static func sameText(_ a: String?, _ b: String?) -> Bool {
        guard let a = a?.trimmingCharacters(in: .whitespaces).lowercased(), !a.isEmpty,
              let b = b?.lowercased() else { return false }
        return b.contains(a)
    }

    /// The audio track picked last, in `tracks`: in the same file its index (when its language
    /// agrees); else (another file, the next episode) the same language, the same title first,
    /// and a commentary or description only if that is what was picked. Nil: none like it, the
    /// usual rules pick.
    static func rememberedAudio(_ tracks: [AudioCandidate], memory: Memory?) -> Int? {
        guard let memory, let want = memory.audio else { return nil }
        let lang = LanguageCode.normalized(want.language)
        if memory.sameFile, let i = want.index, let t = tracks.first(where: { $0.id == i }),
           lang == nil || LanguageCode.normalized(t.language) == lang {
            return i
        }
        let same = tracks.filter { LanguageCode.normalized($0.language) == lang }
        if lang == nil, same.isEmpty || want.title == nil { return nil }
        if let t = same.first(where: { sameText(want.title, $0.name) }) { return t.id }
        guard lang != nil else { return nil }
        let extra = want.title?.range(of: extraAudio, options: .regularExpression) != nil
        let kind = same.filter { ($0.name.range(of: extraAudio, options: .regularExpression) != nil) == extra }
        return (kind.first(where: \.isDefault) ?? kind.first)?.id
    }

    /// What the remembered subtitles say for a title starting.
    enum SubtitleMemory: Equatable {
        /// This track.
        case track(SubtitleOption)
        /// Off, as picked in this file: no subtitles at all.
        case off
        /// Off in another file (the next episode): the usual rules with subtitles off (forced
        /// subtitles for foreign dialogue still come on).
        case usualOff
        /// Nothing remembered, or nothing like it here: the usual rules.
        case usual
    }

    /// The subtitles picked last, among `options`: in the same file the track with its index (when
    /// its language agrees); else one of the same language and kind (forced or full), the same
    /// SDH and title first, text before pictures.
    static func rememberedSubtitle(_ options: [SubtitleOption], memory: Memory?) -> SubtitleMemory {
        guard let memory else { return .usual }
        if memory.subtitleOff { return memory.sameFile ? .off : .usualOff }
        guard let want = memory.subtitle else { return .usual }
        let lang = LanguageCode.normalized(want.language)
        if memory.sameFile, let i = want.index,
           let o = options.first(where: { $0.trackIndex == i || $0.id == "moonrise:\(i)" }),
           lang == nil || subtitleLanguage(o) == lang {
            return .track(o)
        }
        let forced = want.forced ?? false
        let pool = options.filter { subtitleLanguage($0) == lang && isForced($0) == forced }
        let title = want.title.map { $0.trimmingCharacters(in: .whitespaces) }.flatMap { $0.isEmpty ? nil : $0 }
        if lang == nil, pool.isEmpty || title == nil { return .usual }
        let sdh = want.sdh ?? false
        let ordered = pool.filter { !isPicture($0) } + pool.filter(isPicture)
        let found = ordered.first { isSDH($0) == sdh && sameText(title, $0.title) }
            ?? ordered.first { sameText(title, $0.title) && lang != nil }
            ?? ordered.first { isSDH($0) == sdh && lang != nil }
            ?? (lang != nil ? ordered.first : nil)
        return found.map { .track($0) } ?? .usual
    }
}
