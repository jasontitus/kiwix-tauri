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
        // A reader page, an archive or provider link and a path may all be
        // written with spaces, so each is redacted to the end of its line, and
        // an external address keeps its scheme and host only. The shared cases
        // in CrashRedactionTest pin every shape on its own.
        assertEquals(
            "[reader-url] [reader-url] https://example.org path=[path] [url] [url] [url]",
            text
        )
        assertEquals("https://example.org",
            CrashIntake.redact("https://user:pass@example.org/read?q=private#private"))
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
