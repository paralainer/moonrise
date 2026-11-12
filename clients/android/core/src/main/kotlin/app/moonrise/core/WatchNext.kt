package app.moonrise.core

import java.time.OffsetDateTime

// What the TV's home screen shows of this viewer's watching (Android TV's Watch Next row, "Continue
// watching" on Google TV): the Home tab's Continue Watching, kept in step with the server. Plain
// rules here (tested on the JVM); the TV app writes them to the system's TV provider.

/** Why a title is in the row: part-way through, or the next episode of a show being watched. */
enum class WatchNextKind { Continue, Next }

/** One title as the home screen should show it. Times in milliseconds. */
data class WatchNextEntry(
    val itemId: String,
    val kind: WatchNextKind,
    val isEpisode: Boolean,
    /** The film's title, or the show's name for an episode. */
    val title: String,
    val episodeTitle: String?,
    val seasonNumber: Int?,
    val episodeNumber: Int?,
    val description: String?,
    val year: Int?,
    /** The server's image id for the card. */
    val image: String?,
    /** The image is a 2:3 poster (no landscape artwork); else 16:9. */
    val posterImage: Boolean,
    val positionMs: Long,
    val durationMs: Long,
    val engagementMs: Long,
) {
    /**
     * A digest of what the home screen shows: a row whose stored fingerprint matches needs no
     * update. Kept in the row (the provider's internal data), so it survives the app restarting.
     */
    val fingerprint: String
        get() = listOf(
            kind, title, episodeTitle, seasonNumber, episodeNumber, description?.hashCode(), year, image, posterImage,
            positionMs / 1000, durationMs / 1000, engagementMs,
        ).joinToString("|")
}

/** A row this app has in the provider now. `browsable` false: the viewer removed it from the row. */
data class WatchNextRow(
    val rowId: Long,
    val itemId: String?,
    val engagementMs: Long,
    val browsable: Boolean,
    val fingerprint: String?,
)

/** A change to the provider's rows. */
sealed interface WatchNextOp {
    data class Insert(val entry: WatchNextEntry) : WatchNextOp
    data class Update(val rowId: Long, val entry: WatchNextEntry) : WatchNextOp
    data class Delete(val rowId: Long) : WatchNextOp
}

object WatchNextRules {
    /** At most this many titles in the row (the system's row is shared with other apps). */
    const val MAX = 20

    /**
     * The titles the row should hold, from `GET /v1/home`: its Continue Watching, in order, as the
     * Home tab lists it. Anything that can't play is left out, as is a title in progress with no
     * resume point (watched to the end).
     *
     * The time each was last engaged with orders the system's row (across apps): the server's
     * `continue.at` (when it was last played, or for a next episode when the previous one ended).
     * From a server before 0.24.0 (no `continue`): for a title in progress when it was last played;
     * for a next episode when it first showed up in the row (kept from `existing`), else `now`.
     */
    fun entries(home: Home, existing: List<WatchNextRow>, now: Long): List<WatchNextEntry> {
        val known = existing.filter { it.itemId != null }.associate { it.itemId!! to it.engagementMs }
        val out = mutableListOf<WatchNextEntry>()
        val seen = mutableSetOf<String>()
        val resumeIds = home.resume.map { it.id }.toSet()
        for ((i, item) in home.continueRow.withIndex()) {
            if (!item.isPlayable) continue
            val info = item.continueInfo
            val inProgress = (info?.reason ?: if (item.id in resumeIds) "resume" else "next") == "resume"
            val pos = if (inProgress) item.resumeSeconds else null
            if (inProgress && pos == null) continue
            // Before 0.24.0 a next episode already watched wasn't offered; since, the server offers
            // the episode right after a finished one even on a rewatch, and the row follows it.
            if (!inProgress && info == null && item.user?.played == true) continue
            val at = playedAt(info?.at) ?: (if (inProgress) playedAt(item.user?.lastPlayedAt) else null) ?: known[item.id] ?: (now - i * 1000L)
            val kind = if (inProgress) WatchNextKind.Continue else WatchNextKind.Next
            entry(item, kind, pos ?: 0.0, at)?.let { if (seen.add(it.itemId)) out += it }
        }
        return out.take(MAX)
    }

    private fun entry(item: Item, kind: WatchNextKind, position: Double, engagement: Long): WatchNextEntry? {
        val duration = item.runtime ?: item.presentFiles.firstOrNull()?.duration ?: 0.0
        // Landscape artwork as the Home tab's cards: an episode's still, else the title's backdrop.
        val landscape = item.images?.thumb ?: item.parentImages?.thumb ?: item.backdropImage
        val image = landscape ?: item.posterImage
        return WatchNextEntry(
            itemId = item.id,
            kind = kind,
            isEpisode = item.isEpisode,
            title = item.cardTitle,
            episodeTitle = if (item.isEpisode) item.title else null,
            seasonNumber = if (item.isEpisode) item.seasonNo else null,
            episodeNumber = if (item.isEpisode) item.episodeNo else null,
            description = item.overview?.takeIf { it.isNotBlank() },
            year = item.year,
            image = image,
            posterImage = landscape == null && image != null,
            positionMs = (position * 1000).toLong(),
            durationMs = (duration * 1000).toLong(),
            engagementMs = engagement,
        )
    }

    /** RFC 3339 → milliseconds, or null. */
    fun playedAt(value: String?): Long? = value?.let { runCatching { OffsetDateTime.parse(it).toInstant().toEpochMilli() }.getOrNull() }

    /**
     * The changes that make the provider's rows match `desired`:
     * - a title no longer wanted (finished, marked watched, or not on this server) is deleted;
     * - a new title is inserted; a changed one (position, artwork…) is updated in place;
     * - a row the viewer removed from the home screen stays removed (left as it is) until the title
     *   is watched again (engaged with after the removed row), when it comes back as a new row;
     * - duplicates for one title are deleted.
     */
    fun plan(desired: List<WatchNextEntry>, existing: List<WatchNextRow>): List<WatchNextOp> {
        val ops = mutableListOf<WatchNextOp>()
        val byItem = mutableMapOf<String, WatchNextRow>()
        for (row in existing.sortedByDescending { it.browsable }) {
            val id = row.itemId
            if (id == null || byItem.containsKey(id)) ops += WatchNextOp.Delete(row.rowId) else byItem[id] = row
        }
        val wanted = desired.associateBy { it.itemId }
        for ((id, row) in byItem) if (id !in wanted) ops += WatchNextOp.Delete(row.rowId)
        for (e in desired) {
            val row = byItem[e.itemId]
            when {
                row == null -> ops += WatchNextOp.Insert(e)
                !row.browsable -> if (e.engagementMs > row.engagementMs) {
                    ops += WatchNextOp.Delete(row.rowId)
                    ops += WatchNextOp.Insert(e)
                }
                row.fingerprint != e.fingerprint -> ops += WatchNextOp.Update(row.rowId, e)
            }
        }
        return ops
    }
}
