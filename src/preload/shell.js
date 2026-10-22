// Schnittstelle zwischen der eigenen Oberfläche (src/renderer) und dem Hauptprozess.
const { contextBridge, ipcRenderer } = require('electron');

const on = (channel) => (callback) => {
  const listener = (_e, payload) => callback(payload);
  ipcRenderer.on(channel, listener);
  return () => ipcRenderer.removeListener(channel, listener);
};

contextBridge.exposeInMainWorld('ask', {
  getState: () => ipcRenderer.invoke('state:get'),
  onState: on('state'),
  onFiles: on('files'),
  onTasks: on('tasks'),
  onAskLoaded: on('ask:loaded'),
  onAskLoadFailed: on('ask:load-failed'),
  onMenuChooseFolder: on('menu:choose-folder'),
  onMenuNewFolder: on('menu:new-folder'),
  onFolderError: on('folder:error'),
  chooseFolder: () => ipcRenderer.invoke('folder:choose'),
  newFolder: () => ipcRenderer.invoke('folder:new'),
  openFolder: (folder) => ipcRenderer.invoke('folder:open', folder),
  forgetFolder: (folder) => ipcRenderer.invoke('folder:forget', folder),
  folderMenu: () => ipcRenderer.invoke('folder:menu'),
  showFolder: () => ipcRenderer.invoke('folder:show'),
  restartTerminal: () => ipcRenderer.invoke('terminal:restart'),
  terminalLogs: () => ipcRenderer.invoke('terminal:logs'),
  acceptWarning: () => ipcRenderer.invoke('warning:accept'),
  setAskUrl: (url) => ipcRenderer.invoke('ask:set-url', url),
  connectAsk: () => ipcRenderer.invoke('ask:connect'),
  reloadAsk: () => ipcRenderer.invoke('ask:reload'),
  setSidebar: (open) => ipcRenderer.invoke('ui:sidebar', open),
  setAskBounds: (rect) => ipcRenderer.send('ui:ask-bounds', rect),
  setAskVisible: (visible) => ipcRenderer.send('ui:ask-visible', visible),
  copy: (text) => ipcRenderer.invoke('clipboard:write', text),
  files: () => ipcRenderer.invoke('workspace:files'),
  tasks: () => ipcRenderer.invoke('workspace:tasks'),
  preview: (rel) => ipcRenderer.invoke('file:preview', rel),
  openFile: (rel) => ipcRenderer.invoke('file:open', rel),
  toPdf: (rel) => ipcRenderer.invoke('file:pdf', rel),
  showFile: (rel) => ipcRenderer.invoke('file:show', rel),
});
