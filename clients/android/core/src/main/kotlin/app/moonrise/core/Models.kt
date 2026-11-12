package app.moonrise.core

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

// The Moonrise server's JSON (`server/`, `/v1/...`): snake_case, times in seconds, dates RFC 3339.
// Only the fields the clients use are declared; the decoder ignores the rest.

@Serializable
data class ServerInfo(
    @SerialName("server_id") val serverId: String,
    val name: String,
    val version: String,
    val capabilities: List<String> = emptyList(),
    /** "unclaimed": a new server that still needs setting up; "claimed"; null on older servers. */
    val setup: String? = null,
) {
    val unclaimed: Boolean get() = setup == "unclaimed"
    val deviceCode: Boolean get() = "device-code" in capabilities
    val invites: Boolean get() = "invites" in capabilities
}

@Serializable
data class User(
    val id: String,
    val name: String,
    @SerialName("is_admin") val isAdmin: Boolean? = null,
)

@Serializable
data class AuthResult(val token: String, val user: User)

@Serializable
data class DeviceCode(
    @SerialName("device_code") val deviceCode: String,
    @SerialName("user_code") val userCode: String,
    @SerialName("verify_url") val verifyUrl: String,
    @SerialName("verify_url_complete") val verifyUrlComplete: String,
    val interval: Int,
    @SerialName("expires_in") val expiresIn: Int,
)

sealed interface DevicePoll {
    data object Pending : DevicePoll
    data object SlowDown : DevicePoll
    data object Expired : DevicePoll
    data class Approved(val auth: AuthResult) : DevicePoll
}

@Serializable
data class ItemImages(
    val poster: String? = null,
    val backdrop: List<String>? = null,
    val logo: String? = null,
    val thumb: String? = null,
    @SerialName("season_poster") val seasonPoster: String? = null,
)

@Serializable
data class UserData(
    val position: Double? = null,
    val version: String? = null,
    val played: Boolean? = null,
    @SerialName("play_count") val playCount: Int? = null,
    @SerialName("played_percentage") val playedPercentage: Double? = null,
    @SerialName("unplayed_item_count") val unplayedItemCount: Int? = null,
    @SerialName("last_played_at") val lastPlayedAt: String? = null,
)

@Serializable
data class Track(
    val index: Int,
    val kind: String,
    val codec: String? = null,
    val language: String? = null,
    val title: String? = null,
    @SerialName("display_title") val displayTitle: String? = null,
    @SerialName("is_default") val isDefault: Boolean? = null,
    @SerialName("is_forced") val isForced: Boolean? = null,
    @SerialName("is_text") val isText: Boolean? = null,
    val width: Int? = null,
    val height: Int? = null,
    val channels: Int? = null,
    @SerialName("video_range_type") val videoRangeType: String? = null,
    val profile: String? = null,
    @SerialName("is_external") val isExternal: Boolean? = null,
    @SerialName("is_hearing_impaired") val isHearingImpaired: Boolean? = null,
)

@Serializable
data class VersionInfo(
    val label: String? = null,
    val details: String? = null,
    val edition: String? = null,
    val hdr: String? = null,
    val resolution: String? = null,
    @SerialName("audio_languages") val audioLanguages: List<String>? = null,
)

/** A file of a title: one version of it. */
@Serializable
data class MediaFile(
    val id: String,
    val name: String? = null,
    val duration: Double? = null,
    val size: Long? = null,
    val bitrate: Long? = null,
    @SerialName("version_label") val versionLabel: String? = null,
    val missing: Boolean? = null,
    val broken: Boolean? = null,
    @SerialName("broken_reason") val brokenReason: String? = null,
    /** The viewer's resume position in this file. */
    val position: Double? = null,
    val version: VersionInfo? = null,
    val tracks: List<Track> = emptyList(),
) {
    val isBroken: Boolean get() = broken == true

    /** The version's name in menus (`PlayerModel.versionTitle`, Apple apps). */
    val versionTitle: String get() = version?.label ?: versionLabel ?: name ?: "Version"

    /** The version's second line in menus: its details, or that it can't play (`versionDetails`). */
    val versionDetails: String? get() = if (isBroken) "Damaged or incomplete file · can't play" else version?.details

    val videoHeight: Int? get() = tracks.firstOrNull { it.kind.equals("video", true) }?.height
}

/** Someone in a title's cast or crew. `kind`: "Actor", "Director", "Writer"… as the server names them. */
@Serializable
data class Person(
    val id: String,
    val name: String? = null,
    val kind: String? = null,
    val role: String? = null,
    val image: String? = null,
) {
    /** "Actor", "Director", "Writer" as the Apple apps group them (`MoonriseImages.personKind`). */
    val group: String
        get() = when (kind?.lowercase()) {
            "actor", "cast", "guest_star", "gueststar" -> "Actor"
            "director" -> "Director"
            "writer", "screenplay" -> "Writer"
            null -> ""
            else -> kind.replaceFirstChar { it.uppercase() }
        }
}

/** One score: IMDb 0-10 (with its votes), Rotten Tomatoes and Metacritic 0-100. */
@Serializable
data class Score(val value: Double? = null, val votes: Long? = null)

/** IMDb, Rotten Tomatoes and Metacritic, as the server keeps them (servers from 0.26). */
@Serializable
data class Ratings(
    val imdb: Score? = null,
    @SerialName("rotten_tomatoes") val rottenTomatoes: Score? = null,
    val metacritic: Score? = null,
)

@Serializable
data class Item(
    val id: String,
    /** "movie", "series", "season" or "episode". */
    val kind: String,
    val title: String? = null,
    val year: Int? = null,
    val overview: String? = null,
    val tagline: String? = null,
    @SerialName("official_rating") val officialRating: String? = null,
    @SerialName("community_rating") val communityRating: Double? = null,
    /** Scores beside TMDB's (servers from 0.26). */
    val ratings: Ratings? = null,
    val genres: List<String>? = null,
    val runtime: Double? = null,
    @SerialName("premiere_date") val premiereDate: String? = null,
    val studios: List<String>? = null,
    val people: List<Person>? = null,
    val images: ItemImages? = null,
    @SerialName("parent_images") val parentImages: ItemImages? = null,
    @SerialName("series_id") val seriesId: String? = null,
    @SerialName("series_title") val seriesTitle: String? = null,
    @SerialName("season_id") val seasonId: String? = null,
    @SerialName("season_title") val seasonTitle: String? = null,
    @SerialName("season_no") val seasonNo: Int? = null,
    @SerialName("episode_no") val episodeNo: Int? = null,
    @SerialName("episode_end") val episodeEnd: Int? = null,
    /** One part of an episode split over several files: part 1 of `parts`. */
    val part: Int? = null,
    val parts: Int? = null,
    @SerialName("library_id") val libraryId: String? = null,
    /** When the title was added (RFC 3339): the offline Library's Date Added and Recently Added. */
    @SerialName("added_at") val addedAt: String? = null,
    /** The TMDB collection a movie belongs to. */
    val collection: CollectionRef? = null,
    val user: UserData? = null,
    val files: List<MediaFile>? = null,
    /** Lists: how many files (versions) the title has. */
    val versions: Int? = null,
    /** Lists: a summary of the main file's tracks (tech badges). */
    val media: MediaSummary? = null,
    /** "Imdb", "Tmdb", "Tvdb" ids. */
    @SerialName("provider_ids") val providerIds: Map<String, String>? = null,
    /** In `/v1/home`'s Continue Watching: why the title is there and since when. */
    @SerialName("continue") val continueInfo: ContinueInfo? = null,
) {
    val isEpisode: Boolean get() = kind == "episode"
    fun providerId(name: String): String? = providerIds?.entries?.firstOrNull { it.key.equals(name, ignoreCase = true) }?.value?.takeIf { it.isNotEmpty() }
    val isSeries: Boolean get() = kind == "series"
    val isSeason: Boolean get() = kind == "season"
    val isPlayable: Boolean get() = kind == "movie" || kind == "episode"

    /** Files that are on disk. */
    val presentFiles: List<MediaFile> get() = files.orEmpty().filter { it.missing != true }

    val isMovie: Boolean get() = kind == "movie"

    /** "3", or "12–13" for a file with two episodes. */
    val episodeNumberText: String?
        get() = episodeNo?.let { n -> if (episodeEnd != null && episodeEnd > n) "$n–$episodeEnd" else "$n" }

    /** An episode in a season's list: "4. Title", "12–13. Title", "23. Part 2 · Title" (`episodeListTitle`). */
    val episodeListTitle: String
        get() = (episodeNumberText?.let { "$it. " } ?: "") + (part?.let { "Part $it · " } ?: "") + (title ?: "Untitled")

    /** Has several versions (files) to choose from. */
    val hasVersions: Boolean get() = presentFiles.size > 1

    /** "S1 · E3", "S3 · E12–13" for a file with two, "S5 · E23 · Part 2" (`episodeLabel`, Apple apps). */
    val episodeLabel: String?
        get() {
            if (!isEpisode) return null
            val e = episodeNo?.let { n -> if (episodeEnd != null && episodeEnd > n) "E$n–$episodeEnd" else "E$n" }
            return listOfNotNull(seasonNo?.let { "S$it" }, e, part?.let { "Part $it" }).joinToString(" · ")
        }

    /**
     * The two lines under a card (`PosterCard.labels` / `LandscapeCard`, Apple apps): the title
     * (the show's name for an episode or season, which carry the show's artwork), then the year,
     * the season's name, or "S2 · E6 · Title".
     */
    val cardTitle: String
        get() = (if (isEpisode || isSeason) seriesTitle else null) ?: title ?: "Untitled"

    val cardSubtitle: String
        get() = when {
            isSeason -> title ?: "Untitled"
            isEpisode -> listOfNotNull(episodeLabel?.takeIf { it.isNotEmpty() }, title).joinToString(" · ")
            else -> year?.toString() ?: " "
        }

    /** The resume position, when there is one (> 0), watched before or not: the rule of `api/resume-rules.json`. */
    val resumeSeconds: Double?
        get() = user?.position?.takeIf { it > 0 }

    /** 0–1 progress of the resume position, for the bar on cards. A Continue Watching resume
     *  card counts even when the title was watched before (a rewatch part-way through). */
    val progress: Double?
        get() {
            // A watched title shows no bar on its page or in other rows, only as a Continue Watching resume.
            val p = user?.position?.takeIf { it >= 1 && (user?.played != true || continueInfo?.reason == "resume") } ?: return null
            val total = runtime ?: presentFiles.firstOrNull()?.duration ?: return user?.playedPercentage?.let { it / 100 }
            return if (total > 0) (p / total).coerceIn(0.0, 1.0) else null
        }

    // Artwork by role, as the Apple apps pick it: an episode's card art is its own still, else
    // its show's; posters fall back to the parent's.
    val posterImage: String? get() = images?.poster ?: parentImages?.poster
    val backdropImage: String? get() = images?.backdrop?.firstOrNull() ?: parentImages?.backdrop?.firstOrNull()
    val logoImage: String? get() = images?.logo ?: parentImages?.logo
    val cardImage: String?
        get() = (if (isEpisode) images?.thumb else null)
            ?: images?.thumb ?: parentImages?.thumb ?: backdropImage ?: posterImage
}

@Serializable
data class MediaSummary(
    val video: Video? = null,
    val audio: List<Audio> = emptyList(),
    val subtitles: List<Subtitle> = emptyList(),
) {
    @Serializable
    data class Video(val codec: String? = null, val width: Int? = null, val height: Int? = null, val range: String? = null)

    @Serializable
    data class Audio(val codec: String? = null, val channels: Int? = null, val profile: String? = null, val default: Boolean? = null)

    @Serializable
    data class Subtitle(val codec: String? = null, val language: String? = null)
}

@Serializable
data class Library(
    val id: String,
    val name: String,
    @SerialName("collection_type") val collectionType: String? = null,
    val backdrop: String? = null,
    @SerialName("item_count") val itemCount: Int? = null,
)

@Serializable
data class ListAnswer<T>(val items: List<T>, val total: Int? = null)

@Serializable
data class HomeRow(val type: String, val title: String, val items: List<Item>)

/**
 * Why a title is in Continue Watching: `resume` (part-way through), `next_episode` (after the one
 * finished), `collection_next` (the next film of a collection); `at` when that activity happened
 * (RFC 3339), `from` the title it was on.
 */
@Serializable
data class ContinueInfo(val reason: String, val at: String? = null, val from: String? = null)

@Serializable
data class Home(
    /** Continue Watching, latest activity first (servers from 0.24.0; null before). */
    @SerialName("continue") val continueWatching: List<Item>? = null,
    val resume: List<Item> = emptyList(),
    @SerialName("next_up") val nextUp: List<Item> = emptyList(),
    @SerialName("latest_types") val latestTypes: List<HomeRow> = emptyList(),
) {
    /**
     * The one Continue Watching row: the server's (the same on every device). From an older
     * server, its resume points, then its next episodes without shows already in progress.
     */
    val continueRow: List<Item>
        get() = continueWatching ?: run {
            val inProgress = resume.filter { it.isEpisode && it.user?.played != true }.mapNotNull { it.seriesId }.toSet()
            resume + nextUp.filter { it.seriesId !in inProgress }
        }
}

/** `GET /v1/items/{id}/playback`: how the server serves a title, and its HLS session. */
@Serializable
data class Playback(
    val supported: Boolean,
    val reasons: List<String> = emptyList(),
    /** The session's master playlist, relative to the server. */
    val master: String? = null,
    val video: PlaybackVideo? = null,
    val audio: List<PlaybackTrack> = emptyList(),
    val subtitles: List<PlaybackTrack> = emptyList(),
    /** "copy", "convert" (some audio converted) or "transcode". */
    val method: String? = null,
    val variant: String? = null,
    val session: String? = null,
    /** The keyframe index is still being made: ask again later. */
    val pending: Boolean? = null,
    @SerialName("media_source_id") val mediaSourceId: String? = null,
    val duration: Double? = null,
    /** The tracks this user picked last in the file, the title or the show (server 0.29.0+, `track-memory`). */
    val tracks: TrackMemory? = null,
)

@Serializable
data class PlaybackVideo(
    val codec: String,
    /** SDR, PQ or HLG. */
    val range: String,
    val width: Int? = null,
    val height: Int? = null,
)

@Serializable
data class PlaybackTrack(
    val index: Int,
    val name: String,
    val language: String? = null,
    val default: Boolean? = null,
    val converted: Boolean? = null,
)

/**
 * A title's scrubbing thumbnails (`GET /v1/items/{id}/trickplay`, capability `trickplay`): one
 * picture every `interval` seconds, on JPEG sheets of `columns` × `rows` pictures of
 * `width` × `height`. `state`: "ready", "pending" (not made yet) or "off".
 */
@Serializable
data class Thumbnails(
    val state: String,
    val interval: Double? = null,
    val width: Int? = null,
    val height: Int? = null,
    val columns: Int? = null,
    val rows: Int? = null,
    val count: Int? = null,
    val sheets: List<String> = emptyList(),
) {
    /** Where a picture is: its sheet, and its top left corner on that sheet (pixels). */
    data class Tile(val sheet: Int, val x: Int, val y: Int)

    val ready: Boolean
        get() = state == "ready" && (interval ?: 0.0) > 0 && (width ?: 0) > 0 && (height ?: 0) > 0 &&
            (columns ?: 0) > 0 && (rows ?: 0) > 0 && (count ?: 0) > 0 && sheets.isNotEmpty()

    /** The picture shown for `time` (seconds). */
    fun tile(time: Double): Tile? {
        if (!ready || time.isNaN()) return null
        val i = (time / interval!!).toInt().coerceIn(0, count!! - 1)
        val per = columns!! * rows!!
        val k = i % per
        val sheet = i / per
        if (sheet >= sheets.size) return null
        return Tile(sheet, (k % columns) * width!!, (k / columns) * height!!)
    }
}
