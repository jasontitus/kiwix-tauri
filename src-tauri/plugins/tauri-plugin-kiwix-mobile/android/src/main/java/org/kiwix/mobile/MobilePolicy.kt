package org.kiwix.mobile

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Build
import android.os.ParcelFileDescriptor
import android.system.Os
import android.system.OsConstants
import app.tauri.plugin.JSObject

/** Native capabilities, including unvalidated Wi-Fi used only for local-LAN peers. */
internal class MobileNetworkPolicy(context: Context, private val changed: (JSObject) -> Unit) {
    private val connectivity = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    private val lock = Any()
    private var generation = 0L
    // No network produces no onAvailable callback. Treat that observed absence
    // as offline, not an indefinitely unknown state. Connected paths still wait
    // for ordered callbacks; this never grants Internet or local admission.
    private var known = connectivity.activeNetwork == null
    private var defaultNetwork: Network? = null
    private var defaultCapabilities: NetworkCapabilities? = null
    private val localCapabilities = mutableMapOf<Network, NetworkCapabilities>()
    private val blockedNetworks = mutableSetOf<Network>()
    /** The app itself is the AP; the hotspot link is a usable local network. */
    private var hotspotActive = false
    /** AP interface prefixes, refreshed only by lifecycle/callback state changes. */
    private var hotspotPrefixCache: List<Pair<java.net.Inet4Address, Int>>? = null
    /** Last published derived state, so unchanged callbacks emit nothing. */
    private var lastState: PolicyState? = null
    private data class PolicyState(val known: Boolean, val connected: Boolean, val metered: Boolean,
        val cellular: Boolean, val localAvailable: Boolean)
    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) = update {
            known = true
            defaultNetwork = network
            // Capabilities arrive in the ordered callback that follows availability.
            defaultCapabilities = null
        }
        override fun onLost(network: Network) = update {
            known = true
            if (defaultNetwork == network) {
                defaultNetwork = null
                defaultCapabilities = null
            }
            if (!localCapabilities.containsKey(network)) blockedNetworks.remove(network)
        }
        override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) = update {
            if (defaultNetwork == network) {
                known = true
                defaultCapabilities = capabilities
            }
        }
        override fun onBlockedStatusChanged(network: Network, blocked: Boolean) = update {
            if (blocked) blockedNetworks.add(network) else blockedNetworks.remove(network)
        }
    }
    private val localCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) = update {
            known = true
            hotspotPrefixCache = null
        }
        override fun onLost(network: Network) = update {
            localCapabilities.remove(network)
            if (defaultNetwork != network) blockedNetworks.remove(network)
            hotspotPrefixCache = null
        }
        override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) = update {
            known = true
            localCapabilities[network] = capabilities
            // An AP interface can appear or change with the local network state.
            hotspotPrefixCache = null
        }
        override fun onBlockedStatusChanged(network: Network, blocked: Boolean) = update {
            if (blocked) blockedNetworks.add(network) else blockedNetworks.remove(network)
        }
    }
    init {
        // Callback-owned state preserves every ordered metered/blocked transition.
        // Synchronous capability queries inside callbacks can erase a rapid denial.
        connectivity.registerDefaultNetworkCallback(callback)
        connectivity.registerNetworkCallback(NetworkRequest.Builder().clearCapabilities()
            .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
            .addTransportType(NetworkCapabilities.TRANSPORT_ETHERNET).build(), localCallback)
    }
    private fun usable(caps: NetworkCapabilities) = Build.VERSION.SDK_INT < 28 ||
        caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_SUSPENDED)
    private fun physicalLan(caps: NetworkCapabilities) =
        !caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN) &&
            (caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
                caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)) && usable(caps)
    private fun stateLocked(): PolicyState {
        val caps = defaultCapabilities
        val cellular = caps?.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) == true
        val connected = caps != null && usable(caps) &&
            caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) &&
            caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED) &&
            !blockedNetworks.contains(defaultNetwork)
        val metered = caps == null || cellular ||
            !caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
        val localAvailable = hotspotActive || localCapabilities.any { (network, local) ->
            physicalLan(local) && !blockedNetworks.contains(network)
        }
        return PolicyState(known, connected, metered, cellular, localAvailable)
    }
    private fun snapshotLocked(): JSObject {
        val state = stateLocked()
        return JSObject().put("known", state.known).put("connected", state.connected)
            .put("metered", state.metered).put("cellular", state.cellular)
            .put("localAvailable", state.localAvailable).put("generation", generation)
    }
    fun snapshot(): JSObject = synchronized(lock) { snapshotLocked() }

    /** Publish the hotspot as a local network; the AP interface is not a client network. */
    fun setHotspotActive(active: Boolean): Unit = update {
        hotspotActive = active
        hotspotPrefixCache = null
    }
    private fun update(change: () -> Unit): Unit = synchronized(lock) {
        change()
        val state = stateLocked()
        // Ordered callbacks repeat the same derived state; only a real transition
        // bumps the generation or re-emits the snapshot.
        if (state == lastState) return@synchronized
        lastState = state
        generation += 1
        // Publish under the same lock so concurrent callback types cannot reorder
        // generation-tagged transitions after taking their snapshots.
        changed(snapshotLocked())
    }

    fun bindLocalSocket(fd: Int): Long = synchronized(lock) {
        if (!known || fd < 0) throw NativeDocumentError("LOCAL_NETWORK_REQUIRED")
        val network = localCapabilities.entries.firstOrNull { (candidate, caps) ->
            physicalLan(caps) && !blockedNetworks.contains(candidate)
        }?.key ?: throw NativeDocumentError("LOCAL_NETWORK_REQUIRED")
        // Live queries are only used to revalidate a binding operation, never to
        // replace the capability argument of an ordered policy callback.
        val before = connectivity.getNetworkCapabilities(network)
        if (before == null || !physicalLan(before)) throw NativeDocumentError("LOCAL_NETWORK_CHANGED")
        // fromFd duplicates only for this trusted call. Binding sets the underlying socket
        // network mark, so the original Rust-owned descriptor remains route-confined.
        ParcelFileDescriptor.fromFd(fd).use { duplicate -> network.bindSocket(duplicate.fileDescriptor) }
        val current = connectivity.getNetworkCapabilities(network)
        if (current == null || !physicalLan(current)) {
            throw NativeDocumentError("LOCAL_NETWORK_CHANGED")
        }
        network.networkHandle
    }

    private fun accessPointName(name: String): Boolean =
        name == "wlan1" || name.startsWith("ap") || name.startsWith("swlan") || name.startsWith("softap")
    /** Prefix containment without hidden IpPrefix constructors (Android 9 safe). */
    private fun inPrefix(address: java.net.InetAddress, peer: java.net.InetAddress, prefix: Int): Boolean {
        val bytes = address.address
        val other = peer.address
        if (bytes.size != other.size || prefix !in 0..bytes.size * 8) return false
        var bits = prefix
        var index = 0
        while (bits >= 8) {
            if (bytes[index] != other[index]) return false
            index++
            bits -= 8
        }
        if (bits > 0) {
            val mask = (0xFF shl (8 - bits)) and 0xFF
            if ((bytes[index].toInt() and mask) != (other[index].toInt() and mask)) return false
        }
        return true
    }
    private fun hotspotPrefixes(): List<Pair<java.net.Inet4Address, Int>> =
        java.net.NetworkInterface.getNetworkInterfaces()?.toList().orEmpty()
            .filter { it.isUp && !it.isLoopback && accessPointName(it.name) }
            .flatMap { iface -> iface.interfaceAddresses.mapNotNull { link ->
                val address = link.address as? java.net.Inet4Address ?: return@mapNotNull null
                val prefix = link.networkPrefixLength.toInt()
                if (prefix in 1..32) address to prefix else null
            } }
    fun isLocalPeer(route: Long, address: String): Boolean = synchronized(lock) {
        if (!known || route <= 0) throw NativeDocumentError("LOCAL_NETWORK_REQUIRED")
        val family = if (address.contains(':')) OsConstants.AF_INET6 else OsConstants.AF_INET
        val numeric = Os.inet_pton(family, address) ?: throw NativeDocumentError("INVALID_NUMERIC_PEER")
        // The app-owned AP is not a ConnectivityManager client network. Its
        // clients are on-link peers of the AP interface, not of the default route.
        if (hotspotActive) {
            val prefixes = hotspotPrefixCache ?: hotspotPrefixes().also { hotspotPrefixCache = it }
            if (prefixes.any { (address, prefix) -> inPrefix(address, numeric, prefix) }) return true
        }
        val network = Network.fromNetworkHandle(route)
        val observed = localCapabilities[network]
        val current = connectivity.getNetworkCapabilities(network)
        if (observed == null || !physicalLan(observed) || current == null || !physicalLan(current) ||
            blockedNetworks.contains(network)) throw NativeDocumentError("LOCAL_NETWORK_CHANGED")
        val peer = numeric
        val properties = connectivity.getLinkProperties(network)
            ?: throw NativeDocumentError("LOCAL_NETWORK_CHANGED")
        val attached = properties.linkAddresses.any { link ->
            link.prefixLength > 0 && inPrefix(link.address, peer, link.prefixLength)
        }
        val routed = properties.routes.any { routeInfo ->
            val gateway = routeInfo.gateway
            Build.VERSION.SDK_INT >= 29 && routeInfo.type == android.net.RouteInfo.RTN_UNICAST &&
                routeInfo.destination.prefixLength > 0 &&
                routeInfo.getInterface() == properties.interfaceName &&
                (gateway == null || gateway.isAnyLocalAddress) &&
                routeInfo.destination.contains(peer)
        }
        val after = connectivity.getNetworkCapabilities(network)
        if (after == null || !physicalLan(after)) throw NativeDocumentError("LOCAL_NETWORK_CHANGED")
        attached || routed
    }
}
