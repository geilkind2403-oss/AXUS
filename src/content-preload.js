const { contextBridge, ipcRenderer, webFrame } = require('electron');

// Launcher over its RAM budget (memory-main.js): drop Blink caches, collect.
ipcRenderer.on('memory-trim', () => {
  webFrame.clearCache();
  setTimeout(() => { if (typeof gc === 'function') gc(); }, 0);
});

// Attached to the shared content view for every site it opens (YouTube,
// Netflix, live TV, local media...). Only mediaplayer.html actually uses
// this -- it's a no-op, harmless global on everything else.
contextBridge.exposeInMainWorld('mediaPlayer', {
  reportProgress: (relativePath, seconds) => ipcRenderer.send('media-progress', relativePath, seconds)
});

// Live TV app (livetv.html): playlist from data/livetv, "back" presses routed
// to the page (main.js contentHandlesBack), and a way to close itself.
contextBridge.exposeInMainWorld('liveTv', {
  getPlaylist: () => ipcRenderer.invoke('livetv-playlist'),
  exit: () => ipcRenderer.send('livetv-exit'),
  onBack: (cb) => ipcRenderer.on('tv-back', () => cb())
});

// --- TV-OS page layer: runs on every page the content view opens (YouTube,
// Netflix, ...). Removes the desktop-browser tells a web page can show, and
// draws system toasts on top of the page (the launcher UI is hidden beneath a
// web app). Every step is best effort: nothing here may stop a page from loading.
(() => {
  const safe = (fn) => { try { fn(); } catch (err) { /* ignore */ } };

  // No text selection (except in text fields), no drag ghosts of images/links.
  safe(() => webFrame.insertCSS(`
    html, body, body * { -webkit-user-select: none !important; user-select: none !important; }
    input, textarea, [contenteditable=""], [contenteditable="true"], [contenteditable="true"] *, [contenteditable="plaintext-only"] {
      -webkit-user-select: text !important; user-select: text !important;
    }
    img, a, svg, video { -webkit-user-drag: none !important; }
  `));
  // Slim translucent scrollbars like the launcher's own, not Windows' grey bars.
  // User origin: a page that styles or hides its own scrollbars still wins.
  safe(() => webFrame.insertCSS(`
    ::-webkit-scrollbar { width: 10px; height: 10px; background: transparent; }
    ::-webkit-scrollbar-track { background: transparent; }
    ::-webkit-scrollbar-thumb { background: rgba(128, 128, 140, 0.5); border-radius: 999px; }
  `, { cssOrigin: 'user' }));

  // The mouse cursor only exists while the mouse actually moves (same as the
  // launcher itself): a real remote has no pointer.
  const HIDE_CURSOR_CSS = 'html, html * { cursor: none !important; }';
  let hideKey = null;
  let hideTimer = null;
  const hideCursor = () => {
    hideTimer = null;
    if (hideKey === null) safe(() => { hideKey = webFrame.insertCSS(HIDE_CURSOR_CSS); });
  };
  const showCursor = () => {
    if (hideKey !== null) { const key = hideKey; hideKey = null; safe(() => webFrame.removeInsertedCSS(key)); }
    clearTimeout(hideTimer);
    hideTimer = setTimeout(hideCursor, 2500);
  };
  hideCursor();
  window.addEventListener('mousemove', showCursor, { passive: true, capture: true });

  // No native alert/confirm/prompt/print dialogs (they look like Windows and
  // block the page). confirm() answers "no", the safe default.
  safe(() => webFrame.executeJavaScript(
    '(() => { window.alert = () => undefined; window.confirm = () => false; window.prompt = () => null; window.print = () => undefined; })();'
  ).catch(() => {}));

  // No zoom, no context menu, no drag & drop of files onto the page.
  window.addEventListener('wheel', (e) => { if (e.ctrlKey) e.preventDefault(); }, { passive: false, capture: true });
  ['contextmenu', 'dragstart', 'dragover', 'drop'].forEach((type) => {
    window.addEventListener(type, (e) => e.preventDefault(), true);
  });
  // Middle click would start Windows-style autoscroll (a little arrow icon).
  window.addEventListener('mousedown', (e) => { if (e.button === 1) e.preventDefault(); }, true);

  // Tell the main process whether a text field has focus: the Home key goes
  // "home" unless the user is typing (see tvos-main.js).
  const NON_TEXT_INPUTS = /^(button|checkbox|radio|submit|reset|range|image|file|color)$/i;
  const isEditing = (el) => !!el && (el.isContentEditable || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT' ||
    (el.tagName === 'INPUT' && !NON_TEXT_INPUTS.test(el.type)));
  let editing = false;
  const reportEditing = () => {
    const now = isEditing(document.activeElement);
    if (now !== editing) { editing = now; ipcRenderer.send('tvos-editing', now); }
  };
  window.addEventListener('focusin', reportEditing, true);
  window.addEventListener('focusout', () => setTimeout(reportEditing, 0), true);

  // ---- system toasts drawn inside the page --------------------------------
  // Built with DOM calls and CSSOM only (no innerHTML, no <style>), so strict
  // CSP / Trusted Types pages (YouTube) can't block it; a popover puts it in
  // the top layer, i.e. above a fullscreen video too.
  const KIND_COLOR = { info: '#3b6dff', success: '#3ddc84', warning: '#e0a01c', error: '#ff6b6b' };
  const ICON_PATHS = {
    wifi: ['M2 8.5a16 16 0 0 1 20 0', 'M5.5 12a11 11 0 0 1 13 0', 'M9 15.5a6 6 0 0 1 6 0', 'M12 19h.01'],
    'wifi-off': ['M2 8.5a16 16 0 0 1 20 0', 'M5.5 12a11 11 0 0 1 13 0', 'M9 15.5a6 6 0 0 1 6 0', 'M12 19h.01', 'M3 3l18 18'],
    phone: ['M9.5 2.5h5a2 2 0 0 1 2 2v15a2 2 0 0 1-2 2h-5a2 2 0 0 1-2-2v-15a2 2 0 0 1 2-2z', 'M11 18.5h2'],
    bluetooth: ['M6.5 7.5l11 9-5.5 5V2l5.5 5-11 9'],
    check: ['M5 12.5l4.5 4.5L19 7.5'],
    info: ['M12 3a9 9 0 1 0 0 18a9 9 0 0 0 0-18z', 'M12 11v5.5', 'M12 7.6h.01'],
    warning: ['M12 3.5l9.5 17h-19z', 'M12 10v4.5', 'M12 17.5h.01']
  };
  const SVG_NS = 'http://www.w3.org/2000/svg';
  const EXPO = 'cubic-bezier(.16, 1, .3, 1)';

  function getToastHost() {
    let host = document.getElementById('__tvos_toasts__');
    if (host && host.isConnected) return host;
    host = document.createElement('div');
    host.id = '__tvos_toasts__';
    host.style.cssText = 'position:fixed;inset:auto;top:24px;right:32px;left:auto;bottom:auto;margin:0;padding:0;border:0;' +
      'background:transparent;overflow:visible;width:360px;max-width:calc(100vw - 48px);height:auto;display:flex;' +
      'flex-direction:column;gap:10px;pointer-events:none;z-index:2147483647;';
    host.setAttribute('popover', 'manual');
    document.documentElement.appendChild(host);
    try { host.showPopover(); } catch (err) { host.removeAttribute('popover'); }
    return host;
  }

  function dismissToast(card) {
    if (!card.isConnected || card.__leaving) return;
    card.__leaving = true;
    const out = card.animate([{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'translateY(-8px)' }],
      { duration: 160, easing: 'cubic-bezier(.4, 0, 1, 1)', fill: 'forwards' });
    out.onfinish = () => card.remove();
  }

  function showPageToast(t) {
    safe(() => {
      if (!t || (!t.title && !t.text) || !document.documentElement) return;
      const host = getToastHost();
      if (t.id) host.querySelectorAll('[data-toast-id="' + t.id + '"]').forEach((c) => c.remove());
      while (host.children.length >= 3) host.firstElementChild.remove();

      const color = KIND_COLOR[t.kind] || KIND_COLOR.info;
      const card = document.createElement('div');
      if (t.id) card.setAttribute('data-toast-id', t.id);
      card.style.cssText = 'display:flex;align-items:center;gap:14px;padding:14px 20px 14px 14px;border-radius:18px;' +
        'background:rgba(28,28,33,0.86);border:1px solid rgba(255,255,255,0.1);box-shadow:0 14px 44px rgba(0,0,0,0.55);' +
        'backdrop-filter:blur(16px) saturate(1.3);color:#f1f1f3;font-family:Inter,"Segoe UI",sans-serif;';

      const badge = document.createElement('div');
      badge.style.cssText = 'flex:none;width:42px;height:42px;border-radius:50%;display:flex;align-items:center;justify-content:center;' +
        'background:' + color + '2e;color:' + color + ';';
      const paths = ICON_PATHS[t.icon];
      if (paths) {
        const svg = document.createElementNS(SVG_NS, 'svg');
        svg.setAttribute('viewBox', '0 0 24 24');
        svg.setAttribute('fill', 'none');
        svg.setAttribute('stroke', 'currentColor');
        svg.setAttribute('stroke-width', '1.8');
        svg.setAttribute('stroke-linecap', 'round');
        svg.setAttribute('stroke-linejoin', 'round');
        svg.style.cssText = 'width:22px;height:22px;';
        paths.forEach((d) => {
          const p = document.createElementNS(SVG_NS, 'path');
          p.setAttribute('d', d);
          svg.appendChild(p);
        });
        badge.appendChild(svg);
      } else if (t.icon) {
        badge.textContent = String(t.icon);
        badge.style.fontSize = '20px';
      }

      const body = document.createElement('div');
      body.style.cssText = 'min-width:0;';
      if (t.title) {
        const title = document.createElement('div');
        title.textContent = String(t.title);
        title.style.cssText = 'font-size:15px;font-weight:700;line-height:1.3;';
        body.appendChild(title);
      }
      if (t.text) {
        const text = document.createElement('div');
        text.textContent = String(t.text);
        text.style.cssText = 'font-size:13px;font-weight:500;color:#a4a4ad;line-height:1.35;margin-top:2px;';
        body.appendChild(text);
      }
      card.append(badge, body);
      host.appendChild(card);
      card.animate([{ opacity: 0, transform: 'translateX(28px)' }, { opacity: 1, transform: 'none' }],
        { duration: 480, easing: EXPO });
      setTimeout(() => dismissToast(card), Math.max(1500, Number(t.duration) || 4000));
    });
  }

  ipcRenderer.on('tvos-toast', (event, payload) => showPageToast(payload));
})();

// --- Einstellungen > Bild on every page (films, Live TV, Netflix, YouTube,
// ...): the same filter the launcher puts on its own root element (built by
// picture.js, checked by main.js). CSS filter functions, plus -- only when
// Farbtemperatur, Augenschonend or Schärfe need it -- a tiny inline SVG
// filter. That SVG has to live in the page itself: a data: URL filter would be
// blocked by the Content-Security-Policy of many sites. :fullscreen elements
// get the filter too, because the top layer (a fullscreen video) is not
// painted inside <html>'s filter. Fetched synchronously as each page starts,
// so its first frame is already right; changes arrive live as 'tv-picture'.
(() => {
  const FILTER_ID = '__tv_picture__';
  const SVG_NS = 'http://www.w3.org/2000/svg';
  let payload = null;
  let cssKey = null;

  function stampSvg() {
    const root = document.documentElement;
    if (!root) return; // too early: DOMContentLoaded comes back for it
    let svg = document.getElementById(`${FILTER_ID}svg`);
    if (!payload || (!payload.matrix && !payload.kernel)) {
      if (svg) svg.remove();
      return;
    }
    if (!svg) {
      svg = document.createElementNS(SVG_NS, 'svg');
      svg.id = `${FILTER_ID}svg`;
      svg.setAttribute('width', '0');
      svg.setAttribute('height', '0');
      svg.setAttribute('aria-hidden', 'true');
      svg.style.cssText = 'position:absolute;width:0;height:0;overflow:hidden;pointer-events:none;';
      root.appendChild(svg);
    }
    const filter = document.createElementNS(SVG_NS, 'filter');
    filter.id = FILTER_ID;
    filter.setAttribute('color-interpolation-filters', 'sRGB');
    if (payload.matrix) {
      const m = document.createElementNS(SVG_NS, 'feColorMatrix');
      m.setAttribute('type', 'matrix');
      m.setAttribute('values', payload.matrix);
      filter.appendChild(m);
    }
    if (payload.kernel) {
      const cm = document.createElementNS(SVG_NS, 'feConvolveMatrix');
      cm.setAttribute('order', '3');
      cm.setAttribute('kernelMatrix', payload.kernel);
      cm.setAttribute('divisor', '1');
      cm.setAttribute('edgeMode', 'duplicate');
      cm.setAttribute('preserveAlpha', 'true');
      filter.appendChild(cm);
    }
    svg.replaceChildren(filter);
  }

  function apply(next) {
    payload = next && typeof next === 'object' && typeof next.filter === 'string' ? next : null;
    // Author origin (the default) on purpose: removeInsertedCSS() only finds
    // author sheets -- a 'user' sheet could never be taken out again.
    // !important keeps the page's own rules from overriding it. Removing and
    // inserting in the same task: no unfiltered frame in between.
    if (cssKey !== null) {
      const key = cssKey;
      cssKey = null;
      try { webFrame.removeInsertedCSS(key); } catch (err) { /* ignore */ }
    }
    if (payload) {
      try {
        cssKey = webFrame.insertCSS(`html, :fullscreen { filter: ${payload.filter} !important; }`);
      } catch (err) { /* ignore */ }
    }
    try { stampSvg(); } catch (err) { /* the CSS functions still apply */ }
  }

  ipcRenderer.on('tv-picture', (event, next) => apply(next));
  let initial = null;
  try { initial = ipcRenderer.sendSync('tv-picture-get'); } catch (err) { /* launcher not ready: no filter */ }
  apply(initial);
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', () => { try { stampSvg(); } catch (err) { /* ignore */ } }, { once: true });
  }
})();
