// SPDX-License-Identifier: GPL-3.0-or-later
//
// Redaction of crash-record text. This is a port of `redact` and `redact_line`
// in `src-tauri/src/crash.rs` and must change only together with them: both
// are pinned to `src-tauri/tests/fixtures/crash-redaction-cases.json`.
//
// The steps, in order: bound the input, secret values, URLs, paths, long
// opaque tokens, collapse whitespace, cut. A "character" is a Unicode scalar
// value, as in Rust, so a bound or a cut never splits a surrogate pair. The
// text is scanned as an array of scalars with plain loops; there is no pattern
// engine whose limits could let a long line through unredacted.
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

  /// Full redaction for one message field: all seven steps.
  static func redact(_ value: String) -> String {
    var text = Text(value.unicodeScalars.prefix(maxInput))
    text = redactSecrets(text)
    text = replaceURLs(text)
    text = redactPaths(text)
    text = redactOpaqueTokens(text)
    return string(truncated(collapseWhitespace(text), limit: maxMessage))
  }

  /// Redaction for one stack frame line: steps one to four, keeping spacing.
  static func redactLine(_ value: String) -> String {
    var text = Text(value.unicodeScalars.prefix(maxInput))
    text = redactSecrets(text)
    text = replaceURLs(text)
    return string(redactPaths(text))
  }

  /// At most `limit` characters; a longer value ends in an ellipsis.
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
    return Text(text.prefix(max(limit - 1, 0))) + ["\u{2026}"]
  }

  // MARK: Character classes

  /// Unicode White_Space, spelled out so every platform agrees with Rust's
  /// `char::is_whitespace`.
  private static func isWhitespace(_ scalar: Unicode.Scalar) -> Bool {
    switch scalar.value {
    case 0x09...0x0D, 0x20, 0x85, 0xA0, 0x1680, 0x2000...0x200A, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000:
      return true
    default:
      return false
    }
  }

  private static func isASCIILetter(_ scalar: Unicode.Scalar) -> Bool {
    (0x41...0x5A).contains(scalar.value) || (0x61...0x7A).contains(scalar.value)
  }

  private static func isASCIIDigit(_ scalar: Unicode.Scalar) -> Bool {
    (0x30...0x39).contains(scalar.value)
  }

  private static func isASCIIAlphanumeric(_ scalar: Unicode.Scalar) -> Bool {
    isASCIILetter(scalar) || isASCIIDigit(scalar)
  }

  private static func lowercasedASCII(_ scalar: Unicode.Scalar) -> Unicode.Scalar {
    (0x41...0x5A).contains(scalar.value) ? Unicode.Scalar(UInt8(scalar.value + 0x20)) : scalar
  }

  private static func isLineBreak(_ scalar: Unicode.Scalar) -> Bool {
    scalar == "\n" || scalar == "\r"
  }

  /// Space, tab or a line break: the gaps the secret rules skip.
  private static func isGap(_ scalar: Unicode.Scalar) -> Bool {
    scalar == " " || scalar == "\t" || isLineBreak(scalar)
  }

  /// Whether the character at `index` follows an odd run of backslashes that
  /// begins no earlier than `floor`.
  private static func backslashEscaped(_ chars: Text, floor: Int, index: Int) -> Bool {
    var run = 0
    var position = index
    while position > floor, chars[position - 1] == "\\" {
      run += 1
      position -= 1
    }
    return run % 2 == 1
  }

  private static func lineEnd(_ chars: Text, from: Int) -> Int {
    var index = from
    while index < chars.count, !isLineBreak(chars[index]) { index += 1 }
    return index
  }

  // MARK: Step 2, secret values

  private static let secretKeys: [(name: String, scalars: Text)] = [
    "authorization", "access_token", "refresh_token", "api_key", "api-key", "apikey",
    "password", "passwd", "cookie", "bearer", "token", "secret", "pin",
  ].map { (name: $0, scalars: Text($0.unicodeScalars)) }

  private static func redactSecrets(_ chars: Text) -> Text {
    var output = Text()
    output.reserveCapacity(chars.count)
    var cursor = 0
    while cursor < chars.count {
      if let value = secretAt(chars, cursor) {
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
  private static func secretAt(_ chars: Text, _ at: Int) -> Range<Int>? {
    if at > 0, isASCIIAlphanumeric(chars[at - 1]) { return nil }
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
      var next = afterKey
      // `"password":` — the closing quote of a quoted key.
      let quotedKey = next < chars.count && (chars[next] == "\"" || chars[next] == "'")
      if quotedKey { next += 1 }
      let gap = next
      while next < chars.count, isGap(chars[next]) { next += 1 }
      var separator = false
      if next < chars.count {
        if chars[next] == "=" {
          separator = true
        } else if chars[next] == ":" {
          // `::` separates Rust path segments; it does not introduce a value.
          separator = !(next + 1 < chars.count && chars[next + 1] == ":")
        }
      }
      if separator {
        next += 1
        while next < chars.count, chars[next] == " " || chars[next] == "\t" { next += 1 }
      } else if quotedKey || next == gap || !(key.name == "authorization" || key.name == "bearer") {
        continue
      }
      let value = secretValue(chars, next, key: key.name)
      if value.upperBound > value.lowerBound { return value }
    }
    return nil
  }

  /// The range of the secret value that begins at `start`.
  private static func secretValue(_ chars: Text, _ start: Int, key: String) -> Range<Int> {
    func tokenEnd(_ from: Int) -> Int {
      var index = from
      while index < chars.count {
        let scalar = chars[index]
        if isGap(scalar) || scalar == "," || scalar == ";" || scalar == "&"
            || scalar == ")" || scalar == "]" || scalar == "}" { break }
        index += 1
      }
      return index
    }
    func hasPathSeparator(_ range: Range<Int>) -> Bool {
      for index in range where chars[index] == "/" || chars[index] == "\\" { return true }
      return false
    }
    if start < chars.count, chars[start] == "\"" {
      // Up to the closing quote (one not escaped by a backslash), else the
      // rest of the line. The quotes stay.
      var close = start + 1
      while close < chars.count, !isLineBreak(chars[close]) {
        if chars[close] == "\"", !backslashEscaped(chars, floor: start + 1, index: close) { break }
        close += 1
      }
      return (start + 1)..<close
    }
    if start < chars.count, chars[start] == "'" {
      // Up to the last `'` on the line, because a value may contain an
      // apostrophe; else the rest of the line.
      let line = lineEnd(chars, from: start + 1)
      var end = line
      var index = line
      while index > start + 1 {
        index -= 1
        if chars[index] == "'" {
          end = index
          break
        }
      }
      // The closing quote may be missing, and then the last `'` is an
      // apostrophe inside a path.
      if hasPathSeparator((start + 1)..<end) { end = max(end, pathEnd(chars, start + 1)) }
      return (start + 1)..<end
    }
    if key == "cookie" {
      // One header carries several `name=value` pairs.
      return start..<lineEnd(chars, from: start)
    }
    var end = tokenEnd(start)
    if key == "authorization", end > start {
      // `Basic dXNlcjpwYXNz`, `Bearer …`: the scheme and the credential.
      var second = end
      while second < chars.count, isGap(chars[second]) { second += 1 }
      if second > end {
        let secondEnd = tokenEnd(second)
        if secondEnd > second { end = secondEnd }
      }
    }
    // A value with a separator is a path or a URL and may contain spaces.
    if hasPathSeparator(start..<end) { end = max(end, pathEnd(chars, start)) }
    return start..<end
  }

  // MARK: Step 3, URLs

  private static let opaqueSchemes: Set<String> = ["blob", "content", "data", "file", "mailto", "tauri", "tel", "zim"]

  private static func replaceURLs(_ chars: Text) -> Text {
    var output = Text()
    output.reserveCapacity(chars.count)
    var cursor = 0
    for colon in chars.indices where chars[colon] == ":" {
      if colon < cursor { continue }
      var start = colon
      while start > cursor {
        let previous = chars[start - 1]
        guard isASCIIAlphanumeric(previous) || previous == "+" || previous == "-" || previous == "." else { break }
        start -= 1
      }
      guard start < colon, isASCIILetter(chars[start]) else { continue }
      let scheme = string(chars[start..<colon].map(lowercasedASCII))
      let tail = colon + 1
      let authority = tail + 1 < chars.count && chars[tail] == "/" && chars[tail + 1] == "/"
      if !authority {
        if tail < chars.count, chars[tail] == ":" { continue }
        if !opaqueSchemes.contains(scheme) { continue }
      }
      var tokenEnd = tail
      while tokenEnd < chars.count {
        let scalar = chars[tokenEnd]
        if isWhitespace(scalar) || scalar == "\"" || scalar == "<" || scalar == ">" || scalar == "`" { break }
        tokenEnd += 1
      }
      let hasBody = tail < chars.count && !isWhitespace(chars[tail])
      let end: Int
      let replacement: Text
      if scheme == "file", hasBody {
        // A file URL is a path and may be written with unencoded spaces.
        // `file: ` followed by a space is prose; step 4 finds the path.
        end = pathEnd(chars, tail)
        replacement = urlMark
      } else if scheme == "data", hasBody {
        // A data URL carries content itself: the rest of the line goes.
        end = lineEnd(chars, from: tail)
        replacement = urlMark
      } else {
        replacement = safeURL(Text(chars[start..<tokenEnd]), scheme: scheme)
        // A `scheme://` URL that is not kept goes whole, like a path.
        if authority, replacement == urlMark || replacement == readerURLMark {
          end = max(tokenEnd, pathEnd(chars, tail))
        } else {
          end = tokenEnd
        }
      }
      output.append(contentsOf: chars[cursor..<start])
      output.append(contentsOf: replacement)
      cursor = end
    }
    output.append(contentsOf: chars[cursor...])
    return output
  }

  /// `[reader-url]` for a page served by this device, `[url]` for anything
  /// that is not a well-formed web address, and otherwise the address reduced
  /// to scheme, host, port and path.
  private static func safeURL(_ raw: Text, scheme: String) -> Text {
    guard scheme == "http" || scheme == "https", let url = WebAddress(raw, scheme: scheme) else { return urlMark }
    let decoded = String(decoding: percentDecoded(url.path), as: UTF8.self)
    let nested = decoded.contains("://")
      || ["mailto:", "data:", "file:", "content:", "zim:", "blob:", "tauri:", "tel:"].contains { decoded.contains($0) }
    if url.isLocal || decoded.contains("/.kiwix/") || nested { return readerURLMark }
    let port = url.port.map { ":\($0)" } ?? ""
    return Text("\(scheme)://\(url.host)\(port)".unicodeScalars) + url.path.map { Unicode.Scalar($0) }
  }

  private static func percentDecoded(_ bytes: [UInt8]) -> [UInt8] {
    func hex(_ byte: UInt8) -> UInt8? {
      switch byte {
      case 0x30...0x39: return byte - 0x30
      case 0x41...0x46: return byte - 0x41 + 10
      case 0x61...0x66: return byte - 0x61 + 10
      default: return nil
      }
    }
    var output: [UInt8] = []
    output.reserveCapacity(bytes.count)
    var index = 0
    while index < bytes.count {
      if bytes[index] == 0x25, index + 2 < bytes.count, let high = hex(bytes[index + 1]), let low = hex(bytes[index + 2]) {
        output.append(high << 4 | low)
        index += 3
      } else {
        output.append(bytes[index])
        index += 1
      }
    }
    return output
  }

  /// The parts of an `http` or `https` address the redaction needs, read the
  /// way the `url` crate reads them in `crash.rs`. An address this cannot read
  /// with certainty yields nil and is then replaced whole, never kept: a host
  /// with percent escapes, non-ASCII characters or a punycode label.
  private struct WebAddress {
    let host: String
    let port: Int?
    /// Normalized and percent-encoded; always starts with `/`.
    let path: [UInt8]
    let isLocal: Bool

    init?(_ raw: Text, scheme: String) {
      // Trailing control characters are not part of the address.
      var end = raw.count
      while end > 0, raw[end - 1].value <= 0x20 { end -= 1 }
      var index = scheme.unicodeScalars.count + 1
      while index < end, raw[index] == "/" || raw[index] == "\\" { index += 1 }
      // Credentials end at the last `@` before the path.
      var authorityEnd = index
      while authorityEnd < end {
        let scalar = raw[authorityEnd]
        if scalar == "/" || scalar == "\\" || scalar == "?" || scalar == "#" { break }
        authorityEnd += 1
      }
      for position in index..<authorityEnd where raw[position] == "@" { index = position + 1 }
      var hostEnd = index
      var bracketed = false
      while hostEnd < authorityEnd {
        let scalar = raw[hostEnd]
        if scalar == ":", !bracketed { break }
        if scalar == "[" { bracketed = true }
        if scalar == "]" { bracketed = false }
        hostEnd += 1
      }
      guard hostEnd > index, let host = WebAddress.host(Text(raw[index..<hostEnd])) else { return nil }
      self.host = host.text
      self.isLocal = host.isLocal
      index = hostEnd
      var port: Int?
      if index < authorityEnd {
        // Only a colon can end the host before the authority does.
        index += 1
        var value = 0
        var hasDigit = false
        while index < authorityEnd {
          guard isASCIIDigit(raw[index]) else { return nil }
          value = value * 10 + Int(raw[index].value - 0x30)
          guard value <= 65_535 else { return nil }
          hasDigit = true
          index += 1
        }
        if hasDigit, value != (scheme == "http" ? 80 : 443) { port = value }
      }
      self.port = port
      self.path = WebAddress.path(raw, from: index, to: end)
    }

    private static func host(_ text: Text) -> (text: String, isLocal: Bool)? {
      if text[0] == "[" {
        guard text[text.count - 1] == "]", let pieces = ipv6(Text(text[1..<(text.count - 1)])) else { return nil }
        let loopback = pieces == [0, 0, 0, 0, 0, 0, 0, 1]
        let mappedLoopback = Array(pieces[0..<5]) == [0, 0, 0, 0, 0] && pieces[5] == 0xFFFF && pieces[6] >> 8 == 127
        return ("[" + serialized(ipv6: pieces) + "]", loopback || mappedLoopback)
      }
      var name = ""
      for scalar in text {
        // Forbidden in a domain, or outside what this reader vouches for.
        guard (0x21...0x7E).contains(scalar.value), !"%#/:<>?@[\\]^|".unicodeScalars.contains(scalar) else { return nil }
        name.unicodeScalars.append(lowercasedASCII(scalar))
      }
      let labels = name.split(separator: ".", omittingEmptySubsequences: false)
      guard !labels.contains(where: { $0.hasPrefix("xn--") }) else { return nil }
      // A name whose last label is a number is an IPv4 address or nothing.
      var last = labels[labels.count - 1]
      if last.isEmpty, labels.count > 1 { last = labels[labels.count - 2] }
      let numeric = !last.isEmpty && last.unicodeScalars.allSatisfy(isASCIIDigit)
      if numeric || ipv4Number(last) != nil {
        var parts = labels
        if parts[parts.count - 1].isEmpty { parts.removeLast() }
        guard parts.count <= 4 else { return nil }
        var numbers: [UInt64] = []
        for part in parts {
          // A part that does not fit 32 bits is no address at all.
          guard let number = ipv4Number(part), number <= 0xFFFF_FFFF else { return nil }
          numbers.append(number)
        }
        guard var address = numbers.popLast(), address <= UInt64(0xFFFF_FFFF) >> UInt64(8 * numbers.count),
              !numbers.contains(where: { $0 > 255 }) else { return nil }
        for (position, number) in numbers.enumerated() { address += number << UInt64(8 * (3 - position)) }
        let octets = [address >> 24 & 0xFF, address >> 16 & 0xFF, address >> 8 & 0xFF, address & 0xFF]
        return (octets.map { String($0) }.joined(separator: "."), octets[0] == 127)
      }
      var trimmed = Substring(name)
      while trimmed.hasSuffix(".") { trimmed = trimmed.dropLast() }
      return (name, trimmed == "localhost" || trimmed.hasSuffix(".localhost"))
    }

    /// One part of a dotted IPv4 address: decimal, `0x` hexadecimal or
    /// leading-zero octal. Nil when it is not a number; a value above 32 bits
    /// is returned as is and rejected by the caller.
    private static func ipv4Number(_ part: Substring) -> UInt64? {
      var digits = Text(part.unicodeScalars)
      guard !digits.isEmpty else { return nil }
      var radix: UInt64 = 10
      if digits.count >= 2, digits[0] == "0", digits[1] == "x" || digits[1] == "X" {
        digits.removeFirst(2)
        radix = 16
      } else if digits.count >= 2, digits[0] == "0" {
        digits.removeFirst()
        radix = 8
      }
      var value: UInt64 = 0
      for scalar in digits {
        let digit: UInt64
        switch scalar.value {
        case 0x30...0x39: digit = UInt64(scalar.value - 0x30)
        case 0x41...0x46: digit = UInt64(scalar.value - 0x41 + 10)
        case 0x61...0x66: digit = UInt64(scalar.value - 0x61 + 10)
        default: return nil
        }
        guard digit < radix else { return nil }
        // Saturate: anything this large is rejected by the caller anyway.
        value = min(value * radix + digit, 0x1_0000_0000)
      }
      return value
    }

    /// The eight 16-bit pieces of the address between the brackets.
    private static func ipv6(_ input: Text) -> [UInt32]? {
      let length = input.count
      var pieces = [UInt32](repeating: 0, count: 8)
      var piece = 0
      var compress: Int?
      var index = 0
      var embeddedIPv4 = false
      guard length >= 2 else { return nil }
      if input[0] == ":" {
        guard input[1] == ":" else { return nil }
        index = 2
        piece = 1
        compress = 1
      }
      while index < length {
        guard piece < 8 else { return nil }
        if input[index] == ":" {
          guard compress == nil else { return nil }
          index += 1
          piece += 1
          compress = piece
          continue
        }
        let start = index
        let limit = min(length, start + 4)
        var value: UInt32 = 0
        while index < limit {
          let digit: UInt32
          switch input[index].value {
          case 0x30...0x39: digit = input[index].value - 0x30
          case 0x41...0x46: digit = input[index].value - 0x41 + 10
          case 0x61...0x66: digit = input[index].value - 0x61 + 10
          default: digit = 16
          }
          guard digit < 16 else { break }
          value = value * 16 + digit
          index += 1
        }
        if index < length {
          if input[index] == "." {
            guard index > start, piece <= 6 else { return nil }
            index = start
            embeddedIPv4 = true
          } else if input[index] == ":" {
            index += 1
            guard index < length else { return nil }
          } else {
            return nil
          }
        }
        if embeddedIPv4 { break }
        pieces[piece] = value
        piece += 1
      }
      if embeddedIPv4 {
        guard piece <= 6 else { return nil }
        var seen = 0
        while index < length {
          if seen > 0 {
            guard seen < 4, input[index] == "." else { return nil }
            index += 1
          }
          var octet: UInt32?
          while index < length, isASCIIDigit(input[index]) {
            let digit = input[index].value - 0x30
            if let current = octet {
              // No leading zero, and no more than one byte.
              guard current != 0 else { return nil }
              octet = current * 10 + digit
              guard current * 10 + digit <= 255 else { return nil }
            } else {
              octet = digit
            }
            index += 1
          }
          guard let parsed = octet else { return nil }
          pieces[piece] = pieces[piece] * 0x100 + parsed
          seen += 1
          if seen == 2 || seen == 4 { piece += 1 }
        }
        guard seen == 4 else { return nil }
      }
      guard index >= length else { return nil }
      if let compress = compress {
        var swaps = piece - compress
        piece = 7
        while swaps > 0 {
          pieces.swapAt(piece, compress + swaps - 1)
          swaps -= 1
          piece -= 1
        }
      } else if piece != 8 {
        return nil
      }
      return pieces
    }

    /// Lowercase hexadecimal with the longest run of zero pieces compressed.
    private static func serialized(ipv6 pieces: [UInt32]) -> String {
      var longest = -1
      var longestLength = -1
      var start = -1
      for position in 0...8 {
        if position < 8, pieces[position] == 0 {
          if start < 0 { start = position }
        } else {
          if start >= 0, position - start > longestLength {
            longest = start
            longestLength = position - start
          }
          start = -1
        }
      }
      let compressStart = longestLength < 2 ? -1 : longest
      let compressEnd = longestLength < 2 ? -2 : longest + longestLength
      var output = ""
      var position = 0
      while position < 8 {
        if position == compressStart {
          output += position == 0 ? "::" : ":"
          guard compressEnd < 8 else { break }
          position = compressEnd
        }
        output += String(pieces[position], radix: 16)
        if position < 7 { output += ":" }
        position += 1
      }
      return output
    }

    /// The path up to `?` or `#`: backslashes become slashes, dot segments are
    /// resolved and everything outside printable ASCII is percent-encoded.
    private static func path(_ raw: Text, from: Int, to end: Int) -> [UInt8] {
      let slash = UInt8(ascii: "/")
      let hexDigits = Array("0123456789ABCDEF".utf8)
      var path = [slash]
      var index = from
      if index < end, raw[index] == "/" || raw[index] == "\\" { index += 1 }
      while true {
        let segmentStart = path.count
        var endsWithSlash = false
        while index < end {
          let scalar = raw[index]
          if scalar == "?" || scalar == "#" {
            index = end
            break
          }
          index += 1
          if scalar == "/" || scalar == "\\" {
            path.append(slash)
            endsWithSlash = true
            break
          }
          if (0x21...0x7E).contains(scalar.value), !"\"<>`{}".unicodeScalars.contains(scalar) {
            path.append(UInt8(scalar.value))
          } else {
            for byte in String(scalar).utf8 {
              path.append(UInt8(ascii: "%"))
              path.append(hexDigits[Int(byte >> 4)])
              path.append(hexDigits[Int(byte & 0x0F)])
            }
          }
        }
        let segmentEnd = endsWithSlash ? path.count - 1 : path.count
        let segment = String(decoding: path[segmentStart..<segmentEnd], as: UTF8.self).lowercased()
        if segment == ".." || segment == "%2e%2e" || segment == "%2e." || segment == ".%2e" {
          path.removeSubrange(segmentStart...)
          if path.last == slash, lastSlashCanBeRemoved(path) { path.removeLast() }
          if let last = path.lastIndex(of: slash) { path.removeSubrange((last + 1)...) }
          if endsWithSlash, path.last != slash { path.append(slash) }
        } else if segment == "." || segment == "%2e" {
          path.removeSubrange(segmentStart...)
          if path.last != slash { path.append(slash) }
        }
        if !endsWithSlash { break }
      }
      return path
    }

    /// Whether the slash that ends `path` closes a segment that `..` may
    /// remove: not the root, and not a drive letter such as `/C:/`.
    private static func lastSlashCanBeRemoved(_ path: [UInt8]) -> Bool {
      guard let before = path.dropLast().lastIndex(of: UInt8(ascii: "/")) else { return false }
      let segment = path[(before + 1)...]
      guard segment.count >= 2 else { return true }
      let first = segment[segment.startIndex]
      let second = segment[segment.startIndex + 1]
      let letter = (0x41...0x5A).contains(first) || (0x61...0x7A).contains(first)
      let drive = letter && (second == UInt8(ascii: ":") || second == UInt8(ascii: "|"))
      guard drive else { return true }
      if segment.count == 2 { return false }
      let third = segment[segment.startIndex + 2]
      return !(third == UInt8(ascii: "/") || third == UInt8(ascii: "\\") || third == UInt8(ascii: "?") || third == UInt8(ascii: "#"))
    }
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

  /// Whether a path may begin right after `previous`.
  private static func isPathBoundary(_ previous: Unicode.Scalar) -> Bool {
    if isWhitespace(previous) { return true }
    switch previous {
    case "=", "(", "[", "{", "\"", "'", ",", ";", ":", "`", "<", ">", "|", "\u{201C}", "\u{2018}": return true
    default: return false
    }
  }

  /// A drive (`C:\`, `C:/`), an absolute path (`/x`), a home path (`~/`,
  /// `~\`) or a share (`\\`, `//host`).
  private static func isPathStart(_ chars: Text, _ index: Int) -> Bool {
    func scalar(_ offset: Int) -> Unicode.Scalar? {
      index + offset < chars.count ? chars[index + offset] : nil
    }
    func isName(_ scalar: Unicode.Scalar?) -> Bool {
      guard let scalar = scalar else { return false }
      return scalar != "/" && !isWhitespace(scalar)
    }
    let current = chars[index]
    let next = scalar(1)
    let drive = isASCIILetter(current) && next == ":" && (scalar(2) == "\\" || scalar(2) == "/")
    let absolute = current == "/" && isName(next)
    let home = current == "~" && (next == "/" || next == "\\")
    let share = (current == "\\" && next == "\\") || (current == "/" && next == "/" && isName(scalar(2)))
    return drive || absolute || home || share
  }

  private static func pathAt(_ chars: Text, _ index: Int) -> Int? {
    let previous: Unicode.Scalar? = index > 0 ? chars[index - 1] : nil
    if let previous = previous, !isPathBoundary(previous) { return nil }
    guard isPathStart(chars, index) else { return nil }
    // `//host` right after a colon is the authority of a URL that step 3
    // kept (`https://example.org/a`), not a share.
    if previous == ":", chars[index] == "/", index + 1 < chars.count, chars[index + 1] == "/" { return nil }
    return pathEnd(chars, index)
  }

  /// Where the redaction of a path that starts at `start` stops. Names
  /// contain spaces and apostrophes, so neither ends a path. It ends before a
  /// closing double quote (not `\"`), an angle bracket, a backtick, a line
  /// break or the `: ` that introduces an error's cause; words in between go
  /// with it. The whitespace before that delimiter is not part of the path.
  private static func pathEnd(_ chars: Text, _ start: Int) -> Int {
    var end = start
    scan: while end < chars.count {
      switch chars[end] {
      case "\n", "\r", "<", ">", "`":
        break scan
      case "\"":
        if !backslashEscaped(chars, floor: start, index: end) { break scan }
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
