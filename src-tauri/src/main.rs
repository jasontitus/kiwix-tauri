// SPDX-License-Identifier: GPL-3.0-or-later
// The binary is a pure entry point: `kiwix_app::run()` owns every platform
// boundary, so this crate allows no unsafe of its own.
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    kiwix_app::run();
}
