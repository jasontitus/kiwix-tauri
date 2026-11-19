import Darwin
import Foundation
import XCTest
import UIKit
@testable import KiwixMobile

private final class IncomingDocumentDelegate: NSObject, UIApplicationDelegate {
  var forwarded = 0
  func application(_ app: UIApplication, open url: URL,
                   options: [UIApplication.OpenURLOptionsKey: Any] = [:]) -> Bool {
    forwarded += 1
    return true
  }
}

final class ExternalDocumentsTests: XCTestCase {
  private let key = "kiwix.external-document-references.v1"

  private func fixture(_ body: (URL, UserDefaults) throws -> Void) throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
    let suite = "ExternalDocumentsTests.\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    defer {
      defaults.removePersistentDomain(forName: suite)
      try? FileManager.default.removeItem(at: root)
    }
    try body(root, defaults)
  }

  private func archive(_ url: URL, uuid: UInt8 = 1) throws {
    var bytes = [UInt8](repeating: 0, count: 128)
    bytes.replaceSubrange(0..<4, with: [0x5a, 0x49, 0x4d, 0x04])
    bytes.replaceSubrange(8..<24, with: repeatElement(uuid, count: 16))
    try Data(bytes).write(to: url)
  }

  func testShareCopiesCanonicalArchiveAndCleansOnlyItsScratch() throws {
    let support = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
    let root = support.appendingPathComponent("export-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: root) }
    let source = root.appendingPathComponent("book.zim")
    try archive(source)
    let original = try Data(contentsOf: source)
    let canonical = try XCTUnwrap(realpath(source.path, nil))
    defer { free(canonical) }
    let export = try prepareExport(path: String(cString: canonical), suggestedName: "shared.zim",
      cancellation: WorkCancellation(), progress: { _, _ in })
    XCTAssertEqual(try Data(contentsOf: export.url), original)
    XCTAssertNotEqual(export.url, source)
    try export.cleanup()
    XCTAssertFalse(FileManager.default.fileExists(atPath: export.directoryURL.path))
    XCTAssertEqual(try Data(contentsOf: source), original)
  }

  func testShareRejectsSymlinkSourceWithoutRemovingOriginal() throws {
    let support = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
    let root = support.appendingPathComponent("export-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: root) }
    let real = root.appendingPathComponent("real", isDirectory: true)
    try FileManager.default.createDirectory(at: real, withIntermediateDirectories: false)
    let source = real.appendingPathComponent("book.zim")
    try archive(source)
    let original = try Data(contentsOf: source)
    let alias = root.appendingPathComponent("alias", isDirectory: true)
    try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: real)
    let leaf = root.appendingPathComponent("link.zim")
    try FileManager.default.createSymbolicLink(at: leaf, withDestinationURL: source)
    for path in [alias.appendingPathComponent("book.zim").path, leaf.path, "/etc/hosts"] {
      XCTAssertThrowsError(try prepareExport(path: path, suggestedName: "shared.zim",
        cancellation: WorkCancellation(), progress: { _, _ in }))
    }
    XCTAssertEqual(try Data(contentsOf: source), original)
  }

  func testSavedOriginalReopensAfterRestartWithoutCopyAndForgetPreservesSource() throws {
    try fixture { root, defaults in
      let source = root.appendingPathComponent("original.zim")
      try archive(source)
      let original = try Data(contentsOf: source)
      var before = stat()
      XCTAssertEqual(stat(source.path, &before), 0)
      let selected = try ExternalDocuments(defaults: defaults).select(source, folder: false,
        persistent: true, cancellation: WorkCancellation())
      XCTAssertTrue(selected.persistent)
      let reopened = ExternalDocuments(defaults: defaults)
      let acquired = try reopened.acquire(selected.reference, cancellation: WorkCancellation())
      var opened = stat()
      XCTAssertEqual(fstat(try XCTUnwrap(acquired.files.first).fd, &opened), 0)
      XCTAssertEqual(opened.st_dev, before.st_dev)
      XCTAssertEqual(opened.st_ino, before.st_ino, "The reader must use the original, not a copied ZIM")
      XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: root.path), ["original.zim"])
      reopened.release(acquired.lease)
      try reopened.forget(selected.reference)
      XCTAssertEqual(try Data(contentsOf: source), original)
      XCTAssertThrowsError(try reopened.acquire(selected.reference, cancellation: WorkCancellation()))
    }
  }

  func testSessionOnlyIncomingOriginalDoesNotSaveReferenceOrCreateCopy() throws {
    try fixture { root, defaults in
      let source = root.appendingPathComponent("original.zim")
      try archive(source)
      let documents = ExternalDocuments(defaults: defaults)
      let selected = try documents.selectIncoming(source, persistent: false, cancellation: WorkCancellation())
      XCTAssertFalse(selected.persistent)
      XCTAssertNil(defaults.dictionary(forKey: key))
      let acquired = try documents.acquire(selected.reference, cancellation: WorkCancellation())
      documents.release(acquired.lease)
      XCTAssertThrowsError(try ExternalDocuments(defaults: defaults).acquire(
        selected.reference, cancellation: WorkCancellation()))
      try documents.forget(selected.reference)
      XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: root.path), ["original.zim"])
      XCTAssertTrue(FileManager.default.fileExists(atPath: source.path))
    }
  }

  func testCollidingAcquisitionKeepsTheExistingLeaseOpen() throws {
    try fixture { root, defaults in
      let source = root.appendingPathComponent("original.zim")
      try archive(source)
      let id = UUID()
      let documents = ExternalDocuments(defaults: defaults, newLeaseID: { id })
      let selected = try documents.selectIncoming(source, persistent: false, cancellation: WorkCancellation())
      let first = try documents.acquire(selected.reference, cancellation: WorkCancellation())
      defer { documents.release(first.lease) }
      let descriptor = try XCTUnwrap(first.files.first).fd
      var original = stat()
      XCTAssertEqual(fstat(descriptor, &original), 0)

      XCTAssertThrowsError(try documents.acquire(selected.reference, cancellation: WorkCancellation())) { error in
        XCTAssertEqual(providerAcquisitionCode(error), "PROVIDER_LEASE_CONFLICT")
      }
      var retained = stat()
      XCTAssertEqual(fstat(descriptor, &retained), 0)
      XCTAssertEqual(retained.st_dev, original.st_dev)
      XCTAssertEqual(retained.st_ino, original.st_ino)
    }
  }

  func testIncomingPlainFileURLCannotPretendToCarryPersistentProviderPermission() throws {
    try fixture { root, defaults in
      let source = root.appendingPathComponent("original.zim")
      try archive(source)
      let original = try Data(contentsOf: source)
      XCTAssertThrowsError(try ExternalDocuments(defaults: defaults).selectIncoming(
        source, persistent: true, cancellation: WorkCancellation())) { error in
        XCTAssertEqual((error as? MobileFailure)?.code, "provider_incoming_choose_original_or_copy")
      }
      XCTAssertNil(defaults.dictionary(forKey: key))
      XCTAssertEqual(try Data(contentsOf: source), original)
      XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: root.path), ["original.zim"])
    }
  }

  @MainActor
  func testUIKitOpenInPlaceAuthorityIsRequiredAndDismissalDoesNotRevokeSavedReference() throws {
    try fixture { root, defaults in
      let source = root.appendingPathComponent("original.zim")
      try archive(source)
      let application = UIApplication.shared
      let originalDelegate = application.delegate
      let instance = IncomingDocumentDelegate()
      XCTAssertTrue(IncomingOriginalAuthority.install(on: IncomingDocumentDelegate.self))
      XCTAssertTrue(IncomingOriginalAuthority.install(on: IncomingDocumentDelegate.self))
      let delegate: UIApplicationDelegate = instance
      defer { IncomingOriginalAuthority.forget([source]) }
      let documents = ExternalDocuments(defaults: defaults)
      XCTAssertFalse(IncomingOriginalAuthority.canOpenInPlace(source.absoluteString))

      // A real delegate callback without UIKit's in-place attestation is still
      // only a temporary handoff, even if Foundation says scope access worked.
      XCTAssertEqual(delegate.application?(application, open: source, options: [:]), true)
      XCTAssertFalse(IncomingOriginalAuthority.canOpenInPlace(source.absoluteString))
      XCTAssertThrowsError(try documents.selectIncoming(source, persistent: true,
        cancellation: WorkCancellation()))
      XCTAssertNil(defaults.dictionary(forKey: key))
      IncomingOriginalAuthority.forget([source])

      XCTAssertEqual(delegate.application?(application, open: source, options: [.openInPlace: true]), true)
      XCTAssertTrue(IncomingOriginalAuthority.canOpenInPlace(source.absoluteString))
      let selected = try documents.selectIncoming(source, persistent: true, cancellation: WorkCancellation())
      XCTAssertTrue(selected.persistent)
      XCTAssertEqual(instance.forwarded, 2)
      XCTAssertTrue(application.delegate === originalDelegate)
      IncomingOriginalAuthority.forget([source])
      XCTAssertFalse(IncomingOriginalAuthority.canOpenInPlace(source.absoluteString))
      XCTAssertThrowsError(try documents.selectIncoming(source, persistent: true,
        cancellation: WorkCancellation()), "A dismissed handoff cannot mint another saved capability")

      let reopened = ExternalDocuments(defaults: defaults)
      let acquired = try reopened.acquire(selected.reference, cancellation: WorkCancellation())
      var original = stat()
      var opened = stat()
      XCTAssertEqual(stat(source.path, &original), 0)
      XCTAssertEqual(fstat(try XCTUnwrap(acquired.files.first).fd, &opened), 0)
      XCTAssertEqual(original.st_dev, opened.st_dev)
      XCTAssertEqual(original.st_ino, opened.st_ino)
      reopened.release(acquired.lease)
      try reopened.forget(selected.reference)
      XCTAssertTrue(FileManager.default.fileExists(atPath: source.path))
    }
  }

  @MainActor
  func testOverlappingOriginalHandoffsKeepAuthorityUntilTheLastAcknowledgment() throws {
    try fixture { root, defaults in
      let source = root.appendingPathComponent("original.zim")
      try archive(source)
      XCTAssertTrue(IncomingOriginalAuthority.install(on: IncomingDocumentDelegate.self))
      let delegate: UIApplicationDelegate = IncomingDocumentDelegate()
      let application = UIApplication.shared
      defer { IncomingOriginalAuthority.forget([source, source]) }
      XCTAssertEqual(delegate.application?(application, open: source, options: [.openInPlace: true]), true)
      XCTAssertEqual(delegate.application?(application, open: source, options: [.openInPlace: true]), true)
      IncomingOriginalAuthority.forget([source])

      // The first queued request was dismissed, but the second still owns the
      // captured original. It must not be sent back through the picker.
      let documents = ExternalDocuments(defaults: defaults)
      let selected = try documents.selectIncoming(source, persistent: true, cancellation: WorkCancellation())
      XCTAssertTrue(selected.persistent)
      IncomingOriginalAuthority.forget([source])
      XCTAssertThrowsError(try documents.selectIncoming(source, persistent: true,
        cancellation: WorkCancellation()))
      let reopened = ExternalDocuments(defaults: defaults)
      let acquired = try reopened.acquire(selected.reference, cancellation: WorkCancellation())
      reopened.release(acquired.lease)
      try reopened.forget(selected.reference)
      XCTAssertTrue(FileManager.default.fileExists(atPath: source.path))
    }
  }

  func testSplitFolderReadsOriginalPartsWithoutGrantingSiblingPaths() throws {
    try fixture { root, defaults in
      let source = root.appendingPathComponent("parts", isDirectory: true)
      try FileManager.default.createDirectory(at: source, withIntermediateDirectories: false)
      try archive(source.appendingPathComponent("book.zimaa"))
      try Data(repeating: 7, count: 64).write(to: source.appendingPathComponent("book.zimab"))
      let unrelated = source.appendingPathComponent("unrelated.txt")
      try Data("keep".utf8).write(to: unrelated)
      let documents = ExternalDocuments(defaults: defaults)
      let selected = try documents.select(source, folder: true, persistent: true, cancellation: WorkCancellation())
      let acquired = try documents.acquire(selected.reference, cancellation: WorkCancellation())
      XCTAssertEqual(acquired.files.map(\.name), ["book.zimaa", "book.zimab"])
      for file in acquired.files {
        var original = stat()
        var opened = stat()
        XCTAssertEqual(stat(source.appendingPathComponent(file.name).path, &original), 0)
        XCTAssertEqual(fstat(file.fd, &opened), 0)
        XCTAssertEqual(opened.st_dev, original.st_dev)
        XCTAssertEqual(opened.st_ino, original.st_ino)
      }
      documents.release(acquired.lease)
      try documents.forget(selected.reference)
      XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: source.path).sorted(),
        ["book.zimaa", "book.zimab", "unrelated.txt"])
      XCTAssertEqual(try Data(contentsOf: unrelated), Data("keep".utf8))
    }
  }

  func testSplitFolderPreservesMixedCaseNamesAndRejectsMissingParts() throws {
    try fixture { root, defaults in
      let source = root.appendingPathComponent("parts", isDirectory: true)
      try FileManager.default.createDirectory(at: source, withIntermediateDirectories: false)
      try archive(source.appendingPathComponent("BOOK.ZIMAA"))
      let second = source.appendingPathComponent("book.zimAB")
      try Data(repeating: 7, count: 64).write(to: second)
      let documents = ExternalDocuments(defaults: defaults)
      let selected = try documents.select(source, folder: true, persistent: true, cancellation: WorkCancellation())
      let acquired = try documents.acquire(selected.reference, cancellation: WorkCancellation())
      XCTAssertEqual(acquired.files.map(\.name), ["BOOK.ZIMAA", "book.zimAB"])
      documents.release(acquired.lease)
      try FileManager.default.moveItem(at: second, to: source.appendingPathComponent("book.zimAC"))
      XCTAssertThrowsError(try documents.acquire(selected.reference, cancellation: WorkCancellation()))
      XCTAssertThrowsError(try documents.select(source, folder: true, persistent: true, cancellation: WorkCancellation()))
      XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: source.path).sorted(),
        ["BOOK.ZIMAA", "book.zimAC"])
    }
  }

  func testBatchFilesKeepMultipleOriginalsAndReportSplitFileWithoutCopy() throws {
    try fixture { root, defaults in
      let first = root.appendingPathComponent("first.zim")
      let second = root.appendingPathComponent("second.zim")
      let split = root.appendingPathComponent("parts.zimaa")
      try archive(first, uuid: 1)
      try archive(second, uuid: 2)
      try archive(split, uuid: 3)
      let documents = ExternalDocuments(defaults: defaults)
      let result = try documents.selectBatch([first, second, split], folder: false,
        persistent: true, cancellation: WorkCancellation())
      XCTAssertEqual(result.selections.map(\.displayName), ["first.zim", "second.zim"])
      XCTAssertEqual(result.errors.count, 1)
      XCTAssertEqual(result.errors.first?.path, "parts.zimaa")
      for selection in result.selections {
        let acquired = try documents.acquire(selection.reference, cancellation: WorkCancellation())
        var original = stat()
        var opened = stat()
        XCTAssertEqual(stat(root.appendingPathComponent(selection.displayName).path, &original), 0)
        XCTAssertEqual(fstat(try XCTUnwrap(acquired.files.first).fd, &opened), 0)
        XCTAssertEqual(original.st_ino, opened.st_ino)
        documents.release(acquired.lease)
        try documents.forget(selection.reference)
      }
      XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: root.path).sorted(),
        ["first.zim", "parts.zimaa", "second.zim"])
    }
  }

  func testFolderBatchStillRejectsNonRegularPartsWhileWalking() throws {
    try fixture { root, defaults in
      let folder = root.appendingPathComponent("parts", isDirectory: true)
      try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false)
      try archive(folder.appendingPathComponent("book.zimaa"))
      // A batch cohort is admitted during the walk instead of being reopened,
      // so the non-seekable rejection must happen there.
      XCTAssertEqual(mkfifo(folder.appendingPathComponent("book.zimab").path, 0o600), 0)
      let result = try ExternalDocuments(defaults: defaults).selectBatch([folder], folder: true,
        persistent: true, cancellation: WorkCancellation())
      XCTAssertTrue(result.selections.isEmpty)
      XCTAssertEqual(result.errors.map(\.path), ["book.zimaa"])
      XCTAssertEqual(result.errors.first?.error, "kiwix_mobile.provider_not_seekable_use_copy_import")
      XCTAssertNil(defaults.dictionary(forKey: key))
    }
  }

  func testRecursiveFolderBatchReopensEveryOriginalCohortAfterRestart() throws {
    try fixture { root, defaults in
      let nested = root.appendingPathComponent("nested", isDirectory: true)
      try FileManager.default.createDirectory(at: nested, withIntermediateDirectories: false)
      let originals = [root.appendingPathComponent("whole.zim"), nested.appendingPathComponent("other.zim"),
        nested.appendingPathComponent("parts.zimaa"), nested.appendingPathComponent("parts.zimab")]
      try archive(originals[0], uuid: 1)
      try archive(originals[1], uuid: 2)
      try archive(originals[2], uuid: 3)
      try Data(repeating: 7, count: 64).write(to: originals[3])
      let incomplete = nested.appendingPathComponent("missing.zimab")
      try archive(incomplete, uuid: 4)
      let result = try ExternalDocuments(defaults: defaults).selectBatch([root], folder: true,
        persistent: true, cancellation: WorkCancellation())
      XCTAssertEqual(result.selections.map(\.displayName).sorted(), ["other.zim", "parts.zimaa", "whole.zim"])
      XCTAssertEqual(result.errors.map(\.path), ["nested/missing.zimab"])
      let reopened = ExternalDocuments(defaults: defaults)
      for selection in result.selections {
        let acquired = try reopened.acquire(selection.reference, cancellation: WorkCancellation())
        for file in acquired.files {
          let original = try XCTUnwrap(originals.first { $0.lastPathComponent == file.name })
          var expected = stat()
          var opened = stat()
          XCTAssertEqual(stat(original.path, &expected), 0)
          XCTAssertEqual(fstat(file.fd, &opened), 0)
          XCTAssertEqual(opened.st_dev, expected.st_dev)
          XCTAssertEqual(opened.st_ino, expected.st_ino)
        }
        reopened.release(acquired.lease)
        try reopened.forget(selection.reference)
      }
      for original in originals + [incomplete] { XCTAssertTrue(FileManager.default.fileExists(atPath: original.path)) }
    }
  }

  func testRecursiveFolderCapabilityDoesNotFollowReplacedDirectorySymlink() throws {
    try fixture { root, defaults in
      let selectedRoot = root.appendingPathComponent("selected", isDirectory: true)
      let nested = selectedRoot.appendingPathComponent("nested", isDirectory: true)
      let outside = root.appendingPathComponent("outside", isDirectory: true)
      try FileManager.default.createDirectory(at: nested, withIntermediateDirectories: true)
      try FileManager.default.createDirectory(at: outside, withIntermediateDirectories: false)
      try archive(nested.appendingPathComponent("book.zim"))
      try archive(outside.appendingPathComponent("book.zim"))
      let documents = ExternalDocuments(defaults: defaults)
      let result = try documents.selectBatch([selectedRoot], folder: true, persistent: true,
        cancellation: WorkCancellation())
      let selected = try XCTUnwrap(result.selections.first)
      try FileManager.default.moveItem(at: nested, to: selectedRoot.appendingPathComponent("retained"))
      try FileManager.default.createSymbolicLink(at: nested, withDestinationURL: outside)
      XCTAssertThrowsError(try documents.acquire(selected.reference, cancellation: WorkCancellation()))
      try documents.forget(selected.reference)
      XCTAssertTrue(FileManager.default.fileExists(atPath: outside.appendingPathComponent("book.zim").path))
      XCTAssertTrue(FileManager.default.fileExists(atPath: selectedRoot.appendingPathComponent("retained/book.zim").path))
    }
  }

  func testAcquisitionErrorsKeepRetryAndReselectionClassifications() {
    XCTAssertEqual(providerAcquisitionCode(MobileFailure("provider_reference_revoked")), "PROVIDER_REFERENCE_REVOKED")
    XCTAssertEqual(providerAcquisitionCode(MobileFailure("provider_archive_parts_changed")), "PROVIDER_ARCHIVE_PARTS_CHANGED")
    XCTAssertEqual(providerAcquisitionCode(NSError(domain: NSCocoaErrorDomain, code: NSFileReadNoSuchFileError)),
      "PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT")
    XCTAssertEqual(providerAcquisitionCode(NSError(domain: NSCocoaErrorDomain, code: NSFileReadNoPermissionError)),
      "PROVIDER_PERMISSION_DENIED")
  }

  func testStaleMovedBookmarkRefreshesOnlyAfterAllSplitPartsValidate() throws {
    try fixture { root, defaults in
      let folder = root.appendingPathComponent("original", isDirectory: true)
      try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false)
      try archive(folder.appendingPathComponent("book.zimaa"))
      try Data(repeating: 7, count: 64).write(to: folder.appendingPathComponent("book.zimab"))
      let selected = try ExternalDocuments(defaults: defaults).select(folder, folder: true,
        persistent: true, cancellation: WorkCancellation())
      let before = try XCTUnwrap((defaults.dictionary(forKey: key) as? [String: Data])?[selected.reference])
      let moved = root.appendingPathComponent("moved", isDirectory: true)
      try FileManager.default.moveItem(at: folder, to: moved)
      let documents = ExternalDocuments(defaults: defaults, resolveBookmark: { _ in (moved, true) })
      let acquired = try documents.acquire(selected.reference, cancellation: WorkCancellation())
      defer { documents.release(acquired.lease) }
      XCTAssertEqual(acquired.files.map(\.name), ["book.zimaa", "book.zimab"])
      let after = try XCTUnwrap((defaults.dictionary(forKey: key) as? [String: Data])?[selected.reference])
      let record = try XCTUnwrap(JSONSerialization.jsonObject(with: after) as? [String: Any])
      let bookmark = try XCTUnwrap((record["bookmark"] as? String).flatMap { Data(base64Encoded: $0) })
      var stale = false
      let refreshedURL = try URL(resolvingBookmarkData: bookmark, options: [.withoutUI],
        relativeTo: nil, bookmarkDataIsStale: &stale)
      XCTAssertEqual(refreshedURL.standardizedFileURL, moved.standardizedFileURL)
      XCTAssertNotEqual(before, after, "The moved security-scoped reference must actually be persisted")
    }
  }

  func testStaleBookmarkCannotRefreshDifferentArchiveUUID() throws {
    try fixture { root, defaults in
      let file = root.appendingPathComponent("book.zim")
      try archive(file)
      let selected = try ExternalDocuments(defaults: defaults).select(file, folder: false,
        persistent: true, cancellation: WorkCancellation())
      let before = defaults.dictionary(forKey: key) as? [String: Data]
      try archive(file, uuid: 2)
      let documents = ExternalDocuments(defaults: defaults, resolveBookmark: { _ in (file, true) })
      XCTAssertThrowsError(try documents.acquire(selected.reference, cancellation: WorkCancellation()))
      XCTAssertEqual(defaults.dictionary(forKey: key) as? [String: Data], before)
    }
  }

  func testStaleBookmarkCannotRefreshReplacementSplitPartWithSameNameAndSize() throws {
    try fixture { root, defaults in
      let folder = root.appendingPathComponent("parts", isDirectory: true)
      try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false)
      try archive(folder.appendingPathComponent("book.zimaa"))
      let part = folder.appendingPathComponent("book.zimab")
      try Data(repeating: 7, count: 64).write(to: part)
      let selected = try ExternalDocuments(defaults: defaults).select(folder, folder: true,
        persistent: true, cancellation: WorkCancellation())
      let before = defaults.dictionary(forKey: key) as? [String: Data]
      try FileManager.default.moveItem(at: part, to: root.appendingPathComponent("retained-original"))
      try Data(repeating: 8, count: 64).write(to: part)
      let documents = ExternalDocuments(defaults: defaults, resolveBookmark: { _ in (folder, true) })
      XCTAssertThrowsError(try documents.acquire(selected.reference, cancellation: WorkCancellation()))
      XCTAssertEqual(defaults.dictionary(forKey: key) as? [String: Data], before)
    }
  }
  func testLegacyStaleRecordIsPreservedUntilIdentityCanBeEstablished() throws {
    try fixture { root, defaults in
      let file = root.appendingPathComponent("book.zim")
      try archive(file)
      let selected = try ExternalDocuments(defaults: defaults).select(file, folder: false,
        persistent: true, cancellation: WorkCancellation())
      var stored = try XCTUnwrap(defaults.dictionary(forKey: key) as? [String: Data])
      let original = try XCTUnwrap(stored[selected.reference])
      var record = try XCTUnwrap(JSONSerialization.jsonObject(with: original) as? [String: Any])
      record.removeValue(forKey: "identities")
      record.removeValue(forKey: "archiveUUID")
      stored[selected.reference] = try JSONSerialization.data(withJSONObject: record)
      defaults.set(stored, forKey: key)
      let stale = ExternalDocuments(defaults: defaults, resolveBookmark: { _ in (file, true) })
      XCTAssertThrowsError(try stale.acquire(selected.reference, cancellation: WorkCancellation()))
      XCTAssertEqual(defaults.dictionary(forKey: key) as? [String: Data], stored)
      // A valid, nonstale reopen establishes a baseline for later safe refresh.
      let valid = ExternalDocuments(defaults: defaults, resolveBookmark: { _ in (file, false) })
      let first = try valid.acquire(selected.reference, cancellation: WorkCancellation())
      valid.release(first.lease)
      let recovered = try stale.acquire(selected.reference, cancellation: WorkCancellation())
      stale.release(recovered.lease)
    }
  }
}
