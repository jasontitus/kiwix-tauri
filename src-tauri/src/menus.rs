// SPDX-License-Identifier: GPL-3.0-or-later
//! Application shortcuts originate here, never in archive postMessage traffic.
use std::collections::HashMap;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    Emitter, Manager,
};

#[cfg(target_os = "macos")]
#[path = "macos_menu_keys.rs"]
mod macos_menu_keys;

const ACTIONS: [(&str, &str, &str); 38] = [
    ("new-tab", "New tab", "CmdOrCtrl+T"),
    ("new-window", "New window", "CmdOrCtrl+Shift+N"),
    // Explicitly reopens the last saved session; startup restore stays a setting.
    ("restore-session", "Restore previous session", ""),
    ("import", "Import archives", "CmdOrCtrl+O"),
    // Alt+Home navigates the focused reader to its archive's main page.
    ("home", "Archive home", "Alt+Home"),
    ("settings", "Settings", "F12"),
    ("close-tab", "Close tab", "CmdOrCtrl+W"),
    ("reopen-tab", "Reopen closed tab", "CmdOrCtrl+Shift+T"),
    ("next-tab", "Next tab", "Ctrl+Tab"),
    ("previous-tab", "Previous tab", "Ctrl+Shift+Tab"),
    ("select-tab-1", "Select tab 1", "Alt+1"),
    ("select-tab-2", "Select tab 2", "Alt+2"),
    ("select-tab-3", "Select tab 3", "Alt+3"),
    ("select-tab-4", "Select tab 4", "Alt+4"),
    ("select-tab-5", "Select tab 5", "Alt+5"),
    ("select-tab-6", "Select tab 6", "Alt+6"),
    ("select-tab-7", "Select tab 7", "Alt+7"),
    ("select-tab-8", "Select tab 8", "Alt+8"),
    ("select-tab-9", "Select tab 9", "Alt+9"),
    ("select-tab-0", "Select tab 10", "Alt+0"),
    ("reader-search", "Search books", "CmdOrCtrl+K"),
    ("reader-find", "Find in article", "CmdOrCtrl+F"),
    ("reader-print", "Print / save as PDF", "CmdOrCtrl+P"),
    ("reader-save", "Save article as PDF / original media", "CmdOrCtrl+S"),
    ("reader-bookmark", "Bookmark and notes", "CmdOrCtrl+D"),
    ("reader-zoom-in", "Zoom in", "CmdOrCtrl+Plus"),
    ("reader-zoom-out", "Zoom out", "CmdOrCtrl+-"),
    ("reader-zoom-reset", "Reset zoom", "CmdOrCtrl+0"),
    ("reader-back", "Back", "Alt+Left"),
    ("reader-forward", "Forward", "Alt+Right"),
    ("reader-random", "Random article", "CmdOrCtrl+R"),
    ("reader-toc", "Table of contents", "CmdOrCtrl+M"),
    ("reading-list", "Reading list", "CmdOrCtrl+B"),
    ("reader-fullscreen", "Fullscreen", "F11"),
    ("reader-speech-slower", "Read slower", "CmdOrCtrl+Shift+,"),
    ("reader-speech-faster", "Read faster", "CmdOrCtrl+Shift+."),
    // Upstream Kiwix keeps both search actions in Edit.
    ("library-search", "Filter your library", "CmdOrCtrl+Shift+R"),
    ("search-scope", "Search options", "CmdOrCtrl+Shift+L"),
];

/// Where macOS conventions differ from the `ACTIONS` keys, which Windows and
/// Linux keep. Cmd+M is the system Minimize key (the Window menu carries it),
/// F11 shows the desktop, Settings lives in the application menu on Cmd+,,
/// and full screen is Control+Command+F.
const MACOS_ACCELERATORS: [(&str, &str); 3] = [
    ("settings", "CmdOrCtrl+,"),
    ("reader-toc", "Alt+CmdOrCtrl+T"),
    ("reader-fullscreen", "Ctrl+Cmd+F"),
];

/// The accelerator `action` uses on macOS (`macos`) or on Windows and Linux.
fn platform_accelerator(action: &str, accelerator: &'static str, macos: bool) -> &'static str {
    if !macos {
        return accelerator;
    }
    MACOS_ACCELERATORS
        .iter()
        .find(|(id, _)| *id == action)
        .map_or(accelerator, |(_, macos)| *macos)
}

pub struct NativeMenus {
    file: Submenu<tauri::Wry>,
    tabs: Submenu<tauri::Wry>,
    items: Vec<(&'static str, MenuItem<tauri::Wry>)>,
    edit: Submenu<tauri::Wry>,
    standard: Vec<PredefinedMenuItem<tauri::Wry>>,
    /// About, Services, Hide, Hide Others and Show All, in `MenuLabels` order.
    #[cfg(target_os = "macos")]
    application: [PredefinedMenuItem<tauri::Wry>; 5],
    #[cfg(target_os = "macos")]
    quit: PredefinedMenuItem<tauri::Wry>,
    #[cfg(not(target_os = "macos"))]
    quit: MenuItem<tauri::Wry>,
}

fn shortcut_event(action: &str) -> (&'static str, &str) {
    if action == "home" {
        ("kiwix-reader-shortcut", "home")
    } else if let Some(reader_action) = action.strip_prefix("reader-") {
        ("kiwix-reader-shortcut", reader_action)
    } else {
        ("kiwix-shortcut", action)
    }
}

pub fn install(app: &tauri::AppHandle) -> tauri::Result<()> {
    let file = Submenu::with_id(app, "kiwix-file", "File", true)?;
    let tabs = Submenu::with_id(app, "kiwix-tabs", "Tabs", true)?;
    let edit = Submenu::with_id(app, "kiwix-edit", "Edit", true)?;
    let standard = vec![
        PredefinedMenuItem::undo(app, Some("Undo"))?,
        PredefinedMenuItem::redo(app, Some("Redo"))?,
        PredefinedMenuItem::cut(app, Some("Cut"))?,
        PredefinedMenuItem::copy(app, Some("Copy"))?,
        PredefinedMenuItem::paste(app, Some("Paste"))?,
        PredefinedMenuItem::select_all(app, Some("Select all"))?,
    ];
    for item in &standard {
        edit.append(item)?;
    }
    let mut items = Vec::with_capacity(ACTIONS.len());
    for (action, label, accelerator) in ACTIONS {
        let accelerator = platform_accelerator(action, accelerator, cfg!(target_os = "macos"));
        let item = MenuItem::with_id(
            app,
            format!("kiwix-{action}"),
            label,
            true,
            (!accelerator.is_empty()).then_some(accelerator),
        )?;
        // Settings/import/session actions are file-scoped, the two search
        // commands belong with editing, and Home is an archive reader action.
        // On macOS Settings belongs to the application menu instead (below).
        let parent = if cfg!(target_os = "macos") && action == "settings" {
            None
        } else if matches!(
            action,
            "new-window" | "restore-session" | "import" | "settings"
        ) {
            Some(&file)
        } else if matches!(action, "library-search" | "search-scope") {
            Some(&edit)
        } else {
            Some(&tabs)
        };
        if let Some(parent) = parent {
            parent.append(&item)?;
        }
        items.push((action, item));
    }
    #[cfg(target_os = "macos")]
    let quit = PredefinedMenuItem::quit(app, Some("Quit"))?;
    #[cfg(not(target_os = "macos"))]
    let quit = MenuItem::with_id(app, "kiwix-quit", "Quit", true, Some("Ctrl+Q"))?;
    #[cfg(target_os = "macos")]
    let main_thread = objc2::MainThreadMarker::new()
        .ok_or_else(|| std::io::Error::other("Install the application menu on the main thread"))?;
    let application = Submenu::with_id(app, "kiwix-application", "Kiwix", true)?;
    // The standard macOS application menu: About; Settings; Services; Hide,
    // Hide Others, Show All; Quit. Hide and Hide Others carry their system keys.
    #[cfg(target_os = "macos")]
    let application_items = [
        PredefinedMenuItem::about(app, Some("About Kiwix"), None)?,
        PredefinedMenuItem::services(app, Some("Services"))?,
        PredefinedMenuItem::hide(app, Some("Hide Kiwix"))?,
        PredefinedMenuItem::hide_others(app, Some("Hide others"))?,
        PredefinedMenuItem::show_all(app, Some("Show all"))?,
    ];
    #[cfg(target_os = "macos")]
    {
        let [about, services, hide, hide_others, show_all] = &application_items;
        application.append(about)?;
        application.append(&PredefinedMenuItem::separator(app)?)?;
        if let Some((_, settings)) = items.iter().find(|(action, _)| *action == "settings") {
            application.append(settings)?;
        }
        application.append(&PredefinedMenuItem::separator(app)?)?;
        application.append(services)?;
        application.append(&PredefinedMenuItem::separator(app)?)?;
        application.append(hide)?;
        application.append(hide_others)?;
        application.append(show_all)?;
        application.append(&PredefinedMenuItem::separator(app)?)?;
    }
    application.append(&quit)?;
    let menu = Menu::with_items(app, &[&application, &file, &edit, &tabs])?;
    app.set_menu(menu)?;
    // muda writes "Shift + lowercase letter" key equivalents, which Cocoa also
    // matches without Shift, so Cmd+Shift+R swallowed Cmd+R (and Cmd+Shift+T
    // would swallow Cmd+T). Normalise to Cocoa's uppercase-without-Shift form.
    // AppKit's Window menu carries the native tab-group commands
    // (Show Previous/Next Tab, Merge All Windows, Show Tab Bar).
    #[cfg(target_os = "macos")]
    {
        if let Some(menu) = objc2_app_kit::NSApplication::sharedApplication(main_thread).mainMenu() {
            macos_menu_keys::normalize(&menu);
        }
        crate::windows::install_tabbing_menu();
        let locale = app.try_state::<std::sync::Arc<crate::service::Service>>()
            .map(|service| service.store.locale())
            .transpose()
            .map_err(std::io::Error::other)?
            .unwrap_or_else(|| "system".into());
        crate::windows::update_tabbing_menu(&locale).map_err(std::io::Error::other)?;
    }
    app.manage(NativeMenus {
        file,
        tabs,
        items,
        edit,
        standard,
        #[cfg(target_os = "macos")]
        application: application_items,
        quit,
    });
    app.on_menu_event(|app, event| {
        #[cfg(not(target_os = "macos"))]
        if event.id().as_ref() == "kiwix-quit" {
            // AppHandle::exit enters the existing ExitRequested retirement/flush gate.
            app.exit(0);
            return;
        }
        let Some(action) = event.id().as_ref().strip_prefix("kiwix-") else {
            return;
        };
        if !ACTIONS.iter().any(|(allowed, _, _)| *allowed == action) {
            return;
        }
        // Emit only to the focused shell, not every window or any archive frame.
        if let Some(window) = focused_shell(app) {
            let target = tauri::EventTarget::webview_window(window.label());
            let (event, payload) = shortcut_event(action);
            let _ = app.emit_to(target, event, payload);
        }
    });
    Ok(())
}

fn is_shell(label: &str) -> bool {
    label == "main" || label.starts_with("window-")
}

fn focused_shell(app: &tauri::AppHandle) -> Option<tauri::WebviewWindow> {
    let windows = app.webview_windows();
    if let Some((_, window)) = windows
        .iter()
        .find(|(label, window)| is_shell(label) && window.is_focused().unwrap_or(false))
    {
        return Some(window.clone());
    }
    // On Windows a single-webview window reports focus from its top-level HWND,
    // which loses keyboard focus to WebView2 whenever a page is focused, so
    // `is_focused` is false exactly when menu shortcuts and clicks arrive. The
    // foreground window is the one the user is acting in.
    #[cfg(windows)]
    {
        let foreground =
            // SAFETY: the call takes no arguments and returns a borrowed window
            // handle value, which is only compared by address and never dereferenced.
            unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() }.0 as isize;
        if foreground != 0 {
            return windows
                .into_iter()
                .find(|(label, window)| {
                    is_shell(label) && window.hwnd().is_ok_and(|hwnd| hwnd.0 as isize == foreground)
                })
                .map(|(_, window)| window);
        }
    }
    None
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MenuLabels {
    file: String,
    tabs: String,
    new_tab: String,
    new_window: String,
    restore_session: String,
    import: String,
    home: String,
    settings: String,
    reading_list: String,
    library_search: String,
    search_scope: String,
    close_tab: String,
    reopen_tab: String,
    next_tab: String,
    previous_tab: String,
    select_tabs: Vec<String>,
    reader: HashMap<String, String>,
    edit: String,
    undo: String,
    redo: String,
    cut: String,
    copy: String,
    paste: String,
    select_all: String,
    quit: String,
    /// The macOS application menu; accepted and validated everywhere.
    about: String,
    services: String,
    hide: String,
    hide_others: String,
    show_all: String,
}

/// Only text can change: IDs, actions and accelerators are fixed in Rust.
#[tauri::command]
pub fn update_native_menu(
    window: tauri::WebviewWindow,
    menus: tauri::State<'_, NativeMenus>,
    labels: MenuLabels,
) -> Result<(), String> {
    if window.label() != "main" && !window.label().starts_with("window-") {
        return Err("Not an application window".into());
    }
    let values = [
        &labels.file,
        &labels.tabs,
        &labels.new_tab,
        &labels.new_window,
        &labels.restore_session,
        &labels.import,
        &labels.home,
        &labels.settings,
        &labels.reading_list,
        &labels.library_search,
        &labels.search_scope,
        &labels.close_tab,
        &labels.reopen_tab,
        &labels.next_tab,
        &labels.previous_tab,
        &labels.edit,
        &labels.undo,
        &labels.redo,
        &labels.cut,
        &labels.copy,
        &labels.paste,
        &labels.select_all,
        &labels.quit,
        &labels.about,
        &labels.services,
        &labels.hide,
        &labels.hide_others,
        &labels.show_all,
    ];
    if values
        .iter()
        .any(|text| text.is_empty() || text.len() > 256 || text.chars().any(char::is_control))
    {
        return Err("Invalid menu label".into());
    }
    if labels.select_tabs.len() != 10
        || labels
            .select_tabs
            .iter()
            .any(|text| text.is_empty() || text.len() > 256 || text.chars().any(char::is_control))
    {
        return Err("Invalid tab selection labels".into());
    }
    if labels.reader.len()
        != ACTIONS
            .iter()
            .filter(|(action, _, _)| action.starts_with("reader-"))
            .count()
        || labels
            .reader
            .values()
            .any(|text| text.is_empty() || text.len() > 256 || text.chars().any(char::is_control))
    {
        return Err("Invalid reader menu labels".into());
    }
    for reader_action in ACTIONS
        .iter()
        .filter_map(|(action, _, _)| action.strip_prefix("reader-"))
    {
        if !labels.reader.contains_key(reader_action) {
            return Err("Missing reader menu label".into());
        }
    }
    #[cfg(target_os = "macos")]
    {
        let service = window.try_state::<std::sync::Arc<crate::service::Service>>()
            .ok_or("Library service unavailable during menu localization")?;
        crate::windows::update_tabbing_menu(&service.store.locale()?)?;
    }
    let escaped = |text: &str| text.replace('&', "&&");
    menus
        .file
        .set_text(escaped(&labels.file))
        .map_err(|e| e.to_string())?;
    menus
        .tabs
        .set_text(escaped(&labels.tabs))
        .map_err(|e| e.to_string())?;
    menus
        .edit
        .set_text(escaped(&labels.edit))
        .map_err(|e| e.to_string())?;
    menus
        .quit
        .set_text(escaped(&labels.quit))
        .map_err(|e| e.to_string())?;
    for (item, label) in menus.standard.iter().zip([
        labels.undo,
        labels.redo,
        labels.cut,
        labels.copy,
        labels.paste,
        labels.select_all,
    ]) {
        item.set_text(escaped(&label)).map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    for (item, label) in menus.application.iter().zip([
        &labels.about,
        &labels.services,
        &labels.hide,
        &labels.hide_others,
        &labels.show_all,
    ]) {
        item.set_text(escaped(label)).map_err(|e| e.to_string())?;
    }
    for (action, item) in &menus.items {
        let label = match *action {
            "new-tab" => &labels.new_tab,
            "new-window" => &labels.new_window,
            "restore-session" => &labels.restore_session,
            "import" => &labels.import,
            "home" => &labels.home,
            "settings" => &labels.settings,
            "reading-list" => &labels.reading_list,
            "library-search" => &labels.library_search,
            "search-scope" => &labels.search_scope,
            "close-tab" => &labels.close_tab,
            "reopen-tab" => &labels.reopen_tab,
            "next-tab" => &labels.next_tab,
            "previous-tab" => &labels.previous_tab,
            action => {
                if let Some(reader_action) = action.strip_prefix("reader-") {
                    &labels.reader[reader_action]
                } else {
                    // Only the fixed ACTIONS allowlist reaches here, never a caller-supplied ID.
                    let digit = action.as_bytes()[action.len() - 1] - b'0';
                    let index = if digit == 0 {
                        9
                    } else {
                        usize::from(digit - 1)
                    };
                    &labels.select_tabs[index]
                }
            }
        };
        item.set_text(escaped(label)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// The vendored wry's rules for forwarding WebView2 keys to this menu. They are
// plain integer logic, so the exact shipped file is tested here on every
// platform (wry's own test build needs dev-dependencies that are not vendored).
#[cfg(test)]
#[path = "../vendor/wry-0.57.1/src/webview2/accelerator.rs"]
mod wry_accelerator;

#[cfg(test)]
mod tests {
    use super::{is_shell, platform_accelerator, shortcut_event, wry_accelerator::forward, ACTIONS, MACOS_ACCELERATORS};
    use std::collections::{BTreeSet, HashMap};

    /// An accelerator as the key a person presses: modifiers spelled one way,
    /// CmdOrCtrl resolved for the platform, the key upper-cased.
    fn chord(accelerator: &str, macos: bool) -> (BTreeSet<&'static str>, String) {
        let mut modifiers = BTreeSet::new();
        let mut key = None;
        for part in accelerator.split('+').filter(|part| !part.is_empty()) {
            let modifier = match part.to_ascii_lowercase().as_str() {
                "cmdorctrl" | "commandorcontrol" => Some(if macos { "Cmd" } else { "Ctrl" }),
                "cmd" | "command" | "super" | "meta" => Some("Cmd"),
                "ctrl" | "control" => Some("Ctrl"),
                "alt" | "option" => Some("Alt"),
                "shift" => Some("Shift"),
                _ => None,
            };
            match modifier {
                Some(modifier) => assert!(modifiers.insert(modifier), "{accelerator} repeats {modifier}"),
                None => {
                    assert!(key.is_none(), "{accelerator} has two keys");
                    key = Some(part.to_ascii_uppercase());
                }
            }
        }
        (modifiers, key.unwrap_or_else(|| panic!("{accelerator} has no key")))
    }

    /// Keys macOS or its standard menus own: Minimize (the Window menu also
    /// carries it) and Minimize All, Hide and Hide Others, Quit, Settings
    /// (reserved here for the application menu's own item), window cycling,
    /// the Window menu's tab commands, Show Desktop, Spotlight, the character
    /// viewer, lock screen, log out, screenshots, Dock hiding, app switching
    /// and Force Quit.
    const MACOS_RESERVED: [&str; 20] = [
        "Cmd+M", "Cmd+Alt+M", "Cmd+H", "Cmd+Alt+H", "Cmd+Q", "Cmd+,", "Cmd+`",
        "Cmd+Shift+[", "Cmd+Shift+]", "F11", "Cmd+Space", "Ctrl+Cmd+Space",
        "Ctrl+Cmd+Q", "Cmd+Shift+Q", "Cmd+Shift+3", "Cmd+Shift+4", "Cmd+Shift+5",
        "Cmd+Alt+D", "Cmd+Tab", "Cmd+Alt+Escape",
    ];

    #[test]
    fn macos_accelerators_leave_system_keys_alone_and_none_repeat() {
        let reserved: BTreeSet<_> = MACOS_RESERVED.iter().map(|key| chord(key, true)).collect();
        for macos in [true, false] {
            // The predefined Edit items (and, on Windows and Linux, Quit) are taken too.
            let mut taken: HashMap<_, &str> = if macos {
                ["Cmd+Z", "Cmd+Shift+Z", "Cmd+X", "Cmd+C", "Cmd+V", "Cmd+A"].as_slice()
            } else {
                ["Ctrl+Z", "Ctrl+Y", "Ctrl+X", "Ctrl+C", "Ctrl+V", "Ctrl+A", "Ctrl+Q"].as_slice()
            }
            .iter()
            .map(|key| (chord(key, macos), *key))
            .collect();
            for (action, _, default) in ACTIONS {
                let accelerator = platform_accelerator(action, default, macos);
                if accelerator.is_empty() {
                    continue;
                }
                let pressed = chord(accelerator, macos);
                if macos && action != "settings" {
                    assert!(!reserved.contains(&pressed), "{action} ({accelerator}) is a macOS system key");
                }
                if let Some(other) = taken.insert(pressed, action) {
                    panic!("{action} ({accelerator}) repeats {other} (macOS: {macos})");
                }
            }
        }
        // Settings sits on the macOS convention, and the overrides name real actions.
        assert_eq!(chord(platform_accelerator("settings", "F12", true), true), chord("Cmd+,", true));
        assert_eq!(chord(platform_accelerator("reader-fullscreen", "F11", true), true), chord("Ctrl+Cmd+F", true));
        for (id, _) in MACOS_ACCELERATORS {
            assert!(ACTIONS.iter().any(|(action, _, _)| *action == id), "{id}");
        }
    }

    #[test]
    fn windows_and_linux_keep_their_documented_accelerators() {
        for (action, _, default) in ACTIONS {
            assert_eq!(platform_accelerator(action, default, false), default, "{action}");
        }
        assert_eq!(platform_accelerator("settings", "F12", false), "F12");
        assert_eq!(platform_accelerator("reader-toc", "CmdOrCtrl+M", false), "CmdOrCtrl+M");
        assert_eq!(platform_accelerator("reader-fullscreen", "F11", false), "F11");
    }

    const DOWN: i32 = 1; // lParam of a first key-down: repeat count 1, bit 30 clear.
    const REPEAT: i32 = 1 | (1 << 30);

    /// The Windows virtual key and modifiers of an ACTIONS accelerator.
    fn windows_key(accelerator: &str) -> (u16, bool, bool) {
        let (mut control, mut alt) = (false, false);
        let mut key = None;
        for part in accelerator.split('+').filter(|part| !part.is_empty()) {
            match part {
                "CmdOrCtrl" | "Ctrl" => control = true,
                "Alt" => alt = true,
                "Shift" => {}
                _ => key = Some(part),
            }
        }
        let key = key.unwrap_or_else(|| panic!("{accelerator} has no key"));
        let vk = match key {
            "Plus" | "+" => 0xBB,
            "-" => 0xBD,
            "," => 0xBC,
            "." => 0xBE,
            "Tab" => 0x09,
            "Home" => 0x24,
            "Left" => 0x25,
            "Right" => 0x27,
            function if function.starts_with('F') && function.len() > 1 => {
                0x70 + function[1..].parse::<u16>().unwrap() - 1
            }
            single if single.len() == 1 => single.as_bytes()[0].to_ascii_uppercase() as u16,
            other => panic!("map {other} in {accelerator} to a virtual key"),
        };
        (vk, control, alt)
    }

    // With Ctrl held every key but the six editing keys is forwarded, so for Ctrl
    // shortcuts this mainly proves none of them is an editing key; the Alt and
    // function-key shortcuts are checked against their exact allowlists.
    #[test]
    fn every_menu_accelerator_reaches_the_menu_from_a_focused_page() {
        let mut checked = 0;
        for (action, _, accelerator) in ACTIONS {
            if accelerator.is_empty() {
                continue;
            }
            let (vk, control, alt) = windows_key(accelerator);
            assert!(
                forward(vk, control, alt, DOWN),
                "{action} ({accelerator}) would be dead on Windows while a page has focus"
            );
            assert!(!forward(vk, control, alt, REPEAT), "{action}: auto-repeat must not fire again");
            checked += 1;
        }
        assert!(checked >= 30, "only {checked} accelerators were checked");
    }

    #[test]
    fn page_keys_that_are_not_menu_shortcuts_stay_with_the_page() {
        // Editing keys: the browser handles them, and muda's predefined Edit
        // items re-synthesize them, so forwarding would loop.
        for key in b"ACVXYZ" {
            assert!(!forward(u16::from(*key), true, false, DOWN), "Ctrl+{}", *key as char);
        }
        // AltGr (Ctrl+Alt) is text input on many layouts.
        assert!(!forward(u16::from(b'Q'), true, true, DOWN));
        assert!(!forward(u16::from(b'2'), true, true, DOWN));
        // Alt keys outside the menu would reach DefWindowProc.
        assert!(!forward(0x73, false, true, DOWN), "Alt+F4 must not close the window twice");
        assert!(!forward(0x20, false, true, DOWN), "Alt+Space");
        assert!(!forward(u16::from(b'F'), false, true, DOWN), "Alt+letter menu mnemonics");
        // Plain keys, F10 and bare modifiers.
        assert!(!forward(u16::from(b'T'), false, false, DOWN));
        assert!(!forward(0x79, false, false, DOWN), "F10 activates the menu bar");
        for modifier in [0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5] {
            assert!(!forward(modifier, true, false, DOWN), "bare modifier {modifier:#x}");
        }
        assert!(forward(0x7B, false, false, DOWN), "F12 opens Settings");
    }

    #[test]
    fn only_reader_shells_receive_menu_actions() {
        assert!(is_shell("main"));
        assert!(is_shell("window-3"));
        assert!(!is_shell("camera-capture"));
        assert!(!is_shell("pdf-1"));
        assert!(!is_shell(""));
    }

    #[test]
    fn archive_home_uses_the_focused_reader_instead_of_leaving_for_the_library() {
        assert_eq!(shortcut_event("home"), ("kiwix-reader-shortcut", "home"));
        assert_eq!(shortcut_event("reader-back"), ("kiwix-reader-shortcut", "back"));
        assert_eq!(shortcut_event("settings"), ("kiwix-shortcut", "settings"));
    }
}
