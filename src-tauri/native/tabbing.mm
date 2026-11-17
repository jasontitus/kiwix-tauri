// Native window tabs group whole shells, independently of reader tabs.
// Cmd+T / Ctrl+Tab remain reader actions; Cmd+Shift+[ / ] switch windows.
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <cstdlib>
#include <cstring>

static NSString *const KiwixTabbingIdentifier = @"org.kiwix.workbench.reader";
static const NSInteger KiwixWindowMenuTag = 0x4B57;
static char KiwixWindowLabelKey;
static NSDictionary<NSString *, NSString *> *windowLabels;

static void onMain(dispatch_block_t operation) {
  if ([NSThread isMainThread]) operation();
  else dispatch_sync(dispatch_get_main_queue(), operation);
}

static NSString *windowLabel(NSWindow *window) {
  return objc_getAssociatedObject(window, &KiwixWindowLabelKey);
}

static NSString *text(NSString *source) {
  return windowLabels[source] ?: source;
}

static void prepareWindow(NSWindow *window) {
  if (!window) return;
  if (window.tabbingIdentifier.length == 0) window.tabbingIdentifier = KiwixTabbingIdentifier;
  window.tabbingMode = NSWindowTabbingModePreferred;
}

extern "C" void kiwix_tabbing_install_early(void) {
  onMain(^{ [NSWindow setAllowsAutomaticWindowTabbing:YES]; });
}

extern "C" void kiwix_tabbing_prepare_window(void *raw, const char *label) {
  if (!raw || !label) return;
  NSWindow *window = (__bridge NSWindow *)raw;
  NSString *name = [NSString stringWithUTF8String:label];
  if (!name || !([name isEqualToString:@"main"] || [name hasPrefix:@"window-"])) return;
  onMain(^{
    objc_setAssociatedObject(window, &KiwixWindowLabelKey, name, OBJC_ASSOCIATION_COPY_NONATOMIC);
    prepareWindow(window);
  });
}

// Capture all groups in one main-thread observation, including singleton windows
// and library-only shells. AppKit's tab order is unrelated to NSApp.windows order.
extern "C" char *kiwix_tabbing_snapshot(void) {
  __block char *snapshot = nullptr;
  onMain(^{
    NSMutableArray *members = [NSMutableArray array];
    NSMutableSet<NSWindow *> *seen = [NSMutableSet set];
    for (NSWindow *window in NSApp.windows) {
      if (!windowLabel(window) || [seen containsObject:window]) continue;
      NSArray<NSWindow *> *tabs = window.tabGroup.windows;
      NSMutableArray<NSWindow *> *owned = [NSMutableArray array];
      for (NSWindow *tab in tabs) if (windowLabel(tab)) [owned addObject:tab];
      if (owned.count == 0) continue;
      NSString *group = windowLabel(owned.firstObject);
      NSWindow *selected = window.tabGroup.selectedWindow ?: window;
      BOOL focused = [owned containsObject:NSApp.keyWindow];
      for (NSUInteger index = 0; index < owned.count; index++) {
        NSWindow *tab = owned[index];
        [seen addObject:tab];
        [members addObject:@{@"label":windowLabel(tab), @"group":group, @"index":@(index),
                             @"selected":@(tab == selected), @"focused":@(focused)}];
      }
    }
    NSData *data = [NSJSONSerialization dataWithJSONObject:members options:0 error:nil];
    if (data) {
      snapshot = static_cast<char *>(malloc(data.length + 1));
      if (snapshot) {
        memcpy(snapshot, data.bytes, data.length);
        snapshot[data.length] = '\0';
      }
    }
  });
  return snapshot;
}

extern "C" void kiwix_tabbing_free_snapshot(char *snapshot) { free(snapshot); }

extern "C" bool kiwix_tabbing_restore(const char *json) {
  if (!json) return false;
  NSData *data = [[NSString stringWithUTF8String:json] dataUsingEncoding:NSUTF8StringEncoding];
  NSArray *groups = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
  if (![groups isKindOfClass:[NSArray class]]) return false;
  __block BOOL success = YES;
  onMain(^{
    NSMutableDictionary<NSString *, NSWindow *> *windows = [NSMutableDictionary dictionary];
    for (NSWindow *window in NSApp.windows) {
      NSString *label = windowLabel(window);
      if (label) windows[label] = window;
    }
    // Validate before changing anything; the Rust plan has already discarded
    // failed creations and excluded every unrelated live window.
    NSMutableSet *restored = [NSMutableSet set];
    for (NSDictionary *group in groups) {
      NSArray *labels = group[@"windows"];
      if (![labels isKindOfClass:[NSArray class]] || labels.count == 0 ||
          ![labels containsObject:group[@"selected"]]) { success = NO; return; }
      for (NSString *label in labels) {
        if (!windows[label] || [restored containsObject:label]) { success = NO; return; }
        [restored addObject:label];
      }
    }
    // A default preferred-tabbing launch may have merged all restored windows.
    // Detach the complete batch before rebuilding its exact saved groups.
    for (NSString *label in restored) {
      NSWindow *window = windows[label];
      [window.tabGroup removeWindow:window];
    }
    NSWindow *focused = nil;
    for (NSDictionary *group in groups) {
      NSWindow *anchor = windows[[group[@"windows"] firstObject]];
      NSUInteger index = 0;
      for (NSString *label in group[@"windows"]) {
        NSWindow *window = windows[label];
        if (index > 0) [anchor.tabGroup insertWindow:window atIndex:index];
        index++;
      }
      NSWindow *selected = windows[group[@"selected"]];
      selected.tabGroup.selectedWindow = selected;
      if ([group[@"focused"] boolValue]) focused = selected;
    }
    if (focused) [focused makeKeyAndOrderFront:nil];
  });
  return success;
}

// AppKit normally rewrites the toggle's title during validation. Owning that
// single validation path keeps Show/Hide Tab Bar in the chosen app locale.
@interface KiwixTabBarAction : NSObject <NSMenuItemValidation>
@end
@implementation KiwixTabBarAction
- (void)toggleTabBar:(id)sender {
  [NSApp sendAction:@selector(toggleTabBar:) to:nil from:sender];
}
- (BOOL)validateMenuItem:(NSMenuItem *)item {
  id target = [NSApp targetForAction:@selector(toggleTabBar:) to:nil from:item];
  NSWindow *window = NSApp.keyWindow;
  item.title = text(window.tabGroup.tabBarVisible ? @"Hide Tab Bar" : @"Show Tab Bar");
  return target != nil;
}
@end

static NSMenuItem *tabItem(NSString *source, SEL action, NSString *key,
                           NSEventModifierFlags modifiers) {
  NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:text(source)
                                              action:action keyEquivalent:key ?: @""];
  if (key) item.keyEquivalentModifierMask = modifiers;
  item.target = nil;
  item.representedObject = source;
  return item;
}

extern "C" void kiwix_tabbing_install_menu(void) {
  onMain(^{
    NSMenu *main = NSApp.mainMenu;
    if (!main) return;
    for (NSMenuItem *existing in [main.itemArray copy]) {
      if (existing.tag == KiwixWindowMenuTag) [main removeItem:existing];
    }
    NSMenuItem *windowItem = [[NSMenuItem alloc] initWithTitle:text(@"Window") action:nil keyEquivalent:@""];
    windowItem.tag = KiwixWindowMenuTag;
    NSMenu *menu = [[NSMenu alloc] initWithTitle:text(@"Window")];
    [menu addItem:tabItem(@"Show Previous Tab", @selector(selectPreviousTab:), @"{", NSEventModifierFlagCommand)];
    [menu addItem:tabItem(@"Show Next Tab", @selector(selectNextTab:), @"}", NSEventModifierFlagCommand)];
    [menu addItem:[NSMenuItem separatorItem]];
    [menu addItem:tabItem(@"Merge All Windows", @selector(mergeAllWindows:), nil, 0)];
    [menu addItem:tabItem(@"Move Tab to New Window", @selector(moveTabToNewWindow:), nil, 0)];
    NSMenuItem *toggle = tabItem(@"Show Tab Bar", @selector(toggleTabBar:), nil, 0);
    static KiwixTabBarAction *toggleAction;
    if (!toggleAction) toggleAction = [KiwixTabBarAction new];
    toggle.target = toggleAction;
    [menu addItem:toggle];
    [menu addItem:[NSMenuItem separatorItem]];
    [menu addItem:tabItem(@"Minimize", @selector(performMiniaturize:), @"m", NSEventModifierFlagCommand)];
    [menu addItem:tabItem(@"Zoom", @selector(performZoom:), nil, 0)];
    [menu addItem:[NSMenuItem separatorItem]];
    [menu addItem:tabItem(@"Bring All to Front", @selector(arrangeInFront:), nil, 0)];
    windowItem.submenu = menu;
    [main addItem:windowItem];
  });
}

extern "C" void kiwix_tabbing_update_menu(const char *json) {
  if (!json) return;
  NSData *data = [[NSString stringWithUTF8String:json] dataUsingEncoding:NSUTF8StringEncoding];
  NSDictionary *labels = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
  if (![labels isKindOfClass:[NSDictionary class]]) return;
  onMain(^{
    windowLabels = [labels copy];
    for (NSMenuItem *item in NSApp.mainMenu.itemArray) {
      if (item.tag != KiwixWindowMenuTag) continue;
      item.title = text(@"Window");
      item.submenu.title = item.title;
      for (NSMenuItem *child in item.submenu.itemArray) {
        if ([child.representedObject isKindOfClass:[NSString class]]) {
          child.title = text(child.representedObject);
        }
      }
    }
  });
}
