//! KeyCastr for Windows — application shell.
//!
//! This is the Windows equivalent of macOS KeyCastr's `KCAppController`: it owns
//! the tray icon, the casting on/off toggle, the always-on-top click-through
//! overlay window that the visualizers draw into, and the persisted preferences.
//!
//! Capture pipeline (mirrors KeyCastr's tap → transformer → visualizer split):
//!
//!   `hook.rs`  installs the low-level keyboard/mouse hooks and pushes every
//!              raw event onto an `mpsc` channel (must return fast — see hook.rs).
//!   `run_worker` (this file) drains that channel on a normal thread: it reads
//!              live modifier state, runs the toggle-hotkey check, gates on the
//!              casting flag, translates keys via `translate.rs`, and emits a
//!              single `kc-event` to the overlay webview for each event.
//!   the frontend (`src/overlay.*`, `src/transformer.js`) turns those events
//!              into the on-screen Default / Svelte / mouse visualizations.
//!
//! Why a worker thread and not work in the hook callback: Windows silently drops
//! a low-level hook whose callback is slow (~300 ms `LowLevelHooksTimeout`), so
//! all translation/emission happens off the hook thread.
//!
//! Toggle hotkey: KeyCastr swallows its toggle chord inside the event tap so it
//! never shows. Our LL hooks are listen-only (we cannot eat an OS keystroke), so
//! instead we swallow it at the *display* layer — `run_worker` recognizes the
//! chord and flips casting without emitting anything for it.
//!
//! Divergence from macOS KeyCastr: KeyCastr's bezel is a draggable window the
//! user repositions. Our overlay is a single fullscreen click-through window, so
//! position is chosen from presets in Preferences (`position`) instead of drag.
//!
//! To change captured events, edit `hook.rs`; to change key→glyph translation,
//! edit `translate.rs`; to change display formatting/visualizers, edit the
//! frontend in `../src`. To add a preference, extend `Settings` here AND the
//! Preferences UI (`../src/prefs.*`) — they share the camelCase JSON shape.

// Prevent a console window from opening alongside the app in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hook;
mod translate;

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, Wry,
};

use hook::{MouseKind, RawEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_updater::UpdaterExt;

// --- Constants ----------------------------------------------------------------

const OVERLAY_LABEL: &str = "overlay";
const PREFS_LABEL: &str = "prefs";
const TRAY_ID: &str = "kc-tray";

// Events emitted to the webview(s). `kc-event` carries every keystroke/flags/
// mouse event to the overlay; `kc-casting` broadcasts the on/off state; and
// `kc-settings` pushes updated preferences to the overlay live.
const KC_EVENT: &str = "kc-event";
const CASTING_EVENT: &str = "kc-casting";
const SETTINGS_EVENT: &str = "kc-settings";

// --- Globals ------------------------------------------------------------------

// Whether we are currently casting (displaying events). Read on every event in
// the worker; toggled by the hotkey, tray item, and commands.
static CASTING: AtomicBool = AtomicBool::new(false);
// Set by `apply_casting` on every on/off transition; the worker consumes it to
// drop its modifier-dedupe cache so the next modifier change is always
// re-emitted (otherwise a modifier held across a stop→start would stay
// suppressed and never re-light the panel).
static FLAGS_RESET: AtomicBool = AtomicBool::new(false);
// The toggle chord, behind a Mutex so the Preferences UI can rebind it live.
static TOGGLE: OnceLock<Mutex<HotKey>> = OnceLock::new();
// The tray menu item whose label flips between "Start"/"Stop Casting".
static TOGGLE_ITEM: OnceLock<MenuItem<Wry>> = OnceLock::new();

// --- Debug logging ------------------------------------------------------------
//
// KeyCastr runs as a windowless tray app, so stderr is invisible once installed.
// We append timestamped lines to a log file in the OS app-log dir
// (`%LOCALAPPDATA%\com.keycastr.windows\logs\keycastr.log`) so a tester can send
// it back. Best-effort; never blocks the app. To change it, edit init/log_line.

static LOG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

fn init_logging(app: &AppHandle) {
    let path = app.path().app_log_dir().ok().map(|dir| {
        let _ = std::fs::create_dir_all(&dir);
        dir.join("keycastr.log")
    });
    let _ = LOG_PATH.set(path);
    log_line(&format!(
        "=== KeyCastr {} starting (os={}, arch={}) ===",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    ));
}

fn log_line(msg: &str) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("[{ts}] {msg}\n");
    if cfg!(debug_assertions) {
        eprint!("{line}");
    }
    if let Some(Some(path)) = LOG_PATH.get() {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

// --- Settings -----------------------------------------------------------------

/// The toggle hotkey, captured as discrete modifier flags + a Win32 virtual-key
/// code (so the worker can compare it against `modifiers_now()` + the event vk
/// without parsing an accelerator string). `label` is the human form shown in
/// the UI. Default mirrors KeyCastr's Ctrl+Opt+Cmd+K, mapped to Windows keys.
#[derive(Serialize, Deserialize, Clone)]
// `default` so a settings.json written by an older/newer build that is missing
// a field still loads (the field falls back to its default) instead of failing
// to parse — see the note on `Settings`.
#[serde(rename_all = "camelCase", default)]
struct HotKey {
    ctrl: bool,
    alt: bool,
    shift: bool,
    win: bool,
    vk: u32,
    label: String,
}

impl Default for HotKey {
    fn default() -> Self {
        Self {
            ctrl: true,
            alt: true,
            shift: true,
            win: false,
            vk: 0x4B, // 'K'
            label: "Ctrl+Alt+Shift+K".into(),
        }
    }
}

/// User-configurable preferences, persisted as camelCase JSON in the app config
/// dir. Defaults mirror macOS KeyCastr's out-of-box behavior (command-keys-only
/// Default visualizer, 2s fade delay, 16pt font, black α0.8 bezel, etc.).
///
/// Field map to KeyCastr concepts:
///   visualizer            "Default" | "Svelte" — which keystroke visualizer
///   display_mode          "command" | "modified" | "all" — KCDefaultVisualizer's
///                         command-only / all-modified / all-keys modes
///   svelte_display_all    Svelte: show every key, not just modified ones
///   display_modified_characters  show the *typed* char (Shift/AltGr applied)
///   font_size/colors/fade_* /keystroke_delay  — bezel appearance & timing
///   mouse_display         "none" | "current" — mouse-click circle visualizer
///   mouse_text            also show each click as a text label ("Left Click")
///                         in the active keystroke visualizer; independent of
///                         mouse_display (you can have circles, text, both, none)
///   position              overlay corner (our substitute for KeyCastr's drag)
///   start_casting_at_launch  begin capturing immediately on launch
///   toggle_hotkey         the casting on/off chord
///
/// `#[serde(default)]` (container level) is load-bearing for upgrades: a
/// settings.json written by a build that didn't have one of these fields (e.g.
/// `mouse_text`, added in 0.1.2) must still deserialize — the missing field
/// falls back to its default rather than failing the whole parse. Without it,
/// `load_settings` would treat an old file as corrupt and silently reset ALL of
/// the user's preferences to defaults.
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    visualizer: String,
    display_mode: String,
    svelte_display_all: bool,
    display_modified_characters: bool,
    font_size: f64,
    bezel_color: String,
    text_color: String,
    fade_delay: f64,
    fade_duration: f64,
    keystroke_delay: f64,
    mouse_display: String,
    mouse_text: bool,
    position: String,
    start_casting_at_launch: bool,
    toggle_hotkey: HotKey,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            visualizer: "Default".into(),
            display_mode: "command".into(),
            svelte_display_all: true,
            display_modified_characters: false,
            font_size: 16.0,
            bezel_color: "rgba(0,0,0,0.8)".into(),
            text_color: "rgba(255,255,255,1)".into(),
            fade_delay: 2.0,
            fade_duration: 0.2,
            keystroke_delay: 0.5,
            mouse_display: "none".into(),
            mouse_text: false,
            position: "bottom-left".into(),
            start_casting_at_launch: true,
            toggle_hotkey: HotKey::default(),
        }
    }
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("settings.json"))
}

fn load_settings(app: &AppHandle) -> Settings {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_settings(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_path(app).ok_or("no config dir available")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

// --- Virtual-desktop geometry -------------------------------------------------

/// Bounding box of all monitors `(origin_x, origin_y, width, height)` in physical
/// pixels. The overlay spans this so a click/keystroke anywhere is on it. Origin
/// can be negative (a monitor left of / above the primary). `get_overlay_origin`
/// exposes the origin to the frontend so it can map screen→overlay coordinates.
fn virtual_desktop() -> (i32, i32, u32, u32) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1) as u32;
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1) as u32;
        (x, y, w, h)
    }
}

// --- Overlay window -----------------------------------------------------------

/// Create the always-on-top, transparent, click-through overlay spanning the
/// whole virtual desktop, hidden until casting starts. Built once at startup and
/// reused. `set_ignore_cursor_events(true)` makes every click pass through to the
/// app underneath — we are listen-only and must never steal input.
fn create_overlay(app: &AppHandle) -> Option<WebviewWindow> {
    if let Some(w) = app.get_webview_window(OVERLAY_LABEL) {
        return Some(w);
    }
    let win = match WebviewWindowBuilder::new(
        app,
        OVERLAY_LABEL,
        WebviewUrl::App("overlay.html".into()),
    )
    .title("KeyCastr")
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .shadow(false)
    .transparent(true)
    .focused(false)
    .visible(false)
    .build()
    {
        Ok(w) => w,
        Err(e) => {
            log_line(&format!("failed to create overlay: {e}"));
            return None;
        }
    };

    let (x, y, w, h) = virtual_desktop();
    let _ = win.set_position(PhysicalPosition::new(x, y));
    let _ = win.set_size(PhysicalSize::new(w, h));
    let _ = win.set_ignore_cursor_events(true);
    Some(win)
}

// --- Casting on/off -----------------------------------------------------------

/// Apply a casting state: store the flag, relabel the tray item + tooltip,
/// show/hide the overlay, and broadcast `kc-casting` so the overlay can clear
/// any visuals when turned off.
///
/// The atomics are written immediately (any thread), but the UI mutations are
/// dispatched to the main thread: this is invoked from the worker thread (the
/// hotkey path) and from command threads, and on Windows the tray/menu
/// (`tray-icon`/`muda`) must be mutated on the main thread.
fn apply_casting(app: &AppHandle, on: bool) {
    CASTING.store(on, Ordering::Relaxed);
    FLAGS_RESET.store(true, Ordering::Relaxed);
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(item) = TOGGLE_ITEM.get() {
            let _ = item.set_text(if on { "Stop Casting" } else { "Start Casting" });
        }
        if let Some(tray) = handle.tray_by_id(TRAY_ID) {
            let _ = tray.set_tooltip(Some(if on {
                "KeyCastr — casting"
            } else {
                "KeyCastr — idle"
            }));
        }
        if let Some(w) = handle.get_webview_window(OVERLAY_LABEL) {
            let _ = if on { w.show() } else { w.hide() };
        }
        let _ = handle.emit(CASTING_EVENT, on);
    });
}

fn flip_casting(app: &AppHandle) {
    apply_casting(app, !CASTING.load(Ordering::Relaxed));
}

/// True when `vk` + current modifier state exactly matches the bound toggle.
fn matches_toggle(vk: u32, mods: translate::Modifiers) -> bool {
    if let Some(cell) = TOGGLE.get() {
        if let Ok(t) = cell.lock() {
            return t.vk == vk
                && t.ctrl == mods.ctrl
                && t.alt == mods.alt
                && t.shift == mods.shift
                && t.win == mods.win;
        }
    }
    false
}

/// True when `vk` is the toggle's bound key, ignoring modifiers. Used to rearm
/// the edge-trigger when the key is released even if a modifier was let go first
/// (so the chord no longer fully matches on the key-up).
fn toggle_is_key(vk: u32) -> bool {
    TOGGLE
        .get()
        .and_then(|cell| cell.lock().ok().map(|t| t.vk == vk))
        .unwrap_or(false)
}

// --- Worker -------------------------------------------------------------------

/// Map a raw mouse event to (button, phase) strings for the frontend.
fn mouse_descr(kind: MouseKind) -> (&'static str, &'static str) {
    match kind {
        MouseKind::LeftDown => ("left", "down"),
        MouseKind::LeftUp => ("left", "up"),
        MouseKind::RightDown => ("right", "down"),
        MouseKind::RightUp => ("right", "up"),
        MouseKind::MiddleDown => ("middle", "down"),
        MouseKind::MiddleUp => ("middle", "up"),
        MouseKind::XDown => ("x", "down"),
        MouseKind::XUp => ("x", "up"),
        MouseKind::Move => ("none", "move"),
    }
}

/// Drain raw hook events and emit `kc-event`s to the overlay. Runs on its own
/// thread for the app's lifetime (the channel closes only at shutdown).
///
/// Per event: snapshot live modifiers; recognize + swallow the toggle chord;
/// gate everything else on `CASTING`. Modifier keys emit a deduped "flags"
/// event (so the panel lights up without spamming on auto-repeat); other keys
/// emit a translated "key" event on key-down only; mouse events pass through.
fn run_worker(app: AppHandle, rx: Receiver<RawEvent>) {
    let mut last_flags: Option<translate::Modifiers> = None;
    // Edge-trigger state for the toggle chord. A held hotkey auto-repeats
    // WM_KEYDOWN ~30x/s, so flipping on every down would thrash casting on/off
    // and land in a random state. We flip only on the first down and rearm once
    // the toggle key is released.
    let mut toggle_armed = true;
    for ev in rx {
        // A casting on/off transition invalidates the dedupe cache so the next
        // modifier change is always re-emitted (see FLAGS_RESET).
        if FLAGS_RESET.swap(false, Ordering::Relaxed) {
            last_flags = None;
        }
        match ev {
            RawEvent::Key { vk, scan, down } => {
                let mods = translate::modifiers_now();

                // Toggle: flip on the rising edge of the bound (non-modifier)
                // chord, then swallow it so it's never displayed. Auto-repeat
                // downs are swallowed without flipping; the key-up rearms.
                if !translate::is_modifier_vk(vk) && matches_toggle(vk, mods) {
                    if down {
                        if toggle_armed {
                            toggle_armed = false;
                            flip_casting(&app);
                        }
                    } else {
                        toggle_armed = true;
                    }
                    continue;
                }
                // Also rearm if the toggle key itself is released when the chord
                // no longer fully matches (a modifier was let go before the key).
                if !down && toggle_is_key(vk) {
                    toggle_armed = true;
                }

                if !CASTING.load(Ordering::Relaxed) {
                    continue;
                }

                if translate::is_modifier_vk(vk) {
                    // Emit only when the modifier set actually changed, so a
                    // held modifier's key-repeat doesn't flood the channel.
                    if last_flags != Some(mods) {
                        last_flags = Some(mods);
                        let _ = app.emit_to(
                            OVERLAY_LABEL,
                            KC_EVENT,
                            json!({ "kind": "flags", "mods": mods }),
                        );
                    }
                } else if down {
                    let label = translate::translate(vk, scan, mods);
                    let _ = app.emit_to(
                        OVERLAY_LABEL,
                        KC_EVENT,
                        json!({ "kind": "key", "vk": vk, "mods": mods, "label": label }),
                    );
                }
            }
            RawEvent::Mouse { kind, x, y } => {
                if !CASTING.load(Ordering::Relaxed) {
                    continue;
                }
                let (button, phase) = mouse_descr(kind);
                // Live modifier state so the overlay can render key+click combos
                // ("Ctrl+Left Click") when click-as-text is enabled. Cheap, and
                // the overlay only uses it on button-down.
                let mods = translate::modifiers_now();
                let _ = app.emit_to(
                    OVERLAY_LABEL,
                    KC_EVENT,
                    json!({ "kind": "mouse", "button": button, "phase": phase, "x": x, "y": y, "mods": mods }),
                );
            }
        }
    }
}

// --- Commands -----------------------------------------------------------------

#[tauri::command]
fn get_settings(app: AppHandle) -> Settings {
    load_settings(&app)
}

/// Persist new preferences, rebind the live toggle chord, and push them to the
/// overlay so appearance/behavior changes apply without a restart.
#[tauri::command]
fn set_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    if let Some(cell) = TOGGLE.get() {
        if let Ok(mut t) = cell.lock() {
            *t = settings.toggle_hotkey.clone();
        }
    }
    save_settings(&app, &settings)?;
    let _ = app.emit_to(OVERLAY_LABEL, SETTINGS_EVENT, settings.clone());
    Ok(())
}

#[tauri::command]
fn get_casting() -> bool {
    CASTING.load(Ordering::Relaxed)
}

#[tauri::command]
fn set_casting(app: AppHandle, on: bool) {
    apply_casting(&app, on);
}

#[tauri::command]
fn toggle_casting(app: AppHandle) {
    flip_casting(&app);
}

/// Top-left of the virtual desktop in physical pixels. The overlay's webview is
/// positioned at this origin, so the frontend subtracts it (then divides by
/// devicePixelRatio) to turn an absolute mouse position into an in-overlay CSS
/// coordinate. Uniform-DPI assumption: a single scale is applied across all
/// monitors, so mixed-DPI setups can be slightly off — documented in README.
#[tauri::command]
fn get_overlay_origin() -> (i32, i32) {
    let (x, y, _, _) = virtual_desktop();
    (x, y)
}

#[tauri::command]
fn open_preferences(app: AppHandle) {
    if let Some(w) = app.get_webview_window(PREFS_LABEL) {
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(&app, PREFS_LABEL, WebviewUrl::App("prefs.html".into()))
        .title("KeyCastr Preferences")
        .inner_size(420.0, 580.0)
        .resizable(false)
        .build();
}

#[tauri::command]
fn close_preferences(app: AppHandle) {
    if let Some(w) = app.get_webview_window(PREFS_LABEL) {
        let _ = w.close();
    }
}

// --- Auto-update --------------------------------------------------------------
//
// The Tauri updater (configured in tauri.conf.json `plugins.updater`) fetches a
// signed `latest.json` from the GitHub "latest" release, compares versions, and
// — if newer — downloads and runs the NSIS installer. We drive it entirely from
// the backend (no JS updater bindings, so no extra capability), prompting via
// `tauri-plugin-dialog`.
//
// `run_check` is launched once at startup with `interactive=false` (stay silent
// when already current) and by the `check_for_updates` command with
// `interactive=true` (also report "no updates" / errors). The network call goes
// over the updater's own HTTP client — NOT the webview — so the webview CSP does
// not apply. To change the manifest URL or install mode, edit `plugins.updater`
// in tauri.conf.json.

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

async fn run_check(app: AppHandle, interactive: bool) {
    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            log_line(&format!("updater init failed: {e}"));
            return;
        }
    };

    match updater.check().await {
        Ok(Some(update)) => {
            let new_version = update.version.clone();
            log_line(&format!(
                "update available: {CURRENT_VERSION} -> {new_version}"
            ));
            let approved = app
                .dialog()
                .message(format!(
                    "KeyCastr {new_version} is available (you have {CURRENT_VERSION}).\n\nUpdate now? The app will close to install and then reopen."
                ))
                .title("KeyCastr update")
                .buttons(MessageDialogButtons::OkCancelCustom(
                    "Update".into(),
                    "Later".into(),
                ))
                .blocking_show();
            if !approved {
                return;
            }
            match update.download_and_install(|_, _| {}, || {}).await {
                Ok(_) => {
                    // On Windows the installer terminates the app itself; the
                    // restart is a best-effort relaunch where supported.
                    log_line("update installed; restarting");
                    app.restart();
                }
                Err(e) => {
                    log_line(&format!("update install failed: {e}"));
                    if interactive {
                        let _ = app
                            .dialog()
                            .message(format!("The update couldn't be installed:\n{e}"))
                            .title("KeyCastr update")
                            .blocking_show();
                    }
                }
            }
        }
        Ok(None) => {
            if interactive {
                let _ = app
                    .dialog()
                    .message(format!("You're on the latest version ({CURRENT_VERSION})."))
                    .title("KeyCastr update")
                    .blocking_show();
            }
        }
        Err(e) => {
            log_line(&format!("update check failed: {e}"));
            if interactive {
                let _ = app
                    .dialog()
                    .message(format!("Couldn't check for updates:\n{e}"))
                    .title("KeyCastr update")
                    .blocking_show();
            }
        }
    }
}

/// The running app version, shown in Preferences.
#[tauri::command]
fn get_version() -> &'static str {
    CURRENT_VERSION
}

/// Manual "Check for updates" (Preferences button). Spawns an interactive check
/// so the user gets feedback even when already up to date.
#[tauri::command]
fn check_for_updates(app: AppHandle) {
    tauri::async_runtime::spawn(run_check(app, true));
}

// --- Entry point --------------------------------------------------------------

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_settings,
            get_casting,
            set_casting,
            toggle_casting,
            get_overlay_origin,
            open_preferences,
            close_preferences,
            get_version,
            check_for_updates,
        ])
        .setup(|app| {
            init_logging(app.handle());
            let settings = load_settings(app.handle());
            let _ = TOGGLE.set(Mutex::new(settings.toggle_hotkey.clone()));

            // --- System tray ---
            let toggle_i = MenuItem::with_id(app, "toggle", "Start Casting", true, None::<&str>)?;
            let prefs_i = MenuItem::with_id(app, "prefs", "Preferences…", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle_i, &prefs_i, &quit_i])?;
            let _ = TOGGLE_ITEM.set(toggle_i);
            TrayIconBuilder::with_id(TRAY_ID)
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("KeyCastr — idle")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => flip_casting(app),
                    "prefs" => open_preferences(app.clone()),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // Pre-build the (hidden) overlay so the first cast doesn't pay the
            // WebView2 cold start.
            let _ = create_overlay(app.handle());

            // Install hooks and start the worker that turns raw events into
            // overlay `kc-event`s.
            let (tx, rx) = std::sync::mpsc::channel::<RawEvent>();
            hook::start(tx);
            let worker_app = app.handle().clone();
            std::thread::spawn(move || run_worker(worker_app, rx));

            if settings.start_casting_at_launch {
                apply_casting(app.handle(), true);
            }

            // Check GitHub for a newer signed release. Silent if already current;
            // prompts only when an update is found (see `run_check`).
            let update_app = app.handle().clone();
            tauri::async_runtime::spawn(run_check(update_app, false));

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building KeyCastr")
        .run(|_app, event| {
            // Keep the app alive in the tray; there is no main window to close.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
