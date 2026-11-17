package org.kiwix.mobile

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.view.View
import android.widget.RemoteViews
import org.json.JSONArray

/**
 * Home-screen search affordance. The launcher owns update scheduling, so a
 * cold process is never started merely to redraw the widget: every update
 * reads the last shell-published snapshot from SharedPreferences. Archive
 * titles are never persisted while private or read-only mode is active.
 */
class KiwixSearchWidget : AppWidgetProvider() {
    override fun onUpdate(context: Context, manager: AppWidgetManager, appWidgetIds: IntArray) {
        if (appWidgetIds.isEmpty()) return
        manager.updateAppWidget(appWidgetIds, buildViews(context))
    }

    override fun onReceive(context: Context, intent: Intent) {
        super.onReceive(context, intent)
        if (intent.action == Intent.ACTION_LOCALE_CHANGED ||
            intent.action == "android.intent.action.APPLICATION_LOCALE_CHANGED") refresh(context)
    }

    override fun onAppWidgetOptionsChanged(
        context: Context,
        manager: AppWidgetManager,
        appWidgetId: Int,
        newOptions: Bundle,
    ) {
        manager.updateAppWidget(appWidgetId, buildViews(context))
    }

    companion object {
        private const val PREFS = "kiwix-mobile-widget"
        private const val KEY_TITLES = "titles"
        private const val KEY_RESTRICTED = "restricted"
        private const val MAX_TITLES = 3
        private const val MAX_TITLE = 120

        /** Persist the latest shell view and redraw every placed instance. */
        fun publish(context: Context, titles: List<String>, restricted: Boolean) {
            val clean = if (restricted) emptyList()
                else titles.asSequence()
                    .map { it.replace('\u0000', ' ').trim().take(MAX_TITLE) }
                    .filter { it.isNotEmpty() }
                    .take(MAX_TITLES)
                    .toList()
            val json = JSONArray(clean).toString()
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
                .putString(KEY_TITLES, json)
                .putBoolean(KEY_RESTRICTED, restricted)
                .apply()
            refresh(context)
        }

        fun refresh(context: Context) {
            val manager = AppWidgetManager.getInstance(context)
            val ids = manager.getAppWidgetIds(ComponentName(context, KiwixSearchWidget::class.java))
            if (ids.isNotEmpty()) manager.updateAppWidget(ids, buildViews(context))
        }

        private fun buildViews(base: Context): RemoteViews {
            val context = AppLocale.localized(base)
            val views = RemoteViews(context.packageName, R.layout.kiwix_search_widget)
            views.setInt(R.id.kiwix_widget_root, "setLayoutDirection", context.resources.configuration.layoutDirection)
            for ((id, resource) in listOf(
                R.id.kiwix_widget_search to R.string.widget_search_hint,
                R.id.kiwix_widget_library to R.string.widget_library,
                R.id.kiwix_widget_content to R.string.widget_get_content,
                R.id.kiwix_widget_microphone to R.string.widget_microphone,
                R.id.kiwix_widget_bookmarks to R.string.widget_bookmarks,
            )) {
                views.setTextViewText(id, context.getString(resource))
                views.setContentDescription(id, context.getString(resource))
            }
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            val restricted = prefs.getBoolean(KEY_RESTRICTED, true)
            val titles = if (restricted) emptyList() else {
                val raw = prefs.getString(KEY_TITLES, "[]") ?: "[]"
                val array = runCatching { JSONArray(raw) }.getOrNull()
                (0 until (array?.length() ?: 0))
                    .mapNotNull { array?.optString(it) }
                    .filter { it.isNotBlank() }
                    .take(MAX_TITLES)
            }
            val summary = when {
                restricted -> context.getString(R.string.widget_search_hint)
                titles.isEmpty() -> context.getString(R.string.widget_no_archives)
                else -> titles.joinToString(" · ")
            }
            views.setTextViewText(R.id.kiwix_widget_summary, summary)
            views.setViewVisibility(
                R.id.kiwix_widget_summary,
                if (restricted && titles.isEmpty()) View.GONE else View.VISIBLE,
            )
            // One package-manager lookup for every click target; each target is
            // still its own explicit intent with its own request code.
            val launch = context.packageManager.getLaunchIntentForPackage(context.packageName)
            if (launch != null) {
                views.setOnClickPendingIntent(R.id.kiwix_widget_search,
                    activityIntent(context, launch, 1, Intent.ACTION_VIEW, "kiwix://search"))
                views.setOnClickPendingIntent(R.id.kiwix_widget_library,
                    activityIntent(context, launch, 2, Intent.ACTION_VIEW, "kiwix://library"))
                views.setOnClickPendingIntent(R.id.kiwix_widget_content,
                    activityIntent(context, launch, 3, AppEntryPoints.GET_CONTENT, null))
                views.setOnClickPendingIntent(R.id.kiwix_widget_microphone,
                    activityIntent(context, launch, 4, Intent.ACTION_VIEW, "kiwix://voice-search"))
                views.setOnClickPendingIntent(R.id.kiwix_widget_bookmarks,
                    activityIntent(context, launch, 5, Intent.ACTION_VIEW, "kiwix://bookmarks"))
            }
            return views
        }

        /**
         * The launcher activity is looked up from the package instead of naming
         * an application class from this library. Explicit intents need no new
         * exported component or scheme filter.
         */
        private fun activityIntent(
            context: Context,
            launch: Intent,
            requestCode: Int,
            action: String,
            data: String?,
        ): PendingIntent {
            val intent = Intent(launch)
            intent.action = action
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            if (data != null) intent.data = Uri.parse(data)
            return PendingIntent.getActivity(
                context,
                requestCode,
                intent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        }
    }
}
