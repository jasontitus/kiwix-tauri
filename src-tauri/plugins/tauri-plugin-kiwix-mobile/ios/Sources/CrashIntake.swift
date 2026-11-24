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
  private static let maxMessage = 2_000
  private static let maxFrames = 24
  private static let maxFrame = 240
  private static let maxRecordBytes = 32 * 1024
  private static let recordDirectory = "crash"
  private static let recordFile = "pending.json"
  private static let previousFile = "previous.json"
  private static let tempFile = ".pending.json.tmp"
  // Compiled once: the redaction helper runs per field and per stack frame.
  private static let urlPattern = try! NSRegularExpression(
    pattern: #"(?i)(?<![a-z0-9+.-])(?:[a-z][a-z0-9+.-]*://|(?:blob|content|data|file|mailto|tauri|tel|zim):(?!:))[^\s"<>`]*"#)
  private static let bearerPattern = try! NSRegularExpression(pattern: #"(?i)\bbearer\s+\S+"#)
  private static let secretPattern = try! NSRegularExpression(
    pattern: #"(?i)\b(authorization|access_token|refresh_token|api_key|apikey|password|passwd|cookie|token|secret|pin)\s*[:=]\s*("[^"]*"|'[^']*'|[^\s,;&)\]}]+)"#)
  private static let pathPattern = try! NSRegularExpression(
    pattern: #"(?<![^\s=(\[{"',;:`])(?:/(?!/)|[a-zA-Z]:[\\/]|~/|\\\\)[^\s"<>`]+"#)
  private static let tokenPattern = try! NSRegularExpression(pattern: #"\b[A-Za-z0-9_+/=-]{40,}\b"#)

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
    let frames = exception.callStackSymbols.prefix(maxFrames).map { frame in
      String(redact(frame).prefix(maxFrame))
    }
    let message = String(redact(exception.reason ?? exception.name.rawValue).prefix(maxMessage))
    let record: [String: Any] = [
      "schema": 1,
      "capturedAt": timestamp(),
      "appVersion": Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "unknown",
      "platform": "ios",
      "architecture": architecture(),
      "phase": "application",
      "operation": "native-uncaught-exception",
      "thread": String(redact(Thread.current.name ?? "unnamed").prefix(64)),
      "message": message,
      "location": String(redact(exception.name.rawValue).prefix(maxFrame)),
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

  static func redact(_ value: String) -> String {
    var text = replacing(value, urlPattern) { match in
      safeURL(match)
    }
    text = replacing(text, bearerPattern) { _ in "bearer [redacted]" }
    text = replacing(text, secretPattern) { match in
      let key = String(match.prefix { $0 != "=" && $0 != ":" })
        .trimmingCharacters(in: .whitespaces)
      return "\(key)=[redacted]"
    }
    text = replacing(text, pathPattern) { _ in "[path]" }
    text = replacing(text, tokenPattern) { _ in "[redacted]" }
    return text.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
  }

  private static func replacing(
    _ text: String,
    _ regex: NSRegularExpression,
    with transform: (String) -> String
  ) -> String {
    let range = NSRange(text.startIndex..<text.endIndex, in: text)
    var result = ""
    var cursor = text.startIndex
    regex.enumerateMatches(in: text, range: range) { match, _, _ in
      guard let match = match, let matchRange = Range(match.range, in: text) else { return }
      result.append(contentsOf: text[cursor..<matchRange.lowerBound])
      result += transform(String(text[matchRange]))
      cursor = matchRange.upperBound
    }
    result.append(contentsOf: text[cursor...])
    return result
  }

  private static func safeURL(_ raw: String) -> String {
    guard let components = URLComponents(string: raw), let scheme = components.scheme?.lowercased(),
          scheme == "http" || scheme == "https", let host = components.host, !host.isEmpty else {
      return "[url]"
    }
    let hostname = host.lowercased().trimmingCharacters(in: CharacterSet(charactersIn: "[]."))
    let loopback = hostname == "localhost" || hostname.hasSuffix(".localhost")
      || (hostname.hasPrefix("127.") && hostname.utf8.allSatisfy { $0 == 46 || (48...57).contains($0) })
      || (hostname.contains(":") && hostname.hasSuffix("1") && hostname.dropLast().allSatisfy { $0 == ":" || $0 == "0" })
    let path = components.path
    let nested = path.contains("://")
      || ["mailto:", "data:", "file:", "content:", "zim:", "blob:", "tauri:", "tel:"].contains { path.contains($0) }
    if loopback || path.contains("/.kiwix/") || nested {
      return "[reader-url]"
    }
    let port = components.port.map { ":\($0)" } ?? ""
    return "\(scheme)://\(host)\(port)\(components.percentEncodedPath)"
  }
}
