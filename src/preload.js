const { contextBridge, ipcRenderer, webFrame } = require('electron');

// Over the RAM budget (memory-main.js): the page drops what it keeps warm
// ('tv-memory-trim', boot.js), Blink drops its caches, V8 collects.
ipcRenderer.on('memory-trim', () => {
  window.dispatchEvent(new Event('tv-memory-trim'));
  webFrame.clearCache();
  setTimeout(() => { if (typeof gc === 'function') gc(); }, 0);
});

contextBridge.exposeInMainWorld('launcher', {
  openUrl: (url, opts) => ipcRenderer.invoke('open-url', url, opts),
  openLiveTv: (streamUrl) => ipcRenderer.invoke('open-livetv', streamUrl),
  closeContent: () => ipcRenderer.invoke('close-content'),
  onContentClosed: (cb) => ipcRenderer.on('content-closed', cb),
  getDisplays: () => ipcRenderer.invoke('get-displays'),
  setAdblock: (enabled) => ipcRenderer.invoke('set-adblock', enabled),
  toggleFullscreen: () => ipcRenderer.invoke('toggle-fullscreen'),
  // Einstellungen > Bild: the finished filter (picture.js) for the web apps in
  // the content view -- { filter, matrix, kernel } or null (= no filter).
  setPicture: (payload) => ipcRenderer.invoke('set-picture', payload),
  musicLibrary: (opts) => ipcRenderer.invoke('music-library', opts),
  musicLyrics: (trackId, url) => ipcRenderer.invoke('music-lyrics', trackId, url),
  getMusicServer: () => ipcRenderer.invoke('music-get-server'),
  setMusicServer: (url) => ipcRenderer.invoke('music-set-server', url),
  listMediaDir: (relativeDir) => ipcRenderer.invoke('list-media-dir', relativeDir),
  listLibrary: () => ipcRenderer.invoke('list-library'),
  getRandomMovie: () => ipcRenderer.invoke('get-random-movie'),
  getWeatherVideo: (condition) => ipcRenderer.invoke('get-weather-video', condition),
  getNews: () => ipcRenderer.invoke('get-news'),
  getWifiStatus: () => ipcRenderer.invoke('get-wifi-status'),
  getWifiNetworks: () => ipcRenderer.invoke('get-wifi-networks'),
  connectWifi: (ssid) => ipcRenderer.invoke('connect-wifi', ssid),
  connectWifiWithPassword: (ssid, password) => ipcRenderer.invoke('connect-wifi-with-password', ssid, password),
  getBluetoothStatus: () => ipcRenderer.invoke('get-bluetooth-status'),
  openBluetoothSettings: () => ipcRenderer.invoke('open-bluetooth-settings'),
  scanBluetoothDevices: (seconds) => ipcRenderer.invoke('scan-bluetooth-devices', seconds),
  pairBluetoothDevice: (deviceId) => ipcRenderer.invoke('pair-bluetooth-device', deviceId),
  openMedia: (fullPath, relativePath, resumeSeconds) =>
    ipcRenderer.invoke('open-media', fullPath, relativePath, resumeSeconds),
  onMediaProgress: (cb) => ipcRenderer.on('media-progress-update', (event, relativePath, seconds) => cb(relativePath, seconds)),
  openMediaFolder: () => ipcRenderer.invoke('open-media-folder'),
  scaffoldMediaFolders: () => ipcRenderer.invoke('scaffold-media-folders'),
  getArtModeVideo: () => ipcRenderer.invoke('get-artmode-video'),
  getRemoteInfo: () => ipcRenderer.invoke('get-remote-info'),
  getMediaServer: () => ipcRenderer.invoke('get-media-server'),
  getLiveTvLogos: () => ipcRenderer.invoke('livetv-logos'),
  setMediaServer: (url) => ipcRenderer.invoke('set-media-server', url),
  getSystemInfo: () => ipcRenderer.invoke('get-system-info'),
  getVolume: () => ipcRenderer.invoke('get-volume'),
  setVolume: (level) => ipcRenderer.invoke('set-volume', level),
  setMute: (muted) => ipcRenderer.invoke('set-mute', muted),
  getLiveTvPlaylist: () => ipcRenderer.invoke('livetv-playlist'),
  setSleepTimer: (minutes) => ipcRenderer.invoke('sleep-timer-set', minutes),
  onSleepTimerState: (cb) => ipcRenderer.on('sleep-timer-state', (event, state) => cb(state)),
  onSleepTimerFire: (cb) => ipcRenderer.on('sleep-timer-fire', () => cb()),
  onRemoteMessage: (cb) => ipcRenderer.on('remote-msg', (event, msg) => cb(msg)),
  remoteFeedback: (kind) => ipcRenderer.send('remote-feedback', kind),
  remoteNowPlaying: (np) => ipcRenderer.send('remote-now-playing', np)
});

// TV-OS system layer (tvos.js <-> tvos-main.js): notifications, network status,
// launch/error state of web apps, power menu, standby, home.
contextBridge.exposeInMainWorld('tvosBridge', {
  onToast: (cb) => ipcRenderer.on('tvos-toast', (event, payload) => cb(payload)),
  onNetwork: (cb) => ipcRenderer.on('tvos-network', (event, state) => cb(state)),
  getNetwork: () => ipcRenderer.invoke('tvos-get-network'),
  probeNetwork: () => ipcRenderer.invoke('tvos-probe-network'),
  onContentState: (cb) => ipcRenderer.on('tvos-content-state', (event, state) => cb(state)),
  retryContent: () => ipcRenderer.invoke('tvos-retry-content'),
  onHome: (cb) => ipcRenderer.on('tvos-home', () => cb()),
  onPowerMenu: (cb) => ipcRenderer.on('tvos-power-menu', () => cb()),
  onWake: (cb) => ipcRenderer.on('tvos-wake', () => cb()),
  setStandby: (on) => ipcRenderer.invoke('tvos-set-standby', on),
  setKeepAwake: (on) => ipcRenderer.invoke('tvos-set-keep-awake', on),
  restart: () => ipcRenderer.invoke('tvos-restart'),
  quit: () => ipcRenderer.invoke('tvos-quit')
});
