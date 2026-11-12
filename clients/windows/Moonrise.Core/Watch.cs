namespace Moonrise.Core;

// Versions and watch state: clients/android/core/.../Watch.kt and the Apple apps' equivalents.

public static class Versions
{
    public static IReadOnlyList<MediaFile> Playable(IReadOnlyList<MediaFile> files)
    {
        var ok = files.Where(f => !f.IsBroken).ToList();
        return ok.Count > 0 ? ok : files;
    }

    /// <summary>The version that plays: the one chosen here (pick) or on any device, if it can play; else the best.</summary>
    public static MediaFile? Chosen(Item item, string? pick, bool hdrDisplay, string? language)
    {
        var vs = item.PresentFiles;
        if (vs.Count == 0) return null;
        var ok = Playable(vs);
        var id = pick ?? item.User?.Version;
        if (id != null && ok.FirstOrDefault(f => f.Id == id) is { } chosen) return chosen;
        return Best(vs, hdrDisplay, language);
    }

    public static MediaFile? Best(IReadOnlyList<MediaFile> vs, bool hdrDisplay, string? language)
    {
        var pool = Playable(vs);
        var want = Languages.Normalized(language);
        if (want != null)
        {
            var first = pool.Where(v => Languages.Normalized(v.Version?.AudioLanguages?.FirstOrDefault()) == want).ToList();
            var any = pool.Where(v => (v.Version?.AudioLanguages ?? []).Any(l => Languages.Normalized(l) == want)).ToList();
            if (first.Count > 0 && first.Count < pool.Count) pool = first;
            else if (any.Count > 0 && any.Count < pool.Count) pool = any;
        }
        if (!hdrDisplay)
        {
            var top = pool.Count == 0 ? 0 : pool.Max(v => v.VideoHeight ?? 0);
            var sdr = pool.FirstOrDefault(v => v.Version?.Hdr == null && (v.VideoHeight ?? 0) >= top);
            if (sdr != null) return sdr;
        }
        return pool.FirstOrDefault();
    }

    /// <summary>Where a title page's Play starts: the version's own resume point, else the title's.</summary>
    public static double? Resume(Item item, MediaFile? version) =>
        item.HasVersions && version != null ? (version.Position is > 0 ? version.Position : null) : item.ResumeSeconds;
}

public sealed record WatchAction(bool Played, string Title)
{
    public static readonly WatchAction Watched = new(true, "Mark as Watched");
    public static readonly WatchAction Unwatched = new(false, "Mark as Unwatched");

    public static IReadOnlyList<WatchAction> Options(Item item)
    {
        if (item.User?.Played == true) return [Unwatched];
        var started = item.IsSeries || item.IsSeason
            ? item.User?.LastPlayedAt != null
            : (item.User?.Position ?? 0) > 0 || (item.User?.PlayedPercentage ?? 0) > 0;
        return started ? [Watched, Unwatched] : [Watched];
    }
}

public static class Languages
{
    private static readonly Dictionary<string, string> Bibliographic = new()
    {
        ["fre"] = "fr", ["ger"] = "de", ["chi"] = "zh", ["cze"] = "cs", ["dut"] = "nl", ["gre"] = "el", ["per"] = "fa", ["arm"] = "hy",
        ["geo"] = "ka", ["mac"] = "mk", ["may"] = "ms", ["bur"] = "my", ["rum"] = "ro", ["slo"] = "sk", ["alb"] = "sq", ["baq"] = "eu",
        ["ice"] = "is", ["wel"] = "cy", ["tib"] = "bo",
    };

    // ISO 639-2/T to 639-1: the same on every OS and in any globalization mode.
    private static readonly Dictionary<string, string> Alpha3 = new()
    {
        ["aar"] = "aa", ["abk"] = "ab", ["afr"] = "af", ["aka"] = "ak", ["amh"] = "am", ["ara"] = "ar", ["arg"] = "an", ["asm"] = "as", ["ava"] = "av", ["ave"] = "ae", ["aym"] = "ay",
        ["aze"] = "az", ["bak"] = "ba", ["bam"] = "bm", ["bel"] = "be", ["ben"] = "bn", ["bis"] = "bi", ["bod"] = "bo", ["bos"] = "bs", ["bre"] = "br", ["bul"] = "bg", ["cat"] = "ca",
        ["ces"] = "cs", ["cha"] = "ch", ["che"] = "ce", ["chu"] = "cu", ["chv"] = "cv", ["cor"] = "kw", ["cos"] = "co", ["cre"] = "cr", ["cym"] = "cy", ["dan"] = "da", ["deu"] = "de",
        ["div"] = "dv", ["dzo"] = "dz", ["ell"] = "el", ["eng"] = "en", ["epo"] = "eo", ["est"] = "et", ["eus"] = "eu", ["ewe"] = "ee", ["fao"] = "fo", ["fas"] = "fa", ["fij"] = "fj",
        ["fin"] = "fi", ["fra"] = "fr", ["fry"] = "fy", ["ful"] = "ff", ["gla"] = "gd", ["gle"] = "ga", ["glg"] = "gl", ["glv"] = "gv", ["grn"] = "gn", ["guj"] = "gu", ["hat"] = "ht",
        ["hau"] = "ha", ["heb"] = "he", ["her"] = "hz", ["hin"] = "hi", ["hmo"] = "ho", ["hrv"] = "hr", ["hun"] = "hu", ["hye"] = "hy", ["ibo"] = "ig", ["ido"] = "io", ["iii"] = "ii",
        ["iku"] = "iu", ["ile"] = "ie", ["ina"] = "ia", ["ind"] = "id", ["ipk"] = "ik", ["isl"] = "is", ["ita"] = "it", ["jav"] = "jv", ["jpn"] = "ja", ["kal"] = "kl", ["kan"] = "kn",
        ["kas"] = "ks", ["kat"] = "ka", ["kau"] = "kr", ["kaz"] = "kk", ["khm"] = "km", ["kik"] = "ki", ["kin"] = "rw", ["kir"] = "ky", ["kom"] = "kv", ["kon"] = "kg", ["kor"] = "ko",
        ["kua"] = "kj", ["kur"] = "ku", ["lao"] = "lo", ["lat"] = "la", ["lav"] = "lv", ["lim"] = "li", ["lin"] = "ln", ["lit"] = "lt", ["ltz"] = "lb", ["lub"] = "lu", ["lug"] = "lg",
        ["mah"] = "mh", ["mal"] = "ml", ["mar"] = "mr", ["mkd"] = "mk", ["mlg"] = "mg", ["mlt"] = "mt", ["mon"] = "mn", ["mri"] = "mi", ["msa"] = "ms", ["mya"] = "my", ["nau"] = "na",
        ["nav"] = "nv", ["nbl"] = "nr", ["nde"] = "nd", ["ndo"] = "ng", ["nep"] = "ne", ["nld"] = "nl", ["nno"] = "nn", ["nob"] = "nb", ["nor"] = "no", ["nya"] = "ny", ["oci"] = "oc",
        ["oji"] = "oj", ["ori"] = "or", ["orm"] = "om", ["oss"] = "os", ["pan"] = "pa", ["pli"] = "pi", ["pol"] = "pl", ["por"] = "pt", ["pus"] = "ps", ["que"] = "qu", ["roh"] = "rm",
        ["ron"] = "ro", ["run"] = "rn", ["rus"] = "ru", ["sag"] = "sg", ["san"] = "sa", ["sin"] = "si", ["slk"] = "sk", ["slv"] = "sl", ["sme"] = "se", ["smo"] = "sm", ["sna"] = "sn",
        ["snd"] = "sd", ["som"] = "so", ["sot"] = "st", ["spa"] = "es", ["srd"] = "sc", ["srp"] = "sr", ["ssw"] = "ss", ["sun"] = "su", ["swa"] = "sw", ["swe"] = "sv", ["tah"] = "ty",
        ["tam"] = "ta", ["tat"] = "tt", ["tel"] = "te", ["tgk"] = "tg", ["tgl"] = "tl", ["tha"] = "th", ["tir"] = "ti", ["ton"] = "to", ["tsn"] = "tn", ["tso"] = "ts", ["tuk"] = "tk",
        ["tur"] = "tr", ["twi"] = "tw", ["uig"] = "ug", ["ukr"] = "uk", ["urd"] = "ur", ["uzb"] = "uz", ["ven"] = "ve", ["vie"] = "vi", ["vol"] = "vo", ["wln"] = "wa", ["wol"] = "wo",
        ["xho"] = "xh", ["yid"] = "yi", ["yor"] = "yo", ["zha"] = "za", ["zho"] = "zh", ["zul"] = "zu",
    };

    public static string? Normalized(string? value)
    {
        var s = value?.Trim().ToLowerInvariant();
        if (string.IsNullOrEmpty(s)) return null;
        var cut = s.IndexOfAny(['-', '_']);
        if (cut > 0) s = s[..cut];
        if (s is "und" or "mul" or "zxx" or "mis" or "qaa") return null;
        if (Bibliographic.TryGetValue(s, out var b)) return b;
        if (Alpha3.TryGetValue(s, out var two)) return two;
        return s;
    }
}
