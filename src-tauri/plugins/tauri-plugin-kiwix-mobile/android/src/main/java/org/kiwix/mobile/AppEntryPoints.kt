package org.kiwix.mobile

import android.content.Context
import android.content.Intent
import android.net.Uri
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID
import java.util.concurrent.Executor

/**
 * One-shot platform entry requests (PROCESS_TEXT, kiwix:// links and dynamic
 * shortcuts). The WebView is not loaded when these intents arrive, so they are
 * queued in SharedPreferences until the trusted shell asks for them and
 * acknowledges each one after acting. Nothing here grants storage or network
 * authority; the shell applies its normal privacy and lifecycle gates.
 */
internal object AppEntryPoints {
    const val NEW_TAB = "org.kiwix.workbench.NEW_TAB"
    const val GET_CONTENT = "org.kiwix.workbench.GET_CONTENT"
    private const val PREFS = "kiwix-mobile-entries"
    private const val KEY = "entries"
    private const val HANDLED = "org.kiwix.mobile.ENTRY_HANDLED"
    private const val MAX_ENTRIES = 32
    private const val MAX_VALUE = 4096

    private class Entry(val id: String, val kind: String, val value: String)
    /** Entries whose durable queue write is still in flight. */
    private val pendingEntries = mutableListOf<Entry>()

    /** Queues one entry. [then] runs on the durable executor's thread with true
     * only after the queue write committed, so the shell never sees an entry
     * that could still be lost on a crash. */
    @Synchronized fun receive(context: Context, intent: Intent?, durable: Executor, then: (Boolean) -> Unit) {
        if (intent == null || intent.getBooleanExtra(HANDLED, false)) { then(false); return }
        val entry = when {
            intent.action == Intent.ACTION_PROCESS_TEXT -> processText(intent)
            intent.action == DownloadNotifications.ACTION -> downloadAction(context, intent)
            intent.action == NEW_TAB -> Entry(UUID.randomUUID().toString(), "new-tab", "")
            intent.action == GET_CONTENT -> Entry(UUID.randomUUID().toString(), "get-content", "")
            intent.action == Intent.ACTION_VIEW -> deepLink(intent.data)
            else -> null
        } ?: run { then(false); return }
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val pending = JSONArray(prefs.getString(KEY, "[]") ?: "[]")
        for (index in 0 until pending.length()) {
            val existing = pending.getJSONObject(index)
            if (existing.optString("kind") == entry.kind &&
                existing.optString("value") == entry.value) { then(false); return }
        }
        if (pendingEntries.any { it.kind == entry.kind && it.value == entry.value }) { then(false); return }
        if (pending.length() + pendingEntries.size >= MAX_ENTRIES) { then(false); return }
        val record = JSONObject().put("requestId", entry.id)
            .put("kind", entry.kind).put("value", entry.value)
        pendingEntries.add(entry)
        durable.execute {
            val saved = synchronized(this) {
                val current = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                val before = current.getString(KEY, "[]") ?: "[]"
                val queue = JSONArray(before)
                val duplicate = (0 until queue.length()).any {
                    val existing = queue.getJSONObject(it)
                    existing.optString("kind") == entry.kind && existing.optString("value") == entry.value
                }
                val stored = duplicate ||
                    current.edit().putString(KEY, JSONArray(before).put(record).toString()).commit()
                if (!stored) current.edit().putString(KEY, before).commit()
                pendingEntries.remove(entry)
                stored
            }
            then(saved)
        }
    }
    private fun downloadAction(context: Context, intent: Intent): Entry? {
        val value = DownloadNotifications.receiveAction(context, intent) ?: return null
        return Entry(value.getString("token"), "download-action", value.toString())
    }


    private fun processText(intent: Intent): Entry? {
        val text = intent.getCharSequenceExtra(Intent.EXTRA_PROCESS_TEXT)?.toString()
            ?.replace("\u0000", "")?.trim()?.take(MAX_VALUE)
        if (text.isNullOrEmpty()) return null
        return Entry(UUID.randomUUID().toString(), "search", text)
    }

    private fun deepLink(uri: Uri?): Entry? {
        if (uri == null) return null
        return when (uri.scheme?.lowercase()) {
            "kiwix" -> kiwixLink(uri)
            "zim" -> articleLink(uri)
            else -> null
        }
    }

    private fun kiwixLink(uri: Uri): Entry? {
        val route = (uri.host ?: uri.path?.trimStart('/'))?.lowercase()
        return when (route) {
            "library", "" -> Entry(UUID.randomUUID().toString(), "library", "")
            "bookmarks" -> Entry(UUID.randomUUID().toString(), "bookmarks", "")
            "voice-search" -> Entry(UUID.randomUUID().toString(), "voice-search", "")
            "zimhost" -> Entry(UUID.randomUUID().toString(), "sharing", "")
            "search" -> {
                val query = (uri.getQueryParameter("q") ?: uri.getQueryParameter("query"))
                    ?.replace("\u0000", "")?.trim()?.take(MAX_VALUE).orEmpty()
                Entry(UUID.randomUUID().toString(), "search", query)
            }
            else -> null
        }
    }

    /**
     * A pinned article shortcut carries `zim://<archive-uuid>/<encoded path>`.
     * Rust validates and canonicalizes it before the shell sees it; this layer
     * only bounds the payload and refuses a non-canonical archive identity.
     */
    private fun articleLink(uri: Uri): Entry? {
        val rawHost = uri.host ?: return null
        val bookId = try {
            UUID.fromString(rawHost).toString()
        } catch (_: IllegalArgumentException) {
            return null
        }
        val path = uri.encodedPath?.trimStart('/').orEmpty()
        val suffix = buildString {
            uri.encodedQuery?.let { append('?').append(it) }
            uri.encodedFragment?.let { append('#').append(it) }
        }
        if (path.length + suffix.length > MAX_VALUE) return null
        val link = "zim://$bookId/" + path + suffix
        if (link.length > MAX_VALUE) return null
        return Entry(UUID.randomUUID().toString(), "article", link)
    }

    /** Marks an intent whose entry is durably queued. Call on the main thread. */
    fun markHandled(intent: Intent?) { intent?.putExtra(HANDLED, true) }

    fun take(context: Context): JSONArray = synchronized(this) {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        JSONArray(prefs.getString(KEY, "[]") ?: "[]")
    }

    /** Removes one acknowledged entry; unknown IDs are already satisfied.
     * [then] runs on the durable executor's thread once the write settled. */
    fun acknowledge(context: Context, id: String, durable: Executor, then: (Exception?) -> Unit) {
        durable.execute {
            val failure = try {
                synchronized(this) { acknowledgeLocked(context, id) }
                null
            } catch (error: Exception) { error }
            then(failure)
        }
    }

    private fun acknowledgeLocked(context: Context, id: String) {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val before = prefs.getString(KEY, "[]") ?: "[]"
        val pending = JSONArray(before)
        val retained = JSONArray()
        var found = false
        for (index in 0 until pending.length()) {
            val entry = pending.getJSONObject(index)
            if (entry.optString("requestId") == id) found = true
            else retained.put(entry)
        }
        if (!found) return
        val issuedBefore = prefs.getString(DownloadNotifications.ISSUED, "{}") ?: "{}"
        var issuedAfter = issuedBefore
        for (index in 0 until pending.length()) {
            val entry = pending.getJSONObject(index)
            if (entry.optString("requestId") == id && entry.optString("kind") == "download-action")
                issuedAfter = DownloadNotifications.retiredAction(issuedAfter, entry)
        }
        val editor = prefs.edit().putString(KEY, retained.toString())
            .putString(DownloadNotifications.ISSUED, issuedAfter)
        if (!editor.commit()) {
            prefs.edit().putString(KEY, before).putString(DownloadNotifications.ISSUED, issuedBefore).commit()
            throw IllegalStateException("APP_ENTRY_DURABILITY_ERROR")
        }
    }
}
