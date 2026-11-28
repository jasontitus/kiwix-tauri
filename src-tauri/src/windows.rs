// SPDX-License-Identifier: GPL-3.0-or-later
//! Native geometry lives in the same transactional, privacy-aware profile as tabs.
use crate::store::{State, Store};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

#[cfg(any(target_os = "macos", test))]
#[path = "native_tabs.rs"]
mod native_tabs;

#[cfg(target_os = "macos")]
#[path = "webkit_backdrop.rs"]
mod webkit_backdrop;

/// How long a new shell may take to commit its first usable snapshot before
/// the native side paints diagnostics over the pre-bundle placeholder.
const FRONTEND_READY_DEADLINE: Duration = Duration::from_secs(10);

/// Shared eligibility for AppKit tabbing; actual membership/order/selection is
/// captured separately and restored after all saved native windows exist.
#[cfg(target_os = "macos")]
pub const TABBING_IDENTIFIER: &str = "org.kiwix.workbench.reader";

#[cfg(target_os = "macos")]
mod tabbing {
    use objc2::rc::Retained;
    use objc2_app_kit::NSWindow;
    use objc2_foundation::{NSData, NSString};

    #[deny(improper_ctypes)]
    unsafe extern "C" {
        pub safe fn kiwix_tabbing_install_early();
        // The window is live for this main-thread call. Native copies the
        // label and associates it with the window itself.
        pub safe fn kiwix_tabbing_prepare_window(window: &NSWindow, label: &NSString);
        pub safe fn kiwix_tabbing_install_menu();
        // Native decodes these immutable bytes synchronously.
        pub safe fn kiwix_tabbing_update_menu(labels: &NSData) -> bool;
        pub safe fn kiwix_tabbing_restore(groups: &NSData) -> bool;
        // NS_RETURNS_RETAINED transfers one owned Foundation reference.
        pub safe fn kiwix_tabbing_snapshot() -> Option<Retained<NSData>>;
    }
}

/// AppKit tabbing must be allowed before the first window exists.
#[cfg(target_os = "macos")]
pub fn install_native_tabbing() {
    tabbing::kiwix_tabbing_install_early();
}

/// Register a shell for native tabbing and session capture. Safe to repeat.
#[cfg(target_os = "macos")]
pub fn adopt_native_tabbing(window: &WebviewWindow) {
    use objc2_foundation::NSString;

    if !native_tabs::shell_label(window.label()) {
        return;
    }
    let label = window.label().to_owned();
    let _ = window.with_webview(move |webview| {
        // The main-thread callback lends only the live window pointer; AppKit
        // supplies the retained NSWindow from its own window list.
        let Some(mtm) = objc2::MainThreadMarker::new() else { return; };
        let Some(window) = crate::apple_window::window_from_pointer(webview.ns_window(), mtm) else { return; };
        tabbing::kiwix_tabbing_prepare_window(&window, &NSString::from_str(&label));
    });
}

/// Window > Show Previous/Next Tab, Merge All Windows and Show Tab Bar are
/// AppKit responder-chain actions, so they are installed natively rather than
/// through muda.
#[cfg(target_os = "macos")]
pub fn install_tabbing_menu() {
    tabbing::kiwix_tabbing_install_menu();
}

/// Refresh on the same locale-update path as the application menu, without
/// allowing the webview to supply native selectors or menu identifiers.
#[cfg(target_os = "macos")]
pub fn update_tabbing_menu(locale: &str) -> Result<(), String> {
    let strings = crate::localization::Localizer::for_app(locale);
    let sources = [
        "Window", "Show Previous Tab", "Show Next Tab", "Merge All Windows",
        "Move Tab to New Window", "Show Tab Bar", "Hide Tab Bar", "Minimize",
        "Zoom", "Bring All to Front",
    ];
    let labels: HashMap<_, _> = sources.into_iter()
        .map(|source| (source, strings.text(source, &[]))).collect();
    let json = serde_json::to_vec(&labels).map_err(|e| e.to_string())?;
    if !tabbing::kiwix_tabbing_update_menu(&objc2_foundation::NSData::with_bytes(&json)) {
        return Err("Native window menu labels could not be updated".into());
    }
    Ok(())
}

/// Apply only after every window in the restore batch exists. In particular,
/// manual restore must pass its restored sessions, not all live sessions.
pub fn restore_native_tabs(
    app: &AppHandle,
    sessions: &HashMap<String, crate::store::WindowSession>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let open = app.webview_windows().into_keys().collect();
        let groups = native_tabs::restore_plan(sessions, &open);
        if groups.is_empty() {
            return Ok(());
        }
        let json = serde_json::to_vec(&groups).map_err(|e| e.to_string())?;
        if !tabbing::kiwix_tabbing_restore(&objc2_foundation::NSData::with_bytes(&json)) {
            return Err("Native window tab groups could not be restored".into());
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, sessions);
    Ok(())
}

#[cfg(target_os = "macos")]
fn capture_native_tabs() -> Result<Vec<native_tabs::Member>, String> {
    let data = tabbing::kiwix_tabbing_snapshot()
        .ok_or("Native window tab groups could not be captured")?;
    serde_json::from_slice(&data.to_vec())
        .map_err(|e| format!("Kiwix could not read the current window groups: {e}"))
}

/// The shell's own paper, the colour every shell surface paints (`--paper` in
/// src/styles.css). `system` is resolved by the webview's appearance, which is
/// also what its `prefers-color-scheme` media query uses.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn shell_paper(theme: &str, dark: bool) -> (u8, u8, u8) {
    match theme {
        "dark" => (0x00, 0x00, 0x00),
        "light" => (0xf2, 0xf2, 0xf7),
        _ if dark => (0x00, 0x00, 0x00),
        _ => (0xf2, 0xf2, 0xf7),
    }
}

/// The page colour `reader-bridge.js` paints for each reader theme. `original`
/// is absent on purpose: it keeps the archive's own colours, so there is no
/// paper to match and the shell paper stands in.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn reader_paper(theme: &str) -> Option<(u8, u8, u8)> {
    match theme {
        "light" => Some((0xff, 0xff, 0xff)),
        "sepia" => Some((0xf4, 0xec, 0xd8)),
        "dark" => Some((0x17, 0x1a, 0x1e)),
        _ => None,
    }
}

/// Labels whose shell currently shows an archive document. The under-page
/// colour is the reader's, so the shell paper returns with the library.
static READER_SURFACES: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

pub fn set_reader_surface(label: &str, reader: bool) {
    if let Ok(mut surfaces) = READER_SURFACES.lock() {
        if reader {
            surfaces.insert(label.to_string());
        } else {
            surfaces.remove(label);
        }
    }
}

/// A window that goes away cannot keep a reader surface; the label may be
/// reused by a restored session, which starts on the library.
pub fn forget_window(label: &str) {
    set_reader_surface(label, false);
}

#[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(dead_code))]
pub(crate) fn has_reader_surface(label: &str) -> bool {
    READER_SURFACES
        .lock()
        .map(|surfaces| surfaces.contains(label))
        .unwrap_or(false)
}

/// Set WebKit's under-page background: the layer it paints while a frame's
/// document is being replaced. Its default is the platform's light system
/// colour, which is the light flash a dark reader showed on every navigation.
///
/// This is deliberately narrower than `Webview::set_background_color`, which
/// also flips `drawsBackground` off (making the webview non-opaque) and, on
/// iOS, never reaches `underPageBackgroundColor` at all. The value follows the
/// reader paper only while a reader surface is on screen, so a light shell
/// never paints a dark first frame at launch or after leaving the reader.
pub fn apply_under_page(window: &WebviewWindow, settings: &crate::store::Settings) {
    #[cfg(target_os = "macos")]
    {
        let reader = has_reader_surface(window.label());
        let reader_theme = settings.reader_theme.clone();
        let shell_theme = settings.theme.clone();
        let _ = window.with_webview(move |webview| {
            // The main-thread callback lends only the live window pointer; the
            // shell's WKWebView comes from AppKit's own view tree.
            let _ = crate::apple_window::with_webkit_view(webview.ns_window(), |view| {
                let colour = reader
                    .then(|| reader_paper(&reader_theme))
                    .flatten()
                    .unwrap_or_else(|| shell_paper(&shell_theme, webkit_backdrop::is_dark(view)));
                webkit_backdrop::set_colour(view, colour);
            });
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Only WebKit owns an under-page layer; the other engines paint their
        // own platform background and have no equivalent to keep in sync.
        let _ = (window, settings);
    }
}

/// Re-apply to every window after a theme change or at startup.
pub fn apply_under_page_all(app: &AppHandle, settings: &crate::store::Settings) {
    #[cfg(target_os = "macos")]
    for window in app.webview_windows().values() {
        apply_under_page(window, settings);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, settings);
}

pub fn create(
    app: &AppHandle,
    label: &str,
    geometry: Option<&Value>,
) -> Result<WebviewWindow, tauri::Error> {
    let nav_app = app.clone();
    let builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title("Kiwix")
        .incognito(true)
        .on_navigation(move |url| {
            let service = nav_app
                .try_state::<std::sync::Arc<crate::service::Service>>();
            crate::navigation_allowed(url, |origin| {
                service.as_ref().is_some_and(|state| state.has_reader_origin(origin))
            })
        })
        .inner_size(1280.0, 850.0)
        .min_inner_size(760.0, 520.0);
    // Matches the main window in tauri.conf.json; src/platform.css insets the
    // shell around the traffic lights.
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(LogicalPosition::new(16.0, 13.0))
        // Native tabs group whole shells; each shell separately owns reader tabs.
        .tabbing_identifier(TABBING_IDENTIFIER);
    let builder = if let Some(service) = app.try_state::<std::sync::Arc<crate::service::Service>>() {
        if let Some(directory) = service.store.portable_data_dir() {
            service.store.validate_portable_directory(directory).map_err(std::io::Error::other)?;
            let profile = crate::portable::PortableProfile { data: directory.to_path_buf() };
            let directory = profile.webview_directory(label).map_err(std::io::Error::other)?;
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            { builder.data_directory(directory) }
            #[cfg(not(any(target_os = "windows", target_os = "linux")))]
            { let _ = directory; builder }
        } else { builder }
    } else { builder };
    let window = builder.build()?;
    // A fresh window starts on the library, whatever its label meant before.
    forget_window(label);
    if let Some(service) = app.try_state::<std::sync::Arc<crate::service::Service>>() {
        if let Ok(settings) = service.store.settings() {
            apply_under_page(&window, &settings);
        }
    }
    #[cfg(target_os = "macos")]
    adopt_native_tabbing(&window);
    if let Some(geometry) = geometry {
        restore(&window, geometry);
    }
    watch_startup(app, label);
    Ok(window)
}

/// If this shell has not acknowledged a usable first snapshot in time, paint
/// an actionable panel in that window and leave a local diagnostic. The page
/// itself owns the panel so it can include asset/IPC/snapshot details.
pub fn watch_startup(app: &AppHandle, label: &str) {
    let app = app.clone();
    let label = label.to_string();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FRONTEND_READY_DEADLINE).await;
        let Some(service) = app.try_state::<Arc<crate::service::Service>>() else {
            return;
        };
        if service.is_frontend_ready(&label) {
            return;
        }
        crate::diagnostics::record(
            &service.store,
            "frontend_ready",
            Some("Frontend did not report ready within 10 seconds"),
        );
        eprintln!(
            "Frontend startup watchdog: window {label} did not report ready within 10 seconds"
        );
        let Some(window) = app.get_webview_window(&label) else {
            return;
        };
        let payload = json!({
            "source": "native-watchdog",
            "window": label,
            "reason": "Kiwix did not finish starting in time. Close this window and open Kiwix again.",
            "deadlineSeconds": FRONTEND_READY_DEADLINE.as_secs(),
        });
        if let Err(error) = window.eval(startup_failure_script(&payload)) {
            eprintln!("Frontend startup watchdog could not paint window {label}: {error}");
            crate::diagnostics::record(
                &service.store,
                "frontend_ready",
                Some("Native watchdog could not evaluate the diagnostic panel"),
            );
        } else {
            crate::diagnostics::record(&service.store, "frontend_ready", None);
        }
    });
}

/// Report a failed native window creation instead of dropping it on the floor.
/// Startup restore keeps going: one bad saved window must not deny the rest.
pub fn report_window_creation_failure(app: &AppHandle, label: &str, error: &str) {
    eprintln!("Window {label} could not be created: {error}");
    if let Some(service) = app.try_state::<Arc<crate::service::Service>>() {
        crate::diagnostics::record(
            &service.store,
            "application_operation",
            Some("A native window could not be created"),
        );
        service.note_startup_warning(format!("A window could not be restored: {error}"));
    }
}

/// Prefer the page's rich diagnostic panel; fall back to a self-contained one
/// when boot-guard.js itself failed to load. Both are opaque and theme-aware.
fn startup_failure_script(payload: &Value) -> String {
    let payload_json = serde_json::to_string(payload).unwrap_or_else(|_| "{}".into());
    let reason = payload
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("Kiwix did not finish starting. Close this window and open Kiwix again.");
    let reason_json =
        serde_json::to_string(reason).unwrap_or_else(|_| "\"Kiwix did not finish starting.\"".into());
    format!(
        r##"(function () {{
  try {{
    if (typeof window.__kiwixShowStartupFailure === "function") {{
      window.__kiwixShowStartupFailure({payload_json});
      return;
    }}
    var dark = document.documentElement.dataset.theme === "dark" ||
      (document.documentElement.dataset.theme === "system" &&
        window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches);
    var panel = document.createElement("div");
    panel.id = "kiwix-startup-failure";
    panel.setAttribute("role", "alertdialog");
    panel.style.cssText = "position:fixed;inset:0;z-index:2147483647;display:flex;align-items:center;justify-content:center;padding:24px;background:" +
      (dark ? "#000" : "#f2f2f7") + ";color:" + (dark ? "#f2f2f7" : "#1c1c1e") +
      ";font:14px -apple-system,BlinkMacSystemFont,sans-serif";
    var card = document.createElement("div");
    card.style.cssText = "max-width:560px;padding:24px;border-radius:16px;background:" +
      (dark ? "#1c1c1e" : "#fff") + ";border:1px solid " + (dark ? "#3a3a3c" : "#d1d1d6");
    var title = document.createElement("h1");
    title.textContent = "Kiwix could not finish starting";
    title.style.cssText = "margin:0 0 10px;font-size:20px";
    var reason = document.createElement("p");
    reason.textContent = {reason_json};
    reason.style.cssText = "margin:0 0 16px;line-height:1.5";
    var actions = document.createElement("div");
    var reload = document.createElement("button");
    reload.textContent = "Reload";
    reload.style.cssText = "min-height:34px;padding:6px 14px;border-radius:9px;border:1px solid " +
      (dark ? "#7ab8ff" : "#0066cc") + ";background:" + (dark ? "#7ab8ff" : "#0066cc") +
      ";color:" + (dark ? "#000" : "#fff") + ";font:inherit;font-weight:600;cursor:pointer";
    reload.onclick = function () {{ window.location.reload(); }};
    actions.appendChild(reload);
    card.appendChild(title);
    card.appendChild(reason);
    card.appendChild(actions);
    panel.appendChild(card);
    document.body.appendChild(panel);
  }} catch (error) {{
    console.error("Kiwix startup diagnostics failed", error);
  }}
}})();"##
    )
}

/// Adopt the config-created main window and any window built before setup.
#[cfg(target_os = "macos")]
pub fn adopt_existing(app: &AppHandle) {
    for window in app.webview_windows().values() {
        adopt_native_tabbing(window);
    }
}

pub fn restore_main(app: &AppHandle, state: &State) {
    if !state.settings.restore_session {
        return;
    }
    if let (Some(window), Some(geometry)) = (
        app.get_webview_window("main"),
        state.windows.get("main").and_then(|w| w.geometry.as_ref()),
    ) {
        restore(&window, geometry);
    }
}

fn restore(window: &WebviewWindow, geometry: &Value) {
    let number = |key: &str| {
        geometry
            .get(key)
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
    };
    let (Some(width), Some(height), Some(x), Some(y)) =
        (number("width"), number("height"), number("x"), number("y"))
    else {
        return;
    };
    let width = width.clamp(760.0, 6000.0);
    let height = height.clamp(520.0, 4000.0);
    let _ = window.set_size(LogicalSize::new(width, height));
    let visible = window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .any(|monitor| {
            let scale = monitor.scale_factor();
            let mx = monitor.position().x as f64 / scale;
            let my = monitor.position().y as f64 / scale;
            let mw = monitor.size().width as f64 / scale;
            let mh = monitor.size().height as f64 / scale;
            x < mx + mw && x + width > mx && y < my + mh && y + height > my
        });
    if visible {
        let _ = window.set_position(LogicalPosition::new(x, y));
    } else {
        let _ = window.center();
    }
    if geometry.get("maximized").and_then(Value::as_bool) == Some(true) {
        let _ = window.maximize();
    }
    if geometry.get("fullscreen").and_then(Value::as_bool) == Some(true) {
        let _ = window.set_fullscreen(true);
    }
}

pub fn capture(app: &AppHandle, store: &Store) -> Result<(), String> {
    let settings = store.settings()?;
    if settings.private_mode || settings.read_only {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    let native_tabs = capture_native_tabs()?;
    let mut geometry = Vec::new();
    for (label, window) in app.webview_windows() {
        let (Ok(position), Ok(size), Ok(scale)) = (
            window.outer_position(),
            window.inner_size(),
            window.scale_factor(),
        ) else {
            continue;
        };
        geometry.push((label, Observed {
            x: position.x as f64 / scale,
            y: position.y as f64 / scale,
            width: size.width as f64 / scale,
            height: size.height as f64 / scale,
            minimized: window.is_minimized().unwrap_or(false),
            maximized: window.is_maximized().unwrap_or(false),
            fullscreen: window.is_fullscreen().unwrap_or(false),
        }));
    }
    persist_capture(
        store,
        geometry,
        #[cfg(target_os = "macos")]
        native_tabs,
    )
}

/// A window's frame as the platform reports it, in logical pixels.
#[derive(Clone, Copy, Debug)]
struct Observed {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    minimized: bool,
    maximized: bool,
    fullscreen: bool,
}

/// The geometry to save for `observed`, given what was saved before. A
/// minimized window saves nothing: Windows parks it at (-32000, -32000) with
/// an icon-sized or empty client area, which `restore` would clamp to the
/// minimum size and centre. A maximized or fullscreen window fills its
/// screen, so it keeps the earlier normal size (its position still names the
/// screen it is on), and un-maximizing after a relaunch returns to that size.
fn captured_geometry(observed: Observed, previous: Option<&Value>) -> Option<Value> {
    let Observed { x, y, width, height, minimized, maximized, fullscreen } = observed;
    if minimized || ![x, y, width, height].iter().all(|value| value.is_finite()) || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let normal = previous.and_then(|previous| {
        let number = |key: &str| previous.get(key).and_then(Value::as_f64).filter(|value| value.is_finite() && *value > 0.0);
        Some((number("width")?, number("height")?))
    });
    let (width, height) = match normal {
        Some(normal) if maximized || fullscreen => normal,
        _ => (width, height),
    };
    Some(json!({"x":x,"y":y,"width":width,"height":height,"maximized":maximized,"fullscreen":fullscreen}))
}

fn persist_capture(
    store: &Store,
    geometry: Vec<(String, Observed)>,
    #[cfg(target_os = "macos")] native_tabs: Vec<native_tabs::Member>,
) -> Result<(), String> {
    store.update_window_capture(|retired, state| {
        if state.settings.private_mode || state.settings.read_only {
            return Ok(());
        }
        for (label, observed) in geometry {
            if retired.contains(&label) {
                continue;
            }
            if label == "main" {
                let session = state.windows.entry(label).or_default();
                if let Some(geometry) = captured_geometry(observed, session.geometry.as_ref()) {
                    session.geometry = Some(geometry);
                }
            } else if let Some(session) = state.windows.get_mut(&label) {
                if let Some(geometry) = captured_geometry(observed, session.geometry.as_ref()) {
                    session.geometry = Some(geometry);
                }
            }
        }
        #[cfg(target_os = "macos")]
        for member in native_tabs {
            if native_tabs::shell_label(&member.label) && !retired.contains(&member.label) {
                let session = state.windows.entry(member.label).or_default();
                session.native_tab = Some(crate::store::NativeWindowTab {
                    group: member.group,
                    index: member.index,
                    selected: member.selected,
                    focused: member.focused,
                });
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn window_retirement_excludes_a_late_capture_and_quit_snapshot() {
        let profile = tempfile::tempdir().unwrap();
        let service = crate::service::Service::new(profile.path().to_owned(), false, false).unwrap();
        let row = |label: &str| native_tabs::Member {
            label: label.into(),
            group: label.into(),
            index: 0,
            selected: true,
            focused: false,
        };
        let captured = vec![row("window-closed"), row("window-live")];
        service.clone().dispatch("close_window", json!({"__windowLabel":"window-closed"}))
            .await.unwrap();

        persist_capture(&service.store, Vec::new(), captured).unwrap();
        let state = service.store.read().unwrap();
        assert!(!state.windows.contains_key("window-closed"));
        assert!(state.windows["window-live"].tabs.is_empty());
        assert!(state.windows["window-live"].native_tab.is_some());

        // Explicit snapshots require at least one reader tab somewhere.
        service.store.update(|state| {
            state.tabs.push(crate::store::Tab {
                id: "main-tab".into(), book_id: None, path: String::new(), title: String::new(),
                scroll: 0.0, restricted: false, back: Vec::new(), forward: Vec::new(),
            });
            Ok(())
        }).unwrap();
        let previously_open = ["main".into(), "window-closed".into(), "window-live".into()]
            .into_iter().collect();
        assert!(service.store.save_session_snapshot(&previously_open).unwrap());
        let snapshot = service.store.saved_session().unwrap().unwrap();
        assert!(!snapshot.windows.contains_key("window-closed"));
        assert!(snapshot.windows.contains_key("window-live"));
        service.shutdown().await.unwrap();
    }

    fn observed(x: f64, y: f64, width: f64, height: f64) -> Observed {
        Observed { x, y, width, height, minimized: false, maximized: false, fullscreen: false }
    }

    #[test]
    fn a_minimized_window_keeps_its_last_saved_frame() {
        let saved = json!({"x":120.0,"y":80.0,"width":1180.0,"height":820.0,"maximized":false,"fullscreen":false});
        // What Windows reports for a minimized window: parked, icon-sized.
        let parked = Observed { minimized: true, ..observed(-32000.0, -32000.0, 160.0, 28.0) };
        assert_eq!(captured_geometry(parked, Some(&saved)), None);
        assert_eq!(captured_geometry(parked, None), None);
        // Some platforms leave the flag unset but still report an empty client area.
        assert_eq!(captured_geometry(observed(-32000.0, -32000.0, 0.0, 0.0), Some(&saved)), None);
        assert_eq!(captured_geometry(observed(10.0, 10.0, f64::NAN, 500.0), Some(&saved)), None);
        // An ordinary frame is saved as observed.
        assert_eq!(
            captured_geometry(observed(200.0, 100.0, 900.0, 700.0), Some(&saved)),
            Some(json!({"x":200.0,"y":100.0,"width":900.0,"height":700.0,"maximized":false,"fullscreen":false}))
        );
    }

    #[test]
    fn a_maximized_window_keeps_its_normal_size_on_its_current_screen() {
        let saved = json!({"x":120.0,"y":80.0,"width":1180.0,"height":820.0,"maximized":false,"fullscreen":false});
        let maximized = Observed { maximized: true, ..observed(1912.0, -8.0, 1920.0, 1040.0) };
        assert_eq!(
            captured_geometry(maximized, Some(&saved)),
            Some(json!({"x":1912.0,"y":-8.0,"width":1180.0,"height":820.0,"maximized":true,"fullscreen":false}))
        );
        let fullscreen = Observed { fullscreen: true, ..observed(0.0, 0.0, 1920.0, 1080.0) };
        assert_eq!(captured_geometry(fullscreen, Some(&saved)).unwrap()["width"], 1180.0);
        // With no earlier frame the screen-sized one is all there is.
        assert_eq!(captured_geometry(maximized, None).unwrap()["width"], 1920.0);
    }

    #[test]
    fn shell_paper_follows_the_saved_theme_and_the_webview_appearance() {
        assert_eq!(shell_paper("dark", false), (0x00, 0x00, 0x00));
        assert_eq!(shell_paper("light", true), (0xf2, 0xf2, 0xf7));
        // "system" is whatever the webview resolves `prefers-color-scheme` to,
        // so the under-page colour cannot disagree with the document.
        assert_eq!(shell_paper("system", true), (0x00, 0x00, 0x00));
        assert_eq!(shell_paper("system", false), (0xf2, 0xf2, 0xf7));
        assert_eq!(shell_paper("chartreuse", false), (0xf2, 0xf2, 0xf7));
    }

    #[test]
    fn reader_themes_paint_their_own_paper_and_original_has_none() {
        assert_eq!(reader_paper("light"), Some((0xff, 0xff, 0xff)));
        assert_eq!(reader_paper("sepia"), Some((0xf4, 0xec, 0xd8)));
        assert_eq!(reader_paper("dark"), Some((0x17, 0x1a, 0x1e)));
        // `original` keeps the archive's own colours; nothing to match.
        assert_eq!(reader_paper("original"), None);
    }

    #[test]
    fn a_reader_surface_belongs_to_one_window_and_ends_with_it() {
        set_reader_surface("backdrop-test-reader", true);
        assert!(has_reader_surface("backdrop-test-reader"));
        assert!(!has_reader_surface("backdrop-test-library"));
        forget_window("backdrop-test-reader");
        assert!(!has_reader_surface("backdrop-test-reader"));
    }
}
