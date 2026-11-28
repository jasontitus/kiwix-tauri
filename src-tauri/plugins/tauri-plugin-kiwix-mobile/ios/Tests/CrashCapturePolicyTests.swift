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
    XCTAssertEqual(text, """
      [reader-url] [reader-url] [reader-url] [reader-url] path=[path] relative=[path] \
      [url] [url] [url] [url] [url] https://example.org/read
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
      // Other URLs still end at whitespace.
      ("see https://example.org/read me", "see https://example.org/read me"),
      ("content://provider/My Books and more", "[url] Books and more"),
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
    // A path far longer than the bound is still one placeholder; a pattern
    // engine gave up on such a line and let it through.
    let long = "/" + String(repeating: "a", count: bound * 4)
    XCTAssertEqual(CrashRedaction.redact(long), "[path]")
    XCTAssertEqual(CrashRedaction.redactLine(long), "[path]")
    XCTAssertEqual(CrashRedaction.redact("/" + String(repeating: ":", count: 1_000_000)), "[path]")
    XCTAssertEqual(CrashRedaction.redact("open file:/" + String(repeating: "x y", count: 400_000)), "open [url]")
    // Text past the bound is dropped, not passed through.
    let late = String(repeating: "word ", count: 300_000) + "/Users/Jane Doe/a.zim"
    XCTAssertFalse(CrashRedaction.redact(late).contains("Jane"))
    XCTAssertFalse(CrashRedaction.redactLine(late).contains("Jane"))

    // Characters are Unicode scalars: not bytes, UTF-16 units or grapheme
    // clusters, so a cut never splits a surrogate pair.
    XCTAssertEqual(CrashRedaction.redactLine(String(repeating: "\u{E9}", count: bound + 10)).unicodeScalars.count, bound)
    let astral = CrashRedaction.redactLine(String(repeating: "\u{1F600}", count: bound + 10))
    XCTAssertEqual(astral.unicodeScalars.count, bound)
    XCTAssertEqual(astral.utf16.count, bound * 2)
    XCTAssertEqual(CrashRedaction.redactLine(String(repeating: "e\u{301}", count: bound)).unicodeScalars.count, bound)
    // "\r\n" is one `Character` but two scalars, each of them whitespace.
    XCTAssertEqual(CrashRedaction.redactLine(String(repeating: "\r\n", count: bound)).unicodeScalars.count, bound)
    XCTAssertEqual(CrashRedaction.redact("open /Users/Jane Doe/a.zim\r\nnext: /b c\r\nend"), "open [path] next: [path] end")

    // The message cut is 1,999 scalars and an ellipsis; a frame is cut the same way.
    let cut = CrashRedaction.redact(String(repeating: "\u{1F600} ", count: 3_000))
    XCTAssertEqual(cut.unicodeScalars.count, 2_000)
    XCTAssertEqual(cut.unicodeScalars.last, "\u{2026}")
    let short = String(repeating: "ab ", count: 666) + "x"
    XCTAssertEqual(CrashRedaction.redact(short), short)
    XCTAssertEqual(CrashRedaction.truncate(String(repeating: "\u{1F600}", count: 300), limit: 240).unicodeScalars.count, 240)
    XCTAssertEqual(CrashRedaction.truncate("short", limit: 240), "short")
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
