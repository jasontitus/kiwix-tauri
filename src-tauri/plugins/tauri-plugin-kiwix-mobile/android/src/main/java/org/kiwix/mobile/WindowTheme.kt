package org.kiwix.mobile

import android.app.Activity
import android.content.res.Configuration
import android.graphics.drawable.ColorDrawable
import android.os.Build
import androidx.core.view.WindowInsetsControllerCompat

/** [background] is an opaque ARGB colour; [dark] selects light system-bar icons. */
internal data class BarAppearance(val dark: Boolean, val background: Int)

/**
 * The Activity is edge to edge and pads its content by the system-bar insets,
 * so the status and navigation strips show the window background. Android
 * picks that background and the bar icon contrast from the system night mode
 * when the Activity is created; the shell's own theme setting ("system",
 * "light" or "dark") has to be applied on top.
 */
internal fun barAppearance(setting: String, systemNight: Boolean): BarAppearance {
    val dark = when (setting) {
        "dark" -> true
        "light" -> false
        else -> systemNight
    }
    // The page background (--paper) of the Android palette in src/platform.css.
    return BarAppearance(dark, if (dark) 0xFF131314.toInt() else 0xFFF8FAFD.toInt())
}

internal object WindowTheme {
    // androidx.activity's default navigation scrims, used where the bar is not transparent.
    private const val LIGHT_SCRIM = 0xE6FFFFFF.toInt()
    private const val DARK_SCRIM = 0x801B1B1B.toInt()

    /** Main thread only. [configuration] carries the system night mode to follow. */
    fun apply(activity: Activity, setting: String, configuration: Configuration) {
        val night = configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK ==
            Configuration.UI_MODE_NIGHT_YES
        val appearance = barAppearance(setting, night)
        val window = activity.window
        window.setBackgroundDrawable(ColorDrawable(appearance.background))
        WindowInsetsControllerCompat(window, window.decorView).apply {
            isAppearanceLightStatusBars = !appearance.dark
            isAppearanceLightNavigationBars = !appearance.dark
        }
        if (Build.VERSION.SDK_INT in 26..28) {
            // Android 8 and 9 draw the navigation bar as a scrim that was chosen
            // from the system night mode at creation; keep it readable under the
            // icon colour set above. Android 10+ keeps the bar transparent and
            // derives its contrast scrim from that icon colour itself.
            @Suppress("DEPRECATION")
            window.navigationBarColor = if (appearance.dark) DARK_SCRIM else LIGHT_SCRIM
        }
    }
}
