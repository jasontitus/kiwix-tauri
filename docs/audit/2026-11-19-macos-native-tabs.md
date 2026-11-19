# macOS native window tabs

Rust passes an NSString label and length-delimited NSData to the app's Objective-C++ tab bridge.
The bridge copies the label into the AppKit window association and decodes the borrowed JSON
synchronously with Foundation. A snapshot returns one owned immutable Foundation reference.
Rust copies the bytes and reads the existing member format. This removes the manual C string
allocation, read and free from the bridge.

Adoption uses Wry's callback for the exact live window. The pinned dispatcher invokes it inline
on the main thread for both startup and window creation, with the NSWindow retained through the
call. The application still has one scoped raw-to-typed window borrow. Native tabbing retains
its AppKit owner, synchronous main-queue serialization and localized menu selectors.

## Review findings and fixes

- Window restore validates the whole plan before detaching a tab. It rejects malformed members,
  missing or repeated labels, non-boolean focus and multiple focused groups. The native fixture
  verifies that an invalid plan preserves the existing groups.
- Close records process-local retirement under the same Store lock that commits captured geometry
  and tab groups. Late capture cannot recreate a retired label. The explicit last-session
  snapshot applies the same filter. A live library-only shell remains eligible for capture.
  AppKit capture takes the Store lock without acquiring the Service's window-operation lock.
- Explicit restore of a retired saved shell assigns a fresh label and preserves the saved group
  metadata. Repeating restore uses the live replacement. The older ID remains retired.
  Capacity checks count closing windows until AppKit removes them.
- Restore requests serialize before taking the saved and live snapshots. The accepted
  desktop restore runs in a task that continues if its IPC waiter leaves. If the inner
  restore task stops while creation is queued, the callback keeps the guard until creation
  ends. When the task continues, it holds that guard through grouping and notification.

Independent review checked the native types and ownership, restore capacity and cancellation,
close and capture ordering, profile protection and the saved-window compatibility paths.
The reviewed base is `fcf7138bdd6888d5bc4a8a77887d74ecf0068a5b`. The eight-path source manifest
has SHA-256 `9d7403f69e7993cfdb2060413af61aeec50d2dbae893b1e71d199fb981a0d10d`.

The production source census counts 132 explicit unsafe Rust blocks, 119 in the app and 13 in
the MCP relay. The previous count was 139. Separate explicit unsafe declarations went from 34
to 35 because the previously implicit unsafe extern declaration now names that boundary.

## Verification

- The macOS AppKit fixture passed under AddressSanitizer and UndefinedBehaviorSanitizer with
  test-owned windows. It verified copied and retained data, invalid plan rejection before
  mutation, valid ordering and selection, preservation of an unrelated group, and Unicode menu
  labels.
- The mixed Rust/AppKit release fixture used the production declarations and implementation.
  It exercised adopt, restore, menu update and snapshot. Rust decoded the same snapshot after
  its autorelease pool retired and the original windows closed.
- The exact signed Mac application test binary passed 13 focused tests. They include a
  late capture after normal Service retirement, a restored secondary with a fresh identity,
  a repeat restore without another window, window count exhaustion, the next saved snapshot,
  and private and read-only snapshot protection. One unrelated physical-LAN test matched the
  filter and was ignored.
- Locked application library checks passed for ARM64 iOS Simulator and ARM64 Android
  with the installed SDKs. These checks cover the shared Store changes. The iOS check still
  reports the pre-existing Apple PDF unused import.

The native fixture ran without foreground activation. Interactive menu actions and foreground
focus remain untested. The service tests exercise the actual persistence and sequential restore
planner with synthetic window lists. The queue handoff was checked in source.
