const { app, BrowserWindow, WebContentsView, ipcMain, globalShortcut, screen, session } = require('electron');
const { ElectronBlocker } = require('@ghostery/adblocker-electron');
const { SpotifyAuth } = require('./spotify');
const fetch = require('cross-fetch');
const path = require('path');
const fs = require('fs');

const DATA_DIR = path.join(__dirname, '..', 'data');
const MEDIA_DIR = path.join(DATA_DIR, 'media');
const WEATHER_DIR = path.join(DATA_DIR, 'weather');
const VIDEO_EXTENSIONS = ['.mp4', '.webm', '.mov', '.m4v'];
const ART_MODE_MKV = path.join(DATA_DIR, 'artmode.mkv');
const ART_MODE_MP4 = path.join(DATA_DIR, 'artmode.mp4');

for (const dir of [DATA_DIR, MEDIA_DIR, WEATHER_DIR]) {
  if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
}

function fileUrlSafe(fullPath) {
  return `file:///${fullPath.replace(/\\/g, '/')}`;
}

// Looks for weather/<condition>.mp4 (or .mkv, auto-remuxed the same way
// artmode is). "condition" is 'sunny' or 'rainy' as decided by the renderer
// from the actual Open-Meteo weather code for Murr.
function getWeatherVideo(condition) {
  const mp4 = path.join(WEATHER_DIR, `${condition}.mp4`);
  const mkv = path.join(WEATHER_DIR, `${condition}.mkv`);
  if (fs.existsSync(mkv)) {
    const mkvStat = fs.statSync(mkv);
    if (!fs.existsSync(mp4) || fs.statSync(mp4).mtimeMs < mkvStat.mtimeMs) {
      const { spawnSync } = require('child_process');
      spawnSync('ffmpeg', ['-y', '-i', mkv, '-c', 'copy', mp4]);
    }
  }
  return fs.existsSync(mp4) ? fileUrlSafe(mp4) : null;
}

let newsCache = { items: [], ts: 0 };
async function getNews() {
  const now = Date.now();
  if (newsCache.items.length && now - newsCache.ts < 5 * 60 * 1000) return newsCache.items;
  try {
    const res = await fetch('https://www.tagesschau.de/xml/rss2/');
    const xml = await res.text();
    const items = [];
    for (const block of xml.split('<item>').slice(1)) {
      const match = block.match(/<title>([\s\S]*?)<\/title>/);
      if (!match) continue;
      items.push(match[1].replace('<![CDATA[', '').replace(']]>', '').trim());
      if (items.length >= 5) break;
    }
    if (items.length) newsCache = { items, ts: now };
    return newsCache.items;
  } catch (err) {
    return newsCache.items;
  }
}

// Chromium doesn't reliably play the .mkv container, even when the codec
// inside (usually H.264) would be fine -- so if someone drops an
// artmode.mkv in data/, we remux it (not re-encode, just swap the
// container) to .mp4 with ffmpeg. That's lossless and takes seconds
// regardless of file size, unlike a real transcode.
function ensureArtModeVideoReady() {
  if (!fs.existsSync(ART_MODE_MKV)) return;
  const mkvStat = fs.statSync(ART_MODE_MKV);
  const mp4Exists = fs.existsSync(ART_MODE_MP4);
  if (mp4Exists && fs.statSync(ART_MODE_MP4).mtimeMs >= mkvStat.mtimeMs) return; // already up to date

  const { spawn } = require('child_process');
  console.log('Remuxing artmode.mkv -> artmode.mp4 ...');
  const proc = spawn('ffmpeg', ['-y', '-i', ART_MODE_MKV, '-c', 'copy', ART_MODE_MP4]);
  proc.on('error', (err) => console.error('ffmpeg not available:', err.message));
  proc.on('close', (code) => {
    console.log(code === 0 ? 'Art-mode video ready.' : `ffmpeg exited with code ${code}`);
  });
}

let win;
let contentView = null;
let blocker = null;
let spotify;

// Picks the TV: the external display if one is connected, otherwise falls
// back to the primary (laptop) display so this still runs standalone.
function pickTargetDisplay() {
  const displays = screen.getAllDisplays();
  const primary = screen.getPrimaryDisplay();
  const external = displays.find((d) => d.id !== primary.id);
  return external || primary;
}

async function setupAdBlocking() {
  blocker = await ElectronBlocker.fromLists(fetch, [
    'https://easylist.to/easylist/easylist.txt',
    'https://easylist.to/easylist/easyprivacy.txt',
    'https://raw.githubusercontent.com/AdguardTeam/AdguardFilters/master/YoutubeFilter/sections/adblock.txt'
  ], {
    // Cosmetic filters need session.registerPreloadScript, unavailable on this
    // Electron build; network-level blocking (the part that stops ad requests)
    // works fine without it.
    loadCosmeticFilters: false
  });
  blocker.enableBlockingInSession(session.defaultSession);
}

function createWindow() {
  const target = pickTargetDisplay();

  win = new BrowserWindow({
    x: target.bounds.x,
    y: target.bounds.y,
    width: target.bounds.width,
    height: target.bounds.height,
    fullscreen: true,
    autoHideMenuBar: true,
    backgroundColor: '#0b0e14',
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false
    }
  });

  win.loadFile('src/index.html');

  win.on('resize', () => {
    if (contentView) fitContentView();
  });
}

let contentIsOverscanned = false;

// Normally the content view exactly matches the window. For TV-optimized
// layouts (YouTube TV) that assume a bit of overscan margin real TVs crop
// off-screen, we instead make the view itself larger than the window and
// center it with a negative offset -- Electron clips child views to their
// parent's bounds, so the excess simply runs off-screen, no black bars and
// nothing the page's own CSS/JS can override.
function fitContentView() {
  const [w, h] = win.getContentSize();
  if (contentIsOverscanned) {
    const scale = 1.08;
    const overW = Math.round(w * scale);
    const overH = Math.round(h * scale);
    contentView.setBounds({
      x: -Math.round((overW - w) / 2),
      y: -Math.round((overH - h) / 2),
      width: overW,
      height: overH
    });
  } else {
    contentView.setBounds({ x: 0, y: 0, width: w, height: h });
  }
}

const SMART_TV_UA = 'Mozilla/5.0 (SMART-TV; Linux; Tizen 6.0) AppleWebKit/537.36 (KHTML, like Gecko) Version/6.0 TV Safari/537.36';

function openContentUrl(url, { asSmartTv = false } = {}) {
  if (!contentView) {
    contentView = new WebContentsView({
      webPreferences: {
        contextIsolation: true,
        preload: path.join(__dirname, 'content-preload.js')
      }
    });
    win.contentView.addChildView(contentView);
  }
  contentIsOverscanned = asSmartTv;
  fitContentView();
  // Reset to a normal UA by default so other sites (Netflix, Prime, ...)
  // aren't affected by a leftover Smart-TV identity from a previous open.
  contentView.webContents.setUserAgent(asSmartTv ? SMART_TV_UA : '');
  contentView.webContents.loadURL(url);
}

function closeContentView() {
  if (contentView) {
    win.contentView.removeChildView(contentView);
    contentView.webContents.close();
    contentView = null;
  }
}

function openLiveTvChannel(streamUrl) {
  openContentUrl(`file://${path.join(__dirname, 'livetv.html')}?src=${encodeURIComponent(streamUrl)}`);
}

// Lists ONE directory's contents (not recursive) so the UI can browse it as
// a tree: folders first (alphabetical), then playable video files.
function listMediaDir(relativeDir = '') {
  const targetDir = path.join(MEDIA_DIR, relativeDir);
  const resolved = path.resolve(targetDir);
  // Guard against a relativeDir that escapes MEDIA_DIR (e.g. "../../..").
  if (!resolved.startsWith(path.resolve(MEDIA_DIR))) {
    throw new Error('Path escapes media directory');
  }

  const entries = fs.readdirSync(targetDir, { withFileTypes: true });
  const folders = entries
    .filter((e) => e.isDirectory())
    .map((e) => e.name)
    .sort((a, b) => a.localeCompare(b));
  const files = entries
    .filter((e) => VIDEO_EXTENSIONS.includes(path.extname(e.name).toLowerCase()))
    .map((e) => ({
      name: e.name,
      relativePath: path.join(relativeDir, e.name),
      fullPath: path.join(targetDir, e.name)
    }))
    .sort((a, b) => a.name.localeCompare(b.name));

  return { relativeDir, folders, files };
}

// --- Library: finds "movie folders" (a folder with a video file directly
// inside, optionally alongside Name/Info/Madeby/FSK/Personas metadata
// subfolders like data/media/Movies/Crossroads) anywhere under MEDIA_DIR,
// and reads their metadata for the rich library UI. ---

const LIBRARY_CACHE_FILE = path.join(DATA_DIR, '.library-cache.json');

function readLibraryCache() {
  try { return JSON.parse(fs.readFileSync(LIBRARY_CACHE_FILE, 'utf-8')); } catch { return {}; }
}
function writeLibraryCache(cache) {
  fs.writeFileSync(LIBRARY_CACHE_FILE, JSON.stringify(cache, null, 2));
}

function readMetaFile(folderPath, subfolder) {
  const dir = path.join(folderPath, subfolder);
  if (!fs.existsSync(dir)) return null;
  const txtFile = fs.readdirSync(dir).find((f) => f.toLowerCase().endsWith('.txt'));
  if (!txtFile) return null;
  return fs.readFileSync(path.join(dir, txtFile), 'utf-8').trim();
}

const IMAGE_EXTENSIONS = ['.png', '.jpg', '.jpeg', '.webp'];
function readMetaImage(folderPath, subfolder) {
  const dir = path.join(folderPath, subfolder);
  if (!fs.existsSync(dir)) return null;
  const imgFile = fs.readdirSync(dir).find((f) => IMAGE_EXTENSIONS.includes(path.extname(f).toLowerCase()));
  if (!imgFile) return null;
  // Windows paths use backslashes, which CSS url('...') treats as escape
  // characters -- normalize to forward slashes so file:// URLs stay intact
  // when used inside a background-image: url('...') in the renderer.
  const fullPath = path.join(dir, imgFile).replace(/\\/g, '/');
  return `file:///${fullPath}`;
}

function getVideoDuration(fullPath) {
  const cache = readLibraryCache();
  const mtimeMs = fs.statSync(fullPath).mtimeMs;
  const cached = cache[fullPath];
  if (cached && cached.mtimeMs === mtimeMs) return Promise.resolve(cached.duration);

  return new Promise((resolve) => {
    const { execFile } = require('child_process');
    execFile('ffprobe', ['-v', 'quiet', '-print_format', 'json', '-show_format', fullPath], (err, stdout) => {
      let duration = null;
      try { duration = Math.round(JSON.parse(stdout).format.duration); } catch { /* leave null */ }
      const updated = readLibraryCache();
      updated[fullPath] = { mtimeMs, duration };
      writeLibraryCache(updated);
      resolve(duration);
    });
  });
}

function findMovieFolders(dir = MEDIA_DIR) {
  let found = [];
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  const videoFile = entries.find((e) => e.isFile() && VIDEO_EXTENSIONS.includes(path.extname(e.name).toLowerCase()));

  if (videoFile) {
    found.push({ folder: dir, videoFile: path.join(dir, videoFile.name) });
  }
  for (const e of entries) {
    if (e.isDirectory() && !['Banner', 'FSK', 'Icon', 'Icon_2', 'Info', 'Madeby', 'Name', 'Personas'].includes(e.name)) {
      found = found.concat(findMovieFolders(path.join(dir, e.name)));
    }
  }
  return found;
}

async function listLibrary() {
  const movieFolders = findMovieFolders();
  const items = [];
  for (const { folder, videoFile } of movieFolders) {
    const rawName = readMetaFile(folder, 'Name');
    const title = (rawName ? rawName.replace(/^-/, '').trim() : null) || path.basename(folder);
    const info = readMetaFile(folder, 'Info');
    const fsk = readMetaFile(folder, 'FSK');
    const cast = readMetaFile(folder, 'Personas');
    const crew = readMetaFile(folder, 'Madeby');
    const genreMatch = info && info.match(/Genre:\s*([^\n]+)/i);
    const genre = genreMatch ? genreMatch[1].split(',')[0].trim() : null;
    const duration = await getVideoDuration(videoFile);
    const icon = readMetaImage(folder, 'Icon') || readMetaImage(folder, 'Icon_2');
    const banner = readMetaImage(folder, 'Banner');

    items.push({
      title,
      relativePath: path.relative(MEDIA_DIR, videoFile),
      fullPath: videoFile,
      info,
      fsk,
      cast,
      icon,
      banner,
      crew,
      genre,
      duration // seconds, or null if ffprobe couldn't read it
    });
  }
  return items;
}

// Scaffolds an organizing folder structure under MEDIA_DIR. Nothing here is
// auto-populated with real actor/director names -- there's no metadata
// source wired up for that -- it's just labeled folders to sort files into
// by hand (or to point a future metadata-lookup feature at).
function scaffoldMediaFolders() {
  const structure = [
    'Filme',
    'Serien',
    'Nach Schauspieler',
    'Nach Regisseur',
    'Nach Genre'
  ];
  for (const folder of structure) {
    const dir = path.join(MEDIA_DIR, folder);
    if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
  }
  return structure;
}

function openMediaFile(fullPath, relativePath, resumeSeconds) {
  const params = new URLSearchParams({
    src: 'file://' + fullPath,
    rel: relativePath || '',
    resume: String(resumeSeconds || 0)
  });
  openContentUrl(`file://${path.join(__dirname, 'mediaplayer.html')}?${params.toString()}`);
}

ipcMain.handle('open-url', (event, url, opts) => openContentUrl(url, opts));
ipcMain.handle('open-livetv', (event, streamUrl) => openLiveTvChannel(streamUrl));
ipcMain.handle('list-media-dir', (event, relativeDir) => listMediaDir(relativeDir || ''));
ipcMain.handle('list-library', () => listLibrary());
ipcMain.handle('get-random-movie', async () => {
  const items = await listLibrary();
  if (items.length === 0) return null;
  return items[Math.floor(Math.random() * items.length)];
});
ipcMain.handle('get-weather-video', (event, condition) => getWeatherVideo(condition));
ipcMain.handle('get-news', () => getNews());
ipcMain.handle('open-media', (event, fullPath, relativePath, resumeSeconds) =>
  openMediaFile(fullPath, relativePath, resumeSeconds)
);
ipcMain.on('media-progress', (event, relativePath, seconds) => {
  win.webContents.send('media-progress-update', relativePath, seconds);
});
ipcMain.handle('open-media-folder', () => require('electron').shell.openPath(MEDIA_DIR));
ipcMain.handle('scaffold-media-folders', () => scaffoldMediaFolders());
ipcMain.handle('get-artmode-video', () => {
  return fs.existsSync(ART_MODE_MP4) ? `file://${ART_MODE_MP4}` : null;
});
ipcMain.handle('close-content', () => closeContentView());
ipcMain.handle('get-displays', () => {
  return screen.getAllDisplays().map((d) => ({
    id: d.id,
    label: `${d.size.width}x${d.size.height}${d.id === screen.getPrimaryDisplay().id ? ' (primär)' : ''}`
  }));
});
ipcMain.handle('set-adblock', (event, enabled) => {
  if (!blocker) return;
  if (enabled) blocker.enableBlockingInSession(session.defaultSession);
  else blocker.disableBlockingInSession(session.defaultSession);
});
ipcMain.handle('toggle-fullscreen', () => {
  win.setFullScreen(!win.isFullScreen());
});

ipcMain.handle('spotify-is-logged-in', () => spotify.isLoggedIn());
ipcMain.handle('spotify-login', async () => {
  try {
    await spotify.login((authUrl) => openContentUrl(authUrl));
    closeContentView();
    win.webContents.send('content-closed');
    win.webContents.send('spotify-logged-in');
    return { ok: true };
  } catch (err) {
    return { ok: false, error: String(err.message || err) };
  }
});
ipcMain.handle('spotify-now-playing', () => spotify.nowPlaying());
ipcMain.handle('spotify-control', (event, action) => spotify.control(action));

app.whenReady().then(async () => {
  spotify = new SpotifyAuth(app.getPath('userData'));
  await setupAdBlocking();
  ensureArtModeVideoReady();
  createWindow();

  globalShortcut.register('Escape', () => {
    closeContentView();
    win.webContents.send('content-closed');
  });

  // Dev/debug escape hatch: leave fullscreen without needing the remote.
  globalShortcut.register('F11', () => {
    win.setFullScreen(!win.isFullScreen());
  });
});

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit();
});
