// --- RAM budget. The launcher is allowed to use RAM freely (boot.js keeps
// everything it preloads decoded in memory so every screen opens instantly),
// up to a budget of 1 GB for all its processes together. Above that it cleans
// up after itself: the renderers drop their preload references and Blink's
// memory caches (preload.js / content-preload.js, 'memory-trim'), and the V8
// heaps get collected. Checked every 20 s; after a trim it waits 2 minutes
// before trimming again, so a film that simply needs the RAM isn't nagged.
const { app } = require('electron');

const BUDGET_MB = Number(process.env.TV_RAM_BUDGET_MB) || 1024; // env override for testing
const CHECK_MS = 20 * 1000;
const COOLDOWN_MS = 2 * 60 * 1000;
// Booting peaks well above the settled level (images decoding, videos buffering,
// ~1.3 GB for a few seconds, then ~770 MB): no trim while that settles, or it
// would throw away exactly what the boot just preloaded.
const BOOT_GRACE_MS = 90 * 1000;
const started = Date.now();

let ctx = null;
let lastTrim = 0;

// workingSetSize is in KB.
const usedMb = () => app.getAppMetrics().reduce((sum, m) => sum + ((m.memory && m.memory.workingSetSize) || 0), 0) / 1024;

function trim(reason) {
  lastTrim = Date.now();
  const targets = [ctx.getWin() && ctx.getWin().webContents, ctx.getContentView() && ctx.getContentView().webContents];
  for (const wc of targets) {
    if (!wc || wc.isDestroyed()) continue;
    wc.send('memory-trim');
  }
  console.log(`RAM ${reason}: aufgeräumt`);
}

function check() {
  const mb = usedMb();
  if (Date.now() - started < BOOT_GRACE_MS) return;
  if (mb > BUDGET_MB && Date.now() - lastTrim > COOLDOWN_MS) trim(`${Math.round(mb)} MB > ${BUDGET_MB} MB`);
}

function init(context) {
  ctx = context;
  setInterval(check, CHECK_MS).unref();
}

module.exports = { init, usedMb, trim };
