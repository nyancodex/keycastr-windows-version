//! Global low-level keyboard + mouse capture — the Windows equivalent of macOS
//! KeyCastr's `KCEventTap` (which uses `CGEventTapCreate`).
//!
//! Windows has no event-tap API; instead we install process-wide low-level hooks
//! (`WH_KEYBOARD_LL`, `WH_MOUSE_LL`) with `SetWindowsHookExW`. Two hard
//! constraints shape this module:
//!
//!   * **A low-level hook needs a message loop on the thread that installed it.**
//!     So `start()` spawns a dedicated thread that installs both hooks and pumps
//!     `GetMessageW` forever. Without the pump the hooks never fire.
//!   * **The hook callbacks must return fast** (Windows silently drops a hook
//!     whose callback exceeds `LowLevelHooksTimeout`, ~300 ms). So the callbacks
//!     do the absolute minimum — read the event struct and push a `RawEvent`
//!     onto an `mpsc` channel — and all real work (layout translation, modifier
//!     dedupe, Tauri event emission) happens on the receiving worker thread in
//!     `main.rs`.
//!
//! Mouse-move flood control: plain moves are ignored; moves are only forwarded
//! while a button is held (a drag), bounding the channel traffic. This matches
//! the mouse visualizer, which only needs to follow the cursor during a click.
//!
//! To change which events are captured, edit `keyboard_proc` / `mouse_proc`.
//! Capture is unconditional and listen-only; gating by the "casting" toggle
//! happens in `main.rs` (we never block real input).

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::OnceLock;
use std::thread;

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN,
    WM_MBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_XBUTTONDOWN,
    WM_XBUTTONUP,
};

const HC_ACTION_CODE: i32 = 0;

#[derive(Clone, Copy, Debug)]
pub enum MouseKind {
    LeftDown,
    LeftUp,
    RightDown,
    RightUp,
    MiddleDown,
    MiddleUp,
    XDown,
    XUp,
    Move,
}

/// A raw input event captured by a hook, sent to the worker for processing.
#[derive(Clone, Copy, Debug)]
pub enum RawEvent {
    Key { vk: u32, scan: u32, down: bool },
    Mouse { kind: MouseKind, x: i32, y: i32 },
}

static EVENT_TX: OnceLock<Sender<RawEvent>> = OnceLock::new();
// Number of mouse buttons currently held, so we only forward drag-moves.
static BUTTONS_DOWN: AtomicI32 = AtomicI32::new(0);

fn send(ev: RawEvent) {
    if let Some(tx) = EVENT_TX.get() {
        let _ = tx.send(ev);
    }
}

/// Install the keyboard + mouse hooks on a dedicated message-pump thread and
/// route every captured event to `tx`. Returns immediately; the hooks live for
/// the rest of the process.
pub fn start(tx: Sender<RawEvent>) {
    let _ = EVENT_TX.set(tx);
    thread::spawn(|| unsafe {
        // A valid module handle for the hook proc. LL hooks aren't injected into
        // other processes, but a non-null hMod is the documented, reliable form.
        // `hmod` takes the handle directly (the windows-crate `Param` form), not
        // wrapped in `Option`.
        let hinst: HINSTANCE = GetModuleHandleW(None)
            .map(|h| HINSTANCE(h.0))
            .unwrap_or_default();

        let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), hinst, 0);
        let ms = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hinst, 0);
        if kb.is_err() || ms.is_err() {
            return;
        }

        // Pump messages so the system can invoke our hook callbacks on this
        // thread. We never post WM_QUIT, so this runs for the app's lifetime.
        let mut msg = MSG::default();
        loop {
            let r = GetMessageW(&mut msg, None, 0, 0);
            if r.0 <= 0 {
                break;
            }
        }
    });
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION_CODE {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        send(RawEvent::Key {
            vk: kb.vkCode,
            scan: kb.scanCode,
            down,
        });
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION_CODE {
        let ms = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        let (x, y) = (ms.pt.x, ms.pt.y);
        let kind = match wparam.0 as u32 {
            WM_LBUTTONDOWN => {
                BUTTONS_DOWN.fetch_add(1, Ordering::Relaxed);
                Some(MouseKind::LeftDown)
            }
            WM_RBUTTONDOWN => {
                BUTTONS_DOWN.fetch_add(1, Ordering::Relaxed);
                Some(MouseKind::RightDown)
            }
            WM_MBUTTONDOWN => {
                BUTTONS_DOWN.fetch_add(1, Ordering::Relaxed);
                Some(MouseKind::MiddleDown)
            }
            WM_XBUTTONDOWN => {
                BUTTONS_DOWN.fetch_add(1, Ordering::Relaxed);
                Some(MouseKind::XDown)
            }
            WM_LBUTTONUP => {
                release_button();
                Some(MouseKind::LeftUp)
            }
            WM_RBUTTONUP => {
                release_button();
                Some(MouseKind::RightUp)
            }
            WM_MBUTTONUP => {
                release_button();
                Some(MouseKind::MiddleUp)
            }
            WM_XBUTTONUP => {
                release_button();
                Some(MouseKind::XUp)
            }
            WM_MOUSEMOVE if BUTTONS_DOWN.load(Ordering::Relaxed) > 0 => Some(MouseKind::Move),
            _ => None,
        };
        if let Some(kind) = kind {
            send(RawEvent::Mouse { kind, x, y });
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

fn release_button() {
    // Clamp at zero so a button-up we didn't see the matching down for (e.g.
    // press happened before the app started) can't drive the counter negative.
    let _ = BUTTONS_DOWN.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
        Some((n - 1).max(0))
    });
}
