package org.kiwix.mobile

import java.io.File
import java.nio.file.Files
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** A copy import that fails part-way must not leave its staged archives behind. */
class StagingCleanupTest {
    private fun staging(): File = Files.createTempDirectory("kiwix-staging").toFile()

    @Test
    fun failedCopyLeavesTheStagingDirectoryEmpty() {
        val root = staging()
        try {
            val staged = StagedEntries()
            // First archive copied completely, second interrupted by a full disk.
            val first = File(root, "first.zim").apply { writeBytes(ByteArray(4096)) }
            staged.file(first)
            val folder = File(root, "nested").apply { check(mkdir()) }
            staged.directory(folder)
            val partial = File(folder, "second.zim").apply { writeBytes(ByteArray(17)) }
            staged.file(partial)

            assertEquals(emptyList<String>(), staged.remove())
            assertEquals(emptyList<String>(), root.list()!!.toList())
            // The directory Rust created for the attempt is not this record's to remove.
            assertTrue(root.isDirectory)
        } finally { root.deleteRecursively() }
    }

    @Test
    fun cleanupNeverRemovesWhatTheOperationDidNotCreate() {
        val root = staging()
        try {
            val foreign = File(root, "library.zim").apply { writeText("kept") }
            val folder = File(root, "shared").apply { check(mkdir()) }
            val neighbour = File(folder, "neighbour.zim").apply { writeText("kept too") }
            val staged = StagedEntries()
            staged.directory(folder)
            val owned = File(folder, "owned.zim").apply { writeBytes(ByteArray(8)) }
            staged.file(owned)

            assertEquals(emptyList<String>(), staged.remove())
            assertFalse(owned.exists())
            assertEquals("kept", foreign.readText())
            assertEquals("kept too", neighbour.readText())
            // A directory that still holds someone else's file stays.
            assertTrue(folder.isDirectory)
        } finally { root.deleteRecursively() }
    }

    @Test
    fun whatCouldNotBeRemovedIsStillReported() {
        val root = staging()
        try {
            val staged = StagedEntries()
            // A recorded path that turned into a non-empty directory cannot be deleted.
            val stubborn = File(root, "stubborn.zim").apply { check(mkdir()) }
            File(stubborn, "content").writeText("x")
            staged.file(stubborn)
            assertEquals(listOf(stubborn.path), staged.remove())
            assertEquals(listOf(stubborn.path), staged.retainedPaths())
        } finally { root.deleteRecursively() }
    }

    @Test
    fun onlyCopyImportsRemoveTheirFilesOnFailure() {
        assertTrue(stagingFailureRemovesCopies("archives"))
        assertTrue(stagingFailureRemovesCopies("directory"))
        // Exports and viewer copies keep their own retained-file recovery.
        for (kind in listOf("export", "open", "text", "external", "externalBatch", "scanFolder"))
            assertFalse(stagingFailureRemovesCopies(kind))
    }
}
