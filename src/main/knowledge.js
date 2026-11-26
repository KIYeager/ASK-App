// Gedächtnis (memory.js, ENTSCHEIDE E50) und eigene Skills (skills.js, E51) in der App: Ablage in userData,
// Abschnitte im Systemprompt, Merken nach dem Auftrag, Aufruf mit /name und die Schnittstelle zur Oberfläche.
const fs = require('node:fs');
const path = require('node:path');
const { Memory, learn, cleanTags } = require('./memory');
const { Skills, invocation, slugify } = require('./skills');

const SKILL_TEXT_MAX = 12000; // so viel der Anleitung geht mit /name an das Modell

class Knowledge {
  // dir: userData; settings: settings.js; ask und getModel für das Merken; send(kanal, daten) an die Oberfläche
  constructor({ dir, settings, ask, getModel, send = () => {}, log = () => {} }) {
    this.memory = new Memory({ file: path.join(dir, 'gedaechtnis.json') });
    this.skills = new Skills({ dir: path.join(dir, 'skills') });
    this.settings = settings;
    this.ask = ask;
    this.getModel = getModel;
    this.send = send;
    this.log = log;
    this.learning = null; // laufende Abfrage (höchstens eine)
    this.next = null; // wartender Auftrag, nur der neueste
    this.done = new Set(); // schon ausgewertete Eingaben (Chat und Zeit)
  }

  // Schalter in den Einstellungen, Standard an
  memoryOn() {
    return this.settings.get('memory') !== false;
  }

  // Für agent.js (Werkzeuge merken, vergessen, skill_lesen, skill_erstellen)
  forAgent() {
    return { memory: this.memory, skills: this.skills, memoryOn: () => this.memoryOn(), changed: () => this.send('memory:changed', {}) };
  }

  // Abschnitte für den Systemprompt
  systemPrompt(folder) {
    let text = '';
    try { if (this.memoryOn()) text += this.memory.prompt(folder); } catch { /* Gedächtnis nicht lesbar */ }
    try { text += this.skills.prompt(); } catch { /* Skills nicht lesbar */ }
    return text;
  }

  // «/name …» in der Eingabe: Skill für diesen Auftrag, sonst null
  skillFor(text) {
    const inv = invocation(text);
    if (!inv) return null;
    const s = this.skills.get(inv.slug);
    if (!s || s.slug !== inv.slug) return null;
    const body = s.body.length > SKILL_TEXT_MAX ? `${s.body.slice(0, SKILL_TEXT_MAX)}\n… (gekürzt)` : s.body;
    return { name: s.slug, text: body };
  }

  // Nach einem fertigen Auftrag (Ereignis done von agent.js): im Hintergrund merken, was dauerhaft wichtig ist.
  // Nie nach Stopp oder Fehler, höchstens einmal pro Eingabe, immer nur eine Abfrage gleichzeitig.
  afterTurn(agent, folder, ev) {
    if (!ev || ev.stopped || ev.failed || !this.memoryOn() || !folder) return;
    const msgs = agent.chat.messages;
    const u = msgs.findLastIndex((m) => m.role === 'user' && !m.auto);
    const a = msgs.findLastIndex((m) => m.role === 'assistant' && !m.stopped && String(m.content || '').trim());
    if (u < 0 || a < u) return;
    const key = `${agent.chat.id}:${msgs[u].time}`;
    if (this.done.has(key)) return;
    this.done.add(key);
    if (this.done.size > 500) this.done = new Set([...this.done].slice(-200));
    const job = { folder, chat: { id: agent.chat.id, title: agent.chat.title }, user: msgs[u].content, answer: msgs[a].content };
    if (this.learning) { this.next = job; return; }
    this.run(job);
  }

  run(job) {
    const model = this.getModel();
    this.learning = learn({ memory: this.memory, ask: this.ask, model, ...job, signal: AbortSignal.timeout(90000) })
      .then((res) => {
        if (res && (res.added.length || res.removed.length)) {
          this.send('memory:changed', { chatId: job.chat.id, folder: job.folder, added: res.added.map((i) => i.text), removed: res.removed.length });
        }
      })
      .catch((err) => this.log(`Gedächtnis: ${err?.message || err}`)) // still: kein Fehler im Chat
      .finally(() => {
        this.learning = null;
        const next = this.next;
        this.next = null;
        if (next && this.memoryOn()) this.run(next);
      });
  }

  // ---------- Schnittstelle zur Oberfläche ----------

  setupIpc({ ipcMain, dialog, getWindow, confirm, projects, onSettings }) {
    const changed = () => this.send('memory:changed', {});
    const safe = (fn) => async (...args) => {
      try { return { ok: true, ...(await fn(...args)) }; } catch (err) { return { ok: false, reason: err.message }; }
    };
    const withWindow = (fn, options) => { const win = getWindow(); return win ? fn.call(dialog, win, options) : fn.call(dialog, options); };
    const handle = (channel, fn) => ipcMain.handle(channel, (_e, ...args) => fn(...args)); // ohne das Ereignis
    const scopeOf = (v) => (v?.scope === 'projekt' && v.project ? { scope: 'projekt', project: String(v.project) } : { scope: 'konto', project: null });

    handle('memory:list', () => ({ enabled: this.memoryOn(), items: this.memory.list(), projects: projects() }));
    handle('memory:add', safe((v) => ({ item: this.memory.add({ text: v?.text, tags: cleanTags(v?.tags), ...scopeOf(v), source: 'hand' }) })));
    handle('memory:update', safe((id, v) => {
      const patch = { text: v?.text, tags: v?.tags };
      if (v?.scope) Object.assign(patch, scopeOf(v));
      const item = this.memory.update(String(id || ''), patch);
      if (!item) throw new Error('Den Eintrag gibt es nicht mehr.');
      return { item };
    }));
    handle('memory:remove', (id) => this.memory.remove(String(id || '')));
    handle('memory:clear', async () => {
      const n = this.memory.list().length;
      if (!n) return false;
      if (!(await confirm('Alles vergessen?', `ASK vergisst alle ${n} Einträge im Gedächtnis, über dich und alle Projekte. Das lässt sich nicht rückgängig machen.`, { danger: true }))) return false;
      this.memory.clear();
      changed();
      return true;
    });
    handle('memory:enable', (on) => {
      this.settings.set({ memory: !!on });
      onSettings?.();
    });
    handle('skills:list', () => this.skills.list());
    handle('skills:get', (name) => this.skills.get(String(name || '')));
    handle('skills:save', safe((v, previous) => ({ skill: this.skills.save({ name: v?.name, description: v?.description, body: v?.body }, { previous: String(previous || '') }) })));
    handle('skills:remove', async (name) => {
      const s = this.skills.get(String(name || ''));
      if (!s) return false;
      if (!(await confirm(`Skill «${s.slug}» löschen?`, 'Die Anleitung wird von diesem Laptop entfernt. Exportiere sie vorher, wenn du sie behalten willst.', { danger: true }))) return false;
      return this.skills.remove(s.slug);
    });
    handle('skills:export', safe(async (name) => {
      const text = this.skills.exportText(String(name || ''));
      if (!text) throw new Error('Den Skill gibt es nicht mehr.');
      const res = await withWindow(dialog.showSaveDialog, { title: 'Skill exportieren', defaultPath: `${slugify(name)}.md`,
        filters: [{ name: 'Skill (Markdown)', extensions: ['md'] }] });
      if (res.canceled || !res.filePath) return { canceled: true };
      fs.writeFileSync(res.filePath, text);
      return { path: res.filePath };
    }));
    handle('skills:import', safe(async () => {
      const res = await withWindow(dialog.showOpenDialog, { title: 'Skill importieren', properties: ['openFile'],
        filters: [{ name: 'Skill (Markdown)', extensions: ['md', 'txt'] }] });
      if (res.canceled || !res.filePaths[0]) return { canceled: true };
      const file = res.filePaths[0];
      if (fs.statSync(file).size > 100 * 1024) throw new Error('Die Datei ist grösser als 100 KB.');
      const data = Skills.readImport(fs.readFileSync(file));
      const exists = !!this.skills.get(data.name);
      if (exists && !(await confirm(`Skill «${data.name}» ersetzen?`, 'Es gibt schon einen Skill mit diesem Namen. Die importierte Datei ersetzt ihn.'))) return { canceled: true };
      return { skill: this.skills.save(data, { overwrite: true }) };
    }));
  }
}

module.exports = { Knowledge };
