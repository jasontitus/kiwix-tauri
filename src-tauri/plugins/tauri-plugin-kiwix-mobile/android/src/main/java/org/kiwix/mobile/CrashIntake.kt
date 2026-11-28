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
    /** Characters a path may directly follow; mirrors `path_at` in src-tauri/src/crash.rs. */
    private const val PATH_BOUNDARY = "=([{\"',;:`"
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
        var text = replaceUrls(value)
        text = bearerPattern.replace(text) { "bearer [redacted]" }
        text = secretPattern.replace(text) { match ->
            val key = match.value.takeWhile { it != '=' && it != ':' }.trim()
            "$key=[redacted]"
        }
        text = redactPaths(text)
        text = opaquePattern.replace(text) { "[redacted]" }
        return whitespacePattern.replace(text, " ").trim()
    }

    /**
     * Where the redaction of a path that starts at [start] stops. The same rule
     * as `path_end` in src-tauri/src/crash.rs.
     *
     * Folder and file names contain spaces (`/storage/emulated/0/My Books/…`),
     * so whitespace cannot end a path: stopping there kept every later
     * component. Only text that cannot continue the path ends it: a closing
     * double quote, an angle bracket, a backtick, a line break, or the `: `
     * that introduces an error's cause. Prose between the path and that
     * delimiter is dropped with it; losing those words is the accepted price of
     * never keeping part of a path. An apostrophe does not end a path, because
     * names such as `Jane's Backup` contain one.
     */
    internal fun pathEnd(text: String, start: Int): Int {
        var end = start
        while (end < text.length) {
            val stop = when (text[end]) {
                '\n', '\r', '<', '>', '`' -> true
                // A path printed inside a quoted string escapes an embedded quote as `\"`.
                '"' -> {
                    var escapes = 0
                    var index = end - 1
                    while (index >= start && text[index] == '\\') { escapes++; index-- }
                    escapes % 2 == 0
                }
                ':' -> end + 1 < text.length && text[end + 1].isWhitespace()
                else -> false
            }
            if (stop) break
            end++
        }
        // The separator before the delimiter is not part of the path.
        while (end > start && text[end - 1].isWhitespace()) end--
        return end
    }

    /** The end of the path that starts at [index], or null when none starts there. */
    private fun pathAt(text: String, index: Int): Int? {
        if (index > 0 && !text[index - 1].isWhitespace() && text[index - 1] !in PATH_BOUNDARY) return null
        val current = text[index]
        val next = text.getOrNull(index + 1)
        val drive = (current in 'a'..'z' || current in 'A'..'Z') && next == ':' &&
            text.getOrNull(index + 2).let { it == '\\' || it == '/' }
        val rooted = current == '/' && next != null && next != '/' && !next.isWhitespace()
        val home = current == '~' && next == '/'
        val share = current == '\\' && next == '\\'
        return if (drive || rooted || home || share) pathEnd(text, index) else null
    }

    private fun redactPaths(value: String): String {
        val output = StringBuilder(value.length)
        var index = 0
        while (index < value.length) {
            val end = pathAt(value, index)
            if (end != null) {
                output.append("[path]")
                index = end
            } else {
                output.append(value[index])
                index++
            }
        }
        return output.toString()
    }

    /**
     * A URL ends at whitespace, except a `file:` URL: it is a path, may be
     * written with unencoded spaces, and is redacted to the end of that path.
     */
    private fun replaceUrls(value: String): String {
        val output = StringBuilder(value.length)
        var cursor = 0
        while (cursor <= value.length) {
            val match = urlPattern.find(value, cursor) ?: break
            val start = match.range.first
            val file = value.regionMatches(start, "file:/", 0, 6, ignoreCase = true)
            // The path of a file URL starts right after its colon.
            val end = if (file) pathEnd(value, start + 5) else match.range.last + 1
            output.append(value, cursor, start)
            output.append(if (file) "[url]" else safeUrl(value.substring(start, end)))
            cursor = end
        }
        output.append(value, cursor, value.length)
        return output.toString()
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
