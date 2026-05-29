// transformer.js — keystroke display formatting for KeyCastr (Windows).
//
// This is the Windows counterpart to macOS KeyCastr's `KCEventTransformer`: it
// turns a raw key event (a Win32 virtual-key code + live modifier flags + the
// translated label produced by `src-tauri/src/translate.rs`) into the string
// shown on screen, and decides whether a given keystroke should be displayed
// under the current display mode.
//
// It is pure (no DOM, no Tauri) so both visualizers in `overlay.js` can share
// it. The Rust side does layout-aware character translation; this side does
// only presentation (modifier prefixes, casing, command/modified/all gating) —
// the same capture/format split KeyCastr uses between its Obj-C tap and its
// display layer.
//
// Modifier model on Windows (differs from macOS): the "command-like" modifiers
// are Ctrl and Win (KeyCastr treats Control and Command as command keys), so
// the command-keys-only mode shows a keystroke when Ctrl or Win is held.
//
// To change modifier glyphs or the gating rules, edit this file. To change
// which raw glyph a special key shows, edit `translate.rs` (`special_key`).

(function () {
  // Order modifiers are rendered in, and their on-screen labels.
  var MOD_ORDER = ["ctrl", "alt", "shift", "win"];
  var MOD_LABEL = { ctrl: "Ctrl", alt: "Alt", shift: "Shift", win: "Win" };

  // KeyCastr's KCKeystroke.isCommand — Control OR Command held. On Windows the
  // command-like modifiers are Ctrl and Win.
  function isCommand(m) {
    return !!(m && (m.ctrl || m.win));
  }
  // KCKeystroke.isModified — any modifier held.
  function isModified(m) {
    return !!(m && (m.ctrl || m.alt || m.shift || m.win));
  }

  // Whether a keystroke should appear under `mode`:
  //   "all"      — every keystroke, including plain typing
  //   "modified" — only keystrokes with any modifier
  //   "command"  — only keystrokes with a command-like modifier (Ctrl/Win)
  function shouldDisplay(mods, mode) {
    if (mode === "all") return true;
    if (mode === "modified") return isModified(mods);
    return isCommand(mods);
  }

  // Build the modifier prefix string, e.g. "Ctrl+Shift+". Shift is folded into
  // the character (omitted here) when we display the literal typed character and
  // the key is not a special key — because the shifted glyph (e.g. "!") already
  // encodes Shift. Ctrl/Alt/Win are always shown when held.
  function modifierPrefix(m, displayModified, isSpecial) {
    var parts = [];
    if (m.ctrl) parts.push(MOD_LABEL.ctrl);
    if (m.alt) parts.push(MOD_LABEL.alt);
    if (m.shift && (isSpecial || !displayModified)) parts.push(MOD_LABEL.shift);
    if (m.win) parts.push(MOD_LABEL.win);
    return parts.length ? parts.join("+") + "+" : "";
  }

  // The base label (no modifier prefix) for a key event. Prefers the special-key
  // glyph; otherwise the plain or typed character depending on the
  // displayModifiedCharacters preference. A single letter is uppercased when any
  // modifier is held, matching KeyCastr's "⌘A" style.
  function baseLabel(ev, displayModified) {
    var L = ev.label || {};
    if (L.special) return L.special;
    if (displayModified) {
      return (L.typed != null ? L.typed : L.plain) || "";
    }
    var p = (L.plain != null ? L.plain : L.typed) || "";
    if (p.length === 1 && isModified(ev.mods)) p = p.toUpperCase();
    return p;
  }

  // Full on-screen string for a key event, e.g. "Ctrl+Shift+A", "Esc", "!".
  function formatKeystroke(ev, settings) {
    var displayModified = !!settings.displayModifiedCharacters;
    var isSpecial = !!(ev.label && ev.label.special);
    var base = baseLabel(ev, displayModified);
    return modifierPrefix(ev.mods, displayModified, isSpecial) + base;
  }

  window.KCTransformer = {
    isCommand: isCommand,
    isModified: isModified,
    shouldDisplay: shouldDisplay,
    modifierPrefix: modifierPrefix,
    baseLabel: baseLabel,
    formatKeystroke: formatKeystroke,
    MOD_ORDER: MOD_ORDER,
    MOD_LABEL: MOD_LABEL,
  };
})();
