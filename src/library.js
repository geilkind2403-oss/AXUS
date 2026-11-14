// --- Mediathek (library home) + film page. Films from this laptop and from
// the media server (main.js listLibrary). TV-OS layout: the focused film fills
// the backdrop and the info area at the top (like Apple TV), rows of cards
// below; OK opens the full-screen film page with Weiterschauen / Von vorne /
// Aus Weiterschauen entfernen and "Mehr wie das".
//
// Globals shared with renderer.js's key router and overlay checks:
// libraryOpen / libraryDetailOpen (declared in renderer.js), openLibrary,
// closeLibrary, closeLibraryDetail, handleLibraryKey, handleLibraryDetailKey.
// Loaded right after renderer.js.

const library = (() => {
  const $ = (id) => document.getElementById(id);
  const overlay = $('library-overlay');
  const detail = $('library-detail-overlay');

  const MIN_POS = 60;     // under a minute = just peeked
  const END_MARGIN = 180; // last 3 minutes = credits, counts as watched
  const LAST_KEY = 'tv-launcher-lastwatched';

  let items = [];
  let rows = [];       // [{ el, cards: [el] }] -- row 0 is the top bar buttons
  let row = 0;
  let col = [];        // remembered column per row
  let current = null;  // film shown in the detail page
  let dRows = [];      // detail focus rows: [buttons], [more cards]
  let dRow = 0;
  let dCol = [0, 0];
  let loadSeq = 0;

  // ------------------------------------------------------------ data helpers
  const esc = escapeHtml; // shared helpers: renderer.js (so is cssUrl)
  const readLast = () => { try { return JSON.parse(localStorage.getItem(LAST_KEY)) || {}; } catch { return {}; } };

  function position(item) {
    const local = loadResumePositions();
    if (Object.prototype.hasOwnProperty.call(local, item.relativePath)) return Number(local[item.relativePath]) || 0;
    return item.remote ? Number(item.position) || 0 : 0;
  }
  function resumable(item) {
    const pos = position(item);
    return pos >= MIN_POS && (!item.duration || pos < item.duration - END_MARGIN);
  }
  // Info files end with "Genre: a, b" / "Erscheinungsjahr: 2019" lines.
  function parseInfo(info) {
    const text = String(info || '');
    const genres = (/Genre:\s*([^\n]+)/i.exec(text) || [])[1];
    const year = (/Erscheinungsjahr:\s*(\d{4})/i.exec(text) || [])[1];
    const plot = text.replace(/^\s*(Genre|Erscheinungsjahr):.*$/gim, '').trim();
    return { plot, year: year || '', genres: genres ? genres.split(',').map((g) => g.trim()).filter(Boolean) : [] };
  }
  const genresOf = (item) => { const g = parseInfo(item.info).genres; return g.length ? g : (item.genre ? [item.genre] : []); };
  const fsk = (item) => (item.fsk ? String(item.fsk).replace(/^\s*FSK\s*/i, '').trim() : '');
  const fmtDur = fmtDuration; // renderer.js
  function fmtClock(sec) {
    const h = Math.floor(sec / 3600);
    const m = Math.floor((sec % 3600) / 60);
    const s = Math.floor(sec % 60);
    return h ? `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}` : `${m}:${String(s).padStart(2, '0')}`;
  }
  const leftText = (item, pos) => (item.duration ? `noch ${fmtDur(item.duration - pos)}` : `ab ${fmtClock(pos)}`);
  function hueFor(title) {
    let h = 0;
    for (let i = 0; i < title.length; i++) h = title.charCodeAt(i) + ((h << 5) - h);
    return Math.abs(h) % 360;
  }
  const fallbackArt = (item) => `linear-gradient(160deg, hsl(${hueFor(item.title)},55%,42%), hsl(${(hueFor(item.title) + 40) % 360},50%,18%))`;

  // ------------------------------------------------------------ markup
  overlay.innerHTML = `
    <div class="lib-bd"><div class="lib-bd-img"></div><div class="lib-bd-img"></div></div>
    <div class="lib-bd-shade"></div>
    <header id="lib-top">
      <div class="lib-brand"><h1>Mediathek</h1><span id="lib-count"></span></div>
      <div id="lib-top-actions" class="lib-row-items">
        <button class="lib-btn" type="button" data-act="folders">Ordner</button>
        <button class="lib-btn" type="button" data-act="close">Schließen</button>
      </div>
    </header>
    <section id="lib-hero">
      <div id="lib-hero-kicker"></div>
      <h2 id="lib-hero-title"></h2>
      <div id="lib-hero-meta" class="lib-meta"></div>
      <p id="lib-hero-plot"></p>
    </section>
    <div id="lib-rows"></div>
    <div id="lib-empty">
      <b>Noch keine Filme</b>
      <span>Leg Filme in data/media ab oder verbinde den Medienserver in den Einstellungen.</span>
    </div>`;

  detail.innerHTML = `
    <div id="library-detail-panel">
      <div class="ld-bd"></div>
      <div class="ld-shade"></div>
      <div class="ld-content">
        <div class="ld-poster" id="ld-poster"></div>
        <div class="ld-main">
          <div class="ld-kicker" id="ld-kicker"></div>
          <h2 id="ld-title"></h2>
          <div class="lib-meta" id="ld-meta"></div>
          <div class="ld-progress" id="ld-progress"><div class="ld-bar"><i id="ld-bar-fill"></i></div><span id="ld-left"></span></div>
          <div class="ld-actions lib-row-items" id="ld-actions"></div>
          <p class="ld-plot" id="ld-plot"></p>
          <div class="ld-credits" id="ld-credits"></div>
        </div>
      </div>
      <section class="ld-more" id="ld-more">
        <h3>Mehr wie das</h3>
        <div class="lib-track"><div class="lib-strip" id="ld-more-strip"></div></div>
      </section>
    </div>`;

  // ------------------------------------------------------------ backdrop
  const bdImgs = overlay.querySelectorAll('.lib-bd-img');
  let bdShown = 0;
  let bdUrl = null;
  function setBackdrop(item) {
    const url = item ? (item.banner || item.icon) : null;
    if (url === bdUrl) return;
    bdUrl = url;
    const show = () => {
      if (bdUrl !== url) return;
      const next = bdImgs[1 - bdShown];
      next.style.backgroundImage = url ? cssUrl(url) : '';
      next.classList.add('on');
      bdImgs[bdShown].classList.remove('on');
      bdShown = 1 - bdShown;
    };
    if (!url) { show(); return; }
    const img = new Image();
    img.onload = show;
    img.onerror = show;
    img.src = url;
  }

  function metaChips(item, { source = false } = {}) {
    const bits = [];
    const f = fsk(item);
    if (f) bits.push(`<span class="lib-chip lib-chip-fsk">FSK ${esc(f)}</span>`);
    const { year } = parseInfo(item.info);
    if (year) bits.push(`<span>${esc(year)}</span>`);
    const g = genresOf(item).slice(0, 3);
    if (g.length) bits.push(`<span>${esc(g.join(' · '))}</span>`);
    if (item.duration) bits.push(`<span>${fmtDur(item.duration)}</span>`);
    if (source) bits.push(`<span class="lib-src">${item.remote ? 'PC-Server' : 'Dieser Laptop'}</span>`);
    return bits.join('<i class="lib-dot"></i>');
  }

  // ------------------------------------------------------------ overview
  function cardHtml(item, kind) {
    const idx = items.indexOf(item);
    const art = kind === 'wide' ? (item.banner || item.icon) : (item.icon || item.banner);
    const bg = art ? `background-image:${esc(cssUrl(art))}` : `background:${fallbackArt(item)}`;
    let extra = '';
    if (kind === 'wide') {
      const pos = position(item);
      const pct = item.duration ? Math.min(100, (pos / item.duration) * 100) : 0;
      extra = `<div class="lib-card-info"><b>${esc(item.title)}</b><span>${esc(leftText(item, pos))}</span>
        <div class="lib-card-bar"><i style="width:${pct.toFixed(1)}%"></i></div></div>`;
    } else if (!art) {
      extra = `<div class="lib-card-fallback">${esc(item.title)}</div>`;
    }
    return `<div class="lib-card lib-card-${kind}" data-idx="${idx}">
      <div class="lib-card-art" style="${bg}">${extra}</div>
      ${kind === 'wide' ? '' : `<div class="lib-card-title">${esc(item.title)}</div>`}
    </div>`;
  }

  function buildRows() {
    const last = readLast();
    const byTitle = [...items].sort((a, b) => a.title.localeCompare(b.title, 'de'));
    const started = items.filter(resumable).sort((a, b) => (last[b.relativePath] || 0) - (last[a.relativePath] || 0));
    const sections = [];
    if (started.length) sections.push({ title: 'Weiterschauen', kind: 'wide', list: started });
    sections.push({ title: 'Alle Filme', kind: 'poster', list: byTitle });
    // Genre rows once there is enough to fill them.
    const genres = new Map();
    byTitle.forEach((it) => genresOf(it).forEach((g) => { if (!genres.has(g)) genres.set(g, []); genres.get(g).push(it); }));
    [...genres.entries()].filter(([, list]) => list.length >= 2 && list.length < items.length)
      .sort((a, b) => b[1].length - a[1].length).slice(0, 5)
      .forEach(([g, list]) => sections.push({ title: g, kind: 'poster', list }));

    $('lib-rows').innerHTML = sections.map((s, i) => `
      <section class="lib-row" data-row="${i + 1}">
        <h3>${esc(s.title)}<span>${s.list.length}</span></h3>
        <div class="lib-track"><div class="lib-strip">${s.list.map((it) => cardHtml(it, s.kind)).join('')}</div></div>
      </section>`).join('');

    rows = [{ el: $('lib-top'), cards: [...overlay.querySelectorAll('#lib-top-actions .lib-btn')] }];
    overlay.querySelectorAll('.lib-row').forEach((el) => rows.push({ el, cards: [...el.querySelectorAll('.lib-card')] }));
    col = rows.map(() => 0);
    $('lib-count').textContent = items.length === 1 ? '1 Film' : `${items.length} Filme`;
    overlay.classList.toggle('lib-is-empty', !items.length);
  }

  function focusEl() { return rows[row] && rows[row].cards[col[row]]; }
  const itemOf = (el) => (el && el.dataset.idx != null ? items[Number(el.dataset.idx)] : null);

  function setFocus(r, c, { instant = false } = {}) {
    if (!rows.length) return;
    row = Math.max(0, Math.min(rows.length - 1, r));
    const cards = rows[row].cards;
    col[row] = Math.max(0, Math.min(cards.length - 1, c));
    overlay.querySelectorAll('.lib-card.focused, .lib-btn.focused').forEach((el) => el.classList.remove('focused'));
    const el = focusEl();
    if (!el) return;
    el.classList.add('focused');
    const behavior = instant ? 'auto' : 'smooth';
    // Keep the focused card in view horizontally (a little margin to the left
    // so the previous card peeks in) and its row vertically.
    const track = el.closest('.lib-track');
    if (track) {
      const target = Math.max(0, el.offsetLeft - 56);
      track.scrollTo({ left: target, behavior });
    }
    if (row === 0) $('lib-rows').scrollTo({ top: 0, behavior });
    else rows[row].el.scrollIntoView({ block: 'nearest', behavior });
    showHero(itemOf(el));
  }

  function showHero(item) {
    const hero = $('lib-hero');
    if (!item) { hero.classList.remove('on'); return; }
    hero.classList.add('on');
    const pos = position(item);
    $('lib-hero-kicker').textContent = resumable(item) ? `Weiterschauen · ${leftText(item, pos)}` : (item.remote ? 'Vom PC-Server' : 'Auf diesem Laptop');
    $('lib-hero-title').textContent = item.title;
    $('lib-hero-meta').innerHTML = metaChips(item);
    $('lib-hero-plot').textContent = parseInfo(item.info).plot || '';
    setBackdrop(item);
  }

  let cached = null;
  let fetching = null;
  function prefetch() {
    if (!fetching) {
      fetching = Promise.resolve(window.launcher.listLibrary())
        .then((list) => { if (Array.isArray(list)) cached = list; return cached || []; })
        .finally(() => { fetching = null; });
    }
    return fetching;
  }

  async function openLibrary() {
    libraryOpen = true;
    animatedOpen(overlay);
    const mySeq = ++loadSeq;
    rows = [];
    overlay.classList.add('lib-loading');
    // The list from boot (prefetch) opens it instantly; a fresh one is fetched
    // in the background for next time.
    let list = cached;
    if (list) prefetch();
    else { try { list = await prefetch(); } catch { /* shown as empty */ } }
    if (mySeq !== loadSeq || !libraryOpen) return;
    items = Array.isArray(list) ? list : [];
    overlay.classList.remove('lib-loading');
    buildRows();
    // Start on the first film, not on the buttons.
    setFocus(rows.length > 1 ? 1 : 0, 0, { instant: true });
    if (rows.length <= 1) setBackdrop(null);
  }

  function closeLibrary() {
    libraryOpen = false;
    loadSeq++;
    animatedClose(overlay, { duration: 220 });
  }

  function activate(el) {
    if (!el) return;
    if (el.dataset.act === 'close') { closeLibrary(); return; }
    if (el.dataset.act === 'folders') { closeLibrary(); openMediathek(); return; }
    const item = itemOf(el);
    if (item) openLibraryDetail(item);
  }

  function moveRow(dir) {
    const next = row + dir;
    if (next < 0 || next >= rows.length) return;
    setFocus(next, col[next]);
  }

  function handleLibraryKey(e) {
    switch (e.key) {
      case 'Escape': case 'Backspace': closeLibrary(); break;
      case 'ArrowRight': setFocus(row, col[row] + 1); break;
      case 'ArrowLeft': setFocus(row, col[row] - 1); break;
      case 'ArrowDown': moveRow(1); break;
      case 'ArrowUp': moveRow(-1); break;
      case 'Enter': activate(focusEl()); break;
    }
  }

  // ------------------------------------------------------------ film page
  function creditsHtml(item) {
    const people = String(item.cast || '').split('\n').map((l) => l.trim()).filter(Boolean).slice(0, 6).map((l) => {
      const [name, role] = l.split(/\s+als\s+/);
      return `<div class="ld-person"><b>${esc(name)}</b>${role ? `<span>${esc(role)}</span>` : ''}</div>`;
    });
    const crew = String(item.crew || '').split('\n').map((l) => l.trim()).filter(Boolean).slice(0, 3).map((l) => {
      const [k, v] = l.split(/:\s*/);
      return v ? `<div class="ld-crew"><span>${esc(k)}</span><b>${esc(v)}</b></div>` : `<div class="ld-crew"><b>${esc(l)}</b></div>`;
    });
    return (people.length ? `<div class="ld-cast">${people.join('')}</div>` : '') + (crew.length ? `<div class="ld-crewlist">${crew.join('')}</div>` : '');
  }

  function renderDetail(item) {
    current = item;
    const panel = $('library-detail-panel');
    const back = item.banner || item.icon;
    panel.querySelector('.ld-bd').style.backgroundImage = back ? cssUrl(back) : fallbackArt(item);
    $('ld-poster').style.background = item.icon ? `${cssUrl(item.icon)} center / cover` : fallbackArt(item);
    $('ld-kicker').textContent = item.remote ? 'Film · vom PC-Server' : 'Film · auf diesem Laptop';
    $('ld-title').textContent = item.title;
    $('ld-meta').innerHTML = metaChips(item);
    $('ld-plot').textContent = parseInfo(item.info).plot || 'Keine Beschreibung vorhanden.';
    $('ld-credits').innerHTML = creditsHtml(item);

    const pos = position(item);
    const resume = resumable(item);
    $('ld-progress').classList.toggle('on', resume);
    if (resume) {
      $('ld-bar-fill').style.width = item.duration ? `${Math.min(100, (pos / item.duration) * 100).toFixed(1)}%` : '0%';
      $('ld-left').textContent = leftText(item, pos);
    }
    $('ld-actions').innerHTML = resume
      ? `<button class="ld-btn ld-primary" data-act="resume" type="button"><span class="ld-ic">▶</span>Weiterschauen<small>ab ${fmtClock(pos)}</small></button>
         <button class="ld-btn" data-act="restart" type="button"><span class="ld-ic">↺</span>Von vorne</button>
         <button class="ld-btn" data-act="forget" type="button"><span class="ld-ic">✓</span>Gesehen</button>`
      : `<button class="ld-btn ld-primary" data-act="play" type="button"><span class="ld-ic">▶</span>Abspielen</button>`;

    // More like this: same genres first, then the rest.
    const gs = new Set(genresOf(item));
    const others = items.filter((it) => it !== item)
      .map((it) => ({ it, score: genresOf(it).filter((g) => gs.has(g)).length }))
      .sort((a, b) => b.score - a.score || a.it.title.localeCompare(b.it.title, 'de'))
      .slice(0, 12).map((x) => x.it);
    $('ld-more').classList.toggle('on', others.length > 0);
    $('ld-more-strip').innerHTML = others.map((it) => cardHtml(it, 'poster')).join('');

    dRows = [[...$('ld-actions').querySelectorAll('.ld-btn')], [...$('ld-more-strip').querySelectorAll('.lib-card')]].filter((r) => r.length);
    dCol = dRows.map(() => 0);
    setDetailFocus(0, 0, { instant: true });
    panel.scrollTop = 0;
  }

  function setDetailFocus(r, c, { instant = false } = {}) {
    if (!dRows.length) return;
    dRow = Math.max(0, Math.min(dRows.length - 1, r));
    dCol[dRow] = Math.max(0, Math.min(dRows[dRow].length - 1, c));
    detail.querySelectorAll('.focused').forEach((el) => el.classList.remove('focused'));
    const el = dRows[dRow][dCol[dRow]];
    el.classList.add('focused');
    const behavior = instant ? 'auto' : 'smooth';
    const track = el.closest('.lib-track');
    if (track) track.scrollTo({ left: Math.max(0, el.offsetLeft - 56), behavior });
    const panel = $('library-detail-panel');
    if (dRow > 0 && dRows[dRow][0].closest('#ld-more')) $('ld-more').scrollIntoView({ block: 'end', behavior });
    else panel.scrollTo({ top: 0, behavior });
  }

  function openLibraryDetail(item) {
    if (!item) return;
    libraryDetailOpen = true;
    renderDetail(item);
    animatedOpen(detail);
  }
  function closeLibraryDetail() {
    libraryDetailOpen = false;
    animatedClose(detail, { duration: 200 });
    if (libraryOpen) {
      // Positions may have changed (Gesehen / Von vorne): refresh the rows, keep the film focused.
      const keep = current;
      buildRows();
      const r = rows.findIndex((rw, i) => i > 0 && rw.cards.some((c) => itemOf(c) === keep));
      if (r > 0) setFocus(r, rows[r].cards.findIndex((c) => itemOf(c) === keep), { instant: true });
      else setFocus(Math.min(1, rows.length - 1), 0, { instant: true });
    }
  }

  function play(item, fromStart) {
    closeLibraryDetail();
    closeLibrary();
    contentOpen = true;
    // -1 = explicitly from the start (main.js would otherwise fall back to the server's position).
    const resume = fromStart ? -1 : position(item);
    if (fromStart) saveResumePosition(item.relativePath, 0);
    launchContent('file:///mediaplayer.html', () => window.launcher.openMedia(item.fullPath, item.relativePath, resume));
  }

  function activateDetail(el) {
    if (!el || !current) return;
    switch (el.dataset.act) {
      case 'play': case 'resume': play(current, false); return;
      case 'restart': play(current, true); return;
      case 'forget':
        saveResumePosition(current.relativePath, 0);
        renderDetail(current);
        return;
    }
    const item = itemOf(el);
    if (item) {
      // Jump to the other film: a quick cross-fade instead of a new overlay.
      const panel = $('library-detail-panel');
      panel.classList.add('ld-swap');
      setTimeout(() => { renderDetail(item); panel.classList.remove('ld-swap'); }, 180);
    }
  }

  function handleLibraryDetailKey(e) {
    switch (e.key) {
      case 'Escape': case 'Backspace': closeLibraryDetail(); break;
      case 'ArrowRight': setDetailFocus(dRow, dCol[dRow] + 1); break;
      case 'ArrowLeft': setDetailFocus(dRow, dCol[dRow] - 1); break;
      case 'ArrowDown': if (dRow < dRows.length - 1) setDetailFocus(dRow + 1, dCol[dRow + 1]); break;
      case 'ArrowUp': if (dRow > 0) setDetailFocus(dRow - 1, dCol[dRow - 1]); break;
      case 'Enter': activateDetail(dRows[dRow] && dRows[dRow][dCol[dRow]]); break;
    }
  }

  // ------------------------------------------------------------ mouse / phone pointer
  function locate(el, grid) {
    for (let r = 0; r < grid.length; r++) {
      const c = grid[r].indexOf(el);
      if (c >= 0) return [r, c];
    }
    return null;
  }
  function pointerHover(target) {
    if (!target || !target.closest) return;
    if (libraryDetailOpen) {
      const el = target.closest('#library-detail-overlay .ld-btn, #library-detail-overlay .lib-card');
      const at = el && !el.classList.contains('focused') && locate(el, dRows);
      if (at) setDetailFocus(at[0], at[1]);
    } else if (libraryOpen) {
      const el = target.closest('#library-overlay .lib-card, #library-overlay .lib-btn');
      const at = el && !el.classList.contains('focused') && locate(el, rows.map((r) => r.cards));
      if (at) setFocus(at[0], at[1]);
    }
  }
  overlay.addEventListener('mousemove', (e) => pointerHover(e.target));
  detail.addEventListener('mousemove', (e) => pointerHover(e.target));
  overlay.addEventListener('click', (e) => {
    const el = e.target.closest('.lib-card, .lib-btn');
    if (el && libraryOpen && !libraryDetailOpen) { pointerHover(el); activate(el); }
  });
  detail.addEventListener('click', (e) => {
    const el = e.target.closest('.ld-btn, .lib-card');
    if (el) { pointerHover(el); activateDetail(el); }
  });

  return {
    openLibrary, closeLibrary, openLibraryDetail, closeLibraryDetail,
    handleLibraryKey, handleLibraryDetailKey, pointerHover, prefetch,
    isOpen: () => libraryOpen || libraryDetailOpen
  };
})();

// Global names renderer.js calls.
function openLibrary() { return library.openLibrary(); }
function closeLibrary() { return library.closeLibrary(); }
function closeLibraryDetail() { return library.closeLibraryDetail(); }
function handleLibraryKey(e) { return library.handleLibraryKey(e); }
function handleLibraryDetailKey(e) { return library.handleLibraryDetailKey(e); }
window.library = library;
