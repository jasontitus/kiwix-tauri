package org.kiwix.mobile

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MobileIntakePolicyTest {
    @Test
    fun queuedScreenUpdateCannotRebaseOntoAReplacementActivity() {
        val original = Any()
        val replacement = Any()
        assertTrue(activityScreenCallbackCurrent(original, 7, original, 7, original))
        assertFalse(activityScreenCallbackCurrent(original, 7, replacement, 8, replacement))
        assertFalse(activityScreenCallbackCurrent(original, 7, original, 8, original))
        assertFalse(activityScreenCallbackCurrent(original, 7, original, 7, replacement))
        assertFalse(activityScreenCallbackCurrent(original, 7, null, 7, null))
        assertFalse(activityScreenCallbackCurrent(null, 7, replacement, 8, replacement))
        assertTrue(activityScreenCallbackCurrent(null, 7, null, 7, null))
    }

    @Test
    fun obscuredActivityKeepsForegroundOwnershipUntilStopped() {
        val state = ActivityWindowState()
        state.resumed()
        state.paused()
        assertTrue(state.foreground)
        assertFalse(state.interactive)
        state.paused()
        assertTrue(state.foreground)
        state.stopped()
        assertFalse(state.foreground)
        assertFalse(state.interactive)
        state.resumed()
        assertTrue(state.foreground)
        assertTrue(state.interactive)
    }

    @Test
    fun pauseCannotAdmitAColdOrRetiredWindow() {
        val state = ActivityWindowState()
        state.paused()
        assertFalse(state.foreground)
        assertFalse(state.interactive)
        state.restore(started = true, resumed = false)
        assertTrue(state.foreground)
        assertFalse(state.interactive)
        state.stopped()
        state.paused()
        assertFalse(state.foreground)
        assertFalse(state.interactive)
    }

    @Test
    fun stoppedActivityAlwaysReportsSuspension() {
        // onPause may have reported inactivity for a dialog, and its flag may
        // be clear by onStop. A real stop must still suspend the engine.
        for (notificationPending in listOf(false, true)) {
            val (event, _) = stoppedActivityLifecycle(notificationPending)
            assertEquals("mobile-suspended", event)
        }
    }

    @Test
    fun stoppedActivityDistinguishesNotificationBackgrounding() {
        assertEquals(
            "activity-stopped",
            stoppedActivityLifecycle(notificationPending = false).second,
        )
        assertEquals(
            "notification-permission-backgrounded",
            stoppedActivityLifecycle(notificationPending = true).second,
        )
    }

    @Test
    fun intakePersistsOnlyWhenEveryGrantIsDurable() {
        assertTrue(intakeCanPersist(listOf(true)))
        assertTrue(intakeCanPersist(listOf(true, true)))
        assertFalse(intakeCanPersist(listOf(true, false)))
        assertFalse(intakeCanPersist(listOf(false)))
        assertFalse(intakeCanPersist(emptyList()))
    }

    @Test
    fun coldStartNoticeOnlyWhenTransientBatchWasLost() {
        assertTrue(intakeColdStartNotice(transientMarker = true, inMemoryBatches = 0))
        assertFalse(intakeColdStartNotice(transientMarker = true, inMemoryBatches = 1))
        assertFalse(intakeColdStartNotice(transientMarker = false, inMemoryBatches = 0))
    }

    @Test
    fun firstLaunchIntentIsOfferedOnce() {
        // "Open with", a link or a shortcut starting the app: no saved state.
        assertTrue(launchIntentConsumable(restored = false, flags = 0))
        assertTrue(launchIntentConsumable(restored = false, flags = android.content.Intent.FLAG_ACTIVITY_NEW_TASK))
    }

    @Test
    fun restoredOrHistoryLaunchNeverReplaysItsIntent() {
        // After process death Android restores the Activity with the original
        // intent, without the in-process handled extras.
        assertFalse(launchIntentConsumable(restored = true, flags = 0))
        // Reopened from Recents after the task was finished.
        val history = android.content.Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY
        assertFalse(launchIntentConsumable(restored = false, flags = history))
        assertFalse(launchIntentConsumable(restored = false,
            flags = history or android.content.Intent.FLAG_ACTIVITY_NEW_TASK))
        assertFalse(launchIntentConsumable(restored = true, flags = history, unofferedByPredecessor = true))
    }

    @Test
    fun recreationBeforeThePluginSawTheIntentKeepsItConsumable() {
        // A configuration change during a cold "Open with" start, before any
        // plugin was offered the intent, must not lose the file.
        assertTrue(launchIntentConsumable(restored = true, flags = 0, unofferedByPredecessor = true))
        assertFalse(launchIntentConsumable(restored = true, flags = 0, unofferedByPredecessor = false))
    }

    @Test
    fun replacedActivityFailsAPendingOperationInsteadOfCancellingIt() {
        for (kind in listOf("archives", "directory", "external", "externalBatch", "text", "scanFolder", "open")) {
            assertEquals(kind, RetirementOutcome.FAIL_RECREATED,
                retirementOutcome(kind, copying = false, viewerScratch = false, replaced = true))
            // Only the person closing the app still reads as their own cancellation.
            assertEquals(kind, RetirementOutcome.CANCELLED,
                retirementOutcome(kind, copying = false, viewerScratch = false, replaced = false))
        }
        // A viewer may already hold its grant: retained and reported either way.
        for (replaced in listOf(false, true)) assertEquals(RetirementOutcome.FAIL_DESTROYED,
            retirementOutcome("open", copying = false, viewerScratch = true, replaced = replaced))
        // A running worker removes what it staged before the operation settles.
        for (replaced in listOf(false, true)) for (viewer in listOf(false, true))
            assertEquals(RetirementOutcome.AWAIT_WORKER,
                retirementOutcome("archives", copying = true, viewerScratch = viewer, replaced = replaced))
    }

    @Test
    fun replacedActivityStillCancelsASaveThatWroteNothing() {
        // Rust deletes the file it prepared for a save only when the save is
        // reported as cancelled; a failure would leave that file behind.
        assertEquals(RetirementOutcome.CANCELLED,
            retirementOutcome("export", copying = false, viewerScratch = false, replaced = true))
        assertFalse(replacementFails("export"))
        for (kind in listOf("archives", "directory", "external", "externalBatch", "text", "scanFolder", "open"))
            assertTrue(kind, replacementFails(kind))
    }
}
