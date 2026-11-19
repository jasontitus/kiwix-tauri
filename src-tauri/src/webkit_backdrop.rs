// SPDX-License-Identifier: GPL-3.0-or-later
//! Change only the colour WebKit paints underneath the page.

use objc2_app_kit::{
    NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSColor,
};
use objc2_foundation::NSArray;
use objc2_web_kit::WKWebView;

pub(super) fn is_dark(view: &WKWebView) -> bool {
    // SAFETY: both constants are immutable AppKit-owned appearance names
    // provided on the app's supported macOS versions (13.3 and later).
    let (aqua, dark) = unsafe { (NSAppearanceNameAqua, NSAppearanceNameDarkAqua) };
    view.effectiveAppearance()
        .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[aqua, dark]))
        .is_some_and(|name| name.isEqualToString(dark))
}

pub(super) fn set_colour(view: &WKWebView, (red, green, blue): (u8, u8, u8)) {
    if objc2::available!(macos = 12.0) {
        let colour = NSColor::colorWithSRGBRed_green_blue_alpha(
            f64::from(red) / 255.0,
            f64::from(green) / 255.0,
            f64::from(blue) / 255.0,
            1.0,
        );
        // SAFETY: the supported WebKit selector copies a non-null AppKit
        // colour. The main-thread WKWebView and colour live through the call.
        unsafe { view.setUnderPageBackgroundColor(Some(&colour)) };
    }
}
