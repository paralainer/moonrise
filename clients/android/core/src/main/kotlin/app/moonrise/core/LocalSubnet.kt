package app.moonrise.core

/**
 * The addresses a subnet sweep asks (`LocalSubnet.hosts`, Apple apps): every host of the /24
 * networks of the device's private IPv4 addresses (10/8, 172.16/12, 192.168/16), this device's
 * own too (a server on the same machine). Public, loopback and repeated networks are left out.
 */
object LocalSubnet {
    fun hosts(addresses: List<String>): List<String> {
        val seen = mutableSetOf<String>()
        val out = mutableListOf<String>()
        for (a in addresses) {
            val p = a.split('.').mapNotNull { it.toIntOrNull() }
            if (p.size != 4 || p.any { it !in 0..255 }) continue
            val private = p[0] == 10 || (p[0] == 192 && p[1] == 168) || (p[0] == 172 && p[1] in 16..31)
            if (!private) continue
            val net = "${p[0]}.${p[1]}.${p[2]}"
            if (!seen.add(net)) continue
            for (h in 1..254) out += "$net.$h"
        }
        return out
    }
}
