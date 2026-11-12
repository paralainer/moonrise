package app.moonrise.core

/** One line of a tablet title page's Details section (`TitleDetails`, Apple apps). */
data class TitleFact(val label: String, val value: String)

/** Details: the title's facts on a tablet's title page (`TitleDetails.facts`). */
object TitleFacts {
    fun of(item: Item, focus: Item?, seasons: Int, locale: java.util.Locale = java.util.Locale.getDefault()): List<TitleFact> {
        val out = mutableListOf<TitleFact>()
        fun add(label: String, value: String?) { if (!value.isNullOrEmpty()) out += TitleFact(label, value) }
        val studios = item.studios.orEmpty()
        fun names(group: String): String? = item.people.orEmpty().filter { it.group == group }.mapNotNull { it.name }.distinct().take(3).joinToString(", ").ifEmpty { null }
        fun count(group: String) = item.people.orEmpty().count { it.group == group }
        if (item.isSeries) {
            add(if (studios.size > 1) "Networks" else "Network", studios.take(3).joinToString(", "))
            add("Seasons", if (seasons > 0) "$seasons" else null)
            add("First Aired", premiereText(item.premiereDate, locale))
        } else {
            add(if (count("Director") > 1) "Directors" else "Director", names("Director"))
            add(if (count("Writer") > 1) "Writers" else "Writer", names("Writer"))
            add(if (studios.size > 1) "Studios" else "Studio", studios.take(3).joinToString(", "))
            add("Released", premiereText(item.premiereDate, locale))
        }
        add("Genres", item.genres.orEmpty().take(4).joinToString(", "))
        val file = if (item.isSeries) focus else item
        add("Rated", item.officialRating ?: file?.officialRating)
        val version = file?.presentFiles?.firstOrNull()
        val tracks = version?.tracks.orEmpty()
        val prefix = if (item.isSeries) "Episode " else ""
        tracks.firstOrNull { it.kind.equals("video", true) }?.let { v ->
            add(prefix + "Video", v.displayTitle ?: listOfNotNull(v.height?.let { h -> "${h}p" }, v.codec?.uppercase(), v.videoRangeType).joinToString(" "))
        }
        add(prefix + "Audio", languages(tracks.filter { it.kind.equals("audio", true) }))
        add(prefix + "Subtitles", languages(tracks.filter { it.kind.equals("subtitle", true) }))
        if (version != null) {
            val container = version.name?.substringAfterLast('.', "")?.takeIf { it.length in 1..5 }?.uppercase()
            val parts = listOfNotNull(container, version.size?.let { bytesText(it) }, version.bitrate?.let { "%.1f Mbps".format(java.util.Locale.ROOT, it / 1_000_000.0) })
            add(prefix + "File", parts.joinToString(" · "))
        }
        return out
    }

    private fun languages(tracks: List<Track>): String? {
        val seen = tracks.map { it.audioName }.distinct()
        if (seen.isEmpty()) return null
        return if (seen.size > 6) seen.take(6).joinToString(", ") + " +${seen.size - 6}" else seen.joinToString(", ")
    }
}
