"""Integrationstest für python/ask_terminal (Starter und Server).

Läuft mit dem mitgelieferten Python, das open-terminal enthält:
    <python>\\python.exe -m unittest tests\\test_terminal.py -v
"""
import asyncio
import json
import os
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LAUNCHER = os.path.join(ROOT, "python", "ask_terminal", "launcher.py")
ORIGIN = "https://ask.example.ch"


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class TerminalTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.base = tempfile.mkdtemp(prefix="ask-test-")
        # Umlaut im Pfad und Einstellungen als UTF-8, genau wie die App sie schickt (JSON.stringify)
        cls.workdir = os.path.join(cls.base, "Arbeitsordner Prüfung")
        cls.outside = os.path.join(cls.base, "ausserhalb")
        cls.runtime = os.path.join(cls.base, "laufzeit")
        os.makedirs(cls.workdir)
        os.makedirs(cls.outside)
        with open(os.path.join(cls.outside, "geheim.txt"), "w", encoding="utf-8") as f:
            f.write("nicht lesen")
        with open(os.path.join(cls.workdir, "Übung.txt"), "w", encoding="utf-8") as f:
            f.write("Grüezi")
        cls.key = secrets.token_urlsafe(32)
        cls.port = free_port()
        cls.events = []
        cls.proc = subprocess.Popen([sys.executable, "-E", "-s", "-B", "-X", "utf8", LAUNCHER], stdin=subprocess.PIPE,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        cfg = {"key": cls.key, "port": cls.port, "workdir": cls.workdir, "origin": ORIGIN,
               "runtime": cls.runtime, "system_prompt": "Testprompt für {ordner}",
               "execute_description": "Windows, cmd.exe"}
        cls.proc.stdin.write((json.dumps(cfg, ensure_ascii=False) + "\n").encode("utf-8"))
        cls.proc.stdin.flush()
        threading.Thread(target=cls._read_events, daemon=True).start()
        deadline = time.time() + 40
        while time.time() < deadline:
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{cls.port}/health", timeout=1)
                break
            except OSError:
                time.sleep(0.3)
        else:
            raise RuntimeError(f"Server startet nicht: {cls.events}")

    @classmethod
    def _read_events(cls):
        for raw in cls.proc.stdout:
            try:
                cls.events.append(json.loads(raw))
            except ValueError:
                cls.events.append({"raw": raw.decode(errors="replace")})

    @classmethod
    def tearDownClass(cls):
        if cls.proc.poll() is None:
            cls.proc.stdin.close()
            cls.proc.wait(10)
        shutil.rmtree(cls.base, ignore_errors=True)

    def request(self, method, path, body=None, key=None, headers=None):
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(f"http://127.0.0.1:{self.port}{path}", data=data, method=method)
        req.add_header("Authorization", f"Bearer {key or self.key}")
        if data is not None:
            req.add_header("Content-Type", "application/json")
        for k, v in (headers or {}).items():
            req.add_header(k, v)
        try:
            with urllib.request.urlopen(req, timeout=90) as res:
                return res.status, json.loads(res.read() or b"null"), dict(res.headers)
        except urllib.error.HTTPError as err:
            return err.code, json.loads(err.read() or b"null"), dict(err.headers)

    def test_low_integrity_and_info(self):
        status, info, _ = self.request("GET", "/ask/info")
        self.assertEqual(status, 200)
        self.assertTrue(info["low_integrity"], info)
        self.assertEqual(os.path.normcase(info["workdir"]), os.path.normcase(os.path.realpath(self.workdir)))

    def test_key_required(self):
        status, _, _ = self.request("GET", "/files/cwd", key="falsch")
        self.assertEqual(status, 401)

    def test_system_prompt(self):
        status, body, _ = self.request("GET", "/system")
        self.assertEqual(status, 200)
        self.assertIn("Testprompt", body["prompt"])

    def test_guard_blocks_outside(self):
        for path in (os.path.join(self.outside, "geheim.txt"), "..\\ausserhalb\\geheim.txt", "C:\\Windows\\win.ini"):
            status, body, _ = self.request("GET", "/files/read?path=" + urllib.request.quote(path))
            self.assertEqual(status, 403, (path, body))
        status, body, _ = self.request("POST", "/files/write", {"path": "..\\ausserhalb\\neu.txt", "content": "x"})
        self.assertIn(status, (400, 403), body)
        self.assertFalse(os.path.exists(os.path.join(self.outside, "neu.txt")))
        status, body, _ = self.request("POST", "/execute?wait=5", {"command": "echo x", "cwd": self.outside})
        self.assertEqual(status, 403, body)

    def test_write_inside(self):
        status, body, _ = self.request("POST", "/files/write", {"path": "ausgabe/test.txt", "content": "Grüsse"})
        self.assertEqual(status, 200, body)
        with open(os.path.join(self.workdir, "ausgabe", "test.txt"), encoding="utf-8") as f:
            self.assertEqual(f.read(), "Grüsse")

    def output(self, body):
        return "".join(entry.get("data", "") for entry in body.get("output", []))

    def test_read_docx_with_content_controls(self):
        # Deckblatt der TF-Bern-Vorlage: Text in <w:sdt>; dazu Kopfzeile und Tabelle
        from docx import Document
        from docx.oxml import parse_xml
        doc = Document()
        doc.sections[0].header.paragraphs[0].text = "Kopf mit Logo"
        sdt = parse_xml(
            '<w:sdt xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:sdtContent>'
            '<w:p><w:r><w:t>Deckblatt Titel</w:t></w:r></w:p>'
            '<w:tbl><w:tr><w:tc><w:p><w:r><w:t>Name: Kayu</w:t></w:r></w:p></w:tc></w:tr></w:tbl>'
            '</w:sdtContent></w:sdt>')
        doc.element.body.insert(0, sdt)
        doc.add_paragraph("Haupttext")
        doc.save(os.path.join(self.workdir, "vorlage.docx"))
        status, body, _ = self.request("GET", "/files/read?path=vorlage.docx")
        self.assertEqual(status, 200, body)
        for text in ("Deckblatt Titel", "Name: Kayu", "Haupttext", "Kopf mit Logo"):
            self.assertIn(text, body["content"])

    def test_execute_utf8_and_cwd(self):
        status, body, _ = self.request("POST", "/execute", {"command": 'python -c "print(\'äöü ß\')" & cd & dir /b'})
        self.assertEqual(status, 200, body)
        self.assertEqual(body["status"], "done", body)
        text = self.output(body)
        self.assertIn("äöü ß", text)
        self.assertIn("Übung.txt", text)
        self.assertIn(os.path.basename(self.workdir), text)
        self.assertNotIn("\x1b[", text)

    def test_execute_oem_output(self):
        # ping schreibt in der OEM-Codepage (deutsches Windows: «Ping wird ausgeführt für …»)
        status, body, _ = self.request("POST", "/execute?wait=30", {"command": "ping -n 1 127.0.0.1"})
        self.assertEqual(status, 200, body)
        text = self.output(body)
        self.assertNotIn("�", text)
        self.assertIn("127.0.0.1", text)

    def test_decode_output(self):
        sys.path.insert(0, os.path.join(ROOT, "python"))
        from ask_terminal.server import decode_output
        self.assertEqual(decode_output("ü".encode("utf-8"), "cp850"), "ü")
        self.assertEqual(decode_output("ausgeführt".encode("cp850"), "cp850"), "ausgeführt")

    def test_shell_cannot_write_outside(self):
        target = os.path.join(self.outside, "shell.txt")
        status, body, _ = self.request("POST", "/execute", {"command": f'echo x > "{target}"'})
        self.assertEqual(status, 200, body)
        self.assertFalse(os.path.exists(target), self.output(body))
        target_home = os.path.join(os.path.expanduser("~"), f"ask-test-{secrets.token_hex(4)}.txt")
        self.request("POST", "/execute", {"command": f'echo x > "{target_home}"'})
        self.assertFalse(os.path.exists(target_home))

    def test_python_libraries(self):
        script = "import docx, openpyxl, pptx, pypdf, reportlab, pandas, matplotlib; print('ok')"
        status, body, _ = self.request("POST", "/execute", {"command": f'python -c "{script}"'})
        self.assertIn("ok", self.output(body), body)

    def test_kill_tree(self):
        script = "import os, time; open('pid.txt', 'w').write(str(os.getpid())); time.sleep(120)"
        status, body, _ = self.request("POST", "/execute?wait=0", {"command": f'python -c "{script}"'})
        process_id = body["id"]
        pid_file = os.path.join(self.workdir, "pid.txt")
        for _ in range(50):
            if os.path.exists(pid_file) and open(pid_file).read():
                break
            time.sleep(0.2)
        pid = int(open(pid_file).read())
        status, _, _ = self.request("DELETE", f"/execute/{process_id}?force=true")
        self.assertEqual(status, 200)
        time.sleep(1.5)
        listing = subprocess.run(["tasklist", "/fi", f"PID eq {pid}", "/fo", "csv"], capture_output=True)
        self.assertNotIn(f'"{pid}"', listing.stdout.decode("cp850", "replace"))

    def test_cors(self):
        req = urllib.request.Request(f"http://127.0.0.1:{self.port}/files/list", method="OPTIONS")
        req.add_header("Origin", ORIGIN)
        req.add_header("Access-Control-Request-Method", "GET")
        req.add_header("Access-Control-Request-Headers", "authorization")
        req.add_header("Access-Control-Request-Private-Network", "true")
        with urllib.request.urlopen(req, timeout=5) as res:
            self.assertEqual(res.headers.get("access-control-allow-origin"), ORIGIN)
            self.assertEqual(res.headers.get("access-control-allow-private-network"), "true")
        req = urllib.request.Request(f"http://127.0.0.1:{self.port}/files/list", method="OPTIONS")
        req.add_header("Origin", "https://fremd.example")
        req.add_header("Access-Control-Request-Method", "GET")
        with self.assertRaises(urllib.error.HTTPError):
            urllib.request.urlopen(req, timeout=5)

    def test_skills_from_ask_folder(self):
        skill_dir = os.path.join(self.workdir, ".ask", "skills", "word")
        os.makedirs(skill_dir, exist_ok=True)
        with open(os.path.join(skill_dir, "SKILL.md"), "w", encoding="utf-8") as f:
            f.write("---\nname: word\ndescription: Word-Dokumente erstellen\n---\nInhalt")
        status, body, _ = self.request("GET", "/skills")
        self.assertEqual(status, 200)
        self.assertIn("word", [s["name"] for s in body])

    def test_websocket_bridge(self):
        import websockets

        status, body, _ = self.request("POST", "/api/terminals", {})
        self.assertEqual(status, 200, body)
        session = body["id"]

        async def run(header_key, token):
            headers = {"Authorization": f"Bearer {header_key}"} if header_key else {}
            try:
                async with websockets.connect(f"ws://127.0.0.1:{self.port}/api/terminals/{session}",
                                              additional_headers=headers, open_timeout=5) as ws:
                    await ws.send(json.dumps({"type": "auth", "token": token}))
                    await ws.send(b"echo bruecke\r\n")
                    received = ""
                    while "bruecke" not in received:
                        received += str(await asyncio.wait_for(ws.recv(), 5))
                    return True
            except Exception:
                return False

        self.assertFalse(asyncio.run(run(None, "ask-desktop")))
        self.assertFalse(asyncio.run(run("falsch", "ask-desktop")))
        self.assertTrue(asyncio.run(run(self.key, "ask-desktop")))


class CleanupTest(unittest.TestCase):
    def test_server_ends_when_app_dies(self):
        base = tempfile.mkdtemp(prefix="ask-test-")
        workdir = os.path.join(base, "ordner")
        os.makedirs(workdir)
        port = free_port()
        proc = subprocess.Popen([sys.executable, "-E", "-s", "-B", LAUNCHER], stdin=subprocess.PIPE,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        cfg = {"key": "k" * 40, "port": port, "workdir": workdir, "origin": ORIGIN,
               "runtime": os.path.join(base, "laufzeit")}
        proc.stdin.write((json.dumps(cfg) + "\n").encode())
        proc.stdin.flush()
        for _ in range(100):
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=1)
                break
            except OSError:
                time.sleep(0.3)
        proc.kill()  # wie ein Absturz der App: Starter wird hart beendet
        proc.wait(5)
        time.sleep(2)
        with self.assertRaises(OSError):
            urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=2)
        shutil.rmtree(base, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
