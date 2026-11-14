// TV Launcher Media-Server -- streams the film library to the launcher over
// the LAN. One file, plain Node.js, no npm packages, so it runs the same in
// Docker (see Dockerfile) or with a bare `node mediaserver.js`.
//
// Library schema (same as the launcher's local data/media):
//   <MEDIA_DIR>/.../<Film>/<video>.mp4|mkv|webm|mov|m4v
//                         /Name/*.txt  /Info/*.txt  /FSK/*.txt
//                         /Personas/*.txt  /Madeby/*.txt
//                         /Icon/*.jpg|png|webp  (/Icon_2)  /Banner/*.jpg|png|webp
//
// API (all JSON, CORS open -- it's a LAN service for the launcher):
//   GET  /api/ping                 -> { name, version, films }
//   GET  /api/library              -> [ { id, title, info, fsk, cast, crew, genre, duration, size, icon, banner, stream, position } ]
//   GET  /stream/<id>              -> the video, with HTTP Range (seek anywhere, nothing is preloaded)
//   GET  /play/<id>?t=&a=&v=       -> MKV/AVI/TS/...: remuxed live to MP4 (videotools.js; needs ffmpeg)
//   GET  /seek/<id>?t=             -> { t } where such a stream really starts (the keyframe before t)
//   GET  /sub/<id>/<n>?t=          -> a text subtitle of such a film as WebVTT
//   GET  /img/<id>/icon|banner     -> cover / banner image
//   POST /api/progress/<id>        -> body { seconds } : resume position, shared by every device
//
// Settings via environment:
//   MEDIA_DIR   library root              (default: ./media, in Docker: /media)
//   DATA_DIR    where progress.json lives (default: ./data, in Docker: /data)
//   PORT        HTTP port                 (default: 8420)
//   SERVER_NAME shown to the launcher     (default: the host name)
//   API_KEY     optional shared key: requests need ?key=... or header X-Api-Key
'use strict';

const http = require('http');
const fs = require('fs');
const path = require('path');
const os = require('os');
const crypto = require('crypto');
const { pipeline } = require('stream');
const { createVideoTools } = require('./videotools');

const VERSION = '1.0.0';
const MEDIA_DIR = path.resolve(process.env.MEDIA_DIR || path.join(__dirname, 'media'));
const DATA_DIR = path.resolve(process.env.DATA_DIR || path.join(__dirname, 'data'));
const PORT = Number(process.env.PORT) || 8420;
const SERVER_NAME = process.env.SERVER_NAME || os.hostname();
const API_KEY = process.env.API_KEY || '';

const VIDEO_TYPES = {
  '.mp4': 'video/mp4', '.m4v': 'video/mp4', '.mov': 'video/quicktime', '.webm': 'video/webm', '.mkv': 'video/x-matroska',
  '.avi': 'video/x-msvideo', '.ts': 'video/mp2t', '.m2ts': 'video/mp2t', '.mts': 'video/mp2t', '.wmv': 'video/x-ms-wmv', '.mpg': 'video/mpeg', '.mpeg': 'video/mpeg'
};
const IMAGE_TYPES = { '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.png': 'image/png', '.webp': 'image/webp' };
const META_DIRS = new Set(['banner', 'fsk', 'icon', 'icon_2', 'info', 'madeby', 'name', 'personas']);

const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);
const video = createVideoTools({ cacheDir: DATA_DIR, log });

// ------------------------------------------------------------ library scan
function readMetaText(folder, sub) {
  try {
    const dir = path.join(folder, sub);
    const f = fs.readdirSync(dir).find((n) => n.toLowerCase().endsWith('.txt'));
    return f ? fs.readFileSync(path.join(dir, f), 'utf8').trim() : null;
  } catch { return null; }
}
function findMetaImage(folder, sub) {
  try {
    const dir = path.join(folder, sub);
    const f = fs.readdirSync(dir).find((n) => IMAGE_TYPES[path.extname(n).toLowerCase()]);
    return f ? path.join(dir, f) : null;
  } catch { return null; }
}

// MP4/MOV duration straight from the file's "mvhd" box -- no ffprobe needed.
// Walks the top-level boxes to "moov" (which may sit at the end of the file),
// reads only a few KB. Other containers (mkv, webm) report null.
function mp4Duration(file) {
  let fd;
  try {
    fd = fs.openSync(file, 'r');
    const size = fs.fstatSync(fd).size;
    const head = Buffer.alloc(16);
    let pos = 0;
    while (pos + 8 <= size) {
      fs.readSync(fd, head, 0, 16, pos);
      let boxSize = head.readUInt32BE(0);
      const type = head.toString('latin1', 4, 8);
      let headerLen = 8;
      if (boxSize === 1) { boxSize = Number(head.readBigUInt64BE(8)); headerLen = 16; }
      else if (boxSize === 0) boxSize = size - pos;
      if (boxSize < headerLen) return null;
      if (type === 'moov') {
        const len = Math.min(boxSize - headerLen, 4 * 1024 * 1024);
        const moov = Buffer.alloc(len);
        fs.readSync(fd, moov, 0, len, pos + headerLen);
        const at = moov.indexOf('mvhd', 0, 'latin1');
        if (at < 4) return null;
        const v = at + 4; // version byte
        const version = moov[v];
        const timescale = version === 1 ? moov.readUInt32BE(v + 20) : moov.readUInt32BE(v + 12);
        const duration = version === 1 ? Number(moov.readBigUInt64BE(v + 24)) : moov.readUInt32BE(v + 16);
        return timescale ? Math.round(duration / timescale) : null;
      }
      pos += boxSize;
    }
    return null;
  } catch { return null; } finally { if (fd !== undefined) fs.closeSync(fd); }
}

function idFor(relPath) {
  return crypto.createHash('sha1').update(relPath.replace(/\\/g, '/')).digest('hex').slice(0, 12);
}

function scan(dir, out = []) {
  let entries;
  try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return out; }
  const video = entries.find((e) => e.isFile() && VIDEO_TYPES[path.extname(e.name).toLowerCase()]);
  if (video) {
    const file = path.join(dir, video.name);
    const rel = path.relative(MEDIA_DIR, file);
    const rawName = readMetaText(dir, 'Name');
    const info = readMetaText(dir, 'Info');
    const genre = info && (info.match(/Genre:\s*([^\n]+)/i) || [])[1];
    const stat = fs.statSync(file);
    out.push({
      id: idFor(rel),
      title: (rawName ? rawName.replace(/^-/, '').trim() : '') || path.basename(dir),
      relativePath: rel,
      file,
      mtimeMs: stat.mtimeMs,
      size: stat.size,
      type: VIDEO_TYPES[path.extname(file).toLowerCase()],
      info,
      fsk: readMetaText(dir, 'FSK'),
      cast: readMetaText(dir, 'Personas'),
      crew: readMetaText(dir, 'Madeby'),
      genre: genre ? genre.split(',')[0].trim() : null,
      iconFile: findMetaImage(dir, 'Icon') || findMetaImage(dir, 'Icon_2'),
      bannerFile: findMetaImage(dir, 'Banner')
    });
  }
  for (const e of entries) {
    if (e.isDirectory() && !META_DIRS.has(e.name.toLowerCase()) && !e.name.startsWith('.')) scan(path.join(dir, e.name), out);
  }
  return out;
}

// The library is rescanned at most every 30 s (on demand), so new films show
// up without a restart; durations are cached per file + mtime.
let library = [];
let byId = new Map();
let scannedAt = 0;
const durations = new Map();
function getLibrary() {
  if (Date.now() - scannedAt < 30000) return library;
  library = scan(MEDIA_DIR);
  for (const f of library) {
    const key = `${f.file}|${f.mtimeMs}`;
    if (!durations.has(key)) {
      const probed = video.needsRemux(f.file) || !/\.(mp4|m4v|mov)$/i.test(f.file) ? video.probe(f.file) : null;
      durations.set(key, probed ? probed.duration : mp4Duration(f.file));
    }
    f.duration = durations.get(key);
  }
  byId = new Map(library.map((f) => [f.id, f]));
  scannedAt = Date.now();
  return library;
}

// ------------------------------------------------------------ progress
const PROGRESS_FILE = path.join(DATA_DIR, 'progress.json');
let progress = {};
try { progress = JSON.parse(fs.readFileSync(PROGRESS_FILE, 'utf8')); } catch { /* first run */ }
let saveTimer = null;
function saveProgress() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    try {
      fs.mkdirSync(DATA_DIR, { recursive: true });
      fs.writeFileSync(PROGRESS_FILE, JSON.stringify(progress, null, 2));
    } catch (err) { log('progress not saved:', err.message); }
  }, 1000);
}

// ------------------------------------------------------------ http
function send(res, status, body, headers = {}) {
  const isJson = typeof body !== 'string' && !Buffer.isBuffer(body);
  res.writeHead(status, { 'Content-Type': isJson ? 'application/json; charset=utf-8' : 'text/plain; charset=utf-8', ...headers });
  res.end(isJson ? JSON.stringify(body) : body);
}

function publicFilm(f, base) {
  const key = API_KEY ? `?key=${encodeURIComponent(API_KEY)}` : '';
  return {
    id: f.id,
    title: f.title,
    relativePath: f.relativePath.replace(/\\/g, '/'),
    info: f.info,
    fsk: f.fsk,
    cast: f.cast,
    crew: f.crew,
    genre: f.genre,
    duration: f.duration,
    size: f.size,
    icon: f.iconFile ? `${base}/img/${f.id}/icon${key}` : null,
    banner: f.bannerFile ? `${base}/img/${f.id}/banner${key}` : null,
    stream: `${base}/${video.needsRemux(f.file) && video.available ? 'play' : 'stream'}/${f.id}${key}`,
    play: playInfo(f, base),
    position: progress[f.id] || 0
  };
}

// MKV & co: how the launcher's player should play it (tracks, subtitle URL).
function playInfo(f, base) {
  if (!video.needsRemux(f.file) || !video.available) return null;
  const info = video.probe(f.file) || {};
  const pick = video.pickTracks(info);
  return {
    remux: true,
    duration: info.duration || null,
    audio: (info.audio || []).map((t) => ({ lang: t.lang, title: t.title, codec: t.codec, channels: t.channels })),
    subs: (info.subs || []).map((t) => ({ n: t.n, lang: t.lang, title: t.title, forced: t.forced })),
    a: pick.a,
    s: pick.s,
    sub: `${base}/sub/${f.id}/{n}${API_KEY ? `?key=${encodeURIComponent(API_KEY)}` : ''}`, // {n} = subtitle number
    hevc: Boolean(info.video && info.video.codec === 'hevc')
  };
}

// Video/image files with Range support: the player asks for exactly the bytes
// it needs, so seeking into the middle of a 1.3 GB film is instant.
function sendFile(req, res, file, type) {
  let stat;
  try { stat = fs.statSync(file); } catch { return send(res, 404, 'not found'); }
  const total = stat.size;
  const common = { 'Content-Type': type, 'Accept-Ranges': 'bytes', 'Cache-Control': type.startsWith('image/') ? 'max-age=86400' : 'no-store' };
  const range = /^bytes=(\d*)-(\d*)$/.exec(req.headers.range || '');
  if (range && (range[1] || range[2])) {
    let start = range[1] ? Number(range[1]) : total - Number(range[2]);
    let end = range[1] && range[2] ? Number(range[2]) : total - 1;
    start = Math.max(0, start);
    end = Math.min(total - 1, end);
    if (start > end) return send(res, 416, '', { 'Content-Range': `bytes */${total}` });
    res.writeHead(206, { ...common, 'Content-Range': `bytes ${start}-${end}/${total}`, 'Content-Length': end - start + 1 });
    if (req.method === 'HEAD') return res.end();
    // pipeline, not pipe: closes the file when the player aborts the request (it does that on every seek).
    return pipeline(fs.createReadStream(file, { start, end }), res, () => {});
  }
  res.writeHead(200, { ...common, 'Content-Length': total });
  if (req.method === 'HEAD') return res.end();
  pipeline(fs.createReadStream(file), res, () => {});
}

const server = http.createServer((req, res) => {
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Headers', 'Range, Content-Type, X-Api-Key');
  res.setHeader('Access-Control-Expose-Headers', 'Content-Range, Content-Length, Accept-Ranges');
  if (req.method === 'OPTIONS') { res.writeHead(204, { 'Access-Control-Allow-Methods': 'GET, HEAD, POST, OPTIONS' }); return res.end(); }

  const url = new URL(req.url, 'http://x');
  if (API_KEY && url.searchParams.get('key') !== API_KEY && req.headers['x-api-key'] !== API_KEY) return send(res, 401, { error: 'Falscher API-Schlüssel' });

  const base = `http://${req.headers.host}`;
  const parts = url.pathname.split('/').filter(Boolean);
  try {
    if (req.method === 'GET' && url.pathname === '/api/ping') {
      return send(res, 200, { name: SERVER_NAME, version: VERSION, films: getLibrary().length });
    }
    if (req.method === 'GET' && url.pathname === '/api/library') {
      return send(res, 200, getLibrary().map((f) => publicFilm(f, base)));
    }
    if ((req.method === 'GET' || req.method === 'HEAD') && parts[0] === 'stream' && parts[1]) {
      getLibrary();
      const f = byId.get(parts[1]);
      if (!f) return send(res, 404, 'unbekannter Film');
      return sendFile(req, res, f.file, f.type);
    }
    if ((req.method === 'GET' || req.method === 'HEAD') && parts[0] === 'play' && parts[1]) {
      getLibrary();
      const f = byId.get(parts[1]);
      if (!f) return send(res, 404, 'unbekannter Film');
      const num = (k) => Math.max(0, Number(url.searchParams.get(k)) || 0);
      return video.remux(req, res, f.file, { t: num('t'), a: num('a'), v: url.searchParams.get('v') || '' });
    }
    if (req.method === 'GET' && parts[0] === 'seek' && parts[1]) {
      getLibrary();
      const f = byId.get(parts[1]);
      if (!f) return send(res, 404, 'unbekannter Film');
      return send(res, 200, { t: video.keyframeBefore(f.file, Math.max(0, Number(url.searchParams.get('t')) || 0)) });
    }
    if (req.method === 'GET' && parts[0] === 'sub' && parts[1] && parts[2]) {
      getLibrary();
      const f = byId.get(parts[1]);
      if (!f) return send(res, 404, 'unbekannter Film');
      return video.subtitle(req, res, f.file, { n: Number(parts[2]) || 0, t: Math.max(0, Number(url.searchParams.get('t')) || 0) });
    }
    if (req.method === 'GET' && parts[0] === 'img' && parts[1] && parts[2]) {
      getLibrary();
      const f = byId.get(parts[1]);
      const file = f && (parts[2] === 'banner' ? f.bannerFile : f.iconFile);
      if (!file) return send(res, 404, 'kein Bild');
      return sendFile(req, res, file, IMAGE_TYPES[path.extname(file).toLowerCase()] || 'application/octet-stream');
    }
    if (req.method === 'POST' && parts[0] === 'api' && parts[1] === 'progress' && parts[2]) {
      let body = '';
      req.on('data', (c) => { body += c; if (body.length > 1000) req.destroy(); });
      req.on('end', () => {
        const seconds = Number((() => { try { return JSON.parse(body).seconds; } catch { return NaN; } })());
        if (!Number.isFinite(seconds) || seconds < 0) return send(res, 400, { error: 'seconds fehlt' });
        progress[parts[2]] = Math.round(seconds);
        saveProgress();
        send(res, 200, { ok: true });
      });
      return;
    }
    send(res, 404, { error: 'unbekannt' });
  } catch (err) {
    log('error', err.message);
    send(res, 500, { error: 'Serverfehler' });
  }
});

server.listen(PORT, '0.0.0.0', () => {
  const ips = Object.values(os.networkInterfaces()).flat().filter((n) => n && n.family === 'IPv4' && !n.internal).map((n) => n.address);
  log(`TV Launcher Media-Server ${VERSION} läuft`);
  log(`Filme aus: ${MEDIA_DIR} (${getLibrary().length} gefunden)`);
  ips.forEach((ip) => log(`Im Launcher eintragen: http://${ip}:${PORT}`));
});
