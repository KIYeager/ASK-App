// ESLint: findet Fehler (unbenutzte oder unbekannte Namen, vertippte Vergleiche), keine Stilregeln.
const js = require('@eslint/js');
const globals = require('globals');
const react = require('eslint-plugin-react');
const reactHooks = require('eslint-plugin-react-hooks');

module.exports = [
  { ignores: ['node_modules/', '.claude/', 'src/vendor/', 'build/', 'dist/', 'src/renderer/dist/', 'src-tauri/target/', 'src-tauri/gen/'] },
  js.configs.recommended,
  {
    files: ['**/*.js'],
    languageOptions: { ecmaVersion: 2023, sourceType: 'commonjs', globals: { ...globals.node } },
    rules: {
      'no-unused-vars': ['error', { args: 'none', caughtErrors: 'none', varsIgnorePattern: '^_', destructuredArrayIgnorePattern: '^_' }],
      'no-empty': ['error', { allowEmptyCatch: true }],
      'no-control-regex': 'off', // Steuerzeichen in Dateinamen und als Platzhalter sind hier Absicht
    },
  },
  {
    files: ['src/renderer/**/*.js', 'src/render/**/*.js'],
    languageOptions: { sourceType: 'script', globals: { ...globals.browser, module: 'readonly', renderMarkdown: 'readonly', pdfjsLib: 'readonly' } },
  },
  // Code von Mods läuft in der Oberfläche mit der Schnittstelle ASK (ENTSCHEIDE E72)
  {
    files: ['resources/mods-beispiele/**/*.js'],
    languageOptions: { sourceType: 'script', globals: { ...globals.browser, ASK: 'readonly' } },
  },
  // Oberfläche mit React und Vite (ENTSCHEIDE E42): ES-Module und JSX
  {
    files: ['src/renderer/src/**/*.{js,jsx,mjs}', 'vite.config.mjs'],
    languageOptions: { ecmaVersion: 2023, sourceType: 'module', parserOptions: { ecmaFeatures: { jsx: true } }, globals: { ...globals.browser } },
    plugins: { react, 'react-hooks': reactHooks },
    rules: {
      'no-unused-vars': ['error', { args: 'none', caughtErrors: 'none', varsIgnorePattern: '^_', destructuredArrayIgnorePattern: '^_' }],
      'react/jsx-uses-vars': 'error',
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'off', // bewusst: einige Effekte laufen nur bei bestimmten Änderungen
      'no-control-regex': 'off', // Platzhalter im Markdown-Renderer
    },
  },
];
