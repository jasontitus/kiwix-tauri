package org.kiwix.mobile

import android.content.Context
import android.content.res.Configuration
import android.os.LocaleList
import androidx.appcompat.app.AppCompatDelegate
import androidx.core.app.LocaleManagerCompat
import androidx.core.os.LocaleListCompat
import java.util.Locale

/** AppCompat persists locale selection on API 24–32 and uses LocaleManager on 33+. */
internal object AppLocale {
    private var applied: LocaleListCompat? = null
    private fun selection(context: Context) = if (android.os.Build.VERSION.SDK_INT >= 33)
        LocaleManagerCompat.getApplicationLocales(context) else applied ?: LocaleManagerCompat.getApplicationLocales(context)
    fun apply(context: Context, value: String, preservePlatform: Boolean = false) {
        if (preservePlatform && (value.isEmpty() || value == "system") &&
            !LocaleManagerCompat.getApplicationLocales(context).isEmpty) {
            KiwixSearchWidget.refresh(context)
            return
        }
        val tag = if (value.isEmpty() || value == "system") "" else {
            require(value.length <= 128 && !value.contains('_')) { "INVALID_APP_LOCALE" }
            val locale = Locale.Builder().setLanguageTag(value).build()
            require(locale.language.isNotEmpty() && locale.language != "und") { "INVALID_APP_LOCALE" }
            locale.toLanguageTag()
        }
        val locales = LocaleListCompat.forLanguageTags(tag)
        applied = locales
        if (AppCompatDelegate.getApplicationLocales() != locales)
            AppCompatDelegate.setApplicationLocales(locales)
        KiwixSearchWidget.refresh(context)
    }
    fun localized(context: Context): Context {
        val locales = selection(context)
        if (locales.isEmpty) return context
        val config = Configuration(context.resources.configuration)
        config.setLocales(LocaleList.forLanguageTags(locales.toLanguageTags()))
        config.setLayoutDirection(locales[0])
        return context.createConfigurationContext(config)
    }

    fun state(context: Context) = app.tauri.plugin.JSObject()
        .put("locale", selection(context).toLanguageTags())
        .put("effectiveLocale", localized(context).resources.configuration.locales[0].toLanguageTag())
}
