"""Startet open-terminal für den Handtest aus Phase 0.

Nur auf 127.0.0.1, mit zufälligem Key und erlaubtem Private-Network-Preflight.
Aufruf: python phase0_start.py <arbeitsordner> <port> <ask-origin oder *>
"""
import os
import secrets
import sys

import uvicorn

folder, port, origin = sys.argv[1], int(sys.argv[2]), sys.argv[3]
key = secrets.token_urlsafe(32)

os.chdir(folder)
os.environ["OPEN_TERMINAL_API_KEY"] = key
os.environ["OPEN_TERMINAL_CORS_ALLOWED_ORIGINS"] = origin
os.environ["SHELL"] = "cmd.exe"
os.environ["OPEN_TERMINAL_SYSTEM_PROMPT"] = (
    "Du arbeitest auf einem Windows-11-Laptop. Befehle laufen in cmd.exe. "
    f"Dein Arbeitsordner ist {os.getcwd()}. Schreibe nur in diesem Ordner."
)

from open_terminal import main  # noqa: E402  (liest die Umgebung beim Import)
from starlette.middleware.cors import CORSMiddleware  # noqa: E402

del os.environ["OPEN_TERMINAL_API_KEY"]
for mw in main.app.user_middleware:
    if mw.cls is CORSMiddleware:
        mw.kwargs["allow_private_network"] = True

print()
print(f"  URL:  http://127.0.0.1:{port}")
print(f"  Key:  {key}")
print(f"  Ordner: {os.getcwd()}")
print("  Beenden mit Ctrl+C")
print()
uvicorn.run(main.app, host="127.0.0.1", port=port, log_level="info")
