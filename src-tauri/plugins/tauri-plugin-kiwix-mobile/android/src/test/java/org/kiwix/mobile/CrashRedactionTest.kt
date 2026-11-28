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
    fun inputIsBoundedInCodePointsBeforeAnyStepRuns() {
        val limit = CrashRedaction.MAX_INPUT
        // Far over the bound: still one path, and the scan stays bounded.
        val long = "/" + "a".repeat(limit * 4)
        assertEquals("[path]", CrashRedaction.redact(long))
        assertEquals("[path]", CrashRedaction.redactLine(long))
        // Characters, not UTF-16 units or bytes.
        val accented = CrashRedaction.redactLine("\u00e9".repeat(limit + 10))
        assertEquals(limit, accented.codePointCount(0, accented.length))
        // A character outside the basic plane is two UTF-16 units and is never split,
        // wherever the bound falls inside the text.
        val emoji = "\uD83D\uDE00"
        assertEquals(emoji.repeat(limit), CrashRedaction.redactLine(emoji.repeat(limit + 5)))
        assertEquals("a" + emoji.repeat(limit - 1), CrashRedaction.redactLine("a" + emoji.repeat(limit + 5)))
        // What lies beyond the bound is never read, so it cannot be kept either.
        val beyond = CrashRedaction.redactLine("x".repeat(limit) + " /storage/emulated/0/My Books/a.zim")
        assertEquals("x".repeat(limit), beyond)
        // A short field is untouched.
        assertEquals("File.kt:12", CrashRedaction.redact("File.kt:12"))
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
    fun whitespaceIsTheUnicodeWhiteSpaceProperty() {
        // Java's Character.isWhitespace excludes the no-break spaces and
        // includes the information separators; the pipeline must do neither.
        for (point in listOf(0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x20, 0x85, 0xA0, 0x1680, 0x2000, 0x200A,
            0x2028, 0x2029, 0x202F, 0x205F, 0x3000)) assertTrue(Integer.toHexString(point), CrashRedaction.isWhiteSpace(point))
        for (point in listOf(0x1C, 0x1D, 0x1E, 0x1F, 0x180E, 0x200B, 0x2060, 0xFEFF, 'a'.code))
            assertFalse(Integer.toHexString(point), CrashRedaction.isWhiteSpace(point))
        // A path after a narrow no-break space, and collapsed around one.
        assertEquals("open [path]", CrashRedaction.redact("open\u202F/storage/emulated/0/My Books/a.zim"))
        assertEquals("a b", CrashRedaction.redact("a\u00A0\u2003 b"))
        // An information separator is not whitespace: it neither starts a path nor is collapsed.
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
            "kotlinx.coroutines.internal.LockFreeLinkedListNode2.helpRemovePrevious8",
        )) assertEquals(frame, CrashRedaction.redactLine(frame))
        // The same steps as a message otherwise: secrets, URLs and paths go.
        assertEquals("3: read [path]: \"[redacted]\"",
            CrashRedaction.redactLine("3: read /sdcard/My Books/a.zim password: \"hunter2\""))
        // Spacing inside a frame is kept.
        assertEquals("at  a.b.C.run(C.kt:1)", CrashRedaction.redactLine("at  a.b.C.run(C.kt:1)"))
    }

    @Test
    fun onlyAnExternalHttpAddressKeepsItsHostAndPath() {
        fun shown(url: String) = CrashRedaction.redact(url)
        assertEquals("https://example.org/a/b", shown("https://user:pass@example.org/a/b?q=1#f"))
        assertEquals("http://example.org/", shown("HTTP://Example.ORG"))
        assertEquals("https://example.org/", shown("https://example.org:443"))
        assertEquals("https://example.org:8443/x", shown("https://example.org:8443/x"))
        assertEquals("http://93.184.216.34/a", shown("http://93.184.216.34/a"))
        assertEquals("http://[2001:db8::1]:8080/a", shown("http://[2001:DB8:0:0:0:0:0:1]:8080/a"))
        assertEquals("https://example.org/b", shown("https://example.org/a/../b"))
        assertEquals("https://example.org/a%7Bb%7D", shown("https://example.org/a{b}"))
        // This device, in every spelling, is a reader page.
        for (local in listOf(
            "http://localhost/x", "http://LOCALHOST.:9/x", "http://book.localhost/x", "http://127.0.0.1/x",
            "http://127.1/x", "http://0x7f.0.0.1/x", "http://[::1]/x", "http://[0:0:0:0:0:0:0:1]:9/x",
            "http://[::ffff:127.0.0.1]/x",
        )) assertEquals(local, "[reader-url]", shown(local))
        // An address this parser does not fully understand is dropped whole.
        for (unclear in listOf(
            "http://exa%6dple.org/a", "http://b\u00fccher.example/a", "http://example.org:port/a",
            "http://example.org:99999/a", "http://[::1/a", "http://[zz::1]/a", "http:///", "http://1.2.3.4.5/a",
        )) assertEquals(unclear, "[url]", shown(unclear))
    }
}
