import Foundation
import UIKit
import Network
import NetworkExtension
import Tauri
import Darwin

struct NearbyJoinArgs: Decodable {
  let owner: String
  let reservationId: String
  let ssid: String
  let passphrase: String
  let security: String
}
struct NearbyJoinLeaseArgs: Decodable { let owner: String; let lease: String }
struct NearbyJoinBindArgs: Decodable { let owner: String; let lease: String; let fd: Int32; let address: String }

// Association/configuration ownership is deliberately separate from BLE and TLS.
// A successful reply proves an observed association, NEVER a pinned publisher.
final class NearbyJoin {
  private final class Attempt {
    let owner: String
    let lease = UUID().uuidString.lowercased()
    let idleTimer = MobileIdleTimerLease()
    let ssid: String
    weak var scene: UIScene?
    let currentScene: () -> UIScene?
    var invoke: Invoke?
    var cancelled = false
    var queryingConfigurations = true
    var applying = false
    var existed = true // Fail-safe until the OS's app-configuration list is received.
    var applied = false
    var cleaning = false
    var cleanupGeneration: UInt64 = 0
    var cleanupDeadline: DispatchWorkItem?
    var cleanupWaiters: [(String?) -> Void] = []
    var bssid: String?
    var interface: NearbyInterfaceSnapshot?
    var route: UInt64?
    var boundPrefixes: [NearbyIPv4Prefix] = []
    var timer: DispatchWorkItem?
    var monitor: NWPathMonitor?
    var checking = false
    var checkCount = 0
    var missSince: TimeInterval?
    var ready = false
    var checkGeneration: UInt64 = 0
    var lastVerified: TimeInterval = 0
    var poll: DispatchWorkItem?
    var pollDelay: TimeInterval = 1
    var activation: NSObjectProtocol?
    var checkTimeout: DispatchWorkItem?
    var queries: [UUID: (Invoke, DispatchWorkItem)] = [:]
    init(_ args: NearbyJoinArgs, scene: UIScene, currentScene: @escaping () -> UIScene?, invoke: Invoke) {
      owner = args.owner; ssid = args.ssid; self.scene = scene; self.currentScene = currentScene; self.invoke = invoke
    }
    var foreground: Bool {
      scene != nil && currentScene() === scene && scene?.activationState != .background && scene?.activationState != .unattached
        && UIApplication.shared.applicationState != .background
    }
  }
  private func joinLog(_ message: String) {
    NearbyDiagnosticLog.shared.append("[nearby] join \(message)\n")
  }
  private var current: Attempt?
  private let emit: (JSObject) -> Void
  private let currentScene: () -> UIScene?
  init(currentScene: @escaping () -> UIScene?, emit: @escaping (JSObject) -> Void) {
    self.currentScene = currentScene; self.emit = emit
  }

  func join(_ args: NearbyJoinArgs, scene: UIScene?, invoke: Invoke) {
    guard UUID(uuidString: args.owner)?.uuidString.lowercased() == args.owner,
          UUID(uuidString: args.reservationId)?.uuidString.lowercased() == args.reservationId,
          args.security == "wpa2", !args.ssid.isEmpty, args.ssid.utf8.count <= 32,
          !args.ssid.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
          (8...63).contains(args.passphrase.utf8.count),
          args.passphrase.utf8.allSatisfy({ (32...126).contains($0) }) else {
      invoke.reject("JOIN_INVALID_REQUEST"); return
    }
    guard current == nil else {
      invoke.reject(current?.cancelled == true ? "JOIN_PREVIOUS_OPERATION_RETIRING" : "JOIN_OWNER_BUSY"); return
    }
    guard let scene = scene, scene.activationState == .foregroundActive,
          UIApplication.shared.applicationState != .background else {
      invoke.reject("JOIN_FOREGROUND_REQUIRED"); return
    }
    joinLog("start ssid=\(args.ssid)")
    let attempt = Attempt(args, scene: scene, currentScene: currentScene, invoke: invoke)
    current = attempt
    // Never remove an earlier app or user configuration, even if it shares this SSID.
    // Keep the passphrase out of attempt state; the system configuration owns its copy.
    let configuration = NEHotspotConfiguration(ssid: args.ssid, passphrase: args.passphrase, isWEP: false)
    configuration.joinOnce = true
    let timeout = DispatchWorkItem { [weak self, weak attempt] in
      guard let self = self, let attempt = attempt, self.current === attempt else { return }
      self.finish(attempt, code: "JOIN_ASSOCIATION_UNAVAILABLE_OR_WIFI_INFO_DENIED", lost: attempt.ready)
    }
    attempt.timer = timeout; DispatchQueue.main.asyncAfter(deadline: .now() + 180, execute: timeout)
    NEHotspotConfigurationManager.shared.getConfiguredSSIDs { [self, attempt, configuration] names in
      DispatchQueue.main.async {
        guard self.current === attempt else { return }
        attempt.queryingConfigurations = false
        joinLog("configured names=\(names.count) foreground=\(attempt.foreground)")
        guard !attempt.cancelled, attempt.foreground else {
          self.finish(attempt, code: "JOIN_CANCELLED", lost: false); return
        }
        attempt.existed = names.contains(attempt.ssid)
        if attempt.existed {
          // Applying joinOnce over an earlier app configuration may replace it.
          // Reuse only an already matching association; never mutate its config.
          self.startAssociationMonitor(attempt)
          return
        }
        attempt.applying = true
        joinLog("apply")
        NEHotspotConfigurationManager.shared.apply(configuration) { [self, attempt] error in
          DispatchQueue.main.async {
            guard self.current === attempt else { return }
            attempt.applying = false
            joinLog("applied error=\(String(describing: error))")
            let failure = error as NSError?
            let already = failure?.domain == NEHotspotConfigurationErrorDomain
              && failure?.code == NEHotspotConfigurationError.alreadyAssociated.rawValue
            attempt.applied = error == nil
            if attempt.cancelled || !attempt.foreground {
              self.finish(attempt, code: "JOIN_CANCELLED", lost: false); return
            }
            guard error == nil || already else {
              self.finish(attempt, code: Self.errorCode(failure), lost: false); return
            }
            self.startAssociationMonitor(attempt)
          }
        }
      }
    }
  }

  private static func errorCode(_ error: NSError?) -> String {
    guard let error = error, error.domain == NEHotspotConfigurationErrorDomain else { return "JOIN_APPLY_FAILED" }
    switch error.code {
    case NEHotspotConfigurationError.userDenied.rawValue: return "JOIN_USER_DENIED"
    case NEHotspotConfigurationError.invalid.rawValue,
         NEHotspotConfigurationError.invalidSSID.rawValue,
         NEHotspotConfigurationError.invalidWPAPassphrase.rawValue: return "JOIN_INVALID_CONFIGURATION"
    case NEHotspotConfigurationError.applicationIsNotInForeground.rawValue: return "JOIN_FOREGROUND_REQUIRED"
    case NEHotspotConfigurationError.systemConfiguration.rawValue: return "JOIN_ENTITLEMENT_OR_SYSTEM_DENIED"
    default: return "JOIN_APPLY_FAILED"
    }
  }

  private func startAssociationMonitor(_ attempt: Attempt) {
    let monitor = NWPathMonitor(requiredInterfaceType: .wifi)
    attempt.monitor = monitor
    monitor.pathUpdateHandler = { [weak self, weak attempt] _ in
      DispatchQueue.main.async {
        guard let self = self, let attempt = attempt, self.current === attempt,
              !attempt.cancelled else { return }
        // Path updates are a reason to verify, not association proof or loss.
        self.checkAssociation(attempt)
      }
    }
    monitor.start(queue: DispatchQueue(label: "org.kiwix.mobile.nearby.join-path"))
    // Returning to the foreground after a permission sheet or interruption is
    // another reason to verify; a backgrounded scene already retires the lease.
    attempt.activation = NotificationCenter.default.addObserver(
      forName: UIScene.didActivateNotification, object: nil, queue: .main
    ) { [weak self, weak attempt] notification in
      guard let self = self, let attempt = attempt, self.current === attempt,
            !attempt.cancelled, notification.object as? UIScene === attempt.scene else { return }
      self.checkAssociation(attempt)
    }
    checkAssociation(attempt)
  }

  private func checkAssociation(_ attempt: Attempt) {
    guard current === attempt, !attempt.cancelled, !attempt.checking else { return }
    guard attempt.foreground else { finish(attempt, code: "JOIN_BACKGROUND", lost: attempt.ready); return }
    attempt.poll?.cancel(); attempt.poll = nil
    attempt.checkCount += 1
    attempt.checking = true; attempt.checkGeneration &+= 1
    let generation = attempt.checkGeneration
    // An app-configured current network can be verified without location
    // permission. fetchCurrent still requires the Wi-Fi information entitlement;
    // nil is never replaced by apply success or arbitrary interface availability.
    let checkTimeout = DispatchWorkItem { [weak self, weak attempt] in
      guard let self = self, let attempt = attempt, self.current === attempt,
            !attempt.cancelled, attempt.checking, attempt.checkGeneration == generation else { return }
      attempt.checking = false
      if attempt.ready {
        attempt.lastVerified = 0
        // A missed proof retries at full rate so the grace window below is
        // evaluated promptly instead of once per backed-off interval.
        attempt.pollDelay = 1
        let now = ProcessInfo.processInfo.systemUptime
        if attempt.missSince == nil { attempt.missSince = now }
        if now - (attempt.missSince ?? now) >= 45 {
          self.finish(attempt, code: "JOIN_ASSOCIATION_QUERY_TIMEOUT", lost: true); return
        }
        attempt.checkGeneration &+= 1
        self.schedulePoll(attempt)
      } else {
        self.finish(attempt, code: "JOIN_ASSOCIATION_QUERY_TIMEOUT", lost: false)
      }
    }
    attempt.checkTimeout = checkTimeout
    DispatchQueue.main.asyncAfter(deadline: .now() + 3, execute: checkTimeout)
    NEHotspotNetwork.fetchCurrent { [weak self, weak attempt] network in
      DispatchQueue.main.async {
        guard let self = self, let attempt = attempt, self.current === attempt,
              !attempt.cancelled, attempt.checkGeneration == generation else { return }
        attempt.checking = false
        attempt.checkTimeout?.cancel(); attempt.checkTimeout = nil
        guard attempt.foreground else { self.finish(attempt, code: "JOIN_BACKGROUND", lost: attempt.ready); return }
        if let failure = self.refreshAssociation(network, attempt: attempt) {
          if attempt.ready {
            // A nil/flaky Wi-Fi-info read after a verified association must not
            // retire a usable join. Keep a monotonic grace from the first
            // failed proof, reset by any genuine success, so prompt failures
            // cannot accumulate faster than the intended window.
            attempt.lastVerified = 0
            attempt.pollDelay = 1
            let now = ProcessInfo.processInfo.systemUptime
            if attempt.missSince == nil { attempt.missSince = now }
            if now - (attempt.missSince ?? now) >= 45 {
              self.finish(attempt, code: failure, lost: true); return
            }
          } else if attempt.existed || attempt.checkCount >= 15 {
            self.finish(attempt, code: attempt.existed && !attempt.ready
              ? "JOIN_EXISTING_CONFIGURATION_NOT_ASSOCIATED" : failure,
              lost: attempt.ready); return
          }
        } else if !attempt.ready {
          attempt.ready = true; attempt.timer?.cancel(); attempt.timer = nil
          let invoke = attempt.invoke; attempt.invoke = nil
          invoke?.resolve(["owner": attempt.owner, "lease": attempt.lease])
        }
        self.schedulePoll(attempt)
      }
    }
  }

  private func schedulePoll(_ attempt: Attempt) {
    // Before readiness this poll is the only association proof, so it stays at
    // 1 s (a lease that never associates fails after 15 checks). A ready lease
    // is additionally verified by path updates, scene activation and every
    // state/bind query, so its safety-net poll backs off to a low cadence
    // instead of querying Wi-Fi information every second for the whole lease.
    let delay = attempt.ready ? attempt.pollDelay : 1
    if attempt.ready { attempt.pollDelay = min(attempt.pollDelay * 2, 30) }
    let poll = DispatchWorkItem { [weak self, weak attempt] in
      guard let self = self, let attempt = attempt else { return }
      self.checkAssociation(attempt)
    }
    attempt.poll = poll
    DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: poll)
  }

  private static func usableBSSID(_ value: String) -> Bool {
    !value.isEmpty && value != "00:00:00:00:00:00" && value != "02:00:00:00:00:00"
  }

  private func refreshAssociation(_ network: NEHotspotNetwork?, attempt: Attempt) -> String? {
    if let network = network {
      guard network.ssid == attempt.ssid, Self.usableBSSID(network.bssid) else {
        return "JOIN_ASSOCIATION_UNAVAILABLE_OR_WIFI_INFO_DENIED"
      }
      if let expected = attempt.bssid, expected != network.bssid { return "JOIN_ASSOCIATION_CHANGED" }
    } else if !attempt.applied {
      return "JOIN_ASSOCIATION_UNAVAILABLE_OR_WIFI_INFO_DENIED"
    }
    // iOS may withhold Wi-Fi information after an app-owned apply. In both
    // cases the interface must remain live, preserve bound local addresses,
    // and pass the exact endpoint/on-link check when binding a socket.
    guard let interface = nearbyCurrentWiFiInterface() else { return "JOIN_ROUTE_LOST" }
    // A freshly verified SSID/BSSID may refresh DHCP/path metadata. Once a
    // socket is bound, its exact interface and selected local prefixes must
    // survive; unrelated address additions or path generations do not revoke it.
    if !attempt.boundPrefixes.isEmpty {
      guard let expected = attempt.interface, expected.index == interface.index,
            attempt.boundPrefixes.allSatisfy({ interface.prefixes.contains($0) }) else {
        return "JOIN_ROUTE_CHANGED"
      }
    }
    if let network = network { attempt.bssid = network.bssid }
    attempt.interface = interface
    attempt.lastVerified = ProcessInfo.processInfo.systemUptime
    attempt.missSince = nil
    return nil
  }

  // Binding gets a fresh exact association proof. Status can use the same
  // snapshot for at most three seconds, but any path/address change requires
  // fresh SSID/BSSID verification before it can report availability again.
  private func query(_ owner: String, lease: String, invoke: Invoke,
                     body: @escaping (Attempt) -> Void) {
    guard let attempt = current, attempt.owner == owner, attempt.lease == lease,
          !attempt.cancelled, attempt.ready, attempt.foreground else {
      invoke.reject("JOIN_LEASE_RETIRED"); return
    }
    guard attempt.queries.count < 8 else { invoke.reject("JOIN_QUERY_BUSY"); return }
    let id = UUID()
    let timeout = DispatchWorkItem { [weak self, weak attempt] in
      guard let self = self, let attempt = attempt, self.current === attempt,
            let pending = attempt.queries.removeValue(forKey: id) else { return }
      // A query is a read-only proof. The association monitor owns lease
      // retirement, so a slow Wi-Fi-info read must not cancel a valid join.
      // It must still invalidate the cached availability so the next state
      // request takes a fresh proof instead of trusting an older success.
      attempt.lastVerified = 0
      pending.0.reject("JOIN_ASSOCIATION_QUERY_TIMEOUT")
    }
    attempt.queries[id] = (invoke, timeout)
    DispatchQueue.main.asyncAfter(deadline: .now() + 5, execute: timeout)
    NEHotspotNetwork.fetchCurrent { [weak self, weak attempt] network in
      DispatchQueue.main.async {
        guard let self = self, let attempt = attempt, self.current === attempt,
              let pending = attempt.queries.removeValue(forKey: id) else { return }
        pending.1.cancel()
        guard !attempt.cancelled, attempt.foreground else {
          pending.0.reject("JOIN_BACKGROUND")
          self.finish(attempt, code: "JOIN_BACKGROUND", lost: true); return
        }
        if let failure = self.refreshAssociation(network, attempt: attempt) {
          attempt.lastVerified = 0
          pending.0.reject(failure)
          return
        }
        body(attempt)
      }
    }
  }
  func state(_ args: NearbyJoinLeaseArgs, invoke: Invoke) {
    guard let attempt = current, attempt.owner == args.owner, attempt.lease == args.lease,
          attempt.ready, !attempt.cancelled else { invoke.resolve(["available": false]); return }
    guard attempt.foreground else {
      finish(attempt, code: "JOIN_BACKGROUND", lost: true)
      invoke.resolve(["available": false]); return
    }
    if let interface = attempt.interface, nearbyCurrentWiFiInterface() == interface,
       attempt.lastVerified > 0,
       ProcessInfo.processInfo.systemUptime - attempt.lastVerified <= 3 {
      invoke.resolve(["available": true]); return
    }
    // Do not retire a valid owner solely for a path-generation/DHCP update.
    // The fresh proof also rejects a recycled index on another Wi-Fi network.
    query(args.owner, lease: args.lease, invoke: invoke) { _ in invoke.resolve(["available": true]) }
  }
  func bind(_ args: NearbyJoinBindArgs, invoke: Invoke) {
    query(args.owner, lease: args.lease, invoke: invoke) { attempt in
      guard let interface = attempt.interface,
            nearbyBindIPv4Socket(args.fd, address: args.address, interface: interface) else {
        invoke.reject("JOIN_ENDPOINT_NOT_ON_LINK_OR_BIND_FAILED"); return
      }
      var remote = in_addr()
      guard inet_pton(AF_INET, args.address, &remote) == 1 else { invoke.reject("JOIN_INVALID_ENDPOINT"); return }
      var selected: NearbyIPv4Prefix?
      for prefix in interface.prefixes where (prefix.address & prefix.mask) == (remote.s_addr & prefix.mask) {
        guard selected == nil else { invoke.reject("JOIN_AMBIGUOUS_INTERFACE_ADDRESS"); return }
        selected = prefix
      }
      guard let prefix = selected else { invoke.reject("JOIN_ENDPOINT_NOT_ON_LINK_OR_BIND_FAILED"); return }
      var local = in_addr(s_addr: prefix.address)
      var bound = sockaddr_in()
      bound.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
      bound.sin_family = sa_family_t(AF_INET)
      bound.sin_addr = local
      let result = withUnsafePointer(to: &bound) { pointer in
        pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
          Darwin.bind(args.fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
        }
      }
      guard result == 0 else { invoke.reject("JOIN_CONFIRMED_ADDRESS_BIND_FAILED"); return }
      var text = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
      guard inet_ntop(AF_INET, &local, &text, socklen_t(text.count)) != nil else { invoke.reject("JOIN_INVALID_LOCAL_ADDRESS"); return }
      let localAddress = String(cString: text)
      if !attempt.boundPrefixes.contains(prefix) { attempt.boundPrefixes.append(prefix) }
      func complete(_ verified: Attempt, mayReverify: Bool) {
        guard let live = verified.interface, live.index == interface.index,
              live.prefixes.contains(prefix) else {
          invoke.reject("JOIN_CONFIRMED_ADDRESS_BIND_FAILED"); return
        }
        guard nearbyCurrentWiFiInterface() == live else {
          guard mayReverify else { invoke.reject("JOIN_CONFIRMED_ADDRESS_BIND_FAILED"); return }
          // Every final snapshot mismatch takes the same single bounded proof,
          // including a generation update racing this final completion check.
          self.query(args.owner, lease: args.lease, invoke: invoke) {
            complete($0, mayReverify: false)
          }
          return
        }
        if verified.route == nil { verified.route = nearbyNextRouteReceipt() }
        invoke.resolve(["route": verified.route!, "localAddress": localAddress])
      }
      complete(attempt, mayReverify: true)
    }
  }
  func release(_ args: NearbyJoinLeaseArgs, invoke: Invoke) {
    guard let attempt = current, attempt.owner == args.owner, attempt.lease == args.lease else {
      invoke.resolve(); return
    }
    cancel(attempt, code: "JOIN_CANCELLED", lost: false) { error in
      if let error = error { invoke.reject(error) } else { invoke.resolve() }
    }
  }
  func cancel(_ owner: String, invoke: Invoke) {
    guard let attempt = current, attempt.owner == owner else { invoke.resolve(); return }
    cancel(attempt, code: "JOIN_CANCELLED", lost: false) { error in
      if let error = error { invoke.reject(error) } else { invoke.resolve() }
    }
  }
  func retire(scene: UIScene? = nil, completion: ((String?) -> Void)? = nil) {
    guard let attempt = current, scene == nil || attempt.scene === scene else {
      completion?(nil); return
    }
    cancel(attempt, code: "JOIN_BACKGROUND", lost: attempt.ready, completion: completion)
  }
  private func cancel(_ attempt: Attempt, code: String, lost: Bool, completion: ((String?) -> Void)?) {
    // A pending apply cannot be cancelled by iOS: cancel admission immediately
    // and retain its tombstone separately. A known app-created configuration,
    // however, must finish confirmed cleanup before release reports success.
    let awaitCleanup = attempt.applied && !attempt.existed
      && !attempt.applying && !attempt.queryingConfigurations
    if let completion = completion, awaitCleanup {
      guard attempt.cleanupWaiters.count < 8 else { completion("JOIN_RELEASE_BUSY"); return }
      attempt.cleanupWaiters.append(completion)
    }
    finish(attempt, code: code, lost: lost)
    if !awaitCleanup { completion?(nil) }
  }
  private func finish(_ attempt: Attempt, code: String, lost: Bool) {
    joinLog("finish code=\(code) lost=\(lost)")
    guard current === attempt else { return }
    attempt.idleTimer.release()
    let notify = lost && !attempt.cancelled && attempt.ready
    attempt.cancelled = true; attempt.timer?.cancel(); attempt.timer = nil
    attempt.monitor?.cancel(); attempt.monitor = nil
    if let activation = attempt.activation {
      NotificationCenter.default.removeObserver(activation); attempt.activation = nil
    }
    attempt.poll?.cancel(); attempt.poll = nil
    attempt.checkTimeout?.cancel(); attempt.checkTimeout = nil
    let invoke = attempt.invoke; attempt.invoke = nil; invoke?.reject(code)
    let queries = Array(attempt.queries.values); attempt.queries.removeAll()
    for (invoke, timeout) in queries { timeout.cancel(); invoke.reject(code) }
    if notify { emit(["owner": attempt.owner, "lease": attempt.lease, "kind": "lost", "error": code]) }
    // apply has no cancellation API. Retain this tombstone through its callback;
    // otherwise a delayed SSID-scoped removal could tear down a successor lease.
    guard !attempt.applying, !attempt.queryingConfigurations else { return }
    guard !attempt.cleaning else { return }
    if attempt.applied && !attempt.existed {
      attempt.cleaning = true; attempt.cleanupGeneration &+= 1
      let generation = attempt.cleanupGeneration
      let deadline = DispatchWorkItem { [weak self, weak attempt] in
        guard let self = self, let attempt = attempt, self.current === attempt,
              attempt.cleaning, attempt.cleanupGeneration == generation else { return }
        self.settleCleanup(attempt, error: "JOIN_CONFIGURATION_CLEANUP_PENDING")
      }
      attempt.cleanupDeadline = deadline
      DispatchQueue.main.asyncAfter(deadline: .now() + 10, execute: deadline)
      NEHotspotConfigurationManager.shared.removeConfiguration(forSSID: attempt.ssid)
      confirmConfigurationRemoved(attempt, generation: generation, remaining: 20)
    } else { current = nil }
  }

  private func confirmConfigurationRemoved(_ attempt: Attempt, generation: UInt64, remaining: Int) {
    // removeConfiguration has no completion handler. Retain the SSID owner until
    // the OS reports that this app's created configuration has really vanished.
    NEHotspotConfigurationManager.shared.getConfiguredSSIDs { [self, attempt] names in
      DispatchQueue.main.async {
        guard self.current === attempt, attempt.cleaning,
              attempt.cleanupGeneration == generation else { return }
        if !names.contains(attempt.ssid) {
          self.settleCleanup(attempt, error: nil)
        } else if remaining > 0 {
          DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) { [self, attempt] in
            guard self.current === attempt, attempt.cleaning,
                  attempt.cleanupGeneration == generation else { return }
            self.confirmConfigurationRemoved(attempt, generation: generation, remaining: remaining - 1)
          }
        } else {
          self.settleCleanup(attempt, error: "JOIN_CONFIGURATION_CLEANUP_PENDING")
        }
      }
    }
  }

  private func settleCleanup(_ attempt: Attempt, error: String?) {
    guard current === attempt, attempt.cleaning else { return }
    attempt.cleanupDeadline?.cancel(); attempt.cleanupDeadline = nil
    attempt.cleaning = false
    let waiters = attempt.cleanupWaiters; attempt.cleanupWaiters.removeAll()
    if let error = error {
      // Retain this obligation and exclude successors. A later exact-owner
      // release may retry, but failed cleanup is never reported as success.
      emit(["owner": attempt.owner, "lease": attempt.lease, "kind": "lost", "error": error])
    } else { current = nil }
    waiters.forEach { $0(error) }
  }

}
