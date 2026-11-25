// Eigene Skills (ENTSCHEIDE E51): Anleitungen der Person für wiederkehrende Aufträge, wie Skills in Claude.
// Jeder Skill ist eine Datei skills/<name>/SKILL.md in den Daten der App (userData), mit Kopf
// («name», «description») und der Anleitung darunter. Teilen geht über Export und Import der Datei.
const fs = require('node:fs');
const path = require('node:path');

const MAX_FILE = 100 * 1024; // Import und Speichern: höchstens 100 KB
const MAX_DESCRIPTION = 300;
const MAX_NAME = 40;
const PROMPT_MAX = 1800; // Liste im Systemprompt
const PROMPT_DESCRIPTION = 160;
const MAX_SKILLS = 100;
// Befehle der App: ein Skill mit diesem Namen liesse sich mit /name nicht aufrufen
const RESERVED = new Set(['neu', 'projekt', 'goal', 'suchen', 'ordner', 'explorer', 'dateien', 'github', 'modell', 'denken', 'web', 'chats',
  'einstellungen', 'update', 'beta', 'neustart', 'abmelden', 'gedaechtnis', 'skills', 'hilfe']);

// Name → Ordnername: klein, Umlaute ausgeschrieben, nur a-z, 0-9 und Bindestriche
function slugify(name) {
  return String(name ?? '').toLowerCase()
    .replace(/ä/g, 'ae').replace(/ö/g, 'oe').replace(/ü/g, 'ue').replace(/ß/g, 'ss')
    .normalize('NFKD').replace(/[\u0300-\u036f]/g, '')
    .replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').replace(/-{2,}/g, '-')
    .slice(0, MAX_NAME).replace(/-+$/, '');
}

function checkName(slug) {
  if (!slug) return 'Der Name fehlt (Buchstaben oder Ziffern).';
  if (RESERVED.has(slug)) return `«/${slug}» ist schon ein Befehl der App. Wähle einen anderen Namen.`;
  return '';
}

const oneLine = (t, max) => String(t ?? '').replace(/\s+/g, ' ').trim().slice(0, max);

// Kopf wie bei Claude-Skills: --- name: … description: … --- und danach die Anleitung
function parse(text) {
  const raw = String(text ?? '').replace(/^\uFEFF/, '').replace(/\r\n?/g, '\n');
  const m = /^---\n([\s\S]*?)\n---[ \t]*(?:\n|$)([\s\S]*)$/.exec(raw);
  if (!m) throw new Error('Die Datei hat keinen Kopf mit «name» und «description» zwischen zwei Zeilen «---».');
  const head = {};
  let key = '';
  for (const line of m[1].split('\n')) {
    const kv = /^([A-Za-z_][\w-]*)\s*:\s*(.*)$/.exec(line);
    if (kv) {
      key = kv[1].toLowerCase();
      head[key] = kv[2].trim();
    } else if (key && /^\s+\S/.test(line)) {
      head[key] = `${head[key]} ${line.trim()}`.trim(); // eingerückte Fortsetzung
    }
  }
  const unquote = (v) => String(v ?? '').replace(/^(['"])([\s\S]*)\1$/, '$2').replace(/^[>|][-+]?\s*/, '').trim();
  const name = unquote(head.name);
  const description = oneLine(unquote(head.description), MAX_DESCRIPTION);
  const body = m[2].replace(/^\n+/, '').replace(/\s+$/, '');
  if (!name) throw new Error('Im Kopf fehlt «name».');
  if (!description) throw new Error('Im Kopf fehlt «description».');
  if (!body.trim()) throw new Error('Die Anleitung unter dem Kopf ist leer.');
  return { name, description, body };
}

function format({ name, description, body }) {
  const desc = oneLine(description, MAX_DESCRIPTION);
  return `---\nname: ${name}\ndescription: ${desc}\n---\n\n${String(body ?? '').replace(/\r\n?/g, '\n').trim()}\n`;
}

class Skills {
  constructor({ dir }) {
    this.dir = dir;
  }

  fileOf(slug) {
    return path.join(this.dir, slug, 'SKILL.md');
  }

  // Alle Skills, nach Name. Kaputte Dateien fallen weg.
  list() {
    let names = [];
    try { names = fs.readdirSync(this.dir, { withFileTypes: true }).filter((d) => d.isDirectory()).map((d) => d.name); } catch { return []; }
    const out = [];
    for (const slug of names) {
      if (slugify(slug) !== slug) continue;
      const s = this.get(slug);
      if (s) out.push({ slug: s.slug, name: s.name, description: s.description, updated: s.updated, size: s.size });
    }
    return out.sort((a, b) => a.slug.localeCompare(b.slug));
  }

  get(name) {
    const slug = slugify(name);
    if (!slug) return null;
    const file = this.fileOf(slug);
    try {
      const st = fs.statSync(file);
      if (st.size > MAX_FILE) return null;
      const parsed = parse(fs.readFileSync(file, 'utf8'));
      return { slug, name: slug, description: parsed.description, body: parsed.body, updated: st.mtimeMs, size: st.size };
    } catch {
      return null;
    }
  }

  // Speichern (neu oder geändert). previous: bisheriger Name, wenn der Skill umbenannt wird.
  // overwrite: einen anderen Skill mit demselben Namen ersetzen.
  save({ name, description, body }, { previous = '', overwrite = false } = {}) {
    const slug = slugify(name);
    const problem = checkName(slug);
    if (problem) throw new Error(problem);
    const desc = oneLine(description, MAX_DESCRIPTION);
    if (!desc) throw new Error('Die Beschreibung fehlt: Wann soll ASK diesen Skill benutzen?');
    if (!String(body ?? '').trim()) throw new Error('Die Anleitung fehlt.');
    const text = format({ name: slug, description: desc, body });
    if (Buffer.byteLength(text) > MAX_FILE) throw new Error('Der Skill ist grösser als 100 KB.');
    const prev = slugify(previous);
    if (!overwrite && slug !== prev && fs.existsSync(this.fileOf(slug))) throw new Error(`Es gibt schon einen Skill «${slug}».`);
    if (!fs.existsSync(this.fileOf(slug)) && this.list().length >= MAX_SKILLS) throw new Error(`Höchstens ${MAX_SKILLS} Skills.`);
    fs.mkdirSync(path.dirname(this.fileOf(slug)), { recursive: true });
    const file = this.fileOf(slug);
    fs.writeFileSync(`${file}.tmp`, text);
    fs.renameSync(`${file}.tmp`, file);
    if (prev && prev !== slug) this.remove(prev);
    return this.get(slug);
  }

  remove(name) {
    const slug = slugify(name);
    if (!slug || !fs.existsSync(this.fileOf(slug))) return false;
    fs.rmSync(path.join(this.dir, slug), { recursive: true, force: true });
    return true;
  }

  // Text der Datei für den Export
  exportText(name) {
    const s = this.get(name);
    return s ? format({ name: s.slug, description: s.description, body: s.body }) : null;
  }

  // Import aus einer Datei: Grösse, Text, Kopf und Name prüfen. Liefert { name, description, body } (noch nicht gespeichert).
  static readImport(buffer) {
    const buf = Buffer.isBuffer(buffer) ? buffer : Buffer.from(String(buffer ?? ''));
    if (buf.length > MAX_FILE) throw new Error('Die Datei ist grösser als 100 KB.');
    if (!buf.length) throw new Error('Die Datei ist leer.');
    if (buf.includes(0)) throw new Error('Das ist keine Textdatei.');
    const text = buf.toString('utf8');
    if (text.includes('\uFFFD')) throw new Error('Die Datei ist nicht in UTF-8 gespeichert.');
    const parsed = parse(text);
    const slug = slugify(parsed.name);
    const problem = checkName(slug);
    if (problem) throw new Error(problem);
    return { name: slug, description: parsed.description, body: parsed.body };
  }

  // Liste für den Systemprompt: Name und Beschreibung, kurz
  prompt(max = PROMPT_MAX) {
    const list = this.list();
    if (!list.length) return '';
    let text = '\n\n# Eigene Skills\nAnleitungen der Person für wiederkehrende Aufträge. Passt ein Skill zum Auftrag, lies ihn zuerst mit skill_lesen und halte dich daran.';
    let shown = 0;
    for (const s of list) {
      const line = `\n- ${s.slug}: ${oneLine(s.description, PROMPT_DESCRIPTION)}`;
      if (text.length + line.length > max) break;
      text += line;
      shown++;
    }
    if (shown < list.length) text += `\n- … und ${list.length - shown} weitere (skill_lesen mit dem Namen)`;
    return text;
  }
}

// «/name Text» in der Eingabe: Name des Skills und der Rest
function invocation(text) {
  const m = /^\/([a-z0-9][a-z0-9-]*)(?:\s+([\s\S]*))?$/i.exec(String(text ?? '').trim());
  return m ? { slug: m[1].toLowerCase(), rest: (m[2] || '').trim() } : null;
}

module.exports = { Skills, slugify, parse, format, invocation, checkName, MAX_FILE, RESERVED };
