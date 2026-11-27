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

  func testReadAloudStartCarriesLocalizedControlNamesAndEarlierShapesStillDecode() throws {
    // The label object Rust builds for a speech session, including the keys
    // this surface does not use.
    let start = try JSONSerialization.data(withJSONObject: [
      "state": "start", "sessionId": "session", "title": "Artikel", "voice": "Heart",
      "status": "reading", "statusLabel": "Vorlesen", "offset": 0, "total": 10,
      "labels": ["title": "Vorlesen", "pause": "Pause", "paused": "Pausiert", "resume": "Fortsetzen",
                 "stop": "Stopp", "previous": "Vorheriger Absatz", "rewind": "Zurück", "forward": "Vor"],
    ] as [String: Any])
    let args = try JSONDecoder().decode(ReadAloudActivityArgs.self, from: start)
    XCTAssertEqual(args.labels?.previous, "Vorheriger Absatz")
    XCTAssertEqual(args.labels?.pause, "Pause")
    XCTAssertEqual(args.labels?.resume, "Fortsetzen")
    XCTAssertEqual(args.labels?.stop, "Stopp")

    // Progress updates never repeat them.
    let update = try JSONSerialization.data(withJSONObject: [
      "state": "active", "sessionId": "session", "status": "reading", "offset": 4, "total": 10,
    ] as [String: Any])
    XCTAssertNil(try JSONDecoder().decode(ReadAloudActivityArgs.self, from: update).labels)

    // An activity that an earlier build started carries no control names; the
    // widget must still decode it and falls back to its English names.
    let earlier = try JSONSerialization.data(withJSONObject: [
      "sessionId": "session", "title": "Artikel", "voice": "Heart",
    ])
    let decoded = try JSONDecoder().decode(ReadAloudActivityAttributes.self, from: earlier)
    XCTAssertEqual(decoded.title, "Artikel")
    XCTAssertNil(decoded.previousLabel)
    XCTAssertNil(decoded.stopLabel)

    let current = ReadAloudActivityAttributes(sessionId: "session", title: "Artikel", voice: "Heart",
      previousLabel: args.labels?.previous, pauseLabel: args.labels?.pause,
      resumeLabel: args.labels?.resume, stopLabel: args.labels?.stop)
    let restored = try JSONDecoder().decode(ReadAloudActivityAttributes.self,
      from: JSONEncoder().encode(current))
    XCTAssertEqual(restored.previousLabel, "Vorheriger Absatz")
    XCTAssertEqual(restored.stopLabel, "Stopp")
  }
}
