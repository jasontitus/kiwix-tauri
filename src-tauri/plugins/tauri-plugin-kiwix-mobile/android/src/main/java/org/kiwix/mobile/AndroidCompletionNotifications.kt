package org.kiwix.mobile

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.util.AtomicFile
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import app.tauri.plugin.JSObject
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.UUID

/** Verified admission only. Transfer progress must never call this helper. */
object AndroidCompletionNotifications {
    private const val CHANNEL = "verified-archive-completions-v1"
    private const val ACTION = "org.kiwix.mobile.VERIFIED_ARCHIVE_COMPLETION"
    private const val EXTRA = "org.kiwix.mobile.COMPLETION_ID"
    private const val TAG = "verified-archive-completion"
    /** Acknowledged tombstones retained for replay-proofing recent Intents. */
    private const val TOMBSTONE_LIMIT = 32
    /** Admission bounds: the journal is a bounded recent history, never a log. */
    private const val NOTICE_LIMIT = 256
    private const val JOURNAL_LIMIT = 512 * 1024
    private var configured = false
    private var enabled = false
    private var protectedPolicy = true

    private fun file(context: Context) = AtomicFile(File(context.noBackupFilesDir, "verified-completion-notifications-v1.json"))
    private fun read(context: Context): JSONObject? {
        val journal = file(context)
        if (!journal.baseFile.exists()) return null
        return try {
            journal.openRead().use { JSONObject(it.readBytes().toString(Charsets.UTF_8)) }
        } catch (_: Exception) {
            // A truncated or foreign journal is absent state, never a launch crash.
            null
        }
    }
    private fun save(context: Context, state: JSONObject) {
        val journal = file(context)
        val stream = journal.startWrite()
        try {
            stream.write(state.toString().toByteArray(Charsets.UTF_8))
            journal.finishWrite(stream)
        } catch (error: Exception) {
            journal.failWrite(stream)
            throw error
        }
    }
    private fun emptyState() = JSONObject().put("profileId", UUID.randomUUID().toString())
        .put("enabled", false).put("protected", true).put("notices", JSONObject()).put("taps", JSONArray())
    private fun permitted(context: Context): Boolean =
        (Build.VERSION.SDK_INT < 33 || ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) &&
            NotificationManagerCompat.from(context).areNotificationsEnabled()

    @Synchronized fun capability(context: Context): JSObject = JSObject().put("available", true)

    /** Configuration is serialized with post/intake/ack, including disk commits. */
    @Synchronized fun configure(context: Context, args: JSONObject): JSObject {
        val nextEnabled = args.getBoolean("enabled")
        val nextProtected = args.getBoolean("protected")
        configured = false
        enabled = false
        protectedPolicy = true
        var state = read(context)
        if (!nextEnabled || nextProtected) {
            // Do not create state in protected operation. Retire existing notice
            // identities durably before acknowledging revocation to Rust.
            if (state != null) {
                val old = state.getJSONObject("notices")
                val manager = NotificationManagerCompat.from(context)
                old.keys().forEach { manager.cancel("$TAG:$it", 1) }
                state = emptyState()
                save(context, state)
            }
        } else {
            if (state == null) state = emptyState()
            state.put("enabled", true).put("protected", false)
            save(context, state)
        }
        enabled = nextEnabled
        protectedPolicy = nextProtected
        configured = true
        return JSObject().put("profileId", state?.getString("profileId") ?: JSONObject.NULL)
    }

    @Synchronized fun admitted(context: Context, args: JSONObject): JSObject {
        if (!configured || !enabled || protectedPolicy || !permitted(context)) return JSObject().put("accepted", false)
        val state = read(context) ?: return JSObject().put("accepted", false)
        if (!state.getBoolean("enabled") || state.getBoolean("protected")) return JSObject().put("accepted", false)
        val notice = args.getJSONObject("notice")
        val id = notice.getString("id")
        require(id.length in 1..1024 && notice.getString("receipt").length in 1..512)
        require(notice.getString("profileId") == state.getString("profileId"))
        require(notice.getString("installationId").isNotEmpty() && notice.getString("bookId").isNotEmpty())
        val notices = state.getJSONObject("notices")
        if (notices.has(id)) return JSObject().put("accepted", false)
        val launch = context.packageManager.getLaunchIntentForPackage(context.packageName)
            ?: throw IllegalStateException("Application launch Activity unavailable")
        launch.action = ACTION
        launch.data = Uri.parse("kiwix-internal://verified-completion/${Uri.encode(id)}")
        launch.putExtra(EXTRA, id)
        launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        val pending = PendingIntent.getActivity(context, 0, launch, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        if (Build.VERSION.SDK_INT >= 26) {
            context.getSystemService(NotificationManager::class.java).createNotificationChannel(
                NotificationChannel(CHANNEL, args.getString("channelName"), NotificationManager.IMPORTANCE_DEFAULT)
            )
        }
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle(args.getString("title"))
            .setContentText(args.getString("body"))
            .setContentIntent(pending).setAutoCancel(true)
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE).setOnlyAlertOnce(true).build()
        // Persist identity before delivery; never trust identity supplied by an Intent.
        notices.put(id, JSONObject(notice.toString()))
        // Admission is bounded so a long-lived install cannot grow the journal
        // without limit: the oldest admitted notice is evicted, never the new one.
        boundNotices(state, id)
        save(context, state)
        NotificationManagerCompat.from(context).notify("$TAG:$id", 1, notification)
        return JSObject().put("accepted", true)
    }

    /** Called for both Activity creation and onNewIntent, even before plugin setup. */
    @Synchronized fun receiveIntent(context: Context, intent: Intent?): Boolean {
        if (intent?.action != ACTION) return false
        val id = intent.getStringExtra(EXTRA) ?: return false
        val state = read(context) ?: return false
        if (!state.getBoolean("enabled") || state.getBoolean("protected")) return false
        val notice = state.getJSONObject("notices").optJSONObject(id) ?: return false
        if (notice.optBoolean("acknowledged", false)) return false
        val taps = state.getJSONArray("taps")
        for (index in 0 until taps.length()) if (taps.getJSONObject(index).getString("id") == id) return true
        taps.put(JSONObject(notice.toString()))
        save(context, state)
        return true
    }

    /** Non-destructive peek: the tab/session must be accepted before ack. */
    @Synchronized fun takeTap(context: Context): JSObject {
        if (!configured || !enabled || protectedPolicy) return JSObject().put("tap", JSONObject.NULL)
        val state = read(context) ?: return JSObject().put("tap", JSONObject.NULL)
        val taps = state.getJSONArray("taps")
        return JSObject().put("tap", if (taps.length() == 0) JSONObject.NULL else taps.getJSONObject(0))
    }

    @Synchronized fun acknowledgeTap(context: Context, args: JSONObject): JSObject {
        check(configured && enabled && !protectedPolicy) { "Notification handoff is protected" }
        val state = read(context) ?: return JSObject()
        val id = args.getString("id")
        val taps = state.getJSONArray("taps")
        val retained = JSONArray()
        for (index in 0 until taps.length()) {
            val tap = taps.getJSONObject(index)
            if (tap.getString("id") != id) retained.put(tap)
        }
        state.put("taps", retained)
        // Keep the notice as a durable delivered/consumed tombstone, but remove
        // tap authority: consumed notices cannot be replayed from stale Intents.
        state.getJSONObject("notices").optJSONObject(id)?.put("acknowledged", true)
        pruneNotices(state.getJSONObject("notices"))
        save(context, state)
        NotificationManagerCompat.from(context).cancel("$TAG:$id", 1)
        return JSObject()
    }

    /** Admission keeps the journal bounded: the notice just admitted is always
     * retained and the oldest admitted notices are evicted first. Android's
     * JSONObject iterates in insertion/document order, so the victim order is
     * deterministic. A notice that cannot fit the journal alone is rejected
     * before anything is persisted or delivered. */
    private fun boundNotices(state: JSONObject, admitted: String) {
        val notices = state.getJSONObject("notices")
        val oldest = notices.keys().asSequence().filter { it != admitted }.toList()
        var evicted = 0
        while (notices.length() > NOTICE_LIMIT) notices.remove(oldest[evicted++])
        while (journalBytes(state) > JOURNAL_LIMIT) {
            if (evicted == oldest.size)
                throw IllegalStateException("Verified completion notice exceeds the notification journal bound")
            notices.remove(oldest[evicted++])
        }
    }

    private fun journalBytes(state: JSONObject) = state.toString().toByteArray(Charsets.UTF_8).size

    /** Acknowledged notices are replay-proof tombstones, not an unbounded log:
     * keep a bounded recent window so intake and configure stay cheap. */
    private fun pruneNotices(notices: JSONObject) {
        val acknowledged = mutableListOf<String>()
        val keys = notices.keys()
        while (keys.hasNext()) {
            val key = keys.next()
            if (notices.optJSONObject(key)?.optBoolean("acknowledged", false) == true) acknowledged.add(key)
        }
        if (acknowledged.size <= TOMBSTONE_LIMIT) return
        for (index in 0 until acknowledged.size - TOMBSTONE_LIMIT) notices.remove(acknowledged[index])
    }
}
