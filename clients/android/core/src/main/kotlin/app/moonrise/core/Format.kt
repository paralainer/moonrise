package app.moonrise.core

// What the title pages and banners say about a title, worded as the Apple apps word it
// (`Fmt.runtime`, `MediaItem.techBadges`).

/** "1h 34m", "48m". */
fun runtimeText(seconds: Double?): String? {
    if (seconds == null || seconds <= 0) return null
    val minutes = (seconds / 60).toInt()
    return if (minutes >= 60) "${minutes / 60}h ${minutes % 60}m" else "${minutes}m"
}

/** "1:02:05", "4:09". */
fun clockText(seconds: Double): String {
    val s = seconds.toLong().coerceAtLeast(0)
    val h = s / 3600
    val m = (s % 3600) / 60
    val sec = s % 60
    return if (h > 0) "%d:%02d:%02d".format(h, m, sec) else "%d:%02d".format(m, sec)
}

/** "4K", "HD" or "SD" for a picture this size. */
fun resolutionLabel(width: Int?, height: Int?): String? {
    val w = width ?: return null
    val h = height ?: 0
    return when {
        w >= 3200 || h >= 1700 -> "4K"
        w >= 1200 || h >= 700 -> "HD"
        else -> "SD"
    }
}

/** Resolution, HDR, Atmos, channels and CC, from the file's tracks or the list's summary. */
val Item.techBadges: List<String> get() = techBadges(::resolutionLabel)

/**
 * The badges as the iPhone's title line has them (`MediaItem.techBadges`): the resolution as the
 * server's bucket ("4K", "1080p", "720p", "SD"; [ResolutionFilter.of]).
 */
val Item.phoneTechBadges: List<String> get() = techBadges { w, h -> if (w == null) null else ResolutionFilter.of(w, h ?: 0).label }

private fun Item.techBadges(resolution: (Int?, Int?) -> String?): List<String> {
        val out = mutableListOf<String>()
        val file = presentFiles.firstOrNull()
        if (file != null && file.tracks.isNotEmpty()) {
            val v = file.tracks.firstOrNull { it.kind.equals("video", true) }
            if (v != null) {
                resolution(v.width, v.height)?.let(out::add)
                hdrLabel(v.videoRangeType)?.let(out::add)
            }
            val audio = file.tracks.filter { it.kind.equals("audio", true) }
            (audio.firstOrNull { it.isDefault == true } ?: audio.firstOrNull())?.let { a ->
                if (a.displayTitle?.contains("Atmos") == true) out += "Atmos"
                channelsLabel(a.channels)?.let(out::add)
            }
            if (file.tracks.any { it.kind.equals("subtitle", true) }) out += "CC"
        } else if (media != null) {
            media.video?.let { v ->
                resolution(v.width, v.height)?.let(out::add)
                hdrLabel(v.range)?.let(out::add)
            }
            (media.audio.firstOrNull { it.default == true } ?: media.audio.firstOrNull())?.let { a ->
                if (a.profile?.contains("Atmos") == true) out += "Atmos"
                channelsLabel(a.channels)?.let(out::add)
            }
            if (media.subtitles.isNotEmpty()) out += "CC"
        }
        return out
    }

private fun hdrLabel(range: String?): String? = when {
    range == null || range == "SDR" || range == "Unknown" -> null
    range.contains("DOVI") -> "Dolby Vision"
    range.contains("HDR10Plus") -> "HDR10+"
    else -> "HDR"
}

private fun channelsLabel(ch: Int?): String? = when {
    ch == null -> null
    ch >= 8 -> "7.1"
    ch >= 6 -> "5.1"
    ch == 2 -> "Stereo"
    else -> "Mono"
}

/** A premiere date ("2016-11-04…") as the Apple TV's title line says it: "Nov 4, 2016" (the device's style). */
fun premiereText(date: String?, locale: java.util.Locale = java.util.Locale.getDefault()): String? {
    val d = date?.take(10) ?: return null
    return runCatching {
        java.time.LocalDate.parse(d).format(java.time.format.DateTimeFormatter.ofLocalizedDate(java.time.format.FormatStyle.MEDIUM).withLocale(locale))
    }.getOrNull()
}

/**
 * The title page's Rating line: TMDB's star first, then IMDb, Rotten Tomatoes and Metacritic as
 * the server has them ("★ 7.9   IMDb 8.1   RT 94%   MC 78"), as on Apple TV. Null: nothing to show.
 */
fun ratingLine(tmdb: Double?, r: Ratings?): String? {
    val parts = listOfNotNull(
        tmdb?.let { "★ %.1f".format(java.util.Locale.ROOT, it) },
        r?.imdb?.value?.let { "IMDb %.1f".format(java.util.Locale.ROOT, it) },
        r?.rottenTomatoes?.value?.let { "RT ${Math.round(it)}%" },
        r?.metacritic?.value?.let { "MC ${Math.round(it)}" },
    )
    return parts.takeIf { it.isNotEmpty() }?.joinToString("   ")
}

/** The credit IMDb asks for with its data, and OMDb's. */
const val RATINGS_ATTRIBUTION = "Information courtesy of IMDb (https://www.imdb.com). Used with permission. Rotten Tomatoes and Metacritic scores through OMDb (omdbapi.com)."
