// SPDX-License-Identifier: GPL-3.0-or-later
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;
pub use platform::render;
