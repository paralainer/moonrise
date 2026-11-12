package app.moonrise.core

// Versions and watch state, with the Apple apps' rules (`VersionChoices.swift`, `VersionViews.swift`,
// `WatchState.swift`).

/** Which version (file) of a title plays. */
object Versions {
    /** The versions that can play: the healthy ones (all of them when none is). */
    fun playable(files: List<MediaFile>): List<MediaFile> = files.filter { !it.isBroken }.ifEmpty { files }

    /**
     * The version that plays for `item`: the one chosen here (`pick`) or on any device
     * (`user.version`), if it can play; else the best for this device. Null: one version or none.
     */
    fun chosen(item: Item, pick: String?, hdrDisplay: Boolean, language: String?): MediaFile? {
        val vs = item.presentFiles
        if (vs.isEmpty()) return null
        val ok = playable(vs)
        (pick ?: item.user?.version)?.let { id -> ok.firstOrNull { it.id == id }?.let { return it } }
        return best(vs, hdrDisplay, language)
    }

    /**
     * The best version for this device: one in the preferred audio language when the versions
     * differ in it, an SDR one where the display can't show HDR and there's one as sharp, else the
     * server's order (its first is its default). A damaged one never, while there's another.
     */
    fun best(vs: List<MediaFile>, hdrDisplay: Boolean, language: String?): MediaFile? {
        var pool = playable(vs)
        val want = Languages.normalized(language)
        if (want != null) {
            val first = pool.filter { Languages.normalized(it.version?.audioLanguages?.firstOrNull()) == want }
            val any = pool.filter { v -> v.version?.audioLanguages.orEmpty().any { Languages.normalized(it) == want } }
            if (first.isNotEmpty() && first.size < pool.size) pool = first
            else if (any.isNotEmpty() && any.size < pool.size) pool = any
        }
        if (!hdrDisplay) {
            val top = pool.maxOfOrNull { it.videoHeight ?: 0 } ?: 0
            pool.firstOrNull { it.version?.hdr == null && (it.videoHeight ?: 0) >= top }?.let { return it }
        }
        return pool.firstOrNull()
    }

    /** Where a title page's Play starts: the version's own resume point, else the title's. */
    fun resume(item: Item, version: MediaFile?): Double? =
        if (item.hasVersions && version != null) version.position?.takeIf { it > 0 } else item.resumeSeconds

    /**
     * Whether a title opened to play can start: a list's item only says how many versions it has,
     * so one with several waits for the page's load (`finished` also when that failed), which says
     * which one plays and where it was left.
     */
    fun readyToAutoplay(item: Item, finished: Boolean): Boolean = finished || item.hasVersions || (item.versions ?: 0) <= 1
}

/** One Mark as Watched / Mark as Unwatched action (`WatchAction`). */
enum class WatchAction(val played: Boolean, val title: String) {
    Watched(true, "Mark as Watched"),
    Unwatched(false, "Mark as Unwatched");

    companion object {
        /**
         * What a title's menu offers: watched → Unwatched; part-way through (a resume position, or a
         * show or season with some episodes watched) → both (Unwatched also clears the resume
         * position); not started → Watched.
         */
        fun options(item: Item): List<WatchAction> {
            if (item.user?.played == true) return listOf(Unwatched)
            val started = if (item.isSeries || item.isSeason) item.user?.lastPlayedAt != null
            else (item.user?.position ?: 0.0) > 0 || (item.user?.playedPercentage ?: 0.0) > 0
            return if (started) listOf(Watched, Unwatched) else listOf(Watched)
        }
    }
}

/** `item` as a mark leaves it, before the server's fresh answer: watched or not, no resume position. */
fun Item.marked(played: Boolean): Item {
    val u = (user ?: UserData()).copy(
        played = played, position = 0.0, playedPercentage = null,
        unplayedItemCount = if (isSeries || isSeason) (if (played) 0 else null) else user?.unplayedItemCount,
    )
    return copy(user = u)
}

/** Language codes as the Apple apps compare them (`LanguageCode.normalized`): ISO 639-1 where there is one. */
object Languages {
    private val bibliographic = mapOf(
        "fre" to "fr", "ger" to "de", "chi" to "zh", "cze" to "cs", "dut" to "nl", "gre" to "el", "per" to "fa", "arm" to "hy",
        "geo" to "ka", "mac" to "mk", "may" to "ms", "bur" to "my", "rum" to "ro", "slo" to "sk", "alb" to "sq", "baq" to "eu",
        "ice" to "is", "wel" to "cy", "tib" to "bo",
    )
    private val alpha3: Map<String, String> by lazy {
        java.util.Locale.getISOLanguages().associateBy { runCatching { java.util.Locale(it).isO3Language }.getOrDefault(it) }
    }

    fun normalized(value: String?): String? {
        var s = value?.trim()?.lowercase()?.takeIf { it.isNotEmpty() } ?: return null
        s = s.substringBefore('-').substringBefore('_')
        if (s in setOf("und", "mul", "zxx", "mis", "qaa")) return null
        bibliographic[s]?.let { return it }
        if (s.length == 3) alpha3[s]?.let { return it }
        return s
    }

    /** Settings' language choices (the Apple apps' list). */
    val common: List<Pair<String, String>> = listOf(
        "en" to "English", "ru" to "Russian", "uk" to "Ukrainian", "es" to "Spanish", "fr" to "French",
        "de" to "German", "it" to "Italian", "pt" to "Portuguese", "nl" to "Dutch", "pl" to "Polish",
        "sv" to "Swedish", "ja" to "Japanese", "ko" to "Korean", "zh" to "Chinese", "ar" to "Arabic",
        "he" to "Hebrew", "tr" to "Turkish", "el" to "Greek", "cs" to "Czech", "fi" to "Finnish",
    )
}

/** The streaming qualities (`QualityOption`): Original, or a bitrate the server transcodes to. */
data class Quality(val label: String, val bitrate: Long?, val height: Int?) {
    companion object {
        val original = Quality("Original", null, null)
        val presets = listOf(
            Quality("4K · 40 Mbps", 40_000_000, 2160),
            Quality("1080p · 20 Mbps", 20_000_000, 1080),
            Quality("1080p · 10 Mbps", 10_000_000, 1080),
            Quality("720p · 4 Mbps", 4_000_000, 720),
            Quality("480p · 1.5 Mbps", 1_500_000, 480),
        )
        val all = listOf(original) + presets

        fun of(label: String?): Quality = all.firstOrNull { it.label == label } ?: original

        /** Original plus the presets below the source: less bandwidth and no larger picture. */
        fun options(sourceBitrate: Long?, sourceHeight: Int?): List<Quality> = listOf(original) + presets.filter { p ->
            if (sourceBitrate != null && sourceBitrate > 0 && p.bitrate!! >= sourceBitrate * 0.9) return@filter false
            if (sourceHeight != null && sourceHeight > 0) {
                val tier = when { sourceHeight > 1080 -> 2160; sourceHeight > 720 -> 1080; sourceHeight > 480 -> 720; else -> 480 }
                if (p.height!! > tier) return@filter false
            }
            true
        }
    }

    /** "≈ 2.3 GB" for a duration (seconds; Original: the source's size). */
    fun sizeLabel(duration: Double?, sourceSize: Long?): String? {
        val bytes = if (bitrate == null) sourceSize else duration?.takeIf { it > 0 }?.let { (bitrate / 8.0 * it).toLong() }
        bytes ?: return null
        val gb = bytes / 1e9
        val text = if (gb >= 1) "%.1f GB".format(java.util.Locale.ROOT, gb) else "%.0f MB".format(java.util.Locale.ROOT, bytes / 1e6)
        return if (bitrate == null) text else "≈ $text"
    }

    /** This quality, or the best the source has below it (Original when none is). */
    fun capped(sourceBitrate: Long?, sourceHeight: Int?): Quality {
        val opts = options(sourceBitrate, sourceHeight)
        if (this in opts) return this
        return opts.firstOrNull { it != original && (it.bitrate ?: Long.MAX_VALUE) <= (bitrate ?: Long.MAX_VALUE) } ?: original
    }
}
