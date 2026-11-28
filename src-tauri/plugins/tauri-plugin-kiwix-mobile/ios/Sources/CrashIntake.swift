import Foundation

/// Bounded, redacted record for an Objective-C/Swift uncaught exception.
/// Rust panics use `crash.rs`; this covers failures that never unwind through
/// Rust. The record is written under the app-private application-support
/// profile, matching Tauri's `app_data_dir`, so the next launch can offer it.
enum CrashIntake {
  private static let lock = NSLock()
  private static var installed = false
  // Fail closed until Rust supplies the effective profile policy.
  private static var enabled = false
  private static let maxFrames = 24
  private static let maxFrame = 240
  private static let maxRecordBytes = 32 * 1024
  private static let recordDirectory = "crash"
  private static let recordFile = "pending.json"
  private static let previousFile = "previous.json"
  private static let tempFile = ".pending.json.tmp"

  static func install() {
    lock.lock()
    defer { lock.unlock() }
    guard !installed else { return }
    installed = true
    NSSetUncaughtExceptionHandler { exception in
      CrashIntake.capture(exception)
    }
  }
  static func setEnabled(_ value: Bool) {
    lock.lock()
    enabled = value
    lock.unlock()
  }

  static func withCapturePermission(_ write: () -> Void) {
    // An exception raised while this same thread changes policy must not
    // deadlock the exception handler. Losing capture is safer than writing
    // without authority or masking the original crash.
    guard lock.try() else { return }
    defer { lock.unlock() }
    guard enabled else { return }
    write()
  }


  private static func capture(_ exception: NSException) {
    withCapturePermission { writeRecord(exception) }
  }

  private static func writeRecord(_ exception: NSException) {
    // A frame keeps its spacing and its symbol; a message field also loses
    // long opaque tokens and extra whitespace. Both are bounded before any
    // scan and cut by character, never inside one.
    let frames = exception.callStackSymbols.prefix(maxFrames).map { frame in
      CrashRedaction.truncate(CrashRedaction.redactLine(frame), limit: maxFrame)
    }
    let message = redact(exception.reason ?? exception.name.rawValue)
    let record: [String: Any] = [
      "schema": 1,
      "capturedAt": timestamp(),
      "appVersion": Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "unknown",
      "platform": "ios",
      "architecture": architecture(),
      "phase": "application",
      "operation": "native-uncaught-exception",
      "thread": CrashRedaction.truncate(redact(Thread.current.name ?? "unnamed"), limit: 64),
      "message": message,
      "location": CrashRedaction.truncate(redact(exception.name.rawValue), limit: maxFrame),
      "stack": Array(frames),
    ]
    guard var data = try? JSONSerialization.data(
      withJSONObject: record,
      options: [.prettyPrinted, .sortedKeys]
    ) else { return }
    if data.count > maxRecordBytes {
      let fallback: [String: Any] = [
        "schema": 1,
        "capturedAt": timestamp(),
        "appVersion": Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "unknown",
        "platform": "ios",
        "architecture": architecture(),
        "phase": "application",
        "operation": "native-uncaught-exception",
        "message": "[record exceeded size limit]",
        "stack": [],
      ]
      data = (try? JSONSerialization.data(
        withJSONObject: fallback,
        options: [.prettyPrinted, .sortedKeys]
      )) ?? Data()
    }
    guard let support = FileManager.default.urls(
      for: .applicationSupportDirectory,
      in: .userDomainMask
    ).first else { return }
    let identifier = Bundle.main.bundleIdentifier ?? "org.kiwix.kiwixworkbench"
    let directory = support
      .appendingPathComponent(identifier, isDirectory: true)
      .appendingPathComponent(recordDirectory, isDirectory: true)
    try? FileManager.default.createDirectory(
      at: directory,
      withIntermediateDirectories: true,
      attributes: [.posixPermissions: 0o700]
    )
    let temp = directory.appendingPathComponent(tempFile)
    let pending = directory.appendingPathComponent(recordFile)
    let previous = directory.appendingPathComponent(previousFile)
    do {
      try data.write(to: temp, options: .atomic)
      try? FileManager.default.setAttributes(
        [.posixPermissions: 0o600],
        ofItemAtPath: temp.path
      )
      if FileManager.default.fileExists(atPath: pending.path) {
        try? FileManager.default.removeItem(at: previous)
        try? FileManager.default.moveItem(at: pending, to: previous)
      }
      try? FileManager.default.removeItem(at: pending)
      try FileManager.default.moveItem(at: temp, to: pending)
    } catch {
      // Never replace the original crash with a reporter failure.
    }
  }

  private static func timestamp() -> String {
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return formatter.string(from: Date())
  }

  private static func architecture() -> String {
    #if arch(arm64)
    return "aarch64"
    #elseif arch(x86_64)
    return "x86_64"
    #else
    return "unknown"
    #endif
  }

  /// Redaction of one text field of the record; see `CrashRedaction`.
  static func redact(_ value: String) -> String {
    CrashRedaction.redact(value)
  }
}
