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
  onSpotifyLoggedIn: (cb) => ipcRenderer.on('spotify-logged-in', cb),
  listMediaDir: (relativeDir) => ipcRenderer.invoke('list-media-dir', relativeDir),
  listLibrary: () => ipcRenderer.invoke('list-library'),
  getRandomMovie: () => ipcRenderer.invoke('get-random-movie'),
  getWeatherVideo: (condition) => ipcRenderer.invoke('get-weather-video', condition),
  getNews: () => ipcRenderer.invoke('get-news'),
  openMedia: (fullPath, relativePath, resumeSeconds) =>
    ipcRenderer.invoke('open-media', fullPath, relativePath, resumeSeconds),
  onMediaProgress: (cb) => ipcRenderer.on('media-progress-update', (event, relativePath, seconds) => cb(relativePath, seconds)),
  openMediaFolder: () => ipcRenderer.invoke('open-media-folder'),
  scaffoldMediaFolders: () => ipcRenderer.invoke('scaffold-media-folders'),
  getArtModeVideo: () => ipcRenderer.invoke('get-artmode-video')
});
