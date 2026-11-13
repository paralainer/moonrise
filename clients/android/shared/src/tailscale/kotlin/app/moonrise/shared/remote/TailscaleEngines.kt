package app.moonrise.shared.remote

import app.moonrise.tsnet.moonrisets.Host
import app.moonrise.tsnet.moonrisets.Moonrisets

/** Built with the Tailscale library (vendor/moonrise-tailscale.aar): the node is the Go one. */
internal object TailscaleEngines {
    val engine: TailscaleEngine? = GoEngine
}

private object GoEngine : TailscaleEngine {
    override fun start(dir: String, hostName: String, authKey: String, verbose: Boolean, host: TailscaleEngine.Host) {
        Moonrisets.start(dir, hostName, authKey, verbose, object : Host {
            override fun onState(state: String, detail: String) = host.onState(state, detail)
            override fun interfaces(): String = host.interfaces()
            override fun log(line: String) = host.log(line)
        })
    }

    override fun stop() = Moonrisets.stop()
    override fun peers(): String = Moonrisets.peers()
    override fun forward(target: String, preferredPort: Int): Int = Moonrisets.forward(target, preferredPort.toLong()).toInt()
    override fun rebind() = Moonrisets.rebind()
}
