package app.moonrise.core

// Skippable segments (intro, recap, credits…) and when the player offers Next Episode, with the
// Apple apps' rules (`clients/apple/Moonrise/Server/SegmentRules.swift`, `PlayerModel.swift`).
// Servers 0.28+ apply the segment rules themselves (`segments?merged=1`); the app keeps them for
// older servers. All three are tested against `api/segment-rules.json` (SegmentRulesCasesTest).

/** What a segment is. Skip Intro auto-skips by default; credits get a button (skipping them ends the video). */
@kotlinx.serialization.Serializable
enum class SegmentKind(val id: String, val title: String) {
    Intro("intro", "Intro"),
    Recap("recap", "Recap"),
    Credits("credits", "Credits"),
    Preview("preview", "Preview"),
    Commercial("commercial", "Commercial");

    val skipLabel: String get() = "Skip $title"
    val defaultMode: SkipMode get() = if (this == Credits) SkipMode.Button else SkipMode.Auto

    companion object {
        fun of(id: String?): SegmentKind? = entries.firstOrNull { it.id == id }
    }
}

/** What the player does on entering a segment (Settings, per kind). */
enum class SkipMode(val id: String, val title: String) {
    Auto("auto", "Skip Automatically"),
    Button("button", "Show Skip Button"),
    Off("off", "Off");

    companion object {
        fun of(id: String?): SkipMode? = entries.firstOrNull { it.id == id }
    }
}

/**
 * A skippable stretch of a title. `source`: "detected" when the server found the end credits in
 * the file itself, else null (named chapters or a community database).
 */
@kotlinx.serialization.Serializable
data class Segment(val kind: SegmentKind, val start: Double, val end: Double, val source: String? = null) {
    val id: String get() = "${kind.id}-${start.toInt()}"
}

/**
 * The server's answer (`GET /v1/items/{id}/segments`). `pending`: it is still looking for the end
 * credits in the file. `reached`: false when the server couldn't be asked (not a final answer).
 * `merged`: the server merged them with the community databases' (servers 0.28+; the segments to
 * offer as they are). `communityPending`: it is still looking the title up there (ask again shortly).
 */
data class ServerSegments(
    val items: List<Segment> = emptyList(),
    val pending: Boolean = false,
    val reached: Boolean = true,
    val merged: Boolean = false,
    val communityPending: Boolean = false,
)

object SegmentRules {
    /**
     * The server's segments, with the community databases' filling the kinds it lacks (the first
     * database that has one wins), and the end credits checked against the file. A credits chapter
     * never starts before the latest database start ([notBefore]). While the server
     * is still looking for the credits in the file (`pending`), the databases' aren't used: they
     * may be another cut's, and the player asks again before the end.
     *
     * A film (`movie`) may have scenes in or after its credits, so its credits are only skippable
     * when they end before the end of the file ([CREDITS_END_BEFORE_END]): Skip Credits then jumps
     * to that end and the scene after them plays. Credits running to the end are left out (the
     * file's own chapter ones give way to the databases', which may have an end).
     *
     * While the title still waits for its community lookup (`communityPending`), no credits at all:
     * the file's could start before a database start not known yet. The other kinds are
     * offered: the file's own always win over the databases'.
     */
    fun merge(
        server: List<Segment>,
        online: List<List<Segment>>,
        runtime: Double,
        pending: Boolean = false,
        movie: Boolean = false,
        communityPending: Boolean = false,
    ): List<Segment> {
        val detected = server.firstOrNull { it.kind == SegmentKind.Credits && it.source == "detected" }
        var found = server.filter { it.source == null && !(movie && it.kind == SegmentKind.Credits && !endsBeforeEnd(it, runtime)) }.toMutableList()
        for (more in online) {
            val have = found.map { it.kind }.toSet()
            found += more.filter { it.kind != SegmentKind.Credits && it.kind !in have }
        }
        val community = online.flatMap { list -> list.filter { it.kind == SegmentKind.Credits } }.filter { plausibleCredits(it, runtime) }
        val latest = community.maxByOrNull { it.start }
        if (!pending && latest != null) {
            found = found.mapNotNull { s ->
                if (s.kind != SegmentKind.Credits) s else notBefore(s, latest).takeIf { !movie || endsBeforeEnd(it, runtime) }
            }.toMutableList()
        }
        if (!pending && found.none { it.kind == SegmentKind.Credits }) {
            credits(community, detected, runtime)?.takeIf { !movie || endsBeforeEnd(it, runtime) }?.let { found += it }
        }
        return found
            .map { Segment(it.kind, maxOf(0.0, it.start), if (runtime > 0) minOf(it.end, runtime) else it.end, it.source) }
            .filter { it.end - it.start >= 3 && !(communityPending && it.kind == SegmentKind.Credits) }
            .sortedBy { it.start }
    }

    /**
     * Credits chapters never start before the latest database start: like what the file shows, a
     * chapter only moves the credits later (Guardians of the Galaxy: chapter 6796, IntroDB 6852).
     * When the database start is past the chapter's end, the database's credits are used.
     */
    fun notBefore(chapter: Segment, database: Segment): Segment = when {
        database.start <= chapter.start -> chapter
        database.start >= chapter.end - 3 -> database
        else -> Segment(SegmentKind.Credits, database.start, chapter.end, chapter.source)
    }

    /**
     * Where the end credits start, as the progress reports tell the server (`credits`): the
     * credits the player offers. The server marks a film watched from there rather than at 90 %
     * (it knows only the file's chapters, not the databases').
     */
    fun creditsStart(segments: List<Segment>): Double? = segments.filter { it.kind == SegmentKind.Credits }.maxOfOrNull { it.start }

    /** A film's credits must end at least this long before the end of the file to be skippable. */
    const val CREDITS_END_BEFORE_END = 5.0

    /** Whether a segment stops before the end of the file (not when the runtime isn't known). */
    fun endsBeforeEnd(s: Segment, runtime: Double): Boolean = runtime > 0 && s.end < runtime - CREDITS_END_BEFORE_END

    /**
     * Whether community end credits can be this file's: in its second half, starting before its
     * end, and not ending well after it (a longer cut). Without a runtime nothing is known.
     */
    fun plausibleCredits(s: Segment, runtime: Double): Boolean {
        if (runtime <= 0) return true
        return s.start >= runtime / 2 && s.start <= runtime - 3 && s.end <= runtime + 5
    }

    /**
     * Community end credits running to the end are trusted this long before it with nothing to
     * back them. Measured: right ones started 34–131 s before the end of their file, and the wrong
     * ones (another cut, inside the last scene) 170–280 s before it. Longer credits are kept when
     * another database has credits too.
     */
    const val UNCONFIRMED_CREDITS_LEAD = 150.0
    /**
     * A database start at most this long before the credits found in the file is used as it is (it
     * times them more closely); one earlier than that gives way to the found start.
     */
    const val DETECTED_SLACK = 10.0

    /**
     * The end credits to use: never earlier than a database's start, and with several, the latest
     * (later is never story). What the file shows only moves them later.
     * - With the file's own start: the latest database start, unless the found start is more than
     *   [DETECTED_SLACK] after it (another cut's value inside the last scene, or long credits found
     *   late in the file): then the found start.
     * - With none found in the file: the latest database start. A lone database value that runs to
     *   the end from far before it (more than [UNCONFIRMED_CREDITS_LEAD]) is left out (the
     *   lone-credits rule); with two or more values, the latest of them all (Agatha 1x01's 2107 and
     *   2235: 2235, whichever of them stops before the end).
     */
    fun credits(community: List<Segment>, detected: Segment?, runtime: Double = 0.0): Segment? {
        if (detected == null) {
            val latest = community.maxByOrNull { it.start } ?: return null
            if (community.size >= 2 || runtime <= 0 || latest.end < runtime - 5 || runtime - latest.start <= UNCONFIRMED_CREDITS_LEAD) return latest
            return null
        }
        val latest = community.maxByOrNull { it.start } ?: return detected
        return if (latest.start >= detected.start - DETECTED_SLACK) latest else detected
    }
}

/**
 * When the player offers Next Episode and what Skip does at the credits (`PlayerModel`).
 */
object NextEpisodeRules {
    /**
     * How long before the end Next Episode shows when the credits aren't known: short, since some
     * shows' story runs to within half a minute of the end.
     */
    const val LEAD = 30.0

    /**
     * The episode after `current` in airing order, crossing into the next season (a part of a
     * split episode counts). The specials (listed last) only follow a special.
     */
    fun next(episodes: List<Item>, current: Item?): Item? {
        val cur = current ?: return null
        val i = episodes.indexOfFirst { it.id == cur.id }
        if (i < 0 || i + 1 >= episodes.size) return null
        val next = episodes[i + 1]
        if (next.seasonNo == 0 && (episodes[i].seasonNo ?: 0) != 0) return null
        return next
    }

    /**
     * Near the end, Next Episode is offered with the controls: in the end credits (from where they
     * start, if they run to the end), or in the last [LEAD] seconds. Not for movies or a show's
     * last episode. `shown`: the time the controls show (a pending skip's target).
     */
    fun offered(duration: Double, currentTime: Double, shown: Double, active: Segment?, segments: List<Segment>, hasNext: Boolean): Boolean {
        if (duration <= 0 || currentTime <= 0 || !hasNext) return false
        if (active?.kind == SegmentKind.Credits) return true
        if (segments.any { it.kind == SegmentKind.Credits && shown >= it.start && it.end >= duration - 5 }) return true
        return duration - shown <= LEAD
    }

    /**
     * What the iPhone's player shows at the end of its bottom row (`PlayerView`, `CornerButtons`):
     * the segment's Skip while the controls show, for a few seconds on entering it
     * (`promptVisible`) or while an auto-skip counts down; Next Episode with the controls when
     * it's offered, unless that Skip (end credits to the end) already plays the next episode.
     */
    fun corner(active: Segment?, controlsVisible: Boolean, promptVisible: Boolean, counting: Boolean, nextOffered: Boolean, creditsSkipPlaysNext: Boolean): Corner {
        val skip = active != null && (controlsVisible || promptVisible || counting)
        val next = controlsVisible && nextOffered && !(skip && creditsSkipPlaysNext)
        return Corner(skip, next)
    }

    /** The corner's buttons: Skip (Intro, Recap, Credits…) and Next Episode. */
    data class Corner(val skip: Boolean, val next: Boolean) {
        val empty: Boolean get() = !skip && !next
    }

    /** Skipping end credits that run to the end moves on to the next episode (the button says Next Episode then). */
    fun creditsSkipPlaysNext(active: Segment?, duration: Double, hasNext: Boolean): Boolean =
        active != null && active.kind == SegmentKind.Credits && hasNext && active.end >= duration - 5

    /**
     * The server was still looking for the end credits when the segments were fetched: asked again
     * from 5 minutes before the end, then each minute while it still is, until the last minute.
     */
    fun refetchDue(pending: Boolean, duration: Double, t: Double, askedAt: Double): Boolean =
        pending && duration > 0 && duration - t < 300 && duration - t > 60 && kotlin.math.abs(t - askedAt) >= 60

    /** The segment playback is inside at `t` that can be skipped (its kind isn't Off). */
    fun active(segments: List<Segment>, t: Double, mode: (SegmentKind) -> SkipMode): Segment? =
        segments.firstOrNull { t >= it.start && t < it.end - 1 && mode(it.kind) != SkipMode.Off }
}

/**
 * Finds a title's skippable segments (`SegmentService`). The server merges the file's own (named
 * chapters, an episode's end credits found in the file) with the community databases' (TheIntroDB,
 * IntroDB), which it looks up and keeps itself (`segments?merged=1`, servers 0.28+): the app
 * contacts no other service. An older server answers with the file's own, and the app
 * applies the same rules to them alone.
 */
class SegmentService(
    private val api: MoonriseApi,
    /** Use the community databases' segments, which the server looks up (Settings). */
    private val useOnline: () -> Boolean,
    private val log: (String) -> Unit = {},
) {
    /** The final answer per title and version: once the server has said all it will. */
    private val cache = java.util.concurrent.ConcurrentHashMap<String, List<Segment>>()
    /** Titles (and versions) whose last answer was still waiting for the community databases. */
    private val lookingUp: MutableSet<String> = java.util.concurrent.ConcurrentHashMap.newKeySet()

    private fun key(item: Item, version: String?) = if (version != null) "${item.id}~$version" else item.id

    /** The server's answer for a title (and version) isn't final yet: worth asking again later. */
    fun awaitingServer(item: Item, version: String? = null): Boolean = cache[key(item, version)] == null

    /** The server was still looking the title up in the community databases: ask again shortly. */
    fun awaitingCommunity(item: Item, version: String? = null): Boolean = key(item, version) in lookingUp

    suspend fun segments(item: Item, version: String? = null, provisional: Boolean = false): List<Segment> {
        val k = key(item, version)
        cache[k]?.let { return it }
        val runtime = version?.let { v -> item.files.orEmpty().firstOrNull { it.id == v }?.duration } ?: item.runtime ?: 0.0
        val server = runCatching { api.segments(item.id, version, provisional = provisional, community = useOnline()) }
            .getOrElse { ServerSegments(reached = false) }
        val result = offered(server, runtime, provisional, item.isMovie)
        val credits = result.firstOrNull { it.kind == SegmentKind.Credits }
        log("$k runtime ${runtime.toInt()} credits ${credits?.let { "${it.start.toInt()}-${it.end.toInt()}" } ?: "none"}" +
            " (${if (server.merged) "merged" else "file only"}${if (server.pending) " pending" else ""}" +
            "${if (server.communityPending) " looking up" else ""}${if (!server.reached) " unreached" else ""})")
        if (server.communityPending) lookingUp += k else lookingUp -= k
        if (!server.pending && !server.communityPending && server.reached) cache[k] = result
        return result
    }

    companion object {
        /** While the server looks a title up in the community databases, asked again this often, this far apart. */
        const val LOOKUP_RETRIES = 3
        const val LOOKUP_RETRY_DELAY_MS = 5000L

        /**
         * When the answer still isn't final (the server waits for the community databases or the
         * file, or wasn't reached), the player asks again near the end: on entering the last
         * [NEAR_END_ASKS] seconds, once each. A film's credits can start minutes before the end.
         */
        val NEAR_END_ASKS = listOf(600.0, 120.0)

        /** How many of the near-end asks are due at `t` (each once): the player asks when this grows. */
        fun nearEndAsksDue(duration: Double, t: Double): Int =
            if (duration <= 0 || t <= 0 || t >= duration) 0 else NEAR_END_ASKS.count { duration - t <= it }

        /**
         * The segments to offer from the server's answer: as they are when it merged them, else the
         * file's own by the same rules (no community databases: the app asks no one else).
         */
        fun offered(server: ServerSegments, runtime: Double, provisional: Boolean, movie: Boolean): List<Segment> =
            if (server.merged) server.items
            else SegmentRules.merge(server.items, emptyList(), runtime, pending = server.pending && !provisional, movie = movie)
    }
}
