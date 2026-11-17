import ActivityKit
import XCTest
@testable import KiwixMobile

@available(iOS 16.2, *)
final class LiveActivityReconciliationTests: XCTestCase {
  func testStaleMatchingActivityIsReusedRatherThanDuplicated() {
    XCTAssertEqual(DownloadLiveActivityBridge.reconciliation(.stale, sameJob: true, hasCurrent: false), .update)
    XCTAssertEqual(DownloadLiveActivityBridge.reconciliation(.stale, sameJob: true, hasCurrent: true), .retire)
    XCTAssertEqual(DownloadLiveActivityBridge.reconciliation(.stale, sameJob: false, hasCurrent: false), .retire)
  }

  func testStillVisibleEndedActivityIsRetiredEvenWhenItMatches() {
    XCTAssertEqual(DownloadLiveActivityBridge.reconciliation(.ended, sameJob: true, hasCurrent: false), .retire)
    XCTAssertEqual(DownloadLiveActivityBridge.reconciliation(.ended, sameJob: false, hasCurrent: true), .retire)
    XCTAssertEqual(DownloadLiveActivityBridge.reconciliation(.dismissed, sameJob: true, hasCurrent: false), .ignore)
  }
}
