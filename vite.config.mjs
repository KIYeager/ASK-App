// Oberfläche (src/renderer) mit Vite und React. Gebaut wird nach src/renderer/dist, das Electron mit
// loadFile lädt; darum relative Pfade (base './').
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  root: 'src/renderer',
  base: './',
  plugins: [react()],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'chrome140',
    sourcemap: false,
    chunkSizeWarningLimit: 1000,
  },
  server: { port: 5173, strictPort: false },
});
