// --- Settings (Einstellungen): a tvOS-style two-pane screen. Markup is the
// #settings-overlay block in index.html, styles are in settings.css.
//
// Focus lives in one of two zones (mirrored in overlay[data-zone]):
//   list   -- the category list on the left; moving focus there switches the
//             detail pane on the right immediately (no page jump)
//   detail -- the rows of the highlighted category
// (plus a rarely used third one, "close", the round X button, which mostly
// exists so the phone pointer can hit it.)
//
// Keys: Up/Down move; Right/Enter go from the list into the detail; Left or
// Escape go back (Escape in the list closes Settings). On a stepper row
// Left/Right change the value instead; Left on its first value leaves the
// row again, so the arrows never trap you.
//
// The globals other files rely on keep their names: settingsOpen,
// settingsNavItems and settingsFocusIndex are declared in renderer.js (the
// keydown router and the phone pointer read them); openSettings,
// closeSettings, handleSettingsKey, setSettingsFocus and activateSettingsRow
// are defined here and exported at the bottom.
(() => {
  'use strict';

  const overlay = document.getElementById('settings-overlay');
  if (!overlay) return;

  const $ = (id) => document.getElementById(id);
  const content = $('settings-content');
  const pill = $('st-pill');
  const glow = $('st-glow');
  const closeBtn = $('settings-close');
  const catButtons = Array.from(overlay.querySelectorAll('.settings-category'));
  const panels = Array.from(overlay.querySelectorAll('.settings-category-panel'));
  const CAT_COUNT = catButtons.length;
  const DEFAULT_TINT = 'rgba(59,109,255,0.16)';

  // Moving down the category list would otherwise fire a netsh scan / two
  // PowerShell calls for every category it passes -- data is only fetched
  // once focus rests on a category for this long (or when it is entered).
  const LOAD_DWELL_MS = 420;
  const TTL = { wifi: 15000, bluetooth: 30000, remote: 5000, system: 30000 };

  let activeCategory = null;
  let loadTimer = null;
  let btPrefetchTimer = null;
  let openingTimer = null;
  let uptimeTimer = null;
  let scaffoldTimer = null;
  let pillSnap = false; // next pill placement jumps instead of gliding (fresh open)

  const state = {
    wifi: { status: null, networks: [], at: 0, loading: false, busySsid: null, message: '' },
    bt: { radioOn: null, devices: [], at: 0, loading: false, scanning: false, scanned: false, found: [], scanError: '', pairingId: null, failedId: null },
    remoteUrl: null, remoteAt: 0,
    sys: { info: null, at: 0, bootMs: 0 }
  };

  // ---------------------------------------------------------------- helpers
  const esc = escapeHtml; // renderer.js
  const icon = (id, cls) => `<svg class="st-ic${cls ? ' ' + cls : ''}"><use href="#ic-${id}"/></svg>`;
  const clamp = (n, lo, hi) => Math.max(lo, Math.min(hi, n));

  // Lists are re-rendered after every async result; writing identical markup
  // again would restart entrance animations and drop the focus highlight.
  function setHtml(el, html) {
    if (!el || el._stHtml === html) return false;
    el._stHtml = html;
    el.innerHTML = html;
    return true;
  }
  function setText(el, text) {
    if (el && el.textContent !== text) el.textContent = text;
  }

  const signalLevel = (s) => (s >= 75 ? 4 : s >= 50 ? 3 : s >= 25 ? 2 : s > 0 ? 1 : 0);
  const SIGNAL_TEXT = ['kein Signal', 'schwach', 'mittel', 'gut', 'sehr gut'];
  const bars = (level) => `<span class="st-bars" aria-hidden="true">${[1, 2, 3, 4].map((n) => `<i${level >= n ? ' class="on"' : ''}></i>`).join('')}</span>`;
  const spinnerHtml = '<span class="st-spinner"></span>';

  function btDeviceIcon(name) {
    const n = String(name || '');
    if (/kopfh|headphone|headset|earbud|buds|airpods|wh-|wf-|beats/i.test(n)) return 'headphones';
    if (/speaker|lautsprecher|soundbar|boom|jbl|echo|homepod|sonos/i.test(n)) return 'speaker';
    if (/keyboard|tastatur|keys/i.test(n)) return 'keyboard';
    if (/mouse|maus|trackpad|mx master/i.test(n)) return 'mouse';
    if (/controller|gamepad|xbox|dualsense|dualshock|joy-?con/i.test(n)) return 'gamepad';
    if (/watch|band\b/i.test(n)) return 'watch';
    if (/phone|galaxy|pixel|handy|android/i.test(n)) return 'phone';
    if (/\btv\b|fernseher|bravia/i.test(n)) return 'tv';
    return 'bluetooth';
  }

  function fmtGb(bytes) {
    return (bytes / 1073741824).toLocaleString('de-DE', { maximumFractionDigits: 1 });
  }
  function fmtUptime(sec) {
    const d = Math.floor(sec / 86400);
    const h = Math.floor((sec % 86400) / 3600);
    const m = Math.floor((sec % 3600) / 60);
    if (d > 0) return `${d} ${d === 1 ? 'Tag' : 'Tage'} ${h} Std.`;
    if (h > 0) return `${h} Std. ${m} Min.`;
    return m > 0 ? `${m} Min.` : 'Gerade gestartet';
  }

  // ------------------------------------------------- focus, zones, the pill
  const itemKey = (el) => (el && (el.dataset.key || el.id)) || '';
  const zoneOf = (el) => {
    const kind = el && el.dataset.kind;
    return kind === 'category' ? 'list' : kind === 'close' ? 'close' : 'detail';
  };
  const activePanel = () => panels.find((p) => p.dataset.category === activeCategory);
  const activeCatButton = () => catButtons.find((b) => b.dataset.category === activeCategory);

  function setZone(z) {
    if (overlay.dataset.zone !== z) overlay.dataset.zone = z;
  }

  function applyFocusClass() {
    const cur = settingsNavItems[settingsFocusIndex];
    overlay.querySelectorAll('.settings-nav.focused').forEach((el) => { if (el !== cur) el.classList.remove('focused'); });
    if (cur) cur.classList.add('focused');
    setZone(cur ? zoneOf(cur) : 'list');
  }

  // Flat list (categories, then the active panel's rows, then close) -- the
  // shape renderer.js's phone pointer expects. Rebuilt whenever the category
  // changes or a list re-renders; focus is carried over by data-key.
  function rebuildSettingsNavItems() {
    const prev = settingsNavItems[settingsFocusIndex];
    const prevKey = itemKey(prev);
    const panel = activePanel();
    settingsNavItems = [
      ...catButtons,
      ...(panel ? Array.from(panel.querySelectorAll('.settings-nav')) : []),
      closeBtn
    ];
    let idx = prevKey ? settingsNavItems.findIndex((el) => itemKey(el) === prevKey) : -1;
    if (idx < 0) {
      // The focused row vanished (a list re-rendered or shrank): stay in the
      // detail pane if a row is left to land on, else fall back to the category.
      const lastDetail = settingsNavItems.length - 2;
      if (prev && zoneOf(prev) === 'detail' && lastDetail >= CAT_COUNT) {
        idx = clamp(settingsFocusIndex, CAT_COUNT, lastDetail);
      } else {
        const b = activeCatButton();
        idx = b ? catButtons.indexOf(b) : 0;
      }
    }
    settingsFocusIndex = idx;
    applyFocusClass();
  }

  // One highlight pill glides to the active category instead of every button
  // lighting up separately.
  function placePill(instant) {
    const b = activeCatButton();
    if (!b || !b.offsetHeight) return;
    const snap = instant || pillSnap;
    pillSnap = false;
    if (snap) pill.classList.add('instant');
    pill.style.height = `${b.offsetHeight}px`;
    pill.style.translate = `0 ${b.offsetTop}px`;
    pill.classList.add('ready');
    if (snap) {
      void pill.offsetWidth; // commit the jump before transitions come back
      pill.classList.remove('instant');
    }
  }

  function showCategory(cat) {
    if (activeCategory === cat) return;
    activeCategory = cat;
    catButtons.forEach((b) => b.classList.toggle('active', b.dataset.category === cat));
    let tint = DEFAULT_TINT;
    panels.forEach((p) => {
      const on = p.dataset.category === cat;
      p.classList.toggle('active', on);
      if (on && p.dataset.tint) tint = p.dataset.tint;
    });
    glow.style.setProperty('--st-tint', tint);
    content.scrollTop = 0;
    placePill();
    rebuildSettingsNavItems();
    updateUptimeTimer();
  }

  // Explicit "open this category and put focus on it" (used on open).
  function setSettingsCategory(category) {
    if (!catButtons.some((b) => b.dataset.category === category)) category = catButtons[0].dataset.category;
    showCategory(category);
    settingsFocusIndex = catButtons.indexOf(activeCatButton());
    rebuildSettingsNavItems();
    scheduleLoad(category, 0);
  }

  // Keeps a focused row inside the visible part of the scroller (clear of the
  // top/bottom fade); the first row scrolls all the way up so the header shows.
  function revealRow(el) {
    if (settingsNavItems[CAT_COUNT] === el) {
      if (content.scrollTop > 0) content.scrollTo({ top: 0, behavior: 'smooth' });
      return;
    }
    const c = content.getBoundingClientRect();
    const r = el.getBoundingClientRect();
    const top = c.top + 34;
    const bottom = c.bottom - 76;
    let delta = 0;
    if (r.top < top) delta = r.top - top;
    else if (r.bottom > bottom) delta = r.bottom - bottom;
    if (delta) content.scrollBy({ top: delta, behavior: 'smooth' });
  }

  // Public: also called by the phone pointer (hover) with a plain index.
  function setSettingsFocus(i) {
    if (!settingsNavItems.length) return;
    const idx = clamp(i, 0, settingsNavItems.length - 1);
    const next = settingsNavItems[idx];
    settingsFocusIndex = idx;
    if (zoneOf(next) === 'list' && next.dataset.category !== activeCategory) {
      showCategory(next.dataset.category); // category indexes never move, so idx stays valid
      settingsFocusIndex = settingsNavItems.indexOf(next);
      scheduleLoad(next.dataset.category, LOAD_DWELL_MS);
    }
    applyFocusClass();
    if (zoneOf(next) === 'detail') revealRow(next);
  }

  // A short nudge when the remote hits the end of a list.
  function bump(el, dx, dy) {
    if (!el) return;
    if (el.classList.contains('st-cat')) el = el.querySelector('.st-cat-text') || el;
    // .st-bump swaps out the element's `animation`; a row still carrying its
    // one-off entrance class would replay that entrance once the bump ends.
    el.classList.remove('st-new');
    el.style.setProperty('--bx', `${dx}px`);
    el.style.setProperty('--by', `${dy}px`);
    el.classList.remove('st-bump');
    void el.offsetWidth;
    el.classList.add('st-bump');
    el.addEventListener('animationend', (e) => {
      if (e.animationName === 'stBump') el.classList.remove('st-bump');
    }, { once: true });
  }

  function focusList() {
    const b = activeCatButton();
    if (b) setSettingsFocus(catButtons.indexOf(b));
  }

  function enterDetail() {
    scheduleLoad(activeCategory, 0); // fetch now instead of waiting out the dwell
    if (settingsNavItems.length - 1 > CAT_COUNT) setSettingsFocus(CAT_COUNT);
    else bump(activeCatButton(), 6, 0);
  }

  function moveFocus(dir) {
    const cur = settingsNavItems[settingsFocusIndex];
    const z = zoneOf(cur);
    if (z === 'list') {
      const i = catButtons.indexOf(cur) + dir;
      if (i < 0 || i >= CAT_COUNT) bump(cur, 0, dir * 5);
      else setSettingsFocus(i);
    } else if (z === 'detail') {
      const i = settingsFocusIndex + dir;
      if (i < CAT_COUNT || i > settingsNavItems.length - 2) bump(cur, 0, dir * 5);
      else setSettingsFocus(i);
    } else if (dir > 0) {
      focusList();
    }
  }

  function handleSettingsKey(e) {
    const current = settingsNavItems[settingsFocusIndex];
    if (!current) return;
    const z = zoneOf(current);
    let handled = true;
    switch (e.key) {
      case 'Escape':
        if (z === 'detail') focusList();
        else closeSettings();
        break;
      case 'Backspace':
        if (z === 'detail') focusList();
        break;
      case 'ArrowDown': moveFocus(1); break;
      case 'ArrowUp': moveFocus(-1); break;
      case 'ArrowRight':
        if (z === 'list') enterDetail();
        else if (z === 'detail') {
          if (current.dataset.kind !== 'select' || !stepSelect(current, 1)) bump(current, 6, 0);
        }
        break;
      case 'ArrowLeft':
        if (z === 'detail') {
          if (current.dataset.kind === 'select') {
            // Left past the first value leaves the row (but not while the key
            // is held, so scrolling down to "Langsam" can't fling you out).
            if (!stepSelect(current, -1) && !e.repeat) focusList();
          } else {
            focusList();
          }
        } else if (z === 'close') {
          focusList();
        }
        break;
      case 'Enter':
      case ' ':
        activateSettingsRow(current);
        break;
      default:
        handled = false;
    }
    if (handled) e.preventDefault();
  }

  // --------------------------------------------- toggles and "‹ value ›" steppers
  const TOGGLES = {
    'toggle-adblock': {
      get: () => prefs.adblock,
      set: (v) => { prefs.adblock = v; savePrefs(prefs); window.launcher.setAdblock(v); }
    },
    'toggle-fullscreen': {
      get: () => prefs.fullscreen,
      set: (v) => { prefs.fullscreen = v; savePrefs(prefs); window.launcher.toggleFullscreen(); }
    },
    'toggle-remote-invert': {
      get: () => prefs.remoteInvertY,
      set: (v) => { prefs.remoteInvertY = v; savePrefs(prefs); }
    },
    'toggle-picture-eyecare': {
      get: () => window.picture.current().eyeCare,
      set: (v) => window.picture.setEyeCare(v) // saves + applies (picture.js)
    }
  };

  function syncToggles() {
    for (const [id, t] of Object.entries(TOGGLES)) {
      const sw = $(id);
      if (!sw) continue;
      const on = !!t.get();
      sw.classList.toggle('on', on);
      sw.setAttribute('aria-checked', String(on));
      setText(sw.parentElement.querySelector('.st-row-value'), on ? 'An' : 'Aus');
    }
  }

  function applyToggle(id) {
    const t = TOGGLES[id];
    if (!t) return;
    t.set(!t.get());
    syncToggles();
    updateSummaries();
  }

  // The visible control is a "‹ value ›" stepper, never a real <select> --
  // opening one shows the OS's own unstylable dropdown, which instantly reads
  // as "this is a desktop app". The <select> stays in the DOM (hidden) purely
  // as the value store, so .value / 'change' keep working. `dir` (+1/-1)
  // makes the new value slide in from that side.
  function syncStepperValue(select, dir) {
    const valueEl = $(`${select.id}-value`);
    const text = (select.options[select.selectedIndex] || {}).textContent || '';
    if (valueEl && valueEl.textContent !== text) {
      valueEl.textContent = text;
      if (dir) {
        valueEl.classList.remove('tick-next', 'tick-prev');
        void valueEl.offsetWidth;
        valueEl.classList.add(dir > 0 ? 'tick-next' : 'tick-prev');
      }
    }
    // Small level dots under the value for real scales (speed, timeout, ...).
    const dots = $(`${select.id}-dots`);
    if (dots) {
      const n = select.options.length;
      const want = n > 2 && n <= 8 ? n : 0;
      if (dots.children.length !== want) dots.innerHTML = '<i></i>'.repeat(want);
      Array.from(dots.children).forEach((d, i) => d.classList.toggle('on', i <= select.selectedIndex));
    }
    // Long 0-100 scales (Bild) get a thin level bar instead of dots.
    const level = $(`${select.id}-level`);
    if (level) {
      const n = select.options.length;
      level.style.setProperty('--lv', n > 1 ? String(Math.max(0, select.selectedIndex) / (n - 1)) : '0');
    }
  }

  // Enter / clicking an arrow: wraps around -- except on a <select data-clamp>
  // (Helligkeit 0-100, ...), where jumping from 100 straight back to 0 would
  // be a shock; there it stops at the end. false = already at that end.
  function cycleSelect(select, dir) {
    const n = select.options.length;
    if (!n) return false;
    let next = select.selectedIndex + dir;
    if ('clamp' in select.dataset) {
      if (next < 0 || next >= n) return false;
    } else {
      next = (next + n) % n;
    }
    select.selectedIndex = next;
    select.dispatchEvent(new Event('change'));
    syncStepperValue(select, dir);
    return true;
  }

  // Stops at the ends (arrow keys); false = already at the edge.
  function stepSelect(row, dir) {
    const select = $(row.dataset.target);
    const next = select.selectedIndex + dir;
    if (next < 0 || next >= select.options.length) return false;
    select.selectedIndex = next;
    select.dispatchEvent(new Event('change'));
    syncStepperValue(select, dir);
    return true;
  }

  function syncControls() {
    const set = (id, value) => {
      const s = $(id);
      s.value = String(value);
      syncStepperValue(s);
    };
    set('artmode-timeout-select', prefs.artmodeTimeout);
    set('remote-sensitivity-select', prefs.remoteSensitivity);
    set('touchpad-speed-select', prefs.touchpadSpeed);
    syncPicture();
    syncToggles();
  }

  // ------------------------------------------------------------------ Bild
  // Model, presets and the filter itself live in picture.js; every change
  // there is saved and applied at once (launcher + content view), so stepping
  // through the values is the live preview.
  const pic = window.picture;
  const PICTURE_SCALES = {
    'picture-brightness-select': 'brightness',
    'picture-contrast-select': 'contrast',
    'picture-saturation-select': 'saturation'
  };
  const pictureModeSelect = $('picture-mode-select');
  const pictureSelects = {
    ...PICTURE_SCALES,
    'picture-temperature-select': 'temperature',
    'picture-sharpness-select': 'sharpness'
  };
  let pictureResetTimer = null;

  // Options come from picture.js, so labels and values have one source.
  (function fillPictureSelects() {
    const opt = (value, label) => `<option value="${esc(value)}">${esc(label)}</option>`;
    const scale = Array.from({ length: 21 }, (_, i) => opt(i * 5, i * 5)).join('');
    Object.keys(PICTURE_SCALES).forEach((id) => { $(id).innerHTML = scale; });
    $('picture-temperature-select').innerHTML = pic.TEMPERATURES.map((t) => opt(t.id, t.label)).join('');
    $('picture-sharpness-select').innerHTML = pic.SHARPNESS.map((s, i) => opt(i, s.label)).join('');
  })();

  // "Benutzerdefiniert" is only offered once there are custom values to go
  // back to. `except` is the <select> the user just stepped: the stepper
  // animates that one itself (syncing it here first would swallow the tick).
  function syncPicture(except) {
    const p = pic.current();
    const rows = [['picture-mode-select', 'mode'], ...Object.entries(pictureSelects)].map(([id, key]) => ({ s: $(id), key }));
    const before = rows.map(({ s }) => s.selectedIndex);
    const modes = [...pic.MODES, ...(pic.hasCustom() ? [pic.CUSTOM] : [])];
    if (pictureModeSelect.options.length !== modes.length) {
      pictureModeSelect.innerHTML = modes.map((m) => `<option value="${esc(m.id)}">${esc(m.label)}</option>`).join('');
    }
    rows.forEach(({ s, key }, i) => {
      if (s === except) return;
      s.value = String(p[key]);
      // Values that change by themselves (a new Bildmodus; the mode turning
      // into "Benutzerdefiniert") tick like a stepped value, in the direction
      // they moved -- visible feedback while the remote is on another row.
      const moved = s.selectedIndex - before[i];
      syncStepperValue(s, except && moved ? Math.sign(moved) : 0);
    });
  }

  pictureModeSelect.addEventListener('change', (e) => {
    pic.setMode(e.target.value);
    syncPicture(e.target);
    updateSummaries();
  });
  for (const [id, key] of Object.entries(pictureSelects)) {
    $(id).addEventListener('change', (e) => {
      pic.setValue(key, key === 'temperature' ? e.target.value : Number(e.target.value));
      syncPicture(e.target);
      updateSummaries();
    });
  }

  function resetPicture() {
    pic.reset();
    syncPicture();
    syncToggles();
    updateSummaries();
    const status = $('picture-reset-status');
    status.textContent = 'Zurückgesetzt';
    status.classList.add('is-ok');
    clearTimeout(pictureResetTimer);
    pictureResetTimer = setTimeout(() => {
      status.textContent = 'Zurücksetzen';
      status.classList.remove('is-ok');
    }, 2500);
  }

  $('artmode-timeout-select').addEventListener('change', (e) => {
    prefs.artmodeTimeout = Number(e.target.value);
    savePrefs(prefs);
    resetIdleTimer(); // apply the new duration immediately
    updateSummaries();
  });
  $('remote-sensitivity-select').addEventListener('change', (e) => {
    prefs.remoteSensitivity = Number(e.target.value);
    savePrefs(prefs);
  });
  $('touchpad-speed-select').addEventListener('change', (e) => {
    prefs.touchpadSpeed = Number(e.target.value);
    savePrefs(prefs);
  });
  $('display-select').addEventListener('change', () => updateSummaries());

  async function loadDisplays() {
    let displays = [];
    try { displays = await window.launcher.getDisplays(); } catch { /* leave the list empty */ }
    const sel = $('display-select');
    sel.innerHTML = displays.map((d) => `<option value="${esc(d.id)}">${esc(d.label)}</option>`).join('');
    syncStepperValue(sel);
    updateSummaries();
  }

  // ----------------------------------------- category summaries (left list)
  const selectedLabel = (id) => {
    const s = $(id);
    return (s && s.options[s.selectedIndex] && s.options[s.selectedIndex].textContent) || '';
  };

  function summaryFor(cat) {
    switch (cat) {
      case 'playback':
        return `Werbeblocker ${prefs.adblock ? 'an' : 'aus'}`;
      case 'display':
        return [prefs.fullscreen ? 'Vollbild' : 'Fenster', selectedLabel('display-select')].filter(Boolean).join(' · ');
      case 'picture':
        return pic.summary(); // "Kino", "Benutzerdefiniert · Augenschonend", ...
      case 'artmode':
        return `Startet nach ${selectedLabel('artmode-timeout-select')}`;
      case 'media':
        return 'Ordner & Bibliothek';
      case 'wifi': {
        const w = state.wifi;
        if (w.busySsid) return `Verbinde mit ${w.busySsid}…`;
        if (w.status) return w.status.connected ? w.status.ssid : 'Nicht verbunden';
        return w.loading ? 'Suche läuft…' : 'Netzwerk & Verbindung';
      }
      case 'bluetooth': {
        const b = state.bt;
        if (b.radioOn === null) return 'Geräte & Kopplung';
        if (!b.radioOn) return 'Aus';
        const n = b.devices.length;
        return n ? `An · ${n} ${n === 1 ? 'Gerät' : 'Geräte'}` : 'An · keine Geräte';
      }
      case 'remote':
        return state.remoteUrl ? state.remoteUrl.replace(/^https?:\/\//, '') : 'Handy als Fernbedienung';
      case 'system': {
        const i = state.sys.info;
        return i ? `Version ${i.version}${i.ip ? ` · ${i.ip}` : ''}` : 'Version, Netzwerk, Bildschirm';
      }
      default:
        return '';
    }
  }

  function updateSummaries() {
    overlay.querySelectorAll('[data-sub]').forEach((el) => setText(el, summaryFor(el.dataset.sub)));
  }

  // ------------------------------------------------------------------ WLAN
  function renderWifi() {
    const w = state.wifi;
    const st = w.status;
    const hero = $('wifi-status');
    const listEl = $('wifi-list');

    // Status card
    let heroState = 'off';
    let title = 'Nicht verbunden';
    let sub = 'Wähle unten ein Netzwerk aus.';
    let iconHtml = icon('wifi');
    let endHtml = '';
    if (w.busySsid) {
      heroState = 'busy'; title = 'Verbinde…'; sub = w.busySsid; iconHtml = '<span class="st-spinner lg"></span>';
    } else if (!st) {
      heroState = 'busy'; title = 'Suche läuft…'; sub = 'Netzwerke werden gesucht'; iconHtml = '<span class="st-spinner lg"></span>';
    } else if (st.connected) {
      const level = st.signal ? signalLevel(st.signal) : 0;
      heroState = 'on'; title = 'Verbunden';
      sub = level ? `${st.ssid} · Signal ${SIGNAL_TEXT[level]}` : st.ssid;
      endHtml = level ? bars(level) : '';
    }
    let subErr = false;
    if (w.message && !w.busySsid) { sub = w.message; subErr = true; if (heroState === 'off') heroState = 'error'; }
    hero.dataset.state = heroState;
    setHtml(hero, `<div class="st-hero-ic">${iconHtml}</div>
      <div class="st-hero-text"><div class="st-hero-title">${esc(title)}</div><div class="st-hero-sub${subErr ? ' err' : ''}">${esc(sub)}</div></div>
      ${endHtml ? `<div class="st-hero-end">${endHtml}</div>` : ''}`);

    $('wifi-busy').hidden = !(w.loading && st && !w.busySsid);
    $('wifi-refresh-ic').classList.toggle('st-spin', w.loading);

    // Network list
    const fresh = !listEl.dataset.filled;
    let rows;
    if (!w.networks.length) {
      rows = (w.loading || !w.at)
        ? [46, 62, 38].map((pct) => `<li class="st-row st-skel"><span class="bar" style="width:26px;height:20px"></span><div class="st-row-main"><span class="bar" style="width:${pct}%"></span></div></li>`).join('')
        : '<li class="st-row st-row-empty">Keine Netzwerke gefunden.</li>';
    } else {
      // The network you are on goes first; data-index still points into the
      // original order, which is what connectToWifiNetwork() indexes.
      const isCur = (n) => !!(st && st.connected && n.ssid === st.ssid);
      const ordered = w.networks.map((n, i) => ({ n, i })).sort((a, b) => Number(isCur(b.n)) - Number(isCur(a.n)));
      rows = ordered.map(({ n, i }, pos) => {
        const cur = isCur(n);
        const busy = w.busySsid === n.ssid;
        const needsPassword = !n.open && !n.hasProfile;
        const level = signalLevel(n.signal);
        let rowSub = '';
        if (busy) rowSub = 'Verbinde…';
        else if (cur) rowSub = `Signal ${SIGNAL_TEXT[level]}`;
        else if (needsPassword) rowSub = 'Passwort erforderlich';
        else if (n.hasProfile) rowSub = 'Gespeichert';
        else if (n.open) rowSub = 'Offenes Netzwerk';
        let end = '';
        if (busy) end = spinnerHtml;
        else if (cur) end = `<span class="st-badge ok">${icon('check')}Verbunden</span>`;
        else if (!n.open) end = icon('lock', 'st-lock');
        return `<li class="st-row settings-nav wifi-item${cur ? ' wifi-current' : ''}${fresh ? ' st-new' : ''}" tabindex="-1" data-kind="wifi-connect" data-index="${i}" data-key="wifi:${esc(n.ssid)}" style="--i:${Math.min(pos, 10)}">
          ${bars(level)}
          <div class="st-row-main"><div class="st-row-label wifi-name">${esc(n.ssid)}</div>${rowSub ? `<div class="st-row-sub">${esc(rowSub)}</div>` : ''}</div>
          <div class="st-row-end">${end}</div>
        </li>`;
      }).join('');
    }
    if (setHtml(listEl, rows)) {
      if (w.networks.length) listEl.dataset.filled = '1'; else delete listEl.dataset.filled;
      if (settingsOpen) rebuildSettingsNavItems();
    }
  }

  async function loadWifi(force) {
    const w = state.wifi;
    if (w.loading || w.busySsid) return;
    if (!force && w.at && Date.now() - w.at < TTL.wifi) return;
    w.loading = true;
    renderWifi();
    updateSummaries();
    try {
      const [status, networks] = await Promise.all([
        window.launcher.getWifiStatus(),
        window.launcher.getWifiNetworks()
      ]);
      w.status = status;
      w.networks = networks || [];
    } catch { /* keep what we had; the empty state explains itself */ }
    w.at = Date.now();
    w.loading = false;
    renderWifi();
    updateSummaries();
  }

  // Saved and open networks connect straight away; a brand-new secured one
  // asks for its password on the on-screen keyboard first.
  async function connectToWifiNetwork(index) {
    const w = state.wifi;
    const network = w.networks[index];
    if (!network || w.busySsid) return;

    let password = null;
    if (!network.open && !network.hasProfile) {
      password = await showKeyboard({ label: `Passwort für ${network.ssid}`, secure: true });
      if (password === null) return; // cancelled
    }

    w.message = '';
    w.busySsid = network.ssid;
    renderWifi();
    updateSummaries();
    let result;
    try {
      result = password === null
        ? await window.launcher.connectWifi(network.ssid)
        : await window.launcher.connectWifiWithPassword(network.ssid, password);
    } catch (err) {
      result = { ok: false, error: String((err && err.message) || err) };
    }
    w.busySsid = null;
    if (!result.ok) {
      // netsh errors are long "Command failed: ..." blobs; the first line is enough.
      w.message = `Verbindung fehlgeschlagen: ${String(result.error || 'Unbekannter Fehler').split(/\r?\n/)[0].slice(0, 140)}`;
      renderWifi();
      updateSummaries();
      return;
    }
    await loadWifi(true);
  }

  // ------------------------------------------------------------- Bluetooth
  function renderBluetooth() {
    const b = state.bt;
    const hero = $('bluetooth-status');

    let heroState = 'busy';
    let title = 'Bluetooth wird geprüft…';
    let sub = 'Einen Moment';
    let iconHtml = '<span class="st-spinner lg"></span>';
    let endHtml = '';
    if (b.radioOn !== null) {
      iconHtml = icon('bluetooth');
      const n = b.devices.length;
      if (b.radioOn) {
        heroState = 'on'; title = 'Bluetooth ist an';
        sub = n ? `${n} gekoppelte${n === 1 ? 's Gerät' : ' Geräte'}` : 'Noch keine Geräte gekoppelt';
        endHtml = '<span class="st-badge ok">An</span>';
      } else {
        heroState = 'off'; title = 'Bluetooth ist aus';
        sub = 'Über „Erweiterte Kopplung“ lässt es sich einschalten.';
        endHtml = '<span class="st-badge">Aus</span>';
      }
    }
    hero.dataset.state = heroState;
    setHtml(hero, `<div class="st-hero-ic">${iconHtml}</div>
      <div class="st-hero-text"><div class="st-hero-title">${esc(title)}</div><div class="st-hero-sub">${esc(sub)}</div></div>
      ${endHtml ? `<div class="st-hero-end">${endHtml}</div>` : ''}`);

    $('bluetooth-busy').hidden = !(b.loading && b.radioOn !== null);

    // Paired devices (read-only: there is no API to unpair or disconnect)
    let paired;
    if (!b.devices.length) {
      paired = (b.loading || !b.at)
        ? [52, 34].map((pct) => `<li class="st-row st-skel"><span class="bar" style="width:42px;height:42px;border-radius:50%"></span><div class="st-row-main"><span class="bar" style="width:${pct}%"></span></div></li>`).join('')
        : '<li class="st-row st-row-empty">Keine gekoppelten Geräte.</li>';
    } else {
      paired = b.devices.map((d) => `<li class="st-row st-static bt-item${d.connected ? ' bt-connected' : ''}">
        <span class="st-dev-ic">${icon(btDeviceIcon(d.name))}</span>
        <div class="st-row-main"><div class="st-row-label bt-name">${esc(d.name)}</div></div>
        <div class="st-row-end">${d.connected ? `<span class="st-badge ok">${icon('check')}Verbunden</span>` : '<span class="st-badge">Gekoppelt</span>'}</div>
      </li>`).join('');
    }
    const changedPaired = setHtml($('bluetooth-list'), paired);

    // Scan row + results
    setText($('bluetooth-scan-hint'), b.scanning ? 'Suche läuft…' : 'Suchen');
    $('bluetooth-scan-spinner').hidden = !b.scanning;
    const scanList = $('bluetooth-scan-list');
    scanList.hidden = !b.scanned;
    let found = '';
    if (b.scanned) {
      if (b.scanError) {
        found = `<li class="st-row st-row-empty">Suche fehlgeschlagen: ${esc(String(b.scanError).split(/\r?\n/)[0].slice(0, 140))}</li>`;
      } else if (!b.found.length) {
        found = '<li class="st-row st-row-empty">Keine Geräte in der Nähe gefunden.</li>';
      } else {
        found = b.found.map((d, i) => {
          let end = '<span class="st-badge accent">Koppeln</span>';
          if (b.pairingId === d.id) end = `${spinnerHtml}<span class="st-badge busy">Koppelt…</span>`;
          else if (b.failedId === d.id) end = '<span class="st-badge err">Fehlgeschlagen</span>';
          return `<li class="st-row settings-nav bt-item" tabindex="-1" data-kind="bluetooth-pair" data-index="${i}" data-key="btf:${esc(d.id)}">
            <span class="st-dev-ic">${icon(btDeviceIcon(d.name))}</span>
            <div class="st-row-main"><div class="st-row-label bt-name">${esc(d.name)}</div></div>
            <div class="st-row-end">${end}</div>
          </li>`;
        }).join('');
      }
    }
    const changedFound = setHtml(scanList, found);
    if ((changedPaired || changedFound) && settingsOpen) rebuildSettingsNavItems();
  }

  async function loadBluetooth(force) {
    const b = state.bt;
    if (b.loading) return;
    if (!force && b.at && Date.now() - b.at < TTL.bluetooth) return;
    b.loading = true;
    renderBluetooth();
    try {
      const r = await window.launcher.getBluetoothStatus();
      b.radioOn = !!r.radioOn;
      b.devices = r.devices || [];
    } catch {
      if (b.radioOn === null) b.radioOn = false;
    }
    b.at = Date.now();
    b.loading = false;
    renderBluetooth();
    updateSummaries();
  }

  async function scanForBluetoothDevices() {
    const b = state.bt;
    if (b.scanning || b.pairingId) return;
    b.scanning = true;
    b.scanned = false;
    b.found = [];
    b.scanError = '';
    b.failedId = null;
    renderBluetooth();
    let result;
    try {
      result = await window.launcher.scanBluetoothDevices(8);
    } catch (err) {
      result = { devices: [], error: String((err && err.message) || err) };
    }
    b.scanning = false;
    b.scanned = true;
    b.found = result.devices || [];
    b.scanError = result.error || '';
    renderBluetooth();
  }

  async function pairScannedDevice(index) {
    const b = state.bt;
    const device = b.found[index];
    if (!device || b.pairingId) return;
    b.pairingId = device.id;
    b.failedId = null;
    renderBluetooth();
    let result;
    try {
      result = await window.launcher.pairBluetoothDevice(device.id);
    } catch {
      result = { ok: false };
    }
    b.pairingId = null;
    if (result && result.ok) {
      b.found = b.found.filter((d) => d.id !== device.id);
      if (!b.found.length) b.scanned = false;
      renderBluetooth();
      await loadBluetooth(true);
    } else {
      b.failedId = device.id;
      renderBluetooth();
    }
  }

  // --------------------------------------------- Fernbedienung, Mediathek
  async function refreshRemoteInfo() {
    let url = null;
    try { ({ url } = await window.launcher.getRemoteInfo()); } catch { /* shown as "no network" */ }
    state.remoteUrl = url || null;
    state.remoteAt = Date.now();
    const el = $('remote-url');
    if (url) {
      el.classList.remove('none');
      const m = /^(https?:\/\/)(.*)$/.exec(url);
      setHtml(el, m ? `<span class="dim">${esc(m[1])}</span>${esc(m[2])}` : esc(url));
    } else {
      el.classList.add('none');
      setHtml(el, 'Kein Netzwerk – erst WLAN verbinden');
    }
    updateSummaries();
  }

  async function runScaffoldMedia() {
    const status = $('scaffold-media-status');
    let folders = [];
    try { folders = await window.launcher.scaffoldMediaFolders(); } catch { /* status below just says 0 */ }
    status.textContent = `Erstellt (${folders.length})`;
    status.classList.add('is-ok');
    clearTimeout(scaffoldTimer);
    scaffoldTimer = setTimeout(() => {
      status.textContent = 'Anlegen';
      status.classList.remove('is-ok');
    }, 2500);
  }

  // ---------------------------------------------------------- Medienserver
  async function refreshMediaServer() {
    const status = $('media-server-status');
    const sub = $('media-server-sub');
    let s = null;
    try { s = await window.launcher.getMediaServer(); } catch { /* shown as not set up */ }
    status.classList.remove('is-ok');
    if (s && s.ok) {
      status.textContent = `${s.films} ${s.films === 1 ? 'Film' : 'Filme'}`;
      status.classList.add('is-ok');
      sub.textContent = `${s.name} · ${s.url.replace(/^https?:\/\//, '')}`;
    } else if (s && s.url) {
      status.textContent = 'Nicht erreichbar';
      sub.textContent = `${s.url.replace(/^https?:\/\//, '')} · PC an und Server gestartet?`;
    } else {
      status.textContent = 'Einrichten';
      sub.textContent = 'Filme von einem PC im WLAN streamen (server/ANLEITUNG.md)';
    }
  }

  async function runMediaServerSetup() {
    let current = '';
    try { current = ((await window.launcher.getMediaServer()) || {}).url || ''; } catch { /* empty */ }
    const input = await showKeyboard({
      label: 'Adresse des Medienservers (z. B. 192.168.0.21) – leer lassen zum Entfernen',
      initialValue: current.replace(/^http:\/\//, '').replace(/:8420$/, '')
    });
    if (input === null) return; // cancelled
    const status = $('media-server-status');
    status.classList.remove('is-ok');
    status.textContent = 'Prüfe …';
    let res = null;
    try { res = await window.launcher.setMediaServer(input); } catch { /* handled below */ }
    if (res && !res.ok) {
      status.textContent = 'Nicht erreichbar';
      $('media-server-sub').textContent = `${String(input).trim()} · ${res.error || 'Server antwortet nicht'}`;
      return;
    }
    refreshMediaServer();
    if (typeof refreshRandomMovieCell === 'function') refreshRandomMovieCell(); // the cover tile may now pick a server film
  }

  // ---------------------------------------------------------- Musik-Server
  async function refreshMusicServer() {
    const status = $('music-server-status');
    const sub = $('music-server-sub');
    let s = null;
    try { s = await window.launcher.getMusicServer(); } catch { /* shown as not set up */ }
    status.classList.remove('is-ok');
    if (s && s.ok) {
      status.textContent = `${s.tracks} Titel`;
      status.classList.add('is-ok');
      sub.textContent = `${s.name} · ${s.url.replace(/^https?:\/\//, '')}${s.auto ? ' · automatisch gefunden' : ''}`;
    } else if (s && s.url) {
      status.textContent = 'Nicht erreichbar';
      sub.textContent = `${s.url.replace(/^https?:\/\//, '')} · PC an und Musik-Server gestartet?`;
    } else {
      status.textContent = 'Einrichten';
      sub.textContent = 'Musik vom PC streamen (server/ANLEITUNG.md)';
    }
  }

  async function runMusicServerSetup() {
    let current = '';
    try { current = ((await window.launcher.getMusicServer()) || {}).url || ''; } catch { /* empty */ }
    const input = await showKeyboard({
      label: 'Adresse des Musik-Servers (z. B. 192.168.0.21) – leer lassen zum Entfernen',
      initialValue: current.replace(/^http:\/\//, '').replace(/:8421$/, '')
    });
    if (input === null) return; // cancelled
    const status = $('music-server-status');
    status.classList.remove('is-ok');
    status.textContent = 'Prüfe …';
    let res = null;
    try { res = await window.launcher.setMusicServer(input); } catch { /* handled below */ }
    if (res && !res.ok) {
      status.textContent = 'Nicht erreichbar';
      $('music-server-sub').textContent = `${String(input).trim()} · ${res.error || 'Server antwortet nicht'}`;
      return;
    }
    refreshMusicServer();
    if (window.music) window.music.reload(); // the player picks the new library up right away
  }

  // ---------------------------------------------------------------- System
  function renderSystem() {
    const i = state.sys.info;
    if (!i) return;
    setText($('sys-version-hero'), `Version ${i.version}`);
    setText($('sys-host'), i.hostname || '–');
    setText($('sys-ip'), i.ip || 'Nicht verbunden');
    const d = i.display;
    if (d) {
      const sf = d.scaleFactor || 1;
      setText($('sys-display'), `${Math.round(d.width * sf)} × ${Math.round(d.height * sf)}${sf !== 1 ? ` · Skalierung ${Math.round(sf * 100)} %` : ''}`);
    }
    setText($('sys-cpu'), i.cpu || '–');
    setText($('sys-mem'), `${fmtGb(i.memFree)} GB frei von ${fmtGb(i.memTotal)} GB`);
    renderUptime();
    setText($('sys-version'), i.version);
    setText($('sys-electron'), i.electron);
    setText($('sys-chrome'), i.chrome);
  }

  function renderUptime() {
    if (state.sys.bootMs) setText($('sys-uptime'), fmtUptime((Date.now() - state.sys.bootMs) / 1000));
  }

  async function loadSystem() {
    let info;
    try { info = await window.launcher.getSystemInfo(); } catch { return; }
    if (!info) return;
    state.sys.info = info;
    state.sys.at = Date.now();
    state.sys.bootMs = Date.now() - info.uptimeSec * 1000;
    renderSystem();
    updateSummaries();
  }

  // The uptime is the only live value; it only ticks while it is on screen.
  function updateUptimeTimer() {
    const want = settingsOpen && activeCategory === 'system';
    if (want && !uptimeTimer) {
      renderUptime();
      uptimeTimer = setInterval(renderUptime, 20000);
    } else if (!want && uptimeTimer) {
      clearInterval(uptimeTimer);
      uptimeTimer = null;
    }
  }

  // ------------------------------------------------------- loading schedule
  function ensureLoaded(cat) {
    if (!settingsOpen || activeCategory !== cat) return;
    if (cat === 'wifi') loadWifi(false);
    else if (cat === 'media') { refreshMediaServer(); refreshMusicServer(); }
    else if (cat === 'bluetooth') loadBluetooth(false);
    else if (cat === 'remote') { if (Date.now() - state.remoteAt > TTL.remote) refreshRemoteInfo(); }
    else if (cat === 'system') { if (Date.now() - state.sys.at > TTL.system) loadSystem(); }
  }

  function scheduleLoad(cat, delay) {
    clearTimeout(loadTimer);
    if (delay > 0) loadTimer = setTimeout(() => ensureLoaded(cat), delay);
    else ensureLoaded(cat);
  }

  // Fills the one-line summaries in the category list without waiting for a
  // visit. The Bluetooth query spawns PowerShell, so it waits until the open
  // animation is over.
  function prefetchSummaries() {
    window.launcher.getWifiStatus().then((s) => {
      if (state.wifi.status) return;
      state.wifi.status = s;
      renderWifi();
      updateSummaries();
    }).catch(() => {});
    if (Date.now() - state.remoteAt > TTL.remote) refreshRemoteInfo();
    if (Date.now() - state.sys.at > TTL.system) loadSystem();
    clearTimeout(btPrefetchTimer);
    if (!state.bt.at) btPrefetchTimer = setTimeout(() => { if (settingsOpen) loadBluetooth(false); }, 1600);
  }

  // ------------------------------------------------------ activate / open / close
  function activateSettingsRow(row) {
    if (!row) return;
    switch (row.dataset.kind) {
      case 'toggle': applyToggle(row.dataset.target); break;
      case 'select': if (!cycleSelect($(row.dataset.target), 1)) bump(row, 6, 0); break;
      case 'close': closeSettings(); break;
      case 'category': enterDetail(); break;
      case 'action':
        if (row.dataset.target === 'scaffold-media') runScaffoldMedia();
        else if (row.dataset.target === 'media-server') runMediaServerSetup();
        else if (row.dataset.target === 'music-server') runMusicServerSetup();
        else if (row.dataset.target === 'picture-reset') resetPicture();
        break;
      case 'wifi-connect': connectToWifiNetwork(Number(row.dataset.index)); break;
      case 'wifi-refresh': state.wifi.message = ''; loadWifi(true); break;
      case 'bluetooth-scan': scanForBluetoothDevices(); break;
      case 'bluetooth-pair': pairScannedDevice(Number(row.dataset.index)); break;
      case 'bluetooth-settings': window.launcher.openBluetoothSettings(); break;
      // 'info' rows are read-only.
    }
  }

  // Mouse / phone-pointer clicks. Arrow buttons step their stepper; anything
  // else focuses and activates its row. (The phone pointer's own remoteClick()
  // routes rows through setSettingsFocus + activateSettingsRow directly and
  // only falls back to a DOM click for arrows and empty space.)
  overlay.addEventListener('click', (e) => {
    const arrow = e.target.closest('.stepper-arrow');
    if (arrow) {
      const row = arrow.closest('.settings-nav');
      const i = settingsNavItems.indexOf(row);
      if (i >= 0) setSettingsFocus(i);
      const dir = Number(arrow.dataset.dir);
      if (!cycleSelect($(arrow.dataset.select), dir)) bump(row, dir * 6, 0);
      return;
    }
    const row = e.target.closest('.settings-nav');
    const i = row ? settingsNavItems.indexOf(row) : -1;
    if (i < 0) return;
    setSettingsFocus(i);
    activateSettingsRow(row);
  });

  // The overlay's own entrance/exit (a zoom out of the gear button) is done
  // by motion.js around these two helpers -- nothing here animates the
  // overlay itself.
  function openSettings(category) {
    settingsOpen = true;
    settingsNavItems = [];
    settingsFocusIndex = 0;
    activeCategory = null;
    pillSnap = true;
    state.wifi.message = ''; // a stale "Verbindung fehlgeschlagen" shouldn't greet you on the next visit
    overlay.classList.add('st-opening'); // delays the inner entrance until the zoom has landed
    clearTimeout(openingTimer);
    openingTimer = setTimeout(() => overlay.classList.remove('st-opening'), 800);
    animatedOpen(overlay);
    syncControls();
    setSettingsCategory(category);
    updateSummaries();
    loadDisplays();
    prefetchSummaries();
  }

  function closeSettings() {
    settingsOpen = false;
    clearTimeout(loadTimer);
    clearTimeout(btPrefetchTimer);
    updateUptimeTimer();
    animatedClose(overlay, { duration: 200 });
  }

  $('btn-settings').addEventListener('click', () => openSettings());
  $('btn-wifi').addEventListener('click', () => openSettings('wifi'));

  // Initial (empty) states so the panels never flash raw placeholders.
  renderWifi();
  renderBluetooth();

  Object.assign(window, {
    openSettings,
    closeSettings,
    handleSettingsKey,
    setSettingsFocus,
    activateSettingsRow,
    setSettingsCategory,
    rebuildSettingsNavItems,
    syncStepperValue,
    cycleSelect,
    refreshRemoteInfo,
    refreshWifi: () => loadWifi(true),
    refreshBluetooth: () => loadBluetooth(true),
    connectToWifiNetwork,
    scanForBluetoothDevices,
    pairScannedDevice
  });
})();
