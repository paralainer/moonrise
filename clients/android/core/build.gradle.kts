// The Moonrise server's API, in plain Kotlin/JVM: no Android classes, so the TV app and a later
// phone/tablet app share it, and its tests run on the JVM.
plugins {
    alias(libs.plugins.kotlin.jvm)
    alias(libs.plugins.kotlin.serialization)
}

kotlin { jvmToolchain(17) }

dependencies {
    api(libs.kotlinx.coroutines.core)
    api(libs.okhttp)
    implementation(libs.kotlinx.serialization.json)
    testImplementation(libs.junit)
    testImplementation(libs.okhttp.mockwebserver)
}

// The API contract's example answers (api/examples at the repository's root), which
// APIExamplesTest decodes; build.sh mounts them into its container (MOONRISE_API_EXAMPLES).
tasks.test {
    val examples = System.getenv("MOONRISE_API_EXAMPLES") ?: rootProject.file("../../api/examples").path
    systemProperty("moonrise.api.examples", examples)
    inputs.dir(examples).withPropertyName("apiExamples")
    // The segment rules' shared cases (api/segment-rules.json), which SegmentRulesCasesTest checks.
    val rules = System.getenv("MOONRISE_SEGMENT_RULES") ?: rootProject.file("../../api/segment-rules.json").path
    systemProperty("moonrise.segment.rules", rules)
    inputs.file(rules).withPropertyName("segmentRules")
    // The resume rule's shared cases (api/resume-rules.json), which ResumeRulesCasesTest checks.
    val resume = System.getenv("MOONRISE_RESUME_RULES") ?: rootProject.file("../../api/resume-rules.json").path
    systemProperty("moonrise.resume.rules", resume)
    inputs.file(resume).withPropertyName("resumeRules")
    // How playback failures are named (api/playback-problem-rules.json), which PlaybackProblemCasesTest checks.
    val problems = System.getenv("MOONRISE_PLAYBACK_PROBLEM_RULES") ?: rootProject.file("../../api/playback-problem-rules.json").path
    systemProperty("moonrise.playback.problem.rules", problems)
    inputs.file(problems).withPropertyName("playbackProblemRules")
    // The phone app's sources, which ClientRulesTest scans: a phone-only edit must re-run it.
    inputs.dir(rootProject.file("phone/src/main/kotlin")).withPropertyName("phoneSources")
    // The other apps' sources, which TrackMemoryTest scans with the phone's.
    inputs.dir(rootProject.file("shared/src/main/kotlin")).withPropertyName("sharedSources")
    inputs.dir(rootProject.file("tv/src/main/kotlin")).withPropertyName("tvSources")
}
