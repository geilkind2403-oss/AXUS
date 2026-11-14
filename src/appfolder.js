// --- Apps folder (Apple-TV style): tile #grid-1 -> openAction('appfolder').
// The eight apps used to be a card stack in a dock on the right; that dock is
// gone and this folder is now the only place they are listed. Logos/colours
// therefore live here exactly once, as data, and feed both the mini icons on
// the tile and the big cards in the full-screen overlay (no duplicated markup
// that can drift apart).
// Loaded after renderer.js and uses its globals (animatedOpen/Close,
// setFocusEl, openAction).
(() => {
  const COLS = 4;

  // face: logo markup unchanged from the old dock. YouTube has no bg because
  // its image fills the whole card (bleed).
  const APPS = [
    {
      action: 'netflix', name: 'Netflix', bg: '#0a0a0a',
      face: `<svg viewBox="0 0 200 60" xmlns="http://www.w3.org/2000/svg">
          <text x="100" y="38" text-anchor="middle" font-family="Arial, sans-serif" font-weight="800" font-size="26" fill="#E50914" letter-spacing="1">NETFLIX</text>
        </svg>`
    },
    {
      action: 'primevideo', name: 'Prime Video', bg: 'linear-gradient(135deg,#1a2a4a,#00A8E1)',
      face: `<svg viewBox="0 0 200 60" xmlns="http://www.w3.org/2000/svg">
          <text x="100" y="30" text-anchor="middle" font-family="Arial, sans-serif" font-weight="700" font-size="19" fill="#ffffff">prime video</text>
          <path d="M70 40 Q100 52 130 40" stroke="#00A8E1" stroke-width="3" fill="none" stroke-linecap="round"/>
        </svg>`
    },
    {
      action: 'disneyplus', name: 'Disney+', bg: 'linear-gradient(135deg,#0b1d5c,#1b3fa8)',
      face: `<svg viewBox="0 0 200 60" xmlns="http://www.w3.org/2000/svg">
          <text x="88" y="38" text-anchor="middle" font-family="Georgia, serif" font-style="italic" font-weight="700" font-size="24" fill="#ffffff">Disney</text>
          <text x="150" y="38" text-anchor="middle" font-family="Arial, sans-serif" font-weight="800" font-size="24" fill="#ffffff">+</text>
        </svg>`
    },
    {
      action: 'youtube', name: 'YouTube', bg: null, bleed: true,
      face: '<img class="af-cover" src="assets/youtube.jpg" alt="YouTube">'
    },
    {
      action: 'browser', name: 'Browser', bg: '#18181b',
      face: '<img class="af-logo" src="assets/chrome.webp" alt="Browser">'
    },
    {
      action: 'livetv', name: 'Live TV', bg: 'linear-gradient(135deg,#2b2f3a,#111319)',
      face: `<svg viewBox="0 0 200 60" xmlns="http://www.w3.org/2000/svg">
          <rect x="34" y="18" width="30" height="22" rx="4" stroke="#fff" stroke-width="2" fill="none"/>
          <path d="M44 24v10l9-5-9-5z" fill="#fff"/>
          <text x="128" y="38" text-anchor="middle" font-family="Arial, sans-serif" font-weight="700" font-size="18" fill="#ffffff">Live TV</text>
        </svg>`
    },
    {
      action: 'music', name: 'Musik', bg: 'linear-gradient(135deg,#ff5c8a,#7b5cff)',
      face: `<svg viewBox="0 0 200 60" xmlns="http://www.w3.org/2000/svg">
          <g transform="translate(34 12)" fill="none" stroke="#fff" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 30V8l20-4v22"/><circle cx="7" cy="30" r="5" fill="#fff"/><circle cx="27" cy="26" r="5" fill="#fff"/>
          </g>
          <text x="130" y="38" text-anchor="middle" font-family="Arial, sans-serif" font-weight="800" font-size="18" fill="#ffffff">Musik</text>
        </svg>`
    },
    {
      action: 'mediathek', name: 'Mediathek', bg: 'linear-gradient(135deg,#3a2f6e,#7b5fd4)',
      face: `<svg viewBox="0 0 200 60" xmlns="http://www.w3.org/2000/svg">
          <rect x="34" y="16" width="28" height="28" rx="5" fill="#fff" opacity="0.9"/>
          <path d="M44 24v12l10-6-10-6z" fill="#3a2f6e"/>
          <text x="132" y="38" text-anchor="middle" font-family="Arial, sans-serif" font-weight="700" font-size="17" fill="#ffffff">Mediathek</text>
        </svg>`
    }
  ];

  const bgStyle = (app) => (app.bg ? ` style="background:${app.bg};"` : '');
  const faceHtml = (app) => `<div class="af-face${app.bleed ? ' af-face-bleed' : ''}">${app.face}</div>`;

  // Preview on the tile: 3x3 grid like tvOS folders (the ninth slot stays empty).
  document.getElementById('af-plate').innerHTML = APPS
    .map((app) => `<div class="af-mini"${bgStyle(app)}>${faceHtml(app)}</div>`)
    .join('');

  // The overlay is built here rather than in index.html: one shared file less
  // to touch, and the app data stays in a single place.
  const overlay = document.createElement('div');
  overlay.id = 'appfolder-overlay';
  overlay.innerHTML = `
    <div id="af-shell">
      <h1 id="af-title">Apps</h1>
      <div id="af-grid">${APPS.map((app, i) => `
        <div class="af-item" data-i="${i}" style="--i:${i};">
          <div class="af-card"${bgStyle(app)}>${faceHtml(app)}</div>
          <div class="af-name">${app.name}</div>
        </div>`).join('')}
      </div>
    </div>`;
  document.body.appendChild(overlay);

  const items = Array.from(overlay.querySelectorAll('.af-item'));
  let open = false;
  let idx = 0;

  function setFocus(i) {
    items.forEach((el) => el.classList.remove('focused'));
    idx = i;
    items[idx].classList.add('focused');
  }

  // --- The Apple-TV folder move: every mini icon on the tile flies out to its
  // big card when the folder opens, and back into the tile when it closes.
  // Done with fixed-position clones whose box (not scale) is animated, so the
  // logos stay undistorted while going from the square minis to 16:9 cards.
  const EXPO = 'cubic-bezier(.16, 1, .3, 1)';

  function miniRects() {
    const rects = Array.from(document.querySelectorAll('#af-plate .af-mini'), (m) => m.getBoundingClientRect());
    const onScreen = rects.length === items.length && rects[0].width > 0 && rects[0].bottom > 0 && rects[0].top < innerHeight;
    return onScreen ? rects : null;
  }
  const cardRects = () => items.map((it) => it.querySelector('.af-card').getBoundingClientRect());
  const box = (r) => ({ left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px` });

  function fly(from, to, { radiusFrom, radiusTo, duration, stagger, onLanded }) {
    const flights = items.map((item, i) => {
      const clone = item.querySelector('.af-card').cloneNode(true);
      clone.classList.add('af-fly');
      Object.assign(clone.style, box(from[i]), { borderRadius: radiusFrom });
      document.body.appendChild(clone);
      return clone.animate(
        [{ ...box(from[i]), borderRadius: radiusFrom }, { ...box(to[i]), borderRadius: radiusTo }],
        { duration, delay: i * stagger, easing: EXPO, fill: 'both' }
      ).finished.catch(() => {}).then(() => clone);
    });
    Promise.all(flights).then((clones) => {
      if (onLanded) onLanded();
      // One frame later, so the real cards are painted before the clones go.
      requestAnimationFrame(() => clones.forEach((c) => c.remove()));
    });
  }

  function openFolder() {
    if (open) return;
    open = true;
    items.forEach((el) => el.classList.remove('focused'));
    idx = 0;
    const from = miniRects();
    overlay.classList.toggle('flip', Boolean(from));
    overlay.classList.toggle('flying', Boolean(from));
    animatedOpen(overlay);
    if (!from) { setFocus(0); return; }
    // Focus lands after the flight, so the first card gets its lift as the
    // finishing beat instead of arriving pre-scaled.
    fly(from, cardRects(), {
      radiusFrom: '8px', radiusTo: '18px', duration: 640, stagger: 24,
      onLanded: () => { overlay.classList.remove('flying'); if (open) setFocus(idx); }
    });
  }

  // { flyBack: false } when an app is being launched: the app's own zoom
  // (motion.js launch) takes the stage, so the icons must not fly back across it.
  function close({ flyBack = true } = {}) {
    if (!open) return;
    open = false;
    const to = flyBack && overlay.classList.contains('flip') ? miniRects() : null;
    if (to) {
      const from = cardRects();
      overlay.classList.add('flying');
      fly(from, to, { radiusFrom: '18px', radiusTo: '8px', duration: 480, stagger: 14 });
      animatedClose(overlay, { duration: 260, onDone: () => overlay.classList.remove('flying', 'flip') });
    } else {
      animatedClose(overlay, { duration: 220, onDone: () => overlay.classList.remove('flip') });
    }
    // The home focus ring goes back on the "Apps" tile, so after Escape from
    // the folder (and after returning from a launched app) it sits where the
    // user came from.
    setFocusEl(document.getElementById('grid-1'));
  }

  // Close the folder first, then start the app -- otherwise Escape from the
  // app would land on a folder still sitting open over the home screen.
  function launch(i) {
    const app = APPS[i];
    if (!app) return;
    close({ flyBack: false });
    openAction(app.action);
  }

  function handleKey(e) {
    const k = e.key;
    if (k === 'Escape' || k === 'Backspace') { close(); return; }
    // A held Enter (auto-repeat) that opened the tile must not immediately
    // launch the first app.
    if (k === 'Enter') { if (!e.repeat) launch(idx); return; }

    // No wrap-around at the edges: focus just stays put, like tvOS.
    let next = idx;
    if (k === 'ArrowLeft' && idx % COLS > 0) next = idx - 1;
    else if (k === 'ArrowRight' && idx % COLS < COLS - 1 && idx + 1 < items.length) next = idx + 1;
    else if (k === 'ArrowUp' && idx - COLS >= 0) next = idx - COLS;
    else if (k === 'ArrowDown' && idx + COLS < items.length) next = idx + COLS;
    if (next !== idx) setFocus(next);
  }

  // Remote pointer hover: focus the card under the pointer, same as the arrow keys would.
  function pointerHover(el) {
    const item = el.closest('#appfolder-overlay .af-item');
    if (item) setFocus(Number(item.dataset.i));
  }

  // Click (real mouse or the phone pointer's synthetic click) launches the
  // app; a click on the empty backdrop closes the folder.
  overlay.addEventListener('click', (e) => {
    if (!open) return; // still visible during the close animation, but no longer live
    const item = e.target.closest('.af-item');
    if (!item) { close(); return; }
    const i = Number(item.dataset.i);
    setFocus(i);
    launch(i);
  });

  // Real mouse: only focus on actual movement -- otherwise a resting mouse
  // under the freshly opened overlay would steal the initial focus.
  let lastMouseX = -1;
  let lastMouseY = -1;
  overlay.addEventListener('mousemove', (e) => {
    if (Math.abs(e.clientX - lastMouseX) + Math.abs(e.clientY - lastMouseY) < 3) return;
    lastMouseX = e.clientX;
    lastMouseY = e.clientY;
    if (open) pointerHover(e.target);
  });

  window.appFolder = {
    open: openFolder,
    close,
    isOpen: () => open,
    handleKey,
    pointerHover
  };
})();
