package org.kiwix.mobile

import android.Manifest
import android.content.ContentUris
import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.CancellationSignal
import android.os.Environment
import android.os.OperationCanceledException
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.system.Os
import android.system.OsConstants
import androidx.core.content.ContextCompat
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import org.json.JSONObject
import java.io.File
import java.util.Locale
import java.util.UUID

/**
 * Device-wide ZIM discovery over the authorities this app can actually read:
 * MediaStore rows and every persisted SAF tree grant. It never opens a raw
 * filesystem path, never copies, and never touches the network. Each candidate
 * keeps an opaque token; [open] mints the same retained-descriptor authority
 * the explicit picker uses.
 */
internal class DeviceZimScan(private val context: Context, private val externalDocuments: ExternalDocuments) {
    private data class Part(
        val name: String,
        val uri: Uri,
        val size: Long,
        val modified: Long,
        val volume: String,
        val location: String,
        val source: String,
        val directory: String,
        val base: String,
        val readable: Boolean,
        val problem: String?,
        val grant: Uri?,
    )
    private data class ScannedArchive(
        val name: String,
        val names: List<String>,
        val parts: List<Pair<String, Uri>>,
        val size: Long,
        val modified: Long,
        val volume: String,
        val location: String,
        val sources: MutableSet<String>,
        val readable: Boolean,
        val problem: String?,
        val grant: Uri?,
    )

    private val archiveName = Regex("(?i).+\\.zim(?:[a-z]{2})?")
    private val splitName = Regex("(?i)^(.+\\.zim)([a-z]{2})$")
    private val tokens = object : LinkedHashMap<String, ScannedArchive>(64, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, ScannedArchive>?) = size > 2048
    }
    /** Read-only sessions keep the picked tree until process death, never on disk. */
    private val transientTrees = LinkedHashSet<Uri>()

    fun addTransientTree(tree: Uri) {
        synchronized(transientTrees) { transientTrees.add(tree) }
    }

    fun scan(maxEntries: Int, cancellation: CancellationSignal): JSObject {
        val limitations = LinkedHashSet<String>()
        val parts = mutableListOf<Part>()
        val cap = (maxEntries.coerceIn(1, 4096)) * 8
        mediaParts(cancellation, parts, cap, limitations)
        storageParts(cancellation, parts, cap, limitations)
        treeParts(cancellation, parts, cap, limitations)
        if (Build.VERSION.SDK_INT >= 33) limitations.add(
            "Android 13 and later only exposes shared-storage files you grant a folder for. Add folder access to scan Downloads or Documents.")
        val transient = synchronized(transientTrees) { transientTrees.isNotEmpty() }
        if (!transient && context.contentResolver.persistedUriPermissions
                .none { it.isReadPermission && DocumentsContract.isTreeUri(it.uri) })
            limitations.add("No folder access has been granted yet. Add folder access to include ZIM files outside app storage.")
        val archives = group(parts).take(maxEntries.coerceIn(1, 4096))
        if (parts.size >= cap) limitations.add("Stopped after $cap archive candidates; narrow the granted folders.")
        val entries = JSArray()
        for (archive in archives) {
            val token = UUID.randomUUID().toString()
            cancellation.throwIfCanceled()
            synchronized(tokens) { tokens[token] = archive }
            entries.put(JSObject().apply {
                put("token", token)
                put("name", archive.name)
                put("names", JSArray.from(archive.names.toTypedArray()))
                put("size", archive.size)
                put("modified", archive.modified)
                put("volume", archive.volume)
                put("location", archive.location)
                put("sources", JSArray.from(archive.sources.toTypedArray()))
                put("readable", archive.readable)
                put("problem", archive.problem ?: JSONObject.NULL)
            })
        }
        return JSObject().put("entries", entries).put("limitations", JSArray.from(limitations.toTypedArray()))
    }

    fun open(token: String, persistent: Boolean, cancellation: CancellationSignal): JSObject {
        if (runCatching { UUID.fromString(token) }.isFailure) throw NativeDocumentError("SCAN_RESULT_EXPIRED")
        val archive = synchronized(tokens) { tokens[token] } ?: throw NativeDocumentError("SCAN_RESULT_EXPIRED")
        if (!archive.readable || archive.problem != null)
            throw NativeDocumentError("SCANNED_ARCHIVE_UNAVAILABLE")
        // Candidate discovery never opens descriptors. Readability and the
        // seekable regular-file contract are proven once, here, before the
        // acquisition below mints the retained authority.
        for ((_, uri) in archive.parts) if (!probe(uri, cancellation).first)
            throw NativeDocumentError("SCANNED_ARCHIVE_UNAVAILABLE")
        return externalDocuments.selectScanned(archive.parts, archive.grant, persistent, cancellation)
    }

    private fun mediaParts(cancellation: CancellationSignal, out: MutableList<Part>, cap: Int,
        limitations: MutableSet<String>) {
        val collection = if (Build.VERSION.SDK_INT >= 29) MediaStore.Files.getContentUri(MediaStore.VOLUME_EXTERNAL)
            else MediaStore.Files.getContentUri("external")
        val projection = mutableListOf(
            MediaStore.MediaColumns._ID, MediaStore.MediaColumns.DISPLAY_NAME,
            MediaStore.MediaColumns.SIZE, MediaStore.MediaColumns.DATE_MODIFIED, MediaStore.MediaColumns.DATA)
        if (Build.VERSION.SDK_INT >= 29) {
            projection.add(MediaStore.MediaColumns.VOLUME_NAME)
            projection.add(MediaStore.MediaColumns.RELATIVE_PATH)
        }
        if (Build.VERSION.SDK_INT >= 30) projection.add(MediaStore.MediaColumns.IS_TRASHED)
        val owned = ownedRoots()
        try {
            context.contentResolver.query(collection, projection.toTypedArray(),
                "${MediaStore.MediaColumns.DISPLAY_NAME} LIKE ?", arrayOf("%.zim%"), null, cancellation)?.use { cursor ->
                val id = cursor.getColumnIndexOrThrow(MediaStore.MediaColumns._ID)
                val nameIndex = cursor.getColumnIndexOrThrow(MediaStore.MediaColumns.DISPLAY_NAME)
                val sizeIndex = cursor.getColumnIndex(MediaStore.MediaColumns.SIZE)
                val modifiedIndex = cursor.getColumnIndex(MediaStore.MediaColumns.DATE_MODIFIED)
                val dataIndex = cursor.getColumnIndex(MediaStore.MediaColumns.DATA)
                val volumeIndex = cursor.getColumnIndex(MediaStore.MediaColumns.VOLUME_NAME)
                val relativeIndex = cursor.getColumnIndex(MediaStore.MediaColumns.RELATIVE_PATH)
                val trashedIndex = cursor.getColumnIndex(MediaStore.MediaColumns.IS_TRASHED)
                while (cursor.moveToNext()) {
                    cancellation.throwIfCanceled()
                    if (out.size >= cap) return
                    val name = cursor.getString(nameIndex) ?: continue
                    if (!archiveName.matches(name) || name.startsWith('.')) continue
                    if (trashedIndex >= 0 && cursor.getInt(trashedIndex) != 0) continue
                    val data = if (dataIndex >= 0) cursor.getString(dataIndex) else null
                    if (data != null && (isOwnedPath(canonicalStoragePath(data), owned) ||
                            data.contains("/.Trash/"))) continue
                    val uri = ContentUris.withAppendedId(collection, cursor.getLong(id))
                    // MediaStore leaves SIZE/DATE_MODIFIED null for non-media rows
                    // on several releases; the indexed path still stats the file.
                    val stat = data?.let { runCatching { Os.stat(it) }.getOrNull() }
                    val rawSize = (if (sizeIndex >= 0) cursor.getLong(sizeIndex) else 0L)
                    val rawModified = (if (modifiedIndex >= 0) cursor.getLong(modifiedIndex) * 1000 else 0L)
                    // Only rows whose indexed metadata is missing pay one
                    // metadata-only descriptor open. That keeps the Rust
                    // size/mtime identity (so MediaStore and SAF sightings
                    // merge) and the truthful readable flag without reading
                    // archive content during listing.
                    val inspected = if (stat == null) inspect(uri, cancellation) else null
                    val readable = inspected?.regular
                        ?: (stat?.let { OsConstants.S_ISREG(it.st_mode) } ?: false)
                    val size = rawSize.takeIf { it > 0 }
                        ?: inspected?.size?.takeIf { it > 0 }
                        ?: stat?.st_size?.takeIf { it > 0 } ?: 0L
                    val modified = rawModified.takeIf { it > 0 }
                        ?: inspected?.modified?.takeIf { it > 0 }
                        ?: (stat?.st_mtime ?: 0L) * 1000
                    val volume = normalizeVolume(if (volumeIndex >= 0) cursor.getString(volumeIndex) else null, data)
                    val location = (if (relativeIndex >= 0) cursor.getString(relativeIndex) else null)
                        ?.trimEnd('/') ?: relativeFromData(data)
                    out.add(part(name, uri, size, modified, volume, location, "media", null,
                        readable, if (readable) null else NOT_READABLE))
                }
            }
        } catch (error: OperationCanceledException) {
            throw error
        } catch (error: SecurityException) {
            limitations.add("Shared-storage access was denied; only app-visible files were searched.")
        } catch (error: Exception) {
            limitations.add("MediaStore could not be searched: ${error.message ?: error.javaClass.simpleName}")
        }
    }

    /** Android 12L and earlier grant read-only access to all shared files with
     * READ_EXTERNAL_STORAGE, but MediaStore does not list every non-media row.
     * Walk the primary shared volume like upstream's file search, bounded and
     * excluding app data, trash and hidden directories. */
    private fun storageParts(cancellation: CancellationSignal, out: MutableList<Part>, cap: Int,
        limitations: MutableSet<String>) {
        if (Build.VERSION.SDK_INT > 32 || ContextCompat.checkSelfPermission(context,
                Manifest.permission.READ_EXTERNAL_STORAGE) != PackageManager.PERMISSION_GRANTED) return
        val root = Environment.getExternalStorageDirectory() ?: return
        val owned = ownedRoots()
        val visited = intArrayOf(0)
        // Roots are canonicalized once; each recursion step appends a single
        // validated component instead of canonicalizing the whole path again.
        val canonicalRoot = runCatching { root.canonicalPath }.getOrDefault(root.absolutePath)
        try {
            walkStorage(root, root, canonicalRoot, 0, cancellation, out, cap, owned, visited)
        } catch (error: OperationCanceledException) {
            throw error
        } catch (error: SecurityException) {
            limitations.add("Shared-storage access was denied; only app-visible files were searched.")
        } catch (error: Exception) {
            limitations.add("Shared storage could not be searched: ${error.message ?: error.javaClass.simpleName}")
        }
    }

    private fun walkStorage(root: File, directory: File, canonical: String, depth: Int,
        cancellation: CancellationSignal, out: MutableList<Part>, cap: Int, owned: List<String>,
        visited: IntArray) {
        if (depth > 12 || out.size >= cap) return
        val children = directory.listFiles() ?: return
        for (child in children) {
            cancellation.throwIfCanceled()
            if (++visited[0] > 20000 || out.size >= cap) return
            val name = child.name
            if (name.startsWith('.')) continue
            val lstat = runCatching { Os.lstat(child.path) }.getOrNull()
            // Only a symlink needs resolution; every other child is exactly one
            // component below a directory that was already proven canonical.
            var canonicalChild = canonical + File.separator + name
            var isDirectory = lstat != null && OsConstants.S_ISDIR(lstat.st_mode)
            var stat = lstat
            if (lstat != null && OsConstants.S_ISLNK(lstat.st_mode)) {
                canonicalChild = runCatching { child.canonicalPath }.getOrDefault(canonicalChild)
                stat = runCatching { Os.stat(child.path) }.getOrNull()
                isDirectory = stat != null && OsConstants.S_ISDIR(stat.st_mode)
            }
            if (isOwnedPath(canonicalChild, owned)) continue
            if (isDirectory) {
                if (name.equals("Android", true) || name.equals("data", true) ||
                    name.equals("obb", true) || name.equals(".Trash", true)) continue
                walkStorage(root, child, canonicalChild, depth + 1, cancellation, out, cap, owned, visited)
                continue
            }
            if (!archiveName.matches(name)) continue
            val relative = child.parentFile?.let { parent ->
                runCatching { parent.relativeTo(root).path }.getOrElse { "" }
            } ?: ""
            // Listing never opens a descriptor: the walk keeps only name/size
            // filters. An entry whose lstat failed cannot be opened in place.
            val readable = stat != null && OsConstants.S_ISREG(stat.st_mode)
            val size = child.length().takeIf { it > 0 } ?: (stat?.st_size ?: 0L)
            val modified = (stat?.st_mtime ?: 0L) * 1000
            out.add(part(name, Uri.fromFile(child), size, modified, "primary", relative,
                "storage", null, readable, if (readable) null else NOT_READABLE))
        }
    }

    private fun treeParts(cancellation: CancellationSignal, out: MutableList<Part>, cap: Int,
        limitations: MutableSet<String>) {
        val transient = synchronized(transientTrees) { transientTrees.toList() }
        val trees = (context.contentResolver.persistedUriPermissions
            .filter { it.isReadPermission && DocumentsContract.isTreeUri(it.uri) }
            .map { it.uri } + transient).distinct().sortedBy { it.toString() }
        for (tree in trees) {
            cancellation.throwIfCanceled()
            try {
                walk(tree, DocumentsContract.getTreeDocumentId(tree), 0, HashSet(), cancellation, out, cap)
            } catch (error: OperationCanceledException) {
                throw error
            } catch (error: SecurityException) {
                limitations.add("Folder access expired for ${tree.authority ?: "a document provider"}.")
            } catch (error: Exception) {
                limitations.add("Could not scan ${tree.lastPathSegment ?: tree.authority ?: "a folder"}: " +
                    (error.message ?: error.javaClass.simpleName))
            }
        }
    }

    private fun walk(tree: Uri, documentId: String, depth: Int, visited: MutableSet<String>,
        cancellation: CancellationSignal, out: MutableList<Part>, cap: Int) {
        if (depth > 64 || !visited.add(documentId)) throw NativeDocumentError("CYCLIC_OR_DEEP_DOCUMENT_TREE")
        val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, documentId)
        val projection = arrayOf(
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
            DocumentsContract.Document.COLUMN_SIZE,
            DocumentsContract.Document.COLUMN_LAST_MODIFIED)
        val directories = mutableListOf<String>()
        context.contentResolver.query(children, projection, null, null, null, cancellation)?.use { cursor ->
            while (cursor.moveToNext()) {
                cancellation.throwIfCanceled()
                if (out.size >= cap) return
                val childId = cursor.getString(0) ?: continue
                val name = runCatching { safeName(cursor.getString(1) ?: "") }.getOrNull() ?: continue
                // Hidden entries (trashed or partial files, private folders) are
                // skipped exactly as the MediaStore and shared-storage walks do.
                if (hiddenScanName(name)) continue
                val mime = cursor.getString(2) ?: ""
                if (mime == DocumentsContract.Document.MIME_TYPE_DIR) {
                    directories.add(childId)
                    continue
                }
                if (!archiveName.matches(name)) continue
                val uri = DocumentsContract.buildDocumentUriUsingTree(tree, childId)
                // ExternalStorageProvider document IDs map to a primary-volume
                // path; stat it when the provider leaves metadata null so the
                // same file keeps the same identity as its MediaStore row.
                // The descriptor itself is opened only at open(token).
                val stat = primaryVolumePath(tree, childId)?.let { runCatching { Os.stat(it) }.getOrNull() }
                val rawSize = if (cursor.isNull(3)) 0L else cursor.getLong(3)
                val rawModified = if (cursor.isNull(4)) 0L else cursor.getLong(4)
                val inspected = if (stat == null) inspect(uri, cancellation) else null
                val readable = inspected?.regular
                    ?: (stat?.let { OsConstants.S_ISREG(it.st_mode) } ?: false)
                val size = rawSize.takeIf { it > 0 }
                    ?: inspected?.size?.takeIf { it > 0 }
                    ?: stat?.st_size?.takeIf { it > 0 } ?: 0L
                val modified = rawModified.takeIf { it > 0 }
                    ?: inspected?.modified?.takeIf { it > 0 }
                    ?: (stat?.st_mtime ?: 0L) * 1000
                val (volume, location) = documentIdentity(tree, childId)
                out.add(part(name, uri, size, modified, volume, location, "tree", tree,
                    readable, if (readable) null else NOT_READABLE))
            }
            if (Build.VERSION.SDK_INT >= 29) {
                val extras = cursor.extras
                if (extras.getBoolean(DocumentsContract.EXTRA_LOADING, false) ||
                    extras.getString(DocumentsContract.EXTRA_ERROR) != null)
                    throw NativeDocumentError("PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT")
            }
        } ?: throw NativeDocumentError("DOCUMENT_TREE_UNAVAILABLE")
        for (directory in directories) {
            if (out.size >= cap) return
            walk(tree, directory, depth + 1, visited, cancellation, out, cap)
        }
    }

    private fun part(name: String, uri: Uri, size: Long, modified: Long, volume: String,
        location: String, source: String, grant: Uri?, readable: Boolean, problem: String?): Part {
        val split = splitName.matchEntire(name)
        val base = split?.groupValues?.get(1) ?: name
        val directory = "$source|$volume|$location"
        return Part(name, uri, size, modified, volume, location, source, directory, base,
            readable, problem, grant)
    }

    private fun group(parts: List<Part>): List<ScannedArchive> {
        val archives = mutableListOf<ScannedArchive>()
        val splitGroups = LinkedHashMap<String, MutableList<Part>>()
        for (part in parts) {
            val split = splitName.matchEntire(part.name)
            if (split == null) {
                archives.add(archiveOf(listOf(part), null))
            } else {
                splitGroups.getOrPut("${part.directory}|${split.groupValues[1].lowercase(Locale.ROOT)}") {
                    mutableListOf()
                }.add(part)
            }
        }
        for (group in splitGroups.values) {
            group.sortBy { it.name.lowercase(Locale.ROOT) }
            val base = group.first().base.lowercase(Locale.ROOT)
            val names = group.map { it.name.lowercase(Locale.ROOT) }
            val expected = names.indices.map { base + suffix(it) }
            val complete = names == expected
            archives.add(archiveOf(group, if (complete) null
                else "Split archive is incomplete or has extra parts; import a copy."))
        }
        return archives.sortedBy { it.name.lowercase(Locale.ROOT) }
    }

    private fun archiveOf(parts: List<Part>, problem: String?): ScannedArchive {
        val first = parts.first()
        val complete = parts.all { it.size > 0 }
        val sources = parts.map { it.source }.toMutableSet()
        return ScannedArchive(
            name = first.name,
            names = parts.map { it.name },
            parts = parts.map { it.name to it.uri },
            size = if (complete) parts.sumOf { it.size } else 0,
            modified = first.modified,
            volume = first.volume,
            location = first.location,
            sources = sources,
            readable = problem == null && parts.all { it.readable },
            problem = problem ?: parts.firstNotNullOfOrNull { it.problem }
                ?: if (parts.any { !it.readable }) NOT_READABLE else null,
            grant = first.grant,
        )
    }

    private fun suffix(index: Int) = "${('a'.code + index / 26).toChar()}${('a'.code + index % 26).toChar()}"

    /** A provider stream or offset slice cannot become a libzim regular-file part. */
    private fun probe(uri: Uri, cancellation: CancellationSignal): Pair<Boolean, Long> = try {
        context.contentResolver.openAssetFileDescriptor(uri, "r", cancellation)?.use { asset ->
            val info = Os.fstat(asset.parcelFileDescriptor.fileDescriptor)
            val regular = asset.startOffset == 0L && OsConstants.S_ISREG(info.st_mode) &&
                (asset.declaredLength < 0 || asset.declaredLength == info.st_size)
            regular to info.st_size
        } ?: (false to 0L)
    } catch (error: OperationCanceledException) {
        throw error
    } catch (error: Exception) {
        false to 0L
    }

    private data class InspectedFile(val regular: Boolean, val size: Long, val modified: Long)

    /** Metadata-only descriptor open for rows whose indexed size/mtime is absent. */
    private fun inspect(uri: Uri, cancellation: CancellationSignal): InspectedFile? = try {
        context.contentResolver.openAssetFileDescriptor(uri, "r", cancellation)?.use { asset ->
            val info = Os.fstat(asset.parcelFileDescriptor.fileDescriptor)
            InspectedFile(
                asset.startOffset == 0L && OsConstants.S_ISREG(info.st_mode) &&
                    (asset.declaredLength < 0 || asset.declaredLength == info.st_size),
                info.st_size,
                info.st_mtime * 1000,
            )
        }
    } catch (error: OperationCanceledException) {
        throw error
    } catch (error: Exception) {
        null
    }

    private fun ownedRoots(): List<String> = buildList {
        add(context.filesDir)
        add(context.noBackupFilesDir)
        add(context.cacheDir)
        context.getExternalFilesDirs(null).filterNotNull().forEach(::add)
        context.getExternalCacheDirs().filterNotNull().forEach(::add)
    }.mapNotNull { runCatching { it.canonicalPath }.getOrNull() }

    /** Roots are canonical; callers pass an already canonical path. */
    private fun isOwnedPath(canonicalPath: String, roots: List<String>): Boolean =
        roots.any { root -> canonicalPath == root || canonicalPath.startsWith(root + File.separator) }

    /** MediaStore DATA is absolute but may use the legacy /storage/self/primary alias. */
    private fun canonicalStoragePath(path: String): String = when {
        path == "/storage/self/primary" -> "/storage/emulated/0"
        path.startsWith("/storage/self/primary/") ->
            "/storage/emulated/0/" + path.removePrefix("/storage/self/primary/")
        else -> path
    }

    private fun normalizeVolume(volume: String?, data: String?): String {
        val raw = volume?.takeIf { it.isNotBlank() } ?: data?.let { path ->
            when {
                path.startsWith("/storage/emulated/0/") || path.startsWith("/storage/self/primary/") -> "primary"
                path.startsWith("/storage/") -> path.removePrefix("/storage/").substringBefore('/')
                else -> "primary"
            }
        } ?: "primary"
        return when (raw.lowercase(Locale.ROOT)) {
            "external_primary", "emulated", "0", "self" -> "primary"
            else -> raw
        }
    }

    private fun relativeFromData(data: String?): String {
        if (data == null) return ""
        val normalized = data.replace('\\', '/')
        val prefix = when {
            normalized.startsWith("/storage/emulated/0/") -> "/storage/emulated/0/"
            normalized.startsWith("/storage/self/primary/") -> "/storage/self/primary/"
            normalized.startsWith("/storage/") -> normalized.removePrefix("/storage/").let { rest ->
                "/storage/" + rest.substringBefore('/') + "/"
            }
            else -> return ""
        }
        val relative = normalized.removePrefix(prefix)
        return relative.substringBeforeLast('/', "")
    }

    private fun primaryVolumePath(tree: Uri, documentId: String): String? {
        if (tree.authority != "com.android.externalstorage.documents") return null
        val colon = documentId.indexOf(':')
        if (colon <= 0 || !documentId.substring(0, colon).equals("primary", true)) return null
        return "/storage/emulated/0/" + documentId.substring(colon + 1)
    }

    /** ExternalStorageProvider document IDs encode the volume and relative path. */
    private fun documentIdentity(tree: Uri, documentId: String): Pair<String, String> {
        val authority = tree.authority ?: "documents"
        if (authority == "com.android.externalstorage.documents") {
            val colon = documentId.indexOf(':')
            if (colon > 0) {
                val volume = normalizeVolume(documentId.substring(0, colon), null)
                val path = documentId.substring(colon + 1)
                return volume to path.substringBeforeLast('/', "")
            }
        }
        return authority to documentId.substringBeforeLast('/', "")
    }

    private companion object {
        const val NOT_READABLE = "This location is not readable in place; import a copy."
    }
}

/** Dot-named files and folders are never device-scan candidates or walked. */
internal fun hiddenScanName(name: String): Boolean = false
