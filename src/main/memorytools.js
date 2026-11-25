// Werkzeuge für Gedächtnis (memory.js, ENTSCHEIDE E50) und eigene Skills (skills.js, E51). Sie laufen im
// Hauptprozess, nicht im Terminal: Gedächtnis und Skills liegen in den Daten der App, die das Terminal
// nicht beschreiben kann. ctx.knowledge = { memory, skills, memoryOn() } kommt von main.js über agent.js.
const { isSensitive } = require('./memory');
const { slugify, checkName } = require('./skills');

const MEMORY_TOOLS = ['merken', 'vergessen'];

function need(ctx, part) {
  const k = ctx.knowledge;
  if (!k || !k[part]) throw new Error(part === 'memory' ? 'Das Gedächtnis ist hier nicht verfügbar.' : 'Skills sind hier nicht verfügbar.');
  if (part === 'memory' && k.memoryOn && !k.memoryOn()) throw new Error('Das Gedächtnis ist in den Einstellungen ausgeschaltet.');
  return k[part];
}

const clipLine = (t, n = 80) => { const s = String(t ?? '').replace(/\s+/g, ' ').trim(); return s.length > n ? `${s.slice(0, n - 1)}…` : s; };

const KNOWLEDGE_TOOLS = [
  {
    name: 'merken',
    label: 'Merken',
    describe: (a) => a.text,
    description: 'Merkt sich eine dauerhafte Angabe für spätere Chats, nur auf diesem Laptop. Benutze es, wenn die Person «merk dir …» sagt '
      + 'oder eine Vorliebe nennt, die auch später gilt. bereich «konto»: über die Person, gilt in allen Projekten; «projekt»: nur für dieses Projekt. '
      + 'Ein kurzer Satz. Nie Passwörter, Tokens, Gesundheit oder Angaben über andere Personen.',
    parameters: {
      type: 'object',
      properties: {
        text: { type: 'string', description: 'Ein kurzer Satz, z. B. «Mag Antworten in Stichworten.»' },
        bereich: { type: 'string', enum: ['konto', 'projekt'], description: 'konto (über die Person) oder projekt (nur dieses Projekt)' },
        tags: { type: 'array', items: { type: 'string' }, description: '1 bis 3 kurze Schlagworte' },
      },
      required: ['text', 'bereich'],
    },
    run: async (terminal, a, ctx) => {
      const memory = need(ctx, 'memory');
      if (isSensitive(a.text)) return { ok: false, output: 'Nicht gemerkt: Das sieht nach heiklen Angaben aus (Zugangsdaten, Gesundheit, Noten). So etwas gehört nicht ins Gedächtnis.', summary: 'nicht gemerkt' };
      const scope = a.bereich === 'projekt' ? 'projekt' : 'konto';
      if (scope === 'projekt' && !ctx.folder) throw new Error('Ohne Projekt geht nur bereich «konto».');
      const item = memory.add({ text: a.text, tags: a.tags, scope, project: scope === 'projekt' ? ctx.folder : null,
        chat: ctx.chat, chatTitle: ctx.chatTitle, chatFolder: ctx.folder, source: 'merken' });
      ctx.knowledge.changed?.();
      return { ok: true, output: `Gemerkt (id ${item.id}, ${scope === 'projekt' ? 'dieses Projekt' : 'alle Projekte'}): ${item.text}`, summary: clipLine(item.text) };
    },
  },
  {
    name: 'vergessen',
    label: 'Vergessen',
    describe: (a) => a.id || a.text,
    description: 'Löscht einen Eintrag aus dem Gedächtnis, mit der id aus dem Abschnitt «Gedächtnis» oder mit seinem Text. '
      + 'Benutze es, wenn die Person sagt, etwas stimme nicht mehr oder ASK solle es vergessen.',
    parameters: {
      type: 'object',
      properties: { id: { type: 'string', description: 'id des Eintrags, z. B. m1a2b3c4' }, text: { type: 'string', description: 'oder der Text des Eintrags' } },
    },
    run: async (terminal, a, ctx) => {
      const memory = need(ctx, 'memory');
      const visible = new Set(memory.visible(ctx.folder).map((i) => i.id));
      let items = [];
      if (a.id) items = visible.has(String(a.id)) ? [memory.get(a.id)] : [];
      else if (a.text) items = memory.findByText(a.text, ctx.folder);
      else throw new Error('id oder text angeben.');
      if (!items.length) return { ok: false, output: 'Kein passender Eintrag im Gedächtnis.', summary: 'nicht gefunden' };
      if (items.length > 3) return { ok: false, output: `${items.length} Einträge passen. Gib die id an:\n${items.slice(0, 10).map((i) => `- [${i.id}] ${i.text}`).join('\n')}`, summary: 'nicht eindeutig' };
      for (const i of items) memory.remove(i.id);
      ctx.knowledge.changed?.();
      return { ok: true, output: `Vergessen: ${items.map((i) => i.text).join(' | ')}`, summary: clipLine(items.map((i) => i.text).join(', ')) };
    },
  },
  {
    name: 'skill_lesen',
    label: 'Skill',
    describe: (a) => a.name,
    description: 'Liest die Anleitung eines eigenen Skills der Person (Liste unter «Eigene Skills»). Lies den Skill, bevor du einen passenden Auftrag ausführst.',
    parameters: { type: 'object', properties: { name: { type: 'string', description: 'Name des Skills aus der Liste' } }, required: ['name'] },
    run: async (terminal, a, ctx) => {
      const skills = need(ctx, 'skills');
      const s = skills.get(a.name);
      if (!s) {
        const names = skills.list().map((x) => x.slug);
        return { ok: false, output: `Keinen Skill «${a.name}» gefunden.${names.length ? ` Vorhanden: ${names.join(', ')}` : ' Es gibt noch keine eigenen Skills.'}`, summary: 'nicht gefunden' };
      }
      return { ok: true, output: `Skill «${s.slug}»: ${s.description}\n\n${s.body}`, summary: `${s.slug} gelesen`, display: `${s.description}\n\n${s.body.slice(0, 3000)}` };
    },
  },
  {
    name: 'skill_erstellen',
    label: 'Skill erstellen',
    describe: (a) => a.name,
    description: 'Legt einen eigenen Skill an (oder ersetzt einen gleichnamigen): eine Anleitung, die ASK später bei passenden Aufträgen liest und die die Person mit /name aufrufen kann. '
      + 'Nur, wenn die Person es wünscht. Die Person bestätigt das Speichern.',
    parameters: {
      type: 'object',
      properties: {
        name: { type: 'string', description: 'kurzer Name, klein mit Bindestrichen, z. B. «lernjournal»' },
        beschreibung: { type: 'string', description: 'Ein Satz: wofür und wann der Skill gilt' },
        anleitung: { type: 'string', description: 'Die Anleitung in Markdown: Schritte, Vorgaben, Beispiele' },
      },
      required: ['name', 'beschreibung', 'anleitung'],
    },
    run: async (terminal, a, ctx) => {
      const skills = need(ctx, 'skills');
      const slug = slugify(a.name);
      const problem = checkName(slug);
      if (problem) throw new Error(problem);
      const exists = !!skills.get(slug);
      const ok = ctx.confirm ? await ctx.confirm(exists ? `Skill «${slug}» ersetzen?` : `Skill «${slug}» speichern?`,
        `${clipLine(a.beschreibung, 200)}\n\n${String(a.anleitung || '').slice(0, 600)}${String(a.anleitung || '').length > 600 ? ' …' : ''}\n\n`
        + `Aufrufen mit /${slug}. Ändern und löschen in den Einstellungen unter «Skills».`) : false;
      if (!ok) return { ok: false, output: 'Die Person hat das Speichern abgelehnt.', summary: 'abgelehnt' };
      const s = skills.save({ name: slug, description: a.beschreibung, body: a.anleitung }, { overwrite: true });
      ctx.knowledge.changed?.();
      return { ok: true, output: `Skill «${s.slug}» gespeichert. Die Person ruft ihn mit /${s.slug} auf.`, summary: `/${s.slug}` };
    },
  },
];

module.exports = { KNOWLEDGE_TOOLS, MEMORY_TOOLS };
