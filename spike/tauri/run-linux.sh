#!/usr/bin/env bash
# Spike unter Linux (WebKitGTK) mit Xvfb durchlaufen lassen. Ergebnisse in results/linux/.
# Voraussetzungen: Rust, libwebkit2gtk-4.1-dev, xvfb, imagemagick, Node 22; im Repo «npm ci» (für vite).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
out="$here/results/linux"
mkdir -p "$out"
rm -f "$out"/*.png
work="$(mktemp -d)"
mkdir -p "$work/folder/daten"
printf 'Datum;Kategorie;Betrag\n' > "$work/folder/ausgaben.csv"
printf 'Bücher\n' > "$work/folder/daten/kategorien.txt"

(cd "$repo" && npx vite build >/dev/null)
node "$here/tools/gen-shim.mjs"
(cd "$here/app" && cargo build ${PROFILE:+--profile $PROFILE})
node "$here/fixtures/server.mjs" & fx=$!
trap 'kill $fx 2>/dev/null; rm -rf "$work"' EXIT
sleep 0.5

bin="$here/app/target/${PROFILE:-debug}/ask-tauri-spike"
SPIKE_SCENARIO=1 SPIKE_OUT="$out" SPIKE_FOLDER="$work/folder" SPIKE_DATA="$work/data" \
  WEBKIT_DISABLE_COMPOSITING_MODE=1 \
  xvfb-run -a -s "-screen 0 1440x920x24" timeout 120 "$bin" 2>"$out/stderr.log" | tee "$out/run.log"
grep -c '^SPIKE ' "$out/run.log" >/dev/null && echo "Fertig: $out"
