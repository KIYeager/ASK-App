// Kleine Helfer für die Anzeige (Zahlen und Zeiten wie in der Schweiz)
export const baseName = (p) => String(p || '').split(/[\\/]/).filter(Boolean).pop() || p;

export function extOf(name) {
  const m = /\.([a-z0-9]+)$/i.exec(String(name || ''));
  return m ? m[1].toLowerCase() : '';
}

export function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export function formatTime(ms) {
  const diff = (Date.now() - ms) / 1000;
  if (diff < 60) return 'gerade eben';
  if (diff < 3600) return `vor ${Math.round(diff / 60)} Min.`;
  const d = new Date(ms);
  const sameDay = d.toDateString() === new Date().toDateString();
  return sameDay ? d.toLocaleTimeString('de-CH', { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleDateString('de-CH', { day: '2-digit', month: '2-digit', year: 'numeric' });
}

export function shortTime(ms) {
  const diff = (Date.now() - ms) / 1000;
  if (diff < 60) return 'jetzt';
  if (diff < 3600) return `${Math.round(diff / 60)} Min.`;
  if (diff < 86400) return `${Math.round(diff / 3600)} Std.`;
  if (diff < 7 * 86400) return `${Math.round(diff / 86400)} T.`;
  return new Date(ms).toLocaleDateString('de-CH', { day: '2-digit', month: '2-digit' });
}

// Datei des Arbeitsordners über das Protokoll askfile: (src-tauri/src/protocol.rs). Unter Windows liefert Tauri
// eigene Protokolle als http://askfile.localhost/ aus (bridge.js setzt das). v: neue Fassung statt Zwischenspeicher
export const fileBase = () => (typeof window !== 'undefined' && window.__ASK_FILE_BASE__) || 'askfile://datei/';
export const fileUrl = (p, v = '') => `${fileBase()}${String(p).replaceAll('\\', '/').split('/').map(encodeURIComponent).join('/')}${v ? `?v=${v}` : ''}`;

// Kurzbezeichnung im farbigen Dateisymbol
export const fileBadge = (name, dir = false) => (dir ? 'DIR' : (extOf(name) || 'Datei').slice(0, 4).toUpperCase());

// Zahlen mit Apostroph als Tausendertrennzeichen (1’234), ab einer Million kurz («1.2 Mio.»)
export function formatCount(n) {
  const v = Math.round(Number(n) || 0);
  if (v >= 1e6) return `${(v / 1e6).toLocaleString('de-CH', { maximumFractionDigits: 1 })} Mio.`;
  return v.toLocaleString('de-CH');
}

// Dauer kurz: «45 s», «16 min 40 s», «3 h 5 min» (dieselbe Funktion wie im Chat)
export { dauer as formatDuration } from './chatModel.mjs';
