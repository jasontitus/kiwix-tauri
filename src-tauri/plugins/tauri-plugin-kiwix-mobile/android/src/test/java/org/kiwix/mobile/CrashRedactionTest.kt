package org.kiwix.mobile

import com.fasterxml.jackson.databind.ObjectMapper
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The crash-record redaction is one pipeline on every platform. The cases in
 * `src-tauri/tests/fixtures/crash-redaction-cases.json` are the contract: the
 * Rust panic hook asserts the same file.
 */
class CrashRedactionTest {
    /** The shared fixture, found from the Gradle module directory; never a copy. */
    private fun fixture(): File {
        var directory: File? = File("").absoluteFile
        while (directory != null) {
            for (candidate in listOf(
                File(directory, "tests/fixtures/crash-redaction-cases.json"),
                File(directory, "src-tauri/tests/fixtures/crash-redaction-cases.json"),
            )) if (candidate.isFile) return candidate
            directory = directory.parentFile
        }
        throw AssertionError("crash-redaction-cases.json not found above ${File("").absolutePath}")
    }

    @Test
    fun redactionMatchesTheCanonicalCasesSharedWithRustAndSwift() {
        val cases = ObjectMapper().readTree(fixture())
        assertTrue("the shared fixture lost cases: ${cases.size()}", cases.isArray && cases.size() >= 80)
        val differing = ArrayList<String>()
        for (case in cases) {
            val names = case.fieldNames().asSequence().toList().sorted()
            assertEquals("a case is exactly {input, expected}", listOf("expected", "input"), names)
            val input = case.get("input").textValue()
            val expected = case.get("expected").textValue()
            val actual = CrashRedaction.redact(input)
            if (actual != expected) differing.add("input:    $input\nexpected: $expected\nactual:   $actual")
        }
        assertTrue(
            "${differing.size} of ${cases.size()} cases differ:\n" + differing.joinToString("\n\n"),
            differing.isEmpty(),
        )
    }

    @Test
    fun inputIsBoundedInScalarsBeforeAnyStepRuns() {
        val limit = CrashRedaction.MAX_INPUT
        val emoji = "\uD83D\uDE00"
        // Scalar values, not UTF-16 units or bytes: the cut lands between words
        // and nothing is split.
        val accented = CrashRedaction.redactLine("\u00e9 ".repeat(limit) + "tail")
        assertEquals(limit, accented.codePointCount(0, accented.length))
        assertEquals("$emoji ".repeat(limit / 2), CrashRedaction.redactLine("$emoji ".repeat(limit)))
        // A short field, and one exactly at the bound, are untouched.
        assertEquals("File.kt:12", CrashRedaction.redact("File.kt:12"))
        assertEquals("a".repeat(limit), CrashRedaction.redactLine("a".repeat(limit)))
        assertEquals(emoji.repeat(limit), CrashRedaction.redactLine(emoji.repeat(limit)))
        // The cut falls between words, on either side of the space: every whole
        // word before it stays.
        assertEquals("a".repeat(limit - 1) + " ", CrashRedaction.redactLine("a".repeat(limit - 1) + " next"))
        assertEquals("a".repeat(limit), CrashRedaction.redactLine("a".repeat(limit) + " next"))

        // The cut falls inside a token: the partial token goes. Kept, the first
        // half of this reader address's host would pass for a public name.
        val prefix = " ".repeat(limit - 19)
        val straddling = prefix + "http://12b34567.localhost:9/A/Private_Page"
        assertEquals(prefix, CrashRedaction.redactLine(straddling))
        assertEquals("", CrashRedaction.redact(straddling))
        assertEquals("[reader-url]", CrashRedaction.redact("http://12b34567.localhost:9/A/Private_Page"))
        // A path cut in half is dropped too, and nothing of it stays.
        val path = CrashRedaction.redactLine("x ".repeat(limit / 2 - 8) + "/home/jane doe/Private Notes/a.zim")
        assertFalse(path.contains("jane") || path.contains("Private") || path.contains("home"))
        // A surrogate pair straddling the cut goes whole with its run, never as half a pair.
        val pairs = CrashRedaction.redactLine("ab " + emoji.repeat(limit))
        assertEquals("ab ", pairs)
        // One unbroken run longer than the bound leaves nothing.
        val long = "/" + "a".repeat(limit * 4)
        assertEquals("", CrashRedaction.redact(long))
        assertEquals("", CrashRedaction.redactLine(long))
        // What lies beyond the bound is never read, so it cannot be kept either.
        assertEquals("x ".repeat(limit / 2),
            CrashRedaction.redactLine("x ".repeat(limit / 2) + "/storage/emulated/0/My Books/a.zim"))
    }

    @Test
    fun aMessageIsCutToTwoThousandCodePointsWithoutSplittingOne() {
        val limit = CrashRedaction.MAX_MESSAGE
        val emoji = "\uD83D\uDE00"
        assertEquals(emoji.repeat(limit), CrashRedaction.redact(emoji.repeat(limit)))
        val cut = CrashRedaction.redact(emoji.repeat(limit + 500))
        assertEquals(emoji.repeat(limit - 1) + "\u2026", cut)
        assertEquals(limit, cut.codePointCount(0, cut.length))
        // Words separated by spaces are not an opaque run and are cut the same way.
        val words = CrashRedaction.redact("word ".repeat(1000))
        assertEquals(limit, words.codePointCount(0, words.length))
        assertTrue(words.endsWith("\u2026"))
        // The thread-name and frame cuts use the same rule.
        assertEquals(emoji.repeat(63) + "\u2026", CrashRedaction.truncate(emoji.repeat(70), 64))
        assertEquals("main", CrashRedaction.truncate("main", 64))
    }

    @Test
    fun anUnpairedSurrogateIsOneOpaqueCodePoint() {
        // Only a Kotlin string can hold one, so the shared vectors cannot: the
        // spec's rule is one non-whitespace code point, never split, dropped or merged.
        val high = "\uD83D"
        val low = "\uDE00"
        assertFalse(CrashRedaction.isWhiteSpace(0xD83D))
        assertFalse(CrashRedaction.isWhiteSpace(0xDE00))
        assertEquals("a $high b $low c", CrashRedaction.redact("a $high  b\t$low c"))
        assertEquals(low + high, CrashRedaction.redact(low + high))
        // Two halves apart stay apart: the collapsed space is still between them.
        assertEquals("$high $low", CrashRedaction.redact("$high \t $low"))
        // Each counts as one for the cuts.
        assertEquals(high.repeat(1_999) + "…", CrashRedaction.redact(high.repeat(2_001)))
        assertEquals(low.repeat(63) + "…", CrashRedaction.truncate(low.repeat(70), 64))
        // It bounds a path or a secret like any other symbol and stays beside the placeholder.
        assertEquals(high + "[path]", CrashRedaction.redact("$high/storage/emulated/0/Books/a.zim"))
        assertEquals("token=[redacted] $low", CrashRedaction.redact("token=abc $low"))
    }

    @Test
    fun theResidualsTheSpecNamesStayAsWritten() {
        assertEquals("token=[redacted] \"x y\") z", CrashRedaction.redact("token=Some( \"x y\") z"))
        assertEquals("password=\"\"", CrashRedaction.redact("password=\"\""))
        assertEquals("token=Some(\"\")", CrashRedaction.redact("token=Some(\"\")"))
        assertEquals("cookie=\"\" a=b", CrashRedaction.redact("cookie=\"\" a=b"))
        assertEquals("http://example.org:000", CrashRedaction.redact("http://example.org:000/a"))
        assertEquals("[reader-url]", CrashRedaction.redact("http://localhost:abc/x"))
    }

    @Test
    fun everyStepIsLinearInItsInput() {
        // The worst inputs found in review: each unit starts a URL, path or
        // secret candidate. Rescanning the rest of the line or text once per
        // candidate makes these quadratic.
        val units = listOf(
            "a://b<", "file:x<", "zz://q`", "password=\"\" ", "data:x\n", "/a: ", " \\a", "a:", "'password'",
            "token='x ", "C:\\a\" ", "pin='' ", "token=\\\"\\\" ", "\\a<", "Some(Some(", "https://example.org/a ",
            "a@b.c ", "a.b@", "@a.b-", "x@y.zz@", "TokenA", "passwordHash ", "~jane ", "~a~",
        )
        fun repeated(unit: String, length: Int) = unit.repeat(length / unit.length)
        fun fastest(text: String): Long {
            var best = Long.MAX_VALUE
            repeat(5) {
                val started = System.nanoTime()
                CrashRedaction.unboundedStepsForTiming(text)
                best = minOf(best, System.nanoTime() - started)
            }
            return best
        }
        // Let the JIT compile the steps before anything is measured.
        for (unit in units) repeat(3) { CrashRedaction.unboundedStepsForTiming(repeated(unit, CrashRedaction.MAX_INPUT)) }
        for (unit in units) {
            val small = fastest(repeated(unit, CrashRedaction.MAX_INPUT))
            val large = fastest(repeated(unit, CrashRedaction.MAX_INPUT * 4))
            // Four times the input takes about four times as long when linear and
            // sixteen times when quadratic. The floor absorbs timer noise.
            val allowed = maxOf(small, 5_000_000L) * 10
            println("linear-time ${unit.trim().ifEmpty { unit }}: ${small / 1000} us at the bound, ${large / 1000} us for four times as much")
            assertTrue("$unit: ${small / 1000} us at the bound, ${large / 1000} us for four times as much", large <= allowed)
            // Generous for a shared machine; a quadratic step needs far longer.
            assertTrue("$unit: ${small / 1_000_000} ms at the bound", small < 300_000_000L)
        }
    }

    @Test
    fun whitespaceIsExactlyTheUnicodeWhiteSpaceProperty() {
        // Every scalar value against the specification's list, and that list
        // against the runtime's own table of the Unicode property.
        val property = java.util.regex.Pattern.compile("\\p{IsWhite_Space}")
        val listed = setOf(0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x20, 0x85, 0xA0, 0x1680, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000) +
            (0x2000..0x200A)
        for (point in 0..0x10FFFF) {
            val expected = point in listed
            if (CrashRedaction.isWhiteSpace(point) != expected)
                throw AssertionError("U+" + Integer.toHexString(point) + " differs from the specification's list")
            if (point !in 0xD800..0xDFFF && property.matcher(String(Character.toChars(point))).matches() != expected)
                throw AssertionError("U+" + Integer.toHexString(point) + " differs from the Unicode property")
        }
    }

    @Test
    fun whitespaceIsNotWhatThePlatformCallsWhitespace() {
        // Java's Character.isWhitespace excludes the no-break spaces and
        // includes the information separators; the pipeline must do neither.
        for (point in listOf(0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x20, 0x85, 0xA0, 0x1680, 0x2000, 0x200A,
            0x2028, 0x2029, 0x202F, 0x205F, 0x3000)) assertTrue(Integer.toHexString(point), CrashRedaction.isWhiteSpace(point))
        for (point in listOf(0x1C, 0x1D, 0x1E, 0x1F, 0x180E, 0x200B, 0x2060, 0xFEFF, 'a'.code))
            assertFalse(Integer.toHexString(point), CrashRedaction.isWhiteSpace(point))
        // A path after a narrow no-break space, and collapsed around one.
        assertEquals("open [path]", CrashRedaction.redact("open\u202F/storage/emulated/0/My Books/a.zim"))
        assertEquals("x [path]", CrashRedaction.redact("x\u0085/sdcard/My Books/a.zim"))
        assertEquals("a b", CrashRedaction.redact("a\u00A0\u2003 b"))
        // An information separator is not whitespace and is not collapsed.
        assertEquals("a\u001Fb", CrashRedaction.redact("a\u001Fb"))
    }

    @Test
    fun androidStoragePathsAndProviderLinksKeepNoPart() {
        val storage = CrashRedaction.redact(
            "open /storage/emulated/0/My Books/Health Records/medical_wikipedia.zim failed")
        assertEquals("open [path]", storage)
        val cause = CrashRedaction.redact(
            "/storage/emulated/0/My Books/Health Records/medical_wikipedia.zim: open failed: EACCES (Permission denied)")
        assertEquals("[path]: open failed: EACCES (Permission denied)", cause)
        for (leaked in listOf("Books", "Health", "medical_wikipedia"))
            for (shown in listOf(storage, cause)) assertFalse("$leaked in $shown", shown.contains(leaked))
        assertEquals("[url]: denied", CrashRedaction.redact("FILE:///sdcard/My Books/Private Notes.txt: denied"))
        assertEquals("read \"[url]\" failed", CrashRedaction.redact("read \"file:///sdcard/My Books/a.zim\" failed"))
        assertEquals("[url]",
            CrashRedaction.redact("content://com.android.externalstorage.documents/tree/primary:My Books was revoked"))
        // A secret whose value is a path with spaces goes whole, key first.
        assertEquals("password=[redacted]", CrashRedaction.redact("password=/storage/emulated/0/My Books/a.kdbx"))
        assertEquals("open [path]: \"[redacted]\"", CrashRedaction.redact("open /sdcard/x password: \"hunter2\""))
    }

    @Test
    fun stackFramesKeepTheirShape() {
        for (frame in listOf(
            "org.kiwix.mobile.OwnedDocuments.copy(OwnedDocuments.kt:259)",
            "java.io.FileOutputStream.write(FileOutputStream.java:381)",
            "android.os.Handler.dispatchMessage(Handler.java:106)",
            // A long dotted name with a digit is not an opaque secret in a frame.
            "androidx.appcompat.app.AppCompatDelegateImpl2.updateResourcesConfiguration(AppCompatDelegateImpl2.java:99)",
            "org.kiwix.mobile.TokenStore.getToken(TokenStore.kt:42)",
            "org.kiwix.mobile.Auth.pass(Auth.kt:7)",
            "kotlinx.coroutines.internal.LockFreeLinkedListNode2.helpRemovePrevious8",
        )) assertEquals(frame, CrashRedaction.redactLine(frame))
        // The same steps as a message otherwise: secrets, URLs and paths go.
        assertEquals("3: read [path]: \"[redacted]\"",
            CrashRedaction.redactLine("3: read /sdcard/My Books/a.zim password: \"hunter2\""))
        // Spacing inside a frame is kept.
        assertEquals("at  a.b.C.run(C.kt:1)", CrashRedaction.redactLine("at  a.b.C.run(C.kt:1)"))
    }

    @Test
    fun onlyAPublicHttpHostIsKeptAndNeverItsPath() {
        fun shown(url: String) = CrashRedaction.redact(url)
        assertEquals("https://example.org", shown("https://user:pass@example.org/a/b?q=1#f"))
        assertEquals("http://example.org", shown("HTTP://Example.ORG."))
        assertEquals("https://example.org", shown("https://example.org:0443/x"))
        assertEquals("https://example.org:8443", shown("https://example.org:8443/Private Page"))
        // This device is a reader page.
        for (local in listOf(
            "http://localhost/x", "http://LOCALHOST.:9/x", "http://book.localhost/A/Private Page",
            "http://127.0.0.1:1234/x", "http://127.1/x", "http://[::1]:9/x", "http://localhost:port/x",
        )) assertEquals(local, "[reader-url]", shown(local))
        // Everything else goes whole: other schemes, IP literals in any spelling,
        // single-label and home-network names, a malformed port.
        for (dropped in listOf(
            "http://93.184.216.34/a", "http://[2001:db8::1]/a", "http://0x7f.0.0.1/x", "http://2130706433/x",
            "http://[0:0:0:0:0:0:0:1]:9/x", "http://[::ffff:127.0.0.1]/x", "http://intranet/a b",
            "http://janes-phone.local:8080/A/Private", "http://b\u00fccher.example/a", "http://example.org:port/a",
            "http://example.org:999999/a", "http://[::1/a", "http:///", "ws://example.org/a",
            "http://192.168.1.5:43123/.kiwix-access/0123456789abcdef",
        )) assertEquals(dropped, "[url]", shown(dropped))
        // Case folding is ASCII only: a Kelvin sign is not `k`, a long s is not `s`.
        assertEquals("[url]", shown("http://\u212Aiwix.example/a"))
        assertEquals("pas\u017Fword=hunter2", shown("pas\u017Fword=hunter2"))
    }
}
