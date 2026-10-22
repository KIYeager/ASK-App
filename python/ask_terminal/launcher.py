"""Starter für den Terminal-Server (läuft mit normaler Integritätsstufe).

Ablauf: Einstellungen als JSON-Zeile von stdin lesen, Arbeitsordner und Laufzeitordner mit Low
kennzeichnen, server.py angehalten starten, dessen Token auf Low absenken, in ein Job-Objekt
legen und fortsetzen. Endet stdin (App beendet oder abgestürzt) oder kommt «stop», beendet der
Job den Server mit allen Kindprozessen. Meldungen gehen als JSON-Zeilen auf stdout.
"""
import json
import os
import subprocess
import sys
import threading

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from ask_terminal import winproc  # noqa: E402

# Umgebungsvariablen, die nicht an das Modell weitergegeben werden.
DROP_ENV = ("ELECTRON_RUN_AS_NODE", "ELECTRON_NO_ATTACH_CONSOLE", "NODE_OPTIONS", "PYTHONPATH", "PYTHONHOME",
            "PYTHONSTARTUP", "VIRTUAL_ENV", "OPEN_TERMINAL_API_KEY")

_emit_lock = threading.Lock()


def emit(event: str, **data) -> None:
    with _emit_lock:
        sys.stdout.write(json.dumps({"event": event, **data}, ensure_ascii=False) + "\n")
        sys.stdout.flush()


def build_env(cfg: dict) -> dict:
    runtime = cfg["runtime"]
    python_dir = os.path.dirname(sys.executable)
    user_base = os.path.join(runtime, "python-benutzer")
    env = {k: v for k, v in os.environ.items() if k.upper() not in DROP_ENV and not k.upper().startswith("ELECTRON_")}
    path = [python_dir, os.path.join(python_dir, "Scripts"), os.path.join(user_base, "Python314", "Scripts")]
    env.update({
        "PATH": os.pathsep.join(path + [env.get("PATH", "")]),
        "TEMP": os.path.join(runtime, "tmp"),
        "TMP": os.path.join(runtime, "tmp"),
        "SHELL": "cmd.exe",
        "USER": os.environ.get("USERNAME", "nutzer"),
        "PYTHONUTF8": "1",
        "PYTHONIOENCODING": "utf-8",
        "PYTHONUNBUFFERED": "1",
        "PYTHONDONTWRITEBYTECODE": "1",
        "PYTHONUSERBASE": user_base,
        "PIP_USER": "1",
        "PIP_DISABLE_PIP_VERSION_CHECK": "1",
        "PIP_NO_WARN_SCRIPT_LOCATION": "1",
        "MPLCONFIGDIR": os.path.join(runtime, "matplotlib"),
        "MPLBACKEND": "Agg",
        "OPEN_TERMINAL_LOG_DIR": os.path.join(runtime, "logs"),
        "OPEN_TERMINAL_ENABLE_NOTEBOOKS": "false",
        "OPEN_TERMINAL_EXECUTE_TIMEOUT": str(cfg.get("execute_timeout", 60)),
        "OPEN_TERMINAL_CORS_ALLOWED_ORIGINS": cfg["origin"],
        "OPEN_TERMINAL_FILE_BROWSER_ROOT": "home",
    })
    return env


def main() -> int:
    line = sys.stdin.readline()
    if not line:
        return 2
    cfg = json.loads(line)
    workdir = os.path.realpath(cfg["workdir"])
    runtime = cfg["runtime"]
    low = cfg.get("low_integrity", True)
    emit("launcher", pid=os.getpid())

    if not os.path.isdir(workdir):
        emit("error", code="workdir", message=f"Ordner nicht gefunden: {workdir}")
        return 3
    for sub in ("tmp", "logs", "matplotlib", "python-benutzer"):
        os.makedirs(os.path.join(runtime, sub), exist_ok=True)
    if low:
        for folder in (workdir, runtime):
            ok, message = winproc.ensure_low_label(folder)
            if not ok:
                emit("error", code="label", path=folder, message=message)
                return 4
        emit("label", ok=True)

    server_py = os.path.join(os.path.dirname(os.path.abspath(__file__)), "server.py")
    job = winproc.Job()
    proc = subprocess.Popen(
        [sys.executable, "-E", "-s", "-B", "-X", "utf8", server_py],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        cwd=workdir, env=build_env(cfg),
        creationflags=winproc.CREATE_SUSPENDED | winproc.CREATE_NO_WINDOW,
    )
    try:
        if low:
            winproc.lower_token_to_low(int(proc._handle))
        job.assign(int(proc._handle))
    except OSError as exc:
        proc.kill()
        emit("error", code="spawn", message=str(exc))
        return 5
    winproc.resume_process(proc.pid)

    server_cfg = {k: cfg[k] for k in ("key", "port", "workdir", "origin") if k in cfg}
    for k in ("system_prompt", "execute_description", "log_level", "access_log"):
        if k in cfg:
            server_cfg[k] = cfg[k]
    proc.stdin.write((json.dumps(server_cfg) + "\n").encode("utf-8"))
    proc.stdin.close()
    emit("started", pid=proc.pid, low_integrity=low)

    def forward_output():
        for raw in proc.stdout:
            text = raw.decode("utf-8", "replace").rstrip()
            if text.startswith("{"):
                try:
                    data = json.loads(text)
                    if isinstance(data, dict) and "event" in data:
                        emit(data.pop("event"), **data)
                        continue
                except ValueError:
                    pass
            emit("log", line=text)

    def watch_parent():
        # stdin endet, wenn die App beendet wird oder abstürzt.
        for command in sys.stdin:
            if command.strip() == "stop":
                break
        job.terminate()

    threading.Thread(target=forward_output, daemon=True).start()
    threading.Thread(target=watch_parent, daemon=True).start()
    code = proc.wait()
    job.terminate()
    emit("exit", code=code)
    return code


if __name__ == "__main__":
    sys.exit(main())
