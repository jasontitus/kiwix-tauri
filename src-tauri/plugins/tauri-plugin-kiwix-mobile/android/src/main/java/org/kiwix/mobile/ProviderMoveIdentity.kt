package org.kiwix.mobile

internal data class ProviderMoveIdentity(
    val uri: String, val document: String, val device: Long, val inode: Long,
    val size: Long, val modified: Long, val changed: Long, val sha256: String? = null,
)

/** An absent object is idempotent only after its exact delete was durably attempted. */
internal fun providerMoveMayDelete(expected: ProviderMoveIdentity, actual: ProviderMoveIdentity?, attempting: Boolean): Boolean {
    if (actual == null && attempting) return false
    check(actual != null && actual == expected) { "PROVIDER_MOVE_IDENTITY_CHANGED" }
    return true
}

/** Provider stat identity without the digest it authorizes. */
internal data class ProviderMoveStat(
    val device: Long, val inode: Long, val size: Long, val modified: Long, val changed: Long,
)

/**
 * One content read per part per invocation. A part is hashed again only after
 * its stat identity changed; an unchanged stat reuses the digest that the
 * first read verified, so a cohort re-validation is metadata-only.
 */
internal class ProviderMoveDigestCache {
    private val verified = HashMap<String, Pair<ProviderMoveStat, String>>()

    fun lookup(uri: String, stat: ProviderMoveStat): String? = verified[uri]?.takeIf { it.first == stat }?.second

    fun record(uri: String, stat: ProviderMoveStat, sha256: String) { verified[uri] = stat to sha256 }
}

/** Hash the exact retained descriptor, bounded by the copied part's size. */
internal fun providerMoveDigest(input: java.io.InputStream, size: Long): String {
    check(size >= 0 && size < Long.MAX_VALUE) { "PROVIDER_MOVE_INVALID_SIZE" }
    val digest = java.security.MessageDigest.getInstance("SHA-256")
    val buffer = ByteArray(128 * 1024)
    var total = 0L
    while (total <= size) {
        val count = input.read(buffer, 0, minOf(buffer.size.toLong(), size - total + 1).toInt())
        if (count < 0) break
        check(count > 0) { "PROVIDER_MOVE_CONTENT_UNAVAILABLE" }
        digest.update(buffer, 0, count)
        total += count
    }
    check(total == size) { "PROVIDER_MOVE_CONTENT_CHANGED" }
    val hex = "0123456789abcdef"
    return buildString(64) {
        for (byte in digest.digest()) {
            val value = byte.toInt() and 255
            append(hex[value ushr 4])
            append(hex[value and 15])
        }
    }
}

/** Never enter a destructive boundary after checking only its first member. */
internal fun <T> withVerifiedProviderMoveCohort(parts: List<T>, validate: (T) -> Unit, action: () -> Unit) {
    parts.forEach(validate)
    action()
}

/** Only the document-query contract's explicit not-found response proves absence.
 * A null cursor means the provider failed to answer, not that deletion completed. */
internal fun <T : Any> queryProviderMoveDocument(query: () -> T?): T? {
    return try {
        checkNotNull(query()) { "PROVIDER_MOVE_IDENTITY_UNAVAILABLE" }
    } catch (_: java.io.FileNotFoundException) {
        null
    }
}
