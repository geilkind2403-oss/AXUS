// --- Live TV app. Runs in the content view (file://livetv.html), fed by the
// M3U playlist in data/livetv/ through content-preload.js (window.liveTv).
//
// Watching:  ▲/▼ (or Page) zap · 0-9 channel number · OK/◀/▶ guide · M mute
// Guide:     groups (Favoriten, Alle, countries) | channels | preview + details
//            ▲▼ move · ◀▶ switch column · ▶ on a channel = favourite · OK tune
// Back (Escape, routed here by main.js) closes the guide first, then the app.
(() => {
  'use strict';

  const $ = (id) => document.getElementById(id);
  const video = $('v');
  const bridge = window.liveTv || null;

  const COUNTRY = {
    DE: 'Deutschland', AT: 'Österreich', CH: 'Schweiz', US: 'USA', UK: 'Großbritannien', GB: 'Großbritannien',
    IT: 'Italien', AU: 'Australien', ES: 'Spanien', IR: 'Iran', CA: 'Kanada', NZ: 'Neuseeland', FR: 'Frankreich',
    NL: 'Niederlande', SW: 'Schweden', SE: 'Schweden', RO: 'Rumänien', MK: 'Nordmazedonien', TR: 'Türkei',
    RU: 'Russland', ID: 'Indonesien', JP: 'Japan', QA: 'Katar', AE: 'Emirate', FO: 'Färöer', AL: 'Albanien',
    IE: 'Irland', JM: 'Jamaika', SG: 'Singapur', KP: 'Nordkorea', KR: 'Südkorea', FI: 'Finnland', PL: 'Polen'
  };
  const countryName = (cc) => COUNTRY[cc] || cc || 'International';

  const store = {
    get(key, fallback) { try { const v = JSON.parse(localStorage.getItem(key)); return v == null ? fallback : v; } catch { return fallback; } },
    set(key, value) { try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* no storage */ } }
  };

  let channels = [];
  let favs = new Set(store.get('livetv-favs', []));
  let current = null;
  let zapGroup = store.get('livetv-zapgroup', 'all');

  // ------------------------------------------------------------ helpers
  const pad2 = (n) => String(n).padStart(2, '0');
  const clock = () => { const d = new Date(); return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`; };
  function initials(name) {
    const words = String(name).replace(/\bHD\b/gi, '').trim().split(/\s+/).filter(Boolean);
    return (words.length > 1 ? words[0][0] + words[1][0] : (words[0] || '?').slice(0, 3)).toUpperCase();
  }
  // The playlist's logos are a mix of light-on-transparent and
  // dark-on-transparent PNGs. Each one's brightness is measured once (tiny
  // canvas, cached across sessions) and dark logos get a light plate, so
  // "arte" or "DF1" don't vanish into the dark UI.
  const logoTone = new Map(Object.entries(store.get('livetv-logotone', {})));
  let toneSaveTimer = null;
  function measureTone(img) {
    try {
      const c = document.createElement('canvas');
      c.width = 24; c.height = 24;
      const ctx = c.getContext('2d', { willReadFrequently: true });
      ctx.drawImage(img, 0, 0, 24, 24);
      const d = ctx.getImageData(0, 0, 24, 24).data;
      let sum = 0; let n = 0;
      for (let i = 0; i < d.length; i += 4) {
        if (d[i + 3] < 128) continue;
        sum += (0.2126 * d[i] + 0.7152 * d[i + 1] + 0.0722 * d[i + 2]) / 255;
        n++;
      }
      return n && sum / n < 0.33 ? 'dark' : 'light';
    } catch {
      return 'light'; // tainted/unreadable: keep the default dark plate
    }
  }
  function applyTone(plate, tone) { plate.classList.toggle('on-light', tone === 'dark'); }

  // Logo on a soft plate; a missing/broken logo becomes the channel's initials.
  function logoPlate(ch) {
    const plate = document.createElement('div');
    plate.className = 'logo-plate';
    const fallback = () => {
      plate.textContent = '';
      const span = document.createElement('span');
      span.className = 'initials';
      span.textContent = initials(ch.name);
      plate.appendChild(span);
    };
    if (ch.logo) {
      const img = document.createElement('img');
      img.loading = 'lazy';
      img.decoding = 'async';
      img.alt = '';
      img.referrerPolicy = 'no-referrer';
      img.onerror = fallback;
      const known = logoTone.get(ch.logo);
      if (known) {
        applyTone(plate, known);
      } else {
        img.crossOrigin = 'anonymous'; // the logo host sends CORS headers, so the pixels are readable
        img.onload = () => {
          const tone = measureTone(img);
          logoTone.set(ch.logo, tone);
          applyTone(plate, tone);
          clearTimeout(toneSaveTimer);
          toneSaveTimer = setTimeout(() => store.set('livetv-logotone', Object.fromEntries(logoTone)), 1500);
        };
      }
      img.src = ch.logo;
      plate.appendChild(img);
    } else {
      fallback();
    }
    return plate;
  }
  function setLogo(el, ch) { el.textContent = ''; el.appendChild(logoPlate(ch)); }

  function channelsOf(groupId) {
    if (groupId === 'fav') return channels.filter((c) => favs.has(c.url));
    if (groupId === 'all') return channels;
    return channels.filter((c) => c.country === groupId);
  }

  // ------------------------------------------------------------ playback
  let hls = null;
  let spinTimer = null;
  let stallTimer = null;
  const spinner = $('spinner');
  const nosignal = $('nosignal');

  function buffering(on) {
    clearTimeout(spinTimer);
    if (on) spinTimer = setTimeout(() => spinner.classList.add('on'), 300);
    else spinner.classList.remove('on');
  }
  function noSignal(title, text) {
    buffering(false);
    clearTimeout(stallTimer);
    nosignal.querySelector('.ns-title').textContent = title || 'Sender momentan nicht verfügbar';
    nosignal.querySelector('.ns-text').textContent = text || '▲ ▼ anderen Sender wählen · OK Senderliste';
    nosignal.classList.add('on');
  }
  function stopStream() {
    if (hls) { hls.destroy(); hls = null; }
    video.removeAttribute('src');
    video.load();
  }

  function tune(ch) {
    if (!ch) return;
    current = ch;
    store.set('livetv-last', ch.url);
    stopStream();
    nosignal.classList.remove('on');
    buffering(true);
    // A stream that never starts is as good as dead -- say so instead of
    // spinning forever.
    clearTimeout(stallTimer);
    stallTimer = setTimeout(() => { if (video.readyState < 3) noSignal(); }, 15000);

    const isFile = /\.(mp4|webm|mkv)(\?|$)/i.test(ch.url);
    if (!isFile && window.Hls && Hls.isSupported()) {
      hls = new Hls({ maxBufferLength: 20, backBufferLength: 10 });
      let retries = 0;
      let recovered = false;
      hls.on(Hls.Events.ERROR, (event, data) => {
        if (!data.fatal) return;
        if (data.type === Hls.ErrorTypes.NETWORK_ERROR && retries < 2) {
          retries += 1;
          setTimeout(() => hls && hls.startLoad(), 1200 * retries);
        } else if (data.type === Hls.ErrorTypes.MEDIA_ERROR && !recovered) {
          recovered = true;
          hls.recoverMediaError();
        } else {
          noSignal(undefined, navigator.onLine ? undefined : 'Keine Internetverbindung');
        }
      });
      hls.on(Hls.Events.FRAG_LOADED, () => { retries = 0; });
      hls.loadSource(ch.url);
      hls.attachMedia(video);
    } else if (!isFile && !window.Hls) {
      noSignal('Live TV braucht Internet', 'Der Player konnte nicht geladen werden.');
      return;
    } else {
      video.src = ch.url;
    }
    video.play().catch(() => {});
    markPlaying();
  }

  video.addEventListener('waiting', () => buffering(true));
  video.addEventListener('playing', () => { buffering(false); clearTimeout(stallTimer); nosignal.classList.remove('on'); });
  video.addEventListener('error', () => { if (current && !hls) noSignal(); });

  try {
    const vol = parseFloat(localStorage.getItem('tv-launcher-volume'));
    if (Number.isFinite(vol)) video.volume = Math.min(1, Math.max(0, vol));
  } catch { /* default volume */ }

  // ------------------------------------------------------------ banner
  const banner = $('banner');
  let bannerTimer = null;
  function showBanner(ch, ms = 4500) {
    if (!ch) return;
    $('bn-num').textContent = ch.number;
    setLogo($('bn-logo'), ch);
    $('bn-name').textContent = ch.name;
    $('bn-country').textContent = countryName(ch.country);
    $('bn-fav').classList.toggle('on', favs.has(ch.url));
    $('bn-clock').textContent = clock();
    banner.classList.add('on');
    clearTimeout(bannerTimer);
    bannerTimer = setTimeout(() => banner.classList.remove('on'), ms);
  }

  // Zapping shows the next channel's banner at once but only switches the
  // stream once the key rests -- holding ▲ must not start twenty streams.
  let zapTarget = null;
  let zapTimer = null;
  function zap(dir) {
    let list = channelsOf(zapGroup);
    if (!list.length) list = channels;
    const from = zapTarget || current;
    let idx = list.indexOf(from);
    if (idx < 0) idx = Math.max(0, list.findIndex((c) => c.number > (from ? from.number : 0)) - (dir > 0 ? 1 : 0));
    zapTarget = list[(idx + dir + list.length) % list.length];
    showBanner(zapTarget);
    clearTimeout(zapTimer);
    zapTimer = setTimeout(() => { const t = zapTarget; zapTarget = null; if (t !== current) tune(t); }, 380);
  }

  // ------------------------------------------------------------ number entry
  const numBox = $('numentry');
  let digits = '';
  let numTimer = null;
  function typeDigit(d) {
    const maxLen = String(channels.length).length;
    digits = (digits + d).slice(0, maxLen);
    const n = Number(digits);
    const match = channels.find((c) => c.number === n);
    $('ne-digits').textContent = digits;
    $('ne-name').textContent = match ? match.name : '';
    numBox.classList.add('on');
    clearTimeout(numTimer);
    numTimer = setTimeout(commitDigits, digits.length >= maxLen ? 250 : 1500);
  }
  function commitDigits() {
    clearTimeout(numTimer);
    const match = channels.find((c) => c.number === Number(digits));
    digits = '';
    numBox.classList.remove('on');
    if (!match) return;
    if (guideOpen) closeGuide();
    tune(match);
    showBanner(match);
  }
  function cancelDigits() { clearTimeout(numTimer); digits = ''; numBox.classList.remove('on'); }

  // ------------------------------------------------------------ guide
  let guideOpen = false;
  let zone = 'channels';
  let groups = [];
  let gIdx = 0;
  let activeGroup = 'all';
  let rows = [];
  let cIdx = 0;
  const groupList = $('g-group-list');
  const list = $('g-list');

  function buildGroups() {
    const counts = {};
    channels.forEach((c) => { counts[c.country] = (counts[c.country] || 0) + 1; });
    const countries = Object.keys(counts).sort((a, b) => (a === 'DE' ? -1 : b === 'DE' ? 1 : counts[b] - counts[a] || countryName(a).localeCompare(countryName(b))));
    groups = [
      { id: 'fav', label: 'Favoriten', badge: '★', cls: 'star' },
      { id: 'all', label: 'Alle Sender', badge: 'TV', cls: 'all' },
      ...countries.map((cc) => ({ id: cc, label: countryName(cc), badge: cc || '–', cls: '' }))
    ];
    groupList.textContent = '';
    groups.forEach((g, i) => {
      const el = document.createElement('div');
      el.className = 'grp';
      el.dataset.i = i;
      const cc = document.createElement('span');
      cc.className = `cc ${g.cls}`;
      cc.textContent = g.badge;
      const label = document.createElement('span');
      label.textContent = g.label;
      const count = document.createElement('span');
      count.className = 'count';
      el.append(cc, label, count);
      el.addEventListener('click', () => { focusGroup(i); zone = 'groups'; paintFocus(); });
      groupList.appendChild(el);
    });
    updateCounts();
  }
  function updateCounts() {
    [...groupList.children].forEach((el, i) => { el.querySelector('.count').textContent = channelsOf(groups[i].id).length; });
  }

  function renderChannels(groupId, focusCh) {
    activeGroup = groupId;
    const chs = channelsOf(groupId);
    const g = groups.find((x) => x.id === groupId);
    $('g-head-title').textContent = g ? g.label : '';
    $('g-head-count').textContent = `${chs.length} Sender`;
    [...groupList.children].forEach((el, i) => el.classList.toggle('active', groups[i].id === groupId));
    list.textContent = '';
    list.scrollTop = 0;
    rows = [];
    if (!chs.length) {
      const empty = document.createElement('div');
      empty.className = 'g-empty';
      empty.textContent = groupId === 'fav'
        ? 'Noch keine Favoriten. Wähle in einer Senderliste einen Sender und drücke ▶, um ihn hier zu speichern.'
        : 'Keine Sender in dieser Liste.';
      list.appendChild(empty);
      cIdx = 0;
      updateDetail();
      return;
    }
    const frag = document.createDocumentFragment();
    chs.forEach((ch, i) => {
      const row = document.createElement('div');
      row.className = 'ch' + (i < 14 ? ' enter' : '') + (favs.has(ch.url) ? ' is-fav' : '') + (ch === current ? ' is-playing' : '');
      row.style.setProperty('--k', i);
      row.innerHTML = '<span class="num"></span><span class="lg"></span><span class="nm"></span><span class="playing live-badge">LIVE</span><span class="fav">★</span>';
      row.querySelector('.num').textContent = ch.number;
      row.querySelector('.lg').appendChild(logoPlate(ch));
      row.querySelector('.nm').textContent = ch.name;
      row.addEventListener('click', () => { cIdx = i; zone = 'channels'; paintFocus(); tuneFromGuide(ch); });
      row.addEventListener('mouseenter', () => { if (zone === 'channels' && cIdx !== i) { cIdx = i; paintFocus(); } });
      row._ch = ch;
      rows.push(row);
      frag.appendChild(row);
    });
    list.appendChild(frag);
    const at = focusCh ? chs.indexOf(focusCh) : -1;
    cIdx = at >= 0 ? at : 0;
    paintFocus(true);
  }

  function markPlaying() {
    rows.forEach((r) => r.classList.toggle('is-playing', r._ch === current));
    if (guideOpen) updateDetail();
  }

  let detailFor = null;
  function updateDetail() {
    const ch = rows[cIdx] ? rows[cIdx]._ch : current;
    const card = document.querySelector('.gd-card');
    if (!ch) { card.style.visibility = 'hidden'; return; }
    card.style.visibility = '';
    if (ch !== detailFor) {
      detailFor = ch;
      setLogo($('gd-logo'), ch);
      $('gd-name').textContent = ch.name;
      $('gd-meta').textContent = `Sender ${ch.number} · ${countryName(ch.country)}${favs.has(ch.url) ? ' · ★ Favorit' : ''}`;
      card.classList.remove('swap');
      void card.offsetWidth;
      card.classList.add('swap');
    }
    const state = $('gd-state');
    if (ch === current) { state.className = 'gd-state now'; state.textContent = '● Läuft gerade'; }
    else { state.className = 'gd-state tune'; state.textContent = 'OK drücken zum Einschalten'; }
  }

  function paintFocus(instant) {
    [...groupList.children].forEach((el, i) => el.classList.toggle('focused', zone === 'groups' && i === gIdx));
    rows.forEach((r, i) => r.classList.toggle('focused', zone === 'channels' && i === cIdx));
    const row = rows[cIdx];
    if (row) row.scrollIntoView({ block: 'nearest', behavior: instant ? 'auto' : 'smooth' });
    const grp = groupList.children[gIdx];
    if (grp && zone === 'groups') grp.scrollIntoView({ block: 'nearest' });
    updateDetail();
    renderHints();
  }

  function renderHints() {
    const h = $('g-hints');
    const items = zone === 'channels'
      ? [['▲▼', 'Sender'], ['OK', 'Einschalten'], ['▶', 'Favorit'], ['◀', 'Listen'], ['Zurück', 'Schließen']]
      : [['▲▼', 'Liste wählen'], ['▶', 'Zu den Sendern'], ['Zurück', 'Schließen']];
    h.textContent = '';
    items.forEach(([k, t]) => {
      const span = document.createElement('span');
      const kbd = document.createElement('kbd');
      kbd.textContent = k;
      span.append(kbd, document.createTextNode(t));
      h.appendChild(span);
    });
  }

  let groupRenderTimer = null;
  function focusGroup(i) {
    gIdx = Math.max(0, Math.min(groups.length - 1, i));
    paintFocus();
    // Rendering 300 rows on every ▲▼ step through the groups would stutter;
    // only the group the focus rests on is drawn.
    clearTimeout(groupRenderTimer);
    groupRenderTimer = setTimeout(() => renderChannels(groups[gIdx].id, current), 140);
  }

  // The preview box is wherever #g-preview-frame lays out; the picture is
  // moved/scaled onto it (transform only), so it never reloads.
  function placePreview() {
    const r = $('g-preview-frame').getBoundingClientRect();
    const s = r.width / window.innerWidth;
    document.body.style.setProperty('--pv-x', `${r.left}px`);
    document.body.style.setProperty('--pv-y', `${r.top}px`);
    document.body.style.setProperty('--pv-s', s.toFixed(4));
  }

  function openGuide() {
    if (guideOpen || !channels.length) return;
    guideOpen = true;
    placePreview();
    banner.classList.remove('on');
    const startGroup = channelsOf(zapGroup).includes(current) ? zapGroup : 'all';
    gIdx = Math.max(0, groups.findIndex((g) => g.id === startGroup));
    zone = 'channels';
    detailFor = null;
    document.body.classList.add('guide');
    renderChannels(startGroup, current);
    $('g-clock').textContent = clock();
  }
  function closeGuide() {
    if (!guideOpen) return;
    guideOpen = false;
    document.body.classList.remove('guide');
    showBanner(current, 3000);
  }
  function tuneFromGuide(ch) {
    zapGroup = activeGroup === 'fav' || channelsOf(activeGroup).includes(ch) ? activeGroup : 'all';
    store.set('livetv-zapgroup', zapGroup);
    if (ch !== current) tune(ch);
    closeGuide();
  }
  function toggleFav() {
    const row = rows[cIdx];
    if (!row) return;
    const url = row._ch.url;
    if (favs.has(url)) favs.delete(url); else favs.add(url);
    store.set('livetv-favs', [...favs]);
    row.classList.toggle('is-fav', favs.has(url));
    row.classList.remove('pop');
    void row.offsetWidth;
    row.classList.add('pop');
    detailFor = null;
    updateCounts();
    updateDetail();
  }

  // ------------------------------------------------------------ input
  function back() {
    if (digits) { cancelDigits(); return; }
    if (guideOpen) { closeGuide(); return; }
    if (bridge) bridge.exit();
  }
  if (bridge) bridge.onBack(back);

  let lastMove = 0;
  document.addEventListener('keydown', (e) => {
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    const k = e.key;
    if (/^[0-9]$/.test(k)) { e.preventDefault(); typeDigit(k); return; }
    if (k === 'Escape' || k === 'Backspace') { e.preventDefault(); back(); return; }
    if (k === 'm' || k === 'M') { video.muted = !video.muted; return; }
    if (digits && k === 'Enter') { e.preventDefault(); commitDigits(); return; }

    if (!guideOpen) {
      if (k === 'ArrowUp' || k === 'PageUp' || k === 'ChannelUp') { e.preventDefault(); zap(1); }
      else if (k === 'ArrowDown' || k === 'PageDown' || k === 'ChannelDown') { e.preventDefault(); zap(-1); }
      else if (k === 'Enter' || k === 'ArrowLeft' || k === 'ArrowRight') { e.preventDefault(); if (!e.repeat) openGuide(); }
      else showBanner(current);
      return;
    }

    e.preventDefault();
    // Held arrows scroll quickly but at a steady pace (auto-repeat is ~30/s).
    const now = performance.now();
    if (e.repeat && now - lastMove < 45) return;
    lastMove = now;
    if (zone === 'groups') {
      if (k === 'ArrowDown') focusGroup(gIdx + 1);
      else if (k === 'ArrowUp') focusGroup(gIdx - 1);
      else if (k === 'ArrowRight' || k === 'Enter') {
        clearTimeout(groupRenderTimer);
        if (groups[gIdx].id !== activeGroup) renderChannels(groups[gIdx].id, current);
        if (rows.length) { zone = 'channels'; paintFocus(); }
      }
    } else {
      if (k === 'ArrowDown') { if (cIdx < rows.length - 1) { cIdx += 1; paintFocus(e.repeat); } }
      else if (k === 'ArrowUp') { if (cIdx > 0) { cIdx -= 1; paintFocus(e.repeat); } }
      else if (k === 'PageDown') { cIdx = Math.min(rows.length - 1, cIdx + 8); paintFocus(); }
      else if (k === 'PageUp') { cIdx = Math.max(0, cIdx - 8); paintFocus(); }
      else if (k === 'ArrowLeft') { zone = 'groups'; gIdx = Math.max(0, groups.findIndex((g) => g.id === activeGroup)); paintFocus(); }
      else if (k === 'ArrowRight') toggleFav();
      else if (k === 'Enter' && !e.repeat && rows[cIdx]) tuneFromGuide(rows[cIdx]._ch);
    }
  });
  window.addEventListener('resize', () => { if (guideOpen) placePreview(); });
  setInterval(() => { $('g-clock').textContent = clock(); $('bn-clock').textContent = clock(); }, 10000);

  // ------------------------------------------------------------ start
  (async () => {
    channels = bridge ? await bridge.getPlaylist().catch(() => []) : [];
    if (!channels.length) {
      noSignal('Keine Sender gefunden', 'Lege eine .m3u-Playlist in den Ordner data/livetv.');
      return;
    }
    buildGroups();
    const wanted = new URLSearchParams(location.search).get('src') || store.get('livetv-last', null);
    const start = channels.find((c) => c.url === wanted) || channels.find((c) => c.country === 'DE') || channels[0];
    tune(start);
    showBanner(start, 5000);
  })();
})();
