// Hilfsagenten (ENTSCHEIDE E54): Das Modell gibt eine Teilfrage ab, ein eigener kleiner Agent klärt sie mit
// lesenden Werkzeugen und gibt nur das Ergebnis zurück. So bleibt der Verlauf des Chats kurz, und mehrere
// unabhängige Suchen laufen gleichzeitig. Hilfsagenten ändern nichts (keine Befehle, kein Schreiben), sehen den
// Chat nicht und starten keine weiteren Hilfsagenten.
const crypto = require('node:crypto');
const { toolSpecs, runTool, describe, clip, cleanArgs } = require('./tools');
const { parseTextToolCalls } = require('./askapi');

const HELPER = 'hilfsagent';
const HELPER_TOOLS = ['read_file', 'list_files', 'find_files', 'search_text', 'web_search', 'web_fetch'];
const MAX_STEPS = 20; // danach muss der Hilfsagent ohne Werkzeuge zusammenfassen
const PARALLEL = 3; // so viele Hilfsagenten eines Chats arbeiten gleichzeitig
const RESULT_CHARS = 8000; // so viel vom Ergebnis geht an das Modell des Chats
const TOOL_CHARS = 20000; // so viel von einem Werkzeugergebnis sieht der Hilfsagent

const HELPER_TOOL = {
  type: 'function',
  function: {
    name: HELPER,
    description: 'Startet einen Hilfsagenten, der eine Teilfrage selbständig klärt und nur sein Ergebnis zurückgibt. Er kann Dateien '
      + 'im Projekt suchen und lesen und im Web recherchieren, ändert aber nichts und sieht diesen Chat nicht: Beschreibe den Auftrag '
      + 'vollständig (Ziel, wo suchen, was zurückmelden). Gut für breite Suchen über viele Dateien und für mehrere unabhängige '
      + `Recherchen: Mehrere Aufrufe im selben Schritt laufen gleichzeitig (höchstens ${PARALLEL}). Eine einzelne bekannte Datei liest du selbst.`,
    parameters: {
      type: 'object',
      properties: {
        titel: { type: 'string', description: 'Kurzer Titel für die Anzeige, z. B. «Verwendung von config.json finden»' },
        auftrag: { type: 'string', description: 'Vollständiger Auftrag: was herausfinden, wo suchen, in welcher Form antworten' },
      },
      required: ['titel', 'auftrag'],
    },
  },
};

function helperPrompt(folder) {
  return 'Du bist ein Hilfsagent von ASK Desktop. Ein anderer Agent hat dir eine Teilaufgabe gegeben; du arbeitest selbständig und '
    + `fragst nicht zurück. Arbeitsordner: ${folder || '(keiner)'}. Pfade sind relativ zu diesem Ordner.\n`
    + 'Du hast nur lesende Werkzeuge: Dateien auflisten, suchen und lesen, im Web suchen und Seiten lesen. Du änderst nichts.\n'
    + 'Geh zielgerichtet vor: zuerst suchen, dann nur das Nötige lesen. Hör auf, sobald du die Frage beantworten kannst.\n'
    + 'Deine letzte Antwort (ohne Werkzeugaufruf) ist dein Ergebnis für den anderen Agenten: knapp und konkret, mit Dateipfaden und '
    + 'Zeilennummern oder Adressen als Beleg. Sag ehrlich, was du nicht gefunden hast.';
}

const FINAL_PROMPT = '\n\nDu hast die Höchstzahl an Schritten erreicht. Rufe kein Werkzeug mehr auf und fasse jetzt zusammen, '
  + 'was du herausgefunden hast und was offen bleibt.';

// Einen Hilfsagenten ausführen. Liefert { ok, output, summary, display }, wie ein Werkzeug.
// deps: { ask, model, thinking, terminal, web, folder, chat, textMode, apiMessages, textModePrompt, signal, onProgress }
async function runHelper(rawArgs, deps) {
  let args = {};
  try { args = JSON.parse(rawArgs || '{}') || {}; } catch { /* unten */ }
  const task = String(args.auftrag || args.task || '').trim();
  const title = String(args.titel || args.title || '').replace(/\s+/g, ' ').trim().slice(0, 120) || clip(task, 80);
  if (!task) return { ok: false, output: 'auftrag fehlt.', summary: 'ohne Auftrag' };
  if (!deps.model) return { ok: false, output: 'Kein Modell für Hilfsagenten.', summary: 'kein Modell' };
  const specs = toolSpecs().filter((s) => HELPER_TOOLS.includes(s.function.name));
  const messages = [{ role: 'user', content: task, time: Date.now() }];
  const steps = [];
  let tokens = 0;
  const progress = () => deps.onProgress?.(`${title}\n${steps.join('\n')}`);
  progress();
  for (let n = 0; n <= MAX_STEPS; n++) {
    if (deps.signal?.aborted) return { ok: false, output: 'Von der Person abgebrochen.', summary: 'abgebrochen' };
    const last = n === MAX_STEPS;
    const textMode = !!deps.textMode;
    const system = helperPrompt(deps.folder) + (last ? FINAL_PROMPT : textMode ? deps.textModePrompt(specs) : '');
    let completion;
    try {
      completion = await deps.ask.chat({ model: deps.model, messages: deps.apiMessages(system, messages, textMode), tools: last || textMode ? null : specs,
        thinking: deps.thinking, signal: deps.signal, onDelta: () => {} });
    } catch (err) {
      if (deps.signal?.aborted) return { ok: false, output: 'Von der Person abgebrochen.', summary: 'abgebrochen' };
      return { ok: false, output: `Der Hilfsagent konnte ASK nicht erreichen: ${err.message}`, summary: 'Fehler bei ASK', display: steps.join('\n') };
    }
    tokens += completion.stats?.().tokens || 0;
    let content = completion.content || '';
    let calls = completion.calls || [];
    if (!calls.length && (textMode || /<tool_call>/.test(content))) {
      const parsed = parseTextToolCalls(content);
      if (parsed.calls.length) { calls = parsed.calls; content = parsed.text; }
    }
    if (last || !calls.length) {
      const text = content.trim() || 'Der Hilfsagent hat kein Ergebnis geschrieben.';
      const summary = `${steps.length} ${steps.length === 1 ? 'Schritt' : 'Schritte'}${tokens ? ` · ${tokens.toLocaleString('de-CH')} Token` : ''}`;
      return { ok: !!content.trim(), output: `Ergebnis des Hilfsagenten «${title}»:\n${clip(text, RESULT_CHARS)}`, summary,
        display: `${title}\n\n${steps.length ? `Schritte:\n${steps.join('\n')}\n\n` : ''}Ergebnis:\n${text}` };
    }
    const toolCalls = calls.map((c) => ({ id: c.id || `call_${crypto.randomBytes(6).toString('hex')}`, type: 'function',
      function: { name: String(c.name || ''), arguments: cleanArgs(c.arguments) } }));
    messages.push({ role: 'assistant', content, tool_calls: toolCalls, textMode, time: Date.now() });
    for (const call of toolCalls) {
      const name = call.function.name;
      let parsed = {};
      try { parsed = JSON.parse(call.function.arguments); } catch { /* roh */ }
      const result = HELPER_TOOLS.includes(name)
        ? await runTool(deps.terminal, name, call.function.arguments, { signal: deps.signal, web: deps.web, folder: deps.folder, chat: deps.chat })
        : { ok: false, output: `Hilfsagenten haben nur diese Werkzeuge: ${HELPER_TOOLS.join(', ')}.`, summary: 'nicht erlaubt' };
      messages.push({ role: 'tool', tool_call_id: call.id, name, content: clip(String(result.output ?? ''), TOOL_CHARS), ok: result.ok, textMode });
      const d = describe(name, parsed);
      steps.push(`${result.ok === false ? '✗' : '●'} ${d.label}(${clip(d.detail, 80)})${result.summary ? ` – ${clip(String(result.summary), 80)}` : ''}`);
      progress();
      if (deps.signal?.aborted) break;
    }
  }
  return { ok: false, output: 'Der Hilfsagent ist ohne Ergebnis zu Ende gegangen.', summary: 'ohne Ergebnis' };
}

// Führt die Aufgaben mit höchstens limit gleichzeitig aus; Ergebnisse in derselben Reihenfolge
async function pool(tasks, limit = PARALLEL) {
  const results = new Array(tasks.length);
  let next = 0;
  const worker = async () => {
    while (next < tasks.length) {
      const i = next++;
      results[i] = await tasks[i]();
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, tasks.length) }, worker));
  return results;
}

module.exports = { HELPER, HELPER_TOOL, HELPER_TOOLS, PARALLEL, MAX_STEPS, runHelper, pool, helperPrompt };
