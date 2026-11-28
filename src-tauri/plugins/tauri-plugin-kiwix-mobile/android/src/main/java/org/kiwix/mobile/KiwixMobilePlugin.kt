package org.kiwix.mobile

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.app.PendingIntent
import android.content.ClipData
import android.content.Intent
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.net.Uri
import android.os.Build
import android.os.CancellationSignal
import android.os.Handler
import android.os.Looper
import android.os.OperationCanceledException
import android.os.ParcelFileDescriptor
import android.os.Bundle
import android.os.ResultReceiver
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.webkit.WebView
import android.webkit.JavascriptInterface
import android.view.ViewGroup
import android.view.View
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.TextView
import androidx.activity.result.ActivityResult
import androidx.annotation.Keep
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import java.lang.ref.WeakReference
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import org.json.JSONObject
import org.json.JSONArray
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.FilterInputStream
import java.io.InputStreamReader
import java.nio.charset.CodingErrorAction
import java.util.UUID
import java.util.WeakHashMap
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/** Implemented by the canonical Activity without coupling this library to Wry. */
interface NativeActivityHost {
    val kiwixWindowId: Int
}

/** Rust NativeJob::drop sends jobId, also used by the export-copy receipt. */
internal inline fun cancelArticlePdfJob(
    readArgument: (String) -> String,
    cancelRenderer: (String) -> Unit,
    exportJobId: String?,
    cancelExport: () -> Unit,
) {
    val jobId = readArgument("jobId")
    require(jobId.isNotEmpty()) { "Missing PDF job" }
    cancelRenderer(jobId)
    if (exportJobId == jobId) cancelExport()
}

/** API 37 runtime permission; kept as text because this module compiles against API 36. */
internal const val LOCAL_NETWORK_PERMISSION = "android.permission.ACCESS_LOCAL_NETWORK"
internal const val LOCAL_NETWORK_API = 37

/** Local-network access is implicit through INTERNET below Android 17 (API 37). */
internal fun localNetworkRequired(sdk: Int): Boolean = sdk >= LOCAL_NETWORK_API
internal fun localNetworkAdmitted(sdk: Int, granted: Boolean): Boolean = !localNetworkRequired(sdk) || granted
/** A check-only call never launches the system sheet. */
internal fun localNetworkShouldRequest(sdk: Int, granted: Boolean, prompt: Boolean): Boolean =
    localNetworkRequired(sdk) && !granted && prompt

@TauriPlugin(permissions = [
    Permission(alias = "notifications", strings = [Manifest.permission.POST_NOTIFICATIONS]),
    // Android 17 gates LAN discovery and transfer behind this runtime permission.
    // It shares the NEARBY_DEVICES group with the Bluetooth and Wi-Fi aliases below.
    Permission(alias = "localNetwork", strings = [LOCAL_NETWORK_PERMISSION]),
    // LocalOnlyHotspot needs nearby Wi-Fi on 33+ and fine location before it.
    // Nothing else is requested; unrelated plugin commands never touch these.
    Permission(alias = "hotspotWifi", strings = [Manifest.permission.NEARBY_WIFI_DEVICES]),
    Permission(alias = "hotspotLocation", strings = [Manifest.permission.ACCESS_FINE_LOCATION]),
    Permission(alias = "nearbyBluetoothCentral", strings = [Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_CONNECT]),
    Permission(alias = "nearbyBluetoothPeripheral", strings = [Manifest.permission.BLUETOOTH_CONNECT, Manifest.permission.BLUETOOTH_ADVERTISE]),
    Permission(alias = "nearbyBluetoothLocation", strings = [Manifest.permission.ACCESS_FINE_LOCATION]),
    // Exact WifiInfo SSID proof is location-sensitive, including on Android 13+.
    Permission(alias = "nearbyJoinWifi", strings = [Manifest.permission.NEARBY_WIFI_DEVICES, Manifest.permission.ACCESS_COARSE_LOCATION, Manifest.permission.ACCESS_FINE_LOCATION]),
    Permission(alias = "nearbyJoinLocation", strings = [Manifest.permission.ACCESS_COARSE_LOCATION, Manifest.permission.ACCESS_FINE_LOCATION]),
    // Device-wide ZIM discovery on Android 12L and earlier, where READ_EXTERNAL_STORAGE
    // is the all-files read grant MediaStore needs. Android 13+ has no equivalent;
    // those releases use persisted SAF tree grants instead and never request this.
    Permission(alias = "deviceFiles", strings = [Manifest.permission.READ_EXTERNAL_STORAGE]),
    // Reader geolocation for an archive the user already allowed in Kiwix's own
    // dialog (ReaderLocation). Android 12+ ignores FINE requested alone, so the
    // precise alias carries both and the user may still keep approximate.
    Permission(alias = ReaderLocation.PRECISE_ALIAS, strings = [Manifest.permission.ACCESS_COARSE_LOCATION, Manifest.permission.ACCESS_FINE_LOCATION]),
    Permission(alias = ReaderLocation.APPROXIMATE_ALIAS, strings = [Manifest.permission.ACCESS_COARSE_LOCATION]),
])
class KiwixMobilePlugin(initialActivity: Activity) : Plugin(initialActivity) {
    private var activityReference = WeakReference(initialActivity)
    private val activity: Activity
        get() = activityReference.get()?.takeUnless { it.isDestroyed }
            ?: throw NativeDocumentError("ACTIVITY_DESTROYED")
    @Volatile private var activityGeneration = generationOf(initialActivity)
    private var uiAdmission = true
    private val applicationContext = initialActivity.applicationContext
    private val main = Handler(Looper.getMainLooper())
    /** Short control commands, durable intake and notification admission. */
    private val worker = Executors.newSingleThreadExecutor()
    /** Long file scans and cache maintenance; never head-of-line blocks control work. */
    private val scanWorker = Executors.newSingleThreadExecutor()
    /** One long copy/export at a time; never head-of-line blocks control work. */
    private val copyWorker = Executors.newSingleThreadExecutor()
    private val windowState = ActivityWindowState()
    private var destroyed = false
    private var operation: Operation? = null
    private var completionNotificationInvoke: Invoke? = null
    /** In-flight external recognizer activity; never a SpeechRecognizer or microphone session. */
    private var speechInvoke: Invoke? = null
    private var hotspotInvoke: Invoke? = null
    private var hotspotOwner: LocalHotspot? = null
    /** Starts awaiting a permission/OEM hotspot dialog (0 or 1). */
    private var hotspotSystemUi = 0
    private fun localHotspot(): LocalHotspot = hotspotOwner ?: LocalHotspot(
        applicationContext, main,
        { trigger("hotspot-stopped", JSObject()) },
        { active -> hotspotSystemUi = 0; networkPolicy.setHotspotActive(active) },
    ).also { hotspotOwner = it }
    private var webView = WeakReference<WebView>(null)
    private var foregroundInvoke: Invoke? = null
    private var foregroundTimeout: Runnable? = null
    /** This view owns only the aggregate native transfer assertion, never another window flag. */
    private var transferScreen: View? = null
    /** The shell's theme setting ("system", "light" or "dark"); null until it reports one. */
    private var themeSetting: String? = null
    private var incomingFailure: String? = null
    /** User-visible notice for an intake batch that could not survive a cold start. */
    private var incomingNotice: String? = null
    /** Admitted transient batches keyed by requestId, in admission order. */
    private val transientBatches = linkedMapOf<String, TransientBatch>()
    /** urisKey -> requestId, so redelivery dedupe is O(1). */
    private val transientBatchUris = hashMapOf<String, String>()
    /** Marker writes in flight; counted against the transient batch cap. */
    private var pendingTransientAdmissions = 0
    /** A marker write failed and should be reconciled by a later retry. */
    private var transientMarkerDirty = false
    /** Durable intake writes handed to the worker but not yet fsynced. */
    private val queuedIntakeBatches = mutableSetOf<String>()
    /** Bounded main-thread cache of already-durable batch keys for dedupe. */
    private val durableIntakeKeys = mutableSetOf<String>()
    /** Offered native intent flags are not themselves a persisted permission. */
    private val incomingGrantFlags = mutableMapOf<Uri, Int>()
    /** Reference count for transient batches sharing the same URI. */
    private val incomingGrantRefs = mutableMapOf<Uri, Int>()
    private class TransientBatch(val json: JSONObject, val urisKey: String, val uris: List<Uri>)
    private class ParsedDocuments(
        val uris: List<Uri>,
        val flags: Int,
        val handled: Boolean = false,
        val malformed: Boolean = false,
    )
    private var transientIntakeChecked = false
    private val externalDocuments by lazy { ExternalDocuments(applicationContext) }
    private val deviceScan by lazy { DeviceZimScan(applicationContext, externalDocuments) }
    /** Permission wait or in-flight scan; guarded on the main thread. */
    private var deviceScanInvoke: Invoke? = null
    private var scanning = false
    private var scanCancellation: CancellationSignal? = null
    private class ScannedOpen(val invoke: Invoke, val cancellation: CancellationSignal)
    private val scannedOpens = mutableSetOf<ScannedOpen>()
    private val networkPolicy by lazy {
        val reference = WeakReference(this)
        MobileNetworkPolicy(applicationContext) { state -> reference.get()?.trigger("network-policy-changed", state) }
    }
    private val hardwareKeys = HardwareShortcutKeys(
        enabled = { windowState.interactive && !destroyed && hasListener("hardware-shortcut") },
        dispatch = { action -> trigger("hardware-shortcut", JSObject().put("action", action)) }
    )
    private val speech = WeakReference(this).let { owner ->
        NativeSpeech(
            applicationContext,
            foreground = { owner.get()?.let { it.windowState.interactive && !it.destroyed && it.uiAdmission &&
                it.activityReference.get()?.let { activity -> !activity.isFinishing && !activity.isDestroyed } == true } == true },
            emit = { state -> owner.get()?.trigger("speech-state", state) }
        ).also {
            // Lock-screen and notification transport re-enters Rust, which owns
            // admission and forwards the action to the same engine commands.
            SpeechPlayback.dispatch = { id, action ->
                owner.get()?.trigger("speech-action", JSObject().put("action", action).put("sessionId", id))
            }
        }
    }
    private val nearbyBluetooth = WeakReference(this).let { reference ->
        NearbyBluetooth(applicationContext, main) { event ->
            reference.get()?.let { plugin ->
                if (!plugin.destroyed && plugin.hasListener("nearby-bluetooth")) {
                    plugin.trigger("nearby-bluetooth", event); true
                } else false
            } ?: false
        }
    }
    private val nearbyJoin = WeakReference(this).let { reference ->
        NearbyJoin(applicationContext, main) { event ->
            reference.get()?.let { plugin ->
                if (!plugin.destroyed && plugin.hasListener("nearby-join")) plugin.trigger("nearby-join", event)
            }
        }
    }
    private val nearbyRoutes = NearbyRoutes(applicationContext, main)
    /** Reader geolocation requests; guarded on the main thread. */
    private val readerLocation = ReaderLocation(applicationContext, main)
    private class NearbyPermissionTicket(val invoke: Invoke, val bluetooth: NearbyBluetooth.Owner? = null,
        val join: NearbyJoin.Attempt? = null)
    private var nearbyPermission: NearbyPermissionTicket? = null
    /** In-flight Android 17 local-network permission sheet. */
    private var localNetworkInvoke: Invoke? = null
    private val applicationResources by lazy {
        ApplicationResources(applicationContext, worker, externalDocuments, speech, networkPolicy, jsonMapper(), WeakReference(this),
            nearbyBluetooth, nearbyJoin, nearbyRoutes)
    }
    @Keep private external fun installServiceEndpoint(endpoint: ApplicationResources)
    @Command fun installApplicationResources(invoke: Invoke) {
        try { installServiceEndpoint(applicationResources); invoke.resolve(JSObject()) }
        catch (error: Exception) { reject(invoke, error) }
    }

    /** Captured source identity, never a query of the currently bound Activity. */
    @Keep
    private class ActivitySource(private val sourceGeneration: String) {
        @JavascriptInterface fun generation(): String = sourceGeneration
    }

    companion object {
        private const val ACTIVITY_SOURCE_INTERFACE = "__KIWIX_ANDROID_ACTIVITY_SOURCE__"
        private const val INTAKE_PREFERENCES = "kiwix-mobile-intake"
        private const val INTAKE_BATCHES = "batches"
        private const val INTAKE_TRANSIENT_PENDING = "transient-pending"
        private const val INTAKE_REOPEN_MESSAGE = "Re-open the file to import it."
        private const val INTAKE_HANDLED_EXTRA = "org.kiwix.mobile.INTAKE_HANDLED"
        private const val MAX_TRANSIENT_INTAKE_BATCHES = 32
        private const val MAX_INCOMING_GRANT_FLAGS = 64
        private const val ACTIVITY_RECREATED = "ACTIVITY_RECREATED"
        private const val SYSTEM_TEXT_CATEGORY = "android"
        private var retained = WeakReference<KiwixMobilePlugin>(null)
        private var currentActivity = WeakReference<Activity>(null)
        private val generations = WeakHashMap<Activity, Long>()
        private val retiredWebViews = WeakHashMap<WebView, Boolean>()
        private var nextGeneration = 0L
        /** Activities whose launch intent an earlier incarnation already acted on. */
        private val consumedLaunches = WeakHashMap<Activity, Boolean>()
        /** The newest Activity's launch intent is still consumable and no plugin was offered it. */
        private var launchUnoffered = false

        private fun generationOf(activity: Activity): Long =
            generations.getOrPut(activity) { ++nextGeneration }

        /**
         * Called before native onCreate, including when no plugin exists yet.
         * [restored] is true when Android supplied saved instance state.
         */
        @JvmStatic fun activityCreated(activity: Activity, restored: Boolean) {
            check(Looper.myLooper() == Looper.getMainLooper())
            val generation = generationOf(activity)
            val flags = try { activity.intent?.flags ?: 0 } catch (_: Exception) { 0 }
            val consumable = launchIntentConsumable(restored, flags, launchUnoffered)
            if (!consumable) consumedLaunches[activity] = true
            launchUnoffered = consumable
            retained.get()?.bindActivity(activity, generation)
            currentActivity = WeakReference(activity)
            AppShortcuts.install(activity)
        }

        /** PluginManager.load is intentionally once-only; every new WebView still binds here. */
        @JvmStatic fun webViewCreated(activity: Activity, view: WebView) {
            check(Looper.myLooper() == Looper.getMainLooper())
            if (currentActivity.get() !== activity || activity.isDestroyed) return
            val generation = generationOf(activity)
            // Wry invokes this creation hook before loading the document. A
            // configuration-created page gets new source authority even though
            // its cached Wry window attributes retain the logical window UUID.
            view.addJavascriptInterface(ActivitySource(generation.toString()), ACTIVITY_SOURCE_INTERFACE)
            retained.get()?.let { plugin ->
                plugin.bindActivity(activity, generation)
                plugin.bindWebView(view)
            }
        }

        /** Run after Wry's onDestroy has emitted its native ownership callbacks. */
        @JvmStatic fun activityDestroyed(activity: Activity, view: WebView?) {
            retained.get()?.let { plugin ->
                if (plugin.activityReference.get() === activity) {
                    plugin.retireWindowUi(replaced = !activity.isFinishing)
                    plugin.destroyed = true
                    plugin.windowState.stopped()
                    plugin.activityReference.clear()
                }
            }
            if (currentActivity.get() === activity) currentActivity.clear()
            view?.let { destroyWebView(it) }
        }

        private fun destroyWebView(view: WebView) {
            if (retiredWebViews.put(view, true) != null) return
            view.setOnKeyListener(null)
            view.removeJavascriptInterface(ACTIVITY_SOURCE_INTERFACE)
            view.stopLoading()
            view.onPause()
            // Revoke camera, media and geolocation authority, including pending
            // Chromium permission requests, rather than retaining a paused page.
            view.settings.javaScriptEnabled = false
            view.settings.setGeolocationEnabled(false)
            view.settings.mediaPlaybackRequiresUserGesture = true
            view.webChromeClient = null
            view.webViewClient = android.webkit.WebViewClient()
            (view.parent as? ViewGroup)?.removeView(view)
            view.removeAllViews()
            view.destroy()
        }
    }

    init {
        retained = WeakReference(this)
        if (currentActivity.get() == null) currentActivity = WeakReference(initialActivity)
    }

    private class Operation(val invoke: Invoke, val kind: String, val args: JSObject, val generation: Long) {
        val cancellation = CancellationSignal()
        val finished = AtomicBoolean(false)
        @Volatile var copying = false
        var dialog: AlertDialog? = null
        @Volatile var staging: OwnedDocuments? = null
        @Volatile var exportUri: Uri? = null
        @Volatile var externalReference: String? = null
        val externalReferences = mutableListOf<String>()
        val exportScratch = mutableListOf<ExportScratch>()
        var lastProgress = 0L
        /** Retired because its Activity was replaced, not because the person cancelled. */
        @Volatile var replaced = false
        val chooserToken = UUID.randomUUID().toString()
        var chooserReceipt: PendingIntent? = null
        /** Share copies go through the system share sheet, not a viewer. */
        var share = false
    }

    override fun load(webView: WebView) {
        val owner = activityReference.get() ?: return
        if (currentActivity.get() !== owner) return
        CrashIntake.install(applicationContext)
        bindWebView(webView)
        val lifecycle = (owner as? LifecycleOwner)?.lifecycle?.currentState
        windowState.restore(
            lifecycle?.isAtLeast(Lifecycle.State.STARTED) == true,
            lifecycle?.isAtLeast(Lifecycle.State.RESUMED) == true,
        )
        TransferEvents.attach(this)
        nearbyRoutes.install()
        // Durable intent intake is enqueued before the best-effort export-cache
        // sweep so a cold-start file open is not delayed behind deletions.
        val launch = launchIntent(owner)
        receiveCompletionNotification(launch)
        receiveDocuments(launch)
        receiveEntries(launch)
        sweepExportCache()
    }

    /**
     * The intent that started [owner], or null when an earlier incarnation
     * already acted on it. Android hands a restored Activity, and a task
     * reopened from Recents, the original launch intent again; the in-process
     * handled extras do not survive that, so it must not be offered twice.
     * Intents delivered through onNewIntent never pass through here.
     */
    private fun launchIntent(owner: Activity): Intent? {
        if (consumedLaunches[owner] == true) return null
        launchUnoffered = false
        return owner.intent
    }

    /** Bound the lifetime of export/share scratch copies: a receiving app may
     * still read a file after the chooser closes, so deletion is deferred to a
     * later launch instead of racing the grant. */
    private fun sweepExportCache() {
        // Best-effort deletion is file IO; keep it off the interactive/control
        // worker so durable intake and notification admission stay responsive.
        scanWorker.execute {
            val base = File(applicationContext.cacheDir, "kiwix-mobile-exports")
            val cutoff = System.currentTimeMillis() - 24L * 60 * 60 * 1000
            base.listFiles()?.forEach { entry ->
                try { if (entry.lastModified() < cutoff) entry.deleteRecursively() } catch (_: Exception) {}
            }
        }
    }

    private fun bindActivity(activity: Activity, generation: Long) {
        if (activityReference.get() === activity) return
        // Close and cancel the old generation before publishing the replacement.
        retireWindowUi(replaced = true)
        webView.get()?.let { destroyWebView(it) }
        webView.clear()
        activityReference = WeakReference(activity)
        activityGeneration = generation
        destroyed = false
        windowState.stopped()
        // Rust opens admission only after the old incarnation's accepted work drains.
        uiAdmission = false
    }

    override fun onResume(activity: AppCompatActivity) {
        if (currentActivity.get() !== activity) return
        bindActivity(activity, generationOf(activity))
        destroyed = false
        windowState.resumed()
        TransferEvents.attach(this)
        val launch = launchIntent(activity)
        receiveCompletionNotification(launch)
        receiveDocuments(launch)
        receiveEntries(launch)
        webView.get()?.onResume()
        readerLocation.resumed()
        // The paused root may have missed transport events while the WebView
        // was frozen. Re-publish the engine's current state after resume.
        speech.reemit()
        for (event in listOf("transfers-pause", "transfers-timeout", "transfers-error"))
            deliverTransferEvent(event)
        notifyIncomingDocuments()
        hotspotSystemUi = 0
        trigger("mobile-resumed", JSObject())
    }
    override fun onPause(activity: AppCompatActivity) {
        if (activity !== activityReference.get()) return
        windowState.paused()
        // Native read-aloud keeps playing in the background: the media service
        // holds foreground admission and Rust decides whether the source may
        // survive. Activity destruction still retires speech.
        hardwareKeys.reset()
        // Public WebView lifecycle API is best-effort; trusted root must stop tracks/clearWatch.
        webView.get()?.onPause()
        // onPause is loss of interactivity, not proof that the Activity left
        // the screen. A picker or permission sheet can return without onStop;
        // starting retirement here closes the root's finite flush admission
        // while its accepted native operation still owns the continuation.
        trigger("mobile-inactive", JSObject().put("reason", "activity-paused"))
    }
    override fun onStop(activity: AppCompatActivity) {
        if (activity !== activityReference.get()) return
        windowState.stopped()
        retireNearby()
        // onPause cannot prove the Activity left the screen: a permission or
        // OEM hotspot dialog pauses it while its window stays visible, and
        // those flags may already be clear when the real background arrives.
        // Report suspension unconditionally here; Rust deduplicates repeated
        // reports for the same lifecycle generation.
        val (event, reason) = stoppedActivityLifecycle(
            completionNotificationInvoke != null)
        android.util.Log.i("KiwixHotspot", "onStop hotspotSystemUi=$hotspotSystemUi")
        hotspotSystemUi = 0
        trigger(event, JSObject().put("reason", reason))
        if (!KiwixTransferService.running) releaseDiscovery()
    }
    override fun onNewIntent(intent: Intent) {
        if (!destroyed && activityReference.get() === currentActivity.get()) {
            receiveCompletionNotification(intent)
            receiveDocuments(intent)
            receiveEntries(intent)
        }
    }
    override fun onDestroy(activity: AppCompatActivity) {
        if (activity !== activityReference.get()) return
        // An Activity that is not finishing is destroyed only to be replaced:
        // a configuration change, or the system reclaiming it in the background.
        retireWindowUi(replaced = !activity.isFinishing)
        destroyed = true
        windowState.stopped()
        activityReference.clear()
    }

    internal fun activityState(): JSObject {
        val owner = activityReference.get()?.takeUnless { it.isDestroyed || destroyed }
        return JSObject().put("id", if (owner == null) JSONObject.NULL else activityGeneration)
            .put("windowId", (owner as? NativeActivityHost)?.kiwixWindowId ?: JSONObject.NULL)
            .put("visible", owner != null && windowState.interactive && !owner.isFinishing)
            .put("foreground", owner != null && windowState.foreground && !owner.isFinishing)
    }

    internal fun finishActivity(generation: Long) {
        if (generation != activityGeneration) return
        activityReference.get()?.takeUnless { it.isDestroyed }?.finish()
    }

    internal fun retireWindow(generation: Long) {
        if (generation == activityGeneration) retireWindowUi()
    }

    internal fun openWindow(generation: Long) {
        check(generation == activityGeneration) { "STALE_ACTIVITY_GENERATION" }
        val owner = activityReference.get()
        check(owner != null && !owner.isDestroyed && !owner.isFinishing && windowState.interactive) { "VISIBLE_ACTIVITY_REQUIRED" }
        destroyed = false
        uiAdmission = true
        TransferEvents.attach(this)
        webView.get()?.setOnKeyListener(hardwareKeys)
    }

    /**
     * [replaced] marks retirement caused by the Activity being replaced rather
     * than closed. The person did not cancel a picker or copy that this ends,
     * so it settles as the ACTIVITY_RECREATED failure, never as a cancellation.
     */
    internal fun retireWindowUi(replaced: Boolean = false) {
        uiAdmission = false
        releaseTransferScreen()
        retireNearby()
        speech.shutdown()?.let { android.util.Log.e("KiwixMobile", "Speech retirement failed: $it") }
        webView.get()?.setOnKeyListener(null)
        hardwareKeys.reset()
        TransferEvents.detach(this)
        if (!KiwixTransferService.running) releaseDiscovery()
        hotspotInvoke?.reject("ACTIVITY_DESTROYED", "ACTIVITY_DESTROYED")
        hotspotInvoke = null
        localNetworkInvoke?.reject("LOCAL_NETWORK_PERMISSION_CANCELLED", "LOCAL_NETWORK_PERMISSION_CANCELLED")
        localNetworkInvoke = null
        hotspotSystemUi = 0
        if (hotspotOwner?.release() == true) trigger("hotspot-stopped", JSObject())
        readerLocation.retire()
        operation?.let { op ->
            if (replaced) op.replaced = true
            op.cancellation.cancel()
            op.dialog?.dismiss()
            op.dialog = null
            when (retirementOutcome(op.copying, op.kind == "open" && op.exportScratch.isNotEmpty(), op.replaced)) {
                // The worker unwinds, removes what it staged and settles the operation itself.
                RetirementOutcome.AWAIT_WORKER -> Unit
                // Teardown is not proof of chooser cancellation: a viewer may already hold its grant.
                // Retain/report the copy rather than deleting a possibly selected viewer's source.
                RetirementOutcome.FAIL_DESTROYED -> fail(op, NativeDocumentError("ACTIVITY_DESTROYED"))
                RetirementOutcome.FAIL_RECREATED -> fail(op, NativeDocumentError(ACTIVITY_RECREATED))
                RetirementOutcome.CANCELLED -> complete(op, cancelledResult(op.kind))
            }
        }
        deviceScanInvoke?.reject("ACTIVITY_DESTROYED", "ACTIVITY_DESTROYED")
        deviceScanInvoke = null
        speechInvoke?.reject("ACTIVITY_DESTROYED", "ACTIVITY_DESTROYED")
        speechInvoke = null
        scanCancellation?.cancel()
        scanCancellation = null
        scanning = false
        scannedOpens.toList().forEach { open ->
            open.cancellation.cancel()
            open.invoke.reject("ACTIVITY_DESTROYED", "ACTIVITY_DESTROYED")
        }
        scannedOpens.clear()
        completionNotificationInvoke?.reject("ACTIVITY_DESTROYED", "ACTIVITY_DESTROYED")
        completionNotificationInvoke = null
        foregroundInvoke?.let {
            KiwixTransferService.stopAfterFailure(applicationContext, "ACTIVITY_DESTROYED")
            finishForegroundStart(it, "ACTIVITY_DESTROYED")
        }
        // PluginManager retains this instance: keep both workers available for the replacement Activity.
    }
    private fun requireNearbyVisible() {
        val owner = activityReference.get()
        nearbyRequire(owner != null && owner === currentActivity.get() && uiAdmission && !destroyed &&
            windowState.foreground && !owner.isDestroyed && !owner.isFinishing, "NEARBY_VISIBLE_ACTIVITY_REQUIRED")
    }

    /** A permission sheet can return before onResume, but never after real stop/replacement. */
    private fun nearbyForegroundLease(): () -> Boolean {
        val source = WeakReference(activityReference.get())
        val generation = activityGeneration
        val reference = WeakReference(this)
        return {
            reference.get()?.let { plugin ->
                val owner = source.get()
                owner != null && plugin.activityReference.get() === owner && currentActivity.get() === owner &&
                    plugin.activityGeneration == generation && !plugin.destroyed && plugin.uiAdmission &&
                    plugin.windowState.foreground && !owner.isDestroyed && !owner.isFinishing
            } == true
        }
    }

    private fun retireNearby() {
        nearbyPermission = null
        try { nearbyBluetooth.retireAll() } catch (_: Exception) {
            android.util.Log.e("KiwixNearby", "NEARBY_BLUETOOTH_CLEANUP_FAILED")
        }
        try { nearbyJoin.retireAll() } catch (_: Exception) {
            android.util.Log.e("KiwixNearby", "NEARBY_JOIN_CLEANUP_FAILED")
        }
    }

    private fun nearbySimple(invoke: Invoke, action: () -> JSObject) {
        val reply = NearbyReply(invoke)
        main.post {
            try { reply.resolve(action()) } catch (error: Exception) { reply.reject(nearbyFailure(error)) }
        }
    }

    @Command fun nearbyBluetoothStart(invoke: Invoke) {
        main.post {
            var reserved: NearbyBluetooth.Owner? = null
            try {
                requireNearbyVisible()
                nearbyRequire(hasListener("nearby-bluetooth"), "NEARBY_BLUETOOTH_LISTENER_REQUIRED")
                val owner = nearbyBluetooth.reserve(invoke, nearbyForegroundLease())
                reserved = owner
                val missing = nearbyBluetooth.permissions(owner.role).any {
                    ContextCompat.checkSelfPermission(applicationContext, it) != PackageManager.PERMISSION_GRANTED
                }
                if (missing) {
                    nearbyRequire(nearbyPermission == null, "NEARBY_PERMISSION_BUSY")
                    nearbyPermission = NearbyPermissionTicket(invoke, bluetooth = owner)
                    val alias = if (Build.VERSION.SDK_INT >= 31) {
                        if (owner.role == "central") "nearbyBluetoothCentral" else "nearbyBluetoothPeripheral"
                    } else "nearbyBluetoothLocation"
                    requestPermissionForAlias(alias, invoke, "nearbyPermissionResult")
                } else nearbyBluetooth.begin(owner)
            } catch (error: Exception) {
                if (nearbyPermission?.invoke === invoke) nearbyPermission = null
                reserved?.let { nearbyBluetooth.fail(it, nearbyFailure(error)) }
                    ?: NearbyReply(invoke).reject(nearbyFailure(error))
            }
        }
    }

    @PermissionCallback fun nearbyPermissionResult(invoke: Invoke) {
        main.post {
            val ticket = nearbyPermission?.takeIf { it.invoke === invoke } ?: return@post
            nearbyPermission = null
            ticket.bluetooth?.let { if (nearbyBluetooth.owns(it)) nearbyBluetooth.begin(it) }
            ticket.join?.let { if (nearbyJoin.owns(it)) nearbyJoin.begin(it) }
        }
    }

    /**
     * Rust calls this before every LocalSwarm LAN command. Android 17 blocks all
     * LAN sockets until ACCESS_LOCAL_NETWORK is granted; on older releases the
     * permission is implicit and no prompt is shown. A previous Bluetooth or
     * nearby-Wi-Fi grant is in the same NEARBY_DEVICES group, so the platform
     * reports the permission as granted and this resolves without a sheet.
     */
    @Command fun localNetworkPermission(invoke: Invoke) {
        main.post {
            try {
                val required = localNetworkRequired(Build.VERSION.SDK_INT)
                val granted = ContextCompat.checkSelfPermission(applicationContext, LOCAL_NETWORK_PERMISSION) ==
                    PackageManager.PERMISSION_GRANTED
                // Automatic status polls pass prompt=false so a denial never
                // turns a two-second refresh into a repeating system sheet.
                val prompt = invoke.getArgs().optBoolean("prompt", true)
                if (!localNetworkShouldRequest(Build.VERSION.SDK_INT, granted, prompt)) {
                    invoke.resolve(JSObject()
                        .put("granted", localNetworkAdmitted(Build.VERSION.SDK_INT, granted))
                        .put("required", required))
                    return@post
                }
                if (localNetworkInvoke != null) {
                    invoke.reject("LOCAL_NETWORK_PERMISSION_BUSY", "LOCAL_NETWORK_PERMISSION_BUSY")
                    return@post
                }
                requireVisible()
                localNetworkInvoke = invoke
                requestPermissionForAlias("localNetwork", invoke, "localNetworkPermissionResult")
            } catch (error: Exception) {
                if (localNetworkInvoke === invoke) localNetworkInvoke = null
                reject(invoke, error)
            }
        }
    }

    @PermissionCallback fun localNetworkPermissionResult(invoke: Invoke) {
        main.post {
            if (localNetworkInvoke !== invoke) return@post
            localNetworkInvoke = null
            val granted = ContextCompat.checkSelfPermission(applicationContext, LOCAL_NETWORK_PERMISSION) ==
                PackageManager.PERMISSION_GRANTED
            invoke.resolve(JSObject()
                .put("granted", localNetworkAdmitted(Build.VERSION.SDK_INT, granted))
                .put("required", localNetworkRequired(Build.VERSION.SDK_INT)))
        }
    }

    /**
     * Rust-only (location_android.rs): one fresh fix for an archive the user
     * already allowed in Kiwix. Args {token, highAccuracy, timeoutMs}; resolves
     * {latitude, longitude, accuracy, timestamp, altitude?, altitudeAccuracy?,
     * heading?, speed?} or rejects with a ReaderLocationCode.
     */
    @Command fun startLocation(invoke: Invoke) {
        main.post {
            readerLocation.start(invoke) { alias ->
                requireVisible()
                requestPermissionForAlias(alias, invoke, "readerLocationPermissionResult")
            }
        }
    }

    @PermissionCallback fun readerLocationPermissionResult(invoke: Invoke) {
        main.post { readerLocation.permissionSettled() }
    }

    /** Idempotent; a cancel that overtakes its start is remembered. */
    @Command fun cancelLocation(invoke: Invoke) {
        main.post {
            try {
                readerLocation.cancel(invoke.getArgs().getLong("token"))
                invoke.resolve(JSObject())
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @Command fun nearbyBluetoothConnect(invoke: Invoke) { main.post { nearbyBluetooth.connect(invoke) } }
    @Command fun nearbyBluetoothSend(invoke: Invoke) { main.post { nearbyBluetooth.send(invoke) } }
    @Command fun nearbyBluetoothDisconnect(invoke: Invoke) = nearbySimple(invoke) { nearbyBluetooth.disconnect(invoke.getArgs()) }
    @Command fun nearbyBluetoothStop(invoke: Invoke) = nearbySimple(invoke) { nearbyBluetooth.stop(invoke.getArgs()) }
    @Command fun nearbyBluetoothScan(invoke: Invoke) = nearbySimple(invoke) { nearbyBluetooth.scan(invoke.getArgs()) }

    @Command fun nearbyJoin(invoke: Invoke) {
        main.post {
            var reserved: NearbyJoin.Attempt? = null
            try {
                requireNearbyVisible()
                nearbyRequire(hasListener("nearby-join"), "NEARBY_JOIN_LISTENER_REQUIRED")
                nearbyRequire(hotspotInvoke == null && hotspotSystemUi == 0 && hotspotOwner?.snapshot() == null, "NEARBY_HOTSPOT_BUSY")
                val attempt = nearbyJoin.reserve(invoke, nearbyForegroundLease())
                reserved = attempt
                if (nearbyJoin.permissions().any { ContextCompat.checkSelfPermission(applicationContext, it) != PackageManager.PERMISSION_GRANTED }) {
                    nearbyRequire(nearbyPermission == null, "NEARBY_PERMISSION_BUSY")
                    nearbyPermission = NearbyPermissionTicket(invoke, join = attempt)
                    requestPermissionForAlias(if (Build.VERSION.SDK_INT >= 33) "nearbyJoinWifi" else "nearbyJoinLocation",
                        invoke, "nearbyPermissionResult")
                } else nearbyJoin.begin(attempt)
            } catch (error: Exception) {
                if (nearbyPermission?.invoke === invoke) nearbyPermission = null
                reserved?.let { nearbyJoin.fail(it, nearbyFailure(error)) }
                    ?: NearbyReply(invoke).reject(nearbyFailure(error))
            }
        }
    }

    @Command fun nearbyJoinState(invoke: Invoke) = nearbySimple(invoke) { nearbyJoin.state(invoke.getArgs()) }
    @Command fun nearbyJoinBindSocket(invoke: Invoke) = nearbySimple(invoke) { nearbyJoin.bind(invoke.getArgs()) }
    @Command fun nearbyJoinRelease(invoke: Invoke) = nearbySimple(invoke) { nearbyJoin.release(invoke.getArgs()) }
    @Command fun nearbyJoinCancel(invoke: Invoke) = nearbySimple(invoke) { nearbyJoin.cancel(invoke.getArgs()) }
    @Command fun nearbyRouteBindSocket(invoke: Invoke) = nearbySimple(invoke) { nearbyRoutes.bind(invoke.getArgs()) }
    @Command fun nearbyRouteState(invoke: Invoke) = nearbySimple(invoke) { nearbyRoutes.state(invoke.getArgs()) }


    private fun ownsUi(op: Operation): Boolean =
        op.generation == activityGeneration && uiAdmission && !destroyed && operation === op

    private fun speechCommand(invoke: Invoke, command: () -> Unit) {
        main.post {
            try { requireVisible(); command() }
            catch (error: Exception) { reject(invoke, error) }
        }
    }
    private fun bindWebView(view: WebView) {
        val previous = webView.get()
        if (activityReference.get() !== currentActivity.get() || destroyed) return
        if (previous !== view) {
            previous?.let { destroyWebView(it) }
            hardwareKeys.reset()
            webView = WeakReference(view)
        }
        view.setOnKeyListener(hardwareKeys)
    }

    // Internal RustMobile.call transport only; the plugin exports no JavaScript commands.
    @Command fun speechVoices(invoke: Invoke) = speechCommand(invoke) { speech.voices(invoke) }
    @Command fun speechPrepare(invoke: Invoke) = speechCommand(invoke) { speech.prepare(invoke) }
    @Command fun speechStart(invoke: Invoke) = speechCommand(invoke) { speech.start(invoke) }

    @Command fun networkState(invoke: Invoke) {
        try { invoke.resolve(networkPolicy.snapshot()) }
        catch (error: Exception) { reject(invoke, error) }
    }

    /** Rust-only socket route admission; no descriptor is exported to JavaScript. */
    @Command fun bindLocalSocket(invoke: Invoke) {
        try {
            val route = networkPolicy.bindLocalSocket(invoke.getArgs().getInt("fd"))
            invoke.resolve(JSObject().put("route", route))
        } catch (error: Exception) {
            android.util.Log.e("KiwixRoute", "bindLocalSocket failed", error)
            invoke.reject(error.javaClass.simpleName + ": " + (error.message ?: "no message"), error.message ?: "LOCAL_SOCKET_BIND_FAILED")
        }
    }

    @Command fun isLocalPeer(invoke: Invoke) {
        try {
            val args = invoke.getArgs()
            val local = networkPolicy.isLocalPeer(args.getLong("route"), args.getString("address"))
            invoke.resolve(JSObject().put("local", local))
        } catch (error: Exception) { reject(invoke, error) }
    }

    private class HotspotError(val code: String, message: String) : Exception(message)

    private fun hotspotPermission(): String =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) Manifest.permission.NEARBY_WIFI_DEVICES
        else Manifest.permission.ACCESS_FINE_LOCATION

    private fun hotspotAlias(): String =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) "hotspotWifi" else "hotspotLocation"

    private fun rejectHotspot(invoke: Invoke, error: Exception) {
        val code = (error as? HotspotError)?.code ?: "HOTSPOT_FAILED"
        invoke.reject(error.message ?: code, code)
    }

    /** Android-only local-only hotspot; Rust owns the lifecycle and state. */
    @Command fun hotspotStart(invoke: Invoke) {
        main.post {
            try {
                android.util.Log.i("KiwixHotspot", "start requested sdk=${Build.VERSION.SDK_INT} visible=${windowState.interactive}")
                hotspotSystemUi = 1
                if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O)
                    throw HotspotError("HOTSPOT_UNSUPPORTED", "Android 8.0 or newer is required to create a Wi-Fi hotspot")
                requireVisible()
                nearbyRequire(!nearbyJoin.busy(), "NEARBY_JOIN_BUSY")
                if (ContextCompat.checkSelfPermission(applicationContext, hotspotPermission())
                        != PackageManager.PERMISSION_GRANTED) {
                    if (hotspotInvoke != null)
                        throw HotspotError("HOTSPOT_REQUEST_PENDING", "A hotspot permission request is already open")
                    hotspotInvoke = invoke
                    android.util.Log.i("KiwixHotspot", "requesting permission alias=${hotspotAlias()}")
                    requestPermissionForAlias(hotspotAlias(), invoke, "hotspotPermissionResult")
                    return@post
                }
                android.util.Log.i("KiwixHotspot", "permission already granted; starting")
                localHotspot().start(invoke)
            } catch (error: Exception) {
                android.util.Log.e("KiwixHotspot", "start rejected", error)
                hotspotSystemUi = 0
                if (hotspotInvoke === invoke) hotspotInvoke = null
                rejectHotspot(invoke, error)
            }
        }
    }

    @PermissionCallback fun hotspotPermissionResult(invoke: Invoke) {
        main.post {
            if (hotspotInvoke !== invoke) {
                android.util.Log.w("KiwixHotspot", "permission result for a superseded request")
                return@post
            }
            hotspotInvoke = null
            try {
                android.util.Log.i("KiwixHotspot", "permission result visible=${windowState.interactive}")
                hotspotSystemUi = 1
                requireVisible()
                if (ContextCompat.checkSelfPermission(applicationContext, hotspotPermission())
                        != PackageManager.PERMISSION_GRANTED)
                    throw HotspotError("HOTSPOT_PERMISSION_DENIED",
                        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU)
                            "Allow nearby Wi-Fi devices to create a hotspot"
                        else "Allow location access to create a hotspot")
                localHotspot().start(invoke)
            } catch (error: Exception) {
                android.util.Log.e("KiwixHotspot", "permission continuation rejected", error)
                hotspotSystemUi = 0
                rejectHotspot(invoke, error)
            }
        }
    }

    /** Idempotent release; safe after Activity retirement and during shutdown. */
    @Command fun hotspotStop(invoke: Invoke) {
        main.post {
            hotspotInvoke?.reject("HOTSPOT_START_CANCELLED", "The Wi-Fi hotspot was stopped before it finished starting")
            hotspotInvoke = null
            try { hotspotOwner?.stop(invoke) ?: invoke.resolve(JSObject()) }
            catch (error: Exception) { rejectHotspot(invoke, error) }
            finally { hotspotSystemUi = 0 }
        }
    }
    @Command fun hotspotQuicRoute(invoke: Invoke) {
        main.post {
            try {
                requireNearbyVisible()
                nearbyRequire(Build.VERSION.SDK_INT >= 26, "NEARBY_HOTSPOT_UNSUPPORTED")
                val owned = hotspotOwner ?: throw NearbyError("NEARBY_HOTSPOT_NOT_OWNED")
                owned.quicRoute(invoke)
            } catch (error: Exception) { NearbyReply(invoke).reject(nearbyFailure(error)) }
        }
    }
    @Command fun hotspotQuicState(invoke: Invoke) = nearbySimple(invoke) {
        nearbyRequire(Build.VERSION.SDK_INT >= 26, "NEARBY_HOTSPOT_UNSUPPORTED")
        hotspotOwner?.quicState(invoke.getArgs()) ?: JSObject().put("available", false)
    }

    /** Setup metadata only: no UI dispatch, caller paths, permissions, or Activity visibility dependency. */
    @Command fun storageRoot(invoke: Invoke) {
        try {
            invoke.resolve(JSObject().put("path", applicationContext.filesDir.canonicalPath)
                .put("cachePath", applicationContext.cacheDir.canonicalPath))
        } catch (error: Exception) { reject(invoke, error) }
    }

    @Command fun bundledVoicePack(invoke: Invoke) {
        try { invoke.resolve(BundledKokoro.location(applicationContext)) }
        catch (error: Exception) { reject(invoke, error) }
    }

    /** App-owned library locations, including removable app-specific folders. */
    @Command fun storageLocations(invoke: Invoke) {
        try { invoke.resolve(StorageLocations.enumerate(applicationContext)) }
        catch (error: Exception) { reject(invoke, error) }
    }

    @Command fun printArticlePdf(invoke: Invoke) {
        try { requireVisible(); NativeArticlePdf.start(activity, invoke) }
        catch (error: Exception) { reject(invoke, error) }
    }
    @Command fun cancelArticlePdf(invoke: Invoke) {
        main.post {
            try {
                val export = operation
                cancelArticlePdfJob(
                    { key -> invoke.getArgs().getString(key) },
                    NativeArticlePdf::cancelId,
                    export?.args?.optString("pdfJobId"),
                ) {
                    export?.let {
                        it.cancellation.cancel()
                        scheduleCancelledCleanup(it)
                    }
                }
                invoke.resolve(JSObject())
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @Command fun applyLocale(invoke: Invoke) {
        main.post {
            try {
                val args = invoke.getArgs()
                AppLocale.apply(applicationContext, args.getString("locale"), args.optBoolean("preservePlatform", false))
                invoke.resolve(AppLocale.state(applicationContext))
            }
            catch (error: Exception) { reject(invoke, error) }
        }
    }

    @Command fun crashCapturePolicy(invoke: Invoke) {
        try { CrashIntake.setEnabled(invoke.getArgs().getBoolean("enabled")); invoke.resolve(JSObject()) }
        catch (error: Exception) { reject(invoke, error) }
    }

    /**
     * The system font scale, in the shape the iOS command of the same name
     * returns. The WebView already multiplies its text by this factor, so the
     * shell uses it only to choose its large-text layout. A change recreates
     * the Activity (fontScale is not a handled configuration change), so the
     * shell reads it once per document and needs no change event.
     */
    @Command fun dynamicTypeState(invoke: Invoke) {
        try {
            invoke.resolve(JSObject().put("category", SYSTEM_TEXT_CATEGORY)
                .put("scale", applicationContext.resources.configuration.fontScale.toDouble()))
        } catch (error: Exception) { reject(invoke, error) }
    }

    /** Keep the strips behind the transparent system bars in the shell's theme. */
    @Command fun applyTheme(invoke: Invoke) {
        main.post {
            try {
                val setting = invoke.getArgs().getString("theme")
                themeSetting = setting
                activityReference.get()?.takeUnless { it.isDestroyed || destroyed }?.let { owner ->
                    WindowTheme.apply(owner, setting, owner.resources.configuration)
                }
                invoke.resolve(JSObject())
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    /** uiMode is handled without recreation, so a "system" theme is re-resolved here. */
    override fun onConfigurationChanged(newConfig: Configuration) {
        val setting = themeSetting ?: return
        val owner = activityReference.get()?.takeUnless { it.isDestroyed || destroyed } ?: return
        try { WindowTheme.apply(owner, setting, newConfig) }
        catch (error: Exception) { android.util.Log.e("KiwixMobile", "Window theme update failed", error) }
    }


    private fun requireVisible() {
        if (!uiAdmission || !windowState.interactive || destroyed || activity.isFinishing || activity.isDestroyed)
            throw NativeDocumentError("VISIBLE_ACTIVITY_REQUIRED")
    }

    private fun label(args: JSObject, key: String): String {
        val value = args.getJSONObject("labels").getString(key)
        if (value.isBlank()) throw NativeDocumentError("LOCALIZED_LABEL_REQUIRED")
        return value
    }

    private fun begin(invoke: Invoke, kind: String, makeIntent: (JSObject) -> Intent) {
        main.post {
            try {
                requireVisible()
                if (operation != null || completionNotificationInvoke != null) throw NativeDocumentError("NATIVE_OPERATION_BUSY")
                val args = invoke.getArgs()
                label(args, "title"); label(args, "cancel")
                if (kind == "archives" || kind == "directory") label(args, "copying")
                if (kind == "text") {
                    val limit = args.getLong("maxBytes")
                    if (limit < 1 || limit > Int.MAX_VALUE) throw NativeDocumentError("INVALID_TEXT_LIMIT")
                }
                if (kind == "export") checkedSource(args.getString("path"))
                val intent = makeIntent(args)
                val op = Operation(invoke, kind, args, activityGeneration)
                operation = op
                try { startActivityForResult(invoke, intent, "documentResult") }
                catch (error: Exception) { fail(op, error) }
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @Command fun pickArchiveCopies(invoke: Invoke) = begin(invoke, "archives") { args ->
        Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*")
            .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true).putExtra(Intent.EXTRA_TITLE, label(args, "title"))
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
    }

    @Command fun pickExternalArchive(invoke: Invoke) = begin(invoke, "external") { args ->
        val folder = args.getBoolean("folder")
        val persistent = args.getBoolean("persistent")
        val intent = if (folder) Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
            else Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*")
        intent.putExtra(Intent.EXTRA_TITLE, label(args, "title"))
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or
                (if (persistent) Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION else 0) or
                (if (folder) Intent.FLAG_GRANT_PREFIX_URI_PERMISSION else 0))
    }

    @Command fun pickExternalArchives(invoke: Invoke) = begin(invoke, "externalBatch") { args ->
        val folder = args.getBoolean("folder")
        val intent = if (folder) Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
            else Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*")
                .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
        intent.putExtra(Intent.EXTRA_TITLE, label(args, "title"))
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or
                (if (args.getBoolean("persistent")) Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION else 0) or
                (if (folder) Intent.FLAG_GRANT_PREFIX_URI_PERMISSION else 0))
    }

    @Command fun openIncomingArchive(invoke: Invoke) {
        main.post {
            try {
                requireVisible()
                if (operation != null || completionNotificationInvoke != null)
                    throw NativeDocumentError("NATIVE_OPERATION_BUSY")
                val args = invoke.getArgs()
                val uri = Uri.parse(args.getString("uri"))
                if (uri.scheme != "content") throw NativeDocumentError("DOCUMENT_URI_REQUIRED")
                val flags = incomingGrantFlags[uri] ?: 0
                val op = Operation(invoke, "external", args, activityGeneration)
                operation = op
                launchWorker(op) {
                    val selection = externalDocuments.selectIncoming(uri,
                        args.getBoolean("persistent"), flags, op.cancellation)
                    op.externalReference = selection.getString("reference")
                    JSObject().put("selection", selection).put("cancelled", false)
                }
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @Command fun pickLibraryDirectoryCopy(invoke: Invoke) = begin(invoke, "directory") { args ->
        Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).putExtra(Intent.EXTRA_TITLE, label(args, "title"))
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION or
                Intent.FLAG_GRANT_PREFIX_URI_PERMISSION)
    }

    @Command fun readTextDocument(invoke: Invoke) = begin(invoke, "text") { args ->
        Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*")
            .putExtra(Intent.EXTRA_MIME_TYPES, arrayOf("application/json", "application/xml", "text/xml", "text/plain"))
            .putExtra(Intent.EXTRA_TITLE, label(args, "title"))
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }

    /** Persist one SAF tree grant for later scans; nothing is copied. */
    @Command fun pickScanFolder(invoke: Invoke) = begin(invoke, "scanFolder") { args ->
        Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).putExtra(Intent.EXTRA_TITLE, label(args, "title"))
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION or
                Intent.FLAG_GRANT_PREFIX_URI_PERMISSION)
    }

    /** Enumerate MediaStore rows and persisted SAF trees. The permission sheet,
     * when required by Android 12L or earlier, precedes the same bounded walk. */
    @Command fun scanDeviceArchives(invoke: Invoke) {
        main.post {
            try {
                requireVisible()
                if (Build.VERSION.SDK_INT <= 32 && ContextCompat.checkSelfPermission(applicationContext,
                        Manifest.permission.READ_EXTERNAL_STORAGE) != PackageManager.PERMISSION_GRANTED) {
                    if (deviceScanInvoke != null) throw NativeDocumentError("NATIVE_OPERATION_BUSY")
                    deviceScanInvoke = invoke
                    requestPermissionForAlias("deviceFiles", invoke, "deviceFilesPermissionResult")
                    return@post
                }
                startDeviceScan(invoke)
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @PermissionCallback fun deviceFilesPermissionResult(invoke: Invoke) {
        main.post {
            if (deviceScanInvoke !== invoke) return@post
            deviceScanInvoke = null
            try { startDeviceScan(invoke) } catch (error: Exception) { reject(invoke, error) }
        }
    }

    private fun startDeviceScan(invoke: Invoke) {
        if (scanning) throw NativeDocumentError("SCAN_ALREADY_RUNNING")
        scanning = true
        deviceScanInvoke = invoke
        val maxEntries = invoke.getArgs().optInt("maxEntries", 512).coerceIn(1, 4096)
        val cancellation = CancellationSignal()
        scanCancellation = cancellation
        scanWorker.execute {
            val outcome = try { deviceScan.scan(maxEntries, cancellation) } catch (error: Exception) { error }
            main.post {
                if (deviceScanInvoke !== invoke) return@post
                deviceScanInvoke = null
                scanning = false
                scanCancellation = null
                if (outcome is Exception) reject(invoke, outcome) else invoke.resolve(outcome as JSObject)
            }
        }
    }

    /** Open a scanned archive in place. The token is the only identifier the
     * trusted Rust layer may hand back; document URIs never leave this plugin. */
    @Command fun openScannedArchive(invoke: Invoke) {
        main.post {
            try {
                requireVisible()
                val args = invoke.getArgs()
                val token = args.getString("token")
                val persistent = args.getBoolean("persistent")
                val open = ScannedOpen(invoke, CancellationSignal())
                scannedOpens.add(open)
                worker.execute {
                    val outcome = try {
                        deviceScan.open(token, persistent, open.cancellation)
                    } catch (error: Exception) { error }
                    main.post {
                        if (!scannedOpens.remove(open)) return@post
                        if (outcome is Exception) reject(invoke, outcome)
                        else invoke.resolve(JSObject().put("selection", outcome as JSObject).put("cancelled", false))
                    }
                }
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @Command fun exportFile(invoke: Invoke) {
        val open = try { invoke.getArgs().getBoolean("open") } catch (error: Exception) {
            reject(invoke, error); return
        }
        val share = try { invoke.getArgs().getBoolean("share") } catch (error: Exception) { false }
        if (!open) {
            begin(invoke, "export") { args ->
                Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                    .setType(checkedMime(args.getString("mimeType")))
                    .putExtra(Intent.EXTRA_TITLE, safeName(args.getString("suggestedName")))
                    .addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION or Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            return
        }
        main.post {
            try {
                requireVisible()
                if (operation != null || completionNotificationInvoke != null) throw NativeDocumentError("NATIVE_OPERATION_BUSY")
                val args = invoke.getArgs()
                label(args, "title"); label(args, "cancel")
                checkedSource(args.getString("path")); checkedMime(args.getString("mimeType"))
                safeName(args.getString("suggestedName"))
                val op = Operation(invoke, "open", args, activityGeneration)
                op.share = share
                operation = op
                launchWorker(op) { prepareViewer(op) }
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @ActivityCallback fun documentResult(invoke: Invoke, result: ActivityResult) {
        main.post {
            val op = operation
            if (op == null || op.invoke !== invoke || op.finished.get() || !ownsUi(op)) return@post
            if (result.resultCode == Activity.RESULT_CANCELED) {
                complete(op, cancelledResult(op.kind)); return@post
            }
            if (result.resultCode != Activity.RESULT_OK) {
                fail(op, NativeDocumentError("DOCUMENT_PICKER_FAILED")); return@post
            }
            try {
                val data = result.data ?: throw NativeDocumentError("DOCUMENT_PICKER_EMPTY_RESULT")
                val uris = mutableListOf<Uri>()
                data.clipData?.let { clip -> for (i in 0 until clip.itemCount) uris.add(clip.getItemAt(i).uri) }
                if (uris.isEmpty()) data.data?.let { uris.add(it) }
                if (uris.isEmpty() || uris.any { it.scheme != "content" })
                    throw NativeDocumentError("DOCUMENT_URI_REQUIRED")
                if (uris.size != 1 && (op.kind != "archives" &&
                    (op.kind != "externalBatch" || op.args.getBoolean("folder"))))
                    throw NativeDocumentError("SINGLE_DOCUMENT_REQUIRED")
                // The transient Activity grant remains alive through the worker. Persist only for explicit tree refresh.
                if (op.kind == "directory" && data.flags and Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION != 0) {
                    applicationContext.contentResolver.takePersistableUriPermission(uris.single(), Intent.FLAG_GRANT_READ_URI_PERMISSION)
                }
                if (op.kind != "scanFolder") showCopyDialog(op)
                launchWorker(op) {
                    when (op.kind) {
                        "archives", "directory" -> {
                            val staging = OwnedDocuments(applicationContext,
                                op.args.getString("destinationDirectory"), op.cancellation) { bytes, path -> progress(op, bytes, path) }
                            op.staging = staging
                            if (op.kind == "directory") staging.tree(uris.single()) else staging.pick(uris)
                            JSObject().put("paths", JSArray.from(staging.paths.toTypedArray())).put("cancelled", false).also {
                                if (op.kind == "directory") it.put("directory", staging.root.path)
                            }
                        }
                        "scanFolder" -> {
                            val tree = uris.single()
                            if (!DocumentsContract.isTreeUri(tree))
                                throw NativeDocumentError("DOCUMENT_TREE_UNAVAILABLE")
                            if (op.args.getBoolean("persistent")) {
                                if (data.flags and Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION == 0)
                                    throw NativeDocumentError("PROVIDER_CANNOT_REMEMBER_USE_COPY_IMPORT")
                                applicationContext.contentResolver.takePersistableUriPermission(
                                    tree, Intent.FLAG_GRANT_READ_URI_PERMISSION)
                            } else {
                                // Read-only sessions keep the transient Activity grant
                                // in memory for this process and write nothing.
                                deviceScan.addTransientTree(tree)
                            }
                            JSObject().put("cancelled", false).put("granted", true)
                                .put("persistent", op.args.getBoolean("persistent"))
                        }
                        "externalBatch" -> {
                            val result = externalDocuments.selectBatch(uris, op.args.getBoolean("folder"),
                                op.args.getBoolean("persistent"), data.flags, op.cancellation)
                            val selections = result.getJSONArray("selections")
                            for (index in 0 until selections.length())
                                op.externalReferences.add(selections.getJSONObject(index).getString("reference"))
                            result
                        }
                        "external" -> {
                            val selection = externalDocuments.select(uris.single(), op.args.getBoolean("folder"),
                                op.args.getBoolean("persistent"), data.flags, op.cancellation)
                            op.externalReference = selection.getString("reference")
                            JSObject().put("selection", selection).put("cancelled", false)
                        }
                        "text" -> readText(op, uris.single())
                        "export" -> saveExport(op, uris.single())
                        else -> throw NativeDocumentError("INVALID_DOCUMENT_OPERATION")
                    }
                }
            } catch (error: Exception) { fail(op, error) }
        }
    }

    private fun showCopyDialog(op: Operation) {
        check(ownsUi(op)) { "STALE_ACTIVITY_GENERATION" }
        val activity = activity
        val layout = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            val padding = (24 * resources.displayMetrics.density).toInt()
            setPadding(padding, padding, padding, padding)
            addView(ProgressBar(activity).apply { isIndeterminate = true })
            if (op.kind == "archives" || op.kind == "directory")
                addView(TextView(activity).apply { text = label(op.args, "copying") })
        }
        op.dialog = AlertDialog.Builder(activity).setTitle(label(op.args, "title")).setView(layout)
            .setNegativeButton(label(op.args, "cancel")) { _, _ -> op.cancellation.cancel() }
            .setOnCancelListener { op.cancellation.cancel() }.create().also { it.show() }
    }

    private fun launchWorker(op: Operation, task: () -> JSObject) {
        op.copying = true
        copyWorker.execute {
            try {
                val result = task()
                op.cancellation.throwIfCanceled()
                main.post {
                    if (op.cancellation.isCanceled || !ownsUi(op)) scheduleCancelledCleanup(op)
                    else if (op.kind == "open") launchViewer(op, result)
                    else complete(op, result)
                }
            } catch (error: Exception) {
                if (op.cancellation.isCanceled && error !is NativeDocumentError) {
                    finishCancelledCleanup(op)
                } else {
                    // A failed staging copy was never handed to Rust, so nothing
                    // refers to its files. Remove them here, on the worker that
                    // wrote them and before rejecting; only files and directories
                    // this operation created are touched, and fail() still
                    // reports any that could not be removed.
                    if (stagingFailureRemovesCopies(op.kind)) op.staging?.cancelCleanup()
                    main.post { fail(op, error) }
                }
            }
        }
    }

    private fun scheduleCancelledCleanup(op: Operation) {
        op.copying = true
        // Same executor as the copy it cleans up: cleanup never overlaps a
        // still-running transfer of the same staging directory.
        copyWorker.execute { finishCancelledCleanup(op) }
    }

    private fun finishCancelledCleanup(op: Operation) {
        if (op.finished.get()) return
        val retained = op.staging?.cancelCleanup().orEmpty().toMutableList()
        var providerCleanupFailure: Exception? = null
        op.externalReference?.let { reference ->
            try {
                externalDocuments.forget(reference)
                op.externalReference = null
            } catch (error: Exception) {
                providerCleanupFailure = NativeDocumentError("PROVIDER_CANCELLATION_CLEANUP_FAILED", error)
            }
        }
        for (reference in op.externalReferences.toList()) {
            try {
                externalDocuments.forget(reference)
                op.externalReferences.remove(reference)
            } catch (error: Exception) {
                providerCleanupFailure = NativeDocumentError("PROVIDER_CANCELLATION_CLEANUP_FAILED", error)
            }
        }
        for (scratch in op.exportScratch.asReversed()) {
            if (!scratch.remove()) retained.add(scratch.file.path)
        }
        main.post {
            providerCleanupFailure?.let { fail(op, it); return@post }
            if (retained.isNotEmpty()) fail(op, NativeDocumentError("CANCELLATION_CLEANUP_FAILED"))
            else if (op.exportUri != null) fail(op, NativeDocumentError("EXPORT_CANCELLED_PARTIAL"))
            else if (op.replaced) fail(op, NativeDocumentError(ACTIVITY_RECREATED))
            else complete(op, cancelledResult(op.kind))
        }
    }

    private fun progress(op: Operation, bytes: Long, path: String?) {
        val now = android.os.SystemClock.elapsedRealtime()
        if (now - op.lastProgress < 150) return
        op.lastProgress = now
        main.post {
            if (!op.finished.get() && ownsUi(op)) trigger("document-progress", JSObject().put("operation", op.kind)
                .put("bytes", bytes).put("path", path ?: JSONObject.NULL))
        }
    }

    private fun readText(op: Operation, uri: Uri): JSObject {
        val maxBytes = op.args.getLong("maxBytes")
        val text = StringBuilder()
        applicationContext.contentResolver.openAssetFileDescriptor(uri, "r", op.cancellation)?.use { asset ->
            val provider = CancellableProviderInputStream(asset.createInputStream(),
                asset.parcelFileDescriptor.fileDescriptor, op.cancellation)
            val bounded = object : FilterInputStream(provider) {
                var count = 0L
                override fun read(): Int {
                    op.cancellation.throwIfCanceled()
                    val value = super.read()
                    if (value != -1 && ++count > maxBytes) throw NativeDocumentError("TEXT_DOCUMENT_TOO_LARGE")
                    return value
                }
                override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
                    op.cancellation.throwIfCanceled()
                    val allowed = minOf(length.toLong(), maxBytes - count + 1).toInt()
                    val size = provider.read(buffer, offset, allowed)
                    if (size > 0) {
                        count += size
                        if (count > maxBytes) throw NativeDocumentError("TEXT_DOCUMENT_TOO_LARGE")
                    }
                    return size
                }
            }
            InputStreamReader(bounded, Charsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT).onUnmappableCharacter(CodingErrorAction.REPORT)).use { reader ->
                val chars = CharArray(8192)
                while (true) {
                    op.cancellation.throwIfCanceled()
                    val count = reader.read(chars)
                    if (count == -1) break
                    text.append(chars, 0, count)
                }
            }
        } ?: throw NativeDocumentError("DOCUMENT_STREAM_UNAVAILABLE")
        return JSObject().put("text", text.toString()).put("cancelled", false)
    }

    private fun checkedSource(path: String): File {
        val source = ownedPath(applicationContext, path)
        if (!source.isFile) throw NativeDocumentError("EXPORT_SOURCE_UNAVAILABLE")
        return source
    }

    private fun checkedMime(mime: String): String {
        if (!Regex("[A-Za-z0-9!#$&^_.+-]+/[A-Za-z0-9!#$&^_.+-]+").matches(mime))
            throw NativeDocumentError("INVALID_EXPORT_MIME")
        return mime
    }

    private fun copyExport(op: Operation, output: FileOutputStream) {
        val source = checkedSource(op.args.getString("path"))
        // O_NOFOLLOW closes the leaf race between validation and opening.
        val fd = android.system.Os.open(source.path,
            android.system.OsConstants.O_RDONLY or android.system.OsConstants.O_NOFOLLOW, 0)
        try {
            // The raw descriptor stays owned by this try/finally. The stream
            // owns only its explicit duplicate through AutoCloseInputStream.
            ParcelFileDescriptor.AutoCloseInputStream(ParcelFileDescriptor.dup(fd)).use { input ->
                val buffer = ByteArray(128 * 1024)
                var bytes = 0L
                while (true) {
                    op.cancellation.throwIfCanceled()
                    val count = input.read(buffer)
                    if (count == -1) break
                    if (count == 0) continue
                    output.write(buffer, 0, count)
                    bytes += count
                    progress(op, bytes, source.path)
                }
                output.flush()
                try { output.fd.sync() } catch (error: Exception) {
                    throw NativeDocumentError("EXPORT_DURABILITY_ERROR", error)
                }
            }
        } finally { android.system.Os.close(fd) }
    }

    private fun saveExport(op: Operation, uri: Uri): JSObject {
        applicationContext.contentResolver.query(uri, arrayOf(OpenableColumns.SIZE), null, null, null, op.cancellation)?.use {
            if (it.moveToFirst() && !it.isNull(0) && it.getLong(0) > 0)
                throw NativeDocumentError("EXPORT_DESTINATION_NOT_EMPTY")
        }
        // CREATE_DOCUMENT requires the OS/provider to confirm the destination; never silently truncate an existing file.
        applicationContext.contentResolver.openFileDescriptor(uri, "rw", op.cancellation)?.use { parcel ->
            if (parcel.statSize > 0) throw NativeDocumentError("EXPORT_DESTINATION_NOT_EMPTY")
            op.exportUri = uri
            ParcelFileDescriptor.AutoCloseOutputStream(parcel).use { output -> copyExport(op, output) }
        } ?: throw NativeDocumentError("EXPORT_DESTINATION_UNAVAILABLE")
        return JSObject().put("cancelled", false)
    }

    private fun prepareViewer(op: Operation): JSObject {
        val base = ownedPath(applicationContext, File(applicationContext.cacheDir.canonicalFile, "kiwix-mobile-exports").path)
        if (!base.isDirectory && !base.mkdir()) throw NativeDocumentError("EXPORT_CACHE_UNAVAILABLE")
        val directory = File(base, UUID.randomUUID().toString())
        if (!directory.mkdir()) throw NativeDocumentError("EXPORT_CACHE_UNAVAILABLE")
        val target = File(directory, safeName(op.args.getString("suggestedName")))
        op.exportScratch.add(ExportScratch(directory))
        val fd = android.system.Os.open(target.path, android.system.OsConstants.O_WRONLY or
            android.system.OsConstants.O_CREAT or android.system.OsConstants.O_EXCL or
            android.system.OsConstants.O_NOFOLLOW, 384)
        try {
            op.exportScratch.add(ExportScratch(target))
            ParcelFileDescriptor.AutoCloseOutputStream(ParcelFileDescriptor.dup(fd)).use { output ->
                copyExport(op, output)
            }
        } finally { android.system.Os.close(fd) }
        syncDirectory(directory)
        syncDirectory(base)
        return JSObject().put("path", target.path)
    }

    private fun launchViewer(op: Operation, result: JSObject) {
        try {
            requireVisible()
            check(ownsUi(op)) { "STALE_ACTIVITY_GENERATION" }
            val activity = activity
            val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.kiwix.mobile.exports",
                File(result.getString("path")))
            val mime = checkedMime(op.args.getString("mimeType"))
            val intent = if (op.share) {
                Intent(Intent.ACTION_SEND).setType(mime)
                    .putExtra(Intent.EXTRA_STREAM, uri)
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            } else {
                Intent(Intent.ACTION_VIEW).setDataAndType(uri, mime)
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            intent.clipData = ClipData.newRawUri("", uri)
            val receiptIntent = Intent(activity, KiwixExportChoiceReceiver::class.java)
                .setData(Uri.parse("kiwix-export-choice:${op.chooserToken}")).putExtra("token", op.chooserToken)
            val receipt = PendingIntent.getBroadcast(activity, 0, receiptIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
            op.chooserReceipt = receipt
            val chooser = Intent.createChooser(intent, label(op.args, "title"), receipt.intentSender)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            chooser.clipData = intent.clipData
            op.copying = false
            val pdfJobId = op.args.optString("pdfJobId", "")
            if (pdfJobId.isEmpty()) startActivityForResult(op.invoke, chooser, "viewerResult")
            else if (!NativeArticlePdf.commitExport(pdfJobId, Runnable {
                    startActivityForResult(op.invoke, chooser, "viewerResult")
                })) throw NativeDocumentError("ARTICLE_PDF_CANCELLED")
        } catch (error: Exception) { fail(op, error) }
    }

    internal fun exportChosen(token: String) {
        // Called by the explicit BroadcastReceiver on the main thread; settle selection immediately.
        val op = operation
        if (op?.kind == "open" && op.chooserToken == token && ownsUi(op))
            complete(op, JSObject().put("cancelled", false))
    }

    @ActivityCallback fun viewerResult(invoke: Invoke, result: ActivityResult) {
        main.post {
            val op = operation
            // Selection receipt completes success; returning without choosing is explicit cancellation.
            if (op?.kind == "open" && op.invoke === invoke && ownsUi(op)) {
                op.cancellation.cancel()
                scheduleCancelledCleanup(op)
            }
        }
    }

    private fun cancelledResult(kind: String): JSObject = JSObject().put("cancelled", true).also {
        when (kind) {
            "archives" -> it.put("paths", JSArray())
            "directory" -> it.put("paths", JSArray()).put("directory", JSONObject.NULL)
            "text" -> it.put("text", JSONObject.NULL)
        }
    }

    private fun complete(op: Operation, result: JSObject) {
        if (!op.finished.compareAndSet(false, true)) return
        op.dialog?.dismiss()
        op.dialog = null
        op.chooserReceipt?.cancel()
        if (operation === op) operation = null
        op.invoke.resolve(result)
    }

    private fun fail(op: Operation, error: Exception) {
        if (!op.finished.compareAndSet(false, true)) return
        op.dialog?.dismiss()
        op.dialog = null
        op.chooserReceipt?.cancel()
        if (operation === op) operation = null
        val retained = op.staging?.retainedPaths().orEmpty() +
            op.exportScratch.filter { it.remains() }.map { it.file.path }
        val data = JSObject().put("paths", JSArray.from(retained.toTypedArray()))
        op.exportUri?.let { data.put("destinationUri", it.toString()) }
        op.externalReference?.let { data.put("retainedProviderReference", it) }
        reject(op.invoke, error, data)
    }

    private fun reject(invoke: Invoke, error: Exception, data: JSObject? = null) {
        val code = when (error) {
            is NativeDocumentError -> error.code
            is SecurityException -> "DOCUMENT_PERMISSION_DENIED"
            is OperationCanceledException -> "DOCUMENT_OPERATION_CANCELLED"
            else -> "NATIVE_DOCUMENT_ERROR"
        }
        // Rust's mobile error Display drops structured data: preserve recovery information in its message too.
        val details = (data ?: JSObject()).put("detail", error.cause?.message ?: error.message ?: code)
        val message = JSObject().put("code", code).put("data", details).toString()
        invoke.reject(message, code, null, details)
    }

    @Command fun openUrl(invoke: Invoke) {
        main.post {
            try {
                requireVisible()
                val value = invoke.getArgs().getString("url")
                val parsed = java.net.URI(value)
                val scheme = parsed.scheme?.lowercase()
                if (scheme !in listOf("http", "https", "mailto") ||
                    (scheme != "mailto" && parsed.host.isNullOrBlank()) ||
                    parsed.rawUserInfo != null || value.any { it.code < 32 || it.code == 127 })
                    throw NativeDocumentError("UNSAFE_EXTERNAL_URL")
                val intent = Intent(
                    if (scheme == "mailto") Intent.ACTION_SENDTO else Intent.ACTION_VIEW,
                    Uri.parse(parsed.toASCIIString()))
                if (scheme != "mailto") intent.addCategory(Intent.CATEGORY_BROWSABLE)
                activity.startActivity(intent)
                invoke.resolve(JSObject())
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @Command fun completionNotificationCapability(invoke: Invoke) {
        try { invoke.resolve(AndroidCompletionNotifications.capability(applicationContext)) }
        catch (error: Exception) { reject(invoke, error) }
    }

    @Command fun configureCompletionNotifications(invoke: Invoke) {
        worker.execute {
            try {
                val args = invoke.getArgs()
                if (args.getBoolean("protected")) DownloadNotifications.revoke(applicationContext)
                invoke.resolve(AndroidCompletionNotifications.configure(applicationContext, args))
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @Command fun admittedCompletionNotification(invoke: Invoke) {
        worker.execute {
            try { invoke.resolve(AndroidCompletionNotifications.admitted(applicationContext, invoke.getArgs())) }
            catch (error: Exception) { reject(invoke, error) }
        }
    }

    @Command fun takeCompletionNotificationTap(invoke: Invoke) {
        main.post {
            try {
                requireVisible()
                worker.execute {
                    try { invoke.resolve(AndroidCompletionNotifications.takeTap(applicationContext)) }
                    catch (error: Exception) { reject(invoke, error) }
                }
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @Command fun acknowledgeCompletionNotificationTap(invoke: Invoke) {
        main.post {
            try {
                requireVisible()
                val args = invoke.getArgs()
                worker.execute {
                    try { invoke.resolve(AndroidCompletionNotifications.acknowledgeTap(applicationContext, args)) }
                    catch (error: Exception) { reject(invoke, error) }
                }
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @Command fun requestCompletionNotifications(invoke: Invoke) {
        main.post {
            try {
                requireVisible()
                if (completionNotificationInvoke != null || operation != null)
                    throw NativeDocumentError("NATIVE_OPERATION_BUSY")
                if (Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(applicationContext,
                        Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                    completionNotificationInvoke = invoke
                    requestPermissionForAlias("notifications", invoke, "completionNotificationPermissionResult")
                } else invoke.resolve(JSObject().put("granted",
                    NotificationManagerCompat.from(applicationContext).areNotificationsEnabled()))
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @PermissionCallback fun completionNotificationPermissionResult(invoke: Invoke) {
        main.post {
            if (completionNotificationInvoke !== invoke) return@post
            completionNotificationInvoke = null
            try {
                if (destroyed || activityReference.get() !== currentActivity.get())
                    throw NativeDocumentError("ACTIVITY_DESTROYED")
                val granted = (Build.VERSION.SDK_INT < 33 || ContextCompat.checkSelfPermission(applicationContext,
                    Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) &&
                    NotificationManagerCompat.from(applicationContext).areNotificationsEnabled()
                invoke.resolve(JSObject().put("granted", granted))
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    private fun receiveCompletionNotification(intent: Intent?) {
        if (intent == null) return
        // The verified-tap journal is an AtomicFile with an fsync. Copy the
        // intent on main and commit on the worker so the Activity callback never
        // blocks; the tap event is delivered only after the durable write.
        val copy = try { Intent(intent) } catch (_: Exception) { return }
        worker.execute {
            val received = try {
                AndroidCompletionNotifications.receiveIntent(applicationContext, copy)
            } catch (_: Exception) { false }
            if (received) main.post {
                if (!destroyed && hasListener("completion-notification-tap"))
                    trigger("completion-notification-tap", JSObject())
            }
        }
    }

    @Command fun transferState(invoke: Invoke) {
        val source = activityReference.get()
        val generation = activityGeneration
        main.post {
            try {
                val args = invoke.getArgs()
                val currentSource = activityScreenCallbackCurrent(source, generation, activityReference.get(),
                    activityGeneration, currentActivity.get())
                if (!args.getBoolean("active")) {
                    KiwixTransferService.stopOwned(applicationContext)
                    if (currentSource) releaseTransferScreen()
                    foregroundInvoke?.let { finishForegroundStart(it, "TRANSFER_START_CANCELLED") }
                    invoke.resolve(JSObject()); return@post
                }
                for (key in listOf("title", "body", "pauseLabel", "channelName")) {
                    if (args.getString(key).isBlank()) throw NativeDocumentError("LOCALIZED_LABEL_REQUIRED")
                }
                if (!KiwixTransferService.running) {
                    if (!currentSource) throw NativeDocumentError("STALE_ACTIVITY_GENERATION")
                    requireVisible()
                }
                // The foreground service is application-owned. A stale screen
                // callback cannot borrow a successor window, but must not pause
                // an already-admitted background transfer either.
                if (currentSource && source?.isDestroyed != true && source?.isFinishing != true && !destroyed)
                    updateTransferScreen(source)
                // Android permits an explicitly started foreground service even
                // when notification permission is denied. Keep its required
                // notification; completion-notification consent is separate.
                startTransferService(invoke, source, generation)
            } catch (error: Exception) {
                if (activityScreenCallbackCurrent(source, generation, activityReference.get(),
                        activityGeneration, currentActivity.get()) &&
                    !KiwixTransferService.running && foregroundInvoke == null) releaseTransferScreen()
                reject(invoke, error)
            }
        }
    }

    private fun updateTransferScreen(source: Activity?) {
        if (source == null || !windowState.foreground || !uiAdmission) {
            releaseTransferScreen()
            return
        }
        if (transferScreen?.context === source) return
        releaseTransferScreen()
        // Android ORs keepScreenOn across attached views. Removing our view
        // cannot clear a reader's assertion or a pre-existing window flag.
        val assertion = View(source).apply { keepScreenOn = true }
        val decor = source.window.decorView as ViewGroup
        decor.addView(assertion, ViewGroup.LayoutParams(0, 0))
        transferScreen = assertion
    }

    private fun releaseTransferScreen() {
        transferScreen?.let { assertion ->
            (assertion.parent as? ViewGroup)?.removeView(assertion)
        }
        transferScreen = null
    }

    private fun startTransferService(invoke: Invoke, source: Activity?, generation: Long) {
        if (foregroundInvoke != null) throw NativeDocumentError("TRANSFER_START_PENDING")
        val args = invoke.getArgs()
        val intent = KiwixTransferService.prepareUpdate(
            Intent(applicationContext, KiwixTransferService::class.java).setAction(KiwixTransferService.ACTION_UPDATE))
        for (key in listOf("title", "body", "pauseLabel", "channelName")) intent.putExtra(key, args.getString(key))
        for (key in listOf("progress", "total")) if (args.has(key) && !args.isNull(key)) {
            val number = args.getLong(key)
            if (number < 0) throw NativeDocumentError("INVALID_TRANSFER_PROGRESS")
            intent.putExtra(key, number)
        }
        if (KiwixTransferService.running) {
            // Deliver directly to the admitted instance. Never restart a stopped
            // foreground-service generation from an Activity-independent update.
            if (!KiwixTransferService.updateRunning(intent)) throw NativeDocumentError("TRANSFER_START_CANCELLED")
            invoke.resolve(JSObject())
            return
        }
        requireVisible()
        foregroundInvoke = invoke
        intent.putExtra("confirmation", object : ResultReceiver(main) {
            override fun onReceiveResult(resultCode: Int, data: Bundle?) {
                finishForegroundStart(invoke, if (resultCode == Activity.RESULT_OK) null
                    else data?.getString("code") ?: "TRANSFER_FOREGROUND_START_FAILED")
            }
        })
        val timeout = Runnable {
            if (foregroundInvoke === invoke) {
                KiwixTransferService.stopAfterFailure(applicationContext, "TRANSFER_FOREGROUND_START_TIMEOUT")
                finishForegroundStart(invoke, "TRANSFER_FOREGROUND_START_TIMEOUT")
            }
        }
        foregroundTimeout = timeout
        main.postDelayed(timeout, 4000)
        // The service refuses to run while a pause, timeout or error signal is
        // still waiting for the shell. Refuse here instead: a service started
        // with startForegroundService that stops at once is a start Android
        // punishes. The signal store is disk-backed, so it is read off main.
        worker.execute {
            val pending: Any = try { TransferEvents.peek(applicationContext).length() != 0 }
                catch (error: Exception) { error }
            main.post {
                // Cancelled, timed out or retired while the signals were read.
                if (foregroundInvoke !== invoke) return@post
                val refusal = when {
                    pending is NativeDocumentError -> pending.code
                    pending is Exception -> "TRANSFER_FOREGROUND_START_FAILED"
                    pending == true -> "TRANSFER_CONTROL_PENDING"
                    else -> null
                }
                if (refusal != null) { finishForegroundStart(invoke, refusal); return@post }
                try {
                    requireVisible()
                    ContextCompat.startForegroundService(applicationContext, intent)
                } catch (error: Exception) {
                    foregroundInvoke = null
                    foregroundTimeout = null
                    main.removeCallbacks(timeout)
                    if (activityScreenCallbackCurrent(source, generation, activityReference.get(),
                            activityGeneration, currentActivity.get()) && !KiwixTransferService.running)
                        releaseTransferScreen()
                    reject(invoke, error)
                }
            }
        }
    }

    private fun finishForegroundStart(invoke: Invoke, code: String?) {
        if (foregroundInvoke !== invoke) return
        foregroundInvoke = null
        foregroundTimeout?.let { main.removeCallbacks(it) }
        foregroundTimeout = null
        if (code == null) invoke.resolve(JSObject())
        else {
            releaseTransferScreen()
            invoke.reject(code, code)
        }
    }

    @Command override fun registerListener(invoke: Invoke) {
        super.registerListener(invoke)
        val event = invoke.getArgs().getString("event")
        if (event in listOf("transfers-pause", "transfers-timeout", "transfers-error")) deliverTransferEvent(event)
        if (event == "incoming-documents") notifyIncomingDocuments()
    }

    internal fun deliverTransferEvent(event: String) {
        val source = activityReference.get()
        val generation = activityGeneration
        main.post {
            if (event == "transfers-timeout" || event == "transfers-pause" || event == "transfers-error")
                releaseDiscovery()
            if (!KiwixTransferService.running && foregroundInvoke == null &&
                activityScreenCallbackCurrent(source, generation, activityReference.get(),
                    activityGeneration, currentActivity.get())) releaseTransferScreen()
            if (destroyed || !hasListener(event)) return@post
            worker.execute {
                val record = TransferEvents.record(applicationContext, event) ?: return@execute
                main.post delivery@{
                    if (destroyed || !hasListener(event)) return@delivery
                    trigger(event, JSObject().put("requestId", record.getString("requestId")).also {
                        if (record.has("code")) it.put("code", record.getString("code"))
                    })
                }
            }
        }
    }

    @Command fun takeTransferEvents(invoke: Invoke) {
        worker.execute {
            try {
                invoke.resolve(JSObject().put("events", TransferEvents.peek(applicationContext)))
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @Command fun acknowledgeTransferEvent(invoke: Invoke) {
        worker.execute {
            try {
                val args = invoke.getArgs()
                TransferEvents.acknowledge(applicationContext,
                    args.getString("event"), args.getString("requestId"))
                invoke.resolve(JSObject())
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    /** Initial VIEW/SEND and warm onNewIntent intake stay native until shell admission. */
    private fun receiveDocuments(intent: Intent?) {
        if (intent == null) return
        if (intent.action !in listOf(Intent.ACTION_VIEW, Intent.ACTION_SEND, Intent.ACTION_SEND_MULTIPLE)) return
        // Copy on main, then parse every extra on the worker. A malformed
        // explicit Intent (bad Parcelable, wrong EXTRA_STREAM type) must never
        // throw from an Activity callback.
        val copy = try { Intent(intent) } catch (_: Exception) { return }
        worker.execute {
            val parsed = try {
                parseDocumentIntent(copy)
            } catch (_: Exception) {
                ParsedDocuments(emptyList(), 0, malformed = true)
            }
            if (parsed.handled || parsed.malformed || parsed.uris.isEmpty()) return@execute
            main.post { onDocumentsParsed(intent, parsed) }
        }
    }

    private fun parseDocumentIntent(intent: Intent): ParsedDocuments {
        if (intent.getBooleanExtra(INTAKE_HANDLED_EXTRA, false))
            return ParsedDocuments(emptyList(), 0, handled = true)
        val uris = linkedSetOf<Uri>()
        intent.data?.let { uris.add(it) }
        intent.clipData?.let { clip ->
            for (index in 0 until clip.itemCount) clip.getItemAt(index)?.uri?.let { uris.add(it) }
        }
        when (intent.action) {
            Intent.ACTION_SEND, Intent.ACTION_SEND_MULTIPLE ->
                uris.addAll(streamUris(intent.extras?.get(Intent.EXTRA_STREAM)))
        }
        return ParsedDocuments(uris.filter { it.scheme == "content" }, intent.flags)
    }

    /** Typed EXTRA_STREAM: never a blind Parcelable cast. */
    private fun streamUris(value: Any?): List<Uri> = when (value) {
        null -> emptyList()
        is Uri -> listOf(value)
        is Iterable<*> -> value.filterIsInstance<Uri>()
        is Array<*> -> value.filterIsInstance<Uri>()
        else -> emptyList()
    }

    private fun onDocumentsParsed(intent: Intent, parsed: ParsedDocuments) {
        if (parsed.handled || parsed.uris.isEmpty()) return
        val batchUris = JSONArray()
        for (uri in parsed.uris) batchUris.put(uri.toString())
        val urisKey = batchUris.toString()
        expireTransientIntake()
        // An admitted transient batch already owns this identity.
        if (transientBatchUris.containsKey(urisKey)) {
            incomingFailure = null
            incomingNotice = null
            intent.putExtra(INTAKE_HANDLED_EXTRA, true)
            notifyIncomingDocuments()
            return
        }
        // A batch already handed to the worker is durable-or-pending, never new.
        if (queuedIntakeBatches.contains(urisKey)) {
            incomingFailure = null
            incomingNotice = null
            // Only a successful worker commit may mark the Intent handled.
            notifyIncomingDocuments()
            return
        }
        // Bound the in-flight queue before paying for a worker task. The worker
        // repeats the durable check after its prefs/grant read; a known durable
        // duplicate still reaches that dedupe even at the cap.
        if (!durableIntakeKeys.contains(urisKey) &&
            (queuedIntakeBatches.size >= MAX_TRANSIENT_INTAKE_BATCHES ||
             transientBatches.size + pendingTransientAdmissions >= MAX_TRANSIENT_INTAKE_BATCHES)) {
            incomingFailure = "INCOMING_DOCUMENTS_LIMIT"
            notifyIncomingDocuments()
            return
        }
        queuedIntakeBatches.add(urisKey)
        val batch = JSONObject().put("requestId", UUID.randomUUID().toString()).put("uris", batchUris)
        worker.execute { commitDocumentBatch(intent, parsed, urisKey, batch) }
    }

    private fun rememberIncomingFlags(uris: List<Uri>, flags: Int): Boolean {
        val unique = uris.toSet()
        val missing = unique.filter { it !in incomingGrantRefs }
        if (incomingGrantRefs.size + missing.size > MAX_INCOMING_GRANT_FLAGS) return false
        for (uri in unique) {
            incomingGrantRefs[uri] = (incomingGrantRefs[uri] ?: 0) + 1
            incomingGrantFlags[uri] = (incomingGrantFlags[uri] ?: 0) or flags
        }
        return true
    }

    private fun forgetIncomingFlags(uris: List<Uri>) {
        for (uri in uris.toSet()) {
            val count = incomingGrantRefs[uri] ?: continue
            if (count <= 1) {
                incomingGrantRefs.remove(uri)
                incomingGrantFlags.remove(uri)
            } else incomingGrantRefs[uri] = count - 1
        }
    }

    private fun removeTransientBatch(requestId: String): TransientBatch? {
        val removed = transientBatches.remove(requestId) ?: return null
        transientBatchUris.remove(removed.urisKey)
        forgetIncomingFlags(removed.uris)
        // Capacity was freed; a prior limit failure is no longer current.
        if (incomingFailure == "INCOMING_DOCUMENTS_LIMIT") incomingFailure = null
        return removed
    }

    /**
     * Every SharedPreferences read/commit and the persisted-grant binder query
     * run here. The Activity callback has already returned; only a durable
     * commit or a durably-marked transient batch may set INTAKE_HANDLED.
     */
    private fun commitDocumentBatch(intent: Intent, parsed: ParsedDocuments, urisKey: String, batch: JSONObject) {
        var duplicate = false
        var saved = false
        var failure: String? = null
        try {
            // One binder call for the whole intake, never one per attached document.
            val persisted = applicationContext.contentResolver.persistedUriPermissions
                .filter { it.isReadPermission }.mapTo(HashSet()) { it.uri }
            val grants = parsed.uris.map { it in persisted }
            val prefs = intakePreferences()
            val before = prefs.getString(INTAKE_BATCHES, "[]") ?: "[]"
            val durable = JSONArray(before)
            duplicate = (0 until durable.length()).any {
                durable.getJSONObject(it).getJSONArray("uris").toString() == urisKey
            }
            if (!duplicate && intakeCanPersist(grants)) {
                try {
                    saved = prefs.edit().putString(INTAKE_BATCHES,
                        JSONArray(before).put(batch).toString()).commit()
                } catch (error: Exception) {
                    // Editor.commit applies the in-memory map before its disk
                    // write; a throwing commit must not expose an unpersisted
                    // batch to the shell.
                    try { prefs.edit().putString(INTAKE_BATCHES, before).commit() } catch (_: Exception) { }
                    throw error
                }
                if (!saved) {
                    // SharedPreferences changes memory before its disk commit; restore the previous visible queue.
                    try { prefs.edit().putString(INTAKE_BATCHES, before).commit() } catch (_: Exception) { }
                    // A failed durable write is not a transient grant: keep the
                    // Intent unhandled so the next delivery retries the batch.
                    failure = "INCOMING_DOCUMENTS_DURABILITY_ERROR"
                }
            }
        } catch (_: Exception) { failure = "INCOMING_DOCUMENTS_DURABILITY_ERROR" }
        main.post {
            if (failure != null) {
                queuedIntakeBatches.remove(urisKey)
                incomingFailure = failure
                notifyIncomingDocuments()
                return@post
            }
            if (duplicate || saved) {
                queuedIntakeBatches.remove(urisKey)
                durableIntakeKeys.add(urisKey)
                if (durableIntakeKeys.size > MAX_TRANSIENT_INTAKE_BATCHES * 2) {
                    durableIntakeKeys.remove(durableIntakeKeys.first())
                }
                incomingFailure = null
                incomingNotice = null
                intent.putExtra(INTAKE_HANDLED_EXTRA, true)
                notifyIncomingDocuments()
                return@post
            }
            if (transientBatches.size + pendingTransientAdmissions >= MAX_TRANSIENT_INTAKE_BATCHES) {
                queuedIntakeBatches.remove(urisKey)
                incomingFailure = "INCOMING_DOCUMENTS_LIMIT"
                notifyIncomingDocuments()
                return@post
            }
            if (!rememberIncomingFlags(parsed.uris, parsed.flags)) {
                queuedIntakeBatches.remove(urisKey)
                incomingFailure = "INCOMING_DOCUMENTS_LIMIT"
                notifyIncomingDocuments()
                return@post
            }
            ++pendingTransientAdmissions
            // Admit the transient batch only after its marker is durable. Keep
            // the queued key until then so a redelivery cannot be marked handled
            // before this write succeeds.
            setTransientIntakeMarker(true) { committed ->
                --pendingTransientAdmissions
                queuedIntakeBatches.remove(urisKey)
                if (committed) {
                    val requestId = batch.getString("requestId")
                    transientBatches[requestId] = TransientBatch(batch, urisKey, parsed.uris)
                    transientBatchUris[urisKey] = requestId
                    durableIntakeKeys.add(urisKey)
                    if (durableIntakeKeys.size > MAX_TRANSIENT_INTAKE_BATCHES * 2) {
                        durableIntakeKeys.remove(durableIntakeKeys.first())
                    }
                    incomingFailure = null
                    incomingNotice = null
                    intent.putExtra(INTAKE_HANDLED_EXTRA, true)
                } else {
                    forgetIncomingFlags(parsed.uris)
                    incomingFailure = "INCOMING_DOCUMENTS_DURABILITY_ERROR"
                    // A concurrent ack may have set the marker true; reconcile
                    // to the live state now that this admission failed.
                    setTransientIntakeMarker(
                        transientBatches.isNotEmpty() || pendingTransientAdmissions > 0,
                    ) { }
                }
                notifyIncomingDocuments()
            }
        }
    }

    /** PROCESS_TEXT, kiwix:// links and dynamic shortcuts; consumed by the shell. */
    private fun receiveEntries(intent: Intent?) {
        if (intent == null) return
        // AppEntryPoints reads SharedPreferences; parse and persist on the
        // worker so the first preference load never runs in the Activity callback.
        val copy = try { Intent(intent) } catch (_: Exception) { return }
        worker.execute {
            try {
                AppEntryPoints.receive(applicationContext, copy, worker) { queued ->
                    if (queued) main.post {
                        // Marked only after the durable write, so a failed commit can be
                        // retried by the next onResume/onNewIntent delivery.
                        AppEntryPoints.markHandled(intent)
                        if (!destroyed && hasListener("app-entry")) trigger("app-entry", JSObject())
                    }
                }
            } catch (_: Exception) {
                // Leave the Intent unhandled so the next delivery can retry.
            }
        }
    }

    private fun intakePreferences() = applicationContext.getSharedPreferences(
        INTAKE_PREFERENCES, android.content.Context.MODE_PRIVATE)

    /** Durable marker write on the worker; [then] runs on the main thread with
     * true only when the fsync committed, so nothing is published before it is
     * durable and a failed write stays retryable. */
    private fun setTransientIntakeMarker(pending: Boolean, then: ((Boolean) -> Unit)? = null) {
        worker.execute {
            var committed = false
            try {
                val prefs = intakePreferences()
                val before = prefs.getBoolean(INTAKE_TRANSIENT_PENDING, false)
                try {
                    committed = prefs.edit().putBoolean(INTAKE_TRANSIENT_PENDING, pending).commit()
                } catch (_: Exception) { committed = false }
                if (!committed) {
                    // commit() applies the in-memory value before its disk write;
                    // restore the previous marker so a failed write is not visible.
                    try { prefs.edit().putBoolean(INTAKE_TRANSIENT_PENDING, before).commit() } catch (_: Exception) { }
                }
            } catch (_: Exception) { committed = false }
            then?.let { continuation -> main.post {
                transientMarkerDirty = !committed
                continuation(committed)
            } }
        }
    }

    private var transientIntakePending = false
    private val transientIntakeWaiters = mutableListOf<() -> Unit>()

    /**
     * A transient batch cannot survive process death. The marker written when
     * it was queued lets the next cold start tell the user to re-open the file
     * instead of silently resurrecting an unusable URI.
     *
     * The marker read and reset run on the worker; [then] runs on the main
     * thread once that decision is known, so the first preference load never
     * blocks the Activity callback.
     */
    private fun expireTransientIntake(then: (() -> Unit)? = null) {
        if (transientIntakeChecked) { then?.invoke(); return }
        then?.let { transientIntakeWaiters.add(it) }
        if (transientIntakePending) return
        transientIntakePending = true
        val inMemoryBatches = transientBatches.size
        worker.execute {
            var resetAttempted = false
            var resetFailed = false
            val notice = try {
                val prefs = intakePreferences()
                val pending = prefs.getBoolean(INTAKE_TRANSIENT_PENDING, false)
                if (intakeColdStartNotice(pending, inMemoryBatches)) {
                    resetAttempted = true
                    // The notice is valid even if clearing fails; the marker
                    // remains for the next cold start to retry.
                    resetFailed = !prefs.edit().putBoolean(INTAKE_TRANSIENT_PENDING, false).commit()
                    true
                } else false
            } catch (_: Exception) { false }
            main.post {
                if (resetAttempted) transientMarkerDirty = resetFailed
                transientIntakeChecked = true
                transientIntakePending = false
                if (notice && incomingNotice == null) incomingNotice = INTAKE_REOPEN_MESSAGE
                val waiters = transientIntakeWaiters.toList()
                transientIntakeWaiters.clear()
                waiters.forEach { waiter -> try { waiter() } catch (_: Exception) { } }
            }
        }
    }

    private fun notifyIncomingDocuments() {
        main.post {
            if (destroyed || !hasListener("incoming-documents")) return@post
            expireTransientIntake {
                if (!destroyed && hasListener("incoming-documents")) {
                    if (incomingFailure != null || incomingNotice != null || transientBatches.isNotEmpty()) {
                        trigger("incoming-documents", JSObject())
                    } else {
                        // The durable queue read is disk work too; never on main.
                        worker.execute {
                            val pending = try {
                                JSONArray(intakePreferences().getString(INTAKE_BATCHES, "[]")).length() > 0
                            } catch (_: Exception) { false }
                            main.post {
                                if (pending && !destroyed && hasListener("incoming-documents"))
                                    trigger("incoming-documents", JSObject())
                            }
                        }
                    }
                }
            }
        }
    }

    @Command fun takeIncomingDocuments(invoke: Invoke) {
        main.post {
            try {
                expireTransientIntake {
                    worker.execute {
                        val values = try {
                            JSONArray(intakePreferences().getString(INTAKE_BATCHES, "[]"))
                        } catch (error: Exception) {
                            main.post { reject(invoke, error) }
                            return@execute
                        }
                        main.post {
                            try {
                                for (batch in transientBatches.values) values.put(batch.json)
                                // Surface a limit/lost-batch failure only when no
                                // already-admitted batch can be handed off.
                                if (values.length() == 0) {
                                    incomingFailure?.let { throw NativeDocumentError(it) }
                                    incomingNotice?.let {
                                        incomingNotice = null
                                        throw NativeDocumentError(it)
                                    }
                                }
                                invoke.resolve(JSObject().put("batches", values))
                            } catch (error: Exception) { reject(invoke, error) }
                        }
                    }
                }
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @Command fun acknowledgeIncomingDocuments(invoke: Invoke) {
        main.post {
            try {
                val requestId = invoke.getArgs().getString("requestId")
                if (requestId.isBlank()) throw NativeDocumentError("INVALID_INCOMING_REQUEST_ID")
                worker.execute {
                    val failure = try {
                        val prefs = intakePreferences()
                        val before = prefs.getString(INTAKE_BATCHES, "[]") ?: "[]"
                        val pending = JSONArray(before)
                        val retained = JSONArray()
                        var found = false
                        for (index in 0 until pending.length()) {
                            val batch = pending.getJSONObject(index)
                            if (batch.getString("requestId") == requestId) found = true
                            else retained.put(batch)
                        }
                        // An already acknowledged ID is harmless; no unrelated batch is changed.
                        if (found &&
                            !prefs.edit().putString(INTAKE_BATCHES, retained.toString()).commit()) {
                            prefs.edit().putString(INTAKE_BATCHES, before).commit()
                            throw NativeDocumentError("INCOMING_DOCUMENTS_DURABILITY_ERROR")
                        }
                        null
                    } catch (error: Exception) { error }
                    main.post {
                        if (failure != null) {
                            reject(invoke, failure)
                            return@post
                        }
                        // Re-read the live keyed map on main: a new intent may
                        // have appended a batch while the worker fsynced.
                        val removed = removeTransientBatch(requestId)
                        if (removed != null || transientMarkerDirty) {
                            // Reconcile after a removal or a previous failed
                            // marker write; count in-flight admissions too.
                            setTransientIntakeMarker(
                                transientBatches.isNotEmpty() || pendingTransientAdmissions > 0,
                            ) { committed ->
                                if (committed) invoke.resolve(JSObject())
                                else reject(invoke, NativeDocumentError("INCOMING_DOCUMENTS_DURABILITY_ERROR"))
                            }
                        } else invoke.resolve(JSObject())
                    }
                }
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @Command fun takeAppEntries(invoke: Invoke) {
        // AppEntryPoints reads SharedPreferences; take it on the worker.
        worker.execute {
            try {
                invoke.resolve(JSObject().put("entries", AppEntryPoints.take(applicationContext)))
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @Command fun acknowledgeAppEntry(invoke: Invoke) {
        main.post {
            try {
                val requestId = invoke.getArgs().getString("requestId")
                if (requestId.isBlank()) throw NativeDocumentError("INVALID_APP_ENTRY_REQUEST_ID")
                AppEntryPoints.acknowledge(applicationContext, requestId, worker) { error ->
                    main.post {
                        if (error == null) invoke.resolve(JSObject()) else reject(invoke, error)
                    }
                }
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    /** Persist the shell's current archive labels for home-screen widgets. */
    @Command fun updateSearchWidget(invoke: Invoke) {
        main.post {
            try {
                val args = invoke.getArgs()
                val values = args.optJSONArray("titles") ?: JSONArray()
                val titles = (0 until minOf(values.length(), 8)).mapNotNull { index ->
                    values.optString(index).replace('\u0000', ' ').trim().takeIf { it.isNotEmpty() }
                }
                val restricted = args.optBoolean("restricted", false)
                KiwixSearchWidget.publish(applicationContext, titles, restricted)
                invoke.resolve(JSObject())
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    /** Ask the launcher to pin the current article; the user still confirms. */
    @Command fun pinArticleShortcut(invoke: Invoke) {
        main.post {
            try {
                requireVisible()
                val args = invoke.getArgs()
                val bookId = args.getString("bookId")
                val path = args.getString("path")
                val title = args.getString("title")
                if (bookId.isBlank() || path.isBlank())
                    throw NativeDocumentError("ARTICLE_LOCATION_REQUIRED")
                if (path.length > 3072 || path.any { it.code < 32 || it.code == 127 })
                    throw NativeDocumentError("INVALID_ARTICLE_LOCATION")
                invoke.resolve(JSObject().put("pinned",
                    AppShortcuts.pinArticle(activity, bookId, path, title)))
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    /** True only when an installed activity can handle ACTION_RECOGNIZE_SPEECH. */
    @Command fun speechRecognitionAvailable(invoke: Invoke) {
        main.post {
            try {
                invoke.resolve(JSObject().put("available",
                    SpeechRecognition.available(applicationContext)))
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    /**
     * Delegate dictation to the system recognizer activity. Kiwix never holds
     * RECORD_AUDIO or a microphone session; the activity returns only a final
     * transcript, which the shell treats as an ordinary search query.
     */
    @Command fun recognizeSpeech(invoke: Invoke) {
        main.post {
            try {
                requireVisible()
                if (!SpeechRecognition.available(applicationContext))
                    throw NativeDocumentError("SPEECH_RECOGNIZER_UNAVAILABLE")
                if (speechInvoke != null) throw NativeDocumentError("SPEECH_RECOGNIZER_BUSY")
                val language = invoke.getArgs().optString("language", "")
                    .takeIf { it.isNotBlank() && it.length <= 35 }
                speechInvoke = invoke
                try {
                    startActivityForResult(
                        invoke,
                        SpeechRecognition.intent(applicationContext, language),
                        "speechResult",
                    )
                } catch (error: Exception) {
                    speechInvoke = null
                    throw error
                }
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @ActivityCallback fun speechResult(invoke: Invoke, result: ActivityResult) {
        main.post {
            if (speechInvoke !== invoke) return@post
            speechInvoke = null
            if (result.resultCode == Activity.RESULT_CANCELED) {
                invoke.resolve(JSObject().put("cancelled", true).put("query", JSONObject.NULL))
                return@post
            }
            if (result.resultCode != Activity.RESULT_OK) {
                reject(invoke, NativeDocumentError("SPEECH_RECOGNITION_FAILED"))
                return@post
            }
            val query = result.data
                ?.getStringArrayListExtra(android.speech.RecognizerIntent.EXTRA_RESULTS)
                ?.firstOrNull()
                ?.replace("\u0000", "")
                ?.trim()
                ?.take(4096)
            if (query.isNullOrEmpty())
                invoke.resolve(JSObject().put("cancelled", true).put("query", JSONObject.NULL))
            else invoke.resolve(JSObject().put("cancelled", false).put("query", query))
        }
    }

    @Command fun copyIncomingArchives(invoke: Invoke) {
        main.post {
            try {
                requireVisible()
                if (operation != null || completionNotificationInvoke != null) throw NativeDocumentError("NATIVE_OPERATION_BUSY")
                val args = invoke.getArgs()
                label(args, "title"); label(args, "cancel"); label(args, "copying")
                val values = args.getJSONArray("uris")
                if (values.length() == 0) throw NativeDocumentError("DOCUMENT_URI_REQUIRED")
                val uris = (0 until values.length()).map { Uri.parse(values.getString(it)) }
                if (uris.any { it.scheme != "content" }) throw NativeDocumentError("DOCUMENT_URI_REQUIRED")
                val op = Operation(invoke, "archives", args, activityGeneration)
                operation = op
                try {
                    showCopyDialog(op)
                    launchWorker(op) {
                        val staging = OwnedDocuments(applicationContext,
                            args.getString("destinationDirectory"), op.cancellation) { bytes, path -> progress(op, bytes, path) }
                        op.staging = staging
                        staging.pick(uris)
                        JSObject().put("paths", JSArray.from(staging.paths.toTypedArray())).put("cancelled", false)
                    }
                } catch (error: Exception) { fail(op, error) }
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    @Command fun discoveryState(invoke: Invoke) {
        main.post {
            try {
                if (!invoke.getArgs().getBoolean("enabled")) releaseDiscovery()
                else {
                    if (!KiwixTransferService.running || !applicationResources.discoveryIsHeld()) requireVisible()
                    applicationResources.enableDiscovery()
                }
                invoke.resolve(JSObject())
            } catch (error: Exception) { reject(invoke, error) }
        }
    }

    private fun releaseDiscovery() {
        applicationResources.releaseDiscovery()
    }
}

/** An obscured Activity is still foreground until onStop proves otherwise. */
internal class ActivityWindowState {
    var foreground = false
        private set
    var interactive = false
        private set

    fun restore(started: Boolean, resumed: Boolean) {
        foreground = started || resumed
        interactive = resumed
    }
    fun resumed() {
        foreground = true
        interactive = true
    }
    fun paused() {
        interactive = false
    }
    fun stopped() {
        foreground = false
        interactive = false
    }
}

/** Nullable sources permit Activity-independent retirement, never rebasing onto a successor. */
internal fun activityScreenCallbackCurrent(
    source: Any?, generation: Long, bound: Any?, boundGeneration: Long, current: Any?,
): Boolean = source === bound && source === current && generation == boundGeneration

/**
 * The lifecycle event emitted from `onStop`. `onStop` is the first callback
 * that proves the Activity actually left the screen; permission and OEM
 * hotspot dialogs only pause it. Emitting suspension unconditionally is safe
 * because Rust deduplicates repeated reports for one lifecycle generation.
 */
internal fun stoppedActivityLifecycle(notificationPending: Boolean): Pair<String, String> =
    "mobile-suspended" to if (notificationPending)
        "notification-permission-backgrounded" else "activity-stopped"

/** A native intake batch may be persisted only when every URI has durable read access. */
internal fun intakeCanPersist(grants: List<Boolean>): Boolean =
    grants.isNotEmpty() && grants.all { it }

/**
 * True when a cold start found the transient-batch marker but no batch in
 * memory, meaning the process died before the user imported it.
 */
internal fun intakeColdStartNotice(transientMarker: Boolean, inMemoryBatches: Int): Boolean =
    transientMarker && inMemoryBatches == 0

/**
 * Whether an Activity's launch intent may still be acted on. Android gives a
 * restored Activity (saved state present, typically after the process was
 * killed in the background) and a task reopened from Recents the original
 * launch intent again, so an "Open with", link, selected-text search or
 * shortcut would otherwise repeat on every return. A recreation inside one
 * process keeps the intent only while no plugin has been offered it yet.
 */
internal fun launchIntentConsumable(
    restored: Boolean, flags: Int, unofferedByPredecessor: Boolean = false,
): Boolean = flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY == 0 &&
    (!restored || unofferedByPredecessor)

internal enum class RetirementOutcome { AWAIT_WORKER, FAIL_DESTROYED, FAIL_RECREATED, CANCELLED }

/**
 * How a pending document operation settles when its window is retired. A
 * running worker settles it after its own cleanup. A replaced Activity is not
 * the person cancelling: that ends as a failure the shell can report.
 */
internal fun retirementOutcome(copying: Boolean, viewerScratch: Boolean, replaced: Boolean): RetirementOutcome = when {
    copying -> RetirementOutcome.AWAIT_WORKER
    viewerScratch -> RetirementOutcome.FAIL_DESTROYED
    replaced -> RetirementOutcome.FAIL_RECREATED
    else -> RetirementOutcome.CANCELLED
}

/**
 * Copy imports stage whole archives in app-private storage. When one fails,
 * its staged files are unreferenced and must not outlive the attempt. Exports
 * and viewer copies keep their own recovery rules.
 */
internal fun stagingFailureRemovesCopies(kind: String): Boolean = kind == "archives" || kind == "directory"
