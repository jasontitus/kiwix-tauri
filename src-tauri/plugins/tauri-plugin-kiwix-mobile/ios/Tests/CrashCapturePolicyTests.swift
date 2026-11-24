import Foundation
import XCTest
@testable import KiwixMobile

final class CrashCapturePolicyTests: XCTestCase {
  func testReportTextRedactsReaderLocationsAndNonHTTPURLs() {
    let text = CrashIntake.redact("""
      HTTP://book-uuid-nonce.localhost.:9/A/Pri)vate_Page
      http://127.0.0.2:9/A/Private http://[::1]:9/A/Private
      https://example.org/book-uuid/%2Ekiwix/token/A/Private
      path=/book-uuid/A/Private relative=/.kiwix/token/A/Private
      ZIM://book-uuid/A/Private MAILTO:private@example.org
      content://provider/Private file:///Documents/Private%20Notes.txt
      custom://host/Private https://user:pass@example.org/read?q=private#private
      """)
    XCTAssertEqual(text, """
      [reader-url] [reader-url] [reader-url] [reader-url] path=[path] relative=[path] \
      [url] [url] [url] [url] [url] https://example.org/read
      """)
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
