package org.kiwix.mobile

import android.annotation.SuppressLint
import android.content.Context
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import androidx.annotation.RequiresApi
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject

/**
 * Owns the single Android LocalOnlyHotspot reservation.
 *
 * All entry points run on the plugin's main Handler, which is also the callback
 * Handler, so the reservation and its pending start need no extra locking. The
 * platform credentials are read from the reservation itself; nothing is guessed
 * or hardcoded.
 */
@RequiresApi(Build.VERSION_CODES.O)
internal class LocalHotspot(
    private val context: Context,
    private val main: Handler,
    private val lost: () -> Unit,
    private val onActive: (Boolean) -> Unit,
) {
    private val wifi: WifiManager
        get() = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager

    private var reservation: WifiManager.LocalOnlyHotspotReservation? = null
    private var pending: Invoke? = null
    /** A Stop arrived while the platform was still starting; close on onStarted. */
    private var stopRequested = false
    /** The close below is ours, so onStopped must not report a platform loss. */
    private var closing = false
    private var generation = 0L
    private data class InterfaceAddress(val name: String, val index: Int, val address: java.net.Inet4Address, val prefix: Int)
    private data class InterfaceIdentity(val name: String, val index: Int)
    private var beforeInterfaces: Set<InterfaceIdentity> = emptySet()
    private var beforeClientInterfaces: Set<String> = emptySet()
    private fun interfaceIdentities(): Set<InterfaceIdentity> =
        java.net.NetworkInterface.getNetworkInterfaces()?.toList().orEmpty().map {
            InterfaceIdentity(it.name, it.index)
        }.toSet()
    private var quicInterface: InterfaceAddress? = null
    private var quicLease: String? = null
    private fun interfaces(): Set<InterfaceAddress> =
        java.net.NetworkInterface.getNetworkInterfaces()?.toList().orEmpty().filter {
            it.isUp && !it.isLoopback && !it.isPointToPoint
        }.flatMap { iface ->
            iface.interfaceAddresses.filter { it.address is java.net.Inet4Address &&
                (it.address.isSiteLocalAddress || it.address.isLinkLocalAddress) && it.networkPrefixLength.toInt() in 1..32
            }.map { InterfaceAddress(iface.name, iface.index, it.address as java.net.Inet4Address, it.networkPrefixLength.toInt()) }
        }.toSet()
    private fun physicalNetworkNames(): Set<String> {
        val connectivity = context.getSystemService(Context.CONNECTIVITY_SERVICE) as android.net.ConnectivityManager
        return connectivity.allNetworks.mapNotNull { connectivity.getLinkProperties(it)?.interfaceName }.toSet()
    }
    /** Only a unique newly appeared, non-client interface can belong to our AP.
     * A soft-AP interface may survive a previous reservation with the same
     * identity; its AP name is still unambiguous and not a client network. */
    private fun accessPointName(name: String): Boolean =
        name == "wlan1" || name.startsWith("ap") || name.startsWith("swlan") || name.startsWith("softap")
    private fun currentQuicInterface(): InterfaceAddress? {
        if (reservation == null || closing || stopRequested) return null
        val current = interfaces()
        quicInterface?.let { return it.takeIf { it in current } }
        val clientInterfaces = physicalNetworkNames()
        val candidates = current.filter { candidate ->
            (beforeInterfaces.none { it.name == candidate.name || it.index == candidate.index } || accessPointName(candidate.name)) &&
                candidate.name !in beforeClientInterfaces
        }
        if (candidates.size != 1) {
            android.util.Log.i("KiwixHotspot", "quic interface candidates=${candidates.size} current=$current before=$beforeInterfaces client=$clientInterfaces beforeClient=$beforeClientInterfaces")
            return null
        }
        return candidates.single().also { quicInterface = it; quicLease = java.util.UUID.randomUUID().toString() }
    }
    fun quicRoute(invoke: Invoke) {
        try {
            val args = invoke.getArgs()
            nearbyRequire(reservation != null && pending == null, "NEARBY_HOTSPOT_NOT_OWNED")
            val iface = currentQuicInterface() ?: throw NearbyError("NEARBY_HOTSPOT_EXACT_INTERFACE_UNAVAILABLE")
            if (args.has("lease")) nearbyRequire(args.getString("lease") == quicLease, "NEARBY_HOTSPOT_ROUTE_LOST")
            if (args.has("address")) {
                val peer = NearbyRouteChecks.address(args.getString("address"))
                val number = peer.address.fold(0L) { result, byte -> (result shl 8) or (byte.toLong() and 255L) }
                val hostBits = 32 - iface.prefix
                val hostMask = if (hostBits == 0) 0L else (1L shl hostBits) - 1L
                nearbyRequire(peer != iface.address && inPrefix(iface.address, peer, iface.prefix) &&
                    (hostBits <= 1 || (number and hostMask) != 0L && (number and hostMask) != hostMask), "NEARBY_ENDPOINT_NOT_ON_HOTSPOT")
            }
            nearbyRequire(currentQuicInterface() == iface, "NEARBY_HOTSPOT_ROUTE_LOST")
            // The Rust socket owner checks the socket type through the OS
            // and exact SO_BINDTODEVICE + address binding. Android's Java Os
            // interface-binding/type-inspection methods are not public SDK APIs.
            invoke.resolve(JSObject().put("address", iface.address.hostAddress).put("lease", quicLease)
                .put("interfaceName", iface.name).put("interfaceIndex", iface.index).put("prefixLength", iface.prefix))
        } catch (error: Exception) {
            val code = nearbyFailure(error)
            invoke.reject(code, code)
        }
    }
    fun quicState(args: org.json.JSONObject): JSObject {
        val iface = quicInterface
        return JSObject().put("available", iface != null && args.getString("lease") == quicLease && currentQuicInterface() == iface)
    }

    fun snapshot(): JSObject? = reservation?.let { describe(it) }

    @SuppressLint("MissingPermission")
    fun start(invoke: Invoke) {
        reservation?.let { current ->
            // Repeated Share/Browse setup must retain the live reservation and
            // its route lease, not disconnect a receiver that is joining it.
            val info = describe(current)
            if (info != null) invoke.resolve(info)
            else invoke.reject("HOTSPOT_CONFIGURATION_UNAVAILABLE",
                "Android did not publish the active hotspot's network name and password")
            return
        }
        if (pending != null) {
            invoke.reject("HOTSPOT_START_PENDING", "The Wi-Fi hotspot is already starting")
            return
        }
        beforeInterfaces = interfaceIdentities()
        beforeClientInterfaces = physicalNetworkNames()
        quicInterface = null; quicLease = null
        pending = invoke
        stopRequested = false
        closing = false
        val attempt = ++generation
        android.util.Log.i("KiwixHotspot", "calling startLocalOnlyHotspot")
        try {
            wifi.startLocalOnlyHotspot(callback(attempt), main)
        } catch (error: SecurityException) {
            pending = null
            invoke.reject("HOTSPOT_PERMISSION_DENIED",
                "Android refused the hotspot. Allow nearby Wi-Fi access and turn on Location services, then try again.")
        } catch (error: IllegalStateException) {
            pending = null
            invoke.reject("HOTSPOT_UNAVAILABLE",
                "Android refused the hotspot: ${error.message ?: "another hotspot request is already active"}")
        } catch (error: Exception) {
            pending = null
            invoke.reject("HOTSPOT_UNAVAILABLE",
                "Android refused the hotspot: ${error.message ?: error.javaClass.simpleName}")
        }
    }

    fun stop(invoke: Invoke) {
        android.util.Log.i("KiwixHotspot", "stop active=${reservation != null} pending=${pending != null}")
        val reservation = reservation
        val pending = pending
        this.reservation = null
        this.pending = null
        val previousGeneration = generation
        generation++
        // Only a still-pending start needs to learn that it was cancelled.
        quicInterface = null; quicLease = null
        stopRequested = pending != null
        if (reservation == null && pending == null) {
            invoke.resolve(JSObject())
            return
        }
        closing = reservation != null
        try {
            reservation?.close()
        } catch (error: Exception) {
            closing = false
            this.reservation = reservation
            generation = previousGeneration
            invoke.reject("HOTSPOT_RELEASE_FAILED",
                "Android did not release the hotspot: ${error.message ?: error.javaClass.simpleName}")
            return
        }
        pending?.reject("HOTSPOT_START_CANCELLED", "The Wi-Fi hotspot was stopped before it finished starting")
        if (reservation != null) onActive(false)
        invoke.resolve(JSObject())
    }

    /** Activity retirement and process teardown: no caller waits for a result. */
    fun release(): Boolean {
        val reservation = reservation
        val pending = pending
        this.reservation = null
        this.pending = null
        val previousGeneration = generation
        generation++
        quicInterface = null; quicLease = null
        stopRequested = pending != null
        closing = reservation != null
        pending?.reject("HOTSPOT_START_CANCELLED", "The Wi-Fi hotspot was stopped before it finished starting")
        try {
            reservation?.close()
        } catch (error: Exception) {
            closing = false
            this.reservation = reservation
            generation = previousGeneration
        }
        if (reservation != null) onActive(false)
        return reservation != null || pending != null
    }

    private fun callback(attempt: Long) = object : WifiManager.LocalOnlyHotspotCallback() {
        override fun onStarted(started: WifiManager.LocalOnlyHotspotReservation) {
            if (attempt != generation) {
                // This cancelled attempt can never adopt a successor's pending Invoke.
                started.close(); return
            }
            android.util.Log.i("KiwixHotspot", "onStarted pending=${pending != null} stopRequested=$stopRequested")
            val invoke = pending
            pending = null
            if (stopRequested || invoke == null) {
                stopRequested = false
                closing = true
                started.close()
                onActive(false)
                invoke?.reject("HOTSPOT_START_CANCELLED", "The Wi-Fi hotspot was stopped before it finished starting")
                return
            }
            reservation = started
            val info = describe(started)
            if (info == null) {
                reservation = null
                closing = true
                started.close()
                onActive(false)
                invoke.reject("HOTSPOT_CONFIGURATION_UNAVAILABLE",
                    "Android started a hotspot but did not publish its network name and password")
                return
            }
            onActive(true)
            invoke.resolve(info)
        }

        override fun onStopped() {
            if (attempt != generation) return
            android.util.Log.i("KiwixHotspot", "onStopped closing=$closing")
            val expected = closing
            closing = false
            reservation = null
            quicInterface = null; quicLease = null
            val invoke = pending
            pending = null
            onActive(false)
            invoke?.reject("HOTSPOT_START_CANCELLED", "Android stopped the hotspot before it started")
            // A platform stop without an application Stop must not leave the
            // UI advertising credentials for a network that is gone.
            if (!expected && invoke == null) lost()
        }

        override fun onFailed(reason: Int) {
            if (attempt != generation) return
            android.util.Log.w("KiwixHotspot", "onFailed reason=$reason")
            val invoke = pending
            pending = null
            stopRequested = false
            onActive(false)
            invoke?.reject(failureCode(reason), failureMessage(reason))
        }
    }

    private fun failureCode(reason: Int): String = when (reason) {
        WifiManager.LocalOnlyHotspotCallback.ERROR_NO_CHANNEL -> "HOTSPOT_UNSUPPORTED"
        WifiManager.LocalOnlyHotspotCallback.ERROR_INCOMPATIBLE_MODE -> "HOTSPOT_INCOMPATIBLE_MODE"
        WifiManager.LocalOnlyHotspotCallback.ERROR_TETHERING_DISALLOWED -> "HOTSPOT_TETHERING_DISALLOWED"
        else -> "HOTSPOT_UNAVAILABLE"
    }

    private fun failureMessage(reason: Int): String = when (reason) {
        WifiManager.LocalOnlyHotspotCallback.ERROR_NO_CHANNEL ->
            "This device cannot create a Wi-Fi hotspot"
        WifiManager.LocalOnlyHotspotCallback.ERROR_INCOMPATIBLE_MODE ->
            "Wi-Fi is busy with another mode. Turn off Wi-Fi Direct or tethering and try again."
        WifiManager.LocalOnlyHotspotCallback.ERROR_TETHERING_DISALLOWED ->
            "Hotspot use is blocked by device policy"
        else -> "Android could not start the Wi-Fi hotspot"
    }

    private fun describe(reservation: WifiManager.LocalOnlyHotspotReservation): JSObject? {
        // Both the UI and BLE metadata use these reservation-owned credentials.
        // Retain Android 8/9's legacy API, with the modern configuration fallback.
        @Suppress("DEPRECATION")
        val legacy = reservation.wifiConfiguration
        @Suppress("DEPRECATION")
        val legacySsid = unquote(legacy?.SSID)?.takeIf { it.isNotEmpty() }
        @Suppress("DEPRECATION")
        val legacyPassphrase = unquote(legacy?.preSharedKey)?.takeIf { it.isNotEmpty() }
        if (legacySsid != null && legacyPassphrase != null) {
            return JSObject().put("ssid", legacySsid).put("passphrase", legacyPassphrase)
        }
        if (android.os.Build.VERSION.SDK_INT >= 30) {
            val configuration = reservation.softApConfiguration
            val ssid = configuration.ssid?.takeIf { it.isNotEmpty() } ?: return null
            val passphrase = configuration.passphrase?.takeIf { it.isNotEmpty() } ?: return null
            return JSObject().put("ssid", ssid).put("passphrase", passphrase)
        }
        return null
    }

    /**
     * WifiConfiguration stores SSIDs as quoted, escaped strings. The QR payload
     * and manual entry need the value the phone's network list shows.
     */
    private fun unquote(value: String?): String? {
        if (value == null || value.length < 2 || !value.startsWith('"') || !value.endsWith('"')) return value
        val body = value.substring(1, value.length - 1)
        val result = StringBuilder(body.length)
        var index = 0
        while (index < body.length) {
            val character = body[index]
            if (character == '\\' && index + 1 < body.length) {
                result.append(body[index + 1])
                index += 2
            } else {
                result.append(character)
                index += 1
            }
        }
        return result.toString()
    }
}
