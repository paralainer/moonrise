package app.moonrise.core

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import java.time.Instant
import java.time.OffsetDateTime

// Downloads and playing them with no server, as the iPhone app has them (`DownloadRecord`,
// `SegmentedDownloads`, `OfflineTitlePage`, `LocalReports`, `DownloadsView`): the rules here, the
// files and threads in the phone app's `shared.downloads`.

/** What the server answers for one offline set (`OfflineSet` in api/openapi.yaml). */
@Serializable
data class OfflineSet(
    val version: String,
    val variant: Variant,
    val duration: Double,
    val segments: Int,
    val audio: List<PlaybackTrack> = emptyList(),
    val subtitles: List<SubtitleEntry> = emptyList(),
    val master: String,
    val playlist: String,
    @SerialName("subs_playlist") val subsPlaylist: String,
    val expected: Long,
    val base: String,
) {
    @Serializable
    data class Variant(val id: String, val label: String, val bitrate: Long = 0, val transcode: Boolean = false)

    @Serializable
    data class SubtitleEntry(val index: Int, val name: String, val language: String? = null, val forced: Boolean = false)
}

@Serializable
enum class DownloadState {
    @SerialName("queued") Queued,
    @SerialName("downloading") Downloading,
    @SerialName("paused") Paused,
    @SerialName("completed") Completed,
    @SerialName("failed") Failed,
}

/** A text subtitle saved with a download. `file`: relative to the download's folder. */
@Serializable
data class LocalSubtitle(val title: String, val language: String? = null, val file: String, val rendition: String? = null)

/**
 * One download: what it is, how far it got, what it saved. Files are under the download's folder,
 * named by relative paths, so the app's storage can move. `item` is the title as the server
 * described it when the download started (offline pages, badges, the next episode, reports).
 */
@Serializable
data class DownloadRecord(
    val id: String,
    val state: DownloadState = DownloadState.Queued,
    val title: String,
    val subtitle: String? = null,
    /** The server it came from (`/v1/info`'s `server_id`). */
    val serverId: String,
    val itemId: String,
    /** The version (file) downloaded, of a title with several. */
    val mediaSourceId: String? = null,
    val qualityLabel: String,
    val versionLabel: String? = null,
    val bytesWritten: Long = 0,
    val bytesExpected: Long = 0,
    val progress: Double = 0.0,
    val posterFile: String? = null,
    val backdropFile: String? = null,
    val subtitles: List<LocalSubtitle> = emptyList(),
    val error: String? = null,
    val createdAt: Long = 0,
    val runtime: Double? = null,
    val year: Int? = null,
    /** Skip markers saved for offline playback; `segmentsPending`: asked for again later. */
    val segments: List<Segment>? = null,
    val segmentsPending: Boolean = false,
    val item: Item? = null,
    val series: Item? = null,
    val setBase: String? = null,
    val setVersion: String? = null,
    val segmentCount: Int? = null,
    val segmentsDone: Int? = null,
    /** The set's audio tracks, the first the default. */
    val audioTracks: List<PlaybackTrack>? = null,
    val subtitleTracks: List<Int>? = null,
) {
    /** "4K HDR10 · Director's Cut · 1080p", or the quality alone. */
    val downloadLabel: String get() = listOfNotNull(versionLabel, qualityLabel).joinToString(" · ")

    /** The set's playlists are on the device and segments are coming in (or all there). */
    val hasPlaylist: Boolean get() = setBase != null && segmentCount != null

    /** Plays from the device now: finished, or a download that has started. */
    val isPlayable: Boolean get() = state == DownloadState.Completed || (hasPlaylist && (segmentsDone ?: 0) > 0)

    val seriesId: String? get() = item?.seriesId

    /** The title as the download saved it, else as much as the record knows of it. */
    val pageItem: Item get() = item ?: Item(id = itemId, kind = "movie", title = title, year = year)

    /** "Downloaded · 480p", "Downloading 34 %": a title page's line under Play. */
    val statusLine: String
        get() = when (state) {
            DownloadState.Completed -> "Downloaded · $downloadLabel"
            DownloadState.Downloading -> "Downloading ${percent(progress)} %"
            DownloadState.Queued -> "Download waiting"
            DownloadState.Paused -> "Download paused at ${percent(progress)} %"
            DownloadState.Failed -> "Download failed: ${error.orEmpty()}"
        }

    /** The Downloads list's line under the progress bar. */
    val rowDetail: String
        get() {
            val p = percent(progress)
            return when {
                state == DownloadState.Paused -> "Paused · $p%"
                state == DownloadState.Queued -> "Waiting…"
                error != null -> "$error · $p%"
                isPlayable -> "$p% · ${bytesText(bytesWritten)} · Tap to play now"
                else -> "$p%"
            }
        }

    /** The small round icon on a card. */
    val badge: DownloadBadge
        get() = when (state) {
            DownloadState.Completed -> DownloadBadge.Downloaded
            DownloadState.Downloading -> DownloadBadge.Downloading(percent(progress))
            DownloadState.Queued -> DownloadBadge.Queued
            DownloadState.Paused -> DownloadBadge.Paused
            DownloadState.Failed -> DownloadBadge.Failed
        }

    companion object {
        fun listJson(r: List<DownloadRecord>): String = MoonriseApi.json.encodeToString(ListSerializer(serializer()), r)
        fun listFrom(s: String): List<DownloadRecord>? = runCatching { MoonriseApi.json.decodeFromString(ListSerializer(serializer()), s) }.getOrNull()
    }
}

/** A download's state on a card (`DownloadManager.Badge`). */
sealed interface DownloadBadge {
    data object Downloaded : DownloadBadge
    data class Downloading(val percent: Int) : DownloadBadge
    data object Queued : DownloadBadge
    data object Paused : DownloadBadge
    data object Failed : DownloadBadge
}

private fun percent(p: Double): Int = (p * 100).toInt()

/** Bytes as ByteCountFormatter's file style says them: "4.2 MB", "1.3 GB". */
fun bytesText(n: Long): String = when {
    n < 1_000 -> "$n bytes"
    n < 1_000_000 -> "%.0f KB".format(java.util.Locale.ROOT, n / 1e3)
    n < 1_000_000_000 -> "%.1f".format(java.util.Locale.ROOT, n / 1e6).removeSuffix(".0") + " MB"
    else -> "%.2f GB".format(java.util.Locale.ROOT, n / 1e9)
}

object DownloadRules {
    /** Segment requests in flight at once; one at a time while someone watches. */
    const val PARALLEL = 3

    /** A file the player asks for that isn't on the device: how long it waits for it (seconds). */
    const val FILE_WAIT_SECONDS = 60

    /**
     * Runs `write`, something that writes into a download's folder. A fetch can outlive the delete
     * of its download and make the folder again, so the folder goes afterwards when `kept` says
     * the download has gone. Whoever deletes a download removes its record before its folder:
     * then a write either ends before the folder is deleted, or sees the record gone here.
     */
    inline fun <T> writingInto(folder: java.io.File, kept: () -> Boolean, write: () -> T): T =
        try {
            write()
        } finally {
            if (!kept()) folder.deleteRecursively()
        }

    /**
     * Keeps a piece of artwork in `file`, and says whether it did. Artwork is never worth stopping
     * for: the server may not answer, the disk may be full, and the download may be deleted while
     * this fetches, which takes the folder away. Nothing else catches an error from here.
     */
    inline fun saveArtwork(file: java.io.File, fetch: () -> ByteArray): Boolean =
        runCatching { file.writeBytes(fetch()) }.isSuccess

    /** What a download keeps of its title: enough for offline pages, badges and reports. */
    fun snapshot(item: Item): Item = item.copy(people = null)

    /** The record for a new download of `item` (`DownloadManager.download`). */
    fun newRecord(item: Item, serverId: String, quality: Quality, version: MediaFile?, id: String, now: Long): DownloadRecord {
        val runtime = version?.duration ?: item.runtime
        return DownloadRecord(
            id = id,
            title = item.title.orEmpty().ifEmpty { "Untitled" },
            subtitle = if (item.isEpisode) listOfNotNull(item.seriesTitle, item.episodeLabel).joinToString(" · ") else item.year?.toString(),
            serverId = serverId,
            itemId = item.id,
            mediaSourceId = version?.id?.takeIf { item.hasVersions },
            qualityLabel = quality.label,
            versionLabel = version?.versionTitle?.takeIf { item.hasVersions },
            runtime = runtime,
            year = item.year,
            item = snapshot(item),
            createdAt = now,
            bytesExpected = if (quality.bitrate != null && runtime != null) (quality.bitrate / 8.0 * runtime).toLong() else 0,
        )
    }

    /** The record once the server has described its set. */
    fun described(rec: DownloadRecord, set: OfflineSet): DownloadRecord {
        val names = set.subtitles.associate { it.index to it.name }
        return rec.copy(
            setBase = set.base,
            setVersion = set.version,
            segmentCount = set.segments,
            segmentsDone = 0,
            audioTracks = set.audio,
            subtitleTracks = set.subtitles.map { it.index },
            bytesExpected = set.expected,
            qualityLabel = if (set.variant.transcode) set.variant.label else "Original",
            runtime = rec.runtime ?: set.duration,
            subtitles = rec.subtitles.map { s -> trackIndex(s.file)?.let { names[it] }?.let { s.copy(rendition = it) } ?: s },
        )
    }

    /** "sub-4.srt" → 4. */
    fun trackIndex(file: String): Int? = file.substringAfterLast('/').takeIf { it.startsWith("sub-") && it.endsWith(".srt") }?.removePrefix("sub-")?.removeSuffix(".srt")?.toIntOrNull()

    /** "video/12.m4s" → 12. */
    fun segment(path: String): Int? = path.substringAfterLast('/').takeIf { it.endsWith(".m4s") }?.removeSuffix(".m4s")?.toIntOrNull()

    /** Video, then each audio track: the folders of the set's tracks. */
    fun tracks(rec: DownloadRecord): List<String> = listOf("video") + rec.audioTracks.orEmpty().map { "audio/${it.index}" }

    /** The next segment to fetch: from where the viewer is, then from the start. */
    fun nextSegment(done: Set<Int>, claimed: Set<Int>, from: Int?, count: Int): Int? {
        val start = minOf(from ?: 0, count)
        for (n in start until count) if (n !in done && n !in claimed) return n
        for (n in 0 until start) if (n !in done && n !in claimed) return n
        return null
    }

    /** Segments from the first one on, without a gap. */
    fun contiguous(done: Set<Int>): Int {
        var n = 0
        while (n in done) n++
        return n
    }

    /** Seconds the first `n` segments of a media playlist cover. */
    fun seconds(playlist: String, n: Int): Double {
        var total = 0.0
        var seen = 0
        for (line in playlist.lineSequence()) {
            if (seen >= n) break
            if (line.startsWith("#EXTINF:")) {
                total += line.removePrefix("#EXTINF:").substringBefore(',').toDoubleOrNull() ?: 0.0
                seen++
            }
        }
        return total
    }

    /**
     * A media playlist cut after its first `n` segments, for a download that isn't complete with
     * no server: playback ends where the download does. A playlist with fewer segments is
     * unchanged.
     */
    fun truncated(playlist: String, n: Int): String {
        val out = StringBuilder()
        var seen = 0
        var uriPending = false
        for (line in playlist.lineSequence()) {
            if (line == "#EXT-X-ENDLIST") continue
            if (line.startsWith("#EXTINF:")) {
                if (seen >= n) break
                seen++
                uriPending = true
            } else if (uriPending && line.isNotBlank() && !line.startsWith("#")) {
                uriPending = false
            }
            out.append(line).append('\n')
        }
        out.append("#EXT-X-ENDLIST\n")
        return out.toString()
    }

    /** An episode's place in its show: season, episode, part. */
    private val episodeOrder = compareBy<Item>({ it.seasonNo ?: 0 }, { it.episodeNo ?: 0 }, { it.part ?: 0 })

    fun inOrder(records: List<DownloadRecord>): List<DownloadRecord> = records.sortedWith { a, b ->
        val x = a.item
        val y = b.item
        when {
            x == null || y == null -> 0
            else -> episodeOrder.compare(x, y)
        }
    }

    /** Downloaded (or downloading) episodes of a show, in airing order. */
    fun episodes(records: List<DownloadRecord>, seriesId: String): List<DownloadRecord> = inOrder(records.filter { it.seriesId == seriesId })

    /** What the title's Downloads list shows: in progress, the films, each show's episodes (`DownloadsView`). */
    data class Groups(val inProgress: List<DownloadRecord>, val moviesTitle: String, val movies: List<DownloadRecord>, val shows: List<Show>) {
        data class Show(val name: String, val episodes: List<DownloadRecord>)
    }

    fun groups(records: List<DownloadRecord>): Groups {
        val inProgress = records.filter { it.state != DownloadState.Completed }
        val done = records.filter { it.state == DownloadState.Completed }
        val movies = done.filter { it.item?.isEpisode != true }
        val shows = done.filter { it.item?.isEpisode == true }.groupBy { it.seriesId ?: "" }
            .map { (_, recs) -> Groups.Show(recs.first().item?.seriesTitle ?: "Episodes", inOrder(recs)) }
            .sortedBy { it.name }
        return Groups(inProgress, if (done.size == movies.size) "Ready to Watch" else "Movies", movies, shows)
    }

    /** Series and season counts: how many of their episodes are downloaded or downloading. */
    fun counts(records: List<DownloadRecord>): Pair<Map<String, Int>, Map<String, Int>> {
        val series = HashMap<String, Int>()
        val seasons = HashMap<String, Int>()
        for (r in records) {
            r.item?.seriesId?.let { series[it] = (series[it] ?: 0) + 1 }
            r.item?.seasonId?.let { seasons[it] = (seasons[it] ?: 0) + 1 }
        }
        return series to seasons
    }

    /** The downloaded titles a Library view shows: films, and shows made of their downloaded episodes (`OfflineLibrary`). */
    data class Title(val item: Item, val episodes: List<Item>)

    fun titles(records: List<DownloadRecord>, shows: Boolean): List<Title> {
        val playable = records.filter { it.isPlayable }
        if (!shows) return playable.mapNotNull { r -> r.item?.takeIf { !it.isEpisode }?.let { Title(it, emptyList()) } }
        return playable.filter { it.item?.isEpisode == true }.groupBy { it.seriesId ?: "" }.mapNotNull { (sid, eps) ->
            val ep = eps.first().item ?: return@mapNotNull null
            val show = eps.firstNotNullOfOrNull { it.series }
                ?: Item(id = sid, kind = "series", title = ep.seriesTitle, images = ep.parentImages ?: ItemImages(poster = ep.images?.poster))
            Title(show, eps.mapNotNull { it.item })
        }
    }

    /** The offline Library: titles that pass the filters, sorted as the server sorts. */
    fun offlineLibrary(records: List<DownloadRecord>, shows: Boolean, filter: LibraryFilter, now: Instant = Instant.now()): List<Item> {
        val titles = titles(records, shows).filter { filter.matches(it.item, it.episodes, now) }
        val latest = titles.associate { t -> t.item.id to t.episodes.mapNotNull { instant(it.addedAt) }.maxOrNull() }
        val added: (Item) -> Instant? = { if (shows) latest[it.id] ?: instant(it.addedAt) else instant(it.addedAt) }
        return filter.sorted(titles.map { it.item }, added)
    }

    /** What the filters can pick, counted over the downloads (each facet under the other filters). */
    fun offlineFacets(titles: List<Title>, filter: LibraryFilter, now: Instant = Instant.now()): LibraryFacets {
        fun pass(f: LibraryFilter) = titles.filter { f.matches(it.item, it.episodes, now) }
        fun tally(list: List<String>) = list.groupingBy { it }.eachCount().map { LibraryFacets.Named(it.key, it.value) }
            .sortedWith(compareBy({ -it.count }, { it.name }))
        val rated = pass(filter.copy(ratings = emptySet())).map { it.item.officialRating?.trim().orEmpty() }
        val ratings = rated.groupBy { if (it.isEmpty()) "NONE" else ratingKey(it) }.map { (key, spellings) ->
            val shown = spellings.groupingBy { it }.eachCount().maxWith(compareBy({ it.value }, { it.key })).key
            LibraryFacets.Named(if (key == "NONE") "none" else shown, spellings.size)
        }.sortedWith(compareBy({ it.name == "none" }, { -it.count }, { it.name }))
        val res = pass(filter.copy(resolutions = emptySet())).flatMap { t -> (if (t.item.isSeries) t.episodes else listOf(t.item)).mapNotNull { resolutionOf(it) }.toSet() }
        val colls = HashMap<Int, LibraryFacets.Collection>()
        for (t in pass(filter.copy(collection = null))) {
            val c = t.item.collection ?: continue
            colls[c.id] = (colls[c.id] ?: LibraryFacets.Collection(c.id, c.name ?: "Collection", 0)).let { it.copy(count = it.count + 1) }
        }
        return LibraryFacets(
            total = pass(filter).size,
            genres = tally(pass(filter.copy(genres = emptySet())).flatMap { it.item.genres.orEmpty().toSet() }),
            studios = tally(pass(filter.copy(studio = null)).flatMap { it.item.studios.orEmpty().toSet() }),
            decades = pass(filter.copy(decade = null)).mapNotNull { t -> t.item.year?.let { it - it % 10 } }.groupingBy { it }.eachCount()
                .map { LibraryFacets.Decade(it.key, it.value) }.sortedByDescending { it.decade },
            ratings = ratings,
            collections = colls.values.sortedWith(compareBy({ -it.count }, { it.name })),
            resolutions = ResolutionFilter.entries.mapNotNull { r -> res.count { it == r }.takeIf { it > 0 }?.let { LibraryFacets.Named(r.key, it) } },
            hdr = pass(filter.copy(hdr = false)).count { t -> (if (t.item.isSeries) t.episodes else listOf(t.item)).any { isHdr(it) } },
            watched = run {
                val all = pass(filter.copy(watched = WatchedFilter.All))
                LibraryFacets.Watched(
                    unplayed = all.count { filter.copy(watched = WatchedFilter.Unwatched).matches(it.item, it.episodes, now) },
                    inProgress = all.count { filter.copy(watched = WatchedFilter.InProgress).matches(it.item, it.episodes, now) },
                    played = all.count { filter.copy(watched = WatchedFilter.Watched).matches(it.item, it.episodes, now) },
                )
            },
            recentlyAdded = LibraryFacets.Recent(pass(filter.copy(recentlyAdded = true)).size),
        )
    }

    /** The server's letters for `items` sorted by title: where each letter starts. */
    fun letters(items: List<Item>): List<ItemPage.Letter> {
        val seen = LinkedHashMap<String, Int>()
        items.forEachIndexed { i, item -> seen.getOrPut(letter(item.title.orEmpty())) { i } }
        return seen.map { ItemPage.Letter(it.key, it.value) }
    }

    fun letter(title: String): String {
        val c = sortTitle(title).firstOrNull()?.uppercaseChar() ?: return "#"
        return if (c in 'A'..'Z') c.toString() else "#"
    }

    /** "The Matrix" → "Matrix" (as the server's sort titles). */
    fun sortTitle(t: String): String {
        for (a in listOf("the ", "a ", "an ")) if (t.length > a.length && t.lowercase().startsWith(a)) return t.substring(a.length)
        return t
    }

    fun ratingKey(r: String): String = r.filterNot { it.isWhitespace() }.uppercase()

    fun instant(s: String?): Instant? {
        s ?: return null
        return runCatching { Instant.parse(s) }.getOrNull() ?: runCatching { OffsetDateTime.parse(s).toInstant() }.getOrNull()
    }

    internal fun resolutionOf(item: Item): ResolutionFilter? {
        val v = item.media?.video
        val w = v?.width ?: item.files?.firstOrNull()?.tracks?.firstOrNull { it.kind.equals("video", true) }?.width
        val h = v?.height ?: item.files?.firstOrNull()?.videoHeight
        return if (w != null && h != null) ResolutionFilter.of(w, h) else null
    }

    internal fun isHdr(item: Item): Boolean = item.media?.video?.range?.let { it != "SDR" && it.isNotEmpty() } ?: false
}

/** Whether a title passes the filters (`LibraryFilter.matches`; shows: its downloaded `episodes`). */
fun LibraryFilter.matches(item: Item, episodes: List<Item> = emptyList(), now: Instant = Instant.now()): Boolean {
    if (genres.isNotEmpty() && genres.none { g -> item.genres.orEmpty().any { it.equals(g, ignoreCase = true) } }) return false
    decade?.let { d -> val y = item.year; if (y == null || y !in d..d + 9) return false }
    if (ratings.isNotEmpty()) {
        val key = item.officialRating?.let(DownloadRules::ratingKey)?.takeIf { it.isNotEmpty() } ?: "NONE"
        if (ratings.none { DownloadRules.ratingKey(it) == key }) return false
    }
    val playable = if (item.isSeries) episodes else listOf(item)
    when (watched) {
        WatchedFilter.All -> {}
        WatchedFilter.Unwatched -> if (playable.none { it.user?.played != true }) return false
        WatchedFilter.Watched -> if (playable.isEmpty() || playable.any { it.user?.played != true }) return false
        WatchedFilter.InProgress -> if (item.isSeries) {
            val started = episodes.any { it.user?.played == true || (it.user?.position ?: 0.0) > 0 }
            if (!started || episodes.none { it.user?.played != true }) return false
        } else if (item.user?.played == true || (item.user?.position ?: 0.0) <= 0) return false
    }
    if (resolutions.isNotEmpty() && playable.none { p -> DownloadRules.resolutionOf(p)?.let { it in resolutions } == true }) return false
    if (hdr && playable.none { DownloadRules.isHdr(it) }) return false
    if (recentlyAdded) {
        val cutoff = now.minusSeconds(LibraryFilter.RECENT_DAYS * 86_400L)
        if (playable.none { DownloadRules.instant(it.addedAt)?.isAfter(cutoff) == true }) return false
    }
    if (studio != null && studio !in item.studios.orEmpty()) return false
    if (collection != null && item.collection?.id != collection.id) return false
    library?.let { l -> (item.libraryId ?: episodes.firstOrNull()?.libraryId)?.let { if (it != l) return false } }
    return true
}

/** Sorted as the server sorts (ties by title); `added`: a show's latest episode's date. */
fun LibraryFilter.sorted(items: List<Item>, added: (Item) -> Instant? = { DownloadRules.instant(it.addedAt) }): List<Item> {
    val title = compareBy<Item, String>(String.CASE_INSENSITIVE_ORDER) { DownloadRules.sortTitle(it.title.orEmpty()) }.thenBy { it.id }
    fun <T : Comparable<T>> by(value: (Item) -> T?): Comparator<Item> = Comparator { x, y ->
        val a = value(x)
        val b = value(y)
        when {
            a != null && b != null && a != b -> if (descending) b.compareTo(a) else a.compareTo(b)
            a == null && b != null -> 1
            a != null && b == null -> -1
            else -> if (descending) title.compare(y, x) else title.compare(x, y)
        }
    }
    val cmp: Comparator<Item> = when (sort) {
        LibrarySort.Title -> if (descending) title.reversed() else title
        LibrarySort.Added -> by(added)
        LibrarySort.Year -> by { it.year }
        LibrarySort.Rating -> by { it.communityRating }
        LibrarySort.Played -> by { DownloadRules.instant(it.user?.lastPlayedAt) }
        LibrarySort.Runtime -> by { it.runtime }
    }
    return items.sortedWith(cmp)
}

/**
 * A title page made from what downloads saved (`OfflineTitlePage`): a download opens the normal
 * page, and with no server that page is this: the title (a show: as its episodes' downloads
 * described it), seasons made from the downloaded episodes, and each season's downloaded episodes.
 */
data class OfflineTitlePage(val item: Item, val seasons: List<Item>, val episodesBySeason: Map<String, List<Item>>) {
    fun episodes(season: Item?): List<Item> = season?.let { episodesBySeason[it.id] }.orEmpty()

    fun season(of: Item?): Item? = of?.let { ep -> seasons.firstOrNull { seasonKey(ep) == it.id } }

    companion object {
        fun build(itemId: String, records: List<DownloadRecord>): OfflineTitlePage? {
            val playable = records.filter { it.isPlayable }
            val mine = playable.filter { it.seriesId == itemId }
            val episodes = DownloadRules.inOrder(mine).mapNotNull { it.item }
            if (episodes.isNotEmpty()) {
                val first = episodes.first()
                val show = mine.firstNotNullOfOrNull { it.series } ?: Item(id = itemId, kind = "series", title = first.seriesTitle, images = first.parentImages)
                val seasons = mutableListOf<Item>()
                val bySeason = linkedMapOf<String, MutableList<Item>>()
                for (ep in episodes) {
                    val key = seasonKey(ep)
                    if (key !in bySeason) seasons += Item(id = key, kind = "season", title = seasonName(ep), seriesId = itemId, seriesTitle = ep.seriesTitle, seasonNo = ep.seasonNo)
                    bySeason.getOrPut(key) { mutableListOf() } += ep
                }
                return OfflineTitlePage(show, seasons, bySeason)
            }
            val rec = playable.firstOrNull { it.itemId == itemId } ?: return null
            return OfflineTitlePage(rec.pageItem, emptyList(), emptyMap())
        }

        /** The server's season id, else one made from the number. */
        fun seasonKey(ep: Item): String = ep.seasonId ?: "season:${ep.seasonNo ?: 0}"

        private fun seasonName(ep: Item): String {
            ep.seasonTitle?.takeIf { it.isNotEmpty() }?.let { return it }
            val s = ep.seasonNo ?: return "Episodes"
            return if (s == 0) "Specials" else "Season $s"
        }
    }
}

/** Playback of a download, kept until the server hears of it (`LocalReport` in api/openapi.yaml). */
@Serializable
data class LocalReport(
    val serverId: String,
    /** One playback (it counts as played once). */
    val viewing: String,
    val item: String,
    /** start, progress or stop. */
    val event: String,
    val position: Double,
    /** Unix seconds. */
    val at: Long,
    val duration: Double? = null,
    val credits: Double? = null,
    val tracks: TrackChoice? = null,
    /** The file (version) the download was made from. */
    val file: String? = null,
) {
    fun json(): JsonObject = JsonObject(buildMap<String, JsonElement> {
        put("viewing", JsonPrimitive(viewing))
        put("item", JsonPrimitive(item))
        put("event", JsonPrimitive(event))
        put("position", JsonPrimitive(position))
        put("at", JsonPrimitive(at))
        duration?.let { put("duration", JsonPrimitive(it)) }
        credits?.let { put("credits", JsonPrimitive(it)) }
        tracks?.let { put("tracks", it.json()) }
        file?.let { put("file", JsonPrimitive(it)) }
    })

    companion object {
        const val BATCH = 200

        /** Progress of the same viewing replaces the progress still waiting. */
        fun added(pending: List<LocalReport>, r: LocalReport): List<LocalReport> {
            val i = if (r.event != "start") pending.indexOfLast { it.viewing == r.viewing && it.event == "progress" } else -1
            return (if (i >= 0) pending.filterIndexed { k, _ -> k != i } else pending) + r
        }

        /** What to do with reports after a failed send: bad requests are dropped, the rest kept. */
        fun dropOnFailure(code: Int?): Boolean = code != null && code in 400..499 && code != 401 && code != 403

        fun listJson(r: List<LocalReport>): String = MoonriseApi.json.encodeToString(ListSerializer(serializer()), r)
        fun listFrom(s: String): List<LocalReport>? = runCatching { MoonriseApi.json.decodeFromString(ListSerializer(serializer()), s) }.getOrNull()
    }
}
