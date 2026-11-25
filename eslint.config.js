// ESLint: findet Fehler (unbenutzte oder unbekannte Namen, vertippte Vergleiche), keine Stilregeln.
const js = require('@eslint/js');
const globals = require('globals');

module.exports = [
  { ignores: ['node_modules/', 'src/vendor/', 'build/', 'dist/'] },
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
    files: ['src/renderer/**/*.js', 'src/main/render/**/*.js'],
    languageOptions: { sourceType: 'script', globals: { ...globals.browser, module: 'readonly', renderMarkdown: 'readonly', pdfjsLib: 'readonly' } },
  },
  { files: ['src/preload/**/*.js'], languageOptions: { globals: { ...globals.node, ...globals.browser } } },
];
