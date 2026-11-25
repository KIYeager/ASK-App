// Backend «nativ mit Low Integrity»: startet python/ask_terminal/launcher.py mit dem mitgelieferten
// Python. Der Starter kennzeichnet den Ordner, senkt den Server auf Low und hält ihn in einem
// Job-Objekt. Der Key geht über stdin, nie über Kommandozeile oder Datei.
//
// Ein anderes Backend (Sandbox, WSL, AppContainer) muss nur start({...}), stop() und running()
// mit derselben Bedeutung anbieten.
const fs = require('node:fs');
const { spawn } = require('node:child_process');
const readline = require('node:readline');
const paths = require('../paths');

class NativeLowBackend {
  constructor() {
    this.child = null;
  }

  running() {
    return !!this.child && this.child.exitCode === null && !this.child.killed;
  }

  async start({ workdir, port, key, origin, systemPrompt, executeDescription, onEvent }) {
    const python = paths.pythonExe();
    if (!fs.existsSync(python)) throw new Error(`Mitgeliefertes Python fehlt: ${python}`);
    // -X utf8: Die Einstellungen gehen als UTF-8 über stdin. Ohne den Schalter liest Python sie auf Windows in
    // der ANSI-Codepage, und ein Ordner oder Benutzername mit Umlaut («Prüfung») wird nicht gefunden.
    const child = spawn(python, ['-E', '-s', '-B', '-X', 'utf8', paths.launcherPy()], {
      stdio: ['pipe', 'pipe', 'pipe'],
      windowsHide: true,
    });
    this.child = child;
    const lines = readline.createInterface({ input: child.stdout });
    lines.on('line', (line) => {
      try {
        onEvent(JSON.parse(line));
      } catch {
        onEvent({ event: 'log', line });
      }
    });
    readline.createInterface({ input: child.stderr }).on('line', (line) => onEvent({ event: 'log', line }));
    child.on('exit', (code) => {
      if (this.child === child) this.child = null;
      onEvent({ event: 'process-exit', code });
    });
    child.on('error', (err) => onEvent({ event: 'error', code: 'spawn', message: err.message }));
    child.stdin.on('error', () => {});
    const cfg = {
      key, port, workdir, origin,
      runtime: paths.runtimeDir(),
      system_prompt: systemPrompt,
      execute_description: executeDescription,
      low_integrity: true,
    };
    child.stdin.write(JSON.stringify(cfg) + '\n');
  }

  // Bittet den Starter, den Job zu beenden; nach 5 s wird er hart beendet (der Job räumt trotzdem auf).
  stop() {
    const child = this.child;
    if (!child) return Promise.resolve();
    this.child = null;
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        try { child.kill(); } catch { /* bereits beendet */ }
        resolve();
      }, 5000);
      child.once('exit', () => {
        clearTimeout(timer);
        resolve();
      });
      try {
        child.stdin.write('stop\n');
        child.stdin.end();
      } catch {
        child.kill();
      }
    });
  }
}

module.exports = NativeLowBackend;
