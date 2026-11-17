package org.kiwix.mobile

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Build
import android.os.Bundle
import android.os.CancellationSignal
import android.os.Handler
import android.os.Looper
import androidx.annotation.RequiresApi
import androidx.core.content.ContextCompat
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import java.util.concurrent.Executor
import java.util.function.Consumer

/** Rejection codes; location_android.rs maps them to Geolocation API error codes. */
internal object ReaderLocationCode {
    const val DENIED = "LOCATION_DENIED"
    const val UNAVAILABLE = "LOCATION_UNAVAILABLE"
    const val TIMEOUT = "LOCATION_TIMEOUT"
    const val CANCELLED = "LOCATION_CANCELLED"
}

/**
 * One fresh location fix per request for archives Kiwix has already consented
 * (src-tauri/src/permissions.rs). Archives never get the WebView's own
 * geolocation; Rust allocates a process-wide token per request, starts it with
 * `startLocation` and ends it with `cancelLocation`.
 *
 * Every entry point and every platform callback runs on the plugin's main
 * Handler, so the registry needs no locking. The page's timeout starts only
 * once Android permission is settled, like the Apple and Windows providers.
 * PluginManager keeps a single permission callback, so at most one location
 * sheet is open; requests that arrive meanwhile wait for that sheet's answer.
 */
internal class ReaderLocation(private val context: Context, private val main: Handler) {
    private class Request(val token: Long, val invoke: Invoke, val highAccuracy: Boolean, val timeoutMs: Long) {
        var finished = false
        var cancellation: CancellationSignal? = null
        var listener: LocationListener? = null
        var timeout: Runnable? = null
    }

    private val requests = HashMap<Long, Request>()
    /** Waiting for the open permission sheet, in arrival order. */
    private val awaiting = ArrayList<Request>()
    /** Cancels that arrived before their start (two async Rust calls may reorder). */
    private val earlyCancels = LinkedHashSet<Long>()
    private var promptOpen = false
    private var promptHighAccuracy = false
    /** The user kept approximate location when asked for precise: do not ask again this process. */
    private var preciseDeclined = false

    private val manager: LocationManager?
        get() = context.getSystemService(Context.LOCATION_SERVICE) as? LocationManager

    private fun granted(permission: String): Boolean =
        ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED
    private fun fine(): Boolean = granted(Manifest.permission.ACCESS_FINE_LOCATION)
    private fun anyLocation(): Boolean = fine() || granted(Manifest.permission.ACCESS_COARSE_LOCATION)

    /**
     * Main thread. `prompt(alias)` opens the plugin's permission sheet; it
     * throws when no visible Activity can show it.
     */
    fun start(invoke: Invoke, prompt: (String) -> Unit) {
        val request = try {
            val args = invoke.getArgs()
            Request(args.getLong("token"), invoke, args.optBoolean("highAccuracy", false),
                args.optLong("timeoutMs", 30_000L).coerceIn(1L, 30_000L))
        } catch (_: Exception) {
            invoke.reject("Invalid location request", ReaderLocationCode.UNAVAILABLE)
            return
        }
        if (earlyCancels.remove(request.token)) {
            invoke.reject(CANCELLED_MESSAGE, ReaderLocationCode.CANCELLED)
            return
        }
        if (requests.containsKey(request.token)) {
            invoke.reject("Duplicate location request", ReaderLocationCode.UNAVAILABLE)
            return
        }
        requests[request.token] = request
        // ACCESS_FINE_LOCATION alone is ignored on Android 12+, so the precise
        // alias carries both; the user may still keep approximate location.
        val needsPrompt = if (request.highAccuracy) !fine() && !(preciseDeclined && anyLocation()) else !anyLocation()
        if (!needsPrompt) {
            begin(request)
            return
        }
        awaiting.add(request)
        if (promptOpen) return
        try {
            promptOpen = true
            promptHighAccuracy = request.highAccuracy
            prompt(if (request.highAccuracy) PRECISE_ALIAS else APPROXIMATE_ALIAS)
        } catch (_: Exception) {
            promptOpen = false
            awaiting.remove(request)
            // A high-accuracy hint may fall back to an existing approximate grant.
            if (anyLocation()) begin(request)
            else fail(request, ReaderLocationCode.UNAVAILABLE, "Open Kiwix to allow location access")
        }
    }

    /** Main thread; the sheet's answer (or a resume that proves it closed). */
    fun permissionSettled() {
        if (!promptOpen) return
        promptOpen = false
        if (promptHighAccuracy && !fine() && anyLocation()) preciseDeclined = true
        val waiting = awaiting.toList()
        awaiting.clear()
        val allowed = anyLocation()
        for (request in waiting) {
            if (request.finished) continue
            if (allowed) begin(request)
            else fail(request, ReaderLocationCode.DENIED, DENIED_MESSAGE)
        }
    }

    /**
     * Main thread. The permission result is delivered before onResume, so a
     * sheet still marked open afterwards lost its callback (PluginManager keeps
     * only one). Re-read the grant instead of leaving requests waiting.
     */
    fun resumed() {
        if (promptOpen) main.post { permissionSettled() }
    }

    /** Main thread; idempotent. Unknown tokens are remembered briefly. */
    fun cancel(token: Long) {
        val request = requests[token]
        if (request == null) {
            earlyCancels.add(token)
            while (earlyCancels.size > EARLY_CANCEL_LIMIT) earlyCancels.remove(earlyCancels.first())
            return
        }
        fail(request, ReaderLocationCode.CANCELLED, CANCELLED_MESSAGE)
    }

    /** Main thread; the Activity was retired or destroyed. */
    fun retire() {
        // promptOpen stays set: the system sheet may still be showing, and a
        // second request would make Android answer it at once as denied.
        // permissionSettled() or the next resumed() clears it.
        awaiting.clear()
        for (request in requests.values.toList()) fail(request, ReaderLocationCode.CANCELLED, CANCELLED_MESSAGE)
    }

    @SuppressLint("MissingPermission")
    private fun begin(request: Request) {
        if (request.finished) return
        if (!anyLocation()) {
            fail(request, ReaderLocationCode.DENIED, DENIED_MESSAGE)
            return
        }
        val locations = manager ?: run {
            fail(request, ReaderLocationCode.UNAVAILABLE, "This device has no location service")
            return
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P && !locations.isLocationEnabled) {
            fail(request, ReaderLocationCode.UNAVAILABLE, LOCATION_OFF_MESSAGE)
            return
        }
        val provider = provider(locations, request.highAccuracy, fine()) ?: run {
            fail(request, ReaderLocationCode.UNAVAILABLE, LOCATION_OFF_MESSAGE)
            return
        }
        val timeout = Runnable { fail(request, ReaderLocationCode.TIMEOUT, "Android could not get a location in time") }
        request.timeout = timeout
        main.postDelayed(timeout, request.timeoutMs)
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) currentLocation(locations, provider, request)
            else singleUpdate(locations, provider, request)
        } catch (_: SecurityException) {
            // Revoked between the check and the request.
            fail(request, ReaderLocationCode.DENIED, DENIED_MESSAGE)
        } catch (error: Exception) {
            fail(request, ReaderLocationCode.UNAVAILABLE, "Android could not get a location: " + (error.message ?: error.javaClass.simpleName))
        }
    }

    /** GPS for high accuracy with a precise grant, else fused (API 31+) or network. */
    private fun provider(manager: LocationManager, highAccuracy: Boolean, fine: Boolean): String? {
        val enabled = manager.getProviders(true)
        val fused = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) LocationManager.FUSED_PROVIDER else null
        // Fused answers in seconds indoors, where raw GPS usually times out.
        val order = if (highAccuracy && fine) listOfNotNull(fused, LocationManager.GPS_PROVIDER, LocationManager.NETWORK_PROVIDER)
            else listOfNotNull(fused, LocationManager.NETWORK_PROVIDER, LocationManager.GPS_PROVIDER.takeIf { fine })
        return order.firstOrNull { enabled.contains(it) }
    }

    @RequiresApi(Build.VERSION_CODES.R)
    @SuppressLint("MissingPermission")
    private fun currentLocation(manager: LocationManager, provider: String, request: Request) {
        val cancellation = CancellationSignal()
        request.cancellation = cancellation
        val executor = Executor { command -> main.post(command) }
        manager.getCurrentLocation(provider, cancellation, executor, Consumer<Location> { location -> deliver(request, location) })
    }

    @SuppressLint("MissingPermission")
    private fun singleUpdate(manager: LocationManager, provider: String, request: Request) {
        // Every method is overridden: LocationListener has no default methods
        // before API 30, so a missing override would be an AbstractMethodError.
        val listener = object : LocationListener {
            override fun onLocationChanged(location: Location) { deliver(request, location) }
            @Suppress("OVERRIDE_DEPRECATION", "DEPRECATION")
            override fun onStatusChanged(provider: String?, status: Int, extras: Bundle?) {}
            override fun onProviderEnabled(provider: String) {}
            override fun onProviderDisabled(disabled: String) {
                if (disabled == provider) fail(request, ReaderLocationCode.UNAVAILABLE, LOCATION_OFF_MESSAGE)
            }
        }
        request.listener = listener
        manager.requestLocationUpdates(provider, 0L, 0f, listener, Looper.getMainLooper())
    }

    private fun deliver(request: Request, location: Location?) {
        if (request.finished) return
        if (location == null) {
            fail(request, ReaderLocationCode.UNAVAILABLE, "Android could not determine the location")
            return
        }
        val position = JSObject()
            .put("latitude", location.latitude)
            .put("longitude", location.longitude)
            .put("timestamp", location.time)
        // Rust refuses a fix without accuracy rather than inventing one.
        if (location.hasAccuracy()) position.put("accuracy", location.accuracy.toDouble())
        if (location.hasAltitude()) position.put("altitude", location.altitude)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && location.hasVerticalAccuracy())
            position.put("altitudeAccuracy", location.verticalAccuracyMeters.toDouble())
        if (location.hasBearing()) position.put("heading", location.bearing.toDouble())
        if (location.hasSpeed()) position.put("speed", location.speed.toDouble())
        if (!finish(request)) return
        request.invoke.resolve(position)
    }

    private fun fail(request: Request, code: String, message: String) {
        if (!finish(request)) return
        request.invoke.reject(message, code)
    }

    /** Releases platform work exactly once; false when already finished. */
    private fun finish(request: Request): Boolean {
        if (request.finished) return false
        request.finished = true
        if (requests[request.token] === request) requests.remove(request.token)
        awaiting.remove(request)
        request.timeout?.let { main.removeCallbacks(it) }
        request.timeout = null
        request.cancellation?.let { try { it.cancel() } catch (_: Exception) {} }
        request.cancellation = null
        request.listener?.let { listener -> try { manager?.removeUpdates(listener) } catch (_: Exception) {} }
        request.listener = null
        return true
    }

    companion object {
        const val PRECISE_ALIAS = "readerLocationPrecise"
        const val APPROXIMATE_ALIAS = "readerLocationApproximate"
        private const val EARLY_CANCEL_LIMIT = 64
        private const val DENIED_MESSAGE = "Location permission was denied for Kiwix in Android settings"
        private const val LOCATION_OFF_MESSAGE = "Location is turned off on this device. Turn on Location in Android settings."
        private const val CANCELLED_MESSAGE = "Location request was cancelled"
    }
}
