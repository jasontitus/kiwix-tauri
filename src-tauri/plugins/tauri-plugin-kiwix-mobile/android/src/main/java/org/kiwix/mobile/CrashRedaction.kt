package org.kiwix.mobile

/**
 * The redaction every crash-record text field goes through. A port of the
 * canonical pipeline in `src-tauri/src/crash.rs`, pinned by the shared vectors
 * in `src-tauri/tests/fixtures/crash-redaction-cases.json`: the same steps in
 * the same order, so the Rust, Swift and Kotlin intakes agree on every case.
 *
 * 1. Keep at most [MAX_INPUT] characters.
 * 2. Secret values. First, because a value that is a path with spaces must go
 *    as a whole, and a path must not swallow the key that marks the text after
 *    it as secret.
 * 3. URLs.
 * 4. Paths.
 * 5. Long opaque tokens.
 * 6. Collapse whitespace.
 * 7. Cut to [MAX_MESSAGE] characters.
 *
 * A "character" is a Unicode code point throughout, as in Rust: the text is
 * handled as an array of code points, so a bound or a cut never splits a
 * surrogate pair. Whitespace is the Unicode White_Space property, which is not
 * what `Char.isWhitespace` or `Character.isWhitespace` test.
 */
internal object CrashRedaction {
    const val MAX_INPUT = 32_768
    const val MAX_MESSAGE = 2_000

    private const val REDACTED = "[redacted]"
    private const val URL = "[url]"
    private const val READER_URL = "[reader-url]"
    private const val PATH = "[path]"

    /** Full redaction for one bounded text field. */
    fun redact(value: String): String {
        var text = bounded(codePoints(value))
        text = redactSecrets(text)
        text = replaceUrls(text)
        text = redactPaths(text)
        text = redactOpaque(text)
        return string(truncate(collapseWhitespace(text), MAX_MESSAGE))
    }

    /** Stack-frame redaction: steps 1 to 4, keeping spacing so frames stay readable. */
    fun redactLine(value: String): String {
        var text = bounded(codePoints(value))
        text = redactSecrets(text)
        text = replaceUrls(text)
        return string(redactPaths(text))
    }

    /** At most [limit] characters, the last one an ellipsis when something was cut. */
    fun truncate(value: String, limit: Int): String = string(truncate(codePoints(value), limit))

    // ---- code points -----------------------------------------------------

    private fun codePoints(value: String): IntArray {
        val result = IntArray(value.codePointCount(0, value.length))
        var index = 0
        var count = 0
        while (index < value.length) {
            val point = value.codePointAt(index)
            result[count++] = point
            index += Character.charCount(point)
        }
        return result
    }

    private fun string(points: IntArray, from: Int = 0, to: Int = points.size): String =
        String(points, from, to - from)

    /** Collects output as code points and plain replacement text. */
    private class Output(capacity: Int) {
        private val text = StringBuilder(capacity)
        fun point(point: Int) { text.appendCodePoint(point) }
        fun range(points: IntArray, from: Int, to: Int) { for (index in from until to) text.appendCodePoint(points[index]) }
        fun literal(value: String) { text.append(value) }
        fun points(): IntArray = codePoints(text.toString())
    }

    private fun bounded(points: IntArray): IntArray =
        if (points.size <= MAX_INPUT) points else points.copyOf(MAX_INPUT)

    private fun truncate(points: IntArray, limit: Int): IntArray {
        if (points.size <= limit) return points
        val cut = points.copyOf(limit.coerceAtLeast(1))
        cut[cut.size - 1] = '…'.code
        return cut
    }

    /** Unicode White_Space, the definition of Rust's `char::is_whitespace`. */
    internal fun isWhiteSpace(point: Int): Boolean = when (point) {
        in 0x09..0x0D, 0x20, 0x85, 0xA0, 0x1680, in 0x2000..0x200A, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000 -> true
        else -> false
    }

    private fun isAsciiLetter(point: Int) = point in 'a'.code..'z'.code || point in 'A'.code..'Z'.code
    private fun isAsciiDigit(point: Int) = point in '0'.code..'9'.code
    private fun isAsciiAlphanumeric(point: Int) = isAsciiLetter(point) || isAsciiDigit(point)
    private fun asciiLower(point: Int) = if (point in 'A'.code..'Z'.code) point + 32 else point
    private fun IntArray.at(index: Int): Int = if (index in indices) this[index] else -1
    private fun isOneOf(point: Int, characters: String) = point in 0..0xFFFF && characters.indexOf(point.toChar()) >= 0

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

    // ---- shared path rule ------------------------------------------------

    /** Whether [index] follows an odd run of backslashes that begins no earlier than [floor]. */
    private fun backslashEscaped(points: IntArray, floor: Int, index: Int): Boolean {
        var run = 0
        var cursor = index - 1
        while (cursor >= floor && points[cursor] == '\\'.code) { run++; cursor-- }
        return run % 2 == 1
    }

    /**
     * Where the redaction of a path that starts at [start] stops.
     *
     * Folder and file names contain spaces (`/storage/emulated/0/My Books/…`),
     * so whitespace cannot end a path: stopping there kept every later
     * component. Only text that cannot continue the path ends it: a closing
     * double quote, an angle bracket, a backtick, a line break, or the `: `
     * that introduces an error's cause. Prose between the path and that
     * delimiter is dropped with it; losing those words is the accepted price
     * of never keeping part of a path. An apostrophe does not end a path,
     * because names such as `Jane's Backup` contain one.
     */
    private fun pathEnd(points: IntArray, start: Int): Int {
        var end = start
        while (end < points.size) {
            val stop = when (points[end]) {
                '\n'.code, '\r'.code, '<'.code, '>'.code, '`'.code -> true
                // A path printed inside a quoted string escapes an embedded quote as `\"`.
                '"'.code -> !backslashEscaped(points, start, end)
                ':'.code -> end + 1 < points.size && isWhiteSpace(points[end + 1])
                else -> false
            }
            if (stop) break
            end++
        }
        // The separator before the delimiter is not part of the path.
        while (end > start && isWhiteSpace(points[end - 1])) end--
        return end
    }

    // ---- step 2: secrets -------------------------------------------------

    /** Tried in this order at every position; ASCII case is ignored. */
    private val secretKeys = arrayOf(
        "authorization", "access_token", "refresh_token", "api_key", "api-key", "apikey",
        "password", "passwd", "cookie", "bearer", "token", "secret", "pin",
    )

    private fun isWordCharacter(point: Int) = isAsciiAlphanumeric(point) || point == '_'.code
    private fun isBlank(point: Int) = point == ' '.code || point == '\t'.code
    private fun isBlankOrBreak(point: Int) = isBlank(point) || point == '\r'.code || point == '\n'.code

    private fun redactSecrets(points: IntArray): IntArray {
        val output = Output(points.size)
        var cursor = 0
        while (cursor < points.size) {
            val value = secretAt(points, cursor)
            if (value != null) {
                output.range(points, cursor, value.first)
                output.literal(REDACTED)
                cursor = value.last
            } else {
                output.point(points[cursor])
                cursor++
            }
        }
        return output.points()
    }

    /** A half-open range of code points. */
    private class Span(val first: Int, val last: Int)

    /** The value range of a secret whose keyword starts at [at], if any. */
    private fun secretAt(points: IntArray, at: Int): Span? {
        // No ASCII letter or digit directly before a key; an underscore may (`client_secret`).
        if (at > 0 && isAsciiAlphanumeric(points[at - 1])) return null
        for (key in secretKeys) {
            val afterKey = at + key.length
            if (afterKey > points.size) continue
            var matches = true
            for (offset in key.indices) {
                if (asciiLower(points[at + offset]) != key[offset].code) { matches = false; break }
            }
            if (!matches || isWordCharacter(points.at(afterKey))) continue
            var next = afterKey
            // The closing quote of a quoted key: `"password":`.
            val quotedKey = points.at(next) == '"'.code || points.at(next) == '\''.code
            if (quotedKey) next++
            val gap = next
            while (next < points.size && isBlankOrBreak(points[next])) next++
            // `::` is a Rust path separator, not a key-value separator.
            val separator = points.at(next) == '='.code ||
                (points.at(next) == ':'.code && points.at(next + 1) != ':'.code)
            if (separator) {
                next++
                while (next < points.size && isBlank(points[next])) next++
            } else if (quotedKey || next == gap || (key != "authorization" && key != "bearer")) {
                // Only `authorization` and `bearer` accept plain whitespace instead.
                continue
            }
            val value = secretValue(points, next, key)
            if (value.last > value.first) return value
        }
        return null
    }

    private fun lineEnd(points: IntArray, from: Int): Int {
        var index = from
        while (index < points.size && points[index] != '\n'.code && points[index] != '\r'.code) index++
        return index
    }

    private fun secretTokenEnd(points: IntArray, from: Int): Int {
        var index = from
        while (index < points.size && !isBlankOrBreak(points[index]) && !isOneOf(points[index], ",;&)]}")) index++
        return index
    }

    private fun hasPathSeparator(points: IntArray, from: Int, to: Int): Boolean {
        for (index in from until to) if (points[index] == '/'.code || points[index] == '\\'.code) return true
        return false
    }

    /**
     * The range of the secret value that begins at [start].
     *
     * * `"…"`: up to the closing quote (one not escaped by a backslash), a line
     *   break or the end. The quotes stay.
     * * `'…'`: up to the last `'` on the line, or to the end of the line when
     *   there is none, because a value may itself contain an apostrophe. With a
     *   `/` or `\` inside it takes at least what [pathEnd] takes.
     * * After `cookie`: the rest of the line; one header carries several pairs.
     * * Otherwise one token, ending at a space, tab, line break or one of
     *   `, ; & ) ] }`. After `authorization` a second token goes too
     *   (`Basic dXNlcjpwYXNz`). A value containing `/` or `\` is a path or a
     *   URL and may contain spaces: it also takes everything [pathEnd] takes.
     */
    private fun secretValue(points: IntArray, start: Int, key: String): Span {
        when {
            points.at(start) == '"'.code -> {
                val line = lineEnd(points, start + 1)
                var close = start + 1
                while (close < line && !(points[close] == '"'.code && !backslashEscaped(points, start + 1, close))) close++
                return Span(start + 1, close)
            }
            points.at(start) == '\''.code -> {
                val line = lineEnd(points, start + 1)
                var end = line
                for (index in line - 1 downTo start + 1) {
                    if (points[index] == '\''.code) { end = index; break }
                }
                // The closing quote may be missing, and then the last `'` is an
                // apostrophe inside a path.
                if (hasPathSeparator(points, start + 1, end)) end = maxOf(end, pathEnd(points, start + 1))
                return Span(start + 1, end)
            }
            key == "cookie" -> return Span(start, lineEnd(points, start))
            else -> {
                var end = secretTokenEnd(points, start)
                if (key == "authorization" && end > start) {
                    var second = end
                    while (second < points.size && isBlankOrBreak(points[second])) second++
                    if (second > end && secretTokenEnd(points, second) > second) end = secretTokenEnd(points, second)
                }
                if (hasPathSeparator(points, start, end)) end = maxOf(end, pathEnd(points, start))
                return Span(start, end)
            }
        }
    }

    // ---- step 3: URLs ----------------------------------------------------

    /** Schemes that are URLs without `//`. */
    private val opaqueSchemes = arrayOf("blob", "content", "data", "file", "mailto", "tauri", "tel", "zim")

    private fun isSchemeCharacter(point: Int) =
        isAsciiAlphanumeric(point) || point == '+'.code || point == '-'.code || point == '.'.code

    private fun replaceUrls(points: IntArray): IntArray {
        val output = Output(points.size)
        var cursor = 0
        for (colon in points.indices) {
            if (points[colon] != ':'.code || colon < cursor) continue
            var start = colon
            while (start > cursor && isSchemeCharacter(points[start - 1])) start--
            if (start == colon || !isAsciiLetter(points[start])) continue
            val scheme = string(points, start, colon)
            val tail = colon + 1
            val authority = points.at(tail) == '/'.code && points.at(tail + 1) == '/'.code
            if (!authority && (points.at(tail) == ':'.code || opaqueSchemes.none { it.equals(scheme, ignoreCase = true) })) continue
            var tokenEnd = tail
            while (tokenEnd < points.size && !isWhiteSpace(points[tokenEnd]) && !isOneOf(points[tokenEnd], "\"<>`")) tokenEnd++
            val hasBody = tail < points.size && !isWhiteSpace(points[tail])
            val end: Int
            val replacement: String
            if (scheme.equals("file", ignoreCase = true) && hasBody) {
                // A file URL is a path and may be written with unencoded spaces: it
                // ends where a path would. `file: ` followed by a space is prose; a
                // path after it is found by the path step.
                end = pathEnd(points, tail)
                replacement = URL
            } else if (scheme.equals("data", ignoreCase = true) && hasBody) {
                // A data URL carries content itself, spaces and markup included.
                end = lineEnd(points, tail)
                replacement = URL
            } else {
                replacement = safeUrl(points, start, tokenEnd, scheme)
                // Every `scheme://` URL that is not kept (reader pages, content and
                // archive links, shares) goes whole, like a path.
                end = if (authority && (replacement == URL || replacement == READER_URL))
                    maxOf(tokenEnd, pathEnd(points, tail)) else tokenEnd
            }
            output.range(points, cursor, start)
            output.literal(replacement)
            cursor = end
        }
        output.range(points, cursor, points.size)
        return output.points()
    }

    /**
     * What may be kept of the URL in `points[start, end)`: nothing but
     * `scheme://host[:port]path` of an external http(s) address. A reader page
     * (a local host, a `/.kiwix/` route, or another URL nested in the path) is
     * `[reader-url]`. Anything else, including an address this parser does not
     * fully understand, is `[url]`.
     */
    private fun safeUrl(points: IntArray, start: Int, end: Int, scheme: String): String {
        if (!scheme.equals("http", ignoreCase = true) && !scheme.equals("https", ignoreCase = true)) return URL
        val address = HttpAddress.parse(points, start + scheme.length + 1, end, scheme.lowercase(java.util.Locale.ROOT))
            ?: return URL
        val decoded = address.decodedPath()
        val nested = decoded.contains("://") ||
            arrayOf("mailto:", "data:", "file:", "content:", "zim:", "blob:", "tauri:", "tel:").any { decoded.contains(it) }
        if (address.local || decoded.contains("/.kiwix/") || nested) return READER_URL
        return address.scheme + "://" + address.host + address.port + address.path
    }

    /**
     * The authority and path of an http(s) address, read the way the WHATWG URL
     * standard (and so the Rust `url` crate) reads them. Hosts that would need
     * percent-decoding or IDNA processing are not parsed: the caller redacts
     * the whole address instead of guessing.
     */
    private class HttpAddress(
        val scheme: String, val host: String, val port: String, val path: String, val local: Boolean,
    ) {
        /** The path with its percent escapes decoded as UTF-8, malformed bytes replaced. */
        fun decodedPath(): String {
            val bytes = java.io.ByteArrayOutputStream(path.length)
            var index = 0
            while (index < path.length) {
                val character = path[index]
                val high = if (character == '%' && index + 2 < path.length + 0 && index + 2 <= path.length - 1 + 0)
                    Character.digit(path[index + 1], 16) else -1
                val low = if (high >= 0) Character.digit(path[index + 2], 16) else -1
                if (high >= 0 && low >= 0) {
                    bytes.write(high * 16 + low)
                    index += 3
                } else {
                    // The serialized path is ASCII: everything else was percent-encoded.
                    bytes.write(character.code)
                    index++
                }
            }
            return String(bytes.toByteArray(), Charsets.UTF_8)
        }

        companion object {
            fun parse(points: IntArray, from: Int, to: Int, scheme: String): HttpAddress? {
                var end = to
                // Trailing C0 controls and spaces are not part of an address.
                while (end > from && points[end - 1] <= 0x20) end--
                var index = from
                // Any run of slashes and backslashes introduces the authority.
                while (index < end && (points[index] == '/'.code || points[index] == '\\'.code)) index++
                var authorityEnd = index
                while (authorityEnd < end && !isOneOf(points[authorityEnd], "/\\?#")) authorityEnd++
                // Credentials end at the last `@` and are never kept.
                var hostStart = index
                for (cursor in index until authorityEnd) if (points[cursor] == '@'.code) hostStart = cursor + 1
                val hostEnd: Int
                if (points.at(hostStart) == '['.code) {
                    var close = hostStart
                    while (close < authorityEnd && points[close] != ']'.code) close++
                    if (close == authorityEnd) return null
                    hostEnd = close + 1
                } else {
                    var colon = hostStart
                    while (colon < authorityEnd && points[colon] != ':'.code) colon++
                    hostEnd = colon
                }
                val port = port(points, hostEnd, authorityEnd, scheme) ?: return null
                val host = host(points, hostStart, hostEnd) ?: return null
                var pathEnd = authorityEnd
                while (pathEnd < end && points[pathEnd] != '?'.code && points[pathEnd] != '#'.code) pathEnd++
                return HttpAddress(scheme, host.first, port, path(points, authorityEnd, pathEnd), host.second)
            }

            /** `""` for none or the scheme's default, `":n"` otherwise; null when it is not a port. */
            private fun port(points: IntArray, from: Int, to: Int, scheme: String): String? {
                if (from == to) return ""
                if (points[from] != ':'.code) return null
                var value = 0
                for (index in from + 1 until to) {
                    if (!isAsciiDigit(points[index])) return null
                    value = value * 10 + (points[index] - '0'.code)
                    if (value > 65_535) return null
                }
                if (from + 1 == to) return ""
                return if (value == (if (scheme == "https") 443 else 80)) "" else ":$value"
            }

            /** The serialized host and whether it is this device. */
            private fun host(points: IntArray, from: Int, to: Int): Pair<String, Boolean>? {
                if (from == to) return null
                if (points[from] == '['.code) {
                    val pieces = ipv6(points, from + 1, to - 1) ?: return null
                    val loopback = (0..6).all { pieces[it] == 0 } && pieces[7] == 1
                    val mappedLoopback = (0..4).all { pieces[it] == 0 } && pieces[5] == 0xFFFF && pieces[6] shr 8 == 127
                    return Pair("[" + serializeIpv6(pieces) + "]", loopback || mappedLoopback)
                }
                val text = StringBuilder(to - from)
                for (index in from until to) {
                    val point = points[index]
                    // Forbidden in a domain, or in need of decoding this parser does not do.
                    if (point <= 0x20 || point >= 0x7F || isOneOf(point, "\"#%/:<>?@[\\]^`|")) return null
                    text.append(asciiLower(point).toChar())
                }
                val name = text.toString()
                if (endsInNumber(name)) {
                    val address = ipv4(name) ?: return null
                    val dotted = "${address ushr 24 and 255}.${address ushr 16 and 255}.${address ushr 8 and 255}.${address and 255}"
                    return Pair(dotted, address ushr 24 and 255 == 127L)
                }
                val trimmed = name.trimEnd('.')
                return Pair(name, trimmed == "localhost" || trimmed.endsWith(".localhost"))
            }

            private fun endsInNumber(name: String): Boolean {
                val parts = name.split('.').toMutableList()
                if (parts.last().isEmpty()) {
                    if (parts.size == 1) return false
                    parts.removeAt(parts.size - 1)
                }
                val last = parts.last()
                if (last.isNotEmpty() && last.all { it in '0'..'9' }) return true
                return ipv4Number(last) != null
            }

            /** One part of a dotted address: decimal, `0x` hexadecimal or leading-zero octal. */
            private fun ipv4Number(part: String): Long? {
                if (part.isEmpty()) return null
                var digits = part
                var radix = 10
                if (digits.length >= 2 && (digits.startsWith("0x") || digits.startsWith("0X"))) {
                    digits = digits.substring(2); radix = 16
                } else if (digits.length >= 2 && digits[0] == '0') {
                    digits = digits.substring(1); radix = 8
                }
                if (digits.isEmpty()) return 0
                var value = 0L
                for (character in digits) {
                    val digit = Character.digit(character, radix)
                    if (digit < 0 || character.code > 0x7F) return null
                    value = value * radix + digit
                    if (value > 0xFFFF_FFFFL) return null
                }
                return value
            }

            private fun ipv4(name: String): Long? {
                val parts = name.split('.').toMutableList()
                if (parts.last().isEmpty() && parts.size > 1) parts.removeAt(parts.size - 1)
                if (parts.size > 4) return null
                val numbers = parts.map { ipv4Number(it) ?: return null }
                for (index in 0 until numbers.size - 1) if (numbers[index] > 255) return null
                var limit = 1L
                repeat(5 - numbers.size) { limit *= 256 }
                if (numbers.last() >= limit) return null
                var address = numbers.last()
                for (index in 0 until numbers.size - 1) {
                    var weight = 1L
                    repeat(3 - index) { weight *= 256 }
                    address += numbers[index] * weight
                }
                return address
            }

            /** The eight 16-bit pieces of the address in `points[from, to)`, or null. */
            private fun ipv6(points: IntArray, from: Int, to: Int): IntArray? {
                val pieces = IntArray(8)
                var piece = 0
                var compress = -1
                var index = from
                if (points.at(index) == ':'.code && index < to) {
                    if (index + 1 >= to || points[index + 1] != ':'.code) return null
                    index += 2
                    piece++
                    compress = piece
                }
                while (index < to) {
                    if (piece == 8) return null
                    if (points[index] == ':'.code) {
                        if (compress != -1) return null
                        index++
                        piece++
                        compress = piece
                        continue
                    }
                    var value = 0
                    var length = 0
                    while (length < 4 && index < to && Character.digit(points[index], 16) >= 0 && points[index] < 0x80) {
                        value = value * 16 + Character.digit(points[index], 16)
                        index++
                        length++
                    }
                    if (index < to && points[index] == '.'.code) {
                        // An embedded dotted address fills the last two pieces.
                        if (length == 0) return null
                        index -= length
                        if (piece > 6) return null
                        var seen = 0
                        while (index < to) {
                            var part = -1
                            if (seen > 0) {
                                if (points[index] == '.'.code && seen < 4) index++ else return null
                            }
                            if (index >= to || !isAsciiDigit(points[index])) return null
                            while (index < to && isAsciiDigit(points[index])) {
                                val digit = points[index] - '0'.code
                                part = when {
                                    part == -1 -> digit
                                    part == 0 -> return null
                                    else -> part * 10 + digit
                                }
                                if (part > 255) return null
                                index++
                            }
                            pieces[piece] = pieces[piece] * 256 + part
                            seen++
                            if (seen == 2 || seen == 4) piece++
                        }
                        if (seen != 4) return null
                        break
                    } else if (index < to && points[index] == ':'.code) {
                        index++
                        if (index >= to) return null
                    } else if (index < to) {
                        return null
                    }
                    pieces[piece] = value
                    piece++
                }
                if (compress != -1) {
                    var swaps = piece - compress
                    piece = 7
                    while (piece != 0 && swaps > 0) {
                        val other = compress + swaps - 1
                        val held = pieces[other]
                        pieces[other] = pieces[piece]
                        pieces[piece] = held
                        piece--
                        swaps--
                    }
                } else if (piece != 8) {
                    return null
                }
                return pieces
            }

            private fun serializeIpv6(pieces: IntArray): String {
                // The first longest run of two or more zero pieces is written as `::`.
                var bestStart = -1
                var bestLength = 0
                var index = 0
                while (index < 8) {
                    if (pieces[index] != 0) { index++; continue }
                    var run = index
                    while (run < 8 && pieces[run] == 0) run++
                    if (run - index > bestLength) { bestStart = index; bestLength = run - index }
                    index = run
                }
                if (bestLength < 2) bestStart = -1
                val text = StringBuilder()
                var ignoreZeros = false
                for (position in 0 until 8) {
                    if (ignoreZeros && pieces[position] == 0) continue
                    ignoreZeros = false
                    if (position == bestStart) {
                        text.append(if (position == 0) "::" else ":")
                        ignoreZeros = true
                        continue
                    }
                    text.append(Integer.toHexString(pieces[position]))
                    if (position != 7) text.append(':')
                }
                return text.toString()
            }

            /** The path as the standard serializes it: dot segments resolved, unsafe characters escaped. */
            private fun path(points: IntArray, from: Int, to: Int): String {
                val segments = ArrayList<String>()
                var index = from
                // The first separator only introduces the path.
                if (index < to && (points[index] == '/'.code || points[index] == '\\'.code)) index++
                var segment = StringBuilder()
                var trailingSeparator = true
                fun close(last: Boolean) {
                    val name = segment.toString()
                    val lower = name.lowercase(java.util.Locale.ROOT)
                    when {
                        lower == ".." || lower == ".%2e" || lower == "%2e." || lower == "%2e%2e" -> {
                            if (segments.isNotEmpty()) segments.removeAt(segments.size - 1)
                            if (last) segments.add("")
                        }
                        lower == "." || lower == "%2e" -> if (last) segments.add("")
                        else -> segments.add(name)
                    }
                    segment = StringBuilder()
                }
                while (index < to) {
                    val point = points[index]
                    if (point == '/'.code || point == '\\'.code) {
                        close(last = false)
                        trailingSeparator = true
                    } else {
                        trailingSeparator = false
                        if (point <= 0x20 || point >= 0x7F || isOneOf(point, "\"#<>?^`{}")) {
                            for (byte in String(Character.toChars(point)).toByteArray(Charsets.UTF_8)) {
                                segment.append('%')
                                segment.append("0123456789ABCDEF"[(byte.toInt() shr 4) and 15])
                                segment.append("0123456789ABCDEF"[byte.toInt() and 15])
                            }
                        } else {
                            segment.append(point.toChar())
                        }
                    }
                    index++
                }
                close(last = true)
                return "/" + segments.joinToString("/")
            }
        }
    }

    // ---- step 4: paths ---------------------------------------------------

    /** Whether a path may begin right after [previous]; -1 is the start of the text. */
    private fun pathBoundary(previous: Int): Boolean =
        previous == -1 || isWhiteSpace(previous) || isOneOf(previous, "=([{\"',;:`<>|\u201C\u2018")

    /**
     * Whether the text at [index] has the shape of a path start: a drive
     * (`C:\`, `C:/`), an absolute path (`/x`), a home path (`~/`, `~\`), or a
     * share (`\\`, `//host`).
     */
    private fun pathStart(points: IntArray, index: Int): Boolean {
        val current = points.at(index)
        if (current == -1) return false
        val next = points.at(index + 1)
        fun name(point: Int) = point != -1 && point != '/'.code && !isWhiteSpace(point)
        val drive = isAsciiLetter(current) && next == ':'.code &&
            (points.at(index + 2) == '\\'.code || points.at(index + 2) == '/'.code)
        val rooted = current == '/'.code && name(next)
        val home = current == '~'.code && (next == '/'.code || next == '\\'.code)
        val share = (current == '\\'.code && next == '\\'.code) ||
            (current == '/'.code && next == '/'.code && name(points.at(index + 2)))
        return drive || rooted || home || share
    }

    private fun pathAt(points: IntArray, index: Int): Int {
        val previous = points.at(index - 1)
        if (!pathBoundary(previous) || !pathStart(points, index)) return -1
        // `//host` right after a colon is the authority of a URL the URL step
        // kept (`https://example.org/a`), not a share.
        if (previous == ':'.code && points[index] == '/'.code && points.at(index + 1) == '/'.code) return -1
        return pathEnd(points, index)
    }

    private fun redactPaths(points: IntArray): IntArray {
        val output = Output(points.size)
        var index = 0
        while (index < points.size) {
            val end = pathAt(points, index)
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

    // ---- step 5: opaque tokens -------------------------------------------

    private const val MIN_OPAQUE = 40

    private fun redactOpaque(points: IntArray): IntArray {
        val output = Output(points.size)
        var runStart = -1
        fun flush(end: Int) {
            if (runStart == -1) return
            var digit = false
            var letter = false
            for (index in runStart until end) {
                if (isAsciiDigit(points[index])) digit = true
                if (isAsciiLetter(points[index])) letter = true
            }
            if (end - runStart >= MIN_OPAQUE && digit && letter) output.literal(REDACTED)
            else output.range(points, runStart, end)
            runStart = -1
        }
        for (index in points.indices) {
            val point = points[index]
            if (isAsciiAlphanumeric(point) || isOneOf(point, "_-+/=.")) {
                if (runStart == -1) runStart = index
            } else {
                flush(index)
                output.point(point)
            }
        }
        flush(points.size)
        return output.points()
    }
}
