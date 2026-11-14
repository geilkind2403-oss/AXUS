// --- Sleep-Timer (main process). Set from the Control Center: 30 / 60 / 90
// minutes. One minute before the end a toast warns "Bildschirm geht gleich
// aus"; any key or deliberate remote input in that minute cancels. At the end
// whatever plays is paused, the web app / film is closed (the player saves
// its position on pause, so the "Weiterschauen" tile picks it up again) and
// the launcher goes into standby.
//
// Lives in the main process because a running film has the keyboard focus in
// its own content view -- the launcher window never sees those keys.
const { app, ipcMain } = require('electron');

const WARN_BEFORE_MS = 60 * 1000;

let ctx = null;
let endsAt = 0;
let warned = false;
let warnTimer = null;
let fireTimer = null;

const state = () => ({ endsAt, minutes: endsAt ? Math.max(0, Math.ceil((endsAt - Date.now()) / 60000)) : 0 });

function broadcast() {
  const w = ctx.getWin();
  if (w && !w.isDestroyed()) w.webContents.send('sleep-timer-state', state());
}

function clear() {
  clearTimeout(warnTimer);
  clearTimeout(fireTimer);
  warnTimer = fireTimer = null;
  endsAt = 0;
  warned = false;
}

function set(minutes) {
  clear();
  const m = Number(minutes) || 0;
  if (m > 0) {
    endsAt = Date.now() + m * 60 * 1000;
    warnTimer = setTimeout(warn, Math.max(0, endsAt - Date.now() - WARN_BEFORE_MS));
    fireTimer = setTimeout(fire, endsAt - Date.now());
  }
  broadcast();
  return state();
}

function warn() {
  warned = true;
  ctx.notify({
    id: 'sleep', icon: 'power', kind: 'warning', force: true, duration: 15000,
    title: 'Bildschirm geht gleich aus', text: 'Sleep-Timer: noch 1 Minute. Beliebige Taste bricht ab.'
  });
}

function fire() {
  clear();
  const view = ctx.getContentView();
  const w = ctx.getWin();
  const finish = () => {
    if (ctx.getContentView()) {
      ctx.closeContentView();
      if (w && !w.isDestroyed()) w.webContents.send('content-closed');
    }
    if (w && !w.isDestroyed()) w.webContents.send('sleep-timer-fire');
    broadcast();
  };
  if (!view || view.webContents.isDestroyed()) { finish(); return; }
  // Pause first, so the player reports its position before the view goes away.
  view.webContents.executeJavaScript(
    "document.querySelectorAll('video, audio').forEach((m) => { try { m.pause(); } catch {} }); 0", true
  ).catch(() => {}).then(() => setTimeout(finish, 400));
}

// Any real input during the warning minute cancels the timer.
function onInput() {
  if (!warned) return;
  clear();
  broadcast();
  ctx.notify({ id: 'sleep', icon: 'check', kind: 'success', force: true, title: 'Sleep-Timer abgebrochen', duration: 3000 });
}

// Phone remote: only deliberate input counts (gyro noise must not cancel).
function onRemoteMessage(msg) {
  if (!msg) return;
  if (msg.t === 'c' || msg.t === 'k' || msg.t === 'cc' || (msg.t === 'm' && Math.abs(msg.x) + Math.abs(msg.y) > 12)) onInput();
}

function init(context) {
  ctx = context;
  ipcMain.handle('sleep-timer-set', (event, minutes) => set(minutes));
  // Keys in the launcher window and in every web app / film view.
  app.on('web-contents-created', (event, wc) => {
    wc.on('before-input-event', (e, input) => { if (input.type === 'keyDown') onInput(); });
  });
}

module.exports = { init, onInput, onRemoteMessage };
