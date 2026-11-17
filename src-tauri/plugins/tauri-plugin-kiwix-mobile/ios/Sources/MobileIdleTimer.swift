import Foundation
import UIKit

// UIKit's idle timer is process-wide, so independent foreground activities must
// restore its original value only after the last assertion has ended.
final class MobileIdleTimerLease {
  private static var owners: Set<UUID> = []
  private static var previousValue: Bool?
  private let owner = UUID()

  init() {
    precondition(Thread.isMainThread)
    if Self.owners.isEmpty {
      Self.previousValue = UIApplication.shared.isIdleTimerDisabled
    }
    Self.owners.insert(owner)
    UIApplication.shared.isIdleTimerDisabled = true
  }

  func release() {
    precondition(Thread.isMainThread)
    Self.release(owner)
  }

  deinit {
    // Teardown can arrive off-main. Capture only the identity, never self.
    let owner = owner
    if Thread.isMainThread {
      Self.release(owner)
    } else {
      DispatchQueue.main.async { Self.release(owner) }
    }
  }

  private static func release(_ owner: UUID) {
    guard owners.remove(owner) != nil, owners.isEmpty else { return }
    let previous = previousValue
    previousValue = nil
    if let previous = previous { UIApplication.shared.isIdleTimerDisabled = previous }
  }
}
