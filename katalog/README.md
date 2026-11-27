# ASK-Erweiterungen (Katalog)

Dieser Ordner ist der Inhalt eines künftigen öffentlichen GitHub-Repositorys **«KIYeager/ASK-Erweiterungen»**. ASK liest es als Katalog und zeigt die Einträge in der App zum Installieren an.

## Aufbau
```
skills/<name>/SKILL.md     ein Skill (Kopf mit name und description, danach die Anleitung)
mods/<id>/mod.json         ein Mod, dazu optional style.css, mod.js und Bilder
```
- `<name>` und `<id>` bestehen nur aus a-z, 0-9 und Bindestrichen (Mods höchstens 40 Zeichen). Der Ordnername ist gleich wie `name` bzw. `id`.
- Skills: `description` höchstens 300 Zeichen, sagt, **was** der Skill macht und **wann** er gebraucht wird.
- Mods: Format siehe den Skill `mod-erstellen` in ASK.

## Eintrag hinzufügen
1. Repository forken oder Zweig anlegen.
2. Ordner anlegen: `skills/<name>/SKILL.md` oder `mods/<id>/mod.json` mit den Dateien.
3. Lokal in ASK ausprobieren (Mod: Einstellungen → Mods, Vorschau prüfen).
4. Pull Request eröffnen. Kurz beschreiben, was der Eintrag tut und für wen er gedacht ist.

Alle Texte auf Deutsch mit Schweizer Rechtschreibung («ss» statt «ß», Anführungszeichen « »), in einfacher, freundlicher Sprache.

## Prüfliste für Skills
- Name und Ordner stimmen überein, Kopf ist vollständig.
- Beschreibung sagt klar, wann der Skill gebraucht wird.
- Anleitung ist knapp, konkret und in Befehlsform.
- Nichts, das ASK zu unsicheren Handlungen drängt (Daten löschen, Zugangsdaten auslesen, Prüfungen umgehen).
- Keine erfundenen Fakten oder Links.

## Prüfliste für Mods
Vor dem Zusammenführen **jede Datei lesen**, besonders `mod.js`:
- [ ] `mod.json` ist gültiges JSON, `id` stimmt mit dem Ordner überein.
- [ ] `mod.js` vollständig gelesen und verstanden. Es tut nur, was die Beschreibung sagt.
- [ ] **Kein Netzwerk:** kein `fetch`, `XMLHttpRequest`, `WebSocket`, `import()` von aussen, keine externen Bilder, Schriften oder Skripte.
- [ ] **Kein verschleierter Code:** keine minimierten oder codierten Texte, kein `eval`, `new Function`, `atob` mit nachgeladenem Code.
- [ ] Keine Versuche, Sicherheitseinstellungen zu ändern, Rückfragen zu bestätigen oder Zugangsdaten und Schlüssel zu lesen.
- [ ] `nachfragen` macht ASK höchstens strenger. `anweisungen` enthält nichts, was ASK zu schädlichem Verhalten bringt.
- [ ] CSS ist eingegrenzt (Klassen mit der Mod-Id, keine Rundumschläge mit `*`) und nutzt die Variablen (`--bg`, `--text`, `--acc` …), Hell und Dunkel funktionieren.
- [ ] Bilder und Töne liegen im Mod-Ordner und sind klein und passend (keine fremden Urheberrechte, keine bekannten Figuren ohne Erlaubnis).
- [ ] Der Mod funktioniert mit `prefers-reduced-motion` ohne starke Animation.

Im Zweifel nicht zusammenführen und die Autorin oder den Autor um Klärung bitten.

## Enthalten
- Skills: `bewerbung-schreiben`, `zusammenfassen`, `pruefung-vorbereiten`
- Mods: `neon`, `robo-helfer`, `pomodoro`, `vokabeln`
