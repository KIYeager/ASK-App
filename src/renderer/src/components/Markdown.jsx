import { useEffect, useMemo, useRef } from 'react';
import { renderMarkdown } from '../lib/md.mjs';

// Antwort als Markdown. Inline-Code, der einer Datei im Arbeitsordner entspricht, wird als Link markiert
// (Klick öffnet die Vorschau); Links öffnen im Browser des Systems; Codeblöcke haben «Kopieren».
export default function Markdown({ text, isFile, onFile, onLink, onCopy, className = '' }) {
  const ref = useRef(null);
  const html = useMemo(() => renderMarkdown(text), [text]);
  useEffect(() => {
    const root = ref.current;
    if (!root) return;
    root.querySelectorAll('code').forEach((c) => {
      if (!c.closest('pre')) c.classList.toggle('file', !!isFile?.(c.textContent));
    });
  });
  const onClick = (e) => {
    const copy = e.target.closest('button.copy');
    if (copy) {
      const code = copy.closest('.codeblock')?.querySelector('pre code')?.textContent ?? '';
      onCopy?.(code);
      copy.textContent = 'Kopiert';
      setTimeout(() => { copy.textContent = 'Kopieren'; }, 1500);
      return;
    }
    const link = e.target.closest('a');
    if (link) {
      e.preventDefault();
      onLink?.(link.getAttribute('href'));
      return;
    }
    const code = e.target.closest('code.file');
    if (code) onFile?.(code.textContent);
  };
  return <div ref={ref} className={`md ${className}`} onClick={onClick} dangerouslySetInnerHTML={{ __html: html }} />;
}
