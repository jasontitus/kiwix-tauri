package org.kiwix.mobile

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Explicit, nonexported PendingIntent receiver; no file URI or writable grant enters the callback. */
class KiwixExportChoiceReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        intent.getStringExtra("token")?.let { TransferEvents.exportChosen(it) }
    }
}
