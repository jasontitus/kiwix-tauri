package org.kiwix.mobile

import android.content.Context
import android.net.Uri
import android.provider.DocumentsContract
import android.system.Os
import android.system.OsConstants
import app.tauri.plugin.JSObject
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

/** Deletion is a separate, durable capability issued for an exact provider cohort.
 * No raw file path, document name, size match, or caller supplied URI authorizes it. */
internal class ExternalArchiveMove(private val context: Context) {
    private val preferences = context.getSharedPreferences("external-archive-moves-v1", Context.MODE_PRIVATE)

    private fun requireGrant(grant: Uri) {
        if (context.contentResolver.persistedUriPermissions.none {
                it.uri == grant && it.isReadPermission && it.isWritePermission
            }) throw NativeDocumentError("PROVIDER_MOVE_REQUIRES_PERSISTED_WRITE_GRANT")
    }

    private fun snapshot(uri: Uri, copiedDigest: String? = null,
        digests: ProviderMoveDigestCache? = null): JSONObject? {
        if (uri.scheme != "content" || !DocumentsContract.isDocumentUri(context, uri))
            throw NativeDocumentError("PROVIDER_MOVE_REQUIRES_DOCUMENT_AUTHORITY")
        val cursor = queryProviderMoveDocument {
            context.contentResolver.query(uri, arrayOf(
                DocumentsContract.Document.COLUMN_DOCUMENT_ID, DocumentsContract.Document.COLUMN_FLAGS), null, null, null)
        } ?: return null
        val id = cursor.use {
            if (it.extras.getBoolean(DocumentsContract.EXTRA_LOADING, false) ||
                it.extras.getString(DocumentsContract.EXTRA_ERROR) != null)
                throw NativeDocumentError("PROVIDER_MOVE_IDENTITY_UNAVAILABLE")
            if (!it.moveToFirst()) return null
            if (it.getLong(1) and DocumentsContract.Document.FLAG_SUPPORTS_DELETE.toLong() == 0L)
                throw NativeDocumentError("PROVIDER_MOVE_DELETE_UNSUPPORTED")
            it.getString(0)
        }
        if (id != DocumentsContract.getDocumentId(uri)) throw NativeDocumentError("PROVIDER_MOVE_IDENTITY_CHANGED")
        val asset = context.contentResolver.openAssetFileDescriptor(uri, "r")
            ?: throw NativeDocumentError("PROVIDER_MOVE_IDENTITY_UNAVAILABLE")
        return asset.use {
            val stat = Os.fstat(it.parcelFileDescriptor.fileDescriptor)
            if (!OsConstants.S_ISREG(stat.st_mode) || stat.st_ino == 0L || it.startOffset != 0L)
                throw NativeDocumentError("PROVIDER_MOVE_IDENTITY_UNAVAILABLE")
            val value = JSONObject().put("uri", uri.toString()).put("document", id)
                .put("device", stat.st_dev).put("inode", stat.st_ino).put("size", stat.st_size)
                .put("modified", stat.st_mtime).put("changed", stat.st_ctime)
            if (copiedDigest != null) {
                val key = uri.toString()
                val identity = ProviderMoveStat(stat.st_dev, stat.st_ino, stat.st_size,
                    stat.st_mtime, stat.st_ctime)
                val verified = digests?.lookup(key, identity)
                if (verified != null) {
                    // Same descriptor and stat identity: the digest that authorized
                    // the first read still binds these exact bytes.
                    if (verified != copiedDigest) throw NativeDocumentError("PROVIDER_MOVE_CONTENT_CHANGED")
                    value.put("sha256", verified)
                } else it.createInputStream().use { input ->
                    val digest = providerMoveDigest(input, stat.st_size)
                    val after = Os.fstat(it.parcelFileDescriptor.fileDescriptor)
                    if (stat.st_dev != after.st_dev || stat.st_ino != after.st_ino ||
                        stat.st_size != after.st_size || stat.st_mtime != after.st_mtime ||
                        stat.st_ctime != after.st_ctime || digest != copiedDigest)
                        throw NativeDocumentError("PROVIDER_MOVE_CONTENT_CHANGED")
                    value.put("sha256", digest)
                    digests?.record(key, identity, digest)
                }
            }
            value
        }
    }

    private fun save(receipt: String, value: JSONObject) {
        if (!preferences.edit().putString(receipt, value.toString()).commit())
            throw NativeDocumentError("PROVIDER_MOVE_RECEIPT_SAVE_FAILED")
    }

    fun prepare(reference: String, grant: Uri, entries: List<Pair<String, Uri>>): JSObject {
        requireGrant(grant)
        val parts = JSONArray()
        for ((name, uri) in entries) {
            val identity = snapshot(uri) ?: throw NativeDocumentError("PROVIDER_MOVE_SOURCE_MISSING")
            parts.put(JSONObject().put("name", name).put("identity", identity).put("attempting", false).put("deleted", false))
        }
        val receipt = UUID.randomUUID().toString()
        save(receipt, JSONObject().put("reference", reference).put("grant", grant.toString()).put("parts", parts))
        return JSObject().put("receipt", receipt)
    }

    fun delete(reference: String, receipt: String, copiedParts: JSONArray): JSObject {
        require(UUID.fromString(receipt).toString() == receipt) { "INVALID_PROVIDER_MOVE_RECEIPT" }
        val value = preferences.getString(receipt, null)?.let(::JSONObject)
            ?: throw NativeDocumentError("PROVIDER_MOVE_RECEIPT_UNKNOWN")
        if (value.getString("reference") != reference) throw NativeDocumentError("PROVIDER_MOVE_REFERENCE_MISMATCH")
        val parts = value.getJSONArray("parts")
        if (copiedParts.length() != parts.length()) throw NativeDocumentError("PROVIDER_MOVE_PARTS_CHANGED")
        // Bind a durable receipt to exactly what Rust copied, not fresh provider
        // bytes or coarse provider timestamps. Recovery cannot rebind its hashes.
        for (index in 0 until parts.length()) {
            val part = parts.getJSONObject(index)
            val copied = copiedParts.getJSONObject(index)
            val digest = copied.getString("sha256")
            val expected = part.getJSONObject("identity")
            if (copied.getString("name") != part.getString("name") ||
                copied.getLong("size") != expected.getLong("size") || !digest.matches(Regex("[0-9a-f]{64}")) ||
                (expected.has("sha256") && expected.getString("sha256") != digest))
                throw NativeDocumentError("PROVIDER_MOVE_COPIED_PARTS_CHANGED")
            expected.put("sha256", digest)
        }
        save(receipt, value)
        val remaining = (0 until parts.length()).map { parts.getJSONObject(it) }
        if ((0 until parts.length()).all { parts.getJSONObject(it).getBoolean("deleted") })
            return JSObject().put("deleted", true)
        requireGrant(Uri.parse(value.getString("grant")))
        // Every part is read once per invocation; later cohort boundaries re-check
        // the same digest against the stat identity that authorized it.
        val digests = ProviderMoveDigestCache()
        // Validate the complete remaining cohort before deleting any member.
        for (index in 0 until parts.length()) {
            val part = parts.getJSONObject(index)
            if (!part.getBoolean("deleted")) validate(part, digests)
        }
        for (index in 0 until parts.length()) {
            val part = parts.getJSONObject(index)
            if (part.getBoolean("deleted")) continue
            if (validate(part, digests)) {
                part.put("attempting", true)
                save(receipt, value)
                // Revalidate every remaining part at each destructive boundary.
                // A later part may have changed after the first one was deleted.
                withVerifiedProviderMoveCohort(remaining, { candidate ->
                    if (!candidate.getBoolean("deleted")) validate(candidate, digests)
                }) {
                    // The cohort checks may reuse a cached digest, but the
                    // destructive target itself is re-hashed against the
                    // receipt immediately before deleteDocument.
                    if (validate(part, null) && !DocumentsContract.deleteDocument(context.contentResolver,
                            Uri.parse(part.getJSONObject("identity").getString("uri"))))
                        throw NativeDocumentError("PROVIDER_MOVE_DELETE_FAILED")
                }
            }
            part.put("deleted", true)
            save(receipt, value)
        }
        return JSObject().put("deleted", true)
    }

    private fun validate(part: JSONObject, digests: ProviderMoveDigestCache? = null): Boolean {
        val expected = part.getJSONObject("identity")
        val actual = snapshot(Uri.parse(expected.getString("uri")), expected.getString("sha256"), digests)
        return providerMoveMayDelete(identity(expected), actual?.let(::identity), part.getBoolean("attempting"))
    }
}

private fun identity(value: JSONObject) = ProviderMoveIdentity(
    value.getString("uri"), value.getString("document"), value.getLong("device"),
    value.getLong("inode"), value.getLong("size"), value.getLong("modified"), value.getLong("changed"),
    value.optString("sha256").takeIf { it.isNotEmpty() },
)
