// SPDX-License-Identifier: GPL-3.0-or-later
import Darwin
import Foundation
import ObjectiveC
import UIKit

private let backgroundIdentifier = "com.tiltastech.zimreader.official-acquisition.v1"
typealias BackgroundReply = ([String: Any]) -> Void

/// These are transport receipts only. Canonical job identity, provenance, part
/// durability, integrity and publication remain the Rust acquisition manager's.
struct BackgroundLease: Codable {
  var id: String
  var generation: String
  var url: String
  var expectedSize: UInt64?
  var allowsCellular: Bool
  var state: String
  var copied: UInt64 = 0
  var total: UInt64?
  var resumeData: Data?
  var device: UInt64?
  var inode: UInt64?
  var responseStatus: Int?
  var finalURL: String?
  var error: String?
  var taskIdentifier: Int?
  var key: String { id + "." + generation }

  func acceptsCallback(task: Int) -> Bool { state == "running" && taskIdentifier == task }

  func validateStage(at path: URL) throws -> Bool {
    var info = stat()
    if lstat(path.path, &info) != 0 {
      if errno == ENOENT { return false }
      throw posixFailure("inspect_background_stage")
    }
    guard (info.st_mode & S_IFMT) == S_IFREG, info.st_nlink == 1,
          UInt64(info.st_dev) == device, UInt64(info.st_ino) == inode,
          info.st_size >= 0, UInt64(info.st_size) == copied else {
      throw MobileFailure("background_stage_identity_changed")
    }
    return true
  }
}

struct BackgroundPolicy: Codable, Equatable {
  var privateMode = false
  var readOnly = false
  var permitsTransferEvents: Bool { !privateMode && !readOnly }
}

struct BackgroundLedger: Codable {
  var version = 1
  var policy: BackgroundPolicy
  var leases: [String: BackgroundLease]
}

/// A process-owned background URLSession, not a foreground download dressed as
/// one. For this explicitly approved official-catalog path, redirects and network
/// restrictions are owned by iOS. Background sessions DO NOT call redirect
/// delegates; custom URL acquisitions must remain on the strict Rust transport.
final class BackgroundAcquisition: NSObject, URLSessionDownloadDelegate {
  static let shared = BackgroundAcquisition(root: FileManager.default
    .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    .appendingPathComponent("KiwixBackgroundAcquisition", isDirectory: true))
  private let queue = DispatchQueue(label: "org.kiwix.background-acquisition")
  private var leases: [String: BackgroundLease] = [:]
  private var loadError: String?
  private var retiring: Set<String> = []
  private var completionHandler: (() -> Void)?
  private var finishedEvents = false
  private var sessionStorage: URLSession?
  private var policy = BackgroundPolicy()
  private var policyTransition = false
  // A restored native receipt is not authority for this process's Store mode.
  private var policyAuthoritative = false
  private static var installedClasses: Set<ObjectIdentifier> = []
  private var hasManifest = false
  private let root: URL
  private let synchronizeDirectory: ((URL) throws -> Void)?
  private var manifest: URL { root.appendingPathComponent("leases.json") }

  init(root: URL, session: URLSession? = nil, synchronizeDirectory: ((URL) throws -> Void)? = nil) {
    self.root = root
    self.sessionStorage = session
    self.synchronizeDirectory = synchronizeDirectory
    super.init()
    do {
      var info = stat()
      if lstat(manifest.path, &info) == 0 {
        guard (info.st_mode & S_IFMT) == S_IFREG, info.st_nlink == 1,
              info.st_size <= 32 * 1024 * 1024 else { throw MobileFailure("invalid_background_manifest") }
        let saved = try JSONDecoder().decode(BackgroundLedger.self, from: Data(contentsOf: manifest))
        guard saved.version == 1, saved.leases.count <= 512 else { throw MobileFailure("invalid_background_ledger") }
        for (key, lease) in saved.leases {
          guard Self.validIdentity(lease.id), Self.validIdentity(lease.generation),
                key == lease.key, Self.validURL(lease.url),
                (lease.resumeData?.count ?? 0) <= 8 * 1024 * 1024 else {
            throw MobileFailure("invalid_background_lease")
          }
        }
        leases = saved.leases; policy = saved.policy; hasManifest = true
      } else if errno != ENOENT { throw posixFailure("read_background_manifest") }
    } catch {
      loadError = "Cannot restore background transfer receipts: \(error)"
      policy = BackgroundPolicy(privateMode: true, readOnly: true)
    }
  }

  /// Tao registers AppDelegate when constructing its EventLoop. Install before
  /// UIApplicationMain creates/assigns the delegate, not from plugin startup.
  static func install() -> Bool {
    precondition(Thread.isMainThread)
    guard let type = NSClassFromString("AppDelegate") else { return false }
    return install(on: type)
  }

  static func install(on type: AnyClass) -> Bool {
    precondition(Thread.isMainThread)
    let identifier = ObjectIdentifier(type)
    guard !installedClasses.contains(identifier) else { return true }
    let selector = NSSelectorFromString("application:handleEventsForBackgroundURLSession:completionHandler:")
    let previous = class_getInstanceMethod(type, selector).map(method_getImplementation)
    typealias Original = @convention(c) (AnyObject, Selector, UIApplication, NSString, @convention(block) () -> Void) -> Void
    let block: @convention(block) (AnyObject, UIApplication, NSString, @escaping @convention(block) () -> Void) -> Void = {
      object, application, identifier, completion in
      if identifier as String == backgroundIdentifier {
        Self.shared.attachCompletion(completion)
      } else if let previous = previous {
        unsafeBitCast(previous, to: Original.self)(object, selector, application, identifier, completion)
      } else { completion() }
    }
    let implementation = imp_implementationWithBlock(block)
    if !class_addMethod(type, selector, implementation, "v@:@@@?") {
      guard let method = class_getInstanceMethod(type, selector) else {
        imp_removeBlock(implementation)
        return false
      }
      method_setImplementation(method, implementation)
    }
    installedClasses.insert(identifier)
    // Never reassign UIApplication.delegate. Its SDK contract is `assign`;
    // clearing UIKit's initial delegate ownership and restoring a local object
    // leaves a dangling pointer after this method returns.
    // Session recreation is deliberately deferred to Rust's authoritative
    // set_policy bootstrap. Until then the daemon retains completion files and
    // no app-managed delegate write can precede actual Store policy admission.
    return true
  }

  private var session: URLSession {
    if let existing = sessionStorage { return existing }
    let configuration = URLSessionConfiguration.background(withIdentifier: backgroundIdentifier)
    configuration.sessionSendsLaunchEvents = true
    configuration.isDiscretionary = false
    configuration.allowsCellularAccess = true
    configuration.allowsExpensiveNetworkAccess = true
    configuration.allowsConstrainedNetworkAccess = false
    configuration.httpMaximumConnectionsPerHost = 3
    configuration.httpAdditionalHeaders = ["Accept-Encoding": "identity"]
    let delegates = OperationQueue()
    delegates.maxConcurrentOperationCount = 1
    delegates.underlyingQueue = queue
    let created = URLSession(configuration: configuration, delegate: self, delegateQueue: delegates)
    sessionStorage = created
    return created
  }

  private func attachCompletion(_ completion: @escaping () -> Void) {
    queue.async {
      // UIKit supplies one completion per delivery cycle. Never replace an
      // outstanding handler silently if a second cycle arrives before drain.
      let previous = self.completionHandler
      self.completionHandler = {
        previous?()
        completion()
      }
      if self.policyAuthoritative { _ = self.session }
      self.finishCompletionIfReady()
    }
  }

  private func finishCompletionIfReady() {
    guard policyAuthoritative, finishedEvents, let completion = completionHandler else { return }
    completionHandler = nil; finishedEvents = false
    DispatchQueue.main.async(execute: completion)
  }

  private static func validIdentity(_ value: String) -> Bool {
    UUID(uuidString: value)?.uuidString.lowercased() == value
  }

  private static func validURL(_ value: String) -> Bool {
    guard value.utf8.count <= 8192, let url = URLComponents(string: value) else { return false }
    return url.scheme == "https" && !(url.host ?? "").isEmpty
      && url.user == nil && url.password == nil && url.fragment == nil
  }

  private func stage(_ lease: BackgroundLease) -> URL { root.appendingPathComponent(lease.key + ".download") }

  private func ensureRoot() throws {
    var info = stat()
    if lstat(root.path, &info) != 0 {
      guard errno == ENOENT else { throw posixFailure("inspect_background_directory") }
      try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true,
        attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication, .posixPermissions: 0o700])
    }
    guard lstat(root.path, &info) == 0, (info.st_mode & S_IFMT) == S_IFDIR else {
      throw MobileFailure("unsafe_background_directory")
    }
    var url = root
    var values = URLResourceValues(); values.isExcludedFromBackup = true
    try url.setResourceValues(values)
  }

  private func syncRoot() throws {
    if let synchronizeDirectory = synchronizeDirectory { try synchronizeDirectory(root); return }
    let descriptor = open(root.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
    guard descriptor >= 0 else { throw posixFailure("open_background_directory") }
    defer { close(descriptor) }
    guard fsync(descriptor) == 0 else { throw posixFailure("sync_background_directory") }
  }

  private func save() throws {
    guard !policy.readOnly else { throw MobileFailure("background_read_only") }
    try persist(policy: policy)
  }

  private func persist(policy: BackgroundPolicy) throws {
    guard !self.policy.readOnly || policyTransition else { throw MobileFailure("background_read_only") }
    try ensureRoot()
    let data = try JSONEncoder().encode(BackgroundLedger(policy: policy, leases: leases))
    guard data.count <= 32 * 1024 * 1024 else { throw MobileFailure("background_manifest_too_large") }
    try data.write(to: manifest, options: .atomic)
    let descriptor = open(manifest.path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
    guard descriptor >= 0 else { throw posixFailure("open_background_manifest") }
    defer { close(descriptor) }
    guard fsync(descriptor) == 0 else { throw posixFailure("sync_background_manifest") }
    try syncRoot()
    hasManifest = true
  }

  private func validateStage(_ lease: BackgroundLease) throws -> Bool {
    try lease.validateStage(at: stage(lease))
  }

  /// The pre-rename journal remains ownership authority even when the final
  /// directory sync or completion-ledger commit fails. Never admit a new task
  /// (or adopt a new temporary inode) while that exact retained stage exists.
  private func recoverStage(_ lease: BackgroundLease) throws -> BackgroundLease? {
    guard try validateStage(lease) else { return nil }
    // A published completion already synced the rename and its ledger entry;
    // rewriting the whole ledger here would make every progress poll pay three
    // fsyncs for a download that is finished.
    if lease.state == "complete" { return lease }
    var repaired = lease
    repaired.state = "staging"; repaired.resumeData = nil
    leases[lease.key] = repaired
    do {
      try syncRoot()
      repaired.state = "complete"; repaired.error = nil
      leases[lease.key] = repaired
      try save()
      return repaired
    } catch {
      repaired.state = "staging"
      repaired.error = "Background stage recovery failed: \(error)"
      leases[lease.key] = repaired
      try? save()
      throw error
    }
  }

  /// Removes only this receipt's exact retained stage. The caller owns the
  /// directory sync so a batch removal pays one fsync instead of one per lease.
  @discardableResult
  private func unlinkStage(_ lease: BackgroundLease) throws -> Bool {
    // An admitted cleanup can outlive an asynchronous task-retirement wait.
    // Recheck at the mutation, not just at command admission.
    guard !policy.readOnly else { throw MobileFailure("background_read_only") }
    guard try validateStage(lease) else { return false }
    guard unlink(stage(lease).path) == 0 else { throw posixFailure("remove_background_stage") }
    return true
  }

  private func removeArtifacts(_ lease: BackgroundLease) throws {
    if try unlinkStage(lease) { try syncRoot() }
  }

  private func identity(_ args: [String: Any]) throws -> String {
    guard let id = args["id"] as? String, let generation = args["generation"] as? String,
          Self.validIdentity(id), Self.validIdentity(generation) else { throw MobileFailure("invalid_background_identity") }
    return id + "." + generation
  }

  fileprivate func call(_ input: Data, reply: @escaping BackgroundReply) {
    queue.async {
      do {
        guard let request = try JSONSerialization.jsonObject(with: input) as? [String: Any],
              let command = request["command"] as? String,
              let args = request["args"] as? [String: Any] else { throw MobileFailure("invalid_background_command") }
        if command == "policy" {
          try self.applyPolicy(args, reply: reply)
          return
        }
        guard self.policyAuthoritative else { throw MobileFailure("background_policy_not_ready") }
        if let error = self.loadError { throw MobileFailure("background_manifest_unavailable", error) }
        // A read-only progress poll needs the daemon's task list only to detect
        // a force-quit receipt that still claims `running`. Every other receipt
        // state is authoritative in the ledger, so polling a finished download
        // costs no XPC round-trip at all.
        if command == "snapshot" {
          let key = try self.identity(args)
          if self.leases[key]?.state != "running" {
            try self.perform(command, args: args, tasks: [], reply: reply)
            return
          }
        }
        self.session.getAllTasks { tasks in
          self.queue.async {
            do { try self.perform(command, args: args, tasks: tasks, reply: reply) }
            catch { reply(["failure": "Background transport operation failed: \(error)"]) }
          }
        }
      } catch { reply(["failure": "Background transport operation failed: \(error)"]) }
    }
  }

  func applyPolicy(_ args: [String: Any], reply: @escaping BackgroundReply) throws {
    guard let privateMode = args["privateMode"] as? Bool, let readOnly = args["readOnly"] as? Bool else {
      throw MobileFailure("invalid_background_policy")
    }
    guard !policyTransition else { throw MobileFailure("background_policy_still_changing") }
    let desired = BackgroundPolicy(privateMode: privateMode, readOnly: readOnly)
    let previous = policy
    policyTransition = true
    policy = desired
    if desired.permitsTransferEvents {
      do {
        if loadError != nil { throw MobileFailure("background_manifest_unavailable") }
        if previous != desired { try persist(policy: desired) }
        // Apply actual Rust policy BEFORE attaching the delegate to pending OS
        // events. An allowed relaunch keeps the daemon's completed file intact
        // until didFinishDownloadingTo can durably retain it.
        policyAuthoritative = true
        policyTransition = false
        _ = session
        reply([:])
      } catch {
        policy = BackgroundPolicy(privateMode: true, readOnly: true)
        policyAuthoritative = true
        session.getAllTasks { tasks in
          self.queue.async {
            self.retire(tasks) { _ in
              self.policyTransition = false
              reply(["failure": "Applying background policy: \(error)"])
            }
          }
        }
      }
      return
    }
    for key in leases.keys {
      if leases[key]?.state == "running" || leases[key]?.state == "queued" { leases[key]?.state = "paused" }
    }
    var persistenceError: Error?
    let mayPersist = !previous.readOnly && loadError == nil && (hasManifest || !desired.readOnly)
    if mayPersist {
      do { try persist(policy: desired) } catch { persistenceError = error }
    }
    policyAuthoritative = true
    session.getAllTasks { tasks in
      self.queue.async {
        self.pauseAll(tasks) {
          self.retire(tasks) { retirement in
            if mayPersist {
              do { try self.persist(policy: desired) } catch { persistenceError = error }
            }
            self.policyTransition = false
            if let error = persistenceError { reply(["failure": "Saving background policy: \(error)"]) }
            else if let retirement = retirement {
              reply(["failure": "Suspending background transfers: \(retirement)"])
            } else { reply([:]) }
          }
        }
      }
    }
  }

  func perform(_ command: String, args: [String: Any], tasks: [URLSessionTask], reply: @escaping BackgroundReply) throws {
    guard policyAuthoritative else { throw MobileFailure("background_policy_not_ready") }
    if policy.readOnly && command != "snapshot" && command != "stop" {
      throw MobileFailure("background_read_only")
    }
    if command == "start" && (!policy.permitsTransferEvents || policyTransition) {
      throw MobileFailure("background_network_denied")
    }
    if command == "reconcile" {
      guard let live = args["live"] as? [[String: Any]], live.count <= 512 else { throw MobileFailure("invalid_background_reconciliation") }
      let allowed = Set(try live.map(identity))
      let orphans = tasks.filter {
        guard let key = $0.taskDescription, allowed.contains(key),
              let lease = leases[key], lease.state == "running",
              lease.taskIdentifier == $0.taskIdentifier else { return true }
        return false
      }
      let stale = leases.values.filter { !allowed.contains($0.key) }
      for lease in stale {
        var retired = lease; retired.state = "cancelled"; retired.resumeData = nil
        leases[lease.key] = retired
      }
      if !stale.isEmpty { try save() }
      retire(orphans) { retirement in
        if let retirement = retirement {
          reply(["failure": "Reconciling background artifacts: \(retirement)"]); return
        }
        do {
          var removed = false
          var failures: [String] = []
          for lease in stale {
            do {
              if try self.unlinkStage(lease) { removed = true }
              self.leases.removeValue(forKey: lease.key)
            } catch {
              // Keep the receipt so the next reconcile retries this artifact;
              // one foreign/replaced stage must not block the rest.
              failures.append("\(lease.key): \(error)")
            }
          }
          // One directory sync for the whole batch. The cancelled receipts stay
          // durable until every artifact removal above is on disk.
          if removed { try self.syncRoot() }
          if !stale.isEmpty { try self.save() }
          reply(failures.isEmpty ? [:] : ["failure": "Reconciling background artifacts: \(failures.joined(separator: "; "))"])
        } catch { reply(["failure": "Reconciling background artifacts: \(error)"]) }
      }
      return
    }
    let key = try identity(args)
    let matching = tasks.filter { $0.taskDescription == key }
    switch command {
    case "start":
      guard !retiring.contains(key) else { throw MobileFailure("background_task_still_stopping") }
      let data = try JSONSerialization.data(withJSONObject: args)
      struct Request: Decodable { let id: String; let generation: String; let url: String; let expectedSize: UInt64?; let allowsCellular: Bool }
      let request = try JSONDecoder().decode(Request.self, from: data)
      guard Self.validURL(request.url), request.expectedSize ?? 0 <= UInt64(Int64.max) else { throw MobileFailure("invalid_background_url") }
      var lease: BackgroundLease
      if let existing = leases[key] {
        guard existing.url == request.url, existing.expectedSize == request.expectedSize else {
          throw MobileFailure("background_identity_conflict")
        }
        guard existing.state != "cancelled" else { throw MobileFailure("background_lease_cancelled") }
        if try recoverStage(existing) != nil { reply([:]); return }
        if existing.state == "complete" { throw MobileFailure("background_stage_missing") }
        lease = existing
        if existing.allowsCellular != request.allowsCellular {
          guard matching.isEmpty else { throw MobileFailure("background_policy_requires_retirement") }
          // Opaque resume data embeds the old request's network permission.
          // A narrower policy must never reuse it after settings cutover. The
          // cleared permission is committed by the single write below, before
          // any task resumes.
          lease.allowsCellular = request.allowsCellular
          lease.resumeData = nil; lease.copied = 0; lease.total = nil
        }
      } else {
        guard leases.count < 512 else { throw MobileFailure("too_many_background_leases") }
        lease = BackgroundLease(id: request.id, generation: request.generation, url: request.url,
          expectedSize: request.expectedSize, allowsCellular: request.allowsCellular, state: "queued")
      }
      if let task = matching.first {
        for duplicate in matching.dropFirst() { duplicate.cancel() }
        lease.state = "running"; lease.error = nil; lease.taskIdentifier = task.taskIdentifier
        leases[key] = lease
        // One durable transition per command: the receipt and the adopted task
        // identifier are committed together before the task runs.
        try save(); task.resume(); reply([:]); return
      }
      let task: URLSessionDownloadTask
      if let data = lease.resumeData { task = session.downloadTask(withResumeData: data) }
      else {
        var request = URLRequest(url: URL(string: lease.url)!)
        request.allowsCellularAccess = lease.allowsCellular
        request.allowsExpensiveNetworkAccess = lease.allowsCellular
        request.allowsConstrainedNetworkAccess = false
        request.setValue("identity", forHTTPHeaderField: "Accept-Encoding")
        task = session.downloadTask(with: request)
      }
      task.taskDescription = key
      task.countOfBytesClientExpectsToReceive = lease.expectedSize.map(Int64.init) ?? NSURLSessionTransferSizeUnknown
      lease.resumeData = nil; lease.state = "running"; lease.error = nil
      lease.taskIdentifier = task.taskIdentifier
      leases[key] = lease
      do { try save() } catch { task.cancel(); throw error }
      task.resume(); reply([:])
    case "snapshot":
      guard var lease = leases[key] else {
        reply(["state": "missing", "copied": 0, "stagingDirectory": root.path]); return
      }
      if lease.state == "staging", !policy.readOnly {
        // Only a staging receipt can still be completed by a surviving stage
        // file. A `complete` receipt awaiting acknowledgement is already
        // durable, and re-recovering it would rewrite and fsync the ledger.
        if let recovered = try recoverStage(lease) { lease = recovered }
        else {
          lease.state = "failed"; lease.error = "Background staging was interrupted"
          leases[key] = lease; try save()
        }
      }
      if lease.state == "complete", try !validateStage(lease) { throw MobileFailure("background_stage_missing") }
      // A force-quit can leave a receipt with no surviving OS task. Report that
      // as missing instead of an immortal running row, but retain its intent so
      // an already-queued completion delegate can still deliver its receipt.
      let state = lease.state == "running" && matching.isEmpty ? "missing" : lease.state
      var result: [String: Any] = ["state": state, "copied": lease.copied,
        "sourceUrl": lease.url, "allowsCellular": lease.allowsCellular, "stagingDirectory": root.path]
      if let total = lease.total { result["total"] = total }
      if lease.state == "complete" { result["path"] = stage(lease).path }
      if let device = lease.device { result["device"] = device }
      if let inode = lease.inode { result["inode"] = inode }
      if let status = lease.responseStatus { result["responseStatus"] = status }
      if let final = lease.finalURL { result["finalUrl"] = final }
      if let error = lease.error { result["error"] = error }
      reply(result)
    case "stop":
      guard var lease = leases[key] else {
        retire(matching) { retirement in
          if let retirement = retirement { reply(["failure": "Stopping background transfer: \(retirement)"]) }
          else { reply([:]) }
        }
        return
      }
      if policy.readOnly {
        if leases[key]?.state == "running" { leases[key]?.state = "paused" }
        retire(matching) { retirement in
          if let retirement = retirement { reply(["failure": "Stopping background transfer: \(retirement)"]) }
          else { reply([:]) }
        }
        return
      }
      let cancel = args["cancel"] as? Bool ?? false
      if !cancel && (lease.state == "complete" || lease.state == "staging") {
        retire(matching) { retirement in
          if let retirement = retirement { reply(["failure": "Stopping background transfer: \(retirement)"]) }
          else { reply([:]) }
        }
        return
      }
      retiring.insert(key)
      lease.state = cancel ? "cancelled" : "paused"
      leases[key] = lease
      do { try save() } catch {
        retire(matching) { retirement in
          if retirement == nil { self.retiring.remove(key) }
          reply(["failure": "Saving background stop intent: \(error)"])
        }
        return
      }
      let downloads = matching.compactMap { $0 as? URLSessionDownloadTask }
      if !cancel, let task = downloads.first {
        for duplicate in matching.dropFirst() { duplicate.cancel() }
        task.cancel(byProducingResumeData: { data in
          self.queue.async {
            do {
              if let data = data, data.count <= 8 * 1024 * 1024 { self.leases[key]?.resumeData = data }
              try self.save()
              self.retire(matching) { retirement in
                if retirement == nil { self.retiring.remove(key) }
                if let retirement = retirement {
                  reply(["failure": "Stopping background transfer: \(retirement)"])
                } else { reply([:]) }
              }
            } catch {
              self.retire(matching) { retirement in
                if retirement == nil { self.retiring.remove(key) }
                reply(["failure": "Saving background pause: \(error)"])
              }
            }
          }
        })
      } else {
        retire(matching) { retirement in
          if let retirement = retirement {
            reply(["failure": "Stopping background transfer: \(retirement)"])
            return
          }
          self.retiring.remove(key)
          do {
            if cancel { try self.removeArtifacts(lease); self.leases[key]?.resumeData = nil }
            try self.save(); reply([:])
          } catch { reply(["failure": "Stopping background transfer: \(error)"]) }
        }
      }
    case "acknowledge":
      retire(matching) { retirement in
        if let retirement = retirement {
          reply(["failure": "Acknowledging background receipt: \(retirement)"]); return
        }
        do {
          if let lease = self.leases[key] {
            try self.removeArtifacts(lease); self.leases.removeValue(forKey: key); try self.save()
          }
          reply([:])
        } catch { reply(["failure": "Acknowledging background receipt: \(error)"]) }
      }
    default: throw MobileFailure("unknown_background_command")
    }
  }

  // getAllTasks no longer lists a task only after native retirement. Waiting for
  // this acknowledgement prevents a quick Resume racing cancellation callbacks.
  private func pauseAll(_ tasks: [URLSessionTask], completion: @escaping () -> Void) {
    let pending = DispatchGroup()
    for task in tasks {
      guard let download = task as? URLSessionDownloadTask else { task.cancel(); continue }
      pending.enter()
      download.cancel(byProducingResumeData: { data in
        self.queue.async {
          if let key = download.taskDescription, let data = data, data.count <= 8 * 1024 * 1024,
             self.leases[key]?.taskIdentifier == download.taskIdentifier {
            self.leases[key]?.resumeData = data
          }
          pending.leave()
        }
      })
    }
    pending.notify(queue: queue, execute: completion)
  }

  private func retire(_ tasks: [URLSessionTask], completion: @escaping (Error?) -> Void) {
    let identifiers = Set(tasks.map(\.taskIdentifier))
    for task in tasks { task.cancel() }
    guard !identifiers.isEmpty else { completion(nil); return }
    // The wait backs off and is bounded: a task the daemon never drops must not
    // spin at 20 Hz or leave the caller's reply unresolved forever.
    let deadline = DispatchTime.now() + 10
    var delay: TimeInterval = 0.05
    func wait() {
      self.session.getAllTasks { remaining in
        self.queue.async {
          guard remaining.contains(where: { identifiers.contains($0.taskIdentifier) }) else {
            completion(nil); return
          }
          guard DispatchTime.now() < deadline else {
            // The daemon may still hold the task. Keep the key blocked until
            // a later enumeration proves it is gone; resuming a zombie would
            // leave the receipt running forever with no callback left.
            self.watchRetirement(Set(tasks.compactMap(\.taskDescription)), identifiers)
            completion(MobileFailure("background_task_retirement_timeout")); return
          }
          let next = delay
          delay = min(delay * 2, 1)
          self.queue.asyncAfter(deadline: .now() + next) { wait() }
        }
      }
    }
    wait()
  }

  /// Clear a timed-out retirement obligation only after the daemon no longer
  /// lists the cancelled task identifiers.
  private func watchRetirement(_ keys: Set<String>, _ identifiers: Set<Int>) {
    guard !keys.isEmpty else { return }
    queue.asyncAfter(deadline: .now() + 1) {
      self.session.getAllTasks { tasks in
        self.queue.async {
          if tasks.contains(where: { identifiers.contains($0.taskIdentifier) }) {
            self.watchRetirement(keys, identifiers)
          } else {
            self.retiring.subtract(keys)
          }
        }
      }
    }
  }

  func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask,
                  didWriteData bytesWritten: Int64, totalBytesWritten: Int64, totalBytesExpectedToWrite: Int64) {
    guard policyAuthoritative, policy.permitsTransferEvents, !policyTransition,
          let key = downloadTask.taskDescription, var lease = leases[key],
          lease.acceptsCallback(task: downloadTask.taskIdentifier) else { return }
    lease.copied = UInt64(max(0, totalBytesWritten))
    lease.total = totalBytesExpectedToWrite >= 0 ? UInt64(totalBytesExpectedToWrite) : nil
    leases[key] = lease
  }

  func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didFinishDownloadingTo location: URL) {
    finishDownload(downloadTask, response: downloadTask.response as? HTTPURLResponse, location: location)
  }

  func finishDownload(_ downloadTask: URLSessionDownloadTask, response: HTTPURLResponse?, location: URL) {
    guard policyAuthoritative, policy.permitsTransferEvents, !policyTransition,
          let key = downloadTask.taskDescription, var lease = leases[key],
          lease.acceptsCallback(task: downloadTask.taskIdentifier) else { return }
    var retained = false
    do {
      // A legacy failed/paused receipt may still own a completed stage. Recover
      // it before a late completion can replace the only deletion authority.
      if try validateStage(lease) {
        retained = true
        _ = try recoverStage(lease)
        return
      }
      guard let response = response,
            let finalURL = response.url, Self.validURL(finalURL.absoluteString) else {
        throw MobileFailure("invalid_background_response")
      }
      let descriptor = open(location.path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
      guard descriptor >= 0 else { throw posixFailure("open_background_result") }
      defer { close(descriptor) }
      var info = stat()
      guard fstat(descriptor, &info) == 0, (info.st_mode & S_IFMT) == S_IFREG,
            info.st_nlink == 1, info.st_size >= 0 else { throw MobileFailure("invalid_background_result") }
      let size = UInt64(info.st_size)
      guard Self.validCompletedResponse(response, size: size, expectedSize: lease.expectedSize) else {
        throw MobileFailure("background_result_size_mismatch")
      }
      try FileManager.default.setAttributes(
        [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication], ofItemAtPath: location.path)
      guard fsync(descriptor) == 0 else { throw posixFailure("sync_background_result") }
      lease.copied = size; lease.total = size
      lease.responseStatus = response.statusCode; lease.finalURL = finalURL.absoluteString
      lease.device = UInt64(info.st_dev); lease.inode = UInt64(info.st_ino)
      // Journal the exact inode before the atomic no-clobber transport move.
      lease.state = "staging"; lease.resumeData = nil
      leases[key] = lease; try save()
      guard renamex_np(location.path, stage(lease).path, UInt32(RENAME_EXCL)) == 0 else {
        throw posixFailure("retain_background_result")
      }
      retained = true
      try syncRoot()
      guard try validateStage(lease) else { throw MobileFailure("background_stage_missing") }
      lease.state = "complete"; leases[key] = lease; try save()
    } catch {
      lease.state = retained ? "staging" : "failed"
      lease.error = "Background completion failed: \(error)"
      leases[key] = lease
      do { try save() } catch {
        // The staging journal was committed before rename. Keep it recoverable
        // rather than installing a global error that prevents retry/cancel.
        if !retained { loadError = "Cannot durably retain background completion failure: \(error)" }
      }
    }
  }

  func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
    guard policyAuthoritative, policy.permitsTransferEvents, !policyTransition,
          let key = task.taskDescription, var lease = leases[key],
          !retiring.contains(key), lease.acceptsCallback(task: task.taskIdentifier) else { return }
    if let error = error {
      let native = error as NSError
      if let data = native.userInfo[NSURLSessionDownloadTaskResumeData] as? Data, data.count <= 8 * 1024 * 1024 {
        lease.resumeData = data
      }
      lease.state = "failed"; lease.error = error.localizedDescription
    } else {
      lease.state = "failed"; lease.error = "Background task ended without a durable completion receipt"
    }
    leases[key] = lease
    do { try save() } catch { loadError = "Cannot durably retain background task failure: \(error)" }
  }

  /// A resumed background task may report a partial HTTP response while handing
  /// us the full reassembled file. Validate the range's whole length, not just
  /// the last response's Content-Length, before Rust's independent hash gate.
  static func validCompletedResponse(_ response: HTTPURLResponse, size: UInt64, expectedSize: UInt64?) -> Bool {
    guard expectedSize.map({ $0 == size }) ?? true,
          response.value(forHTTPHeaderField: "Content-Encoding").map({ $0.lowercased() == "identity" }) ?? true else { return false }
    if response.statusCode == 200 {
      return response.expectedContentLength < 0 || UInt64(response.expectedContentLength) == size
    }
    guard response.statusCode == 206,
          let range = response.value(forHTTPHeaderField: "Content-Range"), range.hasPrefix("bytes ") else { return false }
    let parts = range.dropFirst(6).split(separator: "/", omittingEmptySubsequences: false)
    guard parts.count == 2, let total = UInt64(parts[1]), total == size else { return false }
    let bounds = parts[0].split(separator: "-", omittingEmptySubsequences: false)
    guard bounds.count == 2, let start = UInt64(bounds[0]), let end = UInt64(bounds[1]),
          start <= end, end < UInt64.max, end + 1 == total else { return false }
    return response.expectedContentLength < 0 || UInt64(response.expectedContentLength) == end - start + 1
  }

  func urlSessionDidFinishEvents(forBackgroundURLSession session: URLSession) {
    finishedEvents = true
    finishCompletionIfReady()
  }
}

/// Synchronous pre-UIApplicationMain installation called from Rust after the
/// Tao runtime has registered AppDelegate, before it enters App::run.
@_cdecl("kiwix_ios_background_install_delegate_handoff")
public func kiwixIOSBackgroundInstallDelegateHandoff() -> Int32 {
  BackgroundAcquisition.install() && IncomingOriginalAuthority.install() ? 1 : 0
}

/// Internal Rust ABI only; no webview command can grant official provenance.
/// The callback is invoked exactly once and borrows UTF-8 for its duration.
@_cdecl("kiwix_ios_background_call")
public func kiwixIOSBackgroundCall(_ input: UnsafePointer<CChar>?,
  _ callback: @escaping @convention(c) (UnsafePointer<CChar>?, UnsafeMutableRawPointer?) -> Void,
  _ context: UnsafeMutableRawPointer?) {
  guard let input = input else {
    "{\"failure\":\"Missing background request\"}".withCString { callback($0, context) }; return
  }
  let data = Data(String(cString: input).utf8)
  BackgroundAcquisition.shared.call(data) { result in
    do {
      let response = try JSONSerialization.data(withJSONObject: result)
      let string = String(decoding: response, as: UTF8.self)
      string.withCString { callback($0, context) }
    } catch {
      "{\"failure\":\"Cannot encode background response\"}".withCString { callback($0, context) }
    }
  }
}
