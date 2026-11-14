// --- Motion system (see motion.css). Loaded last, so it can hook into every
// other screen without those screens knowing about it:
//   1. boot: letter-by-letter wordmark, then the home screen rises in
//   2. focus: specular sweep on every newly focused tile + 3D tilt/glare
//      that follows the phone pointer (or a real mouse)
//   3. overlays zoom out of the element that opened them and back into it
//      on close (wraps renderer.js's animatedOpen/animatedClose)
//   4. sliding indicators (top bar, weather categories)
//   5. Art Mode exit replays the home entrance ("welcome back")
(() => {
  const EXPO = 'cubic-bezier(.16, 1, .3, 1)';
  const OUT = 'cubic-bezier(.2, .8, .2, 1)';
  const IN = 'cubic-bezier(.4, 0, 1, 1)';
  const $ = (id) => document.getElementById(id);

  // ------------------------------------------------------------- 1. boot
  const word = $('boot-splash-word');
  if (word && !word.classList.contains('split')) {
    const text = word.textContent;
    word.textContent = '';
    [...text].forEach((ch, i) => {
      const s = document.createElement('span');
      s.className = 'ch';
      s.style.setProperty('--c', i);
      s.textContent = ch;
      word.appendChild(s);
    });
    word.classList.add('split');
  }

  // Tiles rise in a diagonal wave from the top-left, whatever the grid layout.
  function assignRiseOrder() {
    [...document.querySelectorAll('#shortcut-grid .grid-cell')]
      .map((c) => ({ c, r: c.getBoundingClientRect() }))
      .sort((a, b) => (a.r.left + a.r.top * 0.7) - (b.r.left + b.r.top * 0.7))
      .forEach(({ c }, i) => c.style.setProperty('--rise-i', i));
  }
  function playHomeRise() {
    assignRiseOrder();
    document.body.classList.remove('boot-done');
    void document.body.offsetWidth; // restart the CSS animations
    document.body.classList.add('boot-done');
  }
  const splash = $('boot-splash');
  if (splash && !splash.classList.contains('hidden')) {
    // renderer.js lifts the splash (adds .hidden); the home screen rises in
    // at that exact moment instead of animating unseen underneath it.
    new MutationObserver((_, obs) => {
      if (!splash.classList.contains('hidden')) return;
      obs.disconnect();
      playHomeRise();
    }).observe(splash, { attributes: true, attributeFilter: ['class'] });
  } else {
    playHomeRise();
  }

  // ------------------------------------------------------------- 2. focus
  function sheenFor(el) {
    let s = el.querySelector(':scope > .motion-sheen');
    if (!s) {
      s = document.createElement('span');
      s.className = 'motion-sheen';
      el.appendChild(s);
    }
    return s;
  }
  function sweep(el) {
    if (!el) return;
    const s = sheenFor(el);
    s.classList.remove('sweep');
    void s.offsetWidth;
    s.classList.add('sweep');
  }

  // Pointer-driven tilt. Positions come from a real mouse and from the phone
  // remote (renderer.js drawPointer -> motion.pointer).
  let px = -1;
  let py = -1;
  let lastMove = 0;
  let tiltEl = null;
  let raf = 0;
  const cur = { x: 0, y: 0, gx: 50, gy: 0 };

  function currentTiltEl() {
    if (window.appFolder && window.appFolder.isOpen()) return document.querySelector('#appfolder-overlay .af-item.focused .af-card');
    if (typeof anyOverlayOpen === 'function' && anyOverlayOpen()) return null;
    return document.querySelector('#shortcut-grid .grid-cell.focused');
  }
  function clearTilt(el) {
    el.style.removeProperty('--tilt-x');
    el.style.removeProperty('--tilt-y');
    const s = el.querySelector(':scope > .motion-sheen');
    if (s) s.style.setProperty('--glare-o', 0);
  }
  function retarget() {
    const el = currentTiltEl();
    if (el !== tiltEl) {
      if (tiltEl) clearTilt(tiltEl);
      tiltEl = el;
      cur.x = 0; cur.y = 0;
    }
    kick();
  }
  function kick() { if (!raf) raf = requestAnimationFrame(step); }
  function step(now) {
    raf = 0;
    const el = tiltEl;
    if (!el || !el.isConnected) return;
    const r = el.getBoundingClientRect();
    const inside = now - lastMove < 2500 && px >= r.left && px <= r.right && py >= r.top && py <= r.bottom;
    const max = el.classList.contains('af-card') ? 9 : 6;
    let tx = 0;
    let ty = 0;
    let gx = 50;
    let gy = 0;
    if (inside) {
      const nx = (px - r.left) / r.width;
      const ny = (py - r.top) / r.height;
      tx = (0.5 - ny) * 2 * max;
      ty = (nx - 0.5) * 2 * max * 1.2;
      gx = nx * 100;
      gy = ny * 100;
    }
    cur.x += (tx - cur.x) * 0.16;
    cur.y += (ty - cur.y) * 0.16;
    cur.gx += (gx - cur.gx) * 0.2;
    cur.gy += (gy - cur.gy) * 0.2;
    el.style.setProperty('--tilt-x', `${cur.x.toFixed(2)}deg`);
    el.style.setProperty('--tilt-y', `${cur.y.toFixed(2)}deg`);
    const s = sheenFor(el);
    s.style.setProperty('--glare-x', `${cur.gx.toFixed(1)}%`);
    s.style.setProperty('--glare-y', `${cur.gy.toFixed(1)}%`);
    s.style.setProperty('--glare-o', inside ? 1 : 0);
    if (inside || Math.abs(cur.x - tx) > 0.05 || Math.abs(cur.y - ty) > 0.05) kick();
  }
  function pointer(x, y) {
    px = x;
    py = y;
    lastMove = performance.now();
    retarget();
  }
  document.addEventListener('mousemove', (e) => pointer(e.clientX, e.clientY), { passive: true });

  // Every tile/card that newly gains .focused gets the light sweep.
  const focusObs = new MutationObserver((muts) => {
    for (const m of muts) {
      const el = m.target;
      if (!el.classList || !el.classList.contains('focused') || (m.oldValue || '').split(' ').includes('focused')) continue;
      if (el.matches('#shortcut-grid .grid-cell')) sweep(el);
      else if (el.matches('#appfolder-overlay .af-item') && !el.closest('.flying')) sweep(el.querySelector('.af-card'));
    }
    retarget();
  });
  const focusOpts = { attributes: true, attributeFilter: ['class'], attributeOldValue: true, subtree: true };
  if ($('shortcut-grid')) focusObs.observe($('shortcut-grid'), focusOpts);
  if ($('af-grid')) focusObs.observe($('af-grid'), focusOpts);

  // ------------------------------------------------------------- 3. overlays
  // Which element opened the overlay: the thing that was clicked, or the
  // visibly focused element when Enter was pressed (keyboard, D-pad, phone).
  let activator = null;
  let activatorAt = 0;
  function markActivator(el) {
    if (!el) return;
    activator = el;
    activatorAt = performance.now();
    const pressable = el.closest('#shortcut-grid .grid-cell, #bottombar .pill-btn, #appfolder-overlay .af-item');
    if (pressable) {
      pressable.classList.add('motion-press');
      setTimeout(() => pressable.classList.remove('motion-press'), 150);
    }
  }
  function topFocused() {
    const all = [...document.querySelectorAll('.focused')];
    for (let i = all.length - 1; i >= 0; i--) {
      const el = all[i];
      const r = el.getBoundingClientRect();
      if (r.width < 4 || r.height < 4) continue;
      const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      if (hit && (hit === el || el.contains(hit))) return el;
    }
    return null;
  }
  document.addEventListener('click', (e) => {
    markActivator(e.target.closest('[data-action], .grid-cell, .pill-btn, .af-item, button, [data-kind]') || e.target);
  }, true);
  document.addEventListener('keydown', (e) => { if (e.key === 'Enter') markActivator(topFocused()); }, true);

  function visibleRect(el) {
    if (!el || !el.isConnected) return null;
    const r = el.getBoundingClientRect();
    if (r.width < 4 || r.height < 4 || r.bottom < 0 || r.right < 0 || r.top > innerHeight || r.left > innerWidth) return null;
    return { left: r.left, top: r.top, width: r.width, height: r.height };
  }
  const box = (r) => ({ left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px` });

  // Overlays that are full-screen pages zoom to the whole screen; panel-style
  // overlays zoom into their panel.
  const PORTAL = {
    'weather-overlay': null,
    'settings-overlay': null,
    'library-overlay': null,
    'library-detail-overlay': 'library-detail-panel',
    'mediathek-overlay': 'mediathek-panel'
  };
  const origins = new WeakMap();
  const running = new WeakMap();

  function targetRect(overlay) {
    const panelId = PORTAL[overlay.id];
    const panel = panelId && $(panelId);
    if (panel) {
      panel.getAnimations().forEach((a) => a.cancel());
      const r = visibleRect(panel);
      if (r) return r;
    }
    return { left: 0, top: 0, width: innerWidth, height: innerHeight };
  }
  function stopRunning(overlay) {
    (running.get(overlay) || []).forEach((a) => a.cancel());
    running.set(overlay, []);
  }
  function makePortal(r) {
    const p = document.createElement('div');
    p.className = 'motion-portal';
    Object.assign(p.style, box(r));
    document.body.appendChild(p);
    return p;
  }

  function portalIn(overlay, from) {
    const to = targetRect(overlay);
    const radiusTo = PORTAL[overlay.id] ? '22px' : '0px';
    const p = makePortal(from);
    const grow = p.animate(
      [{ ...box(from), borderRadius: '16px' }, { ...box(to), borderRadius: radiusTo }],
      { duration: 560, easing: EXPO, fill: 'forwards' }
    );
    // The real overlay fades in over the grown portal, settling from slightly
    // small -- it reads as one continuous zoom into the tile.
    const show = overlay.animate(
      [{ opacity: 0, transform: 'scale(.94)' }, { opacity: 1, transform: 'none' }],
      { duration: 460, delay: 190, easing: OUT, fill: 'backwards' }
    );
    running.set(overlay, [show]);
    Promise.all([grow.finished, show.finished]).catch(() => {}).then(() => {
      p.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 200, easing: 'ease' }).finished
        .catch(() => {}).then(() => p.remove());
    });
  }
  function portalOut(overlay, into) {
    const from = targetRect(overlay);
    const p = makePortal(from);
    p.style.borderRadius = PORTAL[overlay.id] ? '22px' : '0px';
    const hide = overlay.animate(
      [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'scale(.96)' }],
      { duration: 230, easing: IN, fill: 'forwards' }
    );
    running.set(overlay, [hide]);
    p.animate([
      { ...box(from), borderRadius: p.style.borderRadius, opacity: 1 },
      { ...box(into), borderRadius: '16px', opacity: 1, offset: 0.82 },
      { ...box(into), borderRadius: '16px', opacity: 0 }
    ], { duration: 520, easing: EXPO, fill: 'forwards' }).finished.catch(() => {}).then(() => p.remove());
  }
  function softIn(overlay) {
    running.set(overlay, [overlay.animate(
      [{ opacity: 0, transform: 'scale(1.03)' }, { opacity: 1, transform: 'none' }],
      { duration: 380, easing: EXPO }
    )]);
  }
  function softOut(overlay) {
    running.set(overlay, [overlay.animate(
      [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'scale(.97)' }],
      { duration: 220, easing: IN, fill: 'forwards' }
    )]);
  }

  const baseOpen = window.animatedOpen;
  const baseClose = window.animatedClose;
  if (typeof baseOpen === 'function' && typeof baseClose === 'function') {
    window.animatedOpen = function (overlay) {
      stopRunning(overlay);
      baseOpen(overlay);
      if (!(overlay.id in PORTAL)) { retarget(); return; }
      const fresh = activator && performance.now() - activatorAt < 900 ? activator : null;
      const rect = visibleRect(fresh);
      origins.set(overlay, rect ? fresh : null);
      if (rect) portalIn(overlay, rect);
      else softIn(overlay);
      retarget();
    };
    window.animatedClose = function (overlay, opts = {}) {
      if (overlay.id === 'artmode-overlay') {
        baseClose(overlay, opts);
        playHomeRise(); // waking up: the home screen builds itself up again
        return;
      }
      if (!(overlay.id in PORTAL)) {
        baseClose(overlay, opts);
        setTimeout(retarget, (opts.duration || 200) + 20);
        return;
      }
      stopRunning(overlay);
      const into = visibleRect(origins.get(overlay));
      if (into) portalOut(overlay, into);
      else softOut(overlay);
      const userDone = opts.onDone;
      baseClose(overlay, {
        ...opts,
        duration: Math.max(opts.duration || 200, 240),
        onDone: () => {
          stopRunning(overlay); // drop the fill-forwards fade so the next open starts clean
          if (userDone) userDone();
          retarget();
        }
      });
    };
  }

  // ------------------------------------------------------------- launch
  // Starting a web app / video: the card grows into a full-screen brand
  // panel (same colours and wordmark as tvos.js's loading splash, which fades
  // in on top of it if loading takes longer), so pressing Netflix reads as
  // "zooming into Netflix". The web view itself loads off-screen meanwhile
  // (tvos-main.js) and covers everything once it's ready. Resolves when the
  // caller should actually start loading -- a little into the zoom, so the
  // splash can't cut into the flight.
  function launch(url) {
    const fresh = activator && performance.now() - activatorAt < 900 ? activator : null;
    const from = visibleRect(fresh);
    if (!from) return Promise.resolve();
    const brand = (window.tvos && typeof window.tvos.brandFor === 'function' && window.tvos.brandFor(url)) || {};
    const full = { left: 0, top: 0, width: innerWidth, height: innerHeight };
    const p = makePortal(from);
    p.classList.add('motion-launch');
    if (brand.bg) p.style.background = brand.bg;
    const mark = document.createElement('div');
    mark.className = 'motion-launch-brand';
    if (brand.icon) {
      const icon = document.createElement('span');
      icon.className = 'motion-launch-icon';
      icon.innerHTML = brand.icon; // static SVG from tvos.js's own brand table
      mark.appendChild(icon);
    }
    const word = document.createElement('span');
    word.textContent = brand.word || '';
    mark.appendChild(word);
    p.appendChild(mark);

    p.animate([{ ...box(from), borderRadius: '16px' }, { ...box(full), borderRadius: '0px' }],
      { duration: 600, easing: EXPO, fill: 'forwards' });
    // Lifted by half the splash's spinner block, so the wordmark sits exactly
    // where tvos.js's splash will draw it -- the handover is invisible.
    mark.animate(
      [{ opacity: 0, transform: 'translateY(-38px) scale(.82)' }, { opacity: 1, transform: 'translateY(-38px) scale(1)' }],
      { duration: 460, delay: 200, easing: OUT, fill: 'both' }
    );

    // Once the web app is ready (or its error screen is up) the panel is
    // hidden under it anyway -- remove it outright: while a native view covers
    // the window Chromium pauses this page's animations, so a fade-out would
    // only finish (visibly) when the user comes back home.
    let finished = false;
    const done = (instant) => {
      if (finished) return;
      finished = true;
      pendingLaunches.delete(done);
      if (instant) { p.remove(); return; }
      p.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 260, easing: 'ease', fill: 'forwards' })
        .finished.catch(() => {}).then(() => p.remove());
    };
    pendingLaunches.add(done);
    setTimeout(() => done(true), 30000); // never strand a panel over the home screen
    return new Promise((resolve) => setTimeout(resolve, 380));
  }
  // Launch panels still on screen. The listeners that remove them are
  // registered once here -- registered per launch they piled up with every
  // app start (Electron warned about a listener leak after the 11th).
  const pendingLaunches = new Set();
  const finishLaunches = (instant) => [...pendingLaunches].forEach((done) => done(instant));
  if (window.tvos && typeof window.tvos.on === 'function') {
    window.tvos.on('content-state', (s) => { if (s && s.state !== 'loading') finishLaunches(true); });
    window.tvos.on('home', () => finishLaunches(false));
  }
  window.launcher.onContentClosed(() => finishLaunches(false));

  // ------------------------------------------------------------- 4. indicators
  function slidingIndicator(container, className, pick, axis) {
    if (!container) return;
    const ind = document.createElement('span');
    ind.className = className;
    container.insertBefore(ind, container.firstChild);
    let placed = false;
    const place = () => {
      const el = pick();
      if (!el || !el.offsetWidth) { ind.style.opacity = 0; return; }
      if (!placed) ind.style.transition = 'none';
      if (axis === 'x') {
        ind.style.width = `${el.offsetWidth}px`;
        ind.style.transform = `translateX(${el.offsetLeft}px)`;
      } else {
        ind.style.height = `${el.offsetHeight}px`;
        ind.style.transform = `translateY(${el.offsetTop}px)`;
      }
      ind.style.opacity = 1;
      if (!placed) { void ind.offsetWidth; ind.style.transition = ''; placed = true; }
    };
    new MutationObserver(place).observe(container, { attributes: true, attributeFilter: ['class'], subtree: true });
    window.addEventListener('resize', () => { placed = false; place(); });
    place();
    return () => { placed = false; place(); };
  }
  slidingIndicator($('topbar-nav'), 'motion-nav-indicator',
    () => document.querySelector('#topbar-nav .nav-item.focused') || document.querySelector('#topbar-nav .nav-item.active'), 'x');
  const replaceCatIndicator = slidingIndicator($('wx-cats'), 'motion-cat-indicator',
    () => document.querySelector('#wx-cats .wx-cat.active'), 'y');
  // The weather sidebar is display:none until opened: re-measure without a
  // slide the moment it becomes visible.
  const wxOverlay = $('weather-overlay');
  if (wxOverlay && replaceCatIndicator) {
    new MutationObserver(() => {
      if (wxOverlay.classList.contains('active') && !wxOverlay.classList.contains('closing')) requestAnimationFrame(replaceCatIndicator);
    }).observe(wxOverlay, { attributes: true, attributeFilter: ['class'] });
  }

  window.motion = {
    pointer,
    playHomeRise,
    sweep,
    launch
  };
})();
