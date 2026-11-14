// --- "Weiterschauen" tile (#grid-3): the film you started last, with its
// progress ("noch 48 Min."); OK resumes it where you stopped. Films from this
// laptop and from the media server both count -- their positions come from
// the same resume store the Mediathek uses (renderer.js RESUME_KEY), with the
// server's own position as fallback for films last watched elsewhere.
// Nothing started (or everything watched to the end) -> a film tip instead.
//
// Loaded after renderer.js (loadResumePositions, launchContent, contentOpen,
// pickedRandomMovie, setFocusEl, cssUrl).
(() => {
  const g = (fn) => { try { return fn(); } catch { return undefined; } };
  const cell = document.getElementById('grid-3');
  if (!cell) return;

  const LAST_KEY = 'tv-launcher-lastwatched'; // relativePath -> ms of the last progress report
  const MIN_POS = 60;      // under a minute is just peeking, not "started"
  const END_MARGIN = 180;  // the last 3 minutes are credits: counts as watched

  cell.classList.remove('grid-cell-placeholder');
  cell.classList.add('grid-cell-continue');
  cell.dataset.action = 'continue';
  cell.innerHTML = `
    <div class="ct-art" id="ct-art"></div>
    <div class="ct-scrim"></div>
    <div class="ct-info">
      <div class="ct-label" id="ct-label">Weiterschauen</div>
      <div class="ct-title" id="ct-title"></div>
      <div class="ct-progress" id="ct-progress"><i id="ct-fill"></i></div>
      <div class="ct-meta" id="ct-meta"></div>
      <div class="ct-hint" id="ct-hint"></div>
    </div>`;
  const $ = (id) => document.getElementById(id);

  const readLast = () => { try { return JSON.parse(localStorage.getItem(LAST_KEY)) || {}; } catch { return {}; } };
  const writeLast = (map) => { try { localStorage.setItem(LAST_KEY, JSON.stringify(map)); } catch { /* storage full/blocked */ } };

  // Every progress report stamps the film as "watched just now"; the end
  // of a film reports 0 (mediaplayer.html), which drops it from the tile.
  window.launcher.onMediaProgress((relativePath, seconds) => {
    const map = readLast();
    if (seconds > 0) map[relativePath] = Date.now();
    else delete map[relativePath];
    writeLast(map);
  });

  function fmtLeft(sec) {
    const m = Math.max(1, Math.round(sec / 60));
    return m >= 60 ? `noch ${Math.floor(m / 60)} Std. ${m % 60} Min.` : `noch ${m} Min.`;
  }
  function fmtAt(sec) {
    const h = Math.floor(sec / 3600);
    const m = Math.floor((sec % 3600) / 60);
    return h ? `weiter ab ${h}:${String(m).padStart(2, '0')} Std.` : `weiter ab ${m} Min.`;
  }

  let current = null; // { film, pos } -- pos 0 = film tip
  let seq = 0;

  async function refresh() {
    const mySeq = ++seq;
    let films = [];
    try { films = await window.launcher.listLibrary(); } catch { /* keep what is shown */ }
    if (mySeq !== seq || !Array.isArray(films) || !films.length) return;

    const local = g(() => loadResumePositions()) || {};
    const last = readLast();
    const started = films.map((film) => {
      const pos = Object.prototype.hasOwnProperty.call(local, film.relativePath)
        ? Number(local[film.relativePath]) || 0
        : (film.remote ? Number(film.position) || 0 : 0);
      return { film, pos, at: last[film.relativePath] || 0 };
    }).filter(({ film, pos }) => pos >= MIN_POS && (!film.duration || pos < film.duration - END_MARGIN));
    started.sort((a, b) => b.at - a.at);

    let next = started[0] || null;
    if (!next) {
      // Film tip: anything but the film the big cover tile already suggests.
      const other = g(() => pickedRandomMovie);
      const pool = films.filter((f) => !other || f.relativePath !== other.relativePath);
      const keep = current && !current.pos && pool.find((f) => f.relativePath === current.film.relativePath);
      const film = keep || pool[Math.floor(Math.random() * pool.length)] || films[0];
      next = { film, pos: 0 };
    }
    render(next);

    // Don't let the big cover tile recommend the very film shown here: re-roll it.
    for (let i = 0; i < 3 && next.pos > 0; i++) {
      const other = g(() => pickedRandomMovie);
      if (!other || other.relativePath !== next.film.relativePath || films.length < 2) break;
      await g(() => refreshRandomMovieCell());
    }
  }

  function render(next) {
    const same = current && current.film.relativePath === next.film.relativePath && current.pos === next.pos;
    current = next;
    const { film, pos } = next;
    const resume = pos > 0;
    cell.classList.toggle('ct-resume', resume);
    $('ct-label').textContent = resume ? 'Weiterschauen' : 'Film-Tipp';
    $('ct-title').textContent = film.title || '';
    $('ct-hint').textContent = resume ? '▶ Weiterschauen' : '▶ Abspielen';
    if (resume && film.duration) {
      $('ct-fill').style.width = `${Math.min(100, (pos / film.duration) * 100).toFixed(1)}%`;
      $('ct-meta').textContent = fmtLeft(film.duration - pos);
    } else if (resume) {
      $('ct-fill').style.width = '0%';
      $('ct-meta').textContent = fmtAt(pos);
    } else {
      const bits = [film.genre, film.duration ? `${Math.round(film.duration / 60)} Min.` : ''].filter(Boolean);
      $('ct-meta').textContent = bits.join(' · ');
    }
    if (same) return;

    // Wide tile: the banner fits it; the portrait cover is the fallback.
    const src = film.banner || film.icon;
    const art = $('ct-art');
    if (!src) { art.style.backgroundImage = ''; cell.classList.add('loaded'); return; }
    const img = new Image();
    img.onload = () => {
      if (current !== next) return;
      art.style.backgroundImage = cssUrl(src);
      art.classList.toggle('ct-portrait', !film.banner);
      cell.classList.add('loaded');
    };
    img.src = src;
  }

  function play() {
    if (!current) return;
    const { film, pos } = current;
    contentOpen = true;
    launchContent('file:///mediaplayer.html', () => window.launcher.openMedia(film.fullPath, film.relativePath, pos));
  }

  // Enter on the focused tile goes through openAction(); the tile had no
  // data-action when renderer.js wired its click listeners, so add ours.
  const baseOpenAction = window.openAction;
  window.openAction = function openAction(action) {
    if (action === 'continue') { play(); return undefined; }
    return baseOpenAction.apply(this, arguments);
  };
  cell.addEventListener('click', () => { g(() => setFocusEl(cell)); play(); });

  // Back from a film (new position) -> update; also now and then, since the
  // server may have been watched from somewhere else.
  window.launcher.onContentClosed(() => setTimeout(refresh, 800));
  setInterval(refresh, 10 * 60 * 1000);
  // A moment after start, so the big cover tile has picked its film (the tip avoids it).
  setTimeout(refresh, 1200);

  window.continueTile = { refresh, get current() { return current; } };
})();
