package com.remotedesktop.mobile

import android.view.KeyEvent

/** Android klavish/belgilarini Linux evdev kodlariga o'giradi (US raskladka; `crates/viewer/src/gui/keys.rs` bilan bir xil kodlar). */
object KeyMap {
    const val ESC = 1
    const val TAB = 15
    const val ENTER = 28
    const val BACKSPACE = 14
    const val CTRL = 29
    const val SHIFT = 42
    const val ALT = 56
    const val WIN = 125
    const val DELETE = 111
    const val UP = 103
    const val LEFT = 105
    const val RIGHT = 106
    const val DOWN = 108

    private val letterCodes = HashMap<Char, Int>().apply {
        "qwertyuiop".forEachIndexed { i, c -> put(c, 16 + i) }
        "asdfghjkl".forEachIndexed { i, c -> put(c, 30 + i) }
        "zxcvbnm".forEachIndexed { i, c -> put(c, 44 + i) }
    }

    private val plain = mapOf(
        '-' to 12, '=' to 13, '[' to 26, ']' to 27, ';' to 39, '\'' to 40, '`' to 41, '\\' to 43,
        ',' to 51, '.' to 52, '/' to 53, ' ' to 57, '\n' to ENTER, '\t' to TAB,
    )
    private val shifted = mapOf(
        '_' to 12, '+' to 13, '{' to 26, '}' to 27, ':' to 39, '"' to 40, '~' to 41, '|' to 43,
        '<' to 51, '>' to 52, '?' to 53,
        '!' to 2, '@' to 3, '#' to 4, '$' to 5, '%' to 6, '^' to 7, '&' to 8, '*' to 9, '(' to 10, ')' to 11,
    )

    /** Belgi -> (evdev kodi, Shift kerakmi). Lotin/raqam/ASCII belgilari; boshqalari (masalan, kirill) null. */
    fun forChar(c: Char): Pair<Int, Boolean>? {
        letterCodes[c]?.let { return it to false }
        if (c.isUpperCase()) letterCodes[c.lowercaseChar()]?.let { return it to true }
        if (c in '1'..'9') return (c - '1' + 2) to false
        if (c == '0') return 11 to false
        plain[c]?.let { return it to false }
        shifted[c]?.let { return it to true }
        return null
    }

    /** Android `KeyEvent.KEYCODE_*` -> evdev (belgisiz tugmalar). */
    fun forAndroid(code: Int): Int? = when (code) {
        KeyEvent.KEYCODE_ESCAPE -> ESC
        KeyEvent.KEYCODE_TAB -> TAB
        KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> ENTER
        KeyEvent.KEYCODE_DEL -> BACKSPACE
        KeyEvent.KEYCODE_FORWARD_DEL -> DELETE
        KeyEvent.KEYCODE_SPACE -> 57
        KeyEvent.KEYCODE_DPAD_UP -> UP
        KeyEvent.KEYCODE_DPAD_DOWN -> DOWN
        KeyEvent.KEYCODE_DPAD_LEFT -> LEFT
        KeyEvent.KEYCODE_DPAD_RIGHT -> RIGHT
        KeyEvent.KEYCODE_MOVE_HOME -> 102
        KeyEvent.KEYCODE_MOVE_END -> 107
        KeyEvent.KEYCODE_PAGE_UP -> 104
        KeyEvent.KEYCODE_PAGE_DOWN -> 109
        KeyEvent.KEYCODE_INSERT -> 110
        KeyEvent.KEYCODE_CTRL_LEFT, KeyEvent.KEYCODE_CTRL_RIGHT -> CTRL
        KeyEvent.KEYCODE_SHIFT_LEFT, KeyEvent.KEYCODE_SHIFT_RIGHT -> SHIFT
        KeyEvent.KEYCODE_ALT_LEFT, KeyEvent.KEYCODE_ALT_RIGHT -> ALT
        KeyEvent.KEYCODE_META_LEFT, KeyEvent.KEYCODE_META_RIGHT -> WIN
        in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F10 -> 59 + (code - KeyEvent.KEYCODE_F1)
        KeyEvent.KEYCODE_F11 -> 87
        KeyEvent.KEYCODE_F12 -> 88
        in KeyEvent.KEYCODE_A..KeyEvent.KEYCODE_Z -> letterCodes['a' + (code - KeyEvent.KEYCODE_A)]
        in KeyEvent.KEYCODE_1..KeyEvent.KEYCODE_9 -> 2 + (code - KeyEvent.KEYCODE_1)
        KeyEvent.KEYCODE_0 -> 11
        KeyEvent.KEYCODE_MINUS -> 12
        KeyEvent.KEYCODE_EQUALS -> 13
        KeyEvent.KEYCODE_LEFT_BRACKET -> 26
        KeyEvent.KEYCODE_RIGHT_BRACKET -> 27
        KeyEvent.KEYCODE_SEMICOLON -> 39
        KeyEvent.KEYCODE_APOSTROPHE -> 40
        KeyEvent.KEYCODE_GRAVE -> 41
        KeyEvent.KEYCODE_BACKSLASH -> 43
        KeyEvent.KEYCODE_COMMA -> 51
        KeyEvent.KEYCODE_PERIOD -> 52
        KeyEvent.KEYCODE_SLASH -> 53
        else -> null
    }
}
