import java.util.Properties

// The Android phone app. It follows the iPhone app (clients/apple/Moonrise, iOS): the same screens,
// order, wording and layout, with Android's back navigation, system bars and media controls.
plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "app.moonrise.phone"
    compileSdk = 35

    defaultConfig {
        applicationId = "app.moonrise.phone"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    // Release signing, as the TV app's (tv/build.gradle.kts): MOONRISE_ANDROID_SIGNING names the
    // properties file, else clients/android/signing.properties (git-ignored); without one the release
    // APK is unsigned.
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
        // The debug build is "Moonrise Dev" (app.moonrise.phone.dev), its own app beside the release
        // one: its own sign-in and settings, its name and an amber icon from src/debug/res.
        debug {
            applicationIdSuffix = ".dev"
            versionNameSuffix = "-dev"
        }
        release {
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
            "-opt-in=androidx.compose.material3.ExperimentalMaterial3Api",
            "-opt-in=androidx.compose.foundation.ExperimentalFoundationApi",
        )
    }
}

dependencies {
    implementation(project(":core"))
    implementation(project(":shared"))
    // Settings values and the settings document of the admin API (JSON, as core reads it).
    implementation(libs.kotlinx.serialization.json)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons)
    implementation(libs.activity.compose)
    implementation(libs.lifecycle.runtime.compose)
    implementation(libs.media3.ui)
    implementation(libs.media3.session)
    implementation(libs.coil.compose)
    debugImplementation(libs.compose.ui.tooling)
}
