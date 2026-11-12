package app.moonrise.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class LocalSubnetTest {
    @Test
    fun everyHostOfEachPrivateSlash24Once() {
        val h = LocalSubnet.hosts(listOf("192.168.1.23", "192.168.1.40", "10.0.5.7"))
        assertEquals(508, h.size)
        assertEquals("192.168.1.1", h.first())
        assertEquals("192.168.1.254", h[253])
        assertTrue("10.0.5.7" in h)
    }

    @Test
    fun publicLoopbackAndMalformedAddressesAreLeftOut() {
        assertEquals(emptyList<String>(), LocalSubnet.hosts(listOf("8.8.8.8", "127.0.0.1", "172.32.0.1", "not an address", "300.1.1.1")))
        assertEquals(254, LocalSubnet.hosts(listOf("172.20.3.4")).size)
    }
}
