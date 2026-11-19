package org.kiwix.mobile

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class SpeechAndProviderAuthorityTest {
    @Test fun unreadableVoiceAssetListRemainsRetryable() {
        try {
            BundledKokoro.containsPack(null)
            fail("An unavailable asset list must not select the download-only edition")
        } catch (_: IllegalStateException) { }
        assertFalse(BundledKokoro.containsPack(emptyArray()))
        assertFalse(BundledKokoro.containsPack(arrayOf("model.voicepack")))
        assertTrue(BundledKokoro.containsPack(arrayOf("other", "kiwix-kokoro-v1.voicepack")))
    }

    @Test fun splitCohortsPreserveProviderNamesRegardlessOfExtensionCase() {
        assertTrue(providerSplitPartsComplete(listOf("COHORT.ZIMAA", "COHORT.ZIMAB")))
        assertTrue(providerSplitPartsComplete(listOf("cohort.ZIMaa", "COHORT.zimAB")))
        assertTrue(providerSplitPartsComplete(listOf("cohort.zimaa", "cohort.zimab")))
        for (parts in listOf(emptyList(), listOf("cohort.zimab"),
            listOf("cohort.zimaa", "cohort.zimac"),
            listOf("cohort.zimaa", "COHORT.ZIMAA"),
            listOf("cohort.zimaa", "other.zimab"),
            listOf("cohort.zimaa", "cohort.zim"))) {
            assertFalse(providerSplitPartsComplete(parts))
        }
    }

    @Test fun speechSeeksNeverSplitSurrogatePairsAndClampAtEnds() {
        val text = "a\uD83D\uDE00bc"
        assertEquals(1, SpeechPosition.boundary(text, 2))
        assertEquals(0, SpeechPosition.timed(text, 1, -15.0, 15.0))
        assertEquals(text.length, SpeechPosition.timed(text, 1, 15.0, 15.0))
        assertEquals(3, SpeechPosition.timed(text, 1, 1.0, 2.0))
    }

    @Test fun paragraphTransportMovesBothDirectionsAtBoundaries() {
        val text = "first\nsecond\nthird"
        assertEquals(6, SpeechPosition.next(text, 0))
        assertEquals(13, SpeechPosition.next(text, 6))
        assertEquals(text.length, SpeechPosition.next(text, 13))
        assertEquals(0, SpeechPosition.previous(text, 7))
        assertEquals(6, SpeechPosition.previous(text, 14))
    }

    @Test fun timedSeekHonorsMeasuredRateAndRejectsInvalidTime() {
        val text = "x".repeat(1000)
        assertEquals(400, SpeechPosition.timed(text, 100, 15.0, 20.0))
        assertEquals(100, SpeechPosition.timed(text, 400, -15.0, 20.0))
        for (invalid in listOf(Double.NaN, Double.POSITIVE_INFINITY, 301.0)) {
            try { SpeechPosition.timed(text, 100, invalid, 20.0); fail("invalid time accepted") }
            catch (_: IllegalArgumentException) { }
        }
    }

    private val original = ProviderMoveIdentity("content://provider/document/42", "42", 1, 2, 100, 10, 10)

    @Test fun providerReplacementIsNeverDeletionAuthorityEvenAtSameNameAndSize() {
        assertTrue(providerMoveMayDelete(original, original, false))
        for (replacement in listOf(original.copy(inode = 3), original.copy(document = "43"),
            original.copy(uri = "content://provider/document/43"), original.copy(device = 2),
            original.copy(modified = 11), original.copy(changed = 11), original.copy(size = 101))) {
            for (attempting in listOf(false, true)) {
                try { providerMoveMayDelete(original, replacement, attempting); fail("replacement authorized") }
                catch (_: IllegalStateException) { }
            }
        }
    }

    @Test fun equalLengthSameInodeMutationCannotAuthorizeDeletionWithCoarseTimestamps() {
        val file = java.io.File.createTempFile("provider-original", ".zim")
        try {
            file.writeBytes("first".toByteArray())
            val before = java.nio.file.Files.readAttributes(file.toPath(), java.nio.file.attribute.BasicFileAttributes::class.java)
            val copiedDigest = file.inputStream().use { providerMoveDigest(it, 5) }
            val expected = original.copy(size = 5, sha256 = copiedDigest)
            java.io.RandomAccessFile(file, "rw").use { it.write("newer".toByteArray()) }
            val after = java.nio.file.Files.readAttributes(file.toPath(), java.nio.file.attribute.BasicFileAttributes::class.java)
            assertEquals(before.fileKey(), after.fileKey())
            assertEquals(before.size(), after.size())
            // Deliberately retain the identical provider stat tuple: timestamps
            // may have only second precision even though the bytes are newer.
            val actual = expected.copy(sha256 = file.inputStream().use { providerMoveDigest(it, 5) })
            for (attempting in listOf(false, true)) {
                var deleteCalls = 0
                try {
                    if (providerMoveMayDelete(expected, actual, attempting)) ++deleteCalls
                    fail("same-inode newer bytes authorized")
                } catch (_: IllegalStateException) { }
                assertEquals(0, deleteCalls)
                assertEquals("newer", file.readText())
            }
        } finally { file.delete() }
    }

    @Test fun remainingCohortMutationStopsTheNextDestructiveBoundary() {
        val first = original.copy(sha256 = providerMoveDigest("first".byteInputStream(), 5))
        val second = original.copy(document = "43", inode = 3,
            sha256 = providerMoveDigest("older".byteInputStream(), 5))
        val actual = mutableMapOf(first.document to first, second.document to second)
        val deleted = mutableSetOf<String>()
        var journalRetained = true
        fun boundary(part: ProviderMoveIdentity) {
            withVerifiedProviderMoveCohort(listOf(first, second).filter { it.document !in deleted }, {
                providerMoveMayDelete(it, actual[it.document], false)
            }) { deleted.add(part.document) }
        }
        boundary(first)
        actual[second.document] = second.copy(sha256 = providerMoveDigest("newer".byteInputStream(), 5))
        try {
            boundary(second)
            journalRetained = false
            fail("remaining mutated part deleted")
        } catch (_: IllegalStateException) { }
        assertEquals(setOf(first.document), deleted)
        assertTrue(journalRetained)
        assertEquals(providerMoveDigest("newer".byteInputStream(), 5), actual[second.document]!!.sha256)
    }

    @Test fun cohortValidationChecksLaterPartsBeforeDeletingTheFirst() {
        val expected = original.copy(sha256 = providerMoveDigest("older".byteInputStream(), 5))
        val mutated = expected.copy(sha256 = providerMoveDigest("newer".byteInputStream(), 5))
        var deleteCalls = 0
        try {
            withVerifiedProviderMoveCohort(listOf(expected, mutated), {
                providerMoveMayDelete(expected, it, false)
            }) { ++deleteCalls }
            fail("first part deleted before validating later member")
        } catch (_: IllegalStateException) { }
        assertEquals(0, deleteCalls)
    }

    @Test fun providerHashRejectsTruncatedOrGrownCopiedParts() {
        for (contents in listOf("less", "longer")) {
            try { providerMoveDigest(contents.byteInputStream(), 5); fail("changed size hashed") }
            catch (_: IllegalStateException) { }
        }
    }

    @Test fun unavailableDescriptorAfterMetadataLookupPreservesTheAcquisitionCode() {
        val reference = "2d1d57d1-3762-4e59-9fe6-00e888402a2b"
        val selected = listOf("USB.zim" to reference) // Metadata lookup already succeeded.
        var failure: Exception? = null
        try {
            requireProviderArchiveAsset<Any>(null)
            fail("missing descriptor accepted")
        } catch (error: Exception) {
            failure = providerAcquisitionFailure(error)
            assertTrue(failure === error)
        }
        assertEquals("PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT", (failure as NativeDocumentError).code)
        assertEquals(listOf("USB.zim" to reference), selected)
        // A successful next open uses the same selected reference, not a regrant.
        val descriptor = Any()
        assertTrue(requireProviderArchiveAsset(descriptor) === descriptor)
        assertEquals(reference, selected.single().second)
    }

    @Test fun acquisitionCleanupDoesNotConflateUnsupportedRevokedAndUnavailableSources() {
        for (code in listOf("PROVIDER_NOT_SEEKABLE_USE_COPY_IMPORT", "PROVIDER_REFERENCE_REVOKED",
            "PROVIDER_ARCHIVE_PARTS_CHANGED")) {
            val original = NativeDocumentError(code)
            assertTrue(providerAcquisitionFailure(original) === original)
        }
        val denied = providerAcquisitionFailure(SecurityException("revoked")) as NativeDocumentError
        assertEquals("PROVIDER_PERMISSION_DENIED", denied.code)
        val missing = providerAcquisitionFailure(java.io.FileNotFoundException("unmounted")) as NativeDocumentError
        assertEquals("PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT", missing.code)
        val unknown = providerAcquisitionFailure(java.io.IOException("unknown provider failure")) as NativeDocumentError
        assertEquals("PROVIDER_UNAVAILABLE_USE_COPY_IMPORT", unknown.code)
    }

    @Test fun onlyDurablyAttemptedDeletionCanTreatMissingObjectAsAlreadyDeleted() {
        assertFalse(providerMoveMayDelete(original, null, true))
        try { providerMoveMayDelete(original, null, false); fail("missing original accepted") }
        catch (_: IllegalStateException) { }
    }

    @Test fun documentedMissingDocumentReplaysAnAttemptedDeleteWithoutDeletingAgain() {
        // Durable journal state after deleteDocument succeeds but before deleted=true.
        var deleted = false
        val attempting = true
        var deleteCalls = 0
        val observation = queryProviderMoveDocument<ProviderMoveIdentity> {
            throw java.io.FileNotFoundException("document no longer exists")
        }
        if (!deleted) {
            if (providerMoveMayDelete(original, observation, attempting)) ++deleteCalls
            deleted = true
        }
        assertTrue(deleted)
        assertEquals(0, deleteCalls)
        try {
            providerMoveMayDelete(original, observation, attempting = false)
            fail("missing document without a durable delete attempt accepted")
        } catch (_: IllegalStateException) { }
    }

    @Test fun nullCursorCannotCompleteAnAttemptedDelete() {
        var deleted = false
        var deleteCalls = 0
        try {
            val observation = queryProviderMoveDocument<ProviderMoveIdentity> { null }
            if (providerMoveMayDelete(original, observation, attempting = true)) ++deleteCalls
            deleted = true
            fail("null cursor treated as absence")
        } catch (_: IllegalStateException) { }
        assertFalse(deleted)
        assertEquals(0, deleteCalls)
    }

    @Test fun providerAndPermissionFailuresCannotCompleteAnAttemptedDelete() {
        for (failure in listOf(SecurityException("grant revoked"), IllegalStateException("provider offline"),
            java.io.IOException("provider unavailable"))) {
            var deleted = false
            var deleteCalls = 0
            try {
                val observation = queryProviderMoveDocument<ProviderMoveIdentity> { throw failure }
                if (providerMoveMayDelete(original, observation, attempting = true)) ++deleteCalls
                deleted = true
                fail("unavailable provider treated as absence")
            } catch (actual: Exception) {
                assertTrue(actual === failure)
            }
            assertFalse(deleted)
            assertEquals(0, deleteCalls)
        }
    }

    @Test fun cohortRevalidationReusesOneDigestPerUnchangedPartAndRereadsChangedStat() {
        val cache = ProviderMoveDigestCache()
        val uri = original.uri
        val stat = ProviderMoveStat(original.device, original.inode, original.size, original.modified, original.changed)
        val digest = providerMoveDigest("first".byteInputStream(), 5)
        assertNull(cache.lookup(uri, stat))
        cache.record(uri, stat, digest)
        assertEquals(digest, cache.lookup(uri, stat))
        // A destructive boundary re-validates the same descriptor: no second read.
        repeat(3) { assertEquals(digest, cache.lookup(uri, stat)) }
        // Any stat change drops the cached digest, so the part is hashed again.
        for (changed in listOf(
            stat.copy(inode = stat.inode + 1), stat.copy(size = stat.size + 1),
            stat.copy(modified = stat.modified + 1), stat.copy(changed = stat.changed + 1),
        )) assertNull(cache.lookup(uri, changed))
        // A replacement document URI never inherits another part's verification.
        assertNull(cache.lookup(original.copy(document = "43",
            uri = "content://provider/document/43").uri, stat))
    }
}
