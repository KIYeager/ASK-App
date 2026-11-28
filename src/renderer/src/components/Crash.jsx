import { Component } from 'react';
import { api } from '../api.js';
import { errorDetails } from '../lib/errorHelp.mjs';

// Absturzschutz der Oberfläche: Ein Fehler beim Zeichnen leerte bisher das ganze Fenster. Jetzt fängt Guard ihn ab.
// Ganz aussen (whole) zeigt er eine Karte mit «Neu laden» und «Details kopieren»; um einzelne Nachrichten oder die
// rechte Seite nur einen kurzen Hinweis an der Stelle, der Rest bleibt bedienbar. Chats und laufende Aufträge liegen
// im Hauptprozess, Neuladen der Oberfläche verliert also nichts.

const details = (error, info, state) => {
  const stack = String(error?.stack || '').split('\n').slice(0, 8).join('\n');
  const where = String(info?.componentStack || '').trim().split('\n').slice(0, 6).join('\n');
  return errorDetails(`Oberfläche: ${error?.message || error}\n\n${stack}${where ? `\n\nKomponenten:\n${where}` : ''}`, state);
};

export default class Guard extends Component {
  constructor(props) {
    super(props);
    this.state = { error: null, info: null, copied: false };
  }

  static getDerivedStateFromError(error) {
    return { error };
  }

  componentDidCatch(error, info) {
    this.setState({ info });
    console.error('Fehler in der Oberfläche', error, info?.componentStack);
  }

  // Neuer Inhalt (z. B. anderer Chat, andere Datei): nochmals versuchen
  componentDidUpdate(prev) {
    if (this.state.error && prev.resetKey !== this.props.resetKey) this.setState({ error: null, info: null, copied: false });
  }

  copy = async () => {
    const { error, info } = this.state;
    try {
      // Version, Anmeldung und Terminal gehören in die Details; ganz aussen kennt Guard den Zustand nicht selbst
      const state = this.props.appState || await Promise.resolve(api().getState?.()).catch(() => null);
      await api().copy(details(error, info, state));
    } catch { /* ohne App: nichts */ }
    this.setState({ copied: true });
    setTimeout(() => this.setState({ copied: false }), 1500);
  };

  render() {
    const { error, copied } = this.state;
    if (!error) return this.props.children;
    const copyBtn = (
      <button type="button" className="btn small" onClick={this.copy}>{copied ? 'Kopiert' : 'Details kopieren'}</button>
    );
    if (!this.props.whole) {
      return (
        <div className="msg error crash-inline" role="alert">
          <span>{this.props.label || 'Dieser Teil'} lässt sich nicht anzeigen.</span>
          <button type="button" className="btn small" onClick={() => this.setState({ error: null, info: null })}>Nochmals versuchen</button>
          {copyBtn}
        </div>
      );
    }
    return (
      <div className="crash" role="alert">
        <div className="crash-card">
          <img src="./tf-mark.png" alt="" className="logo-mark" />
          <h1>Hier ist etwas schiefgelaufen</h1>
          <p className="muted">Die Anzeige von ASK hat einen Fehler. Deine Chats und Dateien sind nicht betroffen, und ein laufender Auftrag arbeitet im Hintergrund weiter.</p>
          <p className="crash-msg">{String(error?.message || error)}</p>
          <div className="crash-actions">
            <button type="button" className="btn primary" onClick={() => location.reload()}>Neu laden</button>
            {copyBtn}
          </div>
          <p className="muted small">Passiert es immer wieder, schick die Details deiner Lehrperson.</p>
        </div>
      </div>
    );
  }
}
