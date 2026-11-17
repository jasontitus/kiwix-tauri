package org.kiwix.mobile

import android.content.Context
import android.net.ConnectivityManager
import android.net.IpPrefix
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.RouteInfo
import android.os.Build
import android.os.Handler
import android.os.ParcelFileDescriptor
import java.net.Inet4Address
import java.net.InetAddress
import java.util.concurrent.atomic.AtomicLong
import app.tauri.plugin.JSObject

/** Shared strict endpoint checks. Never performs DNS, process binding, or default-route fallback. */
internal object NearbyRouteChecks {
    fun address(raw: String): Inet4Address {
        nearbyRequire(raw.length in 7..15, "NEARBY_INVALID_ENDPOINT")
        val bytes = ByteArray(4)
        var part = 0
        var digits = 0
        var value = 0
        for (index in 0..raw.length) {
            val character = if (index == raw.length) '.' else raw[index]
            if (character == '.') {
                nearbyRequire(digits in 1..3 && part < 4, "NEARBY_INVALID_ENDPOINT")
                bytes[part++] = value.toByte()
                digits = 0; value = 0
            } else {
                nearbyRequire(character in '0'..'9' && digits < 3 && !(digits == 1 && value == 0), "NEARBY_INVALID_ENDPOINT")
                value = value * 10 + (character - '0')
                digits++
                nearbyRequire(value <= 255, "NEARBY_INVALID_ENDPOINT")
            }
        }
        nearbyRequire(part == 4, "NEARBY_INVALID_ENDPOINT")
        val a = bytes[0].toInt() and 255
        val b = bytes[1].toInt() and 255
        val c = bytes[2].toInt() and 255
        nearbyRequire(a == 10 || a == 172 && b in 16..31 || a == 192 && b == 168 ||
            a == 169 && b == 254 && c in 1..254, "NEARBY_INVALID_ENDPOINT")
        return InetAddress.getByAddress(bytes) as Inet4Address
    }
    fun physical(caps: NetworkCapabilities?): Boolean = caps != null &&
        !caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN) &&
        (caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) || caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)) &&
        (Build.VERSION.SDK_INT < 28 || caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_SUSPENDED))

    fun usable(properties: LinkProperties?): Boolean = properties != null && !properties.interfaceName.isNullOrEmpty() &&
        properties.linkAddresses.any { it.address is Inet4Address && !it.address.isAnyLocalAddress &&
            !it.address.isLoopbackAddress && it.prefixLength in 1..32 }

    fun onLink(properties: LinkProperties?, peer: Inet4Address): Boolean {
        if (!usable(properties)) return false
        properties!!
        if (properties.linkAddresses.any { it.address == peer }) return false
        val attached = properties.linkAddresses.any { link ->
            if (link.address !is Inet4Address || link.prefixLength !in 1..32 || !IpPrefix(link.address, link.prefixLength).contains(peer)) false
            else {
                val hostBits = 32 - link.prefixLength
                val number = peer.address.fold(0L) { result, byte -> (result shl 8) or (byte.toLong() and 255L) }
                val mask = if (hostBits == 0) 0L else (1L shl hostBits) - 1L
                hostBits <= 1 || (number and mask) != 0L && (number and mask) != mask
            }
        }
        val routed = properties.routes.any { route ->
            (Build.VERSION.SDK_INT < 29 || route.type == RouteInfo.RTN_UNICAST) &&
                route.destination.prefixLength > 0 && route.getInterface() == properties.interfaceName &&
                (route.gateway == null || route.gateway!!.isAnyLocalAddress) && route.destination.contains(peer)
        }
        // Attached-prefix check also excludes own/network/broadcast addresses.
        return attached && routed
    }
    fun bind(network: Network, fd: Int) {
        nearbyRequire(fd >= 0, "NEARBY_INVALID_SOCKET")
        ParcelFileDescriptor.fromFd(fd).use { network.bindSocket(it.fileDescriptor) }
    }
}

/** Bounded opaque route IDs fence Network/netId reuse. All state is main-handler owned. */
internal class NearbyRoutes(context: Context, private val main: Handler) {
    private val connectivity = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    private class Entry(val network: Network) {
        var capabilities: NetworkCapabilities? = null
        var properties: LinkProperties? = null
        var blocked = Build.VERSION.SDK_INT >= 29
    }
    private val networks = LinkedHashMap<Network, Entry>()
    private val routes = LinkedHashMap<Long, Network>()
    private var installed = false
    private val ids = AtomicLong(1L)
    private fun update(action: () -> Unit) {
        if (android.os.Looper.myLooper() == main.looper) action() else main.post { action() }
    }
    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) { update {
            if (networks.size < 16 && network !in networks) networks[network] = Entry(network)
        } }
        override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) { update {
            networks[network]?.capabilities = caps
        } }
        override fun onLinkPropertiesChanged(network: Network, properties: LinkProperties) { update {
            networks[network]?.properties = properties
        } }
        override fun onBlockedStatusChanged(network: Network, blocked: Boolean) { update { networks[network]?.blocked = blocked } }
        override fun onLost(network: Network) { update {
            networks.remove(network)
            routes.entries.removeAll { it.value == network }
        } }
    }
    fun install() {
        if (installed) return
        val request = NetworkRequest.Builder().clearCapabilities()
            .addTransportType(NetworkCapabilities.TRANSPORT_WIFI).addTransportType(NetworkCapabilities.TRANSPORT_ETHERNET).build()
        if (Build.VERSION.SDK_INT >= 26) connectivity.registerNetworkCallback(request, callback, main)
        else connectivity.registerNetworkCallback(request, callback)
        installed = true
    }
    /** Ordered-callback state only; no ConnectivityManager binder call. */
    private fun cached(network: Network, peer: Inet4Address): Boolean {
        val entry = networks[network] ?: return false
        return !entry.blocked && NearbyRouteChecks.physical(entry.capabilities) &&
            NearbyRouteChecks.onLink(entry.properties, peer)
    }
    private fun live(network: Network, peer: Inet4Address): Boolean =
        NearbyRouteChecks.physical(connectivity.getNetworkCapabilities(network)) &&
            NearbyRouteChecks.onLink(connectivity.getLinkProperties(network), peer)
    private fun valid(network: Network, peer: Inet4Address): Boolean =
        cached(network, peer) && live(network, peer)
    fun bind(args: org.json.JSONObject): JSObject {
        install()
        val peer = NearbyRouteChecks.address(args.getString("address"))
        val requested = if (args.has("route") && !args.isNull("route")) args.getLong("route") else null
        val network = if (requested != null) routes[requested]?.takeIf { valid(it, peer) }
            // Cheap cached checks first; only a candidate that could still win
            // pays for the two live ConnectivityManager calls.
            else networks.keys.firstOrNull { cached(it, peer) && live(it, peer) }
        nearbyRequire(network != null, "NEARBY_ROUTE_UNAVAILABLE")
        val selected = network!!
        val route = requested ?: routes.entries.firstOrNull { it.value == selected }?.key ?: run {
            nearbyRequire(routes.size < 16, "NEARBY_ROUTE_LIMIT")
            ids.getAndIncrement().also { routes[it] = selected }
        }
        NearbyRouteChecks.bind(selected, args.getInt("fd"))
        nearbyRequire(valid(selected, peer) && routes[route] == selected, "NEARBY_ROUTE_LOST")
        return JSObject().put("route", route)
    }
    fun state(args: org.json.JSONObject): JSObject {
        val peer = NearbyRouteChecks.address(args.getString("address"))
        val network = routes[args.getLong("route")]
        return JSObject().put("available", network != null && valid(network, peer))
    }
}
