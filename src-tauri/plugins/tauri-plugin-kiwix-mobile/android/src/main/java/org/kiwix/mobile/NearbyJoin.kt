package org.kiwix.mobile

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.wifi.WifiInfo
import android.net.wifi.WifiNetworkSpecifier
import android.os.Build
import android.os.Handler
import androidx.core.content.ContextCompat
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import java.util.UUID

/** Application-owned, one request/one exact Network. Contains no Activity or strong plugin reference. */
internal class NearbyJoin(private val context: Context, private val main: Handler, private val emit: (JSObject) -> Unit) {
    private val connectivity = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    internal class Attempt(val owner: String, val lease: String, val ssid: String,
        val password: ByteArray, val reply: NearbyReply, val foreground: () -> Boolean) {
        var callback: ConnectivityManager.NetworkCallback? = null
        var network: Network? = null
        var capabilities: NetworkCapabilities? = null
        var properties: LinkProperties? = null
        var blocked = true
        var associated = false
        var retired = false
        var lossEmitted = false
        var timeout: Runnable? = null
        var watch: Runnable? = null
        val endpoints = LinkedHashSet<java.net.Inet4Address>()
    }
    private var current: Attempt? = null
    fun busy(): Boolean = current != null
    fun owns(attempt: Attempt): Boolean = current === attempt && !attempt.retired

    fun reserve(invoke: Invoke, foreground: () -> Boolean): Attempt {
        nearbyRequire(Build.VERSION.SDK_INT >= 29, "NEARBY_JOIN_UNSUPPORTED")
        nearbyRequire(current == null, "NEARBY_JOIN_BUSY")
        val args = invoke.getArgs()
        val owner = nearbyUuid(args.getString("owner"))
        nearbyUuid(args.getString("reservationId"))
        val ssid = args.getString("ssid")
        val password = args.getString("passphrase")
        nearbyRequire(args.getString("security") == "wpa2" && ssid.isNotEmpty() && Charsets.UTF_8.newEncoder().canEncode(ssid) &&
            ssid.toByteArray(Charsets.UTF_8).size <= 32 && ssid.none { it.isISOControl() } &&
            password.length in 8..63 && password.all { it.code in 32..126 }, "NEARBY_INVALID_ARGUMENT")
        val attempt = Attempt(owner, UUID.randomUUID().toString(), ssid, password.toByteArray(Charsets.US_ASCII), NearbyReply(invoke), foreground)
        current = attempt
        attempt.timeout = Runnable { if (owns(attempt)) retire(attempt, "NEARBY_JOIN_TIMEOUT", true) }.also { main.postDelayed(it, 60_000L) }
        return attempt
    }

    fun permissions(): Array<String> = if (Build.VERSION.SDK_INT >= 33)
        arrayOf(Manifest.permission.NEARBY_WIFI_DEVICES, Manifest.permission.ACCESS_FINE_LOCATION)
        else arrayOf(Manifest.permission.ACCESS_FINE_LOCATION)
    private fun permitted(): Boolean = permissions().all { ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED }

    @SuppressLint("MissingPermission")
    fun begin(attempt: Attempt) {
        if (!owns(attempt)) return
        try {
            nearbyRequire(attempt.foreground(), "NEARBY_ACTIVITY_RETIRED")
            nearbyRequire(permitted(), "NEARBY_PERMISSION_DENIED")
            nearbyRequire(Build.VERSION.SDK_INT >= 29, "NEARBY_JOIN_UNSUPPORTED")
            val location = context.getSystemService(Context.LOCATION_SERVICE) as android.location.LocationManager
            nearbyRequire(location.isLocationEnabled, "NEARBY_LOCATION_SERVICES_REQUIRED")
            val specifier = WifiNetworkSpecifier.Builder().setSsid(attempt.ssid)
                .setWpa2Passphrase(attempt.password.toString(Charsets.US_ASCII)).build()
            attempt.password.fill(0)
            val request = NetworkRequest.Builder().addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
                .removeCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET).setNetworkSpecifier(specifier).build()
            // SSID proof is location-sensitive on Android even for an exact specifier.
            // Its dedicated explicit permission is never borrowed from a reader/archive bridge.
            val flags = if (Build.VERSION.SDK_INT >= 31) ConnectivityManager.NetworkCallback.FLAG_INCLUDE_LOCATION_INFO else 0
            val callback = if (Build.VERSION.SDK_INT >= 31) JoinCallback(attempt, flags) else JoinCallback(attempt)
            attempt.callback = callback
            attempt.timeout?.let { main.removeCallbacks(it) }
            attempt.timeout = Runnable { if (owns(attempt)) retire(attempt, "NEARBY_JOIN_TIMEOUT", true) }.also { main.postDelayed(it, 120_000L) }
            connectivity.requestNetwork(request, callback, main, 120_000)
            watch(attempt)
        } catch (error: Exception) { retire(attempt, nearbyFailure(error), true) }
    }

    private inner class JoinCallback : ConnectivityManager.NetworkCallback {
        private val attempt: Attempt
        constructor(attempt: Attempt) : super() { this.attempt = attempt }
        constructor(attempt: Attempt, flags: Int) : super(flags) { this.attempt = attempt }
        private fun live(): Boolean = owns(attempt) && attempt.callback === this
        override fun onAvailable(network: Network) {
            if (!live()) return
            if (attempt.network != null && attempt.network != network) { retire(attempt, "NEARBY_JOIN_ROUTE_CHANGED", true); return }
            attempt.network = network
            evaluate(attempt)
        }
        override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) {
            if (!live() || attempt.network != network) return
            attempt.capabilities = capabilities
            evaluate(attempt)
        }
        override fun onLinkPropertiesChanged(network: Network, properties: LinkProperties) {
            if (!live() || attempt.network != network) return
            attempt.properties = properties
            evaluate(attempt)
        }
        override fun onBlockedStatusChanged(network: Network, blocked: Boolean) {
            if (!live() || attempt.network != network) return
            attempt.blocked = blocked
            evaluate(attempt)
        }
        override fun onLost(network: Network) {
            if (live() && attempt.network == network) retire(attempt, "NEARBY_JOIN_LOST", true)
        }
        override fun onUnavailable() { if (live()) retire(attempt, "NEARBY_JOIN_UNAVAILABLE", true) }
    }

    private fun association(attempt: Attempt): Boolean {
        val caps = attempt.capabilities ?: return false
        val info = caps.transportInfo as? WifiInfo ?: return false
        val ssid = info.ssid
        return ssid == "\"${attempt.ssid}\"" &&
            caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) && NearbyRouteChecks.physical(caps) &&
            !attempt.blocked && NearbyRouteChecks.usable(attempt.properties)
    }
    private fun valid(attempt: Attempt): Boolean {
        val network = attempt.network ?: return false
        val location = context.getSystemService(Context.LOCATION_SERVICE) as android.location.LocationManager
        val properties = connectivity.getLinkProperties(network)
        return owns(attempt) && attempt.foreground() && permitted() && location.isLocationEnabled && association(attempt) &&
            NearbyRouteChecks.physical(connectivity.getNetworkCapabilities(network)) &&
            NearbyRouteChecks.usable(properties) && attempt.endpoints.all {
                NearbyRouteChecks.onLink(attempt.properties, it) && NearbyRouteChecks.onLink(properties, it)
            }
    }
    private fun evaluate(attempt: Attempt) {
        if (!owns(attempt)) return
        if (!attempt.foreground() || !permitted()) { retire(attempt, "NEARBY_JOIN_RETIRED", true); return }
        if (attempt.associated && !valid(attempt)) { retire(attempt, "NEARBY_JOIN_LOST", true); return }
        if (!attempt.associated && valid(attempt)) {
            attempt.associated = true
            attempt.timeout?.let { main.removeCallbacks(it) }; attempt.timeout = null
            // Association only. Rust must validate the retained route and pinned TLS before Connected.
            attempt.reply.resolve(JSObject().put("owner", attempt.owner).put("lease", attempt.lease))
        }
    }
    private fun watch(attempt: Attempt) {
        val task = object : Runnable {
            override fun run() {
                if (!owns(attempt)) return
                try { evaluate(attempt) } catch (_: Exception) { retire(attempt, "NEARBY_JOIN_LOST", true) }
                // While waiting for association this poll is the only retirement
                // mechanism. Once associated, callbacks cover network loss but
                // not permission/location changes, so keep a low-frequency
                // safety net for those.
                if (owns(attempt)) main.postDelayed(this, if (attempt.associated) 5_000L else 1_000L)
            }
        }
        attempt.watch = task
        main.postDelayed(task, 1_000L)
    }
    fun fail(attempt: Attempt, code: String) { if (owns(attempt)) retire(attempt, code, true) }
    private fun requireLease(args: org.json.JSONObject): Attempt {
        val owner = nearbyUuid(args.getString("owner"))
        val lease = nearbyUuid(args.getString("lease"))
        val attempt = current
        nearbyRequire(attempt != null && attempt.owner == owner && attempt.lease == lease && !attempt.retired, "NEARBY_JOIN_STALE")
        return attempt!!
    }
    fun state(args: org.json.JSONObject): JSObject {
        val attempt = current
        if (attempt == null || attempt.owner != args.getString("owner") || attempt.lease != args.getString("lease")) return JSObject().put("available", false)
        val available = attempt.associated && valid(attempt)
        if (attempt.associated && !available) retire(attempt, "NEARBY_JOIN_LOST", true)
        return JSObject().put("available", available)
    }
    fun bind(args: org.json.JSONObject): JSObject {
        val attempt = requireLease(args)
        nearbyRequire(attempt.associated && valid(attempt), "NEARBY_JOIN_LOST")
        val peer = NearbyRouteChecks.address(args.getString("address"))
        val network = attempt.network!!
        nearbyRequire(NearbyRouteChecks.onLink(attempt.properties, peer) &&
            NearbyRouteChecks.onLink(connectivity.getLinkProperties(network), peer), "NEARBY_ENDPOINT_NOT_ON_LINK")
        nearbyRequire(peer in attempt.endpoints || attempt.endpoints.size < 8, "NEARBY_ENDPOINT_LIMIT")
        NearbyRouteChecks.bind(network, args.getInt("fd"))
        nearbyRequire(valid(attempt) && NearbyRouteChecks.onLink(connectivity.getLinkProperties(network), peer), "NEARBY_JOIN_LOST")
        attempt.endpoints.add(peer)
        return JSObject().put("route", network.networkHandle)
    }
    fun release(args: org.json.JSONObject): JSObject {
        val owner = nearbyUuid(args.getString("owner"))
        val lease = nearbyUuid(args.getString("lease"))
        current?.takeIf { it.owner == owner && it.lease == lease }?.let {
            nearbyRequire(retire(it, "NEARBY_JOIN_RELEASED", true), "NEARBY_JOIN_CLEANUP_FAILED")
        }
        return JSObject()
    }
    fun cancel(args: org.json.JSONObject): JSObject {
        val owner = nearbyUuid(args.getString("owner"))
        current?.takeIf { it.owner == owner }?.let {
            nearbyRequire(retire(it, "NEARBY_JOIN_CANCELLED", true), "NEARBY_JOIN_CLEANUP_FAILED")
        }
        return JSObject()
    }
    fun retireAll() {
        current?.let { nearbyRequire(retire(it, "NEARBY_JOIN_RETIRED", true), "NEARBY_JOIN_CLEANUP_FAILED") }
    }
    private fun retire(attempt: Attempt, code: String, notify: Boolean): Boolean {
        if (current !== attempt) return true
        // Close admission before unregister; a cleanup failure retains this tombstoned owner for retry.
        attempt.retired = true
        attempt.timeout?.let { main.removeCallbacks(it) }
        attempt.watch?.let { main.removeCallbacks(it) }
        attempt.timeout = null; attempt.watch = null
        attempt.password.fill(0)
        attempt.reply.reject(code)
        val callback = attempt.callback
        attempt.callback = null
        try {
            if (callback != null) connectivity.unregisterNetworkCallback(callback)
            current = null
        } catch (_: IllegalArgumentException) { current = null } // Android: callback already unregistered.
        catch (_: Exception) {
            // Keep the retiring owner: a teardown failure must not admit a conflicting successor.
            attempt.callback = callback
            if (notify && !attempt.lossEmitted) {
                attempt.lossEmitted = true
                runCatching { emit(JSObject().put("owner", attempt.owner).put("lease", attempt.lease).put("kind", "lost").put("error", "NEARBY_JOIN_CLEANUP_FAILED")) }
            }
            return false
        }
        attempt.network = null
        attempt.capabilities = null
        attempt.properties = null
        if (notify && !attempt.lossEmitted) {
            attempt.lossEmitted = true
            runCatching { emit(JSObject().put("owner", attempt.owner).put("lease", attempt.lease).put("kind", "lost").put("error", code)) }
        }
        return true
    }
}
