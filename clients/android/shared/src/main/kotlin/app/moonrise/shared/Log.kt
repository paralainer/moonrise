package app.moonrise.shared

/**
 * The app's log (logcat), in place of `android.util.Log`.
 *
 * - **Debug and info** (`d`, `i`) trace what the app does: focus, menus, the player's steps,
 *   sync. They're for development and scripted checks, so they're written in debug builds only
 *   (the Dev app). The message is a lambda and the call sits behind `BuildConfig.DEBUG`, a
 *   constant, so in a release build R8 drops the call, the message and its text.
 * - **Warnings and errors** (`w`, `e`) say something failed, and are kept in every build.
 */
object Log {
    inline fun d(tag: String, message: () -> String) {
        if (BuildConfig.DEBUG) android.util.Log.d(tag, message())
    }

    inline fun i(tag: String, message: () -> String) {
        if (BuildConfig.DEBUG) android.util.Log.i(tag, message())
    }

    fun w(tag: String, message: String, error: Throwable? = null) {
        if (error == null) android.util.Log.w(tag, message) else android.util.Log.w(tag, message, error)
    }

    fun e(tag: String, message: String, error: Throwable? = null) {
        if (error == null) android.util.Log.e(tag, message) else android.util.Log.e(tag, message, error)
    }
}
