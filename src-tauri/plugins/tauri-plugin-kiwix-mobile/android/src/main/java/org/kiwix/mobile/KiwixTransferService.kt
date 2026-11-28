package org.kiwix.mobile

import android.app.NotificationChannel
import android.app.Activity
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.os.Bundle
import android.os.ResultReceiver
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import java.lang.ref.WeakReference
import java.util.UUID
import java.util.concurrent.CountDownLatch
import org.json.JSONArray
import org.json.JSONObject

/** Signals are durable before retiring foreground admission; Rust remains the sole transfer owner. */
internal object TransferEvents {
    private var plugin = WeakReference<KiwixMobilePlugin>(null)
    private val eventNames = arrayOf("transfers-pause", "transfers-timeout", "transfers-error")
    private val uncommitted = mutableMapOf<String, String>()
    @Synchronized fun attach(value: KiwixMobilePlugin) { plugin = WeakReference(value) }
    @Synchronized fun detach(value: KiwixMobilePlugin) {
        if (plugin.get() === value) plugin.clear()
    }
    @Synchronized fun exportChosen(token: String) { plugin.get()?.exportChosen(token) }
    @Synchronized fun emit(context: Context, event: String, code: String? = null) {
        requireEvent(event)
        val record = JSONObject().put("event", event).put("requestId", UUID.randomUUID().toString())
        if (code != null) record.put("code", code)
        uncommitted[event] = record.toString()
        // Keep ownership in memory if storage fails; peek/ACK will fail visibly until persistence succeeds.
        try { persistPending(context) } catch (_: NativeDocumentError) { }
        plugin.get()?.deliverTransferEvent(event)
    }

    @Synchronized fun record(context: Context, event: String): JSONObject? {
        requireEvent(event)
        val serialized = uncommitted[event] ?: preferences(context).getString(event, null) ?: return null
        if (serialized.startsWith("{")) return JSONObject(serialized)
        // Preserve an outstanding signal from the previous code-only on-disk representation.
        val migrated = JSONObject().put("event", event).put("requestId", UUID.randomUUID().toString())
        if (serialized.isNotEmpty()) migrated.put("code", serialized)
        uncommitted[event] = migrated.toString()
        return migrated
    }

    @Synchronized fun peek(context: Context): JSONArray {
        val events = JSONArray()
        for (event in eventNames) record(context, event)?.let { events.put(it) }
        persistPending(context)
        return events
    }

    @Synchronized fun acknowledge(context: Context, event: String, requestId: String) {
        requireEvent(event)
        if (requestId.isBlank()) throw NativeDocumentError("INVALID_TRANSFER_REQUEST_ID")
        val current = record(context, event) ?: return
        persistPending(context)
        val prefs = preferences(context)
        if (current.getString("requestId") != requestId) return
        val serialized = current.toString()
        if (!prefs.edit().remove(event).commit()) {
            // A failed commit already changed SharedPreferences' memory; restore the pending record there.
            prefs.edit().putString(event, serialized).commit()
            throw NativeDocumentError("TRANSFER_EVENT_DURABILITY_ERROR")
        }
    }

    private fun preferences(context: Context) =
        context.getSharedPreferences("kiwix-mobile-events", Context.MODE_PRIVATE)

    private fun requireEvent(event: String) {
        if (event !in eventNames) throw NativeDocumentError("INVALID_TRANSFER_EVENT")
    }

    private fun persistPending(context: Context) {
        if (uncommitted.isEmpty()) return
        val editor = preferences(context).edit()
        for ((event, record) in uncommitted) editor.putString(event, record)
        if (!editor.commit()) throw NativeDocumentError("TRANSFER_EVENT_DURABILITY_ERROR")
        uncommitted.clear()
    }
}

internal enum class TransferStartStep { STALE, CONTROL_PENDING, ADMIT }

/**
 * What one delivered transfer update does. [foregroundFirst] marks a delivery
 * that is about to stop a service which has not entered the foreground yet:
 * when it was started with startForegroundService, Android kills the process
 * unless Service.startForeground runs once before the service stops.
 */
internal data class TransferStartDecision(val step: TransferStartStep, val foregroundFirst: Boolean)

/** [pendingEvents] is read only for an admitted start; it is disk-backed. */
internal fun transferStartDecision(
    admitted: Boolean, foreground: Boolean, pendingEvents: () -> Boolean,
): TransferStartDecision {
    val step = when {
        !admitted -> TransferStartStep.STALE
        pendingEvents() -> TransferStartStep.CONTROL_PENDING
        else -> TransferStartStep.ADMIT
    }
    return TransferStartDecision(step, step != TransferStartStep.ADMIT && !foreground)
}

internal enum class TransferStop { STOP_SERVICE, LEAVE_TO_QUEUED_START }

/**
 * How the service is retired from outside. Android kills the process when a
 * service is stopped while a start requested with startForegroundService has
 * not been delivered yet. Such a start is never stopped from outside: its
 * admission is retired instead, so it arrives stale, enters the foreground
 * once and stops itself.
 */
internal fun transferStopDecision(queuedForegroundStarts: Int): TransferStop =
    if (queuedForegroundStarts > 0) TransferStop.LEAVE_TO_QUEUED_START else TransferStop.STOP_SERVICE

/**
 * The start id a service instance may stop itself up to, or null while the
 * start that created it has not been delivered: that start stops the service
 * when it arrives.
 */
internal fun transferSelfStopId(deliveredStartId: Int): Int? = deliveredStartId.takeIf { it > 0 }

class KiwixTransferService : Service() {
    companion object {
        const val ACTION_UPDATE = "org.kiwix.mobile.TRANSFER_UPDATE"
        const val ACTION_PAUSE = "org.kiwix.mobile.TRANSFER_PAUSE"
        private const val CHANNEL = "kiwix-transfers"
        private const val NOTIFICATION = 7401
        private const val ADMISSION = "org.kiwix.mobile.TRANSFER_ADMISSION"
        private var generation = 0L
        /** Foreground starts requested from Android and not yet delivered to onStartCommand. */
        private var queuedForegroundStarts = 0
        private var instance = WeakReference<KiwixTransferService>(null)
        @Volatile var running = false
            private set

        @Synchronized fun prepareUpdate(intent: Intent): Intent =
            intent.putExtra(ADMISSION, generation)

        /** Main-thread update of the admitted instance, never an Android service start. */
        @Synchronized fun updateRunning(intent: Intent): Boolean {
            check(android.os.Looper.myLooper() == android.os.Looper.getMainLooper())
            val service = instance.get() ?: return false
            val admitted = service.admittedGeneration ?: return false
            if (!running || !isCurrent(admitted) || admission(intent) != admitted) return false
            service.onStartCommand(intent, 0, 0)
            return running && isCurrent(admitted)
        }

        /**
         * Main thread. The only place a foreground start is requested, so every
         * one is counted from the request until Android delivers it.
         */
        @Synchronized fun requestForegroundStart(context: Context, intent: Intent) {
            check(android.os.Looper.myLooper() == android.os.Looper.getMainLooper())
            ContextCompat.startForegroundService(context, intent)
            queuedForegroundStarts += 1
        }

        @Synchronized private fun foregroundStartDelivered() {
            if (queuedForegroundStarts > 0) queuedForegroundStarts -= 1
        }

        /**
         * Retires every admission issued so far. The service is stopped from
         * outside only when no foreground start is queued; a queued start finds
         * its admission retired and stops the service itself.
         */
        @Synchronized fun stop(context: Context) {
            generation += 1
            if (transferStopDecision(queuedForegroundStarts) == TransferStop.LEAVE_TO_QUEUED_START) return
            context.stopService(Intent(context, KiwixTransferService::class.java))
        }

        /**
         * Main-thread retirement releases the owned lock before acknowledging Rust.
         * The returned latch opens when the instance that existed at this call
         * is destroyed, which a queued foreground start may delay until that
         * start has been delivered. Null means no instance existed: a service
         * that a still-queued start creates afterwards stops itself and has no
         * latch here.
         */
        fun stopOwned(context: Context): CountDownLatch? {
            val service = instance.get()
            service?.finish()
            stop(context)
            return service?.destroyed
        }

        fun stopAfterFailure(context: Context, code: String) {
            // An update can fail while older Rust transfers still own this FGS.
            // Persist their stop obligation before suppressing normal onDestroy.
            TransferEvents.emit(context, "transfers-error", code)
            stop(context)
        }

        @Synchronized private fun admission(intent: Intent): Long? =
            if (intent.hasExtra(ADMISSION) && intent.getLongExtra(ADMISSION, -1) == generation) generation else null

        @Synchronized private fun isCurrent(admitted: Long): Boolean = admitted == generation
        @Synchronized private fun retire() { generation += 1 }
    }
    private var wakeLock: PowerManager.WakeLock? = null
    private var admittedGeneration: Long? = null
    /** This instance is in the foreground state Android requires of it. */
    private var foreground = false
    /** The newest start Android has delivered; 0 until the first one arrives. */
    private var deliveredStartId = 0
    private val destroyed = CountDownLatch(1)

    override fun onCreate() {
        super.onCreate()
        instance = WeakReference(this)
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // updateRunning calls this directly with startId 0; only Android's own
        // deliveries carry an id that stopSelf can be bounded by.
        if (startId > 0) deliveredStartId = startId
        // Android's deliveries of ACTION_UPDATE are exactly the starts requested
        // through requestForegroundStart.
        if (startId > 0 && intent?.action == ACTION_UPDATE) foregroundStartDelivered()
        if (intent?.action == ACTION_PAUSE) {
            TransferEvents.emit(this, "transfers-pause")
            finish()
            return START_NOT_STICKY
        }
        if (intent?.action != ACTION_UPDATE) {
            finish()
            return START_NOT_STICKY
        }
        @Suppress("DEPRECATION")
        val confirmation = intent.getParcelableExtra<ResultReceiver>("confirmation")
        try {
            val admitted = admission(intent)
            val decision = transferStartDecision(admitted != null, foreground) {
                TransferEvents.peek(this).length() != 0
            }
            if (decision.foregroundFirst) enterForegroundBeforeStop(intent)
            if (admitted == null || decision.step == TransferStartStep.STALE) {
                confirmation?.send(Activity.RESULT_CANCELED, Bundle().apply {
                    putString("code", "TRANSFER_START_CANCELLED")
                })
                // A late cancelled start must not retire a newer admitted service.
                if (!running) stopSelf(startId)
                return START_NOT_STICKY
            }
            if (decision.step == TransferStartStep.CONTROL_PENDING) {
                finish()
                confirmation?.send(Activity.RESULT_CANCELED, Bundle().apply {
                    putString("code", "TRANSFER_CONTROL_PENDING")
                })
                return START_NOT_STICKY
            }
            val title = requireNotNull(intent.getStringExtra("title"))
            val body = requireNotNull(intent.getStringExtra("body"))
            val pause = requireNotNull(intent.getStringExtra("pauseLabel"))
            val channelName = requireNotNull(intent.getStringExtra("channelName"))
            val manager = getSystemService(NotificationManager::class.java)
            if (Build.VERSION.SDK_INT >= 26) {
                manager.createNotificationChannel(NotificationChannel(CHANNEL, channelName,
                    NotificationManager.IMPORTANCE_LOW).apply { setShowBadge(false) })
            }
            val pauseIntent = PendingIntent.getService(this, 0,
                Intent(this, KiwixTransferService::class.java).setAction(ACTION_PAUSE),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
            val builder = NotificationCompat.Builder(this, CHANNEL)
                .setSmallIcon(android.R.drawable.stat_sys_download)
                .setContentTitle(title).setContentText(body).setOngoing(true)
                .setOnlyAlertOnce(true).setCategory(NotificationCompat.CATEGORY_PROGRESS)
                .addAction(android.R.drawable.ic_media_pause, pause, pauseIntent)
            if (DownloadNotifications.activeCount() > 1) {
                builder.setGroup(DownloadNotifications.GROUP).setGroupSummary(true)
            }
            packageManager.getLaunchIntentForPackage(packageName)?.let { launch ->
                builder.setContentIntent(PendingIntent.getActivity(this, 1, launch,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
            }
            val progress = intent.getLongExtra("progress", -1)
            val total = intent.getLongExtra("total", -1)
            if (progress >= 0 && total > 0) {
                val scaled = ((progress.coerceAtMost(total).toDouble() / total) * 10000).toInt()
                builder.setProgress(10000, scaled, false)
            } else builder.setProgress(0, 0, true)
            ServiceCompat.startForeground(this, NOTIFICATION, builder.build(),
                if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC else 0)
            foreground = true
            if (wakeLock == null) {
                wakeLock = (getSystemService(POWER_SERVICE) as PowerManager)
                    .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "$packageName:kiwix-transfers")
                    .apply { setReferenceCounted(false) }
            }
            // Progress/state updates renew a bounded lease, never a permanent CPU lock.
            wakeLock?.acquire(10 * 60 * 1000L)
            admittedGeneration = admitted
            running = true
            confirmation?.send(Activity.RESULT_OK, Bundle())
        } catch (error: Exception) {
            TransferEvents.emit(this, "transfers-error",
                if (error is NativeDocumentError) error.code else error.javaClass.simpleName)
            enterForegroundBeforeStop(intent)
            finish()
            confirmation?.send(Activity.RESULT_CANCELED, Bundle().apply {
                putString("code", if (error is NativeDocumentError) error.code else "TRANSFER_FOREGROUND_START_FAILED")
            })
        }
        // Process death does not restart a service without the Rust transfer owner.
        return START_NOT_STICKY
    }

    override fun onTimeout(startId: Int, fgsType: Int) {
        TransferEvents.emit(this, "transfers-timeout")
        finish()
    }

    override fun onTimeout(startId: Int) {
        TransferEvents.emit(this, "transfers-timeout")
        finish()
    }

    /**
     * The platform contract requires one startForeground call from a service
     * started with startForegroundService, even when it is about to stop;
     * [SpeechPlayback.foreground] follows the same rule. The notification is
     * removed again by the stop that follows, and reuses the labels this start
     * carried, falling back to the application's own name.
     */
    private fun enterForegroundBeforeStop(intent: Intent?) {
        if (foreground) return
        try {
            val application = applicationInfo.loadLabel(packageManager).toString()
            val channelName = intent?.getStringExtra("channelName")?.takeIf { it.isNotBlank() }
            if (Build.VERSION.SDK_INT >= 26) {
                val manager = getSystemService(NotificationManager::class.java)
                // Never rename an existing channel to the fallback label.
                if (channelName != null || manager.getNotificationChannel(CHANNEL) == null) {
                    manager.createNotificationChannel(NotificationChannel(CHANNEL, channelName ?: application,
                        NotificationManager.IMPORTANCE_LOW).apply { setShowBadge(false) })
                }
            }
            val notification = NotificationCompat.Builder(this, CHANNEL)
                .setSmallIcon(android.R.drawable.stat_sys_download)
                .setContentTitle(intent?.getStringExtra("title")?.takeIf { it.isNotBlank() } ?: application)
                .setOngoing(true).setOnlyAlertOnce(true).build()
            ServiceCompat.startForeground(this, NOTIFICATION, notification,
                if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC else 0)
            foreground = true
        } catch (_: Exception) {
            // Android refused foreground admission. Nothing more can satisfy the
            // contract; the caller still stops the service.
        }
    }

    private fun finish() {
        retire()
        running = false
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        foreground = false
        // Stop only up to the newest delivered start. A start Android has queued
        // but not delivered yet (a foreground start racing a Pause tap or an
        // outside stop) keeps the service alive; it then arrives with a retired
        // admission, enters the foreground once and stops itself. An unbounded
        // stopSelf() would stop the service under that queued foreground start.
        // With nothing delivered yet, the start that created this instance is
        // itself still queued and does the stopping.
        transferSelfStopId(deliveredStartId)?.let { stopSelf(it) }
    }

    override fun onDestroy() {
        try {
            if (running && admittedGeneration?.let { isCurrent(it) } == true) {
                TransferEvents.emit(this, "transfers-error", "TRANSFER_FOREGROUND_SERVICE_DESTROYED")
                retire()
            }
            running = false
            wakeLock?.let { if (it.isHeld) it.release() }
            wakeLock = null
            super.onDestroy()
        } finally {
            if (instance.get() === this) instance.clear()
            destroyed.countDown()
        }
    }
}
