// SPDX-License-Identifier: GPL-3.0-or-later
//! Match Cocoa's shifted key-equivalent conventions for installed app menus.
#![forbid(unsafe_code)]

use objc2_app_kit::{NSEventModifierFlags, NSMenu};
use objc2_foundation::NSString;

pub(super) fn normalize(menu: &NSMenu) {
    for item in menu.itemArray().iter() {
        if let Some(submenu) = item.submenu() {
            normalize(&submenu);
        }
        let modifiers = item.keyEquivalentModifierMask();
        if !modifiers.contains(NSEventModifierFlags::Shift) {
            continue;
        }
        let key = item.keyEquivalent();
        if key.length() != 1 {
            continue;
        }
        if let Some(character) =
            char::from_u32(u32::from(key.characterAtIndex(0))).and_then(shifted)
        {
            let mut utf8 = [0; 4];
            item.setKeyEquivalent(&NSString::from_str(character.encode_utf8(&mut utf8)));
            item.setKeyEquivalentModifierMask(modifiers - NSEventModifierFlags::Shift);
        }
    }
}

fn shifted(key: char) -> Option<char> {
    // Use the fixed US/ANSI equivalents emitted by the menu builder. Leave
    // keys without a mapping alone, including already shifted equivalents.
    Some(match key {
        'a'..='z' => key.to_ascii_uppercase(),
        ',' => '<',
        '.' => '>',
        '/' => '?',
        ';' => ':',
        '\'' => '"',
        '[' => '{',
        ']' => '}',
        '\\' => '|',
        '`' => '~',
        '-' => '_',
        '=' => '+',
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        _ => return None,
    })
}
