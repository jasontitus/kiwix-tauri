import Foundation
import XCTest
@testable import KiwixMobile

final class CrashCapturePolicyTests: XCTestCase {
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
