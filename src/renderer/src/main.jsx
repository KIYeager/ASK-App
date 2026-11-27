import { createRoot } from 'react-dom/client';
import App from './App.jsx';
import './styles/app.css';

async function start() {
  // In der App (Tauri): window.ask aus bridge.js; im Browser (npm run dev:ui ohne App): Attrappe mit Beispieldaten
  // Testansicht für Mods (mod_testen, ENTSCHEIDE E75): immer Beispieldaten, nie der Hauptprozess
  if (new URLSearchParams(location.search).has('modtest')) await import('./dev/mockApi.js').then((m) => m.install());
  else if (window.__TAURI_INTERNALS__) await import('./bridge.js').then((m) => m.install());
  else if (!window.ask) await import('./dev/mockApi.js').then((m) => m.install());
  createRoot(document.getElementById('root')).render(<App />);
}
start();
