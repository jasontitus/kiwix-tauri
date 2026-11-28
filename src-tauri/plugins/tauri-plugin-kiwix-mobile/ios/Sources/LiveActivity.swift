// SPDX-License-Identifier: GPL-3.0-or-later
//
// ActivityKit bridge for download Live Activities. Rust owns the projection:
// every call carries the current acquisition job snapshot and this file only
// mirrors it onto the system surface. It never reads the store, the network or
// the download manager, so there is no second source of truth.
//
// Privacy, read-only and notification-authorization decisions are made before
// content reaches here (Rust) and re-checked here (Live Activity authorization
// and notification permission) so a stale request can never paint a surface.
import ActivityKit
import Foundation
import Tauri
import UIKit
import UserNotifications
import os

private let liveActivityLog = Logger(subsystem: "com.tiltastech.zimreader", category: "live-activity")

struct LiveActivityArgs: Decodable {
  let state: String
  let jobId: String?
  let status: String?
  let statusLabel: String?
  let fileName: String?
  let downloaded: Int64?
  let total: Int64?
  let staleLabel: String?
}

/// One activity at a time, following the transfer Rust selected. Activities are
/// discovered from ActivityKit instead of a retained reference, so a request
/// that arrives after a process restart still updates or retires what the
/// system actually shows.
@MainActor
@available(iOS 16.2, *)
final class DownloadLiveActivityBridge {
  static let shared = DownloadLiveActivityBridge()
  enum Reconciliation: Equatable { case update, retire, ignore }

  nonisolated static func reconciliation(_ state: ActivityState, sameJob: Bool,
                                         hasCurrent: Bool) -> Reconciliation {
    if state == .dismissed { return .ignore }
    if sameJob && !hasCurrent && (state == .active || state == .stale) { return .update }
    // Ended activities may still show their last archive name on the lock
    // screen. They cannot be updated, but must be explicitly dismissed.
    return .retire
  }

  private var lastSent: (id: String, state: DownloadActivityAttributes.ContentState, at: Date)?
  private var notificationDenied: Bool?
  private var pending: Task<Void, Never>?

  private init() {
    NotificationCenter.default.addObserver(
      forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main
    ) { [weak self] _ in
      // Permission can only change in Settings or the explicit in-app request.
      self?.notificationDenied = nil
    }
  }

  nonisolated func apply(_ args: LiveActivityArgs, reply: @escaping ([String: Any]) -> Void) {
    Task { @MainActor in
      // ActivityKit awaits are reentrant. In particular, privacy Clear must
      // finish after any older Show, never let that Show publish after clearing.
      let previous = self.pending
      self.pending = Task { @MainActor in
        await previous?.value
        reply(await self.handle(args))
      }
    }
  }

  private func handle(_ args: LiveActivityArgs) async -> [String: Any] {
    let reply = await dispatch(args)
    liveActivityLog.notice(
      "request state=\(args.state, privacy: .public) job=\(args.jobId ?? "-", privacy: .public) status=\(args.status ?? "-", privacy: .public) downloaded=\(args.downloaded ?? 0, privacy: .public) total=\(args.total ?? 0, privacy: .public) result=\((reply["activity"] as? String) ?? "-", privacy: .public) reason=\((reply["reason"] as? String) ?? "-", privacy: .public)"
    )
    return reply
  }

  private func dispatch(_ args: LiveActivityArgs) async -> [String: Any] {
    guard ActivityAuthorizationInfo().areActivitiesEnabled else {
      await endAll(dismissal: .immediate)
      return ["available": false, "activity": "none",
              "reason": "Live Activities are disabled for Kiwix"]
    }
    switch args.state {
    case "active": return await show(args)
    case "terminal": return await finish(args, dismissAfter: 6)
    case "dismiss": return await finish(args, dismissAfter: 0)
    case "clear":
      await endAll(dismissal: .immediate)
      return ["available": true, "activity": "cleared"]
    default:
      return ["available": true, "activity": "ignored"]
    }
  }

  private func show(_ args: LiveActivityArgs) async -> [String: Any] {
    guard let jobId = args.jobId, let fileName = args.fileName, let status = args.status else {
      return ["available": true, "activity": "ignored"]
    }
    if notificationDenied == nil { notificationDenied = await Self.notificationsDenied() }
    if notificationDenied == true {
      await endAll(dismissal: .immediate)
      return ["available": true, "activity": "suppressed",
              "reason": "Notification permission is denied"]
    }
    let content = DownloadActivityAttributes.ContentState(
      status: status,
      statusLabel: args.statusLabel ?? status,
      fileName: fileName,
      downloaded: max(args.downloaded ?? 0, 0),
      total: args.total.flatMap { $0 > 0 ? $0 : nil },
      staleLabel: args.staleLabel
    )
    // A paused transfer is expected to stop updating; anything else that stops
    // being refreshed should visibly go stale rather than claim live progress.
    let staleDate = status == "paused" ? nil : Date().addingTimeInterval(120)

    var current: Activity<DownloadActivityAttributes>?
    for activity in Activity<DownloadActivityAttributes>.activities {
      switch Self.reconciliation(activity.activityState,
        sameJob: activity.attributes.jobId == jobId, hasCurrent: current != nil) {
      case .update: current = activity
      case .retire: await activity.end(nil, dismissalPolicy: .immediate)
      case .ignore: break
      }
    }
    if let activity = current {
      if activity.activityState == .active, let last = lastSent,
         last.id == activity.id, last.state == content,
         Date().timeIntervalSince(last.at) < 1.5 {
        return ["available": true, "activity": "unchanged"]
      }
      await activity.update(ActivityContent(state: content, staleDate: staleDate))
      lastSent = (activity.id, content, Date())
      return ["available": true, "activity": "updated"]
    }
    do {
      let activity = try Activity.request(
        attributes: DownloadActivityAttributes(jobId: jobId),
        content: ActivityContent(state: content, staleDate: staleDate),
        pushType: nil
      )
      lastSent = (activity.id, content, Date())
      return ["available": true, "activity": "started", "activityId": activity.id]
    } catch {
      // The system can refuse a request (limit reached, feature disabled).
      // A refused activity is not a transfer failure and is never retried from
      // here; the next snapshot asks again.
      return ["available": true, "activity": "refused", "reason": error.localizedDescription]
    }
  }

  private func finish(_ args: LiveActivityArgs, dismissAfter seconds: TimeInterval) async -> [String: Any] {
    let policy: ActivityUIDismissalPolicy =
      seconds > 0 ? .after(Date().addingTimeInterval(seconds)) : .immediate
    var content: DownloadActivityAttributes.ContentState?
    if args.jobId != nil, let fileName = args.fileName, let status = args.status {
      content = DownloadActivityAttributes.ContentState(
        status: status,
        statusLabel: args.statusLabel ?? status,
        fileName: fileName,
        downloaded: max(args.downloaded ?? 0, 0),
        total: args.total.flatMap { $0 > 0 ? $0 : nil }
      )
    }
    var ended = 0
    for activity in Activity<DownloadActivityAttributes>.activities where activity.activityState != .dismissed {
      if let jobId = args.jobId, activity.attributes.jobId != jobId { continue }
      if let content, activity.attributes.jobId == args.jobId {
        await activity.end(ActivityContent(state: content, staleDate: nil), dismissalPolicy: policy)
      } else {
        await activity.end(nil, dismissalPolicy: policy)
      }
      ended += 1
    }
    lastSent = nil
    return ["available": true, "activity": ended > 0 ? "ended" : "none"]
  }

  private func endAll(dismissal: ActivityUIDismissalPolicy) async {
    for activity in Activity<DownloadActivityAttributes>.activities where activity.activityState != .dismissed {
      await activity.end(nil, dismissalPolicy: dismissal)
    }
    lastSent = nil
  }

  /// Live Activities are a system surface like a notification; when the user
  /// denied notifications this app publishes nothing there either.
  private static func notificationsDenied() async -> Bool {
    let settings = await UNUserNotificationCenter.current().notificationSettings()
    return settings.authorizationStatus == .denied
  }
}

extension KiwixMobilePlugin {
  // Internal RustMobile.call transport only; no JavaScript command exports.
  @objc public func liveActivity(_ invoke: Invoke) {
    do {
      let args = try invoke.parseArgs(LiveActivityArgs.self)
      if #available(iOS 16.2, *) {
        DownloadLiveActivityBridge.shared.apply(args) { reply in invoke.resolve(reply) }
      } else {
        // ActivityKit does not exist below 16.1; the app deployment target is
        // 16.4, so this only protects a compiler target that predates it.
        invoke.resolve(["available": false, "activity": "unsupported"])
      }
    } catch {
      invoke.reject("Invalid Live Activity request")
    }
  }
}
