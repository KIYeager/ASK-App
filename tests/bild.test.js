const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { Images, fluxArguments, findImageRefs, imageType, saveNames, plainText } = require('../src/main/bild');

const ORIGIN = 'https://ask.test';
const PNG = Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), Buffer.alloc(120, 7)]);
const JPG = Buffer.concat([Buffer.from([0xff, 0xd8, 0xff, 0xe0]), Buffer.alloc(120, 3)]);
const DATA = `data:image/png;base64,${PNG.toString('base64')}`;

const jsonRes = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
const sseRes = (chunks) => new Response(chunks.map((c) => `data: ${JSON.stringify(c)}\n\n`).join('') + 'data: [DONE]\n\n',
  { headers: { 'content-type': 'text/event-stream' } });
const delta = (d) => ({ choices: [{ index: 0, delta: d }] });

// ASK zum Testen: handler(pfad, methode, body) liefert die Antwort; alle Aufrufe werden notiert
function fakeAsk(handler) {
  const calls = [];
  return {
    base: ORIGIN,
    calls,
    raw: async (p, options = {}) => {
      const call = { path: p, method: options.method || 'GET', body: options.body ? JSON.parse(options.body) : null };
      calls.push(call);
      return (await handler(call)) || jsonRes({ detail: 'not found' }, 404);
    },
  };
}

const TOOLS = [{ id: 'moodle', specs: [{ name: 'moodle_kurse' }] }, { id: 'flux_bild', name: 'Flux Bild', specs: [{ name: 'bild_erstellen' }] }];

function folder() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-bild-'));
  return { root, done: () => fs.rmSync(root, { recursive: true, force: true }) };
}

test('Argumente für Flux: nur gesetzte und gültige Werte', () => {
  assert.deepStrictEqual(fluxArguments({ beschreibung: ' Ein Fuchs ', format: 'Breitbild', qualitaet: 'super', anzahl: 9, stil: '', referenzen: 'Eiger = mountain', tf_branding: true }, false),
    { beschreibung: 'Ein Fuchs', format: 'breitbild', anzahl: 4, referenzen: 'Eiger = mountain', tf_branding: true });
  assert.deepStrictEqual(fluxArguments({ beschreibung: 'Himmel blau', referenzen: 'Eiger = mountain', anzahl: 1, verwandeln: 'true' }, true),
    { beschreibung: 'Himmel blau', bearbeiten: true, verwandeln: true });
});

test('Bildtyp an den ersten Bytes', () => {
  assert.strictEqual(imageType(PNG), '.png');
  assert.strictEqual(imageType(JPG), '.jpg');
  assert.strictEqual(imageType(Buffer.from('<html>kein Bild</html>')), null);
});

test('Bilder in Antworten von ASK finden', () => {
  const found = findImageRefs({
    files: [{ type: 'image', url: '/api/v1/files/abc-1' }, { type: 'image', id: 'def-2' }, { type: 'file', url: '/api/v1/files/doc/content' }],
    content: `Fertig!\n![Bild](${ORIGIN}/cache/image/generations/x1.png)\n![extern](https://cdn.example.com/out/bild.webp)\n`
      + '<details type="tool_calls" result="&quot;![b](/api/v1/files/ghi-3/content)&quot;"></details>'
      + '![ref](https://upload.wikimedia.org/eiger.jpg) ![fremd](https://andere.ch/api/v1/files/zzz/content)',
    embed: DATA,
  }, ORIGIN);
  assert.deepStrictEqual(found.strong, ['/api/v1/files/abc-1/content', '/api/v1/files/def-2/content', '/cache/image/generations/x1.png',
    '/api/v1/files/ghi-3/content', DATA]);
  assert.deepStrictEqual(found.weak, ['https://cdn.example.com/out/bild.webp', 'https://andere.ch/api/v1/files/zzz/content']);
  assert.strictEqual(plainText('Hier ist es:\n![b](/api/v1/files/x/content)\n<details>gross</details>'), 'Hier ist es:');
});

test('Dateinamen: neu, nummeriert, gewünscht, Vorlage ersetzen', () => {
  const f = folder();
  try {
    fs.mkdirSync(path.join(f.root, 'bilder'));
    fs.writeFileSync(path.join(f.root, 'bilder', 'fuchs-im-schnee.png'), 'alt');
    assert.deepStrictEqual(saveNames(f.root, { beschreibung: 'Fuchs im Schnee!' }, ['.png']), ['bilder/fuchs-im-schnee-2.png']);
    assert.deepStrictEqual(saveNames(f.root, { beschreibung: 'Grüsse über Bern' }, ['.png', '.jpg']), ['bilder/gruesse-ueber-bern-1.png', 'bilder/gruesse-ueber-bern-2.jpg']);
    assert.deepStrictEqual(saveNames(f.root, { datei: 'bilder/fuchs-im-schnee.png' }, ['.png']), ['bilder/fuchs-im-schnee.png']);
    assert.deepStrictEqual(saveNames(f.root, { datei: 'plakat/titel.jpg' }, ['.png']), ['plakat/titel.png']);
    assert.deepStrictEqual(saveNames(f.root, { vorlage: 'bilder/fuchs-im-schnee.png' }, ['.png']), ['bilder/fuchs-im-schnee.png']);
    assert.deepStrictEqual(saveNames(f.root, { vorlage: 'fotos/klasse.jpeg' }, ['.jpg']), ['fotos/klasse.jpeg']);
    assert.deepStrictEqual(saveNames(f.root, { vorlage: 'fotos/klasse.jpeg' }, ['.png']), ['fotos/klasse.png']);
    assert.deepStrictEqual(saveNames(f.root, { vorlage: '.ask/anhaenge/c1/Mein Foto.png' }, ['.png']), ['bilder/mein-foto.png']);
    assert.throws(() => saveNames(f.root, { datei: '../draussen.png' }, ['.png']), /nicht im Arbeitsordner/);
  } finally {
    f.done();
  }
});

test('Flux-Werkzeug finden', async () => {
  const images = new Images({ ask: fakeAsk(({ path: p }) => (p === '/api/v1/tools/' ? jsonRes(TOOLS) : null)) });
  assert.deepStrictEqual(await images.discover(), { id: 'flux_bild', name: 'Flux Bild' });
  assert.ok(images.available());
  const none = new Images({ ask: fakeAsk(({ path: p }) => (p === '/api/v1/tools/' ? jsonRes([TOOLS[0]]) : null)) });
  assert.strictEqual(await none.discover(), null);
  const denied = new Images({ ask: fakeAsk(() => jsonRes({ detail: 'nein' }, 401)) });
  assert.strictEqual(await denied.discover(), null);
});

test('Wie die Webseite: Antwort im Hintergrund, Bild als Datei im vorübergehenden Chat', async () => {
  const f = folder();
  let polls = 0;
  const ask = fakeAsk(({ path: p, method, body }) => {
    if (p === '/api/v1/tools/') return jsonRes(TOOLS);
    if (p === '/api/v1/chats/new') return jsonRes({ id: 'tmp1', chat: body.chat });
    if (p === '/api/chat/completions') return jsonRes({ status: true, task_id: 't1' });
    if (p === '/api/v1/chats/tmp1' && method === 'GET') {
      polls++;
      const answer = Object.values(ask.calls.find((c) => c.path === '/api/v1/chats/new').body.chat.history.messages).find((m) => m.role === 'assistant');
      return jsonRes({ id: 'tmp1', chat: { history: { messages: { [answer.id]: { ...answer,
        statusHistory: [{ description: 'Bild wird erstellt' }],
        files: polls > 1 ? [{ type: 'image', url: '/api/v1/files/bild-1/content' }] : [], content: polls > 1 ? 'Hier ist dein Fuchs.' : '' } } } } });
    }
    if (p === '/api/tasks/chat/tmp1') return jsonRes({ task_ids: polls > 1 ? [] : ['t1'] });
    if (p === '/api/v1/files/bild-1/content') return new Response(PNG, { headers: { 'content-type': 'image/png' } });
    if (p === '/api/v1/chats/tmp1' && method === 'DELETE') return jsonRes(true);
    return null;
  });
  const progress = [];
  try {
    const images = new Images({ ask, getModel: () => 'qwen', pollMs: 20 });
    const r = await images.create(f.root, { beschreibung: 'Ein Fuchs im Schnee', format: 'querformat' }, { chat: 'c1', onProgress: (t) => progress.push(t) });
    assert.deepStrictEqual(r.files, ['bilder/ein-fuchs-im-schnee.png']);
    assert.strictEqual(r.text, 'Hier ist dein Fuchs.');
    assert.ok(fs.readFileSync(path.join(f.root, 'bilder/ein-fuchs-im-schnee.png')).equals(PNG));
    const sent = ask.calls.find((c) => c.path === '/api/chat/completions').body;
    assert.deepStrictEqual(sent.tool_ids, ['flux_bild']);
    assert.strictEqual(sent.model, 'qwen');
    assert.strictEqual(sent.chat_id, 'tmp1');
    assert.ok(sent.id && sent.session_id && !sent.tools, 'wie die Webseite, ohne Werkzeuge der App');
    assert.match(sent.messages[1].content, /"format": "querformat"/);
    await new Promise((r2) => setTimeout(r2, 10));
    assert.ok(ask.calls.some((c) => c.path === '/api/v1/chats/tmp1' && c.method === 'DELETE'), 'vorübergehender Chat gelöscht');
    assert.ok(progress.some((t) => /^Flux erstellt das Bild … \d+ s$/.test(t)), progress.join(' | '));
  } finally {
    f.done();
  }
});

test('Gestreamte Antwort mit Bild-Link, ohne Chat; Vorlage bearbeiten', async () => {
  const f = folder();
  fs.mkdirSync(path.join(f.root, 'fotos'));
  fs.writeFileSync(path.join(f.root, 'fotos', 'klasse.png'), PNG);
  const ask = fakeAsk(({ path: p }) => {
    if (p === '/api/v1/tools/') return jsonRes(TOOLS);
    if (p === '/api/v1/chats/new') return jsonRes({ detail: 'Method Not Allowed' }, 405);
    if (p === '/api/chat/completions') {
      // Link in kleinen Stücken wie bei einem echten Strom
      const text = 'Bearbeitet: ![bild](/cache/image/generations/neu.jpg)';
      const parts = text.match(/.{1,7}/g).map((c) => delta({ content: c }));
      return sseRes([...parts, { files: [{ type: 'image', url: '/api/v1/files/gleich' }] }]);
    }
    if (p === '/cache/image/generations/neu.jpg' || p === '/api/v1/files/gleich/content') return new Response(JPG);
    return null;
  });
  try {
    const images = new Images({ ask, getModel: () => 'qwen' });
    const r = await images.create(f.root, { beschreibung: 'Himmel blau', vorlage: 'fotos/klasse.png' }, { chat: 'c1' });
    assert.deepStrictEqual(r.files, ['fotos/klasse.jpg'], 'anderes Format: neben der Vorlage; dasselbe Bild unter zwei Adressen nur einmal');
    assert.ok(ask.calls.some((c) => c.path === '/cache/image/generations/neu.jpg'), 'Link aus Stücken zusammengesetzt');
    await assert.rejects(images.create(f.root, { beschreibung: 'x', datei: '../weg.png' }), /nicht im Arbeitsordner/);
    assert.strictEqual(ask.calls.filter((c) => c.path === '/api/chat/completions').length, 1, 'falscher Pfad: Flux gar nicht erst gefragt');
    assert.ok(fs.readFileSync(path.join(f.root, 'fotos/klasse.jpg')).equals(JPG));
    const sent = ask.calls.find((c) => c.path === '/api/chat/completions').body;
    assert.ok(!sent.chat_id && !sent.session_id);
    assert.strictEqual(sent.messages[1].content[1].image_url.url, DATA, 'Vorlage als Bild in der Nachricht');
    assert.match(sent.messages[1].content[0].text, /"bearbeiten": true/);
  } finally {
    f.done();
  }
});

test('Kein Bild: verständlicher Fehler und Antwort von ASK zum Nachsehen', async () => {
  const f = folder();
  const ask = fakeAsk(({ path: p }) => {
    if (p === '/api/v1/tools/') return jsonRes(TOOLS);
    if (p === '/api/chat/completions') return sseRes([delta({ content: 'Das Bild konnte nicht erstellt werden.' })]);
    return null;
  });
  try {
    const images = new Images({ ask, getModel: () => 'qwen' });
    await assert.rejects(images.create(f.root, { beschreibung: 'Fuchs' }, { chat: 'c1' }),
      /ASK hat kein Bild zurückgegeben\. Antwort von ASK: «Das Bild konnte nicht erstellt werden\.» Details: \.ask\/arbeit\/c1\/flux-\w+\.json/);
    const trace = fs.readdirSync(path.join(f.root, '.ask/arbeit/c1'));
    assert.strictEqual(trace.length, 1);
    assert.ok(!fs.existsSync(path.join(f.root, 'bilder')));

    const passthrough = fakeAsk(({ path: p }) => {
      if (p === '/api/v1/tools/') return jsonRes(TOOLS);
      if (p === '/api/chat/completions') return sseRes([delta({ tool_calls: [{ index: 0, id: 'x', function: { name: 'bild_erstellen', arguments: '{}' } }] })]);
      return null;
    });
    await assert.rejects(new Images({ ask: passthrough, getModel: () => 'qwen' }).create(f.root, { beschreibung: 'Fuchs' }, { chat: 'c1' }),
      /nicht selbst ausgeführt/);
    const refused = fakeAsk(({ path: p }) => (p === '/api/v1/tools/' ? jsonRes(TOOLS) : p === '/api/chat/completions' ? jsonRes({ detail: 'Tool not allowed' }, 403) : null));
    await assert.rejects(new Images({ ask: refused, getModel: () => 'qwen' }).create(f.root, { beschreibung: 'Fuchs' }), /ASK: Tool not allowed/);
    const noFlux = fakeAsk(() => jsonRes([]));
    await assert.rejects(new Images({ ask: noFlux, getModel: () => 'qwen' }).create(f.root, { beschreibung: 'Fuchs' }), /nicht freigegeben/);
  } finally {
    f.done();
  }
});

test('Stoppen hält auch die Antwort in ASK an', async () => {
  const f = folder();
  const ask = fakeAsk(({ path: p, method }) => {
    if (p === '/api/v1/tools/') return jsonRes(TOOLS);
    if (p === '/api/v1/chats/new') return jsonRes({ id: 'tmp2' });
    if (p === '/api/chat/completions') return jsonRes({ status: true, task_id: 't9' });
    if (p === '/api/v1/chats/tmp2' && method === 'GET') return jsonRes({ chat: { history: { messages: {} } } });
    if (p === '/api/tasks/chat/tmp2') return jsonRes({ task_ids: ['t9'] });
    return jsonRes(true);
  });
  try {
    const controller = new AbortController();
    setTimeout(() => controller.abort(), 120);
    const images = new Images({ ask, getModel: () => 'qwen', pollMs: 20 });
    await assert.rejects(images.create(f.root, { beschreibung: 'Fuchs' }, { signal: controller.signal }), (err) => err.name === 'AbortError');
    assert.ok(ask.calls.some((c) => c.path === '/api/tasks/stop/t9' && c.method === 'POST'));
    assert.ok(ask.calls.some((c) => c.path === '/api/v1/chats/tmp2' && c.method === 'DELETE'));
  } finally {
    f.done();
  }
});
