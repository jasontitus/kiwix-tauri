// SPDX-License-Identifier: GPL-3.0-or-later
import Darwin
import XCTest
@testable import KiwixMobile

final class RouteIPAddressTests: XCTestCase {
  private let ipv4 = sa_family_t(AF_INET)
  private let ipv6 = sa_family_t(AF_INET6)

  private func decode(_ record: [UInt8], family: sa_family_t, netmask: Bool) -> RouteIPAddress? {
    record.withUnsafeBytes { bytes in
      RouteIPAddress(bytes.baseAddress!, family: family, netmask: netmask)
    }
  }

  func testCompactIPv4NetmaskIgnoresBytesAfterItsDeclaredLength() {
    let mask: [UInt8] = [7, sa_family_t(AF_UNSPEC), 0, 0, 255, 255, 252]
    let expected = RouteIPAddress.v4(0xfffffc00)
    XCTAssertEqual(decode(mask, family: ipv4, netmask: true), expected)
    XCTAssertEqual(decode(mask + [255], family: ipv4, netmask: true), expected)
    let peer = RouteIPAddress.v4(0xc0a803ff)
    let local = RouteIPAddress.v4(0xc0a80101)
    XCTAssertTrue(peer.isOnLink(local: local, mask: expected))
    XCTAssertFalse(RouteIPAddress.v4(0xc0a80401).isOnLink(local: local, mask: expected))
  }

  func testIPv6MasksFillBothWordsWithTrailingZeros() {
    let prefix: [UInt8] = [17, ipv6, 0, 0, 0, 0, 0, 0]
    let mask = decode(prefix + Array(repeating: 255, count: 8) + [128], family: ipv6, netmask: true)
    let expected = RouteIPAddress.v6(UInt64.max, 0x8000000000000000)
    XCTAssertEqual(mask, expected)
    let local = RouteIPAddress.v6(0x20010db800000000, 0x8000000000000001)
    XCTAssertTrue(RouteIPAddress.v6(0x20010db800000000, 0x80000000000000ff).isOnLink(local: local, mask: expected))
    XCTAssertFalse(RouteIPAddress.v6(0x20010db800000000, 0x7fffffffffffffff).isOnLink(local: local, mask: expected))
    XCTAssertFalse(local.isOnLink(local: local, mask: .v6(0, 0)))
    XCTAssertFalse(local.isOnLink(local: local, mask: .v4(UInt32.max)))
  }

  func testShortAndMismatchedRecordsAreRejected() {
    XCTAssertNil(decode([0], family: ipv4, netmask: true))
    XCTAssertNil(decode([1], family: ipv4, netmask: true))
    XCTAssertNil(decode([4, ipv4, 0, 0], family: ipv4, netmask: true))
    XCTAssertNil(decode([17], family: ipv4, netmask: true))
    XCTAssertNil(decode([5, ipv6, 0, 0, 255], family: ipv4, netmask: true))
    XCTAssertNil(decode([7, ipv4, 0, 0, 192, 168, 1], family: ipv4, netmask: false))
    let complete: [UInt8] = [16, ipv4, 0, 0, 192, 168, 1, 7] + Array(repeating: 0, count: 8)
    XCTAssertEqual(decode(complete, family: ipv4, netmask: false), .v4(0xc0a80107))
  }

  func testByValueAddressesValidateTheirTagAndWidth() {
    XCTAssertEqual(RouteIPAddress(version: 4, high: 0, low: 0x01020304), .v4(0x01020304))
    XCTAssertEqual(RouteIPAddress(version: 6, high: 0x20010db800000000, low: 1), .v6(0x20010db800000000, 1))
    XCTAssertNil(RouteIPAddress(version: 4, high: 1, low: 0))
    XCTAssertNil(RouteIPAddress(version: 4, high: 0, low: 1 << 32))
    XCTAssertNil(RouteIPAddress(version: UInt8.max, high: 0, low: 0))
    XCTAssertEqual(kiwixMobileIsLocalPeerAddress(0, 4, 0, 0x01020304), -1)
  }
}
