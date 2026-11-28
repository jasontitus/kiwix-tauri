package org.kiwix.mobile

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.CancellationSignal
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.system.ErrnoException
import android.system.Os
import android.system.OsConstants
import android.system.StructPollfd
import java.io.FileDescriptor
import java.io.InputStream
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.text.Normalizer
import java.util.Locale
import androidx.core.content.ContextCompat

internal class NativeDocumentError(val code: String, cause: Throwable? = null) : IOException(code, cause)

/** One operation-wide bound on a staging copy. A provider that keeps streaming or
 * keeps enumerating must never fill cacheDir or grow the staging tree without
 * limit, so both entries and bytes are charged across the whole selection. */
internal class DocumentCopyBudget(
    private val entryLimit: Int = ENTRY_LIMIT,
    private val byteLimit: Long = BYTE_LIMIT
) {
    private var entries = 0
    private var usedBytes = 0L

    /** Charge one entry this operation creates (a copied file or a made directory). */
    fun entry() {
        if (++entries > entryLimit) throw NativeDocumentError("STAGING_COPY_BUDGET_EXCEEDED")
    }

    /** Charge bytes before they are written, so the bound holds on disk. */
    fun charge(count: Long) {
        usedBytes += count
        if (usedBytes > byteLimit) throw NativeDocumentError("STAGING_COPY_BUDGET_EXCEEDED")
    }

    private companion object {
        /** Generous ceilings: a whole Kiwix library copy stays far below both. */
        const val ENTRY_LIMIT = 8192
        const val BYTE_LIMIT = 256L * 1024 * 1024 * 1024
    }
}

internal fun providerAcquisitionFailure(error: Exception): Exception = when (error) {
    is NativeDocumentError, is android.os.OperationCanceledException -> error
    is SecurityException -> NativeDocumentError("PROVIDER_PERMISSION_DENIED", error)
    is java.io.FileNotFoundException -> NativeDocumentError("PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT", error)
    else -> NativeDocumentError("PROVIDER_UNAVAILABLE_USE_COPY_IMPORT", error)
}

internal fun <T : Any> requireProviderArchiveAsset(asset: T?): T =
    asset ?: throw NativeDocumentError("PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT")

/** Poll before provider reads so a stalled nonseekable pipe cannot trap cancellation forever. */
internal class CancellableProviderInputStream(
    private val input: InputStream, fd: FileDescriptor, private val cancellation: CancellationSignal
) : InputStream() {
    private val descriptors = arrayOf(StructPollfd().apply {
        this.fd = fd
        events = OsConstants.POLLIN.toShort()
    })
    private val single = ByteArray(1)

    override fun read(): Int = if (read(single, 0, 1) == -1) -1 else single[0].toInt() and 255

    override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
        if (length == 0) return 0
        while (true) {
            cancellation.throwIfCanceled()
            try {
                if (Os.poll(descriptors, 250) > 0) {
                    cancellation.throwIfCanceled()
                    return input.read(buffer, offset, length)
                }
            } catch (error: ErrnoException) {
                if (error.errno != OsConstants.EINTR) throw error
            }
        }
    }

    override fun close() = input.close()
}

internal fun safeName(name: String): String {
    if (name.isEmpty() || name == "." || name == ".." || name.any {
            it == '/' || it == '\\' || it.code < 32 || it.code == 127
        }) throw NativeDocumentError("UNSAFE_DOCUMENT_NAME")
    return name
}

internal fun nameKey(name: String) = Normalizer.normalize(name, Normalizer.Form.NFC).lowercase(Locale.ROOT)

/** Recheck every existing component, not just canonical containment: symlinks are never accepted. */
internal fun ownedPath(context: Context, path: String): File {
    val file = File(path)
    if (!file.isAbsolute || file.absolutePath.split('/').any { it == "." || it == ".." })
        throw NativeDocumentError("UNSAFE_OWNED_PATH")
    val roots = listOf(context.filesDir, context.noBackupFilesDir, context.cacheDir).map { it.canonicalFile }
    val root = roots.firstOrNull { file.absolutePath.startsWith(it.path + File.separator) }
        ?: throw NativeDocumentError("PATH_OUTSIDE_APP_STORAGE")
    var cursor = root
    file.relativeTo(root).path.split(File.separatorChar).forEach { component ->
        safeName(component)
        cursor = File(cursor, component)
        try {
            if (OsConstants.S_ISLNK(Os.lstat(cursor.path).st_mode))
                throw NativeDocumentError("SYMLINK_NOT_ALLOWED")
        } catch (error: ErrnoException) {
            if (error.errno != OsConstants.ENOENT) throw error
        }
    }
    if (file.canonicalPath != file.absolutePath) throw NativeDocumentError("UNSAFE_OWNED_PATH")
    return file
}

internal fun syncDirectory(directory: File) {
    val fd = Os.open(directory.path, OsConstants.O_RDONLY or OsConstants.O_NOFOLLOW, 0)
    try {
        if (!OsConstants.S_ISDIR(Os.fstat(fd).st_mode))
            throw NativeDocumentError("NOT_DIRECTORY")
        Os.fsync(fd)
    } finally { Os.close(fd) }
}

/**
 * The files and directories one staging operation created itself. Cleanup
 * after a cancelled or failed copy goes through this record only, so a file
 * the operation did not create is never removed, and a directory is removed
 * only once it is empty.
 */
internal class StagedEntries {
    private val files = mutableListOf<File>()
    private val created = mutableListOf<File>()

    /** Directories in creation order. */
    val directories: List<File> get() = created

    /** Record a file this operation has just created exclusively. */
    fun file(file: File) { files.add(file) }

    /** Record a directory this operation has just created. */
    fun directory(directory: File) { created.add(directory) }

    fun retainedPaths(): List<String> = files.filter { it.exists() }.map { it.path }

    /** Remove everything recorded, newest first. Returns the files that remain. */
    fun remove(): List<String> {
        return retainedPaths()
    }
}

internal class OwnedDocuments(
    private val context: Context,
    destination: String,
    private val cancellation: CancellationSignal,
    private val progress: (Long, String) -> Unit
) {
    val root = ownedPath(context, destination)
    val paths = mutableListOf<String>()
    private val staged = StagedEntries()
    /** Absolute paths proven by [ownedPath] or created by this operation. */
    private val validatedDirectories = HashSet<String>()
    private var bytes = 0L
    private var reportedBytes = 0L
    private var reportedAt = 0L
    private val archiveName = Regex("(?i).+\\.zim(?:[a-z]{2})?")
    private val buffer = ByteArray(128 * 1024)
    /** One budget for the whole selection, never one per file or per directory. */
    private val budget = DocumentCopyBudget()

    init {
        if (root.exists()) {
            if (!root.isDirectory || root.list()?.isEmpty() != true)
                throw NativeDocumentError("STAGING_DIRECTORY_NOT_EMPTY")
        } else {
            // Parent creates the staging parent; native code never invents directories outside its operation.
            if (!root.parentFile.isDirectory || !root.mkdir())
                throw NativeDocumentError("STAGING_DIRECTORY_UNAVAILABLE")
            staged.directory(root)
        }
        validatedDirectories.add(root.absolutePath)
    }

    fun pick(uris: List<Uri>) {
        val names = HashSet<String>()
        for (uri in uris) {
            cancellation.throwIfCanceled()
            val name = documentName(uri)
            if (!archiveName.matches(name)) throw NativeDocumentError("NOT_AN_ARCHIVE_DOCUMENT")
            if (!names.add(nameKey(name))) throw NativeDocumentError("DUPLICATE_DOCUMENT_NAME")
            copy(uri, File(root, name))
        }
        finish()
    }

    fun tree(uri: Uri) {
        val id = DocumentsContract.getTreeDocumentId(uri)
        val seen = HashSet<String>()
        visit(uri, id, root, seen, 0)
        finish()
    }

    private fun visit(tree: Uri, id: String, directory: File, seen: MutableSet<String>, depth: Int) {
        cancellation.throwIfCanceled()
        if (depth > 128 || !seen.add(id)) throw NativeDocumentError("CYCLIC_OR_DEEP_DOCUMENT_TREE")
        val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, id)
        val projection = arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE)
        val names = HashSet<String>()
        val entries = mutableListOf<Triple<String, String, String>>()
        context.contentResolver.query(children, projection, null, null, null, cancellation)?.use { cursor ->
            while (cursor.moveToNext()) {
                cancellation.throwIfCanceled()
                val childId = cursor.getString(0) ?: throw NativeDocumentError("INVALID_DOCUMENT_METADATA")
                val name = safeName(cursor.getString(1) ?: throw NativeDocumentError("INVALID_DOCUMENT_METADATA"))
                val mime = cursor.getString(2) ?: ""
                if (mime == DocumentsContract.Document.MIME_TYPE_DIR || archiveName.matches(name)) {
                    if (!names.add(nameKey(name))) throw NativeDocumentError("DUPLICATE_DOCUMENT_NAME")
                    entries.add(Triple(childId, name, mime))
                }
            }
            if (cursor.extras.getBoolean(DocumentsContract.EXTRA_LOADING, false))
                throw NativeDocumentError("DOCUMENT_PROVIDER_STILL_LOADING")
            cursor.extras.getString(DocumentsContract.EXTRA_ERROR)?.let {
                throw NativeDocumentError("DOCUMENT_PROVIDER_ERROR", IOException(it))
            }
        } ?: throw NativeDocumentError("DOCUMENT_TREE_UNAVAILABLE")
        for ((childId, name, mime) in entries) {
            val target = File(directory, name)
            if (mime == DocumentsContract.Document.MIME_TYPE_DIR) {
                budget.entry()
                if (!target.mkdir()) throw NativeDocumentError("STAGING_DIRECTORY_UNAVAILABLE")
                staged.directory(target)
                validatedDirectories.add(target.absolutePath)
                visit(tree, childId, target, seen, depth + 1)
            } else copy(DocumentsContract.buildDocumentUriUsingTree(tree, childId), target)
        }
    }

    private fun documentName(uri: Uri): String {
        if (uri.scheme != "content") throw NativeDocumentError("DOCUMENT_URI_REQUIRED")
        context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null, cancellation)?.use {
            if (it.moveToFirst()) return safeName(it.getString(0) ?: throw NativeDocumentError("INVALID_DOCUMENT_METADATA"))
        }
        throw NativeDocumentError("DOCUMENT_METADATA_UNAVAILABLE")
    }

    /** The parent directory was validated before this copy; only the new leaf
     * can still be a symlink or an unsafe name. */
    private fun validateTarget(target: File) {
        safeName(target.name)
        val parent = target.parentFile ?: throw NativeDocumentError("UNSAFE_OWNED_PATH")
        if (parent.absolutePath !in validatedDirectories) throw NativeDocumentError("UNSAFE_OWNED_PATH")
        try {
            if (OsConstants.S_ISLNK(Os.lstat(target.path).st_mode))
                throw NativeDocumentError("SYMLINK_NOT_ALLOWED")
        } catch (error: ErrnoException) {
            if (error.errno != OsConstants.ENOENT) throw error
        }
    }

    private fun copy(uri: Uri, target: File) {
        cancellation.throwIfCanceled()
        validateTarget(target)
        budget.entry()
        context.contentResolver.openAssetFileDescriptor(uri, "r", cancellation)?.use { asset ->
            CancellableProviderInputStream(asset.createInputStream(), asset.parcelFileDescriptor.fileDescriptor,
                cancellation).use { input ->
                val fd = Os.open(target.path, OsConstants.O_WRONLY or OsConstants.O_CREAT or
                    OsConstants.O_EXCL or OsConstants.O_NOFOLLOW, 384 /* 0600 */)
                try {
                    staged.file(target)
                    FileOutputStream(fd).use { output ->
                        while (true) {
                            cancellation.throwIfCanceled()
                            val count = input.read(buffer)
                            if (count == -1) break
                            if (count == 0) continue
                            budget.charge(count.toLong())
                            output.write(buffer, 0, count)
                            bytes += count
                            // Progress is presentation only; one callback per MiB or
                            // 200 ms keeps a multi-GB copy off the event path.
                            val now = System.nanoTime()
                            if (bytes - reportedBytes >= PROGRESS_BYTES ||
                                now - reportedAt >= PROGRESS_NANOS) {
                                reportedBytes = bytes
                                reportedAt = now
                                progress(bytes, target.path)
                            }
                        }
                        output.flush()
                        try { output.fd.sync() } catch (error: IOException) {
                            throw NativeDocumentError("STAGING_DURABILITY_ERROR", error)
                        }
                    }
                } finally { Os.close(fd) }
            }
        } ?: throw NativeDocumentError("DOCUMENT_STREAM_UNAVAILABLE")
        if (bytes != reportedBytes) {
            reportedBytes = bytes
            progress(bytes, target.path)
        }
        paths.add(target.path)
    }

    private fun finish() {
        cancellation.throwIfCanceled()
        try {
            staged.directories.asReversed().forEach { if (it != root) syncDirectory(it) }
            syncDirectory(root)
            syncDirectory(root.parentFile)
        } catch (error: Exception) { throw NativeDocumentError("STAGING_DURABILITY_ERROR", error) }
    }

    fun retainedPaths(): List<String> = staged.retainedPaths()

    /**
     * Used when a copy is cancelled or fails before its result was delivered.
     * Only files created O_EXCL by this operation and directories it created can be removed.
     */
    fun cancelCleanup(): List<String> = staged.remove()

    private companion object {
        const val PROGRESS_BYTES = 1024L * 1024
        const val PROGRESS_NANOS = 200_000_000L
    }
}

/** Extension case is not cohort identity; exact provider names still open the documents. */
internal fun providerSplitPartsComplete(names: List<String>): Boolean {
    if (names.isEmpty() || names.size > 676 || !names[0].endsWith(".zimaa", true)) return false
    val base = names[0].dropLast(2)
    return names.withIndex().all { (index, name) ->
        val suffix = "${('a'.code + index / 26).toChar()}${('a'.code + index % 26).toChar()}"
        name.equals(base + suffix, ignoreCase = true)
    }
}

/** Opaque, native-created reading authority. Never accepts a caller URI or path. */
internal class ExternalDocuments(private val context: Context) {
    private data class Record(val uri: Uri, val folder: Boolean, val names: List<String>,
        val documents: List<String>? = null)
    private data class Lease(val reference: String, val uri: Uri,
        val descriptors: List<Pair<String, android.os.ParcelFileDescriptor>>)
    private val preferences = context.getSharedPreferences("external-document-references-v1", Context.MODE_PRIVATE)
    private val transient = mutableMapOf<String, Record>()
    private val leases = mutableMapOf<String, Lease>()
    private val moves = ExternalArchiveMove(context)

    @Synchronized fun prepareMove(reference: String): app.tauri.plugin.JSObject {
        if (!preferences.contains("ref:$reference"))
            throw NativeDocumentError("PROVIDER_MOVE_REQUIRES_PERSISTENT_REFERENCE")
        val record = record(reference)
        val entries = recordEntries(record, CancellationSignal())
        if (entries.map { it.first } != record.names) throw NativeDocumentError("PROVIDER_ARCHIVE_PARTS_CHANGED")
        return moves.prepare(reference, record.uri, entries)
    }

    @Synchronized fun deleteMove(reference: String, receipt: String, parts: org.json.JSONArray): app.tauri.plugin.JSObject {
        valid(reference)
        return moves.delete(reference, receipt, parts)
    }
    private val archiveName = Regex("(?i).+\\.zim(?:[a-z]{2})?")

    private fun valid(value: String) {
        if (runCatching { java.util.UUID.fromString(value).toString().equals(value, true) }.getOrDefault(false).not())
            throw NativeDocumentError("INVALID_PROVIDER_REFERENCE")
    }
    private fun encode(record: Record) = org.json.JSONObject().put("uri", record.uri.toString())
        .put("folder", record.folder).put("names", org.json.JSONArray(record.names)).apply {
            record.documents?.let { put("documents", org.json.JSONArray(it)) }
        }.toString()
    private fun decode(value: String): Record {
        val json = org.json.JSONObject(value)
        val uri = Uri.parse(json.getString("uri"))
        if (uri.scheme != "content" && uri.scheme != "file") throw NativeDocumentError("INVALID_PROVIDER_REFERENCE")
        val names = json.getJSONArray("names")
        val documents = json.optJSONArray("documents")?.let { array ->
            (0 until array.length()).map { index ->
                val document = Uri.parse(array.getString(index))
                if (document.scheme != "content" && document.scheme != "file")
                    throw NativeDocumentError("INVALID_PROVIDER_REFERENCE")
                document.toString()
            }
        }
        if (documents != null && documents.size != names.length())
            throw NativeDocumentError("INVALID_PROVIDER_REFERENCE")
        return Record(uri, json.getBoolean("folder"),
            (0 until names.length()).map { safeName(names.getString(it)) }, documents)
    }
    private fun record(reference: String): Record {
        valid(reference)
        return transient[reference] ?: preferences.getString("ref:$reference", null)?.let(::decode)
            ?: throw NativeDocumentError("PROVIDER_REFERENCE_REVOKED")
    }

    /** One index entry per stored record, so releasing a grant never decodes
     * every `ref:` JSON. A corrupt index is left alone: [referencedByStoredRecord]
     * then keeps using the safe record scan instead of trusting a partial list. */
    private fun indexReference(edit: android.content.SharedPreferences.Editor, uri: Uri, reference: String) {
        val key = "refs:$uri"
        val raw = preferences.getString(key, null)
        val refs = if (raw == null) org.json.JSONArray()
            else runCatching { org.json.JSONArray(raw) }.getOrNull() ?: return
        refs.put(reference)
        edit.putString(key, refs.toString())
    }

    private fun dropReferenceIndex(edit: android.content.SharedPreferences.Editor, uri: Uri, reference: String) {
        val key = "refs:$uri"
        val raw = preferences.getString(key, null) ?: return
        val refs = runCatching { org.json.JSONArray(raw) }.getOrNull()
        if (refs == null) { edit.remove(key); return }
        val retained = org.json.JSONArray()
        for (index in 0 until refs.length()) {
            val stored = runCatching { refs.getString(index) }.getOrNull() ?: continue
            if (stored != reference) retained.put(stored)
        }
        edit.putString(key, retained.toString())
    }

    /** Indexed lookup; a missing index is rebuilt from the records once. An
     * undecodable record always counts as a live reference. */
    private fun referencedByStoredRecord(uri: Uri): Boolean {
        val key = "refs:$uri"
        val raw = preferences.getString(key, null)
        if (raw != null) {
            val refs = runCatching { org.json.JSONArray(raw) }.getOrNull() ?: return true
            for (index in 0 until refs.length()) {
                val reference = runCatching { refs.getString(index) }.getOrNull() ?: return true
                if (preferences.contains("ref:$reference")) return true
            }
            return false
        }
        var unknown = false
        val rebuilt = LinkedHashMap<String, org.json.JSONArray>()
        for ((name, value) in preferences.all) {
            if (!name.startsWith("ref:") || value !is String) continue
            val stored = runCatching { decode(value) }.getOrNull()
            if (stored == null) { unknown = true; continue }
            rebuilt.getOrPut(stored.uri.toString()) { org.json.JSONArray() }.put(name.removePrefix("ref:"))
        }
        if (rebuilt.isNotEmpty() && !unknown) {
            val edit = preferences.edit()
            for ((stored, refs) in rebuilt) edit.putString("refs:$stored", refs.toString())
            edit.commit()
        }
        return unknown || rebuilt.containsKey(uri.toString())
    }
    private fun recordEntries(record: Record, cancellation: CancellationSignal): List<Pair<String, Uri>> {
        // Legacy folder records did not pin child document IDs. Keep the old
        // capability intact, but never infer replacement identities by name.
        if (record.folder && record.documents == null)
            throw NativeDocumentError("PROVIDER_REFERENCE_RESELECT_REQUIRED")
        val documents = record.documents ?: return entries(record.uri, record.folder, cancellation)
        if (documents.size != record.names.size) throw NativeDocumentError("PROVIDER_ARCHIVE_PARTS_CHANGED")
        // One listing of the pinned tree resolves every part that is its direct
        // child; only parts the listing does not cover keep a per-document query.
        val listed = if (DocumentsContract.isTreeUri(record.uri)) treeChildNames(record.uri, cancellation) else null
        return documents.mapIndexed { index, value ->
            val uri = Uri.parse(value)
            val name = when (uri.scheme) {
                "content" -> {
                    val childId = treeChildId(record.uri, uri)
                    (if (childId == null) null else listed?.get(childId))
                        ?: context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME),
                            null, null, null, cancellation)?.use {
                            if (it.moveToFirst()) it.getString(0) else null
                        } ?: throw NativeDocumentError("PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT")
                }
                "file" -> sharedFile(uri).takeIf { it.isFile }?.name
                    ?: throw NativeDocumentError("PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT")
                else -> throw NativeDocumentError("INVALID_PROVIDER_REFERENCE")
            }
            if (safeName(name) != record.names[index]) throw NativeDocumentError("PROVIDER_ARCHIVE_PARTS_CHANGED")
            record.names[index] to uri
        }
    }

    /** Direct children of the pinned tree in one provider query; null when the
     * listing itself failed, so the caller falls back to per-document metadata. */
    private fun treeChildNames(tree: Uri, cancellation: CancellationSignal): Map<String, String>? {
        val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree,
            DocumentsContract.getTreeDocumentId(tree))
        val names = HashMap<String, String>()
        return try {
            context.contentResolver.query(children, arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                DocumentsContract.Document.COLUMN_DISPLAY_NAME), null, null, null, cancellation)?.use { cursor ->
                while (cursor.moveToNext()) {
                    cancellation.throwIfCanceled()
                    val id = cursor.getString(0) ?: continue
                    val name = cursor.getString(1) ?: continue
                    names[id] = name
                }
                if (cursor.extras.getBoolean(DocumentsContract.EXTRA_LOADING, false) ||
                    cursor.extras.getString(DocumentsContract.EXTRA_ERROR) != null) null else names
            }
        } catch (error: android.os.OperationCanceledException) {
            throw error
        } catch (_: Exception) {
            null
        }
    }

    /** The only tree-document form this class mints: tree/<treeId>/document/<childId>. */
    private fun treeChildId(tree: Uri, document: Uri): String? {
        if (document.authority != tree.authority) return null
        val segments = document.pathSegments
        // DocumentsContract.PATH_TREE/PATH_DOCUMENT are hidden platform constants
        // absent from android.jar; their documented segment values are literals.
        if (segments.size != 4 || segments[0] != "tree" ||
            segments[1] != DocumentsContract.getTreeDocumentId(tree) ||
            segments[2] != "document") return null
        return segments[3]
    }
    private fun entries(uri: Uri, folder: Boolean, cancellation: CancellationSignal): List<Pair<String, Uri>> {
        if (!folder) {
            val name = context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null, cancellation)?.use {
                if (it.moveToFirst()) safeName(it.getString(0)) else null
            } ?: throw NativeDocumentError("PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT")
            if (!name.endsWith(".zim", true)) throw NativeDocumentError("PROVIDER_SPLIT_REQUIRES_FOLDER")
            return listOf(name to uri)
        }
        val children = DocumentsContract.buildChildDocumentsUriUsingTree(uri, DocumentsContract.getTreeDocumentId(uri))
        val result = mutableListOf<Pair<String, Uri>>()
        context.contentResolver.query(children, arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE),
            null, null, null, cancellation)?.use { cursor ->
            while (cursor.moveToNext()) {
                cancellation.throwIfCanceled()
                val name = safeName(cursor.getString(1))
                if (cursor.getString(2) != DocumentsContract.Document.MIME_TYPE_DIR && archiveName.matches(name))
                    result.add(name to DocumentsContract.buildDocumentUriUsingTree(uri, cursor.getString(0)))
            }
            if (cursor.extras.getBoolean(DocumentsContract.EXTRA_LOADING, false) ||
                cursor.extras.getString(DocumentsContract.EXTRA_ERROR) != null)
                throw NativeDocumentError("PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT")
        } ?: throw NativeDocumentError("PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT")
        result.sortWith(compareBy(String.CASE_INSENSITIVE_ORDER) { it.first })
        if (result.size == 1 && result[0].first.endsWith(".zim", true)) return result
        if (result.isEmpty() || result.size > 676 || !result[0].first.endsWith(".zimaa", true))
            throw NativeDocumentError("PROVIDER_CHOOSE_ONE_ARCHIVE_FOLDER")
        if (!providerSplitPartsComplete(result.map { it.first }))
            throw NativeDocumentError("PROVIDER_INCOMPLETE_SPLIT_ARCHIVE")
        return result
    }
    @Synchronized fun select(uri: Uri, folder: Boolean, persistent: Boolean, flags: Int, cancellation: CancellationSignal): app.tauri.plugin.JSObject {
        if (uri.scheme != "content") throw NativeDocumentError("DOCUMENT_URI_REQUIRED")
        val parts = entries(uri, folder, cancellation)
        return publish(Record(uri, folder, parts.map { it.first }, parts.map { it.second.toString() }),
            persistent, flags, cancellation)
    }
    private fun publish(record: Record, persistent: Boolean, flags: Int, cancellation: CancellationSignal): app.tauri.plugin.JSObject {
        val uri = record.uri
        val reference = java.util.UUID.randomUUID().toString()
        transient[reference] = record
        try {
            val acquired = acquire(reference, cancellation)
            release(acquired.getString("lease"))
            if (persistent) {
                val previousGrant = context.contentResolver.persistedUriPermissions.firstOrNull { it.uri == uri }
                if (flags and android.content.Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION == 0 &&
                    previousGrant?.isReadPermission != true)
                    throw NativeDocumentError("PROVIDER_CANNOT_REMEMBER_USE_COPY_IMPORT")
                val previousFlags = (if (previousGrant?.isReadPermission == true) android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION else 0) or
                    (if (previousGrant?.isWritePermission == true) android.content.Intent.FLAG_GRANT_WRITE_URI_PERMISSION else 0)
                val grantFlags = android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION
                context.contentResolver.takePersistableUriPermission(uri, grantFlags)
                val previousReference = preferences.getString("ref:$reference", null)
                val previousIndex = preferences.getString("refs:$uri", null)
                val hadOwnedFlags = preferences.contains("owned-flags:$uri")
                val previousOwnedFlags = preferences.getInt("owned-flags:$uri", 0)
                val edit = preferences.edit().putString("ref:$reference", encode(record))
                val acquiredFlags = grantFlags and previousFlags.inv()
                val ownedBefore = preferences.getInt("owned-flags:$uri",
                    if (preferences.getBoolean("owned:$uri", false)) android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION else 0)
                if (acquiredFlags != 0) edit.putInt("owned-flags:$uri", ownedBefore or acquiredFlags)
                indexReference(edit, uri, reference)
                if (!edit.commit()) {
                    if (acquiredFlags != 0) context.contentResolver.releasePersistableUriPermission(uri, acquiredFlags)
                    // SharedPreferences applies the in-memory map before the
                    // durable write; restore it so a failed reference cannot
                    // leak a grant forever.
                    val rollback = preferences.edit()
                    if (previousReference == null) rollback.remove("ref:$reference")
                    else rollback.putString("ref:$reference", previousReference)
                    if (previousIndex == null) rollback.remove("refs:$uri")
                    else rollback.putString("refs:$uri", previousIndex)
                    if (hadOwnedFlags) rollback.putInt("owned-flags:$uri", previousOwnedFlags)
                    else rollback.remove("owned-flags:$uri")
                    rollback.commit()
                    throw NativeDocumentError("PROVIDER_REFERENCE_SAVE_FAILED")
                }
            }
            return app.tauri.plugin.JSObject().put("reference", reference).put("displayName", record.names[0])
                .put("persistent", persistent)
        } catch (error: Exception) {
            transient.remove(reference)
            throw error
        }
    }
    @Synchronized fun selectBatch(uris: List<Uri>, folder: Boolean, persistent: Boolean, flags: Int,
        cancellation: CancellationSignal): app.tauri.plugin.JSObject {
        val selections = app.tauri.plugin.JSArray()
        val errors = app.tauri.plugin.JSArray()
        val records = mutableListOf<Pair<String, Record>>()
        fun failure(path: String, error: Exception) {
            errors.put(app.tauri.plugin.JSObject().put("path", path).put("error",
                (error as? NativeDocumentError)?.code ?: "PROVIDER_UNAVAILABLE_USE_COPY_IMPORT"))
        }
        try {
            for (uri in uris) {
                cancellation.throwIfCanceled()
                if (!folder) {
                    try { selections.put(select(uri, false, persistent, flags, cancellation)) }
                    catch (error: Exception) { failure(uri.toString(), error) }
                    continue
                }
                if (!DocumentsContract.isTreeUri(uri)) throw NativeDocumentError("DOCUMENT_TREE_UNAVAILABLE")
                val seen = HashSet<String>()
                fun visit(id: String, path: String, depth: Int) {
                    cancellation.throwIfCanceled()
                    if (depth > 128 || seen.size >= 100000 || !seen.add(id))
                        throw NativeDocumentError("CYCLIC_OR_DEEP_DOCUMENT_TREE")
                    val children = DocumentsContract.buildChildDocumentsUriUsingTree(uri, id)
                    val cohorts = linkedMapOf<String, MutableList<Pair<String, Uri>>>()
                    val directories = mutableListOf<Pair<String, String>>()
                    context.contentResolver.query(children, arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                        DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE),
                        null, null, null, cancellation)?.use { cursor ->
                        while (cursor.moveToNext()) {
                            cancellation.throwIfCanceled()
                            val childId = cursor.getString(0)
                            val name = safeName(cursor.getString(1))
                            if (cursor.getString(2) == DocumentsContract.Document.MIME_TYPE_DIR) {
                                directories.add(childId to name)
                            } else if (archiveName.matches(name)) {
                                val key = if (name.endsWith(".zim", true)) "whole:${nameKey(name)}"
                                    else "split:${nameKey(name.dropLast(2))}"
                                cohorts.getOrPut(key) { mutableListOf() }.add(name to
                                    DocumentsContract.buildDocumentUriUsingTree(uri, childId))
                            }
                        }
                        if (cursor.extras.getBoolean(DocumentsContract.EXTRA_LOADING, false) ||
                            cursor.extras.getString(DocumentsContract.EXTRA_ERROR) != null)
                            throw NativeDocumentError("PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT")
                    } ?: throw NativeDocumentError("PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT")
                    for (cohort in cohorts.values) {
                        cohort.sortWith(compareBy(String.CASE_INSENSITIVE_ORDER) { it.first })
                        val display = path + cohort[0].first
                        try {
                            val names = cohort.map { it.first }
                            validateArchiveParts(names)
                            records.add(display to Record(uri, false, names, cohort.map { it.second.toString() }))
                        } catch (error: Exception) { failure(display, error) }
                    }
                    for ((childId, name) in directories) visit(childId, "$path$name/", depth + 1)
                }
                visit(DocumentsContract.getTreeDocumentId(uri), "", 0)
            }
            for ((path, record) in records) {
                cancellation.throwIfCanceled()
                try { selections.put(publish(record, persistent, flags, cancellation)) }
                catch (error: Exception) { failure(path, error) }
            }
            return app.tauri.plugin.JSObject().put("selections", selections).put("errors", errors).put("cancelled", false)
        } catch (error: Exception) {
            for (index in 0 until selections.length()) forget(selections.getJSONObject(index).getString("reference"))
            throw error
        }
    }
    /** Incoming VIEW/SEND authority comes from the native intake queue, never
     * caller-supplied filesystem paths. A transient share must be reselected
     * with OPEN_DOCUMENT before it can become a saved library reference. */
    @Synchronized fun selectIncoming(uri: Uri, persistent: Boolean, flags: Int, cancellation: CancellationSignal): app.tauri.plugin.JSObject {
        if (uri.scheme != "content") throw NativeDocumentError("DOCUMENT_URI_REQUIRED")
        val durable = context.contentResolver.persistedUriPermissions.any {
            it.uri == uri && it.isReadPermission
        }
        val offered = flags and android.content.Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION != 0 &&
            flags and android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION != 0
        if (persistent && !durable && !offered)
            throw NativeDocumentError("PROVIDER_INCOMING_CHOOSE_ORIGINAL_OR_COPY")
        val grantFlags = if (durable || offered) android.content.Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION or
            android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION else 0
        return select(uri, false, persistent, grantFlags, cancellation)
    }
    /** Open one archive discovered by the device scan. The caller supplies the
     * exact document URIs it found; this class still re-queries each name and
     * re-checks seekability before minting reading authority. */
    @Synchronized fun selectScanned(parts: List<Pair<String, Uri>>, grant: Uri?, persistent: Boolean,
        cancellation: CancellationSignal): app.tauri.plugin.JSObject {
        val names = parts.map { (name, uri) ->
            when (uri.scheme) {
                "content" -> safeName(name)
                "file" -> safeName(name).also {
                    if (sharedFile(uri).name != it)
                        throw NativeDocumentError("PROVIDER_ARCHIVE_PARTS_CHANGED")
                }
                else -> throw NativeDocumentError("INVALID_PROVIDER_REFERENCE")
            }
        }
        if (names.toSet().size != names.size) throw NativeDocumentError("PROVIDER_ARCHIVE_PARTS_CHANGED")
        validateArchiveParts(names)
        val reference = java.util.UUID.randomUUID().toString()
        val record = Record(grant ?: parts[0].second, false, names, parts.map { it.second.toString() })
        transient[reference] = record
        try {
            val acquired = acquire(reference, cancellation)
            release(acquired.getString("lease"))
            if (persistent) {
                if (!canPersist(record))
                    throw NativeDocumentError("PROVIDER_CANNOT_REMEMBER_USE_COPY_IMPORT")
                val previousReference = preferences.getString("ref:$reference", null)
                val previousIndex = preferences.getString("refs:${record.uri}", null)
                val edit = preferences.edit().putString("ref:$reference", encode(record))
                indexReference(edit, record.uri, reference)
                if (!edit.commit()) {
                    val rollback = preferences.edit()
                    if (previousReference == null) rollback.remove("ref:$reference")
                    else rollback.putString("ref:$reference", previousReference)
                    if (previousIndex == null) rollback.remove("refs:${record.uri}")
                    else rollback.putString("refs:${record.uri}", previousIndex)
                    rollback.commit()
                    throw NativeDocumentError("PROVIDER_REFERENCE_SAVE_FAILED")
                }
            }
            return app.tauri.plugin.JSObject().put("reference", reference).put("displayName", names[0])
                .put("persistent", persistent)
        } catch (error: Exception) {
            transient.remove(reference)
            throw error
        }
    }
    /** A saved reference needs an authority that outlives this process: either a
     * persisted SAF tree grant or the all-files read permission that MediaStore
     * required on Android 12L and earlier. Nothing is copied to satisfy this. */
    private fun canPersist(record: Record): Boolean {
        if (context.contentResolver.persistedUriPermissions.any { it.uri == record.uri && it.isReadPermission })
            return true
        val documents = record.documents ?: return false
        val allFilesRead = Build.VERSION.SDK_INT <= 32 &&
            ContextCompat.checkSelfPermission(context, Manifest.permission.READ_EXTERNAL_STORAGE) ==
                PackageManager.PERMISSION_GRANTED
        if (!allFilesRead) return false
        return documents.all {
            val uri = Uri.parse(it)
            uri.scheme == "file" || uri.authority == MediaStore.AUTHORITY
        }
    }
    /** A raw path is accepted only for the primary shared volume; app-private
     * storage and every other scheme still go through the provider model. */
    private fun sharedFile(uri: Uri): File {
        val path = uri.path ?: throw NativeDocumentError("INVALID_PROVIDER_REFERENCE")
        val root = Environment.getExternalStorageDirectory()?.path
            ?: throw NativeDocumentError("INVALID_PROVIDER_REFERENCE")
        if (!path.startsWith(root + File.separator)) throw NativeDocumentError("INVALID_PROVIDER_REFERENCE")
        val file = File(path)
        if (!file.isAbsolute) throw NativeDocumentError("INVALID_PROVIDER_REFERENCE")
        return file
    }
    private fun validateArchiveParts(names: List<String>) {
        if (names.isEmpty() || names.size > 676) throw NativeDocumentError("PROVIDER_ARCHIVE_PARTS_CHANGED")
        if (names.size == 1) {
            if (!names[0].endsWith(".zim", true)) throw NativeDocumentError("PROVIDER_SPLIT_REQUIRES_FOLDER")
            return
        }
        if (!names[0].endsWith(".zimaa", true)) throw NativeDocumentError("PROVIDER_CHOOSE_ONE_ARCHIVE_FOLDER")
        if (!providerSplitPartsComplete(names))
            throw NativeDocumentError("PROVIDER_INCOMPLETE_SPLIT_ARCHIVE")
    }
    @Synchronized fun acquire(reference: String, cancellation: CancellationSignal): app.tauri.plugin.JSObject {
        val record = record(reference)
        val entries = recordEntries(record, cancellation)
        if (entries.map { it.first } != record.names) throw NativeDocumentError("PROVIDER_ARCHIVE_PARTS_CHANGED")
        val descriptors = mutableListOf<Pair<String, android.os.ParcelFileDescriptor>>()
        try {
            for ((name, uri) in entries) {
                cancellation.throwIfCanceled()
                if (uri.scheme == "file") {
                    // A shared-storage path reached only with the all-files read
                    // grant; the token registry never accepts caller paths.
                    val descriptor = android.os.ParcelFileDescriptor.open(sharedFile(uri),
                        android.os.ParcelFileDescriptor.MODE_READ_ONLY)
                    try {
                        val info = Os.fstat(descriptor.fileDescriptor)
                        if (!OsConstants.S_ISREG(info.st_mode))
                            throw NativeDocumentError("PROVIDER_NOT_SEEKABLE_USE_COPY_IMPORT")
                        descriptors.add(name to android.os.ParcelFileDescriptor.dup(descriptor.fileDescriptor))
                    } finally { descriptor.close() }
                    continue
                }
                // Asset offsets/pipe streams cannot be represented as libzim regular-file parts.
                val asset = requireProviderArchiveAsset(
                    context.contentResolver.openAssetFileDescriptor(uri, "r", cancellation))
                asset.use {
                    val info = Os.fstat(it.parcelFileDescriptor.fileDescriptor)
                    if (it.startOffset != 0L || !OsConstants.S_ISREG(info.st_mode) ||
                        (it.declaredLength >= 0 && it.declaredLength != info.st_size))
                        throw NativeDocumentError("PROVIDER_NOT_SEEKABLE_USE_COPY_IMPORT")
                    Os.lseek(it.parcelFileDescriptor.fileDescriptor, 0, OsConstants.SEEK_CUR)
                    descriptors.add(name to android.os.ParcelFileDescriptor.dup(it.parcelFileDescriptor.fileDescriptor))
                }
            }
            val lease = java.util.UUID.randomUUID().toString()
            val files = app.tauri.plugin.JSArray()
            descriptors.forEach { (name, fd) ->
                files.put(app.tauri.plugin.JSObject().put("name", name).put("fd", fd.fd))
            }
            val response = app.tauri.plugin.JSObject().put("lease", lease).put("files", files)
            if (lease in leases) throw NativeDocumentError("PROVIDER_LEASE_CONFLICT")
            leases[lease] = Lease(reference, record.uri, descriptors)
            return response
        } catch (error: Exception) {
            descriptors.forEach { runCatching { it.second.close() } }
            throw providerAcquisitionFailure(error)
        }
    }
    @Synchronized fun release(lease: String) {
        val value = leases.remove(lease) ?: return
        value.descriptors.forEach { it.second.close() }
        releaseUnusedGrant(value.uri)
    }
    @Synchronized fun forget(reference: String) {
        valid(reference)
        val value = runCatching { record(reference) }.getOrNull()
        if (preferences.contains("ref:$reference")) {
            val edit = preferences.edit().remove("ref:$reference")
            value?.let { dropReferenceIndex(edit, it.uri, reference) }
            if (!edit.commit()) throw NativeDocumentError("PROVIDER_REFERENCE_REMOVE_FAILED")
        }
        transient.remove(reference)
        value?.let { releaseUnusedGrant(it.uri) }
    }
    private fun releaseUnusedGrant(uri: Uri) {
        if (transient.values.any { it.uri == uri } || leases.values.any { it.uri == uri }) return
        if (referencedByStoredRecord(uri)) return
        val ownedFlags = preferences.getInt("owned-flags:$uri",
            if (preferences.getBoolean("owned:$uri", false)) android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION else 0)
        if (ownedFlags != 0) {
            val current = context.contentResolver.persistedUriPermissions.firstOrNull { it.uri == uri }
            val currentFlags = (if (current?.isReadPermission == true) android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION else 0) or
                (if (current?.isWritePermission == true) android.content.Intent.FLAG_GRANT_WRITE_URI_PERMISSION else 0)
            if (currentFlags and ownedFlags != 0)
                context.contentResolver.releasePersistableUriPermission(uri, currentFlags and ownedFlags)
            if (!preferences.edit().remove("owned:$uri").remove("owned-flags:$uri").remove("refs:$uri").commit())
                throw NativeDocumentError("PROVIDER_REFERENCE_REMOVE_FAILED")
        }
    }
}
