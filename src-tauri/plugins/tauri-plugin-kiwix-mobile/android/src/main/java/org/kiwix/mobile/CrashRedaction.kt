package org.kiwix.mobile

/**
 * The redaction every crash-record text field goes through.
 *
 * This is the Kotlin implementation of the one rule written down in
 * `src-tauri/tests/fixtures/crash-redaction-spec.md`. The Rust panic hook
 * (`src-tauri/src/crash.rs`) and the iOS intake implement the same text, and
 * all three must reproduce every vector in `crash-redaction-cases.json` next
 * to it. The section and step numbers below are the specification's.
 *
 * Three things the specification requires of every port:
 *
 * * Every index, count, scan and cut is in Unicode scalar values. The text is
 *   handled as an array of code points, so nothing can split a surrogate pair.
 * * Whitespace is the explicit `White_Space` list in [isWhiteSpace].
 *   `Char.isWhitespace`, `Character.isWhitespace` and a regular-expression
 *   `\s` all differ from it and are not used.
 * * Case is folded for ASCII letters only. `equals(ignoreCase = true)` and
 *   `lowercase()` fold `ſ` to `s` and the Kelvin sign to `k`, and are not used.
 *
 * Each step runs in time proportional to its input: line ends and the other
 * look-ahead facts are computed once per step, and every forward scan covers
 * only text that the match then consumes.
 */
internal object CrashRedaction {
    const val MAX_INPUT = 32_768
    const val MAX_MESSAGE = 2_000

    private const val REDACTED = "[redacted]"
    private const val URL = "[url]"
    private const val READER_URL = "[reader-url]"
    private const val EMAIL = "[email]"
    private const val PATH = "[path]"

    /** Steps 1 to 8: a message, a thread name, a location. */
    fun redact(value: String): String {
        var text = bounded(codePoints(value))
        text = redactSecrets(text)
        text = replaceUrls(text)
        text = redactEmails(text)
        text = redactPaths(text)
        text = redactOpaque(text)
        return string(truncate(collapseWhitespace(text), MAX_MESSAGE))
    }

    /** Steps 1 to 5, for the text of a stack frame: its spacing is kept. */
    fun redactLine(value: String): String {
        var text = bounded(codePoints(value))
        text = redactSecrets(text)
        text = replaceUrls(text)
        text = redactEmails(text)
        return string(redactPaths(text))
    }

    /** At most [limit] scalars, the last one `…` when something was cut. */
    fun truncate(value: String, limit: Int): String = string(truncate(codePoints(value), limit))

    /**
     * Steps 2 to 6 on text of any length, without the bound of step 1. Only
     * for the test that each step is linear in its input; returns the length
     * of the result.
     */
    internal fun unboundedStepsForTiming(value: String): Int =
        redactOpaque(redactPaths(redactEmails(replaceUrls(redactSecrets(codePoints(value)))))).size

    // ---- 1. Terms --------------------------------------------------------

    private fun codePoints(value: CharSequence): IntArray {
        val result = IntArray(Character.codePointCount(value, 0, value.length))
        var index = 0
        var count = 0
        while (index < value.length) {
            val point = Character.codePointAt(value, index)
            result[count++] = point
            index += Character.charCount(point)
        }
        return result
    }

    private fun string(points: IntArray): String = String(points, 0, points.size)

    /** Collects output as code points and plain placeholder text. */
    private class Output(capacity: Int) {
        private val text = StringBuilder(capacity)
        fun point(point: Int) { text.appendCodePoint(point) }
        fun range(points: IntArray, from: Int, to: Int) { for (index in from until to) text.appendCodePoint(points[index]) }
        fun literal(value: String) { text.append(value) }
        fun points(): IntArray = codePoints(text)
    }

    /** Exactly the Unicode `White_Space` property. */
    internal fun isWhiteSpace(point: Int): Boolean = when (point) {
        in 0x09..0x0D, 0x20, 0x85, 0xA0, 0x1680, in 0x2000..0x200A, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000 -> true
        else -> false
    }

    private fun isLineBreak(point: Int) = point == '\n'.code || point == '\r'.code
    private fun isAsciiUpper(point: Int) = point in 'A'.code..'Z'.code
    private fun isAsciiLower(point: Int) = point in 'a'.code..'z'.code
    private fun isAsciiLetter(point: Int) = isAsciiUpper(point) || isAsciiLower(point)
    private fun isAsciiDigit(point: Int) = point in '0'.code..'9'.code
    private fun isAsciiAlphanumeric(point: Int) = isAsciiLetter(point) || isAsciiDigit(point)
    private fun asciiLower(point: Int) = if (isAsciiUpper(point)) point + 32 else point

    /** The scalar at [index], or -1 outside the text. */
    private fun IntArray.at(index: Int): Int = if (index in indices) this[index] else -1

    /** Whether [point] is one of the ASCII [characters]. */
    private fun isOneOf(point: Int, characters: String) = point in 0..0x7F && characters.indexOf(point.toChar()) >= 0

    /** Whether `points[from, to)` equals [lower] after lowercasing ASCII letters only. */
    private fun asciiEquals(points: IntArray, from: Int, to: Int, lower: String): Boolean {
        if (from < 0 || to > points.size || to - from != lower.length) return false
        for (offset in lower.indices) if (asciiLower(points[from + offset]) != lower[offset].code) return false
        return true
    }

    /** The first index at or after [from] whose scalar is not whitespace, or the text length. */
    private fun skipWhitespace(points: IntArray, from: Int): Int {
        var index = from
        while (index < points.size && isWhiteSpace(points[index])) index++
        return index
    }

    /** The number of `\` directly before [index], counted back no further than [floor]. */
    private fun backslashesBefore(points: IntArray, floor: Int, index: Int): Int {
        var cursor = index - 1
        while (cursor >= floor && points[cursor] == '\\'.code) cursor--
        return index - 1 - cursor
    }

    private fun hasPathSeparator(points: IntArray, from: Int, to: Int): Boolean {
        for (index in from until to) if (points[index] == '/'.code || points[index] == '\\'.code) return true
        return false
    }

    /**
     * 1.1 Where a path-like range that begins at [start] stops.
     *
     * Folder and file names contain spaces (`/storage/emulated/0/My Books/…`),
     * so whitespace cannot end a path: stopping there kept every later
     * component. Only text that cannot continue the path ends it: a line
     * break, `<`, `>`, a backtick, a `"` that is not escaped, or the `: ` that
     * introduces an error's cause. Whitespace directly before that stop is not
     * part of the range. An apostrophe does not end a path, because names such
     * as `Jane's Backup` contain one.
     */
    private fun pathEnd(points: IntArray, start: Int): Int {
        var end = start
        while (end < points.size) {
            val stop = when (points[end]) {
                '\n'.code, '\r'.code, '<'.code, '>'.code, '`'.code -> true
                '"'.code -> backslashesBefore(points, start, end) % 2 == 0
                ':'.code -> end + 1 < points.size && isWhiteSpace(points[end + 1])
                else -> false
            }
            if (stop) break
            end++
        }
        while (end > start && isWhiteSpace(points[end - 1])) end--
        return end
    }

    // ---- Step 1: bound the input -----------------------------------------

    /**
     * The first [MAX_INPUT] scalars. When the cut falls inside a run of
     * non-whitespace, that partial run is dropped as well: half a host name or
     * half a path must not be judged as if it were whole.
     */
    private fun bounded(points: IntArray): IntArray {
        if (points.size <= MAX_INPUT) return points
        var end = MAX_INPUT
        if (!isWhiteSpace(points[MAX_INPUT])) {
            while (end > 0 && !isWhiteSpace(points[end - 1])) end--
        }
        return points.copyOf(end)
    }

    // ---- Step 2: secrets -------------------------------------------------

    /** Tried in this order at every position. */
    private val secretKeys = arrayOf(
        "authorization", "access_token", "refresh_token", "api_key", "api-key", "apikey", "password", "passwd",
        "passphrase", "pass", "pwd", "private_key", "private-key", "privatekey", "client_secret", "secret",
        "session_id", "sessionid", "jsessionid", "credentials", "credential", "signature", "cookie", "bearer",
        "token", "auth", "pin",
    )

    /** A half-open range of scalars. */
    private class Span(val start: Int, val end: Int)

    /**
     * Line ends and the last apostrophe of each line, computed once so that
     * no secret value rescans its line.
     */
    private class Lines(points: IntArray) {
        /** For each index and the text length: the first line break at or after it. */
        val end = IntArray(points.size + 1)
        /** For each index: the last `'` on its line, or -1. */
        val lastApostrophe = IntArray(points.size + 1)

        init {
            var lineEnd = points.size
            var apostrophe = -1
            end[points.size] = points.size
            lastApostrophe[points.size] = -1
            for (index in points.indices.reversed()) {
                if (points[index] == '\n'.code || points[index] == '\r'.code) {
                    lineEnd = index
                    apostrophe = -1
                } else if (points[index] == '\''.code && apostrophe == -1) {
                    apostrophe = index
                }
                end[index] = lineEnd
                lastApostrophe[index] = apostrophe
            }
        }
    }

    private fun redactSecrets(points: IntArray): IntArray {
        val lines = Lines(points)
        val output = Output(points.size)
        var cursor = 0
        while (cursor < points.size) {
            val value = secretAt(points, lines, cursor)
            if (value != null) {
                // Quotes, wrappers, the keyword and the separator stay.
                output.range(points, cursor, value.start)
                output.literal(REDACTED)
                cursor = value.end
            } else {
                output.point(points[cursor])
                cursor++
            }
        }
        return output.points()
    }

    /** The value of the secret whose keyword starts at [at], if a keyword matches there. */
    private fun secretAt(points: IntArray, lines: Lines, at: Int): Span? {
        val first = points[at]
        if (!isAsciiLetter(first)) return null
        // Left side: not inside a longer word, except at a camelCase hump
        // (`accessToken`, `oauth2Token`).
        val previous = points.at(at - 1)
        val apart = previous == -1 || !isAsciiAlphanumeric(previous)
        val hump = isAsciiUpper(first) && (isAsciiLower(previous) || isAsciiDigit(previous))
        if (!apart && !hump) return null
        for (key in secretKeys) {
            val afterKey = at + key.length
            if (!asciiEquals(points, at, afterKey, key)) continue
            // Right side: the keyword is not the start of a longer word.
            val following = points.at(afterKey)
            if (isAsciiAlphanumeric(following) || following == '_'.code) continue
            // 4.1 The closing quote of a quoted key, which an outer string may
            // have escaped: `"password":`, `\"password\":`.
            var next = afterKey
            val quote = if (following == '\\'.code) afterKey + 1 else afterKey
            val quotedKey = points.at(quote) == '"'.code || points.at(quote) == '\''.code
            if (quotedKey) next = quote + 1
            // 4.2 Whitespace, then 4.3 the separator, then 4.4 whitespace.
            val gap = next
            next = skipWhitespace(points, next)
            val separator = when {
                points.at(next) == '='.code && points.at(next + 1) == '>'.code -> 2
                points.at(next) == '='.code -> 1
                // `::` separates Rust path segments, not a key from its value.
                points.at(next) == ':'.code && points.at(next + 1) != ':'.code -> 1
                // Tuples and pairs: `("token", "…")`, `(authorization, Basic …)`.
                points.at(next) == ','.code && (quotedKey || key == "authorization") -> 1
                else -> 0
            }
            if (separator > 0) {
                next = skipWhitespace(points, next + separator)
            } else if (quotedKey || next == gap || (key != "authorization" && key != "bearer")) {
                continue
            }
            val value = secretValue(points, lines, next, key)
            if (value.end > value.start) return value
        }
        return null
    }

    private fun secretTokenEnd(points: IntArray, from: Int): Int {
        var index = from
        while (index < points.size && !isWhiteSpace(points[index]) && !isOneOf(points[index], ",;&)]}")) index++
        return index
    }

    /** The value that begins at [start]: the first form that applies. */
    private fun secretValue(points: IntArray, lines: Lines, start: Int, key: String): Span {
        // 1. Quoted, possibly inside wrappers such as `Some(`.
        quotedValue(points, lines, skipWrappers(points, start))?.let { return it }
        // 2. One cookie header carries several `name=value` pairs.
        if (key == "cookie") return Span(start, lines.end[start])
        // 3. One token; two after `authorization` (`Basic dXNlcjpwYXNz`).
        var end = secretTokenEnd(points, start)
        if (key == "authorization" && end > start) {
            val second = skipWhitespace(points, end)
            if (second > end) {
                val secondEnd = secretTokenEnd(points, second)
                if (secondEnd > second) end = secondEnd
            }
        }
        // A value with `/` or `\` is a path or a URL and may contain spaces.
        if (hasPathSeparator(points, start, end)) end = maxOf(end, pathEnd(points, start))
        return Span(start, end)
    }

    /** The index after any `Name(` wrappers that begin at [start]. */
    private fun skipWrappers(points: IntArray, start: Int): Int {
        var position = start
        while (true) {
            if (!isAsciiLetter(points.at(position))) return position
            var nameEnd = position
            while (nameEnd < points.size && (isAsciiAlphanumeric(points[nameEnd]) || points[nameEnd] == '_'.code)) nameEnd++
            if (points.at(nameEnd) != '('.code) return position
            position = nameEnd + 1
        }
    }

    /** The content of a quoted value whose opening quote is at [open], if one is there. */
    private fun quotedValue(points: IntArray, lines: Lines, open: Int): Span? {
        val escaped = points.at(open) == '\\'.code
        val quote = if (escaped) open + 1 else open
        when (points.at(quote)) {
            '"'.code -> {
                val start = quote + 1
                val line = lines.end[start]
                var close = start
                while (close < line) {
                    if (points[close] == '"'.code) {
                        val before = backslashesBefore(points, start, close)
                        if (if (escaped) before == 1 else before % 2 == 0) break
                    }
                    close++
                }
                // The backslash that escapes the closing quote stays with it.
                return Span(start, if (close < line && escaped) close - 1 else close)
            }
            '\''.code -> {
                if (escaped) return null
                val start = quote + 1
                val last = lines.lastApostrophe[start]
                var end = if (last >= start) last else lines.end[start]
                // The closing quote may be missing, and then the last `'` is an
                // apostrophe inside a path.
                if (hasPathSeparator(points, start, end)) end = maxOf(end, pathEnd(points, start))
                return Span(start, end)
            }
            else -> return null
        }
    }

    // ---- Step 3: URLs ----------------------------------------------------

    /** Schemes that are URLs without `//`. */
    private val opaqueSchemes = arrayOf("blob", "content", "data", "file", "mailto", "tauri", "tel", "zim")

    /**
     * Final labels of host names that only mean something on the person's own
     * network, where the name itself is theirs (`janes-phone.local`).
     */
    private val privateSuffixes = arrayOf("local", "localdomain", "lan", "home", "internal", "arpa")

    private fun isSchemeScalar(point: Int) = isAsciiAlphanumeric(point) || isOneOf(point, "+-.")

    private fun replaceUrls(points: IntArray): IntArray {
        val output = Output(points.size)
        var cursor = 0
        var colon = 0
        while (colon < points.size) {
            if (points[colon] != ':'.code) {
                colon++
                continue
            }
            // The scheme is the run of scheme scalars before the colon, from its
            // first letter (`+https:` is still `https:`). It cannot reach back
            // into text that an earlier URL already replaced.
            var start = colon
            while (start > cursor && isSchemeScalar(points[start - 1])) start--
            while (start < colon && !isAsciiLetter(points[start])) start++
            val tail = colon + 1
            val slashes = points.at(tail) == '/'.code && points.at(tail + 1) == '/'.code
            val opaque = opaqueSchemes.any { asciiEquals(points, start, colon, it) }
            if (start == colon || (!slashes && (points.at(tail) == ':'.code || !opaque))) {
                colon++
                continue
            }
            var tokenEnd = tail
            while (tokenEnd < points.size && !isWhiteSpace(points[tokenEnd]) && !isOneOf(points[tokenEnd], "\"<>`")) tokenEnd++
            val hasBody = tail < points.size && !isWhiteSpace(points[tail])
            val end: Int
            val replacement: String
            if (asciiEquals(points, start, colon, "file")) {
                // A file URL is a path, in any form (`file:///x`, `file:C:/x`),
                // and may be written with unencoded spaces.
                end = if (hasBody) pathEnd(points, tail) else tail
                replacement = URL
            } else if (asciiEquals(points, start, colon, "data")) {
                // A data URL carries content itself, spaces and markup included.
                var lineEnd = tail
                if (hasBody) while (lineEnd < points.size && !isLineBreak(points[lineEnd])) lineEnd++
                end = lineEnd
                replacement = URL
            } else if (slashes) {
                var authorityEnd = tail + 2
                while (authorityEnd < tokenEnd && !isOneOf(points[authorityEnd], "/\\?#")) authorityEnd++
                replacement = classify(points, start, colon, tail + 2, authorityEnd)
                // A URL that is not kept, or whose path, query or fragment is
                // dropped, goes whole, like a path: what follows the first space
                // of an unencoded article title must not stay behind.
                end = if (replacement.startsWith("[") || authorityEnd < tokenEnd)
                    maxOf(tokenEnd, pathEnd(points, tail)) else tokenEnd
            } else {
                end = tokenEnd
                replacement = URL
            }
            output.range(points, cursor, start)
            output.literal(replacement)
            cursor = end
            colon = end
        }
        output.range(points, cursor, points.size)
        return output.points()
    }

    /**
     * What may be kept of a `scheme://authority…` URL, decided from its text
     * alone so that every platform decides alike. The scheme is
     * `points[schemeStart, colon)` and the authority `points[from, to)`.
     *
     * Only `scheme://host[:port]` of an http(s) address with a public DNS name
     * is kept. Its path, query and fragment never are: a path can name the
     * article being read. This device (`localhost`, `127.…`, `[::1]`) is a
     * reader page. Everything else is `[url]`: other schemes, IP literals in
     * any spelling, single-label and private-network names, a malformed port.
     */
    private fun classify(points: IntArray, schemeStart: Int, colon: Int, from: Int, to: Int): String {
        val scheme: String
        val defaultPort: String
        when {
            asciiEquals(points, schemeStart, colon, "http") -> { scheme = "http"; defaultPort = "80" }
            asciiEquals(points, schemeStart, colon, "https") -> { scheme = "https"; defaultPort = "443" }
            else -> return URL
        }
        // 2. The userinfo ends at the last `@`.
        var hostStart = from
        for (index in from until to) if (points[index] == '@'.code) hostStart = index + 1
        // 3. The host, and after it the port part.
        var hostEnd = to
        if (hostStart < to && points[hostStart] == '['.code) {
            var close = hostStart
            while (close < to && points[close] != ']'.code) close++
            if (close == to) return URL
            hostEnd = close + 1
        } else {
            for (index in hostStart until to) if (points[index] == ':'.code) hostEnd = index
        }
        // 4. Lowercased, ASCII only, and without one trailing dot.
        val lowered = StringBuilder(hostEnd - hostStart)
        for (index in hostStart until hostEnd) lowered.appendCodePoint(asciiLower(points[index]))
        if (lowered.isNotEmpty() && lowered[lowered.length - 1] == '.') lowered.setLength(lowered.length - 1)
        val host = lowered.toString()
        // 5. This device.
        val loopback = host.startsWith("127.") && host.substring(4).all { it in '0'..'9' || it == '.' }
        if (host == "localhost" || host.endsWith(".localhost") || loopback || host == "[::1]") return READER_URL
        // 6. No port, or `:` and one to five digits.
        val portLength = to - hostEnd - 1
        if (hostEnd < to) {
            if (points[hostEnd] != ':'.code || portLength !in 1..5) return URL
            for (index in hostEnd + 1 until to) if (!isAsciiDigit(points[index])) return URL
        }
        // 7. A public DNS name.
        val labels = host.split('.')
        val last = labels[labels.size - 1]
        val dnsName = labels.size >= 2 &&
            labels.all { label -> label.isNotEmpty() && label.all { it in 'a'..'z' || it in '0'..'9' || it == '-' } } &&
            last[0] in 'a'..'z' && last !in privateSuffixes
        if (!dnsName) return URL
        // 8. The port as written, unless it is the scheme's default.
        if (hostEnd == to) return "$scheme://$host"
        val port = String(points, hostEnd + 1, portLength)
        return if (port.trimStart('0') == defaultPort) "$scheme://$host" else "$scheme://$host:$port"
    }

    // ---- Step 4: e-mail addresses ----------------------------------------

    /**
     * A bare e-mail address becomes `[email]`.
     *
     * This runs after the URL step, which has already dealt with `mailto:` and
     * with the userinfo of `scheme://user@host`, and before the path step, so
     * that what follows an address (`jane@example.org/Inbox Folder`) starts at
     * a boundary and goes as a path instead of staying behind. Both runs are
     * taken whole: an address is never matched inside a longer run.
     */
    private fun redactEmails(points: IntArray): IntArray {
        val output = Output(points.size)
        var cursor = 0
        var at = 0
        while (at < points.size) {
            if (points[at] != '@'.code) {
                at++
                continue
            }
            // The local part cannot reach back into an address already replaced.
            var start = at
            while (start > cursor && (isAsciiAlphanumeric(points[start - 1]) || isOneOf(points[start - 1], "._%+-"))) start--
            var end = at + 1
            while (end < points.size && (isAsciiAlphanumeric(points[end]) || isOneOf(points[end], ".-"))) end++
            // A dot or hyphen at the very end is sentence punctuation and stays.
            while (end > at + 1 && isOneOf(points[end - 1], ".-")) end--
            if (start == at || !emailHost(points, at + 1, end)) {
                at++
                continue
            }
            output.range(points, cursor, start)
            output.literal(EMAIL)
            cursor = end
            at = end
        }
        output.range(points, cursor, points.size)
        return output.points()
    }

    /**
     * Whether `points[from, to)` is the host of an address: at least two
     * labels, none empty, the last one two or more ASCII letters.
     */
    private fun emailHost(points: IntArray, from: Int, to: Int): Boolean {
        var labels = 1
        var labelStart = from
        for (index in from until to) {
            if (points[index] != '.'.code) continue
            if (index == labelStart) return false
            labels++
            labelStart = index + 1
        }
        if (labels < 2 || to - labelStart < 2) return false
        for (index in labelStart until to) if (!isAsciiLetter(points[index])) return false
        return true
    }

    // ---- Step 5: paths ---------------------------------------------------

    /** Whether a path may begin right after [previous]; -1 is the start of the text. */
    private fun pathBoundary(previous: Int): Boolean =
        previous == -1 || !(isAsciiAlphanumeric(previous) || isOneOf(previous, "_.-"))

    /**
     * For each index, whether a `\` occurs after it and before the next
     * whitespace scalar. Computed once, so a rooted path start (`\Users\…`)
     * is recognised without scanning ahead from every backslash.
     */
    private fun backslashAhead(points: IntArray): BooleanArray {
        val ahead = BooleanArray(points.size + 1)
        var seen = false
        for (index in points.indices.reversed()) {
            if (isWhiteSpace(points[index])) {
                seen = false
                continue
            }
            ahead[index] = seen
            if (points[index] == '\\'.code) seen = true
        }
        return ahead
    }

    /** Whether the text at [index] has one of the path start shapes. */
    private fun pathStart(points: IntArray, backslashAhead: BooleanArray, index: Int): Boolean {
        val current = points[index]
        val next = points.at(index + 1)
        fun name(point: Int) = point != -1 && point != '/'.code && !isWhiteSpace(point)
        val drive = isAsciiLetter(current) && next == ':'.code &&
            (points.at(index + 2) == '\\'.code || points.at(index + 2) == '/'.code)
        val absolute = current == '/'.code && name(next)
        val home = current == '~'.code && (next == '/'.code || next == '\\'.code)
        val share = (current == '\\'.code && next == '\\'.code) ||
            (current == '/'.code && next == '/'.code && name(points.at(index + 2)))
        // One backslash and a name is a path only if another backslash follows
        // before the next whitespace; `\n` in prose is not.
        val rooted = current == '\\'.code && isAsciiAlphanumeric(next) && backslashAhead[index]
        return drive || absolute || home || share || rooted
    }

    private fun pathAt(points: IntArray, backslashAhead: BooleanArray, index: Int): Int {
        val previous = points.at(index - 1)
        if (!pathBoundary(previous) || !pathStart(points, backslashAhead, index)) return -1
        // Neither slash of `://` starts a path: what follows is the authority
        // of a URL that step 3 kept (`https://example.org`).
        if (points[index] == '/'.code) {
            val first = previous == ':'.code && points.at(index + 1) == '/'.code
            val second = previous == '/'.code && points.at(index - 2) == ':'.code
            if (first || second) return -1
        }
        return pathEnd(points, index)
    }

    private fun redactPaths(points: IntArray): IntArray {
        val backslashAhead = backslashAhead(points)
        val output = Output(points.size)
        var index = 0
        while (index < points.size) {
            val end = pathAt(points, backslashAhead, index)
            if (end != -1) {
                output.literal(PATH)
                index = end
            } else {
                output.point(points[index])
                index++
            }
        }
        return output.points()
    }

    // ---- Step 6: opaque tokens -------------------------------------------

    private const val MIN_OPAQUE = 40

    private fun redactOpaque(points: IntArray): IntArray {
        val output = Output(points.size)
        var index = 0
        while (index < points.size) {
            var end = index
            var digit = false
            var letter = false
            while (end < points.size && (isAsciiAlphanumeric(points[end]) || isOneOf(points[end], "_-+/=."))) {
                if (isAsciiDigit(points[end])) digit = true
                if (isAsciiLetter(points[end])) letter = true
                end++
            }
            when {
                end == index -> { output.point(points[index]); index++ }
                end - index >= MIN_OPAQUE && digit && letter -> { output.literal(REDACTED); index = end }
                else -> { output.range(points, index, end); index = end }
            }
        }
        return output.points()
    }

    // ---- Step 7: whitespace, step 8: length ------------------------------

    private fun collapseWhitespace(points: IntArray): IntArray {
        val output = Output(points.size)
        var pendingSpace = false
        var any = false
        for (point in points) {
            if (isWhiteSpace(point)) {
                pendingSpace = any
                continue
            }
            if (pendingSpace) output.point(' '.code)
            pendingSpace = false
            any = true
            output.point(point)
        }
        return output.points()
    }

    private fun truncate(points: IntArray, limit: Int): IntArray {
        if (points.size <= limit) return points
        val cut = points.copyOf(limit.coerceAtLeast(1))
        cut[cut.size - 1] = '…'.code
        return cut
    }
}
