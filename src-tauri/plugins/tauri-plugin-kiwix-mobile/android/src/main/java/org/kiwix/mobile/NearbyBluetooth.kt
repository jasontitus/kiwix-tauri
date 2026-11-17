package org.kiwix.mobile

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothGattServer
import android.bluetooth.BluetoothGattServerCallback
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.BluetoothStatusCodes
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.BluetoothLeAdvertiser
import android.bluetooth.le.BluetoothLeScanner
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.os.Build
import android.os.Handler
import android.os.ParcelUuid
import android.util.Base64
import android.util.Log
import androidx.core.content.ContextCompat
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import java.util.UUID
import java.util.concurrent.atomic.AtomicInteger

/** Public framed byte transport only. Main-owned, selected-Connect authority stays in Rust. */
@SuppressLint("MissingPermission")
internal class NearbyBluetooth(private val context: Context, private val main: Handler, private val emit: (JSObject) -> Boolean) {
    companion object {
        private const val TAG = "KiwixNearby"
        private val SERVICE = UUID.fromString("7136d55f-2a4c-4df0-a75d-4d87f359d701")
        private val CHANNEL = UUID.fromString("7136d55f-2a4c-4df0-a75d-4d87f359d702")
        private val INFO = UUID.fromString("7136d55f-2a4c-4df0-a75d-4d87f359d703")
        private val CCCD = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")
        private const val CHANNEL_PROPERTIES = BluetoothGattCharacteristic.PROPERTY_WRITE or BluetoothGattCharacteristic.PROPERTY_INDICATE
        // CoreBluetooth adds Extended Properties to the write/indicate channel.
        // It does not replace either required operation or permit another mode.
        internal fun channelPropertiesValid(properties: Int): Boolean =
            (properties and BluetoothGattCharacteristic.PROPERTY_EXTENDED_PROPS.inv()) == CHANNEL_PROPERTIES
    }
    internal class Owner(val id: String, val role: String, val info: ByteArray?, val name: String?, val start: NearbyReply, val foreground: () -> Boolean) {
        @Volatile var retired = false
        var started = false
        var sequence = 0L
        var scanner: BluetoothLeScanner? = null
        var scan: ScanCallback? = null
        var advertiser: BluetoothLeAdvertiser? = null
        var advertisement: AdvertiseCallback? = null
        var server: BluetoothGattServer? = null
        var service: BluetoothGattService? = null
        var serviceAdding = false
        var channel: BluetoothGattCharacteristic? = null
        var publicInfo: BluetoothGattCharacteristic? = null
        var cccd: BluetoothGattDescriptor? = null
        val peers = LinkedHashMap<String, BluetoothDevice>()
        val connections = LinkedHashMap<String, Connection>()
        var serverEpoch = 0L
        var scanning = true
        val closingGatts = ArrayList<BluetoothGatt>(1)
        val overflow = java.util.concurrent.atomic.AtomicBoolean(false)
        var deadline: Runnable? = null
        var receiver: BroadcastReceiver? = null
        /** Advertisement addresses already dispatched; read from the scan thread. */
        val scannedAddresses: MutableSet<String> = java.util.concurrent.ConcurrentHashMap.newKeySet()
    }
    internal class Connection(val id: String, val device: BluetoothDevice, val peer: String?, var connect: NearbyReply?) {
        var gatt: BluetoothGatt? = null
        var channel: BluetoothGattCharacteristic? = null
        var info: BluetoothGattCharacteristic? = null
        var cccd: BluetoothGattDescriptor? = null
        var budget = 20
        var ready = false
        var stage = "connecting"
        var publicInfo: ByteArray? = null
        var send: NearbyReply? = null
        var sending: ByteArray? = null
        var deadline: Runnable? = null
        var sendDeadline: Runnable? = null
    }
    private val owners = LinkedHashMap<String, Owner>() // at most central + peripheral
    private val queued = AtomicInteger(0)
    private val manager = context.getSystemService(Context.BLUETOOTH_SERVICE) as BluetoothManager

    fun permissions(role: String): Array<String> = if (Build.VERSION.SDK_INT >= 31) {
        if (role == "central") arrayOf(Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_CONNECT)
        else arrayOf(Manifest.permission.BLUETOOTH_CONNECT, Manifest.permission.BLUETOOTH_ADVERTISE)
    } else if (role == "central") arrayOf(Manifest.permission.ACCESS_FINE_LOCATION) else emptyArray()
    private fun permitted(owner: Owner): Boolean = permissions(owner.role).all { ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED }
    fun owns(owner: Owner): Boolean = !owner.retired && owners[owner.role] === owner
    private fun owns(owner: Owner, connection: Connection): Boolean = owns(owner) && owner.connections[connection.id] === connection

    fun reserve(invoke: Invoke, foreground: () -> Boolean): Owner {
        val args = invoke.getArgs()
        val id = nearbyUuid(args.getString("owner"))
        val role = args.getString("role")
        nearbyRequire(role == "central" || role == "peripheral", "NEARBY_INVALID_ARGUMENT")
        nearbyRequire(owners[role] == null && owners.values.none { it.id == id }, "NEARBY_BLUETOOTH_BUSY")
        val info = if (role == "peripheral") {
            val id = UUID.fromString(args.getString("publicId"))
            nearbyRequire(id.toString() == args.getString("publicId"), "NEARBY_INVALID_ARGUMENT")
            val bytes = java.nio.ByteBuffer.allocate(18).put(2.toByte()).put(0.toByte()).putLong(id.mostSignificantBits).putLong(id.leastSignificantBits).array()
            bytes
        } else null
        val name = if (role == "peripheral") args.getString("name").also {
            nearbyRequire(it.isNotEmpty() && it.toByteArray(Charsets.UTF_8).size <= 256, "NEARBY_INVALID_ARGUMENT")
        } else null
        val owner = Owner(id, role, info, name, NearbyReply(invoke), foreground)
        owners[role] = owner // reserve BEFORE any system authorization prompt
        owner.deadline = Runnable { if (owns(owner)) fail(owner, "NEARBY_BLUETOOTH_START_TIMEOUT") }.also { main.postDelayed(it, 60_000L) }
        return owner
    }
    private fun dispatch(owner: Owner, action: () -> Unit) {
        if (owner.retired || owner.overflow.get()) return
        if (queued.incrementAndGet() > 64) {
            queued.decrementAndGet()
            if (owner.overflow.compareAndSet(false, true)) main.postAtFrontOfQueue {
                if (owns(owner)) fail(owner, "NEARBY_BLUETOOTH_OVERFLOW")
            }
            return
        }
        if (!main.post {
            queued.decrementAndGet()
            if (!owns(owner)) return@post
            try { action() } catch (error: Exception) { fail(owner, nearbyFailure(error)) }
        }) queued.decrementAndGet()
    }
    private fun event(owner: Owner, kind: String, connection: Connection? = null, extra: JSObject = JSObject()): Boolean {
        if (!owns(owner)) return false
        extra.put("owner", owner.id).put("sequence", ++owner.sequence).put("kind", kind)
        connection?.let { extra.put("connection", it.id); it.peer?.let { peer -> extra.put("peer", peer) } }
        if (!runCatching { emit(extra) }.getOrDefault(false)) { fail(owner, "NEARBY_BLUETOOTH_LISTENER_UNAVAILABLE"); return false }
        return true
    }
    fun begin(owner: Owner) {
        if (!owns(owner)) return
        if (Log.isLoggable(TAG, Log.DEBUG)) Log.i(TAG, "start role=${owner.role}")
        try {
            nearbyRequire(owner.foreground(), "NEARBY_ACTIVITY_RETIRED")
            nearbyRequire(permitted(owner), "NEARBY_PERMISSION_DENIED")
            val adapter = manager.adapter ?: throw NearbyError("NEARBY_BLUETOOTH_UNSUPPORTED")
            nearbyRequire(adapter.isEnabled, "NEARBY_BLUETOOTH_UNAVAILABLE")
            nearbyRequire(context.packageManager.hasSystemFeature(PackageManager.FEATURE_BLUETOOTH_LE), "NEARBY_BLUETOOTH_UNSUPPORTED")
            if (owner.role == "central" && Build.VERSION.SDK_INT <= 30) {
                val location = context.getSystemService(Context.LOCATION_SERVICE) as android.location.LocationManager
                val enabled = if (Build.VERSION.SDK_INT >= 28) location.isLocationEnabled
                    else location.isProviderEnabled(android.location.LocationManager.GPS_PROVIDER) || location.isProviderEnabled(android.location.LocationManager.NETWORK_PROVIDER)
                nearbyRequire(enabled, "NEARBY_LOCATION_SERVICES_REQUIRED")
            }
            owner.deadline?.let { main.removeCallbacks(it) }
            owner.deadline = Runnable { if (owns(owner) && !owner.started) fail(owner, "NEARBY_BLUETOOTH_START_TIMEOUT") }.also { main.postDelayed(it, 15_000L) }
            owner.receiver = object : BroadcastReceiver() {
                override fun onReceive(context: Context?, intent: Intent?) {
                    if (intent?.action == BluetoothAdapter.ACTION_STATE_CHANGED && intent.getIntExtra(BluetoothAdapter.EXTRA_STATE, BluetoothAdapter.ERROR) != BluetoothAdapter.STATE_ON)
                        dispatch(owner) { fail(owner, "NEARBY_BLUETOOTH_UNAVAILABLE") }
                }
            }.also { ContextCompat.registerReceiver(context, it, IntentFilter(BluetoothAdapter.ACTION_STATE_CHANGED), ContextCompat.RECEIVER_NOT_EXPORTED) }
            if (owner.role == "central") startScan(owner, adapter) else startPeripheral(owner, adapter)
        } catch (error: Exception) { fail(owner, nearbyFailure(error)) }
    }
    private fun started(owner: Owner) {
        if (!owns(owner)) return
        owner.started = true
        owner.deadline?.let { main.removeCallbacks(it) }; owner.deadline = null
        owner.start.resolve(JSObject().put("owner", owner.id).put("started", true))
    }
    private fun startScan(owner: Owner, adapter: BluetoothAdapter) {
        if (Log.isLoggable(TAG, Log.DEBUG)) Log.i(TAG, "scan start")
        val scanner = adapter.bluetoothLeScanner ?: throw NearbyError("NEARBY_BLUETOOTH_UNAVAILABLE")
        val callback = object : ScanCallback() {
            override fun onScanResult(callbackType: Int, result: ScanResult) {
                if (result.scanRecord?.serviceUuids?.contains(ParcelUuid(SERVICE)) != true) return
                // LOW_LATENCY repeats advertisements several times per second. The
                // address filter runs on the scan thread, before any main-thread
                // closure or event allocation.
                if (!owner.scannedAddresses.add(result.device.address)) return
                dispatch(owner) {
                    if (owner.scan !== this || owner.scanner !== scanner || !owner.started || !owner.scanning) {
                        // A stale scan must not keep this address filtered.
                        owner.scannedAddresses.remove(result.device.address)
                        return@dispatch
                    }
                    if (owner.peers.size >= 16 || owner.peers.values.any { it == result.device }) return@dispatch
                    val peer = UUID.randomUUID().toString()
                    owner.peers[peer] = result.device
                    val extra = JSObject().put("peer", peer)
                    val advertised = result.scanRecord?.getServiceData(ParcelUuid(SERVICE))?.takeIf { it.size <= 8 }?.let {
                        runCatching { Charsets.UTF_8.newDecoder().onMalformedInput(java.nio.charset.CodingErrorAction.REPORT).decode(java.nio.ByteBuffer.wrap(it)).toString() }.getOrNull()
                    } ?: result.scanRecord?.deviceName
                    advertised?.takeIf { it.toByteArray(Charsets.UTF_8).size <= 256 }?.let { extra.put("name", it) }
                    if (Log.isLoggable(TAG, Log.DEBUG)) Log.i(TAG, "peer name=${(advertised ?: "-").filterNot { it.isISOControl() || it == '\n' || it == '\r' }.take(64)}")
                    event(owner, "peer", extra = extra)
                }
            }
            override fun onBatchScanResults(results: MutableList<ScanResult>) {
                // This scan explicitly disables batching. Do not queue a platform-sized list.
                for (index in 0 until minOf(16, results.size))
                    onScanResult(ScanSettings.CALLBACK_TYPE_ALL_MATCHES, results[index])
            }
            override fun onScanFailed(errorCode: Int) { dispatch(owner) { if (owner.scan === this) fail(owner, "NEARBY_BLUETOOTH_SCAN_FAILED") } }
        }
        owner.scanner = scanner; owner.scan = callback
        scanner.startScan(listOf(ScanFilter.Builder().setServiceUuid(ParcelUuid(SERVICE)).build()),
            ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).setReportDelay(0).build(), callback)
        started(owner)
    }
    private fun startPeripheral(owner: Owner, adapter: BluetoothAdapter) {
        if (Log.isLoggable(TAG, Log.DEBUG)) Log.i(TAG, "peripheral start")
        nearbyRequire(adapter.isMultipleAdvertisementSupported, "NEARBY_BLUETOOTH_PERIPHERAL_UNSUPPORTED")
        val advertiser = adapter.bluetoothLeAdvertiser ?: throw NearbyError("NEARBY_BLUETOOTH_PERIPHERAL_UNSUPPORTED")
        val service = BluetoothGattService(SERVICE, BluetoothGattService.SERVICE_TYPE_PRIMARY)
        val channel = BluetoothGattCharacteristic(CHANNEL, CHANNEL_PROPERTIES, BluetoothGattCharacteristic.PERMISSION_WRITE)
        val cccd = BluetoothGattDescriptor(CCCD, BluetoothGattDescriptor.PERMISSION_READ or BluetoothGattDescriptor.PERMISSION_WRITE)
        channel.addDescriptor(cccd)
        val info = BluetoothGattCharacteristic(INFO, BluetoothGattCharacteristic.PROPERTY_READ, BluetoothGattCharacteristic.PERMISSION_READ)
        info.value = owner.info
        service.addCharacteristic(channel); service.addCharacteristic(info)
        owner.service = service; owner.channel = channel; owner.publicInfo = info; owner.cccd = cccd
        owner.advertiser = advertiser
        owner.server = manager.openGattServer(context, serverCallback(owner)) ?: throw NearbyError("NEARBY_BLUETOOTH_PERIPHERAL_UNSUPPORTED")
        owner.serviceAdding = true
        if (!owner.server!!.addService(service)) {
            owner.serviceAdding = false
            throw NearbyError("NEARBY_BLUETOOTH_SERVICE_FAILED")
        }
    }
    private fun advertise(owner: Owner) {
        val advertiser = owner.advertiser ?: return
        val callback = object : AdvertiseCallback() {
            override fun onStartSuccess(settingsInEffect: AdvertiseSettings) { dispatch(owner) { if (owner.advertisement === this) started(owner) } }
            override fun onStartFailure(errorCode: Int) { dispatch(owner) { if (owner.advertisement === this) fail(owner, "NEARBY_BLUETOOTH_ADVERTISE_FAILED") } }
        }
        owner.advertisement = callback
        advertiser.startAdvertising(AdvertiseSettings.Builder().setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_LATENCY)
            .setConnectable(true).setTimeout(0).build(), AdvertiseData.Builder().setIncludeDeviceName(false)
            .setIncludeTxPowerLevel(false).addServiceUuid(ParcelUuid(SERVICE)).build(),
            AdvertiseData.Builder().setIncludeDeviceName(false).apply {
                owner.name?.let { name ->
                    var prefix = ""
                    for (codePoint in name.codePoints().toArray()) {
                        val next = prefix + String(Character.toChars(codePoint))
                        if (next.toByteArray(Charsets.UTF_8).size > 8) break
                        prefix = next
                    }
                    if (prefix.isNotEmpty()) addServiceData(ParcelUuid(SERVICE), prefix.toByteArray(Charsets.UTF_8))
                }
            }.build(), callback)
    }
    private fun serverCallback(owner: Owner): BluetoothGattServerCallback = object : BluetoothGattServerCallback() {
        private val epoch = owner.serverEpoch
        private fun dispatch(owner: Owner, action: () -> Unit) {
            this@NearbyBluetooth.dispatch(owner) { if (epoch == owner.serverEpoch) action() }
        }
        private fun connection(device: BluetoothDevice): Connection? = owner.connections.values.firstOrNull { it.device == device }
        override fun onServiceAdded(status: Int, service: BluetoothGattService) {
            // Addition may finish after Stop. That completion still owns cleanup,
            // but must never advertise or deliver events for the retired owner.
            main.post {
                if (owners[owner.role] !== owner || epoch != owner.serverEpoch ||
                    owner.service !== service || !owner.serviceAdding) return@post
                owner.serviceAdding = false
                if (owner.retired) {
                    retire(owner, "NEARBY_BLUETOOTH_RETIRED")
                    return@post
                }
                try {
                    if (status != BluetoothGatt.GATT_SUCCESS) fail(owner, "NEARBY_BLUETOOTH_SERVICE_FAILED") else advertise(owner)
                } catch (error: Exception) { fail(owner, nearbyFailure(error)) }
            }
        }
        override fun onConnectionStateChange(device: BluetoothDevice, status: Int, newState: Int) { dispatch(owner) {
            if (Log.isLoggable(TAG, Log.DEBUG)) Log.i(TAG, "server state=$newState status=$status")
            if (newState == BluetoothProfile.STATE_CONNECTED && status == BluetoothGatt.GATT_SUCCESS) {
                if (!owner.started || connection(device) != null || owner.connections.isNotEmpty()) { owner.server?.cancelConnection(device); return@dispatch }
                val connection = Connection(UUID.randomUUID().toString(), device, null, null)
                owner.connections[connection.id] = connection
                connection.deadline = Runnable { if (owns(owner, connection)) drop(owner, connection, "NEARBY_BLUETOOTH_SUBSCRIBE_TIMEOUT") }.also { main.postDelayed(it, 10_000L) }
            } else if (newState == BluetoothProfile.STATE_DISCONNECTED || status != BluetoothGatt.GATT_SUCCESS)
                connection(device)?.let { drop(owner, it, "NEARBY_BLUETOOTH_DISCONNECTED") }
        } }
        override fun onMtuChanged(device: BluetoothDevice, mtu: Int) { dispatch(owner) {
            connection(device)?.let {
                if (mtu < 23 || it.ready && it.budget != minOf(mtu - 3, 512)) drop(owner, it, "NEARBY_BLUETOOTH_MTU_CHANGED")
                else it.budget = minOf(mtu - 3, 512)
            }
        } }
        override fun onCharacteristicReadRequest(device: BluetoothDevice, requestId: Int, offset: Int, characteristic: BluetoothGattCharacteristic) {
            if (Log.isLoggable(TAG, Log.DEBUG)) Log.i(TAG, "read request offset=$offset")
            dispatch(owner) {
            val valid = characteristic === owner.publicInfo && offset == 0 && owner.started && connection(device) != null
            owner.server?.sendResponse(device, requestId, if (valid) BluetoothGatt.GATT_SUCCESS else BluetoothGatt.GATT_REQUEST_NOT_SUPPORTED, 0, if (valid) owner.info else null)
        } }
        override fun onCharacteristicWriteRequest(device: BluetoothDevice, requestId: Int, characteristic: BluetoothGattCharacteristic,
            preparedWrite: Boolean, responseNeeded: Boolean, offset: Int, value: ByteArray) {
            if (value.size > 512) { dispatch(owner) { connection(device)?.let { drop(owner, it, "NEARBY_BLUETOOTH_INVALID_PACKET") } }; return }
            val bytes = value.copyOf()
            dispatch(owner) {
                val connection = connection(device)
                val valid = characteristic === owner.channel && !preparedWrite && responseNeeded && offset == 0 &&
                    connection != null && connection.ready && packet(bytes, connection.budget)
                val replied = responseNeeded && owner.server?.sendResponse(device, requestId, if (valid) BluetoothGatt.GATT_SUCCESS else BluetoothGatt.GATT_REQUEST_NOT_SUPPORTED, 0, null) == true
                if (valid && replied) event(owner, "bytes", connection, JSObject().put("bytes", standard(bytes)))
                else if (connection != null) drop(owner, connection, "NEARBY_BLUETOOTH_INVALID_PACKET")
                bytes.fill(0)
            }
        }
        override fun onDescriptorReadRequest(device: BluetoothDevice, requestId: Int, offset: Int, descriptor: BluetoothGattDescriptor) { dispatch(owner) {
            val connection = connection(device)
            val valid = descriptor === owner.cccd && offset == 0 && connection != null
            owner.server?.sendResponse(device, requestId, if (valid) BluetoothGatt.GATT_SUCCESS else BluetoothGatt.GATT_REQUEST_NOT_SUPPORTED, 0,
                if (valid) { if (connection!!.ready) BluetoothGattDescriptor.ENABLE_INDICATION_VALUE else BluetoothGattDescriptor.DISABLE_NOTIFICATION_VALUE } else null)
        } }
        override fun onDescriptorWriteRequest(device: BluetoothDevice, requestId: Int, descriptor: BluetoothGattDescriptor,
            preparedWrite: Boolean, responseNeeded: Boolean, offset: Int, value: ByteArray) {
            if (Log.isLoggable(TAG, Log.DEBUG)) Log.i(TAG, "descriptor write size=${value.size} prepared=$preparedWrite response=$responseNeeded")
            if (value.size > 2) { dispatch(owner) { connection(device)?.let { drop(owner, it, "NEARBY_BLUETOOTH_INVALID_SUBSCRIPTION") } }; return }
            val bytes = value.copyOf()
            dispatch(owner) {
                val connection = connection(device)
                val enable = bytes.contentEquals(BluetoothGattDescriptor.ENABLE_INDICATION_VALUE)
                val disable = bytes.contentEquals(BluetoothGattDescriptor.DISABLE_NOTIFICATION_VALUE)
                val valid = descriptor === owner.cccd && !preparedWrite && responseNeeded && offset == 0 && connection != null && (enable || disable)
                val replied = if (responseNeeded) owner.server?.sendResponse(device, requestId, if (valid) BluetoothGatt.GATT_SUCCESS else BluetoothGatt.GATT_REQUEST_NOT_SUPPORTED, 0, null) == true else false
                if (!valid || !replied) { if (connection != null) drop(owner, connection, "NEARBY_BLUETOOTH_INVALID_SUBSCRIPTION") }
                else if (disable) drop(owner, connection!!, "NEARBY_BLUETOOTH_UNSUBSCRIBED")
                else if (!connection!!.ready) {
                    connection.ready = true; connection.stage = "ready"
                    connection.deadline?.let { main.removeCallbacks(it) }; connection.deadline = null
                    event(owner, "connected", connection, JSObject().put("packetBudget", connection.budget))
                }
                bytes.fill(0)
            }
        }
        override fun onExecuteWrite(device: BluetoothDevice, requestId: Int, execute: Boolean) { dispatch(owner) {
            owner.server?.sendResponse(device, requestId, BluetoothGatt.GATT_REQUEST_NOT_SUPPORTED, 0, null)
            connection(device)?.let { drop(owner, it, "NEARBY_BLUETOOTH_INVALID_PACKET") }
        } }
        override fun onNotificationSent(device: BluetoothDevice, status: Int) { dispatch(owner) {
            connection(device)?.let { connection ->
                if (connection.send == null) return@let
                completeSend(owner, connection, status)
            }
        } }
    }

    fun connect(invoke: Invoke) {
        val reply = NearbyReply(invoke)
        var owner: Owner? = null
        var connection: Connection? = null
        try {
            val args = invoke.getArgs()
            val active = requireOwner(args.getString("owner")); owner = active
            nearbyRequire(active.role == "central" && active.started && active.foreground() && permitted(active), "NEARBY_BLUETOOTH_NOT_STARTED")
            nearbyRequire(active.connections.isEmpty(), "NEARBY_BLUETOOTH_CONNECTION_LIMIT")
            val peer = args.getString("peer")
            val device = active.peers[peer] ?: throw NearbyError("NEARBY_BLUETOOTH_STALE_PEER")
            if (Log.isLoggable(TAG, Log.DEBUG)) Log.i(TAG, "connect peer=$peer")
            val next = Connection(UUID.randomUUID().toString(), device, peer, reply); connection = next
            active.connections[next.id] = next
            next.deadline = Runnable { if (owns(active, next)) drop(active, next, "NEARBY_BLUETOOTH_CONNECT_TIMEOUT") }.also { main.postDelayed(it, 30_000L) }
            next.gatt = device.connectGatt(context, false, centralCallback(active, next), BluetoothDevice.TRANSPORT_LE)
                ?: throw NearbyError("NEARBY_BLUETOOTH_CONNECT_FAILED")
        } catch (error: Exception) {
            reply.reject(nearbyFailure(error))
            if (owner != null && connection != null) drop(owner, connection, nearbyFailure(error))
        }
    }
    private fun centralCallback(owner: Owner, connection: Connection): BluetoothGattCallback = object : BluetoothGattCallback() {
        private fun live(gatt: BluetoothGatt): Boolean = owns(owner, connection) && connection.gatt === gatt
        override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) { dispatch(owner) {
            if (!live(gatt)) return@dispatch
            if (Log.isLoggable(TAG, Log.DEBUG)) Log.i(TAG, "central state=$newState status=$status stage=${connection.stage}")
            if (status != BluetoothGatt.GATT_SUCCESS || newState == BluetoothProfile.STATE_DISCONNECTED) { drop(owner, connection, "NEARBY_BLUETOOTH_DISCONNECTED"); return@dispatch }
            if (newState == BluetoothProfile.STATE_CONNECTED && connection.stage == "connecting") {
                connection.stage = "services"
                if (!gatt.discoverServices()) drop(owner, connection, "NEARBY_BLUETOOTH_DISCOVERY_FAILED")
            }
        } }
        override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) { dispatch(owner) {
            if (!live(gatt) || connection.stage != "services") return@dispatch
            val service = gatt.getService(SERVICE)
            val channel = service?.getCharacteristic(CHANNEL)
            val info = service?.getCharacteristic(INFO)
            val cccd = channel?.getDescriptor(CCCD)
            if (status != BluetoothGatt.GATT_SUCCESS || channel == null || !channelPropertiesValid(channel.properties) || info?.properties != BluetoothGattCharacteristic.PROPERTY_READ || cccd == null) {
                Log.w(TAG, "invalid service status=$status present=${service != null} " +
                    "channelProperties=${channel?.properties} infoProperties=${info?.properties} cccd=${cccd != null}")
                drop(owner, connection, "NEARBY_BLUETOOTH_SERVICE_INVALID"); return@dispatch
            }
            connection.channel = channel; connection.info = info; connection.cccd = cccd
            connection.stage = "mtu"
            if (!gatt.requestMtu(517)) readInfo(owner, connection)
        } }
        override fun onMtuChanged(gatt: BluetoothGatt, mtu: Int, status: Int) { dispatch(owner) {
            if (!live(gatt)) return@dispatch
            if (connection.stage == "mtu") {
                connection.budget = if (status == BluetoothGatt.GATT_SUCCESS && mtu >= 23) minOf(mtu - 3, 512) else 20
                readInfo(owner, connection)
            } else if (status == BluetoothGatt.GATT_SUCCESS && minOf(mtu - 3, 512) != connection.budget)
                drop(owner, connection, "NEARBY_BLUETOOTH_MTU_CHANGED")
        } }
        @Deprecated("Android pre-33 callback")
        override fun onCharacteristicRead(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, status: Int) {
            onCharacteristicRead(gatt, characteristic, characteristic.value ?: byteArrayOf(), status)
        }
        override fun onCharacteristicRead(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, value: ByteArray, status: Int) {
            if (value.size > 18) { dispatch(owner) { if (live(gatt)) drop(owner, connection, "NEARBY_BLUETOOTH_INFO_INVALID") }; return }
            val bytes = value.copyOf()
            dispatch(owner) {
                if (!live(gatt) || connection.info !== characteristic || connection.stage != "info") { bytes.fill(0); return@dispatch }
                if (status != BluetoothGatt.GATT_SUCCESS || bytes.size != 18 || bytes[0] != 2.toByte() || bytes[1] != 0.toByte()) {
                    bytes.fill(0); drop(owner, connection, "NEARBY_BLUETOOTH_INFO_INVALID"); return@dispatch
                }
                connection.publicInfo = bytes
                connection.stage = "subscribe"
                if (!gatt.setCharacteristicNotification(connection.channel!!, true)) { drop(owner, connection, "NEARBY_BLUETOOTH_SUBSCRIBE_FAILED"); return@dispatch }
                val cccd = connection.cccd!!
                val accepted = if (Build.VERSION.SDK_INT >= 33) gatt.writeDescriptor(cccd, BluetoothGattDescriptor.ENABLE_INDICATION_VALUE) == BluetoothStatusCodes.SUCCESS
                    else { cccd.value = BluetoothGattDescriptor.ENABLE_INDICATION_VALUE; gatt.writeDescriptor(cccd) }
                if (!accepted) drop(owner, connection, "NEARBY_BLUETOOTH_SUBSCRIBE_FAILED")
            }
        }
        override fun onDescriptorWrite(gatt: BluetoothGatt, descriptor: BluetoothGattDescriptor, status: Int) { dispatch(owner) {
            if (!live(gatt) || descriptor !== connection.cccd || connection.stage != "subscribe") return@dispatch
            if (status != BluetoothGatt.GATT_SUCCESS) { drop(owner, connection, "NEARBY_BLUETOOTH_SUBSCRIBE_FAILED"); return@dispatch }
            connection.ready = true; connection.stage = "ready"
            connection.deadline?.let { main.removeCallbacks(it) }; connection.deadline = null
            val info = connection.publicInfo!!
            connection.connect?.resolve(JSObject().put("owner", owner.id).put("connection", connection.id).put("info", url(info)).put("packetBudget", connection.budget))
            connection.connect = null
        } }
        @Deprecated("Android pre-33 callback")
        override fun onCharacteristicChanged(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic) {
            onCharacteristicChanged(gatt, characteristic, characteristic.value ?: byteArrayOf())
        }
        override fun onCharacteristicChanged(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, value: ByteArray) {
            if (value.size > 512) { dispatch(owner) { if (live(gatt)) drop(owner, connection, "NEARBY_BLUETOOTH_INVALID_PACKET") }; return }
            val bytes = value.copyOf()
            dispatch(owner) {
                if (!live(gatt)) { bytes.fill(0); return@dispatch }
                if (connection.channel !== characteristic || !connection.ready || !packet(bytes, connection.budget)) drop(owner, connection, "NEARBY_BLUETOOTH_INVALID_PACKET")
                else event(owner, "bytes", connection, JSObject().put("bytes", standard(bytes)))
                bytes.fill(0)
            }
        }
        override fun onCharacteristicWrite(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, status: Int) { dispatch(owner) {
            if (live(gatt) && connection.channel === characteristic && connection.send != null) completeSend(owner, connection, status)
        } }
    }
    private fun readInfo(owner: Owner, connection: Connection) {
        connection.stage = "info"
        if (connection.gatt?.readCharacteristic(connection.info!!) != true) drop(owner, connection, "NEARBY_BLUETOOTH_INFO_INVALID")
    }
    private fun requireOwner(id: String): Owner {
        nearbyUuid(id)
        return owners.values.firstOrNull { it.id == id && !it.retired } ?: throw NearbyError("NEARBY_BLUETOOTH_STALE_OWNER")
    }
    fun send(invoke: Invoke) {
        val reply = NearbyReply(invoke)
        var bytes: ByteArray? = null
        try {
            val args = invoke.getArgs()
            val owner = requireOwner(args.getString("owner"))
            val connection = owner.connections[args.getString("connection")] ?: throw NearbyError("NEARBY_BLUETOOTH_STALE_CONNECTION")
            nearbyRequire(owner.foreground() && permitted(owner) && connection.ready, "NEARBY_BLUETOOTH_NOT_READY")
            nearbyRequire(connection.send == null, "NEARBY_BLUETOOTH_BACKPRESSURE")
            val encoded = args.getString("bytes")
            nearbyRequire(encoded.length <= 684, "NEARBY_BLUETOOTH_INVALID_PACKET")
            val payload = Base64.decode(encoded, Base64.NO_WRAP)
            bytes = payload
            nearbyRequire(standard(payload) == encoded && packet(payload, connection.budget), "NEARBY_BLUETOOTH_INVALID_PACKET")
            connection.send = reply; connection.sending = payload
            connection.sendDeadline = Runnable { if (owns(owner, connection) && connection.send === reply) drop(owner, connection, "NEARBY_BLUETOOTH_SEND_TIMEOUT") }.also { main.postDelayed(it, 10_000L) }
            val accepted = if (owner.role == "central") {
                val gatt = connection.gatt!!; val channel = connection.channel!!
                if (Build.VERSION.SDK_INT >= 33) gatt.writeCharacteristic(channel, payload, BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT) == BluetoothStatusCodes.SUCCESS
                else { channel.writeType = BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT; channel.value = payload; gatt.writeCharacteristic(channel) }
            } else {
                val server = owner.server!!; val channel = owner.channel!!
                if (Build.VERSION.SDK_INT >= 33) server.notifyCharacteristicChanged(connection.device, channel, true, payload) == BluetoothStatusCodes.SUCCESS
                else { channel.value = payload; server.notifyCharacteristicChanged(connection.device, channel, true) }
            }
            if (!accepted) drop(owner, connection, "NEARBY_BLUETOOTH_SEND_FAILED")
        } catch (error: Exception) {
            bytes?.fill(0)
            reply.reject(nearbyFailure(error))
            // If dispatch failed after recording the packet, retire that exact in-flight connection.
            owners.values.toList().forEach { owner -> owner.connections.values.toList().filter { it.send === reply }.forEach { drop(owner, it, nearbyFailure(error)) } }
        }
    }
    private fun completeSend(owner: Owner, connection: Connection, status: Int) {
        if (status != BluetoothGatt.GATT_SUCCESS) { drop(owner, connection, "NEARBY_BLUETOOTH_SEND_FAILED"); return }
        val reply = connection.send ?: return
        connection.send = null
        connection.sending?.fill(0); connection.sending = null
        connection.sendDeadline?.let { main.removeCallbacks(it) }; connection.sendDeadline = null
        reply.resolve(JSObject().put("accepted", true))
    }
    fun disconnect(args: org.json.JSONObject): JSObject {
        val id = nearbyUuid(args.getString("owner"))
        val owner = owners.values.firstOrNull { it.id == id }
        owner?.connections?.get(args.getString("connection"))?.let {
            nearbyRequire(drop(owner, it, "NEARBY_BLUETOOTH_DISCONNECTED"), "NEARBY_BLUETOOTH_CLEANUP_FAILED")
        }
        return JSObject()
    }
    fun stop(args: org.json.JSONObject): JSObject {
        val id = nearbyUuid(args.getString("owner"))
        owners.values.firstOrNull { it.id == id }?.let {
            nearbyRequire(retire(it, "NEARBY_BLUETOOTH_STOPPED"), "NEARBY_BLUETOOTH_CLEANUP_FAILED")
        }
        return JSObject()
    }
    fun scan(args: org.json.JSONObject): JSObject {
        val owner = requireOwner(args.getString("owner"))
        nearbyRequire(owner.role == "central" && owner.foreground() && permitted(owner), "NEARBY_BLUETOOTH_NOT_STARTED")
        val enabled = args.getBoolean("enabled")
        if (owner.scanning == enabled) return JSObject()
        owner.scan?.let { owner.scanner?.stopScan(it) }
        owner.scan = null; owner.scanner = null; owner.scanning = enabled
        if (enabled) startScan(owner, manager.adapter ?: throw NearbyError("NEARBY_BLUETOOTH_UNAVAILABLE"))
        return JSObject()
    }
    fun fail(owner: Owner, code: String) {
        if (!owns(owner)) return
        val sequence = ++owner.sequence
        val clean = retire(owner, code)
        runCatching {
            emit(JSObject().put("owner", owner.id).put("sequence", sequence).put("kind", "failed")
                .put("error", if (clean) code else "NEARBY_BLUETOOTH_CLEANUP_FAILED"))
        }
    }
    private fun drop(owner: Owner, connection: Connection, code: String, notify: Boolean = true): Boolean {
        Log.w(TAG, "drop code=$code ready=${connection.ready} peer=${connection.peer ?: "-"}")
        if (owner.connections.remove(connection.id) !== connection) return true
        connection.deadline?.let { main.removeCallbacks(it) }
        connection.sendDeadline?.let { main.removeCallbacks(it) }
        connection.connect?.reject(code); connection.connect = null
        connection.send?.reject(code); connection.send = null
        connection.sending?.fill(0); connection.sending = null
        connection.publicInfo?.fill(0); connection.publicInfo = null
        val failedPeer = if (!connection.ready) connection.peer else null
        if (failedPeer != null) {
            // A failed candidate may be a stale advertisement; drop it so the
            // next attempt cannot keep timing out against the same peer.
            owner.peers.remove(failedPeer)
            owner.scannedAddresses.remove(connection.device.address)
            event(owner, "lost", extra = JSObject().put("peer", failedPeer))
        }
        val gatt = connection.gatt; connection.gatt = null
        var clean = true
        if (gatt != null) {
            runCatching { gatt.disconnect() }
            try { gatt.close() } catch (_: Exception) { owner.closingGatts.add(gatt); clean = false }
        }
        if (owner.role == "peripheral") {
            try { owner.server?.cancelConnection(connection.device) } catch (_: Exception) { clean = false }
            // Keep the GATT server and advertisement alive across the exchange.
            // Rebuilding them races the receiver's next connect and produced
            // BLE_CONNECT_TIMEOUT. Stale callbacks are already fenced by the
            // per-device connection lookup and the retained server epoch.
        }
        if (notify && owns(owner)) {
            if (!clean) fail(owner, "NEARBY_BLUETOOTH_CLEANUP_FAILED")
            else event(owner, "disconnected", connection, JSObject().put("error", code))
        }
        return clean
    }
    fun retireAll() {
        var clean = true
        owners.values.toList().forEach { if (!retire(it, "NEARBY_BLUETOOTH_RETIRED")) clean = false }
        nearbyRequire(clean, "NEARBY_BLUETOOTH_CLEANUP_FAILED")
    }
    private fun retire(owner: Owner, code: String): Boolean {
        if (owners[owner.role] !== owner) return true
        owner.retired = true // fence callbacks BEFORE teardown; keep failed cleanup owned.
        owner.start.reject(code)
        owner.deadline?.let { main.removeCallbacks(it) }
        owner.deadline = null
        owner.connections.values.toList().forEach { drop(owner, it, code, false) }
        owner.closingGatts.removeAll { gatt -> runCatching { gatt.close() }.isSuccess }
        try {
            owner.scan?.let { owner.scanner?.stopScan(it) }
            owner.scan = null; owner.scanner = null
        } catch (_: Exception) { /* retained for exact-owner Stop retry */ }
        try {
            owner.advertisement?.let { owner.advertiser?.stopAdvertising(it) }
            owner.advertisement = null; owner.advertiser = null
        } catch (_: Exception) { /* retained for exact-owner Stop retry */ }
        // Closing before addService completes can leave a late OS handle-map
        // entry after deregistration. Retain this owner until its exact callback.
        if (!owner.serviceAdding) {
            runCatching { owner.server?.clearServices() }
            try {
                owner.server?.close()
                owner.server = null; owner.service = null; owner.channel = null; owner.publicInfo = null; owner.cccd = null
            } catch (_: Exception) { /* retained for exact-owner Stop retry */ }
        }
        try {
            owner.receiver?.let { context.unregisterReceiver(it) }
            owner.receiver = null
        } catch (_: IllegalArgumentException) { owner.receiver = null }
        catch (_: Exception) { /* retained for exact-owner Stop retry */ }
        owner.peers.clear(); owner.scannedAddresses.clear(); owner.info?.fill(0)
        val clean = owner.scanner == null && owner.advertiser == null && owner.server == null &&
            owner.receiver == null && owner.closingGatts.isEmpty()
        if (clean && owners[owner.role] === owner) owners.remove(owner.role)
        return clean
    }
    private fun packet(bytes: ByteArray, budget: Int): Boolean {
        if (bytes.size !in 9..budget || budget !in 20..512 || bytes[0] != 1.toByte() || bytes[1].toInt() !in 1..2) return false
        val offset = (bytes[4].toInt() and 255) * 256 + (bytes[5].toInt() and 255)
        val total = (bytes[6].toInt() and 255) * 256 + (bytes[7].toInt() and 255)
        return total in 1..4096 && offset < total && offset + bytes.size - 8 <= total
    }
    private fun standard(bytes: ByteArray): String = Base64.encodeToString(bytes, Base64.NO_WRAP)
    private fun url(bytes: ByteArray): String = Base64.encodeToString(bytes, Base64.NO_WRAP or Base64.NO_PADDING or Base64.URL_SAFE)
}
