package org.kiwix.mobile

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.graphics.Color
import android.graphics.pdf.PdfDocument
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import java.io.File
import java.io.FileOutputStream
import java.io.FilterOutputStream
import android.net.Uri
import android.os.Bundle
import android.os.CancellationSignal
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.print.PageRange
import android.print.PrintAttributes
import android.print.PrintDocumentAdapter
import android.print.PrintJob
import android.print.PrintManager
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import java.io.ByteArrayInputStream
import java.io.FilterInputStream
import java.lang.ref.WeakReference
import java.net.HttpURLConnection
import java.net.Proxy
import java.net.URL
import java.util.concurrent.ConcurrentHashMap

/** Supported public SDK export. Print uses Android's normal spool adapter;
 * sharing draws the captured live page into PdfDocument, with screen-layout
 * page slices (not print CSS or selectable/link-preserving Chromium printing).
 * No package-private print callbacks, reader WebView, JS interface or cookies. */
class NativeArticlePdf private constructor(activity: Activity, private var invoke: Invoke?, private val id: String, private val url: String, private val secret: String, private val name: String, private val share: Boolean, private val output: String, private val width: Int, private val capturedHeight: Int, private val cancelLabel: String) {
    companion object {
        private val main = Handler(Looper.getMainLooper())
        /** Serialized PDF writer; encoding and fsync never run on the UI thread. */
        private val writer = java.util.concurrent.Executors.newSingleThreadExecutor()
        /** Bounds visual-state frames: pageCount * ceil(pageHeight / viewport). */
        private const val MAX_CAPTURE_TILES = 4096L
        private val jobs = mutableMapOf<String, NativeArticlePdf>()
        @JvmStatic private external fun authorityCurrent(jobId: String, handedOff: Boolean): Boolean
        @JvmStatic private external fun commitHandoff(jobId: String, action: Runnable): Boolean
        fun commitExport(jobId: String, action: Runnable): Boolean = commitHandoff(jobId, action)
        fun cancelId(jobId: String) { jobs[jobId]?.retire("Article PDF export was cancelled") }
        private fun resourceLabel(uri: Uri): String {
            val path = uri.encodedPath ?: return "<article resource>"
            return if (path.startsWith("/.kiwix-pdf-")) "<article bootstrap>" else path.take(240)
        }
        fun start(activity: Activity, invoke: Invoke) {
            main.post {
                try {
                    val args = invoke.getArgs()
                    val id = args.getString("jobId") ?: throw IllegalArgumentException("Missing PDF job")
                    if (jobs.containsKey(id)) throw IllegalArgumentException("Duplicate PDF job")
                    val job = NativeArticlePdf(activity, invoke, id,
                        args.getString("url") ?: throw IllegalArgumentException("Missing article URL"),
                        args.getString("secret") ?: throw IllegalArgumentException("Missing article authority"),
                        args.getString("suggestedName") ?: "article.pdf",
                        args.optString("destination") == "share", args.optString("output"), args.optInt("width"), args.optInt("height"),
                        args.getJSONObject("labels").getString("cancel"))
                    jobs[id] = job
                    try { job.start() }
                    catch (error: Exception) { job.retire(error.message ?: "Cannot open native PDF export") }
                } catch (error: Exception) { invoke.reject(error.message ?: "Cannot open native PDF export") }
            }
        }
        fun cancel(invoke: Invoke) {
            main.post {
                val id = invoke.getArgs().optString("jobId")
                jobs[id]?.retire("Article PDF export was cancelled")
                invoke.resolve(JSObject())
            }
        }
    }
    private val activity = WeakReference(activity)
    private val context = activity.applicationContext
    private val debugTracing = (context.applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0
    private val origin = Uri.parse(url)
    private val connections = ConcurrentHashMap.newKeySet<HttpURLConnection>()
    private val signals = mutableSetOf<CancellationSignal>()
    @Volatile private var retired = false
    private var handedOff = false
    private var committing = false
    private var view: WebView? = null
    private var delegate: PrintDocumentAdapter? = null
    private var printJob: PrintJob? = null
    private var started = false
    private var host: FrameLayout? = null
    private var preview: AlertDialog? = null
    private var document: PdfDocument? = null
    private var nextPage = 0
    private var pageCount = 0
    private var pageHeight = 0
    private var documentHeight = 0
    private var viewportHeight = 0
    private var tileOffset = 0
    private var currentPage: PdfDocument.Page? = null
    @Volatile private var rendered = false
    private var finished = false
    private var visualSequence = 0L
    private val timeout = Runnable { retire("Article PDF export timed out") }

    private fun admitted() = !retired && (committing || authorityCurrent(id, handedOff))
    private fun trace(stage: String) {
        if (debugTracing)
            android.util.Log.d("KiwixArticlePdf", "job=$id stage=$stage page=$nextPage tile=$tileOffset scroll=${view?.scrollY}")
    }
    private fun start() {
        if (origin.scheme != "http" || origin.host != "127.0.0.1" || origin.port <= 0 || origin.userInfo != null || origin.fragment != null) {
            retire("Invalid native article origin"); return
        }
        if (!admitted()) { retire("Article PDF authority expired"); return }
        if (width !in 1..16384 || capturedHeight !in 1..10_000_000) { retire("Invalid live article capture dimensions"); return }
        if (share && cancelLabel.isBlank()) { retire("Localized PDF cancel label is required"); return }
        trace("start")
        main.postDelayed(timeout, if (share) 90_000L else 16 * 60_000L)
        val web = WebView(context)
        view = web
        web.setBackgroundColor(Color.WHITE)
        // WebView.draw captures native scroll chrome as well as page content.
        web.isVerticalScrollBarEnabled = false
        web.isHorizontalScrollBarEnabled = false
        web.overScrollMode = View.OVER_SCROLL_NEVER
        web.settings.apply {
            javaScriptEnabled = false
            domStorageEnabled = false
            allowFileAccess = false
            allowContentAccess = false
            setSupportMultipleWindows(false)
            mixedContentMode = android.webkit.WebSettings.MIXED_CONTENT_NEVER_ALLOW
            builtInZoomControls = false
        }
        if (share) {
            val owner = activity.get()
            val root = owner?.findViewById<ViewGroup>(android.R.id.content)
            if (owner == null || root == null) { retire("The article window has retired"); return }
            // A transparent/hidden ancestor can suppress the draw traversal a
            // visual-state fence needs. Use a real, cancellable native preview
            // while capturing, not a timer or an unacknowledged offscreen draw.
            val previewHeight = minOf(nativePdfPageHeight(width), root.height)
            if (previewHeight <= 0) { retire("The article window is not laid out"); return }
            val container = FrameLayout(owner).apply {
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
                descendantFocusability = ViewGroup.FOCUS_BLOCK_DESCENDANTS
                minimumWidth = width
                isClickable = false; isFocusable = false
            }
            host = container
            web.isFocusable = false
            web.isFocusableInTouchMode = false
            web.setOnTouchListener { _, _ -> true }
            container.addView(web, FrameLayout.LayoutParams(width, previewHeight))
            val dialog = AlertDialog.Builder(owner).setTitle(name).setView(container)
                .setNegativeButton(cancelLabel) { _, _ -> retire(null, userCancelled = true) }.create()
            dialog.setCanceledOnTouchOutside(false)
            dialog.setOnCancelListener { retire(null, userCancelled = true) }
            preview = dialog
            dialog.show()
            trace("preview-visible")
            web.setLayerType(View.LAYER_TYPE_SOFTWARE, null)
            web.settings.useWideViewPort = true
            web.settings.loadWithOverviewMode = false
            web.setInitialScale(100)
        }
        web.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean =
                retired || handedOff || !request.isForMainFrame || request.url.toString() != url
            override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse {
                val target = request.url
                if (retired || request.method != "GET" || target.scheme != origin.scheme || target.host != origin.host || target.port != origin.port || target.userInfo != null) return denied()
                // This is a cookie-free native fetcher, not WebView's shared
                // CookieManager. No redirect is ever followed with the secret.
                var connection: HttpURLConnection? = null
                try {
                    connection = URL(target.toString()).openConnection(Proxy.NO_PROXY) as HttpURLConnection
                    connection.instanceFollowRedirects = false
                    connection.connectTimeout = 15_000; connection.readTimeout = 15_000
                    connection.setRequestProperty("X-Kiwix-Pdf", secret)
                    connection.setRequestProperty("Cookie", "")
                    connections.add(connection)
                    val status = connection.responseCode
                    if (status != 200 || retired || connection.contentLengthLong > 16L * 1024 * 1024) {
                        connection.disconnect(); connections.remove(connection)
                        val reason = when {
                            status != 200 -> "HTTP $status"
                            retired -> "retired export"
                            else -> "16 MiB resource limit"
                        }
                        val logical = resourceLabel(target)
                        main.post { retire("An article resource could not be exported ($reason: $logical)") }
                        return denied()
                    }
                    val owned = connection
                    val stream = object : FilterInputStream(owned.inputStream) {
                        private var count = 0L
                        private fun account(n: Int) {
                            if (n > 0) {
                                count += n
                                if (count > 16L * 1024 * 1024 || retired) {
                                    // A failed read must not leave a reusable connection.
                                    owned.disconnect()
                                    throw java.io.IOException("Article resource limit exceeded")
                                }
                            }
                        }
                        override fun read(): Int { val byte = super.read(); if (byte >= 0) account(1); return byte }
                        override fun read(bytes: ByteArray, offset: Int, length: Int): Int { val count = super.read(bytes, offset, length); account(count); return count }
                        // Normal completion returns the connection to the keep-alive
                        // pool; only failures disconnect.
                        override fun close() { try { super.close() } finally { connections.remove(owned) } }
                    }
                    val type = (owned.contentType ?: "application/octet-stream").substringBefore(';')
                    val headers = owned.headerFields.filterKeys { it != null }.mapValues { it.value.joinToString(", ") }
                    return WebResourceResponse(type, "UTF-8", 200, "OK", headers, stream)
                } catch (error: Exception) {
                    connection?.disconnect(); connection?.let { connections.remove(it) }
                    val kind = error.javaClass.simpleName.take(80)
                    val logical = resourceLabel(target)
                    main.post { retire("An article resource could not be exported ($kind: $logical)") }
                    return denied()
                }
            }
            override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
                if (!retired) retire("The native article renderer could not load a resource: ${error.description}")
            }
            override fun onPageFinished(web: WebView, loaded: String) {
                if (retired || handedOff || loaded != url) return
                val owner = activity.get()
                if (owner == null || owner.isFinishing || owner.isDestroyed || !admitted()) { retire("Article PDF authority expired"); return }
                trace("page-finished")
                if (share) {
                    if (!started) {
                        started = true
                        val request = ++visualSequence
                        if (debugTracing) trace("initial-visual-request-$request")
                        web.postVisualStateCallback(request, object : WebView.VisualStateCallback() {
                            override fun onComplete(requestId: Long) {
                                if (debugTracing) trace("initial-visual-ready-$requestId")
                                web.postOnAnimation { beginDrawing(web) }
                            }
                        })
                        web.invalidate()
                    }
                    return
                }
                try {
                    delegate = web.createPrintDocumentAdapter(name.removeSuffix(".pdf"))
                    val manager = owner.getSystemService(Context.PRINT_SERVICE) as? PrintManager
                        ?: throw IllegalStateException("Android printing is unavailable")
                    if (owner.isFinishing || owner.isDestroyed) { retire("Article PDF authority expired"); return }
                    // Rust holds the short Mode -> Window -> Store retirement
                    // guards across this synchronous action. Do not reenter
                    // Rust from the action or synchronous framework callbacks.
                    committing = true
                    val opened = try {
                        commitHandoff(id, Runnable {
                            printJob = manager.print(name, adapter, PrintAttributes.Builder().setMediaSize(PrintAttributes.MediaSize.ISO_A4).build())
                                ?: throw IllegalStateException("Android could not open the PDF print UI")
                            handedOff = true
                        })
                    } finally { committing = false }
                    if (!opened) { retire("Article PDF authority expired"); return }
                    activity.clear()
                } catch (error: Exception) { retire(error.message ?: "Android could not open PDF export") }
            }
        }
        web.loadUrl(url)
    }
    private fun denied() = WebResourceResponse("text/plain", "UTF-8", 403, "Forbidden", mapOf("Cache-Control" to "no-store"), ByteArrayInputStream(ByteArray(0)))
    private fun beginDrawing(web: WebView) {
        if (!admitted()) { retire("Article PDF authority expired"); return }
        try {
            // Preserve the captured screen width and slice vertically onto A4.
            // This public SDK path intentionally does not claim WebView's
            // print-engine link/text tagging or CSS page-break fidelity.
            pageHeight = nativePdfPageHeight(width)
            @Suppress("DEPRECATION")
            val height = maxOf(capturedHeight.toLong(), kotlin.math.ceil(web.contentHeight.toDouble() * web.scale.toDouble()).toLong())
            if (height <= 0 || height > 10_000_000) throw IllegalStateException("Unsupported article PDF height")
            documentHeight = height.toInt()
            pageCount = ((height + pageHeight - 1) / pageHeight).toInt()
            if (pageCount !in 1..512) throw IllegalStateException("Article PDF exceeds the 512-page limit")
            val parent = host?.parent as? View ?: throw IllegalStateException("The article window has retired")
            viewportHeight = minOf(pageHeight, parent.height)
            if (viewportHeight <= 0 || parent.width < width) throw IllegalStateException("The article window cannot fit the capture viewport")
            // Every tile is a visual-state round trip plus an animation frame.
            val tilesPerPage = (pageHeight + viewportHeight - 1) / viewportHeight
            if (pageCount.toLong() * tilesPerPage > MAX_CAPTURE_TILES)
                throw IllegalStateException("Article PDF exceeds the capture tile limit")
            host?.layoutParams = host!!.layoutParams.apply { this.height = viewportHeight }
            web.layoutParams = FrameLayout.LayoutParams(width, viewportHeight)
            web.measure(View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(viewportHeight, View.MeasureSpec.EXACTLY))
            web.layout(0, 0, width, viewportHeight)
            if (debugTracing) trace("layout-ready-${width}x$viewportHeight-content-$documentHeight")
            document = PdfDocument()
            drawPage()
        } catch (error: Exception) { retire(error.message ?: "Cannot capture the live article") }
    }
    private fun drawPage() {
        if (!admitted()) { retire("Article PDF authority expired"); return }
        val web = view ?: return
        val top = nextPage * pageHeight + tileOffset
        // Scroll and wait for the visible viewport before each draw. This uses
        // only the public SDK and does not depend on global whole-document
        // drawing (which must be enabled before the app creates ANY WebView).
        web.scrollTo(0, top)
        val request = ++visualSequence
        if (debugTracing) trace("tile-visual-request-$request")
        web.postVisualStateCallback(request, object : WebView.VisualStateCallback() {
            override fun onComplete(requestId: Long) {
                if (debugTracing) trace("tile-visual-ready-$requestId")
                // The fence guarantees the following draw, not a reentrant
                // draw from inside the completion callback itself.
                web.postOnAnimation { drawVisiblePage(top) }
            }
        })
        web.invalidate()
    }
    private fun drawVisiblePage(top: Int) {
        if (!admitted()) { retire("Article PDF authority expired"); return }
        val pdf = document ?: return
        val web = view ?: return
        try {
            val page = currentPage ?: pdf.startPage(PdfDocument.PageInfo.Builder(596, 842, nextPage + 1).create()).also { currentPage = it }
            val tileHeight = nativePdfTileHeight(viewportHeight, pageHeight, tileOffset, documentHeight, top)
            val canvas = page.canvas
            canvas.save()
            try {
                canvas.translate(36f, 36f)
                val scale = 523.28f / width
                canvas.scale(scale, scale)
                canvas.translate(0f, tileOffset.toFloat())
                canvas.clipRect(0, 0, width, tileHeight)
                canvas.translate(0f, -nativePdfViewportOffset(top, web.scrollY).toFloat())
                web.draw(canvas)
                trace("tile-drawn")
            } finally { canvas.restore() }
            tileOffset += tileHeight
            if (tileOffset < pageHeight && nextPage * pageHeight + tileOffset < documentHeight) {
                main.post { drawPage() }; return
            }
            pdf.finishPage(page); currentPage = null
            trace("page-complete")
            nextPage += 1; tileOffset = 0
            if (nextPage < pageCount) { main.post { drawPage() }; return }
            val target = File(output)
            val root = context.cacheDir.canonicalFile.path + File.separator
            if (!target.canonicalPath.startsWith(root) || target.exists()) throw IllegalArgumentException("Invalid article PDF output")
            val descriptor = android.system.Os.open(target.path, android.system.OsConstants.O_WRONLY or
                android.system.OsConstants.O_CREAT or android.system.OsConstants.O_EXCL or
                android.system.OsConstants.O_NOFOLLOW, 384)
            // The worker owns the document and the descriptor from here; retire()
            // must not close either one behind its back.
            document = null
            trace("write-start")
            writer.execute {
                var failure: String? = null
                try {
                    FileOutputStream(android.system.Os.dup(descriptor)).use { file ->
                        val bounded = object : FilterOutputStream(file) {
                            private var bytes = 0L
                            private fun account(count: Int) {
                                bytes += count
                                if (bytes > 64L * 1024 * 1024 || !admitted()) throw java.io.IOException("Article PDF limit exceeded or export cancelled")
                            }
                            override fun write(value: Int) { account(1); out.write(value) }
                            override fun write(buffer: ByteArray, offset: Int, length: Int) { account(length); out.write(buffer, offset, length) }
                        }
                        pdf.writeTo(bounded)
                        file.fd.sync()
                    }
                    rendered = true
                } catch (error: Exception) {
                    failure = error.message ?: "Android live article PDF rendering failed"
                } finally {
                    try { pdf.close() }
                    catch (cleanup: Exception) { if (failure == null) failure = cleanup.message ?: "Closing the PDF document failed" }
                    try { android.system.Os.close(descriptor) } catch (_: Exception) {}
                }
                val outcome = failure
                main.post {
                    if (outcome == null) trace("write-complete")
                    retire(outcome)
                }
            }
        } catch (error: Exception) { retire(error.message ?: "Android live article PDF rendering failed") }
    }
    private val adapter = object : PrintDocumentAdapter() {
        override fun onStart() {
            if (!admitted()) { retire("Article PDF authority expired"); return }
            if (!started) { started = true; delegate?.onStart() }
        }
        override fun onLayout(old: PrintAttributes?, new: PrintAttributes, signal: CancellationSignal, callback: LayoutResultCallback, extras: Bundle?) {
            if (!admitted() || delegate == null) { callback.onLayoutCancelled(); retire("Article PDF authority expired"); return }
            signals.add(signal)
            try { delegate!!.onLayout(old, new, signal, callback, extras) }
            catch (error: Exception) { callback.onLayoutFailed(error.message); retire(error.message ?: "PDF layout failed") }
        }
        override fun onWrite(pages: Array<out PageRange>, destination: ParcelFileDescriptor, signal: CancellationSignal, callback: WriteResultCallback) {
            if (!admitted() || delegate == null) {
                try { destination.close() } catch (_: Exception) {}
                callback.onWriteCancelled(); retire("Article PDF authority expired"); return
            }
            signals.add(signal)
            try { delegate!!.onWrite(pages, destination, signal, callback) }
            catch (error: Exception) { callback.onWriteFailed(error.message); retire(error.message ?: "PDF writing failed") }
        }
        override fun onFinish() { retire(null) }
    }
    private fun retire(error: String?, userCancelled: Boolean = false) {
        if (retired) return
        retired = true
        trace(if (userCancelled) "retire-cancelled" else if (error == null) "retire-complete" else "retire-error")
        var failure = error
        main.removeCallbacks(timeout)
        for (signal in signals) signal.cancel()
        signals.clear()
        for (connection in connections) connection.disconnect()
        connections.clear()
        // PrintJob.cancel is a no-op for CREATED previews. Our independent
        // retired latch rejects every later request even in that state.
        if (error != null || userCancelled) printJob?.cancel()
        printJob = null
        val native = delegate; delegate = null
        if (!finished) { finished = true; if (started) try { native?.onFinish() } catch (_: Exception) {} }
        host?.let { container ->
            container.removeAllViews()
            (container.parent as? ViewGroup)?.removeView(container)
        }; host = null
        preview?.setOnCancelListener(null)
        preview?.dismiss(); preview = null
        view?.apply { stopLoading(); webViewClient = WebViewClient(); destroy() }
        val page = currentPage; currentPage = null
        try { if (page != null) document?.finishPage(page) }
        catch (cleanup: Exception) { if (failure == null) failure = cleanup.message ?: "Closing the PDF page failed" }
        try { document?.close() }
        catch (cleanup: Exception) { if (failure == null) failure = cleanup.message ?: "Closing the PDF document failed" }
        document = null
        view = null
        activity.clear()
        jobs.remove(id)
        val completion = invoke; invoke = null
        if (failure != null) completion?.reject(failure) else completion?.resolve(JSObject()
            .put("opened", handedOff).put("rendered", rendered).put("cancelled", userCancelled))
    }
}
