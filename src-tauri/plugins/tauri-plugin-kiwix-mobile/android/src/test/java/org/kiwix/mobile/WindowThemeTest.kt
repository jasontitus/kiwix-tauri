package org.kiwix.mobile

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The system-bar strips follow the in-app theme, not only the device setting. */
class WindowThemeTest {
    @Test
    fun explicitThemeOverridesTheDeviceSetting() {
        for (systemNight in listOf(false, true)) {
            assertTrue(barAppearance("dark", systemNight).dark)
            assertFalse(barAppearance("light", systemNight).dark)
        }
    }

    @Test
    fun systemThemeFollowsTheDevice() {
        assertTrue(barAppearance("system", systemNight = true).dark)
        assertFalse(barAppearance("system", systemNight = false).dark)
        // An unknown value can never leave unreadable bar icons: follow the device.
        assertTrue(barAppearance("sepia", systemNight = true).dark)
        assertFalse(barAppearance("", systemNight = false).dark)
    }

    @Test
    fun stripColourIsThePageBackgroundOfTheMatchingPalette() {
        // --paper of :root[data-platform="android"] and its dark variant in src/platform.css.
        assertEquals(0xFFF8FAFD.toInt(), barAppearance("light", systemNight = true).background)
        assertEquals(0xFF131314.toInt(), barAppearance("dark", systemNight = false).background)
        assertEquals(barAppearance("dark", false), barAppearance("system", true))
        assertEquals(barAppearance("light", true), barAppearance("system", false))
    }
}
