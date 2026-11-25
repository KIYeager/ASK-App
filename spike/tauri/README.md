# Tauri-Spike

Prototypen zur Frage, ob ASK Desktop auf Tauri 2 laufen könnte. Ergebnis und Empfehlung: [docs/TAURI-SPIKE.md](../../docs/TAURI-SPIKE.md). Kein Teil der App, wird nicht gebaut oder ausgeliefert.

- Linux: `npm ci` im Repo, dann `spike/tauri/run-linux.sh` (braucht Rust, libwebkit2gtk-4.1-dev, xvfb, imagemagick). Ergebnis in `results/linux/`.
- Windows: `spike\tauri\run-windows.ps1` (ungetestet).
- Windows-Installer unter Linux: in `app/` `npm install`, dann `npx tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis` (braucht cargo-xwin, nsis, lld, llvm, clang).
