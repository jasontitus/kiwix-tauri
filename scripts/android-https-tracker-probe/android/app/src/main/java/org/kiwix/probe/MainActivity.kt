package org.kiwix.probe

import android.app.Activity
import android.os.Bundle
import android.util.Log

class MainActivity : Activity() {
    private external fun run(context: android.content.Context, mode: String, dir: String): String

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val mode = intent.getStringExtra("mode") ?: "init"
        val dir = cacheDir.absolutePath
        Thread {
            System.loadLibrary("probe")
            val result = try { run(applicationContext, mode, dir) } catch (t: Throwable) { "THREW $t" }
            result.lines().forEach { Log.i("KiwixProbe", it) }
            Log.i("KiwixProbe", "DONE")
        }.start()
    }
}
