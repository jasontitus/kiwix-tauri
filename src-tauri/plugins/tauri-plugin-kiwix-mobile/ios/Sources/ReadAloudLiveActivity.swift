// SPDX-License-Identifier: GPL-3.0-or-later
//
// ActivityKit bridge for the Kokoro read-aloud Live Activity. Rust owns the
// projection; this file only mirrors the current speech state. It never reads
// archive content, files or the network.
import ActivityKit
import Foundation
import Tauri
import UIKit
import UserNotifications
import os

private let readAloudActivityLog = Logger(
  subsystem: "com.tiltastech.zimreader", category: "read-aloud-activity")

struct ReadAloudActivityArgs: Decodable {
  let state: String
  let sessionId: String?
  let title: String?
  let voice: String?
  let status: String?
  let statusLabel: String?
  let offset: Int64?
  let total: Int64?
}

@MainActor
@available(iOS 16.2, *)
final class ReadAloudLiveActivityBridge {
  static let shared = ReadAloudLiveActivityBridge()

  private var lastSent: (id: String, state: ReadAloudActivityAttributes.ContentState, at: Date)?
  private var notificationDenied: Bool?
  private var pending: Task<Void, Never>?

  private init() {
    NotificationCenter.default.addObserver(
      forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main
    ) { [weak self] _ in
      self?.notificationDenied = nil
    }
  }

  nonisolated func apply(_ args: ReadAloudActivityArgs, reply: @escaping ([String: Any]) -> Void) {
    Task { @MainActor in
      let previous = self.pending
      self.pending = Task { @MainActor in
        await previous?.value
        reply(await self.handle(args))
      }
    }
  }

  private func handle(_ args: ReadAloudActivityArgs) async -> [String: Any] {
    let reply = await dispatch(args)
    readAloudActivityLog.notice(
      "request state=\(args.state, privacy: .public) session=\(args.sessionId ?? "-", privacy: .public) status=\(args.status ?? "-", privacy: .public) offset=\(args.offset ?? 0, privacy: .public) total=\(args.total ?? 0, privacy: .public) result=\((reply["activity"] as? String) ?? "-", privacy: .public) reason=\((reply["reason"] as? String) ?? "-", privacy: .public)"
    )
    return reply
  }

  private func dispatch(_ args: ReadAloudActivityArgs) async -> [String: Any] {
    guard ActivityAuthorizationInfo().areActivitiesEnabled else {
      await endAll(dismissal: .immediate)
      return ["available": false, "activity": "none",
              "reason": "Live Activities are disabled for Kiwix"]
    }
    switch args.state {
    case "active": return await show(args)
    case "terminal": return await finish(args, dismissal: .immediate)
    case "clear":
      await endAll(dismissal: .immediate)
      return ["available": true, "activity": "cleared"]
    default:
      return ["available": true, "activity": "ignored"]
    }
  }

  private func show(_ args: ReadAloudActivityArgs) async -> [String: Any] {
    guard let sessionId = args.sessionId else {
      return ["available": true, "activity": "ignored"]
    }
    if notificationDenied == nil { notificationDenied = await Self.notificationsDenied() }
    if notificationDenied == true {
      await endAll(dismissal: .immediate)
      return ["available": true, "activity": "suppressed",
              "reason": "Notification permission is denied"]
    }

    var current: Activity<ReadAloudActivityAttributes>?
    for activity in Activity<ReadAloudActivityAttributes>.activities {
      if activity.activityState == .dismissed { continue }
      if activity.attributes.sessionId == sessionId {
        current = activity
      } else {
        await activity.end(nil, dismissalPolicy: .immediate)
      }
    }

    let now = Date()
    let status = args.status ?? "reading"
    let staleDate = status == "paused" ? nil : now.addingTimeInterval(120)

    if let activity = current {
      let previous = activity.content.state
      let content = ReadAloudActivityAttributes.ContentState(
        status: status,
        statusLabel: args.statusLabel ?? previous.statusLabel,
        offset: max(args.offset ?? previous.offset, 0),
        total: args.total ?? previous.total
      )
      if let last = lastSent, last.id == activity.id, last.state == content,
         now.timeIntervalSince(last.at) < 1.0 {
        return ["available": true, "activity": "unchanged"]
      }
      await activity.update(ActivityContent(state: content, staleDate: staleDate))
      lastSent = (activity.id, content, now)
      return ["available": true, "activity": "updated"]
    }

    guard let title = args.title, !title.isEmpty else {
      return ["available": true, "activity": "ignored"]
    }
    let content = ReadAloudActivityAttributes.ContentState(
      status: status,
      statusLabel: args.statusLabel ?? "Reading aloud",
      offset: max(args.offset ?? 0, 0),
      total: max(args.total ?? 0, 0)
    )
    do {
      let activity = try Activity.request(
        attributes: ReadAloudActivityAttributes(
          sessionId: sessionId,
          title: title,
          voice: args.voice ?? ""
        ),
        content: ActivityContent(state: content, staleDate: staleDate),
        pushType: nil
      )
      lastSent = (activity.id, content, now)
      return ["available": true, "activity": "started", "activityId": activity.id]
    } catch {
      return ["available": true, "activity": "refused", "reason": error.localizedDescription]
    }
  }

  private func finish(_ args: ReadAloudActivityArgs, dismissal: ActivityUIDismissalPolicy) async -> [String: Any] {
    var ended = 0
    for activity in Activity<ReadAloudActivityAttributes>.activities
      where activity.activityState != .dismissed {
      if let sessionId = args.sessionId, activity.attributes.sessionId != sessionId { continue }
      await activity.end(nil, dismissalPolicy: dismissal)
      ended += 1
    }
    lastSent = nil
    return ["available": true, "activity": ended > 0 ? "ended" : "none"]
  }

  private func endAll(dismissal: ActivityUIDismissalPolicy) async {
    for activity in Activity<ReadAloudActivityAttributes>.activities
      where activity.activityState != .dismissed {
      await activity.end(nil, dismissalPolicy: dismissal)
    }
    lastSent = nil
  }

  private static func notificationsDenied() async -> Bool {
    let settings = await UNUserNotificationCenter.current().notificationSettings()
    return settings.authorizationStatus == .denied
  }
}

extension KiwixMobilePlugin {
  @objc public func readAloudActivity(_ invoke: Invoke) {
    do {
      let args = try invoke.parseArgs(ReadAloudActivityArgs.self)
      if #available(iOS 16.2, *) {
        ReadAloudLiveActivityBridge.shared.apply(args) { reply in invoke.resolve(reply) }
      } else {
        invoke.resolve(["available": false, "activity": "unsupported"])
      }
    } catch {
      invoke.reject("Invalid read-aloud Live Activity request")
    }
  }
}
