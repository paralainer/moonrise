package app.moonrise.shared.player

import android.content.Context
import android.graphics.BitmapFactory
import androidx.annotation.OptIn
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.runtime.setValue
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.TrackSelectionOverride
import androidx.media3.common.Tracks
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.HttpDataSource
import androidx.media3.datasource.okhttp.OkHttpDataSource
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.hls.HlsMediaSource
import androidx.media3.exoplayer.trackselection.DefaultTrackSelector
import app.moonrise.core.AudioCandidate
import app.moonrise.core.ClientRules
import app.moonrise.core.ScreenAwake
import app.moonrise.core.DownloadRecord
import app.moonrise.core.DownloadState
import app.moonrise.core.LocalReport
import app.moonrise.core.Item
import app.moonrise.core.Languages
import app.moonrise.core.MediaFile
import app.moonrise.core.MoonriseApi
import app.moonrise.core.NextEpisodeRules
import app.moonrise.core.Playback
import app.moonrise.core.PlaybackProblem
import app.moonrise.core.unplayableProblem
import app.moonrise.core.Quality
import app.moonrise.core.RecoveryTries
import app.moonrise.core.ReportEvent
import app.moonrise.core.Segment
import app.moonrise.core.SegmentKind
import app.moonrise.core.SegmentRules
import app.moonrise.core.SegmentService
import app.moonrise.core.SkipMode
import app.moonrise.core.StreamChoice
import app.moonrise.core.StreamGiveUp
import app.moonrise.core.StreamSwitches
import app.moonrise.core.SubtitleCandidate
import app.moonrise.core.SubtitleDocument
import app.moonrise.core.SubtitleMemory
import app.moonrise.core.SubtitleParser
import app.moonrise.core.SubtitleTiming
import app.moonrise.core.Thumbnails
import app.moonrise.core.SubtitleTrack
import app.moonrise.core.SubtitleWindow
import app.moonrise.core.TrackChoice
import app.moonrise.core.TrackMemory
import app.moonrise.core.TrackPick
import app.moonrise.core.TrackRules
import app.moonrise.core.Versions
import app.moonrise.core.audioDetail
import app.moonrise.core.audioName
import app.moonrise.core.clockText
import app.moonrise.core.isAudio
import app.moonrise.core.isSubtitle
import app.moonrise.core.subtitleLabel
import app.moonrise.shared.AppSettings
import app.moonrise.shared.Log
import app.moonrise.shared.Session
import app.moonrise.shared.displayDolbyVision
import app.moonrise.shared.downloads.DownloadDataSource
import app.moonrise.shared.downloads.Downloads
import app.moonrise.shared.downloads.LocalReports
import app.moonrise.shared.downloads.LocalTrackMemory
import app.moonrise.shared.downloads.ProgressStore
import app.moonrise.shared.downloads.ServerReach
import app.moonrise.shared.multichannelOutput
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.util.Locale

private const val TAG = "MoonrisePlayer"

/** A choice in the Audio menu: the server's track index, the language, its details. */
data class AudioOption(val id: Int, val title: String, val detail: String?, val language: String?)

/** A choice in the Subtitles menu (`SubtitleOption`). */
data class SubtitleOption(
    val id: String,
    /** "Russian · NovGoblin · Forced": the menu's title is the first part, its detail the rest. */
    val title: String,
    val language: String?,
    /** A text track's file on the server (drawn by the app), or null: a bitmap track, burned in by the server. */
    val url: String?,
    /** The server's index of the track. */
    val trackIndex: Int,
) {
    val burnIn: Boolean get() = url == null
    val name: String get() = title.split(" · ").first()
    val detail: String? get() = title.split(" · ").drop(1).joinToString(" · ").ifEmpty { null }
}

/** Playback stopped and couldn't be brought back: what happened. */
data class StreamFailure(val title: String, val detail: String)

/**
 * The player's state and rules, as the Apple TV app's `PlayerModel` has them: the server's
 * session played through Media3; audio and subtitle choices (text subtitles drawn by the app, so
 * their delay can change; bitmap ones burned in by the server); quality and version; Skip Intro,
 * Skip Credits and Next Episode (`SegmentService`); playback reports; and recovery when the
 * stream is lost (the server restarted, the network dropped).
 *
 * Changed on the main thread only. Compose state, so the screen follows.
 */
@OptIn(UnstableApi::class)
class PlayerModel(
    private val context: Context,
    val api: MoonriseApi,
    item: Item,
    private var wantedVersion: String?,
    private val requestedStart: Double?,
    private val hdr: Boolean,
    /** Another episode is to play (Next Episode, the Episodes panel, the end of this one). */
    private val onPlayEpisode: (Item) -> Unit,
    /** The download this plays from the device, or null: the server's stream (`startLocal`). */
    download: DownloadRecord? = null,
) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    /** The audio output takes more than 2 channels: ask for lossless multichannel audio. */
    private val lossless = multichannelOutput(context)

    /** The display shows Dolby Vision and a decoder for it exists: ask for it (`dv=1`). */
    private val dolbyVision = displayDolbyVision(context, hdr)

    var item by mutableStateOf(item)
        private set
    /** The show of an episode (its age rating and genres), else the title. */
    var titleDetails by mutableStateOf<Item?>(if (item.isEpisode) null else item)
        private set
    var player by mutableStateOf<ExoPlayer?>(null)
        private set
    var status by mutableStateOf<String?>("Loading…")
        private set
    var error by mutableStateOf<String?>(null)
        private set
    /** What [error] says when it's the server's (or the network's) answer: its title, and the server's own text for Details. */
    var problem by mutableStateOf<PlaybackProblem?>(null)
        private set
    /** A short message over the video (a subtitle that couldn't load, a timing change). */
    var notice by mutableStateOf<String?>(null)
        private set
    var isPlaying by mutableStateOf(false)
        private set
    var buffering by mutableStateOf(true)
        private set
    var currentTime by mutableStateOf(0.0)
        private set
    var duration by mutableStateOf(item.runtime ?: 0.0)
        private set
    /** Played to the end (and not going on to another episode): the player goes. */
    var finished by mutableStateOf(false)
        private set
    // What the screen-on rule follows: the player's play-when-ready and whether it ended.
    private var playWhenReady by mutableStateOf(false)
    private var playbackEnded by mutableStateOf(false)
    /**
     * Keep the screen on: only while the video plays or is on its way to play; paused,
     * ended or failed lets the device's screen timeout or screensaver run.
     */
    val keepScreenOn: Boolean get() = ScreenAwake.hold(
        playWhenReady = playWhenReady,
        ended = playbackEnded || finished,
        failed = error != null || failure != null,
        recovering = recovering,
        resumesAfterRecovery = isPlaying,
        closed = player == null,
    )

    // Tracks
    var audioOptions by mutableStateOf<List<AudioOption>>(emptyList())
        private set
    var selectedAudio by mutableStateOf<Int?>(null)
        private set
    var subtitleOptions by mutableStateOf<List<SubtitleOption>>(emptyList())
        private set
    var selectedSubtitleId by mutableStateOf<String?>(null)
        private set
    var subtitleText by mutableStateOf<String?>(null)
        private set
    /** Seconds the selected subtitles are moved (later when positive). */
    var subtitleDelay by mutableStateOf(0.0)
        private set
    /** Bumped when a track's timing changes (the menus say it). */
    var timingRevision by mutableIntStateOf(0)
        private set
    private var serverSubtitles: List<SubtitleTrack>? = null
    private var subtitleDoc: SubtitleDocument? = null
    private var burnIndex: Int? = null

    /** The tracks the user picked last in this title, or in the show (the playback answer's `tracks`). */
    private var trackMemory: TrackMemory? = null
    /** The remembered audio track has been looked for (once, with the first answer). */
    private var audioRemembered = false
    /** The tracks the title starts with have been picked: reports may say which play. */
    private var tracksSettled = false

    // Quality and versions
    var versions by mutableStateOf(item.presentFiles)
        private set

    /** Scrubbing thumbnails of the file playing (server capability `trickplay`; null: none). */
    var thumbnails by mutableStateOf<Thumbnails?>(null)
        private set
    private val thumbnailSheets = mutableStateMapOf<String, ImageBitmap>()
    private val thumbnailLoads = mutableSetOf<String>()

    /** Where the thumbnail over the scrubber is shown: from the first skip press until a moment after the seek. */
    var previewTime by mutableStateOf<Double?>(null)
        private set
    private var previewJob: Job? = null

    var playingVersion by mutableStateOf<String?>(null)
        private set
    /** Plays a download from the device. */
    var local by mutableStateOf(download)
        private set
    private val viewing = java.util.UUID.randomUUID().toString()
    /** Stream restarts for local playback that hasn't played yet. */
    private var localRetries = 0
    private var localStall = 0
    var quality by mutableStateOf(Quality.of(AppSettings.streamingQuality))
        private set
    /** "Remux · Moonrise Server": how the server serves it (the info panel). */
    var playMethod by mutableStateOf("")
        private set
    /** What the server sends: codec, picture size and range (the info panel). */
    var videoLine by mutableStateOf<String?>(null)
        private set

    // Segments and the next episode
    var segments by mutableStateOf<List<Segment>>(emptyList())
        private set
    var activeSegment by mutableStateOf<Segment?>(null)
        private set
    /** 0…1 while an auto-skip countdown runs (in playback time, so pausing pauses it). */
    var autoSkipProgress by mutableStateOf<Double?>(null)
        private set
    /** The skip button shows for a few seconds on entering a segment, then only with the controls. */
    var segmentPromptVisible by mutableStateOf(false)
        private set
    var seriesEpisodes by mutableStateOf<List<Item>>(emptyList())
        private set
    private val handledSegments = mutableSetOf<String>()
    private var autoSkipFrom = 0.0
    private var promptJob: Job? = null
    private var segmentsPending = false
    private var segmentsAskedAt = Double.NEGATIVE_INFINITY
    /** Times the segments were asked for again while the server looked the title up in the community databases. */
    private var segmentsLookupRetries = 0
    /** The near-end asks made while the answer wasn't final ([SegmentService.nearEndAsksDue]). */
    private var segmentsNearEndAsks = 0
    private var loggedNextOffer = false
    private val segmentService = SegmentService(api, { AppSettings.onlineSegments }, log = { Log.i("MoonriseSegments") { it } })

    // Skips: a run of presses adds up to one target, and one seek goes there once they pause.
    var skipTarget by mutableStateOf<Double?>(null)
        private set
    private var skipOrigin: Double? = null
    private var skipJob: Job? = null
    val skipTotal: Double? get() = skipTarget?.let { t -> skipOrigin?.let { t - it } }
    /** The time the controls show: a pending skip's target, else where playback is. */
    val shownTime: Double get() = skipTarget ?: currentTime

    // Recovery of a lost stream
    var recovering by mutableStateOf(false)
        private set
    var failure by mutableStateOf<StreamFailure?>(null)
        private set
    private var recoveryJob: Job? = null
    private val recoveryTries = RecoveryTries()
    private var playedSinceRecovery = 0.0
    private var recoveryPosition = 0.0
    private var recoverPlaying = true
    private var everReady = false
    private var stuckSince: Long? = null
    private var lastTick = -1.0
    private var lastProbe = 0L

    /** The viewer's own play or pause (Media3 pausing itself on a stall doesn't count). */
    private var playIntent = true

    /** Quality and version picks on trial: what to go back to if this TV can't decode one. */
    private val switches = StreamSwitches()
    /** The name of the last quality or version picked, for the notice if it's refused. */
    private var pickedLabel = ""

    private var run: Run? = null
    private var ticker: Job? = null
    private var reportJob: Job? = null
    private var subtitleJob: Job? = null
    private var windowLoader: SubtitleWindowLoader? = null
    private var timingJob: Job? = null
    private var nudgeJob: Job? = null
    private var closed = false

    /** The player is stopped (the app went to the background): [open] brings it back. */
    val stopped: Boolean get() = run == null && !starting
    /** A download is about to start (asking for the server first). */
    private var starting = false
    /** Where playback is, or was when it stopped. */
    var position: Double? = requestedStart
        private set

    // ------------------------------------------------------------------ start and stop

    /** One playback session on the server. */
    private class Run {
        var session: String? = null
        var offer: Playback? = null
        var started = false
        var stopped = false
        @Volatile var closed = false
        var master: String? = null
        /** A download played from the device: no server session. */
        var local = false
    }

    /** Starts the title: its full item (tracks, ids), the show's episodes, the segments, then the session. */
    fun start() {
        val rec = local
        if (rec != null) {
            // An incomplete download plays differently with no server: ask first.
            if (rec.state == DownloadState.Completed) startLocal(rec)
            else {
                starting = true
                scope.launch { ServerReach.refresh(); starting = false; if (!closed) startLocal(rec) }
            }
            return
        }
        if (item.isEpisode && item.seriesId != null) scope.launch {
            runCatching { api.episodes(item.seriesId!!) }.onSuccess { seriesEpisodes = it }
            runCatching { api.item(item.seriesId!!) }.onSuccess { titleDetails = it }
        }
        scope.launch {
            runCatching { api.item(item.id) }.onSuccess { full ->
                item = full
                if (!full.isEpisode) titleDetails = full
                versions = full.presentFiles
                buildAudioOptions()
                buildSubtitleOptions()
            }
            loadSegments()
        }
        val start = ClientRules.startPosition(requestedStart, item, wantedVersion)
        open(start, autoplay = true)
    }

    // ------------------------------------------------------------------ a download, from the device

    private val serverReachable: Boolean get() = local?.let { ServerReach.canReach(it.serverId) } == true

    /** Whether Search Online can be offered: the server does the searching. */
    val canSearchSubtitles: Boolean get() = local == null || serverReachable

    /** A download's title page and tracks, as it saved them (`startLocal`). */
    private fun startLocal(rec: DownloadRecord) {
        trackMemory = LocalTrackMemory.load(rec.id)
        rec.item?.let { item = it; if (!it.isEpisode) titleDetails = it }
        rec.series?.let { titleDetails = it }
        wantedVersion = rec.mediaSourceId ?: wantedVersion
        versions = item.presentFiles
        segments = rec.segments.orEmpty()
        segmentsPending = rec.segmentsPending
        // The next episode: the server's list when it can be reached, else the downloaded ones.
        item.seriesId?.let { sid ->
            seriesEpisodes = Downloads.episodes(sid).mapNotNull { it.item }
            if (serverReachable) scope.launch { runCatching { api.episodes(sid) }.onSuccess { if (it.isNotEmpty()) seriesEpisodes = it } }
        }
        if (serverReachable && (segments.isEmpty() || rec.segmentsPending)) scope.launch { loadSegments() }
        open(requestedStart ?: resumePosition(rec), autoplay = true)
    }

    /** Where to resume a download: this device's position when reports are still waiting or the server can't be asked, else the server's. */
    private fun resumePosition(rec: DownloadRecord): Double? {
        val mine = ProgressStore.position(rec.itemId)
        if (LocalReports.hasPending(rec.itemId)) return mine
        if (!serverReachable) return mine ?: item.resumeSeconds
        return item.resumeSeconds ?: mine
    }

    private fun openLocal(start: Double?, autoplay: Boolean, keepSubtitle: Boolean, restart: Boolean) {
        val rec = local?.let { Downloads.record(it.id) ?: it } ?: return
        local = rec
        val old = run
        val r = Run().apply { this.local = true; started = old?.started == true; stopped = old?.stopped == true }
        run = r
        old?.closed = true
        position = start
        if (!restart) status = "Loading…"
        error = null
        problem = null
        buffering = true
        // Not complete and no server: the playlist ends where the download does.
        val growing = rec.state != DownloadState.Completed && !serverReachable && !restart
        var from = start
        if (growing) {
            val edge = Downloads.contiguousSeconds(rec)
            if (from != null && from > edge - 5) {
                from = maxOf(0.0, edge - 30)
                showNotice("Offline: downloaded up to ${clockText(edge)}")
            }
        }
        val uri = DownloadDataSource.masterUri(rec.id, growing)
        r.offer = Playback(supported = true, audio = rec.audioTracks.orEmpty(), session = null, duration = rec.runtime)
        r.master = uri
        rec.runtime?.takeIf { it > 0 }?.let { duration = it }
        val p = player ?: newPlayer().also { player = it }
        if (from != null && from > 1) currentTime = from
        playIntent = autoplay
        playMethod = "Offline · ${rec.downloadLabel}" + if (rec.state == DownloadState.Completed) "" else " · Downloading"
        Log.i(TAG) { "local ${rec.id} ${rec.downloadLabel} start ${from ?: 0.0} growing=$growing reachable=$serverReachable restart=$restart" }
        buildAudioOptions()
        val media = MediaItem.fromUri(uri)
        val startMs = ClientRules.startPositionMs(from)
        if (startMs != null) p.setMediaItem(media, startMs) else p.setMediaItem(media, true)
        p.playWhenReady = autoplay
        p.prepare()
        if (!keepSubtitle) {
            subtitleOptions = rec.subtitles.mapIndexed { i, s ->
                SubtitleOption("file:${s.file}", s.title, s.language, "file:" + Downloads.file(rec.id, s.file).path, app.moonrise.core.DownloadRules.trackIndex(s.file) ?: (-1 - i))
            }
            autoSelectSubtitle()
        }
        tracksSettled = true
        startTicker()
    }

    /** Opens a session at `start` and plays it (in the same player when there is one). */
    fun open(start: Double?, autoplay: Boolean, keepSubtitle: Boolean = false, restart: Boolean = false) {
        if (closed) return
        if (run != null && !restart) return
        if (local != null) { openLocal(start, autoplay, keepSubtitle, restart); return }
        val old = run
        val r = Run()
        run = r
        old?.let { o ->
            o.closed = true
            // The old session's stop report (where it was), then it closes, as when the player goes.
            val stopAt = if (o.started && !o.stopped) currentTime.also { o.stopped = true } else null
            val audio = selectedAudio
            val tracks = tracksPlaying()
            o.session?.let { sid ->
                Session.background.launch {
                    if (stopAt != null) {
                        Log.i(TAG) { "report stop at ${fmt(stopAt)} paused=true audio=$audio (old session)" }
                        runCatching { api.report(sid, ReportEvent.Stop, stopAt, true, audio, tracks = tracks) }.onFailure { Log.w(TAG, "report stop failed: ${it.message}") }
                    }
                    runCatching { api.closeSession(sid) }
                }
            }
        }
        position = start
        if (!restart) status = "Loading…"
        error = null
        problem = null
        buffering = true
        val p = player ?: newPlayer().also { player = it }
        if (start != null && start > 1) currentTime = start
        playIntent = autoplay
        val version = wantedVersion
        val q = quality.capped(versionFile()?.bitrate, versionFile()?.videoHeight).bitrate
        val burn = burnIndex
        Session.background.launch {
            // On the app's scope: the answer is awaited even if the player has gone, so a session
            // it opens is always closed.
            val result = runCatching { offer(r, start, version, q, burn) }
            val offer = result.getOrNull()
            val keep = withContext(Dispatchers.Main) {
                if (r.closed || closed) return@withContext false
                r.session = offer?.session
                r.offer = offer
                result.exceptionOrNull()?.let {
                    Log.e(TAG, "offer failed", it)
                    if (recovering) return@withContext true
                    showProblem(PlaybackProblem.of(it))
                }
                if (offer != null) played(r, p, offer, start, autoplay, keepSubtitle)
                true
            }
            val sid = offer?.session
            if (!keep && sid != null) {
                Log.i(TAG) { "player left before the server answered: closing its session" }
                runCatching { api.closeSession(sid) }
            }
        }
        startTicker()
    }

    private suspend fun offer(r: Run, start: Double?, version: String?, quality: Long?, burn: Int?): Playback {
        var p = api.playback(item.id, start, hdr, version, quality, burn, lossless, dolbyVision)
        var tries = 0
        while (!p.supported && p.pending == true && tries < 20 && !r.closed) {
            withContext(Dispatchers.Main) { if (!r.closed) status = "Preparing the file on the server…" }
            delay(3000)
            tries++
            p = api.playback(item.id, start, hdr, version, quality, burn, lossless, dolbyVision)
        }
        return p
    }

    /** Playback can't start: the problem's line (its title and the server's text with it). */
    private fun showProblem(p: PlaybackProblem) {
        // The title page knew why (a damaged file, or none found): its words, with its details.
        val known = item.unplayableProblem()
        val shown = if (known != null && (p.kind == PlaybackProblem.Kind.Missing || p.kind == PlaybackProblem.Kind.Damaged)) known else p
        problem = shown
        error = shown.message
    }

    private fun played(r: Run, p: ExoPlayer, offer: Playback, start: Double?, autoplay: Boolean, keepSubtitle: Boolean) {
        if (!offer.supported) {
            showProblem(PlaybackProblem.of(offer))
            return
        }
        val master = api.masterUrl(offer) ?: run { error = "The server gave no stream."; return }
        r.master = master
        offer.mediaSourceId?.let { playingVersion = it; wantedVersion = it }
        offer.duration?.takeIf { it > 0 }?.let { duration = it }
        playMethod = when (offer.method) {
            "transcode" -> (if (burnIndex != null) "Transcode + Subtitles" else "Transcode") + " · Moonrise Server"
            "convert" -> "Remux + Audio · Moonrise Server"
            else -> "Remux · Moonrise Server"
        }
        videoLine = offer.video?.let { v -> listOfNotNull(v.codec.uppercase(), v.height?.let { "${it}p" }, v.range.takeIf { it != "SDR" }).joinToString(" · ") }
        if (!keepSubtitle) trackMemory = offer.tracks
        Log.i(TAG) { "playing ${item.id} method=${offer.method} variant=${offer.variant} video=${offer.video?.codec}/${offer.video?.range} ${offer.video?.width}x${offer.video?.height} audio=${offer.audio.size} burn=$burnIndex start=${start ?: 0.0} autoplay=$autoplay" }
        buildAudioOptions()
        // The start goes with the item: a restart (burn-in, quality, version, recovery) loads from
        // there, never from 0 first (the server would start a transcode at segment 0, and the
        // clock would read 0 for a moment, entering an intro that starts there).
        val item = MediaItem.fromUri(master)
        val startMs = ClientRules.startPositionMs(start)
        if (startMs != null) p.setMediaItem(item, startMs) else p.setMediaItem(item, true)
        p.playWhenReady = autoplay
        p.prepare()
        offer.session?.let { sid -> scope.launch { loadThumbnails(sid) } }
        if (!keepSubtitle) {
            scope.launch {
                serverSubtitles = runCatching { api.subtitles(mediaId) }.getOrNull()
                buildSubtitleOptions()
                autoSelectSubtitle()
                tracksSettled = true
            }
        }
    }

    /** The id the server's subtitle routes take: the title, or the version playing when it has several. */
    private val mediaId: String
        get() = api.mediaId(item.id, if (versions.size > 1) playingVersion ?: wantedVersion else null)

    private fun versionFile(): MediaFile? = versions.firstOrNull { it.id == (playingVersion ?: wantedVersion) } ?: versions.firstOrNull()

    /**
     * Releases the player and closes the session, reporting the stop first (where it was,
     * paused) when playback had started. Safe to call twice.
     */
    fun stop(why: String) {
        val r = run ?: return
        run = null
        r.closed = true
        recoveryJob?.cancel()
        recovering = false
        val sid = r.session
        var stopAt: Pair<Double, Int?>? = null
        val tracks = tracksPlaying()
        player?.let { p ->
            if (p.currentMediaItem != null) {
                position = p.currentPosition / 1000.0
                if (r.local && r.started && !r.stopped) { r.stopped = true; localReport(ReportEvent.Stop, p.currentPosition / 1000.0) }
                if (sid != null && r.started && !r.stopped) {
                    r.stopped = true
                    stopAt = p.currentPosition / 1000.0 to selectedAudio
                }
            }
            p.release()
        }
        player = null
        isPlaying = false
        ticker?.cancel()
        reportJob?.cancel()
        val report = stopAt
        if (sid != null) Session.background.launch {
            // The stop goes before the session closes.
            if (report != null) {
                Log.i(TAG) { "report stop at ${fmt(report.first)} paused=true audio=${report.second}" }
                runCatching { api.report(sid, ReportEvent.Stop, report.first, true, report.second, tracks = tracks) }.onFailure { Log.w(TAG, "report stop failed: ${it.message}") }
            }
            runCatching { api.closeSession(sid) }
            // The home screen's row follows (a title finished leaves it, a new one joins).
            app.moonrise.shared.WatchHooks.changed("playback stopped")
        }
        Log.i(TAG) { "closed ($why) at ${position ?: 0.0}, session ${if (sid != null) "closed" else "not opened yet"}" }
    }

    /** The player goes for good. */
    fun close(why: String) {
        stop(why)
        closed = true
        scope.cancel()
        windowLoader?.stop()
    }

    // ------------------------------------------------------------------ the engine

    private fun newPlayer(): ExoPlayer {
        val http = OkHttpDataSource.Factory(api.http).apply {
            api.token?.let { setDefaultRequestProperties(mapOf("Authorization" to "Bearer $it")) }
        }
        val selector = DefaultTrackSelector(context).apply {
            // Subtitles are drawn by the app (their delay can change), not by Media3.
            parameters = buildUponParameters().setTrackTypeDisabled(C.TRACK_TYPE_TEXT, true).build()
        }
        // The device's own decoders first (and passthrough to a receiver that takes Dolby or
        // DTS); Media3's FFmpeg decoders for audio it can't play itself, e.g. AC-3 on a TV
        // without a Dolby decoder.
        val renderers = DefaultRenderersFactory(context).setExtensionRendererMode(DefaultRenderersFactory.EXTENSION_RENDERER_MODE_ON)
        // Test seam (debug builds): this TV "can't decode" the Original quality's video, to check
        // the fallback after a switch the TV can't play.
        if (app.moonrise.shared.PlayerSeams.refuseOriginalVideo) renderers.setMediaCodecSelector { mime, secure, tunneling ->
            if (mime.startsWith("video/") && quality == Quality.original) emptyList()
            else androidx.media3.exoplayer.mediacodec.MediaCodecSelector.DEFAULT.getDecoderInfos(mime, secure, tunneling)
        }
        val p = ExoPlayer.Builder(context, renderers)
            .setTrackSelector(selector)
            .setMediaSourceFactory(HlsMediaSource.Factory(if (local != null) DownloadDataSource.Factory() else http))
            .setLoadControl(StreamLoadControl())
            .build()
        playWhenReady = p.playWhenReady
        playbackEnded = false
        p.addListener(object : Player.Listener {
            override fun onPlaybackStateChanged(state: Int) {
                Log.i(TAG) { "state ${stateName(state)} at ${fmt(p.currentPosition / 1000.0)}" }
                playbackEnded = state == Player.STATE_ENDED
                buffering = state == Player.STATE_BUFFERING || state == Player.STATE_IDLE
                if (state == Player.STATE_READY) {
                    status = null
                    everReady = true
                    if (p.duration > 0) duration = p.duration / 1000.0
                    applyAudio()
                    if (recovering) finishRecovery()
                }
                if (state == Player.STATE_ENDED) ended(p)
            }

            override fun onIsPlayingChanged(playing: Boolean) {
                Log.i(TAG) { "playing=$playing at ${fmt(p.currentPosition / 1000.0)}" }
                if (recovering) return
                isPlaying = playing
                // Playing: the start report, once (as the Apple apps send it when playback starts).
                if (playing) run?.let { report(it, p, ReportEvent.Start, paused = false) }
            }

            override fun onPlayWhenReadyChanged(playWhenReady: Boolean, reason: Int) {
                this@PlayerModel.playWhenReady = playWhenReady
                if (reason == Player.PLAY_WHEN_READY_CHANGE_REASON_USER_REQUEST) playIntent = playWhenReady
            }

            override fun onRenderedFirstFrame() {
                Log.i(TAG) { "first frame at ${fmt(p.currentPosition / 1000.0)}" }
            }

            override fun onTracksChanged(tracks: Tracks) {
                tracks.groups.filter { it.isSelected }.forEach { g ->
                    for (i in 0 until g.length) if (g.isTrackSelected(i)) {
                        val f = g.getTrackFormat(i)
                        Log.i(TAG) { "selected ${trackType(g.type)}: ${f.label ?: "-"} lang=${f.language} ${f.sampleMimeType} ${f.codecs ?: ""} ${if (f.width > 0) "${f.width}x${f.height}" else ""}" }
                    }
                }
                // Video this TV has no decoder for is left out by Media3 (sound, no picture): after a
                // switch, back to what played; else say so.
                val video = tracks.groups.filter { it.type == C.TRACK_TYPE_VIDEO }
                if (video.isNotEmpty() && video.none { it.isSupported } && run != null) {
                    val what = video.first().getTrackFormat(0).let { "${it.sampleMimeType} ${it.codecs ?: ""} ${it.width}x${it.height}" }
                    Log.w(TAG, "no decoder on this TV for the video ($what)")
                    val target = switches.fallback(currentChoice())
                    if (target != null) fallBack(target, "no decoder for $what")
                    else if (error == null) {
                        p.playWhenReady = false
                        error = "This TV can't play this title's video. Try another version or quality."
                    }
                    return
                }
                val audio = tracks.groups.filter { it.type == C.TRACK_TYPE_AUDIO }
                Log.i(TAG) { "audio renditions: ${audio.size} (${audio.joinToString { "${it.getTrackFormat(0).label}/${it.getTrackFormat(0).language}${if (it.isSupported) "" else " unsupported"}" }})" }
                applyAudio()
            }

            override fun onPlayerError(e: PlaybackException) {
                Log.e(TAG, "playback error ${e.errorCodeName}", e)
                val http = generateSequence(e.cause) { it.cause }.filterIsInstance<HttpDataSource.InvalidResponseCodeException>().firstOrNull()
                val streamLost = e.errorCode in setOf(
                    PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_FAILED, PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_TIMEOUT,
                    PlaybackException.ERROR_CODE_IO_BAD_HTTP_STATUS, PlaybackException.ERROR_CODE_IO_UNSPECIFIED,
                    PlaybackException.ERROR_CODE_BEHIND_LIVE_WINDOW, PlaybackException.ERROR_CODE_TIMEOUT,
                )
                val decoder = isDecoderError(e)
                val target = if (decoder) switches.fallback(currentChoice()) else null
                when {
                    run == null -> {}
                    // A download that stopped playing (a segment that didn't arrive): from where it was, a few times.
                    run?.local == true -> {
                        if (localRetries++ < 3) openLocal(currentTime.takeIf { it > 0.5 } ?: position, playIntent, keepSubtitle = true, restart = true)
                        else error = "This download couldn't be played."
                    }
                    // A quality or version this TV can't decode: back to the one that played.
                    target != null -> fallBack(target, e.errorCodeName)
                    streamLost -> recover(if (http != null) "the stream answered ${http.responseCode}" else "the stream failed (${e.errorCodeName})", failure = true)
                    // It played: whatever stopped it (a track switch that stalled, a decoder that
                    // gave up), a new session at the position with the same choices brings it back.
                    // Errors that come back every time end in the give-up message, which says why.
                    everReady -> recover("playback failed (${e.errorCodeName})", failure = true, decoder = decoder)
                    else -> error = errorText(e)
                }
            }
        })
        return p
    }

    private fun ended(p: ExoPlayer) {
        run?.let { report(it, p, ReportEvent.Stop, paused = true) }
        isPlaying = false
        // Watched to the end (credits not skipped): on to the next episode.
        val next = nextEpisode
        if (AppSettings.autoplayNext && next != null) {
            Log.i("MoonriseNext") { "ended: next episode ${next.id}" }
            playEpisode(next)
        } else {
            finished = true
        }
    }

    /** Every quarter second: where playback is, the subtitles, the segments, the stream's health. */
    private fun startTicker() {
        if (ticker?.isActive == true) return
        ticker = scope.launch {
            var lastReportAt = System.currentTimeMillis()
            var lastLog = 0L
            var ticks = 0
            while (isActive) {
                delay(250)
                ticks++
                val p = player ?: continue
                if (!recovering) {
                    val t = p.currentPosition / 1000.0
                    if (p.currentMediaItem != null && p.playbackState != Player.STATE_IDLE) currentTime = t
                    if (p.duration > 0) duration = p.duration / 1000.0
                    updateSubtitle(currentTime)
                    windowLoader?.advance(currentTime - subtitleDelay)
                    updateSegment(currentTime)
                    // Not final yet (the community databases or the file still looked at, or the
                    // server not reached): ask again on entering the last 10 minutes and 2 minutes
                    // before the end (a film's credits wait for the databases' answer).
                    val nearEnd = SegmentService.nearEndAsksDue(duration, currentTime)
                    if (nearEnd > segmentsNearEndAsks && segmentService.awaitingServer(item, segmentsVersion)) {
                        segmentsNearEndAsks = nearEnd
                        scope.launch { loadSegments() }
                    }
                    if (NextEpisodeRules.refetchDue(segmentsPending, duration, currentTime, segmentsAskedAt)) {
                        segmentsAskedAt = currentTime
                        scope.launch { loadSegments() }
                    }
                    val offered = nextEpisodeOffered
                    if (offered != loggedNextOffer) {
                        loggedNextOffer = offered
                        Log.i("MoonriseNext") { "offered=${if (offered) "yes" else "no"} at ${fmt(currentTime)} of ${fmt(duration)} (credits ${segments.filter { it.kind == SegmentKind.Credits }.joinToString { "${fmt(it.start)}-${fmt(it.end)}${it.source?.let { s -> " $s" } ?: ""}" }})" }
                    }
                }
                val now = System.currentTimeMillis()
                if (now - lastLog >= 10_000) {
                    lastLog = now
                    val c = p.videoDecoderCounters
                    Log.i(TAG) { "position ${fmt(p.currentPosition / 1000.0)} playing=${p.isPlaying} rendered=${c?.renderedOutputBufferCount} dropped=${c?.droppedBufferCount}" }
                }
                if (now - lastReportAt >= 10_000) {
                    lastReportAt = now
                    if (p.isPlaying) run?.let { report(it, p, ReportEvent.Progress) }
                }
                if (ticks % 4 == 0) watchdogTick(p)
            }
        }
    }

    private fun report(r: Run, p: ExoPlayer, event: ReportEvent, paused: Boolean = !p.isPlaying) {
        if (!r.local && r.session == null) return
        when (event) {
            ReportEvent.Start -> { if (r.started) return; r.started = true }
            ReportEvent.Stop -> { if (!r.started || r.stopped) return; r.stopped = true }
            ReportEvent.Progress -> if (!r.started || r.stopped) return
        }
        if (r.local) { localReport(event, p.currentPosition / 1000.0); return }
        val sid = r.session ?: return
        val pos = p.currentPosition / 1000.0
        val audio = selectedAudio
        val credits = SegmentRules.creditsStart(segments)
        val tracks = tracksPlaying()
        Log.i(TAG) { "report ${event.path} at ${fmt(pos)} paused=$paused audio=$audio credits=${credits?.let { fmt(it) } ?: "-"} subtitle=${if (tracks == null) "-" else selectedSubtitleId ?: "off"}" }
        Session.background.launch {
            runCatching { api.report(sid, event, pos, paused, audio, credits = credits, tracks = tracks) }.onFailure { Log.w(TAG, "report ${event.path} failed: ${it.message}") }
        }
    }

    /** Kept on the device and sent when the server can be reached (`localReport`). */
    private fun localReport(event: ReportEvent, pos: Double) {
        val rec = local ?: return
        val d = if (duration > 0) duration else rec.runtime
        val tracks = if (event == ReportEvent.Start) null else tracksPlaying()
        tracks?.let { LocalTrackMemory.save(it, rec.id) }
        ProgressStore.save(rec.itemId, pos)
        LocalReports.add(LocalReport(rec.serverId, viewing, rec.itemId, event.path, pos, System.currentTimeMillis() / 1000, d,
            SegmentRules.creditsStart(segments), tracks, rec.mediaSourceId))
        Log.i(TAG) { "local report ${event.path} at ${fmt(pos)}" }
    }

    // ------------------------------------------------------------------ controls

    fun togglePlay() {
        if (failure != null) { retryStream(); return }
        if (recovering) {
            // Resumes as asked once the new stream plays.
            recoverPlaying = !recoverPlaying
            isPlaying = recoverPlaying
            return
        }
        val p = player ?: return
        if (p.playbackState == Player.STATE_ENDED) p.seekTo(0)
        val play = !(p.playWhenReady)
        playIntent = play
        p.playWhenReady = play
        if (!play) isPlaying = false
    }

    fun play(on: Boolean) {
        if (player?.playWhenReady != on) togglePlay()
    }

    fun seek(to: Double) {
        val target = to.coerceIn(0.0, maxOf(duration - 1, 0.0))
        stuckSince = null
        if (recovering) { recoveryPosition = target; currentTime = target; return }
        currentTime = target
        windowLoader?.advance(target - subtitleDelay)
        player?.seekTo((target * 1000).toLong())
    }

    /** A skip (left/right, the remote's rewind and fast forward): presses add up to one seek. */
    fun skip(delta: Double) {
        val base = skipTarget ?: currentTime
        val end = if (duration > 0) maxOf(duration - 1, 0.0) else Double.MAX_VALUE
        if (skipOrigin == null) skipOrigin = currentTime
        skipTarget = (base + delta).coerceIn(0.0, end)
        previewTime = skipTarget
        previewJob?.cancel()
        previewJob = scope.launch { delay(1500); previewTime = null }
        skipJob?.cancel()
        skipJob = scope.launch {
            delay(450)
            val target = skipTarget ?: return@launch
            Log.i("MoonriseSkip") { "seek to ${fmt(target)} (total ${"%+.0f".format(Locale.ROOT, skipTotal ?: 0.0)})" }
            skipTarget = null
            skipOrigin = null
            seek(target)
        }
    }

    fun clearNotice() { notice = null }

    private fun showNotice(text: String) {
        notice = text
        scope.launch { delay(4000); if (notice == text) notice = null }
    }

    // ------------------------------------------------------------------ segments

    /** The version whose segments are asked for: the one playing when the title has several. */
    private val segmentsVersion: String? get() = if (versions.size > 1) playingVersion ?: wantedVersion else null

    private suspend fun loadSegments() {
        if (local != null && !serverReachable) return
        val it = item
        val version = segmentsVersion
        val s = segmentService.segments(it, version)
        if (item.id != it.id) return
        val final = !segmentService.awaitingServer(it, version)
        if (s.isNotEmpty()) segments = s
        segmentsPending = it.isEpisode && !final
        // The server was still looking the title up in the community databases (it never waits
        // for them): ask again shortly, a few times, so Skip Intro is there early on.
        if (segmentService.awaitingCommunity(it, version) && segmentsLookupRetries < SegmentService.LOOKUP_RETRIES) {
            segmentsLookupRetries++
            scope.launch {
                delay(SegmentService.LOOKUP_RETRY_DELAY_MS)
                if (item.id == it.id) loadSegments()
            }
        }
    }

    private fun updateSegment(t: Double) {
        val seg = NextEpisodeRules.active(segments, t) { AppSettings.skipMode(it) }
        if (seg == activeSegment) {
            autoSkipProgress?.let { p ->
                val delay = maxOf(AppSettings.autoSkipDelay.toDouble(), 0.1)
                val progress = minOf(1.0, maxOf(p, (t - autoSkipFrom) / delay))
                if (progress >= 1) skipSegment() else autoSkipProgress = progress
            }
            return
        }
        activeSegment = seg
        autoSkipProgress = null
        promptJob?.cancel()
        if (seg == null) { segmentPromptVisible = false; return }
        // Only the first entry into a segment auto-skips. Seeking back into a segment you skipped
        // (or chose to watch) just offers the button.
        val firstTime = handledSegments.add(seg.id)
        Log.i("MoonriseSkip") { "in ${seg.kind.id} ${fmt(seg.start)}-${fmt(seg.end)} at ${fmt(t)} (${AppSettings.skipMode(seg.kind).id}${if (firstTime) "" else ", seen"})" }
        if (firstTime && AppSettings.skipMode(seg.kind) == SkipMode.Auto) {
            if (AppSettings.autoSkipDelay <= 0) { skipSegment(); return }
            autoSkipFrom = t
            autoSkipProgress = 0.0
        }
        segmentPromptVisible = true
        promptJob = scope.launch {
            delay(8000)
            if (autoSkipProgress == null) segmentPromptVisible = false
        }
    }

    fun skipSegment() {
        val seg = activeSegment ?: return
        val next = nextEpisode
        if (creditsSkipPlaysNext && next != null) {
            Log.i("MoonriseSkip") { "credits skipped: next episode" }
            playEpisode(next)
            return
        }
        Log.i("MoonriseSkip") { "skip ${seg.kind.id} to ${fmt(seg.end)}" }
        activeSegment = null
        autoSkipProgress = null
        segmentPromptVisible = false
        seek(seg.end)
    }

    /** Stop the countdown and watch the segment; the button stays a moment. */
    fun cancelAutoSkip() {
        autoSkipProgress = null
        promptJob?.cancel()
        promptJob = scope.launch { delay(5000); segmentPromptVisible = false }
    }

    /** The remote did something else while Skip had it: no countdown, no button. */
    fun dismissSegmentPrompt() {
        if (autoSkipProgress != null || segmentPromptVisible) Log.i("MoonriseSkip") { "keep watching" }
        autoSkipProgress = null
        promptJob?.cancel()
        segmentPromptVisible = false
    }

    val nextEpisode: Item? get() = NextEpisodeRules.next(seriesEpisodes, item)

    val nextEpisodeOffered: Boolean
        get() = NextEpisodeRules.offered(duration, currentTime, shownTime, activeSegment, segments, nextEpisode != null)

    val creditsSkipPlaysNext: Boolean get() = NextEpisodeRules.creditsSkipPlaysNext(activeSegment, duration, nextEpisode != null)

    fun playNextEpisode() {
        Log.i("MoonriseNext") { "next episode ${nextEpisode?.id ?: "-"}" }
        nextEpisode?.let { playEpisode(it) }
    }

    fun playEpisode(ep: Item) {
        if (closed) return
        onPlayEpisode(ep)
    }

    // ------------------------------------------------------------------ audio

    private fun buildAudioOptions() {
        val offered = run?.offer?.audio.orEmpty()
        val file = versionFile()
        val tracks = file?.tracks.orEmpty().filter { it.isAudio }
        val opts = if (tracks.isNotEmpty()) {
            tracks.filter { t -> offered.isEmpty() || offered.any { it.index == t.index } }
                .map { AudioOption(it.index, it.audioName, it.audioDetail, it.language) }
        } else {
            offered.map { AudioOption(it.index, app.moonrise.core.languageName(it.language) ?: it.name, it.name, it.language) }
        }
        if (opts.isEmpty()) return
        audioOptions = opts
        val candidates = if (tracks.isNotEmpty()) tracks.filter { t -> opts.any { it.id == t.index } }.map {
            AudioCandidate(it.index, it.language, it.isDefault == true, listOfNotNull(it.title, it.displayTitle).joinToString(" "))
        } else offered.map { AudioCandidate(it.index, it.language, it.default == true, it.name) }
        // The first answer: the track picked last in this title (or like it, in the show).
        val remembered = if (!audioRemembered && run?.offer != null) {
            audioRemembered = true
            TrackRules.rememberedAudio(candidates, trackMemory)
        } else null
        if (remembered != null) {
            selectedAudio = remembered
            Log.i(TAG) { "audio: track $remembered, picked last (same file: ${trackMemory?.sameFile})" }
        } else if (selectedAudio == null || opts.none { it.id == selectedAudio }) {
            // A new title: the language picked last, when its stream has it, else the title's default.
            selectedAudio = TrackRules.preferredAudio(candidates, AppSettings.audioLanguage)
                ?: offered.firstOrNull { it.default == true }?.index ?: candidates.firstOrNull { it.isDefault }?.id ?: opts.first().id
            Log.i(TAG) { "audio: track ${selectedAudio} of ${opts.map { it.id }} (preferred '${AppSettings.audioLanguage}')" }
        }
        applyAudio()
    }

    /** Plays the stream's rendition of the selected audio track (its name is the track's). */
    private fun applyAudio() {
        val p = player ?: return
        val id = selectedAudio ?: return
        val track = run?.offer?.audio?.firstOrNull { it.index == id } ?: return
        val name = track.name
        // Before the stream's tracks are known: its language, so the first rendition picked is
        // already the right one in most cases (no switch a moment after the start).
        if (p.currentTracks.groups.none { it.type == C.TRACK_TYPE_AUDIO }) {
            track.language?.let { lang ->
                if (p.trackSelectionParameters.preferredAudioLanguages.firstOrNull() != lang) {
                    p.trackSelectionParameters = p.trackSelectionParameters.buildUpon().setPreferredAudioLanguage(lang).build()
                }
            }
            return
        }
        val group = p.currentTracks.groups.firstOrNull { g -> g.type == C.TRACK_TYPE_AUDIO && (0 until g.length).any { g.getTrackFormat(it).label == name } } ?: return
        if (group.isSelected) return
        p.trackSelectionParameters = p.trackSelectionParameters.buildUpon()
            .setOverrideForType(TrackSelectionOverride(group.mediaTrackGroup, 0))
            .build()
        Log.i("MoonriseAudio") { "rendition '$name' for track $id" }
    }

    /** The viewer picked an audio track: play it, and its language from now on. */
    fun chooseAudio(id: Int) {
        if (id == selectedAudio) return
        selectedAudio = id
        applyAudio()
        audioOptions.firstOrNull { it.id == id }?.language?.let { Languages.normalized(it) }?.let {
            AppSettings.chooseAudioLanguage(it)
            Log.i(TAG) { "audio preference $it" }
        }
    }

    // ------------------------------------------------------------------ subtitles

    /**
     * The file's tracks (text ones from the server's files, bitmap ones burned in), then the
     * tracks the server fetched from OpenSubtitles.
     */
    private fun buildSubtitleOptions() {
        val file = versionFile()
        val server = serverSubtitles
        val byIndex = server.orEmpty().associateBy { it.index }
        val fileTracks = file?.tracks.orEmpty().filter { it.isSubtitle }
        val known = fileTracks.map { it.index }.toSet()
        val fromFile = fileTracks.mapNotNull { s ->
            if (s.isText != false) {
                val url = byIndex[s.index]?.url?.let { api.resolve(it) } ?: api.resolve("/v1/items/$mediaId/subtitles/${s.index}.srt") ?: return@mapNotNull null
                SubtitleOption("t:${s.index}", s.subtitleLabel, s.language, url, s.index)
            } else {
                SubtitleOption("t:${s.index}", s.subtitleLabel, s.language, null, s.index)
            }
        }
        // Fetched tracks (and, while the file's tracks aren't known, the server's whole list).
        val listed = server.orEmpty().filter { it.index !in known && (it.text || fileTracks.isEmpty()) }.mapNotNull { t ->
            val url = if (t.text) t.url?.let { api.resolve(it) } ?: return@mapNotNull null else null
            SubtitleOption("moonrise:${t.index}", t.name.replace(" - ", " · "), t.language, url, t.index)
        }
        subtitleOptions = fromFile + listed
        timingRevision++
        Log.i("MoonriseSubs") { "${subtitleOptions.size} options, ${if (server == null) "no server list" else "server files (${listed.size} more)"}" }
    }

    /**
     * A title starting: subtitles in the preferred language when they are on (the kind picked last
     * first: SDH or plain; a bitmap track, burned in, when there is no text one), else forced
     * subtitles for foreign dialogue in the audio's language.
     */
    private fun autoSelectSubtitle() {
        if (selectedSubtitleId != null) return
        // The subtitles picked last in this title (or Off), or like those picked last in the show.
        var on = AppSettings.autoSubtitles
        val candidates = subtitleOptions.map { SubtitleCandidate(it.id, it.title, it.language, it.trackIndex, it.burnIn) }
        when (val m = TrackRules.rememberedSubtitle(candidates, trackMemory)) {
            is SubtitleMemory.Track -> {
                Log.i("MoonriseSubs") { "picked last: ${m.id} (same file: ${trackMemory?.sameFile})" }
                subtitleOptions.firstOrNull { it.id == m.id }?.let { selectSubtitle(it); return }
            }
            SubtitleMemory.Off -> { Log.i("MoonriseSubs") { "picked last: off" }; return }
            SubtitleMemory.UsualOff -> on = false
            SubtitleMemory.Usual -> {}
        }
        val lang = Languages.normalized(AppSettings.subtitleLanguage) ?: AppSettings.subtitleLanguage.lowercase()
        fun matches(o: SubtitleOption, l: String) = subtitleLanguage(o) == l
        if (on) {
            val all = subtitleOptions.filter { matches(it, lang) && !TrackRules.isForced(it.title) }
            val text = all.filter { !it.burnIn }
            val pool = text.ifEmpty { all }
            val o = pool.firstOrNull { TrackRules.isSDH(it.title) == AppSettings.preferSDHSubtitles } ?: pool.firstOrNull()
            if (o != null) { selectSubtitle(o); return }
        }
        // Forced subtitles translate the foreign dialogue for viewers of the audio's language.
        val forced = subtitleOptions.filter { TrackRules.isForced(it.title) && !it.burnIn }
        val audioLang = audioOptions.firstOrNull { it.id == selectedAudio }?.language?.let { Languages.normalized(it) }
        val o = if (audioLang != null) forced.firstOrNull { matches(it, audioLang) } else forced.firstOrNull()
        o?.let { selectSubtitle(it) }
    }

    /** The option's language; for a file without one, the code in its name ("Film.en.srt"). */
    private fun subtitleLanguage(o: SubtitleOption): String? = TrackRules.subtitleLanguage(o.language, o.title)

    /** The audio and subtitle tracks playing, as the server keeps them; null until the title's are picked. */
    private fun tracksPlaying(): TrackChoice? {
        if (!tracksSettled) return null
        val audio = selectedAudio?.let { id ->
            val t = versionFile()?.tracks.orEmpty().firstOrNull { it.isAudio && it.index == id }
            val o = audioOptions.firstOrNull { it.id == id }
            TrackPick(index = id, language = Languages.normalized(t?.language ?: o?.language), title = t?.title ?: t?.displayTitle ?: o?.detail)
        }
        val sub = subtitleOptions.firstOrNull { it.id == selectedSubtitleId }?.let { o ->
            TrackPick(index = o.trackIndex, language = subtitleLanguage(o), title = TrackRules.subtitleDetail(o.title),
                forced = TrackRules.isForced(o.title), sdh = TrackRules.isSDH(o.title))
        }
        return TrackChoice(audio, sub)
    }

    /** The viewer picked subtitles (or Off): show them, and that language (and kind) from now on. */
    fun chooseSubtitle(o: SubtitleOption?) {
        selectSubtitle(o)
        if (o != null && !TrackRules.isForced(o.title)) {
            val lang = subtitleLanguage(o) ?: return
            AppSettings.chooseSubtitleLanguage(lang)
            AppSettings.choosePreferSDH(TrackRules.isSDH(o.title))
            AppSettings.chooseAutoSubtitles(true)
        } else {
            AppSettings.chooseAutoSubtitles(false)
        }
        Log.i(TAG) { "subtitle preference ${if (AppSettings.autoSubtitles) AppSettings.subtitleLanguage else "off"}${if (AppSettings.preferSDHSubtitles) " SDH" else ""}" }
    }

    fun selectSubtitle(o: SubtitleOption?) {
        Log.i("MoonriseSubs") { "selected ${o?.id ?: "off"} (${o?.title ?: ""})" }
        selectedSubtitleId = o?.id
        subtitleJob?.cancel()
        timingJob?.cancel()
        windowLoader?.stop()
        windowLoader = null
        subtitleDoc = null
        subtitleText = null
        restoreSubtitleDelay(o)
        val previousBurn = burnIndex
        burnIndex = null
        if (o == null) {
            if (previousBurn != null) reloadKeepingPosition()
            return
        }
        if (o.burnIn) {
            burnIndex = o.trackIndex
            reloadKeepingPosition()
            return
        }
        if (previousBurn != null) reloadKeepingPosition()
        val url = o.url ?: return
        val id = o.id
        subtitleJob = scope.launch {
            // An embedded track the server hasn't taken out of the file yet: windows at the playhead.
            if (startSubtitleWindows(o, url)) return@launch
            // A downloaded track: how its match stands first, so what is loaded is what the answer says.
            val pending = refreshTiming(o)
            if (selectedSubtitleId != id) return@launch
            loadWholeSubtitle(url, id)
            if (pending) watchTiming(o, url)
        }
    }

    private suspend fun loadWholeSubtitle(url: String, id: String) {
        try {
            val text = if (url.startsWith("file:")) withContext(Dispatchers.IO) { SubtitleParser.decode(java.io.File(url.removePrefix("file:")).readBytes()) } else api.fetchText(url)
            if (selectedSubtitleId != id) return
            subtitleDoc = SubtitleParser.parse(text, url.substringAfterLast('.', ""))
            Log.i("MoonriseSubs") { "loaded ${subtitleDoc?.cues?.size} cues for $id" }
            updateSubtitle(currentTime)
        } catch (e: Exception) {
            if (e is kotlinx.coroutines.CancellationException) throw e
            if (selectedSubtitleId == id) showNotice("Couldn't load subtitles: ${e.message}")
        }
    }

    private suspend fun startSubtitleWindows(o: SubtitleOption, url: String): Boolean {
        val track = serverSubtitles?.firstOrNull { it.index == o.trackIndex } ?: return false
        if (!track.embedded || !servesWindows()) return false
        val id = o.id
        if (selectedSubtitleId != id) return true
        val loader = SubtitleWindowLoader(
            api = api, mediaId = mediaId, index = o.trackIndex, scope = scope,
            fetchFull = { api.fetchText(url) },
            onCues = { cues ->
                if (selectedSubtitleId == id) {
                    subtitleDoc = SubtitleDocument(cues)
                    updateSubtitle(currentTime)
                }
            },
            onFallback = {
                if (selectedSubtitleId == id) {
                    windowLoader = null
                    subtitleJob = scope.launch { loadWholeSubtitle(url, id) }
                }
            },
        )
        windowLoader = loader
        loader.advance(currentTime - subtitleDelay)
        return true
    }

    private var capabilities: Set<String>? = null

    /** The thumbnails of the file a new session plays (kept when it is the same file). */
    private suspend fun loadThumbnails(sid: String) {
        val caps = capabilities ?: runCatching { api.info().capabilities.toSet() }.getOrNull()?.also { capabilities = it } ?: return
        if ("trickplay" !in caps) return
        val t = runCatching { api.thumbnails(mediaId, sid) }.onFailure { Log.w(TAG, "thumbnails: ${it.message}") }.getOrNull()?.takeIf { it.ready }
        Log.i(TAG) { "thumbnails: ${t?.let { "${it.count} every ${it.interval} s" } ?: "none"}" }
        if (t?.sheets != thumbnails?.sheets) thumbnails = t
    }

    /** A thumbnail sheet, or null while it is on its way (it is asked for then). */
    fun thumbnailSheet(n: Int): ImageBitmap? {
        val url = thumbnails?.sheets?.getOrNull(n)?.let { api.resolve(it) } ?: return null
        thumbnailSheets[url]?.let { return it }
        if (thumbnailLoads.add(url)) {
            scope.launch {
                val bitmap = runCatching { api.fetchBytes(url) }.getOrNull()?.let { b ->
                    withContext(Dispatchers.Default) { BitmapFactory.decodeByteArray(b, 0, b.size)?.asImageBitmap() }
                }
                if (bitmap != null) thumbnailSheets[url] = bitmap else thumbnailLoads.remove(url)
            }
        }
        return null
    }

    private suspend fun servesWindows(): Boolean {
        val caps = capabilities ?: runCatching { api.info().capabilities.toSet() }.getOrNull()?.also { capabilities = it } ?: return false
        return "subtitle-windows" in caps
    }

    /** Shows the cue for `t` (moved by the delay). */
    private fun updateSubtitle(t: Double) {
        val doc = subtitleDoc ?: return
        val text = doc.textAt(t - subtitleDelay)
        if (text != subtitleText) {
            subtitleText = text
            // What the overlay shows, for checks from logcat (the picture can't be captured).
            Log.d("MoonriseOverlay") { "${fmt(t)} ${text?.replace("\n", " / ") ?: "-"}" }
        }
    }

    // Timing: the server's match of a downloaded track, and the viewer's nudge.

    private fun timingTrack(o: SubtitleOption): SubtitleTrack? = serverSubtitles?.firstOrNull { it.index == o.trackIndex && it.timing != null }

    private fun setTiming(t: SubtitleTiming, index: Int) {
        serverSubtitles = serverSubtitles?.map { if (it.index == index) it.copy(timing = t) else it }
        timingRevision++
    }

    private val selectedOption: SubtitleOption? get() = subtitleOptions.firstOrNull { it.id == selectedSubtitleId }

    private val selectedTiming: SubtitleTiming? get() = selectedOption?.let { timingTrack(it)?.timing }

    /** The selected track's automatic match was applied (it can be switched off). */
    val subtitleSyncApplied: Boolean get() { timingRevision; return selectedTiming?.synced == true }
    val subtitleOriginalTiming: Boolean get() { timingRevision; return selectedTiming?.original == true }

    /** The menu's second line for a track: its details, or how its timing stands. */
    fun subtitleDetail(o: SubtitleOption): String? {
        timingRevision
        timingTrack(o)?.timing?.shortLabel?.let { return it }
        val twins = subtitleOptions.filter { it.name == o.name }
        return TrackRules.distinctDetails(twins.map { it.name to it.detail }).getOrNull(twins.indexOf(o)) ?: o.detail
    }

    fun nudgeSubtitles(by: Double) = applySubtitleDelay(subtitleDelay + by)

    fun resetSubtitleDelay() = applySubtitleDelay(0.0)

    private fun applySubtitleDelay(value: Double) {
        subtitleDelay = Math.round(value * 1000) / 1000.0
        updateSubtitle(currentTime)
        windowLoader?.advance(currentTime - subtitleDelay)
        val o = selectedOption ?: return
        Log.i("MoonriseSubs") { "delay ${"%+.1f".format(Locale.ROOT, subtitleDelay)} s for ${o.id}" }
        val track = timingTrack(o)
        if (track == null) {
            LocalNudges.set(context, "$mediaId|${o.title}", subtitleDelay)
            return
        }
        // Several presses in a row are one change.
        nudgeJob?.cancel()
        val (value2, media, index) = Triple(subtitleDelay, mediaId, track.index)
        nudgeJob = scope.launch {
            delay(600)
            runCatching { api.setSubtitleTiming(media, index, nudge = value2) }.onSuccess { setTiming(it, index) }
        }
    }

    /** The selected track's own nudge, when it is picked. */
    private fun restoreSubtitleDelay(o: SubtitleOption?) {
        nudgeJob?.cancel()
        if (o == null) return
        subtitleDelay = timingTrack(o)?.timing?.nudge ?: LocalNudges.get(context, "$mediaId|${o.title}")
    }

    /** Asks how a downloaded track's match stands (asking starts it). True: it is running. */
    private suspend fun refreshTiming(o: SubtitleOption): Boolean {
        val track = timingTrack(o) ?: return false
        if (!track.downloaded) return false
        val t = runCatching { api.subtitleTiming(mediaId, track.index) }.getOrNull() ?: return false
        setTiming(t, track.index)
        return t.pending
    }

    /** The selected track's match is running: when it is done, the track is loaded again. */
    private fun watchTiming(o: SubtitleOption, url: String) {
        timingJob?.cancel()
        val index = timingTrack(o)?.index ?: return
        val media = mediaId
        timingJob = scope.launch {
            val started = System.currentTimeMillis()
            while (isActive && System.currentTimeMillis() - started < 1_800_000) {
                delay(3000)
                if (selectedSubtitleId != o.id) return@launch
                val t = runCatching { api.subtitleTiming(media, index) }.getOrNull() ?: continue
                setTiming(t, index)
                if (t.pending) continue
                Log.i("MoonriseSubs") { "timing ${t.state} ${t.offset}" }
                if (t.synced && !t.original) {
                    loadWholeSubtitle(url, o.id)
                    t.label?.let { showNotice(it) }
                }
                return@launch
            }
        }
    }

    /** The viewer's choice between the automatic match and the file's original timing. */
    fun useOriginalSubtitleTiming(original: Boolean) {
        val o = selectedOption ?: return
        val url = o.url ?: return
        val track = timingTrack(o) ?: return
        val media = mediaId
        scope.launch {
            val t = runCatching { api.setSubtitleTiming(media, track.index, original = original) }.getOrNull()
            if (t == null) { showNotice("Couldn't change the subtitle timing"); return@launch }
            setTiming(t, track.index)
            if (selectedSubtitleId == o.id) loadWholeSubtitle(url, o.id)
        }
    }

    /** The server fetched a subtitle from OpenSubtitles: list the tracks again and select it. */
    fun subtitleFetched(track: SubtitleTrack) {
        val rec = local
        if (rec != null) {
            scope.launch {
                // A download plays only what it saved: the fetched track is saved with it.
                val url = track.url?.let { api.resolve(it) } ?: return@launch
                val bytes = runCatching { api.fetchBytes(url) }.getOrNull() ?: return@launch
                val s = Downloads.attachSubtitle(rec.itemId, track.name.replace(" - ", " · "), track.language, bytes) ?: return@launch
                val o = SubtitleOption("file:${s.file}", s.title, s.language, "file:" + Downloads.file(rec.id, s.file).path, track.index)
                subtitleOptions = subtitleOptions + o
                chooseSubtitle(o)
            }
            return
        }
        scope.launch {
            serverSubtitles = runCatching { api.subtitles(mediaId) }.getOrNull() ?: serverSubtitles
            buildSubtitleOptions()
            subtitleOptions.firstOrNull { it.id == "moonrise:${track.index}" || it.id == "t:${track.index}" }?.let { chooseSubtitle(it) }
        }
    }

    /** The context for Search Online: the title (an episode's show), and the id to search for. */
    val searchMediaId: String get() = mediaId

    /** Restart the stream where it is (burn-in, quality, version). */
    private fun reloadKeepingPosition() {
        if (recovering) return
        val t = currentTime
        Log.i(TAG) { "restarting the stream at ${fmt(t)}" }
        // Playing or paused as the viewer left it (not paused by an error on the way).
        open(t, autoplay = playIntent, keepSubtitle = true, restart = true)
    }

    // ------------------------------------------------------------------ quality and versions

    val qualityOptions: List<Quality> get() = versionFile().let { Quality.options(it?.bitrate, it?.videoHeight) }

    /** "≈ 2.3 GB" for a quality (Original: the file's size). */
    fun sizeLabel(q: Quality): String? {
        val file = versionFile()
        val bytes = if (q.bitrate == null) file?.size else (file?.duration ?: item.runtime)?.takeIf { it > 0 }?.let { (q.bitrate!! / 8.0 * it).toLong() }
        bytes ?: return null
        val gb = bytes / 1e9
        val text = if (gb >= 1) "%.1f GB".format(Locale.ROOT, gb) else "%.0f MB".format(Locale.ROOT, bytes / 1e6)
        return if (q.bitrate == null) text else "≈ $text"
    }

    fun chooseQuality(q: Quality) {
        if (q == quality) return
        switches.picked(from = currentChoice())
        pickedLabel = q.label
        quality = q
        AppSettings.chooseStreamingQuality(q.label)
        reloadKeepingPosition()
    }

    /** Another version of the title: it plays from the same position, and is the title's version from now on. */
    fun chooseVersion(id: String) {
        if (id == playingVersion || versions.none { it.id == id }) return
        switches.picked(from = currentChoice())
        pickedLabel = versions.firstOrNull { it.id == id }?.versionTitle ?: "this version"
        wantedVersion = id
        app.moonrise.shared.VersionPicks.picks[item.id] = id
        val itemId = item.id
        Session.background.launch { runCatching { api.chooseVersion(itemId, id) } }
        selectedAudio = null
        reloadKeepingPosition()
    }

    /**
     * The TV's decoder refused the stream of a quality or version just picked: back to the one
     * that played (and to it in Settings and on the server), where it was, playing or paused as
     * before, with a word on why.
     */
    private fun fallBack(target: StreamChoice, why: String) {
        val refused = currentChoice()
        switches.fellBack(refused)
        val t = skipTarget ?: (if (currentTime > 0.5) currentTime else position ?: 0.0)
        val previous = if (target.version != refused.version) {
            versions.firstOrNull { it.id == target.version }?.versionTitle ?: "the previous version"
        } else {
            target.quality.label
        }
        Log.i(TAG) { "this TV can't decode $pickedLabel ($why): back to $previous at ${fmt(t)}" }
        if (quality != target.quality) {
            quality = target.quality
            AppSettings.chooseStreamingQuality(target.quality.label)
        }
        if (wantedVersion != target.version) {
            wantedVersion = target.version
            val itemId = item.id
            val version = target.version
            if (version != null) {
                app.moonrise.shared.VersionPicks.picks[itemId] = version
                Session.background.launch { runCatching { api.chooseVersion(itemId, version) } }
            } else {
                app.moonrise.shared.VersionPicks.picks.remove(itemId)
            }
        }
        showNotice("This TV can't play $pickedLabel. Back to $previous.")
        open(t, autoplay = playIntent, keepSubtitle = true, restart = true)
    }

    /** The quality and version asked for now (a version's id once the server has said which). */
    private fun currentChoice() = StreamChoice(quality, wantedVersion ?: playingVersion)

    // ------------------------------------------------------------------ recovery

    // A server stream dies when the server restarts (its sessions answer 410), and stalls when the
    // network goes. Signals: the stream failing, and no progress while it should play (6 s: ask the
    // session; 20 s, or 40 s before it was ever ready: recover anyway). Recovery waits until the
    // server answers, opens a new session at the same position with the same audio, subtitles,
    // burn-in and quality, and plays it, paused or playing as before. It gives up after a minute
    // without the server, or when the stream keeps failing.

    private fun watchdogTick(p: ExoPlayer) {
        if (run == null || recovering || failure != null) { stuckSince = null; return }
        if (run?.local == true) { localWatch(p); return }
        val t = p.currentPosition / 1000.0
        val moving = kotlin.math.abs(t - lastTick) > 0.05
        lastTick = t
        val shouldPlay = playIntent && p.playbackState != Player.STATE_ENDED && error == null
        if (!shouldPlay || moving) {
            stuckSince = null
            if (moving && p.isPlaying) {
                playedSinceRecovery += 1
                if (playedSinceRecovery >= 15) recoveryTries.reset()
                // Only once this run's stream is loaded: until then the old one is still playing.
                if (run?.master != null && switches.played(currentChoice())) Log.i(TAG) { "switch to $pickedLabel plays" }
            }
            return
        }
        val since = stuckSince ?: System.currentTimeMillis().also { stuckSince = it }
        val stuck = (System.currentTimeMillis() - since) / 1000
        if (stuck >= (if (everReady) 20 else 40)) recover("no progress for $stuck s")
        else if (stuck >= 6) checkSession("no progress for $stuck s")
    }

    /** A download stalled (a segment on its way): a nudge, then a restart at the position (`watchStall`). */
    private fun localWatch(p: ExoPlayer) {
        val t = p.currentPosition / 1000.0
        val moving = kotlin.math.abs(t - lastTick) > 0.05
        lastTick = t
        val shouldPlay = playIntent && p.playbackState != Player.STATE_ENDED && error == null
        if (moving || !shouldPlay) {
            stuckSince = null
            if (moving && ++playedSinceRecovery >= 15) { localRetries = 0; localStall = 0 }
            return
        }
        val since = stuckSince ?: System.currentTimeMillis().also { stuckSince = it }
        if ((System.currentTimeMillis() - since) / 1000 < 12) return
        stuckSince = null
        Log.i(TAG) { "local stall ${localStall + 1} at ${fmt(t)}" }
        if (localStall++ < 3) p.seekTo(p.currentPosition)
        else {
            localStall = 0
            openLocal(t, playIntent, keepSubtitle = true, restart = true)
        }
    }

    private val probeHttp by lazy {
        api.http.newBuilder().connectTimeout(4, java.util.concurrent.TimeUnit.SECONDS).readTimeout(4, java.util.concurrent.TimeUnit.SECONDS).build()
    }

    /** Asks the session's master playlist whether the session is still there. */
    private fun checkSession(why: String) {
        val master = run?.master ?: return
        if (System.currentTimeMillis() - lastProbe < 3000) return
        lastProbe = System.currentTimeMillis()
        scope.launch {
            val code = withContext(Dispatchers.IO) {
                runCatching {
                    probeHttp.newCall(okhttp3.Request.Builder().url(master).build()).execute().use { it.code }
                }.getOrNull()
            }
            Log.i(TAG) { "stream check ($why): ${code ?: "unreachable"}" }
            if (run?.master != master || recovering) return@launch
            if (code == 410 || code == 404 || code == null) recover("the session is gone or unreachable ($why)")
        }
    }

    /** [decoder]: the failure that asks for this try was the TV's decoder refusing the stream. */
    private fun recover(why: String, failure: Boolean = false, decoder: Boolean = false) {
        if (run == null || recovering || run?.local == true) return
        val stop = recoveryTries.next(everReady, decoder)
        val p = player
        val pos = skipTarget ?: (if (currentTime > 0.5) currentTime else position ?: 0.0)
        // As the viewer left it: an error doesn't turn a pause into play.
        val playing = playIntent
        Log.i(TAG) { "recovery: $why at ${fmt(pos)}, ${if (playing) "playing" else "paused"}${if (failure) " (after an error)" else ""}, try ${recoveryTries.count}" }
        if (stop != null) {
            giveUp(
                when (stop) {
                    StreamGiveUp.CantDecode -> StreamFailure("Can't play this on this TV", "This TV can't decode ${decodedChoice()}. Pick another quality or version.")
                    StreamGiveUp.StreamKeepsFailing -> StreamFailure("Playback stopped", "The Moonrise server's stream keeps failing.")
                },
            )
            return
        }
        recoveryPosition = pos
        recoverPlaying = playing
        recovering = true
        stuckSince = null
        currentTime = pos
        isPlaying = playing
        buffering = true
        error = null
        problem = null
        p?.playWhenReady = false
        recoveryJob?.cancel()
        recoveryJob = scope.launch { runRecovery() }
    }

    private suspend fun runRecovery() {
        val began = System.currentTimeMillis()
        var wait = 1000L
        while (scope.isActive && recovering && run != null) {
            val reachable = withContext(Dispatchers.IO) { runCatching { api.info() }.isSuccess }
            if (!recovering) return
            if (reachable) {
                Log.i(TAG) { "recovery: server reachable, new session at ${fmt(recoveryPosition)}" }
                open(recoveryPosition, autoplay = false, keepSubtitle = true, restart = true)
                // Ready (finishRecovery) within 20 s, else try again.
                repeat(40) { if (recovering) delay(500) }
                if (recovering) {
                    recovering = false
                    recover("the new stream didn't start")
                }
                return
            }
            if (System.currentTimeMillis() - began >= 60_000) {
                val online = app.moonrise.shared.Network.online
                giveUp(StreamFailure(
                    if (online) "Can't reach the Moonrise server" else "You're offline",
                    "Playback stopped at ${clockText(recoveryPosition)}. " + if (online) "Check your connection." else "Connect to the internet.",
                ))
                return
            }
            delay(wait)
            wait = minOf((wait * 1.5).toLong(), 5000L)
        }
    }

    private fun finishRecovery() {
        recovering = false
        playedSinceRecovery = 0.0
        stuckSince = null
        val p = player ?: return
        if (kotlin.math.abs(p.currentPosition / 1000.0 - recoveryPosition) > 1) p.seekTo((recoveryPosition * 1000).toLong())
        p.playWhenReady = recoverPlaying
        playIntent = recoverPlaying
        isPlaying = recoverPlaying
        Log.i(TAG) { "recovered at ${fmt(recoveryPosition)}, ${if (recoverPlaying) "playing" else "paused"} (audio ${selectedAudio ?: "-"}, subtitle ${selectedSubtitleId ?: "none"}, quality ${quality.label})" }
    }

    private fun giveUp(f: StreamFailure) {
        recoveryJob = null
        recovering = false
        failure = f
        buffering = false
        isPlaying = false
        player?.playWhenReady = false
        Log.i(TAG) { "recovery: gave up: ${f.title}. ${f.detail}" }
    }

    /** The give-up message's Try Again: another minute, from where it stopped. */
    fun retryStream() {
        if (failure == null) return
        failure = null
        recoveryTries.reset()
        playIntent = true
        recover("retry")
    }

    /** What plays, in the decoder's give-up message: "this title at 1080p · 20 Mbps", "the 4K HDR version"… */
    private fun decodedChoice(): String {
        val v = if (versions.size > 1) versionFile()?.versionTitle else null
        return when {
            quality != Quality.original -> "this title at ${quality.label}"
            v != null -> "the $v version"
            else -> "this title's video or audio"
        }
    }

    companion object {
        /** The TV's decoders (or the container) refused the stream: no other session will do better. */
        fun isDecoderError(e: PlaybackException): Boolean =
            e.errorCode / 1000 == 4 || e.errorCode == PlaybackException.ERROR_CODE_PARSING_CONTAINER_UNSUPPORTED

        /** What the viewer reads when a title doesn't play at all (the code is in the log). */
        fun errorText(e: PlaybackException): String = when (e.errorCode / 1000) {
            4, 5 -> "This TV can't play this title's video or audio. Try another version or quality."
            else -> "This title couldn't be played. Try again, or pick another version or quality."
        }

        fun fmt(v: Double): String = "%.1f".format(Locale.ROOT, v)

        fun stateName(s: Int) = when (s) {
            Player.STATE_IDLE -> "idle"
            Player.STATE_BUFFERING -> "buffering"
            Player.STATE_READY -> "ready"
            Player.STATE_ENDED -> "ended"
            else -> "$s"
        }

        fun trackType(t: Int) = when (t) {
            C.TRACK_TYPE_VIDEO -> "video"
            C.TRACK_TYPE_AUDIO -> "audio"
            C.TRACK_TYPE_TEXT -> "text"
            else -> "type$t"
        }
    }
}

/** Nudges of tracks the server keeps no timing for, kept on this device (per title and track). */
private object LocalNudges {
    private fun prefs(c: Context) = c.getSharedPreferences("subtitleNudges", Context.MODE_PRIVATE)
    fun get(c: Context, key: String): Double = prefs(c).getFloat(key, 0f).toDouble()
    fun set(c: Context, key: String, v: Double) {
        prefs(c).edit().apply { if (v == 0.0) remove(key) else putFloat(key, v.toFloat()) }.apply()
    }
}

/**
 * An embedded subtitle track shown before the server has taken all of it out of the file:
 * fetched in windows of 30 s at the playhead and the one after it, one request at a time (a seek
 * drops a request for elsewhere). Once a window says the whole track is ready, the file is
 * fetched once and replaces the windows (`SubtitleWindowLoader`).
 */
private class SubtitleWindowLoader(
    private val api: MoonriseApi,
    private val mediaId: String,
    private val index: Int,
    private val scope: CoroutineScope,
    private val fetchFull: suspend () -> String,
    private val onCues: (List<app.moonrise.core.SubtitleCue>) -> Unit,
    private val onFallback: () -> Unit,
) {
    private val cues = linkedSetOf<app.moonrise.core.SubtitleCue>()
    private val loaded = mutableSetOf<Int>()
    private var last: Int? = null
    private var position = 0.0
    private var current: Int? = null
    private var job: Job? = null
    private var complete = false
    private var stopped = false

    fun stop() {
        stopped = true
        job?.cancel()
    }

    fun advance(t: Double) {
        position = maxOf(0.0, t)
        if (stopped || complete) return
        val k = (position / SubtitleWindow.SPAN).toInt()
        val needed = (k..k + 1).filter { w -> w !in loaded && (last?.let { w <= it } ?: true) }
        val next = needed.firstOrNull() ?: return
        current?.let { c ->
            if (c in needed) return
            job?.cancel()
        }
        fetch(next)
    }

    private fun fetch(k: Int) {
        current = k
        job = scope.launch {
            var wait = 500L
            while (isActive && !stopped) {
                try {
                    val w = api.subtitleWindow(mediaId, index, k)
                    if (stopped) return@launch
                    loaded += w.window
                    val before = cues.size
                    w.cues.forEach { c -> cues += app.moonrise.core.SubtitleCue(c.start, c.end, SubtitleParser.stripTags(c.text).trim()) }
                    Log.i("MoonriseSubs") { "subtitle window $k: ${w.cues.size} cues (${w.source ?: "?"})${if (w.complete) ", track complete" else ""}" }
                    if (cues.size != before) onCues(cues.sortedWith(compareBy({ it.start }, { it.end })))
                    if (w.complete) loadFull()
                    break
                } catch (e: app.moonrise.core.MoonriseHttpException) {
                    if (e.code == 404 && e.body.contains("past the end")) { last = k - 1; break }
                    if (e.code in setOf(404, 405, 410, 501, 503)) {
                        Log.i("MoonriseSubs") { "subtitle windows unavailable (${e.code}): the whole track instead" }
                        stop()
                        onFallback()
                        return@launch
                    }
                    delay(wait); wait = minOf(wait * 2, 4000)
                } catch (e: kotlinx.coroutines.CancellationException) {
                    throw e
                } catch (e: Exception) {
                    delay(wait); wait = minOf(wait * 2, 4000)
                }
            }
            if (current == k) {
                current = null
                advance(position)
            }
        }
    }

    private fun loadFull() {
        if (complete) return
        complete = true
        scope.launch {
            try {
                val doc = SubtitleParser.parse(fetchFull())
                if (stopped) return@launch
                Log.i("MoonriseSubs") { "subtitle track complete: ${doc.cues.size} cues" }
                job?.cancel()
                onCues(doc.cues)
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                if (stopped) return@launch
                complete = false
                advance(position)
            }
        }
    }
}
