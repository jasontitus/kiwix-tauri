import Foundation
import UIKit
import XCTest
@testable import KiwixMobile

private final class IdleTimerLeaseHolder {
  var lease: MobileIdleTimerLease?
  init(_ lease: MobileIdleTimerLease) { self.lease = lease }
}

final class MobileIdleTimerTests: XCTestCase {
  @MainActor
  func testOverlappingActivitiesRestoreBaselineOnlyAfterLastRelease() async {
    let original = UIApplication.shared.isIdleTimerDisabled
    UIApplication.shared.isIdleTimerDisabled = false
    defer { UIApplication.shared.isIdleTimerDisabled = original }
    let publisher = MobileIdleTimerLease()
    let discovery = MobileIdleTimerLease()
    let join = MobileIdleTimerLease()
    let transfer = MobileIdleTimerLease()
    defer { publisher.release(); discovery.release(); join.release(); transfer.release() }

    XCTAssertTrue(UIApplication.shared.isIdleTimerDisabled)
    discovery.release()
    XCTAssertTrue(UIApplication.shared.isIdleTimerDisabled)
    publisher.release()
    XCTAssertTrue(UIApplication.shared.isIdleTimerDisabled)
    join.release()
    XCTAssertTrue(UIApplication.shared.isIdleTimerDisabled)
    discovery.release()
    XCTAssertTrue(UIApplication.shared.isIdleTimerDisabled, "Duplicate release cannot release the transfer owner")
    transfer.release()
    XCTAssertFalse(UIApplication.shared.isIdleTimerDisabled)
  }

  @MainActor
  func testLastOwnerPreservesPreviouslyDisabledIdleTimer() async {
    let original = UIApplication.shared.isIdleTimerDisabled
    UIApplication.shared.isIdleTimerDisabled = true
    defer { UIApplication.shared.isIdleTimerDisabled = original }
    let first = MobileIdleTimerLease()
    let last = MobileIdleTimerLease()
    defer { first.release(); last.release() }

    first.release()
    XCTAssertTrue(UIApplication.shared.isIdleTimerDisabled)
    last.release()
    XCTAssertTrue(UIApplication.shared.isIdleTimerDisabled)
  }

  @MainActor
  func testReleasedOwnerCannotReleaseLaterAssertion() async {
    let original = UIApplication.shared.isIdleTimerDisabled
    UIApplication.shared.isIdleTimerDisabled = false
    defer { UIApplication.shared.isIdleTimerDisabled = original }
    var retired: MobileIdleTimerLease? = MobileIdleTimerLease()
    retired?.release()
    XCTAssertFalse(UIApplication.shared.isIdleTimerDisabled)
    let current = MobileIdleTimerLease()
    defer { current.release() }

    retired?.release()
    retired = nil
    XCTAssertTrue(UIApplication.shared.isIdleTimerDisabled)
    current.release()
    XCTAssertFalse(UIApplication.shared.isIdleTimerDisabled)

    UIApplication.shared.isIdleTimerDisabled = true
    let next = MobileIdleTimerLease()
    next.release()
    XCTAssertTrue(UIApplication.shared.isIdleTimerDisabled, "Each assertion group captures its own baseline")
  }

  @MainActor
  func testOffMainDeinitReleasesOnlyItsOwnerOnMain() async {
    let original = UIApplication.shared.isIdleTimerDisabled
    UIApplication.shared.isIdleTimerDisabled = false
    defer { UIApplication.shared.isIdleTimerDisabled = original }
    let first = IdleTimerLeaseHolder(MobileIdleTimerLease())
    let last = IdleTimerLeaseHolder(MobileIdleTimerLease())
    defer { first.lease?.release(); last.lease?.release() }

    await discardOffMain(first)
    XCTAssertTrue(UIApplication.shared.isIdleTimerDisabled)
    await discardOffMain(last)
    XCTAssertFalse(UIApplication.shared.isIdleTimerDisabled)
  }

  @MainActor
  private func discardOffMain(_ holder: IdleTimerLeaseHolder) async {
    await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
      // Access moves to this block until its main-queue barrier has completed.
      DispatchQueue.global().async {
        holder.lease = nil
        // Lease deinit enqueues its release before this barrier, so assertions
        // cannot race the UIKit write or the holder's background mutation.
        DispatchQueue.main.async { continuation.resume() }
      }
    }
  }
}
