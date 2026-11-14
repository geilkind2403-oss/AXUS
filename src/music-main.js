// --- Music (main process): the library the music player (music.js) shows.
// Two sources, merged:
//   - the Musik-Server on the PC (server/musicserver.js, port 8421), address in
//     data/musicserver.json ({ "url": "http://192.168.0.21:8421" }). Without
//     that file the film server's PC is tried on port 8421 -- both servers
//     normally run on the same PC.
//   - music on this laptop in data/music, same folder layout as on the server
//     (<Interpret>/<Interpret> - <Album>/01 - <Interpret> - <Titel>.mp3).
// The same scanner does both (it lives in server/musicserver.js).
const { ipcMain } = require('electron');
const path = require('path');
const fs = require('fs');
const { pathToFileURL } = require('url');

const DEFAULT_PORT = 8421;
const fetchFn = globalThis.fetch || require('cross-fetch');

let ctx = null;
let serverFile = '';
let localDir = '';
let artDir = '';
let scanner = null;
let readLyrics = null;
let localTracks = new Map();
let cached = null;
let cachedAt = 0;
let listing = null;

async function fetchWithTimeout(url, ms = 4000) {
  const ctrl = new AbortController();
  const t = setTimeout(() => ctrl.abort(), ms);
  try { return await fetchFn(url, { signal: ctrl.signal }); } finally { clearTimeout(t); }
}

function normalizeUrl(raw) {
  let url = String(raw || '').trim();
  if (!url) return '';
  if (!/^https?:\/\//i.test(url)) url = `http://${url}`;
  if (!/:\d+(\/|$)/.test(url.replace(/^https?:\/\//i, ''))) url += `:${DEFAULT_PORT}`;
  return url.replace(/\/+$/, '');
}

// { url, auto }: auto = derived from the film server, not set by hand.
function getServerUrl() {
  try {
    const saved = JSON.parse(fs.readFileSync(serverFile, 'utf8'));
    if (saved && typeof saved.url === 'string') return { url: saved.url.replace(/\/+$/, ''), auto: false };
  } catch { /* not set up by hand */ }
  const media = ctx.getMediaServerUrl();
  if (!media) return { url: '', auto: true };
  try {
    const u = new URL(media);
    return { url: `${u.protocol}//${u.hostname}:${DEFAULT_PORT}`, auto: true };
  } catch { return { url: '', auto: true }; }
}

async function ping(url) {
  if (!url) return { ok: false, url: '', error: 'Kein Server eingetragen' };
  try {
    const res = await fetchWithTimeout(`${url}/api/ping`, 3000);
    const j = await res.json();
    if (j.type !== 'music') return { ok: false, url, error: 'Das ist kein Musik-Server' };
    return { ok: true, url, name: j.name, artists: j.artists, albums: j.albums, tracks: j.tracks };
  } catch {
    return { ok: false, url, error: 'Server nicht erreichbar' };
  }
}

async function setServerUrl(raw) {
  const url = normalizeUrl(raw);
  if (!url) {
    fs.writeFileSync(serverFile, JSON.stringify({ url: '' }));
    cached = null;
    return { ok: true, url: '' };
  }
  const p = await ping(url);
  if (!p.ok) return p;
  fs.writeFileSync(serverFile, JSON.stringify({ url }, null, 2));
  cached = null;
  return p;
}

// ------------------------------------------------------------ library
// A cover that only exists inside the MP3 (no Folder.jpg): written out once
// to data/music-art, so the player can show it like any other image.
function embeddedCover(album) {
  const pic = album.picture;
  if (!pic || !pic.len) return null;
  const ext = /png/i.test(pic.mime) ? '.png' : '.jpg';
  const out = path.join(artDir, `${album.id}${ext}`);
  try {
    if (!fs.existsSync(out) || fs.statSync(out).mtimeMs < fs.statSync(pic.file).mtimeMs) {
      const buf = Buffer.alloc(pic.len);
      const fd = fs.openSync(pic.file, 'r');
      try { fs.readSync(fd, buf, 0, pic.len, pic.pos); } finally { fs.closeSync(fd); }
      fs.mkdirSync(artDir, { recursive: true });
      fs.writeFileSync(out, buf);
    }
    return out;
  } catch { return null; }
}

function localLibrary() {
  localTracks = new Map();
  if (!scanner || !fs.existsSync(localDir)) return { artists: [], albums: [], tracks: [] };
  const lib = scanner.scan();
  const file = (f) => (f ? pathToFileURL(f).href : null);
  lib.tracks.forEach((t) => localTracks.set(`local:${t.id}`, t));
  return {
    artists: lib.artists.map((a) => ({ id: a.id, name: a.name, image: file(a.imageFile), albumIds: a.albumIds.map((id) => `local:${id}`) })),
    albums: lib.albums.map((a) => {
      const cover = a.coverFile || embeddedCover(a);
      return {
        id: `local:${a.id}`, title: a.title, artist: a.artist, artistId: a.artistId, year: a.year, genre: a.genre,
        duration: a.duration, discs: a.discs, trackIds: a.trackIds.map((id) => `local:${id}`),
        cover: file(cover), coverSmall: file(a.coverSmallFile || cover), local: true
      };
    }),
    tracks: lib.tracks.map((t) => ({
      id: `local:${t.id}`, title: t.title, artist: t.artist, album: t.album, albumId: `local:${t.albumId}`, artistId: t.artistId,
      number: t.number, disc: t.disc, duration: t.duration, genre: t.genre, year: t.year,
      stream: file(t.file), lyrics: t.lyricsFile || t.lyricsTag ? 'local' : null, local: true
    }))
  };
}

async function serverLibrary() {
  const { url } = getServerUrl();
  if (!url) return { lib: null, status: { ok: false, url: '' } };
  try {
    const res = await fetchWithTimeout(`${url}/api/library`, 8000);
    const lib = await res.json();
    if (!lib || !Array.isArray(lib.tracks)) throw new Error('bad library');
    return { lib, status: { ok: true, url, name: lib.server } };
  } catch {
    return { lib: null, status: { ok: false, url, error: 'Server nicht erreichbar' } };
  }
}

// Artists are keyed by name (same id on both sides), so an artist with albums
// on the laptop and on the PC shows up once.
function merge(local, remote) {
  const artists = new Map();
  const add = (lib) => {
    if (!lib) return;
    for (const a of lib.artists) {
      const have = artists.get(a.id);
      if (have) { have.albumIds.push(...a.albumIds); if (!have.image) have.image = a.image; } else artists.set(a.id, { ...a, albumIds: [...a.albumIds] });
    }
  };
  add(local);
  add(remote);
  return {
    artists: [...artists.values()],
    albums: [...local.albums, ...(remote ? remote.albums : [])],
    tracks: [...local.tracks, ...(remote ? remote.tracks : [])]
  };
}

async function list({ fresh = false } = {}) {
  if (!fresh && cached && Date.now() - cachedAt < 30000) return cached;
  if (!listing) {
    listing = (async () => {
      let local = { artists: [], albums: [], tracks: [] };
      try { local = localLibrary(); } catch (err) { console.error('Lokale Musik:', err.message); }
      const { lib, status } = await serverLibrary();
      cached = { ...merge(local, lib), server: status, localCount: local.tracks.length };
      cachedAt = Date.now();
      return cached;
    })().finally(() => { listing = null; });
  }
  return listing;
}

async function lyrics(trackId, url) {
  const t = localTracks.get(trackId);
  if (t) return readLyrics ? readLyrics(t) : null;
  if (!url || !/^https?:\/\//i.test(url)) return null;
  try {
    const res = await fetchWithTimeout(url, 4000);
    return res.ok ? await res.json() : null;
  } catch { return null; }
}

function init(options) {
  ctx = options;
  serverFile = path.join(options.dataDir, 'musicserver.json');
  localDir = path.join(options.dataDir, 'music');
  artDir = path.join(options.dataDir, 'music-art');
  try { fs.mkdirSync(localDir, { recursive: true }); } catch { /* read-only data dir: no local music */ }
  try {
    // Shipped next to src/ (server/). A launcher copied without that folder
    // still plays music from the server, just not from data/music.
    const m = require(path.join(__dirname, '..', 'server', 'musicserver.js'));
    scanner = m.createScanner({ root: localDir, cacheFile: path.join(options.dataDir, 'music-cache.json') });
    readLyrics = m.readLyrics;
  } catch (err) {
    console.error('server/musicserver.js fehlt -- Musik aus data/music ist aus:', err.message);
  }

  ipcMain.handle('music-library', (event, opts) => list(opts || {}));
  ipcMain.handle('music-lyrics', (event, trackId, url) => lyrics(String(trackId || ''), url));
  ipcMain.handle('music-get-server', async () => {
    const { url, auto } = getServerUrl();
    return { ...(await ping(url)), auto };
  });
  ipcMain.handle('music-set-server', (event, url) => setServerUrl(url));
}

module.exports = { init };
