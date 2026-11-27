package org.kiwix.workbench

import android.os.Build
import android.os.Bundle
import android.view.View
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import app.tauri.plugin.PluginManager
import org.kiwix.mobile.KiwixMobilePlugin
import org.kiwix.mobile.NativeActivityHost

class MainActivity : TauriActivity(), NativeActivityHost {
  override val kiwixWindowId: Int get() = id
  private var ownedWebView: WebView? = null

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    ownedWebView = webView
    KiwixMobilePlugin.webViewCreated(this, webView)
    PluginManager.onAppWebViewCreated(this, webView)
  }

  override fun onDestroy() {
    try {
      super.onDestroy()
    } finally {
      // Wry has already emitted its ownership callbacks. Destroy the old Java
      // view even on configuration recreation, for which Wry creates a new one.
      KiwixMobilePlugin.activityDestroyed(this, ownedWebView)
      ownedWebView = null
    }
  }
  override fun onCreate(savedInstanceState: Bundle?) {
    // Saved state means Android is restoring this Activity, so its launch
    // intent was already delivered to an earlier incarnation.
    KiwixMobilePlugin.activityCreated(this, savedInstanceState != null)
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    val content = findViewById<View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val protectedAreas = WindowInsetsCompat.Type.systemBars() or
        WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime()
      val occupied = insets.getInsets(protectedAreas)
      view.setPadding(occupied.left, occupied.top, occupied.right, occupied.bottom)
      if (Build.VERSION.SDK_INT >= 30) {
        WindowInsetsCompat.Builder(insets)
          .setInsets(protectedAreas, Insets.NONE)
          .setDisplayCutout(null)
          .build()
      } else {
        // The native container already protects bars and keyboard. Translate
        // remaining descendant insets without applying either occlusion twice.
        WindowInsetsCompat.Builder(insets.inset(occupied.left, occupied.top, occupied.right, occupied.bottom))
          .setDisplayCutout(null)
          .build()
      }
    }
    ViewCompat.requestApplyInsets(content)
  }
}
