import CryptoKit
import Darwin
import Foundation
import ObjectiveC
import UIKit

/// Only UIKit's original Open-In callback can attest that a handoff is in
/// place. A parsed file URL, bookmark encoding, or successful scope call alone
/// does not distinguish a provider original from a temporary Inbox/sandbox URL.
final class IncomingOriginalAuthority {
  private final class Original {
    let url: URL
    let scoped: Bool
    let openInPlace: Bool
    var pendingHandoffs = 1
    init(_ url: URL, openInPlace: Bool) {
      self.url = url; self.openInPlace = openInPlace
      scoped = url.startAccessingSecurityScopedResource()
    }
    deinit { if scoped { url.stopAccessingSecurityScopedResource() } }
  }
  private static let lock = NSLock()
  private static var originals: [String: Original] = [:]
  private static var installedClasses: Set<ObjectIdentifier> = []

  private static func capture(_ url: URL, openInPlace: Bool) {
    guard url.isFileURL else { return }
    lock.lock()
    if let existing = originals[url.absoluteString] {
      if openInPlace && !existing.openInPlace {
        let original = Original(url, openInPlace: true)
        original.pendingHandoffs = existing.pendingHandoffs + 1
        originals[url.absoluteString] = original
      } else {
        existing.pendingHandoffs += 1
      }
    } else {
      originals[url.absoluteString] = Original(url, openInPlace: openInPlace)
    }
    lock.unlock()
  }
  static func forget(_ urls: [URL]) {
    lock.lock()
    for url in urls {
      guard let original = originals[url.absoluteString] else { continue }
      if original.pendingHandoffs > 1 { original.pendingHandoffs -= 1 }
      else { originals.removeValue(forKey: url.absoluteString) }
    }
    lock.unlock()
  }
  static func canOpenInPlace(_ uri: String) -> Bool {
    guard let url = URL(string: uri), url.isFileURL else { return false }
    lock.lock()
    defer { lock.unlock() }
    return originals[url.absoluteString]?.openInPlace == true
  }
  static func withOriginal<T>(_ url: URL, required: Bool, _ body: (URL) throws -> T) throws -> T {
    lock.lock()
    let original = originals[url.absoluteString]
    lock.unlock()
    guard !required || original?.openInPlace == true else {
      throw MobileFailure("provider_incoming_choose_original_or_copy")
    }
    return try withExtendedLifetime(original) { try body(original?.url ?? url) }
  }
  static func install() -> Bool {
    precondition(Thread.isMainThread)
    guard let type = NSClassFromString("AppDelegate") else { return false }
    return install(on: type)
  }
  static func install(on type: AnyClass) -> Bool {
    precondition(Thread.isMainThread)
    let identifier = ObjectIdentifier(type)
    if installedClasses.contains(identifier) { return true }
    let selector = NSSelectorFromString("application:openURL:options:")
    let previous = class_getInstanceMethod(type, selector).map(method_getImplementation)
    typealias OriginalHandler = @convention(c) (AnyObject, Selector, UIApplication, NSURL, NSDictionary) -> Bool
    let block: @convention(block) (AnyObject, UIApplication, NSURL, NSDictionary) -> Bool = {
      object, application, url, options in
      let inPlace = (options[UIApplication.OpenURLOptionsKey.openInPlace.rawValue] as? NSNumber)?.boolValue == true
      capture(url as URL, openInPlace: inPlace)
      if let previous {
        return unsafeBitCast(previous, to: OriginalHandler.self)(object, selector, application, url, options)
      }
      return false
    }
    let implementation = imp_implementationWithBlock(block)
    if !class_addMethod(type, selector, implementation, "B@:@@@") {
      guard let method = class_getInstanceMethod(type, selector) else {
        imp_removeBlock(implementation)
        return false
      }
      method_setImplementation(method, implementation)
    }
    installedClasses.insert(identifier)
    return true
  }
}

// Presentation hint only. selectIncoming still revalidates the captured UIKit
// handoff when the user acts; this does not mint or extend reading authority.
@_cdecl("kiwix_mobile_incoming_can_open_in_place")
public func kiwixMobileIncomingCanOpenInPlace(_ path: UnsafePointer<CChar>?) -> Int32 {
  guard let path, let uri = String(validatingUTF8: path), uri.utf8.count <= 65536 else { return 0 }
  return IncomingOriginalAuthority.canOpenInPlace(uri) ? 1 : 0
}

struct MobileFailure: Error {
  let code: String
  let detail: String?
  let retainedScratchPath: String?
  init(_ code: String, _ detail: String? = nil, retainedScratchPath: String? = nil) {
    self.code = code; self.detail = detail; self.retainedScratchPath = retainedScratchPath
  }
}

final class WorkCancellation {
  private let lock = NSLock()
  private var stopped = false
  private var coordinators: [ObjectIdentifier: NSFileCoordinator] = [:]

  var isCancelled: Bool { lock.lock(); defer { lock.unlock() }; return stopped }
  func check() throws { if isCancelled { throw MobileFailure("cancelled") } }
  func cancel() {
    lock.lock(); stopped = true; let current = Array(coordinators.values); lock.unlock()
    current.forEach { $0.cancel() }
  }
  func register(_ value: NSFileCoordinator) {
    lock.lock(); coordinators[ObjectIdentifier(value)] = value; let cancelled = stopped; lock.unlock()
    if cancelled { value.cancel() }
  }
  func unregister(_ value: NSFileCoordinator) {
    lock.lock(); coordinators.removeValue(forKey: ObjectIdentifier(value)); lock.unlock()
  }
}

func posixFailure(_ operation: String) -> MobileFailure {
  let code = errno
  if operation.hasPrefix("open_provider") {
    if code == ENOENT || code == ENODEV {
      return MobileFailure("provider_document_unavailable_use_copy_import")
    }
    if code == EACCES || code == EPERM { return MobileFailure("provider_permission_denied") }
  }
  return MobileFailure(operation, String(cString: strerror(code)))
}

func providerAcquisitionCode(_ error: Error) -> String {
  if let failure = error as? MobileFailure { return failure.code.uppercased() }
  let native = error as NSError
  if native.domain == NSCocoaErrorDomain {
    if native.code == NSFileNoSuchFileError || native.code == NSFileReadNoSuchFileError {
      return "PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT"
    }
    if native.code == NSFileReadNoPermissionError { return "PROVIDER_PERMISSION_DENIED" }
  }
  return "PROVIDER_UNAVAILABLE_USE_COPY_IMPORT"
}

func safeComponent(_ name: String) throws {
  guard !name.isEmpty, name != ".", name != "..", !name.contains("/"),
        !name.contains("\\"), !name.contains("\0") else { throw MobileFailure("invalid_name") }
}

private func aliasKey(_ name: String) -> String {
  name.precomposedStringWithCanonicalMapping.folding(options: [.caseInsensitive], locale: Locale(identifier: "en_US_POSIX"))
}

func isArchiveName(_ name: String) -> Bool {
  let lower = name.lowercased()
  if lower.hasSuffix(".zim") { return true }
  guard let range = lower.range(of: ".zim", options: .backwards) else { return false }
  let suffix = lower[range.upperBound...]
  return suffix.utf8.count == 2 && suffix.utf8.allSatisfy { $0 >= 97 && $0 <= 122 }
}

// Walk descriptor-relative below a trusted app root. Never follow a symlink supplied
// in a path component, including when the final directory already exists.
private func openDirectory(_ root: Int32, components: [String]) throws -> Int32 {
  var fd = dup(root)
  guard fd >= 0 else { throw posixFailure("open_directory") }
  do {
    for component in components {
      try safeComponent(component)
      let next = openat(fd, component, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
      guard next >= 0 else { throw posixFailure("open_directory") }
      close(fd); fd = next
    }
    return fd
  } catch { close(fd); throw error }
}

private func relativeComponents(_ url: URL, root: URL) -> [String]? {
  // Foundation standardization is existence-dependent on iOS: an existing
  // /private/var URL can become /var, while a new scratch child stays physical.
  // Compare lexical components against the OS root and its POSIX spelling.
  let parts = url.pathComponents
  let base = root.pathComponents
  guard parts.count > base.count, Array(parts.prefix(base.count)) == base else { return nil }
  return Array(parts.dropFirst(base.count))
}

private func openOwnedParent(_ path: String, staging: Bool) throws -> (Int32, String) {
  guard path.hasPrefix("/"), !path.contains("\0") else { throw MobileFailure("invalid_path") }
  let url = URL(fileURLWithPath: path)
  // Reject path normalization, not merely its final canonical location.
  guard !url.pathComponents.contains(".."), !url.pathComponents.contains(".") else { throw MobileFailure("invalid_path") }
  let fm = FileManager.default
  let roots: [URL]
  if staging {
    roots = fm.urls(for: .documentDirectory, in: .userDomainMask) + fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)
  } else {
    roots = [URL(fileURLWithPath: NSHomeDirectory(), isDirectory: true)]
  }
  for original in roots {
    guard let physical = realpath(original.path, nil) else { throw posixFailure("open_app_directory") }
    defer { free(physical) }
    let canonical = URL(fileURLWithPath: String(cString: physical), isDirectory: true)
    guard let relative = relativeComponents(url, root: original) ?? relativeComponents(url, root: canonical), let leaf = relative.last else { continue }
    let rootFD = open(canonical.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
    guard rootFD >= 0 else { throw posixFailure("open_app_directory") }
    defer { close(rootFD) }
    let parent = try openDirectory(rootFD, components: Array(relative.dropLast()))
    try safeComponent(leaf)
    return (parent, leaf)
  }
  throw MobileFailure("outside_app_storage")
}

private func openOwnedFile(_ path: String) throws -> Int32 {
  let (parent, leaf) = try openOwnedParent(path, staging: false)
  defer { close(parent) }
  let fd = openat(parent, leaf, O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK)
  guard fd >= 0 else { throw posixFailure("open_source") }
  var info = stat()
  guard fstat(fd, &info) == 0, (info.st_mode & S_IFMT) == S_IFREG else {
    close(fd); throw MobileFailure("not_regular_file")
  }
  return fd
}

private func writeAll(_ fd: Int32, _ buffer: UnsafeRawBufferPointer, count: Int) throws {
  var written = 0
  while written < count {
    let result = Darwin.write(fd, buffer.baseAddress!.advanced(by: written), count - written)
    if result < 0 && errno == EINTR { continue }
    guard result > 0 else { throw posixFailure("write_failed") }
    written += result
  }
}

private func stream(_ source: Int32, to destination: Int32, cancellation: WorkCancellation,
                    progress: (Int64) -> Void) throws {
  var buffer = [UInt8](repeating: 0, count: 256 * 1024)
  var total: Int64 = 0
  while true {
    try cancellation.check()
    let count = buffer.withUnsafeMutableBytes { Darwin.read(source, $0.baseAddress!, $0.count) }
    if count < 0 && errno == EINTR { continue }
    guard count >= 0 else { throw posixFailure("read_failed") }
    if count == 0 { break }
    try buffer.withUnsafeBytes { try writeAll(destination, $0, count: count) }
    total += Int64(count); progress(total)
  }
  try cancellation.check()
  guard fsync(destination) == 0 else { throw posixFailure("sync_failed") }
}

func withSecurityScope<T>(_ url: URL, body: (URL) throws -> T) throws -> T {
  guard url.isFileURL else { throw MobileFailure("invalid_provider_url") }
  let access = url.startAccessingSecurityScopedResource()
  defer { if access { url.stopAccessingSecurityScopedResource() } }
  let metadata = try url.resourceValues(forKeys: [.isSymbolicLinkKey])
  guard metadata.isSymbolicLink != true else { throw MobileFailure("unsafe_provider_symlink") }
  return try body(url)
}

func coordinatedRead<T>(_ url: URL, cancellation: WorkCancellation,
                        options: NSFileCoordinator.ReadingOptions = [],
                        body: (URL) throws -> T) throws -> T {
  try withSecurityScope(url) { scoped in
  let coordinator = NSFileCoordinator(filePresenter: nil)
  cancellation.register(coordinator)
  defer { cancellation.unregister(coordinator) }
  var coordinationError: NSError?
  var result: Result<T, Error>?
  coordinator.coordinate(readingItemAt: scoped, options: options, error: &coordinationError) { readable in
    result = Result { try cancellation.check(); return try body(readable) }
  }
  if let result = result { return try result.get() }
  if cancellation.isCancelled { throw MobileFailure("cancelled") }
  throw MobileFailure("provider_unavailable", coordinationError?.localizedDescription)
  }
}

private func sourceFD(_ url: URL) throws -> Int32 {
  let fd = open(url.path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK)
  guard fd >= 0 else { throw posixFailure("open_provider_file") }
  var info = stat()
  guard fstat(fd, &info) == 0, (info.st_mode & S_IFMT) == S_IFREG else {
    close(fd); throw MobileFailure("unsafe_provider_file")
  }
  return fd
}

final class ArchiveStaging {
  let url: URL
  private let fd: Int32
  private let parent: Int32
  private let leaf: String
  private let createdRoot: Bool
  private var owned: [(components: [String], directory: Bool, device: dev_t, inode: ino_t)] = []
  private var names: [String: Set<String>] = [:]
  private(set) var paths: [String] = []

  init(path: String) throws {
    let (parentFD, name) = try openOwnedParent(path, staging: true)
    guard UUID(uuidString: name) != nil else { close(parentFD); throw MobileFailure("staging_requires_uuid") }
    let made = mkdirat(parentFD, name, S_IRWXU) == 0
    if !made && errno != EEXIST { close(parentFD); throw posixFailure("create_staging") }
    let directory = openat(parentFD, name, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
    guard directory >= 0 else { close(parentFD); throw posixFailure("open_staging") }
    // Enumeration is anchored to the descriptor, not a potentially replaced pathname.
    let scanFD = dup(directory)
    guard scanFD >= 0, let scan = fdopendir(scanFD) else {
      if scanFD >= 0 { close(scanFD) }; close(directory); close(parentFD); throw posixFailure("inspect_staging")
    }
    var empty = true
    while let entry = readdir(scan) {
      let entryName = withUnsafePointer(to: &entry.pointee.d_name) {
        $0.withMemoryRebound(to: CChar.self, capacity: Int(NAME_MAX) + 1) { String(cString: $0) }
      }
      if entryName != "." && entryName != ".." { empty = false; break }
    }
    closedir(scan)
    guard empty else { close(directory); close(parentFD); throw MobileFailure("staging_not_empty") }
    self.url = URL(fileURLWithPath: path, isDirectory: true)
    self.fd = directory; self.parent = parentFD; self.leaf = name; self.createdRoot = made
  }

  deinit { close(fd); close(parent) }

  private func reserve(_ components: [String]) throws {
    guard let name = components.last else { throw MobileFailure("invalid_name") }
    try safeComponent(name)
    let parentKey = components.dropLast().joined(separator: "/")
    let key = aliasKey(name)
    guard names[parentKey, default: []].insert(key).inserted else { throw MobileFailure("duplicate_name", components.joined(separator: "/")) }
  }

  func makeDirectory(_ components: [String]) throws {
    try reserve(components)
    let parentFD = try openDirectory(fd, components: Array(components.dropLast()))
    defer { close(parentFD) }
    guard mkdirat(parentFD, components.last!, S_IRWXU) == 0 else { throw posixFailure("create_directory") }
    var info = stat()
    guard fstatat(parentFD, components.last!, &info, AT_SYMLINK_NOFOLLOW) == 0 else { throw posixFailure("inspect_owned_file") }
    owned.append((components, true, info.st_dev, info.st_ino))
  }

  func copy(_ source: URL, components: [String], cancellation: WorkCancellation, progress: (String, Int64) -> Void) throws {
    try cancellation.check()
    try reserve(components)
    let input = try sourceFD(source)
    defer { close(input) }
    let parentFD = try openDirectory(fd, components: Array(components.dropLast()))
    defer { close(parentFD) }
    let output = openat(parentFD, components.last!, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, S_IRUSR | S_IWUSR)
    guard output >= 0 else { throw posixFailure("create_file") }
    var info = stat()
    guard fstat(output, &info) == 0 else { close(output); throw posixFailure("inspect_owned_file") }
    owned.append((components, false, info.st_dev, info.st_ino))
    defer { close(output) }
    let destination = components.reduce(url) { $0.appendingPathComponent($1) }
    try applyOwnedBackupPolicy(file: output, path: destination.path, include: false)
    try stream(input, to: output, cancellation: cancellation) { progress(components.joined(separator: "/"), $0) }
    guard fsync(parentFD) == 0 else { throw posixFailure("sync_failed") }
    paths.append(destination.path)
  }

  func copyTree(_ source: URL, relative: [String] = [], cancellation: WorkCancellation, progress: (String, Int64) -> Void) throws {
    try cancellation.check()
    // Do not hold a directory read claim while materializing cloud children:
    // a provider may need a coordinated write to download each file.
    let children = try coordinatedRead(source, cancellation: cancellation, options: .immediatelyAvailableMetadataOnly) { directory -> [(name: String, directory: Bool, regular: Bool)] in
      let metadata = try directory.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
      guard metadata.isDirectory == true, metadata.isSymbolicLink != true else { throw MobileFailure("unsafe_provider_directory") }
      let entries = try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.isDirectoryKey, .isRegularFileKey, .isSymbolicLinkKey], options: [])
      var aliases = Set<String>()
      return try entries.map { child in
        let name = child.lastPathComponent
        try safeComponent(name)
        guard aliases.insert(aliasKey(name)).inserted else { throw MobileFailure("duplicate_name", name) }
        let values = try child.resourceValues(forKeys: [.isDirectoryKey, .isRegularFileKey, .isSymbolicLinkKey])
        guard values.isSymbolicLink != true else { throw MobileFailure("unsafe_provider_symlink", name) }
        return (name: name, directory: values.isDirectory == true, regular: values.isRegularFile == true)
      }
    }
    for child in children {
      try cancellation.check()
      let components = relative + [child.name]
      // Only validated names/metadata escape an accessor. Reconstruct each URL
      // against the selected scoped location and acquire a fresh read claim.
      let selectedChild = source.appendingPathComponent(child.name, isDirectory: child.directory)
      if child.directory {
        try makeDirectory(components)
        try copyTree(selectedChild, relative: components, cancellation: cancellation, progress: progress)
      } else if isArchiveName(child.name) {
        guard child.regular else { throw MobileFailure("unsafe_provider_file", child.name) }
        try coordinatedRead(selectedChild, cancellation: cancellation) { readable in
          try copy(readable, components: components, cancellation: cancellation, progress: progress)
        }
      }
    }
    let directory = try openDirectory(fd, components: relative)
    defer { close(directory) }
    guard fsync(directory) == 0 else { throw posixFailure("sync_failed") }
  }

  func finish() throws {
    let (currentParent, currentLeaf) = try openOwnedParent(url.path, staging: true)
    defer { close(currentParent) }
    var current = stat(); var original = stat()
    guard fstatat(currentParent, currentLeaf, &current, AT_SYMLINK_NOFOLLOW) == 0,
          fstat(fd, &original) == 0, current.st_dev == original.st_dev,
          current.st_ino == original.st_ino else { throw MobileFailure("staging_identity_changed") }
    guard fsync(fd) == 0, fsync(parent) == 0 else { throw posixFailure("sync_failed") }
  }

  // Only paths created with O_EXCL/mkdirat by this operation are eligible.
  // A failed cleanup is reported instead of claiming that partial files vanished.
  func cancelAndRemoveOwnedFiles() throws {
    for item in owned.reversed() {
      let parentFD = try openDirectory(fd, components: Array(item.components.dropLast()))
      var info = stat()
      guard fstatat(parentFD, item.components.last!, &info, AT_SYMLINK_NOFOLLOW) == 0,
            info.st_dev == item.device, info.st_ino == item.inode else {
        close(parentFD); throw MobileFailure("owned_file_identity_changed")
      }
      let result = unlinkat(parentFD, item.components.last!, item.directory ? AT_REMOVEDIR : 0)
      close(parentFD)
      guard result == 0 else { throw posixFailure("cleanup_failed") }
    }
    guard fsync(fd) == 0 else { throw posixFailure("sync_failed") }
    if createdRoot {
      // Refuse removal if the name was replaced while our descriptor stayed open.
      var current = stat(); var original = stat()
      guard fstatat(parent, leaf, &current, AT_SYMLINK_NOFOLLOW) == 0,
            fstat(fd, &original) == 0, current.st_dev == original.st_dev,
            current.st_ino == original.st_ino else { throw MobileFailure("staging_identity_changed") }
      guard unlinkat(parent, leaf, AT_REMOVEDIR) == 0 else { throw posixFailure("cleanup_failed") }
    }
    guard fsync(parent) == 0 else { throw posixFailure("sync_failed") }
  }
}

func readBoundedText(_ url: URL, maxBytes: Int, cancellation: WorkCancellation) throws -> String {
  try coordinatedRead(url, cancellation: cancellation) { source in
    let input = try sourceFD(source)
    defer { close(input) }
    var bytes = Data()
    var chunk = [UInt8](repeating: 0, count: 64 * 1024)
    while true {
      try cancellation.check()
      let remaining = maxBytes - bytes.count
      let requested = remaining >= chunk.count ? chunk.count : remaining + 1
      let count = chunk.withUnsafeMutableBytes { Darwin.read(input, $0.baseAddress!, requested) }
      if count < 0 && errno == EINTR { continue }
      guard count >= 0 else { throw posixFailure("read_failed") }
      if count == 0 { break }
      guard count <= remaining else { throw MobileFailure("text_too_large") }
      bytes.append(contentsOf: chunk.prefix(count))
    }
    guard let text = String(data: bytes, encoding: .utf8) else { throw MobileFailure("invalid_utf8") }
    return text
  }
}

final class ExportScratch {
  let directoryURL: URL
  let url: URL
  private let parentFD: Int32
  private let directoryFD: Int32
  private let leaf: String
  private let name: String
  private let directoryIdentity: (device: dev_t, inode: ino_t)
  private var fileIdentity: (device: dev_t, inode: ino_t)?
  private var removedDirectory = false

  init(name: String) throws {
    try safeComponent(name)
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("kiwix-export-" + UUID().uuidString, isDirectory: true)
    let (parent, component) = try openOwnedParent(directory.path, staging: false)
    guard mkdirat(parent, component, S_IRWXU) == 0 else {
      close(parent); throw posixFailure("create_export_directory")
    }
    let fd = openat(parent, component, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
    guard fd >= 0 else {
      let detail = String(cString: strerror(errno))
      close(parent)
      throw MobileFailure("export_scratch_unavailable", detail, retainedScratchPath: directory.path)
    }
    var info = stat()
    guard fstat(fd, &info) == 0 else {
      let detail = String(cString: strerror(errno))
      close(fd); close(parent)
      throw MobileFailure("export_scratch_unavailable", detail, retainedScratchPath: directory.path)
    }
    directoryURL = directory
    url = directory.appendingPathComponent(name)
    parentFD = parent; directoryFD = fd; leaf = component; self.name = name
    directoryIdentity = (info.st_dev, info.st_ino)
  }

  deinit { close(directoryFD); close(parentFD) }

  func writeFrom(_ input: Int32, cancellation: WorkCancellation, progress: (String, Int64) -> Void) throws {
    let output = openat(directoryFD, name, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, S_IRUSR | S_IWUSR)
    guard output >= 0 else { throw posixFailure("create_export") }
    defer { close(output) }
    var info = stat()
    guard fstat(output, &info) == 0 else { throw posixFailure("inspect_export") }
    fileIdentity = (info.st_dev, info.st_ino)
    try stream(input, to: output, cancellation: cancellation) { progress(name, $0) }
    guard fsync(directoryFD) == 0, fsync(parentFD) == 0 else { throw posixFailure("sync_failed") }
  }

  func cleanup() throws {
    if removedDirectory {
      guard fsync(parentFD) == 0 else { throw posixFailure("export_cleanup_sync_failed") }
      return
    }
    var directory = stat()
    guard fstatat(parentFD, leaf, &directory, AT_SYMLINK_NOFOLLOW) == 0,
          directory.st_dev == directoryIdentity.device,
          directory.st_ino == directoryIdentity.inode else { throw MobileFailure("export_directory_identity_changed") }
    if let expected = fileIdentity {
      var file = stat()
      guard fstatat(directoryFD, name, &file, AT_SYMLINK_NOFOLLOW) == 0,
            file.st_dev == expected.device, file.st_ino == expected.inode else {
        throw MobileFailure("export_file_identity_changed")
      }
      guard unlinkat(directoryFD, name, 0) == 0 else { throw posixFailure("export_unlink_failed") }
      fileIdentity = nil
    }
    guard fsync(directoryFD) == 0 else { throw posixFailure("export_cleanup_sync_failed") }
    // Never recursively remove unknown files or a replaced directory.
    guard unlinkat(parentFD, leaf, AT_REMOVEDIR) == 0 else { throw posixFailure("export_rmdir_failed") }
    removedDirectory = true
    guard fsync(parentFD) == 0 else { throw posixFailure("export_cleanup_sync_failed") }
  }
}

struct ExportPreparationFailure: Error {
  let scratch: ExportScratch
  let underlying: Error
}

func prepareExport(path: String, suggestedName: String, cancellation: WorkCancellation, progress: (String, Int64) -> Void) throws -> ExportScratch {
  try safeComponent(suggestedName)
  let input = try openOwnedFile(path)
  defer { close(input) }
  let scratch = try ExportScratch(name: suggestedName)
  do {
    try scratch.writeFrom(input, cancellation: cancellation, progress: progress)
    return scratch
  } catch {
    // Ownership follows the failure to the plugin. It must finish identity-safe
    // cleanup before resolving either cancellation or the original error.
    throw ExportPreparationFailure(scratch: scratch, underlying: error)
  }
}

func persistFolderBookmark(_ source: URL, destination: URL) throws {
  let scoped = source.startAccessingSecurityScopedResource()
  defer { if scoped { source.stopAccessingSecurityScopedResource() } }
  let bookmark = try source.bookmarkData(options: .minimalBookmark, includingResourceValuesForKeys: nil, relativeTo: nil)
  let support = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
  let directory = support.appendingPathComponent("kiwix-provider-bookmarks", isDirectory: true)
  try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
  let target = directory.appendingPathComponent(destination.lastPathComponent + ".bookmark")
  let (parent, leaf) = try openOwnedParent(target.path, staging: false)
  defer { close(parent) }
  let output = openat(parent, leaf, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, S_IRUSR | S_IWUSR)
  guard output >= 0 else { throw posixFailure("create_bookmark") }
  defer { close(output) }
  try bookmark.withUnsafeBytes { try writeAll(output, $0, count: $0.count) }
  guard fsync(output) == 0, fsync(parent) == 0 else { throw posixFailure("sync_failed") }
}

// Authority is created only by UIDocumentPicker or the native Open-In queue.
// A reference is a random app-owned key, never a URL supplied by the webview.
struct ExternalSelection: Encodable {
  let reference: String
  let displayName: String
  let persistent: Bool
}
struct ExternalDescriptor: Encodable { let name: String; let fd: Int32 }
struct ExternalAcquisition: Encodable { let lease: String; let files: [ExternalDescriptor] }
struct ExternalSelectionError: Encodable { let path: String; let error: String }
struct ExternalBatchSelection: Encodable {
  var selections: [ExternalSelection] = []
  var errors: [ExternalSelectionError] = []
  let cancelled = false
}

final class ExternalDocuments {
  private struct PartIdentity: Codable, Equatable {
    let size: Int64
    let resource: Data?
    let digest: Data?
    /// Providers that expose no resource identifier: a moved but unchanged part
    /// keeps its device, inode and modification time, so the recorded digest can
    /// be reused instead of streaming the whole file again. These are optional so
    /// earlier records still decode.
    let device: UInt64?
    let inode: UInt64?
    let modified: Int64?
  }
  private struct Record: Codable {
    let bookmark: Data
    let names: [String]
    let folder: Bool
    let directory: [String]?
    let identities: [PartIdentity]?
    let archiveUUID: Data?
  }
  /// Part identities and archive UUID validated while the picked tree was
  /// walked, so publishing does not open and re-validate every part again.
  private struct AdmittedParts {
    let identities: [PartIdentity]
    let archiveUUID: Data
  }
  private final class Lease {
    let reference: String
    let url: URL
    let scoped: Bool
    let files: [ExternalDescriptor]
    init(reference: String, url: URL, scoped: Bool, files: [ExternalDescriptor]) {
      self.reference = reference; self.url = url; self.scoped = scoped; self.files = files
    }
    deinit {
      files.forEach { close($0.fd) }
      if scoped { url.stopAccessingSecurityScopedResource() }
    }
  }
  // Protected by `lock` so long intake hashing on `hashWorker` can run
  // concurrently with interactive acquire/release/forget on `providerWorker`.
  private let lock = NSLock()
  private var transient: [String: Record] = [:]
  private var selectedURLs: [String: URL] = [:]
  private var leases: [String: Lease] = [:]
  private let defaults: UserDefaults
  private let newLeaseID: () -> UUID
  private let resolveBookmark: (Data) throws -> (URL, Bool)
  private let key = "kiwix.external-document-references.v1"
  init(defaults: UserDefaults = .standard,
       newLeaseID: @escaping () -> UUID = UUID.init,
       resolveBookmark: @escaping (Data) throws -> (URL, Bool) = { data in
         var stale = false
         let url = try URL(resolvingBookmarkData: data, options: [.withoutUI],
                           relativeTo: nil, bookmarkDataIsStale: &stale)
         return (url, stale)
       }) {
    self.defaults = defaults
    self.newLeaseID = newLeaseID
    self.resolveBookmark = resolveBookmark
  }

  private func locked<T>(_ body: () throws -> T) rethrows -> T {
    lock.lock()
    defer { lock.unlock() }
    return try body()
  }

  private func valid(_ reference: String) throws {
    guard UUID(uuidString: reference) != nil else { throw MobileFailure("invalid_provider_reference") }
  }
  private func persistedLocked() -> [String: Data] {
    defaults.dictionary(forKey: key) as? [String: Data] ?? [:]
  }
  private func recordLocked(_ reference: String) throws -> Record {
    try valid(reference)
    if let record = transient[reference] { return record }
    guard let data = persistedLocked()[reference] else { throw MobileFailure("provider_reference_revoked") }
    return try JSONDecoder().decode(Record.self, from: data)
  }
  private func names(_ url: URL, folder: Bool) throws -> [String] {
    if !folder {
      try safeComponent(url.lastPathComponent)
      guard url.pathExtension.lowercased() == "zim" else {
        throw MobileFailure("provider_split_requires_folder")
      }
      return [url.lastPathComponent]
    }
    // Only immediate children of the explicitly chosen folder are authorized.
    let children = try FileManager.default.contentsOfDirectory(at: url,
      includingPropertiesForKeys: [.isSymbolicLinkKey], options: [])
    let names = children.map { $0.lastPathComponent }.filter(isArchiveName)
      .sorted { $0.caseInsensitiveCompare($1) == .orderedAscending }
    guard !names.isEmpty, names.count <= 676 else { throw MobileFailure("provider_choose_archive_folder") }
    for name in names { try safeComponent(name) }
    if names.count == 1, names[0].lowercased().hasSuffix(".zim") { return names }
    guard let first = names.first, first.lowercased().hasSuffix(".zimaa") else {
      throw MobileFailure("provider_choose_one_archive_folder")
    }
    let base = String(first.dropLast(2))
    for (index, name) in names.enumerated() {
      let suffix = String(UnicodeScalar(97 + index / 26)!) + String(UnicodeScalar(97 + index % 26)!)
      guard name.caseInsensitiveCompare(base + suffix) == .orderedSame else {
        throw MobileFailure("provider_incomplete_split_archive")
      }
    }
    return names
  }
  private func archiveUUID(_ fd: Int32) throws -> Data {
    var header = [UInt8](repeating: 0, count: 24)
    guard pread(fd, &header, header.count, 0) == header.count,
          Array(header.prefix(4)) == [0x5a, 0x49, 0x4d, 0x04] else {
      throw MobileFailure("provider_invalid_archive")
    }
    return Data(header[8..<24])
  }

  /// `expected` is the identity admitted for this part on an earlier
  /// resolution. The second element reports whether the recorded content digest
  /// requirement is satisfied: a stale bookmark only re-hashes when the part is
  /// no longer proven to be the same file, because a full-content read of every
  /// part is the dominant cost of re-resolving a large archive.
  private func identity(_ fd: Int32, source: URL, expected: PartIdentity?, stale: Bool,
                        cancellation: WorkCancellation) throws -> (identity: PartIdentity, digestSatisfied: Bool) {
    var info = stat()
    guard fstat(fd, &info) == 0 else { throw posixFailure("inspect_provider_file") }
    let identifier = try source.resourceValues(forKeys: [.fileResourceIdentifierKey]).fileResourceIdentifier
    let resource = identifier.flatMap {
      try? NSKeyedArchiver.archivedData(withRootObject: $0, requiringSecureCoding: false)
    }
    let size = Int64(info.st_size)
    let device = UInt64(info.st_dev), inode = UInt64(info.st_ino)
    let modified = Int64(info.st_mtimespec.tv_sec) * 1_000_000_000 + Int64(info.st_mtimespec.tv_nsec)
    // A provider resource identifier is itself the file's identity on the
    // volumes that expose one, so the descriptor's device/inode/modification
    // time is the same proof for providers that expose none.
    let sameFile = expected.map {
      $0.size == size && ($0.resource == nil ? $0.device == device && $0.inode == inode && $0.modified == modified
                                             : $0.resource == resource)
    } ?? false
    var digest: Data?
    // A first admission from a provider without a resource identifier must bind
    // content. A stale resolution re-hashes whenever that binding was recorded;
    // device/inode/mtime alone are not content proof for a resource-less
    // provider (mtime can be restored after an in-place rewrite).
    if (expected == nil && resource == nil) || (stale && expected?.digest != nil) {
      var hasher = SHA256()
      var buffer = [UInt8](repeating: 0, count: 256 * 1024)
      var offset: off_t = 0
      while true {
        try cancellation.check()
        let count = pread(fd, &buffer, buffer.count, offset)
        if count < 0 && errno == EINTR { continue }
        guard count >= 0 else { throw posixFailure("read_provider_identity") }
        if count == 0 { break }
        buffer.withUnsafeBytes { hasher.update(bufferPointer: UnsafeRawBufferPointer(rebasing: $0[..<count])) }
        offset += off_t(count)
      }
      guard offset == info.st_size else { throw MobileFailure("provider_archive_parts_changed") }
      digest = Data(hasher.finalize())
    }
    return (PartIdentity(size: size, resource: resource, digest: digest,
                         device: device, inode: inode, modified: modified),
            expected?.digest == nil || sameFile || digest == expected?.digest)
  }

  func select(_ url: URL, folder: Bool, persistent: Bool, cancellation: WorkCancellation) throws -> ExternalSelection {
    let record = try coordinatedRead(url, cancellation: cancellation) { readable in
      let bookmark: Data
      if persistent {
        bookmark = try url.bookmarkData(options: .minimalBookmark,
          includingResourceValuesForKeys: nil, relativeTo: nil)
      } else { bookmark = Data() }
      return Record(bookmark: bookmark, names: try self.names(readable, folder: folder),
                    folder: folder, directory: nil, identities: nil, archiveUUID: nil)
    }
    return try publish(url, record: record, admitted: nil, persistent: persistent, cancellation: cancellation)
  }
  private func publish(_ url: URL, record: Record, admitted: AdmittedParts?, persistent: Bool,
                       cancellation: WorkCancellation) throws -> ExternalSelection {
    var added: [String: Data] = [:]
    let selection = try publish(url, record: record, admitted: admitted, persistent: persistent,
                                stored: &added, cancellation: cancellation)
    if persistent, !added.isEmpty {
      locked {
        var stored = persistedLocked()
        for (reference, data) in added { stored[reference] = data }
        defaults.set(stored, forKey: key)
      }
    }
    return selection
  }
  /// Admits one reference. `admitted` carries part identities that were already
  /// validated while the selection was walked, so a large folder cohort is not
  /// opened and re-validated a second time; `stored` collects the encoded
  /// reference so a batch caller writes the reference map once.
  private func publish(_ url: URL, record: Record, admitted: AdmittedParts?, persistent: Bool,
                       stored: inout [String: Data], cancellation: WorkCancellation) throws -> ExternalSelection {
    let reference = UUID().uuidString
    locked {
      transient[reference] = record
      selectedURLs[reference] = url
    }
    do {
      if let admitted = admitted {
        locked {
          transient[reference] = Record(bookmark: record.bookmark, names: record.names,
            folder: record.folder, directory: record.directory,
            identities: admitted.identities, archiveUUID: admitted.archiveUUID)
        }
      } else {
        // Reject nonseekable/nonregular providers before publishing a reference.
        let acquired = try acquire(reference, cancellation: cancellation)
        release(acquired.lease)
      }
      if persistent {
        let finalRecord = try locked { try self.recordLocked(reference) }
        stored[reference] = try JSONEncoder().encode(finalRecord)
      }
      return ExternalSelection(reference: reference, displayName: record.names[0], persistent: persistent)
    } catch {
      locked {
        transient.removeValue(forKey: reference)
        selectedURLs.removeValue(forKey: reference)
      }
      throw error
    }
  }
  func selectBatch(_ urls: [URL], folder: Bool, persistent: Bool,
                   cancellation: WorkCancellation) throws -> ExternalBatchSelection {
    var result = ExternalBatchSelection()
    func failure(_ path: String, _ error: Error) {
      result.errors.append(ExternalSelectionError(path: path,
        error: "kiwix_mobile." + ((error as? MobileFailure)?.code ?? "provider_error")))
    }
    do {
      // Only the references this batch adds are accumulated, then merged into a
      // fresh read of the map, so the single write cannot clobber an entry that
      // a single-selection path persisted in between.
      var added: [String: Data] = [:]
      for url in urls {
        try cancellation.check()
        if !folder {
          do { result.selections.append(try select(url, folder: false, persistent: persistent, cancellation: cancellation)) }
          catch { failure(url.lastPathComponent, error) }
          continue
        }
        var records: [(path: String, record: Record, admitted: AdmittedParts)] = []
        try coordinatedRead(url, cancellation: cancellation) { root in
          let bookmark = persistent ? try url.bookmarkData(options: .minimalBookmark,
            includingResourceValuesForKeys: nil, relativeTo: nil) : Data()
          let rootFD = open(root.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
          guard rootFD >= 0 else { throw posixFailure("open_provider_directory") }
          defer { close(rootFD) }
          var visited = 0
          func visit(_ directory: URL, components: [String]) throws {
            try cancellation.check()
            visited += 1
            guard components.count <= 128, visited <= 100000 else {
              throw MobileFailure("provider_directory_limit")
            }
            let directoryFD = try openDirectory(rootFD, components: components)
            guard let stream = fdopendir(directoryFD) else {
              close(directoryFD); throw posixFailure("open_provider_directory")
            }
            defer { closedir(stream) }
            var cohorts: [String: [String]] = [:]
            while true {
              try cancellation.check()
              errno = 0
              guard let entry = readdir(stream) else {
                if errno != 0 { throw posixFailure("read_provider_directory") }
                break
              }
              let capacity = MemoryLayout.size(ofValue: entry.pointee.d_name)
              let name = withUnsafePointer(to: &entry.pointee.d_name) { pointer in
                pointer.withMemoryRebound(to: CChar.self, capacity: capacity) {
                  String(cString: $0)
                }
              }
              if name == "." || name == ".." { continue }
              try safeComponent(name)
              var info = stat()
              guard fstatat(directoryFD, name, &info, AT_SYMLINK_NOFOLLOW) == 0 else {
                throw posixFailure("inspect_provider_child")
              }
              if (info.st_mode & S_IFMT) == S_IFLNK {
                failure((components + [name]).joined(separator: "/"), MobileFailure("provider_symlink_not_supported"))
              } else if (info.st_mode & S_IFMT) == S_IFDIR {
                try visit(directory.appendingPathComponent(name, isDirectory: true), components: components + [name])
              } else if isArchiveName(name) {
                let key = name.lowercased().hasSuffix(".zim") ? "whole:" + name.lowercased() :
                  "split:" + String(name.dropLast(2)).lowercased()
                cohorts[key, default: []].append(name)
              }
            }
            for cohort in cohorts.values {
              let names = cohort.sorted { $0.caseInsensitiveCompare($1) == .orderedAscending }
              let path = (components + [names[0]]).joined(separator: "/")
              do {
                if !(names.count == 1 && names[0].lowercased().hasSuffix(".zim")) {
                  let base = String(names[0].dropLast(2))
                  guard names.count <= 676 else { throw MobileFailure("provider_incomplete_split_archive") }
                  for (index, name) in names.enumerated() {
                    let suffix = String(UnicodeScalar(97 + index / 26)!) + String(UnicodeScalar(97 + index % 26)!)
                    guard name.caseInsensitiveCompare(base + suffix) == .orderedSame else {
                      throw MobileFailure("provider_incomplete_split_archive")
                    }
                  }
                }
                // Validate every part once, while the directory descriptor is
                // open, and carry the admitted identities into publishing so a
                // large selection is never opened and re-validated twice.
                var identities: [PartIdentity] = []
                var uuid: Data?
                for (index, name) in names.enumerated() {
                  try cancellation.check()
                  let fd = openat(directoryFD, name, O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK)
                  guard fd >= 0 else { throw posixFailure("open_provider_file") }
                  defer { close(fd) }
                  var info = stat()
                  guard fstat(fd, &info) == 0, (info.st_mode & S_IFMT) == S_IFREG else {
                    throw MobileFailure("provider_not_seekable_use_copy_import")
                  }
                  guard lseek(fd, 0, SEEK_CUR) >= 0 else {
                    throw MobileFailure("provider_not_seekable_use_copy_import")
                  }
                  let checked = try self.identity(fd, source: directory.appendingPathComponent(name),
                                                  expected: nil, stale: false, cancellation: cancellation)
                  identities.append(checked.identity)
                  if index == 0 { uuid = try self.archiveUUID(fd) }
                }
                guard let archiveUUID = uuid else { throw MobileFailure("provider_archive_parts_changed") }
                let record = Record(bookmark: bookmark, names: names, folder: true,
                  directory: components, identities: nil, archiveUUID: nil)
                records.append((path, record, AdmittedParts(identities: identities, archiveUUID: archiveUUID)))
              } catch { failure(path, error) }
            }
          }
          try visit(root, components: [])
        }
        for (path, record, admitted) in records {
          try cancellation.check()
          do {
            result.selections.append(try publish(url, record: record, admitted: admitted,
              persistent: persistent, stored: &added, cancellation: cancellation))
          } catch { failure(path, error) }
        }
      }
      if persistent, !added.isEmpty {
        locked {
          var stored = persistedLocked()
          for (reference, data) in added { stored[reference] = data }
          defaults.set(stored, forKey: key)
        }
      }
      return result
    } catch {
      // No caller received these capabilities; cancel without abandoning
      // saved references or ever touching the selected originals.
      for selection in result.selections { try forget(selection.reference) }
      throw error
    }
  }
  func selectIncoming(_ url: URL, persistent: Bool, cancellation: WorkCancellation) throws -> ExternalSelection {
    guard url.isFileURL else { throw MobileFailure("invalid_provider_url") }
    return try IncomingOriginalAuthority.withOriginal(url, required: persistent) { original in
      try select(original, folder: false, persistent: persistent, cancellation: cancellation)
    }
  }
  func acquire(_ reference: String, cancellation: WorkCancellation) throws -> ExternalAcquisition {
    let (record, selected) = try locked {
      (try recordLocked(reference), selectedURLs[reference])
    }
    var stale = false
    let url: URL
    if let selected { url = selected }
    else {
      (url, stale) = try resolveBookmark(record.bookmark)
    }
    guard url.isFileURL else { throw MobileFailure("provider_reference_revoked") }
    // A legacy bookmark has no admitted part identities against which to check
    // a stale resolution. Preserve it, but require explicit reselection.
    guard !stale || (record.identities != nil && record.archiveUUID != nil) else {
      throw MobileFailure("provider_reference_reselect_required")
    }
    let scoped = url.startAccessingSecurityScopedResource()
    var files: [ExternalDescriptor] = []
    var identities: [PartIdentity] = []
    var uuid: Data?
    do {
      try coordinatedRead(url, cancellation: cancellation) { readable in
        var directoryFD: Int32 = -1
        var sourceDirectory = readable
        if record.folder {
          if record.directory == nil {
            guard try self.names(readable, folder: true) == record.names else {
              throw MobileFailure("provider_archive_parts_changed")
            }
          }
          let root = open(readable.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
          guard root >= 0 else { throw posixFailure("open_provider_directory") }
          defer { close(root) }
          let components = record.directory ?? []
          for component in components { try safeComponent(component) }
          directoryFD = try openDirectory(root, components: components)
          sourceDirectory = components.reduce(readable) { $0.appendingPathComponent($1, isDirectory: true) }
        }
        defer { if directoryFD >= 0 { close(directoryFD) } }
        for (index, name) in record.names.enumerated() {
          try cancellation.check()
          try safeComponent(name)
          let source = record.folder ? sourceDirectory.appendingPathComponent(name) : readable
          let fd: Int32
          if record.folder {
            fd = openat(directoryFD, name, O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK)
            guard fd >= 0 else { throw posixFailure("open_provider_file") }
            var info = stat()
            guard fstat(fd, &info) == 0, (info.st_mode & S_IFMT) == S_IFREG else {
              close(fd); throw MobileFailure("provider_not_seekable_use_copy_import")
            }
          } else { fd = try sourceFD(source) }
          guard lseek(fd, 0, SEEK_CUR) >= 0 else {
            close(fd); throw MobileFailure("provider_not_seekable_use_copy_import")
          }
          files.append(ExternalDescriptor(name: name, fd: fd))
          let expected = record.identities.flatMap { index < $0.count ? $0[index] : nil }
          let checked = try self.identity(fd, source: source, expected: expected, stale: stale,
            cancellation: cancellation)
          if let expected {
            guard expected.size == checked.identity.size,
                  expected.resource == nil || expected.resource == checked.identity.resource,
                  !stale || checked.digestSatisfied else {
              throw MobileFailure("provider_archive_parts_changed")
            }
          }
          identities.append(expected ?? checked.identity)
          if index == 0 { uuid = try self.archiveUUID(fd) }
        }
      }
      guard record.archiveUUID == nil || record.archiveUUID == uuid,
            record.identities == nil || record.identities?.count == files.count else {
        throw MobileFailure("provider_archive_parts_changed")
      }
      let lease = newLeaseID().uuidString
      // Only replace the persisted capability after every admitted part and the
      // archive UUID have been validated while its security scope is retained.
      let bookmark = stale ? try url.bookmarkData(options: .minimalBookmark,
        includingResourceValuesForKeys: nil, relativeTo: nil) : record.bookmark
      let refreshed = Record(bookmark: bookmark, names: record.names, folder: record.folder,
                             directory: record.directory, identities: identities, archiveUUID: uuid)
      let encoded = (stale || record.identities == nil || record.archiveUUID == nil)
        ? try JSONEncoder().encode(refreshed) : nil
      try locked {
        guard leases[lease] == nil else { throw MobileFailure("provider_lease_conflict") }
        var stored = persistedLocked()
        guard transient[reference] != nil || stored[reference] != nil else {
          throw MobileFailure("provider_reference_revoked")
        }
        if let encoded, stored[reference] != nil {
          stored[reference] = encoded
          defaults.set(stored, forKey: key)
        }
        transient[reference] = refreshed
        leases[lease] = Lease(reference: reference, url: url, scoped: scoped, files: files)
      }
      return ExternalAcquisition(lease: lease, files: files)
    } catch {
      files.forEach { close($0.fd) }
      if scoped { url.stopAccessingSecurityScopedResource() }
      throw error
    }
  }
  func release(_ lease: String) {
    let released = locked { leases.removeValue(forKey: lease) }
    _ = released
  }
  func forget(_ reference: String) throws {
    try valid(reference)
    locked {
      transient.removeValue(forKey: reference); selectedURLs.removeValue(forKey: reference)
      var stored = persistedLocked()
      if stored.removeValue(forKey: reference) != nil { defaults.set(stored, forKey: key) }
    }
    // Existing leases own their scope independently until the last archive drops.
  }
}
