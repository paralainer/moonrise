// What the Android apps (TV, phone) share beyond core: the signed-in server, this device's settings,
// servers found on the network, a title page's data, and the player (Media3, with the server's
// sessions, tracks, segments and reports). Android classes, no screens.
plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

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
}
