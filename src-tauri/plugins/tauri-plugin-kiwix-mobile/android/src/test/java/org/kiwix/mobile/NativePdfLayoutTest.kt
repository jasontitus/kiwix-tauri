package org.kiwix.mobile

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class NativePdfLayoutTest {
    @Test
    fun viewportTilesCoverEveryLiveDocumentRowExactlyOnceAcrossPageBoundaries() {
        val pageHeight = nativePdfPageHeight(390)
        for (height in listOf(1, pageHeight - 1, pageHeight, pageHeight + 1, pageHeight * 3 + 23)) {
            for (viewport in listOf(1, 113, pageHeight, pageHeight * 2)) {
                val rows = mutableListOf<Int>()
                var page = 0
                var pageOffset = 0
                while (page * pageHeight + pageOffset < height) {
                    val top = page * pageHeight + pageOffset
                    val tile = nativePdfTileHeight(viewport, pageHeight, pageOffset, height, top)
                    assertTrue(tile > 0)
                    val actualScroll = minOf(top, (height - viewport).coerceAtLeast(0))
                    val drawOffset = nativePdfViewportOffset(top, actualScroll)
                    repeat(tile) { rows.add(actualScroll + drawOffset + it) }
                    pageOffset += tile
                    if (pageOffset == pageHeight) { page += 1; pageOffset = 0 }
                }
                assertEquals((0 until height).toList(), rows)
            }
        }
    }

    @Test
    fun narrowAndWideCapturesKeepA4AspectWithoutIntegerOverflow() {
        assertEquals(2, nativePdfPageHeight(1))
        assertEquals(574, nativePdfPageHeight(390))
        val wide = nativePdfPageHeight(16384)
        assertTrue(wide > 16384)
        assertTrue(wide.toLong() * 52328 >= 16384L * 76989)
        assertTrue((wide - 1L) * 52328 < 16384L * 76989)
    }
}
