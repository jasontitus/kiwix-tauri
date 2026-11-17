// SPDX-License-Identifier: GPL-3.0-or-later
//
// Lock-screen / Dynamic Island presentation for Kokoro read-aloud. The widget
// only renders values supplied through ActivityKit; all authority, privacy and
// lifecycle decisions remain in the app/Rust process.
import ActivityKit
import SwiftUI
import WidgetKit

@available(iOS 16.2, *)
struct KiwixReadAloudLiveActivity: Widget {
  var body: some WidgetConfiguration {
    ActivityConfiguration(for: ReadAloudActivityAttributes.self) { context in
      ReadAloudLockScreenView(attributes: context.attributes, state: context.state)
        .activityBackgroundTint(ReadAloudActivityStyle.background)
        .activitySystemActionForegroundColor(.white)
        // Tapping the card body must never synthesize a speech action. This
        // URL is currently only a guard on iOS (the host is ignored), so a
        // body tap still opens the app; the explicit previous/pause/play/stop
        // links remain the only transport controls.
        .widgetURL(URL(string: "kiwix://library")!)
    } dynamicIsland: { context in
      let state = context.state
      let attributes = context.attributes
      return DynamicIsland {
        DynamicIslandExpandedRegion(.leading) {
          Image(systemName: "book.pages.fill")
            .font(.title3)
            .foregroundStyle(ReadAloudActivityStyle.tint(state))
            .padding(.leading, 4)
        }
        DynamicIslandExpandedRegion(.trailing) {
          Text(ReadAloudActivityStyle.percent(state))
            .font(.system(.subheadline, design: .rounded).monospacedDigit())
            .foregroundStyle(.secondary)
            .padding(.trailing, 4)
        }
        DynamicIslandExpandedRegion(.bottom) {
          VStack(alignment: .leading, spacing: 5) {
            Text(attributes.title)
              .font(.subheadline.weight(.semibold))
              .lineLimit(1)
            Text("\(state.statusLabel) · \(ReadAloudActivityStyle.voice(attributes.voice))")
              .font(.caption)
              .foregroundStyle(.secondary)
              .lineLimit(1)
            ReadAloudActivityStyle.bar(state)
            ReadAloudActivityStyle.controls(state)
          }
          .padding(.horizontal, 4)
          .padding(.bottom, 4)
          .widgetURL(URL(string: "kiwix://library")!)
        }
      } compactLeading: {
        Image(systemName: "book.pages.fill")
          .foregroundStyle(ReadAloudActivityStyle.tint(state))
      } compactTrailing: {
        Text(ReadAloudActivityStyle.compact(state))
          .font(.system(.caption2, design: .rounded).monospacedDigit())
          .foregroundStyle(.secondary)
      } minimal: {
        Image(systemName: "book.pages.fill")
          .foregroundStyle(ReadAloudActivityStyle.tint(state))
      }
      .keylineTint(ReadAloudActivityStyle.tint(state))
      .widgetURL(URL(string: "kiwix://library")!)
    }
  }
}

@available(iOS 16.2, *)
struct ReadAloudLockScreenView: View {
  let attributes: ReadAloudActivityAttributes
  let state: ReadAloudActivityAttributes.ContentState

  var body: some View {
    VStack(alignment: .leading, spacing: 7) {
      HStack(spacing: 8) {
        Image(systemName: "book.pages.fill")
          .font(.system(size: 17, weight: .semibold))
          .foregroundStyle(ReadAloudActivityStyle.tint(state))
        VStack(alignment: .leading, spacing: 1) {
          Text(attributes.title)
            .font(.headline)
            .lineLimit(1)
          Text("\(state.statusLabel) · \(ReadAloudActivityStyle.voice(attributes.voice))")
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(1)
        }
        Spacer(minLength: 6)
        Text(ReadAloudActivityStyle.percent(state))
          .font(.system(.subheadline, design: .rounded).monospacedDigit())
          .foregroundStyle(.secondary)
      }
      ReadAloudActivityStyle.bar(state)
      HStack {
        Text(ReadAloudActivityStyle.offset(state))
          .font(.caption.monospacedDigit())
          .foregroundStyle(.secondary)
        Spacer()
        ReadAloudActivityStyle.controls(state)
      }
    }
    .padding(12)
  }
}

@available(iOS 16.2, *)
enum ReadAloudActivityStyle {
  static let background = Color(red: 0.09, green: 0.10, blue: 0.12)

  static func fraction(_ state: ReadAloudActivityAttributes.ContentState) -> Double {
    guard state.total > 0 else { return 0 }
    let value = Double(state.offset) / Double(state.total)
    return min(max(value, 0), 1)
  }

  static func percent(_ state: ReadAloudActivityAttributes.ContentState) -> String {
    guard state.total > 0 else { return "" }
    return "\(Int((fraction(state) * 100).rounded()))%"
  }

  static func compact(_ state: ReadAloudActivityAttributes.ContentState) -> String {
    if state.status == "paused" { return "❚❚" }
    guard state.total > 0 else { return "…" }
    return "\(Int((fraction(state) * 100).rounded()))%"
  }

  static func offset(_ state: ReadAloudActivityAttributes.ContentState) -> String {
    guard state.total > 0 else { return "" }
    return "\(Int((fraction(state) * 100).rounded()))% read"
  }

  static func voice(_ value: String) -> String {
    value.hasPrefix("kokoro:")
      ? "Kokoro · " + String(value.dropFirst("kokoro:".count))
      : value
  }

  static func tint(_ state: ReadAloudActivityAttributes.ContentState) -> Color {
    state.status == "paused" ? .orange : .accentColor
  }

  @ViewBuilder
  static func bar(_ state: ReadAloudActivityAttributes.ContentState) -> some View {
    if state.total > 0 {
      ProgressView(value: fraction(state))
        .progressViewStyle(.linear)
        .tint(tint(state))
    } else {
      ProgressView().progressViewStyle(.linear)
    }
  }

  @ViewBuilder
  static func controls(_ state: ReadAloudActivityAttributes.ContentState) -> some View {
    HStack(spacing: 18) {
      Link(destination: URL(string: "kiwix://speech?action=previous")!) {
        Image(systemName: "backward.end.fill")
      }
      .accessibilityLabel("Previous paragraph")
      Link(destination: URL(string: state.status == "paused"
        ? "kiwix://speech?action=play" : "kiwix://speech?action=pause")!) {
        Image(systemName: state.status == "paused" ? "play.fill" : "pause.fill")
      }
      .accessibilityLabel(state.status == "paused" ? "Resume reading" : "Pause reading")
      Link(destination: URL(string: "kiwix://speech?action=stop")!) {
        Image(systemName: "stop.fill")
      }
      .accessibilityLabel("Stop reading")
    }
    .font(.system(size: 17, weight: .semibold))
    .foregroundStyle(.white)
    .buttonStyle(.plain)
  }
}
