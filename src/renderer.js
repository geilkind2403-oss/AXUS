// Generic remote-friendly navigation: every focusable element declares its
// neighbors via data-nav-up/down/left/right (IDs), so arrow keys can reach
// literally any element on screen, not just one row of tiles.
const navElements = Array.from(document.querySelectorAll('[data-nav-up], [data-nav-down], [data-nav-left], [data-nav-right]'));
let focusedEl = document.getElementById('nav-artmode');
let contentOpen = false;

function isDockIcon(el) {
  return el.classList.contains('dock-icon');
}

function setFocusEl(el) {
  if (!el) return;
  if (focusedEl) {
    focusedEl.classList.remove('focused');
    if (isDockIcon(focusedEl)) focusedEl.classList.remove('expanded');
  }
  focusedEl = el;
  focusedEl.classList.add('focused');
  if (isDockIcon(focusedEl)) focusedEl.classList.add('expanded');
}
setFocusEl(focusedEl);

function navigate(direction) {
  const attr = `nav${direction}`;
  const targetId = focusedEl.dataset[attr];
  if (!targetId) return;
  const target = document.getElementById(targetId);
  if (target) setFocusEl(target);
}

const ACTIONS = {
  youtube: 'https://www.youtube.com/tv',
  netflix: 'https://www.netflix.com',
  primevideo: 'https://www.primevideo.com',
  disneyplus: 'https://www.disneyplus.com',
  browser: 'https://www.google.com',
  // Live TV needs an actual IPTV/m3u source configured before this does anything real
  livetv: null
};

function openAction(action) {
  if (action === 'livetv') {
    openLiveTV();
    return;
  }
  if (action === 'spotify') {
    handleSpotifyTileClick();
    return;
  }
  const url = ACTIONS[action];
  if (!url) return;
  contentOpen = true;
  window.launcher.openUrl(url, { asSmartTv: action === 'youtube' });
}

async function handleSpotifyTileClick() {
  const loggedIn = await window.launcher.spotifyIsLoggedIn();
  if (loggedIn) {
    contentOpen = true;
    window.launcher.openUrl('https://open.spotify.com');
    return;
  }
  const result = await window.launcher.spotifyLogin();
  if (result.ok) {
    refreshNowPlaying();
  } else {
    alert('Spotify-Login fehlgeschlagen: ' + result.error);
  }
}

// Dock icons: first click/focus expands (shows the card face), a second
// click on an already-expanded icon opens it.
document.querySelectorAll('.dock-icon').forEach((icon) => {
  icon.addEventListener('click', () => {
    if (icon.classList.contains('expanded')) {
      openAction(icon.dataset.action);
    } else {
      setFocusEl(icon);
    }
  });
});

// Top-bar nav items with an action also open directly.
document.querySelectorAll('[data-action]:not(.dock-icon)').forEach((item) => {
  item.addEventListener('click', () => {
    setFocusEl(item);
    openAction(item.dataset.action);
  });
});

// --- Live TV overlay (official public-broadcaster HLS streams) ---
let livetvOpen = false;
const livetvOverlay = document.getElementById('livetv-overlay');
const livetvNavItems = Array.from(document.querySelectorAll('.livetv-nav'));
let livetvFocusIndex = 0;

function openLiveTV() {
  livetvOpen = true;
  livetvOverlay.classList.add('active');
  livetvNavItems.forEach(el => el.classList.remove('focused'));
  livetvFocusIndex = 0;
  livetvNavItems[0].classList.add('focused');
}

function closeLiveTV() {
  livetvOpen = false;
  livetvOverlay.classList.remove('active');
}

function setLivetvFocus(i) {
  livetvNavItems[livetvFocusIndex].classList.remove('focused');
  livetvFocusIndex = (i + livetvNavItems.length) % livetvNavItems.length;
  livetvNavItems[livetvFocusIndex].classList.add('focused');
}

function handleLivetvKey(e) {
  const current = livetvNavItems[livetvFocusIndex];
  switch (e.key) {
    case 'Escape':
      closeLiveTV();
      break;
    case 'ArrowRight':
      setLivetvFocus(livetvFocusIndex + 1);
      break;
    case 'ArrowLeft':
      setLivetvFocus(livetvFocusIndex - 1);
      break;
    case 'ArrowDown':
      setLivetvFocus(livetvNavItems.length - 1); // jump to close button
      break;
    case 'ArrowUp':
      setLivetvFocus(0);
      break;
    case 'Enter':
      activateLivetvItem(current);
      break;
  }
}

function activateLivetvItem(el) {
  if (el.dataset.kind === 'close') {
    closeLiveTV();
  } else if (el.dataset.stream) {
    closeLiveTV();
    contentOpen = true;
    window.launcher.openLiveTv(el.dataset.stream);
  }
}

livetvNavItems.forEach((el) => {
  el.addEventListener('click', () => activateLivetvItem(el));
});
livetvOverlay.addEventListener('click', (e) => {
  if (e.target === livetvOverlay) closeLiveTV();
});

window.launcher.onContentClosed(() => {
  contentOpen = false;
});

document.addEventListener('keydown', (e) => {
  if (settingsOpen) {
    handleSettingsKey(e);
    return;
  }
  if (livetvOpen) {
    handleLivetvKey(e);
    return;
  }
  if (contentOpen) return; // content view has its own focus while open

  switch (e.key) {
    case 'ArrowDown':
      navigate('Down');
      break;
    case 'ArrowUp':
      navigate('Up');
      break;
    case 'ArrowLeft':
      navigate('Left');
      break;
    case 'ArrowRight':
      navigate('Right');
      break;
    case 'Enter':
      if (focusedEl.id === 'btn-settings') {
        openSettings();
      } else if (focusedEl.dataset.action) {
        if (isDockIcon(focusedEl) && !focusedEl.classList.contains('expanded')) {
          // First Enter on a dock card just reveals it, mirrors the click behavior.
          focusedEl.classList.add('expanded');
        } else {
          openAction(focusedEl.dataset.action);
        }
      } else {
        // Plain buttons (transport controls, etc.) without a data-action just get clicked.
        focusedEl.click();
      }
      break;
  }
});

// --- Settings panel ---
let settingsOpen = false;
const settingsOverlay = document.getElementById('settings-overlay');
const adblockToggle = document.getElementById('toggle-adblock');
const fullscreenToggle = document.getElementById('toggle-fullscreen');
const displaySelect = document.getElementById('display-select');
const settingsNavItems = Array.from(document.querySelectorAll('.settings-nav'));
let settingsFocusIndex = 0;

function setSettingsFocus(i) {
  settingsNavItems[settingsFocusIndex].classList.remove('focused');
  settingsFocusIndex = (i + settingsNavItems.length) % settingsNavItems.length;
  settingsNavItems[settingsFocusIndex].classList.add('focused');
}

function handleSettingsKey(e) {
  const current = settingsNavItems[settingsFocusIndex];
  switch (e.key) {
    case 'Escape':
      closeSettings();
      break;
    case 'ArrowDown':
      setSettingsFocus(settingsFocusIndex + 1);
      break;
    case 'ArrowUp':
      setSettingsFocus(settingsFocusIndex - 1);
      break;
    case 'Enter':
      activateSettingsRow(current);
      break;
    case 'ArrowLeft':
    case 'ArrowRight':
      if (current.dataset.kind === 'select') {
        cycleSelect(displaySelect, e.key === 'ArrowRight' ? 1 : -1);
      }
      break;
  }
}

function activateSettingsRow(row) {
  const kind = row.dataset.kind;
  if (kind === 'toggle') {
    document.getElementById(row.dataset.target).click();
  } else if (kind === 'close') {
    closeSettings();
  }
  // 'select' rows are adjusted with Left/Right rather than Enter.
}

function cycleSelect(select, dir) {
  const i = (select.selectedIndex + dir + select.options.length) % select.options.length;
  select.selectedIndex = i;
  select.dispatchEvent(new Event('change'));
}

const PREFS_KEY = 'tv-launcher-prefs';
function loadPrefs() {
  try {
    return JSON.parse(localStorage.getItem(PREFS_KEY)) || { adblock: true, fullscreen: true };
  } catch {
    return { adblock: true, fullscreen: true };
  }
}
function savePrefs(prefs) {
  localStorage.setItem(PREFS_KEY, JSON.stringify(prefs));
}
let prefs = loadPrefs();

async function openSettings() {
  settingsOpen = true;
  settingsOverlay.classList.add('active');

  adblockToggle.classList.toggle('on', prefs.adblock);
  fullscreenToggle.classList.toggle('on', prefs.fullscreen);

  const displays = await window.launcher.getDisplays();
  displaySelect.innerHTML = displays.map(d => `<option value="${d.id}">${d.label}</option>`).join('');

  settingsNavItems.forEach(el => el.classList.remove('focused'));
  settingsFocusIndex = 0;
  settingsNavItems[0].classList.add('focused');
}

function closeSettings() {
  settingsOpen = false;
  settingsOverlay.classList.remove('active');
}

document.getElementById('btn-settings').addEventListener('click', openSettings);
document.getElementById('settings-close').addEventListener('click', closeSettings);
settingsOverlay.addEventListener('click', (e) => {
  if (e.target === settingsOverlay) closeSettings();
});

adblockToggle.addEventListener('click', () => {
  prefs.adblock = !prefs.adblock;
  adblockToggle.classList.toggle('on', prefs.adblock);
  savePrefs(prefs);
  window.launcher.setAdblock(prefs.adblock);
});

fullscreenToggle.addEventListener('click', () => {
  prefs.fullscreen = !prefs.fullscreen;
  fullscreenToggle.classList.toggle('on', prefs.fullscreen);
  savePrefs(prefs);
  window.launcher.toggleFullscreen();
});

// --- Clock (bottom bar) ---
function tickClock() {
  const now = new Date();
  let h = now.getHours();
  const ampm = h >= 12 ? 'PM' : 'AM';
  h = h % 12 || 12;
  const m = String(now.getMinutes()).padStart(2, '0');
  document.getElementById('clock-time').textContent = `${h}:${m}`;
  document.getElementById('clock-ampm').textContent = ampm;
}
tickClock();
setInterval(tickClock, 1000 * 30);

// --- Weather widget: IP-based location (no OS geolocation permission prompt,
// works headless/kiosk) + Open-Meteo for the actual forecast, both free/keyless. ---
async function loadWeather() {
  let lat = 52.52, lon = 13.405, city = 'Berlin'; // fallback if IP lookup fails
  try {
    const geo = await fetch('https://ipapi.co/json/').then(r => r.json());
    if (geo.latitude && geo.longitude) {
      lat = geo.latitude;
      lon = geo.longitude;
      city = geo.city || city;
    }
  } catch {
    // keep fallback
  }
  await renderWeather(lat, lon, city);
}

async function renderWeather(lat, lon, cityLabel) {
  document.getElementById('weather-city').textContent = cityLabel;
  try {
    const res = await fetch(
      `https://api.open-meteo.com/v1/forecast?latitude=${lat}&longitude=${lon}&current=temperature_2m,apparent_temperature`
    );
    const data = await res.json();
    document.getElementById('weather-temp').innerHTML =
      `${Math.round(data.current.temperature_2m)}°<span class="feels" id="weather-feels">Gefühlt ${Math.round(data.current.apparent_temperature)}°</span>`;
  } catch {
    document.getElementById('weather-temp').textContent = '–°';
  }
}
loadWeather();
setInterval(loadWeather, 30 * 60 * 1000);

// --- News list (first poster card) ---
async function loadNews() {
  const list = document.getElementById('news-list');
  try {
    const res = await fetch('https://news.google.com/rss?hl=de&gl=DE&ceid=DE:de');
    const text = await res.text();
    const doc = new DOMParser().parseFromString(text, 'text/xml');
    const items = Array.from(doc.querySelectorAll('item')).slice(0, 4);
    if (items.length === 0) throw new Error('empty feed');
    list.innerHTML = items.map(i => `<li>${i.querySelector('title').textContent}</li>`).join('');
  } catch {
    list.innerHTML = '<li>News konnten nicht geladen werden</li>';
  }
}
loadNews();
setInterval(loadNews, 15 * 60 * 1000);

// Spotify is currently just a login/remote-control tile (see openAction) --
// the on-screen now-playing widget and transport buttons were removed.
