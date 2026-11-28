package org.kiwix.mobile

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A service started with startForegroundService must call startForeground once
 * before it stops, or Android kills the application.
 */
class TransferStartDecisionTest {
    @Test
    fun pendingControlSignalStopsAFreshServiceOnlyAfterEnteringTheForeground() {
        val decision = transferStartDecision(admitted = true, foreground = false) { true }
        assertEquals(TransferStartStep.CONTROL_PENDING, decision.step)
        assertTrue(decision.foregroundFirst)
    }

    @Test
    fun staleStartStopsAFreshServiceOnlyAfterEnteringTheForeground() {
        var read = false
        val decision = transferStartDecision(admitted = false, foreground = false) { read = true; true }
        assertEquals(TransferStartStep.STALE, decision.step)
        assertTrue(decision.foregroundFirst)
        // The disk-backed signal store is not read for a start that is refused anyway.
        assertFalse(read)
    }

    @Test
    fun runningServiceIsAlreadyInTheForeground() {
        // Direct updates of the admitted instance never repeat the notification.
        assertEquals(TransferStartDecision(TransferStartStep.CONTROL_PENDING, false),
            transferStartDecision(admitted = true, foreground = true) { true })
        assertEquals(TransferStartDecision(TransferStartStep.STALE, false),
            transferStartDecision(admitted = false, foreground = true) { false })
    }

    @Test
    fun admittedStartWithoutPendingSignalsRunsNormally() {
        for (foreground in listOf(false, true))
            assertEquals(TransferStartDecision(TransferStartStep.ADMIT, false),
                transferStartDecision(admitted = true, foreground = foreground) { false })
    }

    @Test
    fun queuedForegroundStartIsNeverStoppedFromOutside() {
        // Stopping the service under a start Android has not delivered yet
        // kills the process; the start is left to arrive stale and stop itself.
        assertEquals(TransferStop.LEAVE_TO_QUEUED_START, transferStopDecision(queuedForegroundStarts = 1))
        assertEquals(TransferStop.LEAVE_TO_QUEUED_START, transferStopDecision(queuedForegroundStarts = 2))
        assertEquals(TransferStop.STOP_SERVICE, transferStopDecision(queuedForegroundStarts = 0))
    }

    @Test
    fun serviceStopsItselfOnlyUpToAStartAndroidDelivered() {
        // No start delivered yet: the start that created the instance is queued.
        assertEquals(null, transferSelfStopId(deliveredStartId = 0))
        assertEquals(1, transferSelfStopId(deliveredStartId = 1))
        assertEquals(7, transferSelfStopId(deliveredStartId = 7))
    }
}
