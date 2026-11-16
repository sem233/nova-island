const { contextBridge, ipcRenderer } = require('electron');

const on = (channel) => (cb) => {
  const sub = (_e, data) => cb(data);
  ipcRenderer.on(channel, sub);
  return () => ipcRenderer.removeListener(channel, sub);
};

contextBridge.exposeInMainWorld('widgetsBridge', {
  getFeatures: () => ipcRenderer.invoke('widgets-features'),
  getPrayer: () => ipcRenderer.invoke('widgets-prayer'),
  onFeatures: on('features'),
  onPrayer: on('prayer'),
  resetPos: () => ipcRenderer.send('widgets-reset-pos'),
});
