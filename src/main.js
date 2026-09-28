const { app, BrowserWindow, WebContentsView, ipcMain, globalShortcut, screen, session } = require('electron');
const { ElectronBlocker } = require('@ghostery/adblocker-electron');
const { SpotifyAuth } = require('./spotify');
const fetch = require('cross-fetch');
const path = require('path');

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
      webPreferences: { contextIsolation: true }
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

ipcMain.handle('open-url', (event, url, opts) => openContentUrl(url, opts));
ipcMain.handle('open-livetv', (event, streamUrl) => openLiveTvChannel(streamUrl));
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
