package org.kiwix.mobile

import java.net.InetAddress
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Prefix tests must not need IpPrefix(InetAddress, Int), which is API 33+. */
class NetworkPrefixTest {
    private fun ip(text: String): InetAddress = InetAddress.getByAddress(
        text.split('.').map { it.toInt().toByte() }.toByteArray())

    @Test
    fun octetAlignedPrefixes() {
        assertTrue(inPrefix(ip("192.168.43.1"), ip("192.168.43.200"), 24))
        assertFalse(inPrefix(ip("192.168.43.1"), ip("192.168.44.200"), 24))
        assertTrue(inPrefix(ip("10.0.0.1"), ip("10.255.255.254"), 8))
        assertFalse(inPrefix(ip("10.0.0.1"), ip("11.0.0.1"), 8))
        assertTrue(inPrefix(ip("172.16.5.9"), ip("172.16.200.1"), 16))
    }

    @Test
    fun nonOctetPrefixes() {
        // 192.168.43.1/20 covers 192.168.32.0 - 192.168.47.255.
        assertTrue(inPrefix(ip("192.168.43.1"), ip("192.168.32.7"), 20))
        assertTrue(inPrefix(ip("192.168.43.1"), ip("192.168.47.255"), 20))
        assertFalse(inPrefix(ip("192.168.43.1"), ip("192.168.48.1"), 20))
        // /25 splits the last octet.
        assertTrue(inPrefix(ip("10.1.2.129"), ip("10.1.2.254"), 25))
        assertFalse(inPrefix(ip("10.1.2.129"), ip("10.1.2.126"), 25))
        // /30 and /31 point-to-point links.
        assertTrue(inPrefix(ip("10.1.2.5"), ip("10.1.2.6"), 30))
        assertFalse(inPrefix(ip("10.1.2.5"), ip("10.1.2.8"), 30))
        assertTrue(inPrefix(ip("10.1.2.4"), ip("10.1.2.5"), 31))
        assertFalse(inPrefix(ip("10.1.2.4"), ip("10.1.2.6"), 31))
    }

    @Test
    fun hostAndDefaultPrefixes() {
        assertTrue(inPrefix(ip("10.1.2.3"), ip("10.1.2.3"), 32))
        assertFalse(inPrefix(ip("10.1.2.3"), ip("10.1.2.2"), 32))
        assertTrue(inPrefix(ip("10.1.2.3"), ip("203.0.113.9"), 0))
    }

    @Test
    fun mismatchedFamiliesAndInvalidLengthsNeverMatch() {
        val v6 = InetAddress.getByAddress(ByteArray(16).also { it[0] = 0xfe.toByte(); it[1] = 0x80.toByte() })
        assertFalse(inPrefix(ip("10.1.2.3"), v6, 8))
        assertFalse(inPrefix(v6, ip("10.1.2.3"), 8))
        assertFalse(inPrefix(ip("10.1.2.3"), ip("10.1.2.3"), 33))
        assertFalse(inPrefix(ip("10.1.2.3"), ip("10.1.2.3"), -1))
        assertTrue(inPrefix(v6, v6, 128))
        assertTrue(inPrefix(v6, v6, 10))
    }
}
