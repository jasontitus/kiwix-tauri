//! Desktop photo chooser for archive file inputs that ask for camera capture.
//!
//! Desktop webviews ignore `<input capture>`, so an archive's "Take a photo"
//! only opens a file picker. The reader bridge asks the shell instead; the
//! shell offers "Use camera" or "Choose a file", and this module serves both:
//!
//! - `pick_image`: the native image dialog, parented to the reader window. Only
//!   the chosen file is read (at most 25 MB, image content checked by its magic
//!   bytes) and only its file name is returned, never its path.
//! - `camera_capture_*`: an app-owned window (`camera-capture`, bundled
//!   `camera.html`) that may use the camera once. The webview permission
//!   handler allows Camera for that window label only while a session is open,
//!   so no archive ever gains camera access through it. The window can call only
//!   the two completion commands (see `lib.rs` dispatch).
//!
//! Photos pass through memory only; nothing is written to disk.
#![forbid(unsafe_code)]

use super::Service;
use base64::Engine as _;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

pub(crate) const CAMERA_LABEL: &str = "camera-capture";
const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;
const RESULT_EVENT: &str = "photo-capture";

struct Session {
    token: String,
    /// The reader window that asked, and receives the result.
    target: String,
    /// The shell's request, echoed in every result so a photo can only reach
    /// the request that opened the camera.
    request_id: String,
}

#[derive(Default)]
pub(crate) struct PhotoCapture {
    session: Mutex<Option<Session>>,
    picking: AtomicBool,
}

/// Image MIME type from the file's first bytes; `None` for anything else.
pub(crate) fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    let starts = |prefix: &[u8]| bytes.starts_with(prefix);
    if starts(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if starts(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if starts(b"BM") && bytes.len() > 26 {
        Some("image/bmp")
    } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        match &bytes[8..12] {
            b"avif" | b"avis" => Some("image/avif"),
            b"heic" | b"heix" | b"hevc" | b"heim" | b"heis" | b"mif1" | b"msf1" => Some("image/heic"),
            _ => None,
        }
    } else {
        None
    }
}

/// Dialog extensions for an `accept` list: the image types it names, or every
/// supported image type for `image/*`. SVG is never offered (it is a script
/// container, not a photo).
pub(crate) fn accept_extensions(accept: &str) -> Vec<&'static str> {
    const ALL: [&str; 10] = ["jpg", "jpeg", "png", "webp", "gif", "bmp", "heic", "heif", "avif", "jfif"];
    let mut out = Vec::new();
    for part in accept.split(',').map(|part| part.trim().to_ascii_lowercase()) {
        let add: &[&'static str] = match part.as_str() {
            "image/*" => &ALL,
            "image/jpeg" | "image/jpg" | ".jpg" | ".jpeg" | ".jfif" => &["jpg", "jpeg", "jfif"],
            "image/png" | ".png" => &["png"],
            "image/webp" | ".webp" => &["webp"],
            "image/gif" | ".gif" => &["gif"],
            "image/bmp" | ".bmp" => &["bmp"],
            "image/heic" | "image/heif" | ".heic" | ".heif" => &["heic", "heif"],
            "image/avif" | ".avif" => &["avif"],
            _ => &[],
        };
        for extension in add {
            if !out.contains(extension) {
                out.push(*extension);
            }
        }
    }
    out
}

/// Whether `mime` (sniffed from content) is one the input accepts.
fn accepted(accept: &str, mime: &str) -> bool {
    accept.split(',').map(|part| part.trim().to_ascii_lowercase()).any(|part| {
        part == "image/*"
            || part == mime
            || (mime == "image/jpeg" && matches!(part.as_str(), "image/jpg" | ".jpg" | ".jpeg" | ".jfif"))
            || (mime == "image/heic" && matches!(part.as_str(), "image/heif" | ".heic" | ".heif"))
            || part.strip_prefix('.').is_some_and(|extension| mime == format!("image/{extension}"))
    })
}

/// The capture window can return JPEG or PNG. Pick the first the input
/// accepts, or refuse rather than handing the page an incompatible image.
fn camera_format(accept: &str) -> Result<&'static str, String> {
    if accepted(accept, "image/jpeg") {
        Ok("jpeg")
    } else if accepted(accept, "image/png") {
        Ok("png")
    } else {
        Err("This page does not accept a camera photo".into())
    }
}

fn modified_millis(metadata: &std::fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|elapsed| elapsed.as_millis() as u64)
}

impl Service {
    fn photo_app(&self) -> Result<tauri::AppHandle, String> {
        self.app
            .lock()
            .map_err(|_| "Application handle unavailable")?
            .clone()
            .ok_or_else(|| "No application window is available".to_string())
    }

    /// Whether the capture window may use the camera now.
    pub(crate) fn camera_capture_active(&self) -> bool {
        self.photo.session.lock().map(|session| session.is_some()).unwrap_or(false)
    }

    /// The native image dialog for the reader window `label`. `null` when the
    /// user cancels; otherwise the chosen image's name, type and bytes.
    pub(crate) async fn pick_image(&self, label: &str, args: &Value) -> Result<Value, String> {
        use tauri_plugin_dialog::DialogExt;
        let accept = args.get("accept").and_then(Value::as_str).unwrap_or("image/*").chars().take(1024).collect::<String>();
        let extensions = accept_extensions(&accept);
        if extensions.is_empty() {
            return Err("This page asks for a file type Kiwix cannot pick as a photo".into());
        }
        if self.photo.picking.swap(true, Ordering::SeqCst) {
            return Err("A file dialog is already open".into());
        }
        struct Release<'a>(&'a AtomicBool);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _release = Release(&self.photo.picking);
        let app = self.photo_app()?;
        let window = app.get_webview_window(label).ok_or("The reader window is gone")?;
        let picked = tauri::async_runtime::spawn_blocking(move || {
            app.dialog()
                .file()
                .set_parent(&window)
                .add_filter("Images", &extensions)
                .blocking_pick_file()
        })
        .await
        .map_err(|_| "The file dialog stopped unexpectedly")?;
        let Some(picked) = picked else { return Ok(Value::Null) };
        let path = picked.into_path().map_err(|_| "That file cannot be opened")?;
        tauri::async_runtime::spawn_blocking(move || {
            let metadata = std::fs::metadata(&path).map_err(|_| "That file cannot be opened".to_string())?;
            if !metadata.is_file() {
                return Err("That is not a file".to_string());
            }
            if metadata.len() > MAX_IMAGE_BYTES {
                return Err("That image is larger than 25 MB".to_string());
            }
            // Read at most one byte past the limit, in case the file grew.
            let mut bytes = Vec::new();
            let file = std::fs::File::open(&path).map_err(|_| "That file cannot be read".to_string())?;
            std::io::Read::read_to_end(&mut std::io::Read::take(file, MAX_IMAGE_BYTES + 1), &mut bytes)
                .map_err(|_| "That file cannot be read".to_string())?;
            if bytes.len() as u64 > MAX_IMAGE_BYTES {
                return Err("That image is larger than 25 MB".to_string());
            }
            let mime = sniff_image(&bytes).ok_or("That file is not a supported image")?;
            if !accepted(&accept, mime) {
                return Err("This page does not accept that kind of image".to_string());
            }
            let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "photo".into());
            Ok(json!({
                "name": name.chars().take(255).collect::<String>(),
                "mime": mime,
                "lastModified": modified_millis(&metadata),
                "data": base64::engine::general_purpose::STANDARD.encode(&bytes),
            }))
        })
        .await
        .map_err(|_| "Reading the image stopped unexpectedly".to_string())?
    }

    /// Open the capture window for the reader window `label`.
    pub(crate) fn camera_capture_open(&self, label: &str, args: &Value) -> Result<Value, String> {
        let app = self.photo_app()?;
        let request_id = args
            .get("requestId")
            .and_then(Value::as_str)
            .filter(|id| uuid::Uuid::parse_str(id).is_ok())
            .ok_or("Missing photo request")?
            .to_string();
        {
            let session = self.photo.session.lock().map_err(|_| "Camera unavailable")?;
            if let Some(current) = session.as_ref() {
                // Never bring up another window's camera for this request.
                return Err(if current.target == label {
                    "The camera is already open".into()
                } else {
                    "The camera is in use in another Kiwix window".into()
                });
            }
        }
        if app.get_webview_window(CAMERA_LABEL).is_some() {
            return Err("The camera is closing; try again in a moment".into());
        }
        let parent = app.get_webview_window(label).ok_or("The reader window is gone")?;
        let facing = if args.get("capture").and_then(Value::as_str) == Some("user") { "user" } else { "environment" };
        let accept = args.get("accept").and_then(Value::as_str).unwrap_or("image/*");
        let format = camera_format(accept)?;
        let token = uuid::Uuid::new_v4().to_string();
        let locale = self.store.settings().map(|settings| settings.locale).unwrap_or_default();
        let query: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("token", &token)
            .append_pair("facing", facing)
            .append_pair("format", format)
            .append_pair("locale", &locale)
            .finish();
        let builder = tauri::WebviewWindowBuilder::new(
            &app,
            CAMERA_LABEL,
            tauri::WebviewUrl::App(format!("camera.html?{query}").into()),
        )
        // English until the shell's catalogs reach native titles; the page's own
        // title is localized. Hidden until the app menu is removed (below).
        .title("Kiwix — Camera")
        .visible(false)
        .inner_size(720.0, 600.0)
        .min_inner_size(420.0, 380.0)
        .incognito(true)
        .parent(&parent)
        .map_err(|error| error.to_string())?
        // Only the bundled capture page, on the app's own origin.
        .on_navigation(|url| {
            let app_origin = matches!(
                (url.scheme(), url.host_str()),
                ("tauri", Some("localhost")) | ("http" | "https", Some("tauri.localhost"))
            );
            let dev_server = cfg!(debug_assertions) && url.scheme() == "http" && url.host_str() == Some("localhost");
            url.path() == "/camera.html" && (app_origin || dev_server)
        });
        // A portable profile keeps its WebView data beside the app, never on the host.
        let builder = match self.store.portable_data_dir() {
            Some(directory) => {
                let profile = crate::portable::PortableProfile { data: directory.to_path_buf() };
                let directory = profile.webview_directory(CAMERA_LABEL)?;
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                { builder.data_directory(directory) }
                #[cfg(not(any(target_os = "windows", target_os = "linux")))]
                { let _ = directory; builder }
            }
            None => builder,
        };
        // Build without holding the session lock: window creation waits on the
        // main thread, where the permission handler takes that lock.
        let window = builder
            .build()
            .map_err(|error| format!("The camera window could not open: {error}"))?;
        let Ok(mut session) = self.photo.session.lock() else {
            let _ = window.close();
            return Err("Camera unavailable".into());
        };
        if session.is_some() {
            let _ = window.close();
            return Err("The camera is already open".into());
        }
        *session = Some(Session { token, target: label.into(), request_id });
        drop(session);
        // Windows and Linux give every window the app menu, whose commands only
        // reach reader windows; in the capture window they would do nothing.
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        let _ = window.remove_menu();
        if let Err(error) = window.show() {
            // Never leave an invisible window holding the camera session.
            if let Ok(mut session) = self.photo.session.lock() { *session = None; }
            let _ = window.close();
            return Err(format!("The camera window could not open: {error}"));
        }
        let _ = window.set_focus();
        Ok(Value::Null)
    }

    /// From the capture window: the photo, or a cancellation. Whatever happens,
    /// the reader hears back (a failure arrives as a cancellation with its
    /// reason, so the user can still choose a file) and the window closes.
    pub(crate) fn camera_capture_finish(&self, label: &str, args: &Value, cancelled: bool) -> Result<Value, String> {
        if label != CAMERA_LABEL {
            return Err("Not available".into());
        }
        let token = args.get("token").and_then(Value::as_str).unwrap_or_default();
        let session = {
            let mut session = self.photo.session.lock().map_err(|_| "Camera unavailable")?;
            if session.as_ref().map(|current| current.token.as_str()) != Some(token) {
                return Err("This camera session has ended".into());
            }
            session.take().expect("checked above")
        };
        let photo: Result<Value, Option<String>> = if cancelled {
            Err(args.get("reason").and_then(Value::as_str).map(|reason| reason.chars().take(300).collect()))
        } else {
            (|| -> Result<Value, String> {
                let data = args.get("data").and_then(Value::as_str).ok_or("Missing photo")?;
                if data.len() as u64 > MAX_IMAGE_BYTES * 4 / 3 + 4 {
                    return Err("The photo is too large".into());
                }
                let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| "The photo is damaged")?;
                let mime = sniff_image(&bytes)
                    .filter(|mime| matches!(*mime, "image/jpeg" | "image/png"))
                    .ok_or("The photo is damaged")?;
                let captured = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
                let stamp = captured.format(time::macros::format_description!("[year][month][day]-[hour][minute][second]")).map_err(|e| e.to_string())?;
                let extension = if mime == "image/png" { "png" } else { "jpg" };
                Ok(json!({
                    "status": "photo",
                    "requestId": session.request_id,
                    "name": format!("photo-{stamp}.{extension}"),
                    "mime": mime,
                    "lastModified": (captured.unix_timestamp_nanos() / 1_000_000) as i64,
                    "data": data,
                }))
            })()
            .map_err(Some)
        };
        let payload = match &photo {
            Ok(payload) => payload.clone(),
            Err(reason) => json!({"status": "cancelled", "requestId": session.request_id, "reason": reason}),
        };
        let app = self.photo_app()?;
        let delivered = app.emit_to(tauri::EventTarget::webview_window(&session.target), RESULT_EVENT, payload);
        if let Some(window) = app.get_webview_window(CAMERA_LABEL) {
            let _ = window.close();
        }
        delivered.map_err(|error| error.to_string())?;
        match photo {
            Ok(_) => Ok(Value::Null),
            Err(_) if cancelled => Ok(Value::Null),
            Err(reason) => Err(reason.unwrap_or_default()),
        }
    }

    /// From the reader shell: close the capture window without a result.
    pub(crate) fn camera_capture_close(&self, label: &str) -> Result<Value, String> {
        let owned = {
            let mut session = self.photo.session.lock().map_err(|_| "Camera unavailable")?;
            if session.as_ref().is_some_and(|current| current.target == label) {
                session.take();
                true
            } else {
                false
            }
        };
        if owned {
            if let Ok(app) = self.photo_app() {
                if let Some(window) = app.get_webview_window(CAMERA_LABEL) {
                    let _ = window.close();
                }
            }
        }
        Ok(Value::Null)
    }

    /// The capture window was closed (its own X, or with its reader window).
    pub(crate) fn camera_capture_window_destroyed(&self) {
        let session = self.photo.session.lock().ok().and_then(|mut session| session.take());
        if let (Some(session), Ok(app)) = (session, self.photo_app()) {
            let _ = app.emit_to(
                tauri::EventTarget::webview_window(&session.target),
                RESULT_EVENT,
                json!({"status": "cancelled", "requestId": session.request_id, "reason": null}),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_recognised_by_content_not_name() {
        assert_eq!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff_image(b"\0\0\0\x18ftypheic\0\0"), Some("image/heic"));
        assert_eq!(sniff_image(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), None);
        assert_eq!(sniff_image(b"MZ\x90\0"), None);
        assert_eq!(sniff_image(b""), None);
    }

    #[test]
    fn accept_lists_map_to_dialog_extensions_and_content_checks() {
        assert!(accept_extensions("image/*").contains(&"jpg"));
        assert_eq!(accept_extensions("image/png"), vec!["png"]);
        assert_eq!(accept_extensions(".jpg, image/webp"), vec!["jpg", "jpeg", "jfif", "webp"]);
        assert!(accept_extensions("application/pdf").is_empty());
        assert!(accept_extensions("image/svg+xml").is_empty());
        assert!(accepted("image/*", "image/heic"));
        assert!(accepted("image/jpeg", "image/jpeg"));
        assert!(accepted(".jpg", "image/jpeg"));
        assert!(!accepted("image/png", "image/jpeg"));
    }

    #[test]
    fn camera_returns_only_a_type_the_input_accepts() {
        assert_eq!(camera_format("image/*"), Ok("jpeg"));
        assert_eq!(camera_format("image/jpeg,image/png"), Ok("jpeg"));
        assert_eq!(camera_format("image/png"), Ok("png"));
        assert_eq!(camera_format(".png"), Ok("png"));
        assert!(camera_format("image/gif").is_err());
        assert!(camera_format("image/webp,image/avif").is_err());
        assert!(camera_format("image/svg+xml").is_err());
    }
}
