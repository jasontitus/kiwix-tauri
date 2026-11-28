// SPDX-License-Identifier: GPL-3.0-or-later
//
// Live Activity / Dynamic Island for archive downloads. The extension renders
// only what Rust put in the content state; it never reads files, the network or
// the acquisition store. Every value arrives through ActivityKit.
import ActivityKit
import SwiftUI
import WidgetKit

@main
struct KiwixDownloadWidgetBundle: WidgetBundle {
  var body: some Widget {
    KiwixDownloadLiveActivity()
    KiwixReadAloudLiveActivity()
  }
}

struct KiwixDownloadLiveActivity: Widget {
  var body: some WidgetConfiguration {
    ActivityConfiguration(for: DownloadActivityAttributes.self) { context in
      DownloadActivityLockScreenView(
        state: DownloadActivityStyle.presented(context.state, isStale: context.isStale))
        .activityBackgroundTint(DownloadActivityStyle.background)
        .activitySystemActionForegroundColor(.white)
    } dynamicIsland: { context in
      let state = DownloadActivityStyle.presented(context.state, isStale: context.isStale)
      return DynamicIsland {
        DynamicIslandExpandedRegion(.leading) {
          Image(systemName: DownloadActivityStyle.symbol(state))
            .font(.title3)
            .foregroundStyle(DownloadActivityStyle.tint(state))
            .padding(.leading, 4)
        }
        DynamicIslandExpandedRegion(.trailing) {
          Text(DownloadActivityStyle.percent(state))
            .font(.system(.subheadline, design: .rounded).monospacedDigit())
            .foregroundStyle(.secondary)
            .padding(.trailing, 4)
        }
        DynamicIslandExpandedRegion(.bottom) {
          VStack(alignment: .leading, spacing: 4) {
            DownloadActivityStyle.bar(state)
            HStack(spacing: 6) {
              Text(state.fileName)
                .font(.caption)
                .lineLimit(1)
                .truncationMode(.middle)
              Spacer(minLength: 4)
              Text(DownloadActivityStyle.bytes(state))
                .font(.caption.monospacedDigit())
                .foregroundStyle(.secondary)
            }
          }
          .padding(.horizontal, 4)
          .padding(.bottom, 4)
        }
      } compactLeading: {
        Image(systemName: DownloadActivityStyle.symbol(state))
          .foregroundStyle(DownloadActivityStyle.tint(state))
          .accessibilityLabel(DownloadActivityStyle.spoken(state))
      } compactTrailing: {
        Text(DownloadActivityStyle.compactPercent(state))
          .font(.system(.caption2, design: .rounded).monospacedDigit())
          .foregroundStyle(.secondary)
          .accessibilityLabel(DownloadActivityStyle.spokenProgress(state))
      } minimal: {
        Image(systemName: DownloadActivityStyle.symbol(state))
          .foregroundStyle(DownloadActivityStyle.tint(state))
          .accessibilityLabel(DownloadActivityStyle.spoken(state))
      }
      .keylineTint(DownloadActivityStyle.tint(state))
    }
  }
}

struct DownloadActivityLockScreenView: View {
  let state: DownloadActivityAttributes.ContentState

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      HStack(spacing: 8) {
        Image(systemName: DownloadActivityStyle.symbol(state))
          .font(.system(size: 16, weight: .semibold))
          .foregroundStyle(DownloadActivityStyle.tint(state))
        Text(state.fileName)
          .font(.headline)
          .lineLimit(1)
          .truncationMode(.middle)
        Spacer(minLength: 6)
        Text(DownloadActivityStyle.percent(state))
          .font(.system(.subheadline, design: .rounded).monospacedDigit())
          .foregroundStyle(.secondary)
      }
      DownloadActivityStyle.bar(state)
      HStack(spacing: 6) {
        Text(state.statusLabel)
        Spacer(minLength: 4)
        Text(DownloadActivityStyle.bytes(state))
          .monospacedDigit()
      }
      .font(.caption)
      .foregroundStyle(.secondary)
    }
    .padding(12)
  }
}

enum DownloadActivityStyle {
  static let background = Color(red: 0.09, green: 0.10, blue: 0.12)

  /// ActivityKit only flags content that passed its stale date; it keeps
  /// drawing the last state. The app sets that date on every refresh of a
  /// running transfer, so stale means the app stopped reporting (suspended or
  /// gone) and the last "Downloading 40%" is no longer known to be true. Such a
  /// state is shown under the widget-only status "stale": last known bytes, no
  /// percentage, and the wording Rust supplied for this case in place of the
  /// activity. Paused and finished transfers never expire and are left alone.
  /// The status never leaves this process.
  static func presented(_ state: DownloadActivityAttributes.ContentState,
                        isStale: Bool) -> DownloadActivityAttributes.ContentState {
    guard isStale else { return state }
    switch state.status {
    case "paused", "complete", "completed", "failed", "error", "cancelled", "canceled": return state
    default: break
    }
    var shown = state
    shown.status = "stale"
    // The extension has no localized wording of its own; without Rust's it
    // shows none rather than English.
    shown.statusLabel = state.staleLabel ?? ""
    return shown
  }

  /// VoiceOver name for a region that only draws a symbol or a glyph.
  static func spoken(_ state: DownloadActivityAttributes.ContentState) -> String {
    state.statusLabel.isEmpty ? state.fileName : state.statusLabel
  }

  /// VoiceOver name for the compact percentage, which is a bare glyph while
  /// paused, failed or of unknown size.
  static func spokenProgress(_ state: DownloadActivityAttributes.ContentState) -> String {
    let text = percent(state)
    return text.isEmpty ? spoken(state) : text
  }

  static func fraction(_ state: DownloadActivityAttributes.ContentState) -> Double {
    if state.status == "complete" { return 1 }
    guard let total = state.total, total > 0 else { return 0 }
    let value = Double(state.downloaded) / Double(total)
    return min(max(value, 0), 1)
  }

  /// A transfer with no content length and no byte count cannot show a
  /// determinate bar; the system spinner is the honest presentation.
  static func indeterminate(_ state: DownloadActivityAttributes.ContentState) -> Bool {
    guard state.status == "queued" || state.status == "verifying" else { return false }
    return state.total == nil || state.total == 0
  }

  static func percent(_ state: DownloadActivityAttributes.ContentState) -> String {
    if state.status == "complete" { return "100%" }
    if state.status == "stale" { return "" }
    guard let total = state.total, total > 0 else { return "" }
    return "\(Int((fraction(state) * 100).rounded()))%"
  }

  static func compactPercent(_ state: DownloadActivityAttributes.ContentState) -> String {
    switch state.status {
    case "complete": return "100%"
    case "paused": return "❚❚"
    case "failed", "cancelled": return "!"
    case "stale": return "…"
    default: break
    }
    guard let total = state.total, total > 0 else { return "…" }
    return "\(Int((fraction(state) * 100).rounded()))%"
  }

  static func bytes(_ state: DownloadActivityAttributes.ContentState) -> String {
    let formatter = ByteCountFormatter()
    formatter.countStyle = .file
    formatter.allowedUnits = [.useMB, .useGB]
    formatter.isAdaptive = true
    let written = formatter.string(fromByteCount: state.downloaded)
    guard let total = state.total, total > 0 else { return written }
    return written + " / " + formatter.string(fromByteCount: total)
  }

  static func symbol(_ state: DownloadActivityAttributes.ContentState) -> String {
    switch state.status {
    case "paused": return "pause.circle.fill"
    case "verifying": return "checkmark.shield.fill"
    case "complete": return "checkmark.circle.fill"
    case "failed": return "exclamationmark.triangle.fill"
    case "cancelled": return "xmark.circle.fill"
    case "queued": return "clock.fill"
    case "stale": return "arrow.down.circle"
    default: return "arrow.down.circle.fill"
    }
  }

  static func tint(_ state: DownloadActivityAttributes.ContentState) -> Color {
    switch state.status {
    case "paused": return .orange
    case "verifying": return .cyan
    case "complete": return .green
    case "failed": return .red
    case "cancelled", "stale": return .gray
    default: return .accentColor
    }
  }

  @ViewBuilder
  static func bar(_ state: DownloadActivityAttributes.ContentState) -> some View {
    if indeterminate(state) {
      ProgressView().progressViewStyle(.linear)
    } else {
      ProgressView(value: fraction(state))
        .progressViewStyle(.linear)
        .tint(tint(state))
    }
  }
}
