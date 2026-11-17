package org.kiwix.mobile

import android.content.Context
import android.os.Build
import android.os.Environment
import android.os.storage.StorageManager
import android.os.storage.StorageVolume
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import java.io.File

/**
 * App-owned library locations. Every external candidate is this package's own
 * `Android/data/<package>/files` directory, so it is writable without a broad
 * storage permission and never exposes a raw SD-card path to Rust or the
 * webview. The Rust layer appends `archives` and validates the returned set
 * again before accepting a selection.
 */
internal object StorageLocations {
    fun enumerate(context: Context): JSObject {
        val locations = JSArray()
        val internal = runCatching { context.filesDir.canonicalFile }.getOrNull()
            ?: context.filesDir
        locations.put(entry(internal, "internal", false, "Internal app storage"))
        val volumes = runCatching {
            (context.getSystemService(Context.STORAGE_SERVICE) as StorageManager).storageVolumes
        }.getOrDefault(emptyList())
        val seen = mutableSetOf(internal.absolutePath)
        context.getExternalFilesDirs(null).filterNotNull().forEach { directory ->
            val canonical = runCatching { directory.canonicalFile }.getOrNull() ?: directory
            if (!seen.add(canonical.absolutePath)) return@forEach
            val volume = volumeFor(canonical, volumes)
            val removable = volume?.isRemovable
                ?: Environment.isExternalStorageRemovable(canonical)
            val label = volume?.getDescription(context)?.takeIf { it.isNotBlank() }
                ?: if (removable) "Removable storage" else "External app storage"
            locations.put(entry(canonical, "external", removable, label))
        }
        return JSObject().put("locations", locations)
    }

    private fun entry(
        directory: File,
        kind: String,
        removable: Boolean,
        label: String,
    ) = JSObject()
        .put("path", directory.absolutePath)
        .put("kind", kind)
        .put("removable", removable)
        .put("label", label)

    private fun volumeFor(directory: File, volumes: List<StorageVolume>): StorageVolume? {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return null
        return volumes.firstOrNull { volume ->
            val root = volume.directory ?: return@firstOrNull false
            directory.absolutePath == root.absolutePath ||
                directory.absolutePath.startsWith(root.absolutePath + File.separator)
        }
    }
}
