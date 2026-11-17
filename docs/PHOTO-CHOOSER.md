# Photo chooser: camera or file for archive image inputs

Status: shipped on Windows (2026-11-15). Designed after three independent adversarial reviews: of the first design, of the revised design, and of the implementation.

## Problem
Archives ask for pictures with `<input type="file" accept="image/*">`, sometimes with `capture`. StreetZim's photo search, for example, reads the text on a sign and finds the place on the map. A desktop webview only ever offers a file picker for such an input: WebView2, WebKit and WebKitGTK ignore `capture`. On a laptop with a camera, the user therefore cannot take the photo.

## Behaviour
On Windows, an image file input in an archive opens the app's chooser instead of the file picker:
- **"“Book” asks for a photo."**
- **Use camera:** opens a small Kiwix camera window with a preview, **Take photo**, **Switch camera** (when there are several) and **Cancel**.
- **Choose a file:** opens the normal Windows image dialog.
- **Cancel:** the page receives the input's `cancel` event.

The page receives exactly one picture, as if the user had picked it: a `File` in `input.files`, followed by `input` and `change` events.

- **macOS and Linux:** no camera window yet, so nothing is intercepted and the normal file picker opens directly.
- **Mobile:** unchanged. The OS already offers a camera.
- **Restricted mode and nested frames:** the native picker.

## How it works
1. **Bridge** (`src/reader-bridge.js`, top archive frame).
   - **What it intercepts:** a trusted click on an enabled single-file input whose whole `accept` list is photo types the shell can return (JPEG, PNG, WebP, GIF, BMP, HEIC/HEIF or AVIF, or `image/*`), or the page's own `input.click()` / `showPicker()` inside a genuine user activation (StreetZim opens its hidden input from a button handler). Mixed lists (`image/*,application/pdf`), unsupported image types (SVG, TIFF), and `multiple` inputs keep the browser's own picker. It cancels the native picker and posts `photo-request`.
   - **The request:**
     - carries the session's **photo secret**, which arrives in `reader-init` (the bridge stops that message from reaching archive scripts);
     - carries a random request ID;
     - only one can be outstanding at a time.
   - **Delivery:** the result is applied with browser natives captured before any archive script runs. It goes to the same input, only if that input is still connected, enabled and a file input.
   - **No dead inputs:** if the shell does not acknowledge within 3 s, the input is released.
2. **Shell** (`src/components/Reader.tsx`).
   - **Request checks:** the shell accepts a request only from its own frame window and origin, with the current session's secret, and only when none is open. It answers every request at once, accepted or cancelled.
   - **Binding:** each request is bound to the frame, session, archive, origin and privacy generation. Any change drops it and closes the camera.
   - **Delivery:** the shell shows the chooser and posts the image bytes (transferred, not copied) back to the frame.
3. **Rust** (`src-tauri/src/photo_capture.rs`).
   - **`pick_image`:**
     - opens the native dialog, parented to the reader window and filtered to the image types the input accepts (SVG is never offered);
     - reads only the chosen file, at most 25 MB, with its image type checked by content;
     - returns the file name, never the path.
   - **`camera_capture_open`:** opens the app-owned window `camera-capture`, which loads only the bundled `camera.html` on the app's origin.
     - It is incognito, and in portable mode its WebView data stays in the portable profile.
     - The session records the requesting window and request ID.
     - Another window's open camera is never brought forward.
   - **Camera permission:** the webview permission handler allows the camera **only for the `camera-capture` window, and only while a session is open**. Archive windows keep their per-archive consent unchanged, so no archive gains camera access through this path.
   - **Capture-window commands:** the capture window may call only `camera_capture_complete` / `camera_capture_cancel` through `dispatch`.
   - **Checking and delivery:**
     - The photo is checked: JPEG or PNG, at most 25 MB.
     - It is sent to the requesting window with its request ID, and the window closes.
     - On any failure the reader still gets a cancellation with the reason, so the user can choose a file instead.
   - **Vendored wry:** WebView2 no longer saves any permission decision in the shared profile, so no Allow outlives the app's own decision.
4. **Camera page** (`camera.html`, `src/camera.ts`).
   - It shows `getUserMedia` video, mirrored for a front camera.
   - It captures to a canvas, with the longer side at most 4096 px, as JPEG (0.92), or PNG when the page accepts PNG but not JPEG. The camera is not offered when the input accepts neither type.
   - It explains blocked or missing cameras, pointing to Windows Settings › Privacy & security › Camera.

## Privacy
- **Nothing is written to disk:** the photo exists only in memory until the page has it, exactly like a picked file.
- **The archive page sees nothing else:** it never sees the camera, the preview, the dialog or the file's path.

## Known limits
- **Shell-side text is English** in other languages until the locale catalogs are regenerated.
- **Activation reuse:** a page's `cancel` handler may reopen the chooser within the same user activation (a few seconds).
- **Direct IPC commands:** the capture window could call the app's few non-`dispatch` IPC commands directly. It only ever loads the bundled page.

## Verified on Windows 11 (Logitech C930e)
- **Use camera:** in StreetZim Bahrain, the camera icon opens the chooser. Use camera opens the camera window (1920×1080 preview), and the photo reaches the page, whose photo search runs on it.
- **Choose a file:** a picked image with the word "MANAMA" reaches the page, which finds Manama on the map.
- **Cancel:** the page gets `cancel`, and no archive camera grant is created.
- **After a reload:** the chooser still works when the page reloads in place.
