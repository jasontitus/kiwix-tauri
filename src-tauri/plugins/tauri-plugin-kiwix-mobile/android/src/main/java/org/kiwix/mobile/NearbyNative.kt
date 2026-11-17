package org.kiwix.mobile

import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean

/** Fixed safe error text only; no platform exception, SSID or credential crosses this boundary. */
internal class NearbyError(val code: String) : Exception(code)
internal fun nearbyRequire(value: Boolean, code: String) { if (!value) throw NearbyError(code) }
internal fun nearbyUuid(value: String): String {
    nearbyRequire(value.length == 36 && runCatching { UUID.fromString(value).toString() == value }.getOrDefault(false), "NEARBY_INVALID_ARGUMENT")
    return value
}
internal class NearbyReply(val invoke: Invoke) {
    private val settled = AtomicBoolean(false)
    fun resolve(value: JSObject = JSObject()) { if (settled.compareAndSet(false, true)) invoke.resolve(value) }
    fun reject(code: String) { if (settled.compareAndSet(false, true)) invoke.reject(code, code) }
}
internal fun nearbyFailure(error: Exception): String =
    if (error is NearbyError) error.code else if (error is SecurityException) "NEARBY_PERMISSION_DENIED" else "NEARBY_NATIVE_FAILED"
