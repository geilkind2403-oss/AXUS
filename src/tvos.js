// --- TV-OS system layer: everything that makes the launcher behave like a
// real TV operating system rather than an app window. Loaded after renderer.js,
// settings.js, weather.js and appfolder.js, before motion.js. Main-process half:
// tvos-main.js (reached through window.tvosBridge, see preload.js).
//
//   1. Toasts        window.tvos.toast({ icon, title, text, kind, duration, id })
//   2. Network       offline pill + red WLAN button; window.tvos.network
//   3. Web apps      brand splash while a web app loads, friendly error screen
//                    when it can't (instead of Chrome's error page)
//   4. Home          Home key / long Escape / phone "Home" close everything
//   5. Power menu    Alt+F4 or the power button: Bildschirm aus / restart / quit
//   6. Standby       "Bildschirm aus": pure black until a key is pressed
//   7. Hardening     no drag ghosts, zoom, Tab focus hopping, native dialogs...
//
// The renderer's own key routing (renderer.js) knows nothing of this file: it
// listens on `window` in the capture phase, so it sees every key first and can
// swallow it while one of its modals is open.
(() => {
  'use strict';

  const bridge = window.tvosBridge || null; // null in a plain browser: everything degrades quietly
  const $ = (id) => document.getElementById(id);
  // Guarded access to renderer.js / settings.js globals: a renamed or missing
  // one must never take the whole system layer down.
  const g = (fn) => { try { return fn(); } catch (err) { return undefined; } };
  const swallow = (e) => { e.preventDefault(); e.stopImmediatePropagation(); };
  const ce = (tag, cls) => {
    const node = document.createElement(tag);
    if (cls) node.className = cls;
    return node;
  };

  // Tiny event emitter for other scripts: tvos.on('home' | 'network' | 'content-state', cb).
  const listeners = {};
  const on = (event, cb) => { (listeners[event] = listeners[event] || []).push(cb); };
  const emit = (event, payload) => (listeners[event] || []).forEach((cb) => g(() => cb(payload)));

  // ------------------------------------------------------------------ icons
  const ICONS = {
    wifi: '<path d="M2 8.5a16 16 0 0 1 20 0M5.5 12a11 11 0 0 1 13 0M9 15.5a6 6 0 0 1 6 0M12 19h.01"/>',
    'wifi-off': '<path d="M2 8.5a16 16 0 0 1 20 0M5.5 12a11 11 0 0 1 13 0M9 15.5a6 6 0 0 1 6 0M12 19h.01M3 3l18 18"/>',
    phone: '<path d="M9.5 2.5h5a2 2 0 0 1 2 2v15a2 2 0 0 1-2 2h-5a2 2 0 0 1-2-2v-15a2 2 0 0 1 2-2zM11 18.5h2"/>',
    bluetooth: '<path d="M6.5 7.5l11 9-5.5 5V2l5.5 5-11 9"/>',
    check: '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
    info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v5.5M12 7.6h.01"/>',
    warning: '<path d="M12 3.5l9.5 17h-19zM12 10v4.5M12 17.5h.01"/>',
    power: '<path d="M12 3v8M6.3 6.6a8 8 0 1 0 11.4 0"/>',
    tv: '<rect x="3" y="4.5" width="18" height="12" rx="2.5"/><path d="M8 20h8M12 16.5V20"/>',
    refresh: '<path d="M20 12a8 8 0 1 1-2.6-5.9M20 4.5V9h-4.5"/>',
    exit: '<path d="M9 4H5.5A1.5 1.5 0 0 0 4 5.5v13A1.5 1.5 0 0 0 5.5 20H9M15 8l4 4-4 4M19 12H9"/>',
    globe: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.6 2.6 3.8 5.6 3.8 9s-1.2 6.4-3.8 9c-2.6-2.6-3.8-5.6-3.8-9S9.4 5.6 12 3z"/>',
    play: '<rect x="3" y="4.5" width="18" height="15" rx="4"/><path d="M10 9l5 3-5 3z"/>'
  };
  const svg = (name) => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[name] || ''}</svg>`;

  // ------------------------------------------------------------------ 1. toasts
  const MAX_TOASTS = 3;
  const toastHost = ce('div');
  toastHost.id = 'tvos-toasts';
  toastHost.setAttribute('aria-live', 'polite');
  document.body.appendChild(toastHost);
  const toasts = new Map(); // id -> handle
  let toastSeq = 0;

  // opts: { id, icon: 'wifi'|'wifi-off'|'phone'|'bluetooth'|'check'|'info'|'warning' | any short text/emoji,
  //         kind: 'info'|'success'|'warning'|'error', title, text, duration (ms, default 4500), sticky, force }
  // A toast with an id that is already showing replaces it ("WLAN verbunden"
  // takes the place of "Kein Internet"). Returns { dismiss, update } or null.
  function toast(opts) {
    if (typeof opts === 'string') opts = { title: opts };
    opts = opts || {};
    if (!opts.title && !opts.text) return null;
    if (!opts.force && g(() => prefs.notifications) === false) return null;

    const id = opts.id || `t${++toastSeq}`;
    const previous = toasts.get(id);
    if (previous) previous.remove(true);
    while (toastHost.children.length >= MAX_TOASTS) {
      const oldest = toasts.get(toastHost.firstElementChild.dataset.toastId);
      if (oldest) oldest.remove(true); else toastHost.firstElementChild.remove();
    }

    const card = ce('div', `tvos-toast tvos-toast-${['info', 'success', 'warning', 'error'].includes(opts.kind) ? opts.kind : 'info'}`);
    card.setAttribute('role', 'status');
    card.dataset.toastId = id;
    const icon = ce('div', 'tvos-toast-icon');
    const body = ce('div', 'tvos-toast-body');
    const title = ce('div', 'tvos-toast-title');
    const text = ce('div', 'tvos-toast-text');
    body.append(title, text);
    card.append(icon, body);

    let timer = null;
    let leaving = false;
    const handle = {
      // instant = skip the exit animation (replacement / overflow)
      remove(instant) {
        if (leaving) return;
        leaving = true;
        clearTimeout(timer);
        if (toasts.get(id) === handle) toasts.delete(id);
        if (instant) { card.remove(); return; }
        card.classList.add('out');
        card.addEventListener('animationend', () => card.remove(), { once: true });
        setTimeout(() => card.remove(), 400); // in case the animation never runs
      },
      dismiss() { handle.remove(false); },
      update(next) {
        if (next.icon !== undefined) {
          if (ICONS[next.icon]) icon.innerHTML = svg(next.icon); else icon.textContent = next.icon ? String(next.icon) : '';
        }
        if (next.title !== undefined) title.textContent = String(next.title || '');
        if (next.text !== undefined) { text.textContent = String(next.text || ''); text.style.display = next.text ? '' : 'none'; }
      }
    };
    handle.update({ icon: opts.icon, title: opts.title || '', text: opts.text || '' });
    toastHost.appendChild(card);
    toasts.set(id, handle);
    if (!opts.sticky) timer = setTimeout(handle.dismiss, Math.max(1500, opts.duration || 4500));
    return handle;
  }
  const dismissToast = (id) => { const h = toasts.get(id); if (h) h.dismiss(); };

  // ------------------------------------------------------------------ 2. network
  const net = { online: true, known: false };
  const netPill = ce('div');
  netPill.id = 'tvos-net';
  netPill.innerHTML = `${svg('wifi-off')}<span>Offline</span>`;
  document.body.appendChild(netPill);

  function applyNetwork(state) {
    if (!state) return;
    net.online = state.online !== false;
    net.known = true;
    document.body.classList.toggle('tvos-offline', !net.online);
    emit('network', { online: net.online });
  }

  // ------------------------------------------------------------------ 3. web apps: launch splash + error screen
  const BRAND_BG = 'radial-gradient(120% 90% at 50% -10%, #1b1b20 0%, #151517 45%, #0a0a0c 100%)';
  const BRANDS = [
    {
      re: /(^|[./])youtube\.com|youtu\.be/i, word: 'YouTube', accent: '#ff3b3b',
      icon: '<svg viewBox="0 0 24 24"><rect x="1.5" y="4.5" width="21" height="15" rx="4.5" fill="#ff0033"/><path d="M10 8.8l5.2 3.2-5.2 3.2z" fill="#fff"/></svg>',
      bg: 'radial-gradient(120% 90% at 50% -10%, #3a1116 0%, #170c0e 52%, #0a0a0c 100%)'
    },
    {
      re: /netflix\.com/i, word: 'NETFLIX', wordColor: '#e50914', spaced: true, accent: '#e50914',
      bg: 'radial-gradient(120% 90% at 50% -10%, #2c0a0e 0%, #0e0708 52%, #050505 100%)'
    },
    {
      re: /primevideo\.com|amazon\.[a-z.]+/i, word: 'prime video', accent: '#00a8e1',
      bg: 'linear-gradient(140deg, #0a1626 0%, #0a3a56 100%)'
    },
    {
      re: /disneyplus\.com/i, word: 'Disney+', accent: '#9db8ff',
      bg: 'linear-gradient(140deg, #0a1848 0%, #1b3fa8 100%)'
    },
    {
      re: /tagesschau\.de/i, word: 'tagesschau', accent: '#6aa9ff',
      bg: 'linear-gradient(140deg, #0a1a33 0%, #123a70 100%)'
    },
    { re: /google\./i, word: 'Browser', accent: '#8ab4ff', icon: svg('globe'), bg: BRAND_BG },
    {
      file: true, re: /\/livetv\.html/i, word: 'Live TV', accent: '#8ab4ff', icon: svg('tv'),
      bg: 'linear-gradient(140deg, #14171f 0%, #2b2f3a 100%)'
    },
    {
      file: true, re: /\/mediaplayer\.html/i, word: 'Mediathek', accent: '#b6a3ff', icon: svg('play'),
      bg: 'linear-gradient(140deg, #1b1638 0%, #3a2f6e 100%)'
    }
  ];

  // { word, icon?, bg, accent, ... } for a URL: brand colours + wordmark. The
  // same table can drive a launch transition (window.tvos.brandFor).
  function brandFor(url) {
    const href = String(url || '');
    const isFile = /^file:/i.test(href);
    const found = BRANDS.find((b) => !!b.file === isFile && b.re.test(href));
    if (found) return found;
    let host = '';
    try { host = new URL(href).hostname.replace(/^www\./, ''); } catch (err) { /* keep empty */ }
    return { word: host, accent: '#ffffff', bg: BRAND_BG };
  }

  const splash = ce('div');
  splash.id = 'tvos-splash';
  splash.setAttribute('role', 'status');
  splash.innerHTML = `
    <div class="tvos-splash-center">
      <div id="tvos-brand"></div>
      <div class="tvos-spinner"></div>
      <div id="tvos-splash-slow">Das dauert länger als gewöhnlich …</div>
      <div class="tvos-err">
        <div class="tvos-err-icon" id="tvos-err-icon"></div>
        <div class="tvos-err-title" id="tvos-err-title"></div>
        <div class="tvos-err-text" id="tvos-err-text"></div>
        <div class="tvos-err-code" id="tvos-err-code"></div>
        <div class="tvos-err-actions">
          <button class="tvos-btn primary" type="button" tabindex="-1" data-act="retry">Erneut versuchen</button>
          <button class="tvos-btn" type="button" tabindex="-1" data-act="home">Zur Startseite</button>
        </div>
      </div>
    </div>`;
  document.body.appendChild(splash);
  const brandEl = $('tvos-brand');
  const slowEl = $('tvos-splash-slow');
  const errButtons = Array.from(splash.querySelectorAll('.tvos-btn'));
  let splashMode = 'off'; // 'off' | 'loading' | 'error'
  let showTimer = null;
  let slowTimer = null;
  let errFocus = 0;
  let contentState = 'idle';

  function applyBrand(brand) {
    brandEl.replaceChildren();
    if (brand.icon) {
      const icon = ce('span', 'tvos-brand-icon');
      icon.innerHTML = brand.icon; // static markup from the table above, never page/network data
      brandEl.appendChild(icon);
    }
    if (brand.word) {
      const word = ce('span', 'tvos-brand-word');
      word.textContent = brand.word;
      word.style.color = brand.wordColor || '';
      word.style.letterSpacing = brand.spaced ? '3px' : '';
      brandEl.appendChild(word);
    }
    splash.style.background = brand.bg || BRAND_BG;
    splash.style.setProperty('--tvos-accent', brand.accent || '#fff');
  }

  function hideSplash() {
    clearTimeout(showTimer);
    clearTimeout(slowTimer);
    splashMode = 'off';
    splash.classList.remove('on', 'error');
    slowEl.classList.remove('show');
  }

  // A page that is ready within a moment never shows the splash at all; local
  // pages (player) get longer, they are up almost instantly.
  function showLoading(url) {
    clearTimeout(showTimer);
    clearTimeout(slowTimer);
    applyBrand(brandFor(url));
    splashMode = 'loading';
    splash.classList.remove('error');
    slowEl.classList.remove('show');
    if (!splash.classList.contains('on')) {
      showTimer = setTimeout(() => { if (splashMode === 'loading') splash.classList.add('on'); }, /^file:/i.test(url) ? 700 : 300);
    }
    slowTimer = setTimeout(() => slowEl.classList.add('show'), 9000);
  }

  function describeError(s, brand) {
    const code = s.code;
    const label = typeof code === 'number' ? `Fehlercode ${code}` : String(code || '');
    if (code === 'crashed') {
      return { icon: 'warning', label, title: `${brand.word || 'Die App'} wurde beendet`, text: 'Die Seite ist unerwartet abgestürzt. Du kannst sie neu starten.' };
    }
    if (s.online === false || net.online === false || code === -106) {
      return { icon: 'wifi-off', label, title: 'Keine Internetverbindung', text: 'Verbinde das Gerät mit dem WLAN. Sobald das Internet wieder da ist, geht es automatisch weiter.' };
    }
    if (typeof code === 'number' && code <= -200 && code > -300) {
      return { icon: 'warning', label, title: 'Sichere Verbindung nicht möglich', text: 'Bitte prüfe, ob Datum und Uhrzeit dieses Geräts stimmen.' };
    }
    return { icon: 'warning', label, title: `${brand.word || 'Die Seite'} ist gerade nicht erreichbar`, text: 'Der Dienst antwortet nicht. Versuche es gleich noch einmal.' };
  }

  function setErrFocus(i) {
    errFocus = (i + errButtons.length) % errButtons.length;
    errButtons.forEach((b, k) => b.classList.toggle('focused', k === errFocus));
  }

  function showError(s) {
    clearTimeout(showTimer);
    clearTimeout(slowTimer);
    const brand = brandFor(s.url);
    const info = describeError(s, brand);
    applyBrand(brand);
    $('tvos-err-icon').innerHTML = svg(info.icon);
    $('tvos-err-title').textContent = info.title;
    $('tvos-err-text').textContent = info.text;
    $('tvos-err-code').textContent = info.label;
    splashMode = 'error';
    splash.classList.add('on', 'error');
    setErrFocus(0);
  }

  function activateErrorButton(button) {
    if (button.dataset.act === 'retry') {
      if (bridge) bridge.retryContent().catch(() => {});
    } else {
      goHome();
    }
  }
  errButtons.forEach((b, i) => {
    b.addEventListener('mouseenter', () => setErrFocus(i));
    b.addEventListener('click', () => activateErrorButton(b));
  });

  function onContentState(s) {
    if (!s) return;
    contentState = s.state;
    if (s.state === 'loading') showLoading(s.url);
    else if (s.state === 'ready') hideSplash();
    else if (s.state === 'error') showError(s);
    emit('content-state', s);
  }

  // ------------------------------------------------------------------ 4. home
  // Every overlay closes through its own Escape handling (so animations and
  // state stay theirs); a few rounds, because some screens (weather, settings
  // sub-pages) step back one level per press.
  const baseAnyOverlayOpen = window.anyOverlayOpen;
  function closeOverlays() {
    if (typeof baseAnyOverlayOpen !== 'function') return;
    for (let i = 0; i < 8 && g(() => baseAnyOverlayOpen()); i++) {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    }
  }

  function goHome() {
    closePower(true);
    hideSplash();
    g(() => {
      if (contentOpen) {
        window.launcher.closeContent();
        contentOpen = false;
      }
    });
    closeOverlays();
    g(() => { if (artModeActive) exitArtMode(); });
    g(() => resetIdleTimer());
    emit('home');
  }

  // ------------------------------------------------------------------ 5. power menu + 6. standby
  const power = ce('div');
  power.id = 'tvos-power';
  power.innerHTML = `
    <div class="tvos-power-panel" role="dialog" aria-label="Ein/Aus">
      <div class="tvos-power-title">Ein/Aus</div>
      <div class="tvos-power-row">
        <button class="tvos-power-btn" type="button" tabindex="-1" data-act="screen-off">${svg('tv')}<span>Bildschirm aus</span></button>
        <button class="tvos-power-btn" type="button" tabindex="-1" data-act="restart">${svg('refresh')}<span>Launcher<br>neu starten</span></button>
        <button class="tvos-power-btn subtle" type="button" tabindex="-1" data-act="quit">${svg('exit')}<span>Launcher<br>beenden</span></button>
      </div>
      <div class="tvos-power-hint">←→ navigieren · Enter bestätigen · Esc abbrechen</div>
    </div>`;
  document.body.appendChild(power);
  const powerButtons = Array.from(power.querySelectorAll('.tvos-power-btn'));
  let powerOpen = false;
  let powerFocus = 0;

  function setPowerFocus(i) {
    powerFocus = (i + powerButtons.length) % powerButtons.length;
    powerButtons.forEach((b, k) => b.classList.toggle('focused', k === powerFocus));
  }

  function openPower() {
    if (powerOpen || standbyOn) return;
    g(() => { if (artModeActive) resetIdleTimer(); }); // wakes the screensaver
    hideSplash();
    closeOverlays();
    powerOpen = true;
    setPowerFocus(0);
    power.classList.add('on');
  }
  function closePower() {
    if (!powerOpen) return;
    powerOpen = false;
    power.classList.remove('on');
  }

  function activatePower(button) {
    const act = button.dataset.act;
    closePower();
    if (act === 'screen-off') enterStandby();
    else if (act === 'restart') { if (bridge) bridge.restart().catch(() => {}); }
    else if (act === 'quit') { if (bridge) bridge.quit().catch(() => {}); }
  }
  powerButtons.forEach((b, i) => {
    b.addEventListener('click', () => activatePower(b));
    b.addEventListener('mouseenter', () => setPowerFocus(i));
  });
  power.addEventListener('click', (e) => { if (e.target === power) closePower(); });

  function powerKey(e) {
    swallow(e);
    switch (e.key) {
      case 'ArrowLeft': case 'ArrowUp': setPowerFocus(powerFocus - 1); break;
      case 'ArrowRight': case 'ArrowDown': setPowerFocus(powerFocus + 1); break;
      case 'Enter': if (!e.repeat) activatePower(powerButtons[powerFocus]); break;
      case 'Escape': case 'Backspace': closePower(); break;
    }
  }

  // The power button next to the settings gear (a phone remote has no keyboard,
  // so the menu needs an on-screen way in).
  (() => {
    const settingsBtn = $('btn-settings');
    if (!settingsBtn || !settingsBtn.parentElement) return;
    const btn = ce('div', 'pill-btn');
    btn.id = 'btn-power';
    btn.setAttribute('role', 'button');
    btn.setAttribute('aria-label', 'Ein/Aus');
    btn.dataset.navUp = settingsBtn.dataset.navUp || '';
    btn.dataset.navLeft = 'btn-settings';
    btn.innerHTML = svg('power');
    settingsBtn.dataset.navRight = 'btn-power';
    settingsBtn.parentElement.appendChild(btn);
    btn.addEventListener('click', openPower);
    g(() => navElements.push(btn)); // so the phone pointer can hover/click it like the other pills
  })();

  // Standby: the picture goes black, nothing runs on screen, the display
  // blocker is released (Windows may then power the panel down by its own
  // timer). Any key / click / phone input wakes it and is swallowed, like the
  // first press on a real remote.
  const standbyEl = ce('div');
  standbyEl.id = 'tvos-standby';
  document.body.appendChild(standbyEl);
  let standbyOn = false;
  let standbyAt = 0;
  let pausedVideos = [];
  let moved = 0;
  let lastMouse = null;

  function enterStandby() {
    if (standbyOn) return;
    standbyOn = true;
    standbyAt = performance.now();
    moved = 0;
    lastMouse = null;
    document.body.classList.add('cursor-hidden');
    standbyEl.classList.add('on');
    pausedVideos = Array.from(document.querySelectorAll('video')).filter((v) => !v.paused);
    pausedVideos.forEach((v) => v.pause());
    if (bridge) bridge.setStandby(true).catch(() => {});
    g(() => notifyHomeVisibility()); // renderer.js: the Live TV tile stops its stream
  }
  function wake() {
    if (!standbyOn) return;
    standbyOn = false;
    standbyEl.classList.remove('on');
    pausedVideos.forEach((v) => v.play().catch(() => {}));
    pausedVideos = [];
    if (bridge) bridge.setStandby(false).catch(() => {});
    g(() => resetIdleTimer());
    g(() => notifyHomeVisibility());
  }
  // The key that chose "Bildschirm aus" may still be held down (auto-repeat).
  const canWake = () => performance.now() - standbyAt > 700;
  ['mousedown', 'wheel', 'touchstart'].forEach((type) => {
    // passive:false so preventDefault is allowed (wheel/touch listeners on window are passive by default)
    window.addEventListener(type, (e) => { if (standbyOn && canWake()) { swallow(e); wake(); } }, { capture: true, passive: false });
  });
  window.addEventListener('mousemove', (e) => {
    if (!standbyOn) return;
    if (lastMouse) moved += Math.abs(e.clientX - lastMouse.x) + Math.abs(e.clientY - lastMouse.y);
    lastMouse = { x: e.clientX, y: e.clientY };
    if (moved > 60 && canWake()) wake(); // a real mouse move, not a bump of the table
  }, true);

  const modalOpen = () => powerOpen || standbyOn || splashMode !== 'off';
  // Art Mode's idle timer and the pointer/tilt code ask renderer.js whether an
  // overlay is open: our modals count, so the screensaver never starts behind
  // the power menu or a black standby screen.
  if (typeof baseAnyOverlayOpen === 'function') {
    window.anyOverlayOpen = function anyOverlayOpen() { return modalOpen() || baseAnyOverlayOpen(); };
  }

  // ------------------------------------------------------------------ keys
  let escTimer = null;
  const LONG_PRESS_MS = 700;

  window.addEventListener('keydown', (e) => {
    if (standbyOn) {
      swallow(e);
      if (!e.repeat && canWake()) wake();
      return;
    }
    if (e.key === 'Tab') e.preventDefault(); // no focus hopping between buttons, no browser focus rings

    if (e.key === 'Home' || e.key === 'BrowserHome') {
      if (g(() => artModeActive)) return; // the first key only wakes the screensaver
      swallow(e);
      goHome();
      return;
    }
    if (powerOpen) { powerKey(e); return; }
    if (splashMode === 'error') {
      swallow(e);
      switch (e.key) {
        case 'ArrowLeft': case 'ArrowUp': setErrFocus(errFocus - 1); break;
        case 'ArrowRight': case 'ArrowDown': setErrFocus(errFocus + 1); break;
        case 'Enter': if (!e.repeat) activateErrorButton(errButtons[errFocus]); break;
        case 'Escape': case 'Backspace': goHome(); break;
      }
      return;
    }
    if (splashMode === 'loading' && (e.key === 'Escape' || e.key === 'Backspace')) {
      swallow(e);
      goHome();
      return;
    }

    if (e.key === 'Escape') {
      // Holding Escape must not peel off overlay after overlay via key repeat...
      if (e.repeat) { swallow(e); return; }
      // ...but a long press is the universal "take me home" (real key presses
      // only: the phone's synthetic Escape has no key-up to cancel the timer).
      if (e.isTrusted) {
        clearTimeout(escTimer);
        escTimer = setTimeout(() => { escTimer = null; goHome(); }, LONG_PRESS_MS);
      }
    }
  }, true);
  window.addEventListener('keyup', (e) => {
    if (e.key === 'Escape') { clearTimeout(escTimer); escTimer = null; }
  }, true);
  window.addEventListener('blur', () => { clearTimeout(escTimer); escTimer = null; });

  // ------------------------------------------------------------------ 7. hardening
  // Phone pointer hover over our own modals (renderer.js hovers only its own screens).
  function pointerHover(x, y) {
    if (!powerOpen && splashMode !== 'error') return;
    const el = document.elementFromPoint(x, y);
    if (!el) return;
    if (powerOpen) {
      const i = powerButtons.indexOf(el.closest('.tvos-power-btn'));
      if (i >= 0 && i !== powerFocus) setPowerFocus(i);
    } else {
      const i = errButtons.indexOf(el.closest('.tvos-btn'));
      if (i >= 0 && i !== errFocus) setErrFocus(i);
    }
  }
  function installPointerHook() {
    const m = window.motion;
    if (!m || typeof m.pointer !== 'function' || m.pointer.__tvos) return;
    const base = m.pointer;
    m.pointer = function (x, y) { base.apply(this, arguments); pointerHover(x, y); };
    m.pointer.__tvos = true;
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', installPointerHook, { once: true });
  else installPointerHook();

  // Dragging an image/link shows a ghost; dropping a file on the window would
  // navigate to it (main.js blocks the navigation too).
  document.addEventListener('dragstart', (e) => e.preventDefault());
  window.addEventListener('dragover', (e) => e.preventDefault());
  window.addEventListener('drop', (e) => e.preventDefault());
  // Ctrl+wheel = browser zoom.
  window.addEventListener('wheel', (e) => { if (e.ctrlKey) e.preventDefault(); }, { passive: false });
  // A mouse click must never move browser focus onto a button (focus ring, and
  // Enter/Space would then "click" it a second time on top of the launcher's own handling).
  document.addEventListener('mousedown', (e) => {
    // Middle click would start Windows-style autoscroll (a little arrow icon).
    if (e.button === 1) { e.preventDefault(); return; }
    if (e.target.closest && e.target.closest('button, [tabindex], select, .pill-btn')) e.preventDefault();
  }, true);
  // Native tooltips (title="...") pop up after a second of hovering: a PC thing.
  document.querySelectorAll('[title]').forEach((node) => {
    if (!node.hasAttribute('aria-label')) node.setAttribute('aria-label', node.getAttribute('title'));
    node.removeAttribute('title');
  });
  // alert() is a native Windows dialog: show a toast instead.
  window.alert = (message) => {
    toast({ icon: 'warning', kind: 'warning', title: 'Hinweis', text: String(message == null ? '' : message), duration: 7000, force: true });
  };

  // ------------------------------------------------------------------ preferences
  function applyPrefs() {
    if (bridge) bridge.setKeepAwake(g(() => prefs.keepAwake) !== false).catch(() => {});
  }
  const baseSavePrefs = window.savePrefs;
  if (typeof baseSavePrefs === 'function') {
    window.savePrefs = function savePrefs() {
      const result = baseSavePrefs.apply(this, arguments);
      applyPrefs();
      return result;
    };
  }

  // ------------------------------------------------------------------ wiring
  if (bridge) {
    bridge.onToast((payload) => toast(payload));
    bridge.onNetwork(applyNetwork);
    bridge.getNetwork().then((s) => { if (s && s.known) applyNetwork(s); }).catch(() => {});
    bridge.onContentState(onContentState);
    bridge.onHome(goHome);
    bridge.onPowerMenu(openPower);
    bridge.onWake(wake);
    applyPrefs();
  }
  // The web app was closed (Escape, phone Back, ...): drop the splash/error screen.
  g(() => window.launcher.onContentClosed(() => { contentState = 'idle'; hideSplash(); }));
  // The OS-level events sharpen the probe; the main process does the actual check.
  window.addEventListener('offline', () => { if (bridge) bridge.probeNetwork().catch(() => {}); });
  window.addEventListener('online', () => { if (bridge) bridge.probeNetwork().catch(() => {}); });

  window.tvos = {
    toast,
    dismissToast,
    goHome,
    on,
    brandFor,
    isModalOpen: modalOpen,
    power: { open: openPower, close: closePower, isOpen: () => powerOpen },
    standby: { enter: enterStandby, wake, isOn: () => standbyOn },
    network: { get online() { return net.online; }, onChange: (cb) => on('network', cb) },
    content: { get state() { return contentState; } }
  };
})();
