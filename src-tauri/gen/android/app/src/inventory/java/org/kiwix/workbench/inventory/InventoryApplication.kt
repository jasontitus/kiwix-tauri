package org.kiwix.workbench.inventory

import android.app.Application
import android.content.pm.ApplicationInfo

/** Platform-only target code required by Android instrumentation; no reader startup. */
class InventoryApplication : Application() {
  override fun onCreate() {
    super.onCreate()
    check(packageName == "org.kiwix.workbench.inventory") {
      "Hardware inventory must use its separate application package"
    }
    check(applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0) {
      "Hardware inventory requires a debuggable target"
    }
  }
}
