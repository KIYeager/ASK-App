# Anweisungen für KI-Agenten

Gilt für alle Agenten, die in diesem Repository arbeiten (Claude Code, Codex, Copilot und andere).

## Pull Requests nach `main`: Release-Art

Jeder PR nach `main` braucht genau eines der Labels `beta`, `release` oder `kein-release`. Beim Merge veröffentlicht GitHub danach automatisch eine neue Version (README, «Vom Pull Request zur neuen Version»; ENTSCHEIDE E38). Ohne Label bleibt die Prüfung «Release-Art gewählt» rot.

Wenn du einen PR nach `main` erstellst:

1. **Frag die Person**, bevor oder während du den PR erstellst, welche Release-Art sie will. Wähl nicht selbst, auch nicht bei scheinbar klaren Fällen. Biete die drei Möglichkeiten an und nenn deinen Vorschlag mit Grund:
   - `beta`: Beta-Version, nur für Lernende mit «Beta-Versionen erhalten»
   - `release`: neue Version für alle Lernenden
   - `kein-release`: keine neue Version (z. B. nur Doku, Tests, CI)
   Frag im selben Zug, ob sie eine eigene Versionsnummer will (sonst zählt sie automatisch hoch) und ob der PR-Titel als Text im Update-Dialog passt.
2. **Setz das gewählte Label selbst** am PR, genau eines. Entferne ein anderes der drei, falls schon eines gesetzt ist.
3. Eine gewünschte eigene Nummer kommt als eigene Zeile `Version: 0.3.0` in den PR-Text, ein eigener Text für den Update-Dialog als Zeile `Release-Text: …`. Schreib den Text für Lernende verständlich, ohne Fachbegriffe.
4. Prüf danach, ob «Release-Art gewählt» grün ist, und nenn der Person die Version aus deren Details («Beim Merge wird daraus ASK …»).

Kannst du die Person nicht fragen (zum Beispiel ohne Rückkanal), setz kein Label und schreib im PR, dass die Release-Art noch gewählt werden muss.

Mergen, freigeben und Labels an PRs anderer Personen ändern nur auf ausdrücklichen Wunsch.

## Vor dem Mergen: neuen Stand von `main` holen

Auf `main` kommen laufend Commits dazu, auch ohne dass jemand daran arbeitet: Nach jedem Merge mit `beta` oder `release` macht der Workflow «Release» einen Commit «Version …» (README, «Vom Pull Request zur neuen Version»), und andere PRs werden gemergt.

1. **Vor der Arbeit:** `git fetch origin main` und vom neuesten `main` aus beginnen. Ist der PR deines Branches schon gemergt, den Branch neu von `origin/main` aufsetzen, statt auf dem alten Stand weiterzumachen.
2. **Vor dem Push und vor dem PR:** nochmals `git fetch origin main`. Ist `main` weiter als die Basis deines Branches (`git log --oneline HEAD..origin/main` zeigt etwas), `origin/main` in den Branch mergen (nicht rebasen, kein Force-Push auf fremde Branches), Konflikte lösen, Lint und Tests erneut laufen lassen.
3. **Vor dem Mergen:** direkt davor prüfen, ob `main` seit dem letzten Lauf der CI neue Commits hat (PR-Status «hinter `main`» oder `git log HEAD..origin/main`). Wenn ja: `main` in den Branch mergen, pushen und die CI auf diesem Stand abwarten. Erst mergen, wenn alle Prüfungen auf dem aktuellen Kopf des PRs grün sind und es keinen Konflikt gibt.
4. **Nach dem Merge:** `git fetch origin main`; der Commit «Version …» des Release-Workflows gehört zur Basis der nächsten Arbeit.
