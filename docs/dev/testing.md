# Testing

## The rules

- **Every bug fix ships with a regression test** that fails without the fix. If the logic sits
  somewhere hard to test (a view, a player callback), move it into a small function or type
  first and test that. Add a row to the [regression map](#regression-map) below.
- **A refactor changes no behaviour, and proves it**: the same tests pass with the same names and
  count, and where output matters, the output is compared before and after (for the server: the
  master and media playlists, a segment's bytes, and item and home JSON from a throwaway server;
  for the apps: screenshots on the simulators).
- **Run `make check` before opening a pull request**, and say in it that it passed (or which part
  was skipped, e.g. the Apple parts without a build Mac). `make check-quick` skips the server
  image and the Apple builds while you iterate.
- **Tests are offline.** No production server, no devices, no network. Anything that needs one is
  a manual check, written down in the pull request.
- Where something can't be automated (device focus, AVKit's own UI, view lifecycle on rotation),
  say why in the regression map, and check it by hand in the **Dev app** (`DEV=1`), never in the
  app someone uses for watching.

## Where the tests are

| Part | Where | Run |
|---|---|---|
| Server (Rust) | `server/src/<file>.rs`, `mod tests` in each module | `scripts/rust-dev.sh cargo test --locked` (Docker), clippy with `-D warnings` |
| API contract | `server/src/api_contract_tests.rs`: every router route is in `api/openapi.yaml`, and only those; `scripts/api-lint.sh`: the spec is valid and its examples match its schemas; `APIExamplesTests.swift` (Apple), `APIExamplesTest.kt` (Android) and `clients/windows/Moonrise.Core.Tests/SharedCasesTests.cs` (Windows): every example in `api/examples/` decodes with the client's own models, or is listed as one that client doesn't use | `scripts/rust-dev.sh cargo test --locked`; `scripts/api-lint.sh` (both in `make check`); the client tests with the Apple and Android rows below |
| Server, Samba | `server/src/storage/smb_it.rs` (`#[ignore]`d, needs Samba in Docker) | `scripts/test-all.sh smb` |
| Android | `clients/android/core/src/test/kotlin/app/moonrise/core/` (JVM) | `clients/android/build.sh test` |
| Windows | `clients/windows/Moonrise.Core.Tests/` (shared rules, any OS) | `scripts/test-windows-core.sh` |
| Apple | `clients/apple/MoonriseTests/` (Swift Testing, hosted in the app) | `scripts/test-apple.sh` on the build Mac (a throwaway simulator); `ONLY=<Suite>` for one suite; `⌘U` in Xcode |
| Apple TV UI | `clients/apple/MoonriseTVUITests/` (XCUITest, the app's demo on a tvOS simulator): for what only the focus engine decides | `scripts/test-apple.sh` (the whole run; a throwaway tvOS simulator); `ONLY=TVHomeFocusUITests` for that suite alone |
| Scripts | `scripts/tests/*.test.sh` (scratch folders, ssh and the build Mac stubbed) | `scripts/tests/run.sh` |
| Docs | `scripts/tests/doc-links.test.sh`: every relative Markdown link resolves | `scripts/tests/run.sh` |
| No private ticket numbers | `scripts/tests/no-ticket-refs.test.sh`: no tracked file names an issue of the project's private tracker | `scripts/tests/run.sh` |
| README | `scripts/tests/readme.test.sh`: the server version it states is the one in `server/Cargo.toml`, no placeholders are left, its screenshots exist | `scripts/tests/run.sh` |
| App illustrations | `scripts/tests/art-credits.test.sh`: every JPEG in the apps' asset folders has its origin in `assets/brand/README.md` | `scripts/tests/run.sh` |
| Support link | `scripts/tests/support-link.test.sh`: the README and `site/index.html` link the Buy Me a Coffee page `.github/FUNDING.yml` names, and no other | `scripts/tests/run.sh` |
| Landing page publishing | `scripts/tests/pages-workflow.test.sh`: `pages.yml` starts only on a push to `main` touching `site/**` or by hand, uploads `site` and runs no commands; the other workflows stay manual only | `scripts/tests/run.sh` |

`scripts/test-all.sh` runs all of them (`make test`; `make test PARTS="server android"` for
some). The Apple part also compiles the macOS app, so a Mac-only break fails the run. Under XCTest
the app starts nothing (no sign-in, no network). The Apple tests are skipped with a note when no
build Mac can be reached.

`make check` (`scripts/check.sh`) adds the server image build, the iPhone and Apple TV simulator
builds, and the public-export scan (`scripts/export-public.sh`: gitleaks, private-detail patterns,
the built-in keys, metadata in images). A test value that looks like a key must be visibly fake,
or gitleaks will flag it. Its one exception, in `.gitleaks.toml`: `file_key:` lines in the server's
golden output (`server/testdata/golden/`), whose 16-hex-char hashes the generic-api-key rule takes
for keys.

### Shared rules, tested in the clients

The clients keep their decisions in small rule types so both can be tested alike. When you change
one, change the other and its tests:

| Rule | Apple | Android |
|---|---|---|
| Skip Intro / Skip Credits / Next Episode | `SegmentRules` (`SegmentRulesTests`, `SegmentRulesCasesTests` on `api/segment-rules.json`) | `Segments.kt` (`SegmentsTest`, `SegmentRulesCasesTest` on `api/segment-rules.json`) |
| Resume (any position above 0, watched before or not) | `MediaItem.resumeSeconds` (`ResumeRulesCasesTests` on `api/resume-rules.json`) | `Item.resumeSeconds` (`ResumeRulesCasesTest` on `api/resume-rules.json`) |
| Starting audio and subtitle tracks | `TrackRules` (`TrackRulesTests`) | `Tracks.kt` |
| Version choice | `VersionChoicesTests` | `BrowseAndWatchTest.versionRules` |
| How a playback failure is named (short title, plain line, the server's text as details), and a title no file of which can play | `PlaybackProblem` (`PlaybackProblemTests` on `api/playback-problem-rules.json`) | `PlaybackProblem.kt` (`PlaybackProblemCasesTest` on `api/playback-problem-rules.json`) |
| Continue Watching, reports | `ContinueWatchingTests` | `WatchNextTest`, `BrowseAndWatchTest` |
| Downloads and offline: progress and states, what's fetched next, the playlist of a partial download, offline title pages and Library, queued reports | `DownloadManager`, `SegmentedDownloads`, `OfflineTitlePage`, `LocalReports` | `Downloads.kt` (`DownloadsTest`) |
| Home's "nothing to show" message | `HomeStore.isEmpty` (`ContinueWatchingTests`) | `ClientRules.homeIsEmpty` (`ClientRulesTest`) |
| Retries, buffering, restarts | | `ClientRules.kt` (`ClientRulesTest`) |

## Regression map

Past bugs and the tests that guard them. Add a row with every fix, saying what broke.

### Server

| What broke | Test(s) |
|---|---|
| A title opened before the scan had probed its file (no bitrate in the library yet) was planned at the 1 Mb/s floor: its master advertised `BANDWIDTH=1640000` for a 12 Mb/s file, the ladder had no transcodes, and a 3 Mb/s quality limit kept the copy, for the 30 min the load is kept | `media/load_golden_tests.rs` `a_title_opened_before_the_scan_probed_it_has_its_real_bitrate`; `media/mod.rs` `a_files_bitrate_comes_from_the_library_else_its_probe_else_its_size` |
| A file with a large start time (a recording whose clock starts at 1001 s): the plan held its keyframes against the duration as if it were the end, so it got one segment or a first segment as long as the start time; copied audio and converted audio seeked past their place, embedded subtitles were off by the start time | `media/mod.rs` `a_file_with_a_large_start_time_is_planned_on_its_own_clock`, `a_file_starting_at_zero_keeps_its_plan`, `files_whose_plan_changes_get_new_segments`; `api.rs` `a_title_with_a_start_time_plays_on_its_own_timeline`; `subtitles/subs.rs` `embedded_cues_are_served_on_the_titles_timeline`; golden `audio/conversion_run` (a file starting at 1001 s) |
| A file with a large start time, once it was planned on its own clock: Matroska chapters (the file's clock) listed past the title's end, so no chapter skips, and a film with a credits chapter never played; Matroska thumbnails planned for the last timestamp (119 pictures for 180 s, 16 s apart from a 30000 s start); subtitle timing matched against the file's own track and audio on the file's clock (an in-sync download moved 1002.6 s) | `media/mod.rs` `a_files_own_clock_from_its_probe`; `library/api.rs` `chapters_of_a_file_with_a_start_time_are_on_its_timeline`; `userdata/mod.rs` `a_recording_with_a_start_time_is_played_at_its_credits_chapter`; `media/trickplay.rs` `a_file_with_a_start_time_gets_pictures_for_its_length`; `subtitles/subsync.rs` `a_file_with_a_start_time_is_matched_on_its_timeline` |
| MPEG-TS and files with a non-zero start time: a transcode run began a GOP late, or failed with "no keyframe at segment" (the seek asked past the keyframe, and ffmpeg adds the file's start time to `-ss`) | `media/mod.rs` `a_seek_is_asked_for_on_the_files_own_clock`, `a_seek_in_a_file_without_an_index_is_asked_for_before_its_keyframe`, `files_whose_runs_could_begin_late_get_new_segments`; `media/transcode.rs` `a_run_in_a_file_with_a_start_time_asks_on_the_files_own_clock`; golden `streams_by_order` |
| AVI, two keyframes in a row: a transcode run starting at the first began at the second, a frame late | `media/mod.rs` `a_seek_between_adjacent_keyframes_stays_on_the_first`, `a_seek_never_goes_before_its_boundary`, `seek_offsets` |
| x265 open-GOP Matroska: a cut came out empty | `media/segment.rs` `a_keyframe_decoded_early_stays_in_its_segment`, `every_keyframe_has_exactly_one_segment`, `a_run_is_checked_before_it_is_trusted`, `retries_widen_and_end`, `a_boundary_without_a_keyframe_repeats_the_fragment_holding_it`; `media/mp4.rs` `file_time_is_the_keyframes_presentation_time` |
| Segment cuts hung while subtitle extraction was paused (unpolled SMB reads held cache blocks) | `storage/smb_it.rs` `smb_stopped_reader_holds_nothing` (`scripts/test-all.sh smb`); `media/segment.rs` `a_silent_run_is_stopped_and_a_slow_one_is_not` |
| The stall limit killed slow audio runs; failed block reads were retried one by one | `media/segment.rs` `a_run_that_only_reads_is_not_stopped`; `storage/smb.rs` `a_failed_block_read_fails_everyone_waiting`. Memory growth from the allocator: measured under load only |
| An LLM sign-in token could be sent to another address; sign-out didn't revoke; ID tokens weren't verified | `llm/admin.rs` `chatgpt_address_is_fixed`, `chatgpt_token_stays_with_openai`, `no_key_reaches_a_catcher`; `llm/chatgpt.rs` `sign_out_revokes_and_keeps_the_registration`, `unconfirmed_revocation`, `id_token_signatures`, `refused_code_for_another_account` |
| A settings PATCH naming `llm.base_url` twice sent the stored key elsewhere | `admin/settings.rs` `strict_json`; `llm/keys.rs` `changes`; `llm/admin.rs` `keys_bind_to_the_configuration_in_effect`; `llm/mod.rs` `credentials_only_where_they_are_bound`, `redirects_are_not_followed`; `llm/chatgpt.rs` `token_and_revocation_redirects_are_not_followed` |
| Subtitle timing match damaged large correct stretches; false splits | `subtitles/subsync.rs` `a_cut_in_a_sampled_film_never_moves_the_part_that_was_right`, `no_part_is_moved_where_nothing_says_it_belongs`, `another_language_is_in_sync_a_little_apart`, `no_split_for_lines_the_detector_misses`, `fixes_a_small_offset_found_clearly` |
| The loopback secret and URL credentials appeared in logs | `admin/logbuf.rs` `scrub_urls`, `scrub_known_loopback_secret`, `redaction`, `tee_scrubs_before_writing` |
| A stop sent before a run listened was lost ("no transcode slot free") | `media/transcode.rs` `a_stop_sent_before_listening_is_seen`, `a_run_stopped_before_it_listens_frees_its_slot`, `a_stopped_prefetch_leaves_the_request_its_run` |
| AVI MPEG-4 with B-frames: runs started 1–3 frames late, also packed Xvid without pts | `media/scan.rs` `avi_keyframes_are_shown_when_the_next_picture_is_decoded`, `avi_packed_b_frames`, `avi_without_b_frames_or_without_any_pts`, `avi_delayed_without_any_pts` |
| A fake key in a test tripped gitleaks | The test value itself; gitleaks runs in `scripts/tests/export-public.test.sh` and in `scripts/export-public.sh` |
| Golden output's `file_key` hashes tripped gitleaks; `make check` failed | `.gitleaks.toml`; `scripts/tests/export-public.test.sh` (the golden `file_key` case) |
| Matroska with a large second SeekHead or damaged Cues: "no keyframe index" | `media/mkv.rs` `large_second_seek_head`, `zeroed_tail_is_an_error_not_garbage`, `seek_position_pointing_elsewhere_is_an_error`, `no_seek_head_cues_before_clusters`, `walk_fills_in_only_what_the_seek_head_leaves_out`, `reads_cues_with_void_crc_relative_and_second_track` |
| Damaged files looked playable, then every segment stalled | `media/mkv.rs` `read_failures_are_not_damage`; `media/index_backfill.rs` `damaged_files_wait_until_they_change` |
| Apps picked a damaged version; its badge came from the broken file | `library/mod.rs` `playback_skips_a_broken_version`; `library/api.rs` `badges_come_from_a_healthy_file`; Kotlin `BrowseAndWatchTest.versionRules`; Swift `VersionChoicesTests` |
| Copied AAC from MPEG-TS or AVI failed with "Malformed AAC bitstream" (`empty_moov` turns ffmpeg's own ADTS filter off) | `media/segment.rs` `adts_framed_aac_goes_through_the_filter_when_copied` |
| A video with no audio track (screen recording, clip with its sound removed) was refused in every container with 422 "no audio track playable as is"; offline sets and download jobs refused it too | `media/load_golden_tests.rs` `load_golden` (`mp4_no_audio`); `api.rs` `a_title_with_no_audio_track_plays_as_video_alone`; `offline.rs` `a_title_with_no_audio_track_is_kept_as_video_alone`; `downloads.rs` `a_title_with_no_audio_track_downloads_as_video_alone` |
| AVI, FLV or ASF with B-frames: a copied run from 0 (`-ss 0.000`, which ffmpeg asks for as 3/23 s before the start) began at the second keyframe | `media/segment.rs` `a_run_from_near_the_start_reads_from_the_files_start` |
| H.264 with 3-byte NAL lengths was refused by Apple players | `media/mod.rs` `short_nal_lengths`; `media/transcode.rs` `short_nal_lengths_are_rewritten_on_copy`; `media/segment.rs` `short_nal_lengths_are_rewritten_in_streamed_segments` |
| Next Episode and Skip Credits appeared during the story | `media/credits.rs` `credits_on_black_with_logos_after`, `dark_story_scenes_are_not_credits`, `short_or_early_dark_stretches_are_not_credits`, `the_video_may_end_before_the_file`; Kotlin `SegmentsTest`; Swift `SegmentRulesTests.neverEarlierThanADatabaseStart`, `nextEpisodeOfferedFromCreditsToTheEndElseTheLast30Seconds` |
| Many credit analyses ran at once | `media/credits.rs` `one_analysis_at_a_time_and_one_per_file`, `a_failed_file_waits` |
| Films marked watched before their credits; "en:End Credits" chapters not recognised | `userdata/mod.rs` `thresholds`, `credits_chapter`, `reported_credits`; `library/api.rs` `segment_names`; Swift `SegmentRulesTests.reportedCreditsStart`, `chapterNames`; Kotlin `SegmentsTest.reportedCreditsStartIsTheCreditsOffered`, `BrowseAndWatchTest.reportsAsTheAppleApps` |
| The segment rules moved to the server (`segments?merged=1`) must give what the apps' rules gave; community lookups paced, kept, never waited for; plain `/segments` unchanged | `media/segment_rules.rs` `shared_cases` (the same `api/segment-rules.json` cases as Swift and Kotlin), `ties_keep_the_first`, `a_chapter_never_starts_before_a_database_start`, `found_in_the_file_only_moves_later`; `library/community.rs` `queries_are_the_apps`, `answers_are_read_as_the_apps_read_them`, `refresh_rules`, `lookups_from_the_library`, `looked_up_once_kept_and_never_waited_for`, `switched_off_nothing_is_asked`; `library/api.rs` `merged_segments` |
| While the community lookup was pending, `segments?merged=1` offered a film's credits chapter before the database start (Guardians: 6796, IntroDB 6852) | `media/segment_rules.rs` `no_credits_while_the_community_lookup_is_pending`, `shared_cases` (the `community_pending` cases in `api/segment-rules.json`); `library/api.rs` `merged_segments` |
| Lossless 7.1 FLAC frames overflowed Android's 32 KiB decoder buffers | `media/audio.rs` `lossless_choices` (compile-time size check), `flac_framing`; `api.rs` `lossless_audio_only_when_asked`, `lossless_sources`. Bit-exactness against the source: checked by hand |
| Dolby Vision signalling | `media/mod.rs` `dolby_vision_records`; `media/mp4.rs` `dolby_vision_config_in_the_init`; `api_dv_tests.rs` `dolby_vision_only_for_players_that_show_it`, `dolby_vision_never_on_transcodes`; Android `MoonriseApiTest` `dolbyVisionNeedsTheDisplayAndADecoder`, `playbackAsksForDolbyVisionOnlyWhenTold`. The display switching: device only |
| A rewatch skipped ahead or dropped out of Continue Watching | `userdata/continuing.rs` `rewatches_carry_on`, `rewatch_does_not_skip_a_watched_episode`, `marks_and_rewatches`; Kotlin `WatchNextTest.rewatchOffersTheNextEpisodeAndFilmThoughWatched`, `BrowseAndWatchTest.rewatchResumeCardShowsProgress`; Swift `ContinueWatchingTests.topShelfCards` |
| MP4 thumbnails re-read the `moov` for every picture | `media/moov.rs` `keyframe_samples_and_decoder_configuration`; `media/trickplay.rs` `mp4_keyframes_become_annex_b_with_their_parameter_sets`, `a_picture_takes_the_keyframe_at_or_before_it` |
| The whole-file thumbnail pass could hang on a stderr flood and had no time limit | `media/trickplay.rs` `a_run_that_floods_stderr_never_hangs`, `a_run_is_given_up_after_its_limit_and_ended_when_switched_off` |
| A full thumbnail budget stopped even for a title being played | `media/trickplay.rs` `a_full_budget_deletes_the_sets_used_longest_ago` |
| Moving the old import's ids off `jf_map` must not change a match | `db/mod.rs` `match_hints_migration` (the migration reads ids as identification did); `library/golden_tests.rs` `match_hints_identify` (hints decide movie, show and episode, and follow renames), `identify_golden` (unchanged output). Checked once on a copy of a real library: every item identical before and after |
| A file whose video wasn't stream 0 got no thumbnails: the decode run asked its one-stream keyframe copy for the file's stream number | `media/trickplay.rs` `a_picture_decodes_the_only_stream_of_the_copied_keyframe`, `a_picture_decoded_from_the_file_keeps_the_files_stream` |
| Dolby Vision profile 5 thumbnails had wrong colours | `media/trickplay.rs` `dolby_vision_without_a_base_is_skipped_with_its_reason` |
| A quick scan where a folder and its parent both changed didn't mark files gone from that folder missing, so a rename there became a new file (new id, watch state left behind) | `library/golden_tests.rs` `quick_rename_with_parent_changed`, `scan_golden` |
| A file copied over another in place (`cp new old`, a file manager's Replace) keeps its folder's mtime, so quick passes didn't see it until the full pass hours later: a damaged file replaced that way stayed "damaged", and playback read the new file with the old probe | `library/golden_tests.rs` `file_replaced_in_place` |
| A new version of a title already in the library (a second file, found by a later scan) didn't bring it back to Recently Added or the Date Added sort: they read the title's own `added_at` only | `library/store.rs` `a_later_file_brings_a_title_back_to_recently_added`; `library/golden_tests.rs` `later_version_is_recently_added` (the first scan reorders nothing; a rename moves nothing) |
| Resuming a title didn't keep the audio and subtitle tracks picked in it (the server ignored them); the next episode gets the same language and kind | `userdata/tracks.rs` `the_same_file_resumes_with_its_tracks`, `the_next_episode_gets_the_shows_latest_choice`, `a_missing_subtitle_is_not_off`, `picks_are_kept_bounded`, `a_downloads_file`; `userdata/store.rs` `reports_keep_the_tracks` |
| TMDB and OpenSubtitles rate limiters could panic on a host booted under 250 ms ago (`Instant::now() - interval`) | `library/util.rs` `now_when_the_clock_is_younger_than_the_interval`, `an_interval_back_when_the_clock_reaches` |
| The Fix Match answers told apps to load posters from TMDB; the server's poster route takes only a TMDB file path and fixed sizes (never an address), for admins, caps the answer and follows no redirect, and answers change only with `server_posters=1` | `library/match_posters.rs` `refuses_non_admins_and_anything_but_a_tmdb_path`, `only_tmdb_file_paths_of_the_expected_form`, `only_a_tmdb_url_gives_a_path`, `serves_the_poster_and_asks_tmdb_once`, `a_bad_answer_is_a_failure_and_is_not_kept`, `an_answer_changes_only_when_asked` |
| A converted-audio segment finished between a request's cache miss and its search for a run started a second run there, rewriting the finished segment with different bytes (this is what made `conversion_run_golden` fail on a busy machine) | `media/audio.rs` `a_segment_made_meanwhile_starts_no_run`; `media/audio_run_golden_tests.rs` `conversion_run_golden` |
| A converted-audio segment that exists in the cache but can't be read used up every try and failed with HTTP 500 ("converted segment N not produced"); a new run now remakes it, while a file already replaced by another run still starts none | `media/audio.rs` `an_unreadable_segment_is_made_again`, `only_the_file_that_failed_counts_as_unreadable` |
| The test for the 4 MiB poster cap sent a body that wasn't an image, so it passed with no cap at all; a body that passes for a JPEG is now refused one byte over 4 MiB and served at it, with and without a Content-Length | `library/match_posters.rs` `a_bad_answer_is_a_failure_and_is_not_kept` |
| Two tests failed under load, with no bug in the server. `tail_and_levels` read the process-wide log buffer while `tee_scrubs_before_writing` wrote to it (they now take turns), and `detect_golden` ran a stand-in ffmpeg it had just written, which fails with "Text file busy" while a child started on another thread still holds the file open for writing (another process now writes it) | `admin/logbuf.rs` `tail_and_levels`; `media/detect_golden_tests.rs` `detect_golden` |
| Video Apple devices can't decode was copied and showed a black picture: H.264 lossless encodes at 4:2:0 (x264 `-qp 0` makes them High 4:4:4 Predictive, which VideoToolbox doesn't decode at all) and RGB (`libx264rgb`, `gbrp`), and HEVC range extensions (4:2:2 and 4:4:4 from cameras, 12-bit). They are now transcoded and decoded on the CPU, judged by profile as well as pixel format; 4:2:0 8/10-bit H.264 and HEVC keep their copy and CODECS | `media/mod.rs` `what_apple_devices_cannot_decode_is_transcoded`, `what_every_player_decodes_is_copied_with_the_codecs_it_had`; `media/transcode.rs` `h264_above_high_is_decoded_on_the_cpu` |

### Apple apps

| What broke | Test(s) or reason |
|---|---|
| The app called TheIntroDB, IntroDB, OMDb and OpenSubtitles itself; now it asks only its server (`segments?merged=1`), and an older server's answer gets the rules without databases | `SegmentRulesCasesTests.theSharedCases`, `ServerSegmentsTests.aMergedAnswerIsOfferedAsItIs`, `anOlderServersAnswerGetsTheRulesWithoutDatabases`; `RatingsTests.olderDeviceKeysAreForgotten`. That no other host is contacted: checked on simulators (connection sampling), not automated |
| No credits while the server waits for the community databases; an answer that isn't final is asked for again 10 and 2 minutes before the end | `SegmentRulesCasesTests.theSharedCases` (`community_pending` cases), `ServerSegmentsTests.askedAgainNearTheEnd` |
| Home was blank when the server had no rows | `ContinueWatchingTests.homeSaysSoOnlyWhenTheServerAnsweredWithNothing` |
| Top Shelf cards and deep links | `ContinueWatchingTests.topShelfCards`, `topShelfLinks` |
| Subtitles under the player's buttons (iPhone landscape and portrait, iPad, the title page's small player) | `SubtitlePlacementTests` |
| Mac player: a pillarboxed (4:3) video had its scrubber and top buttons over the black bars, not on the picture; the scrubber covered the second subtitle line | `PictureFrameTests` `pillarboxedControlsSitOnThePicture`, `letterboxedControlsSitOnThePicture`, `fillOrUnknownSizeIsTheWholeStage`, `neverNarrowerThanTheControlsFit`, `subtitlesLiftAboveTheBottomRowWithoutItsReport`, `aReportedBottomRowIsKept`. The look: a Mac running macOS 26 |
| Mac full-screen player: opening the episode list (or Find Subtitles) brought the window's toolbar (tabs, search, back) back over the video, and so did coming back from another app; the player stays full screen under its sheets | `PlayerChromeTests` `aSheetOverTheFullScreenPlayerKeepsTheToolbarHidden`, `registeringAgainKeepsTheFullScreen`, `leavingFullScreenOrThePlayerShowsTheToolbar`, `anOldPlayersLeavingChangesNothing`. The look, and switching apps: a Mac running macOS 26 |
| Mac full-screen player: the episode list didn't show (its sheet had no size of its own) | `PlayerChromeTests.episodeSheetHasASizeThatFitsTheSmallestWindow`. The look: a Mac running macOS 26 |
| iPad and Mac title page, playing: Download hung alone at the far end of the title column instead of after the title | `PictureFrameTests.downloadFollowsTheTitle` (`TitleAccessoryRow`) |
| Mac menus: the hovered row was a blank white box (AppKit draws it in the app's accent, which was white, with white text) | `scripts/tests/accent-color.test.sh` (macOS and tvOS builds use the system accent). The look: a Mac running macOS 26 |
| tvOS Info tab: missing artwork, genres and rating; a film's heading showed the year | `PlayerInfoFieldsTests` |
| Sign in with ChatGPT callback parsing | `ChatGPTCallbackTests` |
| Home didn't reload after a handoff from another device | `ContinueWatchingTests.homeReloadsWhenStale`, `localPlaysLayOverTheServersCards`, `legacyCards`; Kotlin `ClientRulesTest.homeReloadsWhenTheAppComesBack` |
| tvOS Home, Down from the banner went to the card nearest the banner's centre, not the first card of the row below | `TVHomeFocusUITests.testDownFromTheBannerFocusesTheFirstCard`; `ContinueWatchingTests.downFromTheBannerIsTheFirstCard` |
| tvOS: focus lost when a refresh moved a card, or after Remove from Continue Watching | `ContinueWatchingTests.threads`, `focusAfterRemovingAContinueCard`. The focus engine's timing: device only |
| tvOS audio and subtitle menus: dubs all named alike, details repeated or dropped | `TrackDetailTests` |
| Starting tracks: preferred audio language, forced and SDH subtitles | `TrackRulesTests` |
| Resuming didn't keep the audio and subtitle tracks (a second dub, SDH, a forced track, Off) picked in the title; downloads keep theirs on the device | `TrackRulesTests.resumeKeepsTheAudioTrack`, `resumeKeepsTheSubtitles`, `nextEpisodeAudio`, `nextEpisodeSubtitles`, `reportBody`, `downloadMemory`; `APIExamplesTests` (`playback.json` `tracks`) |
| A picture subtitle track and a text one of the same name read alike in the menus: they are named the same way, and rows that would read alike get their place | `TrackRulesTests.pictureTracksAreNamedLikeTextOnes` |
| A download played in its own screen (the iPhone's portrait player with an episode list, an offline episode grid) instead of the title page; with no server the page is made from what downloads saved | `DownloadPageTests` (the route, the offline page, the old screens gone). Playing offline with no error alerts: checked on a simulator with the server stopped |
| Mac kept awake while video plays | `KeepAwakeTests`; the real power assertion: `scripts/tests/keep-awake`, run natively by `scripts/test-apple.sh` |
| A paused video kept the display awake on iPhone, iPad and Apple TV (the idle timer was off whenever the player showed); it now follows the Mac's rule, through the same assertion | `KeepAwakeTests` `pauseReleasesAndResumeHoldsAgain`, `idleTimerFollowsPlayback`, `idleTimerIsSharedBetweenPlayers`. Apple TV simulator: pause and play with the remote |
| A macOS-only API broke the Mac build while the tests ran on iOS | `PlaybackQueryTests.macAsksForNeither`; `scripts/test-apple.sh` compiles the macOS app too |
| MoonriseTests didn't run after the build Mac moved to another Xcode (the test simulator's runtime and device type were fixed guesses) | `scripts/tests/sim-new.test.sh`: `scripts/sim-new.sh` picks an installed runtime and device type; `test-apple.sh` passes `platform=iOS Simulator,id=<UDID>` |
| Manage Server: scrubbing thumbnails progress and space used | `ManageThumbnailsTests` |
| iPad title page overflowed the 1 MB main-thread stack | Not automatable (simulators have 8 MB stacks). Guarded by device builds always using `-O` (`scripts/build.sh`) |
| tvOS focus and press handling in the system player's panels; view lifecycle on rotation | Not automatable: device or simulator only |
| Fix Match's candidate rows loaded posters straight from TMDB's image host | `FixMatchPosterTests`; `scripts/tests/no-third-party-art.test.sh` |
| A route change (home network and tailnet) reached the stream only once it had stalled, and a route that answered everything but the stream spun through one full wait after another | `StreamRouteRulesTests`; `RouteChangeTests` (a real AVPlayer and HLS stream over loopback routes that hang or refuse, both directions, with and without a route-change event; in `scripts/test-apple.sh`). A real Wi-Fi to cellular move and the embedded Tailscale node: on a device |
| A stream moved while paused gave up at once if a failure came soon after a pause longer than the give-up limit | `RouteChangeTests/pausedMove_thenTailnetDies_movesHomeAfterResume`, `pausedMove_thenServerRestarts_recoversAfterResume` |
| The forwarder left a player's connection silent while the node's dial blocked, kept threads and descriptors on dead tunnels, and a listener could stop accepting for good when out of descriptors | `PortForwarderTests` (the real `PortForwarder`, only the C dial replaced by `FakeTunnel`); end to end through it with a real AVPlayer: `RouteChangeTests/throughForwarder_*`. What the real node does on a real network change: on a device |
| A router pass could wait a full LocalAPI timeout on the node's status | `PortForwarderTests/firstValueGivesUpEvenOnWorkThatIgnoresCancellation` |
| Right after Play or autoplay the title page sometimes had no title, or still showed the Play row: SwiftUI redrew a view from the Play animation's old value (it is told of a change before the value is stored) and wasn't told again | `PageLaunchTests` (each group of `PageDrag` changes ends with every reader told once more). The page itself: repeated autoplay launches on a simulator |

### Android TV

| What broke | Test(s) or reason |
|---|---|
| The app called TheIntroDB and IntroDB itself; now the server's merged segments, asked again while it looks a title up | `SegmentRulesCasesTest.theSharedCases`, `SegmentsTest.serviceAsksForTheMergedAnswerAndCachesOnlyFinalOnes`, `anOlderServersAnswerGetsTheRulesWithoutDatabases`. That no other host is contacted: checked in the emulator through a logging proxy, not automated |
| No credits while the server waits for the community databases; an answer that isn't final is asked for again 10 and 2 minutes before the end | `SegmentRulesCasesTest.theSharedCases` (`community_pending` cases), `askedAgainNearTheEnd` |
| Resuming didn't keep the audio and subtitle tracks picked in the title; the next episode gets the same language and kind | `TrackMemoryTest` (the same cases as the Apple apps'); `APIExamplesTest` (`playback.json` `tracks`) |
| A picture subtitle track and a text one of the same name read alike in the menus (TV and phone): rows that would read alike get their place | `TrackMemoryTest.rowsThatReadAlikeGetTheirPlace` |
| Card captions were one line | `MoonriseApiTest.cardLabelsAsOnAppleTv` |
| Retries didn't back off | `ClientRulesTest.retriesBackOff` |
| An audio switch during a burned-in stream got stuck buffering | `ClientRulesTest.loadingGoesOnWhileATrackHasRunDry` |
| Every restart loaded from 0, then seeked | `ClientRulesTest.restartsBeginAtThePosition` |
| Home said "Nothing here yet." before the first answer, not what the other apps say | `ClientRulesTest.homeSaysSoOnlyWhenTheServerAnsweredWithNothing` |
| Home bobbed on every move along a row | `ClientRulesTest.aLiftedCardIsShownAtItsRestingBounds` |
| The 30-minute Watch Next sync downloaded every newest-titles row; it now asks for the watching rows only | `MoonriseApiTest.theWatchNextSyncAsksOnlyForWatchingRows`; server: `library::api::tests::home_can_skip_the_latest_rows` |
| A quick A → B → A left the fallback on A, so a refused A "went back" to itself and ended on the can't-play screen | `StreamRecoveryTest.switchingBackQuicklyFallsBackToTheChoiceThatPlayedNotTheRefusedOne`, the rest of `StreamRecoveryTest`. A real decoder refusal: on a device |
| A decoder error followed by stalls gave up blaming the decoder | `StreamRecoveryTest.stallsAfterADecoderErrorDontBlameTheDecoder`, `aDecoderErrorThatUsesTheLastTryBlamesTheDecoder` |
| Focus after Back, keyboard on focus, the player kept running on Home | Not automatable: Compose for TV focus and Activity lifecycle, on a device |
| A paused video kept the screen on (the player view's keep-screen-on was always set); now only while it plays or buffers toward play, on the TV and the phone | `ScreenAwakeTest` (core). Window flag playing, paused, seeking, ended: on an emulator (`dumpsys window`) |

### Android phone

| What broke | Test(s) or reason |
|---|---|
| The sign-in code wrapped onto a second line on a 411 dp wide phone | `ClientRulesTest.theSignInCodeFitsOnOneLine` |
| Library titles were cut off on a 600 dp wide tablet ("Mov...", a genre with Back as ".."), with no room beside Sort and Filter | `LayoutMetricsTest.theTitleHasItsOwnRowWhenThePageIsNarrow`, `aTitleShrinksToFitThenStops` |
| Beside the sidebar on a 600 dp wide tablet the Library grid kept 4 across, with 68 dp posters smaller than the phone's and captions cut | `LayoutMetricsTest.aNarrowLibraryPageShowsThreeAcrossNotFourTinyPosters` |
| In a 600 dp tall window the Filter panel and the Sort menu didn't fit under their buttons, and the system moved them up over the button and the tabs | `LayoutMetricsTest.theFilterPanelFitsBelowItsButton` (the system's placement: real device) |
| Home was blank when the server had no rows | `ClientRulesTest.homeSaysSoOnlyWhenTheServerAnsweredWithNothing` |
| Switching tabs while Home loaded logged a failure | `ClientRulesTest.aCancelledLoadIsNotAFailure` |
| Leaving the Library during a load logged a failure; Search and sign-in treated a cancelled load as one too | `ClientRulesTest.everyPhoneScreenTreatsACancelledLoadAsACancellation` |
| Swiping to cancel an invitation could cancel a different one (the swipe state keeps its first action, and the rows had no key) | `AdminRulesTest.aSwipeRemovesTheRowItIsOn` (scans `SwipeToRemove` and the invitation rows). The swipe itself: on a device |
| A swipe row at the top of a form card (an invitation, a library folder) squared off the card's rounded corners: the card painted its shape without clipping its rows | `AdminRulesTest.aFormCardClipsItsRowsToItsRoundedCorners` (scans `FormSection` and `SwipeToRemove`). How the corners look: on a device |
| In Fix Match, the search run by a switch of "This Is" used the kind from before the switch (an episode taken for a movie listed TV shows, and a pick sent a show's id as a movie), and an answer from before a switch could still land | `AdminRulesTest.aSwitchSearchesTheKindSwitchedTo`, `aSwitchBackToItsOwnKindDoesNotSearch`, `onlyAMovieSearchesMovies`, `anAnswerFromBeforeASwitchIsDropped` (`FixMatchRules.switchType`, `LatestSearch`), `theFixMatchPageRunsTheRulesSearch` (scans the page's wiring). With real TMDB results: on a device |
| Deleting a download while a file was still being fetched could make its folder again and leave it behind | `DownloadDeleteTest` (`DownloadRules.writingInto`; the delete's order and the writers in `Downloads.kt` are scanned). On a device: not run |
| Manage Server's summary stayed stale on the way back from a child page (the page under the top one stays composed, and its load ran once) | `AdminRulesTest.manageServerReloadsItsSummaryWhenItShowsAgain` (scans `PageStack.Host` and the page's effect). The return itself: on a device |
| A new invitation read "expires in 1 wk." and a minute later "in 6 days" (the phone compared whole seconds; the iPhone's formatter does not) | `AdminRulesTest.aMomentLessThanAWeekIsSixDays` |
| Deleting a download while its artwork was being fetched ended the app (the write threw in a job with nothing to catch it); a full disk would too | `DownloadDeleteTest.artworkArrivingAfterTheDeleteEndsQuietly`, `artworkTheServerDoesNotHaveWritesNothing`, `artworkOfADownloadStillThereIsKept` (`DownloadRules.saveArtwork`), `artworkIsOnlyWrittenThroughTheRule` (scans `Downloads.kt`). On a device: not run |
| Enter in the Server field opened Sign In with a Code; it now checks the address | Not automatable: Compose focus and the keyboard's action, on a device |
| Fix Match's candidate rows showed only a placeholder (TMDB's host isn't the server); posters now come from the server, and nothing else is loaded | `AdminRulesTest.aMatchPosterIsOnlyEverLoadedFromTheServer`, `aCandidateWithOnlyATmdbAddressHasNoPoster`; `scripts/tests/no-third-party-art.test.sh` |

### Scripts

| What broke | Test(s) |
|---|---|
| The export said OK when the key helper failed; tracing printed key values; the scan printed matching lines; it died early with many hits | `scripts/tests/export-public.test.sh` |
| Scripts exited on a fresh clone | `scripts/tests/fresh-clone.test.sh` |
| A device install script exited 0 when the install failed | `scripts/tests/deploy-install.test.sh` |
| The pre-push hook let a push to `main` or a private detail through | `scripts/tests/git-hooks.test.sh` |
| A missing personal patterns file was skipped without a word, also when `MOONRISE_PRIVATE_PATTERNS` named it | `scripts/tests/git-hooks.test.sh`, `scripts/tests/export-public.test.sh` |

### Windows app

| What broke | Test(s) or reason |
|---|---|
| A second render of a show page threw "already the child of another element" (Season change, an episode card, Mark Watched), and the page failed after coming back from the player | `ClientSourceTests.ARenderNeverHandsALongLivedElementToANewParent`, `TheEpisodeRowIsBuiltForEachRender`. In the app: on the VM |
| Home's hero showed whichever backdrop downloaded last | `ReviewRegressionTests.AHeroPictureThatArrivesLateDoesNotCoverTheCurrentOne`, `AHeroPictureAlreadyArrivedShowsWhenItsTurnComes`, `ClientSourceTests.TheHeroDecidesWhichPictureShows` |
| After a click on the picture, or a full screen change, the keyboard did nothing; Space with focus on a control pressed it as well | `ClientSourceTests.ThePlayerKeepsTheKeyboardAfterAnyClick` (scan). The real keyboard: on the VM |
| A title watched before ignored its resume point | `ReviewRegressionTests.ATitleWatchedBeforeStillResumes`, `AWatchedTitleShowsABarOnlyAsAContinueWatchingResume` |
| The public-export scan failed on a language-code API name and on metadata in the icon PNGs | `ReviewRegressionTests.LanguagesNormalizeWithoutTheSystemsCultures`, and the export scan in `make check-quick` |
| M left the HUD unchanged; the wheel moved a row sideways; no room under the top bar on a Home without a hero; re-clicking a tab stayed on the page | `ClientSourceTests.TheHudShowsTheMutedState`, `TheVerticalWheelMovesThePageNotARow`, `HomeWithoutAHeroLeavesRoomForTheTopBar`, `ReviewRegressionTests.ClickingTheSelectedTabGoesToItsRoot` |
| The title page's ratings were the show's in plain text, not the focused episode's as the Mac app has them | `ReviewRegressionTests.RatingsAreTheFocusedEpisodesWithTheShowsCriticsAdded`, `RatingsAreLeftOutWhenThereAreNone` |
| A window width change on the Library page crashed the app: a card's content, still another element's child, was given to a button again (the same "already the child" failure as the show page) | `ClientSourceTests.AnElementIsPlacedOnce` (fields, call results, elements read back out of a parent, and panels), `TheScanCatchesEveryFormOfIt`, `BuiltOnceMethodsAreOnlyCalledFromTheConstructor`, `PagesOnTheStacksAreShownThroughOneDoor`, `AResizeChangesPosterCardsInPlace`. Resizing every page: on the VM |
| The Library grid was always one column short: cells carry the gap after each poster, so n cells needed 12 more than the width available; the same in Search and the details grid | `ReviewRegressionTests.TheLibraryGridIsTheMacs`, `AllColumnsFitInThePanelTheGridIsGiven`, `ClientSourceTests.WrapGridsGetThePanelWidthThatHoldsTheirColumns` |
| Mark as Watched left the Library card's badge stale until a reload; Continue Watching didn't follow what was just watched | `ReviewRegressionTests.EachPageHearsOfAWatchChangeOnce`, `ClientSourceTests.WatchChangesReachTheCardsThatShowThem` |
| An error in a XAML handler, a dispatcher callback, a timer or an async event handler ended the process (0xc000027b) | `ClientSourceTests.AMissedErrorIsLoggedAndShownNotTheEndOfTheApp`, `DispatcherCallbacksAndTimersGoThroughTheGuard`, `TheMpvThreadSurvivesAnError`, `ReviewRegressionTests.AnErrorRepeatingOnEveryFrameIsShownOnlyAFewTimes`. Each path with an injected failure: on the VM |
| Picking Sort or Filter while a page was loading left the grid empty | `ReviewRegressionTests.AReloadWhileARequestRunsStillLoadsTheNewList`, `AnAnswerForAnOldListIsIgnored`, `PagingStopsAtTheEndAndOnAnEmptyAnswer`, `AFailedPageCanBeTriedAgain` |
| A stream that stopped (the server gone) counted as the title finishing, and went on to the next episode | `ReviewRegressionTests.AStreamThatStopsEarlyIsNotTheTitleFinishing`, `ClientSourceTests.AnEarlyEndOfStreamGoesThroughRecovery` |
| The window couldn't be moved while the player showed: the title bar's element was collapsed and the whole row given to the player | `ReviewRegressionTests.ThePlayersTitleRowMovesTheWindowBetweenItsControls`, `ClientSourceTests.TheWindowStillMovesWhileThePlayerIsOpen`. Dragging the row: on the VM |
| The second click of a double-click on Play paused the title at 0:00 (or pressed a control under the pointer) | `ClientSourceTests.TheClickThatOpenedThePlayerDoesNothingOnIt`. The double-click itself: on the VM |
| Every close of the player left the page, its swap chain and its buffers alive (a timer's Tick held the page from the native side, and the panel held the swap chain) | `ClientSourceTests.ClosingThePlayerLeavesNothingHoldingItAlive`. Object counts over open and close cycles: a gcdump on the VM |
| An error before the window existed was logged and the process stayed running with nothing on the screen | `ClientSourceTests.AnErrorBeforeThereIsAWindowEndsTheProcess`. A failure injected into the window's constructor: on the VM |
| A second launch ran a second copy sharing the log and the sign-in; it now brings the running copy forward | `ClientSourceTests.ASecondLaunchBringsTheRunningCopyForward`. Launching twice: on the VM |
| A long title on the title page was cut with no ellipsis; files under 50 MB read "0.0 GB" | `ClientSourceTests.LongTitlesOnTheTitlePageEndInAnEllipsis`, `ReviewRegressionTests.FileSizesReadInTheUnitThatFits` |
| Home's hero title was cut with no ellipsis | `ClientSourceTests.ALongHeroTitleEndsInAnEllipsis`. The look of a long title: on the VM |
| A moved window could go back to its first position after full screen | `ClientSourceTests.LeavingFullScreenReturnsTheWindowToWhereItWas`. Dragging and leaving full screen: on the VM |
| A moved window, maximized, taken through full screen and then restored came back at an older position | `ClientSourceTests.LeavingFullScreenReturnsTheWindowToWhereItWas` (the whole window placement is kept). Maximize, full screen, Restore: on the VM |
| After Maximize or Restore from the caption buttons the player's keys did nothing until a click on it | `ClientSourceTests.ThePlayerTakesTheKeyboardBackAfterMaximizeOrRestore`. The real keyboard: on the VM |
| With the player closed, Maximize or Restore from the caption buttons left the search box without focus, and typing went nowhere | `ClientSourceTests.SearchsBoxTakesTheKeyboardBackAfterMaximizeOrRestore`. Typing after Maximize and Restore: on the VM |
| A silent title showed Audio and Subtitles, and Audio opened an empty menu (mpv lists no audio or subtitle track, so the buttons were never set) | `ReviewRegressionTests.TrackButtonsShowOnlyWithSomethingToPick`, `ClientSourceTests.TrackButtonsAreSetWhenThePlaybackAnswerArrives`. A silent MP4: on the VM |
| Settings (quality, skip mode per kind, Auto-Skip Countdown, audio and subtitle languages, subtitles on, size, background) kept and used by the player, with the Mac app's defaults; picks in the player set them | `PlayerParityTests.SettingsDefaultsAreTheMacs`, `SettingsSurviveARestartAndABadFile`, `PickingTracksInThePlayerSetsThePreferences`, `TheSettingsLanguagesAreUsedToStartATitle`, `ClientSourceTests.ThePlayerFollowsTheSettings`. Each setting played: on the VM |
| The Quality menu offers the Mac's options for the file (`QualityOption.options`) and restarts the stream at the pick | `PlayerParityTests.QualityOffersOnlyPresetsBelowTheSource`, `QualityStartsAtTheSettingWhenTheFileIsAboveIt`, `QualitySizesAreTheFileForOriginalAndAnEstimateOtherwise`. The server's transcode at the pick: on the VM |
| Episodes in the player, by season, the playing one marked | `PlayerParityTests.EpisodesAreGroupedBySeasonInAiringOrder`. Picking one: on the VM |
| Scrubbing thumbnails from `/trickplay` above the knob | `PlayerParityTests.TheScrubPreviewFollowsTheKnobInsideTheTrack`, `SharedCasesTests` (trickplay.json). Dragging: on the VM |
| Subtitles drawn by the app (Style, Delay), embedded tracks window by window from `/subtitles/{i}/cues`, then whole once the server has them | `PlayerParityTests.SrtVttAndAssParse`, `OldSrtFilesDecodeAsWindows1252`, `TheDelayMenuMovesByStepsAndSaysSo`, `SubtitlesSitAboveTheControlsWhenTheyShow`, `WindowsComeAtThePlayheadThenTheOneAfter`, `AServerWithoutWindowsGetsTheWholeTrackAndTheEndIsKnown`, `AFailedWindowIsAskedForAgain`, `SharedCasesTests` (subtitle-cues.json, playback.json), `ClientSourceTests.SubtitlesAreDrawnByTheAppNotByMpv`. A title the server hadn't extracted yet: on the VM |
| The player plays on its title page (16:9 up to 60 % of the page, under the top bar), goes full screen and back as the same player, and stops when the page is left; Home's Play opens the title's page | `LibraryBrowseTests.TheTitlePagesPlayerIs169UpToSixtyPercentOfThePage`, `ClientSourceTests.ThePlayerPlaysOnItsTitlePageAndGoesFullScreenFromThere`. Play, F, Esc, Maximize, Episodes, Back: on the VM |
| Library categories (Genres, Collections for movies, Studios or Networks, Decades) as tiles, a tile's page with its dimension locked, long studio and collection lists cut to those with more than one title until a search | `LibraryBrowseTests.TheSidebarListsCollectionsOnlyForMovies`, `ATileOpensItsPageWithItsDimensionLocked`, `LongStudioAndCollectionListsShowThoseWithMoreThanOneTitle`, `TheServersFacetsGiveTheTiles`. Each page: on the VM |
| The A–Z strip (title sort, more than 24 titles) jumps to the nearest letter with titles; the grid makes only the rows near the screen and fetches the pages it reaches | `LibraryBrowseTests.TheLetterStripJumpsToTheNearestLetterWithTitles`, `TheGridMakesOnlyTheRowsNearTheScreen`, `PagesAreFetchedForTheRangeShownAndAnOldAnswerIsDropped`, `ClientSourceTests.TheLibraryGridMakesOnlyTheCardsNearTheScreen`. A few hundred titles: on the VM |
| Set Up This Server (setup code, admin account, a folder of the server's, the first scan) and Have an Invitation from the sign-in page | `SetupTests` (all), `ClientSourceTests.ANewServerIsSetUpAndAnInvitationJoinsFromSignIn`. A new server and an invitation: on the VM |
| Set Up This Server on Windows has the Mac's five steps: a network share beside a server folder (file servers the server sees or an address, an account only when the share asks, shares, folders, the server's check), and the optional Extras (own keys and accounts; Finish saves only what was filled in) | `SetupTests.TheNetworkShareAnswersDecode`, `AShareFolderBecomesTheLibrarysRootWithItsAccount`, `ShareRequestsGoToTheServerWithTheAccountOnlyWhenThereIsOne`, `ShareFailuresSayWhatToDo`, `ShareFoldersGoUpToTheShares`, `ExtrasSaveOnlyWhatWasFilledIn`, `ClientSourceTests.TheSetupOffersANetworkShareAndExtrasAndStopsPollingWhenLeft`. A share and Extras: on the VM |
| Leaving the setup's scan step (to Extras, Finish, or the page unloaded) while a progress request is out no longer restarts the polling | `SetupTests.APollAnsweredAfterTheScanStepIsLeftDoesNotPollAgain`, `ClientSourceTests.TheSetupOffersANetworkShareAndExtrasAndStopsPollingWhenLeft` |
| A title watched before, started again and stopped part way restarted from the beginning on Android (its Resume needed "not played" and at least 1 s); the Apple and Windows apps resumed | `ResumeRulesCasesTest` (Android), `ResumeRulesCasesTests` (Apple), `SharedCasesTests.ResumeRuleCases` (Windows), all on `api/resume-rules.json`; `BrowseAndWatchTest.rewatchResumeCardShowsProgress`; `WatchNextTest.recentsThenNextUpWithoutShowsMidEpisode` |
| The phone's Resume on a title page and tapping an episode started at 0: the player took "nothing asked for" as the beginning, not the resume point | `ClientRulesTest.nothingAskedForResumesWhereItWasLeft` |
| The phone's autoplay (a card's Play) on a title with several versions could start at the position another cut was left at: it fired on the list's item, before the page knew the versions, and "nothing asked for" took the title's position, not the version's | `ClientRulesTest.aVersionWithoutAPositionStartsAtTheBeginning`, `ClientRulesTest.autoplayWaitsForTheVersionsOfATitleWithSeveral` |
| Playback errors showed the server's raw text (ffprobe output, internal paths, "no Cues found"), cut to one line in the phone's portrait player; a title whose only file was damaged or gone still offered Play and Download | `PlaybackProblemTests` (Apple), `PlaybackProblemCasesTest` (Android), `SharedCasesTests.PlaybackProblemCases` and `PlaybackProblemTests` (Windows), all on `api/playback-problem-rules.json`, and each app's title-page rule (`unplayableProblem`, `PlaybackProblem.Unplayable`). The look (short title, plain line, Details; the note in place of Play): simulator screenshots |
| The display's hold follows playback: held while playing (and loading or buffering), released when paused, ended, failed or closed; mpv no longer holds it itself | `ScreenAwakeTests` (rule and the player's wiring). `powercfg /requests` playing, paused, after a seek, after the end: on the VM |
| `release-apple.sh` took its Mac from `MAC` / `FASTLANE_MAC`, which can name someone's own Mac; `build-tailscalekit.sh` let `scripts/mac.env` override the Mac a store build passed it | `release-apple.test.sh` (only `RELEASE_MAC`; a Mac in `PERSONAL_MACS` refused; the TailscaleKit step keeps the given Mac and folder) |
| `store-shots.sh` took its Mac from `MAC` too, and the `build.sh` it runs read `MAC` and `MAC_DIR` from `scripts/mac.env` again | `store-shots.test.sh` (only `RELEASE_MAC`, the build included, in its own folder; no `RELEASE_MAC`, a Mac in `PERSONAL_MACS` or `local`: no ssh at all) |
| `ota.sh` built on `FASTLANE_MAC`, else `MAC`, which can name someone's own Mac; `test-apple.sh` and `check.sh` ran the app's tests on whatever `MAC` named | `ota.test.sh` (only `RELEASE_MAC`, in its own folder; no `RELEASE_MAC`, a Mac in `PERSONAL_MACS` or `local`: no ssh at all; the tests skip a `MAC` in `PERSONAL_MACS`) |
| The `submit` lane's guard refused fastlane's `confirm:yes` (fastlane passes it as true); submitted versions stayed on automatic release (`automatic_release: false` is skipped with `skip_app_version_update`) | `clients/apple/fastlane/test/release_helpers_test.rb`, run by `fastlane-helpers.test.sh` (needs ruby: the build Mac, or a local ruby image) |
