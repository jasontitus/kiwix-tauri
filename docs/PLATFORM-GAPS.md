# Platform gaps (audit 2026-11-16)

Goal: every feature works on every platform that can support it. This audit compared docs/FEATURES.md, the README feature table and the code's platform gating. Each item was verified against code unless marked unverified. Location is handled separately: native providers exist for macOS/iOS (CoreLocation) and Windows (WinRT; first-use runtime acceptance pending), and were written for Linux (GeoClue) and Android (LocationManager) but are not yet enabled.

## Features with a gap

| Feature | Win | mac | Linux | Android | iOS |
|---|---|---|---|---|---|
| System share sheet (archive, article PDF) | ✗ | ✓ | ✗ | ✓ | ✓ |
| Native download-complete notifications | ✗ | ✓ | ✗ | ✓ | ✓ |
| Download progress outside the window (taskbar / Live Activity / notification) | ✗ | ✗ | ✗ | ✓ | ✓ |
| Keep the machine awake during transfers | ✗ | partial | ✗ | partial | n/a |
| Metered/cellular download policy | ✗ | ✗ | ✗ | ✓ | ✓ |
| `zim://` article links | ✗ | ✓ | ✗ | ✓ | ✓ |
| `kiwix://search` (Look up) | ✓ | ✓ | ✓ | ✓ | ✗ |
| Read aloud: Next paragraph, ±15 s | ✓ | ✓ | ✓ | ✓ | ✗ |
| Print the article | ✓ | ✓ | ✓ | via PDF | ✗ |
| Camera for archive photo inputs | ✓ | ✗ | ✗ | ✗? | ✓? |
| AI assistant connector | ✓ | ✓ | ✓ | n/a | n/a |
| Shortcuts, home-screen widget, pinned articles | ✗ | ✗ | ✗ | ✓ | ✗ |
| Hosting a temporary Wi-Fi hotspot | ✗ | ✗ | ✗ | ✓ | ✗ |

## Feasible and worth doing, highest value first

1. **Read aloud Next / ±15 s on iOS.**
   - Desktop Kokoro and Android show the controls (`src/components/Reader.tsx` ~5900); iOS's system-voice bridge has only `speechPrevious`.
   - Fix: add next/rewind/forward in `NativeSpeech.swift`.
   - Size S–M.
2. **Windows system share.**
   - Today `share.rs`, `native_pdf.rs`, `pdf_export.rs` and `api.ts` (`canShareSystem`) exclude Windows and Linux.
   - API: `IDataTransferManagerInterop::ShowShareUIForWindow` with a `DataRequested` handler providing a `StorageFile`.
   - Size M; testable on Windows.
   - Linux has no share portal (not possible); "Show in folder" remains.
3. **Download-complete notifications on Windows and Linux.**
   - `service/dispatch.rs` answers "unavailable on this platform", and the Settings toggle is hidden.
   - API: WinRT `ToastNotificationManager` (the NSIS shortcut provides the AppUserModelID); `org.freedesktop.Notifications` or the portal Notification interface.
   - Size M. The README's "a notice when a download finishes" is unqualified, so it is currently untrue on Windows and Linux.
4. **Taskbar / dock download progress.**
   - Nothing calls `set_progress_bar`.
   - API: Tauri `Window::set_progress_bar`, which covers Windows `ITaskbarList3`, the macOS dock and Linux libunity. Feed it the acquisition list `live_activity.rs` already projects.
   - Size S; testable on Windows.
5. **Keep the machine awake during transfers.**
   - No Windows/Linux implementation exists (`power_activity.rs`).
   - The guard is taken for LocalSwarm transfers (effective on macOS and Android; on iOS, Windows and Linux the guard does nothing), and on macOS also for a whole map download (`maps/manager.rs`, macOS only because on Android the same guard takes a high-performance Wi-Fi lock). HTTP and torrent downloads never hold it.
   - API: Windows `PowerCreateRequest` / `PowerSetRequest(PowerRequestSystemRequired)`; Linux logind `Inhibit("sleep")`.
   - Size S; testable with `powercfg /requests`.
6. **`zim://` on Windows and Linux.**
   - The handler is macOS-only (`lib.rs` `open_zim_links`, `RunEvent::Opened`), and the installer registers only `kiwix:`.
   - This also leaves the AI connector's `zim://` citations unclickable on Windows.
   - Fix:
     - register `zim:` in `windows/installer-hooks.nsh` with the same don't-overwrite rule;
     - accept `zim://` in `--open-url`;
     - on Linux, add a URL-handler `.desktop` file.
   - Size S.
7. **Metered-network download policy on desktop.**
   - The setting is mobile-only (`transfer_policy.rs`, `Settings.tsx`).
   - API:
     - Windows `NetworkInformation::GetInternetConnectionProfile().GetConnectionCost()`;
     - NetworkManager `Metered`;
     - macOS `NWPathMonitor` `isExpensive`/`isConstrained`.
   - Size M.
8. **iOS print.**
   - Android: Print routes to the article PDF export, which opens Android's print dialog; the duplicate `window.print()` row is no longer shown.
   - iOS: the Print row is not offered and Cmd+P shows "Printing is not available on this device. Save or share the article as a PDF instead." There is still no native print path.
   - Fix: `UIPrintInteractionController` on the PDF `pdf.mm` already renders.
   - Size S–M; needs a Mac. Neither mobile behaviour has been checked on a device.
9. **iOS `kiwix://search`.**
   - The scheme is registered (Info.plist), but `mobile.rs` only handles host `speech`, so Look up links are silently dropped.
   - Size S–M; needs a Mac.
10. **Android camera for archive photo inputs (likely broken; unverified).**
    - The vendored wry `RustWebChromeClient.kt` offers capture only for `capture` inputs.
    - Its `resolveActivity(ACTION_IMAGE_CAPTURE)` needs a `<queries>` entry on API 30+, which neither manifest has.
    - Fix: add the query, and use a chooser with `EXTRA_INITIAL_INTENTS` for image-only inputs.
    - Size S; needs the Android SDK.

## Known items: feasibility
- **Photo chooser camera window, macOS: S–M.**
  - Already in place: the camera permission mapping in wry's UI delegate, the shared permission handler, `NSCameraUsageDescription` and the sandbox camera entitlement.
  - Main change: the `isWindows` gates in `Reader.tsx`.
- **Photo chooser camera window, Linux: M.** WebKitGTK ships with `enable-media-stream` off and wry never enables it. This needs a wry patch for the camera window plus GStreamer v4l2/pipewire plugins in the AppImage.
- **AI assistant connector and Look up, macOS/Linux: implemented.**
  - Same-UID Unix socket, bundled `kiwix-mcp`, macOS `Info.plist` and Linux desktop-entry registration.
  - The packaged sandboxed macOS app and relay passed end-to-end list/search/read and private-mode stop; a Linux desktop runtime run is still required.

- **Android system Back:** Back closes, in this order, an open dialog, the tab sheet, a popover (a shell picker, menu or suggestion list, or a reader overlay such as the find bar or a preview), then search; then it steps back in the reader's history, then returns to the library (`src/shell-dismiss.ts`, `src/android-back.ts`). At the library root Kiwix registers no handler, so Back is the WebView default: it first walks the WebView's own page history, so after reading articles it can take several presses before the app goes to the background. Fixing that needs a native hand-back from the Tauri app plugin. Not yet checked on a device.

## Feasible, lower value
- **Platform shortcuts and widgets:** iOS quick actions and a WidgetKit search widget; a Windows Jump List, macOS dock menu, and Linux `.desktop` actions.
- **Media keys / Now Playing for desktop Kokoro:** Windows SMTC, MPRIS, `MPNowPlayingInfoCenter`.
- **Nearby on desktop:** Windows Mobile Hotspot (whether policy allows it is unverified); desktop Bluetooth LE Nearby.
- **Android-to-Android temporary Wi-Fi joining:** `can_join` is iOS-only though `NearbyJoin.kt` exists.
- **Linux system voices:** availability depends on the distro's WebKitGTK speech build; speech-dispatcher is an option.

## Not possible on that platform
- Native window tab groups outside macOS.
- A temporary hotspot on iOS/macOS.
- Device-wide ZIM scan on iOS.
- A Linux share sheet.
- Portable mode in the sandboxed macOS build.

## Docs that don't match the code
- **Natural voices:** README says "on desktop and Android", but iOS is also included (`KOKORO.md`, `Cargo.toml`).
- **`zim:` registration:** `PLATFORM-INTEGRATIONS.md` says only macOS/iOS register `zim:`, but Android does too (`AndroidManifest.xml`).
