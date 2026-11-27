// Tests für die Download-Seite (scripts/download-page.js)
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { render, fromRelease, build, dateText, mb } = require('../scripts/download-page');

const release = {
  tagName: 'v0.4.0',
  publishedAt: '2026-11-26T23:30:00Z',
  body: '- Neu: **Download-Seite**\n- <script>alert(1)</script>',
  assets: [
    { name: 'ASK-Setup-0.4.0.exe', size: 1 },
    { name: 'ASK-Setup.exe', size: 187_654_321 },
    { name: 'ASK.exe', size: 191_234_567 },
  ],
};

test('Angaben aus «gh release view»: Version ohne «v», Grössen der festen Dateinamen', () => {
  const info = fromRelease(release);
  assert.strictEqual(info.version, '0.4.0');
  assert.strictEqual(info.setupSize, 187_654_321);
  assert.strictEqual(info.portableSize, 191_234_567);
  assert.match(info.notes, /^- Neu/);
  assert.throws(() => fromRelease({ tagName: 'updater' }), /Keine Version/);
});

test('Datum in Schweizer Schreibweise und Zeitzone, Grösse in MB', () => {
  assert.strictEqual(dateText('2026-11-26T23:30:00Z'), '27. November 2026');
  assert.strictEqual(dateText('kein Datum'), '');
  assert.strictEqual(mb(187_654_321), '188 MB');
  assert.strictEqual(mb(10), '1 MB');
});

test('Seite: Links auf die Dateien genau dieser Version, Neuerungen maskiert', async () => {
  const html = await render({ ...fromRelease(release), owner: 'o', repo: 'r' });
  assert.ok(html.includes('href="https://github.com/o/r/releases/download/v0.4.0/ASK-Setup.exe"'));
  assert.ok(html.includes('href="https://github.com/o/r/releases/download/v0.4.0/ASK.exe"'));
  assert.ok(html.includes('href="https://github.com/o/r/releases"'));
  assert.ok(html.includes('ASK-Setup.exe · 188 MB'));
  assert.ok(html.includes('Version 0.4.0 · 27. November 2026'));
  assert.ok(html.includes('<strong>Download-Seite</strong>'));
  assert.ok(html.includes('&lt;script&gt;alert(1)&lt;/script&gt;'));
  assert.strictEqual((html.match(/<script>/g) || []).length, 1, 'nur das eigene Skript der Seite');
  assert.match(html, /src="data:image\/png;base64,/);
});

test('ohne Text und Grössen: Ersatztext, keine leeren Angaben', async () => {
  const html = await render({ version: '0.4.1', notes: '', date: '', setupSize: 0, portableSize: 0, owner: 'o', repo: 'r' });
  assert.ok(html.includes('Verbesserungen und Fehlerbehebungen.'));
  assert.ok(html.includes('<small>ASK-Setup.exe</small>'));
  assert.ok(!html.includes(' ·  · '));
});

test('build schreibt index.html', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-page-'));
  const file = path.join(dir, 'release.json');
  fs.writeFileSync(file, JSON.stringify(release));
  const info = await build(file, path.join(dir, 'out'));
  assert.strictEqual(info.version, '0.4.0');
  assert.match(fs.readFileSync(path.join(dir, 'out', 'index.html'), 'utf8'), /^<!doctype html>/);
});
