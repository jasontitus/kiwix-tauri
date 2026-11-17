package org.kiwix.mobile

import android.system.ErrnoException
import android.system.Os
import android.system.OsConstants
import java.io.File

/** Captured immediately after exclusive creation; never deletes a substituted inode or symlink. */
internal class ExportScratch(val file: File) {
    private val identity = Os.lstat(file.path)

    fun remove(): Boolean {
        val current = try { Os.lstat(file.path) } catch (error: ErrnoException) {
            return error.errno == OsConstants.ENOENT
        }
        if (current.st_dev != identity.st_dev || current.st_ino != identity.st_ino ||
            current.st_mode != identity.st_mode || OsConstants.S_ISLNK(current.st_mode)) return false
        return file.delete()
    }

    fun remains(): Boolean = try {
        Os.lstat(file.path)
        true
    } catch (error: ErrnoException) { error.errno != OsConstants.ENOENT }
}
