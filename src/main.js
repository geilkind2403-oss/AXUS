const { app, BrowserWindow, WebContentsView, ipcMain, globalShortcut, screen, session, Menu, shell } = require('electron');
const { ElectronBlocker } = require('@ghostery/adblocker-electron');
const { startRemoteServer } = require('./remote-server');
const tvos = require('./tvos-main'); // TV-OS layer: content hardening, load/error states, network, standby, toasts
const sleepTimer = require('./sleep-timer-main'); // Control Center sleep timer
const appliance = require('./appliance-main'); // autostart, watchdog, single instance
const memory = require('./memory-main'); // 1 GB RAM budget, trims above it
const music = require('./music-main'); // music library: Musik-Server on the PC + data/music

// gc() in the renderers, for memory-main.js's trim.
app.commandLine.appendSwitch('js-flags', '--expose-gc');

// A second start (autostart + a click, the watchdog racing a relaunch) just
// brings the running launcher to the front. exit, not quit: this copy must not
// run any quit handlers.
if (!appliance.claimSingleInstance(() => win)) app.exit(0);
const fetch = require('cross-fetch');
const path = require('path');
const fs = require('fs');
const { execFile } = require('child_process');

const DATA_DIR = path.join(__dirname, '..', 'data');
const MEDIA_DIR = path.join(DATA_DIR, 'media');
const WEATHER_DIR = path.join(DATA_DIR, 'weather');
// MKV & co play through a live remux (localVideo below, server/videotools.js).
const VIDEO_EXTENSIONS = ['.mp4', '.webm', '.mov', '.m4v', '.mkv', '.avi', '.ts', '.m2ts', '.mts', '.wmv', '.mpg', '.mpeg'];
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

// News for the home tile: [{ title, topline, date, image, url }].
// Primary source is Tagesschau's homepage JSON -- editorial order (top story
// first), the kicker ("topline") and 16:9 photos in several sizes. The RSS feed
// has no kicker, is ordered by time and only carries one 1920 px photo per item
// inside content:encoded, so it stays as the fallback for when the API is down
// or changes shape (titles then still arrive, photos if the feed has them).
const NEWS_API_URL = 'https://www.tagesschau.de/api2u/homepage/';
const NEWS_RSS_URL = 'https://www.tagesschau.de/xml/rss2/';
const NEWS_MAX_ITEMS = 6;
const NEWS_TTL_MS = 5 * 60 * 1000;

// URLs end up in the renderer's <img src>: accept plain https only.
const newsHttps = (u) => (typeof u === 'string' && /^https:\/\//i.test(u) ? u : '');

function parseNewsApi(json) {
  const items = [];
  for (const n of (json && json.news) || []) {
    if (!n || !n.title || n.type === 'webview') continue;
    const url = newsHttps(n.shareURL);
    // The forecast page is no news, and the weather tile sits right next to it.
    if (!url || url.includes('/wetter/')) continue;
    const variants = (n.teaserImage && n.teaserImage.imageVariants) || {};
    // 960 px wide is sharp enough for the tile at devicePixelRatio 2 and ~60 KB.
    const image = newsHttps(variants['16x9-960'] || variants['16x9-1280'] || variants['16x9-640'] || variants['16x9-512']);
    items.push({ title: String(n.title).trim(), topline: String(n.topline || '').trim(), date: n.date || '', image, url });
    if (items.length >= NEWS_MAX_ITEMS) break;
  }
  return items;
}

function parseNewsRss(xml) {
  const items = [];
  for (const block of xml.split('<item>').slice(1)) {
    const tag = (name) => {
      const m = block.match(new RegExp(`<${name}>([\\s\\S]*?)</${name}>`));
      if (!m) return '';
      const raw = m[1].trim();
      if (raw.startsWith('<![CDATA[')) return raw.replace('<![CDATA[', '').replace(/\]\]>$/, '').trim();
      // Outside CDATA the feed is entity-encoded.
      return raw.replace(/&(#x[0-9a-f]+|#\d+|amp|lt|gt|quot|apos);/gi, (all, e) => {
        if (e[0] === '#') return String.fromCodePoint(e[1].toLowerCase() === 'x' ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10));
        return { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'" }[e.toLowerCase()];
      });
    };
    const title = tag('title');
    if (!title) continue;
    const img = block.match(/<img[^>]+src="([^"]+)"/);
    const pub = new Date(tag('pubDate'));
    items.push({
      title,
      topline: '',
      date: tag('dc:date') || (Number.isNaN(pub.getTime()) ? '' : pub.toISOString()),
      image: newsHttps(img ? img[1].replace(/([?&]width=)\d+/, '$1960') : ''),
      url: newsHttps(tag('link'))
    });
    if (items.length >= NEWS_MAX_ITEMS) break;
  }
  return items;
}

let newsCache = { items: [], ts: 0 };
async function getNews() {
  const now = Date.now();
  if (newsCache.items.length && now - newsCache.ts < NEWS_TTL_MS) return newsCache.items;
  let items = [];
  try {
    const res = await fetch(NEWS_API_URL, { timeout: 8000 });
    if (res.ok) items = parseNewsApi(await res.json());
  } catch (err) { /* fall through to the feed */ }
  if (!items.length) {
    try {
      const res = await fetch(NEWS_RSS_URL, { timeout: 8000 });
      if (res.ok) items = parseNewsRss(await res.text());
    } catch (err) { /* keep whatever is cached */ }
  }
  if (items.length) newsCache = { items, ts: now };
  return newsCache.items;
}

// Windows has no WiFi API exposed to Electron, so this shells out to netsh
// (the standard way desktop apps do it on Windows) and parses its text
// output. execFile (not exec) is used deliberately -- it passes each
// argument straight to netsh.exe with no shell involved, so a malicious
// nearby access point can't broadcast an SSID crafted for shell injection.
// netsh's own labels are localized (this was built and tested against a
// German Windows install, e.g. "Authentifizierung" not "Authentication"),
// so every label is matched in both German and English to be safe.
function runNetsh(args) {
  return new Promise((resolve, reject) => {
    execFile('netsh', args, { windowsHide: true }, (err, stdout) => {
      if (err) reject(err); else resolve(stdout);
    });
  });
}

async function getWifiStatus() {
  try {
    const out = await runNetsh(['wlan', 'show', 'interfaces']);
    const stateMatch = out.match(/^\s*(?:Status|State)\s*:\s*(.+?)\s*$/m);
    const ssidMatch = out.match(/^\s*SSID\s*:\s*(.+?)\s*$/m);
    const signalMatch = out.match(/Signal\s*:\s*(\d+)%/);
    const connected = !!stateMatch && /^(verbunden|connected)$/i.test(stateMatch[1]);
    return {
      connected,
      ssid: connected && ssidMatch ? ssidMatch[1].trim() : null,
      signal: connected && signalMatch ? Number(signalMatch[1]) : null
    };
  } catch {
    return { connected: false, ssid: null, signal: null };
  }
}

async function getWifiNetworks() {
  try {
    const [netOut, profOut] = await Promise.all([
      runNetsh(['wlan', 'show', 'networks', 'mode=bssid']),
      runNetsh(['wlan', 'show', 'profiles'])
    ]);
    const profiles = new Set(
      [...profOut.matchAll(/(?:Profil für alle Benutzer|All User Profile)\s*:\s*(.+?)\s*$/gm)].map(m => m[1].trim())
    );
    const blocks = netOut.split(/\r?\n(?=SSID \d+ : )/);
    const bySsid = new Map();
    for (const block of blocks) {
      const nameMatch = block.match(/^SSID \d+ : (.*)$/m);
      if (!nameMatch) continue;
      const ssid = nameMatch[1].trim();
      if (!ssid) continue; // hidden network, nothing to show or connect to
      const authMatch = block.match(/^\s*(?:Authentifizierung|Authentication)\s*:\s*(.+?)\s*$/m);
      const signalMatch = block.match(/Signal\s*:\s*(\d+)%/);
      const auth = authMatch ? authMatch[1] : '';
      const signal = signalMatch ? Number(signalMatch[1]) : 0;
      const existing = bySsid.get(ssid);
      if (!existing || signal > existing.signal) {
        bySsid.set(ssid, { ssid, signal, open: /offen|open/i.test(auth), hasProfile: profiles.has(ssid) });
      }
    }
    return Array.from(bySsid.values()).sort((a, b) => b.signal - a.signal);
  } catch {
    return [];
  }
}

// Connects to a network Windows already knows (a saved profile) or an open one.
async function connectWifi(ssid) {
  try {
    await runNetsh(['wlan', 'connect', `name="${ssid}"`, `ssid="${ssid}"`]);
    return { ok: true };
  } catch (err) {
    return { ok: false, error: String(err.message || err) };
  }
}

function escapeXml(str) {
  return String(str).replace(/[<>&'"]/g, (c) => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;', "'": '&apos;', '"': '&quot;' }[c]));
}

// A brand-new secured network has no saved profile yet, and `wlan connect`
// itself takes no password -- Windows only accepts a new WPA2 key via a
// profile XML, so one is written to a temp file, registered with
// `wlan add profile`, then connected to normally and deleted either way.
async function connectWifiWithPassword(ssid, password) {
  const tmpFile = path.join(app.getPath('temp'), `wifi-profile-${Date.now()}.xml`);
  const xml = `<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
  <name>${escapeXml(ssid)}</name>
  <SSIDConfig><SSID><name>${escapeXml(ssid)}</name></SSID></SSIDConfig>
  <connectionType>ESS</connectionType>
  <connectionMode>auto</connectionMode>
  <MSM><security>
    <authEncryption>
      <authentication>WPA2PSK</authentication>
      <encryption>AES</encryption>
      <useOneX>false</useOneX>
    </authEncryption>
    <sharedKey>
      <keyType>passPhrase</keyType>
      <protected>false</protected>
      <keyMaterial>${escapeXml(password)}</keyMaterial>
    </sharedKey>
  </security></MSM>
</WLANProfile>`;
  try {
    fs.writeFileSync(tmpFile, xml, 'utf8');
    await runNetsh(['wlan', 'add', 'profile', `filename="${tmpFile}"`]);
    await runNetsh(['wlan', 'connect', `name="${ssid}"`, `ssid="${ssid}"`]);
    return { ok: true };
  } catch (err) {
    return { ok: false, error: String(err.message || err) };
  } finally {
    fs.unlink(tmpFile, () => {});
  }
}

// Windows has no netsh-equivalent CLI for Bluetooth. Adapter state and
// already-paired devices are cheap to read via PnP device enumeration
// (below). Discovering nearby *unpaired* devices and pairing them needs the
// WinRT Windows.Devices.Bluetooth API, which is event-driven -- and Windows
// PowerShell 5.1 flatly refuses to subscribe to WinRT events
// ("Register-ObjectEvent" throws "Windows RT events cannot be subscribed to
// from Windows PowerShell"). Compiled C# has no such restriction, so
// scanning/pairing runs through bt-helper.exe instead, compiled on demand
// from src/bt-helper.cs the first time it's needed (and recompiled if that
// source file changes) using csc.exe, which ships with every .NET
// Framework install -- see ensureBtHelperCompiled().
function runPowerShell(script) {
  return new Promise((resolve, reject) => {
    execFile('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], { windowsHide: true }, (err, stdout) => {
      if (err) reject(err); else resolve(stdout);
    });
  });
}

async function getBluetoothStatus() {
  try {
    const adapterOut = await runPowerShell(
      "Get-PnpDevice -Class Bluetooth -PresentOnly | Where-Object { $_.FriendlyName -match 'Adapter' } | Select-Object -First 1 -ExpandProperty Status"
    );
    const radioOn = adapterOut.trim().toUpperCase() === 'OK';

    const devicesOut = await runPowerShell(
      "Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'BTHENUM*' -and $_.FriendlyName -notlike '*Enumerator*' } | Select-Object FriendlyName, Status | ConvertTo-Json -Compress"
    );
    let devices = [];
    try {
      const parsed = JSON.parse(devicesOut || '[]');
      devices = (Array.isArray(parsed) ? parsed : [parsed]).map((d) => ({
        name: d.FriendlyName,
        connected: String(d.Status).toUpperCase() === 'OK'
      }));
    } catch { /* leave devices empty */ }

    return { radioOn, devices };
  } catch {
    return { radioOn: false, devices: [] };
  }
}

function openBluetoothSettings() {
  shell.openExternal('ms-settings:bluetooth');
}

const BT_HELPER_SRC = path.join(__dirname, 'bt-helper.cs');

function ensureBtHelperCompiled() {
  return new Promise((resolve, reject) => {
    const exePath = path.join(app.getPath('userData'), 'bt-helper.exe');
    if (fs.existsSync(exePath) && fs.statSync(exePath).mtimeMs >= fs.statSync(BT_HELPER_SRC).mtimeMs) {
      resolve(exePath);
      return;
    }
    const csc = [
      'C:\\Windows\\Microsoft.NET\\Framework64\\v4.0.30319\\csc.exe',
      'C:\\Windows\\Microsoft.NET\\Framework\\v4.0.30319\\csc.exe'
    ].find((p) => fs.existsSync(p));
    if (!csc) { reject(new Error('csc.exe nicht gefunden (.NET Framework fehlt?)')); return; }
    const facade = [
      // Bundled with the project so compiling bt-helper.exe doesn't depend
      // on the target machine having Visual Studio / the .NET SDK
      // installed (only csc.exe + WinMetadata, which ship with Windows
      // itself, are required) -- see the comment on getBluetoothStatus().
      path.join(__dirname, 'System.Runtime.dll'),
      'C:\\Program Files (x86)\\Reference Assemblies\\Microsoft\\Framework\\.NETFramework\\v4.8\\Facades\\System.Runtime.dll',
      'C:\\Program Files (x86)\\Reference Assemblies\\Microsoft\\Framework\\.NETFramework\\v4.7.2\\Facades\\System.Runtime.dll',
      'C:\\Program Files (x86)\\Reference Assemblies\\Microsoft\\Framework\\.NETFramework\\v4.6.1\\Facades\\System.Runtime.dll'
    ].find((p) => fs.existsSync(p));
    if (!facade) { reject(new Error('System.Runtime-Referenzassembly nicht gefunden')); return; }

    const args = [
      '/nologo', `/out:${exePath}`,
      '/reference:C:\\Windows\\System32\\WinMetadata\\Windows.Devices.winmd',
      '/reference:C:\\Windows\\System32\\WinMetadata\\Windows.Foundation.winmd',
      '/reference:System.Runtime.WindowsRuntime.dll',
      '/reference:System.Runtime.InteropServices.WindowsRuntime.dll',
      `/reference:${facade}`,
      BT_HELPER_SRC
    ];
    execFile(csc, args, { windowsHide: true }, (err, stdout, stderr) => {
      if (err) { reject(new Error(stderr || err.message)); return; }
      resolve(exePath);
    });
  });
}

async function scanBluetoothDevices(seconds) {
  try {
    const exePath = await ensureBtHelperCompiled();
    const out = await new Promise((resolve, reject) => {
      execFile(exePath, ['scan', String(seconds)], { windowsHide: true, timeout: (seconds + 10) * 1000 }, (err, stdout) => {
        if (err) reject(err); else resolve(stdout);
      });
    });
    return { devices: JSON.parse(out.trim() || '[]'), error: null };
  } catch (err) {
    return { devices: [], error: String(err.message || err) };
  }
}

async function pairBluetoothDevice(deviceId) {
  try {
    const exePath = await ensureBtHelperCompiled();
    const out = await new Promise((resolve, reject) => {
      execFile(exePath, ['pair', deviceId], { windowsHide: true, timeout: 25000 }, (err, stdout) => {
        if (err) reject(err); else resolve(stdout);
      });
    });
    return JSON.parse(out.trim());
  } catch (err) {
    return { ok: false, error: String(err.message || err) };
  }
}

// --- Master volume (Control Center slider): vol-helper.cs, compiled once
// with the csc.exe that ships with Windows -- same approach as bt-helper,
// but it only needs plain COM interop, so no WinMetadata references.
const VOL_HELPER_SRC = path.join(__dirname, 'vol-helper.cs');
let volHelperPromise = null;

function ensureVolHelperCompiled() {
  if (volHelperPromise) return volHelperPromise;
  volHelperPromise = new Promise((resolve, reject) => {
    const exePath = path.join(app.getPath('userData'), 'vol-helper.exe');
    if (fs.existsSync(exePath) && fs.statSync(exePath).mtimeMs >= fs.statSync(VOL_HELPER_SRC).mtimeMs) {
      resolve(exePath);
      return;
    }
    const csc = [
      'C:\\Windows\\Microsoft.NET\\Framework64\\v4.0.30319\\csc.exe',
      'C:\\Windows\\Microsoft.NET\\Framework\\v4.0.30319\\csc.exe'
    ].find((p) => fs.existsSync(p));
    if (!csc) { reject(new Error('csc.exe nicht gefunden (.NET Framework fehlt?)')); return; }
    execFile(csc, ['/nologo', `/out:${exePath}`, VOL_HELPER_SRC], { windowsHide: true }, (err, stdout, stderr) => {
      if (err) { reject(new Error(stderr || stdout || err.message)); return; }
      resolve(exePath);
    });
  });
  volHelperPromise.catch(() => { volHelperPromise = null; }); // let a later call retry
  return volHelperPromise;
}

async function runVolHelper(args) {
  try {
    const exePath = await ensureVolHelperCompiled();
    const out = await new Promise((resolve, reject) => {
      execFile(exePath, args, { windowsHide: true, timeout: 5000 }, (err, stdout) => {
        if (err && !stdout) reject(err); else resolve(stdout);
      });
    });
    const state = JSON.parse(out.trim());
    return state.error ? { available: false, error: state.error } : { available: true, level: state.level, muted: state.muted };
  } catch (err) {
    return { available: false, error: String(err.message || err) };
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
    // No OS title bar/buttons even if fullscreen is ever left (F11 / Settings):
    // a TV has no window chrome. Keeps the name for Alt+Tab and the taskbar.
    frame: false,
    title: 'TV Launcher',
    autoHideMenuBar: true,
    backgroundColor: '#0a0a0c', // matches --bg-1, so there's no repaint flash under the boot splash
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      spellcheck: false // no red squiggles, ever
    }
  });

  win.loadFile('src/index.html');
  tvos.attachWindow(win);

  win.on('resize', () => {
    if (contentView) fitContentView();
  });

  // A frozen or crashed renderer would otherwise just sit there dead (or
  // Electron would show its own "has stopped working"-style prompt) -- a
  // real TV OS never gets stuck on a broken screen, it recovers on its own.
  win.webContents.on('render-process-gone', (event, details) => {
    console.error('Renderer process gone, relaunching:', details.reason);
    app.relaunch();
    app.exit(0);
  });
  win.webContents.on('unresponsive', () => {
    console.error('Renderer unresponsive, relaunching');
    app.relaunch();
    app.exit(0);
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
  if (tvos.isContentHidden()) {
    // Still loading (or failed): park the view just below the window edge, where
    // the parent clips it, while the renderer shows the launch splash / error
    // screen. Unlike setVisible(false) the page keeps rendering normally.
    contentView.setBounds({ x: 0, y: h, width: w, height: h });
    return;
  }
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
        spellcheck: false, // no red squiggles in search boxes
        preload: path.join(__dirname, 'content-preload.js')
      }
    });
    tvos.attachContent(contentView); // before the first layout: it starts parked off-screen
    win.contentView.addChildView(contentView);
    // Escape is only grabbed as a global (OS-level) shortcut while the
    // content view exists, because that view has its own native focus and
    // never lets a normal keydown reach the renderer's own listener -- that's
    // the only situation where the renderer can't just handle Escape itself.
    // Registering it globally all the time used to swallow every Escape
    // press before it reached the renderer, so closing an HTML overlay
    // (Settings, Mediathek, Live TV, Library) with Escape silently did
    // nothing: its own open/closed state never flipped, and every arrow key
    // after that kept being routed into the overlay that (as far as the app
    // knew) was still open.
    globalShortcut.register('Escape', () => {
      sleepTimer.onInput();
      if (contentHandlesBack()) { contentView.webContents.send('tv-back'); return; }
      closeContentView();
      win.webContents.send('content-closed');
    });
  }
  contentIsOverscanned = asSmartTv;
  fitContentView();
  // Reset to a normal UA by default so other sites (Netflix, Prime, ...)
  // aren't affected by a leftover Smart-TV identity from a previous open.
  contentView.webContents.setUserAgent(asSmartTv ? SMART_TV_UA : '');
  // The page loads off-screen while the renderer shows a brand splash (see
  // tvos-main.js); a failed load becomes a TV error screen. The catch matters:
  // loadURL rejects when offline, and an unhandled rejection here would end up
  // in the uncaughtException handler below and restart the whole launcher.
  tvos.beginContentLoad(url);
  contentView.webContents.loadURL(url).catch(() => { /* reported via did-fail-load */ });
}

function closeContentView() {
  if (contentView) {
    tvos.onContentClosed();
    win.contentView.removeChildView(contentView);
    contentView.webContents.close();
    contentView = null;
    globalShortcut.unregister('Escape');
    // The content view is a separate native WebContentsView with its own
    // keyboard focus while it's open. Removing it doesn't hand focus back to
    // the main window on its own, so without this the renderer's keydown
    // listener goes dead (arrow keys do nothing) until the user clicks
    // somewhere to force-refocus the window.
    win.webContents.focus();
  }
}

// --- Live TV app (livetv.html): a full-screen TV app with its own channel
// guide, fed by the M3U playlist in data/livetv/. Without a stream it resumes
// the last watched channel.
const LIVETV_DIR = path.join(DATA_DIR, 'livetv');

function openLiveTvChannel(streamUrl) {
  const query = streamUrl ? `?src=${encodeURIComponent(streamUrl)}` : '';
  openContentUrl(`file://${path.join(__dirname, 'livetv.html')}${query}`);
}

// The Live TV page steps back itself (guide open -> close the guide) and only
// asks to be closed when there's nothing left to step back from.
function contentHandlesBack() {
  return Boolean(contentView) && /\/livetv\.html/i.test(contentView.webContents.getURL()) && !tvos.isContentHidden();
}

// "IPTV-DE" -> "DE". YouTube pages aren't streams a <video> can play, so
// they're left out rather than shown as channels that never work.
function parseM3u(text) {
  const channels = [];
  let meta = null;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    if (line.startsWith('#EXTINF')) {
      const attr = (name) => { const m = line.match(new RegExp(`${name}="([^"]*)"`)); return m ? m[1] : ''; };
      const comma = line.lastIndexOf(',');
      meta = {
        name: (comma >= 0 ? line.slice(comma + 1) : '').trim() || attr('tvg-name'),
        group: attr('group-title'),
        logo: attr('tvg-logo')
      };
    } else if (!line.startsWith('#') && meta) {
      if (/^https?:\/\//i.test(line) && !/youtube\.com|youtu\.be/i.test(line)) {
        const cc = (meta.group.match(/-([A-Z]{2})$/i) || [])[1];
        channels.push({
          number: channels.length + 1,
          name: meta.name,
          country: cc ? cc.toUpperCase() : '',
          logo: /^https?:\/\//i.test(meta.logo) ? meta.logo : '',
          url: line
        });
      }
      meta = null;
    }
  }
  return channels;
}

function readLiveTvPlaylist() {
  try {
    const file = fs.readdirSync(LIVETV_DIR).find((f) => /\.m3u8?$/i.test(f));
    if (!file) return [];
    return parseM3u(fs.readFileSync(path.join(LIVETV_DIR, file), 'utf8'));
  } catch {
    return [];
  }
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

// `cache` is the duration cache read once per library listing; new entries
// are collected in `fresh` and written in one go afterwards (it used to be
// read -- and written -- again for every single film).
function getVideoDuration(fullPath, cache, fresh) {
  const mtimeMs = fs.statSync(fullPath).mtimeMs;
  const cached = cache[fullPath];
  if (cached && cached.mtimeMs === mtimeMs) return Promise.resolve(cached.duration);

  return new Promise((resolve) => {
    execFile('ffprobe', ['-v', 'quiet', '-print_format', 'json', '-show_format', fullPath], (err, stdout) => {
      let duration = null;
      try { duration = Math.round(JSON.parse(stdout).format.duration); } catch { /* leave null */ }
      fresh[fullPath] = { mtimeMs, duration };
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

// --- Media server on another PC (server/mediaserver.js): its films show up
// in the Mediathek next to the local ones and are streamed over the LAN.
// Address lives in data/mediaserver.json ({ "url": "http://192.168.0.21:8420" }).
const MEDIASERVER_FILE = path.join(DATA_DIR, 'mediaserver.json');
function getMediaServerUrl() {
  try { return (JSON.parse(fs.readFileSync(MEDIASERVER_FILE, 'utf8')).url || '').replace(/\/+$/, ''); } catch { return ''; }
}
async function setMediaServerUrl(raw) {
  let url = String(raw || '').trim();
  if (url && !/^https?:\/\//i.test(url)) url = `http://${url}`;
  if (url && !/:\d+(\/|$)/.test(url.replace(/^https?:\/\//i, ''))) url += ':8420';
  url = url.replace(/\/+$/, '');
  if (!url) { fs.writeFileSync(MEDIASERVER_FILE, JSON.stringify({ url: '' })); return { ok: true, url: '' }; }
  const ping = await pingMediaServer(url);
  if (!ping.ok) return ping;
  fs.writeFileSync(MEDIASERVER_FILE, JSON.stringify({ url }, null, 2));
  return { ok: true, url, name: ping.name, films: ping.films };
}
async function fetchWithTimeout(url, opts = {}, ms = 4000) {
  const ctrl = new AbortController();
  const t = setTimeout(() => ctrl.abort(), ms);
  try { return await fetch(url, { ...opts, signal: ctrl.signal }); } finally { clearTimeout(t); }
}
async function pingMediaServer(url = getMediaServerUrl()) {
  if (!url) return { ok: false, error: 'Kein Server eingetragen' };
  try {
    const res = await fetchWithTimeout(`${url}/api/ping`, {}, 3000);
    const j = await res.json();
    return { ok: true, url, name: j.name, films: j.films };
  } catch {
    return { ok: false, url, error: 'Server nicht erreichbar' };
  }
}

// Server-side resume positions, keyed by the relativePath the launcher uses
// for remote films ("server:<id>/<path>").
const remotePositions = new Map();
// MKV & co from the server: how to play them (remux stream, tracks, subtitles).
const remotePlayInfo = new Map();
async function listRemoteLibrary() {
  const url = getMediaServerUrl();
  if (!url) return [];
  try {
    const res = await fetchWithTimeout(`${url}/api/library`);
    const films = await res.json();
    return films.map((f) => {
      const relativePath = `server:${f.id}/${f.relativePath}`;
      remotePositions.set(relativePath, f.position || 0);
      if (f.play) remotePlayInfo.set(relativePath, { ...f.play, seek: f.stream.replace('/play/', '/seek/') });
      return {
        title: f.title, relativePath, fullPath: f.stream, info: f.info, fsk: f.fsk, cast: f.cast,
        icon: f.icon, banner: f.banner, crew: f.crew, genre: f.genre, duration: f.duration, position: f.position || 0, remote: true
      };
    });
  } catch {
    return []; // PC off / unreachable: the Mediathek just shows the local films
  }
}
function reportRemoteProgress(relativePath, seconds) {
  const url = getMediaServerUrl();
  const m = /^server:([^/]+)\//.exec(relativePath);
  if (!url || !m) return;
  remotePositions.set(relativePath, seconds);
  fetchWithTimeout(`${url}/api/progress/${m[1]}`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ seconds })
  }).catch(() => {});
}

// Boot preloading, the random-film tile and the Weiterschauen tile all ask
// at about the same time: callers that arrive while a listing is running share
// it instead of each scanning the disk (synchronously, in this process) and
// asking the media server again.
let libraryListing = null;
function listLibrary() {
  if (!libraryListing) {
    libraryListing = Promise.all([listLocalLibrary(), listRemoteLibrary()])
      .then(([local, remote]) => [...local, ...remote])
      .finally(() => { libraryListing = null; });
  }
  return libraryListing;
}

async function listLocalLibrary() {
  const movieFolders = findMovieFolders();
  const items = [];
  const durationCache = readLibraryCache();
  const freshDurations = {};
  for (const { folder, videoFile } of movieFolders) {
    const rawName = readMetaFile(folder, 'Name');
    const title = (rawName ? rawName.replace(/^-/, '').trim() : null) || path.basename(folder);
    const info = readMetaFile(folder, 'Info');
    const fsk = readMetaFile(folder, 'FSK');
    const cast = readMetaFile(folder, 'Personas');
    const crew = readMetaFile(folder, 'Madeby');
    const genreMatch = info && info.match(/Genre:\s*([^\n]+)/i);
    const genre = genreMatch ? genreMatch[1].split(',')[0].trim() : null;
    const duration = await getVideoDuration(videoFile, durationCache, freshDurations);
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
  if (Object.keys(freshDurations).length) writeLibraryCache({ ...readLibraryCache(), ...freshDurations });
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

// --- MKV & co on this laptop (data/media): the same live remux as the film
// server, served on 127.0.0.1 only, for files inside MEDIA_DIR only.
let localVideo = null;
function getLocalVideo() {
  if (localVideo) return localVideo;
  let tools = null;
  try {
    const { createVideoTools } = require(path.join(__dirname, '..', 'server', 'videotools.js'));
    tools = createVideoTools({ cacheDir: DATA_DIR, log: (...a) => console.log(...a) });
  } catch (err) {
    console.error('server/videotools.js fehlt -- MKV auf dem Laptop geht nicht:', err.message);
  }
  localVideo = { tools, port: 0, ready: null };
  if (!tools || !tools.available) return localVideo;
  const inMedia = (rel) => {
    const file = path.resolve(MEDIA_DIR, String(rel || ''));
    return file.startsWith(path.resolve(MEDIA_DIR) + path.sep) && fs.existsSync(file) ? file : null;
  };
  const server = require('http').createServer((req, res) => {
    const url = new URL(req.url, 'http://x');
    const file = inMedia(url.searchParams.get('f'));
    const num = (k) => Math.max(0, Number(url.searchParams.get(k)) || 0);
    if (!file) { res.writeHead(404); res.end(); return; }
    if (url.pathname === '/play') tools.remux(req, res, file, { t: num('t'), a: num('a'), v: url.searchParams.get('v') || '' });
    else if (url.pathname === '/seek') { res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(JSON.stringify({ t: tools.keyframeBefore(file, num('t')) })); }
    else if (url.pathname === '/sub') tools.subtitle(req, res, file, { n: num('n'), t: num('t') });
    else { res.writeHead(404); res.end(); }
  });
  localVideo.ready = new Promise((resolve) => server.listen(0, '127.0.0.1', () => { localVideo.port = server.address().port; resolve(); }));
  return localVideo;
}

async function localPlayInfo(fullPath) {
  const lv = getLocalVideo();
  if (!lv.tools || !lv.tools.available || !lv.tools.needsRemux(fullPath)) return null;
  await lv.ready;
  const rel = encodeURIComponent(path.relative(MEDIA_DIR, fullPath));
  const base = `http://127.0.0.1:${lv.port}`;
  const info = lv.tools.probe(fullPath) || {};
  const pick = lv.tools.pickTracks(info);
  return {
    stream: `${base}/play?f=${rel}`,
    remux: true,
    duration: info.duration || null,
    audio: (info.audio || []).map((t) => ({ lang: t.lang, title: t.title, codec: t.codec, channels: t.channels })),
    subs: (info.subs || []).map((t) => ({ n: t.n, lang: t.lang, title: t.title, forced: t.forced })),
    a: pick.a,
    s: pick.s,
    sub: `${base}/sub?f=${rel}&n={n}`,
    seek: `${base}/seek?f=${rel}`,
    hevc: Boolean(info.video && info.video.codec === 'hevc')
  };
}

async function openMediaFile(fullPath, relativePath, resumeSeconds) {
  const isRemote = /^https?:\/\//i.test(fullPath);
  // A film from the media server resumes where the server says, unless this
  // launcher has a position of its own.
  // A negative position means "from the start" (film page: Von vorne).
  const resume = resumeSeconds < 0 ? 0 : (resumeSeconds || (isRemote && remotePositions.get(relativePath)) || 0);
  const play = isRemote ? remotePlayInfo.get(relativePath) : await localPlayInfo(fullPath);
  const params = new URLSearchParams({
    src: play && play.stream ? play.stream : (isRemote ? fullPath : 'file://' + fullPath),
    rel: relativePath || '',
    resume: String(resume)
  });
  if (play) params.set('play', JSON.stringify(play));
  openContentUrl(`file://${path.join(__dirname, 'mediaplayer.html')}?${params.toString()}`);
}

ipcMain.handle('open-url', (event, url, opts) => openContentUrl(url, opts));
ipcMain.handle('open-livetv', (event, streamUrl) => openLiveTvChannel(streamUrl));
ipcMain.handle('livetv-playlist', () => readLiveTvPlaylist());
ipcMain.handle('livetv-logos', () => readLiveTvPlaylist().map((c) => c.logo).filter(Boolean));
ipcMain.on('livetv-exit', () => {
  closeContentView();
  if (win && !win.isDestroyed()) win.webContents.send('content-closed');
});
ipcMain.handle('list-media-dir', (event, relativeDir) => listMediaDir(relativeDir || ''));
ipcMain.handle('list-library', () => listLibrary());
ipcMain.handle('get-random-movie', async () => {
  const items = await listLibrary();
  if (items.length === 0) return null;
  return items[Math.floor(Math.random() * items.length)];
});
ipcMain.handle('get-weather-video', (event, condition) => getWeatherVideo(condition));
ipcMain.handle('get-news', () => getNews());
ipcMain.handle('get-wifi-status', () => getWifiStatus());
ipcMain.handle('get-wifi-networks', () => getWifiNetworks());
ipcMain.handle('connect-wifi', async (event, ssid) => {
  const result = await connectWifi(ssid);
  tvos.onWifiConnect(ssid, result); // toast once the connection has actually settled
  return result;
});
ipcMain.handle('connect-wifi-with-password', async (event, ssid, password) => {
  const result = await connectWifiWithPassword(ssid, password);
  tvos.onWifiConnect(ssid, result);
  return result;
});
ipcMain.handle('get-bluetooth-status', () => getBluetoothStatus());
ipcMain.handle('open-bluetooth-settings', () => openBluetoothSettings());
ipcMain.handle('scan-bluetooth-devices', async (event, seconds) => {
  const result = await scanBluetoothDevices(seconds);
  tvos.onBluetoothScan(result); // remembers device names for the "gekoppelt" toast
  return result;
});
ipcMain.handle('pair-bluetooth-device', async (event, deviceId) => {
  const result = await pairBluetoothDevice(deviceId);
  tvos.onBluetoothPair(deviceId, result);
  return result;
});
ipcMain.handle('open-media', (event, fullPath, relativePath, resumeSeconds) =>
  openMediaFile(fullPath, relativePath, resumeSeconds)
);
ipcMain.on('media-progress', (event, relativePath, seconds) => {
  win.webContents.send('media-progress-update', relativePath, seconds);
  if (String(relativePath).startsWith('server:')) reportRemoteProgress(relativePath, seconds);
});
ipcMain.handle('get-media-server', async () => {
  const url = getMediaServerUrl();
  return url ? pingMediaServer(url) : { ok: false, url: '' };
});
ipcMain.handle('set-media-server', (event, url) => setMediaServerUrl(url));
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
// --- Einstellungen > Bild. The launcher (picture.js) sends the finished
// filter; every page the content view loads fetches it as it starts
// (content-preload.js, 'tv-picture-get') and gets changes pushed live
// ('tv-picture'). It ends up in the CSS and DOM of web pages, so only the
// exact shape picture.js builds gets through: known filter functions, plain
// numbers and the fixed filter id. null = Standard, no filter.
let picture = null;
const PICTURE_FILTER_RE = /^(?:(?:brightness|contrast|saturate)\(\d+(?:\.\d+)?\) ?){0,3}(?:url\(#__tv_picture__\))?$/;
const isNumberList = (s, count) => s === '' ||
  (typeof s === 'string' && s.split(' ').length === count && s.split(' ').every((n) => /^-?\d+(\.\d+)?$/.test(n)));
function sanitizePicture(p) {
  if (!p || typeof p !== 'object') return null;
  const filter = String(p.filter || '').trim();
  const matrix = p.matrix || '';
  const kernel = p.kernel || '';
  if (!filter || filter.length > 200 || !PICTURE_FILTER_RE.test(filter)) return null;
  if (!isNumberList(matrix, 20) || !isNumberList(kernel, 9)) return null;
  return { filter, matrix, kernel };
}
ipcMain.handle('set-picture', (event, payload) => {
  if (!win || event.sender !== win.webContents) return;
  picture = sanitizePicture(payload);
  if (contentView && !contentView.webContents.isDestroyed()) contentView.webContents.send('tv-picture', picture);
});
ipcMain.on('tv-picture-get', (event) => { event.returnValue = picture; });

ipcMain.handle('get-volume', () => runVolHelper(['get']));
ipcMain.handle('set-volume', (event, level) => runVolHelper(['set', String(Math.round(Number(level) || 0))]));
ipcMain.handle('set-mute', (event, muted) => runVolHelper(['mute', muted ? '1' : '0']));
ipcMain.handle('toggle-fullscreen', () => {
  win.setFullScreen(!win.isFullScreen());
});

// Handy-Fernbedienung (see remote-server.js). Messages go to the renderer
// with send() rather than invoke() -- fire-and-forget, no round-trip, since
// latency is the whole point of a pointer.
let remote = null;
ipcMain.handle('get-remote-info', () => ({ url: remote ? remote.url : null }));
// Pointer landed on something new -> a tiny vibration on the phone.
ipcMain.on('remote-feedback', () => { if (remote && remote.send) remote.send({ t: 'h' }); });
// What the music player plays, for the transport strip on the phone. The
// last state is sent again whenever a phone connects.
let remoteNowPlaying = null;
ipcMain.on('remote-now-playing', (event, np) => {
  remoteNowPlaying = np && np.title ? { t: 'np', title: String(np.title), artist: String(np.artist || ''), playing: Boolean(np.playing) } : { t: 'np' };
  if (remote && remote.send) remote.send(remoteNowPlaying);
});

// Read-only facts for Settings > System ("Über dieses Gerät"). `os` is
// required inside the handler on purpose: it keeps this block self-contained
// (no top-level name that could clash with another addition to this file).
// The IP is picked with the same rule as remote-server.js getLanIp() -- first
// non-internal IPv4 that isn't link-local -- so the address shown here is the
// one the phone remote uses.
ipcMain.handle('get-system-info', () => {
  const os = require('os');
  const ips = [];
  for (const [name, addrs] of Object.entries(os.networkInterfaces())) {
    for (const a of addrs || []) {
      const isV4 = a.family === 'IPv4' || a.family === 4;
      if (isV4 && !a.internal && !a.address.startsWith('169.254.')) ips.push({ name, address: a.address });
    }
  }
  // The display the launcher window is on (falls back to primary before the
  // window exists). size is in DIPs -- physical pixels = size * scaleFactor.
  let display = screen.getPrimaryDisplay();
  try {
    if (win && !win.isDestroyed()) display = screen.getDisplayMatching(win.getBounds());
  } catch { /* keep primary */ }
  const cpus = os.cpus();
  return {
    version: app.getVersion(),
    electron: process.versions.electron,
    chrome: process.versions.chrome,
    node: process.versions.node,
    hostname: os.hostname(),
    arch: os.arch(),
    ip: ips.length ? ips[0].address : null,
    ips,
    uptimeSec: os.uptime(),
    memTotal: os.totalmem(),
    memFree: os.freemem(),
    cpu: cpus && cpus.length ? cpus[0].model.replace(/\s+/g, ' ').trim() : null,
    display: {
      width: display.size.width,
      height: display.size.height,
      scaleFactor: display.scaleFactor,
      primary: display.id === screen.getPrimaryDisplay().id
    }
  };
});

// While a content view (YouTube, Live TV, a video) is open it sits on top
// of the renderer, so the pointer would be moving around invisibly
// underneath it. There the remote acts as a D-pad instead, Apple-TV style:
// touchpad swipes become arrow keys, a tap is Enter, Zurück closes the view
// -- which is also exactly how those TV-style pages expect to be driven.
const CONTENT_KEYCODES = { Escape: 'Escape', Enter: 'Enter', ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right' };
const SWIPE_STEP = 70; // phone px of (accelerated) travel per arrow press
let swipeAccX = 0;
let swipeAccY = 0;
let lastSwipeAt = 0;

function sendContentKey(keyCode) {
  const wc = contentView.webContents;
  wc.sendInputEvent({ type: 'keyDown', keyCode });
  wc.sendInputEvent({ type: 'keyUp', keyCode });
}

function handleRemoteMessage(msg) {
  if (!win || win.isDestroyed()) return;
  sleepTimer.onRemoteMessage(msg);
  // Standby wake-up (swallows the first input) and the Home button.
  if (tvos.interceptRemote(msg)) return;
  // The Control Center lives in the launcher window, so it only opens over
  // the home screen, not on top of a running web app.
  if (msg.t === 'cc') {
    if (!contentView) win.webContents.send('remote-msg', msg);
    return;
  }
  // The phone's music buttons always steer the launcher's music player.
  if (msg.t === 'k' && /^Media/.test(msg.k)) {
    win.webContents.send('remote-msg', msg);
    return;
  }
  if (!contentView) {
    win.webContents.send('remote-msg', msg);
    return;
  }
  // While a web app is still loading (or failed to) the renderer owns the
  // screen -- splash or error page -- so D-pad and pointer go there. Escape
  // still closes the app below.
  if (tvos.isContentHidden() && !(msg.t === 'k' && msg.k === 'Escape')) {
    win.webContents.send('remote-msg', msg);
    return;
  }
  if (msg.t === 'k' && msg.k === 'Escape') {
    if (contentHandlesBack()) { contentView.webContents.send('tv-back'); return; }
    closeContentView();
    win.webContents.send('content-closed');
  } else if (msg.t === 'k') {
    sendContentKey(CONTENT_KEYCODES[msg.k]);
  } else if (msg.t === 'c') {
    sendContentKey('Enter');
  } else if (msg.t === 'm') {
    const now = Date.now();
    // A pause means a new swipe: don't let leftovers of the last one tip
    // the next into an extra step.
    if (now - lastSwipeAt > 250) { swipeAccX = 0; swipeAccY = 0; }
    lastSwipeAt = now;
    swipeAccX += msg.x;
    swipeAccY += msg.y;
    if (Math.abs(swipeAccX) >= SWIPE_STEP || Math.abs(swipeAccY) >= SWIPE_STEP) {
      if (Math.abs(swipeAccX) > Math.abs(swipeAccY)) sendContentKey(swipeAccX > 0 ? 'Right' : 'Left');
      else sendContentKey(swipeAccY > 0 ? 'Down' : 'Up');
      swipeAccX = 0;
      swipeAccY = 0;
    }
  } else if (msg.t === 's') {
    // Phone scroll wheel: a real mouse-wheel event in the middle of the web
    // app, so lists and pages scroll like with a mouse.
    const b = contentView.getBounds();
    contentView.webContents.sendInputEvent({
      type: 'mouseWheel', x: Math.round(b.width / 2), y: Math.round(b.height / 2),
      deltaX: 0, deltaY: -msg.y * 2.5, canScroll: true
    });
  }
  // Gyro deltas and recenter have no D-pad meaning -- ignored here.
}

// No menu bar at all -- autoHideMenuBar still lets a stray Alt press reveal
// Electron's default File/Edit/View menu, which looks nothing like a TV UI.
Menu.setApplicationMenu(null);

app.whenReady().then(async () => {
  music.init({ dataDir: DATA_DIR, getMediaServerUrl });
  tvos.init({
    getWin: () => win,
    getContentView: () => contentView,
    closeContentView,
    fitContentView,
    getWifiStatus
  });
  sleepTimer.init({
    getWin: () => win,
    getContentView: () => contentView,
    closeContentView,
    notify: tvos.notify
  });
  memory.init({ getWin: () => win, getContentView: () => contentView });
  // Deliberately NOT awaited: a TV boots before its WLAN is up. Awaiting three
  // filter lists here used to delay the whole UI by seconds on a slow link and,
  // offline, prevented the window from ever being created (the rejection ended
  // this callback before createWindow). Ad blocking simply switches on once the
  // lists could be fetched.
  const startAdBlocking = () => setupAdBlocking().catch((err) => {
    console.error('Ad blocking not available yet, retrying in 60 s:', err && err.message);
    setTimeout(startAdBlocking, 60 * 1000);
  });
  startAdBlocking();
  appliance.init(); // first: priority boost before the window loads
  ensureArtModeVideoReady();
  createWindow();

  remote = startRemoteServer(app.getPath('userData'));
  remote.on('message', handleRemoteMessage);
  remote.on('client-connected', () => {
    tvos.onRemoteClient();
    if (remoteNowPlaying) setTimeout(() => remote.send(remoteNowPlaying), 300);
  });

  // Dev/debug escape hatch: leave fullscreen without needing the remote.
  globalShortcut.register('F11', () => {
    win.setFullScreen(!win.isFullScreen());
  });
});

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit();
});

// An uncaught exception in the main process would otherwise crash straight
// to the Windows desktop with no recovery -- relaunch instead, same as a
// renderer crash, so the appliance comes back up on its own.
process.on('uncaughtException', (err) => {
  console.error('Uncaught exception in main process, relaunching:', err);
  app.relaunch();
  app.exit(1);
});
