pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "moonrise-android"
// core: the Moonrise server API client, plain Kotlin/JVM (no Android), for every Android client.
include(":core")
// shared: what the Android apps share beyond core: the sign-in, settings, discovery, title pages' data and
// the player (Media3).
include(":shared")
// tv: the Android TV / Google TV app.
include(":tv")
// phone: the Android phone app.
include(":phone")
