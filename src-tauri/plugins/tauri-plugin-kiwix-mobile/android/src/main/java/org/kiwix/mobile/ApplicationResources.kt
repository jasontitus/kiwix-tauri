package org.kiwix.mobile

import android.content.Context
import android.net.wifi.WifiManager
import android.os.CancellationSignal
import android.os.Handler
import android.os.Looper
import androidx.annotation.Keep
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import com.fasterxml.jackson.databind.ObjectMapper
import java.lang.ref.WeakReference
import java.util.concurrent.CountDownLatch
import java.util.concurrent.ExecutorService
import java.util.concurrent.FutureTask
import org.json.JSONObject

/** Application resources only: never owns an Activity, WebView, reader, or UI callback. */
@Keep
internal class ApplicationResources(
    private val context: Context,
    private val worker: ExecutorService,
    private val documents: ExternalDocuments,
    private val speech: NativeSpeech,
    private val networkPolicy: MobileNetworkPolicy,
    private val mapper: ObjectMapper,
    private val plugin: WeakReference<KiwixMobilePlugin>,
    private val nearbyBluetooth: NearbyBluetooth,
    private val nearbyJoin: NearbyJoin,
    private val nearbyRoutes: NearbyRoutes
) {
    private val main = Handler(Looper.getMainLooper())
    private var multicastLock: WifiManager.MulticastLock? = null
    private var wifiLock: WifiManager.WifiLock? = null

    fun enableDiscovery() {
        if (multicastLock == null) {
            val wifi = context.getSystemService(Context.WIFI_SERVICE) as WifiManager
            multicastLock = wifi.createMulticastLock("${context.packageName}:kiwix-discovery")
                .apply { setReferenceCounted(false) }
        }
        if (multicastLock?.isHeld != true) multicastLock?.acquire()
    }

    fun discoveryIsHeld(): Boolean = multicastLock?.isHeld == true

    fun releaseDiscovery() {
        multicastLock?.let { if (it.isHeld) it.release() }
        multicastLock = null
    }

    /**
     * Keep the Wi-Fi radio out of power save while LocalSwarm work is live.
     *
     * A high-performance lock stops the firmware from napping between the
     * bursty QUIC exchanges that a transfer produces; TCP's continuous ack
     * stream already keeps it awake, which is why raw TCP is unaffected.
     * The lock exists only for the admitted transfer window and is released
     * with the last holder, on failure, cancellation and shutdown.
     */
    fun setWifiPerformance(held: Boolean) {
        if (held) {
            if (wifiLock == null) {
                val wifi = context.getSystemService(Context.WIFI_SERVICE) as WifiManager
                wifiLock = wifi.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF,
                    "${context.packageName}:kiwix-localswarm").apply { setReferenceCounted(false) }
            }
            if (wifiLock?.isHeld != true) wifiLock?.acquire()
        } else {
            wifiLock?.let { if (it.isHeld) it.release() }
            wifiLock = null
        }
    }

    private fun <T> onMain(action: () -> T): T {
        check(Looper.myLooper() != Looper.getMainLooper()) { "NATIVE_OPERATION_REQUIRES_WORKER" }
        val task = FutureTask<T> { action() }
        check(main.post(task)) { "NATIVE_MAIN_LOOP_UNAVAILABLE" }
        return task.get()
    }

    /** Already-serialized JSON from an Invoke callback; never re-parsed. */
    private class RawJson(val json: String)

    /** Fixed Rust-only service operations, not a second general plugin/JavaScript dispatcher. */
    @Keep
    fun call(command: String, payload: String): String {
        return try {
            val value = when (command) {
                "activityState" -> {
                    // Snapshot-only calls are safe inline on Java's main thread;
                    // UI lifecycle changes keep their worker-only dispatch below.
                    if (Looper.myLooper() == Looper.getMainLooper()) requirePlugin().activityState()
                    else onMain { requirePlugin().activityState() }
                }
                "finishActivity" -> {
                    // Rust captures the target before async dispatch; never rebase
                    // a queued explicit exit onto a replacement Activity.
                    val generation = JSONObject(payload).getLong("id")
                    onMain { requirePlugin().finishActivity(generation); JSONObject() }
                }
                "retireWindow", "openWindow" -> {
                    val generation = JSONObject(payload).getLong("id")
                    onMain {
                        val owner = requirePlugin()
                        if (command == "retireWindow") owner.retireWindow(generation)
                        else owner.openWindow(generation)
                        JSONObject()
                    }
                }
                "networkState" -> networkPolicy.snapshot()
                // The Rust async endpoint already owns a blocking worker.
                // Notification authority needs a durable commit, not the UI queue.
                "downloadNotifications" -> {
                    DownloadNotifications.update(context, JSONObject(payload))
                    JSONObject()
                }
                "takeTransferEvents" -> JSONObject().put("events", TransferEvents.peek(context))
                "acknowledgeTransferEvent" -> {
                    val args = JSONObject(payload)
                    TransferEvents.acknowledge(context, args.getString("event"), args.getString("requestId"))
                    JSONObject()
                }
                "crashCapturePolicy" -> {
                    CrashIntake.setEnabled(JSONObject(payload).getBoolean("enabled")); JSONObject()
                }
                "applyLocale" -> onMain {
                    val args = JSONObject(payload)
                    AppLocale.apply(context, args.getString("locale"), args.optBoolean("preservePlatform", false))
                    AppLocale.state(context)
                }
                "bindLocalSocket" -> JSONObject().put("route",
                    networkPolicy.bindLocalSocket(JSONObject(payload).getInt("fd")))
                "isLocalPeer" -> {
                    val args = JSONObject(payload)
                    JSONObject().put("local", networkPolicy.isLocalPeer(args.getLong("route"), args.getString("address")))
                }
                // Wi-Fi locks are process resources with no Activity or main-thread
                // requirement; transfer workers set them directly.
                "wifiPerformance" -> {
                    setWifiPerformance(JSONObject(payload).getBoolean("held")); JSONObject()
                }
                else -> serviceCall(command, payload)
            }
            if (value is RawJson) "{\"value\":${value.json}}"
            else JSONObject().put("value", value).toString()
        } catch (error: Exception) {
            JSONObject().put("error", if (command.startsWith("nearby") || command.startsWith("hotspotQuic"))
                nearbyFailure(error) else error.toString()).toString()
        }
    }

    private fun requirePlugin(): KiwixMobilePlugin =
        plugin.get() ?: throw IllegalStateException("NATIVE_PLUGIN_UNAVAILABLE")

    private fun serviceCall(command: String, payload: String): Any {
        val completed = CountDownLatch(1)
        var response: String? = null
        var failure: String? = null
        val invoke = Invoke(0, command, 0, 1, { callback, data ->
            if (callback == 0L) response = data else failure = data
            completed.countDown()
        }, payload, mapper)
        when (command) {
            "nearbyBluetoothConnect", "nearbyBluetoothSend", "nearbyBluetoothDisconnect", "nearbyBluetoothStop", "nearbyBluetoothScan",
            "nearbyJoinState", "nearbyJoinBindSocket", "nearbyJoinRelease", "nearbyJoinCancel",
            "nearbyRouteBindSocket", "nearbyRouteState", "nearbyRetire", "hotspotQuicRoute", "hotspotQuicState" -> {
                nearbyRequire(Looper.myLooper() != Looper.getMainLooper(), "NEARBY_WORKER_REQUIRED")
                check(main.post {
                    try {
                        when (command) {
                            "nearbyBluetoothConnect" -> nearbyBluetooth.connect(invoke)
                            "nearbyBluetoothSend" -> nearbyBluetooth.send(invoke)
                            "nearbyBluetoothDisconnect" -> invoke.resolve(nearbyBluetooth.disconnect(invoke.getArgs()))
                            "nearbyBluetoothStop" -> invoke.resolve(nearbyBluetooth.stop(invoke.getArgs()))
                            "nearbyBluetoothScan" -> invoke.resolve(nearbyBluetooth.scan(invoke.getArgs()))
                            "hotspotQuicRoute" -> requirePlugin().hotspotQuicRoute(invoke)
                            "hotspotQuicState" -> requirePlugin().hotspotQuicState(invoke)
                            "nearbyJoinState" -> invoke.resolve(nearbyJoin.state(invoke.getArgs()))
                            "nearbyJoinBindSocket" -> invoke.resolve(nearbyJoin.bind(invoke.getArgs()))
                            "nearbyJoinRelease" -> invoke.resolve(nearbyJoin.release(invoke.getArgs()))
                            "nearbyJoinCancel" -> invoke.resolve(nearbyJoin.cancel(invoke.getArgs()))
                            "nearbyRouteBindSocket" -> invoke.resolve(nearbyRoutes.bind(invoke.getArgs()))
                            "nearbyRouteState" -> invoke.resolve(nearbyRoutes.state(invoke.getArgs()))
                            "nearbyRetire" -> {
                                var failure: String? = null
                                try { nearbyBluetooth.retireAll() } catch (error: Exception) { failure = nearbyFailure(error) }
                                try { nearbyJoin.retireAll() } catch (error: Exception) { failure = nearbyFailure(error) }
                                failure?.let { throw NearbyError(it) }
                                invoke.resolve(JSObject())
                            }
                        }
                    } catch (error: Exception) {
                        val code = nearbyFailure(error)
                        invoke.reject(code, code)
                    }
                }) { "NATIVE_MAIN_LOOP_UNAVAILABLE" }
            }
            "acquireExternalArchive", "releaseExternalArchive", "forgetExternalArchive",
            "prepareExternalArchiveMove", "deleteExternalArchiveMove" -> worker.execute {
                try {
                    val args = invoke.getArgs()
                    when (command) {
                        "acquireExternalArchive" -> {
                            invoke.resolve(documents.acquire(args.getString("reference"), CancellationSignal()))
                        }
                        "releaseExternalArchive" -> { documents.release(args.getString("lease")); invoke.resolve(JSObject()) }
                        "prepareExternalArchiveMove" -> invoke.resolve(documents.prepareMove(args.getString("reference")))
                        "deleteExternalArchiveMove" -> invoke.resolve(documents.deleteMove(
                            args.getString("reference"), args.getString("receipt"), args.getJSONArray("parts")))
                        else -> { documents.forget(args.getString("reference")); invoke.resolve(JSObject()) }
                    }
                } catch (error: Exception) {
                    // Acquisition is nondestructive. Preserve its exact typed
                    // failure instead of nesting it in Invoke/Java exceptions.
                    val failure = if (command == "acquireExternalArchive") providerAcquisitionFailure(error) else error
                    if (command == "acquireExternalArchive" && failure is NativeDocumentError)
                        invoke.resolve(JSObject().put("error", JSObject().put("code", failure.code)))
                    else invoke.reject(failure.toString())
                }
            }
            "speechStop", "speechRetire", "speechPause", "speechResume", "speechPrevious",
            "speechNext", "speechRewind", "speechForward", "speechSeek",
            "speechEngineStart", "speechEngineUpdate", "speechEngineStop",
            "discoveryState", "transferState", "registerListener", "takeIncomingDocuments" -> {
                check(main.post {
                    try {
                        when (command) {
                            // Reuse Plugin's configured Channel deserializer and
                            // real listener map; registration survives Activity loss.
                            "registerListener" -> requirePlugin().registerListener(invoke)
                            "takeIncomingDocuments" -> requirePlugin().takeIncomingDocuments(invoke)
                            "speechStop" -> speech.stop(invoke)
                            "speechRetire" -> speech.retire(invoke)
                            "speechPause" -> speech.pause(invoke)
                            "speechResume" -> speech.resume(invoke)
                            "speechPrevious" -> speech.previous(invoke)
                            "speechNext" -> speech.next(invoke)
                            "speechRewind" -> speech.rewind(invoke)
                            "speechForward" -> speech.forward(invoke)
                            "speechSeek" -> speech.seek(invoke)
                            "speechEngineStart" -> {
                                val args = invoke.getArgs()
                                val id = args.getString("id")
                                val token = args.getString("token")
                                if (speech.admitExternal(id, token) && SpeechPlayback.start(context, id,
                                        SpeechLabels.from(args.optJSONObject("labels")), true, true,
                                        title = args.getString("title"), subtitle = args.optString("subtitle"),
                                        external = true)) {
                                    invoke.resolve(JSObject())
                                } else {
                                    invoke.reject("SPEECH_FOREGROUND_REQUIRED")
                                }
                            }
                            "speechEngineUpdate" -> {
                                val args = invoke.getArgs()
                                if (SpeechPlayback.update(args.getString("id"), args.optBoolean("speaking", true),
                                        args.optBoolean("paused", false), args.optBoolean("canPrevious", true))) {
                                    invoke.resolve(JSObject())
                                } else {
                                    invoke.reject("SPEECH_AUDIO_SESSION_FAILED")
                                }
                            }
                            "speechEngineStop" -> {
                                SpeechPlayback.stop(context, invoke.getArgs().getString("id"))
                                invoke.resolve(JSObject())
                            }
                            "discoveryState" -> {
                                if (invoke.getArgs().getBoolean("enabled")) {
                                    // Maintenance of an existing admitted lease
                                    // is not permission to start discovery.
                                    check(KiwixTransferService.running && discoveryIsHeld()) {
                                        "DISCOVERY_UI_ADMISSION_REQUIRED"
                                    }
                                } else releaseDiscovery()
                                invoke.resolve(JSObject())
                            }
                            "transferState" -> requirePlugin().transferState(invoke)
                        }
                    } catch (error: Exception) { invoke.reject(error.toString()) }
                }) { "NATIVE_MAIN_LOOP_UNAVAILABLE" }
            }
            else -> throw IllegalArgumentException("NOT_AN_APPLICATION_RESOURCE_OPERATION")
        }
        completed.await()
        failure?.let {
            if (command.startsWith("nearby") || command.startsWith("hotspotQuic")) {
                val code = runCatching { JSONObject(it).optString("code") }.getOrNull()
                throw NearbyError(code?.takeIf { value -> value.matches(Regex("NEARBY_[A-Z_]+")) } ?: "NEARBY_NATIVE_FAILED")
            }
            throw IllegalStateException(it)
        }
        // The Invoke callback already holds serialized JSON; splice it instead of
        // parsing it back into an object just to serialize it again.
        return RawJson(response ?: "null")
    }
}
