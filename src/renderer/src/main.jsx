import { createRoot } from 'react-dom/client';
import App from './App.jsx';
import './styles/app.css';

async function start() {
  // Ohne Electron (Vite im Browser): Attrappe mit Beispieldaten
  if (!window.ask) await import('./dev/mockApi.js').then((m) => m.install());
  createRoot(document.getElementById('root')).render(<App />);
}
start();
