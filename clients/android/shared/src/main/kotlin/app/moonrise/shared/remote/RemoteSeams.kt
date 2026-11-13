package app.moonrise.shared.remote

import android.content.Intent
import app.moonrise.shared.BuildConfig
import app.moonrise.shared.Log
import app.moonrise.shared.downloads.ServerReach

/**
 * Remote access test seams, debug builds only (the Dev apps): `--es tsauthkey <auth key>` turns on
 * Connect Through Tailscale and signs the node in with the key, as an ephemeral device (no web
 * sign-in; the key is never logged), and `--ez forcetailnet true` (until `false`) pretends the home
 * network doesn't answer, so the route goes through the tailnet at home too.
 */
object RemoteSeams {
    fun take(intent: Intent?) {
        if (!BuildConfig.DEBUG || intent == null) return
        if (intent.hasExtra("forcetailnet")) {
            ServerReach.forceTailnet = intent.getBooleanExtra("forcetailnet", false)
            intent.removeExtra("forcetailnet")
            Log.i("Moonrise") { "test seam: force the tailnet route: ${ServerReach.forceTailnet}" }
            ServerReach.nudge()
        }
        intent.getStringExtra("tsauthkey")?.takeIf { it.isNotBlank() }?.let { key ->
            intent.removeExtra("tsauthkey")
            Log.i("Moonrise") { "test seam: sign in to Tailscale with an auth key" }
            Tailscale.debugAuthKey = key
        }
    }
}
