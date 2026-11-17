// SPDX-License-Identifier: GPL-3.0-or-later
//
// Shared Live Activity model. This file is compiled twice: once into the app
// (the KiwixMobile module, which requests/updates the activity) and once into
// the KiwixDownloadActivity widget extension (which renders it). ActivityKit
// carries the content state as Codable JSON, so both copies must keep the same
// field names; Rust supplies every value from the acquisition job snapshot.
import ActivityKit
import Foundation

@available(iOS 16.2, *)
struct DownloadActivityAttributes: ActivityAttributes {
  /// Mutable part of the activity. `status` is one of the acquisition job
  /// statuses (`queued`, `downloading`, `verifying`, `paused`, `complete`,
  /// `failed`, `cancelled`); `statusLabel` is already localized by Rust.
  struct ContentState: Codable, Hashable {
    var status: String
    var statusLabel: String
    var fileName: String
    var downloaded: Int64
    var total: Int64?
  }

  /// Identity of the transfer this activity follows. Immutable for the
  /// activity's lifetime; a different job ends this activity and starts one.
  var jobId: String
}
