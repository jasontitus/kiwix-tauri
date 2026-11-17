# Android physical PDF-share test — 2026-11-10

Device: Samsung SM-A356U (Android release reported by `ro.build.version.release`), serial `<samsung-serial>`.
Installed package: `org.kiwix.workbench` 0.1.0, last update 2026-11-09 13:40; exact source commit was not recorded by the installed APK.
Archive: Hawaii StreetZim (`osm_en_hawaii_2026-10-26.zim`), opened from the library/deep link.

## Observed

1. Reader overflow → **Reading appearance** → **Print and export** → **Share**.
2. The Android chooser showed one item: `OpenStreetMap Offline.pdf`.
3. Google Drive was selected; Drive's upload sheet showed the filename and **My Drive**, and the upload completed.
4. Drive **Uploads** listed `OpenStreetMap Offline.pdf` (uploaded 8:49 AM); opening it showed a **2-page PDF**.
5. The PDF page preserved the StreetZim UI (search field, category buttons, map controls, attribution) but the **MapLibre/WebGL map area was blank/white**. The map canvas was not captured in the PDF.
6. The test file was moved to Drive trash after verification.

## Result

- Android system share-sheet handoff: **pass** (physical Samsung → Google Drive).
- PDF file validity/page count: **pass** (2 pages).
- StreetZim dynamic map fidelity: **fail** — the WebGL canvas is absent from the PDF.

The existing `NativeArticlePdf` path captures screen-layout page slices; this physical run shows that WebGL/canvas content is not included. This is a fidelity gap for StreetZim-style archives, not a share-sheet wiring failure. Fixing it requires a rendered-surface capture path (for example PixelCopy/window-surface capture) or a pre-export canvas rasterization step, with the same privacy/authority gates.

## Limits

- One physical device and one recipient (Google Drive); other recipients were not tested.
- Installed APK commit/version provenance is only the package metadata above.
- No test of complex multi-page articles without WebGL; the earlier simple-fixture recipient test remains the only non-WebGL evidence.
