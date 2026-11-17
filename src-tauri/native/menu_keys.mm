// Cocoa matches a menu key equivalent by character plus modifier mask, and it
// treats Shift specially: the convention is "shifted character, no Shift
// bit". A lowercase equivalent combined with the Shift bit therefore also
// matches the unshifted key, so an earlier Cmd+Shift+R item swallows Cmd+R.
// muda builds equivalents that way, so normalise the installed menu once.
#import <AppKit/AppKit.h>

// The fixed US/ANSI pairing is used instead of guessing from the current
// keyboard layout: these characters have exactly one standard shifted form on
// every layout muda targets, and anything unlisted is left untouched rather
// than rewritten to a form Cocoa would not match.
static NSString *shiftedEquivalent(unichar character) {
  static const unichar unshifted[] = {
    ',', '.', '/', ';', '\'', '[', ']', '\\', '`', '-', '=',
    '1', '2', '3', '4', '5', '6', '7', '8', '9', '0'
  };
  static const unichar shifted[] = {
    '<', '>', '?', ':', '"', '{', '}', '|', '~', '_', '+',
    '!', '@', '#', '$', '%', '^', '&', '*', '(', ')'
  };
  if (character >= 'a' && character <= 'z') {
    NSString *letter = [NSString stringWithCharacters:&character length:1];
    NSString *uppercase = letter.uppercaseString;
    return uppercase.length == 1 ? uppercase : nil;
  }
  for (size_t index = 0; index < sizeof(unshifted) / sizeof(unshifted[0]); index++) {
    if (unshifted[index] == character) {
      return [NSString stringWithCharacters:&shifted[index] length:1];
    }
  }
  return nil;
}

static void normalizeMenu(NSMenu *menu) {
  for (NSMenuItem *item in menu.itemArray) {
    if (item.submenu) normalizeMenu(item.submenu);
    NSString *key = item.keyEquivalent;
    if (key.length != 1) continue;
    if ((item.keyEquivalentModifierMask & NSEventModifierFlagShift) == 0) continue;
    NSString *shifted = shiftedEquivalent([key characterAtIndex:0]);
    if (!shifted) continue;
    item.keyEquivalent = shifted;
    item.keyEquivalentModifierMask &= ~NSEventModifierFlagShift;
  }
}

/// Must run on the main thread after the application menu is installed.
extern "C" void kiwix_menu_normalize_shift_keys(void) {
  if (![NSThread isMainThread]) {
    dispatch_async(dispatch_get_main_queue(), ^{ kiwix_menu_normalize_shift_keys(); });
    return;
  }
  NSMenu *main = NSApp.mainMenu;
  if (main) normalizeMenu(main);
}
