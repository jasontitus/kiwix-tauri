// SPDX-License-Identifier: GPL-3.0-or-later
//! Open a file, folder or web/mail link with the system's default handler.
//!
//! On macOS this goes through NSWorkspace rather than spawning `/usr/bin/open`:
//! inside the App Sandbox, NSWorkspace hands the receiving application a
//! sandbox extension for the file, so it can read attachments that Kiwix
//! extracted into its container. Elsewhere the `opener` crate is used.
#![forbid(unsafe_code)]
use std::ffi::OsStr;

pub(crate) fn open(target: impl AsRef<OsStr>) -> Result<(), String> {
    let target = target.as_ref();
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::{NSString, NSURL};
        let text = target.to_str().ok_or("The location is not valid UTF-8")?;
        // Callers run on worker threads, which have no autorelease pool.
        objc2::rc::autoreleasepool(|_| {
        let link = url::Url::parse(text)
            .ok()
            .filter(|url| matches!(url.scheme(), "http" | "https" | "mailto"));
        let ns_url = match link {
            Some(link) => NSURL::URLWithString(&NSString::from_str(link.as_str()))
                .ok_or("macOS rejected the link")?,
            None => NSURL::fileURLWithPath(&NSString::from_str(text)),
        };
        if NSWorkspace::sharedWorkspace().openURL(&ns_url) {
            Ok(())
        } else {
            Err("macOS could not find an application to open it".into())
        }
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        opener::open(target).map_err(|error| error.to_string())
    }
}
