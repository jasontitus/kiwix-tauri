// SPDX-License-Identifier: GPL-3.0-or-later
// Compile on macOS with clang++ -std=c++17 -fobjc-arc -fblocks -framework AppKit.
#include "../src-tauri/native/tabbing.mm"
#include <cstdio>

static NSWindow *makeWindow(NSString *title) {
  NSWindow *window = [[NSWindow alloc] initWithContentRect:NSMakeRect(0, 0, 240, 180)
      styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable
      backing:NSBackingStoreBuffered defer:YES];
  window.title = title;
  window.releasedWhenClosed = NO;
  return window;
}

static NSData *json(id value) {
  return [NSJSONSerialization dataWithJSONObject:value options:0 error:nil];
}

#define CHECK(condition, message) do { if (!(condition)) { \
  std::fprintf(stderr, "FAIL: %s\n", message); return 1; \
} } while (0)

int main() {
  @autoreleasepool {
    [NSApplication sharedApplication];
    if (NSApp.activationPolicy != NSApplicationActivationPolicyProhibited)
      [NSApp setActivationPolicy:NSApplicationActivationPolicyProhibited];
    CHECK(NSApp.activationPolicy == NSApplicationActivationPolicyProhibited,
          "fixture must not take foreground activation");
    kiwix_tabbing_install_early();
    NSWindow *main = makeWindow(@"Fixture Main");
    NSWindow *second = makeWindow(@"Fixture Second");
    NSWindow *unrelated = makeWindow(@"Fixture Unrelated");
    NSWindow *other = makeWindow(@"Fixture Other");
    NSWindow *panel = makeWindow(@"Fixture Panel");

    NSMutableString *label = [@"window-second" mutableCopy];
    kiwix_tabbing_prepare_window(main, @"main");
    kiwix_tabbing_prepare_window(second, label);
    kiwix_tabbing_prepare_window(unrelated, @"window-unrelated");
    kiwix_tabbing_prepare_window(other, @"window-other");
    kiwix_tabbing_prepare_window(panel, @"preferences");
    [label setString:@"window-replacement"];
    CHECK([windowLabel(second) isEqualToString:@"window-second"] && !windowLabel(panel),
          "the association must own the admitted label and exclude non-shell windows");

    [main addTabbedWindow:second ordered:NSWindowAbove];
    [unrelated addTabbedWindow:other ordered:NSWindowAbove];
    NSArray *original = [main.tabGroup.windows copy];
    NSArray *unrelatedGroup = [unrelated.tabGroup.windows copy];
    CHECK(original.count == 2 && unrelatedGroup.count == 2, "fixture tab groups");

    NSData *snapshot = nil;
    @autoreleasepool { snapshot = kiwix_tabbing_snapshot(); }
    NSArray *members = [NSJSONSerialization JSONObjectWithData:snapshot options:0 error:nil];
    CHECK(members.count == 4, "owned snapshot must survive pool retirement and exclude the panel");

    NSDictionary *valid = @{@"windows":@[@"window-second", @"main"],
                            @"selected":@"window-second", @"focused":@NO};
    NSDictionary *unknown = @{@"windows":@[@"window-closed"],
                              @"selected":@"window-closed", @"focused":@NO};
    NSDictionary *wrongType = @{@"windows":@[@"main"], @"selected":@"main", @"focused":@1};
    NSDictionary *focused = @{@"windows":@[@"window-second", @"main"],
                              @"selected":@"window-second", @"focused":@YES};
    NSDictionary *alsoFocused = @{@"windows":@[@"window-unrelated"],
                                  @"selected":@"window-unrelated", @"focused":@YES};
    for (NSArray *bad in @[@[valid, unknown], @[wrongType], @[@42], @[valid, valid],
                          @[focused, alsoFocused]]) {
      CHECK(!kiwix_tabbing_restore(json(bad)), "invalid, missing, or repeated groups must be refused");
      CHECK([main.tabGroup.windows isEqualToArray:original] &&
            [unrelated.tabGroup.windows isEqualToArray:unrelatedGroup],
            "restore must validate the whole request before regrouping");
    }
    CHECK(kiwix_tabbing_restore(json(@[valid])), "a live shell group should restore");
    CHECK([main.tabGroup.windows isEqualToArray:(@[second, main])] &&
          main.tabGroup.selectedWindow == second, "restore must set group order and selection");
    CHECK([unrelated.tabGroup.windows isEqualToArray:unrelatedGroup],
          "independent window group must survive restore");

    NSApp.mainMenu = [NSMenu new];
    kiwix_tabbing_install_menu();
    CHECK(kiwix_tabbing_update_menu(json(@{@"Window":@"Fenêtre",
         @"Show Next Tab":@"Onglet suivant — 日本語"})), "localized menu should update");
    NSMenuItem *menu = [NSApp.mainMenu itemWithTag:KiwixWindowMenuTag];
    CHECK([menu.title isEqualToString:@"Fenêtre"] &&
          [menu.submenu.itemArray[1].title isEqualToString:@"Onglet suivant — 日本語"],
          "localized labels must survive the bridge");
    CHECK(!kiwix_tabbing_update_menu(json(@{@"Window":@42})) &&
          [menu.title isEqualToString:@"Fenêtre"], "invalid text must leave the menu intact");

    for (NSWindow *window in @[main, second, unrelated, other, panel]) [window close];
    std::puts("PASS: copied label, owned snapshot, atomic restore validation, group order, selection, unrelated windows, and Unicode menu labels");
  }
}
