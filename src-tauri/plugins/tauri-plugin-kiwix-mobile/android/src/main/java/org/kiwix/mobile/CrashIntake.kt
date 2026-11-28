package org.kiwix.mobile

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.FileOutputStream
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone

/**
 * Bounded, redacted record for a native uncaught exception. The Rust panic hook
 * writes the same schema; this covers failures that never unwind through Rust.
 * The file lives under the app-private profile (`filesDir/kiwix/crash`), which
 * is exactly where the service reads the next-start report from.
 */
object CrashIntake {
    private const val MAX_FRAMES = 24
    private const val MAX_FRAME = 240
    private const val MAX_THREAD = 64
    private const val MAX_RECORD_BYTES = 32 * 1024
    private const val RECORD_DIR = "crash"
    private const val RECORD_FILE = "pending.json"
    private const val PREVIOUS_FILE = "previous.json"
    private const val TEMP_FILE = ".pending.json.tmp"

    /** SimpleDateFormat is not thread-safe; one formatter per crashing thread. */
    private val timestampFormat = object : ThreadLocal<SimpleDateFormat>() {
        override fun initialValue() = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss.SSSXXX", Locale.US).apply {
            timeZone = TimeZone.getTimeZone("UTC")
        }
    }

    @Volatile private var installed = false
    private val policy = CrashCapturePolicy()

    fun setEnabled(enabled: Boolean) { policy.setEnabled(enabled) }

    @Synchronized fun install(context: Context) {
        if (installed) return
        installed = true
        val appContext = context.applicationContext
        val previous = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, error ->
            try {
                policy.capture { write(appContext, thread.name ?: "unnamed", error) }
            } catch (_: Throwable) {
                // A crash reporter must never replace the original failure.
            }
            previous?.uncaughtException(thread, error)
        }
    }

    private fun write(context: Context, threadName: String, error: Throwable) {
        val frames = JSONArray()
        error.stackTrace.take(MAX_FRAMES).forEach { frame ->
            val file = frame.fileName ?: "unknown"
            // A frame keeps its spacing and is not scanned for long opaque runs:
            // a long dotted class name with a digit in it is not a secret.
            frames.put(CrashRedaction.truncate(
                CrashRedaction.redactLine("${frame.className}.${frame.methodName}($file:${frame.lineNumber})"),
                MAX_FRAME,
            ))
        }
        val record = JSONObject()
            .put("schema", 1)
            .put("capturedAt", timestamp())
            .put("appVersion", version(context))
            .put("platform", "android")
            .put("architecture", System.getProperty("os.arch") ?: "unknown")
            .put("phase", "application")
            .put("operation", "native-uncaught-exception")
            .put("thread", CrashRedaction.truncate(redact(threadName), MAX_THREAD))
            .put("message", redact(error.message ?: error.javaClass.name))
            .put("location", redact(location(error)))
            .put("stack", frames)
        var bytes = record.toString().toByteArray(Charsets.UTF_8)
        if (bytes.size > MAX_RECORD_BYTES) {
            val fallback = JSONObject()
                .put("schema", 1)
                .put("capturedAt", timestamp())
                .put("appVersion", version(context))
                .put("platform", "android")
                .put("architecture", System.getProperty("os.arch") ?: "unknown")
                .put("phase", "application")
                .put("operation", "native-uncaught-exception")
                .put("message", "[record exceeded size limit]")
                .put("stack", JSONArray())
            bytes = fallback.toString().toByteArray(Charsets.UTF_8)
        }
        val directory = File(context.filesDir, "kiwix/$RECORD_DIR")
        directory.mkdirs()
        val temp = File(directory, TEMP_FILE)
        FileOutputStream(temp).use { stream ->
            stream.write(bytes)
            stream.fd.sync()
        }
        val pending = File(directory, RECORD_FILE)
        if (pending.exists()) pending.renameTo(File(directory, PREVIOUS_FILE))
        temp.renameTo(pending)
    }

    private fun location(error: Throwable): String {
        val frame = error.stackTrace.firstOrNull() ?: return ""
        return "${frame.fileName ?: "unknown"}:${frame.lineNumber}"
    }

    private fun version(context: Context): String =
        try {
            context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: "unknown"
        } catch (_: Exception) {
            "unknown"
        }

    private fun timestamp(): String = timestampFormat.get().format(Date())

    /**
     * Redaction of one text field of the record: the canonical pipeline shared
     * with the Rust panic hook and the iOS intake ([CrashRedaction]).
     */
    internal fun redact(value: String): String = CrashRedaction.redact(value)
}
