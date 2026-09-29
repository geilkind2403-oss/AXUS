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
  if (action === 'mediathek') {
    openLibrary();
    return;
  }
  if (action === 'opensettings') {
    openSettings();
    return;
  }
  if (action === 'news') {
    contentOpen = true;
    window.launcher.openUrl('https://www.tagesschau.de');
    return;
  }
  if (action === 'randommovie') {
    playRandomMovie();
    return;
  }
  const url = ACTIONS[action];
  if (!url) return;
  contentOpen = true;
  window.launcher.openUrl(url, { asSmartTv: action === 'youtube' });
}

// Random-movie quick-play: currently not wired to any tile in the (empty
// placeholder) home grid, kept here so a future tile can call playRandomMovie().
let pickedRandomMovie = null;

async function refreshRandomMovieCell() {
  pickedRandomMovie = await window.launcher.getRandomMovie();
  const cell = document.getElementById('grid-4');
  if (!cell) return;
  if (pickedRandomMovie && pickedRandomMovie.icon) {
    const preload = new Image();
    preload.onload = () => {
      cell.style.backgroundImage = `url('${pickedRandomMovie.icon}')`;
      cell.classList.add('loaded');
    };
    preload.src = pickedRandomMovie.icon;
  }
}
refreshRandomMovieCell();

function escapeHtml(str) {
  const div = document.createElement('div');
  div.textContent = str;
  return div.innerHTML;
}

async function refreshNewsCell() {
  const list = document.getElementById('news-cell-list');
  if (!list) return;
  const headlines = await window.launcher.getNews();
  if (!headlines || !headlines.length) return;
  list.innerHTML = headlines.slice(0, 4).map(h => `<li>${escapeHtml(h)}</li>`).join('');
}
refreshNewsCell();
setInterval(refreshNewsCell, 5 * 60 * 1000);

function playRandomMovie() {
  if (!pickedRandomMovie) return;
  contentOpen = true;
  window.launcher.openMedia(
    pickedRandomMovie.fullPath,
    pickedRandomMovie.relativePath,
    getResumePosition(pickedRandomMovie.relativePath)
  );
}

async function handleSpotifyTileClick() {
  const loggedIn = await window.launcher.spotifyIsLoggedIn();
  if (loggedIn) {
    contentOpen = true;
    window.launcher.openUrl('https://open.spotify.com');
    return;
  }
  const result = await window.launcher.spotifyLogin();
  if (!result.ok) {
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

// --- Mediathek overlay: folder-tree browser over data/media, with resume
// ("weiterschauen") positions remembered per file. ---
let mediathekOpen = false;
const mediathekOverlay = document.getElementById('mediathek-overlay');
const mediathekList = document.getElementById('mediathek-list');
const mediathekBreadcrumb = document.getElementById('mediathek-breadcrumb');
let mediathekNavItems = [];
let mediathekFocusIndex = 0;
let mediathekCurrentDir = '';
let mediathekEntries = []; // [{type:'up'|'folder'|'file', ...}]

const RESUME_KEY = 'tv-launcher-resume';
function loadResumePositions() {
  try { return JSON.parse(localStorage.getItem(RESUME_KEY)) || {}; } catch { return {}; }
}
function saveResumePosition(relativePath, seconds) {
  const positions = loadResumePositions();
  positions[relativePath] = seconds;
  localStorage.setItem(RESUME_KEY, JSON.stringify(positions));
}
function getResumePosition(relativePath) {
  return loadResumePositions()[relativePath] || 0;
}

async function openMediathek(dir = '') {
  mediathekOpen = true;
  mediathekOverlay.classList.add('active');
  mediathekCurrentDir = dir;
  mediathekBreadcrumb.textContent = 'data/media' + (dir ? '/' + dir.replace(/\\/g, '/') : '');

  const { folders, files } = await window.launcher.listMediaDir(dir);
  const resumePositions = loadResumePositions();

  mediathekEntries = [];
  if (dir) mediathekEntries.push({ type: 'up' });
  for (const name of folders) mediathekEntries.push({ type: 'folder', name });
  for (const f of files) mediathekEntries.push({ type: 'file', ...f });

  if (mediathekEntries.length === 0) {
    mediathekList.innerHTML = '<li class="media-item-empty">Leerer Ordner. Leg Videos oder Unterordner in data/media ab.</li>';
  } else {
    mediathekList.innerHTML = mediathekEntries.map((entry, i) => {
      if (entry.type === 'up') {
        return `<li class="media-item mediathek-nav" tabindex="-1" data-index="${i}"><span class="media-item-icon">↑</span>..</li>`;
      }
      if (entry.type === 'folder') {
        return `<li class="media-item mediathek-nav" tabindex="-1" data-index="${i}"><span class="media-item-icon">📁</span>${entry.name}</li>`;
      }
      const resumeSec = resumePositions[entry.relativePath];
      const progressLabel = resumeSec ? `<span class="media-item-progress">weiter ab ${formatTime(resumeSec)}</span>` : '';
      return `<li class="media-item mediathek-nav" tabindex="-1" data-index="${i}"><span class="media-item-icon">▶</span>${entry.name}${progressLabel}</li>`;
    }).join('');
  }

  mediathekNavItems = Array.from(document.querySelectorAll('#mediathek-overlay .mediathek-nav'));
  mediathekNavItems.forEach(el => el.classList.remove('focused'));
  mediathekFocusIndex = 0;
  if (mediathekNavItems[0]) mediathekNavItems[0].classList.add('focused');

  // Only the freshly rendered list items need a fresh listener each time --
  // the two static buttons below get theirs once, outside this function.
  mediathekList.querySelectorAll('.mediathek-nav').forEach((el) => {
    el.addEventListener('click', () => activateMediathekItem(el));
  });
}

function formatTime(totalSeconds) {
  const h = Math.floor(totalSeconds / 3600);
  const m = Math.floor((totalSeconds % 3600) / 60);
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}h` : `${m}min`;
}

document.getElementById('mediathek-open-folder').addEventListener('click', () => window.launcher.openMediaFolder());
document.getElementById('mediathek-close').addEventListener('click', () => closeMediathek());

function closeMediathek() {
  mediathekOpen = false;
  mediathekOverlay.classList.remove('active');
}

function setMediathekFocus(i) {
  if (!mediathekNavItems.length) return;
  mediathekNavItems[mediathekFocusIndex].classList.remove('focused');
  mediathekFocusIndex = (i + mediathekNavItems.length) % mediathekNavItems.length;
  mediathekNavItems[mediathekFocusIndex].classList.add('focused');
}

function handleMediathekKey(e) {
  switch (e.key) {
    case 'Escape':
      closeMediathek();
      break;
    case 'Backspace':
      if (mediathekCurrentDir) goToParentMediaDir();
      break;
    case 'ArrowDown':
      setMediathekFocus(mediathekFocusIndex + 1);
      break;
    case 'ArrowUp':
      setMediathekFocus(mediathekFocusIndex - 1);
      break;
    case 'Enter':
      if (mediathekNavItems[mediathekFocusIndex]) activateMediathekItem(mediathekNavItems[mediathekFocusIndex]);
      break;
  }
}

function goToParentMediaDir() {
  const parts = mediathekCurrentDir.split(/[\\/]/).filter(Boolean);
  parts.pop();
  openMediathek(parts.join('/'));
}

function activateMediathekItem(el) {
  if (el.dataset.kind === 'close') {
    closeMediathek();
    return;
  }
  const entry = mediathekEntries[Number(el.dataset.index)];
  if (!entry) return;

  if (entry.type === 'up') {
    goToParentMediaDir();
  } else if (entry.type === 'folder') {
    const nextDir = mediathekCurrentDir ? `${mediathekCurrentDir}/${entry.name}` : entry.name;
    openMediathek(nextDir);
  } else if (entry.type === 'file') {
    closeMediathek();
    contentOpen = true;
    window.launcher.openMedia(entry.fullPath, entry.relativePath, getResumePosition(entry.relativePath));
  }
}

// mediaplayer.html reports its playback position through the main process;
// this is how "weiterschauen" (resume) positions get saved as you watch.
window.launcher.onMediaProgress((relativePath, seconds) => {
  saveResumePosition(relativePath, seconds);
});

mediathekOverlay.addEventListener('click', (e) => {
  if (e.target === mediathekOverlay) closeMediathek();
});

window.launcher.onContentClosed(() => {
  contentOpen = false;
});

document.addEventListener('keydown', (e) => {
  if (artModeActive) return; // resetIdleTimer's own listener exits art mode; swallow this keypress
  if (settingsOpen) {
    handleSettingsKey(e);
    return;
  }
  if (livetvOpen) {
    handleLivetvKey(e);
    return;
  }
  if (mediathekOpen) {
    handleMediathekKey(e);
    return;
  }
  if (libraryDetailOpen) {
    handleLibraryDetailKey(e);
    return;
  }
  if (libraryOpen) {
    handleLibraryKey(e);
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
        cycleSelect(document.getElementById(current.dataset.target), e.key === 'ArrowRight' ? 1 : -1);
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
  } else if (kind === 'action' && row.dataset.target === 'scaffold-media') {
    runScaffoldMedia();
  }
  // 'select' rows are adjusted with Left/Right rather than Enter.
}

async function runScaffoldMedia() {
  const status = document.getElementById('scaffold-media-status');
  const folders = await window.launcher.scaffoldMediaFolders();
  status.textContent = `Erstellt (${folders.length})`;
  setTimeout(() => { status.textContent = 'Enter'; }, 2500);
}

document.getElementById('scaffold-media-row').addEventListener('click', runScaffoldMedia);

function cycleSelect(select, dir) {
  const i = (select.selectedIndex + dir + select.options.length) % select.options.length;
  select.selectedIndex = i;
  select.dispatchEvent(new Event('change'));
}

const PREFS_KEY = 'tv-launcher-prefs';
const DEFAULT_PREFS = { adblock: true, fullscreen: true, artmodeTimeout: 30 };
function loadPrefs() {
  try {
    return { ...DEFAULT_PREFS, ...JSON.parse(localStorage.getItem(PREFS_KEY)) };
  } catch {
    return { ...DEFAULT_PREFS };
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

  document.getElementById('artmode-timeout-select').value = String(prefs.artmodeTimeout);

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

document.getElementById('artmode-timeout-select').addEventListener('change', (e) => {
  prefs.artmodeTimeout = Number(e.target.value);
  savePrefs(prefs);
  resetIdleTimer(); // apply the new duration immediately
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
// Murr, Baden-Württemberg -- fixed on purpose (IP-based geolocation was
// pointing at the wrong town, since it only reflects the ISP's routing
// point, not the actual device location).
const MURR_LAT = 48.9744;
const MURR_LON = 9.3714;

// WMO weather codes (used by Open-Meteo): 0-3 = clear/partly cloudy,
// everything with drizzle/rain/showers/thunder counts as "rainy" for
// picking which background video to loop.
function weatherCodeToCondition(code) {
  if (code === 0 || code === 1 || code === 2 || code === 3) return 'sunny';
  const rainyCodes = [51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 80, 81, 82, 95, 96, 99];
  if (rainyCodes.includes(code)) return 'rainy';
  return 'sunny'; // snow/fog etc: no dedicated video yet, default to sunny
}

let currentWeatherVideoCondition = null;
async function updateWeatherTileVideo(condition) {
  if (condition === currentWeatherVideoCondition) return;
  const video = document.getElementById('poster-weather-video');
  if (!video) return;
  const src = await window.launcher.getWeatherVideo(condition);
  if (src) {
    video.src = src;
    currentWeatherVideoCondition = condition;
    video.addEventListener('loadeddata', () => video.classList.add('loaded'), { once: true });
  }
}

async function loadWeather() {
  document.getElementById('weather-city').textContent = 'Murr';
  try {
    const res = await fetch(
      `https://api.open-meteo.com/v1/forecast?latitude=${MURR_LAT}&longitude=${MURR_LON}&current=temperature_2m,apparent_temperature,weathercode`
    );
    const data = await res.json();
    const temp = Math.round(data.current.temperature_2m);
    const feels = Math.round(data.current.apparent_temperature);
    const condition = weatherCodeToCondition(data.current.weathercode);

    document.getElementById('weather-temp').innerHTML =
      `${temp}°<span class="feels" id="weather-feels">Gefühlt ${feels}°</span>`;

    const tileTemp = document.getElementById('poster-weather-temp');
    const tileFeels = document.getElementById('poster-weather-feels');
    if (tileTemp) tileTemp.textContent = `${temp}°`;
    if (tileFeels) tileFeels.textContent = `Gefühlt ${feels}°`;
    updateWeatherTileVideo(condition);
  } catch {
    document.getElementById('weather-temp').textContent = '–°';
  }
}

loadWeather();
setInterval(loadWeather, 30 * 60 * 1000);

// Spotify is currently just a login/remote-control tile (see openAction) --
// the on-screen now-playing widget and transport buttons were removed.

// --- Art Mode: idle screensaver that loops a local video, skipping the
// first 10s and the last 60s (so it never shows the opening titles/credits). ---
const artmodeOverlay = document.getElementById('artmode-overlay');
const artmodeVideo = document.getElementById('artmode-video');
const ART_MODE_START = 10;
const ART_MODE_END_MARGIN = 60;

let idleTimer = null;
let artModeActive = false;

function anyOverlayOpen() {
  return contentOpen || settingsOpen || livetvOpen || mediathekOpen || libraryOpen || libraryDetailOpen;
}

async function enterArtMode() {
  if (anyOverlayOpen() || artModeActive) return;
  const src = await window.launcher.getArtModeVideo();
  if (!src) return; // no data/artmode.mp4 yet -- silently skip

  artModeActive = true;
  artmodeVideo.src = src;
  artmodeOverlay.classList.add('active');

  artmodeVideo.addEventListener('loadedmetadata', function onceReady() {
    artmodeVideo.removeEventListener('loadedmetadata', onceReady);
    artmodeVideo.currentTime = ART_MODE_START;
    artmodeVideo.play();
  });
}

function exitArtMode() {
  if (!artModeActive) return;
  artModeActive = false;
  artmodeOverlay.classList.remove('active');
  artmodeVideo.pause();
  artmodeVideo.removeAttribute('src');
  artmodeVideo.load();
}

artmodeVideo.addEventListener('timeupdate', () => {
  if (!artmodeVideo.duration) return;
  if (artmodeVideo.currentTime >= artmodeVideo.duration - ART_MODE_END_MARGIN) {
    artmodeVideo.currentTime = ART_MODE_START;
  }
});

function resetIdleTimer() {
  if (idleTimer) clearTimeout(idleTimer);
  if (artModeActive) exitArtMode();
  idleTimer = setTimeout(enterArtMode, prefs.artmodeTimeout * 1000);
}

['keydown', 'mousemove', 'mousedown'].forEach((evt) => {
  document.addEventListener(evt, resetIdleTimer);
});
resetIdleTimer();

// --- Library: the rich, poster-grid Mediathek home (reads the Name/Info/
// FSK/Personas/Madeby metadata folders next to each movie's video file). ---
let libraryOpen = false;
let libraryDetailOpen = false;
const libraryOverlay = document.getElementById('library-overlay');
const libraryDetailOverlay = document.getElementById('library-detail-overlay');
let libraryItems = [];
let libraryNavItems = [];
let libraryFocusIndex = 0;
let libraryDetailItem = null;

// Deterministic placeholder gradient per title (no cover art yet).
function gradientForTitle(title) {
  let hash = 0;
  for (let i = 0; i < title.length; i++) hash = title.charCodeAt(i) + ((hash << 5) - hash);
  const hue = Math.abs(hash) % 360;
  return `linear-gradient(160deg, hsl(${hue},65%,55%), hsl(${(hue + 40) % 360},55%,30%))`;
}

const GENRE_COLORS = {
  action: '#e5484d', drama: '#2f9e5e', komödie: '#e0a01c', comedy: '#e0a01c',
  romanze: '#d6499a', romance: '#d6499a', horror: '#7b5fd4', dokumentation: '#3b6dff'
};
function colorForGenre(genre) {
  if (!genre) return '#6c7180';
  return GENRE_COLORS[genre.toLowerCase()] || '#3b6dff';
}

function formatDuration(seconds) {
  if (!seconds) return '';
  const h = Math.floor(seconds / 3600);
  const m = Math.round((seconds % 3600) / 60);
  return h > 0 ? `${h}h ${m}min` : `${m}min`;
}

async function openLibrary() {
  libraryOpen = true;
  libraryOverlay.classList.add('active');
  libraryItems = await window.launcher.listLibrary();
  renderLibraryCards();
}

function renderLibraryCards() {
  const resumePositions = loadResumePositions();
  const continueItems = libraryItems.filter((it) => resumePositions[it.relativePath] > 0);

  document.getElementById('lib-row-continue').style.display = continueItems.length ? 'block' : 'none';
  document.getElementById('lib-cards-continue').innerHTML = continueItems.map((it, i) => libraryCardHtml(it, `c${i}`)).join('');
  document.getElementById('lib-cards-all').innerHTML = libraryItems.map((it, i) => libraryCardHtml(it, `a${i}`)).join('');

  libraryNavItems = Array.from(document.querySelectorAll('#library-overlay .lib-btn, #library-overlay .lib-card'));
  libraryNavItems.forEach((el) => el.classList.remove('focused'));
  libraryFocusIndex = 0;
  if (libraryNavItems[0]) libraryNavItems[0].classList.add('focused');

  document.querySelectorAll('#library-overlay .lib-card').forEach((el) => {
    el.addEventListener('click', () => openLibraryDetail(libraryItems[Number(el.dataset.itemIndex)]));
  });
}

function libraryCardHtml(item, key) {
  const badge = item.genre ? `<div class="lib-card-badge" style="background:${colorForGenre(item.genre)}">${item.genre}</div>` : '';
  const duration = item.duration ? `<div class="lib-card-duration">${formatDuration(item.duration)}</div>` : '';
  const itemIndex = libraryItems.indexOf(item);
  const posterStyle = item.icon
    ? `background-image: url('${item.icon}')`
    : `background: ${gradientForTitle(item.title)}`;
  return `
    <div class="lib-card" tabindex="-1" data-key="${key}" data-item-index="${itemIndex}">
      <div class="lib-card-poster" style="${posterStyle}">${badge}${duration}</div>
      <div class="lib-card-title">${item.title}</div>
    </div>`;
}

document.getElementById('library-close').addEventListener('click', closeLibrary);
document.getElementById('library-browse-folders').addEventListener('click', () => {
  closeLibrary();
  openMediathek();
});

function closeLibrary() {
  libraryOpen = false;
  libraryOverlay.classList.remove('active');
}

function setLibraryFocus(i) {
  if (!libraryNavItems.length) return;
  libraryNavItems[libraryFocusIndex].classList.remove('focused');
  libraryFocusIndex = (i + libraryNavItems.length) % libraryNavItems.length;
  libraryNavItems[libraryFocusIndex].classList.add('focused');
  libraryNavItems[libraryFocusIndex].scrollIntoView({ block: 'nearest', behavior: 'smooth' });
}

function handleLibraryKey(e) {
  switch (e.key) {
    case 'Escape':
      closeLibrary();
      break;
    case 'ArrowRight':
      setLibraryFocus(libraryFocusIndex + 1);
      break;
    case 'ArrowLeft':
      setLibraryFocus(libraryFocusIndex - 1);
      break;
    case 'ArrowDown':
      setLibraryFocus(libraryFocusIndex + 1);
      break;
    case 'ArrowUp':
      setLibraryFocus(libraryFocusIndex - 1);
      break;
    case 'Enter': {
      const el = libraryNavItems[libraryFocusIndex];
      if (!el) break;
      if (el.id === 'library-close') closeLibrary();
      else if (el.id === 'library-browse-folders') { closeLibrary(); openMediathek(); }
      else if (el.classList.contains('lib-card')) openLibraryDetail(libraryItems[Number(el.dataset.itemIndex)]);
      break;
    }
  }
}

function openLibraryDetail(item) {
  if (!item) return;
  libraryDetailItem = item;
  libraryDetailOpen = true;
  libraryDetailOverlay.classList.add('active');

  const hero = document.getElementById('library-detail-hero');
  if (item.banner) {
    hero.style.backgroundImage = `url('${item.banner}')`;
    hero.style.backgroundSize = 'cover';
    hero.style.backgroundPosition = 'center';
  } else {
    hero.style.backgroundImage = 'none';
    hero.style.background = gradientForTitle(item.title);
  }
  document.getElementById('library-detail-title').textContent = item.title;
  document.getElementById('library-detail-info').textContent = item.info || 'Keine Beschreibung vorhanden.';
  document.getElementById('library-detail-cast').textContent = item.cast || '–';
  document.getElementById('library-detail-crew').textContent = item.crew || '–';

  const badges = [];
  if (item.fsk) badges.push(`<span style="background:#1c1e26">${item.fsk}</span>`);
  if (item.genre) badges.push(`<span style="background:${colorForGenre(item.genre)}">${item.genre}</span>`);
  document.getElementById('library-detail-badges').innerHTML = badges.join('');

  const resumeSec = getResumePosition(item.relativePath);
  const playBtn = document.getElementById('library-detail-play');
  playBtn.textContent = resumeSec > 0 ? `▶ Weiterschauen ab ${formatTime(resumeSec)}` : '▶ Abspielen';
}

function closeLibraryDetail() {
  libraryDetailOpen = false;
  libraryDetailOverlay.classList.remove('active');
}

function handleLibraryDetailKey(e) {
  if (e.key === 'Escape') closeLibraryDetail();
  else if (e.key === 'Enter') playLibraryDetailItem();
}

function playLibraryDetailItem() {
  if (!libraryDetailItem) return;
  const item = libraryDetailItem;
  closeLibraryDetail();
  closeLibrary();
  contentOpen = true;
  window.launcher.openMedia(item.fullPath, item.relativePath, getResumePosition(item.relativePath));
}

document.getElementById('library-detail-close').addEventListener('click', closeLibraryDetail);
document.getElementById('library-detail-play').addEventListener('click', playLibraryDetailItem);
libraryDetailOverlay.addEventListener('click', (e) => {
  if (e.target === libraryDetailOverlay) closeLibraryDetail();
});
libraryOverlay.addEventListener('click', (e) => {
  if (e.target === libraryOverlay) closeLibrary();
});
