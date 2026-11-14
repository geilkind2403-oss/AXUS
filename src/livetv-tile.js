// --- Live TV preview tile (#grid-2): the channel you watched last (or your
// first favourite, or channel 1) plays muted right on the home screen. OK on
// the tile opens the Live TV app on that same channel (openAction('livetv')
// -> livetv.html picks up 'livetv-last' itself).
//
// The stream only runs while the home screen is actually visible: it is torn
// down while a web app / Live TV is open, during Art Mode and in standby, so
// the preview never doubles the bandwidth of what you're really watching.
// Loaded after renderer.js and tvos.js (reads contentOpen / artModeActive,
// onHomeVisibilityChange tells it when one of them changes).
(() => {
  const g = (fn) => { try { return fn(); } catch { return undefined; } };
  const $ = (id) => document.getElementById(id);
  const cell = $('grid-2');
  if (!cell) return;

  cell.classList.remove('grid-cell-placeholder');
  cell.classList.add('grid-cell-livetv');
  cell.dataset.action = 'livetv';
  cell.innerHTML = `
    <video class="ltv-video" id="ltv-video" muted playsinline></video>
    <div class="ltv-shade"></div>
    <div class="ltv-badge"><span class="ltv-dot"></span>LIVE</div>
    <div class="ltv-bar">
      <div class="ltv-logo" id="ltv-logo"></div>
      <div class="ltv-txt">
        <div class="ltv-name" id="ltv-name">Live TV</div>
        <div class="ltv-sub" id="ltv-sub">Sender werden geladen …</div>
      </div>
    </div>`;

  const video = $('ltv-video');
  const store = (key, fallback) => { try { const v = JSON.parse(localStorage.getItem(key)); return v == null ? fallback : v; } catch { return fallback; } };

  let channels = [];
  let channel = null;
  let hls = null;
  let playing = false;
  let failedAt = 0;
  let retryTimer = null;

  function pickChannel() {
    if (!channels.length) return null;
    const last = store('livetv-last', null);
    const favs = store('livetv-favs', []);
    return channels.find((c) => c.url === last) ||
      channels.find((c) => favs.includes(c.url)) ||
      channels[0];
  }

  function showChannel(ch) {
    $('ltv-name').textContent = ch ? ch.name : 'Live TV';
    $('ltv-sub').textContent = ch ? `Sender ${ch.number} · OK zum Einschalten` : 'Keine Sender – M3U-Liste in data/livetv ablegen';
    const logo = $('ltv-logo');
    logo.innerHTML = '';
    if (ch && ch.logo) {
      const img = new Image();
      img.alt = '';
      img.onerror = () => img.remove();
      img.src = ch.logo;
      logo.appendChild(img);
    }
    logo.classList.toggle('empty', !(ch && ch.logo));
    cell.classList.toggle('no-channels', !ch);
  }

  // A menu over the home screen: hold the picture (no decoding, no
  // downloading) instead of tearing the stream down, so it goes on at once.
  let held = false;
  function hold() {
    if (!playing || held) return;
    held = true;
    video.pause();
    if (hls) hls.stopLoad();
  }
  function release() {
    if (!held) return;
    held = false;
    if (hls) hls.startLoad(-1);
    video.play().catch(() => {});
  }

  function stop() {
    held = false;
    if (!playing) return;
    playing = false;
    if (hls) { hls.destroy(); hls = null; }
    video.pause();
    video.removeAttribute('src');
    video.load();
    cell.classList.remove('ltv-live');
  }

  function start() {
    // The Live TV app may have switched channel since the last run.
    const next = pickChannel();
    if (next !== channel) { channel = next; showChannel(channel); }
    if (!channel || playing) return;
    playing = true;
    video.muted = true;
    if (window.Hls && Hls.isSupported()) {
      hls = new Hls({ maxBufferLength: 8, backBufferLength: 0, capLevelToPlayerSize: true });
      hls.on(Hls.Events.ERROR, (event, data) => { if (data.fatal) failed(); });
      hls.loadSource(channel.url);
      hls.attachMedia(video);
    } else {
      video.src = channel.url; // native HLS or a plain stream
    }
    video.play().catch(() => {});
  }
  // After a dead stream, wait a bit before trying again instead of hammering it.
  const RETRY_MS = 30000;
  function failed() {
    failedAt = Date.now();
    stop();
    clearTimeout(retryTimer);
    retryTimer = setTimeout(sync, RETRY_MS + 100);
  }
  video.addEventListener('playing', () => cell.classList.add('ltv-live'));
  video.addEventListener('error', failed);

  const homeVisible = () =>
    !g(() => contentOpen) && !g(() => artModeActive) && !g(() => window.tvos.standby.isOn()) && !document.hidden;

  function sync() {
    if (!channels.length) return;
    if (homeVisible() && g(() => menuCovering)) {
      hold();
    } else if (homeVisible()) {
      release();
      if (!playing && Date.now() - failedAt > RETRY_MS) start();
    } else {
      stop();
    }
  }

  Promise.resolve(g(() => window.launcher.getLiveTvPlaylist())).then((list) => {
    channels = Array.isArray(list) ? list : [];
    channel = pickChannel();
    showChannel(channel);
    sync();
  }).catch(() => showChannel(null));

  g(() => onHomeVisibilityChange(sync)); // also covers visibilitychange
})();
