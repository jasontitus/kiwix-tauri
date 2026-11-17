package org.kiwix.mobile

import com.fasterxml.jackson.databind.ObjectMapper
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NativePdfCancellationTest {
    @Test fun rustDropPayloadRetiresWaitingRendererAndMatchingExportBeforeAcknowledgement() {
        // Exact NativeJob::drop payload: no `id` alias is supplied.
        val payload = ObjectMapper().readTree("""{"jobId":"native-pdf-17"}""")
        val effects = mutableListOf<String>()
        var rendererWaiting = true
        var copyCancelled = false
        var chooserPresented = false
        val lateRendererCallback = { if (rendererWaiting) chooserPresented = true }
        val lateCopyCallback = { if (!copyCancelled) chooserPresented = true }
        cancelArticlePdfJob(
            { key -> requireNotNull(payload[key]) { "Missing $key" }.asText() },
            { id ->
                assertEquals("native-pdf-17", id)
                rendererWaiting = false
                effects.add("renderer-retired")
            },
            "native-pdf-17",
        ) {
            copyCancelled = true
            effects.add("export-cancelled")
        }
        effects.add("acknowledged")
        lateRendererCallback()
        lateCopyCallback()
        assertFalse(rendererWaiting)
        assertTrue(copyCancelled)
        assertFalse(chooserPresented)
        assertEquals(listOf("renderer-retired", "export-cancelled", "acknowledged"), effects)
    }

    @Test fun cancellationNeverCancelsAnotherJobOrAnExportWithoutPdfAuthority() {
        for (other in listOf(null, "", "native-pdf-18")) {
            val payload = ObjectMapper().readTree("""{"jobId":"native-pdf-17"}""")
            var rendererCalls = 0
            var exportCalls = 0
            repeat(2) {
                cancelArticlePdfJob(
                    { key -> requireNotNull(payload[key]).asText() },
                    { id -> assertEquals("native-pdf-17", id); rendererCalls += 1 },
                    other,
                ) { exportCalls += 1 }
            }
            assertEquals(2, rendererCalls)
            assertEquals(0, exportCalls)
        }
    }
}
