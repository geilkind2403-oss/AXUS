const { contextBridge, ipcRenderer } = require('electron');

// Attached to the shared content view for every site it opens (YouTube,
// Spotify, live TV, local media...). Only mediaplayer.html actually uses
// this -- it's a no-op, harmless global on everything else.
contextBridge.exposeInMainWorld('mediaPlayer', {
  reportProgress: (relativePath, seconds) => ipcRenderer.send('media-progress', relativePath, seconds)
});
