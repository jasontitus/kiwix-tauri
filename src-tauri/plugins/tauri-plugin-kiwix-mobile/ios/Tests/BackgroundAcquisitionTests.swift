import Darwin
import Foundation
import XCTest
import UIKit
@testable import KiwixMobile

private final class ForwardingBackgroundDelegate: NSObject, UIApplicationDelegate {
  var forwarded: [String] = []
  func application(_ application: UIApplication, handleEventsForBackgroundURLSession identifier: String,
                   completionHandler: @escaping () -> Void) {
    forwarded.append(identifier)
    completionHandler()
  }
}

private final class EmptyBackgroundDelegate: NSObject, UIApplicationDelegate {}

private final class BackgroundAdmissionSession: URLSession, @unchecked Sendable {
  private let backing: URLSession
  private(set) var createdDownloads = 0

  init(recording configuration: URLSessionConfiguration) {
    backing = URLSession(configuration: configuration)
    super.init()
  }

  override func downloadTask(with request: URLRequest) -> URLSessionDownloadTask {
    createdDownloads += 1
    return backing.downloadTask(with: request)
  }

  override func downloadTask(withResumeData resumeData: Data) -> URLSessionDownloadTask {
    createdDownloads += 1
    return backing.downloadTask(withResumeData: resumeData)
  }

  override func invalidateAndCancel() {
    backing.invalidateAndCancel()
  }
}

final class BackgroundAcquisitionTests: XCTestCase {
  private func response(_ status: Int, _ headers: [String: String]) throws -> HTTPURLResponse {
    try XCTUnwrap(HTTPURLResponse(url: URL(string: "https://download.kiwix.org/zim/book.zim")!,
      statusCode: status, httpVersion: "HTTP/1.1", headerFields: headers))
  }

  func testResumedResponseValidatesWholeFileRatherThanRemainingBodyLength() throws {
    let resumed = try response(206, ["Content-Range": "bytes 60-99/100", "Content-Length": "40"])
    XCTAssertTrue(BackgroundAcquisition.validCompletedResponse(resumed, size: 100, expectedSize: 100))
    XCTAssertFalse(BackgroundAcquisition.validCompletedResponse(resumed, size: 40, expectedSize: 100))
    XCTAssertFalse(BackgroundAcquisition.validCompletedResponse(resumed, size: 100, expectedSize: 101))
    let truncated = try response(206, ["Content-Range": "bytes 60-98/100", "Content-Length": "39"])
    XCTAssertFalse(BackgroundAcquisition.validCompletedResponse(truncated, size: 100, expectedSize: 100))
    let wrongLength = try response(206, ["Content-Range": "bytes 60-99/100", "Content-Length": "41"])
    XCTAssertFalse(BackgroundAcquisition.validCompletedResponse(wrongLength, size: 100, expectedSize: 100))
  }

  func testOnlySuccessfulIdentityEncodedWholeResponsesBecomeReceipts() throws {
    XCTAssertTrue(BackgroundAcquisition.validCompletedResponse(try response(200, ["Content-Length": "100"]), size: 100, expectedSize: nil))
    XCTAssertFalse(BackgroundAcquisition.validCompletedResponse(try response(200, ["Content-Length": "99"]), size: 100, expectedSize: nil))
    XCTAssertFalse(BackgroundAcquisition.validCompletedResponse(try response(200, ["Content-Encoding": "gzip"]), size: 100, expectedSize: nil))
    XCTAssertFalse(BackgroundAcquisition.validCompletedResponse(try response(302, ["Content-Length": "100"]), size: 100, expectedSize: nil))
    XCTAssertFalse(BackgroundAcquisition.validCompletedResponse(try response(206, ["Content-Range": "bytes 0-18446744073709551615/100"]), size: 100, expectedSize: 100))
    XCTAssertFalse(BackgroundAcquisition.validCompletedResponse(try response(206, ["Content-Range": "bytes 60-99/*"]), size: 100, expectedSize: 100))
  }

  func testRetiredTaskCannotOverwriteResumedTaskOrExplicitPause() {
    var lease = BackgroundLease(id: UUID().uuidString.lowercased(), generation: UUID().uuidString.lowercased(),
      url: "https://download.kiwix.org/zim/book.zim", allowsCellular: false, state: "running")
    lease.taskIdentifier = 17
    XCTAssertTrue(lease.acceptsCallback(task: 17))
    lease.taskIdentifier = 18
    XCTAssertFalse(lease.acceptsCallback(task: 17))
    XCTAssertTrue(lease.acceptsCallback(task: 18))
    lease.state = "paused"
    XCTAssertFalse(lease.acceptsCallback(task: 18))
    lease.state = "cancelled"
    XCTAssertFalse(lease.acceptsCallback(task: 18))
    lease.state = "staging"
    XCTAssertFalse(lease.acceptsCallback(task: 18))
    lease.state = "complete"
    XCTAssertFalse(lease.acceptsCallback(task: 18))
  }

  func testReceiptRejectsSameSizeReplacementSymlinkAndAdditionalHardlink() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("background-receipt-" + UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: root) }
    let path = root.appendingPathComponent("stage")
    try Data([1, 2, 3, 4]).write(to: path)
    var info = stat()
    XCTAssertEqual(lstat(path.path, &info), 0)
    var lease = BackgroundLease(id: UUID().uuidString.lowercased(), generation: UUID().uuidString.lowercased(),
      url: "https://download.kiwix.org/zim/book.zim", allowsCellular: false, state: "complete")
    lease.copied = 4; lease.device = UInt64(info.st_dev); lease.inode = UInt64(info.st_ino)
    XCTAssertTrue(try lease.validateStage(at: path))
    let retained = root.appendingPathComponent("retained")
    XCTAssertEqual(link(path.path, retained.path), 0)
    XCTAssertThrowsError(try lease.validateStage(at: path))
    XCTAssertEqual(unlink(retained.path), 0)
    XCTAssertTrue(try lease.validateStage(at: path))
    // Keep the original inode alive so a replacement cannot reuse its number.
    try FileManager.default.moveItem(at: path, to: retained)
    try Data([4, 3, 2, 1]).write(to: path)
    XCTAssertThrowsError(try lease.validateStage(at: path))
    try FileManager.default.removeItem(at: path)
    try FileManager.default.createSymbolicLink(at: path, withDestinationURL: retained)
    XCTAssertThrowsError(try lease.validateStage(at: path))
    try FileManager.default.removeItem(at: path)
    XCTAssertFalse(try lease.validateStage(at: path))
  }

  func testPersistedLeaseKeepsNetworkRestrictionAndExactStageIdentity() throws {
    var lease = BackgroundLease(id: UUID().uuidString.lowercased(), generation: UUID().uuidString.lowercased(),
      url: "https://download.kiwix.org/zim/book.zim", allowsCellular: false, state: "paused")
    lease.resumeData = Data([0, 1, 2]); lease.device = 7; lease.inode = 8; lease.taskIdentifier = 9
    let restored = try JSONDecoder().decode(BackgroundLease.self, from: JSONEncoder().encode(lease))
    XCTAssertFalse(restored.allowsCellular)
    XCTAssertEqual(restored.resumeData, lease.resumeData)
    XCTAssertEqual(restored.device, 7)
    XCTAssertEqual(restored.inode, 8)
    XCTAssertEqual(restored.taskIdentifier, 9)
    XCTAssertEqual(restored.key, lease.key)
    XCTAssertFalse(restored.acceptsCallback(task: 9))
  }

  func testRestoredPolicyCannotAuthorizeDelegateWritesBeforeRustBootstrap() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("background-denial-" + UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: root) }
    // Suspended ephemeral tasks exercise the real delegate consumers without
    // issuing a network request or creating an OS-owned background session.
    let session = URLSession(configuration: .ephemeral)
    defer { session.invalidateAndCancel() }
    let task = session.downloadTask(with: URL(string: "https://download.kiwix.org/zim/book.zim")!)
    var lease = BackgroundLease(id: UUID().uuidString.lowercased(), generation: UUID().uuidString.lowercased(),
      url: "https://download.kiwix.org/zim/book.zim", allowsCellular: false, state: "running")
    lease.taskIdentifier = task.taskIdentifier
    task.taskDescription = lease.key
    let temporary = root.appendingPathComponent("system-temporary")
    try Data([1, 2, 3, 4]).write(to: temporary)
    let manifest = root.appendingPathComponent("leases.json")
    // Even a permissive native receipt cannot override the actual Store mode
    // before Rust supplies it. This is the cold-start read-only/private race.
    for restored in [BackgroundPolicy(privateMode: false, readOnly: false),
                     BackgroundPolicy(privateMode: true, readOnly: false),
                     BackgroundPolicy(privateMode: false, readOnly: true),
                     BackgroundPolicy(privateMode: true, readOnly: true)] {
      let before = try JSONEncoder().encode(BackgroundLedger(policy: restored, leases: [lease.key: lease]))
      try before.write(to: manifest)
      let relaunched = BackgroundAcquisition(root: root)
      relaunched.urlSession(session, downloadTask: task, didWriteData: 4, totalBytesWritten: 4, totalBytesExpectedToWrite: 4)
      relaunched.urlSession(session, downloadTask: task, didFinishDownloadingTo: temporary)
      relaunched.urlSession(session, task: task, didCompleteWithError: NSError(domain: NSURLErrorDomain, code: NSURLErrorCancelled))
      XCTAssertEqual(try Data(contentsOf: manifest), before)
      XCTAssertTrue(FileManager.default.fileExists(atPath: temporary.path))
      XCTAssertFalse(FileManager.default.fileExists(atPath: root.appendingPathComponent(lease.key + ".download").path))
    }
  }

  func testPostRenameSyncAndLedgerFaultsPreserveRecoveryAndCancellationAuthority() throws {
    for failurePoint in [2, 3] {
      for cancel in [false, true] {
        let root = FileManager.default.temporaryDirectory
          .appendingPathComponent("background-retention-" + UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: root) }
        // Suspended ephemeral tasks never send requests or create a native
        // background service; the real completion/command consumers run below.
        let session = BackgroundAdmissionSession(recording: .ephemeral)
        defer { session.invalidateAndCancel() }
        let task = session.downloadTask(with: URLRequest(url: URL(string: "https://download.kiwix.org/zim/book.zim")!))
        var lease = BackgroundLease(id: UUID().uuidString.lowercased(), generation: UUID().uuidString.lowercased(),
          url: "https://download.kiwix.org/zim/book.zim", expectedSize: 4, allowsCellular: false, state: "running")
        lease.taskIdentifier = task.taskIdentifier
        task.taskDescription = lease.key
        XCTAssertEqual(session.createdDownloads, 1)
        let manifest = root.appendingPathComponent("leases.json")
        try JSONEncoder().encode(BackgroundLedger(policy: BackgroundPolicy(), leases: [lease.key: lease])).write(to: manifest)
        let temporary = root.appendingPathComponent("system-temporary")
        let bytes = Data([1, 2, 3, 4])
        try bytes.write(to: temporary)
        var original = stat()
        XCTAssertEqual(lstat(temporary.path, &original), 0)
        var syncCalls = 0
        var faultsEnabled = true
        let acquisition = BackgroundAcquisition(root: root, session: session, synchronizeDirectory: { directory in
          syncCalls += 1
          // #1 commits the staging journal, #2 syncs the successful rename,
          // #3 commits the completion ledger. Keep failing subsequent saves.
          if faultsEnabled && syncCalls >= failurePoint {
            throw NSError(domain: NSPOSIXErrorDomain, code: Int(EIO))
          }
          let descriptor = open(directory.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
          guard descriptor >= 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
          defer { close(descriptor) }
          guard fsync(descriptor) == 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
        })
        try acquisition.applyPolicy(["privateMode": false, "readOnly": false]) { XCTAssertNil($0["failure"]) }
        acquisition.finishDownload(task, response: try response(200, ["Content-Length": "4"]), location: temporary)
        let stage = root.appendingPathComponent(lease.key + ".download")
        XCTAssertFalse(FileManager.default.fileExists(atPath: temporary.path))
        XCTAssertEqual(try Data(contentsOf: stage), bytes)
        func receipt() throws -> BackgroundLease {
          try XCTUnwrap(JSONDecoder().decode(BackgroundLedger.self, from: Data(contentsOf: manifest)).leases[lease.key])
        }
        let retained = try receipt()
        XCTAssertEqual(retained.state, "staging")
        XCTAssertEqual(retained.device, UInt64(original.st_dev))
        XCTAssertEqual(retained.inode, UInt64(original.st_ino))
        XCTAssertTrue(try retained.validateStage(at: stage))
        let identity: [String: Any] = ["id": lease.id, "generation": lease.generation]
        var start = identity
        start["url"] = lease.url; start["expectedSize"] = 4; start["allowsCellular"] = false
        // A failed recovery cannot fall through into admitting another task.
        XCTAssertThrowsError(try acquisition.perform("start", args: start, tasks: []) { _ in XCTFail("admitted a new task") })
        // Pause and the URLSession completion-error callback must not downgrade
        // preserved staging (including when its best-effort save also failed).
        try acquisition.perform("stop", args: identity, tasks: []) { XCTAssertNil($0["failure"]) }
        acquisition.urlSession(session, task: task, didCompleteWithError:
          NSError(domain: NSURLErrorDomain, code: NSURLErrorCancelled))
        XCTAssertEqual(try receipt().state, "staging")
        let laterTemporary = root.appendingPathComponent("later-system-temporary")
        try Data([4, 3, 2, 1]).write(to: laterTemporary)
        acquisition.finishDownload(task, response: try response(200, ["Content-Length": "4"]), location: laterTemporary)
        XCTAssertEqual(try receipt().inode, UInt64(original.st_ino))
        XCTAssertTrue(FileManager.default.fileExists(atPath: laterTemporary.path))
        faultsEnabled = false
        if cancel {
          var stop = identity; stop["cancel"] = true
          try acquisition.perform("stop", args: stop, tasks: []) { XCTAssertNil($0["failure"]) }
          XCTAssertFalse(FileManager.default.fileExists(atPath: stage.path))
          XCTAssertTrue(FileManager.default.fileExists(atPath: laterTemporary.path))
          XCTAssertEqual(try receipt().state, "cancelled")
        } else {
          // Cold restore must use the persisted pre-/post-rename identity too.
          let relaunched = BackgroundAcquisition(root: root, session: session)
          try relaunched.applyPolicy(["privateMode": false, "readOnly": false]) { XCTAssertNil($0["failure"]) }
          var snapshot: [String: Any] = [:]
          try relaunched.perform("snapshot", args: identity, tasks: []) { snapshot = $0 }
          XCTAssertEqual(snapshot["state"] as? String, "complete")
          XCTAssertEqual(snapshot["path"] as? String, stage.path)
          XCTAssertEqual(snapshot["stagingDirectory"] as? String, root.path)
          XCTAssertEqual(snapshot["inode"] as? UInt64, UInt64(original.st_ino))
          try relaunched.perform("start", args: start, tasks: []) { XCTAssertNil($0["failure"]) }
          XCTAssertEqual(try receipt().inode, UInt64(original.st_ino))
          XCTAssertEqual(try Data(contentsOf: stage), bytes)
        }
        // A never-resumed ephemeral task need not appear in getAllTasks.
        // Observe both real admission APIs instead of assuming OS enumeration
        // retains our synthetic completion task.
        XCTAssertEqual(session.createdDownloads, 1)
      }
    }
  }

  func testStagingCancellationRejectsReplacementAndCanStillDeleteOwnedInode() throws {
    let root = FileManager.default.temporaryDirectory
      .appendingPathComponent("background-cancel-identity-" + UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: root) }
    var lease = BackgroundLease(id: UUID().uuidString.lowercased(), generation: UUID().uuidString.lowercased(),
      url: "https://download.kiwix.org/zim/book.zim", allowsCellular: false, state: "staging")
    let stage = root.appendingPathComponent(lease.key + ".download")
    let original = root.appendingPathComponent("retained-original")
    try Data([1, 2, 3, 4]).write(to: stage)
    var info = stat()
    XCTAssertEqual(lstat(stage.path, &info), 0)
    lease.copied = 4; lease.device = UInt64(info.st_dev); lease.inode = UInt64(info.st_ino)
    try JSONEncoder().encode(BackgroundLedger(policy: BackgroundPolicy(), leases: [lease.key: lease]))
      .write(to: root.appendingPathComponent("leases.json"))
    try FileManager.default.moveItem(at: stage, to: original)
    let replacement = Data([4, 3, 2, 1])
    try replacement.write(to: stage)
    let session = URLSession(configuration: .ephemeral)
    defer { session.invalidateAndCancel() }
    let acquisition = BackgroundAcquisition(root: root, session: session)
    try acquisition.applyPolicy(["privateMode": false, "readOnly": false]) { XCTAssertNil($0["failure"]) }
    let stop: [String: Any] = ["id": lease.id, "generation": lease.generation, "cancel": true]
    var rejected: [String: Any] = [:]
    try acquisition.perform("stop", args: stop, tasks: []) { rejected = $0 }
    XCTAssertNotNil(rejected["failure"])
    XCTAssertEqual(try Data(contentsOf: stage), replacement)
    XCTAssertTrue(FileManager.default.fileExists(atPath: original.path))
    // The failed cancellation retains the old receipt rather than adopting the
    // replacement. Restoring that exact inode makes explicit cleanup safe.
    try FileManager.default.removeItem(at: stage)
    try FileManager.default.moveItem(at: original, to: stage)
    try acquisition.perform("stop", args: stop, tasks: []) { XCTAssertNil($0["failure"]) }
    XCTAssertFalse(FileManager.default.fileExists(atPath: stage.path))
  }

  func testSnapshotOfCompletedReceiptDoesNotRewriteOrResyncTheLedger() throws {
    let root = FileManager.default.temporaryDirectory
      .appendingPathComponent("background-snapshot-" + UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: root) }
    var lease = BackgroundLease(id: UUID().uuidString.lowercased(), generation: UUID().uuidString.lowercased(),
      url: "https://download.kiwix.org/zim/book.zim", expectedSize: 4, allowsCellular: false, state: "complete")
    let stage = root.appendingPathComponent(lease.key + ".download")
    try Data([1, 2, 3, 4]).write(to: stage)
    var info = stat()
    XCTAssertEqual(lstat(stage.path, &info), 0)
    lease.copied = 4; lease.total = 4
    lease.device = UInt64(info.st_dev); lease.inode = UInt64(info.st_ino)
    let manifest = root.appendingPathComponent("leases.json")
    let saved = try JSONEncoder().encode(BackgroundLedger(policy: BackgroundPolicy(), leases: [lease.key: lease]))
    try saved.write(to: manifest)
    let session = URLSession(configuration: .ephemeral)
    defer { session.invalidateAndCancel() }
    var syncs = 0
    let acquisition = BackgroundAcquisition(root: root, session: session, synchronizeDirectory: { directory in
      syncs += 1
      let descriptor = open(directory.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
      guard descriptor >= 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
      defer { close(descriptor) }
      guard fsync(descriptor) == 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
    })
    try acquisition.applyPolicy(["privateMode": false, "readOnly": false]) { XCTAssertNil($0["failure"]) }
    let before = try Data(contentsOf: manifest)
    let identity: [String: Any] = ["id": lease.id, "generation": lease.generation]
    // Polling progress on a finished download is read-only: the receipt is
    // already durable, so neither the ledger nor its directory may be rewritten.
    for _ in 0..<3 {
      var snapshot: [String: Any] = [:]
      try acquisition.perform("snapshot", args: identity, tasks: []) { snapshot = $0 }
      XCTAssertEqual(snapshot["state"] as? String, "complete")
      XCTAssertEqual(snapshot["path"] as? String, stage.path)
      XCTAssertEqual(snapshot["inode"] as? UInt64, UInt64(info.st_ino))
    }
    XCTAssertEqual(try Data(contentsOf: manifest), before)
    XCTAssertEqual(syncs, 0)
  }

  @MainActor
  func testClassHandoffInstallationPreservesActualApplicationDelegateAndPriorHandler() {
    let application = UIApplication.shared
    let original = application.delegate
    weak var released: ForwardingBackgroundDelegate?
    autoreleasepool {
      let instance = ForwardingBackgroundDelegate()
      released = instance
      XCTAssertTrue(BackgroundAcquisition.install(on: ForwardingBackgroundDelegate.self))
      XCTAssertTrue(BackgroundAcquisition.install(on: ForwardingBackgroundDelegate.self))
      let delegate: UIApplicationDelegate = instance
      var completions = 0
      delegate.application?(application, handleEventsForBackgroundURLSession: "another.owner.session",
        completionHandler: { completions += 1 })
      XCTAssertEqual(instance.forwarded, ["another.owner.session"])
      XCTAssertEqual(completions, 1)
      XCTAssertTrue((application.delegate as AnyObject?) === (original as AnyObject?))
    }
    // Installation captures the class/previous IMP, not a delegate instance or
    // a replacement lifecycle owner. UIApplication's assign property is untouched.
    XCTAssertNil(released)
    XCTAssertTrue((application.delegate as AnyObject?) === (original as AnyObject?))
  }

  @MainActor
  func testClassHandoffWithoutPriorHandlerCompletesUnknownSessionExactlyOnce() {
    XCTAssertTrue(BackgroundAcquisition.install(on: EmptyBackgroundDelegate.self))
    let delegate: UIApplicationDelegate = EmptyBackgroundDelegate()
    var completions = 0
    delegate.application?(UIApplication.shared, handleEventsForBackgroundURLSession: "unrecognized.session",
      completionHandler: { completions += 1 })
    XCTAssertEqual(completions, 1)
  }
}
