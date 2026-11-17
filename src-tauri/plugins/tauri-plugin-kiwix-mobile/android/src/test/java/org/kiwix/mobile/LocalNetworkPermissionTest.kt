package org.kiwix.mobile

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Android 17 (API 37) is the first release that requires ACCESS_LOCAL_NETWORK. */
class LocalNetworkPermissionTest {
    @Test
    fun requiresRuntimeGrantFromApi37() {
        assertFalse(localNetworkRequired(24))
        assertFalse(localNetworkRequired(36))
        assertTrue(localNetworkRequired(37))
        assertTrue(localNetworkRequired(38))
    }

    @Test
    fun olderTargetsKeepImplicitLocalNetworkAccess() {
        assertTrue(localNetworkAdmitted(36, granted = false))
        assertTrue(localNetworkAdmitted(37, granted = true))
        assertFalse(localNetworkAdmitted(37, granted = false))
    }

    @Test
    fun checkOnlyCallsNeverLaunchASheet() {
        assertFalse(localNetworkShouldRequest(37, granted = false, prompt = false))
        assertTrue(localNetworkShouldRequest(37, granted = false, prompt = true))
        assertFalse(localNetworkShouldRequest(37, granted = true, prompt = true))
        assertFalse(localNetworkShouldRequest(36, granted = false, prompt = true))
    }
}
