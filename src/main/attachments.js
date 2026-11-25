// Anhänge aus dem Chat (eingefügt, hineingezogen oder mit «+» gewählt). Sie landen versteckt unter
// .ask/anhaenge/<Chat>/, damit das Modell sie mit seinen Werkzeugen lesen kann; sie verschwinden mit
// dem Chat oder nach 14 Tagen (cleanup.js). Bilder gehen zusätzlich direkt an das Modell.
const fs = require('node:fs');
const path = require('node:path');

const INBOX = '.ask/anhaenge';
const MAX_FILES = 10;
const MAX_SIZE = 50 * 1024 * 1024;
const MAX_IMAGE_SIZE = 20 * 1024 * 1024; // grössere Bilder nur als Datei
const MAX_IMAGE_SIDE = 2048; // grössere Bilder werden für das Modell verkleinert
const IMAGE_TYPES = { '.png': 'image/png', '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.gif': 'image/gif', '.webp': 'image/webp' };

const pad = (n) => String(n).padStart(2, '0');
const stamp = (d = new Date()) => `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`;

// Dateiname ohne Pfad und ohne Zeichen, die Windows nicht erlaubt. Eingefügte Bildschirmfotos
// heissen in Chromium «image.png»: dann ein Name mit Datum und Zeit.
function cleanName(name, type = '') {
  let base = path.posix.basename(String(name || '').replace(/\\/g, '/')); // posix: «a:b» ist kein Laufwerk
  base = base.replace(/[<>:"/\\|?*\u0000-\u001f]/g, '_').replace(/^[.\s]+|[.\s]+$/g, '');
  if (!base || /^image\.(png|jpe?g|gif|webp|bmp)$/i.test(base)) {
    const ext = path.extname(base) || ({ 'image/jpeg': '.jpg', 'image/gif': '.gif', 'image/webp': '.webp' }[type] || '.png');
    base = `bild-${stamp()}${ext.toLowerCase()}`;
  }
  if (base.length > 120) base = base.slice(0, 120 - path.extname(base).length) + path.extname(base);
  // Gerätenamen von Windows (nul.txt landet sonst im Nichts)
  if (/^(con|prn|aux|nul|com[0-9]|lpt[0-9])(\.|$)/i.test(base)) base = `_${base}`;
  return base;
}

// Freier Name: «bericht.docx», sonst «bericht (2).docx» usw.
function freeName(dir, name) {
  const ext = path.extname(name);
  const stem = name.slice(0, name.length - ext.length);
  let candidate = name;
  for (let i = 2; fs.existsSync(path.join(dir, candidate)); i++) candidate = `${stem} (${i})${ext}`;
  return candidate;
}

const isImage = (name) => Object.hasOwn(IMAGE_TYPES, path.extname(name).toLowerCase());

// files: [{ name, type, data (Uint8Array) }] → [{ path: '.ask/anhaenge/<chat>/…', name, size, image }]
function saveAttachments(folder, files, chat) {
  const list = Array.isArray(files) ? files : [];
  if (list.length > MAX_FILES) throw new Error(`Höchstens ${MAX_FILES} Anhänge auf einmal.`);
  if (!/^[\w-]+$/.test(String(chat))) throw new Error('Ungültiger Chat');
  const sub = `${INBOX}/${chat}`;
  const dir = path.join(folder, ...sub.split('/'));
  fs.mkdirSync(dir, { recursive: true });
  return list.map((f) => {
    const data = Buffer.from(f.data || []);
    const original = String(f.name || '');
    if (data.length > MAX_SIZE) throw new Error(`${original || 'Ein Anhang'} ist grösser als 50 MB.`);
    const name = freeName(dir, cleanName(original, f.type));
    fs.writeFileSync(path.join(dir, name), data, { flag: 'wx' });
    return { path: `${sub}/${name}`, name, size: data.length, image: isImage(name) };
  });
}

// Bild als data-URL für das Modell. resize(buffer, ext) darf ein kleineres { data, type } liefern
// (in der App mit nativeImage), sonst geht das Bild unverändert.
function imageUrl(folder, attachment, resize = null) {
  try {
    const full = path.join(folder, attachment.path);
    const ext = path.extname(full).toLowerCase();
    if (!IMAGE_TYPES[ext]) return null;
    let data = fs.readFileSync(full);
    let type = IMAGE_TYPES[ext];
    const smaller = resize ? resize(data, ext) : null;
    if (smaller) ({ data, type } = smaller);
    if (data.length > MAX_IMAGE_SIZE) return null;
    return `data:${type};base64,${data.toString('base64')}`;
  } catch {
    return null; // Datei inzwischen gelöscht oder unlesbar
  }
}

// Verkleinern mit Electrons nativeImage (PNG und JPEG); null, wenn nicht nötig oder nicht möglich.
function nativeResize(nativeImage) {
  return (data, ext) => {
    if (!['.png', '.jpg', '.jpeg'].includes(ext)) return null;
    const img = nativeImage.createFromBuffer(data);
    if (img.isEmpty()) return null;
    const { width, height } = img.getSize();
    if (Math.max(width, height) <= MAX_IMAGE_SIDE) return null;
    const scaled = width >= height ? img.resize({ width: MAX_IMAGE_SIDE, quality: 'good' }) : img.resize({ height: MAX_IMAGE_SIDE, quality: 'good' });
    return ext === '.png' ? { data: scaled.toPNG(), type: 'image/png' } : { data: scaled.toJPEG(85), type: 'image/jpeg' };
  };
}

module.exports = { saveAttachments, imageUrl, nativeResize, cleanName, INBOX, MAX_FILES, MAX_SIZE, MAX_IMAGE_SIDE };
