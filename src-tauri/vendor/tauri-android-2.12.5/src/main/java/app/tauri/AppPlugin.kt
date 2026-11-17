// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

package app.tauri

import android.app.Activity
import android.webkit.WebView
import java.lang.ref.WeakReference
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Plugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject

@TauriPlugin
class AppPlugin(initialActivity: Activity): Plugin(initialActivity) {
  private val BACK_BUTTON_EVENT = "back-button"
  private var activity = WeakReference<Activity>(initialActivity)
  private var webView = WeakReference<WebView>(null)
  private var backCallback: OnBackPressedCallback? = null

  init { bindActivity(initialActivity as AppCompatActivity) }

  private fun bindActivity(owner: AppCompatActivity) {
    if (owner.isDestroyed) return
    if (activity.get() !== owner || backCallback == null) {
      backCallback?.remove()
      webView.clear()
      activity = WeakReference(owner)
      val callback = object : OnBackPressedCallback(true) {
        override fun handleOnBackPressed() {
          val current = activity.get() as? AppCompatActivity ?: return
          if (current.isDestroyed || current.isFinishing) return
          val view = webView.get()
          if (hasListener(BACK_BUTTON_EVENT)) {
            trigger(BACK_BUTTON_EVENT, JSObject().put("canGoBack", view?.canGoBack() ?: false))
          } else if (view?.canGoBack() == true) {
            view.goBack()
          } else {
            isEnabled = false
            try { current.onBackPressedDispatcher.onBackPressed() }
            finally { isEnabled = true }
          }
        }
      }
      backCallback = callback
      owner.onBackPressedDispatcher.addCallback(owner, callback)
    }
  }

  override fun onResume(activity: AppCompatActivity) { bindActivity(activity) }

  override fun onDestroy(activity: AppCompatActivity) {
    if (this.activity.get() !== activity) return
    backCallback?.remove()
    backCallback = null
    webView.clear()
    this.activity.clear()
  }

  override fun load(webView: WebView) {
    val owner = activity.get() as? AppCompatActivity ?: return
    bindWebView(owner, webView)
  }

  /** Called for every creation, including replacement views after initial plugin load. */
  fun bindWebView(owner: AppCompatActivity, view: WebView) {
    if (owner.isDestroyed || owner.isFinishing) return
    bindActivity(owner)
    if (activity.get() === owner) webView = WeakReference(view)
  }

  // Activity lifetime ends gracefully; the Android process and native runtime remain OS-owned.
  @Command
  fun exit(invoke: Invoke) {
    val current = activity.get()
    if (current == null || current.isDestroyed) {
      invoke.reject("ACTIVITY_UNAVAILABLE")
      return
    }
    invoke.resolve()
    current.finishAffinity()
  }
}
