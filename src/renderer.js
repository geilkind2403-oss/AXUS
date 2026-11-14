// A branded boot screen instead of dropping straight into the home screen
// (or flashing whatever loads first) -- real TV OSes always show their own
// logo on power-on. It stays until boot.js has preloaded everything the menus
// need ('tv-boot-done'), so nothing loads in front of the user afterwards;
// the timeout only guards against a hung preload.
let splashLifted = false;
function liftSplash() {
  if (splashLifted) return;
  splashLifted = true;
  const splash = document.getElementById('boot-splash');
  if (!splash) return;
  splash.classList.add('hidden');
  setTimeout(() => splash.remove(), 550);
}
window.addEventListener('tv-boot-done', liftSplash, { once: true });
setTimeout(liftSplash, 150000); // boot.js may take up to 2 min (loading + rehearsal)

// A real remote has no pointer -- so the OS mouse cursor only shows while
// the mouse is actually moving, and disappears again shortly after, instead
// of sitting on screen as a permanent "this is a PC" tell. Any key/click
// still works during the hidden state; the cursor just isn't drawn.
let cursorHideTimer = null;
function showCursor() {
  document.body.classList.remove('cursor-hidden');
  if (cursorHideTimer) clearTimeout(cursorHideTimer);
  cursorHideTimer = setTimeout(() => document.body.classList.add('cursor-hidden'), 2500);
}
document.addEventListener('mousemove', showCursor);
document.body.classList.add('cursor-hidden');

// Right-click's default "Inspect Element" context menu is pure desktop-
// browser chrome; a TV UI never shows one.
document.addEventListener('contextmenu', (e) => e.preventDefault());

// Generic remote-friendly navigation: every focusable element declares its
// neighbors via data-nav-up/down/left/right (IDs), so arrow keys can reach
// literally any element on screen, not just one row of tiles.
const navElements = Array.from(document.querySelectorAll('[data-nav-up], [data-nav-down], [data-nav-left], [data-nav-right]'));
let focusedEl = document.getElementById('nav-home');
let contentOpen = false;

function setFocusEl(el) {
  if (!el) return;
  if (focusedEl) focusedEl.classList.remove('focused');
  focusedEl = el;
  focusedEl.classList.add('focused');
}
setFocusEl(focusedEl);

function navigate(direction) {
  const attr = `nav${direction}`;
  const targetId = focusedEl.dataset[attr];
  if (!targetId) return;
  const target = document.getElementById(targetId);
  if (target) setFocusEl(target);
}

// Every overlay opens with a CSS entrance animation (.active). Closing used
// to just strip .active straight back to display:none, cutting off whatever
// was mid-animation and skipping a close animation entirely -- this is what
// made switching in and out of a section feel broken. animatedClose() instead
// holds the overlay visible (.closing) for its exit animation and only drops
// .active once that finishes; animatedOpen() clears any exit animation still
// in flight so a quick close-then-reopen doesn't get cancelled out from under it.
function animatedOpen(overlay) {
  overlay.classList.remove('closing');
  overlay.classList.add('active');
}
function animatedClose(overlay, { duration = 200, onDone } = {}) {
  overlay.classList.add('closing');
  setTimeout(() => {
    if (!overlay.classList.contains('closing')) return; // reopened before the animation finished
    overlay.classList.remove('active', 'closing');
    if (onDone) onDone();
  }, duration);
}

// --- On-screen keyboard: a reusable modal for any text entry this
// remote-driven UI needs (currently: a WLAN password). Always docked
// bottom-center rather than a full-screen takeover, and kept to a single
// case-toggle + a compact symbols row instead of a full multi-layout
// keyboard, on purpose -- it only has to be fast and simple, not complete.
// showKeyboard() resolves with the typed string, or null if cancelled.
const KB_ROWS = [
  ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0'],
  ['q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p'],
  ['a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l'],
  [{ type: 'shift', label: '⇧' }, 'z', 'x', 'c', 'v', 'b', 'n', 'm', { type: 'backspace', label: '⌫' }],
  ['!', '@', '#', '$', '%', '&', '*', '-', '_', '.'],
  [{ type: 'cancel', label: 'Abbrechen' }, { type: 'space', label: 'Leertaste', wide: true }, { type: 'done', label: 'Fertig' }]
];

const keyboardOverlay = document.getElementById('keyboard-overlay');
const keyboardLabelEl = document.getElementById('keyboard-label');
const keyboardPreviewText = document.getElementById('keyboard-preview-text');
let keyboardOpen = false;
let keyboardValue = '';
let keyboardShift = false;
let keyboardSecure = false;
let keyboardResolve = null;
let keyboardFocusEl = null;

(function buildKeyboard() {
  document.getElementById('keyboard-rows').innerHTML = KB_ROWS.map((row) => {
    const keys = row.map((k) => {
      if (typeof k === 'string') {
        return `<button class="kb-key" tabindex="-1" data-type="char" data-char="${k}">${k}</button>`;
      }
      const cls = ['kb-key', k.wide && 'kb-key-wide', (k.type === 'done') && 'kb-key-action', (k.type === 'cancel') && 'kb-key-cancel']
        .filter(Boolean).join(' ');
      return `<button class="${cls}" tabindex="-1" data-type="${k.type}">${k.label}</button>`;
    }).join('');
    return `<div class="kb-row">${keys}</div>`;
  }).join('');
})();

function showKeyboard({ label = '', initialValue = '', secure = false } = {}) {
  return new Promise((resolve) => {
    keyboardValue = initialValue;
    keyboardShift = false;
    keyboardSecure = secure;
    keyboardResolve = resolve;
    keyboardLabelEl.textContent = label;
    updateKeyboardPreview();
    keyboardOpen = true;
    animatedOpen(keyboardOverlay);
    setKeyboardFocus(document.querySelector('#keyboard-rows .kb-key'));
  });
}

function updateKeyboardPreview() {
  keyboardPreviewText.textContent = keyboardSecure ? '•'.repeat(keyboardValue.length) : keyboardValue;
}

function setKeyboardFocus(el) {
  if (!el) return;
  if (keyboardFocusEl) keyboardFocusEl.classList.remove('focused');
  keyboardFocusEl = el;
  keyboardFocusEl.classList.add('focused');
}

function keyboardMoveHorizontal(dir) {
  const keys = Array.from(keyboardFocusEl.closest('.kb-row').querySelectorAll('.kb-key'));
  const next = keys[keys.indexOf(keyboardFocusEl) + dir];
  if (next) setKeyboardFocus(next);
}

// Rows have different key counts/widths, so "up/down" picks whichever key
// in the neighboring row sits closest to the current one horizontally
// (by actual on-screen position), not by array index.
function keyboardMoveVertical(dir) {
  const rows = Array.from(document.querySelectorAll('#keyboard-rows .kb-row'));
  const targetRow = rows[rows.indexOf(keyboardFocusEl.closest('.kb-row')) + dir];
  if (!targetRow) return;
  const currentRect = keyboardFocusEl.getBoundingClientRect();
  const currentCenter = currentRect.left + currentRect.width / 2;
  let best = null, bestDist = Infinity;
  for (const key of targetRow.querySelectorAll('.kb-key')) {
    const r = key.getBoundingClientRect();
    const dist = Math.abs((r.left + r.width / 2) - currentCenter);
    if (dist < bestDist) { bestDist = dist; best = key; }
  }
  setKeyboardFocus(best);
}

function activateKeyboardKey(el) {
  const type = el.dataset.type;
  if (type === 'char') {
    keyboardValue += keyboardShift ? el.dataset.char.toUpperCase() : el.dataset.char;
  } else if (type === 'space') {
    keyboardValue += ' ';
  } else if (type === 'backspace') {
    keyboardValue = keyboardValue.slice(0, -1);
  } else if (type === 'shift') {
    keyboardShift = !keyboardShift;
    document.querySelectorAll('#keyboard-rows .kb-key[data-type="char"]').forEach((k) => {
      if (/^[a-z]$/i.test(k.dataset.char)) k.textContent = keyboardShift ? k.dataset.char.toUpperCase() : k.dataset.char;
    });
    el.classList.toggle('kb-key-action', keyboardShift);
    return; // doesn't change the typed value, no preview update needed
  } else if (type === 'done') {
    closeKeyboard(keyboardValue);
    return;
  } else if (type === 'cancel') {
    closeKeyboard(null);
    return;
  }
  updateKeyboardPreview();
}

function closeKeyboard(result) {
  keyboardOpen = false;
  animatedClose(keyboardOverlay, { duration: 180 });
  if (keyboardResolve) {
    const resolve = keyboardResolve;
    keyboardResolve = null;
    resolve(result);
  }
}

function handleKeyboardKey(e) {
  switch (e.key) {
    case 'Escape': closeKeyboard(null); break;
    case 'ArrowLeft': keyboardMoveHorizontal(-1); break;
    case 'ArrowRight': keyboardMoveHorizontal(1); break;
    case 'ArrowUp': keyboardMoveVertical(-1); break;
    case 'ArrowDown': keyboardMoveVertical(1); break;
    case 'Enter': if (keyboardFocusEl) activateKeyboardKey(keyboardFocusEl); break;
  }
}

document.getElementById('keyboard-rows').addEventListener('click', (e) => {
  const key = e.target.closest('.kb-key');
  if (key) { setKeyboardFocus(key); activateKeyboardKey(key); }
});
keyboardOverlay.addEventListener('click', (e) => {
  if (e.target === keyboardOverlay) closeKeyboard(null);
});

const ACTIONS = {
  youtube: 'https://www.youtube.com/tv',
  netflix: 'https://www.netflix.com',
  primevideo: 'https://www.primevideo.com',
  disneyplus: 'https://www.disneyplus.com',
  browser: 'https://www.google.com'
};

function openAction(action) {
  if (action === 'livetv') {
    openLiveTV();
    return;
  }
  if (action === 'music') {
    if (window.music) window.music.open();
    return;
  }
  if (action === 'mediathek') {
    openLibrary();
    return;
  }
  if (action === 'opensettings') {
    openSettings();
    return;
  }
  if (action === 'weather') {
    if (window.wx) window.wx.open();
    return;
  }
  if (action === 'appfolder') {
    if (window.appFolder) window.appFolder.open();
    return;
  }
  if (action === 'news') {
    contentOpen = true;
    launchContent('https://www.tagesschau.de', () => window.launcher.openUrl('https://www.tagesschau.de'));
    return;
  }
  if (action === 'randommovie') {
    playRandomMovie();
    return;
  }
  const url = ACTIONS[action];
  if (!url) return;
  contentOpen = true;
  launchContent(url, () => window.launcher.openUrl(url, { asSmartTv: action === 'youtube' }));
}

// The launching card zooms into a full-screen brand panel first (motion.js);
// `go` starts the actual load a moment into that zoom. contentOpen is set by
// the caller right away, so keys can't reach the home screen in between.
function launchContent(url, go) {
  if (window.motion && typeof window.motion.launch === 'function') window.motion.launch(url).then(go);
  else go();
}

// Random-movie cover tile (#grid-4): a random film from the local library,
// played on Enter via playRandomMovie().
let pickedRandomMovie = null;

async function refreshRandomMovieCell() {
  pickedRandomMovie = await window.launcher.getRandomMovie();
  const cell = document.getElementById('grid-4');
  if (!cell) return;

  cell.querySelectorAll('.grid-cover-glow, .grid-cover-art, .grid-cover-overlay').forEach((el) => el.remove());

  if (pickedRandomMovie && pickedRandomMovie.icon) {
    const preload = new Image();
    preload.onload = () => {
      cell.classList.add('loaded');

      // Three layers inside the tile, bottom to top: a blurred copy of the
      // cover that spills a little past the tile's edges (the colour halo),
      // the sharp cover itself, then the info scrim. The cover can't stay the
      // cell's own background-image -- any child, even z-index:-1, paints
      // above its parent's background, so the halo would wash out the art.
      const coverUrl = cssUrl(pickedRandomMovie.icon);
      const glow = document.createElement('div');
      glow.className = 'grid-cover-glow';
      glow.style.backgroundImage = coverUrl;
      const art = document.createElement('div');
      art.className = 'grid-cover-art';
      art.style.backgroundImage = coverUrl;
      cell.append(glow, art);

      // Create and add the overlay with metadata and play hint
      const overlay = document.createElement('div');
      overlay.className = 'grid-cover-overlay';

      // Build the metadata chips
      const metaChips = [];
      // The FSK metadata file may already say "FSK 6" -- don't print "FSK FSK 6".
      const fsk = pickedRandomMovie.fsk ? String(pickedRandomMovie.fsk).replace(/^\s*FSK\s*/i, '').trim() : '';
      if (fsk) {
        metaChips.push(`<span class="cover-chip cover-chip-fsk">FSK ${escapeHtml(fsk)}</span>`);
      }
      if (pickedRandomMovie.genre) {
        metaChips.push(`<span class="cover-chip">${escapeHtml(pickedRandomMovie.genre)}</span>`);
      }
      if (pickedRandomMovie.duration) {
        metaChips.push(`<span class="cover-chip">${fmtDuration(pickedRandomMovie.duration)}</span>`);
      }

      overlay.innerHTML = `
        <div class="cover-label">Empfohlen für dich</div>
        <div class="cover-title">${escapeHtml(pickedRandomMovie.title)}</div>
        <div class="cover-meta">${metaChips.join('')}</div>
        <div class="cover-play-hint">▶ Abspielen</div>
      `;
      cell.appendChild(overlay);
    };
    preload.src = pickedRandomMovie.icon;
  }
}
refreshRandomMovieCell();

// --- Small helpers shared by all the classic scripts (weather.js, library.js,
// settings.js, controlcenter.js, continue-tile.js load after this file).
// Text from files and the network goes into innerHTML only through escapeHtml.
function escapeHtml(str) {
  return String(str == null ? '' : str).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}
// CSS url() value for a file:// or http(s) URL (paths may contain quotes).
const cssUrl = (u) => `url("${String(u).replace(/"/g, '%22')}")`;
// "20:15"
const hhmm = (d) => `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
// Film length: "1 Std. 52 Min." / "48 Min." ('' when unknown)
function fmtDuration(sec) {
  if (!sec) return '';
  const h = Math.floor(sec / 3600);
  const m = Math.round((sec % 3600) / 60);
  return h ? `${h} Std. ${m} Min.` : `${m} Min.`;
}

// --- News tile (#grid-6): one lead story that rotates (photo, kicker, title,
// age) plus the next three headlines beside it. Items are
// { title, topline, date, image, url } from main.js getNews(). Wrapped in a
// function scope so its helpers can't collide with globals of the other
// classic scripts. Everything comes from the network, so text only ever goes in
// via textContent and photos via img.src -- never innerHTML.
const refreshNewsCell = (() => {
  const ROTATE_MS = 8000;
  const LIST_COUNT = 3;
  const MAX_ITEMS = 6;
  const $ = (id) => document.getElementById(id);
  const tile = $('grid-6');
  const imgs = [$('nt-img-a'), $('nt-img-b')];
  const text = $('nt-text');
  const kicker = $('nt-kicker');
  const title = $('nt-title');
  const time = $('nt-time');
  const dots = $('nt-dots');
  const list = $('nt-list');
  if (!tile || !imgs[0] || !imgs[1] || !text || !kicker || !title || !time || !dots || !list) return async () => {};
  tile.style.setProperty('--nt-dur', `${ROTATE_MS}ms`);

  let items = [];
  let idx = -1;      // story currently shown as lead
  let shown = 0;     // which of the two <img> layers is visible
  let timer = null;
  let seq = 0;       // bumped per swap so a slow older swap can't land after a newer one
  let retryTimer = null;

  const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

  function relTime(iso) {
    const t = Date.parse(iso);
    if (!Number.isFinite(t)) return '';
    const mins = Math.floor((Date.now() - t) / 60000);
    if (mins < 1) return 'gerade eben';
    if (mins < 60) return `vor ${mins} Min.`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `vor ${hours} Std.`;
    const days = Math.floor(hours / 24);
    return days === 1 ? 'gestern' : `vor ${days} Tagen`;
  }

  // Resolves true once the photo is downloaded and decoded, so the crossfade
  // never stutters or shows a half-painted image. On error/timeout it resolves
  // false and the story is shown without a photo rather than stalling the tile.
  function preload(url) {
    return new Promise((resolve) => {
      if (!url) return resolve(false);
      const img = new Image();
      const giveUp = setTimeout(() => resolve(false), 6000);
      img.onload = () => {
        (img.decode ? img.decode() : Promise.resolve()).catch(() => {}).then(() => { clearTimeout(giveUp); resolve(true); });
      };
      img.onerror = () => { clearTimeout(giveUp); resolve(false); };
      img.src = url;
    });
  }

  function renderList() {
    const n = items.length;
    const rows = [];
    for (let k = 1; k <= Math.min(LIST_COUNT, n - 1); k++) {
      const it = items[(idx + k) % n];
      const li = document.createElement('li');
      const age = relTime(it.date);
      if (age) {
        const t = document.createElement('span');
        t.className = 'nt-li-time';
        t.textContent = age;
        li.appendChild(t);
      }
      const h = document.createElement('span');
      h.className = 'nt-li-title';
      h.textContent = it.title;
      li.appendChild(h);
      rows.push(li);
    }
    list.replaceChildren(...rows);
  }

  function renderDots() {
    const n = items.length > 1 ? items.length : 0;
    if (dots.children.length !== n) {
      dots.replaceChildren(...Array.from({ length: n }, () => {
        const d = document.createElement('span');
        d.className = 'nt-dot';
        d.appendChild(document.createElement('i'));
        return d;
      }));
    }
    Array.from(dots.children).forEach((d, k) => d.classList.toggle('on', k === idx));
  }

  function scheduleNext() {
    clearTimeout(timer);
    if (items.length < 2) return;
    timer = setTimeout(() => {
      // Nobody is looking (web app, Art Mode, standby, window hidden): don't
      // fetch photos for nothing.
      if (homeHidden()) { scheduleNext(); return; }
      showLead((idx + 1) % items.length);
    }, ROTATE_MS);
  }

  async function showLead(i) {
    const mySeq = ++seq;
    clearTimeout(timer);
    const it = items[i];
    // The next photo is fully loaded BEFORE anything on screen changes.
    const hasPhoto = await preload(it.image);
    if (mySeq !== seq) return;
    text.classList.add('swap');
    list.classList.add('swap');
    await wait(230);
    if (mySeq !== seq) return;

    idx = i;
    if (hasPhoto) {
      const incoming = imgs[1 - shown];
      incoming.src = it.image;
      incoming.classList.add('on');
      imgs[shown].classList.remove('on');
      shown = 1 - shown;
    } else {
      imgs[shown].classList.remove('on'); // no photo: fall back to the tile's gradient
    }
    kicker.textContent = it.topline || 'Tagesschau';
    title.textContent = it.title;
    time.textContent = relTime(it.date);
    renderList();
    renderDots();
    text.classList.remove('swap');
    list.classList.remove('swap');
    scheduleNext();
    if (items.length > 1) preload(items[(i + 1) % items.length].image); // warm the next one
  }

  const normalize = (raw) => (Array.isArray(raw) ? raw : [])
    .map((n) => (typeof n === 'string' ? { title: n } : n)) // tolerate the old plain-title format
    .filter((n) => n && typeof n.title === 'string' && n.title.trim())
    .slice(0, MAX_ITEMS);
  const sameStory = (a, b) => (a.url && b.url ? a.url === b.url : a.title === b.title);

  return async function refreshNewsCell() {
    let fresh = [];
    try { fresh = normalize(await window.launcher.getNews()); } catch { /* offline: keep what we have */ }
    if (!fresh.length) {
      if (!items.length) {
        // Nothing to show yet (e.g. booted before the network was up): say so
        // and retry soon instead of sitting on this for the full 5 minutes.
        title.textContent = 'Nachrichten gerade nicht erreichbar';
        clearTimeout(retryTimer);
        retryTimer = setTimeout(refreshNewsCell, 30 * 1000);
      }
      return;
    }
    const current = items[idx];
    const changed = fresh.length !== items.length || fresh.some((n, k) => !sameStory(n, items[k]));
    items = fresh;
    const k = current ? items.findIndex((n) => sameStory(n, current)) : -1;
    if (k < 0) {
      showLead(0); // first load, or the story on screen dropped off the list
    } else if (changed) {
      idx = k;     // same story still on screen, only the surroundings moved
      renderList();
      renderDots();
      scheduleNext();
    } else {
      time.textContent = relTime(items[idx].date); // just keep the ages current
      renderList();
    }
  };
})();
refreshNewsCell();
setInterval(refreshNewsCell, 5 * 60 * 1000);

function playRandomMovie() {
  if (!pickedRandomMovie) return;
  contentOpen = true;
  const movie = pickedRandomMovie;
  launchContent('file:///mediaplayer.html', () => window.launcher.openMedia(
    movie.fullPath,
    movie.relativePath,
    getResumePosition(movie.relativePath)
  ));
}

// Tiles and top-bar nav items with an action open directly on click.
document.querySelectorAll('[data-action]').forEach((item) => {
  item.addEventListener('click', () => {
    setFocusEl(item);
    openAction(item.dataset.action);
  });
});

// Live TV is its own full-screen app (livetv.html with its own channel guide,
// fed by data/livetv/*.m3u); it resumes the last watched channel.
function openLiveTV() {
  contentOpen = true;
  launchContent('file:///livetv.html', () => window.launcher.openLiveTv());
}

// --- WLAN and Bluetooth are Settings categories: their scan / connect / pair
// logic (netsh + bt-helper.exe via main.js, on-screen keyboard for a WLAN
// password) lives in settings.js together with the rest of Settings. ---

// --- Mediathek overlay: folder-tree browser over data/media, with resume
// ("weiterschauen") positions remembered per file. ---
let mediathekOpen = false;
const mediathekOverlay = document.getElementById('mediathek-overlay');
const mediathekList = document.getElementById('mediathek-list');
const mediathekBreadcrumb = document.getElementById('mediathek-breadcrumb');
let mediathekNavItems = [];
let mediathekFocusIndex = 0;
let mediathekCurrentDir = '';
let mediathekEntries = []; // [{type:'up'|'folder'|'file', ...}]

const RESUME_KEY = 'tv-launcher-resume';
function loadResumePositions() {
  try { return JSON.parse(localStorage.getItem(RESUME_KEY)) || {}; } catch { return {}; }
}
function saveResumePosition(relativePath, seconds) {
  const positions = loadResumePositions();
  positions[relativePath] = seconds;
  localStorage.setItem(RESUME_KEY, JSON.stringify(positions));
}
function getResumePosition(relativePath) {
  return loadResumePositions()[relativePath] || 0;
}

async function openMediathek(dir = '') {
  mediathekOpen = true;
  animatedOpen(mediathekOverlay);
  mediathekCurrentDir = dir;
  mediathekBreadcrumb.textContent = 'data/media' + (dir ? '/' + dir.replace(/\\/g, '/') : '');

  const { folders, files } = await window.launcher.listMediaDir(dir);
  const resumePositions = loadResumePositions();

  mediathekEntries = [];
  if (dir) mediathekEntries.push({ type: 'up' });
  for (const name of folders) mediathekEntries.push({ type: 'folder', name });
  for (const f of files) mediathekEntries.push({ type: 'file', ...f });

  if (mediathekEntries.length === 0) {
    mediathekList.innerHTML = '<li class="media-item-empty">Leerer Ordner. Leg Videos oder Unterordner in data/media ab.</li>';
  } else {
    mediathekList.innerHTML = mediathekEntries.map((entry, i) => {
      if (entry.type === 'up') {
        return `<li class="media-item mediathek-nav" tabindex="-1" data-index="${i}"><span class="media-item-icon">↑</span>..</li>`;
      }
      if (entry.type === 'folder') {
        return `<li class="media-item mediathek-nav" tabindex="-1" data-index="${i}"><span class="media-item-icon">📁</span>${escapeHtml(entry.name)}</li>`;
      }
      const resumeSec = resumePositions[entry.relativePath];
      const progressLabel = resumeSec ? `<span class="media-item-progress">weiter ab ${formatTime(resumeSec)}</span>` : '';
      return `<li class="media-item mediathek-nav" tabindex="-1" data-index="${i}"><span class="media-item-icon">▶</span>${escapeHtml(entry.name)}${progressLabel}</li>`;
    }).join('');
  }

  mediathekNavItems = Array.from(document.querySelectorAll('#mediathek-overlay .mediathek-nav'));
  mediathekNavItems.forEach(el => el.classList.remove('focused'));
  mediathekFocusIndex = 0;
  if (mediathekNavItems[0]) mediathekNavItems[0].classList.add('focused');

  // Only the freshly rendered list items need a fresh listener each time --
  // the two static buttons below get theirs once, outside this function.
  mediathekList.querySelectorAll('.mediathek-nav').forEach((el) => {
    el.addEventListener('click', () => activateMediathekItem(el));
  });
}

function formatTime(totalSeconds) {
  const h = Math.floor(totalSeconds / 3600);
  const m = Math.floor((totalSeconds % 3600) / 60);
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}h` : `${m}min`;
}

document.getElementById('mediathek-open-folder').addEventListener('click', () => window.launcher.openMediaFolder());
document.getElementById('mediathek-close').addEventListener('click', () => closeMediathek());

function closeMediathek() {
  mediathekOpen = false;
  animatedClose(mediathekOverlay, { duration: 200 });
}

function setMediathekFocus(i) {
  if (!mediathekNavItems.length) return;
  mediathekNavItems[mediathekFocusIndex].classList.remove('focused');
  mediathekFocusIndex = (i + mediathekNavItems.length) % mediathekNavItems.length;
  mediathekNavItems[mediathekFocusIndex].classList.add('focused');
}

function handleMediathekKey(e) {
  switch (e.key) {
    case 'Escape':
      closeMediathek();
      break;
    case 'Backspace':
      if (mediathekCurrentDir) goToParentMediaDir();
      break;
    case 'ArrowDown':
      setMediathekFocus(mediathekFocusIndex + 1);
      break;
    case 'ArrowUp':
      setMediathekFocus(mediathekFocusIndex - 1);
      break;
    case 'Enter':
      if (mediathekNavItems[mediathekFocusIndex]) activateMediathekItem(mediathekNavItems[mediathekFocusIndex]);
      break;
  }
}

function goToParentMediaDir() {
  const parts = mediathekCurrentDir.split(/[\\/]/).filter(Boolean);
  parts.pop();
  openMediathek(parts.join('/'));
}

function activateMediathekItem(el) {
  if (el.dataset.kind === 'close') {
    closeMediathek();
    return;
  }
  const entry = mediathekEntries[Number(el.dataset.index)];
  if (!entry) return;

  if (entry.type === 'up') {
    goToParentMediaDir();
  } else if (entry.type === 'folder') {
    const nextDir = mediathekCurrentDir ? `${mediathekCurrentDir}/${entry.name}` : entry.name;
    openMediathek(nextDir);
  } else if (entry.type === 'file') {
    closeMediathek();
    contentOpen = true;
    window.launcher.openMedia(entry.fullPath, entry.relativePath, getResumePosition(entry.relativePath));
  }
}

// mediaplayer.html reports its playback position through the main process;
// this is how "weiterschauen" (resume) positions get saved as you watch.
window.launcher.onMediaProgress((relativePath, seconds) => {
  saveResumePosition(relativePath, seconds);
});

mediathekOverlay.addEventListener('click', (e) => {
  if (e.target === mediathekOverlay) closeMediathek();
});

window.launcher.onContentClosed(() => {
  contentOpen = false;
  contentCovering = false;
  notifyHomeVisibility();
});

// --- Home screen out of sight: a web app / film is open, Art Mode or standby
// is up, or the window is hidden. The news rotation, the weather tile's video
// and the Live TV tile (livetv-tile.js) rest meanwhile instead of working for
// nobody. Every change of one of those states calls notifyHomeVisibility(),
// so they react right away instead of polling for it.
let contentCovering = false; // the web app is on screen (not just loading behind the splash)
let artModeShown = false;    // Art Mode has fully faded in over the home screen
const homeVisibilityListeners = [];
function onHomeVisibilityChange(cb) {
  homeVisibilityListeners.push(cb);
}
function notifyHomeVisibility() {
  syncHomeVideo();
  homeVisibilityListeners.forEach((cb) => cb());
}
function homeHidden() {
  return document.hidden || contentOpen || artModeActive ||
    Boolean(window.tvos && window.tvos.standby && window.tvos.standby.isOn());
}

// A menu over the home screen (Settings, weather, Mediathek, app folder,
// Control Center, ...). Its entrance zoom and frosted glass are composited
// over the tiles' videos; with the Live TV preview and the weather video still
// decoding underneath, the GPU dropped to 30 fps and every menu transition
// stuttered. So the videos hold their frame while a menu is up and go on the
// moment it starts closing.
const MENU_OVERLAYS = ['settings-overlay', 'weather-overlay', 'library-overlay', 'library-detail-overlay',
  'mediathek-overlay', 'appfolder-overlay', 'cc-overlay', 'keyboard-overlay', 'music-overlay'];
let menuCovering = false;
function homeCoveredByMenu() {
  return MENU_OVERLAYS.some((id) => {
    const el = document.getElementById(id);
    return el && (el.classList.contains('active') || el.classList.contains('on')) && !el.classList.contains('closing');
  });
}
function watchMenus() {
  const check = () => {
    const now = homeCoveredByMenu();
    if (now === menuCovering) return;
    menuCovering = now;
    notifyHomeVisibility();
  };
  const obs = new MutationObserver(check);
  MENU_OVERLAYS.forEach((id) => {
    const el = document.getElementById(id);
    if (el) obs.observe(el, { attributes: true, attributeFilter: ['class'] });
  });
}
// The Control Center and app folder create their overlays in their own scripts.
if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', watchMenus, { once: true });
else setTimeout(watchMenus, 0);

// The weather tile's video only pauses once nothing of it can be seen any more
// (the web app is up, Art Mode has faded in) and plays again before the home
// screen shows, so it never freezes in view. Standby pauses and resumes every
// video itself (tvos.js).
let homeVideoPaused = false;
function syncHomeVideo() {
  const video = document.getElementById('poster-weather-video');
  if (!video || !video.getAttribute('src')) return;
  if (contentCovering || artModeShown || menuCovering) {
    if (!video.paused) { video.pause(); homeVideoPaused = true; }
  } else if (homeVideoPaused) {
    homeVideoPaused = false;
    video.play().catch(() => {});
  }
}

document.addEventListener('visibilitychange', notifyHomeVisibility);
// tvos.js loads after this file.
document.addEventListener('DOMContentLoaded', () => {
  if (!window.tvos) return;
  window.tvos.on('content-state', (s) => {
    contentCovering = Boolean(s && s.state === 'ready');
    notifyHomeVisibility();
  });
  window.tvos.on('home', () => {
    contentCovering = false;
    notifyHomeVisibility();
  });
}, { once: true });

document.addEventListener('keydown', (e) => {
  if (artModeActive) return; // resetIdleTimer's own listener exits art mode; swallow this keypress
  if (keyboardOpen) {
    handleKeyboardKey(e);
    return;
  }
  if (settingsOpen) {
    handleSettingsKey(e);
    return;
  }
  if (window.wx && window.wx.isOpen()) {
    window.wx.handleKey(e);
    return;
  }
  if (window.appFolder && window.appFolder.isOpen()) {
    window.appFolder.handleKey(e);
    return;
  }
  if (mediathekOpen) {
    handleMediathekKey(e);
    return;
  }
  if (window.music && window.music.isOpen()) {
    window.music.handleKey(e);
    return;
  }
  if (libraryDetailOpen) {
    handleLibraryDetailKey(e);
    return;
  }
  if (libraryOpen) {
    handleLibraryKey(e);
    return;
  }
  if (contentOpen) return; // content view has its own focus while open

  switch (e.key) {
    case 'ArrowDown':
      navigate('Down');
      break;
    case 'ArrowUp':
      navigate('Up');
      break;
    case 'ArrowLeft':
      navigate('Left');
      break;
    case 'ArrowRight':
      navigate('Right');
      break;
    case 'Enter':
      if (focusedEl.id === 'btn-settings') {
        openSettings();
      } else if (focusedEl.dataset.action) {
        openAction(focusedEl.dataset.action);
      } else {
        // Plain buttons (transport controls, etc.) without a data-action just get clicked.
        focusedEl.click();
      }
      break;
  }
});

// --- Settings (Einstellungen): all logic lives in settings.js (loaded right
// after this file). Only the state that code in this file reads -- the
// keydown router, anyOverlayOpen() and the phone-remote pointer at the
// bottom -- is declared here, so it exists even before settings.js runs.
// settings.js defines openSettings, closeSettings, handleSettingsKey,
// setSettingsFocus and activateSettingsRow (global functions), and keeps
// settingsNavItems / settingsFocusIndex up to date. ---
let settingsOpen = false;
let settingsNavItems = [];
let settingsFocusIndex = 0;

// Persisted preferences (localStorage). prefs is read at load time by the
// idle timer and the remote pointer, so it stays here rather than in settings.js.
const PREFS_KEY = 'tv-launcher-prefs';
// notifications: system toasts (tvos.js) on/off. keepAwake: the display never
// sleeps by itself while the launcher runs (tvos-main.js, powerSaveBlocker).
// picture / pictureCustom: Einstellungen > Bild (picture.js fills in and
// checks the values; null = Bildmodus Standard, i.e. no filter).
const DEFAULT_PREFS = { adblock: true, fullscreen: true, artmodeTimeout: 30, remoteSensitivity: 50, remoteInvertY: false, touchpadSpeed: 2, notifications: true, keepAwake: true, picture: null, pictureCustom: null };
function loadPrefs() {
  try {
    return { ...DEFAULT_PREFS, ...JSON.parse(localStorage.getItem(PREFS_KEY)) };
  } catch {
    return { ...DEFAULT_PREFS };
  }
}
function savePrefs(prefs) {
  localStorage.setItem(PREFS_KEY, JSON.stringify(prefs));
}
let prefs = loadPrefs();

// (openSettings / closeSettings, the gear + WLAN buttons, the toggles and
// every setting's change listener live in settings.js.)

// --- Clock (bottom bar) ---
function tickClock() {
  const now = new Date();
  document.getElementById('clock-time').textContent = hhmm(now);

  // German date format: "Di, 29. Sep."
  const dayNames = ['So', 'Mo', 'Di', 'Mi', 'Do', 'Fr', 'Sa'];
  const monthNames = ['Jan', 'Feb', 'Mär', 'Apr', 'Mai', 'Jun', 'Jul', 'Aug', 'Sep', 'Okt', 'Nov', 'Dez'];
  const dayName = dayNames[now.getDay()];
  const date = now.getDate();
  const monthName = monthNames[now.getMonth()];
  document.getElementById('clock-ampm').textContent = `${dayName}, ${date}. ${monthName}`;
}
tickClock();
setInterval(tickClock, 1000 * 30);

// --- Weather widget: IP-based location (no OS geolocation permission prompt,
// works headless/kiosk) + Open-Meteo for the actual forecast, both free/keyless. ---
// Murr, Baden-Württemberg -- fixed on purpose (IP-based geolocation was
// pointing at the wrong town, since it only reflects the ISP's routing
// point, not the actual device location).
const MURR_LAT = 48.9744;
const MURR_LON = 9.3714;

// WMO weather codes (used by Open-Meteo): 0-3 = clear/partly cloudy,
// everything with drizzle/rain/showers/thunder counts as "rainy" for
// picking which background video to loop.
function weatherCodeToCondition(code) {
  if (code === 0 || code === 1 || code === 2 || code === 3) return 'sunny';
  const rainyCodes = [51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 80, 81, 82, 95, 96, 99];
  if (rainyCodes.includes(code)) return 'rainy';
  return 'sunny'; // snow/fog etc: no dedicated video yet, default to sunny
}

let currentWeatherVideoCondition = null;
async function updateWeatherTileVideo(condition) {
  if (condition === currentWeatherVideoCondition) return;
  const video = document.getElementById('poster-weather-video');
  if (!video) return;
  const src = await window.launcher.getWeatherVideo(condition);
  if (src) {
    video.src = src;
    currentWeatherVideoCondition = condition;
    video.addEventListener('loadeddata', () => video.classList.add('loaded'), { once: true });
  }
}

// Fills the glass bar of the home tile (#grid-5); wxIcon returns our own static SVG.
function renderWeatherTile(w) {
  const bar = document.getElementById('wt-bar');
  if (!bar) return;
  document.getElementById('wt-icon').innerHTML = window.wxIcon(w.code, w.isDay);
  document.getElementById('wt-temp').textContent = `${w.temp}°`;
  document.getElementById('wt-hilo').textContent = `H ${w.hi}° · T ${w.lo}°`;
  document.getElementById('wt-line').textContent = w.line;
  bar.classList.add('ready');
}

// Tile and top-bar pill both read window.wx.homeSummary() (weather.js), which
// shares its cached forecast with the weather menu. weather.js loads AFTER this
// file, so the first call waits for DOMContentLoaded (= all classic scripts ran).
let weatherRetryTimer = null;
async function loadWeather() {
  document.getElementById('weather-city').textContent = 'Murr';
  clearTimeout(weatherRetryTimer);
  try {
    const w = await window.wx.homeSummary();
    document.getElementById('weather-temp').innerHTML =
      `${w.temp}°<span class="feels" id="weather-feels">Gefühlt ${w.feels}°</span>`;
    // The real current condition instead of a permanent sun (icon set
    // lives in weather.js).
    const pillIcon = document.querySelector('#weather .weather-icon');
    if (window.wxIcon && pillIcon) pillIcon.innerHTML = window.wxIcon(w.code, w.isDay);

    renderWeatherTile(w);
    updateWeatherTileVideo(weatherCodeToCondition(w.code));
  } catch {
    document.getElementById('weather-temp').textContent = '–°';
    // No network yet (TV booting) or API hiccup: retry soon instead of leaving
    // the pill empty for the full 30 minutes.
    weatherRetryTimer = setTimeout(loadWeather, 2 * 60 * 1000);
    const bar = document.getElementById('wt-bar');
    if (bar && !bar.classList.contains('ready')) {
      document.getElementById('wt-line').textContent = 'Wetter nicht verfügbar';
      bar.classList.add('ready');
    }
  }
}

if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', loadWeather, { once: true });
else loadWeather();
// Every 10 min (the weather data cache's own lifetime), so a tile line like
// "Regen in etwa 20 Min." can't go stale for half an hour.
setInterval(loadWeather, 10 * 60 * 1000);

// --- Art Mode: idle screensaver that loops a local video, skipping the
// first 10s and the last 60s (so it never shows the opening titles/credits). ---
const artmodeOverlay = document.getElementById('artmode-overlay');
const artmodeVideo = document.getElementById('artmode-video');
const ART_MODE_START = 10;
const ART_MODE_END_MARGIN = 60;

let idleTimer = null;
let artModeActive = false;
let artModeClosing = false; // true for the whole fade-out; artModeActive alone would flip early and let keys leak to the grid while the overlay is still visible
let artModeShownTimer = null;

function anyOverlayOpen() {
  return contentOpen || settingsOpen || mediathekOpen || libraryOpen || libraryDetailOpen || keyboardOpen ||
    Boolean(window.wx && window.wx.isOpen()) || Boolean(window.appFolder && window.appFolder.isOpen()) ||
    Boolean(window.music && window.music.isOpen());
}

async function enterArtMode() {
  if (anyOverlayOpen() || artModeActive) return;
  const src = await window.launcher.getArtModeVideo();
  if (!src) return; // no data/artmode.mp4 yet -- silently skip

  artModeActive = true;
  artmodeVideo.src = src;
  animatedOpen(artmodeOverlay);
  notifyHomeVisibility();
  // The home screen is fully covered once the fade-in (.8s) is over.
  clearTimeout(artModeShownTimer);
  artModeShownTimer = setTimeout(() => {
    if (!artModeActive || artModeClosing) return;
    artModeShown = true;
    syncHomeVideo();
  }, 850);

  artmodeVideo.addEventListener('loadedmetadata', function onceReady() {
    artmodeVideo.removeEventListener('loadedmetadata', onceReady);
    artmodeVideo.currentTime = ART_MODE_START;
    artmodeVideo.play().catch(() => {}); // interrupted by a quick exit: nothing to do
  });
}

function exitArtMode() {
  if (!artModeActive || artModeClosing) return;
  artModeClosing = true;
  clearTimeout(artModeShownTimer);
  artModeShown = false;
  syncHomeVideo(); // playing again before the fade-out reveals the tile
  // Keep the video playing through the fade-out instead of cutting it off,
  // so the screensaver dissolves rather than jump-cutting to the home screen.
  // artModeActive stays true (keeping keydown's guard swallowing keys) until
  // onDone, i.e. until the overlay is actually gone from the screen.
  animatedClose(artmodeOverlay, {
    duration: 400,
    onDone: () => {
      artModeActive = false;
      artModeClosing = false;
      artmodeVideo.pause();
      artmodeVideo.removeAttribute('src');
      artmodeVideo.load();
      notifyHomeVisibility();
    }
  });
}

artmodeVideo.addEventListener('timeupdate', () => {
  if (!artmodeVideo.duration) return;
  if (artmodeVideo.currentTime >= artmodeVideo.duration - ART_MODE_END_MARGIN) {
    artmodeVideo.currentTime = ART_MODE_START;
  }
});

function resetIdleTimer() {
  if (idleTimer) clearTimeout(idleTimer);
  if (artModeActive) exitArtMode();
  idleTimer = setTimeout(enterArtMode, prefs.artmodeTimeout * 1000);
}

['keydown', 'mousemove', 'mousedown'].forEach((evt) => {
  document.addEventListener(evt, resetIdleTimer);
});
resetIdleTimer();

// --- Library (Mediathek home + film page): see library.js, which owns these
// two flags (declared here so the key router can read them before it loads). ---
let libraryOpen = false;
let libraryDetailOpen = false;

// --- Gyro-Fernbedienung: the phone (remote.html, served by
// remote-server.js) streams rotation deltas in degrees; this turns them into
// an on-screen pointer. Hovering a tile/row focuses it exactly like the
// arrow keys would (LG-Magic-Remote style), so the pointer and the D-pad
// never disagree about what's selected, and "OK" reuses each screen's own
// Enter handling instead of duplicating it. ---
const remotePointer = document.getElementById('remote-pointer');
let pointerX = window.innerWidth / 2;
let pointerY = window.innerHeight / 2;
let pointerHideTimer = null;
let lastIdleReset = 0;
let lastHoverEl = null;

function drawPointer() {
  remotePointer.style.transform = `translate3d(${pointerX}px, ${pointerY}px, 0)`;
  // The focused tile tilts toward the pointer (motion.js).
  if (window.motion) window.motion.pointer(pointerX, pointerY);
}
drawPointer();

function wakePointer() {
  remotePointer.classList.add('visible');
  if (pointerHideTimer) clearTimeout(pointerHideTimer);
  pointerHideTimer = setTimeout(() => remotePointer.classList.remove('visible'), 3500);
  // Keep Art Mode away while the remote is in use -- throttled, since
  // deltas arrive ~60x/s and the timer only needs a nudge now and then.
  const now = performance.now();
  if (now - lastIdleReset > 500) {
    lastIdleReset = now;
    resetIdleTimer();
  }
}

// Feedback whenever the pointer lands on something new: the pointer snaps
// (a short pulse) and the phone gives a tiny vibration tick.
let pointerFeedbackAt = 0;
function pointerFeedback() {
  remotePointer.classList.remove('snap');
  void remotePointer.offsetWidth;
  remotePointer.classList.add('snap');
  const now = performance.now();
  if (now - pointerFeedbackAt < 60) return; // a fast sweep across a row: not a buzz
  pointerFeedbackAt = now;
  if (window.launcher.remoteFeedback) window.launcher.remoteFeedback('tick');
}

function hoverFocusAt(x, y) {
  const el = document.elementFromPoint(x, y);
  if (el === lastHoverEl) return;
  lastHoverEl = el;
  if (!el) return;
  const before = [...document.querySelectorAll('.focused')];
  hoverFocusOn(el);
  const after = [...document.querySelectorAll('.focused')];
  if (after.length && (after.length !== before.length || after.some((f, i) => f !== before[i]))) pointerFeedback();
}

function hoverFocusOn(el) {
  // Control Center sits above every screen, so it gets the hover first.
  if (window.controlCenter && window.controlCenter.isOpen()) {
    window.controlCenter.pointerHover(el);
  } else if (settingsOpen) {
    const row = el.closest('.settings-nav');
    const i = row ? settingsNavItems.indexOf(row) : -1;
    if (i >= 0 && i !== settingsFocusIndex) setSettingsFocus(i);
  } else if (window.music && window.music.isOpen()) {
    window.music.pointerHover(el);
  } else if (window.library && window.library.isOpen()) {
    window.library.pointerHover(el);
  } else if (window.wx && window.wx.isOpen()) {
    window.wx.pointerHover(el);
  } else if (window.appFolder && window.appFolder.isOpen()) {
    window.appFolder.pointerHover(el);
  } else if (!anyOverlayOpen()) {
    const nav = navElements.find((n) => n === el || n.contains(el));
    if (nav && nav !== focusedEl) setFocusEl(nav);
  }
}

function remoteClick() {
  wakePointer();
  remotePointer.classList.remove('clicking');
  void remotePointer.offsetWidth; // restart the pulse animation
  remotePointer.classList.add('clicking');

  const el = document.elementFromPoint(pointerX, pointerY);
  if (!el) return;
  const stepper = el.closest('.stepper-arrow');
  if (stepper) { stepper.click(); return; }

  if (settingsOpen) {
    const row = el.closest('.settings-nav');
    if (row && settingsNavItems.includes(row)) {
      setSettingsFocus(settingsNavItems.indexOf(row));
      activateSettingsRow(row);
      return;
    }
  } else if (!anyOverlayOpen()) {
    const nav = navElements.find((n) => n === el || n.contains(el));
    if (nav) {
      setFocusEl(nav);
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
      return;
    }
  }
  el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, clientX: pointerX, clientY: pointerY }));
}

function movePointerBy(dx, dy) {
  pointerX = Math.min(window.innerWidth, Math.max(0, pointerX + dx));
  pointerY = Math.min(window.innerHeight, Math.max(0, pointerY + dy));
  drawPointer();
  wakePointer();
  hoverFocusAt(pointerX, pointerY);
}

function recenterPointer() {
  pointerX = window.innerWidth / 2;
  pointerY = window.innerHeight / 2;
  drawPointer();
  wakePointer();
  hoverFocusAt(pointerX, pointerY);
}

// Two-finger touchpad scroll: scrolls whatever scrollable container is
// under the pointer (Settings list, Mediathek, Library grid, ...).
function remoteScroll(dy) {
  wakePointer();
  let el = document.elementFromPoint(pointerX, pointerY);
  while (el && el !== document.documentElement) {
    const oy = getComputedStyle(el).overflowY;
    if ((oy === 'auto' || oy === 'scroll') && el.scrollHeight > el.clientHeight) {
      el.scrollTop += dy * 2.5;
      return;
    }
    el = el.parentElement;
  }
}

window.launcher.onRemoteMessage((m) => {
  switch (m.t) {
    case 'd': // gyro, degrees -> px
      movePointerBy(m.x * prefs.remoteSensitivity, m.y * prefs.remoteSensitivity * (prefs.remoteInvertY ? -1 : 1));
      break;
    case 'm': // touchpad, phone px -> px
      movePointerBy(m.x * prefs.touchpadSpeed, m.y * prefs.touchpadSpeed);
      break;
    case 's':
      remoteScroll(m.y);
      break;
    case 'c':
      remoteClick();
      break;
    case 'r':
      recenterPointer();
      break;
    case 'cc':
      resetIdleTimer();
      if (window.controlCenter) window.controlCenter.open();
      break;
    case 'k':
      // The phone's music buttons go straight to the player: no key event,
      // so they neither wake a dark screen nor move any focus.
      if (/^Media/.test(m.k) && window.music) {
        if (m.k === 'MediaPlayPause') window.music.toggle();
        else if (m.k === 'MediaTrackNext') window.music.next();
        else window.music.prev();
        break;
      }
      // Goes through the exact same keydown path as a real remote/keyboard,
      // so every overlay's own Escape/arrow handling just works.
      resetIdleTimer();
      document.dispatchEvent(new KeyboardEvent('keydown', { key: m.k, bubbles: true }));
      break;
  }
});
