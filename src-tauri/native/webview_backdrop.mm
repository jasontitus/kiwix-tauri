// SPDX-License-Identifier: GPL-3.0-or-later
//
// WebKit paints an "under-page" background behind page content: the colour
// visible while a frame's document is being replaced and in overscroll areas.
// It defaults to the platform's light system colour, so a dark reader flashes
// grey on every navigation no matter what the document itself paints. The page
// cannot set it, and the setters are only public above macOS 12 / iOS 15, so
// the shell reaches them here.
//
// Only `underPageBackgroundColor` is touched. `drawsBackground` stays at its
// default: turning it off would make the whole webview non-opaque and let the
// window background show through wherever a document paints nothing, which is
// a larger change than the frame gap this fixes.

#import <Foundation/Foundation.h>
#import <TargetConditionals.h>
#if TARGET_OS_IPHONE
#import <UIKit/UIKit.h>
#else
#import <AppKit/AppKit.h>
#endif
#import <WebKit/WebKit.h>

extern "C" void kiwix_webview_set_under_page_background(void *webview, double red, double green, double blue) {
  if (webview == nullptr) return;
  WKWebView *view = (__bridge WKWebView *)webview;
#if TARGET_OS_IPHONE
  if (@available(iOS 15.0, *)) {
    view.underPageBackgroundColor = [UIColor colorWithRed:red green:green blue:blue alpha:1.0];
  }
#else
  if (@available(macOS 12.0, *)) {
    view.underPageBackgroundColor = [NSColor colorWithSRGBRed:red green:green blue:blue alpha:1.0];
  }
#endif
}

// The appearance this webview resolves `prefers-color-scheme` from, which is
// the only authority for what "system" means in this window.
extern "C" int kiwix_webview_appearance_is_dark(void *webview) {
  if (webview == nullptr) return 0;
  WKWebView *view = (__bridge WKWebView *)webview;
#if TARGET_OS_IPHONE
  return view.traitCollection.userInterfaceStyle == UIUserInterfaceStyleDark ? 1 : 0;
#else
  NSAppearance *appearance = view.effectiveAppearance;
  NSString *match = [appearance bestMatchFromAppearancesWithNames:@[
    NSAppearanceNameAqua, NSAppearanceNameDarkAqua
  ]];
  return [match isEqualToString:NSAppearanceNameDarkAqua] ? 1 : 0;
#endif
}
