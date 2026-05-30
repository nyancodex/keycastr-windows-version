# KeyCastr for Windows

A Windows reimplementation of [KeyCastr](https://github.com/keycastr/keycastr),
the keystroke visualizer, built on **Tauri 2 + Rust** (the same stack used by
QuickShot). Because KeyCastr is macOS/Cocoa (Objective-C, `CGEventTap`,
`AppKit`) and none of those APIs exist on Windows, this is a from-scratch
reimplementation that reproduces KeyCastr's behavior and preferences — not a
line-by-line port.

It runs as a tray app: while *casting*, every keystroke and (optionally) mouse
click is drawn into a transparent, always-on-top, click-through overlay that
spans all monitors. Toggle casting from the tray, a hotkey, or the Preferences
window.

## Build & run

Prerequisites: the Rust toolchain (MSVC) and Node (only for the Tauri CLI).

```console
cd windows
npm install            # installs @tauri-apps/cli
npm run dev            # run in dev (hot-reloads the frontend)
npm run build          # produce the NSIS installer
```

To just compile-check the Rust without bundling:

```console
cd windows/src-tauri
cargo build
```

Artifacts after `npm run build`: the binary at
`src-tauri/target/release/keycastr.exe` and the installer under
`src-tauri/target/release/bundle/nsis/`.

There is a debug log at `%LOCALAPPDATA%\com.keycastr.windows\logs\keycastr.log`
(see `init_logging`/`log_line` in `src-tauri/src/main.rs`) — ask a tester for it
when diagnosing capture issues.

## Architecture

The pipeline mirrors KeyCastr's split between raw capture (its Obj-C event tap)
and display formatting (its visualizers):

```
 OS input
   │  WH_KEYBOARD_LL / WH_MOUSE_LL  (low-level global hooks)
   ▼
 hook.rs ──mpsc──▶ run_worker (main.rs) ──"kc-event"──▶ overlay webview
   raw events       translate + gate + emit              (transformer.js +
   (fast, on a      (on a normal thread)                  overlay.js render
   pump thread)                                            Default/Svelte/mouse)
```

- **`src-tauri/src/hook.rs`** — installs the Win32 low-level keyboard & mouse
  hooks on a dedicated message-pump thread and pushes each raw event onto an
  `mpsc` channel. Hook callbacks must return fast (Windows drops slow hooks at
  ~300 ms), so they do the bare minimum. This is the `KCEventTap` equivalent.
- **`src-tauri/src/translate.rs`** — turns a virtual-key code into the
  character(s) it produces using the *foreground window's* keyboard layout
  (`ToUnicodeEx`), reads live modifier state (`GetAsyncKeyState`), and maps
  non-character keys to friendly glyphs. This is the keyboard-layout half of
  `KCEventTransformer`.
- **`src-tauri/src/main.rs`** — the app shell (`KCAppController` equivalent):
  tray icon, casting on/off, the click-through overlay window, preferences
  persistence, the Tauri commands, and `run_worker`, which drains the hook
  channel, recognizes/swallows the toggle hotkey, gates on the casting flag,
  translates keys, and emits one `kc-event` per event to the overlay.
- **`src/transformer.js`** — pure display formatting (`KCEventTransformer`'s
  display half): modifier prefixes, letter casing, and the command/modified/all
  gating rules.
- **`src/overlay.{html,css,js}`** — the three visualizers drawing into the
  overlay: Default (corner bezel rows), Svelte (modifier slots + recent keys),
  and mouse click circles.
- **`src/prefs.{html,css,js}`** — the Preferences window.

### Events emitted to the frontend (from `run_worker`)

All sent to the overlay window via `app.emit_to("overlay", "kc-event", …)`:

| `kind`   | payload                                                       |
| -------- | ------------------------------------------------------------- |
| `key`    | `{ vk, mods:{ctrl,alt,shift,win}, label:{special,plain,typed} }` |
| `flags`  | `{ mods }` — emitted only when the modifier set changes       |
| `mouse`  | `{ button, phase:"down"\|"up"\|"move", x, y, mods }` (x/y screen px) |

Plus `kc-casting` (bool, broadcast) and `kc-settings` (the full settings object,
pushed to the overlay on change).

> **Capabilities (don't delete `src-tauri/capabilities/default.json`).** Tauri 2
> is deny-by-default: the frontend's `event.listen()` is the core command
> `core:event:allow-listen` and is **rejected unless a capability grants it**.
> Our own `#[tauri::command]`s (`get_settings`, `get_casting`, …) are *not* gated
> and work regardless — so a missing capability is silent: the Preferences form
> still loads, but the overlay never receives `kc-event`/`kc-settings`/`kc-casting`
> and **no keystrokes ever render**. `capabilities/default.json` grants
> `core:event:default` to the `overlay` and `prefs` windows; if you add another
> window that listens for events, add its label there. The frontend uses no other
> core/plugin commands, so nothing else is granted (keeps the IPC surface minimal).

## KeyCastr → Windows concept map

| KeyCastr (macOS)                       | This port (Windows)                                  |
| -------------------------------------- | ---------------------------------------------------- |
| `CGEventTapCreate` (listen-only)       | `SetWindowsHookExW` `WH_KEYBOARD_LL` / `WH_MOUSE_LL` |
| `flagsChanged` events                  | modifier-VK key events → deduped `flags` events      |
| Command key = Control **or** Command   | command-like modifier = **Ctrl or Win**              |
| `KCDefaultVisualizer` (bezel)          | `#default-visualizer` (corner bezel rows)            |
| `SvelteVisualizer`                     | `#svelte-visualizer` (modifier slots + recent keys)  |
| `KCMouseEventVisualizer`               | `#mouse-layer` (click circles)                       |
| Draggable bezel window                 | fullscreen click-through overlay + corner **presets** |
| Toggle hotkey ⌃⌥⌘K (default)           | **Ctrl+Alt+Shift+K** (default), rebindable           |
| `NSUserDefaults`                       | `settings.json` in the app config dir                |

## Preferences

Persisted as camelCase JSON (the Rust `Settings` struct) in the app config dir;
the defaults mirror KeyCastr's out-of-box behavior. The struct is
`#[serde(default)]`, so a settings file from an older/newer build that is
missing a field still loads (that field falls back to its default) — upgrades
never silently wipe your preferences.

- **Visualizer**: Default or Svelte.
- **Display** (Default): command keys only / all modified keys / all keystrokes.
- **Show all keys** (Svelte): show every key, not just modified ones.
- **Display modified characters**: show the literal typed glyph (`!`) instead of
  `Shift+1`.
- **Font size, bezel color, text color, position** (one of four corners).
- **Timing**: fade delay, fade duration, and the keystroke "join window" (how
  long a run of plain characters keeps accumulating into one bezel row).
- **Mouse clicks** (`mouseDisplay`): off, or show click circles.
- **Show clicks as text** (`mouseText`): also label each click ("Left Click",
  "Right Click", "Middle Click", "Side Click") in the active keystroke
  visualizer. Modifiers held at click time are prefixed, so **key+mouse combos**
  render as `Ctrl+Left Click` / `Ctrl+Shift+Right Click` (live modifier state
  rides along on the `mouse` event's `mods`). Independent of the click-circles
  setting — enable circles, text, both, or neither. Fires on button-down only;
  rendered via `noteMouseText` in `src/overlay.js` (bypasses the key
  display-mode gating, since a click isn't a keystroke).
- **Start casting at launch**, and the **toggle hotkey**.

## Divergences & limitations

- **No draggable bezel.** KeyCastr lets you drag its bezel anywhere; our overlay
  is a single fullscreen click-through window, so position is chosen from four
  corner **presets** in Preferences (`position`) instead.
- **Listen-only toggle swallow.** KeyCastr eats its toggle chord inside the
  event tap. Win32 low-level hooks can't suppress an OS keystroke, so we swallow
  the toggle at the *display* layer — `run_worker` recognizes the chord and
  flips casting without emitting it.
- **Mixed-DPI mouse mapping.** Mouse positions are mapped to overlay CSS pixels
  with a single `devicePixelRatio`, so click circles can be slightly misplaced
  on multi-monitor setups with *different* per-monitor scaling. Same-scale
  setups are exact. (See `toCss` in `overlay.js` and `get_overlay_origin` in
  `main.rs`.)
- **Hotkey vk capture.** The Preferences hotkey field stores the browser
  `keyCode`, which equals the Win32 virtual-key code for the keys that matter
  (letters/digits/F-keys); exotic keys may not round-trip a friendly label.

## Privacy & security

Captured input is rendered to the local overlay and then discarded — it is
never persisted or transmitted. The app makes **no network calls** (unlike
macOS KeyCastr, this port has no Sparkle/auto-updater). The only files written
are `settings.json` (preferences) and the debug log (startup banner + errors —
*not* keystrokes), both in the app config/log dirs.

The webview is locked down with a strict Content-Security-Policy in
`tauri.conf.json` (`app.security.csp`): `script-src 'self'` (no inline/`eval`)
and `connect-src 'self'` (no outbound requests) are the load-bearing
directives, so injected content cannot fetch remote code or exfiltrate. The
frontend is 100% local static files, so this costs nothing. Tauri augments this
CSP at build time with the nonce its IPC bridge needs.

Being a keystroke visualizer, the app installs global low-level keyboard/mouse
hooks (see `hook.rs`) — it sees all system input by design. AV/EDR may flag this
hook behavior; that is expected for this class of tool.

## Where to change things

| To change…                                  | Edit…                                            |
| ------------------------------------------- | ------------------------------------------------ |
| which events are captured                   | `src-tauri/src/hook.rs`                          |
| key → glyph / character translation         | `src-tauri/src/translate.rs` (`special_key`, `translate`) |
| toggle/casting/tray/overlay/commands        | `src-tauri/src/main.rs`                          |
| modifier glyphs, casing, display-mode gating| `src/transformer.js`                             |
| visualizer behavior                         | `src/overlay.js`                                 |
| overlay appearance                          | `src/overlay.css`                                |
| a preference (add a field)                  | `Settings` in `main.rs` **and** `src/prefs.html` + `src/prefs.js` |
| webview CSP / security                      | `src-tauri/tauri.conf.json` (`app.security.csp`) |
| which windows may use the event API (IPC)   | `src-tauri/capabilities/default.json`            |
```
