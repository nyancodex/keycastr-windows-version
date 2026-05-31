# KeyCastr — personal Windows port (unofficial fork)

> **This is a personal, unofficial fork** of [KeyCastr](https://github.com/keycastr/keycastr),
> the open-source keystroke visualizer. It exists to host a **from-scratch
> Windows port** of the app that I built for my **own personal use**. It is
> **not** an official release, and it is **not affiliated with or endorsed by**
> the original KeyCastr authors or the `keycastr` organization.
>
> If you want the real, supported app for macOS, go to the original project:
> **https://github.com/keycastr/keycastr**

## Download & use (Windows)

1. **Download the installer.** Grab `KeyCastr_<version>_x64-setup.exe` from the
   [latest release](https://github.com/nyancodex/keycastr-windows-version/releases/latest).
   (A portable `keycastr.exe` is also attached if you'd rather not install.)
2. **Run it.** The build is unsigned, so Windows SmartScreen may warn the first
   time — click **More info ▸ Run anyway**. The installer adds KeyCastr to your
   apps and launches it.
3. **It lives in the system tray.** There is no main window. Click the tray icon
   for the menu: **Start / Stop Casting**, **Preferences…**, and **Quit**.
4. **Cast your keystrokes.** By default it starts casting as soon as it launches,
   so your keystrokes appear on screen right away. Toggle casting any time from
   the tray menu or with the hotkey (default **Ctrl+Alt+Shift+K**).

### Using it

- **Two visualizer styles** (choose in Preferences):
  - *Default* — a bezel of recent keystrokes pinned to a screen corner.
  - *Svelte* — a compact panel showing held modifiers plus the latest keys.
- **What gets shown** (Default style): command-key combos only, all modified
  keys, or every keystroke.
- **Mouse clicks** — optionally draw a circle at each click.
- **Customize in Preferences:** overlay corner, font size, bezel/text colors,
  fade timing, whether to show the literal typed character (e.g. `!` instead of
  `Shift+1`), the toggle hotkey, and whether to start casting on launch.

> Heads-up: like the macOS original, this is a keystroke visualizer — it
> installs a global keyboard/mouse hook, so antivirus/SmartScreen may flag it.
> Its only network use is the **update check** (it asks GitHub for new releases
> on launch, and downloads one only if you approve) — no telemetry. It stores
> only its settings and a local debug log — never your keystrokes. Details in
> [`windows/README.md`](windows/README.md) § "Privacy & security".

**Updates are automatic from 0.2.0 on.** The app checks GitHub on launch and
prompts to install a newer signed release; you can also trigger a check from
**Preferences ▸ Updates**. (Upgrading *to* 0.2.0 from an older build is a
one-time manual install.)

Building from source instead? See [`windows/README.md`](windows/README.md).

## What's in this fork

| Path        | What it is                                                                 |
| ----------- | -------------------------------------------------------------------------- |
| `keycastr/` | The **original** macOS KeyCastr source (Objective-C / Cocoa), unmodified.  |
| `windows/`  | A **new, from-scratch Windows port** (Tauri 2 + Rust). My addition.        |

The macOS sources under `keycastr/` are left exactly as they came from upstream
(forked at tag **v0.10.5**). All of my work lives under `windows/`.

## The Windows port

KeyCastr is macOS-only — it relies on Cocoa, `CGEventTap`, and AppKit, none of
which exist on Windows. So `windows/` is **not a line-by-line port**: it is a
reimplementation that reproduces KeyCastr's behavior and preferences on a
Windows-native stack (Win32 low-level hooks + `ToUnicodeEx` for capture, a
Tauri/WebView2 overlay for display).

See **[`windows/README.md`](windows/README.md)** for the architecture, build
instructions, the KeyCastr→Windows concept map, and the known divergences.

Quick build (needs the Rust MSVC toolchain and Node for the Tauri CLI):

```console
cd windows
npm install
npm run build      # produces the NSIS installer under src-tauri/target/release/bundle/nsis/
```

## Personal use only

I made this for myself and I'm sharing the source in the open in case it's
useful to read. It comes with **no warranty and no support**, the same "AS IS"
terms as the upstream BSD license below. Use it at your own risk.

Like the original, this is a keystroke visualizer: it installs global low-level
keyboard/mouse hooks and therefore sees all system input by design. Captured
input is only drawn to a local overlay and then discarded — it is never stored
or transmitted. The port's only network use is the GitHub **update check** (no
telemetry), and it writes nothing but its own preferences and a debug log. See
`windows/README.md` § "Privacy & security" for details.

## Credits

All credit for KeyCastr goes to the original authors. From the upstream README:

- [sdeken](https://github.com/sdeken) (Stephen Deken) — wrote the original version.
- [akitchen](https://github.com/akitchen) — occasional development and maintenance.
- [elia](https://github.com/elia) — created the `keycastr` organization and forked into it.
- [lqez](https://github.com/lqez) — added a new menu bar icon.
- [QuintB](https://github.com/QuintB) — designed an updated application icon.

Original project: **https://github.com/keycastr/keycastr**

## License

KeyCastr is licensed under the [BSD 3-Clause License](https://opensource.org/licenses/BSD-3-Clause),
Copyright (c) 2009 Stephen Deken (see [`LICENSE.md`](LICENSE.md)). The Windows
port in `windows/` is released under the same license, and the original
copyright notice is retained as required.

Per the license, the KeyCastr name is **not** used here to endorse or promote
this fork — "KeyCastr for Windows" is only a description of what it ports.
