// TV-style on-screen display for the two player pages (mediaplayer.html and
// livetv.html). The browser's own <video controls> bar is a dead giveaway of
// "a web page in a window" -- a TV player has an OSD that appears on any key,
// fades out while playing and stays while paused.
//
// Keys (arrive from a keyboard, or from the phone remote's swipes/tap, which
// main.js turns into arrow keys / Enter while a page is open):
//   Enter / Space   play / pause          Left / Right   -10 s / +10 s (hold: +-30 s)
//   Up / Down       volume +-5 %          PageUp / Down  +-60 s
//   M               mute
// Escape / Home are handled by the launcher (main process), not here.
(() => {
  'use strict';

  const CSS = `
    html, body { cursor: default; }
    body.osd-idle, body.osd-idle * { cursor: none !important; }
    #osd { position: fixed; inset: 0; pointer-events: none; opacity: 0; color: #f1f1f3;
      font-family: 'Inter', 'Segoe UI', sans-serif; transition: opacity .35s cubic-bezier(.2, .8, .2, 1); }
    #osd.visible { opacity: 1; }
    .osd-top { position: absolute; left: 0; right: 0; top: 0; padding: 34px 56px 96px;
      background: linear-gradient(rgba(0, 0, 0, .68), rgba(0, 0, 0, 0));
      display: flex; align-items: center; gap: 16px;
      transform: translateY(-14px); transition: transform .4s cubic-bezier(.2, .8, .2, 1); }
    #osd.visible .osd-top, #osd.visible .osd-bottom { transform: none; }
    .osd-title { font-size: 26px; font-weight: 800; letter-spacing: -.2px; text-shadow: 0 2px 14px rgba(0, 0, 0, .55);
      white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .osd-live { display: none; padding: 4px 10px; border-radius: 6px; background: #e5484d; color: #fff;
      font-size: 12px; font-weight: 800; letter-spacing: 1px; }
    #osd.live .osd-live { display: block; }
    .osd-bottom { position: absolute; left: 0; right: 0; bottom: 0; padding: 96px 56px 42px;
      background: linear-gradient(rgba(0, 0, 0, 0), rgba(0, 0, 0, .78));
      transform: translateY(14px); transition: transform .4s cubic-bezier(.2, .8, .2, 1); }
    .osd-bar { position: relative; height: 6px; border-radius: 999px; background: rgba(255, 255, 255, .22); overflow: hidden; }
    .osd-buffered, .osd-played { position: absolute; inset: 0; border-radius: inherit; transform-origin: left center; transform: scaleX(0); }
    .osd-buffered { background: rgba(255, 255, 255, .3); }
    .osd-played { background: #3b6dff; box-shadow: 0 0 14px rgba(59, 109, 255, .8); transition: transform .3s linear; }
    .osd-times { display: flex; justify-content: space-between; margin-top: 12px; font-size: 15px; font-weight: 700;
      font-variant-numeric: tabular-nums; color: rgba(255, 255, 255, .85); }
    #osd.live .osd-bar, #osd.live .osd-times { display: none; }
    .osd-hit { position: fixed; left: 56px; right: 56px; bottom: 56px; height: 34px; pointer-events: auto; cursor: pointer; }
    #osd.live .osd-hit { display: none; }

    #osd-center { position: fixed; left: 50%; top: 50%; width: 112px; height: 112px; margin: -56px 0 0 -56px;
      border-radius: 50%; background: rgba(0, 0, 0, .55); -webkit-backdrop-filter: blur(6px); backdrop-filter: blur(6px);
      display: flex; align-items: center; justify-content: center; color: #fff; opacity: 0; pointer-events: none; }
    #osd-center.pop { animation: osdPop .75s cubic-bezier(.16, 1, .3, 1); }
    @keyframes osdPop { 0% { opacity: 0; transform: scale(.7); } 25% { opacity: 1; transform: scale(1); } 100% { opacity: 0; transform: scale(1.18); } }
    #osd-center svg, #osd-toast svg { width: 48px; height: 48px; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
    #osd-center svg [fill="currentColor"] { fill: currentColor; stroke: none; }
    #osd-toast svg [fill="currentColor"] { fill: currentColor; stroke: none; }

    #osd-toast { position: fixed; left: 50%; bottom: 160px; display: flex; align-items: center; gap: 12px;
      padding: 12px 24px 12px 16px; border-radius: 999px; background: rgba(28, 28, 33, .82);
      border: 1px solid rgba(255, 255, 255, .1); color: #fff; font-size: 17px; font-weight: 700;
      font-family: 'Inter', 'Segoe UI', sans-serif; font-variant-numeric: tabular-nums;
      -webkit-backdrop-filter: blur(14px); backdrop-filter: blur(14px);
      opacity: 0; transform: translate(-50%, 10px); transition: opacity .2s ease, transform .3s cubic-bezier(.2, .8, .2, 1); pointer-events: none; }
    #osd-toast.on { opacity: 1; transform: translate(-50%, 0); }
    #osd-toast svg { width: 26px; height: 26px; }
    .osd-vol { width: 120px; height: 5px; border-radius: 999px; background: rgba(255, 255, 255, .22); overflow: hidden; }
    .osd-vol i { display: block; height: 100%; background: #fff; border-radius: inherit; transition: width .15s linear; }

    #osd-spinner { position: fixed; left: 50%; top: 50%; width: 54px; height: 54px; margin: -27px 0 0 -27px;
      border-radius: 50%; border: 3px solid rgba(255, 255, 255, .18); border-top-color: #fff;
      opacity: 0; pointer-events: none; transition: opacity .25s ease; animation: osdSpin .9s linear infinite; }
    #osd-spinner.on { opacity: 1; }
    @keyframes osdSpin { to { transform: rotate(360deg); } }

    #osd-err { position: fixed; inset: 0; display: none; flex-direction: column; align-items: center; justify-content: center;
      gap: 12px; padding: 0 56px; text-align: center; color: #fff; font-family: 'Inter', 'Segoe UI', sans-serif;
      background: radial-gradient(120% 90% at 50% -10%, #1b1b20 0%, #151517 45%, #0a0a0c 100%); }
    #osd-err.on { display: flex; }
    #osd-err .osd-err-icon { width: 88px; height: 88px; border-radius: 50%; margin-bottom: 8px; background: rgba(255, 255, 255, .07);
      display: flex; align-items: center; justify-content: center; }
    #osd-err svg { width: 44px; height: 44px; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
    #osd-err .osd-err-title { font-size: 32px; font-weight: 800; letter-spacing: -.3px; }
    #osd-err .osd-err-text { font-size: 17px; font-weight: 500; color: rgba(255, 255, 255, .68); line-height: 1.5; max-width: 560px; }
  `;

  // Stroke icons (24x24 viewBox). Parts marked fill="currentColor" are solid.
  const ICON = {
    play: '<path d="M8 5.2v13.6L19.5 12z" fill="currentColor"/>',
    pause: '<path d="M7 5h3.6v14H7zM13.4 5H17v14h-3.6z" fill="currentColor"/>',
    back: '<path d="M11 6l-6 6 6 6M19 6l-6 6 6 6"/>',
    forward: '<path d="M13 6l6 6-6 6M5 6l6 6-6 6"/>',
    volume: '<path d="M4 9.5v5h3.5l4.5 4v-13l-4.5 4z" fill="currentColor"/><path d="M15.5 9a4.5 4.5 0 0 1 0 6M18 6.5a8 8 0 0 1 0 11"/>',
    mute: '<path d="M4 9.5v5h3.5l4.5 4v-13l-4.5 4z" fill="currentColor"/><path d="M16 9.5l5 5M21 9.5l-5 5"/>',
    warning: '<path d="M12 3.5l9.5 17h-19zM12 10v4.5M12 17.5h.01"/>'
  };
  const svg = (name) => `<svg viewBox="0 0 24 24" aria-hidden="true">${ICON[name] || ''}</svg>`;

  const fmt = (seconds) => {
    let s = Number.isFinite(seconds) && seconds > 0 ? Math.floor(seconds) : 0;
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    s %= 60;
    return `${h ? `${h}:${String(m).padStart(2, '0')}` : m}:${String(s).padStart(2, '0')}`;
  };

  // "Movies\Crossroads\crossroads.2002.mp4" -> "crossroads 2002"
  const titleFromPath = (path) => {
    const name = String(path || '').split(/[\\/]/).pop().replace(/\.[a-z0-9]{2,4}$/i, '').replace(/[._]+/g, ' ').trim();
    return name || '';
  };

  let api = null; // set by init(); error() before init() falls back to a plain page

  function init(opts) {
    const video = opts.video;
    const live = !!opts.live;
    video.removeAttribute('controls');
    video.controls = false;

    const style = document.createElement('style');
    style.textContent = CSS;
    document.head.appendChild(style);

    const root = document.createElement('div');
    root.id = 'osd';
    if (live) root.classList.add('live');
    root.innerHTML = `
      <div class="osd-top"><div class="osd-title"></div><div class="osd-live">LIVE</div></div>
      <div class="osd-bottom">
        <div class="osd-bar"><div class="osd-buffered"></div><div class="osd-played"></div></div>
        <div class="osd-times"><span class="osd-cur">0:00</span><span class="osd-dur">0:00</span></div>
      </div>
      <div class="osd-hit"></div>`;
    root.querySelector('.osd-title').textContent = opts.title || '';
    const center = document.createElement('div');
    center.id = 'osd-center';
    const toastEl = document.createElement('div');
    toastEl.id = 'osd-toast';
    const spinner = document.createElement('div');
    spinner.id = 'osd-spinner';
    const errEl = document.createElement('div');
    errEl.id = 'osd-err';
    errEl.innerHTML = `<div class="osd-err-icon">${svg('warning')}</div><div class="osd-err-title"></div><div class="osd-err-text"></div>`;
    document.body.append(root, center, toastEl, spinner, errEl);

    const playedEl = root.querySelector('.osd-played');
    const bufferedEl = root.querySelector('.osd-buffered');
    const curEl = root.querySelector('.osd-cur');
    const durEl = root.querySelector('.osd-dur');

    // ---- visibility: any input shows the OSD, it fades out while playing ----
    let hideTimer = null;
    const isVisible = () => root.classList.contains('visible');

    function updateProgress() {
      const d = video.duration;
      if (live || !Number.isFinite(d) || d <= 0) return;
      playedEl.style.transform = `scaleX(${Math.min(1, video.currentTime / d)})`;
      let end = 0;
      for (let i = 0; i < video.buffered.length; i++) {
        if (video.buffered.start(i) <= video.currentTime + 0.5) end = Math.max(end, video.buffered.end(i));
      }
      bufferedEl.style.transform = `scaleX(${Math.min(1, end / d)})`;
      curEl.textContent = fmt(video.currentTime);
      durEl.textContent = fmt(d);
    }
    function show(sticky) {
      root.classList.add('visible');
      document.body.classList.remove('osd-idle');
      updateProgress();
      clearTimeout(hideTimer);
      if (!sticky && !video.paused && !video.ended) hideTimer = setTimeout(hide, 3800);
    }
    function hide() {
      if (video.paused || video.ended) return; // stays up while paused
      root.classList.remove('visible');
      document.body.classList.add('osd-idle');
    }

    // ---- transient feedback ----
    let toastTimer = null;
    function toast(html, ms) {
      toastEl.innerHTML = html;
      toastEl.classList.add('on');
      clearTimeout(toastTimer);
      toastTimer = setTimeout(() => toastEl.classList.remove('on'), ms || 900);
    }
    function flashCenter(icon) {
      center.innerHTML = svg(icon);
      center.classList.remove('pop');
      void center.offsetWidth; // restart the animation
      center.classList.add('pop');
    }

    // ---- actions ----
    function toggle() {
      if (video.ended) video.currentTime = 0;
      if (video.paused || video.ended) video.play().catch(() => {});
      else video.pause();
      flashCenter(video.paused ? 'pause' : 'play'); // the state we just switched to (like YouTube)
      show();
    }
    let seekSum = 0;
    let seekReset = null;
    function seekBy(delta) {
      if (live) { show(); return; }
      const d = video.duration;
      if (!Number.isFinite(d)) return;
      video.currentTime = Math.max(0, Math.min(d - 0.5, video.currentTime + delta));
      seekSum += delta;
      clearTimeout(seekReset);
      seekReset = setTimeout(() => { seekSum = 0; }, 900);
      toast(`${svg(seekSum < 0 ? 'back' : 'forward')}<span>${seekSum < 0 ? '−' : '+'}${Math.abs(seekSum)} s</span>`);
      show();
    }
    const VOLUME_KEY = 'tv-launcher-volume';
    try {
      const saved = parseFloat(localStorage.getItem(VOLUME_KEY));
      if (Number.isFinite(saved)) video.volume = Math.min(1, Math.max(0, saved));
    } catch (err) { /* no storage: default volume */ }
    function volumeToast() {
      const pct = Math.round(video.volume * 100);
      toast(`${svg(video.muted || pct === 0 ? 'mute' : 'volume')}<div class="osd-vol"><i style="width:${video.muted ? 0 : pct}%"></i></div><span>${video.muted ? 'Stumm' : `${pct} %`}</span>`, 1100);
    }
    function volumeBy(delta) {
      video.muted = false;
      video.volume = Math.min(1, Math.max(0, Math.round((video.volume + delta) * 100) / 100));
      try { localStorage.setItem(VOLUME_KEY, String(video.volume)); } catch (err) { /* ignore */ }
      volumeToast();
      show();
    }
    function toggleMute() {
      video.muted = !video.muted;
      volumeToast();
      show();
    }

    // ---- input ----
    let lastSeekAt = 0;
    document.addEventListener('keydown', (e) => {
      if (e.ctrlKey || e.altKey || e.metaKey) return;
      const now = performance.now();
      switch (e.key) {
        case 'Enter':
        case ' ':
        case 'MediaPlayPause':
          e.preventDefault();
          if (!e.repeat) toggle();
          break;
        case 'MediaPlay': if (video.paused) toggle(); break;
        case 'MediaPause': if (!video.paused) toggle(); break;
        case 'ArrowRight':
        case 'ArrowLeft':
          e.preventDefault();
          // Holding the key keeps seeking, but at a steady pace and in bigger steps.
          if (e.repeat && now - lastSeekAt < 250) break;
          lastSeekAt = now;
          seekBy((e.key === 'ArrowRight' ? 1 : -1) * (e.repeat ? 30 : 10));
          break;
        case 'PageUp': case 'PageDown':
          e.preventDefault();
          seekBy(e.key === 'PageUp' ? 60 : -60);
          break;
        case 'ArrowUp': e.preventDefault(); volumeBy(0.05); break;
        case 'ArrowDown': e.preventDefault(); volumeBy(-0.05); break;
        case 'm': case 'M': toggleMute(); break;
        default: show();
      }
    });
    let lastMove = 0;
    document.addEventListener('mousemove', () => {
      const now = performance.now();
      if (now - lastMove < 120) return;
      lastMove = now;
      show();
    }, { passive: true });
    video.addEventListener('click', () => toggle());
    root.querySelector('.osd-hit').addEventListener('click', (e) => {
      const d = video.duration;
      if (live || !Number.isFinite(d)) return;
      const r = root.querySelector('.osd-bar').getBoundingClientRect();
      video.currentTime = Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)) * d;
      show();
    });

    // ---- media state ----
    let spinTimer = null;
    function buffering(on) {
      clearTimeout(spinTimer);
      if (on) spinTimer = setTimeout(() => spinner.classList.add('on'), 450);
      else spinner.classList.remove('on');
    }
    buffering(true); // black picture until the first frame: show the spinner, not nothing
    video.addEventListener('waiting', () => buffering(true));
    ['playing', 'canplay', 'seeked', 'pause', 'ended', 'error'].forEach((type) => video.addEventListener(type, () => buffering(false)));
    video.addEventListener('playing', () => show());
    video.addEventListener('pause', () => show(true));
    video.addEventListener('ended', () => show(true));
    video.addEventListener('loadedmetadata', updateProgress);
    video.addEventListener('durationchange', updateProgress);
    video.addEventListener('timeupdate', () => { if (isVisible()) updateProgress(); });

    // ---- errors: a friendly TV screen instead of a black frame ----
    function error(title, text) {
      errEl.querySelector('.osd-err-title').textContent = title;
      errEl.querySelector('.osd-err-text').textContent = text || '';
      errEl.classList.add('on');
      root.classList.remove('visible');
      buffering(false);
    }
    video.addEventListener('error', () => {
      const code = video.error && video.error.code;
      if (live) error('Sender momentan nicht verfügbar', 'Der Stream antwortet nicht. Versuche es später noch einmal oder wähle einen anderen Sender.');
      else if (code === 4) error('Format wird nicht unterstützt', 'Dieses Video kann hier nicht abgespielt werden. MKV-Filme brauchen ffmpeg auf dem Rechner, auf dem sie liegen (server/ANLEITUNG.md).');
      else if (code === 2) error('Verbindung unterbrochen', 'Die Datei konnte nicht weiter geladen werden.');
      else error('Wiedergabe nicht möglich', 'Das Video konnte nicht abgespielt werden.');
    });

    show(); // the OSD is up for a moment when playback starts
    api = { error, show, toast };
    return api;
  }

  // Before init() (e.g. no video source at all): a minimal styled message.
  function fallbackError(title, text) {
    document.body.innerHTML = '';
    const el = document.createElement('div');
    el.style.cssText = 'position:fixed;inset:0;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:10px;' +
      'color:#fff;font-family:Inter,"Segoe UI",sans-serif;text-align:center;padding:0 56px;background:#0a0a0c;';
    const h = document.createElement('div');
    h.style.cssText = 'font-size:32px;font-weight:800;';
    h.textContent = title;
    const p = document.createElement('div');
    p.style.cssText = 'font-size:17px;color:rgba(255,255,255,.68);';
    p.textContent = text || '';
    el.append(h, p);
    document.body.appendChild(el);
  }

  window.TvPlayer = {
    init,
    titleFromPath,
    error: (title, text) => (api ? api.error(title, text) : fallbackError(title, text))
  };
})();
