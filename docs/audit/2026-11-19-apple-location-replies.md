# Apple location request ownership

Apple location requests now use the same checked Rust reply registry as other Apple bridges.
The app's native registry retains each request on the main queue. The request serves as the
CoreLocation delegate and owns its manager and acquisition timer. Rust supplies a non-reused request
ID and static functions. The callback receives eight position fields by value and borrows
native-owned immutable UTF-8 diagnostic bytes for the duration of the call. Rust owns the final
position or error. The typed declaration remains the bridge to the Objective-C++ implementation.

A cancelled or dropped request retires the Rust reply before or during native cleanup. A stale ID
cannot complete another request. An authorizing request checks that its waiter is live after the OS
authorization query and before starting location updates. The OS can finish a prompt that it has
already accepted. Rust still rechecks the requesting session, installation consent and native
request ID before returning a result through the archive bridge. The native timer still starts after
authorization.

The production source census went from 144 to 139 explicit unsafe Rust blocks across the app and
MCP relay. The app has 126 and the relay has 13. Separate unsafe declarations went from 36 to 34.
The archive sensor permission module now forbids unsafe Rust. The small Apple adapter retains its
typed FFI declaration and uses the existing Foundation binding and native CoreLocation code.
This is an explicit Rust syntax count across targets. It excludes test-only source, dependencies
and Objective-C++.

## Adversarial review

The first independent pass found an infallible UTF-8 conversion through the NSString binding.
CoreLocation error text now goes through fallible Foundation encoding. Rust checks the returned
bytes and falls back to the existing unavailable message for missing or invalid diagnostics.
Another finding was the gap between checking a request and querying CoreLocation permission.
The new checks cover retirement during that query before a prompt or acquisition, and after
preparing the acquisition timer. Errors from other native domains no longer reuse CoreLocation's
numeric permission-denied code. The reviewer also checked lock ordering, per-window identity,
stale callbacks, cleanup and the native data lifetime.

The corrected five source paths were reviewed against
`5190183cdecca5cae6a4898efb0eda47ec3fda7f`. Their manifest SHA-256 is
`b28b069042fe32cce5605cfd10b048a6535553c8c0246bbef8d43c4c69ba7e25`.

## Verification

- The macOS native fixture passed under AddressSanitizer and UndefinedBehaviorSanitizer.
  It mocks CoreLocation's hardware responses and exercises the production timer, cancellation,
  stale/invalid fix handling, authorization revocation, terminal errors and registry cleanup.
  Retiring a request during the fake authorization query starts neither a prompt nor acquisition.
- The standalone Objective-C++ and Rust ABI fixture passed seven tests. The native finish callback
  returned all eight position values and a Unicode diagnostic to the exact Rust consumer.
  The diagnostic remained valid after the native autorelease pool retired. The fixture ran
  serially against a mocked request owner.
- The exact macOS ARM64 application test binary passed strict signature verification.
  All 16 selected position, reply registry and archive sensor permission tests passed, including
  window closure, a relink with the same archive UUID and revocation before delivery.
- The exact application library passed locked iOS ARM64 device and simulator compile checks.
  The signed standalone native fixture ran on a dedicated iOS 26.5 simulator and passed the same
  mocked location cases. The runner verified that its simulator and owned device directory were
  removed. The iOS app-library checks reported an existing unused import in Apple PDF code.

The checks do not cover a real location sensor, the system permission panel, or on-device
suspension and resume.
