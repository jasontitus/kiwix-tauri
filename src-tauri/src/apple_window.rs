// SPDX-License-Identifier: GPL-3.0-or-later
//! A retained AppKit window for native bridges that may outlive the call.
use objc2::{rc::Retained, MainThreadMarker};
use objc2_app_kit::NSWindow;

#[repr(transparent)]
pub(crate) struct WindowRef(Retained<NSWindow>);

impl WindowRef {
    pub(crate) fn new(window: &tauri::WebviewWindow) -> Result<Self, String> {
        MainThreadMarker::new().ok_or("The AppKit window is only available on the main thread")?;
        let pointer = window.ns_window().map_err(|error| error.to_string())?;
        // SAFETY: this is the main thread and Tauri returns a live autoreleased
        // NSWindow for this WebviewWindow. `retain` adds the one reference this
        // wrapper owns; native bridges add their own reference before storing it.
        let window = unsafe { Retained::retain(pointer.cast::<NSWindow>()) }
            .ok_or("The AppKit window is unavailable")?;
        Ok(Self(window))
    }

    pub(crate) fn window(&self) -> &NSWindow {
        &self.0
    }
}
