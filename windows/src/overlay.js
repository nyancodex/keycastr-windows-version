// overlay.js — drives the three visualizers from Tauri events.
//
// The Rust worker (src-tauri/src/main.rs `run_worker`) emits one `kc-event` per
// captured input event to this window, plus `kc-casting` (on/off) and
// `kc-settings` (live preference updates). This file renders them, choosing the
// Default or Svelte keystroke visualizer per settings, and drawing mouse-click
// circles. Display formatting (modifier prefixes, casing, command/modified/all
// gating) is delegated to transformer.js (`window.KCTransformer`).
//
// Event payload shapes (from run_worker):
//   { kind: "key",   vk, mods:{ctrl,alt,shift,win}, label:{special,plain,typed} }
//   { kind: "flags", mods:{...} }                  // a modifier set changed
//   { kind: "mouse", button, phase:"down|up|move", x, y, mods }  // x/y screen px
//
// To change visualizer behavior, edit the noteX functions below; to change the
// look, edit overlay.css; to change formatting, edit transformer.js.

(function () {
  var invoke = window.__TAURI__.core.invoke;
  var listen = window.__TAURI__.event.listen;
  var T = window.KCTransformer;

  var settings = null;
  var overlayOrigin = [0, 0]; // virtual-desktop top-left in physical px

  var root = document.documentElement;
  var defaultLayer = document.getElementById("default-visualizer");
  var defaultAnchor = document.getElementById("default-anchor");
  var svelteLayer = document.getElementById("svelte-visualizer");
  var svelteKeys = document.getElementById("svelte-keys");
  var mouseLayer = document.getElementById("mouse-layer");

  var svelteSlots = {};
  (function indexSvelteSlots() {
    var nodes = svelteLayer.querySelectorAll(".svelte-mod");
    for (var i = 0; i < nodes.length; i++) {
      svelteSlots[nodes[i].getAttribute("data-mod")] = nodes[i];
    }
  })();

  // --- Settings application -------------------------------------------------

  function applySettings(s) {
    settings = s;
    root.style.setProperty("--kc-font-size", s.fontSize + "px");
    root.style.setProperty("--kc-bezel", s.bezelColor);
    root.style.setProperty("--kc-text", s.textColor);

    var corner = "corner-" + (s.position || "bottom-left");
    [defaultLayer, svelteLayer].forEach(function (el) {
      el.className = el.className.replace(/corner-[\w-]+/g, "").trim();
      el.classList.add(corner);
    });

    var useSvelte = s.visualizer === "Svelte";
    defaultLayer.classList.toggle("hidden", useSvelte);
    svelteLayer.classList.toggle("hidden", !useSvelte);
  }

  function clearAll() {
    defaultAnchor.innerHTML = "";
    svelteKeys.textContent = "";
    svelteKeys.style.opacity = "1"; // undo a fade left mid-flight
    for (var k in svelteSlots) svelteSlots[k].classList.remove("on");
    var circles = mouseLayer.querySelectorAll(".kc-click");
    for (var i = 0; i < circles.length; i++) circles[i].remove();
    activeCircles = {}; // drop refs to the just-removed circle elements
    curLine = null;
  }

  // --- Default visualizer (KCDefaultVisualizer) -----------------------------
  //
  // A column of fading bezel rows. Command/special keystrokes each get their own
  // row; a run of plain printable characters accumulates into one row until a
  // pause longer than `keystrokeDelay`. Each row fades out `fadeDelay` seconds
  // after its last update, over `fadeDuration` seconds, then is removed.

  var curLine = null;
  var lastKeyTime = 0;

  // A key with no special glyph and no printable character (e.g. browser/launch
  // keys the layout maps to nothing) has nothing to show. Skip it so it can't
  // render an empty bezel row or a lone "Ctrl+" prefix when modified.
  function isEmptyKey(ev) {
    var L = ev.label || {};
    return !L.special && !L.plain && !L.typed;
  }

  function defaultNoteKey(ev) {
    if (isEmptyKey(ev)) return;
    if (!T.shouldDisplay(ev.mods, settings.displayMode)) return;
    var now = performance.now();
    var text = T.formatKeystroke(ev, settings);
    var isSpecial = !!(ev.label && ev.label.special);
    var plainRun = !T.isModified(ev.mods) && !isSpecial;
    var withinDelay = now - lastKeyTime <= settings.keystrokeDelay * 1000;

    if (
      curLine &&
      plainRun &&
      withinDelay &&
      curLine.dataset.plainRun === "1"
    ) {
      curLine.textContent += text;
      bumpFade(curLine);
    } else {
      curLine = newLine(text);
      curLine.dataset.plainRun = plainRun ? "1" : "0";
    }
    lastKeyTime = now;
  }

  function newLine(text) {
    var el = document.createElement("div");
    el.className = "kc-line";
    el.textContent = text;
    defaultAnchor.appendChild(el);
    while (defaultAnchor.children.length > 12) {
      defaultAnchor.removeChild(defaultAnchor.firstChild);
    }
    bumpFade(el);
    return el;
  }

  // (Re)start a row's fade countdown. Cancels any pending fade first so a row
  // that keeps receiving characters stays fully visible until typing stops.
  function bumpFade(el) {
    if (el._fadeTimer) clearTimeout(el._fadeTimer);
    if (el._removeTimer) clearTimeout(el._removeTimer);
    el.style.transition = "none";
    el.style.opacity = "1";
    void el.offsetWidth; // flush the opacity reset before the next transition
    var delay = settings.fadeDelay * 1000;
    var dur = settings.fadeDuration * 1000;
    el._fadeTimer = setTimeout(function () {
      el.style.transition = "opacity " + dur + "ms linear";
      el.style.opacity = "0";
      el._removeTimer = setTimeout(function () {
        if (el === curLine) curLine = null;
        el.remove();
      }, dur + 50);
    }, delay);
  }

  // --- Svelte visualizer (SvelteVisualizer) ---------------------------------
  //
  // Four modifier slots that light while held, plus a row showing the most
  // recent keys. With svelteDisplayAll off, only modified keystrokes appear.

  function svelteNoteFlags(mods) {
    for (var i = 0; i < T.MOD_ORDER.length; i++) {
      var k = T.MOD_ORDER[i];
      if (svelteSlots[k]) svelteSlots[k].classList.toggle("on", !!mods[k]);
    }
  }

  function svelteNoteKey(ev) {
    if (isEmptyKey(ev)) return;
    if (!settings.svelteDisplayAll && !T.isModified(ev.mods)) return;
    var text = T.formatKeystroke(ev, settings);
    // Keep a short tail of recent keys; long modified chords replace the line.
    var existing = settings.svelteDisplayAll && !T.isModified(ev.mods)
      ? svelteKeys.textContent
      : "";
    var combined = (existing + text).slice(-12);
    svelteKeys.textContent = combined;
    bumpSvelteFade();
  }

  // (Re)start the keys-row fade. Two cancellable timers: `_fade` starts the
  // opacity transition after fadeDelay, then `_clear` empties the text once it
  // has faded out — without the clear, the next keystroke would read the
  // invisible stale text and concatenate the old keys back onto the line.
  function bumpSvelteFade() {
    if (bumpSvelteFade._fade) clearTimeout(bumpSvelteFade._fade);
    if (bumpSvelteFade._clear) clearTimeout(bumpSvelteFade._clear);
    svelteKeys.style.transition = "none";
    svelteKeys.style.opacity = "1";
    void svelteKeys.offsetWidth;
    var delay = settings.fadeDelay * 1000;
    var dur = settings.fadeDuration * 1000;
    bumpSvelteFade._fade = setTimeout(function () {
      svelteKeys.style.transition = "opacity " + dur + "ms linear";
      svelteKeys.style.opacity = "0";
      bumpSvelteFade._clear = setTimeout(function () {
        svelteKeys.textContent = "";
      }, dur + 50);
    }, delay);
  }

  // --- Mouse visualizer (KCMouseEventVisualizer) ----------------------------
  //
  // Two independent outputs, each toggled in Preferences:
  //   * click circles (settings.mouseDisplay === "current") — a circle at the
  //     click point that follows a drag and fades on release. Mouse x/y arrive
  //     in physical screen pixels; we map them into this window's CSS pixels by
  //     subtracting the overlay origin and dividing by devicePixelRatio.
  //     Uniform-DPI assumption (documented in README): one scale across all
  //     monitors, so mixed-DPI setups can be slightly off.
  //   * text labels (settings.mouseText) — each button-down also shows a label
  //     in the active keystroke visualizer, reusing its fade. Modifiers held at
  //     click time are prefixed, so key+mouse combos read "Ctrl+Left Click".
  // They are independent: enable circles, text, both, or neither.

  var activeCircles = {}; // button -> element

  // Button → human label for the text option.
  var MOUSE_TEXT = {
    left: "Left Click",
    right: "Right Click",
    middle: "Middle Click",
    x: "Side Click",
  };

  function toCss(x, y) {
    var dpr = window.devicePixelRatio || 1;
    return [(x - overlayOrigin[0]) / dpr, (y - overlayOrigin[1]) / dpr];
  }

  // Show a click as a text label in whichever keystroke visualizer is active,
  // bypassing the key display-mode gating (a click isn't a keystroke). In the
  // Default bezel it gets its own row and ends any in-progress plain-key run.
  function noteMouseText(button, mods) {
    // Prefix any modifiers held at click time (isSpecial=true so Shift is always
    // shown, like a special key) → "Ctrl+Left Click", "Ctrl+Shift+Right Click".
    var prefix = mods ? T.modifierPrefix(mods, false, true) : "";
    var text = prefix + (MOUSE_TEXT[button] || "Click");
    if (settings.visualizer === "Svelte") {
      svelteKeys.textContent = text;
      bumpSvelteFade();
    } else {
      newLine(text);
      curLine = null; // a click breaks the current accumulating plain-key row
    }
  }

  function noteMouse(ev) {
    if (!settings) return;
    var showCircles = settings.mouseDisplay === "current";
    var showText = !!settings.mouseText;
    if (!showCircles && !showText) return;

    // Text fires on button-down only (a "click"); move/up are circle-only.
    if (showText && ev.phase === "down") noteMouseText(ev.button, ev.mods);
    if (!showCircles) return;

    var p = toCss(ev.x, ev.y);
    if (ev.phase === "down") {
      activeCircles[ev.button] = makeCircle(p[0], p[1]);
    } else if (ev.phase === "move") {
      for (var b in activeCircles) positionCircle(activeCircles[b], p[0], p[1]);
    } else if (ev.phase === "up") {
      var c = activeCircles[ev.button];
      if (c) {
        fadeCircle(c);
        delete activeCircles[ev.button];
      }
    }
  }

  function makeCircle(cx, cy) {
    var c = document.createElement("div");
    c.className = "kc-click";
    positionCircle(c, cx, cy);
    mouseLayer.appendChild(c);
    return c;
  }
  function positionCircle(c, cx, cy) {
    c.style.left = cx + "px";
    c.style.top = cy + "px";
  }
  function fadeCircle(c) {
    c.classList.add("fade");
    setTimeout(function () {
      c.remove();
    }, 300);
  }

  // --- Event wiring ---------------------------------------------------------

  function onEvent(p) {
    if (!settings) return;
    if (p.kind === "key") {
      if (settings.visualizer === "Svelte") svelteNoteKey(p);
      else defaultNoteKey(p);
    } else if (p.kind === "flags") {
      // Only the Svelte visualizer reflects bare modifier changes.
      if (settings.visualizer === "Svelte") svelteNoteFlags(p.mods);
    } else if (p.kind === "mouse") {
      noteMouse(p);
    }
  }

  listen("kc-event", function (e) {
    onEvent(e.payload);
  });
  listen("kc-settings", function (e) {
    applySettings(e.payload);
  });
  listen("kc-casting", function (e) {
    if (!e.payload) clearAll();
  });

  // Initial state: pull settings + the overlay origin before any event arrives.
  invoke("get_settings").then(function (s) {
    applySettings(s);
  });
  invoke("get_overlay_origin").then(function (o) {
    overlayOrigin = o;
  });
})();
