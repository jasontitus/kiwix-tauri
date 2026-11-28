import Foundation
import XCTest
@testable import KiwixMobile

final class CrashCapturePolicyTests: XCTestCase {
  func testReportTextRedactsReaderLocationsAndNonHTTPURLs() {
    let text = CrashIntake.redact("""
      HTTP://book-uuid-nonce.localhost.:9/A/Pri)vate_Page
      http://127.0.0.2:9/A/Private http://[::1]:9/A/Private
      https://example.org/book-uuid/%2Ekiwix/token/A/Private
      path=/book-uuid/A/Private
      relative=/.kiwix/token/A/Private
      ZIM://book-uuid/A/Private MAILTO:private@example.org
      content://provider/Private file:///Documents/Private%20Notes.txt
      custom://host/Private https://user:pass@example.org/read?q=private#private
      """)
    // A placeholder URL takes the rest of its line with it, and a kept URL
    // keeps only its scheme and host: a path can name the article being read.
    XCTAssertEqual(text, """
      [reader-url] [reader-url] https://example.org path=[path] relative=[path] \
      [url] [url] [url]
      """)
  }

  func testReportTextKeepsNoPartOfAPathOrFileURLThatContainsSpaces() {
    // Stopping at the first space kept the rest of the container or library
    // path. A path now ends only where it cannot continue.
    let cases: [(String, String)] = [
      ("open /var/mobile/Containers/Data/Application/0A1B/Documents/My Books/medical wikipedia.zim failed",
       "open [path]"),
      ("Cannot parse /private/var/mobile/Jane Doe/My Library/state.json: expected value at line 1 column 1",
       "Cannot parse [path]: expected value at line 1 column 1"),
      (#"Cannot parse C:\Users\Jane Doe\My Library\state.json: expected value"#,
       "Cannot parse [path]: expected value"),
      // A quoted path ends at its closing quote, so the cause survives; an
      // escaped quote inside the name does not end it.
      (#"cannot read "/var/mobile/jane doe/My Books/a.zim" (os error 13)"#,
       #"cannot read "[path]" (os error 13)"#),
      (#"cannot read "/var/mobile/My \"Best\" Books/a.zim" (os error 13)"#,
       #"cannot read "[path]" (os error 13)"#),
      (#"cannot read "C:\\Users\\Jane Doe\\" (os error 5)"#,
       #"cannot read "[path]" (os error 5)"#),
      // An apostrophe is part of a name, not the end of the path.
      ("missing '/Volumes/Jane's Backup/Private Notes/a.zim' on disk", "missing '[path]"),
      ("at ~/My Documents/Private Notes.txt now", "at [path]"),
      ("see /var/mobile/My Books/a.zim<end> and `/var/mobile/Other Books/b.zim` next",
       "see [path]<end> and `[path]` next"),
      ("open /var/mobile/My Books/a.zim\nthen retry", "open [path] then retry"),
      // A file URL is a path and may carry unencoded spaces.
      ("open file:///var/mobile/Jane Doe/Private Notes.txt now", "open [url]"),
      ("file:///var/mobile/My Books/a.zim: No such file or directory", "[url]: No such file or directory"),
      (#"opening "file:///var/mobile/My Books/a.zim" did not work"#, #"opening "[url]" did not work"#),
      ("FILE:/var/mobile/My Books/a.zim\nthen retry", "[url] then retry"),
      // A URL that is not kept, or that had a path, goes whole like a path.
      ("see https://example.org/read me", "see https://example.org"),
      ("content://provider/My Books and more", "[url]"),
      ("see https://example.org for help", "see https://example.org for help"),
      // Two paths on one line: the first takes the rest of the line.
      ("path=/book-uuid/A/Private relative=/.kiwix/token/A/Private", "path=[path]"),
      // A path may start right after any punctuation, not only a listed one.
      ("at </var/mobile/Containers/Shared/File Provider Storage/My Books/a.zim> again", "at <[path]> again"),
      ("\u{201C}/var/mobile/My Books/a.zim\u{201D} is missing", "\u{201C}[path]"),
      // A bare address goes, and what follows it is then a path.
      ("mail jane.doe@example.org/Inbox Folder failed", "mail [email][path]"),
      ("pkg@1.2.3 and user@host stay", "pkg@1.2.3 and user@host stay"),
      // Secrets go first, so a path cannot swallow the key that marks one.
      (#"open /var/x password: "hunter2""#, #"open [path]: "[redacted]""#),
      ("Authorization: Bearer abc.def.ghi next", "Authorization: [redacted] next"),
    ]
    for (text, expected) in cases {
      let redacted = CrashIntake.redact(text)
      XCTAssertEqual(redacted, expected)
      for leaked in ["Doe", "doe", "Library", "Books/", "Backup", "Notes", "wikipedia", "Documents", "state.json", "a.zim"] {
        XCTAssertFalse(redacted.contains(leaked), "\(leaked) survived in: \(redacted)")
      }
    }
  }

  /// The vectors shared with `crash.rs` and the Android intake. Each
  /// `expected` is the exact output of `redact` for `input`.
  func testRedactionMatchesTheCanonicalCasesSharedWithRust() throws {
    struct Case: Decodable {
      let input: String
      let expected: String
    }
    // The fixture lives with the Rust tests, outside this package, so it is
    // read from the checkout the tests were built from.
    var fixture = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { fixture.deleteLastPathComponent() }
    fixture.appendPathComponent("tests/fixtures/crash-redaction-cases.json")
    guard let data = try? Data(contentsOf: fixture) else {
      throw XCTSkip("The shared fixture is not readable from here: \(fixture.path)")
    }
    let cases = try JSONDecoder().decode([Case].self, from: data)
    XCTAssertGreaterThanOrEqual(cases.count, 80, "The shared fixture lost cases")
    for item in cases {
      let actual = CrashIntake.redact(item.input)
      // Compared as scalars: `String ==` treats canonically equivalent text
      // as equal, and the ports must agree exactly.
      XCTAssertEqual(Array(actual.unicodeScalars), Array(item.expected.unicodeScalars),
        "input: \(item.input.debugDescription) expected: \(item.expected.debugDescription) actual: \(actual.debugDescription)")
    }
  }

  func testRedactionInputIsBoundedBeforeAnyStepRunsAndCountsScalars() {
    let bound = CrashRedaction.maxInput
    // One unbroken run longer than the bound leaves nothing: the cut fell
    // inside it, and half a path is not judged as if it were whole. A pattern
    // engine gave up on such a line and let it through.
    let long = "/" + String(repeating: "a", count: bound * 4)
    XCTAssertEqual(CrashRedaction.redact(long), "")
    XCTAssertEqual(CrashRedaction.redactLine(long), "")
    XCTAssertEqual(CrashRedaction.redact("/" + String(repeating: ":", count: 1_000_000)), "")
    XCTAssertEqual(CrashRedaction.redact("open file:/" + String(repeating: "x y", count: 400_000)), "open [url]")
    let spaced = "open /Users/Jane Doe/" + String(repeating: "My Books/", count: 200_000)
    XCTAssertEqual(CrashRedaction.redact(spaced), "open [path]")
    XCTAssertEqual(CrashRedaction.redactLine(spaced), "open [path] ")
    // Text past the bound is dropped, not passed through.
    let late = String(repeating: "word ", count: 300_000) + "/Users/Jane Doe/a.zim"
    XCTAssertFalse(CrashRedaction.redact(late).contains("Jane"))
    XCTAssertFalse(CrashRedaction.redactLine(late).contains("Jane"))

    // The cut falls inside a token: the partial token goes. Kept, the first
    // half of this reader URL's host would look like a public name and stay.
    let prefix = String(repeating: " ", count: bound - 19)
    let straddling = prefix + "http://12b34567.localhost:9/A/Private_Page"
    XCTAssertEqual(CrashRedaction.redactLine(straddling), prefix)
    XCTAssertEqual(CrashRedaction.redact(straddling), "")
    XCTAssertEqual(CrashRedaction.redact("http://12b34567.localhost:9/A/Private_Page"), "[reader-url]")
    let cutPath = CrashRedaction.redact(String(repeating: "x ", count: bound / 2 - 8) + "/home/jane doe/Private Notes/a.zim")
    XCTAssertFalse(cutPath.contains("jane") || cutPath.contains("Private"))
    // The cut falls between words, on either side of the space: every whole
    // word before it stays, and a field exactly at the bound is untouched.
    XCTAssertEqual(CrashRedaction.redactLine(String(repeating: "a", count: bound - 1) + " next").unicodeScalars.count, bound)
    XCTAssertEqual(CrashRedaction.redactLine(String(repeating: "a", count: bound) + " next").unicodeScalars.count, bound)
    XCTAssertEqual(CrashRedaction.redactLine(String(repeating: "a", count: bound)).unicodeScalars.count, bound)

    // Every count is in Unicode scalars: not bytes, UTF-16 units or
    // `Character`s, so a cut never splits a surrogate pair.
    XCTAssertEqual(CrashRedaction.redactLine(String(repeating: "\u{E9} ", count: bound)).unicodeScalars.count, bound)
    let astral = CrashRedaction.redactLine(String(repeating: "\u{1F600} ", count: bound))
    XCTAssertEqual(astral.unicodeScalars.count, bound)
    XCTAssertEqual(astral.utf16.count, bound + bound / 2)
    // "\r\n" is one `Character` but two scalars, each of them whitespace.
    XCTAssertEqual(CrashRedaction.redactLine(String(repeating: "\r\n", count: bound)).unicodeScalars.count, bound)
    XCTAssertEqual(CrashRedaction.redact("open /Users/Jane Doe/a.zim\r\nnext: /b c\r\nend"), "open [path] next: [path] end")
    // "e" and U+0301 are one `Character` too. A frame cut may fall between
    // them; the result is still valid text.
    let combining = CrashRedaction.truncate(String(repeating: "e\u{301}", count: 300), limit: 240)
    XCTAssertEqual(combining.unicodeScalars.count, 240)
    XCTAssertEqual(String(combining.utf16), combining)

    // Whitespace is the Unicode White_Space list, and case is ASCII only.
    XCTAssertEqual(CrashRedaction.redact("x\u{85}/home/jane doe/a.zim"), "x [path]")
    XCTAssertEqual(CrashRedaction.redact("a\u{1F}b\u{200B}c"), "a\u{1F}b\u{200B}c")
    XCTAssertEqual(CrashRedaction.redact("pas\u{17F}word=hunter2 coo\u{212A}ie=x"), "pas\u{17F}word=hunter2 coo\u{212A}ie=x")

    // The message cut is 1,999 scalars and an ellipsis; a frame is cut the same way.
    let cut = CrashRedaction.redact(String(repeating: "\u{1F600} ", count: 3_000))
    XCTAssertEqual(cut.unicodeScalars.count, 2_000)
    XCTAssertEqual(cut.unicodeScalars.last, "\u{2026}")
    let short = String(repeating: "ab ", count: 666) + "x"
    XCTAssertEqual(CrashRedaction.redact(short), short)
    XCTAssertEqual(CrashRedaction.truncate(String(repeating: "\u{1F600}", count: 300), limit: 240).unicodeScalars.count, 240)
    XCTAssertEqual(CrashRedaction.truncate("short", limit: 240), "short")
  }

  /// The rule requires every step to take time proportional to its input.
  /// Each unit below starts a URL, path or secret candidate; rescanning the
  /// rest of the text once per candidate made such inputs quadratic.
  func testEveryRedactionStepIsLinearInItsInput() {
    func fastest(_ text: String) -> TimeInterval {
      var best = TimeInterval.infinity
      for _ in 0..<3 {
        let started = Date()
        _ = CrashRedaction.redact(text)
        _ = CrashRedaction.redactLine(text)
        best = min(best, Date().timeIntervalSince(started))
      }
      return best
    }
    let units = [
      "a://b<", "file:x<", "zz://q`", "password=\"\" ", "data:x\n", "/a: ", " \\a", "a:", "'password'",
      "token='x ", "C:\\a\" ", "!\\a", "\\a\"", "pin=Some(", "auth=\\\"x", "https://a.b:1 ", "http://e.org/../",
      "a@", "@a.", "x@y.zz ", "@.-",
    ]
    for unit in units {
      // Whole units only, so the bound does not drop a partial one.
      let length = unit.unicodeScalars.count
      let quarter = fastest(String(repeating: unit, count: CrashRedaction.maxInput / 4 / length))
      let full = fastest(String(repeating: unit, count: CrashRedaction.maxInput / length))
      // Four times the input takes about four times as long when linear and
      // sixteen times when quadratic. The floor absorbs timer noise.
      XCTAssertLessThanOrEqual(full, max(quarter, 0.005) * 10,
        "\(unit.debugDescription): \(quarter) s for a quarter of the bound, \(full) s for all of it")
      // Generous for an unoptimized build on a busy machine.
      XCTAssertLessThan(full, 2, unit.debugDescription)
    }
  }

  /// A frame keeps its spacing and its symbol: steps 1 to 5 only.
  func testStackFrameLineKeepsSpacingAndLongSymbols() {
    let frame = "3   Kiwix                               0x0000000104a3c1f4 $s5Kiwix11ReaderModelC4open3url10completionySS_yycSgtF + 123"
    XCTAssertEqual(CrashRedaction.redactLine(frame), frame)
    XCTAssertEqual(CrashRedaction.redactLine("4   Kiwix   0x01 open /Users/Jane Doe/a.zim password: \"hunter2\""),
      "4   Kiwix   0x01 open [path]: \"[redacted]\"")
  }

  func testReadOnlyCutoverDrainsWritesAndRejectsLaterCapture() {
    CrashIntake.setEnabled(true)
    defer { CrashIntake.setEnabled(false) }
    let entered = DispatchSemaphore(value: 0)
    let release = DispatchSemaphore(value: 0)
    let attemptedDisable = DispatchSemaphore(value: 0)
    let disabled = DispatchSemaphore(value: 0)
    let writerFinished = DispatchSemaphore(value: 0)
    DispatchQueue.global().async {
      CrashIntake.withCapturePermission {
        entered.signal()
        _ = release.wait(timeout: .now() + 5)
      }
      writerFinished.signal()
    }
    XCTAssertEqual(entered.wait(timeout: .now() + 5), .success)
    DispatchQueue.global().async {
      attemptedDisable.signal()
      CrashIntake.setEnabled(false)
      disabled.signal()
    }
    XCTAssertEqual(attemptedDisable.wait(timeout: .now() + 5), .success)
    XCTAssertEqual(disabled.wait(timeout: .now()), .timedOut,
      "Policy cannot acknowledge read-only while an authorized write is still running")
    release.signal()
    XCTAssertEqual(disabled.wait(timeout: .now() + 5), .success)
    XCTAssertEqual(writerFinished.wait(timeout: .now() + 5), .success)
    var wrote = false
    CrashIntake.withCapturePermission { wrote = true }
    XCTAssertFalse(wrote, "Every write boundary must recheck effective runtime policy")
  }
}
