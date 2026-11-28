package org.kiwix.mobile

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

class CrashCapturePolicyTest {
    @Test fun reportTextRedactsReaderLocationsAndNonHttpUrls() {
        val text = CrashIntake.redact(
            """
            HTTP://book-uuid-nonce.localhost.:9/A/Pri)vate_Page
            http://127.0.0.2:9/A/Private http://[::1]:9/A/Private
            https://example.org/book-uuid/%2Ekiwix/token/A/Private
            path=/book-uuid/A/Private relative=/.kiwix/token/A/Private
            ZIM://book-uuid/A/Private MAILTO:private@example.org
            content://provider/Private file:///Documents/Private%20Notes.txt
            custom://host/Private https://user:pass@example.org/read?q=private#private
            """.trimIndent()
        )
        // A path may contain spaces, so it is redacted to the end of its line:
        // the second route on that line goes with the first and is checked alone below.
        assertEquals(
            "[reader-url] [reader-url] [reader-url] [reader-url] path=[path] " +
                "[url] [url] [url] [url] [url] https://example.org/read",
            text
        )
        assertEquals("relative=[path]", CrashIntake.redact("relative=/.kiwix/token/A/Private"))
        assertEquals("[url]", CrashIntake.redact("file:///Documents/Private%20Notes.txt"))
    }

    @Test fun redactionKeepsNoPartOfAPathThatContainsSpaces() {
        // Stopping at the first space kept ` Books/Health Records/…`: the rest
        // of the folder and file names.
        val storage = CrashIntake.redact(
            "open /storage/emulated/0/My Books/Health Records/medical_wikipedia.zim failed")
        assertEquals("open [path]", storage)
        val cause = CrashIntake.redact(
            "/storage/emulated/0/My Books/Health Records/medical_wikipedia.zim: open failed: EACCES (Permission denied)")
        assertEquals("[path]: open failed: EACCES (Permission denied)", cause)
        val windows = CrashIntake.redact(
            "Cannot parse C:\\Users\\Jane Doe\\AppData\\Roaming\\kiwix\\My Library\\state.json: expected value at line 1 column 1")
        assertEquals("Cannot parse [path]: expected value at line 1 column 1", windows)
        for (leaked in listOf("Books", "Health", "medical_wikipedia", "Doe", "AppData", "Library"))
            for (shown in listOf(storage, cause, windows)) assertFalse("$leaked in $shown", shown.contains(leaked))

        // A quoted path ends at its closing quote, so the cause survives.
        assertEquals(
            "cannot read \"[path]\" (os error 13)",
            CrashIntake.redact("cannot read \"/storage/emulated/0/My Books/a.zim\" (os error 13)"))
        // An escaped quote inside the quoted text does not end it.
        assertEquals(
            "cannot read \"[path]\" (os error 5)",
            CrashIntake.redact("cannot read \"C:\\\\Users\\\\Jane Doe\\\\\" (os error 5)"))
        // An apostrophe is part of a name, not the end of the path.
        assertEquals(
            "missing '[path]",
            CrashIntake.redact("missing '/storage/emulated/0/Jane's Backup/Private Notes/a.zim' on disk"))
        // A share, a home-relative path, and a path cut short by a line break.
        assertEquals("at [path]", CrashIntake.redact("at \\\\nas\\Family Share\\a.zim now"))
        assertEquals("at [path]", CrashIntake.redact("at ~/My Documents/Private Notes.txt now"))
        assertEquals("first [path] second line",
            CrashIntake.redact("first /sdcard/My Books/a.zim\nsecond line"))
    }

    @Test fun fileUrlsWithSpacesAreRedactedToTheEndOfThePath() {
        assertEquals("open [url]",
            CrashIntake.redact("open file:///storage/emulated/0/My Books/Private Notes.txt now"))
        assertEquals("[url]: denied",
            CrashIntake.redact("FILE:///sdcard/My Books/Private Notes.txt: denied"))
        assertEquals("read \"[url]\" failed",
            CrashIntake.redact("read \"file:///sdcard/My Books/a.zim\" failed"))
        // Other URLs still end at whitespace; the words after them are kept.
        assertEquals("[url] was revoked",
            CrashIntake.redact("content://com.android.providers/document/My%20Books was revoked"))
        assertEquals("https://example.org/read then stopped",
            CrashIntake.redact("https://example.org/read?q=private then stopped"))
    }

    @Test fun stackFramesKeepTheirFileAndLine() {
        for (frame in listOf(
            "org.kiwix.mobile.OwnedDocuments.copy(OwnedDocuments.kt:259)",
            "java.io.FileOutputStream.write(FileOutputStream.java:381)",
            "android.os.Handler.dispatchMessage(Handler.java:106)",
            "OwnedDocuments.kt:259",
        )) assertEquals(frame, CrashIntake.redact(frame))
    }

    @Test fun policyStartsDisabledAndCanBeDisabledAgain() {
        val policy = CrashCapturePolicy()
        var writes = 0
        policy.capture { ++writes }
        assertEquals(0, writes)
        policy.setEnabled(true)
        policy.capture { ++writes }
        policy.setEnabled(false)
        policy.capture { ++writes }
        assertEquals(1, writes)
    }

    @Test fun disableAcknowledgesOnlyAfterAnAlreadyAdmittedWriteCompletes() {
        val policy = CrashCapturePolicy()
        policy.setEnabled(true)
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val disabling = CountDownLatch(1)
        val disabled = CountDownLatch(1)
        val writes = AtomicInteger()
        val writer = Thread {
            policy.capture {
                entered.countDown()
                check(release.await(5, TimeUnit.SECONDS))
                writes.incrementAndGet()
            }
        }
        val cutover = Thread {
            disabling.countDown()
            policy.setEnabled(false)
            disabled.countDown()
        }
        writer.start()
        assertTrue(entered.await(5, TimeUnit.SECONDS))
        cutover.start()
        assertTrue(disabling.await(5, TimeUnit.SECONDS))
        assertFalse(disabled.await(50, TimeUnit.MILLISECONDS))
        release.countDown()
        writer.join(5000)
        cutover.join(5000)
        assertTrue(disabled.await(5, TimeUnit.SECONDS))
        policy.capture { writes.incrementAndGet() }
        assertEquals(1, writes.get())
    }
}
