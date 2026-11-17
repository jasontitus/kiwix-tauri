package org.kiwix.mobile

/** Integer screen-layout geometry shared by the public WebView drawing path. */
internal fun nativePdfPageHeight(width: Int): Int = ((width * 76989L + 52328 - 1) / 52328).toInt()
internal fun nativePdfTileHeight(viewport: Int, pageHeight: Int, pageOffset: Int, documentHeight: Int, top: Int): Int =
    minOf(viewport, pageHeight - pageOffset, documentHeight - top)

/** Android clamps the last scroll to documentHeight - viewportHeight. Draw only
 * the remaining part of that final viewport rather than duplicating its top. */
internal fun nativePdfViewportOffset(top: Int, actualScroll: Int): Int = (top - actualScroll).coerceAtLeast(0)
