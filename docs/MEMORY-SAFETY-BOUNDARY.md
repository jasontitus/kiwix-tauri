# Memory-safety boundary decision — 2026-11-20

## Decision

The project accepts these components as **audited platform or optional boundaries** for the
memory-safety target:

- **ring** for TLS, QUIC and other cryptographic primitives. It is a vetted, widely deployed
  BoringSSL-derived implementation. AWS-LC was removed; ring is the only provider the app installs.
- **sherpa-onnx / ONNX Runtime** for the optional Kokoro neural voice pack. The system voice path
  remains the default, the model is checksum-pinned, and the performance gate omits Kokoro when the
  device cannot keep up.
- **OS-provided WebViews, media decoders, audio output and platform services** (WKWebView, Android
  WebView, WebView2, WebKitGTK, GStreamer, cpal/rodio backends, UIKit/AppKit/Win32/GTK entry points).
- **The OS kernel, libc and standard runtime.**

This is a scope decision, not a claim that these components contain no unsafe code. Their own
memory-safety and update processes remain the owners' responsibility. The app must not pass
archive-controlled pointers or lengths into them outside documented APIs, and app-owned adapters
around them remain in scope.

## Still in scope

- Every first-party Rust `unsafe` block under `src-tauri/src` and `kiwix-mcp/src`, including the
  Windows ACL/pipe/print bridges, GTK print widgets, Apple DNS-SD/QUIC transport, Android JNI, and
  the MCP named pipe.
- Native sources under `src-tauri/native` and the mobile plugin that are not OS-provided.
- Rust dependencies that parse archive, torrent, HTML, image, compression, search or network bytes,
  except the accepted boundaries above.
- Build scripts and packaged native libraries that select or load code outside Cargo.
- The retained-file, publication, cancellation and authority contracts that make the unsafe
  adapters safe.

## Evidence and limits

- First-party census: 117 documented blocks in 23 files; `npm run check:unsafe` enforces the
  baseline and 140 modules forbid unsafe Rust.
- Host clippy and 1,287 lib tests pass; iOS and Android lib type checks pass; Windows lib
  type-checks with mingw and `-D warnings`; Linux is cross-checked in Docker.
- The reader benchmark remains within 10% of the prior run.
- No device runtime, sanitizer, Miri or binary-level certification is claimed.

## Audit status

The Windows, Apple and Linux/Android/MCP independent audits are complete. They found and closed the
ACE/SID extent, printer-name padding, DEVMODEW prefix, WebView2 reentrancy, dialog cancellation,
NT rename length, COM initialization, relative-volume and JNI-reference defects. The GTK
file-printer name is now rejected when another backend shares it.

Two non-memory-safety residuals remain documented: GTK printer enumeration is still synchronous
and can delay an export deadline if a backend never completes, and the Windows volume-capacity
query still classifies remote mounted folders by the share root. Device runtime, sanitizer and
binary-level certification are not claimed.
