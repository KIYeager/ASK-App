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

// Datei des Arbeitsordners über das Protokoll askfile: (main.js). v: neue Fassung statt Zwischenspeicher
export const fileUrl = (p, v = '') => `askfile://datei/${String(p).replaceAll('\\', '/').split('/').map(encodeURIComponent).join('/')}${v ? `?v=${v}` : ''}`;

// Kurzbezeichnung im farbigen Dateisymbol
export const fileBadge = (name, dir = false) => (dir ? 'DIR' : (extOf(name) || 'Datei').slice(0, 4).toUpperCase());
