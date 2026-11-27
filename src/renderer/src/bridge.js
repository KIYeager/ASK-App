// Schnittstelle zwischen der eigenen Oberfläche und dem Hauptprozess (Tauri, src-tauri). Ersetzt den
// Preload von Electron (früher src/preload/shell.js): dieselben Funktionen unter window.ask, darum ändert sich in
// der Oberfläche nichts. Aufrufe gehen über den Befehl «ipc» an src-tauri/src/ipc.rs, Ereignisse kommen als
// Tauri-Ereignisse mit demselben Namen wie früher die Kanäle von Electron.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

// Binärdaten (Anhänge, importierte Dateien) gehen als Base64, sonst würde JSON aus jedem Uint8Array ein Objekt
// mit einem Feld pro Byte machen. Der Hauptprozess erkennt { $b64 } wieder.
function toBase64(bytes) {
  let bin = '';
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) bin += String.fromCharCode.apply(null, bytes.subarray(i, i + CHUNK));
  return btoa(bin);
}
function pack(value) {
  if (value instanceof Uint8Array) return { $b64: toBase64(value) };
  if (value instanceof ArrayBuffer) return { $b64: toBase64(new Uint8Array(value)) };
  if (Array.isArray(value)) return value.map(pack);
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, pack(v)]));
  return value === undefined ? null : value;
}

// Wie ipcRenderer.invoke: Fehler des Hauptprozesses kommen als abgelehntes Promise
const call = (channel, ...args) => invoke('ipc', { channel, args: pack(args) });
// Wie ipcRenderer.send: ohne Antwort (Grösse und Sichtbarkeit der Webansichten)
const fire = (channel, ...args) => { invoke('ipc', { channel, args: pack(args) }).catch(() => {}); };

// Wie ipcRenderer.on: gibt eine Funktion zum Abmelden zurück
const on = (channel) => (callback) => {
  let stop = null;
  let cancelled = false;
  listen(channel, (event) => callback(event.payload)).then((unlisten) => {
    if (cancelled) unlisten(); else stop = unlisten;
  });
  return () => { cancelled = true; if (stop) stop(); };
};

export function install() {
  // Eigene Protokolle: unter Windows (WebView2) http://<name>.localhost/, sonst <name>://
  window.__ASK_FILE_BASE__ = /Windows/i.test(navigator.userAgent) ? 'http://askfile.localhost/datei/' : 'askfile://datei/';
  window.ask = {
  getState: () => call('state:get'),
  onState: on('state'),
  onFiles: on('files'),
  onMenuChooseFolder: on('menu:choose-folder'),
  onMenuNewFolder: on('menu:new-folder'),
  onMenuGithub: on('menu:github'),
  onFolderError: on('folder:error'),
  chooseFolder: () => call('folder:choose'),
  newFolder: (name) => call('folder:new', name),
  projectMenu: (folder) => call('project:menu', folder),
  projectUpdate: (folder, patch) => call('project:update', folder, patch),
  onProjectRename: on('project:rename'),
  onGroupsChanged: on('chat:groups-changed'),
  setSettings: (values) => call('settings:set', values),
  listDir: (rel) => call('workspace:list', rel),
  findFiles: (query) => call('workspace:find', query),
  importFiles: (rel, files) => call('workspace:import', rel, files),
  openFolder: (folder) => call('folder:open', folder),
  forgetFolder: (folder) => call('folder:forget', folder),
  folderMenu: () => call('folder:menu'),
  showFolder: () => call('folder:show'),
  restartTerminal: () => call('terminal:restart'),
  terminalLogs: () => call('terminal:logs'),
  acceptWarning: () => call('warning:accept'),
  setAskUrl: (url) => call('ask:set-url', url),
  connectAsk: () => call('ask:connect'),
  logout: () => call('ask:logout'),
  askLoginWindow: () => call('ask:login-window'),
  askOpenBrowser: () => call('ask:open-browser'),
  askUseToken: (token) => call('ask:use-token', token),
  models: () => call('ask:models'),
  setModel: (id) => call('model:set', id),
  setWebSearch: (on) => call('web:set', on),
  setChatMemory: (on) => call('memory:set', on),
  setThinking: (level) => call('thinking:set', level),
  chatSend: (text, files = [], kept = []) => call('chat:send', text, files, kept),
  chatRewind: () => call('chat:rewind'),
  chatRename: (folder, id, title) => call('chat:rename', folder, id, title),
  chatPin: (folder, id, on) => call('chat:pin', folder, id, on),
  chatSearch: (query) => call('chat:search', query),
  whatsNewSeen: () => call('whatsnew:seen'),
  setPrefs: (values) => call('prefs:set', values),
  usageGet: (withAsk = true) => call('usage:get', withAsk),
  chatUndo: (snapshot) => call('chat:undo', snapshot),
  chatStop: () => call('chat:stop'),
  chatStopChat: (folder, id) => call('chat:stop-chat', folder, id),
  chatApprove: (id, answer) => call('chat:approve', id, answer),
  chatAnswer: (id, text) => call('chat:answer', id, text),
  chatGoal: (text) => call('chat:goal', text),
  githubStatus: () => call('github:status'),
  githubConnect: (token) => call('github:connect', token),
  githubDisconnect: () => call('github:disconnect'),
  githubUntrust: (repo) => call('github:untrust', repo),
  githubBranches: () => call('github:branches'),
  githubPulls: () => call('github:pulls'),
  githubPull: () => call('github:pull'),
  githubClone: (repo) => call('github:clone', repo),
  githubPush: (message, overwrite = false) => call('github:push', message, overwrite),
  githubCreate: (name, privat = true) => call('github:create', name, privat),
  githubSwitch: (branch) => call('github:switch', branch),
  githubBranchCreate: (branch) => call('github:branch-create', branch),
  githubPrCreate: (title, body) => call('github:pr-create', title, body),
  chatNew: (folder) => call('chat:new', folder),
  chatGroups: () => call('chat:groups'),
  chatOpen: (folder, id) => call('chat:open', folder, id),
  chatDelete: (folder, id, title) => call('chat:delete', folder, id, title),
  chatTranscript: () => call('chat:transcript'),
  onChatEvent: on('chat:event'),
  onChatChanged: on('chat:changed'),
  reloadAsk: () => call('ask:reload'),
  setSidebar: (open) => call('ui:sidebar', open),
  setChatbar: (open) => call('ui:chatbar', open),
  setAskBounds: (rect) => fire('ui:ask-bounds', rect),
  setAskVisible: (visible) => fire('ui:ask-visible', visible),
  setBrowserBounds: (rect) => fire('ui:browser-bounds', rect),
  setBrowserVisible: (visible) => fire('ui:browser-visible', visible),
  servers: () => call('servers:list'),
  onServers: on('servers'),
  serverStart: (command, name) => call('servers:start', command, name),
  serverStop: (id) => call('servers:stop', id),
  serverRemove: (id) => call('servers:remove', id),
  serverOutput: (id) => call('servers:output', id),
  browserShow: () => call('browser:show'),
  browserGo: (text) => call('browser:go', text),
  browserBack: () => call('browser:back'),
  browserForward: () => call('browser:forward'),
  browserClear: () => call('browser:clear'),
  onBrowserState: on('browser:state'),
  onBrowserNotice: on('browser:notice'),
  browserReload: () => call('browser:reload'),
  browserClose: () => call('browser:close'),
  copy: (text) => call('clipboard:write', text),
  openLink: (url) => call('link:open', url),
  files: () => call('workspace:files'),
  preview: (rel) => call('file:preview', rel),
  fileData: (rel) => call('file:data', rel),
  openFile: (rel) => call('file:open', rel),
  toPdf: (rel, preview) => call('file:pdf', rel, !!preview),
  showFile: (rel) => call('file:show', rel),
  updateAction: () => call('update:action'),
  onUiAsk: on('ui:ask'),
  onUiMenu: on('ui:menu'),
  uiAnswer: (id, value) => call('ui:answer', id, value),
  updateLater: () => call('update:later'),
  setBetaUpdates: (on, tell) => call('update:beta', on, tell),
  // Gedächtnis und eigene Skills (src/main/knowledge.js)
  memoryList: () => call('memory:list'),
  memoryAdd: (item) => call('memory:add', item),
  memoryUpdate: (id, patch) => call('memory:update', id, patch),
  memoryRemove: (id) => call('memory:remove', id),
  memoryClear: () => call('memory:clear'),
  setMemoryEnabled: (on) => call('memory:enable', on),
  onMemoryChanged: on('memory:changed'),
  skillsList: () => call('skills:list'),
  skillsGet: (name, source = '') => call('skills:get', name, source),
  skillsEnable: (name, on) => call('skills:enable', name, on),
  skillsSave: (skill, previous) => call('skills:save', skill, previous),
  skillsRemove: (name) => call('skills:remove', name),
  skillsExport: (name) => call('skills:export', name),
  skillsImport: () => call('skills:import'),
  // Mods und Kataloge (ENTSCHEIDE E71, E72)
  modsList: () => call('mods:list'),
  modsGet: (id) => call('mods:get', id),
  modsActive: () => call('mods:active'),
  modsEnable: (id, on) => call('mods:enable', id, on),
  modsOffAll: () => call('mods:off-all'),
  modsRemove: (id) => call('mods:remove', id),
  modsImport: () => call('mods:import'),
  modsExample: (id) => call('mods:example', id),
  modsFolder: () => call('mods:folder'),
  onModsChanged: on('mods:changed'),
  catalogSources: (list) => call('catalog:sources', list === undefined ? null : list),
  catalogList: (repo) => call('catalog:list', repo),
  catalogInstall: (repo, kind, path) => call('catalog:install', repo, kind, path),
  catalogLink: (url) => call('catalog:link', url),
  // Fenster ohne Rahmen von Windows (E73)
  winMinimize: () => call('win:minimize'),
  winMaximize: () => call('win:maximize'),
  winClose: () => call('win:close'),
  winDrag: () => fire('win:drag'),
  winState: () => call('win:state'),
  onWinState: on('win:state'),
};
}
