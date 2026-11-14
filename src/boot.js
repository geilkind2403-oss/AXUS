// --- Boot preloading. While the boot splash is up, everything the menus will
// need is fetched and decoded, so afterwards every screen opens instantly
// instead of loading in front of the user. Films themselves are NOT
// preloaded (far too big) -- only their covers.
//
// Each task is independent and time-boxed: no internet or the media PC being
// off just skips that task, it never blocks the boot. The splash lifts when
// all tasks are done (at least ~2.4 s for the intro animation; loading may take
// up to 2 minutes -- the TV should be fully ready, not fast).
(() => {
  const MIN_MS = 2400;
  const MAX_MS = 120000;
  const TASK_TIMEOUT = 30000;
  const started = performance.now();

  // ---- progress UI under the wordmark
  const splash = document.getElementById('boot-splash');
  const ui = document.createElement('div');
  ui.id = 'boot-progress';
  ui.innerHTML = '<div class="bp-bar"><i></i></div><div class="bp-text">Wird gestartet …</div>';
  if (splash) splash.appendChild(ui);
  const bar = ui.querySelector('i');
  const text = ui.querySelector('.bp-text');

  const withTimeout = (p, ms = TASK_TIMEOUT) => Promise.race([p, new Promise((r) => setTimeout(r, ms))]);
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  // Everything preloaded stays referenced here, so Chromium keeps it decoded
  // in RAM and screens show it instantly. The launcher may use up to ~1 GB;
  // above that main.js asks for a trim ('tv-memory-trim') and this lets go.
  const warm = [];
  window.addEventListener('tv-memory-trim', () => { warm.length = 0; });

  // Loads + decodes images with limited parallelism, so 300 channel logos
  // don't open 300 connections at once.
  async function preloadImages(urls, parallel = 8) {
    const list = [...new Set(urls.filter(Boolean))];
    let i = 0;
    const worker = async () => {
      while (i < list.length) {
        const url = list[i++];
        await new Promise((resolve) => {
          const img = new Image();
          img.onload = () => { warm.push(img); (img.decode ? img.decode() : Promise.resolve()).catch(() => {}).then(resolve); };
          img.onerror = resolve;
          img.src = url;
        });
      }
    };
    await Promise.all(Array.from({ length: Math.min(parallel, list.length) }, worker));
  }
  function preloadVideo(src) {
    return new Promise((resolve) => {
      if (!src) { resolve(); return; }
      const v = document.createElement('video');
      v.muted = true;
      v.preload = 'auto';
      v.oncanplaythrough = () => { warm.push(v); resolve(); };
      v.onerror = resolve;
      v.src = src;
      setTimeout(resolve, 6000);
    });
  }

  const L = window.launcher;
  const tasks = [
    ['Schriften', () => document.fonts && document.fonts.ready],
    ['Wetter', async () => { if (window.wx && window.wx.homeSummary) await window.wx.homeSummary(); }],
    ['Regenradar', async () => { if (window.wx && window.wx.preloadRadar) await window.wx.preloadRadar(); }],
    ['Wetter-Videos', async () => {
      const [sunny, rainy] = await Promise.all([L.getWeatherVideo('sunny'), L.getWeatherVideo('rainy')]);
      await Promise.all([preloadVideo(sunny), preloadVideo(rainy)]);
    }],
    ['Nachrichten', async () => {
      const news = await L.getNews();
      await preloadImages((news || []).map((n) => n && n.image));
    }],
    ['Mediathek', async () => {
      // library.js keeps the list, so the Mediathek opens without waiting.
      const films = window.library && window.library.prefetch ? await window.library.prefetch() : await L.listLibrary();
      await preloadImages((films || []).flatMap((f) => [f.icon, f.banner]), 4);
    }],
    ['Musik', async () => {
      // music.js keeps the library; covers and artist photos land in the cache.
      if (!window.music || !window.music.prefetch) return;
      await preloadImages(await window.music.prefetch(), 6);
    }],
    ['App-Symbole', () => preloadImages([...document.querySelectorAll('img[src]')].map((i) => i.src))],
    ['Einstellungen', async () => {
      // Status the Control Center and Settings show first thing.
      await Promise.all([
        L.getWifiStatus && L.getWifiStatus(),
        L.getBluetoothStatus && L.getBluetoothStatus(),
        L.getVolume && L.getVolume(),
        L.getLiveTvPlaylist && L.getLiveTvPlaylist()
      ].map((p) => Promise.resolve(p).catch(() => {})));
    }],
    ['Live TV', async () => {
      // The Live TV app runs in its own view but shares this session's HTTP
      // cache, so the logos it shows later come straight from disk.
      if (!L.getLiveTvLogos) return;
      await preloadImages(await L.getLiveTvLogos(), 8);
    }],
    ['Animationen', async () => {
      // Every screen rendered once, invisibly behind the splash, and every
      // @keyframes run once: styles, layout, GPU layers and decoded images
      // are ready, so even the first open of a menu animates smoothly.
      const frames = async (n) => { for (let i = 0; i < n; i++) await new Promise((r) => requestAnimationFrame(() => r())); };
      const shown = [];
      const warmOpen = (id, cls) => {
        const el = document.getElementById(id);
        if (!el || el.classList.contains(cls)) return;
        el.style.visibility = 'hidden';
        el.style.animation = 'none';
        el.classList.add(cls);
        shown.push([el, cls]);
      };
      ['appfolder-overlay', 'weather-overlay', 'settings-overlay', 'library-overlay', 'library-detail-overlay', 'mediathek-overlay', 'music-overlay']
        .forEach((id) => warmOpen(id, 'active'));
      warmOpen('cc-overlay', 'on');

      const names = new Set();
      for (const sheet of document.styleSheets) {
        let rules = [];
        try { rules = sheet.cssRules; } catch { continue; } // cross-origin (fonts)
        for (const r of rules) if (r.type === CSSRule.KEYFRAMES_RULE) names.add(r.name);
      }
      const host = document.createElement('div');
      host.style.cssText = 'position:fixed;left:0;top:0;width:40px;height:40px;visibility:hidden;pointer-events:none;';
      names.forEach((n) => {
        const d = document.createElement('div');
        d.style.cssText = `position:absolute;inset:0;animation:${n} 50ms linear;`;
        host.appendChild(d);
      });
      document.body.appendChild(host);

      await frames(4);
      host.remove();
      shown.forEach(([el, cls]) => { el.classList.remove(cls); el.style.visibility = ''; el.style.animation = ''; });
    }],
    ['System', async () => { if (L.getSystemInfo) await L.getSystemInfo(); }]
  ];

  let done = 0;
  const running = new Set();
  function report() {
    const pct = Math.round((done / tasks.length) * 100);
    bar.style.transform = `scaleX(${Math.max(0.04, done / tasks.length)})`;
    const next = [...running][0];
    text.textContent = next ? `Lade ${next} …` : `Fertig · ${pct} %`;
  }

  const all = Promise.all(tasks.map(async ([label, fn]) => {
    running.add(label);
    report();
    try { await withTimeout(Promise.resolve().then(fn)); } catch { /* skipped, never blocks the boot */ }
    running.delete(label);
    done += 1;
    report();
  }));

  // Dress rehearsal: with everything loaded, every menu is really opened and
  // closed once behind the splash -- the same functions, animations, image
  // decodes, GPU layers and JIT-compiled code paths as when the user opens
  // it. Afterwards nothing is being done for the first time, so nothing
  // stutters. Each step is guarded; a failing one is simply skipped.
  async function rehearse() {
    const g = (fn) => { try { return fn(); } catch { return undefined; } };
    // Whatever a step's Zurück didn't close (a sub-page needs two), closed here.
    const cleanup = async () => {
      if (g(() => window.controlCenter.isOpen())) g(() => window.controlCenter.close());
      if (g(() => window.music.isOpen())) g(() => window.music.close());
      if (g(() => libraryDetailOpen)) g(() => closeLibraryDetail());
      if (g(() => libraryOpen)) g(() => closeLibrary());
      if (g(() => mediathekOpen)) g(() => closeMediathek());
      if (g(() => settingsOpen)) g(() => closeSettings());
      if (g(() => window.wx.isOpen())) g(() => window.wx.close());
      if (g(() => window.appFolder.isOpen())) g(() => window.appFolder.close());
      await sleep(300);
    };
    // Opened the way the user opens them: focus + OK / Zurück through the
    // real key router, so the zoom out of the tile or button runs too.
    const press = (k) => document.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }));
    const via = (id) => g(() => setFocusEl(document.getElementById(id)));
    const steps = [
      ['App-Ordner', async () => { via('grid-1'); press('Enter'); await sleep(1000); press('ArrowRight'); await sleep(300); press('Escape'); await sleep(800); }],
      ['Wetter', async () => { via('grid-5'); press('Enter'); await sleep(1600); press('ArrowDown'); await sleep(500); press('Escape'); await sleep(800); }],
      ['Einstellungen', async () => { via('btn-settings'); press('Enter'); await sleep(1200); press('ArrowDown'); await sleep(400); press('ArrowDown'); await sleep(400); press('Escape'); await sleep(700); }],
      ['WLAN', async () => { via('btn-wifi'); press('Enter'); await sleep(1100); press('Escape'); await sleep(700); }],
      ['Mediathek', async () => {
        g(() => openAction('mediathek'));
        await sleep(1200);
        press('ArrowRight'); await sleep(500);
        press('ArrowDown'); await sleep(500);
        press('Enter'); await sleep(1200);
        press('ArrowDown'); await sleep(600);
        press('ArrowRight'); await sleep(400);
        press('ArrowUp'); await sleep(400);
        press('Escape'); await sleep(500);
        press('Escape'); await sleep(700);
      }],
      ['Musik', async () => {
        // Its own walk-through (music.js): every screen once, nothing starts playing.
        if (window.music && window.music.rehearse) await window.music.rehearse(sleep);
      }],
      ['Ordner', async () => { await g(() => openMediathek()); await sleep(800); press('Escape'); await sleep(500); }],
      ['Kontrollzentrum', async () => { g(() => window.controlCenter.open()); await sleep(900); press('ArrowDown'); await sleep(300); press('Escape'); await sleep(600); }],
      ['Startseite', async () => {
        // Focus lift, tilt and sheen on every tile and top-bar entry.
        for (const el of document.querySelectorAll('#shortcut-grid .grid-cell, #topbar #weather, #topbar-nav .nav-item, #bottombar .pill-btn')) {
          g(() => setFocusEl(el));
          await sleep(140);
        }
      }]
    ];
    const home = g(() => focusedEl);
    // Twice: the first round compiles and rasterizes, the second one runs
    // everything again from warm caches, like every later open will.
    for (let round = 1; round <= 2; round++) {
      for (const [label, fn] of steps) {
        text.textContent = `Bereite vor (${round}/2): ${label} …`;
        try { await withTimeout(fn(), 15000); } catch { /* skipped */ }
        await cleanup();
      }
    }
    g(() => setFocusEl(home || document.getElementById('nav-home')));
    await sleep(300);
  }

  Promise.race([all, sleep(MAX_MS)]).then(async () => {
    await rehearse();
    const left = MIN_MS - (performance.now() - started);
    if (left > 0) await sleep(left);
    text.textContent = 'Bereit';
    await sleep(250);
    window.dispatchEvent(new Event('tv-boot-done'));
  });
})();
