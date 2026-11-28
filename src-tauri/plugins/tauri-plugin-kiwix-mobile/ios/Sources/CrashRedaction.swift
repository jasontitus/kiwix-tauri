// SPDX-License-Identifier: GPL-3.0-or-later
//
// Redaction of crash-record text: the Swift implementation of
// `src-tauri/tests/fixtures/crash-redaction-spec.md`, pinned to the vectors in
// `crash-redaction-cases.json` next to it. `src-tauri/src/crash.rs` and the
// Android intake follow the same text; none of them may change alone.
//
// The steps, in order: bound the input, secret values, URLs, paths, long
// opaque tokens, collapse whitespace, cut. Every index, count, scan and cut is
// over Unicode scalar values, never `Character`: a `Character` can hold
// several scalars, and "\r\n" is one. Case is folded for ASCII letters only.
// The text is scanned as an array of scalars with plain loops, each step in
// time proportional to its input; there is no pattern engine whose limits
// could let a long line through unredacted, and no platform URL parser whose
// reading could differ between systems.
enum CrashRedaction {
  /// Longest input any step looks at. The bound comes first, so no later scan
  /// sees more than this.
  static let maxInput = 32_768
  static let maxMessage = 2_000

  private typealias Text = [Unicode.Scalar]

  private static let redactedMark = Text("[redacted]".unicodeScalars)
  private static let pathMark = Text("[path]".unicodeScalars)
  private static let urlMark = Text("[url]".unicodeScalars)
  private static let readerURLMark = Text("[reader-url]".unicodeScalars)

  /// Full redaction for one text field: steps 1 to 7.
  static func redact(_ value: String) -> String {
    var text = bounded(value)
    text = redactSecrets(text)
    text = replaceURLs(text)
    text = redactPaths(text)
    text = redactOpaqueTokens(text)
    return string(truncated(collapseWhitespace(text), limit: maxMessage))
  }

  /// Redaction for one stack frame line: steps 1 to 4, keeping its spacing.
  static func redactLine(_ value: String) -> String {
    var text = bounded(value)
    text = redactSecrets(text)
    text = replaceURLs(text)
    return string(redactPaths(text))
  }

  /// At most `limit` scalars; a longer value ends in an ellipsis.
  static func truncate(_ value: String, limit: Int) -> String {
    string(truncated(Text(value.unicodeScalars), limit: limit))
  }

  private static func string(_ text: Text) -> String {
    var view = String.UnicodeScalarView()
    view.append(contentsOf: text)
    return String(view)
  }

  private static func truncated(_ text: Text, limit: Int) -> Text {
    if text.count <= limit { return text }
    var cut = Text(text.prefix(max(limit - 1, 0)))
    cut.append("\u{2026}")
    return cut
  }

  // MARK: Step 1, bound

  /// The first `maxInput` scalars. When that cuts through a run of
  /// non-whitespace the partial run goes too: half a host name or half a path
  /// must not be judged as if it were whole.
  private static func bounded(_ value: String) -> Text {
    var text = Text()
    var rest = value.unicodeScalars.makeIterator()
    while text.count < maxInput, let scalar = rest.next() { text.append(scalar) }
    if let dropped = rest.next(), !isWhitespace(dropped) {
      while let last = text.last, !isWhitespace(last) { text.removeLast() }
    }
    return text
  }

  // MARK: Terms

  /// Unicode White_Space, spelled out: no platform function is trusted to
  /// agree with this list.
  private static func isWhitespace(_ scalar: Unicode.Scalar) -> Bool {
    switch scalar.value {
    case 0x09...0x0D, 0x20, 0x85, 0xA0, 0x1680, 0x2000...0x200A, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000:
      return true
    default:
      return false
    }
  }

  private static func isLineBreak(_ scalar: Unicode.Scalar) -> Bool {
    scalar == "\n" || scalar == "\r"
  }

  private static func isASCIIUppercase(_ scalar: Unicode.Scalar) -> Bool {
    (0x41...0x5A).contains(scalar.value)
  }

  private static func isASCIILowercase(_ scalar: Unicode.Scalar) -> Bool {
    (0x61...0x7A).contains(scalar.value)
  }

  private static func isASCIILetter(_ scalar: Unicode.Scalar) -> Bool {
    isASCIIUppercase(scalar) || isASCIILowercase(scalar)
  }

  private static func isASCIIDigit(_ scalar: Unicode.Scalar) -> Bool {
    (0x30...0x39).contains(scalar.value)
  }

  private static func isASCIIAlphanumeric(_ scalar: Unicode.Scalar) -> Bool {
    isASCIILetter(scalar) || isASCIIDigit(scalar)
  }

  /// ASCII letters only: `ſ` is not `s` and the Kelvin sign is not `k`.
  private static func lowercasedASCII(_ scalar: Unicode.Scalar) -> Unicode.Scalar {
    isASCIIUppercase(scalar) ? Unicode.Scalar(UInt8(scalar.value + 0x20)) : scalar
  }

  /// The scalar at `index`, or nil past either end.
  private static func scalar(_ chars: Text, _ index: Int) -> Unicode.Scalar? {
    index >= 0 && index < chars.count ? chars[index] : nil
  }

  /// The first index at or after `from` whose scalar is not whitespace.
  private static func skipWhitespace(_ chars: Text, from: Int) -> Int {
    var index = from
    while index < chars.count, isWhitespace(chars[index]) { index += 1 }
    return index
  }

  /// The number of backslashes directly before `index`, counted back no
  /// further than `floor`.
  private static func backslashesBefore(_ chars: Text, floor: Int, index: Int) -> Int {
    var position = index
    while position > floor, chars[position - 1] == "\\" { position -= 1 }
    return index - position
  }

  private static func hasPathSeparator(_ chars: Text, _ range: Range<Int>) -> Bool {
    for index in range where chars[index] == "/" || chars[index] == "\\" { return true }
    return false
  }

  /// Where a path-like range that begins at `start` stops (specification
  /// 1.1). Names contain spaces and apostrophes, so neither ends it. It ends
  /// before a line break, an angle bracket, a backtick, a `"` that is not
  /// escaped, or the `: ` that introduces an error's cause; words in between
  /// go with it. The whitespace before that stop is not part of the range.
  private static func pathEnd(_ chars: Text, _ start: Int) -> Int {
    var end = start
    scan: while end < chars.count {
      switch chars[end] {
      case "\n", "\r", "<", ">", "`":
        break scan
      case "\"":
        if backslashesBefore(chars, floor: start, index: end) % 2 == 0 { break scan }
      case ":":
        if end + 1 < chars.count, isWhitespace(chars[end + 1]) { break scan }
      default:
        break
      }
      end += 1
    }
    while end > start, isWhitespace(chars[end - 1]) { end -= 1 }
    return end
  }

  // MARK: Step 2, secrets

  private static let secretKeys: [(name: String, scalars: Text)] = [
    "authorization", "access_token", "refresh_token", "api_key", "api-key", "apikey",
    "password", "passwd", "passphrase", "pass", "pwd", "private_key", "private-key", "privatekey",
    "client_secret", "secret", "session_id", "sessionid", "jsessionid", "credentials", "credential",
    "signature", "cookie", "bearer", "token", "auth", "pin",
  ].map { (name: $0, scalars: Text($0.unicodeScalars)) }

  /// Line ends and the last apostrophe of each line, computed once so that no
  /// secret value rescans its line.
  private struct Lines {
    /// For each index, and for the text length: the first line break at or
    /// after it, or the text length.
    var end: [Int]
    /// For each index: the last `'` on its line, or -1.
    var lastApostrophe: [Int]

    init(_ chars: Text) {
      end = [Int](repeating: chars.count, count: chars.count + 1)
      lastApostrophe = [Int](repeating: -1, count: chars.count + 1)
      var lineEnd = chars.count
      var apostrophe = -1
      var index = chars.count
      while index > 0 {
        index -= 1
        if isLineBreak(chars[index]) {
          lineEnd = index
          apostrophe = -1
        } else if chars[index] == "'", apostrophe < 0 {
          apostrophe = index
        }
        end[index] = lineEnd
        lastApostrophe[index] = apostrophe
      }
    }
  }

  private static func redactSecrets(_ chars: Text) -> Text {
    let lines = Lines(chars)
    var output = Text()
    output.reserveCapacity(chars.count)
    var cursor = 0
    while cursor < chars.count {
      if let value = secretAt(chars, lines, cursor) {
        output.append(contentsOf: chars[cursor..<value.lowerBound])
        output.append(contentsOf: redactedMark)
        cursor = value.upperBound
        continue
      }
      output.append(chars[cursor])
      cursor += 1
    }
    return output
  }

  /// The value range of a secret whose keyword starts at `at`, if any.
  private static func secretAt(_ chars: Text, _ lines: Lines, _ at: Int) -> Range<Int>? {
    let first = chars[at]
    guard isASCIILetter(first) else { return nil }
    // Left side: not inside a longer word, except at a camelCase hump
    // (`accessToken`, `oauth2Token`).
    if at > 0 {
      let previous = chars[at - 1]
      let hump = isASCIIUppercase(first) && (isASCIILowercase(previous) || isASCIIDigit(previous))
      guard !isASCIIAlphanumeric(previous) || hump else { return nil }
    }
    for key in secretKeys {
      let afterKey = at + key.scalars.count
      if afterKey > chars.count { continue }
      var matches = true
      for offset in 0..<key.scalars.count where lowercasedASCII(chars[at + offset]) != key.scalars[offset] {
        matches = false
        break
      }
      if !matches { continue }
      if afterKey < chars.count, isASCIIAlphanumeric(chars[afterKey]) || chars[afterKey] == "_" { continue }
      // A quoted key ends with its quote, which an outer string may have
      // escaped: `"password":`, `\"password\":`, `("authorization", …)`.
      var next = afterKey
      let quote = scalar(chars, afterKey) == "\\" ? afterKey + 1 : afterKey
      let quotedKey = scalar(chars, quote) == "\"" || scalar(chars, quote) == "'"
      if quotedKey { next = quote + 1 }
      let gap = next
      next = skipWhitespace(chars, from: next)
      var separator = 0
      switch scalar(chars, next) {
      case "=":
        separator = scalar(chars, next + 1) == ">" ? 2 : 1
      case ":":
        // `::` separates Rust path segments, not a key from its value.
        if scalar(chars, next + 1) != ":" { separator = 1 }
      case ",":
        // Tuples and pairs: `("token", "…")`, `(authorization, Basic …)`.
        if quotedKey || key.name == "authorization" { separator = 1 }
      default:
        break
      }
      if separator > 0 {
        next = skipWhitespace(chars, from: next + separator)
      } else if quotedKey || next == gap || !(key.name == "authorization" || key.name == "bearer") {
        continue
      }
      let value = secretValue(chars, lines, next, key: key.name)
      if value.upperBound > value.lowerBound { return value }
    }
    return nil
  }

  /// The range of the secret value that begins at `start`: quoted (possibly
  /// inside wrappers such as `Some(`), the rest of the line after `cookie`,
  /// or one token.
  private static func secretValue(_ chars: Text, _ lines: Lines, _ start: Int, key: String) -> Range<Int> {
    if let quoted = quotedValue(chars, lines, open: skipWrappers(chars, start)) { return quoted }
    if key == "cookie" {
      // One header carries several `name=value` pairs.
      return start..<lines.end[start]
    }
    func tokenEnd(_ from: Int) -> Int {
      var index = from
      while index < chars.count {
        let scalar = chars[index]
        if isWhitespace(scalar) || scalar == "," || scalar == ";" || scalar == "&"
            || scalar == ")" || scalar == "]" || scalar == "}" { break }
        index += 1
      }
      return index
    }
    var end = tokenEnd(start)
    if key == "authorization", end > start {
      // `Basic dXNlcjpwYXNz`, `Bearer …`: the scheme and the credential.
      let second = skipWhitespace(chars, from: end)
      if second > end {
        let secondEnd = tokenEnd(second)
        if secondEnd > second { end = secondEnd }
      }
    }
    // A value with a separator is a path or a URL and may contain spaces.
    if hasPathSeparator(chars, start..<end) { end = max(end, pathEnd(chars, start)) }
    return start..<end
  }

  /// The index after any `Name(` wrappers that begin at `start`.
  private static func skipWrappers(_ chars: Text, _ start: Int) -> Int {
    var position = start
    while position < chars.count, isASCIILetter(chars[position]) {
      var nameEnd = position
      while nameEnd < chars.count, isASCIIAlphanumeric(chars[nameEnd]) || chars[nameEnd] == "_" { nameEnd += 1 }
      guard scalar(chars, nameEnd) == "(" else { break }
      position = nameEnd + 1
    }
    return position
  }

  /// The content range of a quoted value whose opening quote, or the
  /// backslash that escapes it, is at `open`. The quotes stay.
  private static func quotedValue(_ chars: Text, _ lines: Lines, open: Int) -> Range<Int>? {
    let escaped = scalar(chars, open) == "\\"
    let quote = escaped ? open + 1 : open
    if scalar(chars, quote) == "\"" {
      // `"…"` ends at a quote that is not escaped; `\"…\"`, as inside an
      // outer string, at the next quote with exactly one backslash before
      // it. Either way the line end is the limit.
      let start = quote + 1
      let line = lines.end[start]
      var close = start
      while close < line {
        if chars[close] == "\"" {
          let before = backslashesBefore(chars, floor: start, index: close)
          if escaped ? before == 1 : before % 2 == 0 { break }
        }
        close += 1
      }
      // The backslash that escapes the closing quote stays with it.
      return start..<(close < line && escaped ? close - 1 : close)
    }
    if scalar(chars, quote) == "'", !escaped {
      // Up to the last `'` on the line, because a value may contain an
      // apostrophe; else the rest of the line.
      let start = quote + 1
      var end = lines.lastApostrophe[start] >= start ? lines.lastApostrophe[start] : lines.end[start]
      // The closing quote may be missing, and then the last `'` is an
      // apostrophe inside a path.
      if hasPathSeparator(chars, start..<end) { end = max(end, pathEnd(chars, start)) }
      return start..<end
    }
    return nil
  }

  // MARK: Step 3, URLs

  private static let opaqueSchemes: [Text] = ["blob", "content", "data", "file", "mailto", "tauri", "tel", "zim"]
    .map { Text($0.unicodeScalars) }
  private static let fileScheme = Text("file".unicodeScalars)
  private static let dataScheme = Text("data".unicodeScalars)
  /// Final labels of host names that only mean something on the person's own
  /// network, where the name itself is theirs (`janes-macbook.local`).
  private static let privateSuffixes: [Text] = ["local", "localdomain", "lan", "home", "internal", "arpa"]
    .map { Text($0.unicodeScalars) }

  private static func replaceURLs(_ chars: Text) -> Text {
    var output = Text()
    output.reserveCapacity(chars.count)
    var cursor = 0
    var colon = 0
    while colon < chars.count {
      guard chars[colon] == ":" else {
        colon += 1
        continue
      }
      // The scheme is the run of scheme scalars before the colon, from its
      // first letter (`+https:` is still `https:`). It cannot reach back into
      // text that an earlier URL already replaced.
      var start = colon
      while start > cursor {
        let previous = chars[start - 1]
        guard isASCIIAlphanumeric(previous) || previous == "+" || previous == "-" || previous == "." else { break }
        start -= 1
      }
      while start < colon, !isASCIILetter(chars[start]) { start += 1 }
      let tail = colon + 1
      let slashes = scalar(chars, tail) == "/" && scalar(chars, tail + 1) == "/"
      let scheme = chars[start..<colon].map(lowercasedASCII)
      if start == colon || (!slashes && (scalar(chars, tail) == ":" || !opaqueSchemes.contains(scheme))) {
        colon += 1
        continue
      }
      var tokenEnd = tail
      while tokenEnd < chars.count {
        let scalar = chars[tokenEnd]
        if isWhitespace(scalar) || scalar == "\"" || scalar == "<" || scalar == ">" || scalar == "`" { break }
        tokenEnd += 1
      }
      let hasBody = tail < chars.count && !isWhitespace(chars[tail])
      var end = tokenEnd
      var replacement = urlMark
      if scheme == fileScheme || scheme == dataScheme {
        // A file URL is a path, in any form (`file:///x`, `file:C:/x`), and
        // may be written with unencoded spaces. A data URL carries content
        // itself, spaces and markup included: the rest of the line goes.
        if !hasBody {
          end = tail
        } else if scheme == fileScheme {
          end = pathEnd(chars, tail)
        } else {
          end = tail
          while end < chars.count, !isLineBreak(chars[end]) { end += 1 }
        }
      } else if slashes {
        var authorityEnd = tail + 2
        while authorityEnd < tokenEnd {
          let scalar = chars[authorityEnd]
          if scalar == "/" || scalar == "\\" || scalar == "?" || scalar == "#" { break }
          authorityEnd += 1
        }
        replacement = classify(scheme: scheme, authority: chars[(tail + 2)..<authorityEnd])
        // A URL that is not kept, or whose path, query or fragment is
        // dropped, goes whole, like a path: what follows the first space of
        // an unencoded article title must not stay behind.
        if replacement.first == "[" || authorityEnd < tokenEnd { end = max(tokenEnd, pathEnd(chars, tail)) }
      }
      output.append(contentsOf: chars[cursor..<start])
      output.append(contentsOf: replacement)
      cursor = end
      colon = end
    }
    output.append(contentsOf: chars[cursor...])
    return output
  }

  /// What may be kept of a `scheme://authority…` URL, decided from its text
  /// alone so that every platform decides alike.
  ///
  /// * `http` and `https` only; any other scheme is `[url]`.
  /// * `[reader-url]`: `localhost`, a name ending in `.localhost`, `127.`
  ///   followed only by digits and dots, or `[::1]`. These are archive pages.
  /// * Kept as `scheme://host[:port]`: a DNS name of at least two labels of
  ///   `a-z 0-9 -` whose last label starts with a letter and is not a
  ///   private-network suffix. The path, query and fragment are never kept: a
  ///   path can name the article being read.
  /// * Everything else is `[url]`: IP literals in any spelling, single-label
  ///   hosts, private-network names, a malformed port.
  private static func classify(scheme: Text, authority: ArraySlice<Unicode.Scalar>) -> Text {
    let http = Text("http".unicodeScalars)
    guard scheme == http || scheme == Text("https".unicodeScalars) else { return urlMark }
    // Userinfo ends at the last `@`.
    var rest = authority
    if let at = rest.lastIndex(of: "@") { rest = rest[(at + 1)...] }
    var hostPart = rest
    var portPart = rest[rest.endIndex...]
    if rest.first == "[" {
      guard let close = rest.firstIndex(of: "]") else { return urlMark }
      hostPart = rest[...close]
      portPart = rest[(close + 1)...]
    } else if let colon = rest.lastIndex(of: ":") {
      hostPart = rest[..<colon]
      portPart = rest[colon...]
    }
    var host = hostPart.map(lowercasedASCII)
    if host.last == "." { host.removeLast() }
    let localhost = Text("localhost".unicodeScalars)
    let dotLocalhost = Text(".localhost".unicodeScalars)
    let loopbackPrefix = Text("127.".unicodeScalars)
    let loopback = host.starts(with: loopbackPrefix)
      && host.dropFirst(loopbackPrefix.count).allSatisfy { isASCIIDigit($0) || $0 == "." }
    if host == localhost || host.suffix(dotLocalhost.count) == dotLocalhost[...]
        || loopback || host == Text("[::1]".unicodeScalars) {
      return readerURLMark
    }
    var port = Text()
    if !portPart.isEmpty {
      let digits = portPart.dropFirst()
      guard (1...5).contains(digits.count), digits.allSatisfy(isASCIIDigit) else { return urlMark }
      port = Text(digits)
    }
    // A DNS name: at least two labels, each non-empty and made of `a-z`,
    // `0-9` and `-`; the last one starts with a letter and is not private.
    let labels = host.split(separator: ".", omittingEmptySubsequences: false)
    guard labels.count >= 2, let last = labels.last, let lastFirst = last.first, isASCIILowercase(lastFirst),
          !privateSuffixes.contains(Text(last)) else { return urlMark }
    for label in labels {
      guard !label.isEmpty, label.allSatisfy({ isASCIILowercase($0) || isASCIIDigit($0) || $0 == "-" }) else {
        return urlMark
      }
    }
    var kept = scheme + Text("://".unicodeScalars) + host
    // The default port is dropped, whatever its leading zeros.
    let defaultPort = Text((scheme == http ? "80" : "443").unicodeScalars)
    if !port.isEmpty, Text(port.drop(while: { $0 == "0" })) != defaultPort {
      kept.append(":")
      kept.append(contentsOf: port)
    }
    return kept
  }

  // MARK: Step 4, paths

  private static func redactPaths(_ chars: Text) -> Text {
    var output = Text()
    output.reserveCapacity(chars.count)
    var index = 0
    while index < chars.count {
      if let end = pathAt(chars, index) {
        output.append(contentsOf: pathMark)
        index = end
        continue
      }
      output.append(chars[index])
      index += 1
    }
    return output
  }

  /// A drive (`C:\`, `C:/`), an absolute path (`/x`), a home path (`~/`,
  /// `~\`), a share (`\\`, `//host`) or a rooted Windows path (`\Users\…`).
  private static func isPathStart(_ chars: Text, _ index: Int) -> Bool {
    func isName(_ scalar: Unicode.Scalar?) -> Bool {
      guard let scalar = scalar else { return false }
      return scalar != "/" && !isWhitespace(scalar)
    }
    let current = chars[index]
    let next = scalar(chars, index + 1)
    let third = scalar(chars, index + 2)
    if isASCIILetter(current), next == ":", third == "\\" || third == "/" { return true }
    if current == "/", isName(next) { return true }
    if current == "~", next == "/" || next == "\\" { return true }
    if current == "\\", next == "\\" { return true }
    if current == "/", next == "/", isName(third) { return true }
    // One backslash and a name is a path only if another backslash follows
    // before the next whitespace; `\n` in prose is not. The scan ends at that
    // backslash, so it never passes the next candidate.
    if current == "\\", let next = next, isASCIIAlphanumeric(next) {
      var position = index + 1
      while position < chars.count, !isWhitespace(chars[position]) {
        if chars[position] == "\\" { return true }
        position += 1
      }
    }
    return false
  }

  private static func pathAt(_ chars: Text, _ index: Int) -> Int? {
    // A path may begin anywhere but directly after an ASCII letter, an ASCII
    // digit, `_`, `.` or `-`.
    let previous = scalar(chars, index - 1)
    if let previous = previous, isASCIIAlphanumeric(previous) || previous == "_" || previous == "." || previous == "-" {
      return nil
    }
    guard isPathStart(chars, index) else { return nil }
    // Neither slash of `://` starts a path: what follows is the host of a URL
    // that step 3 kept (`https://example.org`).
    if chars[index] == "/" {
      let first = previous == ":" && scalar(chars, index + 1) == "/"
      let second = previous == "/" && scalar(chars, index - 2) == ":"
      if first || second { return nil }
    }
    return pathEnd(chars, index)
  }

  // MARK: Step 5, opaque tokens

  private static func redactOpaqueTokens(_ chars: Text) -> Text {
    var output = Text()
    output.reserveCapacity(chars.count)
    var runStart = 0
    func flush(_ runEnd: Int) {
      let run = chars[runStart..<runEnd]
      if run.count >= 40, run.contains(where: isASCIIDigit), run.contains(where: isASCIILetter) {
        output.append(contentsOf: redactedMark)
      } else {
        output.append(contentsOf: run)
      }
    }
    for (index, scalar) in chars.enumerated() {
      if isASCIIAlphanumeric(scalar) || scalar == "_" || scalar == "-" || scalar == "+"
          || scalar == "/" || scalar == "=" || scalar == "." { continue }
      flush(index)
      output.append(scalar)
      runStart = index + 1
    }
    flush(chars.count)
    return output
  }

  // MARK: Step 6, whitespace

  private static func collapseWhitespace(_ chars: Text) -> Text {
    var output = Text()
    output.reserveCapacity(chars.count)
    var pendingSpace = false
    for scalar in chars {
      if isWhitespace(scalar) {
        pendingSpace = !output.isEmpty
        continue
      }
      if pendingSpace {
        output.append(" ")
        pendingSpace = false
      }
      output.append(scalar)
    }
    return output
  }
}
