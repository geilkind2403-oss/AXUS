// --- Appliance mode: the laptop behaves like a TV.
//   - Starts with Windows (login item), and starts the watchdog itself.
//   - Watchdog (watchdog.ps1): brings the launcher back if the process ever
//     dies completely. Crashes inside the app are already handled by
//     app.relaunch() in main.js; this covers kills, OOM and hard crashes.
//   - Only one launcher at a time: a second start just shows the running one.
//   - Quitting on purpose (menu "Beenden", Alt+F4) leaves quit.flag, so the
//     watchdog stops instead of starting it again.
// Off in dev mode (TV_LAUNCHER_DEV=1), so development restarts stay manual.
const { app } = require('electron');
const path = require('path');
const fs = require('fs');
const os = require('os');
const { spawn } = require('child_process');

const DEV = process.env.TV_LAUNCHER_DEV === '1';
const WSCRIPT = path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'wscript.exe');
const WATCHDOG_VBS = path.join(__dirname, 'watchdog.vbs');

const quitFlag = () => path.join(app.getPath('userData'), 'quit.flag');
const watchdogArgs = () => [WATCHDOG_VBS, process.execPath, app.getAppPath(), app.getPath('userData')];

// Returns false when another launcher already runs -- the caller must stop then.
function claimSingleInstance(getWin) {
  if (!app.requestSingleInstanceLock()) return false;
  app.on('second-instance', () => {
    const w = getWin();
    if (!w || w.isDestroyed()) return;
    if (w.isMinimized()) w.restore();
    w.show();
    w.focus();
  });
  return true;
}

// CPU priority for every launcher process (main, renderer, GPU, films):
// high while booting, so the launcher wins against the other login apps,
// then above normal, so a busy PC never makes the TV stutter. Children
// don't inherit it on Windows, so it's re-applied as new ones appear.
function boostPriority() {
  const { PRIORITY_HIGH: HIGH, PRIORITY_ABOVE_NORMAL: ABOVE_NORMAL } = os.constants.priority;
  const started = Date.now();
  const apply = () => {
    const level = Date.now() - started < 45000 ? HIGH : ABOVE_NORMAL;
    const pids = [process.pid, ...app.getAppMetrics().map((m) => m.pid)];
    for (const pid of new Set(pids)) {
      try { os.setPriority(pid, level); } catch { /* process just ended */ }
    }
  };
  apply();
  const fast = setInterval(apply, 3000);
  setTimeout(() => { clearInterval(fast); apply(); setInterval(apply, 30000).unref(); }, 46000);
}

function init() {
  if (DEV || process.platform !== 'win32') return;
  boostPriority();
  try { fs.rmSync(quitFlag(), { force: true }); } catch { /* not there */ }

  // Login item -> launcher, which starts the watchdog below. (A Run entry
  // pointing at wscript.exe was silently skipped by Explorer at login.)
  try {
    // Quoted: the Run entry is one command line, and paths may contain spaces.
    app.setLoginItemSettings({ openAtLogin: true, name: 'TV Launcher', path: process.execPath, args: [`"${app.getAppPath()}"`] });
  } catch (err) {
    console.error('Autostart konnte nicht eingetragen werden:', err && err.message);
  }

  // Started by hand: make sure the watchdog runs too (its mutex keeps it single).
  try {
    spawn(WSCRIPT, watchdogArgs(), { detached: true, stdio: 'ignore', windowsHide: true }).unref();
  } catch (err) {
    console.error('Watchdog konnte nicht gestartet werden:', err && err.message);
  }

  // Deliberate quit only: crash relaunches use app.exit(), which skips this.
  app.on('before-quit', () => {
    try { fs.writeFileSync(quitFlag(), String(Date.now())); } catch { /* watchdog restarts us: harmless */ }
  });
}

module.exports = { claimSingleInstance, init };
