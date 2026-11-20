// SPDX-License-Identifier: GPL-3.0-or-later
//! A retained AppKit window for native bridges that may outlive the call.
use objc2::{rc::Retained, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSWindow};
use objc2_web_kit::WKWebView;
use std::ffi::c_void;

/// Resolve a live window pointer to an owned AppKit reference. AppKit registers
/// every window it creates in `NSApplication.windows`, so an exact pointer match
/// needs no dereference of the caller's raw pointer.
pub(crate) fn window_from_pointer(pointer: *mut c_void, mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
    NSApplication::sharedApplication(mtm).windows().iter()
        .find(|window| Retained::as_ptr(window).cast_mut().cast::<c_void>() == pointer.cast::<c_void>())
}

/// Run `f` on the WKWebView that belongs to the window named by `pointer`. The
/// shell creates one webview per window; a window that hosts none yields `None`.
pub(crate) fn with_webkit_view<R>(pointer: *mut c_void, f: impl FnOnce(&WKWebView) -> R) -> Option<R> {
    let mtm = MainThreadMarker::new()?;
    let window = window_from_pointer(pointer, mtm)?;
    let mut stack = vec![window.contentView()?];
    while let Some(view) = stack.pop() {
        match view.downcast::<WKWebView>() {
            Ok(webview) => return Some(f(&webview)),
            Err(view) => stack.extend(view.subviews().to_vec()),
        }
    }
    None
}

#[repr(transparent)]
pub(crate) struct WindowRef(Retained<NSWindow>);

impl WindowRef {
    pub(crate) fn new(window: &tauri::WebviewWindow) -> Result<Self, String> {
        let mtm = MainThreadMarker::new().ok_or("The AppKit window is only available on the main thread")?;
        let pointer = window.ns_window().map_err(|error| error.to_string())?;
        // The returned reference is owned here, and native bridges add their own
        // reference before storing it.
        let window = window_from_pointer(pointer, mtm).ok_or("The AppKit window is unavailable")?;
        Ok(Self(window))
    }

    pub(crate) fn window(&self) -> &NSWindow {
        &self.0
    }
}
