// --- Musik: the launcher's own music player ("Music" in the top bar).
// Plays the library of the Musik-Server on the PC (server/musicserver.js) and
// of data/music on this laptop; music-main.js merges both.
//
// Screens: Start (weiter hören, zuletzt gehört, Playlists, meistgehört, Alben,
// Interpreten), Alben, Interpreten, Titel, Playlists, Suche; album, artist and
// playlist pages; "Jetzt läuft" full screen with Warteschlange and Songtext.
//
// Remote: every screen is one focus field. An arrow goes to the nearest item
// in that direction, rows remember where you were (like Apple TV), the phone
// pointer focuses whatever it points at. Seek bar: ◀/▶ spult 10 s.
//
// Playback keeps running in the background -- home screen, Control Center
// (transport), media keys and Windows' media overlay (Media Session). Opening
// a film or a web app pauses it. The next song is loaded ahead of time, so one
// follows the other without a gap.
//
// Likes, playlists, play counts and the queue live in localStorage.
// Loaded after renderer.js (escapeHtml, cssUrl, fmtDuration, showKeyboard,
// animatedOpen/Close, contentOpen, onHomeVisibilityChange).
(() => {
  const $ = (id) => document.getElementById(id);
  const g = (fn) => { try { return fn(); } catch { return undefined; } };
  const esc = escapeHtml;
  const L = window.launcher;

  // ------------------------------------------------------------ storage
  const KEYS = { state: 'tv-music-state', stats: 'tv-music-stats', likes: 'tv-music-likes', lists: 'tv-music-playlists' };
  const load = (k, fb) => { try { const v = JSON.parse(localStorage.getItem(k)); return v == null ? fb : v; } catch { return fb; } };
  const save = (k, v) => { try { localStorage.setItem(k, JSON.stringify(v)); } catch { /* storage full: not fatal */ } };

  let likes = load(KEYS.likes, []);       // track ids, newest first
  let playlists = load(KEYS.lists, []);   // [{ id, name, trackIds, created }]
  const stats = { plays: {}, last: {}, albums: [], ...load(KEYS.stats, {}) }; // albums: recently played album ids

  // ------------------------------------------------------------ icons
  const HEART = 'M12 20.2s-7.6-4.5-7.6-10.2A4.3 4.3 0 0 1 12 7.2a4.3 4.3 0 0 1 7.6 2.8c0 5.7-7.6 10.2-7.6 10.2z';
  const REPEAT = '<path d="M4 11V9.5A3.5 3.5 0 0 1 7.5 6H19M16.5 3.5 19 6l-2.5 2.5M20 13v1.5a3.5 3.5 0 0 1-3.5 3.5H5M7.5 20.5 5 18l2.5-2.5"/>';
  const P = {
    play: '<path d="M7.5 5.2v13.6c0 .7.8 1.1 1.4.7l10.2-6.8a.8.8 0 0 0 0-1.4L8.9 4.5c-.6-.4-1.4 0-1.4.7z" fill="currentColor" stroke="none"/>',
    pause: '<rect x="6" y="4.8" width="4.2" height="14.4" rx="1.3" fill="currentColor" stroke="none"/><rect x="13.8" y="4.8" width="4.2" height="14.4" rx="1.3" fill="currentColor" stroke="none"/>',
    next: '<path d="M5 6v12c0 .6.7 1 1.2.6l8.3-6a.8.8 0 0 0 0-1.2L6.2 5.4C5.7 5 5 5.4 5 6z" fill="currentColor" stroke="none"/><rect x="16.2" y="5" width="2.8" height="14" rx="1.2" fill="currentColor" stroke="none"/>',
    prev: '<path d="M19 6v12c0 .6-.7 1-1.2.6l-8.3-6a.8.8 0 0 1 0-1.2l8.3-6c.5-.4 1.2 0 1.2.6z" fill="currentColor" stroke="none"/><rect x="5" y="5" width="2.8" height="14" rx="1.2" fill="currentColor" stroke="none"/>',
    shuffle: '<path d="M3 7h3.2c2 0 3.2 1 4.4 2.8l2.4 3.6c1.2 1.8 2.4 3.6 4.5 3.6H21M3 17h3.2c1.3 0 2.3-.6 3.2-1.6M13.6 8.6C14.5 7.6 15.6 7 17.5 7H21M18.5 4.5 21 7l-2.5 2.5M18.5 14.5 21 17l-2.5 2.5"/>',
    repeat: REPEAT,
    repeatOne: `${REPEAT}<path d="M11 10.6l1.6-1v5" stroke-width="1.7"/>`,
    heart: `<path d="${HEART}"/>`,
    heartFill: `<path d="${HEART}" fill="currentColor"/>`,
    plus: '<path d="M12 5v14M5 12h14"/>',
    more: '<circle cx="5.5" cy="12" r="1.7" fill="currentColor" stroke="none"/><circle cx="12" cy="12" r="1.7" fill="currentColor" stroke="none"/><circle cx="18.5" cy="12" r="1.7" fill="currentColor" stroke="none"/>',
    queue: '<path d="M4 6h13M4 11h13M4 16h7"/><path d="M15 14.5v6l5-3z" fill="currentColor" stroke="none"/>',
    lyrics: '<path d="M6.5 8h4v3.6c0 2.7-1.3 4.5-4 5.4M14 8h4v3.6c0 2.7-1.3 4.5-4 5.4"/>',
    note: '<path d="M9 18V6.2l10-2.2v12"/><circle cx="6.5" cy="18" r="2.5"/><circle cx="16.5" cy="16" r="2.5"/>',
    search: '<circle cx="11" cy="11" r="6.5"/><path d="M16 16l4.5 4.5"/>',
    disc: '<circle cx="12" cy="12" r="8.5"/><circle cx="12" cy="12" r="2.3"/>',
    user: '<circle cx="12" cy="8.5" r="3.8"/><path d="M4.5 20c.9-3.8 3.9-6 7.5-6s6.6 2.2 7.5 6"/>',
    list: '<path d="M9 6.5h11M9 12h11M9 17.5h11"/><circle cx="4.5" cy="6.5" r="1.2" fill="currentColor" stroke="none"/><circle cx="4.5" cy="12" r="1.2" fill="currentColor" stroke="none"/><circle cx="4.5" cy="17.5" r="1.2" fill="currentColor" stroke="none"/>',
    chart: '<path d="M5 20V11M12 20V5M19 20v-6"/>',
    close: '<path d="M6 6l12 12M18 6 6 18"/>',
    trash: '<path d="M4.5 7h15M9.5 7V4.8h5V7M6.5 7l.9 12.2h9.2l.9-12.2"/>',
    edit: '<path d="M4 20h4L19 9l-4-4L4 16z"/>',
    queueNext: '<path d="M4 7h10M4 12h10M4 17h6"/><path d="M18 9v8M14 13h8"/>',
    expand: '<path d="M14 4h6v6M10 20H4v-6M20 4l-7 7M4 20l7-7"/>',
    refresh: '<path d="M20 12a8 8 0 1 1-2.6-5.9M20 4.5V9h-4.5"/>',
    gear: '<circle cx="12" cy="12" r="3"/><path d="M12 2.8v2.4M12 18.8v2.4M4.2 7.5l2.1 1.2M17.7 15.3l2.1 1.2M4.2 16.5l2.1-1.2M17.7 8.7l2.1-1.2"/>'
  };
  const ic = (name, cls = '') => `<svg class="mu-ic ${cls}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${P[name] || ''}</svg>`;
  const EQ = '<span class="mu-eq" aria-hidden="true"><i></i><i></i><i></i><i></i></span>';

  // ------------------------------------------------------------ helpers
  const coll = new Intl.Collator('de', { sensitivity: 'base', numeric: true });
  const fold = (s) => String(s || '').toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g, '');
  function fmtTime(s) {
    s = Math.max(0, Math.floor(Number(s) || 0));
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    return h ? `${h}:${String(m).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}` : `${m}:${String(s % 60).padStart(2, '0')}`;
  }
  function hue(str) {
    let h = 0;
    for (let i = 0; i < str.length; i++) h = str.charCodeAt(i) + ((h << 5) - h);
    return Math.abs(h) % 360;
  }
  const fallbackBg = (seed) => `linear-gradient(145deg, hsl(${hue(String(seed))},52%,44%), hsl(${(hue(String(seed)) + 48) % 360},55%,17%))`;
  const initials = (name) => String(name).split(/\s+/).filter(Boolean).slice(0, 2).map((w) => w[0]).join('').toUpperCase() || '♪';
  function artHtml(url, seed, cls = '', inner = '') {
    const style = url ? `background-image:${cssUrl(url)}` : `background:${fallbackBg(seed)}`;
    return `<div class="mu-art ${cls}" style="${esc(style)}">${url ? '' : ic('note', 'mu-art-ic')}${inner}</div>`;
  }
  const toast = (title, text, icon = 'check') => g(() => window.tvos.toast({ icon, kind: 'info', title, text, duration: 2600, id: 'music' }));
  function shuffled(list) {
    const a = list.slice();
    for (let i = a.length - 1; i > 0; i--) { const j = Math.floor(Math.random() * (i + 1)); [a[i], a[j]] = [a[j], a[i]]; }
    return a;
  }

  // ------------------------------------------------------------ library
  let lib = null; // { artists, albums, tracks, server, localCount }
  const T = new Map();
  const A = new Map();
  const R = new Map();
  let libPromise = null;
  let libSig = '';

  function fetchLibrary(fresh = false) {
    if (!libPromise) {
      libPromise = Promise.resolve(L.musicLibrary({ fresh }))
        .then((l) => {
          // Same library as before (the usual background refresh): nothing to redraw.
          const sig = l && Array.isArray(l.tracks) ? `${l.tracks.length}|${l.albums.length}|${l.artists.length}|${l.tracks.reduce((n, t) => n + t.id.charCodeAt(0) + t.title.length + (t.duration || 0), 0)}|${Boolean(l.server && l.server.ok)}` : '';
          if (sig && sig === libSig) { lib.server = l.server; return lib; }
          if (l && Array.isArray(l.tracks)) {
            libSig = sig;
            lib = l;
            T.clear(); A.clear(); R.clear();
            l.tracks.forEach((t) => T.set(t.id, t));
            l.albums.forEach((a) => A.set(a.id, a));
            l.artists.forEach((a) => R.set(a.id, a));
            onLibrary();
          }
          return lib;
        })
        .catch(() => lib)
        .finally(() => { libPromise = null; });
    }
    return libPromise;
  }

  const albumTracks = (al) => (al ? al.trackIds.map((id) => T.get(id)).filter(Boolean) : []);
  const artistAlbums = (ar) => (ar ? ar.albumIds.map((id) => A.get(id)).filter(Boolean) : []);
  const artistTracks = (ar) => artistAlbums(ar).flatMap(albumTracks);
  const albumsSorted = () => [...A.values()].sort((a, b) => coll.compare(a.artist, b.artist) || (b.year || 0) - (a.year || 0) || coll.compare(a.title, b.title));
  const artistsSorted = () => [...R.values()].sort((a, b) => coll.compare(a.name, b.name));
  const tracksSorted = () => [...T.values()].sort((a, b) => coll.compare(a.title, b.title) || coll.compare(a.artist, b.artist));
  const coverOf = (t) => { const al = t && A.get(t.albumId); return al ? (al.cover || al.coverSmall) : null; };
  const smallCoverOf = (t) => { const al = t && A.get(t.albumId); return al ? (al.coverSmall || al.cover) : null; };
  const recentAlbums = () => stats.albums.map((id) => A.get(id)).filter(Boolean);
  const topTracks = (n) => Object.entries(stats.plays).filter(([id, c]) => c > 0 && T.has(id))
    .sort((a, b) => b[1] - a[1] || (stats.last[b[0]] || 0) - (stats.last[a[0]] || 0)).slice(0, n).map(([id]) => T.get(id));
  function featuredAlbum() {
    const all = albumsSorted();
    return all.length ? all[Math.floor(Date.now() / 86400000) % all.length] : null; // a different one each day
  }
  function artistTop(ar, n) {
    const tracks = artistTracks(ar);
    const played = tracks.filter((t) => stats.plays[t.id]).sort((a, b) => stats.plays[b.id] - stats.plays[a.id]);
    return [...played, ...tracks.filter((t) => !stats.plays[t.id])].slice(0, n);
  }

  // ------------------------------------------------------------ likes, playlists, stats
  const isLiked = (id) => likes.includes(id);
  function toggleLike(id) {
    const t = T.get(id);
    if (!t) return;
    const on = !isLiked(id);
    likes = on ? [id, ...likes] : likes.filter((x) => x !== id);
    save(KEYS.likes, likes);
    overlay.querySelectorAll(`[data-act="like"][data-id="${CSS.escape(id)}"]`).forEach((b) => {
      b.classList.toggle('on', on);
      b.innerHTML = ic(on ? 'heartFill' : 'heart');
    });
    if (nowOpen) renderNowButtons();
    toast(on ? 'Zu Lieblingssongs hinzugefügt' : 'Aus Lieblingssongs entfernt', t.title, on ? '♥' : 'check');
  }
  function playlistById(id) {
    if (id === 'likes') return { id, name: 'Lieblingssongs', kind: 'likes', trackIds: likes.filter((x) => T.has(x)) };
    if (id === 'top') return { id, name: 'Meistgehört', kind: 'top', trackIds: topTracks(50).map((t) => t.id) };
    const p = playlists.find((x) => x.id === id);
    return p ? { ...p, kind: 'user', trackIds: p.trackIds.filter((x) => T.has(x)) } : null;
  }
  function savePlaylists() { save(KEYS.lists, playlists); }
  async function newPlaylist(firstIds = []) {
    const name = await showKeyboard({ label: 'Name der neuen Playlist', initialValue: '' });
    if (name === null || !name.trim()) return null;
    const p = { id: `pl-${Date.now().toString(36)}`, name: name.trim(), trackIds: [...new Set(firstIds)], created: Date.now() };
    playlists.unshift(p);
    savePlaylists();
    toast('Playlist erstellt', firstIds.length ? `${p.name} · ${firstIds.length} Titel` : p.name);
    return p;
  }
  function addToPlaylist(p, ids) {
    const before = p.trackIds.length;
    p.trackIds = [...p.trackIds, ...ids.filter((id) => !p.trackIds.includes(id))];
    savePlaylists();
    const added = p.trackIds.length - before;
    toast(added ? `Zu „${p.name}“ hinzugefügt` : `Schon in „${p.name}“`, added === 1 ? (T.get(ids[0]) || {}).title || '' : `${added} Titel`);
  }
  function countPlay(t) {
    stats.plays[t.id] = (stats.plays[t.id] || 0) + 1;
    stats.last[t.id] = Date.now();
    save(KEYS.stats, stats);
  }
  function noteAlbum(t) {
    if (!t) return;
    stats.albums = [t.albumId, ...stats.albums.filter((id) => id !== t.albumId)].slice(0, 24);
    save(KEYS.stats, stats);
  }

  // ------------------------------------------------------------ player
  // Two audio elements: one plays, the other already loads the next song, so
  // the change at the end of a track has no gap (and no spinner on a slow WLAN).
  const player = (() => {
    const els = [new Audio(), new Audio()];
    els.forEach((a) => { a.preload = 'auto'; });
    let cur = 0;
    let queue = [];     // track ids in play order
    let base = [];      // the same list in its own order (to turn shuffle off again)
    let pos = -1;
    let shuffle = false;
    let repeat = 'off'; // off | all | one
    let context = null; // { type, id, name }
    let counted = false;
    let errors = 0;
    let resumeAt = 0;   // restored position, applied on the first play
    let savedAt = 0;
    const listeners = new Set();

    const audio = () => els[cur];
    const spare = () => els[1 - cur];
    const current = () => (pos >= 0 ? T.get(queue[pos]) || null : null);
    const loaded = () => Boolean(audio().getAttribute('src'));
    const isPlaying = () => loaded() && !audio().paused;
    const emit = (what) => listeners.forEach((cb) => g(() => cb(what)));

    function persist(force) {
      if (!force && Date.now() - savedAt < 5000) return;
      savedAt = Date.now();
      save(KEYS.state, { queue, base, pos, shuffle, repeat, context, at: loaded() ? audio().currentTime : resumeAt });
    }

    function clearSpare() {
      const s = spare();
      if (!s.getAttribute('src')) return;
      s.pause();
      s.removeAttribute('src');
      s.dataset.id = '';
      s.load();
    }

    function start(i, { play = true, at = 0 } = {}) {
      if (!queue.length) return;
      pos = Math.max(0, Math.min(queue.length - 1, i));
      const t = current();
      if (!t) return;
      counted = false;
      resumeAt = 0;
      audio().pause();
      if (spare().dataset.id === t.id && !at) {
        cur = 1 - cur; // the preloaded next song
      } else {
        const a = audio();
        a.dataset.id = t.id;
        a.src = t.stream;
        if (at) a.currentTime = at;
      }
      clearSpare();
      if (play) audio().play().catch(() => { /* reported through the 'error' event */ });
      noteAlbum(t);
      persist(true);
      emit('track');
    }

    function nextIndex() {
      if (repeat === 'one') return pos;
      if (pos + 1 < queue.length) return pos + 1;
      return repeat === 'all' ? 0 : -1;
    }

    function onTime() {
      const a = audio();
      const t = current();
      if (!t) return;
      if (!counted && (a.currentTime >= 30 || (a.duration && a.currentTime >= a.duration * 0.5))) {
        counted = true;
        countPlay(t);
      }
      // Load the next song ~20 s before the end.
      const left = (a.duration || t.duration || 0) - a.currentTime;
      if (left > 0 && left < 20) {
        const n = nextIndex();
        const nt = n >= 0 && n !== pos ? T.get(queue[n]) : null;
        if (nt && spare().dataset.id !== nt.id) {
          const s = spare();
          s.dataset.id = nt.id;
          s.src = nt.stream;
          s.load();
        }
      }
      persist(false);
      emit('time');
    }

    function onEnded() {
      if (repeat === 'one') { audio().currentTime = 0; audio().play().catch(() => {}); counted = false; return; }
      const n = nextIndex();
      if (n >= 0) { start(n); return; }
      // End of the list: stay on the last song, rewound, paused.
      audio().currentTime = 0;
      audio().pause();
      emit('state');
    }

    function onError() {
      const t = current();
      errors += 1;
      if (errors >= Math.min(3, queue.length)) {
        errors = 0;
        audio().pause();
        toast('Musik nicht erreichbar', lib && lib.server && lib.server.url ? 'Läuft der Musik-Server auf dem PC?' : 'Die Datei lässt sich nicht abspielen.', 'warning');
        emit('state');
        return;
      }
      toast('Titel übersprungen', t ? `${t.title} lässt sich nicht abspielen` : '', 'warning');
      setTimeout(() => { if (current() === t) next(); }, 900);
    }

    els.forEach((el, i) => {
      el.addEventListener('timeupdate', () => { if (i === cur) onTime(); });
      el.addEventListener('ended', () => { if (i === cur) onEnded(); });
      el.addEventListener('playing', () => { if (i === cur) { errors = 0; emit('state'); } });
      el.addEventListener('play', () => { if (i === cur) emit('state'); });
      el.addEventListener('pause', () => { if (i === cur) { persist(true); emit('state'); } });
      el.addEventListener('waiting', () => { if (i === cur) emit('buffering'); });
      el.addEventListener('loadedmetadata', () => { if (i === cur) emit('time'); });
      el.addEventListener('error', () => { if (i === cur && el.getAttribute('src')) onError(); });
    });

    function playList(ids, startAt = 0, opts = {}) {
      ids = ids.filter((id) => T.has(id));
      if (!ids.length) return;
      context = opts.context || null;
      base = ids.slice();
      if (opts.shuffle != null) shuffle = opts.shuffle;
      const first = ids[Math.max(0, Math.min(ids.length - 1, startAt))];
      if (shuffle) {
        const startId = opts.shuffle && opts.randomStart !== false && startAt === 0 ? ids[Math.floor(Math.random() * ids.length)] : first;
        queue = [startId, ...shuffled(ids.filter((id) => id !== startId))];
        start(0);
      } else {
        queue = ids.slice();
        start(Math.max(0, queue.indexOf(first)));
      }
      emit('queue');
    }

    function toggle() {
      if (!current()) return;
      if (!loaded()) { start(pos, { at: resumeAt }); return; }
      if (audio().paused) audio().play().catch(() => {}); else audio().pause();
    }
    function pause() { if (isPlaying()) audio().pause(); }
    function next() {
      if (!queue.length) return;
      if (pos + 1 < queue.length) start(pos + 1);
      else if (repeat !== 'off') start(0);
      else { start(pos, { play: false }); emit('state'); }
    }
    function prev() {
      if (!queue.length) return;
      if (loaded() && audio().currentTime > 3) { seek(0); return; }
      if (pos > 0) start(pos - 1);
      else if (repeat === 'all') start(queue.length - 1);
      else seek(0);
    }
    function seek(sec) {
      const t = current();
      if (!t) return;
      const dur = audio().duration || t.duration || 0;
      const to = Math.max(0, dur ? Math.min(dur - 0.5, sec) : sec);
      if (!loaded()) { resumeAt = to; emit('time'); return; }
      audio().currentTime = to;
      emit('time');
    }
    const seekBy = (d) => seek(time() + d);
    function time() { return loaded() ? audio().currentTime : resumeAt; }
    function duration() { const t = current(); return (loaded() && Number.isFinite(audio().duration) && audio().duration) || (t && t.duration) || 0; }

    function setShuffle(on) {
      if (on === shuffle) return;
      shuffle = on;
      const id = queue[pos];
      if (on) {
        queue = [id, ...shuffled(base.filter((x) => x !== id))];
        pos = 0;
      } else {
        queue = base.slice();
        pos = Math.max(0, queue.indexOf(id));
      }
      clearSpare();
      persist(true);
      emit('queue');
    }
    function cycleRepeat() {
      repeat = repeat === 'off' ? 'all' : repeat === 'all' ? 'one' : 'off';
      clearSpare();
      persist(true);
      emit('state');
    }
    function playNext(ids) {
      ids = ids.filter((id) => T.has(id));
      if (!ids.length) return;
      if (!current()) { playList(ids, 0); return; }
      queue.splice(pos + 1, 0, ...ids);
      const bi = base.indexOf(queue[pos]);
      base.splice(bi + 1, 0, ...ids);
      clearSpare();
      persist(true);
      emit('queue');
    }
    function addToQueue(ids) {
      ids = ids.filter((id) => T.has(id));
      if (!ids.length) return;
      if (!current()) { playList(ids, 0); return; }
      queue.push(...ids);
      base.push(...ids);
      persist(true);
      emit('queue');
    }
    function jump(i) { if (i >= 0 && i < queue.length) start(i); }

    // After the library is known: the queue from last time, paused where it stopped.
    function restore() {
      if (queue.length) return;
      const s = load(KEYS.state, null);
      if (!s || !Array.isArray(s.queue)) return;
      const id = s.queue[s.pos];
      queue = s.queue.filter((x) => T.has(x));
      base = (Array.isArray(s.base) ? s.base : s.queue).filter((x) => T.has(x));
      if (!queue.length) { queue = []; base = []; return; }
      pos = Math.max(0, queue.indexOf(id));
      shuffle = Boolean(s.shuffle);
      repeat = ['off', 'all', 'one'].includes(s.repeat) ? s.repeat : 'off';
      context = s.context || null;
      resumeAt = queue[pos] === id ? Number(s.at) || 0 : 0;
      emit('track');
    }

    return {
      on: (cb) => listeners.add(cb),
      current, isPlaying, toggle, pause, next, prev, seek, seekBy, time, duration,
      playList, playNext, addToQueue, jump, setShuffle, cycleRepeat, restore,
      get queue() { return queue; },
      get pos() { return pos; },
      get shuffle() { return shuffle; },
      get repeat() { return repeat; },
      get context() { return context; }
    };
  })();

  // ------------------------------------------------------------ markup
  const TABS = [['start', 'Start'], ['albums', 'Alben'], ['artists', 'Interpreten'], ['tracks', 'Titel'], ['playlists', 'Playlists'], ['search', 'Suche']];
  const overlay = document.createElement('div');
  overlay.id = 'music-overlay';
  overlay.innerHTML = `
    <div class="mu-bg"><div class="mu-bg-img"></div><div class="mu-bg-img"></div></div>
    <div class="mu-bg-shade"></div>
    <header id="mu-top">
      <div class="mu-brand">${ic('note')}<h1>Musik</h1><span id="mu-count"></span></div>
      <nav id="mu-tabs" data-fgroup>
        ${TABS.map(([id, label]) => `<button class="mu-tab mu-f" type="button" data-act="tab" data-tab="${id}">${id === 'search' ? ic('search') : ''}${label}</button>`).join('')}
        <button class="mu-tab mu-tab-now mu-f" type="button" data-act="now">${EQ}<span>Jetzt läuft</span></button>
      </nav>
      <div id="mu-src"></div>
    </header>
    <main id="mu-page" class="mu-scroll"></main>
    <footer id="mu-bar">
      <div class="mb-prog"><i id="mb-fill"></i></div>
      <div class="mb-art" id="mb-art"></div>
      <div class="mb-text"><b id="mb-title"></b><span id="mb-sub"></span></div>
      <div class="mb-time" id="mb-time"></div>
      <div class="mb-btns">
        <button class="mb-btn mu-f" type="button" data-flast data-act="prev" aria-label="Zurück">${ic('prev')}</button>
        <button class="mb-btn mb-play mu-f" type="button" data-flast data-act="toggle" id="mb-play" aria-label="Abspielen/Pause">${ic('play')}</button>
        <button class="mb-btn mu-f" type="button" data-flast data-act="next" aria-label="Weiter">${ic('next')}</button>
        <button class="mb-btn mu-f" type="button" data-flast data-act="now" aria-label="Jetzt läuft">${ic('expand')}</button>
      </div>
    </footer>
    <section id="mu-now">
      <div class="np-bg"><div class="np-bg-img"></div><div class="np-bg-img"></div></div>
      <div class="np-shade"></div>
      <div class="np-wrap">
        <div class="np-art" id="np-art"></div>
        <div class="np-info">
          <div class="np-kicker" id="np-kicker"></div>
          <h2 id="np-title"></h2>
          <div class="np-sub" id="np-sub"></div>
          <div class="np-seek mu-f" data-act="np-seek" id="np-seek" role="slider" aria-label="Position">
            <div class="np-bar" id="np-bar"><i id="np-fill"></i><b id="np-knob"></b></div>
            <div class="np-times"><span id="np-el">0:00</span><span id="np-hint">◀ ▶ spulen</span><span id="np-rem">0:00</span></div>
          </div>
          <div class="np-ctrl" data-fgroup>
            <button class="np-btn np-small mu-f" type="button" data-act="np-shuffle" id="np-shuffle" aria-label="Zufällige Wiedergabe">${ic('shuffle')}</button>
            <button class="np-btn mu-f" type="button" data-act="prev" aria-label="Zurück">${ic('prev')}</button>
            <button class="np-btn np-play mu-f" type="button" data-act="toggle" id="np-play" aria-label="Abspielen/Pause">${ic('play')}</button>
            <button class="np-btn mu-f" type="button" data-act="next" aria-label="Weiter">${ic('next')}</button>
            <button class="np-btn np-small mu-f" type="button" data-act="np-repeat" id="np-repeat" aria-label="Wiederholen">${ic('repeat')}</button>
          </div>
          <div class="np-acts" data-fgroup>
            <button class="np-pill mu-f" type="button" data-act="np-like" id="np-like">${ic('heart')}<span>Lieblingssong</span></button>
            <button class="np-pill mu-f" type="button" data-act="np-add">${ic('plus')}<span>Playlist</span></button>
            <button class="np-pill mu-f" type="button" data-act="np-queue" id="np-queue">${ic('queue')}<span>Als Nächstes</span></button>
            <button class="np-pill mu-f" type="button" data-act="np-lyrics" id="np-lyrics">${ic('lyrics')}<span>Songtext</span></button>
            <button class="np-pill mu-f" type="button" data-act="np-album">${ic('disc')}<span>Album</span></button>
          </div>
        </div>
        <aside class="np-side" id="np-side">
          <h3 id="np-side-title"></h3>
          <div class="np-side-list mu-scroll" id="np-side-list"></div>
        </aside>
      </div>
    </section>
    <div id="mu-sheet">
      <div class="mu-sheet-panel">
        <div class="mu-sheet-head" id="mu-sheet-head"></div>
        <div class="mu-sheet-list mu-scroll" id="mu-sheet-list"></div>
      </div>
    </div>`;
  document.body.appendChild(overlay);

  const page = $('mu-page');
  const tabsEl = $('mu-tabs');
  const nowEl = $('mu-now');
  const sheetEl = $('mu-sheet');

  // ------------------------------------------------------------ state
  let open = false;
  let tab = 'start';
  let stack = [];          // detail pages over the tab: { view, id, ret, scroll }
  let nowOpen = false;
  let nowReturn = null;
  let side = null;         // 'queue' | 'lyrics' | null (Jetzt läuft)
  let sheetOpen = false;
  let sheetItems = [];
  let sheetReturn = null;
  let focusEl = null;
  let query = '';
  let tabTimer = null;
  const lists = new Map(); // rendered track lists: key -> [track ids]
  const groupMem = new WeakMap();

  // ------------------------------------------------------------ focus engine
  const visible = (el) => el.offsetParent !== null || el.getClientRects().length > 0;
  function scopeEl() { return sheetOpen ? sheetEl : nowOpen ? nowEl : overlay; }
  function focusables(root = scopeEl()) {
    return [...root.querySelectorAll('.mu-f')].filter((el) => visible(el) &&
      (root !== overlay || (!el.closest('#mu-now') && !el.closest('#mu-sheet'))));
  }
  const focusKey = (el) => (el ? [el.dataset.act, el.dataset.id, el.dataset.idx, el.dataset.tab, el.dataset.list].join('|') : '');
  const findKey = (key, root) => focusables(root).find((el) => focusKey(el) === key) || null;

  function setFocus(el, { reveal: doReveal = true, instant = false, pointer = false } = {}) {
    if (!el) return;
    if (focusEl && focusEl !== el) focusEl.classList.remove('focused');
    focusEl = el;
    el.classList.add('focused');
    const grp = el.closest('[data-fgroup]');
    if (grp) groupMem.set(grp, el);
    if (doReveal && !pointer) reveal(el, instant);
    if (el.dataset.bg) setBackdrop(el.dataset.bg);
    // Walking across the tabs switches the page (a moment later, so passing
    // through a tab doesn't render it).
    clearTimeout(tabTimer);
    if (el.dataset.act === 'tab' && (el.dataset.tab !== tab || stack.length)) {
      tabTimer = setTimeout(() => { if (focusEl === el) switchTab(el.dataset.tab, { keepFocus: true }); }, pointer ? 420 : 260);
    }
  }

  function reveal(el, instant) {
    const behavior = instant ? 'auto' : 'smooth';
    const track = el.closest('.mu-track');
    if (track) track.scrollTo({ left: Math.max(0, el.offsetLeft - 56), behavior });
    const cont = el.closest('.mu-scroll');
    if (!cont) return;
    const target = el.closest('.mu-row, .mu-head, .mu-page-head') || el;
    const cr = cont.getBoundingClientRect();
    const tr = target.getBoundingClientRect();
    const top = tr.top - cr.top + cont.scrollTop;
    const bottom = top + tr.height;
    const padTop = 28;
    const padBottom = cont === page ? 150 : 40;
    let y = cont.scrollTop;
    if (top - padTop < y) y = top - padTop;
    else if (bottom + padBottom > y + cont.clientHeight) y = bottom + padBottom - cont.clientHeight;
    if (y < 80) y = 0;
    if (Math.abs(y - cont.scrollTop) > 1) cont.scrollTo({ top: y, behavior });
  }

  // Spatial navigation: the nearest focusable in the arrow's direction.
  // Left/right only within the same visual row, up/down prefer what lies
  // straight below/above. The mini player is only reached when nothing else is there.
  function score(c, r, dir) {
    const cx = (c.left + c.right) / 2;
    const cy = (c.top + c.bottom) / 2;
    const rx = (r.left + r.right) / 2;
    const ry = (r.top + r.bottom) / 2;
    const gapX = Math.max(0, r.left - c.right, c.left - r.right);
    const gapY = Math.max(0, r.top - c.bottom, c.top - r.bottom);
    switch (dir) {
      case 'Right': return rx > cx + 1 && r.right > c.right && gapY === 0 ? gapX + Math.abs(ry - cy) * 0.3 : null;
      case 'Left': return rx < cx - 1 && r.left < c.left && gapY === 0 ? gapX + Math.abs(ry - cy) * 0.3 : null;
      case 'Down': return ry > cy + 1 && r.bottom > c.bottom ? gapY + gapX * 2.5 + Math.abs(rx - cx) * 0.04 : null;
      case 'Up': return ry < cy - 1 && r.top < c.top ? gapY + gapX * 2.5 + Math.abs(rx - cx) * 0.04 : null;
      default: return null;
    }
  }
  function move(dir) {
    const all = focusables();
    if (!focusEl || !all.includes(focusEl)) { setFocus(defaultFocus()); return true; }
    const c = focusEl.getBoundingClientRect();
    const fromBar = focusEl.hasAttribute('data-flast');
    let best = null;
    let bestScore = Infinity;
    let last = null;
    let lastScore = Infinity;
    for (const el of all) {
      if (el === focusEl) continue;
      const s = score(c, el.getBoundingClientRect(), dir);
      if (s == null) continue;
      if (el.hasAttribute('data-flast') && !fromBar) { if (s < lastScore) { lastScore = s; last = el; } continue; }
      if (s < bestScore) { bestScore = s; best = el; }
    }
    let target = best || last;
    if (!target) return false;
    // Coming into a row from above/below: back to where you were in that row.
    const grp = target.closest('[data-fgroup]');
    if (grp && (dir === 'Up' || dir === 'Down') && grp !== focusEl.closest('[data-fgroup]')) {
      const mem = groupMem.get(grp);
      if (mem && mem.isConnected && all.includes(mem)) target = mem;
    }
    setFocus(target);
    return true;
  }
  function bump(dir) {
    if (!focusEl) return;
    focusEl.classList.remove('mu-bump-x', 'mu-bump-y');
    void focusEl.offsetWidth;
    focusEl.classList.add(dir === 'Left' || dir === 'Right' ? 'mu-bump-x' : 'mu-bump-y');
  }
  function defaultFocus() {
    const root = scopeEl();
    if (root === overlay) {
      return page.querySelector('.mu-primary.mu-f') || focusables(page)[0] || tabsEl.querySelector('.mu-tab.active') || focusables()[0];
    }
    return root.querySelector('.np-play') || focusables(root)[0];
  }

  // ------------------------------------------------------------ backdrop
  const bgImgs = overlay.querySelectorAll('.mu-bg-img');
  let bgShown = 0;
  let bgUrl = null;
  let bgTimer = null;
  function setBackdrop(url) {
    if (url === bgUrl) return;
    bgUrl = url;
    clearTimeout(bgTimer);
    bgTimer = setTimeout(() => {
      const show = () => {
        if (bgUrl !== url) return;
        const nextImg = bgImgs[1 - bgShown];
        nextImg.style.backgroundImage = url ? cssUrl(url) : '';
        nextImg.classList.add('on');
        bgImgs[bgShown].classList.remove('on');
        bgShown = 1 - bgShown;
      };
      if (!url) { show(); return; }
      const img = new Image();
      img.onload = show;
      img.onerror = show;
      img.src = url;
    }, 140);
  }

  // Accent colour of the playing cover: progress bars, the play button and the
  // "Jetzt läuft" glow take it on (brightened so it always reads on the dark UI).
  const tintCache = new Map();
  function tintFrom(url) {
    const apply = (rgb) => {
      overlay.style.setProperty('--mu-tint', rgb ? `rgb(${rgb.join(',')})` : '');
      overlay.style.setProperty('--mu-tint-a', rgb ? `rgba(${rgb.join(',')},0.35)` : '');
    };
    if (!url) { apply(null); return; }
    if (tintCache.has(url)) { apply(tintCache.get(url)); return; }
    const img = new Image();
    if (/^https?:/i.test(url)) img.crossOrigin = 'anonymous';
    img.onload = () => {
      let rgb = null;
      try {
        const c = document.createElement('canvas');
        c.width = 24; c.height = 24;
        const ctx = c.getContext('2d', { willReadFrequently: true });
        ctx.drawImage(img, 0, 0, 24, 24);
        const d = ctx.getImageData(0, 0, 24, 24).data;
        let best = null;
        let bestSat = -1;
        let sum = [0, 0, 0];
        for (let i = 0; i < d.length; i += 4) {
          const [r, gg, b] = [d[i], d[i + 1], d[i + 2]];
          sum = [sum[0] + r, sum[1] + gg, sum[2] + b];
          const max = Math.max(r, gg, b);
          const min = Math.min(r, gg, b);
          const sat = max ? (max - min) / max : 0;
          if (max > 60 && sat > bestSat) { bestSat = sat; best = [r, gg, b]; }
        }
        rgb = bestSat > 0.25 ? best : sum.map((v) => v / (d.length / 4));
        const max = Math.max(...rgb) || 1;
        const k = Math.max(1, 200 / max); // lift dark covers
        rgb = rgb.map((v) => Math.min(255, Math.round(v * k)));
      } catch { rgb = null; } // a file:// cover can't be read back: default accent
      tintCache.set(url, rgb);
      apply(rgb);
    };
    img.onerror = () => apply(null);
    img.src = url;
  }

  // ------------------------------------------------------------ building blocks
  const row = (title, count, inner, act = '') => `
    <section class="mu-row">
      <h3>${esc(title)}${count != null ? `<span>${count}</span>` : ''}${act}</h3>
      <div class="mu-track"><div class="mu-strip" data-fgroup>${inner}</div></div>
    </section>`;

  function albumCard(al, sub) {
    const art = al.coverSmall || al.cover;
    return `<div class="mu-card mu-card-album mu-f" data-act="open-album" data-id="${esc(al.id)}"${art ? ` data-bg="${esc(al.cover || art)}"` : ''}>
      ${artHtml(art, al.title, 'mu-card-art', EQ)}
      <div class="mu-card-t">${esc(al.title)}</div>
      <div class="mu-card-s">${esc(sub != null ? sub : al.artist)}</div>
    </div>`;
  }
  function artistCard(ar) {
    const n = ar.albumIds.length;
    const art = ar.image
      ? artHtml(ar.image, ar.name, 'mu-card-art mu-round')
      : `<div class="mu-art mu-card-art mu-round mu-initials" style="${esc(`background:${fallbackBg(ar.name)}`)}">${esc(initials(ar.name))}</div>`;
    return `<div class="mu-card mu-card-artist mu-f" data-act="open-artist" data-id="${esc(ar.id)}"${ar.image ? ` data-bg="${esc(ar.image)}"` : ''}>
      ${art}<div class="mu-card-t">${esc(ar.name)}</div><div class="mu-card-s">${n} ${n === 1 ? 'Album' : 'Alben'}</div></div>`;
  }
  function uniqueCovers(ids, n) {
    const out = [];
    for (const id of ids) {
      const c = smallCoverOf(T.get(id));
      if (c && !out.includes(c)) out.push(c);
      if (out.length >= n) break;
    }
    return out;
  }
  function playlistArt(pl, cls) {
    if (pl.kind === 'likes') return `<div class="mu-art ${cls} mu-pl-likes">${ic('heartFill')}</div>`;
    const covers = uniqueCovers(pl.trackIds, 4);
    if (covers.length >= 4) return `<div class="mu-art ${cls} mu-mosaic">${covers.map((c) => `<i style="${esc(`background-image:${cssUrl(c)}`)}"></i>`).join('')}</div>`;
    if (covers.length) return artHtml(covers[0], pl.name, cls);
    return `<div class="mu-art ${cls}" style="${esc(`background:${fallbackBg(pl.name)}`)}">${ic(pl.kind === 'top' ? 'chart' : 'list', 'mu-art-ic')}</div>`;
  }
  function playlistCard(pl) {
    return `<div class="mu-card mu-card-album mu-f" data-act="open-playlist" data-id="${esc(pl.id)}">
      ${playlistArt(pl, 'mu-card-art')}
      <div class="mu-card-t">${esc(pl.name)}</div><div class="mu-card-s">${pl.trackIds.length} Titel</div></div>`;
  }
  function trackChip(t, key, i) {
    const art = smallCoverOf(t);
    return `<div class="mu-chip mu-f" data-act="play-track" data-list="${key}" data-idx="${i}" data-id="${esc(t.id)}"${art ? ` data-bg="${esc(coverOf(t))}"` : ''}>
      ${artHtml(art, t.album, 'mu-chip-art', EQ)}
      <div class="mu-chip-text"><b>${esc(t.title)}</b><span>${esc(t.artist)}</span></div>
    </div>`;
  }
  function trackList(tracks, key, { showArtist = false, showArt = false, discs = false } = {}) {
    lists.set(key, tracks.map((t) => t.id));
    let lastDisc = null;
    const multi = discs && new Set(tracks.map((t) => t.disc)).size > 1;
    const out = tracks.map((t, i) => {
      let head = '';
      if (multi && t.disc !== lastDisc) { head = `<div class="mu-disc">CD ${t.disc}</div>`; lastDisc = t.disc; }
      const liked = isLiked(t.id);
      const lead = showArt ? artHtml(smallCoverOf(t), t.album, 'mu-tr-art', EQ) : `<span class="mu-tr-no">${t.number || i + 1}</span>${EQ}`;
      const sub = showArtist ? `<small>${esc(t.artist)}${showArt ? ` · ${esc(t.album)}` : ''}</small>` : '';
      return `${head}<div class="mu-tr" data-id="${esc(t.id)}">
        <div class="mu-tr-main mu-f" data-act="play-track" data-list="${key}" data-idx="${i}">
          <span class="mu-tr-lead">${lead}</span>
          <span class="mu-tr-text"><b>${esc(t.title)}</b>${sub}</span>
          <span class="mu-tr-dur">${t.duration ? fmtTime(t.duration) : ''}</span>
        </div>
        <button class="mu-tr-btn mu-f${liked ? ' on' : ''}" type="button" data-act="like" data-id="${esc(t.id)}" aria-label="Lieblingssong">${ic(liked ? 'heartFill' : 'heart')}</button>
        <button class="mu-tr-btn mu-f" type="button" data-act="more" data-id="${esc(t.id)}" data-list="${key}" data-idx="${i}" aria-label="Mehr">${ic('more')}</button>
      </div>`;
    }).join('');
    return `<div class="mu-list">${out}</div>`;
  }
  const btn = (act, icon, label, { id, primary, extra = '' } = {}) =>
    `<button class="mu-btn mu-f${primary ? ' mu-primary' : ''}" type="button" data-act="${act}"${id != null ? ` data-id="${esc(id)}"` : ''}${extra}>${ic(icon)}<span>${esc(label)}</span></button>`;
  const metaLine = (bits) => bits.filter(Boolean).map((b) => `<span>${esc(b)}</span>`).join('<i class="mu-dot"></i>');

  // ------------------------------------------------------------ views
  function stateHtml() {
    if (!lib) {
      const sk = '<div class="mu-card mu-skel"><div class="mu-art mu-card-art"></div><div class="mu-card-t"></div><div class="mu-card-s"></div></div>';
      return `<div class="mu-loading">${row('Musik wird geladen …', null, sk.repeat(8))}</div>`;
    }
    const s = lib.server || {};
    const text = s.url && !s.ok
      ? `Der Musik-Server (${esc(s.url.replace(/^https?:\/\//, ''))}) antwortet nicht. Ist der PC an und der Server gestartet?`
      : 'Starte den Musik-Server auf dem PC (server/ANLEITUNG.md) oder leg Musik in data/music ab – Ordner wie „Interpret/Interpret - Album/01 - Interpret - Titel.mp3“.';
    return `<section class="mu-empty mu-head">
      <div class="mu-empty-ic">${ic('note')}</div>
      <h2>${s.url && !s.ok ? 'PC nicht erreichbar' : 'Noch keine Musik'}</h2>
      <p>${text}</p>
      <div class="mu-actions" data-fgroup>${btn('retry', 'refresh', 'Erneut versuchen', { primary: true })}${btn('setup', 'gear', 'Musik-Server einrichten')}</div>
    </section>`;
  }

  function heroHtml() {
    const t = player.current();
    let al = t ? A.get(t.albumId) : recentAlbums()[0];
    let kicker = t ? (player.isPlaying() ? 'Läuft gerade' : 'Weiter hören') : 'Zuletzt gehört';
    if (!al) { al = featuredAlbum(); kicker = 'Entdecken'; }
    if (!al) return '';
    const art = al.cover || al.coverSmall;
    const primary = t
      ? (player.isPlaying() ? btn('now', 'note', 'Jetzt läuft', { primary: true }) : btn('resume', 'play', 'Weiter hören', { primary: true }))
      : btn('play-album', 'play', 'Abspielen', { id: al.id, primary: true });
    return `<section class="mu-hero mu-head"${art ? ` data-bg="${esc(art)}"` : ''}>
      ${artHtml(art, al.title, 'mu-hero-art')}
      <div class="mu-hero-info">
        <div class="mu-kicker">${esc(kicker)}</div>
        <h2>${esc(t ? t.title : al.title)}</h2>
        <div class="mu-hero-sub">${esc(t ? `${t.artist} · ${t.album}` : `${al.artist}${al.year ? ` · ${al.year}` : ''}`)}</div>
        <div class="mu-actions" data-fgroup>
          ${primary}${btn('mix', 'shuffle', 'Zufallsmix')}${btn('open-album', 'disc', 'Zum Album', { id: al.id })}
        </div>
      </div>
    </section>`;
  }

  function viewStart() {
    if (!lib || !T.size) return stateHtml();
    const recent = recentAlbums();
    const top = topTracks(20);
    const pls = [playlistById('likes'), ...(top.length ? [playlistById('top')] : []), ...playlists.map((p) => playlistById(p.id))].filter(Boolean);
    const albums = albumsSorted();
    const shown = albums.slice(0, 30);
    const more = albums.length > shown.length
      ? `<div class="mu-card mu-card-more mu-f" data-act="tab" data-tab="albums"><div class="mu-art mu-card-art">${ic('disc', 'mu-art-ic')}</div><div class="mu-card-t">Alle ${albums.length} Alben</div><div class="mu-card-s">anzeigen</div></div>`
      : '';
    return [
      heroHtml(),
      recent.length ? row('Zuletzt gehört', null, recent.slice(0, 16).map((a) => albumCard(a)).join('')) : '',
      top.length ? row('Meistgehört', null, (lists.set('top-row', top.map((t) => t.id)), top.map((t, i) => trackChip(t, 'top-row', i)).join(''))) : '',
      row('Deine Playlists', null, pls.map(playlistCard).join('') + newPlaylistCard()),
      row('Alben', albums.length, shown.map((a) => albumCard(a, `${a.artist}${a.year ? ` · ${a.year}` : ''}`)).join('') + more),
      row('Interpreten', R.size, artistsSorted().map(artistCard).join(''))
    ].join('');
  }
  const newPlaylistCard = () => `<div class="mu-card mu-card-more mu-f" data-act="new-playlist">
    <div class="mu-art mu-card-art">${ic('plus', 'mu-art-ic')}</div><div class="mu-card-t">Neue Playlist</div><div class="mu-card-s">erstellen</div></div>`;

  function pageHead(title, count, actions = '') {
    return `<header class="mu-page-head mu-head"><div><h2>${esc(title)}</h2>${count != null ? `<span>${esc(count)}</span>` : ''}</div>
      ${actions ? `<div class="mu-actions" data-fgroup>${actions}</div>` : ''}</header>`;
  }
  function viewAlbums() {
    if (!lib || !T.size) return stateHtml();
    const all = albumsSorted();
    return pageHead('Alben', `${all.length}`, btn('mix', 'shuffle', 'Zufallsmix', { primary: true })) +
      `<div class="mu-grid">${all.map((a) => albumCard(a, `${a.artist}${a.year ? ` · ${a.year}` : ''}`)).join('')}</div>`;
  }
  function viewArtists() {
    if (!lib || !T.size) return stateHtml();
    const all = artistsSorted();
    return pageHead('Interpreten', `${all.length}`) + `<div class="mu-grid mu-grid-artists">${all.map(artistCard).join('')}</div>`;
  }
  function viewTracks() {
    if (!lib || !T.size) return stateHtml();
    const all = tracksSorted();
    return pageHead('Titel', `${all.length}`, btn('play-list', 'play', 'Alle abspielen', { primary: true, extra: ' data-list="all"' }) + btn('shuffle-list', 'shuffle', 'Zufall', { extra: ' data-list="all"' })) +
      trackList(all, 'all', { showArtist: true, showArt: true });
  }
  function viewPlaylists() {
    if (!lib) return stateHtml();
    const top = topTracks(1).length ? [playlistById('top')] : [];
    const pls = [playlistById('likes'), ...top, ...playlists.map((p) => playlistById(p.id))].filter(Boolean);
    return pageHead('Playlists', `${pls.length}`) + `<div class="mu-grid">${newPlaylistCard()}${pls.map(playlistCard).join('')}</div>`;
  }
  function viewSearch() {
    if (!lib || !T.size) return stateHtml();
    let out = `<section class="mu-search mu-head"><div class="mu-actions" data-fgroup>
      <button class="mu-searchbox mu-f${query ? '' : ' mu-primary'}" type="button" data-act="search">${ic('search')}<span>${query ? esc(query) : 'Titel, Album oder Interpret suchen …'}</span></button>
      ${query ? btn('clear-search', 'close', 'Leeren') : ''}</div></section>`;
    if (!query) return out;
    const q = fold(query);
    const hit = (s) => fold(s).includes(q);
    const artists = artistsSorted().filter((a) => hit(a.name));
    const albums = albumsSorted().filter((a) => hit(a.title) || hit(a.artist));
    const tracks = tracksSorted().filter((t) => hit(t.title) || hit(t.artist) || hit(t.album)).slice(0, 80);
    if (!artists.length && !albums.length && !tracks.length) return `${out}<p class="mu-note">Nichts gefunden für „${esc(query)}“.</p>`;
    if (artists.length) out += row('Interpreten', artists.length, artists.map(artistCard).join(''));
    if (albums.length) out += row('Alben', albums.length, albums.map((a) => albumCard(a)).join(''));
    if (tracks.length) out += `<section class="mu-row mu-row-list"><h3>Titel<span>${tracks.length}</span></h3>${trackList(tracks, 'search', { showArtist: true, showArt: true })}</section>`;
    return out;
  }

  function detailHead({ art, kicker, title, link, meta, actions, bg, round }) {
    return `<section class="mu-detail-head mu-head"${bg ? ` data-bg="${esc(bg)}"` : ''}>
      ${art.replace('class="mu-art ', `class="mu-art mu-detail-art${round ? ' mu-round' : ''} `)}
      <div class="mu-detail-info">
        <div class="mu-kicker">${esc(kicker)}</div>
        <h2>${esc(title)}</h2>
        ${link || ''}
        <div class="mu-meta">${meta}</div>
        <div class="mu-actions" data-fgroup>${actions}</div>
      </div>
    </section>`;
  }
  function viewAlbum(id) {
    const al = A.get(id);
    if (!al) return `<p class="mu-note">Album nicht gefunden.</p>`;
    const tracks = albumTracks(al);
    const art = al.cover || al.coverSmall;
    const ar = R.get(al.artistId);
    const others = artistAlbums(ar).filter((a) => a.id !== id);
    return detailHead({
      art: artHtml(art, al.title), bg: art, kicker: al.local ? 'Album · auf diesem Laptop' : 'Album',
      title: al.title,
      link: `<button class="mu-link mu-f" type="button" data-act="open-artist" data-id="${esc(al.artistId)}">${esc(al.artist)}</button>`,
      meta: metaLine([al.year, `${tracks.length} Titel`, al.duration ? fmtDuration(al.duration) : '', al.genre]),
      actions: btn('play-album', 'play', 'Abspielen', { id, primary: true }) + btn('shuffle-album', 'shuffle', 'Zufall', { id }) +
        `<button class="mu-btn mu-btn-ic mu-f" type="button" data-act="album-more" data-id="${esc(id)}" aria-label="Mehr">${ic('more')}</button>`
    }) + trackList(tracks, `album:${id}`, { discs: true, showArtist: tracks.some((t) => t.artist !== al.artist) }) +
      (others.length ? row(`Mehr von ${al.artist}`, null, others.map((a) => albumCard(a, a.year || '')).join('')) : '');
  }
  function viewArtist(id) {
    const ar = R.get(id);
    if (!ar) return `<p class="mu-note">Interpret nicht gefunden.</p>`;
    const albums = artistAlbums(ar);
    const all = artistTracks(ar);
    const top = artistTop(ar, 6);
    const art = ar.image ? artHtml(ar.image, ar.name) : `<div class="mu-art mu-initials" style="${esc(`background:${fallbackBg(ar.name)}`)}">${esc(initials(ar.name))}</div>`;
    return detailHead({
      art, round: true, bg: ar.image || (albums[0] && (albums[0].cover || albums[0].coverSmall)), kicker: 'Interpret', title: ar.name,
      meta: metaLine([`${albums.length} ${albums.length === 1 ? 'Album' : 'Alben'}`, `${all.length} Titel`]),
      actions: btn('play-artist', 'play', 'Abspielen', { id, primary: true }) + btn('shuffle-artist', 'shuffle', 'Zufall', { id })
    }) + `<section class="mu-row mu-row-list"><h3>Beliebte Titel</h3>${trackList(top, `artist:${id}`, { showArt: true, showArtist: false })}</section>` +
      row('Alben', albums.length, albums.map((a) => albumCard(a, `${a.year || ''}${a.year ? ' · ' : ''}${a.trackIds.length} Titel`)).join(''));
  }
  function viewPlaylist(id) {
    const pl = playlistById(id);
    if (!pl) return `<p class="mu-note">Playlist nicht gefunden.</p>`;
    const tracks = pl.trackIds.map((x) => T.get(x));
    const dur = tracks.reduce((s, t) => s + (t.duration || 0), 0);
    const covers = uniqueCovers(pl.trackIds, 1);
    const empty = !tracks.length
      ? `<p class="mu-note">${pl.kind === 'likes' ? 'Noch keine Lieblingssongs. Drück bei einem Titel auf ♥.' : pl.kind === 'top' ? 'Hör ein bisschen Musik – hier landen deine meistgehörten Titel.' : 'Noch leer. Füg Titel über ⋯ → „Zur Playlist hinzufügen“ hinzu.'}</p>`
      : '';
    return detailHead({
      art: playlistArt(pl, ''), bg: covers[0], kicker: pl.kind === 'user' ? 'Playlist' : 'Automatische Playlist', title: pl.name,
      meta: metaLine([`${tracks.length} Titel`, dur ? fmtDuration(dur) : '']),
      actions: (tracks.length ? btn('play-list', 'play', 'Abspielen', { primary: true, extra: ` data-list="pl:${esc(id)}"` }) + btn('shuffle-list', 'shuffle', 'Zufall', { extra: ` data-list="pl:${esc(id)}"` }) : '') +
        (pl.kind === 'user' ? btn('rename-playlist', 'edit', 'Umbenennen', { id }) + btn('delete-playlist', 'trash', 'Löschen', { id }) : '')
    }) + (tracks.length ? trackList(tracks, `pl:${id}`, { showArtist: true, showArt: true }) : empty);
  }

  const VIEWS = { start: viewStart, albums: viewAlbums, artists: viewArtists, tracks: viewTracks, playlists: viewPlaylists, search: viewSearch };
  function currentView() { return stack.length ? stack[stack.length - 1] : { view: tab }; }
  function render({ focus = 'first', keepScroll = false, scroll = 0 } = {}) {
    const v = currentView();
    lists.clear();
    let html;
    if (v.view === 'album') html = viewAlbum(v.id);
    else if (v.view === 'artist') html = viewArtist(v.id);
    else if (v.view === 'playlist') html = viewPlaylist(v.id);
    else html = (VIEWS[v.view] || viewStart)();
    const keepTop = page.scrollTop;
    page.innerHTML = html;
    page.scrollTop = keepScroll ? keepTop : scroll;
    tabsEl.querySelectorAll('.mu-tab').forEach((t) => t.classList.toggle('active', t.dataset.tab === tab && !stack.length));
    overlay.classList.toggle('mu-in-detail', stack.length > 0);
    groupMem.set(tabsEl, tabsEl.querySelector(`.mu-tab[data-tab="${tab}"]`));
    const head = page.querySelector('[data-bg]');
    setBackdrop(head ? head.dataset.bg : null);
    markPlaying();
    updateHeader();
    if (focus === 'first') setFocus(defaultFocus(), { instant: true });
    else if (focus && focus !== 'keep') setFocus(findKey(focus, overlay) || defaultFocus(), { instant: true });
  }

  function switchTab(id, { keepFocus = false } = {}) {
    tab = id;
    stack = [];
    render({ focus: keepFocus ? 'keep' : 'first' });
    if (keepFocus && focusEl && !focusEl.isConnected) setFocus(tabsEl.querySelector(`.mu-tab[data-tab="${id}"]`));
  }
  function push(view, id) {
    stack.push({ view, id, ret: focusKey(focusEl), scroll: page.scrollTop });
    if (stack.length > 12) stack.shift();
    render();
  }
  function back() {
    if (sheetOpen) { closeSheet(); return; }
    if (nowOpen) { closeNow(); return; }
    if (stack.length) {
      const e = stack.pop();
      render({ focus: e.ret, scroll: e.scroll });
      return;
    }
    if (focusEl && !tabsEl.contains(focusEl)) {
      page.scrollTo({ top: 0, behavior: 'smooth' });
      setFocus(tabsEl.querySelector('.mu-tab.active') || tabsEl.querySelector('.mu-tab'));
      return;
    }
    close();
  }

  function markPlaying() {
    overlay.querySelectorAll('.playing').forEach((e) => e.classList.remove('playing'));
    const t = player.current();
    if (!t) return;
    const sel = CSS.escape(t.id);
    overlay.querySelectorAll(`.mu-tr[data-id="${sel}"], .mu-chip[data-id="${sel}"], .np-q[data-id="${sel}"]`).forEach((e) => e.classList.add('playing'));
    overlay.querySelectorAll(`.mu-card-album[data-id="${CSS.escape(t.albumId)}"]`).forEach((e) => e.classList.add('playing'));
  }

  function updateHeader() {
    $('mu-count').textContent = lib && T.size ? `${T.size} Titel` : '';
    const s = (lib && lib.server) || {};
    const src = $('mu-src');
    if (!lib) src.innerHTML = '';
    else if (s.ok) src.innerHTML = `<span class="mu-src-ok">${ic('disc')}${esc(s.name || 'Musik-Server')}</span>`;
    else if (s.url) src.innerHTML = `<span class="mu-src-bad">${ic('refresh')}PC nicht erreichbar</span>`;
    else src.innerHTML = lib.localCount ? `<span class="mu-src-ok">${ic('disc')}Dieser Laptop</span>` : '';
    overlay.classList.toggle('mu-has-track', Boolean(player.current()));
  }

  // ------------------------------------------------------------ mini player
  function updateBar() {
    const t = player.current();
    overlay.classList.toggle('mu-has-track', Boolean(t));
    if (!t) return;
    $('mb-art').innerHTML = artHtml(smallCoverOf(t), t.album, 'mb-cover');
    $('mb-title').textContent = t.title;
    $('mb-sub').textContent = `${t.artist} · ${t.album}`;
    $('mb-play').innerHTML = ic(player.isPlaying() ? 'pause' : 'play');
    updateBarTime();
  }
  function updateBarTime() {
    const d = player.duration();
    const now = player.time();
    $('mb-fill').style.transform = `scaleX(${d ? Math.min(1, now / d) : 0})`;
    $('mb-time').textContent = d ? `${fmtTime(now)} / ${fmtTime(d)}` : fmtTime(now);
  }

  // ------------------------------------------------------------ Jetzt läuft
  const npBg = nowEl.querySelectorAll('.np-bg-img');
  let npBgShown = 0;
  let npTrackId = null;
  function openNow() {
    if (!player.current()) return;
    nowReturn = focusKey(focusEl);
    nowOpen = true;
    nowEl.classList.add('on');
    overlay.classList.add('mu-now-open');
    renderNow(true);
    setFocus($('np-play'), { instant: true });
  }
  function closeNow() {
    nowOpen = false;
    nowEl.classList.remove('on');
    overlay.classList.remove('mu-now-open');
    render({ focus: nowReturn || 'first', keepScroll: true });
  }
  function renderNow(force) {
    const t = player.current();
    if (!t) { if (nowOpen) closeNow(); return; }
    if (force || npTrackId !== t.id) {
      npTrackId = t.id;
      const art = coverOf(t);
      $('np-art').innerHTML = artHtml(art, t.album, 'np-cover');
      const nextBg = npBg[1 - npBgShown];
      nextBg.style.backgroundImage = art ? cssUrl(art) : fallbackBg(t.album);
      nextBg.classList.add('on');
      npBg[npBgShown].classList.remove('on');
      npBgShown = 1 - npBgShown;
      const ctx = player.context;
      $('np-kicker').textContent = ctx && ctx.name ? `${ctx.type === 'album' ? 'Album' : ctx.type === 'artist' ? 'Interpret' : ctx.type === 'playlist' ? 'Playlist' : ''}${ctx.type && ctx.type !== 'mix' && ctx.type !== 'list' ? ' · ' : ''}${ctx.name}` : 'Jetzt läuft';
      $('np-title').textContent = t.title;
      $('np-sub').textContent = `${t.artist} — ${t.album}${t.year ? ` (${t.year})` : ''}`;
      lyricsFor = null;
      if (side) renderSide();
    }
    renderNowButtons();
    updateNowTime();
  }
  function renderNowButtons() {
    const t = player.current();
    if (!t) return;
    $('np-play').innerHTML = ic(player.isPlaying() ? 'pause' : 'play');
    $('np-shuffle').classList.toggle('on', player.shuffle);
    $('np-repeat').classList.toggle('on', player.repeat !== 'off');
    $('np-repeat').innerHTML = ic(player.repeat === 'one' ? 'repeatOne' : 'repeat');
    const liked = isLiked(t.id);
    $('np-like').classList.toggle('on', liked);
    $('np-like').innerHTML = `${ic(liked ? 'heartFill' : 'heart')}<span>${liked ? 'Gefällt dir' : 'Lieblingssong'}</span>`;
    $('np-queue').classList.toggle('on', side === 'queue');
    $('np-lyrics').classList.toggle('on', side === 'lyrics');
    $('np-lyrics').classList.toggle('mu-dim', !t.lyrics);
  }
  function updateNowTime() {
    const d = player.duration();
    const now = player.time();
    const p = d ? Math.min(1, now / d) : 0;
    $('np-fill').style.transform = `scaleX(${p})`;
    $('np-knob').style.left = `${(p * 100).toFixed(2)}%`;
    $('np-el').textContent = fmtTime(now);
    $('np-rem').textContent = d ? `-${fmtTime(d - now)}` : '';
    if (side === 'lyrics') syncLyrics(now);
  }

  // Side panel: Als Nächstes / Songtext.
  function toggleSide(which) {
    side = side === which ? null : which;
    nowEl.classList.toggle('np-side-on', Boolean(side));
    renderNowButtons();
    renderSide();
  }
  function renderSide() {
    const listEl = $('np-side-list');
    if (side === 'queue') {
      const q = player.queue;
      const p = player.pos;
      const upcoming = q.slice(p + 1, p + 101);
      $('np-side-title').innerHTML = `Als Nächstes<span>${player.shuffle ? 'Zufällig' : ''}${player.repeat === 'all' ? ' · Wiederholen' : ''}</span>`;
      const keep = focusEl && listEl.contains(focusEl) ? focusKey(focusEl) : null;
      listEl.innerHTML = upcoming.length
        ? upcoming.map((id, i) => {
          const t = T.get(id);
          if (!t) return '';
          return `<div class="np-q mu-f" data-act="q-jump" data-idx="${p + 1 + i}" data-id="${esc(t.id)}">
            ${artHtml(smallCoverOf(t), t.album, 'np-q-art')}<div class="np-q-text"><b>${esc(t.title)}</b><span>${esc(t.artist)}</span></div>
            <span class="np-q-dur">${t.duration ? fmtTime(t.duration) : ''}</span></div>`;
        }).join('')
        : `<p class="np-side-note">Danach ist Schluss.${player.repeat === 'off' ? ' Mit ↻ wird die Liste wiederholt.' : ''}</p>`;
      listEl.classList.remove('np-lyrics');
      listEl.removeAttribute('data-scroll');
      if (keep) { const el = findKey(keep, nowEl); if (el) setFocus(el, { reveal: false }); }
      markPlaying();
    } else if (side === 'lyrics') {
      $('np-side-title').innerHTML = 'Songtext';
      listEl.classList.add('np-lyrics');
      loadLyrics();
    } else {
      listEl.innerHTML = '';
    }
  }

  // Lyrics: an .lrc next to the song scrolls along line by line; plain text
  // just shows (focus the panel and ▲▼ scrolls it).
  let lyricsFor = null;
  let lrc = null;
  let lrcIdx = -1;
  const lyricsCache = new Map();
  function parseLrc(text) {
    const lines = [];
    String(text).split(/\r?\n/).forEach((raw) => {
      const stamps = [...raw.matchAll(/\[(\d{1,2}):(\d{2})(?:[.:](\d{1,3}))?\]/g)];
      if (!stamps.length) return;
      const words = raw.replace(/\[[^\]]*\]/g, '').trim();
      stamps.forEach((m) => lines.push({ at: Number(m[1]) * 60 + Number(m[2]) + (m[3] ? Number(`0.${m[3]}`) : 0), text: words }));
    });
    return lines.sort((a, b) => a.at - b.at);
  }
  async function loadLyrics() {
    const t = player.current();
    const listEl = $('np-side-list');
    if (!t) return;
    if (lyricsFor === t.id) return;
    lyricsFor = t.id;
    lrc = null;
    lrcIdx = -1;
    if (!t.lyrics) {
      listEl.innerHTML = '<p class="np-side-note">Kein Songtext vorhanden.<br>Leg eine .lrc-Datei mit dem gleichen Namen neben den Song, dann läuft er hier mit.</p>';
      return;
    }
    listEl.innerHTML = '<p class="np-side-note">Songtext wird geladen …</p>';
    let ly = lyricsCache.get(t.id);
    if (ly === undefined) {
      ly = await Promise.resolve(L.musicLyrics(t.id, t.lyrics)).catch(() => null);
      lyricsCache.set(t.id, ly);
    }
    if (lyricsFor !== t.id || side !== 'lyrics') return;
    if (!ly || !ly.text) { listEl.innerHTML = '<p class="np-side-note">Songtext konnte nicht geladen werden.</p>'; return; }
    if (ly.type === 'lrc') {
      lrc = parseLrc(ly.text).filter((l) => l.text);
      listEl.innerHTML = lrc.map((l, i) => `<p class="np-ly" data-i="${i}">${esc(l.text)}</p>`).join('') + '<div class="np-ly-pad"></div>';
      listEl.removeAttribute('data-scroll');
      syncLyrics(player.time());
    } else {
      listEl.innerHTML = `<div class="np-ly-plain mu-f" data-act="np-ly" data-scroll>${ly.text.split(/\r?\n/).map((l) => `<p>${esc(l) || '&nbsp;'}</p>`).join('')}</div>`;
    }
  }
  function syncLyrics(now) {
    if (!lrc || !lrc.length) return;
    let i = -1;
    for (let k = 0; k < lrc.length && lrc[k].at <= now + 0.25; k++) i = k;
    if (i === lrcIdx) return;
    lrcIdx = i;
    const listEl = $('np-side-list');
    listEl.querySelectorAll('.np-ly.on').forEach((e) => e.classList.remove('on'));
    const el = listEl.querySelector(`.np-ly[data-i="${i}"]`);
    if (!el) return;
    el.classList.add('on');
    listEl.scrollTo({ top: Math.max(0, el.offsetTop - listEl.clientHeight * 0.38), behavior: 'smooth' });
  }

  // ------------------------------------------------------------ sheet (⋯ menu)
  function openSheet({ title, sub, art, items }) {
    sheetItems = items;
    sheetReturn = focusEl;
    $('mu-sheet-head').innerHTML = `${art || ''}<div><b>${esc(title)}</b><span>${esc(sub || '')}</span></div>`;
    $('mu-sheet-list').innerHTML = items.map((it, i) => `<button class="mu-sheet-item mu-f${it.danger ? ' mu-danger' : ''}" type="button" data-act="sheet" data-idx="${i}">${ic(it.icon)}<span>${esc(it.label)}</span></button>`).join('');
    sheetOpen = true;
    sheetEl.classList.add('on');
    $('mu-sheet-list').scrollTop = 0;
    setFocus(focusables(sheetEl)[0], { instant: true });
  }
  function closeSheet({ restore = true } = {}) {
    sheetOpen = false;
    sheetEl.classList.remove('on');
    if (restore && sheetReturn && sheetReturn.isConnected) setFocus(sheetReturn, { reveal: false });
  }

  function trackMenu(t, key) {
    const inUserPlaylist = key && key.startsWith('pl:') && playlists.some((p) => `pl:${p.id}` === key);
    const items = [
      { icon: 'queueNext', label: 'Als Nächstes spielen', run: () => { player.playNext([t.id]); toast('Kommt als Nächstes', t.title); } },
      { icon: 'queue', label: 'Zur Warteschlange hinzufügen', run: () => { player.addToQueue([t.id]); toast('Zur Warteschlange hinzugefügt', t.title); } },
      { icon: isLiked(t.id) ? 'heartFill' : 'heart', label: isLiked(t.id) ? 'Aus Lieblingssongs entfernen' : 'Zu Lieblingssongs', run: () => toggleLike(t.id) },
      { icon: 'plus', label: 'Zur Playlist hinzufügen …', run: () => playlistPicker([t.id], t.title) },
      { icon: 'disc', label: 'Zum Album', run: () => { if (nowOpen) closeNow(); push('album', t.albumId); } },
      { icon: 'user', label: 'Zum Interpreten', run: () => { if (nowOpen) closeNow(); push('artist', t.artistId); } }
    ];
    if (inUserPlaylist) {
      items.push({ icon: 'trash', label: 'Aus dieser Playlist entfernen', danger: true, run: () => {
        const p = playlists.find((x) => `pl:${x.id}` === key);
        p.trackIds = p.trackIds.filter((id) => id !== t.id);
        savePlaylists();
        render({ focus: 'keep', keepScroll: true });
        setFocus(defaultFocus());
      } });
    }
    openSheet({ title: t.title, sub: `${t.artist} · ${t.album}`, art: artHtml(smallCoverOf(t), t.album, 'mu-sheet-art'), items });
  }
  function playlistPicker(ids, label) {
    const items = [
      { icon: 'plus', label: 'Neue Playlist …', run: async () => { await newPlaylist(ids); if (!nowOpen) render({ focus: 'keep', keepScroll: true }); } },
      ...playlists.map((p) => ({ icon: 'list', label: `${p.name} (${p.trackIds.length})`, run: () => addToPlaylist(p, ids) }))
    ];
    openSheet({ title: 'Zur Playlist hinzufügen', sub: label, items });
  }

  // ------------------------------------------------------------ actions
  const listIds = (key) => {
    if (key === 'all') return tracksSorted().map((t) => t.id);
    if (key && key.startsWith('pl:')) { const pl = playlistById(key.slice(3)); return pl ? pl.trackIds : []; }
    return lists.get(key) || [];
  };
  function contextFor(key) {
    if (!key) return null;
    if (key.startsWith('album:')) { const al = A.get(key.slice(6)); return al && { type: 'album', id: al.id, name: al.title }; }
    if (key.startsWith('artist:')) { const ar = R.get(key.slice(7)); return ar && { type: 'artist', id: ar.id, name: ar.name }; }
    if (key.startsWith('pl:')) { const pl = playlistById(key.slice(3)); return pl && { type: 'playlist', id: pl.id, name: pl.name }; }
    if (key === 'all') return { type: 'list', name: 'Alle Titel' };
    if (key === 'search') return { type: 'list', name: `Suche „${query}“` };
    if (key === 'top-row') return { type: 'list', name: 'Meistgehört' };
    return null;
  }

  async function activate(el) {
    if (!el) return;
    const d = el.dataset;
    const id = d.id;
    switch (d.act) {
      case 'tab': clearTimeout(tabTimer); if (d.tab !== tab || stack.length) switchTab(d.tab); else setFocus(defaultFocus()); return;
      case 'now': openNow(); return;
      case 'open-album': push('album', id); return;
      case 'open-artist': push('artist', id); return;
      case 'open-playlist': push('playlist', id); return;
      case 'play-album': { const al = A.get(id); if (al) player.playList(al.trackIds, 0, { shuffle: false, context: { type: 'album', id, name: al.title } }); return; }
      case 'shuffle-album': { const al = A.get(id); if (al) player.playList(al.trackIds, 0, { shuffle: true, context: { type: 'album', id, name: al.title } }); return; }
      case 'play-artist': { const ar = R.get(id); if (ar) player.playList(artistTracks(ar).map((t) => t.id), 0, { shuffle: false, context: { type: 'artist', id, name: ar.name } }); return; }
      case 'shuffle-artist': { const ar = R.get(id); if (ar) player.playList(artistTracks(ar).map((t) => t.id), 0, { shuffle: true, context: { type: 'artist', id, name: ar.name } }); return; }
      case 'play-list': player.playList(listIds(d.list), 0, { shuffle: false, context: contextFor(d.list) }); return;
      case 'shuffle-list': player.playList(listIds(d.list), 0, { shuffle: true, context: contextFor(d.list) }); return;
      case 'play-track': {
        const ids = listIds(d.list);
        const i = Number(d.idx) || 0;
        // OK on the song that is already playing: straight to "Jetzt läuft".
        const t = player.current();
        if (t && ids[i] === t.id && player.isPlaying()) { openNow(); return; }
        player.playList(ids, i, { context: contextFor(d.list), randomStart: false });
        return;
      }
      case 'mix': player.playList([...T.keys()], 0, { shuffle: true, context: { type: 'mix', name: 'Zufallsmix' } }); openNow(); return;
      case 'resume': player.toggle(); return;
      case 'like': toggleLike(id); return;
      case 'more': { const t = T.get(id); if (t) trackMenu(t, d.list); return; }
      case 'album-more': {
        const al = A.get(id);
        if (!al) return;
        openSheet({
          title: al.title, sub: al.artist, art: artHtml(al.coverSmall || al.cover, al.title, 'mu-sheet-art'),
          items: [
            { icon: 'queueNext', label: 'Als Nächstes spielen', run: () => { player.playNext(al.trackIds); toast('Kommt als Nächstes', al.title); } },
            { icon: 'queue', label: 'Zur Warteschlange hinzufügen', run: () => { player.addToQueue(al.trackIds); toast('Zur Warteschlange hinzugefügt', al.title); } },
            { icon: 'plus', label: 'Zur Playlist hinzufügen …', run: () => playlistPicker(al.trackIds, al.title) },
            { icon: 'user', label: 'Zum Interpreten', run: () => push('artist', al.artistId) }
          ]
        });
        return;
      }
      case 'new-playlist': { const p = await newPlaylist(); if (p) push('playlist', p.id); return; }
      case 'rename-playlist': {
        const p = playlists.find((x) => x.id === id);
        if (!p) return;
        const name = await showKeyboard({ label: 'Neuer Name der Playlist', initialValue: p.name });
        if (name && name.trim()) { p.name = name.trim(); savePlaylists(); render({ focus: 'keep', keepScroll: true }); }
        return;
      }
      case 'delete-playlist': {
        const p = playlists.find((x) => x.id === id);
        if (!p) return;
        openSheet({
          title: `„${p.name}“ löschen?`, sub: `${p.trackIds.length} Titel · die Songs selbst bleiben erhalten`,
          items: [
            { icon: 'trash', label: 'Ja, Playlist löschen', danger: true, run: () => { playlists = playlists.filter((x) => x.id !== id); savePlaylists(); back(); toast('Playlist gelöscht', p.name); } },
            { icon: 'close', label: 'Abbrechen', run: () => {} }
          ]
        });
        return;
      }
      case 'search': {
        const q = await showKeyboard({ label: 'Suche nach Titel, Album oder Interpret', initialValue: query });
        if (q !== null) { query = q.trim(); render(); const first = page.querySelector('.mu-row .mu-f, .mu-list .mu-f'); if (first) setFocus(first); }
        return;
      }
      case 'clear-search': query = ''; render(); return;
      case 'retry': fetchLibrary(true).then(() => render()); render(); return;
      case 'setup': close(); setTimeout(() => g(() => window.openSettings('media')), 250); return;
      case 'sheet': { const it = sheetItems[Number(d.idx)]; closeSheet({ restore: true }); if (it) it.run(); return; }
      // transport (mini player + Jetzt läuft)
      case 'toggle': player.toggle(); return;
      case 'prev': player.prev(); return;
      case 'next': player.next(); return;
      case 'np-shuffle': player.setShuffle(!player.shuffle); return;
      case 'np-repeat': player.cycleRepeat(); return;
      case 'np-like': { const t = player.current(); if (t) toggleLike(t.id); return; }
      case 'np-add': { const t = player.current(); if (t) playlistPicker([t.id], t.title); return; }
      case 'np-queue': toggleSide('queue'); return;
      case 'np-lyrics': toggleSide('lyrics'); return;
      case 'np-album': { const t = player.current(); if (t) { closeNow(); push('album', t.albumId); } return; }
      case 'np-seek': player.toggle(); return;
      case 'q-jump': player.jump(Number(d.idx)); return;
      default:
    }
  }

  // ------------------------------------------------------------ keys
  function handleKey(e) {
    const k = e.key;
    if (k === 'Escape' || k === 'Backspace') { if (!e.repeat) back(); return; }
    if (k === 'MediaPlayPause' || k === ' ') { player.toggle(); return; }
    if (k === 'MediaTrackNext') { player.next(); return; }
    if (k === 'MediaTrackPrevious') { player.prev(); return; }
    if (k.startsWith('Arrow')) {
      const dir = k.slice(5);
      if (focusEl && focusEl.dataset.act === 'np-seek' && (dir === 'Left' || dir === 'Right')) {
        player.seekBy(dir === 'Left' ? -10 : 10);
        focusEl.classList.add('np-seeking');
        clearTimeout(seekHintTimer);
        seekHintTimer = setTimeout(() => $('np-seek').classList.remove('np-seeking'), 900);
        return;
      }
      if (focusEl && focusEl.hasAttribute('data-scroll') && (dir === 'Up' || dir === 'Down')) {
        const box = focusEl.closest('.mu-scroll') || focusEl;
        const before = box.scrollTop;
        box.scrollBy({ top: dir === 'Down' ? 140 : -140, behavior: 'smooth' });
        if (dir === 'Down' || before > 0) return;
      }
      if (!move(dir)) bump(dir);
      return;
    }
    if (k === 'Enter' && !e.repeat) activate(focusEl);
  }
  let seekHintTimer = null;

  // ------------------------------------------------------------ pointer / mouse
  function pointerHover(target) {
    if (!open || !target || !target.closest) return;
    const el = target.closest('.mu-f');
    if (!el || el === focusEl || !scopeEl().contains(el)) return;
    if (scopeEl() === overlay && (el.closest('#mu-now') || el.closest('#mu-sheet'))) return;
    setFocus(el, { pointer: true });
  }
  let lastMouse = [-1, -1];
  overlay.addEventListener('mousemove', (e) => {
    if (Math.abs(e.clientX - lastMouse[0]) + Math.abs(e.clientY - lastMouse[1]) < 3) return;
    lastMouse = [e.clientX, e.clientY];
    pointerHover(e.target);
  });
  overlay.addEventListener('click', (e) => {
    if (!open) return;
    if (sheetOpen && e.target === sheetEl) { closeSheet(); return; }
    const el = e.target.closest('.mu-f');
    if (!el || !scopeEl().contains(el)) return;
    if (el.dataset.act === 'np-seek') {
      const bar = $('np-bar').getBoundingClientRect();
      if (e.clientX >= bar.left - 10 && e.clientX <= bar.right + 10) {
        player.seek(((e.clientX - bar.left) / bar.width) * player.duration());
        setFocus(el, { pointer: true });
        return;
      }
    }
    setFocus(el, { pointer: true });
    activate(el);
  });

  // ------------------------------------------------------------ open / close
  function openMusic() {
    if (open) return;
    open = true;
    animatedOpen(overlay);
    if (!lib) fetchLibrary().then(() => { if (open) render(); });
    else fetchLibrary(); // refresh in the background (new albums on the PC)
    render();
  }
  function close() {
    if (!open) return;
    open = false;
    if (sheetOpen) closeSheet({ restore: false });
    if (nowOpen) { nowOpen = false; nowEl.classList.remove('on'); overlay.classList.remove('mu-now-open'); }
    clearTimeout(tabTimer);
    animatedClose(overlay, { duration: 220 });
    // Back on the home screen: the focus ring goes onto "Music" again.
    g(() => { const nav = document.getElementById('nav-music'); if (nav && !contentOpen) setFocusEl(nav); });
  }

  let firstLibrary = true;
  function onLibrary() {
    if (firstLibrary) { firstLibrary = false; player.restore(); }
    if (!open) return;
    // Re-render only the lists (not while the user is inside a sub-menu).
    if (!sheetOpen && !nowOpen) render({ focus: focusKey(focusEl) || 'first', keepScroll: true });
    else updateHeader();
  }

  // ------------------------------------------------------------ player -> UI, system
  const navMusic = () => document.getElementById('nav-music');
  player.on((what) => {
    if (what === 'time') {
      if (open) updateBarTime();
      if (nowOpen) updateNowTime();
      return;
    }
    const playing = player.isPlaying();
    overlay.classList.toggle('mu-is-playing', playing);
    g(() => navMusic().classList.toggle('nav-playing', playing));
    updateBar();
    if (open) { markPlaying(); updateHeader(); }
    if (nowOpen) { renderNow(); if (what === 'queue' && side === 'queue') renderSide(); }
    if (what === 'track') {
      const t = player.current();
      tintFrom(t ? smallCoverOf(t) : null);
      if (nowOpen && side === 'queue') renderSide();
    }
    updateSession();
    const t = player.current();
    g(() => L.remoteNowPlaying(t ? { title: t.title, artist: t.artist, playing } : null)); // the phone's music strip
    changeListeners.forEach((cb) => g(() => cb(what)));
  });

  // Windows' media overlay, keyboard media keys and Bluetooth headphones.
  function updateSession() {
    if (!('mediaSession' in navigator)) return;
    const t = player.current();
    if (!t) { navigator.mediaSession.metadata = null; return; }
    const art = coverOf(t);
    g(() => {
      navigator.mediaSession.metadata = new MediaMetadata({
        title: t.title, artist: t.artist, album: t.album,
        artwork: art && /^https?:/i.test(art) ? [{ src: art, sizes: '512x512' }] : []
      });
      navigator.mediaSession.playbackState = player.isPlaying() ? 'playing' : 'paused';
    });
  }
  if ('mediaSession' in navigator) {
    const set = (a, fn) => g(() => navigator.mediaSession.setActionHandler(a, fn));
    set('play', () => player.toggle());
    set('pause', () => player.pause());
    set('previoustrack', () => player.prev());
    set('nexttrack', () => player.next());
    set('seekto', (d) => player.seek(d.seekTime));
    set('seekbackward', () => player.seekBy(-10));
    set('seekforward', () => player.seekBy(10));
  }
  // Media keys on the home screen too (inside the player handleKey has them).
  document.addEventListener('keydown', (e) => {
    if (open || !player.current()) return;
    if (e.key === 'MediaPlayPause') player.toggle();
    else if (e.key === 'MediaTrackNext') player.next();
    else if (e.key === 'MediaTrackPrevious') player.prev();
  });

  // A film, Live TV or a web app takes over the sound: the music pauses.
  g(() => onHomeVisibilityChange(() => {
    if (contentOpen && player.isPlaying()) {
      player.pause();
      toast('Musik pausiert', 'Weiter geht’s im Kontrollzentrum oder unter Music.', 'info');
    }
  }));
  // Home button: the player closes completely, whatever sub-page was open.
  document.addEventListener('DOMContentLoaded', () => g(() => window.tvos.on('home', close)), { once: true });

  // Boot (boot.js): library + covers in the background, so the first open is instant.
  async function prefetch() {
    const l = await fetchLibrary();
    if (!l) return [];
    return [...new Set([...l.albums.map((a) => a.coverSmall || a.cover), ...l.artists.map((a) => a.image)].filter(Boolean))];
  }

  const changeListeners = new Set();
  window.music = {
    open: openMusic,
    close,
    isOpen: () => open,
    handleKey,
    pointerHover,
    prefetch,
    reload: () => fetchLibrary(true).then(() => { if (open) render({ focus: 'keep', keepScroll: true }); }),
    // For the Control Center / sleep timer.
    state: () => ({ track: player.current(), playing: player.isPlaying(), cover: smallCoverOf(player.current()) }),
    toggle: () => player.toggle(),
    pause: () => player.pause(),
    next: () => player.next(),
    prev: () => player.prev(),
    openNowPlaying: () => { openMusic(); setTimeout(openNow, 60); },
    // Boot rehearsal (boot.js): every screen rendered and animated once
    // behind the splash, without touching playback.
    rehearse: async (sleep) => {
      await fetchLibrary();
      openMusic();
      await sleep(900);
      for (const id of ['albums', 'artists', 'tracks', 'playlists', 'start']) { switchTab(id); await sleep(350); move('Down'); await sleep(250); }
      const al = albumsSorted()[0];
      if (al) { push('album', al.id); await sleep(700); move('Down'); await sleep(300); back(); await sleep(300); }
      const ar = artistsSorted()[0];
      if (ar) { push('artist', ar.id); await sleep(600); back(); await sleep(300); }
      if (player.current()) { openNow(); await sleep(700); toggleSide('queue'); await sleep(400); toggleSide('queue'); closeNow(); await sleep(300); }
      close();
      await sleep(400);
    },
    onChange: (cb) => changeListeners.add(cb)
  };

  // Library early (queue restore, Control Center) even if the boot preload is skipped.
  setTimeout(() => { if (!lib) fetchLibrary(); }, 1500);
})();
