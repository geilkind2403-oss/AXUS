// --- TV-OS layer, main-process half (the renderer half is tvos.js).
// Everything here exists to remove "this is a PC running an app" tells that
// only the main process can fix, and to give the renderer the system events a
// real TV OS has (network status, notifications, standby, home).
//
//   - Web content (YouTube, Netflix, ...) is hardened: no popups/new windows,
//     no permission prompts, no download dialogs, no "Leave site?" prompt, no
//     DevTools/zoom/reload shortcuts, no navigation to non-web schemes.
//   - Web apps load OFF-SCREEN while the renderer shows a brand splash, and a
//     failed load becomes a friendly TV error screen instead of Chrome's page.
//   - Network status is probed (Windows' own NCSI endpoint) and pushed out.
//   - The display never sleeps by itself (powerSaveBlocker) except in standby.
//   - Alt+F4 opens the power menu instead of killing the "OS".
//
// main.js owns the window / content view and hands them in via init().
const { app, ipcMain, session, powerSaveBlocker, net } = require('electron');

// TV_LAUNCHER_DEV=1 keeps F12 (DevTools), F5 (reload) and zoom usable while
// developing; in normal use none of the browser shortcuts exist.
const DEV = process.env.TV_LAUNCHER_DEV === '1';

const HOME_BG = '#0a0a0c'; // same as the window/boot background, so no white flash
const REVEAL_DELAY_MS = 150;   // after did-finish-load: let the page paint its first frame
const LOAD_FAILSAFE_MS = 25000; // a page that never "finishes" is shown anyway after this
const NCSI_URLS = [
  // Windows' own connectivity check first (already talked to by every PC),
  // Google's 204 endpoint as second opinion so one blocked host can't fake "offline".
  { url: 'http://www.msftconnecttest.com/connecttest.txt', ok: (res, text) => res.ok && text.trim() === 'Microsoft Connect Test' },
  { url: 'http://connectivitycheck.gstatic.com/generate_204', ok: (res) => res.status === 204 }
];

// Web content only gets what a TV app needs. Everything else (notifications,
// geolocation, camera, ...) is refused silently: Electron would otherwise grant
// it automatically, and e.g. site notifications would pop up as Windows toasts.
const ALLOWED_PERMISSIONS = new Set(['fullscreen', 'mediaKeySystem', 'clipboard-sanitized-write', 'persistent-storage', 'background-sync']);

let ctx = null; // { getWin, getContentView, closeContentView, fitContentView, getWifiStatus }
let quitting = false;
let standby = false;
let standbySince = 0;
let keepAwake = true;
let awakeId = null;

// State of the web app currently in the content view (null when none is open).
// state: 'loading' (off-screen, renderer shows the splash) | 'ready' | 'error'
let content = null;

const netState = { online: true, known: false };
let probeTimer = null;
let probing = false;
let failStreak = 0;

const win = () => {
  const w = ctx && ctx.getWin();
  return w && !w.isDestroyed() ? w : null;
};
const sendToLauncher = (channel, payload) => {
  const w = win();
  if (w && !w.webContents.isDestroyed()) w.webContents.send(channel, payload);
};
const fromLauncher = (event) => {
  const w = win();
  return !!w && event.sender === w.webContents;
};

// ---------------------------------------------------------------- notifications

// Toast payload: { id?, icon, kind: 'info'|'success'|'warning'|'error', title, text?, duration? }.
// Goes to the launcher UI, and (best effort, drawn by content-preload.js) into
// the web app on screen too, since a web app covers the whole launcher window.
function notify(payload) {
  sendToLauncher('tvos-toast', payload);
  if (content && content.revealed && !content.wc.isDestroyed()) content.wc.send('tvos-toast', payload);
}

// ---------------------------------------------------------------- display awake

function applyKeepAwake() {
  const want = keepAwake && !standby;
  if (want && awakeId === null) awakeId = powerSaveBlocker.start('prevent-display-sleep');
  else if (!want && awakeId !== null) { powerSaveBlocker.stop(awakeId); awakeId = null; }
}

// ---------------------------------------------------------------- session

function setupSession() {
  const ses = session.defaultSession;
  ses.setPermissionRequestHandler((wc, permission, callback) => callback(ALLOWED_PERMISSIONS.has(permission)));
  ses.setPermissionCheckHandler((wc, permission) => ALLOWED_PERMISSIONS.has(permission));
  // A download would open a native "Save as" dialog. This is a TV, not a PC.
  ses.on('will-download', (event) => event.preventDefault());
  try { ses.setSpellCheckerEnabled(false); } catch { /* not supported: harmless */ }

  app.on('web-contents-created', (event, wc) => {
    wc.on('will-attach-webview', (e) => e.preventDefault());
    // Default for every page: no extra windows. attachContent() upgrades this
    // for the content view (links that want a new tab just open in place).
    wc.setWindowOpenHandler(() => ({ action: 'deny' }));
  });
}

// ---------------------------------------------------------------- input hardening

// Applied to the launcher window and to every content view.
function hardenWebContents(wc) {
  try { Promise.resolve(wc.setVisualZoomLevelLimits(1, 1)).catch(() => {}); } catch { /* destroyed */ } // no pinch zoom
  wc.on('will-prevent-unload', (event) => event.preventDefault()); // never a "Leave site?" dialog
  wc.on('devtools-opened', () => { if (!DEV) wc.closeDevTools(); });

  wc.on('before-input-event', (event, input) => {
    if (input.type !== 'keyDown') return;
    const key = input.key;
    if (DEV) {
      if (key === 'F12') { event.preventDefault(); wc.toggleDevTools(); }
      return;
    }
    const ctrl = input.control || input.meta;
    const block =
      key === 'F12' || key === 'F5' || key === 'F7' ||                    // DevTools, reload, caret browsing
      (ctrl && input.shift && /^[ijc]$/i.test(key)) ||                    // DevTools
      (ctrl && /^[rupsog]$/i.test(key)) ||                                // reload, view-source, print, save, open, find-next
      (ctrl && (key === '+' || key === '-' || key === '=' || key === '_' || key === '0')); // zoom
    if (block) event.preventDefault();
  });
}

function refocus() {
  const w = win();
  if (!w) return;
  const wc = content && content.revealed && !content.wc.isDestroyed() ? content.wc : w.webContents;
  if (!wc.isDestroyed()) wc.focus();
}

function goHome() {
  const w = win();
  if (!w) return;
  if (ctx.getContentView()) {
    ctx.closeContentView();
    w.webContents.send('content-closed');
  }
  w.webContents.send('tvos-home');
}

function showPowerMenu() {
  const w = win();
  if (!w) return;
  if (ctx.getContentView()) {
    ctx.closeContentView();
    w.webContents.send('content-closed');
  }
  if (w.isMinimized()) w.restore();
  w.webContents.send('tvos-power-menu');
}

function attachWindow(w) {
  hardenWebContents(w.webContents);
  // A TV has no window title that changes with the page.
  w.on('page-title-updated', (e) => e.preventDefault());
  // The launcher never navigates: dropping a file on it would otherwise load
  // that file as the page, replacing the whole UI.
  w.webContents.on('will-navigate', (e) => e.preventDefault());
  // Alt+F4 / a stray close request: an appliance can't be "closed" -- Windows
  // showing up behind it is the biggest illusion-breaker there is. The power
  // menu (screen off / restart / quit) takes its place. Real quit paths
  // (app.quit, Windows shutdown) set `quitting` first and pass through.
  w.on('close', (e) => {
    if (quitting) return;
    e.preventDefault();
    showPowerMenu();
  });
  w.on('session-end', () => { quitting = true; }); // Windows shutdown/logoff must never be blocked
  // After a Windows notification or another window stole focus and gave it
  // back, keyboard input must land in whatever is on screen right now.
  w.on('focus', refocus);
  // First network probe once the UI is up, so an "offline" toast isn't sent into the void.
  w.webContents.once('did-finish-load', () => scheduleProbe(3500));
}

// ---------------------------------------------------------------- web content view

function clearContentTimers() {
  if (!content) return;
  clearTimeout(content.revealTimer);
  clearTimeout(content.failsafeTimer);
  content.revealTimer = null;
  content.failsafeTimer = null;
}

function sendContentState(payload) {
  sendToLauncher('tvos-content-state', payload);
}

// Show the page: it was loading off-screen behind the renderer's splash.
function reveal() {
  if (!content || content.revealed || content.state === 'error') return;
  clearContentTimers();
  content.state = 'ready';
  content.revealed = true;
  ctx.fitContentView();
  if (!content.wc.isDestroyed()) content.wc.focus();
  sendContentState({ state: 'ready' });
}

function showLoadError(info) {
  if (!content) return;
  clearContentTimers();
  content.state = 'error';
  content.revealed = false; // back off-screen: Chrome's own error page must never be seen
  content.failedUrl = info.url || content.url;
  ctx.fitContentView();
  const w = win();
  if (w) w.webContents.focus(); // the renderer's error screen takes the keys
  sendContentState({ state: 'error', url: content.failedUrl, code: info.code, desc: info.desc, online: netState.online });
}

function beginContentLoad(url) {
  if (!content) return;
  clearContentTimers();
  content.url = url;
  content.failedUrl = '';
  content.state = 'loading';
  content.revealed = false;
  ctx.fitContentView(); // off-screen while loading
  const w = win();
  if (w) w.webContents.focus(); // the renderer shows the splash and owns the keys until reveal
  sendContentState({ state: 'loading', url });
  content.failsafeTimer = setTimeout(reveal, LOAD_FAILSAFE_MS);
}

function retryContent() {
  if (!content || content.wc.isDestroyed()) return;
  const url = content.failedUrl || content.url;
  if (!url) return;
  beginContentLoad(url);
  content.wc.loadURL(url).catch(() => { /* did-fail-load reports it */ });
}

// Web pages may only navigate to web pages (and our own two player pages):
// mailto:/ms-settings:/file: links or a dropped file would otherwise hand the
// user a Windows "open with" prompt or replace the app with a bare file.
function isAllowedNavigation(url) {
  if (/^(https?|blob|data|about):/i.test(url)) return true;
  if (/^file:/i.test(url)) return /\/(livetv|mediaplayer)\.html(\?|#|$)/i.test(url);
  return false;
}

function attachContent(view) {
  const wc = view.webContents;
  view.setBackgroundColor(HOME_BG); // never a white flash before the first paint
  hardenWebContents(wc);
  content = { view, wc, url: '', failedUrl: '', state: 'loading', revealed: false, revealTimer: null, failsafeTimer: null, editing: false };

  // window.open / target=_blank: a TV has no tabs -- open the link in place.
  wc.setWindowOpenHandler(({ url }) => {
    if (/^https?:/i.test(url)) setImmediate(() => { if (!wc.isDestroyed()) wc.loadURL(url).catch(() => {}); });
    return { action: 'deny' };
  });
  wc.on('will-navigate', (event, url) => { if (!isAllowedNavigation(url)) event.preventDefault(); });

  wc.on('did-finish-load', () => {
    if (!content || content.wc !== wc || content.state !== 'loading') return;
    clearTimeout(content.revealTimer);
    content.revealTimer = setTimeout(reveal, REVEAL_DELAY_MS);
  });
  // Main-frame failures at any time (first load, or a link clicked inside the
  // app while the network is gone). -3 = aborted (redirect/cancel), not an error.
  wc.on('did-fail-load', (event, code, desc, validatedUrl, isMainFrame) => {
    if (!isMainFrame || code === -3 || !content || content.wc !== wc) return;
    showLoadError({ code, desc, url: validatedUrl });
    scheduleProbe(400);
  });
  wc.on('render-process-gone', (event, details) => {
    if (details.reason === 'clean-exit' || !content || content.wc !== wc) return;
    showLoadError({ code: 'crashed', desc: details.reason, url: content.url });
  });

  // Home key on a keyboard/remote while an app is open. Only when no text field
  // has focus (content-preload.js reports that), or Home would break typing.
  wc.on('before-input-event', (event, input) => {
    if (input.type !== 'keyDown') return;
    const plainHome = input.key === 'Home' && !input.control && !input.alt && !input.shift && !input.meta;
    if (input.key === 'BrowserHome' || (plainHome && !(content && content.editing))) {
      event.preventDefault();
      goHome();
    }
  });
}

function onContentClosed() {
  clearContentTimers();
  content = null;
}

// ---------------------------------------------------------------- network

async function probeOnce() {
  if (!net.isOnline()) return false; // no interface at all
  for (const { url, ok } of NCSI_URLS) {
    const ctl = new AbortController();
    const timer = setTimeout(() => ctl.abort(), 4000);
    try {
      const res = await net.fetch(url, { signal: ctl.signal, cache: 'no-store' });
      const text = await res.text();
      if (ok(res, text)) return true;
    } catch { /* try the next endpoint */ } finally { clearTimeout(timer); }
  }
  return false;
}

function scheduleProbe(ms) {
  clearTimeout(probeTimer);
  probeTimer = setTimeout(checkNetwork, ms);
}

// Never throws and always schedules the next probe: main.js turns any uncaught
// exception into a launcher restart, so a hiccup here must stay contained.
async function checkNetwork() {
  if (probing) return;
  probing = true;
  let next = 30000;
  try {
    const ok = await probeOnce();
    failStreak = ok ? 0 : failStreak + 1;
    // One failed probe alone isn't "offline" (a hiccup); two in a row, or no interface, is.
    const online = ok ? true : (failStreak >= 2 || !net.isOnline() ? false : netState.online);
    setNetwork(online);
    next = online ? (failStreak === 1 ? 3000 : 30000) : 5000;
  } catch (err) {
    console.error('Network check failed:', err && err.message);
  } finally {
    probing = false;
    scheduleProbe(next);
  }
}

function setNetwork(online) {
  const wasKnown = netState.known;
  const was = netState.online;
  netState.known = true;
  netState.online = online;
  if (wasKnown && online === was) return;
  sendToLauncher('tvos-network', { online });

  if (!online) {
    notify({ id: 'net', icon: 'wifi-off', kind: 'error', title: 'Kein Internet', text: 'Prüfe die WLAN-Verbindung.', duration: 7000 });
    return;
  }
  if (wasKnown && !was) {
    // Back online: say which network (Ethernet has no SSID), and retry a web app that failed meanwhile.
    Promise.resolve(ctx.getWifiStatus()).catch(() => null).then((s) => {
      const ssid = s && s.connected && s.ssid ? s.ssid : '';
      notify({ id: 'net', icon: 'wifi', kind: 'success', title: ssid ? 'WLAN verbunden' : 'Wieder online', text: ssid || 'Internetverbindung hergestellt' });
    }).catch(() => { /* a missing toast is not worth a crash */ });
    if (content && content.state === 'error') retryContent();
  }
}

// ---------------------------------------------------------------- hooks called from main.js

// A phone opened the remote page. Debounced: a phone that locks and wakes
// reconnects every few seconds, which must not spam the screen.
let lastPhoneToast = 0;
function onRemoteClient() {
  const now = Date.now();
  if (now - lastPhoneToast < 30000) return;
  lastPhoneToast = now;
  notify({ id: 'remote', icon: 'phone', kind: 'success', title: 'Handy-Fernbedienung verbunden', text: 'Wischen bewegt den Zeiger, Tippen wählt aus.' });
}

// Called for every message from the phone before main.js routes it.
// Returns true when handled here (standby wake-up swallows the first input, like a real remote).
function interceptRemote(msg) {
  // The phone's music buttons work with the screen off too (music in the
  // background is what "Bildschirm aus" is good for) and don't wake it.
  if (standby && msg.t === 'k' && /^Media/.test(msg.k)) return false;
  if (standby) {
    // Gyro noise, scrolling and recentering must not wake the screen; a click,
    // a key or a real touchpad swipe does. The first 700 ms are ignored: the
    // tap that chose "Bildschirm aus" is still settling.
    const deliberate = msg.t === 'c' || msg.t === 'k' || (msg.t === 'm' && Math.abs(msg.x) + Math.abs(msg.y) > 12);
    if (deliberate && Date.now() - standbySince > 700) {
      standby = false;
      applyKeepAwake();
      sendToLauncher('tvos-wake');
    }
    return true;
  }
  if (msg.t === 'k' && msg.k === 'Home') {
    goHome();
    return true;
  }
  return false;
}

// netsh's "connect" returns before the association is done: check a moment
// later what actually happened, then tell the user.
function onWifiConnect(ssid, result) {
  if (!result || !result.ok) return;
  setTimeout(async () => {
    try {
      let status = null;
      try { status = await ctx.getWifiStatus(); } catch { /* treated as failure below */ }
      if (status && status.connected && status.ssid === ssid) {
        notify({ id: 'net', icon: 'wifi', kind: 'success', title: 'WLAN verbunden', text: ssid });
      } else {
        notify({ id: 'net', icon: 'wifi-off', kind: 'warning', title: 'WLAN-Verbindung fehlgeschlagen', text: ssid });
      }
      scheduleProbe(500);
    } catch (err) {
      console.error('WLAN toast failed:', err && err.message);
    }
  }, 4500);
}

const bluetoothNames = new Map();
function onBluetoothScan(result) {
  for (const d of (result && result.devices) || []) bluetoothNames.set(d.id, d.name);
}
function onBluetoothPair(deviceId, result) {
  const name = bluetoothNames.get(deviceId) || '';
  if (result && result.ok) {
    notify({ icon: 'bluetooth', kind: 'success', title: 'Bluetooth-Gerät gekoppelt', text: name });
  } else {
    notify({ icon: 'bluetooth', kind: 'warning', title: 'Kopplung fehlgeschlagen', text: name });
  }
}

// ---------------------------------------------------------------- IPC

function registerIpc() {
  ipcMain.handle('tvos-get-network', () => ({ online: netState.online, known: netState.known }));
  ipcMain.handle('tvos-probe-network', () => scheduleProbe(300));
  ipcMain.handle('tvos-retry-content', (event) => { if (fromLauncher(event)) retryContent(); });
  ipcMain.handle('tvos-set-standby', (event, on) => {
    if (!fromLauncher(event)) return;
    standby = !!on;
    if (standby) standbySince = Date.now();
    applyKeepAwake();
  });
  ipcMain.handle('tvos-set-keep-awake', (event, on) => {
    if (!fromLauncher(event)) return;
    keepAwake = on !== false;
    applyKeepAwake();
  });
  ipcMain.handle('tvos-restart', (event) => {
    if (!fromLauncher(event)) return;
    quitting = true;
    app.relaunch();
    app.exit(0);
  });
  ipcMain.handle('tvos-quit', (event) => {
    if (!fromLauncher(event)) return;
    quitting = true;
    app.quit();
  });
  // content-preload.js: is a text field focused in the web app? (decides whether Home goes home)
  ipcMain.on('tvos-editing', (event, editing) => {
    if (content && event.sender === content.wc) content.editing = !!editing;
  });
}

function init(context) {
  ctx = context;
  app.on('before-quit', () => { quitting = true; });
  setupSession();
  registerIpc();
  applyKeepAwake();
}

module.exports = {
  init,
  attachWindow,
  attachContent,
  beginContentLoad,
  isContentHidden: () => !!content && !content.revealed,
  onContentClosed,
  interceptRemote,
  onRemoteClient,
  onWifiConnect,
  onBluetoothScan,
  onBluetoothPair,
  notify,
  goHome,
  showPowerMenu
};
