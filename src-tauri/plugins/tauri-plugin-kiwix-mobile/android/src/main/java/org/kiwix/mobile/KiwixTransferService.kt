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

class KiwixTransferService : Service() {
    companion object {
        const val ACTION_UPDATE = "org.kiwix.mobile.TRANSFER_UPDATE"
        const val ACTION_PAUSE = "org.kiwix.mobile.TRANSFER_PAUSE"
        private const val CHANNEL = "kiwix-transfers"
        private const val NOTIFICATION = 7401
        private const val ADMISSION = "org.kiwix.mobile.TRANSFER_ADMISSION"
        private var generation = 0L
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

        @Synchronized fun stop(context: Context) {
            generation += 1
            context.stopService(Intent(context, KiwixTransferService::class.java))
        }

        /** Main-thread retirement releases the owned lock before acknowledging Rust. */
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
    private val destroyed = CountDownLatch(1)

    override fun onCreate() {
        super.onCreate()
        instance = WeakReference(this)
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
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
            if (admitted == null) {
                confirmation?.send(Activity.RESULT_CANCELED, Bundle().apply {
                    putString("code", "TRANSFER_START_CANCELLED")
                })
                // A late cancelled start must not retire a newer admitted service.
                if (!running) stopSelf(startId)
                return START_NOT_STICKY
            }
            if (TransferEvents.peek(this).length() != 0) {
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

    private fun finish() {
        retire()
        running = false
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        stopSelf()
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
