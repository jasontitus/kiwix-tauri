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
    private const val MAX_MESSAGE = 2_000
    private const val MAX_FRAMES = 24
    private const val MAX_FRAME = 240
    private const val MAX_RECORD_BYTES = 32 * 1024
    private const val RECORD_DIR = "crash"
    private const val RECORD_FILE = "pending.json"
    private const val PREVIOUS_FILE = "previous.json"
    private const val TEMP_FILE = ".pending.json.tmp"

    private val urlPattern = Regex(
        """(?<![a-z0-9+.-])(?:[a-z][a-z0-9+.-]*://|(?:blob|content|data|file|mailto|tauri|tel|zim):(?!:))[^\s"<>`]*""",
        RegexOption.IGNORE_CASE
    )
    private val bearerPattern = Regex("""(?i)\bbearer\s+\S+""")
    private val secretPattern = Regex(
        """(?i)\b(authorization|access_token|refresh_token|api_key|apikey|password|passwd|cookie|token|secret|pin)\s*[:=]\s*("[^"]*"|'[^']*'|[^\s,;&)\]}]+)"""
    )
    private val pathPattern = Regex("""(?<![^\s=(\[{"',;:`])(?:/(?!/)|[a-zA-Z]:[\\/]|~/|\\\\)[^\s"<>`]+""")
    private val opaquePattern = Regex("""\b[A-Za-z0-9_+/=-]{40,}\b""")
    private val whitespacePattern = Regex("""\s+""")
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
            frames.put(
                redact("${frame.className}.${frame.methodName}($file:${frame.lineNumber})")
                    .take(MAX_FRAME)
            )
        }
        val record = JSONObject()
            .put("schema", 1)
            .put("capturedAt", timestamp())
            .put("appVersion", version(context))
            .put("platform", "android")
            .put("architecture", System.getProperty("os.arch") ?: "unknown")
            .put("phase", "application")
            .put("operation", "native-uncaught-exception")
            .put("thread", redact(threadName).take(64))
            .put("message", redact(error.message ?: error.javaClass.name).take(MAX_MESSAGE))
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

    internal fun redact(value: String): String {
        var text = urlPattern.replace(value) { match -> safeUrl(match.value) }
        text = bearerPattern.replace(text) { "bearer [redacted]" }
        text = secretPattern.replace(text) { match ->
            val key = match.value.takeWhile { it != '=' && it != ':' }.trim()
            "$key=[redacted]"
        }
        text = pathPattern.replace(text) { "[path]" }
        text = opaquePattern.replace(text) { "[redacted]" }
        return whitespacePattern.replace(text, " ").trim()
    }

    private fun safeUrl(raw: String): String {
        return try {
            val uri = java.net.URI(raw)
            val scheme = uri.scheme?.lowercase(Locale.ROOT)
            if (scheme != "http" && scheme != "https") return "[url]"
            val host = uri.host ?: return "[url]"
            val hostname = host.lowercase(Locale.ROOT).trim('[', ']', '.')
            val loopback = hostname == "localhost" || hostname.endsWith(".localhost") ||
                (hostname.startsWith("127.") && hostname.all { it == '.' || it in '0'..'9' }) ||
                (':' in hostname && hostname.endsWith("1") && hostname.dropLast(1).all { it == ':' || it == '0' })
            val path = uri.path.orEmpty()
            val nested = path.contains("://") ||
                listOf("mailto:", "data:", "file:", "content:", "zim:", "blob:", "tauri:", "tel:").any { path.contains(it) }
            if (loopback || path.contains("/.kiwix/") || nested) return "[reader-url]"
            val port = if (uri.port >= 0) ":${uri.port}" else ""
            "$scheme://$host$port${uri.rawPath.orEmpty()}"
        } catch (_: Exception) {
            "[url]"
        }
    }
}
