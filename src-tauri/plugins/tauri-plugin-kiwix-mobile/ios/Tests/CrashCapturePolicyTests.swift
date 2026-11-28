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
