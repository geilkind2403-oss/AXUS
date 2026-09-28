const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('launcher', {
  openUrl: (url, opts) => ipcRenderer.invoke('open-url', url, opts),
  openLiveTv: (streamUrl) => ipcRenderer.invoke('open-livetv', streamUrl),
  closeContent: () => ipcRenderer.invoke('close-content'),
  onContentClosed: (cb) => ipcRenderer.on('content-closed', cb),
  getDisplays: () => ipcRenderer.invoke('get-displays'),
  setAdblock: (enabled) => ipcRenderer.invoke('set-adblock', enabled),
  toggleFullscreen: () => ipcRenderer.invoke('toggle-fullscreen'),
  spotifyIsLoggedIn: () => ipcRenderer.invoke('spotify-is-logged-in'),
  spotifyLogin: () => ipcRenderer.invoke('spotify-login'),
  spotifyNowPlaying: () => ipcRenderer.invoke('spotify-now-playing'),
  spotifyControl: (action) => ipcRenderer.invoke('spotify-control', action),
  onSpotifyLoggedIn: (cb) => ipcRenderer.on('spotify-logged-in', cb)
});
