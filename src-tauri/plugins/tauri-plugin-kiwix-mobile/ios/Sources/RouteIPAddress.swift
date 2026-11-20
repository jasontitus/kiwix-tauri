// SPDX-License-Identifier: GPL-3.0-or-later
import Darwin

/// Network-order IP words independent of the layout of the native record.
enum RouteIPAddress: Equatable {
  case v4(UInt32)
  case v6(UInt64, UInt64)

  var family: sa_family_t {
    switch self {
    case .v4: return sa_family_t(AF_INET)
    case .v6: return sa_family_t(AF_INET6)
    }
  }

  init?(version: UInt8, high: UInt64, low: UInt64) {
    switch version {
    case 4:
      guard high == 0, let address = UInt32(exactly: low) else { return nil }
      self = .v4(address)
    case 6:
      self = .v6(high, low)
    default:
      return nil
    }
  }

  /// Borrow a record from Darwin's live getifaddrs list. Darwin omits trailing
  /// zeroes from some netmasks. Socket address records must be full sized.
  init?(_ record: UnsafeRawPointer, family: sa_family_t, netmask: Bool) {
    let offset: Int?
    let size: Int
    let width: Int
    switch Int32(family) {
    case AF_INET:
      offset = MemoryLayout<sockaddr_in>.offset(of: \.sin_addr)
      size = MemoryLayout<sockaddr_in>.size
      width = 4
    case AF_INET6:
      offset = MemoryLayout<sockaddr_in6>.offset(of: \.sin6_addr)
      size = MemoryLayout<sockaddr_in6>.size
      width = 16
    default:
      return nil
    }
    guard let offset = offset else { return nil }
    let length = Int(record.load(as: UInt8.self)) // sa_len is the first byte in Darwin.
    guard length > offset, length <= size, netmask || length == size else { return nil }
    let actualFamily = record.load(fromByteOffset: 1, as: UInt8.self)
    guard actualFamily == family || (netmask && actualFamily == sa_family_t(AF_UNSPEC)) else { return nil }
    let bytes = UnsafeRawBufferPointer(start: record + offset, count: min(width, length - offset))
    if family == sa_family_t(AF_INET) {
      self = .v4(Self.networkWord(bytes[...]))
    } else {
      self = .v6(Self.networkWord(bytes.prefix(8)), Self.networkWord(bytes.dropFirst(8)))
    }
  }

  private static func networkWord<T: FixedWidthInteger & UnsignedInteger>(_ bytes: UnsafeRawBufferPointer.SubSequence) -> T {
    var word: T = 0
    for (index, byte) in bytes.enumerated() {
      word |= T(byte) << (T.bitWidth - 8 * (index + 1))
    }
    return word
  }

  func isOnLink(local: Self, mask: Self) -> Bool {
    switch (self, local, mask) {
    case let (.v4(peer), .v4(local), .v4(mask)):
      return mask != 0 && ((peer ^ local) & mask) == 0
    case let (.v6(peerHigh, peerLow), .v6(localHigh, localLow), .v6(maskHigh, maskLow)):
      return (maskHigh != 0 || maskLow != 0)
        && ((peerHigh ^ localHigh) & maskHigh) == 0 && ((peerLow ^ localLow) & maskLow) == 0
    default:
      return false
    }
  }
}
