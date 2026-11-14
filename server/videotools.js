// MKV & Co. for the TV launcher: Chromium (the launcher's player) plays MP4
// with H.264/AAC, but not the Matroska container, nor the AC3/DTS/TrueHD
// sound most MKV rips carry. So such files are remuxed on the fly with ffmpeg
// into a streamable (fragmented) MP4 while they play:
//   - the picture is copied as it is (no quality loss, next to no CPU) when it
//     is H.264 / HEVC / VP9 / AV1, and only converted when it has to be
//     (MPEG-4/XviD, VC-1, ... -- or when the TV can't decode HEVC: ?v=h264);
//   - the sound is copied when it is AAC/MP3, otherwise converted to AAC stereo;
//   - seeking restarts the stream at the new time (?t=seconds, on the keyframe
//     keyframeBefore() names -- the player asks for it first);
//   - the German audio track is chosen when there is one, and text subtitles
//     (SRT/ASS) are served as WebVTT -- German forced subs to German audio,
//     German subs to foreign audio.
//
// Used by the film server (mediaserver.js) and by the launcher itself for
// films in data/media (main.js). Needs ffmpeg + ffprobe: in PATH, next to this
// file (ffmpeg.exe / ffprobe.exe), or FFMPEG_PATH / FFPROBE_PATH.
'use strict';

const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const { spawn, spawnSync } = require('child_process');

// Containers Chromium can't open itself: always remuxed.
const REMUX_EXT = new Set(['.mkv', '.avi', '.ts', '.m2ts', '.mts', '.wmv', '.flv', '.mpg', '.mpeg', '.vob', '.divx']);
const COPY_VIDEO = new Set(['h264', 'hevc', 'vp9', 'av1']);
const COPY_AUDIO = new Set(['aac', 'mp3']);
const TEXT_SUBS = new Set(['subrip', 'srt', 'ass', 'ssa', 'webvtt', 'mov_text', 'text']);
const GERMAN = /^(ger|deu|de|german|deutsch)$/i;

function findTool(name, envVar) {
  const exe = process.platform === 'win32' ? `${name}.exe` : name;
  const candidates = [process.env[envVar], path.join(__dirname, exe), path.join(__dirname, 'ffmpeg', 'bin', exe), name].filter(Boolean);
  for (const c of candidates) {
    try {
      const r = spawnSync(c, ['-version'], { windowsHide: true, timeout: 8000 });
      if (r.status === 0) return c;
    } catch { /* next */ }
  }
  return null;
}

function createVideoTools({ cacheDir, log = () => {} } = {}) {
  const ffmpeg = findTool('ffmpeg', 'FFMPEG_PATH');
  const ffprobe = findTool('ffprobe', 'FFPROBE_PATH');
  if (!ffmpeg) log('Hinweis: ffmpeg nicht gefunden -- MKV-Filme spielen dann nicht (siehe ANLEITUNG.md).');

  // ---------------------------------------------------------- probe (cached)
  const probeFile = cacheDir && path.join(cacheDir, 'probe-cache.json');
  let probes = {};
  try { probes = probeFile ? JSON.parse(fs.readFileSync(probeFile, 'utf8')) : {}; } catch { probes = {}; }
  let saveTimer = null;
  const saveProbes = () => {
    if (!probeFile) return;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      try { fs.mkdirSync(cacheDir, { recursive: true }); fs.writeFileSync(probeFile, JSON.stringify(probes)); } catch { /* cache is optional */ }
    }, 1000);
  };

  const needsRemux = (file) => REMUX_EXT.has(path.extname(file).toLowerCase());

  // { duration, video: { codec, width, height }, audio: [{ lang, title, codec, channels, default }],
  //   subs: [{ n, lang, title, forced, default }] }  -- subs: text subtitles only, n = index among all subtitle streams
  function probe(file) {
    let stat;
    try { stat = fs.statSync(file); } catch { return null; }
    const key = `${stat.size}|${Math.round(stat.mtimeMs)}`;
    const c = probes[file];
    if (c && c.key === key) return c.info;
    if (!ffprobe) return null;
    let info = null;
    try {
      const r = spawnSync(ffprobe, ['-v', 'quiet', '-print_format', 'json', '-show_format', '-show_streams', file],
        { windowsHide: true, timeout: 20000, maxBuffer: 8 * 1024 * 1024 });
      const j = JSON.parse(String(r.stdout || '{}'));
      const streams = j.streams || [];
      const tag = (s, k) => (s.tags && (s.tags[k] || s.tags[k.toUpperCase()])) || '';
      const v = streams.find((s) => s.codec_type === 'video' && !(s.disposition && s.disposition.attached_pic));
      let subN = -1;
      info = {
        duration: Math.round(Number(j.format && j.format.duration) || 0) || null,
        video: v ? { codec: v.codec_name, width: v.width, height: v.height } : null,
        audio: streams.filter((s) => s.codec_type === 'audio').map((s) => ({
          lang: tag(s, 'language'), title: tag(s, 'title'), codec: s.codec_name, channels: s.channels || 0,
          default: Boolean(s.disposition && s.disposition.default)
        })),
        subs: streams.filter((s) => s.codec_type === 'subtitle').map((s) => {
          subN += 1;
          return TEXT_SUBS.has(s.codec_name) ? {
            n: subN, lang: tag(s, 'language'), title: tag(s, 'title'),
            forced: Boolean((s.disposition && s.disposition.forced) || /forced|erzwungen/i.test(tag(s, 'title'))),
            default: Boolean(s.disposition && s.disposition.default)
          } : null;
        }).filter(Boolean)
      };
    } catch { info = null; }
    if (info) { probes[file] = { key, info }; saveProbes(); }
    return info;
  }

  // Which audio track and subtitle to start with.
  function pickTracks(info) {
    if (!info) return { a: 0, s: -1 };
    const audio = info.audio || [];
    let a = audio.findIndex((t) => GERMAN.test(t.lang));
    if (a < 0) a = audio.findIndex((t) => t.default);
    if (a < 0) a = 0;
    const germanAudio = audio[a] ? GERMAN.test(audio[a].lang) : false;
    const subs = (info.subs || []).filter((s) => GERMAN.test(s.lang));
    const sub = germanAudio ? subs.find((s) => s.forced) : (subs.find((s) => !s.forced) || subs[0]);
    return { a, s: sub ? sub.n : -1 };
  }

  // A copied picture can only start on a keyframe. The stream for time t
  // therefore starts at the keyframe just before t -- asked for first, so the
  // player's clock and the subtitles line up exactly.
  function keyframeBefore(file, t) {
    if (!ffprobe || !(t > 0)) return 0;
    try {
      const r = spawnSync(ffprobe, ['-v', 'error', '-select_streams', 'v:0', '-read_intervals', `${Math.max(0, t - 30)}%${t + 0.2}`,
        '-show_entries', 'packet=pts_time,flags', '-of', 'csv=p=0', file], { windowsHide: true, timeout: 8000, maxBuffer: 4 * 1024 * 1024 });
      let best = -1;
      String(r.stdout || '').split('\n').forEach((line) => {
        const [pts, flags] = line.split(',');
        const x = Number(pts);
        if (flags && flags.includes('K') && Number.isFinite(x) && x <= t + 0.05 && x > best) best = x;
      });
      return best >= 0 ? Math.round(best * 1000) / 1000 : t;
    } catch { return t; }
  }

  // ---------------------------------------------------------- live remux
  function remux(req, res, file, { t = 0, a = 0, v = '' } = {}) {
    if (!ffmpeg) {
      res.writeHead(501, { 'Content-Type': 'text/plain; charset=utf-8' });
      res.end('ffmpeg fehlt auf diesem Rechner');
      return;
    }
    const info = probe(file) || {};
    const vcodec = info.video && info.video.codec;
    const audio = (info.audio || [])[a];
    const copyVideo = v !== 'h264' && (!vcodec || COPY_VIDEO.has(vcodec));
    const copyAudio = Boolean(audio && COPY_AUDIO.has(audio.codec));
    const args = ['-hide_banner', '-loglevel', 'error', '-nostdin'];
    // A hair past the keyframe: aimed exactly at it, the seek can land one keyframe
    // earlier (files whose clock starts a few ms before zero) -- 10 s off.
    if (t > 0) args.push('-ss', String(Math.round((t + 0.1) * 1000) / 1000));
    args.push('-i', file, '-map', '0:v:0', '-map', `0:a:${Math.max(0, a)}?`);
    if (copyVideo) {
      args.push('-c:v', 'copy');
      if (vcodec === 'hevc') args.push('-tag:v', 'hvc1');
    } else {
      args.push('-c:v', 'libx264', '-preset', 'veryfast', '-crf', '20', '-pix_fmt', 'yuv420p', '-vf', "scale='min(1920,iw)':-2");
    }
    if (copyAudio) args.push('-c:a', 'copy');
    else args.push('-c:a', 'aac', '-b:a', '192k', '-ac', '2');
    args.push('-sn', '-dn', '-map_chapters', '-1', '-avoid_negative_ts', 'make_zero',
      '-movflags', 'frag_keyframe+empty_moov+default_base_moof', '-f', 'mp4', 'pipe:1');

    const proc = spawn(ffmpeg, args, { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    let err = '';
    proc.stderr.on('data', (d) => { if (err.length < 2000) err += d; });
    proc.on('error', () => { if (!res.headersSent) { res.writeHead(500); res.end(); } });
    proc.on('close', (code) => { if (code && code !== 255 && err) log(`Remux ${path.basename(file)}: ${err.trim().split('\n').pop()}`); });
    res.writeHead(200, { 'Content-Type': 'video/mp4', 'Cache-Control': 'no-store', 'Accept-Ranges': 'none' });
    if (req.method === 'HEAD') { proc.kill('SIGKILL'); res.end(); return; }
    proc.stdout.pipe(res);
    // The player seeks or closes: this stream is done, and so is its ffmpeg.
    const stop = () => { if (proc.exitCode === null) proc.kill('SIGKILL'); };
    req.on('close', stop);
    res.on('close', stop);
  }

  // ---------------------------------------------------------- subtitles
  // Extracted once per file and track (reading a whole film takes a while),
  // then served shifted to the stream's start time.
  const subJobs = new Map();
  function extractSub(file, n) {
    const stat = fs.statSync(file);
    const hash = crypto.createHash('sha1').update(`${file}|${stat.size}|${stat.mtimeMs}|${n}`).digest('hex').slice(0, 16);
    const out = path.join(cacheDir || require('os').tmpdir(), 'subs', `${hash}.vtt`);
    if (fs.existsSync(out)) return Promise.resolve(out);
    if (subJobs.has(out)) return subJobs.get(out);
    const job = new Promise((resolve, reject) => {
      fs.mkdirSync(path.dirname(out), { recursive: true });
      const tmp = `${out}.part`;
      const proc = spawn(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-nostdin', '-y', '-i', file, '-map', `0:s:${n}`, '-f', 'webvtt', tmp], { windowsHide: true });
      proc.on('error', reject);
      proc.on('close', (code) => {
        if (code === 0 && fs.existsSync(tmp)) { fs.renameSync(tmp, out); resolve(out); } else reject(new Error(`ffmpeg ${code}`));
      });
    }).finally(() => subJobs.delete(out));
    subJobs.set(out, job);
    return job;
  }
  const toSec = (s) => { const p = s.split(':').map(Number); return p.length === 3 ? p[0] * 3600 + p[1] * 60 + p[2] : p[0] * 60 + p[1]; };
  const fromSec = (x) => {
    const ms = Math.round(Math.max(0, x) * 1000);
    const h = Math.floor(ms / 3600000); const m = Math.floor(ms / 60000) % 60; const s = Math.floor(ms / 1000) % 60;
    return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}.${String(ms % 1000).padStart(3, '0')}`;
  };
  function shiftVtt(text, t) {
    if (!t) return text;
    const [head, ...cues] = text.replace(/\r/g, '').split(/\n\n+/);
    const out = [head];
    for (const cue of cues) {
      const m = /((?:\d{1,2}:)?\d{2}:\d{2}\.\d{3}) --> ((?:\d{1,2}:)?\d{2}:\d{2}\.\d{3})/.exec(cue);
      if (!m) continue;
      const end = toSec(m[2]) - t;
      if (end <= 0) continue;
      out.push(cue.replace(m[0], `${fromSec(toSec(m[1]) - t)} --> ${fromSec(end)}`));
    }
    return out.join('\n\n') + '\n';
  }
  async function subtitle(req, res, file, { n = 0, t = 0 } = {}) {
    if (!ffmpeg) { res.writeHead(501); res.end(); return; }
    try {
      const vtt = fs.readFileSync(await extractSub(file, n), 'utf8');
      res.writeHead(200, { 'Content-Type': 'text/vtt; charset=utf-8', 'Cache-Control': 'no-store' });
      res.end(shiftVtt(vtt, t));
    } catch {
      res.writeHead(404, { 'Content-Type': 'text/plain' });
      res.end('kein Untertitel');
    }
  }

  return { available: Boolean(ffmpeg), probe, needsRemux, pickTracks, keyframeBefore, remux, subtitle, shiftVtt };
}

module.exports = { createVideoTools, REMUX_EXT };
