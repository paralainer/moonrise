import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "app.moonrise.tv"
    compileSdk = 35

    defaultConfig {
        applicationId = "app.moonrise.tv"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
    }

    buildFeatures {
        compose = true
        // BuildConfig.DEBUG: debug-only behaviour (the logs are app.moonrise.shared.Log, gated by the
        // shared module's own BuildConfig, which follows this build type).
        buildConfig = true
    }

    // Release signing, from a properties file kept out of the repository (storeFile, relative to the
    // file, storePassword, keyAlias, keyPassword): MOONRISE_ANDROID_SIGNING names it, else
    // clients/android/signing.properties (git-ignored). Without one, the release APK is unsigned.
    val signing = (System.getenv("MOONRISE_ANDROID_SIGNING")?.takeIf { it.isNotBlank() }?.let { file(it) }
        ?: rootProject.file("signing.properties")).takeIf { it.isFile }
    if (signing != null) {
        val p = Properties().apply { signing.inputStream().use { load(it) } }
        fun prop(name: String) = p.getProperty(name)?.takeIf { it.isNotBlank() } ?: error("$signing has no $name")
        signingConfigs {
            create("release") {
                storeFile = signing.parentFile.resolve(prop("storeFile"))
                storePassword = prop("storePassword")
                keyAlias = prop("keyAlias")
                keyPassword = prop("keyPassword")
            }
        }
    }

    buildTypes {
        // The debug build is "Moonrise Dev" (app.moonrise.tv.dev), its own app beside the release
        // one (app.moonrise.tv): its own sign-in, settings and Watch Next rows, its name and an amber
        // icon and banner from src/debug/res. Testing installs it without touching the release app.
        debug {
            applicationIdSuffix = ".dev"
            versionNameSuffix = "-dev"
        }
        release {
            // R8: much faster Compose on a TV's CPU. The libraries bring their own keep rules.
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
        freeCompilerArgs += listOf(
            "-opt-in=androidx.tv.material3.ExperimentalTvMaterial3Api",
            // focusRestorer: Back to a page lands on the card it left.
            "-opt-in=androidx.compose.ui.ExperimentalComposeUiApi",
        )
    }
}

dependencies {
    implementation(project(":core"))
    implementation(project(":shared"))
    implementation(libs.kotlinx.coroutines.android)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.tv.material)
    implementation(libs.activity.compose)
    implementation(libs.lifecycle.viewmodel.compose)
    implementation(libs.lifecycle.runtime.compose)
    implementation(libs.media3.exoplayer)
    implementation(libs.media3.exoplayer.hls)
    implementation(libs.media3.datasource.okhttp)
    implementation(libs.media3.ui)
    implementation(libs.media3.ffmpeg.decoder)
    implementation(libs.coil.compose)
    implementation(libs.zxing.core)
    implementation(libs.tvprovider)
    implementation(libs.work.runtime)
    debugImplementation(libs.compose.ui.tooling)
}
