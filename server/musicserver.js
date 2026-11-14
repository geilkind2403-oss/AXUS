// TV Launcher Musik-Server -- streams your music library to the launcher's
// music player over the LAN. Like mediaserver.js: one file, plain Node.js, no
// npm packages; runs in Docker or with a bare `node musicserver.js`.
//
// Library layout (what a ripper / Windows Media Player writes):
//   <MUSIC_DIR>/<Interpret>/artist.jpg|png                 (optional)
//              /<Interpret>/<Interpret> - <Album>/Folder.jpg
//                                                /AlbumArtSmall.jpg
//                                                /01 - <Interpret> - <Titel>.mp3
//                                                /01 - <Interpret> - <Titel>.lrc   (optional, Songtext)
// Also fine: MUSIC_DIR being one artist folder itself, albums directly in
// MUSIC_DIR, "CD1"/"Disc 2" subfolders, flac/m4a/ogg/opus/wav. ID3 tags win
// over file names where they have better data (e.g. "Channel Surfing / The End",
// which can't be written in a Windows file name); covers embedded in the MP3
// are used when the folder has no Folder.jpg.
//
// API (JSON, CORS open -- it's a LAN service for the launcher):
//   GET /api/ping              -> { name, version, type: 'music', artists, albums, tracks }
//   GET /api/library           -> { artists: [...], albums: [...], tracks: [...] }
//   GET /stream/<trackId>      -> the audio file, with HTTP Range (instant seeking)
//   GET /img/album/<albumId>   -> Folder.jpg (?size=small -> AlbumArtSmall.jpg)
//   GET /img/artist/<artistId> -> artist.jpg / artist.png
//   GET /lyrics/<trackId>      -> { type: 'lrc' | 'text', text }
//
// Settings via environment:
//   MUSIC_DIR   library root                 (default: ./music, in Docker: /music)
//   DATA_DIR    where the tag cache lives    (default: ./data,  in Docker: /data)
//   PORT        HTTP port                    (default: 8421 -- the film server is 8420)
//   SERVER_NAME shown to the launcher        (default: the host name)
//   API_KEY     optional shared key: requests need ?key=... or header X-Api-Key
'use strict';

const http = require('http');
const fs = require('fs');
const path = require('path');
const os = require('os');
const crypto = require('crypto');
const { pipeline } = require('stream');

const VERSION = '1.0.0';
const DEFAULT_PORT = 8421;

const AUDIO_TYPES = {
  '.mp3': 'audio/mpeg', '.m4a': 'audio/mp4', '.aac': 'audio/aac', '.flac': 'audio/flac',
  '.ogg': 'audio/ogg', '.oga': 'audio/ogg', '.opus': 'audio/ogg', '.wav': 'audio/wav', '.webm': 'audio/webm'
};
const IMAGE_TYPES = { '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.png': 'image/png', '.webp': 'image/webp' };
const COVER_NAMES = ['folder', 'cover', 'front', 'album'];
const COVER_SMALL_NAMES = ['albumartsmall', 'folder_small', 'cover_small'];
const ARTIST_NAMES = ['artist', 'interpret', 'folder'];
const DISC_DIR = /^(?:cd|disc|disk|seite|side)[\s_-]*(\d+)$/i;

const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);
const idFor = (s) => crypto.createHash('sha1').update(String(s).replace(/\\/g, '/').toLowerCase()).digest('hex').slice(0, 12);
const clean = (s) => String(s || '').replace(/\u0000/g, '').replace(/\s+/g, ' ').trim();

// ------------------------------------------------------------ ID3v2 tags
function readAt(fd, pos, len) {
  const buf = Buffer.alloc(len);
  const n = fs.readSync(fd, buf, 0, len, pos);
  return n === len ? buf : buf.subarray(0, n);
}
const syncsafe = (b, o) => ((b[o] & 0x7f) << 21) | ((b[o + 1] & 0x7f) << 14) | ((b[o + 2] & 0x7f) << 7) | (b[o + 3] & 0x7f);

function decodeText(buf, enc) {
  if (!buf.length) return '';
  if (enc === 0) return buf.toString('latin1');
  if (enc === 3) return buf.toString('utf8');
  if (enc === 1 || enc === 2) {
    let b = buf;
    let le = enc === 1; // UTF-16 with BOM (default LE), UTF-16BE without
    if (b.length >= 2 && b[0] === 0xff && b[1] === 0xfe) { le = true; b = b.subarray(2); }
    else if (b.length >= 2 && b[0] === 0xfe && b[1] === 0xff) { le = false; b = b.subarray(2); }
    if (!le) { b = Buffer.from(b); for (let i = 0; i + 1 < b.length; i += 2) { const t = b[i]; b[i] = b[i + 1]; b[i + 1] = t; } }
    return b.toString('utf16le');
  }
  return buf.toString('latin1');
}
// Index of the text terminator for an encoding (1 zero byte, or 2 aligned for UTF-16).
function termIndex(buf, from, enc) {
  if (enc === 1 || enc === 2) {
    for (let i = from; i + 1 < buf.length; i += 2) if (buf[i] === 0 && buf[i + 1] === 0) return i;
    return buf.length;
  }
  const i = buf.indexOf(0, from);
  return i < 0 ? buf.length : i;
}
const textFrame = (data) => clean(decodeText(data.subarray(1), data[0]).split('\u0000')[0]);

// Reads only frame headers and the small text frames; pictures and lyrics are
// remembered by position and read when someone actually asks for them.
function readId3(fd, size) {
  const out = { tagEnd: 0 };
  if (size < 10) return out;
  const head = readAt(fd, 0, 10);
  if (head.toString('latin1', 0, 3) !== 'ID3') return out;
  const ver = head[3];
  const flags = head[5];
  const tagSize = syncsafe(head, 6);
  out.tagEnd = 10 + tagSize + (flags & 0x10 ? 10 : 0);
  if (ver < 2 || ver > 4) return out;
  let pos = 10;
  if (ver >= 3 && (flags & 0x40)) { // extended header
    const ext = readAt(fd, pos, 4);
    pos += ver === 4 ? syncsafe(ext, 0) : ext.readUInt32BE(0) + 4;
  }
  const end = Math.min(10 + tagSize, size);
  const idLen = ver === 2 ? 3 : 4;
  const hdrLen = ver === 2 ? 6 : 10;
  const WANT = { TIT2: 'title', TT2: 'title', TPE1: 'artist', TP1: 'artist', TPE2: 'albumArtist', TP2: 'albumArtist',
    TALB: 'album', TAL: 'album', TRCK: 'track', TRK: 'track', TPOS: 'disc', TPA: 'disc',
    TYER: 'year', TYE: 'year', TDRC: 'year', TCON: 'genre', TCO: 'genre' };
  while (pos + hdrLen <= end) {
    const h = readAt(fd, pos, hdrLen);
    if (h.length < hdrLen || h[0] === 0) break; // padding
    const id = h.toString('latin1', 0, idLen);
    const len = ver === 2 ? (h[3] << 16) | (h[4] << 8) | h[5] : ver === 4 ? syncsafe(h, 4) : h.readUInt32BE(4);
    if (!/^[A-Z0-9]+$/.test(id) || len <= 0 || pos + hdrLen + len > end) break;
    const dataPos = pos + hdrLen;
    if (WANT[id] && len < 4096 && !out[WANT[id]]) {
      out[WANT[id]] = textFrame(readAt(fd, dataPos, len));
    } else if ((id === 'APIC' || id === 'PIC') && !out.picture) {
      const d = readAt(fd, dataPos, Math.min(len, 512));
      const enc = d[0];
      let mime; let i;
      if (id === 'PIC') { mime = /png/i.test(d.toString('latin1', 1, 4)) ? 'image/png' : 'image/jpeg'; i = 5; }
      else { const m = d.indexOf(0, 1); mime = d.toString('latin1', 1, m) || 'image/jpeg'; i = m + 2; }
      i = termIndex(d, i, enc) + (enc === 1 || enc === 2 ? 2 : 1);
      if (i < d.length) out.picture = { pos: dataPos + i, len: len - i, mime: mime.includes('/') ? mime : `image/${mime.toLowerCase()}` };
    } else if (id === 'USLT' && !out.lyrics) {
      out.lyrics = { pos: dataPos, len };
    }
    pos = dataPos + len;
  }
  return out;
}

function readUslt(file, at) {
  let fd;
  try {
    fd = fs.openSync(file, 'r');
    const d = readAt(fd, at.pos, Math.min(at.len, 256 * 1024));
    const enc = d[0];
    const start = termIndex(d, 4, enc) + (enc === 1 || enc === 2 ? 2 : 1);
    return decodeText(d.subarray(start), enc).replace(/\u0000/g, '').trim();
  } catch { return ''; } finally { if (fd !== undefined) fs.closeSync(fd); }
}

// ------------------------------------------------------------ durations
const BITRATES = {
  '1-1': [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448],
  '1-2': [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384],
  '1-3': [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320],
  '2-1': [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256],
  '2-2': [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
  '2-3': [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160]
};
const RATES = { 1: [44100, 48000, 32000], 2: [22050, 24000, 16000], 25: [11025, 12000, 8000] };

// MP3: Xing/Info or VBRI header when present (VBR), else size / bitrate (CBR).
function mp3Duration(fd, size, audioStart) {
  const buf = readAt(fd, audioStart, Math.min(64 * 1024, Math.max(0, size - audioStart)));
  for (let i = 0; i + 4 < buf.length; i++) {
    if (buf[i] !== 0xff || (buf[i + 1] & 0xe0) !== 0xe0) continue;
    const vBits = (buf[i + 1] >> 3) & 3; // 3 = MPEG1, 2 = MPEG2, 0 = MPEG2.5
    const lBits = (buf[i + 1] >> 1) & 3; // 1 = L3, 2 = L2, 3 = L1
    const brIdx = buf[i + 2] >> 4;
    const srIdx = (buf[i + 2] >> 2) & 3;
    if (vBits === 1 || lBits === 0 || brIdx === 0 || brIdx === 15 || srIdx === 3) continue;
    const version = vBits === 3 ? 1 : vBits === 2 ? 2 : 25;
    const layer = 4 - lBits;
    const sampleRate = RATES[version][srIdx];
    const bitrate = BITRATES[`${version === 1 ? 1 : 2}-${layer}`][brIdx] * 1000;
    const mono = (buf[i + 3] >> 6) === 3;
    const spf = layer === 1 ? 384 : layer === 2 ? 1152 : version === 1 ? 1152 : 576;
    const xingAt = i + 4 + (version === 1 ? (mono ? 17 : 32) : (mono ? 9 : 17));
    const tagName = buf.toString('latin1', xingAt, xingAt + 4);
    if ((tagName === 'Xing' || tagName === 'Info') && xingAt + 12 <= buf.length && (buf.readUInt32BE(xingAt + 4) & 1)) {
      return Math.round((buf.readUInt32BE(xingAt + 8) * spf) / sampleRate);
    }
    if (buf.toString('latin1', i + 36, i + 40) === 'VBRI' && i + 54 <= buf.length) {
      return Math.round((buf.readUInt32BE(i + 36 + 14) * spf) / sampleRate);
    }
    let bytes = size - (audioStart + i);
    if (size > 128 && readAt(fd, size - 128, 3).toString('latin1') === 'TAG') bytes -= 128;
    return bitrate ? Math.round((bytes * 8) / bitrate) : null;
  }
  return null;
}

function flacDuration(fd) {
  const b = readAt(fd, 0, 42);
  if (b.toString('latin1', 0, 4) !== 'fLaC') return null;
  const s = 8; // STREAMINFO body starts after 'fLaC' + 4-byte block header
  const rate = (b[s + 10] << 12) | (b[s + 11] << 4) | (b[s + 12] >> 4);
  const total = (b[s + 13] & 0x0f) * 2 ** 32 + b.readUInt32BE(s + 14);
  return rate ? Math.round(total / rate) : null;
}

// M4A/MP4: the "mvhd" box inside "moov" (same trick as the film server).
function mp4Duration(fd, size) {
  let pos = 0;
  while (pos + 8 <= size) {
    const h = readAt(fd, pos, 16);
    let box = h.readUInt32BE(0);
    const type = h.toString('latin1', 4, 8);
    let hl = 8;
    if (box === 1) { box = Number(h.readBigUInt64BE(8)); hl = 16; } else if (box === 0) box = size - pos;
    if (box < hl) return null;
    if (type === 'moov') {
      const moov = readAt(fd, pos + hl, Math.min(box - hl, 4 * 1024 * 1024));
      const at = moov.indexOf('mvhd', 0, 'latin1');
      if (at < 4) return null;
      const v = at + 4;
      const ts = moov[v] === 1 ? moov.readUInt32BE(v + 20) : moov.readUInt32BE(v + 12);
      const dur = moov[v] === 1 ? Number(moov.readBigUInt64BE(v + 24)) : moov.readUInt32BE(v + 16);
      return ts ? Math.round(dur / ts) : null;
    }
    pos += box;
  }
  return null;
}

function probe(file, size) {
  let fd;
  try {
    fd = fs.openSync(file, 'r');
    const tags = readId3(fd, size);
    const ext = path.extname(file).toLowerCase();
    let duration = null;
    if (ext === '.mp3') duration = mp3Duration(fd, size, tags.tagEnd);
    else if (ext === '.flac') duration = flacDuration(fd);
    else if (ext === '.m4a') duration = mp4Duration(fd, size);
    return { ...tags, duration };
  } catch { return {}; } finally { if (fd !== undefined) fs.closeSync(fd); }
}

// ------------------------------------------------------------ names
// "01 - Madison Beer - Good In Goodbye" -> { number: 1, artist, title }
// Also "01. Title", "1-03 Title" (disc-track), "Title".
function parseTrackName(base) {
  let m = /^(\d{1,2})-(\d{1,3})[\s._-]+(.+)$/.exec(base);
  let disc = null;
  if (m) { disc = Number(m[1]); base = `${m[2]} - ${m[3]}`; }
  m = /^(\d{1,3})[\s.)_]*(?:-\s*)?(.+)$/.exec(base);
  if (!m) return { number: null, artist: null, title: clean(base), disc };
  const parts = m[2].split(/\s+-\s+/);
  if (parts.length >= 2) return { number: Number(m[1]), artist: clean(parts[0]), title: clean(parts.slice(1).join(' - ')), disc };
  return { number: Number(m[1]), artist: null, title: clean(m[2]), disc };
}
// "Madison Beer - Life Support" -> { artist, album }
function parseAlbumName(name) {
  const parts = name.split(/\s+-\s+/);
  if (parts.length >= 2) return { artist: clean(parts[0]), album: clean(parts.slice(1).join(' - ')) };
  return { artist: null, album: clean(name) };
}
const firstNum = (s) => { const n = parseInt(String(s || ''), 10); return Number.isFinite(n) ? n : null; };
const cleanGenre = (g) => clean(String(g || '').replace(/^\(\d+\)/, '')) || null;

function findImage(dir, names, files) {
  const list = files || (() => { try { return fs.readdirSync(dir); } catch { return []; } })();
  for (const want of names) {
    const hit = list.find((n) => IMAGE_TYPES[path.extname(n).toLowerCase()] && path.basename(n, path.extname(n)).toLowerCase() === want);
    if (hit) return path.join(dir, hit);
  }
  return null;
}

// ------------------------------------------------------------ library scan
// Tag/duration cache (per file + size + mtime), kept on disk so a restart
// doesn't have to open every file again.
function createScanner({ root, cacheFile }) {
  let cache = {};
  try { cache = cacheFile ? JSON.parse(fs.readFileSync(cacheFile, 'utf8')) : {}; } catch { cache = {}; }
  let dirty = false;

  function tagsFor(file, stat) {
    const key = `${stat.size}|${Math.round(stat.mtimeMs)}`;
    const c = cache[file];
    if (c && c.key === key) return c.t;
    const t = probe(file, stat.size);
    cache[file] = { key, t };
    dirty = true;
    return t;
  }

  function scan() {
    const albums = new Map(); // albumDir -> album
    const seen = new Set();

    function walk(dir, depth) {
      if (depth > 6) return;
      let entries;
      try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
      const names = entries.map((e) => e.name);
      const audio = entries.filter((e) => e.isFile() && AUDIO_TYPES[path.extname(e.name).toLowerCase()]);
      if (audio.length) {
        const discMatch = DISC_DIR.exec(path.basename(dir));
        const albumDir = discMatch && dir !== root ? path.dirname(dir) : dir;
        const discNo = discMatch ? Number(discMatch[1]) : null;
        let album = albums.get(albumDir);
        if (!album) {
          const albumFiles = albumDir === dir ? names : (() => { try { return fs.readdirSync(albumDir); } catch { return []; } })();
          const parsed = parseAlbumName(path.basename(albumDir));
          const parentDir = albumDir === root ? root : path.dirname(albumDir);
          album = {
            dir: albumDir,
            rel: path.relative(root, albumDir).replace(/\\/g, '/') || '.',
            folderArtist: parsed.artist,
            folderAlbum: parsed.album,
            parentName: parentDir !== root || albumDir !== root ? path.basename(parentDir) : null,
            parentDir,
            coverFile: findImage(albumDir, COVER_NAMES, albumFiles) || (discMatch ? findImage(dir, COVER_NAMES, names) : null),
            coverSmallFile: findImage(albumDir, COVER_SMALL_NAMES, albumFiles),
            tracks: []
          };
          albums.set(albumDir, album);
        }
        for (const e of audio) {
          const file = path.join(dir, e.name);
          if (seen.has(file)) continue;
          seen.add(file);
          let stat;
          try { stat = fs.statSync(file); } catch { continue; }
          const t = tagsFor(file, stat);
          const base = path.basename(e.name, path.extname(e.name));
          const byName = parseTrackName(base);
          const lrc = names.find((n) => n.toLowerCase() === `${base.toLowerCase()}.lrc`) ||
            names.find((n) => n.toLowerCase() === `${base.toLowerCase()}.txt`);
          album.tracks.push({
            file,
            rel: path.relative(root, file).replace(/\\/g, '/'),
            type: AUDIO_TYPES[path.extname(e.name).toLowerCase()],
            size: stat.size,
            title: t.title || byName.title,
            artist: t.artist || byName.artist,
            number: byName.number || firstNum(t.track),
            disc: discNo || byName.disc || firstNum(t.disc) || 1,
            duration: t.duration || null,
            tagAlbum: t.album, tagAlbumArtist: t.albumArtist, year: firstNum(t.year), genre: cleanGenre(t.genre),
            picture: t.picture || null,
            lyricsFile: lrc ? path.join(dir, lrc) : null,
            lyricsTag: t.lyrics || null
          });
        }
      }
      for (const e of entries) {
        if (e.isDirectory() && !e.name.startsWith('.') && !e.name.startsWith('$')) walk(path.join(dir, e.name), depth + 1);
      }
    }
    walk(root, 0);

    if (dirty && cacheFile) {
      // Forget files that are gone, then save.
      const all = new Set([...albums.values()].flatMap((a) => a.tracks.map((t) => t.file)));
      Object.keys(cache).forEach((f) => { if (!all.has(f)) delete cache[f]; });
      try { fs.mkdirSync(path.dirname(cacheFile), { recursive: true }); fs.writeFileSync(cacheFile, JSON.stringify(cache)); } catch { /* cache is optional */ }
      dirty = false;
    }

    // ---- shape it: artists -> albums -> tracks
    const artists = new Map();
    const outAlbums = [];
    const outTracks = [];
    const mostCommon = (list) => {
      const counts = new Map();
      list.filter(Boolean).forEach((v) => counts.set(v, (counts.get(v) || 0) + 1));
      return [...counts.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] || null;
    };
    for (const a of albums.values()) {
      if (!a.tracks.length) continue;
      const artistName = a.folderArtist || mostCommon(a.tracks.map((t) => t.tagAlbumArtist)) ||
        mostCommon(a.tracks.map((t) => t.artist)) || a.parentName || 'Unbekannter Interpret';
      const title = (a.folderArtist ? a.folderAlbum : null) || mostCommon(a.tracks.map((t) => t.tagAlbum)) || a.folderAlbum;
      const artistId = idFor(`artist:${artistName}`);
      if (!artists.has(artistId)) {
        // artist.jpg next to the album folders (the artist's folder), or in the library root.
        const img = findImage(a.parentDir, ARTIST_NAMES.filter((n) => a.parentDir !== a.dir || n !== 'folder')) ||
          (a.parentDir !== root ? null : findImage(root, ['artist', 'interpret']));
        artists.set(artistId, { id: artistId, name: artistName, imageFile: img, albumIds: [] });
      } else if (!artists.get(artistId).imageFile) {
        artists.get(artistId).imageFile = findImage(a.parentDir, ['artist', 'interpret']);
      }
      const id = idFor(`album:${a.rel}`);
      a.tracks.sort((x, y) => (x.disc - y.disc) || ((x.number ?? 999) - (y.number ?? 999)) || x.title.localeCompare(y.title, 'de'));
      const pictureTrack = a.tracks.find((t) => t.picture);
      const album = {
        id, title, artist: artistName, artistId,
        year: mostCommon(a.tracks.map((t) => t.year)),
        genre: mostCommon(a.tracks.map((t) => t.genre)),
        coverFile: a.coverFile, coverSmallFile: a.coverSmallFile,
        picture: !a.coverFile && pictureTrack ? { file: pictureTrack.file, ...pictureTrack.picture } : null,
        discs: new Set(a.tracks.map((t) => t.disc)).size,
        trackIds: []
      };
      for (const t of a.tracks) {
        const tid = idFor(`track:${t.rel}`);
        album.trackIds.push(tid);
        outTracks.push({
          id: tid, title: t.title, artist: t.artist || artistName, album: title, albumId: id, artistId,
          number: t.number, disc: t.disc, duration: t.duration, file: t.file, rel: t.rel, type: t.type, size: t.size,
          genre: t.genre || album.genre, year: t.year || album.year,
          lyricsFile: t.lyricsFile, lyricsTag: t.lyricsTag ? { file: t.file, ...t.lyricsTag } : null
        });
      }
      album.duration = a.tracks.reduce((s, t) => s + (t.duration || 0), 0) || null;
      artists.get(artistId).albumIds.push(id);
      outAlbums.push(album);
    }
    // Newest album first per artist.
    for (const ar of artists.values()) {
      ar.albumIds.sort((x, y) => {
        const ax = outAlbums.find((a) => a.id === x); const ay = outAlbums.find((a) => a.id === y);
        return (ay.year || 0) - (ax.year || 0) || ax.title.localeCompare(ay.title, 'de');
      });
    }
    return { artists: [...artists.values()], albums: outAlbums, tracks: outTracks };
  }

  return { scan };
}

// .lrc/.txt as Windows editors write them: UTF-8 (BOM or not), UTF-16 with BOM ("Unicode"), or ANSI.
function decodeTextFile(buf) {
  if (buf.length >= 2 && ((buf[0] === 0xff && buf[1] === 0xfe) || (buf[0] === 0xfe && buf[1] === 0xff))) return decodeText(buf, 1);
  const text = buf.toString('utf8');
  return (text.includes('�') ? new TextDecoder('windows-1252').decode(buf) : text).replace(/^﻿/, '');
}

// Lyrics for a scanned track: the .lrc/.txt next to it, else the ID3 USLT frame.
function readLyrics(track) {
  if (track.lyricsFile) {
    try {
      const text = decodeTextFile(fs.readFileSync(track.lyricsFile));
      return { type: /^\s*\[\d{1,2}:\d{2}/m.test(text) ? 'lrc' : 'text', text };
    } catch { /* fall through */ }
  }
  if (track.lyricsTag) {
    const text = readUslt(track.lyricsTag.file, track.lyricsTag);
    if (text) return { type: /^\s*\[\d{1,2}:\d{2}/m.test(text) ? 'lrc' : 'text', text };
  }
  return null;
}

// ------------------------------------------------------------ http
function send(res, status, body, headers = {}) {
  const isJson = typeof body !== 'string' && !Buffer.isBuffer(body);
  res.writeHead(status, { 'Content-Type': isJson ? 'application/json; charset=utf-8' : 'text/plain; charset=utf-8', ...headers });
  res.end(isJson ? JSON.stringify(body) : body);
}

// Files with Range support: the player asks for exactly the bytes it needs,
// so seeking anywhere in a song is instant and nothing is sent twice.
function sendFile(req, res, file, type, { start: from = 0, length } = {}) {
  let total;
  try { total = length != null ? length : fs.statSync(file).size - from; } catch { return send(res, 404, 'not found'); }
  const common = { 'Content-Type': type, 'Accept-Ranges': 'bytes', 'Cache-Control': 'private, max-age=86400' };
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
    return pipeline(fs.createReadStream(file, { start: from + start, end: from + end }), res, () => {});
  }
  res.writeHead(200, { ...common, 'Content-Length': total });
  if (req.method === 'HEAD' || total <= 0) return res.end(); // an empty file (broken copy) must not throw
  pipeline(fs.createReadStream(file, { start: from, end: from + total - 1 }), res, () => {});
}

function startServer() {
  const MUSIC_DIR = path.resolve(process.env.MUSIC_DIR || path.join(__dirname, 'music'));
  const DATA_DIR = path.resolve(process.env.DATA_DIR || path.join(__dirname, 'data'));
  const PORT = Number(process.env.PORT) || DEFAULT_PORT;
  const SERVER_NAME = process.env.SERVER_NAME || os.hostname();
  const API_KEY = process.env.API_KEY || '';
  const scanner = createScanner({ root: MUSIC_DIR, cacheFile: path.join(DATA_DIR, 'music-cache.json') });

  // Rescanned at most every 30 s (on demand): new albums show up without a restart.
  let lib = { artists: [], albums: [], tracks: [] };
  let index = { artists: new Map(), albums: new Map(), tracks: new Map() };
  let scannedAt = 0;
  function library() {
    if (Date.now() - scannedAt < 30000) return lib;
    lib = scanner.scan();
    index = {
      artists: new Map(lib.artists.map((a) => [a.id, a])),
      albums: new Map(lib.albums.map((a) => [a.id, a])),
      tracks: new Map(lib.tracks.map((t) => [t.id, t]))
    };
    scannedAt = Date.now();
    return lib;
  }

  function publicLibrary(base) {
    const key = API_KEY ? `?key=${encodeURIComponent(API_KEY)}` : '';
    const amp = key ? '&' : '?';
    const l = library();
    return {
      server: SERVER_NAME,
      artists: l.artists.map((a) => ({ id: a.id, name: a.name, image: a.imageFile ? `${base}/img/artist/${a.id}${key}` : null, albumIds: a.albumIds })),
      albums: l.albums.map((a) => {
        const hasCover = a.coverFile || a.picture;
        return {
          id: a.id, title: a.title, artist: a.artist, artistId: a.artistId, year: a.year, genre: a.genre,
          duration: a.duration, discs: a.discs, trackIds: a.trackIds,
          cover: hasCover ? `${base}/img/album/${a.id}${key}` : null,
          coverSmall: a.coverSmallFile ? `${base}/img/album/${a.id}${key}${amp}size=small` : (hasCover ? `${base}/img/album/${a.id}${key}` : null)
        };
      }),
      tracks: l.tracks.map((t) => ({
        id: t.id, title: t.title, artist: t.artist, album: t.album, albumId: t.albumId, artistId: t.artistId,
        number: t.number, disc: t.disc, duration: t.duration, genre: t.genre, year: t.year, size: t.size,
        stream: `${base}/stream/${t.id}${key}`,
        lyrics: t.lyricsFile || t.lyricsTag ? `${base}/lyrics/${t.id}${key}` : null
      }))
    };
  }

  const server = http.createServer((req, res) => {
    res.setHeader('Access-Control-Allow-Origin', '*');
    res.setHeader('Access-Control-Allow-Headers', 'Range, Content-Type, X-Api-Key');
    res.setHeader('Access-Control-Expose-Headers', 'Content-Range, Content-Length, Accept-Ranges');
    if (req.method === 'OPTIONS') { res.writeHead(204, { 'Access-Control-Allow-Methods': 'GET, HEAD, OPTIONS' }); return res.end(); }

    const url = new URL(req.url, 'http://x');
    if (API_KEY && url.searchParams.get('key') !== API_KEY && req.headers['x-api-key'] !== API_KEY) return send(res, 401, { error: 'Falscher API-Schlüssel' });
    const base = `http://${req.headers.host}`;
    const parts = url.pathname.split('/').filter(Boolean);
    try {
      if (req.method === 'GET' && url.pathname === '/api/ping') {
        const l = library();
        return send(res, 200, { name: SERVER_NAME, version: VERSION, type: 'music', artists: l.artists.length, albums: l.albums.length, tracks: l.tracks.length });
      }
      if (req.method === 'GET' && url.pathname === '/api/library') return send(res, 200, publicLibrary(base));
      if ((req.method === 'GET' || req.method === 'HEAD') && parts[0] === 'stream' && parts[1]) {
        library();
        const t = index.tracks.get(parts[1]);
        if (!t) return send(res, 404, 'unbekannter Titel');
        return sendFile(req, res, t.file, t.type);
      }
      if ((req.method === 'GET' || req.method === 'HEAD') && parts[0] === 'img' && parts[2]) {
        library();
        if (parts[1] === 'artist') {
          const a = index.artists.get(parts[2]);
          if (!a || !a.imageFile) return send(res, 404, 'kein Bild');
          return sendFile(req, res, a.imageFile, IMAGE_TYPES[path.extname(a.imageFile).toLowerCase()]);
        }
        const a = index.albums.get(parts[2]);
        if (!a) return send(res, 404, 'kein Bild');
        const file = url.searchParams.get('size') === 'small' && a.coverSmallFile ? a.coverSmallFile : a.coverFile;
        if (file) return sendFile(req, res, file, IMAGE_TYPES[path.extname(file).toLowerCase()]);
        if (a.picture) return sendFile(req, res, a.picture.file, a.picture.mime, { start: a.picture.pos, length: a.picture.len });
        return send(res, 404, 'kein Bild');
      }
      if (req.method === 'GET' && parts[0] === 'lyrics' && parts[1]) {
        library();
        const t = index.tracks.get(parts[1]);
        const ly = t && readLyrics(t);
        return ly ? send(res, 200, ly) : send(res, 404, { error: 'kein Songtext' });
      }
      send(res, 404, { error: 'unbekannt' });
    } catch (err) {
      log('error', err.message);
      if (res.headersSent) res.destroy(); else send(res, 500, { error: 'Serverfehler' });
    }
  });

  server.on('error', (err) => {
    log(err.code === 'EADDRINUSE' ? `Port ${PORT} ist schon belegt -- läuft der Musik-Server schon?` : `Fehler: ${err.message}`);
    process.exit(1);
  });
  server.listen(PORT, '0.0.0.0', () => {
    const ips = Object.values(os.networkInterfaces()).flat().filter((n) => n && n.family === 'IPv4' && !n.internal).map((n) => n.address);
    const l = library();
    log(`TV Launcher Musik-Server ${VERSION} läuft`);
    log(`Musik aus: ${MUSIC_DIR} (${l.artists.length} Interpreten, ${l.albums.length} Alben, ${l.tracks.length} Titel)`);
    ips.forEach((ip) => log(`Im Launcher eintragen: http://${ip}:${PORT}`));
  });
}

// The launcher reuses the scanner for music on the laptop itself (data/music).
module.exports = { createScanner, readLyrics, parseTrackName, parseAlbumName, AUDIO_TYPES, IMAGE_TYPES, DEFAULT_PORT };

if (require.main === module) startServer();
