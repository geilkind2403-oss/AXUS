// --- Control Center (Apple-TV style): hold OK/Enter for a moment and a panel
// slides in at the top right with the quick switches -- WLAN, Bluetooth,
// volume, music (the launcher's own player), ad blocker, notifications,
// keep-awake, sleep timer, screen off, settings.
//
// Long press vs. short press: a real Enter keydown is held back here (window
// capture phase, so it runs before renderer.js's router) until we know which
// one it is. Released before HOLD_MS -> the same Enter is replayed as a
// synthetic keydown, so every screen handles it exactly as before, just on
// release instead of on press. Still held at HOLD_MS -> Control Center opens.
// Synthetic Enters (the phone remote's 'k' messages, the replay itself) have
// no key-up and pass straight through.
//
// Loaded after tvos.js: its capture listener (standby, power menu, error
// screen) runs first and swallows the keys it owns.
(() => {
  const HOLD_MS = 600;
  const g = (fn) => { try { return fn(); } catch { return undefined; } };
  const swallow = (e) => { e.preventDefault(); e.stopImmediatePropagation(); };
  const $ = (id) => document.getElementById(id);

  const ICONS = {
    wifi: '<path d="M2.5 9a14 14 0 0 1 19 0M5.5 12.5a9.5 9.5 0 0 1 13 0M8.6 15.9a5 5 0 0 1 6.8 0"/><circle cx="12" cy="19.2" r="1.1" fill="currentColor" stroke="none"/>',
    wifiOff: '<path d="M3 3l18 18M8.6 15.9a5 5 0 0 1 6.8 0M5.5 12.5a9.5 9.5 0 0 1 4.2-2.3M2.5 9a14 14 0 0 1 5-3M14.5 10.3a9.5 9.5 0 0 1 4 2.2M12.5 5a14 14 0 0 1 9 4"/>',
    bluetooth: '<path d="M7 7l10 10-5 4V3l5 4L7 17"/>',
    shield: '<path d="M12 3l8 3v6c0 4.5-3.4 8.2-8 9-4.6-.8-8-4.5-8-9V6z"/><path d="M8.5 12l2.5 2.5 4.5-5"/>',
    bell: '<path d="M6 16V11a6 6 0 0 1 12 0v5l1.5 2h-15z"/><path d="M10 20.5a2 2 0 0 0 4 0"/>',
    sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2.5v2.5M12 19v2.5M2.5 12H5M19 12h2.5M5.3 5.3l1.8 1.8M16.9 16.9l1.8 1.8M5.3 18.7l1.8-1.8M16.9 7.1l1.8-1.8"/>',
    tv: '<rect x="3" y="5" width="18" height="12" rx="2"/><path d="M8 21h8M12 17v4"/>',
    gear: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/>',
    prev: '<path d="M6 5v14M19 5l-10 7 10 7z" fill="currentColor"/>',
    next: '<path d="M18 5v14M5 5l10 7-10 7z" fill="currentColor"/>',
    play: '<path d="M7 4.5l13 7.5-13 7.5z" fill="currentColor"/>',
    pause: '<path d="M7 4.5h3.5v15H7zM13.5 4.5H17v15h-3.5z" fill="currentColor" stroke="none"/>',
    volume: '<path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z" fill="currentColor" stroke="none"/><path d="M15.5 9a4 4 0 0 1 0 6M18 6.5a7.5 7.5 0 0 1 0 11"/>',
    volumeOff: '<path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z" fill="currentColor" stroke="none"/><path d="M16 9.5l5 5M21 9.5l-5 5"/>',
    moon: '<path d="M20 14.5A8.5 8.5 0 0 1 9.5 4a8.5 8.5 0 1 0 10.5 10.5z"/>',
    music: '<path d="M9 18V5l11-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="17" cy="16" r="3"/>'
  };
  const svg = (name) => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${ICONS[name]}</svg>`;

  // The three switches write the same prefs as the Settings screen, so both
  // always agree (Settings re-reads prefs every time it opens).
  const TOGGLES = {
    adblock: { set: (v) => g(() => window.launcher.setAdblock(v)) },
    notifications: { set: () => {} }, // tvos.js reads prefs.notifications per toast
    keepAwake: { set: () => {} } // tvos.js's savePrefs wrapper pushes it to the main process
  };

  // Sleep timer: OK steps Aus -> 30 -> 60 -> 90 -> Aus (sleep-timer-main.js runs it).
  const SLEEP_STEPS = [30, 60, 90];

  const overlay = document.createElement('div');
  overlay.id = 'cc-overlay';
  overlay.innerHTML = `
    <div id="cc-panel" role="dialog" aria-label="Kontrollzentrum">
      <div class="cc-head">
        <div class="cc-title">Kontrollzentrum</div>
        <div class="cc-clock" id="cc-clock"></div>
      </div>
      <div class="cc-row cc-row-2">
        <button class="cc-item cc-tile" type="button" tabindex="-1" data-act="wifi">
          <span class="cc-ic" id="cc-wifi-ic">${svg('wifi')}</span>
          <span class="cc-txt"><span class="cc-label">WLAN</span><span class="cc-sub" id="cc-wifi-sub">…</span></span>
        </button>
        <button class="cc-item cc-tile" type="button" tabindex="-1" data-act="bluetooth">
          <span class="cc-ic" id="cc-bt-ic">${svg('bluetooth')}</span>
          <span class="cc-txt"><span class="cc-label">Bluetooth</span><span class="cc-sub" id="cc-bt-sub">…</span></span>
        </button>
      </div>
      <div class="cc-row cc-row-1">
        <button class="cc-item cc-volume" type="button" tabindex="-1" data-act="volume" aria-label="Lautstärke">
          <span class="cc-ic" id="cc-vol-ic">${svg('volume')}</span>
          <span class="cc-vol-track" id="cc-vol-track"><span class="cc-vol-fill" id="cc-vol-fill"></span></span>
          <span class="cc-vol-val" id="cc-vol-val">–</span>
        </button>
      </div>
      <div class="cc-music" id="cc-music">
        <div class="cc-music-info">
          <div class="cc-cover" id="cc-cover">${svg('music')}</div>
          <div class="cc-txt"><span class="cc-label" id="cc-song">Musik</span><span class="cc-sub" id="cc-artist">Nichts wird abgespielt</span></div>
        </div>
        <div class="cc-row cc-row-3 cc-transport">
          <button class="cc-item cc-round" type="button" tabindex="-1" data-act="previous" aria-label="Zurück">${svg('prev')}</button>
          <button class="cc-item cc-round" type="button" tabindex="-1" data-act="playpause" aria-label="Abspielen/Pause" id="cc-playpause">${svg('play')}</button>
          <button class="cc-item cc-round" type="button" tabindex="-1" data-act="next" aria-label="Weiter">${svg('next')}</button>
        </div>
      </div>
      <div class="cc-row cc-row-3">
        <button class="cc-item cc-switch" type="button" tabindex="-1" data-toggle="adblock">
          <span class="cc-ic">${svg('shield')}</span><span class="cc-label">Werbeblocker</span><span class="cc-sub"></span>
        </button>
        <button class="cc-item cc-switch" type="button" tabindex="-1" data-toggle="notifications">
          <span class="cc-ic">${svg('bell')}</span><span class="cc-label">Mitteilungen</span><span class="cc-sub"></span>
        </button>
        <button class="cc-item cc-switch" type="button" tabindex="-1" data-toggle="keepAwake">
          <span class="cc-ic">${svg('sun')}</span><span class="cc-label">Wach bleiben</span><span class="cc-sub"></span>
        </button>
      </div>
      <div class="cc-row cc-row-1">
        <button class="cc-item cc-tile cc-sleep" type="button" tabindex="-1" data-act="sleep">
          <span class="cc-ic">${svg('moon')}</span>
          <span class="cc-txt"><span class="cc-label">Sleep-Timer</span><span class="cc-sub" id="cc-sleep-sub">Aus</span></span>
          <span class="cc-sleep-steps">${SLEEP_STEPS.map((m) => `<span data-min="${m}">${m}</span>`).join('')}</span>
        </button>
      </div>
      <div class="cc-row cc-row-2">
        <button class="cc-item cc-wide" type="button" tabindex="-1" data-act="standby">${svg('tv')}<span>Bildschirm aus</span></button>
        <button class="cc-item cc-wide" type="button" tabindex="-1" data-act="settings">${svg('gear')}<span>Einstellungen</span></button>
      </div>
      <div class="cc-hint">OK gedrückt halten öffnet · Zurück schließt</div>
    </div>`;
  document.body.appendChild(overlay);

  const panel = $('cc-panel');
  const musicEl = $('cc-music');
  let open = false;
  let rows = [];
  let row = 0;
  let col = 0;
  let clockTimer = null;

  // Focus grid: one array of buttons per visible row, rebuilt when the music
  // row appears or disappears.
  function buildRows() {
    rows = Array.from(panel.querySelectorAll('.cc-row'))
      .filter((r) => r.offsetParent !== null)
      .map((r) => Array.from(r.querySelectorAll('.cc-item')));
  }
  function setFocus(r, c) {
    row = Math.max(0, Math.min(rows.length - 1, r));
    col = Math.max(0, Math.min(rows[row].length - 1, c));
    panel.querySelectorAll('.cc-item.focused').forEach((el) => el.classList.remove('focused'));
    rows[row][col].classList.add('focused');
  }
  const focusedItem = () => rows[row] && rows[row][col];

  function syncToggles() {
    panel.querySelectorAll('[data-toggle]').forEach((btn) => {
      const on = g(() => prefs[btn.dataset.toggle]) !== false;
      btn.classList.toggle('on', on);
      btn.querySelector('.cc-sub').textContent = on ? 'An' : 'Aus';
    });
  }

  function tickClock() { // hhmm: renderer.js
    $('cc-clock').textContent = hhmm(new Date());
    renderSleep();
  }

  // ------------------------------------------------------------ sleep timer
  let sleep = { endsAt: 0, step: 0 }; // step: the 30/60/90 last chosen
  function renderSleep() {
    const btn = panel.querySelector('[data-act="sleep"]');
    const active = sleep.endsAt > Date.now();
    btn.classList.toggle('on', active);
    const left = active ? Math.max(1, Math.ceil((sleep.endsAt - Date.now()) / 60000)) : 0;
    $('cc-sleep-sub').textContent = active ? `Noch ${left} Min. · aus um ${hhmm(new Date(sleep.endsAt))}` : 'Aus';
    btn.querySelectorAll('[data-min]').forEach((el) => el.classList.toggle('sel', active && Number(el.dataset.min) === sleep.step));
  }
  function cycleSleep() {
    const i = sleep.endsAt > Date.now() ? SLEEP_STEPS.indexOf(sleep.step) : -1;
    const next = SLEEP_STEPS[i + 1] || 0; // after 90: off
    sleep = { endsAt: next ? Date.now() + next * 60000 : 0, step: next };
    renderSleep();
    Promise.resolve(g(() => window.launcher.setSleepTimer(next))).catch(() => {});
  }
  g(() => window.launcher.onSleepTimerState((s) => {
    const endsAt = (s && s.endsAt) || 0;
    if (!endsAt) sleep = { endsAt: 0, step: 0 };
    else sleep.endsAt = endsAt;
    renderSleep();
  }));
  // Time's up: main has already paused and closed any film -> screen off.
  g(() => window.launcher.onSleepTimerFire(() => {
    closeCC();
    g(() => window.music.pause());
    g(() => window.tvos.standby.enter());
  }));

  // Status is fetched fresh on every open (netsh/PowerShell behind these are
  // slow, so the panel shows "…" first and fills in).
  function refreshStatus() {
    const online = g(() => window.tvos.network.online) !== false;
    Promise.resolve(g(() => window.launcher.getWifiStatus())).then((s) => {
      const on = s && s.connected;
      $('cc-wifi-ic').innerHTML = svg(on || online ? 'wifi' : 'wifiOff');
      panel.querySelector('[data-act="wifi"]').classList.toggle('on', Boolean(on || online));
      $('cc-wifi-sub').textContent = on ? s.ssid : (online ? 'Verbunden' : 'Nicht verbunden');
    }).catch(() => { $('cc-wifi-sub').textContent = online ? 'Verbunden' : 'Nicht verbunden'; });

    Promise.resolve(g(() => window.launcher.getBluetoothStatus())).then((s) => {
      const on = s && s.radioOn;
      const n = (s && s.devices ? s.devices.filter((d) => d.connected).length : 0);
      panel.querySelector('[data-act="bluetooth"]').classList.toggle('on', Boolean(on));
      $('cc-bt-sub').textContent = !on ? 'Aus' : (n ? `${n} verbunden` : 'An');
    }).catch(() => { $('cc-bt-sub').textContent = 'Nicht verfügbar'; });

    refreshMusic();
    refreshVolume();
  }

  // ------------------------------------------------------------ volume
  // Optimistic: the bar moves at once, the (slow-ish) helper call follows,
  // debounced so holding ◀/▶ doesn't queue a process per key repeat.
  const VOL_STEP = 5;
  let vol = { available: false, level: 0, muted: false };
  let volTimer = null;

  function renderVolume() {
    const volBtn = panel.querySelector('[data-act="volume"]');
    volBtn.classList.toggle('unavailable', !vol.available);
    volBtn.classList.toggle('on', vol.available && !vol.muted);
    $('cc-vol-ic').innerHTML = svg(vol.muted || vol.level === 0 ? 'volumeOff' : 'volume');
    $('cc-vol-fill').style.width = `${vol.available ? vol.level : 0}%`;
    $('cc-vol-val').textContent = !vol.available ? 'n. v.' : (vol.muted ? 'Stumm' : String(vol.level));
  }
  function refreshVolume() {
    Promise.resolve(g(() => window.launcher.getVolume())).then((v) => {
      if (v && v.available) vol = { available: true, level: v.level, muted: v.muted };
      else vol.available = false;
      renderVolume();
    }).catch(() => { vol.available = false; renderVolume(); });
  }
  function setVolume(level) {
    if (!vol.available) return;
    vol.level = Math.max(0, Math.min(100, Math.round(level)));
    if (vol.level > 0) vol.muted = false;
    renderVolume();
    clearTimeout(volTimer);
    volTimer = setTimeout(() => window.launcher.setVolume(vol.level).catch(() => {}), 120);
  }
  function toggleMute() {
    if (!vol.available) return;
    vol.muted = !vol.muted;
    renderVolume();
    window.launcher.setMute(vol.muted).catch(() => {});
  }

  // The music player (music.js): shown while something is loaded in it.
  function refreshMusic() {
    const st = g(() => window.music.state()) || {};
    const t = st.track;
    const had = musicEl.classList.contains('on');
    musicEl.classList.toggle('on', Boolean(t));
    if (open && had !== Boolean(t)) { const item = focusedItem(); buildRows(); restoreFocus(item); }
    if (!t) return;
    $('cc-song').textContent = t.title;
    $('cc-artist').textContent = `${t.artist}${st.playing ? '' : ' · pausiert'}`;
    $('cc-cover').innerHTML = st.cover ? `<img src="${escapeHtml(st.cover)}" alt="">` : svg('music');
    $('cc-playpause').innerHTML = svg(st.playing ? 'pause' : 'play');
  }
  g(() => window.music.onChange(() => { if (open) refreshMusic(); }));

  function restoreFocus(item) {
    for (let r = 0; r < rows.length; r++) {
      const c = rows[r].indexOf(item);
      if (c >= 0) { setFocus(r, c); return; }
    }
    setFocus(0, 0);
  }

  function openCC() {
    if (open) return;
    open = true;
    syncToggles();
    tickClock();
    clockTimer = setInterval(tickClock, 15000);
    overlay.classList.add('on');
    document.body.classList.add('cc-open');
    buildRows();
    setFocus(0, 0);
    refreshStatus();
  }
  function closeCC() {
    if (!open) return;
    open = false;
    clearInterval(clockTimer);
    overlay.classList.remove('on');
    document.body.classList.remove('cc-open');
  }

  function activate(btn) {
    if (!btn) return;
    const toggle = btn.dataset.toggle;
    if (toggle) {
      const next = g(() => prefs[toggle]) === false;
      g(() => { prefs[toggle] = next; window.savePrefs(prefs); });
      TOGGLES[toggle].set(next);
      syncToggles();
      return;
    }
    switch (btn.dataset.act) {
      case 'wifi': closeCC(); g(() => window.openSettings('wifi')); break;
      case 'bluetooth': closeCC(); g(() => window.openSettings('bluetooth')); break;
      case 'settings': closeCC(); g(() => window.openSettings()); break;
      case 'volume': toggleMute(); break;
      case 'sleep': cycleSleep(); break;
      case 'standby': closeCC(); g(() => window.tvos.standby.enter()); break;
      case 'playpause': g(() => window.music.toggle()); break;
      case 'previous': g(() => window.music.prev()); break;
      case 'next': g(() => window.music.next()); break;
    }
  }

  function handleKey(e) {
    swallow(e);
    switch (e.key) {
      case 'ArrowUp': if (row > 0) setFocus(row - 1, Math.round(col * (rows[row - 1].length - 1) / Math.max(1, rows[row].length - 1))); break;
      case 'ArrowDown': if (row < rows.length - 1) setFocus(row + 1, Math.round(col * (rows[row + 1].length - 1) / Math.max(1, rows[row].length - 1))); break;
      case 'ArrowLeft':
        if (focusedItem().dataset.act === 'volume') setVolume(vol.level - VOL_STEP);
        else setFocus(row, col - 1);
        break;
      case 'ArrowRight':
        if (focusedItem().dataset.act === 'volume') setVolume(vol.level + VOL_STEP);
        else setFocus(row, col + 1);
        break;
      case 'Enter': case ' ': if (!e.repeat) activate(focusedItem()); break;
      case 'Escape': case 'Backspace': if (!e.repeat) closeCC(); break;
    }
  }

  panel.addEventListener('click', (e) => {
    if (!open) return;
    const btn = e.target.closest('.cc-item');
    if (!btn) return;
    restoreFocus(btn);
    const track = e.target.closest('#cc-vol-track');
    if (track) {
      const r = track.getBoundingClientRect();
      setVolume(((e.clientX - r.left) / r.width) * 100);
      return;
    }
    activate(btn);
  });
  // Hover focuses like the D-pad would. Real mouse via mousemove; the phone
  // pointer fires no mouse events, so renderer.js's hoverFocusAt calls this.
  function pointerHover(el) {
    const btn = open && el && el.closest && el.closest('.cc-item');
    if (btn && panel.contains(btn) && !btn.classList.contains('focused')) restoreFocus(btn);
  }
  panel.addEventListener('mousemove', (e) => pointerHover(e.target));
  overlay.addEventListener('click', (e) => { if (e.target === overlay) closeCC(); });

  // ------------------------------------------------------------ long press
  let holdTimer = null;
  let holding = false; // a real Enter is down and being held back

  // Only where the launcher itself has the keys: not over the screensaver
  // (the first key just wakes it) and not while a web app has focus.
  const canIntercept = () => !g(() => artModeActive) && !g(() => contentOpen);

  function cancelHold() {
    clearTimeout(holdTimer);
    holdTimer = null;
    holding = false;
  }

  window.addEventListener('keydown', (e) => {
    if (open) { handleKey(e); return; }
    if (e.key !== 'Enter' || !e.isTrusted) return;
    if (holding) { swallow(e); return; } // auto-repeat while deciding / after opening
    if (e.repeat || !canIntercept()) return;
    swallow(e);
    holding = true;
    holdTimer = setTimeout(() => {
      holdTimer = null;
      g(() => resetIdleTimer());
      openCC();
    }, HOLD_MS);
  }, true);

  window.addEventListener('keyup', (e) => {
    if (e.key !== 'Enter' || !holding) return;
    swallow(e);
    const shortPress = holdTimer !== null;
    cancelHold();
    if (shortPress) document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', bubbles: true, cancelable: true }));
  }, true);
  window.addEventListener('blur', cancelHold);

  // Art Mode's idle timer asks anyOverlayOpen(): an open Control Center counts.
  const baseAnyOverlayOpen = window.anyOverlayOpen;
  if (typeof baseAnyOverlayOpen === 'function') {
    window.anyOverlayOpen = function anyOverlayOpen() { return open || baseAnyOverlayOpen(); };
  }
  g(() => window.tvos.on('home', closeCC));

  window.controlCenter = { open: openCC, close: closeCC, isOpen: () => open, pointerHover };
})();
