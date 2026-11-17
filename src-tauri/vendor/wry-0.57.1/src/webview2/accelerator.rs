// Kiwix: which WebView2 key-downs are re-posted to the top-level window so the
// app menu's accelerators fire while a page has focus (see
// `InnerWebView::forward_accelerator_keys`). Plain integers only, so the app's
// test suite can include this file directly (src-tauri/src/menus.rs).

const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12;
const VK_HOME: u16 = 0x24;
const VK_LEFT: u16 = 0x25;
const VK_RIGHT: u16 = 0x27;
const VK_0: u16 = 0x30;
const VK_9: u16 = 0x39;
const VK_A: u16 = 0x41;
const VK_C: u16 = 0x43;
const VK_V: u16 = 0x56;
const VK_X: u16 = 0x58;
const VK_Y: u16 = 0x59;
const VK_Z: u16 = 0x5A;
const VK_LWIN: u16 = 0x5B;
const VK_RWIN: u16 = 0x5C;
const VK_F1: u16 = 0x70;
const VK_F10: u16 = 0x79;
const VK_F24: u16 = 0x87;
const VK_LSHIFT: u16 = 0xA0;
const VK_RSHIFT: u16 = 0xA1;
const VK_LCONTROL: u16 = 0xA2;
const VK_RCONTROL: u16 = 0xA3;
const VK_LMENU: u16 = 0xA4;
const VK_RMENU: u16 = 0xA5;

/// Whether a key-down typed into WebView2 should be forwarded to the
/// top-level window. `control`/`alt` are the live modifier states; `lparam`
/// is the original key message's lParam.
pub fn forward(vk: u16, control: bool, alt: bool, lparam: i32) -> bool {
  // Bit 30: the key was already down (auto-repeat). Holding Ctrl+W must
  // close one tab, not every tab in turn.
  if (lparam >> 30) & 1 == 1 {
    return false;
  }
  if [
    VK_CONTROL, VK_LCONTROL, VK_RCONTROL, VK_MENU, VK_LMENU, VK_RMENU, VK_SHIFT, VK_LSHIFT,
    VK_RSHIFT, VK_LWIN, VK_RWIN,
  ]
  .contains(&vk)
  {
    return false;
  }
  match (control, alt) {
    // Ctrl+Alt is AltGr text input on many layouts; no shortcut uses it.
    (true, true) => false,
    // The browser edits text itself, and muda runs its predefined Edit items
    // by synthesizing these same keys: forwarding them would loop.
    (true, false) => ![VK_A, VK_C, VK_V, VK_X, VK_Y, VK_Z].contains(&vk),
    // The app's Alt shortcuts: tabs (Alt+0…9), Home, Back and Forward. Any
    // other Alt key would reach DefWindowProc (Alt+F4 closing twice,
    // Alt+Space, menu mnemonics stealing focus from the page).
    (false, true) => (VK_0..=VK_9).contains(&vk) || [VK_HOME, VK_LEFT, VK_RIGHT].contains(&vk),
    // Function keys (F12 opens Settings); F10 would activate the menu bar.
    (false, false) => (VK_F1..=VK_F24).contains(&vk) && vk != VK_F10,
  }
}
