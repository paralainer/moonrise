package app.moonrise.core

import kotlinx.serialization.KSerializer
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.File

/**
 * The API contract's example answers (the `.json` files in `api/examples/` at the repository's root, each
 * `{summary, value}`) decode with this client's own models and its JSON settings
 * ([MoonriseApi.json]): a field renamed or retyped in the spec, or in a model, fails here.
 * Every example is either decoded or listed in [notUsed] with the reason, so a new example
 * fails until it is one or the other. Gradle passes the folder (`moonrise.api.examples`).
 */
class APIExamplesTest {
    private val dir: File = File(
        System.getProperty("moonrise.api.examples") ?: error("moonrise.api.examples isn't set (core/build.gradle.kts)"),
    )

    private fun <T> decodes(serializer: KSerializer<T>, check: (T) -> Unit = {}): (JsonObject) -> Unit = { value ->
        check(MoonriseApi.json.decodeFromJsonElement(serializer, value))
    }

    private val items = ListAnswer.serializer(Item.serializer())

    /** Example file → how this client decodes that answer. */
    private val decoded: Map<String, (JsonObject) -> Unit> = mapOf(
        "info.json" to decodes(ServerInfo.serializer()) { assertTrue(it.capabilities.isNotEmpty()) },
        "sign-in.json" to decodes(AuthResult.serializer()) { assertTrue(it.token.isNotEmpty()) },
        "me.json" to decodes(User.serializer()),
        "device-code.json" to decodes(DeviceCode.serializer()),
        "device-poll-approved.json" to decodes(PollAnswer.serializer()) { assertEquals("approved", it.status) },
        "libraries.json" to decodes(ListAnswer.serializer(Library.serializer())) { assertTrue(it.items.isNotEmpty()) },
        "facets.json" to decodes(LibraryFacets.serializer()),
        "items.json" to decodes(PageAnswer.serializer()) { assertTrue(it.items.isNotEmpty()) },
        "item-movie.json" to decodes(Item.serializer()),
        "item-episode.json" to decodes(Item.serializer()),
        "seasons.json" to decodes(items) { assertTrue(it.items.isNotEmpty()) },
        "episodes.json" to decodes(items) { assertTrue(it.items.isNotEmpty()) },
        "show-continue.json" to decodes(ContinueAnswer.serializer()),
        "search.json" to decodes(SearchResults.serializer()),
        "search-start.json" to decodes(SearchStart.serializer()) { assertTrue(it.recent.isNotEmpty() && it.suggestions.isNotEmpty()) },
        "home.json" to decodes(Home.serializer()),
        "segments.json" to decodes(SegmentsAnswer.serializer()),
        "segments-merged.json" to decodes(SegmentsAnswer.serializer()),
        "playback.json" to decodes(Playback.serializer()) {
            assertTrue(it.tracks?.sameFile == true && it.tracks?.audio?.index == 2 && it.tracks?.subtitle?.language == "fr")
        },
        "subtitles.json" to decodes(SubtitleList.serializer()) { assertTrue(it.subtitles.isNotEmpty()) },
        "subtitle-cues.json" to decodes(SubtitleWindow.serializer()),
        "subtitle-picture-cues.json" to decodes(SubtitleWindow.serializer()),
        "subtitle-timing.json" to decodes(SubtitleTiming.serializer()),
        "subtitle-search.json" to decodes(SubtitleSearch.serializer()),
        // An uploaded track: the same answer as a fetched one (`fetchSubtitle`).
        "subtitle-track.json" to decodes(SubtitleTrack.serializer()),
        "trickplay.json" to decodes(Thumbnails.serializer()),
        // The phone app's Manage Server, setup and Fix Match (`Admin.kt`).
        "admin-libraries.json" to decodes(LibraryList.serializer()) { assertTrue(it.items.isNotEmpty()) },
        "admin-library.json" to decodes(ManagedLibrary.serializer()),
        "admin-scan-progress.json" to decodes(ScanProgress.serializer()) { assertEquals("identifying", it.phase) },
        "admin-services.json" to decodes(ServiceList.serializer()) { assertTrue(it.services.isNotEmpty()) },
        "admin-system.json" to decodes(SystemInfo.serializer()) { assertTrue(it.transcode?.choices?.isNotEmpty() == true) },
        "admin-activity.json" to decodes(Activity.serializer()) { assertTrue(it.sessions.isNotEmpty()) },
        "admin-users.json" to decodes(ListAnswer.serializer(Account.serializer())) { assertTrue(it.items[0].me) },
        "admin-devices.json" to decodes(ListAnswer.serializer(AccountDevice.serializer())),
        "admin-invites.json" to decodes(ListAnswer.serializer(Invite.serializer())),
        "admin-invite.json" to decodes(Invite.serializer()) { assertEquals("ABCD-EFGH", it.code) },
        "admin-logs.json" to decodes(LogLines.serializer()),
        "admin-fs-browse.json" to decodes(FolderListing.serializer()),
        "admin-smb-discover.json" to decodes(FileServerList.serializer()),
        "admin-smb-shares.json" to decodes(ShareList.serializer()),
        "admin-smb-browse.json" to decodes(ShareListing.serializer()),
        "admin-smb-test.json" to decodes(ShareTest.serializer()) { assertTrue(it.ok) },
        "admin-review.json" to decodes(ReviewList.serializer()),
        "admin-match.json" to decodes(MatchInfo.serializer()),
        "admin-match-search.json" to decodes(MatchCandidateList.serializer()),
        "admin-match-search-server-posters.json" to decodes(MatchCandidateList.serializer()),
        "admin-match-episodes.json" to decodes(TMDBSeason.serializer()),
        "admin-identify.json" to decodes(IdentifyAnswer.serializer()),
        "admin-llm.json" to decodes(LLMOverview.serializer()) { assertTrue(it.presets.isNotEmpty()) },
        "admin-llm-models.json" to decodes(LLMModelList.serializer()),
        "admin-llm-test.json" to decodes(LLMTestResult.serializer()),
    )

    /** Examples of answers this client doesn't read, and why. */
    private val notUsed: Map<String, String> = mapOf(
        "latest.json" to "per-library Recently Added: the TV app uses the home screen's rows",
        "report.json" to "playback reports: the answer isn't read",
        "local-reports.json" to "no downloads on Android TV",
        "continue-hidden.json" to "Remove from Continue Watching: the answer isn't read",
        "played.json" to "played marks: the answer isn't read",
        "search-recent-recorded.json" to "a title opened from Search: the answer isn't read",
        "search-recent-cleared.json" to "clearing Search's Recent: the answer isn't read",
        "version.json" to "choosing a version: the answer isn't read",
        "download-job.json" to "no downloads on Android TV",
        "download-jobs.json" to "no downloads on Android TV",
        "offline.json" to "no downloads on Android TV",
        "setup-claim.json" to "the answer is an AuthResult, as sign-in.json",
    )

    /** The rest of the admin API (`admin-*.json`) the phone reads as JSON or not at all (ChatGPT sign-in, settings, credentials). */
    private fun isAdmin(name: String) = name.startsWith("admin-")

    @Test
    fun everyExampleDecodesOrIsListed() {
        val files = dir.listFiles { f -> f.name.endsWith(".json") }?.sortedBy { it.name }.orEmpty()
        assertTrue("no examples in $dir", files.size > 30)
        val failures = mutableListOf<String>()
        for (f in files) {
            val name = f.name
            val decode = decoded[name]
            when {
                decode != null -> {
                    val value = MoonriseApi.json.parseToJsonElement(f.readText()).jsonObject["value"]?.jsonObject
                    if (value == null) {
                        failures += "$name: no `value` object"
                        continue
                    }
                    runCatching { decode(value) }.onFailure { failures += "$name: ${it.message}" }
                }
                name in notUsed || isAdmin(name) -> {}
                else -> failures += "$name: not decoded and not listed as unused (APIExamplesTest)"
            }
        }
        val names = files.map { it.name }.toSet()
        (decoded.keys + notUsed.keys).filter { it !in names }.forEach { failures += "$it: listed, but there is no such example" }
        if (failures.isNotEmpty()) fail(failures.joinToString("\n"))
    }

    @Test
    fun aRenamedFieldFails() {
        // The check itself: an answer whose field was renamed (`server_id` → `serverId`) doesn't decode.
        val broken = MoonriseApi.json.parseToJsonElement("""{"serverId": "x", "name": "n", "version": "1"}""").jsonObject
        assertTrue(runCatching { decodes(ServerInfo.serializer())(broken) }.isFailure)
    }
}
