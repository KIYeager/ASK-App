// Schnittstelle zum Hauptprozess (src/renderer/src/bridge.js, Tauri). Im Browser (npm run dev:ui) ersetzt
// dev/mockApi.js sie durch eine Attrappe mit Beispieldaten.
export const api = () => window.ask;
