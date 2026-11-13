package app.moonrise.tv

import android.app.Application
import app.moonrise.shared.AppSettings
import app.moonrise.shared.Network
import app.moonrise.shared.Session
import app.moonrise.shared.WatchHooks
import app.moonrise.shared.downloads.ServerReach
import app.moonrise.shared.remote.Tailscale

class MoonriseApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        // The home screen's Watch Next row follows sign-in, marks and playback.
        WatchHooks.changed = { WatchNext.syncSoon(it) }
        WatchHooks.signedOut = { WatchNext.clear(it) }
        Session.init(this)
        AppSettings.init(this)
        Network.init(this)
        // Remote access: the node, then the route to the server through it or at home.
        Tailscale.init(this, "moonrise-androidtv")
        ServerReach.start(this)
        WatchNext.init(this)
    }
}
