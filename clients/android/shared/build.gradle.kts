// What the Android apps (TV, phone) share beyond core: the signed-in server, this device's settings,
// servers found on the network, a title page's data, and the player (Media3, with the server's
// sessions, tracks, segments and reports). Android classes, no screens.
plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

// Remote access through Tailscale, when its library is built: vendor/moonrise-tailscale.aar
// (scripts/build-tailscale-android.sh, git-ignored). Without it, or with -Pmoonrise.tailscale=false,
// the apps build and run without the Remote Access section (src/notailscale).
val tailscaleAar = rootProject.file("vendor/moonrise-tailscale.aar")
val withTailscale = tailscaleAar.exists() && (findProperty("moonrise.tailscale") as String?) != "false"
logger.lifecycle("Tailscale remote access: ${if (withTailscale) "built in (${tailscaleAar.name})" else "left out"}")

android {
    namespace = "app.moonrise.shared"
    compileSdk = 35

    defaultConfig {
        minSdk = 26
    }

    buildFeatures {
        compose = true
        // BuildConfig.DEBUG gates the debug and info logs (app.moonrise.shared.Log): the debug
        // build of an app (its Dev app) uses this library's debug build.
        buildConfig = true
    }

    sourceSets["main"].java.srcDir(if (withTailscale) "src/tailscale/kotlin" else "src/notailscale/kotlin")

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
}

dependencies {
    api(project(":core"))
    api(libs.kotlinx.coroutines.android)
    api(platform(libs.compose.bom))
    api(libs.compose.ui)
    api(libs.media3.exoplayer)
    api(libs.media3.exoplayer.hls)
    implementation(libs.media3.datasource.okhttp)
    implementation(libs.media3.ffmpeg.decoder)
    implementation(libs.zxing.core)
    if (withTailscale) implementation(":moonrise-tailscale@aar")
}
