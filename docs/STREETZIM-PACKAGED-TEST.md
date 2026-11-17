# Packaged StreetZim runtime/security acceptance — 2026-11-10

Artifact: post-merge `src-tauri/target/release/bundle/macos/Kiwix.app` (49.36 MiB, ad-hoc signed, App Sandbox; commit `f101591` with the rebuilt desktop SDK; executable SHA-256 `a4f5329cc075f06dc38b197b9b84d833bd299c4f1eaf7f7f047c6a5262ba0f19`).
Profile: `~/Downloads/streetzim-native-current`; archive `~/Downloads/hawaii.zim` (150,605,015 bytes; StreetZim/Osm Hawaii).
Harness: the actual packaged macOS application, launched with the archive path and inspected through its native accessibility tree (AX). The earlier headless-Chromium observations below are retained as a separate, weaker harness; they are **not** native WKWebView evidence.

## Native packaged WKWebView acceptance

The packaged app opened `OpenStreetMap Offline` and exposed a native `AXWebArea` for `Article: OpenStreetMap Offline`.

Normal mode:
- `AXGroup` **Map** present; MapLibre, OpenStreetMap and OpenMapTiles attribution links present.
- Map controls present: Zoom in, Zoom out, Show the whole map, map theme, Find my location, Satellite, 3D, Wiki.
- Hawaii heading and `Map data: 2026/10` present.
- Category toolbar and the real `50 mi` scale control present.

Restricted mode (pressed the real toolbar checkbox):
- banner: “Restricted mode blocks archive scripts, forms, workers and remote network requests. Some pages may not work.”
- fallback heading `Offline OSM` / `Vector tiles · OpenStreetMap data`
- **no** `AXGroup` Map, map controls, MapLibre attribution or category toolbar.
- no archive JavaScript-driven map surface was exposed.

Leaving restricted mode restored the Map group, attribution and controls in the same reader. This is direct native-WKWebView evidence that the packaged reader runs the archive's scripted map in normal mode and suppresses it under the restricted CSP path. It does not certify accelerated pixels, camera/GPS, or a hostile archive beyond this real StreetZim content.

## Normal mode observed

- Reader opened the real StreetZim archive; iframe origin was the per-installation loopback archive origin.
- MapLibre rendered one `<canvas>`; `webgl2` context creation succeeded.
- No browser console errors.
- Archive JavaScript ran: `maplibre-gl.js`, search-data and category-index requests completed.
- JS capabilities from the archive frame:
  - blob Worker round-trip: `ok`
  - WebAssembly instantiate: `true`
  - `history.pushState`/`replaceState`: URL changed and was restored
  - `navigator.geolocation`: present
  - `navigator.mediaDevices.getUserMedia`: present
  - `BarcodeDetector` and `createImageBitmap`: present; a blank-image detect returned zero detections
- Search for `Honolulu` returned place results, exercising the map's JS search/routing path.

These establish that the normal archive lane still permits scripts, WebGL, blob Workers, WASM, routing history, and the camera/geolocation/image APIs. Actual camera frames, GPS fixes, and recognition accuracy require physical hardware.

## Restricted mode observed

Clicking **Open in restricted mode** for the same StreetZim reader produced:

- banner: “Restricted mode blocks archive scripts, forms, workers and remote network requests…”
- iframe fallback text: `Offline OSM / Vector tiles · OpenStreetMap data / About`
- `<canvas>` count: **0**
- only the injected `storage.js` and `bridge.js` plus the unexecuted `maplibre-gl.js` script tag were present
- no console errors

Clicking **Leave restricted mode** reloaded the same reader; MapLibre canvas and search UI returned, with no console errors.

This demonstrates that restricted mode blocks the archive's script-driven map without breaking the trusted bridge or the ability to leave the mode. It does not certify native WKWebView CSP behavior.

## Limits

- Headless Chromium, not packaged WKWebView/WebView2/WebKitGTK.
- No physical camera, GPS, image-recognition model, or sensor permission UI.
- Ad-hoc macOS signing only; not notarized or a distribution build.
- The packaged app's native window was not exercised; only its headless server + bundled shell.
- The exact archive was StreetZim Hawaii; a separate camera/image-recognition StreetZim fixture was not available.
