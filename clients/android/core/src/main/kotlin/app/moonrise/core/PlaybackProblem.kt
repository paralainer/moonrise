package app.moonrise.core

/**
 * A playback failure in plain words: a short [title], one [message] on what it means or
 * what to do, and the server's own text (ffprobe output, paths, codes) kept as [detail] for
 * Details. The rule is shared with the Apple and Windows apps: `api/playback-problem-rules.json`.
 */
data class PlaybackProblem(val kind: Kind, val detail: String?) {
    enum class Kind(val key: String) {
        Unreachable("unreachable"), Preparing("preparing"), Missing("missing"), Share("share"), Damaged("damaged"),
        Unindexed("unindexed"), Busy("busy"), Server("server"), Unsupported("unsupported"),
    }

    val title: String
        get() = when (kind) {
            Kind.Unreachable -> "Can't reach the server"
            Kind.Preparing -> "Still getting ready"
            Kind.Missing -> "File not found"
            Kind.Share -> "Can't reach the share"
            Kind.Damaged -> "Damaged file"
            Kind.Unindexed -> "Can't stream this file"
            Kind.Busy -> "Server busy"
            Kind.Server -> "Server problem"
            Kind.Unsupported -> "Can't play this video"
        }

    val message: String
        get() = when (kind) {
            Kind.Unreachable -> "Check that the Moonrise server is on and this device is on its network."
            Kind.Preparing -> "The server is still reading this file. Try again in a minute."
            Kind.Missing -> "The server can't find this video's file. It may have been moved or deleted, or its share is offline."
            Kind.Share -> "The server can't read the network share this video is on. Check that it's on and connected."
            Kind.Damaged -> "This file is damaged or incomplete, so it can't play. Replacing it fixes this."
            Kind.Unindexed -> "The file has no index to stream from. Saving it again (remuxing) fixes this."
            Kind.Busy -> "The server is converting other videos right now. Try again in a moment."
            Kind.Server -> "The server ran into a problem with this video. Try again."
            Kind.Unsupported -> "The server can't play this video on this device."
        }

    /** The title and its line, for places with room for one text only. */
    val text: String get() = "$title. $message"

    companion object {
        /**
         * - status: the HTTP status of the server's answer; null when nothing answered.
         * - text: its body, an offer's reasons, or a file's `broken_reason`.
         * - pending: an offer whose file is still being indexed.
         */
        fun classify(status: Int?, text: String, pending: Boolean = false): PlaybackProblem {
            val raw = text.trim()
            return PlaybackProblem(kind(status, raw, pending), raw.ifEmpty { null })
        }

        fun kind(status: Int?, text: String, pending: Boolean): Kind {
            if (status == null) return Kind.Unreachable
            if (pending) return Kind.Preparing
            val t = text.lowercase()
            fun has(vararg words: String) = words.any { t.contains(it) }
            return when {
                has("no such file or directory") -> Kind.Missing
                has("can't be reached", "not mounted", "isn't mounted") -> Kind.Share
                has("damaged", "incomplete", "truncated", "ebml", "invalid data found", "moov atom not found", "end of file") -> Kind.Damaged
                has("keyframe index pending") -> Kind.Preparing
                has("keyframe index", "no cues") -> Kind.Unindexed
                has("slots busy") -> Kind.Busy
                // (A title the server lists but can't play from any file it finds.)
                status == 404 -> Kind.Missing
                status >= 500 -> Kind.Server
                else -> Kind.Unsupported
            }
        }

        /** A title page's note for a file the server marked broken (its `broken_reason`). */
        fun broken(reason: String?): PlaybackProblem {
            val p = classify(422, reason ?: "")
            // A broken file is one the server couldn't read: damaged, unless the reason says otherwise.
            return if (p.kind == Kind.Unsupported) p.copy(kind = Kind.Damaged) else p
        }

        /** Any error from asking for a stream: an HTTP refusal, else no answer. */
        fun of(e: Throwable): PlaybackProblem = when (e) {
            is MoonriseHttpException -> classify(e.code, e.body)
            else -> classify(null, e.message ?: "")
        }

        /** An offer the server answered but can't serve (`supported: false`). */
        fun of(offer: Playback): PlaybackProblem = classify(200, offer.reasons.joinToString("; "), offer.pending == true)
    }
}

/**
 * Why a title can't play, for its page (null: it can, or nothing says): every file the server
 * lists is damaged, or none was found on disk. A title with a healthy version plays that one.
 */
fun Item.unplayableProblem(): PlaybackProblem? {
    val all = files.orEmpty()
    if (all.isEmpty()) return null
    val present = presentFiles
    if (present.isNotEmpty()) {
        return if (present.all { it.isBroken }) PlaybackProblem.broken(present.firstOrNull { it.brokenReason != null }?.brokenReason) else null
    }
    val names = all.mapNotNull { it.name?.takeIf(String::isNotEmpty) }
    return PlaybackProblem(PlaybackProblem.Kind.Missing, (names.joinToString("\n") + "\n").trimStart() + "Not found at the server's last scan.")
}
