"""open-terminal mit den Anpassungen für ASK Desktop.

Wird vom Starter (launcher.py) mit Low Integrity gestartet und liest seine Einstellungen als
eine JSON-Zeile von stdin, damit der API-Key weder in der Kommandozeile noch in einer Datei steht.
Anpassungen gegenüber open-terminal 0.14.2:
  - Ordnersperre für die Datei-API (guard.py)
  - CORS nur für den ASK-Origin, Private-Network-Preflight erlaubt
  - Befehle ohne ConPTY (saubere UTF-8-Ausgabe, Zeilen in der OEM-Codepage werden umgewandelt,
    beliebig lange Zeilen), Abbruch beendet den ganzen Prozessbaum
  - Skills aus .ask/skills, eigener Systemprompt
  - read_file liest Word vollständig, auch Inhaltssteuerelemente und Kopfzeilen (documents.py)
  - WebSocket-Anmeldung über den Header, den die App einsetzt
"""
import asyncio
import ctypes
import hmac
import json
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from ask_terminal import VERSION, winproc  # noqa: E402
from ask_terminal.guard import PathGuard, install as install_guard  # noqa: E402
from ask_terminal.streams import read_lines  # noqa: E402

CREATE_NO_WINDOW = 0x08000000


def decode_output(line: bytes, fallback: str) -> str:
    """Python schreibt UTF-8, viele Windows-Programme (ping, ipconfig, Meldungen von cmd) die OEM-Codepage."""
    try:
        return line.decode("utf-8")
    except UnicodeDecodeError:
        return line.decode(fallback, errors="replace")


def read_config() -> dict:
    line = sys.stdin.readline()
    if not line:
        raise SystemExit("Keine Einstellungen auf stdin erhalten")
    return json.loads(line)


class WsAuthBridge:
    """Lässt WebSocket-Verbindungen mit gültigem Authorization-Header ohne echten Token zu.

    Open WebUI schickt beim interaktiven Terminal den Key als erste Nachricht. In ASK steht nur
    ein Platzhalter; die App setzt den echten Key als Header in den Handshake. Ist dieser gültig,
    wird der Token der ersten Nachricht durch den echten Key ersetzt.
    """

    def __init__(self, app, key: str):
        self.app = app
        self.expected = f"Bearer {key}".encode()
        self.key = key

    async def __call__(self, scope, receive, send):
        if scope["type"] != "websocket":
            return await self.app(scope, receive, send)
        header = dict(scope.get("headers") or []).get(b"authorization", b"")
        if not hmac.compare_digest(header, self.expected):
            return await self.app(scope, receive, send)

        first = True

        async def patched_receive():
            nonlocal first
            message = await receive()
            if first and message.get("type") == "websocket.receive" and message.get("text"):
                first = False
                try:
                    data = json.loads(message["text"])
                except ValueError:
                    return message
                if isinstance(data, dict) and data.get("type") == "auth":
                    data["token"] = self.key
                    message = {**message, "text": json.dumps(data)}
            return message

        return await self.app(scope, patched_receive, send)


def main() -> None:
    cfg = read_config()
    key = cfg["key"]
    workdir = os.path.realpath(cfg["workdir"])

    # Die versteckte Konsole teilen alle Befehle; UTF-8 vermeidet kaputte Umlaute bei cmd-Befehlen.
    ctypes.windll.kernel32.SetConsoleOutputCP(65001)
    ctypes.windll.kernel32.SetConsoleCP(65001)

    os.environ["OPEN_TERMINAL_API_KEY"] = key
    os.environ["OPEN_TERMINAL_SYSTEM_PROMPT"] = cfg.get("system_prompt", "")
    os.environ["OPEN_TERMINAL_EXECUTE_DESCRIPTION"] = cfg.get("execute_description", "")
    os.chdir(workdir)

    from open_terminal import main as ot  # liest die Umgebung beim Import
    from open_terminal.utils.fs import UserFS
    from open_terminal.utils.runner import PipeRunner
    from starlette.middleware.cors import CORSMiddleware

    for name in ("OPEN_TERMINAL_API_KEY", "OPEN_TERMINAL_SYSTEM_PROMPT", "OPEN_TERMINAL_EXECUTE_DESCRIPTION"):
        os.environ.pop(name, None)

    for middleware in ot.app.user_middleware:
        if middleware.cls is CORSMiddleware:
            middleware.kwargs["allow_origins"] = [cfg["origin"]]
            middleware.kwargs["allow_private_network"] = True

    install_guard(UserFS, PathGuard(workdir))
    ot.GLOBAL_SKILL_DIRS = (".ask/skills",) + tuple(ot.GLOBAL_SKILL_DIRS)

    oem = f"cp{ctypes.windll.kernel32.GetOEMCP()}"

    class TreePipeRunner(PipeRunner):
        async def read_output(self, log_file) -> None:
            async def read_stream(stream, label):
                async def write(line):
                    if log_file:
                        entry = {"type": label, "data": decode_output(line, oem), "ts": time.time()}
                        await log_file.write(json.dumps(entry) + "\n")

                # nicht «async for line in stream»: das bricht bei Zeilen über 64 KiB ab (streams.py)
                await read_lines(stream, write)

            async with asyncio.TaskGroup() as readers:
                readers.create_task(read_stream(self._process.stdout, "stdout"))
                readers.create_task(read_stream(self._process.stderr, "stderr"))

        def kill(self, force: bool = False) -> None:
            if self._process is not None and self._process.returncode is None:
                subprocess.run(["taskkill", "/T", "/F", "/PID", str(self._process.pid)],
                               capture_output=True, creationflags=CREATE_NO_WINDOW)

    async def create_runner(command, cwd, env, run_as_user=None):
        runner = TreePipeRunner(command, cwd, env)
        await runner.start()
        return runner

    ot.create_runner = create_runner

    # read_file: eigener Word-Extraktor (documents.py), der auch Inhaltssteuerelemente liest
    original_worker = ot.run_worker
    documents_py = os.path.join(os.path.dirname(os.path.abspath(__file__)), "documents.py")

    async def run_worker(command, *args, **kwargs):
        if list(command[1:]) == ["-m", "open_terminal.utils.documents"]:
            command = [command[0], documents_py]
        return await original_worker(command, *args, **kwargs)

    ot.run_worker = run_worker

    try:
        integrity = winproc.current_integrity_sid()
    except OSError:
        integrity = "unbekannt"

    @ot.app.get("/ask/info", include_in_schema=False, dependencies=[ot.Depends(ot.verify_api_key)])
    async def ask_info():
        return {"version": VERSION, "workdir": workdir, "low_integrity": integrity == winproc.LOW_SID,
                "integrity_sid": integrity, "pid": os.getpid()}

    import uvicorn

    print(json.dumps({"event": "server", "pid": os.getpid(), "integrity": integrity}), flush=True)
    uvicorn.run(WsAuthBridge(ot.app, key), host="127.0.0.1", port=int(cfg["port"]),
                log_level=cfg.get("log_level", "warning"), access_log=bool(cfg.get("access_log", False)))


if __name__ == "__main__":
    main()
