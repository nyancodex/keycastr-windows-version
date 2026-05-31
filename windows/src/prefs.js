// prefs.js — binds the Preferences form to the Rust `Settings`.
//
// Flow: load current settings (`get_settings`) → fill the form → on any change,
// collect the form into a Settings object and call `set_settings`, which
// persists it and live-pushes it to the overlay. The casting button mirrors and
// drives `get_casting`/`toggle_casting` (kept in sync via the `kc-casting`
// event). Colors are stored as `rgba(...)` strings but edited with a native
// color picker + an alpha slider, so we convert both ways here.
//
// To add a preference: add a control in prefs.html, then read it in `collect`
// and write it in `fillForm`, matching the field name in main.rs `Settings`.

(function () {
  var invoke = window.__TAURI__.core.invoke;
  var listen = window.__TAURI__.event.listen;

  var $ = function (id) {
    return document.getElementById(id);
  };

  // The current toggle hotkey, captured from the keydown handler below.
  var hotkey = null;

  // --- rgba <-> (hex, alpha) ------------------------------------------------

  function clamp255(n) {
    return Math.max(0, Math.min(255, n | 0));
  }
  function toHex2(n) {
    var s = clamp255(n).toString(16);
    return s.length === 1 ? "0" + s : s;
  }
  function parseRgba(str) {
    str = (str || "").trim();
    var m = str.match(/^rgba?\(([^)]+)\)$/i);
    if (m) {
      var p = m[1].split(",").map(function (x) {
        return x.trim();
      });
      return {
        hex: "#" + toHex2(+p[0]) + toHex2(+p[1]) + toHex2(+p[2]),
        a: p.length > 3 ? Math.max(0, Math.min(1, parseFloat(p[3]))) : 1,
      };
    }
    var h = str.match(/^#([0-9a-f]{6})$/i);
    if (h) return { hex: "#" + h[1].toLowerCase(), a: 1 };
    return { hex: "#000000", a: 1 };
  }
  function toRgba(hex, a) {
    var r = parseInt(hex.slice(1, 3), 16);
    var g = parseInt(hex.slice(3, 5), 16);
    var b = parseInt(hex.slice(5, 7), 16);
    return "rgba(" + r + "," + g + "," + b + "," + a + ")";
  }

  // --- Form <-> Settings ----------------------------------------------------

  function fillForm(s) {
    $("visualizer").value = s.visualizer;
    $("displayMode").value = s.displayMode;
    $("svelteDisplayAll").checked = !!s.svelteDisplayAll;
    $("displayModifiedCharacters").checked = !!s.displayModifiedCharacters;
    $("fontSize").value = s.fontSize;

    var bz = parseRgba(s.bezelColor);
    $("bezelColorHex").value = bz.hex;
    $("bezelColorA").value = bz.a;
    var tx = parseRgba(s.textColor);
    $("textColorHex").value = tx.hex;
    $("textColorA").value = tx.a;

    $("position").value = s.position;
    $("fadeDelay").value = s.fadeDelay;
    $("fadeDuration").value = s.fadeDuration;
    $("keystrokeDelay").value = s.keystrokeDelay;
    $("mouseDisplay").value = s.mouseDisplay;
    $("mouseText").checked = !!s.mouseText;
    $("startCastingAtLaunch").checked = !!s.startCastingAtLaunch;

    hotkey = s.toggleHotkey;
    $("toggleHotkey").value = hotkey.label;

    updateVisibility();
  }

  function collect() {
    return {
      visualizer: $("visualizer").value,
      displayMode: $("displayMode").value,
      svelteDisplayAll: $("svelteDisplayAll").checked,
      displayModifiedCharacters: $("displayModifiedCharacters").checked,
      fontSize: parseFloat($("fontSize").value) || 16,
      bezelColor: toRgba($("bezelColorHex").value, parseFloat($("bezelColorA").value)),
      textColor: toRgba($("textColorHex").value, parseFloat($("textColorA").value)),
      fadeDelay: parseFloat($("fadeDelay").value) || 0,
      fadeDuration: parseFloat($("fadeDuration").value) || 0,
      keystrokeDelay: parseFloat($("keystrokeDelay").value) || 0,
      mouseDisplay: $("mouseDisplay").value,
      mouseText: $("mouseText").checked,
      position: $("position").value,
      startCastingAtLaunch: $("startCastingAtLaunch").checked,
      toggleHotkey: hotkey,
    };
  }

  function save() {
    invoke("set_settings", { settings: collect() }).catch(function (e) {
      console.error("set_settings failed:", e);
    });
  }

  // Show only the rows relevant to the chosen visualizer style.
  function updateVisibility() {
    var style = $("visualizer").value;
    var rows = document.querySelectorAll(".row[data-only]");
    for (var i = 0; i < rows.length; i++) {
      rows[i].classList.toggle("visible", rows[i].getAttribute("data-only") === style);
    }
  }

  // --- Hotkey capture -------------------------------------------------------
  //
  // `keyCode` is deprecated in the DOM but still reports the Win32 virtual-key
  // code for the keys we care about (letters/digits/F-keys), which is exactly
  // what the Rust worker compares against — so we store it as `vk`.

  var MODIFIER_KEYCODES = [16, 17, 18, 91, 92]; // Shift, Ctrl, Alt, L/R Win

  function captureHotkey(e) {
    e.preventDefault();
    if (MODIFIER_KEYCODES.indexOf(e.keyCode) !== -1) return; // ignore bare mods
    var mods = {
      ctrl: e.ctrlKey,
      alt: e.altKey,
      shift: e.shiftKey,
      win: e.metaKey,
    };
    var name = e.key.length === 1 ? e.key.toUpperCase() : e.key;
    var parts = [];
    if (mods.ctrl) parts.push("Ctrl");
    if (mods.alt) parts.push("Alt");
    if (mods.shift) parts.push("Shift");
    if (mods.win) parts.push("Win");
    parts.push(name);
    hotkey = {
      ctrl: mods.ctrl,
      alt: mods.alt,
      shift: mods.shift,
      win: mods.win,
      vk: e.keyCode,
      label: parts.join("+"),
    };
    $("toggleHotkey").value = hotkey.label;
    save();
  }

  // --- Casting button -------------------------------------------------------

  function reflectCasting(on) {
    var btn = $("cast-toggle");
    btn.textContent = on ? "Stop Casting" : "Start Casting";
    btn.classList.toggle("on", on);
  }

  // --- Wiring ---------------------------------------------------------------

  // Save on any input change.
  document.querySelectorAll("input, select").forEach(function (el) {
    if (el.id === "toggleHotkey") return; // handled by capture
    el.addEventListener("change", function () {
      updateVisibility();
      save();
    });
  });

  $("toggleHotkey").addEventListener("keydown", captureHotkey);

  $("cast-toggle").addEventListener("click", function () {
    invoke("toggle_casting");
  });
  $("close-btn").addEventListener("click", function () {
    invoke("close_preferences");
  });

  // Manual update check. The backend (run_check in main.rs) shows the result in
  // a native dialog, so this just kicks it off; we briefly disable the button as
  // feedback since the command returns immediately (the check runs async).
  $("check-updates").addEventListener("click", function () {
    var btn = $("check-updates");
    var prev = btn.textContent;
    btn.disabled = true;
    btn.textContent = "Checking…";
    invoke("check_for_updates");
    setTimeout(function () {
      btn.disabled = false;
      btn.textContent = prev;
    }, 4000);
  });

  listen("kc-casting", function (e) {
    reflectCasting(e.payload);
  });

  // Initial load.
  invoke("get_settings").then(fillForm);
  invoke("get_casting").then(reflectCasting);
  invoke("get_version").then(function (v) {
    $("appVersion").textContent = "v" + v;
  });
})();
