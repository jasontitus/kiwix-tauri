package org.kiwix.mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon
import android.net.Uri
import android.os.Build
import android.os.Bundle
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

/** Presentation only. Actions re-enter the foreground shell's ordinary Downloads controls. */
internal object DownloadNotifications {
    const val ACTION = "org.kiwix.mobile.DOWNLOAD_ACTION"
    const val GROUP = "kiwix-http-downloads"
    const val ISSUED = "issuedDownloadActions"
    private const val CHANNEL = "kiwix-transfers"
    private const val TAG = "kiwix-http-download"
    private val activeStates = setOf(
        "queued", "connecting", "resolving", "initializing", "metadata",
        "checking", "downloading", "verifying", "publishing")
    private val shownStates = activeStates + setOf("paused", "failed", "error")
    private var activeJobs = 0
    private var labels = JSONObject()
    private var issued = JSONObject()
    /** The channel is created once per label; re-creating it is a binder call. */
    private var channelLabel: String? = null
    private var launchResolved = false
    private var launchTemplate: Intent? = null
    /** Rendered notification content per job, so unchanged jobs cost no binder call. */
    private val rendered = mutableMapOf<String, String>()
    /** PendingIntents keyed by job+action, valid until the phase nonce changes. */
    private val actionIntents = mutableMapOf<String, Pair<String, PendingIntent>>()
    private val contentIntents = mutableMapOf<String, PendingIntent>()

    private fun active(job: JSONObject) = job.optString("status") in activeStates
    private fun phase(status: String) = when (status) {
        "queued", "connecting", "resolving", "initializing", "metadata",
        "checking", "downloading", "verifying", "publishing" -> "active"
        "error" -> "failed"
        else -> status
    }
    private fun actions(phase: String): List<String> = when (phase) {
        "active" -> listOf("pause", "cancel")
        "paused" -> listOf("resume", "cancel")
        "failed" -> listOf("retry")
        else -> emptyList()
    }
    private fun token(record: JSONObject, action: String) = UUID.nameUUIDFromBytes(
        (record.getString("nonce") + ":" + action).toByteArray(Charsets.UTF_8)).toString()
    fun activeCount(): Int = activeJobs

    fun update(context: Context, args: JSONObject): Unit = synchronized(AppEntryPoints) {
        labels = args.getJSONObject("labels")
        val jobs = args.getJSONArray("jobs")
        var enabled = args.getBoolean("enabled") && NotificationManagerCompat.from(context).areNotificationsEnabled()
        val manager = context.getSystemService(NotificationManager::class.java)
        if (enabled && Build.VERSION.SDK_INT >= 26) {
            val label = labels.getString("channel")
            if (channelLabel != label) {
                manager.createNotificationChannel(NotificationChannel(CHANNEL, label,
                    NotificationManager.IMPORTANCE_LOW).apply { setShowBadge(false) })
                channelLabel = label
            }
            enabled = manager.getNotificationChannel(CHANNEL)?.importance != NotificationManager.IMPORTANCE_NONE
        }
        if (!enabled) {
            revoke(context)
            return@synchronized
        }
        val entries = (0 until jobs.length()).map { jobs.getJSONObject(it) }
        activeJobs = if (enabled) entries.count { active(it) } else 0
        // Issuance and entry acknowledgement use the same preferences transaction.
        // A new phase gets a fresh nonce; progress updates do not expire a live tap.
        val authority = context.getSharedPreferences("kiwix-mobile-entries", Context.MODE_PRIVATE)
        val before = authority.getString(ISSUED, "{}") ?: "{}"
        val previousIssued = JSONObject(before)
        val next = JSONObject()
        if (enabled) for (job in entries) {
            val currentPhase = phase(job.getString("status"))
            if (actions(currentPhase).isEmpty()) continue
            val id = job.getString("id")
            val old = previousIssued.optJSONObject(id)
            next.put(id, if (old?.optString("phase") == currentPhase) old else
                JSONObject().put("phase", currentPhase).put("nonce", UUID.randomUUID().toString()))
        }
        val serialized = next.toString()
        if (serialized != before && !authority.edit().putString(ISSUED, serialized).commit()) {
            authority.edit().putString(ISSUED, before).commit()
            throw NativeDocumentError("DOWNLOAD_ACTION_DURABILITY_ERROR")
        }
        issued = next
        val prefs = context.getSharedPreferences("kiwix-download-notifications", Context.MODE_PRIVATE)
        val previous = prefs.getStringSet("jobs", emptySet()).orEmpty()
        val shown = mutableSetOf<String>()
        val signatures = mutableMapOf<String, String>()
        val labelSignature = labels.toString()
        if (enabled) for (job in entries) {
            if (job.optString("status") !in shownStates) continue
            val id = job.getString("id")
            shown.add(id)
            // Only content that changed reaches the system notification service.
            val signature = labelSignature + "\u0000" + activeJobs + "\u0000" + job
            signatures[id] = signature
            if (rendered[id] == signature) continue
            manager.notify(TAG + id, 0, builder(context, job))
        }
        rendered.clear()
        rendered.putAll(signatures)
        contentIntents.keys.retainAll(shown)
        actionIntents.keys.retainAll { it.substringBefore('|') in shown }
        for (id in previous - shown) manager.cancel(TAG + id, 0)
        if (shown != previous) prefs.edit().putStringSet("jobs", shown).apply()
    }

    fun revoke(context: Context): Unit = synchronized(AppEntryPoints) {
        val authority = context.getSharedPreferences("kiwix-mobile-entries", Context.MODE_PRIVATE)
        val before = authority.getString("entries", "[]") ?: "[]"
        val issuedBefore = authority.getString(ISSUED, "{}") ?: "{}"
        val pending = JSONArray(before)
        val retained = JSONArray()
        for (index in 0 until pending.length()) {
            val entry = pending.getJSONObject(index)
            if (entry.optString("kind") != "download-action") retained.put(entry)
        }
        if (retained.length() != pending.length() || issuedBefore != "{}") {
            if (!authority.edit().putString("entries", retained.toString()).putString(ISSUED, "{}").commit()) {
                authority.edit().putString("entries", before).putString(ISSUED, issuedBefore).commit()
                throw NativeDocumentError("DOWNLOAD_ACTION_DURABILITY_ERROR")
            }
        }
        val prefs = context.getSharedPreferences("kiwix-download-notifications", Context.MODE_PRIVATE)
        val previous = prefs.getStringSet("jobs", emptySet()).orEmpty()
        val manager = context.getSystemService(NotificationManager::class.java)
        for (id in previous) manager.cancel(TAG + id, 0)
        if (previous.isNotEmpty()) prefs.edit().putStringSet("jobs", emptySet()).apply()
        activeJobs = 0
        issued = JSONObject()
        rendered.clear()
        actionIntents.clear()
        contentIntents.clear()
    }

    /** One package-manager lookup per process; every intent is a clone. */
    private fun launchIntent(context: Context): Intent? {
        if (!launchResolved) {
            launchTemplate = context.packageManager.getLaunchIntentForPackage(context.packageName)
            launchResolved = true
        }
        return launchTemplate
    }

    private fun builder(context: Context, job: JSONObject): Notification {
        if (Build.VERSION.SDK_INT >= 36 && active(job)) return liveBuilder(context, job)
        return compatBuilder(context, job).build()
    }

    /** Android 16 progress-centric Live Update. Promoted only while transferring. */
    private fun liveBuilder(context: Context, job: JSONObject): Notification {
        val total = job.optLong("total", -1)
        val progress = job.optLong("downloaded", 0)
        val percent = if (total > 0) {
            ((progress.coerceAtMost(total).toDouble() / total) * 100).toInt()
        } else 0
        val style = Notification.ProgressStyle()
            .setProgress(percent)
            .setProgressIndeterminate(total <= 0)
        val builder = Notification.Builder(context, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle(job.getString("fileName"))
            .setContentText(labels.optString(job.getString("status"), job.getString("status")))
            .setOnlyAlertOnce(true)
            .setOngoing(true)
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setCategory(Notification.CATEGORY_PROGRESS)
            .setStyle(style)
        // The promotion setter is API 36.1; the documented extra also requests
        // promotion on API 36.0 (where the platform simply does not promote).
        // Do not colorize: colorized notifications are ineligible for promotion.
        builder.addExtras(Bundle().apply { putBoolean("android.requestPromotedOngoing", true) })
        if (activeCount() > 1) builder.setGroup(GROUP)
        for (control in actions(phase(job.getString("status")))) {
            val icon = if (control == "pause") android.R.drawable.ic_media_pause
            else if (control == "cancel") android.R.drawable.ic_menu_close_clear_cancel
            else android.R.drawable.ic_media_play
            builder.addAction(Notification.Action.Builder(
                Icon.createWithResource(context, icon),
                labels.getString(control),
                action(context, job.getString("id"), control)).build())
        }
        launchIntent(context)?.let { launch ->
            val id = job.getString("id")
            val pending = contentIntents.getOrPut(id) {
                PendingIntent.getActivity(context, id.hashCode(), Intent(launch),
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
            }
            builder.setContentIntent(pending)
        }
        return builder.build()
    }

    private fun compatBuilder(context: Context, job: JSONObject): NotificationCompat.Builder {
        val status = job.getString("status")
        val builder = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle(job.getString("fileName"))
            .setContentText(labels.optString(status, status))
            .setOnlyAlertOnce(true).setOngoing(active(job))
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .setCategory(NotificationCompat.CATEGORY_PROGRESS)
        if (activeCount() > 1) builder.setGroup(GROUP)
        val total = job.optLong("total", -1)
        val progress = job.optLong("downloaded", 0)
        if (active(job)) {
            if (total > 0) builder.setProgress(10000,
                ((progress.coerceAtMost(total).toDouble() / total) * 10000).toInt(), false)
            else builder.setProgress(0, 0, true)
        }
        for (control in actions(phase(status))) builder.addAction(
            if (control == "pause") android.R.drawable.ic_media_pause
            else if (control == "cancel") android.R.drawable.ic_menu_close_clear_cancel
            else android.R.drawable.ic_media_play,
            labels.getString(control), action(context, job.getString("id"), control))
        launchIntent(context)?.let { launch ->
            val id = job.getString("id")
            val pending = contentIntents.getOrPut(id) {
                PendingIntent.getActivity(context, id.hashCode(), Intent(launch),
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
            }
            builder.setContentIntent(pending)
        }
        return builder
    }

    private fun action(context: Context, id: String, action: String): PendingIntent {
        val launch = launchIntent(context) ?: throw NativeDocumentError("APPLICATION_ACTIVITY_UNAVAILABLE")
        val identity = token(issued.getJSONObject(id), action)
        val key = "$id|$action"
        // A phase change rotates the nonce, which rotates this intent identity.
        actionIntents[key]?.takeIf { it.first == identity }?.let { return it.second }
        val intent = Intent(launch)
        intent.action = ACTION
        intent.data = Uri.parse("kiwix-notification://download/" + identity)
        intent.putExtra("downloadId", id).putExtra("downloadAction", action)
            .putExtra("downloadToken", identity)
        val pending = PendingIntent.getActivity(context, 0, intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        actionIntents[key] = identity to pending
        return pending
    }

    fun receiveAction(context: Context, intent: Intent): JSONObject? {
        val id = intent.getStringExtra("downloadId") ?: return null
        val action = intent.getStringExtra("downloadAction") ?: return null
        val identity = intent.getStringExtra("downloadToken") ?: return null
        val prefs = context.getSharedPreferences("kiwix-mobile-entries", Context.MODE_PRIVATE)
        val record = JSONObject(prefs.getString(ISSUED, "{}") ?: "{}").optJSONObject(id) ?: return null
        val currentPhase = record.getString("phase")
        if (action !in actions(currentPhase) || identity != token(record, action)) return null
        return JSONObject().put("id", id).put("action", action).put("phase", currentPhase).put("token", identity)
    }

    /** Pure preparation; caller commits nonce retirement AND entry removal atomically. */
    fun retiredAction(before: String, entry: JSONObject): String {
        val value = JSONObject(entry.getString("value"))
        val records = JSONObject(before)
        val record = records.optJSONObject(value.getString("id")) ?: return before
        val action = value.getString("action")
        if (value.getString("token") == token(record, action)) record.put("nonce", UUID.randomUUID().toString())
        return records.toString()
    }
}
