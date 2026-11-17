package org.kiwix.mobile

/** Fail-closed startup and a write-completion barrier for runtime read-only changes. */
internal class CrashCapturePolicy {
    private var enabled = false
    @Synchronized fun setEnabled(value: Boolean) { enabled = value }
    @Synchronized fun capture(write: () -> Unit) { if (enabled) write() }
}
