package app.moonrise.core

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

// The Library's filters, sorts and categories, as the Apple apps have them
// (`clients/apple/Moonrise/Models/LibraryFilter.swift`). The server applies them (`GET /v1/items`,
// `/v1/facets`, `/v1/libraries/{id}/facets`).

/** The Library's two views: the movies and the shows of every library (`type=` across libraries). */
enum class LibraryView(val id: String, val type: String, val title: String) {
    Movies("@movie", "movie", "Movies"),
    Shows("@series", "series", "Shows");

    val shows: Boolean get() = this == Shows

    /** "300 Movies", "1 Show". */
    fun countTitle(n: Int): String = "$n " + if (shows) (if (n == 1) "Show" else "Shows") else (if (n == 1) "Movie" else "Movies")

    companion object {
        fun of(id: String?): LibraryView = entries.firstOrNull { it.id == id } ?: Movies
    }
}

enum class LibrarySort(val query: String, val label: String) {
    Title("title", "Title"),
    Added("added", "Date Added"),
    Year("year", "Year"),
    Rating("rating", "Rating"),
    Played("played", "Last Played"),
    Runtime("runtime", "Runtime");

    /** The direction a sort starts in (newest, best, longest first; titles A to Z). */
    val descending: Boolean get() = this != Title

    /** The two directions' names (ascending, descending), as the Apple TV's Sort menu says them. */
    val directions: Pair<String, String>
        get() = when (this) {
            Title -> "A to Z" to "Z to A"
            Added, Year -> "Oldest First" to "Newest First"
            Rating -> "Lowest First" to "Highest First"
            Played -> "Least Recent First" to "Most Recent First"
            Runtime -> "Shortest First" to "Longest First"
        }
}

enum class WatchedFilter(val label: String) {
    All("All"), Unwatched("Unwatched"), InProgress("In Progress"), Watched("Watched")
}

enum class ResolutionFilter(val key: String, val label: String) {
    Uhd("4k", "4K"), Fhd("1080", "1080p"), Hd("720", "720p"), Sd("sd", "SD");

    companion object {
        fun of(key: String): ResolutionFilter? = entries.firstOrNull { it.key == key }

        /** The server's bucket for a picture this size (`browse::res_bucket`): 576p and below are SD. */
        fun of(width: Int, height: Int): ResolutionFilter = when {
            width <= 1024 && height <= 576 -> Sd
            width <= 1280 && height <= 962 -> Hd
            width <= 2560 && height <= 1440 -> Fhd
            else -> Uhd
        }
    }
}

enum class LibraryCategory(val key: String) {
    Genres("genres"), Collections("collections"), Studios("studios"), Decades("decades");

    fun title(shows: Boolean): String = when (this) {
        Genres -> "Genres"
        Collections -> "Collections"
        Studios -> if (shows) "Networks" else "Studios"
        Decades -> "Decades"
    }

    companion object {
        /** A view's categories: shows have no collections. */
        fun of(view: LibraryView): List<LibraryCategory> = entries.filter { it != Collections || !view.shows }
    }
}

/** A TMDB collection a movie belongs to. */
@Serializable
data class CollectionRef(val id: Int, val name: String? = null)

/** The Library's filters and sort (`LibraryFilter`). Kept per view as JSON. */
@Serializable
data class LibraryFilter(
    val sort: LibrarySort = LibrarySort.Title,
    val descending: Boolean = false,
    val genres: Set<String> = emptySet(),
    /** First year of a decade (1990 = 1990–1999). */
    val decade: Int? = null,
    /** Ratings as the facets name them ("none": not rated). */
    val ratings: Set<String> = emptySet(),
    val watched: WatchedFilter = WatchedFilter.All,
    val resolutions: Set<ResolutionFilter> = emptySet(),
    val hdr: Boolean = false,
    val recentlyAdded: Boolean = false,
    /** One library's titles (with several), and its name to show. */
    val library: String? = null,
    val libraryName: String? = null,
    /** Categories (set by a category's tile). */
    val studio: String? = null,
    val collection: CollectionRef? = null,
) {
    companion object {
        const val RECENT_DAYS = 30

        /** A filter kept as JSON; null when it can't be read. */
        fun fromJson(s: String): LibraryFilter? = runCatching { MoonriseApi.json.decodeFromString(serializer(), s) }.getOrNull()
    }

    fun toJson(): String = MoonriseApi.json.encodeToString(serializer(), this)

    /** The same sort, no filters. */
    val cleared: LibraryFilter get() = LibraryFilter(sort = sort, descending = descending)

    /** Any filter on (the sort isn't one). */
    val isFiltered: Boolean get() = cleared != this

    val activeCount: Int
        get() = genres.size + (if (decade == null) 0 else 1) + ratings.size + (if (watched == WatchedFilter.All) 0 else 1) +
            resolutions.size + (if (hdr) 1 else 0) + (if (recentlyAdded) 1 else 0) + (if (studio == null) 0 else 1) +
            (if (collection == null) 0 else 1) + (if (library == null) 0 else 1)

    /** A new sort starts in its own direction. */
    fun sortedBy(s: LibrarySort): LibraryFilter = if (s == sort) this else copy(sort = s, descending = s.descending)

    /** The server's query parameters for these filters (not the sort). */
    val serverQuery: Map<String, String>
        get() {
            val q = linkedMapOf<String, String>()
            if (genres.isNotEmpty()) q["genre"] = genres.sorted().joinToString(",")
            decade?.let { q["year_from"] = "$it"; q["year_to"] = "${it + 9}" }
            if (ratings.isNotEmpty()) q["rating"] = ratings.sorted().joinToString(",")
            when (watched) {
                WatchedFilter.All -> {}
                WatchedFilter.Unwatched -> q["unplayed"] = "1"
                WatchedFilter.InProgress -> q["in_progress"] = "1"
                WatchedFilter.Watched -> q["played"] = "1"
            }
            if (resolutions.isNotEmpty()) q["resolution"] = resolutions.sortedBy { it.ordinal }.joinToString(",") { it.key }
            if (hdr) q["hdr"] = "1"
            if (recentlyAdded) q["added_days"] = "$RECENT_DAYS"
            studio?.let { q["studio"] = it }
            collection?.let { q["collection"] = "${it.id}" }
            library?.let { q["library"] = it }
            return q
        }

    /** The filters that are on, as one line ("Action · 1990s · Unwatched"), as on Apple TV. */
    val summary: String
        get() {
            val parts = mutableListOf<String>()
            collection?.let { parts += it.name ?: "Collection" }
            studio?.let { parts += it }
            parts += genres.sorted()
            decade?.let { parts += "${it}s" }
            parts += ratings.sorted().map(::ratingLabel)
            if (watched != WatchedFilter.All) parts += watched.label
            parts += resolutions.sortedBy { it.ordinal }.map { it.label }
            if (hdr) parts += "HDR"
            if (recentlyAdded) parts += "Added in $RECENT_DAYS days"
            return parts.joinToString(" · ")
        }
}

/**
 * The phone Library's count line (`LibraryBrowser.countLine`, iPhone): "7 movies · Title, A to Z",
 * "1 show · Date Added, newest first".
 */
fun LibraryFilter.countLine(shows: Boolean, total: Int): String {
    val noun = if (shows) (if (total == 1) "show" else "shows") else (if (total == 1) "movie" else "movies")
    val (up, down) = sort.directions
    val dir = (if (descending) down else up).let { if (sort == LibrarySort.Title) it else it.lowercase() }
    return "$total $noun · ${sort.label}, $dir"
}

/** "none" is how the facets name titles without a rating. */
fun ratingLabel(r: String): String = if (r == "none") "Not Rated" else r

/** One page of a library's titles: `index` (a title sort) says where each letter starts. */
data class ItemPage(val items: List<Item>, val total: Int, val index: List<Letter>) {
    @Serializable
    data class Letter(val letter: String, val offset: Int)
}

/** What the filters can pick, with counts (`/v1/facets`). */
@Serializable
data class LibraryFacets(
    val total: Int = 0,
    val genres: List<Named> = emptyList(),
    val studios: List<Named> = emptyList(),
    val decades: List<Decade> = emptyList(),
    val ratings: List<Named> = emptyList(),
    val collections: List<Collection> = emptyList(),
    val resolutions: List<Named> = emptyList(),
    val hdr: Int = 0,
    val watched: Watched? = null,
    @SerialName("recently_added") val recentlyAdded: Recent? = null,
    val libraries: List<Lib> = emptyList(),
) {
    @Serializable
    data class Named(val name: String, val count: Int, val backdrop: String? = null)

    @Serializable
    data class Decade(val decade: Int, val count: Int, val backdrop: String? = null) {
        val label: String get() = "${decade}s"
    }

    @Serializable
    data class Collection(val id: Int, val name: String, val count: Int, val poster: String? = null, val backdrop: String? = null)

    @Serializable
    data class Watched(
        val unplayed: Int = 0,
        @SerialName("in_progress") val inProgress: Int = 0,
        val played: Int = 0,
    ) {
        fun count(w: WatchedFilter): Int? = when (w) {
            WatchedFilter.Unwatched -> unplayed
            WatchedFilter.InProgress -> inProgress
            WatchedFilter.Watched -> played
            WatchedFilter.All -> null
        }
    }

    @Serializable
    data class Recent(val count: Int = 0)

    @Serializable
    data class Lib(val id: String, val name: String, val count: Int)

    /** Libraries to offer (several, with titles under the other filters, or the chosen one). */
    fun libraryChoices(chosen: String?): List<Lib> = libraries.filter { it.count > 0 || it.id == chosen }

    /** A category page's tiles: name, count, artwork, and the filter a tile sets (`LibraryCategoryView`). */
    fun tiles(category: LibraryCategory): List<Tile> {
        val base = LibraryFilter(sort = LibrarySort.Title)
        return when (category) {
            LibraryCategory.Genres -> genres.map { Tile(it.name, it.name, it.count, it.backdrop, null, base.copy(genres = setOf(it.name))) }
            LibraryCategory.Studios -> (if (studios.size > 24) studios.filter { it.count > 1 } else studios).map {
                Tile(it.name, it.name, it.count, it.backdrop, null, base.copy(studio = it.name))
            }
            LibraryCategory.Decades -> decades.map {
                Tile(it.label, it.label, it.count, it.backdrop, null, base.copy(decade = it.decade, sort = LibrarySort.Year, descending = false))
            }
            LibraryCategory.Collections -> (if (collections.size > 24) collections.filter { it.count > 1 } else collections).map {
                Tile("${it.id}", it.name, it.count, it.backdrop, it.poster,
                    base.copy(collection = CollectionRef(it.id, it.name), sort = LibrarySort.Year, descending = false))
            }
        }
    }

    data class Tile(val id: String, val title: String, val count: Int, val art: String?, val poster: String?, val filter: LibraryFilter)
}
