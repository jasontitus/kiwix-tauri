// SPDX-License-Identifier: GPL-3.0-or-later
//
// Shared read-aloud Live Activity model. This file is compiled into both the
// app (which requests/updates the activity) and the widget extension (which
// renders it). ActivityKit carries ContentState as Codable JSON, so both
// copies must keep the same field names.
import ActivityKit
import Foundation

@available(iOS 16.2, *)
struct ReadAloudActivityAttributes: ActivityAttributes {
  struct ContentState: Codable, Hashable {
    /// "reading" or "paused".
    var status: String
    /// Already localized by Rust.
    var statusLabel: String
    /// UTF-16 offset into the spoken text.
    var offset: Int64
    /// UTF-16 length of the spoken text, or 0 when unknown.
    var total: Int64
  }

  /// One immutable identity per started Kokoro session.
  var sessionId: String
  /// Article title; never read from the archive in the widget process.
  var title: String
  /// Human-readable voice name or URI.
  var voice: String
  /// Names of the transport controls, already localized by Rust. Optional so
  /// an activity that an earlier build started still decodes; the widget then
  /// falls back to its English names.
  var previousLabel: String?
  var pauseLabel: String?
  var resumeLabel: String?
  var stopLabel: String?
}
