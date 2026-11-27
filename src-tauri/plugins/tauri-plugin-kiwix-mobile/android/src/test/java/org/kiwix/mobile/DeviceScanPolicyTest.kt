package org.kiwix.mobile

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DeviceScanPolicyTest {
    @Test
    fun dotNamedEntriesAreNeverScanCandidates() {
        // Rust rejects a whole scan that contains a dot-named archive.
        assertTrue(hiddenScanName(".trashed-1764201600-wikipedia.zim"))
        assertTrue(hiddenScanName(".pending-wikipedia.zim"))
        assertTrue(hiddenScanName(".Trash-1000"))
        assertFalse(hiddenScanName("wikipedia.zim"))
        assertFalse(hiddenScanName("wikipedia.en.zimaa"))
        assertFalse(hiddenScanName("Downloads"))
    }
}
