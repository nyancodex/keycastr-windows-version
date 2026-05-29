//! Virtual-key → display-character translation, the Windows equivalent of
//! macOS KeyCastr's `KCEventTransformer` keyboard-layout half.
//!
//! Two jobs:
//!   1. `modifiers_now()` — read live modifier state (Ctrl/Alt/Shift/Win) via
//!      `GetAsyncKeyState`, so a keystroke event carries the modifiers held at
//!      the instant it fired.
//!   2. `translate()` — turn a virtual-key code into the character(s) it would
//!      produce, using the *foreground window's* keyboard layout (so the
//!      visualizer follows the user's active layout, e.g. QWERTY vs AZERTY).
//!      Non-character keys (arrows, F-keys, Enter, …) resolve through
//!      `special_key()` instead.
//!
//! The composition of modifier glyphs + this base label into the final on-screen
//! string lives in the frontend (`src/transformer.js`), mirroring how KeyCastr
//! splits raw capture (Obj-C) from display formatting. To change *which* glyph a
//! special key shows, edit `special_key()`; to change modifier glyphs or the
//! command/modified/all-keys display rules, edit `src/transformer.js`.

use serde::Serialize;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyboardLayout, ToUnicodeEx, HKL,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

// Virtual-key codes we care about (Win32 VK_* values). Kept as raw literals to
// avoid depending on the exact constant names/types across `windows` versions.
const VK_BACK: u32 = 0x08;
const VK_TAB: u32 = 0x09;
const VK_RETURN: u32 = 0x0D;
const VK_SHIFT: u32 = 0x10;
const VK_CONTROL: u32 = 0x11;
const VK_MENU: u32 = 0x12; // Alt
const VK_PAUSE: u32 = 0x13;
const VK_CAPITAL: u32 = 0x14; // Caps Lock
const VK_ESCAPE: u32 = 0x1B;
const VK_SPACE: u32 = 0x20;
const VK_PRIOR: u32 = 0x21; // Page Up
const VK_NEXT: u32 = 0x22; // Page Down
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;
const VK_SNAPSHOT: u32 = 0x2C; // Print Screen
const VK_INSERT: u32 = 0x2D;
const VK_DELETE: u32 = 0x2E;
const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;
const VK_APPS: u32 = 0x5D; // Context-menu key
const VK_NUMLOCK: u32 = 0x90;
const VK_SCROLL: u32 = 0x91; // Scroll Lock

const VK_LSHIFT: u32 = 0xA0;
const VK_RSHIFT: u32 = 0xA1;
const VK_LCONTROL: u32 = 0xA2;
const VK_RCONTROL: u32 = 0xA3;
const VK_LMENU: u32 = 0xA4;
const VK_RMENU: u32 = 0xA5;

const VK_VOLUME_MUTE: u32 = 0xAD;
const VK_VOLUME_DOWN: u32 = 0xAE;
const VK_VOLUME_UP: u32 = 0xAF;
const VK_MEDIA_NEXT: u32 = 0xB0;
const VK_MEDIA_PREV: u32 = 0xB1;
const VK_MEDIA_STOP: u32 = 0xB2;
const VK_MEDIA_PLAY_PAUSE: u32 = 0xB3;

const KEYSTATE_DOWN: u8 = 0x80;
const KEYSTATE_TOGGLED: u8 = 0x01;

#[derive(Serialize, Clone, Copy, Default, Debug, PartialEq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

fn key_down(vk: u32) -> bool {
    // High-order bit of GetAsyncKeyState is set while the key is physically down.
    (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0
}

fn caps_on() -> bool {
    // Low-order bit reflects the *toggle* state for lock keys.
    (unsafe { GetAsyncKeyState(VK_CAPITAL as i32) } as u16 & 0x0001) != 0
}

/// Snapshot the modifiers currently held down.
pub fn modifiers_now() -> Modifiers {
    Modifiers {
        ctrl: key_down(VK_CONTROL),
        alt: key_down(VK_MENU),
        shift: key_down(VK_SHIFT),
        win: key_down(VK_LWIN) || key_down(VK_RWIN),
    }
}

/// True for keys that should surface as modifier-state changes ("flags" events
/// that light up the Svelte panel) rather than as printable keystrokes.
pub fn is_modifier_vk(vk: u32) -> bool {
    matches!(
        vk,
        VK_SHIFT
            | VK_CONTROL
            | VK_MENU
            | VK_LWIN
            | VK_RWIN
            | VK_LSHIFT
            | VK_RSHIFT
            | VK_LCONTROL
            | VK_RCONTROL
            | VK_LMENU
            | VK_RMENU
    )
}

/// Friendly label for non-character keys. `None` means "ask the layout"
/// (`to_unicode`). Returning a label here short-circuits character translation.
pub fn special_key(vk: u32) -> Option<&'static str> {
    let s = match vk {
        VK_BACK => "⌫",
        VK_TAB => "⇥",
        VK_RETURN => "⏎",
        VK_PAUSE => "Pause",
        VK_CAPITAL => "Caps",
        VK_ESCAPE => "Esc",
        VK_SPACE => "Space",
        VK_PRIOR => "PgUp",
        VK_NEXT => "PgDn",
        VK_END => "End",
        VK_HOME => "Home",
        VK_LEFT => "←",
        VK_UP => "↑",
        VK_RIGHT => "→",
        VK_DOWN => "↓",
        VK_SNAPSHOT => "PrtSc",
        VK_INSERT => "Ins",
        VK_DELETE => "Del",
        VK_APPS => "Menu",
        VK_NUMLOCK => "NumLk",
        VK_SCROLL => "ScrLk",
        VK_VOLUME_MUTE => "🔇",
        VK_VOLUME_DOWN => "🔉",
        VK_VOLUME_UP => "🔊",
        VK_MEDIA_NEXT => "⏭",
        VK_MEDIA_PREV => "⏮",
        VK_MEDIA_STOP => "⏹",
        VK_MEDIA_PLAY_PAUSE => "⏯",
        0x70..=0x87 => return Some(F_KEYS[(vk - 0x70) as usize]),
        _ => return None,
    };
    Some(s)
}

const F_KEYS: [&str; 24] = [
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14", "F15",
    "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
];

/// HKL of the foreground window's thread, so translation follows the layout the
/// user is actually typing into (not necessarily this process's layout).
fn foreground_layout() -> HKL {
    unsafe {
        let hwnd = GetForegroundWindow();
        let tid = GetWindowThreadProcessId(hwnd, None);
        GetKeyboardLayout(tid)
    }
}

/// Translate `vk`/`scan` to the character it produces under the given modifier
/// state, via `ToUnicodeEx`. Control characters and empty results return `None`.
fn to_unicode(vk: u32, scan: u32, shift: bool, ctrl: bool, alt: bool, caps: bool) -> Option<String> {
    let mut state = [0u8; 256];
    if shift {
        state[VK_SHIFT as usize] = KEYSTATE_DOWN;
    }
    if ctrl {
        state[VK_CONTROL as usize] = KEYSTATE_DOWN;
    }
    if alt {
        state[VK_MENU as usize] = KEYSTATE_DOWN;
    }
    if caps {
        state[VK_CAPITAL as usize] = KEYSTATE_TOGGLED;
    }

    let mut buf = [0u16; 8];
    // wFlags bit 2 (=4): do not alter the kernel keyboard state / dead-key
    // buffer (Win 10 1607+). Keeps repeated display translations from eating
    // the user's real dead-key sequences.
    let n = unsafe { ToUnicodeEx(vk, scan, &state, &mut buf, 4, foreground_layout()) };
    if n == 0 {
        return None;
    }
    let len = n.unsigned_abs() as usize; // n<0 => dead key (one char written)
    let s: String = String::from_utf16_lossy(&buf[..len.min(buf.len())]);
    let s = s.trim_end_matches('\0').to_string();
    if s.is_empty() || s.chars().all(|c| (c as u32) < 0x20) {
        return None;
    }
    Some(s)
}

/// What a key event resolves to for the frontend transformer.
#[derive(Serialize, Clone, Debug)]
pub struct KeyLabel {
    /// Friendly name for a non-character key (e.g. "⏎", "F5", "←"), else null.
    pub special: Option<&'static str>,
    /// Base character ignoring Shift/AltGr (e.g. "a", "1", "["), else null.
    pub plain: Option<String>,
    /// Character as actually typed with Shift/AltGr applied (e.g. "A", "!"),
    /// used by the "display modified characters" preference.
    pub typed: Option<String>,
}

/// Resolve a virtual key into a [`KeyLabel`] using the live keyboard layout.
pub fn translate(vk: u32, scan: u32, mods: Modifiers) -> KeyLabel {
    if let Some(special) = special_key(vk) {
        return KeyLabel {
            special: Some(special),
            plain: None,
            typed: None,
        };
    }

    let caps = caps_on();
    // AltGr is reported as Ctrl+Alt; only then do we let those modifiers reach
    // the typed translation, otherwise a bare Ctrl would yield a control char.
    let altgr = mods.ctrl && mods.alt;
    let plain = to_unicode(vk, scan, false, false, false, false);
    let typed = to_unicode(vk, scan, mods.shift, altgr, altgr, caps);

    KeyLabel {
        special: None,
        plain,
        typed,
    }
}
