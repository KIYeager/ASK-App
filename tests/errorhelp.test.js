// Hilfe bei Fehlern (ENTSCHEIDE E68): passende Erklärung je Meldung, sonst keine
const test = require('node:test');
const assert = require('node:assert');
const path = require('node:path');
const { pathToFileURL } = require('node:url');

test('Fehlerhilfe erkennt häufige Meldungen', async () => {
  const { errorHelp, errorDetails } = await import(pathToFileURL(path.join(__dirname, '..', 'src', 'renderer', 'src', 'lib', 'errorHelp.mjs')).href);
  assert.match(errorHelp('HTTP 401: Unauthorized').title, /Anmeldung/);
  assert.match(errorHelp('error sending request: dns error: failed to lookup address').title, /Keine Verbindung/);
  assert.match(errorHelp("This model's maximum context length is 8192 tokens").title, /zu lang/);
  assert.match(errorHelp('HTTP 503 Service Unavailable').title, /Problem/);
  assert.match(errorHelp('HTTP 429 Too Many Requests').title, /zu viele/);
  assert.match(errorHelp('Terminal: Ordner nicht gefunden').title, /Terminal/);
  assert.equal(errorHelp('Etwas ganz anderes'), null);
  const d = errorDetails('HTTP 503', { version: '0.3.11', askUrl: 'https://ask.example', ask: { state: 'angemeldet' } });
  assert.match(d, /^ASK 0\.3\.11/);
  assert.match(d, /Meldung: HTTP 503$/);
});
