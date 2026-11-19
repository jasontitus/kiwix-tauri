# Typed AppKit shortcuts and WebKit page background

Two application-owned Objective-C++ shims were removed. The menu installer now normalizes
AppKit key equivalents using the pinned Rust bindings after Tauri installs the menu. It runs
on the main thread before adding the native Window tab menu. The normalizer handles the same
ASCII letters, digits and punctuation, clears only the Shift modifier on a mapped key, and
ignores multicode-unit and unmapped keys. The module forbids unsafe Rust.

The macOS background path borrows the retained WKWebView during Wry's main-thread callback.
Typed AppKit bindings resolve its effective Aqua or Dark Aqua appearance and construct an opaque
sRGB colour from three byte values. A version check gates the WebKit setter at macOS 12.
The setter copies the colour. The app changes only `underPageBackgroundColor`. The Rust
`windows` module is desktop-only and the removed iOS static shim had no application caller.

The remaining WebKit operations that require unsafe Rust are the scoped view borrow, access
to AppKit's immutable appearance constants and the version-gated setter. The production source
census records 132 explicit unsafe Rust blocks across the app and MCP relay and 35 explicit
declarations. It excludes test-only code, dependencies and Objective-C++. The lockfile adds a
direct edge to the existing objc2-web-kit 0.3.2 package. No package version moved.

## Verification

- On macOS ARM64, a standalone fixture ran the production Rust helpers with real AppKit menus
  and two WKWebViews. It checked nested menus, plain and shifted modifiers, repeat normalization,
  unmapped Unicode, effective appearance and sRGB colour readback. A test-only Objective-C++
  driver sent synthetic Cmd+R and Cmd+Shift+R to the fixture menu with
  `performKeyEquivalent:` and observed two distinct callbacks. The process ran without
  foreground activation.
- The macOS ARM64 app library test binary built with the locked production protocol feature.
  Its signature passed strict verification. Eleven focused tests passed, covering window
  retirement, tab restoration, theme selection and menu routing.
- Locked app library compile checks passed for ARM64 iOS device and simulator targets.
  Both reported an unused import in the unchanged Apple PDF module.

The fixture exercises in-process AppKit dispatch and WebKit properties. It does not exercise
hardware keyboard layouts, the full app's visible navigation transitions or iOS UI.

Independent review used base `bbff5fa9c1849f004a173d6bd76954fbefced3ac`. The nine-path source
manifest, including the two deleted files, has SHA-256
`25166fae0386ec2b6ab362b7c3e34ee0007f6c2219a5c9911df80f9c809f9cfe`.
Review checked the installed-menu order, AppKit key mapping, Wry callback and borrowed object
lifetimes, setter availability, and iOS call and build scope.
