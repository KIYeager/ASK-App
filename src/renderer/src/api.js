// Schnittstelle zum Hauptprozess (src/preload/shell.js). Im Browser (npm run dev:ui) ersetzt
// dev/mockApi.js sie durch eine Attrappe mit Beispieldaten.
export const api = () => window.ask;
