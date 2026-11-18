import Darwin
import Foundation
import XCTest
@testable import KiwixMobile

final class MobileBackupPolicyTests: XCTestCase {
  private func ownedDirectory() throws -> URL {
    let support = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
    let root = support.appendingPathComponent("backup-policy-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
    return root
  }

  private func create(_ path: URL) throws -> Int32 {
    let file = open(path.path, O_RDWR | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, S_IRUSR | S_IWUSR)
    guard file >= 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
    return file
  }

  private func admitted(_ file: Int32, _ path: URL) -> Int32 {
    path.path.withCString { kiwixMobileArchiveAdmitted(file, $0) }
  }

  func testRepeatedBackupReconciliationPreservesArchiveChangeTime() throws {
    let root = try ownedDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    let path = root.appendingPathComponent("book.zim")
    let file = try create(path)
    defer { close(file) }
    func changeTime() throws -> [Int] {
      var info = stat()
      guard fstat(file, &info) == 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
      return [info.st_ctimespec.tv_sec, info.st_ctimespec.tv_nsec]
    }
    for include in [false, true, false] {
      try applyOwnedBackupPolicy(file: file, path: path.path, include: include, admit: true)
      let established = try changeTime()
      try applyOwnedBackupPolicy(file: file, path: path.path, include: include, admit: true)
      XCTAssertEqual(try changeTime(), established)
      XCTAssertEqual(admitted(file, path), 1)
      let current = try URL(fileURLWithPath: path.path).resourceValues(forKeys: [.isExcludedFromBackupKey])
      XCTAssertEqual(current.isExcludedFromBackup, !include)
    }
  }

  func testExclusionUsesSupportedFoundationKeyAndPublicationSurvivesHiddenMove() throws {
    let root = try ownedDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    let path = root.appendingPathComponent("book.zim")
    let file = try create(path)
    defer { close(file) }
    try applyOwnedBackupPolicy(file: file, path: path.path, include: false)
    XCTAssertEqual(try path.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup, true)
    XCTAssertEqual(admitted(file, path), 0, "A final-looking basename is not publication evidence")
    try applyOwnedBackupPolicy(file: file, path: path.path, include: true, admit: true)
    // Foundation caches resource values on URL instances; query current policy.
    XCTAssertEqual(try URL(fileURLWithPath: path.path).resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup, false)
    XCTAssertEqual(admitted(file, path), 1)
    let collection = root.appendingPathComponent(".collection", isDirectory: true)
    try FileManager.default.createDirectory(at: collection, withIntermediateDirectories: false)
    let moved = collection.appendingPathComponent(".book.zim")
    XCTAssertEqual(rename(path.path, moved.path), 0)
    XCTAssertEqual(admitted(file, moved), 1, "Admission follows the retained inode, not the old pathname")
    try applyOwnedBackupPolicy(file: file, path: moved.path, include: false)
    XCTAssertEqual(try moved.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup, true)
    XCTAssertGreaterThanOrEqual(fcntl(file, F_GETFD), 0, "The caller still owns its descriptor")
  }

  func testReplacementBeforeHookCannotMutateEitherInode() throws {
    let root = try ownedDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    let path = root.appendingPathComponent("book.zim")
    let retainedPath = root.appendingPathComponent("retained.zim")
    let file = try create(path)
    defer { close(file) }
    XCTAssertEqual(rename(path.path, retainedPath.path), 0)
    let replacement = try create(path)
    defer { close(replacement) }
    XCTAssertThrowsError(try applyOwnedBackupPolicy(file: file, path: path.path, include: false))
    let key = "com.apple.metadata:com_apple_backup_excludeItem"
    XCTAssertEqual(fgetxattr(file, key, nil, 0, 0, 0), -1)
    XCTAssertEqual(errno, ENOATTR)
    XCTAssertEqual(fgetxattr(replacement, key, nil, 0, 0, 0), -1)
    XCTAssertEqual(errno, ENOATTR)
    XCTAssertEqual(admitted(replacement, path), 0)
  }

  func testStaleNegativeScanCannotUndoConcurrentPublication() throws {
    let root = try ownedDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    let scanned = root.appendingPathComponent(".kiwix-\(UUID().uuidString).part")
    let published = root.appendingPathComponent("book.zim")
    let file = try create(scanned)
    defer { close(file) }
    try applyOwnedBackupPolicy(file: file, path: scanned.path, include: false)
    XCTAssertEqual(admitted(file, scanned), 0, "The scan snapshots no admission")
    XCTAssertEqual(scanned.path.withCString { kiwixMobileExcludeUnadmittedPayload(file, $0) }, 0)
    XCTAssertEqual(try scanned.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup, true)
    // Publication wins after the negative snapshot but before its delayed
    // exclusion. This is the exact race without timing-dependent scheduling.
    XCTAssertEqual(rename(scanned.path, published.path), 0)
    try applyOwnedBackupPolicy(file: file, path: published.path, include: true, admit: true)
    XCTAssertEqual(scanned.path.withCString { kiwixMobileExcludeUnadmittedPayload(file, $0) }, 0)
    XCTAssertEqual(try published.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup, false)
    XCTAssertEqual(admitted(file, published), 1)
  }

  func testIncompleteNativeImportNeverGainsAdmissionFromItsName() throws {
    let root = try ownedDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    let staging = root.appendingPathComponent(UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: staging, withIntermediateDirectories: false)
    let path = staging.appendingPathComponent(".unfinished.zim")
    let file = try create(path)
    defer { close(file) }
    try applyOwnedBackupPolicy(file: file, path: path.path, include: false)
    XCTAssertEqual(admitted(file, path), 0)
    XCTAssertThrowsError(try applyOwnedBackupPolicy(file: file, path: path.path, include: true))
    XCTAssertEqual(try path.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup, true)
  }

  func testExportedHookAcceptsUnmodifiedPosixCanonicalPath() throws {
    let root = try ownedDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    let path = root.appendingPathComponent("canonical.zim")
    let file = try create(path)
    defer { close(file) }
    guard let canonical = realpath(path.path, nil) else {
      throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno))
    }
    defer { free(canonical) }
    // Keep the FFI argument in POSIX form; a Foundation roundtrip can hide /private.
    XCTAssertEqual(kiwixMobileArchiveBackup(file, canonical, 0, 0), 0)
    XCTAssertEqual(try URL(fileURLWithPath: path.path).resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup, true)
    XCTAssertEqual(kiwixMobileArchiveBackup(file, canonical, 1, 1), 0)
    XCTAssertEqual(kiwixMobileArchiveAdmitted(file, canonical), 1)
    XCTAssertEqual(try URL(fileURLWithPath: path.path).resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup, false)
  }

  func testAliasNormalizationNeverAcceptsDescendantSymlink() throws {
    let root = try ownedDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    let target = root.appendingPathComponent("real", isDirectory: true)
    try FileManager.default.createDirectory(at: target, withIntermediateDirectories: false)
    let path = target.appendingPathComponent("book.zim")
    let file = try create(path)
    defer { close(file) }
    try applyOwnedBackupPolicy(file: file, path: path.path, include: true, admit: true)
    let alias = root.appendingPathComponent("alias", isDirectory: true)
    try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: target)
    let aliasPath = alias.appendingPathComponent("book.zim").path
    XCTAssertEqual(aliasPath.withCString { kiwixMobileArchiveBackup(file, $0, 0, 0) }, -1)
    XCTAssertEqual(try URL(fileURLWithPath: path.path).resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup, false)
    XCTAssertEqual(admitted(file, path), 1)
  }
}
