package org.kiwix.mobile

import android.content.Context
import android.system.Os
import app.tauri.plugin.JSObject

/** Locate the stored voice asset in the installed base or split APK. This
 * performs no installation and accepts no paths from a caller or document.
 * Rust reopens the same APK identity, bounds reads to this asset, and checks
 * every model file against its compiled manifest before publication.
 */
internal object BundledKokoro {
    private const val ASSET = "kiwix-kokoro-v1.voicepack"

    fun containsPack(listing: Array<String>?): Boolean {
        val names = checkNotNull(listing) { "Cannot read the installed package's voice asset list" }
        return ASSET in names
    }

    fun location(context: Context): JSObject {
        if (!containsPack(context.assets.list(""))) return JSObject().put("available", false)
        context.assets.openFd(ASSET).use { asset ->
            val stat = Os.fstat(asset.fileDescriptor)
            val info = context.applicationInfo
            val apks = listOf(info.sourceDir) + info.splitSourceDirs.orEmpty()
            val path = apks.firstOrNull { apk ->
                val current = Os.stat(apk)
                current.st_dev == stat.st_dev && current.st_ino == stat.st_ino
            } ?: error("Cannot locate the offline voice asset in an installed APK")
            require(asset.startOffset >= 0 && asset.declaredLength > 0 &&
                asset.startOffset <= stat.st_size - asset.declaredLength) { "Invalid offline voice asset extent" }
            return JSObject().put("available", true).put("path", path)
                .put("offset", asset.startOffset).put("length", asset.declaredLength)
                .put("device", stat.st_dev).put("inode", stat.st_ino)
        }
    }
}
