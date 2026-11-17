package org.kiwix.mobile

import android.app.Activity
import android.content.Intent
import android.content.pm.ShortcutInfo
import android.content.pm.ShortcutManager
import android.graphics.drawable.Icon
import android.net.Uri
import android.os.Build
import java.util.Locale
import java.util.UUID

/**
 * Launcher long-press shortcuts. They are dynamic so no static XML can drift
 * from the intake constants, and every shortcut reuses the Activity's own
 * exported entry point instead of adding another component.
 */
internal object AppShortcuts {
    private const val NEW_TAB_ID = "new-tab"
    private const val GET_CONTENT_ID = "get-content"
    private const val ARTICLE_ID_PREFIX = "article-"
    /** Native entry values are capped at 4096 characters; keep the whole link well below it. */
    private const val MAX_ARTICLE_PATH = 3072
    private const val MAX_TITLE = 120
    /** Activity recreation, including rotation, must not reassign the same set. */
    @Volatile private var installed = false

    fun install(activity: Activity) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.N_MR1) return
        if (installed) return
        val manager = activity.getSystemService(ShortcutManager::class.java) ?: return
        val iconId = activity.applicationInfo.icon
        if (iconId == 0) return
        installed = true
        val icon = Icon.createWithResource(activity, iconId)
        val shortcuts = listOf(
            ShortcutInfo.Builder(activity, NEW_TAB_ID)
                .setShortLabel(activity.getString(R.string.shortcut_new_tab_short))
                .setLongLabel(activity.getString(R.string.shortcut_new_tab_long))
                .setIcon(icon)
                .setIntent(Intent(activity, activity.javaClass).setAction(AppEntryPoints.NEW_TAB))
                .build(),
            ShortcutInfo.Builder(activity, GET_CONTENT_ID)
                .setShortLabel(activity.getString(R.string.shortcut_get_content_short))
                .setLongLabel(activity.getString(R.string.shortcut_get_content_long))
                .setIcon(icon)
                .setIntent(Intent(activity, activity.javaClass).setAction(AppEntryPoints.GET_CONTENT))
                .build(),
        )
        try {
            manager.dynamicShortcuts = shortcuts
        } catch (_: Exception) {
            // Rate-limited or disabled by the launcher; the app still works.
        }
    }

    /**
     * Ask the launcher to pin one article. The shortcut carries the same
     * `zim://<archive-uuid>/<path>` link the app already accepts, so opening it
     * goes through the ordinary native entry queue and privacy/lifecycle gates.
     * The user still confirms the pin in the launcher's dialog.
     */
    fun pinArticle(activity: Activity, bookId: String, path: String, title: String): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return false
        val manager = activity.getSystemService(ShortcutManager::class.java) ?: return false
        if (!manager.isRequestPinShortcutSupported) return false
        val iconId = activity.applicationInfo.icon
        if (iconId == 0) return false
        val canonicalId = try {
            UUID.fromString(bookId).toString()
        } catch (_: IllegalArgumentException) {
            return false
        }
        val location = path.trim().trimStart('/')
        if (location.length > MAX_ARTICLE_PATH) return false
        if (location.any { it.code < 32 || it.code == 127 }) return false
        val cleanTitle = title.replace('\u0000', ' ').trim().take(MAX_TITLE)
        if (cleanTitle.isEmpty()) return false
        val shortLabel = if (cleanTitle.length <= 40) cleanTitle else cleanTitle.take(39) + "…"
        val id = ARTICLE_ID_PREFIX + UUID.nameUUIDFromBytes(
            "$canonicalId\u0000$location".toByteArray(Charsets.UTF_8)
        ).toString().lowercase(Locale.ROOT)
        val link = if (location.isEmpty()) "zim://$canonicalId" else "zim://$canonicalId/$location"
        val intent = Intent(Intent.ACTION_VIEW, Uri.parse(link))
            .setClass(activity, activity.javaClass)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val shortcut = ShortcutInfo.Builder(activity, id)
            .setShortLabel(shortLabel)
            .setLongLabel(cleanTitle)
            .setIcon(Icon.createWithResource(activity, iconId))
            .setIntent(intent)
            .build()
        return try {
            manager.requestPinShortcut(shortcut, null)
        } catch (_: Exception) {
            // Disabled by policy or rate-limited by the launcher.
            false
        }
    }
}
