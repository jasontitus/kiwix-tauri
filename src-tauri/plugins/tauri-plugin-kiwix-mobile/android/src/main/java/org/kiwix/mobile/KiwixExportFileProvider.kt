package org.kiwix.mobile

import androidx.core.content.FileProvider

/** Separate manifest identity from the host app's provider; paths remain export-cache-only. */
class KiwixExportFileProvider : FileProvider()
