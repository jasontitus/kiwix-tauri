package org.kiwix.mobile

import android.util.SparseBooleanArray
import android.view.InputDevice
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.View

/** Handles Android hardware dispatch, not DOM events or messages from archive content. */
internal class HardwareShortcutKeys(
    private val enabled: () -> Boolean,
    private val dispatch: (String) -> Unit
) : View.OnKeyListener {
    private val consumed = SparseBooleanArray()
    private val tabs = arrayOf("select-tab-0", "select-tab-1", "select-tab-2", "select-tab-3",
        "select-tab-4", "select-tab-5", "select-tab-6", "select-tab-7", "select-tab-8", "select-tab-9")

    fun reset() = consumed.clear()

    override fun onKey(view: View, keyCode: Int, event: KeyEvent): Boolean {
        if (!enabled() || event.deviceId == KeyCharacterMap.VIRTUAL_KEYBOARD ||
            event.device?.isVirtual != false || event.flags and KeyEvent.FLAG_SOFT_KEYBOARD != 0 ||
            event.source and InputDevice.SOURCE_KEYBOARD != InputDevice.SOURCE_KEYBOARD) return false
        if (event.action == KeyEvent.ACTION_UP) {
            if (!consumed.get(keyCode)) return false
            consumed.delete(keyCode)
            return true
        }
        if (event.action != KeyEvent.ACTION_DOWN) return false
        // Consume held-key repeats without creating repeated tabs, prompts, or exports.
        if (consumed.get(keyCode)) return true
        val action = action(keyCode, event) ?: return false
        consumed.put(keyCode, true)
        if (event.repeatCount == 0 && !event.isCanceled) dispatch(action)
        return true
    }

    private fun action(key: Int, event: KeyEvent): String? {
        val ctrl = event.isCtrlPressed
        val meta = event.isMetaPressed
        val alt = event.isAltPressed
        val shift = event.isShiftPressed
        if (alt && !ctrl && !meta && !shift && key in KeyEvent.KEYCODE_0..KeyEvent.KEYCODE_9)
            return tabs[key - KeyEvent.KEYCODE_0]
        if (ctrl && !alt && !meta && key == KeyEvent.KEYCODE_TAB)
            return if (shift) "previous-tab" else "next-tab"
        if ((ctrl || meta) && !alt) return when (key) {
            KeyEvent.KEYCODE_O -> "import"
            KeyEvent.KEYCODE_T -> if (shift) "reopen-tab" else "new-tab"
            KeyEvent.KEYCODE_W -> "close-tab"
            KeyEvent.KEYCODE_K -> "search"
            KeyEvent.KEYCODE_F -> "find"
            KeyEvent.KEYCODE_P -> "print"
            KeyEvent.KEYCODE_S -> "save"
            KeyEvent.KEYCODE_D -> "bookmark"
            KeyEvent.KEYCODE_PLUS, KeyEvent.KEYCODE_EQUALS, KeyEvent.KEYCODE_NUMPAD_ADD -> "zoom-in"
            KeyEvent.KEYCODE_MINUS, KeyEvent.KEYCODE_NUMPAD_SUBTRACT -> "zoom-out"
            KeyEvent.KEYCODE_0, KeyEvent.KEYCODE_NUMPAD_0 -> "zoom-reset"
            else -> null
        }
        if (alt && !ctrl && !meta && !shift) return when (key) {
            KeyEvent.KEYCODE_DPAD_LEFT -> "back"
            KeyEvent.KEYCODE_DPAD_RIGHT -> "forward"
            else -> null
        }
        if (!ctrl && !meta && !alt && !shift) return when (key) {
            KeyEvent.KEYCODE_F11 -> "fullscreen"
            KeyEvent.KEYCODE_ESCAPE -> "escape"
            else -> null
        }
        return null
    }
}
