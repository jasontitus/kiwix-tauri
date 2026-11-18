import Darwin
import Foundation
import Tauri
import Network

private let localBindingLock = NSLock()
private var wifiInterfaceIndex: UInt32 = 0
private var lastWiFiInterfaceIndex: UInt32 = 0
private var wiredInterfaceIndex: UInt32 = 0
private var wifiInterfaceGeneration: UInt64 = 0
private var wiredInterfaceGeneration: UInt64 = 0

/// No browser reachability guesses: unknown paths deny acquisition until Network.framework reports.
final class MobileNetworkPolicy {
  private let queue = DispatchQueue(label: "org.kiwix.mobile.network-policy")
  private let lock = NSLock()
  private let internet = NWPathMonitor()
  private let wifi = NWPathMonitor(requiredInterfaceType: .wifi)
  private let wired = NWPathMonitor(requiredInterfaceType: .wiredEthernet)
  private var known = false
  private var connected = false
  private var metered = true
  private var cellular = false
  private var wifiAvailable = false
  private var wiredAvailable = false
  private var generation = 0
  private let changed: (JSObject) -> Void

  init(changed: @escaping (JSObject) -> Void) {
    self.changed = changed
    internet.pathUpdateHandler = { [weak self] path in
      self?.update {
        guard let self = self else { return }
        self.known = true; self.connected = path.status == .satisfied
        self.cellular = path.usesInterfaceType(.cellular)
        self.metered = path.isExpensive || path.isConstrained || self.cellular
      }
    }
    wifi.pathUpdateHandler = { [weak self] path in
      localBindingLock.lock()
      wifiInterfaceGeneration &+= 1
      let index = UInt32(path.availableInterfaces.first(where: { $0.type == .wifi })?.index ?? 0)
      if index != 0 { lastWiFiInterfaceIndex = index }
      // A local-only hotspot has no internet, so Network.framework can mark this
      // path unsatisfied while the Wi-Fi interface is up. Keep the last known
      // Wi-Fi index; nearbyInterface() still validates IFF_UP/RUNNING and IPv4.
      wifiInterfaceIndex = path.status == .satisfied ? index : 0
      localBindingLock.unlock()
      self?.update { self?.wifiAvailable = path.status == .satisfied }
    }
    wired.pathUpdateHandler = { [weak self] path in
      localBindingLock.lock()
      wiredInterfaceGeneration &+= 1
      wiredInterfaceIndex = path.status == .satisfied
        ? UInt32(path.availableInterfaces.first(where: { $0.type == .wiredEthernet })?.index ?? 0) : 0
      localBindingLock.unlock()
      self?.update { self?.wiredAvailable = path.status == .satisfied }
    }
    internet.start(queue: queue); wifi.start(queue: queue); wired.start(queue: queue)
  }
  deinit { internet.cancel(); wifi.cancel(); wired.cancel() }
  private func snapshotLocked() -> JSObject {
    ["known": known, "connected": connected, "metered": metered, "cellular": cellular,
     "localAvailable": wifiAvailable || wiredAvailable, "generation": generation]
  }
  func snapshot() -> JSObject { lock.lock(); defer { lock.unlock() }; return snapshotLocked() }
  private func update(_ body: () -> Void) {
    lock.lock(); body(); generation += 1; let state = snapshotLocked(); lock.unlock()
    changed(state)
  }
}

private let backupAttribute = "com.apple.metadata:com_apple_backup_excludeItem"
private let archiveAdmissionAttribute = "org.kiwix.archive-publication-v1"
private let archiveAdmission = Data([1])
// Only native backup/admission attribute transactions share this lock. A scan
// cannot exclude an inode after a concurrent publisher has admitted/included it.
private let backupOperationLock = NSLock()

private func descriptorAttribute(_ file: Int32, _ name: String) throws -> Data? {
  let count = fgetxattr(file, name, nil, 0, 0, 0)
  if count < 0 {
    if errno == ENOATTR { return nil }
    throw posixFailure("read_backup_attribute")
  }
  guard count <= 4096 else { throw MobileFailure("invalid_backup_attribute") }
  var value = Data(count: count)
  let read = value.withUnsafeMutableBytes { fgetxattr(file, name, $0.baseAddress, count, 0, 0) }
  guard read == count else { throw MobileFailure("backup_attribute_changed") }
  return value
}

private func setDescriptorAttribute(_ file: Int32, _ name: String, _ value: Data?) throws {
  // Rewriting identical bytes advances inode ctime and invalidates archive-derived indexes.
  // An unreadable or oversized old value still goes through the normal repair and verification.
  let alreadyApplied = (try? (descriptorAttribute(file, name) == value)) ?? false
  if !alreadyApplied {
    if let value = value {
      let result = value.withUnsafeBytes { fsetxattr(file, name, $0.baseAddress, value.count, 0, 0) }
      guard result == 0 else { throw posixFailure("write_backup_attribute") }
    } else if fremovexattr(file, name, 0) != 0 && errno != ENOATTR {
      throw posixFailure("remove_backup_attribute")
    }
  }
  guard try descriptorAttribute(file, name) == value else { throw MobileFailure("backup_policy_not_applied") }
}

/// Ask this iOS Foundation for the backing value of its supported NSURL key once.
/// No macOS/legacy MobileBackup encoding is assumed. Only a disposable EMPTY inode
/// in a freshly-created private temporary directory is ever mutated through NSURL.
/// Archive payloads are always changed with fsetxattr/fremovexattr on their retained fd.
private let nativeBackupExclusion: Result<Data, Error> = Result {
  let temporary = FileManager.default.temporaryDirectory.resolvingSymlinksInPath()
  var template = Array(temporary.appendingPathComponent("kiwix-backup-XXXXXX").path.utf8CString)
  let made = template.withUnsafeMutableBufferPointer { mkdtemp($0.baseAddress!) != nil }
  guard made else { throw posixFailure("create_backup_probe") }
  let path = String(cString: template)
  let directory = open(path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
  guard directory >= 0 else { rmdir(path); throw posixFailure("open_backup_probe") }
  defer { close(directory); rmdir(path) }
  let file = openat(directory, "attribute", O_RDWR | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, S_IRUSR | S_IWUSR)
  guard file >= 0 else { throw posixFailure("create_backup_probe_file") }
  defer { close(file); unlinkat(directory, "attribute", 0) }
  var identity = stat()
  guard fstat(file, &identity) == 0, identity.st_size == 0 else { throw MobileFailure("invalid_backup_probe") }
  func unchanged() throws {
    var current = stat()
    guard fstatat(directory, "attribute", &current, AT_SYMLINK_NOFOLLOW) == 0,
          current.st_dev == identity.st_dev, current.st_ino == identity.st_ino,
          (current.st_mode & S_IFMT) == S_IFREG, current.st_nlink == 1, current.st_size == 0
    else { throw MobileFailure("backup_probe_changed") }
  }
  let probe = URL(fileURLWithPath: path).appendingPathComponent("attribute") as NSURL
  guard let reference = probe.fileReferenceURL() else { throw MobileFailure("unsupported_backup_probe") }
  var url = reference as URL
  try unchanged()
  guard try descriptorAttribute(file, backupAttribute) == nil else { throw MobileFailure("invalid_backup_probe") }
  var values = URLResourceValues(); values.isExcludedFromBackup = true
  try url.setResourceValues(values)
  try unchanged()
  guard let excluded = try descriptorAttribute(file, backupAttribute), !excluded.isEmpty,
        try url.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup == true
  else { throw MobileFailure("unsupported_backup_attribute") }
  values.isExcludedFromBackup = false
  try url.setResourceValues(values)
  try unchanged()
  guard try descriptorAttribute(file, backupAttribute) == nil,
        try url.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup == false
  else { throw MobileFailure("unsupported_backup_attribute") }
  return excluded
}

/// Validate the borrowed file against an app-owned nofollow directory walk BEFORE
/// mutation. A later pathname replacement cannot redirect the descriptor operation.
private func withOwnedBackupFile<T>(file: Int32, path: String, archive: Bool, _ body: () throws -> T) throws -> T {
  guard file >= 0, path.hasPrefix("/"), !path.contains("\0") else { throw MobileFailure("invalid_backup_path") }
  let url = URL(fileURLWithPath: path)
  guard !url.pathComponents.contains(".."), !url.pathComponents.contains(".") else { throw MobileFailure("invalid_backup_path") }
  var retained = stat()
  guard fstat(file, &retained) == 0, (retained.st_mode & S_IFMT) == S_IFREG,
        retained.st_nlink == 1, !archive || isArchiveName(url.lastPathComponent)
  else { throw MobileFailure("invalid_backup_target") }
  let fm = FileManager.default
  let roots = fm.urls(for: .documentDirectory, in: .userDomainMask)
    + fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)
  for original in roots {
    let root = original.resolvingSymlinksInPath()
    let components = url.standardizedFileURL.pathComponents
    guard let prefix = [original.standardizedFileURL.pathComponents, root.standardizedFileURL.pathComponents].first(where: {
      components.count > $0.count && Array(components.prefix($0.count)) == $0
    }) else { continue }
    var parent = open(root.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
    guard parent >= 0 else { throw MobileFailure("backup_root_unavailable") }
    defer { close(parent) }
    for component in components.dropFirst(prefix.count).dropLast() {
      try safeComponent(component)
      let next = openat(parent, component, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
      guard next >= 0 else { throw MobileFailure("unsafe_backup_parent") }
      close(parent); parent = next
    }
    let leaf = url.lastPathComponent
    try safeComponent(leaf)
    func unchanged() throws {
      var named = stat()
      var ambient = stat()
      var current = stat()
      guard fstatat(parent, leaf, &named, AT_SYMLINK_NOFOLLOW) == 0,
            lstat(path, &ambient) == 0, fstat(file, &current) == 0,
            (named.st_mode & S_IFMT) == S_IFREG, named.st_nlink == 1,
            named.st_dev == retained.st_dev, named.st_ino == retained.st_ino,
            (ambient.st_mode & S_IFMT) == S_IFREG, ambient.st_nlink == 1,
            ambient.st_dev == retained.st_dev, ambient.st_ino == retained.st_ino,
            (current.st_mode & S_IFMT) == S_IFREG, current.st_nlink == 1,
            current.st_dev == retained.st_dev, current.st_ino == retained.st_ino
      else { throw MobileFailure("backup_target_changed") }
    }
    try unchanged()
    let result = try body()
    try unchanged()
    return result
  }
  throw MobileFailure("outside_app_storage")
}

/// `admit` is native publication authority, never a browser-supplied path API.
func applyOwnedBackupPolicy(file: Int32, path: String, include: Bool, admit: Bool = false, onlyUnadmitted: Bool = false) throws {
  backupOperationLock.lock(); defer { backupOperationLock.unlock() }
  // This no-mutation exit also handles a scanned temporary name that the
  // publisher has already renamed: the retained inode now carries admission.
  if onlyUnadmitted, try descriptorAttribute(file, archiveAdmissionAttribute) == archiveAdmission { return }
  try withOwnedBackupFile(file: file, path: path, archive: include || admit) {
    if admit {
      try setDescriptorAttribute(file, archiveAdmissionAttribute, archiveAdmission)
      // Publication/admission was already verified by the capability owner.
      // Persist that evidence independently before backup mutation can fail:
      // recovery must still find this admitted inode after such a failure.
      guard fsync(file) == 0 else { throw posixFailure("sync_archive_admission") }
    }
    let excluded = try nativeBackupExclusion.get()
    if include {
      guard try descriptorAttribute(file, archiveAdmissionAttribute) == archiveAdmission else { throw MobileFailure("archive_not_admitted") }
    }
    try setDescriptorAttribute(file, backupAttribute, include ? nil : excluded)
    // Foundation is read-only on the payload. Confirm this OS interprets the
    // discovered descriptor attribute as its supported resource key; surrounding
    // identity checks reject a replaced pathname without touching its new inode.
    let applied = try URL(fileURLWithPath: path).resourceValues(forKeys: [.isExcludedFromBackupKey])
    guard applied.isExcludedFromBackup == !include else { throw MobileFailure("backup_policy_not_applied") }
    guard fsync(file) == 0 else { throw posixFailure("sync_backup_policy") }
  }
}

/// Both Rust and native imports borrow the descriptor they exclusively created.
@_cdecl("kiwix_mobile_archive_backup")
public func kiwixMobileArchiveBackup(_ file: Int32, _ path: UnsafePointer<CChar>?, _ include: Int32, _ admit: Int32) -> Int32 {
  guard let path = path, include == 0 || include == 1, admit == 0 || admit == 1 else { return -1 }
  do {
    try applyOwnedBackupPolicy(file: file, path: String(cString: path), include: include == 1, admit: admit == 1)
    return 0
  } catch { return -1 }
}

/// A stale negative scan is never authority to undo a newer publication.
@_cdecl("kiwix_mobile_exclude_unadmitted_payload")
public func kiwixMobileExcludeUnadmittedPayload(_ file: Int32, _ path: UnsafePointer<CChar>?) -> Int32 {
  guard file >= 0, let path = path else { return -1 }
  do {
    try applyOwnedBackupPolicy(file: file, path: String(cString: path), include: false, onlyUnadmitted: true)
    return 0
  } catch { return -1 }
}

@_cdecl("kiwix_mobile_archive_admitted")
public func kiwixMobileArchiveAdmitted(_ file: Int32, _ path: UnsafePointer<CChar>?) -> Int32 {
  guard file >= 0, let path = path else { return -1 }
  do {
    // Unmarked metadata/cache files confer no authority. Reading only their
    // descriptor avoids treating ordinary unrelated temporary-file churn as a
    // failed archive reconciliation; every positive admission is still checked.
    guard try descriptorAttribute(file, archiveAdmissionAttribute) == archiveAdmission else { return 0 }
    return try withOwnedBackupFile(file: file, path: String(cString: path), archive: true) { 1 }
  } catch { return -1 }
}

/// Bind a retained Rust UDP or TCP descriptor to a confirmed physical LAN interface.
/// A disappearing interface makes the socket unusable; it never falls back to cellular.
@_cdecl("kiwix_mobile_bind_local_socket")
public func kiwixMobileBindLocalSocket(_ fd: Int32) -> Int64 {
  localBindingLock.lock(); defer { localBindingLock.unlock() }
  var index = wiredInterfaceIndex != 0 ? wiredInterfaceIndex : wifiInterfaceIndex
  guard fd >= 0, index != 0 else { return -1 }
  var kind: Int32 = 0
  var kindLength = socklen_t(MemoryLayout<Int32>.size)
  guard getsockopt(fd, SOL_SOCKET, SO_TYPE, &kind, &kindLength) == 0,
        kind == SOCK_DGRAM || kind == SOCK_STREAM else { return -1 }
  var address = sockaddr_storage()
  var length = socklen_t(MemoryLayout<sockaddr_storage>.size)
  let status = withUnsafeMutablePointer(to: &address) { pointer in
    pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { getsockname(fd, $0, &length) }
  }
  guard status == 0 else { return -1 }
  let level: Int32
  let option: Int32
  switch Int32(address.ss_family) {
  case AF_INET:
    level = IPPROTO_IP; option = IP_BOUND_IF
  case AF_INET6:
    // Darwin stores IPV6_BOUND_IF on the socket's shared inpcb; IPv4-mapped
    // traffic from a dual-stack socket retains this same interface constraint.
    level = IPPROTO_IPV6; option = IPV6_BOUND_IF
  default:
    return -1
  }
  guard setsockopt(fd, level, option, &index, socklen_t(MemoryLayout<UInt32>.size)) == 0 else { return -1 }
  var applied: UInt32 = 0
  var appliedLength = socklen_t(MemoryLayout<UInt32>.size)
  guard getsockopt(fd, level, option, &applied, &appliedLength) == 0, applied == index else { return -1 }
  return Int64(applied)
}

private func sameOnLinkPrefix<T>(_ peer: T, _ local: T, _ mask: T) -> Bool {
  withUnsafeBytes(of: peer) { peerBytes in
    withUnsafeBytes(of: local) { localBytes in
      withUnsafeBytes(of: mask) { maskBytes in
        var nondefault = false
        for offset in peerBytes.indices {
          let byte = maskBytes[offset]
          nondefault = nondefault || byte != 0
          if (peerBytes[offset] & byte) != (localBytes[offset] & byte) { return false }
        }
        return nondefault
      }
    }
  }
}

/// Classify only the actual bound route receipt. A public IPv6 LAN prefix is local;
/// a default route or another simultaneously available physical interface is not proof.
@_cdecl("kiwix_mobile_is_local_peer")
public func kiwixMobileIsLocalPeer(_ route: UInt32, _ numeric: UnsafePointer<CChar>?) -> Int32 {
  guard let numeric = numeric else { return -1 }
  localBindingLock.lock(); defer { localBindingLock.unlock() }
  guard route != 0, route == wifiInterfaceIndex || route == wiredInterfaceIndex else { return -1 }
  var ipv4 = in_addr()
  var ipv6 = in6_addr()
  let family: Int32
  if inet_pton(AF_INET, numeric, &ipv4) == 1 { family = AF_INET }
  else if inet_pton(AF_INET6, numeric, &ipv6) == 1 { family = AF_INET6 }
  else { return -1 }
  var first: UnsafeMutablePointer<ifaddrs>?
  guard getifaddrs(&first) == 0, let head = first else { return -1 }
  defer { freeifaddrs(head) }
  var current: UnsafeMutablePointer<ifaddrs>? = head
  while let pointer = current {
    let interface = pointer.pointee
    current = interface.ifa_next
    guard if_nametoindex(interface.ifa_name) == route,
          interface.ifa_flags & UInt32(IFF_UP) != 0,
          let address = interface.ifa_addr, let mask = interface.ifa_netmask,
          Int32(address.pointee.sa_family) == family else { continue }
    if family == AF_INET {
      let local = UnsafeRawPointer(address).assumingMemoryBound(to: sockaddr_in.self).pointee.sin_addr
      let subnet = UnsafeRawPointer(mask).assumingMemoryBound(to: sockaddr_in.self).pointee.sin_addr
      if sameOnLinkPrefix(ipv4, local, subnet) { return 1 }
    } else {
      let local = UnsafeRawPointer(address).assumingMemoryBound(to: sockaddr_in6.self).pointee.sin6_addr
      let subnet = UnsafeRawPointer(mask).assumingMemoryBound(to: sockaddr_in6.self).pointee.sin6_addr
      if sameOnLinkPrefix(ipv6, local, subnet) { return 1 }
    }
  }
  return 0
}

struct NearbyRouteBindArgs: Decodable { let fd: Int32; let address: String; let route: UInt64? }
struct NearbyRouteStateArgs: Decodable { let route: UInt64; let address: String }

struct NearbyIPv4Prefix: Equatable {
  let address: UInt32
  let mask: UInt32
}
struct NearbyInterfaceSnapshot: Equatable {
  let index: UInt32
  let generation: UInt64
  let prefixes: [NearbyIPv4Prefix]
}

// Main-queue-owned opaque receipts are not bare, recyclable interface indexes.
private var nearbyRouteSerial: UInt64 = 0
func nearbyNextRouteReceipt() -> UInt64 {
  precondition(Thread.isMainThread)
  nearbyRouteSerial += 1
  return nearbyRouteSerial
}

private func nearbyInterface(_ index: UInt32, generation: UInt64) -> NearbyInterfaceSnapshot? {
  guard index != 0 else { return nil }
  var first: UnsafeMutablePointer<ifaddrs>?
  guard getifaddrs(&first) == 0, let head = first else { return nil }
  defer { freeifaddrs(head) }
  var prefixes: [NearbyIPv4Prefix] = []
  var current: UnsafeMutablePointer<ifaddrs>? = head
  while let pointer = current {
    let item = pointer.pointee; current = item.ifa_next
    guard if_nametoindex(item.ifa_name) == index,
          item.ifa_flags & UInt32(IFF_UP) != 0,
          item.ifa_flags & UInt32(IFF_RUNNING) != 0,
          item.ifa_flags & UInt32(IFF_LOOPBACK) == 0,
          item.ifa_flags & UInt32(IFF_POINTOPOINT) == 0,
          let address = item.ifa_addr, let mask = item.ifa_netmask,
          Int32(address.pointee.sa_family) == AF_INET else { continue }
    let local = UnsafeRawPointer(address).assumingMemoryBound(to: sockaddr_in.self).pointee.sin_addr.s_addr
    let subnet = UnsafeRawPointer(mask).assumingMemoryBound(to: sockaddr_in.self).pointee.sin_addr.s_addr
    if local != 0, subnet != 0 {
      prefixes.append(NearbyIPv4Prefix(address: local, mask: subnet))
    }
  }
  guard !prefixes.isEmpty else { return nil }
  prefixes.sort { $0.address == $1.address ? $0.mask < $1.mask : $0.address < $1.address }
  return NearbyInterfaceSnapshot(index: index, generation: generation, prefixes: prefixes)
}

func nearbyCurrentWiFiInterface() -> NearbyInterfaceSnapshot? {
  localBindingLock.lock(); defer { localBindingLock.unlock() }
  let index = wifiInterfaceIndex != 0 ? wifiInterfaceIndex : lastWiFiInterfaceIndex
  return nearbyInterface(index, generation: wifiInterfaceGeneration)
}
func nearbySnapshotIsLive(_ snapshot: NearbyInterfaceSnapshot) -> Bool {
  localBindingLock.lock(); defer { localBindingLock.unlock() }
  let wifiIndex = wifiInterfaceIndex != 0 ? wifiInterfaceIndex : lastWiFiInterfaceIndex
  if snapshot.index == wifiIndex {
    return nearbyInterface(wifiIndex, generation: wifiInterfaceGeneration) == snapshot
  }
  if snapshot.index == wiredInterfaceIndex {
    return nearbyInterface(wiredInterfaceIndex, generation: wiredInterfaceGeneration) == snapshot
  }
  return false
}

private func nearbyEndpointOnLink(_ raw: String, interface: NearbyInterfaceSnapshot) -> Bool {
  guard raw.utf8.count <= 15, nearbySnapshotIsLive(interface) else { return false }
  var peer = in_addr()
  guard inet_pton(AF_INET, raw, &peer) == 1 else { return false }
  let host = UInt32(bigEndian: peer.s_addr)
  // Only the reviewed descriptor's private/link-local usable IPv4 family.
  guard host >> 24 == 10 || host >> 20 == 0xac1 || host >> 16 == 0xc0a8 || host >> 16 == 0xa9fe else { return false }
  return interface.prefixes.contains { prefix in
    let mask = UInt32(bigEndian: prefix.mask)
    let local = UInt32(bigEndian: prefix.address)
    let network = local & mask
    let broadcast = network | ~mask
    return host != local && host != network && host != broadcast && (host & mask) == network
  }
}

func nearbyBindIPv4Socket(_ fd: Int32, address: String, interface: NearbyInterfaceSnapshot) -> Bool {
  guard fd >= 0, nearbyEndpointOnLink(address, interface: interface) else { return false }
  var kind: Int32 = 0
  var kindLength = socklen_t(MemoryLayout<Int32>.size)
  guard getsockopt(fd, SOL_SOCKET, SO_TYPE, &kind, &kindLength) == 0,
        kind == SOCK_STREAM || kind == SOCK_DGRAM else { return false }
  var socketAddress = sockaddr_storage()
  var length = socklen_t(MemoryLayout<sockaddr_storage>.size)
  let status = withUnsafeMutablePointer(to: &socketAddress) {
    $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { getsockname(fd, $0, &length) }
  }
  guard status == 0, Int32(socketAddress.ss_family) == AF_INET else { return false }
  var remote = sockaddr_storage()
  var remoteLength = socklen_t(MemoryLayout<sockaddr_storage>.size)
  let connected = withUnsafeMutablePointer(to: &remote) {
    $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { getpeername(fd, $0, &remoteLength) }
  }
  guard connected != 0, errno == ENOTCONN else { return false }
  var index = interface.index
  guard setsockopt(fd, IPPROTO_IP, IP_BOUND_IF, &index, socklen_t(MemoryLayout<UInt32>.size)) == 0 else { return false }
  var applied: UInt32 = 0
  var appliedLength = socklen_t(MemoryLayout<UInt32>.size)
  return getsockopt(fd, IPPROTO_IP, IP_BOUND_IF, &applied, &appliedLength) == 0
    && applied == index && nearbyEndpointOnLink(address, interface: interface)
}

final class NearbyRoutes {
  private var receipts: [UInt64: NearbyInterfaceSnapshot] = [:]
  func bind(_ args: NearbyRouteBindArgs, invoke: Invoke) {
    precondition(Thread.isMainThread)
    let interface: NearbyInterfaceSnapshot?
    if let route = args.route {
      interface = receipts[route]
    } else {
      localBindingLock.lock()
      let candidates = [
        nearbyInterface(wiredInterfaceIndex, generation: wiredInterfaceGeneration),
        nearbyInterface(wifiInterfaceIndex, generation: wifiInterfaceGeneration)
      ].compactMap { $0 }
      localBindingLock.unlock()
      interface = candidates.first { nearbyEndpointOnLink(args.address, interface: $0) }
    }
    guard let interface = interface, nearbyBindIPv4Socket(args.fd, address: args.address, interface: interface) else {
      invoke.reject("NEARBY_ROUTE_UNAVAILABLE_OR_ENDPOINT_NOT_ON_LINK"); return
    }
    if let route = args.route { invoke.resolve(["route": route]); return }
    if let existing = receipts.first(where: { $0.value == interface }) {
      invoke.resolve(["route": existing.key]); return
    }
    guard receipts.count < 128 else { invoke.reject("NEARBY_ROUTE_RECEIPTS_FULL"); return }
    let route = nearbyNextRouteReceipt(); receipts[route] = interface
    invoke.resolve(["route": route])
  }
  func state(_ args: NearbyRouteStateArgs, invoke: Invoke) {
    guard let interface = receipts[args.route] else { invoke.resolve(["available": false]); return }
    let available = nearbyEndpointOnLink(args.address, interface: interface)
    if !nearbySnapshotIsLive(interface) { receipts.removeValue(forKey: args.route) }
    invoke.resolve(["available": available])
  }
  func retire() { receipts.removeAll() }
}
